//! First verb family: `serve`/`web`, `acp`, `mcp`, `plugin`, `subagent`,
//! `doctor`. Validation, usage, and exit codes are decided here; the store,
//! registry, and provider kernels route to [`Route::Unmigrated`] /
//! [`Route::Verb`] anchors.

use crate::flags::{ParsedFlags, parse};
use crate::flags::{acp_specs, serve_specs};
use crate::permission::parse_permission_mode;
use crate::preset;
use crate::route::{CliOutcome, Route};
use crate::text::*;

fn verb_route(command: &str, verb: &str, args: Vec<String>, flags: ParsedFlags) -> Route {
    Route::Verb {
        command: command.to_owned(),
        verb: verb.to_owned(),
        args,
        flags,
    }
}

fn parse_flags(specs: &[crate::flags::FlagSpec], rest: &[String]) -> ParsedFlags {
    parse(specs, rest, true)
}

// ---------------------------------------------------------------------------
// serve / web (CLI-025).
// ---------------------------------------------------------------------------

/// `reasonix serve` / `reasonix web`.
pub(crate) fn serve_command(web: bool, rest: &[String]) -> CliOutcome {
    let flags = parse_flags(&serve_specs(), rest);
    let route = Route::Serve {
        web,
        args: rest.to_vec(),
        flags: flags.clone(),
    };
    if flags.help {
        return CliOutcome::new(String::new(), String::new(), 0, route);
    }
    if let Some(err) = &flags.error {
        return CliOutcome::new(String::new(), format!("{ERROR_PREFIX} {err}\n"), 2, route);
    }

    if flags.value("resume").is_some() && flags.value("session-id").is_some() {
        return CliOutcome::new(
            String::new(),
            format!("{ERROR_PREFIX} {SERVE_RESUME_SESSION_ID_CONFLICT}\n"),
            2,
            route,
        );
    }

    let broker = flags.value("provider-broker").unwrap_or("");
    let broker_file = flags.value("provider-broker-file").unwrap_or("");
    let token_file = flags.value("provider-broker-token-file").unwrap_or("");
    if !broker.is_empty() && !broker_file.is_empty() {
        return CliOutcome::new(
            String::new(),
            format!("{ERROR_PREFIX} {BROKER_TWICE_ERROR}\n"),
            2,
            route,
        );
    }
    if !broker.is_empty() && token_file.is_empty() {
        return CliOutcome::new(
            String::new(),
            format!("{ERROR_PREFIX} {BROKER_TOKEN_FILE_ERROR}\n"),
            2,
            route,
        );
    }

    CliOutcome::new(
        String::new(),
        crate::route::unmigrated_notice(if web { "web" } else { "serve" }),
        2,
        route,
    )
}

// ---------------------------------------------------------------------------
// acp (CLI-026).
// ---------------------------------------------------------------------------

