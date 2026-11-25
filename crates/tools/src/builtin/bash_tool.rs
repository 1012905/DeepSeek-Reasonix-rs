//! TOOL-025 bash satellite: the decision-only pieces that do not need a real
//! subprocess, job manager, or OS sandbox backend. Split out of `bash.rs` so
//! that file stays under the per-source line ceiling; the frozen constants,
//! pure decisions, and refusal identities the contract pins live here, while
//! process spawn, the background job table, leases, and sandbox fault
//! detection land with the later host-machinery slices.
//!
//! Parity targets: `internal/tools/builtin/bash.go`, `bash_job_exit.go`,
//! `bash_delete.go`, `bash_gitmeta.go`, `internal/safety/sandbox/sandbox.go`,
//! `win_faults.go`.

#![forbid(unsafe_code)]

use reasonix_core::shellparse_analysis::lift;
use reasonix_safety::sandbox::shell::ShellKind;

use crate::refusal::Refusal;
use crate::shell::ShellPhase;

// ---------------------------------------------------------------------------
// SnipHint (Go `bash.go:142-144`): equal-weight head/tail slices. The `Tool`
// trait has no snip-hint method yet; exposing this shape through the ABI is
// the contract-capability gap recorded with the surface slice.
// ---------------------------------------------------------------------------

/// Head slice: lines kept from the command's opening context.
pub const BASH_SNIP_HEAD: usize = 40;
/// Tail slice: lines kept from the command's final diagnostics.
pub const BASH_SNIP_TAIL: usize = 40;
/// Head character budget.
pub const BASH_SNIP_HEAD_CHARS: usize = 8000;
/// Tail character budget.
pub const BASH_SNIP_TAIL_CHARS: usize = 8000;

/// The head/tail snipping bounds a bash result carries (Go `SnipHint`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BashSnipHint {
    pub head: usize,
    pub tail: usize,
    pub head_chars: usize,
    pub tail_chars: usize,
}

impl Default for BashSnipHint {
    fn default() -> Self {
        Self {
            head: BASH_SNIP_HEAD,
            tail: BASH_SNIP_TAIL,
            head_chars: BASH_SNIP_HEAD_CHARS,
            tail_chars: BASH_SNIP_TAIL_CHARS,
        }
    }
}

// ---------------------------------------------------------------------------
// Command preview (Go `commandPreview`, `bash.go:616-624`): a one-line label
// for a background job, shown in the status bar and completion notices.
// ---------------------------------------------------------------------------

/// Rune budget for one background-job preview label.
pub const COMMAND_PREVIEW_MAX_RUNES: usize = 48;

/// Flattens newlines to spaces, trims, and truncates to [`COMMAND_PREVIEW_MAX_RUNES`]
/// runes with an ellipsis.
pub fn command_preview(cmd: &str) -> String {
    let flat = cmd.trim().replace('\n', " ");
    let runes: Vec<char> = flat.chars().collect();
    if runes.len() > COMMAND_PREVIEW_MAX_RUNES {
        runes[..COMMAND_PREVIEW_MAX_RUNES]
            .iter()
            .collect::<String>()
            + "…"
    } else {
        flat
    }
}

// ---------------------------------------------------------------------------
// Track / reap / keepalive decisions (Go `bash.go:472-518,543-551`). The AST
// walk is the pure half of process-tree bookkeeping; the tracked handle and
// the kill-tree reap itself are the proc slice's machine.
// ---------------------------------------------------------------------------

/// Shell-level daemonization commands that hand the process off deliberately
/// (Go `staticShellCallName` + the disown/nohup/setsid switch).
const KEEPALIVE_COMMANDS: [&str; 3] = ["disown", "nohup", "setsid"];

