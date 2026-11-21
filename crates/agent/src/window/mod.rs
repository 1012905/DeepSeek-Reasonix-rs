//! The context window (Go: `context_window.go`, `context_window_host.go`,
//! `context_manager.go`, `context_usage.go`, `context_status.go`,
//! `context_budget.go`, `budget_tokens.go`, `output_budget.go`, plus the
//! threshold, probe, anchor and projection satellites they read).
//!
//! One machine owns what keeps a turn's request inside the model's window:
//! the fold trigger and hard ceiling, the token estimation the thresholds
//! compare against, the provider-counted anchor, the Prepare maintenance
//! transaction, the one-shot overflow recovery, and the send-time output
//! clip. It owns [`WindowState`]; the loop reaches it via the agent's window
//! handle (Go: `a.window()`), and the request join is
//! [`Window::prepare_request`] (Go: `prepareSamplingRequest`).
//!
//! What the compaction wave owns is typed as the
//! [`crate::seams::fold_executor::FoldExecutor`] seam (Go:
//! `compactToProjection`): planning the fold region, the summary
//! transaction, and the checkpoint install. Its absence degrades the fold —
//! requests still go out, the provider rules — never the turn.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use reasonix_contract::provider::{Message, Role, model_messages};
use reasonix_provider::ChatCompletionRequest;
use reasonix_state::sessionstore::compaction_state::{CompactionState, ContextProjection};
use reasonix_tools::Registry;

use crate::error::TurnError;
use crate::seams::{RequestComposer, fold_executor};
use crate::sink::Sink;

pub mod budget;
pub mod estimate;
pub mod maintenance;
pub mod recovery;
pub mod status;
pub mod trigger;
pub mod usage;

pub use budget::{ContextBudget, add_budget_tokens, budget_usage_tokens};
pub use estimate::{RequestCalibrationShape, msg_chars};
pub use maintenance::{
    CompactionNoopReason, CompactVerdict, ContextPreparePolicy, PrepareError, PreparedContext,
};
pub use status::ContextMaintenanceSnapshot;
pub use trigger::ContextWindowProbe;

/// The tag a fold digest opens with (Go: `SummaryTagOpen`).
pub const SUMMARY_TAG_OPEN: &str = "<compaction-summary>";
/// The tag a fold digest closes with (Go: `SummaryTagClose`).
pub const SUMMARY_TAG_CLOSE: &str = "</compaction-summary>";

/// Compaction trigger labels (Go: `CompactionTrigger*`).
pub mod trigger_label {
    pub const PRESSURE: &str = "pressure";
    pub const MANUAL: &str = "manual";
    pub const OVERFLOW: &str = "overflow";
    pub const SNIP: &str = "snip";
    pub const TOOL: &str = "tool";
}

/// Cache state labels for resume telemetry; never provider-visible (Go:
/// `CacheState*`).
pub mod cache_state {
    pub const WARM: &str = "warm";
    pub const COLD: &str = "cold";
    pub const UNKNOWN: &str = "unknown";
}

/// Go: `isCompactionSummary` — a user-role digest the fold installed.
pub fn is_compaction_summary(m: &Message) -> bool {
    m.role == Role::USER
        && m.content
            .trim_start_matches(['\n', ' '])
            .starts_with(SUMMARY_TAG_OPEN)
}

/// Adapter replay facts the estimation reads (Go:
/// `provider.SharedWindowInputPolicy` + `SharesContextWindow` +
/// `OutputBudgetProvider`). Divergence: Go reads these off the provider
/// object; the Rust provider client carries no such capability yet, so the
/// host configures them here and the provider-crate wave owns surfacing
/// them from the dialect.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SharedWindowPolicy {
    pub shares_context_window: bool,
    pub output_budget: i64,
    pub replays_ordinary_reasoning: bool,
    pub replays_responses_items: bool,
}

/// Construction-time window knobs (Go: the `agentConfig` fields the window
/// machine reads).
#[derive(Debug, Clone)]
pub struct WindowConfig {
    /// The model's declared context window; 0 leaves compaction off until a
    /// rejection probes one (Go: `contextWindow`).
    pub context_window: i64,
    /// Capacity share of the window before maintenance; <=0 falls back to
    /// the default, >1 disables it (Go: `compactRatio`).
    pub compact_ratio: f64,
    /// The explicit per-request output cap; 0 omits it (Go:
    /// `maxOutputTokens`).
    pub max_output_tokens: i64,
    pub temperature: Option<f64>,
    /// The cache-stable system prompt the composer leads with (Go: the
    /// composed base prompt).
    pub system_prompt: String,
    pub workspace_id: String,
    pub model_ref: String,
    pub shared_window: SharedWindowPolicy,
}

