//! Compaction context sidecar (Go:
//! `internal/state/sessionstore/compaction_state.go`).
//!
//! `<stem>.context.json` holds the model-visible projection that
//! compaction maintains beside the canonical transcript, the durable
//! receipt of the last maintenance transaction, and the ledger of what a
//! projection generation has already recalled. The canonical transcript
//! always remains the source of truth: the sidecar is a cache that a
//! crash, a foreign schema, or a mismatched coverage fingerprint can
//! safely invalidate (docs/contracts/session-format.md 11).

use std::io;
use std::path::Path;

use serde::Serialize;
use serde_json::value::RawValue;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use reasonix_contract::go_struct;
use reasonix_contract::gojson::{
    self, escape_html, go_vec, is_zero_bool, is_zero_i64, null_default,
};
use reasonix_contract::provider::{Message, model_messages};
use reasonix_core::fileutil::atomic_write_file_strict;

use super::branch::is_zero_f64;
use super::eventlog::record::{CreatedAt, deserialize_created_at, now_utc};
use super::session::Session;
use super::session_lease::WriteAuthorityError;
use crate::store::naming::session_context;

/// Context-projection schema version 1 (Go: `CompactionStateSchemaV1`).
pub const COMPACTION_STATE_SCHEMA_V1: i64 = 1;
/// Context-projection schema version 2 (Go: `compactionStateSchemaV2`).
const COMPACTION_STATE_SCHEMA_V2: i64 = 2;
/// Context-projection schema version 3 (Go: `compactionStateSchemaV3`).
const COMPACTION_STATE_SCHEMA_V3: i64 = 3;
/// The schema version this build writes (Go:
/// `CompactionStateSchemaCurrent`).
pub const COMPACTION_STATE_SCHEMA_CURRENT: i64 = COMPACTION_STATE_SCHEMA_V3;
/// What 1.x writes from 1.36.0: schema 3 plus
/// `projection.pinned_context_hash` (Go:
/// `compactionStateSchema1xV4`). A 2.x schema bump must not reuse 4.
pub const COMPACTION_STATE_SCHEMA_1X_V4: i64 = 4;
/// Marks a context sidecar whose schema this build cannot interpret (Go:
/// `ErrContextSchemaUnsupported`).
pub const UNSUPPORTED_CONTEXT_SCHEMA: &str = "unsupported context schema";

/// `omitempty` skip predicate for Go unsigned integer fields: Go's
/// `omitempty` drops a zero; `gojson` has no u64 twin.
fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

/// Serializes `messages` exactly as Go marshals a
/// `[]provider.Message`: the zero value marshals as `null` (a
/// nil slice), a non-empty projection as the message array.
fn serialize_messages<S>(messages: &[Message], serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    if messages.is_empty() {
        return serializer.serialize_none();
    }
    Serialize::serialize(messages, serializer)
}

go_struct! {
    /// The session context sidecar (Go: `CompactionState`): the
    /// model-visible projection compaction maintains beside the canonical
    /// transcript, plus the durable receipt of the last maintenance
    /// transaction.
    ///
    /// `save_compaction_state` publishes it as a commit pointer
    /// through the strict atomic write (temp + rename only). A sidecar
    /// another release wrote is read, never rewritten in place; see
    /// `foreign`.
    pub struct CompactionState : CompactionStateWire {
        #[serde(rename = "schema_version", default, deserialize_with = "null_default")]
        pub schema_version: i64,
        #[serde(rename = "transcript_version", default, deserialize_with = "null_default")]
        pub transcript_version: u64,
        #[serde(rename = "projection", default, deserialize_with = "null_default")]
        pub projection: ContextProjection,
        #[serde(
            rename = "prompt_cache_key",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub prompt_cache_key: String,
        #[serde(
            rename = "last_cache_state",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub last_cache_state: String,
        #[serde(
            rename = "last_trigger",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub last_trigger: String,
        #[serde(
            rename = "last_mode",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub last_mode: String,
        #[serde(
            rename = "last_source_tokens",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub last_source_tokens: i64,
        #[serde(
            rename = "last_result_tokens",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub last_result_tokens: i64,
        #[serde(
            rename = "last_compaction_cost",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_f64"
        )]
        pub last_compaction_cost: f64,
        #[serde(
            rename = "generation",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_u64"
        )]
        pub generation: u64,
        // Go's omitempty never omits a struct, so the recall ledger is
        // always serialized once the state is.
        #[serde(rename = "recall", default, deserialize_with = "null_default")]
        pub recall: RecallLedger,
        #[serde(
            rename = "last_receipt",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "Option::is_none"
        )]
        pub last_receipt: Option<ContextMaintenanceReceipt>,
        #[serde(
            rename = "blocked_input_hash",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub blocked_input_hash: String,
        #[serde(
            rename = "blocked_reason",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub blocked_reason: String,
        #[serde(
            rename = "native_context_editing_accepted",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_bool"
        )]
        pub native_context_editing_accepted: bool,
        #[serde(
            rename = "context_editing_fallback_local",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_bool"
        )]
        pub context_editing_fallback_local: bool,
        #[serde(rename = "updated_at", default, deserialize_with = "deserialize_created_at")]
        pub updated_at: CreatedAt,
    }
}

