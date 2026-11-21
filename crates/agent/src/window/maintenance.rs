//! The Prepare maintenance transaction (Go: `context_manager.go`,
//! `context_receipt.go`).
//!
//! The window is the sole owner of provider-visible context maintenance.
//! Canonical session messages are immutable inputs; Prepare evolves only
//! the durable projection and returns the exact visible view for one
//! sampling round. Below `compact_ratio` it does nothing. At or above the
//! trigger it runs one summary transaction — the compaction wave's
//! [`crate::seams::fold_executor::FoldExecutor`] — that either installs a
//! checkpoint or records a generation-scoped blocked/failed receipt.

use reasonix_contract::eventwire::event::Event;
use reasonix_contract::provider::Message;
use reasonix_state::sessionstore::compaction_state::{
    COMPACTION_STATE_SCHEMA_CURRENT, ContextMaintenanceReceipt,
};
use sha2::{Digest, Sha256};

use crate::error::TurnError;
use crate::seams::fold_executor::{FoldError, FoldRequest, FoldScope};

use super::trigger_label;
use super::{Window, estimated_visible_request_tokens, visible_request_shape};
use super::usage;

/// Identifies why an attempt folded nothing (Go: `CompactionNoopReason`).
/// The code is the identity a frontend matches; the sentence is a
/// diagnostic. Closed set — a verdict nobody declared has no code.
pub type CompactionNoopReason = &'static str;

/// Go: the `Noop*` and `Fail*` code set.
pub mod noop {
    use super::CompactionNoopReason;

    pub const NO_NEW_CLOSED_PREFIX: CompactionNoopReason = "no_new_closed_prefix";
    pub const ACTIVE_TURN_BOUNDARY: CompactionNoopReason = "active_turn_boundary";
    pub const NO_FOLDABLE_REGION: CompactionNoopReason = "no_foldable_region";
    pub const FOLD_BELOW_ECONOMICS: CompactionNoopReason = "fold_below_economics";
    pub const INPUT_UNCHANGED: CompactionNoopReason = "input_unchanged";
    pub const FOLD_EMPTY_AFTER_HOOKS: CompactionNoopReason = "fold_empty_after_hooks";
    pub const FIXED_PREFIX_ABOVE_TRIGGER: CompactionNoopReason = "fixed_prefix_above_trigger";

    pub const CANDIDATE_NOT_SMALLER: CompactionNoopReason = "candidate_not_smaller";
    pub const CANDIDATE_ABOVE_CEILING: CompactionNoopReason = "candidate_above_ceiling";
    pub const CANDIDATE_ABOVE_TRIGGER: CompactionNoopReason = "candidate_above_trigger";
    pub const CANDIDATE_ABOVE_PHYSICAL: CompactionNoopReason = "candidate_above_physical_ceiling";
    pub const SAVINGS_BELOW_MINIMUM: CompactionNoopReason = "savings_below_minimum";

    pub const SUMMARY_FAILED: CompactionNoopReason = "summary_failed";
    pub const SUMMARY_TIMEOUT: CompactionNoopReason = "summary_timeout";
    pub const SUMMARY_CEILING: CompactionNoopReason = "summary_ceiling";
    pub const SUMMARY_TRUNCATED: CompactionNoopReason = "summary_truncated";
    pub const SUMMARY_NOT_DIGEST: CompactionNoopReason = "summary_not_digest";
    pub const CONTEXT_CHANGED: CompactionNoopReason = "context_changed";
    pub const HOOK_REFUSED: CompactionNoopReason = "hook_refused";
    pub const PERSIST_FAILED: CompactionNoopReason = "persist_failed";
    pub const RESULT_ABOVE_TRIGGER: CompactionNoopReason = "result_above_trigger";
    pub const CANCELLED: CompactionNoopReason = "cancelled";
    pub const UNCLASSIFIED: CompactionNoopReason = "unclassified";
    pub const BUSY: CompactionNoopReason = "busy";

