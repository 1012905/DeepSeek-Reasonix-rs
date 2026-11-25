//! TOOL-038: turn-scoped capability ledger (docs/contracts/tools.md §3.2,
//! Go `internal/runtime/capability/ledger.go`).
//!
//! The ledger records route candidates and host-proven outcomes for one turn,
//! then evaluates the require/prefer final-answer gate. Terminal outcomes stick;
//! only succeeded/unavailable may overwrite each other; invoked upgrades
//! pending. A declined preference needs a non-empty reason.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;

use thiserror::Error;

use super::capability_catalog::AutoUse;
use super::capability_router::RouteDecision;

/// What happened for a routed capability this turn (Go `capability.Outcome`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Outcome {
    #[default]
    Pending,
    Invoked,
    Succeeded,
    Failed,
    Unavailable,
    Declined,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Pending => "pending",
            Outcome::Invoked => "invoked",
            Outcome::Succeeded => "succeeded",
            Outcome::Failed => "failed",
            Outcome::Unavailable => "unavailable",
            Outcome::Declined => "declined",
        }
    }
}

/// One turn-scoped capability tracking record (Go `capability.LedgerEntry`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LedgerEntry {
    pub id: String,
    pub policy: AutoUse,
    pub reason: String,
    pub outcome: Outcome,
    pub failure_reason: String,
    pub declined_reason: String,
    pub invoked_at: Option<SystemTime>,
    /// Prefer: the host already issued one retry reminder.
    pub reminded: bool,
}

/// Ledger write failure (Go `errEmptyDecline`).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LedgerError {
    #[error("decline reason must be non-empty")]
    EmptyDecline,
}

/// Records capability route candidates and outcomes for one turn
/// (Go `capability.Ledger`).
#[derive(Debug, Default)]
pub struct CapabilityLedger {
    entries: Mutex<HashMap<String, LedgerEntry>>,
    order: Mutex<Vec<String>>,
}

impl CapabilityLedger {
    /// Builds an empty turn-scoped ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears the ledger between user turns.
    pub fn reset(&self) {
        *self.entries.lock().unwrap() = HashMap::new();
        *self.order.lock().unwrap() = Vec::new();
    }

    /// Records the route decision for this turn; repeats keep the strongest
    /// policy (Go `SeedCandidates`).
    pub fn seed_candidates(&self, decision: &RouteDecision) {
        let mut entries = self.entries.lock().unwrap();
        let mut order = self.order.lock().unwrap();
        for candidate in &decision.candidates {
            let id = candidate.entry.id.as_str();
            if id.is_empty() {
                continue;
            }
            match entries.get_mut(id) {
                Some(existing) => {
                    if candidate.policy.rank() > existing.policy.rank() {
                        existing.policy = candidate.policy;
                        existing.reason = candidate.reason.clone();
                    }
                }
                None => {
                    entries.insert(
                        id.to_owned(),
                        LedgerEntry {
                            id: id.to_owned(),
                            policy: candidate.policy,
                            reason: candidate.reason.clone(),
                            outcome: Outcome::Pending,
                            ..Default::default()
                        },
                    );
                    order.push(id.to_owned());
                }
            }
        }
    }

    /// Records that the agent called a capability.
    pub fn mark_invoked(&self, id: &str) {
        self.set_outcome(id, Outcome::Invoked, "", "");
    }

    /// Records a successful capability call.
    pub fn mark_succeeded(&self, id: &str) {
        self.set_outcome(id, Outcome::Succeeded, "", "");
    }

    /// Records a failed capability call with host-proven detail.
    pub fn mark_failed(&self, id: &str, reason: &str) {
        self.set_outcome(id, Outcome::Failed, reason, "");
    }

    /// Records a host-proven unavailable state.
    pub fn mark_unavailable(&self, id: &str, reason: &str) {
        self.set_outcome(id, Outcome::Unavailable, reason, "");
    }

    /// Records a prefer decline with a non-empty reason (Go `MarkDeclined`).
    pub fn mark_declined(&self, id: &str, reason: &str) -> Result<(), LedgerError> {
        if reason.trim().is_empty() {
            return Err(LedgerError::EmptyDecline);
        }
        self.set_outcome(id, Outcome::Declined, "", reason.trim());
        Ok(())
    }

