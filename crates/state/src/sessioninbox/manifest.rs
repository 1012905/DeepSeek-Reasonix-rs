//! Manifest decoding, validation, and bookkeeping without disk ownership.

use std::collections::{BTreeMap, BTreeSet};

use chrono::Duration;
use reasonix_contract::go_struct;
use reasonix_contract::gojson::{go_vec, is_zero_i64, null_default};

use crate::sessionstore::eventlog::record::{CreatedAt, deserialize_created_at};

use super::types::{
    Disposition, Error, InboxItemMeta, PauseCode, SCHEMA_VERSION, go_map, valid_inbox_intent,
    valid_inbox_state,
};

pub const MAX_IDEMPOTENCY_RECEIPTS: usize = 512;
pub const IDEMPOTENCY_RECEIPT_TTL: Duration = Duration::hours(7 * 24);
pub const MAX_IDEMPOTENCY_KEY_BYTES: usize = 1024;

go_struct! {
    #[derive(PartialEq)]
    pub struct IdempotencyReceipt : IdempotencyReceiptWire {
        #[serde(rename = "itemId", default, deserialize_with = "null_default")]
        pub item_id: String,
        #[serde(rename = "requestHash", default, deserialize_with = "null_default")]
        pub request_hash: String,
        #[serde(rename = "disposition", default, deserialize_with = "null_default")]
        pub disposition: Disposition,
        #[serde(rename = "completedAt", default, deserialize_with = "deserialize_created_at")]
        pub completed_at: CreatedAt,
    }
}

go_struct! {
    #[derive(PartialEq)]
    pub struct Manifest : ManifestWire {
        #[serde(rename = "schemaVersion", default, deserialize_with = "null_default")]
        pub schema_version: i64,
        #[serde(rename = "revision", default, deserialize_with = "null_default")]
        pub revision: i64,
        #[serde(rename = "runId", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub run_id: String,
        #[serde(rename = "paused", default, deserialize_with = "null_default")]
        pub paused: bool,
        #[serde(rename = "pauseCode", default, deserialize_with = "null_default", skip_serializing_if = "PauseCode::is_empty")]
        pub pause_code: PauseCode,
        #[serde(rename = "recovered", default, deserialize_with = "null_default")]
        pub recovered: bool,
        #[serde(rename = "recoveredCount", default, deserialize_with = "null_default", skip_serializing_if = "is_zero_i64")]
        pub recovered_n: i64,
        #[serde(rename = "items", default, deserialize_with = "go_vec")]
        pub items: Vec<InboxItemMeta>,
        #[serde(rename = "idempotency", default, deserialize_with = "go_map", skip_serializing_if = "BTreeMap::is_empty")]
        pub idempotency: BTreeMap<String, String>,
        #[serde(rename = "idempotencyHashes", default, deserialize_with = "go_map", skip_serializing_if = "BTreeMap::is_empty")]
        pub idempotency_hashes: BTreeMap<String, String>,
        #[serde(rename = "receipts", default, deserialize_with = "go_map", skip_serializing_if = "BTreeMap::is_empty")]
        pub receipts: BTreeMap<String, IdempotencyReceipt>,
        #[serde(rename = "updatedAt", default, deserialize_with = "deserialize_created_at")]
        pub updated_at: CreatedAt,
    }
}

pub fn empty_manifest(run_id: impl Into<String>) -> Manifest {
    Manifest {
        schema_version: SCHEMA_VERSION,
        run_id: run_id.into(),
        items: Vec::new(),
        idempotency: BTreeMap::new(),
        idempotency_hashes: BTreeMap::new(),
        receipts: BTreeMap::new(),
        updated_at: CreatedAt::now(),
        ..Manifest::default()
    }
}

pub fn decode_manifest(data: &[u8]) -> Result<Manifest, Error> {
    let manifest = serde_json::from_slice::<Option<Manifest>>(data)?.unwrap_or_default();
    validate_manifest(&manifest, manifest.schema_version > SCHEMA_VERSION)?;
    Ok(manifest)
}

pub fn validate_manifest(manifest: &Manifest, allow_unknown_enums: bool) -> Result<(), Error> {
    if manifest.schema_version < 0 || manifest.revision < 0 {
        return Err(Error::InvalidVersionOrRevision);
    }
    let mut ids = BTreeSet::new();
    for item in &manifest.items {
        if !valid_blob_stem(&item.id) || !valid_blob_stem(blob_name_for(item)) {
            return Err(Error::InvalidItemPath);
        }
        if !ids.insert(item.id.as_str()) {
            return Err(Error::DuplicateItemId);
        }
        if item.byte_size < 0 {
            return Err(Error::NegativeItemSize);
        }
        if !allow_unknown_enums
            && (!valid_inbox_intent(&item.intent) || !valid_inbox_state(&item.state))
        {
            return Err(Error::InvalidItemIntentOrState);
        }
        if !item.checksum.is_empty() && !valid_sha256(&item.checksum) {
            return Err(Error::InvalidItemChecksum);
        }
    }
    if allow_unknown_enums {
        return Ok(());
    }
    for (key, item_id) in &manifest.idempotency {
        if !valid_idempotency_key(key) {
            return Err(Error::InvalidIdempotencyKey);
        }
        if !ids.contains(item_id.as_str()) {
            return Err(Error::IdempotencyMissingItem);
        }
        let hash = manifest
            .idempotency_hashes
            .get(key)
            .map_or("", String::as_str);
        if hash.is_empty() {
            if manifest.schema_version >= 2 {
                return Err(Error::MissingIdempotencyRequestHash);
            }
        } else if !valid_sha256(hash) {
            return Err(Error::InvalidIdempotencyRequestHash);
        }
    }
    for (key, receipt) in &manifest.receipts {
        if !valid_idempotency_key(key)
            || !valid_blob_stem(&receipt.item_id)
            || !valid_sha256(&receipt.request_hash)
        {
            return Err(Error::InvalidIdempotencyReceipt);
        }
    }
    Ok(())
}

