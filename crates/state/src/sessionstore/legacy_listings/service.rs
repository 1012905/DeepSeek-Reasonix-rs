use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::discovery::{
    discover_legacy_sessions, legacy_session_info, legacy_session_visible, sort_legacy_rows,
};
use super::resolver::{clean_directory, same_source};
use super::{
    CancellationToken, LegacyListingError, LegacyReplay, Listing, read_legacy_transcript,
    resolver_for,
};
use crate::sessionstore::listing::SessionInfo;
use crate::sessionstore::sessionv4::Session as V4Session;
use crate::sessionstore::v4_import::v4_import_path;

const TICK: Duration = Duration::from_millis(250);
const SCAN_FRESHNESS: Duration = Duration::from_secs(5);
const ERROR_RETRY: Duration = Duration::from_secs(60);
const WRITE_WINDOW: Duration = Duration::from_secs(1);
const WRITE_BATCH: usize = 32;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ListingState {
    #[default]
    Discovering,
    Indexing,
    Ready,
    Degraded,
}

impl ListingState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Discovering => "discovering",
            Self::Indexing => "indexing",
            Self::Ready => "ready",
            Self::Degraded => "degraded",
        }
    }
}

impl fmt::Display for ListingState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RowState {
    #[default]
    Pending,
    Ready,
    Error,
}

impl RowState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::Error => "error",
        }
    }
}

impl fmt::Display for RowState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListingStatus {
    #[serde(skip_serializing_if = "is_default_listing_state")]
    pub listing_state: ListingState,
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub listing_revision: u64,
    pub counts_complete: bool,
    #[serde(skip_serializing_if = "is_zero_usize")]
    pub pending_metadata: usize,
    #[serde(skip_serializing_if = "is_zero_usize")]
    pub failed_metadata: usize,
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

fn is_zero_usize(value: &usize) -> bool {
    *value == 0
}

fn is_default_listing_state(state: &ListingState) -> bool {
    *state == ListingState::Discovering
}

#[derive(Clone, Debug)]
struct LegacyRow {
    source: V4Session,
    listing: Listing,
    state: RowState,
    retry_at: Option<Instant>,
}

#[derive(Debug, Default)]
struct WriteSchedule {
    next: Option<Instant>,
    writes: usize,
}

#[derive(Debug)]
struct DirectoryState {
    rows: BTreeMap<String, LegacyRow>,
    status: ListingStatus,
    scan_at: Option<Instant>,
    scan_needed: bool,
    priority: BTreeSet<PathBuf>,
    writes: WriteSchedule,
    cache_error: bool,
    scan_error: bool,
    scan_failures: usize,
}

impl DirectoryState {
    fn new() -> Self {
        Self {
            rows: BTreeMap::new(),
            status: ListingStatus::default(),
            scan_at: None,
            scan_needed: true,
            priority: BTreeSet::new(),
            writes: WriteSchedule::default(),
            cache_error: false,
            scan_error: false,
            scan_failures: 0,
        }
    }

    fn flush_due(&self, now: Instant) -> bool {
        self.writes.writes >= WRITE_BATCH || self.writes.next.is_none_or(|next| now >= next)
    }

