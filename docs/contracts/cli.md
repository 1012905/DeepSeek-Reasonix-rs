# CLI Contract — Frozen Inventory (M1)

> Area: CLI commands, flags, exit codes, usage text, env vars.
> Reference: `reference/DeepSeek-Reasonix-studio` (Go). All file citations below are relative to
> `reference/DeepSeek-Reasonix-studio/` unless prefixed `docs/`.
> Status: frozen from source on 2026-11-15. Rust crate target: `reasonix-cli` (frontend).
> Behavior oracle priority (AGENTS.md §11.3): actual run > reference tests/golden > source+docs > this file.

## 1. Process entry point

- `cmd/reasonix/main.go:25-29`: build identity via ldflags: `version = "dev"`, `gitCommit = ""`, `buildTimeUTC = ""`. `version` is the single-line contract for `--version`.
- `cmd/reasonix/main.go:46-48`: `init()` installs DNS servers from `REASONIX_DNS` (comma/space separated) before any socket opens (CGO-free static binary cannot use system resolvers).
- `cmd/reasonix/main.go:50-53`: `main()` sets provider client version, then `os.Exit(runWithCrashCapture(os.Args[1:], version))`.
- `cmd/reasonix/main.go:55-69`: crash capture wraps the whole CLI: `crashreport.CaptureFatalDumps(home)`, `InstallFatalOutput(home)`; a recovered panic is captured via `crashreport.CapturePanic` then re-raised.
- `cmd/reasonix/main.go:14-20`: blank imports register built-in model providers (anthropic, gemini, openai, responses) and builtin tools.

## 2. Argument normalization (before dispatch)

- `internal/frontend/cli/cli.go:1312-1330` `normalizeCommand`:
  1. `--acp` → command `acp`.
  2. `leadingFlagsIntoRun` may move leading session flags after the verb `run` (see §2.1).
  3. Leading `-p`/`--print` → `run --print <rest>`.
  4. First token is an interactive flag (`isDefaultInteractiveFlag`, cli.go:171-180) → bare interactive invocation (command = TUI).
- `internal/frontend/cli/cli.go:171-180` `isDefaultInteractiveFlag` accepts exactly: `--model`, `--max-steps`, `--continue`, `-c`, `--resume`, `-r`, `--copy`, `--dangerously-skip-permissions`, `--yolo`, `--permission-mode`, `--effort`, `--dir`, `--add-dir`, `--allowed-tools`, `--allowedTools`, `--profile`, `--preset`, `--agent-composition` (with `=` value form, recursing on the name after `=`).
- Bare invocation (`internal/frontend/cli/cli.go:79-87`): no command at all → TUI when **stdin and stdout are both terminals**; otherwise `bareOutsideConsole`.
- `internal/frontend/cli/bare_usage.go:15-37`:
  - `bareUsage` prints `usage()`; when the process owns the console alone (double-clicked on Windows), also prints `StandaloneConsoleHint` (i18n.M, messages_en.go:748) and waits for Enter.
  - Bare with no args at all → exit **0**; session flags without a console → exit **2**.
  - `bareOutsideConsole`: mintty/Git Bash → `MinttyConsoleHint` (messages_en.go:746) exit **2**; otherwise `bareUsage`.

### 2.1 Leading-flag routing into run

- `internal/frontend/cli/cli_flags.go:267-284` `leadingFlagsIntoRun`: leading flags are moved after `run` **only when** the TUI flag set (plus `-y`, `-p`) and the run flag set parse the leading block identically (equal flag values, zero positionals), the verb is exactly `run`, and no `--` terminator appears in the lead.
- `internal/frontend/cli/cli_flags.go:236` `startsWithSessionFlag`: `-y`, `--auto`, `-p`, `--print`, any `isDefaultInteractiveFlag`, `--auto=` prefix.
- `internal/frontend/cli/cli_flags.go:54-85`: `hasLeadingPrintFlag`/`stripLeadingPrintFlag` treat standalone `-p`/`--print` before a `--` terminator as print intent.
- Leading run-only flag (e.g. `--output-format`) stays with the TUI parser, which reports it (exit 2). Leading TUI-only flag (`-r`, bare `--resume`) → whole line goes to TUI.

### 2.2 Legacy config migration trigger

- `internal/frontend/cli/cli.go:182-189` `shouldMigrateLegacyConfigForCLI`: commands `""`, `run`, `serve`, `web`, `setup`, `config`, `init`, `acp`, `mcp`, `remote`, `plugin`, `subagent`, `doctor`, `upgrade`, `update`, `login`, `whoami`, `logout` migrate legacy config first.
- `internal/frontend/cli/schedule_cmd.go:14`: language is read from config for every command except `doctor repair` and `schedule`.

## 3. Top-level dispatch table

`internal/frontend/cli/cli.go:90-164` — in order:

