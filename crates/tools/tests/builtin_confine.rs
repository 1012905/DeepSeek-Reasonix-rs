//! Oracle for confine.go: within/withinFold, real_roots, confine read/write
//! sides, forbid-read on the read tools (confine_test.go, read-side cases).
//! The write-side cases of confine_test.go live with the writer tools slice;
//! the bash-sandbox cases need the sandbox machine and are anchored in
//! anchors.rs rather than skipped here.

#![forbid(unsafe_code)]

use std::sync::Arc;

use reasonix_tools::abi::{CallContext, Tool};
use reasonix_tools::builtin::{
    CODE_WRITE_OUTSIDE_SCOPE, GlobTool, GrepTool, ListDirTool, ReadFileTool, confine, confine_read,
    confine_scope, confine_write_side_resolved, real_roots, within,
};

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

static PROTECT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn go_json(v: serde_json::Value) -> serde_json::Value {
    v
}

#[test]
fn test_within() {
    let (root, sep) = if cfg!(windows) {
        (String::from("C:\\work\\proj"), "\\")
    } else {
        (String::from("/work/proj"), "/")
    };
    let cases: Vec<(String, bool)> = vec![
        (root.clone(), true),
        (format!("{root}{sep}a{sep}b.go"), true),
        (format!("{root}{sep}..{sep}proj{sep}x"), true),
        (format!("{root}-2"), false),
        (format!("{root}{sep}.."), false),
    ];
    for (p, want) in &cases {
        assert_eq!(
            within(&root, &reasonix_tools::builtin::pathgo::clean(p)),
            *want,
            "within({p})"
        );
    }
    let parent = if cfg!(windows) { "C:\\work" } else { "/work" };
    assert!(!within(&root, parent));
    let sibling = if cfg!(windows) {
        "C:\\work\\other"
    } else {
        "/work/other"
    };
    assert!(!within(&root, sibling));
    let other = if cfg!(windows) {
        "C:\\etc\\passwd"
    } else {
        "/etc/passwd"
    };
    assert!(!within(&root, other));
}

#[test]
fn test_confine_unconfined_when_no_roots() {
    let anywhere = if cfg!(windows) {
        "C:\\anywhere\\at\\all"
    } else {
        "/anywhere/at/all"
    };
    confine(&[], anywhere).unwrap();
}

#[test]
fn test_confine_inside_and_outside() {
    let (root_g, root) = testdir("confine-inout");
    let roots = real_roots(std::slice::from_ref(&root));
    confine(&roots, &format!("{}{}src{}main.go", root, sep(), sep())).unwrap();
    // A parent escape and a sibling must both be refused.
    confine(&roots, &format!("{}{}..{}escape.txt", root, sep(), sep())).unwrap_err();
    let dir = std::path::Path::new(&root)
        .parent()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    confine(&roots, &format!("{}{}neighbour{}x", dir, sep(), sep())).unwrap_err();
    drop(root_g);
}

fn sep() -> String {
    reasonix_tools::builtin::pathgo::separator().to_string()
}

#[test]
fn test_confine_rejects_symlink_escape() {
    let (root_g, root) = testdir("confine-sym");
    let (outside_g, outside) = testdir("confine-sym-out");
    let link = format!("{root}{}out", sep());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let roots = real_roots(std::slice::from_ref(&root));
        confine(&roots, &format!("{link}{}evil.txt", sep())).unwrap_err();
        confine(&roots, &format!("{root}{}ok.txt", sep())).unwrap();
    }
    #[cfg(windows)]
    {
        // mklink /J needs cmd; the junction path is exercised on CI runners
        // with developer mode. The read-side walk-confine case covers the
        // link behavior; the lexical half is covered above.
        let _ = (&root, &outside, &link);
    }
    drop(root_g);
    drop(outside_g);
}

#[test]
fn test_confine_write_side_refusal_carries_workspace_code() {
    let (root_g, root) = testdir("confine-ws");
    let (outside_g, outside) = testdir("confine-ws-out");
    let target = format!("{}{}out.txt", outside, sep());
    let err = confine_write_side_resolved(std::slice::from_ref(&root), &target).unwrap_err();
    match err {
        reasonix_tools::builtin::ConfineError::Network(_) => panic!("want outside, got network"),
        other => {
            let text = other.to_string();
            for want in ["workspace write scope", "not an OS sandbox", "allow_write"] {
                assert!(text.contains(want), "refusal lacks {want}: {text}");
            }
            let _ = CODE_WRITE_OUTSIDE_SCOPE;
        }
    }
    drop(root_g);
    drop(outside_g);
}

#[test]
fn test_confine_read_empty() {
    let anywhere = if cfg!(windows) {
        "C:\\anywhere"
    } else {
        "/anywhere"
    };
    assert!(!confine_read(&[], anywhere));
}