    /// A failure of the summary request itself, which a later attempt can
    /// answer differently (Go: `transientSummaryFailure`). Every other
    /// class is a property of the input or the install, so retrying the
    /// same input cannot change it.
    pub fn transient_summary_failure(code: &str) -> bool {
        matches!(
            code,
            SUMMARY_TRUNCATED | SUMMARY_TIMEOUT | SUMMARY_CEILING | SUMMARY_FAILED
                | SUMMARY_NOT_DIGEST
        )
    }

    /// A failure the input's growth releases: the summary request failed,
    /// the checkpoint was refused for the protected content it would carry,
    /// or the fold lost the race to a transcript append (Go:
    /// `retriesOnGrowth`). Anything else holds, because retrying the same
    /// input cannot change it.
    pub fn retries_on_growth(code: &str) -> bool {
        transient_summary_failure(code)
            || code == CANDIDATE_ABOVE_CEILING
            || code == CONTEXT_CHANGED
    }
}

/// Describes one maintenance transaction (Go: `ContextPreparePolicy`).
#[derive(Debug, Clone, Default)]
pub struct ContextPreparePolicy {
    pub trigger: String,
    pub instructions: String,
    /// Folds without waiting for the automatic trigger.
    pub ignore_threshold: bool,
    /// Folds even when the saving does not pay for it. One bool for both
    /// would let a hand-typed compact re-summarize an unchanged context.
    pub ignore_economics: bool,
    /// Pins the input size instead of estimating it. Tests use it to
    /// trigger a fold at an exact size; production reads the calibrated
    /// shape, which is anchored on real provider counts.
    pub observed_input_tokens: i64,
}

/// Answers a maintenance request (Go: `CompactVerdict`). Declining is an
/// answer and not a failure: `reason` names the economics that declined.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompactVerdict {
    /// Says a projection was written. A verdict nobody filled in must not
    /// claim a fold happened.
    pub installed: bool,
    pub reason: CompactionNoopReason,
}

/// The frozen result of a successful Prepare transaction (Go:
/// `PreparedContext`).
#[derive(Debug, Clone, Default)]
pub struct PreparedContext {
    pub messages: Vec<Message>,
    pub input_tokens: i64,
    pub projection_version: u64,
    /// What this preparation did to the context, and when it did nothing,
    /// which economics declined it.
    pub maintenance: CompactVerdict,
}

/// How a maintenance transaction failed (Go: the error surface `Prepare`
/// returns). The loop sees only `CompactionRequired` and `Cancelled`; the
/// raw fold error is a manual-trigger answer, and a caller that reports it
/// to a frontend uses `code`, never the sentence.
#[derive(Debug, Clone, PartialEq)]
pub enum PrepareError {
    /// Go: `ErrCompactionRequired` — the prompt exceeds the provider limit
    /// and compaction could not produce a usable projection.
    CompactionRequired { reason: String },
    /// Go: the fold transaction's own error, surfaced to manual callers.
    Fold(FoldError),
    /// The caller cancelled the turn (Go: `context.Canceled`).
    Cancelled,
}

impl PrepareError {
    /// The loop's projection of a Prepare failure (Go: `Run`'s error
    /// surface). The raw fold error never reaches the loop: non-manual
    /// triggers degrade or become compaction-required inside the machine.
    pub fn into_turn_error(self) -> TurnError {
        match self {
            PrepareError::CompactionRequired { reason } => {
                TurnError::CompactionRequired { reason }
            }
            PrepareError::Cancelled => TurnError::Cancelled,
            // The spine only runs pressure and overflow triggers, whose
            // fold failures the machine itself resolves; a raw fold error
            // here means a manual caller reached into the loop path.
            PrepareError::Fold(err) => TurnError::Composition(format!("{err}")),
        }
    }

    /// Go: `IsCompactionDeclined` — a verdict, not a failure.
    pub fn is_compaction_declined(&self) -> bool {
        matches!(
            self,
            PrepareError::Fold(FoldError::CheckpointRejected { .. })
        )
    }
}

