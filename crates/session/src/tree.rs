//! Workspace tree read model: Go serve `hub_projects.go`
//! (`groupWorkspaces`) and the no-window, no-panes slice of `hub_tree.go`
//! `tree` + `hub_tree_page.go` `projectPage` — the `GET /tree` body.
//!
//! Tolerance is the reference's (OPT-006): one unreadable session directory
//! is skipped with a warn line, a missing workspace root is a `missing`
//! node with an empty list, and nothing ever fails the snapshot. Workspace
//! identity folds through `reasonix_state`'s canonical session paths
//! (OPT-007), so two spellings of one folder group into one node.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use reasonix_state::sessionstore::listing::list_sessions;
use reasonix_state::sessionstore::session::canonical_session_path;
use reasonix_state::store::naming::is_subagent_transcript_name;

use crate::listing::{
    SessionInfo, compare_sessions, hidden_recovery_copy, session_content_mod_time,
};
use crate::model::{SessionRow, WorkspaceNode, WorkspaceTreeSnapshot};
use crate::paths::{delivery_worktree_dir, root_name, session_dir_for};
use crate::title::{TitleCache, TitleSnapshot, session_title};
use crate::workspacelist;

/// One candidate folder of the sidebar: where it is and whether the user
/// ever chose it (Go serve `rootRef`).
#[derive(Debug, Clone, PartialEq)]
pub struct RootRef {
    pub dir: String,
    pub remembered: bool,
}

/// One grouped sidebar folder (Go serve `workspaceProject`): the root a
/// person sees, every directory that folds into it, and whether any of
/// them was remembered.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceGroup {
    pub root: String,
    pub roots: Vec<String>,
    pub remembered: bool,
}

/// Everything a snapshot needs. Go reaches these through Hub fields;
/// an explicit record keeps the read side constructible without the hub.
#[derive(Debug, Clone)]
pub struct TreeInputs {
    /// State root (Go `MemoryUserDir`): session dirs and worktrees hang
    /// off it.
    pub state_root: PathBuf,
    /// The `serve-workspaces.json` path (Go serve `workspacesPath()`).
    pub workspaces_path: PathBuf,
}

impl TreeInputs {
    /// The environment-resolved inputs (Go `processRoots()`).
    pub fn from_process_roots() -> Option<Self> {
        let state_root = crate::paths::state_root()?;
        Some(Self {
            workspaces_path: workspacelist::workspaces_file(&state_root),
            state_root,
        })
    }
}

/// Go serve `projectRoot`: a managed worktree checkout maps back to its
/// source project. The worktree machine is not migrated, so the deep
/// gitdir mapping cannot resolve here — a checkout stays its own root,
/// which is the identity this build can ever list.
pub fn project_root(root: &str, managed_root: &str) -> String {
    if !is_managed_path(root, managed_root) {
        return root.to_string();
    }
    root.to_string()
}

/// Go serve `projectIdentity`: the canonical session-path key of a
/// project root (OPT-007: one key, reused, not re-derived).
pub fn project_identity(root: &str, managed_root: &str) -> String {
    canonical_session_path(&project_root(root, managed_root))
}

/// Go `worktree.IsManagedPath`: whether `path` belongs to Reasonix's
/// durable worktree storage. Lexical UI identity, not an authorization
/// boundary; case-sensitive exactly as the Go check is.
pub fn is_managed_path(path: &str, managed_root: &str) -> bool {
    let path = path.trim();
    let managed_root = managed_root.trim();
    if path.is_empty() || managed_root.is_empty() {
        return false;
    }
    let Ok(abs_path) = std::path::absolute(path) else {
        return false;
    };
    let Ok(abs_managed) = std::path::absolute(managed_root) else {
        return false;
    };
    let Ok(rel) = abs_path.strip_prefix(&abs_managed) else {
        return false;
    };
    // The storage root itself is not one of its managed checkouts.
    !rel.as_os_str().is_empty()
}

