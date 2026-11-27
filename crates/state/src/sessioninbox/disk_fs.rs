//! Descriptor-relative filesystem operations and the inbox transaction lock.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Component, Path};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cap_fs_ext::{FollowSymlinks, MetadataExt as _, OpenOptionsFollowExt};
use cap_std::ambient_authority;
#[cfg(windows)]
use cap_std::fs::OpenOptionsExt as _;
use cap_std::fs::{Dir, OpenOptions};
#[cfg(unix)]
use cap_std::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use fs4::fs_std::FileExt;
use reasonix_core::fileutil::{MAX_REPLACE_RETRIES, REPLACE_RETRY_BASE_MS, mkdir_all_mode};

use super::Result;
use super::disk::{TRANSACTION_LOCK_NAME, trim_session_path};
use super::ids::new_random_id;
use super::types::Error;

pub(super) const DISK_LOCK_WAIT: Duration = Duration::from_secs(5);
pub(super) const DISK_LOCK_POLL: Duration = Duration::from_millis(20);

#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
#[cfg(windows)]
const FILE_SHARE_ALL: u32 = 0x0000_0007;

pub(super) fn ensure_private_dir(path: &Path) -> Result<Dir> {
    let parent_path = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path.file_name().ok_or_else(|| Error::NotRegularFile {
        path: path.to_path_buf(),
    })?;
    mkdir_all_mode(parent_path, 0o700)
        .map_err(|source| io_error("create inbox parent", parent_path, source))?;
    let parent = Dir::open_ambient_dir(parent_path, ambient_authority())
        .map_err(|source| io_error("open inbox parent", parent_path, source))?;
    open_private_child_dir(&parent, Path::new(name), path)
}

pub(super) fn validate_private_dir(path: &Path) -> Result<Dir> {
    open_inbox_dir(path)
}

fn open_inbox_dir(path: &Path) -> Result<Dir> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let parent = parent.unwrap_or_else(|| Path::new("."));
    let name = path.file_name().ok_or_else(|| Error::NotRegularFile {
        path: path.to_path_buf(),
    })?;
    let parent = Dir::open_ambient_dir(parent, ambient_authority())
        .map_err(|source| io_error("open inbox parent", parent, source))?;
    open_child_dir(&parent, Path::new(name), path)
}

pub(super) fn validate_private_child_dir(root: &Dir, name: &Path, path: &Path) -> Result<Dir> {
    match root.symlink_metadata(name) {
        Ok(metadata) if metadata.is_dir() => open_child_dir(root, name, path),
        Ok(_) => Err(Error::NotRegularFile {
            path: path.to_path_buf(),
        }),
        Err(source) => Err(io_error("inspect private directory", path, source)),
    }
}

pub(super) fn open_private_child_dir(root: &Dir, name: &Path, path: &Path) -> Result<Dir> {
    match root.symlink_metadata(name) {
        Ok(metadata) if metadata.is_dir() => open_child_dir(root, name, path),
        Ok(_) => Err(Error::NotRegularFile {
            path: path.to_path_buf(),
        }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            let builder = cap_std::fs::DirBuilder::new();
            #[cfg(unix)]
            let mut builder = builder;
            #[cfg(unix)]
            builder.mode(0o700);
            match root.create_dir_with(name, &builder) {
                Ok(()) => open_child_dir(root, name, path),
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                    open_child_dir(root, name, path)
                }
                Err(source) => Err(io_error("create private directory", path, source)),
            }
        }
        Err(source) => Err(io_error("inspect private directory", path, source)),
    }
}

fn open_child_dir(root: &Dir, name: &Path, path: &Path) -> Result<Dir> {
    let before = root
        .symlink_metadata(name)
        .map_err(|source| io_error("inspect private directory", path, source))?;
    if before.is_symlink() || !before.is_dir() {
        return Err(Error::NotRegularFile {
            path: path.to_path_buf(),
        });
    }
    let file = open_entry_nofollow(root, name, path, true)?;
    let opened = file
        .metadata()
        .map_err(|source| io_error("inspect opened inbox directory", path, source))?;
    if !same_file(&before, &opened) {
        return Err(Error::FileChanged {
            path: path.to_path_buf(),
        });
    }
    Ok(Dir::from_std_file(file.into_std()))
}

fn open_entry_nofollow(
    root: &Dir,
    name: &Path,
    path: &Path,
    maybe_dir: bool,
) -> Result<cap_std::fs::File> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(windows)]
    {
        let mut flags = FILE_FLAG_OPEN_REPARSE_POINT;
        if maybe_dir {
            flags |= FILE_FLAG_BACKUP_SEMANTICS;
        }
        options.custom_flags(flags).share_mode(FILE_SHARE_ALL);
    }
    let file = root
        .open_with(name, &options)
        .map_err(|source| io_error("open inbox entry", path, source))?;
    let metadata = file
        .metadata()
        .map_err(|source| io_error("inspect opened inbox entry", path, source))?;
    if metadata.is_symlink()
        || (maybe_dir && !metadata.is_dir())
        || (!maybe_dir && !metadata.is_file())
    {
        return Err(Error::NotRegularFile {
            path: path.to_path_buf(),
        });
    }
    Ok(file)
}

