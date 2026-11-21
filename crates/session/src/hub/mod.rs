//! The hub (Go serve `hub.go` Hub, docs/contract.md §4.1): the frozen read
//! API every frontend and host consumes, plus the write side that opens and
//! publishes runtimes (`open`, this module's child). What runtimes share —
//! the workspace tree, the remembered list, the one-session-one-runtime
//! rule — answers here; what they drive lands with its machine, refusing
//! with a typed error until then (close is still such a machine).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use reasonix_state::sessionstore::session::canonical_session_path;

use crate::controller::{BroadcasterHandle, Controller};
use crate::listing::session_rows;
use crate::model::{RuntimeId, RuntimeSummary, SessionRow, WorkspaceTreeSnapshot};
use crate::paths::{root_name, session_dir_for};
use crate::title::TitleCache;
use crate::tree::{self, TreeInputs};
use crate::turns::TurnController;
use crate::workspacelist;

pub mod open;

pub use self::open::{HubOptions, RuntimeKit, RuntimeKitFactory, RuntimeSpec};

/// Paths the hub reads. Go resolves them through `config.Roots()`; explicit
/// inputs keep the read side constructible without the config crate.
#[derive(Debug, Clone)]
pub struct HubPaths {
    /// State root (Go `MemoryUserDir`): holds `serve-workspaces.json`,
    /// `projects/<slug>/sessions`, and the global `sessions` store.
    pub state_root: PathBuf,
}

impl HubPaths {
    /// The state root the process environment names (Go
    /// `processRoots()`); `None` when no OS location can be resolved.
    pub fn from_process_roots() -> Option<Self> {
        Some(Self {
            state_root: crate::paths::state_root()?,
        })
    }

    /// Go serve `workspacesPath()`.
    pub fn workspaces_path(&self) -> PathBuf {
        workspacelist::workspaces_file(&self.state_root)
    }

    fn tree_inputs(&self) -> TreeInputs {
        TreeInputs {
            state_root: self.state_root.clone(),
            workspaces_path: self.workspaces_path(),
        }
    }
}

/// One published runtime: what the hub must remember to read it, and the
/// write side the runtime's own routes drive (Go `Runtime`).
struct RuntimeSlot {
    id: RuntimeId,
    controller: Arc<dyn Controller>,
    broadcaster: Option<Arc<dyn BroadcasterHandle>>,
    /// The machine-built controller's turn group. A runtime the host
    /// adopted carries its write side itself; `None` there.
    turn: Option<Arc<TurnController>>,
}

/// The workspace-tree hub (Go serve `Hub`, the read half).
pub struct Hub {
    paths: HubPaths,
    /// The installed open machine (Go `HubOptions`' construction
    /// collaborators); `None` leaves open_runtime refusing.
    machine: Option<HubOptions>,
    // Write-side anchor: open/publish appends here; reads iterate in
    // insertion order, which is Go's `h.order`.
    runtimes: RwLock<Vec<RuntimeSlot>>,
    seq: AtomicU64,
}

impl Hub {
    /// A hub over explicit paths, with no open machine: every open request
    /// is refused with a typed error. Go's `NewHub(HubOptions{})` would
    /// build from config; the config→client edge lives above this crate,
    /// so a machine-less hub has nothing honest to construct from.
    pub fn new(paths: HubPaths) -> Self {
        Self {
            paths,
            machine: None,
            runtimes: RwLock::new(Vec::new()),
            seq: AtomicU64::new(0),
        }
    }

    /// A hub over explicit paths with the open machine installed (Go
    /// `NewHub(HubOptions{...})` carrying the construction collaborators).
    pub fn with_options(paths: HubPaths, options: HubOptions) -> Self {
        Self {
            paths,
            machine: Some(options),
            runtimes: RwLock::new(Vec::new()),
            seq: AtomicU64::new(0),
        }
    }

    /// A hub over the process environment's roots (Go `NewHub` reaching
    /// `config.Roots()`); `None` when the OS cannot name a state root.
    pub fn from_process_roots() -> Option<Self> {
        HubPaths::from_process_roots().map(Self::new)
    }

    pub fn paths(&self) -> &HubPaths {
        &self.paths
    }

    /// Frozen (docs/contract.md §4.1): the whole sidebar in one snapshot.
    pub fn tree(&self) -> WorkspaceTreeSnapshot {
        tree::snapshot(&self.paths.tree_inputs())
    }

    /// Frozen: the open runtimes in the order they were opened. The empty
    /// hub answers an empty list.
    pub fn runtimes(&self) -> Vec<RuntimeSummary> {
        self.runtimes
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|slot| runtime_summary(&slot.id, slot.controller.as_ref()))
            .collect()
    }

    /// Frozen: `GET /sessions` rows for one workspace's transcript
    /// directory. Exclusion and tolerance rules are http.md §7's; the hub
    /// has no pane of its own, so no row is `current`.
    pub fn sessions(&self, workspace: &Path) -> Vec<SessionRow> {
        let dir = session_dir_for(&self.paths.state_root, &workspace.to_string_lossy());
        if dir.as_os_str().is_empty() {
            return Vec::new();
        }
        let titles = TitleCache::new(&dir);
        session_rows(&dir, "", &titles)
    }

    /// Frozen: the runtime's event fan-out, when one is published. The
    /// broadcaster entity lives in serve; a hub with no such runtime
    /// answers `None`.
    pub fn broadcaster(&self, runtime: RuntimeId) -> Option<Arc<dyn BroadcasterHandle>> {
        self.runtimes
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|slot| slot.id == runtime)
            .and_then(|slot| slot.broadcaster.clone())
    }

    /// Go `findSession`: the runtime already driving `path`, compared by
    /// canonical session path (OPT-007) so two spellings of one transcript
    /// never bind two writers — binding a second writer to one transcript
    /// is what forks a recovery branch on every save.
    pub fn find_session(&self, path: &str) -> Option<RuntimeId> {
        let path = canonical_session_path(path);
        if path.is_empty() {
            return None;
        }
        self.runtimes
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|slot| canonical_session_path(&slot.controller.session_path()) == path)
            .map(|slot| slot.id.clone())
    }

    /// Registers a runtime the host already built (Go `hub.Adopt`, the
    /// host half of adoption). The hub never builds one here: open_runtime
    /// is the construction machine.
    pub fn adopt(
        &self,
        id: RuntimeId,
        controller: Arc<dyn Controller>,
        broadcaster: Option<Arc<dyn BroadcasterHandle>>,
    ) {
        self.runtimes
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .push(RuntimeSlot {
                id,
                controller,
                broadcaster,
                turn: None,
            });
    }

    /// Go `nextID`: mints `"r"+seq` in publish order.
    fn next_id(&self) -> RuntimeId {
        let n = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        RuntimeId(format!("r{n}"))
    }
}

