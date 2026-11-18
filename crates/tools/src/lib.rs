//! The built-in tool catalog.
//!
//! Parity target: `reference/.../internal/tools` — every tool's schema,
//! approval posture, output limits, truncation/spill behavior, and the shell
//! AST safety checks. Tool names, argument schemas, and result shapes are
//! frozen contracts (AGENTS.md 5.1).
//! Contents land slice by slice (see `docs/parity/LEDGER.md`).

#![forbid(unsafe_code)]
