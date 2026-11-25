use std::fs::{self, FileTimes};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::time::{Duration, SystemTime};

use reasonix_contract::provider::{Message, Role};
use tempfile::TempDir;

use super::{
    LegacyMigrationError, LegacyReport, SkipReason, import_legacy_sessions_from_explicit_dir,
    migrate_legacy_sessions, migrate_legacy_sessions_from_config_dir,
};
use crate::sessionstore::branch::{branch_id, load_branch_meta, save_branch_meta};
use crate::sessionstore::session::{Session, load_session, with_session_save_path_lock};
use crate::sessionstore::session_lease::try_acquire_session_lease;
use crate::sessionstore::session_lock::{SessionFileLockError, try_lock_session_file};

const EVENTS: &str = r#"{"type":"user.message","text":"list the files"}
{"type":"model.final","content":"","reasoningContent":"inspect first","toolCalls":[{"id":"call_1","function":{"name":"read_file","arguments":"{\"path\":\"main.go\"}","thought_signature":"signed"}}]}
{"type":"tool.result","callId":"call_1","output":"package main"}
{"type":"model.final","content":"Found it."}
"#;
const MESSAGES: &str = r#"{"role":"user","content":"hello from v0.x"}
{"role":"assistant","content":"","tool_calls":[{"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"main.go\"}","thought_signature":"gemini-jsonl-signed"}}],"reasoning_content":"need to read it"}
{"role":"tool","tool_call_id":"call_1","name":"read_file","content":"package main"}
{"role":"assistant","content":"Found the main function."}
"#;
const ROUTED: &str = ".legacy-imported.v2-routed";
const ROUTED_CONFIG: &str = ".legacy-imported.v0-events-config.v2-routed";
const JSONL: &str = ".legacy-imported.v3-jsonl";
const FLAT: &str = ".legacy-imported";

fn write(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

fn set_mtime(path: &Path, at: SystemTime) {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(at))
        .unwrap();
}

fn mtime(path: &Path) -> SystemTime {
    fs::metadata(path).unwrap().modified().unwrap()
}

fn title(dest: &Path, base: &str) -> String {
    let titles: serde_json::Value =
        serde_json::from_str(&read(&dest.join(".titles.json"))).unwrap();
    titles[format!("{base}.jsonl")].as_str().unwrap().to_owned()
}

fn legacy_meta(dir: &Path, base: &str, workspace: &Path, summary: &str) {
    write(
        &dir.join(format!("{base}.meta.json")),
        &serde_json::json!({"workspace": workspace, "summary": summary}).to_string(),
    );
}

fn branch_meta(path: &Path, workspace: &Path, scope: &str, topic: &str) {
    let meta = crate::sessionstore::branch::BranchMeta {
        scope: scope.to_owned(),
        workspace_root: workspace.to_string_lossy().into_owned(),
        topic_title: topic.to_owned(),
        archived: true,
        pinned: true,
        ..Default::default()
    };
    save_branch_meta(path, meta).unwrap();
}

fn subagent(dir: &Path, ref_: &str, parent: &str) {
    write(
        &dir.join("subagents").join(format!("{ref_}.jsonl")),
        "{\"role\":\"user\",\"content\":\"sub\"}\n",
    );
    write(
        &dir.join("subagents").join(format!("{ref_}.meta.json")),
        &serde_json::json!({"ref":ref_,"parentSession":parent,"status":"completed","kind":"task","name":"task"}).to_string(),
    );
}

fn route_to<'a>(dest: &'a Path) -> impl Fn(&Path) -> Option<PathBuf> + 'a {
    move |_| Some(dest.to_path_buf())
}

fn fixture() -> (TempDir, PathBuf, PathBuf, PathBuf, PathBuf) {
    let root = TempDir::new().unwrap();
    let src = root.path().join("src");
    let global = root.path().join("global");
    let workspace = root.path().join("workspace");
    let project = root.path().join("project");
    for dir in [&src, &global, &workspace, &project] {
        fs::create_dir_all(dir).unwrap();
    }
    (root, src, global, workspace, project)
}

fn stamp_imported(dest: &Path, at: SystemTime) {
    for marker in [ROUTED, JSONL, FLAT] {
        let path = dest.join(marker);
        write(&path, "");
        set_mtime(&path, at);
    }
}

static MOVE_HOOK_TEST_LOCK: Mutex<()> = Mutex::new(());

struct MoveHookGuard {
    _lock: MutexGuard<'static, ()>,
}

impl MoveHookGuard {
    fn new() -> Self {
        Self {
            _lock: MOVE_HOOK_TEST_LOCK
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        }
    }
}

