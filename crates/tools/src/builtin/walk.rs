//! The generic directory walker (Go `filepath.WalkDir` semantics: entries
//! lexical, no symlink following, SkipDir/SkipAll) and walkConfine — the
//! fast containment verdict for the entries one walk reaches
//! (walk_confine.go). Walks descend only into plain directories, so a plain
//! entry below the root resolves to the root's real path joined with its
//! relative path; only a link or special entry pays a full resolve.

#![forbid(unsafe_code)]

use std::fs::FileType;
use std::io;

use super::confine::{confine_read, confine_resolved, read_outside_scope, real_path, within};
use super::pathgo;

/// Vendor / VCS / build-cache directory names grep and glob skip during a
/// recursive walk; only skipped when nested, so an explicit walk rooted at
/// one still searches it (workspace.go `vendorDirs`).
pub const VENDOR_DIRS: [&str; 10] = [
    ".git",
    ".svn",
    ".hg",
    ".jj",
    "node_modules",
    "vendor",
    ".venv",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
];

/// Whether a walk should prune a directory by name (workspace.go
/// `skipWalkDir`; the root is never pruned — handled by callers).
pub fn is_vendor_dir(name: &str) -> bool {
    VENDOR_DIRS.contains(&name)
}

/// Go `fileutil.IsVCSStoreDir` — a single path element naming a VCS store.
pub fn is_vcs_store_dir(name: &str) -> bool {
    [".jj", ".git", ".hg", ".svn"].contains(&name)
}

/// Go `isHiddenName`: the dot entries git hides by default.
pub fn is_hidden_name(name: &str) -> bool {
    name.len() > 1 && name.as_bytes()[0] == b'.' && name != ".."
}

/// Whether a directory should be pruned from a walk rooted at `root`
/// (workspace.go `skipWalkDir`). The root itself is never pruned. The
/// darwin-only protected-dirs table is platform-conditional on the Go side
/// and empty on this platform; it is anchored rather than ported.
pub fn skip_walk_dir(root: &str, path: &str, name: &str) -> bool {
    if path == root {
        return false;
    }
    is_vendor_dir(name)
}

/// workspace.go `skipForbidDir`: pruned because it is within a forbid-read
/// root.
pub fn skip_forbid_dir(path: &str, forbid_roots: &[String]) -> bool {
    confine_read(forbid_roots, path)
}

/// Go `absClean` (gitignore.go): abs with a Clean fallback, so a caller
/// never holds a path that is sometimes relative.
pub fn abs_clean(p: &str) -> String {
    match pathgo::abs_path(p) {
        Ok(abs) => abs,
        Err(_) => pathgo::clean(p),
    }
}

/// What one walked entry carries. `err` mirrors WalkDir handing the callback
/// a non-nil error (root stat failure or an unreadable directory); the
/// callback decides whether to abort.
pub struct WalkEntry {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub is_regular: bool,
    pub err: Option<io::Error>,
}

/// What the visit decides (Go: nil / filepath.SkipDir / filepath.SkipAll).
pub enum WalkFlow {
    Continue,
    SkipDir,
    SkipAll,
}

/// Walks `root` depth-first in lexical order, never following symlinks,
/// mirroring `filepath.WalkDir`. The root is visited first; a read error on
/// a directory visits that directory again with `err` set, exactly where
/// WalkDir calls the callback with a non-nil error, and continues when the
/// visit answers `Continue`. Returns whether the walk ended in SkipAll.
pub fn walk_dir(
    root: &str,
    visit: &mut dyn FnMut(&WalkEntry) -> Result<WalkFlow, io::Error>,
) -> Result<bool, io::Error> {
    let stopped = walk_recurse(root, None, visit)?;
    Ok(stopped)
}

