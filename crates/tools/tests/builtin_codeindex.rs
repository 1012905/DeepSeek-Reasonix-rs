//! Oracle for codeindex.go: outline/search over a real temporary tree,
//! definition-candidate location by symbol name, kind filters, noise-dir
//! pruning, the forbid-read boundary, and the text-fallback degradation (the
//! tree-sitter grammar slice and the Go AST parser are unmigrated).

#![forbid(unsafe_code)]

use reasonix_tools::abi::{CallContext, Tool};
use reasonix_tools::builtin::CodeIndexTool;

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

fn write_rel(root: &str, rel: &str, body: &str) {
    let sep = reasonix_tools::builtin::pathgo::separator().to_string();
    let path = format!("{root}{}{}", sep, rel.replace('/', &sep));
    if let Some(parent) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, body).unwrap();
}

fn tool(work_dir: &str) -> CodeIndexTool {
    CodeIndexTool {
        net_roots: Vec::new(),
        work_dir: String::from(work_dir),
        forbid_roots: Vec::new(),
        read_roots: Vec::new(),
    }
}

fn run(t: &CodeIndexTool, args: serde_json::Value) -> String {
    Tool::execute(t, &CallContext::new(), &args).unwrap_or_else(|e| panic!("code_index: {e}"))
}

#[test]
fn test_code_index_search_finds_rust_symbol_definition_location() {
    let (_g, root) = testdir("codeindex-rust");
    write_rel(
        &root,
        "src/lib.rs",
        "pub struct Store {}\npub trait Repository {}\npub enum Mode { Fast }\npub fn load_store() {}\n",
    );
    let t = tool(&root);

    let out = run(
        &t,
        serde_json::json!({"action": "search", "query": "Store", "kind": "struct"}),
    );
    assert!(
        out.contains("src/lib.rs:1: struct Store"),
        "struct definition not located:\n{out}"
    );

    let out = run(
        &t,
        serde_json::json!({"action": "search", "query": "Repository", "kind": "trait"}),
    );
    assert!(
        out.contains("src/lib.rs:2: trait Repository"),
        "trait definition not located:\n{out}"
    );

    let out = run(
        &t,
        serde_json::json!({"action": "search", "query": "load_store", "kind": "fn"}),
    );
    assert!(
        out.contains("src/lib.rs:4: fn load_store"),
        "fn definition not located:\n{out}"
    );
}

#[test]
fn test_code_index_outline_typescript_and_skips_noise_dirs() {
    let (_g, root) = testdir("codeindex-ts");
    write_rel(
        &root,
        "src/app.ts",
        "export interface User {\n  name: string\n}\nexport class Client {}\nexport const loadUser = async () => {}\n",
    );
    write_rel(
        &root,
        "node_modules/dep/dep.ts",
        "export class Dependency {}\n",
    );
    let t = tool(&root);
    let out = run(&t, serde_json::json!({"action": "outline", "path": "."}));
    for want in [
        "src/app.ts:1: interface User",
        "src/app.ts:4: class Client",
        "src/app.ts:5: func loadUser",
    ] {
        assert!(out.contains(want), "outline missing {want}:\n{out}");
    }
    assert!(
        !out.contains("Dependency"),
        "node_modules must be pruned:\n{out}"
    );
}

#[test]
fn test_code_index_text_fallback_parses_go_symbols() {
    // No Go AST parser migrated: the text fallback (codeindex.go parseText)
    // locates the struct, function, method and const the AST would.
    let (_g, root) = testdir("codeindex-go");
    write_rel(
        &root,
        "service.go",
        "package demo\n\ntype Service struct{}\n\nfunc NewService() *Service { return &Service{} }\n\nfunc (s *Service) Handle() {}\n\nconst DefaultName = \"demo\"\n",
    );
    let t = tool(&root);
    let out = run(
        &t,
        serde_json::json!({"action": "search", "query": "Service"}),
    );
    for want in [
        "service.go:3: struct Service",
        "service.go:5: func NewService",
        "service.go:7: method Service.Handle",
    ] {
        assert!(out.contains(want), "go fallback missing {want}:\n{out}");
    }
    // The const does not match the "Service" query; outline lists it directly.
    let outline = run(&t, serde_json::json!({"action": "outline"}));
    assert!(
        outline.contains("service.go:9: const DefaultName"),
        "{outline}"
    );
}

#[test]
fn test_code_index_kind_filter_runs_before_the_limit() {
    let (_g, root) = testdir("codeindex-limit");
    write_rel(
        &root,
        "a.ts",
        "export const first = () => {}\nexport const second = () => {}\n",
    );
    write_rel(&root, "z.ts", "export interface Later {\n  id: string\n}\n");
    let t = tool(&root);
    let out = run(
        &t,
        serde_json::json!({"action": "outline", "kind": "interface", "limit": 1}),
    );
    assert!(out.contains("z.ts:1: interface Later"), "{out}");
    assert!(!out.contains("first") && !out.contains("second"), "{out}");
}

#[test]
fn test_code_index_search_requires_a_query() {
    let (_g, root) = testdir("codeindex-query");
    let t = tool(&root);
    let err = Tool::execute(
        &t,
        &CallContext::new(),
        &serde_json::json!({"action": "search"}),
    )
    .unwrap_err();
    assert!(err.to_string().contains("query is required"), "{err}");
}

#[test]
fn test_code_index_forbid_read_hides_forbidden_symbols() {
    let (_g, root) = testdir("codeindex-forbid");
    write_rel(
        &root,
        "allowed.go",
        "package demo\nfunc AllowedNeedle() {}\n",
    );
    write_rel(
        &root,
        "secret/secret.go",
        "package secret\nfunc SecretNeedle() {}\n",
    );
    let forbid = reasonix_tools::builtin::confine::real_roots(&[format!(
        "{}{}secret",
        root,
        reasonix_tools::builtin::pathgo::separator()
    )]);
    let t = CodeIndexTool {
        net_roots: Vec::new(),
        work_dir: root.clone(),
        forbid_roots: forbid,
        read_roots: Vec::new(),
    };
    let out = run(
        &t,
        serde_json::json!({"action": "search", "query": "Needle", "path": "."}),
    );
    assert!(
        out.contains("AllowedNeedle"),
        "allowed symbol dropped:\n{out}"
    );
    assert!(
        !out.contains("SecretNeedle") && !out.contains("secret.go"),
        "forbidden symbol leaked:\n{out}"
    );

    let out = run(
        &t,
        serde_json::json!({"action": "search", "query": "SecretNeedle", "path": "secret"}),
    );
    assert_eq!(
        out, "(no symbols)",
        "forbidden root must read empty:\n{out}"
    );
}

#[test]
fn test_code_index_ignores_oversized_files() {
    let (_g, root) = testdir("codeindex-large");
    let huge =
        "x".repeat(reasonix_tools::builtin::codeindex::CODE_INDEX_MAX_FILE_SIZE as usize + 1);
    write_rel(&root, "huge.ts", &huge);
    let t = tool(&root);
    let out = run(&t, serde_json::json!({"action": "outline"}));
    assert_eq!(out, "(no symbols)", "large file must be skipped:\n{out}");
}

#[test]
fn test_code_index_outline_of_a_plain_file_without_symbols() {
    let (_g, root) = testdir("codeindex-empty");
    write_rel(&root, "notes.txt", "just some prose\n");
    let t = tool(&root);
    let out = run(
        &t,
        serde_json::json!({"action": "outline", "path": "notes.txt"}),
    );
    assert_eq!(out, "(no symbols)");
}
