//! Non-session command flag tables: serve/web, ACP, upgrade, review, and the
//! shared machine-interface options.
//!
//! Parity targets: `serve_frontend.go:247-280`, `acp.go:39-46`,
//! `upgrade.go:101-103`, `review.go:23-27`, and the machine option parsers in
//! `session_machine.go` / `hook_machine.go` / `task_machine.go`.

use super::{FlagKind, FlagSpec};

fn string(name: &'static str) -> FlagSpec {
    FlagSpec {
        name,
        short: None,
        kind: FlagKind::String,
        optional_value: false,
    }
}

fn boolean(name: &'static str) -> FlagSpec {
    FlagSpec {
        name,
        short: None,
        kind: FlagKind::Bool,
        optional_value: false,
    }
}

/// Shared `serve`/`web` flag set (`runServeWithOptions`). `session-id`
/// registers under `web` only; the parser keeps it in the set and the web
/// handler alone reads it.
pub(crate) fn serve_specs() -> Vec<FlagSpec> {
    vec![
        string("model"),
        string("profile"),
        string("preset"),
        string("agent-composition"),
        FlagSpec {
            name: "max-steps",
            short: None,
            kind: FlagKind::Int,
            optional_value: false,
        },
        string("addr"),
        string("resume"),
        string("session-id"),
        string("auth"),
        string("token"),
        string("token-file"),
        string("password"),
        string("password-file"),
        boolean("hash-password"),
        boolean("allow-open-network"),
        boolean("behind-proxy"),
        string("public-url"),
        string("port-file"),
        string("pid-file"),
        string("page"),
        string("dir"),
        string("provider-broker"),
        string("provider-broker-file"),
        string("provider-broker-token-file"),
        boolean("open"),
        boolean("no-open"),
    ]
}

/// `acp` flag set (`acpCommand`).
pub(crate) fn acp_specs() -> Vec<FlagSpec> {
    vec![
        string("model"),
        string("profile"),
        string("planner"),
        string("sandbox-network"),
        string("sandbox-bash"),
        string("permission-mode"),
        boolean("workspace-only"),
    ]
}

/// `review` flag set (`reviewCommand`).
pub(crate) fn review_specs() -> Vec<FlagSpec> {
    vec![
        string("base"),
        string("commit"),
        string("model"),
        string("instructions"),
    ]
}
