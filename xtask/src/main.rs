//! Repository automation (not part of any shipped binary).
//!
//! Usage (AGENTS.md 8.6):
//!     cargo xtask parity --check    validate docs/parity/LEDGER.md
//!     cargo xtask deps  --check     enforce the crate dependency direction (ADR-0001)

#![forbid(unsafe_code)]

mod deps;
mod ledger;
mod manifest;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.split_first() {
        Some((cmd, rest)) => match cmd.as_str() {
            "parity" => ledger::run(rest),
            "deps" => deps::run(rest),
            _ => usage(),
        },
        None => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!("usage: cargo xtask <parity|deps> [--check]");
    ExitCode::FAILURE
}
