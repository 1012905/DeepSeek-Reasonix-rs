//! Command reasonix is a config- and plugin-driven coding agent CLI.
//!
//! Rust rewrite of `reference/.../cmd/reasonix`: argument capture, the
//! crash-report wrapper, and kernel bootstrap live in `reasonix-assembly`;
//! this entry stays a thin process shim (ADR-0001).

#![forbid(unsafe_code)]

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = reasonix_assembly::boot::run_cli(&args);
    std::process::ExitCode::from(code.clamp(0, 255) as u8)
}
