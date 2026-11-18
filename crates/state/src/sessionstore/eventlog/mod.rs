//! Session event log: wire records, replay, probe, and writers.
//!
//! Parity target: reference/.../internal/state/sessionstore
//! (docs/contracts/session-format.md 3). The event log is the
//! authoritative transcript; this module reads it exactly the way the
//! Go kernel does, and writes through the same append/replace/fold
//! matrix (writer.rs, images.rs write path).
//!
//! Three failure classes stay distinct, as in Go:
//! - hard errors (unsupported schema version / event type) mean a
//!   newer writer owns the log; nothing is truncated;
//! - budget refusals ([SessionReplayLimitError]) leave the session
//!   untouched and never fall back to an older checkpoint;
//! - damage (torn record, broken append chain) stops the replay with
//!   everything applied so far and reports damaged=true so the writer
//!   can self-heal the tail.
//!
//! Not ported: the context.Context carried by Go sessionReplayLimits
//! (load_context.go 16-40) surfaces cancellation between decode steps;
//! the Rust core stays synchronous and callers cancel through the
//! blocking-task boundary (AGENTS.md 10.3).

pub mod images;
pub mod index;
pub mod probe;
pub mod record;
pub mod repair;
pub mod replay;
pub mod writer;
pub mod writer_id;

pub(crate) mod reader;

mod preflight;

pub use images::{
    DegradedCheckpointError, ERR_SESSION_IMAGES_UNAVAILABLE, ExternalizeError, ExternalizedImages,
    SESSION_IMAGE_BLOB_MAX_BYTES, SESSION_IMAGE_BLOB_SCHEME, SESSION_IMAGE_BLOB_TEMP_MAX_AGE,
    backfill_unavailable_images, carries_stored_image, checkpoint_images,
    externalize_session_images, holds_any_image, inline_image_bytes, is_data_url,
    message_image_slots, read_session_image_blob, refuse_degraded_checkpoint_rewrite,
    resolve_session_images, restore_unavailable_image, salvaged_image_keys,
    session_image_blob_path, session_image_key, session_image_slot_count,
    sweep_session_image_blobs, unavailable_image_keys, write_session_image_blob,
};
pub use index::{SessionEventIndex, read_session_event_index, write_session_event_index};
pub use probe::{
    SessionEventLogProbe, probe_session_event_log, probe_session_event_log_with_limits,
};
pub use record::{
    CreatedAt, SessionEventImages, SessionEventWireRecord, SessionImageSlot,
    composition_event_schema, composition_removed, native_session_event_schema,
    session_event_record_schema,
};
pub use repair::{RepairError, preserve_damaged_event_log_tail, repair_session_event_log_tail};
pub use replay::{
    ReplayError, SessionEventReplay, replay_session_event_log, replay_session_event_log_with_limits,
};
pub use writer::{
    SessionEventRecord, SessionEventWriteError, append_session_append_event, append_session_event,
    append_session_replace_event, compact_session_event_log, session_event_log_oversized,
    session_event_log_size,
};

/// Schema of a plain event record (Go: sessionEventSchemaVersion).
pub const SESSION_EVENT_SCHEMA_VERSION: i64 = 1;
/// Schema of the 1.x DAG log, replayed by its own reader (Go:
/// sessionDAGSchemaVersion, session_dag_read.go).
pub const SESSION_EVENT_DAG_SCHEMA_VERSION: i64 = 2;
/// Schema stamping a record whose images live in blobs (Go:
/// sessionEventImageBlobsSchemaVersion).
pub const SESSION_EVENT_IMAGE_BLOBS_SCHEMA_VERSION: i64 = 3;
/// Schema of an agent-composition event (Go:
/// sessionEventCompositionSchemaVersion, composition_events.go).
pub const SESSION_EVENT_COMPOSITION_SCHEMA_VERSION: i64 = 4;
/// Schema of a composition event whose binding is live (Go:
/// sessionEventLiveCompositionSchemaVersion).
pub const SESSION_EVENT_LIVE_COMPOSITION_SCHEMA_VERSION: i64 = 5;

pub const SESSION_EVENT_TYPE_REPLACE: &str = "replace";
pub const SESSION_EVENT_TYPE_APPEND: &str = "append";

/// Path's parent, or "." when it has none (a bare name) — the Go
/// filepath.Dir shape used for mkdir/sync parents.
pub(crate) fn parent_or_dot(path: &std::path::Path) -> &std::path::Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => std::path::Path::new("."),
    }
}

/// Cap on decoder input before any record is materialized (Go:
/// sessionEventReplayMaxBytes).
pub const SESSION_EVENT_REPLAY_MAX_BYTES: i64 = 512 << 20;
/// Bound on what a save may leave behind before folding (Go:
/// sessionEventLogFoldBytes).
pub const SESSION_EVENT_LOG_FOLD_BYTES: i64 = 128 << 20;
/// A byte limit alone is insufficient: compact JSON can expand into a
/// much larger graph of messages (Go: sessionEventReplayMaxRecords).
pub const SESSION_EVENT_REPLAY_MAX_RECORDS: usize = 100_000;
pub const SESSION_EVENT_REPLAY_MAX_MESSAGES: usize = 100_000;
pub const SESSION_EVENT_REPLAY_MAX_COLLECTION_ITEMS: usize = 100_000;
/// The probe reads at most this many bytes hunting the header fields
/// (Go: sessionEventProbeMaxBytes).
pub const SESSION_EVENT_PROBE_MAX_BYTES: i64 = 4 << 10;
/// Smallest log size that can trigger event-log maintenance (Go:
/// sessionEventLogCompactFloor).
pub const SESSION_EVENT_LOG_COMPACT_FLOOR: i64 = 256 << 10;
/// The log stays within this multiple of the live transcript size (Go:
/// sessionEventLogCompactFactor).
pub const SESSION_EVENT_LOG_COMPACT_FACTOR: i64 = 4;
/// Default bound on the message bytes one replay holds (Go:
/// sessionReplayMaxPayloadBytes, session_replay_reader.go).
pub const SESSION_REPLAY_MAX_PAYLOAD_BYTES: i64 = 64 << 20;
/// Default tail budget of loadSessionTail (Go: sessionTailBudgetBytes,
/// session_tail.go).
pub const SESSION_TAIL_BUDGET_BYTES: i64 = 16 << 20;

