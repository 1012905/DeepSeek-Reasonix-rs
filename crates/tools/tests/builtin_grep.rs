//! Oracles for grep.go: grep_glob_test.go (native engine), grep_ignored_test
//! .go (the two-pass tracked/ignored scope), grep_cjk_peek_test.go, and the
//! engine tests (timeout clamp/format). The ripgrep-subprocess cases
//! (grep_engine_test's rg runs, grep_path_test) are anchored to the
//! subprocess slice — the native scanner covers the same user-visible
//! contract here.

#![forbid(unsafe_code)]

use reasonix_tools::abi::{CallContext, Tool};
use reasonix_tools::builtin::{
    GREP_DEFAULT_TIMEOUT, GREP_MAX_TIMEOUT, GrepScope, GrepTool, format_grep, grep_glob_matches,
    resolve_search,
};
use std::time::Duration;

fn testdir(label: &str) -> (tempfile::TempDir, String) {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
    std::fs::create_dir_all(&base).unwrap();
    let dir = tempfile::Builder::new()
        .prefix(label)
        .tempdir_in(&base)
        .unwrap();
    let path = dir.path().to_string_lossy().into_owned();
    (dir, path)
}

fn grep_tool(work_dir: &str) -> GrepTool {
    GrepTool {
        net_roots: Vec::new(),
        work_dir: String::from(work_dir),
        rg: String::new(),
        forbid_roots: Vec::new(),
        read_roots: Vec::new(),
    }
}

fn run(t: &GrepTool, args: serde_json::Value) -> String {
    Tool::execute(t, &CallContext::new(), &args).unwrap_or_else(|e| panic!("grep: {e}"))
}

fn write_rel(root: &str, rel: &str, body: &str) {
    let sep = reasonix_tools::builtin::pathgo::separator().to_string();
    let path = format!("{root}{}{}", sep, rel.replace('/', &sep));
    if let Some(parent) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, body).unwrap();
}

#[test]
fn test_grep_timeout_clamp() {
    let cases = [
        (0i64, GREP_DEFAULT_TIMEOUT),
        (-5, GREP_DEFAULT_TIMEOUT),
        (5, Duration::from_secs(5)),
        (99999, GREP_MAX_TIMEOUT),
    ];
    for (sec, want) in cases {
        assert_eq!(
            reasonix_tools::builtin::tool_timeout(sec, GREP_DEFAULT_TIMEOUT, GREP_MAX_TIMEOUT),
            want,
            "{sec}"
        );
    }
}

#[test]
fn test_grep_timeout_preserves_partial_results() {
    let hit = vec![String::from("a.go:1:match")];
    let got = format_grep(
        true,
        &hit,
        false,
        Duration::from_secs(30),
        GrepScope::Tracked,
    );
    assert!(got.starts_with("a.go:1:match"), "{got}");
    assert!(
        got.contains("timed out") && got.contains("timeout_seconds"),
        "{got}"
    );
    let got = format_grep(
        true,
        &[],
        false,
        Duration::from_secs(30),
        GrepScope::Tracked,
    );
    assert!(got.contains("timed out"), "{got}");
    let got = format_grep(
        false,
        &[],
        false,
        Duration::from_secs(30),
        GrepScope::Tracked,
    );
    assert_eq!(got, "(no matches)");
}

#[test]
fn test_resolve_search() {
    let (_g, dir) = testdir("grep-rg");
    let rg_file = format!("{dir}{}rg", sep());
    std::fs::write(&rg_file, b"x").unwrap();
    let missing = format!("{dir}{}absent", sep());
    let mut warn = Vec::new();
    assert_eq!(resolve_search("native", &rg_file, &mut warn), None);
    assert_eq!(
        resolve_search("rg", &rg_file, &mut warn),
        Some(rg_file.clone())
    );
    assert_eq!(
        resolve_search("auto", &rg_file, &mut warn),
        Some(rg_file.clone())
    );
    warn.clear();
    assert_eq!(resolve_search("rg", &missing, &mut warn), None);
    assert!(String::from_utf8_lossy(&warn).contains("ripgrep"));
}

#[test]
fn test_a_search_that_finds_nothing_tracked_looks_where_the_rules_point_away() {
    let (_g, root) = ignored_tree("grep-ign1", "SENTINEL_TOKEN");
    let g = grep_tool("");
    let out = run(
        &g,
        serde_json::json!({"pattern": "SENTINEL_TOKEN", "path": root}),
    );
    assert!(
        out.contains("SENTINEL_TOKEN"),
        "the match in an ignored path was never reached:\n{out}"
    );
    assert!(out.contains("no match in the tracked files"), "{out}");
    assert!(
        out.contains("dist") || out.contains("node_modules"),
        "the answer names no ignored path:\n{out}"
    );
}