| Command (aliases) | Handler | Notes |
|---|---|---|
| `run` | `runAgent` (cli.go:337) | one-shot agent; see §7.1 |
| `serve` | `runServe` | HTTP+SSE host; flags §7.6 |
| `tui` `chat` `code` | `runTUI` | interactive terminal UI; flags §7.2 |
| `web` | `runWebCommand` | serve + open browser |
| `setup` | `setupConfig` (cli.go:632-657) | wizard; §7.14 |
| `config` | `configCommand` (cli.go:1345-1365) | §7.9 |
| `init` | prints `InitHint`, exit 0 | how to generate AGENTS.md |
| `acp` | `acpCommand` (acp.go:38-107) | ACP over stdio; §7.5 |
| `mcp` | `mcpCommand` (mcp.go:31-84) | §7.8 |
| `login` `whoami` `logout` | `accountCommand` (account.go:36-45) | §7.11 |
| `remote` | `remoteCommand` (remote.go:21-67) | §7.10 |
| `plugin` | `pluginCommand` (plugin.go:22-52) | §7.7 |
| `subagent` | `subagentCommand` (subagent.go:37-62) | §7.12 |
| `doctor` | `doctorCommand` (doctor.go:41-84) | §7.15 |
| `report` | `reportCommand` (report.go:20-99) | crash reports; §7.16 |
| `session` `sessions` `catalogs` | `runSessionOrCatalogCommand` (catalogs.go:93-102) | §7.17–7.19 |
| `hook` `hooks` | `hookCommand` (hook_machine.go:45) | §7.18 |
| `trust` | `trustCommand` (trust.go:23) | §7.13 |
| `task` | `taskCommand` (task.go:62-103) | §7.20 |
| `review` | `reviewCommand` (review.go:22-122) | §7.4 |
| `upgrade` `update` | `upgradeCommand` (upgrade.go:181) | §7.3 |
| `version` | `versionCommand(rest, info, allowFlags=true)` (buildinfo.go:32) | §7.21 |
| `--version` `-v` | `versionCommand(nil, info, allowFlags=false)` | single line |
| `completion` | `completionCommand` (shell_completion.go:402-431) | bash/zsh/fish |
| `docs-manifest` | `docsManifestCommand` (docs_manifest.go:17-69) | release diagnostic, undocumented |
| `help` `--help` `-h` | `usage()` to stdout, exit 0 | |
| anything else | `unknownCommand` (schedule_cmd.go:20-30) | `schedule` → hidden `scheduleCommand`; `sandbox` → hidden `sandboxCommand`; else `i18n.M.UnknownCommandFmt` on stderr + `usage()` + exit **2** |

Not listed in top-level usage text (intentional): `remote`, `plugin`, `docs-manifest`, `schedule`, `sandbox`, `config auto-plan`, `config currency` (see §8.1 for the exact usage body).

## 4. Flag parsing rules

- Two flag styles coexist:
  - Go stdlib `flag` sets (`flag.NewFlagSet`, `ContinueOnError`) for most verbs.
  - `spf13/pflag` sets for `run`, `tui`, `upgrade` (interspersed parsing).
- `internal/frontend/cli/flag_parse.go:76-86` `parseCommandFlags`: on help request, parse output goes to **stdout** and the command exits **0**; on malformed input, `error: <err>` (`i18n.M.ErrorPrefix`) and exit **2**.
- `internal/frontend/cli/flag_parse.go:36-46` `commandHelpRequested`: `-h`/`--help` in the positional prefix requests help.
- Stdlib `flag` stops at the first positional (relevant for `remote add`, remote.go:125-130); pflag is interspersed.

### 4.1 Permission-mode parsing

`internal/frontend/cli/cli.go:235-258` `parsePermissionMode`:

| Input | Result |
|---|---|
| `""` `default` `ask` | Ask |
| `read-only` `readonly` | ReadOnly |
| `auto` `workspace-write` | Auto |
| `acceptedits` `accept-edits` | Ask + allow [write_file, edit_file, multi_edit, move_file, notebook_edit, delete_range, delete_symbol] |
| `manual` | Ask |
| `dontask` `dont-ask` | DontAsk |
| `plan` | Ask + plan |
| `bypasspermissions` `bypass-permissions` `yolo` `danger-full-access` | Yolo |

Error: `unknown permission mode %q (want read-only, manual, ask, auto, acceptEdits, dontAsk, plan, or bypassPermissions; 1.x's workspace-write and danger-full-access also work)`.

### 4.2 Runtime profile (preset) parsing

`internal/frontend/cli/cli.go:260-273` `parseRuntimeProfile`:

| Input | Result |
|---|---|
| `""` `balanced` `full` | TokenModeFull |
| `economy` `light` `lite` `eco` | TokenModeEconomy |
| `delivery` `deliver` `quality` | TokenModeDelivery |

Error: `unknown execution setting %q (want %s; legacy: economy, full)`. `--profile` is the deprecated alias of `--preset` (warnProfileDeprecated); both flags default to `balanced` where registered.

### 4.3 Approval-flag conflicts

`internal/frontend/cli/cli_flags.go:211-232`:

- Registered approval flags: `--auto`/`-y`, `--yolo`, `--dangerously-skip-permissions` (also `--yolo`/`--dangerously-skip-permissions` are equivalent).
- `--auto` + `--yolo` → `--auto/-y cannot be combined with --yolo` (exit 2).
- `--auto` + explicit `--permission-mode` → `--auto/-y cannot be combined with --permission-mode` (exit 2).
- `--yolo` + explicit `--permission-mode` → `--yolo cannot be combined with --permission-mode` (exit 2).
- TUI has **no** `--auto` (tui_flags.go:30-52); `--yolo` + `--permission-mode` errors there too.