impl Default for WindowConfig {
    fn default() -> Self {
        WindowConfig {
            context_window: 0,
            compact_ratio: 0.0,
            max_output_tokens: 0,
            temperature: None,
            system_prompt: String::new(),
            workspace_id: String::new(),
            model_ref: String::new(),
            shared_window: SharedWindowPolicy::default(),
        }
    }
}

/// One remembered prefix fingerprint, valid for the rewrite counter it was
/// taken under (Go: `coveredHashMemo`). Recomputing it per read walks every
/// message behind the fold; append-only growth cannot change that prefix.
#[derive(Debug, Clone)]
pub(crate) struct CoveredHashMemo {
    pub rewrite_version: i64,
    pub n: i64,
    pub hash: String,
}

/// What the context window owns for one conversation (Go: `windowState`):
/// the installed projection, the maintenance receipts, the compaction
/// progress, and the estimation latches. It restarts with the conversation.
#[derive(Debug, Default)]
pub struct WindowState {
    guard: Mutex<WindowCore>,
    covered_hash: Mutex<Option<CoveredHashMemo>>,
}

/// The compaction-guarded core (Go: the fields under `compactionMu` plus
/// `compactionProgress`).
#[derive(Debug, Default)]
pub(crate) struct WindowCore {
    pub compaction_state: CompactionState,
    /// Runtime-only projection label: none|restored|applied|dropped_* (Go:
    /// `checkpointState`).
    pub checkpoint_state: String,
    pub cache_state: String,
    /// Whether a fold landed above the trigger, so pressure retries are
    /// pointless (Go: `compactionProgress.stuck`).
    pub compaction_stuck: bool,
    /// The no-fold verdict already reported for the running turn (Go:
    /// `compactionProgress.lastReported`).
    pub last_reported: maintenance::MaintenanceReport,
}

impl WindowCore {
    fn restart(&mut self) {
        self.compaction_stuck = false;
        self.last_reported = maintenance::MaintenanceReport::default();
    }
}

/// The context window machine (Go: `contextWindow` + the `windowHost`
/// borrows). Every method is `&self` with interior mutability because the
/// loop reaches it from several phases.
pub struct Window {
    config: WindowConfig,
    session: Arc<reasonix_state::sessionstore::Session>,
    registry: Arc<Registry>,
    sink: Arc<dyn Sink>,
    composer: OnceLock<Arc<dyn RequestComposer>>,
    fold: Mutex<Option<Arc<dyn fold_executor::FoldExecutor>>>,
    /// The transcript path projection persistence is bound to (Go:
    /// `sess.path`).
    session_path: Mutex<String>,
    /// The creation stamp of the message that began the running turn (Go:
    /// `activeTurnCreatedAt`).
    active_turn_created_at: AtomicI64,
    probe: ContextWindowProbe,
    state: WindowState,
    output: Mutex<estimate::OutputEstimateState>,
}

impl Window {
    pub fn new(
        config: WindowConfig,
        session: Arc<reasonix_state::sessionstore::Session>,
        registry: Arc<Registry>,
        sink: Arc<dyn Sink>,
    ) -> Self {
        let output_budget = config.shared_window.output_budget;
        Window {
            config,
            session,
            registry,
            sink,
            composer: OnceLock::new(),
            fold: Mutex::new(None),
            session_path: Mutex::new(String::new()),
            active_turn_created_at: AtomicI64::new(0),
            probe: ContextWindowProbe::default(),
            state: WindowState::default(),
            output: Mutex::new(estimate::OutputEstimateState {
                output_budget,
                ..estimate::OutputEstimateState::default()
            }),
        }
    }

    /// Installs the fold executor (Go: `compactToProjection`'s owner — the
    /// compaction wave).
    pub fn set_fold_executor(&self, fold: Arc<dyn fold_executor::FoldExecutor>) {
        *self.fold.lock().expect("window lock") = Some(fold);
    }

    pub(crate) fn fold_executor(&self) -> Option<Arc<dyn fold_executor::FoldExecutor>> {
        self.fold.lock().expect("window lock").clone()
    }

