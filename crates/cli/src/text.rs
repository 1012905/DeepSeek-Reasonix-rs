//! Frozen user-facing strings, copied byte-for-byte from the reference
//! `internal/base/i18n/messages_en.go` and the per-command usage bodies.
//!
//! Parity rule (AGENTS.md §11.3): wording is not rephrased here; a changed
//! string fails the byte-level contract test.

/// `i18n.M.ErrorPrefix` — prefix for every command-line usage diagnostic.
pub const ERROR_PREFIX: &str = "error:";

/// `i18n.M.UnknownCommandFmt` — `%q` is filled with the unknown verb.
pub const UNKNOWN_COMMAND_FMT: &str = "unknown command %q";

/// `i18n.M.UsageRunHint` — printed when `run` has no prompt and no piped stdin.
pub const USAGE_RUN_HINT: &str = "usage: reasonix -p [--model NAME] <task>";

/// `i18n.M.InitHint` — the `reasonix init` pointer to the in-session `/init`.
pub const INIT_HINT: &str = "Project memory (AGENTS.md) is generated in-session: run `reasonix`, then `/init` — the model analyzes the codebase and writes it. For configuration, use `reasonix setup`.";

/// `i18n.M.MinttyConsoleHint` — bare/TUI invocation under Git Bash / mintty.
pub const MINTTY_CONSOLE_HINT: &str = "The Reasonix terminal UI needs a Windows console, and this terminal (Git Bash / mintty) is not one.\nRun it from cmd, PowerShell or Windows Terminal. `reasonix run \"task\"` still works from here.\n";

/// `i18n.M.StandaloneConsoleHint` — a console this process owns alone was opened
/// by a double-click; point the person at Studio and wait for Enter.
pub const STANDALONE_CONSOLE_HINT: &str = "This is the Reasonix command-line tool; run it from a terminal such as PowerShell.\nFor the desktop app, install Reasonix Studio (ReasonixStudio-windows-amd64-installer.exe):\n  https://github.com/esengine/DeepSeek-Reasonix/releases?q=studio-v&expanded=true\n\nPress Enter to close this window.\n";

/// `i18n.M.UsageBody` — the top-level usage text printed by `help`/`--help`/`-h`
/// and by usage errors. Hidden verbs (`remote`, `plugin`, `docs-manifest`,
/// `schedule`, `sandbox`, `config auto-plan`, `config currency`) stay out on
/// purpose; do not add them.
pub const USAGE_BODY: &str = "reasonix — a config- and plugin-driven coding agent (multi-model)\n\nUsage:\n  reasonix [--model NAME] [-c|--continue] [-r|--resume [QUERY]] [--permission-mode MODE] [--effort LEVEL] [--add-dir PATH]   interactive session (the terminal UI, same as reasonix tui)\n  reasonix -p|--print [--model NAME] [--output-format text|json|stream-json] [--allowed-tools RULES] [--add-dir PATH] <task>\n  reasonix run [--model NAME] [--max-steps N] [-c|--continue] [--resume PATH] [--copy] [--output-format FORMAT] <task>\n  reasonix run --events-jsonl [--model NAME] <task>      emit redacted structured events as JSONL\n  reasonix review [--base BRANCH] [--commit SHA] [--model NAME]  AI-powered code review on local diffs\n  reasonix web [--model NAME] [--addr HOST:PORT] [--no-open]  start the local Web UI and open it in the default browser\n  reasonix serve [--model NAME] [--addr HOST:PORT] [--auth none|token|password] [--token STR] [--password STR] [--hash-password]  serve over HTTP+SSE (with optional auth)\n  reasonix acp [--model NAME]                           serve Agent Client Protocol over stdio (also: reasonix --acp)\n  reasonix setup [path]                                 interactive config wizard; writes reasonix.toml (+ .env)\n  reasonix config reasoning-language [auto|zh|en]        configure visible reasoning language\n  reasonix config compact-ratio [--local] [PERCENT]       configure automatic compaction threshold\n  reasonix config telemetry [auto|on|off]                configure content-free CLI usage metrics\n  reasonix report [list|show|send|delete] [ID]           review and explicitly send local CLI crash reports\n  reasonix mcp <add|remove|list|import>                 manage MCP servers in reasonix.toml\n  reasonix login | whoami | logout                      sign in to reasonix.io (forum and crash follow-ups only)\n  reasonix subagent <list|create|edit|delete|try|run>   manage and run isolated subagent profiles\n  reasonix init                                         show how to generate project memory (AGENTS.md)\n  reasonix doctor [--json]                              print redacted local diagnostics\n  reasonix doctor session <branch-id> [--zip] [--out PATH]  export a session conflict diagnostic zip\n  reasonix session list --json [--dir PATH]             list redacted sessions for machine clients\n  reasonix session show|status <machine-session-id> --json [--dir PATH]  query one redacted session\n  reasonix session recovery [<machine-session-id>] --json [--dir PATH]  query redacted recovery state\n  reasonix hook list|status --json [--dir PATH]         inspect redacted hook state\n  reasonix trust [--dir PATH] [--yes|--revoke]          review and approve programs this project runs\n  reasonix task list|show|status|events|stop|cancel|monitor|tmux --json [--dir PATH]\n                                                         inspect or control redacted tasks\n  reasonix upgrade [--check] [--force]                   update to the latest official release (also: reasonix update)\n  reasonix completion bash|zsh|fish                     print a shell completion script to stdout\n  reasonix version [--verbose|--json]                   print version (single line) or build metadata\n  reasonix --version | -v                               single-line version (script-safe)\n  reasonix help\n\nExamples:\n  reasonix web\n  reasonix run \"implement the TODOs in main.go\"\n  reasonix run --model mimo-pro \"add unit tests for this function\"\n  reasonix -p \"summarize this repository\" --output-format json\n  reasonix subagent run review \"review the current changes\"\n  echo \"explain this code\" | reasonix run\n\nConfiguration:\n  Resolution: flag > ./reasonix.toml > <Reasonix home>/config.toml > built-in defaults\n  Secrets come from the environment via api_key_env (e.g. DEEPSEEK_API_KEY).\n  Run 'reasonix setup' to scaffold a config; see docs/SPEC.md.\n";