impl Drop for MoveHookGuard {
    fn drop(&mut self) {
        super::native::set_move_hook(None);
    }
}

#[test]
fn event_pass_reconstructs_messages_and_tool_names_without_touching_source() {
    let (_root, src, global, _workspace, _project) = fixture();
    let events = src.join("chat.events.jsonl");
    write(&events, EVENTS);
    let before = read(&events);

    assert_eq!(migrate_legacy_sessions(&src, &global, None).unwrap(), 1);
    let loaded = load_session(global.join("chat.jsonl").to_str().unwrap()).unwrap();
    assert_eq!(loaded.snapshot().len(), 4);
    assert_eq!(loaded.snapshot()[1].reasoning_content, "inspect first");
    assert_eq!(
        loaded.snapshot()[1].tool_calls[0].thought_signature,
        "signed"
    );
    assert_eq!(loaded.snapshot()[2].role, Role::TOOL);
    assert_eq!(loaded.snapshot()[2].name, "read_file");
    assert_eq!(read(&events), before);
    assert_eq!(mtime(&global.join("chat.jsonl")), mtime(&events));
}

#[test]
fn jsonl_pass_flattens_nested_calls_and_records_imported_title() {
    let (_root, src, global, _workspace, _project) = fixture();
    write(&src.join("chat.jsonl"), MESSAGES);
    legacy_meta(&src, "chat", Path::new(""), "legacy summary");

    assert_eq!(migrate_legacy_sessions(&src, &global, None).unwrap(), 1);
    let loaded = load_session(global.join("chat.jsonl").to_str().unwrap()).unwrap();
    let call = &loaded.snapshot()[1].tool_calls[0];
    assert_eq!(call.name, "read_file");
    assert_eq!(call.arguments, r#"{"path":"main.go"}"#);
    assert_eq!(call.thought_signature, "gemini-jsonl-signed");
    assert_eq!(title(&global, "chat"), "legacy summary");
    assert!(global.join(JSONL).exists());
}

#[test]
fn jsonl_pass_preserves_already_flat_tool_calls() {
    let (_root, src, global, _workspace, _project) = fixture();
    let record = r#"{"role":"assistant","content":"","tool_calls":[{"id":"call_flat","name":"read_file","arguments":"{\"path\":\"main.go\"}","thought_signature":"flat-signed"}]}"#;
    write(&src.join("chat.jsonl"), &format!("{record}\n"));

    assert_eq!(migrate_legacy_sessions(&src, &global, None).unwrap(), 1);
    assert_eq!(read(&global.join("chat.jsonl")), format!("{record}\n"));
    let loaded = load_session(global.join("chat.jsonl").to_str().unwrap()).unwrap();
    let call = &loaded.snapshot()[0].tool_calls[0];
    assert_eq!(call.name, "read_file");
    assert_eq!(call.arguments, r#"{"path":"main.go"}"#);
    assert_eq!(call.thought_signature, "flat-signed");
}

#[test]
fn jsonl_pass_preserves_unmodified_fields_in_transformed_records() {
    let (_root, src, global, _workspace, _project) = fixture();
    let record = r#"{"role":"assistant","content":"keep","extra":1.00e+2,"tool_calls":[{"id":"call_1","function":{"name":"read_file","arguments":"{\"path\":\"main.go\"}","thought_signature":"signed"}}]}"#;
    write(&src.join("chat.jsonl"), &format!("{record}\n"));

    assert_eq!(migrate_legacy_sessions(&src, &global, None).unwrap(), 1);
    let output = read(&global.join("chat.jsonl"));
    assert!(output.contains(r#""content":"keep""#));
    assert!(output.contains(r#""extra":1.00e+2"#));
    let loaded = load_session(global.join("chat.jsonl").to_str().unwrap()).unwrap();
    assert_eq!(loaded.snapshot()[0].tool_calls[0].name, "read_file");
}

#[test]
fn newer_message_sidecar_wins_and_uses_event_log_mtime() {
    let (_root, src, global, _workspace, _project) = fixture();
    let events = src.join("chat.events.jsonl");
    write(&events, EVENTS);
    let at = SystemTime::now() - Duration::from_secs(100);
    set_mtime(&events, at);
    write(&src.join("chat.jsonl"), MESSAGES);

    assert_eq!(migrate_legacy_sessions(&src, &global, None).unwrap(), 1);
    assert!(read(&global.join("chat.jsonl")).contains("hello from v0.x"));
    assert_eq!(mtime(&global.join("chat.jsonl")), at);
}

#[test]
fn newer_event_log_wins_over_message_sidecar() {
    let (_root, src, global, _workspace, _project) = fixture();
    write(&src.join("chat.jsonl"), MESSAGES);
    write(&src.join("chat.events.jsonl"), EVENTS);

    assert_eq!(migrate_legacy_sessions(&src, &global, None).unwrap(), 1);
    assert!(read(&global.join("chat.jsonl")).contains("list the files"));
}

#[test]
fn backup_is_recovered_only_without_a_message_transcript() {
    let (_root, src, global, _workspace, _project) = fixture();
    write(&src.join("recovered.jsonl.bak"), MESSAGES);
    write(&src.join("current.jsonl"), MESSAGES);
    write(
        &src.join("current.jsonl.bak"),
        r#"{"role":"user","content":"stale"}"#,
    );

    assert_eq!(migrate_legacy_sessions(&src, &global, None).unwrap(), 2);
    assert!(read(&global.join("recovered.jsonl")).contains("hello from v0.x"));
    assert!(!read(&global.join("current.jsonl")).contains("stale"));
}

#[test]
fn completed_markers_do_not_resurrect_a_deleted_import() {
    let (_root, src, global, _workspace, _project) = fixture();
    write(&src.join("chat.jsonl"), MESSAGES);
    assert_eq!(migrate_legacy_sessions(&src, &global, None).unwrap(), 1);
    fs::remove_file(global.join("chat.jsonl")).unwrap();

    assert_eq!(migrate_legacy_sessions(&src, &global, None).unwrap(), 0);
    assert!(!global.join("chat.jsonl").exists());
    for marker in [
        ROUTED,
        JSONL,
        FLAT,
        ".legacy-imported.v0-events-home",
        ".legacy-imported.v0-events-config",
    ] {
        assert!(global.join(marker).exists(), "missing marker {marker}");
    }
}

#[test]
fn routed_and_jsonl_markers_gate_passes_independently() {
    let (_root, src, global, _workspace, _project) = fixture();
    write(&global.join(ROUTED), "");
    let old_watermark = SystemTime::now() - Duration::from_secs(86400);
    set_mtime(&global.join(ROUTED), old_watermark);
    write(&src.join("chat.jsonl"), MESSAGES);

    assert_eq!(migrate_legacy_sessions(&src, &global, None).unwrap(), 1);
    assert!(global.join("chat.jsonl").exists());
    assert!(global.join(JSONL).exists());
    assert!(mtime(&global.join(ROUTED)) > old_watermark);
}

#[test]
fn marker_write_failure_does_not_fail_a_completed_import() {
    for blocked in [ROUTED, FLAT] {
        let (_root, src, global, _workspace, _project) = fixture();
        fs::create_dir(global.join(blocked)).unwrap();
        write(&src.join("chat.jsonl"), MESSAGES);

        assert_eq!(migrate_legacy_sessions(&src, &global, None).unwrap(), 1);
        assert!(global.join("chat.jsonl").exists());
        assert!(global.join(blocked).is_dir());
        if blocked == ROUTED {
            assert!(global.join(JSONL).exists());
            assert!(global.join(FLAT).is_file());
        } else {
            assert!(global.join(ROUTED).is_file());
            assert!(global.join(JSONL).is_file());
        }
    }
}

#[test]
fn config_entry_point_has_an_independent_routed_marker() {
    let (_root, src, global, _workspace, _project) = fixture();
    stamp_imported(&global, SystemTime::now());
    write(&src.join("config-chat.events.jsonl"), EVENTS);

    assert_eq!(
        migrate_legacy_sessions_from_config_dir(&src, &global, None).unwrap(),
        1
    );
    assert!(global.join(ROUTED_CONFIG).exists());
    assert!(global.join("config-chat.jsonl").exists());
}

#[test]
fn explicit_import_ignores_default_markers_and_stamps_its_own_pair() {
    let (_root, src, global, _workspace, _project) = fixture();
    stamp_imported(&global, SystemTime::now());
    write(&src.join("explicit.jsonl"), MESSAGES);

    let result = import_legacy_sessions_from_explicit_dir(&src, &global, None).unwrap();
    assert_eq!(result.imported, 1);
    assert!(result.report.skipped.is_empty());
    assert!(global.join("explicit.jsonl").exists());
    let markers: Vec<_> = fs::read_dir(&global)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".legacy-imported.explicit."))
        .collect();
    assert_eq!(markers.len(), 2);
    assert!(markers.iter().any(|name| name.ends_with(".jsonl")));
}

#[test]
fn absent_and_unreadable_sources_have_distinct_taxonomy() {
    let root = TempDir::new().unwrap();
    let global = root.path().join("global");
    fs::create_dir_all(&global).unwrap();
    let absent = root.path().join("absent");

    assert_eq!(migrate_legacy_sessions(&absent, &global, None).unwrap(), 0);
    assert!(!global.join(ROUTED).exists());
    let unreadable = root.path().join("not-a-directory");
    write(&unreadable, "block");
    let error = migrate_legacy_sessions(&unreadable, &global, None).unwrap_err();
    assert!(matches!(error, LegacyMigrationError::Source(_)));
    assert!(!global.join(ROUTED).exists());

    let result = import_legacy_sessions_from_explicit_dir(&unreadable, &global, None).unwrap();
    assert_eq!(result.imported, 0);
    assert_eq!(result.report.skipped.len(), 1);
    assert_eq!(result.report.skipped[0].reason, SkipReason::Corrupt);
}

#[test]
fn explicit_import_of_absent_source_is_a_silent_noop() {
    let root = TempDir::new().unwrap();
    let global = root.path().join("global");
    fs::create_dir_all(&global).unwrap();
    let absent = root.path().join("absent");

    let result = import_legacy_sessions_from_explicit_dir(&absent, &global, None).unwrap();

    assert_eq!(result.imported, 0);
    assert!(result.report.skipped.is_empty());
    assert!(!global.join(ROUTED).exists());
    assert!(!global.join(JSONL).exists());
}

#[test]
fn copy_failure_withholds_markers_and_a_later_run_retries() {
    let (_root, src, global, workspace, _project) = fixture();
    write(&src.join("chat.events.jsonl"), EVENTS);
    legacy_meta(&src, "chat", &workspace, "retry me");
    let blocker = _root.path().join("blocker");
    write(&blocker, "not a directory");
    let router = |_: &Path| Some(blocker.join("sessions"));

    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&router)).unwrap(),
        0
    );
    assert!(!global.join(ROUTED).exists());
    assert!(!global.join(JSONL).exists());
    fs::remove_file(&blocker).unwrap();
    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&router)).unwrap(),
        1
    );
    assert!(blocker.join("sessions/chat.jsonl").exists());
    assert!(global.join(ROUTED).exists());
}

