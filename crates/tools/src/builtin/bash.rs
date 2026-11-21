//! The bash shell tool (parity target: `internal/tools/builtin/bash.go` and
//! its foreground satellites, the execution pipeline): permission-surface
//! schema, the sandbox seam, session-temp and egress env construction, the
//! bounded foreground runner, timeout and exit classification, and the
//! host-side notes appended to command output. Background jobs are the
//! bgjobs slice's machine and refuse here the way Go refuses without a job
//! manager.

#![forbid(unsafe_code)]

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use reasonix_safety::sandbox::shell;
use reasonix_safety::sandbox::shell::ShellKind;
use reasonix_safety::sandbox::{self, Prepared, Shell, Spec};

use super::bash_cmdline::check_command_line;
use super::bash_env::{apply_env_overrides, command_env, shell_env_overrides};
use super::bash_nul::{NulWatch, arm_nul_watch};
use super::bash_pipestatus::PipeStatusProbe;
use super::bashrun::{CancelToken, RunError, go_duration_text, run_foreground};
use super::confine::SessionTempSource;
use super::grep::timeout as tool_timeout;
use crate::abi::{CallContext, Tool, ToolResult};
use crate::refusal::Refusal;
use crate::shell::DetailedExecutor;
use crate::shell::{
    DetailedResult, MutationRisk, ShellExecution, ShellName, ShellPhase, ShellState, ShellVersion,
    Verification, duration_ms,
};

/// An abstract "prefer the dedicated tools" loses to a runnable Get-Content
/// line, so the steer states what read_file returns and what those line
/// numbers buy (Go `bashToolSteer`; the named tools are read_file,
/// edit_file, write_file).
pub const BASH_TOOL_STEER: &str = " Use for builds, tests, git, package managers, etc. To read or modify a file use read_file / edit_file / write_file — never Get-Content, cat, sed, head/tail or a shell redirect. read_file returns up to 2000 lines per call, each prefixed with its line number, and those numbers are what edit_file targets; paging a file through the shell instead costs one round trip per chunk and yields no line anchors. To search, call tool:grep (contents) or tool:glob (names) through use_capability — not grep, rg or find in the shell; the built-ins honor .gitignore and cap their output. A pipeline over another command's output (git log | grep) is the exception.";

/// The egress note the host appends when the proxy refused this launch's
/// traffic (Go `egressNote`). A download that failed on policy otherwise
/// reaches the model as a bare connection error it has to guess the cause
/// of.
pub fn egress_note_text(refused: &[String]) -> String {
    if refused.is_empty() {
        return String::new();
    }
    format!(
        "[host] The sandbox refused network egress to: {}. Bash reaches only the hosts in [sandbox] \
         allowed_domains and those the user allows when asked; do not retry a host the user declined.",
        refused.join(", ")
    )
}

/// The token-alias question bash asks before launch (Go
/// `refuseExternalRef`): a command naming a folder mounted read-only from
/// outside the workspace cannot reach it, and the shell's "No such file"
/// would read as proof the file is gone. The resolver is the read tools'
/// machine; `None` is Go's nil resolver and never refuses.
pub trait ExternalRefTokens: Send + Sync {
    /// The tokens of this session's read-only external references that
    /// appear in command.
    fn tokens_in(&self, command: &str) -> Vec<String>;
}

/// bash runs a shell command. The zero-value instance runs unconfined and is
/// overridden per run by [`confine_bash`]. `shell`, when set, is the
/// resolved interpreter; unset resolves lazily. `work_dir`, when non-empty,
/// is the directory the command runs in; empty uses the process cwd.
/// `timeout` caps foreground commands; zero means no tool-local cap.
#[derive(Default, Clone)]
pub struct BashTool {
    pub spec: Spec,
    pub shell: Option<Shell>,
    pub guard: Option<Arc<dyn super::confine::SessionDataGuardHook>>,
    pub work_dir: String,
    pub timeout: Duration,
    pub session_temp: Option<Arc<dyn SessionTempSource>>,
    pub external_refs: Option<Arc<dyn ExternalRefTokens>>,
}

