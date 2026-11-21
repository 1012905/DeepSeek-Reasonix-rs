//! Session preview derivation and user-authored text classification
//! (Go: `preview.go`, `user_said_text.go`, `markers.go`,
//! `steertext.go`).
//!
//! The host stores raw user turns with injected runtime blocks. These
//! helpers recover what the user actually said so previews, turn counts,
//! and steer detection agree with the reference kernel.

use std::borrow::Cow;
use std::sync::OnceLock;

use reasonix_contract::provider::{Message, Role};
use regex::{Captures, Regex};

/// Tags of user-role blocks the host strips before a message counts as
/// user-authored (Go: `TransientUserBlockTags`).
pub const TRANSIENT_USER_BLOCK_TAGS: [&str; 21] = [
    "response-language",
    "reasoning-language",
    "turn-narration",
    "memory-update",
    "background-jobs",
    "active-goal",
    "autoresearch-runtime",
    "hook-context",
    "available-skills",
    "current-date",
    "session-scratch",
    "project-instructions",
    "project-rules",
    "scoped-rules",
    "capability-route",
    "interrupted-turn-recovery",
    "execution-policy",
    "workspace",
    "scheduled-run",
    "mcp-prompt-failure",
    "refused-calls",
];

/// Tags whose superseded blocks no longer appear in new turns (Go:
/// `SupersededUserBlockTags`).
pub const SUPERSEDED_USER_BLOCK_TAGS: [&str; 8] = [
    "active-goal",
    "response-language",
    "reasoning-language",
    "turn-narration",
    "workspace",
    "project-instructions",
    "scheduled-run",
    "current-date",
];

/// Host-injected user-role openings that must never count as a user turn
/// (Go: `SyntheticUserPrefixes`).
pub const SYNTHETIC_USER_PREFIXES: [&str; 17] = [
    "<reasoning-language>",
    "Plan approved — plan mode is off",
    "Host final-answer readiness check failed",
    "Host: this turn ended with work still outstanding",
    "This turn ended with work still outstanding",
    "You are already in the executor phase",
    "The previous assistant response was interrupted while a tool call",
    "The previous assistant response was interrupted during streaming",
    "The previous assistant response was interrupted before visible",
    "The previous assistant response finished without any visible answer",
    "<compaction-summary>",
    "Summary of the later conversation (compacted from here on):",
    "Summary of earlier conversation (compacted up to here):",
    "Continue pursuing the active goal",
    "The agent signaled goal completion and all tasks are marked done.",
    "Goal signaled complete but issues remain:",
    "No tool calls in recent turns.",
];

/// Prefix of the persisted marker that wraps a user turn captured under
/// plan mode (Go: `planModeMarkerPrefix`).
pub const PLAN_MODE_MARKER_PREFIX: &str = "[Plan mode";

/// Stable identity of the executor handoff document (Go:
/// `ExecutorHandoffMarker`).
pub const EXECUTOR_HANDOFF_MARKER: &str = "Reasonix executor handoff";

/// Opening tag of the memory-compiler execution block (Go:
/// `memoryCompilerExecutionOpen`).
pub const MEMORY_COMPILER_EXECUTION_OPEN: &str = "<memory-compiler-execution>";

/// The delivery-first runtime notice appended to user turns (Go:
/// `DeliveryRuntimeMarker`).
pub const DELIVERY_RUNTIME_MARKER: &str = "<delivery-runtime>\nThis session is in delivery-first mode. Before any state-changing tool call,\nestablish concrete, verifiable acceptance criteria with todo_write. After the\nchange, inspect the result, run relevant verification, and sign off each step\nwith complete_step citing the successful verification command. The host enforces\nthese gates and will reject mutation or finalization when evidence is missing.\n</delivery-runtime>";

/// Prefix of a queued mid-turn user steer (Go: `MidTurnSteerPrefix`).
pub const MID_TURN_STEER_PREFIX: &str = "[Mid-turn steer queued by the user. Do not treat this as a new task; use it only as additional guidance for the current task after completing the current step.]";

/// Prefix of a host-injected mid-turn notice (Go: `HostNoticePrefix`).
pub const HOST_NOTICE_PREFIX: &str = "[Mid-turn notice from the host runtime; the user did not send this. Do not treat this as a new task or as a message to answer; use it only as additional guidance for the current task after completing the current step.]";

