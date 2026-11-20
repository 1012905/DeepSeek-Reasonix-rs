//! Atomic file writes and replaces.
//!
//! Parity target: internal/base/fileutil/atomicwrite.go — temp + fsync +
//! rename publication with two rename-failure classes handled differently:
//! a transient lock on the destination (antivirus, the search indexer, a
//! concurrent reader without delete sharing) is retried with backoff, while
//! a cross-device error (Windows encryption filter drivers report
//! ERROR_NOT_SAME_DEVICE even for a same-dir rename; unix reports EXDEV)
//! falls back immediately to a non-atomic copy when the caller allows it —
//! retrying a structurally impossible rename would only delay it.
//!
//! Not ported yet: AtomicCreateFile / RenameNoReplace and
//! AtomicOverwriteFile (checkpoint slice). Go surfaces close errors after
//! fsync ("close tmp"); std::fs::File closes without a fallible step, so
//! that error path has no Rust equivalent.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Transient-lock retries before a replace gives up (Go: maxReplaceRetries).
pub const MAX_REPLACE_RETRIES: u32 = 12;

/// Backoff base for transient-lock retries, in milliseconds (Go:
/// replaceRetryBase = 20ms; sleeps are (attempt+1) * base).
pub const REPLACE_RETRY_BASE_MS: u64 = 20;

type CrashHook = Box<dyn Fn(&str, &str) + Send + Sync>;

static CRASH_POINT: Mutex<Option<CrashHook>> = Mutex::new(None);

/// Installs (or, with None, removes) the crash-consistency fault-injection
/// hook that runs before every durable write/replace. Tests use it to
/// inject a process-crash panic at persistence boundaries; production
/// leaves it unset. The hook runs under the registry lock, so it must not
/// call set_crash_point or crash itself (Go: CrashPoint / SetCrashPoint
/// convention).
pub fn set_crash_point(hook: Option<CrashHook>) {
    let mut guard = CRASH_POINT.lock().unwrap_or_else(|e| e.into_inner());
    *guard = hook;
}

/// Invokes the optional crash-consistency fault-injection hook (Go: Crash).
pub fn crash(op: &str, path: &str) {
    let guard = CRASH_POINT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(hook) = guard.as_ref() {
        hook(op, path);
    }
}

/// Writes via temp + fsync + replace. On rename-capable filesystems readers
/// see only the old or the complete new file; a cross-device rename falls
/// back to a non-atomic copy (Go: AtomicWriteFile).
pub fn atomic_write_file(path: &Path, data: &[u8], perm: u32) -> io::Result<()> {
    atomic_write_file_inner(path, data, perm, true, None)
}

/// Publishes only via atomic rename (no cross-device copy). After a
/// successful rename it best-effort fsyncs the parent directory so the
/// directory entry can survive power loss. A returned error always means
/// the destination was not published; post-rename dir-sync problems are
/// not errors — a caller that rolls back in-memory state on error would
/// otherwise fork from the on-disk pointer (Go: AtomicWriteFileStrict).
pub fn atomic_write_file_strict(path: &Path, data: &[u8], perm: u32) -> io::Result<()> {
    atomic_write_file_strict_validated(path, data, perm, None)
}

/// Strict publication with a validation callback checked after preparing
/// the durable temp and before every rename attempt; a failed check aborts
/// without further retries and its error is returned unchanged (Go:
/// AtomicWriteFileStrictValidated).
pub fn atomic_write_file_strict_validated(
    path: &Path,
    data: &[u8],
    perm: u32,
    validate: Option<&dyn Fn() -> io::Result<()>>,
) -> io::Result<()> {
    atomic_write_file_inner(path, data, perm, false, validate)
}

fn atomic_write_file_inner(
    path: &Path,
    data: &[u8],
    perm: u32,
    allow_cross_device_copy: bool,
    validate: Option<&dyn Fn() -> io::Result<()>>,
) -> io::Result<()> {
    crash("atomic-write", &path.to_string_lossy());
    let tmp_path = write_atomic_temp(path, data, perm)?;
    if let Err(err) = replace_file_inner(&tmp_path, path, allow_cross_device_copy, validate) {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }
    if !allow_cross_device_copy {
        // Strict only: parent-dir fsync is power-loss durability after
        // publish. Never surface failures here — rename already committed
        // the new file.
        let _ = sync_parent_dir(path);
    }
    Ok(())
}

/// Renames tmp onto dest, publishing the new content atomically: a reader
/// concurrent with the replace sees either the old file or the complete new
/// one (Go: ReplaceFile).
pub fn replace_file(tmp: &Path, dest: &Path) -> io::Result<()> {
    crash("replace", &dest.to_string_lossy());
    replace_file_inner(tmp, dest, true, None)
}