    fn scheduled_flush(&mut self, now: Instant) {
        self.writes.next = Some(now + WRITE_WINDOW);
        self.writes.writes = 0;
    }
}

type Observer = Arc<dyn Fn() + Send + Sync + 'static>;

#[derive(Default)]
struct ServiceState {
    dirs: BTreeMap<PathBuf, DirectoryState>,
    observers: HashMap<u64, Observer>,
    next_observer: u64,
    changed: bool,
    last_notice: Option<Instant>,
}

struct ServiceInner {
    state: Mutex<ServiceState>,
    wake: SyncSender<()>,
    token: CancellationToken,
    closed: AtomicBool,
    replay: LegacyReplay,
}

impl ServiceInner {
    fn signal(&self) {
        let _ = self.wake.try_send(());
    }
}

#[derive(Clone)]
pub struct LegacyListings {
    inner: Arc<ServiceInner>,
}

impl fmt::Debug for LegacyListings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyListings")
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

impl LegacyListings {
    pub fn new<F>(replay: F) -> Self
    where
        F: Fn(&V4Session, &CancellationToken) -> Result<super::Transcript, LegacyListingError>
            + Send
            + Sync
            + 'static,
    {
        let (wake, receiver) = mpsc::sync_channel(1);
        let inner = Arc::new(ServiceInner {
            state: Mutex::new(ServiceState::default()),
            wake,
            token: CancellationToken::new(),
            closed: AtomicBool::new(false),
            replay: Arc::new(replay),
        });
        let service = Self { inner };
        let weak = Arc::downgrade(&service.inner);
        std::thread::Builder::new()
            .name("reasonix-legacy-listings".into())
            .spawn(move || run(weak, receiver))
            .expect("start legacy listing worker");
        service
    }