/// A run that failed after the call was parsed, carrying the record the
/// host keeps and the output the model still needs (Go returns the
/// `(DetailedResult, err)` pair; Rust's Result carries one arm, so the pair
/// rides the error).
#[derive(Debug)]
pub struct BashError {
    /// The error text a host renders (Go's `err.Error()`).
    pub message: String,
    /// Model-visible output collected before the failure; empty when the
    /// call never reached the shell.
    pub output: String,
    /// The structured execution record, always set.
    pub execution: ShellExecution,
    /// The typed refusal, when the host stopped the call by identity.
    pub refusal: Option<Refusal>,
}

impl std::fmt::Display for BashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BashError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.refusal
            .as_ref()
            .map(|r| r as &(dyn std::error::Error + 'static))
    }
}

/// The failure behind `err` when the bash tool produced it, with the record
/// and output a caller owes the model.
pub fn bash_error<'a>(
    err: &'a (dyn std::error::Error + Send + Sync + 'static),
) -> Option<&'a BashError> {
    err.downcast_ref::<BashError>()
}

impl BashTool {
    /// The bound shell, resolving lazily for the zero-value instance (e.g.
    /// a registry that never went through confine_bash).
    fn resolved(&self) -> Shell {
        if let Some(shell) = &self.shell {
            return shell.clone();
        }
        if !self.spec.shell.path.is_empty() {
            return self.spec.shell.clone();
        }
        shell::resolve_shell()
    }

    /// Resolves this call's timeout_seconds against the host cap. A call
    /// may tighten the cap but never raise it; with the cap disabled the
    /// per-call value becomes the only deadline.
    fn foreground_timeout(&self, sec: i64) -> Duration {
        let host = self.timeout.max(Duration::ZERO);
        tool_timeout(sec, host, host)
    }

    /// Names the way out of a deadline, which differs by whose it was. A
    /// call that asked for longer than the cap is the case worth
    /// separating: telling it to raise a number the cap already overrode
    /// sends it back to the same wall.
    fn timeout_exit(&self, sec: i64) -> String {
        let host = self.timeout.max(Duration::ZERO);
        if sec <= 0 {
            return String::from(
                "that was the host cap; set timeout_seconds to fail fast next time, narrow the command, or use run_in_background",
            );
        }
        if !host.is_zero() && Duration::from_secs(sec as u64) > host {
            return format!(
                "the host cap of {} overrode the {}s you asked for, so asking for longer cannot help; narrow the command or use run_in_background",
                go_duration_text(host),
                sec
            );
        }
        String::from("raise timeout_seconds, narrow the command, or use run_in_background")
    }
}

struct Params {
    command: String,
    run_in_background: bool,
    #[allow(dead_code)]
    preserve_background_processes: bool,
    timeout_seconds: i64,
    #[allow(dead_code)]
    verifies: String,
    #[allow(dead_code)]
    description: String,
}

