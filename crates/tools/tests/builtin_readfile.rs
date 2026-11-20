//! Oracle for readfile.go (readfile_window/nested/utf16/image tests): the
//! numbered scan window, the nested verbatim mode, BOM-less UTF-16 decode,
//! and the image refusal. The GB18030 streaming test and the bounded-memory
//! assertion depend on the Go streaming transformer seam; the decode side is
//! covered through the shared fileencoding cascade instead.

#![forbid(unsafe_code)]

use reasonix_tools::abi::{CallContext, Tool};
use reasonix_tools::builtin::{READ_FILE_DEFAULT_LIMIT, ReadFileTool};

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

fn tool() -> ReadFileTool {
    ReadFileTool {
        net_roots: Vec::new(),
        work_dir: String::new(),
        forbid_roots: Vec::new(),
        read_roots: Vec::new(),
    }
}

#[test]
fn test_scan_windowed_read_stops_early() {
    // A small windowed read of a large file must read only a small prefix,
    // not all of it (readfile_window_test.go).
    let body: String = (1..=100_000).map(|i| format!("line {i}\n")).collect();
    let mut out = String::new();
    let mut reader = body.as_bytes();
    reasonix_tools::builtin::readfile::scan(&mut reader, 0, 3).unwrap();
    let _ = &mut out;
    // The scan consumed at most the first lines' bytes: feed a counting
    // reader to bound consumption.
    struct Counting<'a> {
        r: &'a [u8],
        n: usize,
    }
    impl<'a> std::io::Read for Counting<'a> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.r.read(buf)?;
            self.n += n;
            Ok(n)
        }
    }
    let mut cr = Counting {
        r: body.as_bytes(),
        n: 0,
    };
    let out = reasonix_tools::builtin::readfile::scan(&mut cr, 0, 3).unwrap();
    assert!(out.contains("1→line 1"), "{out}");
    assert!(out.contains("3→line 3"), "{out}");
    assert!(!out.contains("line 4"), "window leaked line 4:\n{out}");
    assert!(out.contains("more lines below"), "{out}");
    assert!(
        cr.n <= 100 * 1024,
        "read {} of {} bytes for a 3-line window; should read a small prefix",
        cr.n,
        body.len()
    );
}

#[test]
fn test_scan_past_eof_and_empty() {
    let out = reasonix_tools::builtin::readfile::scan(&mut std::io::empty(), 0, 5).unwrap();
    assert_eq!(out, "(empty file)");
    let body = "one\ntwo\nthree\n";
    let out = reasonix_tools::builtin::readfile::scan(&mut body.as_bytes(), 10, 5).unwrap();
    assert_eq!(out, "(offset 10 is past EOF — file has 3 lines)");
}

#[test]
fn test_read_file_nested_returns_the_text() {
    let (_g, dir) = testdir("rf-nested");
    let crlf = format!("{dir}{}crlf.txt", sep());
    std::fs::write(&crlf, b"one\r\ntwo\r\nthree").unwrap();
    std::fs::write(format!("{dir}{}bom.txt", sep()), "\u{FEFF}hi\n".as_bytes()).unwrap();
    std::fs::write(format!("{dir}{}empty.txt", sep()), b"").unwrap();
    let long = "x\n".repeat(READ_FILE_DEFAULT_LIMIT + 5);
    std::fs::write(format!("{dir}{}long.txt", sep()), &long).unwrap();

    let t = tool();
    let ctx = CallContext::new().mark_nested();
    let cases: Vec<(String, serde_json::Value, &str)> = vec![
        (
            "crlf whole".into(),
            serde_json::json!({"path": crlf}),
            "one\r\ntwo\r\nthree",
        ),
        (
            "bom stripped".into(),
            serde_json::json!({"path": format!("{dir}{}bom.txt", sep())}),
            "hi\n",
        ),
        (
            "empty".into(),
            serde_json::json!({"path": format!("{dir}{}empty.txt", sep())}),
            "",
        ),
        (
            "past the default page".into(),
            serde_json::json!({"path": format!("{dir}{}long.txt", sep())}),
            long.as_str(),
        ),
        (
            "window".into(),
            serde_json::json!({"path": crlf, "offset": 1, "limit": 1}),
            "two\r\n",
        ),
        (
            "last line".into(),
            serde_json::json!({"path": crlf, "offset": 2}),
            "three",
        ),
        (
            "past eof".into(),
            serde_json::json!({"path": crlf, "offset": 9}),
            "",
        ),
    ];
    for (name, args, want) in cases {
        let got = Tool::execute(&t, &ctx, &args).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(got, want, "{name}");
    }
    // The model's own read keeps its line numbers.
    let args = serde_json::json!({"path": crlf});
    let out = Tool::execute(&t, &CallContext::new(), &args).unwrap();
    assert!(out.contains("1→one"), "line numbers lost:\n{out}");
}

