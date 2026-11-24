//! Second verb family: `report`, `config`, `completion`, `docs-manifest`,
//! `upgrade`/`update`, `review`, `setup`, `login`/`whoami`/`logout`,
//! `remote`, `trust`.

use crate::flags::{ParsedFlags, parse, review_specs};
use crate::route::{CliEnv, CliOutcome, Route};
use crate::text::*;

fn verb_route(command: &str, verb: &str, args: Vec<String>, flags: ParsedFlags) -> Route {
    Route::Verb {
        command: command.to_owned(),
        verb: verb.to_owned(),
        args,
        flags,
    }
}

// ---------------------------------------------------------------------------
// report (CLI-040).
// ---------------------------------------------------------------------------

const REPORT_ACTIONS: [&str; 4] = ["list", "show", "send", "delete"];

/// `reasonix report [action] [ID]`.
pub(crate) fn report_command(rest: &[String]) -> CliOutcome {
    let usage = || {
        CliOutcome::new(
            String::new(),
            format!("{REPORT_USAGE_BODY}\n"),
            2,
            verb_route("report", "", rest.to_vec(), ParsedFlags::default()),
        )
    };
    if rest.len() > 2 {
        return usage();
    }
    let Some(action) = rest.first() else {
        // Bare preview: backend flow.
        return CliOutcome::new(
            String::new(),
            crate::route::unmigrated_notice("report"),
            2,
            verb_route("report", "", rest.to_vec(), ParsedFlags::default()),
        );
    };
    if !REPORT_ACTIONS.contains(&action.as_str()) {
        return usage();
    }
    let id = rest.get(1).map(String::as_str).unwrap_or("");
    if action == "list" && !id.is_empty() {
        return usage();
    }
    CliOutcome::new(
        String::new(),
        crate::route::unmigrated_notice(&format!("report {action}")),
        2,
        verb_route("report", action, rest[1..].to_vec(), ParsedFlags::default()),
    )
}

// ---------------------------------------------------------------------------
// config (CLI-039).
// ---------------------------------------------------------------------------

const CONFIG_VERBS: [&str; 5] = [
    "auto-plan",
    "reasoning-language",
    "compact-ratio",
    "currency",
    "telemetry",
];

/// `reasonix config <verb> [--local] [value]`.
pub(crate) fn config_command(rest: &[String]) -> CliOutcome {
    let Some(verb) = rest.first() else {
        return config_usage_outcome(rest, "");
    };
    if !CONFIG_VERBS.contains(&verb.as_str()) {
        return config_usage_outcome(rest, verb);
    }

    let mut local = false;
    let mut values: Vec<String> = Vec::new();
    for arg in &rest[1..] {
        match arg.as_str() {
            "--local" => local = true,
            other => values.push(other.to_owned()),
        }
    }
    let value = values.first().map(String::as_str).unwrap_or("");

    let usage_body = match verb.as_str() {
        "telemetry" => CONFIG_TELEMETRY_USAGE,
        "compact-ratio" => CONFIG_COMPACT_RATIO_USAGE,
        "auto-plan" => CONFIG_AUTO_PLAN_USAGE,
        "reasoning-language" => CONFIG_LANGUAGE_USAGE,
        _ => CONFIG_CURRENCY_USAGE,
    };
    if values.len() > 1 {
        return CliOutcome::new(
            String::new(),
            format!("{usage_body}\n"),
            2,
            verb_route("config", verb, rest[1..].to_vec(), ParsedFlags::default()),
        );
    }

    if verb == "currency" && local {
        return config_err(CONFIG_CURRENCY_LOCAL_ERROR, verb, rest);
    }
    if verb == "currency" && !value.is_empty() {
        let normalized = value.trim().to_uppercase();
        if !matches!(normalized.as_str(), "" | "AUTO" | "CNY" | "USD") {
            let msg = CONFIG_CURRENCY_VALUE_ERROR.replace("%q", &format!("{value:?}"));
            return config_err(&msg, verb, rest);
        }
    }
    if verb == "auto-plan" && local {
        return config_err(CONFIG_AUTO_PLAN_LOCAL_ERROR, verb, rest);
    }
    if verb == "reasoning-language" && !value.is_empty() {
        let normalized = value.trim().to_lowercase();
        if !matches!(normalized.as_str(), "auto" | "zh" | "en") {
            let msg = CONFIG_LANGUAGE_VALUE_ERROR.replace("%q", &format!("{value:?}"));
            return config_err(&msg, verb, rest);
        }
    }
    if verb == "compact-ratio"
        && !value.is_empty()
        && let Err(msg) = parse_compact_ratio(value)
    {
        return config_err(&msg, verb, rest);
    }

    CliOutcome::new(
        String::new(),
        crate::route::unmigrated_notice(&format!("config {verb}")),
        2,
        verb_route("config", verb, rest[1..].to_vec(), ParsedFlags::default()),
    )
}