/// `versionUsage` (buildinfo.go:76-86).
pub const VERSION_USAGE: &str = "Usage:\n  reasonix version\n  reasonix version --verbose\n  reasonix version --json\n  reasonix --version\n  reasonix -v\n\n--version / -v always print a single line (reasonix <version>).\nversion --verbose prints build metadata; version --json prints the same as JSON.\n";

/// `scheduleUsage` (schedule_manage.go:35-41).
pub const SCHEDULE_USAGE: &str = "usage: reasonix schedule <command>\n  add --every 2h|--at <time> --prompt <text> [--workspace dir] [--label name] [--model provider/model]\n  list\n  pause <id> | resume <id> | renew <id> | rm <id>\n  pause-all | resume-all\n  tick   claim and run what is due now, then exit (for an OS scheduler to call)\nCreating, resuming and deleting need a terminal: a person confirms them by typing back the plan id.";

/// `sandboxCommand` usage line (sandbox_cmd.go:57).
pub const SANDBOX_USAGE: &str = "usage: reasonix sandbox list | revoke   (Windows only: the folders the sandbox granted, and taking them back)";

/// `--permission-mode plan` refused in a headless `run` (cli.go:389).
pub const PLAN_REQUIRES_INTERACTIVE: &str =
    "--permission-mode plan requires an interactive session";

/// `--events-jsonl` combined with `--output-format` (cli.go:362).
pub const EVENTS_JSONL_CONFLICT: &str = "--events-jsonl cannot be combined with --output-format";

/// Approval-flag conflict messages (cli_flags.go:221-225).
pub const AUTO_YOLO_CONFLICT: &str = "--auto/-y cannot be combined with --yolo";
pub const AUTO_PERMISSION_CONFLICT: &str = "--auto/-y cannot be combined with --permission-mode";
pub const YOLO_PERMISSION_CONFLICT: &str = "--yolo cannot be combined with --permission-mode";

/// TUI refused without an interactive terminal (tui_command.go:43).
pub const TUI_NEEDS_TERMINAL: &str =
    "reasonix tui needs an interactive terminal; use `reasonix run` for scripts";

/// Environment variable marking an agent-tool child (secrets.AgentChildEnv).
pub const AGENT_CHILD_ENV: &str = "REASONIX_AGENT_CHILD";

