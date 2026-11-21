//! The fold's archive half (Go: `compact_archive.go`, `compact_fold_input.go`
//! `foldSummary`, and `projection.go`'s summary helpers).
//!
//! A fold moves the earlier conversation into a file and leaves a pointer;
//! the model that continues knows what it still needs, so it reads the
//! archive itself instead of inheriting a digest. The pointer text is the
//! "summary" the projection freezes — no provider call rides inside the
//! fold.

use reasonix_contract::provider::{Message, Role};
use sha2::{Digest, Sha256};

use super::Window;
use super::estimate;
use super::{SUMMARY_TAG_CLOSE, SUMMARY_TAG_OPEN};

/// Compaction mode labels (Go: `CompactionMode*`). The archive fold is the
/// only mode this build installs.
pub mod mode {
    pub const SUMMARIZED: &str = "summarized";
    pub const DEGRADED: &str = "degraded";
    pub const ARCHIVED: &str = "archived";
}

/// The checkpoint lineage stamp (Go: `promptCacheKey`): workspace, session
/// lineage and model, deliberately excluding message counts, timestamps and
/// projection hashes.
pub fn prompt_cache_key(workspace_id: &str, session_lineage: &str, model_ref: &str) -> String {
    [
        workspace_id.trim(),
        session_lineage.trim(),
        model_ref.trim(),
    ]
    .join("|")
}

/// Lowercase hex of `bytes` (Go: `hex.EncodeToString`).
pub fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Fingerprints a compaction summary for projection metadata (Go:
/// `summaryContentHash`).
pub fn summary_content_hash(summary: &str) -> String {
    if summary.is_empty() {
        return String::new();
    }
    let sum = Sha256::digest(summary.as_bytes());
    hex(&sum[..16])
}

/// The stable user-turn wrapper around a digest (Go: `formatSummaryMessage`).
pub fn format_summary_message(summary: &str) -> Message {
    Message {
        role: Role::USER,
        content: format!(
            "{SUMMARY_TAG_OPEN}\nSummary of earlier conversation (older messages were compacted to save context):\n{summary}\n{SUMMARY_TAG_CLOSE}"
        ),
        ..Message::default()
    }
}

/// Summarizer input framing (Go: `foldSummary`). Populated even when the
/// archive write fails, so a caller can still record how large the attempt
/// was.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FoldSummary {
    pub text: String,
    pub mode: String,
    pub fold_tokens: i64,
    pub spans: i64,
}

/// Sizes messages as archive input in the unit the budgets speak (Go:
/// `summaryInputTokens`): the rendered transcript is what actually rides in
/// the request, so its framing is measured rather than assumed away.
pub fn summary_input_tokens(window: &Window, msgs: &[Message]) -> i64 {
    if msgs.is_empty() {
        return 0;
    }
    estimate::estimated_prompt_tokens(
        window,
        &[Message {
            role: Role::USER,
            content: render_transcript(msgs),
            ..Message::default()
        }],
    )
}

/// Flattens messages into a readable transcript for summarization (Go:
/// `renderTranscript`). Tool-call arguments are summarized so a digest
/// cannot reproduce a long one (#4317).
pub fn render_transcript(msgs: &[Message]) -> String {
    render_transcript_with(msgs, summarize_tool_args, |m| m.content.clone())
}

/// Keeps the arguments (Go: `renderTranscriptVerbatim`). Recall exists to
/// return what was actually said and run; a recall result is a tool result
/// whose own budget bounds the size.
pub fn render_transcript_verbatim(msgs: &[Message]) -> String {
    render_transcript_with(
        msgs,
        |args| {
            if args.is_empty() {
                "(no arguments)".to_owned()
            } else {
                args.to_owned()
            }
        },
        |m| {
            if m.raw_content.is_empty() {
                m.content.clone()
            } else {
                m.raw_content.clone()
            }
        },
    )
}

