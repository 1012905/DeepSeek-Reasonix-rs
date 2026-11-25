//! TOOL-038: non-persisted capability/routing counters (docs/contracts/tools.md
//! §3.2, Go `internal/runtime/capability/audit.go`).
//!
//! Counters back `run --metrics` and e2ebench. They are in-memory for the run;
//! cross-process audit landing lives in the host assembly. Gate recovery is
//! recorded separately from gate misses so a recovery never double-counts as
//! a miss.

#![forbid(unsafe_code)]

use std::sync::Mutex;

use super::capability_catalog::AutoUse;
use super::capability_router::RouteDecision;

/// The semantic router's own funnel and spend (Go `capability.RouterAudit`).
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct RouterAudit {
    pub semantic_routes: u64,
    pub semantic_fallbacks: u64,
    /// Entries the router never saw because the candidate cap bound.
    pub candidates_dropped: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost: f64,
    pub latency_ms: i64,
}

/// In-memory capability/routing counters sink (Go `capability.Audit`).
#[derive(Debug, Default)]
pub struct CapabilityAudit {
    inner: Mutex<AuditCounters>,
}

#[derive(Debug, Default, Clone, Copy)]
struct AuditCounters {
    routes: u64,
    routed_candidates: u64,
    routed_require: u64,
    routed_prefer: u64,
    routed_suggest: u64,
    declines: u64,
    require_missing: u64,
    require_recovered: u64,
    prefer_missing: u64,
    prefer_recovered: u64,
    skill_invocations: u64,
    skill_failures: u64,
    skill_unavailable: u64,
    mcp_inspect: u64,
    mcp_call: u64,
    mcp_call_failures: u64,
    review_blocks: u64,
    security_review_blocks: u64,
    router: RouterAudit,
}

impl CapabilityAudit {
    /// Builds an empty counter sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// Captures the route-to-invocation funnel before the model acts
    /// (Go `RecordDecision`).
    pub fn record_decision(&self, decision: &RouteDecision) {
        let mut counters = self.inner.lock().unwrap();
        for candidate in &decision.candidates {
            counters.routed_candidates += 1;
            match candidate.policy {
                AutoUse::Require => counters.routed_require += 1,
                AutoUse::Prefer => counters.routed_prefer += 1,
                AutoUse::Suggest => counters.routed_suggest += 1,
                _ => {}
            }
        }
    }

    /// Counts an explicit model decision not to use a preferred route.
    pub fn record_decline(&self) {
        self.inner.lock().unwrap().declines += 1;
    }

    /// Increments deterministic/hybrid route counts (Go `RecordRoute`).
    pub fn record_route(&self, semantic: bool, fallback: bool) {
        let mut counters = self.inner.lock().unwrap();
        counters.routes += 1;
        if semantic {
            counters.router.semantic_routes += 1;
        }
        if fallback {
            counters.router.semantic_fallbacks += 1;
        }
    }

    /// Records require/prefer missing and recovery (Go `RecordGate`).
    pub fn record_gate(&self, require_missing: bool, prefer_missing: bool, recovered: bool) {
        let mut counters = self.inner.lock().unwrap();
        if require_missing {
            counters.require_missing += 1;
        }
        if prefer_missing {
            counters.prefer_missing += 1;
        }
        if recovered {
            if require_missing {
                counters.require_recovered += 1;
            }
            if prefer_missing {
                counters.prefer_recovered += 1;
            }
        }
    }

    /// Records skill invocation outcomes (Go `RecordSkill`).
    pub fn record_skill(&self, failed: bool, unavailable: bool) {
        let mut counters = self.inner.lock().unwrap();
        counters.skill_invocations += 1;
        if failed {
            counters.skill_failures += 1;
        }
        if unavailable {
            counters.skill_unavailable += 1;
        }
    }

    /// Records use_capability proxy activity (Go `RecordMCPProxy`).
    pub fn record_mcp_proxy(&self, inspect: bool, call: bool, failed: bool) {
        let mut counters = self.inner.lock().unwrap();
        if inspect {
            counters.mcp_inspect += 1;
        }
        if call {
            counters.mcp_call += 1;
        }
        if failed {
            counters.mcp_call_failures += 1;
        }
    }

    /// Gate kinds that missed earlier in the turn later passed cleanly. Kept
    /// separate from [`Self::record_gate`] so a recovery never double-counts
    /// as a miss (Go `RecordGateRecovery`).
    pub fn record_gate_recovery(&self, require: bool, prefer: bool) {
        let mut counters = self.inner.lock().unwrap();
        if require {
            counters.require_recovered += 1;
        }
        if prefer {
            counters.prefer_recovered += 1;
        }
    }

