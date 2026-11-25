use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::ambient_authority;
#[cfg(windows)]
use cap_std::fs::OpenOptionsExt as _;
use cap_std::fs::{Dir, OpenOptions};
use reasonix_core::fileencoding::{self, Kind};
use thiserror::Error;

use super::blob::digest;
use super::types::{
    CONFLICT_COVERAGE_LEGACY, CONFLICT_DELETED_RECREATE, CONFLICT_EXTERNAL_CHANGE,
    CONFLICT_MANUAL_EDIT, CONFLICT_MODE_CHANGE, CaptureOptions, CoverageGap,
    DEFAULT_MAX_FILE_BYTES, GAP_CAPTURE_FAILED, GAP_HARDLINK, GAP_OUTSIDE_WORKSPACE, GAP_OVERSIZED,
    GAP_SYMLINK, GAP_UNREADABLE,
};

// Windows has no O_NOFOLLOW; opening the reparse point itself makes the check descriptor-relative.
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
#[cfg(windows)]
const FILE_SHARE_READ_WRITE: u32 = 0x0000_0003;

#[derive(Debug, Error)]
pub enum PathError {
    #[error("checkpoint path {path} escapes workspace {root}")]
    OutsideWorkspace { path: PathBuf, root: PathBuf },
    #[error("workspace path contains symbolic link: {component}")]
    Symlink { component: PathBuf },
    #[error("{op} {path}: {source}")]
    Io {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error(transparent)]
    Path(#[from] PathError),
    #[error("relative path without workspace root: {0}")]
    RelativeWithoutRoot(PathBuf),
    #[error("{op} {path}: {source}")]
    Inspect {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("open workspace file {path}: {source}")]
    Unreadable {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("hard link nlink={nlink}: {path}")]
    Hardlink { path: PathBuf, nlink: u64 },
    #[error("path is a directory: {path}")]
    Directory { path: PathBuf },
    #[error("file too large: {path} ({size} bytes, limit {max_bytes})")]
    Oversized {
        path: PathBuf,
        size: u64,
        max_bytes: u64,
    },
    #[error("read workspace file {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl CaptureError {
    pub fn gap(&self) -> CoverageGap {
        let path = match self {
            Self::Path(PathError::OutsideWorkspace { path, .. } | PathError::Io { path, .. }) => {
                path
            }
            Self::Path(PathError::Symlink { component }) => component,
            Self::RelativeWithoutRoot(path)
            | Self::Inspect { path, .. }
            | Self::Unreadable { path, .. }
            | Self::Hardlink { path, .. }
            | Self::Directory { path, .. }
            | Self::Oversized { path, .. }
            | Self::Read { path, .. } => path,
        };
        self.gap_at(path)
    }

    pub(crate) fn gap_at(&self, path: &Path) -> CoverageGap {
        let reason = match self {
            Self::Path(PathError::OutsideWorkspace { .. }) => GAP_OUTSIDE_WORKSPACE,
            Self::Path(PathError::Symlink { .. }) => GAP_SYMLINK,
            Self::RelativeWithoutRoot(_)
            | Self::Inspect { .. }
            | Self::Unreadable { .. }
            | Self::Path(PathError::Io { .. })
            | Self::Read { .. } => GAP_UNREADABLE,
            Self::Hardlink { .. } => GAP_HARDLINK,
            Self::Directory { .. } => GAP_CAPTURE_FAILED,
            Self::Oversized { .. } => GAP_OVERSIZED,
        };
        let mut gap = CoverageGap::new(reason, &self.to_string());
        gap.path = path.to_string_lossy().into_owned();
        gap
    }

    pub fn is_outside_workspace(&self) -> bool {
        matches!(self, Self::Path(PathError::OutsideWorkspace { .. }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    pub abs_path: PathBuf,
    pub existed: bool,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub nlink: u64,
    pub mode: u32,
    pub size: u64,
    pub sha256: String,
    pub encoding: Kind,
    pub content: Option<Vec<u8>>,
}

impl Default for Fingerprint {
    fn default() -> Self {
        Self {
            abs_path: PathBuf::new(),
            existed: false,
            is_dir: false,
            is_symlink: false,
            nlink: 1,
            mode: 0,
            size: 0,
            sha256: String::new(),
            encoding: Kind::Utf8,
            content: None,
        }
    }
}

pub fn safe_path(root: &Path, path: &Path) -> Result<PathBuf, PathError> {
    if path.as_os_str().is_empty() {
        return Err(PathError::Io {
            op: "resolve checkpoint path",
            path: path.to_path_buf(),
            source: io::Error::new(io::ErrorKind::InvalidInput, "empty path"),
        });
    }
    if root.as_os_str().is_empty() {
        if !path.is_absolute() {
            return Err(PathError::Io {
                op: "resolve checkpoint path",
                path: path.to_path_buf(),
                source: io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "relative path without workspace root",
                ),
            });
        }
        let absolute = lexical_clean(path);
        validate_absolute_symlinks(&absolute)?;
        return Ok(absolute);
    }

    let root = std::path::absolute(root).map_err(|source| PathError::Io {
        op: "resolve workspace root",
        path: root.to_path_buf(),
        source,
    })?;
    let root = lexical_clean(&root);
    let absolute = if path.is_absolute() {
        lexical_clean(path)
    } else {
        lexical_clean(&root.join(path))
    };
    let relative =
        relative_components(&root, &absolute).ok_or_else(|| PathError::OutsideWorkspace {
            path: absolute.clone(),
            root: root.clone(),
        })?;
    let mut current = root;
    for component in relative {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(PathError::Symlink { component: current });
            }
            Ok(_) => {}
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(absolute),
            Err(source) => {
                return Err(PathError::Io {
                    op: "inspect workspace path",
                    path: current,
                    source,
                });
            }
        }
    }
    Ok(absolute)
}

fn validate_absolute_symlinks(absolute: &Path) -> Result<(), PathError> {
    let mut current = absolute.to_path_buf();
    loop {
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(PathError::Symlink { component: current });
            }
            Ok(_) => {}
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(PathError::Io {
                    op: "inspect workspace path",
                    path: current,
                    source,
                });
            }
        }
        if !current.pop() {
            return Ok(());
        }
    }
}

