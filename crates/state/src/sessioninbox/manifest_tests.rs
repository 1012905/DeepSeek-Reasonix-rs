use chrono::Duration;
use reasonix_contract::gojson::marshal;

use super::*;
use crate::sessionstore::eventlog::record::CreatedAt;

fn hash() -> String {
    "a".repeat(64)
}

fn at(value: &str) -> CreatedAt {
    CreatedAt::from_datetime(
        chrono::DateTime::parse_from_rfc3339(value).expect("RFC3339 timestamp"),
    )
}

fn item(id: &str) -> InboxItemMeta {
    InboxItemMeta {
        id: id.to_owned(),
        intent: INTENT_FOLLOWUP,
        state: STATE_QUEUED,
        ..InboxItemMeta::default()
    }
}

fn manifest() -> Manifest {
    Manifest {
        schema_version: SCHEMA_VERSION,
        items: vec![item("item")],
        ..Manifest::default()
    }
}

fn receipt(completed_at: CreatedAt) -> IdempotencyReceipt {
    IdempotencyReceipt {
        item_id: "item".to_owned(),
        request_hash: hash(),
        disposition: DISPOSITION_STARTED,
        completed_at,
    }
}

#[test]
fn manifest_constants_match_go() {
    assert_eq!(MAX_IDEMPOTENCY_RECEIPTS, 512);
    assert_eq!(IDEMPOTENCY_RECEIPT_TTL, Duration::hours(7 * 24));
    assert_eq!(MAX_IDEMPOTENCY_KEY_BYTES, 1024);
}

#[test]
fn empty_manifest_initializes_schema_run_id_and_collections() {
    let manifest = empty_manifest("run");
    assert_eq!(manifest.schema_version, SCHEMA_VERSION);
    assert_eq!(manifest.run_id, "run");
    assert!(manifest.items.is_empty());
    assert!(manifest.idempotency.is_empty());
    assert!(manifest.idempotency_hashes.is_empty());
    assert!(manifest.receipts.is_empty());
    assert!(!manifest.updated_at.is_zero());
}

#[test]
fn decode_manifest_normalizes_missing_and_null_collections() {
    for input in [
        r#"{"schemaVersion":3}"#,
        r#"{"schemaVersion":3,"items":null,"idempotency":null,"idempotencyHashes":null,"receipts":null}"#,
    ] {
        let manifest = decode_manifest(input.as_bytes()).unwrap();
        assert!(manifest.items.is_empty());
        assert!(manifest.idempotency.is_empty());
        assert!(manifest.idempotency_hashes.is_empty());
        assert!(manifest.receipts.is_empty());
        assert_eq!(
            marshal(&manifest).unwrap(),
            r#"{"schemaVersion":3,"revision":0,"paused":false,"recovered":false,"items":[],"updatedAt":"0001-01-01T00:00:00Z"}"#
        );
    }
    let null = decode_manifest(b"null").unwrap();
    assert_eq!(null.schema_version, 0);
    assert!(null.items.is_empty());
    assert!(validate_manifest(&null, false).is_ok());
}

#[test]
fn decode_manifest_rejects_malformed_json_and_non_objects() {
    assert!(matches!(decode_manifest(b"{"), Err(Error::Json(_))));
    assert!(matches!(decode_manifest(b"[]"), Err(Error::Json(_))));
}

