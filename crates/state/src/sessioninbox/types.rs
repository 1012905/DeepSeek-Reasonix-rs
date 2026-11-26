//! Session-inbox wire types and Go-compatible JSON decoding.

use std::borrow::Cow;
use std::collections::BTreeMap;

use base64::Engine;
use base64::alphabet;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig, STANDARD};
use reasonix_contract::go_struct;
use reasonix_contract::gojson::{
    go_opt_vec, go_raw, go_vec, is_zero_bool, is_zero_i64, null_default,
};
use reasonix_contract::provider::Via;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::value::RawValue;

use crate::sessionstore::eventlog::record::{CreatedAt, deserialize_created_at};

pub const SCHEMA_VERSION: i64 = 3;
pub const DEFAULT_MAX_ITEMS: i64 = 64;
pub const DEFAULT_MAX_ITEM_BYTES: i64 = 4 << 20;
pub const DEFAULT_MAX_TOTAL_BYTES: i64 = 64 << 20;
pub const DEFAULT_PREVIEW_RUNES: i64 = 120;
pub const NO_ITEM_LIMIT: i64 = -1;

macro_rules! wire_string {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(Cow<'static, str>);

        impl $name {
            pub const fn literal(value: &'static str) -> Self {
                Self(Cow::Borrowed(value))
            }

            pub fn new(value: impl Into<String>) -> Self {
                Self(Cow::Owned(value.into()))
            }

            pub fn as_str(&self) -> &str {
                self.0.as_ref()
            }

            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self(Cow::Borrowed(""))
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Ok(Self(Cow::Owned(
                    Option::<String>::deserialize(deserializer)?.unwrap_or_default(),
                )))
            }
        }
    };
}

wire_string!(InboxIntent);
wire_string!(PromptOrigin);
wire_string!(InboxState);
wire_string!(Disposition);
wire_string!(PauseCode);
wire_string!(BlockCode);

pub const INTENT_FOLLOWUP: InboxIntent = InboxIntent::literal("followup");
pub const INTENT_STEER: InboxIntent = InboxIntent::literal("steer");
pub const ORIGIN_USER: PromptOrigin = PromptOrigin::literal("");
pub const ORIGIN_HOST: PromptOrigin = PromptOrigin::literal("host");
pub const STATE_QUEUED: InboxState = InboxState::literal("queued");
pub const STATE_STEER_ACCEPTED: InboxState = InboxState::literal("steer_accepted");
pub const STATE_STEER_CONSUMED: InboxState = InboxState::literal("steer_consumed");
pub const STATE_RUNNING: InboxState = InboxState::literal("running");
pub const STATE_BLOCKED: InboxState = InboxState::literal("blocked");
pub const STATE_UNCERTAIN: InboxState = InboxState::literal("uncertain");
pub const DISPOSITION_STARTED: Disposition = Disposition::literal("started");
pub const DISPOSITION_STEER_ACCEPTED: Disposition = Disposition::literal("steer_accepted");
pub const DISPOSITION_QUEUED_FOLLOWUP: Disposition = Disposition::literal("queued_followup");
pub const DISPOSITION_REJECTED_BUSY: Disposition = Disposition::literal("rejected_busy");
pub const DISPOSITION_REJECTED_ROTATING: Disposition = Disposition::literal("rejected_rotating");
pub const DISPOSITION_REJECTED_CLOSED: Disposition = Disposition::literal("rejected_closed");
pub const DISPOSITION_REJECTED_CAPACITY: Disposition = Disposition::literal("rejected_capacity");
pub const DISPOSITION_IDEMPOTENT_HIT: Disposition = Disposition::literal("idempotent_hit");
pub const DISPOSITION_DELETED: Disposition = Disposition::literal("deleted");
pub const DISPOSITION_CANCELLED: Disposition = Disposition::literal("cancelled");
pub const PAUSE_USER: PauseCode = PauseCode::literal("user");
pub const PAUSE_SESSION_CHANGED: PauseCode = PauseCode::literal("session_changed");
pub const PAUSE_REFERENCE_BLOCKED: PauseCode = PauseCode::literal("reference_blocked");
pub const PAUSE_WRITE_AUTHORITY: PauseCode = PauseCode::literal("write_authority");
pub const PAUSE_TURN_RECORD: PauseCode = PauseCode::literal("turn_record");
pub const PAUSE_RECOVERED: PauseCode = PauseCode::literal("recovered");
pub const BLOCK_STEER_UNAPPLIED: BlockCode = BlockCode::literal("steer_unapplied");
pub const BLOCK_SNAPSHOT_FAILED: BlockCode = BlockCode::literal("turn_snapshot_failed");
pub const BLOCK_ACK_FAILED: BlockCode = BlockCode::literal("turn_ack_failed");
pub const BLOCK_OWNER_INACTIVE: BlockCode = BlockCode::literal("owner_inactive");
pub const BLOCK_MANIFEST_SALVAGED: BlockCode = BlockCode::literal("manifest_salvaged");
pub const BLOCK_FOREIGN_IMAGES: BlockCode = BlockCode::literal("foreign_images");