/// Go's regex `\s` class is ASCII-only while the regex crate's is
/// Unicode-aware, so the port spells the class out.
const GO_WHITESPACE: &str = r"[\t\n\f\r ]";

/// Go: `ReTransientUserBlock`. The open and close tag alternations
/// are independent, exactly as the reference builds them.
fn re_transient_user_block() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        let alt = TRANSIENT_USER_BLOCK_TAGS.join("|");
        let pattern = format!(
            r"(?s)^{ws}*<(?:{alt})(?:{ws}+[^>]*)?>.*?</(?:{alt})>{ws}*\n?",
            ws = GO_WHITESPACE,
            alt = alt,
        );
        Regex::new(&pattern).expect("transient user block regex must compile")
    })
}

/// Go: `reMemoryCompilerExecution`.
fn re_memory_compiler_execution() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        let pattern = format!(
            r"(?s)<memory-compiler-execution>{ws}*(.*?){ws}*</memory-compiler-execution>",
            ws = GO_WHITESPACE,
        );
        Regex::new(&pattern).expect("memory compiler execution regex must compile")
    })
}

/// Go: `pasteDisplayLabelPattern`.
fn re_paste_display_label() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^\[(?:已粘贴文本|已貼上文字|Pasted text) #[0-9]+ · [0-9]+ (?:行|lines)\][ \t]*(?:\r?\n)?",
        )
        .expect("paste display label regex must compile")
    })
}

/// Go: `ContainsMemoryCompilerExecution`.
pub fn contains_memory_compiler_execution(content: &str) -> bool {
    content.contains(MEMORY_COMPILER_EXECUTION_OPEN)
}

/// The `(tag, opening, end)` read out of one block that opens `s` (Go:
/// `leadingTransientBlock`, kept_projection.go's parser). `end` indexes
/// past the whole match, so a caller can advance past the block; the tag
/// comes from the opening because a regex cannot backreference a capture.
pub fn leading_transient_block(s: &str) -> Option<(String, String, usize)> {
    let m = re_transient_user_block().find(s)?;
    if m.start() != 0 {
        return None;
    }
    let head = m.as_str().trim_start_matches([' ', '\t', '\r', '\n']);
    let close = head.find('>')?;
    for t in TRANSIENT_USER_BLOCK_TAGS {
        if head.starts_with(&format!("<{t}>")) || head.starts_with(&format!("<{t} ")) {
            return Some((t.to_string(), head[..close + 1].to_string(), m.end()));
        }
    }
    None
}

/// Go: `StripTransientUserBlocks`.
pub fn strip_transient_user_blocks(content: &str) -> String {
    let s = drop_leading_transient_blocks(&unwrap_memory_compiler_execution(content));
    let s = strip_trailing_delivery_runtime(&s);
    let s = strip_trailing_execution_policy(&s);
    let s = strip_trailing_memory_recall(&s);
    s.trim_start_matches([' ', '\t', '\r', '\n']).to_string()
}

/// Go: `DropLeadingTransientBlocks`. Blocks must appear at the very
/// start; a block later in the message is user content and stays.
fn drop_leading_transient_blocks(mut s: &str) -> String {
    loop {
        let Some(m) = re_transient_user_block().find(s) else {
            return s.to_string();
        };
        if m.start() != 0 {
            return s.to_string();
        }
        s = &s[m.end()..];
    }
}

/// Go: `stripTrailingDeliveryRuntime`. Without the marker the input
/// returns untrimmed.
fn strip_trailing_delivery_runtime(s: &str) -> String {
    let trimmed = s.trim_end_matches([' ', '\t', '\r', '\n']);
    match trimmed.strip_suffix(DELIVERY_RUNTIME_MARKER) {
        Some(cut) => cut.trim_end_matches([' ', '\t', '\r', '\n']).to_string(),
        None => s.to_string(),
    }
}

/// Go: `stripTrailingExecutionPolicy`.
fn strip_trailing_execution_policy(s: &str) -> String {
    let trimmed = s.trim_end_matches([' ', '\t', '\r', '\n']);
    if !trimmed.ends_with("</execution-policy>") {
        return s.to_string();
    }
    match trimmed.rfind("<execution-policy") {
        Some(index) => trimmed[..index]
            .trim_end_matches([' ', '\t', '\r', '\n'])
            .to_string(),
        None => s.to_string(),
    }
}