### 4.4 Session query resolution (resume/continue/copy)

`internal/frontend/cli/cli_flags.go:104-162` `resolveSessionQuery`:

- Empty or the TUI picker sentinel → "".
- Existing file path → absolute path.
- Machine session ID (`session_` + 32 hex, `looksLikeMachineSessionID` cli_flags.go:191) → resolved via the machine identity key (`loadMachineIdentityKey`).
- Exact matches (id, basename, path) preferred over partial matches (substring of id, base, CustomTitle, TopicTitle, Preview).
- 0 matches → `no session matches %q`; >1 → `session query %q is ambiguous (%d matches)` + reportResumeQueryError listing matches.
- `normalizeOptionalResumeArg` (cli_flags.go:90-102): `--resume`/`-r` followed by a non-dash arg becomes `--resume=<value>`.
- TUI `--resume` has NoOptDefVal `__reasonix_resume_picker__` (bare `--resume`/`-r` opens the picker).
- `--copy` without `--resume`/`--continue` (and not fresh) → `--copy requires --resume or --continue` (exit 2, cli.go:313-335). Copy notice `continuing in a session copy: %s` → stdout for text non-print mode, stderr otherwise.

## 5. Exit-code summary

| Code | Meaning | Source |
|---|---|---|
| 0 | success (run finishes 0 even with permission refusals); bare usage; help; `-h` | run_output.go, cli.go |
| 1 | runtime failure: config load, provider, session/lease, boot, upgrade dev build, report errors, copy failure | various |
| 2 | usage/argument errors: flag parse, unknown command/subcommand, invalid values, conflicting flags | flag_parse.go, all verbs |
| 4 | untrusted-folder refusal in `run` (`runExitUntrustedFolder`) | run_verdict.go:19 |
| 77 | refused because started by an agent tool (`exitAgentChild`, agent_child.go:11) or schedule needs a human (`scheduleExitNotHuman`, schedule_manage.go:32) | agent_child.go, schedule_manage.go:28-33 |
| 130 | `login` cancelled by Ctrl-C | account.go:81 |
| exit(status) | panic re-raised after crash capture | main.go:63-66 |

`run` completion mapping (cli.go:337-581, run_completion.go, run_verdict.go): provider/config/limit/cancellation failure → 1; untrusted-folder refusal → 4; finished (including refusals) → 0.

## 6. Environment variables (CLI surface)

| Variable | Effect | Source |
|---|---|---|
| `REASONIX_DNS` | DNS servers (comma/space separated) installed before any socket | cmd/reasonix/main.go:46-48, internal/base/netdns |
| `REASONIX_ACCOUNTS_URL` | account service base URL (login/whoami/logout) | internal/frontend/cli/account.go:31 |
| `REASONIX_AGENT_CHILD` | `1` marks agent-tool children; person-only commands refuse with exit 77 | internal/base/secrets (AgentChildEnv), agent_child.go:9-17 |
| `GITHUB_TOKEN` / `GH_TOKEN` | GitHub API token for upgrade release lookups | internal/frontend/cli/upgrade.go:499-506 |
| `REASONIX_HOME` | Reasonix home override (config paths) | config package (paths contract, separate doc) |
| provider `api_key_env` (e.g. `DEEPSEEK_API_KEY`) | model credentials come from env, never config | UsageBody §8.1, cli.go:1156-1184 |

## 7. Command matrix

### 7.1 `reasonix run` (one-shot)

- Flag set (`internal/frontend/cli/run_flags.go:23-53`, pflag, interspersed): `--model`, `--profile` (deprecated), `--preset` (default `balanced`), `--agent-composition`, `--max-steps` (0 = automatic), `--show-thinking`, `--debug` (NoOptDefVal `true`), `--metrics PATH`, `--trajectory PATH`, `--ablate`/`--fold-index` (arms: evidence, planner, subagent, retrieval, compaction, upstream, recall-search, none, all), `--dir`, `--continue`/`-c`, `--resume`, `--takeover` (1.x compat, lease still refused), `--copy`, `--effort`, `--permission-mode`, `--auto`/`-y`, `--yolo`, `--dangerously-skip-permissions`, `--print`/`-p`, `--events-jsonl`, `--output-format` (default `text`), `--add-dir` (repeatable), `--allowed-tools`, `--allowedTools` (alias).
- Exit-code map (cli.go:337-581): flag parse → 2; §4.3 conflicts → 2; allowed-tools parse → 2; `--events-jsonl` + `--output-format` → `--events-jsonl cannot be combined with --output-format` → 2; profile → 2; ablation arm → 2; permission-mode → 2; `--permission-mode plan` → `--permission-mode plan requires an interactive session` → 2; `--dir` chdir failure → 2; empty prompt and empty stdin → `UsageRunHint` → 2; machine identity unavailable (events-jsonl) → 1; session query errors → 1; lease held → 1; profile setup failure → 1; final exit from completion classifier (0/1/4).
- Output formats (`--output-format`): `text`, `json`, `stream-json`; `--events-jsonl` is a separate redacted JSONL stream (schema_version, sequence, kind; final kind `run_done`) and cannot combine with `--output-format`.
- Prompt: positional words; stdin read when no positional and stdin is piped.

### 7.2 `reasonix tui` (aliases `chat`, `code`; also bare)

