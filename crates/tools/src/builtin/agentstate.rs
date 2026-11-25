//! TOOL-042: agent / state / platform tool cards (docs/contracts/tools.md
//! §3.7; Go `internal/runtime/agent/await_user.go`,
//! `internal/tools/builtin/viewimage.go`,
//! `internal/runtime/agent/readonly_bash.go`).
//!
//! `await_user` is the turn-exit sibling of `ask`; `view_image` is the
//! read-only image display tool; the read-only bash wrapper rewrites the
//! description and names what disqualified a command for a sub-agent. The
//! argument validation, todo resolution, refusal classification and rendering
//! are pure here; the interactive asker, the image decode/fit and the
//! permission-layer bash classifier stay behind seams the integration slice
//! binds.

#![forbid(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::abi::{CallContext, Capability, CapabilitySet, DecisionBarrier, Tool, ToolResult};
use crate::ask::asker_from;
use crate::builtin::plan_evidence::{
    TodoItem, in_progress_todo, incomplete_todos, match_step_id, todo_citation, todo_step_ids,
};
use crate::refusal::{Refusal, RemedyOp};
use crate::registry::FEATURE_TURN_EXIT;
use crate::registry::HostRequired;

// --- await_user -------------------------------------------------------------

/// The `await_user` tool name.
pub const AWAIT_USER_TOOL_NAME: &str = "await_user";
/// The one-line status the host shows holds at most this many runes (Go
/// `len([]rune(need)) > 200`).
pub const AWAIT_USER_NEED_LIMIT: usize = 200;

/// The Go verbatim `await_user` description.
pub const AWAIT_USER_DESCRIPTION: &str = "Stop the turn and hand the current task-list item to the user. Use it \
when the plan reaches something that needs a decision, a choice, or a \
check only the user can make: a product trade-off, a review, confirmation \
that a risky step may proceed. The turn ends here; the host will not \
continue it, and the user's next message resumes the list from the item you \
named. One open item at a time. The `need` you give is the one-line status \
the host parks on the list — say what you are waiting for, in a short \
phrase, not the reasoning and not the options. Put the reasoning and the \
choices in your reply instead, and if you are offering choices call `ask` \
with them as options first.";

const AWAIT_USER_SCHEMA: &str = r#"{"type":"object","properties":{"need":{"type":"string","description":"One-line status: what you are waiting for the user to give you. Hold it to the substance, not the reasoning."},"step_id":{"type":"string","description":"The stable step_id of the list item you are parking. Omit to park the current in_progress item."}},"required":["need"],"additionalProperties":false}"#;

/// The one todo an `await_user` parks, resolved against the current list.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AwaitTarget {
    /// The citation the model and list render.
    pub citation: String,
    /// The item's status before parking.
    pub status: String,
    /// The item's active form, when it carries one.
    pub active_form: String,
}

/// Typed `await_user` failure identity; messages track the Go sentences so
/// the model's remedy stays the same.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AwaitUserError {
    /// The turn is a plan submission; `await_user` cannot hand that back.
    #[error(
        "await_user hands back an execution turn, not a planning one; a plan that needs the user's decision states the unknown, or calls ask"
    )]
    PlanSubmission,
    /// The status line is empty.
    #[error("await_user needs a `need`: say what you are waiting for the user to give you")]
    EmptyNeed,
    /// The status line exceeds the rune limit.
    #[error(
        "`need` is {count} characters and is shown to the user as a one-line status, so it holds at most {limit}; put the substance in your reply, and if you are offering choices call ask with them as options instead"
    )]
    NeedTooLong { count: usize, limit: usize },
    /// The run has nobody to answer.
    #[error(
        "await_user needs an interactive user and this run has none, so there is nobody whose answer would arrive; do whatever the list allows without them, then say in your reply which input is missing"
    )]
    NoAsker,
    /// The turn left no open list.
    #[error(
        "await_user parks an open task list and this turn has none; end your reply with what you want from the user instead — a turn that left no list open is not continued"
    )]
    NoList,
    /// Every list item is completed.
    #[error(
        "every item on the current task list is completed, so nothing is waiting; end your reply with what you want from the user instead"
    )]
    AllComplete,
    /// The cited id names no item, and the list carries ids to cite.
    #[error("step_id {step_id:?} names no item on the current task list; cite one of: {ids}")]
    UnknownStepWithIds { step_id: String, ids: String },
    /// The cited id names no item, and the list carries no ids at all.
    #[error(
        "step_id {step_id:?} names no item on the current task list, and the list carries no ids at all; leave step_id out and the current item is taken"
    )]
    UnknownStepNoIds { step_id: String },
    /// The cited item is already completed.
    #[error(
        "todo {index} {content:?} is already completed, so nothing about it waits on the user; name the item that does, or leave step_id out"
    )]
    CompletedStep { index: usize, content: String },
    /// No item is in_progress and the list carries ids.
    #[error(
        "no item on the list is in_progress, so there is no current one to name; give step_id — available ids: {ids}"
    )]
    NoCurrentWithIds { ids: String },
    /// No item is in_progress and the list carries no ids.
    #[error(
        "no item on the list is in_progress, so there is no current one to name; mark the item you are waiting on with todo_write first"
    )]
    NoCurrentNoIds,
}

