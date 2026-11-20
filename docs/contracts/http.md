# HTTP Contract — Frozen Inventory (M1)

> **STATUS: FROZEN** from source on 2026-11-15. Area: the kernel's HTTP surface — routes, methods, status codes, JSON envelopes, auth, binding, SSE transport framing.
> Reference: `reference/DeepSeek-Reasonix-studio` (Go). Citations relative to `internal/frontend/serve/` unless noted.
> Rust crate target: `reasonix-serve`.
> Oracle priority: actual runtime behavior > tests > source > this file. Divergences are recorded in §8 and go to `docs/parity/BREAKS.md` when they affect users.

## 0. Server topology

Two host shapes share one package. A `Server` drives exactly one session (one controller, one broadcaster); a `Hub` drives several `Runtime`s behind one listener.

- `Server` struct: serve.go:38-102. One controller (`ctrl control.SessionAPI`), one `Broadcaster` (`bc`), one wire log, one title cache, one terminal manager. `bindMu` serializes every entry point that changes the active session path or controller generation — `/resume`, `/new`, `/fork`, switchModel, extension reload — because net/http runs handlers concurrently and serve serves multiple browser tabs (serve.go:43-51).
- `Hub` struct: hub.go:49-88. Owns what runtimes share: one `authGate`, the workspace tree, wallets (`billing.Store`), the Ask/Auto/YOLO `approvalStance` (hub.go:90-117 — one setting for the window, not per pane), a changes broadcaster, event muxes. Rule: one session file gets one runtime (`findSession` canonicalizes paths, hub.go:717-736).
- `Runtime`: hub.go:119-134 — `{ID, Root, Server, Events, handler, leases, stop}`; a remote pane has `remote != nil` with nil Server/Events and a proxying handler.
- `HubOptions` (hub.go:136-207) declares which optional route families exist: Remote, WSL, Asks, BrowserHost, Notifications + NotifySender, Tray, Install, NotesTransport/NotesDir, Update, Share. Each nil option leaves its route family UNREGISTERED (see §1.2 for which registrations are conditional).

Mounting trees (middleware listed outermost last):

```
Server.handler()  (serve.go:350-352)
  routes()                       — the 330-route session mux (§1.1)
  auth.mutationGate              — launch token on mutations when auth off (§2.5)
  csrfGuard                      — POST content-type guard (§2.9)
  auth.hostGuard                 — DNS-rebound Host allowlist (§2.4)
  auth.middleware                — none/token/password gate (§2.1-2.3)
  logMiddleware                  — request logging (§4.4)

Hub.Handler()  (hub.go:540-582)
  mux: /runtimes, /runtimes/{id}/close, tree routes, mux routes, studio version
       routes, /device, /device/leave, /rt/{id}/ -> routeRuntime,
       /host/settings/..., / -> routeDefault     — desktop surface (§1.2)
  hostMux: /host/capabilities, /host/pick-folder, remotes family, tray, notify,
       browser-host, update, asks, share, open-with, /studio/logs — window-only (§1.3)
  watchTreeWrites                — treeChanged after tree writes (hub_sync.go:71-104)
  hostOnly(hostMux, mux)         — devices reach mux, never hostMux (reach.go:63-77)
  auth.mutationGate
  csrfGuard
  withPage                       — built page files win over kernel (page.go:71-98)
  auth.hostGuard
  auth.middleware
  logMiddleware
```

- Runtime prefix: `runtimePrefix = "/rt/"` (hub.go:47). `publish()` freezes each runtime's handler as `http.StripPrefix("/rt/"+ID, Server.routes())` (hub.go:671-705) — runtime routes stay unprefixed. Runtime IDs are `"r"+seq` (hub.go:738-743).
- `routeRuntime` (hub.go:622-629): unknown id -> 404 `request.not_found` `{kind:"runtime", name:id}`; found -> rt.handler.
- `routeDefault` (hub.go:635-667): serves the FIRST LOCAL runtime's `routes()` unprefixed (a browser opened straight at the port, or an older frontend); remote-only -> proxy to the far kernel (behind refuseDeviceBeforeForwarding + refuseLocalOnly); no runtime -> GET / with a page while authenticated serves the shell, else 503 `hub.no_runtime_open`.
- `hostOnly(host, rest)` (reach.go:65-77): a path the host mux answers is refused to a paired device with 404 `device.host_only`; everything else goes to `rest`.
- `watchTreeWrites` (hub_sync.go:92-104): non-GET/HEAD requests whose path is a tree write (treeWrite, hub_sync.go:71-90: `/tree/...` except `/tree/sessions/export`, `/runtimes...`, `/new`, `/resume`, `/fork`, `/rewind`, `/summarize`, `/delete-session`, `/workspace`) trigger `treeChanged()` after a 2xx.
- `withPage(kernel, page)` (page.go:76-98): a path naming a real file in the built page is served as that file, with a fixed content-type map (page.go:64-69 — the Windows registry lookup can lie about SVG); any other path reaches the kernel, whose root hands the shell back.
- `InProcessClient()` (inprocess.go:48-97): an `http.Client` whose transport calls `Hub.Handler()` directly — no listener, no socket; requests are marked `inProcessKey` so `fromInProcess(r)` is true. This is how a same-process frontend reaches the kernel. `!`-prefixed shell commands are refused over any socket: 403 `shell.unavailable_over_http` (inprocess.go:26-32).

## 1. Route matrix (all 420 registrations, parsed from product files)

Sources: routes.go (the bare mux of the session host) + 58 `register*Routes` functions across the package (grep `\.HandleFunc\("`, product files only, 2026-11-15). Go 1.22 pattern syntax: `"METHOD /path"`; a spec without a method matches any method. Path wildcards `{id}` are Go 1.22 wildcards. This table is the complete path inventory; per-route JSON shapes and status codes follow in later sections.

### 1.1 Session host (`*Server`, handler `s.*`) — 330 routes

