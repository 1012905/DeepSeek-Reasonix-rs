//! Machine interfaces (`session`, `hook`, `task`), the `sessions` 1.x
//! maintenance verbs, and `catalogs reindex`.
//!
//! The reference builds these around `sessionstore`/`jobs`/`hook` backends;
//! this layer decides the operation matrix, option validation, error
//! identities, exit codes, and the wire JSON shape. Record payloads render from
//! empty collections until the store anchors migrate; tests assert envelopes,
//! field names/order, and redaction boundaries, not backend data.

use crate::flags::ParsedFlags;
use crate::route::{CliOutcome, Route};
use crate::text::*;

/// Which machine interface a route carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineInterface {
    Session,
    Hook,
    Task,
}

impl MachineInterface {
    fn prefix(self) -> &'static str {
        match self {
            MachineInterface::Session => "session",
            MachineInterface::Hook => "hook",
            MachineInterface::Task => "task",
        }
    }
}

/// Parsed machine options shared by the three interfaces.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MachineOptions {
    pub json: bool,
    pub dir: String,
    pub project_root: String,
    pub session: String,
    pub home_dir: String,
    pub target: String,
}

/// A machine-interface validation failure: typed code plus human message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineError {
    pub code: String,
    pub message: String,
}

/// `writeMachineError` exit mapping: argument/unknown-command failures are
/// usage errors (2); every other code is a runtime failure (1).
pub(crate) fn machine_exit_code(code: &str) -> i32 {
    if code == "invalid_argument" || code == "unknown_command" {
        2
    } else {
        1
    }
}

fn invalid(message: &str) -> MachineError {
    MachineError {
        code: "invalid_argument".to_owned(),
        message: message.to_owned(),
    }
}

