//! Pure parse-route-render layer: `dispatch` normalizes argv, routes it, and
//! renders the deterministic pre-handler outcome (stdout text, stderr text,
//! exit code, parsed flag structure). No handler here opens a config, a
//! provider, or a terminal; commands whose kernel slice has not landed yet
//! route to [`Route::Unmigrated`] with a typed marker instead of a half-built
//! body.
//!
//! TTY and console facts come through [`CliEnv`] so tests inject fakes instead
//! of probing real stdio; the real probes live in [`CliEnv::detected`].

use std::io::IsTerminal as _;

use crate::build_info::BuildInfo;
use crate::flags::{self, ParsedFlags, normalize_optional_resume_arg, parse, run_specs, tui_specs};
use crate::permission::parse_permission_mode;
use crate::text::*;

/// Injectable process-environment facts the routing decision depends on.
#[derive(Debug, Clone)]
pub struct CliEnv {
    /// stdin is a terminal (reference `term.IsTerminal(os.Stdin.Fd())`).
    pub stdin_is_tty: bool,
    /// stdout is a terminal (reference `term.IsTerminal(os.Stdout.Fd())`).
    pub stdout_is_tty: bool,
    /// Either end is a Cygwin/MSYS pty (mintty / Git Bash).
    pub mintty: bool,
    /// This process owns the console alone (Windows double-click).
    pub owns_console_alone: bool,
    /// The host started this process for an agent tool.
    pub agent_child: bool,
    /// Piped stdin text; empty when stdin is a terminal or closed.
    pub stdin_text: String,
}

impl CliEnv {
    /// Reads the real process environment. Console-owner and mintty probes are
    /// Windows-specific; on non-Windows platforms the reference returns false,
    /// which this mirrors until the Windows console slice lands.
    pub fn detected() -> Self {
        let agent_child = std::env::var(AGENT_CHILD_ENV)
            .map(|v| v == "1")
            .unwrap_or(false);
        Self {
            stdin_is_tty: std::io::stdin().is_terminal(),
            stdout_is_tty: std::io::stdout().is_terminal(),
            mintty: false,
            owns_console_alone: false,
            agent_child,
            stdin_text: String::new(),
        }
    }

    /// Both streams are terminals: the interactive branch.
    pub fn tty() -> Self {
        Self {
            stdin_is_tty: true,
            stdout_is_tty: true,
            mintty: false,
            owns_console_alone: false,
            agent_child: false,
            stdin_text: String::new(),
        }
    }

    /// Neither stream is a terminal: scripts and pipes.
    pub fn pipe() -> Self {
        Self {
            stdin_is_tty: false,
            stdout_is_tty: false,
            mintty: false,
            owns_console_alone: false,
            agent_child: false,
            stdin_text: String::new(),
        }
    }
}

/// Where a normalized argv landed. The parsed flags travel with the routes the
/// kernel will consume once their handler slice lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Bare invocation with both streams a terminal: straight into the TUI.
    BareTui,
    /// Bare invocation outside a console, after the usage body.
    BareUsage { bare: bool },
    /// Mintty / Cygwin pty: the mintty hint replaces the usage body.
    BareMintty,
    /// `tui` / `chat` / `code` (or bare interactive flags) parsed as TUI flags.
    Tui {
        args: Vec<String>,
        flags: ParsedFlags,
    },
    /// `run` (or a leading-flag reroute) parsed as run flags.
    Run {
        args: Vec<String>,
        flags: ParsedFlags,
    },
    /// `--acp` / `acp`.
    Acp,
    /// `init` hint.
    Init,
    /// `help` / `--help` / `-h`.
    Help,
    /// `version` / `--version` / `-v`.
    Version,
    /// Hidden `schedule` verb (`exec` included).
    Schedule { verb: String },
    /// Hidden `sandbox` verb.
    Sandbox { verb: String },
    /// Command the dispatch table does not know.
    Unknown { command: String },
    /// Known command whose kernel handler has not been migrated yet.
    Unmigrated { command: String },
}

/// Structured result of dispatching one argv slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOutcome {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
    pub route: Route,
}

impl CliOutcome {
    fn new(stdout: String, stderr: String, code: i32, route: Route) -> Self {
        Self {
            stdout,
            stderr,
            code,
            route,
        }
    }
}

