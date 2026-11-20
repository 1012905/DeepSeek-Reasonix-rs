//! Oracle for editfile.go, multiedit.go, edit_access_test.go, crlf_edit_test.go,
//! edit_literal_endings_test.go, fuzzy_edit_test.go and the write-tool cases
//! of e2e_encoding_test.go: exact-match and fuzzy edits, CRLF/GBK handling,
//! refusal texts, multi_edit atomicity, and the post-write receipt.

#![forbid(unsafe_code)]

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use reasonix_core::fileencoding::Kind;
use reasonix_tools::abi::{CallContext, Tool};
use reasonix_tools::builtin::{
    CODE_READ_FORBIDDEN, EditFileTool, EditReadPolicy, EditStep, FileOverlay, MultiEditTool, encode,
};
use serde_json::json;

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

fn edit_tool() -> EditFileTool {
    EditFileTool::default()
}

fn multi_tool() -> MultiEditTool {
    MultiEditTool::default()
}

fn run_edit(args: serde_json::Value) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    edit_tool().execute(&CallContext::new(), &args)
}

fn run_multi(args: serde_json::Value) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    multi_tool().execute(&CallContext::new(), &args)
}

fn read(path: &str) -> String {
    String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned()
}

/// Go `errors.As(err, &tool.Refusal)`: digs the typed refusal out through
/// the read/write wraps.
fn as_refusal(err: &(dyn std::error::Error + 'static)) -> Option<reasonix_tools::refusal::Refusal> {
    let mut cur = Some(err);
    while let Some(e) = cur {
        if let Some(r) = e.downcast_ref::<reasonix_tools::refusal::Refusal>() {
            return Some(r.clone());
        }
        cur = e.source();
    }
    None
}

/// editsource.go FileOverlay probe: counts every read/write, serves no
/// buffer.
struct ProbeOverlay {
    reads: AtomicUsize,
    writes: AtomicUsize,
}

impl FileOverlay for ProbeOverlay {
    fn read_text_file(&self, _path: &str) -> Option<String> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Some(String::from("overlay-canary"))
    }
    fn write_text_file(&self, _path: &str, _content: &str) -> std::io::Result<bool> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        Ok(false)
    }
}

#[test]
fn test_edit_file_not_found_names_recovery_hint_and_leaves_file() {
    let (_g, dir) = testdir("edit-not-found");
    let f = Path::new(&dir).join("a.txt");
    std::fs::write(&f, b"hello world").unwrap();
    let err =
        run_edit(json!({"path": f, "old_string": "nonexistent", "new_string": "x"})).unwrap_err();
    assert!(
        err.to_string().contains("Re-read the current file"),
        "not-found error should include recovery hint, got: {err}"
    );
    assert_eq!(
        read(&f.to_string_lossy()),
        "hello world",
        "file modified despite error"
    );
}

#[test]
fn test_edit_file_not_unique_reports_matching_lines() {
    let (_g, dir) = testdir("edit-not-unique");
    let f = Path::new(&dir).join("map.html");
    let separator = "    // ═══════════════════════════════════════";
    let body = format!("{separator}\nconst a = 1;\n{separator}\nconst b = 2;\n{separator}\n");
    std::fs::write(&f, body.as_bytes()).unwrap();
    let err = run_edit(json!({"path": f, "old_string": separator, "new_string": "// section"}))
        .unwrap_err();
    for want in [
        "not unique",
        "matching lines include 1, 3, 5",
        "repeated separator lines",
    ] {
        assert!(
            err.to_string().contains(want),
            "error should mention {want:?}: {err}"
        );
    }
}

#[test]
fn test_edit_file_delete_receipt_bounds_to_the_span() {
    let (_g, dir) = testdir("edit-delete");
    let f = Path::new(&dir).join("a.txt");
    std::fs::write(&f, b"remove this line\nkeep this\n").unwrap();
    let out =
        run_edit(json!({"path": f, "old_string": "remove this line\n", "new_string": ""})).unwrap();
    for want in [
        "Actual replacement receipt after write:",
        "-remove this line",
        "+<empty>",
    ] {
        assert!(
            out.contains(want),
            "delete result should contain {want:?}:\n{out}"
        );
    }
    assert!(
        !out.contains(" keep this"),
        "actual receipt should not auto-upload unchanged neighboring lines:\n{out}"
    );
    assert_eq!(read(&f.to_string_lossy()), "keep this\n");
}

