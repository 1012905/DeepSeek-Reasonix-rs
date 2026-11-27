use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use cap_fs_ext::{FollowSymlinks, MetadataExt as _, OpenOptionsFollowExt};
use cap_std::ambient_authority;
#[cfg(windows)]
use cap_std::fs::OpenOptionsExt as _;
use cap_std::fs::{Dir, File, OpenOptions};
use thiserror::Error;

use super::sha256_hex;
use super::types::{DEFAULT_MAX_ITEM_BYTES, RefSnapshot};
use crate::checkpoint::{PathError, scoped_path};

pub const MAX_FROZEN_DIR_ENTRIES: usize = 100;
const DIRECTORY_TRUNCATION_MARKER: &str = "…[truncated; directory has more entries]…";

#[derive(Debug, Error)]
pub enum RefError {
    #[error("path requires workspace root")]
    MissingWorkspace,
    #[error("open workspace root {path}: {source}")]
    OpenRoot {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[derive(Debug, Error)]
enum FreezeError {
    #[error("path outside workspace: {0}")]
    OutsideWorkspace(PathBuf),
    #[error("path escapes workspace through symlink: {0}")]
    Symlink(PathBuf),
    #[error("reference changed while opening: {0}")]
    ChangedOpening(PathBuf),
    #[error("reference changed while reading: {0}")]
    ChangedReading(PathBuf),
    #[error("directory reference changed while opening: {0}")]
    DirectoryChanged(PathBuf),
    #[error("directory entry escapes reference root")]
    EntryEscapes,
    #[error("directory entry name is not valid UTF-8")]
    NonUtf8Name,
    #[error("{op} {path}: {source}")]
    Io {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

pub fn freeze_refs(workspace: &str, paths: &[String]) -> Result<Vec<RefSnapshot>, RefError> {
    let workspace = workspace.trim();
    if workspace.is_empty() {
        return Err(RefError::MissingWorkspace);
    }
    let (root_path, _, _) = scoped_path(Path::new(workspace), Path::new("."))
        .map_err(|_| RefError::MissingWorkspace)?;
    let root = Dir::open_ambient_dir(&root_path, ambient_authority()).map_err(|source| {
        RefError::OpenRoot {
            path: root_path.clone(),
            source,
        }
    })?;
    let mut seen = BTreeSet::new();
    let mut snapshots = Vec::with_capacity(paths.len());
    for path in paths {
        let path = path.trim();
        if path.is_empty() || !seen.insert(path.to_owned()) {
            continue;
        }
        let snapshot = match freeze_scoped_ref(&root, &root_path, path) {
            Ok(snapshot) => snapshot,
            Err(error) => RefSnapshot {
                kind: "frozen".to_owned(),
                path: path.to_owned(),
                display_path: path.to_owned(),
                content: format!("/* ref freeze failed: {error} */").into_bytes(),
                ..RefSnapshot::default()
            },
        };
        snapshots.push(snapshot);
    }
    Ok(snapshots)
}

fn freeze_scoped_ref(root: &Dir, root_path: &Path, path: &str) -> Result<RefSnapshot, FreezeError> {
    let (_, _, relative) =
        scoped_path(root_path, Path::new(path)).map_err(|error| match error {
            PathError::OutsideWorkspace { .. } => {
                FreezeError::OutsideWorkspace(PathBuf::from(path))
            }
            PathError::Io { op, path, source } => FreezeError::Io { op, path, source },
            PathError::Symlink { component } => FreezeError::Symlink(component),
        })?;
    let display = if relative.as_os_str().is_empty() {
        ".".to_owned()
    } else {
        to_slash(&relative)
    };
    match open_target(root, &relative, path)? {
        Target::Directory(dir) => {
            let mut entries = Vec::new();
            walk_directory(&dir, "", path, &mut entries)?;
            let entries = cap_frozen_entries(entries);
            Ok(frozen_snapshot(
                display,
                entries.join("\n").into_bytes(),
                false,
            ))
        }
        Target::File(mut file) => {
            let opened = file
                .metadata()
                .map_err(|source| FreezeError::io("inspect opened reference", path, source))?;
            let max_bytes = u64::try_from(DEFAULT_MAX_ITEM_BYTES).unwrap_or(u64::MAX);
            let mut content = Vec::new();
            (&mut file)
                .take(max_bytes.saturating_add(1))
                .read_to_end(&mut content)
                .map_err(|source| FreezeError::io("read reference", path, source))?;
            let truncated = u64::try_from(content.len()).unwrap_or(u64::MAX) > max_bytes;
            if truncated {
                content.truncate(max_bytes as usize);
            }
            let after = file
                .metadata()
                .map_err(|_| FreezeError::ChangedReading(path.into()))?;
            if !same_file_state(&opened, &after) {
                return Err(FreezeError::ChangedReading(path.into()));
            }
            Ok(frozen_snapshot(display, content, truncated))
        }
    }
}

pub(super) fn cap_frozen_entries(mut entries: Vec<String>) -> Vec<String> {
    entries.sort();
    let truncated = entries.len() > MAX_FROZEN_DIR_ENTRIES;
    if truncated {
        entries.truncate(MAX_FROZEN_DIR_ENTRIES);
        entries.push(DIRECTORY_TRUNCATION_MARKER.to_owned());
    }
    entries
}

enum Target {
    Directory(Dir),
    File(File),
}

fn open_target(root: &Dir, relative: &Path, path: &str) -> Result<Target, FreezeError> {
    let mut current = root
        .try_clone()
        .map_err(|source| FreezeError::io("clone workspace root", path, source))?;
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(FreezeError::EntryEscapes);
        };
        let last = components.peek().is_none();
        let before = current
            .symlink_metadata(name)
            .map_err(|source| FreezeError::io("inspect reference", path, source))?;
        if before.is_symlink() {
            return Err(FreezeError::Symlink(path.into()));
        }
        if !last && !before.is_dir() {
            return Err(FreezeError::io(
                "open reference parent",
                path,
                io::Error::new(
                    io::ErrorKind::NotADirectory,
                    "reference parent is not a directory",
                ),
            ));
        }
        let (file, opened) = open_entry(&current, Path::new(name), path, &before)?;
        if last {
            return Ok(if opened.is_dir() {
                Target::Directory(Dir::from_std_file(file.into_std()))
            } else {
                Target::File(file)
            });
        }
        current = Dir::from_std_file(file.into_std());
    }
    Ok(Target::Directory(current))
}

fn walk_directory(
    dir: &Dir,
    display_base: &str,
    reference: &str,
    entries: &mut Vec<String>,
) -> Result<(), FreezeError> {
    if entries.len() > MAX_FROZEN_DIR_ENTRIES {
        return Ok(());
    }
    let children = dir
        .read_dir(".")
        .map_err(|source| FreezeError::io("read reference directory", reference, source))?;
    let mut names: Vec<OsString> = Vec::new();
    for child in children {
        let child = child
            .map_err(|source| FreezeError::io("read reference directory", reference, source))?;
        names.push(child.file_name());
    }
    for name in names {
        let before = dir
            .symlink_metadata(&name)
            .map_err(|source| FreezeError::io("inspect reference entry", reference, source))?;
        if before.is_symlink()
            || name
                .to_str()
                .is_some_and(|name| skip_frozen_dir_entry(name, before.is_dir()))
        {
            continue;
        }
        let name = name.to_str().ok_or(FreezeError::NonUtf8Name)?.to_owned();
        let display = if display_base.is_empty() {
            name.clone()
        } else {
            format!("{display_base}/{name}")
        };
        if before.is_dir() {
            entries.push(format!("{display}/"));
            let (file, _) = open_entry(dir, Path::new(&name), reference, &before)?;
            let child = Dir::from_std_file(file.into_std());
            walk_directory(&child, &display, reference, entries)?;
        } else {
            entries.push(display);
        }
        if entries.len() > MAX_FROZEN_DIR_ENTRIES {
            return Ok(());
        }
    }
    Ok(())
}

fn open_entry(
    dir: &Dir,
    name: &Path,
    path: &str,
    before: &cap_std::fs::Metadata,
) -> Result<(File, cap_std::fs::Metadata), FreezeError> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(windows)]
    {
        let mut flags = FILE_FLAG_OPEN_REPARSE_POINT;
        if before.is_dir() {
            flags |= FILE_FLAG_BACKUP_SEMANTICS;
        }
        options.custom_flags(flags).share_mode(FILE_SHARE_ALL);
    }
    let file = dir
        .open_with(name, &options)
        .map_err(|source| FreezeError::io("open reference entry", path, source))?;
    let opened = file
        .metadata()
        .map_err(|source| FreezeError::io("inspect opened reference", path, source))?;
    if opened.is_symlink() {
        return Err(FreezeError::Symlink(path.into()));
    }
    if before.is_dir() && (!opened.is_dir() || !same_file(before, &opened)) {
        return Err(FreezeError::DirectoryChanged(path.into()));
    }
    if !same_file(before, &opened) {
        return Err(FreezeError::ChangedOpening(path.into()));
    }
    Ok((file, opened))
}

