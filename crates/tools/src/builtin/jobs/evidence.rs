//! TOOL-046/048 mutation evidence lease, commit and recovery mapping
//! (Go `internal/tools/jobs.go` §490-547, §1696-1796).
//!
//! Done here: the minimal receipt/summary shape this slice reads, the
//! recovered-task receipt shape, the meta-version / risk-level recovery
//! decision table, and the lease/pending/commit gate decisions. Persisting
//! receipts from the live runner, the evidence classifier and the meta
//! rewrite after commit are host WIP; the full evidence schema lives in the
//! safety evidence package and is mirrored there.

#![forbid(unsafe_code)]

use super::artifacts::RECOVERED_BACKGROUND_TOOL_NAME;

/// `MutationProven` stamped on a recovered synthetic receipt.
pub const MUTATION_PROVEN: &str = "proven";

/// Mutation risk levels read back from persisted meta. The classifier that
/// assigns these lives in the safety evidence package; this slice only keeps
/// or drops paths per level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationRisk {
    Low,
    Medium,
    High,
    Unknown,
}

impl MutationRisk {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Unknown => "unknown",
        }
    }
}

/// One recorded mutation receipt, the subset the jobs package accesses.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EvidenceReceipt {
    pub tool_name: String,
    pub success: bool,
    pub write: bool,
    pub mutation: bool,
    pub paths: Vec<String>,
    pub mutation_evidence: String,
}

/// Collected evidence for one job (`evidence.ChildEvidenceSummary` trimmed to
/// the fields jobs reads).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChildEvidenceSummary {
    pub receipts: Vec<EvidenceReceipt>,
    pub paths: Vec<String>,
}

impl ChildEvidenceSummary {
    pub fn has_receipts(&self) -> bool {
        !self.receipts.is_empty()
    }
}

/// A receipt counts as a successful mutation when it succeeded and marked a
/// write/mutation (`mutationEvidenceForArtifact`).
pub fn is_success_mutation(receipt: &EvidenceReceipt) -> bool {
    receipt.success && receipt.write && receipt.mutation
}

/// `recoveredTaskMutationReceipt`: the synthetic receipt standing in for
/// receipts a recovered task job used to carry (`artifacts.go:104-113`).
pub fn recovered_task_mutation_receipt(paths: Vec<String>) -> EvidenceReceipt {
    EvidenceReceipt {
        tool_name: RECOVERED_BACKGROUND_TOOL_NAME.to_owned(),
        success: true,
        write: true,
        mutation: true,
        paths,
        mutation_evidence: MUTATION_PROVEN.to_owned(),
    }
}

/// Outcome of restoring a persisted task job's evidence summary.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecoveredEvidence {
    pub summary: Option<ChildEvidenceSummary>,
    /// Version mismatch or an unrecognized risk level: treat as opaque
    /// high-risk, do not downgrade the safety review.
    pub opaque: bool,
}

/// `mutationEvidenceFromArtifact` decision (`jobs.go:507-543`):
///
/// * version mismatch → opaque high-risk, paths dropped;
/// * same version, no recorded evidence → empty summary;
/// * Low/Medium keep their paths; High drops paths but keeps the risk;
/// * unknown risk level → opaque.
pub fn recover_from_meta(
    version: Option<u64>,
    risk: &str,
    paths: Vec<String>,
) -> RecoveredEvidence {
    let Some(version) = version else {
        return RecoveredEvidence {
            summary: Some(ChildEvidenceSummary {
                receipts: vec![],
                paths: Vec::new(),
            }),
            opaque: false,
        };
    };
    if version != super::artifacts::MUTATION_EVIDENCE_VERSION {
        return RecoveredEvidence {
            summary: Some(ChildEvidenceSummary {
                receipts: vec![recovered_task_mutation_receipt(Vec::new())],
                paths: Vec::new(),
            }),
            opaque: true,
        };
    }
    if risk.is_empty() && paths.is_empty() {
        return RecoveredEvidence {
            summary: Some(ChildEvidenceSummary::default()),
            opaque: false,
        };
    }
    match MutationRisk::from_str(risk) {
        MutationRisk::Low | MutationRisk::Medium => RecoveredEvidence {
            summary: Some(ChildEvidenceSummary {
                receipts: vec![recovered_task_mutation_receipt(paths.clone())],
                paths,
            }),
            opaque: false,
        },
        MutationRisk::High => RecoveredEvidence {
            summary: Some(ChildEvidenceSummary {
                receipts: vec![recovered_task_mutation_receipt(Vec::new())],
                paths: Vec::new(),
            }),
            opaque: false,
        },
        MutationRisk::Unknown => RecoveredEvidence {
            summary: Some(ChildEvidenceSummary {
                receipts: vec![recovered_task_mutation_receipt(Vec::new())],
                paths: Vec::new(),
            }),
            opaque: true,
        },
    }
}

