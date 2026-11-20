
# Host Handshake Contract — Frozen Inventory (M1)

Status: Frozen inventory extracted from source on 2026-11-15. Rust target: `apps/reasonix-studio-host` (the host binary); the gate, page discovery, and listener primitives it wires are frozen in docs/contracts/http.md §2.6 and §3.

Reference sources: `cmd/reasonix-studio-host/**` (Go host: main.go 626L, owner.go, startup.go, launchroot.go, schedule_ticker.go, diskwatch.go, packagegrants.go and their tests), `internal/platform/instanceid`, and `desktop/electron/src/**` (the Electron shell, unchanged). The loopback gate, page discovery, and the graceful listener are already frozen in docs/contracts/http.md; this document recaps them only where the host binary wires them and does not re-freeze their internals.

## 0. Overview

Studio runs as two processes:

- **The kernel host** — `cmd/reasonix-studio-host` in the reference, `apps/reasonix-studio-host` in Rust. One binary. It binds a loopback-only listener, mints a fresh credential, writes exactly one JSON handshake line to stdout, serves the Studio hub behind the loopback gate, reads a parent lease on stdin, and writes act lines back on the same stdout pipe.
- **The Electron shell** — `desktop/electron`. Preserved unchanged (AGENTS.md §1.2). It spawns the host, parses the handshake, arms the credential as a cookie before any page load, loads the page from the origin, and reads act lines from the host's stdout.

The handshake is the whole trust boundary. The host serves only on `127.0.0.1`; the only way in is the token printed on stdout. No configuration value contributes to the credential (§4.2), and a user-configured serve token does not open this gate.

Everything in §1–§6 is implemented by the Rust host binary. §7 records what the unchanged Electron shell does with the host's output — acceptance constraints the Rust host must satisfy, not code to rewrite.

## 1. Process lifecycle

### 1.1 argv precedence — before flag parsing

`main()` offers three re-entry modes before `flag.Parse` can see the arguments. Each may take over the process and exit before any host flag exists:

1. `update.MaybeRunMacHandoff(os.Args[1:])` — on macOS an update swaps the bundle and re-execs this binary as a child carrying the update's argv; parsing that argv as host flags would die on an undefined flag, so the handoff is offered first.
2. `update.MaybeRunTreeHandoff` — the Windows counterpart: a copy of this binary started with the staged plan waits for the application to exit and moves the staged tree in.
3. `scheduleChildArgs(args)` — true iff `args[0] == "schedule" && args[1] == "exec"`; then `os.Exit(schedulerun.Exec(os.Args[3:]))`. The scheduled child runs through the small schedulerun package, never the CLI (§6.8).

Test: `TestAScheduleChildIsRecognisedByItsArguments` — `"schedule exec sch_x/1"` and `"schedule exec"` are children; `"schedule list"`, `"exec schedule"`, `""` are not.

Then `provider.SetClientVersion(version)` runs; `version` defaults to `"dev"` and is stamped at release (ldflags in Go; the Rust release pipeline stamps the same value).

### 1.2 Flags

| Flag | Kind | Meaning |
|---|---|---|
| `-page` | string | Directory of the built Studio page (`frontend-next/dist`). |
| `-instance-id` | bool | Print the Studio instance id to stdout and exit 0 (§5.1). |
| `-studio-version` | string | The shell's version (install/update identity). |
| `-studio-app` | string | The shell's executable path (install/update identity). |
| `-studio-app-pid` | int | The shell's pid. |
| `-cua-driver` | string | Path to the CUA driver binary. |
| `-computer-abort-socket` | string | Abort socket for the embedded computer driver. |
| `-computer-socket` | string | Socket for the embedded computer driver. |
| `-crash-dir` | string | Crash log directory (§1.6). |
| `-strip-package-grants` | bool | Run the Windows package-grant strip and exit (§5.2). |

`modhost.SetEmbedder(*studioApp, os.Environ())` — Studio's own binary hosts Claude Code mod modules as Node; the embedder identity is the stated application, not this host process.

### 1.3 Exit codes