/// Sentinel every budget refusal renders with (Go:
/// ErrSessionReplayLimitExceeded). Callers must not fall back to an
/// older checkpoint: the event log may contain newer turns.
pub const ERR_SESSION_REPLAY_LIMIT_EXCEEDED: &str = "session history exceeds safe replay limits";

/// Replay budgets (Go: sessionReplayLimits, session_events.go 83). A
/// zero max_payload_bytes selects the default; a zero tail_budget
/// disables tail trimming, so replay refuses oversized payloads
/// instead of keeping only their tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionReplayLimits {
    /// Decoder input cap (resource encoded_bytes).
    pub max_bytes: i64,
    /// Cap on applied event records.
    pub max_records: usize,
    /// Cap on decoded messages across the whole log.
    pub max_messages: usize,
    /// Cap on nested JSON collection items across the whole log.
    pub max_collection_items: usize,
    /// Cap on message payload bytes held at once; 0 selects
    /// [SESSION_REPLAY_MAX_PAYLOAD_BYTES].
    pub max_payload_bytes: i64,
    /// When non-zero, keep only the newest messages within this many
    /// encoded bytes.
    pub tail_budget: i64,
}

impl Default for SessionReplayLimits {
    /// Go: defaultSessionReplayLimits (session_events.go 107).
    fn default() -> Self {
        Self {
            max_bytes: SESSION_EVENT_REPLAY_MAX_BYTES,
            max_records: SESSION_EVENT_REPLAY_MAX_RECORDS,
            max_messages: SESSION_EVENT_REPLAY_MAX_MESSAGES,
            max_collection_items: SESSION_EVENT_REPLAY_MAX_COLLECTION_ITEMS,
            max_payload_bytes: 0,
            tail_budget: 0,
        }
    }
}

impl SessionReplayLimits {
    /// Effective payload bound: the field value or the default when
    /// zero (Go: the fallback inside payloadErr).
    pub(crate) fn payload_limit(&self) -> i64 {
        if self.max_payload_bytes == 0 {
            SESSION_REPLAY_MAX_PAYLOAD_BYTES
        } else {
            self.max_payload_bytes
        }
    }

    /// The refusal for a payload of n message bytes, or None when it
    /// fits (Go: sessionReplayLimits.payloadErr, session_events.go 96).
    pub(crate) fn payload_err(&self, path: &str, n: i64) -> Option<SessionReplayLimitError> {
        let limit = self.payload_limit();
        if n > limit {
            Some(session_replay_limit_error(
                path,
                "message_payload_bytes",
                n,
                limit,
            ))
        } else {
            None
        }
    }
}

/// A budget refusal: machine-readable diagnostics, rendered without
/// local paths so Desktop surfaces can show it safely (Go:
/// SessionReplayLimitError, session_events.go 64).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionReplayLimitError {
    /// The log that was refused.
    pub path: String,
    /// Which budget tripped (encoded_bytes, event_records, messages,
    /// message_collection_items, message_payload_bytes,
    /// event_record_bytes).
    pub resource: &'static str,
    /// Observed size that tripped the budget.
    pub value: i64,
    /// The budget itself.
    pub limit: i64,
}

impl std::fmt::Display for SessionReplayLimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{ERR_SESSION_REPLAY_LIMIT_EXCEEDED}: {}={}, limit={}; session files were left unchanged",
            self.resource, self.value, self.limit
        )
    }
}

impl std::error::Error for SessionReplayLimitError {}

/// Builds a budget refusal and logs it the way Go does (Go:
/// sessionReplayLimitError, session_events.go 114: slog.Warn).
pub(crate) fn session_replay_limit_error(
    path: &str,
    resource: &'static str,
    value: i64,
    limit: i64,
) -> SessionReplayLimitError {
    tracing::warn!(
        path = path,
        resource = resource,
        value = value,
        limit = limit,
        "session: refusing unsafe event-log replay"
    );
    SessionReplayLimitError {
        path: path.to_owned(),
        resource,
        value,
        limit,
    }
}

/// Internal decode outcome: budget refusals propagate to the caller,
/// everything else marks the log damaged (the error split inside
/// replaySessionEventRecords, session_events.go 301-307).
#[derive(Debug)]
pub(crate) enum DecodeError {
    Limit(SessionReplayLimitError),
    Other(String),
}

/// Shared scratch-space helper for eventlog tests.
///
/// The workspace target directory is used instead of `env::temp_dir()`
/// because some test environments deny creating directories under the
/// system temp path, while the target directory is always writable
/// (same approach as the store::artifacts tests).
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};

    fn temp_base() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("target")
            .join("test-tmp")
    }

    pub(crate) fn tempdir() -> tempfile::TempDir {
        let base = temp_base();
        std::fs::create_dir_all(&base).expect("create test-tmp base");
        tempfile::Builder::new()
            .prefix(".tmpevlog")
            .tempdir_in(&base)
            .expect("create eventlog test temp dir")
    }
}
