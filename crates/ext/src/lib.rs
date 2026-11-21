//! The extension system.
//!
//! Parity target: `reference/.../internal/ext` — MCP clients (stdio and
//! HTTP), plugins, skills, hooks, and themes, plus the extension protocol v2
//! surface: manifest, handshake, stream, UI, drain, and receipt
//! (AGENTS.md 5.1).
//! Contents land slice by slice (see `docs/parity/LEDGER.md`).
//!
//! EXT-001..005 carry the Extension Protocol v2 wire boundary: NDJSON
//! framing, the initialize handshake, content refs, interception, and
//! streaming providers (docs/contracts/ext.md §1-5).

#![forbid(unsafe_code)]

pub mod content;
pub mod enums;
pub mod error;
pub mod framing;
pub mod intercept;
pub mod lifecycle;
pub mod limits;
pub mod provider;
pub mod rawjson;
pub mod ui;