impl std::fmt::Display for PrepareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrepareError::CompactionRequired { reason } => write!(
                f,
                "context exceeds provider limit and compaction failed: {reason}"
            ),
            PrepareError::Fold(err) => write!(f, "{err}"),
            PrepareError::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// One reported no-fold verdict, scoped to the turn it was reached under:
/// the same reason in a later turn is news again (Go: `maintenanceReport`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MaintenanceReport {
    pub reason: String,
    pub turn: i64,
}

impl Window {
    /// The sole automatic maintenance entry (Go: `ContextManager.Prepare`).
    pub fn prepare(
        &self,
        mut policy: ContextPreparePolicy,
    ) -> Result<PreparedContext, PrepareError> {
        if policy.trigger.is_empty() {
            policy.trigger = trigger_label::PRESSURE.to_owned();
        }
        self.prepare_once(policy)
    }

    /// The one-shot public entry for callers that need a Prepare without
    /// sampling (Go: `PrepareContext`).
    pub fn prepare_context(&self) -> Result<(), PrepareError> {
        self.prepare(ContextPreparePolicy {
            trigger: trigger_label::PRESSURE.to_owned(),
            ..ContextPreparePolicy::default()
        })
        .map(|_| ())
    }

    /// The threshold decision (Go: `prepareOnce`).
    pub(crate) fn prepare_once(
        &self,
        policy: ContextPreparePolicy,
    ) -> Result<PreparedContext, PrepareError> {
        let visible = self.model_visible_messages();
        // Threshold uses the stable pre-interceptor request shape (messages
        // + tools + role projection). Extension interceptors run only on
        // the real sampling request so side-effecting plugins are not
        // double-invoked; if they expand the prompt past the hard ceiling,
        // overflow recovery still fires.
        let visible_shape = visible_request_shape(self, &visible);
        let est = estimated_visible_request_tokens(self, &visible);
        let own_est = est; // before an observation that may count provider-injected content
        let mut prepared = PreparedContext {
            messages: visible.clone(),
            input_tokens: est,
            projection_version: self.current_projection_version(),
            maintenance: CompactVerdict::default(),
        };
        if self.effective_context_window() <= 0 || visible.is_empty() {
            return Ok(prepared);
        }
        let fold = self.compact_trigger();
        let hard = self.hard_input_ceiling();
        // pressure only decides whether to fold; the hard ceiling stays on
        // the local estimate, because a provider count no fold can lower
        // must not end the turn.
        let mut est = est;
        let mut pressure = est;
        if policy.observed_input_tokens > 0 {
            est = policy.observed_input_tokens;
            prepared.input_tokens = est;
            pressure = est;
        } else {
            pressure = usage::prompt_pressure(
                self,
                &visible_shape,
                prepared.projection_version,
                pressure,
            );
        }
        let input_hash = self.context_maintenance_input_hash(&visible);
        let manual = policy.trigger == trigger_label::MANUAL;
        let overflow = policy.trigger == trigger_label::OVERFLOW;
        let (blocked, reason) = self.context_maintenance_blocked(&input_hash, own_est, overflow);
        if blocked && !manual {
            // A generation that freed nothing has nothing left to try, so
            // the request goes out and the provider rules.
            if overflow {
                return Err(PrepareError::CompactionRequired { reason });
            }
            if policy.trigger == trigger_label::PRESSURE && own_est >= fold {
                self.note_maintenance_held();
            }
            return Ok(prepared);
        }
        if pressure < fold {
            self.with_core(|core| core.compaction_stuck = false);
        }
        if self.with_core(|core| core.compaction_stuck) && policy.trigger == trigger_label::PRESSURE
        {
            return Ok(prepared);
        }
        // Asking, overflow and a physical ceiling each waive the trigger.
        // Only the last two, and an explicit force, waive what the fold has
        // to be worth; asking by hand waives only how much was added.
        let scope = FoldScope {
            ignore_threshold: policy.ignore_threshold
                || manual
                || overflow
                || est >= hard,
            ignore_economics: policy.ignore_economics || overflow || est >= hard,
            ignore_amount: manual,
        };
        if pressure < fold && !scope.ignore_threshold {
            return Ok(prepared);
        }
        self.fold_context(prepared, &policy, input_hash, est, own_est, fold, hard, scope)
    }