#[test]
fn manifest_wire_uses_go_names_and_sorted_maps() {
    let mut manifest = manifest();
    manifest.run_id = "run".to_owned();
    manifest.paused = true;
    manifest.pause_code = PAUSE_USER;
    manifest.recovered = true;
    manifest.recovered_n = 2;
    manifest.updated_at = at("2026-01-02T03:04:05.1200Z");
    manifest.idempotency = [
        ("z".to_owned(), "item".to_owned()),
        ("a".to_owned(), "item".to_owned()),
    ]
    .into_iter()
    .collect();
    manifest.idempotency_hashes = [("z".to_owned(), hash()), ("a".to_owned(), hash())]
        .into_iter()
        .collect();
    manifest.receipts.insert(
        "a".to_owned(),
        IdempotencyReceipt {
            item_id: "item".to_owned(),
            request_hash: hash(),
            disposition: DISPOSITION_DELETED,
            completed_at: at("2026-01-02T03:04:05Z"),
        },
    );
    let encoded = marshal(&manifest).unwrap();
    assert!(encoded.contains(r#""idempotency":{"a":""#));
    assert!(
        encoded.find(r#""idempotency":{"a""#).unwrap() < encoded.find(r#""z":"item""#).unwrap()
    );
    assert!(encoded.contains(r#""pauseCode":"user""#));
    assert!(encoded.contains(r#""recoveredCount":2"#), "{encoded}");
    assert!(encoded.contains(r#""updatedAt":"2026-01-02T03:04:05.12Z""#));
    let decoded = decode_manifest(encoded.as_bytes()).unwrap();
    assert_eq!(decoded, manifest);
}

#[test]
fn idempotency_receipt_wire_keeps_unknown_dispositions() {
    let receipt = IdempotencyReceipt {
        item_id: "item".to_owned(),
        request_hash: hash(),
        disposition: Disposition::new("future_disposition"),
        completed_at: at("2026-01-02T03:04:05Z"),
    };
    let mut expected = r#"{"itemId":"item","requestHash":""#.to_owned();
    expected.push_str(&hash());
    expected
        .push_str(r#"","disposition":"future_disposition","completedAt":"2026-01-02T03:04:05Z"}"#);
    assert_eq!(marshal(&receipt).unwrap(), expected);
}

#[test]
fn only_negative_schema_or_revision_is_the_broad_version_error() {
    for (schema_version, revision) in [(-1, 0), (0, -1)] {
        let mut manifest = manifest();
        manifest.schema_version = schema_version;
        manifest.revision = revision;
        assert!(matches!(
            validate_manifest(&manifest, false),
            Err(Error::InvalidVersionOrRevision)
        ));
        assert!(matches!(
            validate_manifest(&manifest, true),
            Err(Error::InvalidVersionOrRevision)
        ));
    }
    for (schema_version, revision) in [(0, 0), (1, 0), (2, 0), (3, 0), (99, 0)] {
        let mut manifest = manifest();
        manifest.schema_version = schema_version;
        manifest.revision = revision;
        assert!(validate_manifest(&manifest, false).is_ok());
    }
}

#[test]
fn newer_schema_decode_preserves_unknown_enums_and_skips_receipt_validation() {
    let mut manifest = manifest();
    manifest.schema_version = SCHEMA_VERSION + 1;
    manifest.items[0].intent = InboxIntent::new("future_intent");
    manifest.items[0].state = InboxState::new("future_state");
    manifest
        .idempotency
        .insert(" bad ".to_owned(), "missing".to_owned());
    manifest
        .idempotency_hashes
        .insert(" bad ".to_owned(), "bad".to_owned());
    manifest.receipts.insert(
        "".to_owned(),
        IdempotencyReceipt {
            item_id: "../bad".to_owned(),
            request_hash: "bad".to_owned(),
            disposition: Disposition::new("future_disposition"),
            completed_at: CreatedAt::zero(),
        },
    );
    let decoded = decode_manifest(&marshal(&manifest).unwrap().into_bytes()).unwrap();
    assert_eq!(decoded.items[0].intent.as_str(), "future_intent");
    assert_eq!(decoded.items[0].state.as_str(), "future_state");
    assert_eq!(
        decoded.idempotency.get(" bad ").map(String::as_str),
        Some("missing")
    );
    assert_eq!(decoded.receipts.len(), 1);
    assert!(validate_manifest(&decoded, true).is_ok());
    assert!(matches!(
        validate_manifest(&decoded, false),
        Err(Error::InvalidItemIntentOrState)
    ));
}

#[test]
fn unknown_enum_tolerance_still_enforces_structural_item_checks() {
    let mut bad_path = manifest();
    bad_path.items[0].id = "../bad".to_owned();
    let mut bad_blob = manifest();
    bad_blob.items[0].blob_name = "../bad".to_owned();
    let mut duplicate = manifest();
    duplicate.items.push(item("item"));
    let mut negative = manifest();
    negative.items[0].byte_size = -1;
    let mut checksum = manifest();
    checksum.items[0].checksum = "A".repeat(64);
    let cases = [
        (bad_path, Error::InvalidItemPath),
        (bad_blob, Error::InvalidItemPath),
        (duplicate, Error::DuplicateItemId),
        (negative, Error::NegativeItemSize),
        (checksum, Error::InvalidItemChecksum),
    ];
    for (manifest, expected) in &cases {
        assert!(matches!(
            validate_manifest(manifest, true),
            Err(error) if std::mem::discriminant(&error) == std::mem::discriminant(expected)
        ));
    }
}

#[test]
fn schema_two_requires_hashes_but_zero_and_one_allow_backfill() {
    let mut manifest = manifest();
    manifest
        .idempotency
        .insert("key".to_owned(), "item".to_owned());
    for schema_version in [0, 1] {
        manifest.schema_version = schema_version;
        assert!(validate_manifest(&manifest, false).is_ok());
    }
    manifest.schema_version = 2;
    assert!(matches!(
        validate_manifest(&manifest, false),
        Err(Error::MissingIdempotencyRequestHash)
    ));
    manifest.idempotency_hashes.insert("key".to_owned(), hash());
    assert!(validate_manifest(&manifest, false).is_ok());
    manifest
        .idempotency_hashes
        .insert("key".to_owned(), "bad".to_owned());
    assert!(matches!(
        validate_manifest(&manifest, false),
        Err(Error::InvalidIdempotencyRequestHash)
    ));
}

#[test]
fn item_blob_name_uses_trimmed_nonempty_value_then_id() {
    let mut item = item("item");
    assert_eq!(blob_name_for(&item), "item");
    item.blob_name = "  ".to_owned();
    assert_eq!(blob_name_for(&item), "item");
    item.blob_name = "  blob  ".to_owned();
    assert_eq!(blob_name_for(&item), "blob");
    let mut manifest = manifest();
    manifest.items[0].blob_name = "  blob  ".to_owned();
    assert!(validate_manifest(&manifest, false).is_ok());
    item.blob_name = " blob ".to_owned();
    assert!(!valid_blob_stem(&item.blob_name));
    assert!(valid_blob_stem(blob_name_for(&item)));
}

#[test]
fn validation_rejects_duplicate_ids_negative_sizes_and_unknown_enums() {
    let mut duplicate = manifest();
    duplicate.items.push(item("item"));
    assert!(matches!(
        validate_manifest(&duplicate, false),
        Err(Error::DuplicateItemId)
    ));
    let mut negative = manifest();
    negative.items[0].byte_size = -1;
    assert!(matches!(
        validate_manifest(&negative, false),
        Err(Error::NegativeItemSize)
    ));
    let mut intent = manifest();
    intent.items[0].intent = InboxIntent::new("future");
    assert!(matches!(
        validate_manifest(&intent, false),
        Err(Error::InvalidItemIntentOrState)
    ));
    let mut state = manifest();
    state.items[0].state = InboxState::new("future");
    assert!(matches!(
        validate_manifest(&state, false),
        Err(Error::InvalidItemIntentOrState)
    ));
}

#[test]
fn checksum_validation_is_optional_but_nonempty_values_are_strict() {
    let mut manifest = manifest();
    assert!(validate_manifest(&manifest, false).is_ok());
    manifest.items[0].checksum = hash();
    assert!(validate_manifest(&manifest, false).is_ok());
    for checksum in [
        "A".repeat(64),
        "g".repeat(64),
        "a".repeat(63),
        "a".repeat(65),
        "é".repeat(64),
    ] {
        manifest.items[0].checksum = checksum;
        assert!(matches!(
            validate_manifest(&manifest, false),
            Err(Error::InvalidItemChecksum)
        ));
    }
}

#[test]
fn valid_blob_stem_enforces_byte_length_trim_and_separators() {
    for valid in ["a", ".hidden", "...", "name.", &"é".repeat(100)] {
        assert!(valid_blob_stem(valid), "{valid:?}");
    }
    for invalid in [
        "",
        ".",
        "..",
        " a",
        "a ",
        "a\u{00a0}",
        "a/b",
        "a\\b",
        "a\0b",
        &"é".repeat(101),
    ] {
        assert!(!valid_blob_stem(invalid), "{invalid:?}");
    }
}

#[cfg(windows)]
#[test]
fn valid_blob_stem_matches_go_windows_local_and_device_rules() {
    for invalid in [
        "CON",
        "con",
        "PRN",
        "AUX",
        "NUL",
        "COM1",
        "LPT9",
        "COM¹",
        "COM²",
        "COM³",
        "CONIN$",
        "CONOUT$",
        "CON.",
        "CON..",
        "NUL...",
        "PRN...",
        "COM1.",
        "CONIN$.",
        "CON .",
        "C:foo",
        "1:foo",
        "foo:bar",
        "\\\\?\\C:\\foo",
        "\\\\.\\pipe\\x",
    ] {
        assert!(!valid_blob_stem(invalid), "{invalid:?}");
    }
    for valid in [
        "CON.txt",
        "NUL.tar.gz",
        "COM1.txt",
        "CON.txt.",
        "CON.txt..",
        "CON..txt",
        "CON .txt",
        "COM10",
        "LPT0",
        "CONSOLE",
        "CLOCK$",
        "...",
        "name.",
    ] {
        assert!(valid_blob_stem(valid), "{valid:?}");
    }
}

#[cfg(not(windows))]
#[test]
fn valid_blob_stem_keeps_unix_local_name_behavior() {
    assert!(valid_blob_stem("CON"));
    assert!(valid_blob_stem("NUL..."));
    assert!(valid_blob_stem("C:foo"));
    assert!(valid_blob_stem("COM1.txt"));
}

#[test]
fn valid_idempotency_key_enforces_trim_bytes_and_nul_only() {
    for valid in [
        "key",
        "a/b\\c:d",
        "CON",
        &"x".repeat(1024),
        &"é".repeat(512),
    ] {
        assert!(valid_idempotency_key(valid), "{valid:?}");
    }
    for invalid in [
        "",
        " key",
        "key ",
        "\u{2003}",
        "a\0b",
        &"x".repeat(1025),
        &"é".repeat(513),
    ] {
        assert!(!valid_idempotency_key(invalid), "{invalid:?}");
    }
}

#[test]
fn valid_sha256_accepts_only_lowercase_hex_digests() {
    assert!(valid_sha256(&hash()));
    assert!(valid_sha256(&"0123456789abcdef".repeat(4)));
    for invalid in [
        "",
        &"a".repeat(63),
        &"a".repeat(65),
        &"A".repeat(64),
        &"g".repeat(64),
        &"é".repeat(64),
    ] {
        assert!(!valid_sha256(invalid), "{invalid:?}");
    }
}

#[test]
fn idempotency_keys_must_reference_live_items_and_valid_hashes() {
    let mut manifest = manifest();
    manifest
        .idempotency
        .insert(" bad ".to_owned(), "item".to_owned());
    assert!(matches!(
        validate_manifest(&manifest, false),
        Err(Error::InvalidIdempotencyKey)
    ));
    manifest.idempotency.clear();
    manifest
        .idempotency
        .insert("key".to_owned(), "missing".to_owned());
    assert!(matches!(
        validate_manifest(&manifest, false),
        Err(Error::IdempotencyMissingItem)
    ));
}

#[test]
fn receipt_validation_checks_key_item_and_hash_but_not_disposition() {
    let mut manifest = manifest();
    let mut value = receipt(at("2026-01-02T03:04:05Z"));
    value.disposition = Disposition::new("future_disposition");
    value.item_id = "not-live".to_owned();
    manifest.receipts.insert("key".to_owned(), value.clone());
    assert!(validate_manifest(&manifest, false).is_ok());

    manifest.receipts.insert(" bad ".to_owned(), value.clone());
    assert!(matches!(
        validate_manifest(&manifest, false),
        Err(Error::InvalidIdempotencyReceipt)
    ));
    manifest.receipts.clear();
    value.item_id = "../bad".to_owned();
    manifest.receipts.insert("key".to_owned(), value.clone());
    assert!(matches!(
        validate_manifest(&manifest, false),
        Err(Error::InvalidIdempotencyReceipt)
    ));
    value.item_id = "item".to_owned();
    value.request_hash.clear();
    manifest.receipts.insert("key".to_owned(), value);
    assert!(matches!(
        validate_manifest(&manifest, false),
        Err(Error::InvalidIdempotencyReceipt)
    ));
}

#[test]
fn all_twelve_manifest_validation_errors_keep_go_display_text() {
    let errors = [
        (Error::NilManifest, "nil manifest"),
        (
            Error::InvalidVersionOrRevision,
            "invalid manifest version or revision",
        ),
        (Error::InvalidItemPath, "invalid inbox item path"),
        (Error::DuplicateItemId, "duplicate inbox item id"),
        (Error::NegativeItemSize, "negative inbox item size"),
        (
            Error::InvalidItemIntentOrState,
            "invalid inbox item intent or state",
        ),
        (Error::InvalidItemChecksum, "invalid inbox item checksum"),
        (Error::InvalidIdempotencyKey, "invalid idempotency key"),
        (
            Error::IdempotencyMissingItem,
            "idempotency key references missing item",
        ),
        (
            Error::MissingIdempotencyRequestHash,
            "missing idempotency request hash",
        ),
        (
            Error::InvalidIdempotencyRequestHash,
            "invalid idempotency request hash",
        ),
        (
            Error::InvalidIdempotencyReceipt,
            "invalid idempotency receipt",
        ),
    ];
    for (error, expected) in &errors {
        assert_eq!(error.to_string(), *expected);
    }
}

#[test]
fn clone_is_safe_and_independent_for_items_maps_and_receipts() {
    let mut original = manifest();
    original.items[0].refs.push(RefSummary {
        kind: "frozen".to_owned(),
        ..RefSummary::default()
    });
    original
        .idempotency
        .insert("key".to_owned(), "item".to_owned());
    original.idempotency_hashes.insert("key".to_owned(), hash());
    original
        .receipts
        .insert("key".to_owned(), receipt(at("2026-01-02T03:04:05Z")));
    let mut cloned = original.clone();
    cloned.items[0].refs[0].path = "changed".to_owned();
    cloned
        .idempotency
        .insert("other".to_owned(), "item".to_owned());
    cloned.idempotency_hashes.clear();
    cloned.receipts.clear();
    assert_eq!(original.items[0].refs[0].path, "");
    assert!(!original.idempotency.contains_key("other"));
    assert_eq!(original.idempotency_hashes.len(), 1);
    assert_eq!(original.receipts.len(), 1);
}

#[test]
fn total_bytes_uses_go_signed_wrapping_addition() {
    let mut manifest = manifest();
    manifest.items[0].byte_size = i64::MAX;
    manifest.items.push(InboxItemMeta {
        byte_size: 1,
        ..item("second")
    });
    assert_eq!(manifest.total_bytes(), i64::MIN);
}

#[test]
fn item_lookup_and_removal_clean_all_idempotency_aliases() {
    let mut manifest = manifest();
    manifest.items.push(item("second"));
    manifest.idempotency = [
        ("b".to_owned(), "item".to_owned()),
        ("a".to_owned(), "item".to_owned()),
        ("other".to_owned(), "second".to_owned()),
    ]
    .into_iter()
    .collect();
    manifest.idempotency_hashes = [
        ("b".to_owned(), hash()),
        ("a".to_owned(), hash()),
        ("other".to_owned(), hash()),
    ]
    .into_iter()
    .collect();
    manifest
        .receipts
        .insert("a".to_owned(), receipt(at("2026-01-02T03:04:05Z")));
    assert_eq!(manifest.index_of("item"), 0);
    assert_eq!(manifest.index_of("second"), 1);
    assert_eq!(manifest.index_of("missing"), -1);
    assert_eq!(manifest.item("item").unwrap().id, "item");
    assert!(manifest.item("missing").is_none());
    assert_eq!(manifest.idempotency_keys_for("item"), vec!["a", "b"]);
    assert_eq!(
        manifest.idempotency_keys_for("missing"),
        Vec::<String>::new()
    );

    let removed = manifest.remove_item("item").unwrap();
    assert_eq!(removed.id, "item");
    assert_eq!(manifest.items.len(), 1);
    assert_eq!(manifest.items[0].id, "second");
    assert!(!manifest.idempotency.contains_key("a"));
    assert!(!manifest.idempotency.contains_key("b"));
    assert!(!manifest.idempotency_hashes.contains_key("a"));
    assert!(!manifest.idempotency_hashes.contains_key("b"));
    assert!(manifest.idempotency.contains_key("other"));
    assert!(manifest.idempotency_hashes.contains_key("other"));
    assert!(manifest.receipts.contains_key("a"));
    assert!(manifest.remove_item("missing").is_none());
}

#[test]
fn remember_receipt_skips_blank_keys_and_captures_request_hash() {
    let now = at("2026-01-02T03:04:05Z");
    let mut manifest = manifest();
    manifest.idempotency_hashes.insert("key".to_owned(), hash());
    let keys = ["".to_owned(), " \t".to_owned(), "key".to_owned()];
    manifest.remember_receipt(&keys, "item", Disposition::new("future_disposition"), now);
    assert_eq!(manifest.receipts.len(), 1);
    let receipt = &manifest.receipts["key"];
    assert_eq!(receipt.item_id, "item");
    assert_eq!(receipt.request_hash, hash());
    assert_eq!(receipt.disposition.as_str(), "future_disposition");
    assert_eq!(receipt.completed_at, now);
}

#[test]
fn receipt_pruning_removes_zero_expired_and_keeps_ttl_boundary() {
    let now = at("2026-01-02T03:04:05Z");
    let mut manifest = manifest();
    manifest
        .receipts
        .insert("zero".to_owned(), receipt(CreatedAt::zero()));
    manifest.receipts.insert(
        "expired".to_owned(),
        receipt(CreatedAt::from_datetime(
            now.get() - Duration::hours(7 * 24) - Duration::seconds(1),
        )),
    );
    manifest.receipts.insert(
        "boundary".to_owned(),
        receipt(CreatedAt::from_datetime(
            now.get() - Duration::hours(7 * 24),
        )),
    );
    manifest.receipts.insert("fresh".to_owned(), receipt(now));
    manifest.receipts.insert(
        "future".to_owned(),
        receipt(CreatedAt::from_datetime(now.get() + Duration::seconds(1))),
    );
    manifest.prune_receipts(now);
    assert_eq!(manifest.receipts.len(), 3);
    assert!(manifest.receipts.contains_key("boundary"));
    assert!(manifest.receipts.contains_key("fresh"));
    assert!(manifest.receipts.contains_key("future"));
}

#[test]
fn receipt_pruning_evicts_oldest_until_count_is_at_cap() {
    let now = at("2026-01-02T03:04:05Z");
    let mut manifest = manifest();
    for index in 0..513 {
        manifest.receipts.insert(
            format!("k{index:04}"),
            receipt(CreatedAt::from_datetime(
                now.get() + Duration::seconds(i64::from(index)),
            )),
        );
    }
    manifest.prune_receipts(now);
    assert_eq!(manifest.receipts.len(), MAX_IDEMPOTENCY_RECEIPTS);
    assert!(!manifest.receipts.contains_key("k0000"));
    assert!(manifest.receipts.contains_key("k0512"));
}

#[test]
fn receipt_pruning_uses_lexical_tie_break_deterministically() {
    let now = at("2026-01-02T03:04:05Z");
    let mut manifest = manifest();
    for index in 0..513 {
        manifest
            .receipts
            .insert(format!("k{index:04}"), receipt(now));
    }
    manifest.prune_receipts(now);
    assert_eq!(manifest.receipts.len(), MAX_IDEMPOTENCY_RECEIPTS);
    assert!(!manifest.receipts.contains_key("k0000"));
    assert!(manifest.receipts.contains_key("k0001"));
    assert!(manifest.receipts.contains_key("k0512"));
}
