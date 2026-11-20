//! Oracles for glob.go (glob_root_test.go + walk_confine_test.go's glob
//! case) and ls.go (directory listing shapes). The Windows-only drive-root
//! case runs only where a per-drive current directory exists.

#![forbid(unsafe_code)]

use reasonix_tools::abi::{CallContext, Tool};
use reasonix_tools::builtin::{GlobTool, ListDirTool, glob_walk_root};

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

fn sep() -> String {
    reasonix_tools::builtin::pathgo::separator().to_string()
}

fn glob_tool(work_dir: &str) -> GlobTool {
    GlobTool {
        net_roots: Vec::new(),
        work_dir: String::from(work_dir),
        forbid_roots: Vec::new(),
        read_roots: Vec::new(),
    }
}

fn ls_tool(work_dir: &str) -> ListDirTool {
    ListDirTool {
        net_roots: Vec::new(),
        work_dir: String::from(work_dir),
        forbid_roots: Vec::new(),
        read_roots: Vec::new(),
    }
}

#[test]
fn test_a_glob_rooted_at_a_drive_walks_the_drive_root() {
    if !cfg!(windows) {
        println!("only Windows has a per-drive current directory to be relative to");
        return;
    }
    let (root, rel) = glob_walk_root("D:\\**\\*xray*");
    assert_eq!(
        root, "D:\\",
        "a bare volume names the drive's current directory"
    );
    assert_eq!(rel, "**/*xray*");
    assert!(
        reasonix_tools::builtin::pathgo::is_abs(&root),
        "root {root:?} is not absolute, so every match under it is drive-relative"
    );
    let (root2, rel2) = glob_walk_root("D:\\");
    assert_eq!(root2, "D:\\");
    assert_eq!(rel2, "**");
}

#[test]
fn test_a_glob_root_that_names_a_directory_is_untouched() {
    let (_g, dir) = testdir("glob-root-dir");
    let (root, rel) = glob_walk_root(&format!("{}{}**{}*.go", dir, sep(), sep()));
    let dir_c = std::fs::canonicalize(&dir)
        .unwrap()
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_owned();
    assert_eq!(root, dir_c);
    assert_eq!(rel, "**/*.go");
}

#[test]
fn test_glob_walk_keeps_confinement() {
    let (_g, root) = testdir("glob-confine");
    for p in ["keep/a.txt", "secret/b.txt"] {
        let full = format!("{root}{}{}", sep(), p.replace('/', &sep()));
        if let Some(parent) = std::path::Path::new(&full).parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&full, b"x").unwrap();
    }
    #[cfg(unix)]
    {
        let _ = std::os::unix::fs::symlink(
            format!("{root}{}secret{}b.txt", sep(), sep()),
            format!("{root}{}keep{}b-link.txt", sep(), sep()),
        );
    }
    let g = GlobTool {
        net_roots: Vec::new(),
        work_dir: root.clone(),
        forbid_roots: reasonix_tools::builtin::real_roots(&[format!("{root}{}secret", sep())]),
        read_roots: Vec::new(),
    };
    let args = serde_json::json!({"pattern": "**/*.txt"});
    let out = Tool::execute(&g, &CallContext::new(), &args).unwrap();
    assert!(out.contains("a.txt"), "{out}");
    assert!(!out.contains("b.txt"), "{out}");
    #[cfg(unix)]
    assert!(!out.contains("b-link.txt"), "{out}");
}

#[test]
fn test_glob_simple_filename_falls_back_to_recursive() {
    let (_g, root) = testdir("glob-fallback");
    std::fs::create_dir_all(format!("{root}{}deep{}nest", sep(), sep())).unwrap();
    std::fs::write(
        format!("{root}{}deep{}nest{}target.go", sep(), sep(), sep()),
        b"",
    )
    .unwrap();
    std::fs::write(format!("{root}{}top.md", sep()), b"").unwrap();
    let g = glob_tool(&root);
    let args = serde_json::json!({"pattern": "target.go"});
    let out = Tool::execute(&g, &CallContext::new(), &args).unwrap();
    assert!(out.contains("target.go"), "{out}");
    let args = serde_json::json!({"pattern": "*.md"});
    let out = Tool::execute(&g, &CallContext::new(), &args).unwrap();
    assert!(out.contains("top.md"), "{out}");
}

#[test]
fn test_glob_no_matches_constant() {
    let (_g, root) = testdir("glob-none");
    let g = glob_tool(&root);
    let args = serde_json::json!({"pattern": "*.zzz-absent"});
    let out = Tool::execute(&g, &CallContext::new(), &args).unwrap();
    assert_eq!(out, "(no matches)");
}