pub fn blob_name_for(item: &InboxItemMeta) -> &str {
    let name = item.blob_name.trim();
    if name.is_empty() { &item.id } else { name }
}

pub fn valid_blob_stem(name: &str) -> bool {
    if name.is_empty()
        || name.trim() != name
        || name.len() > 200
        || matches!(name, "." | "..")
        || name.contains(['/', '\\', '\0'])
    {
        return false;
    }
    #[cfg(windows)]
    {
        is_local_windows_name(name)
    }
    #[cfg(not(windows))]
    {
        true
    }
}

pub fn valid_idempotency_key(key: &str) -> bool {
    !key.is_empty()
        && key.trim() == key
        && key.len() <= MAX_IDEMPOTENCY_KEY_BYTES
        && !key.contains('\0')
}

pub fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl Manifest {
    pub fn total_bytes(&self) -> i64 {
        self.items
            .iter()
            .fold(0i64, |total, item| total.wrapping_add(item.byte_size))
    }

    pub fn index_of(&self, id: &str) -> i64 {
        self.items
            .iter()
            .position(|item| item.id == id)
            .and_then(|index| i64::try_from(index).ok())
            .unwrap_or(-1)
    }

    pub fn item(&self, id: &str) -> Option<InboxItemMeta> {
        self.items.iter().find(|item| item.id == id).cloned()
    }

    pub fn remove_item(&mut self, id: &str) -> Option<InboxItemMeta> {
        let index = self.items.iter().position(|item| item.id == id)?;
        let item = self.items.remove(index);
        let keys: Vec<String> = self
            .idempotency
            .iter()
            .filter(|(_, item_id)| item_id.as_str() == id)
            .map(|(key, _)| key.clone())
            .collect();
        for key in keys {
            self.idempotency.remove(&key);
            self.idempotency_hashes.remove(&key);
        }
        Some(item)
    }

    pub fn idempotency_keys_for(&self, id: &str) -> Vec<String> {
        self.idempotency
            .iter()
            .filter(|(_, item_id)| item_id.as_str() == id)
            .map(|(key, _)| key.clone())
            .collect()
    }

    pub fn remember_receipt(
        &mut self,
        keys: &[String],
        item_id: &str,
        disposition: Disposition,
        now: CreatedAt,
    ) {
        for key in keys {
            if key.trim().is_empty() {
                continue;
            }
            self.receipts.insert(
                key.clone(),
                IdempotencyReceipt {
                    item_id: item_id.to_owned(),
                    request_hash: self
                        .idempotency_hashes
                        .get(key)
                        .cloned()
                        .unwrap_or_default(),
                    disposition: disposition.clone(),
                    completed_at: now,
                },
            );
        }
        self.prune_receipts(now);
    }

    pub fn prune_receipts(&mut self, now: CreatedAt) {
        self.receipts.retain(|_, receipt| {
            !receipt.completed_at.is_zero()
                && now.get().signed_duration_since(receipt.completed_at.get())
                    <= IDEMPOTENCY_RECEIPT_TTL
        });
        let excess = self.receipts.len().saturating_sub(MAX_IDEMPOTENCY_RECEIPTS);
        if excess == 0 {
            return;
        }
        let mut ordered: Vec<(String, CreatedAt)> = self
            .receipts
            .iter()
            .map(|(key, receipt)| (key.clone(), receipt.completed_at))
            .collect();
        ordered.sort_by(|left, right| {
            left.1
                .get()
                .cmp(&right.1.get())
                .then_with(|| left.0.cmp(&right.0))
        });
        for (key, _) in ordered.into_iter().take(excess) {
            self.receipts.remove(&key);
        }
    }
}

#[cfg(windows)]
fn is_local_windows_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(['/', '\\'])
        && !name.contains(':')
        && !is_reserved_windows_name(name)
}

#[cfg(windows)]
fn is_reserved_windows_name(name: &str) -> bool {
    let end = name
        .as_bytes()
        .iter()
        .position(|byte| matches!(byte, b'.' | b':'))
        .unwrap_or(name.len());
    let base = name[..end].trim_end_matches(' ');
    if !is_reserved_windows_base(base) {
        return false;
    }
    if base.len() == name.len() {
        return true;
    }
    is_reserved_windows_base(name.trim_end_matches(['.', ' ']))
}

#[cfg(windows)]
fn is_reserved_windows_base(name: &str) -> bool {
    if name.len() == 3
        && ["CON", "PRN", "AUX", "NUL"]
            .iter()
            .any(|reserved| name.eq_ignore_ascii_case(reserved))
    {
        return true;
    }
    if name.len() >= 4
        && ["COM", "LPT"].iter().any(|prefix| {
            name.get(..3)
                .is_some_and(|value| value.eq_ignore_ascii_case(prefix))
        })
    {
        let suffix = &name[3..];
        return matches!(
            suffix,
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
        );
    }
    name.eq_ignore_ascii_case("CONIN$") || name.eq_ignore_ascii_case("CONOUT$")
}