impl Params {
    fn parse(args: &Value) -> Result<Params, String> {
        let obj = args
            .as_object()
            .ok_or_else(|| String::from("invalid args: expected an object"))?;
        let string_field = |name: &str| -> Result<String, String> {
            match obj.get(name) {
                None | Some(Value::Null) => Ok(String::new()),
                Some(Value::String(s)) => Ok(s.clone()),
                Some(other) => Err(format!(
                    "invalid args: {name} must be a string, got {other}"
                )),
            }
        };
        let bool_field = |name: &str| -> Result<bool, String> {
            match obj.get(name) {
                None | Some(Value::Null) => Ok(false),
                Some(Value::Bool(b)) => Ok(*b),
                Some(other) => Err(format!(
                    "invalid args: {name} must be a boolean, got {other}"
                )),
            }
        };
        let number_field = |name: &str| -> Result<i64, String> {
            match obj.get(name) {
                None | Some(Value::Null) => Ok(0),
                Some(Value::Number(n)) => n
                    .as_i64()
                    .ok_or_else(|| format!("invalid args: {name} must be an integer")),
                Some(other) => Err(format!(
                    "invalid args: {name} must be an integer, got {other}"
                )),
            }
        };
        Ok(Params {
            command: string_field("command")?,
            run_in_background: bool_field("run_in_background")?,
            preserve_background_processes: bool_field("preserve_background_processes")?,
            timeout_seconds: number_field("timeout_seconds")?,
            verifies: string_field("verifies")?,
            description: string_field("description")?,
        })
    }
}

/// Reports whether seq appears in s outside any single- or double-quoted
/// span, so a literal "a && b" string argument does not trip the PowerShell
/// chaining guard (Go `hasUnquotedSeq`).
pub fn has_unquoted_seq(s: &str, seq: &str) -> bool {
    let b = s.as_bytes();
    let mut quote: u8 = 0;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
            i += 1;
            continue;
        }
        if c == b'\'' || c == b'"' {
            quote = c;
            i += 1;
            continue;
        }
        if b[i..].starts_with(seq.as_bytes()) {
            return true;
        }
        i += 1;
    }
    false
}

/// Go `appendSessionDataHint`: with no output the hint stands alone; an
/// empty hint is a no-op.
pub fn append_session_data_hint(out: &str, hint: &str) -> String {
    if hint.is_empty() {
        return out.to_owned();
    }
    if out.trim().is_empty() {
        return hint.to_owned();
    }
    format!("{out}\n\n{hint}")
}

/// Describes the shell identity without launching a process (Go
/// `shellrun.DescriptorFromShell`).
pub fn descriptor_from_shell(sh: &Shell) -> ShellExecution {
    let (name, version) = classify_shell(sh);
    ShellExecution {
        supports_and_and: sh.supports_chaining(),
        platform: Some(platform_name().to_owned()),
        shell: Some(name),
        shell_version: version,
        ..Default::default()
    }
}

fn platform_name() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

/// Go `classifyShell`: powershell.exe → powershell / 5.1; pwsh → pwsh / 7+;
/// a Git-for-Windows bash → git-bash; else bash.
fn classify_shell(sh: &Shell) -> (ShellName, Option<ShellVersion>) {
    let base = sh.path.rsplit(['/', '\\']).next().unwrap_or(&sh.path);
    let base = base.strip_suffix(".exe").unwrap_or(base);
    let base = base.to_lowercase();
    if sh.kind == ShellKind::PowerShell {
        if base == "pwsh" || sh.supports_chaining() {
            return (ShellName::Pwsh, Some(ShellVersion::Ps7));
        }
        return (ShellName::PowerShell, Some(ShellVersion::Ps51));
    }
    if is_git_bash_path(&sh.path) {
        return (ShellName::GitBash, None);
    }
    (ShellName::Bash, None)
}

fn is_git_bash_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    let norm = path.replace('\\', "/").to_lowercase();
    norm.contains("/git/") && norm.contains("bash")
}

/// The environment the child runs in: the process env, the configured shell
/// env, then the host-owned session-temp and egress overrides, which win
/// (Go `applyEnvOverrides(shellrun.CommandEnv(ctx), ...)`).
fn child_env(spec: &Spec, prepared: &Prepared) -> Vec<String> {
    let mut overrides = shell_env_overrides(&spec.shell_env);
    overrides.extend(prepared.env_overrides.iter().cloned());
    apply_env_overrides(command_env(), &overrides)
}