| Method | Path | Handler | Source |
|---|---|---|---|
| POST | `/account/login` | `s.accountLogin` | routes.go:108 |
| POST | `/account/logout` | `s.accountLogout` | routes.go:110 |
| POST | `/account/poll` | `s.accountPoll` | routes.go:109 |
| GET | `/account` | `s.account` | hub_settings.go:6 |
| GET | `/adjudications` | `s.adjudications` | routes.go:25 |
| POST | `/agent-composition` | `s.agentPresetEdit(s.selectAgentPreset)` | agent_presets.go:54 |
| GET | `/agent-presets/{id}/export` | `s.exportAgentPreset` | agent_presets.go:49 |
| DELETE | `/agent-presets/{id}` | `s.agentPresetEdit(func (inline: delete))` | agent_presets.go:41 |
| POST | `/agent-presets/default` | `s.agentPresetEdit(s.defaultAgentPreset)` | agent_presets.go:48 |
| POST | `/agent-presets/import` | `s.agentPresetEdit(s.importAgentPreset)` | agent_presets.go:50 |
| POST | `/agent-presets` | `s.agentPresetEdit(s.saveAgentPreset)` | agent_presets.go:40 |
| POST | `/answer` | `s.answer` | routes.go:81 |
| GET | `/appearance/wallpaper/{name}` | `s.wallpaperAsset` | appearance.go:42 |
| POST | `/appearance/wallpaper/scene` | `s.uploadWallpaperScene` | appearance.go:40 |
| DELETE | `/appearance/wallpaper` | `s.clearWallpaper` | appearance.go:41 |
| POST | `/appearance/wallpaper` | `s.uploadWallpaper` | appearance.go:39 |
| GET | `/appearance` | `s.appearance` | appearance.go:37 |
| POST | `/appearance` | `s.saveAppearance` | appearance.go:38 |
| POST | `/approve` | `s.approve` | routes.go:47 |
| GET | `/ask-timeout` | `s.askTimeoutSettings` | ask_timeout_settings.go:12 |
| POST | `/ask-timeout` | `s.saveAskTimeoutSettings` | ask_timeout_settings.go:13 |
| GET | `/assets/logo-wordmark.svg` | `s.logoWordmark` | routes.go:15 |
| POST | `/attachments/upload` | `s.attachmentUpload` | routes.go:126 |
| POST | `/attachments` | `s.attachments` | routes.go:125 |
| POST | `/auto-approve-tools` | `s.autoApproveTools` | routes.go:76 |
| GET | `/auto-archive` | `s.autoArchiveSettings` | auto_archive_settings.go:31 |
| POST | `/auto-archive` | `s.saveAutoArchiveSettings` | auto_archive_settings.go:32 |
| POST | `/backups/{id}/preview` | `s.previewBackup` | backup.go:26 |
| DELETE | `/backups/{id}` | `s.deleteBackup` | backup.go:25 |
| POST | `/backups/apply` | `s.applyBackup` | backup.go:27 |
| POST | `/backups/file/apply` | `s.applyBackupFile` | backup_file.go:23 |
| POST | `/backups/file/export` | `s.exportBackupFile` | backup_file.go:21 |
| POST | `/backups/file/preview` | `s.previewBackupFile` | backup_file.go:22 |
| GET | `/backups/file` | `s.backupFileCatalog` | backup_file.go:20 |
| GET | `/backups` | `s.listBackups` | backup.go:23 |
| POST | `/backups` | `s.createBackup` | backup.go:24 |
| GET | `/balance` | `s.balance` | routes.go:90 |
| POST | `/branch/at` | `s.branchAt` | routes.go:65 |
| GET | `/branches` | `s.branches` | routes.go:87 |
| GET | `/browser-tools` | `s.browserToolsSettings` | boundary.go:25 |
| POST | `/browser-tools` | `s.saveBrowserToolsSettings` | boundary.go:26 |
| POST | `/browser/close` | `s.browserClose` | browser_close.go:37 |
| POST | `/browser/open` | `s.browserOpen` | browser_close.go:36 |
| GET | `/browser/tabs` | `s.browserTabs` | browser_close.go:35 |
| POST | `/bypass` | `s.bypass` | routes.go:77 |
| POST | `/cancel` | `s.cancel` | routes.go:46 |
| GET | `/capability-scope` | `s.capabilityScopeHandler` | routes.go:107 |
| GET | `/changed-files-settings` | `s.changedFilesSettings` | changed_files_settings.go:9 |
| POST | `/changed-files-settings` | `s.saveChangedFilesSettings` | changed_files_settings.go:10 |
| GET | `/changes/diff` | `s.changeDiff` | commit.go:21 |
| POST | `/changes/discard` | `s.discardChange` | commit.go:30 |
| POST | `/changes/stage` | `s.stageChanges` | commit.go:29 |
| GET | `/changes` | `s.changes` | commit.go:20 |
| GET | `/checkpoints` | `s.checkpoints` | routes.go:86 |
| POST | `/commit/propose` | `s.commitPropose` | commit.go:27 |
| POST | `/commit` | `s.commitStaged` | commit.go:28 |
| GET | `/compaction` | `s.compactionSettings` | compaction_settings.go:8 |
| POST | `/compact` | `s.compact` | routes.go:54 |
| GET | `/complete` | `s.complete` | routes.go:105 |
| GET | `/config/problem` | `s.configProblem` | config_file.go:9 |
| POST | `/config/repair` | `s.repairConfigFile` | config_file.go:10 |
| POST | `/context/window` | `s.setContextWindow` | routes.go:28 |
| GET | `/context` | `s.context` | routes.go:24 |
| GET | `/default-mode` | `s.defaultModeSettings` | boundary.go:35 |
| POST | `/default-model` | `s.setDefaultModel` | hub_settings.go:34 |
| POST | `/default-mode` | `s.saveDefaultModeSettings` | boundary.go:36 |
| POST | `/delete-session` | `s.deleteSession` | routes.go:128 |
| GET | `/display-currency` | `s.displayCurrencySettings` | display_currency_settings.go:12 |
| POST | `/display-currency` | `s.saveDisplayCurrency` | display_currency_settings.go:13 |
| POST | `/drop` | `s.drop` | routes.go:127 |
| POST | `/effort` | `s.effort` | routes.go:52 |
| GET | `/events/replay` | `s.eventsReplay` | routes.go:22 |
| GET | `/events` | `s.events` | routes.go:21 |
| GET | `/execution-graph` | `s.executionGraph` | executiongraph.go:24 |
| POST | `/extensions/action` | `s.invokeExtensionAction` | extensions.go:16 |
| GET | `/extensions/actions` | `s.extensionActions` | extensions.go:14 |
| POST | `/extensions/reload` | `s.reloadExtensionsHTTP` | routes.go:88 |
| POST | `/extensions/submit` | `s.submitExtensionForm` | extensions.go:17 |
| GET | `/extensions/surfaces` | `s.extensionSurfaces` | extensions.go:15 |
| GET | `/feedback-level-notices` | `s.feedbackLevelNotices` | feedback_level_notice.go:9 |
| POST | `/feedback-level-notices` | `s.saveFeedbackLevelNotices` | feedback_level_notice.go:10 |
| POST | `/feedback/{receipt}/reply` | `s.feedbackReply` | feedback.go:65 |
| POST | `/feedback/{receipt}/seen` | `s.feedbackSeen` | feedback.go:66 |
| POST | `/feedback/{receipt}/withdraw` | `s.feedbackWithdraw` | feedback.go:68 |
| POST | `/feedback/board/join` | `s.feedbackBoardJoin` | feedback.go:75 |
| POST | `/feedback/board/leave` | `s.feedbackBoardLeave` | feedback.go:76 |
| GET | `/feedback/board/me` | `s.feedbackBoardMe` | feedback.go:74 |
| GET | `/feedback/board` | `s.feedbackBoard` | feedback.go:73 |
| GET | `/feedback/env` | `s.feedbackEnv` | feedback.go:61 |
| DELETE | `/feedback/link` | `s.feedbackUnlink` | feedback.go:71 |
| POST | `/feedback/link` | `s.feedbackLink` | feedback.go:70 |
| DELETE | `/feedback/links/{id}` | `s.feedbackUnlinkInstall` | feedback.go:72 |
| GET | `/feedback/links` | `s.feedbackLinks` | feedback.go:69 |
| GET | `/feedback/mine` | `s.feedbackMine` | feedback.go:62 |
| POST | `/feedback/name` | `s.feedbackName` | feedback.go:64 |
| POST | `/feedback/promotion/ack` | `s.feedbackPromotionAck` | feedback.go:67 |
| POST | `/feedback` | `s.feedbackSubmit` | feedback.go:63 |
| GET | `/fold-work-settings` | `s.foldWorkSettings` | fold_work_settings.go:9 |
| POST | `/fold-work-settings` | `s.saveFoldWorkSettings` | fold_work_settings.go:10 |
| POST | `/forget` | `s.forget` | routes.go:84 |
| GET | `/` | `s.index` | routes.go:13 |
| GET | `/git/commit/diff` | `s.commitDiff` | commit.go:19 |
| GET | `/git/commit` | `s.commitFiles` | commit.go:18 |
| GET | `/git/push` | `s.pushPreview` | commit.go:16 |
| POST | `/git/push` | `s.push` | commit.go:17 |
| GET | `/git/repos` | `s.gitRepos` | commit.go:15 |
| POST | `/goal/pause` | `s.goalPause` | routes.go:80 |
| POST | `/goal/resume` | `s.goalResume` | routes.go:79 |
| POST | `/goal` | `s.goal` | routes.go:78 |
| GET | `/history/page` | `s.historyPage` | history.go:190 |
| GET | `/history/tool/{id}` | `s.historyToolOutput` | history.go:191 |
| GET | `/history` | `s.history` | history.go:189 |
| POST | `/hooks/dry-run` | `s.dryRunHook` | routes.go:99 |
| GET | `/hooks` | `s.hooks` | routes.go:97 |
| POST | `/hooks` | `s.saveHooks` | routes.go:98 |
| POST | `/inbox/items/{id}/refresh` | `s.inboxRefresh` | inbox.go:23 |
| POST | `/inbox/items/{id}/retry` | `s.inboxRetry` | inbox.go:22 |
| DELETE | `/inbox/items/{id}` | `s.inboxDelete` | inbox.go:18 |
| GET | `/inbox/items/{id}` | `s.inboxGet` | inbox.go:16 |
| PATCH | `/inbox/items/{id}` | `s.inboxUpdate` | inbox.go:17 |
| POST | `/inbox/items` | `s.inboxEnqueue` | inbox.go:15 |
| POST | `/inbox/move` | `s.inboxMove` | inbox.go:19 |
| POST | `/inbox/pause` | `s.inboxPause` | inbox.go:20 |
| POST | `/inbox/resume` | `s.inboxResume` | inbox.go:21 |
| GET | `/inbox` | `s.inboxList` | inbox.go:14 |
| POST | `/jobs/{id}/cancel` | `s.cancelJob` | jobs.go:9 |
| GET | `/jobs/{id}/output` | `s.jobOutput` | jobs.go:10 |
| POST | `/market/install` | `s.marketInstall` | market.go:34 |
| POST | `/market/mine/{handle}/{name}/submit` | `s.marketOwnSubmit` | market_publish.go:23 |
| POST | `/market/mine/install` | `s.marketOwnInstall` | market_publish.go:22 |
| POST | `/market/mine/plan` | `s.marketOwnPlan` | market_publish.go:21 |
| GET | `/market/mine` | `s.marketMine` | market_publish.go:20 |
| GET | `/market/packages/{handle}/{name}/vote` | `s.marketMyVote` | market_votes.go:28 |
| POST | `/market/packages/{handle}/{name}/vote` | `s.marketVote` | market_votes.go:29 |
| GET | `/market/packages/{handle}/{name}` | `s.marketDetail` | market.go:32 |
| GET | `/market/packages` | `s.marketList` | market.go:31 |
| POST | `/market/plan` | `s.marketPlan` | market.go:33 |
| POST | `/market/publish` | `s.marketPublish` | market_publish.go:19 |
| POST | `/mcp/enabled` | `s.mcpEnabled` | catalog.go:445 |
| POST | `/mcp/follow` | `s.mcpFollow` | catalog.go:444 |
| POST | `/mcp/install` | `s.mcpInstall` | catalog.go:448 |
| POST | `/mcp/load` | `s.mcpLoad` | catalog.go:446 |
| POST | `/mcp/parse` | `s.mcpParse` | catalog.go:447 |
| POST | `/mcp/reconnect` | `s.mcpReconnect` | catalog.go:440 |
| POST | `/mcp/remove` | `s.mcpRemove` | catalog.go:449 |
| POST | `/mcp/signin` | `s.mcpSignIn` | catalog.go:442 |
| POST | `/mcp/trust` | `s.mcpTrust` | catalog.go:441 |
| POST | `/mcp/upgrade` | `s.mcpUpgrade` | catalog.go:443 |
| GET | `/mcp` | `s.mcp` | catalog.go:439 |
| POST | `/memory/confirm` | `s.confirmMemory` | hub_settings.go:23 |
| POST | `/memory/forget` | `s.forgetMemory` | hub_settings.go:21 |
| GET | `/memory/proactive` | `s.memoryProactive` | hub_settings.go:24 |
| POST | `/memory/proactive` | `s.saveMemoryProactive` | hub_settings.go:25 |
| POST | `/memory/restore` | `s.restoreMemory` | hub_settings.go:30 |
| GET | `/memory/revisions` | `s.memoryRevisions` | hub_settings.go:29 |
| POST | `/memory/save` | `s.saveMemory` | hub_settings.go:22 |
| POST | `/memory/suggestions/accept` | `s.acceptMemorySuggestion` | hub_settings.go:27 |
| POST | `/memory/suggestions/dismiss` | `s.dismissMemorySuggestion` | hub_settings.go:28 |
| GET | `/memory/suggestions` | `s.memorySuggestions` | hub_settings.go:26 |
| GET | `/memory` | `s.memories` | hub_settings.go:20 |
| POST | `/model-mode` | `s.modelMode` | routes.go:53 |
| POST | `/model` | `s.model` | settings.go:37 |
| GET | `/models` | `s.models` | hub_settings.go:19 |
| GET | `/mods/settings` | `s.modsSettings` | mods.go:18 |
| POST | `/mods/settings` | `s.saveModsSettings` | mods.go:19 |
| GET | `/mods` | `s.mods` | mods.go:17 |
| POST | `/network/diagnose` | `s.diagnoseNetwork` | hub_settings.go:33 |
| GET | `/network` | `s.network` | hub_settings.go:31 |
| POST | `/network` | `s.saveNetwork` | hub_settings.go:32 |
| POST | `/new` | `s.newSession` | routes.go:63 |
| GET | `/observability` | `s.observabilitySettings` | observability.go:13 |
| POST | `/observability` | `s.saveObservabilitySettings` | observability.go:14 |
| POST | `/peers/held/{id}` | `s.resolvePeerMessage` | peers.go:31 |
| POST | `/peers/send` | `s.sendPeerMessage` | peers.go:30 |
| GET | `/peers/settings` | `s.peersSettings` | peers.go:32 |
| POST | `/peers/settings` | `s.savePeersSettings` | peers.go:33 |
| GET | `/peers` | `s.peers` | peers.go:29 |
| POST | `/permissions/remembered/revoke` | `s.revokeRememberedProjectRule` | boundary.go:19 |
| POST | `/permissions/revoke` | `s.revokeSessionGrant` | boundary.go:18 |
| GET | `/permissions` | `s.permissions` | boundary.go:16 |
| POST | `/permissions` | `s.savePermissions` | boundary.go:17 |
| GET | `/persona` | `s.personaSettings` | persona_settings.go:15 |
| POST | `/persona` | `s.savePersonaSettings` | persona_settings.go:16 |
| GET | `/plan-approval` | `s.planApprovalSettings` | boundary.go:33 |
| POST | `/plan-approval` | `s.savePlanApprovalSettings` | boundary.go:34 |
| POST | `/plan-decision` | `s.planDecision` | routes.go:48 |
| POST | `/plan` | `s.plan` | routes.go:49 |
| GET | `/plugins/{name}/export` | `s.exportPlugin` | plugins.go:30 |
| DELETE | `/plugins/{name}` | `s.removePlugin` | plugins.go:29 |
| POST | `/plugins/enabled` | `s.pluginEnabled` | plugins.go:28 |
| POST | `/plugins/install` | `s.installPlugin` | plugins.go:27 |
| POST | `/plugins/plan` | `s.planPlugin` | plugins.go:26 |
| GET | `/plugins` | `s.plugins` | plugins.go:25 |
| GET | `/present-files` | `s.presentFilesSettings` | boundary.go:27 |
| POST | `/present-files` | `s.savePresentFilesSettings` | boundary.go:28 |
| POST | `/preset` | `s.preset` | routes.go:50 |
| GET | `/processes` | `s.processes` | processes.go:13 |
| GET | `/progress-watch` | `s.progressWatchSettings` | progress_watch_settings.go:13 |
| POST | `/progress-watch` | `s.saveProgressWatchSettings` | progress_watch_settings.go:14 |
| POST | `/prompt/refine` | `s.refinePrompt` | routes.go:85 |
| POST | `/provider-setup/connection` | `s.connectionSave` | routes.go:20 |
| GET | `/provider-setup/connections` | `s.connectionsList` | routes.go:18 |
| POST | `/provider-setup/test` | `s.connectionTest` | routes.go:19 |
| GET | `/provider-setup` | `s.providerSetupStatus` | routes.go:16 |
| POST | `/provider-setup` | `s.providerSetupSave` | routes.go:17 |
| POST | `/providers/check/model` | `s.checkProviderModel` | provider_check.go:85 |
| POST | `/providers/check` | `s.checkProvider` | provider_check.go:84 |
| POST | `/providers/display-name` | `s.renameProvider` | providers.go:51 |
| POST | `/providers/edit` | `s.editProvider` | providers.go:50 |
| GET | `/providers/health/settings` | `s.providerHealthSettings` | provider_check.go:82 |
| POST | `/providers/health/settings` | `s.saveProviderHealthSettings` | provider_check.go:83 |
| POST | `/providers/health` | `s.providerHealth` | provider_check.go:81 |
| POST | `/providers/presets/install` | `s.installProviderPreset` | providers.go:46 |
| GET | `/providers/presets` | `s.providerPresets` | providers.go:45 |
| POST | `/providers/probe` | `s.probeProvider` | providers.go:48 |
| GET | `/providers/protocols` | `s.providerProtocols` | providers.go:44 |
| POST | `/providers/remove` | `s.removeProvider` | providers.go:49 |
| GET | `/providers` | `s.providers` | providers.go:43 |
| POST | `/providers` | `s.saveProvider` | providers.go:47 |
| POST | `/resume` | `s.resume` | routes.go:82 |
| POST | `/rewind/commit` | `s.rewindCommit` | routes.go:68 |
| POST | `/rewind/file/commit` | `s.fileRevertCommit` | routes.go:72 |
| POST | `/rewind/file/prepare` | `s.fileRevertPrepare` | routes.go:71 |
| POST | `/rewind/prepare` | `s.rewindPrepare` | routes.go:67 |
| GET | `/rewind/undo` | `s.rewindAvailableUndo` | routes.go:70 |
| POST | `/rewind/undo` | `s.rewindUndo` | routes.go:69 |
| POST | `/rewind` | `s.rewind` | routes.go:64 |
| POST | `/roles/overrides/clear` | `s.clearRoleOverride` | roles.go:51 |
| GET | `/roles/overrides` | `s.roleOverrides` | roles.go:50 |
| GET | `/roles` | `s.roles` | roles.go:48 |
| POST | `/roles` | `s.setRole` | roles.go:49 |
| GET | `/sandbox` | `s.sandboxSettings` | boundary.go:20 |
| POST | `/sandbox` | `s.saveSandboxSettings` | boundary.go:24 |
| POST | `/schedules/{id}/delete` | `s.scheduleOne("delete")` | schedules.go:23 |
| POST | `/schedules/{id}/pause` | `s.scheduleOne("pause")` | schedules.go:20 |
| POST | `/schedules/{id}/renew` | `s.scheduleOne("renew")` | schedules.go:22 |
| POST | `/schedules/{id}/resume` | `s.scheduleOne("resume")` | schedules.go:21 |
| POST | `/schedules/pause-all` | `s.scheduleSwitch(true)` | schedules.go:18 |
| POST | `/schedules/preview` | `s.previewSchedule` | schedules.go:16 |
| GET | `/schedules/result` | `s.scheduleResult` | schedules.go:15 |
| POST | `/schedules/resume-all` | `s.scheduleSwitch(false)` | schedules.go:19 |
| GET | `/schedules` | `s.schedules` | schedules.go:14 |
| POST | `/schedules` | `s.createSchedule` | schedules.go:17 |
| GET | `/session-ref-settings` | `s.sessionRefSettings` | session_ref_settings.go:12 |
| POST | `/session-ref-settings` | `s.saveSessionRefSettings` | session_ref_settings.go:13 |
| POST | `/session/tail` | `s.sessionTail` | routes.go:66 |
| GET | `/sessions/{id}` | `s.index` | routes.go:14 |
| POST | `/sessions/export` | `s.exportConversation` | export.go:14 |
| POST | `/sessions/rename` | `s.renameSessionAt` | session_viewed.go:6 |
| POST | `/sessions/viewed` | `s.sessionViewed` | session_viewed.go:7 |
| GET | `/sessions` | `s.sessions` | routes.go:91 |
| GET | `/settings-changed` | `s.changedSettings` | changed_settings.go:6 |
| GET | `/shell` | `s.shellSettings` | shell.go:10 |
| POST | `/shell` | `s.saveShellSettings` | shell.go:11 |
| POST | `/side/{id}/ask` | `s.askSide` | side.go:38 |
| DELETE | `/side/{id}` | `s.closeSide` | side.go:39 |
| POST | `/side` | `s.openSide` | side.go:37 |
| POST | `/skills/{name}/update/apply` | `s.applySkillUpdate` | skill_update.go:32 |
| POST | `/skills/{name}/update/plan` | `s.planSkillUpdate` | skill_update.go:31 |
| POST | `/skills/enabled` | `s.skillEnabled` | routes.go:101 |
| POST | `/skills/remove` | `s.skillRemove` | skill_remove.go:14 |
| GET | `/skills` | `s.skills` | routes.go:100 |
| GET | `/slash` | `s.slash` | routes.go:104 |
| POST | `/storage/move` | `s.storageMove` | storage.go:117 |
| POST | `/storage/plan` | `s.storagePlan` | storage.go:116 |
| GET | `/storage` | `s.storage` | storage.go:115 |
| GET | `/subagent-claims` | `s.subagentClaimsSettings` | boundary.go:29 |
| POST | `/subagent-claims` | `s.saveSubagentClaimsSettings` | boundary.go:30 |
| GET | `/subagents/{ref}/transcript` | `s.subagentTranscript` | subagents.go:19 |
| GET | `/subagents` | `s.subagentRuns` | subagents.go:18 |
| POST | `/submit` | `s.submit` | routes.go:29 |
| POST | `/summarize` | `s.summarize` | routes.go:73 |
| GET | `/surfaces/{id}/action` | `s.surfaceActionState` | surfaces.go:19 |
| POST | `/surfaces/action` | `s.surfaceAction` | surfaces.go:20 |
| GET | `/surfaces` | `s.surfaceSlots` | surfaces.go:21 |
| POST | `/surfaces` | `s.assignSurfaceSlot` | surfaces.go:22 |
| POST | `/terminals/{id}/input` | `s.terminalInput` | terminals.go:38 |
| POST | `/terminals/{id}/resize` | `s.terminalResize` | terminals.go:37 |
| POST | `/terminals/{id}/restart` | `s.terminalRestart` | terminals.go:36 |
| GET | `/terminals/{id}/stream` | `s.terminalStream` | terminals.go:39 |
| DELETE | `/terminals/{id}` | `s.terminalClose` | terminals.go:35 |
| GET | `/terminals/shells` | `s.terminalShells` | terminals.go:32 |
| GET | `/terminals` | `s.terminalList` | terminals.go:33 |
| POST | `/terminals` | `s.terminalOpen` | terminals.go:34 |
| GET | `/themes/{id}/{asset}` | `s.themeAsset` | themes.go:24 |
| DELETE | `/themes/{id}` | `s.removeTheme` | themes.go:23 |
| POST | `/themes/folder` | `s.openThemeFolder` | themes.go:22 |
| POST | `/themes/import` | `s.importTheme` | themes.go:21 |
| GET | `/themes` | `s.themes` | themes.go:19 |
| POST | `/themes` | `s.activateTheme` | themes.go:20 |
| GET | `/todos` | `s.todos` | routes.go:116 |
| POST | `/tool-approval-mode` | `s.toolApprovalMode` | routes.go:74 |
| POST | `/tool-calls/{id}/cancel` | `s.cancelToolCall` | tool_calls.go:6 |
| POST | `/tools/disabled` | `s.saveToolDisabled` | boundary.go:38 |
| GET | `/tools` | `s.toolSettings` | boundary.go:37 |
| GET | `/trajectory` | `s.trajectory` | routes.go:92 |
| GET | `/turn-narration-settings` | `s.turnNarrationSettings` | turn_narration_settings.go:12 |
| POST | `/turn-narration-settings` | `s.saveTurnNarrationSettings` | turn_narration_settings.go:13 |
| GET | `/usage/upload` | `s.uploadUsage` | hub_settings.go:8 |
| GET | `/usage` | `s.usage` | hub_settings.go:7 |
| POST | `/web-access-settings/probe` | `s.probeSearxng` | web_access_settings.go:15 |
| GET | `/web-access-settings` | `s.webAccessSettings` | web_access_settings.go:13 |
| POST | `/web-access-settings` | `s.saveWebAccessSettings` | web_access_settings.go:14 |
| GET | `/welcome` | `s.welcomeState` | welcome.go:11 |
| POST | `/welcome` | `s.markWelcomed` | welcome.go:12 |
| POST | `/workspace-trust` | `s.workspaceTrust` | routes.go:75 |
| POST | `/workspace/branch/switch` | `s.workspaceSwitchBranch` | commit.go:26 |
| GET | `/workspace/branches` | `s.workspaceBranches` | commit.go:24 |
| POST | `/workspace/editor` | `s.openInEditor` | routes.go:122 |
| GET | `/workspace/file` | `s.workspaceFileRead` | routes.go:118 |
| PUT | `/workspace/file` | `s.workspaceFileWrite` | routes.go:121 |
| GET | `/workspace/files` | `s.workspaceFiles` | routes.go:117 |
| GET | `/workspace/git/log` | `s.gitHistory` | commit.go:23 |
| GET | `/workspace/git` | `s.workspaceGit` | commit.go:22 |
| GET | `/workspace/image` | `s.workspaceImage` | routes.go:120 |
| GET | `/workspace/locate` | `s.workspaceLocate` | routes.go:124 |
| POST | `/workspace/open-document` | `s.openDocument` | routes.go:123 |
| POST | `/workspace/prepare/{id}/cancel` | `s.readWorkspacePreparation` | routes.go:115 |
| GET | `/workspace/prepare/{id}` | `s.readWorkspacePreparation` | routes.go:114 |
| POST | `/workspace/prepare` | `s.startWorkspacePreparation` | routes.go:113 |
| GET | `/workspace/preview/content` | `s.workspacePreviewContent` | workspace_preview.go:16 |
| GET | `/workspace/preview/info` | `s.workspacePreviewInfo` | workspace_preview.go:15 |
| POST | `/workspace/roots/remove` | `s.removeWorkspaceRoot` | boundary.go:23 |
| GET | `/workspace/roots` | `s.workspaceRoots` | boundary.go:21 |
| POST | `/workspace/roots` | `s.addWorkspaceRoot` | boundary.go:22 |
| GET | `/workspace/worktree-sources` | `s.workspaceSources` | commit.go:25 |
| POST | `/workspace` | `s.workspace` | routes.go:112 |
| GET | `/workspaces` | `s.workspaces` | routes.go:111 |
| GET | `/write-lease` | `s.writeLeaseSettings` | boundary.go:31 |
| POST | `/write-lease` | `s.saveWriteLeaseSettings` | boundary.go:32 |