/// The static program name of a leaf, skipping assignment words and the
/// `command`/`env` wrappers Go `staticShellCallName` peels off. A dynamic
/// (non-static) name answers `None`.
fn static_call_name(argv: &[String]) -> Option<&str> {
    for word in argv {
        if word.is_empty() || word.contains('=') {
            continue;
        }
        if word.contains('\u{0}') {
            // The analysis folds a dynamic argument to the DYNAMIC_ARG marker.
            return None;
        }
        let base = word.rsplit(['/', '\\']).next().unwrap_or(word);
        if base == "command" || base == "env" {
            continue;
        }
        return Some(base);
    }
    None
}

/// Whether the command both backgrounds a statement and names one of the
/// deliberate keepalive wrappers. A plain `cmd &` does not satisfy this: it
/// must not bypass stray-process cleanup (#3702).
pub fn has_explicit_background_keepalive(command: &str) -> bool {
    let Ok(lifted) = lift(command) else {
        return false;
    };
    let mut background = false;
    let mut keepalive = false;
    for leaf in &lifted.leaves {
        if leaf.background {
            background = true;
        }
        if let Some(name) = static_call_name(&leaf.argv)
            && KEEPALIVE_COMMANDS.contains(&name)
        {
            keepalive = true;
        }
    }
    background && keepalive
}

/// Whether the direct child is tracked for later teardown (Go
/// `shouldTrackShellProcess`). Windows wrapped children are untracked; an
/// intentional preserve keeps the whole process group.
pub fn should_track_shell_process(
    wrapped: bool,
    shell: ShellKind,
    command: &str,
    preserve_background_processes: bool,
) -> bool {
    if preserve_background_processes {
        return false;
    }
    if cfg!(windows) && wrapped {
        return false;
    }
    shell != ShellKind::Bash || !has_explicit_background_keepalive(command)
}

/// Whether stray process-group members are reaped after the run (Go
/// `shouldReapAfterRun`). A cancelled context always reaps; a preserved or
/// deliberately daemonized bash group does not.
pub fn should_reap_after_run(
    ctx_cancelled: bool,
    shell: ShellKind,
    command: &str,
    preserve_background_processes: bool,
) -> bool {
    if ctx_cancelled {
        return true;
    }
    if preserve_background_processes {
        return false;
    }
    shell != ShellKind::Bash || !has_explicit_background_keepalive(command)
}

/// Whether an `ErrWaitDelay` grandchild hold is dropped instead of reported
/// as failure (Go `normalizeBashRunError`): only under an intentional
/// preserve, with the context still live.
pub fn suppress_wait_delay_error(
    preserve_background_processes: bool,
    ctx_cancelled: bool,
    wait_delay_expired: bool,
) -> bool {
    preserve_background_processes && !ctx_cancelled && wait_delay_expired
}

// ---------------------------------------------------------------------------
// Background entry (Go `bash.go:226-270`). The job manager, lease transfer,
// and streaming buffer are the A3 jobs slice; this side keeps the refusal
// sentence and the start-message template.
// ---------------------------------------------------------------------------

/// The Dependency-phase refusal when the call context carries no job manager
/// (Go `bash.go:233`).
pub const BACKGROUND_NOT_AVAILABLE: &str = "background execution is not available in this context";

/// The message returned when a background job starts (Go `bash.go:259`). The
/// job has not finished at this point, so the start is not a completed exit 0.
pub fn background_start_message(job_id: &str) -> String {
    format!(
        "Started background job {job_id:?}. It keeps running across turns; read new output with \
         bash_output(job_id={job_id:?}), wait for it with wait, or stop it with \
         kill_shell(job_id={job_id:?})."
    )
}

// ---------------------------------------------------------------------------
// Progress suppression (Go `bash_foreground.go:34`): the pipe-status probe's
// marker line must not ride the live progress stream.
// ---------------------------------------------------------------------------

/// Whether `line` is the probe marker line the live-progress sink suppresses.
/// An empty marker means the probe is disarmed and every line passes through.
pub fn progress_line_suppressed(marker: &str, line: &str) -> bool {
    !marker.is_empty() && line.trim_start().starts_with(marker)
}

