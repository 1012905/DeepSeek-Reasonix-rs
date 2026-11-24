//! ACP-026 `session/prompt` turn flow decisions, inbox drain state machine, and
//! stop-reason mapping (`session.go:353-427`; `inbox_drain.go:15-47`).
//!
//! The real turn admission, slash resolution, goal-draft consumption, drift
//! reconciliation, persistence, and the controller-side `RunInboxTurn` are
//! runtime wiring; this module owns the block flattening, the admission refusal
//! identity, the drain FIFO/pause/refresh decisions, the stop-reason table,
//! and the vendor transcript meta shape.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::errors::RpcError;
use crate::extensions::unknown_session;
use crate::lifecycle::{active_prompt_conflict, empty_prompt};
use crate::wire::{ContentBlock, StopReason};

/// Byte clip for a turn-failure message (`status.go:570-576`). Secret redaction
/// is a runtime transform; this boundary trims whitespace and truncates bytes.
pub const TURN_ERROR_CLIP_BYTES: usize = 2_048;

/// Ordered deferred cleanup steps run when the turn finishes
/// (`session.go:386-390`): clear the turn context, reconcile drift and drain
/// queued work, then cancel the turn context.
pub const TURN_DEFER_STEPS: &[&str] = &["clear_turn_context", "finish_turn", "cancel_context"];

/// Ordered post-run steps before the prompt result is assembled
/// (`session.go:393-403`): finalize telemetry, publish status, send usage,
/// persist the transcript and snapshot.
pub const TURN_POST_STEPS: &[&str] = &[
    "finish_turn_status",
    "publish_status",
    "send_usage_update",
    "persist_after_turn",
];

/// Flattened prompt: the joined text plus how many blocks carried bytes that
/// only the runtime attachment store can resolve (`prompt_attachments.go:18-30`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PromptText {
    pub text: String,
    pub attachment_blocks: usize,
}

/// Extracts the text of one block. Image/resource blocks carrying bytes need
/// the session root to persist; this boundary counts them instead of saving
/// anything. Audio is not advertised and is ignored (`prompt_attachments.go:17`).
fn block_text(block: &ContentBlock) -> (Option<String>, bool) {
    match block.block_type.as_str() {
        "text" => (block.text.clone(), false),
        "resource" => match &block.resource {
            Some(resource) => {
                if let Some(text) = resource.text.clone().filter(|t| !t.is_empty()) {
                    (Some(text), false)
                } else if resource.blob.is_some() {
                    (None, true)
                } else {
                    (None, false)
                }
            }
            None => (None, false),
        },
        "image" => (None, block.data.is_some()),
        _ => (None, false),
    }
}

/// Joins the non-empty text parts with a blank line, then trims
/// (`prompt_attachments.go:29`).
pub fn flatten_prompt(blocks: &[ContentBlock]) -> PromptText {
    let mut parts = Vec::new();
    let mut attachment_blocks = 0;
    for block in blocks {
        let (text, attachment) = block_text(block);
        if attachment {
            attachment_blocks += 1;
        }
        if let Some(part) = text.filter(|t| !t.is_empty()) {
            parts.push(part);
        }
    }
    PromptText {
        text: parts.join("\n\n").trim().to_owned(),
        attachment_blocks,
    }
}

/// `-32602` block-level parse refusal (`prompt_attachments.go:23`).
pub fn block_parse_error(method: &str, index: usize, block_type: &str, inner: &str) -> RpcError {
    RpcError::invalid_params(format!(
        "{method}: prompt block {index} ({block_type}): {inner}"
    ))
}

/// Unknown session for `session/prompt` (`session.go:358-361`).
pub fn unknown_session_error(id: &str) -> RpcError {
    unknown_session("session/prompt", id)
}

/// Admission refusal identity shared by all four begin conditions: active
/// turn, deleted, maintenance in flight, or queued config
/// (`session.go:372-375`; `service.go:287-295`).
pub fn admission_refusal() -> RpcError {
    active_prompt_conflict("session/prompt")
}

/// Snapshot the `begin` preconditions. All four refusal branches share one
/// `-32600` identity (`service.go:287-295`): an in-flight turn, a deleted
/// session, maintenance in flight, or a non-empty pending config queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AdmissionGate {
    pub active: bool,
    pub deleted: bool,
    pub maintenance: bool,
    pub pending_config: bool,
}

/// Admits a new turn when none of the four refusal preconditions holds.
pub fn admit(gate: &AdmissionGate) -> Result<(), RpcError> {
    if gate.active || gate.deleted || gate.maintenance || gate.pending_config {
        return Err(admission_refusal());
    }
    Ok(())
}

