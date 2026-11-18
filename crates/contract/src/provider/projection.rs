//! Provider-visible projections (Go: `projection.go`).
//!
//! Two copies of a transcript are derived from the stored one: the bytes a
//! provider receives, and the projection compaction writes back. They differ
//! in exactly one field, and that difference is load-bearing. Healthy
//! sessions without display-only records keep their original backing slice
//! (`Cow::Borrowed`), preserving the allocation and prompt-cache fast
//! path.

use std::borrow::Cow;

use crate::provider::message::Message;
use crate::provider::tool_call::ToolCall;
use crate::provider::video::{VIDEO_SHOWN_NOTE, answered_after, without_videos};

/// What a request says in place of images the host could not read back, so
/// the model is not left to guess why they are gone (Go:
/// `UnavailableImagesNote`).
pub const UNAVAILABLE_IMAGES_NOTE: &str = "\n[%d image(s) unavailable: the stored copy is missing]";

/// Renders [UNAVAILABLE_IMAGES_NOTE] for `n` missing images.
pub fn unavailable_images_note(n: usize) -> String {
    UNAVAILABLE_IMAGES_NOTE.replacen("%d", &n.to_string(), 1)
}

/// Removes durable display-only records before a request is handed to any
/// provider (Go: `ModelMessages`).
pub fn model_messages<'a>(msgs: &'a [Message]) -> Cow<'a, [Message]> {
    project_messages(msgs, false)
}

/// `model_messages` for a stored projection, except that host records
/// survive: `ToolExecution` because only it says a tool call failed,
/// and unavailable images because a stored copy may yet come back. A
/// projection is the next compaction's input, so both are stripped, or
/// rendered, at the provider boundary, which every request path already
/// crosses (Go: `ProjectionMessages`).
pub fn projection_messages<'a>(msgs: &'a [Message]) -> Cow<'a, [Message]> {
    project_messages(msgs, true)
}

fn project_messages<'a>(msgs: &'a [Message], keep_execution: bool) -> Cow<'a, [Message]> {
    let answered = video_answered(msgs, keep_execution);
    let needs_copy = msgs.iter().enumerate().any(|(i, m)| {
        answered
            .as_ref()
            .is_some_and(|a| !m.videos.is_empty() && a[i])
            || needs_projection(m, keep_execution)
    });
    if !needs_copy {
        return Cow::Borrowed(msgs);
    }
    let mut out = Vec::with_capacity(msgs.len());
    for (i, candidate) in msgs.iter().enumerate() {
        if candidate.local_only {
            continue;
        }
        let mut candidate = candidate.clone();
        if answered.as_ref().is_some_and(|a| a[i]) {
            candidate = without_videos(candidate, VIDEO_SHOWN_NOTE);
        }
        if !candidate.provider_content.is_empty() {
            candidate.content = std::mem::take(&mut candidate.provider_content);
        }
        candidate.raw_content = String::new();
        candidate.host_authored = false;
        candidate.model_ref = String::new();
        candidate.via = None;
        candidate.decision_receipt = None;
        candidate.decision_receipts = Vec::new();
        if !keep_execution {
            candidate.agent_composition = None;
            candidate.tool_calls = without_argument_normalization(candidate.tool_calls);
            // Local shell metadata must never enter provider request bytes.
            candidate.tool_execution = None;
            candidate.tool_failure = None;
            let n = candidate.unavailable_images.len();
            if n > 0 {
                candidate.content.push_str(&unavailable_images_note(n));
                candidate.unavailable_images = Vec::new();
            }
        }
        out.push(candidate);
    }
    Cow::Owned(out)
}

/// Reports a host-only field this message carries (Go: `needsProjection`).
fn needs_projection(m: &Message, keep_execution: bool) -> bool {
    if (has_argument_normalization(&m.tool_calls)
        || m.agent_composition.is_some()
        || !m.unavailable_images.is_empty()
        || m.tool_execution.is_some()
        || m.tool_failure.is_some())
        && !keep_execution
    {
        return true;
    }
    m.local_only
        || !m.raw_content.is_empty()
        || !m.provider_content.is_empty()
        || m.host_authored
        || !m.model_ref.is_empty()
        || m.via.is_some()
        || m.decision_receipt.is_some()
        || !m.decision_receipts.is_empty()
}

/// Says, per message, whether the model has answered since it; `None`
/// when no message carries a video or the projection is a stored one that
/// keeps the references (Go: `videoAnswered`).
fn video_answered(msgs: &[Message], keep_execution: bool) -> Option<Vec<bool>> {
    if keep_execution {
        return None;
    }
    if msgs.iter().any(|m| !m.videos.is_empty()) {
        return Some(answered_after(msgs));
    }
    None
}

/// Whether any call carries argument-normalization evidence (Go:
/// `hasArgumentNormalization`).
fn has_argument_normalization(calls: &[ToolCall]) -> bool {
    calls.iter().any(|c| c.argument_normalization.is_some())
}

