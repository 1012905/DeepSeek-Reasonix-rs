//! Kernel judgments on the wire payloads must be shared, not re-derived by
//! each frontend (ledger SSE-016, SSE-020): one verdict means one answer.

use reasonix_contract::eventwire::event::CompletionSummary;
use reasonix_contract::eventwire::receipt::{CompletionReceipt, ReceiptGap};
use reasonix_contract::eventwire::vocab::{
    is_assistant_work, is_model_work, receipt_gap_severity, receipt_severity,
};

#[test]
fn completion_summary_attention_and_blocked_match_go() {
    let blocked = CompletionSummary {
        verdict: " Blocked ".into(),
        ..CompletionSummary::default()
    };
    assert!(blocked.blocked());
    assert!(blocked.needs_attention());

    let partial = CompletionSummary {
        verdict: "partial".into(),
        ..CompletionSummary::default()
    };
    assert!(!partial.blocked());
    assert!(partial.needs_attention());

    let warned = CompletionSummary {
        review: "WARNED".into(),
        ..CompletionSummary::default()
    };
    assert!(warned.needs_attention());

    let failed_checks = CompletionSummary {
        checks_failed: 1,
        ..CompletionSummary::default()
    };
    assert!(failed_checks.needs_attention());

    let rewritten = CompletionSummary {
        criteria_rewritten: vec!["test.go".into()],
        ..CompletionSummary::default()
    };
    assert!(rewritten.needs_attention());

    let clean = CompletionSummary {
        verdict: "complete".into(),
        review: "passed".into(),
        ..CompletionSummary::default()
    };
    assert!(!clean.needs_attention());
    assert!(!clean.blocked());
}

#[test]
fn receipt_says_something_and_severity_match_go() {
    let empty = CompletionReceipt::default();
    assert!(!empty.says_something());
    assert_eq!(empty.severity(), receipt_severity::NONE);

    let done = CompletionReceipt {
        verdict: "done".into(),
        ..CompletionReceipt::default()
    };
    assert!(done.says_something());
    assert_eq!(done.severity(), receipt_severity::NONE);

    let risks = CompletionReceipt {
        risks: vec!["r".into()],
        ..CompletionReceipt::default()
    };
    assert_eq!(risks.severity(), receipt_severity::NOTICE);

    let known_gap = CompletionReceipt {
        gaps: vec![ReceiptGap {
            kind: "stale_verification".into(),
            ..ReceiptGap::default()
        }],
        ..CompletionReceipt::default()
    };
    assert_eq!(known_gap.severity(), receipt_severity::NOTICE);

    let unknown_gap = CompletionReceipt {
        gaps: vec![ReceiptGap {
            kind: "future_kind".into(),
            ..ReceiptGap::default()
        }],
        ..CompletionReceipt::default()
    };
    assert_eq!(unknown_gap.severity(), receipt_severity::WARN);
    assert_eq!(
        receipt_gap_severity("outside_observed_root"),
        receipt_severity::NOTICE
    );
}

#[test]
fn issuer_work_predicates_match_go() {
    use reasonix_contract::eventwire::vocab::issuer;
    assert!(is_model_work(issuer::MODEL));
    assert!(!is_model_work(issuer::PROVIDER));
    assert!(is_assistant_work(issuer::MODEL));
    assert!(is_assistant_work(issuer::PROVIDER));
    assert!(!is_assistant_work(issuer::HOST));
    assert!(!is_assistant_work(issuer::USER));
}
