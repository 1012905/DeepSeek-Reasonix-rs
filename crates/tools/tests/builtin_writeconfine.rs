//! Oracle for the write side of confine.go (confine, confineWrite,
//! underSessionTemp, withSessionTempAlternative) — the write-tool slice's
//! share of confine_test.go. The session-data guard's tier texts and the
//! managed-config approval flow live with the safety/config slices; here
//! the hook traits carry Go's zero-value semantics (None = unconfined).

#![forbid(unsafe_code)]

use std::path::Path;
use std::sync::Arc;

use reasonix_tools::abi::Tool as _;
use reasonix_tools::builtin::{
    ConfineError, SessionDataGuardHook, SessionTempSource, WriteFileTool, confine, confine_write,
    under_session_temp,
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

#[test]
fn test_confine_rejects_escape() {
    let (_g, dir) = testdir("confine-escape");
    let escape = Path::new(&dir).join("..").join("outside").join("file.txt");
    let err = confine(std::slice::from_ref(&dir), &escape.to_string_lossy()).unwrap_err();
    let ConfineError::Outside(refusal) = err else {
        panic!("want outside refusal");
    };
    assert_eq!(refusal.code, "workspace.write_outside_scope");
}

#[test]
fn test_confine_allows_inside() {
    let (_g, dir) = testdir("confine-inside");
    let real = std::fs::canonicalize(&dir)
        .map(|p| p.to_string_lossy().trim_start_matches(r"\\?\").to_owned())
        .unwrap_or_else(|_| dir.clone());
    let target = Path::new(&real).join("inside").join("file.txt");
    confine(&[real], &target.to_string_lossy()).unwrap();
}

#[test]
fn test_confine_empty_roots_allows_all() {
    confine(&[], "/any/path").unwrap();
}

/// A guard that refuses everything, so tests can see which branch ran.
struct RefuseAll;

impl SessionDataGuardHook for RefuseAll {
    fn check(&self, target: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Err(Box::new(std::io::Error::other(format!(
            "guard refused {target}"
        ))))
    }
}

struct FixedTemp(String);

impl SessionTempSource for FixedTemp {
    fn dir(&self) -> Option<String> {
        Some(self.0.clone())
    }
}

fn hooks(
    guard: Option<Arc<dyn SessionDataGuardHook>>,
    temp: Option<Arc<dyn SessionTempSource>>,
) -> reasonix_tools::builtin::WriteHooks {
    reasonix_tools::builtin::WriteHooks {
        guard,
        managed: None,
        session_temp: temp,
    }
}

#[test]
fn test_confine_write_inside_roots_passes_without_guard() {
    let (_g, dir) = testdir("confine-write-inside");
    let target = Path::new(&dir).join("a.txt");
    confine_write(
        std::slice::from_ref(&dir),
        &hooks(None, None),
        &target.to_string_lossy(),
    )
    .unwrap();
}

#[test]
fn test_confine_write_outside_roots_refuses_with_the_workspace_code() {
    let (_g, dir) = testdir("confine-write-outside");
    let (_g2, other) = testdir("confine-write-elsewhere");
    let target = Path::new(&other).join("b.txt");
    let err = confine_write(
        std::slice::from_ref(&dir),
        &hooks(None, None),
        &target.to_string_lossy(),
    )
    .unwrap_err();
    // The refusal text names the target, the roots and the setting, with
    // the target printed verbatim.
    let text = err.to_string();
    assert!(text.contains("workspace.write_outside_scope"), "{text}");
    assert!(text.contains("write refused"), "{text}");
    assert!(text.contains("allow_write"), "{text}");
}

#[test]
fn test_confine_write_checks_the_guard_inside_and_outside() {
    // Inside the roots: the guard runs and its refusal surfaces (a write
    // can be inside the roots and still target the session stores).
    let (_g, dir) = testdir("confine-write-guard-in");
    let inside = Path::new(&dir).join("sessions").join("s.json");
    let err = confine_write(
        std::slice::from_ref(&dir),
        &hooks(Some(Arc::new(RefuseAll)), None),
        &inside.to_string_lossy(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("guard refused"), "{err}");

    // Outside the roots: confinement refuses first, the guard never runs.
    let (_g2, other) = testdir("confine-write-guard-out");
    let outside = Path::new(&other).join("x.txt");
    let err = confine_write(
        std::slice::from_ref(&dir),
        &hooks(Some(Arc::new(RefuseAll)), None),
        &outside.to_string_lossy(),
    )
    .unwrap_err();
    assert!(
        !err.to_string().contains("guard refused"),
        "confinement must answer before the guard: {err}"
    );
    assert!(
        err.to_string().contains("workspace.write_outside_scope"),
        "{err}"
    );
}

#[test]
fn test_confine_write_session_temp_is_writable_outside_the_roots() {
    let (_g, dir) = testdir("confine-write-temp");
    let (_g2, temp) = testdir("confine-write-tempdir");
    let target = Path::new(&temp).join("scratch.txt");
    // With no guard bound, a temp write passes like Go's nil guard.
    confine_write(
        std::slice::from_ref(&dir),
        &hooks(None, Some(Arc::new(FixedTemp(temp.clone())))),
        &target.to_string_lossy(),
    )
    .unwrap();
    assert!(under_session_temp(
        Some(&FixedTemp(temp.clone())),
        &target.to_string_lossy()
    ));
    assert!(!under_session_temp(Some(&FixedTemp(temp.clone())), &dir));
}

#[test]
fn test_refused_scratch_write_names_the_temp_alternative() {
    let (_g, dir) = testdir("confine-write-alt");
    let (_g2, temp) = testdir("confine-write-alt-dir");
    let target = Path::new(&dir).join("..").join("gone.txt");
    let err = confine_write(
        std::slice::from_ref(&dir),
        &hooks(None, Some(Arc::new(FixedTemp(temp.clone())))),
        &target.to_string_lossy(),
    )
    .unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains(&format!(
            "; scratch files may also go under $TMPDIR ({temp})"
        )),
        "the refusal must name the one writable place outside the roots: {text}"
    );
}

#[test]
fn test_write_file_refuses_outside_roots_through_the_tool() {
    let (_g, dir) = testdir("confine-tool");
    let (_g2, other) = testdir("confine-tool-other");
    let t = WriteFileTool {
        roots: vec![dir.clone()],
        ..WriteFileTool::default()
    };
    let err = t
        .execute(
            &reasonix_tools::abi::CallContext::new(),
            &json!({"path": Path::new(&other).join("x.txt"), "content": "x"}),
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("workspace.write_outside_scope"),
        "{err}"
    );
    assert!(
        !Path::new(&other).join("x.txt").exists(),
        "a refused write must not create the file"
    );
}