impl PromptOrigin {
    pub fn is_host(&self) -> bool {
        self.as_str() == ORIGIN_HOST.as_str()
    }
}

impl PauseCode {
    pub fn guards_stale_input(&self) -> bool {
        self == &PAUSE_SESSION_CHANGED
            || self == &PAUSE_REFERENCE_BLOCKED
            || self == &PAUSE_RECOVERED
    }
}

impl BlockCode {
    pub fn retry_safe(&self) -> bool {
        self.as_str() == BLOCK_STEER_UNAPPLIED.as_str()
    }
}

pub fn valid_inbox_intent(intent: &InboxIntent) -> bool {
    intent == &INTENT_FOLLOWUP || intent == &INTENT_STEER
}

pub fn valid_inbox_state(state: &InboxState) -> bool {
    state == &STATE_QUEUED
        || state == &STATE_STEER_ACCEPTED
        || state == &STATE_STEER_CONSUMED
        || state == &STATE_RUNNING
        || state == &STATE_BLOCKED
        || state == &STATE_UNCERTAIN
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("session inbox item limit reached")]
    CapacityItems,
    #[error("session inbox byte limit reached")]
    CapacityBytes,
    #[error("inbox item exceeds single-item size limit")]
    ItemTooLarge,
    #[error("inbox item not found")]
    NotFound,
    #[error("inbox item state does not allow this operation")]
    InvalidState,
    #[error("inbox item may already have run, so retrying it could repeat it")]
    RetryMayRepeat,
    #[error("inbox schema is newer and is read-only")]
    SchemaReadonly,
    #[error("inbox is closed")]
    Closed,
    #[error("inbox item body is empty")]
    Empty,
    #[error("inbox is paused")]
    Paused,
    #[error("sessioninbox.foreign_run")]
    ForeignRun,
    #[error("idempotency key was already used for different input")]
    IdempotencyConflict,
    #[error("sessioninbox.queue_unreadable")]
    Unrecovered,
    #[error("nil manifest")]
    NilManifest,
    #[error("invalid manifest version or revision")]
    InvalidVersionOrRevision,
    #[error("invalid inbox item path")]
    InvalidItemPath,
    #[error("duplicate inbox item id")]
    DuplicateItemId,
    #[error("negative inbox item size")]
    NegativeItemSize,
    #[error("invalid inbox item intent or state")]
    InvalidItemIntentOrState,
    #[error("invalid inbox item checksum")]
    InvalidItemChecksum,
    #[error("invalid idempotency key")]
    InvalidIdempotencyKey,
    #[error("idempotency key references missing item")]
    IdempotencyMissingItem,
    #[error("missing idempotency request hash")]
    MissingIdempotencyRequestHash,
    #[error("invalid idempotency request hash")]
    InvalidIdempotencyRequestHash,
    #[error("invalid idempotency receipt")]
    InvalidIdempotencyReceipt,
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub(crate) fn go_map<'de, D, T>(de: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<BTreeMap<String, Option<T>>>::deserialize(de)?
        .unwrap_or_default()
        .into_iter()
        .map(|(key, value)| (key, value.unwrap_or_default()))
        .collect())
}

