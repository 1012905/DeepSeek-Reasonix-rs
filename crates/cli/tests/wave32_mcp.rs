//! `mcp` verb matrix, usage stream ownership, and browse option bounds:
//! CLI-027.

use reasonix_cli::text::{
    MCP_BROWSE_LIMIT_RANGE, MCP_BROWSE_LIMIT_VALUE, MCP_BROWSE_ONE_QUERY, MCP_USAGE,
};
use reasonix_cli::{BuildInfo, CliEnv, CliOutcome, Route};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str]) -> CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// A bare `mcp` prints its usage to stdout and exits 2 (mcp.go:32-35).
#[test]
fn mcp_bare_usage_stdout_exit_two() {
    let out = run(&["mcp"]);
    assert_eq!(out.stdout, format!("{MCP_USAGE}\n"));
    assert!(out.stderr.is_empty());
    assert_eq!(out.code, 2);
}

/// `mcp help` prints the usage to stdout, exit 0.
#[test]
fn mcp_help_usage_stdout_exit_zero() {
    let out = run(&["mcp", "help"]);
    assert_eq!(out.stdout, format!("{MCP_USAGE}\n"));
    assert_eq!(out.code, 0);
}

/// An unknown subcommand prints the usage to stdout and the diagnostic to
/// stderr, exit 2.
#[test]
fn mcp_unknown_subcommand_stderr_usage_exit_two() {
    let out = run(&["mcp", "frobnicate"]);
    assert_eq!(out.stdout, format!("{MCP_USAGE}\n"));
    assert_eq!(out.stderr, "unknown mcp subcommand \"frobnicate\"\n\n");
    assert_eq!(out.code, 2);
}

/// `list`/`ls` take no extra argument.
#[test]
fn mcp_list_rejects_extra_argument() {
    for verb in ["list", "ls"] {
        let out = run(&["mcp", verb, "extra"]);
        assert_eq!(out.code, 2, "{verb}");
        assert_eq!(
            out.stderr, "mcp list: unexpected argument \"extra\"\n",
            "{verb}"
        );
    }
}

/// `import` takes no extra argument.
#[test]
fn mcp_import_rejects_extra_argument() {
    let out = run(&["mcp", "import", "extra"]);
    assert_eq!(out.code, 2);
    assert_eq!(out.stderr, "mcp import: unexpected argument \"extra\"\n");
}

/// Browse `--limit` must be an integer in 1..=100.
#[test]
fn mcp_browse_limit_bounds() {
    let below = run(&["mcp", "browse", "--limit", "0"]);
    assert_eq!(below.code, 2);
    assert_eq!(below.stderr, format!("{MCP_BROWSE_LIMIT_RANGE}\n"));

    let above = run(&["mcp", "search", "--limit=101"]);
    assert_eq!(above.code, 2);
    assert_eq!(above.stderr, format!("{MCP_BROWSE_LIMIT_RANGE}\n"));

    let missing = run(&["mcp", "browse", "--limit"]);
    assert_eq!(missing.code, 2);
    assert_eq!(missing.stderr, format!("{MCP_BROWSE_LIMIT_VALUE}\n"));
}

/// Browse rejects unknown flags and more than one query.
#[test]
fn mcp_browse_rejects_unknown_flag_and_two_queries() {
    let flag = run(&["mcp", "browse", "--bogus", "q"]);
    assert_eq!(flag.code, 2);
    assert!(
        flag.stderr
            .starts_with("mcp browse: unknown flag \"--bogus\"")
    );

    let two = run(&["mcp", "browse", "one", "two"]);
    assert_eq!(two.code, 2);
    assert_eq!(two.stderr, format!("{MCP_BROWSE_ONE_QUERY}\n"));
}

/// A valid browse invocation routes to the unmigrated kernel anchor.
#[test]
fn mcp_browse_valid_routes_through() {
    let out = run(&["mcp", "browse", "filesystem", "--limit", "5"]);
    assert!(matches!(out.route, Route::Verb { command, .. } if command == "mcp"));
}

/// Known write verbs route through without a parse-layer rejection.
#[test]
fn mcp_known_verbs_route_through() {
    for verb in [
        "add",
        "get",
        "remove",
        "rm",
        "enable",
        "disable",
        "retry",
        "connect",
        "trust",
        "auth",
        "authorize",
        "update",
        "install",
    ] {
        let out = run(&["mcp", verb]);
        assert!(
            !matches!(out.route, Route::Unknown { .. }),
            "{verb}: {out:?}"
        );
    }
}