### 1.2 Desktop-host surface (`h.*` handlers and inline closures on a `mux`) — 74 routes

| Method | Path | Handler | Source |
|---|---|---|---|
| ANY | `/` | `h.routeDefault` | hub.go:553 |
| GET | `/agent-composition` | `func (inline: composition)` | agent_presets.go:51 |
| GET | `/agent-presets/{id}` | `func (inline: preset detail)` | agent_presets.go:26 |
| GET | `/agent-presets` | `func (inline: preset list)` | agent_presets.go:18 |
| POST | `/asks/{id}/answer` | `h.answerAsk` | ask.go:228 |
| GET | `/asks` | `h.listAsks` | ask.go:227 |
| POST | `/browser-host/frames` | `routine(h.browserHostFrames)` | browser_host.go:184 |
| GET | `/browser-host/stream` | `h.browserHostStream` | browser_host.go:183 |
| POST | `/device/leave` | `notADevice` | hub.go:549 |
| GET | `/device` | `notADevice` | hub.go:548 |
| GET | `/events/mux/{conn}/metrics` | `h.muxMetrics` | hub_mux.go:267 |
| DELETE | `/events/mux/{conn}/panes/{id}` | `h.muxDetach` | hub_mux.go:270 |
| PUT | `/events/mux/{conn}/panes/{id}` | `h.muxAttach` | hub_mux.go:269 |
| GET | `/events/mux` | `h.muxStream` | hub_mux.go:268 |
| GET | `/history/search` | `historySearch` | hub_tree.go:31 |
| GET | `/host/open-with` | `h.openWithTargets` | hub_openwith.go:29 |
| POST | `/host/open-with` | `h.openWithProject` | hub_openwith.go:30 |
| ANY | `/host/settings/runtime/{runtime}/` | `h.localWorkspaceSettings` | hub.go:551 |
| POST | `/notifications/feedback-reply` | `h.announceFeedbackReply` | notifications.go:49 |
| GET | `/notifications` | `h.readNotifyPrefs` | notifications.go:46 |
| PUT | `/notifications` | `h.writeNotifyPrefs` | notifications.go:47 |
| POST | `/remotes/{host}/sessions/archive` | `h.remoteSessionFlag("archive")` | remote_session_flags.go:11 |
| POST | `/remotes/{host}/sessions/pin` | `h.remoteSessionFlag("pin")` | remote_session_flags.go:10 |
| POST | `/remotes/{host}/sessions/remove` | `h.removeRemoteSession` | remote_session_flags.go:9 |
| POST | `/remotes/{host}/sessions/viewed` | `h.remoteSessionFlag("viewed")` | remote_session_flags.go:12 |
| POST | `/runtimes/{id}/close` | `h.closeRuntime` | hub.go:544 |
| GET | `/runtimes` | `h.listRuntimes` | hub.go:542 |
| POST | `/runtimes` | `h.openRuntime` | hub.go:543 |
| POST | `/share/close` | `h.shareClose` | device_share_routes.go:33 |
| POST | `/share/cloud-offer` | `h.shareCloudOffer` | device_share_routes.go:35 |
| POST | `/share/offer` | `h.shareOffer` | device_share_routes.go:34 |
| POST | `/share/open` | `h.shareOpen` | device_share_routes.go:31 |
| POST | `/share/phones/resolve` | `h.sharePhoneResolve` | share_phones.go:91 |
| POST | `/share/phones/revoke` | `h.sharePhoneRevoke` | share_phones.go:92 |
| GET | `/share/phones` | `h.sharePhones` | share_phones.go:90 |
| POST | `/share/port` | `h.shareSetPort` | device_share_routes.go:32 |
| GET | `/share/remember` | `h.shareRememberGet` | device_share_routes.go:37 |
| POST | `/share/remember` | `h.shareRememberSave` | device_share_routes.go:38 |
| POST | `/share/revoke` | `h.shareRevoke` | device_share_routes.go:36 |
| GET | `/share` | `h.shareStatus` | device_share_routes.go:30 |
| GET | `/status` | `routine(s.status)` | routes.go:89 |
| POST | `/studio/pin` | `h.pinStudioVersion` | studio_version.go:39 |
| GET | `/studio/update-mode` | `h.readUpdateMode` | studio_version.go:40 |
| POST | `/studio/update-mode` | `h.saveUpdateMode` | studio_version.go:41 |
| GET | `/studio/versions/{version}/notes` | `h.readStudioNotes` | studio_version.go:42 |
| GET | `/studio/versions` | `h.readStudioVersions` | studio_version.go:38 |
| GET | `/tray/prefs` | `h.readTrayPrefs` | tray.go:100 |
| PUT | `/tray/prefs` | `h.writeTrayPrefs` | tray.go:101 |
| GET | `/tray/running` | `h.readTrayRunning` | tray.go:103 |
| GET | `/tray/state` | `h.readTrayState` | tray.go:102 |
| GET | `/tree/events/replay` | `h.treeEventsReplay` | hub_tree.go:33 |
| GET | `/tree/events` | `h.treeEvents` | hub_tree.go:32 |
| POST | `/tree/sessions/archive` | `h.archiveSession` | hub_tree.go:39 |
| POST | `/tree/sessions/auto-name` | `h.autoNameSession` | hub_tree.go:46 |
| POST | `/tree/sessions/export` | `h.exportSession` | hub_tree.go:47 |
| POST | `/tree/sessions/import-legacy` | `h.importLegacySessions` | hub_tree.go:48 |
| POST | `/tree/sessions/import-native` | `h.importNativeSession` | hub_tree.go:49 |
| POST | `/tree/sessions/move-to` | `h.moveSessionTo` | hub_tree.go:43 |
| POST | `/tree/sessions/move` | `h.moveSession` | hub_tree.go:42 |
| POST | `/tree/sessions/pin` | `h.pinSession` | hub_tree.go:40 |
| POST | `/tree/sessions/pins` | `h.syncPins` | hub_tree.go:44 |
| POST | `/tree/sessions/remove-many` | `h.removeSessions` | hub_tree.go:38 |
| POST | `/tree/sessions/remove` | `h.removeSession` | hub_tree.go:37 |
| POST | `/tree/sessions/rename` | `h.renameSession` | hub_tree.go:45 |
| POST | `/tree/sessions/viewed` | `h.markSessionViewed` | hub_tree.go:41 |
| POST | `/tree/workspaces/move` | `h.moveWorkspace` | hub_tree.go:36 |
| POST | `/tree/workspaces/remove` | `h.removeWorkspace` | hub_tree.go:35 |
| POST | `/tree/workspaces` | `h.addWorkspace` | hub_tree.go:34 |
| GET | `/tree` | `h.tree` | hub_tree.go:30 |
| POST | `/update/cancel` | `h.cancelInstall` | update.go:57 |
| POST | `/update/health` | `h.acknowledgeLaunchHealth` | update.go:55 |
| GET | `/update/install` | `h.readInstallProgress` | update.go:58 |
| POST | `/update/install` | `h.startInstall` | update.go:56 |
| POST | `/update/restart` | `h.commitInstall` | update.go:59 |

