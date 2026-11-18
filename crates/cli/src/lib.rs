//! reasonix command-line entry: subcommand routing, flag parsing, and exit
//! codes.
//!
//! Parity target: `reference/.../internal/frontend/cli` (tracked row by row
//! in `docs/parity/LEDGER.md`). The full surface is config-driven; this
//! crate currently wires the routing skeleton and the single-line
//! `--version` contract. Surfaces that are not implemented yet exit `2`
//! with an explicit notice instead of half-behavior: every surface ships as a
//! complete vertical slice, and the ledger says which slices exist.

#![forbid(unsafe_code)]

/// Version reported by bare source builds. Release builds inject the real
/// version via `REASONIX_VERSION` at compile time, mirroring the reference
/// `-ldflags` flow where `version` defaults to `"dev"`.
pub const DEFAULT_VERSION: &str = "dev";

/// Machine- and human-readable build identity for `reasonix version` and
/// `reasonix --version`.
///
/// Contract (reference `internal/frontend/cli/buildinfo.go`):
///   - `reasonix --version` / `-v` stay single-line: `reasonix <version>`
///   - `reasonix version --verbose` prints structured fields
///   - `reasonix version --json` prints the same as JSON
///
/// Fields are limited to version, commit, build time, and target; no config
/// paths and no fixed timezone conversions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInfo {
    /// Version string; the single-line contract for `--version`.
    pub version: String,
    /// Git commit hash, when injected at build time.
    pub git_commit: String,
    /// Build time (RFC 3339, UTC), when injected at build time.
    pub build_time_utc: String,
    /// Rust target triple, when injected at build time.
    pub build_target: String,
}

impl Default for BuildInfo {
    fn default() -> Self {
        Self::from_build()
    }
}

impl BuildInfo {
    /// Reads build identity from compile-time environment. A bare source
    /// build reports `dev` with empty metadata — identical to a reference
    /// binary built without `-ldflags`.
    pub fn from_build() -> Self {
        macro_rules! inject {
            ($var:literal, $default:expr) => {
                match option_env!($var) {
                    Some(value) if !value.trim().is_empty() => value.trim(),
                    _ => $default,
                }
            };
        }
        Self {
            version: inject!("REASONIX_VERSION", DEFAULT_VERSION).to_owned(),
            git_commit: inject!("REASONIX_GIT_COMMIT", "").to_owned(),
            build_time_utc: inject!("REASONIX_BUILD_TIME_UTC", "").to_owned(),
            build_target: inject!("REASONIX_BUILD_TARGET", "").to_owned(),
        }
    }

    /// Single-line form: `reasonix <version>`.
    pub fn single_line(&self) -> String {
        format!("reasonix {}", self.version.trim())
    }
}

/// CLI entry point; returns the process exit code.
///
/// Mirrors the top-level dispatch of the reference `RunWithBuildInfo`: a
/// `--version`/`-v` before the verb stays single-line for script
/// compatibility, `version` accepts its own flags, and everything else
/// routes by subcommand name.
pub fn run_with_build_info(args: &[String], info: BuildInfo) -> i32 {
    let cmd = args.first().map(String::as_str).unwrap_or_default();
    match cmd {
        "--version" | "-v" => {
            // Top-level flags stay single-line; no flag handling (reference
            // cli.go: versionCommand(nil, info, false)).
            println!("{}", info.single_line());
            0
        }
        "version" => version_command(&args[1..], &info),
        _ => not_implemented(cmd),
    }
}

/// `reasonix version [flags]`.
///
/// Only the flagless single-line form exists in this build; `--verbose`,
/// `--json`, and `--help` ship with the CLI matrix slice (ledger
/// CLI-004..CLI-006) once the `go` runtime-field parity break is decided
/// (`docs/parity/BREAKS.md`).
fn version_command(args: &[String], info: &BuildInfo) -> i32 {
    if args.is_empty() {
        println!("{}", info.single_line());
        return 0;
    }
    eprintln!(
        "reasonix: version flags are not implemented in this build (ledger CLI-004..CLI-006)"
    );
    2
}

/// Reports a surface that is not implemented yet. Exit code `2` matches the
/// reference command-line usage-error convention; exact per-command text is
/// frozen with each vertical slice.
fn not_implemented(cmd: &str) -> i32 {
    if cmd.is_empty() {
        eprintln!("reasonix: interactive mode is not implemented in this build (ledger CLI-008)");
    } else {
        eprintln!("reasonix: command not implemented in this build: {cmd}");
    }
    2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> BuildInfo {
        BuildInfo {
            version: "9.9.9-test".into(),
            git_commit: String::new(),
            build_time_utc: String::new(),
            build_target: String::new(),
        }
    }

    #[test]
    fn single_line_format_matches_reference() {
        assert_eq!(info().single_line(), "reasonix 9.9.9-test");
    }

    #[test]
    fn version_trims_whitespace() {
        let mut i = info();
        i.version = "  1.2.3  ".into();
        assert_eq!(i.single_line(), "reasonix 1.2.3");
    }

    #[test]
    fn default_build_info_reports_dev() {
        // Tests never inject REASONIX_VERSION, so the compile-time default
        // must surface exactly like a bare source build of the reference.
        assert_eq!(BuildInfo::from_build().version, DEFAULT_VERSION);
    }

    #[test]
    fn top_level_version_flag_is_single_line() {
        assert_eq!(run_with_build_info(&["--version".into()], info()), 0);
        assert_eq!(run_with_build_info(&["-v".into()], info()), 0);
    }

    #[test]
    fn version_subcommand_without_flags_is_single_line() {
        assert_eq!(run_with_build_info(&["version".into()], info()), 0);
    }

    #[test]
    fn version_subcommand_flags_exit_two() {
        assert_eq!(
            run_with_build_info(&["version".into(), "--verbose".into()], info()),
            2
        );
    }

    #[test]
    fn unknown_command_exits_two() {
        assert_eq!(run_with_build_info(&["nosuch".into()], info()), 2);
    }

    #[test]
    fn bare_invocation_exits_two_for_now() {
        assert_eq!(run_with_build_info(&[], info()), 2);
    }
}