fn frozen_snapshot(path: String, content: Vec<u8>, truncated: bool) -> RefSnapshot {
    RefSnapshot {
        kind: "frozen".to_owned(),
        content_sha: sha256_hex(&content),
        path: path.clone(),
        display_path: path,
        content,
        truncated,
        ..RefSnapshot::default()
    }
}

pub fn apply_frozen_refs(submit: &str, bodies: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    if bodies.is_empty() {
        return submit.as_bytes().to_vec();
    }
    let mut output = Vec::from(submit.as_bytes());
    output.extend_from_slice(b"\n\n<!-- frozen inbox references (enqueue-time snapshot) -->\n");
    for (path, body) in bodies {
        output.extend_from_slice(format!("\n### @{path}\n```\n").as_bytes());
        output.extend_from_slice(body);
        output.extend_from_slice(b"\n```\n");
    }
    output
}

pub fn materialize_refs(refs: &[RefSnapshot]) -> (String, BTreeMap<String, Vec<u8>>) {
    let mut bodies = BTreeMap::new();
    for reference in refs {
        if reference.kind == "clean_git" {
            return (
                format!(
                    "legacy clean-git reference {} requires refresh",
                    reference.path
                ),
                bodies,
            );
        }
        if !reference.content_sha.is_empty()
            && sha256_hex(&reference.content) != reference.content_sha
        {
            return (
                format!("checksum mismatch for frozen ref {}", reference.path),
                bodies,
            );
        }
        let key = [reference.display_path.trim(), reference.path.trim()]
            .into_iter()
            .find(|value| !value.is_empty())
            .unwrap_or_default();
        bodies.insert(key.to_owned(), reference.content.clone());
    }
    (String::new(), bodies)
}

fn skip_frozen_dir_entry(name: &str, is_dir: bool) -> bool {
    if name == ".DS_Store" || name == "Thumbs.db" {
        return true;
    }
    is_dir
        && matches!(
            name,
            ".git" | ".idea" | ".vscode" | "build" | "dist" | "node_modules" | "__pycache__"
        )
}

fn same_file(left: &cap_std::fs::Metadata, right: &cap_std::fs::Metadata) -> bool {
    left.dev() == right.dev() && left.ino() == right.ino()
}

pub(super) fn same_file_state(left: &cap_std::fs::Metadata, right: &cap_std::fs::Metadata) -> bool {
    same_file(left, right)
        && left.len() == right.len()
        && matches!((left.modified(), right.modified()), (Ok(left), Ok(right)) if left == right)
}

fn to_slash(path: &Path) -> String {
    let path = path.to_string_lossy();
    #[cfg(windows)]
    {
        path.replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        path.into_owned()
    }
}

impl FreezeError {
    fn io(op: &'static str, path: &str, source: io::Error) -> Self {
        Self::Io {
            op,
            path: PathBuf::from(path),
            source,
        }
    }
}

#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
#[cfg(windows)]
const FILE_SHARE_ALL: u32 = 0x0000_0007;