/// `--copy` without `--resume`/`--continue` (cli.go:316).
pub const COPY_REQUIRES_RESUME: &str = "--copy requires --resume or --continue";

/// Environment variable naming the account service base URL (account.go:31).
pub const ACCOUNTS_URL_ENV: &str = "REASONIX_ACCOUNTS_URL";

/// `REASONIX_DNS` (cmd/reasonix/main.go:46-48): DNS servers installed before
/// any socket opens.
pub const DNS_ENV: &str = "REASONIX_DNS";

// ---------------------------------------------------------------------------
// Preset / profile (cli.go:260-273, preset_flags.go).
// ---------------------------------------------------------------------------

/// Live preset menu joined for the profile error text (`agentpreset.All`).
pub const PRESET_MENU_OR: &str = "balanced or delivery";
/// Live preset menu joined with `|` for the deprecation warning.
pub const PRESET_MENU_PIPE: &str = "balanced|delivery";

/// `--profile` deprecation warning (preset_flags.go:25), stderr.
pub const PROFILE_DEPRECATED_WARNING: &str =
    "warning: --profile is deprecated; use --preset balanced|delivery";

// ---------------------------------------------------------------------------
// ACP (acp.go:38-107).
// ---------------------------------------------------------------------------

pub const ACP_PLANNER_ERROR: &str = "planner must be auto or off";
pub const ACP_NETWORK_ERROR: &str = "sandbox-network must be auto, on, or off";
pub const ACP_BASH_ERROR: &str = "sandbox-bash must be auto or enforce";

// ---------------------------------------------------------------------------
// serve / web (serve_frontend.go, serve_broker.go).
// ---------------------------------------------------------------------------

pub const SERVE_RESUME_SESSION_ID_CONFLICT: &str =
    "--resume and --session-id cannot be used together";
pub const BROKER_TWICE_ERROR: &str =
    "--provider-broker and --provider-broker-file name the broker twice; give one";
pub const BROKER_TOKEN_FILE_ERROR: &str = "--provider-broker needs --provider-broker-token-file";

// ---------------------------------------------------------------------------
// mcp (mcp.go:31-128).
// ---------------------------------------------------------------------------

pub const MCP_LIST_UNEXPECTED: &str = "mcp list: unexpected argument %q";
pub const MCP_IMPORT_UNEXPECTED: &str = "mcp import: unexpected argument %q";
pub const MCP_UNKNOWN_SUBCOMMAND: &str = "unknown mcp subcommand %q";
pub const MCP_BROWSE_LIMIT_VALUE: &str = "mcp browse: --limit needs a value";
pub const MCP_BROWSE_LIMIT_RANGE: &str = "mcp browse: --limit must be between 1 and 100";
pub const MCP_BROWSE_UNKNOWN_FLAG: &str = "mcp browse: unknown flag %q";
pub const MCP_BROWSE_ONE_QUERY: &str = "mcp browse: provide at most one search query";

pub const MCP_USAGE: &str = "Manage MCP servers (global installs use config.toml; project entries stay in project config).

Usage:
  reasonix mcp list
  reasonix mcp get <name>
  reasonix mcp install <registry-name> [--as <name>]
  reasonix mcp add -- <command> [args...]            stdio argv (no shell)
  reasonix mcp add <name> -- <command> [args...]
  reasonix mcp add <name> <command> [args...]        legacy stdio form
  reasonix mcp add https://example.com/mcp           remote HTTP
  reasonix mcp add <name> --http <url> [--header K=V]
  reasonix mcp add <name> --sse  <url>
  reasonix mcp enable <name>
  reasonix mcp disable <name>
  reasonix mcp retry <name>
  reasonix mcp trust <name> [--digest <hex>]    accept tool definitions held back as changed
  reasonix mcp auth <name> [--upgrade]              remote OAuth (opens browser); --upgrade grants the access a server asked for
  reasonix mcp update <name>
  reasonix mcp browse [query] [--limit N] [--json]
  reasonix mcp import
  reasonix mcp remove <name>

Flags for add:
  --http <url> | --sse <url>   remote transport (omit for a stdio command)
  --env K=V                    set an environment variable (repeatable, stdio)
  --header K=V                 set an HTTP header (repeatable, remote)

