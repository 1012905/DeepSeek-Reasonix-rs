//! Mirrors `internal/contract/eventwire` — the shared frontend JSON contract
//! for kernel events. `eventwire_manifest.json` is generated from the Go wire
//! types by reflection and is the field-set authority both parity gates read.

pub mod approval;
pub mod ask;
pub mod event;
pub mod extension;
pub mod guardian;
pub mod lease;
pub mod receipt;
pub mod tool;
pub mod usage;

/// The stable wire name of every kernel event kind, in `event.Kind` order.
pub const KIND_NAMES: &[&str] = &[
    "turn_started",
    "reasoning",
    "text",
    "message",
    "tool_dispatch",
    "tool_result",
    "usage",
    "notice",
    "phase",
    "approval_request",
    "ask_request",
    "turn_done",
    "compaction_started",
    "compaction_progress",
    "compaction_done",
    "tool_progress",
    "mcp_surface_ready",
    "retrying",
    "steer",
    "guardian_assessment",
    "extension_surface",
    "extension_status",
    "stream_attempt",
    "context_maintenance",
    "todo_progress",
    "workspace_lease",
    "workspace_changed",
    "turn_phase",
    "completion_summary",
    "adjudications_changed",
    "browser_tabs_changed",
    "inbox_changed",
    "rewind_changed",
    "graph_delta",
    "progress_watch",
    "ask_engaged",
];

/// Field-set authority for both parity gates: generated from the Go wire types
/// by reflection at recording time, read by the Rust and TS parity tests.
pub const MANIFEST: &str = include_str!("../eventwire_manifest.json");

pub(crate) mod util {
    // Go's omitempty: a false bool carries nothing a reader needs.
    pub fn is_false(v: &bool) -> bool {
        !*v
    }
    pub fn is_zero<T: Default + PartialEq>(v: &T) -> bool {
        *v == T::default()
    }
}
