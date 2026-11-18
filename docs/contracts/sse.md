# SSE Event Contract — Frozen Inventory (M1)

> **STATUS: FROZEN** from source on 2026-11-15. Area: the typed event stream the kernel emits — event kinds, wire JSON, numbering/replay semantics, notice codes, sink pipeline.
> Reference: `reference/DeepSeek-Reasonix-studio` (Go). Primary sources: `internal/contract/event/**` (27 product files), `internal/contract/eventwire/**`, `internal/frontend/serve/broadcaster.go`.
> Rust crate target: `reasonix-core` (vocabulary) / `reasonix-serve` (broadcaster).
> Transport framing (SSE wire format, keepalive, Last-Event-ID resume, /events/replay, events mux) is frozen in docs/contracts/http.md §5 — this document owns the event VOCABULARY and NUMBERING, and does not repeat the transport.
> Oracle priority: actual runtime behavior > tests > source > this file. Divergences are recorded in §9 and go to `docs/parity/BREAKS.md` when they affect users.

## 0. Scope and sources

The Go kernel emits `event.Event` values to an `event.Sink` (event.go:695-697). Frontends implement sinks; the server wraps one `Broadcaster` (broadcaster.go:19) that fans out to SSE subscribers. The JSON a client ever sees is `eventwire.Event` (wire.go:16-89) produced by `eventwire.ToWire` (wire.go:139-239). The typed enum `event.Kind` NEVER crosses the wire — only the string names in `eventwire/kinds.go`.

## 1. Event kind taxonomy (36 kinds, wire-stable)

Order = `event.Kind` iota order (event.go:27-168). Wire names from `eventwire/kinds.go:25-62`. New kinds append at the end; older clients ignore unknown kinds. "Numbered" below = assigned `seq` by the broadcaster (§3).

