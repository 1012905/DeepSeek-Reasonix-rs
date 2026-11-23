//! Atomic config writes and the symlink policy they obey (edit.go
//! atomicWriteToConfigFile, path_access.go, docs/contracts/config.md §5, §15.3).
//!
//! A write goes through temp + fsync + rename (reasonix-core fileutil), so an
//! interrupted write or power loss can never truncate the main config into an
//! unparseable state that leaves the app with no usable models.

use std::io;
use std::path::{Path, PathBuf};

use reasonix_core::fileutil::atomic_write_file;

/// The mode a user config file gets: 0600, because it can name credential
/// slots and endpoints.
pub const USER_CONFIG_PERM: u32 = 0o600;

/// The mode any other config file gets.
pub const PROJECT_CONFIG_PERM: u32 = 0o644;

/// Why a configuration write was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum WriteError {
    /// A blank path is refused before any file access.
    #[error("config path is empty")]
    EmptyPath,
    /// The path could not be made absolute.
    #[error("resolve config path {path:?}: {message}")]
    ResolvePath { path: String, message: String },
    /// Symlink resolution failed on a link we will not trust.
    #[error("resolve {scope} config path {path:?}: {message}")]
    ResolveSymlink {
        scope: &'static str,
        path: String,
        message: String,
    },
    /// A project-controlled link lands outside the workspace.
    #[error("project config path {path:?} resolves outside project root {root:?}: {resolved:?}")]
    OutsideProject {
        path: String,
        root: String,
        resolved: String,
    },
    /// The atomic write itself failed.
    #[error("write symlink target {resolved:?}: {message}")]
    Write { resolved: String, message: String },
}

/// Writes `body` to `path` atomically, resolving a symlink target first.
///
/// A user config link is an explicit user choice and may target any valid
/// file; a project link is repository-controlled, so its final target must stay
/// inside the project root, including for `.reasonix/reasonix.toml`.
pub fn write_config_file(path: &Path, body: &[u8], user_config: bool) -> Result<(), WriteError> {
    let resolved = resolve_config_write_path(path, user_config)?;
    let perm = config_file_perm(user_config);
    atomic_write_file(&resolved, body, perm).map_err(|error| WriteError::Write {
        resolved: resolved.to_string_lossy().into_owned(),
        message: error.to_string(),
    })
}

/// The mode a config file at this path gets: 0600 for the user's own config,
/// 0644 otherwise (edit.go configFilePerm).
pub fn config_file_perm(user_config: bool) -> u32 {
    if user_config {
        USER_CONFIG_PERM
    } else {
        PROJECT_CONFIG_PERM
    }
}

/// Resolves a config file symlink before any content is read or written
/// (path_access.go resolveConfigAccessPath). `user_config` selects the scope:
/// a user link may point anywhere, a project link must stay inside the root.
pub fn resolve_config_write_path(path: &Path, user_config: bool) -> Result<PathBuf, WriteError> {
    let raw = path.to_string_lossy();
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(WriteError::EmptyPath);
    }
    let logical = std::path::absolute(Path::new(raw)).map_err(|error| WriteError::ResolvePath {
        path: raw.to_owned(),
        message: error.to_string(),
    })?;
    let logical = crate::paths::clean_path(&logical);
    let resolved = match eval_symlinks_allow_missing(&logical) {
        Ok(resolved) => resolved,
        // A plain (absent or non-link) entry lives in its own directory
        // whatever its ancestors resolve to, so a mount that refuses
        // canonicalization need not block it.
        Err(_error) if plain_config_entry(&logical) => return Ok(logical),
        Err(error) => {
            let scope = if user_config { "user" } else { "project" };
            return Err(WriteError::ResolveSymlink {
                scope,
                path: logical.to_string_lossy().into_owned(),
                message: error.to_string(),
            });
        }
    };
    if user_config {
        return Ok(resolved);
    }

    let mut project_root = logical
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    if project_root
        .file_name()
        .is_some_and(|name| name == ".reasonix")
        && logical
            .file_name()
            .is_some_and(|name| name == "reasonix.toml")
        && let Some(parent) = project_root.parent()
    {
        project_root = parent.to_path_buf();
    }
    let root =
        eval_symlinks_allow_missing(&project_root).map_err(|error| WriteError::ResolveSymlink {
            scope: "project root",
            path: project_root.to_string_lossy().into_owned(),
            message: error.to_string(),
        })?;
    if !path_within_root(&root, &resolved) {
        return Err(WriteError::OutsideProject {
            path: logical.to_string_lossy().into_owned(),
            root: root.to_string_lossy().into_owned(),
            resolved: resolved.to_string_lossy().into_owned(),
        });
    }
    Ok(resolved)
}

/// Whether `path` is absent or a non-link file, in which case it lives in its
/// own directory whatever its ancestors resolve to.
fn plain_config_entry(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => !metadata.file_type().is_symlink() && !metadata.is_dir(),
        Err(error) => error.kind() == io::ErrorKind::NotFound,
    }
}

/// Canonicalizes every existing path component while allowing a new file (and
/// missing parent directories) to be created later. A broken final symlink is
/// not "missing": `canonicalize` sees the link and fails, which is what stops
/// a write from replacing it.
fn eval_symlinks_allow_missing(path: &Path) -> io::Result<PathBuf> {
    let path = crate::paths::clean_path(path);
    let mut current = path.clone();
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    loop {
        match std::fs::symlink_metadata(&current) {
            Ok(_) => {
                let mut resolved = std::fs::canonicalize(&current)?;
                for part in suffix.iter().rev() {
                    resolved.push(part);
                }
                return Ok(crate::paths::clean_path(&resolved));
            }
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            Err(_) => {
                let Some(parent) = current.parent() else {
                    return Ok(path);
                };
                if parent == current {
                    return Ok(path);
                }
                if let Some(name) = current.file_name() {
                    suffix.push(name.to_os_string());
                }
                current = parent.to_path_buf();
            }
        }
    }
}

