//! ACP-036: the inbox surface — enqueue intent normalization, the refusal map
//! from queue sentinel to frozen JSON-RPC code, and the prepared request /
//! receipt shapes (`inbox.go:17-140`; `sessioninbox/types.go:21-26,72-83,212`).
//!
//! The real queue, controller `EditorAPI`, and session registry are runtime
//! wiring. This module owns the pure decision table: every condition the queue
//! can refuse with, its stable `data.code`, and which of the two refusal
//! classes a client must act on.

use serde_json::json;

use crate::errors::{RpcError, RpcErrorCode};
use crate::methods::SESSION_INBOX_ENQUEUE;
use crate::wire::SessionInboxEnqueueParams;

/// Inbox address source stamped on every ACP-enqueued item (`inbox.go:118`).
pub const ENQUEUE_SOURCE: &str = "acp";

/// Receipt disposition vocabulary (`sessioninbox/types.go:58-67`).
pub const DISPOSITION_STARTED: &str = "started";
pub const DISPOSITION_STEER_ACCEPTED: &str = "steer_accepted";
pub const DISPOSITION_QUEUED_FOLLOWUP: &str = "queued_followup";
pub const DISPOSITION_REJECTED_BUSY: &str = "rejected_busy";
pub const DISPOSITION_REJECTED_ROTATING: &str = "rejected_rotating";
pub const DISPOSITION_REJECTED_CLOSED: &str = "rejected_closed";
pub const DISPOSITION_REJECTED_CAPACITY: &str = "rejected_capacity";
pub const DISPOSITION_IDEMPOTENT_HIT: &str = "idempotent_hit";
pub const DISPOSITION_DELETED: &str = "deleted";
pub const DISPOSITION_CANCELLED: &str = "cancelled";

/// Every disposition the kernel can report on a receipt.
pub const DISPOSITIONS: [&str; 10] = [
    DISPOSITION_STARTED,
    DISPOSITION_STEER_ACCEPTED,
    DISPOSITION_QUEUED_FOLLOWUP,
    DISPOSITION_REJECTED_BUSY,
    DISPOSITION_REJECTED_ROTATING,
    DISPOSITION_REJECTED_CLOSED,
    DISPOSITION_REJECTED_CAPACITY,
    DISPOSITION_IDEMPOTENT_HIT,
    DISPOSITION_DELETED,
    DISPOSITION_CANCELLED,
];

/// Durable follow-up turn vs a mid-turn steer (`sessioninbox/types.go:25-26`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboxIntent {
    Followup,
    Steer,
}

impl InboxIntent {
    /// Kernel intent wire value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Followup => "followup",
            Self::Steer => "steer",
        }
    }

    /// Resolves the request's `intent` field: only a case-insensitive `"steer"`
    /// switches intent; anything else — including the missing field — stays a
    /// passive follow-up (`inbox.go:109-112`).
    pub fn parse(raw: Option<&str>) -> Self {
        match raw {
            Some(value) if value.eq_ignore_ascii_case("steer") => Self::Steer,
            _ => Self::Followup,
        }
    }
}

/// Every condition the queue can refuse an inbox call with (`inbox.go:40-78`).
///
/// [`InboxRefusal::RetryMayRepeat`] is the one answer a client must never retry
/// blindly; [`InboxRefusal::Empty`] is a malformed request, not a queue
/// refusal; [`InboxRefusal::UnknownFault`] is a fault the surface cannot name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboxRefusal {
    SteerAlreadyApplied,
    ItemTooLarge,
    CapacityItems,
    CapacityBytes,
    InvalidState,
    RetryMayRepeat,
    ForeignRun,
    Paused,
    Unrecovered,
    NotFound,
    IdempotencyConflict,
    SchemaReadonly,
    Closed,
    Empty,
    UnknownFault,
}

impl InboxRefusal {
    /// Stable dotted identity reused from the HTTP frontend; omitted for the
    /// unnamed internal fault (`inbox.go:40-78`).
    pub fn data_code(self) -> Option<&'static str> {
        Some(match self {
            Self::SteerAlreadyApplied => "steer.already_applied",
            Self::ItemTooLarge => "inbox.item_too_large",
            Self::CapacityItems => "inbox.capacity_items",
            Self::CapacityBytes => "inbox.capacity_bytes",
            Self::InvalidState => "inbox.invalid_state",
            Self::RetryMayRepeat => "inbox.retry_may_repeat",
            Self::ForeignRun => "inbox.foreign_run",
            Self::Paused => "inbox.paused",
            Self::Unrecovered => "inbox.unrecovered",
            Self::NotFound => "inbox.not_found",
            Self::IdempotencyConflict => "inbox.idempotency_conflict",
            Self::SchemaReadonly => "inbox.schema_readonly",
            Self::Closed => "inbox.closed",
            Self::Empty => "inbox.empty",
            Self::UnknownFault => return None,
        })
    }

    /// Frozen JSON-RPC class code (`inbox.go:17-20,40-78`).
    pub fn rpc_code(self) -> RpcErrorCode {
        match self {
            Self::RetryMayRepeat => RpcErrorCode::InboxMayHaveRun,
            Self::Empty => RpcErrorCode::InvalidParams,
            Self::UnknownFault => RpcErrorCode::Internal,
            _ => RpcErrorCode::InboxRefused,
        }
    }
}

