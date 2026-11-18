//! Canonical session-save path keys (Go: save.go
//! `canonicalSessionSavePath` / `CanonicalSessionPath` and
//! `lockSessionSavePath`).
//!
//! The key folds case (Windows), resolves symlinks through the nearest
//! existing ancestor, and strips the `\\?\` long-path prefix, so
//! two spellings of one file map to one persistence baseline and one
//! process-wide save lock.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// Canonical, case-folded (Windows) identity of a session save path,
/// used as the persistence-baseline key (Go:
/// `canonicalSessionSavePath`).
pub(crate) fn canonical_session_save_path(path: &str) -> String {
    let trimmed = path.trim();
    // Go: filepath.Clean turns the empty string into "."; filepath.Abs
    // then re-cleans and joins onto the working directory.
    let cleaned = if trimmed.is_empty() {
        ".".to_string()
    } else {
        trimmed.to_string()
    };
    let mut key = match std::path::absolute(&cleaned) {
        Ok(abs) => abs.to_string_lossy().into_owned(),
        Err(_) => cleaned,
    };
    key = resolve_path_through_existing_ancestor(&key);
    if cfg!(windows) {
        let upper = key.to_uppercase();
        if upper.starts_with("\\\\?\\UNC\\") && key.len() >= 8 && key.is_char_boundary(8) {
            key = "\\\\".to_string() + &key[8..];
        } else if let Some(stripped) = key.strip_prefix("\\\\?\\") {
            key = stripped.to_string();
        }
        key = key.to_lowercase();
    }
    key
}

/// Public wrapper that keeps empty input empty (Go:
/// `CanonicalSessionPath`).
pub fn canonical_session_path(path: &str) -> String {
    if path.trim().is_empty() {
        String::new()
    } else {
        canonical_session_save_path(path)
    }
}

/// Resolves symlinks through the deepest existing ancestor and re-joins
/// the missing tail (Go: `resolvePathThroughExistingAncestor`).
///
/// Returns the input unchanged when the walk hits the filesystem root;
/// Go behaves the same instead of failing.
fn resolve_path_through_existing_ancestor(path: &str) -> String {
    let mut current = PathBuf::from(path);
    let mut missing: Vec<OsString> = Vec::new();
    loop {
        if let Ok(mut resolved) = std::fs::canonicalize(&current) {
            for part in missing.iter().rev() {
                resolved.push(part);
            }
            return resolved.to_string_lossy().into_owned();
        }
        let Some(parent) = current.parent().map(PathBuf::from) else {
            return path.to_string();
        };
        if parent == current {
            return path.to_string();
        }
        match current.file_name() {
            Some(base) => missing.push(base.to_os_string()),
            None => return path.to_string(),
        }
        current = parent;
    }
}

static SESSION_SAVE_LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();

/// Runs `f` under the process-wide save lock for a canonical save
/// path (Go: `lockSessionSavePath`). Locks are never evicted,
/// matching the Go registry.
pub(crate) fn with_session_save_path_lock<R>(path: &str, f: impl FnOnce() -> R) -> R {
    let key = canonical_session_save_path(path);
    let registry = SESSION_SAVE_LOCKS.get_or_init(Mutex::default);
    let lock = {
        let mut map = registry.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_session_path_empty_is_empty() {
        assert_eq!(canonical_session_path(""), "");
        assert_eq!(canonical_session_path("   "), "");
        assert!(!canonical_session_save_path("").is_empty());
    }

    #[test]
    fn canonical_save_path_is_idempotent() {
        let dir = crate::sessionstore::eventlog::test_support::tempdir();
        let p = dir.path().join("a").join("s.jsonl");
        let c1 = canonical_session_save_path(&p.to_string_lossy());
        let c2 = canonical_session_save_path(&c1);
        assert_eq!(c1, c2);
    }

    #[test]
    #[cfg(windows)]
    fn canonical_save_path_folds_case_on_windows() {
        let dir = crate::sessionstore::eventlog::test_support::tempdir();
        let raw = dir.path().to_string_lossy().into_owned();
        let lower = canonical_session_save_path(&raw.to_lowercase());
        let upper = canonical_session_save_path(&raw.to_uppercase());
        assert_eq!(lower, upper);
    }

    #[test]
    fn canonical_save_path_resolves_through_existing_ancestor() {
        let dir = crate::sessionstore::eventlog::test_support::tempdir();
        let existing = dir.path().join("a").join("b");
        std::fs::create_dir_all(&existing).unwrap();
        let present = existing.join("x.jsonl");
        let absent = existing.join("newdir").join("y.jsonl");

        let c_present = canonical_session_save_path(&present.to_string_lossy());
        let c_absent = canonical_session_save_path(&absent.to_string_lossy());

        // The existing-ancestor prefix must be shared.
        let prefix = canonical_session_save_path(&existing.to_string_lossy());
        assert!(c_present.starts_with(&prefix));
        assert!(c_absent.starts_with(&prefix));
        // The missing tail must survive the resolution.
        assert!(c_absent.ends_with("newdir\\y.jsonl") || c_absent.ends_with("newdir/y.jsonl"));
    }

    #[test]
    fn save_path_lock_runs_and_reuses_keys() {
        let dir = crate::sessionstore::eventlog::test_support::tempdir();
        let p = dir.path().join("s.jsonl");
        let key = canonical_session_save_path(&p.to_string_lossy());
        let got = with_session_save_path_lock(&p.to_string_lossy(), || key.clone());
        assert_eq!(got, canonical_session_save_path(&p.to_string_lossy()));
        assert_eq!(with_session_save_path_lock("", || 7), 7);
    }
}