/// Top-level dispatch: the pure mirror of `RunWithBuildInfo`.
pub fn dispatch(args: &[String], info: &BuildInfo, env: &CliEnv) -> CliOutcome {
    let (cmd, normalized) = flags::normalize_command(args);
    if normalized.is_empty() || cmd.is_empty() {
        return bare_invocation(&normalized, env);
    }
    match cmd.as_str() {
        "run" => run_command(&normalized[1..], env),
        "serve" => unmigrated("serve"),
        "tui" | "chat" | "code" => tui_command(&normalized[1..], env),
        "web" => unmigrated("web"),
        "setup" => unmigrated("setup"),
        "config" => unmigrated("config"),
        "init" => init_hint(),
        "acp" => CliOutcome::new(String::new(), String::new(), 2, Route::Acp),
        "mcp" => unmigrated("mcp"),
        "login" | "whoami" | "logout" => unmigrated(&cmd),
        "remote" => unmigrated("remote"),
        "plugin" => unmigrated("plugin"),
        "subagent" => unmigrated("subagent"),
        "doctor" => unmigrated("doctor"),
        "report" => unmigrated("report"),
        "session" | "sessions" | "catalogs" => unmigrated(&cmd),
        "hook" | "hooks" => unmigrated(&cmd),
        "trust" => unmigrated("trust"),
        "task" => unmigrated("task"),
        "review" => unmigrated("review"),
        "upgrade" | "update" => unmigrated(&cmd),
        "version" => version_command(&normalized[1..], info, true),
        "--version" | "-v" => version_command(&[], info, false),
        "completion" => unmigrated("completion"),
        "docs-manifest" => unmigrated("docs-manifest"),
        "help" | "--help" | "-h" => help_command(),
        "schedule" => schedule_command(&normalized[1..], env),
        "sandbox" => sandbox_command(&normalized[1..]),
        other => unknown_command(other),
    }
}

/// `reasonix` with no verb: both terminals start the TUI; otherwise the bare
/// outside-console rules (bare exit 0, session flags exit 2, mintty hint).
fn bare_invocation(args: &[String], env: &CliEnv) -> CliOutcome {
    if env.stdin_is_tty && env.stdout_is_tty {
        return CliOutcome::new(String::new(), unmigrated_notice("tui"), 2, Route::BareTui);
    }
    if env.mintty {
        return CliOutcome::new(
            String::new(),
            MINTTY_CONSOLE_HINT.to_owned(),
            2,
            Route::BareMintty,
        );
    }
    let bare = args.is_empty();
    let mut stdout = USAGE_BODY.to_owned();
    if env.owns_console_alone {
        stdout.push('\n');
        stdout.push_str(STANDALONE_CONSOLE_HINT);
    }
    let code = if bare { 0 } else { 2 };
    CliOutcome::new(stdout, String::new(), code, Route::BareUsage { bare })
}

fn help_command() -> CliOutcome {
    CliOutcome::new(USAGE_BODY.to_owned(), String::new(), 0, Route::Help)
}

fn init_hint() -> CliOutcome {
    CliOutcome::new(format!("{INIT_HINT}\n"), String::new(), 0, Route::Init)
}

fn unknown_command(cmd: &str) -> CliOutcome {
    let stderr = format!("unknown command {:?}\n\n", cmd);
    CliOutcome::new(
        USAGE_BODY.to_owned(),
        stderr,
        2,
        Route::Unknown {
            command: cmd.to_owned(),
        },
    )
}

fn unmigrated(cmd: &str) -> CliOutcome {
    CliOutcome::new(
        String::new(),
        unmigrated_notice(cmd),
        2,
        Route::Unmigrated {
            command: cmd.to_owned(),
        },
    )
}

fn unmigrated_notice(cmd: &str) -> String {
    format!("reasonix: command not implemented in this build: {cmd}\n")
}