// ---------------------------------------------------------------------------
// Destructive-delete guard identities (Go `bash_delete.go:20-32,144`). The
// parser/target-boundary analysis itself is its own slice (matrix G13); this
// module pins the typed codes, the shared refusal prefix, and the verbatim
// messages the analysis will return.
// ---------------------------------------------------------------------------

/// Prefix every delete-guard refusal carries (Go `deleteRefusal`).
pub const DELETE_REFUSAL_PREFIX: &str = "Shell command refused before execution: ";

/// `shell.destructive_target`: the target is home, a filesystem root, a
/// workspace ancestor, or otherwise out of scope.
pub const CODE_DESTRUCTIVE_TARGET: &str = "shell.destructive_target";
pub const MESSAGE_DESTRUCTIVE_TARGET: &str = "Use a target strictly inside the workspace or granted write roots; home, filesystem roots, workspace ancestors and out-of-scope paths are protected in every approval mode.";

/// `shell.analysis_unknown`: the recursive delete cannot be bounded.
pub const CODE_ANALYSIS_UNKNOWN: &str = "shell.analysis_unknown";
pub const MESSAGE_ANALYSIS_INTERPRETER_PAYLOAD: &str =
    "Use a literal interpreter payload or split it from the recursive delete.";
pub const MESSAGE_ANALYSIS_NESTED_EXTENT: &str =
    "Nested interpreter extent is unknown; use a direct delete with a literal workspace path.";
pub const MESSAGE_ANALYSIS_DYNAMIC_EXECUTABLE: &str = "A dynamic executable has recursive-delete arguments; use a literal executable and a literal workspace path.";

/// `shell.delete_sequence`: a non-literal directory change precedes the
/// recursive delete.
pub const CODE_DELETE_SEQUENCE: &str = "shell.delete_sequence";
pub const MESSAGE_DELETE_SEQUENCE: &str = "Split the call; only literal directory changes may precede a recursive delete. Assignments and nested control flow cannot establish a safe target.";

/// `shell.delete_nonliteral`: the target is an expansion, variable, glob, or
/// pipeline input.
pub const CODE_DELETE_NONLITERAL: &str = "shell.delete_nonliteral";
pub const MESSAGE_DELETE_NONLITERAL: &str = "Use a literal target path strictly inside the workspace or granted write roots; expansions, variables, globs and pipeline input are not bounded.";

/// `shell.delete_option`: an unknown delete option or an unspecified value.
pub const CODE_DELETE_OPTION: &str = "shell.delete_option";
pub const MESSAGE_DELETE_OPTION: &str =
    "Use known delete options and common parameters with explicit values.";

/// `shell.syntax_error`: the host parser could not parse the command.
pub const CODE_SYNTAX_ERROR: &str = "shell.syntax_error";

/// `shell.parser_unavailable`: the host shell parser is missing (PowerShell
/// branch only).
pub const CODE_PARSER_UNAVAILABLE: &str = "shell.parser_unavailable";
pub const MESSAGE_PARSER_UNAVAILABLE: &str =
    "The host shell parser is unavailable; retry when the host parser is restored.";

/// `shell.parser_timeout`: the host parser timed out or was canceled.
pub const CODE_PARSER_TIMEOUT: &str = "shell.parser_timeout";
pub const MESSAGE_PARSER_TIMEOUT: &str =
    "The host shell parser timed out or was canceled; retry the call.";

/// The syntax-error refusal message with the parser's own error interpolated
/// (Go `bash_delete.go:74,86`).
pub fn syntax_error_message(err: &str) -> String {
    format!("Fix the shell syntax and retry: {err}")
}

/// Builds a delete-guard refusal: the shared prefix plus the analysis message.
pub fn delete_refusal(code: &str, message: &str) -> Refusal {
    Refusal::code(code, format!("{DELETE_REFUSAL_PREFIX}{message}"))
}

