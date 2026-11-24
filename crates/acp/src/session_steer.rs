//! ACP-027 `_reasonix.io/session/steer` durable admission and pathless fallback
//! (`session.go:432-474`).
//!
//! The durable branch requires an `EditorAPI` whose session path is non-empty;
//! a controller that is not an `EditorAPI`, or one that is an `EditorAPI` with
//! an empty path, falls back to the ephemeral `TrySteer` compatibility path.
//! The real inbox record, steer admission state machine, and executor injection
//! are runtime wiring; this module owns the branch decision, the durable
//! request shape, the receipt mapping, and the error identities.

use crate::errors::RpcError;
use crate::lifecycle::no_active_prompt;
use crate::wire::SessionSteerResult;

/// Durable inbox intent carried by the steer request (`session.go:457`).
pub const STEER_INTENT: &str = "steer";
/// Inbox source tag for ACP-originated guidance (`session.go:461`).
pub const STEER_SOURCE: &str = "acp";

/// Durable inbox request the adapter hands to the controller
/// (`session.go:456-462`): all three text copies carry the same flattened
/// prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteerRequest {
    pub intent: String,
    pub display: String,
    pub raw: String,
    pub submit: String,
    pub source: String,
}

impl SteerRequest {
    pub fn steer(text: &str) -> Self {
        Self {
            intent: STEER_INTENT.to_owned(),
            display: text.to_owned(),
            raw: text.to_owned(),
            submit: text.to_owned(),
            source: STEER_SOURCE.to_owned(),
        }
    }
}

/// Receipt returned by the durable enqueue-and-steer call (`sessioninbox/types.go:231-238`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteerReceipt {
    pub item_id: String,
    pub disposition: String,
}

/// Controller seam the steer decision routes through.
pub trait SteerBackend {
    /// Non-empty session path means the durable inbox is usable; `None` means
    /// the fallback ephemeral path applies.
    fn durable_path(&self) -> Option<&str>;
    /// Durable admission; the inner `String` is the kernel error message,
    /// wrapped message-only without an inbox refusal `data.code`.
    fn enqueue_steer(&mut self, request: &SteerRequest) -> Result<SteerReceipt, String>;
    /// Ephemeral mid-turn redirect for pathless controllers.
    fn try_steer(&mut self, text: &str) -> bool;
}

/// Routes a validated, non-empty steer prompt.
///
/// Durable enqueue failures map to `-32600` with a message only: unlike the
/// `inbox/enqueue` verb, the steer handler does not translate kernel refusal
/// codes into `data.code` identities (`session.go:463-465`; matrix A §027 S5).
pub fn admit_steer(
    method: &str,
    text: &str,
    backend: &mut dyn SteerBackend,
) -> Result<SessionSteerResult, RpcError> {
    match backend.durable_path() {
        Some(path) if !path.is_empty() => {
            let request = SteerRequest::steer(text);
            match backend.enqueue_steer(&request) {
                Ok(receipt) => Ok(SessionSteerResult {
                    item_id: Some(receipt.item_id),
                    disposition: Some(receipt.disposition),
                }),
                Err(message) => Err(RpcError::new(
                    crate::errors::RpcErrorCode::InvalidRequest,
                    format!("{method}: {message}"),
                )),
            }
        }
        _ => {
            if !backend.try_steer(text) {
                return Err(no_active_prompt(method));
            }
            Ok(SessionSteerResult {
                item_id: None,
                disposition: Some(crate::wire::STEER_DISPOSITION_ACCEPTED.to_owned()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::RpcErrorCode;
    use crate::methods::SESSION_STEER;

    struct FakeBackend {
        path: Option<String>,
        enqueue: Result<SteerReceipt, String>,
        steer_ok: bool,
        last_request: Option<SteerRequest>,
        last_text: Option<String>,
    }

    impl SteerBackend for FakeBackend {
        fn durable_path(&self) -> Option<&str> {
            self.path.as_deref()
        }
        fn enqueue_steer(&mut self, request: &SteerRequest) -> Result<SteerReceipt, String> {
            self.last_request = Some(request.clone());
            self.enqueue.clone()
        }
        fn try_steer(&mut self, text: &str) -> bool {
            self.last_text = Some(text.to_owned());
            self.steer_ok
        }
    }

    #[test]
    fn durable_path_enqueues_and_returns_receipt() {
        let mut backend = FakeBackend {
            path: Some("/sessions/s1.jsonl".to_owned()),
            enqueue: Ok(SteerReceipt {
                item_id: "i7".to_owned(),
                disposition: crate::wire::STEER_DISPOSITION_QUEUED.to_owned(),
            }),
            steer_ok: false,
            last_request: None,
            last_text: None,
        };
        let result = admit_steer(SESSION_STEER, "refine the plan", &mut backend).unwrap();
        assert_eq!(result.item_id.as_deref(), Some("i7"));
        assert_eq!(result.disposition.as_deref(), Some("queued_followup"));

        let request = backend.last_request.as_ref().unwrap();
        assert_eq!(request.intent, STEER_INTENT);
        assert_eq!(request.source, STEER_SOURCE);
        assert_eq!(request.display, "refine the plan");
        assert_eq!(request.raw, "refine the plan");
        assert_eq!(request.submit, "refine the plan");
        assert!(backend.last_text.is_none());
    }

    #[test]
    fn durable_failure_is_message_only_without_data_code() {
        let mut backend = FakeBackend {
            path: Some("/sessions/s1.jsonl".to_owned()),
            enqueue: Err("inbox is closed".to_owned()),
            steer_ok: false,
            last_request: None,
            last_text: None,
        };
        let err = admit_steer(SESSION_STEER, "go faster", &mut backend).unwrap_err();
        assert_eq!(err.code, RpcErrorCode::InvalidRequest);
        assert_eq!(err.message, "_reasonix.io/session/steer: inbox is closed");
        assert!(err.data.is_none());
    }

    #[test]
    fn pathless_controller_falls_back_to_try_steer() {
        let mut backend = FakeBackend {
            path: None,
            enqueue: Ok(SteerReceipt {
                item_id: "should-not-use".to_owned(),
                disposition: "queued_followup".to_owned(),
            }),
            steer_ok: true,
            last_request: None,
            last_text: None,
        };
        let result = admit_steer(SESSION_STEER, "adjust", &mut backend).unwrap();
        assert!(result.item_id.is_none());
        assert_eq!(result.disposition.as_deref(), Some("steer_accepted"));
        assert_eq!(backend.last_text.as_deref(), Some("adjust"));
    }

    #[test]
    fn editorapi_with_empty_path_also_falls_back() {
        let mut backend = FakeBackend {
            path: Some(String::new()),
            enqueue: Ok(SteerReceipt {
                item_id: "x".to_owned(),
                disposition: "queued_followup".to_owned(),
            }),
            steer_ok: false,
            last_request: None,
            last_text: None,
        };
        let err = admit_steer(SESSION_STEER, "nudge", &mut backend).unwrap_err();
        assert_eq!(err.code, RpcErrorCode::InvalidRequest);
        assert_eq!(
            err.message,
            "_reasonix.io/session/steer: session has no active prompt"
        );
    }
}