#[test]
fn test_edit_file_actual_diff_is_bounded() {
    let (_g, dir) = testdir("edit-bounded");
    let f = Path::new(&dir).join("large.txt");
    let old = "old line with enough content to grow the diff\n".repeat(300);
    let new = "new line with enough content to grow the diff\n".repeat(300);
    std::fs::write(&f, old.as_bytes()).unwrap();
    let out = run_edit(json!({"path": f, "old_string": old, "new_string": new})).unwrap();
    assert!(
        out.len() <= reasonix_tools::builtin::MAX_POST_WRITE_RECEIPT_BYTES + 1024,
        "bounded edit result is too large: {} bytes",
        out.len()
    );
    assert!(
        out.contains(reasonix_tools::builtin::POST_WRITE_SPAN_TRUNCATED),
        "bounded edit result should disclose truncation:\n{out}"
    );
}

#[test]
fn test_edit_file_receipt_does_not_expose_unchanged_same_line_content() {
    let (_g, dir) = testdir("edit-private");
    let f = Path::new(&dir).join("private.txt");
    let private_marker = "PRIVATE_SAME_LINE_CONTEXT_6504";
    let seed = format!("customer_note={private_marker} enabled=false\n");
    std::fs::write(&f, seed.as_bytes()).unwrap();
    let out = run_edit(json!({"path": f, "old_string": "false", "new_string": "true"})).unwrap();
    assert!(
        !out.contains(private_marker)
            && !out.contains("customer_note")
            && !out.contains("enabled="),
        "replacement receipt exposed unchanged same-line content:\n{out}"
    );
    for want in ["-false", "+true"] {
        assert!(
            out.contains(want),
            "replacement receipt should contain {want:?}:\n{out}"
        );
    }
    assert_eq!(
        read(&f.to_string_lossy()),
        format!("customer_note={private_marker} enabled=true\n")
    );
}

#[test]
fn test_edit_file_missing_old_string() {
    let (_g, dir) = testdir("edit-missing-old");
    let f = Path::new(&dir).join("a.txt");
    std::fs::write(&f, b"content").unwrap();
    let err = run_edit(json!({"path": f, "new_string": "x"})).unwrap_err();
    assert!(err.to_string().contains("old_string is required"), "{err}");
}

#[test]
fn test_edit_file_missing_path() {
    let err = run_edit(json!({"old_string": "x", "new_string": "y"})).unwrap_err();
    assert!(err.to_string().contains("path is required"), "{err}");
}

#[test]
fn test_edit_file_not_found_missing_file() {
    let (_g, dir) = testdir("edit-missing-file");
    let f = Path::new(&dir).join("missing.txt");
    let err = run_edit(json!({"path": f, "old_string": "x", "new_string": "y"})).unwrap_err();
    assert!(
        err.to_string().contains("read "),
        "expected the read wrap, got: {err}"
    );
}

#[test]
fn test_edit_tools_check_read_access_without_agent() {
    // edit_access_test.go for edit_file and multi_edit: the read policy
    // refuses before anything touches the file.
    let args_of = |name: &str, p: String| -> serde_json::Value {
        if name == "edit_file" {
            json!({"path": p, "old_string": "missing", "new_string": "changed"})
        } else {
            json!({"path": p, "edits": [{"old_string": "missing", "new_string": "changed"}]})
        }
    };
    for name in ["edit_file", "multi_edit"] {
        let (_g, dir) = testdir("edit-access");
        let p = Path::new(&dir).join("private.go");
        let original = "package main\n// PRIVATE_CANARY\nfunc RemoveMe() {}\n";
        std::fs::write(&p, original.as_bytes()).unwrap();
        let probe = Arc::new(ProbeOverlay {
            reads: AtomicUsize::new(0),
            writes: AtomicUsize::new(0),
        });
        let args = args_of(name, p.to_string_lossy().into_owned());
        let err = match name {
            "edit_file" => {
                let t = EditFileTool {
                    read_policy: EditReadPolicy {
                        forbid_roots: vec![p.to_string_lossy().into_owned()],
                        ..EditReadPolicy::default()
                    },
                    overlay: Some(probe.clone()),
                    ..EditFileTool::default()
                };
                t.execute(&CallContext::new(), &args).unwrap_err()
            }
            _ => {
                let t = MultiEditTool {
                    read_policy: EditReadPolicy {
                        forbid_roots: vec![p.to_string_lossy().into_owned()],
                        ..EditReadPolicy::default()
                    },
                    overlay: Some(probe.clone()),
                    ..MultiEditTool::default()
                };
                t.execute(&CallContext::new(), &args).unwrap_err()
            }
        };
        let refusal = as_refusal(err.as_ref())
            .unwrap_or_else(|| panic!("{name}: expected typed refusal, got {err}"));
        assert_eq!(
            refusal.code, CODE_READ_FORBIDDEN,
            "direct {name} bypassed read policy: {err}"
        );
        assert_eq!(
            probe.reads.load(Ordering::SeqCst),
            0,
            "denied operation reached the buffer"
        );
        assert_eq!(
            std::fs::read(&p).unwrap(),
            original.as_bytes(),
            "protected file changed"
        );
    }
}