#[test]
fn test_a_tracked_match_is_answered_without_widening_the_search() {
    let (_g, root) = ignored_tree("grep-ign2", "SENTINEL_TOKEN");
    write_rel(&root, "src/hit.go", "// SENTINEL_TOKEN\n");
    let g = grep_tool("");
    let out = run(
        &g,
        serde_json::json!({"pattern": "SENTINEL_TOKEN", "path": root}),
    );
    assert!(!out.contains("no match in the tracked files"), "{out}");
    assert!(
        !out.contains("dist") && !out.contains("node_modules"),
        "{out}"
    );
}

#[test]
fn test_nothing_anywhere_says_the_ignored_paths_were_searched_too() {
    let (_g, root) = ignored_tree("grep-ign3", "SENTINEL_TOKEN");
    let g = grep_tool("");
    let out = run(
        &g,
        serde_json::json!({"pattern": "NOTHING_HAS_THIS_STRING", "path": root}),
    );
    assert!(out.contains("absent rather than filtered"), "{out}");
}

#[test]
fn test_widening_does_not_reach_into_the_vcs_store() {
    let (_g, root) = ignored_tree("grep-ign4", "SENTINEL_TOKEN");
    write_rel(&root, ".git/COMMIT_EDITMSG", "VCS_ONLY_TOKEN\n");
    let g = grep_tool("");
    let out = run(
        &g,
        serde_json::json!({"pattern": "VCS_ONLY_TOKEN", "path": root}),
    );
    assert!(
        !out.contains("VCS_ONLY_TOKEN"),
        "the widened search read the VCS store:\n{out}"
    );
}

fn ignored_tree(label: &str, needle: &str) -> (tempfile::TempDir, String) {
    let (g, root) = testdir(label);
    // The Go oracle runs under %TEMP% outside any repository and still
    // honors the tree's own .gitignore (mechanism flagged in the report);
    // here the .git marker pins the tree as its own rules root.
    write_rel(&root, ".git/HEAD", "ref: refs/heads/main\n");
    write_rel(&root, ".gitignore", "dist/\nnode_modules/\n");
    write_rel(&root, "src/main.go", "package main\n");
    write_rel(&root, "dist/bundle.js", &format!("var x = \"{needle}\";\n"));
    write_rel(
        &root,
        "node_modules/dep/index.js",
        &format!("// {needle}\n"),
    );
    (g, root)
}

#[test]
fn test_grep_glob_matches_file_name_at_any_depth() {
    let (_g, dir) = grep_glob_tree("grep-glob1");
    let g = grep_tool("");
    let out = run(
        &g,
        serde_json::json!({"pattern": "needle", "path": dir, "glob": "*.go"}),
    );
    assert!(out.contains("top.go") && out.contains("nested.go"), "{out}");
    assert!(!out.contains(".md"), "{out}");
}

#[test]
fn test_grep_glob_with_slash_matches_path() {
    let (_g, dir) = grep_glob_tree("grep-glob2");
    let g = grep_tool("");
    let out = run(
        &g,
        serde_json::json!({"pattern": "needle", "path": dir, "glob": "internal/**/*_test.go"}),
    );
    assert!(out.contains("nested_test.go"), "{out}");
    assert!(
        !out.contains("top.go") && !out.contains("notes.md"),
        "{out}"
    );
}

#[test]
fn test_grep_without_glob_searches_everything() {
    let (_g, dir) = grep_glob_tree("grep-glob3");
    let g = grep_tool("");
    let out = run(&g, serde_json::json!({"pattern": "needle", "path": dir}));
    for want in ["top.go", "top.md", "nested.go", "notes.md"] {
        assert!(out.contains(want), "{out}");
    }
}

#[test]
fn test_grep_rejects_negated_glob() {
    let (_g, dir) = grep_glob_tree("grep-glob4");
    let g = GrepTool {
        net_roots: Vec::new(),
        work_dir: String::new(),
        rg: String::new(),
        forbid_roots: Vec::new(),
        read_roots: Vec::new(),
    };
    let args = serde_json::json!({"pattern": "needle", "path": dir, "glob": "!*.go"});
    let err = Tool::execute(&g, &CallContext::new(), &args)
        .unwrap_err()
        .to_string();
    assert!(err.contains("select files"), "{err}");
}