/// Go: `stripTrailingMemoryRecall`.
fn strip_trailing_memory_recall(s: &str) -> String {
    let trimmed = s.trim_end_matches([' ', '\t', '\r', '\n']);
    if !trimmed.ends_with("</memory-recall>") {
        return s.to_string();
    }
    match trimmed.rfind("<memory-recall") {
        Some(index) => trimmed[..index]
            .trim_end_matches([' ', '\t', '\r', '\n'])
            .to_string(),
        None => s.to_string(),
    }
}

/// Go: `unwrapMemoryCompilerExecution`. Rewrites well-formed blocks
/// to their recorded source event, then trims anything left before a
/// dangling opening tag so raw contract JSON never surfaces.
pub fn unwrap_memory_compiler_execution(content: &str) -> String {
    const MAX_DEPTH: usize = 24;
    let mut content = content.to_string();
    for _ in 0..MAX_DEPTH {
        if !content.contains(MEMORY_COMPILER_EXECUTION_OPEN) {
            return content;
        }
        let next = re_memory_compiler_execution()
            .replace_all(&content, |caps: &Captures<'_>| {
                memory_compiler_source_event(caps.get(1).map_or("", |m| m.as_str()))
            })
            .into_owned();
        if next == content {
            break;
        }
        content = next;
    }
    if let Some(index) = content.find(MEMORY_COMPILER_EXECUTION_OPEN) {
        return content[..index]
            .trim_end_matches([' ', '\t', '\r', '\n'])
            .to_string();
    }
    content
}

/// Go: `memoryCompilerSourceEvent`. Prefers the planner IR's event
/// over the top-level one.
fn memory_compiler_source_event(body: &str) -> String {
    #[derive(Default, serde::Deserialize)]
    struct PlannerIr {
        #[serde(default)]
        source_event: String,
    }
    #[derive(serde::Deserialize)]
    struct Doc {
        #[serde(default)]
        source_event: String,
        #[serde(default)]
        planner_ir: PlannerIr,
    }
    let Ok(doc) = serde_json::from_str::<Doc>(body.trim()) else {
        return String::new();
    };
    let planner = doc.planner_ir.source_event.trim();
    if !planner.is_empty() {
        return planner.to_string();
    }
    doc.source_event.trim().to_string()
}

/// Go: `HandoffTask`. Recovers the original task from an executor
/// handoff document.
pub fn handoff_task(s: &str) -> String {
    let trimmed = s.trim();
    if !trimmed.starts_with("# Reasonix executor handoff") {
        return s.to_string();
    }
    let Some((_, after)) = trimmed.split_once("Original task:\n") else {
        return s.to_string();
    };
    let mut rest = after;
    if let Some(j) = rest.find("\n\nPlanner output:") {
        rest = &rest[..j];
    }
    let task = rest.trim();
    if !task.is_empty() {
        task.to_string()
    } else {
        s.to_string()
    }
}

/// Go: `userAuthoredText`.
pub fn user_authored_text(content: &str) -> String {
    let s = strip_transient_user_blocks(content);
    let s = handoff_task(&s);
    let s = strip_transient_user_blocks(&s);
    s.trim().to_string()
}

/// Go: `UserPreviewText`.
pub fn user_preview_text(content: &str) -> String {
    user_authored_text(content)
}

/// Go: `UserSaidText`. Drops the plan-mode wrapper when present.
pub fn user_said_text(content: &str) -> String {
    let trimmed = user_authored_text(content);
    if let Some(rest) = trimmed.strip_prefix(PLAN_MODE_MARKER_PREFIX)
        && let Some((_, after)) = rest.split_once(']')
    {
        return after.trim().to_string();
    }
    trimmed
}

/// Go: `StripPasteDisplayLabel`.
pub fn strip_paste_display_label(content: &str) -> String {
    re_paste_display_label()
        .replace_all(content, "")
        .into_owned()
}

/// Go: `UserMessageText`. The raw content is authoritative; the
/// display content is the fallback.
pub fn user_message_text(msg: &Message) -> String {
    if !msg.raw_content.is_empty() {
        user_said_text(&msg.raw_content)
    } else {
        user_said_text(&msg.content)
    }
}