/// Builds the refusal envelope: typed class code, `data.code` condition, and
/// the kernel's English message (`inbox.go:31-33`).
pub fn refuse(refusal: InboxRefusal, message: impl Into<String>) -> RpcError {
    let code = refusal.rpc_code();
    match refusal.data_code() {
        Some(data_code) => RpcError::new(code, message).with_data(json!({"code": data_code})),
        None => RpcError::new(code, message),
    }
}

/// One enqueue request resolved against the ACP boundary (`inbox.go:113-120`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedEnqueue {
    pub intent: InboxIntent,
    pub display: String,
    pub raw: String,
    pub submit: String,
    pub source: &'static str,
    pub idempotency: String,
}

/// Trims and validates the enqueue body, then fills the three text copies the
/// kernel stores. Empty text is a `-32602` `inbox.empty` refusal
/// (`inbox.go:105-120`).
pub fn prepare_enqueue(params: &SessionInboxEnqueueParams) -> Result<PreparedEnqueue, RpcError> {
    let text = params.text.trim().to_owned();
    if text.is_empty() {
        return Err(refuse(
            InboxRefusal::Empty,
            format!("{SESSION_INBOX_ENQUEUE}: empty text"),
        ));
    }
    let intent = InboxIntent::parse(params.intent.as_deref());
    let idempotency = params
        .idempotency_key
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_owned();
    Ok(PreparedEnqueue {
        intent,
        display: text.clone(),
        raw: text.clone(),
        submit: text,
        source: ENQUEUE_SOURCE,
        idempotency,
    })
}

/// Receipt returned by the queue (`inbox.go:133-139`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnqueueReceipt {
    pub item_id: String,
    pub disposition: String,
    pub position: i32,
    pub paused: bool,
    pub idempotent: bool,
}

impl From<EnqueueReceipt> for crate::wire::SessionInboxEnqueueResult {
    fn from(receipt: EnqueueReceipt) -> Self {
        Self {
            item_id: receipt.item_id,
            disposition: receipt.disposition,
            position: receipt.position,
            paused: receipt.paused,
            idempotent: receipt.idempotent,
        }
    }
}

/// Session lookup seam result (`inbox.go:80-93`): the session itself is missing,
/// or its controller does not expose the inbox editor API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionApiRefusal {
    UnknownSession,
    NoInboxApi,
}

impl SessionApiRefusal {
    /// Validates the two lookup preconditions; the runtime passes what it found.
    pub fn check(has_session: bool, has_inbox_api: bool) -> Result<(), Self> {
        if !has_session {
            return Err(Self::UnknownSession);
        }
        if !has_inbox_api {
            return Err(Self::NoInboxApi);
        }
        Ok(())
    }