/// Empty flattened text refusal (`session.go:366-368`).
pub fn empty_prompt_error() -> RpcError {
    empty_prompt("session/prompt")
}

/// Trims and byte-truncates a turn-failure message to the reference limit.
pub fn clip_turn_message(message: &str) -> String {
    let trimmed = message.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() <= TURN_ERROR_CLIP_BYTES {
        return trimmed.to_owned();
    }
    String::from_utf8_lossy(&bytes[..TURN_ERROR_CLIP_BYTES]).into_owned()
}

/// How the finished turn reports back (`session.go:405-418`). A nil run result
/// always lands on `end_turn` even if the context carries a residual signal; a
/// failed run whose context was cancelled reports `cancelled` as a normal
/// result; any other failure becomes the `-32603` turn-failed error frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptOutcome {
    EndTurn,
    Cancelled,
    TurnFailed,
}

pub fn classify_run(run_failed: bool, ctx_cancelled: bool) -> PromptOutcome {
    match (run_failed, ctx_cancelled) {
        (false, _) => PromptOutcome::EndTurn,
        (true, true) => PromptOutcome::Cancelled,
        (true, false) => PromptOutcome::TurnFailed,
    }
}

/// Stop-reason for a normal result frame; the turn-failed branch is an error
/// frame built by [`turn_failed_error`].
pub fn stop_reason(outcome: PromptOutcome) -> Option<StopReason> {
    match outcome {
        PromptOutcome::EndTurn => Some(StopReason::EndTurn),
        PromptOutcome::Cancelled => Some(StopReason::Cancelled),
        PromptOutcome::TurnFailed => None,
    }
}

/// Builds the `-32603` turn-failed error with `data.code = "turn_failed"` and a
/// clipped message (`protocol.go:37-44`; `session.go:412-416`).
pub fn turn_failed_error(message: &str) -> RpcError {
    crate::wire::turn_failed_error(clip_turn_message(message))
}

/// Vendor `_meta` block carrying the transcript path (`session.go:419-425`).
/// Omitted entirely when the session has no transcript pinned.
pub fn prompt_meta(transcript: &str) -> Option<BTreeMap<String, Value>> {
    if transcript.is_empty() {
        return None;
    }
    let mut vendor = BTreeMap::new();
    vendor.insert("transcriptPath".to_owned(), Value::from(transcript));
    let mut meta = BTreeMap::new();
    meta.insert(
        "reasonix.io".to_owned(),
        Value::Object(vendor.into_iter().collect()),
    );
    Some(meta)
}

// ---- Inbox drain -------------------------------------------------------------

/// One queued inbox item in a drain snapshot (`inbox_drain.go:17-22`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainItem {
    pub id: String,
    /// Inbox state; only `queued` items are eligible for the follow-up turn.
    pub state: String,
}

/// A drain snapshot read once per loop round.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DrainSnapshot {
    pub paused: bool,
    pub items: Vec<DrainItem>,
}

/// Next drain action for the current snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrainAction {
    /// Inbox paused, or no queued item: stop draining.
    Stop,
    /// Run the first queued item in snapshot order (FIFO).
    RunNext(String),
}

/// Selects the first queued item in snapshot order; a paused inbox stops
/// before scanning (`inbox_drain.go:25-32`).
pub fn next_drain_action(snapshot: &DrainSnapshot) -> DrainAction {
    if snapshot.paused {
        return DrainAction::Stop;
    }
    match snapshot.items.iter().find(|item| item.state == "queued") {
        Some(item) => DrainAction::RunNext(item.id.clone()),
        None => DrainAction::Stop,
    }
}

/// Classification of a `RunInboxTurn` failure (`inbox_drain.go:33-43`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrainRunError {
    /// Item vanished or left the queued state: another queue edit won; refresh
    /// the snapshot and continue.
    NotFound,
    InvalidState,
    /// Any other failure: propagate up and stop draining.
    Other(String),
}

/// One round's outcome, recorded by the loop driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrainRound {
    Ran(String),
    /// NotFound/InvalidState: snapshot refreshed, loop continues.
    Refreshed,
    Stopped,
    Propagated(String),
}