| Code | Meaning |
|---|---|
| 0 | Clean shutdown: the parent released the lease, or a signal ended the context. |
| 1 | Every startup failure: page discovery error, assemble error, bind error, announce error, serve error. |
| 2 | `-strip-package-grants` with no usable `-studio-app` (§5.2). |

### 1.4 Startup phases

Every phase transition logs one line to stderr:

``
reasonix-studio-host: startup phase=<name> elapsed_ms=<n>
``

Boot phases log as `boot.<name>` (e.g. `boot.sessions`). Phase order: `page`, `config-upgrade`, `config`, `sinks`, `workspace`, `runtime`, `hub`, `adopt`, `background`, `bind`, `provider-setup`, `announce`. Test: `TestStartupPhasesUseElapsedMilliseconds` asserts the exact three-line output format.

### 1.5 Parent lease

`parentLease(f *os.File)` classifies stdin: only a named pipe or a socket counts (Node/libuv hands a child's stdio a socketpair). A terminal, a regular file, or `/dev/null` is not a lease; the host then runs unsupervised (no watchdog). Tests: `TestOnlyAPipeIsAParentLease`, `TestASocketpairIsAParentLease` (unix).

When a lease exists, a goroutine copies it to `io.Discard`; when the copy returns (EOF — the parent closed its end), the host stops serving. The lease is a pipe or socket held open by the parent: closing it is how the shell says "time to go", identically on every platform. Test: `TestRunServesUntilTheParentLetsGo` — lease close ends in a clean exit 0 with the port released.

### 1.6 Crash and fatal logs

`crashreport.InstallFatalLog(*crashDir, defaultVersion(shell.version))` installs the crash reporter with the crash directory (`logsDirOf(crashDir) = filepath.Dir(crashDir)`). stderr is wrapped by `lastLine`, which remembers the last line starting with `reasonix-studio-host:`. On exit code 0 the fatal log is released (`releaseFatalLog()`); on any other code the exit is recorded with the code and the remembered line (`exitedFatalLog(code, last)`).

### 1.7 Startup flow (run)

`run(lease, handshakeTo, logs, page, shell, driver)`:

1. `signal.NotifyContext(Interrupt, SIGTERM)` — either signal ends the context.
2. If a lease exists, drain it in a goroutine (§1.5).
3. `serve.FindPage(page)` — an error exits 1; a nil result logs `reasonix-studio-host: no built page found; serving the kernel only` and serves without a page (§6.7).
4. `assemble()` — an error exits 1 (§6).
5. `defer hub.Shutdown()` — after the socket has drained, never before.
6. `startScheduleTicker(ctx, lease != nil, hub, logs)` — only a supervised host schedules (§6.5).
7. `bind(hub.Handler())` — loopback listener, fresh credential, gate (§4). An error exits 1.
8. `hub.EnableProviderSetupForListener(bound.listener.Addr().String())` — the credential-writing provider setup opens on the loopback listener only.
9. `announce()` — the handshake line (§2). An error exits 1.
10. `bound.serve(ctx)` = `serve.RunGracefulHandler(ctx, listener, handler)` — an error exits 1; otherwise exit 0.


## 2. Handshake protocol

### 2.1 Announcement

After bind and before serving, the host writes exactly one JSON line to stdout:

``json
{"version":1,"origin":"http://127.0.0.1:<port>","token":"<64 hex chars>"}
``

- `version` = `handshakeVersion` = 1.
- `origin` = `serve.LoopbackOrigin(listener)` = `"http://"` + the listener address — always `http://127.0.0.1:<port>`.
- `token` = `launchCredential()`: 32 bytes from `crypto/rand`, hex-encoded (64 hex chars). Never persisted, never derived from configuration, never reused across launches.
- Field order is `version`, `origin`, `token` (Go's `encoding/json` emits struct fields in declaration order). The Electron parser is field-order-insensitive, but the Rust host must emit the same compact single-line form with a trailing newline: the shell reads the first line as the handshake and feeds the remaining bytes to the act reader.

Tests: `TestHostMintsAFreshCredentialPerLaunch` (token is 64 hex chars and differs per launch), `TestHostBindsOnlyToLoopback` (127.0.0.1, origin `http://host:port`), `TestRunServesUntilTheParentLetsGo` (the handshake is the first stdout line).

### 2.2 What the shell accepts (host.js parse rules)

The Electron shell parses the handshake line as JSON and applies, in order:

1. `body.version !== 1` → `unsupported handshake version <X>`.
2. `token` must be a string of at least 32 characters → otherwise `the handshake carried no usable credential`.
3. `origin` must parse as a URL → otherwise `the handshake carried no usable origin`.
4. The protocol must be `http:` → otherwise `the handshake named <X>:, which this shell does not load`.
5. The hostname must be `127.0.0.1` → otherwise `the handshake named <X>, which is not this machine`.
6. The origin must carry a port → otherwise `the handshake named no port`.

Malformed JSON → `malformed handshake, not JSON: <...>`. On success the shell keeps `{origin: url.origin, token}`.

### 2.3 Limits and timeouts (shell side)

| Constant | Value | Meaning |
|---|---|---|
| `HANDSHAKE_LIMIT` | 4096 bytes | The first line must end within this many bytes; past it the launch fails with code `overflow` (`the handshake ran past its limit`). |
| `HANDSHAKE_TIMEOUT_MS` | 60000 | Default handshake timeout, overridable through env `REASONIX_STUDIO_HANDSHAKE_TIMEOUT_MS`. On expiry the launch fails with code `timeout` (`the kernel was still running but sent no handshake within <N> s`). |
| `SLOW_AFTER_MS` | 3000 | After 3 s the shell shows the starting window (`onSlow`); a slow kernel is waited for, never restarted. |

If the child exits before the handshake, the failure code is `exited` (`the kernel exited with <code or signal> before saying anything`); a spawn failure is `spawn` (`the kernel could not be started: ...`). The error codes are exactly: `timeout`, `exited`, `spawn`, `overflow`, `malformed`.

### 2.4 Retry contract

`shouldRetry(err, attempt, elapsed)` in the shell: retry only while `attempt < 2` and the code is `exited` or `spawn` and `elapsed <= 10000` ms — an installer or a scanner holding a fresh binary lets go in about that time. One retry after a 1000 ms delay; a slow or silent kernel is never restarted.

## 3. Acts (kernel → shell)

After the handshake, the same stdout pipe carries act lines — one JSON object per line, each newline-terminated:

``json
{"act":"relaunch"}
{"act":"quit"}
{"act":"show"}
``

- Exactly three acts exist: `relaunch`, `quit`, `show`.
- A mutex keeps each act on its own line: two acts interleaved into one write would reach the shell as neither. Test: `TestEachActTravelsAsItsOwnLine`.
- `PrepareForUpdate` → nil, writes nothing (releasing the application is the hub's own work).
- `RelaunchAfterUpdate` asks `relaunch` **only on Linux**: the Windows installer and the macOS helper start the successor themselves, and arming there would double-start. Everywhere else it writes nothing.
- `EndApplication` asks `quit`.
- A toast click asks `show`: `showOnToastClick` logs `notify: toast clicked`, then asks `show`; a failure logs `notify: toast click could not reach the shell: <err>`. Tests: `TestAToastClickAsksTheShellToShowItsWindow` (exact line `{"act":"show"}` plus newline), `TestAToastClickIsWrittenToTheLog`.
- An ask that cannot be written is reported as an error naming the act: `ask the shell to <act>: <err>`. Test: `TestAnActThatCouldNotBeSentIsReported`.
- Nothing is acknowledged: each act is performed by the shell ending, relaunching, or showing its window.
- Shell-side reader (`readActs`): every line after the handshake is `JSON.parse`d; only a body with a string `act` field is dispatched; unparseable lines are silently skipped; a buffer over 4096 bytes without a newline is dropped.


## 4. Loopback boundary

### 4.1 Bind and gate

`bind(hub.Handler())`:

1. `serve.ListenLoopback()` — TCP, `127.0.0.1:0`, IPv4 loopback only.
2. `launchCredential()` — the token (§2.1).
3. `origin = serve.LoopbackOrigin(ln)`.
4. `handler = serve.NewLoopbackGate(next, LoopbackGateConfig{Token, Origin})`.

The gate's request checks (host authority → origin for state changes → constant-time cookie comparison, with the `loopback.host_rejected` / `loopback.origin_rejected` / `loopback.unauthorized` / `loopback.misconfigured` codes) are frozen in docs/contracts/http.md §2.6 and are not repeated here. Host-side tests that pin the wiring: `TestHostRequiresItsCredentialUnderAuthModeNone` (a serve auth `none` config does not open this gate; no cookie → `403 loopback.unauthorized`), `TestHostDoesNotAdoptTheConfiguredServeToken` (a configured serve token is rejected; the launch credential works), `TestHostPassesRealTrafficToTheKernel` (`GET /status` without an Origin header → 200), `TestHostStreamsEventsThroughTheGate` (the SSE opening comment flushes through the gate), `TestTrayEndpointsSitBehindTheSameBoundary` (tray endpoints sit behind the same gate; a PUT without an Origin header → `403 loopback.origin_rejected`), `TestHostReleasesTheSocketOnShutdown`.

### 4.2 No configuration contributes

`hostServeConfig(cfg.Serve)` zeroes `AuthMode`, `Token`, `PasswordHash`, and `BehindProxy` from the user's serve config: the Studio boundary is the loopback gate, and a configured token would make the hub refuse the minted credential. Nothing in the user's configuration can open, weaken, or predict the Studio boundary.

### 4.3 Provider setup on loopback only

`hub.EnableProviderSetupForListener(bound.listener.Addr().String())` — the credential-writing provider setup flow opens only on the loopback listener.

### 4.4 Graceful drain

`bound.serve(ctx)` is `serve.RunGracefulHandler` (docs/contracts/http.md §3: `ReadHeaderTimeout` 10 s, `IdleTimeout` 120 s, `Shutdown` with a 10 s drain budget, `ErrServerClosed` → nil). `hub.Shutdown()` runs after the socket has drained, never before.

## 5. Helper modes

### 5.1 `-instance-id`

Prints `instanceid.Current()` to stdout and exits 0. One data home is one Studio instance:

- `Prefix` = `io.reasonix.studio`.
- `For(home)`: trim the home; empty → the bare prefix. Canonicalize through the lease-path canonicalizer at `<home>/.reasonix-home.identity` (a home below a link, or one that does not exist yet, still names the physical directory), then `sha256(filepath.Clean(root))[:8]` hex → `io.reasonix.studio.<16 hex>`.
- A different `REASONIX_HOME` is a different instance — the way to run two Studios side by side. Which shell holds the window is not part of the identity; the home is.

The shell runs this mode synchronously (10 s timeout) to scope its Electron profile and single-instance lock (§7.4); a failure there yields an empty identity, so a launch that is going to fail anyway is not misreported as "already running".

### 5.2 `-strip-package-grants`

`stripPackageGrants(stdout, stderr, *studioApp)` — Windows only (the shell calls it on win32 packaged launches, before Chromium starts its first child). Chromium's sandboxed children exit while loading a DLL whose access entries grant one Windows app package, so the window never paints; the kernel removes those grants from the tree the stated application runs from.

Exit semantics:

- **2** — `-studio-app` is empty (`strip-package-grants: -studio-app names no application`), `EvalSymlinks` fails, or the path is not a regular file. Nothing on stdout.
- **1** — `packagegrant.Strip` error, or the JSON report cannot be encoded.
- **0** — prints one line of JSON to stdout: `{"stripped":[...],"refused":[...],"unread":[...]}`. All three lists are present even when empty — the shell reads the line as structure, and a null would be a third answer it has to guess about.

Tests: `TestStripPackageGrantsOnlyAnswersForAnApplication` (exit 2, empty stdout for: no application, a path that is nothing, a directory), `TestStripPackageGrantsReportsOneLineOfJSON` (`"stripped":[]`, `"refused":[]`, `"unread":[]` all present).

### 5.3 `schedule exec` child

`args[0] == "schedule" && args[1] == "exec"` → `os.Exit(schedulerun.Exec(os.Args[3:]))` (§1.1). The scheduled child runs through the small schedulerun package; the host must not depend on the command-line package for it (§6.8).

## 6. Host invariants

### 6.1 Launch workspace

`launchWorkspace(appExe)` → `pickLaunchWorkspace(cwd, appExe, remembered, auto)`:

1. `userChoseDir(cwd, appExe)` → the cwd wins. False when the cwd is empty, when the cwd is the filesystem root, or when the cwd is the install directory or below it (`Rel(Dir(appExe), cwd)` staying inside the install tree). An unpackaged launch (no `appExe`) keeps its cwd.
2. Otherwise the first remembered workspace that `os.Stat`s as a directory.
3. Otherwise the auto workspace (`config.AutoWorkspaceDir()`), created with `MkdirAll 0o755` — created last, only when nothing else won.
4. Otherwise the cwd.

The remembered list comes from `serve-workspaces.json` (`{"paths":[...],"launch":...}`): the `launch` project wins, then the remembered order. A first-ever launch opens the auto workspace — never the home directory. Tests: `TestUserChoseDirRejectsWhatALauncherInherits`, `TestPickLaunchWorkspaceFallbackOrder`, `TestLaunchWorkspaceKeepsLaunchProjectAfterSidebarReorder`, `TestLaunchWithNothingChosenOrRememberedOpensTheAutoWorkspaceNotHome`, `TestPickLaunchWorkspaceCreatesTheAutoWorkspaceLast`.

### 6.2 Kernel language

`resolveKernelLanguage(cfg)` = `i18n.DetectLanguage(cfg.Language)` — the **top-level** `language` setting, not the desktop one. The shell sets `REASONIX_SYSTEM_LANG` to its preferred system language before spawn. Resolution matrix (tests `TestKernelLanguageWithShellSystemLanguage`, `TestAssembledTrayLanguage`, `TestTheHostPointsTheKernelsCatalogueAtTheConfiguredLanguage`):

| system | desktop setting | REASONIX_LANG | locale | resolved |
|---|---|---|---|---|
| zh-Hans-CN | auto | — | — | zh |
| zh-Hant-TW | auto | — | — | zh-TW |
| zh-Hans-CN | en | — | — | en |
| zh-Hans-CN | auto | en-US | — | en |
| en-US | zh | en | — | zh (desktop overrides env) |
| zh-Hans-CN | auto | — | C / POSIX.UTF-8 | en |

`POST /appearance` with `{"language":"..."}` live-switches the language: `/tray/state` returns `Labels{Open, CloseToTray, Quit}` + `Line` from the current catalog.

### 6.3 Telemetry consent

Telemetry runs when `cfg.DesktopTelemetry()` or `cfg.DesktopMetrics()`. Studio's telemetry options: `Mode "on"`, `Surface surface.Studio`, `SuppressPing = !DesktopTelemetry()`, and `PingAllowed` re-reads the **user** config strictly on every ping: a project config cannot turn the ping on, a corrupt file is not consent, and a missing file means the default. `AllowMarketInstallReport` is granted only when `telemetry.ReleaseVersion(version)` — a dev build grants nothing.

### 6.4 Tray

`studioTray`: `IconLive()` re-reads the user config (`DesktopTray() != "off"`); `TrayFold()` is the tracker state; `ApplyTrayPrefs` is a no-op (the tray endpoint owns prefs).

### 6.5 Schedule ticker, notifications, disk watch

- `scheduleEligible(supervised, iconLive, panes) = supervised && (iconLive || panes > 0)` — an orphan or an invisible kernel never starts a scheduled run. The ticker takes a machine-wide file lock (`scheduler.lock`, `TryAcquire`) so one machine ticks once; per-run logs `schedule: ran <id>: <state> <code>`, `schedule: skipped <id>: <reason>`. Tests: `TestSchedulesStartOnlyForASupervisedKernelThatCanBeSeen`, `TestAScheduleChildIsRecognisedByItsArguments`.
- Notifications wrap every runtime, reading the switch live (phone link shared). Tests: `TestHostNotificationsCarryTheSharedSettingLive`, `TestAnApprovalOnAPaneReachesThePhoneLinkAsATypeAndACount`.
- Disk watch: every 5 minutes, re-read the notify settings; `Enabled && DiskLow` gates it; the monitor dedups, so a low volume is said **once** however many times it is looked at, and two folders on one volume do not make two notices; the message language follows the current desktop language. Watched dirs: the workspace root and its session dir. Tests: `TestTheDiskWatchSaysALowVolumeOnce`, `TestTheDiskWatchHonoursTheSwitches`.

### 6.6 Install/update capability guards

- `shellIdentity{version, exe, pid, logsDir}`; `stated()` = version and exe non-empty and pid > 0.
- `studioInstall`: nil unless a version is stated; the layout follows the **stated application** (`update.At(shell.exe, update.StudioLine())`), never this process's own executable. Test: `TestTheInstallLayoutFollowsTheStatedApplication`.
- `studioUpdateHost`: nil unless `stated()` — a typed nil inside the interface would satisfy `HubOptions.Update != nil` and register install routes for a host that cannot name the application it would replace. Version-only, exe-only, pid-only, or nothing all yield nil. Tests: `TestAHostThatCannotNameTheApplicationOwnsNoCapability`, `TestAHostThatNamesTheApplicationOwnsOne`.

### 6.7 No page → kernel only

When `FindPage` returns nil the host logs `reasonix-studio-host: no built page found; serving the kernel only` and serves the hub without a page. Page discovery itself (an explicit `-page` dir must hold `index.html`, else beside the executable following symlinks, else `../frontend-next/dist`, else `desktop/frontend-next/dist`) is frozen in docs/contracts/http.md §3.

### 6.8 Structural guards

- The host must not depend on the command-line package. Test: `TestTheHostDoesNotCarryTheCommandLine` — the scheduled child goes through schedulerun, and importing the CLI for one entry point would bring back every command it carries.
- `HubOptions` must be handed every capability this host implements: the keys `Tray`, `Asks`, `Remote`, `DecorateSink`, `Grant` must be set. Test: `TestTheHostHandsItsHubEveryCapabilityItImplements` — a capability implemented but not handed to the hub is a route the kernel never registers.
- The first build must sweep sessions in the background: `CleanupPendingReconciler: serve.BackgroundCleanupReconciler`. Test: `TestTheHostsFirstBuildSweepsSessionsInTheBackground` — a build that sweeps inline makes the first launch after a cold cache wait on history it never displays.
- `OpenOnFallbackModel: true` — a stale `default_model` must not block the window.


## 7. Electron consumer expectations

These describe the unchanged shell (desktop/electron). The Rust host must satisfy them; none of this is code to rewrite.

### 7.1 Spawn and arguments

The shell spawns the host with `stdio: ['pipe','pipe','pipe']`, `windowsHide: true`, and `REASONIX_SYSTEM_LANG` set to the preferred system language. Arguments: `-page <pageDir> -crash-dir <crashDir>`; on darwin with an embedded CUA: `-computer-socket` / `-computer-abort-socket` and `-cua-driver`; packaged launches add `-studio-version <version> -studio-app <execPath> -studio-app-pid <pid>`. Binary, page, and driver paths resolve from `REASONIX_STUDIO_HOST` / `REASONIX_STUDIO_PAGE` / `REASONIX_CUA_DRIVER` env or the packaged layout (`bin/reasonix-studio-host[.exe]`, `frontend-next/dist`, `bin/cua-driver[.exe]`).

### 7.2 Credential cookie

After the handshake the shell adds the token to its redaction list and arms the credential as a cookie **before any load**: name `reasonix_token` (must match `serve.TokenCookie`), value = the handshake token, `path: /`, `httpOnly: true`, `secure: false`, `sameSite: 'strict'`, on the default session, at the origin. `keepCredential` watches for the cookie's removal and restores it; `restoreIfStale` re-reads the jar when a request from the origin was refused. The renderer never sees the token: host HTTP goes through the main process only (`StudioHost(origin, token)` with a 5000 ms timeout; `json()` returns null on refusal or unreachable, so every caller survives a dying kernel).

### 7.3 Host exit marker

If the kernel exits after the handshake without being asked (a non-zero code while not quitting), the shell records `<userData>/unexpected-host-exit.json` (`{at, code, signal, crashFile}`, mode 0o600), tails the matching crash file (`^host-.*\.log$`, 8192 bytes / last 30 non-empty lines) into the shell log, and shows a notice dialog (zh/en) on the next launch; the marker clears after it is shown.

### 7.4 Single instance

`app.setName("Reasonix Studio")`; the userData profile — and so the platform lock — is scoped by the host's `-instance-id` answer (10 s synchronous timeout, trimmed; a failure yields an empty identity so a broken launch is not misreported as "already running"). A second instance that fails `requestSingleInstanceLock` shows the first instance's window and exits without spawning a kernel.

### 7.5 Quit drains through the lease

On quit the shell ends the child's stdin — the lease — which tells the kernel to drain (§1.5). `window-all-closed` quits only once the handshake happened. A dev launch watched by a launcher (`ppid > 1`, not handed over) quits when the launcher process disappears (1 s poll); a hand-over sets `REASONIX_STUDIO_HANDED_OVER=1` so the successor is nobody's child.

### 7.6 Shell log redaction

The shell log redacts auth schemes (Bearer/Basic/Digest), URL userinfo passwords, and named fields matching token/secret/password/passwd/credentials/auth/authorization/api/access/private-key/key as a whole or last path segment; secrets of at least 8 characters are registered via `addSecret` (the handshake token is registered). Startup failure dialogs (zh/en) hint the codes `timeout`, `exited`, `spawn`, `unreachable`.

## 8. Divergences and notes

No intentional breaks in this area. Notes for the Rust implementation:

1. **Field order and framing** — emit the handshake as one compact JSON line with fields in the order version, origin, token, terminated by a newline (Go's `encoding/json` field order). The shell's parser is order-insensitive, but byte parity keeps golden diffs empty.
2. **The gate is http.md's** — the loopback gate checks, page discovery, and the graceful listener are owned by docs/contracts/http.md §2.6/§3; the HOST rows reference them instead of re-freezing.
3. **Update handoffs** — `MaybeRunMacHandoff` / `MaybeRunTreeHandoff` internals are a platform surface (M5); this contract fixes only their precedence slot: they run before flag parsing, may take over the process, and their child argv must never reach the host flag parser.
4. **Electron shell out of scope** — every §7 behavior is implemented by the existing JS; the Rust side is accepted when the unchanged shell passes its desktop smoke (AGENTS.md §8.1 Desktop rows).
5. **`schedulerun.Exec`** — the schedule child entry is part of the host binary's surface; its internals belong to the scheduler contract (frozen with the session/runtime area), not re-inventable ad hoc.

## 9. Proposed ledger rows

| ID | Area | Reference | Rust | Status | Evidence |
|---|---|---|---|---|---|
| HOST-001 | Host | main() argv precedence (mac handoff, tree handoff, schedule exec before flags) | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §1.1 |
| HOST-002 | Host | Host flag set (-page, -instance-id, -studio-*, -cua-driver, -computer-*-socket, -crash-dir, -strip-package-grants) + modhost embedder | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §1.2 |
| HOST-003 | Host | Exit codes: 0 clean, 1 every startup failure | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §1.3 |
| HOST-004 | Host | Startup phase log lines (startup phase=%s elapsed_ms=%d; boot.* phases; 12-phase order) | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §1.4 |
| HOST-005 | Host | Parent lease: stdin named pipe or socket only; terminal/file/dev-null unsupervised | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §1.5 |
| HOST-006 | Host | Parent EOF on the lease ends serving cleanly (exit 0, port released) | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §1.5 |
| HOST-007 | Host | Crash/fatal log: InstallFatalLog(crash-dir); released on 0, recorded with last line otherwise | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §1.6 |
| HOST-008 | Host | Handshake line: one JSON line (version 1, origin, token), field order version/origin/token | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §2.1 |
| HOST-009 | Host | Fresh credential per launch: 32 random bytes hex, never persisted, never from config | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §2.1 |
| HOST-010 | Host | Loopback bind 127.0.0.1:0 (tcp4); origin http://127.0.0.1:<port> | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §2.1 |
| HOST-011 | Host | Announce ordering: after bind + provider-setup enable, before serve loop | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §1.7 |
| HOST-012 | Host | host.js acceptance rules (version, token >= 32 chars, origin URL, http:, 127.0.0.1, port; 4096-byte limit; 60 s timeout; codes timeout/exited/spawn/overflow/malformed) | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §2.2-2.3 |
| HOST-013 | Host | Shell retry contract: only exited/spawn within 10 s, one retry | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §2.4 |
| HOST-014 | Host | Acts: relaunch/quit/show, one JSON line each, mutex-serialized | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §3 |
| HOST-015 | Host | Relaunch asked on Linux only; quit on EndApplication; PrepareForUpdate writes nothing | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §3 |
| HOST-016 | Host | Toast click asks show; notify: toast clicked / could-not-reach logs | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §3 |
| HOST-017 | Host | Loopback gate wiring with the launch credential (gate checks per http.md §2.6) | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §4.1 |
| HOST-018 | Host | hostServeConfig zeroes AuthMode/Token/PasswordHash/BehindProxy (no config contribution) | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §4.2 |
| HOST-019 | Host | Provider setup enabled on the loopback listener only | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §4.3 |
| HOST-020 | Host | Graceful drain: RunGracefulHandler; hub.Shutdown after socket drain | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §4.4 |
| HOST-021 | Host | -instance-id: io.reasonix.studio.<16 hex> (canonicalized home sha256[:8]) | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §5.1 |
| HOST-022 | Host | -strip-package-grants: exit 2/1/0 semantics; one-line JSON report with stripped/refused/unread always present | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §5.2 |
| HOST-023 | Host | schedule exec child: os.Args[3:] through schedulerun.Exec | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §5.3 |
| HOST-024 | Host | Schedule ticker eligibility (supervised && (iconLive or panes > 0)) + machine-wide scheduler.lock | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.5 |
| HOST-025 | Host | Launch workspace: userChoseDir rules, remembered fallback, auto created last, never home | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.1 |
| HOST-026 | Host | serve-workspaces.json: launch project wins over remembered order | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.1 |
| HOST-027 | Host | Kernel language: top-level language + REASONIX_SYSTEM_LANG matrix; /appearance live-switch | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.2 |
| HOST-028 | Host | Telemetry consent: strict user-config re-read per ping; project config cannot enable; AllowMarketInstallReport release-only | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.3 |
| HOST-029 | Host | Tray: IconLive re-read (DesktopTray != off); fold from tracker; ApplyTrayPrefs noop | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.4 |
| HOST-030 | Host | Notifications wrap every runtime live; disk watch says a low volume once, honours both switches, 5 min cadence | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.5 |
| HOST-031 | Host | Install/update typed-nil guards: nil unless version+exe+pid stated; layout follows stated application | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.6 |
| HOST-032 | Host | No built page: kernel-only serve + log line | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.7 |
| HOST-033 | Host | Host must not depend on the CLI crate (schedule child via schedulerun) | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.8 |
| HOST-034 | Host | Hub wiring keys (Tray/Asks/Remote/DecorateSink/Grant) + background cleanup reconciler + OpenOnFallbackModel | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.8 |
| HOST-035 | Host | Credential cookie contract: reasonix_token, httpOnly, sameSite strict, armed before load, restored on removal/refusal | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §7.2 |
| HOST-036 | Host | Shell single instance scoped by -instance-id answer (profile + lock) | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §7.4 |
| HOST-037 | Host | Quit drains through the lease (child stdin end); window-all-closed quits only after handshake | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §7.5 |
| HOST-038 | Host | Host exit marker + notice (unexpected-host-exit.json) for post-handshake death | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §7.3 |