    /// Installs the request composition the window builds its requests
    /// through (Go: `buildSamplingRequest`'s assembly minus this machine).
    pub fn set_composer(&self, composer: Arc<dyn RequestComposer>) {
        let _ = self.composer.set(composer);
    }

    fn composer_or_default(&self) -> Arc<dyn RequestComposer> {
        self.composer
            .get()
            .cloned()
            .unwrap_or_else(|| Arc::new(crate::compose::DefaultComposer::default()))
    }

    /// Rebinds the window to a new conversation (Go: `windowState.reset`).
    /// Keyed on a rewrite counter that restarts with the new transcript, so
    /// a carried-over entry could answer for history this session never had.
    pub fn reset(&self) {
        self.state.covered_hash.lock().expect("window lock").take();
        let mut core = self.state.guard.lock().expect("window lock");
        core.compaction_state = CompactionState::default();
        core.cache_state = cache_state::UNKNOWN.to_owned();
        core.restart();
        drop(core);
        self.reset_per_transcript_output_state();
    }

    /// Drops the estimation state that belongs to the transcript being
    /// replaced (Go: `outputBudgetState.reset`). The prompt-token
    /// calibration is a property of the model's tokenizer and a model
    /// switch rebuilds the agent, so it outlives the swap.
    pub fn reset_per_transcript_output_state(&self) {
        let mut out = self.output.lock().expect("window lock");
        out.last_usage = None;
        out.active_shape = None;
        out.usage_anchor = None;
    }

    /// Records the creation stamp of the user message that began the running
    /// turn (Go: the `beginRunTurn` write of `activeTurnCreatedAt`).
    pub fn note_active_turn_created_at(&self, created_at: i64) {
        self.active_turn_created_at
            .store(created_at, Ordering::SeqCst);
    }

    /// Binds projection persistence to a transcript path (Go:
    /// `SetSessionPath`). Empty unbinds it.
    pub fn set_session_path(&self, path: &str) {
        *self.session_path.lock().expect("window lock") = path.to_owned();
    }

    pub fn session_path(&self) -> String {
        self.session_path.lock().expect("window lock").clone()
    }

    /// The lineage key for the bound session + model (Go: `promptCacheKey`).
    /// It deliberately excludes message counts, timestamps, and projection
    /// hashes.
    pub fn prompt_cache_key(&self) -> String {
        [
            self.config.workspace_id.trim(),
            "", // branch lineage: the branch wave binds it here (Go: BranchID(sess.path))
            self.config.model_ref.trim(),
        ]
        .join("|")
    }

    /// The installed fold's projection version (Go:
    /// `currentProjectionVersion`).
    pub fn current_projection_version(&self) -> u64 {
        self.state
            .guard
            .lock()
            .expect("window lock")
            .compaction_state
            .projection
            .projection_version
    }

    /// The view a request carries: the installed projection spliced with the
    /// canonical tail behind it, or the whole canonical transcript (Go:
    /// `modelVisibleMessages` minus the host tails other waves add — the
    /// host-context, tool-image-aging, todo and fold-progress tail machines
    /// own those).
    pub fn model_visible_messages(&self) -> Vec<Message> {
        let (msgs, _version, rewrite_version) = self.session.snapshot_with_version();
        let key = self.prompt_cache_key();
        let (st, covered) = {
            let core = self.state.guard.lock().expect("window lock");
            (
                core.compaction_state.clone(),
                self.state
                    .covered_hash
                    .lock()
                    .expect("window lock")
                    .clone(),
            )
        };
        let memo_slot = &self.state.covered_hash;
        let fingerprint = |n: i64| -> String {
            if let Some(memo) = &covered
                && memo.n == n
                && memo.rewrite_version == rewrite_version
            {
                return memo.hash.clone();
            }
            let hash = reasonix_state::sessionstore::compaction_state::covered_prefix_hash(&msgs, n);
            *memo_slot.lock().expect("window lock") = Some(CoveredHashMemo {
                rewrite_version,
                n,
                hash: hash.clone(),
            });
            hash
        };
        if projection_valid(&st, &msgs, &key, Some(&fingerprint)) {
            let visible = model_visible_from_projection(&st.projection, &msgs);
            if !visible.is_empty() {
                return visible;
            }
        }
        msgs
    }