/// Returns true when the whole walk was aborted by SkipAll.
fn walk_recurse(
    path: &str,
    child_type: Option<FileType>,
    visit: &mut dyn FnMut(&WalkEntry) -> Result<WalkFlow, io::Error>,
) -> Result<bool, io::Error> {
    let entry = match child_type {
        Some(ft) => WalkEntry {
            path: String::from(path),
            name: file_name_of(path),
            is_dir: ft.is_dir(),
            is_regular: ft.is_file(),
            err: None,
        },
        None => match std::fs::symlink_metadata(path) {
            Ok(m) => WalkEntry {
                path: String::from(path),
                name: file_name_of(path),
                is_dir: m.is_dir(),
                is_regular: m.is_file(),
                err: None,
            },
            Err(e) => WalkEntry {
                path: String::from(path),
                name: file_name_of(path),
                is_dir: false,
                is_regular: false,
                err: Some(e),
            },
        },
    };
    match visit(&entry)? {
        WalkFlow::SkipAll => return Ok(true),
        WalkFlow::SkipDir => return Ok(false),
        WalkFlow::Continue => {}
    }
    if entry.err.is_some() || !entry.is_dir {
        return Ok(false);
    }
    let children = match std::fs::read_dir(path) {
        Ok(c) => c,
        Err(e) => {
            let err_entry = WalkEntry {
                path: String::from(path),
                name: entry.name.clone(),
                is_dir: true,
                is_regular: false,
                err: Some(e),
            };
            return match visit(&err_entry)? {
                WalkFlow::SkipAll => Ok(true),
                _ => Ok(false),
            };
        }
    };
    let mut listed: Vec<(String, FileType)> = Vec::new();
    for child in children {
        let child = child?;
        listed.push((
            child.file_name().to_string_lossy().into_owned(),
            child.file_type()?,
        ));
    }
    listed.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, ft) in listed {
        let child_path = pathgo::join(&[path, &name]);
        if walk_recurse(&child_path, Some(ft), visit)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn file_name_of(path: &str) -> String {
    let b = path.as_bytes();
    let mut i = b.len();
    while i > 0 && !pathgo::is_path_separator(b[i - 1]) {
        i -= 1;
    }
    String::from(&path[i..])
}

/// walkConfine answers confineRead for the entries one walk reaches.
pub struct WalkConfine {
    forbid_roots: Vec<String>,
    scope: Vec<String>,
    root: String,
    real_root: String,
    protect: bool,
    active: bool,
}

impl WalkConfine {
    pub fn new(forbid_roots: &[String], scope: &[String], root: &str) -> Self {
        let protect = super::secretpaths::protect_sensitive_files();
        let active = !forbid_roots.is_empty()
            || !scope.is_empty()
            || protect
            || super::secretpaths::PROTECT_CREDENTIAL_FILES;
        let real_root = if active { real_path(root).ok() } else { None }.unwrap_or_default();
        Self {
            forbid_roots: forbid_roots.to_vec(),
            scope: scope.to_vec(),
            root: String::from(root),
            real_root,
            protect,
            active,
        }
    }

    /// Whether the walked entry at path is confined. `d` is the entry the
    /// walk handed over; None mirrors Go passing a nil DirEntry.
    pub fn blocked(&self, path: &str, d: Option<(&str, bool, bool)>) -> bool {
        // (name, is_dir, is_regular); a link or special entry has type bits
        // beyond ModeDir, matching Go's d.Type()&^fs.ModeDir != 0.
        if !self.active {
            return false;
        }
        let special = match d {
            None => true,
            Some((_, is_dir, is_regular)) => !is_dir && !is_regular,
        };
        if self.real_root.is_empty() || special {
            return read_outside_scope(&self.scope, path) || confine_read(&self.forbid_roots, path);
        }
        let rel = match pathgo::rel(&self.root, path) {
            Ok(r) => r,
            Err(_) => {
                return read_outside_scope(&self.scope, path)
                    || confine_read(&self.forbid_roots, path);
            }
        };
        if rel == ".." || rel.starts_with(&format!("..{}", pathgo::separator())) {
            return read_outside_scope(&self.scope, path) || confine_read(&self.forbid_roots, path);
        }
        confine_resolved(
            &self.forbid_roots,
            &pathgo::join(&[&self.real_root, &rel]),
            self.protect,
        )
    }
}

/// Go `underDir` (gitignore.go): path at or below dir.
pub fn under_dir(dir: &str, path: &str) -> bool {
    path == dir || path.starts_with(&format!("{}{}", dir, pathgo::separator()))
}

/// Whether `path` is within `root` or below it (used by tests and walks).
pub fn at_or_under(root: &str, path: &str) -> bool {
    within(root, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_visits_lexically_without_following_links() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("walktest")
            .tempdir_in(&base)
            .unwrap();
        std::fs::create_dir_all(dir.path().join("b")).unwrap();
        std::fs::create_dir_all(dir.path().join("a")).unwrap();
        std::fs::write(dir.path().join("a/f.txt"), b"x").unwrap();
        let mut seen: Vec<String> = Vec::new();
        walk_dir(dir.path().to_str().unwrap(), &mut |e| {
            seen.push(e.path.clone());
            Ok(WalkFlow::Continue)
        })
        .unwrap();
        // root + a + a/f.txt + b
        assert_eq!(seen.len(), 4, "{seen:?}");
        let sep = pathgo::separator();
        assert!(
            seen[0].ends_with("walktest") || seen[0].contains("walktest"),
            "{seen:?}"
        );
        assert!(seen[1].ends_with(&format!("{sep}a")), "{seen:?}");
        assert!(seen[2].ends_with(&format!("{sep}a{sep}f.txt")), "{seen:?}");
        assert!(seen[3].ends_with(&format!("{sep}b")), "{seen:?}");
    }

    #[test]
    fn skip_dir_prunes_subtree() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("walkskip")
            .tempdir_in(&base)
            .unwrap();
        std::fs::create_dir_all(dir.path().join("skip/inner")).unwrap();
        std::fs::create_dir_all(dir.path().join("keep")).unwrap();
        std::fs::write(dir.path().join("skip/inner/f.txt"), b"x").unwrap();
        std::fs::write(dir.path().join("keep/f.txt"), b"x").unwrap();
        let mut seen: Vec<String> = Vec::new();
        walk_dir(dir.path().to_str().unwrap(), &mut |e| {
            seen.push(e.name.clone());
            if e.is_dir && e.name == "skip" {
                return Ok(WalkFlow::SkipDir);
            }
            Ok(WalkFlow::Continue)
        })
        .unwrap();
        assert!(seen.contains(&String::from("keep")));
        assert!(seen.iter().filter(|n| **n == "f.txt").count() == 1);
        assert!(!seen.contains(&String::from("inner")));
    }
}
