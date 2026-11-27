use std::path::PathBuf;

use crate::sessionstore::eventlog::record::{CreatedAt, deserialize_created_at};
pub use reasonix_core::fileencoding::Kind;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const SCHEMA_V1: i64 = 1;
pub const SCHEMA_V2: i64 = 2;
pub const DEFAULT_RETAIN_CHECKPOINTS: usize = 100;
pub const DEFAULT_BLOB_QUOTA_BYTES: u64 = 1 << 30;
pub const DEFAULT_MAX_FILE_BYTES: u64 = 32 << 20;

pub const DISABLED_LEGACY: &str = "legacy checkpoint cannot verify later manual edits";
pub const DISABLED_EXPIRED: &str = "file recovery payload expired";
pub const EXPIRED_DETAIL: &str = "file recovery payload expired";

pub const GAP_BASH_SIDE_EFFECT: &str = "bash_side_effect";
pub const GAP_HOOK_WRITE: &str = "hook_write";
pub const GAP_MCP_EXTERNAL: &str = "mcp_external";
pub const GAP_OUTSIDE_WORKSPACE: &str = "outside_workspace";
pub const GAP_SYMLINK: &str = "symlink";
pub const GAP_HARDLINK: &str = "hardlink";
pub const GAP_UNREADABLE: &str = "unreadable";
pub const GAP_OVERSIZED: &str = "oversized";
pub const GAP_BACKGROUND_WRITER: &str = "background_writer_cross_turn";
pub const GAP_WORKSPACE_CLAIM_YIELDED: &str = "workspace_claim_yielded";
pub const GAP_LEGACY_UNVERIFIED: &str = "legacy_unverified";
pub const GAP_CAPTURE_FAILED: &str = "capture_failed";
pub const GAP_EXPIRED_PAYLOAD: &str = "expired_file_payload";

pub const CONFLICT_MANUAL_EDIT: &str = "manual_edit";
pub const CONFLICT_EXTERNAL_CHANGE: &str = "external_change";
pub const CONFLICT_DELETED_RECREATE: &str = "deleted_and_recreated";
pub const CONFLICT_TYPE_CHANGE: &str = "type_change";
pub const CONFLICT_MODE_CHANGE: &str = "mode_change";
pub const CONFLICT_MISSING_PAYLOAD: &str = "missing_payload";
pub const CONFLICT_PATH_UNSAFE: &str = "path_unsafe";
pub const CONFLICT_BUSY_WRITER: &str = "active_writer";
pub const CONFLICT_STALE_PLAN: &str = "stale_plan";
pub const CONFLICT_BOUNDARY_INVALID: &str = "boundary_invalid";
pub const CONFLICT_COVERAGE_LEGACY: &str = "legacy_unverified";
pub const CONFLICT_EXPIRED: &str = "expired_payload";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    #[serde(rename = "")]
    #[default]
    Unspecified,
    Complete,
    Partial,
    None,
    Legacy,
}