### 1.3 Host-scoped mux (`hostMux`) — 16 routes

| Method | Path | Handler | Source |
|---|---|---|---|
| GET | `/host/capabilities` | `h.hostCapabilities` | hub.go:557 |
| POST | `/host/pick-folder` | `h.pickLocalFolderHTTP` | hub.go:558 |
| GET | `/host/workspaces/locate` | `h.locateWorkspace` | hub.go:559 |
| GET | `/remotes/{host}/dirs` | `h.remoteDirs` | hub.go:566 |
| POST | `/remotes/{host}/place` | `h.placeRemoteHost` | hub.go:570 |
| GET | `/remotes/{host}/probe` | `h.remoteProbe` | hub.go:567 |
| GET | `/remotes/{host}/tree` | `h.remoteTree` | hub.go:564 |
| POST | `/remotes/{host}/workspaces/remove` | `h.removeRemoteWorkspace` | hub.go:569 |
| POST | `/remotes/{host}/workspaces` | `h.addRemoteWorkspace` | hub.go:568 |
| GET | `/remotes/candidates` | `h.remoteCandidates` | hub.go:562 |
| POST | `/remotes/open` | `h.openRemoteRuntime` | hub.go:572 |
| POST | `/remotes/remove` | `h.removeRemoteHost` | hub.go:571 |
| GET | `/remotes/wsl` | `h.remoteWSLDistros` | hub.go:563 |
| GET | `/remotes` | `h.listRemoteHosts` | hub.go:560 |
| POST | `/remotes` | `h.saveRemoteHost` | hub.go:561 |
| GET | `/studio/logs` | `h.readStudioLogs` | hub.go:580 |


