//! ACP-028: `session/cancel` silent notification rules. Malformed bodies,
//! empty ids, and unknown sessions are dropped without any error response
//! (`session.go:713-725`).

use reasonix_acp::session_cancel::{CancelRegistry, handle_cancel};
use reasonix_acp::wire::SessionCancelParams;
use serde_json::Value;

struct RecordingRegistry {
    known: Vec<String>,
    aborted: Vec<String>,
}

impl CancelRegistry for RecordingRegistry {
    fn abort(&mut self, id: &str) -> bool {
        if !self.known.iter().any(|k| k == id) {
            return false;
        }
        self.aborted.push(id.to_owned());
        true
    }
}

fn params(params: &SessionCancelParams) -> Value {
    serde_json::to_value(params).unwrap()
}

#[test]
fn missing_body_is_silently_ignored() {
    let mut registry = RecordingRegistry {
        known: vec!["s1".to_owned()],
        aborted: vec![],
    };
    handle_cancel(None, &mut registry);
    assert!(registry.aborted.is_empty());
}

#[test]
fn malformed_body_is_silently_ignored() {
    let mut registry = RecordingRegistry {
        known: vec!["s1".to_owned()],
        aborted: vec![],
    };
    handle_cancel(Some(&Value::String("nope".to_owned())), &mut registry);
    handle_cancel(Some(&Value::Null), &mut registry);
    assert!(registry.aborted.is_empty());
}

#[test]
fn empty_and_whitespace_ids_are_ignored() {
    let mut registry = RecordingRegistry {
        known: vec!["s1".to_owned()],
        aborted: vec![],
    };
    handle_cancel(
        Some(&params(&SessionCancelParams {
            session_id: String::new(),
        })),
        &mut registry,
    );
    handle_cancel(
        Some(&params(&SessionCancelParams {
            session_id: "   ".to_owned(),
        })),
        &mut registry,
    );
    assert!(registry.aborted.is_empty());
}

#[test]
fn unknown_session_is_aborted_nowhere_and_stays_silent() {
    let mut registry = RecordingRegistry {
        known: vec!["s1".to_owned()],
        aborted: vec![],
    };
    handle_cancel(
        Some(&params(&SessionCancelParams {
            session_id: "ghost".to_owned(),
        })),
        &mut registry,
    );
    assert!(registry.aborted.is_empty());
}

#[test]
fn known_session_is_aborted_once_without_reply() {
    let mut registry = RecordingRegistry {
        known: vec!["s1".to_owned(), "s2".to_owned()],
        aborted: vec![],
    };
    handle_cancel(
        Some(&params(&SessionCancelParams {
            session_id: "s1".to_owned(),
        })),
        &mut registry,
    );
    assert_eq!(registry.aborted, vec!["s1".to_owned()]);
}