fn is_unspecified_coverage(value: &Coverage) -> bool {
    matches!(value, Coverage::Unspecified)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSource {
    Previewer,
    BeforeMutation,
    AfterMutation,
    Legacy,
    Manual,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageGap {
    pub reason: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
}

impl CoverageGap {
    pub fn new(reason: &str, detail: &str) -> Self {
        Self {
            reason: reason.to_owned(),
            detail: detail.to_owned(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureBeforeOpts {
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub source: Option<CaptureSource>,
    #[serde(default)]
    pub writer_id: Option<String>,
    #[serde(default)]
    pub ownership_turn: Option<i64>,
    #[serde(default)]
    pub background: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureAfterOpts {
    #[serde(default)]
    pub seq: i64,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub source: Option<CaptureSource>,
    #[serde(default)]
    pub writer_id: Option<String>,
    #[serde(default)]
    pub ownership_turn: Option<i64>,
    #[serde(default)]
    pub background: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureOptions {
    #[serde(default)]
    pub max_bytes: u64,
    #[serde(default)]
    pub read_content: bool,
    #[serde(default)]
    pub workspace_root: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSnap {
    pub path: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "encoding_wire::serialize_option",
        deserialize_with = "encoding_wire::deserialize_option"
    )]
    pub encoding: Option<Kind>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub mode: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha256: String,
    #[serde(default, rename = "blobRef", skip_serializing_if = "String::is_empty")]
    pub blob_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_source: Option<CaptureSource>,
    #[serde(
        default,
        rename = "afterSha256",
        skip_serializing_if = "String::is_empty"
    )]
    pub after_sha256: String,
    #[serde(
        default,
        rename = "afterExisted",
        skip_serializing_if = "Option::is_none"
    )]
    pub after_existed: Option<bool>,
    #[serde(default, rename = "afterMode", skip_serializing_if = "is_zero_u32")]
    pub after_mode: u32,
    #[serde(default, rename = "payloadExpired", skip_serializing_if = "is_false")]
    pub payload_expired: bool,
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRevision {
    pub path: String,
    pub existed: bool,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub mode: u32,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "encoding_wire::serialize_option",
        deserialize_with = "encoding_wire::deserialize_option"
    )]
    pub encoding: Option<Kind>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha256: String,
    #[serde(default, rename = "blobRef", skip_serializing_if = "String::is_empty")]
    pub blob_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_source: Option<CaptureSource>,
    #[serde(
        default,
        rename = "afterSha256",
        skip_serializing_if = "String::is_empty"
    )]
    pub after_sha256: String,
    #[serde(
        default,
        rename = "afterExisted",
        skip_serializing_if = "Option::is_none"
    )]
    pub after_existed: Option<bool>,
    #[serde(default, rename = "afterMode", skip_serializing_if = "is_zero_u32")]
    pub after_mode: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Checkpoint {
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub schema_version: i64,
    pub turn: i64,
    #[serde(default, deserialize_with = "deserialize_created_at")]
    pub time: CreatedAt,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub msg_index: i64,
    #[serde(
        default,
        rename = "sessionId",
        skip_serializing_if = "String::is_empty"
    )]
    pub session_id: String,
    #[serde(default)]
    pub files: Vec<FileSnap>,
    #[serde(default, skip_serializing_if = "is_unspecified_coverage")]
    pub coverage: Coverage,
    #[serde(
        default,
        rename = "coverageGaps",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub coverage_gaps: Vec<CoverageGap>,
    #[serde(
        default,
        rename = "activeWriters",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub active_writers: Vec<ActiveWriter>,
    #[serde(
        default,
        rename = "lastMutationSeq",
        skip_serializing_if = "is_zero_i64"
    )]
    pub last_mutation_seq: i64,
    #[serde(
        default,
        rename = "sessionRevision",
        skip_serializing_if = "is_zero_i64"
    )]
    pub session_revision: i64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub legacy: bool,
    #[serde(
        default,
        rename = "expiredFilePayload",
        skip_serializing_if = "is_false"
    )]
    pub expired_file_payload: bool,
    #[serde(default, rename = "forkCopied", skip_serializing_if = "is_false")]
    pub fork_copied: bool,
}

fn is_zero_i64(value: &i64) -> bool {
    *value == 0
}

impl Checkpoint {
    pub fn recompute_coverage(&mut self) {
        self.coverage = if self.legacy || self.schema_version < SCHEMA_V2 {
            Coverage::Legacy
        } else if self.expired_file_payload {
            Coverage::Partial
        } else if self.files.is_empty() {
            Coverage::None
        } else if self.coverage_gaps.is_empty() {
            Coverage::Complete
        } else {
            Coverage::Partial
        };
    }