    /// Maps the seam refusal onto its wire error (`inbox.go:81-88`).
    pub fn into_error(self) -> RpcError {
        match self {
            Self::UnknownSession => RpcError::invalid_params("unknown session"),
            Self::NoInboxApi => RpcError::internal("session controller does not expose inbox"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::inbox_refusal_rpc_code;

    #[test]
    fn intent_defaults_to_followup_and_matches_steer_case_insensitively() {
        assert_eq!(InboxIntent::parse(None), InboxIntent::Followup);
        assert_eq!(InboxIntent::parse(Some("")), InboxIntent::Followup);
        assert_eq!(InboxIntent::parse(Some("STeEr")), InboxIntent::Steer);
        assert_eq!(InboxIntent::parse(Some("steer")), InboxIntent::Steer);
        assert_eq!(InboxIntent::parse(Some("followup")), InboxIntent::Followup);
        assert_eq!(InboxIntent::Followup.as_str(), "followup");
        assert_eq!(InboxIntent::Steer.as_str(), "steer");
    }

    #[test]
    fn refusal_map_matches_the_extensions_code_table() {
        let cases = [
            (InboxRefusal::SteerAlreadyApplied, "steer.already_applied"),
            (InboxRefusal::ItemTooLarge, "inbox.item_too_large"),
            (InboxRefusal::CapacityItems, "inbox.capacity_items"),
            (InboxRefusal::CapacityBytes, "inbox.capacity_bytes"),
            (InboxRefusal::InvalidState, "inbox.invalid_state"),
            (InboxRefusal::ForeignRun, "inbox.foreign_run"),
            (InboxRefusal::Paused, "inbox.paused"),
            (InboxRefusal::Unrecovered, "inbox.unrecovered"),
            (InboxRefusal::NotFound, "inbox.not_found"),
            (
                InboxRefusal::IdempotencyConflict,
                "inbox.idempotency_conflict",
            ),
            (InboxRefusal::SchemaReadonly, "inbox.schema_readonly"),
            (InboxRefusal::Closed, "inbox.closed"),
        ];
        for (refusal, code) in cases {
            assert_eq!(refusal.data_code(), Some(code));
            assert_eq!(refusal.rpc_code(), RpcErrorCode::InboxRefused);
            assert_eq!(refusal.rpc_code(), inbox_refusal_rpc_code(code), "{code}");
        }
        assert_eq!(
            InboxRefusal::RetryMayRepeat.rpc_code(),
            RpcErrorCode::InboxMayHaveRun
        );
        assert_eq!(
            InboxRefusal::RetryMayRepeat.data_code(),
            Some("inbox.retry_may_repeat")
        );
        assert_eq!(InboxRefusal::Empty.rpc_code(), RpcErrorCode::InvalidParams);
        assert_eq!(InboxRefusal::Empty.data_code(), Some("inbox.empty"));
        assert_eq!(
            InboxRefusal::UnknownFault.rpc_code(),
            RpcErrorCode::Internal
        );
        assert_eq!(InboxRefusal::UnknownFault.data_code(), None);
    }

    #[test]
    fn refusal_envelope_carries_class_code_and_data_identity() {
        let err = refuse(InboxRefusal::NotFound, "inbox item not found");
        assert_eq!(err.code, RpcErrorCode::InboxRefused);
        let data = err.data.expect("data present");
        assert_eq!(data["code"], "inbox.not_found");

        let fault = refuse(InboxRefusal::UnknownFault, "queue boom");
        assert_eq!(fault.code, RpcErrorCode::Internal);
        assert!(fault.data.is_none());
    }

    #[test]
    fn prepare_enqueue_trims_text_stamps_acp_source_and_keeps_idempotency_key() {
        let params = SessionInboxEnqueueParams {
            session_id: "s1".to_owned(),
            text: "  fix the ledger  ".to_owned(),
            intent: Some("STEER".to_owned()),
            idempotency_key: Some("  key-7 ".to_owned()),
        };
        let prepared = prepare_enqueue(&params).unwrap();
        assert_eq!(prepared.intent, InboxIntent::Steer);
        assert_eq!(prepared.display, "fix the ledger");
        assert_eq!(prepared.raw, "fix the ledger");
        assert_eq!(prepared.submit, "fix the ledger");
        assert_eq!(prepared.source, ENQUEUE_SOURCE);
        assert_eq!(prepared.idempotency, "key-7");
    }

    #[test]
    fn empty_text_refuses_with_inbox_empty() {
        let params = SessionInboxEnqueueParams {
            session_id: "s1".to_owned(),
            text: "   ".to_owned(),
            intent: None,
            idempotency_key: None,
        };
        let err = prepare_enqueue(&params).unwrap_err();
        assert_eq!(err.code, RpcErrorCode::InvalidParams);
        assert_eq!(
            err.message,
            "_reasonix.io/session/inbox/enqueue: empty text"
        );
        assert_eq!(err.data.expect("data present")["code"], "inbox.empty");
    }

    #[test]
    fn disposition_vocabulary_covers_the_ten_kernel_values() {
        assert_eq!(DISPOSITIONS.len(), 10);
        assert!(DISPOSITIONS.contains(&"steer_accepted"));
        assert!(DISPOSITIONS.contains(&"idempotent_hit"));
    }

    #[test]
    fn session_api_refusal_distinguishes_missing_session_from_missing_api() {
        assert!(SessionApiRefusal::check(false, true).is_err());
        assert!(SessionApiRefusal::check(true, false).is_err());
        assert!(SessionApiRefusal::check(true, true).is_ok());

        let missing = SessionApiRefusal::UnknownSession.into_error();
        assert_eq!(missing.code, RpcErrorCode::InvalidParams);
        assert_eq!(missing.message, "unknown session");

        let no_api = SessionApiRefusal::NoInboxApi.into_error();
        assert_eq!(no_api.code, RpcErrorCode::Internal);
        assert_eq!(no_api.message, "session controller does not expose inbox");
    }
}
