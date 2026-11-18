# ACP Contract — Frozen Inventory (M1)

Status: FROZEN 2026-11-15. Extracted from source; every product file in the
reference ACP package has been read in full (provenance in §12). Oracle
priority per AGENTS.md §11.3: reference source and recorded behavior first,
then docs/ACP.md, then this file.

Sources: reference/DeepSeek-Reasonix-studio/docs/ACP.md (557 lines, read in
full) and reference/DeepSeek-Reasonix-studio/internal/frontend/acp/ (34 product
files). Rust target crate: reasonix-acp.

The reference implements Agent Client Protocol v1 (protocolVersion 1) as an
NDJSON JSON-RPC 2.0 agent over stdio. stdout carries only ACP frames;
diagnostics go to stderr and must never be merged (docs/ACP.md:35-36).

## 0. Transport and framing (server.go)

- NDJSON: one JSON-RPC 2.0 frame per line; the read loop uses a 64 KiB
  buffered reader (server.go:140). A trailing line without a final newline is
  still processed (readLine, server.go:168-186).
- Inbound line cap: maxMessageBytes = 32 MiB (32 << 20). A longer line is a
  framing error "acp: message exceeds size limit" (server.go:16, server.go:174).
- Lines are trimmed of trailing CR/LF and of leading/trailing ASCII whitespace
  (space, tab, CR, LF) before parsing (server.go:180-200).
- Outbound encoding: json.Encoder with SetEscapeHTML(false), so payloads match
  JSON.stringify byte-for-byte; a mutex serializes writes so concurrent
  handlers never interleave a line (server.go:52,110-112).
- Outbound frame shape: {jsonrpc, id?, method?, params?, result?, error?} with
  omitempty selecting request / notification / success / error forms
  (server.go:88-95). Outbound request ids are int64, monotonically increasing
  from 1 (server.go:309). Inbound response ids that do not parse as int64 are
  ignored — "we only issue integer ids" (server.go:275-277).
- Dispatch rules (server.go:205-230):
  - method + id → request handler on its own goroutine;
  - method == $/cancel_request without id → routed to the in-flight set
    (flight.cancel), never a notification handler;
  - method without id → notification handler if registered (else dropped),
    on its own goroutine;
  - no method + id → response resolution against pending outbound requests;
  - neither → writeError(id=null, -32600, "invalid request").
- Parse failure of a frame → -32700 "parse error" with id null
  (server.go:207-209).
- Unknown request method → -32601 "method not found: <method>"
  (server.go:236).
- Handler error mapping: *RPCError carries {Code, Message, Data}; any other
  error becomes -32603. If the request was $/cancel_request'd and its ctx is
  done, the error is rewritten to ErrRequestCancelled (-32800) with no data
  (server.go:241-253).
- A successful handler result is marshaled and written, then any
  responseWithAfter hook runs (server.go:255-270): replay of session/load
  transcripts must wait until the response is on the wire.
- Serve ends on EOF (clean, returns nil) or ctx cancel; it then cancels
  in-flight handlers, waits for them to flush, and fails pending outbound
  requests with "acp: connection closed" (server.go:136-163, 353-363).
- Conn.Request waits on response / ctx cancel / connection close
  (server.go:304-332). Conn.Notify is fire-and-forget (server.go:294-300).

## 1. JSON-RPC error codes (protocol.go:27-39 + vendor sources)

| Code | Constant | Meaning |
|---|---|---|
| -32700 | ErrParse | Frame does not parse as JSON. |
| -32600 | ErrInvalidRequest | Frame is neither request, notification, nor response; also malformed JSON-RPC frame per docs/ACP.md:450-454, and pathless steer with no active prompt. |
| -32601 | ErrMethodNotFound | Unknown request method; also the answer to unnamespaced session/steer (docs/ACP.md:385). |
| -32602 | ErrInvalidParams | Bad params, unknown configId, unknown session; NOT a lease refusal (docs/ACP.md:148-149). |
| -32603 | ErrInternal | Internal failure; also turn_failed envelope and unclassifiable lease failures. |
| -32000 | ErrAuthRequired | ACP schema "Authentication required" (session/new without a key). |
| -32800 | ErrRequestCancelled | Answer to a $/cancel_request'd request (or -32800 per docs/ACP.md:270-272 when it had no result). |
| -32010 / -32011 | goal resume/pause | _reasonix.io/session/goal/resume: no resumable Goal; pause: no running Goal (docs/ACP.md:465-466). |
| -32020 / -32021 | inbox refused / may-have-run | -32020 "the queue refused this call"; -32021 "the work may already have run" (docs/ACP.md:419-431). |
| -32022 / -32023 | lease held / unprovable | busy.session_in_use / session.lease_unprovable (docs/ACP.md:144-156). |

Turn failure identity: TurnFailed = "turn_failed" in TurnErrorData
{code} riding error.data (protocol.go:37-44); message clipped to 2048
bytes, secrets redacted (docs/ACP.md:315-319).

## 2. initialize and authenticate (handshake.go:14-91)

initialize returns (handshake.go:19-69):
- protocolVersion: 1.
- agentCapabilities.loadSession: true.
- agentCapabilities.sessionCapabilities: list, resume, close, delete,
  additionalDirectories — each serializing to {}.
- agentCapabilities.promptCapabilities: image true, audio false,
  embeddedContext true.
- agentCapabilities.mcpCapabilities: http true, sse false.
- agentCapabilities._meta (goalCapabilities merge, handshake.go:36-65):
  - "reasonix.io": { sessionSteer: {method: "_reasonix.io/session/steer"},
    sessionInbox: {schemaVersion: 1, methods: 9 entries — enqueue, list, get,
    update, delete, move, setPaused, retry, refresh, each mapping to
    "_reasonix.io/session/inbox/<verb>"}, sessionReloadExtensions: {method:
    "_reasonix.io/session/reloadExtensions"}, extensionSurface: {supported:
    true, schemaVersion: 1} }.
  - plus ReasonixSchemaCapability entries (schemaVersion only; method-name
    constants and version values live in status.go / session_mcp_control.go /
    session_info.go — recorded in §9 once read): sessionStatusMethod,
    sessionStatusUpdateMethod, sessionMCPStatusMethod,
    sessionMCPStatusUpdateMethod, sessionInfoMethod, sessionMCPServersMethod,
    sessionMCPReconnectMethod, sessionMCPDisconnectMethod,
    sessionExtensionSubmitFormMethod.
- agentInfo: {name, version} from the service info (ldflags-injected).
- authMethods: exactly one — {id: "reasonix-setup", name: "Reasonix setup",
  description: "Configure Reasonix providers and credentials in a terminal",
  type: "terminal", args: ["setup"]} (handshake.go:72-80).

initialize tolerantly records client capabilities when params parse
(handshake.go:15-17). authenticate accepts only methodId "reasonix-setup";
anything else → -32602 ("authenticate: unknown methodId <id>")
(handshake.go:82-91). Per docs/ACP.md:94-95 the terminal method means a client
runs reasonix setup and reconnects; it must not send authenticate for it, and
provider keys come from <Reasonix home>/.env, never host environment
variables.

## 3. Inbound wire types (protocol.go)

Client → agent params/results, with JSON field names exactly as serialized:

- InitializeParams { protocolVersion int, clientInfo? Implementation, clientCapabilities? }.
- ClientCapabilities { fs? {readTextFile bool, writeTextFile bool}, terminal bool, elicitation? *ElicitationCapabilities (present-and-not-null semantics only), _meta? map } (protocol.go:62-69).
- Implementation { name, title?, version? }.
- AuthenticateParams { methodId }.
- SessionNewParams { cwd?, additionalDirectories? []string, mcpServers? []MCPServerSpec }.
- MCPServerSpec { name, type?, command?, args?, env?, url?, headers? }.
- MCPEnv / MCPHeaders: both map[string]string with dual-shape UnmarshalJSON
  (protocol.go:234-277) — ACP's official [{"name":...,"value":...}] array or
  the legacy {k: v} object map; empty/whitespace name in array form → error
  "env[i].name is required"; neither shape → "env must be an array of
  {name,value} objects" (same messages for "headers").
- SessionNewResult { sessionId, models? *SessionModelState, modes? *SessionModeState, configOptions? []SessionConfigOption }.
- SessionMode { id, name, description? }; SessionModeState { currentModeId, availableModes []SessionMode }.
- SessionModelState (legacy selector) { availableModels []ModelInfo, currentModelId }; ModelInfo { modelId, name, description? }.
- SessionLoadParams / SessionResumeParams { sessionId, cwd?, additionalDirectories?, mcpServers? }; results are SessionLoadResult / SessionResumeResult with the same models?/modes?/configOptions? members as SessionNewResult (no sessionId).
- SetSessionConfigOptionParams { sessionId, configId, value } (field is configId, not optionId — docs/ACP.md:196) → SetSessionConfigOptionResult { configOptions } (full refreshed array).
- SessionConfigOption { id, name, description?, category?, type, currentValue, options []SessionConfigSelectOption }; option entries { value, name, description? } (protocol.go:378-393).
- SetSessionModelParams { sessionId, modelId } (legacy, empty ack).
- SessionListParams { cwd?, cursor? } → SessionListResult { sessions []SessionInfo, nextCursor? (never set — in-process list is unpaged, protocol.go:414-419) }.
- SessionInfo { sessionId, cwd, title?, updatedAt?, _meta? }.
- SessionCloseParams / SessionDeleteParams { sessionId }, empty acks.
- ContentBlock { type, text?, resource? *ResourceContents, mimeType?, data? }; ResourceContents { uri, mimeType?, text?, blob? } (protocol.go:454-468).
- SessionPromptParams { sessionId, prompt []ContentBlock }.
- SessionSteerParams { sessionId, prompt []ContentBlock } → SessionSteerResult { itemId?, disposition? }.
- Inbox params: enqueue { sessionId, text, intent? (followup|steer), idempotencyKey? }; item { sessionId, itemId }; update adds text; move adds toIndex (0-based); setPaused { sessionId, paused bool } (protocol.go:494-539).
- SessionReloadExtensionsParams { sessionId } → result { queued? bool }.
- StopReason wire values: "end_turn", "cancelled" only (protocol.go:559-565). SessionPromptResult { stopReason, _meta? }; transcript path rides _meta["reasonix.io"].transcriptPath (docs/ACP.md:303).
- SessionCancelParams { sessionId } (notification).
- PermissionRequestResult { outcome: { outcome: "selected"|"cancelled", optionId? } }.