/// `reasonix version [flags]`; `allow_flags` is false for the top-level
/// `--version`/`-v`, which stay single-line and parse nothing.
fn version_command(rest: &[String], info: &BuildInfo, allow_flags: bool) -> CliOutcome {
    let info = info.with_defaults();
    if !allow_flags {
        return CliOutcome::new(
            format!("{}\n", info.single_line()),
            String::new(),
            0,
            Route::Version,
        );
    }

    let mut verbose = false;
    let mut json_out = false;
    for arg in rest {
        match arg.as_str() {
            "--verbose" | "-verbose" => verbose = true,
            "--json" => json_out = true,
            "--help" | "-h" => {
                return CliOutcome::new(VERSION_USAGE.to_owned(), String::new(), 0, Route::Version);
            }
            other => {
                let mut stderr = format!("unknown version flag {:?}\n", other);
                stderr.push_str(VERSION_USAGE);
                return CliOutcome::new(String::new(), stderr, 2, Route::Version);
            }
        }
    }

    if json_out && verbose {
        return CliOutcome::new(
            String::new(),
            format!("{ERROR_PREFIX} --json and --verbose are mutually exclusive\n"),
            2,
            Route::Version,
        );
    }
    if json_out {
        return CliOutcome::new(info.json_payload(), String::new(), 0, Route::Version);
    }
    if verbose {
        return CliOutcome::new(info.verbose_text(), String::new(), 0, Route::Version);
    }
    CliOutcome::new(
        format!("{}\n", info.single_line()),
        String::new(),
        0,
        Route::Version,
    )
}

/// `reasonix tui|chat|code [flags]`. Flag parsing and the non-terminal guard
/// are deterministic; the interactive session itself is the unmigrated body.
fn tui_command(rest: &[String], env: &CliEnv) -> CliOutcome {
    let normalized = normalize_optional_resume_arg(rest);
    let flags = parse(&tui_specs(), &normalized, true);
    let route = Route::Tui {
        args: normalized.clone(),
        flags: flags.clone(),
    };
    if flags.help {
        return CliOutcome::new(String::new(), String::new(), 0, route);
    }
    if let Some(err) = &flags.error {
        return CliOutcome::new(String::new(), format!("{ERROR_PREFIX} {err}\n"), 2, route);
    }
    if !(env.stdin_is_tty && env.stdout_is_tty) {
        if env.mintty {
            return CliOutcome::new(String::new(), MINTTY_CONSOLE_HINT.to_owned(), 2, route);
        }
        return CliOutcome::new(
            String::new(),
            format!("{ERROR_PREFIX} {TUI_NEEDS_TERMINAL}\n"),
            2,
            route,
        );
    }
    CliOutcome::new(String::new(), unmigrated_notice("tui"), 2, route)
}

/// `reasonix run [flags] <task>`. The decidable pre-handler checks run here:
/// flag parsing, approval conflicts, output format, events-jsonl combination,
/// permission mode, plan refusal, and the empty-prompt hint. Session, lease,
/// boot, and completion-classification outcomes belong to the unmigrated body.
fn run_command(rest: &[String], env: &CliEnv) -> CliOutcome {
    let flags = parse(&run_specs(), rest, true);
    let route = Route::Run {
        args: rest.to_vec(),
        flags: flags.clone(),
    };
    if flags.help {
        return CliOutcome::new(String::new(), String::new(), 0, route);
    }
    if let Some(err) = &flags.error {
        return CliOutcome::new(String::new(), format!("{ERROR_PREFIX} {err}\n"), 2, route);
    }

    let auto = flags.bool("auto");
    let yolo = flags.bool("yolo");
    let mode_explicit = flags.changed("permission-mode");
    let conflict = if auto && yolo {
        Some(AUTO_YOLO_CONFLICT)
    } else if auto && mode_explicit {
        Some(AUTO_PERMISSION_CONFLICT)
    } else if yolo && mode_explicit {
        Some(YOLO_PERMISSION_CONFLICT)
    } else {
        None
    };
    if let Some(msg) = conflict {
        return CliOutcome::new(String::new(), format!("{ERROR_PREFIX} {msg}\n"), 2, route);
    }

    let output_format = flags.value("output-format").unwrap_or("text");
    if flags.changed("output-format") && !matches!(output_format, "text" | "json" | "stream-json") {
        return CliOutcome::new(
            String::new(),
            format!(
                "{ERROR_PREFIX} unknown output format {:?} (want text, json, or stream-json)\n",
                output_format
            ),
            2,
            route,
        );
    }

    if flags.bool("events-jsonl") && flags.changed("output-format") {
        return CliOutcome::new(
            String::new(),
            format!("{ERROR_PREFIX} {EVENTS_JSONL_CONFLICT}\n"),
            2,
            route,
        );
    }

    let mode_raw = flags.value("permission-mode").unwrap_or("");
    let mode = match parse_permission_mode(mode_raw) {
        Ok(mode) => mode,
        Err(err) => {
            return CliOutcome::new(String::new(), format!("{ERROR_PREFIX} {err}\n"), 2, route);
        }
    };
    if mode.plan {
        return CliOutcome::new(
            String::new(),
            format!("{ERROR_PREFIX} {PLAN_REQUIRES_INTERACTIVE}\n"),
            2,
            route,
        );
    }

    if flags.positionals.is_empty() && env.stdin_text.trim().is_empty() {
        return CliOutcome::new(String::new(), format!("{USAGE_RUN_HINT}\n"), 2, route);
    }

    CliOutcome::new(String::new(), unmigrated_notice("run"), 2, route)
}

