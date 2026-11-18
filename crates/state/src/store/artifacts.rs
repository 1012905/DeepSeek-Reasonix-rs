//! Removal of session artifacts.
//!
//! Parity target: `reference/.../internal/state/store/session.go`
//! (`RemoveSessionArtifacts`, docs/contracts/session-format.md 1,
//! 20.1). The event log is authoritative, so a delete surface that leaves
//! it behind both leaks the "deleted" conversation and lets a later load
//! resurrect it; every delete path (desktop trash, /clear, serve, ACP) must
//! go through this module.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::naming::{session_sidecar_dirs, session_sidecar_files};

/// Error joining every removal refusal, mirroring Go `errors.Join`:
/// the Display text is the failures' texts joined by newlines.
#[derive(Debug, thiserror::Error)]
#[error("{}", failures.iter().map(|err| err.to_string()).collect::<Vec<_>>().join("\n"))]
pub struct RemoveArtifactsError {
    /// Every non-`NotFound` refusal, in removal order.
    pub failures: Vec<io::Error>,
}

/// Removes the transcript, the per-frontend `extra` companions, the
/// thirteen regular-file sidecars, then the outputs/blobs directories
/// (`RemoveSessionArtifacts`, session-format.md 20.1). Missing files
/// are skipped; every refusal is joined into one error. Checkpoints, jobs
/// and the inbox keep their own lifecycles and are intentionally not
/// touched.
pub fn remove_session_artifacts(
    session_path: &Path,
    extra: &[impl AsRef<Path>],
) -> Result<(), RemoveArtifactsError> {
    let mut failures: Vec<io::Error> = Vec::new();

    let mut paths: Vec<PathBuf> = Vec::with_capacity(1 + extra.len());
    paths.push(session_path.to_path_buf());
    paths.extend(extra.iter().map(|path| path.as_ref().to_path_buf()));
    paths.extend(session_sidecar_files(session_path));

    for path in &paths {
        if path.as_os_str().is_empty() {
            continue; // Go skips the empty path.
        }
        if let Err(err) = remove_path(path)
            && err.kind() != io::ErrorKind::NotFound
        {
            failures.push(err);
        }
    }

    for dir in session_sidecar_dirs(session_path) {
        if let Err(err) = fs::remove_dir_all(&dir)
            && err.kind() != io::ErrorKind::NotFound
        {
            failures.push(err);
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(RemoveArtifactsError { failures })
    }
}

/// `os.Remove` parity: unlink, then rmdir (Go tries both because the
/// syscall layer forces the choice), then on Windows clear the read-only
/// attribute and retry unlink. Reports the unlink error when everything
/// fails.
fn remove_path(path: &Path) -> io::Result<()> {
    let unlink_err = match fs::remove_file(path) {
        Ok(()) => return Ok(()),
        Err(err) => err,
    };
    if fs::remove_dir(path).is_ok() {
        return Ok(());
    }
    if clear_read_only(path) && fs::remove_file(path).is_ok() {
        return Ok(());
    }
    Err(unlink_err)
}

/// Windows `os.Remove` parity: a read-only file is unlinked after its
/// attribute is cleared. No-op elsewhere (mode bits never gate unlink on
/// Unix).
#[cfg(windows)]
#[allow(clippy::permissions_set_readonly_false)] // clearing FILE_ATTRIBUTE_READONLY is the whole point; Unix is cfg'd out
fn clear_read_only(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    let mut permissions = metadata.permissions();
    if !permissions.readonly() {
        return false;
    }
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions).is_ok()
}

