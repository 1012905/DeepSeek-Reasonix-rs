use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use crate::sessionstore::eventlog::record::CreatedAt;
use reasonix_core::fileencoding;
use reasonix_core::filelock::{self, FileLockError, FileLockGuard};
use reasonix_core::fileutil::atomic_write_file_strict;
use thiserror::Error;
use tracing::warn;

use super::blob::{BlobError, BlobStore, digest, valid_blob_ref};
use super::path_capture::{CaptureError, capture_path, normalize_rel_path, safe_path};
use super::rewind::PlanRegistry;
use super::secure_path::SecureError;
use super::types::{
    ActiveWriter, CONFLICT_BUSY_WRITER, CaptureAfterOpts, CaptureBeforeOpts, CaptureOptions,
    CaptureSource, ChangeKind, Checkpoint, Coverage, CoverageGap, DEFAULT_BLOB_QUOTA_BYTES,
    DEFAULT_MAX_FILE_BYTES, DEFAULT_RETAIN_CHECKPOINTS, DISABLED_EXPIRED, DISABLED_LEGACY,
    EXPIRED_DETAIL, FileRevision, FileSnap, FileState, GAP_EXPIRED_PAYLOAD, GAP_LEGACY_UNVERIFIED,
    Meta, PreviewChange, RewindConflict, SCHEMA_V1, SCHEMA_V2, TransactionManifest,
    transaction_dir,
};