/// `reasonix acp`: flag set, enum validations, and plan refusal.
pub(crate) fn acp_command(rest: &[String]) -> CliOutcome {
    let flags = parse_flags(&acp_specs(), rest);
    let route = Route::Acp;
    if flags.help {
        return CliOutcome::new(String::new(), String::new(), 0, route);
    }
    if let Some(err) = &flags.error {
        return CliOutcome::new(String::new(), format!("{ERROR_PREFIX} {err}\n"), 2, route);
    }

    let planner = flags.value("planner").unwrap_or("auto");
    if !matches!(planner, "" | "auto" | "off") {
        return CliOutcome::new(
            String::new(),
            format!("{ERROR_PREFIX} {ACP_PLANNER_ERROR}\n"),
            2,
            route,
        );
    }
    let network = flags.value("sandbox-network").unwrap_or("auto");
    if !matches!(network, "auto" | "on" | "off") {
        return CliOutcome::new(
            String::new(),
            format!("{ERROR_PREFIX} {ACP_NETWORK_ERROR}\n"),
            2,
            route,
        );
    }
    let bash = flags.value("sandbox-bash").unwrap_or("auto");
    if !matches!(bash, "auto" | "enforce") {
        return CliOutcome::new(
            String::new(),
            format!("{ERROR_PREFIX} {ACP_BASH_ERROR}\n"),
            2,
            route,
        );
    }

    let profile = flags.value("profile").unwrap_or("");
    if let Err(err) = preset::parse_runtime_profile(profile) {
        return CliOutcome::new(String::new(), format!("{ERROR_PREFIX} {err}\n"), 2, route);
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

    CliOutcome::new(
        String::new(),
        crate::route::unmigrated_notice("acp"),
        2,
        route,
    )
}

// ---------------------------------------------------------------------------
// mcp (CLI-027).
// ---------------------------------------------------------------------------

const MCP_LIST_LIKE: [&str; 2] = ["list", "ls"];
const MCP_BROWSE: [&str; 2] = ["browse", "search"];
const MCP_KNOWN: [&str; 18] = [
    "list",
    "ls",
    "add",
    "get",
    "remove",
    "rm",
    "enable",
    "disable",
    "retry",
    "connect",
    "trust",
    "auth",
    "authorize",
    "update",
    "import",
    "browse",
    "search",
    "install",
];

/// `reasonix mcp <verb>`.
pub(crate) fn mcp_command(rest: &[String]) -> CliOutcome {
    let Some(first) = rest.first() else {
        // Bare: usage on stdout, exit 2 (mcp.go:32-35).
        return CliOutcome::new(
            format!("{MCP_USAGE}\n"),
            String::new(),
            2,
            verb_route("mcp", "", vec![], ParsedFlags::default()),
        );
    };
    if matches!(first.as_str(), "help" | "-h" | "--help") {
        return CliOutcome::new(
            format!("{MCP_USAGE}\n"),
            String::new(),
            0,
            verb_route("mcp", "help", rest[1..].to_vec(), ParsedFlags::default()),
        );
    }

    let verb = first.as_str();
    if MCP_LIST_LIKE.contains(&verb) && rest.len() > 1 {
        return CliOutcome::new(
            String::new(),
            format!(
                "{}\n",
                MCP_LIST_UNEXPECTED.replace("%q", &format!("{:?}", rest[1]))
            ),
            2,
            verb_route("mcp", verb, rest[1..].to_vec(), ParsedFlags::default()),
        );
    }
    if verb == "import" && rest.len() > 1 {
        return CliOutcome::new(
            String::new(),
            format!(
                "{}\n",
                MCP_IMPORT_UNEXPECTED.replace("%q", &format!("{:?}", rest[1]))
            ),
            2,
            verb_route("mcp", verb, rest[1..].to_vec(), ParsedFlags::default()),
        );
    }
    if MCP_BROWSE.contains(&verb) {
        return mcp_browse(&rest[1..], verb);
    }
    if !MCP_KNOWN.contains(&verb) {
        let stderr = format!(
            "{}\n\n",
            MCP_UNKNOWN_SUBCOMMAND.replace("%q", &format!("{:?}", verb))
        );
        return CliOutcome::new(
            format!("{MCP_USAGE}\n"),
            stderr,
            2,
            verb_route("mcp", verb, rest[1..].to_vec(), ParsedFlags::default()),
        );
    }
    CliOutcome::new(
        String::new(),
        crate::route::unmigrated_notice(&format!("mcp {verb}")),
        2,
        verb_route("mcp", verb, rest[1..].to_vec(), ParsedFlags::default()),
    )
}

fn mcp_browse(args: &[String], verb: &str) -> CliOutcome {
    let mut limit: Option<u32> = None;
    let mut queries: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--json" => {}
            "--limit" => {
                let Some(next) = args.get(i + 1) else {
                    return browse_err(MCP_BROWSE_LIMIT_VALUE, verb, args);
                };
                i += 1;
                match next.parse::<u32>() {
                    Ok(n) if (1..=100).contains(&n) => limit = Some(n),
                    _ => return browse_err(MCP_BROWSE_LIMIT_RANGE, verb, args),
                }
            }
            _ if arg.starts_with("--limit=") => {
                let value = arg.split_once('=').map(|(_, v)| v).unwrap_or_default();
                match value.parse::<u32>() {
                    Ok(n) if (1..=100).contains(&n) => limit = Some(n),
                    _ => return browse_err(MCP_BROWSE_LIMIT_RANGE, verb, args),
                }
            }
            _ if arg.starts_with('-') => {
                return browse_err(
                    &MCP_BROWSE_UNKNOWN_FLAG.replace("%q", &format!("{arg:?}")),
                    verb,
                    args,
                );
            }
            _ => queries.push(arg.to_owned()),
        }
        i += 1;
    }
    if queries.len() > 1 {
        return browse_err(MCP_BROWSE_ONE_QUERY, verb, args);
    }
    let _ = limit;
    CliOutcome::new(
        String::new(),
        crate::route::unmigrated_notice(&format!("mcp {verb}")),
        2,
        verb_route("mcp", verb, args.to_vec(), ParsedFlags::default()),
    )
}

