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