fn config_usage_outcome(rest: &[String], verb: &str) -> CliOutcome {
    // Reference prints configUsage to stdout but exits 2 (cli.go:1346-1348).
    let args = rest.get(1..).unwrap_or(&[]).to_vec();
    CliOutcome::new(
        format!("{CONFIG_USAGE}\n"),
        String::new(),
        2,
        verb_route("config", verb, args, ParsedFlags::default()),
    )
}

fn config_err(message: &str, verb: &str, rest: &[String]) -> CliOutcome {
    CliOutcome::new(
        String::new(),
        format!("{ERROR_PREFIX} {message}\n"),
        2,
        verb_route("config", verb, rest[1..].to_vec(), ParsedFlags::default()),
    )
}

/// Percentage in (0, 100); mirrors `parseCLICompactRatio` bounds.
fn parse_compact_ratio(value: &str) -> Result<(), String> {
    let percent: f64 = value
        .trim()
        .parse()
        .map_err(|_| COMPACT_RATIO_VALUE_ERROR.replace("%w", "not a number"))?;
    if !(percent > 0.0 && percent < 100.0) {
        return Err(COMPACT_RATIO_VALUE_ERROR.replace("%w", "ratio out of range"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// completion (CLI-041).
// ---------------------------------------------------------------------------

const BASH_SCRIPT: &str = "# bash completion for reasonix

_reasonix_completion() {
  COMPREPLY=($(command reasonix completion __complete \"$COMP_CWORD\" \"${COMP_WORDS[@]}\" 2>/dev/null))
}

complete -o default -F _reasonix_completion reasonix
";

const ZSH_SCRIPT: &str = "#compdef reasonix

_reasonix_completion() {
  compadd -- $(command reasonix completion __complete \"$((CURRENT - 1))\" \"${words[@]}\" 2>/dev/null)
}

compdef _reasonix_completion reasonix
";

const FISH_SCRIPT: &str = "function __reasonix_completion
  set -l tokens (commandline -opc)
  set -l current_index (math (count $tokens) - 1)
  command reasonix completion __complete $current_index $tokens 2>/dev/null
end

complete -c reasonix -a '(__reasonix_completion)'
";

/// `reasonix completion bash|zsh|fish` plus the hidden `__complete` protocol.
pub(crate) fn completion_command(rest: &[String]) -> CliOutcome {
    if rest.is_empty() {
        return CliOutcome::new(
            format!("{COMPLETION_USAGE}\n"),
            String::new(),
            2,
            Route::Completion {
                shell: String::new(),
            },
        );
    }
    let first = &rest[0];
    if matches!(first.as_str(), "-h" | "--help" | "help") {
        return CliOutcome::new(
            format!("{COMPLETION_USAGE}\n"),
            String::new(),
            0,
            Route::Completion {
                shell: "help".into(),
            },
        );
    }
    if first == "__complete" {
        return complete_protocol(&rest[1..]);
    }
    if rest.len() != 1 {
        return CliOutcome::new(
            format!("{COMPLETION_USAGE}\n"),
            String::new(),
            2,
            Route::Completion {
                shell: first.clone(),
            },
        );
    }
    let script = match first.as_str() {
        "bash" => BASH_SCRIPT,
        "zsh" => ZSH_SCRIPT,
        "fish" => FISH_SCRIPT,
        other => {
            return CliOutcome::new(
                String::new(),
                format!(
                    "{}\n",
                    COMPLETION_UNKNOWN_SHELL.replace("%q", &format!("{other:?}"))
                ),
                2,
                Route::Completion {
                    shell: other.to_owned(),
                },
            );
        }
    };
    CliOutcome::new(
        script.to_owned(),
        String::new(),
        0,
        Route::Completion {
            shell: first.clone(),
        },
    )
}

/// Hidden completion protocol: `__complete <cword> <words...>`.
fn complete_protocol(args: &[String]) -> CliOutcome {
    let Some(cword) = args.first() else {
        return CliOutcome::new(
            String::new(),
            String::new(),
            2,
            Route::Completion {
                shell: "__complete".into(),
            },
        );
    };
    let Ok(cword) = cword.parse::<i32>() else {
        return CliOutcome::new(
            String::new(),
            String::new(),
            2,
            Route::Completion {
                shell: "__complete".into(),
            },
        );
    };
    if cword < 0 {
        return CliOutcome::new(
            String::new(),
            String::new(),
            2,
            Route::Completion {
                shell: "__complete".into(),
            },
        );
    }
    // Static candidates live in the completion spec; dynamic model/session
    // lists are the backend anchor. The parse layer emits an empty candidate
    // stream until those lists migrate.
    let _ = cword;
    CliOutcome::new(
        String::new(),
        String::new(),
        0,
        Route::Completion {
            shell: "__complete".into(),
        },
    )
}

// ---------------------------------------------------------------------------
// docs-manifest (CLI-042).
// ---------------------------------------------------------------------------

/// `reasonix docs-manifest [--verify-source DIR] [--expect-version V] [--expect-revision R]`.
pub(crate) fn docs_manifest_command(rest: &[String]) -> CliOutcome {
    let mut i = 0;
    while i < rest.len() {
        let arg = rest[i].as_str();
        match arg {
            "--verify-source" | "--expect-version" | "--expect-revision" => {
                i += 1;
            }
            _ if arg.starts_with("--verify-source=")
                || arg.starts_with("--expect-version=")
                || arg.starts_with("--expect-revision=") => {}
            _ => {
                return CliOutcome::new(
                    String::new(),
                    format!("{DOCS_MANIFEST_FLAGS_ONLY}\n"),
                    2,
                    verb_route("docs-manifest", "", rest.to_vec(), ParsedFlags::default()),
                );
            }
        }
        i += 1;
    }
    CliOutcome::new(
        String::new(),
        crate::route::unmigrated_notice("docs-manifest"),
        2,
        verb_route("docs-manifest", "", rest.to_vec(), ParsedFlags::default()),
    )
}

// ---------------------------------------------------------------------------
// upgrade / update (CLI-043).
// ---------------------------------------------------------------------------

/// Channel names normalize by trim + lower-case before the accepted set check
/// (`parseCLIReleaseChannel`, upgrade.go:75-82).
fn channel_accepted(value: &str) -> bool {
    let normalized = value.trim().to_lowercase();
    matches!(
        normalized.as_str(),
        "" | "stable" | "preview" | "canary" | "beta" | "next"
    )
}

/// `reasonix upgrade|update [--check] [--force] [--channel CHANNEL] [CHANNEL]`.
pub(crate) fn upgrade_command(rest: &[String], cmd: &str) -> CliOutcome {
    let mut check = false;
    let mut force = false;
    let mut channel_flag: Option<String> = None;
    let mut positional: Vec<String> = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        let arg = rest[i].as_str();
        match arg {
            "--check" => check = true,
            "--force" => force = true,
            "--channel" => {
                let Some(next) = rest.get(i + 1) else {
                    return upgrade_err(UPGRADE_CHANNEL_VALUE);
                };
                channel_flag = Some(next.clone());
                i += 1;
            }
            _ if let Some(v) = arg.strip_prefix("--channel=") => channel_flag = Some(v.to_owned()),
            _ => positional.push(arg.to_owned()),
        }
        i += 1;
    }
    let _ = (check, force);

    if positional.len() > 1 {
        return upgrade_err(UPGRADE_ONE_POSITIONAL);
    }
    let positional_channel = positional.first().cloned().unwrap_or_default();
    if positional.len() == 1 && positional_channel.is_empty() {
        return upgrade_err("channel is required");
    }
    if let Some(flag) = &channel_flag {
        if flag.is_empty() {
            return upgrade_err(UPGRADE_CHANNEL_VALUE);
        }
        if !channel_accepted(flag) {
            let msg = UPGRADE_CHANNEL_UNSUPPORTED.replace("%q", &format!("{flag:?}"));
            return upgrade_err(&msg);
        }
    }
    if !positional_channel.is_empty()
        && channel_flag.is_some()
        && positional_channel != channel_flag.as_deref().unwrap_or_default()
    {
        let msg = UPGRADE_CHANNEL_CONFLICT
            .replace("%q", &format!("{positional_channel:?}"))
            .replace(
                "%q",
                &format!("{:?}", channel_flag.as_deref().unwrap_or_default()),
            );
        return upgrade_err(&msg);
    }
    if !positional_channel.is_empty() && !channel_accepted(&positional_channel) {
        let msg = UPGRADE_CHANNEL_UNSUPPORTED.replace("%q", &format!("{positional_channel:?}"));
        return upgrade_err(&msg);
    }
    CliOutcome::new(
        String::new(),
        crate::route::unmigrated_notice(cmd),
        2,
        verb_route(cmd, "", rest.to_vec(), ParsedFlags::default()),
    )
}

fn upgrade_err(message: &str) -> CliOutcome {
    CliOutcome::new(
        String::new(),
        format!("{ERROR_PREFIX} {message}\n"),
        2,
        verb_route("upgrade", "", vec![], ParsedFlags::default()),
    )
}

// ---------------------------------------------------------------------------
// review (CLI-044).
// ---------------------------------------------------------------------------

/// `reasonix review [--base BRANCH] [--commit SHA] [--model NAME] [--instructions TEXT]`.
pub(crate) fn review_command(rest: &[String]) -> CliOutcome {
    let flags = parse(&review_specs(), rest, true);
    let route = verb_route("review", "", rest.to_vec(), flags.clone());
    if flags.help {
        return CliOutcome::new(String::new(), String::new(), 0, route);
    }
    if let Some(err) = &flags.error {
        return CliOutcome::new(String::new(), format!("{ERROR_PREFIX} {err}\n"), 2, route);
    }
    CliOutcome::new(
        String::new(),
        crate::route::unmigrated_notice("review"),
        2,
        route,
    )
}

// ---------------------------------------------------------------------------
// setup (CLI-045).
// ---------------------------------------------------------------------------

/// `reasonix setup [--local|-l] [path]`.
pub(crate) fn setup_command(rest: &[String]) -> CliOutcome {
    if rest.iter().any(|a| a == "-h" || a == "--help") {
        return CliOutcome::new(
            format!("{SETUP_USAGE}\n"),
            String::new(),
            0,
            verb_route("setup", "", rest.to_vec(), ParsedFlags::default()),
        );
    }
    let mut local = false;
    let mut path: Option<String> = None;
    for arg in rest {
        match arg.as_str() {
            "--local" | "-l" => local = true,
            other if other.starts_with('-') => {
                let stderr = format!(
                    "{}\n\n{SETUP_USAGE}\n",
                    SETUP_UNKNOWN_FLAG.replace("%q", &format!("{other:?}"))
                );
                return CliOutcome::new(
                    String::new(),
                    stderr,
                    2,
                    verb_route("setup", "", rest.to_vec(), ParsedFlags::default()),
                );
            }
            other => path = Some(other.to_owned()),
        }
    }
    let _ = (local, path);
    CliOutcome::new(
        String::new(),
        crate::route::unmigrated_notice("setup"),
        2,
        verb_route("setup", "", rest.to_vec(), ParsedFlags::default()),
    )
}

// ---------------------------------------------------------------------------
// account: login / whoami / logout (CLI-030).
// ---------------------------------------------------------------------------

/// `reasonix login|whoami|logout [args]`.
pub(crate) fn account_command(cmd: &str, rest: &[String]) -> CliOutcome {
    if let Some(extra) = rest.first() {
        let msg = ACCOUNT_UNEXPECTED_ARG
            .replace("%s", cmd)
            .replace("%q", &format!("{extra:?}"));
        return CliOutcome::new(
            String::new(),
            format!("{msg}\n"),
            2,
            verb_route(cmd, "", rest.to_vec(), ParsedFlags::default()),
        );
    }
    CliOutcome::new(
        String::new(),
        crate::route::unmigrated_notice(cmd),
        2,
        verb_route(cmd, "", rest.to_vec(), ParsedFlags::default()),
    )
}

// ---------------------------------------------------------------------------
// remote (CLI-029).
// ---------------------------------------------------------------------------

const REMOTE_READONLY_TOP: [&str; 5] = ["list", "ls", "help", "-h", "--help"];
const REMOTE_STUBS: [&str; 3] = [
    "attach-workspace",
    "runtime-workbench",
    "workbench-build-id",
];

/// `reasonix remote <verb>`.
pub(crate) fn remote_command(rest: &[String], env: &CliEnv) -> CliOutcome {
    let Some(first) = rest.first() else {
        return CliOutcome::new(
            format!("{REMOTE_USAGE}\n"),
            String::new(),
            2,
            verb_route("remote", "", vec![], ParsedFlags::default()),
        );
    };
    if matches!(first.as_str(), "help" | "-h" | "--help") {
        return CliOutcome::new(
            format!("{REMOTE_USAGE}\n"),
            String::new(),
            0,
            verb_route("remote", "help", rest[1..].to_vec(), ParsedFlags::default()),
        );
    }
    if env.agent_child && !remote_readonly(rest) {
        let msg = REMOTE_REFUSED_AGENT_CHILD
            .replace("%s", first)
            .replace("%s", AGENT_CHILD_ENV);
        return CliOutcome::new(
            String::new(),
            format!("{msg}\n"),
            77,
            verb_route("remote", first, rest[1..].to_vec(), ParsedFlags::default()),
        );
    }
    if REMOTE_STUBS.contains(&first.as_str()) {
        let msg = REMOTE_REMOVED_STUB.replace("%s", first);
        return CliOutcome::new(
            String::new(),
            format!("{msg}\n"),
            1,
            verb_route("remote", first, rest[1..].to_vec(), ParsedFlags::default()),
        );
    }
    let known = [
        "add", "list", "ls", "remove", "rm", "import", "test", "connect", "open", "wsl", "status",
        "forward", "serve", "fs",
    ];
    if !known.contains(&first.as_str()) {
        let stderr = format!(
            "{}\n\n",
            REMOTE_UNKNOWN_SUBCOMMAND.replace("%q", &format!("{first:?}"))
        );
        return CliOutcome::new(
            format!("{REMOTE_USAGE}\n"),
            stderr,
            2,
            verb_route("remote", first, rest[1..].to_vec(), ParsedFlags::default()),
        );
    }
    CliOutcome::new(
        String::new(),
        crate::route::unmigrated_notice(&format!("remote {first}")),
        2,
        verb_route("remote", first, rest[1..].to_vec(), ParsedFlags::default()),
    )
}

fn remote_readonly(rest: &[String]) -> bool {
    let first = rest.first().map(String::as_str).unwrap_or_default();
    if REMOTE_READONLY_TOP.contains(&first) {
        return true;
    }
    if first == "wsl" {
        return matches!(rest.get(1).map(String::as_str), Some("list") | Some("test"));
    }
    false
}

// ---------------------------------------------------------------------------
// trust (CLI-037).
// ---------------------------------------------------------------------------

/// `reasonix trust [--dir PATH] [--yes [--allow-providers]|--revoke]`.
pub(crate) fn trust_command(rest: &[String]) -> CliOutcome {
    let mut yes = false;
    let mut revoke = false;
    let mut allow_providers = false;
    let mut dir: Option<String> = None;
    let mut i = 0;
    while i < rest.len() {
        let arg = rest[i].as_str();
        match arg {
            "--yes" | "-y" => yes = true,
            "--revoke" => revoke = true,
            "--allow-providers" => allow_providers = true,
            "--dir" => {
                let Some(next) = rest.get(i + 1) else {
                    return trust_err(&TRUST_UNKNOWN_ARG.replace("%q", "--dir"));
                };
                dir = Some(next.clone());
                i += 1;
            }
            _ if let Some(v) = arg.strip_prefix("--dir=") => dir = Some(v.to_owned()),
            other => {
                return trust_err(&TRUST_UNKNOWN_ARG.replace("%q", &format!("{other:?}")));
            }
        }
        i += 1;
    }
    if yes && revoke {
        return trust_err(TRUST_CHOOSE_ONE);
    }
    let mut flags = ParsedFlags::default();
    if yes {
        flags.values.push(("yes".into(), "true".into()));
    }
    if revoke {
        flags.values.push(("revoke".into(), "true".into()));
    }
    if allow_providers {
        flags.values.push(("allow-providers".into(), "true".into()));
    }
    if let Some(d) = dir {
        flags.values.push(("dir".into(), d));
    }
    CliOutcome::new(
        String::new(),
        crate::route::unmigrated_notice("trust"),
        2,
        verb_route("trust", "", rest.to_vec(), flags),
    )
}

fn trust_err(message: &str) -> CliOutcome {
    // Reference writes parser errors to stdout (out=os.Stdout), exit 2.
    CliOutcome::new(
        format!("{message}\n"),
        String::new(),
        2,
        verb_route("trust", "", vec![], ParsedFlags::default()),
    )
}
