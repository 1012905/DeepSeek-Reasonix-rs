//! The provider-bound context snapshot (Go: `context_status.go`).
//!
//! A read-only view separating present composition from cumulative
//! summary-call cost, for status gauges and `/context`-style surfaces.

use reasonix_state::sessionstore::compaction_state::ContextMaintenanceReceipt;

use super::{
    Window, estimated_visible_request_tokens, is_compaction_summary, model_visible_from_projection,
    projection_valid,
};
use super::estimate;

/// Read-only view of the current provider-bound context (Go:
/// `ContextMaintenanceSnapshot`).
#[derive(Debug, Clone, Default)]
pub struct ContextMaintenanceSnapshot {
    pub canonical_tokens: i64,
    pub projected_tokens: i64,
    pub summary_tokens: i64,
    pub last_saved_tokens: i64,
    pub snip_trigger: i64,
    pub fold_trigger: i64,
    pub force_trigger: i64,
    pub trigger_tokens: i64,
    pub checkpoint_state: String,
    pub hard_input_ceiling: i64,
    pub headroom: i64,
    pub projection_version: u64,
    pub blocked: bool,
    pub last_receipt: Option<ContextMaintenanceReceipt>,
}

impl Window {
    pub fn context_maintenance_snapshot(&self) -> ContextMaintenanceSnapshot {
        let canonical = self.session().snapshot();
        if canonical.is_empty() {
            // Go answers the zero snapshot without a conversation; an empty
            // transcript has nothing to measure either way.
        }
        let (compaction_state, checkpoint_state) = self.with_core(|core| {
            (
                core.compaction_state.clone(),
                core.checkpoint_state.clone(),
            )
        });
        let mut visible = canonical.clone();
        let key = self.prompt_cache_key();
        let valid = projection_valid(
            &compaction_state,
            &canonical,
            &key,
            None,
        );
        if valid {
            let projected =
                model_visible_from_projection(&compaction_state.projection, &canonical);
            if !projected.is_empty() {
                visible = projected;
            }
        }
        let trigger = self.compact_trigger();
        // UI checkpoint label requires a still-valid covered prefix, not
        // merely that the sidecar loaded.
        let ui_checkpoint = if valid && !compaction_state.projection.messages.is_empty() {
            state_checkpoint_state(&checkpoint_state, &compaction_state)
        } else {
            "none".to_owned()
        };
        let mut snapshot = ContextMaintenanceSnapshot {
            canonical_tokens: estimated_visible_request_tokens(self, &canonical),
            projected_tokens: estimated_visible_request_tokens(self, &visible),
            fold_trigger: trigger,
            trigger_tokens: trigger,
            checkpoint_state: ui_checkpoint,
            hard_input_ceiling: self.hard_input_ceiling(),
            projection_version: compaction_state.projection.projection_version,
            ..ContextMaintenanceSnapshot::default()
        };
        for msg in &visible {
            if is_compaction_summary(msg) {
                snapshot.summary_tokens +=
                    estimate::estimated_prompt_tokens(self, std::slice::from_ref(msg));
            }
        }
        snapshot.headroom = (snapshot.hard_input_ceiling - snapshot.projected_tokens).max(0);
        let current_hash = self.context_maintenance_input_hash(&visible);
        if let Some(receipt) = &compaction_state.last_receipt {
            snapshot.last_receipt = Some(receipt.clone());
            if receipt.status == "applied"
                && (receipt.action == "prune" || receipt.action == "summary")
            {
                snapshot.last_saved_tokens = receipt.saved_tokens;
            }
            // Generation-scoped blocked/failed receipts match
            // contextMaintenanceBlocked.
            if self.blocked_receipt_holds(receipt, snapshot.projected_tokens) {
                snapshot.blocked = true;
            }
        }
        // Legacy sidecars may only have top-level BlockedInputHash; a
        // receipt, when present, is the sole authority, as in
        // contextMaintenanceBlocked.
        if compaction_state.last_receipt.is_none()
            && !compaction_state.blocked_input_hash.is_empty()
            && compaction_state.blocked_input_hash == current_hash
        {
            snapshot.blocked = true;
        }
        snapshot
    }
}

/// Go: `stateCheckpointState` — the runtime label the UI shows for the
/// installed checkpoint.
fn state_checkpoint_state(
    runtime_state: &str,
    state: &reasonix_state::sessionstore::compaction_state::CompactionState,
) -> String {
    if state.projection.messages.is_empty() {
        return "none".to_owned();
    }
    if runtime_state == "applied" {
        return "applied".to_owned();
    }
    "restored".to_owned()
}