// ---- multi_edit ----

#[test]
fn test_multi_edit_empty_edits() {
    let (_g, dir) = testdir("multi-empty");
    let f = Path::new(&dir).join("a.txt");
    std::fs::write(&f, b"content").unwrap();
    let err = run_multi(json!({"path": f, "edits": []})).unwrap_err();
    assert!(err.to_string().contains("edits must not be empty"), "{err}");
}

#[test]
fn test_multi_edit_missing_path() {
    let err = run_multi(json!({"edits": [{"old_string": "x", "new_string": "y"}]})).unwrap_err();
    assert!(err.to_string().contains("path is required"), "{err}");
}

#[test]
fn test_multi_edit_step_not_found_leaves_file_untouched() {
    let (_g, dir) = testdir("multi-atomic");
    let f = Path::new(&dir).join("a.txt");
    std::fs::write(&f, b"alpha\nbeta\n").unwrap();
    let err = run_multi(json!({
        "path": f,
        "edits": [
            {"old_string": "alpha", "new_string": "ALPHA"},
            {"old_string": "nonexistent", "new_string": "x"},
        ],
    }))
    .unwrap_err();
    for want in ["edit 2", "Re-read the current file"] {
        assert!(
            err.to_string().contains(want),
            "multi_edit error should mention {want:?}, got: {err}"
        );
    }
    assert_eq!(
        read(&f.to_string_lossy()),
        "alpha\nbeta\n",
        "file modified despite error"
    );
}

#[test]
fn test_multi_edit_reports_every_failed_step() {
    let (_g, dir) = testdir("multi-report");
    let f = Path::new(&dir).join("a.txt");
    let original = "alpha\nbeta\nreturn nil\nreturn nil\ngamma\n";
    std::fs::write(&f, original.as_bytes()).unwrap();
    let err = run_multi(json!({
        "path": f,
        "edits": [
            {"old_string": "alpha", "new_string": "ALPHA"},
            {"old_string": "nonexistent", "new_string": "x"},
            {"old_string": "gamma", "new_string": "GAMMA"},
            {"old_string": "return nil", "new_string": "return err"},
            {"new_string": "x"},
        ],
    }))
    .unwrap_err();
    let msg = err.to_string();
    for want in [
        "3 of 5 edits failed",
        "edit 2: old_string not found",
        "edit 4: old_string is not unique",
        "edit 5: old_string is required",
    ] {
        assert!(
            msg.contains(want),
            "multi_edit error should mention {want:?}, got:\n{msg}"
        );
    }
    for unwanted in ["edit 1:", "edit 3:"] {
        assert!(
            !msg.contains(unwanted),
            "multi_edit error should not report succeeding step {unwanted:?}, got:\n{msg}"
        );
    }
    assert_eq!(
        read(&f.to_string_lossy()),
        original,
        "file modified despite error"
    );
}

#[test]
fn test_multi_edit_replace_all() {
    let (_g, dir) = testdir("multi-replace-all");
    let f = Path::new(&dir).join("a.txt");
    std::fs::write(&f, b"foo bar foo baz foo").unwrap();
    run_multi(json!({
        "path": f,
        "edits": [{"old_string": "foo", "new_string": "qux", "replace_all": true}],
    }))
    .unwrap();
    assert_eq!(read(&f.to_string_lossy()), "qux bar qux baz qux");
}

#[test]
fn test_multi_edit_replace_all_not_found() {
    let (_g, dir) = testdir("multi-replace-miss");
    let f = Path::new(&dir).join("a.txt");
    std::fs::write(&f, b"hello").unwrap();
    let err = run_multi(json!({
        "path": f,
        "edits": [{"old_string": "nonexistent", "new_string": "x", "replace_all": true}],
    }))
    .unwrap_err();
    assert!(err.to_string().contains("old_string not found"), "{err}");
}

#[test]
fn test_multi_edit_missing_old_string_in_step() {
    let (_g, dir) = testdir("multi-missing-old");
    let f = Path::new(&dir).join("a.txt");
    std::fs::write(&f, b"content").unwrap();
    let err = run_multi(json!({"path": f, "edits": [{"new_string": "x"}]})).unwrap_err();
    assert!(err.to_string().contains("old_string is required"), "{err}");
}

