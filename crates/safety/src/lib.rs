//! Security invariants for the reasonix kernel.
//!
//! Parity target: `reference/.../internal/safety` — path validation, shell
//! AST rules, redaction, and SSRF/egress guards. Checks here may be stricter
//! than the reference but never weaker (AGENTS.md 3.4).
//! Contents land slice by slice (see `docs/parity/LEDGER.md`).

#![forbid(unsafe_code)]
