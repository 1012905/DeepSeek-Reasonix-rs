use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use reasonix_core::filelock::try_acquire;
use reasonix_core::fileutil::{atomic_write_file_strict_validated, mkdir_all_mode};
use serde::{Deserialize, Serialize};

use super::LegacyListingError;
use crate::sessionstore::sessionv4::{Listing, Manifest, Session, open};
use crate::store::naming::v4_sessions_root;

pub const CACHE_NAME: &str = ".v4-listing.json";
pub const CACHE_FORMAT: i64 = 2;

fn unix_nanos(time: chrono::DateTime<chrono::FixedOffset>) -> i64 {
    time.timestamp()
        .saturating_mul(1_000_000_000)
        .saturating_add(i64::from(time.timestamp_subsec_nanos()))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct CacheEntry {
    identity: Manifest,
    #[serde(rename = "s")]
    log_size: i64,
    #[serde(rename = "m")]
    log_mod: i64,
    #[serde(rename = "l")]
    listing: Listing,
}

impl CacheEntry {
    fn matches(&self, session: &Session) -> bool {
        self.identity.equal(&session.manifest)
            && self.log_size == session.log_size
            && self.log_mod == unix_nanos(session.log_mod_time)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ListingCache {
    format: i64,
    pub(crate) entries: BTreeMap<String, CacheEntry>,
    #[serde(skip)]
    pub(crate) dirty: bool,
    #[serde(skip)]
    changed: BTreeMap<String, CacheEntry>,
}

impl Default for ListingCache {
    fn default() -> Self {
        Self {
            format: CACHE_FORMAT,
            entries: BTreeMap::new(),
            dirty: false,
            changed: BTreeMap::new(),
        }
    }
}

impl ListingCache {
    pub fn load(dir: &Path) -> Self {
        let Ok(raw) = fs::read(dir.join(CACHE_NAME)) else {
            return Self::default();
        };
        let Ok(cache) = serde_json::from_slice::<Self>(&raw) else {
            return Self::default();
        };
        if cache.format != CACHE_FORMAT {
            return Self::default();
        }
        Self {
            format: CACHE_FORMAT,
            entries: cache.entries,
            dirty: false,
            changed: BTreeMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn get(&self, session: &Session) -> Option<Listing> {
        self.entries
            .get(&session.manifest.session_id)
            .filter(|entry| entry.matches(session))
            .map(|entry| entry.listing.clone())
    }

    pub fn put(&mut self, session: &Session, listing: Listing) {
        let entry = CacheEntry {
            identity: session.manifest.clone(),
            log_size: session.log_size,
            log_mod: unix_nanos(session.log_mod_time),
            listing,
        };
        self.entries
            .insert(session.manifest.session_id.clone(), entry.clone());
        self.changed
            .insert(session.manifest.session_id.clone(), entry);
        self.dirty = true;
    }

    pub fn save(&mut self, dir: &Path) -> Result<(), LegacyListingError> {
        self.save_with_token(dir, &super::CancellationToken::new())
    }

    pub(crate) fn save_with_token(
        &mut self,
        dir: &Path,
        token: &super::CancellationToken,
    ) -> Result<(), LegacyListingError> {
        if token.is_cancelled() {
            return Err(LegacyListingError::Cancelled);
        }
        if !self.dirty {
            return Ok(());
        }
        mkdir_all_mode(dir, 0o755).map_err(|source| LegacyListingError::Io {
            path: dir.to_path_buf(),
            source: source.into(),
        })?;
        let lock_path = dir.join(format!("{CACHE_NAME}.lock"));
        let _lock = try_acquire(&lock_path)?;
        let mut merged = Self::load(dir);
        let root = v4_sessions_root(dir);
        for (id, entry) in &self.changed {
            if token.is_cancelled() {
                return Err(LegacyListingError::Cancelled);
            }
            if root.as_os_str().is_empty() {
                continue;
            }
            let Ok(source) = open(&root.join(id)) else {
                continue;
            };
            if entry.matches(&source) {
                merged.entries.insert(id.clone(), entry.clone());
            }
        }
        if token.is_cancelled() {
            return Err(LegacyListingError::Cancelled);
        }
        let raw = serde_json::to_vec(&merged)?;
        let target = dir.join(CACHE_NAME);
        let publication_cancelled = Cell::new(false);
        let validate = || {
            if token.is_cancelled() {
                publication_cancelled.set(true);
                Err(io::Error::from(io::ErrorKind::Interrupted))
            } else {
                Ok(())
            }
        };
        if let Err(source) =
            atomic_write_file_strict_validated(&target, &raw, 0o600, Some(&validate))
        {
            if publication_cancelled.get() {
                return Err(LegacyListingError::Cancelled);
            }
            return Err(LegacyListingError::Io {
                path: target,
                source: source.into(),
            });
        }
        self.entries.extend(merged.entries);
        self.changed.clear();
        self.dirty = false;
        Ok(())
    }
}
