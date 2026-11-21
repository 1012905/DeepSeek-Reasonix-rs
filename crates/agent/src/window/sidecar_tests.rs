//! The sidecar bind/revalidate oracles (Go: compact_commit_emit_test.go's
//! load half, projection_valid_test.go's rebind half,
//! revalidate_projection_test.go, compaction_sidecar_authority_test.go).

use std::path::PathBuf;
use std::sync::Arc;

use reasonix_state::sessionstore::Session;
use reasonix_state::sessionstore::compaction_state::{
    COMPACTION_STATE_SCHEMA_CURRENT, CompactionState, ContextProjection, covered_prefix_hash,
    load_compaction_state, save_compaction_state,
};
use reasonix_state::store::naming::session_context;

use reasonix_contract::provider::{Message, Role};

use crate::seams::fold_executor::{FoldRequest, FoldScope};
use crate::sink::Discard;
use crate::window::Window;
use crate::window::WindowConfig;
use crate::window::fold_archive::prompt_cache_key;
use crate::window::sidecar::{drop_reason, fold_not_reused_notice};
use crate::window::trigger_label;

fn msg(role: Role, content: &str) -> Message {
    Message {
        role,
        content: content.to_owned(),
        ..Message::default()
    }
}

/// A unique scratch root per test (Go: testenv.TempDir).
fn scratch(name: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "reasonix-sidecar-test-{}-{}-{}",
        name,
        std::process::id(),
        SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

/// The reusable live tail fixture (Go: projectionWithLiveTail's transcript).
fn live_fixture() -> Vec<Message> {
    let mut msgs = vec![msg(Role::SYSTEM, "sys")];
    for _ in 0..8 {
        msgs.push(msg(Role::USER, &"u".repeat(80)));
        msgs.push(msg(Role::ASSISTANT, &"a".repeat(600)));
    }
    msgs
}

fn sidecar_config(workspace: &str, model: &str) -> WindowConfig {
    WindowConfig {
        context_window: 20_000,
        compact_ratio: 0.5,
        workspace_id: workspace.to_owned(),
        model_ref: model.to_owned(),
        ..WindowConfig::default()
    }
}

fn session_with(msgs: Vec<Message>) -> Arc<Session> {
    let sess = Session::new("");
    sess.replace(msgs);
    Arc::new(sess)
}

fn sidecar_window(workspace: &str, model: &str, session: Arc<Session>, path: &str) -> Arc<Window> {
    let window = Arc::new(Window::new(
        sidecar_config(workspace, model),
        session,
        Arc::new(reasonix_tools::Registry::new()),
        Arc::new(Discard),
    ));
    window.load_projection_sidecar(path);
    window
}

fn save_sidecar(path: &std::path::Path, st: CompactionState) {
    save_compaction_state(path, st).expect("save sidecar");
}

#[allow(clippy::too_many_arguments)]
fn projection_state(
    key: &str,
    messages: Vec<Message>,
    covered: i64,
    hash: &str,
    version: u64,
) -> CompactionState {
    CompactionState {
        schema_version: COMPACTION_STATE_SCHEMA_CURRENT,
        prompt_cache_key: key.to_owned(),
        projection: ContextProjection {
            messages,
            covered_count: covered,
            covered_prefix_hash: hash.to_owned(),
            projection_version: version,
            ..ContextProjection::default()
        },
        ..CompactionState::default()
    }
}

fn sidecar_path(session: &std::path::Path) -> PathBuf {
    session_context(session)
}

// Go: TestLoadProjectionSidecarDoesNotRewriteExactKey — an exact-key
// restore stays pure-read.
#[test]
fn load_sidecar_does_not_rewrite_exact_key() {
    let dir = scratch("exact-key");
    let path = dir.join("session.jsonl");
    let msgs = vec![msg(Role::SYSTEM, "sys"), msg(Role::USER, "u")];
    let hash = covered_prefix_hash(&msgs, msgs.len() as i64);
    let key = prompt_cache_key("ws", "", "p/m");
    save_sidecar(
        &path,
        projection_state(&key, msgs.clone(), msgs.len() as i64, &hash, 3),
    );

    let sidecar = sidecar_path(&path);
    let before = std::fs::read(&sidecar).expect("read before");
    let window = sidecar_window("ws", "p/m", session_with(msgs), &path.to_string_lossy());
    assert_eq!(window.current_projection_version(), 3);
    let after = std::fs::read(&sidecar).expect("read after");
    assert_eq!(
        before,
        after,
        "exact-key restore rewrote the sidecar ({} -> {} bytes)",
        before.len(),
        after.len()
    );
}

// Go: TestLoadProjectionSidecarNormalizesNativeKeyOnce — a legacy
// native-editing key is rewritten once, then stays pure-read.
#[test]
fn load_sidecar_normalizes_native_key_once() {
    let dir = scratch("native-key");
    let path = dir.join("session.jsonl");
    let msgs = vec![msg(Role::SYSTEM, "sys"), msg(Role::USER, "u")];
    let hash = covered_prefix_hash(&msgs, msgs.len() as i64);
    let key = prompt_cache_key("ws", "", "p/m");
    save_sidecar(
        &path,
        projection_state(
            &format!("{key}|context-editing-native-anthropic"),
            msgs.clone(),
            msgs.len() as i64,
            &hash,
            2,
        ),
    );

    let window = sidecar_window(
        "ws",
        "p/m",
        session_with(msgs.clone()),
        &path.to_string_lossy(),
    );
    assert_eq!(
        window.current_projection_version(),
        2,
        "wrong sidecar loaded"
    );
    let loaded = load_compaction_state(&path)
        .expect("reload")
        .expect("sidecar");
    assert_eq!(loaded.prompt_cache_key, key, "native key not normalized");

    let sidecar = sidecar_path(&path);
    let before = std::fs::read(&sidecar).expect("read before");
    let again = sidecar_window("ws", "p/m", session_with(msgs), &path.to_string_lossy());
    assert_eq!(again.current_projection_version(), 2);
    let after = std::fs::read(&sidecar).expect("read after");
    assert_eq!(
        before, after,
        "second restore rewrote an already-normalized sidecar"
    );
}

// Go: TestLoadProjectionSidecarRebindsMatchingContentAcrossLineage — the
// upgrade/model-switch path: a matching body is rebound, and the rebind is
// persisted so the next launch does not re-downgrade.
#[test]
fn load_sidecar_rebinds_matching_content_across_lineage() {
    let dir = scratch("rebind-lineage");
    let path = dir.join("s.jsonl");
    let msgs = vec![msg(Role::SYSTEM, "sys"), msg(Role::USER, "task")];
    let hash = covered_prefix_hash(&msgs, 2);
    save_sidecar(
        &path,
        CompactionState {
            schema_version:
                reasonix_state::sessionstore::compaction_state::COMPACTION_STATE_SCHEMA_V1,
            prompt_cache_key: "ws|s|other-model".to_owned(),
            transcript_version: 1,
            projection: ContextProjection {
                messages: vec![msg(Role::SYSTEM, "sys summary")],
                covered_count: 2,
                covered_prefix_hash: hash,
                ..ContextProjection::default()
            },
            ..CompactionState::default()
        },
    );
    let sess = session_with(vec![msg(Role::SYSTEM, "sys"), msg(Role::USER, "task")]);
    let window = sidecar_window("ws", "this-model", sess, &path.to_string_lossy());

    let st = window.with_core(|core| core.compaction_state.clone());
    assert!(
        !st.projection.messages.is_empty(),
        "matching projection body was dropped on lineage change"
    );
    let want_key = window.prompt_cache_key();
    assert_eq!(st.prompt_cache_key, want_key);
    assert_eq!(
        window.with_core(|core| core.checkpoint_state.clone()),
        "restored"
    );
    // The rebind must be persisted so the next launch does not re-downgrade.
    let disk = load_compaction_state(&path)
        .expect("load")
        .expect("sidecar");
    assert_eq!(disk.prompt_cache_key, want_key);
}

// Go: TestLoadProjectionSidecarDropsForeignCacheKey — lineage rebinding
// cannot resurrect a projection whose canonical prefix differs.
#[test]
fn load_sidecar_drops_foreign_cache_key() {
    let dir = scratch("foreign-key");
    let path = dir.join("s.jsonl");
    let foreign = vec![msg(Role::SYSTEM, "sys-old")];
    save_sidecar(
        &path,
        CompactionState {
            schema_version:
                reasonix_state::sessionstore::compaction_state::COMPACTION_STATE_SCHEMA_V1,
            prompt_cache_key: "ws|s|other-model".to_owned(),
            projection: ContextProjection {
                messages: vec![msg(Role::SYSTEM, "sys")],
                covered_count: 1,
                covered_prefix_hash: covered_prefix_hash(&foreign, 1),
                ..ContextProjection::default()
            },
            ..CompactionState::default()
        },
    );
    let window = sidecar_window(
        "ws",
        "this-model",
        session_with(vec![msg(Role::SYSTEM, "sys")]),
        &path.to_string_lossy(),
    );
    let st = window.with_core(|core| core.compaction_state.clone());
    assert!(
        st.projection.messages.is_empty(),
        "foreign projection loaded: {st:?}"
    );
    assert!(
        load_compaction_state(&path).expect("load").is_some(),
        "sidecar should remain on disk for the other model"
    );
}

// Go: TestLoadProjectionSidecarSaysWhyAFoldWasNotReused + TestProjection-
// DroppedIsClearedByANewBinding.
#[test]
fn load_sidecar_says_why_a_fold_was_not_reused() {
    let cases: [(&str, &str, Vec<Message>, &str); 3] = [
        (
            "history rewritten under the same lineage",
            "",
            vec![msg(Role::SYSTEM, "sys-old")],
            drop_reason::HISTORY,
        ),
        (
            "history rewritten under another lineage",
            "ws|s|other-model",
            vec![msg(Role::SYSTEM, "sys-old")],
            drop_reason::LINEAGE,
        ),
        (
            "still valid",
            "",
            vec![msg(Role::SYSTEM, "sys")],
            drop_reason::NONE,
        ),
    ];
    for (name, key_override, covers, want) in cases {
        let dir = scratch("why-dropped");
        let path = dir.join("s.jsonl");
        let key = if key_override.is_empty() {
            prompt_cache_key("ws", "", "this-model")
        } else {
            key_override.to_owned()
        };
        save_sidecar(
            &path,
            CompactionState {
                schema_version:
                    reasonix_state::sessionstore::compaction_state::COMPACTION_STATE_SCHEMA_V1,
                prompt_cache_key: key,
                projection: ContextProjection {
                    messages: vec![msg(Role::SYSTEM, "summary")],
                    covered_count: 1,
                    covered_prefix_hash: covered_prefix_hash(&covers, 1),
                    ..ContextProjection::default()
                },
                ..CompactionState::default()
            },
        );
        let window = sidecar_window(
            "ws",
            "this-model",
            session_with(vec![msg(Role::SYSTEM, "sys")]),
            &path.to_string_lossy(),
        );
        assert_eq!(window.projection_dropped(), want, "{name}");
    }

    // A new binding with no sidecar clears the report.
    let dir = scratch("dropped-cleared");
    let path = dir.join("s.jsonl");
    save_sidecar(
        &path,
        projection_state(
            &prompt_cache_key("ws", "", "m"),
            vec![msg(Role::SYSTEM, "summary")],
            1,
            &covered_prefix_hash(&[msg(Role::SYSTEM, "other")], 1),
            1,
        ),
    );
    let window = sidecar_window(
        "ws",
        "m",
        session_with(vec![msg(Role::SYSTEM, "sys")]),
        &path.to_string_lossy(),
    );
    assert_eq!(window.projection_dropped(), drop_reason::HISTORY);
    window.bind_session_path(&dir.join("fresh.jsonl").to_string_lossy(), true);
    assert_eq!(
        window.projection_dropped(),
        drop_reason::NONE,
        "a session with no sidecar still reports a drop"
    );
}

// Go: TestRevalidateProjectionAfterRewrite — a caller that rewrote history
// asks instead of deciding for itself. Truncating into the live tail leaves
// the folded prefix untouched; an empty tail is not a reason to distrust
// coverage.
#[test]
fn revalidate_after_rewrite() {
    let dir = scratch("revalidate");
    let path = dir.join("s.jsonl");
    let sess = session_with(live_fixture());
    let window = Arc::new(Window::new(
        WindowConfig {
            context_window: 2_000,
            compact_ratio: 0.9,
            workspace_id: "ws".to_owned(),
            model_ref: "m".to_owned(),
            ..WindowConfig::default()
        },
        sess.clone(),
        Arc::new(reasonix_tools::Registry::new()),
        Arc::new(Discard),
    ));
    window.set_session_path(&path.to_string_lossy());
    let request = FoldRequest {
        trigger: trigger_label::MANUAL.to_owned(),
        instructions: String::new(),
        scope: FoldScope {
            ignore_threshold: true,
            ignore_economics: true,
            ignore_amount: true,
        },
        must_free: false,
    };
    assert!(
        window
            .compact_to_projection(&request)
            .expect("fold")
            .installed
    );
    let covered = window.with_core(|core| core.compaction_state.projection.covered_count);
    assert!(covered > 1, "fold covered {covered} messages");
    for _ in 0..2 {
        sess.add(msg(Role::USER, &"t".repeat(80)));
        sess.add(msg(Role::ASSISTANT, &"r".repeat(120)));
    }

    let has_projection = |window: &Window| {
        window.with_core(|core| !core.compaction_state.projection.messages.is_empty())
    };
    assert!(has_projection(&window), "no projection to revalidate");

    type Rewrite = Box<dyn Fn(Vec<Message>, i64) -> Vec<Message>>;
    let cases: [(&str, Rewrite, bool); 4] = [
        (
            "part of the live tail truncated",
            Box::new(|m, c| m[..(c + 2) as usize].to_vec()),
            true,
        ),
        (
            "truncated to exactly the fold boundary",
            Box::new(|m, c| m[..c as usize].to_vec()),
            true,
        ),
        (
            "truncated below the fold boundary",
            Box::new(|m, c| m[..(c - 1) as usize].to_vec()),
            false,
        ),
        (
            "a covered row rewritten",
            Box::new(|mut m, c| {
                let at = (c - 1) as usize;
                m[at].content = format!("{} (rewritten)", m[at].content);
                m
            }),
            false,
        ),
    ];
    for (name, rewrite, keep) in cases {
        // Fresh fold per case so rewrites do not compose.
        let sess = session_with(live_fixture());
        let window = Arc::new(Window::new(
            WindowConfig {
                context_window: 2_000,
                compact_ratio: 0.9,
                workspace_id: "ws".to_owned(),
                model_ref: "m".to_owned(),
                ..WindowConfig::default()
            },
            sess.clone(),
            Arc::new(reasonix_tools::Registry::new()),
            Arc::new(Discard),
        ));
        window.set_session_path(&path.to_string_lossy());
        assert!(
            window
                .compact_to_projection(&request)
                .expect("fold")
                .installed
        );
        let covered = window.with_core(|core| core.compaction_state.projection.covered_count);
        for _ in 0..2 {
            sess.add(msg(Role::USER, &"t".repeat(80)));
            sess.add(msg(Role::ASSISTANT, &"r".repeat(120)));
        }
        let snapshot = sess.snapshot();
        sess.rewrite(rewrite(snapshot, covered), "test_rewrite");
        window.revalidate_projection();
        assert_eq!(has_projection(&window), keep, "{name}");
    }
}

// Go: TestCompactionSidecarIsOutsideTheSessionWriteAuthority — a
// controller with no write authority for the path must not install and
// publish a fold the owner already wrote.
#[test]
fn sidecar_is_outside_the_session_write_authority() {
    let dir = scratch("authority");
    let path = dir.join("session.jsonl");
    let owner_msgs = || {
        let mut m = vec![msg(Role::SYSTEM, "sys"), msg(Role::USER, "task")];
        for _ in 0..6 {
            m.push(msg(Role::ASSISTANT, &"work line\n".repeat(200)));
            m.push(msg(Role::USER, "continue"));
        }
        m
    };

    // The stale writer binds before anything is folded.
    let stale_sess = session_with(owner_msgs());
    stale_sess.require_write_authority();
    let stale = sidecar_window("ws", "p/m", stale_sess.clone(), &path.to_string_lossy());

    let owner = sidecar_window(
        "ws",
        "p/m",
        session_with(owner_msgs()),
        &path.to_string_lossy(),
    );
    let owner_request = FoldRequest {
        trigger: trigger_label::MANUAL.to_owned(),
        instructions: String::new(),
        scope: FoldScope {
            ignore_threshold: true,
            ignore_economics: true,
            ignore_amount: true,
        },
        must_free: false,
    };
    assert!(
        owner
            .compact_to_projection(&owner_request)
            .expect("owner fold")
            .installed
    );
    let owned = load_compaction_state(&path)
        .expect("load")
        .expect("owned sidecar");
    assert!(
        !owned.projection.summary_hash.is_empty() && owned.projection.covered_count > 0,
        "owner fold not published"
    );
    let owner_version = owner.current_projection_version();
    let owned_generation = owned.generation;

    // The stale controller's transcript save is refused (fail closed).
    let stale_path = path.to_string_lossy().into_owned();
    assert!(
        stale_sess.save(&stale_path).is_err(),
        "setup: the stale session was allowed to write the transcript"
    );

    let result = stale.compact_to_projection(&owner_request);
    assert!(
        result.is_err(),
        "a controller with no write authority installed and published a fold"
    );

    let after = load_compaction_state(&path)
        .expect("reload")
        .expect("sidecar");
    assert_eq!(after.projection.summary_hash, owned.projection.summary_hash);
    assert_eq!(after.projection.projection_version, owner_version);
    assert_eq!(after.generation, owned_generation);
}

// Go: InvalidateProjection — memory and disk drop together; a revalidate
// on an empty projection is a no-op.
#[test]
fn invalidate_drops_disk_and_memory() {
    let dir = scratch("invalidate");
    let path = dir.join("s.jsonl");
    let sess = session_with(live_fixture());
    let window = Arc::new(Window::new(
        WindowConfig {
            context_window: 2_000,
            compact_ratio: 0.9,
            workspace_id: "ws".to_owned(),
            model_ref: "m".to_owned(),
            ..WindowConfig::default()
        },
        sess.clone(),
        Arc::new(reasonix_tools::Registry::new()),
        Arc::new(Discard),
    ));
    window.set_session_path(&path.to_string_lossy());
    let request = FoldRequest {
        trigger: trigger_label::MANUAL.to_owned(),
        instructions: String::new(),
        scope: FoldScope {
            ignore_threshold: true,
            ignore_economics: true,
            ignore_amount: true,
        },
        must_free: false,
    };
    assert!(
        window
            .compact_to_projection(&request)
            .expect("fold")
            .installed
    );
    assert!(load_compaction_state(&path).expect("load").is_some());
    window.invalidate_projection();
    assert!(window.with_core(|core| core.compaction_state.projection.messages.is_empty()));
    assert!(load_compaction_state(&path).expect("load").is_none());
    // Revalidating nothing is not an error and touches nothing.
    window.revalidate_projection();
    assert!(load_compaction_state(&path).expect("load").is_none());
}

// Go: FoldNotReusedNotice — both codes speak, anything else stays silent.
#[test]
fn fold_not_reused_notice_names_both_drops() {
    let (code, text) = fold_not_reused_notice(drop_reason::HISTORY).expect("history");
    assert_eq!(code, "fold_not_reused_history");
    assert!(text.contains("has changed since"));
    let (code, text) = fold_not_reused_notice(drop_reason::LINEAGE).expect("lineage");
    assert_eq!(code, "fold_not_reused_lineage");
    assert!(text.contains("another model or workspace"));
    assert!(fold_not_reused_notice(drop_reason::NONE).is_none());
    assert!(fold_not_reused_notice("").is_none());
}
