//! The HTTP/SSE loopback frontend.
//!
//! Parity target: `reference/.../internal/frontend/serve` — the loopback gate,
//! the `Reason` refusal envelope, the event broadcaster and its SSE transport,
//! the listener and page resolution, and the hub read-side routes.
//! Contents land slice by slice (see `docs/parity/LEDGER.md`).

#![forbid(unsafe_code)]

pub mod auth;
pub mod boot;
pub mod broadcaster;
pub mod eventstream;
pub mod gate;
pub mod listen;
pub mod page;
pub mod password;
pub mod refuse;
pub mod router;
pub mod sink;

// Host wiring surface (docs/contract.md S3): the four calls a host needs to
// open the loopback control plane.
pub use gate::{listen_loopback, loopback_origin, to_tokio_listener};
pub use listen::run_graceful_handler;
pub use page::find_page;

/// Shared scratch space for tests. Test temp directories live under the
/// workspace target because the system temp path is denied in some
/// environments (same approach as the session crate's tests).
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::Path;

    #[allow(dead_code)]
    pub(crate) fn tempdir() -> tempfile::TempDir {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("target")
            .join("test-tmp");
        std::fs::create_dir_all(&base).expect("create test-tmp base");
        tempfile::Builder::new()
            .prefix(".tmpserve")
            .tempdir_in(&base)
            .expect("create serve test temp dir")
    }
}