pub(super) fn read_regular_file_in(
    dir: &Dir,
    name: &Path,
    path: &Path,
    max_bytes: u64,
) -> Result<Vec<u8>> {
    let before = dir
        .symlink_metadata(name)
        .map_err(|source| io_error("inspect regular file", path, source))?;
    if !before.is_file() {
        return Err(Error::NotRegularFile {
            path: path.to_path_buf(),
        });
    }
    if max_bytes > 0 && before.len() > max_bytes {
        return Err(Error::TooLarge {
            path: path.to_path_buf(),
            max_bytes,
        });
    }
    let mut file = open_entry_nofollow(dir, name, path, false)?;
    let opened = file
        .metadata()
        .map_err(|source| io_error("inspect opened regular file", path, source))?;
    if !same_file(&before, &opened) || before.len() != opened.len() {
        return Err(Error::FileChanged {
            path: path.to_path_buf(),
        });
    }
    let mut data = Vec::new();
    if max_bytes > 0 {
        (&mut file)
            .take(max_bytes.saturating_add(1))
            .read_to_end(&mut data)
            .map_err(|source| io_error("read regular file", path, source))?;
    } else {
        file.read_to_end(&mut data)
            .map_err(|source| io_error("read regular file", path, source))?;
    }
    if max_bytes > 0 && u64::try_from(data.len()).unwrap_or(u64::MAX) > max_bytes {
        return Err(Error::TooLarge {
            path: path.to_path_buf(),
            max_bytes,
        });
    }
    Ok(data)
}

fn same_file(left: &cap_std::fs::Metadata, right: &cap_std::fs::Metadata) -> bool {
    left.dev() == right.dev() && left.ino() == right.ino()
}

pub(super) fn write_atomic_file(dir: &Dir, name: &Path, path: &Path, data: &[u8]) -> Result<()> {
    validate_local_name(name)?;
    validate_destination(dir, name, path)?;
    let temp_name = format!(".{}.{}.tmp", name.to_string_lossy(), new_random_id());
    let temp_path = path.with_file_name(&temp_name);
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    #[cfg(unix)]
    options.mode(0o600);
    #[cfg(windows)]
    options.share_mode(FILE_SHARE_ALL);
    let mut file = dir
        .open_with(&temp_name, &options)
        .map_err(|source| io_error("create atomic temp", &temp_path, source))?;
    #[cfg(unix)]
    file.set_permissions(cap_std::fs::Permissions::from_mode(0o600))
        .map_err(|source| io_error("set atomic temp permissions", &temp_path, source))?;
    let prepared = (|| {
        file.write_all(data)
            .map_err(|source| io_error("write atomic temp", &temp_path, source))?;
        file.sync_all()
            .map_err(|source| io_error("sync atomic temp", &temp_path, source))?;
        Ok::<(), Error>(())
    })();
    drop(file);
    if let Err(err) = prepared {
        let _ = dir.remove_file(&temp_name);
        return Err(err);
    }

    for attempt in 0..=MAX_REPLACE_RETRIES {
        validate_destination(dir, name, path)?;
        match dir.rename(&temp_name, dir, name) {
            Ok(()) => {
                sync_directory(dir);
                return Ok(());
            }
            Err(source) if rename_crosses_device(&source) => {
                let _ = dir.remove_file(&temp_name);
                return Err(io_error("publish atomic file", path, source));
            }
            Err(_source) if attempt < MAX_REPLACE_RETRIES => {
                std::thread::sleep(Duration::from_millis(
                    (u64::from(attempt) + 1) * REPLACE_RETRY_BASE_MS,
                ));
            }
            Err(source) => {
                let _ = dir.remove_file(&temp_name);
                return Err(io_error("publish atomic file", path, source));
            }
        }
    }
    unreachable!("rename loop returns on every path")
}

fn validate_destination(dir: &Dir, name: &Path, path: &Path) -> Result<()> {
    match dir.symlink_metadata(name) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(Error::NotRegularFile {
            path: path.to_path_buf(),
        }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error("inspect atomic destination", path, source)),
    }
}

fn validate_local_name(name: &Path) -> Result<()> {
    let mut components = name.components();
    if matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none() {
        Ok(())
    } else {
        Err(Error::InvalidItemPath)
    }
}

fn sync_directory(dir: &Dir) {
    if let Ok(copy) = dir.try_clone() {
        let _ = copy.into_std_file().sync_all();
    }
}

