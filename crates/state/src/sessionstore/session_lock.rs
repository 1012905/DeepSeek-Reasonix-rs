//! Session file locks (Go: `session_lock_windows.go`,
//! `session_lock_unix.go`).
//!
//! Cross-process exclusive lock on the `.jsonl.lock` sidecar of a
//! session transcript, held for the duration of a save. Unlike the
//! generic file lock in reasonix-core, the Go session lock polls at its
//! own cadence, opens the sidecar with mode 0600, and maps only the
//! sharing/lock-violation family to the `Held` error; every other
//! failure surfaces as-is. The save-path in-process mutex lives in
//! `canonical.rs` (`with_session_save_path_lock`).

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use thiserror::Error;

/// How long a save waits for a busy session file lock (Go:
/// `sessionFileLockWait`).
const SESSION_FILE_LOCK_WAIT: Duration = Duration::from_secs(5);

/// Interval between lock attempts while waiting (Go:
/// `sessionFileLockPollInterval`).
const SESSION_FILE_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Errors from taking the session file lock (Go: the
/// `ErrSessionFileLockHeld` sentinel and everything else).
#[derive(Debug, Error)]
pub enum SessionFileLockError {
    /// The lock is held by another runtime (Go:
    /// `ErrSessionFileLockHeld`).
    #[error("session file lock held")]
    Held,
    /// Any other filesystem failure.
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// A held session file lock. Dropping releases the OS lock (Go: the
/// returned unlock closure).
#[derive(Debug)]
pub struct SessionFileLockGuard {
    file: File,
}

impl Drop for SessionFileLockGuard {
    fn drop(&mut self) {
        let _ = fs4::fs_std::FileExt::unlock(&self.file);
    }
}

fn open_lock_file(path: &Path) -> io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .mode(0o600)
            .open(path)
    }
    #[cfg(not(unix))]
    {
        OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)
    }
}

/// Maps an open/lock failure to the Go error split: sharing violations
/// on Windows (and would-block on Unix, should the crate surface it)
/// mean the lock is held; anything else is a plain IO error.
fn map_lock_error(err: io::Error) -> SessionFileLockError {
    #[cfg(windows)]
    {
        match err.raw_os_error() {
            // ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION.
            Some(32) | Some(33) => SessionFileLockError::Held,
            _ => SessionFileLockError::Io(err),
        }
    }
    #[cfg(not(windows))]
    {
        if err.kind() == io::ErrorKind::WouldBlock {
            SessionFileLockError::Held
        } else {
            SessionFileLockError::Io(err)
        }
    }
}

/// Takes the session file lock without waiting (Go:
/// `tryLockSessionFile`). A lock held by any other runtime — or
/// another thread of this one — reports `Held` immediately.
pub fn try_lock_session_file(
    session_path: &Path,
) -> Result<SessionFileLockGuard, SessionFileLockError> {
    let lock_path = crate::store::naming::session_lock_file(session_path);
    let file = open_lock_file(&lock_path).map_err(map_lock_error)?;
    match fs4::fs_std::FileExt::try_lock_exclusive(&file) {
        Ok(true) => Ok(SessionFileLockGuard { file }),
        Ok(false) => Err(SessionFileLockError::Held),
        Err(err) => Err(map_lock_error(err)),
    }
}

/// Waits up to the session lock deadline for the lock (Go:
/// `lockSessionFile`): polls the lock, returns non-`Held`
/// failures immediately, and reports `Held` once the deadline
/// lapses.
pub fn lock_session_file(
    session_path: &Path,
) -> Result<SessionFileLockGuard, SessionFileLockError> {
    lock_session_file_deadline(
        session_path,
        SESSION_FILE_LOCK_WAIT,
        SESSION_FILE_LOCK_POLL_INTERVAL,
    )
}