- Flag set (`internal/frontend/cli/tui_flags.go:30-52`): `--model`, `--preset` (default `balanced`), `--agent-composition`, `--profile` (deprecated), `--dir`, `--inline` (scrollback view instead of full screen; **only** `tui` registers it — bare `--inline` is an unknown command, exit 2), `--continue`/`-c`, `--resume`/`-r` (optional value → picker), `--copy`, `--effort`, `--permission-mode` (`plan` allowed here), `--yolo`, `--dangerously-skip-permissions`, `--max-steps`, `--add-dir`, `--allowed-tools`, `--allowedTools`.
- No `--auto` in TUI. `--yolo` + `--permission-mode` → error (§4.3).

### 7.3 `reasonix upgrade` (alias `update`)

- `internal/frontend/cli/upgrade.go:60-199`: stable tag pattern `^v(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)$`; required release assets: reasonix-{darwin-amd64,darwin-arm64,linux-amd64,linux-arm64}.tar.gz, reasonix-{windows-amd64,windows-arm64}.zip, SHA256SUMS.
- Flags (pflag): `--check` (check only), `--force` (reinstall), `--channel` (deprecated legacy value), plus at most one deprecated positional channel.
- Channel parsing (`parseCLIReleaseChannel`): `""`/`stable`/`preview`/`canary`/`beta`/`next` → stable; anything else → `release channel %q is unsupported; Reasonix now uses the official release`.
- Errors: >1 positional → `upgrade accepts at most one deprecated positional channel` (2); empty `--channel` value → `--channel requires a legacy channel value` (2); positional vs `--channel` conflict → `conflicting release channels: positional %q and --channel %q` (2); dev build → `UpgradeDevBuild` (1).
- `GITHUB_TOKEN`/`GH_TOKEN` authenticate release lookups (upgrade.go:499-506).

### 7.4 `reasonix review`

- `internal/frontend/cli/review.go:22-122`. Flags: `--base` (default HEAD = uncommitted working tree), `--commit SHA`, `--model`, `--instructions`.
- Empty diff → `No changes to review.` exit 0. All failures (diff, config, model resolve, provider, review skill missing/not a subagent, run) → 1.
- Runs a user-scope-only read-only subagent (MaxSteps 12); the reviewed checkout is untrusted: no project hooks, user-only config.

### 7.5 `reasonix acp` (also `--acp`)

- `internal/frontend/cli/acp.go:38-107`. Flags: `--model`, `--profile` (default `balanced`; no `--preset` here), `--planner` (`auto`|`off`, default `auto`; other → `planner must be auto or off`, exit 2), `--sandbox-network` (`auto`|`on`|`off`, default `auto`; other → `sandbox-network must be auto, on, or off`, exit 2), `--sandbox-bash` (`auto`|`enforce`, default `auto`), `--permission-mode` (read-only, dontAsk, manual, ask, auto, acceptEdits, bypassPermissions; `plan` → `--permission-mode plan requires an interactive session`, exit 2), `--workspace-only`.
- AgentInfo name `reasonix`; PostureCeiling from `--permission-mode` (clients cannot raise); `--planner=off` maps to ablation planner fold. Serve error → exit 1.

### 7.6 `reasonix serve` / `reasonix web`

- Shared `runServeWithOptions` (`internal/frontend/cli/serve_frontend.go:242-426`). Flags: `--model`, `--profile` (deprecated), `--preset` (default `balanced`), `--agent-composition`, `--max-steps`, `--addr` (default `127.0.0.1:8787`), `--resume`, `--session-id` (web only; validated), `--auth` (`none`|`token`|`password`), `--token`, `--token-file`, `--password`, `--password-file`, `--hash-password` (print bcrypt hash and exit), `--allow-open-network`, `--behind-proxy`, `--public-url`, `--port-file`, `--pid-file`, `--page` (built Studio page dir served under `/_studio/`), `--dir`, broker flags (`--provider-broker`, `--provider-broker-file`, `--provider-broker-token-file`; serve_broker.go:25-34), `--open`, `--no-open`.
- `--resume` + `--session-id` → `--resume and --session-id cannot be used together` (2). Open-bind refusal → 2. Resume lease held → `SessionInUseMessage` + `SessionLeaseCloseHint` (1). Boot failure → 1. Profile errors → 2.
- `web` auth help differs: `auth mode: none, token, or password (default: generated token)`.
- Broker resolver errors (serve_broker.go:39-51): `--provider-broker and --provider-broker-file name the broker twice; give one`; `--provider-broker needs --provider-broker-token-file` (exit 2).

### 7.7 `reasonix plugin`

- `internal/frontend/cli/plugin.go:22-52`. Verbs: `install`, `list`, `show`, `remove`/`uninstall`, `enable`, `disable`, `doctor`, `migrate`, `help`/`--help`/`-h`. Bare → usage, exit **0**. Unknown → `unknown plugin command %q` + usage (2).
- `plugin install <source>` flags (plugin.go:101-130): `--yes`, `--dry-run`, `--link`, `--replace`, `--name` (or `--name=`). Without `--yes`/`--dry-run` → `plugin install writes files; re-run with --yes to apply, or --dry-run to preview` (2).
- Usage text: plugin.go:54-64.

### 7.8 `reasonix mcp`