/// Hidden `reasonix schedule`. The human-guard decisions are deterministic and
/// routed here; the store-backed verbs and the `add` confirmation flow belong to
/// the unmigrated scheduler slice.
fn schedule_command(rest: &[String], env: &CliEnv) -> CliOutcome {
    let Some(verb) = rest.first() else {
        return CliOutcome::new(
            String::new(),
            format!("{SCHEDULE_USAGE}\n"),
            2,
            Route::Schedule {
                verb: String::new(),
            },
        );
    };

    if verb == "exec" {
        return CliOutcome::new(
            String::new(),
            unmigrated_notice("schedule exec"),
            2,
            Route::Schedule { verb: verb.clone() },
        );
    }

    const HUMAN_VERBS: [&str; 5] = ["resume-all", "resume", "renew", "rm", "add"];
    const KNOWN_VERBS: [&str; 9] = [
        "list",
        "tick",
        "pause-all",
        "pause",
        "resume-all",
        "resume",
        "renew",
        "rm",
        "add",
    ];
    if !KNOWN_VERBS.contains(&verb.as_str()) {
        return CliOutcome::new(
            String::new(),
            format!("{SCHEDULE_USAGE}\n"),
            2,
            Route::Schedule { verb: verb.clone() },
        );
    }

    // One-id verbs take exactly one rest argument (reference scheduleOne).
    if matches!(verb.as_str(), "pause" | "resume" | "renew" | "rm") && rest.len() != 2 {
        return CliOutcome::new(
            String::new(),
            format!("{SCHEDULE_USAGE}\n"),
            2,
            Route::Schedule { verb: verb.clone() },
        );
    }

    if HUMAN_VERBS.contains(&verb.as_str()) {
        let refused = if env.agent_child {
            Some(format!(
                "schedule {verb} is refused: this command was started by an agent tool ({AGENT_CHILD_ENV} is set)"
            ))
        } else if !(env.stdin_is_tty && env.stdout_is_tty) {
            Some(format!(
                "schedule {verb} needs a person at a terminal to confirm it"
            ))
        } else {
            None
        };
        if let Some(msg) = refused {
            return CliOutcome::new(
                String::new(),
                format!("{msg}\n"),
                77,
                Route::Schedule { verb: verb.clone() },
            );
        }
    }

    CliOutcome::new(
        String::new(),
        unmigrated_notice(&format!("schedule {verb}")),
        2,
        Route::Schedule { verb: verb.clone() },
    )
}

/// Hidden `reasonix sandbox list|revoke` (Windows). The grant store is the
/// unmigrated body; only the verb table and usage error are decided here.
fn sandbox_command(rest: &[String]) -> CliOutcome {
    let verb = rest.first().map(String::as_str).unwrap_or_default();
    if matches!(verb, "list" | "revoke") && rest.len() == 1 {
        return CliOutcome::new(
            String::new(),
            unmigrated_notice(&format!("sandbox {verb}")),
            2,
            Route::Sandbox {
                verb: verb.to_owned(),
            },
        );
    }
    CliOutcome::new(
        String::new(),
        format!("{SANDBOX_USAGE}\n"),
        2,
        Route::Sandbox {
            verb: verb.to_owned(),
        },
    )
}
