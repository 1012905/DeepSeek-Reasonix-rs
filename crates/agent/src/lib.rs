//! The agent loop.
//!
//! Parity target: `reference/.../internal/runtime` — turns, approval flow,
//! budgets, cancellation, delegation, and compaction. Cancellation semantics
//! must match the reference under every race the test suite can provoke
//! (AGENTS.md 12, risk table row 1).
//! Contents land slice by slice (see `docs/parity/LEDGER.md`).

#![forbid(unsafe_code)]
