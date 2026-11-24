//! reasonix command-line entry: subcommand routing, flag parsing, and exit
//! codes.
//!
//! Parity target: `reference/.../internal/frontend/cli` (tracked row by row in
//! `docs/parity/LEDGER.md`). Parse, route, and render are pure functions that
//! return a structured [`CliOutcome`] (stdout text, stderr text, exit code,
//! parsed flags); [`run_with_build_info`] is the thin process wrapper that
//! prints it. Surfaces whose kernel handler has not migrated yet route to
//! [`Route::Unmigrated`] instead of shipping half-behavior: every surface lands
//! as a complete vertical slice, and the ledger says which slices exist.

#![forbid(unsafe_code)]

pub mod approval;
pub mod build_info;
pub mod env;
pub mod flags;
pub mod permission;
pub mod preset;
pub mod route;
pub mod text;

pub use build_info::{BuildInfo, DEFAULT_VERSION};
pub use route::{CliEnv, CliOutcome, Route, dispatch};

/// CLI entry point; returns the process exit code.
///
/// Mirrors the top-level dispatch of the reference `RunWithBuildInfo`: a
/// `--version`/`-v` before the verb stays single-line for script
/// compatibility, `version` accepts its own flags, and everything else routes
/// by subcommand name.
pub fn run_with_build_info(args: &[String], info: BuildInfo) -> i32 {
    let env = CliEnv::detected();
    let outcome = dispatch(args, &info, &env);
    print!("{}", outcome.stdout);
    eprint!("{}", outcome.stderr);
    outcome.code
}