## 2. Auth model

### 2.1 Modes

`authMode`: none | token | password, plus `authInvalid` (deny everything) when the configured mode string does not normalize (auth.go:32-49). `NormalizeAuthMode` (auth.go:52-64) lowercases/trims; empty -> none. Invalid -> every request is denied (auth.go:219-221).

`deny` (auth.go:403-409) is NOT a Reason envelope: plain text `Unauthorized\n` with 401 — deliberately generic so it leaks nothing about which mode is active.

### 2.2 Token mode

- Token: 32 random bytes, base64url (auth.go:46, 626-633); a configured token wins, unset -> generated per launch (auth.go:144-150).
- Credential accepted from (`checkToken`, auth.go:316-354):
  1. cookie `reasonix_token` or `Authorization: Bearer <token>`, constant-time compare (launch_token.go:37-51);
  2. legacy `?token=` query parameter — on match sets the cookie (HttpOnly, Lax, 30d) and 302-redirects to the same path with the parameter stripped (auth.go:327-349); redirect targets are sanitized (`safeRedirectTarget`, auth.go:507-530: same-origin paths only, no `//`, no backslashes);
  3. otherwise 401 deny.
- Shell bootstrap while unauthenticated: only `tokenBootstrapPublicPath` is public — GET/HEAD of `/`, `/assets/logo-wordmark.svg`, or exactly one `/sessions/{id}` segment (auth.go:256-268). The page then POSTs `/auth/token` `{token}` (JSON, <=8KiB, exactly one JSON value) -> constant-time compare -> HttpOnly `reasonix_token` cookie, 30d -> 204 (auth.go:270-314). Mismatch -> 401 deny. Keeping the token in a body keeps it out of request lines, logs, history, referrers.

