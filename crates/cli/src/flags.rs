//! Declarative flag tables and a small pflag-compatible parser.
//!
//! The reference parses both session-facing sets with `spf13/pflag`
//! (interspersed); this parser reproduces only the surface the routing and
//! flag-matrix rows depend on: long and shorthand flags, `--name=value`,
//! optional-value `--resume`, repeatable arrays, the `--` terminator, and help
//! detection. Business validation (permission modes, output formats, ablation
//! arms) lives with its command.
//!
//! Spec tables live in [`session`] (TUI/run) and [`serve`] (serve/web, ACP,
//! upgrade, review, machine options) to keep this core under the line limit.

pub(crate) mod serve;
pub(crate) mod session;

pub(crate) use serve::{acp_specs, review_specs, serve_specs};
pub(crate) use session::{run_specs, session_specs, tui_specs, union_specs};

/// Value-optional sentinel a bare TUI `--resume`/`-r` carries (pflag
/// `NoOptDefVal`); the picker expands it later.
pub const RESUME_PICKER_SENTINEL: &str = "__reasonix_resume_picker__";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlagKind {
    Bool,
    String,
    StringArray,
    Int,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FlagSpec {
    pub(crate) name: &'static str,
    pub(crate) short: Option<char>,
    pub(crate) kind: FlagKind,
    pub(crate) optional_value: bool,
}

/// `--continue` / `-c` registered through BoolP so the shorthand stays a
/// shorthand, not an accidental long flag (reference registerContinueFlag).
const CONTINUE: FlagSpec = FlagSpec {
    name: "continue",
    short: Some('c'),
    kind: FlagKind::Bool,
    optional_value: false,
};

/// Parsed result: set flag values (`name=value`), remaining positionals, and
/// the help/error signal a command boundary renders.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedFlags {
    pub values: Vec<(String, String)>,
    pub positionals: Vec<String>,
    pub help: bool,
    pub error: Option<String>,
}

impl ParsedFlags {
    fn ok(values: Vec<(String, String)>, positionals: Vec<String>) -> Self {
        Self {
            values,
            positionals,
            help: false,
            error: None,
        }
    }

    fn failed(error: String) -> Self {
        Self {
            values: Vec::new(),
            positionals: Vec::new(),
            help: false,
            error: Some(error),
        }
    }

    /// Looks up a set string flag, preserving whether it was given.
    pub fn value(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn bool(&self, name: &str) -> bool {
        self.values.iter().any(|(n, v)| n == name && v == "true")
    }

    pub fn changed(&self, name: &str) -> bool {
        self.values.iter().any(|(n, _)| n == name)
    }

    /// Repeatable string values for `name`, in order.
    pub fn values(&self, name: &str) -> Vec<&str> {
        self.values
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }
}

fn find_spec<'a>(specs: &'a [FlagSpec], name: &str, short: Option<char>) -> Option<&'a FlagSpec> {
    specs
        .iter()
        .find(|s| s.name == name || short.is_some_and(|c| s.short == Some(c)))
}

