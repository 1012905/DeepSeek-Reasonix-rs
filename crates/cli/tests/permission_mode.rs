//! Permission-mode parsing matrix and caller wrapping: CLI-020.

use reasonix_cli::permission::{ApprovalMode, parse_permission_mode};
use reasonix_cli::{BuildInfo, CliEnv};

fn info() -> BuildInfo {
    BuildInfo::from_build()
}

fn run(argv: &[&str]) -> reasonix_cli::CliOutcome {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    reasonix_cli::dispatch(&owned, &info(), &CliEnv::pipe())
}

/// All 17 accepted spellings map to the reference posture; matching is
/// case-insensitive and trims surrounding space.
#[test]
fn permission_mode_matrix_maps_accepted_values() {
    let cases: [(&str, ApprovalMode, bool); 17] = [
        ("", ApprovalMode::Ask, false),
        ("default", ApprovalMode::Ask, false),
        ("ask", ApprovalMode::Ask, false),
        ("read-only", ApprovalMode::ReadOnly, false),
        ("readonly", ApprovalMode::ReadOnly, false),
        ("auto", ApprovalMode::Auto, false),
        ("workspace-write", ApprovalMode::Auto, false),
        ("acceptedits", ApprovalMode::Ask, false),
        ("accept-edits", ApprovalMode::Ask, false),
        ("manual", ApprovalMode::Ask, false),
        ("dontask", ApprovalMode::DontAsk, false),
        ("dont-ask", ApprovalMode::DontAsk, false),
        ("plan", ApprovalMode::Ask, true),
        ("bypasspermissions", ApprovalMode::Yolo, false),
        ("bypass-permissions", ApprovalMode::Yolo, false),
        ("yolo", ApprovalMode::Yolo, false),
        ("danger-full-access", ApprovalMode::Yolo, false),
    ];
    for (raw, approval, plan) in cases {
        let mode = parse_permission_mode(raw).expect(raw);
        assert_eq!(mode.approval, approval, "{raw}");
        assert_eq!(mode.plan, plan, "{raw}");
    }

    let upper = parse_permission_mode("  AUTO ").unwrap();
    assert_eq!(upper.approval, ApprovalMode::Auto);
}

/// `acceptEdits` carries the seven write tools in its allow-list.
#[test]
fn acceptedits_allow_list_matches_reference() {
    let mode = parse_permission_mode("acceptEdits").unwrap();
    assert_eq!(
        mode.allow,
        [
            "write_file",
            "edit_file",
            "multi_edit",
            "move_file",
            "notebook_edit",
            "delete_range",
            "delete_symbol",
        ]
    );
}

/// An invalid mode fails `run` with the `error:` prefix, the verbatim matrix
/// text quoting the raw value, and exit 2.
#[test]
fn invalid_permission_mode_wrapped_by_run_exit_two() {
    let out = run(&["run", "--permission-mode", "sneaky", "hi"]);
    assert_eq!(out.code, 2);
    assert_eq!(
        out.stderr,
        "error: unknown permission mode \"sneaky\" (want read-only, manual, ask, auto, acceptEdits, dontAsk, plan, or bypassPermissions; 1.x's workspace-write and danger-full-access also work)\n"
    );
}