const GO_BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
);

fn serialize_bytes<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&STANDARD.encode(bytes))
}

fn deserialize_bytes<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Bytes {
        Base64(String),
        Array(Vec<Option<u8>>),
    }

    match Option::<Bytes>::deserialize(deserializer)? {
        None => Ok(Vec::new()),
        Some(Bytes::Array(values)) => Ok(values
            .into_iter()
            .map(|value| value.unwrap_or_default())
            .collect()),
        Some(Bytes::Base64(value)) => {
            let value: String = value
                .chars()
                .filter(|ch| !matches!(ch, '\r' | '\n'))
                .collect();
            GO_BASE64.decode(value.as_bytes()).map_err(D::Error::custom)
        }
    }
}

fn compact_raw_json(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut in_string = false;
    let mut escaped = false;
    for ch in input.chars() {
        if in_string {
            output.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => {
                in_string = true;
                output.push(ch);
            }
            ' ' | '\t' | '\r' | '\n' => {}
            _ => output.push(ch),
        }
    }
    output
}

fn serialize_raw_message<S>(value: &Option<Box<RawValue>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let Some(value) = value else {
        return serializer.serialize_none();
    };
    let compact = compact_raw_json(value.get());
    let raw = RawValue::from_string(compact).map_err(serde::ser::Error::custom)?;
    raw.serialize(serializer)
}

go_struct! {
    #[derive(PartialEq)]
    pub struct RefSummary : RefSummaryWire {
        #[serde(rename = "kind", default, deserialize_with = "null_default")]
        pub kind: String,
        #[serde(rename = "path", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub path: String,
        #[serde(rename = "commit", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub commit: String,
        #[serde(rename = "bytes", default, deserialize_with = "null_default", skip_serializing_if = "is_zero_i64")]
        pub bytes: i64,
        #[serde(rename = "preview", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub preview: String,
    }
}

go_struct! {
    #[derive(PartialEq)]
    pub struct RefSnapshot : RefSnapshotWire {
        #[serde(rename = "kind", default, deserialize_with = "null_default")]
        pub kind: String,
        #[serde(rename = "path", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub path: String,
        #[serde(rename = "displayPath", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub display_path: String,
        #[serde(rename = "repoIdentity", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub repo_identity: String,
        #[serde(rename = "commit", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub commit: String,
        #[serde(rename = "rangeStart", default, deserialize_with = "null_default", skip_serializing_if = "is_zero_i64")]
        pub range_start: i64,
        #[serde(rename = "rangeEnd", default, deserialize_with = "null_default", skip_serializing_if = "is_zero_i64")]
        pub range_end: i64,
        #[serde(rename = "content", default, deserialize_with = "deserialize_bytes", serialize_with = "serialize_bytes", skip_serializing_if = "Vec::is_empty")]
        pub content: Vec<u8>,
        #[serde(rename = "contentSha", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub content_sha: String,
        #[serde(rename = "truncated", default, deserialize_with = "null_default", skip_serializing_if = "is_zero_bool")]
        pub truncated: bool,
        #[serde(rename = "server", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub server: String,
        #[serde(rename = "uri", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub uri: String,
    }
}

go_struct! {
    #[derive(PartialEq)]
    pub struct StructuredInvocation : StructuredInvocationWire {
        #[serde(rename = "name", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub name: String,
        #[serde(rename = "kind", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub kind: String,
        #[serde(rename = "offset", default, deserialize_with = "null_default", skip_serializing_if = "is_zero_i64")]
        pub offset: i64,
        #[serde(rename = "args", default, deserialize_with = "go_map", skip_serializing_if = "BTreeMap::is_empty")]
        pub args: BTreeMap<String, String>,
        #[serde(rename = "display", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub display: String,
    }
}

