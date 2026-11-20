//! The cross-process session lease (Go: `session_lease.go`;
//! docs/contracts/session-format.md 9.1).
//!
//! A lease marks one runtime as the sole writer of a session
//! transcript. The holder takes an exclusive OS lock on the
//! `<transcript>.lease.lock` sidecar and records its identity in
//! the `<transcript>.lease.json` sidecar, so a competing runtime
//! either fails the lock or reads the holder identity. Two
//! process-local registries separate reservations from proven
//! ownership: the owners registry maps a canonical path to the
//! reserving generation (a reservation or a live lease), while the
//! active registry records only the generations that took the
//! cross-process lock and wrote the lease info. Ownership-sensitive
//! repair must never treat a pending or failed acquisition as
//! ownership, which is why the two are kept apart.
//!
//! Divergence from Go: the unleased-write observation records the
//! deduplicated canonical path but does not yet emit a log line; the
//! Go source logs `session: save without a held lease
//! (write-authority probe, store P2)` through `slog` and the
//! Rust workspace has no logger wired yet. The deduplicated record is
//! the tested contract. `unleasedWriteOrigin` is likewise
//! deferred: the Rust standard library exposes no stable stack-frame
//! iteration API.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};

use reasonix_contract::gojson::escape_html;
use reasonix_core::fileencoding::read_file_utf8;
use reasonix_core::fileutil::{create_temp_file, mkdir_all_mode, replace_file};

use super::recovery_gc::go_dir;
use super::session_lock::{SessionFileLockError, SessionLockFile, try_take_session_lock_file};
use super::session_removal::SessionRemovalGuard;
use crate::sessionstore::eventlog::record::{CreatedAt, deserialize_created_at, now_utc};
use crate::sessionstore::eventlog::writer_id::session_writer_id;
use crate::sessionstore::session::{
    SessionSaveMode, canonical_session_save_path, session_artifact_exists,
};
use crate::store::naming;

/// Errors from acquiring, reclaiming, or probing a session lease (Go:
/// the `ErrSessionLeaseHeld` and `ErrSessionLeaseUnprovable`
/// sentinels plus their wrapped failures).
#[derive(Debug)]
pub enum SessionLeaseError {
    /// The lease is held by another runtime (Go:
    /// `&SessionLeaseError{Path, Info}`).
    Held {
        /// The session path the error names.
        path: String,
        /// The holder identity, when the lease info sidecar was
        /// readable.
        info: Option<SessionLeaseInfo>,
    },
    /// The bare held sentinel (Go: `ErrSessionLeaseHeld` without
    /// a wrapper): the session is held, no holder identity attached.
    HeldBare,
    /// Lease ownership could not be proven (Go:
    /// `ErrSessionLeaseUnprovable`): neither held nor free;
    /// fail-closed callers refuse, state surfaces report unknown.
    Unprovable {
        /// The failure that made ownership unprovable.
        source: Option<Box<SessionLeaseError>>,
    },
    /// An empty session path was passed where one is required.
    EmptyPath,
    /// Any other filesystem failure.
    Io(io::Error),
}

impl std::fmt::Display for SessionLeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionLeaseError::Held { path, info } => {
                write!(f, "session lease held by another runtime: {path}")?;
                if let Some(info) = info
                    && !info.writer_id.is_empty()
                {
                    write!(f, " is held by {}", info.writer_id)?;
                }
                Ok(())
            }
            SessionLeaseError::HeldBare => {
                write!(f, "session lease held by another runtime")
            }
            SessionLeaseError::Unprovable { source } => {
                write!(f, "session lease ownership could not be proven")?;
                if let Some(source) = source {
                    write!(f, ": {source}")?;
                }
                Ok(())
            }
            SessionLeaseError::EmptyPath => write!(f, "empty session path"),
            SessionLeaseError::Io(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for SessionLeaseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SessionLeaseError::Unprovable {
                source: Some(source),
            } => Some(source.as_ref()),
            SessionLeaseError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for SessionLeaseError {
    fn from(err: io::Error) -> SessionLeaseError {
        SessionLeaseError::Io(err)
    }
}

impl SessionLeaseError {
    /// Whether this error reports the lease as held by another
    /// runtime (Go: `errors.Is(err, ErrSessionLeaseHeld)`).
    pub fn is_held(&self) -> bool {
        matches!(
            self,
            SessionLeaseError::Held { .. } | SessionLeaseError::HeldBare
        )
    }

    /// Whether lease ownership could not be proven (Go:
    /// `errors.Is(err, ErrSessionLeaseUnprovable)`).
    pub fn is_unprovable(&self) -> bool {
        matches!(self, SessionLeaseError::Unprovable { .. })
    }
}

/// The registry of lease reservations and live leases: canonical
/// path to the reserving owner id (Go: `sessionLeaseOwners`).
/// Deletions compare the owner id, so an old generation can never
/// evict a newer one.
fn lease_owners() -> &'static Mutex<HashMap<String, u64>> {
    static OWNERS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    OWNERS.get_or_init(Mutex::default)
}

/// The registry of proven owners: canonical path to the owner id of
/// the generation that took the cross-process lock and wrote the
/// lease info (Go: `sessionLeaseActiveOwners`).
fn lease_active_owners() -> &'static Mutex<HashMap<String, u64>> {
    static ACTIVE_OWNERS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    ACTIVE_OWNERS.get_or_init(Mutex::default)
}

/// The generation counter for lease owners (Go:
/// `sessionLeaseSeq`).
static LEASE_SEQ: AtomicU64 = AtomicU64::new(0);

/// The deduplicated set of canonical paths observed written without
/// a held lease (Go: `unleasedWriteObserved`): one record per
/// path per process.
fn unleased_write_observed() -> &'static Mutex<HashSet<String>> {
    static OBSERVED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    OBSERVED.get_or_init(Mutex::default)
}

/// Lock a mutex, treating a poisoned lock as unlocked content (the
/// lock-protected data is never left structurally invalid).
fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn next_owner_id() -> u64 {
    LEASE_SEQ.fetch_add(1, Ordering::SeqCst) + 1
}