## 4. Agent → client requests (clientio.go)

Registered only against capabilities the client declared at initialize:

- fs/read_text_file — params { sessionId, path, line?, limit? } (Reasonix always reads whole files and pages locally, so it sends neither line nor limit, protocol.go:668-676). Result { content }. Degrades: ok=false on missing capability, client error, or bad JSON → the tool reads local disk (clientio.go:61-74).
- fs/write_text_file — params { sessionId, path, content }. Degrades to local write only when the capability is absent; with the capability present a client error is a real write failure and is surfaced, because falling back could double-apply (clientio.go:80-88).
- terminal/create — params { sessionId, command, args?, cwd?, env? []EnvVariable, outputByteLimit? }. clientio sets outputByteLimit = 1 MiB (terminalOutputByteLimit = 1 << 20, clientio.go:92). Env carries ONLY session-private TMPDIR/TMP/TEMP overrides plus the user's [tools.shell] env — never a full environment dump — sorted by key for stability (clientio.go:103-113, 157-175).
- terminal/wait_for_exit, terminal/output, terminal/kill, terminal/release — params { sessionId, terminalId }. Flow: create → wait_for_exit (with the tool timeout) → output → release (deferred, context.WithoutCancel); timeout → terminal/kill then typed TerminalTimeoutError; non-zero exit preserved via TerminalExitError; signal → error "terminated by signal <sig>"; client-truncated output gets a "
…(output truncated by the client terminal)" suffix (clientio.go:103-191).
- Creation failure or unparsable reply → ok=false and the bash tool runs locally; every miss degrades rather than failing the call (clientio.go:20-23, 103-121).

Per docs/ACP.md:77-85, when the client offers fs or terminal capabilities,
every file tool (reads, edits, writes) routes through the editor's unsaved
buffers and eligible foreground commands through the client terminal; a
non-UTF-8 file is not eligible and stays on the local encoding-preserving
path.

## 5. session/update notification variants (protocol.go:574-664)

SessionUpdateParams { sessionId, update any } — update is a tagged union
discriminated by sessionUpdate:

- agent_message_chunk / agent_thought_chunk: { sessionUpdate, messageId?, content ContentBlock (single block), metadata? {error? {name, message}} }. messageId is opaque and per sampling attempt; a retried attempt gets a new one; a chunk outside any attempt carries none (docs/ACP.md:261-265).
- _reasonix.io/extension_surface (extensionSurfaceUpdate): { sessionUpdate: "_reasonix.io/extension_surface", _meta } — the structured Extension Protocol v2 surface DTO rides _meta["reasonix.io"]["extensionSurface"]; always paired with a flattened agent_message_chunk text fallback (protocol.go:596-607).
- available_commands_update: { sessionUpdate, availableCommands []AvailableCommand }; AvailableCommand { name, description, input? {hint} } — invocations come back as session/prompt text like "/review diff" (protocol.go:619-637). Plugin extension actions appear as /<plugin>:<action> (docs/ACP.md:526-528).
- config_options_update (configOptionUpdate): { sessionUpdate, configOptions []SessionConfigOption } — complete refreshed state.
- plan (planUpdate): { sessionUpdate, entries []PlanEntry } with PlanEntry { content, priority, status }; each update carries the complete plan and replaces the previous one, mirroring todo_write (protocol.go:645-658).
- current_mode_update (currentModeUpdate): { sessionUpdate, currentModeId }.
- tool_call: pending/completed tool-call updates — shape defined with the dispatch mapping (§8, pending).
- session/request_permission: { sessionId, toolCall { toolCallId, name?, title?, kind?, status?, content?, rawInput?, locations?, _meta? }, options []{ optionId, name, kind } }; option kinds are exactly allow_once, allow_always, reject_once, reject_always (protocol.go:748-793). The tool's own name rides beside its title; the name is informational (docs/ACP.md:266-268).

## 6. Session lifecycle and controls (docs/ACP.md)

- Methods: session/new, session/load (replays transcript as session/update
  before returning), session/resume (no replay), session/prompt (streams
  until a stop reason), session/cancel (notification), session/list,
  session/close (stop + release, keeps history), session/delete (stop +
  remove persisted ACP history) (docs/ACP.md:103-112).
- Each session owns an independent controller, workspace root, model,
  execution setting, collaboration mode, approval mode, MCP set, and persisted
  transcript; state does not leak (docs/ACP.md:99-101).
- mcpServers on new/load/resume: stdio, Streamable HTTP, legacy SSE accepted;
  official [{"name","value"}] shape for stdio env and HTTP headers, older
  object-map still accepted (docs/ACP.md:114-117).
- additionalDirectories on new/load/resume: absolute folders joining as write
  roots; session-only; vetted like user-granted roots (exists, user-owned, no
  keys, not a link, no overlap); one refused folder refuses the whole request
  with -32602 and data.code naming the reason (e.g.
  workspace_roots.protected); at most 32; write-inside-cwd-only agents refuse
  them (acp.additional_directories.disabled); resuming an open session with a
  different set is refused (acp.additional_directories.session_open)
  (docs/ACP.md:119-131).
- Lease: new/load/resume take the transcript lease for the session lifetime;
  refusals -32022 busy.session_in_use / -32023 session.lease_unprovable with
  human-readable message; -32603 when unclassifiable; -32600 is never a lease
  refusal (docs/ACP.md:133-164).
- Controls (docs/ACP.md:166-206): collaboration mode normal|plan|goal via
  modes + session/set_mode (legacy values default→Normal+Ask,
  auto→Normal+Yolo); model, reasoning effort, execution setting
  (agent_preset; legacy id work_mode, economy→light), tool approval
  (ask|auto|yolo) via configOptions + session/set_config_option. Unknown id →
  -32602. Model/effort changes rebuild the controller preserving history;
  agent_preset/tool_approval update the gate in place.
- --permission-mode ceiling (docs/ACP.md:208-234): order lowest→highest
  readOnly, dontAsk, ask, auto, yolo; client may move down but not up; a raise
  answers -32602 with data.code acp.tool_approval.above_ceiling and
  data.ceiling; plan needs an interactive session and is refused; flag is
  command-line only; bypassPermissions ceiling equals no ceiling; a saved
  session file with a higher posture opens at the ceiling and keeps the stored
  value until the client chooses; a posture is not an OS sandbox (Windows has
  no shell sandbox).
- session/prompt content (docs/ACP.md:238-245): text blocks, image blocks,
  embedded resources; audio not advertised; image or blob (base64) resources
  are saved under .reasonix/attachments/ in the session workspace and
  referenced from the prompt; non-base64 data or unsupported image →
  invalid params, never dropped.
- During a turn the client may receive: message/thought chunks, pending and
  completed tool-call updates, complete plan updates, available slash
  commands, current-mode and config-option updates, and
  session/request_permission (docs/ACP.md:247-259).
- $/cancel_request {requestId} cancels one in-flight request without
  ending the session: answered with the request's own result if it has one,
  else -32800; unknown or already-answered ids are ignored; session/cancel
  still stops the active turn (docs/ACP.md:268-273).
- Elicitation (docs/ACP.md:274-290): an MCP server's form or external-page
  request becomes elicitation/create when the client advertised
  clientCapabilities.elicitation.form or .url; a mode counts only when its
  key is present and not null; message begins "MCP server <name> asks:";
  control characters removed, text capped; requestedSchema is a flat object
  (string, enum, or array of enum per question); out-of-enum / wrong shape /
  over-cap declines the whole form; a page is an http(s) address the host
  validated; accept for a page means the person agreed to open it.
- Turn failure (docs/ACP.md:305-319): a turn that fails after admission ends
  session/prompt with a JSON-RPC error, not a stop reason: -32603, data.code
  turn_failed, message clipped to 2048 bytes, secrets redacted. ACP allows
  only end_turn, max_tokens, max_turn_requests, refusal, cancelled as stop
  reasons, and the reference only ever emits end_turn / cancelled
  (protocol.go:562-565).
- Paused turn (docs/ACP.md:321-328): stopReason "end_turn" plus typed outcome
  in _meta["reasonix.io"].turnOutcome e.g. {kind: "paused", reason}; the
  streamed text ends with a host-written "[paused] Not verified, …" line.

## 7. Vendor extensions (docs/ACP.md + protocol.go constants)

- Steer: method _reasonix.io/session/steer (sessionSteerMethod,
  protocol.go:492); discover via agentCapabilities._meta["reasonix.io"].sessionSteer.method;
  call while session/prompt is active; result {itemId, disposition:
  steer_accepted | queued_followup}; guidance durably committed before
  returning; unknown session or empty prompt → -32602; pathless compatibility
  session with no active prompt → -32600 and nothing queued; bare
  session/steer → -32601 (docs/ACP.md:330-387).