Examples:
  reasonix mcp add fs npx -y @modelcontextprotocol/server-filesystem .
  reasonix mcp add stripe --http https://mcp.stripe.com --header \"Authorization=Bearer $STRIPE_KEY\"

CLI config changes take effect on the next session. Inside a running chat,
press n in the /mcp panel to save and connect a server immediately. Installing
a server is also its launch authorization; there is no separate trust step.
Remote OAuth, when requested by the server, is completed with reasonix mcp auth
<name> and stored in Reasonix-private MCP state.

Servers declared by project reasonix.toml or .mcp.json are trusted configuration
and need no separate launch confirmation. Project entries override same-name
global entries; within a project, reasonix.toml overrides .mcp.json. Writer or
destructive annotations never trigger per-call approval. Explicit deny rules
still win; Plan Mode and strict read-only subagents may filter which tools are
available.";

// ---------------------------------------------------------------------------
// plugin (plugin.go:22-134).
// ---------------------------------------------------------------------------

pub const PLUGIN_UNKNOWN_COMMAND: &str = "unknown plugin command %q";
pub const PLUGIN_NAME_VALUE: &str = "--name requires a value";
pub const PLUGIN_UNKNOWN_INSTALL_FLAG: &str = "unknown plugin install flag %q";
pub const PLUGIN_ONE_SOURCE: &str = "plugin install requires exactly one source";
pub const PLUGIN_INSTALL_CONFIRM: &str =
    "plugin install writes files; re-run with --yes to apply, or --dry-run to preview";

pub const PLUGIN_USAGE: &str = "usage:
  reasonix plugin install <source> [--yes] [--dry-run] [--link] [--replace]
  reasonix plugin list
  reasonix plugin show <name>
  reasonix plugin enable <name>
  reasonix plugin disable <name>
  reasonix plugin remove <name> --yes
  reasonix plugin doctor <name>
  reasonix plugin migrate <name> --to-v2";

// ---------------------------------------------------------------------------
// subagent (subagent.go:26-62).
// ---------------------------------------------------------------------------

pub const SUBAGENT_UNKNOWN_COMMAND: &str = "unknown subagent command %q";

pub const SUBAGENT_USAGE: &str = "usage:
  reasonix subagent list [--dir PATH]
  reasonix subagent create <name> --description TEXT (--prompt TEXT | --prompt-file PATH) [--scope project|global] [--model REF] [--effort LEVEL] [--tools a,b] [--color NAME] [--dir PATH]
  reasonix subagent edit <name> [--description TEXT] [--prompt TEXT | --prompt-file PATH] [--model REF] [--effort LEVEL] [--tools a,b] [--color NAME] [--dir PATH]
  reasonix subagent delete <name> --yes [--dir PATH]
  reasonix subagent try <name> [--model REF] [--max-steps N] [--dir PATH] <task>
  reasonix subagent run <name> [--model REF] [--max-steps N] [--dir PATH] <task>

Use --prompt-file - or pipe stdin to read a system prompt from stdin.";

// ---------------------------------------------------------------------------
// remote (remote.go:21-100, 427-445).
// ---------------------------------------------------------------------------

pub const REMOTE_UNKNOWN_SUBCOMMAND: &str = "unknown remote subcommand %q";
pub const REMOTE_REFUSED_AGENT_CHILD: &str = "reasonix remote %s is refused: this command was started by an agent tool (%s is set); it changes the machine book or opens a link, which a person does";
pub const REMOTE_REMOVED_STUB: &str =
    "reasonix remote %s: Remote Workbench 已移除，请使用 `reasonix remote connect <host> --open`";

pub const REMOTE_USAGE: &str = "Manage remote SSH hosts and their persistent serve (user-global config).