// ---------------------------------------------------------------------------
// Recursive-delete extent analysis (shellsafe delete.go / delete_options.go).
// The argv-level classifier, option and literal-target decisions land here; the
// shellparse AST walk, nested-interpreter recursion, PowerShell host parser and
// realPath target bounding are the host-machinery slice's gap.
// ---------------------------------------------------------------------------

/// shellsafe `DeleteExtent`: one recursive delete call's reach.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeleteExtent {
    pub targets: Vec<String>,
    pub recursive: bool,
    pub unknown_option: bool,
    pub dynamic_command: bool,
}

/// The static program name, taking the last path segment (shellsafe
/// `ExecutableBase`).
fn executable_base(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

/// deleteDirectoryChange: the directory-change wrappers that may precede a
/// recursive delete.
pub fn delete_directory_change(name: &str) -> bool {
    matches!(
        executable_base(name).to_ascii_lowercase().as_str(),
        "cd" | "set-location" | "sl" | "pushd" | "push-location"
    )
}

/// literalDeletePath: non-empty, no expansion/glob/pipeline characters, and a
/// tilde accepted only inside a Windows 8.3 short-name segment.
pub fn literal_delete_path(target: &str) -> bool {
    !target.is_empty()
        && !target.chars().any(|c| {
            matches!(
                c,
                '$' | '%' | '*' | '?' | '[' | ']' | '{' | '}' | '`' | '\u{0}'
            )
        })
        && tilde_is_literal(target)
}

/// tildeIsLiteral: a tilde expands to the home directory everywhere except
/// inside a `SHORT~1` 8.3 segment of a drive-letter path.
pub fn tilde_is_literal(target: &str) -> bool {
    if !target.contains('~') {
        return true;
    }
    let bytes = target.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' {
        return false;
    }
    if bytes[2] != b'\\' && bytes[2] != b'/' {
        return false;
    }
    for segment in target[3..].split(['/', '\\']) {
        if segment.contains('~') && !short_name_segment(segment) {
            return false;
        }
    }
    true
}

/// The 8.3 short-name shape `SHORT~1` (shortNameSegment regex).
fn short_name_segment(segment: &str) -> bool {
    let mut parts = segment.split('~');
    let Some(head) = parts.next() else {
        return false;
    };
    let Some(tail) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && !head.is_empty()
        && !head.contains(['\\', '/'])
        && !head.contains('~')
        && !tail.is_empty()
        && tail.bytes().all(|b| b.is_ascii_digit())
}

/// AnalyzeRecursiveDelete: classifies one command into its delete extent.
/// `powershell` selects the PowerShell/PowerShell-CMD option vocabulary.
pub fn analyze_recursive_delete(name: &str, args: &[String], powershell: bool) -> DeleteExtent {
    let base = executable_base(name).to_ascii_lowercase();
    match base.as_str() {
        "cmd" => cmd_delete(args),
        "find" => find_delete(args),
        "xargs" => DeleteExtent::default(),
        "rm" | "remove-item" | "ri" | "del" | "erase" | "rd" | "rmdir" | "" => {
            direct_delete(&base, args, powershell)
        }
        _ => DeleteExtent::default(),
    }
}

fn direct_delete(base: &str, args: &[String], powershell: bool) -> DeleteExtent {
    let mut extent = DeleteExtent::default();
    let mut options = true;
    let ps_parameters =
        powershell && matches!(base, "remove-item" | "ri" | "rm" | "del" | "erase" | "");
    let cmd_parameters = powershell && matches!(base, "del" | "erase" | "rd" | "rmdir");

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if options && arg == "--" {
            options = false;
            i += 1;
            continue;
        }
        if options && arg.starts_with('-') {
            if ps_parameters {
                let consumed = powershell_delete_option(&mut extent, arg, &args[i + 1..]);
                i += consumed;
            } else {
                posix_delete_option(&mut extent, arg);
            }
            i += 1;
            continue;
        }
        if cmd_parameters && arg.starts_with('/') {
            cmd_delete_option(&mut extent, arg);
            i += 1;
            continue;
        }
        extent.targets.push(arg.clone());
        i += 1;
    }

    if !extent.recursive {
        return DeleteExtent::default();
    }
    if extent.targets.is_empty() {
        extent.targets.push(String::new());
    }
    extent
}

