//! `config` verb validations: language/currency enums, compact-ratio bounds,
//! and the `--local` ownership guards: CLI-039. The config read/write round
//! trip is the unmigrated backend anchor.

use reasonix_cli::text::{CONFIG_AUTO_PLAN_LOCAL_ERROR, CONFIG_CURRENCY_LOCAL_ERROR, CONFIG_USAGE};
use reasonix_cli::{BuildInfo, CliEnv, CliOutcome, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str]) -> CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// Bare and unknown verbs both print the shared config usage to stdout, exit 2
/// (cli.go:1346-1348).
#[test]
fn config_usage_goes_to_stdout_exit_two() {
    let cases: &[&[&str]] = &[&["config"], &["config", "frobnicate"]];
    for argv in cases {
        let out = run(argv);
        assert_eq!(out.stdout, format!("{CONFIG_USAGE}\n"), "{argv:?}");
        assert!(out.stderr.is_empty(), "{argv:?}");
        assert_eq!(out.code, 2, "{argv:?}");
    }
}

/// Currency is user-level only: `--local` is rejected.
#[test]
fn config_currency_local_rejected() {
    let out = run(&["config", "currency", "--local", "USD"]);
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stderr,
        format!("error: {CONFIG_CURRENCY_LOCAL_ERROR}\n")
    );
}

/// Currency rejects values outside auto|CNY|USD, quoting the raw input.
#[test]
fn config_currency_rejects_unknown_value() {
    let out = run(&["config", "currency", "eur"]);
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stderr,
        "error: pricing currency \"eur\": must be auto|CNY|USD\n"
    );
}

/// Reasoning language rejects values outside auto|zh|en.
#[test]
fn config_language_rejects_unknown_value() {
    let out = run(&["config", "reasoning-language", "fr"]);
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stderr,
        "error: reasoning_language \"fr\": must be auto|zh|en\n"
    );
}

/// Auto-plan is user-level only: `--local` is rejected.
#[test]
fn config_auto_plan_local_rejected() {
    let out = run(&["config", "auto-plan", "--local"]);
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stderr,
        format!("error: {CONFIG_AUTO_PLAN_LOCAL_ERROR}\n")
    );
}

/// Compact ratio must be above 0 and below 100; zero and 100 both fail.
#[test]
fn config_compact_ratio_bounds() {
    for raw in ["0", "100", "150", "abc"] {
        let out = run(&["config", "compact-ratio", raw]);
        assert_eq!(out.code, 2, "{raw}");
        assert!(
            out.stderr
                .starts_with("error: compact ratio must be a percentage above 0 and below 100:"),
            "{raw}: {}",
            out.stderr
        );
    }
}

/// More than one positional value prints the per-verb usage, exit 2.
#[test]
fn config_rejects_two_values() {
    let out = run(&["config", "currency", "CNY", "USD"]);
    assert_eq!(out.code, 2);
    assert!(out.stderr.starts_with("Usage:"));
}

/// Valid spellings route to the config anchor.
#[test]
fn config_valid_values_route_through() {
    let cases: &[&[&str]] = &[
        &["config", "currency", "CNY"],
        &["config", "reasoning-language", "zh"],
        &["config", "compact-ratio", "35"],
        &["config", "telemetry", "on"],
        &["config", "auto-plan"],
    ];
    for argv in cases {
        let out = run(argv);
        assert!(
            matches!(out.route, Route::Verb { command, .. } if command == "config"),
            "{argv:?}"
        );
    }
}
