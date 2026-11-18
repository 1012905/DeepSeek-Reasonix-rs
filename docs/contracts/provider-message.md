# Provider Message Model Contract — Frozen Inventory (M2)

> Scope: the persisted wire shape of `provider.Message` and the types it references, the
> agent-composition binding model it carries, and the message-normalization behavior that guards
> saved sessions. This is the model the session format stores (session-format.md §3.1 records,
> §5 image side table, checkpoints) and the one every later surface (SSE, HTTP, export) projects.
> Rust landing: `reasonix-contract` (Go: `internal/contract/provider` message-model subset +
> `internal/contract/agentcomposition`). NOT in scope here: the provider client, streaming, retry,
> usage accounting, vendor dialects, schema canonicalization, projection stripping — those form
> the M3 provider-client contract.
>
> Authority: reference source as listed per section; the on-disk `docs/contracts/session-format.md`
> for how these records land on disk.

## 0. Package map and decode rules

- `internal/contract/provider`: 71 Go files (122,409 prod bytes). The message model lives in
  `provider.go` (660 lines), `tool_args.go`, `argument_normalization.go`, `decision.go`,
  `local_metadata.go`, `video.go`, `stream_failure.go`.
- `internal/contract/agentcomposition`: 7 prod files — `composition.go` (209 lines),
  `live.go`, `authority.go`, `parameters.go`, `validate_parameters.go`,
  `builtins.go`, `diagnostics.go`.
- Decode rules that hold for `provider.Message`: plain Go struct JSON — NO custom
  `MarshalJSON`/`UnmarshalJSON` exists on `Message` (verified by grep across the package).
  Unknown fields are ignored; a field whose JSON type does not match fails the whole decode; JSON
  `null` decodes to the zero value. In the event-log replay this is what turns a corrupt
  message into `damaged=true` (session-format.md §3.4).
- `agentcomposition.Definition` decode is STRICT where `Message` is tolerant:
  `Decode` uses `DisallowUnknownFields` plus a trailing-data EOF check (§6.6).

## 1. Message wire shape (provider.go:40-129)

One JSON object. Keys in field-declaration order:

| JSON key | Go field (type) | omitempty | Class |
|---|---|---|---|
| `agent_composition` | `AgentComposition *agentcomposition.Binding` | yes | persisted; non-Live() binding → event schema 4, Live() → schema 5 (session-format.md §3.2) |
| `role` | `Role` (string) | no | provider-visible |
| `content` | `Content string` | yes | provider-visible legacy text field |
| `raw_content` | `RawContent string` | yes | host-only: user text before host-injected context; full tool result when first-visible content was bounded; `ModelMessages` always clears it |
| `provider_content` | `ProviderContent string` | yes | transitional Context Engine v2 field; loaders migrate it into `content`/`raw_content` before normal use |
| `images` | `Images []string` | yes | data URLs (`data:<mime>;base64,…`) on user (attachments) and tool (MCP image results) messages; embedded only for vision-capable models |
| `videos` | `Videos []VideoRef` | yes | file references; bytes read per request (§5) |
| (none) | `VideoParts []VideoPart` | `json:"-"` | never stored: videos resolved for one request |
| `unavailable_images` | `UnavailableImages []UnavailableImage` | yes | images whose stored bytes the store could not read back (§4.1) |
| `reasoning_content` | `ReasoningContent string` | yes | assistant thinking-mode chain-of-thought, round-tripped on multi-turn |
| `reasoning_id` | `ReasoningID string` | yes | provider-issued reasoning-item id (OpenAI Responses `Reasoning.id`) |
| `reasoning_status` | `ReasoningStatus string` | yes | `"in_progress" | "completed"` from the server's done event |
| `reasoning_signature` | `ReasoningSignature string` | yes | opaque provider-issued proof that reasoning is genuine (Anthropic signed thinking) |
| `tool_calls` | `ToolCalls []ToolCall` | yes | set by assistant (§3) |
| `responses_items` | `ResponsesItems []json.RawMessage` | yes | opaque provider Responses API output items replayed on stateless follow-up (DeepSeek `web_search_call`) |
| `tool_call_id` | `ToolCallID string` | yes | links a tool result to its call |
| `name` | `Name string` | yes | tool message: tool name |
| `memoryCitations` | `MemoryCitations []MemoryCitation` | yes | local UI metadata; provider requests ignore it |
| `workDurationMs` | `WorkDurationMs int64` | yes | local UI metadata |
| `thoughtMs` | `ThoughtMs int64` | yes | host-measured thinking time; local UI |
| `createdAt` | `CreatedAt int64` | yes | local UI metadata, unix milliseconds; stripped before provider requests |
| `edited` | `Edited bool` | yes | local UI |
| `original` | `Original string` | yes | user prompt before inline edit |
| `host_authored` | `HostAuthored bool` | yes | marks a user-role message the host composed; declared by the writer |
| `modelRef` | `ModelRef string` | yes | model that wrote this assistant turn; stripped before every provider request |
| `via` | `Via *Via` | yes | paired device a user message was sent from; nil is the window itself |
| (none) | `Derived bool` | `json:"-"` | never stored: request-scoped fact |
| `local_only` | `LocalOnly bool` | yes | durable transcript content never sent to a model provider (interrupted streaming output) |
| `decision_receipt` | `DecisionReceipt *DecisionReceipt` | yes | durable, provider-excluded approval evidence (§4.4) |
| `decision_receipts` | `DecisionReceipts []*DecisionReceipt` | yes | local-only receipts attached to a provider-visible message |
| `interrupted_turn` | `InterruptedTurnRecovery *InterruptedTurnRecovery` | yes | provider-excluded turn handoff (§4.5) |
| `tool_execution` | `ToolExecution *ToolExecution` | yes | local shell UI metadata on tool-result messages; stripped by `ModelMessages` |
| `tool_failure` | `ToolFailure *ToolFailure` | yes | host's own account of a failed result (§4.7) |