- Durable inbox: methods _reasonix.io/session/inbox/{enqueue|list|get|update|delete|move|setPaused|retry|refresh}
  (protocol.go:494-505), schemaVersion 1. enqueue returns itemId,
  disposition, position, paused, idempotent; list responses contain previews
  and byte counts, never prompt bodies; a recovered inbox is paused
  (docs/ACP.md:389-409). Refusal vocabulary with classes terminal /
  retry-after-repair / do-not-retry (docs/ACP.md:411-447): inbox.not_found,
  inbox.invalid_state, inbox.item_too_large, inbox.capacity_items,
  inbox.capacity_bytes, inbox.paused, inbox.unrecovered, inbox.closed,
  inbox.schema_readonly, inbox.foreign_run, inbox.idempotency_conflict,
  steer.already_applied (all -32020 except retry_may_repeat -32021);
  inbox.empty carries -32602. Not queue refusals: -32602 without data.code
  (unknown sessionId / unparsable params), -32600 (malformed frame), -32603
  (the queue failed rather than refused) (docs/ACP.md:449-454).
- Goal pause/resume: _reasonix.io/session/goal/resume (-32010 when no
  resumable Goal), _reasonix.io/session/goal/pause (-32011 when none is
  running); both return {goal} in the shape of status's goal field,
  publish a _reasonix.io/session/status_update with event goal, send
  current_mode_update when the mode changes, and replace a cancelled/failed
  turn-outcome label with the Goal's own state; goal.status failed labels the
  last turn, not the Goal (docs/ACP.md:456-478).
- MCP status: _reasonix.io/session/mcpStatus (request) and
  _reasonix.io/session/mcpStatus_update (notification), schemaVersion 1;
  snapshot {schemaVersion, sessionId, servers: [{name, status, tools}]};
  statuses ready, connecting, failed, pending, disabled, standby, idle; an
  initial snapshot follows session/new, session/load, session/resume; a
  status read never starts a server or probes an endpoint; failed entries may
  add sanitized error and httpStatus, omitted when absent
  (docs/ACP.md:480-504).
- Status updates opt-in: _reasonix.io/session/status_update and
  _reasonix.io/session/mcpStatus_update go only to a client that declared
  clientCapabilities._meta.reasonix.io.statusUpdates.supported = true; the
  matching requests work either way (docs/ACP.md:292-303).
- Runtime reload: _reasonix.io/session/reloadExtensions
  (sessionReloadExtensionsMethod, protocol.go:557); while a turn or rebuild is
  active exactly one reload is queued ({queued: true}) and runs when the
  session goes idle; otherwise atomic swap; a failed rebuild keeps the
  previous runtime; a successful reload pushes fresh
  available_commands_update (docs/ACP.md:506-518).
- Extension surface: clients that advertise reasonix.io.extensionSurface in
  initialize _meta receive structured extension-surface payloads; clients
  without it receive equivalent text fallbacks (agent_message_chunk for cards
  and statuses, permission requests for extension forms)
  (docs/ACP.md:519-524).
- Authentication: session/new fails -32000 "Authentication required" when the
  selected model has no API key or no provider is configured
  (docs/ACP.md:88-95).

## 8. Dispatch mapping (dispatch.go, toolcall_wire.go, message_ids.go, cancel_request.go)

updateSink is the event.Sink bound to one session: it maps the agent's typed
event stream onto session/update notifications. maxResultChars = 8000 clips a
tool result for DISPLAY only (the full result still reaches the model)
(dispatch.go:32-34).

Event → update mapping (dispatch.go:138-252):

| event.Event | Emitted |
|---|---|
| MCPSurfaceReady | mcpStatus emit (see §9) |
| Reasoning | agent_thought_chunk (empty text skipped) |
| Text | agent_message_chunk with liveMessageID() |
| StreamAttempt | bookkeeping only: begin sets activeAttemptID + clears speculative tool ids; commit/discard clears both. No speculative partial tool cards are published. |
| ToolDispatch | skip Partial and Refreshed; todo_write also emits a plan update; then tool_call pending (below) |
| ToolResult | tool_call_update completed/failed (below) |
| Notice | warn level only → warningChunk; info stays out of band |
| CompactionDone | agent_message_chunk "

[compacted %d earlier messages to save context]" (only when a summary exists) |
| ApprovalRequest | goroutine requestPermission: session/request_permission round-trip |
| AskRequest | goroutine requestAsk: MCP origin → elicit() when the mode is advertised; else per-question session/request_permission |
| ExtensionSurface/ExtensionStatus | emitExtension (below) |

Message/Usage/Phase/TurnStarted/TurnDone have no ACP update and are dropped
(TurnDone surfaces as the session/prompt stopReason) (dispatch.go:36-51).

