use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::sessionstore::eventlog::record::CreatedAt;

use super::*;

fn fixture() -> (TempDir, PathBuf, PathBuf, Store) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let dir = temp.path().join("source.ckpt");
    fs::create_dir_all(&root).unwrap();
    let store = Store::new(Some(&dir), &root);
    (temp, root, dir, store)
}

fn load_fixture(checkpoints: &[Checkpoint]) -> (TempDir, PathBuf, PathBuf, Store) {
    let (temp, root, dir, _) = fixture();
    for checkpoint in checkpoints {
        write_json_atomic(&turn_path(&dir, checkpoint.turn, false), checkpoint).unwrap();
    }
    let store = Store::new(Some(&dir), &root);
    (temp, root, dir, store)
}

fn checkpoint(turn: i64, msg_index: i64) -> Checkpoint {
    Checkpoint {
        schema_version: SCHEMA_V2,
        turn,
        time: CreatedAt::now(),
        prompt: format!("prompt-{turn}"),
        msg_index,
        session_id: "source".to_owned(),
        coverage: Coverage::None,
        ..Checkpoint::default()
    }
}

fn fixed_time(value: &str) -> CreatedAt {
    serde_json::from_value::<Checkpoint>(json!({"turn": 0, "time": value}))
        .unwrap()
        .time
}

