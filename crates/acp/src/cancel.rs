//! In-flight request cancellation set (`$/cancel_request`) and the
//! `-32800` answer rewrite (cancel_request.go:19-72; server.go:232-271).
//!
//! Pure synchronous bookkeeping: the set maps a trimmed request id to its
//! cancel flag. There is no runtime, no thread, and no stored response body.
//! Real context cancellation lives in the session runtime; this module owns
//! the id accounting and the rewrite decision.

use std::collections::HashMap;

use serde_json::Value;

/// One registered in-flight request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Flight {
    cancelled: bool,
}

/// Maps raw request ids to their flight state.
#[derive(Debug, Default)]
pub struct FlightSet {
    flights: HashMap<String, Flight>,
}

/// Normalizes a raw id the way the reference keys its map: trimmed original
/// string (cancel_request.go:31).
pub fn flight_key(id: &Value) -> String {
    match id {
        Value::String(s) => s.trim().to_owned(),
        other => other.to_string().trim().to_owned(),
    }
}

/// Extracts `requestId` from a cancel notification params. Malformed bodies and
/// empty ids are silently ignored — a notification has no channel to report
/// the problem on.
pub fn cancel_request_id(params: Option<&Value>) -> Option<String> {
    let params = params?;
    let id = params.get("requestId")?;
    match id {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

impl FlightSet {
    /// Registers a flight when a request enters the handler, returning its
    /// normalized key. A cancel that arrives before registration has no entry
    /// to hit, so a queued request is never pre-cancelled (matrix B §6.8).
    pub fn begin(&mut self, id: &Value) -> String {
        let key = flight_key(id);
        self.flights
            .insert(key.clone(), Flight { cancelled: false });
        key
    }

    /// Marks a flight cancelled and signals its cancellation. Unknown,
    /// already-answered, or malformed ids are no-ops.
    pub fn cancel(&mut self, params: Option<&Value>) {
        let Some(key) = cancel_request_id(params) else {
            return;
        };
        if let Some(flight) = self.flights.get_mut(&key) {
            flight.cancelled = true;
        }
    }

    /// True when the flight for this id was cancelled.
    pub fn is_cancelled(&self, id: &str) -> bool {
        self.flights.get(id).is_some_and(|flight| flight.cancelled)
    }

    /// Removes a flight. Only the same flight entry is removed; a late cancel
    /// after removal stays a no-op.
    pub fn end(&mut self, id: &str) {
        self.flights.remove(id);
    }

    /// Number of registered flights, for diagnostics and tests.
    pub fn len(&self) -> usize {
        self.flights.len()
    }

    pub fn is_empty(&self) -> bool {
        self.flights.is_empty()
    }
}

/// Rewrite condition (server.go:249-251): the flight must be flagged cancelled
/// and its context must actually be done. Either half missing keeps the
/// original error identity.
pub fn should_rewrite_cancelled(flight_cancelled: bool, ctx_done: bool) -> bool {
    flight_cancelled && ctx_done
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::RpcError;

    fn raw(id: &str) -> Value {
        serde_json::from_str(id).unwrap()
    }

    #[test]
    fn begin_end_registers_and_removes_flight() {
        let mut set = FlightSet::default();
        let key = set.begin(&raw("42"));
        assert_eq!(set.len(), 1);
        assert!(!set.is_cancelled(&key));
        set.end(&key);
        assert!(set.is_empty());
    }

    #[test]
    fn cancel_marks_known_flight_and_ignores_unknown_or_malformed() {
        let mut set = FlightSet::default();
        let key = set.begin(&raw("7"));

        set.cancel(Some(&serde_json::json!({"requestId": "7"})));
        assert!(set.is_cancelled(&key));

        set.cancel(Some(&serde_json::json!({"requestId": "missing"})));
        set.cancel(Some(&serde_json::json!({"requestId": ""})));
        set.cancel(Some(&serde_json::json!({"requestId": {"nested": 1}})));
        set.cancel(None);
        assert_eq!(set.len(), 1);
        set.end(&key);
    }

    #[test]
    fn answered_flight_cancel_is_noop() {
        let mut set = FlightSet::default();
        let key = set.begin(&raw("1"));
        set.end(&key);
        set.cancel(Some(&serde_json::json!({"requestId": "1"})));
        assert!(!set.is_cancelled("1"));
    }

    #[test]
    fn cancel_before_registration_does_not_precancel() {
        let mut set = FlightSet::default();
        set.cancel(Some(&serde_json::json!({"requestId": "9"})));
        let key = set.begin(&raw("9"));
        assert!(!set.is_cancelled(&key));
        set.end(&key);
    }

    #[test]
    fn rewrite_requires_flag_and_context_done() {
        assert!(should_rewrite_cancelled(true, true));
        assert!(!should_rewrite_cancelled(true, false));
        assert!(!should_rewrite_cancelled(false, true));
        assert!(!should_rewrite_cancelled(false, false));
    }

    #[test]
    fn cancelled_rewrite_keeps_message_drops_data() {
        let err = RpcError::internal("boom").with_data(serde_json::json!({"x": 1}));
        assert!(should_rewrite_cancelled(true, true));
        let rewritten = err.rewrite_cancelled();
        assert_eq!(
            rewritten.code,
            crate::errors::RpcErrorCode::RequestCancelled
        );
        assert_eq!(rewritten.message, "boom");
        assert!(rewritten.data.is_none());
    }
}
