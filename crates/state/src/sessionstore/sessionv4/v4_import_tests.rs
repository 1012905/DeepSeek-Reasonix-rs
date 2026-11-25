use std::fs;
use std::path::Path;

use reasonix_contract::provider::{Message, Role};
use serde_json::json;
use tempfile::TempDir;

use super::*;
use crate::sessionstore::branch::{BranchMeta, load_branch_meta};
use crate::sessionstore::session::{Session, digest_session_messages, load_session_unlocked};
use crate::sessionstore::sessionv4::test_support::{End, Fixture, ID, message};
use crate::store::naming::{
    is_v4_session_id, session_cleanup_pending, session_display_index, session_event_index,
    session_meta, session_v4_removed_marker,
};

fn native(path: &Path, contents: &[&str]) {
    let session = Session::new("");
    session.replace(
        contents
            .iter()
            .map(|content| Message {
                role: Role::USER,
                content: (*content).to_string(),
                ..Message::default()
            })
            .collect(),
    );
    session
        .save_if_absent(&path.to_string_lossy())
        .expect("save native copy");
}

fn loaded(path: &Path) -> Vec<Message> {
    load_session_unlocked(path)
        .expect("load native copy")
        .snapshot()
}

fn source_bytes(fixture: &Fixture) -> (Vec<u8>, Vec<u8>) {
    (
        fs::read(fixture.dir.join("manifest.json")).expect("manifest"),
        fs::read(fixture.dir.join("events.frames")).expect("frames"),
    )
}

fn pending(path: &Path, operation: &str) {
    fs::write(
        session_cleanup_pending(path),
        json!({"operation": operation, "sessionPath": path, "markerPath": path}).to_string(),
    )
    .expect("pending marker");
}

#[test]
fn ids_and_paths_accept_only_native_import_shapes() {
    for id in [
        "0123456789abcdef",
        &"a".repeat(64),
        "desktop-0123456789abcdef",
        "desktop-manual-0123456789abcdef",
    ] {
        assert!(is_v4_session_id(id), "{id}");
    }
    for id in [
        "",
        "0123456789abcde",
        &"a".repeat(65),
        "0123456789ABCDEF",
        "0123456789abcdeg",
        "desktop-manual-desktop-0123456789abcdef",
        "../0123456789abcdef",
    ] {
        assert!(!is_v4_session_id(id), "{id}");
    }
    let temp = TempDir::new().expect("tempdir");
    let desktop_id = "desktop-manual-0123456789abcdef";
    let fixture = Fixture::new(desktop_id);
    let sessions = fixture.sessions();
    let path = v4_import_path(&sessions, desktop_id);
    assert_eq!(
        path,
        sessions.join("v4-desktop-manual-0123456789abcdef.jsonl")
    );
    assert_eq!(
        v4_session_for(&path)
            .expect("source")
            .expect("v4 path")
            .manifest
            .session_id,
        desktop_id
    );
    assert!(
        v4_session_for(&sessions.join("v4-../0123456789abcdef.jsonl"))
            .expect("probe")
            .is_none()
    );
    assert!(
        v4_session_for(&temp.path().join("other/v4-0123456789abcdef.jsonl"))
            .expect("probe")
            .is_none()
    );
}

