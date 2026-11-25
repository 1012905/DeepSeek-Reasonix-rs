use serde_json::{Value, json};

use super::*;

#[test]
fn file_revision_omits_empty_optional_fields_and_keeps_required_identity() {
    let revision = FileRevision {
        path: "a.txt".into(),
        existed: false,
        ..FileRevision::default()
    };
    assert_eq!(
        serde_json::to_value(revision).unwrap(),
        json!({"path": "a.txt", "existed": false})
    );
}

#[test]
fn checkpoint_schema_defaults_match_the_contract() {
    let config = StoreConfig::default();
    assert_eq!(config.retain, 100);
    assert_eq!(config.blob_quota_bytes, 1 << 30);
    assert_eq!(config.max_file_bytes, 32 << 20);
    assert_eq!(SCHEMA_V2, 2);
    assert_eq!(
        serde_json::to_value(Coverage::Complete).unwrap(),
        "complete"
    );
    assert_eq!(
        serde_json::to_value(CoverageGap::default()).unwrap()["reason"],
        ""
    );
}

#[test]
fn checkpoint_wire_distinguishes_unspecified_and_explicit_none_coverage() {
    let unspecified = serde_json::to_value(Checkpoint::default()).unwrap();
    assert!(unspecified.get("coverage").is_none());
    assert!(unspecified.get("schemaVersion").is_none());
    assert!(unspecified.get("sessionId").is_none());

    let explicit = Checkpoint {
        schema_version: SCHEMA_V2,
        coverage: Coverage::None,
        ..Checkpoint::default()
    };
    assert_eq!(serde_json::to_value(explicit).unwrap()["coverage"], "none");
}

#[test]
fn rewind_plan_wire_matches_go_omitempty_fields() {
    let value = serde_json::to_value(RewindPlan::default()).unwrap();
    for field in [
        "coverageGaps",
        "legacy",
        "expiredFilePayload",
        "disabledReason",
        "conflicts",
        "files",
        "activeWriters",
        "workspaceToken",
        "boundaryIndex",
        "path",
        "conflictResolution",
    ] {
        assert!(value.get(field).is_none(), "unexpected field {field}");
    }
    for field in [
        "planId",
        "turn",
        "scope",
        "coverage",
        "canFiles",
        "canConversation",
        "fileCount",
        "sessionRevision",
        "hasBoundary",
        "createdAt",
    ] {
        assert!(value.get(field).is_some(), "missing field {field}");
    }
    assert_eq!(value["coverage"], "");
}

#[test]
fn rewind_result_wire_matches_go_omitempty_fields() {
    let value = serde_json::to_value(RewindResult::default()).unwrap();
    assert_eq!(value, json!({"ok": false, "undoAvailable": false}));
}

#[test]
fn transaction_manifest_wire_matches_go_omitempty_fields() {
    let value = serde_json::to_value(TransactionManifest::default()).unwrap();
    for field in ["coverage", "boundaryIndex", "truncateFrom"] {
        assert!(value.get(field).is_none(), "unexpected field {field}");
    }
    assert_eq!(value["hasBoundary"], false);
    assert_eq!(value["scope"], 0);
}

#[test]
fn file_snap_wire_keeps_content_null_and_omits_v2_zero_fields() {
    let value: Value = serde_json::to_value(FileSnap {
        path: "created.txt".into(),
        ..FileSnap::default()
    })
    .unwrap();
    assert_eq!(value, json!({"path": "created.txt", "content": null}));
}