- tool_call pending: {sessionUpdate:"tool_call", toolCallId, name, title=name, kind=toolKindFor(name), status:"pending", rawInput (omitted unless the args parse as valid JSON), locations} (dispatch.go:188-197, rawJSON dispatch.go:639-644).
- tool_call_update: {sessionUpdate:"tool_call_update", toolCallId, status:"completed"|"failed", content:[{type:"content", content:{type:"text", text: toolResultText(output-or-err, outputDiff)}}]} (dispatch.go:199-216, toolcall_wire.go:5-36).
- Wire shapes (toolcall_wire.go): toolCall {sessionUpdate, toolCallId, name?, title?, kind?, status?, rawInput?, locations?}; ToolCallLocation {path, line?}; toolCallUpdateMsg {sessionUpdate, toolCallId, status?, content?}; toolContent {type:"content", content:ContentBlock}.
- Message ids (message_ids.go:15-48): prefix = 12 hex chars from crypto/rand, drawn once per session open; live id = prefix + "-" + attemptID (no active attempt → no id, so a chunk outside any attempt carries none); replayed id = prefix + "-r" + counter. A retried attempt is a new message.
- $/cancel_request (cancel_request.go): ErrRequestCancelled = -32800; flightSet maps trimmed raw id string → flight{cancel, cancelled}; cancel(params) parses {requestId} — unknown, already-answered, or malformed ids are silently ignored; the cancelled flag forces the -32800 rewrite in server.go:249-251.
- toolResultText (dispatch.go:650-673): a result the host marked as a whole unified diff passes UNTRUNCATED inside a ```diff fence whose backtick count is max(3, longest backtick run in body + 1); any other text is clip()ped.
- clip (dispatch.go:675-686): 8000 chars, backs off to a UTF-8 boundary, appends "
…(<n> more chars truncated)".
- toolKindFor (dispatch.go:692-718): read_file/ls/glob→"read"; grep→"search"; edit_file/move_file/multiedit/write_file→"edit"; bash→"execute"; sandbox-escape→"execute"; then substring heuristics on the lowercased name (contains search|grep|find→search; edit|write|replace→edit; read|cat|view→read; bash|exec|shell|run→execute); default "other".
- Locations (dispatch.go:723-763): only locationTools get follow-along locations — read_file, write_file, edit_file, multi_edit, notebook_edit, delete_range, delete_symbol, code_index. Path is absolutized against the session cwd; read_file offset>0 → line = offset+1 (1-based). Search/list tools are excluded (their path is a scope, not a file).
- Plan mirroring (dispatch.go:768-800): todo_write args {todos:[{content,status,level}]} → plan entries; level 0 → priority "high", else "medium"; statuses pending/in_progress/completed kept, unknown → "pending"; blank contents skipped; empty result → no update. Replay re-sends the latest plan so the client's plan view matches the restored conversation.
- Replay (dispatch.go:352-406, session/load): user → user_message_chunk via sessionstore.UserMessageText (strips injected transient blocks like <response-language>, unwraps memory-compiler contracts; SteerText for steer records; a pure-injection turn replays as nothing); assistant → agent_thought_chunk for reasoning + agent_message_chunk through agent.DisplayAssistantText (goal markers and evidence blocks never reach the client) with replayed ids; tool calls → tool_call "completed" with rawInput + locations; role tool → tool_call_update "completed" with clipped content. The system message is skipped.
- Permission round-trip (dispatch.go:412-451): toolCallId "gate-<approvalId>"; title "<tool> <subject>"; options from approvalOptions; _meta from permissionMeta. Outcome mapping: allow_once → allow; allow_always → allow+session; vendor optionId "allow_persist" (kind allow_always, optAllowPersist) → allow+session+persist; reject, cancel, transport error, or unparsable reply denies the call.
- permissionMeta (dispatch.go:458-496): _meta["reasonix.io"] = {approvalId, tool, subject, fresh, reason?}; for bash with a bound cwd, not run_in_background and not preserve_background_processes, and a parseable input: adds commandSchemaVersion:1 + argv + cwd ONLY when the command is a single static command — fail-closed on shellparse AnalyzeApprovalFeatures DynamicCommandName, NestedExecution, Expansion, Assignment, Redirection, unquoted glob, or any tilde-prefixed argv (shell-dependent). A foreground bash argv a supervisor may trust; anything dynamic stays interactive.
- approvalOptions (dispatch.go:596-632): paid search → allow_once "Allow", allow_always "Allow <subject> (charged per request) for this session", plus allow_persist "Always allow <subject> (charged per request; saved to your settings)" when the host says persist, then reject "Reject"; fresh-or-requires-fresh approval → sandbox escape / network egress / setting tools get allow_once + allow_always + reject, other tools get allow_once + reject only; default → allow_once "Allow" + allow_always (approvalSessionOptionName) + reject "Reject".
- approvalSessionOptionName (dispatch.go:577-592): sandbox escape → "Use real environment for this session"; setting → "Allow changes to <subject> for this session"; network egress → "Allow <subject> for this session"; otherwise permission.SessionGrantRuleForScope(tool, subject) → "Allow <rule> for this session".
- Ask questions without MCP origin (dispatch.go:498-575): title = prompt || header || "Question"; content = header text block when distinct from title; options "<qid>:<n>" kind allow_once named "<label> - <description>" (description optional); trailing "<qid>:cancel" "Cancel" reject_once; toolCallId "ask-<askID>-<qid>"; kind "other"; rawInput {id, question, options, multi}; a selected option id maps back to its label; cancel/unknown/transport failure → nil answers (the ask fails rather than hangs).
- Extension surface dual emission (dispatch.go:261-342): when negotiated, the structured eventwire.ToWireExtensionSurface DTO rides extensionSurfaceUpdate._meta["reasonix.io"].extensionSurface; every client also gets the flattened text fallback as agent_message_chunk — status → "[plugin] label: detail"; card → title + body (markdown wins over text) + "key: value" fields; form → title + message; notification → title + body; lines joined with "
", prefixed "

" and "[warning] " when status/notification severity is warn/error. Blocking form prompts never arrive here (they ride AskRequest → the permission round-trip).


## 9. Vendor status shapes (status.go)

Constants (status.go:20-24): reasonixStatusSchemaVersion = 1;
sessionStatusMethod = "_reasonix.io/session/status"; sessionStatusUpdateMethod
= "_reasonix.io/session/status_update". ReasonixSchemaCapability is the
{schemaVersion}-only capability entry whose map key is the method name.

The status request takes {sessionId}; an unknown session answers -32602
"<method>: unknown session <id>" (status.go:665-675). The request works
without opt-in; the update notification is gated on the client's
clientCapabilities._meta.reasonix.io.statusUpdates.supported declaration
(publishStatus, status.go:677-689).

ReasonixSessionStatus (schemaVersion 1, status.go:121-137) — the stable
recovery snapshot; reasoning text and unbounded terminal output are
intentionally absent:

{ schemaVersion, sequence uint64, sessionId, state, model, effort, mode,
workMode, plannerMode, goal, phase, turnOutcome, finalReadiness, sandbox,
usage }

- state: "running" | "idle" only (anything else coerces to idle,
  status.go:757-760).
- effort: trimmed override or "auto" when unset (status.go:554-559).
- workMode: lowercased runtime profile; "light" dual-writes as "economy";
  unknown → "balanced" (status.go:734-742).
- plannerMode: "on" | "off"; light runtime profiles (economy|light|lite|eco)
  force "off" (status.go:641-648, 743-749).
- goal: {status, objective?, runtime?} — status running|complete|blocked
  else "none"; objective clipped to 16384; a telemetry goalOverride
  (cancelled/failed from a finished turn) overrides the controller status
  (status.go:707-732). ReasonixGoalRuntime {turnsUsed, turnsLimit (deprecated
  0), tokensUsed, requestsUsed?, workDurationMs?, tokensLimit (deprecated 0),
  noProgressTurns, noProgressLimit (deprecated 0), lastReason?, stopCause?,
  budgetExtensions (deprecated 0)}; runtime rides only when an objective
  exists (status.go:713-728).
- phase vocabulary (status.go:604-615 whitelist): idle, starting, planning,
  working, implementing, waiting_permission, waiting_input,
  checking_readiness, cancelled, review_ready, completed, paused,
  readiness_paused, recovery_paused, error.
- turnOutcome: {kind, reason?}; kinds none|cancelled|completed|paused|error.
- finalReadiness: {readyForReview, summary, risks[]}.
- sandbox: {mode, engine, available, workspaceRoot, writeRoots[],
  networkEnabled} — no credentials or environment; writeRoots never null
  (status.go:40-47, 750-752). Default runtime state: engine "bubblewrap"
  ("seatbelt" on darwin), mode "enforce", planner "on" (status.go:537-552).
- usage: {turn, cumulative} of ReasonixUsage {promptTokens,
  completionTokens, reasoningTokens, cacheHitTokens, cacheMissTokens,
  estimated?, cacheHitRatio?, estimatedCost?, currency?, costComplete?,
  displayComplete?, displayStatus?, aggregateMode?, originalTotals?, costQuote?,
  usageSource} (status.go:95-112). Cost aggregation prefers the pricing
  quote ledger (costQuote + originalTotals) when every priced event has
  quotes, else the summed estimatedCost; usageSource defaults to the
  executor source, "mixed" when sources disagree (status.go:147-287).

Update envelope (status.go:139-145, 677-689): {schemaVersion, sequence,
sessionId, event, status}.

Telemetry state machine (status.go:289-396):

- beginTurn: state "running", phase "starting", turnOutcome none, fresh turn
  usage, risks cleared.
- onEvent → publish: Phase → phase event (source planner → "planning",
  executor → "implementing", else text heuristic planning/implementing/
  working, status.go:586-602); Usage → "usage" event accumulating turn +
  cumulative; ApprovalRequest → phase "waiting_permission"; AskRequest →
  phase "waiting_input"; ToolDispatch → phase "implementing" without
  publishing.
- finishTurn (status.go:356-396) event names: cancelled → "completion" with
  phase "cancelled" + outcome cancelled + goal override "cancelled"; goal
  complete / no goal / stopped → "completion" with phase "review_ready" or
  "completed", outcome completed, readyForReview true; goal blocked →
  "pause" with phase "paused" + outcome paused "goal blocked"; run error →
  "error" with phase "error" + outcome error (reason clipped 2048) + goal
  override "failed"; goal incomplete → "pause" with outcome paused "goal is
  not complete".
- clipStatusText applies secrets.Redact then trims then clips (16384 summary,
  2048 reason/risks) (status.go:570-584).
- Persistence and restore (status.go:409-514): persistedStatusTelemetry round-
  trips under those JSON names. A restored session never owns the turn that
  wrote a "running" snapshot: state is forced to idle; a running snapshot
  additionally bumps the sequence, sets phase "recovery_paused", outcome
  paused "previous turn interrupted", readyForReview false. Unknown persisted
  phases renormalize through the text heuristic.
- finalAssistantSummary (status.go:787-798): the last non-empty assistant
  message in history.


## 10. Vendor session-info, MCP-control, and goal methods

### 10.1 Session info (session_info.go)

Constants (session_info.go:10-14): sessionInfoSchemaVersion = 1;
sessionInfoMethod = "_reasonix.io/session/info"; sessionMCPServersMethod =
"_reasonix.io/session/mcpServers". Both registered with conn.Handle
(session_info.go:61-64).

- {sessionId} params; unknown session → -32602 "<method>: unknown session <id>".
- ReasonixSessionInfo {schemaVersion, sessionId, label, modelRef, workspaceRoot, scope, agentPreset, autoApproveTools, cache} (session_info.go:18-28); cache {hitTokens, missTokens, hitRatio?} — ratio only when hit+miss > 0 (session_info.go:32-36, 66-79). Fields come from the controller: Label, ModelRef, WorkspaceRoot, CapabilityScope, AgentPreset, AutoApproveTools, SessionCache.
- ReasonixMCPServers {schemaVersion, sessionId, servers[]}; ReasonixConfiguredServer {name, enabled, localOverride, pending, changed, tools int} — reports NO command, argument, or environment (session_info.go:46-59). Optional {sessionId, name?} filter: a name filter also resolves the enabled flag through MCPServerEnabled (session_info.go:93-121).

### 10.2 MCP control (session_mcp_control.go)

Constants (session_mcp_control.go:12-26): sessionControlSchemaVersion = 1;
sessionMCPReconnectMethod = "_reasonix.io/session/mcp/reconnect";
sessionMCPDisconnectMethod = "_reasonix.io/session/mcp/disconnect";
sessionExtensionSubmitForm = "_reasonix.io/session/extension/submitForm";
states mcpControlReady "ready" / mcpControlFailed "failed" /
mcpControlApprovalOwed "approval_owed" + code "mcp.approval_owed".

- Reconnect (session_mcp_control.go:96-116): {sessionId, name} → ReasonixMCPReconnect {schemaVersion, sessionId, name, state, tools?, code?, launch?, error?}. Approval owed → state "approval_owed" + code + launch (the command the server would run — a project-declared server the user has not approved is never started, only shown); failure → "failed" + error; success → "ready" + tool count. Guarded by stateChangeMu; publishMCPStatus always follows.
- Disconnect (session_mcp_control.go:120-130): {sessionId, name} → {schemaVersion, sessionId, name, disconnected} — session-only; nothing is written to configuration or user activation choices.
- submitForm (session_mcp_control.go:132-148): {sessionId, pluginId, surfaceId, values map}; blank pluginId or surfaceId → -32602 "<method>: pluginId and surfaceId are required"; controller failure → -32603; success → {schemaVersion, sessionId, accepted: true}.
- namedMCPServer guard (session_mcp_control.go:77-91): unparsable params, unknown session → -32602; blank name → -32602 "<method>: name is required".

### 10.3 Goal pause/resume (session_goal.go)

- Methods (session_goal.go:15-26): sessionGoalResumeMethod = "_reasonix.io/session/goal/resume", sessionGoalPauseMethod = "_reasonix.io/session/goal/pause", sessionGoalSchemaVersion = 1; codes ErrGoalNotResumable = -32010, ErrGoalNotRunning = -32011.
- Resume (session_goal.go:62-79) re-enters a retained Goal with its objective, counters, todos, and delivery checkpoint; it does NOT start a turn — the next session/prompt continues it. On success it also clears plan mode (SetPlanMode(false)) and the goal draft mode. No resumable goal → -32010 "<method>: session has no resumable goal".
- Pause (session_goal.go:81-93) takes effect at the next Goal continuation boundary; the current turn is left to session/cancel. Nothing running → -32011 "<method>: session has no running goal".
- Both return SessionGoalResult {goal: ReasonixStatusGoal} — the status snapshot's goal right after the change (session_goal.go:35-37, 97-107): the telemetry goalOverride (cancelled/failed label) is cleared, mode drift emits a current_mode_update, and a status update with event "goal" is published.
- goalCapabilities (session_goal.go:44-48) registers both methods in initialize _meta at schemaVersion 1. runningGoal(c) returns the objective only while GoalStatus() == running.

### 10.4 Session metadata sidecar (session_meta.go)

- ACP sessions keep metadata beside the transcript: acpMetaPath strips the transcript extension and appends ".acp.json" (session_meta.go:193-198).
- acpSessionMeta (metaLocked, session_meta.go:48-62): SessionID, Cwd, Model, EffortOverride, RuntimeProfile, ToolApprovalMode (kept posture or normalized mode), CollaborationMode (normalized), Title, CreatedAt, UpdatedAt, Status (persistedStatusTelemetry, §9), ActiveTranscript (redirect sidecar marker).
- Save (session_meta.go:117-155): MarshalIndent two-space + trailing newline; temp file in the same dir; atomic ReplaceFile; parent mkdir 0755. saveMetaIfPresent writes only when a transcript file exists.
- Load (session_meta.go:98-115): UTF-8 read; missing file → not-ok, no error; decode error surfaces as "decode ACP session metadata <path>: ...". metadataForLoadedSession (session_meta.go:64-96) backfills: id, cwd override, title from the first user message, CreatedAt/UpdatedAt from file mtime when the meta file is absent.
- listACPMetas (session_meta.go:157-191): scans the session dir for *.acp.json; skips cleanup-pending sessions, missing transcripts, and metas without Cwd; id comes from the filename.
- listMetaBeats (session_meta.go:200-211): a live transcript (no ActiveTranscript redirect) always beats a redirect sidecar; between the same kind, the later UpdatedAt wins.
- Titles (session_meta.go:213-231): first user message, whitespace-normalized, ≤80 runes, else first 77 + "...". parseSessionUpdatedAt uses RFC3339Nano.
- persistedSessionMeta (session_meta.go:241-257): cleanup-pending id → -32602 "unknown session"; decode failure → -32603.

### 10.5 Service wiring and registration (service.go)

- Factory interface (service.go:33-35): NewSession(ctx, SessionParams) (*control.Controller, error); the returned controller owns its cleanup (Close stops MCP subprocesses). Optional capability interfaces a Factory may also implement: SessionConfigStateProvider.SessionConfigState (model/effort/work-mode selectors), SessionDirProvider.SessionDir, SessionRebuilder.RebuildSession (boot.Rebuild path; absent → reloadExtensions reports unavailable).
- Serve() registration table (service.go:100-125), the complete method surface:
  - conn.Handle: initialize, authenticate, session/new, session/load, session/resume, session/prompt, session/steer (_reasonix.io/session/steer), the nine inbox methods (enqueue/list/get/update/delete/move/pause/retry/refresh, §10.9), sessionReloadExtensionsMethod, registerGoalMethods (goal/resume + goal/pause), registerSessionHealthMethods (status, sessionInfoMethod, sessionMCPServersMethod, §9/§10.1), session/set_config_option, session/set_model, session/set_mode, session/close, session/list, session/delete.
  - conn.HandleNotify: session/cancel only.
  - defer svc.closeAll(); return conn.Serve(ctx).
- afterResponse (service.go:148-159): result + AfterResponse() hook — session-opening notifications run only after the transport wrote the result, so a client can register the session before its first update.
- acpController driving port (service.go:192-208): AgentCompositions, Lifecycle, TurnControl, RunTurnWithRaw(ctx, input, raw, invokedSkill), RunSkillWithName, TrySteer, Approvals, SlashDispatch, MCPControl, Extensions, SessionPersistence, Goals, ContextSnapshot.
- Session id rules (service.go:1422-1456): validateSessionID — trimmed non-empty, no surrounding whitespace, not "." or "..", charset [a-zA-Z0-9._-] only; violations → -32602 "method: missing sessionId" / "method: invalid sessionId". newSessionID = random RFC 4122 v4 UUID, lowercase hex, 8-4-4-4-12 (service.go:1548-1556).
- transcriptPath = dir/id.jsonl (service.go:664-666), id-keyed, distinct from the CLI's timestamp-labelled chat files. resolveTranscriptPath (service.go:676-698) follows the id-keyed sidecar's ActiveTranscript redirect: basename only, must stay inside dir, target must exist and claim the same session id; anything else falls back to the id-keyed path.
- mcpSpecs (service.go:1494-1536): type defaults "stdio"; stdio requires command; "http"/"streamable-http"/"streamable_http" normalize to "http", "sse" stays "sse", both require url; unsupported transport → error "MCP server %q uses unsupported transport %q"; blank name → "MCP server name is required"; Dir and WorkspaceRoot are set to the session cwd.
- closeAll (service.go:1175-1185): on connection end abort every in-flight turn (abortAndWait), ctrl.Close(), releaseSessionLease().
- Session teardown on delete: deleteSessionFiles (service.go:1452-1468) removes transcript + .acp.json sidecar (store.RemoveSessionArtifacts), the checkpoint dir (RemoveAll), delegation subagents by parent (sessionstore.BranchID), job artifacts, and clears cleanup-pending. ReconcileCleanupPending (service.go:1472-1476) retries delayed cleanup left by a previous process.

### 10.6 Session state machine and leases (service.go, session.go)

- acpSession fields (service.go:213-275): id, ctrl, sink, transcript, cwd, mcpServers, additionalDirs, model, effortOverride (*string; nil = config wins, non-nil empty = provider default), runtimeProfile, posture, runtimeState, status, modeID, goalDraftMode, pendingConfig (one queue entry per axis), pendingReload (coalescing flag), title, createdAt, updatedAt; mu + stateChangeMu (serializes rebuilds vs collaboration/approval changes), cancel/done/running/deleted, lease, retiredLeases, maintenanceDone.
- begin (service.go:277-301): TryLock stateChangeMu — prompt admission is non-blocking; refuse when running, deleted, maintenance in flight, or pendingConfig non-empty (a queued config blocks new turns so a prompt never runs on the outgoing config).
- finish/abort/abortAndWait/deleteAndWait (service.go:303-357): abort cancels the turn context; abortAndWait also waits done and maintenanceDone; deleteAndWait marks deleted first.
- Session lease refusals (session.go:27-74): ErrSessionLeaseHeld = -32022 data {code: "busy.session_in_use"} message includes control.SessionInUseMessage + SessionLeaseCloseHint; ErrSessionLeaseUnprovable = -32023 data {code: "session.lease_unprovable"} (no holder observed — client must not be told to close a window); other lease errors → -32603 "method: session lease: ...". sessionLeaseAcquireRefusal re-probes with the store's three-state ownership check.
- sessionOpenError (session.go:79-85): provider.ErrKeyMissing or provider.ErrNoProvider → -32000 ErrAuthRequired (the "authentication required" error clients key on to offer setup); anything else → -32603.
- Write authority (session_write_authority.go:16-52): bindACPWriteAuthority issues a generation-bound write authority via control.Controller.BindSessionWriteAuthority(lease); test fakes without the method stay unbound. resumeACPControllerForWrite refuses while a turn is in flight (that turn owns the session it writes to). snapshotACPController uses SnapshotRewrite for concrete controllers.
- prepareACPReplacementAuthority (session_write_authority.go:54-75): binds the lease to the replacement, snapshots with rollback — on snapshot failure it rebinds the current controller and re-snapshots, returning errors.Join.
- sessionRecoveredHandler (session_write_authority.go:83-150): on snapshot-conflict recovery, acquires the recovery-path lease, rebinds authority, moves sess.lease + sess.transcript, saves meta, retires the old lease, and writes an id-keyed redirect sidecar (ActiveTranscript = basename of recovery path) so restart-time lookups follow the session.
- retiredLeases (service.go:397-432): lease Release is deferred to a goroutine when called inside a recovery callback (synchronous release would deadlock on the save executing the callback); waitForRetiredSessionLeases blocks until all complete.

### 10.7 Session lifecycle methods (session.go)

- sessionNew (session.go:91-211): params optional; resolveSessionCwd (explicit abs cwd > stored meta cwd > os.Getwd); mcpSpecs; vetAdditionalDirs; sessionConfigState + startingPosture + withToolApprovalConfig; sessionRuntimeState; newSessionID; sink bound (bindCwd, bindExtensionSurface, bindElicitation); SessionParams with NewConversation: true and OnSessionRecovered; bindClientIO (fs overlay / host terminal from client capabilities); factory.NewSession → newSessionError; ctrl.EnableInteractiveApproval + SetToolApprovalMode(startPosture); sink.bindApprove/bindAnswer; startMode = startingModeID (plan when the controller starts in plan mode, else normal); acpSession built; transcript lease acquired with the id-keyed path and SetFreshSessionPath; registered; afterResponse → sendAvailableCommands + publishMCPStatus. Result: SessionNewResult{sessionId, models, modes, configOptions} with extension catalog models folded in (composition.go:198).
- sessionModesState (session.go:222-231): available modes exactly: Normal "Work directly and pause when user input is required"; Plan "Research and propose a plan before making changes"; Goal "Keep advancing the next prompt as a goal until complete or blocked".
- sessionSetMode (session.go:235-290): holds stateChangeMu. normal → SetPlanMode(false)+ClearGoal; plan → ClearGoal+SetPlanMode(true); goal → SetPlanMode(false); legacy "default" → posture-ceiling check at ask, nextMode normal, legacyApproval ask; legacy "auto" → ceiling check at yolo, legacyApproval yolo; unknown → -32602 "session/set_mode: unknown modeId <id>". goalDraftMode armed when goal && GoalStatus != running. Legacy approval also calls SetAutoApproveTools + configOptionUpdate. current_mode_update emitted only when the mode actually changed (swapModeID). saveMetaIfPresent. Result SessionSetModeResult{} (empty object).
- sessionLoad / sessionResume (session.go:297-347): both via openExistingSession (replay=true/false); result{models, modes, configOptions}; after: sendAvailableCommands + sendUsageUpdate + publishMCPStatus. Modes come from the live session when it exists (sessionModesFor) so a reconnecting client's picker is not reset.
- openExistingSession (service.go:486-659): live-session branch — cleanup-pending transcript → -32602 "unknown session"; different additionalDirs → -32602 data{code: "acp.additional_directories.session_open"}; replay via a fresh sink. Persisted branch — persistedSessionMeta; config state resolved from saved overrides with fallback to defaults; newPersistedSession; SessionDir required else -32603 "persistence is disabled"; resolveTranscriptPath; cleanup-pending → -32602 "unknown session"; TryAcquireSessionLease → sessionLeaseAcquireRefusal; LoadSession; resumeACPControllerForWrite; restoredPosture (kept posture or normalized mode); mode restore: plan → SetPlanMode(true); goal → SetPlanMode(false) + goalDraftMode when not running; default → goal running ? goal : normal; metadataForLoadedSession; saveACPMeta (failure → -32603, lease released, ctrl closed); replay when asked. Returns config state enriched with extension models.
- sessionPrompt (session.go:353-427): promptText flattens blocks (§3); empty → -32602 "session/prompt: empty prompt"; resolveSlashPrompt (§10.5); begin() refusal → -32600 "session/prompt: session already has an active prompt"; status.beginTurn + publishStatus "phase"; sink.setTurnContext; goalDraftMode consumed → SetGoal(text)+saveMetaIfPresent; run under defer clearTurnContext + finishTurn + cancel; drainACPInbox(runPrompt(...)) — follow-up inbox items drain with the same sink inside the turn boundary; status.finishTurn(runErr, cancelled, GoalStatus, finalAssistantSummary) → publishStatus + sendUsageUpdate; persistAfterTurn (snapshot + title + updatedAt + saveACPMeta). Stop mapping: ctx cancelled → stopReason "cancelled"; other error → -32603 message clipStatusText(err, 2048) data{code: "turn_failed"}; success → "end_turn". vendor meta.reasonix.io.transcriptPath present when a transcript is pinned.
- sessionSteer (session.go:432-474): durable path when the controller is an EditorAPI with a session path: TryEnqueueAndSteer{Intent: steer, Source: "acp"} → result{itemId, disposition}; enqueue error → -32600. Fallback (pathless test controllers): ctrl.TrySteer → disposition "steer_accepted" or -32600 "session has no active prompt".
- sessionCancel (session.go:713-721): notification, no reply ever; decode errors and unknown sessions silently ignored.
- sessionClose (session.go:572-586): validateSessionID; takeSession → abortAndWait + ctrl.Close + releaseSessionLease; unknown session is a no-op (idempotent cleanup).
- sessionList (session.go:590-649): params optional; cwd filter must be absolute; non-empty cursor → -32602 "session/list: unsupported cursor" (unpaged by design). Merges: sidecar metas (listACPMetas) reduced to one representative per id by listMetaBeats, then live sessions override by id; cwd filter is filepath.Clean equality. Sort: UpdatedAt (RFC3339Nano) descending, tie-break SessionID ascending.
- sessionDelete (session.go:653-709): validateSessionID; live session → deleteAndWait + releaseSessionLease + BeginDestroySession + Wait; timeout → MarkCleanupPending (its failure → -32603) + delayed delete in background; CloseAfterDestroy; deleteSessionFiles for the resolved path; then the id-keyed twin path is deleted too (a recovered session lives in two files — the recovery transcript and the id-keyed redirect — the ghost would resurface in session/list). Deleting a missing session succeeds silently.
- Mode/approval drift reconciliation (service.go:445-484, 869-887): emitModeDrift and emitToolApprovalDrift serialize on stateChangeMu and emit current_mode_update / config_option_update only on change; finishTurn reconciles drift BEFORE exposing the session as idle (a concurrent config switch could otherwise rebuild from stale state), then drains pending config and pending reload, then re-reconciles.
- reloadExtensions (service.go:700-848): factory must implement SessionRebuilder else -32600 "runtime reload is unavailable in this session"; busy → coalesce exactly one pendingReload, result{queued: true}; snapshot before rebuild; prevPath captured after Snapshot (a conflict can retarget the controller); boot.Rebuild via the factory; EnableInteractiveApproval; runtime state recomputed; prepareACPReplacementAuthority "snapshot after reload"; swap under mu (deleted → -32600; ctrl changed → busy-retry error); saveACPMeta; bindCompositionIO; the outgoing controller is ReleaseResources'd, NOT closed — the session logically continues, so SessionEnd hooks must not fire; sendAvailableCommands. Result SessionReloadExtensionsResult{} (or queued variant).
- resolveSlashPrompt (service.go:1332-1354): for "/"-prefixed prompts only — CustomCommand, then RunSkillWithName, then MCPPrompt, then invokeExtensionAction ("/<plugin>:<action>" against handshake-declared actions, service.go:1362-1386); a miss leaves the line untouched. Returns (expandedText, invokedSkillName).
- availableCommandsFor (service.go:1246-1330): non-hidden commands + slash skills + MCP host prompts + extension actions, deduped by name (commands first), sorted by name; input hints from ArgHint / skill hint / "arguments".

### 10.8 Config switching (session_config.go)

- normalizeConfigID (session_config.go:559-572) — accepted aliases: "models"→model; "reasoning_effort"/"thought_level"→effort; "profile"/"runtime_profile"/"token_mode"→work_mode; "approval"/"approval_mode"/"tool_approval_mode"→tool_approval; anything else trimmed as-is.
- configOptionCategory (session_config.go:583-599): explicit option.Category wins; else by normalized id: model→"model", effort→"thought_level", work_mode→"work_mode", tool_approval→"tool_approval".
- session/set_config_option dispatch (session.go:497-545): resolve current config state; unknown configId → -32602 "unknown config option <id>"; tool_approval values additionally ceiling-checked; value not in option.Options → -32602 "invalid value <v> for <id>"; then by category: agent_composition → SelectAgentComposition; model → switchSessionModel; thought_level → switchSessionEffort; work_mode/agent_preset → switchSessionRuntimeProfile; tool_approval → switchSessionToolApproval; anything else → -32602 "unsupported config option <id>". Result{configOptions}.
- session/set_model (session.go:549-562): switchSessionModel; model unavailable → -32600 "session/set_model: model switching is unavailable in this session" (when the resolved model is empty).
- sessionConfigDelta axes (session_config.go:889-907, 120-129): "model", "thought_level", "work_mode"/"agent_preset" (both → RuntimeProfile); tool approval never rebuilds the controller.
- Queueing contract (session_config.go:70-107): pendingConfig holds at most one entry per axis (last-write-wins within an axis, merges never drop other axes); a queued config blocks new prompts until drained.
- switchSessionConfig (session_config.go:264-338): resolve → TryLock stateChangeMu; when maintenance is in flight, queue the deltas and still return the resolved state plus an immediate config_option_update (the drain re-resolves later); after locking, re-resolve (the baseline may be stale); sessionConfigMatchesRuntime drops queued deltas whose axes already match the running generation; effortAppliedInPlace short-circuits; else rebuildSessionLocked, then applyPendingSessionConfig + drainPendingReload, and refresh the returned state so an older response cannot overwrite the newer update.
- switchSessionRuntimeProfile (session_config.go:196-257): in-place when idle, no rebuild: TryLock fail → -32600 "session config: session is busy; retry when idle"; deleted → -32600; PendingPrompt → "answer pending prompts before switching execution setting"; running → "finish or cancel the active turn before switching execution setting"; BackgroundJobs > 0 → "stop background jobs before switching execution setting". SetAgentPreset; label normalization light/economy/eco/lite→"economy", delivery/deliver/quality→"delivery", else "balanced"; PlannerMode off for economy, on otherwise; saveMetaIfPresent + configOptionUpdate.
- Rebuild pipeline (service.go:909-1105): busy checks (PendingPrompt / BackgroundJobs / running / maintenance) queue instead of failing; axes claimed and maintenanceDone raised in one critical section; snapshot first; prevPath and carried history captured AFTER snapshot (conflict may retarget); the fresh controller's leading system message is spliced into carried[0] so the model sees the target profile's contract; AdoptHistory(carried, prevPath); re-apply all three axes (tool approval, plan/goal mode, running goal); InheritLifecycleFrom + RestoreSessionAuthorizations (a rebuild must not force re-approving session-granted tools or re-trusting plan-mode commands); persist before publishing (failure leaves the outgoing controller fully usable); swap under mu; saveACPMeta; ReleaseResources the old controller; sendAvailableCommands + configOptionUpdate.
- applyPendingSessionConfig (session_config.go:359-435): loop claiming the queue under stateChangeMu; re-resolve against live state each iteration (never replay a stale baseline); on resolution failure with no maintenance, drop the claimed axes so the session stays promptable; loop while requests queue during builds.
- withToolApprovalConfig (session_config.go:515-547): option id "tool_approval", name "Tool Approval", type "select"; choices in order: read_only "Read only" ("Refuse every call that is not a read"), dont_ask "Don't ask" ("Refuse permission-gated calls instead of asking"), ask "Ask" ("Ask before permission-gated tool calls"), auto "Auto" ("Follow configured permission rules without fallback prompts"), yolo "Yolo" ("Approve tool calls except protected decisions"); options below the posture ceiling are filtered out.
- normalizeACPToolApprovalMode (session_config.go:489-502): case-insensitive auto/yolo/read only/don't ask; default ask. normalizeACPCollaborationMode: plan/goal; default normal.
- switchSessionToolApproval (session_config.go:340-357): blocking stateChangeMu; posture-ceiling check (aboveCeilingError); ctrl.SetToolApprovalMode + sess.setToolApprovalMode + saveMetaIfPresent + configOptionUpdate. No rebuild.
- session/set_mode legacy ids (session.go:249-273): "default"/"auto" cross into the tool-approval axis and are ceiling-checked at ask/yolo respectively (see §10.7).

### 10.9 Inbox surface (inbox.go)

- JSON-RPC classes (inbox.go:17-20): ErrInboxRefused = -32020, ErrInboxMayHaveRun = -32021; refusal data{code} reuses the HTTP frontend's dotted identities.
- inboxRefusal mapping (inbox.go:40-78) — the complete refusal vocabulary: steer.already_applied, inbox.item_too_large, inbox.capacity_items, inbox.capacity_bytes, inbox.invalid_state, inbox.foreign_run, inbox.paused, inbox.unrecovered, inbox.not_found, inbox.idempotency_conflict, inbox.schema_readonly, inbox.closed → all -32020; inbox.retry_may_repeat → -32021 (the one answer a client must never retry blindly); inbox.empty → -32602; anything else → -32603 (a fault, not a refusal).
- enqueue (inbox.go:95-140): text trimmed, empty → -32602 inbox.empty; intent followup (default) | steer (EqualFold); Source "acp"; idempotency key passed through; steer → TryEnqueueAndSteer, followup → EnqueueInbox kept passive (ACP owns the blocking session/prompt response boundary — the prompt handler drains follow-ups with the same sink). Result: {itemId, disposition, position, paused, idempotent}.
- list → api.InboxSnapshot() verbatim; get → {meta, envelope}; update → meta (display/raw/submit all set to the same trimmed text); delete/move/retry/refresh → {ok: true}; setPaused → {paused}.
- sessionAPI (inbox.go:80-93): controller must expose control.EditorAPI else -32603 "session controller does not expose inbox"; EnsureSessionPath called when available.

### 10.10 Composition binding (composition.go)

- SessionParams (composition.go:18-32): AgentComposition, Composition, Cwd, MCPServers, AdditionalDirs, Sink, Model, EffortOverride, RuntimeProfile, OnSessionRecovered, NewConversation, FileOverlay, Terminal.
- newPersistedSession (composition.go:41-52): binding from the persisted transcript's Composition(); when the factory fails but a binding exists and the error is not cancellation, boot.BuildCompositionHistory builds an error-session controller carrying the history — the client still gets a usable session that reports the failure.
- withCompositionConfig (composition.go:65-97): appends config option id "agent_composition", name "Agent composition", type "select"; default current "builtin:general" ("Standard"); locked when the composition started, a prompt is pending, background jobs run, or the binding reports Started — locked collapses options to the current one with description "This conversation keeps its bound version. Create a new session to use another composition."; unlocked description "Choose a composition before the first prompt. Models are configured independently."
- switchComposition (composition.go:107-151): TryLock stateChangeMu → agentcomposition.ErrLocked; refuses when deleted/running/maintenance in flight/controller changed; rebuilds via factory with cloned params; ApplyRuntimeMigration; EnableInteractiveApproval; prepareACPReplacementAuthority "snapshot Agent composition"; swap; rebind sink; sendAvailableCommands; ReleaseResources the old controller.

### 10.11 Additional directories (additional_dirs.go)

- Limits: maxAdditionalDirectories = 32, maxAdditionalDirLength = 4096.
- vetAdditionalDirs (additional_dirs.go:38-67): each declared folder gets the same vetting a user-granted write root gets (config.VetExtraRoot), one refusal for the whole request — a folder dropped silently would leave the client believing the agent can write where it cannot; dedups resolved real paths; result is this session's alone (nothing written to the user's grants).
- Refusal data: AdditionalDirsData{code, path?}; codes: acp.additional_directories.too_many, .session_open (open with different dirs — close first), .disabled (factory started write-only-in-cwd), plus config.WorkspaceRootError codes per folder (e.g. RootNotDirectory for over-long paths); all -32602 via additionalDirsError.
- newSessionError (additional_dirs.go:83-88): ErrAdditionalDirectoriesDisabled → codeAdditionalDirsOff; otherwise sessionOpenError.

### 10.12 Warning notices (notice_meta.go)

- warningUpdate (notice_meta.go:7-13): an agent_message_chunk whose metadata carries {notice: {code, payload?}}; content is textBlock("\n\n[warning] " + event.Text) — the two leading newlines are part of the contract.
- warningChunk (notice_meta.go:22-38): payload present only when event.DetailIsPayload(e.Code); no code → no metadata at all.


## 11. Elicitation (elicitation.go)

- Limits: elicitMessageLimit = 1024 runes, elicitValueLimit = 4096 runes.
- Modes are read once from client capabilities (bindElicitation):
  ElicitationCapabilities is clientCapabilities.elicitation with raw-JSON
  "form"/"url" members; a mode counts only when its key is present and not
  null (present(), elicitation.go:216-219). FormMode()/URLMode() implement
  the rule (elicitation.go:222-226).
- elicit (elicitation.go:30-44): no origin → decline; URL origin → url mode
  only when url is advertised and there is exactly one question; form origin →
  form mode only when form is advertised and there is at least one question.
- elicitation/create params: url mode {sessionId, mode:"url", elicitationId:"elicit-<askId>", url, message}; form mode {sessionId, mode:"form", message, requestedSchema} (no elicitationId). Reply {action, content: map[string]raw} (elicitReply, elicitation.go:46-58).
- url reply: action "accept" or "cancel" → answer {questionId, selected:[action]}; anything else declines (elicitation.go:79-92).
- form reply: action must be "accept"; answers are re-validated agent-side
  (elicitation.go:150-208): each value must be a JSON string or array of
  strings; every value must be one of the offered labels; !multi with >1
  value declines; any value over 4096 runes declines; a violation declines
  the WHOLE form rather than forwarding something the person did not choose.
- requestedSchema (elicitation.go:107-141): {type:"object", properties}; per question: title clipPlain(header||id, 120); description clipPlain(prompt, 1024); with options and multi → {type:"array", items:{type:"string", enum:labels}, default?}; with options → {type:"string", enum:labels, default?first}; without options → {type:"string", maxLength:4096, default?first}. The form is the server's; nothing is added.
- elicitMessage (elicitation.go:62-77): "MCP server <source> asks: <message>", control characters (except newline) and category-Cf runes removed, invalid UTF-8 dropped, capped at 1024 runes with a trailing "…".

## 12. Extraction provenance

Every product file in internal/frontend/acp/ (34 files) has been read in full;
each is accounted for below with the section that carries its contract.

| Reference file | Section(s) |
|---|---|
| server.go | §0 |
| protocol.go | §1, §3, §5 |
| handshake.go | §2 |
| clientio.go | §4 |
| dispatch.go | §8 |
| toolcall_wire.go, message_ids.go, cancel_request.go, gate.go, toolresult.go | §8 |
| status.go | §9 |
| session_info.go, session_mcp_control.go, session_goal.go, session_meta.go | §10.1–§10.4 |
| service.go | §10.5, §10.6, §10.7 |
| session.go | §10.6, §10.7 |
| session_config.go | §10.8 |
| inbox.go | §10.9 |
| composition.go | §10.10 |
| additional_dirs.go | §10.11 |
| notice_meta.go | §10.12 |
| elicitation.go | §11 |
| docs/ACP.md | §6, §7 |

Not extraction targets: *_test.go (40 files), go.mod (module reasonix, stdlib-only ACP surface).

## 13. Divergences and open decisions

Behaviors where the wire contract is subtle, undocumented in docs/ACP.md, or
needs an explicit decision for the Rust port. None is a sanctioned break; all
default to byte-compatible reproduction.

1. **warningChunk leading newlines.** The notice content block is textBlock("\n\n[warning] " + Text) (notice_meta.go:29). The two leading newlines are part of the user-visible rendering; the Rust port must reproduce them exactly, not "clean them up".
2. **Legacy set_mode ids cross axes.** session/set_mode accepts "default"/"auto" as aliases that adjust the tool-approval posture (ceiling-checked at ask/yolo), while "normal"/"plan"/"goal" select the collaboration mode (session.go:249-273). docs/ACP.md documents only the mode semantics. Keep both surfaces; isolate the legacy path in code so the Rust port cannot accidentally widen the ceiling.
3. **Undocumented configId aliases.** normalizeConfigID accepts eight aliases beyond the canonical ids (models, reasoning_effort, thought_level, profile, runtime_profile, token_mode, approval, approval_mode, tool_approval_mode — session_config.go:559-572). All must be accepted by the Rust resolver.
4. **session/list is unpaged by design.** A non-empty cursor is rejected with -32602 "session/list: unsupported cursor" (session.go:598). Do not add pagination in the Rust port.
5. **Rebuild pipeline ordering is behavioral, not stylistic.** The snapshot must be taken before prevPath capture (conflicts retarget the controller); the fresh controller's system message is spliced into carried[0]; RestoreSessionAuthorizations + InheritLifecycleFrom preserve grants; the old controller is ReleaseResources'd, never Closed (SessionEnd hooks must not fire mid-session); meta is persisted before publishing the swap (service.go:909-1105). Violating any step corrupts recovery or leaks MCP subprocesses.
6. **Inbox asymmetry.** steer enqueues durably (TryEnqueueAndSteer, result carries itemId/disposition) while followup is passive (EnqueueInbox) and drains inside the prompt handler's turn boundary with the same sink (inbox.go:95-140). The Rust port must keep the followup drain inside the turn, or steering/prompts interleave differently than reference.
7. **EffortOverride nil vs empty.** SessionParams.EffortOverride nil = config wins; non-nil empty string = provider default requested explicitly (session.go:117-126). A Rust Option<String> with None/Some("") maps 1:1; collapsing the two states is a behavior change.
8. **Error clipping.** session/prompt errors clip at 2048 bytes (clipStatusText, session.go:420-424); tool result text clips at 8000 chars UTF-8-safe (§8). Both limits are wire-visible and must match.
9. **Sequential notification, concurrent processing.** The dispatcher delivers session/update notifications in event order but does not serialize handler bodies (dispatch.go:31-40); consumers must be prepared for concurrent handler execution. The Rust actor port should preserve ordered dispatch while allowing concurrent sink work.
10. **ACP storage is id-keyed, CLI storage is time-keyed.** ACP sessions persist as dir/<id>.jsonl + dir/<id>.acp.json while CLI chats use timestamp-labelled files (§10.4). Both live in the same history root; the Rust port must not conflate them in listing/filtering code.

## 14. Proposed ledger rows

| ID | Area | Reference | Rust | Status | Evidence |
|---|---|---|---|---|---|
| ACP-001 | ACP | NDJSON framing + 32MiB line cap (server.go) | reasonix-acp | todo | M1: docs/contracts/acp.md §0 |
| ACP-002 | ACP | Error code table + cancelled rewrite (server.go, protocol.go) | reasonix-acp | todo | M1: docs/contracts/acp.md §1 |
| ACP-003 | ACP | initialize capabilities JSON + _meta capability schema | reasonix-acp | todo | M1: docs/contracts/acp.md §2 |
| ACP-004 | ACP | authenticate (reasonix-setup only, else -32602) | reasonix-acp | todo | M1: docs/contracts/acp.md §2 |
| ACP-005 | ACP | Inbound wire types round-trip (SessionNew..SessionDelete params/results) | reasonix-acp | todo | M1: docs/contracts/acp.md §3 |
| ACP-006 | ACP | fs/read_text_file + fs/write_text_file degradation asymmetry | reasonix-acp | todo | M1: docs/contracts/acp.md §4 |
| ACP-007 | ACP | terminal create/wait_for_exit/output/release + 1MiB cap | reasonix-acp | todo | M1: docs/contracts/acp.md §4 |
| ACP-008 | ACP | session/update variants (messageChunk..currentModeUpdate) | reasonix-acp | todo | M1: docs/contracts/acp.md §5 |
| ACP-009 | ACP | Permission request round-trip + option kinds | reasonix-acp | todo | M1: docs/contracts/acp.md §5 |
| ACP-010 | ACP | Session lifecycle, lease codes, controls, permission ceiling | reasonix-acp | todo | M1: docs/contracts/acp.md §6 |
| ACP-011 | ACP | Vendor extensions opt-in (steer/inbox/goal/status/…) | reasonix-acp | todo | M1: docs/contracts/acp.md §7 |
| ACP-012 | ACP | Event→update dispatch mapping + flightSet cancel | reasonix-acp | todo | M1: docs/contracts/acp.md §8 |
| ACP-013 | ACP | Tool call/update wire shapes + message ids | reasonix-acp | todo | M1: docs/contracts/acp.md §8 |
| ACP-014 | ACP | Permission gating, argv fail-closed, toolResultText clip | reasonix-acp | todo | M1: docs/contracts/acp.md §8 |
| ACP-015 | ACP | session/status method + ReasonixSessionStatus v1 | reasonix-acp | todo | M1: docs/contracts/acp.md §9 |
| ACP-016 | ACP | Usage accounting + finishTurn usage event | reasonix-acp | todo | M1: docs/contracts/acp.md §9 |
| ACP-017 | ACP | session/info + session/mcpServers | reasonix-acp | todo | M1: docs/contracts/acp.md §10.1 |
| ACP-018 | ACP | MCP control reconnect/disconnect/submitForm states | reasonix-acp | todo | M1: docs/contracts/acp.md §10.2 |
| ACP-019 | ACP | goal/resume + goal/pause semantics | reasonix-acp | todo | M1: docs/contracts/acp.md §10.3 |
| ACP-020 | ACP | .acp.json sidecar schema + write rules | reasonix-acp | todo | M1: docs/contracts/acp.md §10.4 |
| ACP-021 | ACP | elicitation/create + form/url limits + capability gating | reasonix-acp | todo | M1: docs/contracts/acp.md §11 |
| ACP-022 | ACP | Serve registration table (27 Handle + session/cancel notify) | reasonix-acp | todo | M1: docs/contracts/acp.md §10.5 |
| ACP-023 | ACP | acpSession state machine: begin/finish/maintenance/retired leases | reasonix-acp | todo | M1: docs/contracts/acp.md §10.6 |
| ACP-024 | ACP | session/new full flow + lease + afterResponse | reasonix-acp | todo | M1: docs/contracts/acp.md §10.7 |
| ACP-025 | ACP | session/load + session/resume replay semantics | reasonix-acp | todo | M1: docs/contracts/acp.md §10.7 |
| ACP-026 | ACP | session/prompt turn flow + inbox drain + stop mapping | reasonix-acp | todo | M1: docs/contracts/acp.md §10.7 |
| ACP-027 | ACP | session/steer durable + fallback paths | reasonix-acp | todo | M1: docs/contracts/acp.md §10.7 |
| ACP-028 | ACP | session/cancel silent notification | reasonix-acp | todo | M1: docs/contracts/acp.md §10.7 |
| ACP-029 | ACP | session/close idempotent teardown + closeAll | reasonix-acp | todo | M1: docs/contracts/acp.md §10.7 |
| ACP-030 | ACP | session/list merge + sort + cursor rejection | reasonix-acp | todo | M1: docs/contracts/acp.md §10.7 |
| ACP-031 | ACP | session/delete + cleanup-pending + twin deletion | reasonix-acp | todo | M1: docs/contracts/acp.md §10.7 |
| ACP-032 | ACP | session/set_mode incl. legacy default/auto | reasonix-acp | todo | M1: docs/contracts/acp.md §10.7 |
| ACP-033 | ACP | session/set_config_option dispatch + aliases | reasonix-acp | todo | M1: docs/contracts/acp.md §10.8 |
| ACP-034 | ACP | Config rebuild pipeline ordering invariants | reasonix-acp | todo | M1: docs/contracts/acp.md §10.8 |
| ACP-035 | ACP | tool_approval option: 5 choices + ceiling filtering | reasonix-acp | todo | M1: docs/contracts/acp.md §10.8 |
| ACP-036 | ACP | Inbox methods + refusal map + enqueue intents | reasonix-acp | todo | M1: docs/contracts/acp.md §10.9 |
| ACP-037 | ACP | agent_composition option + switch + error-session | reasonix-acp | todo | M1: docs/contracts/acp.md §10.10 |
| ACP-038 | ACP | Write authority binding + recovery handler + redirect sidecar | reasonix-acp | todo | M1: docs/contracts/acp.md §10.6 |
| ACP-039 | ACP | additionalDirectories vetting + refusal codes | reasonix-acp | todo | M1: docs/contracts/acp.md §10.11 |
| ACP-040 | ACP | Warning notices wire shape | reasonix-acp | todo | M1: docs/contracts/acp.md §10.12 |
| ACP-041 | ACP | session/reloadExtensions coalescing + rebuild | reasonix-acp | todo | M1: docs/contracts/acp.md §10.7 |
| ACP-042 | ACP | Session id validation + UUID v4 generation | reasonix-acp | todo | M1: docs/contracts/acp.md §10.5 |
| ACP-043 | ACP | MCP spec conversion + transport normalization | reasonix-acp | todo | M1: docs/contracts/acp.md §10.5 |
| ACP-044 | ACP | availableCommands + slash resolution order | reasonix-acp | todo | M1: docs/contracts/acp.md §10.5 |
| ACP-045 | ACP | Mode/approval drift reconciliation ordering | reasonix-acp | todo | M1: docs/contracts/acp.md §10.6 |
| ACP-046 | ACP | Divergences/open decisions review | reasonix-acp | todo | M1: docs/contracts/acp.md §13 |