### 2.3 Password mode

- Hash: bcrypt cost 12 (auth.go:47, 187-195). Session key: PBKDF2-HMAC-SHA256 over the password hash, fixed salt `"reasonix serve session key"`, 32 bytes (auth.go:197-212); random 32-byte key when no hash is configured.
- `/login` and `/login/` are intercepted INSIDE `auth.middleware` before the CSRF guard — the form posts `application/x-www-form-urlencoded`, not JSON (auth.go:231-236). GET -> embedded `login.html` (auth.go:29-30, 423-427); other methods -> 405 `request.method_not_allowed`.
- `loginSubmit` (auth.go:429-495): rate limit first (below), body capped at 4KiB (`maxLoginFormBytes`); empty password -> login page with "Password is required."; no hash configured -> "Server not configured for password authentication."; bcrypt mismatch -> "Invalid password." (all 200 HTML, not refusal codes); success -> clear `reasonix_redirect` cookie, set HMAC session cookie `reasonix_session` (HttpOnly, Lax, Secure=isTLS, 30d) and 302 to the sanitized redirect target or `/`.
- Session format: `base64url(expiry_base10|random_16_bytes).hex(hmac_sha256)`, 30d lifetime (signSession auth.go:551-570; verifySession auth.go:573-604).
- Brute force: 5 attempts per IP per minute (auth.go:77-79); over the limit -> 429 plain text "Too many attempts. Please wait a minute.\n" (auth.go:432-441). `X-Forwarded-For` trusted only when `serve.behind_proxy` (`clientIP`, auth.go:646-664).
- Unauthenticated (`checkSession`, auth.go:372-401): browsers (Accept prefers text/html) get a `reasonix_redirect` cookie (5 min) and 302 to `/login`; API/SSE get 401 deny.

### 2.4 hostGuard — DNS-rebound allowlist (auth none only)

When auth is off, `Host` must be one the listener was bound to: `ListenOn(addr)` records the bound host; a wildcard bind expands to this machine's interface IP literals, never a name (host_guard.go:25-52). Loopback always passes (host_guard.go:72-80). A request through a device gate, the in-process transport, or carrying the launch token passes. Otherwise: 421 `serve.host_rejected` (host_guard.go:93-104). A request off a socket cannot forge any of those marks.

### 2.5 mutationGate — launch token for mutations (auth none only)

Any non-GET/HEAD/OPTIONS request requires the launch token (cookie or Bearer) unless `provenOperator` — in-process, host-gated via the loopback gate, or a paired device (launch_token.go:24-33) — else 403 `auth.launch_token_required` (launch_token.go:53-69). Approvals and every other mutation stay gated even with auth off: the METHOD decides, not a list of routes.

### 2.6 Loopback gate (desktop host control plane)

`ListenLoopback()` binds `tcp4 127.0.0.1:0` only — no wildcard, no name (loopback.go:34-39). `NewLoopbackGate` (loopback.go:49-83) checks, outermost first so a caller under the wrong name learns nothing about the credential: Host == listener authority (else 403 `loopback.host_rejected`); Origin == listener origin for state changes — an empty Origin is allowed for GET/HEAD only (else 403 `loopback.origin_rejected`); cookie `reasonix_token` constant-time, cookie only — no query token here (else 403 `loopback.unauthorized`). A gate built without both halves refuses everything with 500 `loopback.misconfigured`. Passing requests are marked `withHostGated` (launch_token.go:18-20) — the mark `provenOperator` reads.

### 2.7 Device gate (paired devices)

`NewDeviceGate` (device_gate.go:60-95): Host must equal the configured origin authority (else 403 `device.host_rejected`); Origin must be allowed (else 403 `device.origin_rejected`); `/pair` is answered inside the gate, not on any mux (device_gate.go:77) — POST, JSON `{code}`, <=8KiB, registry redeem -> `reasonix_device` cookie (HttpOnly, Strict SameSite, Secure=origin-is-https, 30d); invalid code -> 403 `device.pairing_invalid` (device_gate.go:156-197). A device with a valid cookie gets its own `GET /device` -> `{id, ordinal, machine}` and `POST /device/leave` -> revoke + clear cookie + 204 (device_gate.go:100-113). An unpaired device may load only page files and shell entry points (`devicePublicPath`, device_gate.go:138-154); anything else -> 401 `device.unauthorized`. Device requests are marked `withDeviceReach` (reach.go:27-29): `hostReach(r)` is false for them, so window grants and hostMux stay shut; an SSE stream from a device runs under a context unpairing cancels.