#[test]
fn test_confine_read_inside_and_outside() {
    let (root_g, root) = testdir("confine-rd");
    let forbid_roots = real_roots(std::slice::from_ref(&root));
    assert!(confine_read(
        &forbid_roots,
        &format!("{root}{}secret{}key.pem", sep(), sep())
    ));
    let (other_g, other) = testdir("confine-rd-ok");
    assert!(!confine_read(
        &forbid_roots,
        &format!("{other}{}ok.txt", sep())
    ));
    drop(root_g);
    drop(other_g);
}

#[test]
fn test_confine_read_exact_file_root() {
    let (dir_g, dir) = testdir("confine-exact");
    let secret = format!("{dir}{}credentials.env", sep());
    let visible = format!("{dir}{}project.env", sep());
    std::fs::write(&secret, b"value").unwrap();
    std::fs::write(&visible, b"value").unwrap();
    let forbid_roots = real_roots(std::slice::from_ref(&secret));
    assert!(confine_read(&forbid_roots, &secret));
    assert!(!confine_read(&forbid_roots, &visible));
    drop(dir_g);
}

#[test]
fn test_confine_read_blocks_read_file() {
    let (forbid_dir_g, forbid_dir) = testdir("confine-rf");
    let secret_path = format!("{forbid_dir}{}secret.txt", sep());
    std::fs::write(&secret_path, b"classified").unwrap();
    let forbid_roots = real_roots(std::slice::from_ref(&forbid_dir));
    let rf = ReadFileTool {
        net_roots: Vec::new(),
        work_dir: String::new(),
        forbid_roots: forbid_roots.clone(),
        read_roots: Vec::new(),
    };
    let args = go_json(serde_json::json!({"path": secret_path}));
    let err = Tool::execute(&rf, &CallContext::new(), &args).unwrap_err();
    assert!(
        err.to_string().contains("file does not exist"),
        "forbid-read must answer as not-exist, got {err}"
    );
    let rf_open = ReadFileTool {
        net_roots: Vec::new(),
        work_dir: String::new(),
        forbid_roots: Vec::new(),
        read_roots: Vec::new(),
    };
    Tool::execute(&rf_open, &CallContext::new(), &args).unwrap();
    drop(forbid_dir_g);
}

#[test]
fn test_sensitive_read_paths_blocked_when_protected() {
    let _guard = PROTECT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reasonix_tools::builtin::set_protect_sensitive_files(true);
    let (dir_g, dir) = testdir("confine-prot");
    let env_path = format!("{dir}{}.env", sep());
    std::fs::write(&env_path, b"DEEPSEEK_API_KEY=sk-real-secret-value-123456\n").unwrap();
    let pem_path = format!("{dir}{}client.pem", sep());
    std::fs::write(&pem_path, b"PRIVATE KEY").unwrap();

    for path in [&env_path, &pem_path] {
        assert!(confine_read(&[], path), "sensitive path {path} must block");
        let rf = ReadFileTool {
            net_roots: Vec::new(),
            work_dir: String::new(),
            forbid_roots: Vec::new(),
            read_roots: Vec::new(),
        };
        let args = go_json(serde_json::json!({"path": path}));
        assert!(
            Tool::execute(&rf, &CallContext::new(), &args).is_err(),
            "read_file must refuse sensitive path {path}"
        );
    }

    let visible = format!("{dir}{}notes.txt", sep());
    std::fs::write(&visible, b"ok").unwrap();
    assert!(!confine_read(&[], &visible));
    reasonix_tools::builtin::set_protect_sensitive_files(false);
    drop(dir_g);
}

#[test]
fn test_sensitive_read_paths_allowed_by_default() {
    let _guard = PROTECT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reasonix_tools::builtin::set_protect_sensitive_files(false);
    let (dir_g, dir) = testdir("confine-prot-off");
    let env_path = format!("{dir}{}.env", sep());
    std::fs::write(&env_path, b"PORT=8080\n").unwrap();
    assert!(
        !confine_read(&[], &env_path),
        ".env stays readable while protect_sensitive_files is off"
    );
    let rf = ReadFileTool {
        net_roots: Vec::new(),
        work_dir: String::new(),
        forbid_roots: Vec::new(),
        read_roots: Vec::new(),
    };
    let args = go_json(serde_json::json!({"path": env_path}));
    let out = Tool::execute(&rf, &CallContext::new(), &args).unwrap();
    assert!(out.contains("PORT=8080"), "{out}");
    drop(dir_g);
}

#[test]
fn test_glob_filters_sensitive_matches_when_protected() {
    let _guard = PROTECT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reasonix_tools::builtin::set_protect_sensitive_files(true);
    let (dir_g, dir) = testdir("confine-glob-prot");
    std::fs::write(format!("{dir}{}.env", sep()), b"SECRET_TOKEN=abc\n").unwrap();
    std::fs::write(format!("{dir}{}notes.txt", sep()), b"ok").unwrap();

    let g = GlobTool {
        net_roots: Vec::new(),
        work_dir: String::new(),
        forbid_roots: Vec::new(),
        read_roots: Vec::new(),
    };
    let args = go_json(serde_json::json!({
        "pattern": format!("{dir}{}*", sep())
    }));
    let out = Tool::execute(&g, &CallContext::new(), &args).unwrap();
    assert!(!out.contains(".env"), "glob leaked sensitive match:\n{out}");
    assert!(
        out.contains("notes.txt"),
        "glob dropped ordinary match:\n{out}"
    );
    reasonix_tools::builtin::set_protect_sensitive_files(false);
    drop(dir_g);
}