    pub fn revisions(&self) -> Vec<FileRevision> {
        self.files
            .iter()
            .map(|file| {
                let mut revision = FileRevision {
                    path: file.path.clone(),
                    existed: file.content.is_some()
                        || !file.blob_ref.is_empty()
                        || !file.sha256.is_empty(),
                    mode: file.mode,
                    encoding: file.encoding,
                    sha256: file.sha256.clone(),
                    blob_ref: file.blob_ref.clone(),
                    capture_source: file.capture_source,
                    after_sha256: file.after_sha256.clone(),
                    after_existed: file.after_existed,
                    after_mode: file.after_mode,
                    content: file.content.clone(),
                };
                if let Some(content) = &file.content {
                    revision.existed = true;
                    if revision.sha256.is_empty() {
                        revision.sha256 = super::blob::digest(content.as_bytes());
                    }
                }
                if file.payload_expired {
                    revision.blob_ref.clear();
                    revision.content = None;
                }
                revision
            })
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileState {
    pub content: Option<String>,
    pub encoding: Option<Kind>,
    pub mode: u32,
    pub sha256: String,
    pub blob_ref: String,
    pub owned: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meta {
    pub turn: i64,
    pub msg_index: i64,
    pub time: CreatedAt,
    pub prompt: String,
    pub paths: Vec<String>,
    pub rewind_files: usize,
    pub coverage: Coverage,
    pub coverage_gaps: Vec<CoverageGap>,
    pub expired_file_payload: bool,
    pub active_writers: Vec<ActiveWriter>,
    pub fork_copied: bool,
    pub legacy: bool,
    pub disabled_reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationRecord {
    pub seq: i64,
    pub path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<CaptureSource>,
    #[serde(default, rename = "writerId", skip_serializing_if = "String::is_empty")]
    pub writer_id: String,
    pub turn: i64,
    #[serde(
        default,
        rename = "beforeSha",
        skip_serializing_if = "String::is_empty"
    )]
    pub before_sha: String,
    #[serde(default, rename = "afterSha", skip_serializing_if = "String::is_empty")]
    pub after_sha: String,
    #[serde(default, deserialize_with = "deserialize_created_at")]
    pub time: CreatedAt,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveWriter {
    pub id: String,
    pub turn: i64,
    #[serde(
        default,
        rename = "startedAt",
        deserialize_with = "deserialize_created_at"
    )]
    pub started_at: CreatedAt,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RewindScope {
    #[default]
    Code,
    Conversation,
    Both,
}

impl RewindScope {
    pub const fn value(self) -> u8 {
        match self {
            Self::Code => 0,
            Self::Conversation => 1,
            Self::Both => 2,
        }
    }
}

impl Serialize for RewindScope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u8(self.value())
    }
}