    /// The fold transaction and its verdict matrix (Go: `foldContext`).
    #[allow(clippy::too_many_arguments)]
    fn fold_context(
        &self,
        prepared: PreparedContext,
        policy: &ContextPreparePolicy,
        input_hash: String,
        est: i64,
        own_est: i64,
        fold: i64,
        hard: i64,
        scope: FoldScope,
    ) -> Result<PreparedContext, PrepareError> {
        let manual = policy.trigger == trigger_label::MANUAL;
        let overflow = policy.trigger == trigger_label::OVERFLOW;
        // Where this function would answer compaction-required, the fold is
        // the only way out and a failed summary must degrade rather than
        // strand the turn.
        let must_free = !manual && (overflow || est >= hard);
        let Some(executor) = self.fold_executor() else {
            // Go-nil seam: no fold can run, so none is attempted and no
            // receipt is written. The request goes out unchanged and the
            // provider rules; overflow recovery answers that the fold freed
            // nothing.
            return Ok(prepared);
        };
        let request = FoldRequest {
            trigger: policy.trigger.clone(),
            instructions: policy.instructions.clone(),
            scope,
            must_free,
        };
        let outcome = match executor.fold(&request) {
            Ok(result) => result,
            Err(FoldError::Cancelled) => {
                // Cancellation is the caller's decision, not a summary that
                // failed: recording it as one would blame the summarizer for
                // this generation, and degrading around it would fold a view
                // nobody is waiting for.
                return Err(PrepareError::Cancelled);
            }
            Err(FoldError::StaleContext) if !manual => {
                // Transcript changed during the summary call: discard the
                // candidate and block this generation so we do not pay for a
                // second summary.
                let reason =
                    "context changed during summary; automatic retry blocked for this generation";
                self.record_context_maintenance_outcome(
                    &input_hash,
                    &policy.trigger,
                    "summary",
                    "blocked",
                    noop::CONTEXT_CHANGED,
                    reason,
                );
                if overflow || est >= hard {
                    return Err(PrepareError::CompactionRequired {
                        reason: reason.to_owned(),
                    });
                }
                return Ok(prepared);
            }
            Err(err) => {
                let (status, code) = failure_code(&err);
                let reason = format!("context summary failed: {err}");
                self.record_context_maintenance_outcome(
                    &input_hash,
                    &policy.trigger,
                    "summary",
                    status,
                    code,
                    &reason,
                );
                if manual {
                    return Err(PrepareError::Fold(err));
                }
                if overflow || est >= hard {
                    // The overflow may not be ours to fold: refusing sends no
                    // request, so the observation never updates and every
                    // later turn decides the same. When our own transcript
                    // fits, let the provider rule instead.
                    if own_est < hard {
                        return Ok(prepared);
                    }
                    return Err(PrepareError::CompactionRequired { reason });
                }
                return Ok(prepared);
            }
        };
        if !outcome.installed {
            let canonical = self.session().snapshot_messages_version().0;
            if policy.trigger == trigger_label::PRESSURE && self.active_turn_start(&canonical) >= 0
            {
                // Saying so is the only record that the attempt happened;
                // this path deliberately does not block the next one.
                self.note_maintenance_noop(
                    &policy.trigger,
                    outcome.noop_reason.unwrap_or(""),
                    est,
                );
                return Ok(prepared);
            }
            let reason = if manual {
                "no foldable region remains"
            } else {
                "context is above the maintenance threshold but no foldable region remains"
            };
            let code = outcome.noop_reason.unwrap_or(noop::NO_FOLDABLE_REGION);
            self.record_context_maintenance_outcome(
                &input_hash,
                &policy.trigger,
                "summary",
                "blocked",
                code,
                reason,
            );
            if manual && est >= fold {
                self.note_context_composition();
            }
            // A request that was declined is answered, not failed: the
            // caller gets the reason the host already settled and says so in
            // its own words. Only a fold nothing else can free is an error.
            if manual && est < hard {
                let mut prepared = prepared;
                prepared.maintenance = CompactVerdict {
                    installed: false,
                    reason: code,
                };
                return Ok(prepared);
            }
            if overflow || est >= hard {
                return Err(PrepareError::CompactionRequired {
                    reason: reason.to_owned(),
                });
            }
            let mut prepared = prepared;
            prepared.maintenance = CompactVerdict {
                installed: false,
                reason: code,
            };
            return Ok(prepared);
        }

        let mut result = self.current_prepared();
        result.maintenance = CompactVerdict {
            installed: true,
            reason: "",
        };
        if manual {
            return Ok(result);
        }
        if result.input_tokens >= fold {
            let reason = format!(
                "summary result remains above fold trigger ({} >= {})",
                result.input_tokens, fold
            );
            let result_hash = self.context_maintenance_input_hash(&result.messages);
            self.record_context_maintenance_outcome(
                &result_hash,
                &policy.trigger,
                "summary",
                "blocked",
                noop::RESULT_ABOVE_TRIGGER,
                &reason,
            );
            self.with_core(|core| core.compaction_stuck = true);
            self.note_context_composition();
            // Only a provider that already refused ends the turn here. Our
            // ceiling is an estimate, and refusing on it turns a window too
            // small to summarize one turn into a failed run the provider
            // would have served.
            if overflow {
                return Err(PrepareError::CompactionRequired { reason });
            }
        }
        Ok(result)
    }

