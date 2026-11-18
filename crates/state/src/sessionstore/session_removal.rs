//! The session removal guard (Go: `session_removal.go`;
//! docs/contracts/session-format.md 15.6).
//!
//! A guard holds both the save lock and the lease lock of a session
//! that is about to be removed (trash, purge, delete), deleting the
//! lock files atomically with the release so a later acquirer can
//! never lock a surviving inode. Taking the guard closes the
//! probe-then-delete window: no writer can appear between the
//! unusedness probe and the delete.

use std::io;
use std::path::Path;

use super::session_lease::{
    SessionLease, SessionLeaseError, lease_owners_get, load_session_lease_info,
    remove_active_owner_if, remove_owner_if, remove_session_lease_info, republish_active_owner,
    session_lease_held_locally, try_lock_session_lease_file,
};
use super::session_lock::{SessionFileLockError, SessionLockFile, try_take_session_lock_file};
use crate::sessionstore::session::canonical_session_save_path;
use crate::store::naming;

/// A guard holding the save and lease locks of a session about to be
/// removed (Go: `SessionRemovalGuard`).
///
/// Acquire through `try_acquire_session_removal_guard` or by
/// converting a held lease (`SessionLease::try_convert_to_removal_guard`).
/// Abandoning a guard without releasing drops both OS locks, leaving
/// exactly the orphaned-entry scenario the lease reclaim handles.
#[derive(Debug)]
pub struct SessionRemovalGuard {
    path: String,
    save_lock: Option<SessionLockFile>,
    lease_lock: Option<SessionLockFile>,
    restore_owner: u64,
}

/// Take the removal guard (Go: `TryAcquireSessionRemovalGuard`):
/// the lease lock first, then the save lock, so a failed attempt
/// leaves nothing behind.
pub fn try_acquire_session_removal_guard(
    path: &str,
) -> Result<SessionRemovalGuard, SessionLeaseError> {
    let path = canonical_session_save_path(path);
    if session_lease_held_locally(&path) {
        let info = load_session_lease_info(&path).ok();
        return Err(SessionLeaseError::Held { path, info });
    }
    let lease_lock = match try_lock_session_lease_file(&path) {
        Ok(lock) => lock,
        Err(err) if err.is_held() => {
            let info = load_session_lease_info(&path).ok();
            return Err(SessionLeaseError::Held { path, info });
        }
        Err(err) => return Err(err),
    };
    let save_lock = match try_take_session_lock_file(&naming::session_lock_file(Path::new(&path))) {
        Ok(lock) => lock,
        Err(err) => {
            lease_lock.unlock();
            return Err(match err {
                // A save is in flight; deleting mid-write would race
                // it.
                SessionFileLockError::Held => SessionLeaseError::Held { path, info: None },
                SessionFileLockError::Io(err) => SessionLeaseError::Io(err),
            });
        }
    };
    Ok(SessionRemovalGuard {
        path,
        save_lock: Some(save_lock),
        lease_lock: Some(lease_lock),
        restore_owner: 0,
    })
}

impl SessionRemovalGuard {
    /// Canonical path the guard was taken for.
    pub(crate) fn path(&self) -> &str {
        &self.path
    }

    pub(crate) fn from_parts(
        path: String,
        save_lock: SessionLockFile,
        lease_lock: Option<SessionLockFile>,
        restore_owner: u64,
    ) -> SessionRemovalGuard {
        SessionRemovalGuard {
            path,
            save_lock: Some(save_lock),
            lease_lock,
            restore_owner,
        }
    }

    /// Restore the converted lease (Go: `RestoreSessionLease`):
    /// republish the proven ownership and hand the locks back as a
    /// live lease, with no writer gap. The lease info sidecar was
    /// never touched by the conversion.
    pub fn restore_session_lease(&mut self) -> Result<SessionLease, SessionLeaseError> {
        if self.restore_owner == 0 || self.lease_lock.is_none() || self.save_lock.is_none() {
            return Err(SessionLeaseError::Io(io::Error::other(
                "removal guard cannot restore a session lease",
            )));
        }
        let owner_id = self.restore_owner;
        if lease_owners_get(&self.path) != Some(owner_id) {
            return Err(SessionLeaseError::Held {
                path: self.path.clone(),
                info: None,
            });
        }
        republish_active_owner(&self.path, owner_id);
        let lease_lock = self.lease_lock.take().expect("checked above");
        self.restore_owner = 0;
        self.save_lock.take().expect("checked above").unlock();
        Ok(SessionLease::from_parts(
            self.path.clone(),
            owner_id,
            lease_lock,
        ))
    }

    /// Abort without deleting anything (Go: `Release`): unlock
    /// both locks but keep the files. A converted lease's identity
    /// sidecar is removed, so an abort does not leave it naming a
    /// writer. Idempotent and safe after
    /// `remove_sidecars_and_release`.
    pub fn release(mut self) {
        if self.restore_owner != 0 {
            remove_active_owner_if(&self.path, self.restore_owner);
            remove_owner_if(&self.path, self.restore_owner);
            let _ = remove_session_lease_info(&self.path);
            self.restore_owner = 0;
        }
        if let Some(lock) = self.save_lock.take() {
            lock.unlock();
        }
        if let Some(lock) = self.lease_lock.take() {
            lock.unlock();
        }
    }

