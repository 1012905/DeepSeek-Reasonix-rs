use std::collections::BTreeMap;

use chrono::DateTime;
use reasonix_contract::{gojson::marshal, provider::Via};
use serde_json::{Value, json, value::RawValue};

use super::*;
use crate::sessionstore::eventlog::record::CreatedAt;

fn raw(value: &str) -> Option<Box<RawValue>> {
    Some(serde_json::from_str(value).expect("valid raw JSON"))
}

fn at(value: &str) -> CreatedAt {
    CreatedAt::from_datetime(DateTime::parse_from_rfc3339(value).expect("RFC3339 timestamp"))
}

#[test]
fn schema_defaults_and_no_item_limit_match_go() {
    assert_eq!(SCHEMA_VERSION, 3);
    assert_eq!(DEFAULT_MAX_ITEMS, 64);
    assert_eq!(DEFAULT_MAX_ITEM_BYTES, 4 << 20);
    assert_eq!(DEFAULT_MAX_TOTAL_BYTES, 64 << 20);
    assert_eq!(DEFAULT_PREVIEW_RUNES, 120);
    assert_eq!(NO_ITEM_LIMIT, -1);
    assert_eq!(
        Limits::default().with_defaults(),
        Limits {
            max_items: DEFAULT_MAX_ITEMS,
            max_item_bytes: DEFAULT_MAX_ITEM_BYTES,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
        }
    );
    assert_eq!(
        Limits {
            max_items: -2,
            max_item_bytes: -1,
            max_total_bytes: 0,
        }
        .with_defaults(),
        Limits {
            max_items: -2,
            max_item_bytes: DEFAULT_MAX_ITEM_BYTES,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
        }
    );
    assert_eq!(
        Limits {
            max_items: NO_ITEM_LIMIT,
            max_item_bytes: 1,
            max_total_bytes: 2,
        }
        .with_defaults(),
        Limits {
            max_items: NO_ITEM_LIMIT,
            max_item_bytes: 1,
            max_total_bytes: 2,
        }
    );
}

#[test]
fn enum_wire_values_and_zero_origins_match_go() {
    assert_eq!(serde_json::to_value(INTENT_FOLLOWUP).unwrap(), "followup");
    assert_eq!(serde_json::to_value(INTENT_STEER).unwrap(), "steer");
    assert_eq!(serde_json::to_value(ORIGIN_USER).unwrap(), "");
    assert_eq!(serde_json::to_value(ORIGIN_HOST).unwrap(), "host");
    assert_eq!(PromptOrigin::default(), ORIGIN_USER);

    let states = [
        (STATE_QUEUED, "queued"),
        (STATE_STEER_ACCEPTED, "steer_accepted"),
        (STATE_STEER_CONSUMED, "steer_consumed"),
        (STATE_RUNNING, "running"),
        (STATE_BLOCKED, "blocked"),
        (STATE_UNCERTAIN, "uncertain"),
    ];
    for (state, wire) in states {
        assert_eq!(serde_json::to_value(state).unwrap(), wire);
    }

    let dispositions = [
        (DISPOSITION_STARTED, "started"),
        (DISPOSITION_STEER_ACCEPTED, "steer_accepted"),
        (DISPOSITION_QUEUED_FOLLOWUP, "queued_followup"),
        (DISPOSITION_REJECTED_BUSY, "rejected_busy"),
        (DISPOSITION_REJECTED_ROTATING, "rejected_rotating"),
        (DISPOSITION_REJECTED_CLOSED, "rejected_closed"),
        (DISPOSITION_REJECTED_CAPACITY, "rejected_capacity"),
        (DISPOSITION_IDEMPOTENT_HIT, "idempotent_hit"),
        (DISPOSITION_DELETED, "deleted"),
        (DISPOSITION_CANCELLED, "cancelled"),
    ];
    for (disposition, wire) in dispositions {
        assert_eq!(serde_json::to_value(disposition).unwrap(), wire);
    }

    let blocks = [
        (BLOCK_STEER_UNAPPLIED, "steer_unapplied"),
        (BLOCK_SNAPSHOT_FAILED, "turn_snapshot_failed"),
        (BLOCK_ACK_FAILED, "turn_ack_failed"),
        (BLOCK_OWNER_INACTIVE, "owner_inactive"),
        (BLOCK_MANIFEST_SALVAGED, "manifest_salvaged"),
        (BLOCK_FOREIGN_IMAGES, "foreign_images"),
    ];
    for (code, wire) in blocks {
        assert_eq!(serde_json::to_value(code).unwrap(), wire);
    }

    let pauses = [
        (PAUSE_USER, "user"),
        (PAUSE_SESSION_CHANGED, "session_changed"),
        (PAUSE_REFERENCE_BLOCKED, "reference_blocked"),
        (PAUSE_WRITE_AUTHORITY, "write_authority"),
        (PAUSE_TURN_RECORD, "turn_record"),
        (PAUSE_RECOVERED, "recovered"),
    ];
    for (code, wire) in pauses {
        assert_eq!(serde_json::to_value(code).unwrap(), wire);
    }
}