    /// Re-reads the view after a fold installed (Go: `currentPrepared`).
    fn current_prepared(&self) -> PreparedContext {
        let visible = self.model_visible_messages();
        PreparedContext {
            input_tokens: estimated_visible_request_tokens(self, &visible),
            messages: visible,
            projection_version: self.current_projection_version(),
            maintenance: CompactVerdict::default(),
        }
    }

    /// The stable hash of the view a maintenance decision is about (Go:
    /// `contextMaintenanceInputHash`).
    pub fn context_maintenance_input_hash(&self, visible: &[Message]) -> String {
        let fingerprint =
            reasonix_state::sessionstore::compaction_state::provider_visible_fingerprint(&model_visible_wire(
                visible,
            ));
        let seed = format!("{}\n{}", self.prompt_cache_key(), fingerprint);
        let mut hasher = Sha256::new();
        hasher.update(seed.as_bytes());
        hex(&hasher.finalize())
    }

    /// Whether a blocked or failed receipt still suppresses automatic
    /// maintenance at this input size (Go: `blockedReceiptHolds`). A failure
    /// that growth releases, recorded without a size (older sidecars), is
    /// released once.
    pub fn blocked_receipt_holds(
        &self,
        receipt: &ContextMaintenanceReceipt,
        tokens: i64,
    ) -> bool {
        if receipt.status != "blocked" && receipt.status != "failed" {
            return false;
        }
        if noop::transient_summary_failure(&receipt.code) {
            return receipt.input_tokens > 0
                && tokens < receipt.input_tokens + self.retry_growth_step(receipt.input_tokens);
        }
        if noop::retries_on_growth(&receipt.code) {
            // Growth releases a protected-content refusal only below the
            // physical ceiling: past it the request goes out and the
            // provider rules.
            return receipt.input_tokens == 0
                || tokens < receipt.input_tokens + self.retry_growth_step(receipt.input_tokens)
                || tokens >= self.hard_input_ceiling();
        }
        true
    }