#[test]
fn test_multi_edit_chained() {
    let (_g, dir) = testdir("multi-chained");
    let f = Path::new(&dir).join("code.go");
    std::fs::write(&f, b"package old\n\nfunc Old() {\n\tOld()\n}\n").unwrap();
    run_multi(json!({
        "path": f,
        "edits": [
            {"old_string": "package old", "new_string": "package new"},
            {"old_string": "Old", "new_string": "New", "replace_all": true},
        ],
    }))
    .unwrap();
    assert_eq!(
        read(&f.to_string_lossy()),
        "package new\n\nfunc New() {\n\tNew()\n}\n"
    );
}

#[test]
fn test_multi_edit_summary_counts_replacements() {
    let (_g, dir) = testdir("multi-summary");
    let f = Path::new(&dir).join("list.txt");
    std::fs::write(&f, b"item   \nitem\t\n").unwrap();
    let out = run_multi(json!({
        "path": f,
        "edits": [{"old_string": "item\n", "new_string": "thing\n", "replace_all": true}],
    }))
    .unwrap();
    assert!(
        out.contains("2 total replacements") && out.contains("fuzzy match"),
        "unexpected output: {out:?}"
    );
    assert_eq!(read(&f.to_string_lossy()), "thing\nthing\n");
}

// ---- CRLF / encoding ----

#[test]
fn test_edit_file_lf_stays_lf() {
    let (_g, dir) = testdir("edit-lf");
    let f = Path::new(&dir).join("f.txt");
    std::fs::write(&f, b"alpha\nbeta\ngamma\n").unwrap();
    run_edit(json!({"path": f, "old_string": "alpha\nbeta", "new_string": "ALPHA\nBETA"})).unwrap();
    assert_eq!(read(&f.to_string_lossy()), "ALPHA\nBETA\ngamma\n");
}

#[test]
fn test_edit_file_literal_match_in_crlf_file_leaves_no_bare_lf() {
    let (_g, dir) = testdir("edit-crlf-ini");
    let f = Path::new(&dir).join("settings.ini");
    std::fs::write(&f, b"[client]\r\nretries=3\r\nbackoff=2\r\n").unwrap();
    run_edit(json!({"path": f, "old_string": "retries=3", "new_string": "retries=3\ntimeout=30"}))
        .unwrap();
    assert_eq!(
        read(&f.to_string_lossy()),
        "[client]\r\nretries=3\r\ntimeout=30\r\nbackoff=2\r\n"
    );
}

#[test]
fn test_multi_edit_crlf() {
    let (_g, dir) = testdir("multi-crlf");
    let f = Path::new(&dir).join("f.txt");
    std::fs::write(&f, b"one\r\ntwo\r\nthree\r\nfour\r\n").unwrap();
    run_multi(json!({
        "path": f,
        "edits": [
            {"old_string": "one\ntwo", "new_string": "ONE\nTWO"},
            {"old_string": "three", "new_string": "THREE"},
        ],
    }))
    .unwrap();
    assert_eq!(
        read(&f.to_string_lossy()),
        "ONE\r\nTWO\r\nTHREE\r\nfour\r\n"
    );
}

#[test]
fn test_edit_file_crlf_not_found_hint_avoids_misattribution() {
    let (_g, dir) = testdir("edit-crlf-hint");
    let f = Path::new(&dir).join("win.txt");
    std::fs::write(&f, b"one\r\ntwo\r\nthree\r\n").unwrap();
    let err =
        run_edit(json!({"path": f, "old_string": "one\nmissing", "new_string": "ONE\nMISSING"}))
            .unwrap_err();
    let msg = err.to_string();
    for want in [
        "old_string not found",
        "CRLF line endings",
        "already tolerate LF-only old_string",
        "stale, incomplete, or non-unique context",
    ] {
        assert!(
            msg.contains(want),
            "error {msg:?} does not contain {want:?}"
        );
    }
}

#[test]
fn test_edit_file_fuzzy_trailing_whitespace() {
    let (_g, dir) = testdir("edit-fuzzy-ws");
    let f = Path::new(&dir).join("main.go");
    std::fs::write(&f, b"func main() {   \n\tfmt.Println(\"hello\")  \n}\n").unwrap();
    let out = run_edit(json!({
        "path": f,
        "old_string": "func main() {\n\tfmt.Println(\"hello\")\n}",
        "new_string": "func main() {\n\tfmt.Println(\"bye\")\n}",
    }))
    .unwrap();
    assert!(
        out.contains("fuzzy match"),
        "output should disclose fuzzy matching, got {out:?}"
    );
    assert_eq!(
        read(&f.to_string_lossy()),
        "func main() {\n\tfmt.Println(\"bye\")\n}\n"
    );
}

