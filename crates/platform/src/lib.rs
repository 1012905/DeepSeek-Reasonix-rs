//! OS surfaces for the reasonix kernel.
//!
//! Parity target: `reference/.../internal/platform` — process control
//! (Windows Job Objects and conpty, POSIX process groups), sandbox helpers
//! (macOS seatbelt, Linux bubblewrap, per reference support), keyring,
//! notifications, crash reporting, and update plumbing.
//! Platform-specific `unsafe` will be confined to audited blocks here and in
//! `reasonix-safety` (AGENTS.md 3.4).
//! Contents land slice by slice (see `docs/parity/LEDGER.md`).

#![forbid(unsafe_code)]
