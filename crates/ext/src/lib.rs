//! The extension system.
//!
//! Parity target: `reference/.../internal/ext` — MCP clients (stdio and
//! HTTP), plugins, skills, hooks, and themes, plus the extension protocol v2
//! surface: manifest, handshake, stream, UI, drain, and receipt
//! (AGENTS.md 5.1).
//! Contents land slice by slice (see `docs/parity/LEDGER.md`).

#![forbid(unsafe_code)]