| # | Kind | Wire name | Payload fields | Numbered |
|---|---|---|---|---|
| 1 | TurnStarted | `turn_started` | Text (the submitted message), AuthoredTurn, MsgIndex, Via, Attachments, ModelRef | yes |
| 2 | Reasoning | `reasoning` | Text (delta) | no (droppable) |
| 3 | Text | `text` | Text (delta) | no (droppable) |
| 4 | Message | `message` | Text (full answer), Reasoning (full CoT), ThoughtMs, MemoryCitations, ModelRef | yes |
| 5 | ToolDispatch | `tool_dispatch` | Tool: ID/Name/Args/ReadOnly, Partial, ArgChars, Refreshed, Diff/Added/Removed, Profile, ParentID, Issuer, AttemptID, Deliveries, ResolvedName/CapabilityID | yes |
| 6 | ToolResult | `tool_result` | Tool: Output/Err/RefusalCode/Bound, Images, Videos, DurationMs, StartedAt/EndedAt, Execution, OutputDiff, ReadOnly, ContextTokens | yes |
| 7 | Usage | `usage` | Usage, CostQuote (+cost aliases), UsageSource, AttemptID, CacheDiagnostics, SessionCacheHit/Miss, ModelRef, Source | yes |
| 8 | Notice | `notice` | Text, Detail, Code, Level, Audience, WorkspaceLease, DecisionReceipt | yes |
| 9 | Phase | `phase` | Text (label, e.g. "deepseek · planning") | yes |
| 10 | ApprovalRequest | `approval_request` | Approval (ID/Tool/Subject/Reason/ReasonCode/RawInput/Scope/Fresh/AllowsSession/AllowsPersist/Kind/Recovery/Diff) | yes |
| 11 | AskRequest | `ask_request` | Ask (ID/Questions/Origin/SkipInMs); NonPersistable=true when the ask carries a URL | yes |
| 12 | TurnDone | `turn_done` | Err (string), Cancelled, Outcome, Receipt, CheckpointTurn, WorkDurationMs, WorkStartedAt | yes |
| 13 | CompactionStarted | `compaction_started` | Compaction (Trigger + scale about to fold) | yes |
| 14 | CompactionDone | `compaction_done` | Compaction (full; aborted pass: empty Summary) | yes |
| 15 | ToolProgress | `tool_progress` | Tool.ID + Tool.Output (the new chunk) | no (droppable), EXCEPT sub-agent status channel (§3) |
| 16 | MCPSurfaceReady | `mcp_surface_ready` | Text = "<server>: <what changed>"; consumer re-reads /mcp | yes |
| 17 | Retrying | `retrying` | RetryAttempt, RetryMax, RetryScope ("headers"|"stream"), RetryCause, RetryStatus, RetryDelayMs, RetryTimeoutSecs | yes |
| 18 | Steer | `steer` | Text (raw steer content), Via, ItemID, HostAuthored, Attachments | yes |
| 19 | GuardianAssessment | `guardian_assessment` | Guardian (ID/Tool/Subject/Outcome/RiskLevel/UserAuthorization/Rationale/DurationMs/Usage) | yes |
| 20 | ExtensionSurface | `extension_surface` | Extension payload (one of Status/Card/Form/Notification/Panel/View) | yes |
| 21 | ExtensionStatus | `extension_status` | Extension payload (Status set) | yes |
| 22 | StreamAttempt | `stream_attempt` | StreamAttempt {id, action: begin|discard|commit, attempt, max, reason, firstTokenMs} | yes |
| 23 | ContextMaintenanceEvent | `context_maintenance` | Maintenance (ContextMaintenance struct) | yes |
| 24 | TodoProgressEvent | `todo_progress` | TodoProgress {kind, steps, completed, contentRevision, planRevision, progressRevision} | yes |
| 25 | WorkspaceLeaseEvent | `workspace_lease` | WorkspaceLease (contended/wait/held accounting) | yes |
| 26 | WorkspaceChanged | `workspace_changed` | Workspace {revisions, changes, allPaths, source, watchState} | yes |
| 27 | TurnPhase | `turn_phase` | Phase = working|checking|verifying|reviewing (falls back to Text when empty) | yes |
| 28 | CompletionSummary | `completion_summary` | Completion (content-free quality summary) | yes |
| 29 | CompactionProgress | `compaction_progress` | Text (digest chunk; a frame with NO Text restarts the digest) | no (droppable) |
| 30 | InboxChanged | `inbox_changed` | content-free: re-read the queue from the kernel | yes |
| 31 | GraphDelta | `graph_delta` | Graph (agentgraph.Delta — nodes declared/settled + typed edges) | yes |
| 32 | AdjudicationsChanged | `adjudications_changed` | content-free: re-read the journal | yes |
| 33 | BrowserTabsChanged | `browser_tabs_changed` | content-free: re-read tabs | yes |
| 34 | ProgressWatchEvent | `progress_watch` | ProgressWatch (Stalled/Cause/IdleRounds/RoundLimit/PromptTokens/TokenLimit/TokenMultiple/Pausing) | yes |
| 35 | RewindChanged | `rewind_changed` | content-free: re-read the undo offer | yes |
| 36 | AskEngaged | `ask_engaged` | Ask.ID only (person took the question over) | yes |

Transport-synthetic kinds (eventwire.Event JSON with a `kind` string, NOT `event.Kind` members — broadcaster.go:258, 308):
- `stream_watermark` {kind, seq} — keepalive + attach marker; a client resumes from its seq.
- `stream_gap` {kind, seq} — first trustworthy seq after a replay gap; everything before it must come from the transcript.