/// Go: `hasLegacyProviderWrapper`.
fn has_legacy_provider_wrapper(content: &str) -> bool {
    if contains_memory_compiler_execution(content) {
        return true;
    }
    if re_transient_user_block().is_match(content) {
        return true;
    }
    if strip_trailing_delivery_runtime(content) != content {
        return true;
    }
    let stripped = strip_transient_user_blocks(content);
    handoff_task(&stripped) != stripped
}

/// Go: `MigrateLegacyProviderContent`. Copy-on-write upgrade of the
/// legacy provider-content layout; clean inputs borrow through unchanged.
pub fn migrate_legacy_provider_content(msgs: &[Message]) -> Cow<'_, [Message]> {
    let mut upgraded: Option<Vec<Message>> = None;
    for (i, msg) in msgs.iter().enumerate() {
        if msg.role != Role::USER {
            continue;
        }
        if !msg.provider_content.is_empty() {
            let v = upgraded.get_or_insert_with(|| msgs.to_vec());
            if v[i].raw_content.is_empty() {
                v[i].raw_content = msg.content.clone();
            }
            v[i].content = msg.provider_content.clone();
            v[i].provider_content = String::new();
        } else if msg.raw_content.is_empty() && has_legacy_provider_wrapper(&msg.content) {
            let v = upgraded.get_or_insert_with(|| msgs.to_vec());
            v[i].raw_content = user_preview_text(&msg.content);
        }
    }
    match upgraded {
        Some(v) => Cow::Owned(v),
        None => Cow::Borrowed(msgs),
    }
}

/// Go: `IsSyntheticUserText`.
pub fn is_synthetic_user_text(content: &str) -> bool {
    let trimmed = strip_transient_user_blocks(content);
    let trimmed = trimmed.trim();
    SYNTHETIC_USER_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
}

/// Go: `IsUserAuthoredTurn`.
pub fn is_user_authored_turn(content: &str) -> bool {
    let stripped = strip_transient_user_blocks(content);
    if stripped.trim().is_empty() {
        return false;
    }
    if is_synthetic_user_text(content) {
        return false;
    }
    if steer_kind(content).2 {
        return false;
    }
    true
}

/// Go: `MidTurnSteerMessage`.
pub fn mid_turn_steer_message(text: &str, host: bool) -> String {
    let prefix = if host {
        HOST_NOTICE_PREFIX
    } else {
        MID_TURN_STEER_PREFIX
    };
    format!("{prefix}\n{text}")
}

/// Go: `SteerText`: the steer text and whether the content is one.
pub fn steer_text(content: &str) -> (String, bool) {
    let (text, _, is_steer) = steer_kind(content);
    (text, is_steer)
}

/// Go: `SteerKind`: (text, came-from-host, is-steer). A steer may
/// hide behind leading transient blocks; the unwrap loop walks them.
pub fn steer_kind(content: &str) -> (String, bool, bool) {
    let mut s = content.to_string();
    loop {
        for (i, prefix) in [MID_TURN_STEER_PREFIX, HOST_NOTICE_PREFIX]
            .iter()
            .enumerate()
        {
            if let Some(after_prefix) = s.strip_prefix(prefix) {
                let after_sep = after_prefix.strip_prefix('\n').unwrap_or(after_prefix);
                let suffix = format!("\n\n{DELIVERY_RUNTIME_MARKER}");
                let after = after_sep.strip_suffix(&suffix).unwrap_or(after_sep);
                return (after.to_string(), i == 1, true);
            }
        }
        match trim_leading_steer_wrapper(&s) {
            Some(next) => s = next,
            None => return (String::new(), false, false),
        }
    }
}

/// Go: `TrimLeadingSteerWrapper`.
fn trim_leading_steer_wrapper(content: &str) -> Option<String> {
    let s = content.trim_start_matches([' ', '\t', '\r', '\n']);
    for tag in TRANSIENT_USER_BLOCK_TAGS {
        if !has_open_tag(s, tag) {
            continue;
        }
        if let Some(rest) = trim_leading_transient_block(s, tag) {
            return Some(rest);
        }
    }
    None
}

/// Go: `HasOpenTag`.
fn has_open_tag(s: &str, tag: &str) -> bool {
    s.starts_with(&format!("<{tag}>")) || s.starts_with(&format!("<{tag} "))
}