/// Parses `args` against `specs`. When `interspersed` is false (the reference
/// `quietFlagSet` sets `SetInterspersed(false)`), the first positional stops
/// flag parsing and everything after it is a positional; when true, flags may
/// follow positionals. A `--` token always moves the rest to positionals.
pub(crate) fn parse(specs: &[FlagSpec], args: &[String], interspersed: bool) -> ParsedFlags {
    let mut values: Vec<(String, String)> = Vec::new();
    let mut positionals: Vec<String> = Vec::new();
    let mut after_terminator = false;
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        if after_terminator {
            positionals.push(arg.clone());
            i += 1;
            continue;
        }
        if arg == "--" {
            after_terminator = true;
            i += 1;
            continue;
        }
        if arg == "-h" || arg == "--help" {
            return ParsedFlags {
                values,
                positionals,
                help: true,
                error: None,
            };
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (name, inline_value) = match long.split_once('=') {
                Some((n, v)) => (n.to_owned(), Some(v.to_owned())),
                None => (long.to_owned(), None),
            };
            let Some(spec) = find_spec(specs, &name, None) else {
                return ParsedFlags::failed(format!("unknown flag: --{name}"));
            };
            match spec.kind {
                FlagKind::Bool => {
                    let v = inline_value.as_deref().unwrap_or("true");
                    values.push((spec.name.to_owned(), bool_literal(v).to_string()));
                }
                FlagKind::String | FlagKind::Int => {
                    let value = match inline_value {
                        Some(v) => v,
                        None if spec.optional_value => RESUME_PICKER_SENTINEL.to_owned(),
                        None => match args.get(i + 1) {
                            Some(next) => {
                                i += 1;
                                next.clone()
                            }
                            None => {
                                return ParsedFlags::failed(format!(
                                    "flag needs an argument: --{name}"
                                ));
                            }
                        },
                    };
                    values.push((spec.name.to_owned(), value));
                }
                FlagKind::StringArray => {
                    let value = match inline_value {
                        Some(v) => v,
                        None => match args.get(i + 1) {
                            Some(next) => {
                                i += 1;
                                next.clone()
                            }
                            None => {
                                return ParsedFlags::failed(format!(
                                    "flag needs an argument: --{name}"
                                ));
                            }
                        },
                    };
                    values.push((spec.name.to_owned(), value));
                }
            }
            i += 1;
            continue;
        }
        if let Some(short_arg) = arg.strip_prefix('-') {
            let mut chars = short_arg.chars();
            let Some(ch) = chars.next() else {
                positionals.push(arg.clone());
                i += 1;
                continue;
            };
            let rest: String = chars.collect();
            let Some(spec) = find_spec(specs, "", Some(ch)) else {
                return ParsedFlags::failed(format!(
                    "unknown shorthand flag: '{ch}' in -{short_arg}"
                ));
            };
            match spec.kind {
                FlagKind::Bool => {
                    let value = if rest.is_empty() {
                        "true"
                    } else {
                        rest.strip_prefix('=').unwrap_or(&rest)
                    };
                    values.push((spec.name.to_owned(), bool_literal(value).to_string()));
                }
                FlagKind::String | FlagKind::Int | FlagKind::StringArray => {
                    let value = if let Some(v) = rest.strip_prefix('=') {
                        v.to_owned()
                    } else if !rest.is_empty() {
                        rest
                    } else if spec.optional_value {
                        RESUME_PICKER_SENTINEL.to_owned()
                    } else {
                        match args.get(i + 1) {
                            Some(next) => {
                                i += 1;
                                next.clone()
                            }
                            None => {
                                return ParsedFlags::failed(format!(
                                    "flag needs an argument: -{ch}"
                                ));
                            }
                        }
                    };
                    values.push((spec.name.to_owned(), value));
                }
            }
            i += 1;
            continue;
        }
        if !interspersed {
            positionals.extend_from_slice(&args[i..]);
            break;
        }
        positionals.push(arg.clone());
        i += 1;
    }
    ParsedFlags::ok(values, positionals)
}

fn bool_literal(raw: &str) -> bool {
    !matches!(raw, "false" | "0" | "off" | "no")
}

/// Normalizes the TUI's optional-value `--resume`/`-r`: a following non-flag
/// token becomes `--resume=<value>` (reference normalizeOptionalResumeArg).
pub(crate) fn normalize_optional_resume_arg(args: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if (arg == "--resume" || arg == "-r") && i + 1 < args.len() && !args[i + 1].starts_with('-')
        {
            out.push(format!("{arg}={}", args[i + 1]));
            i += 2;
        } else {
            out.push(arg.clone());
            i += 1;
        }
    }
    out
}

/// `isDefaultInteractiveFlag` (cli.go:171-180): the flags a bare `reasonix`
/// accepts, including their `--name=value` form.
pub fn is_default_interactive_flag(arg: &str) -> bool {
    const NAMES: [&str; 18] = [
        "--model",
        "--max-steps",
        "--continue",
        "-c",
        "--resume",
        "-r",
        "--copy",
        "--dangerously-skip-permissions",
        "--yolo",
        "--permission-mode",
        "--effort",
        "--dir",
        "--add-dir",
        "--allowed-tools",
        "--allowedTools",
        "--profile",
        "--preset",
        "--agent-composition",
    ];
    if NAMES.contains(&arg) {
        return true;
    }
    match arg.split_once('=') {
        Some((name, _)) => NAMES.contains(&name),
        None => false,
    }
}

/// `startsWithSessionFlag` (cli_flags.go:236-242): argv opens with a flag
/// rather than a verb; `-y`/`-p` belong to run but may be written first.
pub fn starts_with_session_flag(arg: &str) -> bool {
    matches!(arg, "-y" | "--auto" | "-p" | "--print")
        || is_default_interactive_flag(arg)
        || arg.starts_with("--auto=")
}

/// `hasLeadingPrintFlag`: a standalone `-p`/`--print` before any `--`
/// terminator means one-shot print mode.
pub fn has_leading_print_flag(args: &[String]) -> bool {
    for arg in args {
        if arg == "--" {
            return false;
        }
        if arg == "-p" || arg == "--print" {
            return true;
        }
    }
    false
}

/// `stripLeadingPrintFlag`: drops the first leading `-p`/`--print`, leaving
/// everything after `--` untouched.
pub fn strip_leading_print_flag(args: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(args.len());
    let mut dropped = false;
    for (i, arg) in args.iter().enumerate() {
        if arg == "--" {
            out.extend_from_slice(&args[i..]);
            break;
        }
        if !dropped && (arg == "-p" || arg == "--print") {
            dropped = true;
            continue;
        }
        out.push(arg.clone());
    }
    out
}