#[test]
fn failed_event_copy_does_not_starve_the_jsonl_pass() {
    let (_root, src, global, workspace, _project) = fixture();
    write(&src.join("routed.events.jsonl"), EVENTS);
    legacy_meta(&src, "routed", &workspace, "blocked");
    write(&src.join("flat.jsonl"), MESSAGES);
    let blocker = _root.path().join("blocker");
    write(&blocker, "not a directory");
    let router = |_: &Path| Some(blocker.join("sessions"));

    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&router)).unwrap(),
        1
    );
    assert!(global.join("flat.jsonl").exists());
    assert!(!global.join(ROUTED).exists());
}

#[test]
fn existing_destination_is_preserved() {
    let (_root, src, global, _workspace, _project) = fixture();
    write(&src.join("chat.events.jsonl"), EVENTS);
    write(
        &global.join("chat.jsonl"),
        r#"{"role":"user","content":"native"}"#,
    );

    assert_eq!(migrate_legacy_sessions(&src, &global, None).unwrap(), 0);
    assert_eq!(
        read(&global.join("chat.jsonl")),
        r#"{"role":"user","content":"native"}"#
    );
}

#[test]
fn existing_destination_completes_branch_meta_once() {
    let (_root, src, global, workspace, project) = fixture();
    let source = src.join("chat.jsonl");
    write(&source, MESSAGES);
    branch_meta(&source, &workspace, "project", "1.x topic");
    let dest = project.join("chat.jsonl");
    write(&dest, MESSAGES);
    let router = route_to(&project);

    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&router)).unwrap(),
        1
    );
    let meta = load_branch_meta(&dest).unwrap().unwrap();
    assert_eq!(meta.scope, "project");
    assert!(meta.archived && meta.pinned);
    assert_eq!(title(&project, "chat"), "1.x topic");
    let pinned = SystemTime::now() - Duration::from_secs(3600);
    set_mtime(&dest, pinned);
    set_mtime(&crate::store::session_meta(&dest), pinned);
    let result = import_legacy_sessions_from_explicit_dir(&src, &global, Some(&router)).unwrap();
    assert_eq!(result.imported, 0);
    assert_eq!(mtime(&dest), pinned);
    assert_eq!(mtime(&crate::store::session_meta(&dest)), pinned);
}