/// Go `rt.view()`: the wire view of one runtime, named by its workspace.
fn runtime_summary(id: &RuntimeId, controller: &dyn Controller) -> RuntimeSummary {
    let root = controller.workspace_root();
    RuntimeSummary {
        id: id.0.clone(),
        base: format!("/rt/{}", id.0),
        root: root.clone(),
        name: root_name(&root),
        session_path: controller.session_path(),
        host: String::new(),
        read_only: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{HubRefusal, OpenRuntimeRequest};

    /// A controller pinned to fixed answers (Go test doubles of
    /// `control.SessionAPI`).
    struct FixedController {
        session_path: String,
        session_dir: String,
        workspace_root: String,
    }

    impl Controller for FixedController {
        fn session_path(&self) -> String {
            self.session_path.clone()
        }
        fn session_dir(&self) -> String {
            self.session_dir.clone()
        }
        fn workspace_root(&self) -> String {
            self.workspace_root.clone()
        }
    }

    struct Broadcaster;

    impl BroadcasterHandle for Broadcaster {}

    fn hub_with_root(root: &Path) -> Hub {
        Hub::new(HubPaths {
            state_root: root.to_path_buf(),
        })
    }

    #[test]
    fn empty_hub_answers_empty_and_refuses_to_open() {
        let tmp = crate::test_support::tempdir();
        let hub = hub_with_root(tmp.path());
        assert!(hub.runtimes().is_empty());
        assert!(hub.tree().0.is_empty());
        assert!(hub.sessions(Path::new("C:\\nowhere")).is_empty());
        assert_eq!(
            hub.open_runtime(OpenRuntimeRequest::default()),
            Err(HubRefusal::UnsupportedInThisBuild)
        );
        assert!(hub.broadcaster(RuntimeId("r1".into())).is_none());
        assert!(hub.find_session("/d/s.jsonl").is_none());
    }

    #[test]
    fn tree_lists_remembered_workspaces_in_list_order() {
        let tmp = crate::test_support::tempdir();
        let hub = hub_with_root(tmp.path());
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(
            hub.paths().workspaces_path(),
            serde_json::json!({
                "paths": [b.to_string_lossy(), a.to_string_lossy()],
                "launch": a.to_string_lossy(),
            })
            .to_string(),
        )
        .unwrap();
        let tree = hub.tree();
        assert_eq!(tree.0.len(), 2);
        // The tree shows the remembered order, not the launch order.
        assert!(tree.0[0].root.ends_with("b"), "{:?}", tree.0[0].root);
        assert!(tree.0[0].remembered);
        assert_eq!(tree.0[0].sessions, Vec::<SessionRow>::new());
    }

    #[test]
    fn adopt_then_read_the_frozen_surface() {
        let tmp = crate::test_support::tempdir();
        let hub = hub_with_root(tmp.path());
        let ctrl = Arc::new(FixedController {
            session_path: r"C:\w\s.JSONL".to_string(),
            session_dir: r"C:\w".to_string(),
            workspace_root: r"C:\w".to_string(),
        });
        hub.adopt(
            RuntimeId("r1".into()),
            ctrl.clone(),
            Some(Arc::new(Broadcaster)),
        );
        let summaries = hub.runtimes();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, "r1");
        assert_eq!(summaries[0].base, "/rt/r1");
        assert_eq!(summaries[0].name, "w");
        assert_eq!(summaries[0].session_path, r"C:\w\s.JSONL");
        // findSession keys on the canonical path: any spelling finds it,
        // with the case fold riding the platform like the filesystem it
        // names.
        #[cfg(windows)]
        assert_eq!(
            hub.find_session("c:\\W\\s.jsonl"),
            Some(RuntimeId("r1".into()))
        );
        #[cfg(not(windows))]
        assert_eq!(
            hub.find_session("C:\\w\\s.JSONL"),
            Some(RuntimeId("r1".into()))
        );
        assert!(hub.broadcaster(RuntimeId("r1".into())).is_some());
        assert!(hub.broadcaster(RuntimeId("r2".into())).is_none());
        // An adopted runtime's write side is the host's, not the hub's.
        assert!(hub.turn_controller(RuntimeId("r1".into())).is_none());
    }

    #[test]
    fn sessions_of_an_unresolvable_workspace_answer_empty() {
        let tmp = crate::test_support::tempdir();
        let hub = hub_with_root(tmp.path());
        assert!(hub.sessions(Path::new("")).is_empty());
        let absent = tmp.path().join("absent").to_string_lossy().into_owned();
        assert!(hub.sessions(Path::new(&absent)).is_empty());
    }
}
