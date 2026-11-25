use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::{CancellationToken, LegacyListingError, LegacyReplay, ListingCache};
use crate::sessionstore::sessionpreview::session_preview_from_messages;
use crate::sessionstore::sessionv4::{Listing, Session, open};

const BACKOFF: [Duration; 4] = [
    Duration::from_secs(1),
    Duration::from_secs(5),
    Duration::from_secs(30),
    Duration::from_secs(60),
];

#[derive(Debug, Default)]
struct WriteState {
    next: Option<Instant>,
    failures: usize,
    error: Option<LegacyListingError>,
}

#[derive(Debug, Default)]
struct ResolverState {
    cache: Option<ListingCache>,
    write: WriteState,
}

#[derive(Debug)]
pub struct LegacyResolver {
    dir: PathBuf,
    state: Mutex<ResolverState>,
}

impl LegacyResolver {
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            state: Mutex::new(ResolverState::default()),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.dir
    }

    pub fn lookup(&self, source: &Session) -> Option<Listing> {
        let mut state = self.state.lock().unwrap_or_else(|err| err.into_inner());
        let cache = state
            .cache
            .get_or_insert_with(|| ListingCache::load(&self.dir));
        if let Some(listing) = cache.get(source) {
            return Some(listing);
        }
        let listing = source.cached_listing()?;
        cache.put(source, listing.clone());
        Some(listing)
    }

    pub fn resolve(
        &self,
        source: &Session,
        replay: LegacyReplay,
    ) -> Result<Listing, LegacyListingError> {
        self.resolve_with_token(source, replay, &CancellationToken::new())
    }

    pub(crate) fn resolve_with_token(
        &self,
        source: &Session,
        replay: LegacyReplay,
        token: &CancellationToken,
    ) -> Result<Listing, LegacyListingError> {
        if token.is_cancelled() {
            return Err(LegacyListingError::Cancelled);
        }
        let mut state = self.state.lock().unwrap_or_else(|err| err.into_inner());
        if token.is_cancelled() {
            return Err(LegacyListingError::Cancelled);
        }
        let cache = state
            .cache
            .get_or_insert_with(|| ListingCache::load(&self.dir));
        if let Some(listing) = cache.get(source) {
            return Ok(listing);
        }
        if let Some(listing) = source.cached_listing() {
            cache.put(source, listing.clone());
            return Ok(listing);
        }
        let transcript = replay(source, token)?;
        if token.is_cancelled() {
            return Err(LegacyListingError::Cancelled);
        }
        let current = open(&source.dir)?;
        if !same_source(source, &current) {
            return Err(LegacyListingError::SourceChanged);
        }
        let (preview, turns) = session_preview_from_messages(&transcript.messages);
        let listing = Listing {
            title: transcript.title,
            preview,
            turns,
            model_ref: transcript.model_ref,
        };
        state
            .cache
            .as_mut()
            .expect("cache initialized before replay")
            .put(source, listing.clone());
        Ok(listing)
    }

    pub fn flush(&self, dir: &Path, force: bool) -> Result<(), LegacyListingError> {
        let mut state = self.state.lock().unwrap_or_else(|err| err.into_inner());
        if !state.cache.as_ref().is_some_and(|cache| cache.is_dirty()) {
            return Ok(());
        }
        if !force && state.write.next.is_some_and(|next| Instant::now() < next) {
            return Err(state
                .write
                .error
                .clone()
                .unwrap_or(LegacyListingError::Backoff));
        }
        let cache = state.cache.as_mut().expect("dirty cache checked above");
        match cache.save(dir) {
            Ok(()) => {
                state.write = WriteState::default();
                Ok(())
            }
            Err(error) => {
                state.write.failures += 1;
                let delay = BACKOFF[(state.write.failures - 1).min(BACKOFF.len() - 1)];
                state.write.next = Some(Instant::now() + delay);
                state.write.error = Some(error.clone());
                tracing::warn!(directory = %dir.display(), error = %error, ?delay, "legacy listing cache could not be persisted");
                Err(error)
            }
        }
    }
}

pub(crate) fn same_source(left: &Session, right: &Session) -> bool {
    left.manifest.equal(&right.manifest)
        && left.log_size == right.log_size
        && left.log_mod_time == right.log_mod_time
}

pub fn resolver_for(dir: &Path) -> Arc<LegacyResolver> {
    static RESOLVERS: OnceLock<Mutex<HashMap<PathBuf, Arc<LegacyResolver>>>> = OnceLock::new();
    let key = clean_directory(dir);
    let mut resolvers = RESOLVERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    resolvers
        .entry(key.clone())
        .or_insert_with(|| Arc::new(LegacyResolver::new(&key)))
        .clone()
}

pub(crate) fn clean_directory(dir: &Path) -> PathBuf {
    if dir.as_os_str().is_empty() {
        return PathBuf::from(".");
    }
    let mut clean = PathBuf::new();
    for component in dir.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !clean.as_os_str().is_empty()
                    && clean.file_name().is_some_and(|name| name != "..")
                {
                    clean.pop();
                } else {
                    clean.push("..");
                }
            }
            other => clean.push(other.as_os_str()),
        }
    }
    if clean.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        clean
    }
}