fn write(root: &Path, path: &str, content: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn read_turn(dir: &Path, turn: i64) -> Value {
    serde_json::from_slice(&fs::read(turn_path(dir, turn, false)).unwrap()).unwrap()
}

#[test]
fn rejects_unavailable_sources_and_empty_destinations() {
    let (_temp, root, _dir, store) = fixture();
    let destination = root.join("child.ckpt");

    let error = store
        .copy_conversation_prefix_to(Path::new(""), "child", 1)
        .unwrap_err();
    assert!(matches!(&error, ConversationForkError::Unavailable));
    assert_eq!(
        error.to_string(),
        "conversation checkpoint destination unavailable"
    );

    let memory = Store::new(None, &root);
    let error = memory
        .copy_conversation_prefix_to(&destination, "child", 1)
        .unwrap_err();
    assert!(matches!(&error, ConversationForkError::Unavailable));
    assert!(!destination.exists());
}

#[test]
fn rejects_lexically_identical_source_and_destination() {
    let (_temp, _root, dir, store) = fixture();
    store.begin(0, "source", 0).unwrap();
    let original = fs::read(turn_path(&dir, 0, false)).unwrap();
    let forward_slash = dir.to_string_lossy().replace('\\', "/");
    let destinations = [
        dir.clone(),
        dir.join("."),
        dir.join("missing").join(".."),
        PathBuf::from(format!("{forward_slash}/./missing//../")),
    ];
    assert!(!dir.join("missing").exists());

    for destination in destinations {
        let error = store
            .copy_conversation_prefix_to(&destination, "child", 1)
            .unwrap_err();
        assert!(matches!(&error, ConversationForkError::SourceIsDestination));
        assert_eq!(
            error.to_string(),
            "conversation checkpoint destination is the source"
        );
        assert_eq!(fs::read(turn_path(&dir, 0, false)).unwrap(), original);
    }
}

#[test]
#[cfg(windows)]
fn source_destination_equality_uses_exact_cleaned_units() {
    let source = PathBuf::from(r"C:\Case\Source");
    let destination = PathBuf::from(r"c:\case\source");
    let store = Store::persistence_only(source);

    store
        .copy_conversation_prefix_to(&destination, "child", 0)
        .expect("case-distinct cleaned strings are not the same source");
}

#[test]
#[cfg(windows)]
fn lexical_clean_matches_go_volume_and_device_semantics() {
    use std::os::windows::ffi::OsStrExt;

    let cases = [
        (r"C:", r"C:."),
        (r"C:foo\..", r"C:."),
        (r"C:foo\..\bar", r"C:bar"),
        (r"C:.\", r"C:."),
        (r"C:\..\abc", r"C:\abc"),
        (r"C:\b:\..\..\..\d", r"C:\d"),
        (r"C:..\foo\..", r"C:.."),
        (r"abc\def\..", r"abc"),
        (r"abc\def\..\..", r"."),
        (r"\abc\def\..\..", r"\"),
        (r"a\b\c\..\..\..\..", r".."),
        (r"a\..\c:", r".\c:"),
        (r"a\..\c:\foo", r".\c:\foo"),
        (r"a\..\..\c:", r"..\c:"),
        (r".\c:", r".\c:"),
        (r"foo:bar", r"foo:bar"),
        (r"foo:/bar", r".\foo:\bar"),
        (r"é:", r"é:"),
        (r"é:\foo", r"é:\foo"),
        (r"a\..\é:", r".\é:"),
        (r"\a\..\??\c:\x", r"\.\??\c:\x"),
        (r"/a/../??/a", r"\.\??\a"),
        (r"\??\c:\x", r"\??\c:\x"),
        (r"\\i\..\c$", r"\c$"),
        (r"\\..\..\a", r"\a"),
        (r"//../../a", r"\a"),
        (r"\\host\share\foo\..", r"\\host\share\"),
        (r"\\a\b\..\c", r"\\a\b\c"),
        (r"//host/share/../x", r"\\host\share\x"),
        (r"//abc", r"\\abc"),
        (r"//abc//", r"\\abc\\"),
        (r"///abc", r"\\\abc"),
        (r"\\.\C:\\a", r"\\.\C:\a"),
        (r"\\.\C:\a\..\..\..\..\bar", r"\\.\C:\bar"),
        (r"\\?\C:\foo\..", r"\\?\C:\"),
        (r"\\?\C:\a", r"\\?\C:\a"),
        (r"\\?\C:", r"\\?\C:"),
        (r"\\?\UNC\host\share\foo\..", r"\\?\UNC\host\share\"),
        (
            r"\\?\UNC\host\share\foo\..\..\..\..\bar",
            r"\\?\UNC\host\share\bar",
        ),
        (
            r"\??\UNC\host\share\foo\..\..\..\..\bar",
            r"\??\UNC\host\share\bar",
        ),
    ];

    let mut mismatches = Vec::new();
    for (input, expected) in cases {
        let actual = super::path_clean::lexical_clean(Path::new(input));
        if actual.as_os_str() != Path::new(expected).as_os_str() {
            mismatches.push(format!(
                "{input:?}: got {actual:?} ({} units), expected {expected:?}",
                actual.as_os_str().encode_wide().count()
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn filters_message_prefix_inclusively_and_excludes_negative_indices() {
    let (_temp, root, _dir, store) = fixture();
    store.set_session_id("source");
    store.begin(0, "negative", -3).unwrap();
    store.begin(1, "lower", 0).unwrap();
    store.begin(2, "middle", 3).unwrap();
    store.begin(3, "upper", 5).unwrap();
    store.begin(4, "current", 4).unwrap();

    let destination = root.join("child.ckpt");
    store
        .copy_conversation_prefix_to(&destination, "child", 5)
        .unwrap();

    let child = Store::new(Some(&destination), &root);
    let copied = child.list();
    assert_eq!(
        copied.iter().map(|meta| meta.turn).collect::<Vec<_>>(),
        vec![1, 2, 4]
    );
    assert_eq!(
        copied.iter().map(|meta| meta.msg_index).collect::<Vec<_>>(),
        vec![0, 3, 4]
    );
    assert_eq!(child.next_turn(), 5);
    assert!(!turn_path(&destination, 0, false).exists());
    assert!(!turn_path(&destination, 3, false).exists());
}

#[test]
fn zero_and_negative_message_counts_copy_nothing() {
    let (_temp, root, dir, store) = fixture();
    store.begin(0, "current", 0).unwrap();
    let original = fs::read(turn_path(&dir, 0, false)).unwrap();

    for message_count in [0, -1, i64::MIN] {
        let destination = root.join(format!("child-{message_count}.ckpt"));
        store
            .copy_conversation_prefix_to(&destination, "child", message_count)
            .unwrap();
        assert!(!destination.exists());
        assert_eq!(fs::read(turn_path(&dir, 0, false)).unwrap(), original);
    }
}

#[test]
fn copies_only_conversation_fields_and_marks_fork() {
    let original = Checkpoint {
        schema_version: SCHEMA_V1,
        turn: 7,
        time: fixed_time("2026-02-03T04:05:06.123456789Z"),
        prompt: "keep this prompt".to_owned(),
        msg_index: 4,
        session_id: "source".to_owned(),
        files: vec![FileSnap {
            path: "a.txt".to_owned(),
            content: Some("payload".to_owned()),
            ..FileSnap::default()
        }],
        coverage: Coverage::Complete,
        coverage_gaps: vec![CoverageGap::new(GAP_HOOK_WRITE, "source gap")],
        active_writers: vec![ActiveWriter {
            id: "writer".to_owned(),
            turn: 7,
            started_at: fixed_time("2026-02-03T04:05:06Z"),
            kind: "background".to_owned(),
        }],
        last_mutation_seq: 42,
        session_revision: 9,
        legacy: true,
        expired_file_payload: true,
        fork_copied: false,
    };
    let (_temp, root, _dir, store) = load_fixture(&[original]);
    let source_meta = store.list().pop().unwrap();
    let destination = root.join("child.ckpt");

    store
        .copy_conversation_prefix_to(&destination, "child", 5)
        .unwrap();

    let stored: Checkpoint =
        serde_json::from_slice(&fs::read(turn_path(&destination, 7, false)).unwrap()).unwrap();
    assert_eq!(
        stored,
        Checkpoint {
            schema_version: SCHEMA_V2,
            turn: source_meta.turn,
            time: source_meta.time,
            prompt: source_meta.prompt,
            msg_index: source_meta.msg_index,
            session_id: "child".to_owned(),
            files: Vec::new(),
            coverage: Coverage::None,
            fork_copied: true,
            ..Checkpoint::default()
        }
    );
    let wire = read_turn(&destination, 7);
    assert_eq!(wire["schemaVersion"], 2);
    assert_eq!(wire["sessionId"], "child");
    assert_eq!(wire["files"], json!([]));
    assert_eq!(wire["coverage"], "none");
    assert_eq!(wire["forkCopied"], true);
    assert_eq!(
        wire.as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "coverage",
            "files",
            "forkCopied",
            "msgIndex",
            "prompt",
            "schemaVersion",
            "sessionId",
            "time",
            "turn",
        ])
    );
}

#[test]
fn conversation_fork_uses_compact_go_json_bytes() {
    let sensitive = "<>&\u{2028}\u{2029}";
    let original = Checkpoint {
        schema_version: SCHEMA_V2,
        turn: 1,
        time: fixed_time("2026-02-03T04:05:06.123456789Z"),
        prompt: format!("prompt {sensitive}"),
        msg_index: 2,
        session_id: "source".to_owned(),
        coverage: Coverage::None,
        ..Checkpoint::default()
    };
    let (_temp, root, _dir, store) = load_fixture(std::slice::from_ref(&original));
    let destination = root.join("child.ckpt");

    store
        .copy_conversation_prefix_to(&destination, "child", 3)
        .unwrap();

    let bytes = fs::read(turn_path(&destination, original.turn, false)).unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(!text.contains('\n'));
    assert!(
        !text
            .chars()
            .any(|character| matches!(character, '<' | '>' | '&' | '\u{2028}' | '\u{2029}'))
    );
    assert!(text.contains(r#""prompt":"prompt \u003c\u003e\u0026\u2028\u2029""#));
    let stored: Checkpoint = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(stored.prompt, original.prompt);
    assert_eq!(stored.session_id, "child");
}

#[test]
fn does_not_copy_blobs_plans_or_undo_transactions() {
    let (_temp, root, dir, store) = fixture();
    write(&root, "a.txt", "before");
    store.set_session_id("source");
    store.begin(0, "source", 0).unwrap();
    store.capture_before(
        "a.txt",
        CaptureBeforeOpts {
            source: Some(CaptureSource::BeforeMutation),
            ..CaptureBeforeOpts::default()
        },
    );
    write(&root, "a.txt", "after");
    store
        .capture_after(
            "a.txt",
            CaptureAfterOpts {
                seq: 1,
                source: Some(CaptureSource::AfterMutation),
                ..CaptureAfterOpts::default()
            },
        )
        .unwrap();
    let blob_ref = store.revisions()[0].blob_ref.clone();
    assert!(valid_blob_ref(&blob_ref));
    let plan = store.prepare_rewind(0, RewindScope::Code, 7, 0, false);
    assert!(plan.can_files, "{plan:?}");
    let result = store
        .commit_rewind(&plan.plan_id, 7, b"forward", None, None)
        .unwrap();
    assert!(result.ok);
    assert!(store.current_undo_offer().is_some());
    assert!(!transaction_test_support::transaction_paths(&dir).is_empty());
    let pending = store.prepare_rewind(0, RewindScope::Code, 7, 0, false);
    assert!(pending.can_files, "{pending:?}");
    let original = fs::read(turn_path(&dir, 0, false)).unwrap();
    let blob = store.blobs().unwrap().get(&blob_ref).unwrap();
    let destination = root.join("child.ckpt");

    store
        .copy_conversation_prefix_to(&destination, "child", 1)
        .unwrap();

    let mut entries: Vec<_> = fs::read_dir(&destination)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    entries.sort();
    assert_eq!(entries, vec!["turn-0.json"]);
    let child = Store::new(Some(&destination), &root);
    assert!(child.current_undo_offer().is_none());
    assert!(matches!(
        child.validate_plan_session_revision(&pending.plan_id, 7),
        Err(RewindError::UnknownOrExpiredPlan)
    ));
    assert!(child.file_state("a.txt").is_none());
    assert!(store.current_undo_offer().is_some());
    store
        .validate_plan_session_revision(&pending.plan_id, 7)
        .unwrap();
    assert_eq!(fs::read(turn_path(&dir, 0, false)).unwrap(), original);
    assert_eq!(store.blobs().unwrap().get(&blob_ref).unwrap(), blob);
}

#[test]
fn preserves_unrelated_destination_files() {
    let (_temp, root, _dir, store) = fixture();
    store.begin(2, "source", 2).unwrap();
    let destination = root.join("child.ckpt");
    fs::create_dir_all(destination.join("notes")).unwrap();
    fs::create_dir_all(destination.join("blobs")).unwrap();
    fs::write(destination.join("notes/keep.txt"), b"keep notes").unwrap();
    fs::write(destination.join("blobs/keep.bin"), b"keep blob").unwrap();
    fs::write(destination.join("turn-99.json"), b"unrelated checkpoint").unwrap();

    store
        .copy_conversation_prefix_to(&destination, "child", 3)
        .unwrap();

    assert_eq!(
        fs::read(destination.join("notes/keep.txt")).unwrap(),
        b"keep notes"
    );
    assert_eq!(
        fs::read(destination.join("blobs/keep.bin")).unwrap(),
        b"keep blob"
    );
    assert_eq!(
        fs::read(destination.join("turn-99.json")).unwrap(),
        b"unrelated checkpoint"
    );
    assert_eq!(read_turn(&destination, 2)["forkCopied"], true);
}

#[test]
fn does_not_load_or_recover_existing_destination_state() {
    let (_temp, root, _dir, store) = fixture();
    store.begin(0, "source", 0).unwrap();
    let destination = root.join("child.ckpt");
    fs::create_dir_all(transaction_dir(&destination)).unwrap();
    let prepared = TransactionManifest {
        schema_version: SCHEMA_V2,
        id: "prepared".to_owned(),
        session_id: "destination".to_owned(),
        state: TransactionState::Prepared,
        kind: "rewind".to_owned(),
        turn: 9,
        created_at: fixed_time("2026-02-03T04:05:06Z"),
        updated_at: fixed_time("2026-02-03T04:05:06Z"),
        ..TransactionManifest::default()
    };
    let manifest = transaction_dir(&destination).join("prepared.json");
    write_json_atomic(&manifest, &prepared).unwrap();
    let original = fs::read(&manifest).unwrap();

    store
        .copy_conversation_prefix_to(&destination, "child", 1)
        .unwrap();

    assert_eq!(fs::read(&manifest).unwrap(), original);
    assert_eq!(read_turn(&destination, 0)["sessionId"], "child");
    let recovered = Store::new(Some(&destination), &root);
    assert!(
        recovered
            .recovery_notes()
            .iter()
            .any(|note| note.phase == RecoveryPhase::AbortPrepared)
    );
    assert_ne!(fs::read(&manifest).unwrap(), original);
}

#[test]
fn persistence_failure_retains_turn_and_store_error() {
    let first = checkpoint(2, 0);
    let second = checkpoint(7, 1);
    let (_temp, root, dir, store) = load_fixture(&[first.clone(), second.clone()]);
    let destination = root.join("child.ckpt");
    let blocked = turn_path(&destination, 7, false);
    fs::create_dir_all(&blocked).unwrap();
    fs::write(blocked.join("keep"), b"obstruction").unwrap();
    let first_before = fs::read(turn_path(&dir, 2, false)).unwrap();
    let second_before = fs::read(turn_path(&dir, 7, false)).unwrap();

    let error = store
        .copy_conversation_prefix_to(&destination, "child", 2)
        .expect_err("destination persistence must fail");

    let ConversationForkError::Persist { turn, source } = &error else {
        panic!("expected typed persistence error, got {error:?}");
    };
    assert_eq!(*turn, 7);
    assert!(matches!(
        source,
        StoreError::Io {
            op: "write checkpoint",
            path,
            ..
        } if path.as_path() == blocked.as_path()
    ));
    let chained = std::error::Error::source(&error).unwrap();
    assert!(chained.downcast_ref::<StoreError>().is_some());
    assert!(
        std::error::Error::source(chained)
            .unwrap()
            .downcast_ref::<io::Error>()
            .is_some()
    );
    assert_eq!(read_turn(&destination, 2)["forkCopied"], true);
    assert_eq!(fs::read(turn_path(&dir, 2, false)).unwrap(), first_before);
    assert_eq!(fs::read(turn_path(&dir, 7, false)).unwrap(), second_before);
}