/// Strips argument-normalization evidence, cloning only when present (Go:
/// `withoutArgumentNormalization`).
fn without_argument_normalization(calls: Vec<ToolCall>) -> Vec<ToolCall> {
    if !has_argument_normalization(&calls) {
        return calls;
    }
    let mut out = calls;
    for call in &mut out {
        call.argument_normalization = None;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agentcomposition::Binding;
    use crate::provider::Role;
    use crate::provider::local_metadata::{
        DecisionReceipt, ToolExecution, ToolFailure, UnavailableImage, Via,
    };
    use crate::provider::tool_call::{ARGUMENT_ENVELOPE_UNWRAPPED, ArgumentNormalization};
    use crate::provider::video::VideoRef;

    fn clean(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn clean_transcripts_borrow_the_input() {
        let msgs = vec![clean(Role::USER, "hello"), clean(Role::ASSISTANT, "hi")];
        assert!(matches!(model_messages(&msgs), Cow::Borrowed(_)));
        assert!(matches!(projection_messages(&msgs), Cow::Borrowed(_)));
    }

    #[test]
    fn model_messages_strips_host_only_fields() {
        let mut m = clean(Role::USER, "shown");
        m.raw_content = "typed".to_string();
        m.provider_content = "shown".to_string();
        m.host_authored = true;
        m.model_ref = "m".to_string();
        m.via = Some(Via::default());
        m.decision_receipt = Some(DecisionReceipt::default());
        m.decision_receipts = vec![Some(DecisionReceipt::default())];
        m.agent_composition = Some(Binding::default());
        let input = [m];
        let out = model_messages(&input);
        let Cow::Owned(out) = out else {
            panic!("expected a projected copy");
        };
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].content, "shown");
        assert!(out[0].raw_content.is_empty());
        assert!(out[0].provider_content.is_empty());
        assert!(!out[0].host_authored);
        assert!(out[0].model_ref.is_empty());
        assert!(out[0].via.is_none());
        assert!(out[0].decision_receipt.is_none());
        assert!(out[0].decision_receipts.is_empty());
        assert!(out[0].agent_composition.is_none());
    }

    #[test]
    fn provider_content_replaces_the_stored_text() {
        let mut m = clean(Role::USER, "legacy");
        m.provider_content = "contextual".to_string();
        let input = [m];
        let out = model_messages(&input);
        assert_eq!(out[0].content, "contextual");
        assert!(out[0].provider_content.is_empty());
    }

    #[test]
    fn local_only_messages_never_reach_consumers() {
        let mut secret = clean(Role::USER, "secret");
        secret.local_only = true;
        let msgs = vec![clean(Role::USER, "a"), secret, clean(Role::ASSISTANT, "b")];
        for out in [model_messages(&msgs), projection_messages(&msgs)] {
            assert_eq!(out.len(), 2);
            assert_eq!(out[0].content, "a");
            assert_eq!(out[1].content, "b");
        }
    }

    #[test]
    fn unavailable_images_render_a_note() {
        let mut m = clean(Role::TOOL, "");
        m.unavailable_images = vec![UnavailableImage::default(), UnavailableImage::default()];
        let input = [m];
        let out = model_messages(&input);
        assert_eq!(
            out[0].content,
            "\n[2 image(s) unavailable: the stored copy is missing]"
        );
        assert!(out[0].unavailable_images.is_empty());
        // Stored projections keep the records for the next compaction.
        let mut stored = clean(Role::TOOL, "");
        stored.unavailable_images = vec![UnavailableImage::default()];
        assert!(matches!(projection_messages(&[stored]), Cow::Borrowed(_)));
        assert_eq!(
            unavailable_images_note(2),
            UNAVAILABLE_IMAGES_NOTE.replacen("%d", "2", 1)
        );
    }

    #[test]
    fn answered_videos_become_notes_and_unanswered_stay() {
        let mut ask = clean(Role::USER, "watch");
        ask.videos = vec![VideoRef {
            name: "demo.mp4".to_string(),
            ..Default::default()
        }];
        let answer = clean(Role::ASSISTANT, "seen it");
        let input = [ask.clone(), answer.clone()];
        let out = model_messages(&input);
        assert_eq!(
            out[0].content,
            "watch\n[video demo.mp4 was shown to you when it was read and is not attached again; call read_video to look at it again]"
        );
        assert!(out[0].videos.is_empty());
        assert_eq!(out[1].content, "seen it");
        // No assistant answer yet: the video is still attached and the slice
        // is borrowed untouched.
        assert!(matches!(model_messages(&[ask.clone()]), Cow::Borrowed(_)));
        // Stored projections keep video references for the next compaction.
        assert!(matches!(
            projection_messages(&[ask, answer]),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn projection_messages_keep_execution_metadata() {
        let mut m = clean(Role::TOOL, "result");
        m.tool_execution = Some(ToolExecution::default());
        m.tool_failure = Some(ToolFailure::default());
        let input = [m.clone()];
        let out = projection_messages(&input);
        assert!(out[0].tool_execution.is_some());
        assert!(out[0].tool_failure.is_some());
        // The provider boundary strips local shell metadata.
        let input = [m];
        let out = model_messages(&input);
        assert!(out[0].tool_execution.is_none());
        assert!(out[0].tool_failure.is_none());
    }

    #[test]
    fn argument_normalization_is_stripped_only_when_present() {
        let mut m = clean(Role::ASSISTANT, "");
        m.tool_calls = vec![ToolCall {
            id: "c1".to_string(),
            arguments: "{}".to_string(),
            argument_normalization: Some(ArgumentNormalization {
                reason: ARGUMENT_ENVELOPE_UNWRAPPED.to_string(),
                original: "{}".to_string(),
                effective: "{\"a\":1}".to_string(),
            }),
            ..Default::default()
        }];
        let input = [m.clone()];
        let out = model_messages(&input);
        assert_eq!(out[0].tool_calls[0].id, "c1");
        assert!(out[0].tool_calls[0].argument_normalization.is_none());
        // Host records survive in stored projections.
        assert!(matches!(projection_messages(&[m]), Cow::Borrowed(_)));

        // Without evidence the calls pass through untouched.
        let calls = vec![ToolCall::default()];
        assert_eq!(without_argument_normalization(calls.clone()), calls);
    }
}