#[test]
fn existing_corrupt_branch_meta_is_never_rewritten() {
    let (_root, src, global, workspace, project) = fixture();
    let source = src.join("chat.jsonl");
    write(&source, MESSAGES);
    branch_meta(&source, &workspace, "project", "topic");
    let dest = project.join("chat.jsonl");
    write(&dest, MESSAGES);
    let meta_path = crate::store::session_meta(&dest);
    write(&meta_path, "corrupt");

    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&route_to(&project))).unwrap(),
        0
    );
    assert_eq!(read(&meta_path), "corrupt");
    assert!(!global.join(ROUTED).exists());
}

#[test]
fn flat_subagents_are_excluded_and_parent_artifacts_are_copied() {
    let (_root, src, global, workspace, project) = fixture();
    write(&src.join("chat.jsonl"), MESSAGES);
    branch_meta(&src.join("chat.jsonl"), &workspace, "project", "parent");
    write(&src.join("subagent-flat.jsonl"), MESSAGES);
    subagent(&src, "sa_20260101_000000_000000000_aabbccddeeff", "chat");

    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&route_to(&project))).unwrap(),
        1
    );
    assert!(!project.join("subagent-flat.jsonl").exists());
    assert!(
        project
            .join("subagents/sa_20260101_000000_000000000_aabbccddeeff.jsonl")
            .exists()
    );
    assert!(
        project
            .join("subagents/sa_20260101_000000_000000000_aabbccddeeff.meta.json")
            .exists()
    );
}