impl<'de> Deserialize<'de> for RewindScope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match u8::deserialize(deserializer)? {
            0 => Ok(Self::Code),
            1 => Ok(Self::Conversation),
            2 => Ok(Self::Both),
            _ => Err(serde::de::Error::custom("unknown rewind scope")),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RewindConflict {
    pub path: String,
    pub reason: String,
    #[serde(
        default,
        rename = "checkpointSha",
        skip_serializing_if = "String::is_empty"
    )]
    pub checkpoint_sha: String,
    #[serde(
        default,
        rename = "lastOwnedSha",
        skip_serializing_if = "String::is_empty"
    )]
    pub last_owned_sha: String,
    #[serde(
        default,
        rename = "currentSha",
        skip_serializing_if = "String::is_empty"
    )]
    pub current_sha: String,
    #[serde(
        default,
        rename = "checkpointMode",
        skip_serializing_if = "is_zero_u32"
    )]
    pub checkpoint_mode: u32,
    #[serde(default, rename = "currentMode", skip_serializing_if = "is_zero_u32")]
    pub current_mode: u32,
    #[serde(default, rename = "currentExisted")]
    pub current_existed: bool,
    #[serde(default, rename = "checkpointExisted")]
    pub checkpoint_existed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStage {
    pub path: String,
    pub phase: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub action: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub compensated: bool,
    #[serde(
        default,
        rename = "compensateError",
        skip_serializing_if = "String::is_empty"
    )]
    pub comp_error: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RewindPlan {
    #[serde(rename = "planId")]
    pub plan_id: String,
    pub turn: i64,
    pub scope: RewindScope,
    pub coverage: Coverage,
    #[serde(rename = "coverageGaps", skip_serializing_if = "Vec::is_empty")]
    pub coverage_gaps: Vec<CoverageGap>,
    #[serde(skip_serializing_if = "is_false")]
    pub legacy: bool,
    #[serde(rename = "expiredFilePayload", skip_serializing_if = "is_false")]
    pub expired_file_payload: bool,
    #[serde(rename = "canFiles")]
    pub can_files: bool,
    #[serde(rename = "canConversation")]
    pub can_conversation: bool,
    #[serde(rename = "disabledReason", skip_serializing_if = "String::is_empty")]
    pub disabled_reason: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<RewindConflict>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(rename = "fileCount")]
    pub file_count: usize,
    #[serde(rename = "activeWriters", skip_serializing_if = "Vec::is_empty")]
    pub active_writers: Vec<ActiveWriter>,
    #[serde(rename = "sessionRevision")]
    pub session_revision: i64,
    #[serde(rename = "workspaceToken", skip_serializing_if = "String::is_empty")]
    pub workspace_token: String,
    #[serde(rename = "boundaryIndex", skip_serializing_if = "is_zero_i64")]
    pub boundary_index: i64,
    #[serde(rename = "hasBoundary")]
    pub has_boundary: bool,
    #[serde(rename = "createdAt", deserialize_with = "deserialize_created_at")]
    pub created_at: CreatedAt,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub path: String,
    #[serde(
        rename = "conflictResolution",
        skip_serializing_if = "String::is_empty"
    )]
    pub conflict_resolution: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RewindUndo {
    #[serde(rename = "transactionId")]
    pub transaction_id: String,
    pub turn: i64,
    pub files: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RewindResult {
    pub ok: bool,
    #[serde(
        default,
        rename = "transactionId",
        skip_serializing_if = "String::is_empty"
    )]
    pub transaction_id: String,
    #[serde(default, rename = "undoAvailable")]
    pub undo_available: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub written: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deleted: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FileStage>,
    #[serde(default, rename = "conversationOk", skip_serializing_if = "is_false")]
    pub conversation_ok: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<RewindConflict>,
    #[serde(default, skip_serializing_if = "is_unspecified_coverage")]
    pub coverage: Coverage,
    #[serde(
        default,
        rename = "coverageGaps",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub coverage_gaps: Vec<CoverageGap>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolution {
    #[default]
    KeepCurrent,
    OverwriteCheckpoint,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionState {
    #[default]
    Prepared,
    Committing,
    Committed,
    Aborted,
    Undone,
    Invalidated,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TransactionTarget {
    pub path: String,
    #[serde(rename = "absPath")]
    pub abs_path: String,
    #[serde(rename = "restoreExisted")]
    pub restore_existed: bool,
    #[serde(rename = "restoreMode", skip_serializing_if = "is_zero_u32")]
    pub restore_mode: u32,
    #[serde(rename = "restoreSha", skip_serializing_if = "String::is_empty")]
    pub restore_sha: String,
    #[serde(rename = "restoreBlob", skip_serializing_if = "String::is_empty")]
    pub restore_blob: String,
    #[serde(
        default,
        rename = "restoreInline",
        skip_serializing_if = "Option::is_none",
        serialize_with = "bytes_wire::serialize_option",
        deserialize_with = "bytes_wire::deserialize_option"
    )]
    pub restore_inline: Option<Vec<u8>>,
    #[serde(
        rename = "restoreEncoding",
        skip_serializing_if = "Option::is_none",
        serialize_with = "encoding_wire::serialize_option",
        deserialize_with = "encoding_wire::deserialize_option"
    )]
    pub restore_encoding: Option<Kind>,
    #[serde(rename = "forwardExisted")]
    pub forward_existed: bool,
    #[serde(rename = "forwardMode", skip_serializing_if = "is_zero_u32")]
    pub forward_mode: u32,
    #[serde(rename = "forwardSha", skip_serializing_if = "String::is_empty")]
    pub forward_sha: String,
    #[serde(rename = "forwardBlob", skip_serializing_if = "String::is_empty")]
    pub forward_blob: String,
    #[serde(
        default,
        rename = "forwardInline",
        skip_serializing_if = "Option::is_none",
        serialize_with = "bytes_wire::serialize_option",
        deserialize_with = "bytes_wire::deserialize_option"
    )]
    pub forward_inline: Option<Vec<u8>>,
    #[serde(rename = "publishTmp", skip_serializing_if = "String::is_empty")]
    pub publish_tmp: String,
    #[serde(rename = "backupPath", skip_serializing_if = "String::is_empty")]
    pub backup_path: String,
    pub published: bool,
    pub action: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TransactionManifest {
    #[serde(rename = "schemaVersion")]
    pub schema_version: i64,
    pub id: String,
    #[serde(rename = "sessionId", skip_serializing_if = "String::is_empty")]
    pub session_id: String,
    #[serde(rename = "workspaceRoot")]
    pub workspace_root: String,
    pub state: TransactionState,
    pub kind: String,
    pub turn: i64,
    pub scope: RewindScope,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub path: String,
    #[serde(rename = "createdAt", deserialize_with = "deserialize_created_at")]
    pub created_at: CreatedAt,
    #[serde(rename = "updatedAt", deserialize_with = "deserialize_created_at")]
    pub updated_at: CreatedAt,
    #[serde(rename = "sessionRevision")]
    pub session_revision: i64,
    #[serde(rename = "workspaceToken", skip_serializing_if = "String::is_empty")]
    pub workspace_token: String,
    #[serde(default, skip_serializing_if = "is_unspecified_coverage")]
    pub coverage: Coverage,
    #[serde(rename = "coverageGaps", skip_serializing_if = "Vec::is_empty")]
    pub coverage_gaps: Vec<CoverageGap>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<TransactionTarget>,
    #[serde(
        default,
        rename = "conversationForward",
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "bytes_wire::serialize",
        deserialize_with = "bytes_wire::deserialize"
    )]
    pub conversation_forward: Vec<u8>,
    #[serde(default, rename = "boundaryIndex", skip_serializing_if = "is_zero_i64")]
    pub boundary_index: i64,
    #[serde(default, rename = "hasBoundary")]
    pub has_boundary: bool,
    #[serde(default, rename = "truncateFrom", skip_serializing_if = "is_zero_i64")]
    pub truncate_from: i64,
    #[serde(
        default,
        rename = "checkpointBackup",
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "bytes_wire::serialize",
        deserialize_with = "bytes_wire::deserialize"
    )]
    pub checkpoint_backup: Vec<u8>,
    #[serde(
        default,
        rename = "parentTransaction",
        skip_serializing_if = "String::is_empty"
    )]
    pub parent_transaction: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewChange {
    pub path: String,
    pub kind: ChangeKind,
    #[serde(default, rename = "oldText")]
    pub old_text: String,
    #[serde(default, rename = "newText")]
    pub new_text: String,
    #[serde(default)]
    pub added: i64,
    #[serde(default)]
    pub removed: i64,
    #[serde(default)]
    pub diff: String,
    #[serde(default)]
    pub binary: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mode: String,
    #[serde(default)]
    pub hunks: i64,
}

