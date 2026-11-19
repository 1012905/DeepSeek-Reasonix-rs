//! The remembered workspace list (Go `internal/state/workspacelist`,
//! docs/contracts/session-format.md §19.1) — the read half.
//!
//! The write half (`Update`/`MergeOnce`/backup) lands with the tree
//! mutation routes; a read-side build never writes, so it cannot corrupt
//! the list it reads.

use serde::Deserialize;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Go `workspacelist.FileName`: the list file at the top of the state root.
pub const FILE_NAME: &str = "serve-workspaces.json";

/// Go `workspacelist.AdoptedSuffix`: marks a list already merged once, so
/// the merge happens exactly once.
pub const ADOPTED_SUFFIX: &str = ".adopted";

/// Go `workspacelist.List`: the remembered project order and the project a
/// launch opens first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorkspaceList {
    pub paths: Vec<String>,
    pub launch: String,
}

/// Wire shape of the object form. The bare-array legacy form is parsed
/// separately, mirroring Go's `data[0] == '['` branch.
#[derive(Deserialize, Default)]
struct WireList {
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    launch: String,
}

/// Go `workspacelist.Read`: parse the list at `path`. A missing file or an
/// empty path is an empty list; a present-but-unparsable file is an error —
/// including an empty file, exactly as Go's Unmarshal of zero bytes is.
pub fn read(path: &Path) -> io::Result<WorkspaceList> {
    if path.as_os_str().is_empty() {
        return Ok(WorkspaceList::default());
    }
    let data = match fs::read(path) {
        Ok(data) => data,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok(WorkspaceList::default());
        }
        Err(err) => return Err(err),
    };
    // Go TrimSpaces the bytes (Unicode-aware) before Unmarshal, which also
    // replaces invalid UTF-8 with U+FFFD; lossy decode + Unicode trim does
    // both.
    let text = String::from_utf8_lossy(&data);
    let text = text.trim();
    let mut list = if text.starts_with('[') {
        WorkspaceList {
            paths: serde_json::from_str::<Vec<String>>(text).map_err(read_error)?,
            launch: String::new(),
        }
    } else {
        let wire: WireList = serde_json::from_str(text).map_err(read_error)?;
        WorkspaceList {
            paths: wire.paths,
            launch: wire.launch,
        }
    };
    list.paths = normalize(&list.paths);
    if !list.paths.contains(&list.launch) {
        list.launch = list.paths.first().cloned().unwrap_or_default();
    }
    Ok(list)
}

/// Go serve `Workspaces()`: the remembered project order. A read error
/// answers an empty list (the sidebar must render), with a warn line.
pub fn workspaces(path: &Path) -> Vec<String> {
    match read(path) {
        Ok(list) => list.paths,
        Err(err) => {
            tracing::warn!(path = %path.display(), err = %err, "read workspace list");
            Vec::new()
        }
    }
}

/// Go serve `LaunchWorkspaces()`: the launch project first, the rest in
/// remembered order. The stored order itself never changes.
pub fn launch_workspaces(path: &Path) -> Vec<String> {
    let list = match read(path) {
        Ok(list) => list,
        Err(err) => {
            tracing::warn!(path = %path.display(), err = %err, "read workspace list");
            return Vec::new();
        }
    };
    if list.launch.is_empty() {
        return list.paths;
    }
    let mut out = Vec::with_capacity(list.paths.len());
    out.push(list.launch.clone());
    out.extend(list.paths.iter().filter(|p| **p != list.launch).cloned());
    out
}

/// Where the list lives for a state root (Go serve `workspacesPath()`).
pub fn workspaces_file(state_root: &Path) -> PathBuf {
    state_root.join(FILE_NAME)
}

/// Go Read's normalization: trim each entry, drop empties, keep the first
/// occurrence of a duplicate.
fn normalize(paths: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(paths.len());
    for dir in paths {
        let dir = dir.trim();
        if !dir.is_empty() && !out.iter().any(|have: &String| have == dir) {
            out.push(dir.to_string());
        }
    }
    out
}

/// Go surfaces `json.SyntaxError`/`json.UnmarshalTypeError`; the caller
/// only ever needs "unparsable", so any decode failure reads as invalid
/// data.
fn read_error(err: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("serve-workspaces.json");
        fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn missing_file_and_empty_path_are_empty_lists() {
        let tmp = crate::test_support::tempdir();
        assert_eq!(
            read(&tmp.path().join("absent.json")).unwrap(),
            WorkspaceList::default()
        );
        assert_eq!(read(Path::new("")).unwrap(), WorkspaceList::default());
    }

    #[test]
    fn object_form_keeps_paths_and_launch() {
        let tmp = crate::test_support::tempdir();
        let path = write(
            tmp.path(),
            r#"{"paths":["  C:\\a  ", "C:\\a", "", "C:\\b"], "launch":"C:\\b"}"#,
        );
        let list = read(&path).unwrap();
        assert_eq!(list.paths, vec![r"C:\a", r"C:\b"]);
        assert_eq!(list.launch, r"C:\b");
    }

    #[test]
    fn legacy_array_form_launches_the_first_path() {
        let tmp = crate::test_support::tempdir();
        let path = write(tmp.path(), r#"[" C:\\a ", "C:\\b"]"#);
        let list = read(&path).unwrap();
        assert_eq!(list.paths, vec![r"C:\a", r"C:\b"]);
        assert_eq!(list.launch, r"C:\a");
    }

    #[test]
    fn launch_outside_paths_falls_back_to_the_first() {
        let tmp = crate::test_support::tempdir();
        let path = write(
            tmp.path(),
            r#"{"paths":["C:\\a","C:\\b"], "launch":"C:\\gone"}"#,
        );
        let list = read(&path).unwrap();
        assert_eq!(list.launch, r"C:\a");
    }

    #[test]
    fn empty_file_is_an_error_like_go_unmarshal() {
        let tmp = crate::test_support::tempdir();
        let path = write(tmp.path(), "   ");
        assert!(read(&path).is_err());
        let path = write(tmp.path(), "{not json");
        assert!(read(&path).is_err());
    }

    #[test]
    fn unreadable_path_answers_empty_from_the_tolerant_wrappers() {
        // Reading a directory as the list file fails on every platform,
        // which stands in for an unreadable file.
        let tmp = crate::test_support::tempdir();
        assert!(read(tmp.path()).is_err());
        assert!(workspaces(tmp.path()).is_empty());
        assert!(launch_workspaces(tmp.path()).is_empty());
    }

    #[test]
    fn launch_workspaces_put_launch_first_without_reordering_the_rest() {
        let tmp = crate::test_support::tempdir();
        let path = write(
            tmp.path(),
            r#"{"paths":["C:\\a","C:\\b","C:\\c"], "launch":"C:\\b"}"#,
        );
        assert_eq!(launch_workspaces(&path), vec![r"C:\b", r"C:\a", r"C:\c"]);
        assert_eq!(workspaces(&path), vec![r"C:\a", r"C:\b", r"C:\c"]);
    }
}
