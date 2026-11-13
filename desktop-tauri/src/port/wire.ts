// The kernel event wire, mirrored from Go `internal/contract/eventwire` and
// Rust `reasonix-contract`. Field names, kind strings and omitempty sets are
// the contract, not a convenience shape: `eventwire_manifest.json` — generated
// from the Go types by reflection — is the field-set authority, and
// wire.parity.test.ts holds this file to it in both directions.

export const KINDS = [
  "turn_started",
  "reasoning",
  "text",
  "message",
  "tool_dispatch",
  "tool_result",
  "usage",
  "notice",
  "phase",
  "approval_request",
  "ask_request",
  "turn_done",
  "compaction_started",
  "compaction_progress",
  "compaction_done",
  "tool_progress",
  "mcp_surface_ready",
  "retrying",
  "steer",
  "guardian_assessment",
  "extension_surface",
  "extension_status",
  "stream_attempt",
  "context_maintenance",
  "todo_progress",
  "workspace_lease",
  "workspace_changed",
  "turn_phase",
  "completion_summary",
  "adjudications_changed",
  "browser_tabs_changed",
  "inbox_changed",
  "rewind_changed",
  "graph_delta",
  "progress_watch",
  "ask_engaged",
] as const;

export type Kind = (typeof KINDS)[number];

export interface GraphNode {
  id: string;
  parentId?: string;
  kind?: string;
  state?: string;
  label?: string;
  profile?: string;
  model?: string;
  effort?: string;
  grant?: string;
  wait?: string;
  ref?: string;
  queuedAt?: number;
  startedAt?: number;
  endedAt?: number;
  err?: string;
}

export interface GraphEdge {
  from: string;
  to: string;
  kind: string;
}

export interface GraphDelta {
  nodes?: GraphNode[];
  edges?: GraphEdge[];
}

// The folded graph: same two arrays a delta carries and a different thing —
// a state, replaced rather than folded.
export interface GraphSnapshot {
  nodes?: GraphNode[];
  edges?: GraphEdge[];
}

export interface DeliveredFile {
  path: string;
  root?: string;
  description?: string;
}

export interface ContextPart {
  name: string;
  tokens: number;
}

// `active` is the running turn, which a fold cannot take.
export interface ContextItem {
  role: string;
  tokens: number;
  tool?: string;
  active?: boolean;
}

export interface ContextComposition {
  total: number;
  at: number;
  active: number;
  parts: ContextPart[];
  largest?: ContextItem[];
}

export interface Money {
  amount: string;
  currency: string;
}

export interface Valuation {
  money: Money;
  basis: string;
  source: string;
  asOf: string;
  rateSnapshot?: RateSnapshot;
  stale?: boolean;
}

export interface RateSnapshot {
  base: string;
  quote: string;
  rate: number;
  source: string;
  asOf: string;
  stale?: boolean;
}

// How much of the usage behind a quote carries a complete price fact. Kept
// apart from displayStatus, which is about whether the requested currency
// could be shown, not about whether the cost is known.
export interface CostQuote {
  original: Money;
  originalTotals?: Money[];
  valuations?: Record<string, Valuation>;
  selected?: Money;
  billingMode?: string;
  estimated: boolean;
  costComplete: boolean;
  coverage?: string;
  displayComplete: boolean;
  complete: boolean;
  displayStatus?: string;
  aggregateMode?: string;
  modelRef?: string;
  usageSource?: string;
  pricingFingerprint?: string;
  rateDate?: string;
  incompleteReason?: string;
  legacyEstimate?: boolean;
  catalogSource?: string;
  rateBand?: string;
}

export interface CacheDiagnostics {
  prefixHash: string;
  prefixChanged: boolean;
  prefixChangeReasons?: string[];
  systemHash: string;
  toolsHash: string;
  logRewriteVersion: number;
  toolSchemaTokens: number;
  cacheMissTokens: number;
  cacheHitTokens: number;
  carriedMessages: number;
  bodyChanged: boolean;
  bodyHash: string;
}