impl Default for PreviewChange {
    fn default() -> Self {
        Self {
            path: String::new(),
            kind: ChangeKind::Modify,
            old_text: String::new(),
            new_text: String::new(),
            added: 0,
            removed: 0,
            diff: String::new(),
            binary: false,
            mode: String::new(),
            hunks: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Create,
    #[default]
    Modify,
    Delete,
}

mod bytes_wire {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let Some(value) = Option::<String>::deserialize(deserializer)? else {
            return Ok(Vec::new());
        };
        base64::engine::general_purpose::STANDARD
            .decode(value)
            .map_err(serde::de::Error::custom)
    }

    pub fn serialize_option<S>(bytes: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match bytes {
            Some(bytes) => serialize(bytes, serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize_option<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<String>::deserialize(deserializer)?
            .map(|value| {
                base64::engine::general_purpose::STANDARD
                    .decode(value)
                    .map_err(serde::de::Error::custom)
            })
            .transpose()
    }
}

mod encoding_wire {
    use super::{Deserialize, Deserializer, Kind, Serializer};
    use serde::de::Error as _;

    pub fn serialize<S>(kind: &Kind, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u8(*kind as u8)
    }

    fn decode(value: u8) -> Result<Kind, &'static str> {
        match value {
            0 => Ok(Kind::Utf8),
            1 => Ok(Kind::Utf8Bom),
            2 => Ok(Kind::Utf16Le),
            3 => Ok(Kind::Utf16Be),
            4 => Ok(Kind::Gb18030),
            5 => Ok(Kind::LossyUtf8),
            6 => Ok(Kind::Utf16LeNoBom),
            7 => Ok(Kind::Utf16BeNoBom),
            8 => Ok(Kind::Gbk),
            _ => Err("unknown file encoding kind"),
        }
    }

    pub fn serialize_option<S>(kind: &Option<Kind>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match kind {
            Some(kind) => serialize(kind, serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize_option<'de, D>(deserializer: D) -> Result<Option<Kind>, D::Error>
    where
        D: Deserializer<'de>,
    {
        match Option::<u8>::deserialize(deserializer)? {
            Some(value) => decode(value).map(Some).map_err(D::Error::custom),
            None => Ok(None),
        }
    }
}

pub fn transaction_dir(checkpoint_dir: &std::path::Path) -> PathBuf {
    checkpoint_dir.join("transactions")
}