/// The host's notes appended after a run: the egress refusals this launch
/// hit (Go `confinementNotes` minus the git-metadata and write-root notes,
/// which fire only under an OS-wrapped command and land with the OS-sandbox
/// wave).
fn confinement_notes(out: &str, token: &str, route: Option<&dyn sandbox::EgressRoute>) -> String {
    let note = match (token, route) {
        (t, Some(r)) if !t.is_empty() => egress_note_text(&r.refusals(t)),
        _ => String::new(),
    };
    append_session_data_hint(out, &note)
}

/// The parent's stop switch, attached through the call context.
fn cancel_from(ctx: &CallContext) -> Option<CancelToken> {
    ctx.value()?.downcast_ref::<CancelToken>().cloned()
}

impl BashTool {
    /// The full execution pipeline (Go `ExecuteDetailed`).
    #[allow(clippy::result_large_err)]
    fn run_detailed(
        &self,
        args: &Value,
        cancel: Option<&CancelToken>,
    ) -> Result<DetailedResult, BashError> {
        let start = Instant::now();
        let sh = self.resolved();
        let mut ex = descriptor_from_shell(&sh);
        ex.state = Some(ShellState::Running);
        ex.mutation_risk = Some(MutationRisk::Unknown);
        ex.verification = Some(Verification::NotVerification);

        // Records a call the host stopped before any process started (Go
        // `notRun` / `refusedBeforeLaunch`).
        let not_run = |ex: ShellExecution,
                       message: String,
                       phase: ShellPhase,
                       refusal: Option<Refusal>| BashError {
            message,
            output: String::new(),
            execution: ShellExecution {
                state: Some(ShellState::NotRun),
                failure_phase: Some(phase),
                mutation_risk: Some(MutationRisk::NotStarted),
                duration_ms: duration_ms(start.elapsed()),
                ..ex
            },
            refusal,
        };

        let p = match Params::parse(args) {
            Ok(p) => p,
            Err(detail) => {
                return Err(not_run(
                    ex,
                    format!("invalid args: {detail}"),
                    ShellPhase::Preflight,
                    None,
                ));
            }
        };
        if p.command.is_empty() {
            return Err(not_run(
                ex,
                String::from("command is required"),
                ShellPhase::Preflight,
                None,
            ));
        }
        if !sh.supports_chaining()
            && (has_unquoted_seq(&p.command, "&&") || has_unquoted_seq(&p.command, "||"))
        {
            return Err(not_run(
                ex,
                String::from(
                    "this shell is Windows PowerShell, which does not parse '&&' or '||'. \
                     Sequence with ';' (both run regardless of the first's result), use 'if ($?) { ... }' for \
                     conditional chaining, or issue the commands as separate calls",
                ),
                ShellPhase::Preflight,
                None,
            ));
        }

        // The pipe-status probe and the NUL watch arm before any launch
        // path: the NUL pre-check must see the directory before this
        // command can create the file.
        let probe =
            PipeStatusProbe::arm(sh.kind == ShellKind::Bash, &p.command, p.run_in_background);
        let launch = probe.wrap(&p.command);
        let nul: NulWatch = arm_nul_watch(sh.kind, &self.work_dir);

        // Pin the session-private temporary generation before any launch
        // path so foreground, background, and host-terminal runs share one
        // directory.
        let session_dir = self
            .session_temp
            .as_ref()
            .and_then(|m| m.ensure_dir())
            .unwrap_or_default();

        // The seam: Go-nil (no engine) never wraps, so an enforcing spec
        // fails closed here instead of running unconfined.
        let mut spec = self.spec.clone();
        spec.session_temp = session_dir.clone();
        let (argv, wrapped) = sandbox::command(&spec, &sh, &launch);
        let linux_sandboxed = wrapped && !session_dir.is_empty() && cfg!(target_os = "linux");
        let env_overrides = sandbox::session_temp_env(&session_dir, linux_sandboxed);
        let mut prepared = Prepared {
            argv,
            wrapped,
            session_temp: session_dir,
            env_overrides,
            linux_sandboxed,
            egress_token: String::new(),
        };
        if spec.enforce() && !prepared.wrapped {
            return Err(not_run(
                ex,
                sandbox::unavailable_message(),
                ShellPhase::Preflight,
                None,
            ));
        }
        let mut egress_route: Option<Arc<dyn sandbox::EgressRoute>> = None;
        if prepared.wrapped
            && spec.network
            && let Some(route) = spec.egress.clone()
        {
            let token = reasonix_safety::egress::new_token();
            prepared.env_overrides.extend(route.env(&token));
            prepared.egress_token = token;
            egress_route = Some(route);
        }

        if let Some(refs) = &self.external_refs
            && let Some(first) = refs.tokens_in(&p.command).first()
        {
            return Err(not_run(
                ex,
                format!(
                    "{first} is this session's read-only reference to a folder outside the workspace, \
                     not a path on disk — bash cannot reach it. read_file, ls, glob and grep resolve these; \
                     use one of them"
                ),
                ShellPhase::Preflight,
                None,
            ));
        }

        if let Err(refusal) = check_command_line(&prepared.argv) {
            return Err(not_run(
                ex,
                refusal.to_string(),
                ShellPhase::Preflight,
                Some(refusal),
            ));
        }

        if p.run_in_background {
            // Background jobs own the lease and the streaming buffer; this
            // slice carries no job manager, so the call refuses exactly as
            // Go does when the context carries none.
            return Err(not_run(
                ex,
                String::from("background execution is not available in this context"),
                ShellPhase::Dependency,
                None,
            ));
        }

        let cmd_env = child_env(&spec, &prepared);
        let result = run_foreground(
            super::bashrun::Request {
                argv: prepared.argv.clone(),
                dir: self.work_dir.clone(),
                env: cmd_env,
                timeout: self.foreground_timeout(p.timeout_seconds),
                wait_delay: Duration::from_secs(5),
                progress: None,
            },
            cancel,
        );

        let (combined, tail, pipe_status) = probe.harvest(&result.combined, &result.output_tail);
        ex.pipe_status = pipe_status;
        ex.state = Some(result.state);
        ex.failure_phase = Some(match result.state {
            ShellState::TimedOut => ShellPhase::Timeout,
            ShellState::Cancelled => ShellPhase::Cancellation,
            ShellState::Failed if !result.started => ShellPhase::Launch,
            ShellState::Failed => ShellPhase::Execution,
            ShellState::NotRun => ShellPhase::Launch,
            _ => ShellPhase::Execution,
        });
        ex.exit_code = result.exit_code;
        ex.output_tail = (!tail.is_empty()).then_some(tail);
        ex.mutation_risk = Some(match result.state {
            ShellState::Completed => MutationRisk::MayHaveCompleted,
            ShellState::NotRun => MutationRisk::NotStarted,
            ShellState::Failed
                if matches!(
                    ex.failure_phase,
                    Some(ShellPhase::Launch) | Some(ShellPhase::Preflight)
                ) =>
            {
                MutationRisk::NotStarted
            }
            ShellState::Failed | ShellState::TimedOut | ShellState::Cancelled => {
                MutationRisk::MayBePartial
            }
            _ => MutationRisk::Unknown,
        });
        ex.duration_ms = duration_ms(start.elapsed());

        let message = match result.err.clone() {
            Some(RunError::Timeout { .. }) => format!(
                "{}; {}",
                result
                    .err
                    .as_ref()
                    .map(|e| e.to_string())
                    .unwrap_or_default(),
                self.timeout_exit(p.timeout_seconds)
            ),
            Some(RunError::Exit { .. }) => {
                format!(
                    "command exited: {}",
                    result
                        .err
                        .as_ref()
                        .map(|e| e.to_string())
                        .unwrap_or_default()
                )
            }
            Some(err) => err.to_string(),
            None => String::new(),
        };

        let hint = self
            .guard
            .as_ref()
            .map(|g| g.command_hint(&self.work_dir, &p.command))
            .unwrap_or_default();
        let out = append_session_data_hint(
            &append_session_data_hint(
                &confinement_notes(&combined, &prepared.egress_token, egress_route.as_deref()),
                &hint,
            ),
            &nul.note(),
        );
        if result.state == ShellState::Completed {
            return Ok(DetailedResult {
                output: out,
                images: Vec::new(),
                execution: Some(ex),
            });
        }
        Err(BashError {
            message,
            output: out,
            execution: ex,
            refusal: None,
        })
    }
}

impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        // Two variants resolved from the bound shell (tools.md §2.2). The
        // strings are cached per variant; the PowerShell one is keyed by
        // chaining support because pwsh and 5.1 answer differently.
        let sh = self.resolved();
        if sh.kind == ShellKind::PowerShell {
            static POWERSHELL_5_1: OnceLock<String> = OnceLock::new();
            static POWERSHELL_7: OnceLock<String> = OnceLock::new();
            let cache = if sh.supports_chaining() {
                &POWERSHELL_7
            } else {
                &POWERSHELL_5_1
            };
            let (shell_name, chaining) = if sh.supports_chaining() {
                (
                    "PowerShell 7 (pwsh)",
                    "'&&' and '||' are parsed for conditional chaining; ';' runs both regardless.",
                )
            } else {
                (
                    "Windows PowerShell",
                    "';' runs both regardless; 'if ($?) { ... }' is conditional. '&&' and '||' are NOT parsed.",
                )
            };
            cache.get_or_init(|| {
                format!(
                    "Execute a command in the shell and return combined stdout/stderr. \
                     NOTE: bash is not available on this host — commands run under {shell_name}, so write PowerShell, not bash:\n\
                     \x20 - chaining: {chaining}\n\
                     \x20 - redirect/vars: $null not /dev/null; $env:VAR not $VAR; '2>$null' drops stderr.\n\
                     \x20 - file ops: Get-ChildItem (ls), Remove-Item -Recurse -Force (rm -rf), Copy-Item (cp), Select-String (grep).\n\
                     \x20 - no head/tail/which/touch: use Select-Object -First/-Last N, (Get-Command x).Source, New-Item.\n\
                     \x20 - multi-line text to a native exe (e.g. git commit -m): use a single-quoted here-string @'...'@ (closing '@ at column 0).{BASH_TOOL_STEER}"
                )
            })
        } else {
            static POSIX: OnceLock<String> = OnceLock::new();
            POSIX.get_or_init(|| format!("Execute a command in the shell and return combined stdout/stderr.{BASH_TOOL_STEER}"))
        }
    }

    fn schema(&self) -> Value {
        go_schema()
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let cancel = cancel_from(ctx);
        match self.run_detailed(args, cancel.as_ref()) {
            Ok(res) => Ok(res.output),
            Err(err) => match &err.refusal {
                Some(refusal) => Err(Box::new(refusal.clone())),
                None => Err(Box::new(err)),
            },
        }
    }

    fn read_only(&self) -> bool {
        // bash's effect cannot be inferred from args (rm, curl, git commit
        // are all reachable). Conservative even when a particular command
        // happens to be read-only — the batch decision cannot tell.
        false
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl DetailedExecutor for BashTool {
    fn execution_descriptor(&self, _args: &Value) -> ShellExecution {
        descriptor_from_shell(&self.resolved())
    }

    fn execute_detailed(
        &self,
        ctx: &CallContext,
        args: &Value,
    ) -> Result<DetailedResult, ToolResult> {
        let cancel = cancel_from(ctx);
        match self.run_detailed(args, cancel.as_ref()) {
            Ok(res) => Ok(res),
            Err(err) => Err(Err(Box::new(err))),
        }
    }
}

/// Go `bash.Schema()`, verbatim.
fn go_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "command": {"type": "string", "description": "Shell command to execute"},
            "description": {"type": "string", "description": "Optional short name for this call, in the user's language, shown in task lists."},
            "run_in_background": {"type": "boolean", "description": "Run detached: returns a job id immediately and keeps running across turns (no foreground timeout). Read new output with bash_output, wait with wait, stop it with kill_shell. Use for long-running commands like servers, watchers, or builds you don't need to block on."},
            "preserve_background_processes": {"type": "boolean", "description": "After the shell command exits normally, keep any process-group members it intentionally left behind. Use only for deliberate daemonization, browser/GUI/session launchers such as playwright-cli open, or nohup/disown/setsid; cancellation and timeouts still kill the process group."},
            "timeout_seconds": {"type": "integer", "description": "Deadline for this foreground call. Only tightens the host cap — a larger value is clamped to it. Output collected before the deadline is still returned, so set it low when a command's cost is unpredictable (a broad filesystem walk, a network fetch) instead of paying the full cap to find out. For work that legitimately runs long, use run_in_background.", "minimum": 1},
            "verifies": {"type": "string", "description": "Name what this call decides (e.g. \"balances.json keys are sorted\") to record it as a check. The exit status becomes the verdict, so the command must be one command or an && chain; a ';', a pipe, or '||' hands the status to something else and the call is refused."}
        },
        "required": ["command"]
    })
}