#[test]
fn first_import_preserves_source_title_archive_pin_and_identity() {
    let mut fixture = Fixture::new(ID);
    fixture.batch(
        vec![
            message("m1", "user", "hello"),
            json!({"kind":"session/title","payload":{"title":" Source title "}}),
            json!({"kind":"session/config","payload":{"modelRef":" source-model "}}),
        ],
        End::Ended,
    );
    let before = source_bytes(&fixture);
    let source = crate::sessionstore::sessionv4::open(&fixture.dir).expect("source");
    let sessions = fixture.sessions();
    fs::create_dir_all(&sessions).expect("sessions");
    let path = v4_import_path(&sessions, ID);
    let start = BranchMeta {
        archived: true,
        pinned: true,
        ..BranchMeta::default()
    };
    let before_import = CreatedAt::now();
    import_v4_as(&path, &source, start).expect("import");
    let after_import = CreatedAt::now();
    assert_eq!(source_bytes(&fixture), before);
    assert_eq!(loaded(&path)[0].content, "hello");
    let meta = load_branch_meta(&path).expect("meta").expect("present");
    assert_eq!(meta.custom_title, "Source title");
    assert!(meta.archived && meta.pinned);
    assert_eq!(meta.model, "source-model");
    assert_eq!(meta.turns, 1);
    assert_eq!(meta.preview, "hello");
    assert_eq!(
        meta.schema_version,
        crate::sessionstore::branch::BRANCH_META_COUNTS_VERSION
    );
    assert_eq!(
        meta.created_at,
        crate::sessionstore::eventlog::CreatedAt::from_datetime(source.manifest.created_at)
    );
    assert!(meta.updated_at.get() >= before_import.get());
    assert!(meta.updated_at.get() <= after_import.get());
    let digest = digest_session_messages(&source.transcript().expect("transcript").messages)
        .expect("digest");
    let imported = meta.imported_from.expect("import identity");
    assert_eq!(imported.line, IMPORT_LINE);
    assert_eq!(imported.log_size, source.log_size);
    assert_eq!(
        imported.digest,
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
}

#[test]
fn prepare_refreshes_only_an_untouched_copy() {
    let mut fixture = Fixture::new(ID);
    fixture.batch(vec![message("m1", "user", "one")], End::Ended);
    let sessions = fixture.sessions();
    fs::create_dir_all(&sessions).expect("sessions");
    let path = v4_import_path(&sessions, ID);
    prepare_session_path(&path).expect("first open");
    fixture.batch(vec![message("m2", "user", "two")], End::Ended);
    prepare_session_path(&path).expect("refresh");
    assert_eq!(loaded(&path).len(), 2);
    let mut local = loaded(&path);
    local.push(Message {
        role: Role::USER,
        content: "local".into(),
        ..Message::default()
    });
    let session = load_session_unlocked(&path).expect("load");
    session.replace(local);
    session
        .save_rewrite(&path.to_string_lossy())
        .expect("continue copy");
    fixture.batch(vec![message("m3", "user", "three")], End::Ended);
    prepare_session_path(&path).expect("continued open");
    let contents = loaded(&path);
    assert_eq!(contents.len(), 3);
    assert_eq!(contents[2].content, "local");
}

#[test]
fn interrupted_import_completes_a_strict_prefix_without_losing_a_superset() {
    let mut fixture = Fixture::new(ID);
    fixture.batch(vec![message("m1", "user", "one")], End::Ended);
    fixture.batch(vec![message("m2", "user", "two")], End::Ended);
    let source = crate::sessionstore::sessionv4::open(&fixture.dir).expect("source");
    let sessions = fixture.sessions();
    fs::create_dir_all(&sessions).expect("sessions");
    let prefix = v4_import_path(&sessions, ID);
    native(&prefix, &["one"]);
    assert!(complete_v4_import(&prefix, &source, BranchMeta::default()).expect("complete"));
    assert_eq!(loaded(&prefix).len(), 2);
    assert!(
        load_branch_meta(&prefix)
            .expect("meta")
            .expect("present")
            .imported_from
            .is_some()
    );

    let mut superset_fixture = Fixture::new("1123456789abcdef0123456789abcdef");
    superset_fixture.batch(vec![message("m1", "user", "one")], End::Ended);
    let superset_source =
        crate::sessionstore::sessionv4::open(&superset_fixture.dir).expect("source");
    let superset = v4_import_path(
        &superset_fixture.sessions(),
        superset_source.manifest.session_id.as_str(),
    );
    fs::create_dir_all(superset.parent().expect("parent")).expect("sessions");
    native(&superset, &["one", "local"]);
    let before = fs::read(&superset).expect("superset bytes");
    assert!(
        complete_v4_import(&superset, &superset_source, BranchMeta::default())
            .expect("complete sidecar without rewriting superset")
    );
    assert_eq!(fs::read(&superset).expect("unchanged superset"), before);
    assert_eq!(loaded(&superset).len(), 2);
    assert_eq!(loaded(&superset)[1].content, "local");
    let meta = load_branch_meta(&superset).expect("meta").expect("present");
    assert_eq!(meta.turns, 2);
}

#[test]
fn interrupted_equal_length_divergence_stays_untouched_and_empty_source_is_omitted() {
    let mut fixture = Fixture::new(ID);
    fixture.batch(vec![message("m1", "user", "source")], End::Ended);
    let source = crate::sessionstore::sessionv4::open(&fixture.dir).expect("source");
    let path = v4_import_path(&fixture.sessions(), ID);
    fs::create_dir_all(path.parent().expect("parent")).expect("sessions");
    native(&path, &["different"]);
    assert!(complete_v4_import(&path, &source, BranchMeta::default()).expect("sidecar"));
    assert_eq!(loaded(&path)[0].content, "different");

    let empty = Fixture::new("1123456789abcdef0123456789abcdef");
    let empty_source = crate::sessionstore::sessionv4::open(&empty.dir).expect("source");
    let empty_path = v4_import_path(&empty.sessions(), empty_source.manifest.session_id.as_str());
    fs::create_dir_all(empty_path.parent().expect("parent")).expect("sessions");
    native(&empty_path, &["existing"]);
    let sidecar = session_meta(&empty_path);
    let before = fs::read(&sidecar).expect("existing sidecar");
    assert!(!complete_v4_import(&empty_path, &empty_source, BranchMeta::default()).expect("empty"));
    assert_eq!(fs::read(&sidecar).expect("unchanged sidecar"), before);
    assert!(
        load_branch_meta(&empty_path)
            .expect("meta")
            .expect("existing sidecar")
            .imported_from
            .is_none()
    );
    let report = import_v4_from(&empty.root, &empty.sessions()).expect("bulk");
    assert_eq!(report.imported, 0);
    assert!(report.skipped.is_empty());
}

#[test]
fn bulk_import_removes_orphan_indexes_and_reports_typed_skips() {
    let mut fixture = Fixture::new(ID);
    fixture.batch(vec![message("m1", "user", "hello")], End::Ended);
    let sessions = fixture.sessions();
    fs::create_dir_all(&sessions).expect("sessions");
    let path = v4_import_path(&sessions, ID);
    fs::write(session_event_index(&path), b"stale").expect("index");
    fs::write(session_display_index(&path), b"stale").expect("index");
    assert!(!v4_copy_begun(&path));
    let report = import_v4_from(&fixture.root, &sessions).expect("bulk");
    assert_eq!(report.imported, 1);
    assert!(report.skipped.is_empty());
    let event_index = fs::read(session_event_index(&path)).expect("rebuilt event index");
    assert_ne!(event_index, b"stale");
    serde_json::from_slice::<serde_json::Value>(&event_index).expect("valid event index");
    if let Ok(display_index) = fs::read(session_display_index(&path)) {
        assert_ne!(display_index, b"stale");
        serde_json::from_slice::<serde_json::Value>(&display_index).expect("valid display index");
    }
    assert!(v4_copy_begun(&path));

    let invalid = Fixture::new("1123456789ABCDEF0123456789abcdef");
    let unsupported = Fixture::new("2123456789abcdef0123456789abcdef");
    let mut raw: serde_json::Value =
        serde_json::from_slice(&fs::read(unsupported.dir.join("manifest.json")).expect("manifest"))
            .expect("json");
    raw["storageRevision"] = json!(9);
    fs::write(unsupported.dir.join("manifest.json"), raw.to_string()).expect("manifest");
    let corrupt = Fixture::new("3123456789abcdef0123456789abcdef");
    fs::write(corrupt.dir.join("events.frames"), b"brokenframe!").expect("frames");
    let source_root = fixture.root.join("combined");
    for source_dir in [&invalid.dir, &unsupported.dir, &corrupt.dir] {
        let name = source_dir.file_name().expect("source name");
        copy_dir(source_dir, &source_root.join(name));
    }
    let report = import_v4_from(&source_root, &sessions).expect("bulk skips");
    assert_eq!(report.imported, 0);
    assert_eq!(report.skipped.len(), 3);
    for reason in [
        SkipReason::UnreadableFormat,
        SkipReason::SchemaUnsupported,
        SkipReason::Corrupt,
    ] {
        assert_eq!(
            report
                .skipped
                .iter()
                .filter(|skip| skip.reason == reason)
                .count(),
            1,
            "{reason:?}"
        );
    }
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("dir");
    for entry in fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy");
        }
    }
}

