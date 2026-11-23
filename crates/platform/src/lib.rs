//! OS-facing platform primitives: process, signal, log, crash, and app identity.
//!
//! This crate owns the platform side of the path contract (paths.md): layouts
//! derived from roots the caller has already resolved. Root resolution — env
//! pins, `${VAR}` expansion, the home chain, and the user-config selection
//! chain — stays in reasonix-config; this crate never imports it (AGENTS.md 4.2,
//! platform -> {contract, core}).
#![forbid(unsafe_code)]

pub mod locks;
pub mod project_config;
pub mod state_entries;
pub mod studio_logs;
pub mod user_config;

mod path_clean;

/// Shared scratch space for tests. The workspace target directory is used
/// instead of `env::temp_dir()` because some test environments deny creating
/// directories under the system temp path (same approach as the session crate).
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::Path;

    pub(crate) fn tempdir() -> tempfile::TempDir {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("target")
            .join("test-tmp");
        std::fs::create_dir_all(&base).expect("create test-tmp base");
        tempfile::Builder::new()
            .prefix(".tmpplatform")
            .tempdir_in(&base)
            .expect("create platform test temp dir")
    }
}