    /// How much the input must grow past a transient failure before another
    /// summary request is worth its cost: an eighth of the trigger, never
    /// more than half of what the failure left below the hard ceiling, so
    /// the release point always sits short of it (Go: `retryGrowthStep`).
    pub fn retry_growth_step(&self, failed_at: i64) -> i64 {
        (self.compact_trigger() / 8)
            .min((self.hard_input_ceiling() - failed_at) / 2)
            .max(1)
    }

    /// Whether automatic maintenance is suppressed (Go:
    /// `contextMaintenanceBlocked`). Overflow is a fresh provider refusal,
    /// so it passes a failure growth releases; one overflow recovery per
    /// request bounds the cost.
    pub fn context_maintenance_blocked(
        &self,
        input_hash: &str,
        tokens: i64,
        overflow: bool,
    ) -> (bool, String) {
        let core = self.with_core(|core| {
            (
                core.compaction_state.last_receipt.clone(),
                core.compaction_state.blocked_input_hash.clone(),
                core.compaction_state.blocked_reason.clone(),
            )
        });
        let (receipt, blocked_hash, blocked_reason) = core;
        let Some(receipt) = receipt else {
            // Legacy sidecars may only have BlockedInputHash without a
            // receipt.
            if !blocked_hash.is_empty() && (input_hash.is_empty() || blocked_hash == input_hash) {
                return (true, blocked_reason);
            }
            return (false, String::new());
        };
        if overflow
            && (noop::transient_summary_failure(&receipt.code)
                || (noop::retries_on_growth(&receipt.code) && receipt.input_hash != input_hash))
        {
            return (false, String::new());
        }
        if !self.blocked_receipt_holds(&receipt, tokens) {
            return (false, String::new());
        }
        let reason = if blocked_reason.is_empty() {
            receipt.reason.clone()
        } else {
            blocked_reason
        };
        (true, reason)
    }

    /// Persists a generation-scoped blocked receipt (Go:
    /// `recordContextMaintenanceBlocked`).
    pub fn record_context_maintenance_blocked(
        &self,
        input_hash: &str,
        trigger: &str,
        action: &str,
        code: CompactionNoopReason,
        reason: &str,
    ) {
        self.record_context_maintenance_outcome(input_hash, trigger, action, "blocked", code, reason)
    }

    /// Records blocked or failed for the current generation (Go:
    /// `recordContextMaintenanceOutcome`). Automatic Prepare will not
    /// re-enter summary until the generation advances, or, for a transient
    /// summary failure, the input has grown.
    pub fn record_context_maintenance_outcome(
        &self,
        input_hash: &str,
        trigger: &str,
        action: &str,
        status: &str,
        code: CompactionNoopReason,
        reason: &str,
    ) {
        let input_hash = if input_hash.is_empty() {
            let visible = self.model_visible_messages();
            self.context_maintenance_input_hash(&visible)
        } else {
            input_hash.to_owned()
        };
        let trigger = if trigger.is_empty() {
            trigger_label::PRESSURE
        } else {
            trigger
        };
        let action = if action.is_empty() { "summary" } else { action };
        let status = if status != "failed" { "blocked" } else { status };
        let visible = self.model_visible_messages();
        let input_tokens = estimated_visible_request_tokens(self, &visible);
        let (_, transcript_version) = self.session().snapshot_messages_version();
        let prompt_cache_key = self.prompt_cache_key();
        let boundary = self.compact_boundary().to_owned();
        let trigger_tokens = self.compact_trigger();
        let mut emitted: Option<ContextMaintenanceReceipt> = None;
        self.with_core(|core| {
            let state = &mut core.compaction_state;
            if let Some(last) = &state.last_receipt
                && (last.status == "blocked" || last.status == "failed")
                && last.action == action
                && !noop::retries_on_growth(code)
            {
                return;
            }
            let previous = state.clone();
            state.schema_version = COMPACTION_STATE_SCHEMA_CURRENT;
            state.transcript_version = transcript_version;
            state.prompt_cache_key = prompt_cache_key.clone();
            // Do not advance projection version on failure; generation still
            // advances so CAS losers and concurrent writers cannot overwrite
            // a newer success.
            state.generation += 1;
            // LastReceipt carries the blocked signal; clear legacy top-level
            // mirrors.
            state.blocked_input_hash = String::new();
            state.blocked_reason = String::new();
            state.last_trigger = String::new();
            state.last_mode = String::new();
            state.last_source_tokens = 0;
            state.last_result_tokens = 0;
            let receipt = ContextMaintenanceReceipt {
                operation_id: format!("{status}-{action}-{}", state.generation),
                status: status.to_owned(),
                action: action.to_owned(),
                trigger: trigger.to_owned(),
                source_projection: state.projection.projection_version,
                projection_version: state.projection.projection_version,
                input_hash: input_hash.clone(),
                input_tokens,
                blocked_input_hash: input_hash.clone(),
                reason: reason.to_owned(),
                code: code.to_owned(),
                boundary: boundary.clone(),
                trigger_tokens,
                ..ContextMaintenanceReceipt::default()
            };
            state.last_receipt = Some(receipt.clone());
            state.updated_at = now_created_at();
            if self.persist_compaction_state(state).is_err() {
                *state = previous;
                return;
            }
            emitted = Some(receipt);
        });
        if let Some(receipt) = emitted {
            self.emit_context_maintenance(&receipt);
        }
    }