    /// Records that prefer was missing once and the host reminded.
    pub fn mark_reminded(&self, id: &str) {
        if let Some(entry) = self.entries.lock().unwrap().get_mut(id) {
            entry.reminded = true;
        }
    }

    fn set_outcome(&self, id: &str, outcome: Outcome, fail_reason: &str, decline_reason: &str) {
        let id = id.trim();
        if id.is_empty() {
            return;
        }
        let mut entries = self.entries.lock().unwrap();
        let mut order = self.order.lock().unwrap();
        let entry = entries.entry(id.to_owned()).or_insert_with(|| LedgerEntry {
            id: id.to_owned(),
            policy: AutoUse::Suggest,
            outcome: Outcome::Pending,
            ..Default::default()
        });
        // Terminal outcomes stick; only succeeded/unavailable may overwrite.
        match entry.outcome {
            Outcome::Succeeded | Outcome::Unavailable | Outcome::Declined => {
                if matches!(outcome, Outcome::Succeeded | Outcome::Unavailable) {
                    entry.outcome = outcome;
                }
            }
            _ => entry.outcome = outcome,
        }
        if !fail_reason.is_empty() {
            entry.failure_reason = fail_reason.to_owned();
        }
        if !decline_reason.is_empty() {
            entry.declined_reason = decline_reason.to_owned();
        }
        if matches!(
            outcome,
            Outcome::Invoked | Outcome::Succeeded | Outcome::Failed
        ) {
            entry.invoked_at = Some(SystemTime::now());
        }
        if !order.iter().any(|o| o == id) {
            order.push(id.to_owned());
        }
    }

    /// Copies entries in seed order (Go `Snapshot`).
    pub fn snapshot(&self) -> Vec<LedgerEntry> {
        let entries = self.entries.lock().unwrap();
        self.order
            .lock()
            .unwrap()
            .iter()
            .filter_map(|id| entries.get(id).cloned())
            .collect()
    }

    /// Returns one entry by ID (Go `Get`).
    pub fn get(&self, id: &str) -> Option<LedgerEntry> {
        self.entries.lock().unwrap().get(id.trim()).cloned()
    }

    /// Evaluates require/prefer policy for the final answer (Go
    /// `CheckFinalGate`). An empty `reason` means the gate passes.
    pub fn check_final_gate(&self) -> GateFailure {
        let entries = self.entries.lock().unwrap();
        let order = self.order.lock().unwrap();
        let mut require_missing: Vec<String> = Vec::new();
        let mut prefer_missing: Vec<String> = Vec::new();
        let mut unavailable: Vec<String> = Vec::new();
        let mut prefer_remind: Vec<String> = Vec::new();
        for id in order.iter() {
            let Some(entry) = entries.get(id) else {
                continue;
            };
            match entry.policy {
                AutoUse::Require => match entry.outcome {
                    Outcome::Succeeded => {}
                    Outcome::Unavailable => {
                        unavailable.push(format!("{}: {}", entry.id, entry.failure_reason));
                    }
                    _ => require_missing.push(entry.id.clone()),
                },
                AutoUse::Prefer => match entry.outcome {
                    Outcome::Succeeded | Outcome::Declined | Outcome::Unavailable => {}
                    _ => {
                        if entry.reminded {
                            prefer_missing.push(entry.id.clone());
                        } else {
                            prefer_remind.push(entry.id.clone());
                        }
                    }
                },
                _ => {}
            }
        }

        if !require_missing.is_empty() {
            return GateFailure {
                reason: format!(
                    "required capabilities not successfully invoked: {}",
                    require_missing.join(", ")
                ),
                require_ids: require_missing,
                ..Default::default()
            };
        }
        if !unavailable.is_empty() {
            return GateFailure {
                reason: format!(
                    "required capabilities unavailable (host-proven): {}",
                    unavailable.join("; ")
                ),
                unavailable_ok: true,
                ..Default::default()
            };
        }
        if !prefer_remind.is_empty() {
            return GateFailure {
                reason: format!(
                    "preferred capabilities not yet used; call them or use_capability(action=\"decline\", reason=...): {}",
                    prefer_remind.join(", ")
                ),
                prefer_remind: true,
                prefer_ids: prefer_remind,
                ..Default::default()
            };
        }
        if !prefer_missing.is_empty() {
            return GateFailure {
                reason: format!(
                    "preferred capabilities still unused after reminder; call them or decline with a non-empty reason: {}",
                    prefer_missing.join(", ")
                ),
                prefer_ids: prefer_missing,
                ..Default::default()
            };
        }
        GateFailure::default()
    }
}