/// Go: `TrimLeadingTransientBlock`.
fn trim_leading_transient_block(content: &str, tag: &str) -> Option<String> {
    let close_tag = format!("</{tag}>");
    let (_, after) = content.split_once(&close_tag)?;
    Some(
        after
            .trim_start_matches([' ', '\t', '\r', '\n'])
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_leading_transient_block() {
        let content = "<current-date>2026-01-01</current-date>\nhello";
        assert_eq!(strip_transient_user_blocks(content), "hello");
    }

    #[test]
    fn strips_leading_block_with_attributes() {
        let content = "<workspace root=\"/a\">ctx</workspace>hi";
        assert_eq!(strip_transient_user_blocks(content), "hi");
    }

    #[test]
    fn open_and_close_tags_alternate_independently() {
        let content = "<active-goal>x</workspace>rest";
        assert_eq!(strip_transient_user_blocks(content), "rest");
    }

    #[test]
    fn keeps_non_leading_blocks() {
        let content = "hello <workspace>x</workspace>";
        assert_eq!(strip_transient_user_blocks(content), content);
    }

    #[test]
    fn delivery_runtime_marker_strips_from_the_tail() {
        let content = format!("do things{DELIVERY_RUNTIME_MARKER}");
        assert_eq!(strip_trailing_delivery_runtime(&content), "do things");
    }

    #[test]
    fn delivery_runtime_absence_returns_input_untrimmed() {
        assert_eq!(strip_trailing_delivery_runtime("abc   "), "abc   ");
    }

    #[test]
    fn execution_policy_strips_from_the_tail() {
        let content = "plan <execution-policy>strict</execution-policy>";
        assert_eq!(strip_trailing_execution_policy(content), "plan");
    }

    #[test]
    fn memory_recall_strips_from_the_tail() {
        let content = "plan <memory-recall>ctx</memory-recall>";
        assert_eq!(strip_trailing_memory_recall(content), "plan");
    }

    #[test]
    fn unwrap_rewrites_to_the_recorded_source_event() {
        let content = "<memory-compiler-execution>{\"source_event\":\"did things\"}</memory-compiler-execution>";
        assert_eq!(unwrap_memory_compiler_execution(content), "did things");
    }

    #[test]
    fn unwrap_prefers_the_planner_ir_event() {
        let content = "<memory-compiler-execution>{\"planner_ir\":{\"source_event\":\"plan a\"},\"source_event\":\"direct\"}</memory-compiler-execution>";
        assert_eq!(unwrap_memory_compiler_execution(content), "plan a");
    }

    #[test]
    fn unwrap_trims_before_a_dangling_opening_tag() {
        let content = "prefix <memory-compiler-execution> {\"source_event\":\"x\"}";
        assert_eq!(unwrap_memory_compiler_execution(content), "prefix");
    }

    #[test]
    fn handoff_task_extracts_the_original_task() {
        let s = "# Reasonix executor handoff\n\nOriginal task:\nship the feature\n\nPlanner output:\nsteps";
        assert_eq!(handoff_task(s), "ship the feature");
    }

    #[test]
    fn handoff_task_leaves_plain_text_alone() {
        assert_eq!(handoff_task("just a task"), "just a task");
    }

    #[test]
    fn user_said_text_drops_the_plan_mode_wrapper() {
        assert_eq!(
            user_said_text("[Plan mode on] do the thing"),
            "do the thing"
        );
        assert_eq!(user_said_text("plain words"), "plain words");
    }

    #[test]
    fn synthetic_user_text_detection() {
        assert!(is_synthetic_user_text("<reasoning-language>\nChinese"));
        assert!(is_synthetic_user_text("Continue pursuing the active goal"));
        assert!(!is_synthetic_user_text("Please fix the bug"));
        assert!(!is_synthetic_user_text(
            "<reasoning-language>\nChinese</reasoning-language>\nreal task"
        ));
    }

    #[test]
    fn user_authored_turn_classification() {
        assert!(!is_user_authored_turn("   "));
        assert!(!is_user_authored_turn("Continue pursuing the active goal"));
        assert!(!is_user_authored_turn(&format!(
            "{MID_TURN_STEER_PREFIX}\nand also check Y"
        )));
        assert!(is_user_authored_turn("Fix the login bug"));
    }

    #[test]
    fn steer_kind_detects_user_and_host_prefixes() {
        let content = format!("{MID_TURN_STEER_PREFIX}\nuse faster mode");
        let (text, host, is_steer) = steer_kind(&content);
        assert_eq!(text, "use faster mode");
        assert!(!host);
        assert!(is_steer);

        let content = format!("{HOST_NOTICE_PREFIX}\nuse faster mode");
        let (text, host, is_steer) = steer_kind(&content);
        assert_eq!(text, "use faster mode");
        assert!(host);
        assert!(is_steer);
    }

    #[test]
    fn steer_kind_strips_delivery_runtime_suffix() {
        let content =
            format!("{MID_TURN_STEER_PREFIX}\nuse faster mode\n\n{DELIVERY_RUNTIME_MARKER}");
        let (text, _, is_steer) = steer_kind(&content);
        assert_eq!(text, "use faster mode");
        assert!(is_steer);
    }

    #[test]
    fn steer_kind_unwraps_leading_transient_blocks() {
        let content = format!("<workspace>/tmp</workspace>\n{MID_TURN_STEER_PREFIX}\nfaster");
        let (text, _, is_steer) = steer_kind(&content);
        assert_eq!(text, "faster");
        assert!(is_steer);
    }

    #[test]
    fn non_steer_content_is_not_a_steer() {
        let (text, host, is_steer) = steer_kind("just a task");
        assert_eq!(text, "");
        assert!(!host);
        assert!(!is_steer);
        assert_eq!(steer_text("just a task"), (String::new(), false));
    }

    #[test]
    fn mid_turn_steer_message_prefixes_by_origin() {
        assert_eq!(
            mid_turn_steer_message("go", false),
            format!("{MID_TURN_STEER_PREFIX}\ngo")
        );
        assert_eq!(
            mid_turn_steer_message("go", true),
            format!("{HOST_NOTICE_PREFIX}\ngo")
        );
    }

    #[test]
    fn paste_display_label_is_stripped() {
        let content = "[已粘贴文本 #12 · 34 行]\npasted body";
        assert_eq!(strip_paste_display_label(content), "pasted body");
        let content = "[Pasted text #1 · 2 lines]\nbody";
        assert_eq!(strip_paste_display_label(content), "body");
    }

    #[test]
    fn user_message_text_prefers_raw_content() {
        let mut msg = Message {
            role: Role::USER,
            content: "display".to_string(),
            raw_content: "raw words".to_string(),
            ..Message::default()
        };
        assert_eq!(user_message_text(&msg), "raw words");
        msg.raw_content = String::new();
        assert_eq!(user_message_text(&msg), "display");
    }

    #[test]
    fn migrate_upgrades_provider_content_layout() {
        let msg = Message {
            role: Role::USER,
            content: "wrapped display".to_string(),
            provider_content: "legacy provider text".to_string(),
            ..Message::default()
        };
        let msgs = [msg];
        let out = migrate_legacy_provider_content(&msgs);
        assert!(matches!(out, Cow::Owned(_)));
        assert_eq!(out[0].content, "legacy provider text");
        assert_eq!(out[0].raw_content, "wrapped display");
        assert_eq!(out[0].provider_content, "");
    }

    #[test]
    fn migrate_backfills_wrapped_raw_content() {
        let msg = Message {
            role: Role::USER,
            content:
                "<memory-compiler-execution>{\"source_event\":\"real\"}</memory-compiler-execution>"
                    .to_string(),
            ..Message::default()
        };
        let msgs = [msg];
        let out = migrate_legacy_provider_content(&msgs);
        assert_eq!(out[0].raw_content, "real");
    }

    #[test]
    fn migrate_borrows_clean_input() {
        let msg = Message {
            role: Role::USER,
            content: "plain".to_string(),
            ..Message::default()
        };
        let msgs = [msg];
        let out = migrate_legacy_provider_content(&msgs);
        assert!(matches!(out, Cow::Borrowed(_)));
    }

    #[test]
    fn user_preview_text_strips_transient_blocks() {
        let content = "<current-date>2026</current-date>\nactual request";
        assert_eq!(user_preview_text(content), "actual request");
    }
}
