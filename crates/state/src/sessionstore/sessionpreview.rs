//! Session listing preview (Go: `sessionpreview.go`).
//!
//! Derives the one-line listing projection — a human preview string and a
//! user-turn count — from a message list (Go:
//! `SessionPreviewFromMessages`). The disk-side variants
//! (`SessionPreview`, `previewSession`, and
//! `UserMessagesTitleSource`) belong to the load path and are
//! covered by SESS-023.

use reasonix_contract::provider::{Message, Role};

use super::preview::{is_user_authored_turn, user_message_text};

/// Derives the listing preview and user-turn count from the messages
/// (Go: `SessionPreviewFromMessages`).
///
/// A message counts as a turn when it is user-role, not host-authored,
/// and its text passes `is_user_authored_turn`. The preview is the
/// prose form of the first such turn, truncated to the listing limit.
pub fn session_preview_from_messages(msgs: &[Message]) -> (String, i64) {
    let mut preview = String::new();
    let mut turns: i64 = 0;
    for m in msgs {
        if m.role != Role::USER || m.host_authored {
            continue;
        }
        let text = user_message_text(m);
        if !is_user_authored_turn(&text) {
            continue;
        }
        turns += 1;
        if preview.is_empty() {
            preview = truncate_preview(&preview_prose(&text));
        }
    }
    (preview, turns)
}

/// Strips leading `` references from turn text (Go:
/// `PreviewProse`).
///
/// Leading spaces, tabs, and ``-prefixed mentions (up to the first
/// whitespace run ending the mention) are dropped. When no mention
/// terminates, when the remaining text is blank, or when the input is
/// empty, the original text is kept verbatim.
pub fn preview_prose(s: &str) -> String {
    let mut rest = s.trim_start_matches([' ', '\t']);
    while rest.starts_with('@') {
        let Some(end) = rest.find([' ', '\t', '\r', '\n']) else {
            return s.to_string();
        };
        let next = rest[end..].trim_start_matches([' ', '\t']);
        if next.trim().is_empty() {
            return s.to_string();
        }
        rest = next;
    }
    if rest.is_empty() {
        s.to_string()
    } else {
        rest.to_string()
    }
}

/// Truncates a preview to the listing limit: at most 80 runes, longer
/// text becomes the first 77 runes plus an ellipsis (Go:
/// `TruncatePreview`).
pub fn truncate_preview(s: &str) -> String {
    let runes: Vec<char> = s.chars().collect();
    if runes.len() > 80 {
        let mut out: String = runes[..77].iter().collect();
        out.push('…');
        out
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_msg(content: &str) -> Message {
        Message {
            role: Role::USER,
            content: content.to_string(),
            ..Message::default()
        }
    }

    #[test]
    fn preview_prose_strips_leading_references() {
        assert_eq!(preview_prose("@ref hello world"), "hello world");
        assert_eq!(preview_prose("  @a @b text"), "text");
        assert_eq!(preview_prose("\t@x\tvalue"), "value");
        assert_eq!(preview_prose("plain text"), "plain text");
        // A newline ends the mention but the trimmed continuation keeps
        // the newline: only spaces and tabs are stripped as separators.
        assert_eq!(preview_prose("@x\nvalue"), "\nvalue");
    }

    #[test]
    fn preview_prose_keeps_degenerate_inputs() {
        // A mention with no terminator keeps the original.
        assert_eq!(preview_prose("@only"), "@only");
        // A mention followed by only blank text keeps the original.
        assert_eq!(preview_prose("@a "), "@a ");
        // Empty input stays empty.
        assert_eq!(preview_prose(""), "");
        // Whitespace-only input keeps the original.
        assert_eq!(preview_prose("  \t"), "  \t");
    }

    #[test]
    fn truncate_preview_limits_listings() {
        let exact: String = "x".repeat(80);
        assert_eq!(truncate_preview(&exact), exact);
        let long: String = "x".repeat(81);
        let mut want: String = "x".repeat(77);
        want.push('…');
        assert_eq!(truncate_preview(&long), want);
        assert_eq!(truncate_preview(""), "");
    }

    #[test]
    fn preview_counts_user_turns_and_keeps_first() {
        let msgs = vec![
            Message {
                role: Role::SYSTEM,
                content: "sys".to_string(),
                ..Message::default()
            },
            user_msg("hello world"),
            Message {
                role: Role::ASSISTANT,
                content: "hi".to_string(),
                ..Message::default()
            },
            user_msg("@model do the thing"),
        ];
        let (preview, turns) = session_preview_from_messages(&msgs);
        assert_eq!(preview, "hello world");
        assert_eq!(turns, 2);
    }

    #[test]
    fn preview_skips_host_authored_messages() {
        let host = Message {
            host_authored: true,
            ..user_msg("host wrote this")
        };
        let msgs = vec![host, user_msg("real turn")];
        let (preview, turns) = session_preview_from_messages(&msgs);
        assert_eq!(preview, "real turn");
        assert_eq!(turns, 1);
    }

    #[test]
    fn preview_empty_message_list_is_blank() {
        let (preview, turns) = session_preview_from_messages(&[]);
        assert_eq!(preview, "");
        assert_eq!(turns, 0);
    }
}
