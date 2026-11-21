//! The completion receipt, mirroring `eventwire/receipt.go`: what the host
//! verified about a turn's work, and what it could not.

use crate::eventwire::util;
use serde::{Deserialize, Serialize};

/// `risks` and `unverified` are the turn's declarations, apart from the gaps
/// the host found: a reader that folds them together reads a volunteered
/// caveat as a failure.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletionReceipt {
    #[serde(default)]
    pub verdict: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<ReceiptChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verifications: Vec<ReceiptVerification>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<ReceiptGap>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risks: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unverified: Vec<String>,
    #[serde(rename = "saysSomething", default)]
    pub says_something: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub severity: String,
}

impl CompletionReceipt {
    /// Whether this receipt has anything a person needs.
    pub fn says_something(&self) -> bool {
        !self.gaps.is_empty()
            || !self.risks.is_empty()
            || !self.unverified.is_empty()
            || self.verdict == "done"
    }

    /// How hard this receipt asks to be read. An unfamiliar gap kind is warn.
    pub fn severity(&self) -> &'static str {
        use crate::eventwire::vocab::{receipt_gap_severity, receipt_severity};
        let mut out = receipt_severity::NONE;
        if !self.risks.is_empty() || !self.unverified.is_empty() {
            out = receipt_severity::NOTICE;
        }
        for gap in &self.gaps {
            if receipt_gap_severity(&gap.kind) == receipt_severity::WARN {
                return receipt_severity::WARN;
            }
            out = receipt_severity::NOTICE;
        }
        out
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptChange {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub reviewed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptVerification {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub passed: bool,
    #[serde(default, skip_serializing_if = "util::is_false")]
    pub stale: bool,
    #[serde(default, skip_serializing_if = "util::is_false")]
    pub inconclusive: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptGap {
    #[serde(default)]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}