export interface Usage {
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  cacheHitTokens: number;
  cacheMissTokens: number;
  sessionCacheHitTokens: number;
  sessionCacheMissTokens: number;
  reasoningTokens?: number;
  estimated?: boolean;
  source?: string;
  // The stream attempt that billed these tokens; usage lands after its round
  // has settled, so this is what attaches it.
  attemptId?: string;
  cacheDiagnostics?: CacheDiagnostics;
  contextPromptTokens?: number;
  contextCompletionTokens?: number;
  contextReasoningTokens?: number;
  contextCacheHitTokens?: number;
  contextCacheMissTokens?: number;
  cost?: number;
  currency?: string;
  currencyCode?: string;
  costUsd?: number;
  costQuote?: CostQuote;
  costComplete?: boolean;
  displayComplete?: boolean;
  displayStatus?: string;
  aggregateMode?: string;
  originalTotals?: Money[];
}

export interface RecoveryApproval {
  source_agent?: string;
  failed_tool?: string;
  failed_summary?: string;
  diagnosis?: string;
  next_tool?: string;
  next_action?: string;
  change_kind?: string;
  change_rationale?: string;
  review_rationale?: string;
  plan_before?: string;
  plan_after?: string;
  can_grant_task?: boolean;
  task_grant_scope?: string;
}

// A decision the model may not make alone. `scope` is what approving
// authorizes beyond the call; `input`, the call's arguments, rides only with
// a scope, since they are what it applies to.
export interface Approval {
  id: string;
  tool: string;
  subject: string;
  reason?: string;
  reasonCode?: string;
  fresh?: boolean;
  allowsSession?: boolean;
  allowsPersist?: boolean;
  kind?: string;
  recovery?: RecoveryApproval;
  scope?: string;
  input?: Record<string, unknown>;
}

export interface AskOption {
  label: string;
  description?: string;
}

export interface AskQuestion {
  id: string;
  prompt: string;
  options: AskOption[];
  header?: string;
  // Why only the user can answer. Two values; a third would be a decision
  // with a different owner.
  reason?: string;
  multi?: boolean;
  // Prefilled answer: a form sent back to be corrected keeps what was given.
  default?: string[];
  surface?: Spec;
  callId?: string;
}

// Where the ask came from. `urlHost` and `urlLocal` are the host's own reading
// of `url` and ride along so clients need not re-resolve it.
export interface AskOrigin {
  kind: string;
  source: string;
  message?: string;
  note?: string;
  url?: string;
  urlHost?: string;
  urlLocal?: boolean;
}

export interface Ask {
  id: string;
  questions: AskQuestion[];
  origin?: AskOrigin;
  skipInMs?: number;
}

// A stored image a message references; only its path rides the wire.
export interface Attachment {
  path: string;
}

// How a result was fitted into the model's context, absent when it arrived
// whole. "spilled" keeps `path`; "truncated" keeps `keptBytes`.
export interface Bound {
  kind: string;
  lines?: number;
  bytes?: number;
  keptBytes?: number;
  path?: string;
}

export interface Profile {
  name?: string;
  count?: number;
  model?: string;
  effort?: string;
  // The task's own short description, one sub-agent only.
  label?: string;
}

export interface VideoShown {
  name: string;
  size: number;
  model: string;
  durationMs?: number;
}

// How the command ran, which its output alone cannot say.
export interface ShellExecution {
  supportsAndAnd: boolean;
  jobId?: string;
  kind?: string;
  shell?: string;
  // PowerShell 5.1 and 7 are different languages; the shell name alone
  // cannot say which answered.
  shellVersion?: string;
  platform?: string;
  state?: string;
  failurePhase?: string;
  exitCode?: number;
  outputTail?: string;
  // The command without its leading environment assignments, when that
  // changed it; never what authorised it.
  subject?: string;
  mutationRisk?: string;
  verification?: string;
  durationMs?: number;
}

// Where the host corrected the model's arguments before the call ran.
export interface ArgumentNormalization {
  reason: string;
  original: string;
  effective: string;
}

// Who issued a call is the one thing separating the model's work from the
// host's own bookkeeping.
export interface Tool {
  name: string;
  readOnly: boolean;
  deliveries?: DeliveredFile[];
  id?: string;
  args?: string;
  argumentNormalization?: ArgumentNormalization;
  resolvedName?: string;
  capabilityId?: string;
  output?: string;
  // What the call showed the model, as data URLs.
  images?: string[];
  videos?: VideoShown[];
  err?: string;
  // The host's dotted identity for a refusal. err is only its wording.
  refusalCode?: string;
  workspaceLease?: WorkspaceLease;
  outputDiff?: boolean;
  truncated?: boolean;
  bound?: Bound;
  durationMs?: number;
  // A sub-agent's estimated tokens per second; not provider usage.
  rate?: number;
  contextTokens?: number;
  startedAt?: number;
  endedAt?: number;
  partial?: boolean;
  // Cumulative argument characters received so far on a partial dispatch.
  argChars?: number;
  refreshed?: boolean;
  parentId?: string;
  issuer?: string;
  attemptId?: string;
  diff?: string;
  added?: number;
  removed?: number;
  profile?: Profile;
  execution?: ShellExecution;
}