### 2.8 hostOnlyRoutes — what a paired device may never call

27 routes (host_routes.go:16-44): git push (GET+POST), changes/discard, feedback board (me/join/leave), plugins install/enabled/delete, mcp install/trust/enabled/load/follow/upgrade, hooks, market install (+mine), workspace-trust, and the terminals family (shells, list, create, delete, restart, resize, input, stream). A device hitting one -> 404 `device.host_only`. Matching: segment by segment, `{name}` matches one non-empty segment, HEAD counts as GET, `/rt/{id}/` prefixes are stripped first (host_routes.go:48-78). `GET /history/tool/{id}` is deliberately NOT on the list — it is a window onto what GET /history carries (host_routes.go:46-47). This list is enforced at THREE points: inside the device gate before routing, by `refuseDeviceBeforeForwarding` in front of remote-pane proxies, and by `refuseDevice` in handlers — the Rust port must keep all three.

### 2.9 csrfGuard and CORS

POST must carry `Content-Type: application/json`, except binary upload routes — path suffix `/attachments/upload` or `/appearance/wallpaper` with `application/octet-stream` — else 415 `request.bad_content_type` (serve.go:362-385). There is no CORS: same-origin policy is the protection and there is no cross-origin opt-in; a dev frontend proxies through its own origin (see §6) rather than being allowed in (serve.go:341-345).

### 2.10 Network binding safety

`OpenBindRefusal` (auth.go:710-718): auth `none` on a non-loopback bind is refused at startup with a sentence naming the fix, unless the operator passes `--allow-open-network`. `PlainHTTPAuthWarning` (auth.go:696-708) flags authenticated access over plain non-loopback HTTP.

## 3. Binding model

- `Server.Run(addr)` (listen.go:16-20): interactive approval on, host allowlist records the address, plain `http.ListenAndServe`.
- `RunGraceful`/`RunGracefulListener` (listen.go:22-40): listen first (so `--addr 127.0.0.1:0` + `--port-file` can record the real address), then serve until the context ends.
- `runGracefulListener` (listen.go:52-81): `ReadHeaderTimeout 10s`, `IdleTimeout 120s`; on context end, `Shutdown` with a 10s drain budget; `http.ErrServerClosed` -> nil.
- Desktop host: `ListenLoopback()` + `LoopbackOrigin(ln)` + `NewLoopbackGate` (§2.6) — the only listener shape the desktop control plane uses.
- Same-process: `InProcessClient()` — no socket at all (§0).
- Static page resolution `FindPage` (page.go:17-61): an explicit dir must contain index.html (else error — "a launch that would open on nothing"); else beside the executable (following symlinks, page.go:42-56); else `../frontend-next/dist`, `desktop/frontend-next/dist`; else nil — the kernel serves its API alone.

## 4. Response conventions

### 4.1 Success

- `writeJSON` (serve.go:549-557): `Content-Type: application/json` + encoded body. `writeJSONStatus` for explicit codes; `writeJSONCached` (serve.go:569-587) adds ETag / If-None-Match handling.
- Mutations answer 202 Accepted (`POST /submit`, serve.go:499), 204 No Content (cancel, approve, new, effort, closeRuntime, token bootstrap, device leave), or 302 redirects where a browser is driven (token query cleanup, login).

### 4.2 Failure envelope

Every refusal is a `Reason` (fail.go:18-22):

```json
{"code": "dotted.stable_code", "error": "English fallback sentence", "params": {"k": "v"}}
```

- `refuse(status, code, message, params)` (fail.go:26-30). Codes are dotted and STABLE — renaming one is a breaking change, same as renaming a route (fail.go:24-25). `busy(...)` = 409 Conflict (fail.go:36-38).
- `writeErr(w, fallback, err)` (fail.go:84-107) maps errors, in order: a `coded` refusal keeps its own status+code; memory read failures -> 500 `memory.directory_unreadable` / `memory.fact_unreadable`; unparsable config -> 409 `config.unparsed` with params `{path, line, excerpt, repair, recovered}`; `AnswersMismatchError` -> 409 `model.decision_only` / `model.not_decision_source`; anything else -> `http.Error` (text/plain) with the fallback status.
- `saveFailed` -> refusal carrying `{detail}` params (fail.go:135-143); `rebuildFailed` -> 409 `runtime.saved_while_running` when a turn holds the runtime, else 409 `runtime.rebuild_failed` (fail.go:183-189).
- Exceptions that are NOT Reasons: `deny` 401 plain text (§2.1), rate-limited login 429 plain text (§2.3), `http.Error` fallbacks.

### 4.3 Shared codes (refuse_common.go:15-35)

`request.bad_body`, `request.missing_field`, `request.not_found`, `project.unknown`, `busy.session_in_use`, `request.bad_value`, `model.decision_only`, `model.not_decision_source`, `busy.session_running`, `session.bad_name`, `session.bad_path`, `session.path_unreadable`, `session.unknown`, `session.replay_limit`, `session.outside_dir`, `busy.session_active`, `busy.switch_model`, `session.open_failed`, `session.title_save_failed`.

Helpers: `badBody` 400 (the parse error itself is never shown — it names Go types and offsets, refuse_common.go:37-42); `missingField` 400 `{field}`; `notFound` 404 `{kind, name}` — only for things that could exist; an invalid name is a 400 (refuse_common.go:50-57); `unknownProject` 400 `{root}`; `sessionInUse` 409 `{detail}`; `badValue` 400 `{field, allowed}`; `sessionBusy` 409; `decodeBody` (refuse_common.go:94-103) caps bodies at 64KiB (`http.MaxBytesReader`) and REJECTS unknown JSON fields (`DisallowUnknownFields`).

### 4.4 Request logging

`logMiddleware` (requestlog.go:27-45): one INFO line per request (method, path, status, duration). Routes wrapped in `routine()` (timer/per-frame routes, requestlog.go:18-25) log successful (<400) requests at Debug; failures keep their INFO line.

### 4.5 Wire log and trajectory

The wire log mirrors qualifying broadcast frames to the session's wire log file (wirelog.go:139-204): 8MiB cap (`wireLogMaxBytes`), kind whitelist (wirelog.go:25-52: turn_started, turn_done, message, tool_dispatch, tool_result, usage, stream_attempt, guardian_assessment, approval_request, ask_request, compaction_started, compaction_done, retrying, steer, context_maintenance, completion_summary, notice, graph_delta, todo_progress, progress_watch, workspace_lease, extension_surface, extension_status) and a deliberate skip list (wirelog.go:58-77: reasoning, text, phase, tool_progress, compaction_progress, mcp_surface_ready, workspace_changed, turn_phase, inbox_changed, adjudications_changed, rewind_changed, browser_tabs_changed, ask_engaged). Past the cap frames are DROPPED, never truncated mid-file, and a witness meta `{truncated, surfacesDropped}` is written on the refusal (wirelog.go:206-230). `GET /trajectory` answers `{availability: "complete"|"truncated"|"not_recorded", events: [...]}` — availability is read from the witness, never inferred from file size; an unreadable log or witness is 500 `trajectory.unreadable`, never an empty array (wirelog.go:255-290).

## 5. SSE transport framing (events endpoints)

Event KIND taxonomy is the SSE contract, frozen separately; this section freezes the transport.

- `GET /events` (eventstream.go:293-298, 327-367): SSE — `Content-Type: text/event-stream`, `Cache-Control: no-cache`, `Connection: keep-alive`; opens with `: connected`; each frame is `id: <seq>` ONLY when numbered (unnumbered frames get no id so a reconnect never asks for a delta replay) + `data: <json>\n\n`, flushed per frame. Keepalive every 15s (`sseKeepaliveInterval`, eventstream.go:288): comment `: ping` PLUS a real frame `data: {"kind":"stream_watermark","seq":N}` — a comment never reaches onmessage, and the watermark is how a client learns of a lost last frame within one interval.
- Resume: `Last-Event-ID` header or `lastEventId` query parameter (eventstream.go:88-98); `attaching` = neither present (eventstream.go:65-67). Subscription modes (eventstream.go:304-325): first+attached -> SubscribeAttached; first -> SubscribeInitialNotices; attached -> SubscribeResume(after); else SubscribeFrom(after). Subscribe and replay are one handoff serialized with prompt producers. Replay budget 8MiB (`replayBudget`), live queue budget 4MiB (`queueBudget`), eventstream.go:100-108.
- `GET /events/replay` (eventstream.go:27-42, 69-83): JSON `{frames: [...], complete: bool, watermark: N}` — frames default to `[]`, never null.
- Event mux (`GET /events/mux` family, hub_mux.go:266-271): one SSE connection carries several panes. Opens with a `mux_open {conn}` frame; `PUT /events/mux/{conn}/panes/{id}` attach, `DELETE` detach; `GET /events/mux/{conn}/metrics` -> `{conn, panes: [{pane, sent, lastSeq, watermark, behind}]}`; unknown conn -> 404 `events.mux.unknown_conn` (hub_mux.go:257-264).