    /// Remove the identity sidecar and both lock files, then release
    /// everything (Go: `RemoveSidecarsAndRelease`). The identity
    /// sidecar goes first, while the locks are still held.
    pub fn remove_sidecars_and_release(mut self) -> io::Result<()> {
        let mut errors: Vec<io::Error> = Vec::new();
        if self.restore_owner != 0 {
            remove_active_owner_if(&self.path, self.restore_owner);
            remove_owner_if(&self.path, self.restore_owner);
            self.restore_owner = 0;
        }
        if let Err(err) = remove_session_lease_info(&self.path) {
            errors.push(err);
        }
        if let Some(lock) = self.save_lock.take()
            && let Err(err) = lock.remove_and_unlock()
        {
            errors.push(err);
        }
        if let Some(lock) = self.lease_lock.take()
            && let Err(err) = lock.remove_and_unlock()
        {
            errors.push(err);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(io::Error::other(
                errors
                    .iter()
                    .map(|err| err.to_string())
                    .collect::<Vec<_>>()
                    .join("\n"),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionstore::eventlog::test_support::tempdir;
    use crate::sessionstore::session::Session;
    use crate::sessionstore::session_lease::{
        session_lease_held_by_current_runtime, try_acquire_session_lease,
    };

    fn saved_session() -> (tempfile::TempDir, String) {
        let dir = tempdir();
        let path = dir.path().join("Session-Removal.jsonl");
        let text = path.to_string_lossy().into_owned();
        Session::new("sys").save(&text).expect("save");
        (dir, text)
    }

    #[test]
    fn removal_guard_blocks_while_lease_held() {
        let (_dir, path_text) = saved_session();
        let lease = try_acquire_session_lease(&path_text).expect("lease");
        let guard = try_acquire_session_removal_guard(&path_text);
        assert!(guard.expect_err("lease held").is_held());
        // The lease lock file is undisturbed.
        assert!(naming::session_lease_lock(Path::new(&path_text)).exists());
        lease.release();
        let guard = try_acquire_session_removal_guard(&path_text).expect("guard");
        // While the guard is held, a lease cannot be acquired.
        assert!(try_acquire_session_lease(&path_text).is_err());
        guard.remove_sidecars_and_release().expect("remove");
        assert!(!naming::session_lock_file(Path::new(&path_text)).exists());
        assert!(!naming::session_lease_lock(Path::new(&path_text)).exists());
        assert!(!naming::session_lease_info(Path::new(&path_text)).exists());
        try_acquire_session_lease(&path_text)
            .expect("fresh acquire after removal")
            .release();
    }

    #[test]
    fn removal_guard_release_keeps_sidecars() {
        let (_dir, path_text) = saved_session();
        let guard = try_acquire_session_removal_guard(&path_text).expect("guard");
        guard.release();
        // An abort unlocks but keeps the lock files.
        assert!(naming::session_lease_lock(Path::new(&path_text)).exists());
        try_acquire_session_lease(&path_text)
            .expect("acquire after abort")
            .release();
    }

    #[test]
    fn lease_converts_to_removal_guard_without_ownership_gap() {
        let (_dir, path_text) = saved_session();
        let lease = try_acquire_session_lease(&path_text).expect("lease");
        let guard = lease.try_convert_to_removal_guard().expect("convert");
        // A stale release of the converted lease is a no-op.
        lease.release();
        // The converted guard still blocks competing acquisition.
        assert!(try_acquire_session_lease(&path_text).is_err());
        guard.remove_sidecars_and_release().expect("remove");
        try_acquire_session_lease(&path_text)
            .expect("fresh acquire")
            .release();
    }

    #[test]
    fn converted_removal_guard_restores_session_lease() {
        let (_dir, path_text) = saved_session();
        let lease = try_acquire_session_lease(&path_text).expect("lease");
        let mut guard = lease.try_convert_to_removal_guard().expect("convert");
        let lease = guard.restore_session_lease().expect("restore");
        assert!(session_lease_held_by_current_runtime(&path_text));
        assert!(try_acquire_session_lease(&path_text).is_err());
        lease.release();
    }

    #[test]
    fn lease_conversion_failure_keeps_lease_active() {
        let (_dir, path_text) = saved_session();
        let lease = try_acquire_session_lease(&path_text).expect("lease");
        // An external save lock makes the conversion fail: the save
        // lock is taken first, so the failure leaves the lease fully
        // active.
        let external =
            try_take_session_lock_file(&naming::session_lock_file(Path::new(&path_text)))
                .expect("external save lock");
        let guard = lease.try_convert_to_removal_guard();
        assert!(guard.expect_err("save lock held").is_held());
        assert!(session_lease_held_by_current_runtime(&path_text));
        assert!(try_acquire_session_lease(&path_text).is_err());
        external.unlock();
        lease.release();
    }
}