go_struct! {
    /// The model-visible view of a session (Go: `ContextProjection`).
    /// The canonical transcript is never replaced by this structure; it is
    /// a projection the compaction pipeline commits beside the transcript.
    pub struct ContextProjection : ContextProjectionWire {
        #[serde(
            rename = "messages",
            default,
            deserialize_with = "go_vec",
            serialize_with = "serialize_messages"
        )]
        pub messages: Vec<Message>,
        #[serde(rename = "transcript_version", default, deserialize_with = "null_default")]
        pub transcript_version: u64,
        #[serde(rename = "projection_version", default, deserialize_with = "null_default")]
        pub projection_version: u64,
        #[serde(rename = "covered_count", default, deserialize_with = "null_default")]
        pub covered_count: i64,
        #[serde(
            rename = "covered_prefix_hash",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub covered_prefix_hash: String,
        #[serde(
            rename = "summary_hash",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub summary_hash: String,
        #[serde(
            rename = "source_tokens",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub source_tokens: i64,
        #[serde(
            rename = "projection_tokens",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub projection_tokens: i64,
        #[serde(
            rename = "view_input_hash",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub view_input_hash: String,
        #[serde(
            rename = "view_output_hash",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub view_output_hash: String,
        #[serde(rename = "created_at", default, deserialize_with = "deserialize_created_at")]
        pub created_at: CreatedAt,
    }
}

go_struct! {
    /// The durable, provider-neutral outcome of one context maintenance
    /// transaction (Go: `ContextMaintenanceReceipt`). Transcript
    /// content is intentionally not included: hashes and counts are
    /// sufficient for dedupe and diagnostics.
    pub struct ContextMaintenanceReceipt : ContextMaintenanceReceiptWire {
        #[serde(
            rename = "operation_id",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub operation_id: String,
        #[serde(
            rename = "status",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub status: String,
        #[serde(
            rename = "action",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub action: String,
        #[serde(
            rename = "trigger",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub trigger: String,
        #[serde(
            rename = "source_projection",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_u64"
        )]
        pub source_projection: u64,
        #[serde(
            rename = "projection_version",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_u64"
        )]
        pub projection_version: u64,
        #[serde(
            rename = "covered_count",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub covered_count: i64,
        #[serde(
            rename = "covered_prefix_hash",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub covered_prefix_hash: String,
        #[serde(
            rename = "input_hash",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub input_hash: String,
        #[serde(
            rename = "output_hash",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub output_hash: String,
        #[serde(
            rename = "input_tokens",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub input_tokens: i64,
        #[serde(
            rename = "result_tokens",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub result_tokens: i64,
        #[serde(
            rename = "saved_tokens",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub saved_tokens: i64,
        #[serde(
            rename = "affected_tool_results",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub affected_tool_results: i64,
        #[serde(
            rename = "summary_hash",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub summary_hash: String,
        #[serde(
            rename = "cache_break",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_bool"
        )]
        pub cache_break: bool,
        #[serde(
            rename = "reason",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub reason: String,
        #[serde(
            rename = "code",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub code: String,
        #[serde(
            rename = "boundary",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub boundary: String,
        #[serde(
            rename = "trigger_tokens",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub trigger_tokens: i64,
        #[serde(
            rename = "blocked_input_hash",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub blocked_input_hash: String,
        // Go's omitempty never omits a time.Time value, so a receipt's
        // created_at is always emitted once the receipt is.
        #[serde(rename = "created_at", default, deserialize_with = "deserialize_created_at")]
        pub created_at: CreatedAt,
        #[serde(
            rename = "summary_usage",
            default,
            skip_serializing_if = "CompactionUsage::is_zero"
        )]
        pub summary_usage: CompactionUsage,
    }
}