fn sep() -> String {
    reasonix_tools::builtin::pathgo::separator().to_string()
}

#[test]
fn test_read_file_bom_less_utf16_le() {
    let (_g, dir) = testdir("rf-utf16");
    let path = format!("{dir}{}CameraManagerLogLoader.cpp", sep());
    let src = "// Created by 69431 on 2024/12/31\n#include \"alson/CameraManagerLogLoader.h\"\n";
    let mut bytes = Vec::new();
    for unit in src.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(&path, &bytes).unwrap();
    let t = tool();
    let args = serde_json::json!({"path": path});
    let out = Tool::execute(&t, &CallContext::new(), &args)
        .unwrap_or_else(|e| panic!("read rejected BOM-less UTF-16: {e}"));
    assert!(out.contains("Created by 69431"), "{out}");
    assert!(out.contains("#include"), "{out}");
    assert!(!out.contains('\0'), "NUL bytes leaked into decoded output");
}

#[test]
fn test_read_file_directory_message() {
    let (_g, dir) = testdir("rf-dir");
    let t = tool();
    let args = serde_json::json!({"path": dir});
    let err = Tool::execute(&t, &CallContext::new(), &args)
        .unwrap_err()
        .to_string();
    assert!(err.contains("is a directory, not a file"), "{err}");
    assert!(err.contains("ls tool"), "{err}");
}

#[test]
fn test_read_file_image_refusal_points_at_the_routes_that_work() {
    let (_g, dir) = testdir("rf-image");
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&[0u8; 32]);
    std::fs::write(format!("{dir}{}shot.png", sep()), &png).unwrap();
    let t = ReadFileTool {
        net_roots: Vec::new(),
        work_dir: dir.clone(),
        forbid_roots: Vec::new(),
        read_roots: Vec::new(),
    };
    let args = serde_json::json!({"path": "shot.png"});
    let err = Tool::execute(&t, &CallContext::new(), &args)
        .unwrap_err()
        .to_string();
    for want in [
        "PNG image",
        "view_image",
        "references it as @",
        "shot.png",
        "ask them to",
    ] {
        assert!(err.contains(want), "refusal lacks {want:?}: {err}");
    }
}

#[test]
fn test_read_file_binary_refusal_names_hexdump() {
    let (_g, dir) = testdir("rf-binary");
    let path = format!("{dir}{}blob.bin", sep());
    std::fs::write(&path, b"ok\x00binary").unwrap();
    let t = tool();
    let args = serde_json::json!({"path": path});
    let err = Tool::execute(&t, &CallContext::new(), &args)
        .unwrap_err()
        .to_string();
    assert!(err.contains("binary file"), "{err}");
    assert!(err.contains("NUL byte detected"), "{err}");
    assert!(err.contains("bash hexdump"), "{err}");
}

#[test]
fn test_read_file_missing_path_error() {
    let (_g, dir) = testdir("rf-missing");
    let t = tool();
    let args = serde_json::json!({"path": format!("{dir}{}absent.txt", sep())});
    let err = Tool::execute(&t, &CallContext::new(), &args)
        .unwrap_err()
        .to_string();
    assert!(err.contains("read "), "{err}");
}

#[test]
fn test_read_file_line_number_formatting() {
    let (_g, dir) = testdir("rf-numbering");
    let path = format!("{dir}{}num.txt", sep());
    std::fs::write(&path, b"alpha\nbeta\ngamma\n").unwrap();
    let t = tool();
    let args = serde_json::json!({"path": path});
    let out = Tool::execute(&t, &CallContext::new(), &args).unwrap();
    assert!(out.contains("1→alpha"), "{out}");
    assert!(out.contains("2→beta"), "{out}");
    assert!(out.contains("3→gamma"), "{out}");
    // A window past the first line numbers with the file's width.
    let args = serde_json::json!({"path": path, "offset": 1, "limit": 1});
    let out = Tool::execute(&t, &CallContext::new(), &args).unwrap();
    assert!(out.contains("2→beta"), "{out}");
    assert!(out.contains("more lines below"), "{out}");
}
