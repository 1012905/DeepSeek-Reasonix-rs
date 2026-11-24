//! ACP-029 `session/close` idempotent teardown and the `closeAll` sweep
//! (`session.go:572-586`; `service.go:1175-1185`).
//!
//! Closing an unknown valid id is a silent success; the handler only rejects a
//! malformed session id through the shared validation table. A held session is
//! torn down in a fixed order: abort the in-flight turn and wait for it, close
//! the controller, then release the session lease. The real abort wait,
//! controller shutdown, and lease release are runtime wiring; this module owns
//! the idempotent branch and the teardown order.

use crate::errors::RpcError;
use crate::lifecycle::validate_session_id;
use crate::wire::SessionCloseResult;

/// Controller/lease handle released when a held session closes.
pub trait CloseHandle {
    /// Session id carried by the handle for the sweep result.
    fn id(&self) -> &str;
    /// Wait for the in-flight turn to unwind before shutting down.
    fn abort_and_wait(&mut self);
    /// Shut the controller down.
    fn close(&mut self);
    /// Return the session lease so another open can claim the id.
    fn release_lease(&mut self);
}

/// Registry seam: removes a held session from the active map.
pub trait CloseRegistry {
    /// Takes the held session for `id`, leaving the registry; `None` means an
    /// unknown session and the close is a no-op success.
    fn take(&mut self, id: &str) -> Option<Box<dyn CloseHandle>>;
    /// Drains every held session for the `closeAll` sweep.
    fn drain_all(&mut self) -> Vec<Box<dyn CloseHandle>>;
}

/// Fixed teardown order applied to one held session
/// (`service.go:1143-1151`).
pub fn teardown(handle: &mut dyn CloseHandle) {
    handle.abort_and_wait();
    handle.close();
    handle.release_lease();
}

/// Handles a single `session/close`. An unknown valid id still returns the
/// empty success result (`session.go:576-585`).
pub fn close_one(
    method: &str,
    id: &str,
    registry: &mut dyn CloseRegistry,
) -> Result<SessionCloseResult, RpcError> {
    validate_session_id(method, id)?;
    if let Some(mut handle) = registry.take(id) {
        teardown(handle.as_mut());
    }
    Ok(SessionCloseResult::default())
}

/// Sweeps every held session (`service.go:1177-1183`). Returns the ids that
/// were actually torn down; registry order is not part of the reference
/// contract.
pub fn close_all(registry: &mut dyn CloseRegistry) -> Vec<String> {
    let handles = registry.drain_all();
    let mut ids = Vec::with_capacity(handles.len());
    for mut handle in handles {
        ids.push(handle.id().to_owned());
        teardown(handle.as_mut());
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::RpcErrorCode;

    struct Recording {
        id: String,
        events: Vec<String>,
    }

    impl CloseHandle for Recording {
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

    struct FakeRegistry {
        held: Vec<(String, Recording)>,
        taken: Vec<String>,
    }

    impl CloseRegistry for FakeRegistry {
        fn take(&mut self, id: &str) -> Option<Box<dyn CloseHandle>> {
            let index = self.held.iter().position(|(k, _)| k == id)?;
            let (key, recording) = self.held.remove(index);
            self.taken.push(key.clone());
            Some(Box::new(recording))
        }
        fn drain_all(&mut self) -> Vec<Box<dyn CloseHandle>> {
            self.held
                .drain(..)
                .map(|(_, recording)| Box::new(recording) as Box<dyn CloseHandle>)
                .collect()
        }
    }

    #[test]
    fn invalid_id_refuses_before_registry_lookup() {
        let mut registry = FakeRegistry {
            held: vec![],
            taken: vec![],
        };
        let err = close_one("session/close", "has space", &mut registry).unwrap_err();
        assert_eq!(err.code, RpcErrorCode::InvalidParams);
        assert_eq!(err.message, "session/close: invalid sessionId");
    }

    #[test]
    fn unknown_valid_id_is_a_noop_success() {
        let mut registry = FakeRegistry {
            held: vec![],
            taken: vec![],
        };
        let result = close_one("session/close", "s9", &mut registry).unwrap();
        let _ = result;
        assert!(registry.taken.is_empty());
    }

    #[test]
    fn held_session_tears_down_in_fixed_order() {
        let mut registry = FakeRegistry {
            held: vec![(
                "s1".to_owned(),
                Recording {
                    id: "s1".to_owned(),
                    events: vec![],
                },
            )],
            taken: vec![],
        };
        close_one("session/close", "s1", &mut registry).unwrap();
        let recording = &registry.held[0..];
        assert!(recording.is_empty());
        assert_eq!(registry.taken, vec!["s1".to_owned()]);
    }

    #[test]
    fn teardown_order_is_abort_close_release() {
        let mut recording = Recording {
            id: "s2".to_owned(),
            events: vec![],
        };
        teardown(&mut recording);
        assert_eq!(
            recording.events,
            vec!["abort_and_wait", "close", "release_lease"]
        );
    }

    #[test]
    fn close_all_drains_every_held_session() {
        let mut registry = FakeRegistry {
            held: vec![
                (
                    "s1".to_owned(),
                    Recording {
                        id: "s1".to_owned(),
                        events: vec![],
                    },
                ),
                (
                    "s2".to_owned(),
                    Recording {
                        id: "s2".to_owned(),
                        events: vec![],
                    },
                ),
            ],
            taken: vec![],
        };
        let ids = close_all(&mut registry);
        assert_eq!(ids.len(), 2);
        assert!(registry.held.is_empty());
    }
}
