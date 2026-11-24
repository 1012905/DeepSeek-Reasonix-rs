//! Session-facing flag tables: the terminal UI's set, `run`'s set, and the
//! two unions used by leading-flag routing.
//!
//! Parity targets: `tui_flags.go:30-52`, `run_flags.go:23-53`, and
//! `cli_flags.go:247-284`.

use super::{CONTINUE, FlagKind, FlagSpec};

/// The terminal UI's flag set (reference `tui_flags.go:30-52`). The TUI has
/// no `--auto`; approval there goes through `--yolo` /
/// `--dangerously-skip-permissions`.
pub(crate) fn tui_specs() -> Vec<FlagSpec> {
    vec![
        FlagSpec {
            name: "model",
            short: None,
            kind: FlagKind::String,
            optional_value: false,
        },
        FlagSpec {
            name: "preset",
            short: None,
            kind: FlagKind::String,
            optional_value: false,
        },
        FlagSpec {
            name: "agent-composition",
            short: None,
            kind: FlagKind::String,
            optional_value: false,
        },
        FlagSpec {
            name: "profile",
            short: None,
            kind: FlagKind::String,
            optional_value: false,
        },
        FlagSpec {
            name: "dir",
            short: None,
            kind: FlagKind::String,
            optional_value: false,
        },
        FlagSpec {
            name: "inline",
            short: None,
            kind: FlagKind::Bool,
            optional_value: false,
        },
        CONTINUE,
        FlagSpec {
            name: "resume",
            short: Some('r'),
            kind: FlagKind::String,
            optional_value: true,
        },
        FlagSpec {
            name: "copy",
            short: None,
            kind: FlagKind::Bool,
            optional_value: false,
        },
        FlagSpec {
            name: "effort",
            short: None,
            kind: FlagKind::String,
            optional_value: false,
        },
        FlagSpec {
            name: "permission-mode",
            short: None,
            kind: FlagKind::String,
            optional_value: false,
        },
        FlagSpec {
            name: "yolo",
            short: None,
            kind: FlagKind::Bool,
            optional_value: false,
        },
        FlagSpec {
            name: "dangerously-skip-permissions",
            short: None,
            kind: FlagKind::Bool,
            optional_value: false,
        },
        FlagSpec {
            name: "max-steps",
            short: None,
            kind: FlagKind::Int,
            optional_value: false,
        },
        FlagSpec {
            name: "add-dir",
            short: None,
            kind: FlagKind::StringArray,
            optional_value: false,
        },
        FlagSpec {
            name: "allowed-tools",
            short: None,
            kind: FlagKind::StringArray,
            optional_value: false,
        },
        FlagSpec {
            name: "allowedTools",
            short: None,
            kind: FlagKind::StringArray,
            optional_value: false,
        },
    ]
}

/// `run`'s flag set (reference `run_flags.go:23-53`).
pub(crate) fn run_specs() -> Vec<FlagSpec> {
    vec![
        FlagSpec {
            name: "model",
            short: None,
            kind: FlagKind::String,
            optional_value: false,
        },
        FlagSpec {
            name: "profile",
            short: None,
            kind: FlagKind::String,
            optional_value: false,
        },
        FlagSpec {
            name: "preset",
            short: None,
            kind: FlagKind::String,
            optional_value: false,
        },
        FlagSpec {
            name: "agent-composition",
            short: None,
            kind: FlagKind::String,
            optional_value: false,
        },
        FlagSpec {
            name: "max-steps",
            short: None,
            kind: FlagKind::Int,
            optional_value: false,
        },
        FlagSpec {
            name: "show-thinking",
            short: None,
            kind: FlagKind::Bool,
            optional_value: false,
        },
        FlagSpec {
            name: "debug",
            short: None,
            kind: FlagKind::Bool,
            optional_value: true,
        },
        FlagSpec {
            name: "metrics",
            short: None,
            kind: FlagKind::String,
            optional_value: false,
        },
        FlagSpec {
            name: "trajectory",
            short: None,
            kind: FlagKind::String,
            optional_value: false,
        },
        FlagSpec {
            name: "ablate",
            short: None,
            kind: FlagKind::String,
            optional_value: false,
        },
        FlagSpec {
            name: "fold-index",
            short: None,
            kind: FlagKind::String,
            optional_value: false,
        },
        FlagSpec {
            name: "dir",
            short: None,
            kind: FlagKind::String,
            optional_value: false,
        },
        CONTINUE,
        FlagSpec {
            name: "resume",
            short: None,
            kind: FlagKind::String,
            optional_value: false,
        },
        FlagSpec {
            name: "takeover",
            short: None,
            kind: FlagKind::Bool,
            optional_value: false,
        },
        FlagSpec {
            name: "copy",
            short: None,
            kind: FlagKind::Bool,
            optional_value: false,
        },
        FlagSpec {
            name: "effort",
            short: None,
            kind: FlagKind::String,
            optional_value: false,
        },
        FlagSpec {
            name: "permission-mode",
            short: None,
            kind: FlagKind::String,
            optional_value: false,
        },
        FlagSpec {
            name: "auto",
            short: Some('y'),
            kind: FlagKind::Bool,
            optional_value: false,
        },
        FlagSpec {
            name: "yolo",
            short: None,
            kind: FlagKind::Bool,
            optional_value: false,
        },
        FlagSpec {
            name: "dangerously-skip-permissions",
            short: None,
            kind: FlagKind::Bool,
            optional_value: false,
        },
        FlagSpec {
            name: "print",
            short: Some('p'),
            kind: FlagKind::Bool,
            optional_value: false,
        },
        FlagSpec {
            name: "events-jsonl",
            short: None,
            kind: FlagKind::Bool,
            optional_value: false,
        },
        FlagSpec {
            name: "output-format",
            short: None,
            kind: FlagKind::String,
            optional_value: false,
        },
        FlagSpec {
            name: "add-dir",
            short: None,
            kind: FlagKind::StringArray,
            optional_value: false,
        },
        FlagSpec {
            name: "allowed-tools",
            short: None,
            kind: FlagKind::StringArray,
            optional_value: false,
        },
        FlagSpec {
            name: "allowedTools",
            short: None,
            kind: FlagKind::StringArray,
            optional_value: false,
        },
    ]
}

/// The session set used when deciding whether leading flags travel into
/// `run`: the TUI set plus `-y/--auto` and `-p/--print` (reference
/// `leadingFlagsIntoRun`).
pub(crate) fn session_specs() -> Vec<FlagSpec> {
    let mut specs = tui_specs();
    specs.push(FlagSpec {
        name: "auto",
        short: Some('y'),
        kind: FlagKind::Bool,
        optional_value: false,
    });
    specs.push(FlagSpec {
        name: "print",
        short: Some('p'),
        kind: FlagKind::Bool,
        optional_value: false,
    });
    specs
}

/// Union of the TUI and run sets, used to split leading flags from the verb.
pub(crate) fn union_specs() -> Vec<FlagSpec> {
    let mut specs = tui_specs();
    for spec in run_specs() {
        if !specs.iter().any(|s| s.name == spec.name) {
            specs.push(spec);
        }
    }
    specs
}
