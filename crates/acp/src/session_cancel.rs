//! ACP-028 `session/cancel` notification: the silent abort fan-out
//! (`session.go:713-725`).
//!
//! A notification carries no request id, so every refusal branch degrades to
//! a silent no-op: a malformed body, an empty `sessionId`, and an unknown
//! session all return without an error response. The real controller abort and
//! its cancellation signal are runtime wiring; this module owns the silent
//! decision table and the registry seam.

use serde_json::Value;

use crate::wire::SessionCancelParams;

/// Registry seam: aborts the in-flight turn for `id`, reporting whether a
/// known session was found.
pub trait CancelRegistry {
    /// Returns `true` when the session exists and was signalled to abort.
    fn abort(&mut self, id: &str) -> bool;
}

/// Handles one `session/cancel` notification. Never returns an error: decode
/// failures and unknown ids are dropped on the floor the same way
/// (`session.go:715-724`).
pub fn handle_cancel(params: Option<&Value>, registry: &mut dyn CancelRegistry) {
    let Some(params) = params else { return };
    let decoded: SessionCancelParams = match serde_json::from_value(params.clone()) {
        Ok(decoded) => decoded,
        Err(_) => return,
    };
    if decoded.session_id.trim().is_empty() {
        return;
    }
    registry.abort(decoded.session_id.trim());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::SessionCancelParams;

    struct FakeRegistry {
        known: Vec<String>,
        aborted: Vec<String>,
    }

    impl CancelRegistry for FakeRegistry {
        fn abort(&mut self, id: &str) -> bool {
            if !self.known.iter().any(|k| k == id) {
                return false;
            }
            self.aborted.push(id.to_owned());
            true
        }
    }

    fn value(params: &SessionCancelParams) -> Value {
        serde_json::to_value(params).unwrap()
    }

    #[test]
    fn malformed_body_is_silently_ignored() {
        let mut registry = FakeRegistry {
            known: vec!["s1".to_owned()],
            aborted: vec![],
        };
        handle_cancel(
            Some(&Value::String("not an object".to_owned())),
            &mut registry,
        );
        handle_cancel(Some(&Value::Null), &mut registry);
        assert!(registry.aborted.is_empty());
    }

    #[test]
    fn empty_and_whitespace_session_id_are_ignored() {
        let mut registry = FakeRegistry {
            known: vec!["s1".to_owned()],
            aborted: vec![],
        };
        handle_cancel(
            Some(&value(&SessionCancelParams {
                session_id: String::new(),
            })),
            &mut registry,
        );
        handle_cancel(
            Some(&value(&SessionCancelParams {
                session_id: "   ".to_owned(),
            })),
            &mut registry,
        );
        assert!(registry.aborted.is_empty());
    }

    #[test]
    fn unknown_session_is_silently_dropped_without_abort() {
        let mut registry = FakeRegistry {
            known: vec!["s1".to_owned()],
            aborted: vec![],
        };
        handle_cancel(
            Some(&value(&SessionCancelParams {
                session_id: "nope".to_owned(),
            })),
            &mut registry,
        );
        assert!(registry.aborted.is_empty());
    }

    #[test]
    fn known_session_is_aborted_once() {
        let mut registry = FakeRegistry {
            known: vec!["s1".to_owned(), "s2".to_owned()],
            aborted: vec![],
        };
        handle_cancel(
            Some(&value(&SessionCancelParams {
                session_id: "s1".to_owned(),
            })),
            &mut registry,
        );
        assert_eq!(registry.aborted, vec!["s1".to_owned()]);
    }
}