Fixed vocabularies:
- TurnPhase names (event.go:174-177): working, checking, verifying, reviewing.
- StreamAttempt actions (event.go:184-186): begin, discard, commit. Reasons (event.go:199): connection_reset, premature_eof, idle_timeout. IDs are host-local only — never persisted or sent to the model.
- RetryScope (event.go:194-195): headers, stream (empty = older emitter).
- Notice Level (event.go:223-225): info (0), warn (1). Wire renders lowercased strings.
- Notice Audience (event.go:235-237): empty (default — every frontend) or operator (runtime maintenance; must not be forwarded as end-user chat).
- TurnOutcome constants (event.go:212-217): no_progress (progress-watch pause), recovery_paused (legacy recorded sessions only — nothing emits it any more; readers keep rendering it as informational).
- ToolIssuer (tool_issuer.go:13-22): model (from a response's tool_calls), host (host-minted), user (slash command / inline shell), provider (provider-side execution, no dispatch). Separate axis from Event.Source (which model produced the frame). ModelWork() = model only; AssistantWork() = model + provider.
- Sub-agent progress channels (subagent_progress.go:18-23), reserved prefix reasonix.subagent.: .status (phase values queued/running/reasoning/responding/tool/retrying/completed/failed/cancelled in Tool.Output), .reasoning, .text, .notice. Never provider-visible tool names; keyed by Tool.ID (child task card ID); Tool.DurationMs rides terminal status.

## 2. Wire envelope (eventwire.Event, wire.go:16-89)

JSON object; every field omitempty except kind:

```text
kind, nonPersistable, text, detail, code, reasoning, thoughtMs, memoryCitations,
level, audience, tool, usage, approval, ask, compaction, maintenance, todoProgress,
progressWatch, workspaceLease, guardian, decisionReceipt, extension, err, cancelled,
outcome, receipt, checkpointTurn, workDurationMs, workStartedAt, source, authoredTurn,
msgIndex, via, attachments, hostAuthored, modelRef, retryAttempt, retryMax, retryScope,
retryCause, retryStatus, retryDelayMs, retryTimeoutSecs, streamAttempt, itemId,
workspace, phase, completion, graph, seq
```

Key semantics:
- seq (wire.go:85-88): set by the transport on frames a client must not miss; streaming deltas carry none — the same reason SSE omits id: on a frame that should not move Last-Event-ID.
- nonPersistable: live delivery and prompt replay only; never written to a durable event log (set for URL asks, wire.go:166).
- source: which producer wrote the frame (executor, planner, sub-agent) — a turn can carry two models.
- modelRef: which model wrote this; empty = the turn's own.
- externalizable:"true" marks large text payloads (text, detail, reasoning, args, output, err, summary, description, rationale, diff) the Remote protocol may offload via content refs without changing provider-visible semantics.
- err on turn_done is e.Err.Error() — a plain string; JSON field is err, NOT error (unlike the HTTP Reason envelope).
- Attachments (attachment.go): [{path}] — images stored under .reasonix/attachments, workspace-relative, slash-separated.

## 3. Numbering, droppable frames, replay (broadcaster.go)

- droppable(e) (broadcaster.go:44-52): ToolProgress (except the sub-agent status channel reasonix.subagent.status), Text, Reasoning, CompactionProgress. These get NO seq, are NOT held for replay, and a slow subscriber LOSES them — a drop degrades the view; a numbered frame's drop breaks it.
- Emit (broadcaster.go:113-169): droppable → broadcast unnumbered; everything else → seq++, wire.Seq set, added to the replay log, then fanned out. NEVER blocks; a marshal failure is dropped silently; one bad event must not stall the stream.
- Seq is TRANSPORT-scoped, not session-scoped (broadcaster.go:26-29): numbering outlives /new and /resume, so a resume across a switch is told to rebuild rather than handed another conversation's frames under numbers it already has. ResetSession() (broadcaster.go:82-93) clears the usage ledger and DROPS the replay tail but keeps counting.
- Usage with CostQuote adds to the session pricing ledger; executor-sourced first quote seeds cold-start usage (broadcaster.go:151-165). TurnDone with WorkDurationMs>0 accumulates workMs. Notice display_currency rebinds the ledger display currency (broadcaster.go:145-147).
- gapFrame (broadcaster.go:303-313): on an incomplete replay, a stream_gap frame with the first seq the client can trust goes out BEFORE the missed frames.
- Replay(after) (broadcaster.go:287-298): frames since after + complete flag; serves request-shaped transports (desktop bus) at the same log SSE resumes from.
- Watermark() = last numbered frame (broadcaster.go:276-280) — the client compares it against what it saw to notice a lost turn-ending frame no later frame would reveal.
- Replay budget 8MiB, live queue budget 4MiB (eventstream.go:100-108; framing in http.md §5).

## 4. Subscription modes (broadcaster.go:192-271)

- Subscribe() = SubscribeFrom(0): from here on, no history.
- SubscribeFrom(after) (SSE reconnect with Last-Event-ID): replay what the log still holds, gap-announced when incomplete, BEFORE the live stream — no seam.
- SubscribeInitialNotices(): attach without a cursor; only the FIRST such window for a session receives pre-connection NOTICES, as UNNUMBERED copies (seq stripped, broadcaster.go:232-247) — skipped non-notice history must not look like a gap.
- SubscribeAttached(): initial notices + an unnumbered stream_watermark ahead of every later frame — a client with that number resumes from it.
- SubscribeResume(after): a window that stated its own position; cursor 0 means it holds no numbered frame and is owed the whole log (SubscribeFrom keeps 0 as "no cursor").
- EmitTo(target, e) (broadcaster.go:175-190): per-subscriber delivery, unnumbered by definition — connection-local recovery only.

## 5. Sink pipeline and capabilities

- Sink interface (event.go:695-697): Emit(Event). Serial by contract — the agent run loop emits one at a time; implementations need not be concurrent-safe.
- Coalesce(inner, window) (coalesce.go:28-36): merges bursts of PURE streaming deltas — Text, Reasoning, CompactionProgress (Text payload) and ToolProgress (ID+Output) that are otherwise zero-valued events (streamDelta probe, coalesce.go:68-91: with source/payload cleared the event must DeepEqual a zero Event of the same kind). First delta of a burst forwards immediately (TTFT unchanged — gated by lastForward + window, coalesce.go:116-121); later deltas buffer at most window (default 16ms, DefaultStreamDeltaWindow coalesce.go:21), flushing earlier on any non-delta event (total order preserved), a key switch, or the 16KiB merged-delta cap (coalesceMaxBytes coalesce.go:15). deltaKey (coalesce.go:59-63) = {kind, source, toolID} — two sources never merge; text, reasoning and one tool's progress never merge into another's.
- Sync(inner) (sync.go:17-22): serializes concurrent Emit — background jobs (internal/tools/jobs) emit from their own goroutines and can overlap a running turn.
- NoticeLedger (notice_ledger.go:8-93): remembers which notices a lineage's previous build showed (identity = level+code+text+detail); a rebuild re-deriving the same notice says nothing. Begin(sink) opens a NoticeWindow; Close(true) replaces the set (a notice that disappears and returns is shown again); Close(false) merges (old runtime keeps serving). After Close the window forwards unchanged.
- CostQuoteSink (costquote_sink.go:110-141): fills CostQuote on Usage and GuardianAssessment before forwarding; frontends must consume e.CostQuote and must not call Pricing.Cost for aggregation. Notice display_currency updates QuoteContext.SetDisplay. EnsureCostQuote (costquote_sink.go:158-190): no Pricing → estimated/incomplete no_price quote {Estimated, CostComplete:false, DisplayStatus:unavailable, IncompleteReason:"no_price"}; with Pricing → pricing.BuildQuote with billing mode (config-resolved BillingModeForModel, else token-plan heuristic on ModelRef/Source), catalog provider (CatalogProviderForModel by endpoint host), display request from QuoteContext (explicit currency, else auto), occurred-at clock. Guardian usage is spelled as a Usage event (guardianUsageEvent, costquote_sink.go:146-155) so reviews and turns price by one code path.
- MarkEmbeddedDiffs(sink, enabled) (markdiff.go:10-26): tags a finished shell ToolResult whose whole output is a unified diff with OutputDiff — only when Execution != nil, Err empty, Bound not lossy, not already tagged. Computed once at the shared sink so the tool loop, synthetic calls, and sub-agents are tagged alike.
- Optional sink capabilities — delivery by TYPE ASSERTION; a wrapper missing one silently drops that channel. Registry capabilityContracts (sink_capabilities.go:10-18): DelegationAudit, MemoryRecall, RunBudget, TurnCompletion, UsageQuote, WorkspaceMutation, UsageScope. AuditForwarder (audit_forwarder.go:10-32) forwards all of them for wrappers; hand-written forwarding lost capabilities at multiple wrappers even while tests stayed green.
- Host-only audit channels, never provider-visible: RunBudgetSample {turn, task, currency?} (run_budget.go:8-25; RunBudgetTotals = rounds, requests, promptTokens, outputTokens, cost, priced, elapsedMs; shadow not enforced; Task spans every Run continuing the same work), WorkspaceMutation {ToolID, ToolName, Paths, AllPaths, Content, Tree, WorkingTree, GitMeta} (workspace_mutation.go:8-17 — invalidation hint incl. through hooks; neither proof of write nor delivery receipt; independent of ToolResult ordering; failed writers still qualify), MemoryRecallAudit (record.go:24-45 — identifiers/scores/budget only, never query or fact text: hits [{ID, Revision, Scope, Type, Freshness, Score}], charBudget, usedChars, omitted, omittedByLimit, omittedByBudget, suppressed "<code>: <sentence>", suppressionCode, shadow ranking).
- Discard drops everything; FuncSink adapts a function (event.go:699-711). TurnCompletionSink.RecordTurnCompletion (event.go:686-688) serves synchronous controller entry points that publish no TurnDone UI event.
- TaskLabel (task_label.go:10-24): names a sub-agent from its own task — first non-empty line of description else prompt, trimmed, rune-capped at MaxTaskLabel 120 (119 runes + "…").

## 6. Notice codes (wire-stable vocabulary, notice_codes.go:9-152)

Frontends localize by code and fall back to English text when empty/unknown. Values are wire-stable: never rename or reuse once shipped. Full registry:

```text
session_tree_changed, checkpoint_recording_failed, turn_identity_mismatch,
plan_auto_approved, empty_final, executor_handoff, tool_budget, prompt_queued,
loop_guard, perseveration_loop, perseveration_retry, progress_guard, evidence_nudge,
reasoning_governor, workspace_lease, hook_blocked, hook_warned, hook_failed,
hook_unevaluable, cancelled_turn_display, unapplied_steer, session_recovery_forked,
session_recovery_adopted, session_recovery_adopted_covered, session_recovery_depth_cap,
session_shutdown_recovery_forked, verification_stalled, setting_changed, setting_open,
ask_timeout_invalid, decision_receipt, context_editing_fallback,
workspace_lease_resumed, workspace_lease_abandoned,
permission_saved, permission_covered, permission_save_failed,
suspected_injection, context_report, project_programs_awaiting_approval,
project_program_changed, project_mcp_awaiting_approval, project_mcp_changed,
mcp_tools_held, mcp_tools_followed, mcp_scope_upgrade, peer_message_arrived,
session_continued_from_1x, approval_mode_unrecognized, permission_rules_dormant,
input_refused, await_user, goal_retry, goal_retry_gave_up, egress_auto_allowed,
await_timed_out, await_gave_up, unknown_command, display_currency,
language_overridden, default_model_unavailable, inherited_subagent_effort_dropped,
branch_model_unavailable, memory_migration_backup, compacted, compact_declined,
compact_failed, compact_held, context_composition, subagent_waiting, extension_skipped,
inbox_recovered, inbox_unrecovered, compaction_telemetry, user_turns_dropped,
kept_turns_folded, job_started, job_finished, job_killed, job_failed, job_stalled,
web_search_model_bad_ref, web_search_model_not_added, web_search_model_removed,
web_search_model_unsupported, web_search_model_no_credentials,
web_search_backend_fallback, web_search_backend_unusable,
web_fetch_backend_fallback, web_fetch_backend_unusable, composition_tools_changed,
fold_not_reused_history, fold_not_reused_lineage
```

Notable semantics (notice_codes.go comments): peer_message_arrived is operator-only (a window re-reads the session list, nothing drawn for the person); mcp_tools_held / mcp_tools_followed Detail lists the tools; mcp_scope_upgrade Detail is the server name; egress_auto_allowed Detail is the host; display_currency Detail is the stored value (empty = auto); compacted / compact_declined / compact_failed / compact_held are /compact's own codes (Detail = the decline/failure/held code); project_programs_awaiting_approval / project_mcp_awaiting_approval hold programs until approved (Detail = launch line for the latter); job_* notices carry JobNotice payloads (§7).

## 7. Notice Detail payloads (notice_payload.go)

DetailIsPayload(code) (notice_payload.go:7-11) — true for: unapplied_steer, extension_skipped, inbox_recovered, inbox_unrecovered, permission_rules_dormant, subagent_waiting, context_composition, compaction_telemetry, user_turns_dropped, kept_turns_folded, job_started, job_finished, job_killed, job_failed, job_stalled. For these, Detail is the typed JSON payload its sentence is worded from — a sink that prints Text must not append it again.

| Code | Detail JSON schema | Decode guard |
|---|---|---|
| extension_skipped | {extension, point, reason}; reason no_live_sidecar | extension != "" |
| inbox_recovered | {count} | count > 0 |
| inbox_unrecovered | {dir} | dir != "" |
| job_* | {kind, id, label?, error?, exitCode?, signal?, quietSeconds?, steps?} | id != "" |
| permission_rules_dormant | {rules: [{list, rule, tool}]} | — |
| context_composition | {total, at, active, parts: [{name, tokens}], largest?: [{role, tool?, tokens, active?}]} | — |
| subagent_waiting | {task?, by?, whole?} | — |
| compaction_telemetry | {trigger, mode, cache_state?, source_tokens, fold_tokens, spans, projection_tokens, input_tokens, output_tokens, cache_hit_tokens, cache_miss_tokens, cache_write_tokens, request_count, user_turns_kept, user_turns_dropped, provider_request_id?, error?}; mode/trigger are identifiers (summarized/degraded/native/snip; pressure/manual/overflow/snip/tool), never sentences | mode != "" |
| user_turns_dropped / kept_turns_folded | {count, tokens, budget} | — |

Decode rule for every payload: ok=false for a Detail that is not the payload — the caller keeps the kernel's English.

## 8. Sub-struct schemas (wire JSON)

- Tool (wire.go:448-500): {deliveries, id, name, args, argumentNormalization, resolvedName, capabilityId, output, images, videos, err, refusalCode, workspaceLease, outputDiff, readOnly, truncated, bound, durationMs, rate, contextTokens, startedAt, endedAt, partial, argChars, refreshed, parentId, issuer, attemptId, diff, added, removed, profile, execution}. name and readOnly are the only non-omitempty fields. truncated is the pre-Bound compatibility projection; bound is current. rate = RateTokensPerSec (sub-agent stream estimate, never billed). ContextTokens() (tool_cost.go:10-17) = estimate of args+output+err left in the prompt; 0 on partial dispatches (args still streaming). Profile {name?, count?, model?, effort?, label?} — non-nil Profile marks a delegation; Label set only for a single sub-agent. Videos [{name, size, durationMs?, model}] — never bytes. Issuer values per §1 (model|host|user|provider); must be identical on a call's dispatch and result.
- Bound (bound.go:8-27, wire.go:414-426): kind spilled (moved to file; Path kept; Output is the pointer the model got) | windowed (leading window of a paged read; rest one call away) | truncated (middle discarded; KeptBytes only; Lossy()=true). Wire: {kind, lines?, bytes?, keptBytes?, path?}. One named sub-state, not independent flags.
- Usage (wire.go:539-575): {promptTokens, completionTokens, totalTokens, cacheHitTokens, cacheMissTokens, reasoningTokens?, estimated?, source?, attemptId?, cacheDiagnostics?, sessionCacheHitTokens, sessionCacheMissTokens, contextPromptTokens?, contextCompletionTokens?, contextReasoningTokens?, contextCacheHitTokens?, contextCacheMissTokens?, cost?, currency?, currencyCode?, costUsd?, costQuote?, costComplete?, displayComplete?, displayStatus?, aggregateMode?, originalTotals?}. costUsd is a compatibility alias mirroring cost (selected display valuation, NOT USD). Usage sources (event.go:544-564): executor, planner, subagent, compaction, classifier, title, capability-router, recovery-reviewer, goal-evaluator, prompt-refine, commit-message, injection-screen, advisor, best-of, best-of-judge, deepseek-search, deepseek-fetch, side. Empty = executor (compatibility).
- CacheDiagnostics (cache_diagnostics.go:7-20, wire.go:578-591): prefix/system/tools/body hashes, logRewriteVersion, toolSchemaTokens, cache hit/miss tokens, carriedMessages; prefixChangeReasons from: system, tools, log_rewrite, body_unreported.
- Compaction (event.go:464-481, wire.go:345-364): trigger auto|manual; boundary capacity|economic; triggerTokens; messages, summary, sourceTokens, projectionTokens, coverageRequired/coverageMissing/coverageBackstopped, code, remains (ContextComposition).
- ContextMaintenance (event.go:485-503): {status?, action?, trigger?, operationId?, inputTokens?, resultTokens?, savedTokens?, affectedToolResults?, projectionVersion?, cacheBreak?, reason?, code?, boundary?, triggerTokens?} — wire-safe receipt for snip/prune/noop/blocked; transcript bytes are hashes and counts.
- TodoProgress (event.go:507-514): {kind, steps?, completed?, contentRevision?, planRevision?, progressRevision?} — structural verdict + counters that move independently.
- ProgressWatch (progress_watch.go:7-16): stalled, cause (rounds|tokens|perseveration), idleRounds, roundLimit, promptTokens, tokenLimit, tokenMultiple, pausing. Advisory — the model never sees it; pauses nothing unless pausing says so.
- WorkspaceLease (workspace_lease.go:6-17): {contended, reported?, waitedMs?, heldMs, idleMs, holder?, holderSessionId?, paths?, requestedPaths?, repository?} — Paths = blocking extent; RequestedPaths = granted/requested extent (workspace root for whole-workspace claims).
- Guardian (wire.go:594-604): {id, tool, subject, outcome, risk_level?, user_authorization?, rationale?, duration_ms?, usage?} — snake_case here, see §9. Outcome allow|deny; risk_level low|medium|high|critical.
- Approval (event.go:360-406): ID/Tool/Subject/Reason/ReasonCode/RawInput (exact structured tool input for ACP permission clients)/Scope/Fresh (current decision required; no remembered grants)/AllowsSession/AllowsPersist/Kind ("tool"|"plan"|"recovery"; empty = tool for backward compat)/Recovery (RecoveryApproval: sourceAgent, failedTool, failedSummary, diagnosis, nextTool, nextAction, changeKind same_strategy|strategy|scope|risk|uncertain, changeRationale, reviewRationale, planBefore, planAfter, canGrantTask, taskGrantScope) + FileDiff preview (Diff/Added/Removed).
- Ask (event.go:409-456, wire.go:377-400): {id, questions: [{id, header, prompt, reason, options: [{label, description?}], multi, default?, surface?, callId}], origin?, skipInMs?}. Reasons (only two legal values): user_decision | missing_value. Origin kinds: mcp (AskOrigin: kind, source, message?, note?, url?; wire adds urlHost/urlLocal). skipInMs = wait left before the host skips.
- ExtensionSurfacePayload (extension.go:27-124): {code, pluginId, surfaceId, sessionId, generation, kind, status?|card?|form?|notification?|panel?|view?} — exactly one sub-struct set. Kinds: status, card, form, notification, panel, view ("request" reserved — blocking prompts route through AskRequest). Mirrors Extension Protocol v2 UI DTOs field-for-field; structured-only, no HTML/CSS/JS/URLs; credential-redacted before emit. StatusView {label, detail, severity info|warn|error, progress?}; CardView {title, markdown, text, fields [{key, value}], progress?, actions [{actionId, label}]}; FormView {title, message, fields [{key, label, kind confirm|input|select|multiselect, options, default, required}]}; NotificationView {title, body, severity}; PanelView {title, text, fields, progress?, actions}; ViewSurface {slot, anchor ("tool:<callId>" only), body: ExtensionViewNode[]} (node type = surfacespec.Node; Tone says what a node means, not its color).
- CompletionSummary (completion.go:8-47, wire.go:92-103): {preset, verdict, mutations, checks_passed, checks_failed, checks_suppressed, review, gap_kinds?, criteria_rewritten?}. Verdicts complete|partial|blocked|continue; review none|passed|warned|failed|unavailable; presets light|balanced|delivery. NeedsAttention(): partial/blocked verdict, warned/failed/unavailable review, checksFailed>0, checksSuppressed>0, gaps, or rewritten criteria. Blocked() = verdict blocked. Content-free by construction.
- CompletionReceipt (receipt.go:7-88): {verdict, changes: [{path, reviewed}], verifications: [{command, passed, stale, inconclusive}], gaps: [{kind, detail}], risks, unverified}. Gap kinds: stale_verification, inconclusive_verification, unreviewed_change, declared_unverified, mod_effects_unobserved, outside_observed_root (these = notice severity; unknown kinds = warn — an unfamiliar finding must not drop to a whisper). SaysSomething(): gaps/risks/unverified non-empty or verdict "done". Severities: "" | notice | warn.
- DecisionReceipt (wire.go:285-299): {id, kind, tool?, subject?, outcome, via?} — provider-excluded user decision.
- WorkspaceChanged (wire.go:105-125): {revisions: {content, tree, workingTree, gitMeta, session}, changes: [{path, oldPath?, op}], allPaths, source, watchState}. watchState: active|degraded|unavailable.
- MemoryCitation (wire.go:316-323): {id?, source, lineStart?, lineEnd?, note?, kind?} — entries with empty source+id+note are dropped by ToWire.
- Via (wire.go:302-313): {device, ordinal} — the paired device a message came from; nil = the window.
- StreamAttempt wire (wire.go:128-136): {id, action, attempt?, max?, reason?, firstTokenMs?}.
- Attachment: {path} — image under .reasonix/attachments, workspace-relative, slash-separated.

## 9. Divergences and open notes

1. Guardian wire fields are snake_case (risk_level, user_authorization, duration_ms, wire.go:599-602) while every other payload is camelCase — a wire-stable quirk to reproduce exactly, not fix.
2. truncated (wire Tool) duplicates bound.kind == truncated for journals written before Bound existed — keep both on the wire.
3. err on turn_done vs error field on the HTTP Reason envelope: same word, different JSON keys, both frozen.
4. stream_watermark / stream_gap are transport-synthetic kinds that exist only in eventwire JSON, not in the typed enum — a Rust port should model them as transport frames, not event kinds.
5. NoticeLedger dedup (§5) means a notice repeated by a rebuild is intentionally silent — parity tests must not expect it twice.
6. Coalescing means delta frame COUNTS are timing-dependent and not stable across runs; golden corpora must compare merged text, not frame boundaries. First-delta immediacy and the 16KiB cap ARE stable.
7. recovery_paused outcome: legacy recorded sessions only — the Rust reader must keep rendering it, the writer must never emit it.
8. Event ordering guarantee: total order through the sink pipeline (Coalesce flushes before any non-delta event; Sync serializes) — SSE frame order is exactly emit order.
9. The first sse.md write attempt (single 30K-char template literal with escaped backticks) failed TS lexing ("Expected ',', got '<lexing error>'"); the doc was written in two chunks using a placeholder character for markdown backticks instead — escaping every backtick in one giant literal is fragile; split and substitute.

## 10. Proposed ledger rows

| ID | Behavior | Section |
|---|---|---|
| SSE-001 | 36-kind taxonomy: names, order, payload fields, numbered/droppable | §1 |
| SSE-002 | Transport-synthetic kinds stream_watermark / stream_gap | §1, §3 |
| SSE-003 | Fixed vocabularies: phases, attempts, retry scopes, levels, audiences, issuers, sub-agent channels | §1 |
| SSE-004 | eventwire.Event envelope: field set, omitempty, externalizable marks | §2 |
| SSE-005 | droppable() set + seq assignment + never-blocking Emit | §3 |
| SSE-006 | Transport-scoped seq: survives /new; ResetSession drops replay tail | §3 |
| SSE-007 | Replay log, gap frame, watermark, Replay(after) | §3 |
| SSE-008 | Five subscription modes incl. initial-notices unnumbered replay | §4 |
| SSE-009 | Coalesce: 16ms window, 16KiB cap, first-delta immediate, order preserved | §5 |
| SSE-010 | Sink capability registry + AuditForwarder completeness | §5 |
| SSE-011 | CostQuoteSink quoting rules + no_price fallback | §5 |
| SSE-012 | NoticeLedger dedup across rebuilds | §5 |
| SSE-013 | MarkEmbeddedDiffs tagging rules | §5 |
| SSE-014 | Notice code registry (~100 codes) + localization fallback | §6 |
| SSE-015 | Notice Detail payload schemas + DetailIsPayload set + decode guards | §7 |
| SSE-016 | Tool wire struct incl. Bound, truncated compat, Profile, Execution | §8 |
| SSE-017 | Usage wire struct + usage source constants + CostQuote aliases | §8 |
| SSE-018 | Approval + Ask payloads (reasons, origin kinds, surfaces) | §8 |
| SSE-019 | Extension surface payload (6 kinds, one sub-struct set) | §8 |
| SSE-020 | CompletionSummary + CompletionReceipt + severities | §8 |
| SSE-021 | Compaction + ContextMaintenance + TodoProgress + ProgressWatch + WorkspaceLease + Guardian + WorkspaceChanged + DecisionReceipt | §8 |
| SSE-022 | Divergences: snake_case guardian, recovery_paused legacy, order guarantee | §9 |