// The safety guardian's reading of one tool call before it runs. Its JSON
// names stay snake_case because that is what the Go tags spell.
export interface Guardian {
  id: string;
  tool: string;
  subject: string;
  outcome: string;
  risk_level?: string;
  user_authorization?: string;
  rationale?: string;
  duration_ms?: number;
  usage?: Usage;
}

// What the kernel recorded when a prompt was settled, on the same ordered
// stream as the request that raised it.
export interface DecisionReceipt {
  id: string;
  kind: string;
  outcome: string;
  tool?: string;
  subject?: string;
  via?: Via;
}

// The paired device a person acted from; absent is the window's own.
export interface Via {
  device: string;
  ordinal: number;
}

// One local memory the turn drew on, so an answer can show what it was
// grounded in rather than asserting it.
export interface MemoryCitation {
  source: string;
  id?: string;
  lineStart?: number;
  lineEnd?: number;
  note?: string;
  kind?: string;
}

// One speculative stream attempt over one provider response.
export interface StreamAttempt {
  id: string;
  action: string;
  attempt?: number;
  max?: number;
  reason?: string;
  // Commit only: request sent to first output, host-measured.
  firstTokenMs?: number;
}

// Content-free quality summary of a finished turn.
export interface CompletionSummary {
  preset: string;
  verdict: string;
  mutations: number;
  checks_passed: number;
  checks_failed: number;
  checks_suppressed: number;
  review: string;
  gap_kinds?: string[];
  // Existing tests the turn rewrote or removed.
  criteria_rewritten?: string[];
}

// `risks` and `unverified` are the turn's declarations, apart from the gaps
// the host found: a reader that folds them together reads a volunteered
// caveat as a failure.
export interface CompletionReceipt {
  verdict: string;
  saysSomething: boolean;
  changes?: ReceiptChange[];
  verifications?: ReceiptVerification[];
  gaps?: ReceiptGap[];
  risks?: string[];
  unverified?: string[];
  severity?: string;
}

export interface ReceiptChange {
  path: string;
  reviewed: boolean;
}

export interface ReceiptVerification {
  command: string;
  passed: boolean;
  stale?: boolean;
  inconclusive?: boolean;
}

export interface ReceiptGap {
  kind: string;
  detail?: string;
}

// One context-maintenance transaction. status "noop" carries code: the
// attempt ran and freed nothing, and code tells "already folded this turn"
// from "nothing left to fold".
export interface ContextMaintenance {
  status?: string;
  action?: string;
  trigger?: string;
  operationId?: string;
  inputTokens?: number;
  resultTokens?: number;
  savedTokens?: number;
  affectedToolResults?: number;
  projectionVersion?: number;
  cacheBreak?: boolean;
  reason?: string;
  code?: string;
  boundary?: string;
  triggerTokens?: number;
}

// One task-list transition. kind is the structural verdict; the three
// revisions count writes, plan changes, and execution movement separately.
export interface TodoProgress {
  kind: string;
  steps?: number;
  completed?: number;
  contentRevision?: number;
  planRevision?: number;
  progressRevision?: number;
}

// The host's reading of whether the run still moves: rounds since the last
// effect it could observe, and input spent against the backstop.
export interface ProgressWatch {
  stalled: boolean;
  idleRounds: number;
  cause?: string;
  roundLimit?: number;
  promptTokens?: number;
  tokenLimit?: number;
  tokenMultiple?: number;
  pausing?: boolean;
}

// Who holds the workspace and how long it has been held or contended.
export interface WorkspaceLease {
  contended: number;
  heldMs: number;
  idleMs: number;
  reported?: number;
  waitedMs?: number;
  holder?: string;
  holderSessionId?: string;
  paths?: string[];
  requestedPaths?: string[];
  repository?: string;
}

export interface WorkspaceRevision {
  content: number;
  tree: number;
  workingTree: number;
  gitMeta: number;
  session: number;
}

export interface WorkspacePathChange {
  path: string;
  op: string;
  oldPath?: string;
}