/// `TryLeaseEvidenceForSession` gate: no job, still running, or already
/// committed return no ready evidence; otherwise the receipts are handed out
/// without being consumed (`jobs.go:1711-1733`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseOutcome {
    pub summary: Option<ChildEvidenceSummary>,
    pub ready: bool,
    /// Already committed: the caller learns the evidence was drained even
    /// though nothing new is leased.
    pub already_committed: bool,
}

pub fn try_lease(
    has_job: bool,
    done_open: bool,
    committed: bool,
    evidence: Option<&ChildEvidenceSummary>,
) -> LeaseOutcome {
    if !has_job {
        return LeaseOutcome {
            summary: None,
            ready: false,
            already_committed: false,
        };
    }
    if done_open {
        return LeaseOutcome {
            summary: None,
            ready: false,
            already_committed: false,
        };
    }
    if committed {
        return LeaseOutcome {
            summary: None,
            ready: true,
            already_committed: true,
        };
    }
    LeaseOutcome {
        summary: evidence.cloned(),
        ready: true,
        already_committed: false,
    }
}

/// `PendingEvidenceJobIDsForSession` rule: terminal, not yet committed, and
/// carries receipts (`jobs.go:1742-1765`).
pub fn pending_evidence(terminal: bool, committed: bool, has_receipts: bool) -> bool {
    terminal && !committed && has_receipts
}

/// `CommitEvidenceForSession` gate: absent job, still running, or already
/// committed do nothing; committing clears the held evidence
/// (`jobs.go:1773-1796`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitOutcome {
    pub committed: bool,
    pub had_evidence: bool,
}

pub fn commit_decision(has_job: bool, done_open: bool, committed: bool) -> CommitOutcome {
    if !has_job || done_open || committed {
        return CommitOutcome {
            committed: false,
            had_evidence: false,
        };
    }
    CommitOutcome {
        committed: true,
        had_evidence: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_mismatch_recovers_as_opaque_without_paths() {
        let out = recover_from_meta(Some(0), "high", vec!["/secret".to_string()]);
        assert!(out.opaque);
        let summary = out.summary.expect("opaque summary present");
        assert!(summary.paths.is_empty());
    }

    #[test]
    fn same_version_without_evidence_is_an_empty_summary() {
        let out = recover_from_meta(Some(1), "", Vec::new());
        assert!(!out.opaque);
        assert!(out.summary.expect("summary").receipts.is_empty());
    }

    #[test]
    fn low_and_medium_keep_paths_high_drops_them() {
        let low = recover_from_meta(Some(1), "low", vec!["/a".to_string()]);
        assert_eq!(low.summary.as_ref().expect("low").paths, vec!["/a"]);
        let medium = recover_from_meta(Some(1), "medium", vec!["/b".to_string()]);
        assert_eq!(medium.summary.as_ref().expect("medium").paths, vec!["/b"]);
        let high = recover_from_meta(Some(1), "high", vec!["/c".to_string()]);
        assert!(high.summary.as_ref().expect("high").paths.is_empty());
    }

    #[test]
    fn unknown_risk_is_opaque() {
        let out = recover_from_meta(Some(1), "weird", vec!["/d".to_string()]);
        assert!(out.opaque);
    }

    #[test]
    fn lease_does_not_consume_and_gates_on_done_and_committed() {
        let evidence = ChildEvidenceSummary::default();
        assert!(!try_lease(false, false, false, Some(&evidence)).ready);
        assert!(!try_lease(true, true, false, Some(&evidence)).ready);
        let committed = try_lease(true, false, true, Some(&evidence));
        assert!(committed.ready);
        assert!(committed.already_committed);
        let ready = try_lease(true, false, false, Some(&evidence));
        assert!(ready.ready);
        assert!(!ready.already_committed);
        assert!(ready.summary.is_some());
    }

    #[test]
    fn pending_evidence_needs_terminal_uncommitted_receipts() {
        assert!(pending_evidence(true, false, true));
        assert!(!pending_evidence(false, false, true));
        assert!(!pending_evidence(true, true, true));
        assert!(!pending_evidence(true, false, false));
    }

    #[test]
    fn commit_is_a_one_way_gate() {
        assert!(!commit_decision(false, false, false).committed);
        assert!(!commit_decision(true, true, false).committed);
        assert!(!commit_decision(true, false, true).committed);
        assert!(commit_decision(true, false, false).committed);
    }

    #[test]
    fn recovered_receipt_carries_the_recovery_tool_name() {
        let receipt = recovered_task_mutation_receipt(vec!["/x".to_string()]);
        assert_eq!(receipt.tool_name, RECOVERED_BACKGROUND_TOOL_NAME);
        assert!(receipt.success && receipt.write && receipt.mutation);
        assert_eq!(receipt.mutation_evidence, MUTATION_PROVEN);
    }
}