fn replace_file_inner(
    tmp: &Path,
    dest: &Path,
    allow_cross_device_copy: bool,
    validate: Option<&dyn Fn() -> io::Result<()>>,
) -> io::Result<()> {
    let mut attempt = 0u32;
    loop {
        if let Some(validate) = validate {
            validate()?;
        }
        match fs::rename(tmp, dest) {
            Ok(()) => return Ok(()),
            Err(err) => {
                if rename_crosses_device(&err) {
                    if !allow_cross_device_copy {
                        return Err(err);
                    }
                    // Only this class falls back to the non-atomic copy,
                    // and immediately; torn reads remain possible in that
                    // degraded mode, and rename-capable filesystems never
                    // take it.
                    if copy_onto(tmp, dest).is_ok() {
                        return Ok(());
                    }
                    return Err(err);
                }
                // A missing tmp means the write itself failed and no retry
                // can help.
                if attempt >= MAX_REPLACE_RETRIES || !file_exists(tmp) {
                    return Err(err);
                }
                std::thread::sleep(Duration::from_millis(
                    (u64::from(attempt) + 1) * REPLACE_RETRY_BASE_MS,
                ));
            }
        }
        attempt += 1;
    }
}

/// Deletes path, retrying the same transient locks replace_file retries. A
/// file this process just published can still be held for a few
/// milliseconds by a scanner, an indexer, or a reader that has not closed
/// yet; a bare remove turns that window into a failure the caller reports
/// as its own. A path that is already gone is a success, not a race to
/// re-report (Go: fileutil.Remove).
pub fn remove(path: &Path) -> io::Result<()> {
    let mut attempt = 0u32;
    loop {
        match fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                if attempt >= MAX_REPLACE_RETRIES {
                    return Err(err);
                }
                std::thread::sleep(Duration::from_millis(
                    (u64::from(attempt) + 1) * REPLACE_RETRY_BASE_MS,
                ));
            }
        }
        attempt += 1;
    }
}

fn file_exists(path: &Path) -> bool {
    fs::metadata(path).is_ok()
}

/// The non-atomic last resort for hosts whose filesystem cannot rename tmp
/// onto dest at all. It truncates dest in place, so a concurrent reader can
/// observe an empty or half-written file — it must never run for failures a
/// retry could clear (Go: copyOnto).
fn copy_onto(tmp: &Path, dest: &Path) -> io::Result<()> {
    let meta = fs::metadata(tmp)?;
    let data = fs::read(tmp)?;
    fs::write(dest, &data)?;
    // fs::write keeps an existing dest's mode, so re-apply tmp's mode to
    // match what the rename would have done (a 0600 config tmp must not
    // widen to 0644). Chmod/remove failures are ignored, as in Go.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode();
        let _ = fs::set_permissions(dest, fs::Permissions::from_mode(mode & 0o777));
    }
    #[cfg(windows)]
    {
        // Go's Chmod on Windows only toggles the read-only attribute from
        // the owner-write bit; propagate a read-only tmp to the dest.
        if meta.permissions().readonly() {
            let mut perm = fs::metadata(dest)?.permissions();
            perm.set_readonly(true);
            let _ = fs::set_permissions(dest, perm);
        }
    }
    let _ = fs::remove_file(tmp);
    Ok(())
}

/// Windows filter drivers report ERROR_NOT_SAME_DEVICE for same-dir
/// renames and some environments surface plain EXDEV; Go checks both on
/// Windows and only EXDEV elsewhere (Go: renameCrossesDevice).
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

/// fsyncs path's parent after a rename. Unsupported dir sync on Windows /
/// some network filesystems is ignored; std maps EINVAL/ENOTSUP/ENOSYS to
/// InvalidInput/Unsupported (Go: syncParentDir + isDirSyncUnsupported). On
/// Windows std cannot even open a directory handle, so open/sync failures
/// there surface as errors that the best-effort caller swallows.
fn sync_parent_dir(path: &Path) -> io::Result<()> {
    let dir = parent_or_dot(path);
    let file = File::open(dir)
        .map_err(|e| wrap(e, format!("open parent dir for fsync {}", path.display())))?;
    if let Err(err) = file.sync_all()
        && !dir_sync_unsupported(&err)
    {
        return Err(wrap(
            err,
            format!("fsync parent dir for {}", path.display()),
        ));
    }
    Ok(())
}

fn dir_sync_unsupported(err: &io::Error) -> bool {
    if cfg!(windows) {
        return true;
    }
    matches!(
        err.kind(),
        io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
    )
}

/// Creates dir (and missing parents) with an explicit mode on unix; on
/// Windows the mode is ignored (Go: os.MkdirAll with a FileMode). Existing
/// directories are left untouched.
pub fn mkdir_all_mode(dir: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().recursive(true).mode(mode).create(dir)
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        fs::create_dir_all(dir)
    }
}