/// Splits `args` into the leading flag block and the positionals, using the
/// union of the TUI and run sets so a value of either is not read as the verb.
fn split_at_run_verb(args: &[String]) -> (Vec<String>, Vec<String>, bool) {
    let norm = normalize_optional_resume_arg(args);
    let parsed = parse(&union_specs(), &norm, false);
    let rest = parsed.positionals;
    let lead_len = norm.len() - rest.len();
    let lead = norm[..lead_len].to_vec();
    let verb = !rest.is_empty() && rest[0] == "run" && !lead.iter().any(|a| a == "--");
    (lead, rest, verb)
}

/// `leadingFlagsIntoRun`: leading flags move after `run` only when the session
/// set and run's set read the whole lead identically (same names and values,
/// zero positionals), the verb is exactly `run`, and no `--` sits in the lead.
pub fn leading_flags_into_run(args: &[String]) -> Option<Vec<String>> {
    if args.is_empty() || !starts_with_session_flag(&args[0]) {
        return None;
    }
    let (lead, rest, verb) = split_at_run_verb(args);
    if !verb {
        return None;
    }
    let session = parse(&session_specs(), &lead, false);
    let run = parse(&run_specs(), &lead, false);
    if session.error.is_some()
        || run.error.is_some()
        || !session.positionals.is_empty()
        || !run.positionals.is_empty()
        || set_flag_values(&session) != set_flag_values(&run)
    {
        return None;
    }
    let mut routed = Vec::with_capacity(lead.len() + rest.len());
    routed.push("run".to_owned());
    routed.extend(lead);
    routed.extend_from_slice(&rest[1..]);
    Some(routed)
}

fn set_flag_values(parsed: &ParsedFlags) -> Vec<(String, String)> {
    let mut values = parsed.values.clone();
    values.sort();
    values
}

/// `flagsBeforeVerb`: where a leading `-p` may sit; after `run` it belongs to
/// run itself.
fn flags_before_verb(args: &[String]) -> Vec<String> {
    let (_lead, _rest, verb) = split_at_run_verb(args);
    if verb { _lead } else { args.to_vec() }
}

/// `normalizeCommand`: names the subcommand argv asks for, applying the three
/// leading-flag routing rules before the top-level switch.
pub fn normalize_command(args: &[String]) -> (String, Vec<String>) {
    let mut cmd = args
        .first()
        .map(String::as_str)
        .unwrap_or_default()
        .to_owned();
    if cmd == "--acp" {
        cmd = "acp".to_owned();
    }
    if let Some(routed) = leading_flags_into_run(args) {
        return ("run".to_owned(), routed);
    }
    if cmd == "-p"
        || cmd == "--print"
        || (starts_with_session_flag(&cmd) && has_leading_print_flag(&flags_before_verb(args)))
    {
        let mut routed = vec!["run".to_owned(), "--print".to_owned()];
        routed.extend(strip_leading_print_flag(args));
        return ("run".to_owned(), routed);
    }
    if !args.is_empty() && is_default_interactive_flag(&cmd) {
        cmd.clear();
    }
    (cmd, args.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_flag_detection_respects_terminator() {
        assert!(has_leading_print_flag(&["-p".into(), "task".into()]));
        assert!(has_leading_print_flag(&[
            "--model".into(),
            "x".into(),
            "-p".into(),
            "task".into()
        ]));
        assert!(!has_leading_print_flag(&["--".into(), "-p".into()]));
        assert!(!has_leading_print_flag(&[
            "--model".into(),
            "x".into(),
            "--".into(),
            "-p".into()
        ]));
    }

    #[test]
    fn strip_drops_only_first_print_token() {
        assert_eq!(
            strip_leading_print_flag(&["-p".into(), "task".into()]),
            vec!["task".to_owned()]
        );
        assert_eq!(
            strip_leading_print_flag(&["--model".into(), "x".into(), "-p".into(), "task".into()]),
            vec!["--model".to_owned(), "x".to_owned(), "task".to_owned()]
        );
        assert_eq!(
            strip_leading_print_flag(&["-p".into(), "--".into(), "--print".into()]),
            vec!["--".to_owned(), "--print".to_owned()]
        );
    }

    #[test]
    fn optional_resume_joins_following_token() {
        assert_eq!(
            normalize_optional_resume_arg(&[
                "--model".into(),
                "x".into(),
                "--resume".into(),
                "session-id".into(),
                "--copy".into()
            ]),
            vec![
                "--model".to_owned(),
                "x".to_owned(),
                "--resume=session-id".to_owned(),
                "--copy".to_owned()
            ]
        );
        assert_eq!(
            normalize_optional_resume_arg(&["-r".into(), "--copy".to_owned()]),
            vec!["-r".to_owned(), "--copy".to_owned()]
        );
    }
}
