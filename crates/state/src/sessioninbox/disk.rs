//! Transactional disk ownership for one session inbox.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use super::Result;
use super::disk_fs::{DISK_LOCK_POLL, DISK_LOCK_WAIT, TransactionLock, remove_inbox_dir};
use super::manifest::empty_manifest;
use super::types::{Error, InboxSnapshot, Limits};
use crate::store::naming::session_inbox_dir;

pub(super) const TRANSACTION_LOCK_NAME: &str = "transaction.lock";

type Listener = Arc<dyn Fn(InboxSnapshot) + Send + Sync>;

pub struct Store {
    pub(super) state: Mutex<StoreState>,
    pub(super) run_id: &'static str,
    pub(super) limits: Limits,
}

pub(super) struct StoreState {
    pub(super) binding: Binding,
    pub(super) lifecycle: Lifecycle,
    pub(super) queue: QueueState,
    pub(super) listeners: Vec<Listener>,
}

pub(super) struct Binding {
    pub(super) session: PathBuf,
    pub(super) dir: PathBuf,
}

pub(super) struct Lifecycle {
    pub(super) closed: bool,
}

pub(super) struct QueueState {
    pub(super) manifest: super::manifest::Manifest,
    pub(super) readonly: bool,
    pub(super) unrecovered: bool,
}

impl fmt::Debug for Store {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        formatter
            .debug_struct("Store")
            .field("session", &state.binding.session)
            .field("dir", &state.binding.dir)
            .field("run_id", &self.run_id)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl Store {
    pub fn open(session_path: &Path, limits: Limits) -> Result<Self> {
        let session = clean_session_path(session_path)?;
        let store = Self {
            state: Mutex::new(StoreState {
                binding: Binding {
                    dir: session_inbox_dir(&session),
                    session,
                },
                lifecycle: Lifecycle { closed: false },
                queue: QueueState {
                    manifest: empty_manifest(super::process_run_id()),
                    readonly: false,
                    unrecovered: false,
                },
                listeners: Vec::new(),
            }),
            run_id: super::process_run_id(),
            limits: limits.with_defaults(),
        };
        {
            let mut state = store.lock_state();
            let _transaction = store.begin_disk_transaction_locked(&mut state)?;
        }
        Ok(store)
    }

    pub fn dir(&self) -> PathBuf {
        self.lock_state().binding.dir.clone()
    }

    pub fn session_path(&self) -> PathBuf {
        self.lock_state().binding.session.clone()
    }

    pub fn rebind(&self, session_path: &Path) -> Result<()> {
        let session = clean_session_path(session_path)?;
        let mut state = self.lock_state();
        if state.lifecycle.closed {
            return Err(Error::Closed);
        }
        state.binding.session = session;
        state.binding.dir = session_inbox_dir(&state.binding.session);
        let _transaction = self.begin_disk_transaction_locked(&mut state)?;
        Ok(())
    }

    pub fn close(&self) {
        self.lock_state().lifecycle.closed = true;
    }

    pub fn on_change<F>(&self, listener: F)
    where
        F: Fn(InboxSnapshot) + Send + Sync + 'static,
    {
        self.lock_state().listeners.push(Arc::new(listener));
    }

    pub fn snapshot(&self) -> InboxSnapshot {
        let mut state = self.lock_state();
        let _ = self.begin_disk_transaction_locked(&mut state);
        self.snapshot_locked(&state)
    }

    pub(super) fn begin_disk_transaction_locked(
        &self,
        state: &mut StoreState,
    ) -> Result<TransactionLock> {
        let transaction = acquire_transaction_lock_deadline(
            &state.binding.dir,
            Instant::now() + DISK_LOCK_WAIT,
            DISK_LOCK_POLL,
        )?;
        self.load_or_init_locked(state)?;
        Ok(transaction)
    }

    pub(super) fn notify_locked(&self, state: &StoreState, snapshot: InboxSnapshot) {
        if state.listeners.is_empty() {
            return;
        }
        let listeners = state.listeners.clone();
        std::thread::spawn(move || {
            for listener in listeners {
                listener(snapshot.clone());
            }
        });
    }

    pub(super) fn lock_state(&self) -> MutexGuard<'_, StoreState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub fn remove_dir(session_path: &Path) -> Result<()> {
    remove_inbox_dir(session_path)
}

fn clean_session_path(path: &Path) -> Result<PathBuf> {
    let trimmed = trim_session_path(path.as_os_str());
    if trimmed.is_empty() {
        return Err(Error::EmptySessionPath);
    }
    Ok(PathBuf::from(trimmed))
}

pub(super) fn trim_session_path(path: &OsStr) -> OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let bytes = path.as_bytes();
        let mut start = 0;
        while let Some((ch, len)) = first_utf8_char(bytes, start) {
            if !ch.is_whitespace() {
                break;
            }
            start += len;
        }
        let mut end = bytes.len();
        while let Some((ch, len)) = last_utf8_char(bytes, start, end) {
            if !ch.is_whitespace() {
                break;
            }
            end -= len;
        }
        OsString::from_vec(bytes[start..end].to_vec())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        let units: Vec<u16> = path.encode_wide().collect();
        let mut start = 0;
        while let Some(ch) = char::decode_utf16(units[start..].iter().copied())
            .next()
            .and_then(|result| result.ok())
        {
            if !ch.is_whitespace() {
                break;
            }
            start += ch.len_utf16();
        }
        let mut end = units.len();
        while end > start {
            let Some((ch, len)) = last_utf16_char(&units[start..end]) else {
                break;
            };
            if !ch.is_whitespace() {
                break;
            }
            end -= len;
        }
        OsString::from_wide(&units[start..end])
    }
}

#[cfg(unix)]
fn first_utf8_char(bytes: &[u8], start: usize) -> Option<(char, usize)> {
    for len in 1..=4.min(bytes.len().saturating_sub(start)) {
        let Ok(text) = std::str::from_utf8(&bytes[start..start + len]) else {
            continue;
        };
        let ch = text.chars().next()?;
        if ch.len_utf8() == len {
            return Some((ch, len));
        }
    }
    None
}

#[cfg(unix)]
fn last_utf8_char(bytes: &[u8], start: usize, end: usize) -> Option<(char, usize)> {
    for len in 1..=4.min(end.saturating_sub(start)) {
        let Ok(text) = std::str::from_utf8(&bytes[end - len..end]) else {
            continue;
        };
        let ch = text.chars().next()?;
        if ch.len_utf8() == len {
            return Some((ch, len));
        }
    }
    None
}

#[cfg(windows)]
fn last_utf16_char(units: &[u16]) -> Option<(char, usize)> {
    let last = *units.last()?;
    if units.len() >= 2 {
        let previous = units[units.len() - 2];
        if (0xdc00..=0xdfff).contains(&last) && (0xd800..=0xdbff).contains(&previous) {
            return char::decode_utf16([previous, last].iter().copied())
                .next()
                .and_then(|result| result.ok())
                .map(|ch| (ch, 2));
        }
    }
    char::decode_utf16([last].iter().copied())
        .next()
        .and_then(|result| result.ok())
        .map(|ch| (ch, 1))
}

pub(super) use super::disk_fs::acquire_transaction_lock_deadline;