fn browse_err(message: &str, verb: &str, args: &[String]) -> CliOutcome {
    CliOutcome::new(
        String::new(),
        format!("{message}\n"),
        2,
        verb_route("mcp", verb, args.to_vec(), ParsedFlags::default()),
    )
}

// ---------------------------------------------------------------------------
// plugin (CLI-028).
// ---------------------------------------------------------------------------

const PLUGIN_KNOWN: [&str; 9] = [
    "install",
    "list",
    "show",
    "remove",
    "uninstall",
    "enable",
    "disable",
    "doctor",
    "migrate",
];

/// `reasonix plugin <verb>`.
pub(crate) fn plugin_command(rest: &[String]) -> CliOutcome {
    let Some(first) = rest.first() else {
        // Bare: usage on stderr, exit 0 (plugin.go:23-26).
        return CliOutcome::new(
            String::new(),
            format!("{PLUGIN_USAGE}\n"),
            0,
            verb_route("plugin", "", vec![], ParsedFlags::default()),
        );
    };
    if matches!(first.as_str(), "help" | "--help" | "-h") {
        return CliOutcome::new(
            String::new(),
            format!("{PLUGIN_USAGE}\n"),
            0,
            verb_route("plugin", "help", rest[1..].to_vec(), ParsedFlags::default()),
        );
    }
    let verb = first.as_str();
    if verb == "install" {
        return plugin_install(&rest[1..]);
    }
    if !PLUGIN_KNOWN.contains(&verb) {
        let stderr = format!(
            "{}\n\n{PLUGIN_USAGE}\n",
            PLUGIN_UNKNOWN_COMMAND.replace("%q", &format!("{verb:?}"))
        );
        return CliOutcome::new(
            String::new(),
            stderr,
            2,
            verb_route("plugin", verb, rest[1..].to_vec(), ParsedFlags::default()),
        );
    }
    CliOutcome::new(
        String::new(),
        crate::route::unmigrated_notice(&format!("plugin {verb}")),
        2,
        verb_route("plugin", verb, rest[1..].to_vec(), ParsedFlags::default()),
    )
}

fn plugin_install(args: &[String]) -> CliOutcome {
    let mut yes = false;
    let mut dry_run = false;
    let mut link = false;
    let mut replace = false;
    let mut name: Option<String> = None;
    let mut sources: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--yes" => yes = true,
            "--dry-run" => dry_run = true,
            "--link" => link = true,
            "--replace" => replace = true,
            "--name" => {
                let Some(next) = args.get(i + 1) else {
                    return plugin_install_err(PLUGIN_NAME_VALUE, args);
                };
                name = Some(next.clone());
                i += 1;
            }
            _ if let Some(v) = arg.strip_prefix("--name=") => name = Some(v.to_owned()),
            _ if arg.starts_with('-') => {
                return plugin_install_err(
                    &PLUGIN_UNKNOWN_INSTALL_FLAG.replace("%q", &format!("{arg:?}")),
                    args,
                );
            }
            _ => sources.push(arg.to_owned()),
        }
        i += 1;
    }
    let _ = (link, replace, name);
    if sources.len() != 1 {
        return plugin_install_err(PLUGIN_ONE_SOURCE, args);
    }
    if !yes && !dry_run {
        return plugin_install_err(PLUGIN_INSTALL_CONFIRM, args);
    }
    CliOutcome::new(
        String::new(),
        crate::route::unmigrated_notice("plugin install"),
        2,
        verb_route("plugin", "install", args.to_vec(), ParsedFlags::default()),
    )
}