#[test]
fn flat_import_is_rehomed_without_reconstructing() {
    let (_root, src, global, workspace, project) = fixture();
    let events = src.join("chat.events.jsonl");
    write(&events, EVENTS);
    legacy_meta(&src, "chat", &workspace, "flat title");
    let flat = global.join("chat.jsonl");
    write(&flat, r#"{"role":"user","content":"flat-imported"}"#);
    set_mtime(&flat, mtime(&events));

    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&route_to(&project))).unwrap(),
        1
    );
    assert!(read(&project.join("chat.jsonl")).contains("flat-imported"));
    assert!(!flat.exists());
    assert_eq!(title(&project, "chat"), "flat title");
    assert!(src.join("chat.events.jsonl").exists());
}

#[test]
fn native_flat_import_rehomes_wal_state_ahead_of_checkpoint() {
    let (_root, src, global, workspace, project) = fixture();
    let events = src.join("chat.events.jsonl");
    write(&events, EVENTS);
    legacy_meta(&src, "chat", &workspace, "native flat");
    let flat = global.join("chat.jsonl");
    let native = Session::new("");
    native.add(Message {
        role: Role::USER,
        content: "WAL newest".into(),
        ..Default::default()
    });
    native.save(flat.to_str().unwrap()).unwrap();
    write(&flat, r#"{"role":"user","content":"stale checkpoint"}"#);
    branch_meta(&flat, &workspace, "project", "native topic");
    set_mtime(&flat, mtime(&events));

    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&route_to(&project))).unwrap(),
        1
    );
    let loaded = load_session(project.join("chat.jsonl").to_str().unwrap()).unwrap();
    assert_eq!(loaded.snapshot()[0].content, "WAL newest");
    assert_eq!(
        load_branch_meta(&project.join("chat.jsonl"))
            .unwrap()
            .unwrap()
            .id,
        branch_id(project.join("chat.jsonl").to_str().unwrap())
    );
    assert!(!flat.exists());
    assert!(!global.join("chat.events.jsonl").exists());
    assert!(project.join("chat.event-index.json").exists());
    assert!(project.join("chat.display-index.json").exists());
    assert!(fs::read_dir(&project).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".legacy-import")
    }));
}