/// `filepath.Rel`-based containment: "." or a relative path that does not
/// start with "..".
fn path_within_root(root: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    let mut parts = rel.components();
    match parts.next() {
        None => true,
        Some(component) => !matches!(component, std::path::Component::ParentDir),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PROJECT_CONFIG_PERM, USER_CONFIG_PERM, WriteError, config_file_perm,
        resolve_config_write_path, write_config_file,
    };
    use std::fs;
    use std::path::{Path, PathBuf};

    fn temp_dir(name: &str) -> PathBuf {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        fs::create_dir_all(&base).expect("create test-tmp base");
        let dir = base.join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    #[test]
    fn the_user_config_is_0600_and_a_project_file_0644() {
        // edit.go configFilePerm.
        assert_eq!(config_file_perm(true), USER_CONFIG_PERM);
        assert_eq!(config_file_perm(false), PROJECT_CONFIG_PERM);
        assert_eq!(USER_CONFIG_PERM, 0o600);
        assert_eq!(PROJECT_CONFIG_PERM, 0o644);
    }

    #[test]
    fn a_write_publishes_the_body_and_leaves_no_temp_behind() {
        let dir = temp_dir("write-plain");
        let path = dir.join("config.toml");
        write_config_file(&path, b"language = \"en\"\n", true).expect("write");
        assert_eq!(fs::read(&path).unwrap(), b"language = \"en\"\n");
        write_config_file(&path, b"language = \"zh\"\n", true).expect("replace");
        assert_eq!(fs::read(&path).unwrap(), b"language = \"zh\"\n");
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".atomic-"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");
    }

    #[test]
    fn a_blank_path_is_refused_before_any_file_access() {
        // path_access.go: "config path is empty".
        assert_eq!(
            write_config_file(Path::new("   "), b"x", true).unwrap_err(),
            WriteError::EmptyPath
        );
        assert_eq!(
            resolve_config_write_path(Path::new(""), false).unwrap_err(),
            WriteError::EmptyPath
        );
        assert_eq!(WriteError::EmptyPath.to_string(), "config path is empty");
    }

    #[cfg(unix)]
    #[test]
    fn a_user_link_may_target_any_file() {
        let dir = temp_dir("write-user-link");
        let target = dir.join("real-config.toml");
        fs::write(&target, "old\n").unwrap();
        let link = dir.join("config.toml");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        write_config_file(&link, b"new\n", true).expect("write through user link");
        // The link is preserved and the target carries the body.
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&target).unwrap(), b"new\n");
    }

    #[cfg(unix)]
    #[test]
    fn a_project_link_out_of_the_workspace_is_refused() {
        // The project link is repository-controlled, so its target must stay
        // inside the root: a checked-in link cannot redirect a write.
        let root = temp_dir("write-project-escape");
        let outside = temp_dir("write-project-outside");
        let decoy = outside.join("config.toml");
        fs::write(&decoy, "old\n").unwrap();
        let link = root.join("reasonix.toml");
        std::os::unix::fs::symlink(&decoy, &link).unwrap();
        let error = write_config_file(&link, b"new\n", false).unwrap_err();
        assert!(
            matches!(error, WriteError::OutsideProject { .. }),
            "error = {error:?}"
        );
        assert_eq!(fs::read(&decoy).unwrap(), b"old\n");
    }

    #[cfg(unix)]
    #[test]
    fn a_project_link_inside_the_workspace_is_followed() {
        let root = temp_dir("write-project-inside");
        let target = root.join("real.toml");
        fs::write(&target, "old\n").unwrap();
        let link = root.join("reasonix.toml");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        write_config_file(&link, b"new\n", false).expect("write through project link");
        assert_eq!(fs::read(&target).unwrap(), b"new\n");
    }

    #[cfg(unix)]
    #[test]
    fn a_broken_user_link_is_refused_rather_than_replaced() {
        // evalSymlinksAllowMissing: a broken link is not "missing"; the write
        // must not silently replace the link with a file.
        let dir = temp_dir("write-broken-link");
        let link = dir.join("config.toml");
        std::os::unix::fs::symlink(dir.join("gone.toml"), &link).unwrap();
        let error = write_config_file(&link, b"new\n", true);
        assert!(error.is_err(), "a broken link must fail closed");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn a_missing_file_resolves_to_its_absolute_path() {
        let dir = temp_dir("write-missing");
        let path = dir.join("nested").join("reasonix.toml");
        let resolved = resolve_config_write_path(&path, false).expect("resolve");
        assert!(
            resolved.ends_with("nested/reasonix.toml")
                || resolved.ends_with("nested\\reasonix.toml")
        );
    }

    #[test]
    fn a_nested_project_config_roots_at_the_workspace() {
        // The `.reasonix/reasonix.toml` spelling roots at the workspace, not at
        // the `.reasonix` directory.
        let dir = temp_dir("write-nested-root");
        fs::create_dir_all(dir.join(".reasonix")).unwrap();
        let path = dir.join(".reasonix").join("reasonix.toml");
        let resolved = resolve_config_write_path(&path, false).expect("resolve");
        assert!(
            resolved.starts_with(fs::canonicalize(&dir).unwrap()) || resolved.starts_with(&dir)
        );
    }
}
