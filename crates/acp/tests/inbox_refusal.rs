//! ACP-036: inbox refusal map (13 named codes + empty + default), intent
//! normalization, prepared enqueue request, receipt shape, and session-API
//! preconditions.

use reasonix_acp::errors::RpcErrorCode;
use reasonix_acp::inbox::{
    ENQUEUE_SOURCE, EnqueueReceipt, InboxIntent, InboxRefusal, SessionApiRefusal, prepare_enqueue,
    refuse,
};
use reasonix_acp::wire::SessionInboxEnqueueParams;

#[test]
fn named_refusals_map_to_32020_except_retry_may_repeat_to_32021() {
    let refused = [
        InboxRefusal::SteerAlreadyApplied,
        InboxRefusal::ItemTooLarge,
        InboxRefusal::CapacityItems,
        InboxRefusal::CapacityBytes,
        InboxRefusal::InvalidState,
        InboxRefusal::ForeignRun,
        InboxRefusal::Paused,
        InboxRefusal::Unrecovered,
        InboxRefusal::NotFound,
        InboxRefusal::IdempotencyConflict,
        InboxRefusal::SchemaReadonly,
        InboxRefusal::Closed,
    ];
    for refusal in refused {
        assert_eq!(refusal.rpc_code(), RpcErrorCode::InboxRefused);
        let err = refuse(refusal, "kernel message");
        assert_eq!(err.code, RpcErrorCode::InboxRefused);
        assert_eq!(
            err.data.expect("data present")["code"],
            refusal.data_code().unwrap()
        );
    }

    assert_eq!(
        InboxRefusal::RetryMayRepeat.rpc_code(),
        RpcErrorCode::InboxMayHaveRun
    );
    let retry = refuse(InboxRefusal::RetryMayRepeat, "may already have run");
    assert_eq!(retry.code, RpcErrorCode::InboxMayHaveRun);
    assert_eq!(
        retry.data.expect("data present")["code"],
        "inbox.retry_may_repeat"
    );

    assert_eq!(InboxRefusal::Empty.rpc_code(), RpcErrorCode::InvalidParams);
    assert_eq!(InboxRefusal::Empty.data_code(), Some("inbox.empty"));

    assert_eq!(
        InboxRefusal::UnknownFault.rpc_code(),
        RpcErrorCode::Internal
    );
    assert_eq!(InboxRefusal::UnknownFault.data_code(), None);
    let fault = refuse(InboxRefusal::UnknownFault, "queue boom");
    assert!(fault.data.is_none());
}

#[test]
fn intent_defaults_to_followup_and_only_steer_matches_case_insensitively() {
    assert_eq!(InboxIntent::parse(None), InboxIntent::Followup);
    assert_eq!(InboxIntent::parse(Some("")), InboxIntent::Followup);
    assert_eq!(InboxIntent::parse(Some("STEER")), InboxIntent::Steer);
    assert_eq!(InboxIntent::parse(Some("steer")), InboxIntent::Steer);
    assert_eq!(InboxIntent::parse(Some("followup")), InboxIntent::Followup);
}

#[test]
fn prepare_enqueue_trims_text_stamps_acp_and_keeps_idempotency_key() {
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
fn empty_text_refuses_with_inbox_empty_and_vendor_method_prefix() {
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
fn receipt_round_trips_the_five_wire_fields() {
    let receipt = EnqueueReceipt {
        item_id: "item-9".to_owned(),
        disposition: "steer_accepted".to_owned(),
        position: 2,
        paused: false,
        idempotent: false,
    };
    let result = reasonix_acp::wire::SessionInboxEnqueueResult::from(receipt);
    assert_eq!(result.item_id, "item-9");
    assert_eq!(result.disposition, "steer_accepted");
    assert_eq!(result.position, 2);
    assert!(!result.paused);
    assert!(!result.idempotent);
}

#[test]
fn session_api_refusal_distinguishes_missing_session_from_missing_inbox_api() {
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
