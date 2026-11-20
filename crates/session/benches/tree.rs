//! OPT-014 (docs/optimization-plan.md): tree-build benchmarks, ported from
//! Go `hub_projects_benchmark_test.go` (`BenchmarkProjectTree100Roots`)
//! plus the 500-session synthetic tree the wave asks for.
//!
//! `tree_build_optimized` is the shipping path (one pass over the rows,
//! one sort); `tree_build_literal` is the naive shape the optimization
//! replaced — per-row re-sorting and per-row title-cache loads — so the
//! two numbers stay comparable on the same fixture. Go's own baseline is
//! recorded separately (go test -bench BenchmarkProjectTree100Roots);
//! when it cannot run locally the numbers are marked unquantified.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use criterion::{Criterion, criterion_group, criterion_main};

use reasonix_session::hub::{Hub, HubPaths};
use reasonix_session::model::WorkspaceTreeSnapshot;
use reasonix_session::tree::TreeInputs;
use reasonix_session::workspacelist;

/// Go testenv.TempDir: one disposable fixture root per bench. The target
/// directory stands in for the system temp path, which this environment
/// denies (same approach as the crate's tests).
struct Fixture(tempfile::TempDir);

impl Fixture {
    fn new() -> Self {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("target")
            .join("test-tmp");
        fs::create_dir_all(&base).unwrap();
        Self(
            tempfile::Builder::new()
                .prefix(".tmpbench")
                .tempdir_in(&base)
                .unwrap(),
        )
    }

    fn state_root(&self) -> PathBuf {
        self.0.path().join("state")
    }
}

/// Writes the remembered list the way the desktop does.
fn remember(inputs: &TreeInputs, dirs: &[String]) {
    let body = serde_json::json!({
        "paths": dirs,
        "launch": dirs.first().cloned().unwrap_or_default(),
    })
    .to_string();
    fs::write(&inputs.workspaces_path, body).unwrap();
}

fn tree_inputs(fixture: &Fixture) -> TreeInputs {
    let state_root = fixture.state_root();
    TreeInputs {
        workspaces_path: workspacelist::workspaces_file(&state_root),
        state_root,
    }
}

/// Go BenchmarkProjectTree100Roots: 100 remembered project roots, no
/// sessions — the cost of the workspace walk itself.
fn fixture_100_roots() -> (Fixture, Vec<String>) {
    let fixture = Fixture::new();
    let inputs = tree_inputs(&fixture);
    let roots: Vec<String> = (0..100)
        .map(|i| {
            let dir = fixture
                .state_root()
                .join(format!("project-{i:03}"))
                .to_string_lossy()
                .into_owned();
            fs::create_dir_all(&dir).unwrap();
            dir
        })
        .collect();
    remember(&inputs, &roots);
    (fixture, roots)
}

/// The wave's 500-session synthetic tree: one remembered project whose
/// transcript directory holds 500 sidecar-less sessions.
fn fixture_500_sessions() -> (Fixture, String) {
    let fixture = Fixture::new();
    let inputs = tree_inputs(&fixture);
    let root = fixture.state_root().join("project-busy");
    let dir =
        reasonix_session::paths::session_dir_for(&fixture.state_root(), &root.to_string_lossy());
    fs::create_dir_all(&dir).unwrap();
    for i in 0..500 {
        let msg = serde_json::json!({"role": "user", "content": "history"});
        fs::write(
            dir.join(format!("20260101-{i:06}-000-model.jsonl")),
            format!("{msg}\n"),
        )
        .unwrap();
    }
    remember(&inputs, &[root.to_string_lossy().into_owned()]);
    (fixture, root.to_string_lossy().into_owned())
}

/// Go BenchmarkProjectTree100Roots: cold read measured separately, then
/// the steady-state loop.
fn bench_100_roots(c: &mut Criterion) {
    let (fixture, _roots) = fixture_100_roots();
    let hub = Hub::new(HubPaths {
        state_root: fixture.state_root(),
    });
    let start = Instant::now();
    let _cold = hub.tree();
    let cold = start.elapsed();
    let mut group = c.benchmark_group("tree_100_roots");
    group.bench_function("hub_tree", |b| {
        b.iter(|| {
            let tree = hub.tree();
            assert_eq!(tree.0.len(), 100);
        })
    });
    group.finish();
    eprintln!("cold first /tree (100 roots, no sessions): {cold:?}");
}

/// 500 sessions through the shipping read model.
fn bench_500_sessions(c: &mut Criterion) {
    let (fixture, _root) = fixture_500_sessions();
    let inputs = tree_inputs(&fixture);
    let mut group = c.benchmark_group("tree_500_sessions");
    group.bench_function("optimized", |b| {
        b.iter(|| {
            let tree: WorkspaceTreeSnapshot = reasonix_session::tree::snapshot(&inputs);
            assert_eq!(tree.0[0].total, 500);
        })
    });
    group.bench_function("literal", |b| {
        b.iter(|| {
            let tree = literal_snapshot(&inputs);
            assert_eq!(tree.0[0].total, 500);
        })
    });
    group.finish();
}

/// The shape OPT-014 replaced, kept so the two numbers stay comparable:
/// every row reloads the title sidecar (the listing never shares one
/// snapshot), rows sort once per directory before the merged list sorts
/// again.
fn literal_snapshot(inputs: &TreeInputs) -> WorkspaceTreeSnapshot {
    let roots = reasonix_session::tree::remembered_roots(inputs);
    let mut nodes = Vec::new();
    for root in &roots {
        let dir = reasonix_session::paths::session_dir_for(&inputs.state_root, &root.dir);
        let listed = reasonix_session::listing::list_native_sessions(&dir).unwrap_or_default();
        let mut rows = Vec::new();
        for listed in &listed {
            let path = Path::new(&listed.info.path);
            let base = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let modified = reasonix_session::listing::session_content_mod_time(path);
            // The literal shape loads the sidecar fresh for every row.
            let titles = reasonix_session::title::TitleCache::new(&dir).snapshot();
            let title = reasonix_session::title::session_title(
                &titles,
                &base,
                &listed.info.preview,
                modified.unix_nanos(),
                &listed.info.custom_title,
            );
            rows.push(reasonix_session::model::SessionRow {
                name: base.strip_suffix(".jsonl").unwrap_or(&base).to_string(),
                path: listed.info.path.clone(),
                title,
                turns: listed.info.turns,
                current: false,
                unread: listed.info.unread,
                modified,
            });
        }
        // The literal shape sorts each directory's rows before the merged
        // rows sort once more.
        rows.sort_by(|a, b| {
            b.modified
                .get()
                .cmp(&a.modified.get())
                .then(a.path.cmp(&b.path))
        });
        let total = i64::try_from(listed.len()).unwrap_or(i64::MAX);
        nodes.push(reasonix_session::model::WorkspaceNode {
            root: root.dir.clone(),
            name: reasonix_session::paths::root_name(&root.dir),
            total,
            archived: 0,
            remaining: 0,
            sessions: rows,
            ..reasonix_session::model::WorkspaceNode::default()
        });
    }
    WorkspaceTreeSnapshot(nodes)
}

criterion_group!(benches, bench_100_roots, bench_500_sessions);
criterion_main!(benches);