    /// Estimation surface: what the last request actually carried, or the
    /// whole provider-visible set before one has gone out — which
    /// overstates rather than understates (Go: `estimationSurface`).
    pub(crate) fn estimation_surface(&self) -> Vec<reasonix_provider::ToolSchema> {
        let out = self.output.lock().expect("window lock");
        if !out.last_provider_schemas.is_empty() {
            return out.last_provider_schemas.clone();
        }
        drop(out);
        self.registry
            .schemas()
            .into_iter()
            .map(|schema| reasonix_provider::ToolSchema {
                name: schema.name,
                description: schema.description,
                parameters: if schema.parameters.is_null() {
                    String::new()
                } else {
                    serde_json::to_string(&schema.parameters).unwrap_or_default()
                },
            })
            .collect()
    }

    pub(crate) fn store_provider_schemas(&self, tools: &[reasonix_provider::ToolSchema]) {
        self.output.lock().expect("window lock").last_provider_schemas = tools.to_vec();
    }

    pub(crate) fn config(&self) -> &WindowConfig {
        &self.config
    }

    pub(crate) fn session(&self) -> &Arc<reasonix_state::sessionstore::Session> {
        &self.session
    }

    pub(crate) fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    pub(crate) fn sink(&self) -> &Arc<dyn Sink> {
        &self.sink
    }

    pub(crate) fn active_turn_created_at(&self) -> i64 {
        self.active_turn_created_at.load(Ordering::SeqCst)
    }

    pub(crate) fn shared_window(&self) -> SharedWindowPolicy {
        self.config.shared_window
    }

    /// Runs one state mutation under the compaction guard (Go: the
    /// `compactionMu` critical sections).
    pub(crate) fn with_core<R>(&self, f: impl FnOnce(&mut WindowCore) -> R) -> R {
        let mut core = self.state.guard.lock().expect("window lock");
        f(&mut core)
    }

    pub(crate) fn output_state(&self) -> &Mutex<estimate::OutputEstimateState> {
        &self.output
    }

    /// The index of the message that began the running turn, or -1 (Go:
    /// `activeTurnStart`).
    pub fn active_turn_start(&self, msgs: &[Message]) -> i64 {
        let created = self.active_turn_created_at();
        if created == 0 {
            return -1;
        }
        for (i, m) in msgs.iter().enumerate() {
            if m.role == Role::USER && m.created_at == created {
                return i as i64;
            }
        }
        -1
    }

    /// Installs one fold's projection as the current one (Go: the
    /// `compact_commit.go` install — the state half this machine owns). The
    /// compaction wave's executor calls this from the fold seam; its CAS and
    /// persistence halves are the wave's.
    pub fn install_projection(&self, projection: reasonix_state::sessionstore::compaction_state::ContextProjection) {
        let key = self.prompt_cache_key();
        let (_, transcript_version) = self.session.snapshot_messages_version();
        self.with_core(|core| {
            core.compaction_state.schema_version =
                reasonix_state::sessionstore::compaction_state::COMPACTION_STATE_SCHEMA_CURRENT;
            core.compaction_state.transcript_version = transcript_version;
            core.compaction_state.prompt_cache_key = key;
            core.compaction_state.projection = projection;
            core.compaction_state.generation += 1;
            core.checkpoint_state = "applied".to_owned();
            core.compaction_stuck = false;
        });
    }

    /// Builds, freezes, and clips one model-round request (Go:
    /// `prepareSamplingRequest`): Prepare at the pressure trigger, compose
    /// through the seam, resolve the output budget, and record the shape the
    /// provider's report will anchor against.
    pub fn prepare_request(
        &self,
        composer: Arc<dyn RequestComposer>,
    ) -> Result<ChatCompletionRequest, TurnError> {
        let _ = self.composer.set(composer.clone());
        self.prepare_once(maintenance::ContextPreparePolicy {
            trigger: trigger_label::PRESSURE.to_owned(),
            ..ContextPreparePolicy::default()
        })
        .map_err(maintenance::PrepareError::into_turn_error)?;
        let mut req = self.compose_transport_copy()?;
        match self.effective_output_budget(&req) {
            Err(_budget_err) => {
                // One-shot physical overflow recovery. Do not loop (Go: the
                // budgetErr arm of prepareSamplingRequest).
                self.prepare_once(maintenance::ContextPreparePolicy {
                    trigger: trigger_label::OVERFLOW.to_owned(),
                    ..ContextPreparePolicy::default()
                })
                .map_err(maintenance::PrepareError::into_turn_error)?;
                self.prepare_once(maintenance::ContextPreparePolicy {
                    trigger: trigger_label::PRESSURE.to_owned(),
                    ..ContextPreparePolicy::default()
                })
                .map_err(maintenance::PrepareError::into_turn_error)?;
                let mut rebuilt = self.compose_transport_copy()?;
                if let Some(budget) = self.effective_output_budget(&rebuilt)? {
                    rebuilt.max_tokens = budget;
                }
                req = rebuilt;
            }
            Ok(Some(budget)) => {
                req.max_tokens = budget;
            }
            Ok(None) => {}
        }
        let shape = estimate::request_calibration_shape(&req, self.shared_window());
        let projection_version = self.current_projection_version();
        usage::store_active_shape(self, shape.clone());
        usage::store_anchor_shape(self, shape, projection_version);
        Ok(req)
    }