/// Validates the `need` status line: non-empty and within the rune limit.
pub fn validate_need(need: &str) -> Result<(), AwaitUserError> {
    let trimmed = need.trim();
    if trimmed.is_empty() {
        return Err(AwaitUserError::EmptyNeed);
    }
    let count = trimmed.chars().count();
    if count > AWAIT_USER_NEED_LIMIT {
        return Err(AwaitUserError::NeedTooLong {
            count,
            limit: AWAIT_USER_NEED_LIMIT,
        });
    }
    Ok(())
}

/// Resolves the parked item against the current list (Go `awaitUser` step
/// resolution, minus the asker and plan-submission gates the host owns).
pub fn resolve_await_target(
    todos: &[TodoItem],
    step_id: Option<&str>,
) -> Result<AwaitTarget, AwaitUserError> {
    if todos.is_empty() {
        return Err(AwaitUserError::NoList);
    }
    if incomplete_todos(todos).is_empty() {
        return Err(AwaitUserError::AllComplete);
    }

    let ids = todo_step_ids(todos);
    if let Some(step_id) = step_id.filter(|s| !s.trim().is_empty()) {
        let (match_, found) = match_step_id(step_id, todos);
        if !found {
            return if ids.is_empty() {
                Err(AwaitUserError::UnknownStepNoIds {
                    step_id: step_id.to_owned(),
                })
            } else {
                Err(AwaitUserError::UnknownStepWithIds {
                    step_id: step_id.to_owned(),
                    ids: ids.join(", "),
                })
            };
        }
        if match_.status == "completed" {
            return Err(AwaitUserError::CompletedStep {
                index: match_.index,
                content: match_.content,
            });
        }
        return Ok(AwaitTarget {
            citation: todo_citation(&match_.step_id, match_.index, &match_.content),
            status: match_.status,
            active_form: match_.active_form,
        });
    }

    let (current, found) = in_progress_todo(todos);
    if found && current.status == "completed" {
        return Err(AwaitUserError::CompletedStep {
            index: current.index,
            content: current.content,
        });
    }
    if !found {
        return if ids.is_empty() {
            Err(AwaitUserError::NoCurrentNoIds)
        } else {
            Err(AwaitUserError::NoCurrentWithIds {
                ids: ids.join(", "),
            })
        };
    }
    Ok(AwaitTarget {
        citation: todo_citation(&current.step_id, current.index, &current.content),
        status: current.status,
        active_form: current.active_form,
    })
}

/// The success line the host records after parking the item.
pub fn render_recorded(citation: &str) -> String {
    format!(
        "Recorded: waiting on the user at {citation}. The turn ends here with the list untouched, \
and the host will not continue it — their next message does. Put what you are waiting for to them in your reply."
    )
}

/// The host seam behind `await_user`: the current list and whether this turn
/// is a plan submission.
pub trait AwaitUserHost: Send + Sync {
    /// The current todo list.
    fn todos(&self) -> Vec<TodoItem>;
    /// Whether this turn hands a plan back rather than an execution list.
    fn plan_submission(&self) -> bool;
}