pub fn normalize_rel_path(root: &Path, path: &Path) -> String {
    let normalized = if root.as_os_str().is_empty() {
        lexical_clean(path)
    } else if let Ok(root) = std::path::absolute(root).map(|path| lexical_clean(&path)) {
        let absolute = if path.is_absolute() {
            lexical_clean(path)
        } else {
            lexical_clean(&root.join(path))
        };
        relative_components(&root, &absolute)
            .map(|parts| parts.iter().collect::<PathBuf>())
            .unwrap_or_else(|| lexical_clean(path))
    } else {
        lexical_clean(path)
    };
    let normalized = normalized.to_string_lossy();
    #[cfg(windows)]
    {
        normalized.replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        normalized.into_owned()
    }
}

fn relative_components(root: &Path, absolute: &Path) -> Option<Vec<std::ffi::OsString>> {
    let mut root_components = root.components();
    let mut absolute_components = absolute.components();
    loop {
        match (root_components.next(), absolute_components.next()) {
            (Some(left), Some(right)) if component_eq(&left, &right) => {}
            (Some(_), _) => return None,
            (None, remaining) => {
                let mut parts = Vec::new();
                if let Some(component) = remaining {
                    parts.push(component.as_os_str().to_owned());
                }
                parts.extend(absolute_components.map(|component| component.as_os_str().to_owned()));
                return Some(parts);
            }
        }
    }
}

fn component_eq(left: &Component<'_>, right: &Component<'_>) -> bool {
    #[cfg(windows)]
    {
        left.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn lexical_clean(path: &Path) -> PathBuf {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => clean.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if clean.file_name().is_some() {
                    clean.pop();
                } else if !clean.has_root() {
                    clean.push("..");
                }
            }
            Component::Normal(part) => clean.push(part),
        }
    }
    if clean.as_os_str().is_empty() {
        clean.push(".");
    }
    clean
}

pub fn capture_path(path: &Path, opts: &CaptureOptions) -> Result<Fingerprint, CaptureError> {
    if !path.is_absolute() && opts.workspace_root.as_os_str().is_empty() {
        return Err(CaptureError::RelativeWithoutRoot(path.to_path_buf()));
    }
    let absolute = safe_path(&opts.workspace_root, path).map_err(|error| match error {
        error @ (PathError::OutsideWorkspace { .. } | PathError::Symlink { .. }) => {
            CaptureError::Path(error)
        }
        PathError::Io { op, path, source } => CaptureError::Inspect { op, path, source },
    })?;
    let (root, relative) = capture_scope(&opts.workspace_root, &absolute)?;
    let Some((mut file, metadata)) = open_captured_file(&root, &relative, &absolute)? else {
        return Ok(Fingerprint {
            abs_path: absolute,
            ..Fingerprint::default()
        });
    };
    let mut fingerprint = Fingerprint {
        abs_path: absolute.clone(),
        existed: true,
        is_dir: metadata.is_dir(),
        is_symlink: metadata.file_type().is_symlink(),
        nlink: link_count(&metadata),
        mode: file_mode(&metadata),
        size: metadata.len(),
        ..Fingerprint::default()
    };
    if fingerprint.is_dir {
        return Err(CaptureError::Directory { path: absolute });
    }
    if fingerprint.nlink > 1 {
        return Err(CaptureError::Hardlink {
            path: absolute,
            nlink: fingerprint.nlink,
        });
    }
    if !opts.read_content {
        return Ok(fingerprint);
    }
    let max_bytes = if opts.max_bytes == 0 {
        DEFAULT_MAX_FILE_BYTES
    } else {
        opts.max_bytes
    };
    if fingerprint.size > max_bytes {
        return Err(CaptureError::Oversized {
            path: absolute,
            size: fingerprint.size,
            max_bytes,
        });
    }
    let mut content = Vec::new();
    file.by_ref()
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut content)
        .map_err(|source| CaptureError::Read {
            path: absolute.clone(),
            source,
        })?;
    if content.len() as u64 > max_bytes {
        return Err(CaptureError::Oversized {
            path: absolute,
            size: content.len() as u64,
            max_bytes,
        });
    }
    fingerprint.sha256 = digest(&content);
    fingerprint.encoding = fileencoding::detect_and_decode(&content).0;
    fingerprint.content = Some(content);
    Ok(fingerprint)
}