#[test]
fn bulk_routing_distinguishes_unknown_fallback_and_explicit_projects() {
    let mut fixture = Fixture::new(ID);
    fixture.batch(vec![message("m1", "user", "hello")], End::Ended);
    fs::write(
        fixture.dir.join("header.json"),
        json!({"cwd":" /project "}).to_string(),
    )
    .expect("header");
    let sessions = fixture.sessions();
    fs::create_dir_all(&sessions).expect("sessions");
    let report = import_v4_from_routed(
        &fixture.root,
        &sessions,
        |_| V4Placement::default(),
        |_| BranchMeta::default(),
    )
    .expect("route");
    assert_eq!(report.imported, 0);
    assert_eq!(report.skipped[0].reason, SkipReason::NoProject);
    let explicit = fixture.temp.path().join("project/sessions");
    let report = import_v4_from_routed(
        &fixture.root,
        &sessions,
        |origin| {
            assert_eq!(origin.cwd, "/project");
            V4Placement {
                dir: explicit.clone(),
                known: true,
            }
        },
        |_| BranchMeta {
            archived: true,
            pinned: true,
            ..BranchMeta::default()
        },
    )
    .expect("route");
    assert_eq!(report.imported, 1);
    let meta = load_branch_meta(&v4_import_path(&explicit, ID))
        .expect("meta")
        .expect("present");
    assert!(meta.archived && meta.pinned);
    assert!(v4_import_path(&sessions, ID).symlink_metadata().is_err());
}