#[cfg(not(windows))]
fn clear_read_only(_path: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Scratch base for artifact tests.
    ///
    /// The workspace target directory is used instead of `env::temp_dir()`
    /// because some test environments deny creating directories under the
    /// system temp path, while the target directory is always writable and
    /// never surfaces inside the source tree.
    fn temp_base() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("target")
            .join("test-tmp")
    }

    const SESSION: &str = "20260101-000000.000000000-m.jsonl";
    const STEM: &str = "20260101-000000.000000000-m";

    /// Minimal temp-dir guard (no `tempfile` dependency yet).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let unique = TEMP_DIR_COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = temp_base().join(format!(
                "reasonix-state-tests-{}-{}-{}",
                tag,
                std::process::id(),
                unique,
            ));
            fs::create_dir_all(&dir).expect("create temp dir");
            TempDir(dir)
        }

        fn join(&self, relative: &str) -> PathBuf {
            self.0.join(relative)
        }

        fn touch(&self, relative: &str) -> PathBuf {
            let path = self.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent dir");
            }
            fs::write(&path, b"x").expect("write file");
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn create_full_artifact_set(dir: &TempDir) -> PathBuf {
        // Transcript + the 13 regular-file sidecars; meta hangs off the full
        // transcript path (session-format.md 1).
        dir.touch(SESSION);
        dir.touch(&format!("{SESSION}.meta"));
        for sidecar in [
            "goal-state.json",
            "adjudication.jsonl",
            "execution.jsonl",
            "events.jsonl",
            "wire.jsonl",
            "wire.meta.json",
            "events.jsonl.damaged",
            "event-index.json",
            "display-index.json",
            "conflicts.jsonl",
            "recovery.json",
            "context.json",
        ] {
            dir.touch(&format!("{STEM}.{sidecar}"));
        }
        // Sidecar directories with content (RemoveAll territory).
        dir.touch(&format!("{STEM}.outputs/tool-1.out"));
        dir.touch(&format!("{STEM}.blobs/ab/cd/abcd"));
        // Tolerated survivors: own lifecycles, never removed here.
        dir.touch(&format!("{STEM}.guardian.jsonl"));
        dir.touch(&format!("{SESSION}.lock"));
        dir.touch(&format!("{SESSION}.lease.lock"));
        dir.touch(&format!("{SESSION}.lease.json"));
        dir.touch(&format!("{STEM}.ckpt/turn-1.json"));
        dir.touch(&format!("{STEM}.jobs/keep-1.json"));
        dir.touch(&format!("{STEM}.inbox/manifest.json"));
        dir.touch(&format!("{STEM}.superseded.jsonl.bak"));
        dir.touch(&format!("{STEM}.cleanup-pending.json"));
        dir.join(SESSION)
    }

    #[test]
    fn removes_transcript_sidecars_and_output_dirs_but_not_tolerated_survivors() {
        let dir = TempDir::new("remove");
        let session = create_full_artifact_set(&dir);
        let extra = dir.touch("acp-companion.json");

        remove_session_artifacts(&session, &[&extra]).expect("remove succeeds");

        assert!(!session.exists());
        assert!(!dir.join(&format!("{SESSION}.meta")).exists());
        for sidecar in [
            "goal-state.json",
            "adjudication.jsonl",
            "execution.jsonl",
            "events.jsonl",
            "wire.jsonl",
            "wire.meta.json",
            "events.jsonl.damaged",
            "event-index.json",
            "display-index.json",
            "conflicts.jsonl",
            "recovery.json",
            "context.json",
        ] {
            assert!(
                !dir.join(&format!("{STEM}.{sidecar}")).exists(),
                "{sidecar}"
            );
        }
        assert!(!dir.join(&format!("{STEM}.outputs")).exists());
        assert!(!dir.join(&format!("{STEM}.blobs")).exists());
        assert!(!extra.exists());
        // Survivors keep their own lifecycles.
        for survivor in [
            "guardian.jsonl",
            "superseded.jsonl.bak",
            "cleanup-pending.json",
        ] {
            assert!(
                dir.join(&format!("{STEM}.{survivor}")).exists(),
                "{survivor}"
            );
        }
        for lock in ["lock", "lease.lock", "lease.json"] {
            assert!(dir.join(&format!("{SESSION}.{lock}")).exists(), "{lock}");
        }
        for owned in ["ckpt", "jobs", "inbox"] {
            assert!(dir.join(&format!("{STEM}.{owned}")).exists(), "{owned}");
        }
    }

    #[test]
    fn missing_artifacts_are_not_errors() {
        let dir = TempDir::new("missing");
        let session = dir.join(SESSION);
        let extra = dir.join("acp-companion.json");
        remove_session_artifacts(&session, &[&extra]).expect("missing files are skipped");
    }

    #[test]
    fn failures_are_joined_not_short_circuited() {
        let dir = TempDir::new("refusals");
        // A non-empty directory where the transcript should be: os.Remove
        // refuses it, but the other sidecars must still go.
        fs::create_dir_all(dir.join(&format!("{SESSION}/inner"))).expect("create blocking dir");
        let session = dir.join(SESSION);
        dir.touch(&format!("{STEM}.events.jsonl"));
        dir.touch(&format!("{STEM}.context.json"));

        let err = remove_session_artifacts(&session, &[] as &[&Path])
            .expect_err("transcript slot is a non-empty dir");

        assert_eq!(err.failures.len(), 1);
        assert!(!err.to_string().is_empty());
        assert!(!dir.join(&format!("{STEM}.events.jsonl")).exists());
        assert!(!dir.join(&format!("{STEM}.context.json")).exists());
    }

    #[test]
    fn blank_session_paths_still_remove_extras() {
        let dir = TempDir::new("blank");
        let extra = dir.touch("acp-companion.json");
        // The blank path itself is attempted like Go os.Remove; its outcome
        // is platform-specific, so only the extras are asserted.
        let _ = remove_session_artifacts(Path::new("   "), &[&extra]);
        assert!(!extra.exists());
    }

    #[test]
    #[cfg(windows)]
    fn windows_readonly_files_are_cleared_and_removed() {
        let dir = TempDir::new("readonly");
        let session = create_full_artifact_set(&dir);
        let mut permissions = fs::metadata(&session)
            .expect("stat transcript")
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&session, permissions).expect("make transcript read-only");

        remove_session_artifacts(&session, &[] as &[&Path])
            .expect("os.Remove clears the read-only attribute");
        assert!(!session.exists());
    }
}
