//! Oracle for writefile.go + writefile_access_test.go + e2e_encoding_test.go
//! (write_file cases): creation, overwrite, encoding preservation, the
//! same-content no-op, the write receipt, and the overwrite read-policy
//! refusals. The overlay/preview and Windows trailing-dot-alias cases ride
//! the fileviews/admission slices and are anchored, not skipped.

#![forbid(unsafe_code)]

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use reasonix_core::fileencoding::{Kind, detect_and_decode};
use reasonix_tools::abi::{CallContext, Tool};
use reasonix_tools::builtin::{
    CODE_OVERWRITE_READ_FORBIDDEN, CODE_OVERWRITE_READ_OUTSIDE_SCOPE, FileOverlay,
    OVERWRITE_READ_ACCESS_DENIED_MESSAGE, WriteFileTool, encode,
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

fn tool() -> WriteFileTool {
    WriteFileTool::default()
}

fn run(
    t: &WriteFileTool,
    args: serde_json::Value,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    t.execute(&CallContext::new(), &args)
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

/// editsource.go FileOverlay probe: counts reads/writes, never serves a
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
fn test_write_file_creates_parent_dirs() {
    let (_guard, dir) = testdir("writefile-parents");
    let f = Path::new(&dir)
        .join("a")
        .join("b")
        .join("c")
        .join("file.txt");
    let out = run(&tool(), json!({"path": f, "content": "nested"})).unwrap();
    assert_eq!(out, format!("wrote 6 bytes to {}", f.display()));
    assert_eq!(std::fs::read(&f).unwrap(), b"nested");
}

#[test]
fn test_write_file_overwrites() {
    let (_guard, dir) = testdir("writefile-overwrite");
    let f = Path::new(&dir).join("x.txt");
    std::fs::write(&f, b"old").unwrap();
    run(&tool(), json!({"path": f, "content": "new"})).unwrap();
    assert_eq!(std::fs::read(&f).unwrap(), b"new");
}

#[test]
fn test_write_file_records_local_prior() {
    let (_guard, dir) = testdir("writefile-receipt");
    let f = Path::new(&dir).join("receipt.txt");
    std::fs::write(&f, b"old").unwrap();
    let got: Arc<Mutex<(String, bool, Vec<u8>)>> =
        Arc::new(Mutex::new((String::new(), false, Vec::new())));
    let sink = got.clone();
    let w = WriteFileTool {
        receipt: Some(Arc::new(
            move |path: &str, had_prior: bool, prior: &[u8]| {
                *sink.lock().unwrap() = (path.to_owned(), had_prior, prior.to_vec());
            },
        )),
        ..WriteFileTool::default()
    };
    run(&w, json!({"path": f, "content": "new"})).unwrap();
    let g = got.lock().unwrap().clone();
    assert_eq!(g.0, f.to_string_lossy());
    assert!(g.1, "hadPrior must be set for an overwrite");
    assert_eq!(g.2, b"old");
}

#[test]
fn test_write_file_same_content_no_op() {
    let (_guard, dir) = testdir("writefile-noop");
    let f = Path::new(&dir).join("x.txt");
    std::fs::write(&f, b"same").unwrap();
    let out = run(&tool(), json!({"path": f, "content": "same"})).unwrap();
    assert!(
        out.contains("already contains the exact content"),
        "same-content write should return a no-op signal, got {out:?}"
    );
    assert_eq!(std::fs::read(&f).unwrap(), b"same");
}

#[test]
fn test_write_file_empty_content() {
    let (_guard, dir) = testdir("writefile-empty");
    let f = Path::new(&dir).join("empty.txt");
    run(&tool(), json!({"path": f, "content": ""})).unwrap();
    assert_eq!(std::fs::read(&f).unwrap().len(), 0, "expected empty file");
}

#[test]
fn test_write_file_missing_path() {
    let err = run(&tool(), json!({"content": "x"})).unwrap_err();
    assert!(err.to_string().contains("path is required"), "{err}");
}

#[test]
fn test_write_file_missing_content_writes_empty_file() {
    let (_guard, dir) = testdir("writefile-missing-content");
    let f = Path::new(&dir).join("x.txt");
    run(&tool(), json!({"path": f})).unwrap();
    assert_eq!(std::fs::read(&f).unwrap().len(), 0);
}

#[test]
fn test_write_file_creates_without_read_permission() {
    let (_guard, dir) = testdir("writefile-new-protected");
    let path = Path::new(&dir).join("server.key");
    let probe = Arc::new(ProbeOverlay {
        reads: AtomicUsize::new(0),
        writes: AtomicUsize::new(0),
    });
    let w = WriteFileTool {
        forbid_roots: vec![dir.clone()],
        overlay: Some(probe.clone()),
        ..WriteFileTool::default()
    };
    run(&w, json!({"path": path, "content": "new data\n"})).unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"new data\n",
        "a new file must not require reading its directory"
    );
    assert_eq!(
        probe.reads.load(Ordering::SeqCst),
        0,
        "new file accessed prior content"
    );
}

#[test]
fn test_write_file_rejects_forbidden_existing_source() {
    let (_guard, dir) = testdir("writefile-forbidden-source");
    let path = Path::new(&dir).join("existing.txt");
    let original = "EXISTING_CANARY_DO_NOT_READ\n";
    std::fs::write(&path, original).unwrap();
    let probe = Arc::new(ProbeOverlay {
        reads: AtomicUsize::new(0),
        writes: AtomicUsize::new(0),
    });
    let w = WriteFileTool {
        forbid_roots: vec![path.to_string_lossy().into_owned()],
        overlay: Some(probe.clone()),
        ..WriteFileTool::default()
    };
    let err = run(&w, json!({"path": path, "content": "replacement"})).unwrap_err();
    let refusal = as_refusal(err.as_ref()).expect("expected typed refusal");
    assert_eq!(refusal.code, CODE_OVERWRITE_READ_FORBIDDEN);
    assert_eq!(refusal.message, OVERWRITE_READ_ACCESS_DENIED_MESSAGE);
    assert_eq!(
        probe.reads.load(Ordering::SeqCst),
        0,
        "forbidden source reached the overlay"
    );
    assert_eq!(probe.writes.load(Ordering::SeqCst), 0);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        original.as_bytes(),
        "protected file changed"
    );
}