#[test]
fn test_edit_file_fuzzy_read_file_line_prefixes() {
    let (_g, dir) = testdir("edit-fuzzy-prefix");
    let f = Path::new(&dir).join("notes.txt");
    std::fs::write(&f, b"alpha\nbeta\ngamma\n").unwrap();
    run_edit(json!({
        "path": f,
        "old_string": "1\u{2192}alpha\n2\u{2192}beta",
        "new_string": "ALPHA\nBETA",
    }))
    .unwrap();
    assert_eq!(read(&f.to_string_lossy()), "ALPHA\nBETA\ngamma\n");
}

#[test]
fn test_edit_file_fuzzy_crlf_preserves_line_endings() {
    let (_g, dir) = testdir("edit-fuzzy-crlf");
    let f = Path::new(&dir).join("win.txt");
    std::fs::write(&f, b"one   \r\ntwo   \r\nthree\r\n").unwrap();
    run_edit(json!({"path": f, "old_string": "one\ntwo", "new_string": "ONE\nTWO"})).unwrap();
    assert_eq!(read(&f.to_string_lossy()), "ONE\r\nTWO\r\nthree\r\n");
}

#[test]
fn test_edit_file_fuzzy_ambiguous_does_not_write() {
    let (_g, dir) = testdir("edit-fuzzy-ambiguous");
    let f = Path::new(&dir).join("dup.txt");
    let seed = "target   \ntarget   \n";
    std::fs::write(&f, seed.as_bytes()).unwrap();
    let err = run_edit(json!({"path": f, "old_string": "target\n", "new_string": "updated\n"}))
        .unwrap_err();
    assert!(
        err.to_string().contains("not unique"),
        "expected not-unique error, got {err}"
    );
    assert_eq!(
        read(&f.to_string_lossy()),
        seed,
        "ambiguous fuzzy edit changed file"
    );
}

#[test]
fn test_edit_file_fuzzy_leading_indent_drift_does_not_write() {
    let (_g, dir) = testdir("edit-fuzzy-indent");
    let f = Path::new(&dir).join("indent.go");
    let seed = "func f() {\n    if ok {\n        return nil\n    }\n}\n";
    std::fs::write(&f, seed.as_bytes()).unwrap();
    let err = run_edit(json!({
        "path": f,
        "old_string": "if ok {\n    return nil\n}",
        "new_string": "if ok {\n    return errors.New(\"nope\")\n}",
    }))
    .unwrap_err();
    assert!(
        err.to_string().contains("old_string not found"),
        "expected not-found error for leading indentation drift, got {err}"
    );
    assert_eq!(
        read(&f.to_string_lossy()),
        seed,
        "leading indentation drift changed file"
    );
}

#[test]
fn test_edit_file_fuzzy_blank_line_run() {
    let (_g, dir) = testdir("edit-blank-run");
    let f = Path::new(&dir).join("metrics.py");
    let seed =
        "_count = 0\n\n\ndef bump(by=1):\n    global _count\n    _count += by\n    return _count\n";
    std::fs::write(&f, seed.as_bytes()).unwrap();
    let out = run_edit(json!({
        "path": f,
        "old_string": "_count = 0\n\ndef bump(by=1):\n    global _count\n    _count += by\n    return _count",
        "new_string": "_count = 0\n\ndef bump(by=1):\n    global _count\n    _count += by * 2\n    return _count",
    }))
    .unwrap();
    assert!(
        out.contains("fuzzy match"),
        "output should disclose fuzzy matching, got {out:?}"
    );
    let want = "_count = 0\n\ndef bump(by=1):\n    global _count\n    _count += by * 2\n    return _count\n";
    assert_eq!(read(&f.to_string_lossy()), want);
}

#[test]
fn test_edit_file_blank_line_run_keeps_uniqueness_rule() {
    let (_g, dir) = testdir("edit-blank-unique");
    let f = Path::new(&dir).join("dup.py");
    let seed = "def a():\n    pass\n\n\ndef b():\n    pass\n\n\ndef a():\n    pass\n\n\ndef b():\n    pass\n";
    std::fs::write(&f, seed.as_bytes()).unwrap();
    let err = run_edit(json!({
        "path": f,
        "old_string": "def a():\n    pass\n\ndef b():\n    pass",
        "new_string": "def c():\n    pass",
    }))
    .unwrap_err();
    assert!(
        err.to_string().contains("not unique"),
        "expected an ambiguous old_string to be rejected, not silently applied: {err}"
    );
    assert_eq!(
        read(&f.to_string_lossy()),
        seed,
        "file must be untouched on an ambiguous match"
    );
}