    /// Accumulates the semantic router's own model spend (Go
    /// `RecordRouterUsage`).
    pub fn record_router_usage(
        &self,
        prompt_tokens: u64,
        completion_tokens: u64,
        cost: f64,
        latency_ms: i64,
    ) {
        let mut counters = self.inner.lock().unwrap();
        counters.router.prompt_tokens += prompt_tokens;
        counters.router.completion_tokens += completion_tokens;
        counters.router.cost += cost;
        counters.router.latency_ms += latency_ms;
    }

    /// Records candidates the cap kept from the router pool
    /// (Go `RecordSemanticPoolTruncation`).
    pub fn record_semantic_pool_truncation(&self, dropped: u64) {
        if dropped == 0 {
            return;
        }
        self.inner.lock().unwrap().router.candidates_dropped += dropped;
    }

    /// Records blocking structured review outcomes (Go `RecordReviewBlock`).
    pub fn record_review_block(&self, security: bool) {
        let mut counters = self.inner.lock().unwrap();
        if security {
            counters.security_review_blocks += 1;
        } else {
            counters.review_blocks += 1;
        }
    }

    /// Copies counters for metrics export (Go `Snapshot`).
    pub fn snapshot(&self) -> AuditSnapshot {
        let c = self.inner.lock().unwrap();
        AuditSnapshot {
            routes: c.routes,
            routed_candidates: c.routed_candidates,
            routed_require: c.routed_require,
            routed_prefer: c.routed_prefer,
            routed_suggest: c.routed_suggest,
            declines: c.declines,
            require_missing: c.require_missing,
            require_recovered: c.require_recovered,
            prefer_missing: c.prefer_missing,
            prefer_recovered: c.prefer_recovered,
            skill_invocations: c.skill_invocations,
            skill_failures: c.skill_failures,
            skill_unavailable: c.skill_unavailable,
            mcp_inspect: c.mcp_inspect,
            mcp_call: c.mcp_call,
            mcp_call_failures: c.mcp_call_failures,
            review_blocks: c.review_blocks,
            security_review_blocks: c.security_review_blocks,
            router: c.router,
        }
    }
}

/// Exportable counter snapshot (Go `capability.Audit` after `Snapshot`).
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct AuditSnapshot {
    pub routes: u64,
    pub routed_candidates: u64,
    pub routed_require: u64,
    pub routed_prefer: u64,
    pub routed_suggest: u64,
    pub declines: u64,
    pub require_missing: u64,
    pub require_recovered: u64,
    pub prefer_missing: u64,
    pub prefer_recovered: u64,
    pub skill_invocations: u64,
    pub skill_failures: u64,
    pub skill_unavailable: u64,
    pub mcp_inspect: u64,
    pub mcp_call: u64,
    pub mcp_call_failures: u64,
    pub review_blocks: u64,
    pub security_review_blocks: u64,
    pub router: RouterAudit,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::capability_catalog::CapabilityEntry;
    use crate::builtin::capability_router::{RouteCandidate, RouteDecision};

    #[test]
    fn decision_funnel_counts_by_policy() {
        let audit = CapabilityAudit::new();
        audit.record_decision(&RouteDecision {
            candidates: vec![
                RouteCandidate {
                    entry: CapabilityEntry::default(),
                    policy: AutoUse::Require,
                    reason: String::new(),
                },
                RouteCandidate {
                    entry: CapabilityEntry::default(),
                    policy: AutoUse::Prefer,
                    reason: String::new(),
                },
                RouteCandidate {
                    entry: CapabilityEntry::default(),
                    policy: AutoUse::Suggest,
                    reason: String::new(),
                },
            ],
            ..Default::default()
        });
        let snap = audit.snapshot();
        assert_eq!(snap.routed_candidates, 3);
        assert_eq!(snap.routed_require, 1);
        assert_eq!(snap.routed_prefer, 1);
        assert_eq!(snap.routed_suggest, 1);
    }

    #[test]
    fn recovery_kept_separate_from_misses() {
        let audit = CapabilityAudit::new();
        audit.record_gate(true, false, false);
        audit.record_gate_recovery(true, false);
        let snap = audit.snapshot();
        assert_eq!(snap.require_missing, 1);
        assert_eq!(snap.require_recovered, 1);
    }

    #[test]
    fn review_blocks_split_by_security() {
        let audit = CapabilityAudit::new();
        audit.record_review_block(false);
        audit.record_review_block(true);
        let snap = audit.snapshot();
        assert_eq!(snap.review_blocks, 1);
        assert_eq!(snap.security_review_blocks, 1);
    }

    #[test]
    fn router_spend_accumulates() {
        let audit = CapabilityAudit::new();
        audit.record_router_usage(10, 4, 0.001, 12);
        audit.record_router_usage(5, 2, 0.0005, 8);
        let snap = audit.snapshot();
        assert_eq!(snap.router.prompt_tokens, 15);
        assert_eq!(snap.router.completion_tokens, 6);
        assert_eq!(snap.router.latency_ms, 20);
    }
}