go_struct! {
    pub struct PromptEnvelope : PromptEnvelopeWire {
        #[serde(rename = "fingerprintVersion", default, deserialize_with = "null_default", skip_serializing_if = "is_zero_i64")]
        pub fingerprint_version: i64,
        #[serde(rename = "requestFingerprint", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub request_fingerprint: String,
        #[serde(rename = "displayText", default, deserialize_with = "null_default")]
        pub display_text: String,
        #[serde(rename = "rawText", default, deserialize_with = "null_default")]
        pub raw_text: String,
        #[serde(rename = "submitText", default, deserialize_with = "null_default")]
        pub submit_text: String,
        #[serde(rename = "invocation", default, deserialize_with = "null_default", skip_serializing_if = "Option::is_none")]
        pub invocation: Option<StructuredInvocation>,
        #[serde(rename = "invocations", default, deserialize_with = "go_vec", skip_serializing_if = "Vec::is_empty")]
        pub invocations: Vec<StructuredInvocation>,
        #[serde(rename = "format", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub format: String,
        #[serde(rename = "attachments", default, deserialize_with = "go_vec", skip_serializing_if = "Vec::is_empty")]
        pub attachments: Vec<String>,
        #[serde(rename = "attachmentIdentities", default, deserialize_with = "go_vec", skip_serializing_if = "Vec::is_empty")]
        pub attachment_identities: Vec<String>,
        #[serde(rename = "refs", default, deserialize_with = "go_vec", skip_serializing_if = "Vec::is_empty")]
        pub refs: Vec<RefSnapshot>,
        #[serde(rename = "frozenRefBlock", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub frozen_ref_block: String,
        #[serde(rename = "frozenImages", default, deserialize_with = "go_vec", skip_serializing_if = "Vec::is_empty")]
        pub frozen_images: Vec<String>,
        #[serde(rename = "imageInputs", default, deserialize_with = "go_raw", serialize_with = "serialize_raw_message", skip_serializing_if = "Option::is_none")]
        pub image_inputs: Option<Box<RawValue>>,
        #[serde(rename = "imageSourceRefs", default, deserialize_with = "go_map", skip_serializing_if = "BTreeMap::is_empty")]
        pub image_source_refs: BTreeMap<String, String>,
        #[serde(rename = "referenceErrors", default, deserialize_with = "go_vec", skip_serializing_if = "Vec::is_empty")]
        pub reference_errors: Vec<String>,
        #[serde(rename = "explicitRefs", default, deserialize_with = "go_vec", skip_serializing_if = "Vec::is_empty")]
        pub explicit_refs: Vec<String>,
        #[serde(rename = "idempotencyKey", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub idempotency: String,
        #[serde(rename = "source", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub source: String,
        #[serde(rename = "extra", default, deserialize_with = "go_map", skip_serializing_if = "BTreeMap::is_empty")]
        pub extra: BTreeMap<String, String>,
        #[serde(rename = "via", default, deserialize_with = "null_default", skip_serializing_if = "Option::is_none")]
        pub via: Option<Via>,
    }
}

impl PartialEq for PromptEnvelope {
    fn eq(&self, other: &Self) -> bool {
        self.fingerprint_version == other.fingerprint_version
            && self.request_fingerprint == other.request_fingerprint
            && self.display_text == other.display_text
            && self.raw_text == other.raw_text
            && self.submit_text == other.submit_text
            && self.invocation == other.invocation
            && self.invocations == other.invocations
            && self.format == other.format
            && self.attachments == other.attachments
            && self.attachment_identities == other.attachment_identities
            && self.refs == other.refs
            && self.frozen_ref_block == other.frozen_ref_block
            && self.frozen_images == other.frozen_images
            && self.image_inputs.as_deref().map(RawValue::get)
                == other.image_inputs.as_deref().map(RawValue::get)
            && self.image_source_refs == other.image_source_refs
            && self.reference_errors == other.reference_errors
            && self.explicit_refs == other.explicit_refs
            && self.idempotency == other.idempotency
            && self.source == other.source
            && self.extra == other.extra
            && self.via == other.via
    }
}

