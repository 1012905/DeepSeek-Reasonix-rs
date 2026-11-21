//! Approval oracles ported from Go `internal/session/control/approval*_test.go`
//! and `internal/assembly/boot/effect_ask_timeout_test.go` at the Gate/Asker
//! seam: the verdict matrix, timeout-as-blocked, ask timeout, and decision
//! receipt identity. The controller-level gated → asked → answered turn is
//! in `crates/session/tests/approval.rs`.

mod support;

use std::sync::Arc;
use std::time::Duration;

use reasonix_agent::approval::{
    ApprovalBroker, ApprovalConfig, AskAnswer, BrokerAsker, DecisionInput, InteractiveGate, mode,
    policy_for_mode,
};
use reasonix_agent::seams::Asker;
use reasonix_contract::eventwire::ask::AskQuestion;
use reasonix_safety::permission::{
    Policy, REFUSAL_DENY_RULE, REFUSAL_READ_ONLY, REFUSAL_UNATTENDED,
};

fn raw(s: &str) -> serde_json::Value {
    serde_json::from_str(s).expect("args parse")
}

#[test]
fn verdict_matrix_modes_and_read_only_posture() {
    let base = Policy::new("ask", &["read_file"], &["bash(rm:*)"], &["bash(git push*)"]);
    let ask = InteractiveGate::new(
        &ApprovalBroker::new(
            ApprovalConfig {
                mode: mode::ASK,
                timeout: Duration::ZERO,
            },
            Arc::new(reasonix_state::sessionstore::Session::new("")),
            Arc::new(reasonix_agent::sink::Discard),
        ),
        &base,
        &ApprovalConfig {
            mode: mode::ASK,
            timeout: Duration::ZERO,
        },
    );
    // A denied command keeps its own code in every mode.
    let v = ask.verdict("bash", &raw(r#"{"command":"git push origin"}"#), false);
    assert!(!v.allow && v.code == REFUSAL_DENY_RULE, "{v:?}");
    let v = ask.verdict("read_file", &raw(r#"{"path":"a"}"#), true);
    assert!(v.allow, "a declared reader runs: {v:?}");

    // dontAsk and readOnly block without opening a prompt: no approver, so a
    // would-ask call fails closed instead of inventing an allow.
    let dont = InteractiveGate::new(
        &ApprovalBroker::new(
            ApprovalConfig {
                mode: mode::DONT_ASK,
                timeout: Duration::ZERO,
            },
            Arc::new(reasonix_state::sessionstore::Session::new("")),
            Arc::new(reasonix_agent::sink::Discard),
        ),
        &base,
        &ApprovalConfig {
            mode: mode::DONT_ASK,
            timeout: Duration::ZERO,
        },
    );
    let v = dont.verdict("bash", &raw(r#"{"command":"go build"}"#), false);
    assert!(!v.allow, "dontAsk refuses a would-ask writer: {v:?}");

    let ro = InteractiveGate::new(
        &ApprovalBroker::new(
            ApprovalConfig {
                mode: mode::READ_ONLY,
                timeout: Duration::ZERO,
            },
            Arc::new(reasonix_state::sessionstore::Session::new("")),
            Arc::new(reasonix_agent::sink::Discard),
        ),
        &base,
        &ApprovalConfig {
            mode: mode::READ_ONLY,
            timeout: Duration::ZERO,
        },
    );
    let v = ro.verdict("bash", &raw(r#"{"command":"go build"}"#), false);
    assert!(!v.allow && v.code == REFUSAL_READ_ONLY, "readOnly: {v:?}");

    // Mode shapes only the fallback; explicit rules survive.
    let auto = policy_for_mode(&base, mode::AUTO);
    assert_eq!(auto.mode, reasonix_safety::permission::Decision::Allow);
    assert_eq!(auto.ask.len(), 1);
    assert_eq!(auto.deny.len(), 1);
}

#[test]
fn approval_timeout_blocks_without_a_fake_auto_allow() {
    // Go `TestApprovalTimeoutDeniesWhenUnanswered`: a positive timeout turns
    // an unanswered prompt into a refusal near the deadline, not an allow.
    let broker = ApprovalBroker::new(
        ApprovalConfig {
            mode: mode::ASK,
            timeout: Duration::from_millis(40),
        },
        Arc::new(reasonix_state::sessionstore::Session::new("")),
        Arc::new(reasonix_agent::sink::Discard),
    );
    let gate = InteractiveGate::new(
        &broker,
        &Policy::new("ask", &[], &[], &[]),
        &ApprovalConfig {
            mode: mode::ASK,
            timeout: Duration::from_millis(40),
        },
    );
    let start = std::time::Instant::now();
    let v = gate.verdict("write_file", &raw(r#"{"path":"a"}"#), false);
    assert!(!v.allow, "unanswered prompt must not run: {v:?}");
    assert_eq!(v.code, REFUSAL_UNATTENDED, "timeout is a host refusal");
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "must return near timeout"
    );
}

#[test]
fn ask_timeout_reports_timed_out_identity_and_a_receipt() {
    // Go `TestEffectUnansweredAskIsSkippedAsATimeoutThroughRealBuild`: an
    // unanswered question is a typed timeout with a receipt, never an empty
    // answer a model can read as a person's choice.
    let session = Arc::new(reasonix_state::sessionstore::Session::new(""));
    let rec = support::Recorder::shared();
    let broker = ApprovalBroker::new(
        ApprovalConfig {
            mode: mode::ASK,
            timeout: Duration::from_millis(30),
        },
        session.clone(),
        rec.clone(),
    );
    let q = vec![AskQuestion {
        id: "q1".to_owned(),
        prompt: "pick one".to_owned(),
        ..AskQuestion::default()
    }];
    let start = std::time::Instant::now();
    let result = broker.ask_questions(&q);
    assert!(result.timed_out, "a skip is a timeout identity: {result:?}");
    assert!(result.answers.is_empty());
    assert!(start.elapsed() < Duration::from_secs(2), "near timeout");
    let asks = rec.events_of("ask_request");
    assert_eq!(asks.len(), 1, "the question reached the frontend");
    assert_eq!(asks[0].ask.as_ref().unwrap().id, result.id);
    let receipts = rec.events_of("notice");
    assert!(
        receipts.iter().any(|e| {
            e.code == "decision_receipt"
                && e.decision_receipt.as_ref().is_some_and(|r| {
                    r.id == result.id && r.kind == "ask" && r.outcome == "timed_out"
                })
        }),
        "the frontend heard a timed-out receipt: {receipts:?}"
    );
    let msgs = session.snapshot();
    assert!(
        msgs.iter().any(|m| {
            m.decision_receipts.iter().any(|r| {
                r.as_ref()
                    .is_some_and(|r| r.id == result.id && r.outcome == "timed_out")
            }) || m
                .decision_receipt
                .as_ref()
                .is_some_and(|r| r.id == result.id && r.outcome == "timed_out")
        }),
        "the transcript carries the same receipt identity: {msgs:?}"
    );
}

#[test]
fn answered_ask_carries_selections_and_a_stable_id() {
    let session = Arc::new(reasonix_state::sessionstore::Session::new(""));
    let rec = support::Recorder::shared();
    let broker = ApprovalBroker::new(ApprovalConfig::default(), session, rec.clone());
    let q = vec![AskQuestion {
        id: "q1".to_owned(),
        prompt: "pick".to_owned(),
        ..AskQuestion::default()
    }];
    let asker = BrokerAsker(broker.clone());
    let handle = std::thread::spawn(move || asker.ask(&q));
    // Wait for the ask_request, then answer by that id (Go: AnswerQuestion).
    let id = loop {
        let asks = rec.events_of("ask_request");
        if let Some(a) = asks.first().and_then(|e| e.ask.as_ref()) {
            break a.id.clone();
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    assert!(broker.answer(
        &id,
        vec![AskAnswer {
            id: "q1".to_owned(),
            selected: vec!["B".to_owned()]
        }]
    ));
    let answers = handle.join().expect("ask thread");
    assert_eq!(answers.answers.len(), 1);
    assert_eq!(answers.answers[0].question_id, "q1");
    assert_eq!(answers.answers[0].selected, vec!["B"]);
    assert!(answers.timed_out.is_none(), "a person answered");
    let receipts = rec.events_of("notice");
    assert!(
        receipts.iter().any(|e| {
            e.decision_receipt
                .as_ref()
                .is_some_and(|r| r.id == id && r.kind == "ask" && r.outcome == "answered")
        }),
        "answered receipt: {receipts:?}"
    );
    assert!(!broker.answer(&id, vec![]), "a duplicate answer is ignored");
}

#[test]
fn tool_decision_receipts_identity_is_allow_once_or_deny() {
    let session = Arc::new(reasonix_state::sessionstore::Session::new(""));
    let rec = support::Recorder::shared();
    let broker = ApprovalBroker::new(ApprovalConfig::default(), session, rec.clone());
    let gate = InteractiveGate::new(
        &broker,
        &Policy::new("ask", &[], &[], &[]),
        &ApprovalConfig::default(),
    );
    let args = raw(r#"{"path":"a"}"#);
    let handle = std::thread::spawn(move || gate.verdict("write_file", &args, false));
    let id = loop {
        let reqs = rec.events_of("approval_request");
        if let Some(a) = reqs.first().and_then(|e| e.approval.as_ref()) {
            break a.id.clone();
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    assert!(broker.decide(
        &id,
        DecisionInput {
            allow: true,
            session: false,
            persist: false
        }
    ));
    let v = handle.join().expect("gate thread");
    assert!(v.allow, "the human allowed it: {v:?}");
    let receipts = rec.events_of("notice");
    assert!(
        receipts.iter().any(|e| {
            e.decision_receipt
                .as_ref()
                .is_some_and(|r| r.id == id && r.kind == "tool" && r.outcome == "allow_once")
        }),
        "allow_once receipt: {receipts:?}"
    );
}