#[test]
fn native_flat_import_holds_destination_locks_through_retirement() {
    let (_root, src, global, workspace, project) = fixture();
    let events = src.join("chat.events.jsonl");
    write(&events, EVENTS);
    legacy_meta(&src, "chat", &workspace, "locked native");
    let flat = global.join("chat.jsonl");
    let native = Session::new("");
    native.add(Message {
        role: Role::USER,
        content: "locked WAL".into(),
        ..Default::default()
    });
    native.save(flat.to_str().unwrap()).unwrap();
    branch_meta(&flat, &workspace, "project", "locked topic");
    set_mtime(&flat, mtime(&events));
    let dest = project.join("chat.jsonl");

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let hook_old = flat.clone();
    let _hook_guard = MoveHookGuard::new();
    super::native::set_move_hook(Some(Box::new(move |path| {
        if path == hook_old {
            let _ = entered_tx.send(());
            let _ = release_rx.recv();
        }
    })));

    let (file_held, save_held, imported) = std::thread::scope(|scope| {
        let worker =
            scope.spawn(|| migrate_legacy_sessions(&src, &global, Some(&route_to(&project))));
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let file_held = match try_lock_session_file(&dest) {
            Ok(_) => false,
            Err(SessionFileLockError::Held) => true,
            Err(error) => panic!("destination lock probe failed: {error}"),
        };
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let waiter = scope.spawn(move || {
            with_session_save_path_lock(&dest.to_string_lossy(), || {
                let _ = acquired_tx.send(());
            });
        });
        let save_held = acquired_rx
            .recv_timeout(Duration::from_millis(250))
            .is_err();
        let _ = release_tx.send(());
        let imported = worker.join().unwrap().unwrap();
        if save_held {
            acquired_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        waiter.join().unwrap();
        (file_held, save_held, imported)
    });

    assert_eq!(imported, 1);
    assert!(file_held);
    assert!(save_held);
    assert!(!flat.exists());
}

#[test]
fn native_flat_import_sidecar_failure_withholds_markers_and_retries() {
    let (_root, src, global, workspace, project) = fixture();
    let events = src.join("chat.events.jsonl");
    write(&events, EVENTS);
    legacy_meta(&src, "chat", &workspace, "retry native");
    let flat = global.join("chat.jsonl");
    let native = Session::new("");
    native.add(Message {
        role: Role::USER,
        content: "retry WAL".into(),
        ..Default::default()
    });
    native.save(flat.to_str().unwrap()).unwrap();
    branch_meta(&flat, &workspace, "project", "retry topic");
    set_mtime(&flat, mtime(&events));
    let dest = project.join("chat.jsonl");
    let dest_meta = project.join("chat.jsonl.meta");
    let hook_meta = dest_meta.clone();
    let hook_old = flat.clone();
    let triggered = Arc::new(AtomicBool::new(false));
    let hook_triggered = Arc::clone(&triggered);
    let _hook_guard = MoveHookGuard::new();
    super::native::set_move_hook(Some(Box::new(move |path| {
        if path == hook_old && !hook_triggered.swap(true, Ordering::SeqCst) {
            let _ = fs::remove_file(&hook_meta);
            fs::create_dir_all(&hook_meta).unwrap();
        }
    })));
    let router = route_to(&project);

    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&router)).unwrap(),
        0
    );
    assert!(dest.exists());
    assert!(flat.exists());
    assert!(!global.join(ROUTED).exists());
    assert!(!global.join(JSONL).exists());

    fs::remove_dir_all(&dest_meta).unwrap();
    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&router)).unwrap(),
        1
    );
    assert!(!flat.exists());
    assert_eq!(
        load_session(dest.to_str().unwrap()).unwrap().snapshot()[0].content,
        "retry WAL"
    );
    assert_eq!(
        load_branch_meta(&dest).unwrap().unwrap().id,
        branch_id(dest.to_str().unwrap())
    );
}

#[test]
fn same_named_native_global_session_is_not_moved() {
    let (_root, src, global, workspace, project) = fixture();
    let events = src.join("chat.events.jsonl");
    write(&events, EVENTS);
    let old = SystemTime::now() - Duration::from_secs(172800);
    set_mtime(&events, old);
    legacy_meta(&src, "chat", &workspace, "");
    let native = global.join("chat.jsonl");
    write(&native, r#"{"role":"user","content":"native"}"#);

    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&route_to(&project))).unwrap(),
        1
    );
    assert!(read(&native).contains("native"));
    assert!(read(&project.join("chat.jsonl")).contains("list the files"));
}

#[test]
fn leased_flat_import_defers_and_withholds_markers() {
    let (_root, src, global, workspace, project) = fixture();
    let events = src.join("chat.events.jsonl");
    write(&events, EVENTS);
    legacy_meta(&src, "chat", &workspace, "leased");
    let flat = global.join("chat.jsonl");
    write(&flat, MESSAGES);
    set_mtime(&flat, mtime(&events));
    let lease = try_acquire_session_lease(flat.to_str().unwrap()).unwrap();
    let router = route_to(&project);

    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&router)).unwrap(),
        0
    );
    assert!(!project.join("chat.jsonl").exists());
    assert!(!global.join(ROUTED).exists());
    lease.release();
    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&router)).unwrap(),
        1
    );
    assert!(project.join("chat.jsonl").exists());
}

#[test]
fn stranded_project_session_rehomes_with_sidecars_and_is_idempotent() {
    let (_root, src, global, workspace, project) = fixture();
    stamp_imported(&global, SystemTime::now() - Duration::from_secs(86400));
    let source = src.join("chat.jsonl");
    write(&source, MESSAGES);
    branch_meta(&source, &workspace, "project", "downgrade work");
    subagent(&src, "sa_20260101_000000_000000000_aabbccddeeff", "chat");
    let router = route_to(&project);

    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&router)).unwrap(),
        1
    );
    assert!(project.join("chat.jsonl").exists());
    assert!(project.join("chat.jsonl.meta").exists());
    assert!(
        project
            .join("subagents/sa_20260101_000000_000000000_aabbccddeeff.jsonl")
            .exists()
    );
    assert_eq!(mtime(&project.join("chat.jsonl")), mtime(&source));
    assert!(source.exists());
    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&router)).unwrap(),
        0
    );
}