/// Non-empty reason when the final answer must be blocked for capability
/// policy (Go `capability.GateFailure`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GateFailure {
    pub reason: String,
    pub prefer_remind: bool,
    pub prefer_ids: Vec<String>,
    pub require_ids: Vec<String>,
    /// Require is host-unavailable: the agent may end with a blocker, not a
    /// success claim.
    pub unavailable_ok: bool,
}

impl GateFailure {
    /// Whether the gate blocks the final answer.
    pub fn blocked(&self) -> bool {
        !self.reason.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::capability_catalog::CapabilityEntry;
    use crate::builtin::capability_router::{RouteCandidate, RouteDecision};

    fn candidate(id: &str, policy: AutoUse) -> RouteCandidate {
        RouteCandidate {
            entry: CapabilityEntry {
                id: id.to_owned(),
                ..Default::default()
            },
            policy,
            reason: "test".to_owned(),
        }
    }

    #[test]
    fn decline_requires_a_reason() {
        let ledger = CapabilityLedger::new();
        assert!(ledger.mark_declined("skill:review", "  ").is_err());
        assert!(
            ledger
                .mark_declined("skill:review", "covered by read")
                .is_ok()
        );
    }

    #[test]
    fn terminal_outcomes_stick() {
        let ledger = CapabilityLedger::new();
        ledger.mark_succeeded("tool:read_file");
        ledger.mark_failed("tool:read_file", "boom");
        assert_eq!(
            ledger.get("tool:read_file").unwrap().outcome,
            Outcome::Succeeded
        );
        ledger.mark_unavailable("tool:read_file", "host down");
        assert_eq!(
            ledger.get("tool:read_file").unwrap().outcome,
            Outcome::Unavailable
        );
    }

    #[test]
    fn gate_reports_require_missing_first_then_unavailable_then_prefer() {
        let ledger = CapabilityLedger::new();
        ledger.seed_candidates(&RouteDecision {
            candidates: vec![
                candidate("skill:prefer-me", AutoUse::Prefer),
                candidate("tool:need-me", AutoUse::Require),
            ],
            ..Default::default()
        });
        let gate = ledger.check_final_gate();
        assert!(gate.blocked());
        assert!(
            gate.reason
                .starts_with("required capabilities not successfully invoked")
        );

        let ledger = CapabilityLedger::new();
        ledger.seed_candidates(&RouteDecision {
            candidates: vec![candidate("tool:need-me", AutoUse::Require)],
            ..Default::default()
        });
        ledger.mark_unavailable("tool:need-me", "host down");
        let gate = ledger.check_final_gate();
        assert!(gate.unavailable_ok);
        assert!(gate.reason.contains("host-proven"));

        let ledger = CapabilityLedger::new();
        ledger.seed_candidates(&RouteDecision {
            candidates: vec![candidate("skill:prefer-me", AutoUse::Prefer)],
            ..Default::default()
        });
        let gate = ledger.check_final_gate();
        assert!(gate.prefer_remind);
        ledger.mark_reminded("skill:prefer-me");
        let gate = ledger.check_final_gate();
        assert!(!gate.prefer_remind);
        assert!(gate.reason.contains("still unused after reminder"));
    }

    #[test]
    fn succeeded_prefer_and_require_pass_the_gate() {
        let ledger = CapabilityLedger::new();
        ledger.seed_candidates(&RouteDecision {
            candidates: vec![
                candidate("tool:need-me", AutoUse::Require),
                candidate("skill:prefer-me", AutoUse::Prefer),
            ],
            ..Default::default()
        });
        ledger.mark_succeeded("tool:need-me");
        ledger.mark_succeeded("skill:prefer-me");
        assert!(!ledger.check_final_gate().blocked());
    }
}