// A workspace mutation. Revisions and watch state always arrive; a nil payload
// still renders, so readers never split "no changes" from "the event arrived
// without one".
export interface WorkspaceChanged {
  revisions: WorkspaceRevision;
  changes: WorkspacePathChange[];
  allPaths: boolean;
  source: string;
  watchState: string;
}

// What a fold did, what it cost, and what the prompt still holds after it.
export interface Compaction {
  trigger?: string;
  messages?: number;
  summary?: string;
  sourceTokens?: number;
  projectionTokens?: number;
  coverageRequired?: number;
  coverageMissing?: number;
  coverageBackstopped?: boolean;
  boundary?: string;
  triggerTokens?: number;
  // Why a fold that installed nothing did not; the host's class, not a sentence.
  code?: string;
  remains?: ContextComposition;
}

// ─── surfacespec: the surface protocol DTO the wire embeds ───
// Mirrors internal/contract/surfacespec. A view is composed rather than filled
// in: the producer sends a tree of primitives and the client renders them with
// its own components. Tone says what a node means, never what colour it is.

export type SurfaceTone = "dim" | "strong" | "ok" | "warn" | "err" | "accent";

export interface Spec {
  // `spec_version` is the Go tag spelled; the only snake_case on the wire.
  spec_version: number;
  title?: string;
  body: SurfaceNode[];
}

export interface SurfaceNode {
  kind: string;
  value?: string;
  key?: string;
  label?: string;
  tone?: SurfaceTone;
  progress?: number;
  actionId?: string;
  children?: SurfaceNode[];
  field?: SurfaceField;
  diff?: SurfaceDiff;
  dataset?: SurfaceDataset;
}

export interface SurfaceDiff {
  before: string;
  after: string;
}

export interface SurfaceField {
  type: string;
  required?: boolean;
  default?: string | number | boolean | string[];
  min?: number;
  max?: number;
  max_length?: number;
  options?: SurfaceOption[];
}

export interface SurfaceOption {
  value: string;
  label: string;
  description?: string;
  preview?: string;
  group?: string;
  location?: SurfaceLocation;
  price?: number;
  currency?: string;
  facts?: SurfaceFact[];
  image?: SurfaceImage;
}

export interface SurfaceImage {
  path: string;
  alt: string;
  source?: string;
}

// Both coordinates always arrive: absent means unknown, which null says.
export interface SurfaceLocation {
  latitude: number | null;
  longitude: number | null;
}

export interface SurfaceFact {
  label: string;
  value: string;
}

export interface SurfaceDataset {
  categories: string[];
  series: SurfaceSeries[];
}

export interface SurfaceSeries {
  id: string;
  label: string;
  values: number[];
  unit?: string;
}

// ─── extension surfaces ───
// Mirrors internal/contract/eventwire's extension payload. A panel has no
// markdown on purpose — see the protocol DTO: the side rail is a narrow
// column, and a rendered document there costs more than it tells.

export interface ExtensionKeyValue {
  key: string;
  value: string;
}

export interface ExtensionActionRef {
  actionId: string;
  label: string;
}

export interface ExtensionStatus {
  label: string;
  detail?: string;
  progress?: number;
  severity?: string;
}

export interface ExtensionCard {
  title?: string;
  markdown?: string;
  text?: string;
  fields?: ExtensionKeyValue[];
  progress?: number;
  actions?: ExtensionActionRef[];
}

export interface ExtensionFormField {
  key: string;
  label?: string;
  kind?: "confirm" | "input" | "select" | "multiselect";
  options?: string[];
  default?: unknown;
  required?: boolean;
}

export interface ExtensionForm {
  title?: string;
  message?: string;
  fields: ExtensionFormField[];
}

export interface ExtensionPanel {
  title?: string;
  text?: string;
  fields?: ExtensionKeyValue[];
  progress?: number;
  actions?: ExtensionActionRef[];
}

export interface ExtensionNotification {
  title: string;
  body?: string;
  severity?: string;
}

export interface ExtensionViewNode {
  kind: string;
  value?: string;
  key?: string;
  label?: string;
  tone?: SurfaceTone;
  progress?: number;
  actionId?: string;
  children?: ExtensionViewNode[];
  field?: SurfaceField;
  diff?: SurfaceDiff;
  dataset?: SurfaceDataset;
}

