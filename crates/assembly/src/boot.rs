//! Process entry wiring shared by the `reasonix` and `reasonix-studio-host`
//! binaries.
//!
//! M0 wires the CLI skeleton only; bootstrapping a full controller from
//! configuration lands with the agent/session slices (see
//! `docs/parity/LEDGER.md`).

use reasonix_cli::{BuildInfo, run_with_build_info};

/// Runs the CLI with the build identity baked in at compile time.
pub fn run_cli(args: &[String]) -> i32 {
    run_with_build_info(args, BuildInfo::from_build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_routes_through_cli() {
        assert_eq!(run_cli(&["--version".to_owned()]), 0);
    }
}
