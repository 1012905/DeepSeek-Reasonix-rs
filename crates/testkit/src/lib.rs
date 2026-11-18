//! Shared test utilities.
//!
//! Test-only crate: isolated `REASONIX_HOME` fixtures, golden-case
//! normalization, and mock providers. Never linked into shipped binaries;
//! exempt from the dependency-direction rules (AGENTS.md 4.2) and usable as a
//! dev-dependency by any crate.

#![forbid(unsafe_code)]