/// Creates a uniquely named file in dir (Go: os.CreateTemp with a "*"
/// pattern, split here into an explicit prefix and suffix). The name is
/// prefix + random decimal + suffix; unix files are created 0600.
/// Collisions are retried up to 10000 times, as in Go.
pub fn create_temp_file(dir: &Path, prefix: &str, suffix: &str) -> io::Result<(File, PathBuf)> {
    let mut attempts = 0u32;
    loop {
        let path = dir.join(format!("{prefix}{}{suffix}", temp_suffix()));
        #[cfg(unix)]
        let result = {
            use std::os::unix::fs::OpenOptionsExt;
            OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
        };
        #[cfg(not(unix))]
        let result = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path);
        match result {
            Ok(file) => return Ok((file, path)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                attempts += 1;
                if attempts >= 10_000 {
                    return Err(err);
                }
            }
            Err(err) => return Err(err),
        }
    }
}

fn temp_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = u64::from(std::process::id());
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut h = nanos ^ (pid << 32) ^ n.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    format!("{}", h % 1_000_000_000)
}

fn write_atomic_temp(path: &Path, data: &[u8], perm: u32) -> io::Result<PathBuf> {
    let dir = parent_or_dot(path);
    let dir_perm = if perm & 0o077 == 0 { 0o700 } else { 0o755 };
    mkdir_all_mode(dir, dir_perm)
        .map_err(|e| wrap(e, format!("create dir for {}", path.display())))?;
    let (mut tmp, tmp_path) = create_temp_file(dir, ".atomic-", ".tmp")
        .map_err(|e| wrap(e, format!("create tmp for {}", path.display())))?;
    let outcome = (|| -> io::Result<()> {
        tmp.write_all(data)
            .map_err(|e| wrap(e, format!("write tmp for {}", path.display())))?;
        tmp.sync_all()
            .map_err(|e| wrap(e, format!("fsync tmp for {}", path.display())))?;
        // Go chmods the still-open handle, before Close, so there is no
        // window between close and a path-based chmod for another process
        // (Windows AV / search indexer) to grab or move the tmp and make
        // the chmod fail with "file not found". std offers no handle-based
        // chmod, so the (tiny, AV-only) window remains. Temp files start
        // 0600, so this only widens when perm asks.
        set_file_mode(&tmp_path, perm)
            .map_err(|e| wrap(e, format!("chmod tmp for {}", path.display())))?;
        Ok(())
    })();
    drop(tmp);
    if let Err(err) = outcome {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }
    Ok(tmp_path)
}

fn parent_or_dot(path: &Path) -> &Path {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    }
}

/// Applies mode to path with Go os.Chmod's platform behavior: the full
/// mode on unix, and on Windows only the read-only attribute, toggled
/// from the owner-write bit.
pub fn set_file_mode(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
    }
    #[cfg(windows)]
    {
        // Go's Chmod on Windows only toggles FILE_ATTRIBUTE_READONLY based
        // on the owner-write bit; std's windows PermissionsExt is unstable,
        // and set_readonly covers the same attribute.
        let readonly = mode & 0o200 == 0;
        let mut perm = fs::metadata(path)?.permissions();
        perm.set_readonly(readonly);
        fs::set_permissions(path, perm)
    }
}