Notes:

- The provider-visible vs host-only split is enforced at request-build time (`ModelMessages` /
  `projection.go`), NOT by the wire shape: the stored JSON carries both classes.
- `LocalOnlyToolName = "__reasonix_local_only__"` and `LocalOnlyToolID` (same value) make
  display-only records safe when a newer transcript is opened by an older binary that does not
  know `local_only`: old wire normalization treats the unmatched tool result as an orphan and
  drops it instead of replaying partial content to the model.
- `interruptedToolResult = "[no result: the previous turn was interrupted before this tool call completed]"`
  is the placeholder a repaired unanswered tool call receives.

## 2. Roles

`Role` is a string type: `"system" | "user" | "assistant" | "tool"` (`RoleSystem`,
`RoleUser`, `RoleAssistant`, `RoleTool`). It is the only always-present key besides
the schema version of the enclosing record; the value is not validated against the enum on decode
(any string decodes), so role checks are behavioral, not wire-level.

## 3. Tool calls (provider.go:174-192, argument_normalization.go)

`ToolCall` — one JSON object per call:

| JSON key | Go field (type) | omitempty | Notes |
|---|---|---|---|
| `id` | `ID string` | no | pairs with the tool message's `tool_call_id` |
| `name` | `Name string` | no | may be empty in old sessions (#4727); backfilled from results (§7) |
| `arguments` | `Arguments string` | no | raw JSON, may be truncated from a cut stream (#3953); repaired by §7 |
| `argument_normalization` | `ArgumentNormalization *ArgumentNormalization` | yes | host-only evidence of a redundant-envelope removal |
| `thought_signature` | `ThoughtSignature string` | yes | opaque Gemini-issued proof; OpenAI-compatible Gemini endpoints require it on replay |
| `diff` | `Diff string` | yes | |
| `added` / `removed` | `int` | yes | |
| `resolved_name` | `ResolvedName string` | yes | Reasonix-local display metadata (use_capability proxy calls); never in provider serialization |
| `capability_id` | `CapabilityID string` | yes | same class |
| `resolved_read_only` | `ResolvedReadOnly *bool` | yes | same class |

`ArgumentNormalization` (argument_normalization.go): `{"reason": string, "original": string, "effective": string}` —
all three keys always present (no omitempty). `Reason == "redundant_arguments_envelope"`
(constant `ArgumentEnvelopeUnwrapped`) marks a redundant arguments envelope removal;
`Arguments` keeps the original provider output including signed payloads.
`ToolCall.ExecutionArguments()` returns `Effective` when
`ArgumentNormalization.Reason == ArgumentEnvelopeUnwrapped && ArgumentNormalization.Original == Arguments`,
else `Arguments`.

`ToolSchema` (request-side, listed for completeness): `{"name", "description", "parameters" (raw JSON Schema)}` —
never stored in transcripts.

## 4. Local metadata types

### 4.1 UnavailableImage (provider.go:133-136)

`{"index": int, "ref": string}` — both keys always present. Names one image by its stored
content hash (`ref`, a `sha256:<hex64>` blob key) and its position among the message's
images as written. A stored copy that reappears restores the image; `ModelMessages` tells the
model so a host-owned placeholder stands in.

### 4.2 MemoryCitation (provider.go:164-171)

`{"id"?, "source", "lineStart"?, "lineEnd"?, "note"?, "kind"?}` (omitempty on all but
`source`). Local display metadata for memories that influenced an assistant turn; provider
implementations must not forward it to model APIs.

### 4.3 Via (decision.go:19-26)

`{"device": string, "ordinal": int}` — both keys always present. The paired device a person
acted from; declared by the host that authenticated the device, never read off the input.
`device` is the pairing id, stable while paired; `ordinal` is the number the window
listed the device under when it acted.

### 4.4 DecisionReceipt (decision.go:6-17)

`{"id", "kind", "tool"?, "subject"?, "outcome", "via"?}` (omitempty on `tool`/`subject`/`via`).
Durable, provider-excluded evidence of a user-owned approval decision: bounded labels and outcome
only, never free-form guidance or provider-visible content.

### 4.5 InterruptedTurnRecovery (provider.go:142-160)

`{"stream_failure"?, "pending"?, "completed_tools"?, "interrupted_tools"?, "dropped_partial_text"?, "dropped_partial_reasoning"?}`.
The durable, provider-excluded handoff for a turn that stopped before a clean final answer; only
bounded structural facts — raw partial reasoning stays on the `local_only` Message.

`InterruptedToolSummary`: `{"id"?, "name", "files"?, "added"?, "removed"?}` — a completed,
fully paired tool call without duplicating arguments or result.

### 4.6 ToolExecution (local_metadata.go:16-30)

`{"kind"?, "shell"?, "shellVersion"?, "platform"?, "supportsAndAnd", "state"?, "failurePhase"?, "exitCode"?, "outputTail"?, "mutationRisk"?, "verification"?, "durationMs"?}`.
Host-local shell metadata mirrored from `tool.ShellExecution`; provider serializers never emit
it. Note `supportsAndAnd` has NO omitempty (always serialized); `exitCode` is `*int`
with omitempty (the key is omitted when unset); `durationMs` is `int64`.

### 4.7 ToolFailure (local_metadata.go:9-14)

`{"refusalCode"?, "blocked"?}` — the host's own account of a result that did not succeed;
non-nil `tool_failure` is the fact, `refusalCode` names a refusal. Shell calls also
carry `tool_execution`; every other tool has only this.

### 4.8 StreamFailureCause (stream_failure.go)

A string type, safe to persist and project: `"responses.invalid_function_call"` /
`"responses.unfinished_function_call"` (constants `StreamFailureInvalidFunctionCall` /
`StreamFailureUnfinishedFunctionCall`; sentinel errors `ErrInvalidFunctionCall` "responses:
stream contains an invalid function call", `ErrUnfinishedFunctionCall` "responses: stream ended
with an unfinished function call"). `Description()` returns fixed prose per cause (never
incorporates provider payloads, tool arguments, or error prose): for invalid — "The Responses stream
contained a function call with an invalid wire shape. The host refused the whole response; none of
its tool calls ran or entered model history."; for unfinished — "The Responses stream ended
successfully while a function call was still unclosed. The host refused the whole response; none of
its tool calls ran or entered model history."; empty cause → empty string.

## 5. Video references (video.go:12-22)

`VideoRef` names a video file the host read for the model; it carries no bytes:

| JSON key | Go field | omitempty | Notes |
|---|---|---|---|
| `url` | `URL string` | yes | public address the vendor itself visits; such a reference has no file — `path`/`sha256`/`size` empty |
| `path` | `Path string` | no | |
| `name` | `Name string` | no | |
| `sha256` | `SHA256 string` | no | |
| `mime` | `MIME string` | no | |
| `size` | `Size int64` | no | |
| `duration_ms` | `DurationMs int64` | yes | |

`VideoPart` `{Name, URL, MIME}` is the per-request resolution and is never stored
(`json:"-"` on `Message.VideoParts`). Provider-side constants kept with the model:
`VideoShownNote` "\n[video %s was shown to you when it was read and is not attached again; call
read_video to look at it again]", `VideoNotSentNote` "\n[video %s was not sent: this model cannot
take video natively]", `VideoFailedNote` "\n[video %s was not sent (%s)]", `SafeVideoName`
(replaces `\n→" "`, `\r→" "`, `]→")"`). The five video protocols
(`inline_data_url`, `moonshot_file`, `gemini_inline`, `gemini_files`,
`zhipu_url`) and their vendor limits belong to the M3 provider-client contract.

## 6. Agent composition binding (internal/contract/agentcomposition)

Immutable session capability compositions. The message carries a `Binding` in
`agent_composition`; its shape decides the event-log schema version (§3.2 of session-format.md).

### 6.1 Wire shapes

- `Persona`: `{"prefix": string, "suffix": string, "include_runtime_context": bool}` — no omitempty.
- `Module`: `{"id": string, "parameters"?: raw JSON, "optional"?: bool}`.
- `Selection`: `{"mode": string, "members": []Module}` — no omitempty.
- `Definition`: `{"schemaVersion": int, "id": string, "name": string, "description": string, "persona": Persona, "tools": Selection, "skills": Selection, "mcp": Selection, "extensions": Selection, "hooks": Selection, "commands": Selection}` — no omitempty anywhere.
- `Version`: `{"definition": Definition, "revision": string}`.
- `Resource`: `{"id": string, "kind": string, "data": raw JSON}`.
- `Binding`: `{"formatVersion"?, "resourceMode"?, "resolved": Definition, "started"?, "version": Version, "digest": string, "workspace"?, "resources": []Resource, "systemPrompt"?, "toolSchemaDigest"?, "toolSurface": []string}` (omitempty on the marked keys; `resources` and `toolSurface` always present, may be null).
- `Summary`: `{"id", "name", "revision", "digest", "started", "formatVersion"?, "resourceMode"?}`.
- `Diagnostic`: `{"code": string, "module"?, "detail": string}`.

Constants: `SchemaVersion = 1`; `Inherit = "inherit"`; `Select = "select"`;
`LiveBindingVersion = 2`; `LiveResources = "live"`. Identifier regex:
`^(builtin|user|project):[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$`.

Errors (sentinels): `ErrInvalid` "invalid agent composition", `ErrUnavailable` "agent
composition unavailable", `ErrLocked` "agent composition locked", `ErrWorkspaceBound`
"agent composition is bound to its workspace". `DiagnosticError` renders
`ErrUnavailable`'s message then one `"<module>: <detail>"` line per diagnostic, joined
with "; "; `Unavailable(module, detail)` builds a `DiagnosticError` whose diagnostic has
`code = "agent_composition.unavailable"`.

### 6.2 Live semantics (live.go)

`Binding.Live()` is true iff `FormatVersion == 2 && ResourceMode == "live"` (nil binding →
false). `validateResourceFormat`: `FormatVersion == 0 && ResourceMode == ""` is the
legacy embedded form and passes; anything else must be `Live()` with zero `resources`, or
it is `ErrInvalid: unsupported binding resource format`. `Selection.Allows(id)`:
`mode == inherit`, or `mode == select` and a member has that id.

### 6.3 Digest and revision (composition.go)

`Digest(v any)`: `json.Marshal(v)` → `json.Unmarshal` into `any` (normalizing
away key-order and int formats) → `json.Marshal` again → SHA-256 → lowercase hex. Marshal or
unmarshal failure returns "". `Revision(d Definition) = Digest(d)`. `Binding.Seal()`
computes `Digest` with `started` forced false, then restores it — the digest never covers
`started`. `Clone(v)` is marshal→unmarshal round-trip.

### 6.4 Binding.Validate (composition.go)

In order: nil → `ErrInvalid`; `validateResourceFormat` (§6.2); `Version.Definition.Validate`
(§6.5); `Version.Revision == Revision(Version.Definition)` else `ErrInvalid`;
`Resolved.Validate`; `Resolved.ID == Version.Definition.ID` else `ErrInvalid`;
resources — each `id`/`kind` non-empty, `kind:id` unique, `json.Valid(data)`,
running total (prompt length + all resource data) ≤ 128 MiB, ≤ 20000 entries, else
`ErrInvalid: retained resource limit`; finally a cloned `Seal()` must reproduce
`digest` else `ErrInvalid: binding digest`.

### 6.5 Definition.Validate (composition.go)

`schemaVersion == 1`; `id` matches the identifier regex; `name` trimmed non-empty
and ≤ 1024 bytes; `description` ≤ 16384 bytes; persona `prefix`/`suffix` each
≤ 32768 bytes. For each of the six selections (tools, skills, mcp, extensions, hooks, commands):
≤ 10000 members; `mode` is `inherit` or `select`, and `inherit` requires zero
members; `optional` is only allowed in the tools group (group index 0); members have non-empty
unique ids ≤ 1024 bytes; non-empty `parameters` must be a JSON object. Then
`validateModuleParameters` (§6.7). Failure messages wrap `ErrInvalid` with a short cause
("schemaVersion, id or name", "persona", "selection mode", "optional is only supported for tools",
"duplicate or empty module %q", "module parameters").

### 6.6 Decode (composition.go)

`Decode(data)`: `json.Decoder` with `DisallowUnknownFields`; a second `Decode`
into `any` must return `io.EOF` (else `ErrInvalid: trailing data`); then
`ValidateForExecution` (§6.8). History decoding checks structure only; executable
declarations also obey source authority.

### 6.7 Parameters (parameters.go, validate_parameters.go)

`Parameters`: `{"args"?: []string, "env"?: map[string]string, "headers"?: map[string]string, "url"?: string, "callTimeoutSeconds"?: int, "startupTimeoutSeconds"?: int, "timeoutMillis"?: int}`.
`ParseParameters`: empty raw → zero value; `DisallowUnknownFields` decode; every
`env`/`headers` value must match the environment-reference regex `^${[a-zA-Z_][a-zA-Z0-9_]*}$`
(references only); timeouts must be ≥ 0; `args` entries must survive secret redaction (a
redacted arg must itself be an `${NAME}` reference, or an `inline=`-shaped entry whose
value is a reference) else `ErrInvalid: credentials require references`; `url` likewise
(`ErrInvalid: endpoint credentials require references`).

`validateModuleParameters`: MCP and extensions members pass `ParseParameters`;
extensions forbid `url`/`headers`/`callTimeoutSeconds`/`startupTimeoutSeconds`
(`ErrInvalid: unsupported extension parameter`); MCP forbids `timeoutMillis`
(`ErrInvalid: unsupported MCP parameter`); tools/skills/hooks/commands members may carry only
empty or `"{}"` parameters (`ErrInvalid: parameters unsupported for module <id>`).

### 6.8 Execution authority (authority.go)

`Definition.ValidateForExecution` = `Validate` + for `project:`-prefixed ids: every
MCP/extensions member's parameters may override ONLY timeouts — `args`/`url`/`env`/`headers`
set → `ErrInvalid: project module <id> may override only timeouts`. `Binding.ValidateForExecution`
= `Validate` + both definitions' `ValidateForExecution`.

### 6.9 Summary and builtins

`Binding.Summary()` (nil binding → nil) projects `{id, name, revision, digest, started,
formatVersion?, resourceMode?}` from the binding. `Builtins()` decodes the embedded
`builtins.json` (errors ignored — empty/nil list on failure).

## 7. Message normalization (provider.go:220-533, tool_args.go)

Normalization is TOTAL: every input yields a well-formed, model-acceptable message list. It is the
gate between stored history (any shape the log preserved) and the two consumers: the provider wire
(strict pairing, orphan-free) and session display (lossless, receipts folded).

### 7.1 Entrypoints

- `NormalizeMessages(msgs)` = `normalizeMessages(msgs, true)` — provider wire-safe
  (dropOrphanTools=true); `SanitizeToolPairing` is an alias.
- `NormalizeSessionMessages(msgs)` = `normalizeMessages(attachStandaloneDecisionReceipts(msgs), false)`
  — session view (lossless).

### 7.2 attachStandaloneDecisionReceipts

Two passes over the list. Pass 1 detects whether any `LocalOnly` message carrying a
`DecisionReceipt` sits after a non-LocalOnly assistant message (and before the next
non-LocalOnly user message); healthy histories return the input unchanged. Pass 2 folds: each such
LocalOnly message is removed and its receipt is appended to that assistant's `DecisionReceipts`.
A non-LocalOnly user message resets the fold target; an assistant message sets it.

### 7.3 normalizeMessages(msgs, dropOrphanTools)

First `tryNormalizeFastPath` (§7.4); on miss, rebuild:

- `LocalOnly` message → keep for session / drop for provider.
- Assistant message with `ToolCalls` → scan `j` over the following run of non-LocalOnly
  `RoleTool` messages; `calls = backfillToolCallNames` (§7.7); set `m.ToolCalls = calls`;
  append `repairToolCallArgs(m, dropOrphanTools)` (§7.8); then append
  `pairToolResults(calls, msgs[i+1:j])` when dropOrphanTools, else
  `sessionToolResults(calls, msgs[i+1:j])` (§7.6); continue at `i = j`.
- Bare `RoleTool` (no preceding assistant-with-calls) → keep for session / drop for provider.
- Everything else appended as-is.

### 7.4 tryNormalizeFastPath(msgs, dropOrphanTools)

Returns the input unchanged (same backing array, zero allocations) when nothing needs repair:
no LocalOnly message present while dropOrphanTools; every assistant-with-calls turn is
`toolTurnWellFormed` and needs no argument repair; no bare RoleTool while dropOrphanTools.
Otherwise falls through to the slow rebuild.

### 7.5 Well-formedness primitives

- `toolTurnWellFormed(calls, results)`: `len(calls) == len(results)`; every call has a
  non-empty Name; `results[k].ToolCallID == calls[k].ID && results[k].Name == calls[k].Name`.
- `idDistinct(calls)`: every ID non-empty and unique within the batch.

### 7.6 Result pairing

`pairToolResults(calls, avail)` (provider mode) — `avail` is the following tool-result
run. When `idDistinct`: build a ToolCallID→result map; per call, found → append with
`r.Name = tc.Name`; missing → append the placeholder
`Message{Role: tool, ToolCallID: tc.ID, Name: tc.Name, Content: interruptedToolResult}`.
Not distinct → positional pairing: result `k` exists → its ToolCallID/Name are overwritten
from call `k`; beyond the end → placeholder.

`sessionToolResults(calls, avail)` (session mode) — keeps ALL avail results in order. When
`idDistinct`: append placeholders for calls whose ID is not in the answered set. Not distinct:
append placeholders positionally for `k` in `len(avail)..len(calls)-1`. The placeholder
literal is the same in both modes (§7.6).

### 7.7 backfillToolCallNames(calls, results)

Old sessions may record empty call Names (#4727). If no Name is empty, return unchanged. Otherwise
copy and fill: distinct IDs → fill from the paired results by ToolCallID (results with non-empty
Name); otherwise positional fill from `results[k].Name`; unpaired calls keep the empty Name.

### 7.8 Argument repair (tool_args.go + provider.go)

- `argsNeedRepair(args, forProvider)`: `TrimSpace(args) == ""` → forProvider (empty
  arguments are repaired only on the provider path — session keeps them as recorded); otherwise
  `!json.Valid(args)`.
- `needsToolCallArgRepair(calls, forProvider)`: any call needs repair.
- `repairToolCallArgs(m, forProvider)`: copy-on-write; per call, empty → `"{}"`; non-empty
  invalid → `closeTruncatedJSON` (non-empty invalid JSON is repaired in BOTH modes — a cut
  stream must not poison the provider wire, and session replay must show a closed shape).
- `closeTruncatedJSON(s)`: scan with a closer stack and in-string/escape state; if the scan
  ends mid-escape drop the last character; if mid-string append `"`; `TrimRight " \t\r\n"`;
  a trailing `,` is dropped; a trailing `:` gets `null` appended; append the closer
  stack in reverse; if the result still fails `json.Valid` → `"{}"`.

## 8. Event-record binding (image side table)

The event log (session-format.md §3) stores each turn's messages as a record whose `messages`
field materializes exactly the §1 model. Images ride a side table, not inline bytes:

- The record's `image_blobs` field maps §5 keys (`sha256:<hex64>`) to blob references;
  the on-disk blob lives at `blobs/<first2>/<rest62>` under the session directory
  (session-format.md §5).
- `sessionEventImages` = `{"message": <index into the record's messages>, "slots": []sessionImageSlot}`;
  `sessionImageSlot` = `{"blob"?: key, "url"?: string}`. A slot with only a URL names a
  remote image the vendor fetched itself; a blob slot names stored content.
- Slot layout rule: unavailable images occupy their original indexes; readable images fill in order;
  stray images are appended (session_image_blobs.go messageImageSlots).
- `externalizeSessionImages` writes blobs (fsynced) BEFORE the record is written, and on the
  log copy clears `Images`/`UnavailableImages` so transcripts never inline image bytes.
- Replay reattaches blobs (64 MiB per-image read cap; key must be bare lowercase hex 64).

## 9. Divergences and notes

1. Message decode is tolerant (unknown fields ignored, type errors fail the whole record) while
   agentcomposition `Definition`/`Parameters` decode is strict
   (`DisallowUnknownFields` + trailing-data rejection). Both are load-bearing: the log must
   survive newer writers, composition execution must not.
2. `Message` carries no `time.Time`: `createdAt` is int64 milliseconds; timestamps
   live RFC3339 on the enclosing record, not on the message.
3. Rust mapping: `*T`/`omitempty` → `Option` + `skip_serializing_if`; plain
   fields serialize always (no `skip_serializing_if`) to match Go's no-omitempty output;
   unknown fields must be IGNORED by default (serde default) to mirror Go tolerance; the
   composition strict paths need `deny_unknown_fields` + a trailing-data check on the raw
   buffer (serde_json from_slice already rejects trailing data).
4. `arguments` stays a raw JSON string in both languages — never parse-then-reserialize on
   the passthrough path (vendor signatures and key order must survive byte-for-byte where valid).
5. Normalization (§7) is total and non-failing: every path returns an in-model list. Rust must
   reproduce the same repair outputs, including the exact placeholder content string and the
   session/provider asymmetries (empty-args and orphaned-tool handling).
6. The 128 MiB resource total and 20000-entry resource cap (§6.4) are DoS bounds, not user limits;
   they match session replay budgets and must not drift independently.
7. Rust strict-decode diagnostics differ from Go (serde `unknown field` wording vs Go
   `json: unknown field`); only the `invalid agent composition: ` prefix and the
   invalid-error classification are contractual — wrapped decode details are diagnostics only.
8. serde_json rejects duplicate JSON object keys where Go's decoder is last-wins; unreachable
   for marshal-produced input (Go never emits duplicates) and stricter in the safe direction.
   Accepted divergence.
9. Digest canonicalization: Go re-marshals through `any` (float64 numbers) while the Rust
   port round-trips `serde_json::Value` (u64/i64 preserved); digests are identical for
   every integer the corpus can produce (≤ 2^53) — beyond that Go loses precision first.
10. `decode(null)` decodes to a zero `Definition` (Go null-into-struct no-op) and
    then fails `ValidateForExecution` on it.

## 10. Evidence (reference test inventory)

- internal/contract/provider: `provider_test.go` (normalization: repair, pairing, fast path,
  session/provider asymmetry), `argument_normalization_test.go`, `image_test.go`
  (UnavailableImage), `stream_failure_test.go`, `tool_execution_cache_test.go`,
  `projection_composition_test.go` / `projection_unavailable_test.go` /
  `projection_video_test.go` (provider projection — M3 boundary, listed not frozen).
- internal/contract/agentcomposition: `composition_test.go`, `live_test.go`,
  `authority_test.go`, `builtin_revisions_test.go`.
- Cross-cutting: session replay exercises the model through
  internal/state/sessionstore tests (session-format.md §3.16 evidence list).

## 11. Proposed ledger rows

Splice after SESS-100 in docs/parity/LEDGER.md; a new Area `Provider` (crate
`reasonix-contract`, mirroring Go's internal/contract/provider placement under the §4.2
dependency rule state → {contract, platform}). Rows are extracted from this table verbatim.

| ID | Area | Reference | Rust | Status | Evidence |
|---|---|---|---|---|---|
| PMSG-001 | Provider | provider.Message wire shape (JSON keys, omitempty set) | reasonix-contract | todo | M2: docs/contracts/provider-message.md §1 |
| PMSG-002 | Provider | Message decode semantics (tolerant, type errors fail record) | reasonix-contract | todo | M2: docs/contracts/provider-message.md §0 |
| PMSG-003 | Provider | Roles + local-only sentinel messages | reasonix-contract | todo | M2: docs/contracts/provider-message.md §2 |
| PMSG-004 | Provider | ToolCall + ArgumentNormalization + ExecutionArguments | reasonix-contract | todo | M2: docs/contracts/provider-message.md §3 |
| PMSG-005 | Provider | Local metadata types (Via, DecisionReceipt, ToolExecution, ToolFailure, StreamFailureCause, interrupted_turn) | reasonix-contract | todo | M2: docs/contracts/provider-message.md §4 |
| PMSG-006 | Provider | VideoRef wire shape + video notes | reasonix-contract | todo | M2: docs/contracts/provider-message.md §5 |
| PMSG-007 | Provider | agentcomposition wire shapes (Persona..Binding) | reasonix-contract | todo | M2: docs/contracts/provider-message.md §6.1 |
| PMSG-008 | Provider | Digest / Revision / Seal | reasonix-contract | todo | M2: docs/contracts/provider-message.md §6.3 |
| PMSG-009 | Provider | Live() + resource format validation | reasonix-contract | todo | M2: docs/contracts/provider-message.md §6.2 |
| PMSG-010 | Provider | Binding.Validate | reasonix-contract | todo | M2: docs/contracts/provider-message.md §6.4 |
| PMSG-011 | Provider | Definition.Validate | reasonix-contract | todo | M2: docs/contracts/provider-message.md §6.5 |
| PMSG-012 | Provider | Composition decode strictness | reasonix-contract | todo | M2: docs/contracts/provider-message.md §6.6 |
| PMSG-013 | Provider | ParseParameters + module parameter rules + execution authority | reasonix-contract | todo | M2: docs/contracts/provider-message.md §6.7-6.8 |
| PMSG-014 | Provider | NormalizeMessages (provider wire-safe) | reasonix-contract | todo | M2: docs/contracts/provider-message.md §7.1-7.3 |
| PMSG-015 | Provider | NormalizeSessionMessages (session view) | reasonix-contract | todo | M2: docs/contracts/provider-message.md §7.1-7.3 |
| PMSG-016 | Provider | attachStandaloneDecisionReceipts | reasonix-contract | todo | M2: docs/contracts/provider-message.md §7.2 |
| PMSG-017 | Provider | pairToolResults / sessionToolResults / idDistinct | reasonix-contract | todo | M2: docs/contracts/provider-message.md §7.5-7.6 |
| PMSG-018 | Provider | backfillToolCallNames | reasonix-contract | todo | M2: docs/contracts/provider-message.md §7.7 |
| PMSG-019 | Provider | repairToolCallArgs / closeTruncatedJSON | reasonix-contract | todo | M2: docs/contracts/provider-message.md §7.8 |
| PMSG-020 | Provider | tryNormalizeFastPath (zero-alloc passthrough) | reasonix-contract | todo | M2: docs/contracts/provider-message.md §7.4 |

Frozen 2026-11-15. Scope boundary: provider client/streaming/retry/usage/vendors/schema dialects
remain the M3 provider-client contract; this document freezes only the message model, composition
binding, and normalization.