/// confine.go `ConfineBash`: the bash built-in bound to a sandbox spec,
/// overriding the unconfined instance. An enforcing spec without a backend
/// fails closed at execute time.
pub fn confine_bash(
    spec: Spec,
    guard: Option<Arc<dyn super::confine::SessionDataGuardHook>>,
) -> BashTool {
    let shell = if !spec.shell.path.is_empty() {
        Some(spec.shell.clone())
    } else {
        Some(shell::resolve_shell())
    };
    BashTool {
        spec,
        shell,
        guard,
        ..BashTool::default()
    }
}

/// The unconfined instance a composition root overrides, mirroring the Go
/// init-time registration.
pub fn unconfined_bash() -> BashTool {
    BashTool::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_unquoted_seq_reads_quote_spans() {
        assert!(has_unquoted_seq("a && b", "&&"));
        assert!(!has_unquoted_seq("echo 'a && b'", "&&"));
        assert!(!has_unquoted_seq("echo \"a && b\"", "&&"));
        assert!(has_unquoted_seq("echo 'x' && echo 'y'", "&&"));
        assert!(has_unquoted_seq("a || b", "||"));
        assert!(!has_unquoted_seq("echo 'a || b'", "||"));
    }

    #[test]
    fn descriptor_classifies_the_shell() {
        let bash = Shell {
            kind: ShellKind::Bash,
            path: String::from("/usr/bin/bash"),
            ..Shell::default()
        };
        let ex = descriptor_from_shell(&bash);
        assert!(ex.supports_and_and);
        assert_eq!(ex.shell, Some(ShellName::Bash));
        assert_eq!(ex.shell_version, None);
        let git_bash = Shell {
            kind: ShellKind::Bash,
            path: String::from("C:\\Program Files\\Git\\bin\\bash.exe"),
            ..Shell::default()
        };
        let ex = descriptor_from_shell(&git_bash);
        assert_eq!(ex.shell, Some(ShellName::GitBash));
        let ps51 = Shell {
            kind: ShellKind::PowerShell,
            path: String::from("powershell"),
            ..Shell::default()
        };
        let ex = descriptor_from_shell(&ps51);
        assert!(!ex.supports_and_and);
        assert_eq!(ex.shell, Some(ShellName::PowerShell));
        assert_eq!(ex.shell_version, Some(ShellVersion::Ps51));
    }

    #[test]
    fn foreground_timeout_only_tightens_the_cap() {
        // Go TestBashForegroundTimeoutResolution, verbatim in expectation.
        let b = BashTool {
            timeout: Duration::from_secs(120),
            ..BashTool::default()
        };
        assert_eq!(b.foreground_timeout(0), Duration::from_secs(120));
        assert_eq!(b.foreground_timeout(10), Duration::from_secs(10));
        assert_eq!(b.foreground_timeout(600), Duration::from_secs(120));
        let uncapped = BashTool::default();
        assert_eq!(uncapped.foreground_timeout(10), Duration::from_secs(10));
        assert_eq!(uncapped.foreground_timeout(0), Duration::ZERO);
        let negative = BashTool {
            timeout: Duration::from_secs(120),
            ..BashTool::default()
        };
        assert_eq!(negative.foreground_timeout(-5), Duration::from_secs(120));
    }

    #[test]
    fn timeout_exit_separates_whose_deadline_fired() {
        // Go TestBashTimeoutExitDoesNotAdviseRaisingACappedRequest.
        let b = BashTool {
            timeout: Duration::from_secs(120),
            ..BashTool::default()
        };
        let capped = b.timeout_exit(600);
        assert!(!capped.contains("raise timeout_seconds"));
        for want in ["2m0s", "600", "run_in_background"] {
            assert!(capped.contains(want), "exit = {capped}, missing {want}");
        }
        assert!(b.timeout_exit(10).contains("raise timeout_seconds"));
        assert!(b.timeout_exit(0).contains("host cap"));
        let uncapped = BashTool::default();
        assert!(uncapped.timeout_exit(600).contains("raise timeout_seconds"));
    }

    #[test]
    fn schema_matches_go_verbatim() {
        let b = unconfined_bash();
        let schema = b.schema();
        let props = schema.get("properties").unwrap();
        for name in [
            "command",
            "description",
            "run_in_background",
            "preserve_background_processes",
            "timeout_seconds",
            "verifies",
        ] {
            assert!(props.get(name).is_some(), "{name} missing from the schema");
        }
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(required, vec!["command"], "description stays optional");
        assert_eq!(props["timeout_seconds"]["minimum"].as_i64(), Some(1));
    }

    #[test]
    fn description_carries_the_steer_and_the_variant() {
        let posix = unconfined_bash();
        assert!(
            posix
                .description()
                .starts_with("Execute a command in the shell")
        );
        assert!(
            posix
                .description()
                .contains("read_file / edit_file / write_file")
        );
        assert!(posix.description().contains("tool:grep"));
        let ps = BashTool {
            shell: Some(Shell {
                kind: ShellKind::PowerShell,
                path: String::from("powershell"),
                ..Shell::default()
            }),
            ..BashTool::default()
        };
        assert!(ps.description().contains("write PowerShell, not bash"));
        assert!(
            ps.description().contains("NOT parsed"),
            "5.1 names the limit"
        );
        let pwsh = BashTool {
            shell: Some(Shell {
                kind: ShellKind::PowerShell,
                path: String::from("pwsh"),
                ..Shell::default()
            }),
            ..BashTool::default()
        };
        assert!(pwsh.description().contains("PowerShell 7 (pwsh)"));
    }

    #[test]
    fn session_data_hint_appends_once() {
        assert_eq!(append_session_data_hint("out", ""), "out");
        assert_eq!(append_session_data_hint("", "hint"), "hint");
        assert_eq!(append_session_data_hint("   ", "hint"), "hint");
        assert_eq!(append_session_data_hint("out", "hint"), "out\n\nhint");
    }

    #[test]
    fn egress_note_names_the_refused_hosts() {
        assert_eq!(egress_note_text(&[]), "");
        let note = egress_note_text(&[String::from("evil.com (blocked-by-allowlist)")]);
        assert!(note.contains("[host] The sandbox refused network egress to: evil.com"));
        assert!(note.contains("do not retry a host the user declined"));
    }
}