#[test]
fn removal_operations_publish_verbatim_and_failures_keep_pending() {
    let temp = TempDir::new().expect("tempdir");
    let sessions = temp.path().join("sessions");
    fs::create_dir_all(&sessions).expect("sessions");
    fs::create_dir_all(temp.path().join("sessions-v4")).expect("v4 root");
    let path = v4_import_path(&sessions, ID);
    let marker = session_v4_removed_marker(&path);
    for operation in ["remove", "delete", "clear"] {
        pending(&path, operation);
        preserve_v4_removal(&path).expect("preserve");
        assert_eq!(
            fs::read(&marker).expect("marker"),
            fs::read(session_cleanup_pending(&path)).expect("pending")
        );
        assert!(is_v4_session_removed(&path));
        fs::remove_file(&marker).expect("remove marker");
    }
    pending(&path, "fork");
    preserve_v4_removal(&path).expect("fork");
    assert!(marker.symlink_metadata().is_err());
    fs::remove_file(session_cleanup_pending(&path)).expect("pending");
    pending(&path, "delete");
    fs::create_dir(&marker).expect("blocking directory");
    assert!(preserve_v4_removal(&path).is_err());
    assert!(session_cleanup_pending(&path).is_file());
}

#[cfg(unix)]
#[test]
fn removal_marker_reads_refuse_external_symlinks() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().expect("tempdir");
    let sessions = temp.path().join("sessions");
    fs::create_dir_all(&sessions).expect("sessions");
    fs::create_dir_all(temp.path().join("sessions-v4")).expect("v4 root");
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).expect("outside");
    let path = v4_import_path(&sessions, ID);
    symlink(&outside, session_v4_removed_marker(&path)).expect("symlink");
    assert!(!is_v4_session_removed(&path));
}

#[cfg(windows)]
#[test]
fn removal_marker_reads_refuse_external_junctions() {
    let temp = TempDir::new().expect("tempdir");
    let sessions = temp.path().join("sessions");
    fs::create_dir_all(&sessions).expect("sessions");
    fs::create_dir_all(temp.path().join("sessions-v4")).expect("v4 root");
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).expect("outside");
    let path = v4_import_path(&sessions, ID);
    let status = std::process::Command::new("cmd")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(session_v4_removed_marker(&path))
        .arg(&outside)
        .status()
        .expect("create junction");
    assert!(status.success());
    assert!(!is_v4_session_removed(&path));
}