go_struct! {
    /// What one projection generation has already pulled back out of the
    /// fold (Go: `RecallLedger`). Carrying the generation it belongs
    /// to is what resets the budget: a stale generation reads as an
    /// unspent one, with no reset call to forget.
    pub struct RecallLedger : RecallLedgerWire {
        #[serde(
            rename = "generation",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_u64"
        )]
        pub generation: u64,
        #[serde(
            rename = "spent_tokens",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub spent_tokens: i64,
    }
}

go_struct! {
    /// What one maintenance transaction spent with the provider (Go:
    /// `CompactionUsage`): every summarizer call it made, including
    /// the ones whose answer was thrown away. This is not the size of the
    /// context that was worked on — that is the receipt's
    /// `input_tokens`, and the two are routinely orders of
    /// magnitude apart.
    #[derive(PartialEq, Eq)]
    pub struct CompactionUsage : CompactionUsageWire {
        #[serde(
            rename = "calls",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub calls: i64,
        #[serde(
            rename = "request_attempts",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub request_attempts: i64,
        #[serde(
            rename = "input_tokens",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub input_tokens: i64,
        #[serde(
            rename = "output_tokens",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub output_tokens: i64,
        #[serde(
            rename = "reasoning_tokens",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub reasoning_tokens: i64,
        #[serde(
            rename = "cache_hit_tokens",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub cache_hit_tokens: i64,
        #[serde(
            rename = "cache_miss_tokens",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub cache_miss_tokens: i64,
        #[serde(
            rename = "cache_write_tokens",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub cache_write_tokens: i64,
    }
}

impl CompactionUsage {
    /// The `omitzero` predicate for a receipt's `summary_usage`.
    pub fn is_zero(&self) -> bool {
        *self == Self::default()
    }
}

/// Failures loading, saving, or removing the context sidecar.
#[derive(Debug, Error)]
pub enum CompactionStateError {
    /// Filesystem failure.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// The sidecar is not valid JSON (Go: `decode context state %s: %w`).
    #[error("decode context state {path}: {source}")]
    Decode {
        /// The sidecar path.
        path: String,
        /// The first JSON syntax error.
        source: serde_json::Error,
    },
    /// The sidecar's schema cannot be interpreted by this build (Go:
    /// `ErrContextSchemaUnsupported`).
    #[error("context state {path}: {detail}")]
    Schema {
        /// The sidecar path.
        path: String,
        /// Which schema rule the file broke.
        detail: String,
    },
    /// The session path is empty (Go: `empty session path`).
    #[error("empty session path")]
    EmptyPath,
    /// The state could not be encoded.
    #[error(transparent)]
    Encode(serde_json::Error),
}

impl CompactionStateError {
    /// Whether the error is Go's `ErrContextSchemaUnsupported`.
    pub fn is_unsupported_schema(&self) -> bool {
        matches!(self, CompactionStateError::Schema { .. })
    }
}

/// One tool call in the fingerprint wire (Go: `wireCall`).
#[derive(Serialize)]
struct FingerprintCall<'a> {
    #[serde(rename = "id", skip_serializing_if = "str::is_empty")]
    id: &'a str,
    #[serde(rename = "name", skip_serializing_if = "str::is_empty")]
    name: &'a str,
    #[serde(rename = "args", skip_serializing_if = "str::is_empty")]
    arguments: &'a str,
    #[serde(rename = "ts", skip_serializing_if = "str::is_empty")]
    thought_signature: &'a str,
}