Usage:
  reasonix remote add <name> [user@]host[:port] [--identity F] [--jump SPEC]
                     [--workspace PATH] [--use-ssh-config] [--serve-install auto|npm|upload|never]
                     [--passphrase-env NAME] [--password-env NAME]
  reasonix remote list
  reasonix remote remove <name>
  reasonix remote import [alias...|--all]      # from ~/.ssh/config
  reasonix remote test <name|user@host>        # dial, auth, host key, and whether a kernel can run there
  reasonix remote wsl list | wsl test <distribution>   # WSL distributions on this Windows machine
  reasonix remote wsl add <name> <distribution> [--workspace PATH] [--serve-install MODE]   # connect/test/open then work on it
  reasonix remote connect <name> [--workspace PATH] [--local-port N] [--no-serve] [--open] [--forward-only]
  reasonix remote open <name>                  # connect --open
  reasonix remote status [<name>]
  reasonix remote forward add <host> (-L|-R) <spec> | forward rm <host> <name> | forward ls <host>
  reasonix remote serve start|stop|status|logs <name> [--workspace PATH] [-n N]
  reasonix remote fs ls <name>:<path> | fs get <name>:<remote> [local] | fs put <local> <name>:<remote>";

// ---------------------------------------------------------------------------
// doctor (doctor.go:86-111, catalogs.go:111).
// ---------------------------------------------------------------------------

pub const DOCTOR_REPAIR_USAGE: &str =
    "usage: reasonix doctor repair [--root PATH] [--apply] [--project] [--json]";
pub const DOCTOR_CATALOGS_USAGE: &str = "usage: reasonix doctor catalogs [--json]";

// ---------------------------------------------------------------------------
// config (cli.go:1345-1770).
// ---------------------------------------------------------------------------

pub const CONFIG_CURRENCY_LOCAL_ERROR: &str =
    "currency is user-level only; --local is not supported";
pub const CONFIG_AUTO_PLAN_LOCAL_ERROR: &str =
    "auto-plan is user-level only; --local is not supported";
pub const CONFIG_LANGUAGE_VALUE_ERROR: &str = "reasoning_language %q: must be auto|zh|en";
pub const CONFIG_CURRENCY_VALUE_ERROR: &str = "pricing currency %q: must be auto|CNY|USD";
pub const COMPACT_RATIO_REQUIREMENT: &str = "percentage above 0 and below 100";
pub const COMPACT_RATIO_VALUE_ERROR: &str =
    "compact ratio must be a percentage above 0 and below 100: %w";

pub const CONFIG_USAGE: &str = "Usage:
  reasonix config reasoning-language [--local] [auto|zh|en]
  reasonix config compact-ratio [--local] [PERCENT] (percentage above 0 and below 100)
  reasonix config currency [auto|CNY|USD]
  reasonix config telemetry [auto|on|off]";
pub const CONFIG_TELEMETRY_USAGE: &str = "Usage:
  reasonix config telemetry [auto|on|off]";
pub const CONFIG_AUTO_PLAN_USAGE: &str = "Usage:
  reasonix config auto-plan [off]";
pub const CONFIG_LANGUAGE_USAGE: &str = "Usage:
  reasonix config reasoning-language [--local] [auto|zh|en]";
pub const CONFIG_CURRENCY_USAGE: &str = "Usage:
  reasonix config currency [auto|CNY|USD]";
pub const CONFIG_COMPACT_RATIO_USAGE: &str = "Usage:
  reasonix config compact-ratio [--local] [PERCENT] (percentage above 0 and below 100)";

// ---------------------------------------------------------------------------
// report (report.go:20-160, messages_en.go:733-738).
// ---------------------------------------------------------------------------

pub const REPORT_USAGE_BODY: &str = "Usage:
  reasonix report                 preview the newest local report and confirm sending
  reasonix report list            list local CLI crash reports
  reasonix report show [ID]       preview a report (newest when ID is omitted)
  reasonix report send [ID]       send a reviewed report and delete it after success
  reasonix report delete [ID]     delete a local report without sending";

// ---------------------------------------------------------------------------
// completion (shell_completion.go:402-441).
// ---------------------------------------------------------------------------

pub const COMPLETION_UNKNOWN_SHELL: &str = "unknown shell %q (want bash, zsh, or fish)";
pub const COMPLETION_USAGE: &str = "Usage:
  reasonix completion bash
  reasonix completion zsh
  reasonix completion fish

The command prints a completion script to stdout. Source it directly or save it
in your shell's completion directory.";

// ---------------------------------------------------------------------------
// docs-manifest (docs_manifest.go:17-69).
// ---------------------------------------------------------------------------

pub const DOCS_MANIFEST_FLAGS_ONLY: &str = "error: docs-manifest accepts flags only";

// ---------------------------------------------------------------------------
// upgrade (upgrade.go:75-145).
// ---------------------------------------------------------------------------