fn capture_scope(root: &Path, absolute: &Path) -> Result<(PathBuf, Vec<OsString>), CaptureError> {
    let root = if root.as_os_str().is_empty() {
        absolute
            .ancestors()
            .last()
            .expect("an absolute path has a root")
            .to_path_buf()
    } else {
        lexical_clean(
            &std::path::absolute(root).map_err(|source| CaptureError::Inspect {
                op: "resolve workspace root",
                path: root.to_path_buf(),
                source,
            })?,
        )
    };
    let relative = relative_components(&root, absolute).ok_or_else(|| {
        CaptureError::Path(PathError::OutsideWorkspace {
            path: absolute.to_path_buf(),
            root: root.clone(),
        })
    })?;
    Ok((root, relative))
}

fn open_captured_file(
    root: &Path,
    relative: &[OsString],
    absolute: &Path,
) -> Result<Option<(fs::File, fs::Metadata)>, CaptureError> {
    let mut dir = match Dir::open_ambient_dir(root, ambient_authority()) {
        Ok(dir) => dir,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(CaptureError::Unreadable {
                path: absolute.to_path_buf(),
                source,
            });
        }
    };
    if relative.is_empty() {
        let file = dir.into_std_file();
        let metadata = file.metadata().map_err(|source| CaptureError::Unreadable {
            path: absolute.to_path_buf(),
            source,
        })?;
        return Ok(Some((file, metadata)));
    }
    let mut current = root.to_path_buf();
    for (index, component) in relative.iter().enumerate() {
        current.push(component);
        let Some((file, metadata)) = open_checkpoint_entry(&dir, Path::new(component), &current)?
        else {
            return Ok(None);
        };
        if index + 1 == relative.len() {
            return Ok(Some((file, metadata)));
        }
        if !metadata.is_dir() {
            return Err(CaptureError::Unreadable {
                path: current,
                source: io::Error::new(
                    io::ErrorKind::NotADirectory,
                    "path component is not a directory",
                ),
            });
        }
        dir = Dir::from_std_file(file);
    }
    Ok(None)
}

fn open_checkpoint_entry(
    dir: &Dir,
    relative: &Path,
    absolute: &Path,
) -> Result<Option<(fs::File, fs::Metadata)>, CaptureError> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(windows)]
    options
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .share_mode(FILE_SHARE_READ_WRITE);
    match dir.open_with(relative, &options) {
        Ok(file) => {
            let opened = file.metadata().map_err(|source| CaptureError::Unreadable {
                path: absolute.to_path_buf(),
                source,
            })?;
            if opened.file_type().is_symlink() {
                return Err(CaptureError::Path(PathError::Symlink {
                    component: absolute.to_path_buf(),
                }));
            }
            let file = file.into_std();
            let metadata = file.metadata().map_err(|source| CaptureError::Unreadable {
                path: absolute.to_path_buf(),
                source,
            })?;
            Ok(Some((file, metadata)))
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => {
            if dir
                .symlink_metadata(relative)
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                return Err(CaptureError::Path(PathError::Symlink {
                    component: absolute.to_path_buf(),
                }));
            }
            Err(CaptureError::Unreadable {
                path: absolute.to_path_buf(),
                source,
            })
        }
    }
}

pub fn fingerprint_path(root: &Path, path: &Path) -> Result<Fingerprint, CaptureError> {
    capture_path(
        path,
        &CaptureOptions {
            workspace_root: root.to_path_buf(),
            read_content: true,
            ..CaptureOptions::default()
        },
    )
}

pub fn compare_identity(
    current: &Fingerprint,
    after_sha256: &str,
    after_existed: Option<bool>,
    after_mode: u32,
) -> Option<&'static str> {
    if after_existed.is_none() && after_sha256.is_empty() {
        return Some(CONFLICT_COVERAGE_LEGACY);
    }
    let wanted = after_existed.unwrap_or(!after_sha256.is_empty());
    if current.existed != wanted {
        return Some(if !wanted && current.existed {
            CONFLICT_DELETED_RECREATE
        } else {
            CONFLICT_EXTERNAL_CHANGE
        });
    }
    if !current.existed {
        return None;
    }
    if after_mode != 0 && current.mode != 0 && current.mode != after_mode {
        return Some(CONFLICT_MODE_CHANGE);
    }
    if !after_sha256.is_empty() && current.sha256 != after_sha256 {
        return Some(CONFLICT_MANUAL_EDIT);
    }
    None
}

#[cfg(unix)]
fn link_count(metadata: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.nlink()
}

#[cfg(not(unix))]
fn link_count(_metadata: &fs::Metadata) -> u64 {
    1
}

#[cfg(unix)]
fn file_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn file_mode(metadata: &fs::Metadata) -> u32 {
    if metadata.permissions().readonly() {
        0o444
    } else {
        0o666
    }
}
