//! Session orchestration above the agent loop.
//!
//! Parity target: `reference/.../internal/session` — the controller surface
//! both frontends and the desktop host drive: session lifecycle, resume,
//! leases, and event fan-out.
//! Contents land slice by slice (see `docs/parity/LEDGER.md`).

#![forbid(unsafe_code)]
