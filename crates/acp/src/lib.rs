//! The Agent Client Protocol (ACP) frontend.
//!
//! Parity target: `reference/.../internal/frontend/acp` and `docs/ACP.md`
//! — stdio framing, message shapes, and the official TypeScript SDK
//! conformance suite as the acceptance bar (AGENTS.md 8.2).
//! Contents land slice by slice (see `docs/parity/LEDGER.md`).

#![forbid(unsafe_code)]