    /// Publishes the sidecar under the session's write authority (Go:
    /// `persistCompactionStateLocked`): a controller whose transcript save
    /// would be refused must not install a fold either. No path bound means
    /// nothing to publish — the receipts stay the in-memory generation's.
    fn persist_compaction_state(&self, state: &reasonix_state::sessionstore::compaction_state::CompactionState) -> Result<(), reasonix_state::sessionstore::compaction_state::SaveSessionCompactionStateError> {
        let path = self.session_path();
        if path.is_empty() {
            return Ok(());
        }
        self.session().save_compaction_state(&path, state.clone())
    }

    /// Emits one maintenance record (Go: `emitContextMaintenance`).
    pub(crate) fn emit_context_maintenance(&self, receipt: &ContextMaintenanceReceipt) {
        self.sink().emit(Event {
            kind: crate::sink::kind::CONTEXT_MAINTENANCE.to_owned(),
            maintenance: Some(reasonix_contract::eventwire::event::ContextMaintenance {
                status: receipt.status.clone(),
                action: receipt.action.clone(),
                trigger: receipt.trigger.clone(),
                operation_id: receipt.operation_id.clone(),
                input_tokens: receipt.input_tokens,
                result_tokens: receipt.result_tokens,
                saved_tokens: receipt.saved_tokens,
                affected_tool_results: receipt.affected_tool_results,
                projection_version: receipt.projection_version,
                cache_break: receipt.cache_break,
                reason: receipt.reason.clone(),
                code: receipt.code.clone(),
                boundary: receipt.boundary.clone(),
                trigger_tokens: receipt.trigger_tokens,
            }),
            ..Event::default()
        });
    }

    /// Reports one attempt that folded nothing (Go: `noteMaintenanceNoop`).
    /// It writes no receipt and advances no generation: nothing was spent
    /// and nothing is blocked, so the next round is free to try again —
    /// which is exactly why the attempt would otherwise leave no trace.
    pub fn note_maintenance_noop(
        &self,
        trigger: &str,
        reason: CompactionNoopReason,
        input_tokens: i64,
    ) {
        if reason.is_empty() {
            return;
        }
        let turn = self.active_turn_created_at();
        let reported = self.with_core(|core| {
            if core.last_reported.reason == reason && core.last_reported.turn == turn {
                return false;
            }
            core.last_reported = MaintenanceReport {
                reason: reason.to_owned(),
                turn,
            };
            true
        });
        if !reported {
            return;
        }
        self.sink().emit(Event {
            kind: crate::sink::kind::CONTEXT_MAINTENANCE.to_owned(),
            maintenance: Some(reasonix_contract::eventwire::event::ContextMaintenance {
                status: "noop".to_owned(),
                action: "summary".to_owned(),
                trigger: trigger.to_owned(),
                code: reason.to_owned(),
                boundary: self.compact_boundary().to_owned(),
                trigger_tokens: self.compact_trigger(),
                input_tokens,
                ..reasonix_contract::eventwire::event::ContextMaintenance::default()
            }),
            ..Event::default()
        });
    }

