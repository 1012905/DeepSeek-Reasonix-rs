//! Hub read side: the remembered workspace list, the workspace tree, and the
//! session listings both frontends render, plus the controller write side
//! that starts and steers turns.
//!
//! Parity target: `reference/.../internal/state/workspacelist` plus
//! `internal/frontend/serve` (`hub_tree.go`, `sessions_list.go`, `hub.go`
//! Hub — its Open/publish subset included), and `internal/session/control`'s
//! turn group. The open machine builds runtimes from a typed [`hub::RuntimeSpec`]
//! and an installed kit factory; hubs without one answer open requests with a
//! typed refusal instead of a fake success (docs/contract.md §3, §4.1). Wire
//! shapes are frozen in docs/contracts/http.md §1.2/§7.

#![forbid(unsafe_code)]

pub mod controller;
pub mod fanout;
pub mod hub;
pub mod listing;
pub mod model;
pub mod paths;
pub mod title;
pub mod tree;
pub mod turns;
pub mod workspacelist;

/// Shared scratch space for tests. The workspace target directory is used
/// instead of `env::temp_dir()` because some test environments deny
/// creating directories under the system temp path (same approach as the
/// state crate's eventlog tests).
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
            .prefix(".tmpsession")
            .tempdir_in(&base)
            .expect("create session test temp dir")
    }
}