fn powershell_delete_option(extent: &mut DeleteExtent, arg: &str, rest: &[String]) -> usize {
    let stripped = arg.strip_prefix('-').unwrap_or(arg).to_ascii_lowercase();
    let (key, value) = match stripped.split_once(':') {
        Some((k, v)) => (k.to_owned(), Some(v.to_owned())),
        None => (stripped, None),
    };
    match key.as_str() {
        k if !k.is_empty() && k.starts_with("recurse") => {
            extent.recursive |= value.as_deref() != Some("$false");
        }
        "force" | "literalpath" | "path" => {}
        "erroraction" | "ea" | "warningaction" | "wa" | "informationaction" | "confirm" => {
            if value.is_none() {
                if rest.first().is_some_and(|r| !r.starts_with('-')) {
                    return 1;
                }
                extent.unknown_option = true;
            }
        }
        "verbose" | "debug" | "whatif" => {}
        _ => extent.unknown_option = true,
    }
    0
}

fn posix_delete_option(extent: &mut DeleteExtent, arg: &str) {
    if arg.starts_with("--") {
        if arg.starts_with("--recursive") {
            extent.recursive = true;
        } else if arg != "--force" && arg != "--verbose" && arg != "--dir" {
            extent.unknown_option = true;
        }
        return;
    }
    extent.recursive |= arg[1..].contains(['r', 'R']);
    extent.unknown_option |= !arg[1..]
        .trim_matches(|c: char| "rfRvidI".contains(c))
        .is_empty();
}

fn cmd_delete_option(extent: &mut DeleteExtent, arg: &str) {
    match arg.to_ascii_lowercase().as_str() {
        "/s" => extent.recursive = true,
        "/q" | "/f" => {}
        _ => extent.unknown_option = true,
    }
}

fn cmd_delete(args: &[String]) -> DeleteExtent {
    let Some(first) = args.first() else {
        return DeleteExtent::default();
    };
    if !first.eq_ignore_ascii_case("/c") {
        return DeleteExtent::default();
    }
    let rest: Vec<String> = if args.len() == 2 {
        args[1].split_whitespace().map(str::to_owned).collect()
    } else {
        args[1..].to_vec()
    };
    if rest.is_empty() {
        return DeleteExtent::default();
    }
    let recursive = rest.iter().any(|a| a.eq_ignore_ascii_case("/s"));
    let complex = rest
        .iter()
        .any(|a| a.contains(['^', '%', '&', '|', '\r', '\n']));
    if recursive && complex {
        return DeleteExtent {
            recursive: true,
            dynamic_command: true,
            ..Default::default()
        };
    }
    analyze_recursive_delete(&rest[0], &rest[1..], true)
}

fn find_delete(args: &[String]) -> DeleteExtent {
    let mut extent = DeleteExtent::default();
    let mut targets: Vec<String> = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        match arg.as_str() {
            "-delete" => extent.recursive = true,
            "-follow" | "-L" => extent.unknown_option = true,
            "-exec" | "-execdir" => {
                // Nested `-exec` delete target extraction is the parser slice's gap.
                if i + 1 >= args.len() {
                    continue;
                }
            }
            _ => {
                if !arg.starts_with('-') {
                    targets.push(arg.clone());
                }
            }
        }
    }
    if targets.is_empty() {
        targets.push(".".to_owned());
    }
    extent.targets = targets;
    extent
}

