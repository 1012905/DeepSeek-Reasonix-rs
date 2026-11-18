# MCP Contract — Frozen Inventory (M1)

> **STATUS: FROZEN** — extracted from source on 2026-11-15.
> Oracle: reference/DeepSeek-Reasonix-studio/internal/ext/plugin/** (read-only, pinned with file:line).
> Target crate: reasonix-ext (MCP client portion).
> AGENTS.md §5.1 correction: that table maps the MCP contract to docs/EXTENSION_PROTOCOL.md, but that
> document is the extension sidecar protocol (reasonix.extension.v2) and belongs to the Extension v2
> contract area (internal/ext/extension/**). The MCP client contract documented here is pinned to
> internal/ext/plugin/**; AGENTS.md §5.1 will be corrected at finalize time.

## 0. Overview

- Package internal/ext/plugin is Reasonix's MCP **client** (plugin.go:1-7): it connects to external
  MCP servers and adapts their tools to the tool.Tool interface so the agent treats plugin tools and
  built-ins uniformly. Wire protocol is JSON-RPC 2.0 on every transport; only the carrier differs:
  stdio subprocess (NDJSON), Streamable HTTP, or legacy HTTP+SSE.
- The Host aggregates prompts (surfaced as slash commands) and resources (@-references) across
  servers (plugin.go:178-180). Tool names are namespaced mcp__<server>__<tool>.
- ~56 product files. Largest: plugin.go (1658L), oauth.go, transport_stdio.go (703L), lazy.go,
  transport_http.go (470L), cache.go, elicit.go, launcher_lock.go, transport_sse.go (404L),
  transport_reconnect.go (392L).

## 1. Protocol versions and eras

### 1.1 Versions (protocol_version.go:12, plugin.go:35)

- Advertised legacy protocolVersion = "2025-11-25" (plugin.go:35).
- supportedProtocolVersions = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"], newest first;
  initialize offers the first; the server may answer with any of them and the session runs on its
  choice (protocol_version.go:9-12).
- negotiatedVersion(reply): empty reply → oldest revision ("2024-11-05"); reply must be in the list
  else ErrUnsupportedProtocolVersion "MCP server chose a protocol version this client does not
  support" (protocol_version.go:21-29). The spec has the client disconnect rather than guess.
- protocolVersioned interface: setProtocolVersion(string) — Streamable HTTP's MCP-Protocol-Version
  header; a server without the header assumes 2025-03-26, which is not what a newer session agreed
  (protocol_version.go:31-36).

### 1.2 Modern era "2026-07-28" (protocol_modern.go)

- modernProtocolVersion = "2026-07-28": drops the initialize handshake entirely. Every request
  states protocol version, client identity and capabilities in _meta; a server is asked which era it
  speaks with server/discover (protocol_modern.go:16-42).
- _meta keys: io.modelcontextprotocol/protocolVersion, io.modelcontextprotocol/clientInfo
  {name:"reasonix", version:"dev"}, io.modelcontextprotocol/clientCapabilities (protocol_modern.go:25-27, 75-81).
- clientCapabilities: {elicitation: <elicitCapability()>, extensions: {<tasksExtension>: {}}} plus
  roots: {} when the workspace root is non-empty (protocol_modern.go:64-73).
- Era codes (implementation-defined range, protocol_modern.go:29-31): -32020 codeHeaderMismatch,
  -32021 codeMissingClientCapability, -32022 codeUnsupportedProtocolVersion.
- probe (protocol_modern.go:120-155): waits min(3s modernProbeWait, ctx-deadline/2); sends
  server/discover with _meta clientMeta. Result with supportedVersions[] non-empty → pickEra;
  unparseable/empty → legacy fallback. RPC error -32022 with data.supported non-empty → pickEra from
  that list; other RPC errors → legacy; no reply at all → errProbeUnanswered "MCP server did not
  answer server/discover".
- pickEra (protocol_modern.go:159-171): modern version both sides share wins; else any legacy
  supportedProtocolVersions the server lists → "" (legacy handshake); else
  ErrUnsupportedProtocolVersion "(server supports ...)".
- connect (protocol_modern.go:174-198): sse transport always initialize (nothing modern over
  HTTP+SSE). Otherwise probe; errProbeUnanswered → initializeOnFreshChild (stdio: restart the child
  via reconnectingTransport.restart because a server SDK may abort serving on a first non-initialize
  message; protocol_modern.go:204-211); modern → set c.modern.version, read capabilities
  tools/prompts/resources + instructions from the discover result.
- callModern (protocol_modern.go:224-267): sends request with _meta clientMeta; on result
  resultType=="input_required" answers input rounds: maxInputRounds 4 total, maxInputRequests 16 per
  round, maxRequestStateSz 1MiB requestState — over bound → errMCPInputOverBounds; echoes
  inputResponses + requestState back. resultType=="task" && method=="tools/call" → awaitTask (tasks
  extension). answerInputRequests (protocol_modern.go:271-294): "roots/list" → {roots: mcpRoots};
  elicitMethod → elicit() via tool.ElicitorFrom(ctx); anything else → ErrMCPInputRequired "MCP server
  needs input this client does not provide".
- ErrMCPInputRequired, ErrUnsupportedProtocolVersion are package-level sentinels.

## 2. Spec, Host, and Client lifecycle (plugin.go)

### 2.1 Spec (plugin.go:70-165)

Fields: Name, Package (installed plugin package provenance, excluded from fingerprints), Type
(transport: "" / "stdio" / "http" / "streamable-http" / "streamable_http" / "sse"), Command, Args,
Env, URL, Headers, DisabledTools (exact server-local names excluded before schema registration),
DefaultStartupTimeout, StartupTimeout, DefaultCallTimeout, CallTimeout, ToolTimeouts (per raw tool
name), Dir (stdio working dir), WorkspaceRoot (exposed via roots capability), WritableRoots, Stderr
(io.Writer receiving byte-count summaries), LaunchManager (*mcplaunch.Manager), ConfigSource,
Authorized (single runtime authorization result), RequireLaunchApproval, LaunchCheck (func() error
run before every start; refuses a project declaration that changed since spec build), LaunchArgs,
LauncherIdentityArgs (same resolution without the offline/no-install flag), LauncherLocator,
LauncherResolvedVersion, LauncherDigest, ProcessMode, Sandbox (sandbox.Spec; only applied when
confined), StateDir, ProcessStateDir, OAuthHTTPClient, OAuthAllowMissingPKCEMetadata,
OAuthScopeUpgrade, StripRawPrefix (raw tool name prefix removed before namespacing; original kept
for protocol calls), LowPriority.

- MCPProcessMode (plugin.go:38-50): "host" (default — trusted host process, NOT wrapped in the agent
  Bash sandbox, so servers like chrome-devtools-mcp reach the real browser/Keychain/LaunchServices)
  or "confined" (wrapped with sandbox.CommandArgs; reserved for internal managed deployments and
  tests, never auto-selected). ResolvedProcessMode: empty → host.
- defaultCallTimeout = 300s (plugin.go:65) — finite so a hung server cannot block a turn forever.
- ServerAuthorized() = s.Authorized (plugin.go:1306-1308): the single MCP authorization source;
  tools carry no independent trust bit.
- MCPRuntimeSpecMatches (runtime_spec.go:9-11): reflect.DeepEqual over mcpRuntimeSpecIdentity —
  behavioral identity deliberately excluding Stderr writer and LaunchManager pointer
  (plugin.go:863-933). canonicalMCPRuntimeTransport: ""/stdio→"stdio",
  http/streamable-http/streamable_http→"streamable-http", sse→"sse", else lowercased as-is.

### 2.2 Batch startup (plugin.go:250-456)

- StartPolicy {PerPluginTimeout, Concurrency, AbortOnError, SkipPersistence} (plugin.go:253-274).
- defaultStartConcurrency = 8 (plugin.go:280) — process-storm guardrail; defaultStartTimeout = 5s
  (plugin.go:286) for StartAvailable.
- StartAll: Concurrency 8, AbortOnError true — any failure tears down the whole batch.
- StartAvailable: 5s per-plugin timeout, AbortOnError false — failures recorded on the Host.
- Start (plugin.go:343-456): semaphore fan-out; result ordering matches specs (stable /mcp status);
  per-plugin timeout caps initialize+listTools but the stdio child is bound to parent ctx
  (exec.CommandContext) and outlives startup; tools/list runs in phase A; saveHandshakeSchema is a
  detached background write (bgWrites, drained by Close) unless SkipPersistence; prompts/resources
  deferred to StartPhaseB.
- StartPhaseB (plugin.go:510-575): async fetchPrompts/fetchResources per client (only when
  hasPrompts/hasResources advertised) on a session-scoped ctx; each finished surface emits
  event.MCPSurfaceReady ("%s: prompts ready (%d items)"); errors logged+sunk (slog.Warn with
  secrets.DiagnosticError), never fatal.
- advertisedToolsEmptyListRetryDelays = [50ms, 150ms, 300ms] (plugin.go:288-292): a server that
  advertises tools but lists empty gets this bounded startup window (listToolsRawSettled,
  plugin.go:1409-1424).
- Host.Close (plugin.go:459-491): cancel deferred startup cancels with ErrHostClosed, wait
  deferredWG, snapshot+close clients, closeServerProxies, drain bgWrites. Idempotent via closed flag.

### 2.3 Live add/remove and single-flight spawn (plugin.go:769-1176)

- Add(ctx, spec) / AddWithLifecycle(lifeCtx, callCtx, spec): connect live, discover tools, prompts
  and resources when advertised; appends to Host; errors if name taken
  (ErrServerAlreadyConnected "plugin server already connected").
- EnsureConnected / EnsureConnectedWithLifecycle: single-flight shared handshake per server
  (spawningMu + spawning map; spawnAttempt {server, done chan, tools, err}); concurrent callers
  share one initialize/tools-list; ErrSpawningInFlight "server spawn already in progress".
- Remove(name) (plugin.go:1139-1176): cancels deferred spawns with ErrServerRemoved, bumps
  deferredGenerations, removes client, closes it (kills subprocess), returns prefix
  "mcp__"+normalizeName(name)+"__" and found. Host keeps pointer stability for the session
  (NewHost always constructs one, plugin.go:716).
- Deferred/lazy spawns carry a generation; ErrDeferredSpawnCancelled "deferred MCP spawn cancelled"
  when the generation is stale. ErrHostClosed = hostDiagnosticCause("the MCP host shut down (session
  ended or runtime rebuilt)"); ErrServerRemoved = hostDiagnosticCause("this MCP server was removed,
  disabled, or reconnected") (plugin.go:1186-1195).
- ToolsFor/ToolsForSpec: cached tools (toolsListed) or fresh listTools; ToolsForSpec refuses a
  same-name client from another identity ("connected MCP server %q identity does not match the
  current runtime configuration"). MCPToolMatchesSpec: *remoteTool / *lazyTool only, fail closed.
- Host.ReadResource(ctx, server, uri) — the @server:uri resolution path; uri need not be listed.

### 2.4 start() and launch authorization (plugin.go:1197-1326)

- start(lifeCtx, callCtx, spec): applyStoredLauncherLock → resolveProjectLaunchAuthorization →
  checkDeclaredLaunch → newTransport → wrap in newReconnectingTransport(lifeCtx, t,
  ResolvedStartupTimeout, redial, handshakeOn) → connect. Failure phases recorded as
  newStartupFailure("launch"/"authorization"/"initialize", started, startupStderr, err).
- resolveProjectLaunchAuthorization: skips installed/host-session servers (their install IS the
  authorization); only repository-declared servers (RequireLaunchApproval) need an exact
  projectLaunchIdentityDigest before startup; applyEstablishedLaunchGrant → LaunchManager.
  LaunchAuthorized(name, source, digest) else launchApprovalError{server, changed}.
- AuthorizeSpecLaunch / AuthorizeProjectSpecLaunch: record durable consent without starting;
  AuthorizeProjectSpecLaunch also resolves+locks mutable launchers (preparePersistentLauncher) and
  stores the lock before the grant.
- ResolveStoredAuthorization: applies an existing grant without process/network; fails closed
  (returns original spec on error).
- newTransport (plugin.go:1312-1326): stdio (default, "" → stdio), http/streamable-http/
  streamable_http → HTTP, sse → SSE; unknown → error "unknown transport type %q (want
  stdio|http|sse)".

## 3. Tool naming, schema, and the remote adapter (plugin.go:1338-1658)

- mcpTool wire shape (plugin.go:1338-1348): {name, title?, description, inputSchema, outputSchema?,
  annotations?}. hints() reads annotations.readOnlyHint / annotations.destructiveHint; destructive
  stays destructive even when another hint claims read-only.
- toolName(server, raw) = boundedToolName(ToolPrefix(server), normalizeName(raw)) →
  "mcp__<server>__<tool>". ToolPrefix(server) = "mcp__"+normalizeName(server)+"__".
- normalizeName (plugin.go:1493-1511): replace [^a-zA-Z0-9_-]+ with "_", trim "_" from both ends,
  empty → "unnamed"; if the raw string changed, append "_"+shortNameHash(raw) (fnv32a, 6 hex chars).
  Every permission/hook/audit surface must build names through ModelToolName (plugin.go:1484-1491).
- MCPConnectPermissionName(server) = "mcp_connect__"+normalizeName(server) — intentionally outside
  the mcp__ tool namespace so permission rules cannot collide (plugin.go:1476-1482).
- normalizeAndValidateToolSchema: canonicalizeSchema (provider.CanonicalizeSchema — stable bytes for
  cache fingerprints) + provider.ValidateToolSchema; schemaValidationError clips to 512 runes +
  "..." with prefix "invalid input schema: " (plugin.go:1381-1397).
- validateMCPToolNames (plugin.go:1426-1439): empty tool name → ErrAmbiguousToolCatalog "tools/list
  returned an empty tool name"; two raw names colliding on one model-visible name →
  ErrAmbiguousToolCatalog naming both raw names and the shared name.
- listTools caches adapters (toolsListed/toolAdapters); listToolsRaw pages tools/list via
  listAllPagesMeta and records the tool-list TTL meta (noteToolListTTL).
- remoteTool adapter (plugin.go:1544-1658): fields name (namespaced), rawName (for tools/call),
  visibleName (after StripRawPrefix), desc, schema, outputSchema, declaredReadOnly, readOnly
  (effective), destructive, paramHeaders (modern HTTP only). Schema() defaults to
  {"type":"object"} when empty. ExecuteWithImages: compositionContractError gate; parses args;
  reader-intent linearizable check under toolsMu — HasReaderExecutionIntent requires
  authorized+readOnly+non-destructive else error "MCP server %q changed the authorization or
  security metadata for tool %q; the call was blocked before dispatch — refresh the server from a
  parent session before retrying"; HasNonDestructiveMCPExecutionIntent requires authorized +
  non-destructive (missing readOnlyHint does NOT block) else "...retry so Reasonix can re-apply the
  current Planner MCP safety boundary". Dispatches tools/call {name: rawName, arguments: argMap}
  with withParamHeaders; error → client.scopeRefusal(err); result → parseToolResult (images
  forwarded via tool.ImageTool).
- JSON-RPC envelope (plugin.go:1528-1540): rpcRequest {jsonrpc:"2.0", id int (omitted for
  notifications), method, params}; rpcResponse {jsonrpc, id, result, error}.
- rpcError (protocol_errors.go): {code int, message string, data?}; Error() = "rpc error %d
  (message omitted; %d bytes)" — message content deliberately never rendered; DiagnosticFacts()
  same.
- Progress (protocol_inbound.go:19-98): progressRouter maps progressToken (string | float64 |
  json.Number; other types ignored) → tool.ProgressFunc. formatMCPProgress: "label (p/t)\n" |
  "label (p)\n" | "label\n"; empty message → "MCP progress".
- Server requests (protocol_inbound.go:158-174): serverRequestReply answers "ping" → {} and
  "roots/list" → {roots}; anything else → -32601 "Method not found". mcpRoots(workspaceRoot) builds
  a single file:// URL (Windows volume name → leading slash; UNC → host component; name = base dir)
  (protocol_inbound.go:100-136).
- Inbound frame decode: inboundMessage {jsonrpc, id, method, params}; isNotificationID = empty id or
  literal null (protocol_inbound.go:138-156).

## 4. stdio transport (transport_stdio.go, 703L)

- NDJSON JSON-RPC 2.0 over subprocess stdin/stdout (MCP stdio convention: one JSON message per line,
  no embedded newlines — json.Marshal output never contains literal newlines, transport_stdio.go:584-595).
- Spawn (newStdioTransport, transport_stdio.go:61-152): CodeGraph-named servers must acquire a
  bounded instance slot first (acquireCodeGraphSlot); env = mergeEnv(secrets.ProcessEnv(), spec.Env);
  resolveStdioExecutable; prepareMCPPrivateState; exec.CommandContext(lifeCtx); proc.HideWindow;
  proc.LowPriority when LowPriority; cmd.Dir = stdioWorkingDir(s); stderr → tailBuffer{limit:16KiB}
  (+ io.MultiWriter with omittedStderrWriter when spec.Stderr set — byte-count summaries only);
  proc.StartTracked (job reaps detached grandchildren on close).
- prepareMCPPrivateStateForOS (transport_stdio.go:158-211): StateDir (or ProcessStateDir for
  composition processes, which must be a real non-symlink dir, 0700) → private XDG_CACHE_HOME /
  XDG_STATE_HOME / npm_config_cache / UV_CACHE_DIR / BUN_INSTALL_CACHE_DIR; on non-Windows also
  TMP/TEMP/TMPDIR under the private root. Windows keeps host TEMP: nesting below the workspace-
  scoped state path can exceed the 108-byte Unix-domain-socket limit (MATLAB case). Sandbox
  WriteRoots/Pins/AppContainerWriteRoots gain the private root.
- Executable resolution (transport_stdio.go:268-305): PATH always enriched with the user's login
  shell PATH first (cachedShellPATH memoizes one probe per process: up to 3 login-shell invocations
  (-l -i -c / -l -c / -c) printing marker __REASONIX_PATH__=, 2s timeout each, runs on non-Windows
  only; empty results cached except cancelled probes). Command with a path separator resolves
  relative to stdioWorkingDir then Abs. Bare command: LookPath in env; on Windows a fallback PATH is
  merged (candidates: ProgramFiles/nodejs, ProgramFiles(x86)/nodejs, LOCALAPPDATA/Programs/nodejs,
  APPDATA/npm, LOCALAPPDATA/Microsoft/WindowsApps, USERPROFILE/scoop/shims, USERPROFILE/.bun/bin,
  USERPROFILE/.cargo/bin, ChocolateyInstall/bin — existing dirs only). Not found →
  commandMissingError.
- mergeEnv/setEnvValue/envValue (transport_stdio.go:465-508): overrides applied in sorted key order;
  env key comparison case-insensitive on Windows (envKeyEqual), first spelling wins position,
  duplicates collapse.
- stdioWorkingDir (transport_stdio.go:310-318): Dir if set; else WorkspaceRoot only when
  RequireLaunchApproval (repository-declared); else "" (inherit reasonix cwd).
- Reader (transport_stdio_inbound.go:20-117): readLoop owns stdout; server-request replies go
  through a bounded reply queue (stdioReplyQueueBound 16) drained by replyLoop — never written
  directly from the read loop, so a jamed stdin pipe cannot deadlock both pipes; overflow drops the
  reply (server's own timeout applies). Routing: notifications/progress → progressRouter; other
  notifications → notificationRouter; server requests → serverRequestReply (elicitMethod handled
  async off the read loop via elicitRouter.claim); responses demux by id (buffered chan 1,
  abandoned callers never block).
- call (transport_stdio.go:540-578): fail-fast on latched readErr with markGone(...) — the request
  was never written, so a replacement connection may carry it; write under writeMu; ctx cancel →
  cancelInFlight and return ctx.Err(); closed response channel → stdioReadFailure with stderr tail.
- close (transport_stdio.go:653-677): stdin Close → graceful wait 750ms (gracefulCloseWaitBudget)
  → job.Finish if exited; else job.Kill(cmd) + wait ≤ 5s (closeWaitBudget). Removes the private
  state dir. Budgets guarantee a wedged server cannot stall teardown.
- tailBuffer (transport_stdio.go:679-703): 16KiB tail; String() decodes in the server's own console
  code page (fileenc.DecodeOutput) — cmd.exe on Chinese Windows reports in the console code page.
- withStderr wraps errors with subprocessOutputError{cause, bytes} carrying the stderr byte count.

## 5. Streamable HTTP transport (transport_http.go, 470L)

- Every JSON-RPC message is one POST to the server URL (httpTransport, transport_http.go:35-48).
  Accept: "application/json, text/event-stream"; Content-Type: application/json. Response is either
  application/json (single response) or text/event-stream (SSE stream carrying the response plus
  server notifications). Mcp-Session-Id captured from any response header and echoed on every
  subsequent request; MCP-Protocol-Version header set once negotiated (setProtocolVersion).
- maxHTTPBody = 16 MiB caps JSON/SSE response reads (transport_http.go:23).
- Redirect policy (transport_http.go:80-110): same-origin only (scheme/host/effective-port
  case-insensitive compare); cross-origin redirect → http.ErrUseLastResponse — configured
  credentials are never sent to another origin; the 3xx surfaces as a status failure.
- OAuth wiring (transport_http.go:57-73, 242-284): when no static auth config exists
  (mcpdiag.HasAuthConfig(headers, env, url)), an mcpOAuthClient is loaded from StateDir; a stored
  token for a different canonical resource → error "stored OAuth token belongs to a different MCP
  resource; clear authentication and authorize this endpoint". doOAuth: sets Authorization from the
  OAuth client (unless a static header exists); on 401 with usedOAuth + canRefresh →
  authorizationHeaderAfterReject(sent) → retry exactly once (refreshed=true).
- call (transport_http.go:112-154): non-2xx → statusFailure with httpStatusError{Status, BodyBytes,
  RPC} (bodyRPCError extracts a typed JSON-RPC error from ≤4096 read bytes; operationOf names
  "tools/call:<name>" for the upgrade count); 404 with a held session id → clearSession +
  httpSessionExpiredError "http %d: MCP session expired (response body omitted; read %d bytes)".
  text/event-stream → readSSEResponse; else decodeRPCResult. Legacy cancellation (cancelInFlight)
  only when the request was NOT modern (a modern request is cancelled by closing its stream).
- readSSEResponse (transport_http.go:335-402): SSE scan bounded by maxHTTPBody; consecutive data:
  lines joined with "\n"; event boundary = blank line; progress/notifications dispatched; server
  requests answered via a SEPARATE Streamable HTTP POST (replyServerRequest) while the response
  stream stays open; the response matching the call id returns; stream end without it → "SSE stream
  ended without a response to id %d".
- close (transport_http.go:177-192): DELETE the URL with Mcp-Session-Id (best effort — 405 leaves
  the session to expire), then CloseIdleConnections.
- httpStatusError.Error() = "http %d (response body omitted; read %d bytes)" (transport_http.go:314-318).

### 5.1 Modern request headers (modern_headers.go, 207L)

- A 2026-07-28 request over Streamable HTTP mirrors body parts into headers for gateways
  (modern_headers.go:16-19): MCP-Protocol-Version, Mcp-Method, and Mcp-Name for tools/call +
  prompts/get (name), tasks/get|update|cancel (taskId), resources/read (uri) — values pass through
  encodeHeaderValue.
- encodeHeaderValue (modern_headers.go:160-172): plain when non-empty, no surrounding space, visible
  ASCII only and not already the base64 sentinel form; else "=?base64?<std>...?=" sentinel.
- Tool argument mirroring: x-mcp-header annotations in a tool's input schema map argument values at
  the annotated property path to Mcp-Param-{Name} headers (withParamHeaders, only for tools/call).
  toolParamHeaders (modern_headers.go:35-91) validates: annotation only on plain properties paths
  (reachable through "properties" objects); valid RFC 9110 field-name token; unique
  case-insensitive; type string|integer|boolean — violations return errBadParamHeader "invalid
  x-mcp-header annotation" and the tool is left out of the catalog rather than guessed at.
- headerScalar (modern_headers.go:138-156): string/bool pass; float64 only when integral and
  |x| ≤ maxSafeInteger (2^53-1); json.Number only when it parses as an integer. Absent/null/other →
  header omitted.
- modernRequestHeaders only fires when params._meta carries the modern protocol version; requests
  without modern _meta keep legacy behavior (no extra headers).

## 6. Legacy HTTP+SSE transport (transport_sse.go, 404L)

- Long-lived GET (Accept: text/event-stream) kept open; the server announces the POST endpoint via
  an "event: endpoint" frame (resolved against the response URL; same-origin enforced — cross-origin
  → errSSECrossOrigin hostDiagnosticCause "server announced a cross-origin endpoint"); all JSON-RPC
  messages POST to that endpoint; responses and server messages return on the GET stream
  (transport_sse.go:22-49, 174-188).
- Content type of the GET must be text/event-stream (else errSSEContentType "SSE response has an
  unexpected content type"); scanner buffer 64KiB → maxHTTPBody; event: name defaults to "message".
- Server-request replies are serialized through one bounded worker (sseReplyQueueBound 16; overflow
  dropped so the reader keeps routing); each reply POST bounded by replyTimeout = CallTimeout ||
  DefaultCallTimeout || defaultCallTimeout (transport_sse.go:244-261).
- call (transport_sse.go:263-310): waitEndpoint first (endpointReady; endpointErr or
  errSSEEndpointMissing "SSE stream ended before announcing an endpoint"); readErr latch fail-fast
  with markGone; response demux by id via pending map; ctx cancel → cancelInFlight.
- fail/failPending latch readErr and close every pending channel (quiet-death detection).
- close: cancel context + CloseIdleConnections, once (closeOnce).
- Errors: httpStatusError shared with HTTP transport; GET failures carry secrets.RedactEndpoint(url).

## 7. Reconnecting wrapper (transport_reconnect.go, 392L)

- reconnectingTransport is the stable handle consumers hold while the connection rolls underneath:
  dial + handshake bring a replacement to a usable MCP session before any caller reaches it
  (transport_reconnect.go:39-56).
- errTransportGone "mcp connection ended" marks only deaths BEFORE the request was written (safe for
  a replacement connection to carry the call without repeating a side effect); a death mid-request is
  deliberately unmarked — the server may have run the call (transport_reconnect.go:21-37).
- call: dispatch once; on errTransportGone → renew (concurrent callers share one dial via the
  dialing channel; a caller whose ctx ends leaves without disturbing the dial) → dispatch again on
  the replacement. Exactly one replacement per call — a server dying on every spawn costs one
  attempt per call, not a loop (transport_reconnect.go:93-107).
- notify: forwarded without reconnecting — a lost fire-and-forget is not worth a spawn.
- restart (transport_reconnect.go:208-258): replaces the connection with a fresh UN-handshaken child
  (used by initializeOnFreshChild after an unanswered server/discover); the old child shuts down
  first so a port/file-lock holder never meets its predecessor; dial bounded by ctx and the startup
  budget (default 5s).
- Registrations outlive connections: registerProgress tokens and registerNotification handlers are
  kept at the wrapper and rebound onto each replacement (rebindProgressLocked /
  rebindNotificationsLocked / rebindNotices). registerElicitCall reaches the live connection only.
- close: closes active, waits ≤ closeWaitBudget (5s) for an in-flight dial; a dial that completes
  after close closes its own child.
- startupStderr reaches the active transport when it implements startupDiagnosticTransport.

## 8. Extraction provenance (checklist)

- [DONE -> §9, §10] cache.go + client_session.go: schema cache, saveHandshakeSchema, cache fingerprints.
- [DONE -> §11, §12] lazy.go + deferred_spawn.go + startup.go: lazy toolset, on-demand spawn.
- [DONE -> §13, §14, §15] oauth.go + oauth_refresh.go + oauth_state.go + oauth_helpers.go + signin_consent.go +
  auth_required.go: OAuth client, PKCE, refresh semantics, resource binding.
- [DONE -> §19, §20, §21, §22] toolpin.go + toolpin_follow.go + registration_scope.go + scope_challenge.go +
  security.go: pinned tools, registration scoping, security reconciliation.
- [DONE -> §23] tool_list_changed.go + tool_list_ttl.go + tool_catalog.go + toolname.go +
  disabled_tools.go + list_pages.go + notification_router.go + known_overrides.go + codegraph_limit.go:
  catalog refresh, notifications/tools/list_changed, paging, naming, overrides.
- [DONE -> §24] elicit.go + elicit_url.go: elicitation/create handling (elicitMethod, elicitCapability).
- [DONE -> §25] prompts.go + resources.go + resource_read.go + resource_template.go: prompts/list,
  resources/list, resources/read, resource templates.
- [DONE -> §26] toolresult.go + toolresult_image.go: parseToolResult content mapping, image forwarding.
- [DONE -> §27, §28] task_driver.go + task_job.go + cancellation.go: tasks extension (awaitTask),
  cancelInFlight.
- [DONE -> §16, §29] launcher_lock.go + install.go: launcher identity locks, install path.
- [DONE -> §17, §18, §30, §31] failures.go + host_queries.go + host_meta.go + diagnostic.go + status.go + proxy.go +
  notification_router.go + known_overrides.go + codegraph_limit.go + composition_contract.go +
  composition_process.go: failure records, host queries, status, serverProxy, notification routing,
  CodeGraph slot, composition guards.
- [DONE -> §10] initialize() handshake details (legacy initialize/initialized flow captured in §10 from client_session.go; plugin.go connect() in §2).
- [DONE -> §32] docs/MCP_CREDENTIAL_PROJECTION.md: credential projection/redaction contract.
- [DONE -> §33, §34] divergences + proposed MCP-001..MCP-038 ledger rows.

## 9. Handshake schema cache (cache.go)

Purpose: the handshake (initialize + tools/list, optional prompts/resources) costs hundreds of ms to seconds per server on cold start, so its result is persisted under the user cache dir keyed by load-bearing, NON-secret Spec fields. Caching is purely an optimisation: any miss (missing dir, bad JSON, key mismatch) silently degrades to a fresh handshake (cache.go:1-10).

- cacheVersion = 2 (cache.go:58); a file with a different version is a miss, so a stale layout never crashes a reader.
- CachedSchema JSON (cache.go:64): {version, spec_hash (historical JSON key for CacheKey, kept so older versions reuse the cache), capabilities: {tools,prompts,resources: bool}, instructions (omitempty — server self-description), tools: [CachedTool], last_validated (time, UTC)}.
- CachedTool JSON (cache.go:82): {name (raw, server-local), description, schema (raw JSON), output_schema (omitempty), read_only, destructive (omitempty)}.
- SchemaCacheKey(spec) (cache.go:96): sha256 hex over tagged fields name, type, command, url (normalizeIdentityURL), dir, each arg, each disabled tool name, sorted env KEY names, sorted header KEY names. Env/header VALUES are excluded (credential rotation must not leak through a stable digest or force a trust re-review) while their key names stay identity-bearing. Field framing: key + 0x00 + value + 0x01 so boundaries cannot collide (writeField, cache.go:386); maps hash as key+"."+name -> "present" (writeKeys, cache.go:395); empty map -> empty field.
- LoadCachedSchema(name, expectedKey) (cache.go:134): returns the schema only if it exists, parses, version matches, and CacheKey == expectedKey; any error -> (nil, false), silently.
- LoadCachedSchemaForSpec (cache.go:147): filters to enabled tools (spec.EnabledCachedTools); invalid model-visible names -> miss (cachedToolNamesUsable warns "plugin cache: catalog rejected"); a legacy-URL-keyed entry (legacySchemaCacheKey, cache.go:104 — pre-credential-aware URL normalization) is upgraded in place: the upgraded key is claimed only after SaveCachedSchema reached disk, else the hit keeps the legacy key the file holds.
- LoadCachedSchemaAny (cache.go:190): returns the schema regardless of key match plus keyOK; search order specCachePath(name, key) -> cachePath(name) -> newestSiblingCachePath(name) (keyOK false). Catalog building uses it to surface stale candidates; EXECUTION paths must use LoadCachedSchema, which refuses key mismatches.
- SaveCachedSchema (cache.go:237): atomic write 0600 (fileutil.AtomicWriteFile), Version set, LastValidated zero -> now UTC, MarshalIndent 2-space, then pruneSpecCaches.
- Path layout: <config.CacheDir()>/mcp/<slug(name)>.<tag>.json (per-spec, cache.go:272) and <slug(name)>.json (name-level, cache.go:342); tag = lowercase hex-only chars of the key, first 16.
- maxSpecCachesPerName = 8 (cache.go:266): per-name per-spec entries pruned oldest-first past 8, never the just-written one.
- slug(name) (cache.go:370): lowercase, [^a-z0-9_-]+ -> "-", trim "-", empty -> "_"; if sanitization changed the name OR the stem is a Windows reserved device name (con prn aux nul com1-9 lpt1-9) -> append "-" + 12-hex sha256[:6], so confusable names cannot make one server consume another's cache, stats, or private state dir.
- Client.saveHandshakeSchema (cache.go:408): SaveCachedSchema{CacheKey: SchemaCacheKey(spec), Capabilities: advertisedCapabilities(), Instructions: c.instructions, Tools: cacheableToolsOf(tools)}; cacheableToolsOf keeps only *remoteTool, copying securitySnapshot() readOnly/destructive.
- Host.cachedCapabilities(server, haveTools) (cache.go:425): the live client's {tools,prompts,resources} or {tools: haveTools} when not connected.

## 10. Client session: dispatch, timeouts, initialize handshake (client_session.go)

- Dispatch (client_session.go:21): modern era (c.modern.version != "") -> callModern (§1); legacy -> callOnce. tools/call on an elicitTransport registers the elicitation-call window (registerElicitCall).
- Progress tokens (withProgress, client_session.go:53): only tools/call, only when ctx carries a tool.Progress sink and the transport is a progressTransport; token "reasonix-<N>" (atomic counter) injected as params._meta.progressToken (existing _meta preserved), unregistered after the call.
- Call timeout (contextWithCallTimeout/callTimeout, client_session.go:95): an existing ctx deadline is honoured unchanged; otherwise tools/call -> spec.ToolTimeouts[rawToolName] -> spec.CallTimeout -> spec.DefaultCallTimeout -> defaultCallTimeout (300s).
- Timeout errors (timeoutError, client_session.go:133): tool call -> "MCP tool %q timed out after %s; increase tool_timeout_seconds or call_timeout_seconds to allow longer runs: %w" (name rendered server.raw); other methods -> "MCP method %q on server %q timed out after %s; increase mcp_call_timeout_seconds or call_timeout_seconds to allow longer runs: %w". formatTimeout: whole seconds -> "Ns", else Go duration string.
- HTTP session-expiry retry (callTransport, client_session.go:83): a non-initialize, non-modern call failing with httpSessionExpiredError -> initializeSessionOn(ctx, t, false) then retry once; reinit failure -> "<err>; reinitialize failed: <err>".
- Legacy initialize handshake (initializeSessionOn, client_session.go:168): params {protocolVersion: "2025-11-25" (legacy advertised version, §1), capabilities: {roots: {listChanged: false} only when mcpRoots(spec.WorkspaceRoot) is non-empty, elicitation: elicitCapability()}, clientInfo: {name: "reasonix", version: "dev"}}. A versioned transport's MCP-Protocol-Version header is reset to "" before initialize (the header belongs to a session; initialize opens one). Result protocolVersion -> negotiatedVersion (§1); setProtocolVersion(version). recordCapabilities=true (startup): hasTools/hasPrompts/hasResources = capability key PRESENCE (even an empty object), declareToolListChanged(tools.listChanged), instructions = TrimSpace'd result.instructions; then notification notifications/initialized with empty params. recordCapabilities=false (runtime session refresh) must NOT rewrite startup-only capability flags.
- declaresToolListChanged (client_session.go:226): absent or unparseable tools capability -> false.
- redial (client_session.go:243): a replacement connection re-clears the same gates as the first: applyStoredLauncherLock -> resolveProjectLaunchAuthorization -> checkDeclaredLaunch -> newTransport — a replacement child can never start on an authorization the user withdrew.

## 11. Lazy placeholder toolset (lazy.go)

- Boot registers cheap placeholders (from the on-disk schema cache when present) and kicks the real spawn immediately, so the connection is usually up by first call (lazy.go:1-12).
- PINNING invariant (lazy.go:1-12, trySwap lazy.go:190): cache-hit placeholders are PINNED for the whole session — the registry entries are never replaced, because the provider request's tools array is part of the cached prompt prefix and swapping mid-session (whenever the live handshake differs from the cache) would invalidate the whole conversation's provider cache at 10x miss pricing. Live drift lands in the schema cache and surfaces NEXT session. Only the cache-miss connect stub still swaps (one-time, nothing real existed to present).
- Spawn state machine (lazy.go:36): idle -> inFlight -> ready | failed, gated by lazySpawn.mu so racing Execute calls share one handshake. beginInFlight fails (state failed, error "plugin host is closed") when the host is closed.
- run (lazy.go:115): startupCtx = spec.startupTimeout(); host.EnsureConnectedWithLifecycle; on error -> newStartupFailure("connect", ...) + host.RecordFailure (except ErrDeferredSpawnCancelled / context.Canceled, which fail silently); on success -> real map keyed by tool Name(), ApplyHeldMCPPolicy, trySwap, broadcastReady, then host.queueBackgroundWrite(cacheWrite) — the cache write is registered BEFORE the deferred spawn ends so Host.Close drains it, and cacheWrite captures instructions + capabilities before release (Close may clear clients).
- trySwap (lazy.go:190): only when removePrefix is set (cache miss): reg.RemovePrefix(prefix) + reg.Add(real tools), once.
- lazyTool (lazy.go:206): name (mcp__<server>__<tool>), rawName, visibleName (post StripRawPrefix), desc, schema, readOnly/destructive guarded by shared.mu (a live handshake can demote a stale cached reader or promote destructive), hasCache. Schema() empty -> {"type":"object"} else canonicalizeSchema.
- Execute outcomes (lazy.go:271):
  - ready + cache: forward to the real tool after MCPMetadata match (server + rawName) and reconcileLiveSafety; real==nil -> heldRefusal() (a HeldTools-withheld tool surfaces as tool.HeldMCPRefusalFor) or error "MCP server %q did not expose tool %q (the cached schema may be stale)".
  - ready + no cache: success text "MCP server %q is connected; its real tools are now available on the next turn".
  - failed: error "MCP server %q failed to start: %w".
  - inFlight: wait on the ready channel bounded by waitBudget (default defaultStartTimeout); budget expiry -> "MCP server %q is still initializing after %s; startup continues in background (limit %s) — retry this tool on a later turn".
  - idle + no cache: kick the spawn async, error "MCP server %q is initializing on first use — call again on the next turn for its real tools".
  - idle + cache: kick async, wait briefly — fast servers complete in the same turn; slow ones keep initializing under the session-owned lifecycle instead of being killed and restarted.
- reconcileLiveSafety (lazy.go:401): ReconcileCachedToolSafety(server, rawName, cached {ReadOnly, Destructive}, live) updates the pinned flags; a reader->writer demotion or destructive promotion stops the current call so the next attempt re-enters Plan/read-only safety checks with current metadata.
- LazyToolset(spec, cs, host, reg, sessionCtx, kick) (lazy.go:431): ResolveStoredAuthorization runs first (read-only, no process/network); spawnCtx = context.WithCancelCause(sessionCtx); generation = host.registerDeferredCancel; a cs with unusable cached names is treated as a miss; a cs with ZERO tools also degrades to the connect stub (otherwise live tools would join the registry mid-session with no reserved placeholder names); cache hit -> one lazyTool per cached tool filtered by spec.ToolEnabled, visibleName = StripRawPrefix-trimmed, name = toolName(server, visibleName); kick=true starts the handshake immediately. Connect stub: name = ToolPrefix(server)+"connect", description "Connect MCP server %q. Call this once to drive the handshake; the server's real tools become available on the next turn."


## 12. Deferred-spawn bookkeeping and startup budget (deferred_spawn.go, startup.go)

- registerDeferredCancel (deferred_spawn.go:15): records a background connect so Close can reach it; returns the generation the result is checked against; host closed -> cancel(ErrHostClosed) + generation 0 (caller must not start); first generation for a name is 1.
- beginDeferredSpawn (deferred_spawn.go:39): deferredWG.Add(1) under h.mu so Add cannot race Close's Wait; false when closed. endDeferredSpawn -> Done.
- beginSpawn (deferred_spawn.go:56): sole-claim spawn right keyed by server key; joiner receives the spawnAttempt with a done channel and stays quiet; the owner announces "%s: connecting". endSpawn (deferred_spawn.go:73) copies tools/err into the attempt, closes done.
- defaultStartupTimeout = 30s (startup.go:15): background cap on the real initialize + tools/list handshake — the model-facing wait stays shorter so a slow-but-healthy server finishes in the background without holding a turn open.
- Spec.startupTimeout (startup.go:25): StartupTimeout -> DefaultStartupTimeout -> defaultStartupTimeout (30s). DefaultStartupWaitBudget() = defaultStartTimeout.
- startupFailure (startup.go:39): Error() = "MCP startup <stage> failed after <elapsed>: <err>" (+ "; stderr: <stderr>" when present — the stderr string is only ever "subprocess output omitted (<N> bytes)", never content); Unwrap -> Err; DiagnosticFacts = Error(). newStartupFailure (startup.go:70) wraps once (existing startupFailure passes through), pulls byte counts from subprocessOutputError, and wraps the cause with secrets.DiagnosticError.
- formatElapsed (startup.go:114): sub-millisecond -> Go string, else rounded to milliseconds.
- terminalHTTPStatus: the httpStatusError status of a failure known to be terminal, else 0. Client.startupStderr() reaches through a startupDiagnosticTransport.

## 13. OAuth authorization flow (oauth.go)

Protocol sources: MCP Authorization (2025-11-25), RFC 9728 protected resources, RFC 8414 server metadata, RFC 7591 DCR, RFC 7636 PKCE. Files: oauth.json (state), oauth.generation (generation counter); maxOAuthBody = 1 MiB (oauth.go:25).

- mcpOAuthState JSON v1 (oauth.go:36): {version, resource, issuer, authorization_endpoint, token_endpoint, registration_endpoint (omitted), client_id, client_secret (omitted), token_endpoint_auth_method (omitted), scope (omitted), access_token (omitted), refresh_token (omitted), token_type (omitted), expiry}. Lives under Spec.StateDir only, never in a project or another client's keychain.
- AuthorizeHTTPMCP (oauth.go:96) — user-initiated authorization-code flow, in order: validate -> MkdirAll(StateDir, 0700) -> captureMCPOAuthGeneration -> parseSecureOAuthURL(spec.URL, allow loopback) -> discoverProtectedResource -> oauthResourceAndIssuer -> discoverAuthorizationServer -> checkAuthorizationServer -> loopback listener 127.0.0.1:0 with redirectURI "http://<addr>/oauth/callback" -> registerOAuthClient (DCR) -> PKCE verifier randomBase64URL(64) + request state randomBase64URL(32) -> authorizationScope -> callback server (ReadHeaderTimeout 5s) -> buildAuthorizationURL -> openURL -> await ctx / serveDone / callback -> exchangeAuthorizationCode -> applyTokenResponse -> saveMCPOAuthStateIfGenerationUnchanged -> OAuthScopeUpgrade -> ClearScopeUpgrade.
- validateHTTPMCPAuthorization (oauth.go:216): openURL required ("MCP OAuth: browser opener is required"); only HTTP transports ("MCP OAuth is only available for HTTP transports"); unavailable while explicit auth configured ("MCP OAuth is unavailable while explicit authentication is configured" — mcpdiag.HasAuthConfig(headers, env, url)); StateDir required.
- ReconcileHTTPMCPOAuthAfterRemoval (oauth.go:240): removes Reasonix-owned OAuth state after a declaration is removed UNLESS the remaining effective HTTP declaration uses the same resource (sameCanonicalResource); bumps generation, ForgetSignInConsent, removes the state file; 5s timeout; state lock held.
- newMCPOAuthClient (oauth.go:276): loads state; neither access nor refresh token -> (nil, nil) (no client).
- authorizationHeader (oauth.go:289): refreshes only when this client's own clock says expiry is within 30s (stale = refresh_token set AND !Expiry.IsZero() AND now+30s > Expiry). authorizationHeaderAfterReject (oauth.go:297) takes the credential that actually went out (rejected), not what the store holds now. authorization (oauth.go:304): refresh if rejected or stale; empty access token -> no header ("" , false, nil); token_type default "Bearer", non-Bearer (case-insensitive) -> "MCP OAuth: unsupported token type %q"; header = "Bearer <access_token>". canRefresh (oauth.go:327): refresh_token AND token_endpoint set.
- discoverProtectedResource (oauth.go:331): POST probe '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' (Content-Type application/json, Accept "application/json, text/event-stream"), body drained to 4096 bytes; on 401 parse WWW-Authenticate Bearer challenges -> resource_metadata URL (+ scope, error params; bearer word must be delimiter-bounded, oauth.go:655, params lowercased keys, oauth.go:675); metadata URL must parse secure AND keep the same HTTP origin ("MCP OAuth resource metadata URL changed origin"); else fall back to well-known candidates (oauth.go:615): "/.well-known/oauth-protected-resource/<path>" (when the endpoint has a path) then "/.well-known/oauth-protected-resource", both query/fragment-stripped. Failure -> "MCP OAuth protected resource discovery failed: ..." / "... protected resource metadata: ...".
- discoverAuthorizationServer (oauth.go:383): candidates (oauth.go:629) — issuer without path: "/.well-known/oauth-authorization-server" then "/.well-known/openid-configuration"; issuer with path: "/.well-known/oauth-authorization-server/<path>", "/.well-known/openid-configuration/<path>", "<issuer>/.well-known/openid-configuration". Issuer must match metadata issuer (TrimRight "/") else "issuer mismatch: metadata=%q requested=%q"; authorization/token endpoints must parseSecureOAuthURL.
- checkAuthorizationServer (oauth.go:784): authorization_endpoint + token_endpoint required ("MCP OAuth: authorization server metadata is missing authorization_endpoint or token_endpoint"); PKCE gate: no code_challenge_methods AND !OAuthAllowMissingPKCEMetadata -> ErrOAuthPKCEUnadvertised + " (set oauth_allow_missing_pkce_metadata = true on server %q in your own config to proceed)"; methods advertised without S256 -> "MCP OAuth: authorization server does not support PKCE S256".
- registerOAuthClient (oauth.go:406): requires registration_endpoint ("MCP OAuth: authorization server does not advertise dynamic client registration"); DCR body {client_name: "Reasonix", application_type: "native", redirect_uris: [loopback], grant_types: ["authorization_code","refresh_token"], response_types: ["code"], token_endpoint_auth_method: chosen}; chooseTokenEndpointAuthMethod (oauth.go:606) prefers client_secret_basic > client_secret_post > none (empty supported list -> first preference); missing client_id -> "MCP OAuth client registration returned no client_id"; empty method -> the requested one.
- buildAuthorizationURL (oauth.go:573): response_type=code, client_id, redirect_uri, state, code_challenge = base64url(sha256(verifier)) (pkceChallenge, oauth.go:593), code_challenge_method=S256, resource, scope only when non-empty.
- Callback handler (oauth.go:540): path must be /oauth/callback (else 404); state mismatch -> "MCP OAuth callback state did not match"; RFC 9207 issuer check (required when AuthorizationResponseIssParameterSupported; mismatch -> ErrOAuthIssuerMismatch); error param -> errOAuthAuthorizationResponse; empty code -> "MCP OAuth callback did not include an authorization code"; failure page 400 "Reasonix could not complete MCP authorization. You can close this window."; success HTML "Reasonix MCP authorized" body "Authorization completed. You can close this window and return to Reasonix."; single buffered result.
- Token request (exchangeAuthorizationCode oauth.go:454 + requestOAuthToken oauth.go:466): form grant_type=authorization_code, code, redirect_uri, code_verifier, client_id, resource; POST token_endpoint, Content-Type x-www-form-urlencoded, Accept application/json; client auth by token_endpoint_auth_method: client_secret_post -> client_secret in form; none -> nothing; default -> HTTP Basic with url.QueryEscape'd client_id/client_secret when client_secret non-empty; non-2xx -> oauthHTTPError; token response error field -> errOAuthTokenResponse; empty access_token -> "token response has no access_token".
- applyTokenResponse (oauth.go:507): access_token always; refresh_token only when non-empty (no downgrade on missing); token_type default Bearer; scope only when non-empty; expires_in > 0 -> Expiry = now + expires_in else zero time.
- HTTP plumbing: getOAuthJSON (oauth.go:694) GET Accept application/json, non-2xx -> oauthHTTPError "GET <url>"; decodeLimitedJSON (oauth.go:711) caps at maxOAuthBody (1 MiB); oauthHTTPError (oauth.go:719) reads at most 4096 body bytes and reports ONLY the count via httpStatusError{Status, BodyBytes}; newOAuthHTTPClient (oauth.go:724) default Timeout 30s (base client kept, 0 timeout -> 30s), CheckRedirect stops cross-origin redirects with http.ErrUseLastResponse (chaining any prior hook).
- parseSecureOAuthURL (oauth.go:745): TrimSpace; must have host, no userinfo, no fragment; https always OK; http only when allowLoopbackHTTP and host is "localhost" or a loopback IP; else "URL must use HTTPS" / "invalid URL".
- sameCanonicalResource (oauth.go:760): userinfo on either side -> false (credential material is never resource identity); fragments stripped; scheme+host case-insensitive, EscapedPath + RawQuery exact.
- isHTTPMCPTransport (oauth.go:775): "http", "streamable-http", "streamable_http" (trimmed, case-insensitive).


## 14. OAuth token refresh and state persistence (oauth_refresh.go, oauth_state.go)

- Refresh gate order (refresh, oauth_refresh.go): in-process gate first (acquireMCPOAuthRefreshGate — sync.Map of buffered chan(1) keyed by cleaned stateDir, oauth_state.go), then the cross-process file lock; the file lock is NEVER held across the token-endpoint network request.
- refresh steps: reload latest under state lock -> resource mismatch -> "MCP OAuth token refresh: stored token belongs to a different MCP resource" -> adopt latest -> latestSupersedes(latest, rejected, now) = latest usable AND (rejected == nil OR latest.AccessToken != rejected; a server's refusal outranks this machine's opinion of freshness) -> done; else !canRefresh -> errOAuthRefreshMissing; read generation; unlock; POST refresh (grant_type=refresh_token, refresh_token, client_id, resource, scope only when stored non-empty); re-lock; re-read generation + state: generation changed -> errOAuthRefreshInvalidated; state changed (sameOAuthRefreshState compares every field incl. Expiry.Equal) -> adopt current when usable else "MCP OAuth token state changed while refreshing; authorize again"; applyTokenResponse; a response that drops refresh_token keeps the old one; saveMCPOAuthState.
- oauthAccessTokenUsable: access token non-empty AND (Expiry zero OR now+30s < Expiry).
- File layout (oauth_state.go): <stateDir>/oauth.json (state), <stateDir>/oauth.generation, lock at <stateDir>/oauth.json.lock (filelock.Acquire).
- loadMCPOAuthState: missing -> empty, no error; symlink or non-regular -> "read MCP OAuth state: refusing non-regular file"; size > 1 MiB -> "file is too large"; Version != 1 -> "unsupported version %d".
- saveMCPOAuthState: refuses to overwrite a symlink/non-regular file; Version forced 1; MarshalIndent 2-space + trailing newline; fileutil.AtomicWriteFileStrict 0600.
- Generation: loadMCPOAuthGeneration refuses symlink/non-regular/size>256/empty; saveMCPOAuthGeneration TrimSpace + newline, AtomicWriteFileStrict 0600; bumpMCPOAuthGeneration = randomBase64URL(24); captureMCPOAuthGeneration runs under the state lock; saveMCPOAuthStateIfGenerationUnchanged: mismatch -> "MCP OAuth authorization was invalidated while waiting for the browser; authorize again".

## 15. OAuth helper errors, sign-in consent, auth-required promotion (oauth_helpers.go, signin_consent.go, auth_required.go)

- oauthResourceAndIssuer (oauth_helpers.go): resource = metadata.resource or the endpoint itself; must sameCanonicalResource the configured endpoint else 'MCP OAuth: protected resource %q does not match configured endpoint %q'; empty authorization_servers -> "MCP OAuth: protected resource metadata has no authorization_servers"; issuer = first entry, parseSecureOAuthURL.
- Auth-challenge param parsing: bearer word must be delimiter-bounded; params have lowercased keys; values quoted (backslash escapes) or bare tokens.
- ErrOAuthPKCEUnadvertised = "MCP OAuth: authorization server does not advertise PKCE support"; ErrOAuthIssuerMismatch = "MCP OAuth: authorization response came from a different issuer"; issuerMatches (RFC 9207): absent iss ok only when not required; present iss compared TrimRight("/").
- Sign-in consent (signin_consent.go): file signin-consent.json {version: 1, hosts: [lowercased host:port]}; AuthorizationHost(url) lowercases host ("" on parse failure); SignInHostRemembered / RememberSignInHost (append + AtomicWriteFile 0600, MkdirAll 0700); ForgetSignInConsent via os.OpenRoot Remove (missing OK). The file sits beside OAuth state so removing state forgets the agreement.
- ErrAuthenticationRequired = "MCP server requires authentication"; AuthRequiredError{Server, Resource, Scope} Error() 'MCP server %q requires authentication: %v', unwraps to [ErrAuthenticationRequired, httpStatusError] (auth_required.go).
- statusFailure (httpTransport, auth_required.go): 403 + no explicit auth config + oauth client + Bearer challenge error="insufficient_scope" -> InsufficientScopeError{Server, Resource: RedactEndpoint(url), Scopes: scopeTokens, Exhausted: recordInsufficientScope(...)}; 401 + no explicit auth config + Bearer challenge -> AuthRequiredError; otherwise plain httpStatusError. A server with explicit credentials configured is never "requires authentication" — its token is simply wrong.

## 16. Install readiness contract (install.go)

- MCPInstallInput{registryName?, definition?, name?, env?, headers?, spec (in-process, not JSON)}; old AddMCPServer/UpdateMCPServer are compatibility adapters around the same readiness path.
- MCPInstallResult JSON: {name, state: ready|action_required|issue, toolCount, action: none|authenticate|authorize|retry, message}; only state=="ready" may be phrased as success in CLI/UI copy.
- ReadyInstallResult message: '"<name>" is available with <N> tools'. InstallResultForError: requiresLaunchApproval -> action_required/authorize; ErrAuthenticationRequired -> action_required/authenticate; anything else -> issue/retry.
- Host.InstallAndConnectWithLifecycle: empty name -> state issue/retry, error "MCP server name is required"; ProcessMode defaults to host; !RequireLaunchApproval -> Authorized=true (the install action itself authorizes user/host installs); EnsureConnectedWithLifecycle; SaveCachedSchema of the discovered surface (error ignored); ReadyInstallResult(name, len(tools)).

## 17. Failure records, host queries, diagnostics, status sink (failures.go, host_queries.go, diagnostic.go, status.go)

- Failure (failures.go): {Name, Transport, Error, Stage, Elapsed, Stderr, HTTPStatus, RequiresLaunchApproval, DeclarationChanged}. Transport defaults "stdio" (lowercased, trimmed). recordFailure replaces by name or appends, then announces "<name>: <what>" AFTER unlock ("connection failed" via RecordFailure; "awaiting approval" via RecordLaunchApprovalRequired).
- launchApprovalError (failures.go): changed -> 'project-provided MCP server %q changed; blocked before process or network startup and requires explicit re-authorization'; otherwise 'project-provided MCP server %q is blocked before process or network startup until the user authorizes it'. checkDeclaredLaunch wraps any s.LaunchCheck() refusal with changed=true. requiresLaunchApproval/launchDeclarationChanged use errors.As.
- Host queries (host_queries.go): Prompts/Resources return copies; ServerNames in connection order; Failures copy; ConnectingServers = spawning keys (attempt.server preferred), sorted, status-only; Servers() -> ServerStatus{Name, Transport, ConfigSource, Description (initialize instructions), Tools, Prompts, Resources, HasTools, ToolList []ToolInfo{Name, Description, ReadOnlyHint, DestructiveHint, SchemaError}} with count+list taken from one toolsMu snapshot (ConfigSource surfaced per #6578).
- Diagnostics (diagnostic.go): errOAuthTokenResponse "MCP OAuth token response rejected"; errOAuthAuthorizationResponse "MCP OAuth authorization rejected"; errOAuthRefreshMissing "MCP OAuth access token expired and no refresh token is available; authorize again"; errOAuthRefreshInvalidated "MCP OAuth token refresh was invalidated while contacting the token endpoint; authorize again"; subprocessOutputError '<diag>; subprocess output omitted (<N> bytes)'; commandMissingError 'command %q not found on PATH; use an absolute command path or set PATH in the MCP server env'; catalogPageError "MCP catalog returned a cursor twice" / "MCP catalog did not finish within %d pages"; stdioReadFailure 'MCP subprocess read failed: <diag>' or 'server exited while handling <method>; the next call starts a fresh one: <diag>'; omittedStderrWriter writes secrets.OmittedText.
- Status sink (status.go): SetStatusSink(event.Sink) — Host-held; announce() emits event{Kind: event.MCPSurfaceReady, Text: fmt-formatted} and is never called while holding h.mu.

## 18. Host context metadata for local servers (host_meta.go)

- Vendor _meta keys on tools/call: io.reasonix/cwd (workspace root), io.reasonix/readOnly (planmode.Active(ctx)), io.reasonix/writableRoots.
- Only stdio-transport servers receive them; a remote server is told nothing (workspace path must not leave the machine).
- withHostContext: clones the caller's params map; existing _meta preserved; never overwrites a key the call already carries.
- metaWritableRoots: absolute + cleaned + de-duplicated, EXCLUDING the workspace root itself, config order preserved, capped at maxMetaWritableRoots = 32.


## 19. Tool definition approval pins (toolpin.go, toolpin_follow.go)

- HeldTool (toolpin.go): {RawName, VisibleName, Class tool.HeldMCPClass, Digest (live definition), Approved (recorded one, empty for added)}; Binding -> tool.MCPBinding{Package, Server, RawName, VisibleName, CallableName ModelToolName(server, visible), CapabilityID "mcp-tool:<server>/<raw>"}.
- Approval record: <root>/.tool-pins/<sha256(name) hex 64>.json, root = Dir(StateDir) for project servers, Dir(Dir(StateDir))/.user for user-owned servers (config.MCPConfigSource(...).UserOwned()); one record per server name across all workspaces for user-level servers; guards remote servers and unattended upgrades (an unconfined local server could rewrite it). Path exists only for a 64-hex key that stays inside the dir.
- toolPinFile {version: 1, tools: map[rawName]digest, follow bool omitempty}; MarshalIndent 2-space + fileutil.AtomicWriteFile 0600 (writeToolPins logs a warning on failure).
- mcpToolDigest: sha256 over {name, title, description, inputSchema, outputSchema, annotations} with JSON canonicalized via json.Number re-marshal (sorted keys, array order kept) — depends on nothing Reasonix rewrites, so upgrading Reasonix never moves it.
- judgeToolPins: record missing + ServerAuthorized + non-empty listing -> silently record as approved (writeToolPins); read error, parse error, wrong version, or nil Tools -> hold every tool as HeldMCPUnverifiable; otherwise per tool: unknown -> HeldMCPAdded, digest mismatch -> HeldMCPChanged, else pass. follow = pins.Follow && ServerAuthorized(); followed changes recorded by recordFollowed (merge into pins, follow=true) — when that write fails the changes move back to held, so a followed change is never accepted without leaving the record saying so. Both lists sorted by RawName.
- AcceptHeldTools(server, digest): no live client or empty held -> ErrNoHeldTools "MCP server has no held tool definitions"; digest != HeldDigest -> ErrHeldDigestMismatch "the held MCP tool definitions changed since they were shown"; records pins.Tools[raw]=digest for each held tool, returns raw names. HeldDigest = sha256 of sorted "raw\x00digest" lines joined by newline.
- Announcements: held tools -> event Notice/operator/warn code NoticeCodeMCPToolsHeld, text 'MCP server %q: %d tool(s) are held out of the model's tools because their definitions are new or differ from the ones you approved. Review the server, then run <trustCommand> to accept them.'; trustCommand embeds the name only when config.IsValidMCPServerName(name) ("reasonix mcp trust <name>") else a generic phrase. Followed changes -> Notice/info NoticeCodeMCPToolsFollowed, '...accepted because you chose to follow this server's changes.', emitted once (drains c.followed).
- ToolPinsFollow / SetToolPinsFollow (toolpin_follow.go): standing choice to take definition changes without being asked; each change still announced; unreadable record never overwritten; turning it off puts the next change back behind the user's answer.

## 20. Build-scoped registration ownership (registration_scope.go)

- HostClientRef {Name, ID uint64}: one live Client instance on a Host; desktop generation-rollback uses RemoveIfInstance so a lost build cannot tear down sibling or newer-generation connections that share a server name.
- RegistrationScope: per-build ownership token (BeginRegistrationScope, id from nextScopeID); attached via ContextWithRegistrationScope on both sync and async connection paths; states active -> committed | aborted.
- record(): aborted -> rejected (late LazyToolset registration refused with ErrRegistrationScopeAborted "plugin: registration scope aborted"); committed -> instance commits immediately (late connections after publish become Host-owned); active -> claim journaled in client.registrationClaims map[scopeID].
- Commit(): publishes journaled instances into Host ownership, idempotent, false only when already aborted; abort is terminal only for scopes that never published.
- AbortAndRollback(): marks aborted (rejects late registrations) and removes every journaled instance (rollbackRegistration: skips committed instances, closes the rest after unlock).
- noteClientLocked: assigns c.instanceID (nextInstanceID), appends to h.clients FIRST, then records on scope; rejected -> un-append + ErrRegistrationScopeAborted (caller closes); no scope -> registrationCommitted = true (Host-owned).
- claimClientFromContext: revalidates the exact instance pointer under Host.mu ("client %q changed while being claimed"), then journals the claim.
- RemoveIfInstance(name, id): exact-instance disconnect; removeClientAtLocked detaches the serverProxy (proxy.detachIf), filters prompts/resources by server name, clearFailure — never touches name-wide deferred startup generations (the owning build context cancels its own lazy work).

## 21. OAuth scope challenge and step-up (scope_challenge.go)

- Step-up authorization (MCP 2025-11-25 Scope Challenge Handling): a 403 whose Bearer error is insufficient_scope names the scope the operation needs; that header is the server's text — shown and passed on, never used to decide.
- Limits: maxChallengeScopes 32, maxScopeLength 128, maxScopeUpgrades 2, file "mcp-oauth-scope-challenge.json" in StateDir, scopeUpgradeMemory 24h.
- Errors: ErrNoScopeUpgradePending "no MCP server asked for more access"; ErrInsufficientScope "MCP server needs more access than the user granted"; ErrScopeUpgradeExhausted "MCP server still refuses after the access was extended". InsufficientScopeError{Server, Resource, Scopes, Exhausted, cause}: Error() 'MCP server %q needs more access than the user granted: %v'; Unwrap: exhausted -> [ErrScopeUpgradeExhausted, ErrInsufficientScope, cause] else [ErrInsufficientScope, cause].
- scopeTokens: RFC 6749 scope parsing — space-separated tokens, printable 0x21..0x7e except quote and backslash, length <= 128, deduped, capped at 32 (the header is a third party's).
- scopeChallenge file {resource, scopes, at, upgrades map[operation]int}: load guards regular-file/1MiB/maxScopes; save atomic strict 0600 with trailing newline; kept beside the server's OAuth state so every frontend and the next process finds the same one.
- recordInsufficientScope: >24h resets Upgrades; exhausted = allGranted(state.Scope, scopes) OR Upgrades[operation] >= 2; else Upgrades[operation]++; saves the challenge. An operation whose challenge names only already-granted scopes, or that has asked twice, will not be helped by asking again.
- scopeRefusal (Client): non-InsufficientScopeError passes through; announces once; exhausted -> tool.Refusal{CodeMCPScopeUpgradeExhausted, 'MCP server %q still refuses this call after the user extended its access. Do not retry it; tell the user what you were trying to do.'} else Refusal{CodeMCPInsufficientScope, 'MCP server %q refused this call: the access the user granted it does not cover this operation. Only the user can authorize more, and they have been told. Say what you were trying to do and wait; do not retry until they say it is done.'}.
- announceScopeChallenge: once per unique asked-scope-set (scopeAnnounced atomic compare-and-swap), Notice/warn NoticeCodeMCPScopeUpgrade 'MCP server %q asks for more access than you granted it. Sign in again to review and grant it.'
- authorizationScope: challenged scope else resource metadata scopesSupported; spec.OAuthScopeUpgrade -> merge granted + PendingScopeUpgrade (none pending -> ErrNoScopeUpgradePending); mergeScopes keeps granted tokens first and only appends new ones (never a downgrade). ClearScopeUpgrade forgets pending scopes but keeps per-operation counts.

## 22. Project launch identity, credential-free URLs, cached/live safety (security.go)

- projectLaunchIdentityDigest -> mcplaunch.ProjectLaunchIdentityDigest(buildProjectLaunchIdentity): stdio -> Dir = stdioWorkingDir(s), merged process env, resolveStdioExecutable, CommandPath + FileSHA256 of the executable (hash error: 'hash MCP executable %q: %w'); http/streamable-http/streamable_http -> Transport "http", URL normalizeIdentityURL; identity {Server, Transport, Dir, Args (LauncherIdentityArgs overrides effective args), EnvKeys sorted, HeaderKeys sorted, LauncherDigest}; installed and host-session servers never call this path.
- MCPStateDir(reasonixHome, workspace, server): <home>/mcp-state/<WorkspaceFingerprint[:16] or "global">/<slug(server)> — stable server-scoped host directory outside the workspace.
- normalizeIdentityURL: scheme/host lowercased, default ports folded (https:443, http:80), fragment dropped, query keys sorted via Encode, non-credential query values sorted; userinfo replaced by "__redacted__" (user vs user+password preserved structurally); credential query values replaced by "__redacted__" so rotation never invalidates an exact authorization. Network requests always use the raw URL, never this form.
- Credential query classification (case-insensitive, "-" and "_" stripped): exact keys auth, authorization, bearer, credential, credentials, sig, key, accesskey, secretkey, privatekey, authkey, appkey, clientkey, subscriptionkey, sharedkey; suffix families token, secret, password, passwd, apikey, signature. (Bare "*key" suffix deliberately NOT a rule — would swallow unrelated words.)
- legacyNormalizeIdentityURL: pre-credential-aware form kept only so old schema-cache keys stay readable; no longer participates in authorization or tool classification.
- CachedToolSafety {ReadOnly, Destructive}; LiveToolSafety: remoteTool -> securitySnapshot(); other tools -> ReadOnly() + MCPAnnotations.MCPDestructiveHint(). ReconcileCachedToolSafety is the single cached-to-live boundary shared by lazy and on-demand adapters — both stop the current call when the live server is stricter, before the direct tool executes: reader demotion 'MCP server %q no longer marks tool %q as read-only; the current call was blocked before execution — retry so Reasonix can apply the current Plan/read-only safety boundary'; destructive promotion 'MCP server %q now marks tool %q as destructive; retry so Reasonix can apply the current Plan/read-only safety boundary before execution'.


## 23. Catalog refresh: list_changed, TTL, paging, naming, disabled tools, known overrides (tool_list_changed.go, tool_list_ttl.go, tool_catalog.go, toolname.go, disabled_tools.go, list_pages.go, notification_router.go, known_overrides.go, codegraph_limit.go)

- notifications/tools/list_changed (tool_list_changed.go): watched only when the server declared tools.listChanged at initialize (declareToolListChanged) and the transport implements notificationTransport; a burst of notices costs one tools/list (debounce 300ms, backoff 5s, at most 4 attempts per cycle — some servers notify on every tools/list, which would be an endless loop). An abandoned cycle leaves the catalog stale on purpose: the next notice starts a fresh bounded cycle.
- One re-list is bounded by min(call timeout for tools/list (or defaultCallTimeout), ResolvedStartupTimeout). On success with a changed fingerprint: saveHandshakeSchema + onChanged(tools) (the next session pins its provider surface from this cache). On failure: warn log + retryToolListTTL, cycle ends.
- toolCatalogFingerprint: sha256 over {infos, schemas map[modelName]schema, held: HeldDigest(held)} — covers what the model is sent, what status shows, and what was withheld; a tool held for a changed definition moves the fingerprint even when the model-visible set is identical.
- Host.SubscribeToolListChanges(ctx, callback(Spec, []tool.Tool)): reports a server's refreshed set after list_changed; unsubscribe func; ctx end unsubscribes; startup catalogs are NOT replayed. Only a client the Host still holds publishes (a replaced connection cannot re-register over its successor); publishToolListChange calls announceHeld first (a refresh can withhold a tool changed under the session).
- TTL hint (tool_list_ttl.go): ttlMs on a tools/list response, clamped 30s..1h (compared in milliseconds before Duration conversion to avoid wrap; <= 0 means no timer). Arms a re-read timer; retryToolListTTL re-arms with the same hint after a failed re-read (no notices exist to start a new cycle).
- readToolCatalog (tool_catalog.go) is THE one path from a tools/list response to registrable tools: list -> filter !spec.ToolEnabled(raw name, before prefix stripping) -> validateMCPToolNames -> judgeToolPins (run on the FULL listing, including disabled tools) -> drop held from the model set (held itself still filtered by ToolEnabled) -> buildToolCatalog -> fingerprint -> checkCompositionContract. Takes no lock and publishes nothing; the caller swaps the result in as one piece (publishToolCatalogLocked moves tools/held/followed/adapters/fingerprint/toolsListed together under toolsMu).
- buildToolCatalog: normalizeAndValidateToolSchema first — an invalid schema keeps the ToolInfo row (with SchemaError) but drops the tool; toolParamHeaders failure likewise drops the tool with info.SchemaError set (warn log "tool left out for its header annotations"); remoteTool carries {name: toolName(server, visibleName), rawName, visibleName, desc, schema, outputSchema, declaredReadOnly/readOnly, destructive, paramHeaders}; rows and tools name-sorted (stable).
- Model tool-name limit (toolname.go): maxModelToolName 128 (Anthropic ^[a-zA-Z0-9_-]{1,128}$ — one long name makes the provider refuse the WHOLE request); boundedToolName(prefix, tail) keeps prefix + head of tail + "_" + shortNameHash(tail) when prefix+tail overflows (no room for one character -> returned as is). ErrAmbiguousToolCatalog = hostDiagnosticCause("MCP tool catalog has empty or colliding names"); validateCachedToolNames re-validates a stale cache against current policy.
- Disabled tools (disabled_tools.go): spec.ToolEnabled matches the RAW name before prefix stripping; DisabledMCPBindings registers bindings without tools so a stale call receives a typed refusal; ApplyDisabledMCPPolicy is a complete replacement (reg.ReplaceDisabledMCP) so entries cannot survive a reconnect whose disabled_tools shrank.
- Paging (list_pages.go): maxListPages 64; nextCursor followed until empty; a cursor seen twice -> catalogPageError (server loop); 64 pages without end -> catalogPageError{limit}; ttlMs = minimum across pages; decode errors 'plugin %q: decode %s: %w'.
- Notification router (notification_router.go): handlers run on the transport's read goroutine and must not block on I/O; per-method handler maps; dispatch copies handlers off the lock; returns whether any handler ran.
- Known overrides (known_overrides.go, runtime-only, never make a server builtin): CodeGraph (name "codegraph" EqualFold) — stdio: Dir = workspace root when empty, env CODEGRAPH_DAEMON_IDLE_TIMEOUT_MS = "5000" (default only, never overrides), and LowPriority = true (full-tree indexing + file watching must not starve the machine, #3797 #2992); codebase-memory family (ids "codebase-memory-mcp", "codebase-memory", "io.github.deusdata/" prefix, "codebase-memory-mcp@" versioned form, command basename check with .exe trimmed) — stdio Dir = workspace root when empty, LowPriority = true. isStdioSpecType: empty or "stdio".
- CodeGraph instance cap (codegraph_limit.go): maxCodeGraphInstances 4 process-wide (each tab/session boots its own Host, so many tabs used to spawn one full-tree indexer per tab — fd exhaustion, #4361 #2992 #3797); over cap: 'codegraph: %d instances already running (cap %d); not starting another to avoid file-descriptor exhaustion — close some tabs/sessions or set codegraph off for this one'; release idempotent.

## 24. MCP elicitation (elicit.go, elicit_url.go)

- Direction: MCP server -> client request "elicitation/create" (declared capability elicitCapability = {"form": {}, "url": {}}). Constants: elicitAttempts 3, elicitMaxField 32, elicitMaxMessage 2000 (runes — a longer message REFUSES the form, never trims), elicitMaxLabel 200, elicitMaxChoices 64, maxExactInteger 2^53, boolean labels "Yes"/"No".
- errBadElicitation = "unsupported elicitation request": a nested or unknown field type, an undeclared mode, duplicate property keys, or caps exceeded.
- Form parsing (parseElicitation): mode "" or "form" (anything else -> errBadElicitation); requestedSchema {type, properties (preserved in server-written order — the order the person meets the fields), required[]}; at most 32 fields. Field types: string (choices from enum/enumNames/oneOf {const,title}), number, integer, boolean (labels Yes/No), array (items.enum/anyOf, multi, must offer choices); every other type refused. Label discipline: each label <= 200 runes, description <= 800, <= 64 choices, duplicate labels refused. Defaults are rendered the way the person would give them (a choice by its label).
- elicit(): no elicitor available -> {"action": "decline"}; Elicitor error (stopped turn / timeout) -> "cancel"; Declined -> "decline"; accepted values validated by elicitContent — choices must be labels ("%q is not one of the choices"), integer parsed exactly with ParseInt and bounded to +/-2^53 ("not a whole number this form can carry"), number via ParseFloat (Inf/NaN rejected), string length by runes ("wrong length"), formats email (mail.ParseAddress round-trip), uri (scheme required), date (DateOnly), date-time (RFC3339) ("not a valid %s"). Problems -> Note carries them, the person's entries become the new defaults, retried up to 3 attempts, then "cancel". Required-but-empty -> '%q is required'.
- URL mode (elicit_url.go): mode "url" -> elicitURL: message required (nil -> errBadElicitation "invalid URL-mode request"), requestedSchema must be ABSENT/empty, url <= 8192 bytes with no control/space/Cf runes, parseable with http/https scheme, non-empty host, no userinfo, no opaque (externalurl.Host). Reply action only passes through "accept"/"cancel", everything else -> "decline"; Elicit error -> "cancel"; no elicitor -> "decline".
- stdio elicitRouter: server requests arrive unattached to any call; a form goes to the most recent registered call that has an Elicitor behind it (tool.ElicitorFrom(ctx)); ONE form open at a time (atomic CAS — a server asking again meanwhile gets no elicitor, i.e. decline); claim release reopens.
- Wire reply (elicitationReply): errBadElicitation -> rpcError -32602 (its message); any other failure -> rpcError -32603 "elicitation failed"; success -> result map {action, content?}.


## 25. Prompts and resources (prompts.go, resources.go, resource_read.go, resource_template.go)

- Prompt (prompts.go): {Name "mcp__<normalized server>__<normalized prompt>" (the slash-command body), Server, Raw (original name for prompts/get), Description, Args []PromptArg{name, description, required}}; surfaces in chat as "/mcp__<server>__<prompt>"; space-separated positional command arguments map onto declared args in order (Claude Code compatible). prompts/list via listAllPages (field "prompts"); prompts/get {name, arguments?} -> response messages[] {role, content{type,text}} flattened: text-type non-empty messages joined with blank lines.
- Resource (resources.go): {Server, URI, Name, Description, MimeType}; referenced in chat as "@<server>:<uri>" (content fetched and prepended to the message). resources/list (field "resources"); resources/read {uri} -> contents[] with text joined by blank lines, blob noted as "[binary resource %s, %s — %d base64 bytes omitted]" (a coding agent consumes text).
- read_mcp_resource tool (resource_read.go): Tool name "read_mcp_resource", registered only where the user turned it on and a server advertises resources; schema {server, uri} both required; ReadOnly false; description tells the model the address must be one the server lists or templates. Refusal codes: mcp_resource.no_server ('no connected MCP server is named %q'), .not_supported ('MCP server %q does not offer resources'), .list_failed, .not_declared ('MCP server %q does not list %q among its resources or resource templates' — a poisoned link cannot turn the tool into arbitrary access), .read_failed.
- ReadDeclaredResource: maxBytes <= 0 -> default 64 KiB (defaultResourceReadBytes); truncation backs up to a UTF-8 rune boundary and sets Truncated; truncated output appends "[truncated: the server sent %d bytes; the first %d are shown]". DeclaresResources checks the live client's hasResources, else the cached schema's capabilities["resources"].
- Resource templates (resource_template.go): RFC 6570, but only literal text, simple expansion {name} and reserved expansion {+name} are understood — any other operator makes the template match NOTHING (the host never admits an address by guessing an operator it does not read). resources/templates/list (field "resourceTemplates" {uriTemplate}); uriMatchesTemplate compiles template to anchored regex: {name} -> [A-Za-z0-9._~%-]+, {+name} -> [A-Za-z0-9._~%:/?#[]@!$&'()*+,;=-]+; a uri with whitespace or control runes never matches; empty expr, leading operator chars, or nested braces -> no match.

## 26. Tool result parsing and images (toolresult.go, toolresult_image.go)

- toolContent covers every content type defined up to 2025-11-25: {type, text, data, mimeType, uri, name, title, resource{uri, mimeType, text}}.
- parseToolResult flattens a tools/call result into plain text plus image data URLs: text -> verbatim; image -> placeholder + forwarded URL; audio -> "[audio: %s, not shown]"; resource_link -> "[resource: %s <%s>]"; resource -> "[resource <%s>\n<text>" or "[resource: <%s> %s, binary, not shown]". A result whose flattened text is empty but that carries structuredContent is given as its compacted JSON. isError=true -> error "plugin tool reported error: <text>".
- Images (toolresult_image.go): every item budgeted, never trusted from the server — kept only when: fewer than maxToolResultImages 5 already kept, mime in {image/jpeg, image/png, image/gif, image/webp} (missing -> image/png), base64 non-empty after stripping ALL whitespace (servers wrap base64; vision APIs reject non-canonical payloads), base64 length <= maxToolResultImageBytes 4 MiB, decodes as standard base64, and visionimage.Fit + DetectMime accept the BYTES (the declared type is a claim; the bytes decide). Placeholders otherwise: "[image omitted: per-result image limit reached]", "[image omitted: unsupported type <mime>]", "[image omitted: no data]", "[image omitted: %d bytes exceeds the %d-byte limit]", "[image omitted: invalid base64]", "[image omitted: <err>]". Forwarded image -> "[image: <mime>]" + "data:<mime>;base64,..." (re-encoded when Fit changed the bytes).

## 27. Tasks extension (task_driver.go, task_job.go)

- Extension "io.modelcontextprotocol/tasks": a tools/call whose result type is task is followed to completion. Poll hint is the server's word, clamped 1s..30s, default 2s (milliseconds compared before Duration conversion to avoid wrap). maxTaskPollErrors 3 consecutive transport failures ends the task; maxConcurrentTasks 8 inline slots per server; taskCancelGrace 5s for the best-effort cancel.
- Errors: ErrMCPTaskFailed "MCP task failed on the server" (with JSON-RPC code), ErrMCPTaskCancelled "MCP task was cancelled", ErrMCPTaskGone "MCP server no longer has this task" (tasks/get -32602), ErrMCPTaskUnknownStatus "MCP task reported a status this client does not know", ErrMCPTaskUnsupported "MCP server requires the tasks extension" (codeMissingClientCapability from any task call).
- awaitTask: decode failure -> 'plugin %q: unreadable task'; missing taskId -> 'task result without a taskId'; the wait carries its OWN call-timeout budget on top of the create call's (up to twice the tool timeout in all); caller context ending -> tasks/cancel; inline budget exhausted -> promoteTask (background job) else cancel + timeout error.
- followTask state machine: completed (no result -> 'completed task without a result'), failed, cancelled, input_required (bounded by maxInputRounds; more -> cancel + ErrMCPInputRequired 'task still needed input after %d rounds'; answered through the SAME answerInputRequests elicitation path — "a task is not a higher-trust channel" — then tasks/update {inputResponses}; > maxInputRequests 16 -> errMCPInputOverBounds), working or "" (sleep the clamped poll), anything else -> unknown status.
- tasks/cancel is cooperative and never waited on past the 5s grace.
- promoteTask (task_job.go): only when the turn carries a tool.BackgroundStarter; promoted jobs use a SEPARATE background slot cap (they outlive the call and must not hold inline slots); job label "<server>.<raw tool>"; output starts "following the task on server %s"; result text carries a Provenance header (Kind ProvenanceMCP, Source server); taskJobCeiling 24h bounds a promoted task whose server gave no ttl; the model never sees the task's own handle — the call returns 'MCP task %s is still running on the server and continues as background job %s: use wait or bash_output to collect its result, kill_shell to cancel it.'

## 28. In-flight cancellation (cancellation.go)

- cancelInFlight(t, method, id, cause): never sent for initialize ("initialize" — spec: a client MUST NOT cancel initialize); runs on its own goroutine with a 5s deadline (cancelNotifyLimit) so a wedged transport cannot hold it; fire-and-forget notifications/cancelled {requestId, reason} — the spec has the receiver ignore unknown ids. reason: context.DeadlineExceeded -> "client deadline exceeded" else "client cancelled the call" (wording an operator reading a Python server's log can use).


## 29. Mutable launcher locks (launcher_lock.go)

- Launchers npx/bunx/uvx are MUTABLE: the same spec can resolve to different code on each start, so a persisted authorization must pin the exact artifact. launcherLocator{kind, value, arg, prefix, command} is extracted from the spec: command base name lowercased minus .exe must be npx, bunx or uvx; uvx takes the value from --from <pkg> / --from=<pkg>; otherwise args are scanned: "--" makes the next token the value, safe launcher flags are skipped (npx: -y --yes --quiet --silent --offline --prefer-offline; bunx: --bun --no-install --silent; uvx: --offline --refresh --no-cache), any other flag WITHOUT "=" returns an empty value (unknown flags may consume the next token — refuse persistent authorization rather than accidentally pinning a flag value as the package), the first non-flag token is the value.
- preparePersistentLauncher resolves the locator into mcplaunch.LauncherLock{Server, Locator: digestText(value), ResolvedVersion, ContentSHA256, Workspace: LaunchManager.WorkspaceFingerprint()} and rewrites the spec (applyLauncherResolution, offline=false). applyStoredLauncherLock (called at the top of every start) skips when LauncherDigest is already set or LaunchManager is nil; a missing lock degrades to a normal start.
- applyLauncherResolution: args[locator.arg] = prefix + resolved; for a git+ locator whose resolved value is a full commit, resolved becomes repo@commit (ref replaced in place). spec.LauncherIdentityArgs captures the canonical identity args BEFORE the enforcement flag is added, so preflight and subsequent starts compare equal while the process runs offline. Offline mode (stored-lock starts) injects --offline (bunx: --no-install) when absent — for "uvx --from pkg cmd" the flag is inserted BEFORE --from so the option/value pair stays adjacent. spec.LauncherLocator/LauncherResolvedVersion/LauncherDigest = the lock's fields; LauncherDigest = mcplaunch.LauncherLockFingerprint(lock).
- Resolution: fullGitCommit accepts ONLY a complete SHA-1 (40 hex) or SHA-256 (64 hex); abbreviations and custom refs are never treated as a verified commit. npm: "npm view <locator> version dist.integrity --json" (npm must be on the enriched PATH) -> "<name>@<version>" + digestText(integrity); npmPackageName refuses ":" and unscoped "/". PyPI (uvx): pep508Package regex, wildcard versions refused ("uvx locator %q uses a wildcard version; pin one exact version"), GET https://pypi.org/pypi/<name>[/<version>]/json (4 MiB body cap), urls[].digests.sha256 sorted and joined with newlines for the digest, resolved "<name>[<extras>]==<version>". git+ locators need an explicit @ref; a non-commit ref resolves via "git ls-remote -- <remote> <ref>" where the remote must match https://, ssh:// or git:// (http:// and local paths refused — the remote reaches ls-remote as an operand), the git environment is gitcmd.Env() with ONLY PATH taken from the launcher env (GIT_* settings may name a program), gitcmd.Detached cleanup, and the first output field must be a full commit SHA (lowercased). digestText = sha256 hex of the string.

## 30. Stable server proxy (proxy.go)

- serverProxy{name, active *Client, generation uint64, closed atomic.Bool} is a stable handle for one MCP server name: consumers keep the same tool names while the live *Client rolls underneath (swap under the proxy's own leaf lock; swap refuses once closed: 'plugin: server proxy %q closed'; replace closes the previous client when it differs and has a transport; detachIf clears the backend only for the exact instance).
- Host.ReplaceServerBackend(ctx, name, next, generation) swaps the live client behind the stable proxy: tool schemas and names stay owned by the registry, only the connection moves. The proxy and the clients slice are published under the SAME Host lock (a scope rollback could otherwise remove next after h.mu is released but before p.replace publishes it, leaving the proxy pointed at a closed client); noteClientFromContext registers the new client; the previous client is dropped from h.clients and closed after unlock; bindToolListChanges rebinds the new client. Host.CancelInFlightMCP is a best-effort drain hook: it closes every server's active proxied backend so generation drain can abort mid-call work. lookupClientLocked consults proxies first (by name), then falls back to h.clients, and returns nil when the host is closed.

## 31. Composition guards (composition_contract.go, composition_process.go)

- A composition session PINS the first MCP tool catalog: Host.PinCompositionTools(name, expected) stores a compositionToolGuard{fingerprint} (sha256 of the catalog) on the client and in the host map and returns the fingerprint hex; a non-empty expected that differs -> 'MCP %s tool contract differs from the session binding'; a fingerprint that changes while binding -> guard marked invalid + 'MCP %s tool contract changed while binding'. Every later catalog refresh runs checkCompositionContract: a changed fingerprint marks the guard invalid and fails with 'MCP %s tool contract differs from the session binding'; tool EXECUTION checks compositionContractError and fails with 'MCP %s tool contract changed; this session cannot continue'; Host.CompositionContractError() reports any invalid guard as 'MCP %s tool contract changed'. attachCompositionGuard re-binds the stored guard on reconnect so drift is detected across reconnects, not just refreshes.
- CompositionProcessRoot(s Spec) validates ProcessStateDir: it must be EXACTLY one local path element beneath <StateDir>/sessions (filepath.Rel is local, not ".", and base == rel), and each of StateDir, StateDir/sessions, ProcessStateDir is created 0700 and must be a real directory that is not a symbolic link ('MCP process state crosses a symbolic link'; 'invalid MCP process state directory'). compositionTransportState appends /runtime when a Package is present.

## 32. Credential projection and redaction (docs/MCP_CREDENTIAL_PROJECTION.md, enforced by internal/base/secrets)

- The STRUCTURAL rule (internal/base/secrets owns it): keys are percent-decoded, NFKC-normalized, split at camel-case boundaries and every non-letter/non-digit separator, then Unicode-folded; comparisons use complete components. Sensitive components: token, password, passwd, secret, key, auth, authorization, credential, signature, sig, session, cookie, bearer, jwt, apikey, access, private (credentials and separated pwd also recognized; bare PWD/OLDPWD keep their working-directory meaning).
- URL userinfo is REMOVED; query and fragment keys and values are inspected (including values containing nested URLs or query strings); keyed path credentials and opaque fragments are hidden. Headers hide the entire value of credential-bearing names regardless of scheme; assignments and argument carriers use the same key rule; header/env carrier flags hide their operands entirely. Invalid parses, ambiguous commands and depth exhaustion hide the whole affected value; recursive projection bounded to 32 levels.
- Diagnostics: dependency prose is NOT a credential grammar — HTTP/SSE bodies, RPC messages, subprocess tails and OAuth rejection descriptions NEVER become diagnostic text; producers expose typed identity and bounded facts (status, RPC code, byte counts). DiagnosticError preserves errors.Is/errors.As; unknown causes expose their Go type, never their message. Subprocess stderr mirrors emit byte-count summaries.
- Guards: sink registry checks projection owners and scans MCP consumers for new raw endpoint/header/env formatting; deterministic corpus of 512 seeds plus structural carriers, separators, encodings, Unicode folding, nesting, IPv6. MCP producers do NOT use legacy transcript/extension text scrubbers. Operational config remains the authority for installation, approval hashing, persistence and connections; display and export consume projected copies.


## 33. Divergences and decisions to carry into implementation

1. **AGENTS.md §5.1 mapping correction (documentation)**: the MCP row points at docs/EXTENSION_PROTOCOL.md, but that document specifies the extension SIDECAR protocol (reasonix.extension.v2, area internal/ext/extension/**). The MCP client contract is pinned to internal/ext/plugin/** — this document. AGENTS.md §5.1 gets that correction when this contract lands.
2. **Version matrix is load-bearing**: four supported protocol versions (2025-11-25, 2025-06-18, 2025-03-26, 2024-11-05) plus the handshake-free "2026-07-28" modern era with era codes -32020/-32021/-32022 and pickEra negotiation; empty negotiated version falls back to the OLDEST supported version, unknown versions are refused. The Rust client must reproduce the whole matrix, including the MCP-Protocol-Version header rule (send on every streamable-HTTP request once versioned; a missing header means the server assumes 2025-03-26) and "legacy SSE transport always uses legacy initialize".
3. **Error prose discipline**: rpcError.Error() prints "rpc error %d (message omitted; %d bytes)" — server messages NEVER become error strings; DiagnosticError preserves errors.Is/errors.As and unknown causes expose their Go type, never their message. Rust needs the same typed-code + bounded-facts error design (thiserror with codes, no message passthrough).
4. **Identity asymmetry**: normalizeIdentityURL (userinfo->'__redacted__', sorted query keys, folded default ports) feeds identity digests and cache keys while the RAW URL is always used for the network. legacyNormalizeIdentityURL exists only so old cache keys still hit. Rust must keep both and must reproduce the digest inputs byte-for-byte where files are shared with the Go binary at cutover (schema cache key, tool-pin digests, launcher lock fingerprints, HeldDigest).
5. **Shared on-disk artifacts**: <root>/.tool-pins/<sha256(name)>.json, <CacheDir>/mcp/<slug>.[<tag16>.]json (cacheVersion 2), oauth.json (v1) + oauth.json.lock + signin-consent.json, mcp-oauth-scope-challenge.json, <home>/mcp-state/<WorkspaceFingerprint[:16]|global>/<slug>. During cutover both binaries may read these; format and key computation must be identical (slug rules include Windows reserved device names + sha256[:6] suffix).
6. **Fail-closed catalog policy**: x-mcp-header annotation errors (errBadParamHeader) SILENTLY drop the tool from the catalog rather than failing the server; resource templates with unsupported RFC 6570 operators match NOTHING; tool pins that cannot be verified hold ALL tools as HeldMCPUnverifiable. Silent degradation here is intentional; do not turn it into an error in Rust.
7. **Server word vs host policy**: poll hints clamped (1s..30s, milliseconds compared BEFORE duration conversion to avoid overflow), TTL hints clamped 30s..1h, images budgeted (5 per result, 4 MiB base64, bytes-decided mime via Fit+DetectMime), outputByteLimit/stdio caps — the server's claims are never trusted for limits. Keep every clamp.
8. **Platform asymmetries to keep**: envKeyEqual is EqualFold on Windows ONLY; Windows stdio keeps the host TEMP (108-byte unix-socket limit) while non-Windows redirects TMP/TEMP/TMPDIR; cachedShellPATH login-shell probing is non-Windows ONLY; Windows fallback PATH candidates list. maxCodeGraphInstances 4 is PROCESS-WIDE, not per-host.
9. **Security invariants (no relaxation)**: same-origin redirect policy with ErrUseLastResponse (credentials never leak cross-origin); launcher git remotes restricted to https/ssh/git:// with operands past --; launcher git environment stripped of GIT_*; OAuth state files refused when symlink/non-regular/over 1 MiB; OAuth http only for loopback when allowed; resource reads must be declared or template-matched; scope step-up bounded (2 upgrades, 24h memory); elicitation and task input share the same trust path.
10. **The pinning invariant is behavioral, not cosmetic**: cache-hit placeholder tools stay pinned for the whole session (swapping a provider tools array re-primes the cache at 10x miss pricing); live drift lands in the schema cache and surfaces NEXT session; a composition session pins the first catalog and refuses drift. These orderings are observable by users and must be tested as behaviors.

## 34. Proposed ledger rows

| ID | Area | Reference | Rust | Status | Evidence |
|---|---|---|---|---|---|
| MCP-001 | MCP | internal/ext/plugin (namespacing mcp__) | reasonix-ext | todo | M1: docs/contracts/mcp.md §0 |
| MCP-002 | MCP | protocol versions/eras | reasonix-ext | todo | M1: docs/contracts/mcp.md §1 |
| MCP-003 | MCP | modern era negotiation + era codes | reasonix-ext | todo | M1: docs/contracts/mcp.md §1 |
| MCP-004 | MCP | Spec/Host lifecycle | reasonix-ext | todo | M1: docs/contracts/mcp.md §2 |
| MCP-005 | MCP | StartPolicy/start phases | reasonix-ext | todo | M1: docs/contracts/mcp.md §2 |
| MCP-006 | MCP | tool naming/normalization | reasonix-ext | todo | M1: docs/contracts/mcp.md §3 |
| MCP-007 | MCP | rpcRequest/response wire | reasonix-ext | todo | M1: docs/contracts/mcp.md §3 |
| MCP-008 | MCP | stdio transport | reasonix-ext | todo | M1: docs/contracts/mcp.md §4 |
| MCP-009 | MCP | stdio env/PATH handling | reasonix-ext | todo | M1: docs/contracts/mcp.md §4 |
| MCP-010 | MCP | streamable HTTP transport | reasonix-ext | todo | M1: docs/contracts/mcp.md §5 |
| MCP-011 | MCP | HTTP headers/x-mcp-header | reasonix-ext | todo | M1: docs/contracts/mcp.md §5 |
| MCP-012 | MCP | legacy SSE transport | reasonix-ext | todo | M1: docs/contracts/mcp.md §6 |
| MCP-013 | MCP | reconnecting wrapper | reasonix-ext | todo | M1: docs/contracts/mcp.md §7 |
| MCP-014 | MCP | handshake schema cache | reasonix-ext | todo | M1: docs/contracts/mcp.md §9 |
| MCP-015 | MCP | client session dispatch/initialize | reasonix-ext | todo | M1: docs/contracts/mcp.md §10 |
| MCP-016 | MCP | lazy toolset pinning | reasonix-ext | todo | M1: docs/contracts/mcp.md §11 |
| MCP-017 | MCP | deferred spawn/startup | reasonix-ext | todo | M1: docs/contracts/mcp.md §12 |
| MCP-018 | MCP | OAuth authorize flow | reasonix-ext | todo | M1: docs/contracts/mcp.md §13 |
| MCP-019 | MCP | OAuth refresh/state | reasonix-ext | todo | M1: docs/contracts/mcp.md §14 |
| MCP-020 | MCP | OAuth helpers/consent/auth-required | reasonix-ext | todo | M1: docs/contracts/mcp.md §15 |
| MCP-021 | MCP | install | reasonix-ext | todo | M1: docs/contracts/mcp.md §16 |
| MCP-022 | MCP | failures/queries/diagnostics/status | reasonix-ext | todo | M1: docs/contracts/mcp.md §17 |
| MCP-023 | MCP | host _meta | reasonix-ext | todo | M1: docs/contracts/mcp.md §18 |
| MCP-024 | MCP | tool pins | reasonix-ext | todo | M1: docs/contracts/mcp.md §19 |
| MCP-025 | MCP | registration scope | reasonix-ext | todo | M1: docs/contracts/mcp.md §20 |
| MCP-026 | MCP | scope challenge | reasonix-ext | todo | M1: docs/contracts/mcp.md §21 |
| MCP-027 | MCP | identity/safety reconciliation | reasonix-ext | todo | M1: docs/contracts/mcp.md §22 |
| MCP-028 | MCP | catalog refresh | reasonix-ext | todo | M1: docs/contracts/mcp.md §23 |
| MCP-029 | MCP | MCP elicitation | reasonix-ext | todo | M1: docs/contracts/mcp.md §24 |
| MCP-030 | MCP | prompts/resources | reasonix-ext | todo | M1: docs/contracts/mcp.md §25 |
| MCP-031 | MCP | tool result parsing/images | reasonix-ext | todo | M1: docs/contracts/mcp.md §26 |
| MCP-032 | MCP | tasks extension | reasonix-ext | todo | M1: docs/contracts/mcp.md §27 |
| MCP-033 | MCP | in-flight cancellation | reasonix-ext | todo | M1: docs/contracts/mcp.md §28 |
| MCP-034 | MCP | mutable launcher locks | reasonix-ext | todo | M1: docs/contracts/mcp.md §29 |
| MCP-035 | MCP | stable server proxy | reasonix-ext | todo | M1: docs/contracts/mcp.md §30 |
| MCP-036 | MCP | composition guards | reasonix-ext | todo | M1: docs/contracts/mcp.md §31 |
| MCP-037 | MCP | credential projection | reasonix-ext | todo | M1: docs/contracts/mcp.md §32 |
| MCP-038 | MCP | divergences §33 | reasonix-ext | todo | M1: docs/contracts/mcp.md §33 |
