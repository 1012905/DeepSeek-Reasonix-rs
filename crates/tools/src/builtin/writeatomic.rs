//! Atomic publication for the file-writing tools (fileutil atomicwrite.go
//! AtomicCreateFile / AtomicOverwriteFile). reasonix-core::fileutil carries
//! AtomicWriteFile and ReplaceFile; the two entry points this slice needs
//! that core has not migrated yet live here, so core stays untouched.

#![forbid(unsafe_code)]

use std::fs::{self};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use reasonix_core::fileutil::{atomic_write_file, create_temp_file, mkdir_all_mode};

/// atomicwrite.go `AtomicCreateFile`: publishes a complete file only when
/// path is still absent. A concurrent writer that creates path wins and
/// its file is never replaced. The temp file is fsynced, then linked into
/// place — link creation is itself atomic and fails on an existing
/// destination (Go's link fallback; without FFI the rename-no-replace
/// syscall is out of reach, and link is Go's second publisher).
pub fn atomic_create_file(path: &Path, data: &[u8], perm: u32) -> io::Result<()> {
    let tmp_path = write_atomic_temp(path, data, perm)?;
    let publish = fs::hard_link(&tmp_path, path);
    let _ = fs::remove_file(&tmp_path);
    match publish {
        Ok(()) => Ok(()),
        Err(link_err) => Err(io::Error::other(format!(
            "publish new file {}: {link_err}",
            path.display()
        ))),
    }
}

/// atomicwrite.go `AtomicOverwriteFile`: replaces an existing file's
/// contents atomically while keeping the two properties a bare rename
/// drops: the file's current permission bits (an executable script must
/// not come back 0644) and the symlink target (a link must be written
/// through, not replaced by a regular file). `default_perm` applies only
/// when path does not exist yet.
pub fn atomic_overwrite_file(path: &Path, data: &[u8], default_perm: u32) -> io::Result<()> {
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let perm = fs::metadata(&target)
        .map(|info| unix_perm(&info, default_perm))
        .unwrap_or(default_perm);
    atomic_write_file(&target, data, perm)
}

/// atomicwrite.go `writeAtomicTemp`: temp in the target's directory, mode
/// tightened to 0700 for a private file, fsync before publish.
fn write_atomic_temp(path: &Path, data: &[u8], perm: u32) -> io::Result<PathBuf> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let dir_perm = if perm & 0o077 == 0 { 0o700 } else { 0o755 };
    mkdir_all_mode(&dir, dir_perm)
        .map_err(|e| io::Error::other(format!("create dir for {}: {e}", path.display())))?;
    let (mut tmp, tmp_path) = create_temp_file(&dir, ".atomic-", ".tmp")
        .map_err(|e| io::Error::other(format!("create tmp for {}: {e}", path.display())))?;
    if let Err(err) = (|| -> io::Result<()> {
        apply_perm(&mut tmp, perm)?;
        tmp.write_all(data)?;
        tmp.sync_all()?;
        Ok(())
    })() {
        let _ = fs::remove_file(&tmp_path);
        return Err(io::Error::other(format!(
            "write tmp for {}: {err}",
            path.display()
        )));
    }
    Ok(tmp_path)
}

fn apply_perm(file: &mut fs::File, perm: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(perm))
    }
    #[cfg(not(unix))]
    {
        let _ = (file, perm);
        Ok(())
    }
}

fn unix_perm(info: &fs::Metadata, default_perm: u32) -> u32 {
    #[cfg(unix)]
    {
        // The mode carries the permission; the default matters only to the
        // non-unix arm.
        let _ = default_perm;
        use std::os::unix::fs::PermissionsExt;
        info.permissions().mode() & 0o777
    }
    #[cfg(not(unix))]
    {
        let _ = info;
        default_perm
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(label: &str) -> (tempfile::TempDir, String) {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new()
            .prefix(label)
            .tempdir_in(&base)
            .unwrap();
        let path = dir.path().to_string_lossy().into_owned();
        (dir, path)
    }

    #[test]
    fn atomic_create_never_replaces() {
        let (_guard, dir) = tmp("writeatomic-create");
        let path = Path::new(&dir).join("new.txt");
        atomic_create_file(&path, b"one", 0o644).unwrap();
        // A concurrent writer that created the file first wins.
        atomic_create_file(&path, b"two", 0o644).unwrap_err();
        assert_eq!(fs::read(&path).unwrap(), b"one");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_overwrite_keeps_mode_and_content() {
        let (_guard, dir) = tmp("writeatomic-overwrite");
        let path = Path::new(&dir).join("script.sh");
        fs::write(&path, b"#!/bin/sh\nold\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        atomic_overwrite_file(&path, b"#!/bin/sh\nnew\n", 0o644).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"#!/bin/sh\nnew\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755, "overwrite must keep the file's own mode");
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
