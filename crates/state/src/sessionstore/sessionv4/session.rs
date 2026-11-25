use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset, Utc};
use reasonix_contract::provider::Message;
use serde::{Deserialize, Serialize};

use super::content::ContentPool;
use super::error::{Damage, Limit, Unsupported, V4Error};
use super::frames::scan_commits;
use super::fs::{ConfinedRoot, child_name};
use super::message::{Body, MessageFold};
use crate::sessionstore::sessionpreview::session_preview_from_messages;

pub const CATALOG_VERSION: i64 = 5;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    #[serde(rename = "schemaVersion")]
    pub schema_version: i64,
    #[serde(default)]
    pub codec: String,
    #[serde(rename = "storageRevision", default)]
    pub storage_revision: i64,
    #[serde(rename = "contentRoot", default)]
    pub content_root: String,
    #[serde(rename = "sessionId", default)]
    pub session_id: String,
    #[serde(
        rename = "createdAt",
        default = "super::timestamp::zero",
        deserialize_with = "super::timestamp::deserialize",
        serialize_with = "super::timestamp::serialize"
    )]
    pub created_at: DateTime<FixedOffset>,
}

impl Manifest {
    pub fn equal(&self, other: &Self) -> bool {
        self.schema_version == other.schema_version
            && self.codec == other.codec
            && self.storage_revision == other.storage_revision
            && self.content_root == other.content_root
            && self.session_id == other.session_id
            && self.created_at == other.created_at
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Listing {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub preview: String,
    #[serde(default)]
    pub turns: i64,
    #[serde(rename = "modelRef", default)]
    pub model_ref: String,
}

#[derive(Clone, Debug)]
pub struct Session {
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub log_size: i64,
    pub log_mod_time: DateTime<FixedOffset>,
    pub(crate) store: ConfinedRoot,
    pub(crate) name: String,
}

impl Session {
    pub fn updated_at(&self) -> DateTime<FixedOffset> {
        if self.log_mod_time > self.manifest.created_at {
            self.log_mod_time
        } else {
            self.manifest.created_at
        }
    }

    pub fn transcript(&self) -> Result<Transcript, V4Error> {
        let mut output = Transcript::default();
        if self.log_size == 0 {
            return Ok(output);
        }
        let pool = ContentPool::for_session(self);
        let mut fold = MessageFold::default();
        scan_commits(&self.store, &self.log_path(), &pool, |events| {
            for event in events {
                if !matches!(
                    event.kind.as_str(),
                    "message/complete"
                        | "message/upsert"
                        | "message/retract"
                        | "history/replace"
                        | "legacy/import"
                        | "session/title"
                        | "session/config"
                ) {
                    continue;
                }
                let body: Body = serde_json::from_slice(&event.payload)
                    .map_err(|err| V4Error::Damaged(Damage::PayloadJson(err)))?;
                fold.apply(&event.kind, &body, &pool)?;
                match event.kind.as_str() {
                    "session/title" => output.title = body.title.trim().to_string(),
                    "session/config" | "legacy/import" => {
                        let model = body.model_ref.trim();
                        if !model.is_empty() {
                            output.model_ref = model.to_string();
                        }
                    }
                    _ => {}
                }
            }
            Ok(())
        })?;
        output.messages = fold.messages();
        Ok(output)
    }

    pub fn listing(&self) -> Result<Listing, V4Error> {
        if let Some(listing) = self.cached_listing() {
            return Ok(listing);
        }
        let transcript = self.transcript()?;
        let (preview, turns) = session_preview_from_messages(&transcript.messages);
        Ok(Listing {
            title: transcript.title,
            preview,
            turns,
            model_ref: transcript.model_ref,
        })
    }

    pub fn cached_listing(&self) -> Option<Listing> {
        let raw = self.store.read(&self.catalog_path()).ok()?;
        let cached: CatalogMetadata = serde_json::from_slice(&raw).ok()?;
        if cached.version != CATALOG_VERSION
            || cached.codec != super::CODEC
            || cached.session_id != self.manifest.session_id
            || cached.created_at != utc_rfc3339(self.manifest.created_at)
            || cached.log_size != self.log_size
            || cached.log_mod_time_ns != unix_nanos(self.log_mod_time)
        {
            return None;
        }
        Some(Listing {
            title: cached.title.trim().to_string(),
            preview: cached.preview,
            turns: cached.turns,
            model_ref: cached.model_ref,
        })
    }

    pub(crate) fn log_path(&self) -> PathBuf {
        Path::new(&self.name).join("events.frames")
    }

    fn catalog_path(&self) -> PathBuf {
        Path::new(".query-cache")
            .join(&self.manifest.session_id)
            .join("catalog-metadata.json")
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Transcript {
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub title: String,
    #[serde(rename = "modelRef", default)]
    pub model_ref: String,
}

#[derive(Clone, Debug, Deserialize)]
struct CatalogMetadata {
    version: i64,
    codec: String,
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "createdAt")]
    created_at: String,
    title: String,
    #[serde(rename = "modelRef")]
    model_ref: String,
    turns: i64,
    preview: String,
    #[serde(rename = "logSize")]
    log_size: i64,
    #[serde(rename = "logModTimeNs")]
    log_mod_time_ns: i64,
}

#[derive(Clone, Debug)]
struct CacheIdentity {
    manifest: Manifest,
    log_size: i64,
    log_mod_time_ns: i64,
}

impl CacheIdentity {
    fn of(session: &Session) -> Self {
        Self {
            manifest: session.manifest.clone(),
            log_size: session.log_size,
            log_mod_time_ns: unix_nanos(session.log_mod_time),
        }
    }

    fn matches(&self, session: &Session) -> bool {
        self.manifest.equal(&session.manifest)
            && self.log_size == session.log_size
            && self.log_mod_time_ns == unix_nanos(session.log_mod_time)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ListingCache {
    entries: BTreeMap<String, (CacheIdentity, Listing)>,
}

impl ListingCache {
    pub fn put(&mut self, session: &Session, mut listing: Listing) {
        listing.title = listing.title.trim().to_string();
        self.entries.insert(
            session.manifest.session_id.clone(),
            (CacheIdentity::of(session), listing),
        );
    }

    pub fn get(&self, session: &Session) -> Option<Listing> {
        if let Some((identity, listing)) = self.entries.get(&session.manifest.session_id)
            && identity.matches(session)
        {
            return Some(listing.clone());
        }
        session.cached_listing()
    }
}

pub fn open(dir: &Path) -> Result<Session, V4Error> {
    let name = dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(Damage::ManifestPath)?;
    let parent = dir.parent().unwrap_or_else(|| Path::new("."));
    let parent = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };
    let mut session = open_in(parent, name)?;
    session.dir = dir.to_path_buf();
    Ok(session)
}

pub fn open_in(root: &Path, name: &str) -> Result<Session, V4Error> {
    if !child_name(name) {
        return Err(Damage::ManifestPath.into());
    }
    let store = ConfinedRoot::new(root)?;
    let raw = store.read_limited(
        &Path::new(name).join("manifest.json"),
        super::MAX_MANIFEST_BYTES,
    )?;
    if raw.len() > super::MAX_MANIFEST_BYTES {
        return Err(Limit::Manifest(raw.len()).into());
    }
    let manifest: Manifest =
        serde_json::from_slice(&raw).map_err(|err| V4Error::Damaged(Damage::ManifestJson(err)))?;
    if manifest.schema_version != super::SCHEMA_VERSION
        || manifest.codec != super::CODEC
        || !(0..=3).contains(&manifest.storage_revision)
    {
        return Err(Unsupported::Schema {
            schema: manifest.schema_version,
            codec: manifest.codec,
            revision: manifest.storage_revision,
        }
        .into());
    }
    if manifest.session_id != name {
        return Err(Damage::ManifestIdentity.into());
    }
    let (log_size, log_mod_time) = match store.metadata(&Path::new(name).join("events.frames")) {
        Ok(info) => (
            i64::try_from(info.len()).unwrap_or(i64::MAX),
            info.modified()
                .map(|time| DateTime::<Utc>::from(time.into_std()).fixed_offset())
                .unwrap_or_default(),
        ),
        Err(_) => (0, DateTime::<FixedOffset>::default()),
    };
    Ok(Session {
        dir: store.path().join(name),
        manifest,
        log_size,
        log_mod_time,
        store,
        name: name.to_string(),
    })
}

pub fn list(root: &Path) -> Vec<Session> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort();
    names
        .iter()
        .filter_map(|name| open_in(root, name).ok())
        .map(|mut session| {
            session.dir = root.join(&session.name);
            session
        })
        .collect()
}

pub(crate) fn unix_nanos(time: DateTime<FixedOffset>) -> i64 {
    time.timestamp()
        .saturating_mul(1_000_000_000)
        .saturating_add(i64::from(time.timestamp_subsec_nanos()))
}

pub(crate) fn utc_rfc3339(time: DateTime<FixedOffset>) -> String {
    super::timestamp::utc_nano(time)
}