- `internal/frontend/cli/mcp.go:31-84`. Bare → `mcpUsage` (2); `<verb> -h|--help` → usage (0). Verbs: `list`/`ls`, `add`, `get`, `remove`/`rm`, `enable`, `disable`, `retry`/`connect` (compat alias), `trust`, `auth`/`authorize`, `update`, `import`, `browse`/`search`, `install`, `help`/`-h`/`--help`. Unknown → `unknown mcp subcommand %q` + usage (2).
- `list` with extra arg → `mcp list: unexpected argument %q` (2); same for `import`.
- `browse`: hand-rolled parser — `--json`, `--limit N` (1..100, default 20; errors `mcp browse: --limit needs a value`, `mcp browse: --limit must be between 1 and 100`, unknown flag; all 2).
- Full usage text: mcp.go:510-549 (add forms: stdio argv, `--http`, `--sse`; `--env`/`--header` repeatable).

### 7.9 `reasonix config`

- `internal/frontend/cli/cli.go:1345-1365`: subcommands `auto-plan`, `reasoning-language`, `compact-ratio`, `currency`, `telemetry`; bare/unknown → `configUsage` (cli.go:1671-1678) exit 2.
- `currency` (cli.go:1367): `--local` → `currency is user-level only; --local is not supported` (2); show `currency = %q (display: %s)`; set errors 2 (parse) / 1 (save).
- `telemetry` (cli.go:1438): show `cli_metrics = %q`; set `auto`|`on`|`off`; `off` also deletes pending metrics.
- `auto-plan` (cli.go:1486-1512): retired; `--local` rejected (2); prints `auto_plan = "off"`; enabling rejected.
- `reasoning-language` (cli.go:1514-1582): `--local` writes `./reasonix.toml`; show `reasoning_language = %q`; set `auto`|`zh`|`en`.
- `compact-ratio` (cli.go:1584-1654): `--local`; show `compact_ratio = %s (%s)` with source resolution project > user > built-in default (`compactRatioSource` cli.go:1656-1664); set errors 2; percent formatting trims trailing zeros (cli.go:1666-1669).

### 7.10 `reasonix remote`

- `internal/frontend/cli/remote.go:21-67`. Bare → usage (2). Agent-child guard: non-read-only verbs refuse with exit 77 when `REASONIX_AGENT_CHILD=1` (remote.go:26-29).
- Verbs: `add`, `list`/`ls`, `remove`/`rm`, `import`, `test`, `connect`/`open`, `wsl`, `status`, `forward`, `serve`, `fs`, `attach-workspace` (removed stub), `runtime-workbench` (removed stub), `workbench-build-id` (removed stub), `help`/`-h`/`--help`. Unknown → `unknown remote subcommand %q` + usage (2).
- Removed workbench stubs print `reasonix remote %s: Remote Workbench 已移除，请使用 \`reasonix remote connect <host> --open\`` and exit 1 (remote.go:87-100).
- Remote hosts are user-global (never written to project reasonix.toml) — remote.go:102-120.
- Full usage text: remote.go:427-445.

### 7.11 `reasonix login` / `whoami` / `logout`

- `internal/frontend/cli/account.go:36-45`. Extra args → `login: unexpected argument %q` (2) etc.
- `login`: device-code flow; already signed in → prints account and exits 0; Ctrl-C → exit 130; token stored under Reasonix home. Service URL from `REASONIX_ACCOUNTS_URL`; proxy from model proxy config (account.go:20-32).

### 7.12 `reasonix subagent`

- `internal/frontend/cli/subagent.go:37-62`. Verbs (case-insensitive): `list`/`ls`, `create`/`new`, `edit`/`update`, `delete`/`remove`/`rm`, `try`, `run`, `help`/`--help`/`-h`. Bare → usage (2); unknown → `unknown subagent command %q` + usage (2).
- `list` flags: `--dir`. `create`: `--description`, `--prompt`|`--prompt-file`, `--scope project|global`, `--model`, `--effort`, `--tools`, `--color`, `--dir`. `delete`: `--yes`, `--dir`. `try`/`run`: `--model`, `--max-steps`, `--dir`, task words.
- Usage text: subagent.go:26-35 (`--prompt-file -` or stdin for prompt).

### 7.13 `reasonix trust`

- `internal/frontend/cli/trust.go:23-165`. Hand-rolled parser: `--yes`/`-y`, `--revoke`, `--allow-providers`, `--dir PATH` (or `--dir=`).
- Usage error: `usage: reasonix trust [--dir PATH] [--yes [--allow-providers]|--revoke] (unknown argument %q)` (2); `--yes` + `--revoke` → `(...) (choose one)` (2).
- `--revoke` → `Revoked the folder trust and every program approval for %s.` (0; errors 1).
- Non-trustable root (filesystem root, home dir, Reasonix own files) reported, not trusted (trust.go:59-61).
- Non-interactive without `--yes` → `Nothing approved. Review the above, then run \`reasonix trust --yes\` here to approve.` (1).
- Provider-endpoint programs (ProjectProgramProvider/Endpoint) held back without `--allow-providers`: `Not approved: [%s] %s receives your API key at its own address; approve it by name with --allow-providers.`
- Success: `Approved. Any change to a program it names needs approval again.` (0).

### 7.14 `reasonix setup`

