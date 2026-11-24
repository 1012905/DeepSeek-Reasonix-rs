//! `upgrade`/`update` channel parsing and syntax conflicts: CLI-043. The
//! network download and checksum verification are unmigrated backend anchors.

use reasonix_cli::text::{UPGRADE_CHANNEL_CONFLICT, UPGRADE_CHANNEL_VALUE, UPGRADE_ONE_POSITIONAL};
use reasonix_cli::{BuildInfo, CliEnv, CliOutcome, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str]) -> CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// At most one deprecated positional channel.
#[test]
fn upgrade_rejects_two_positionals() {
    let out = run(&["upgrade", "stable", "preview"]);
    assert_eq!(out.code, 2);
    assert_eq!(out.stderr, format!("error: {UPGRADE_ONE_POSITIONAL}\n"));
}

/// An unsupported channel names itself in the diagnostic, exit 2.
#[test]
fn upgrade_rejects_unsupported_channel() {
    let cases: &[&[&str]] = &[&["upgrade", "dev"], &["update", "--channel", "dev"]];
    for argv in cases {
        let out = run(argv);
        assert_eq!(out.code, 2, "{argv:?}");
        assert_eq!(
            out.stderr,
            "error: release channel \"dev\" is unsupported; Reasonix now uses the official release\n",
            "{argv:?}"
        );
    }
}

/// `--channel` and a positional naming different channels conflict.
#[test]
fn upgrade_rejects_channel_conflict() {
    let out = run(&["upgrade", "stable", "--channel", "preview"]);
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stderr,
        format!(
            "error: {}\n",
            UPGRADE_CHANNEL_CONFLICT
                .replace("%q", "\"stable\"")
                .replace("%q", "\"preview\"")
        )
    );
}

/// `--channel` requires a value.
#[test]
fn upgrade_channel_requires_value() {
    let out = run(&["upgrade", "--channel"]);
    assert_eq!(out.code, 2);
    assert_eq!(out.stderr, format!("error: {UPGRADE_CHANNEL_VALUE}\n"));
}

/// Normalized channels route to the upgrade anchor; `update` is the alias.
#[test]
fn upgrade_normalized_channels_route_through() {
    let cases: &[&[&str]] = &[
        &["upgrade"],
        &["upgrade", "stable"],
        &["upgrade", "--channel", "preview"],
        &["upgrade", "--check", "--force"],
        &["update", "canary"],
    ];
    for argv in cases {
        let out = run(argv);
        assert!(
            matches!(out.route, Route::Verb { command, .. } if command == "upgrade" || command == "update"),
            "{argv:?}"
        );
    }
}