/// One message in the fingerprint wire (Go: `wireMsg`). The short
/// keys are the persisted fingerprint format: renaming any of them, or
/// reordering fields, invalidates every stored coverage hash.
#[derive(Serialize)]
struct FingerprintMsg<'a> {
    #[serde(rename = "r")]
    role: &'a str,
    #[serde(rename = "c", skip_serializing_if = "str::is_empty")]
    content: &'a str,
    #[serde(rename = "img", skip_serializing_if = "<[String]>::is_empty")]
    images: &'a [String],
    #[serde(rename = "rc", skip_serializing_if = "str::is_empty")]
    reasoning_content: &'a str,
    #[serde(rename = "rid", skip_serializing_if = "str::is_empty")]
    reasoning_id: &'a str,
    #[serde(rename = "rst", skip_serializing_if = "str::is_empty")]
    reasoning_status: &'a str,
    #[serde(rename = "rsig", skip_serializing_if = "str::is_empty")]
    reasoning_signature: &'a str,
    #[serde(rename = "tid", skip_serializing_if = "str::is_empty")]
    tool_call_id: &'a str,
    #[serde(rename = "n", skip_serializing_if = "str::is_empty")]
    name: &'a str,
    #[serde(rename = "tc", skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<FingerprintCall<'a>>,
    #[serde(rename = "ri", skip_serializing_if = "<[Box<RawValue>]>::is_empty")]
    responses_items: &'a [Box<RawValue>],
}

impl CompactionState {
    /// Reports whether another release line wrote this state (Go:
    /// `Foreign`). Its file is read, never rewritten in place by a
    /// load; only a projection commit for this release overwrites it.
    pub fn foreign(&self) -> bool {
        self.schema_version > COMPACTION_STATE_SCHEMA_CURRENT
    }
}

/// Reads the context sidecar (Go: `LoadCompactionState`). A missing
/// file returns `None` (Go: `ok=false`); corrupt or
/// unsupported state returns an error and the caller rebuilds from the
/// canonical transcript, leaving the file alone.
pub fn load_compaction_state(
    session_path: &Path,
) -> Result<Option<CompactionState>, CompactionStateError> {
    let path = session_context(session_path);
    if path.as_os_str().is_empty() {
        return Ok(None);
    }
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut st: CompactionState =
        serde_json::from_slice(&bytes).map_err(|source| CompactionStateError::Decode {
            path: path.to_string_lossy().into_owned(),
            source,
        })?;
    match st.schema_version {
        0 => st.schema_version = COMPACTION_STATE_SCHEMA_V1,
        COMPACTION_STATE_SCHEMA_V1 | COMPACTION_STATE_SCHEMA_V2 | COMPACTION_STATE_SCHEMA_V3 => {}
        COMPACTION_STATE_SCHEMA_1X_V4 => {
            check_1x_v4_projection(&bytes).map_err(|detail| CompactionStateError::Schema {
                path: path.to_string_lossy().into_owned(),
                detail,
            })?;
        }
        _ => {
            return Err(CompactionStateError::Schema {
                path: path.to_string_lossy().into_owned(),
                detail: format!("{UNSUPPORTED_CONTEXT_SCHEMA} version {}", st.schema_version),
            });
        }
    }
    Ok(Some(st))
}

/// Admits a 1.x schema-4 sidecar whose projection carries no
/// pinned-context checkpoint (Go: `check1xV4Projection`). A
/// checkpoint is authenticated against 1.x's pinned revision provenance,
/// which this build does not model, so it cannot be trusted.
fn check_1x_v4_projection(bytes: &[u8]) -> Result<(), String> {
    #[derive(serde::Deserialize)]
    struct V4Probe {
        #[serde(default)]
        projection: V4ProjectionProbe,
    }
    #[derive(serde::Deserialize, Default)]
    struct V4ProjectionProbe {
        #[serde(rename = "pinned_context_hash", default)]
        pinned_context_hash: String,
    }
    match serde_json::from_slice::<V4Probe>(bytes) {
        Ok(probe) if probe.projection.pinned_context_hash.is_empty() => Ok(()),
        Ok(_) => Err(format!(
            "{UNSUPPORTED_CONTEXT_SCHEMA}: 1.x schema 4 with a pinned-context checkpoint"
        )),
        Err(e) => Err(e.to_string()),
    }
}

