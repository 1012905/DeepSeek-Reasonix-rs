//! Persistent session state: the file-backed session store and the SQLite
//! surface.
//!
//! Parity target: `reference/.../internal/state`. The on-disk session
//! format (transcripts, event logs, sidecars, checkpoints, memory, inbox)
//! is frozen in `docs/contracts/session-format.md`; the SQLite
//! contract is frozen in `docs/contracts/state.md`. Writes must stay
//! idempotent and backward-compatible with data created by the reference
//! implementation (AGENTS.md 6.4).
//!
//! Contents land slice by slice (see `docs/parity/LEDGER.md`).

#![forbid(unsafe_code)]

pub mod sessionstore;
pub mod store;
