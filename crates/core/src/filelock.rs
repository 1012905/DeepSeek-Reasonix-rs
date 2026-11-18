//! Cross-process file locks with a process-local re-entry registry.
//!
//! Port of the Go package internal/base/filelock. Every lock path is first
//! taken in a process-local registry (so two threads in one process serialize
//! the same way two processes do) and then on the filesystem itself: flock on
//! Unix, LockFileEx on Windows. Error strings match the Go implementation
//! verbatim because callers wrap them into user-visible diagnostics.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use thiserror::Error;

/// Interval between filesystem lock attempts while waiting for a busy lock.
const RETRY_INTERVAL: Duration = Duration::from_millis(20);

/// Errors returned by [`acquire`] and [`try_acquire`].
///
/// Display texts match the Go package (internal/base/filelock) so wrapped
/// diagnostics stay byte-identical.
#[derive(Debug, Error)]
pub enum FileLockError {
    #[error("file lock path is empty")]
    EmptyPath,
    #[error("resolve file lock path: {0}")]
    ResolvePath(#[source] std::io::Error),
    #[error("file lock held")]
    Held,
    #[error("acquire file lock: context deadline exceeded")]
    DeadlineExceeded,
    #[error("acquire file lock: {0}")]
    Acquire(#[source] std::io::Error),
    #[error("try acquire file lock: {0}")]
    TryAcquire(#[source] std::io::Error),
}

/// Outcome of one non-blocking attempt on the filesystem lock.
enum FileTry {
    Locked(File),
    Conflict,
    Other(std::io::Error),
}

fn registry() -> &'static Mutex<HashMap<PathBuf, Arc<LocalLock>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Arc<LocalLock>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// One process-local slot: [`token`] is held while a thread in this
/// process owns the lock, and [`refs`] counts owners. Like Go's
/// localRegistry, the registry entry is evicted when the last owner releases.
/// The registry is only an optimization: the filesystem lock is the real
/// mutual exclusion, so the (inherited) race between refcount bump and token
/// release can never grant the same file lock twice — the OS lock arbitrates.
struct LocalLock {
    canonical: PathBuf,
    token: Mutex<bool>,
    released: Condvar,
    refs: AtomicU64,
}

impl LocalLock {
    fn new(canonical: PathBuf) -> Self {
        Self {
            canonical,
            token: Mutex::new(false),
            released: Condvar::new(),
            refs: AtomicU64::new(0),
        }
    }

    /// Free the process-local token and drop this owner's reference. The
    /// token mutex is always released before the registry lock is taken, so
    /// the registry→token ordering in [`try_acquire_local`] cannot
    /// deadlock.
    fn release(self: &Arc<Self>) {
        {
            let mut token = self.token.lock().unwrap();
            *token = false;
        }
        self.released.notify_all();
        self.discard_ref();
    }

    /// Drop this owner's reference without touching the token (used when the
    /// token was never acquired, e.g. a timed-out wait).
    fn discard_ref(self: &Arc<Self>) {
        let mut map = registry().lock().unwrap();
        let refs = self.refs.fetch_sub(1, Ordering::SeqCst) - 1;
        if refs == 0 {
            map.remove(&self.canonical);
        }
    }
}

/// Normalize a lock path for registry lookups: trim, absolutize, and on
/// Windows also normalize separators and lowercase (Go's ToLower(ToSlash)).
fn canonical_lock_path(path: &Path) -> Result<PathBuf, FileLockError> {
    let trimmed = path.as_os_str().to_string_lossy().trim().to_string();
    if trimmed.is_empty() {
        return Err(FileLockError::EmptyPath);
    }
    let absolute = std::path::absolute(&trimmed).map_err(FileLockError::ResolvePath)?;
    #[cfg(windows)]
    {
        let normalized = absolute.to_string_lossy().replace('/', "\\").to_lowercase();
        Ok(PathBuf::from(normalized))
    }
    #[cfg(not(windows))]
    {
        Ok(absolute)
    }
}

fn acquire_local(canonical: &Path, deadline: Instant) -> Result<Arc<LocalLock>, FileLockError> {
    let lock = {
        let mut map = registry().lock().unwrap();
        let entry = map
            .entry(canonical.to_path_buf())
            .or_insert_with(|| Arc::new(LocalLock::new(canonical.to_path_buf())))
            .clone();
        entry.refs.fetch_add(1, Ordering::SeqCst);
        entry
    };
    let acquired = {
        let mut token = lock.token.lock().unwrap();
        loop {
            if !*token {
                *token = true;
                break true;
            }
            let now = Instant::now();
            if now >= deadline {
                break false;
            }
            let (next, _) = lock.released.wait_timeout(token, deadline - now).unwrap();
            token = next;
        }
    };
    if acquired {
        Ok(lock)
    } else {
        lock.discard_ref();
        Err(FileLockError::DeadlineExceeded)
    }
}

fn try_acquire_local(canonical: &Path) -> Result<Arc<LocalLock>, FileLockError> {
    let mut map = registry().lock().unwrap();
    let entry = map
        .entry(canonical.to_path_buf())
        .or_insert_with(|| Arc::new(LocalLock::new(canonical.to_path_buf())))
        .clone();
    {
        let mut token = entry.token.lock().unwrap();
        if *token {
            return Err(FileLockError::Held);
        }
        *token = true;
    }
    entry.refs.fetch_add(1, Ordering::SeqCst);
    Ok(entry)
}

fn try_lock_file(canonical: &Path) -> FileTry {
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = match options.open(canonical) {
        Ok(file) => file,
        Err(err) => return FileTry::Other(err),
    };
    use fs4::fs_std::FileExt;
    match FileExt::try_lock_exclusive(&file) {
        Ok(true) => FileTry::Locked(file),
        Ok(false) => FileTry::Conflict,
        Err(err) => FileTry::Other(err),
    }
}

/// An owned cross-process file lock. Dropping it releases the filesystem lock
/// and the process-local slot.
pub struct FileLockGuard {
    file: File,
    local: Arc<LocalLock>,
}

impl std::fmt::Debug for FileLockGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileLockGuard").finish_non_exhaustive()
    }
}

impl Drop for FileLockGuard {
    fn drop(&mut self) {
        use fs4::fs_std::FileExt;
        let _ = FileExt::unlock(&self.file);
        self.local.release();
    }
}

/// Acquire an exclusive lock on [`path`], waiting up to [`timeout`].
///
/// Mirrors Go's filelock.Acquire: first the process-local registry, then the
/// filesystem lock, polling every 20ms until the deadline expires.
pub fn acquire(path: &Path, timeout: Duration) -> Result<FileLockGuard, FileLockError> {
    let canonical = canonical_lock_path(path)?;
    let deadline = Instant::now() + timeout;
    let local = acquire_local(&canonical, deadline)?;
    loop {
        match try_lock_file(&canonical) {
            FileTry::Locked(file) => return Ok(FileLockGuard { file, local }),
            FileTry::Conflict => {
                let now = Instant::now();
                if now >= deadline {
                    local.release();
                    return Err(FileLockError::DeadlineExceeded);
                }
                let wait = RETRY_INTERVAL.min(deadline - now);
                std::thread::sleep(wait);
            }
            FileTry::Other(err) => {
                local.release();
                return Err(FileLockError::Acquire(err));
            }
        }
    }
}

/// Try to acquire an exclusive lock on [`path`] without waiting.
pub fn try_acquire(path: &Path) -> Result<FileLockGuard, FileLockError> {
    let canonical = canonical_lock_path(path)?;
    let local = try_acquire_local(&canonical)?;
    match try_lock_file(&canonical) {
        FileTry::Locked(file) => Ok(FileLockGuard { file, local }),
        FileTry::Conflict => {
            local.release();
            Err(FileLockError::Held)
        }
        FileTry::Other(err) => {
            local.release();
            Err(FileLockError::TryAcquire(err))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_base() -> PathBuf {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn empty_path_is_rejected() {
        assert!(matches!(
            try_acquire(Path::new("   ")),
            Err(FileLockError::EmptyPath)
        ));
        assert!(matches!(
            acquire(Path::new(""), Duration::from_millis(10)),
            Err(FileLockError::EmptyPath)
        ));
    }

    #[test]
    fn acquire_and_try_acquire_round_trip() {
        let dir = tempfile::Builder::new().tempdir_in(temp_base()).unwrap();
        let lock = dir.path().join("a.lock");
        let guard = acquire(&lock, Duration::from_secs(5)).unwrap();
        assert!(matches!(try_acquire(&lock), Err(FileLockError::Held)));
        drop(guard);
        assert!(try_acquire(&lock).is_ok());
    }

    #[test]
    fn local_waiter_times_out_then_succeeds_after_release() {
        let dir = tempfile::Builder::new().tempdir_in(temp_base()).unwrap();
        let lock = dir.path().join("b.lock");
        let held = try_acquire(&lock).unwrap();
        let start = Instant::now();
        let result = acquire(&lock, Duration::from_millis(150));
        assert!(matches!(result, Err(FileLockError::DeadlineExceeded)));
        assert!(start.elapsed() >= Duration::from_millis(120));
        drop(held);
        acquire(&lock, Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn local_waiter_wakes_on_release() {
        let dir = tempfile::Builder::new().tempdir_in(temp_base()).unwrap();
        let lock = dir.path().join("c.lock");
        let held = try_acquire(&lock).unwrap();
        let lock2 = lock.clone();
        let waiter = std::thread::spawn(move || acquire(&lock2, Duration::from_secs(5)));
        std::thread::sleep(Duration::from_millis(150));
        assert!(!waiter.is_finished());
        drop(held);
        waiter.join().unwrap().unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn canonical_lock_path_is_case_insensitive_on_windows() {
        let a = canonical_lock_path(Path::new("C:\\Windows\\Temp\\CaseTest.LOCK")).unwrap();
        let b = canonical_lock_path(Path::new("c:/windows/temp/casetest.lock")).unwrap();
        assert_eq!(a, b);
    }
}