#[test]
fn test_edit_file_blank_line_run_does_not_invent_separation() {
    let (_g, dir) = testdir("edit-blank-invent");
    let f = Path::new(&dir).join("tight.py");
    let seed = "x = 1\ny = 2\n";
    std::fs::write(&f, seed.as_bytes()).unwrap();
    let err = run_edit(
        json!({"path": f, "old_string": "x = 1\n\ny = 2", "new_string": "x = 9\n\ny = 9"}),
    )
    .unwrap_err();
    assert!(
        !err.to_string().is_empty(),
        "a blank line absent from the file must not match"
    );
    assert_eq!(read(&f.to_string_lossy()), seed, "file must be untouched");
}

#[test]
fn test_multi_edit_blank_line_run_replace_all() {
    let (_g, dir) = testdir("multi-blank-all");
    let f = Path::new(&dir).join("dup.py");
    std::fs::write(
        &f,
        b"def f():\n    p\n\n\ndef g():\n    p\n\n\ndef f():\n    p\n\n\ndef g():\n    p\n",
    )
    .unwrap();
    run_multi(json!({
        "path": f,
        "edits": [{
            "old_string": "def f():\n    p\n\ndef g():\n    p",
            "new_string": "def fg():\n    p",
            "replace_all": true,
        }],
    }))
    .unwrap();
    assert_eq!(
        read(&f.to_string_lossy()),
        "def fg():\n    p\n\n\ndef fg():\n    p\n"
    );
}

#[test]
fn test_edit_file_blank_line_run_preserves_crlf() {
    let (_g, dir) = testdir("edit-blank-crlf");
    let f = Path::new(&dir).join("crlf.py");
    std::fs::write(&f, b"a = 1\r\n\r\n\r\nb = 2\r\n").unwrap();
    run_edit(json!({"path": f, "old_string": "a = 1\n\nb = 2", "new_string": "a = 9\n\nb = 9"}))
        .unwrap();
    assert_eq!(read(&f.to_string_lossy()), "a = 9\r\n\r\nb = 9\r\n");
}

#[test]
fn test_edit_file_blank_line_run_keeps_final_newline() {
    let (_g, dir) = testdir("edit-blank-final");
    let f = Path::new(&dir).join("tail.py");
    std::fs::write(&f, b"a = 1\n\n\nb = 2\n").unwrap();
    run_edit(json!({"path": f, "old_string": "a = 1\n\nb = 2", "new_string": "a = 9\n\nb = 9"}))
        .unwrap();
    assert_eq!(read(&f.to_string_lossy()), "a = 9\n\nb = 9\n");
}

#[test]
fn test_multi_edit_blank_line_run_replace_all_rejects_blank_only() {
    let (_g, dir) = testdir("multi-blank-only");
    let f = Path::new(&dir).join("runs.py");
    let seed = "a = 1\n\n\nb = 2\n\n\nc = 3\n";
    std::fs::write(&f, seed.as_bytes()).unwrap();
    let err = run_multi(json!({
        "path": f,
        "edits": [{"old_string": "\n\n\n\n", "new_string": "", "replace_all": true}],
    }))
    .unwrap_err();
    assert!(
        !err.to_string().is_empty(),
        "a blank-only old_string must not match every blank run"
    );
    assert_eq!(read(&f.to_string_lossy()), seed, "file must be untouched");
}

#[test]
fn test_edit_file_not_found_names_blank_line_presence_drift() {
    for (label, seed, old, span) in [
        (
            "old_string drops a blank line",
            "tex.uOffset = u;\r\ntex.vOffset = v;\r\n}\r\n\r\n/**\r\n * doc\r\n */\r\n",
            "tex.vOffset = v;\n}\n/**",
            "lines 2-5",
        ),
        (
            "old_string invents a blank line",
            "x = 1\ny = 2\n",
            "x = 1\n\ny = 2",
            "lines 1-2",
        ),
    ] {
        let (_g, dir) = testdir("edit-blank-cause");
        let p = Path::new(&dir).join("src.ts");
        std::fs::write(&p, seed.as_bytes()).unwrap();
        let err =
            run_edit(json!({"path": p, "old_string": old, "new_string": "replaced"})).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains(span) && msg.contains("blank lines"),
            "{label}: error must name the span and the blank-line cause, got {msg:?}"
        );
        let err =
            run_multi(json!({"path": p, "edits": [{"old_string": old, "new_string": "replaced"}]}))
                .unwrap_err();
        assert!(
            err.to_string().contains(span),
            "{label}: multi_edit must report the same cause, got {err}"
        );
        assert_eq!(
            read(&p.to_string_lossy()),
            seed,
            "{label}: file must be untouched"
        );
    }
}

