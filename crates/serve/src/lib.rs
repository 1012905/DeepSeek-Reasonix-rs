//! The HTTP/SSE loopback frontend.
//!
//! Parity target: `reference/.../internal/frontend/serve` and
//! `remotehost` — routes, status codes, JSON shapes, SSE event names and
//! ordering, and the same-origin static hosting of the Studio frontend.
//! Contents land slice by slice (see `docs/parity/LEDGER.md`).

#![forbid(unsafe_code)]
