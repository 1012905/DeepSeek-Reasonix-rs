//! Version surface: CLI-001..CLI-006b. Drives the pure `dispatch` entry and
//! asserts stdout bytes, stderr bytes, and exit codes directly.

use reasonix_cli::{BuildInfo, CliEnv, Route};

fn info() -> BuildInfo {
    BuildInfo {
        version: "v1.0.0".into(),
        git_commit: String::new(),
        build_time_utc: String::new(),
        build_target: String::new(),
    }
}

fn dispatch(args: &[&str]) -> reasonix_cli::CliOutcome {
    let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// CLI-001: `reasonix --version` prints the single line and nothing else.
#[test]
fn version_flag_prints_single_line() {
    let out = dispatch(&["--version"]);
    assert_eq!(out.stdout, "reasonix v1.0.0\n");
    assert!(out.stderr.is_empty(), "stderr: {}", out.stderr);
    assert_eq!(out.code, 0);
    assert_eq!(out.route, Route::Version);
}

/// CLI-002: `-v` matches `--version` byte for byte.
#[test]
fn short_v_matches_long_version_flag() {
    let long = dispatch(&["--version"]);
    let short = dispatch(&["-v"]);
    assert_eq!(short.stdout, long.stdout);
    assert_eq!(short.stderr, long.stderr);
    assert_eq!(short.code, long.code);
}

/// CLI-003: bare `reasonix version` prints the same single line.
#[test]
fn version_subcommand_prints_single_line() {
    let out = dispatch(&["version"]);
    assert_eq!(out.stdout, "reasonix v1.0.0\n");
    assert!(out.stderr.is_empty());
    assert_eq!(out.code, 0);
}

/// CLI-001: stray tokens after a top-level `--version` are ignored; the line
/// stays single.
#[test]
fn top_level_version_flag_ignores_trailing_tokens() {
    let out = dispatch(&["--version", "--verbose"]);
    assert_eq!(out.stdout, "reasonix v1.0.0\n");
    assert!(out.stderr.is_empty());
    assert_eq!(out.code, 0);
}

/// CLI-004: `version --verbose` lists fields in the fixed order; default
/// `unknown` values are retained on the verbose path.
#[test]
fn version_verbose_lists_build_fields_in_order() {
    let detailed = BuildInfo {
        version: "v9.9.9".into(),
        git_commit: "abcdef0123456789".into(),
        build_time_utc: "2026-08-06T12:00:00Z".into(),
        build_target: "darwin/arm64".into(),
    };
    let owned: Vec<String> = vec!["version".into(), "--verbose".into()];
    let out = reasonix_cli::dispatch(&owned, &detailed, &CliEnv::pipe());
    let lines: Vec<&str> = out.stdout.trim_end().split('\n').collect();
    assert_eq!(
        lines,
        vec![
            "reasonix v9.9.9",
            "git_commit: abcdef0123456789",
            "build_time_utc: 2026-08-06T12:00:00Z",
            "build_target: darwin/arm64",
            "go: unknown",
        ]
    );
    assert!(out.stderr.is_empty());
    assert_eq!(out.code, 0);
}

/// CLI-004: the `-verbose` shorthand behaves like `--verbose`.
#[test]
fn version_verbose_short_alias() {
    let owned: Vec<String> = vec!["version".into(), "-verbose".into()];
    let out = reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe());
    assert!(out.stdout.starts_with("reasonix v1.0.0\n"));
    assert!(out.stdout.contains("git_commit: unknown"));
    assert!(out.stdout.contains("build_time_utc: unknown"));
    assert_eq!(out.code, 0);
}

/// CLI-005: `version --json` emits the indented object; empty/unknown metadata
/// and the runtime field are omitted.
#[test]
fn version_json_payload_fields_and_omitempty() {
    let detailed = BuildInfo {
        version: "v1.0.0".into(),
        git_commit: "deadbeefcafebabe".into(),
        build_time_utc: "2026-01-02T03:04:05Z".into(),
        build_target: "linux/amd64".into(),
    };
    let owned: Vec<String> = vec!["version".into(), "--json".into()];
    let out = reasonix_cli::dispatch(&owned, &detailed, &CliEnv::pipe());
    assert_eq!(
        out.stdout,
        "{\n  \"version\": \"v1.0.0\",\n  \"git_commit\": \"deadbeefcafebabe\",\n  \"build_time_utc\": \"2026-01-02T03:04:05Z\",\n  \"build_target\": \"linux/amd64\"\n}\n"
    );
    assert!(out.stderr.is_empty());
    assert_eq!(out.code, 0);
}

/// CLI-005: `unknown` commit/build time drop out of the JSON payload, while
/// `build_target` always serializes; the Rust runtime field stays omitted.
#[test]
fn version_json_drops_unknown_metadata() {
    let out = dispatch(&["version", "--json"]);
    assert!(out.stdout.contains("\"version\": \"v1.0.0\""));
    assert!(out.stdout.contains("\"build_target\":"));
    assert!(!out.stdout.contains("git_commit"));
    assert!(!out.stdout.contains("build_time_utc"));
    assert!(!out.stdout.contains("\"go\""));
    assert_eq!(out.code, 0);
}

/// CLI-006: `version --help` prints versionUsage to stdout, exit 0.
#[test]
fn version_help_usage_stdout_zero() {
    for flag in ["--help", "-h"] {
        let out = dispatch(&["version", flag]);
        assert!(out.stdout.starts_with("Usage:\n  reasonix version\n"));
        assert!(out.stderr.is_empty(), "{flag}: {}", out.stderr);
        assert_eq!(out.code, 0, "{flag}");
    }
}

/// CLI-006a: an unknown version flag names the token on stderr, appends the
/// version usage there, and exits 2.
#[test]
fn version_unknown_flag_exit_two() {
    let out = dispatch(&["version", "--bogus"]);
    assert!(out.stdout.is_empty(), "stdout: {}", out.stdout);
    assert!(out.stderr.starts_with("unknown version flag \"--bogus\"\n"));
    assert!(out.stderr.contains("Usage:\n  reasonix version\n"));
    assert_eq!(out.code, 2);
}

/// CLI-006b: `--json` and `--verbose` are mutually exclusive.
#[test]
fn version_json_verbose_mutex() {
    for argv in [["--json", "--verbose"], ["--verbose", "--json"]] {
        let out = dispatch(&["version", argv[0], argv[1]]);
        assert!(out.stdout.is_empty());
        assert_eq!(
            out.stderr,
            "error: --json and --verbose are mutually exclusive\n"
        );
        assert_eq!(out.code, 2);
    }
}