/// Go serve `groupWorkspaces`: fold every candidate folder into sidebar
/// folders by canonical project identity. A worktree checkout and its
/// source share one node whose root is the spelled-out source.
pub fn group_workspaces(refs: &[RootRef], managed_root: &str) -> Vec<WorkspaceGroup> {
    let mut index: std::collections::HashMap<String, usize> = Default::default();
    let mut aliases: std::collections::HashMap<String, String> = Default::default();
    let mut sources: std::collections::HashMap<&str, String> = Default::default();
    for ref_ in refs {
        let source = project_root(&ref_.dir, managed_root);
        if source == ref_.dir {
            aliases.insert(canonical_session_path(&source), ref_.dir.clone());
        }
        sources.insert(ref_.dir.as_str(), source);
    }
    let mut out: Vec<WorkspaceGroup> = Vec::new();
    for ref_ in refs {
        let source = &sources[ref_.dir.as_str()];
        let identity = canonical_session_path(source);
        let mut root = source.clone();
        if let Some(alias) = aliases.get(&identity) {
            root = alias.clone();
        }
        let at = match index.get(&identity) {
            Some(&at) => at,
            None => {
                out.push(WorkspaceGroup {
                    root: root.clone(),
                    roots: vec![root.clone()],
                    remembered: false,
                });
                index.insert(identity, out.len() - 1);
                out.len() - 1
            }
        };
        let group = &mut out[at];
        group.remembered = group.remembered || ref_.remembered;
        for dir in [source.clone(), ref_.dir.clone()] {
            if !group.roots.contains(&dir) {
                group.roots.push(dir);
            }
        }
    }
    // A group whose visible root is gone falls back to a directory that
    // still exists, so a renamed project keeps its conversations listed.
    for group in &mut out {
        if !project_directory_exists(&group.root)
            && let Some(live) = group.roots.iter().find(|r| project_directory_exists(r))
        {
            group.root = live.clone();
        }
    }
    out
}

fn project_directory_exists(root: &str) -> bool {
    std::fs::metadata(root)
        .map(|info| info.is_dir())
        .unwrap_or(false)
}

/// The remembered workspace list as tree inputs (Go serve `roots()` with
/// no runtimes: only folders someone picked).
pub fn remembered_roots(inputs: &TreeInputs) -> Vec<RootRef> {
    workspacelist::workspaces(&inputs.workspaces_path)
        .into_iter()
        .map(|dir| RootRef {
            dir,
            remembered: true,
        })
        .collect()
}

/// The `GET /tree` snapshot (Go serve `tree`, the no-window slice): every
/// remembered workspace with the sessions saved under it.
pub fn snapshot(inputs: &TreeInputs) -> WorkspaceTreeSnapshot {
    let refs = remembered_roots(inputs);
    let managed_root = delivery_worktree_dir(&inputs.state_root)
        .to_string_lossy()
        .into_owned();
    let groups = group_workspaces(&refs, &managed_root);
    let mut out = Vec::with_capacity(groups.len());
    for group in groups {
        let listed = group_sessions(inputs, &group);
        let mut node = WorkspaceNode {
            root: group.root.clone(),
            name: root_name(&group.root),
            workspace_roots: group.roots.clone(),
            isolated: is_managed_path(&group.root, &managed_root),
            missing: !project_directory_exists(&group.root),
            // Open means a pane is driving this folder; no pane exists
            // before the runtime machine migrates.
            open: false,
            remembered: group.remembered,
            ..WorkspaceNode::default()
        };
        let (rows, total, archived, said) = page_rows(&listed);
        node.sessions = rows;
        node.total = total;
        node.archived = archived;
        // Without a window every matching row is carried, so nothing
        // remains (Go sends this always, so a client can tell a kernel
        // with nothing left from one that predates the field).
        node.remaining = 0;
        // A folder the user picked is always listed; one a pane merely
        // opened needs a first turn before it earns a row of its own.
        if !group.remembered && !said {
            continue;
        }
        out.push(node);
    }
    WorkspaceTreeSnapshot(out)
}

