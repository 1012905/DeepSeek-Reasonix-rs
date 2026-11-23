//! Project config file selection (paths.md §2.2, config.md §12.5).
//!
//! The workspace root is resolved by the caller; this slice probes the two
//! declared candidates under it. Only an absent entry permits fallback to the
//! next candidate — a broken link or an access error stays attached to the
//! higher-priority path. Root confinement itself is enforced by the loader's
//! path-access check, not by this selection.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const PROJECT_CONFIG_FILE: &str = "reasonix.toml";
const NESTED_DIR_NAME: &str = ".reasonix";

/// Outcome of a metadata probe, without following symlinks (Go `Lstat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeStatus {
    /// Entry exists: regular file, directory, or symlink. A broken link reports
    /// here too — the link itself is the higher-priority answer.
    Present,
    /// Nothing at the path; fallback to the next candidate is permitted.
    Missing,
    /// Probe failed for another reason (permission, I/O); keep this path.
    Blocked,
}

impl ProbeStatus {
    fn classify(err: &io::Error) -> Self {
        match err.kind() {
            io::ErrorKind::NotFound => Self::Missing,
            _ => Self::Blocked,
        }
    }
}

/// The two candidates in precedence order: `root/reasonix.toml`, then
/// `root/.reasonix/reasonix.toml` (Go `ProjectConfigPaths`).
pub fn project_config_paths(root: &Path) -> [PathBuf; 2] {
    [
        root.join(PROJECT_CONFIG_FILE),
        root.join(NESTED_DIR_NAME).join(PROJECT_CONFIG_FILE),
    ]
}

/// Probe one candidate without following symlinks (Go `os.Lstat`).
pub fn probe_status(path: &Path) -> ProbeStatus {
    match fs::symlink_metadata(path) {
        Ok(_) => ProbeStatus::Present,
        Err(err) => ProbeStatus::classify(&err),
    }
}

/// Select the project config file for the injected root. The primary path
/// wins whenever anything reports for it; fallback to the nested candidate
/// happens only when the primary is absent (Go `ProjectConfigPath`).
pub fn project_config_path(root: &Path) -> PathBuf {
    let [primary, nested] = project_config_paths(root);
    match probe_status(&primary) {
        ProbeStatus::Missing => match probe_status(&nested) {
            ProbeStatus::Missing => primary,
            _ => nested,
        },
        _ => primary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use std::fs;
    use std::io::ErrorKind;

    #[test]
    fn candidate_paths_match_reference_layout() {
        let root = Path::new("/repo/ws");
        let [primary, nested] = project_config_paths(root);
        assert_eq!(primary, Path::new("/repo/ws/reasonix.toml"));
        assert_eq!(nested, Path::new("/repo/ws/.reasonix/reasonix.toml"));
    }

    #[test]
    fn absent_both_falls_back_to_primary_as_creation_target() {
        let tmp = test_support::tempdir();
        let root = tmp.path();
        assert_eq!(project_config_path(root), root.join(PROJECT_CONFIG_FILE));
    }

    #[test]
    fn nested_only_is_selected() {
        let tmp = test_support::tempdir();
        let root = tmp.path();
        fs::create_dir_all(root.join(NESTED_DIR_NAME)).unwrap();
        fs::write(
            root.join(NESTED_DIR_NAME).join(PROJECT_CONFIG_FILE),
            "[providers]\n",
        )
        .unwrap();
        assert_eq!(
            project_config_path(root),
            root.join(NESTED_DIR_NAME).join(PROJECT_CONFIG_FILE)
        );
    }

    #[test]
    fn primary_wins_when_both_exist() {
        let tmp = test_support::tempdir();
        let root = tmp.path();
        fs::write(root.join(PROJECT_CONFIG_FILE), "[providers]\n").unwrap();
        fs::create_dir_all(root.join(NESTED_DIR_NAME)).unwrap();
        fs::write(
            root.join(NESTED_DIR_NAME).join(PROJECT_CONFIG_FILE),
            "[providers]\n",
        )
        .unwrap();
        assert_eq!(project_config_path(root), root.join(PROJECT_CONFIG_FILE));
    }

    #[test]
    fn primary_directory_stays_primary() {
        let tmp = test_support::tempdir();
        let root = tmp.path();
        fs::create_dir(root.join(PROJECT_CONFIG_FILE)).unwrap();
        fs::create_dir_all(root.join(NESTED_DIR_NAME)).unwrap();
        fs::write(
            root.join(NESTED_DIR_NAME).join(PROJECT_CONFIG_FILE),
            "[providers]\n",
        )
        .unwrap();
        assert_eq!(project_config_path(root), root.join(PROJECT_CONFIG_FILE));
    }

    #[test]
    fn broken_symlink_at_primary_stays_primary() {
        let tmp = test_support::tempdir();
        let root = tmp.path();
        let dangling = root.join("missing.toml");
        let link = root.join(PROJECT_CONFIG_FILE);
        fs::write(&dangling, "x").unwrap();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(&dangling, &link).is_ok();
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&dangling, &link).is_ok();
        if !linked {
            // No symlink privilege (Go: t.Skip).
            return;
        }
        fs::remove_file(&dangling).unwrap();
        assert_eq!(probe_status(&link), ProbeStatus::Present);
        assert_eq!(project_config_path(root), link);
    }

    #[test]
    fn probe_classifies_error_kinds_without_string_matching() {
        let not_found = io::Error::new(ErrorKind::NotFound, "missing");
        let denied = io::Error::new(ErrorKind::PermissionDenied, "denied");
        assert_eq!(ProbeStatus::classify(&not_found), ProbeStatus::Missing);
        assert_eq!(ProbeStatus::classify(&denied), ProbeStatus::Blocked);
    }
}