pub const UPGRADE_ONE_POSITIONAL: &str =
    "upgrade accepts at most one deprecated positional channel";
pub const UPGRADE_CHANNEL_VALUE: &str = "--channel requires a legacy channel value";
pub const UPGRADE_CHANNEL_UNSUPPORTED: &str =
    "release channel %q is unsupported; Reasonix now uses the official release";
pub const UPGRADE_CHANNEL_CONFLICT: &str =
    "conflicting release channels: positional %q and --channel %q";

// ---------------------------------------------------------------------------
// review (review.go:22-41).
// ---------------------------------------------------------------------------

pub const REVIEW_EMPTY_DIFF: &str = "No changes to review.";

// ---------------------------------------------------------------------------
// setup (setup_flags.go:13-37).
// ---------------------------------------------------------------------------

pub const SETUP_UNKNOWN_FLAG: &str = "unknown setup flag %q";
pub const SETUP_USAGE: &str = "usage: reasonix setup [--local|-l] [path]
Interactive configuration wizard. Writes a reasonix config and stores the
provider API key in Reasonix's credential file, never in the config itself.
  --local, -l   write ./reasonix.toml instead of the user-global config
  path          write the config to this path instead of the default";

// ---------------------------------------------------------------------------
// account (account.go:47-159).
// ---------------------------------------------------------------------------

pub const ACCOUNT_UNEXPECTED_ARG: &str = "%s: unexpected argument %q";
pub const LOGIN_CANCELLED: &str = "\nlogin: cancelled.";

// ---------------------------------------------------------------------------
// trust (trust.go:142-165).
// ---------------------------------------------------------------------------

pub const TRUST_UNKNOWN_ARG: &str =
    "usage: reasonix trust [--dir PATH] [--yes [--allow-providers]|--revoke] (unknown argument %q)";
pub const TRUST_CHOOSE_ONE: &str =
    "usage: reasonix trust [--dir PATH] [--yes|--revoke] (choose one)";
pub const TRUST_NON_INTERACTIVE: &str =
    "Nothing approved. Review the above, then run `reasonix trust --yes` here to approve.";
pub const TRUST_PROVIDER_HELD_BACK: &str = "Not approved: [%s] %s receives your API key at its own address; approve it by name with --allow-providers.";
pub const TRUST_APPROVED: &str = "Approved. Any change to a program it names needs approval again.";
pub const TRUST_REVOKED: &str = "Revoked the folder trust and every program approval for %s.";

// ---------------------------------------------------------------------------
// task (task.go:62-150, task_machine.go).
// ---------------------------------------------------------------------------

pub const TASK_USAGE: &str = "usage: reasonix task <list|show|monitor|status|events|stop|cancel|requeue|open-session|tmux> [flags]";
pub const TASK_MONITOR_USAGE: &str =
    "usage: reasonix task monitor <list|status|events|stop|cancel|requeue|open-session> [flags]";
pub const TASK_TMUX_USAGE: &str = "usage: reasonix task tmux <attach|status|open|detach>";
pub const TASK_UNKNOWN_SUBCOMMAND: &str = "unknown task subcommand: %s";
pub const TASK_MONITOR_UNKNOWN_SUBCOMMAND: &str = "unknown task monitor subcommand: %s";
pub const TASK_TMUX_UNKNOWN_SUBCOMMAND: &str = "unknown task tmux subcommand: %s";

// ---------------------------------------------------------------------------
// sessions maintenance / catalogs (sessions_maintenance.go, catalogs.go).
// ---------------------------------------------------------------------------

pub const SESSIONS_MAINTENANCE_USAGE: &str =
    "usage: reasonix sessions <reindex|diagnose|cleanup> [--dir PATH] [--json]";
pub const CATALOGS_REINDEX_USAGE: &str = "usage: reasonix catalogs reindex <catalog> [options]";
pub const CATALOGS_UNKNOWN: &str = "unknown catalog: %s";

// ---------------------------------------------------------------------------
// Machine interface shared messages (session_machine.go, hook_machine.go,
// task_machine.go).
// ---------------------------------------------------------------------------

pub const MACHINE_HELP_HINT: &str = "use the documented machine interface";
pub const MACHINE_JSON_REQUIRED: &str = "--json is required";
