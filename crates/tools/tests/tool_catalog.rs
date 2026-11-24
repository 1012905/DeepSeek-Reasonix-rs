//! TOOL-024 contract snapshot: the 33 built-in rows (tools.md §2.1) validate,
//! match the Go registration order, and carry verbatim descriptions/schemas.

use reasonix_tools::builtin::catalog::{by_name, entries, validate};

const EXPECTED_NAMES: [&str; 33] = [
    "bash",
    "bash_output",
    "kill_shell",
    "wait",
    "browser_open",
    "browser_read",
    "browser_act",
    "render_chart",
    "code_index",
    "complete_step",
    "compress",
    "computer_read",
    "computer_act",
    "context_budget",
    "delete_range",
    "delete_symbol",
    "edit_file",
    "glob",
    "grep",
    "ls",
    "move_file",
    "multi_edit",
    "notebook_edit",
    "present_files",
    "read_file",
    "read_video",
    "recall",
    "system_one",
    "todo_write",
    "update_goal",
    "web_fetch",
    "import_web_image",
    "write_file",
];

#[test]
fn frozen_table_passes_contract_validation() {
    validate().expect("the 33-row catalog snapshot parses and is internally consistent");
}

#[test]
fn entries_match_the_go_builtin_names_in_source_order() {
    let names: Vec<&str> = entries().iter().map(|e| e.name).collect();
    assert_eq!(
        names, EXPECTED_NAMES,
        "catalog order follows RegisterBuiltin"
    );
    assert_eq!(entries().len(), 33);
}

#[test]
fn conditional_rows_match_the_registration_source() {
    let conditional: Vec<&str> = entries()
        .iter()
        .filter(|e| e.conditional)
        .map(|e| e.name)
        .collect();
    assert_eq!(conditional, vec!["present_files", "system_one"]);
    for entry in entries() {
        if entry.conditional {
            assert!(entry.notes.contains("Conditional"));
        }
    }
}

#[test]
fn read_only_flags_match_the_go_table() {
    let read_only: Vec<&str> = entries()
        .iter()
        .filter(|e| e.read_only)
        .map(|e| e.name)
        .collect();
    let expected_read_only = vec![
        "bash_output",
        "wait",
        "browser_read",
        "render_chart",
        "code_index",
        "complete_step",
        "compress",
        "context_budget",
        "glob",
        "grep",
        "ls",
        "present_files",
        "read_file",
        "read_video",
        "recall",
        "system_one",
        "todo_write",
        "update_goal",
        "web_fetch",
    ];
    assert_eq!(read_only, expected_read_only);
}

#[test]
fn by_name_finds_a_row_and_misses_unknown_tools() {
    let bash = by_name("bash").expect("bash row exists");
    assert!(!bash.read_only);
    assert!(bash.description.contains("combined stdout/stderr"));
    assert!(by_name("not_a_tool").is_none());
}

#[test]
fn descriptions_match_the_go_source_verbatim_for_simple_rows() {
    let cases: &[(&str, &str)] = &[
        (
            "ls",
            "List the entries of a directory. Directories are shown with a trailing slash; files show their byte size. Set recursive=true to list all nested files depth-first (skips .git/node_modules).",
        ),
        (
            "read_video",
            "Look at a video: native if the model watches video, else ffmpeg frames.",
        ),
        (
            "write_file",
            "Write full file content, overwriting and creating parent directories. Prefer edit_file/multi_edit for partial changes.",
        ),
        (
            "web_fetch",
            "Read HTTP(S) text. Reduces HTML to text and image candidates for import_web_image; returns JSON/plain text verbatim.",
        ),
        (
            "import_web_image",
            "Import web_fetch image_url from source url into new path. Validates PNG/JPEG/GIF/WebP. Returns image {path,alt,source} for render_surface. Match extension.",
        ),
        (
            "present_files",
            "Present existing final files as clickable cards, including shell-created documents. Paths must be in the workspace or authorized folders. Opens current files, not snapshots; does not certify correctness.",
        ),
    ];
    for (name, want) in cases {
        let entry = by_name(name).expect("row exists");
        assert_eq!(entry.description, *want, "{name}.Description()");
    }
}

#[test]
fn long_descriptions_carry_the_go_phrases_verbatim() {
    let checks: &[(&str, &[&str])] = &[
        (
            "read_file",
            &["1-based number", "target exact lines", "pagination hints"],
        ),
        (
            "update_goal",
            &[
                "Delivery acceptance criteria",
                "missing user-only information",
                "changed scope",
            ],
        ),
        (
            "compress",
            &[
                "explicitly asks for context compression",
                "exact, unique excerpt",
                "multiple matches",
            ],
        ),
        (
            "bash",
            &[
                "read_file returns up to 2000 lines",
                "git log | grep",
                "package managers",
            ],
        ),
        (
            "grep",
            &[
                "path:line:text",
                "200 matches",
                "Ignored paths are searched",
            ],
        ),
        (
            "recall",
            &[
                "#n positions",
                "current copy beats a folded one",
                "refused whole, not truncated",
            ],
        ),
        (
            "computer_act",
            &[
                "CUA Driver",
                "refs from its latest snapshot",
                "requires separate approval",
            ],
        ),
    ];
    for (name, phrases) in checks {
        let entry = by_name(name).expect("row exists");
        for phrase in *phrases {
            assert!(
                entry.description.contains(phrase),
                "{name} missing phrase {phrase:?}"
            );
        }
    }
}

#[test]
fn schemas_parse_as_json_objects_with_expected_required_keys() {
    let required: &[(&str, &[&str])] = &[
        ("bash", &["command"]),
        ("kill_shell", &["job_id"]),
        ("browser_act", &["steps"]),
        ("write_file", &["path", "content"]),
        ("present_files", &["files"]),
        ("move_file", &["source_path", "destination_path"]),
        ("update_goal", &["status"]),
    ];
    for (name, want_required) in required {
        let entry = by_name(name).expect("row exists");
        let schema: serde_json::Value = serde_json::from_str(entry.schema).expect("schema parses");
        let required_keys = schema
            .get("required")
            .and_then(|r| r.as_array())
            .expect("required array");
        let got: Vec<&str> = required_keys.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(&got, want_required, "{name}.Schema().required");
    }
}

#[test]
fn system_one_zero_spec_snapshot_carries_an_empty_backend_enum() {
    let entry = by_name("system_one").expect("row exists");
    let schema: serde_json::Value = serde_json::from_str(entry.schema).expect("schema parses");
    let backend_enum = schema
        .pointer("/properties/backend/enum")
        .expect("backend enum");
    assert!(backend_enum.as_array().expect("enum array").is_empty());
    assert!(entry.notes.contains("dynamic"));
}