impl PromptEnvelope {
    pub fn carries_images(&self) -> bool {
        !self.frozen_images.is_empty() || self.has_foreign_images()
    }

    pub fn has_foreign_images(&self) -> bool {
        let Some(inputs) = &self.image_inputs else {
            return false;
        };
        serde_json::from_str::<Option<Vec<Box<RawValue>>>>(inputs.get())
            .map(|inputs| inputs.is_some_and(|inputs| !inputs.is_empty()))
            .unwrap_or(true)
    }
}

go_struct! {
    #[derive(PartialEq)]
    pub struct InboxItemMeta : InboxItemMetaWire {
        #[serde(rename = "id", default, deserialize_with = "null_default")]
        pub id: String,
        #[serde(rename = "sessionId", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub session_id: String,
        #[serde(rename = "intent", default, deserialize_with = "null_default")]
        pub intent: InboxIntent,
        #[serde(rename = "state", default, deserialize_with = "null_default")]
        pub state: InboxState,
        #[serde(rename = "revision", default, deserialize_with = "null_default")]
        pub revision: i64,
        #[serde(rename = "blobName", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub blob_name: String,
        #[serde(rename = "source", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub source: String,
        #[serde(rename = "createdAt", default, deserialize_with = "deserialize_created_at")]
        pub created_at: CreatedAt,
        #[serde(rename = "updatedAt", default, deserialize_with = "deserialize_created_at")]
        pub updated_at: CreatedAt,
        #[serde(rename = "preview", default, deserialize_with = "null_default")]
        pub preview: String,
        #[serde(rename = "byteSize", default, deserialize_with = "null_default")]
        pub byte_size: i64,
        #[serde(rename = "checksum", default, deserialize_with = "null_default")]
        pub checksum: String,
        #[serde(rename = "idempotencyKey", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub idempotency: String,
        #[serde(rename = "origin", default, deserialize_with = "null_default", skip_serializing_if = "PromptOrigin::is_empty")]
        pub origin: PromptOrigin,
        #[serde(rename = "refs", default, deserialize_with = "go_vec", skip_serializing_if = "Vec::is_empty")]
        pub refs: Vec<RefSummary>,
        #[serde(rename = "images", default, deserialize_with = "null_default", skip_serializing_if = "is_zero_bool")]
        pub images: bool,
        #[serde(rename = "blockReason", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub block_reason: String,
        #[serde(rename = "blockCode", default, deserialize_with = "null_default", skip_serializing_if = "BlockCode::is_empty")]
        pub block_code: BlockCode,
        #[serde(rename = "runId", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub run_id: String,
    }
}

go_struct! {
    #[derive(PartialEq, Copy)]
    pub struct Capacity : CapacityWire {
        #[serde(rename = "items", default, deserialize_with = "null_default")]
        pub items: i64,
        #[serde(rename = "maxItems", default, deserialize_with = "null_default")]
        pub max_items: i64,
        #[serde(rename = "bytes", default, deserialize_with = "null_default")]
        pub bytes: i64,
        #[serde(rename = "maxBytes", default, deserialize_with = "null_default")]
        pub max_bytes: i64,
        #[serde(rename = "maxItemBytes", default, deserialize_with = "null_default")]
        pub max_item_bytes: i64,
    }
}