#[test]
fn unknown_enum_strings_decode_and_reencode_without_loss() {
    let intent: InboxIntent = serde_json::from_str(r#""future_intent""#).unwrap();
    let origin: PromptOrigin = serde_json::from_str(r#""future_origin""#).unwrap();
    let state: InboxState = serde_json::from_str(r#""future_state""#).unwrap();
    let disposition: Disposition = serde_json::from_str(r#""future_disposition""#).unwrap();
    let block: BlockCode = serde_json::from_str(r#""future_block""#).unwrap();
    let pause: PauseCode = serde_json::from_str(r#""future_pause""#).unwrap();

    assert_eq!(intent.as_str(), "future_intent");
    assert_eq!(origin.as_str(), "future_origin");
    assert_eq!(state.as_str(), "future_state");
    assert_eq!(disposition.as_str(), "future_disposition");
    assert_eq!(block.as_str(), "future_block");
    assert_eq!(pause.as_str(), "future_pause");
    assert!(!valid_inbox_intent(&intent));
    assert!(!valid_inbox_state(&state));
    assert!(!origin.is_host());
    assert!(!block.retry_safe());
    assert!(!pause.guards_stale_input());

    assert_eq!(marshal(&intent).unwrap(), r#""future_intent""#);
    assert_eq!(marshal(&origin).unwrap(), r#""future_origin""#);
    assert_eq!(marshal(&state).unwrap(), r#""future_state""#);
    assert_eq!(marshal(&disposition).unwrap(), r#""future_disposition""#);
    assert_eq!(marshal(&block).unwrap(), r#""future_block""#);
    assert_eq!(marshal(&pause).unwrap(), r#""future_pause""#);
}

#[test]
fn enum_validation_is_separate_from_decode() {
    assert!(valid_inbox_intent(&INTENT_FOLLOWUP));
    assert!(valid_inbox_intent(&INTENT_STEER));
    assert!(!valid_inbox_intent(&InboxIntent::default()));
    assert!(!valid_inbox_intent(&InboxIntent::new("future")));
    for state in [
        STATE_QUEUED,
        STATE_STEER_ACCEPTED,
        STATE_STEER_CONSUMED,
        STATE_RUNNING,
        STATE_BLOCKED,
        STATE_UNCERTAIN,
    ] {
        assert!(valid_inbox_state(&state));
    }
    assert!(!valid_inbox_state(&InboxState::default()));
    assert!(!valid_inbox_state(&InboxState::new("future")));
    assert!(serde_json::from_str::<InboxIntent>("7").is_err());
}

#[test]
fn all_thirteen_sentinel_errors_keep_their_display_identities() {
    let errors = [
        (Error::CapacityItems, "session inbox item limit reached"),
        (Error::CapacityBytes, "session inbox byte limit reached"),
        (
            Error::ItemTooLarge,
            "inbox item exceeds single-item size limit",
        ),
        (Error::NotFound, "inbox item not found"),
        (
            Error::InvalidState,
            "inbox item state does not allow this operation",
        ),
        (
            Error::RetryMayRepeat,
            "inbox item may already have run, so retrying it could repeat it",
        ),
        (
            Error::SchemaReadonly,
            "inbox schema is newer and is read-only",
        ),
        (Error::Closed, "inbox is closed"),
        (Error::Empty, "inbox item body is empty"),
        (Error::Paused, "inbox is paused"),
        (Error::ForeignRun, "sessioninbox.foreign_run"),
        (
            Error::IdempotencyConflict,
            "idempotency key was already used for different input",
        ),
        (Error::Unrecovered, "sessioninbox.queue_unreadable"),
    ];
    for (error, expected) in &errors {
        assert_eq!(error.to_string(), *expected);
    }
}

#[test]
fn origin_pause_and_block_predicates_are_exact() {
    assert!(!ORIGIN_USER.is_host());
    assert!(ORIGIN_HOST.is_host());
    assert!(!PromptOrigin::new("HOST").is_host());

    assert!(PAUSE_SESSION_CHANGED.guards_stale_input());
    assert!(PAUSE_REFERENCE_BLOCKED.guards_stale_input());
    assert!(PAUSE_RECOVERED.guards_stale_input());
    for code in [
        PAUSE_USER,
        PAUSE_WRITE_AUTHORITY,
        PAUSE_TURN_RECORD,
        PauseCode::default(),
        PauseCode::new("future"),
    ] {
        assert!(!code.guards_stale_input());
    }

    assert!(BLOCK_STEER_UNAPPLIED.retry_safe());
    for code in [
        BLOCK_SNAPSHOT_FAILED,
        BLOCK_ACK_FAILED,
        BLOCK_OWNER_INACTIVE,
        BLOCK_MANIFEST_SALVAGED,
        BLOCK_FOREIGN_IMAGES,
        BlockCode::default(),
        BlockCode::new("future"),
    ] {
        assert!(!code.retry_safe());
    }
}

#[test]
fn empty_envelope_omits_optional_fields_and_keeps_required_zero_fields() {
    let envelope: PromptEnvelope = serde_json::from_str("{}").unwrap();
    assert_eq!(envelope.display_text, "");
    assert_eq!(envelope.raw_text, "");
    assert_eq!(envelope.submit_text, "");
    assert_eq!(envelope.invocation, None);
    assert_eq!(envelope.via, None);
    assert!(envelope.image_inputs.is_none());
    assert!(envelope.refs.is_empty());
    assert!(envelope.extra.is_empty());
    assert_eq!(
        marshal(&envelope).unwrap(),
        r#"{"displayText":"","rawText":"","submitText":""}"#
    );
}

#[test]
fn explicit_null_raw_image_input_remains_null_while_other_nulls_normalize() {
    let envelope: PromptEnvelope = serde_json::from_str(
        r#"{"displayText":null,"rawText":null,"submitText":null,"refs":null,"frozenImages":null,"imageInputs":null,"imageSourceRefs":null,"extra":null,"via":null}"#,
    )
    .unwrap();
    assert_eq!(envelope.image_inputs.as_deref().unwrap().get(), "null");
    assert!(envelope.refs.is_empty());
    assert!(envelope.frozen_images.is_empty());
    assert!(envelope.image_source_refs.is_empty());
    assert!(envelope.extra.is_empty());
    assert_eq!(envelope.via, None);
    assert_eq!(
        marshal(&envelope).unwrap(),
        r#"{"displayText":"","rawText":"","submitText":"","imageInputs":null}"#
    );
}

#[test]
fn arbitrary_image_input_json_survives_decode_and_encode() {
    let raw_json = r#"{"z":[1e+02,null],"a":"<"}"#;
    let input =
        format!(r#"{{"displayText":"d","rawText":"r","submitText":"s","imageInputs":{raw_json}}}"#);
    let envelope: PromptEnvelope = serde_json::from_str(&input).unwrap();
    assert_eq!(envelope.image_inputs.as_deref().unwrap().get(), raw_json);
    let encoded = marshal(&envelope).unwrap();
    assert!(encoded.contains(r#""imageInputs":{"z":[1e+02,null],"a":"\u003c"}"#));
    let decoded: PromptEnvelope = serde_json::from_str(&encoded).unwrap();
    assert_eq!(
        decoded.image_inputs.as_deref().unwrap().get(),
        r#"{"z":[1e+02,null],"a":"\u003c"}"#
    );
}

#[test]
fn raw_image_input_serialization_compacts_go_whitespace_without_reordering() {
    let raw_json = r#"{ "z" : [ 1e+02, null ], "a" : "a b" }"#;
    let input =
        format!(r#"{{"displayText":"d","rawText":"r","submitText":"s","imageInputs":{raw_json}}}"#);
    let envelope: PromptEnvelope = serde_json::from_str(&input).unwrap();
    assert_eq!(envelope.image_inputs.as_deref().unwrap().get(), raw_json);
    let encoded = marshal(&envelope).unwrap();
    assert!(
        encoded.contains(r#""imageInputs":{"z":[1e+02,null],"a":"a b"}"#),
        "{encoded}"
    );
}

#[test]
fn null_slice_elements_and_map_values_match_go_zero_values() {
    let envelope: PromptEnvelope = serde_json::from_str(
        r#"{"refs":[null],"attachments":[null,"a"],"invocations":[null],"extra":{"x":null},"imageSourceRefs":{"y":null}}"#,
    )
    .unwrap();
    assert_eq!(envelope.refs, vec![RefSnapshot::default()]);
    assert_eq!(envelope.attachments, vec![String::new(), "a".to_owned()]);
    assert_eq!(envelope.invocations, vec![StructuredInvocation::default()]);
    assert_eq!(envelope.extra.get("x").map(String::as_str), Some(""));
    assert_eq!(
        envelope.image_source_refs.get("y").map(String::as_str),
        Some("")
    );
}

#[test]
fn envelope_bytes_are_compact_go_json_with_sorted_maps_and_html_escaping() {
    let envelope = PromptEnvelope {
        fingerprint_version: 1,
        request_fingerprint: "fp".to_owned(),
        display_text: "<>&\u{2028}\u{2029}".to_owned(),
        raw_text: "raw".to_owned(),
        submit_text: "submit".to_owned(),
        invocation: Some(StructuredInvocation {
            name: "name".to_owned(),
            kind: "kind".to_owned(),
            offset: 2,
            args: BTreeMap::from([
                ("z".to_owned(), "2".to_owned()),
                ("a".to_owned(), "1".to_owned()),
            ]),
            display: "display".to_owned(),
        }),
        invocations: vec![StructuredInvocation {
            name: "multi".to_owned(),
            ..StructuredInvocation::default()
        }],
        format: "markdown".to_owned(),
        attachments: vec!["attach".to_owned()],
        attachment_identities: vec!["identity".to_owned()],
        refs: vec![RefSnapshot {
            kind: "attachment".to_owned(),
            path: "path".to_owned(),
            display_path: "displayPath".to_owned(),
            repo_identity: "repo".to_owned(),
            commit: "commit".to_owned(),
            range_start: 1,
            range_end: 2,
            content: vec![0, 1, 2],
            content_sha: "contentSha".to_owned(),
            truncated: true,
            server: "server".to_owned(),
            uri: "uri".to_owned(),
        }],
        frozen_ref_block: "refs".to_owned(),
        frozen_images: vec!["data:image/png;base64,AA==".to_owned()],
        image_inputs: raw(r#"{"z":[1e+02,null],"a":"<"}"#),
        image_source_refs: BTreeMap::from([
            ("z".to_owned(), "src".to_owned()),
            ("a".to_owned(), "ref".to_owned()),
        ]),
        reference_errors: vec!["error".to_owned()],
        explicit_refs: vec!["@path".to_owned()],
        idempotency: "key".to_owned(),
        source: "source".to_owned(),
        extra: BTreeMap::from([
            ("z".to_owned(), "last".to_owned()),
            ("a".to_owned(), "first".to_owned()),
        ]),
        via: Some(Via {
            device: "device".to_owned(),
            ordinal: 2,
        }),
    };
    assert_eq!(
        marshal(&envelope).unwrap(),
        concat!(
            r#"{"fingerprintVersion":1,"requestFingerprint":"fp","displayText":"\u003c\u003e\u0026\u2028\u2029","rawText":"raw","submitText":"submit","#,
            r#""invocation":{"name":"name","kind":"kind","offset":2,"args":{"a":"1","z":"2"},"display":"display"},"#,
            r#""invocations":[{"name":"multi"}],"format":"markdown","attachments":["attach"],"attachmentIdentities":["identity"],"#,
            r#""refs":[{"kind":"attachment","path":"path","displayPath":"displayPath","repoIdentity":"repo","commit":"commit","rangeStart":1,"rangeEnd":2,"content":"AAEC","contentSha":"contentSha","truncated":true,"server":"server","uri":"uri"}],"#,
            r#""frozenRefBlock":"refs","frozenImages":["data:image/png;base64,AA=="],"imageInputs":{"z":[1e+02,null],"a":"\u003c"},"#,
            r#""imageSourceRefs":{"a":"ref","z":"src"},"referenceErrors":["error"],"explicitRefs":["@path"],"idempotencyKey":"key","source":"source","extra":{"a":"first","z":"last"},"via":{"device":"device","ordinal":2}}"#,
        )
    );
}

#[test]
fn item_meta_wire_preserves_names_omission_and_reference_bytes() {
    let item = InboxItemMeta {
        id: "item".to_owned(),
        session_id: "session".to_owned(),
        intent: INTENT_STEER,
        state: STATE_BLOCKED,
        revision: 4,
        blob_name: "blob".to_owned(),
        source: "source".to_owned(),
        created_at: at("2026-01-02T03:04:05.123400Z"),
        updated_at: at("2026-01-02T03:04:06Z"),
        preview: "preview".to_owned(),
        byte_size: 12,
        checksum: "a".repeat(64),
        idempotency: "key".to_owned(),
        origin: ORIGIN_HOST,
        refs: vec![RefSummary {
            kind: "frozen".to_owned(),
            path: "path".to_owned(),
            commit: "commit".to_owned(),
            bytes: 3,
            preview: "ref".to_owned(),
        }],
        images: true,
        block_reason: "reason".to_owned(),
        block_code: BLOCK_FOREIGN_IMAGES,
        run_id: "run".to_owned(),
    };
    let mut expected = concat!(
        r#"{"id":"item","sessionId":"session","intent":"steer","state":"blocked","revision":4,"blobName":"blob","source":"source","#,
        r#""createdAt":"2026-01-02T03:04:05.1234Z","updatedAt":"2026-01-02T03:04:06Z","preview":"preview","byteSize":12,"checksum":""#,
    )
    .to_owned();
    expected.push_str(&"a".repeat(64));
    expected.push_str(
        r#"","idempotencyKey":"key","origin":"host","refs":[{"kind":"frozen","path":"path","commit":"commit","bytes":3,"preview":"ref"}],"images":true,"blockReason":"reason","blockCode":"foreign_images","runId":"run"}"#,
    );
    assert_eq!(marshal(&item).unwrap(), expected);
}

#[test]
fn missing_and_null_times_decode_to_go_zero_time() {
    for input in ["{}", r#"{"createdAt":null,"updatedAt":null}"#] {
        let item: InboxItemMeta = serde_json::from_str(input).unwrap();
        assert_eq!(item.created_at, CreatedAt::zero());
        assert_eq!(item.updated_at, CreatedAt::zero());
        let value = serde_json::to_value(item).unwrap();
        assert_eq!(value["createdAt"], "0001-01-01T00:00:00Z");
        assert_eq!(value["updatedAt"], "0001-01-01T00:00:00Z");
    }
}

#[test]
fn snapshot_distinguishes_nil_items_from_an_empty_array() {
    let missing: InboxSnapshot = serde_json::from_str("{}").unwrap();
    let null: InboxSnapshot = serde_json::from_str(r#"{"items":null}"#).unwrap();
    let empty: InboxSnapshot = serde_json::from_str(r#"{"items":[]}"#).unwrap();
    assert_eq!(missing.items, None);
    assert_eq!(null.items, None);
    assert_eq!(empty.items, Some(Vec::new()));
    assert_eq!(serde_json::to_value(missing).unwrap()["items"], Value::Null);
    assert_eq!(serde_json::to_value(empty).unwrap()["items"], json!([]));
}

#[test]
fn receipt_snapshot_and_input_types_keep_go_wire_names() {
    let capacity = Capacity {
        items: 2,
        max_items: 64,
        bytes: 12,
        max_bytes: 64 << 20,
        max_item_bytes: 4 << 20,
    };
    let receipt = InboxReceipt {
        item_id: "item".to_owned(),
        disposition: DISPOSITION_IDEMPOTENT_HIT,
        position: 1,
        paused: false,
        capacity,
        idempotent: false,
    };
    assert_eq!(
        marshal(&receipt).unwrap(),
        r#"{"itemId":"item","disposition":"idempotent_hit","position":1,"paused":false,"capacity":{"items":2,"maxItems":64,"bytes":12,"maxBytes":67108864,"maxItemBytes":4194304}}"#
    );

    let snapshot = InboxSnapshot {
        schema_version: SCHEMA_VERSION,
        revision: 3,
        paused: true,
        pause_code: PauseCode::new("future_pause"),
        recovered: true,
        recovered_n: 2,
        unrecovered: true,
        readonly: true,
        run_id: "run".to_owned(),
        items: None,
        capacity,
    };
    assert_eq!(
        marshal(&snapshot).unwrap(),
        r#"{"schemaVersion":3,"revision":3,"paused":true,"pauseCode":"future_pause","recovered":true,"recoveredCount":2,"unrecovered":true,"readonly":true,"runId":"run","items":null,"capacity":{"items":2,"maxItems":64,"bytes":12,"maxBytes":67108864,"maxItemBytes":4194304}}"#
    );

    let request = EnqueueRequest {
        intent: INTENT_FOLLOWUP,
        origin: ORIGIN_USER,
        envelope: PromptEnvelope::default(),
        source: "source".to_owned(),
        idempotency: "key".to_owned(),
        session_id: "session".to_owned(),
    };
    assert_eq!(
        marshal(&request).unwrap(),
        r#"{"Intent":"followup","Origin":"","Envelope":{"displayText":"","rawText":"","submitText":""},"Source":"source","Idempotency":"key","SessionID":"session"}"#
    );
    assert_eq!(
        marshal(&Limits::default()).unwrap(),
        r#"{"MaxItems":0,"MaxItemBytes":0,"MaxTotalBytes":0}"#
    );
}

#[test]
fn image_detection_matches_raw_message_and_frozen_image_rules() {
    let cases = [
        (None, false),
        (Some("null"), false),
        (Some("[]"), false),
        (Some("[null]"), true),
        (Some("[{}]"), true),
        (Some("{}"), true),
        (Some(r#""image""#), true),
        (Some("0"), true),
        (Some("false"), true),
    ];
    for (input, expected) in cases {
        let envelope = PromptEnvelope {
            image_inputs: input.and_then(raw),
            ..PromptEnvelope::default()
        };
        assert_eq!(envelope.has_foreign_images(), expected, "{input:?}");
        assert_eq!(envelope.carries_images(), expected, "{input:?}");
    }
    let frozen = PromptEnvelope {
        frozen_images: vec!["data:image/png;base64,AA==".to_owned()],
        ..PromptEnvelope::default()
    };
    assert!(frozen.carries_images());
    assert!(!frozen.has_foreign_images());
}

#[test]
fn ref_snapshot_content_uses_go_base64_byte_wire_shape() {
    let input = r#"{"kind":"attachment","content":"AAEC","truncated":false}"#;
    let snapshot: RefSnapshot = serde_json::from_str(input).unwrap();
    assert_eq!(snapshot.content, vec![0, 1, 2]);
    assert_eq!(
        marshal(&snapshot).unwrap(),
        r#"{"kind":"attachment","content":"AAEC"}"#
    );
    let null: RefSnapshot = serde_json::from_str(r#"{"kind":"","content":null}"#).unwrap();
    assert!(null.content.is_empty());
    assert_eq!(marshal(&null).unwrap(), r#"{"kind":""}"#);
}

#[test]
fn ref_snapshot_content_accepts_go_byte_array_and_newline_base64_forms() {
    let array: RefSnapshot = serde_json::from_str(r#"{"content":[0,null,255]}"#).unwrap();
    assert_eq!(array.content, vec![0, 0, 255]);
    let base64: RefSnapshot = serde_json::from_str(r#"{"content":"AAE\r\nC"}"#).unwrap();
    assert_eq!(base64.content, vec![0, 1, 2]);
    assert_eq!(marshal(&base64).unwrap(), r#"{"kind":"","content":"AAEC"}"#);
}