#[test]
fn test_grep_relative_search_root_keeps_its_output_path() {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (_g, dir) = grep_glob_tree("grep-rel");
    let parent = std::path::Path::new(&dir)
        .parent()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let root = std::path::Path::new(&dir)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    // The Go oracle runs the process-cwd built-ins (t.Chdir); the Rust
    // equivalent is a process chdir held under a lock.
    let before = std::env::current_dir().unwrap();
    std::env::set_current_dir(&parent).unwrap();
    let g = GrepTool {
        net_roots: Vec::new(),
        work_dir: String::new(),
        rg: String::new(),
        forbid_roots: Vec::new(),
        read_roots: Vec::new(),
    };
    let out = run(
        &g,
        serde_json::json!({"pattern": "needle", "path": root, "glob": "internal/**/*_test.go"}),
    );
    std::env::set_current_dir(before).unwrap();
    let sep = reasonix_tools::builtin::pathgo::separator().to_string();
    let want = format!("{root}{sep}internal{sep}deep{sep}nested_test.go:1:needle in a test");
    assert_eq!(out, want, "result must keep the path as requested");
}

fn grep_glob_tree(label: &str) -> (tempfile::TempDir, String) {
    let (g, dir) = testdir(label);
    write_rel(&dir, "top.go", "needle at the top\n");
    write_rel(&dir, "top.md", "needle in markdown\n");
    write_rel(&dir, "internal/deep/nested.go", "needle nested deep\n");
    write_rel(&dir, "internal/deep/nested_test.go", "needle in a test\n");
    write_rel(&dir, "internal/deep/notes.md", "needle in notes\n");
    (g, dir)
}

#[test]
fn test_grep_reads_a_large_cjk_file_as_what_it_is() {
    let (_g, dir) = testdir("grep-cjk");
    let needle = "skin_manifest";
    // Rebuild the Go body exactly: repeated line with the needle.
    let mut body = String::new();
    for _ in 0..2000 {
        body.push_str("美术给的那包图 ");
        body.push_str(needle);
        body.push('\n');
    }
    assert!(
        body.len() > 8 * 1024,
        "fixture must exceed the peek: {} bytes",
        body.len()
    );
    let path = format!("{dir}{}进度.md", sep());
    std::fs::write(&path, body.as_bytes()).unwrap();

    let g = grep_tool(&dir);
    let out = run(&g, serde_json::json!({"pattern": needle, "path": dir}));
    assert!(
        out.contains("美术给的那包图"),
        "grep returned the file in the wrong charset:\n{out}"
    );
}

#[test]
fn test_grep_invalid_pattern_is_an_error() {
    let (_g, dir) = grep_glob_tree("grep-badre");
    let g = grep_tool("");
    let args = serde_json::json!({"pattern": "(unclosed", "path": dir});
    assert!(Tool::execute(&g, &CallContext::new(), &args).is_err());
}

#[test]
fn test_grep_output_shape_is_path_line_text() {
    let (_g, dir) = testdir("grep-shape");
    let path = format!("{dir}{}a.txt", sep());
    std::fs::write(&path, b"alpha\nBETA needle here\ngamma\n").unwrap();
    let g = grep_tool("");
    let out = run(&g, serde_json::json!({"pattern": "needle", "path": dir}));
    assert!(out.contains("a.txt:2:BETA needle here"), "{out}");
}

#[test]
fn test_grep_glob_matcher_rules_directly() {
    assert!(grep_glob_matches("*.go", "/r", "/r/deep/main.go"));
    assert!(!grep_glob_matches("*.go", "/r", "/r/main.md"));
    assert!(grep_glob_matches(
        "internal/**/*_test.go",
        "/r",
        "/r/internal/deep/x_test.go"
    ));
    assert!(!grep_glob_matches(
        "internal/**/*_test.go",
        "/r",
        "/r/top.go"
    ));
}

#[test]
fn test_grep_description_ripgrep_variant_matches_go_byte_for_byte() {
    // grep.go: the rg != "" branch description, verbatim.
    let t = GrepTool {
        net_roots: Vec::new(),
        work_dir: String::new(),
        rg: String::from("rg"),
        forbid_roots: Vec::new(),
        read_roots: Vec::new(),
    };
    assert_eq!(
        t.description(),
        "Search for a regular expression in a file, or recursively under a directory — ripgrep-backed, so it honors .gitignore. Returns matching lines as path:line:text, capped at 200 matches. Ignored paths are searched only when nothing else matched; such matches say so, so an empty answer means absent rather than filtered."
    );
}

#[test]
fn test_grep_description_native_variant_matches_go_byte_for_byte() {
    // grep.go: the rg == "" native fallback description, verbatim.
    let t = GrepTool {
        net_roots: Vec::new(),
        work_dir: String::new(),
        rg: String::new(),
        forbid_roots: Vec::new(),
        read_roots: Vec::new(),
    };
    assert_eq!(
        t.description(),
        "Search for a regular expression in a file, or recursively under a directory (skips hidden files and files matched by .gitignore). Returns matching lines as path:line:text, capped at 200 matches. Ignored paths are searched only when nothing else matched; such matches say so, so an empty answer means absent rather than filtered."
    );
}

fn sep() -> String {
    reasonix_tools::builtin::pathgo::separator().to_string()
}