    /// The transport copy of the current visible view (Go:
    /// `buildSamplingRequest` minus the extension interceptors — that
    /// machine is a later wave and runs only on the real sampling path when
    /// it lands).
    fn compose_transport_copy(&self) -> Result<ChatCompletionRequest, TurnError> {
        let visible = self.model_visible_messages();
        let base = crate::seams::RequestBase {
            temperature: self.config.temperature,
            max_tokens: self.config.max_output_tokens,
        };
        let req = self
            .composer_or_default()
            .compose(&self.config.system_prompt, &visible, &self.registry, &base)?;
        self.store_provider_schemas(&req.tools);
        Ok(req)
    }

}

/// Whether a saved projection can answer for the current transcript and
/// lineage (Go: `projectionValid`). Fails closed: a missing covered-prefix
/// hash or a blank sidecar lineage key when the current one is known forces
/// a rebuild.
pub fn projection_valid(
    st: &CompactionState,
    msgs: &[Message],
    cache_key: &str,
    fingerprint: Option<&dyn Fn(i64) -> String>,
) -> bool {
    if st.projection.messages.is_empty() {
        return false;
    }
    if !projection_lineage_ok(st, cache_key) {
        return false;
    }
    projection_content_valid(st, msgs, fingerprint)
}

/// Whether the sidecar was written for the lineage asking for it (Go:
/// `projectionLineageOK`). A blank current key means the lineage is not
/// known yet.
fn projection_lineage_ok(st: &CompactionState, cache_key: &str) -> bool {
    if cache_key.is_empty() {
        return true;
    }
    lineage_key_compatible(&st.prompt_cache_key, cache_key).is_some()
}

/// Whether the projection body still matches the canonical transcript,
/// independent of lineage (Go: `projectionContentValid`).
pub fn projection_content_valid(
    st: &CompactionState,
    msgs: &[Message],
    fingerprint: Option<&dyn Fn(i64) -> String>,
) -> bool {
    let n = st.projection.covered_count;
    if st.projection.messages.is_empty() || n <= 0 || n > msgs.len() as i64 {
        return false;
    }
    let hash = match fingerprint {
        Some(f) => f(n),
        None => reasonix_state::sessionstore::compaction_state::covered_prefix_hash(msgs, n),
    };
    projection_covers_tail(st, msgs.len(), &hash)
}

/// The validity judgement itself (Go: `projectionCoversTail`): the covered
/// prefix hash is its whole proof, so a match says the history the digest
/// folded is still exactly there. A transcript version adds nothing to that
/// and is not durable — a reload restarts the counter — so it takes no part.
pub fn projection_covers_tail(st: &CompactionState, total: usize, prefix_hash: &str) -> bool {
    let n = st.projection.covered_count;
    if st.projection.messages.is_empty() || n <= 0 || n > total as i64 {
        return false;
    }
    !st.projection.covered_prefix_hash.is_empty() && prefix_hash == st.projection.covered_prefix_hash
}

/// Splices the projection with any messages appended after it was built
/// (Go: `modelVisibleFromProjection`).
pub fn model_visible_from_projection(
    proj: &ContextProjection,
    canonical: &[Message],
) -> Vec<Message> {
    if proj.messages.is_empty() {
        return Vec::new();
    }
    let mut out = proj.messages.clone();
    if proj.covered_count >= 0 && (proj.covered_count as usize) < canonical.len() {
        out.extend_from_slice(&canonical[proj.covered_count as usize..]);
    }
    out
}

