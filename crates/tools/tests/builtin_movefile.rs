//! Oracle for movefile.go, movefile_access_test.go and the write_tool_test.go
//! move_file cases: rename into parent directories, destination-exists and
//! confinement refusals, the same-file-destination fallback, the
//! cross-filesystem copy fallback, and the source read policy.

#![forbid(unsafe_code)]

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use reasonix_tools::abi::{CallContext, Tool};
use reasonix_tools::builtin::{
    CODE_READ_FORBIDDEN, MoveFileTool, RenameFn, is_cross_device, std_rename,
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

fn tool() -> MoveFileTool {
    MoveFileTool::default()
}

fn run(
    t: &MoveFileTool,
    args: serde_json::Value,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    t.execute(&CallContext::new(), &args)
}

#[test]
fn test_move_file_moves_into_parent_dir() {
    let (_g, dir) = testdir("move-parent");
    let src = Path::new(&dir).join("a.md");
    let dst = Path::new(&dir).join("docs").join("a.md");
    std::fs::write(&src, b"hello").unwrap();
    let out = run(
        &tool(),
        json!({"source_path": src, "destination_path": dst}),
    )
    .unwrap();
    assert!(
        out.contains("moved"),
        "move_file output = {out:?}, want moved"
    );
    assert!(!src.exists(), "source still exists");
    assert_eq!(std::fs::read(&dst).unwrap(), b"hello");
}

#[test]
fn test_move_file_rejects_destination_exists() {
    let (_g, dir) = testdir("move-exists");
    let src = Path::new(&dir).join("a.md");
    let dst = Path::new(&dir).join("b.md");
    std::fs::write(&src, b"a").unwrap();
    std::fs::write(&dst, b"b").unwrap();
    let err = run(
        &tool(),
        json!({"source_path": src, "destination_path": dst}),
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains(&format!("destination {}", dst.display())),
        "{err}"
    );
}

/// Go `errors.As(err, &tool.Refusal)`: digs the typed refusal out through
/// the error chain.
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

#[test]
fn test_move_file_rejects_escape() {
    let (_g1, dir) = testdir("move-inside");
    let (_g2, outside) = testdir("move-outside");
    let src = Path::new(&dir).join("a.md");
    std::fs::write(&src, b"hello").unwrap();
    let dst = Path::new(&outside).join("a.md");
    let t = MoveFileTool {
        roots: vec![dir.clone()],
        ..MoveFileTool::default()
    };
    let err = run(&t, json!({"source_path": src, "destination_path": dst})).unwrap_err();
    let refusal = as_refusal(err.as_ref()).expect("expected typed refusal");
    assert_eq!(refusal.code, "workspace.write_outside_scope", "{err}");
    assert!(src.exists(), "source should remain after refused move");
}

#[test]
fn test_move_file_same_path_requires_existing_file() {
    let (_g, dir) = testdir("move-same-missing");
    let missing = Path::new(&dir).join("missing.md");
    let err = run(
        &tool(),
        json!({"source_path": missing, "destination_path": missing}),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("stat "),
        "expected error for missing source even when source and destination match: {err}"
    );
}

#[test]
fn test_move_file_rejects_directory_source() {
    let (_g, dir) = testdir("move-dir");
    let sub = Path::new(&dir).join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let err = run(
        &tool(),
        json!({"source_path": sub, "destination_path": Path::new(&dir).join("x")}),
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("is a directory; move_file only moves files"),
        "{err}"
    );
}

#[test]
fn test_move_file_allows_case_only_rename() {
    let (_g, dir) = testdir("move-caseonly");
    let src = Path::new(&dir).join("caseonly.txt");
    let dst = Path::new(&dir).join("CASEONLY.txt");
    std::fs::write(&src, b"hello").unwrap();
    let dst_info = std::fs::metadata(&dst);
    if dst_info.is_err() {
        // Filesystem is case-sensitive: the case-only rename is a plain
        // rename there and Go skips this test.
        eprintln!("skipping: filesystem is case-sensitive");
        return;
    }
    run(
        &tool(),
        json!({"source_path": src, "destination_path": dst}),
    )
    .unwrap();
    assert_eq!(std::fs::read(&dst).unwrap(), b"hello");
}

#[test]
fn test_move_file_falls_back_when_same_file_destination_rename_fails() {
    let (_g, dir) = testdir("move-samefile");
    let src = Path::new(&dir).join("a.md");
    std::fs::write(&src, b"hello").unwrap();
    // The same-file destination: a hard link of the source on Unix, a
    // case-duplicate spelling on Windows (std has no stable file index
    // there, so Go's os.SameFile stands in as canonical-path equality,
    // which the case-only rename is the real case for).
    let dst = if cfg!(windows) {
        Path::new(&dir).join("A.MD")
    } else {
        let linked = Path::new(&dir).join("same-file.md");
        if std::fs::hard_link(&src, &linked).is_err() {
            eprintln!("skipping: hard links unavailable");
            return;
        }
        linked
    };
    let src_path: PathBuf = src.clone();
    let dst_path: PathBuf = dst.clone();
    let injected: RenameFn = Arc::new(move |from: &Path, to: &Path| {
        if from == src_path && to == dst_path {
            return Err(io::Error::from_raw_os_error(exists_errno()));
        }
        std_rename(from, to)
    });
    let t = MoveFileTool {
        rename: injected,
        ..MoveFileTool::default()
    };
    run(&t, json!({"source_path": src, "destination_path": dst})).unwrap();
    assert_eq!(std::fs::read(&dst).unwrap(), b"hello");
    // The case-duplicate spelling IS the source on a case-insensitive
    // volume; only the hard-link variant (Unix) can assert it is gone.
    if !cfg!(windows) {
        assert!(!src.exists(), "source still exists");
    }
}

#[test]
fn test_move_file_falls_back_for_cross_device_rename() {
    let (_g, dir) = testdir("move-cross");
    let src = Path::new(&dir).join("a.md");
    let dst_dir = Path::new(&dir).join("docs");
    let dst = dst_dir.join("a.md");
    std::fs::write(&src, b"hello").unwrap();
    let injected: RenameFn = Arc::new(|_from: &Path, _to: &Path| {
        Err(io::Error::from_raw_os_error(cross_device_errno()))
    });
    let t = MoveFileTool {
        rename: injected,
        ..MoveFileTool::default()
    };
    let out = run(&t, json!({"source_path": src, "destination_path": dst})).unwrap();
    assert!(
        out.contains("moved"),
        "move_file output = {out:?}, want moved"
    );
    assert!(!src.exists(), "source still exists");
    assert_eq!(std::fs::read(&dst).unwrap(), b"hello");
}

/// Go os.ErrExist codes for the same-file fallback injection.
fn exists_errno() -> i32 {
    #[cfg(windows)]
    {
        80 // ERROR_FILE_EXISTS
    }
    #[cfg(not(windows))]
    {
        17 // EEXIST
    }
}

/// The errno is_cross_device must recognize (movefile_windows/other.go).
fn cross_device_errno() -> i32 {
    #[cfg(windows)]
    {
        17 // ERROR_NOT_SAME_DEVICE
    }
    #[cfg(not(windows))]
    {
        18 // EXDEV
    }
}

#[test]
fn test_cross_device_move_is_read_from_the_code_not_the_message() {
    // movefile_windows_test.go: a localized message wrapping the errno.
    let localized = io::Error::from_raw_os_error(cross_device_errno());
    assert!(
        is_cross_device(&localized),
        "ERROR_NOT_SAME_DEVICE/EXDEV was not recognized"
    );
    let other = io::Error::from_raw_os_error(if cfg!(windows) { 5 } else { 13 });
    assert!(
        !is_cross_device(&other),
        "an unrelated failure was treated as cross-volume"
    );
}

#[test]
fn test_move_file_rename_only_requires_read_of_its_source() {
    let (_g, dir) = testdir("move-policy");
    let private = Path::new(&dir).join("private");
    std::fs::create_dir_all(&private).unwrap();
    let forbid = private.to_string_lossy().into_owned();

    // protected destination: only written, never read.
    let src = Path::new(&dir).join("source.txt");
    let dst = private.join("server.key");
    std::fs::write(&src, b"synthetic content\n").unwrap();
    let t = MoveFileTool {
        forbid_roots: vec![forbid.clone()],
        ..MoveFileTool::default()
    };
    run(&t, json!({"source_path": src, "destination_path": dst})).unwrap();
    assert_eq!(std::fs::read(&dst).unwrap(), b"synthetic content\n");
    assert!(!src.exists(), "source remained after rename");

    // protected source: refused before anything moves.
    let src2 = private.join("note.txt");
    let dst2 = Path::new(&dir).join("leak.txt");
    let secret = "PROTECTED_RENAME_CANARY\n";
    std::fs::write(&src2, secret).unwrap();
    let t = MoveFileTool {
        forbid_roots: vec![forbid],
        ..MoveFileTool::default()
    };
    let err = run(&t, json!({"source_path": src2, "destination_path": dst2})).unwrap_err();
    let refusal = err
        .downcast_ref::<reasonix_tools::refusal::Refusal>()
        .unwrap_or_else(|| panic!("execution must refuse a forbidden source, got {err}"));
    assert_eq!(refusal.code, CODE_READ_FORBIDDEN);
    assert_eq!(
        std::fs::read(&src2).unwrap(),
        secret.as_bytes(),
        "source changed"
    );
    assert!(
        !dst2.exists(),
        "a refused move left the content at the destination"
    );
}

#[test]
fn test_move_file_copy_fallback_agrees_with_admission() {
    for forbidden_source in [false, true] {
        let (_g, dir) = testdir("move-copy-fallback");
        let src = Path::new(&dir).join("source.txt");
        let dst = Path::new(&dir).join("moved.txt");
        let original = "COPY_SOURCE_CANARY\n";
        std::fs::write(&src, original.as_bytes()).unwrap();
        let forbidden = if forbidden_source {
            src.to_string_lossy().into_owned()
        } else {
            dst.to_string_lossy().into_owned()
        };
        let t = MoveFileTool {
            forbid_roots: vec![forbidden],
            rename: Arc::new(|_f: &Path, _t: &Path| {
                Err(io::Error::from_raw_os_error(cross_device_errno()))
            }),
            ..MoveFileTool::default()
        };
        let err = run(&t, json!({"source_path": src, "destination_path": dst}));
        if forbidden_source {
            let refusal =
                as_refusal(err.unwrap_err().as_ref()).expect("copy must deny forbidden source");
            assert_eq!(refusal.code, CODE_READ_FORBIDDEN);
            assert_eq!(
                std::fs::read(&src).unwrap(),
                original.as_bytes(),
                "source changed"
            );
            assert!(!dst.exists(), "denied copy created destination");
        } else {
            err.unwrap();
            assert_eq!(std::fs::read(&dst).unwrap(), original.as_bytes());
        }
    }
}

#[test]
fn test_move_file_schema_matches_go() {
    let t = tool();
    assert_eq!(t.name(), "move_file");
    assert_eq!(
        t.description(),
        "Move or rename a file from source_path to destination_path. Creates the destination parent directory as needed. Use instead of shell mv, Move-Item, or ren for file moves so workspace confinement and file-edit permissions apply."
    );
    assert_eq!(
        t.schema(),
        json!({
            "type": "object",
            "properties": {
                "source_path": {"type": "string", "description": "Existing file path to move"},
                "destination_path": {"type": "string", "description": "Destination file path; must not already exist"}
            },
            "required": ["source_path", "destination_path"]
        })
    );
    assert!(!t.read_only());
    assert!(reasonix_tools::abi::writes_named_paths(&t));
}