fn render_transcript_with(
    msgs: &[Message],
    render_args: impl Fn(&str) -> String,
    tool_body: impl Fn(&Message) -> String,
) -> String {
    let mut b = String::new();
    for m in msgs {
        if m.local_only {
            continue;
        }
        if m.role == Role::USER {
            b.push_str(&format!("[user]\n{}\n\n", m.content));
        } else if m.role == Role::ASSISTANT {
            if !m.content.is_empty() {
                b.push_str(&format!("[assistant]\n{}\n", m.content));
            }
            for tc in &m.tool_calls {
                b.push_str(&format!(
                    "[assistant calls {}] {}\n",
                    tc.name,
                    render_args(&tc.arguments)
                ));
            }
            b.push('\n');
        } else if m.role == Role::TOOL {
            b.push_str(&format!("[tool {} result]\n{}\n\n", m.name, tool_body(m)));
        } else if m.role == Role::SYSTEM {
            b.push_str(&format!("[system]\n{}\n\n", m.content));
        }
    }
    b
}

/// A short summary of tool-call arguments instead of the full JSON (Go:
/// `summarizeToolArgs`), so the digest cannot leak long argument text into
/// the session as a user message (#4317).
pub fn summarize_tool_args(args: &str) -> String {
    if args.is_empty() {
        return "(no arguments)".to_owned();
    }
    let Ok(parsed) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(args)
    else {
        return format!("({} bytes)", args.len());
    };
    let mut keys: Vec<&str> = parsed.keys().map(String::as_str).collect();
    keys.sort_unstable();
    format!("{{{}}} ({} keys)", keys.join(", "), keys.len())
}

/// Where content moved out of context is parked (Go: `outputsDir`): beside
/// the transcript when there is one, the configured archive otherwise, and
/// failing both the scratch directory.
pub fn outputs_dir(window: &Window) -> std::path::PathBuf {
    let path = window.session_path();
    if !path.is_empty() {
        let dir = reasonix_state::store::naming::session_outputs_dir(std::path::Path::new(&path));
        if !dir.as_os_str().is_empty() {
            return dir;
        }
    }
    let root = window.config().archive_dir.trim();
    if !root.is_empty() {
        return std::path::Path::new(root).join("outputs");
    }
    reasonix_state::sessionstore::cleanup_pending::scratch_outputs_dir()
}

/// Writes the fold verbatim and returns the pointer that stands in for it
/// (Go: `foldToArchive`). The file is named by its content, so folding the
/// same span twice finds the same file. A failure to write it is the fold's
/// failure: nothing else stands in for the archive.
pub fn fold_to_archive(
    window: &Window,
    fold: &[Message],
    instructions: &str,
) -> Result<FoldSummary, crate::seams::fold_executor::FoldError> {
    use crate::seams::fold_executor::FoldError;

    let mut res = FoldSummary {
        mode: mode::ARCHIVED.to_owned(),
        spans: 1,
        fold_tokens: summary_input_tokens(window, fold),
        ..FoldSummary::default()
    };
    let body = render_transcript_verbatim(fold);
    if body.trim().is_empty() {
        return Err(FoldError::Failed("archive: nothing to archive".to_owned()));
    }
    let name = format!("fold-{}.md", hex(&Sha256::digest(body.as_bytes())[..6]));
    let mut last_err = "nowhere to write".to_owned();
    for dir in [
        outputs_dir(window),
        reasonix_state::sessionstore::cleanup_pending::scratch_outputs_dir(),
    ] {
        if std::fs::create_dir_all(&dir).is_ok() {
            let path = dir.join(&name);
            match std::fs::write(&path, body.as_bytes()) {
                Ok(()) => {
                    res.text = archive_pointer(&path.to_string_lossy(), fold.len(), instructions);
                    return Ok(res);
                }
                Err(err) => {
                    last_err = err.to_string();
                }
            }
        }
    }
    Err(FoldError::Failed(format!("archive: {last_err}")))
}

/// The pointer message that stands in for an archived span (Go:
/// `archivePointer`).
pub fn archive_pointer(path: &str, n: usize, instructions: &str) -> String {
    let mut b = String::new();
    b.push_str(&format!(
        "{n} earlier message(s) were moved out of this conversation to free context. They were archived, not summarized: the record is complete and verbatim.\n\n"
    ));
    b.push_str(&format!("Archive: {path}\n\n"));
    b.push_str(
        "Read it with read_file (a window at a time when it is long) or search it with grep for a name, path or keyword. Before you continue, recover from it the goal, the decisions made, the files touched and what remains, and say what you will do next. Do not guess what it contains.",
    );
    let focus = instructions.trim();
    if !focus.is_empty() {
        b.push_str(&format!(
            "\n\nWhat the user asked you to keep in focus: {focus}"
        ));
    }
    b
}