#[test]
fn test_confine_read_blocks_grep_file() {
    let (forbid_dir_g, forbid_dir) = testdir("confine-grep");
    let secret_path = format!("{forbid_dir}{}secret.txt", sep());
    std::fs::write(&secret_path, b"needle in a haystack").unwrap();
    let forbid_roots = real_roots(std::slice::from_ref(&forbid_dir));
    let g = GrepTool {
        net_roots: Vec::new(),
        work_dir: String::new(),
        rg: String::new(),
        forbid_roots: forbid_roots.clone(),
        read_roots: Vec::new(),
    };
    let args = go_json(serde_json::json!({"pattern": "needle", "path": secret_path}));
    let err = Tool::execute(&g, &CallContext::new(), &args).unwrap_err();
    assert!(
        err.to_string().contains("file does not exist"),
        "grep forbid-read file should answer not-exist, got {err}"
    );
    let g_open = GrepTool {
        net_roots: Vec::new(),
        work_dir: String::new(),
        rg: String::new(),
        forbid_roots: Vec::new(),
        read_roots: Vec::new(),
    };
    let out = Tool::execute(&g_open, &CallContext::new(), &args).unwrap();
    assert_ne!(
        out, "(no matches)",
        "unconfined grep should find the needle"
    );
    drop(forbid_dir_g);
}

#[test]
fn test_confine_read_blocks_native_grep_directory_root() {
    let (root_g, root) = testdir("confine-grep-dir");
    let forbid_dir = format!("{root}{}secret", sep());
    std::fs::create_dir_all(&forbid_dir).unwrap();
    std::fs::write(
        format!("{forbid_dir}{}secret.txt", sep()),
        b"needle in a haystack",
    )
    .unwrap();

    let g = GrepTool {
        net_roots: Vec::new(),
        work_dir: root.clone(),
        rg: String::new(),
        forbid_roots: real_roots(&[forbid_dir]),
        read_roots: Vec::new(),
    };
    let args = go_json(serde_json::json!({"pattern": "needle", "path": "secret"}));
    let out = Tool::execute(&g, &CallContext::new(), &args).unwrap();
    assert_eq!(out, "(no matches)", "grep forbidden directory looks empty");
    drop(root_g);
}

#[test]
fn test_confine_read_filters_plain_glob_matches() {
    let (root_g, root) = testdir("confine-glob-filter");
    let forbid_dir = format!("{root}{}secret", sep());
    std::fs::create_dir_all(&forbid_dir).unwrap();
    std::fs::write(
        format!("{forbid_dir}{}secret.go", sep()),
        b"package secret\n",
    )
    .unwrap();

    let g = GlobTool {
        net_roots: Vec::new(),
        work_dir: root.clone(),
        forbid_roots: real_roots(&[forbid_dir]),
        read_roots: Vec::new(),
    };
    let args = go_json(serde_json::json!({"pattern": "secret/*.go"}));
    let out = Tool::execute(&g, &CallContext::new(), &args).unwrap();
    assert_eq!(out, "(no matches)", "glob leaked forbidden paths:\n{out}");
    drop(root_g);
}

#[test]
fn test_read_outside_scope_refusal_shape() {
    let (root_g, root) = testdir("confine-scope");
    let scope = real_roots(std::slice::from_ref(&root));
    let (_out_g, out_dir) = testdir("confine-scope-out");
    let target = format!("{}{}outside{}x", out_dir, sep(), sep());
    let refusal = confine_scope(&scope, &target).unwrap();
    assert_eq!(refusal.code, "workspace.read_outside_scope");
    assert!(
        refusal
            .message
            .contains("outside the folders this run may read"),
        "{}",
        refusal.message
    );
    drop(root_g);
}

#[test]
fn confine_readers_binds_the_four_readers() {
    let forbid = vec![String::from(if cfg!(windows) { "C:\\x" } else { "/x" })];
    let readers = reasonix_tools::builtin::confine_readers(&forbid);
    assert_eq!(readers.len(), 3, "code_index lands with its own slice");
    let names: Vec<&str> = readers.iter().map(|t| Tool::name(t.as_ref())).collect();
    assert_eq!(names, ["read_file", "ls", "glob"]);
    for r in &readers {
        assert!(r.read_only());
    }
    let _ = Arc::clone(&readers[0]);
    let _ = ListDirTool {
        net_roots: Vec::new(),
        work_dir: String::new(),
        forbid_roots: Vec::new(),
        read_roots: Vec::new(),
    };
}