- `internal/frontend/cli/cli.go:632-657`: `--local`/`-l` → `./reasonix.toml` (else `.reasonix/reasonix.toml`); a positional path names the config file. Existing config + non-interactive → `NotOverwriting` (1). TTY → interactive wizard (language prompt English/中文, staged provider manager; keys stored in shared `.env`, only `api_key_env` in config). Piped → default config written (refuses existing, 1).

### 7.15 `reasonix doctor`

- `internal/frontend/cli/doctor.go:41-84`: subcommands `catalogs`, `quality`, `session`, `redact-sessions`, `capabilities`, `runtime`, `billing`, `repair`; default collects diagnostics with `--json` optional.
- `doctor repair` (doctor.go:86-125): `--root`, `--apply`, `--project`, `--json`; usage error exit 2; inspection errors 1.
- `doctor catalogs` (catalogs.go:104-137): `--json`; inspection always exits 0 (reindex reports the same failures with 0).
- `doctor billing` (doctor.go:15-39): `--json`, `--root`.

### 7.16 `reasonix report`

- `internal/frontend/cli/report.go:20-99`: actions `list`, `show [ID]`, `send [ID]`, `delete [ID]`, bare (preview + interactive send prompt y/N). >2 args → usage (2). Load failures → 1 (or 0-shaped "no reports" handling via reportLoadResult).
- Usage body: `i18n.M.ReportUsageBody` (messages_en.go:733-738).

### 7.17 `reasonix session` (machine interface)

- `internal/frontend/cli/session_machine.go:96-154`: operations `list`, `show`, `status`, `recovery`; anything else → machine error `unknown_command`. `--json` required (missing → `invalid_argument`: `--json is required`).
- Options (`parseSessionMachineOptions` session_machine.go:198-236): `--json`, `--dir` (session dir), `--project-root` (mutually exclusive with `--dir` → `--dir and --project-root cannot be combined`), positional target (session id; slashes rejected; required for show/status), `-h` → `use the documented machine interface`.
- Errors are machine JSON: {schema_version, command, error:{code,message}}; codes: `invalid_argument`, `unknown_command`, `machine_identity_unavailable`, `session_dir_unavailable`, `session_not_found`, `recovery_state_unavailable`.
- `sessions` alias routes 1.x maintenance verbs first (`sessions_maintenance.go:188-193`): `reindex`, `diagnose`, `cleanup` (`--dir` repeatable, `--json`, `cleanup --apply`); otherwise falls through to `session`.

### 7.18 `reasonix hook` (machine interface)

- `internal/frontend/cli/hook_machine.go:45-112`: operations `list`, `status` only; `--json` required. Options (`parseHookMachineOptions` hook_machine.go:138-163): `--json`, `--project-root`/`--dir` (alias), `--home-dir`; unknown option → `invalid_argument`: `unknown hook option`.
- Output: hooks sorted by event, scope, match; status includes trusted_project, project_defines, sources.

### 7.19 `reasonix catalogs reindex`

- `internal/frontend/cli/catalogs.go:232-244`: requires `reindex <catalog>`; unknown catalog → `unknown catalog: %s` (2); delegates to registered catalog commands (history, usage, tasks — registered via `registerCatalogCommand`).

### 7.20 `reasonix task`

- `internal/frontend/cli/task.go:62-103`: subcommands `list`, `show`, `monitor`, `machine-list`, `machine-show`, `status`, `events`, `stop`, `cancel`, `requeue`, `open-session`, `tmux`; unknown → `unknown task subcommand: %s` (2). Bare → usage (2).
- `task monitor <list|status|events|stop|cancel|requeue|open-session>` (task.go:105-129); `task tmux <attach|status|open|detach>` (task.go:131-150).
- Machine list/show (`task_machine.go`): `--json`, `--dir`, `--project-root`, `--session`; redacted snapshots strip ErrorSummary (task.go:34-60).

### 7.21 `reasonix version` / `--version` / `-v`

- `internal/frontend/cli/buildinfo.go:32-74`: `--version`/`-v` → single line `reasonix <version>` (0), no flags allowed.
- `version` flags: `--verbose`/`-verbose`, `--json`, `--help`/`-h`; unknown → `unknown version flag %q` + versionUsage (stderr, 2); `--json`+`--verbose` → `error: --json and --verbose are mutually exclusive` (2); `-h` → versionUsage (stdout, 0).
- Verbose adds non-empty `git_commit`, `build_time_utc`, `build_target`, `go: <runtime.Version() GOOS/GOARCH>`.
- JSON payload: {version, git_commit omitempty, build_time_utc omitempty, build_target omitempty, go}; empty/unknown omitted.
- Defaults (buildinfo.go:136-159): version `dev`; build target `GOOS/GOARCH`; git commit = embedded `vcs.revision` truncated to 12 chars else `unknown`; build time `unknown`.
- versionUsage text: buildinfo.go:76-86.

### 7.22 `reasonix completion`

- `internal/frontend/cli/shell_completion.go:402-441`: `bash`, `zsh`, `fish` print scripts (0); unknown shell → `unknown shell %q (want bash, zsh, or fish)` (2); bare → usage (2); `help`/`-h`/`--help` → usage (0). Hidden `__complete <cword> <words...>` drives shell completion candidates (shell_completion.go:443-458).

### 7.23 `reasonix docs-manifest` (undocumented release command)

