//! TOOL-046 artifact timeline, in-memory tail ring and persisted meta shape
//! (Go `internal/tools/jobs/artifacts.go`).
//!
//! Done here: the `jobTimes` record, the `artifactMeta` JSON shape, the bounded
//! tail ring, the persisted-artifact completeness decision and the on-disk
//! constants. Real file creation, the 0700/0600 permission tightening, the
//! temp-file + rename meta write, incremental offset reads and directory
//! migration are host machinery this slice's decision tables assume; see the
//! WIP notes on [`ArtifactMeta`] and the slice report.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// `jobLogExt` — the per-job stream artifact suffix.
pub const JOB_LOG_EXT: &str = ".log";
/// `jobMetaExt` — the per-job metadata artifact suffix.
pub const JOB_META_EXT: &str = ".json";
/// `defaultTailBytes` — the in-memory tail retained until the log survives on
/// disk. 64 KiB.
pub const DEFAULT_TAIL_BYTES: usize = 64 << 10;
/// `mutationEvidenceVersion` written alongside a task job's recorded receipts.
pub const MUTATION_EVIDENCE_VERSION: u64 = 1;
/// `recoveredBackgroundTaskToolName` stamped on the synthetic receipt that
/// stands in for receipts a recovered task job used to carry.
pub const RECOVERED_BACKGROUND_TOOL_NAME: &str = "background_task_recovery";

/// `jobTimes`: started/activity/finished are unix milliseconds; `steps` is
/// the count the runner reported. Activity is read by stall detection and
/// cannot be derived from the other two.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JobTimes {
    pub started: i64,
    pub activity: i64,
    pub finished: i64,
    pub steps: i64,
}

impl JobTimes {
    /// `touch` — refresh the activity timestamp.
    pub fn touch(&mut self, now_ms: i64) {
        self.activity = now_ms;
    }
}

/// `artifactMeta` — the JSON persisted beside the job log. Field names and
/// omitempty sets mirror `artifacts.go`; the on-disk write itself is WIP.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArtifactMeta {
    pub id: String,
    pub kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
    #[serde(rename = "sessionId", skip_serializing_if = "String::is_empty")]
    pub session_id: String,
    #[serde(rename = "ownerId", skip_serializing_if = "String::is_empty")]
    pub owner_id: String,
    pub status: String,
    #[serde(rename = "startedAt")]
    pub started_at: i64,
    #[serde(rename = "finishedAt", skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    #[serde(rename = "artifactComplete")]
    pub artifact_complete: bool,
    #[serde(rename = "artifactError", skip_serializing_if = "String::is_empty")]
    pub artifact_error: String,
    #[serde(rename = "logPath", skip_serializing_if = "String::is_empty")]
    pub log_path: String,
    #[serde(
        rename = "mutationEvidenceVersion",
        skip_serializing_if = "Option::is_none"
    )]
    pub mutation_evidence_version: Option<u64>,
    #[serde(rename = "mutationEvidence", skip_serializing_if = "Option::is_none")]
    pub mutation_evidence: Option<MetaMutationEvidence>,
}

/// The mutation-risk summary persisted with a task job's meta. Risk levels
/// reuse the safety evidence vocabulary; this slice records the strings the
/// jobs package reads back.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MetaMutationEvidence {
    pub risk: String,
    pub paths: Vec<String>,
}

/// `appendTail`: keep the final `limit` bytes of the combined buffer.
///
/// `limit <= 0` clears the buffer (Go returns nil); a fresh write at least as
/// large as the limit replaces the ring with only that write's tail; otherwise
/// the combined buffer is trimmed to its last `limit` bytes.
pub fn append_tail(buf: &mut Vec<u8>, incoming: &[u8], limit: usize) {
    if limit == 0 {
        buf.clear();
        return;
    }
    if incoming.len() >= limit {
        buf.clear();
        buf.extend_from_slice(&incoming[incoming.len() - limit..]);
        return;
    }
    buf.extend_from_slice(incoming);
    if buf.len() > limit {
        let drain = buf.len() - limit;
        buf.drain(..drain);
    }
}

/// Whether the surviving result reference may point at the log artifact: the
/// artifact completed cleanly and a path was recorded (`artifactRefLocked`).
pub fn persisted_artifact_complete(complete: bool, path: &str) -> bool {
    complete && !path.is_empty()
}

/// Milliseconds elapsed since a timestamp; `ms <= 0` reads as 0
/// (`sinceMs`).
pub fn since_ms(origin_ms: i64, now_ms: i64) -> i64 {
    if origin_ms <= 0 {
        0
    } else {
        (now_ms - origin_ms).max(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_tail_drops_everything_when_limit_is_zero() {
        let mut buf = vec![1, 2, 3];
        append_tail(&mut buf, &[4, 5], 0);
        assert!(buf.is_empty());
    }

    #[test]
    fn a_write_at_least_as_long_as_the_limit_replaces_the_ring() {
        let mut buf = vec![9, 9, 9];
        append_tail(&mut buf, b"abcdef", 4);
        assert_eq!(buf, b"cdef");
    }

    #[test]
    fn combined_buffer_keeps_only_the_final_limit_bytes() {
        let mut buf = b"hello ".to_vec();
        append_tail(&mut buf, b"world", 8);
        assert_eq!(buf, b"lo world");
    }

    #[test]
    fn one_byte_overflows_the_limit_by_dropping_the_oldest_prefix_byte() {
        let mut buf = vec![7u8; 4];
        append_tail(&mut buf, &[1, 2], 5);
        assert_eq!(buf, vec![7, 7, 7, 1, 2]);
    }

    #[test]
    fn artifact_ref_needs_both_completion_and_path() {
        assert!(!persisted_artifact_complete(false, "/x/job.log"));
        assert!(!persisted_artifact_complete(true, ""));
        assert!(persisted_artifact_complete(true, "/x/job.log"));
    }

    #[test]
    fn since_ms_clamps_unknown_origin_to_zero() {
        assert_eq!(since_ms(0, 12_000), 0);
        assert_eq!(since_ms(5_000, 5_000), 0);
        assert_eq!(since_ms(5_000, 9_000), 4_000);
    }
}
