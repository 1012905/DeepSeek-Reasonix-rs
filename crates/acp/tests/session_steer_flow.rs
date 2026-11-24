//! ACP-027: steer durable admission versus the pathless ephemeral fallback,
//! request shape, receipt mapping, and message-only durable failures
//! (`session.go:432-474`).

use reasonix_acp::errors::RpcErrorCode;
use reasonix_acp::methods::SESSION_STEER;
use reasonix_acp::session_steer::{
    STEER_INTENT, STEER_SOURCE, SteerBackend, SteerReceipt, SteerRequest, admit_steer,
};

struct RecordingBackend {
    path: Option<String>,
    enqueue: Result<SteerReceipt, String>,
    steer_ok: bool,
    last_request: Option<SteerRequest>,
    last_text: Option<String>,
}

impl SteerBackend for RecordingBackend {
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
fn durable_path_enqueues_with_steer_acp_shape_and_receipt() {
    let mut backend = RecordingBackend {
        path: Some("/sessions/s1.jsonl".to_owned()),
        enqueue: Ok(SteerReceipt {
            item_id: "i7".to_owned(),
            disposition: "queued_followup".to_owned(),
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
    let mut backend = RecordingBackend {
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
fn pathless_controller_falls_back_to_try_steer_without_item_id() {
    let mut backend = RecordingBackend {
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
fn editorapi_with_empty_session_path_also_falls_back() {
    let mut backend = RecordingBackend {
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

#[test]
fn fallback_without_active_prompt_refuses() {
    let mut backend = RecordingBackend {
        path: None,
        enqueue: Ok(SteerReceipt {
            item_id: "x".to_owned(),
            disposition: "queued_followup".to_owned(),
        }),
        steer_ok: false,
        last_request: None,
        last_text: None,
    };
    let err = admit_steer(SESSION_STEER, "nudge", &mut backend).unwrap_err();
    assert_eq!(
        err.message,
        "_reasonix.io/session/steer: session has no active prompt"
    );
}