/// The deadline loop of `lock_session_file` with injectable timing,
/// so tests can exercise the timeout without the production wait.
fn lock_session_file_deadline(
    session_path: &Path,
    wait: Duration,
    poll: Duration,
) -> Result<SessionFileLockGuard, SessionFileLockError> {
    let poll = if poll.is_zero() {
        Duration::from_millis(1)
    } else {
        poll
    };
    let deadline = Instant::now() + wait;
    loop {
        match try_lock_session_file(session_path) {
            Ok(guard) => return Ok(guard),
            Err(SessionFileLockError::Held) => {}
            Err(err) => return Err(err),
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(SessionFileLockError::Held);
        }
        std::thread::sleep(poll.min(deadline - now));
    }
}

/// A take-then-hold OS lock over an arbitrary session lock file (Go:
/// `sessionLockFile` plus `tryTakeSessionLockFile`).
///
/// Unlike `SessionFileLockGuard`, the lock file path is retained so
/// the file can be removed while the lock is still held (Go:
/// `RemoveAndUnlock`). On Windows the Rust standard library opens
/// files with `FILE_SHARE_DELETE`, so removing the path under a
/// held handle succeeds and the name disappears immediately; Go needs
/// the `FileDispositionInfo` dance plus a fallback counter for the
/// same effect. The path-based remove is the primary mechanism here:
/// no disposition fallback, no adoption window.
#[derive(Debug)]
pub(crate) struct SessionLockFile {
    file: File,
    path: PathBuf,
}

impl SessionLockFile {
    /// Release the OS lock, keeping the lock file (Go: `Unlock`).
    pub(crate) fn unlock(self) {
        // Dropping the handle releases the fs4 lock and closes the
        // file.
        drop(self.file);
    }

    /// Remove the lock file while the lock is still held, then
    /// release the OS lock (Go: `RemoveAndUnlock`). A missing
    /// file is tolerated.
    pub(crate) fn remove_and_unlock(self) -> io::Result<()> {
        let result = match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        };
        drop(self.file);
        result
    }
}

/// Take the OS lock at `lock_path`, creating the file when missing
/// (Go: `tryTakeSessionLockFile`): a plain exclusive lock whose
/// file can be removed under the held handle.
pub(crate) fn try_take_session_lock_file(
    lock_path: &Path,
) -> Result<SessionLockFile, SessionFileLockError> {
    let file = open_lock_file(lock_path).map_err(map_lock_error)?;
    match fs4::fs_std::FileExt::try_lock_exclusive(&file) {
        Ok(true) => Ok(SessionLockFile {
            file,
            path: lock_path.to_path_buf(),
        }),
        Ok(false) => Err(SessionFileLockError::Held),
        Err(err) => Err(map_lock_error(err)),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::sessionstore::eventlog::test_support::tempdir;

    #[test]
    fn second_lock_attempt_reports_held() {
        let dir = tempdir();
        let session = dir.path().join("a.jsonl");
        let _first = try_lock_session_file(&session).expect("first lock");
        match try_lock_session_file(&session) {
            Err(SessionFileLockError::Held) => {}
            other => panic!("expected held, got {other:?}"),
        }
    }

    #[test]
    fn lock_is_released_on_drop() {
        let dir = tempdir();
        let session = dir.path().join("b.jsonl");
        {
            let _guard = try_lock_session_file(&session).expect("first lock");
        }
        let _again = try_lock_session_file(&session).expect("relock after drop");
    }

    #[test]
    fn lock_waits_out_deadline_then_reports_held() {
        let dir = tempdir();
        let session = dir.path().join("c.jsonl");
        let _held = try_lock_session_file(&session).expect("first lock");
        let start = Instant::now();
        let err = lock_session_file_deadline(
            &session,
            Duration::from_millis(60),
            Duration::from_millis(10),
        )
        .expect_err("deadline must lapse");
        assert!(matches!(err, SessionFileLockError::Held));
        assert!(start.elapsed() >= Duration::from_millis(50));
    }

    #[test]
    fn lock_session_file_takes_a_free_lock() {
        let dir = tempdir();
        let session = dir.path().join("d.jsonl");
        let _guard = lock_session_file(&session).expect("free lock");
    }
}