#[test]
fn test_write_file_rejects_existing_source_outside_read_scope() {
    let (_guard, dir) = testdir("writefile-scope-source");
    let path = Path::new(&dir).join("existing.txt");
    std::fs::write(&path, "original\n").unwrap();
    let (other_guard, other) = testdir("writefile-scope-other");
    let w = WriteFileTool {
        read_roots: vec![other],
        ..WriteFileTool::default()
    };
    drop(other_guard);
    let err = run(&w, json!({"path": path, "content": "replacement"})).unwrap_err();
    let refusal = as_refusal(err.as_ref()).expect("expected typed refusal");
    assert_eq!(refusal.code, CODE_OVERWRITE_READ_OUTSIDE_SCOPE);
    assert_eq!(refusal.message, OVERWRITE_READ_ACCESS_DENIED_MESSAGE);
}

#[test]
fn test_write_file_allowed_source_preserves_encoding_and_receipt() {
    let (_guard, dir) = testdir("writefile-utf16");
    let path = Path::new(&dir).join("existing.txt");
    let original = "old content\n";
    let encoded = encode(original, Kind::Utf16Le).unwrap();
    std::fs::write(&path, &encoded).unwrap();
    let got: Arc<Mutex<(bool, String)>> = Arc::new(Mutex::new((false, String::new())));
    let sink = got.clone();
    let w = WriteFileTool {
        receipt: Some(Arc::new(
            move |_path: &str, had_prior: bool, prior: &[u8]| {
                // The receipt carries the decoded prior content (Go:
                // prior = []byte(src.content)).
                *sink.lock().unwrap() = (had_prior, String::from_utf8_lossy(prior).into_owned());
            },
        )),
        ..WriteFileTool::default()
    };
    run(&w, json!({"path": path, "content": "updated content\n"})).unwrap();
    let raw = std::fs::read(&path).unwrap();
    let (kind, text) = detect_and_decode(&raw);
    assert_eq!(
        kind,
        Kind::Utf16Le,
        "overwrite must keep the file's charset"
    );
    assert_eq!(String::from_utf8_lossy(&text), "updated content\n");
    let g = got.lock().unwrap().clone();
    assert!(g.0);
    assert_eq!(g.1, original);
}

#[test]
fn test_e2e_write_file_preserves_gbk_on_overwrite() {
    let (_guard, dir) = testdir("writefile-gbk");
    let path = Path::new(&dir).join("gbk.txt");
    std::fs::write(&path, encode("原始内容\n", Kind::Gb18030).unwrap()).unwrap();
    run(
        &tool(),
        json!({"path": path, "content": "全新中文内容\n第二行\n"}),
    )
    .unwrap();
    let raw = std::fs::read(&path).unwrap();
    assert!(
        std::str::from_utf8(&raw).is_err() || raw == "全新中文内容\n第二行\n".as_bytes(),
        "write_file rewrote an existing GBK file as UTF-8 on disk"
    );
    let decoded = reasonix_core::fileencoding::decode_to_utf8(&raw);
    let s = String::from_utf8_lossy(&decoded).into_owned();
    assert!(s.contains("全新中文内容") && s.contains("第二行"), "{s}");
}

#[test]
fn test_e2e_write_file_new_file_is_utf8() {
    let (_guard, dir) = testdir("writefile-new-utf8");
    let path = Path::new(&dir).join("new.txt");
    run(&tool(), json!({"path": path, "content": "新文件中文\n"})).unwrap();
    let raw = std::fs::read(&path).unwrap();
    assert_eq!(
        raw,
        "新文件中文\n".as_bytes(),
        "a new file should be written as UTF-8"
    );
}

#[test]
fn test_file_tools_keep_utf8_bom() {
    // encoding_preserve_test.go's utf8-bom row for write_file.
    let (_guard, dir) = testdir("writefile-bom");
    let path = Path::new(&dir).join("f.txt");
    let before = "标题：旧的说明\r\n第二行\r\n";
    let after = "标题：新的说明\r\n第二行\r\n";
    let mut seeded = vec![0xEF, 0xBB, 0xBF];
    seeded.extend_from_slice(before.as_bytes());
    std::fs::write(&path, &seeded).unwrap();
    run(&tool(), json!({"path": path, "content": after})).unwrap();
    let mut want = vec![0xEF, 0xBB, 0xBF];
    want.extend_from_slice(after.as_bytes());
    assert_eq!(std::fs::read(&path).unwrap(), want);
}

#[test]
fn test_write_file_schema_matches_go() {
    let t = tool();
    assert_eq!(t.name(), "write_file");
    assert_eq!(
        t.description(),
        "Write full file content, overwriting and creating parent directories. Prefer edit_file/multi_edit for partial changes."
    );
    assert_eq!(
        t.schema(),
        json!({
            "type": "object",
            "properties": {"path": {"type": "string"}, "content": {"type": "string"}},
            "required": ["path", "content"]
        })
    );
    assert!(!t.read_only());
}
