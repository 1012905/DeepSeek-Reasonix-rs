use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use reasonix_core::fileutil::mkdir_all_mode;
#[cfg(windows)]
use reasonix_core::fileutil::set_file_mode;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt as _;
use thiserror::Error;

use super::path_capture::{PathError, safe_path};

#[cfg(not(unix))]
#[path = "secure_path_fallback.rs"]
mod secure_path_fallback;
#[cfg(unix)]
#[path = "secure_path_unix.rs"]
mod secure_path_unix;

#[cfg(not(unix))]
use secure_path_fallback as platform;
#[cfg(unix)]
use secure_path_unix as platform;

#[derive(Debug, Error)]
pub enum SecureError {
    #[error(transparent)]
    Path(#[from] PathError),
    #[error("relative path without workspace root: {0}")]
    RelativeWithoutRoot(PathBuf),
    #[error("workspace root is not a file target")]
    WorkspaceRoot,
    #[error("{op} {path}: {source}")]
    Io {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("rename {from} to {to}: {source}")]
    Rename {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("encode JSON {path}: {source}")]
    JsonEncode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("read JSON {path}: {source}")]
    JsonRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("decode JSON {path}: {source}")]
    JsonDecode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

impl SecureError {
    pub fn is_not_found(&self) -> bool {
        match self {
            Self::Io { source, .. }
            | Self::Rename { source, .. }
            | Self::JsonRead { source, .. } => source.kind() == io::ErrorKind::NotFound,
            Self::Path(PathError::Io { source, .. }) => source.kind() == io::ErrorKind::NotFound,
            Self::Path(_)
            | Self::RelativeWithoutRoot(_)
            | Self::WorkspaceRoot
            | Self::JsonEncode { .. }
            | Self::JsonDecode { .. } => false,
        }
    }

    pub(super) fn io(op: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            op,
            path: path.to_path_buf(),
            source,
        }
    }
}

fn resolve_target(root: &Path, path: &Path) -> Result<(PathBuf, PathBuf), SecureError> {
    if root.as_os_str().is_empty() && !path.is_absolute() {
        return Err(SecureError::RelativeWithoutRoot(path.to_path_buf()));
    }
    let root = if root.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        safe_path(root, Path::new("."))?
    };
    let absolute = safe_path(&root, path)?;
    Ok((root, absolute))
}

pub fn open_workspace_file(root: &Path, path: &Path) -> Result<File, SecureError> {
    let (root, absolute) = resolve_target(root, path)?;
    platform::open_workspace_file(&root, &absolute)
}

pub fn write_new_file(
    root: &Path,
    path: &Path,
    bytes: &[u8],
    mode: u32,
) -> Result<(), SecureError> {
    let (root, absolute) = resolve_target(root, path)?;
    platform::write_new_file(&root, &absolute, bytes, mode)
}

pub fn rename(root: &Path, from: &Path, to: &Path) -> Result<(), SecureError> {
    let (root, from) = resolve_target(root, from)?;
    let (_, to) = resolve_target(&root, to)?;
    platform::rename(&root, &from, &to)
}

pub fn remove(root: &Path, path: &Path) -> Result<(), SecureError> {
    let (root, absolute) = resolve_target(root, path)?;
    platform::remove(&root, &absolute)
}

pub fn chmod(root: &Path, path: &Path, mode: u32) -> Result<(), SecureError> {
    let (root, absolute) = resolve_target(root, path)?;
    platform::chmod(&root, &absolute, mode)
}

pub fn path_exists(root: &Path, path: &Path) -> Result<bool, SecureError> {
    let (root, absolute) = resolve_target(root, path)?;
    platform::path_exists(&root, &absolute)
}

pub fn read_file(root: &Path, path: &Path) -> Result<Vec<u8>, SecureError> {
    let (root, absolute) = resolve_target(root, path)?;
    platform::read_file(&root, &absolute)
}

pub(super) fn write_new_file_at(path: &Path, bytes: &[u8], mode: u32) -> Result<(), SecureError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    mkdir_all_mode(parent, 0o755)
        .map_err(|source| SecureError::io("create file directory", path, source))?;

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(mode & 0o777);
    #[cfg(not(any(unix, windows)))]
    let _ = mode;
    let mut file = options
        .open(path)
        .map_err(|source| SecureError::io("create", path, source))?;
    let result = (|| {
        #[cfg(windows)]
        {
            let mut permissions = file
                .metadata()
                .map_err(|source| SecureError::io("inspect", path, source))?
                .permissions();
            permissions.set_readonly(mode & 0o200 == 0);
            file.set_permissions(permissions)
                .map_err(|source| SecureError::io("set file mode", path, source))?;
        }
        file.write_all(bytes)
            .map_err(|source| SecureError::io("write", path, source))?;
        file.sync_all()
            .map_err(|source| SecureError::io("sync", path, source))
    })();
    #[cfg(windows)]
    if result.is_err() {
        let _ = set_file_mode(path, 0o600);
    }
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

pub(super) fn remove_plain(path: &Path) -> Result<(), SecureError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|source| SecureError::io("remove", path, source))?;
    if metadata.is_dir() {
        fs::remove_dir(path).map_err(|source| SecureError::io("remove", path, source))
    } else {
        fs::remove_file(path).map_err(|source| SecureError::io("remove", path, source))
    }
}