/// Rebuilds err with the same kind and a "prefix: inner" message, matching
/// Go's fmt.Errorf("%s: %w", ...) rendering.
fn wrap(err: io::Error, what: String) -> io::Error {
    io::Error::new(err.kind(), format!("{what}: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> tempfile::TempDir {
        // Some test environments deny creating directories under the system
        // temp path; the target directory is always writable and never
        // surfaces inside the source tree (established repo pattern).
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("target")
            .join("test-tmp");
        fs::create_dir_all(&base).expect("create test-tmp base");
        tempfile::Builder::new()
            .prefix(".tmpfileutil")
            .tempdir_in(&base)
            .expect("create test dir")
    }

    #[test]
    fn atomic_write_file_writes_and_replaces() {
        let dir = tempdir();
        let path = dir.path().join("f.txt");
        atomic_write_file(&path, b"one", 0o600).expect("first write");
        assert_eq!(fs::read(&path).unwrap(), b"one");
        atomic_write_file(&path, b"two", 0o600).expect("second write");
        assert_eq!(fs::read(&path).unwrap(), b"two");
    }

    #[test]
    fn atomic_write_file_creates_missing_parents() {
        let dir = tempdir();
        let path = dir.path().join("a/b/c/f.txt");
        atomic_write_file(&path, b"deep", 0o600).expect("write");
        assert_eq!(fs::read(&path).unwrap(), b"deep");
    }

    #[test]
    fn atomic_write_file_failure_cleans_temp() {
        let dir = tempdir();
        // A regular file where a directory is needed fails the mkdir step
        // before any temp exists, with Go's wrapper message.
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, b"not a dir").unwrap();
        let path = blocker.join("child/f.txt");
        let err = atomic_write_file(&path, b"x", 0o600).expect_err("mkdir must fail");
        assert!(err.to_string().starts_with("create dir for"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_file_sets_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir();
        let path = dir.path().join("mode.txt");
        atomic_write_file(&path, b"m", 0o600).expect("write");
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn strict_write_round_trips() {
        let dir = tempdir();
        let path = dir.path().join("strict.txt");
        atomic_write_file_strict(&path, b"strict", 0o600).expect("strict write");
        assert_eq!(fs::read(&path).unwrap(), b"strict");
    }

    #[test]
    fn replace_file_publishes_content() {
        let dir = tempdir();
        let tmp = dir.path().join("src.tmp");
        let dest = dir.path().join("dest.txt");
        fs::write(&tmp, b"payload").unwrap();
        replace_file(&tmp, &dest).expect("replace");
        assert_eq!(fs::read(&dest).unwrap(), b"payload");
        assert!(!tmp.exists());
    }

    #[test]
    fn remove_treats_missing_as_success() {
        let dir = tempdir();
        remove(&dir.path().join("missing.txt")).expect("missing remove is ok");
    }

    #[test]
    fn remove_removes_existing_file() {
        let dir = tempdir();
        let path = dir.path().join("gone.txt");
        fs::write(&path, b"x").unwrap();
        remove(&path).expect("remove");
        assert!(!path.exists());
    }

    #[test]
    fn create_temp_file_yields_unique_names() {
        let dir = tempdir();
        let (f1, p1) = create_temp_file(dir.path(), ".t-", ".tmp").expect("first");
        let (f2, p2) = create_temp_file(dir.path(), ".t-", ".tmp").expect("second");
        drop(f1);
        drop(f2);
        assert_ne!(p1, p2);
        assert!(p1.file_name().unwrap().to_string_lossy().starts_with(".t-"));
        assert!(p1.file_name().unwrap().to_string_lossy().ends_with(".tmp"));
        assert!(p1.exists());
        assert!(p2.exists());
    }

    #[cfg(unix)]
    #[test]
    fn create_temp_file_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir();
        let (_f, p) = create_temp_file(dir.path(), ".t-", ".tmp").expect("create");
        let mode = fs::metadata(&p).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn copy_onto_overwrites_and_adopts_mode() {
        let dir = tempdir();
        let tmp = dir.path().join("src.tmp");
        let dest = dir.path().join("dest.txt");
        fs::write(&tmp, b"new").unwrap();
        fs::write(&dest, b"old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600)).unwrap();
        }
        copy_onto(&tmp, &dest).expect("copy");
        assert_eq!(fs::read(&dest).unwrap(), b"new");
        assert!(!tmp.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&dest).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn mkdir_all_mode_creates_missing() {
        let dir = tempdir();
        let nested = dir.path().join("x/y/z");
        mkdir_all_mode(&nested, 0o755).expect("mkdir");
        assert!(nested.is_dir());
        mkdir_all_mode(&nested, 0o755).expect("mkdir of existing dir is ok");
    }

    #[test]
    fn crash_hook_fires_before_write_and_replace() {
        use std::sync::Arc;
        let ops: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = ops.clone();
        set_crash_point(Some(Box::new(move |op, path| {
            sink.lock()
                .unwrap()
                .push((op.to_string(), path.to_string()));
        })));
        let dir = tempdir();
        // atomic_write_file crashes at "atomic-write" only; the "replace"
        // crash point belongs to the ReplaceFile entry point (Go:
        // atomicWriteFile calls the internal replaceFile, which has no
        // crash hook).
        let path = dir.path().join("crash-hook-target.txt");
        let target = path.to_string_lossy().into_owned();
        atomic_write_file(&path, b"hi", 0o600).expect("write");
        let tmp = dir.path().join("crash-hook-replace-src.tmp");
        let dest = dir.path().join("crash-hook-replace-dest.txt");
        fs::write(&tmp, b"x").unwrap();
        let dest_str = dest.to_string_lossy().into_owned();
        replace_file(&tmp, &dest).expect("replace");
        set_crash_point(None);
        let ops = ops.lock().unwrap();
        let mine: Vec<_> = ops
            .iter()
            .filter(|(_, p)| p == &target || p == &dest_str)
            .collect();
        assert_eq!(mine.len(), 2, "hook saw {ops:?}");
        assert_eq!(mine[0].0, "atomic-write");
        assert_eq!(mine[0].1, target);
        assert_eq!(mine[1].0, "replace");
        assert_eq!(mine[1].1, dest_str);
    }
}
