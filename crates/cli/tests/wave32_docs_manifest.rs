//! `docs-manifest` flag-only guard: CLI-042. The embedded corpus manifest build
//! is the unmigrated backend anchor.

use reasonix_cli::text::DOCS_MANIFEST_FLAGS_ONLY;
use reasonix_cli::{BuildInfo, CliEnv, CliOutcome};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str]) -> CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// A positional argument is rejected; the command accepts flags only.
#[test]
fn docs_manifest_rejects_positional() {
    let out = run(&["docs-manifest", "corpus"]);
    assert_eq!(out.code, 2);
    assert_eq!(out.stderr, format!("{DOCS_MANIFEST_FLAGS_ONLY}\n"));
}

/// Flag forms parse and reach the manifest anchor.
#[test]
fn docs_manifest_flags_only_routes_through() {
    let out = run(&[
        "docs-manifest",
        "--verify-source",
        "docs",
        "--expect-version",
        "1.2.3",
    ]);
    assert!(matches!(
        out.route,
        reasonix_cli::Route::Verb { command, .. } if command == "docs-manifest"
    ));
}