fn plugin_install_err(message: &str, args: &[String]) -> CliOutcome {
    CliOutcome::new(
        String::new(),
        format!("{message}\n"),
        2,
        verb_route("plugin", "install", args.to_vec(), ParsedFlags::default()),
    )
}

// ---------------------------------------------------------------------------
// subagent (CLI-031).
// ---------------------------------------------------------------------------

const SUBAGENT_KNOWN: [&str; 11] = [
    "list", "ls", "create", "new", "edit", "update", "delete", "remove", "rm", "try", "run",
];

/// `reasonix subagent <verb>`; verb names are case-insensitive.
pub(crate) fn subagent_command(rest: &[String]) -> CliOutcome {
    let Some(first) = rest.first() else {
        return CliOutcome::new(
            String::new(),
            format!("{SUBAGENT_USAGE}\n"),
            2,
            verb_route("subagent", "", vec![], ParsedFlags::default()),
        );
    };
    if matches!(first.as_str(), "help" | "--help" | "-h") {
        return CliOutcome::new(
            format!("{SUBAGENT_USAGE}\n"),
            String::new(),
            0,
            verb_route(
                "subagent",
                "help",
                rest[1..].to_vec(),
                ParsedFlags::default(),
            ),
        );
    }
    let verb = first.to_lowercase();
    if !SUBAGENT_KNOWN.contains(&verb.as_str()) {
        let stderr = format!(
            "{}\n\n{SUBAGENT_USAGE}\n",
            SUBAGENT_UNKNOWN_COMMAND.replace("%q", &format!("{first:?}"))
        );
        return CliOutcome::new(
            String::new(),
            stderr,
            2,
            verb_route(
                "subagent",
                &verb,
                rest[1..].to_vec(),
                ParsedFlags::default(),
            ),
        );
    }
    CliOutcome::new(
        String::new(),
        crate::route::unmigrated_notice(&format!("subagent {verb}")),
        2,
        verb_route(
            "subagent",
            &verb,
            rest[1..].to_vec(),
            ParsedFlags::default(),
        ),
    )
}

// ---------------------------------------------------------------------------
// doctor (CLI-038).
// ---------------------------------------------------------------------------

const DOCTOR_KNOWN: [&str; 8] = [
    "catalogs",
    "quality",
    "session",
    "redact-sessions",
    "capabilities",
    "runtime",
    "billing",
    "repair",
];

/// `reasonix doctor [verb]`.
pub(crate) fn doctor_command(rest: &[String]) -> CliOutcome {
    let Some(verb) = rest.first() else {
        return CliOutcome::new(
            String::new(),
            crate::route::unmigrated_notice("doctor"),
            2,
            verb_route("doctor", "", rest.to_vec(), ParsedFlags::default()),
        );
    };
    match verb.as_str() {
        "repair" if rest.len() > 1 => CliOutcome::new(
            String::new(),
            format!("{DOCTOR_REPAIR_USAGE}\n"),
            2,
            verb_route("doctor", verb, rest[1..].to_vec(), ParsedFlags::default()),
        ),
        "catalogs" if rest.len() > 1 => CliOutcome::new(
            String::new(),
            format!("{DOCTOR_CATALOGS_USAGE}\n"),
            2,
            verb_route("doctor", verb, rest[1..].to_vec(), ParsedFlags::default()),
        ),
        _ if DOCTOR_KNOWN.contains(&verb.as_str()) => CliOutcome::new(
            String::new(),
            crate::route::unmigrated_notice(&format!("doctor {verb}")),
            2,
            verb_route("doctor", verb, rest[1..].to_vec(), ParsedFlags::default()),
        ),
        _ => CliOutcome::new(
            String::new(),
            crate::route::unmigrated_notice("doctor"),
            2,
            verb_route("doctor", verb, rest[1..].to_vec(), ParsedFlags::default()),
        ),
    }
}