#[test]
fn test_edit_file_not_found_blank_line_cause_needs_one_span() {
    let (_g, dir) = testdir("edit-blank-two-spans");
    let f = Path::new(&dir).join("dup.py");
    std::fs::write(&f, b"a\nb\nz\na\nb\n").unwrap();
    let err = run_edit(json!({"path": f, "old_string": "a\n\nb", "new_string": "c"})).unwrap_err();
    assert!(
        !err.to_string().contains("blank lines"),
        "two spans match once blank lines are ignored, so no cause may be claimed, got {err}"
    );
}

#[test]
fn test_edit_file_not_found_blank_line_cause_needs_different_layout() {
    let (_g, dir) = testdir("edit-blank-same-layout");
    let f = Path::new(&dir).join("eof.py");
    std::fs::write(&f, b"a\n\nb").unwrap();
    let err =
        run_edit(json!({"path": f, "old_string": "a\n\nb\n", "new_string": "c\n"})).unwrap_err();
    assert!(
        !err.to_string().contains("blank lines"),
        "the blank lines agree and the miss is the final newline, got {err}"
    );
}

// ---- GBK / charset round trips ----

fn gbk_bytes(text: &str) -> Vec<u8> {
    encode(text, Kind::Gb18030).unwrap()
}

fn decode_gb(raw: &[u8]) -> String {
    let decoded = reasonix_core::fileencoding::decode_to_utf8(raw);
    String::from_utf8_lossy(&decoded).into_owned()
}

#[test]
fn test_e2e_gbk_round_trip_composes_with_read_and_edit() {
    let (_g, dir) = testdir("e2e-gbk");
    let path = Path::new(&dir).join("test_gbk.txt");
    std::fs::write(&path, gbk_bytes("你好世界\n这是第二行\n包含函数的测试\n")).unwrap();

    let read_tool = reasonix_tools::builtin::ReadFileTool {
        net_roots: Vec::new(),
        work_dir: String::new(),
        forbid_roots: Vec::new(),
        read_roots: Vec::new(),
    };
    let out = read_tool
        .execute(&CallContext::new(), &json!({"path": path}))
        .unwrap();
    assert!(
        out.contains("你好世界") && out.contains("这是第二行"),
        "read_file did not decode GBK to readable Chinese:\n{out}"
    );
    let raw = std::fs::read(&path).unwrap();
    assert!(
        std::str::from_utf8(&raw).is_err()
            || raw == gbk_bytes("你好世界\n这是第二行\n包含函数的测试\n"),
        "read_file must not rewrite the GBK file"
    );

    run_edit(json!({"path": path, "old_string": "这是第二行", "new_string": "这是新的行"}))
        .unwrap();
    let raw2 = std::fs::read(&path).unwrap();
    assert!(
        std::str::from_utf8(&raw2).is_err() || decode_gb(&raw2).contains("这是新的行"),
        "edit_file rewrote the GBK file as UTF-8 on disk"
    );
    let s = decode_gb(&raw2);
    assert!(s.contains("这是新的行") && !s.contains("这是第二行"), "{s}");

    let out2 = read_tool
        .execute(&CallContext::new(), &json!({"path": path}))
        .unwrap();
    assert!(
        out2.contains("这是新的行"),
        "read_file after edit missing new text:\n{out2}"
    );
}

#[test]
fn test_e2e_edit_keeps_bytes_of_utf8_file_with_stray_byte() {
    let (_g, dir) = testdir("e2e-stray");
    let path = Path::new(&dir).join("mixed.go");
    let before: Vec<u8> = String::from("// 中文注释：修改前的内容\nvalue := 1\n// 结尾")
        .into_bytes()
        .into_iter()
        .chain([0xff])
        .chain(String::from("\n").into_bytes())
        .collect();
    std::fs::write(&path, &before).unwrap();
    run_edit(json!({"path": path, "old_string": "value := 1", "new_string": "value := 2"}))
        .unwrap();
    let got = std::fs::read(&path).unwrap();
    let mut want = before.clone();
    let pos = want.windows(10).position(|w| w == b"value := 1").unwrap();
    want.splice(pos..pos + 10, b"value := 2".to_vec());
    assert_eq!(got, want, "edit_file changed bytes outside the edit");
}

#[test]
fn test_edit_file_crlf_gbk_preserves_encoding_and_endings() {
    let (_g, dir) = testdir("edit-gbk-crlf");
    let f = Path::new(&dir).join("f.cs");
    let src = "第一行 hello\r\n第二行 world\r\n第三行 done\r\n";
    std::fs::write(&f, gbk_bytes(src)).unwrap();
    run_edit(json!({
        "path": f,
        "old_string": "第一行 hello\n第二行 world",
        "new_string": "第一行 HELLO\n第二行 WORLD",
    }))
    .unwrap();
    let got = decode_gb(&std::fs::read(&f).unwrap());
    assert_eq!(got, "第一行 HELLO\r\n第二行 WORLD\r\n第三行 done\r\n");
    let stripped = got.replace("\r\n", "");
    assert!(
        !stripped.contains('\n'),
        "a bare \\n leaked into a CRLF file: {got:?}"
    );
}