/// Publishes the sidecar through the strict atomic write (temp + rename
/// only; Go: `SaveCompactionState`). Checkpoint sidecars are commit
/// pointers: copy fallbacks that can tear an existing file are rejected,
/// so a failed write leaves the previous checkpoint intact. The state is
/// stamped with the current schema, `updated_at` is defaulted, and
/// the mirrored `last_*`/`blocked_*` writer fields are
/// cleared — the receipt is authoritative.
pub fn save_compaction_state(
    session_path: &Path,
    mut st: CompactionState,
) -> Result<(), CompactionStateError> {
    let path = session_context(session_path);
    if path.as_os_str().is_empty() {
        return Err(CompactionStateError::EmptyPath);
    }
    st.schema_version = COMPACTION_STATE_SCHEMA_CURRENT;
    if st.updated_at.is_zero() {
        st.updated_at = now_utc();
    }
    st.last_trigger = String::new();
    st.last_mode = String::new();
    st.last_source_tokens = 0;
    st.last_result_tokens = 0;
    st.last_compaction_cost = 0.0;
    if st.last_receipt.is_some() {
        st.blocked_input_hash = String::new();
        st.blocked_reason = String::new();
    }
    let json = serde_json::to_string_pretty(&st).map_err(CompactionStateError::Encode)?;
    let mut json = escape_html(&json);
    json.push('\n');
    atomic_write_file_strict(&path, json.as_bytes(), 0o644)?;
    Ok(())
}

