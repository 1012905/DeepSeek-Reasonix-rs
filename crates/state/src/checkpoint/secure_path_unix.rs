use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::ambient_authority;
use cap_std::fs::{Dir, DirBuilder, DirBuilderExt as _, OpenOptions, OpenOptionsExt as _};
use std::os::unix::fs::PermissionsExt;

use super::{SecureError, remove_plain, write_new_file_at};

fn open_parent(root: &Path, absolute: &Path, create: bool) -> Result<(Dir, OsString), SecureError> {
    let relative = absolute.strip_prefix(root).map_err(|_| {
        SecureError::Path(super::PathError::OutsideWorkspace {
            path: absolute.to_path_buf(),
            root: root.to_path_buf(),
        })
    })?;
    let parts: Vec<OsString> = relative.iter().map(|part| part.to_os_string()).collect();
    let Some((base, parents)) = parts.split_last() else {
        return Err(SecureError::WorkspaceRoot);
    };

    // The configured root is the trust boundary and may itself be a symlink.
    let mut dir = Dir::open_ambient_dir(root, ambient_authority())
        .map_err(|source| SecureError::io("open workspace root", root, source))?;
    for part in parents {
        let part = Path::new(part);
        let opened = open_child_dir(&dir, part);
        dir = match opened {
            Ok(next) => next,
            Err(source) if create && source.kind() == io::ErrorKind::NotFound => {
                let mut builder = DirBuilder::new();
                builder.mode(0o755);
                match dir.create_dir_with(part, &builder) {
                    Ok(()) => {}
                    Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(source) => {
                        return Err(SecureError::io("create workspace directory", part, source));
                    }
                }
                open_child_dir(&dir, part)
                    .map_err(|source| SecureError::io("open workspace directory", part, source))?
            }
            Err(source) => {
                return Err(SecureError::io("open workspace directory", part, source));
            }
        };
    }
    Ok((dir, base.clone()))
}

fn open_child_dir(dir: &Dir, part: &Path) -> io::Result<Dir> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = dir.open_with(part, &options)?;
    if !file.metadata()?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("{}: path component is not a directory", part.display()),
        ));
    }
    Ok(Dir::from_std_file(file.into_std()))
}

fn open_at(dir: &Dir, base: &Path, absolute: &Path, op: &'static str) -> Result<File, SecureError> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    dir.open_with(base, &options)
        .map(|file| file.into_std())
        .map_err(|source| SecureError::io(op, absolute, source))
}

pub(super) fn open_workspace_file(root: &Path, absolute: &Path) -> Result<File, SecureError> {
    if root.as_os_str().is_empty() {
        return File::open(absolute).map_err(|source| SecureError::io("open", absolute, source));
    }
    let (parent, base) = open_parent(root, absolute, false)?;
    open_at(&parent, Path::new(&base), absolute, "open")
}

pub(super) fn write_new_file(
    root: &Path,
    absolute: &Path,
    bytes: &[u8],
    mode: u32,
) -> Result<(), SecureError> {
    if root.as_os_str().is_empty() {
        return write_new_file_at(absolute, bytes, mode);
    }
    let (parent, base) = open_parent(root, absolute, true)?;
    let base = Path::new(&base);
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No)
        .mode(mode & 0o777);
    let mut file = parent
        .open_with(base, &options)
        .map_err(|source| SecureError::io("create", absolute, source))?;
    let result = (|| {
        file.write_all(bytes)
            .map_err(|source| SecureError::io("write", absolute, source))?;
        file.sync_all()
            .map_err(|source| SecureError::io("sync", absolute, source))
    })();
    drop(file);
    if result.is_err() {
        let _ = parent.remove_file(base);
    }
    result
}

pub(super) fn rename(root: &Path, from: &Path, to: &Path) -> Result<(), SecureError> {
    if root.as_os_str().is_empty() {
        return fs::rename(from, to).map_err(|source| SecureError::Rename {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
            source,
        });
    }
    let (old_parent, old_base) = open_parent(root, from, false)?;
    let (new_parent, new_base) = open_parent(root, to, true)?;
    old_parent
        .rename(Path::new(&old_base), &new_parent, Path::new(&new_base))
        .map_err(|source| SecureError::Rename {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
            source,
        })
}

pub(super) fn remove(root: &Path, absolute: &Path) -> Result<(), SecureError> {
    if root.as_os_str().is_empty() {
        return remove_plain(absolute);
    }
    let (parent, base) = open_parent(root, absolute, false)?;
    parent
        .remove_file(Path::new(&base))
        .map_err(|source| SecureError::io("remove", absolute, source))
}

pub(super) fn chmod(root: &Path, absolute: &Path, mode: u32) -> Result<(), SecureError> {
    let file = open_workspace_file(root, absolute)?;
    file.set_permissions(fs::Permissions::from_mode(mode))
        .map_err(|source| SecureError::io("chmod", absolute, source))
}

pub(super) fn path_exists(root: &Path, absolute: &Path) -> Result<bool, SecureError> {
    match open_workspace_file(root, absolute) {
        Ok(_) => Ok(true),
        Err(error) if error.is_not_found() => Ok(false),
        Err(error) => Err(error),
    }
}

pub(super) fn read_file(root: &Path, absolute: &Path) -> Result<Vec<u8>, SecureError> {
    let mut file = open_workspace_file(root, absolute)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|source| SecureError::io("read", absolute, source))?;
    Ok(bytes)
}