const LOCK_FILE: &str = ".store.lock";
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct StoreConfig {
    pub retain: usize,
    pub blob_quota_bytes: u64,
    pub max_file_bytes: u64,
    pub lock_timeout: Duration,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            retain: DEFAULT_RETAIN_CHECKPOINTS,
            blob_quota_bytes: DEFAULT_BLOB_QUOTA_BYTES,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            lock_timeout: LOCK_TIMEOUT,
        }
    }
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error(transparent)]
    Lock(#[from] FileLockError),
    #[error(transparent)]
    Blob(#[from] BlobError),
    #[error(transparent)]
    Capture(#[from] CaptureError),
    #[error("{op} {path}: {source}")]
    Io {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("decode {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("persist undo invalidation {id}: {source}")]
    UndoInvalidation {
        id: String,
        #[source]
        source: SecureError,
    },
}

#[derive(Debug, Default)]
pub(super) struct StoreState {
    pub(super) done: Vec<Checkpoint>,
    pub(super) current: Option<Checkpoint>,
    pub(super) seen: HashSet<String>,
    pub(super) session_id: String,
    pub(super) active_writers: Vec<ActiveWriter>,
    pub(super) mutation_seq: i64,
    pub(super) protected_turns: BTreeSet<i64>,
    pub(super) unreadable: BTreeSet<i64>,
}

impl StoreState {
    pub(super) fn all(&self) -> Vec<Checkpoint> {
        let mut checkpoints = self.done.clone();
        if let Some(current) = &self.current {
            checkpoints.push(current.clone());
        }
        checkpoints.sort_by_key(|checkpoint| checkpoint.turn);
        checkpoints
    }

    pub(super) fn checkpoint_mut(&mut self, turn: i64) -> Option<&mut Checkpoint> {
        if self
            .current
            .as_ref()
            .is_some_and(|current| current.turn == turn)
        {
            return self.current.as_mut();
        }
        self.done
            .iter_mut()
            .find(|checkpoint| checkpoint.turn == turn)
    }

    pub(super) fn next_turn(&self) -> i64 {
        let known = self
            .all()
            .iter()
            .map(|checkpoint| checkpoint.turn)
            .max()
            .and_then(|turn| turn.checked_add(1))
            .unwrap_or(0);
        let floor = self
            .unreadable
            .iter()
            .next_back()
            .and_then(|turn| turn.checked_add(1))
            .unwrap_or(0);
        known.max(floor)
    }
}

#[derive(Debug, Default)]
pub(super) struct StoreRuntime {
    pub(super) barrier: super::barrier::MutationBarrier,
    pub(super) plans: Mutex<PlanRegistry>,
    pub(super) recovery_notes: Mutex<Vec<super::transaction::RecoveryNote>>,
    pub(super) undo: super::undo::UndoRuntime,
}

#[derive(Debug)]
pub struct Store {
    pub(super) dir: Option<PathBuf>,
    pub(super) root: PathBuf,
    pub(super) blobs: Option<BlobStore>,
    pub(super) config: StoreConfig,
    pub(super) runtime: StoreRuntime,
    pub(super) state: Mutex<StoreState>,
}

impl Store {
    pub fn new(dir: Option<&Path>, root: &Path) -> Self {
        Self::new_with_config(dir, root, StoreConfig::default())
    }

    pub fn new_with_config(dir: Option<&Path>, root: &Path, config: StoreConfig) -> Self {
        let dir = dir
            .filter(|path| !path.as_os_str().is_empty())
            .map(Path::to_path_buf);
        let root = if root.as_os_str().is_empty() {
            PathBuf::new()
        } else {
            std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf())
        };
        let blobs = dir.as_ref().map(|path| BlobStore::new(path.join("blobs")));
        let state = dir.as_deref().map(load_state).unwrap_or_default();
        let store = Self {
            dir,
            root,
            blobs,
            config,
            runtime: StoreRuntime::default(),
            state: Mutex::new(state),
        };
        store.recover_file_transactions_on_startup();
        store
    }

    pub fn blobs(&self) -> Option<&BlobStore> {
        self.blobs.as_ref()
    }

    pub fn barrier(&self) -> &super::barrier::MutationBarrier {
        &self.runtime.barrier
    }

    pub fn load(&self) -> Result<(), StoreError> {
        let _disk = self.lock_existing_disk()?;
        let loaded = self.dir.as_deref().map(load_state).unwrap_or_default();
        *self.state() = loaded;
        Ok(())
    }

    pub fn begin(&self, turn: i64, prompt: &str, msg_index: i64) -> Result<(), StoreError> {
        let _disk = self.lock_disk()?;
        let mut state = self.state();
        if let Some(current) = state.current.take() {
            let mut current = current;
            current.recompute_coverage();
            state.done.push(current);
        }
        let mut current = Checkpoint {
            schema_version: SCHEMA_V2,
            turn,
            time: CreatedAt::now(),
            prompt: prompt.to_owned(),
            msg_index,
            session_id: state.session_id.clone(),
            active_writers: state.active_writers.clone(),
            coverage: Coverage::None,
            ..Checkpoint::default()
        };
        current.recompute_coverage();
        state.current = Some(current);
        state.seen.clear();
        self.persist_best_effort(state.current.as_ref().expect("current checkpoint"));
        self.gc_best_effort(&mut state);
        Ok(())
    }

    pub fn snapshot(&self, change: PreviewChange) {
        self.capture_before_from_change(
            change,
            CaptureBeforeOpts {
                source: Some(CaptureSource::Previewer),
                ..CaptureBeforeOpts::default()
            },
        );
    }

    pub fn capture_before_from_change(&self, change: PreviewChange, opts: CaptureBeforeOpts) {
        if change.path.is_empty() {
            return;
        }
        let path = Path::new(&change.path);
        let key = normalize_rel_path(&self.root, path);
        let source = opts.source.unwrap_or(CaptureSource::Previewer);
        let mut snap = FileSnap {
            path: change.path.clone(),
            capture_source: Some(source),
            ..FileSnap::default()
        };
        if change.kind != ChangeKind::Create {
            snap.content = Some(change.old_text.clone());
            snap.sha256 = digest(change.old_text.as_bytes());
            let metadata_opts = CaptureOptions {
                workspace_root: self.root.clone(),
                ..CaptureOptions::default()
            };
            match capture_path(path, &metadata_opts) {
                Ok(fingerprint) => snap.mode = fingerprint.mode,
                Err(error) => self.record_gap(error.gap_at(path)),
            }
            let read_opts = CaptureOptions {
                workspace_root: self.root.clone(),
                read_content: true,
                max_bytes: self.config.max_file_bytes,
            };
            match capture_path(path, &read_opts) {
                Ok(fingerprint) => {
                    snap.mode = fingerprint.mode;
                    if let Some(raw) = fingerprint.content {
                        snap.sha256 = digest(&raw);
                        snap.encoding = Some(fingerprint.encoding);
                        if let Some(blobs) = &self.blobs
                            && let Ok(reference) = blobs.put(&raw)
                        {
                            snap.blob_ref = reference;
                        }
                    }
                }
                Err(error) => self.record_gap(error.gap_at(path)),
            }
        }
        let Ok(_disk) = self.lock_existing_disk() else {
            return;
        };
        let mut state = self.state();
        if state.current.is_none() || state.seen.contains(&key) {
            return;
        }
        state.seen.insert(key);
        if snap.blob_ref.is_empty()
            && change.kind != ChangeKind::Create
            && let Some(blobs) = &self.blobs
            && let Ok(reference) = blobs.put(change.old_text.as_bytes())
        {
            snap.blob_ref = reference;
        }
        self.push_snapshot(&mut state, snap);
    }

    pub fn capture_before(&self, path: impl AsRef<Path>, opts: CaptureBeforeOpts) {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return;
        }
        let key = normalize_rel_path(&self.root, path);
        let capture_opts = CaptureOptions {
            workspace_root: self.root.clone(),
            read_content: true,
            max_bytes: self.config.max_file_bytes,
        };
        let fingerprint = match capture_path(path, &capture_opts) {
            Ok(fingerprint) => Some(fingerprint),
            Err(error) => {
                self.record_gap(error.gap_at(path));
                None
            }
        };
        let Ok(_disk) = self.lock_existing_disk() else {
            return;
        };
        let mut state = self.state();
        if state.current.is_none() || state.seen.contains(&key) {
            return;
        }
        state.seen.insert(key);
        let Some(fingerprint) = fingerprint else {
            return;
        };
        let mut snap = FileSnap {
            path: path.to_string_lossy().into_owned(),
            capture_source: Some(opts.source.unwrap_or(CaptureSource::BeforeMutation)),
            ..FileSnap::default()
        };
        if fingerprint.existed
            && let Some(raw) = fingerprint.content
        {
            snap.mode = fingerprint.mode;
            snap.sha256 = digest(&raw);
            snap.encoding = Some(fingerprint.encoding);
            snap.content =
                Some(String::from_utf8_lossy(&fileencoding::decode_to_utf8(&raw)).into_owned());
            if !raw.is_empty()
                && let Some(blobs) = &self.blobs
                && let Ok(reference) = blobs.put(&raw)
            {
                snap.blob_ref = reference;
            }
        }
        self.push_snapshot(&mut state, snap);
    }

    fn push_snapshot(&self, state: &mut StoreState, snap: FileSnap) {
        let Some(current) = state.current.as_mut() else {
            return;
        };
        current.files.push(snap);
        current.schema_version = SCHEMA_V2;
        current.recompute_coverage();
        self.persist_best_effort(current);
    }

    pub fn capture_after(
        &self,
        path: impl AsRef<Path>,
        opts: CaptureAfterOpts,
    ) -> Result<(), StoreError> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Ok(());
        }
        let key = normalize_rel_path(&self.root, path);
        let fingerprint = match capture_path(
            path,
            &CaptureOptions {
                workspace_root: self.root.clone(),
                read_content: true,
                max_bytes: self.config.max_file_bytes,
            },
        ) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                self.record_gap(error.gap_at(path));
                if error.is_outside_workspace() {
                    return Ok(());
                }
                return Err(error.into());
            }
        };
        let _disk = self.lock_existing_disk()?;
        let mut state = self.state();
        state.mutation_seq = opts.seq;
        if let Some(current) = state.current.as_mut() {
            current.last_mutation_seq = opts.seq;
            let mut updated = false;
            for file in &mut current.files {
                if normalize_rel_path(&self.root, Path::new(&file.path)) == key {
                    file.after_existed = Some(fingerprint.existed);
                    file.after_sha256 = fingerprint.sha256.clone();
                    file.after_mode = fingerprint.mode;
                    updated = true;
                }
            }
            if updated {
                current.recompute_coverage();
                return self.persist_checkpoint(current);
            }
        }
        if let Some(turn) = state
            .done
            .iter()
            .rev()
            .find(|checkpoint| {
                checkpoint
                    .files
                    .iter()
                    .any(|file| normalize_rel_path(&self.root, Path::new(&file.path)) == key)
            })
            .map(|checkpoint| checkpoint.turn)
            && let Some(checkpoint) = state.checkpoint_mut(turn)
        {
            for file in &mut checkpoint.files {
                if normalize_rel_path(&self.root, Path::new(&file.path)) == key {
                    file.after_existed = Some(fingerprint.existed);
                    file.after_sha256 = fingerprint.sha256.clone();
                    file.after_mode = fingerprint.mode;
                    break;
                }
            }
            return self.persist_checkpoint(checkpoint);
        }
        Ok(())
    }

    pub fn record_gap(&self, gap: CoverageGap) {
        let Ok(_disk) = self.lock_existing_disk() else {
            return;
        };
        let mut state = self.state();
        let Some(current) = state.current.as_mut() else {
            return;
        };
        if current.coverage_gaps.contains(&gap) {
            return;
        }
        current.coverage_gaps.push(gap);
        current.recompute_coverage();
        self.persist_best_effort(current);
    }

    pub fn set_session_id(&self, id: impl Into<String>) {
        self.state().session_id = id.into();
    }

    pub fn set_active_writers(&self, writers: Vec<ActiveWriter>) {
        let Ok(_disk) = self.lock_existing_disk() else {
            return;
        };
        let mut state = self.state();
        state.active_writers = writers.clone();
        if let Some(current) = state.current.as_mut() {
            current.active_writers = writers;
            current.recompute_coverage();
            self.persist_best_effort(current);
        }
    }

    pub fn active_writer_conflicts(&self) -> Vec<RewindConflict> {
        self.state()
            .active_writers
            .iter()
            .map(|_| RewindConflict {
                reason: CONFLICT_BUSY_WRITER.to_owned(),
                ..RewindConflict::default()
            })
            .collect()
    }

    pub fn persist(&self) -> Result<(), StoreError> {
        let _disk = self.lock_disk()?;
        let state = self.state();
        for checkpoint in state.all() {
            self.persist_checkpoint(&checkpoint)?;
        }
        Ok(())
    }

    pub fn recompute_coverage(&self) -> Result<(), StoreError> {
        let _disk = self.lock_existing_disk()?;
        let mut state = self.state();
        let Some(current) = state.current.as_mut() else {
            return Ok(());
        };
        current.recompute_coverage();
        self.persist_checkpoint(current)
    }

    pub fn next_turn(&self) -> i64 {
        self.state().next_turn()
    }

    pub fn unreadable_turns(&self) -> Vec<i64> {
        self.state().unreadable.iter().copied().collect()
    }

    pub fn bounds(&self) -> BTreeMap<i64, i64> {
        self.state()
            .all()
            .into_iter()
            .map(|checkpoint| (checkpoint.turn, checkpoint.msg_index))
            .collect()
    }

    pub fn file_state(&self, path: impl AsRef<Path>) -> Option<FileState> {
        let path = path.as_ref();
        let wanted = safe_path(&self.root, path).ok()?;
        let state = self.state();
        let mut earliest: Option<FileSnap> = None;
        let mut latest_after_sha = String::new();
        let mut latest_after_existed = None;
        for checkpoint in state.all() {
            for file in checkpoint.files {
                let Ok(got) = safe_path(&self.root, Path::new(&file.path)) else {
                    continue;
                };
                if got != wanted {
                    continue;
                }
                if earliest.is_none() {
                    earliest = Some(file.clone());
                }
                latest_after_sha = file.after_sha256;
                latest_after_existed = file.after_existed;
            }
        }
        let earliest = earliest?;
        if earliest.payload_expired {
            return None;
        }
        let mut state = FileState {
            encoding: earliest.encoding,
            mode: earliest.mode,
            sha256: earliest.sha256,
            blob_ref: earliest.blob_ref,
            owned: !latest_after_sha.is_empty() || latest_after_existed.is_some(),
            content: earliest.content,
        };
        if state.content.is_none()
            && !state.blob_ref.is_empty()
            && let Some(blobs) = &self.blobs
            && let Ok(raw) = blobs.get(&state.blob_ref)
        {
            let (encoding, decoded) = fileencoding::detect_and_decode(&raw);
            state.encoding = Some(encoding);
            state.content = Some(String::from_utf8_lossy(&decoded).into_owned());
        }
        Some(state)
    }

    pub fn revisions(&self) -> Vec<FileRevision> {
        self.state()
            .all()
            .iter()
            .flat_map(Checkpoint::revisions)
            .collect()
    }

    pub fn list(&self) -> Vec<Meta> {
        let state = self.state();
        let checkpoints = state.all();
        let mut metas = vec![Meta::default(); checkpoints.len()];
        let mut seen = HashSet::new();
        for (index, checkpoint) in checkpoints.iter().enumerate().rev() {
            let paths: Vec<String> = checkpoint
                .files
                .iter()
                .map(|file| {
                    seen.insert(normalize_rel_path(&self.root, Path::new(&file.path)));
                    file.path.clone()
                })
                .collect();
            let legacy = checkpoint.legacy || checkpoint.coverage == Coverage::Legacy;
            let disabled_reason = if legacy {
                DISABLED_LEGACY
            } else if checkpoint.expired_file_payload {
                DISABLED_EXPIRED
            } else {
                ""
            };
            metas[index] = Meta {
                turn: checkpoint.turn,
                msg_index: checkpoint.msg_index,
                time: checkpoint.time,
                prompt: checkpoint.prompt.clone(),
                paths,
                rewind_files: seen.len(),
                coverage: checkpoint.coverage,
                coverage_gaps: checkpoint.coverage_gaps.clone(),
                expired_file_payload: checkpoint.expired_file_payload,
                active_writers: checkpoint.active_writers.clone(),
                fork_copied: checkpoint.fork_copied,
                legacy,
                disabled_reason: disabled_reason.to_owned(),
            };
        }
        metas
    }

    pub fn truncate_from(&self, from_turn: i64) -> Result<(), StoreError> {
        let _disk = self.lock_existing_disk()?;
        let mut state = self.state();
        let mut turns: BTreeSet<i64> = state
            .done
            .iter()
            .filter(|checkpoint| checkpoint.turn >= from_turn)
            .map(|checkpoint| checkpoint.turn)
            .collect();
        if state
            .current
            .as_ref()
            .is_some_and(|current| current.turn >= from_turn)
            && let Some(current) = &state.current
        {
            turns.insert(current.turn);
        }
        if let Some(dir) = &self.dir {
            for turn in &turns {
                for path in [turn_path(dir, *turn, false), turn_path(dir, *turn, true)] {
                    match fs::remove_file(&path) {
                        Ok(()) => {}
                        Err(source) if source.kind() == io::ErrorKind::NotFound => {}
                        Err(source) => {
                            return Err(StoreError::Io {
                                op: "remove checkpoint turn",
                                path,
                                source,
                            });
                        }
                    }
                }
            }
        }
        state.done.retain(|checkpoint| checkpoint.turn < from_turn);
        if state
            .current
            .as_ref()
            .is_some_and(|current| current.turn >= from_turn)
        {
            state.current = None;
            state.seen.clear();
        }
        Ok(())
    }

    pub fn protect_turn(&self, turn: i64) {
        self.state().protected_turns.insert(turn);
    }

    pub fn unprotect_turn(&self, turn: i64) {
        self.state().protected_turns.remove(&turn);
    }

    pub fn gc(&self) -> Result<(), StoreError> {
        let _disk = self.lock_existing_disk()?;
        let mut state = self.state();
        self.gc_best_effort(&mut state);
        Ok(())
    }

    pub fn prune_blobs(&self) -> Result<(), StoreError> {
        let _disk = self.lock_existing_disk()?;
        let state = self.state();
        self.prune_blobs_best_effort(&state);
        Ok(())
    }

    fn gc_best_effort(&self, state: &mut StoreState) {
        let Some(blobs) = &self.blobs else {
            return;
        };
        if self.config.retain == 0 {
            return;
        }
        let with_files: Vec<i64> = state
            .all()
            .into_iter()
            .filter(|checkpoint| !checkpoint.files.is_empty())
            .map(|checkpoint| checkpoint.turn)
            .collect();
        if with_files.len() > self.config.retain {
            let expire_count = with_files.len() - self.config.retain;
            for turn in with_files.iter().take(expire_count) {
                if state.protected_turns.contains(turn) {
                    continue;
                }
                let _ = self.expire_payload(state, *turn);
            }
        }
        let mut size = match blobs.size() {
            Ok(size) => size,
            Err(error) => {
                warn!(%error, "checkpoint: size blobs failed");
                return;
            }
        };
        if size > self.config.blob_quota_bytes {
            for turn in with_files {
                if size <= self.config.blob_quota_bytes {
                    break;
                }
                if state.protected_turns.contains(&turn)
                    || state
                        .checkpoint_mut(turn)
                        .is_some_and(|checkpoint| checkpoint.expired_file_payload)
                {
                    continue;
                }
                if self.expire_payload(state, turn).is_err() {
                    continue;
                }
                self.prune_blobs_best_effort(state);
                size = blobs.size().unwrap_or(size);
            }
        }
        self.prune_blobs_best_effort(state);
    }

    fn expire_payload(&self, state: &mut StoreState, turn: i64) -> Result<(), StoreError> {
        let Some(checkpoint) = state.checkpoint_mut(turn) else {
            return Ok(());
        };
        if checkpoint.expired_file_payload {
            return Ok(());
        }
        let mut expired = checkpoint.clone();
        for file in &mut expired.files {
            file.blob_ref.clear();
            file.content = None;
            file.payload_expired = true;
        }
        expired.expired_file_payload = true;
        expired.coverage = Coverage::Partial;
        expired
            .coverage_gaps
            .push(CoverageGap::new(GAP_EXPIRED_PAYLOAD, EXPIRED_DETAIL));
        self.persist_checkpoint(&expired)?;
        if let Some(dir) = &self.dir {
            let root_copy = turn_path(dir, turn, false);
            match fs::remove_file(&root_copy) {
                Ok(()) => {}
                Err(source) if source.kind() == io::ErrorKind::NotFound => {}
                Err(source) => {
                    let _ = fs::remove_file(turn_path(dir, turn, true));
                    return Err(StoreError::Io {
                        op: "remove legacy-visible checkpoint",
                        path: root_copy,
                        source,
                    });
                }
            }
        }
        *checkpoint = expired;
        Ok(())
    }

    fn prune_blobs_best_effort(&self, state: &StoreState) {
        let Some(blobs) = &self.blobs else {
            return;
        };
        let mut live = HashSet::new();
        for checkpoint in state.all() {
            for file in checkpoint.files {
                if valid_blob_ref(&file.blob_ref) {
                    live.insert(file.blob_ref);
                }
            }
        }
        if let Some(dir) = &self.dir {
            let transactions = transaction_dir(dir);
            if let Ok(entries) = fs::read_dir(transactions) {
                for entry in entries.flatten() {
                    if !entry.path().extension().is_some_and(|ext| ext == "json") {
                        continue;
                    }
                    if let Ok(bytes) = fs::read(entry.path())
                        && let Ok(manifest) = serde_json::from_slice::<TransactionManifest>(&bytes)
                    {
                        for target in manifest.targets {
                            if valid_blob_ref(&target.restore_blob) {
                                live.insert(target.restore_blob);
                            }
                            if valid_blob_ref(&target.forward_blob) {
                                live.insert(target.forward_blob);
                            }
                        }
                    }
                }
            }
        }
        if let Err(error) = blobs.prune(&live) {
            warn!(%error, "checkpoint: prune blobs");
        }
    }

    fn persist_best_effort(&self, checkpoint: &Checkpoint) {
        if let Err(error) = self.persist_checkpoint(checkpoint) {
            warn!(turn = checkpoint.turn, %error, "checkpoint: persist failed");
        }
    }

    pub(super) fn persist_checkpoint(&self, checkpoint: &Checkpoint) -> Result<(), StoreError> {
        let Some(dir) = &self.dir else {
            return Ok(());
        };
        let bytes = serde_json::to_vec(checkpoint).map_err(|source| StoreError::Json {
            path: turn_path(dir, checkpoint.turn, checkpoint.expired_file_payload),
            source,
        })?;
        let path = turn_path(dir, checkpoint.turn, checkpoint.expired_file_payload);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| StoreError::Io {
                op: "create checkpoint directory",
                path: parent.to_path_buf(),
                source,
            })?;
        }
        atomic_write_file_strict(&path, &bytes, 0o644).map_err(|source| StoreError::Io {
            op: "write checkpoint",
            path,
            source,
        })
    }

    pub(super) fn lock_existing_disk(&self) -> Result<Option<FileLockGuard>, StoreError> {
        let Some(dir) = &self.dir else {
            return Ok(None);
        };
        match fs::symlink_metadata(dir) {
            Ok(_) => self.lock_disk(),
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(StoreError::Io {
                op: "inspect checkpoint directory",
                path: dir.clone(),
                source,
            }),
        }
    }

    pub(super) fn lock_disk(&self) -> Result<Option<FileLockGuard>, StoreError> {
        let Some(dir) = &self.dir else {
            return Ok(None);
        };
        fs::create_dir_all(dir).map_err(|source| StoreError::Io {
            op: "create checkpoint directory",
            path: dir.clone(),
            source,
        })?;
        filelock::acquire(&dir.join(LOCK_FILE), self.config.lock_timeout)
            .map(Some)
            .map_err(StoreError::from)
    }

    pub(super) fn state(&self) -> MutexGuard<'_, StoreState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
}

