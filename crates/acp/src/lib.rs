//! # reasonix-acp
//!
//! Agent Client Protocol server-side protocol surface. This crate is a pure
//! synchronous library: frame codecs work on byte slices, JSON-RPC types are
//! plain serde structs, and dispatch routes through injectable handler traits.
//! No stdio runtime, no async executor, no subprocess spawning lives here.
//!
//! Modules follow the wire domain:
//!
//! * [`frame`] — NDJSON framing, 32 MiB per-line byte budget.
//! * [`errors`] — typed JSON-RPC error codes and the cancelled rewrite.
//! * [`jsonrpc`] — envelope classification and outbound frame builders.
//! * [`caps`] — `initialize` capabilities, `authenticate`, client capability parsing.
//! * [`wire`] — inbound session DTOs and MCP spec dual-shape decoding.
//! * [`fs_api`] — asymmetric filesystem proxy degradation.
//! * [`terminal`] — terminal lifecycle request shapes and exit classification.
//! * [`update`] — `session/update` variants.
//! * [`permission`] — permission round-trip, option kinds, posture ceiling.
//! * [`elicitation`] — `elicitation/create` URL/form params and reply shape.
//! * [`lifecycle`] — session state machine, lease code table, control-axis rules.
//! * [`extensions`] — vendor method registry, notification opt-in, inbox refusal map.
//! * [`dispatch`] — handler registry and event → update mapping table.
//! * [`cancel`] — in-flight request cancellation set and rewrite condition.
//!
//! Real session storage, lease acquisition, controller subprocesses, and the
//! agent event stream are runtime concerns outside this protocol crate; the
//! pure decision tables they consume are implemented here.

#![forbid(unsafe_code)]

pub mod cancel;
pub mod caps;
pub mod dispatch;
pub mod elicitation;
pub mod errors;
pub mod extensions;
pub mod frame;
pub mod fs_api;
pub mod jsonrpc;
pub mod lifecycle;
pub mod methods;
pub mod permission;
pub mod terminal;
pub mod update;
pub mod wire;
