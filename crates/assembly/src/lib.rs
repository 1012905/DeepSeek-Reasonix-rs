//! Composition root wiring configuration, providers, tools, sessions, and
//! frontends into the runnable binaries.
//!
//! Parity target: `reference/.../internal/assembly` — `boot` builds the
//! controller stack the frontends drive. The Rust workspace keeps the same
//! position in the dependency graph (AGENTS.md 4.2): frontends and sessions
//! below, binaries above (ADR-0001).

#![forbid(unsafe_code)]

pub mod boot;
pub mod emit;
pub mod host;
