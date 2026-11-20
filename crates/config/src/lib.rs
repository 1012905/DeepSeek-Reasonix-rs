//! TOML configuration: loading, layering, precedence, and secret references.
//!
//! Parity target: `reference/.../internal/contract/config` plus
//! `reasonix.example.toml` — every key, default, precedence rule, and
//! unknown-key behavior must match the reference exactly.
//! Contents land slice by slice (see `docs/parity/LEDGER.md`).

#![forbid(unsafe_code)]