/// Read the reserving owner id for a canonical path, when present.
pub(crate) fn lease_owners_get(path: &str) -> Option<u64> {
    lock_unpoisoned(lease_owners()).get(path).copied()
}

/// Remove the owners-registry entry only when it still names
/// `owner_id` (Go: `CompareAndDelete`): a stale release
/// must not evict a newer owner.
pub(crate) fn remove_owner_if(path: &str, owner_id: u64) {
    let mut owners = lock_unpoisoned(lease_owners());
    if owners.get(path) == Some(&owner_id) {
        owners.remove(path);
    }
}

/// Remove the active-registry entry only when it still names
/// `owner_id` (Go: `CompareAndDelete`).
pub(crate) fn remove_active_owner_if(path: &str, owner_id: u64) {
    let mut active = lock_unpoisoned(lease_active_owners());
    if active.get(path) == Some(&owner_id) {
        active.remove(path);
    }
}

/// Remove the active-registry entry unconditionally (Go:
/// `sessionLeaseActiveOwners.Delete`): taking the OS lock
/// proves any leftover entry is stale.
fn remove_active_owner(path: &str) {
    lock_unpoisoned(lease_active_owners()).remove(path);
}

/// Republish proven ownership without a writer gap (Go: delete +
/// store on the active registry).
pub(crate) fn republish_active_owner(path: &str, owner_id: u64) {
    let mut active = lock_unpoisoned(lease_active_owners());
    active.remove(path);
    active.insert(path.to_string(), owner_id);
}

/// The holder identity recorded in the lease info sidecar (Go:
/// `SessionLeaseInfo`).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SessionLeaseInfo {
    /// The session path the lease covers.
    #[serde(default, rename = "session_path")]
    pub session_path: String,
    /// The writer id of the holder (Go: `SessionWriterID()`).
    #[serde(default, rename = "writer_id")]
    pub writer_id: String,
    /// The operating-system process id of the holder.
    #[serde(default, rename = "pid")]
    pub pid: i64,
    /// The hostname of the holder, omitted when empty (Go:
    /// `omitempty`).
    #[serde(default, rename = "hostname", skip_serializing_if = "String::is_empty")]
    pub hostname: String,
    /// When the lease was acquired (Go: `time.Time`).
    #[serde(
        default,
        rename = "acquired_at",
        deserialize_with = "deserialize_created_at"
    )]
    pub acquired_at: CreatedAt,
}

/// Build the holder identity for `path` (Go:
/// `newSessionLeaseInfo`).
fn new_session_lease_info(path: &str) -> SessionLeaseInfo {
    SessionLeaseInfo {
        session_path: path.to_string(),
        writer_id: session_writer_id().to_string(),
        pid: i64::from(std::process::id()),
        hostname: hostname::get()
            .ok()
            .and_then(|host| host.into_string().ok())
            .unwrap_or_default(),
        acquired_at: now_utc(),
    }
}

/// The path of the lease info sidecar for a session path (Go:
/// `sessionLeaseInfoPath`).
fn lease_info_path(path: &str) -> PathBuf {
    naming::session_lease_info(Path::new(&canonical_session_save_path(path)))
}

/// A held session lease (Go: `SessionLease`). Dropping the
/// value does not release the lease: call `release` (or convert
/// it into a removal guard); an abandoned lease keeps the OS lock
/// until process exit, which is exactly the stale-owner scenario
/// `try_reclaim_current_process_session_lease` handles.
pub struct SessionLease {
    shared: Arc<LeaseShared>,
}

/// File-local shared lease state: both the lease handle and every
/// write authority it issues observe the same release lifecycle (Go:
/// the authority keeps a `*SessionLease` pointer).
struct LeaseShared {
    path: String,
    owner_id: u64,
    inner: Mutex<LeaseInner>,
    release_wait: Condvar,
}

struct LeaseInner {
    lease_lock: Option<SessionLockFile>,
    released: bool,
    /// The number of in-flight authority-guarded saves (Go:
    /// `activeSaves`): release waits for them to finish so a
    /// successor lease cannot be issued while a passed-validity save
    /// is still writing.
    active_saves: i32,
    /// The generation of the newest issued write authority (Go:
    /// `writeGeneration`): re-issuing invalidates prior tokens.
    write_generation: u64,
    #[cfg(test)]
    before_release_lock: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(test)]
    before_release_wait: Option<Box<dyn FnOnce() + Send>>,
}

impl LeaseInner {
    fn new(lease_lock: Option<SessionLockFile>) -> LeaseInner {
        LeaseInner {
            lease_lock,
            released: false,
            active_saves: 0,
            write_generation: 0,
            #[cfg(test)]
            before_release_lock: None,
            #[cfg(test)]
            before_release_wait: None,
        }
    }
}

impl std::fmt::Debug for SessionLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionLease")
            .field("path", &self.shared.path)
            .field("owner_id", &self.shared.owner_id)
            .finish_non_exhaustive()
    }
}

impl SessionLease {
    pub(crate) fn from_parts(
        path: String,
        owner_id: u64,
        lease_lock: SessionLockFile,
    ) -> SessionLease {
        SessionLease {
            shared: Arc::new(LeaseShared {
                path,
                owner_id,
                inner: Mutex::new(LeaseInner::new(Some(lease_lock))),
                release_wait: Condvar::new(),
            }),
        }
    }

    /// The canonical session path this lease covers.
    pub fn path(&self) -> &str {
        &self.shared.path
    }