    pub fn with_default_replay() -> Self {
        Self::new(read_legacy_transcript)
    }

    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire)
    }

    pub fn close(&self) {
        if self.inner.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.inner.token.cancel();
        self.inner.signal();
    }

    pub fn snapshot(&self, dir: &Path) -> (Vec<SessionInfo>, ListingStatus) {
        let dir = clean_directory(dir);
        let now = Instant::now();
        let (rows, status) = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            let directory = state
                .dirs
                .entry(dir.clone())
                .or_insert_with(DirectoryState::new);
            if directory.scan_at.is_none_or(|scan_at| now >= scan_at) {
                directory.scan_needed = true;
            }
            let mut rows = Vec::with_capacity(directory.rows.len());
            for row in directory.rows.values() {
                if row.state == RowState::Ready && row.listing.turns == 0 {
                    continue;
                }
                let mut info = legacy_session_info(
                    &dir,
                    &row.source,
                    &row.listing,
                    row.state == RowState::Ready,
                );
                if !legacy_session_visible(Path::new(&info.path)) {
                    continue;
                }
                info.metadata_state = row.state.as_str().to_string();
                rows.push(info);
            }
            sort_legacy_rows(&mut rows);
            (rows, directory.status.clone())
        };
        self.inner.signal();
        (rows, status)
    }

    pub fn status(&self, dir: &Path) -> ListingStatus {
        self.snapshot(dir).1
    }

    pub fn prioritize(&self, dir: &Path, paths: &[PathBuf]) {
        let dir = clean_directory(dir);
        {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            if let Some(directory) = state.dirs.get_mut(&dir) {
                directory.priority = paths.iter().cloned().collect();
            }
        }
        self.inner.signal();
    }

    pub fn retry(&self, dir: &Path) {
        let dir = clean_directory(dir);
        {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            if let Some(directory) = state.dirs.get_mut(&dir) {
                directory.scan_needed = true;
                for row in directory.rows.values_mut() {
                    if row.state == RowState::Error {
                        row.state = RowState::Pending;
                        row.retry_at = None;
                    }
                }
                update_status(directory);
                state.changed = true;
            }
        }
        self.inner.signal();
    }

    pub fn subscribe<F>(&self, callback: F) -> LegacyListingsSubscription
    where
        F: Fn() + Send + Sync + 'static,
    {
        let id = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            let id = state.next_observer;
            state.next_observer = state.next_observer.wrapping_add(1);
            state.observers.insert(id, Arc::new(callback));
            id
        };
        LegacyListingsSubscription {
            service: self.clone(),
            id,
        }
    }

    fn notify(&self) {
        let callbacks = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            if !state.changed || state.last_notice.is_some_and(|last| last.elapsed() < TICK) {
                return;
            }
            state.changed = false;
            state.last_notice = Some(Instant::now());
            state.observers.values().cloned().collect::<Vec<_>>()
        };
        for callback in callbacks {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback()));
        }
    }

    fn step(&self) -> bool {
        if self.is_closed() {
            return false;
        }
        let now = Instant::now();
        let scan = {
            let state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            state
                .dirs
                .iter()
                .find(|(_, directory)| directory.scan_needed)
                .map(|(dir, _)| dir.clone())
        };
        if let Some(dir) = scan {
            if let Some(directory) = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .dirs
                .get_mut(&dir)
            {
                directory.scan_needed = false;
                directory.scan_at = Some(now + SCAN_FRESHNESS);
            }
            self.scan(&dir);
            return true;
        }

        let flush = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            state
                .dirs
                .iter_mut()
                .find(|(_, directory)| directory.flush_due(now))
                .map(|(dir, directory)| {
                    directory.scheduled_flush(now);
                    dir.clone()
                })
        };
        if let Some(dir) = flush {
            let result = resolver_for(&dir).flush(&dir, false);
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            let Some(directory) = state.dirs.get_mut(&dir) else {
                return true;
            };
            let degraded = result.is_err();
            if directory.cache_error != degraded {
                directory.cache_error = degraded;
                update_status(directory);
                state.changed = true;
            }
            return true;
        }

        let chosen = self.choose_row(now);
        let Some(chosen) = chosen else {
            return false;
        };
        let result = resolver_for(&chosen.dir).resolve_with_token(
            &chosen.row.source,
            Arc::clone(&self.inner.replay),
            &self.inner.token,
        );
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let Some(directory) = state.dirs.get_mut(&chosen.dir) else {
            return true;
        };
        let Some(row) = directory.rows.get_mut(&chosen.id) else {
            return true;
        };
        if !same_source(&row.source, &chosen.row.source) {
            directory.scan_needed = true;
            return true;
        }
        match result {
            Ok(listing) => {
                row.listing = listing;
                row.state = RowState::Ready;
                row.retry_at = None;
                directory.writes.writes += 1;
            }
            Err(error) => {
                row.state = RowState::Error;
                row.retry_at = Some(Instant::now() + ERROR_RETRY);
                if matches!(error, LegacyListingError::SourceChanged) {
                    directory.scan_needed = true;
                }
            }
        }
        update_status(directory);
        state.changed = true;
        true
    }

    fn choose_row(&self, now: Instant) -> Option<ChosenRow> {
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let mut chosen: Option<ChosenRow> = None;
        for (dir, directory) in &state.dirs {
            for (id, row) in &directory.rows {
                if row.state == RowState::Ready || row.retry_at.is_some_and(|retry| now < retry) {
                    continue;
                }
                let candidate = ChosenRow {
                    dir: dir.clone(),
                    id: id.clone(),
                    row: row.clone(),
                    preferred: directory.priority.contains(&v4_import_path(dir, id)),
                };
                if chosen.as_ref().is_none_or(|current| {
                    candidate.preferred && !current.preferred
                        || (candidate.preferred == current.preferred
                            && candidate.row.source.updated_at() > current.row.source.updated_at())
                        || (candidate.preferred == current.preferred
                            && candidate.row.source.updated_at() == current.row.source.updated_at()
                            && (&candidate.dir, candidate.id.as_str())
                                < (&current.dir, current.id.as_str()))
                }) {
                    chosen = Some(candidate);
                }
            }
        }
        chosen
    }

    fn scan(&self, dir: &Path) {
        let discovery = discover_legacy_sessions(dir);
        let failed_scan = discovery.is_err();
        let discovery = discovery.unwrap_or_default();
        for source in discovery.sources {
            if self.is_closed() {
                return;
            }
            let cached = resolver_for(dir).lookup(&source);
            let id = source.manifest.session_id.clone();
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            let Some(directory) = state.dirs.get_mut(dir) else {
                return;
            };
            let replace = directory.rows.get(&id).is_none_or(|previous| {
                !same_source(&previous.source, &source)
                    || (cached.is_some() && previous.state != RowState::Ready)
            });
            if replace {
                let row_state = if cached.is_some() {
                    RowState::Ready
                } else {
                    RowState::Pending
                };
                directory.rows.insert(
                    id,
                    LegacyRow {
                        source,
                        listing: cached.unwrap_or_default(),
                        state: row_state,
                        retry_at: None,
                    },
                );
                update_status(directory);
                state.changed = true;
            }
        }
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let Some(directory) = state.dirs.get_mut(dir) else {
            return;
        };
        directory.scan_failures = discovery.failures;
        directory.scan_error = failed_scan;
        if !failed_scan {
            directory.rows.retain(|id, _| discovery.seen.contains(id));
        }
        directory.scan_at = Some(Instant::now() + SCAN_FRESHNESS);
        directory.status.listing_state = ListingState::Indexing;
        update_status(directory);
        state.changed = true;
    }
}