go_struct! {
    #[derive(PartialEq)]
    pub struct InboxSnapshot : InboxSnapshotWire {
        #[serde(rename = "schemaVersion", default, deserialize_with = "null_default")]
        pub schema_version: i64,
        #[serde(rename = "revision", default, deserialize_with = "null_default")]
        pub revision: i64,
        #[serde(rename = "paused", default, deserialize_with = "null_default")]
        pub paused: bool,
        #[serde(rename = "pauseCode", default, deserialize_with = "null_default", skip_serializing_if = "PauseCode::is_empty")]
        pub pause_code: PauseCode,
        #[serde(rename = "recovered", default, deserialize_with = "null_default")]
        pub recovered: bool,
        #[serde(rename = "recoveredCount", default, deserialize_with = "null_default", skip_serializing_if = "is_zero_i64")]
        pub recovered_n: i64,
        #[serde(rename = "unrecovered", default, deserialize_with = "null_default", skip_serializing_if = "is_zero_bool")]
        pub unrecovered: bool,
        #[serde(rename = "readonly", default, deserialize_with = "null_default", skip_serializing_if = "is_zero_bool")]
        pub readonly: bool,
        #[serde(rename = "runId", default, deserialize_with = "null_default", skip_serializing_if = "String::is_empty")]
        pub run_id: String,
        #[serde(rename = "items", default, deserialize_with = "go_opt_vec")]
        pub items: Option<Vec<InboxItemMeta>>,
        #[serde(rename = "capacity", default, deserialize_with = "null_default")]
        pub capacity: Capacity,
    }
}

go_struct! {
    #[derive(PartialEq)]
    pub struct InboxReceipt : InboxReceiptWire {
        #[serde(rename = "itemId", default, deserialize_with = "null_default")]
        pub item_id: String,
        #[serde(rename = "disposition", default, deserialize_with = "null_default")]
        pub disposition: Disposition,
        #[serde(rename = "position", default, deserialize_with = "null_default")]
        pub position: i64,
        #[serde(rename = "paused", default, deserialize_with = "null_default")]
        pub paused: bool,
        #[serde(rename = "capacity", default, deserialize_with = "null_default")]
        pub capacity: Capacity,
        #[serde(rename = "idempotent", default, deserialize_with = "null_default", skip_serializing_if = "is_zero_bool")]
        pub idempotent: bool,
    }
}

go_struct! {
    #[derive(PartialEq)]
    pub struct EnqueueRequest : EnqueueRequestWire {
        #[serde(rename = "Intent", default, deserialize_with = "null_default")]
        pub intent: InboxIntent,
        #[serde(rename = "Origin", default, deserialize_with = "null_default")]
        pub origin: PromptOrigin,
        #[serde(rename = "Envelope", default, deserialize_with = "null_default")]
        pub envelope: PromptEnvelope,
        #[serde(rename = "Source", default, deserialize_with = "null_default")]
        pub source: String,
        #[serde(rename = "Idempotency", default, deserialize_with = "null_default")]
        pub idempotency: String,
        #[serde(rename = "SessionID", default, deserialize_with = "null_default")]
        pub session_id: String,
    }
}

go_struct! {
    #[derive(PartialEq)]
    pub struct Limits : LimitsWire {
        #[serde(rename = "MaxItems", default, deserialize_with = "null_default")]
        pub max_items: i64,
        #[serde(rename = "MaxItemBytes", default, deserialize_with = "null_default")]
        pub max_item_bytes: i64,
        #[serde(rename = "MaxTotalBytes", default, deserialize_with = "null_default")]
        pub max_total_bytes: i64,
    }
}

impl Limits {
    pub fn with_defaults(mut self) -> Self {
        if self.max_items == 0 {
            self.max_items = DEFAULT_MAX_ITEMS;
        }
        if self.max_item_bytes <= 0 {
            self.max_item_bytes = DEFAULT_MAX_ITEM_BYTES;
        }
        if self.max_total_bytes <= 0 {
            self.max_total_bytes = DEFAULT_MAX_TOTAL_BYTES;
        }
        self
    }
}
