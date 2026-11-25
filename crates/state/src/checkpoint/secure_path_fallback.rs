//! Validation is lexical; plain path operations do not provide race confinement.

use std::fs::{self, File};
use std::io;
use std::path::Path;

use reasonix_core::fileutil::set_file_mode;

use super::{SecureError, remove_plain, write_new_file_at};

pub(super) fn open_workspace_file(_root: &Path, absolute: &Path) -> Result<File, SecureError> {
    File::open(absolute).map_err(|source| SecureError::io("open", absolute, source))
}

pub(super) fn write_new_file(
    _root: &Path,
    absolute: &Path,
    bytes: &[u8],
    mode: u32,
) -> Result<(), SecureError> {
    write_new_file_at(absolute, bytes, mode)
}

pub(super) fn rename(_root: &Path, from: &Path, to: &Path) -> Result<(), SecureError> {
    fs::rename(from, to).map_err(|source| SecureError::Rename {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
        source,
    })
}

pub(super) fn remove(_root: &Path, absolute: &Path) -> Result<(), SecureError> {
    remove_plain(absolute)
}

pub(super) fn chmod(_root: &Path, absolute: &Path, mode: u32) -> Result<(), SecureError> {
    set_file_mode(absolute, mode).map_err(|source| SecureError::io("chmod", absolute, source))
}

pub(super) fn path_exists(_root: &Path, absolute: &Path) -> Result<bool, SecureError> {
    match fs::symlink_metadata(absolute) {
        Ok(_) => Ok(true),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(SecureError::io("inspect", absolute, source)),
    }
}

pub(super) fn read_file(_root: &Path, absolute: &Path) -> Result<Vec<u8>, SecureError> {
    fs::read(absolute).map_err(|source| SecureError::io("read", absolute, source))
}
