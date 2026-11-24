//! ACP-029: `session/close` idempotent teardown, unknown-session no-op, and the
//! `closeAll` sweep (`session.go:572-586`; `service.go:1175-1185`).

use reasonix_acp::errors::RpcErrorCode;
use reasonix_acp::session_close::{CloseHandle, CloseRegistry, close_all, close_one, teardown};

struct RecordingHandle {
    id: String,
    events: Vec<String>,
}

impl CloseHandle for RecordingHandle {
    fn id(&self) -> &str {
        &self.id
    }
    fn abort_and_wait(&mut self) {
        self.events.push("abort_and_wait".to_owned());
    }
    fn close(&mut self) {
        self.events.push("close".to_owned());
    }
    fn release_lease(&mut self) {
        self.events.push("release_lease".to_owned());
    }
}

struct RecordingRegistry {
    held: Vec<RecordingHandle>,
    taken: Vec<String>,
}

impl CloseRegistry for RecordingRegistry {
    fn take(&mut self, id: &str) -> Option<Box<dyn CloseHandle>> {
        let index = self.held.iter().position(|h| h.id == id)?;
        let handle = self.held.remove(index);
        self.taken.push(handle.id.clone());
        Some(Box::new(handle))
    }
    fn drain_all(&mut self) -> Vec<Box<dyn CloseHandle>> {
        self.held
            .drain(..)
            .map(|h| Box::new(h) as Box<dyn CloseHandle>)
            .collect()
    }
}

#[test]
fn invalid_id_refuses_before_registry_lookup() {
    let mut registry = RecordingRegistry {
        held: vec![],
        taken: vec![],
    };
    let err = close_one("session/close", "has space", &mut registry).unwrap_err();
    assert_eq!(err.code, RpcErrorCode::InvalidParams);
    assert_eq!(err.message, "session/close: invalid sessionId");
}

#[test]
fn unknown_valid_id_closes_as_silent_noop_success() {
    let mut registry = RecordingRegistry {
        held: vec![],
        taken: vec![],
    };
    close_one("session/close", "s9", &mut registry).unwrap();
    assert!(registry.taken.is_empty());
}

#[test]
fn held_session_tears_down_abort_then_close_then_release() {
    let mut registry = RecordingRegistry {
        held: vec![RecordingHandle {
            id: "s1".to_owned(),
            events: vec![],
        }],
        taken: vec![],
    };
    close_one("session/close", "s1", &mut registry).unwrap();
    assert_eq!(registry.taken, vec!["s1".to_owned()]);
    assert!(registry.held.is_empty());
}

#[test]
fn teardown_order_is_abort_close_release() {
    let mut handle = RecordingHandle {
        id: "s2".to_owned(),
        events: vec![],
    };
    teardown(&mut handle);
    assert_eq!(
        handle.events,
        vec!["abort_and_wait", "close", "release_lease"]
    );
}

#[test]
fn second_close_of_the_same_id_is_again_a_noop() {
    let mut registry = RecordingRegistry {
        held: vec![RecordingHandle {
            id: "s1".to_owned(),
            events: vec![],
        }],
        taken: vec![],
    };
    close_one("session/close", "s1", &mut registry).unwrap();
    close_one("session/close", "s1", &mut registry).unwrap();
    assert_eq!(registry.taken.len(), 1);
}

#[test]
fn close_all_drains_every_held_session() {
    let mut registry = RecordingRegistry {
        held: vec![
            RecordingHandle {
                id: "s1".to_owned(),
                events: vec![],
            },
            RecordingHandle {
                id: "s2".to_owned(),
                events: vec![],
            },
        ],
        taken: vec![],
    };
    let ids = close_all(&mut registry);
    assert_eq!(ids, vec!["s1".to_owned(), "s2".to_owned()]);
    assert!(registry.held.is_empty());
}