#[test]
fn test_file_tools_keep_encoding_and_bom_all_ops() {
    // encoding_preserve_test.go: gbk / utf8 / utf8-bom x edit_file /
    // multi_edit / write_file.
    type ToolError = Box<dyn std::error::Error + Send + Sync>;
    type Op = Box<dyn Fn(String, &str) -> Result<String, ToolError>>;
    let before = "标题：旧的说明\r\n第二行\r\n";
    let after = "标题：新的说明\r\n第二行\r\n";
    type Encoder = Box<dyn Fn(&str) -> Vec<u8>>;
    let encodings: Vec<(&str, Encoder)> = vec![
        ("gbk", Box::new(|s: &str| gbk_bytes(s))),
        ("utf8", Box::new(|s: &str| s.as_bytes().to_vec())),
        (
            "utf8-bom",
            Box::new(|s: &str| {
                let mut v = vec![0xEF, 0xBB, 0xBF];
                v.extend_from_slice(s.as_bytes());
                v
            }),
        ),
    ];
    let ops: Vec<(&str, Op)> = vec![
        (
            "edit_file",
            Box::new(|p: String, _after: &str| {
                run_edit(json!({"path": p, "old_string": "旧的", "new_string": "新的"}))
            }),
        ),
        (
            "multi_edit",
            Box::new(|p: String, _after: &str| {
                run_multi(
                    json!({"path": p, "edits": [{"old_string": "旧的", "new_string": "新的"}]}),
                )
            }),
        ),
        (
            "write_file",
            Box::new(|p: String, after: &str| {
                let t = reasonix_tools::builtin::WriteFileTool::default();
                t.execute(&CallContext::new(), &json!({"path": p, "content": after}))
            }),
        ),
    ];
    for (enc_name, encode_fn) in &encodings {
        for (op, run_op) in &ops {
            let (_g, dir) = testdir("keep-enc");
            let path = Path::new(&dir).join("f.txt");
            std::fs::write(&path, encode_fn(before)).unwrap();
            run_op(path.to_string_lossy().into_owned(), after)
                .unwrap_or_else(|e| panic!("{enc_name}/{op}: {e}"));
            let got = std::fs::read(&path).unwrap();
            let want = encode_fn(after);
            assert_eq!(
                got, want,
                "{enc_name}/{op}: on disk {:x?} want {:x?}",
                got, want
            );
        }
    }
}

#[test]
fn test_edit_file_schema_matches_go() {
    let t = edit_tool();
    assert_eq!(t.name(), "edit_file");
    assert_eq!(
        t.description(),
        "Replace an exact string in a file with another. old_string must occur exactly once; add surrounding context to disambiguate. Use for targeted edits instead of rewriting the whole file."
    );
    assert_eq!(
        t.schema(),
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path"},
                "old_string": {"type": "string", "description": "Exact text to replace (must be unique in the file)"},
                "new_string": {"type": "string", "description": "Replacement text (may be empty to delete)"}
            },
            "required": ["path", "old_string", "new_string"]
        })
    );
    assert!(!t.read_only());
    assert!(reasonix_tools::abi::writes_named_paths(&t));
}

#[test]
fn test_multi_edit_schema_matches_go() {
    let t = multi_tool();
    assert_eq!(t.name(), "multi_edit");
    assert_eq!(
        t.description(),
        "Apply a list of edits to a single file atomically: each edit runs against the result of the previous one, all in memory; the file is rewritten only if every edit succeeds. Cheaper and safer than chaining edit_file calls — a failure in step 3 leaves the file untouched instead of half-edited."
    );
    assert_eq!(
        t.schema(),
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path"},
                "edits": {
                    "type": "array",
                    "minItems": 1,
                    "description": "Ordered edits. Each step sees the file as left by the previous step.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "old_string": {"type": "string", "description": "Exact text to find. Without replace_all, must match exactly once."},
                            "new_string": {"type": "string", "description": "Replacement text (empty deletes)."},
                            "replace_all": {"type": "boolean", "description": "Replace every occurrence instead of requiring uniqueness."}
                        },
                        "required": ["old_string", "new_string"]
                    }
                }
            },
            "required": ["path", "edits"]
        })
    );
    assert!(!t.read_only());
    assert!(reasonix_tools::abi::writes_named_paths(&t));
}

// The EditStep shape stays importable for composition roots.
#[allow(dead_code)]
fn _step_witness() -> EditStep {
    EditStep::default()
}