/// Errors from saving a session's compaction state through the
/// authority gate.
#[derive(Debug, Error)]
pub enum SaveSessionCompactionStateError {
    #[error(transparent)]
    WriteAuthority(#[from] WriteAuthorityError),
    #[error(transparent)]
    CompactionState(#[from] CompactionStateError),
}

impl Session {
    /// Save the session's compaction state under the bound write
    /// authority (Go: compaction_state.go `SaveCompactionState`): the
    /// `requireWriteAuthorityForSave` gate plus the free function.
    pub fn save_compaction_state(
        &self,
        session_path: &str,
        st: CompactionState,
    ) -> Result<(), SaveSessionCompactionStateError> {
        let _authority_save = self
            .require_write_authority_for_save(session_path)
            .map_err(SaveSessionCompactionStateError::WriteAuthority)?;
        save_compaction_state(Path::new(session_path), st)
            .map_err(SaveSessionCompactionStateError::CompactionState)
    }
}

/// Deletes an invalidated projection sidecar (Go:
/// `RemoveCompactionState`). A missing file is not an error:
/// removal merely drops the cache.
pub fn remove_compaction_state(session_path: &Path) -> Result<(), CompactionStateError> {
    let path = session_context(session_path);
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Fingerprints the provider-visible prefix `msgs[..n]` (Go:
/// `CoveredPrefixHash`). Out-of-range counts return the empty
/// string: the caller must not claim coverage it cannot prove.
pub fn covered_prefix_hash(msgs: &[Message], n: i64) -> String {
    if n <= 0 || n > msgs.len() as i64 {
        return String::new();
    }
    provider_visible_fingerprint(&model_messages(&msgs[..n as usize]))
}

/// The stable hash of what a provider sees (Go:
/// `ProviderVisibleFingerprint`): SHA-256 over the compact,
/// HTML-escaped wire of the visible fields, hex of the first 16 bytes.
///
/// Divergence from Go: `json.RawMessage` items (Responses items)
/// are emitted verbatim, while Go's encoder re-compacts them. Only
/// fixtures hand-whitespaced between JSON tokens differ; provider bytes
/// are compact in practice.
pub fn provider_visible_fingerprint(msgs: &[Message]) -> String {
    let wire: Vec<FingerprintMsg<'_>> = msgs
        .iter()
        .map(|m| FingerprintMsg {
            role: m.role.as_str(),
            content: &m.content,
            images: &m.images,
            reasoning_content: &m.reasoning_content,
            reasoning_id: &m.reasoning_id,
            reasoning_status: &m.reasoning_status,
            reasoning_signature: &m.reasoning_signature,
            tool_call_id: &m.tool_call_id,
            name: &m.name,
            tool_calls: m
                .tool_calls
                .iter()
                .map(|call| FingerprintCall {
                    id: &call.id,
                    name: &call.name,
                    arguments: &call.arguments,
                    thought_signature: &call.thought_signature,
                })
                .collect(),
            responses_items: &m.responses_items,
        })
        .collect();
    let Ok(wire_json) = gojson::marshal(&wire) else {
        return String::new();
    };
    let sum = Sha256::digest(wire_json.as_bytes());
    hex_16(&sum[..16])
}

/// Lowercase hex of `bytes` (Go: `hex.EncodeToString`).
fn hex_16(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::sessionstore::eventlog::test_support::tempdir;
    use reasonix_contract::provider::{Role, ToolCall, VideoRef};
    use std::path::PathBuf;

    use super::*;

    fn session(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("s.jsonl")
    }

    fn sidecar_path(session_path: &Path) -> PathBuf {
        session_context(session_path)
    }

    fn write_sidecar(session_path: &Path, body: &str) {
        std::fs::write(sidecar_path(session_path), body).unwrap();
    }

    fn visible(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
            ..Message::default()
        }
    }

    #[test]
    fn load_reads_other_release_schemas() {
        // (a) A 1.x schema-4 sidecar without a pinned checkpoint loads.
        let dir = tempdir();
        let session_path = session(&dir);
        write_sidecar(
            &session_path,
            r#"{"schema_version":4,"projection":{"messages":[{"role":"user","content":"s"}],"covered_count":1,"covered_prefix_hash":"h"}}"#,
        );
        let st = load_compaction_state(&session_path)
            .unwrap()
            .expect("sidecar");
        assert_eq!(st.schema_version, COMPACTION_STATE_SCHEMA_1X_V4);
        assert!(st.foreign());
        assert_eq!(st.projection.covered_prefix_hash, "h");
        assert_eq!(st.projection.covered_count, 1);
        assert_eq!(st.projection.messages.len(), 1);
        assert_eq!(st.projection.messages[0].content, "s");

        // (b) A pinned-context checkpoint is 1.x-revisioned content this
        // build cannot authenticate.
        let dir = tempdir();
        let session_path = session(&dir);
        write_sidecar(
            &session_path,
            r#"{"schema_version":4,"projection":{"messages":[{"role":"user","content":"s"}],"pinned_context_hash":"sha256:ab","covered_count":1}}"#,
        );
        let err = load_compaction_state(&session_path).unwrap_err();
        assert!(err.is_unsupported_schema());
        assert_eq!(
            err.to_string(),
            format!(
                "context state {}: unsupported context schema: 1.x schema 4 with a pinned-context checkpoint",
                sidecar_path(&session_path).to_string_lossy()
            )
        );

        // (c) Anything else belongs to a future release.
        let dir = tempdir();
        let session_path = session(&dir);
        write_sidecar(
            &session_path,
            r#"{"schema_version":99,"projection":{"messages":[]}}"#,
        );
        let err = load_compaction_state(&session_path).unwrap_err();
        assert!(err.is_unsupported_schema());
        assert_eq!(
            err.to_string(),
            format!(
                "context state {}: unsupported context schema version 99",
                sidecar_path(&session_path).to_string_lossy()
            )
        );
    }

    #[test]
    fn schema_zero_reads_as_v1_and_missing_sidecars_are_absent() {
        let dir = tempdir();
        let session_path = session(&dir);
        write_sidecar(&session_path, r#"{"schema_version":0}"#);
        let st = load_compaction_state(&session_path)
            .unwrap()
            .expect("sidecar");
        assert_eq!(st.schema_version, COMPACTION_STATE_SCHEMA_V1);
        assert!(!st.foreign());
        assert!(st.updated_at.is_zero());

        let dir = tempdir();
        assert!(load_compaction_state(&session(&dir)).unwrap().is_none());
        assert!(load_compaction_state(Path::new("")).unwrap().is_none());
    }

    #[test]
    fn corrupt_sidecars_fail_to_decode() {
        let dir = tempdir();
        let session_path = session(&dir);
        write_sidecar(&session_path, "{not json");
        let err = load_compaction_state(&session_path).unwrap_err();
        assert!(
            err.to_string().starts_with("decode context state "),
            "unexpected error: {err}"
        );
        let sidecar = sidecar_path(&session_path).to_string_lossy().into_owned();
        assert!(err.to_string().contains(&sidecar));
    }

    #[test]
    fn save_round_trips_and_rewrites_writer_fields() {
        let dir = tempdir();
        let session_path = session(&dir);
        let mut st = CompactionState {
            transcript_version: 7,
            prompt_cache_key: "pck".to_string(),
            last_cache_state: "warm".to_string(),
            last_trigger: "auto".to_string(),
            last_mode: "snip".to_string(),
            last_source_tokens: 11,
            last_result_tokens: 12,
            last_compaction_cost: 1.5,
            generation: 2,
            blocked_input_hash: "bh".to_string(),
            blocked_reason: "br".to_string(),
            ..CompactionState::default()
        };
        st.last_receipt = Some(ContextMaintenanceReceipt {
            operation_id: "op".to_string(),
            status: "applied".to_string(),
            action: "snip".to_string(),
            covered_count: 3,
            reason: "<script>&".to_string(),
            ..ContextMaintenanceReceipt::default()
        });
        save_compaction_state(&session_path, st).unwrap();

        let path = sidecar_path(&session_path);
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.starts_with("{\n  \""));
        assert!(raw.ends_with("}\n"));
        assert!(raw.contains("\\u003cscript\\u003e\\u0026"));
        assert!(raw.contains("\"created_at\": \"0001-01-01T00:00:00Z\""));
        assert!(raw.contains("\"recall\": {}"));
        assert!(raw.contains("\"messages\": null"));
        assert!(raw.contains("\"status\": \"applied\""));
        assert!(!raw.contains("last_trigger"));
        assert!(!raw.contains("blocked_input_hash"));
        assert!(!raw.contains("summary_usage"));

        let st = load_compaction_state(&session_path)
            .unwrap()
            .expect("sidecar");
        assert_eq!(st.schema_version, COMPACTION_STATE_SCHEMA_CURRENT);
        assert!(!st.updated_at.is_zero());
        assert_eq!(st.transcript_version, 7);
        assert_eq!(st.prompt_cache_key, "pck");
        assert_eq!(st.last_cache_state, "warm");
        assert_eq!(st.last_trigger, "");
        assert_eq!(st.last_mode, "");
        assert_eq!(st.last_source_tokens, 0);
        assert_eq!(st.last_result_tokens, 0);
        assert_eq!(st.last_compaction_cost, 0.0);
        assert_eq!(st.generation, 2);
        assert_eq!(st.blocked_input_hash, "");
        assert_eq!(st.blocked_reason, "");
        let receipt = st.last_receipt.clone().expect("receipt");
        assert_eq!(receipt.status, "applied");
        assert_eq!(receipt.action, "snip");
        assert_eq!(receipt.covered_count, 3);
        assert_eq!(receipt.reason, "<script>&");
        assert!(receipt.created_at.is_zero());

        // Usage rides along once the summarizer spent something.
        let mut spent = st.clone();
        let mut receipt = spent.last_receipt.clone().expect("receipt");
        receipt.summary_usage = CompactionUsage {
            calls: 2,
            input_tokens: 30,
            ..CompactionUsage::default()
        };
        spent.last_receipt = Some(receipt);
        save_compaction_state(&session_path, spent).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"summary_usage\""));
        assert!(raw.contains("\"calls\": 2"));
        let st = load_compaction_state(&session_path)
            .unwrap()
            .expect("sidecar");
        assert_eq!(st.last_receipt.expect("receipt").summary_usage.calls, 2);
    }

    #[test]
    fn save_requires_a_path_and_remove_tolerates_missing_files() {
        let err = save_compaction_state(Path::new(""), CompactionState::default()).unwrap_err();
        assert_eq!(err.to_string(), "empty session path");
        remove_compaction_state(Path::new("")).unwrap();

        let dir = tempdir();
        let session_path = session(&dir);
        remove_compaction_state(&session_path).unwrap();

        let mut st = CompactionState::default();
        st.projection.covered_count = 1;
        save_compaction_state(&session_path, st).unwrap();
        let path = sidecar_path(&session_path);
        assert!(path.exists());
        remove_compaction_state(&session_path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn covered_prefix_hash_bounds_out_of_range_counts() {
        let msgs = vec![
            visible(Role::USER, "hello"),
            visible(Role::ASSISTANT, "hi"),
            visible(Role::TOOL, "done"),
        ];
        assert_eq!(covered_prefix_hash(&msgs, 0), "");
        assert_eq!(covered_prefix_hash(&msgs, -1), "");
        assert_eq!(covered_prefix_hash(&msgs, 4), "");
        let all = covered_prefix_hash(&msgs, 3);
        let head = covered_prefix_hash(&msgs, 2);
        assert_eq!(all.len(), 32);
        assert_eq!(head.len(), 32);
        assert_ne!(all, head);
        assert!(all.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn covered_prefix_hash_strips_local_only_messages() {
        let mut msgs = vec![visible(Role::USER, "hello"), visible(Role::ASSISTANT, "hi")];
        msgs[1].local_only = true;
        let with_local = covered_prefix_hash(&msgs, 2);
        msgs[1].local_only = false;
        let without_local = covered_prefix_hash(&msgs, 2);
        assert_ne!(with_local, without_local);
        assert_eq!(
            with_local,
            covered_prefix_hash(&[visible(Role::USER, "hello")], 1)
        );
    }

    #[test]
    fn fingerprint_matches_the_go_wire_bytes() {
        // Expected value produced by the Go oracle
        // (.parity-gocopy/fp/main.go): the same wire structs, tags, and
        // marshal options over the same messages.
        let mut user = visible(Role::USER, "hello <world> & friends");
        user.images = vec!["data:image/png;base64,AAA".to_string()];
        let mut assistant = visible(Role::ASSISTANT, "");
        assistant.reasoning_content = "thinking".to_string();
        assistant.reasoning_id = "rs_1".to_string();
        assistant.reasoning_status = "completed".to_string();
        assistant.reasoning_signature = "sig==".to_string();
        assistant.tool_calls = vec![
            ToolCall {
                id: "c1".to_string(),
                name: "bash".to_string(),
                arguments: "{\"cmd\":\"ls\"}".to_string(),
                thought_signature: "ts1".to_string(),
                ..ToolCall::default()
            },
            ToolCall {
                id: "c2".to_string(),
                name: "read".to_string(),
                arguments: "{}".to_string(),
                ..ToolCall::default()
            },
        ];
        assistant.responses_items = vec![
            serde_json::from_str::<Box<RawValue>>(r#"{"type":"reasoning","id":"r1"}"#).unwrap(),
            serde_json::from_str::<Box<RawValue>>(r#"{"type":"message"}"#).unwrap(),
        ];
        let mut tool = visible(Role::TOOL, "result");
        tool.tool_call_id = "c1".to_string();
        tool.name = "bash".to_string();
        let msgs = vec![user, assistant, tool];
        assert_eq!(
            provider_visible_fingerprint(&msgs),
            "b882b7a17fcd6f3ebae72b0b195f9206"
        );
    }

    #[test]
    fn fingerprint_ignores_local_metadata_and_tracks_visible_fields() {
        let msgs = vec![
            visible(Role::USER, "hello"),
            visible(Role::ASSISTANT, "hi"),
            visible(Role::TOOL, "done"),
        ];
        // Non-visible metadata never moves the fingerprint.
        let mut stamped = msgs.clone();
        stamped[0].created_at = 12345;
        stamped[0].videos = vec![VideoRef {
            name: "v.mp4".to_string(),
            ..VideoRef::default()
        }];
        assert_eq!(
            provider_visible_fingerprint(&msgs),
            provider_visible_fingerprint(&stamped)
        );
        // Visible fields do.
        let mut edited = msgs.clone();
        edited[1].reasoning_signature = "sig".to_string();
        assert_ne!(
            provider_visible_fingerprint(&msgs),
            provider_visible_fingerprint(&edited)
        );
        let mut called = msgs.clone();
        called[1].tool_calls = vec![ToolCall {
            id: "c".to_string(),
            arguments: "{}".to_string(),
            ..ToolCall::default()
        }];
        assert_ne!(
            provider_visible_fingerprint(&msgs),
            provider_visible_fingerprint(&called)
        );
    }
}