- `internal/frontend/cli/docs_manifest.go:17-69`: flags `--verify-source`, `--expect-version`, `--expect-revision`; positional args rejected (2); prints the embedded docs corpus manifest as one JSON line; mismatch errors exit 1.

### 7.24 `reasonix sandbox` (hidden, Windows)

- `internal/frontend/cli/sandbox_cmd.go:14-59`: `list` (granted folders) or `revoke`; anything else → usage (2); errors 1.

### 7.25 `reasonix schedule` (hidden)

- `internal/frontend/cli/schedule_cmd.go:20-39`: `exec` → scheduler child entry; otherwise `scheduleManage`.
- `internal/frontend/cli/schedule_manage.go:27-41`: exits OK=0, Failed=1, Usage=2, NotHuman=77. Verbs: `add` (`--every`|`--at` exactly one, `--prompt`, `--workspace`, `--label`, `--model`), `list`, `tick`, `pause-all`, `resume-all`, `pause`/`resume`/`renew`/`rm <id>`. Creating/resuming/deleting require a human at a terminal (type-back confirmation of the plan id); agent children refused with 77.

## 8. Usage texts (verbatim, en)

### 8.1 Top-level usage body

`internal/base/i18n/messages_en.go:749-796` (`i18n.M.UsageBody`); Chinese at `messages_zh.go:733`. Printed by `help`/`--help`/`-h` and by `usage()` on errors:

```text
reasonix — a config- and plugin-driven coding agent (multi-model)

Usage:
  reasonix [--model NAME] [-c|--continue] [-r|--resume [QUERY]] [--permission-mode MODE] [--effort LEVEL] [--add-dir PATH]   interactive session (the terminal UI, same as reasonix tui)
  reasonix -p|--print [--model NAME] [--output-format text|json|stream-json] [--allowed-tools RULES] [--add-dir PATH] <task>
  reasonix run [--model NAME] [--max-steps N] [-c|--continue] [--resume PATH] [--copy] [--output-format FORMAT] <task>
  reasonix run --events-jsonl [--model NAME] <task>      emit redacted structured events as JSONL
  reasonix review [--base BRANCH] [--commit SHA] [--model NAME]  AI-powered code review on local diffs
  reasonix web [--model NAME] [--addr HOST:PORT] [--no-open]  start the local Web UI and open it in the default browser
  reasonix serve [--model NAME] [--addr HOST:PORT] [--auth none|token|password] [--token STR] [--password STR] [--hash-password]  serve over HTTP+SSE (with optional auth)
  reasonix acp [--model NAME]                           serve Agent Client Protocol over stdio (also: reasonix --acp)
  reasonix setup [path]                                 interactive config wizard; writes reasonix.toml (+ .env)
  reasonix config reasoning-language [auto|zh|en]        configure visible reasoning language
  reasonix config compact-ratio [--local] [PERCENT]       configure automatic compaction threshold
  reasonix config telemetry [auto|on|off]                configure content-free CLI usage metrics
  reasonix report [list|show|send|delete] [ID]           review and explicitly send local CLI crash reports
  reasonix mcp <add|remove|list|import>                 manage MCP servers in reasonix.toml
  reasonix login | whoami | logout                      sign in to reasonix.io (forum and crash follow-ups only)
  reasonix subagent <list|create|edit|delete|try|run>   manage and run isolated subagent profiles
  reasonix init                                         show how to generate project memory (AGENTS.md)
  reasonix doctor [--json]                              print redacted local diagnostics
  reasonix doctor session <branch-id> [--zip] [--out PATH]  export a session conflict diagnostic zip
  reasonix session list --json [--dir PATH]             list redacted sessions for machine clients
  reasonix session show|status <machine-session-id> --json [--dir PATH]  query one redacted session
  reasonix session recovery [<machine-session-id>] --json [--dir PATH]  query redacted recovery state
  reasonix hook list|status --json [--dir PATH]         inspect redacted hook state
  reasonix trust [--dir PATH] [--yes|--revoke]          review and approve programs this project runs
  reasonix task list|show|status|events|stop|cancel|monitor|tmux --json [--dir PATH]
                                                         inspect or control redacted tasks
  reasonix upgrade [--check] [--force]                   update to the latest official release (also: reasonix update)
  reasonix completion bash|zsh|fish                     print a shell completion script to stdout
  reasonix version [--verbose|--json]                   print version (single line) or build metadata
  reasonix --version | -v                               single-line version (script-safe)
  reasonix help

Examples:
  reasonix web
  reasonix run "implement the TODOs in main.go"
  reasonix run --model mimo-pro "add unit tests for this function"
  reasonix -p "summarize this repository" --output-format json
  reasonix subagent run review "review the current changes"
  echo "explain this code" | reasonix run

Configuration:
  Resolution: flag > ./reasonix.toml > <Reasonix home>/config.toml > built-in defaults
  Secrets come from the environment via api_key_env (e.g. DEEPSEEK_API_KEY).
  Run 'reasonix setup' to scaffold a config; see docs/SPEC.md.
```

