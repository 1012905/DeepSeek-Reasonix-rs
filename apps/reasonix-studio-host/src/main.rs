//! Command reasonix-studio-host runs Studio's kernel behind a loopback
//! socket.
//!
//! Rust rewrite of `reference/.../cmd/reasonix-studio-host`. The desktop
//! handshake (first stdout line carries `origin` and `token`) and the
//! HTTP/SSE surface land with the studio-host slice (ledger HOST rows, M4).
//! This M0 build proves the binary name, link, and failure posture only:
//! the reference exits 2 on undefined flags, so the stub reports itself and
//! exits 2 for every invocation.

#![forbid(unsafe_code)]

fn main() -> std::process::ExitCode {
    eprintln!(
        "reasonix-studio-host: this build does not implement the Studio host yet \
         (see docs/parity/LEDGER.md)"
    );
    std::process::ExitCode::from(2)
}
