//! Hidden `sandbox list|revoke`: CLI-017.

use reasonix_cli::text::SANDBOX_USAGE;
use reasonix_cli::{BuildInfo, CliEnv, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str]) -> reasonix_cli::CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// `list` and `revoke` are the recognized hidden verbs; the grant store itself
/// is the unmigrated Windows body.
#[test]
fn sandbox_list_and_revoke_are_routed() {
    for verb in ["list", "revoke"] {
        let out = run(&["sandbox", verb]);
        assert!(matches!(out.route, Route::Sandbox { verb: v } if v == verb));
    }
}

/// Anything else (empty, unknown verb, extra argument) is a usage error on
/// stderr with the sandbox usage line, exit 2.
#[test]
fn sandbox_other_arguments_usage_two() {
    let cases: Vec<&[&str]> = vec![
        &["sandbox"],
        &["sandbox", "frob"],
        &["sandbox", "list", "extra"],
    ];
    for argv in cases {
        let out = run(argv);
        assert!(out.stdout.is_empty());
        assert_eq!(out.stderr, format!("{SANDBOX_USAGE}\n"));
        assert_eq!(out.code, 2, "{argv:?}");
    }
}