    /// Convert the lease into a removal guard (Go:
    /// `TryConvertToRemovalGuard`): the save lock is taken
    /// first, so a failed conversion leaves the lease fully active.
    /// On success the OS lease lock and the owners reservation stay
    /// held, blocking competing acquisition and making rollback
    /// immune to an in-process lease race.
    pub fn try_convert_to_removal_guard(&self) -> Result<SessionRemovalGuard, SessionLeaseError> {
        let shared = &self.shared;
        let mut inner = lock_unpoisoned(&shared.inner);
        if inner.released || inner.lease_lock.is_none() {
            return Err(SessionLeaseError::Held {
                path: shared.path.clone(),
                info: None,
            });
        }
        let save_lock =
            match try_take_session_lock_file(&naming::session_lock_file(Path::new(&shared.path))) {
                Ok(lock) => lock,
                Err(SessionFileLockError::Held) => {
                    return Err(SessionLeaseError::Held {
                        path: shared.path.clone(),
                        info: None,
                    });
                }
                Err(SessionFileLockError::Io(err)) => return Err(SessionLeaseError::Io(err)),
            };
        remove_active_owner_if(&shared.path, shared.owner_id);
        let lease_lock = inner.lease_lock.take();
        inner.released = true;
        drop(inner);
        Ok(SessionRemovalGuard::from_parts(
            shared.path.clone(),
            save_lock,
            lease_lock,
            shared.owner_id,
        ))
    }

    /// Release the lease (Go: `Release`): drop the proven
    /// ownership, delete the identity sidecar, remove the lease lock
    /// file while the lock is still held, then retire a stale save
    /// lock file. Idempotent.
    pub fn release(&self) {
        let shared = &self.shared;
        let mut inner = lock_unpoisoned(&shared.inner);
        loop {
            if inner.released {
                return;
            }
            if inner.active_saves == 0 {
                break;
            }
            #[cfg(test)]
            {
                if let Some(hook) = inner.before_release_wait.take() {
                    drop(inner);
                    hook();
                    inner = lock_unpoisoned(&shared.inner);
                    continue;
                }
            }
            inner = shared
                .release_wait
                .wait(inner)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        inner.released = true;
        let lease_lock = inner.lease_lock.take();
        #[cfg(test)]
        let before_release_lock = inner.before_release_lock.take();
        drop(inner);
        remove_active_owner_if(&shared.path, shared.owner_id);
        let _ = remove_session_lease_info(&shared.path);
        remove_owner_if(&shared.path, shared.owner_id);
        #[cfg(test)]
        {
            if let Some(hook) = before_release_lock {
                hook();
            }
        }
        if let Some(lease_lock) = lease_lock {
            let _ = lease_lock.remove_and_unlock();
        }
        let _ = remove_stale_session_lock_sidecar(
            &shared.path,
            &naming::session_lock_file(Path::new(&shared.path)),
        );
    }

    /// Issue a write authority bound to `generation` (Go:
    /// `IssueWriteAuthority`). A zero generation is refused; a
    /// released lease cannot mint tokens. Issuing bumps the lease's
    /// generation and invalidates every previously issued authority.
    pub fn issue_write_authority(
        &self,
        generation: u64,
    ) -> Result<SessionWriteAuthority, WriteAuthorityError> {
        if generation == 0 {
            return Err(WriteAuthorityError::MissingZeroGeneration);
        }
        let mut inner = lock_unpoisoned(&self.shared.inner);
        if inner.released || inner.lease_lock.is_none() {
            return Err(WriteAuthorityError::Stale);
        }
        inner.write_generation = generation;
        drop(inner);
        Ok(SessionWriteAuthority {
            path: self.shared.path.clone(),
            owner_id: self.shared.owner_id,
            generation,
            lease: Arc::clone(&self.shared),
        })
    }
}

// ---------------------------------------------------------------------------
// Write authority (Go: session_write_authority.go)
// ---------------------------------------------------------------------------

/// Errors of a session write authority (Go:
/// `ErrSessionWriteAuthorityMissing`,
/// `ErrSessionWriteAuthorityStale`, and the zero-generation wrap).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteAuthorityError {
    /// No authority is bound where one is required.
    Missing,
    /// The authority was requested with a zero generation (Go:
    /// `fmt.Errorf("%w: zero generation", ErrSessionWriteAuthorityMissing)`).
    MissingZeroGeneration,
    /// The authority no longer matches the lease that issued it.
    Stale,
}

impl std::fmt::Display for WriteAuthorityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteAuthorityError::Missing => write!(f, "session write authority missing"),
            WriteAuthorityError::MissingZeroGeneration => {
                write!(f, "session write authority missing: zero generation")
            }
            WriteAuthorityError::Stale => write!(f, "session write authority stale"),
        }
    }
}

impl std::error::Error for WriteAuthorityError {}