// ---------------------------------------------------------------------------
// Sandbox-unavailable identities (Go `sandbox.go:200-221`, `win_faults.go`).
// Fault detection and the per-fault remediation sentences live in the safety
// crate / OS-sandbox wave; the codes are the classification a caller matches
// on, pinned here with their platform group.
// ---------------------------------------------------------------------------

/// Linux: bubblewrap cannot give the command its own network namespace.
pub const CODE_BWRAP_NETWORK_DENIED: &str = "sandbox.bwrap_network_denied";
/// Linux: bubblewrap cannot create a namespace here.
pub const CODE_BWRAP_NAMESPACE_DENIED: &str = "sandbox.bwrap_namespace_denied";
/// Linux: bubblewrap could not be started at all.
pub const CODE_BWRAP_UNRUNNABLE: &str = "sandbox.bwrap_unrunnable";
/// Linux default: no usable bubblewrap on PATH.
pub const CODE_NO_BUBBLEWRAP: &str = "sandbox.no_bubblewrap";
/// macOS: `sandbox-exec` is missing or unusable.
pub const CODE_NO_SANDBOX_EXEC: &str = "sandbox.no_sandbox_exec";
/// Windows: the grant record exists and cannot be trusted.
pub const CODE_WIN_GRANT_UNREADABLE: &str = "sandbox.win_grant_unreadable";
/// Windows: the host refused to change a root's permissions.
pub const CODE_WIN_GRANT_DENIED: &str = "sandbox.win_grant_denied";
/// Windows: the session has no interactive logon SID to restrict with.
pub const CODE_WIN_NO_LOGON_SID: &str = "sandbox.win_no_logon_sid";
/// Windows: the grant record sits inside a writable folder.
pub const CODE_WIN_RECORD_EXPOSED: &str = "sandbox.win_record_exposed";
/// Windows: the restricted token or job could not be made.
pub const CODE_WIN_LAUNCHER: &str = "sandbox.win_launcher";
/// Windows: enforcement was asked for and nothing prepared the launch.
pub const CODE_WIN_NOT_PREPARED: &str = "sandbox.win_not_prepared";
/// Windows fallback: a fault the code table does not name.
pub const CODE_WIN_UNSUPPORTED: &str = "sandbox.unsupported_on_windows";
/// Other platform: the OS has no sandbox backend.
pub const CODE_UNSUPPORTED_PLATFORM: &str = "sandbox.unsupported_platform";

/// The platform-default unavailable code, when no finer fault is known (Go
/// `unavailableCode`'s fallbacks). Fault-specific codes are selected by the
/// safety crate's fault type once that lands.
pub fn default_unavailable_code(platform: &str) -> &'static str {
    match platform {
        "linux" => CODE_NO_BUBBLEWRAP,
        "darwin" => CODE_NO_SANDBOX_EXEC,
        "windows" => CODE_WIN_UNSUPPORTED,
        _ => CODE_UNSUPPORTED_PLATFORM,
    }
}

// ---------------------------------------------------------------------------
// Linked Git metadata (Go `bash_gitmeta.go:13-22`, `git_metadata_groups.go`).
// The path-token matching rides with the OS-sandbox wave; the refusal identity
// is pinned here.
// ---------------------------------------------------------------------------

/// `sandbox.git_metadata_linked`: the command reaches Git configuration or
/// hooks the host's own git reads, so the sandbox keeps them read-only.
pub const CODE_GIT_METADATA_LINKED: &str = "sandbox.git_metadata_linked";

/// Sentinel for a launch that touches linked Git metadata (Go
/// `ErrGitMetadataLinked = errors.New(GitMetadataLinkedCode)`). The error
/// text is the code itself; there is no separate model-facing sentence.
#[derive(Debug, Clone, Default)]
pub struct GitMetadataLinkedError;

impl std::fmt::Display for GitMetadataLinkedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(CODE_GIT_METADATA_LINKED)
    }
}

impl std::error::Error for GitMetadataLinkedError {}