/// Every listed session across the group's directories, most recent first
/// (Go `projectPage`'s listing pass, deduplicated by directory).
fn group_sessions(inputs: &TreeInputs, group: &WorkspaceGroup) -> Vec<SessionInfo> {
    let mut listed: Vec<SessionInfo> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for root in &group.roots {
        let dir = session_dir_for(&inputs.state_root, root);
        if !seen.insert(dir.to_string_lossy().into_owned()) {
            continue;
        }
        match list_sessions(&dir) {
            Ok(rows) => listed.extend(rows),
            Err(err) => {
                // One unreadable directory costs its own rows, not the
                // tree (OPT-006).
                tracing::warn!(dir = %dir.display(), err = %err, "list tree sessions");
            }
        }
    }
    // One sort, once, over the merged rows (OPT-014).
    listed.sort_by(compare_sessions);
    listed
}

/// Rows, counts, and the said flag for one group (Go `projectPage`'s row
/// pass, the no-window slice).
fn page_rows(listed: &[SessionInfo]) -> (Vec<SessionRow>, i64, i64, bool) {
    let mut rows: Vec<SessionRow> = Vec::with_capacity(listed.len());
    let mut total: i64 = 0;
    let mut archived: i64 = 0;
    let mut said = false;
    // One title sidecar read per project directory (Go `titleCacheFor`
    // + one `snapshot()` outside the row loop): titles are file-name
    // keyed and unique only within a project.
    let mut titles_by_dir: std::collections::HashMap<String, TitleSnapshot> = Default::default();
    for info in listed {
        let base = file_name(&info.path);
        // A subagent transcript only means anything through the session
        // that spawned it.
        if is_subagent_transcript_name(Path::new(&base).as_os_str()) {
            continue;
        }
        if hidden_recovery_copy(info, "") {
            continue;
        }
        // A recovered copy is its own row until the tree can fold copies
        // under their lead (the row shape has no copies field yet).
        total += 1;
        archived += i64::from(info.archived);
        said = said || info.turns > 0;
        let dir = Path::new(&info.path)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let titles = titles_by_dir
            .entry(dir.clone())
            .or_insert_with(|| TitleCache::new(&dir).snapshot());
        let modified = session_content_mod_time(Path::new(&info.path));
        rows.push(SessionRow {
            name: base.strip_suffix(".jsonl").unwrap_or(&base).to_string(),
            path: info.path.clone(),
            title: session_title(
                titles,
                &base,
                &info.preview,
                modified.unix_nanos(),
                &info.custom_title,
            ),
            turns: info.turns,
            current: false,
            unread: info.unread,
            modified,
        });
    }
    (rows, total, archived, said)
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// Where one workspace's transcripts live (Go serve `SessionDirFor`).
pub fn session_dir(inputs: &TreeInputs, workspace_root: &str) -> PathBuf {
    session_dir_for(&inputs.state_root, workspace_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspacelist::WorkspaceList;
    use std::fs;

    fn inputs(state_root: &Path) -> TreeInputs {
        TreeInputs {
            state_root: state_root.to_path_buf(),
            workspaces_path: workspacelist::workspaces_file(state_root),
        }
    }

    fn remember(inputs: &TreeInputs, dirs: &[&str]) {
        let list = WorkspaceList {
            paths: dirs.iter().map(|d| d.to_string()).collect(),
            launch: dirs.first().map(|d| d.to_string()).unwrap_or_default(),
        };
        write_list(inputs, &list);
    }

    fn write_list(inputs: &TreeInputs, list: &WorkspaceList) {
        let body = serde_json::json!({"paths": list.paths, "launch": list.launch}).to_string();
        fs::write(&inputs.workspaces_path, body).unwrap();
    }

    fn add_session(inputs: &TreeInputs, root: &str, stem: &str) -> PathBuf {
        let dir = session_dir(inputs, root);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{stem}.jsonl"));
        let msg = serde_json::json!({"role": "user", "content": "find me"});
        fs::write(&path, format!("{msg}\n")).unwrap();
        path
    }

    #[test]
    fn tree_sends_an_empty_list_for_a_folder_without_sessions() {
        // Port of TestTreeSendsAnEmptyListForAFolderWithoutSessions: the
        // frontend iterates rows, so null would blank the window.
        let tmp = crate::test_support::tempdir();
        let inputs = inputs(tmp.path());
        let root = tmp.path().join("project-a");
        fs::create_dir_all(&root).unwrap();
        remember(&inputs, &[&root.to_string_lossy()]);
        let tree = snapshot(&inputs);
        assert_eq!(tree.0.len(), 1);
        assert_eq!(tree.0[0].sessions, Vec::<SessionRow>::new());
        let json = serde_json::to_string(&tree).unwrap();
        assert!(json.contains(r#""sessions":[]"#), "{json}");
    }

    #[test]
    fn missing_workspace_root_is_a_missing_node_with_no_rows() {
        let tmp = crate::test_support::tempdir();
        let inputs = inputs(tmp.path());
        remember(&inputs, &[&tmp.path().join("gone").to_string_lossy()]);
        let tree = snapshot(&inputs);
        assert_eq!(tree.0.len(), 1);
        assert!(tree.0[0].missing);
        assert!(tree.0[0].remembered);
    }

    #[test]
    fn remembered_order_is_the_node_order() {
        let tmp = crate::test_support::tempdir();
        let inputs = inputs(tmp.path());
        let b = tmp.path().join("b");
        let a = tmp.path().join("a");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        remember(&inputs, &[&b.to_string_lossy(), &a.to_string_lossy()]);
        let tree = snapshot(&inputs);
        let roots: Vec<&str> = tree.0.iter().map(|n| n.root.as_str()).collect();
        assert_eq!(roots.len(), 2);
        assert!(roots[0].ends_with("b"), "{roots:?}");
        assert!(roots[1].ends_with("a"), "{roots:?}");
    }

    #[test]
    fn sessions_of_a_workspace_list_under_its_project_dir() {
        let tmp = crate::test_support::tempdir();
        let inputs = inputs(tmp.path());
        let root = tmp.path().join("demo");
        fs::create_dir_all(&root).unwrap();
        remember(&inputs, &[&root.to_string_lossy()]);
        add_session(
            &inputs,
            &root.to_string_lossy(),
            "20260101-000000-000-model",
        );
        let tree = snapshot(&inputs);
        assert_eq!(tree.0.len(), 1);
        assert_eq!(tree.0[0].total, 1);
        assert_eq!(tree.0[0].sessions.len(), 1);
        assert_eq!(tree.0[0].sessions[0].name, "20260101-000000-000-model");
        // No branch sidecar: the row has no recorded counts, so the title
        // is the preview fallback, cut like Go's previewTitle (titles come
        // from sidecars only).
        assert_eq!(
            tree.0[0].sessions[0].title,
            "History is being indexed — 20260101-000000-000-..."
        );
        assert!(!tree.0[0].sessions[0].modified.is_zero());
    }

    #[test]
    fn one_folder_spelled_two_ways_groups_into_one_node() {
        // OPT-007: grouping keys on the state crate's canonical identity.
        if !cfg!(windows) {
            return;
        }
        let tmp = crate::test_support::tempdir();
        let inputs = inputs(tmp.path());
        let root = tmp.path().join("Demo");
        fs::create_dir_all(&root).unwrap();
        let lower = root.to_string_lossy().to_lowercase();
        remember(&inputs, &[&root.to_string_lossy(), &lower]);
        let tree = snapshot(&inputs);
        assert_eq!(tree.0.len(), 1, "{:?}", tree.0);
        assert_eq!(tree.0[0].workspace_roots.len(), 2, "{:?}", tree.0[0]);
    }

    #[test]
    fn unreadable_session_directory_yields_empty_rows_not_a_failure() {
        let tmp = crate::test_support::tempdir();
        let inputs = inputs(tmp.path());
        let root = tmp.path().join("demo");
        fs::create_dir_all(&root).unwrap();
        remember(&inputs, &[&root.to_string_lossy()]);
        // The sessions dir for a folder that was never a transcript store
        // is a plain missing directory.
        let tree = snapshot(&inputs);
        assert_eq!(tree.0[0].sessions, Vec::<SessionRow>::new());
    }

    #[test]
    fn subagent_transcripts_stay_out_of_the_tree() {
        let tmp = crate::test_support::tempdir();
        let inputs = inputs(tmp.path());
        let root = tmp.path().join("demo");
        fs::create_dir_all(&root).unwrap();
        remember(&inputs, &[&root.to_string_lossy()]);
        let dir = session_dir(&inputs, &root.to_string_lossy());
        fs::create_dir_all(&dir).unwrap();
        let msg = serde_json::json!({"role": "user", "content": "find me"});
        fs::write(dir.join("subagent-abc123.jsonl"), format!("{msg}\n")).unwrap();
        let tree = snapshot(&inputs);
        assert_eq!(tree.0[0].total, 0);
        assert!(tree.0[0].sessions.is_empty());
    }

    #[test]
    fn tree_includes_foreign_sessions_and_omits_empty_ones() {
        let tmp = crate::test_support::tempdir();
        let inputs = inputs(tmp.path());
        let root = tmp.path().join("demo");
        fs::create_dir_all(&root).unwrap();
        remember(&inputs, &[&root.to_string_lossy()]);
        add_session(
            &inputs,
            &root.to_string_lossy(),
            "20260101-000000-000-model",
        );
        let id = "0123456789abcdef0123456789abcdef";
        let empty_id = "1123456789abcdef0123456789abcdef";
        let sessions = session_dir(&inputs, &root.to_string_lossy());
        let foreign = crate::test_support::write_v4_fixture(
            &sessions,
            id,
            vec![serde_json::json!({
                "kind": "message/complete",
                "payload": {"message": {"id": "m1", "role": "user", "content": "legacy preview"}}
            })],
        );
        crate::test_support::write_v4_fixture(&sessions, empty_id, Vec::new());

        let tree = snapshot(&inputs);
        assert_eq!(tree.0.len(), 1);
        assert_eq!(tree.0[0].total, 2);
        assert_eq!(tree.0[0].sessions.len(), 2);
        let row = tree.0[0]
            .sessions
            .iter()
            .find(|row| row.path == foreign.to_string_lossy())
            .expect("foreign tree row");
        assert_eq!(row.name, format!("v4-{id}"));
        assert_eq!(row.turns, 1);
    }

    #[test]
    fn group_folds_aliases_into_the_existing_root() {
        let tmp = crate::test_support::tempdir();
        let root = tmp.path().join("demo");
        fs::create_dir_all(&root).unwrap();
        let managed = tmp.path().join("worktrees");
        fs::create_dir_all(&managed).unwrap();
        let groups = group_workspaces(
            &[
                RootRef {
                    dir: root.to_string_lossy().into_owned(),
                    remembered: true,
                },
                RootRef {
                    dir: root.to_string_lossy().into_owned(),
                    remembered: false,
                },
            ],
            &managed.to_string_lossy(),
        );
        assert_eq!(groups.len(), 1);
        assert!(groups[0].remembered);
    }

    #[test]
    fn managed_worktree_roots_are_flagged_isolated() {
        let tmp = crate::test_support::tempdir();
        let managed = tmp.path().join("worktrees");
        let checkout = managed.join("some-repo-main");
        fs::create_dir_all(&checkout).unwrap();
        assert!(is_managed_path(
            &checkout.to_string_lossy(),
            &managed.to_string_lossy()
        ));
        assert!(!is_managed_path(
            &tmp.path().join("demo").to_string_lossy(),
            &managed.to_string_lossy()
        ));
        assert!(!is_managed_path("", &managed.to_string_lossy()));
        // The storage root itself is not one of its checkouts.
        assert!(!is_managed_path(
            &managed.to_string_lossy(),
            &managed.to_string_lossy()
        ));
    }

    #[test]
    fn project_identity_reuses_the_state_canonical_key() {
        let tmp = crate::test_support::tempdir();
        let root = tmp.path().join("demo");
        fs::create_dir_all(&root).unwrap();
        let identity = project_identity(&root.to_string_lossy(), "");
        assert_eq!(identity, canonical_session_path(&root.to_string_lossy()));
    }
}