fn unknown(message: &str) -> MachineError {
    MachineError {
        code: "unknown_command".to_owned(),
        message: message.to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Minimal JSON writer (no serde in this crate).
// ---------------------------------------------------------------------------

fn escape_json(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Error envelope: `{schema_version, command, error:{code,message}}`.
pub(crate) fn error_envelope(command: &str, code: &str, message: &str) -> String {
    format!(
        "{{\"schema_version\":1,\"command\":\"{}\",\"error\":{{\"code\":\"{}\",\"message\":\"{}\"}}}}",
        escape_json(command),
        escape_json(code),
        escape_json(message)
    )
}

fn valid_machine_id(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && !trimmed.contains('/') && !trimmed.contains('\\')
}

// ---------------------------------------------------------------------------
// Option parsing per interface.
// ---------------------------------------------------------------------------

fn parse_options(
    kind: MachineInterface,
    args: &[String],
    operation: &str,
) -> Result<MachineOptions, MachineError> {
    let mut opts = MachineOptions::default();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--json" => opts.json = true,
            "--dir" | "--project-root" | "--home-dir" => {
                let value = match args.get(i + 1) {
                    Some(next) if !next.trim().is_empty() => next.clone(),
                    _ => {
                        let message = match (kind, arg) {
                            (MachineInterface::Hook, "--home-dir") => {
                                "home directory requires a value"
                            }
                            (MachineInterface::Hook, _) => "project root requires a value",
                            _ => "--dir requires a value",
                        };
                        return Err(invalid(message));
                    }
                };
                i += 1;
                match arg {
                    "--dir" => opts.dir = value,
                    "--project-root" => opts.project_root = value,
                    "--home-dir" => opts.home_dir = value,
                    _ => unreachable!(),
                }
            }
            "--session" => {
                let value = match args.get(i + 1) {
                    Some(next) if valid_machine_id(next) => next.clone(),
                    _ => return Err(invalid("--session requires a valid identifier")),
                };
                i += 1;
                opts.session = value;
            }
            "--help" | "-h" => return Err(invalid(MACHINE_HELP_HINT)),
            _ if arg.starts_with('-') => {
                let message = match kind {
                    MachineInterface::Session => "unknown session option",
                    MachineInterface::Hook => "unknown hook option",
                    MachineInterface::Task => "unknown task option",
                };
                return Err(invalid(message));
            }
            _ => {
                // Positional target. Hook has no target.
                if kind == MachineInterface::Hook {
                    return Err(invalid("unknown hook option"));
                }
                if !opts.target.is_empty() || !valid_machine_id(arg) {
                    let message = match kind {
                        MachineInterface::Session => "invalid session identifier",
                        _ => "invalid task identifier",
                    };
                    return Err(invalid(message));
                }
                opts.target = arg.to_owned();
            }
        }
        i += 1;
    }

    if !opts.dir.is_empty() && !opts.project_root.is_empty() {
        return Err(invalid("--dir and --project-root cannot be combined"));
    }
    // `--session` names the same target as the positional identifier.
    if opts.target.is_empty() && !opts.session.is_empty() {
        opts.target = opts.session.clone();
    }
    let requires_target = match kind {
        MachineInterface::Session => operation != "list" && operation != "recovery",
        MachineInterface::Task => operation == "show",
        MachineInterface::Hook => false,
    };
    if requires_target && opts.target.is_empty() {
        let message = match kind {
            MachineInterface::Session => "a session identifier is required",
            _ => "a task identifier is required",
        };
        return Err(invalid(message));
    }
    Ok(opts)
}

// ---------------------------------------------------------------------------
// Handlers.
// ---------------------------------------------------------------------------

fn machine_outcome(
    interface: MachineInterface,
    operation: &str,
    opts: MachineOptions,
    payload: Option<&str>,
    err: Option<MachineError>,
) -> CliOutcome {
    let command = format!("{}.{}", interface.prefix(), operation);
    if let Some(err) = err {
        let code = machine_exit_code(&err.code);
        return CliOutcome::new(
            error_envelope(&command, &err.code, &err.message),
            String::new(),
            code,
            Route::Machine {
                interface,
                operation: operation.to_owned(),
                options: opts,
            },
        );
    }
    let stdout = payload.unwrap_or_default().to_owned();
    CliOutcome::new(
        stdout,
        String::new(),
        0,
        Route::Machine {
            interface,
            operation: operation.to_owned(),
            options: opts,
        },
    )
}

/// `reasonix session list|show|status|recovery`.
pub(crate) fn session_command(rest: &[String]) -> CliOutcome {
    let Some(op) = rest.first() else {
        return machine_outcome(
            MachineInterface::Session,
            "",
            MachineOptions::default(),
            None,
            Some(invalid("a session operation is required")),
        );
    };
    if !matches!(op.as_str(), "list" | "show" | "status" | "recovery") {
        return machine_outcome(
            MachineInterface::Session,
            op,
            MachineOptions::default(),
            None,
            Some(unknown("unknown session operation")),
        );
    }
    let opts = match parse_options(MachineInterface::Session, &rest[1..], op) {
        Ok(opts) => opts,
        Err(err) => {
            return machine_outcome(
                MachineInterface::Session,
                op,
                MachineOptions::default(),
                None,
                Some(err),
            );
        }
    };
    if !opts.json {
        return machine_outcome(
            MachineInterface::Session,
            op,
            opts,
            None,
            Some(invalid(MACHINE_JSON_REQUIRED)),
        );
    }
    let payload = match op.as_str() {
        "list" => "{\"schema_version\":1,\"command\":\"session.list\",\"sessions\":[]}",
        "recovery" => "{\"schema_version\":1,\"command\":\"session.recovery\",\"recoveries\":[]}",
        "show" | "status" => "{\"schema_version\":1,\"command\":\"session.show\",\"session\":null}",
        _ => unreachable!(),
    };
    machine_outcome(MachineInterface::Session, op, opts, Some(payload), None)
}

/// `reasonix hook list|status`.
pub(crate) fn hook_command(rest: &[String]) -> CliOutcome {
    let Some(op) = rest.first() else {
        return machine_outcome(
            MachineInterface::Hook,
            "",
            MachineOptions::default(),
            None,
            Some(invalid("a hook operation is required")),
        );
    };
    if !matches!(op.as_str(), "list" | "status") {
        return machine_outcome(
            MachineInterface::Hook,
            op,
            MachineOptions::default(),
            None,
            Some(unknown("unknown hook operation")),
        );
    }
    let opts = match parse_options(MachineInterface::Hook, &rest[1..], op) {
        Ok(opts) => opts,
        Err(err) => {
            return machine_outcome(
                MachineInterface::Hook,
                op,
                MachineOptions::default(),
                None,
                Some(err),
            );
        }
    };
    if !opts.json {
        return machine_outcome(
            MachineInterface::Hook,
            op,
            opts,
            None,
            Some(invalid(MACHINE_JSON_REQUIRED)),
        );
    }
    let payload = match op.as_str() {
        "list" => "{\"schema_version\":1,\"command\":\"hook.list\",\"hooks\":[]}",
        "status" => {
            "{\"schema_version\":1,\"command\":\"hook.status\",\"trusted_project\":false,\"project_defines\":false,\"sources\":[]}"
        }
        _ => unreachable!(),
    };
    machine_outcome(MachineInterface::Hook, op, opts, Some(payload), None)
}

const TASK_TOP_LEVEL: [&str; 12] = [
    "list",
    "show",
    "monitor",
    "machine-list",
    "machine-show",
    "status",
    "events",
    "stop",
    "cancel",
    "requeue",
    "open-session",
    "tmux",
];
const TASK_MONITOR: [&str; 7] = [
    "list",
    "status",
    "events",
    "stop",
    "cancel",
    "requeue",
    "open-session",
];
const TASK_TMUX: [&str; 4] = ["attach", "status", "open", "detach"];

/// `reasonix task ...`; machine-list/machine-show route to the task machine
/// interface, monitor/tmux validate their sub-matrices, the rest are
/// unmigrated store-backed verbs.
pub(crate) fn task_command(rest: &[String]) -> CliOutcome {
    let Some(top) = rest.first() else {
        return CliOutcome::new(
            String::new(),
            format!("{TASK_USAGE}\n"),
            2,
            Route::Verb {
                command: "task".into(),
                verb: String::new(),
                args: vec![],
                flags: ParsedFlags::default(),
            },
        );
    };
    match top.as_str() {
        "machine-list" => task_machine("list", &rest[1..]),
        "machine-show" => task_machine("show", &rest[1..]),
        "monitor" => {
            let sub = rest.get(1).map(String::as_str).unwrap_or_default();
            if sub.is_empty() {
                return CliOutcome::new(
                    String::new(),
                    format!("{TASK_MONITOR_USAGE}\n"),
                    2,
                    Route::Verb {
                        command: "task".into(),
                        verb: "monitor".into(),
                        args: rest[1..].to_vec(),
                        flags: ParsedFlags::default(),
                    },
                );
            }
            if !TASK_MONITOR.contains(&sub) {
                return CliOutcome::new(
                    String::new(),
                    format!("{}\n", TASK_MONITOR_UNKNOWN_SUBCOMMAND.replace("%s", sub)),
                    2,
                    Route::Verb {
                        command: "task".into(),
                        verb: "monitor".into(),
                        args: rest[1..].to_vec(),
                        flags: ParsedFlags::default(),
                    },
                );
            }
            CliOutcome::new(
                String::new(),
                crate::route::unmigrated_notice(&format!("task monitor {sub}")),
                2,
                Route::Verb {
                    command: "task".into(),
                    verb: "monitor".into(),
                    args: rest[1..].to_vec(),
                    flags: ParsedFlags::default(),
                },
            )
        }
        "tmux" => {
            let sub = rest.get(1).map(String::as_str).unwrap_or_default();
            if sub.is_empty() {
                return CliOutcome::new(
                    String::new(),
                    format!("{TASK_TMUX_USAGE}\n"),
                    2,
                    Route::Verb {
                        command: "task".into(),
                        verb: "tmux".into(),
                        args: rest[1..].to_vec(),
                        flags: ParsedFlags::default(),
                    },
                );
            }
            if !TASK_TMUX.contains(&sub) {
                return CliOutcome::new(
                    String::new(),
                    format!("{}\n", TASK_TMUX_UNKNOWN_SUBCOMMAND.replace("%s", sub)),
                    2,
                    Route::Verb {
                        command: "task".into(),
                        verb: "tmux".into(),
                        args: rest[1..].to_vec(),
                        flags: ParsedFlags::default(),
                    },
                );
            }
            CliOutcome::new(
                String::new(),
                crate::route::unmigrated_notice(&format!("task tmux {sub}")),
                2,
                Route::Verb {
                    command: "task".into(),
                    verb: "tmux".into(),
                    args: rest[1..].to_vec(),
                    flags: ParsedFlags::default(),
                },
            )
        }
        _ if TASK_TOP_LEVEL.contains(&top.as_str()) => CliOutcome::new(
            String::new(),
            crate::route::unmigrated_notice(&format!("task {top}")),
            2,
            Route::Verb {
                command: "task".into(),
                verb: top.clone(),
                args: rest[1..].to_vec(),
                flags: ParsedFlags::default(),
            },
        ),
        _ => CliOutcome::new(
            String::new(),
            format!("{}\n", TASK_UNKNOWN_SUBCOMMAND.replace("%s", top)),
            2,
            Route::Verb {
                command: "task".into(),
                verb: top.clone(),
                args: rest[1..].to_vec(),
                flags: ParsedFlags::default(),
            },
        ),
    }
}

fn task_machine(operation: &str, args: &[String]) -> CliOutcome {
    let opts = match parse_options(MachineInterface::Task, args, operation) {
        Ok(opts) => opts,
        Err(err) => {
            return machine_outcome(
                MachineInterface::Task,
                operation,
                MachineOptions::default(),
                None,
                Some(err),
            );
        }
    };
    if !opts.json {
        return machine_outcome(
            MachineInterface::Task,
            operation,
            opts,
            None,
            Some(invalid(MACHINE_JSON_REQUIRED)),
        );
    }
    let payload = match operation {
        "list" => "{\"schema_version\":1,\"command\":\"task.list\",\"tasks\":[]}",
        "show" => "{\"schema_version\":1,\"command\":\"task.show\",\"task\":null}",
        _ => unreachable!(),
    };
    machine_outcome(MachineInterface::Task, operation, opts, Some(payload), None)
}

// ---------------------------------------------------------------------------
// `sessions` 1.x maintenance and `catalogs reindex`.
// ---------------------------------------------------------------------------

const MAINTENANCE_VERBS: [&str; 3] = ["reindex", "diagnose", "cleanup"];

/// `reasonix sessions ...`: maintenance verbs route here; anything else falls
/// through to the session machine interface.
pub(crate) fn sessions_command(rest: &[String]) -> CliOutcome {
    let Some(verb) = rest.first() else {
        return session_command(rest);
    };
    if !MAINTENANCE_VERBS.contains(&verb.as_str()) {
        return session_command(rest);
    }
    let mut flags = ParsedFlags::default();
    let mut i = 1;
    while i < rest.len() {
        let arg = rest[i].as_str();
        match arg {
            "--json" => {
                flags.values.push(("json".to_owned(), "true".to_owned()));
            }
            "--apply" => {
                flags.values.push(("apply".to_owned(), "true".to_owned()));
            }
            "--dir" => {
                let Some(next) = rest.get(i + 1) else {
                    return maintenance_usage(&flags, verb);
                };
                flags.values.push(("dir".to_owned(), next.clone()));
                i += 1;
            }
            _ => {
                if arg.starts_with("--dir=") {
                    let value = arg.split_once('=').map(|(_, v)| v).unwrap_or_default();
                    flags.values.push(("dir".to_owned(), value.to_owned()));
                } else {
                    return maintenance_usage(&flags, verb);
                }
            }
        }
        i += 1;
    }
    CliOutcome::new(
        String::new(),
        crate::route::unmigrated_notice(&format!("sessions {verb}")),
        2,
        Route::SessionsMaintenance {
            verb: verb.clone(),
            flags,
        },
    )
}

fn maintenance_usage(flags: &ParsedFlags, verb: &str) -> CliOutcome {
    CliOutcome::new(
        String::new(),
        format!("{SESSIONS_MAINTENANCE_USAGE}\n"),
        2,
        Route::SessionsMaintenance {
            verb: verb.to_owned(),
            flags: flags.clone(),
        },
    )
}

const KNOWN_CATALOGS: [&str; 2] = ["history", "usage"];

/// `reasonix catalogs reindex <catalog>`.
pub(crate) fn catalogs_command(rest: &[String]) -> CliOutcome {
    if rest.len() < 2 || rest[0] != "reindex" {
        return CliOutcome::new(
            String::new(),
            format!("{CATALOGS_REINDEX_USAGE}\n"),
            2,
            Route::Catalogs {
                catalog: String::new(),
                args: rest.to_vec(),
            },
        );
    }
    let catalog = rest[1].clone();
    if !KNOWN_CATALOGS.contains(&catalog.as_str()) {
        return CliOutcome::new(
            String::new(),
            format!("{}\n", CATALOGS_UNKNOWN.replace("%s", &catalog)),
            2,
            Route::Catalogs {
                catalog,
                args: rest[1..].to_vec(),
            },
        );
    }
    CliOutcome::new(
        String::new(),
        crate::route::unmigrated_notice(&format!("catalogs reindex {catalog}")),
        2,
        Route::Catalogs {
            catalog,
            args: rest[1..].to_vec(),
        },
    )
}