// ---------------------------------------------------------------------------
// Session-temp lease failure (Go `sessiontemp_error.go:8`). The Acquire/Release
// lease lifecycle is the session-temp slice's machine; the failure phase this
// error maps to is Launch, distinguished from the Preflight authorization
// refusals.
// ---------------------------------------------------------------------------

/// The failure phase a session-temp acquisition failure lands on.
pub const SESSION_TEMP_PHASE: ShellPhase = ShellPhase::Launch;

/// A session-private temporary directory could not be acquired.
#[derive(Debug)]
pub struct SessionTempError {
    message: String,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl SessionTempError {
    /// Wraps the underlying acquisition failure (Go `errSessionTemp`).
    pub fn wrap(source: Box<dyn std::error::Error + Send + Sync>) -> Self {
        Self {
            message: source.to_string(),
            source: Some(source),
        }
    }
}

impl std::fmt::Display for SessionTempError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SessionTempError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|s| s.as_ref() as &(dyn std::error::Error + 'static))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_flattens_newlines_and_truncates_at_forty_eight_runes() {
        assert_eq!(command_preview("echo hi"), "echo hi");
        assert_eq!(command_preview("  ls -la\n/path  "), "ls -la /path");
        let long = "x".repeat(60);
        let got = command_preview(&long);
        assert_eq!(got.len(), "x".repeat(48).len() + "…".len());
        assert!(got.ends_with("…"));
        // A multibyte rune boundary is counted in runes, not bytes.
        let emoji = "😀".repeat(60);
        let got = command_preview(&emoji);
        assert!(got.starts_with("😀".repeat(48).as_str()));
        assert!(got.ends_with("…"));
    }

    #[test]
    fn keepalive_needs_both_background_and_a_wrapper() {
        assert!(has_explicit_background_keepalive("nohup sleep 120 &"));
        assert!(has_explicit_background_keepalive("setsid firefox &"));
        assert!(!has_explicit_background_keepalive("sleep 120 &"));
        assert!(!has_explicit_background_keepalive("nohup sleep 120"));
        assert!(!has_explicit_background_keepalive("echo 'disown &'"));
        assert!(!has_explicit_background_keepalive(""));
    }

    #[test]
    fn track_and_reap_follow_the_same_preserve_rules() {
        let bash = ShellKind::Bash;
        assert!(!should_track_shell_process(false, bash, "nohup x &", false));
        assert!(!should_reap_after_run(false, bash, "nohup x &", false));
        assert!(should_track_shell_process(false, bash, "echo x", false));
        assert!(should_reap_after_run(false, bash, "echo x", false));
        assert!(!should_track_shell_process(false, bash, "echo x", true));
        assert!(!should_reap_after_run(false, bash, "echo x", true));
        assert!(should_reap_after_run(true, bash, "echo x", false));
        #[cfg(windows)]
        assert!(!should_track_shell_process(true, bash, "echo x", false));
        #[cfg(not(windows))]
        assert!(should_track_shell_process(true, bash, "echo x", false));
        assert!(suppress_wait_delay_error(true, false, true));
        assert!(!suppress_wait_delay_error(true, true, true));
        assert!(!suppress_wait_delay_error(false, false, true));
    }

    #[test]
    fn start_message_names_the_id_in_each_place() {
        let msg = background_start_message("bash-1");
        assert_eq!(
            msg,
            "Started background job \"bash-1\". It keeps running across turns; read new output \
             with bash_output(job_id=\"bash-1\"), wait for it with wait, or stop it with \
             kill_shell(job_id=\"bash-1\")."
        );
    }

    #[test]
    fn progress_suppression_needs_an_active_marker() {
        let marker = "__rx_pipestatus_abc";
        assert!(progress_line_suppressed(
            marker,
            "  __rx_pipestatus_abc 0 1"
        ));
        assert!(!progress_line_suppressed(marker, "go test output"));
        assert!(!progress_line_suppressed("", "__rx_pipestatus_abc 0 1"));
    }