#[test]
fn stranded_global_branch_meta_overrides_stale_legacy_project_meta() {
    let (_root, src, global, workspace, project) = fixture();
    stamp_imported(&global, SystemTime::now() - Duration::from_secs(86400));
    let source = src.join("chat.jsonl");
    write(&source, MESSAGES);
    legacy_meta(&src, "chat", &workspace, "stale project");
    branch_meta(&source, &workspace, "global", "explicit global");

    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&route_to(&project))).unwrap(),
        0
    );
    assert!(!project.join("chat.jsonl").exists());
    assert!(source.exists());
}

#[test]
fn stranded_rehome_ignores_sessions_older_than_watermark() {
    let (_root, src, global, workspace, project) = fixture();
    let watermark = SystemTime::now();
    stamp_imported(&global, watermark);
    let source = src.join("chat.jsonl");
    write(&source, MESSAGES);
    branch_meta(&source, &workspace, "project", "old");
    set_mtime(&source, watermark - Duration::from_secs(86400));

    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&route_to(&project))).unwrap(),
        0
    );
    assert!(!project.join("chat.jsonl").exists());
    assert!(mtime(&global.join(ROUTED)) >= watermark);
}

#[test]
fn stranded_artifact_failure_keeps_watermark_for_retry() {
    let (_root, src, global, workspace, project) = fixture();
    let watermark = SystemTime::now() - Duration::from_secs(86400);
    stamp_imported(&global, watermark);
    let source = src.join("chat.jsonl");
    write(&source, MESSAGES);
    branch_meta(&source, &workspace, "project", "artifacts");
    subagent(&src, "sa_20260101_000000_000000000_aabbccddeeff", "chat");
    write(&project.join("subagents"), "block");
    let router = route_to(&project);

    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&router)).unwrap(),
        1
    );
    assert_eq!(mtime(&global.join(ROUTED)), watermark);
    fs::remove_file(project.join("subagents")).unwrap();
    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&router)).unwrap(),
        0
    );
    assert!(
        project
            .join("subagents/sa_20260101_000000_000000000_aabbccddeeff.jsonl")
            .exists()
    );
}

#[test]
fn empty_project_event_log_does_not_create_a_session() {
    let (_root, src, global, _workspace, _project) = fixture();
    write(&src.join("legacy-project/empty.events.jsonl"), "");

    assert_eq!(migrate_legacy_sessions(&src, &global, None).unwrap(), 0);
    assert!(!global.join("empty.jsonl").exists());
    assert!(global.join(ROUTED).exists());
}

#[test]
fn project_subdirectory_sessions_are_routed() {
    let (_root, src, global, workspace, project) = fixture();
    let sub = src.join("Users_Yuki_git_polytone-audio-engine");
    write(&sub.join("chat.events.jsonl"), EVENTS);
    legacy_meta(&sub, "chat", &workspace, "nested");

    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&route_to(&project))).unwrap(),
        1
    );
    assert!(project.join("chat.jsonl").exists());
    assert!(!global.join("chat.jsonl").exists());
    assert_eq!(title(&project, "chat"), "nested");
}

#[test]
fn dead_workspace_falls_back_to_global() {
    let (_root, src, global, _workspace, _project) = fixture();
    write(&src.join("chat.events.jsonl"), EVENTS);
    legacy_meta(&src, "chat", &src.join("missing"), "dead workspace");

    assert_eq!(
        migrate_legacy_sessions(&src, &global, Some(&route_to(&_project))).unwrap(),
        1
    );
    assert!(global.join("chat.jsonl").exists());
}