export interface ExtensionView {
  // Where the extension would like to stand. A name we do not know renders
  // where we put views we have no place for, rather than not at all.
  slot?: string;
  // "tool:<callId>" when this view replaces a card's body instead of standing
  // on its own. Only tool calls can be anchored — an approval prompt or an
  // error state is not addressable, which is what keeps a takeover from being
  // able to redraw a decision.
  anchor?: string;
  body: ExtensionViewNode[];
}

export interface ExtensionSurface {
  kind: "status" | "card" | "form" | "notification" | "panel" | "view";
  pluginId: string;
  surfaceId: string;
  code?: string;
  sessionId?: string;
  generation?: number;
  status?: ExtensionStatus;
  card?: ExtensionCard;
  form?: ExtensionForm;
  notification?: ExtensionNotification;
  panel?: ExtensionPanel;
  view?: ExtensionView;
}

// ─── the event envelope ───
// One frame on the event stream. kind is the discriminant; every payload is
// optional because kinds and payloads pair, not nest.

export interface WireEvent {
  kind: Kind;
  // Set by the transport on frames that must not be missed, so a client can
  // tell it missed one. Streaming deltas carry none — losing one costs nothing
  // the next frame does not restate.
  seq?: number;
  nonPersistable?: boolean;
  text?: string;
  detail?: string;
  code?: string;
  reasoning?: string;
  // message: how long the kernel measured this reply thinking, in ms.
  thoughtMs?: number;
  memoryCitations?: MemoryCitation[];
  level?: string;
  // "operator" means the notice is about the machine running this
  // conversation, not about the conversation. Only the latter belongs in a
  // transcript.
  audience?: string;
  tool?: Tool;
  usage?: Usage;
  approval?: Approval;
  ask?: Ask;
  guardian?: Guardian;
  extension?: ExtensionSurface;
  compaction?: Compaction;
  maintenance?: ContextMaintenance;
  todoProgress?: TodoProgress;
  progressWatch?: ProgressWatch;
  workspaceLease?: WorkspaceLease;
  streamAttempt?: StreamAttempt;
  completion?: CompletionSummary;
  graph?: GraphDelta;
  decisionReceipt?: DecisionReceipt;
  // The completion receipt the turn volunteered, apart from the summary the
  // host adjudicated.
  receipt?: CompletionReceipt;
  err?: string;
  // turn_done: the user asked to stop. A cancelled turn and a dropped
  // connection both end with a context-canceled err; only this tells them apart.
  cancelled?: boolean;
  // final_readiness and recovery_paused are no longer emitted; sessions
  // recorded before they were removed still carry them, so a reader has to
  // render them.
  outcome?: "final_readiness" | "recovery_paused" | "no_progress";
  phase?: string;
  retryAttempt?: number;
  retryMax?: number;
  retryScope?: "headers" | "stream";
  retryCause?:
    | "connection_closed"
    | "timeout"
    | "upstream_status"
    | "stream_idle"
    | "upstream_error";
  retryStatus?: number;
  retryDelayMs?: number;
  retryTimeoutSecs?: number;
  itemId?: string;
  // The turn a compaction checkpoint committed under, so a reader can tell a
  // fold apart from the turns around it.
  checkpointTurn?: number;
  // turn_done: ms the run took up to its final answer; absent when it gave none.
  workDurationMs?: number;
  // turn_done: unix ms the turn began; sent only alongside workDurationMs.
  workStartedAt?: number;
  // Which producer wrote this frame: "executor", "planner", "subagent". One
  // turn can carry two models, and the alternative is reading it off the last
  // phase marker — an adjacency the record does not even keep.
  source?: string;
  // turn_started: which message this turn is about. authoredTurn numbers the
  // conversation's own turns, msgIndex the session log — a checkpoint's `turn`
  // is neither. A client mints its own user row, so this is the only thing
  // that says which row the kernel just started a turn for.
  authoredTurn?: number;
  msgIndex?: number;
  // turn_started / steer: the paired device the message came from; absent is
  // the window. The message itself rides text, for every client that did not
  // send it.
  via?: Via;
  // turn_started / steer: the stored attachments the message references.
  attachments?: Attachment[];
  // steer: the host wrote this guidance, so it is never drawn as the person's.
  hostAuthored?: boolean;
  // Which model wrote this: the turn's on turn_started, a second model's on
  // the text it wrote. A reply with none is the turn's.
  modelRef?: string;
  // A workspace mutation event.
  workspace?: WorkspaceChanged;
}