pub fn expired_dir(checkpoint_dir: &Path) -> PathBuf {
    checkpoint_dir.join("expired")
}

pub fn turn_path(checkpoint_dir: &Path, turn: i64, expired: bool) -> PathBuf {
    let dir = if expired {
        expired_dir(checkpoint_dir)
    } else {
        checkpoint_dir.to_path_buf()
    };
    dir.join(format!("turn-{turn}.json"))
}

fn load_state(dir: &Path) -> StoreState {
    let mut state = StoreState::default();
    let mut seen = HashSet::new();
    for (path, expired) in [(dir.to_path_buf(), false), (expired_dir(dir), true)] {
        let Ok(entries) = fs::read_dir(path) else {
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                continue;
            }
            let Some(turn) = parse_turn_file(entry.file_name().to_string_lossy().as_ref()) else {
                continue;
            };
            if seen.contains(&turn) {
                continue;
            }
            let path = entry.path();
            let decoded = match fileencoding::read_file_utf8(&path) {
                Ok(bytes) => serde_json::from_slice::<Checkpoint>(&bytes),
                Err(error) => {
                    warn!(%error, ?path, "checkpoint: turn file unreadable; its number stays reserved");
                    state.unreadable.insert(turn);
                    continue;
                }
            };
            let Ok(mut checkpoint) = decoded else {
                warn!(
                    ?path,
                    "checkpoint: turn file is not a checkpoint; its number stays reserved"
                );
                state.unreadable.insert(turn);
                continue;
            };
            if expired {
                checkpoint.expired_file_payload = true;
                for file in &mut checkpoint.files {
                    file.payload_expired = true;
                    file.blob_ref.clear();
                    file.content = None;
                }
                if !checkpoint
                    .coverage_gaps
                    .iter()
                    .any(|gap| gap.reason == GAP_EXPIRED_PAYLOAD)
                {
                    checkpoint
                        .coverage_gaps
                        .push(CoverageGap::new(GAP_EXPIRED_PAYLOAD, EXPIRED_DETAIL));
                }
                checkpoint.recompute_coverage();
            }
            if checkpoint.schema_version < SCHEMA_V2 {
                checkpoint.schema_version = SCHEMA_V1;
                checkpoint.legacy = true;
                checkpoint.coverage = Coverage::Legacy;
                if !checkpoint
                    .coverage_gaps
                    .iter()
                    .any(|gap| gap.reason == GAP_LEGACY_UNVERIFIED)
                {
                    checkpoint.coverage_gaps.push(CoverageGap::new(
                        GAP_LEGACY_UNVERIFIED,
                        "v1 checkpoint cannot verify later manual edits",
                    ));
                }
            }
            state.unreadable.remove(&turn);
            seen.insert(turn);
            state.done.push(checkpoint);
        }
    }
    state.done.sort_by_key(|checkpoint| checkpoint.turn);
    state
}

fn parse_turn_file(name: &str) -> Option<i64> {
    let number = name.strip_prefix("turn-")?.strip_suffix(".json")?;
    if number.is_empty() {
        return None;
    }
    number.parse().ok()
}
