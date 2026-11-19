//! TOML configuration: loading, layering, precedence, and secret references.
//!
//! Parity target: `reference/.../internal/contract/config` plus
//! `reasonix.example.toml` — every key, default, precedence rule, and
//! unknown-key behavior must match the reference exactly.
//! This slice carries the host-startup subset (docs/contract.md S1): layered
//! user → project loading with the frozen unknown-key passthrough, the [serve]
//! section and its auth normalization, the host's credential-free serve view
//! (HOST-018), the desktop telemetry/metrics/tray keys, the auto workspace
//! path, and the top-level `language` key.
//!
//! Contents land slice by slice (see `docs/parity/LEDGER.md`).

#![forbid(unsafe_code)]

pub mod desktop;
pub mod language;
pub mod load;
pub mod model;
pub mod paths;
pub mod serve;

pub use language::normalize_language;
pub use load::ConfigError;
pub use model::Config;
pub use paths::auto_workspace_dir;
pub use serve::{AuthMode, ServeSection, host_serve_section};