## 6. Frontend ROUTES cross-check (vite dev proxy)

`desktop/frontend-next/vite.config.ts:12-32` proxies 121 path prefixes to `REASONIX_SERVE` on the dev server (port 5273), rewriting the Origin header so the hop looks same-origin (vite.config.ts:65-83). Cross-check against §1 (2026-11-15):

- 116 prefixes map to at least one registered kernel route.
- `/rt` is the runtime prefix registered dynamically as `/rt/{id}/` (hub.go:550) — not a literal row in the matrix.
- `/pair` is answered inside the device gate, not on any mux (device_gate.go:77).
- `/fork`, `/decision-models`, `/receipt-settings` have NO kernel route and NO frontend caller — stale proxy prefixes. `/fork` survives only in the tree-write list (hub_sync.go:86). A request to one falls through to `routeDefault` and gets the first local runtime's answer.

## 7. Core endpoint shapes

- `GET /status` (settings.go:193-274): JSON object with keys `agentCompositionLocked`, `label`, `running` (turn in flight), `busy` (refuses a new turn), `plan`, `autoApproveTools`, `bypass`, `toolApprovalMode`, `approvalDefault`, `preset`, `goal`, `goalStatus`, `cwd`, `workspaceRoot`, `sessionPath`, `used`, `window`, `cacheHit`, `cacheMiss`; conditional keys (present only when they have a value): `agentComposition`, `goalRuntime`, `decisions`, `planPhase` (absent = outside the plan workflow), `lastUsage`, `effort`, `modelRef`, `providerDisplayName`, `vision`, `visionDeclared`, `modes`, `sessionCostQuote`, `sessionUsage`, `jobs`, `endedJobs`, `tariff`.
- `POST /submit` (serve.go:417-500): body `{input, invocations[], format, refuseUnknownSlash, localShell}` (chipLine embedded); `format` must be `""` or `"json_object"` else 400 `request.bad_value`; empty input and no invocations -> 400 `request.missing_field`; a turn in flight -> 409 `busy.session_running`; `!`-prefixed shell input over a socket -> 403 `shell.unavailable_over_http`; success -> 202 Accepted. Turn admission is serialized against controller rebuilds by `bindMu`; the session path is pinned on first submit (EnsureSessionPath) and the lease follows it — a freshly opened pane must not 409 its own first turn.
- `POST /cancel` -> 204 (serve.go:502-505). `POST /approve` `{id, allow, session, persist}` -> 204 (serve.go:507-520). `POST /new` -> 204 (serve.go:522-538; bindMu-serialized; lease rebound to the fresh path; held elsewhere -> 409 `busy.session_in_use`).
- `GET /sessions` (sessions_list.go:15-65): array of `{name, path, title?, turns?, current?, unread?, modified}`; subagent transcripts and hidden recovery copies excluded; an empty or unreadable directory answers `[]`, never an error. Titles come from sidecars, never transcript decoding.
- `GET /todos` (todos.go:11-25): array of `{content, status, activeForm?, level?}` — the canonical task list, NOT derivable from the transcript (advances write no todo_write).
- `GET /balance` (balance.go:32-58): `{display, available, stale, fetchedAt, lines?}` with `lines[] = {currency, total, granted?}` — currencies are stacked, never summed; 204 when the provider has no wallet; failures carry `wallet.unauthorized` | `wallet.unreadable` | `wallet.format_not_declared` | `wallet.unreachable` (balance.go:71-82).
- `GET /context` (serve.go:540-547): ETag-cached window gauge.

## 8. Divergences and open notes

1. Stale vite proxy prefixes `/fork`, `/decision-models`, `/receipt-settings` (§6) — no kernel action; the frontend is preserved as-is. Recorded so the Rust route matrix is not "completed" against them.
2. `hub_sync.go:86` still lists `/fork` as a tree-write path although no route registers it — kept for parity (harmless: the path never matches).
3. The 420-row matrix counts only literal `mux.HandleFunc("METHOD /path", ...)` registrations. Routes answered inside middleware — `/login`, `/auth/token`, `/pair`, device self routes — are contract routes without matrix rows; they are frozen in §2.
4. `hostOnlyRoutes` (§2.8) is a SEPARATE list from the hostMux mounting: it is enforced inside the device gate, by the remote-pane proxy, and in handlers. The Rust port must keep all three enforcement points.
5. SSE event kinds, broadcaster semantics, and the full event taxonomy are frozen in the SSE contract (separate document), not here.

## 9. Proposed ledger rows

| ID | Behavior | Section |
|---|---|---|
| ROUTE-001 | Route matrix: 420 registrations, methods, paths | §1 |
| ROUTE-002 | Server/Hub/Runtime topology and middleware order | §0 |
| ROUTE-003 | /rt/{id} prefix, routeRuntime, routeDefault fallback | §0 |
| ROUTE-004 | hostOnly split: devices reach mux, not hostMux | §0, §2.8 |
| ROUTE-005 | watchTreeWrites tree-change trigger set | §0 |
| ROUTE-006 | withPage file-first serving + content-type map | §0, §3 |
| ROUTE-007 | InProcessClient + shell.unavailable_over_http | §0, §7 |
| ROUTE-008 | Auth modes none/token/password/invalid + deny 401 text | §2.1 |
| ROUTE-009 | Token mode: cookie/bearer/query + redirect cleanup | §2.2 |
| ROUTE-010 | /auth/token bootstrap (POST JSON, constant-time, 204) | §2.2 |
| ROUTE-011 | Password mode: login form, bcrypt 12, HMAC session 30d, rate limit | §2.3 |
| ROUTE-012 | hostGuard DNS-rebound allowlist (421 serve.host_rejected) | §2.4 |
| ROUTE-013 | mutationGate launch token (403 auth.launch_token_required) | §2.5 |
| ROUTE-014 | Loopback gate: tcp4 127.0.0.1:0, host/origin/token checks | §2.6 |
| ROUTE-015 | Device gate: /pair, self routes, public paths, 401 | §2.7 |
| ROUTE-016 | hostOnlyRoutes list + triple enforcement | §2.8 |
| ROUTE-017 | csrfGuard content types + binary upload exceptions | §2.9 |
| ROUTE-018 | No CORS / same-origin policy | §2.9 |
| ROUTE-019 | OpenBindRefusal + PlainHTTPAuthWarning | §2.10 |
| ROUTE-020 | Binding: Run/RunGraceful timeouts, 10s drain | §3 |
| ROUTE-021 | FindPage resolution order | §3 |
| ROUTE-022 | Reason envelope {code, error, params} | §4.2 |
| ROUTE-023 | writeErr mapping: coded/memory/config/answers | §4.2 |
| ROUTE-024 | Shared refusal codes + decodeBody 64KiB/unknown-fields | §4.3 |
| ROUTE-025 | Request logging levels (routine -> Debug) | §4.4 |
| ROUTE-026 | Wire log kinds, 8MiB cap, witness, /trajectory availability | §4.5 |
| ROUTE-027 | SSE framing: id/data/keepalive/watermark | §5 |
| ROUTE-028 | Last-Event-ID resume + subscription modes | §5 |
| ROUTE-029 | /events/replay shape + events mux | §5 |
| ROUTE-030 | /status shape (fixed + conditional keys) | §7 |
| ROUTE-031 | /submit contract (202, busy, format, shell refusal) | §7 |
| ROUTE-032 | /sessions, /todos, /balance, /context, /new shapes | §7 |
| ROUTE-033 | vite ROUTES cross-check + stale prefixes | §6 |
