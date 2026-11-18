//! Frozen wire and data contracts for the reasonix kernel.
//!
//! Parity target: `reference/.../internal/contract` — the config schema,
//! event envelopes, provider and tool surfaces, and error codes that every
//! other crate speaks. These types are the compatibility surface with the
//! reference implementation: field names, JSON shapes, and versioning are
//! frozen before implementation work starts (AGENTS.md 5.1).
//! Contents land slice by slice (see `docs/parity/LEDGER.md`).

#![forbid(unsafe_code)]

pub mod agentcomposition;
pub mod gojson;
pub mod provider;

// Wire-type slices carried from the earlier contract crate: their golden
// eventwire parity gate is ahead of the ledger's SSE-004, so they stay.
pub mod agentgraph;
pub mod delivery;
pub mod eventwire;
pub mod pricing;
pub mod surfacespec;