fn rename_crosses_device(err: &io::Error) -> bool {
    if err.kind() == io::ErrorKind::CrossesDevices {
        return true;
    }
    match err.raw_os_error() {
        #[cfg(windows)]
        Some(17) | Some(18) => true,
        #[cfg(not(windows))]
        Some(18) => true,
        _ => false,
    }
}

#[derive(Debug)]
pub(super) struct TransactionLock {
    file: File,
}

impl Drop for TransactionLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

pub(super) fn acquire_transaction_lock_deadline(
    inbox: &Path,
    deadline: Instant,
    poll: Duration,
) -> Result<TransactionLock> {
    let dir = ensure_private_dir(inbox)?;
    let path = inbox.join(TRANSACTION_LOCK_NAME);
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .follow(FollowSymlinks::No);
    #[cfg(unix)]
    options.mode(0o600);
    #[cfg(windows)]
    options.share_mode(FILE_SHARE_ALL);
    let file = dir
        .open_with(TRANSACTION_LOCK_NAME, &options)
        .map_err(|source| io_error("open transaction lock", &path, source))?;
    if !file
        .metadata()
        .map_err(|source| io_error("inspect transaction lock", &path, source))?
        .is_file()
    {
        return Err(Error::NotRegularFile { path: path.clone() });
    }
    let file = file.into_std();
    loop {
        match file.try_lock_exclusive() {
            Ok(true) => return Ok(TransactionLock { file }),
            Ok(false) => {}
            Err(source) => return Err(io_error("acquire transaction lock", &path, source)),
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(Error::LockTimeout { path });
        }
        std::thread::sleep(poll.min(deadline.saturating_duration_since(now)));
    }
}

pub(super) fn quarantine_name(base: &OsStr, tag: Option<&str>) -> OsString {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut destination = base.to_os_string();
    if let Some(tag) = tag {
        destination.push(".");
        destination.push(tag);
    }
    destination.push(".");
    destination.push(nanos.to_string());
    destination
}

pub(super) fn io_error(op: &'static str, path: &Path, source: io::Error) -> Error {
    Error::Io {
        op,
        path: path.to_path_buf(),
        source,
    }
}

pub(super) fn is_not_found(err: &Error) -> bool {
    matches!(err, Error::Io { source, .. } if source.kind() == io::ErrorKind::NotFound)
}

pub(super) fn remove_inbox_dir(session_path: &Path) -> Result<()> {
    let session_path = trim_session_path(session_path.as_os_str());
    if session_path.is_empty() {
        return Ok(());
    }
    let inbox_path = crate::store::naming::session_inbox_dir(Path::new(&session_path));
    let parent_path = inbox_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = inbox_path
        .file_name()
        .ok_or_else(|| Error::NotRegularFile {
            path: inbox_path.clone(),
        })?;
    let parent = match Dir::open_ambient_dir(parent_path, ambient_authority()) {
        Ok(parent) => parent,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(io_error("open inbox parent", parent_path, source)),
    };
    let before = match parent.symlink_metadata(name) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(io_error("inspect inbox directory", &inbox_path, source)),
    };
    if before.is_symlink() || !before.is_dir() {
        return Err(Error::NotRegularFile {
            path: inbox_path.clone(),
        });
    }
    let inbox = open_child_dir(&parent, Path::new(name), &inbox_path)?;
    remove_tree(&inbox, &inbox_path)?;
    let after = match parent.symlink_metadata(name) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(io_error("inspect inbox directory", &inbox_path, source)),
    };
    if !same_file(&before, &after) {
        return Err(Error::FileChanged {
            path: inbox_path.clone(),
        });
    }
    match parent.remove_dir(name) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error("remove inbox directory", &inbox_path, source)),
    }
}

fn remove_tree(dir: &Dir, path: &Path) -> Result<()> {
    let entries = dir
        .entries()
        .map_err(|source| io_error("read inbox directory", path, source))?;
    for entry in entries {
        let entry = entry.map_err(|source| io_error("read inbox entry", path, source))?;
        let file_name = entry.file_name();
        let name = Path::new(&file_name);
        let entry_path = path.join(name);
        let file_type = entry
            .file_type()
            .map_err(|source| io_error("inspect inbox entry", &entry_path, source))?;
        if file_type.is_dir() {
            let child = open_child_dir(dir, name, &entry_path)?;
            remove_tree(&child, &entry_path)?;
            match dir.remove_dir(name) {
                Ok(()) => {}
                Err(source) if source.kind() == io::ErrorKind::NotFound => {}
                Err(source) => return Err(io_error("remove inbox directory", &entry_path, source)),
            }
        } else {
            match dir.remove_file(name) {
                Ok(()) => {}
                Err(source) if source.kind() == io::ErrorKind::NotFound => {}
                Err(source) => return Err(io_error("remove inbox file", &entry_path, source)),
            }
        }
    }
    Ok(())
}