/// Classifies a runner error the way the drain loop reacts to it.
pub fn classify_drain_error(err: DrainRunError) -> DrainRound {
    match err {
        DrainRunError::NotFound | DrainRunError::InvalidState => DrainRound::Refreshed,
        DrainRunError::Other(message) => DrainRound::Propagated(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::RpcErrorCode;

    #[test]
    fn flatten_joins_text_parts_and_counts_attachments() {
        let blocks = vec![
            ContentBlock {
                block_type: "text".to_owned(),
                text: Some("first".to_owned()),
                ..Default::default()
            },
            ContentBlock {
                block_type: "resource".to_owned(),
                resource: Some(crate::wire::ResourceContents {
                    uri: "r".to_owned(),
                    text: Some("embedded".to_owned()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ContentBlock {
                block_type: "image".to_owned(),
                data: Some("base64".to_owned()),
                ..Default::default()
            },
        ];
        let flattened = flatten_prompt(&blocks);
        assert_eq!(flattened.text, "first\n\nembedded");
        assert_eq!(flattened.attachment_blocks, 1);
    }

    #[test]
    fn stop_mapping_follows_run_result_and_context() {
        assert_eq!(classify_run(false, false), PromptOutcome::EndTurn);
        assert_eq!(classify_run(false, true), PromptOutcome::EndTurn);
        assert_eq!(classify_run(true, true), PromptOutcome::Cancelled);
        assert_eq!(classify_run(true, false), PromptOutcome::TurnFailed);

        assert_eq!(
            stop_reason(PromptOutcome::EndTurn),
            Some(StopReason::EndTurn)
        );
        assert_eq!(
            stop_reason(PromptOutcome::Cancelled),
            Some(StopReason::Cancelled)
        );
        assert!(stop_reason(PromptOutcome::TurnFailed).is_none());
    }

    #[test]
    fn turn_failed_error_clips_to_2048_bytes_and_carries_data_code() {
        let long = "x".repeat(3_000);
        let err = turn_failed_error(&long);
        assert_eq!(err.code, RpcErrorCode::Internal);
        assert!(err.message.len() <= TURN_ERROR_CLIP_BYTES);
        assert_eq!(err.data.as_ref().unwrap()["code"], "turn_failed");
    }

    #[test]
    fn prompt_meta_omits_when_transcript_absent() {
        assert!(prompt_meta("").is_none());
        let meta = prompt_meta("/sessions/s1.jsonl").unwrap();
        assert_eq!(meta["reasonix.io"]["transcriptPath"], "/sessions/s1.jsonl");
    }

    #[test]
    fn drain_prefers_paused_then_first_queued_in_fifo_order() {
        let paused = DrainSnapshot {
            paused: true,
            items: vec![DrainItem {
                id: "i1".to_owned(),
                state: "queued".to_owned(),
            }],
        };
        assert_eq!(next_drain_action(&paused), DrainAction::Stop);

        let snapshot = DrainSnapshot {
            paused: false,
            items: vec![
                DrainItem {
                    id: "i1".to_owned(),
                    state: "done".to_owned(),
                },
                DrainItem {
                    id: "i2".to_owned(),
                    state: "queued".to_owned(),
                },
                DrainItem {
                    id: "i3".to_owned(),
                    state: "queued".to_owned(),
                },
            ],
        };
        assert_eq!(
            next_drain_action(&snapshot),
            DrainAction::RunNext("i2".to_owned())
        );

        let empty = DrainSnapshot::default();
        assert_eq!(next_drain_action(&empty), DrainAction::Stop);
    }

    #[test]
    fn drain_error_classification_refreshes_or_propagates() {
        assert_eq!(
            classify_drain_error(DrainRunError::NotFound),
            DrainRound::Refreshed
        );
        assert_eq!(
            classify_drain_error(DrainRunError::InvalidState),
            DrainRound::Refreshed
        );
        assert_eq!(
            classify_drain_error(DrainRunError::Other("boom".to_owned())),
            DrainRound::Propagated("boom".to_owned())
        );
    }

    #[test]
    fn begin_refuses_on_each_of_the_four_preconditions() {
        let clean = AdmissionGate::default();
        assert!(admit(&clean).is_ok());

        let active = AdmissionGate {
            active: true,
            ..Default::default()
        };
        let deleted = AdmissionGate {
            deleted: true,
            ..Default::default()
        };
        let maintenance = AdmissionGate {
            maintenance: true,
            ..Default::default()
        };
        let pending = AdmissionGate {
            pending_config: true,
            ..Default::default()
        };
        for gate in [active, deleted, maintenance, pending] {
            let err = admit(&gate).unwrap_err();
            assert_eq!(err.code, RpcErrorCode::InvalidRequest);
            assert_eq!(
                err.message,
                "session/prompt: session already has an active prompt"
            );
        }
    }

    #[test]
    fn prompt_error_templates_are_verbatim() {
        assert_eq!(
            unknown_session_error("s9").message,
            "session/prompt: unknown session s9"
        );
        assert_eq!(
            admission_refusal().message,
            "session/prompt: session already has an active prompt"
        );
        assert_eq!(empty_prompt_error().message, "session/prompt: empty prompt");
        assert_eq!(
            block_parse_error("session/prompt", 2, "image", "no data").message,
            "session/prompt: prompt block 2 (image): no data"
        );
    }
}