/// The process-wide write-authority generation counter (Go:
/// `sessionWriteGeneration`).
static SESSION_WRITE_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Allocate the next process-wide write-authority generation (Go:
/// `NextSessionWriteGeneration`). Controllers call this on
/// session create, restore, path change, and authority re-issue so a
/// stale token from a previous lifecycle can never authorize a save.
pub fn next_session_write_generation() -> u64 {
    SESSION_WRITE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1
}

/// A generation-bound permission to save one session (Go:
/// `SessionWriteAuthority`). The token is only valid while the
/// lease that issued it still holds the session, still carries the
/// same generation, and the issuing owner is still the proven active
/// owner: reclaiming the lease replaces the active owner, which makes
/// every previously issued authority stale.
#[derive(Clone)]
pub struct SessionWriteAuthority {
    path: String,
    owner_id: u64,
    generation: u64,
    lease: Arc<LeaseShared>,
}

impl SessionWriteAuthority {
    /// The canonical session path this authority covers.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The generation this authority was issued under.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether the authority still authorizes saves (Go: `Valid`).
    pub fn valid(&self) -> bool {
        if self.generation == 0 {
            return false;
        }
        let inner = lock_unpoisoned(&self.lease.inner);
        if inner.released || inner.lease_lock.is_none() {
            return false;
        }
        if self.lease.path != self.path
            || self.lease.owner_id != self.owner_id
            || inner.write_generation != self.generation
        {
            return false;
        }
        drop(inner);
        lock_unpoisoned(lease_active_owners()).get(&self.path) == Some(&self.owner_id)
    }

    /// Whether the authority still covers `path` (Go: `Covers`).
    pub fn covers(&self, path: &str) -> bool {
        self.valid() && self.path == canonical_session_save_path(path)
    }

    /// Open a save under this authority (Go: `BeginSave`): the
    /// returned guard marks the save in flight; releasing the lease
    /// parks until the guard drops. Exactly-once through Drop.
    pub fn begin_save(&self, path: &str) -> Result<SessionWriteAuthoritySave, WriteAuthorityError> {
        if self.generation == 0 {
            return Err(WriteAuthorityError::Missing);
        }
        let mut inner = lock_unpoisoned(&self.lease.inner);
        if self.path != canonical_session_save_path(path)
            || inner.released
            || inner.lease_lock.is_none()
            || self.lease.path != self.path
            || self.lease.owner_id != self.owner_id
            || inner.write_generation != self.generation
        {
            return Err(WriteAuthorityError::Stale);
        }
        // Holding the inner lock keeps release from completing between
        // the registry check and the in-flight bump.
        if lock_unpoisoned(lease_active_owners()).get(&self.path) != Some(&self.owner_id) {
            return Err(WriteAuthorityError::Stale);
        }
        inner.active_saves += 1;
        drop(inner);
        Ok(SessionWriteAuthoritySave {
            lease: Arc::clone(&self.lease),
        })
    }
}

/// An in-flight authority-guarded save (Go: the `sync.Once`
/// closure `BeginSave` returns). Dropping the guard ends the
/// save and wakes a parked release.
pub struct SessionWriteAuthoritySave {
    lease: Arc<LeaseShared>,
}

impl Drop for SessionWriteAuthoritySave {
    fn drop(&mut self) {
        let mut inner = lock_unpoisoned(&self.lease.inner);
        if inner.active_saves > 0 {
            inner.active_saves -= 1;
        }
        if inner.active_saves == 0 {
            drop(inner);
            self.lease.release_wait.notify_all();
        }
    }
}

/// Acquire the session lease (Go: `TryAcquireSessionLease`).
/// A competing reservation or lock reports held; the holder identity
/// is attached when readable. On success the identity sidecar is
/// written and proven ownership is published.
pub fn try_acquire_session_lease(path: &str) -> Result<SessionLease, SessionLeaseError> {
    let path = path.trim();
    if path.is_empty() {
        return Err(SessionLeaseError::EmptyPath);
    }
    let path = canonical_session_save_path(path);
    mkdir_all_mode(&go_dir(Path::new(&path)), 0o755)?;
    let owner_id = next_owner_id();
    {
        let mut owners = lock_unpoisoned(lease_owners());
        if owners.contains_key(&path) {
            let info = load_session_lease_info(&path).ok();
            return Err(SessionLeaseError::Held { path, info });
        }
        owners.insert(path.clone(), owner_id);
    }
    let lease_lock = match try_lock_session_lease_file(&path) {
        Ok(lock) => lock,
        Err(err) => {
            remove_owner_if(&path, owner_id);
            return Err(match err {
                SessionLeaseError::Held { .. } | SessionLeaseError::HeldBare => {
                    let info = load_session_lease_info(&path).ok();
                    SessionLeaseError::Held { path, info }
                }
                err => err,
            });
        }
    };
    // The OS lock proves any active-registry entry left without its
    // reservation is stale.
    remove_active_owner(&path);
    let lease = SessionLease::from_parts(path.clone(), owner_id, lease_lock);
    if let Err(err) = save_session_lease_info(&path, &new_session_lease_info(&path)) {
        lease.release();
        return Err(SessionLeaseError::Io(err));
    }
    republish_active_owner(&path, owner_id);
    Ok(lease)
}

/// Reclaim a lease this process left behind (Go:
/// `TryReclaimCurrentProcessSessionLease`): readable info
/// naming another live runtime is never stolen; unreadable or
/// missing info falls through to the lock probe, and holding the
/// lock proves no live lease owns the path.
pub fn try_reclaim_current_process_session_lease(
    path: &str,
) -> Result<SessionLease, SessionLeaseError> {
    let path = canonical_session_save_path(path);
    let info = match load_session_lease_info(&path) {
        Ok(info) => {
            // Readable info naming another live runtime: never steal.
            if info.pid != i64::from(std::process::id()) || info.writer_id != session_writer_id() {
                return Err(SessionLeaseError::Held {
                    path,
                    info: Some(info),
                });
            }
            Some(info)
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        // Unreadable (corrupt or torn) info: a broken sidecar must
        // not wedge the session as busy; fall to the lock probe.
        Err(_) => None,
    };
    let lease_lock = match try_lock_session_lease_file(&path) {
        Ok(lock) => lock,
        Err(SessionLeaseError::Held { .. }) | Err(SessionLeaseError::HeldBare) => {
            return Err(SessionLeaseError::Held { path, info });
        }
        Err(err) => return Err(err),
    };
    let owner_id = next_owner_id();
    // Holding the OS lock proves no live lease owns this path:
    // concurrent reclaimers failed the lock above, and a stale lease
    // released later misses its compare-and-delete.
    remove_active_owner(&path);
    lock_unpoisoned(lease_owners()).insert(path.clone(), owner_id);
    let lease = SessionLease::from_parts(path.clone(), owner_id, lease_lock);
    if let Err(err) = save_session_lease_info(&path, &new_session_lease_info(&path)) {
        lease.release();
        return Err(SessionLeaseError::Io(err));
    }
    republish_active_owner(&path, owner_id);
    Ok(lease)
}

/// Whether this process holds a reservation or a live lease on the
/// canonical path (Go: `sessionLeaseHeldLocally`).
pub(crate) fn session_lease_held_locally(path: &str) -> bool {
    let path = canonical_session_save_path(path);
    lock_unpoisoned(lease_owners()).contains_key(&path)
}

/// Probe lease ownership (Go: `SessionLeaseOwnership`):
/// reservations count as held too, so a caller never acts on a
/// session this process is acquiring. Free answers `Ok(())`;
/// held and unprovable answer through the error kind.
pub fn session_lease_ownership(path: &str) -> Result<(), SessionLeaseError> {
    if path.trim().is_empty() {
        return Ok(());
    }
    let path = canonical_session_save_path(path);
    if lock_unpoisoned(lease_owners()).contains_key(&path) {
        return Err(SessionLeaseError::HeldBare);
    }
    probe_session_lease_ownership(&path)
}

/// Whether another runtime may hold the lease (Go:
/// `SessionLeaseHeldByOtherRuntime`): unprovable answers true,
/// because a session that may be busy is treated as busy. The
/// process-local registries are not consulted.
pub fn session_lease_held_by_other_runtime(path: &str) -> bool {
    if path.trim().is_empty() {
        return false;
    }
    let path = canonical_session_save_path(path);
    probe_session_lease_ownership(&path).is_err()
}

/// Whether this process currently owns the lease (Go:
/// `SessionLeaseHeldByCurrentRuntime`): only proven
/// generations count; a pending reservation is not ownership.
pub fn session_lease_held_by_current_runtime(path: &str) -> bool {
    if path.trim().is_empty() {
        return false;
    }
    let path = canonical_session_save_path(path);
    lock_unpoisoned(lease_active_owners()).contains_key(&path)
}

/// The three-answer ownership probe (Go:
/// `probeSessionLeaseOwnership`): free, held, or unprovable.
fn probe_session_lease_ownership(path: &str) -> Result<(), SessionLeaseError> {
    if lock_unpoisoned(lease_active_owners()).contains_key(path) {
        // Held by this process: no file touch.
        return Ok(());
    }
    let lease_info_path = lease_info_path(path);
    match load_session_lease_info(path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            // The common free case touches nothing: check for the
            // lock artifact before attempting the lock.
            if !session_artifact_exists(&naming::session_lease_lock(Path::new(path))) {
                return Ok(());
            }
            match try_lock_session_lease_file(path) {
                Ok(lock) => {
                    lock.unlock();
                    Ok(())
                }
                Err(err) => Err(session_lease_lock_verdict(path, None, err)),
            }
        }
        // Corrupt or empty sidecar.
        Err(_) => match try_lock_session_lease_file(path) {
            // A crashed process left it behind: remove the bad
            // metadata so probes stop reporting a ghost owner.
            Ok(lock) => {
                let _ = fs::remove_file(&lease_info_path);
                lock.unlock();
                Ok(())
            }
            Err(err) => Err(session_lease_lock_verdict(path, None, err)),
        },
        Ok(info) => {
            if info.pid == i64::from(std::process::id()) && info.writer_id == session_writer_id() {
                return Ok(());
            }
            // Foreign info: a free lock means the holder crashed;
            // retire the stale sidecar.
            match try_lock_session_lease_file(path) {
                Ok(lock) => {
                    let _ = fs::remove_file(&lease_info_path);
                    lock.unlock();
                    Ok(())
                }
                Err(err) => Err(session_lease_lock_verdict(path, Some(info), err)),
            }
        }
    }
}

/// Map a lock failure to the three-way answer (Go:
/// `sessionLeaseLockVerdict`): held stays held, anything else
/// is unprovable.
fn session_lease_lock_verdict(
    path: &str,
    info: Option<SessionLeaseInfo>,
    err: SessionLeaseError,
) -> SessionLeaseError {
    if err.is_held() {
        return SessionLeaseError::Held {
            path: path.to_string(),
            info,
        };
    }
    SessionLeaseError::Unprovable {
        source: Some(Box::new(err)),
    }
}

/// Take the lease lock (Go: `tryTakeSessionLeaseLock` and
/// `tryLockSessionLeaseFile`): the same lock file and the same
/// held/plain-io split, wrapped in the lease error type.
pub(crate) fn try_lock_session_lease_file(
    path: &str,
) -> Result<SessionLockFile, SessionLeaseError> {
    let lock_path = naming::session_lease_lock(Path::new(&canonical_session_save_path(path)));
    try_take_session_lock_file(&lock_path).map_err(|err| match err {
        SessionFileLockError::Held => SessionLeaseError::Held {
            path: path.to_string(),
            info: None,
        },
        SessionFileLockError::Io(err) => SessionLeaseError::Io(err),
    })
}

/// Load the lease info sidecar (Go: `LoadSessionLeaseInfo`): a
/// missing file reports `NotFound`; a parse failure is a
/// corrupt sidecar.
pub fn load_session_lease_info(path: &str) -> io::Result<SessionLeaseInfo> {
    let bytes = read_file_utf8(&lease_info_path(path))?;
    serde_json::from_slice(&bytes).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

/// Write the lease info sidecar atomically (Go:
/// `SaveSessionLeaseInfo`): pretty-printed, HTML-escaped, one
/// trailing newline, temp file plus replace.
pub fn save_session_lease_info(path: &str, info: &SessionLeaseInfo) -> io::Result<()> {
    let lease_path = lease_info_path(path);
    let mut json = serde_json::to_string_pretty(info)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    json = escape_html(&json);
    json.push('\n');
    let dir = go_dir(&lease_path);
    mkdir_all_mode(&dir, 0o755)?;
    let (mut file, tmp) = create_temp_file(&dir, ".lease.", ".tmp")?;
    if let Err(err) = file.write_all(json.as_bytes()) {
        drop(file);
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    drop(file);
    if let Err(err) = replace_file(&tmp, &lease_path) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
}

/// Remove the lease info sidecar (Go: the `os.Remove` calls at
/// release and guard-abort time); a missing file is tolerated.
pub(crate) fn remove_session_lease_info(path: &str) -> io::Result<()> {
    match fs::remove_file(lease_info_path(path)) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Record a save that happened without a held lease (Go:
/// `observeUnleasedSessionWrite`): the registry miss is the
/// write-authority violation, and the deduplicated set keeps one
/// record per canonical path per process.
pub(crate) fn observe_unleased_session_write(path: &str, _mode: SessionSaveMode) {
    let path = canonical_session_save_path(path);
    if lock_unpoisoned(lease_owners()).contains_key(&path) {
        return;
    }
    lock_unpoisoned(unleased_write_observed()).insert(path);
}

/// Whether an unleased write was observed for the canonical path.
#[cfg(test)]
pub(crate) fn unleased_write_recorded(path: &str) -> bool {
    let path = canonical_session_save_path(path);
    lock_unpoisoned(unleased_write_observed()).contains(&path)
}

/// Retire a stale save lock file left by a crashed runtime (Go:
/// `removeStaleSessionLockSidecar`): when no lease is held,
/// take the lock and delete the file under it; a held lock means the
/// sidecar is live and stays.
pub(crate) fn remove_stale_session_lock_sidecar(
    session_path: &str,
    sidecar: &Path,
) -> io::Result<()> {
    let path = canonical_session_save_path(session_path);
    if session_lease_held_locally(&path) || session_lease_held_by_other_runtime(&path) {
        return Ok(());
    }
    match try_take_session_lock_file(sidecar) {
        Ok(lock) => lock.remove_and_unlock(),
        Err(SessionFileLockError::Held) => Ok(()),
        Err(SessionFileLockError::Io(err)) => Err(err),
    }
}

/// Retire a stale lease lock file left by a crashed runtime (Go:
/// `removeStaleSessionLeaseLockSidecar`): the lease lock is only
/// present while some runtime holds the lease, so a takeable lock
/// file is garbage.
pub(crate) fn remove_stale_session_lease_lock_sidecar(
    session_path: &str,
    sidecar: &Path,
) -> io::Result<()> {
    let path = canonical_session_save_path(session_path);
    if session_lease_held_locally(&path) {
        return Ok(());
    }
    match try_take_session_lock_file(sidecar) {
        Ok(lock) => lock.remove_and_unlock(),
        Err(SessionFileLockError::Held) => Ok(()),
        Err(SessionFileLockError::Io(err)) => Err(err),
    }
}

/// Retire a stale lease info sidecar (Go:
/// `removeStaleSessionLeaseInfoSidecar`): lease holders keep the lock
/// file present and locked for their whole lifetime, so an info file
/// without a lock file belongs to no one.
pub(crate) fn remove_stale_session_lease_info_sidecar(
    session_path: &str,
    sidecar: &Path,
) -> io::Result<()> {
    let path = canonical_session_save_path(session_path);
    if session_lease_held_locally(&path) {
        return Ok(());
    }
    let lock_path = naming::session_lease_lock(Path::new(&path));
    match fs::metadata(&lock_path) {
        Ok(_) => {
            let lock = match try_lock_session_lease_file(&path) {
                Ok(lock) => lock,
                Err(err) if err.is_held() => return Ok(()),
                Err(err) => return Err(io::Error::other(err.to_string())),
            };
            let outcome = match fs::remove_file(sidecar) {
                Ok(()) => Ok(()),
                Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(err),
            };
            lock.unlock();
            outcome
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => match fs::remove_file(sidecar) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        },
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessionstore::eventlog::test_support::tempdir;
    use std::sync::mpsc;

    fn lease_test_dir() -> (tempfile::TempDir, PathBuf, String) {
        let dir = tempdir();
        let path = dir.path().join("Sessions-Dir").join("Session-Test.jsonl");
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        let key = canonical_session_save_path(&path.to_string_lossy());
        (dir, path, key)
    }

    fn own_info(path: &str) -> SessionLeaseInfo {
        SessionLeaseInfo {
            session_path: path.to_string(),
            writer_id: session_writer_id().to_string(),
            pid: i64::from(std::process::id()),
            hostname: String::new(),
            acquired_at: now_utc(),
        }
    }

    fn foreign_info(path: &str) -> SessionLeaseInfo {
        SessionLeaseInfo {
            session_path: path.to_string(),
            writer_id: "other-host-4242-beef".to_string(),
            pid: i64::from(std::process::id()) + 1,
            hostname: String::new(),
            acquired_at: now_utc(),
        }
    }

    #[test]
    fn lease_rejects_concurrent_writer_and_releases() {
        let (_dir, path, key) = lease_test_dir();
        let path_text = path.to_string_lossy().into_owned();
        let first = try_acquire_session_lease(&path_text).expect("first acquire");
        assert!(!first.path().is_empty());
        let info = load_session_lease_info(&path_text).expect("info written");
        assert!(!info.writer_id.is_empty());
        assert_ne!(info.pid, 0);
        assert!(!info.session_path.is_empty());
        let second = try_acquire_session_lease(&path_text);
        assert!(second.expect_err("second acquire").is_held());
        first.release();
        let third = try_acquire_session_lease(&path_text).expect("third acquire");
        assert_eq!(third.path(), key);
        third.release();
    }

    #[test]
    fn lease_reclaims_current_process_stale_owner() {
        let (_dir, path, key) = lease_test_dir();
        let path_text = path.to_string_lossy().into_owned();
        lock_unpoisoned(lease_owners()).insert(key, u64::MAX);
        save_session_lease_info(&path_text, &own_info(&path_text)).expect("seed info");
        let acquired = try_acquire_session_lease(&path_text);
        assert!(acquired.expect_err("owned path").is_held());
        try_reclaim_current_process_session_lease(&path_text)
            .expect("reclaim")
            .release();
    }

    #[test]
    fn lease_reclaims_orphaned_entry_without_info() {
        let (_dir, path, key) = lease_test_dir();
        let path_text = path.to_string_lossy().into_owned();
        lock_unpoisoned(lease_owners()).insert(key, 1 << 61);
        let acquired = try_acquire_session_lease(&path_text);
        assert!(acquired.expect_err("owned path").is_held());
        let lease =
            try_reclaim_current_process_session_lease(&path_text).expect("reclaim orphaned");
        load_session_lease_info(&path_text).expect("info rewritten after reclaim");
        lease.release();
    }

    #[test]
    fn lease_reclaims_orphaned_entry_with_corrupt_info() {
        let (_dir, path, key) = lease_test_dir();
        let path_text = path.to_string_lossy().into_owned();
        lock_unpoisoned(lease_owners()).insert(key, 1 << 61);
        fs::write(lease_info_path(&path_text), b"{torn").expect("seed torn info");
        try_reclaim_current_process_session_lease(&path_text)
            .expect("reclaim ignores corrupt info")
            .release();
    }

    #[test]
    fn lease_reclaim_refuses_foreign_info() {
        let (_dir, path, _key) = lease_test_dir();
        let path_text = path.to_string_lossy().into_owned();
        save_session_lease_info(&path_text, &foreign_info(&path_text)).expect("seed foreign");
        let err = try_reclaim_current_process_session_lease(&path_text)
            .expect_err("never steal readable foreign info");
        assert!(err.is_held());
    }

    #[test]
    fn lease_concurrent_reclaim_single_winner() {
        let (_dir, path, key) = lease_test_dir();
        let path_text = path.to_string_lossy().into_owned();
        lock_unpoisoned(lease_owners()).insert(key.clone(), u64::MAX);
        save_session_lease_info(&path_text, &own_info(&path_text)).expect("seed info");
        let (winners, losers) = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..16)
                .map(|_| {
                    let text = path_text.clone();
                    scope.spawn(move || try_reclaim_current_process_session_lease(&text))
                })
                .collect();
            let mut winners = Vec::new();
            let mut losers = 0;
            for handle in handles {
                match handle.join().expect("thread") {
                    Ok(lease) => winners.push(lease),
                    Err(_) => losers += 1,
                }
            }
            (winners, losers)
        });
        assert_eq!(winners.len(), 1);
        assert_eq!(winners.len() + losers, 16);
        // The losers did not evict the winner's registry entry.
        assert!(lock_unpoisoned(lease_owners()).contains_key(&key));
        assert!(
            try_acquire_session_lease(&path_text)
                .expect_err("held by winner")
                .is_held()
        );
        winners.into_iter().next().expect("winner").release();
        try_acquire_session_lease(&path_text)
            .expect("acquire after release")
            .release();
    }

    #[test]
    fn lease_reclaim_refuses_active_holder() {
        let (_dir, path, _key) = lease_test_dir();
        let path_text = path.to_string_lossy().into_owned();
        let holder = try_acquire_session_lease(&path_text).expect("holder");
        let err = try_reclaim_current_process_session_lease(&path_text).expect_err("held");
        assert!(err.is_held());
        assert!(session_lease_held_locally(&path_text));
        assert!(try_acquire_session_lease(&path_text).is_err());
        holder.release();
    }

    #[test]
    fn lease_reclaim_after_holder_released() {
        let (_dir, path, _key) = lease_test_dir();
        let path_text = path.to_string_lossy().into_owned();
        try_acquire_session_lease(&path_text)
            .expect("acquire")
            .release();
        try_reclaim_current_process_session_lease(&path_text)
            .expect("reclaim after release")
            .release();
    }

    #[test]
    fn lease_stale_release_keeps_new_owner_entry() {
        let (_dir, path, key) = lease_test_dir();
        let path_text = path.to_string_lossy().into_owned();
        let stale = try_acquire_session_lease(&path_text).expect("stale acquire");
        // Simulate a reclaim takeover by a newer generation.
        lock_unpoisoned(lease_owners()).insert(key.clone(), 1 << 62);
        stale.release();
        assert!(lock_unpoisoned(lease_owners()).contains_key(&key));
    }

    #[test]
    fn lease_held_by_other_runtime_answers() {
        // No lease: not held.
        let (_dir, path, _key) = lease_test_dir();
        let path_text = path.to_string_lossy().into_owned();
        assert!(!session_lease_held_by_other_runtime(&path_text));
        // Held by this process: not another runtime.
        let lease = try_acquire_session_lease(&path_text).expect("acquire");
        assert!(!session_lease_held_by_other_runtime(&path_text));
        lease.release();
        // Foreign info plus a live lock: another runtime's answer is
        // true.
        let other = lease_test_dir();
        let other_text = other.1.to_string_lossy().into_owned();
        let lock = try_lock_session_lease_file(&other_text).expect("lease lock");
        save_session_lease_info(&other_text, &foreign_info(&other_text)).expect("foreign info");
        assert!(session_lease_held_by_other_runtime(&other_text));
        lock.unlock();
        // Foreign info from a crashed process: the probe retires the
        // stale sidecar.
        assert!(!session_lease_held_by_other_runtime(&other_text));
        assert!(!lease_info_path(&other_text).exists());
        // Corrupt info: the probe removes the bad metadata.
        fs::write(lease_info_path(&other_text), b"").expect("empty info");
        assert!(!session_lease_held_by_other_runtime(&other_text));
        assert!(!lease_info_path(&other_text).exists());
    }

    #[test]
    fn lease_held_by_current_runtime_answers() {
        let (_dir, path, _key) = lease_test_dir();
        let path_text = path.to_string_lossy().into_owned();
        assert!(!session_lease_held_by_current_runtime(&path_text));
        let lease = try_acquire_session_lease(&path_text).expect("acquire");
        assert!(session_lease_held_by_current_runtime(&path_text));
        lease.release();
        assert!(!session_lease_held_by_current_runtime(&path_text));
    }

    #[test]
    fn lease_pending_reservation_is_not_current_ownership() {
        let (_dir, path, key) = lease_test_dir();
        let owner_id = next_owner_id();
        lock_unpoisoned(lease_owners()).insert(key, owner_id);
        assert!(!session_lease_held_by_current_runtime(
            &path.to_string_lossy()
        ));
    }

    #[test]
    fn lease_release_revokes_repair_authorization_before_unlock() {
        let (_dir, path, _key) = lease_test_dir();
        let path_text = path.to_string_lossy().into_owned();
        let lease = try_acquire_session_lease(&path_text).expect("acquire");
        let (tx, rx) = mpsc::channel();
        let hook_path = path_text.clone();
        {
            let mut inner = lock_unpoisoned(&lease.shared.inner);
            inner.before_release_lock = Some(Box::new(move || {
                let still = session_lease_held_by_current_runtime(&hook_path);
                tx.send(still).expect("send");
            }));
        }
        lease.release();
        assert!(!rx.recv().expect("hook ran"));
    }

    #[test]
    fn lease_release_retires_lock_sidecars() {
        let (_dir, path, _key) = lease_test_dir();
        let path_text = path.to_string_lossy().into_owned();
        let lease = try_acquire_session_lease(&path_text).expect("acquire");
        // A failed competing acquire must not disturb the holder's
        // lease lock.
        assert!(try_acquire_session_lease(&path_text).is_err());
        assert!(naming::session_lease_lock(&path).exists());
        lease.release();
        assert!(!naming::session_lease_lock(&path).exists());
        assert!(!naming::session_lock_file(&path).exists());
        assert!(!lease_info_path(&path_text).exists());
    }

    #[test]
    fn live_lock_without_sidecar_is_still_held() {
        let (_dir, path, _key) = lease_test_dir();
        let path_text = path.to_string_lossy().into_owned();
        let lock = try_lock_session_lease_file(&path_text).expect("lease lock");
        assert!(session_lease_held_by_other_runtime(&path_text));
        lock.unlock();
    }

    #[test]
    fn probe_does_not_create_the_lease_lock() {
        let (_dir, path, _key) = lease_test_dir();
        let path_text = path.to_string_lossy().into_owned();
        // The common uncontended case touches nothing.
        assert!(!session_lease_held_by_other_runtime(&path_text));
        assert!(!naming::session_lease_lock(&path).exists());
        assert!(session_lease_ownership(&path_text).is_ok());
    }

    #[test]
    fn lease_ownership_keeps_the_three_answers_apart() {
        // Free: no sidecar, no lock file.
        let (_dir, path, _key) = lease_test_dir();
        let path_text = path.to_string_lossy().into_owned();
        assert!(session_lease_ownership(&path_text).is_ok());
        // Foreign sidecar plus a live lock: held.
        let lock = try_lock_session_lease_file(&path_text).expect("lease lock");
        save_session_lease_info(&path_text, &foreign_info(&path_text)).expect("foreign info");
        let err = session_lease_ownership(&path_text).expect_err("held");
        assert!(err.is_held());
        assert!(!err.is_unprovable());
        lock.unlock();
        // Crashed foreign leftover: the probe retires the stale
        // sidecar and answers free.
        assert!(session_lease_ownership(&path_text).is_ok());
        assert!(!lease_info_path(&path_text).exists());
        // A live lock without its sidecar: held.
        let lock = try_lock_session_lease_file(&path_text).expect("lease lock");
        let err = session_lease_ownership(&path_text).expect_err("held");
        assert!(err.is_held());
        lock.unlock();
        // An unopenable lock file: unprovable, not held; the
        // may-be-busy surface still answers true.
        let (_dir2, path2, _key2) = lease_test_dir();
        let path2_text = path2.to_string_lossy().into_owned();
        fs::create_dir_all(naming::session_lease_lock(&path2)).expect("directory at lock path");
        let err = session_lease_ownership(&path2_text).expect_err("unprovable");
        assert!(err.is_unprovable());
        assert!(!err.is_held());
        assert!(session_lease_held_by_other_runtime(&path2_text));
    }

    #[test]
    fn unleased_write_probe_records_once_per_path() {
        let (_dir, path, _key) = lease_test_dir();
        let path_text = path.to_string_lossy().into_owned();
        observe_unleased_session_write(&path_text, SessionSaveMode::Snapshot);
        assert!(unleased_write_recorded(&path_text));
        // The record stays after a second observation.
        observe_unleased_session_write(&path_text, SessionSaveMode::Rewrite);
        assert!(unleased_write_recorded(&path_text));
        // A lease holder's save is not recorded.
        let leased = lease_test_dir();
        let leased_text = leased.1.to_string_lossy().into_owned();
        let lease = try_acquire_session_lease(&leased_text).expect("acquire");
        observe_unleased_session_write(&leased_text, SessionSaveMode::Snapshot);
        assert!(!unleased_write_recorded(&leased_text));
        lease.release();
    }

    #[test]
    fn write_authority_stale_after_release() {
        let (_dir, path, _key) = lease_test_dir();
        let path_text = path.to_string_lossy().into_owned();
        let lease = try_acquire_session_lease(&path_text).expect("acquire");
        let authority = lease
            .issue_write_authority(next_session_write_generation())
            .expect("issue authority");
        assert!(authority.valid());
        assert!(authority.covers(&path_text));
        lease.release();
        assert!(!authority.valid());
        assert!(matches!(
            authority.begin_save(&path_text),
            Err(WriteAuthorityError::Stale)
        ));
    }

    #[test]
    fn write_authority_release_waits_for_in_flight_save() {
        let (_dir, path, _key) = lease_test_dir();
        let path_text = path.to_string_lossy().into_owned();
        let lease = try_acquire_session_lease(&path_text).expect("acquire");
        let authority = lease.issue_write_authority(1).expect("issue authority");
        let save = authority.begin_save(&path_text).expect("begin save");
        let (tx, rx) = mpsc::channel();
        {
            let mut inner = lock_unpoisoned(&lease.shared.inner);
            inner.before_release_wait = Some(Box::new(move || {
                tx.send(()).expect("send");
            }));
        }
        let releaser = std::thread::spawn(move || {
            lease.release();
        });
        rx.recv().expect("release parked behind the in-flight save");
        assert!(!releaser.is_finished());
        drop(save);
        releaser.join().expect("release finished");
        assert!(!authority.valid());
    }

    #[test]
    fn write_authority_generation_invalidates_prior_token() {
        let (_dir, path, _key) = lease_test_dir();
        let path_text = path.to_string_lossy().into_owned();
        let lease = try_acquire_session_lease(&path_text).expect("acquire");
        let old = lease
            .issue_write_authority(next_session_write_generation())
            .expect("old authority");
        let new = lease
            .issue_write_authority(next_session_write_generation())
            .expect("new authority");
        assert!(!old.valid());
        assert!(matches!(
            old.begin_save(&path_text),
            Err(WriteAuthorityError::Stale)
        ));
        assert!(new.valid());
    }
}