#[test]
fn native_sidecar_copy_uses_wal_not_checkpoint() {
    let (_root, src, global, _workspace, _project) = fixture();
    let source = src.join("native.jsonl");
    let native = Session::new("");
    native.add(Message {
        role: Role::USER,
        content: "native WAL".into(),
        ..Default::default()
    });
    native.save(source.to_str().unwrap()).unwrap();
    write(&source, r#"{"role":"user","content":"old checkpoint"}"#);

    assert_eq!(migrate_legacy_sessions(&src, &global, None).unwrap(), 1);
    let loaded = load_session(global.join("native.jsonl").to_str().unwrap()).unwrap();
    assert_eq!(loaded.snapshot()[0].content, "native WAL");
    assert!(read(&source).contains("old checkpoint"));
}

#[test]
fn malformed_jsonl_tail_keeps_the_parsed_prefix() {
    let (_root, src, global, _workspace, _project) = fixture();
    write(
        &src.join("chat.jsonl"),
        "{\"role\":\"user\",\"content\":\"kept\"}\n{broken",
    );

    assert_eq!(migrate_legacy_sessions(&src, &global, None).unwrap(), 1);
    let loaded = load_session(global.join("chat.jsonl").to_str().unwrap()).unwrap();
    assert_eq!(loaded.snapshot()[0].content, "kept");
}

#[test]
fn jsonl_pass_preserves_non_transformed_records_verbatim() {
    let (_root, src, global, _workspace, _project) = fixture();
    let source = concat!(
        "{\"role\":\"user\", \"content\" : \"hello\", \"extra\" : 1.00e+2 }\n",
        "{ \"content\" : \"no calls\", \"role\" : \"assistant\" }\n",
        "{ \"role\" : \"tool\", \"content\" : \"done\", \"tool_call_id\" : \"call_1\" }\n",
    );
    write(&src.join("chat.jsonl"), source);

    assert_eq!(migrate_legacy_sessions(&src, &global, None).unwrap(), 1);
    assert_eq!(read(&global.join("chat.jsonl")), source);
}

#[test]
fn jsonl_pass_keeps_verbatim_prefix_before_malformed_tail() {
    let (_root, src, global, _workspace, _project) = fixture();
    let prefix = "{\"role\":\"user\", \"content\" : \"kept\" }";
    write(
        &src.join("chat.jsonl"),
        &format!("{prefix}\n{{\"role\":\"assistant\",\"content\":"),
    );

    assert_eq!(migrate_legacy_sessions(&src, &global, None).unwrap(), 1);
    assert_eq!(read(&global.join("chat.jsonl")), format!("{prefix}\n"));
}

#[test]
fn event_named_directory_does_not_shadow_a_jsonl_session() {
    let (_root, src, global, _workspace, _project) = fixture();
    fs::create_dir(src.join("chat.events.jsonl")).unwrap();
    write(&src.join("chat.jsonl"), MESSAGES);

    assert_eq!(migrate_legacy_sessions(&src, &global, None).unwrap(), 1);
    assert!(read(&global.join("chat.jsonl")).contains("hello from v0.x"));
}

#[test]
fn empty_event_log_does_not_create_a_session() {
    let (_root, src, global, _workspace, _project) = fixture();
    write(
        &src.join("empty.events.jsonl"),
        r#"{"type":"model.turn.started","id":1}"#,
    );

    assert_eq!(migrate_legacy_sessions(&src, &global, None).unwrap(), 0);
    assert!(!global.join("empty.jsonl").exists());
}

#[test]
fn existing_imported_title_is_not_overwritten() {
    for existing in ["user title", "   "] {
        let (_root, src, global, _workspace, _project) = fixture();
        write(&src.join("chat.jsonl"), MESSAGES);
        legacy_meta(&src, "chat", Path::new(""), "imported");
        write(
            &global.join(".titles.json"),
            &serde_json::json!({"chat.jsonl": existing}).to_string(),
        );

        assert_eq!(migrate_legacy_sessions(&src, &global, None).unwrap(), 1);
        assert_eq!(title(&global, "chat"), existing);
    }
}

#[test]
fn same_source_and_destination_leaves_legacy_events_untouched() {
    let (_root, src, _global, _workspace, _project) = fixture();
    write(&src.join("chat.events.jsonl"), EVENTS);
    let before = read(&src.join("chat.events.jsonl"));

    assert_eq!(migrate_legacy_sessions(&src, &src, None).unwrap(), 1);
    assert_eq!(read(&src.join("chat.events.jsonl")), before);
    assert!(src.join("chat.jsonl").exists());
}

#[test]
fn skip_reason_codes_are_stable() {
    assert_eq!(SkipReason::TooLarge.as_str(), "too_large");
    assert_eq!(SkipReason::UnreadableFormat.as_str(), "unreadable_format");
    assert_eq!(SkipReason::SchemaUnsupported.as_str(), "schema_unsupported");
    assert_eq!(SkipReason::Permission.as_str(), "permission");
    assert_eq!(SkipReason::Corrupt.as_str(), "corrupt");
    assert_eq!(SkipReason::CopyFailed.as_str(), "copy_failed");
    assert_eq!(SkipReason::NoProject.as_str(), "no_project");
}

#[test]
fn explicit_report_type_is_public_and_defaultable() {
    let report = LegacyReport::default();
    assert!(report.skipped.is_empty());
}
