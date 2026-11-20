//! Foundational primitives for the reasonix kernel.
//!
//! Parity target: `reference/.../internal/base` — small self-contained
//! utilities (file encoding helpers, i18n primitives, DNS bootstrap, HTTP
//! client plumbing) that depend on no domain crate. Everything in the kernel
//! may depend on this crate; it may depend on nothing domain-specific.
//! Contents land slice by slice (see `docs/parity/LEDGER.md`).

#![forbid(unsafe_code)]

pub mod fileencoding;
pub mod filelock;
pub mod fileutil;
pub mod secrets;
pub mod shellparse;
pub mod shellparse_analysis;
