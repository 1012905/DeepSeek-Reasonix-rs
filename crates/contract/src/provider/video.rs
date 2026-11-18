//! Video references (Go: `video.go`).

use crate::gojson::{go_struct, is_zero_i64, null_default};
use crate::provider::Role;
use crate::provider::message::Message;

/// Appended note for a video the model already saw (Go: `VideoShownNote`).
pub const VIDEO_SHOWN_NOTE: &str = "\n[video %s was shown to you when it was read and is not attached again; call read_video to look at it again]";

/// Appended note for a video this model cannot take natively (Go:
/// `VideoNotSentNote`).
pub const VIDEO_NOT_SENT_NOTE: &str =
    "\n[video %s was not sent: this model cannot take video natively]";

/// Appended note for a video that failed to send (Go: `VideoFailedNote`).
pub const VIDEO_FAILED_NOTE: &str = "\n[video %s was not sent (%s)]";

go_struct! {
    /// A video file the host read for the model; it carries no bytes (Go:
    /// `VideoRef`).
    #[derive(PartialEq, Eq)]
    pub struct VideoRef : VideoRefWire {
        /// Public address the vendor itself visits; such a reference has no file.
        #[serde(
            rename = "url",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub url: String,
        #[serde(rename = "path", default, deserialize_with = "null_default")]
        pub path: String,
        #[serde(rename = "name", default, deserialize_with = "null_default")]
        pub name: String,
        #[serde(rename = "sha256", default, deserialize_with = "null_default")]
        pub sha256: String,
        #[serde(rename = "mime", default, deserialize_with = "null_default")]
        pub mime: String,
        #[serde(rename = "size", default, deserialize_with = "null_default")]
        pub size: i64,
        #[serde(
            rename = "duration_ms",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub duration_ms: i64,
    }
}

/// Per-request video resolution, never stored (Go: `VideoPart`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VideoPart {
    pub name: String,
    pub url: String,
    pub mime: String,
}

/// Renders [VIDEO_SHOWN_NOTE] for one video.
pub fn video_shown_note(name: &str) -> String {
    VIDEO_SHOWN_NOTE.replacen("%s", name, 1)
}

/// Renders [VIDEO_NOT_SENT_NOTE] for one video.
pub fn video_not_sent_note(name: &str) -> String {
    VIDEO_NOT_SENT_NOTE.replacen("%s", name, 1)
}

/// Renders [VIDEO_FAILED_NOTE] for one video and its send error.
pub fn video_failed_note(name: &str, reason: &str) -> String {
    VIDEO_FAILED_NOTE
        .replacen("%s", name, 1)
        .replacen("%s", reason, 1)
}

/// Newlines and closing brackets that would break the note bracket are
/// replaced (Go: `SafeVideoName`).
pub fn safe_video_name(name: &str) -> String {
    name.replace(['\n', '\r'], " ").replace(']', ")")
}

/// Returns the message with its videos replaced by the rendered note, one
/// line per video, for a consumer that has no wire for them (Go:
/// `WithoutVideos`; the note takes the file name).
pub fn without_videos(mut m: Message, note: &str) -> Message {
    if m.videos.is_empty() {
        return m;
    }
    for v in &m.videos {
        m.content.push_str(&note.replacen("%s", &v.name, 1));
    }
    m.videos = Vec::new();
    m
}

/// Marks, for each message, whether an assistant message follows it (Go:
/// `answeredAfter`).
pub(crate) fn answered_after(msgs: &[Message]) -> Vec<bool> {
    let mut out = vec![false; msgs.len()];
    let mut seen = false;
    for (i, m) in msgs.iter().enumerate().rev() {
        out[i] = seen;
        if m.role == Role::ASSISTANT {
            seen = true;
        }
    }
    out
}