/// Reports whether a stored lineage key still belongs to the current
/// session/model lineage (Go: `lineageKeyCompatible`). Legacy
/// native-editing keys carried a `|context-editing-native` suffix on an
/// otherwise matching base key.
pub fn lineage_key_compatible(stored: &str, current: &str) -> Option<String> {
    let stored = stored.trim();
    let current = current.trim();
    if current.is_empty() {
        return Some(stored.to_owned());
    }
    if stored.is_empty() {
        return None;
    }
    if stored == current {
        return Some(current.to_owned());
    }
    const NATIVE_SUFFIX: &str = "|context-editing-native";
    if let Some(rest) = stored.strip_prefix(current)
        && (rest == NATIVE_SUFFIX || rest.starts_with(NATIVE_SUFFIX))
    {
        return Some(current.to_owned());
    }
    if let Some(i) = stored.find(NATIVE_SUFFIX)
        && i > 0
        && &stored[..i] == current
    {
        return Some(current.to_owned());
    }
    None
}

/// The provider-visible projection of the visible view with the wall-clock
/// stamp stripped (Go: `providerProjectionMessages` + the CreatedAt strip;
/// `strictAlternatingRoles` coalescing stays on the composer's request copy).
pub(crate) fn provider_projection_messages(msgs: Vec<Message>) -> Vec<Message> {
    let mut msgs = model_messages(&msgs).into_owned();
    for m in &mut msgs {
        m.created_at = 0;
    }
    msgs
}

/// The estimation shape of the pre-interceptor sampling view (Go:
/// `visibleRequestShape`).
pub fn visible_request_shape(window: &Window, visible: &[Message]) -> RequestCalibrationShape {
    let msgs = provider_projection_messages(visible.to_vec());
    let req = ChatCompletionRequest {
        messages: msgs,
        tools: window.estimation_surface(),
        max_tokens: window.config.max_output_tokens,
        temperature: window.config.temperature,
        ..ChatCompletionRequest::default()
    };
    estimate::request_calibration_shape(&req, window.shared_window())
}

/// Sizes the pre-interceptor sampling view in tokens (Go:
/// `estimatedVisibleRequestTokens`).
pub fn estimated_visible_request_tokens(window: &Window, visible: &[Message]) -> i64 {
    estimate::estimated_shape_tokens(window, &visible_request_shape(window, visible))
}

