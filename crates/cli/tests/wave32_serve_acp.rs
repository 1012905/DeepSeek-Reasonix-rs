//! `serve`/`web` broker and resume guards, and `acp` enum validations:
//! CLI-025, CLI-026. The server kernels are unmigrated anchors; only parse,
//! validation, stream, and exit-code behavior is decidable here.

use reasonix_cli::text::{
    ACP_BASH_ERROR, ACP_NETWORK_ERROR, ACP_PLANNER_ERROR, BROKER_TOKEN_FILE_ERROR,
    BROKER_TWICE_ERROR, PLAN_REQUIRES_INTERACTIVE, SERVE_RESUME_SESSION_ID_CONFLICT,
};
use reasonix_cli::{BuildInfo, CliEnv, CliOutcome, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str]) -> CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// `serve` and `web` both land on the serve route; `web` keeps the `web` flag.
#[test]
fn serve_and_web_routes_carry_web_flag() {
    let serve = run(&["serve"]);
    match serve.route {
        Route::Serve { web, .. } => assert!(!web),
        other => panic!("expected Serve route, got {other:?}"),
    }
    let web = run(&["web"]);
    match web.route {
        Route::Serve { web, .. } => assert!(web),
        other => panic!("expected Serve route, got {other:?}"),
    }
}

/// `--resume` and `--session-id` cannot name two session starts together.
#[test]
fn serve_resume_and_session_id_conflict() {
    let out = run(&["serve", "--resume", "one", "--session-id", "two"]);
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stderr,
        format!("error: {SERVE_RESUME_SESSION_ID_CONFLICT}\n")
    );
}

/// Naming the broker by both path forms is rejected.
#[test]
fn serve_broker_twice_rejected() {
    let out = run(&[
        "serve",
        "--provider-broker",
        "broker.toml",
        "--provider-broker-file",
        "other.toml",
    ]);
    assert_eq!(out.code, 2);
    assert_eq!(out.stderr, format!("error: {BROKER_TWICE_ERROR}\n"));
}

/// A broker given by name still needs its token file.
#[test]
fn serve_broker_without_token_file_rejected() {
    let out = run(&["serve", "--provider-broker", "broker.toml"]);
    assert_eq!(out.code, 2);
    assert_eq!(out.stderr, format!("error: {BROKER_TOKEN_FILE_ERROR}\n"));
}

/// ACP planner/sandbox-network/sandbox-bash each validate against their enum.
#[test]
fn acp_rejects_invalid_enums() {
    let cases: &[(&[&str], &str)] = &[
        (&["acp", "--planner", "always"], ACP_PLANNER_ERROR),
        (&["acp", "--sandbox-network", "isolated"], ACP_NETWORK_ERROR),
        (&["acp", "--sandbox-bash", "forbid"], ACP_BASH_ERROR),
    ];
    for (argv, msg) in cases {
        let out = run(argv);
        assert_eq!(out.code, 2, "{argv:?}");
        assert_eq!(out.stderr, format!("error: {msg}\n"), "{argv:?}");
    }
}

/// ACP is headless: `--permission-mode plan` is refused.
#[test]
fn acp_rejects_plan_mode() {
    let out = run(&["acp", "--permission-mode", "plan"]);
    assert_eq!(out.code, 2);
    assert_eq!(out.stderr, format!("error: {PLAN_REQUIRES_INTERACTIVE}\n"));
}

/// A clean ACP argv routes to the ACP anchor.
#[test]
fn acp_clean_argv_routes_to_acp() {
    let out = run(&["acp", "--planner", "off", "--sandbox-network", "on"]);
    assert!(matches!(out.route, Route::Acp));
}