#[derive(Clone, Debug)]
struct ChosenRow {
    dir: PathBuf,
    id: String,
    row: LegacyRow,
    preferred: bool,
}

fn update_status(directory: &mut DirectoryState) {
    directory.status.pending_metadata = directory
        .rows
        .values()
        .filter(|row| row.state == RowState::Pending)
        .count();
    directory.status.failed_metadata = directory.scan_failures
        + directory
            .rows
            .values()
            .filter(|row| row.state == RowState::Error)
            .count();
    if directory.status.listing_state != ListingState::Discovering {
        directory.status.counts_complete = directory.status.pending_metadata == 0
            && directory.status.failed_metadata == 0
            && !directory.scan_error;
        directory.status.listing_state = if directory.status.failed_metadata > 0
            || directory.cache_error
            || directory.scan_error
        {
            ListingState::Degraded
        } else if directory.status.pending_metadata > 0 {
            ListingState::Indexing
        } else {
            ListingState::Ready
        };
    }
    directory.status.listing_revision = directory.status.listing_revision.wrapping_add(1);
}

fn run(service: Weak<ServiceInner>, receiver: Receiver<()>) {
    while let Some(inner) = service.upgrade() {
        if inner.closed.load(Ordering::Acquire) {
            break;
        }
        let service = LegacyListings {
            inner: Arc::clone(&inner),
        };
        service.notify();
        if service.step() {
            continue;
        }
        let _ = receiver.recv_timeout(TICK);
    }
    if let Some(inner) = service.upgrade() {
        let dirs = inner
            .state
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .dirs
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for dir in dirs {
            let _ = resolver_for(&dir).flush(&dir, false);
        }
    }
}

pub struct LegacyListingsSubscription {
    service: LegacyListings,
    id: u64,
}

impl fmt::Debug for LegacyListingsSubscription {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyListingsSubscription")
            .finish_non_exhaustive()
    }
}

impl Drop for LegacyListingsSubscription {
    fn drop(&mut self) {
        let mut state = self
            .service
            .inner
            .state
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        state.observers.remove(&self.id);
    }
}

#[derive(Debug, Default)]
struct SharedListings {
    service: Option<LegacyListings>,
    refs: usize,
}

pub struct AcquireLegacyListingsGuard {
    service: LegacyListings,
    released: bool,
}

impl fmt::Debug for AcquireLegacyListingsGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcquireLegacyListingsGuard")
            .finish_non_exhaustive()
    }
}

impl Drop for AcquireLegacyListingsGuard {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        let last = {
            let mut shared = shared_listings()
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            shared.refs = shared.refs.saturating_sub(1);
            if shared.refs == 0 {
                shared.service.take();
                true
            } else {
                false
            }
        };
        if last {
            self.service.close();
        }
    }
}

fn shared_listings() -> &'static Mutex<SharedListings> {
    static SHARED: OnceLock<Mutex<SharedListings>> = OnceLock::new();
    SHARED.get_or_init(|| Mutex::new(SharedListings::default()))
}

pub fn acquire_legacy_listings() -> (LegacyListings, AcquireLegacyListingsGuard) {
    let mut shared = shared_listings()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let service = shared
        .service
        .clone()
        .unwrap_or_else(LegacyListings::with_default_replay);
    shared.service = Some(service.clone());
    shared.refs += 1;
    let guard = AcquireLegacyListingsGuard {
        service: service.clone(),
        released: false,
    };
    (service, guard)
}