    /// Tells the user that compaction is due but a failed attempt still
    /// holds the retry (Go: `noteMaintenanceHeld`). It speaks once per turn
    /// and cause; the model is not told, because the hold has no
    /// model-visible symptom of its own.
    pub fn note_maintenance_held(&self) {
        let code = self.with_core(|core| {
            core.compaction_state
                .last_receipt
                .as_ref()
                .map(|r| r.code.clone())
                .unwrap_or_default()
        });
        let reason: &str = if code.is_empty() {
            noop::UNCLASSIFIED
        } else {
            &code
        };
        let turn = self.active_turn_created_at();
        let reported = self.with_core(|core| {
            if core.last_reported.reason == reason && core.last_reported.turn == turn {
                return false;
            }
            core.last_reported = MaintenanceReport {
                reason: reason.to_owned(),
                turn,
            };
            true
        });
        if !reported {
            return;
        }
        self.sink().emit(Event {
            kind: crate::sink::kind::NOTICE.to_owned(),
            level: crate::sink::level::WARN.to_owned(),
            code: crate::sink::notice_code::COMPACT_HELD.to_owned(),
            detail: reason.to_owned(),
            text: format!(
                "Automatic compaction is paused: the last attempt did not finish ({reason})."
            ),
            ..Event::default()
        });
        self.note_context_composition();
    }

    /// Tells the user what an over-threshold prompt is made of (Go:
    /// `noteContextComposition`). The composition breakdown itself is the
    /// context-composition wave's surface; until it lands the notice says
    /// that maintenance held, which is the part this machine settled.
    pub(crate) fn note_context_composition(&self) {
        self.sink().emit(Event {
            kind: crate::sink::kind::NOTICE.to_owned(),
            level: crate::sink::level::WARN.to_owned(),
            code: crate::sink::notice_code::CONTEXT_COMPOSITION.to_owned(),
            ..Event::default()
        });
    }
}

/// Go: `compactionFailureCode` — the class of a fold error. A rejection
/// carries its own code; every other class is a sentinel its producer
/// wrapped.
pub fn failure_code(err: &FoldError) -> (&'static str, CompactionNoopReason) {
    match err {
        FoldError::CheckpointRejected { code, .. } => ("blocked", code),
        FoldError::StaleContext => ("failed", noop::CONTEXT_CHANGED),
        FoldError::HookRefused => ("failed", noop::HOOK_REFUSED),
        FoldError::PersistFailed => ("failed", noop::PERSIST_FAILED),
        FoldError::Cancelled => ("failed", noop::CANCELLED),
        FoldError::Failed(_) => ("failed", noop::UNCLASSIFIED),
    }
}

/// The wire shape the fingerprint reads (Go: `provider.ModelMessages` at
/// the hash boundary — the stored view minus host-only fields).
fn model_visible_wire(visible: &[Message]) -> Vec<Message> {
    reasonix_contract::provider::model_messages(visible).into_owned()
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// The receipt timestamp. Divergence from Go: the state crate stamps "now"
/// internally (`now_utc` is pub(crate)), so this port records the zero
/// time, which the wire omits. Receipt ordering answers to the generation
/// counter either way; the compaction wave owns the persisted path and
/// stamps it there.
fn now_created_at() -> reasonix_state::sessionstore::eventlog::CreatedAt {
    reasonix_state::sessionstore::eventlog::CreatedAt::default()
}