#[test]
fn test_glob_doublestar_pattern() {
    let (_g, root) = testdir("glob-dstar");
    std::fs::create_dir_all(format!("{root}{}internal{}deep", sep(), sep())).unwrap();
    std::fs::write(format!("{root}{}a.go", sep()), b"").unwrap();
    std::fs::write(
        format!("{root}{}internal{}deep{}b_test.go", sep(), sep(), sep()),
        b"",
    )
    .unwrap();
    let g = glob_tool(&root);
    let args = serde_json::json!({"pattern": "**/*_test.go"});
    let out = Tool::execute(&g, &CallContext::new(), &args).unwrap();
    assert!(out.contains("b_test.go"), "{out}");
    assert!(!out.contains("a.go"), "{out}");
}

#[test]
fn test_glob_pattern_required() {
    let g = glob_tool("");
    let args = serde_json::json!({"pattern": ""});
    let err = Tool::execute(&g, &CallContext::new(), &args)
        .unwrap_err()
        .to_string();
    assert!(err.contains("pattern is required"), "{err}");
}

#[test]
fn test_ls_flat_listing() {
    let (_g, root) = testdir("ls-flat");
    std::fs::create_dir_all(format!("{root}{}sub", sep())).unwrap();
    std::fs::write(format!("{root}{}a.txt", sep()), b"12345").unwrap();
    std::fs::write(format!("{root}{}b.txt", sep()), b"x").unwrap();
    let ls = ls_tool(&root);
    let args = serde_json::json!({"path": root});
    let out = Tool::execute(&ls, &CallContext::new(), &args).unwrap();
    assert!(out.contains("sub/"), "{out}");
    assert!(out.contains("a.txt\t5"), "{out}");
    assert!(out.contains("b.txt\t1"), "{out}");
}

#[test]
fn test_ls_empty_directory() {
    let (_g, root) = testdir("ls-empty");
    let ls = ls_tool(&root);
    let args = serde_json::json!({"path": root});
    let out = Tool::execute(&ls, &CallContext::new(), &args).unwrap();
    assert_eq!(out, "(empty directory)");
}

#[test]
fn test_ls_recursive_skips_noise_dirs() {
    let (_g, root) = testdir("ls-rec");
    std::fs::create_dir_all(format!("{root}{}src{}deep", sep(), sep())).unwrap();
    std::fs::create_dir_all(format!("{root}{}node_modules{}pkg", sep(), sep())).unwrap();
    std::fs::create_dir_all(format!("{root}{}.git", sep())).unwrap();
    std::fs::write(
        format!("{root}{}src{}deep{}f.txt", sep(), sep(), sep()),
        b"x",
    )
    .unwrap();
    std::fs::write(
        format!("{root}{}node_modules{}pkg{}g.txt", sep(), sep(), sep()),
        b"x",
    )
    .unwrap();
    let ls = ls_tool(&root);
    let args = serde_json::json!({"path": root, "recursive": true});
    let out = Tool::execute(&ls, &CallContext::new(), &args).unwrap();
    assert!(out.contains("src/"), "{out}");
    assert!(
        out.contains("src/deep/f.txt") || out.contains("src\\deep\\f.txt"),
        "{out}"
    );
    assert!(!out.contains("node_modules"), "{out}");
    assert!(!out.contains(".git/"), "{out}");
}

#[test]
fn test_ls_recursive_empty_tree_message() {
    let (_g, root) = testdir("ls-rec-empty");
    std::fs::create_dir_all(format!("{root}{}only-dirs{}more", sep(), sep())).unwrap();
    let ls = ls_tool(&root);
    let args = serde_json::json!({"path": root, "recursive": true});
    let out = Tool::execute(&ls, &CallContext::new(), &args).unwrap();
    assert!(out.contains("only-dirs/"), "{out}");
    assert!(out.contains("more/"), "{out}");
}

#[test]
fn test_ls_default_path_is_dot() {
    let (_g, root) = testdir("ls-dot");
    std::fs::write(format!("{root}{}here.txt", sep()), b"hi").unwrap();
    // A defaulted "." targets the workspace root: run the tool bound to the
    // test dir and pass no path.
    let ls = ls_tool(&root);
    let args = serde_json::json!({});
    let out = Tool::execute(&ls, &CallContext::new(), &args).unwrap();
    assert!(out.contains("here.txt"), "{out}");
}