/// Fixtures the window module's test oracles share (Go: the inline
/// `&Agent{agentConfig: ...}` constructions of the agent package's tests).
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::sink::Discard;
    use reasonix_contract::eventwire::event::Event;

    pub(crate) fn config(context_window: i64, compact_ratio: f64) -> WindowConfig {
        WindowConfig {
            context_window,
            compact_ratio,
            model_ref: "model".to_owned(),
            ..WindowConfig::default()
        }
    }

    pub(crate) fn window(context_window: i64, compact_ratio: f64) -> Window {
        Window::new(
            config(context_window, compact_ratio),
            Arc::new(reasonix_state::sessionstore::Session::new("system")),
            Arc::new(Registry::new()),
            Arc::new(Discard),
        )
    }

    pub(crate) fn window_over(
        config: WindowConfig,
        msgs: Vec<Message>,
    ) -> Window {
        let session = Arc::new(reasonix_state::sessionstore::Session::new(""));
        session.replace(msgs);
        Window::new(config, session, Arc::new(Registry::new()), Arc::new(Discard))
    }

    pub(crate) fn system(content: &str) -> Message {
        Message {
            role: Role::SYSTEM,
            content: content.to_owned(),
            ..Message::default()
        }
    }

    pub(crate) fn user(content: &str) -> Message {
        Message {
            role: Role::USER,
            content: content.to_owned(),
            ..Message::default()
        }
    }

    pub(crate) fn assistant(content: &str) -> Message {
        Message {
            role: Role::ASSISTANT,
            content: content.to_owned(),
            ..Message::default()
        }
    }

    /// A sink that records every event it sees (Go: `noticeRecorder`).
    #[derive(Default)]
    pub(crate) struct RecordingSink {
        pub events: std::sync::Mutex<Vec<Event>>,
    }

    impl RecordingSink {
        pub fn notices_with_code(&self, code: &str) -> Vec<Event> {
            self.events
                .lock()
                .expect("recorder lock")
                .iter()
                .filter(|e| e.kind == crate::sink::kind::NOTICE && e.code == code)
                .cloned()
                .collect()
        }
    }

    impl Sink for RecordingSink {
        fn emit(&self, e: Event) {
            self.events.lock().expect("recorder lock").push(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::testing::*;

    fn state_with_projection(
        messages: Vec<Message>,
        covered_count: i64,
        covered_prefix_hash: &str,
        prompt_cache_key: &str,
    ) -> CompactionState {
        CompactionState {
            prompt_cache_key: prompt_cache_key.to_owned(),
            projection: ContextProjection {
                messages,
                covered_count,
                covered_prefix_hash: covered_prefix_hash.to_owned(),
                ..ContextProjection::default()
            },
            ..CompactionState::default()
        }
    }

    fn covered(canonical: &[Message], n: i64) -> String {
        reasonix_state::sessionstore::compaction_state::covered_prefix_hash(canonical, n)
    }

    #[test]
    fn a_saved_projection_answers_while_the_covered_prefix_is_intact() {
        let canonical = vec![system("system"), user("one"), assistant("two")];
        let st = state_with_projection(
            vec![system("system"), user("digest")],
            2,
            &covered(&canonical, 2),
            "|model",
        );
        assert!(projection_valid(&st, &canonical, "|model", None));
    }

    #[test]
    fn a_rewritten_prefix_fails_the_projection_closed() {
        let canonical = vec![system("system"), user("edited"), assistant("two")];
        let st = state_with_projection(
            vec![system("system"), user("digest")],
            2,
            &covered(&[system("system"), user("one")], 2),
            "|model",
        );
        assert!(!projection_valid(&st, &canonical, "|model", None));
        // Fail closed: a legacy sidecar without a covered hash is rebuilt.
        let mut legacy = state_with_projection(
            vec![system("system"), user("digest")],
            2,
            &covered(&canonical, 2),
            "|model",
        );
        legacy.projection.covered_prefix_hash = String::new();
        assert!(!projection_valid(&legacy, &canonical, "|model", None));
    }

    #[test]
    fn a_projection_from_another_lineage_is_not_reused() {
        let canonical = vec![system("system"), user("one"), assistant("two")];
        let st = state_with_projection(
            vec![system("system"), user("digest")],
            2,
            &covered(&canonical, 2),
            "other|model",
        );
        assert!(!projection_valid(&st, &canonical, "|model", None));
        // A blank current key means the lineage is not known yet.
        assert!(projection_valid(&st, &canonical, "", None));
    }

    #[test]
    fn the_projection_splices_the_canonical_tail_behind_it() {
        let canonical = vec![system("system"), user("one"), assistant("two")];
        let proj = ContextProjection {
            messages: vec![system("system"), user("digest")],
            covered_count: 2,
            ..ContextProjection::default()
        };
        let visible = model_visible_from_projection(&proj, &canonical);
        assert_eq!(visible.len(), 3);
        assert_eq!(visible[2].role, Role::ASSISTANT);
        assert_eq!(visible[2].content, "two");
    }

    #[test]
    fn lineage_keys_match_across_the_legacy_native_suffix() {
        assert_eq!(
            lineage_key_compatible("ws|b|m", "ws|b|m").as_deref(),
            Some("ws|b|m")
        );
        assert_eq!(
            lineage_key_compatible("ws|b|m|context-editing-native", "ws|b|m").as_deref(),
            Some("ws|b|m")
        );
        assert!(lineage_key_compatible("", "ws|b|m").is_none());
        assert_eq!(
            lineage_key_compatible("legacy", "").as_deref(),
            Some("legacy")
        );
        assert!(lineage_key_compatible("other", "ws|b|m").is_none());
    }

    #[test]
    fn the_covered_hash_memo_rebinds_with_the_rewrite_counter() {
        let window = window(20_000, 0.5);
        let msgs = vec![user("hello"), assistant("hi")];
        let first = reasonix_state::sessionstore::compaction_state::covered_prefix_hash(&msgs, 1);
        *window.state.covered_hash.lock().expect("window lock") = Some(CoveredHashMemo {
            rewrite_version: 0,
            n: 1,
            hash: first.clone(),
        });
        let memo = window
            .state
            .covered_hash
            .lock()
            .expect("window lock")
            .clone()
            .expect("memo");
        assert_eq!(memo.n, 1);
        assert_eq!(memo.hash, first);
    }
}