    #[test]
    fn delete_refusal_carries_the_shared_prefix_and_code() {
        let r = delete_refusal(CODE_DELETE_SEQUENCE, MESSAGE_DELETE_SEQUENCE);
        assert_eq!(r.code, CODE_DELETE_SEQUENCE);
        assert!(r.message.starts_with(DELETE_REFUSAL_PREFIX));
        assert!(r.message.contains("only literal directory changes"));
        assert_eq!(
            syntax_error_message("near \";\""),
            "Fix the shell syntax and retry: near \";\""
        );
    }

    #[test]
    fn analyze_rm_rf_collects_the_target_and_known_flags() {
        let extent =
            analyze_recursive_delete("rm", &["-rf".to_owned(), "src/old".to_owned()], false);
        assert!(extent.recursive);
        assert!(!extent.unknown_option);
        assert_eq!(extent.targets, vec!["src/old".to_owned()]);
    }

    #[test]
    fn an_unknown_delete_option_is_flagged() {
        // A non-recursive call returns an empty extent per shellsafe; the
        // unknown-option flag only survives on a recursive delete.
        let extent = analyze_recursive_delete("rm", &["-rz".to_owned(), "x".to_owned()], false);
        assert!(extent.recursive);
        assert!(extent.unknown_option);
    }

    #[test]
    fn powershell_remove_item_recurses_on_recurse_flag() {
        let extent = analyze_recursive_delete(
            "Remove-Item",
            &["-Recurse".to_owned(), "C:\\data\\old".to_owned()],
            true,
        );
        assert!(extent.recursive);
        assert_eq!(extent.targets, vec!["C:\\data\\old".to_owned()]);
    }

    #[test]
    fn literal_targets_reject_expansions_globs_and_home_tildes() {
        assert!(!literal_delete_path("$VAR/old"));
        assert!(!literal_delete_path("*"));
        assert!(!literal_delete_path("~/old"));
        assert!(!literal_delete_path("a[b]"));
        assert!(literal_delete_path("src/old"));
        // A Windows 8.3 short-name segment keeps the tilde literal.
        assert!(literal_delete_path(r"C:\PROGRA~1\Old"));
    }

    #[test]
    fn directory_change_wrappers_are_recognized() {
        assert!(delete_directory_change("cd"));
        assert!(delete_directory_change("pushd"));
        assert!(delete_directory_change("Set-Location"));
        assert!(!delete_directory_change("rm"));
    }

    #[test]
    fn find_delete_recurses_on_delete_flag() {
        let extent = find_delete(&["build".to_owned(), "-delete".to_owned()]);
        assert!(extent.recursive);
        assert_eq!(extent.targets, vec!["build".to_owned()]);
    }

    #[test]
    fn unavailable_codes_keep_their_platform_groups() {
        assert_eq!(default_unavailable_code("linux"), CODE_NO_BUBBLEWRAP);
        assert_eq!(default_unavailable_code("darwin"), CODE_NO_SANDBOX_EXEC);
        assert_eq!(default_unavailable_code("windows"), CODE_WIN_UNSUPPORTED);
        assert_eq!(
            default_unavailable_code("freebsd"),
            CODE_UNSUPPORTED_PLATFORM
        );
    }

    #[test]
    fn git_metadata_error_text_is_the_code() {
        let err = GitMetadataLinkedError;
        assert_eq!(err.to_string(), CODE_GIT_METADATA_LINKED);
    }

    #[test]
    fn snip_hint_defaults_match_go() {
        let hint = BashSnipHint::default();
        assert_eq!(hint.head, BASH_SNIP_HEAD);
        assert_eq!(hint.tail, BASH_SNIP_TAIL);
        assert_eq!(hint.head_chars, BASH_SNIP_HEAD_CHARS);
        assert_eq!(hint.tail_chars, BASH_SNIP_TAIL_CHARS);
    }
}