Other usage texts: versionUsage (buildinfo.go:76-86), configUsage (cli.go:1671-1678), mcpUsage (mcp.go:510-549), remoteUsage (remote.go:427-445), subagentUsageText (subagent.go:26-35), pluginUsage (plugin.go:54-64), completionUsage (shell_completion.go:433-441), scheduleUsage (schedule_manage.go:35-41), task usage line (task.go:64), sandbox usage line (sandbox_cmd.go:57), report usage (`i18n.M.ReportUsageBody` messages_en.go:733-738).

## 9. Divergences and open questions (docs vs source)

1. **docs/CLI.md does not document `--agent-composition`** as a bare/TUI/run flag, but `isDefaultInteractiveFlag` (cli.go:171-180) and both flag sets accept it. Source wins; docs gap.
2. **docs/CLI.md documents exit codes 0/1/2/4 for run**; source also uses **77** (agent-child / schedule not-human) and **130** (login cancelled). Not a conflict — extend the Rust matrix with 77/130.
3. **Top-level usage text omits** `remote`, `plugin`, `docs-manifest`, `schedule`, `sandbox`, `config auto-plan`, `config currency`. Intentional; the Rust usage text must reproduce the same omissions.
4. **docs/CLI.md says TUI bare `--inline` is an unknown command** — confirmed: `--inline` is not in `isDefaultInteractiveFlag`, so bare `reasonix --inline` → unknownCommand (exit 2). Only `reasonix tui --inline` works.
5. `version --verbose`/`--json` expose a `go` field (runtime version). For the Rust rewrite this field needs a BREAKS.md decision (keep the field with rustc version, or drop it); ledger CLI-005 currently blocks on this decision.
6. `run --takeover` is accepted (1.x compat) but the session lease is still refused — flag is a no-op acceptance, not behavior.
7. docs/CLI.md compact-ratio: bounds 0<x<100 exclusive, default 85%, TOML stores a fraction; source formatting trims trailing zeros (cli.go:1666-1669). Verify exact bound error text against `compactRatioPercentageRequirement` during the config slice.

## 10. Ledger rows proposed (CLI-010..CLI-041)

Each row: Area=CLI, Status=todo, Evidence=pending; Rust=reasonix-cli unless noted.

| ID | Behavior |
|---|---|
| CLI-010 | Bare invocation: TUI when stdin+stdout are terminals; bareUsage otherwise (exit 0 bare / 2 session flags); mintty hint |
| CLI-011 | `--acp` shortcut routes to the acp command |
| CLI-012 | Leading session flags move into run when both parsers agree (§2.1) |
| CLI-013 | Leading `-p`/`--print` → `run --print` |
| CLI-014 | First-token interactive flag → bare TUI (isDefaultInteractiveFlag set) |
| CLI-015 | Unknown command: stderr message + usage + exit 2 |
| CLI-016 | Hidden `schedule` command verbs + human-confirmation exits (77) |
| CLI-017 | Hidden `sandbox list/revoke` (Windows) |
| CLI-018 | `tui`/`chat`/`code` aliases; full TUI flag set incl. `--inline`, optional `--resume` |
| CLI-019 | `run` full flag set + exit-code map (§7.1) |
| CLI-020 | Permission-mode parsing matrix + error text (§4.1) |
| CLI-021 | Preset/profile parsing matrix + error text (§4.2) |
| CLI-022 | Approval flag conflicts (§4.3) |
| CLI-023 | Session query resolution: exact/partial/machine-id/ambiguous (§4.4) |
| CLI-024 | `--copy` semantics + copy notice stream |
| CLI-025 | `serve`/`web` flag set + auth defaults + broker flags (§7.6) |
| CLI-026 | `acp` flag set + PostureCeiling (§7.5) |
| CLI-027 | `mcp` verb matrix + usage text + browse limits (§7.8) |
| CLI-028 | `plugin` verb matrix (§7.7) |
| CLI-029 | `remote` verb matrix + agent-child refusal 77 (§7.10) |
| CLI-030 | `login`/`whoami`/`logout` (device flow, exit 130 on cancel) (§7.11) |
| CLI-031 | `subagent` verb matrix + usage (§7.12) |
| CLI-032 | `task` verb matrix + machine list/show redaction (§7.20) |
| CLI-033 | `session` machine interface + error codes (§7.17) |
| CLI-034 | `hook` machine interface (§7.18) |
| CLI-035 | `sessions` 1.x maintenance verbs (reindex/diagnose/cleanup) |
| CLI-036 | `catalogs reindex <catalog>` dispatch |
| CLI-037 | `trust` full behavior (§7.13) |
| CLI-038 | `doctor` verb matrix (§7.15) |
| CLI-039 | `config` verb matrix (§7.9) |
| CLI-040 | `report` verb matrix (§7.16) |
| CLI-041 | `completion` bash/zsh/fish + `__complete` candidates (§7.22) |
| CLI-042 | `docs-manifest` (§7.23) |
| CLI-043 | `upgrade`/`update` syntax + channel compat (§7.3) |
| CLI-044 | `review` flags + empty-diff exit 0 (§7.4) |
| CLI-045 | `setup` targets + overwrite refusal (§7.14) |
| CLI-046 | `init` hint, exit 0 |
| CLI-047 | Top-level usage text byte parity (en) (§8.1) |
| CLI-048 | Exit-code matrix incl. 4/77/130 (§5) |
| CLI-049 | `REASONIX_DNS`/env handling at process start (§6) |
| CLI-050 | Legacy config migration trigger set (§2.2) |