#[derive(Deserialize, Default)]
struct AwaitArgs {
    #[serde(default)]
    need: String,
    #[serde(default)]
    step_id: Option<String>,
}

struct AlwaysBarrier;
impl Capability for AlwaysBarrier {}
impl DecisionBarrier for AlwaysBarrier {
    fn decision_barrier(&self) -> bool {
        true
    }
}

struct HostTurnExit;
impl Capability for HostTurnExit {}
impl HostRequired for HostTurnExit {
    fn host_required_by(&self) -> String {
        FEATURE_TURN_EXIT.to_owned()
    }
}

/// The `await_user` tool card.
#[derive(Clone)]
pub struct AwaitUserTool {
    host: Option<Arc<dyn AwaitUserHost>>,
}

impl AwaitUserTool {
    /// Builds the card over an optional host seam.
    pub fn new(host: Option<Arc<dyn AwaitUserHost>>) -> Self {
        Self { host }
    }
}

impl Tool for AwaitUserTool {
    fn name(&self) -> &str {
        AWAIT_USER_TOOL_NAME
    }

    fn description(&self) -> &str {
        AWAIT_USER_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(AWAIT_USER_SCHEMA).expect("the await_user schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let parsed: AwaitArgs = serde_json::from_value(args.clone())
            .map_err(|e| invalid_err(format!("invalid args: {e}")))?;
        validate_need(&parsed.need)?;
        let Some(host) = &self.host else {
            return Err(Box::new(AwaitUserError::NoAsker));
        };
        if host.plan_submission() {
            return Err(Box::new(AwaitUserError::PlanSubmission));
        }
        if asker_from(ctx).is_none() {
            return Err(Box::new(AwaitUserError::NoAsker));
        }
        let todos = host.todos();
        let target = resolve_await_target(&todos, parsed.step_id.as_deref())?;
        Ok(render_recorded(&target.citation))
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default()
            .with(
                "decision_barrier",
                Arc::new(AlwaysBarrier) as Arc<dyn DecisionBarrier>,
            )
            .with(
                "host_required",
                Arc::new(HostTurnExit) as Arc<dyn HostRequired>,
            )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// --- view_image -------------------------------------------------------------

/// The `view_image` tool name.
pub const VIEW_IMAGE_TOOL_NAME: &str = "view_image";
/// Largest carried image, 20 MiB (Go `maxViewImageBytes`).
pub const VIEW_IMAGE_MAX_BYTES: usize = 20 * 1024 * 1024;

/// `image.unsupported_type` — the MIME is not a viewable image.
pub const CODE_IMAGE_UNSUPPORTED: &str = "image.unsupported_type";
/// `image.too_large` — the file is over the carried-byte limit.
pub const CODE_IMAGE_TOO_LARGE: &str = "image.too_large";
/// `image.unfit` — the image cannot be decoded into a fitable display image.
pub const CODE_IMAGE_UNFIT: &str = "image.unfit";

/// The Go verbatim `view_image` description.
pub const VIEW_IMAGE_DESCRIPTION: &str = "Display an image file from the workspace so you can see it. PNG, JPEG, \
WebP, GIF and SVG. The display size and fit are handled by the host; you see \
a single still. Read large diagrams and screenshots that way instead of \
guessing from the bytes or the file name.";

const VIEW_IMAGE_SCHEMA: &str = r#"{"type":"object","properties":{"path":{"type":"string","description":"Workspace-relative path to the image file."}},"required":["path"],"additionalProperties":false}"#;

/// MIME types the host may display.
const VIEWABLE_MIMES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "image/svg+xml",
];

/// Whether `mime` is a viewable image type.
pub fn mime_supported(mime: &str) -> bool {
    VIEWABLE_MIMES.contains(&mime.trim().to_ascii_lowercase().as_str())
}

/// The unsupported-type refusal (Go verbatim message + remedy).
pub fn unsupported_refusal(mime: &str) -> Refusal {
    Refusal::code(
        CODE_IMAGE_UNSUPPORTED,
        format!(
            "{CODE_IMAGE_UNSUPPORTED}: {mime} is not a viewable image (png, jpeg, webp, gif, svg)"
        ),
    )
    .with_remedy(RemedyOp::Propose, "attachments.max_view_bytes")
}

/// The too-large refusal (Go verbatim message + remedy).
pub fn too_large_refusal(path: &str, size: usize, max: usize) -> Refusal {
    Refusal::code(
        CODE_IMAGE_TOO_LARGE,
        format!("{CODE_IMAGE_TOO_LARGE}: {path} is {size} bytes, over the {max} bytes a view_image carries"),
    )
    .with_remedy(RemedyOp::Propose, "attachments.max_view_bytes")
}

/// The unfit refusal (Go verbatim message + remedy, open only).
pub fn unfit_refusal(path: &str, detail: &str) -> Refusal {
    Refusal::code(
        CODE_IMAGE_UNFIT,
        format!(
            "{CODE_IMAGE_UNFIT}: {path} cannot be decoded into a fitable display image: {detail}"
        ),
    )
    .with_remedy(RemedyOp::Open, "attachments.max_view_bytes")
}

/// One decoded image the host returns through the seam.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ViewImageAsset {
    /// The display path shown in the caption.
    pub display_path: String,
    /// The detected MIME type.
    pub mime: String,
    /// The image bytes carried in the data URL.
    pub bytes: Vec<u8>,
}

/// The host seam behind `view_image`: resolve, read, decode and fit. The
/// decode/fit implementation lives in the integration slice; this card only
/// classifies the refusal the seam hands back.
pub trait ViewImageHost: Send + Sync {
    /// Resolves and reads one image, or returns a typed refusal.
    fn read_image(&self, path: &str) -> Result<ViewImageAsset, Refusal>;
}

/// Standard alphabet base64, no padding tricks (the data URL needs StdEncoding).
fn base64_std(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((triple >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(triple & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// The data URL carrying the image bytes.
pub fn data_url(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", base64_std(bytes))
}

/// Renders the visible line plus the data URL (Go `viewImage` output shape).
pub fn render_image(asset: &ViewImageAsset) -> String {
    format!(
        "[image: {}, {}, {} bytes shown]\n{}",
        asset.display_path,
        asset.mime,
        asset.bytes.len(),
        data_url(&asset.mime, &asset.bytes)
    )
}

/// The `view_image` tool card.
#[derive(Clone)]
pub struct ViewImageTool {
    host: Option<Arc<dyn ViewImageHost>>,
}

impl ViewImageTool {
    /// Builds the card over an optional host seam.
    pub fn new(host: Option<Arc<dyn ViewImageHost>>) -> Self {
        Self { host }
    }
}

impl Tool for ViewImageTool {
    fn name(&self) -> &str {
        VIEW_IMAGE_TOOL_NAME
    }

    fn description(&self) -> &str {
        VIEW_IMAGE_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(VIEW_IMAGE_SCHEMA).expect("the view_image schema is valid JSON")
    }

    fn execute(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if path.is_empty() {
            return Err(invalid_err("image target required: path"));
        }
        let Some(host) = &self.host else {
            return Err(Box::new(Refusal::code(
                CODE_IMAGE_UNFIT,
                "view_image host seam is not wired in this slice",
            )));
        };
        let asset = host.read_image(path)?;
        Ok(render_image(&asset))
    }

    fn read_only(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// --- read-only bash wrapper -------------------------------------------------

/// What disqualified a command from the read-only set (Go
/// `permission.BashApprovalBlocker*`). The classifier seam fills these; this
/// slice owns the wording each one renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOnlyBashBlocker {
    /// Inline interpreter code (`python -c`, `node -e`, `bash -c`).
    InlineCode,
    /// Nested execution through substitution.
    NestedExecution,
    /// The program name comes from a variable.
    DynamicName,
    /// A wrapper executes something the command does not name.
    IndirectExecution,
    /// A here-document whose body is file content.
    HereDocBody,
    /// Static analysis could not parse the command.
    Unparsable,
    /// Anything outside the read-only set, or a write-capable argument.
    Default,
}

/// The read-only wrapper's rewritten description (Go `readOnlyBash.Description`).
pub fn readonly_bash_description(inner: &str) -> String {
    let desc = inner.trim();
    let desc = if desc.is_empty() {
        "Execute a command in the shell and return combined stdout/stderr."
    } else {
        desc
    };
    let rewritten = desc.replace(
        "Execute a command in the shell",
        "Execute a foreground read-only command in the shell",
    );
    format!(
        "{rewritten} Only permission-classified read-only commands are allowed; shell operators, background execution, process preservation, and write-capable arguments are blocked."
    )
}

/// The blocker-specific block reason (Go `readOnlyBashBlockReason`).
pub fn readonly_block_reason(blocker: ReadOnlyBashBlocker) -> String {
    const LEAD: &str =
        "blocked: a read-only sub-agent runs only commands the host classifies as read-only. ";
    const REMEDY: &str = " Prefer read_file, grep, ls, or glob for inspection; anything that writes belongs to the parent turn.";
    let detail = match blocker {
        ReadOnlyBashBlocker::InlineCode => {
            "This one carries inline interpreter code (python -c, node -e, bash -c), which the host cannot audit."
        }
        ReadOnlyBashBlocker::NestedExecution => {
            "This one nests another command through substitution ($(...), `...`, or <(...)), so what would run is not visible in it."
        }
        ReadOnlyBashBlocker::DynamicName => {
            "The program this one runs comes from a variable, so the host cannot tell what would execute."
        }
        ReadOnlyBashBlocker::IndirectExecution => {
            "This one runs through a wrapper (eval, source, xargs, find -exec) that executes something it does not name."
        }
        ReadOnlyBashBlocker::HereDocBody => {
            "This one feeds a here-document, whose body is file content rather than arguments the host can read."
        }
        ReadOnlyBashBlocker::Unparsable => {
            "Static analysis could not read this one — shell operators, loops, and redirections put it out of reach."
        }
        ReadOnlyBashBlocker::Default => {
            "This one is outside the read-only command set, or carries an argument that can write."
        }
    };
    format!("{LEAD}{detail}{REMEDY}")
}

/// The classifier seam: `None` means the command is read-only; a blocker
/// names what disqualified it. The real permission classifier is owned by the
/// safety slice; this card consumes its verdict.
pub trait ReadOnlyBashClassifier: Send + Sync {
    fn blocker(&self, command: &str) -> Option<ReadOnlyBashBlocker>;
}

/// The read-only bash wrapper card registered inside sub-agent boundaries.
#[derive(Clone)]
pub struct ReadOnlyBashTool {
    inner: Option<Arc<dyn Tool>>,
    classifier: Option<Arc<dyn ReadOnlyBashClassifier>>,
    description: String,
}

impl ReadOnlyBashTool {
    /// Builds the wrapper over an optional inner tool and classifier seam.
    pub fn new(
        inner: Option<Arc<dyn Tool>>,
        classifier: Option<Arc<dyn ReadOnlyBashClassifier>>,
    ) -> Self {
        let description = inner
            .as_ref()
            .map(|inner| readonly_bash_description(inner.description()))
            .unwrap_or_else(|| readonly_bash_description(""));
        Self {
            inner,
            classifier,
            description,
        }
    }
}

impl Tool for ReadOnlyBashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> Value {
        json!({"type":"object","properties":{"command":{"type":"string","description":"Read-only shell command to execute in the foreground. Must match the permission-layer read-only command policy."}},"required":["command"]})
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let command = args
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        let Some(classifier) = &self.classifier else {
            return Err(invalid_err(
                "read-only bash classifier seam is not wired in this slice",
            ));
        };
        if let Some(blocker) = classifier.blocker(&command) {
            return Err(Box::new(crate::refusal::blocked(readonly_block_reason(
                blocker,
            ))));
        }
        let Some(inner) = &self.inner else {
            return Err(invalid_err(
                "bash inner tool seam is not wired in this slice",
            ));
        };
        inner.execute(ctx, args)
    }

    fn read_only(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn invalid_err(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(std::io::Error::other(message.into()))
}
