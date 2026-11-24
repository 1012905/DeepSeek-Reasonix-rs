//! The code_index tool (codeindex.go): a lightweight symbol outline and
//! definition-candidate search. The tree-sitter grammar slice
//! (codeindex_treesitter.go, JS/TS/Python/Rust queries) and the Go AST parser
//! are unmigrated; this implementation answers from the regex text fallback
//! (codeindex.go `parseText`) for every supported language — the degradation
//! path Go takes when a grammar is unavailable. Schema, output format, limits
//! and the read-side confinement mirror the Go oracle.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use super::confine::{confine_read, confine_scope};
use super::network::resolve_readable_path;
use super::pathgo;
use super::walk::{WalkConfine, WalkFlow, walk_dir};
use crate::abi::{CallContext, Tool, ToolResult};

const CODE_INDEX_DEFAULT_LIMIT: usize = 100;
const CODE_INDEX_MAX_LIMIT: usize = 200;
pub const CODE_INDEX_MAX_FILE_SIZE: u64 = 1 << 20;
const CODE_INDEX_MAX_FILES: usize = 2000;

/// codeindex.go `codeIndexSymbol`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CodeSymbol {
    name: String,
    kind: String,
    file: String,
    line: usize,
    parent: String,
    signature: String,
}

/// codeindex.go `codeIndexMatcher`: one line regex, the capture carrying the
/// symbol name, and an optional capture carrying the symbol kind.
#[derive(Clone)]
struct Matcher {
    kind: String,
    re: Regex,
    name_group: usize,
    kind_group: usize,
}

impl Matcher {
    fn match_line(&self, line: &str) -> Option<CodeSymbol> {
        let caps = self.re.captures(line)?;
        let name = caps.get(self.name_group)?.as_str().trim().to_owned();
        if name.is_empty() {
            return None;
        }
        let kind = if self.kind_group > 0
            && let Some(k) = caps.get(self.kind_group)
        {
            normalize_kind(k.as_str().trim())
        } else {
            self.kind.clone()
        };
        Some(CodeSymbol {
            name,
            kind,
            file: String::new(),
            line: 0,
            parent: String::new(),
            signature: line.trim().to_owned(),
        })
    }
}

fn normalize_kind(kind: &str) -> String {
    let kind = kind.trim().to_lowercase();
    if kind == "function" {
        String::from("func")
    } else {
        kind
    }
}

fn matcher(kind: &str, pattern: &str, name_group: usize, kind_group: usize) -> Matcher {
    Matcher {
        kind: String::from(kind),
        re: Regex::new(pattern).expect("codeindex matcher regex compiles"),
        name_group,
        kind_group,
    }
}

/// codeindex.go `codeIndexMatchers(ext)`: the per-extension text matchers.
/// The `.go` list replaces the unmigrated Go AST with the same text fallback.
fn matchers_for(ext: &str) -> Vec<Matcher> {
    match ext {
        ".py" => vec![
            matcher("class", r"^\s*class\s+([A-Za-z_][A-Za-z0-9_]*)\b", 1, 0),
            matcher(
                "func",
                r"^\s*(?:async\s+)?def\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(",
                1,
                0,
            ),
        ],
        ".js" | ".jsx" | ".ts" | ".tsx" => vec![
            matcher(
                "class",
                r"^\s*(?:export\s+)?(?:default\s+)?(?:abstract\s+)?class\s+([A-Za-z_$][A-Za-z0-9_$]*)\b",
                1,
                0,
            ),
            matcher(
                "func",
                r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*\(",
                1,
                0,
            ),
            matcher(
                "interface",
                r"^\s*(?:export\s+)?interface\s+([A-Za-z_$][A-Za-z0-9_$]*)\b",
                1,
                0,
            ),
            matcher(
                "type",
                r"^\s*(?:export\s+)?type\s+([A-Za-z_$][A-Za-z0-9_$]*)\b",
                1,
                0,
            ),
            matcher(
                "enum",
                r"^\s*(?:export\s+)?enum\s+([A-Za-z_$][A-Za-z0-9_$]*)\b",
                1,
                0,
            ),
            matcher(
                "func",
                r"^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*(?:async\s*)?(?:\([^)]*\)|[A-Za-z_$][A-Za-z0-9_$]*)\s*=>",
                1,
                0,
            ),
        ],
        ".java" | ".kt" | ".kts" | ".cs" => vec![
            matcher(
                "",
                r"^\s*(?:public|protected|private|abstract|final|static|sealed|data|\s)*\s*(class|interface|enum|record|object)\s+([A-Za-z_][A-Za-z0-9_]*)\b",
                2,
                1,
            ),
            matcher(
                "method",
                r"^\s*(?:public|protected|private|static|final|abstract|synchronized|native|\s)+[\w<>\[\], ?]+\s+([A-Za-z_][A-Za-z0-9_]*)\s*\([^;]*\)\s*(?:\{|$)",
                1,
                0,
            ),
        ],
        ".rs" => vec![matcher(
            "",
            r"^\s*(?:pub(?:\([^)]*\))?\s+)?(fn|struct|enum|trait)\s+([A-Za-z_][A-Za-z0-9_]*)\b",
            2,
            1,
        )],
        ".c" | ".cc" | ".cpp" | ".h" | ".hpp" => vec![matcher(
            "func",
            r"^\s*(?:[\w:*&<>\[\],]+\s+)+([A-Za-z_][A-Za-z0-9_]*)\s*\([^;]*\)\s*(?:\{|$)",
            1,
            0,
        )],
        ".go" => vec![
            matcher(
                "struct",
                r"^\s*type\s+([A-Za-z_][A-Za-z0-9_]*)\s+struct\b",
                1,
                0,
            ),
            matcher(
                "interface",
                r"^\s*type\s+([A-Za-z_][A-Za-z0-9_]*)\s+interface\b",
                1,
                0,
            ),
            matcher("type", r"^\s*type\s+([A-Za-z_][A-Za-z0-9_]*)\b", 1, 0),
            matcher("func", r"^\s*func\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(", 1, 0),
            matcher(
                "method",
                r"^\s*func\s*\(\s*(?:\w+\s+\*)?([A-Za-z_][A-Za-z0-9_]*)\s*\)\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(",
                2,
                0,
            ),
            matcher("const", r"^\s*const\s+([A-Za-z_][A-Za-z0-9_]*)", 1, 0),
            matcher("var", r"^\s*var\s+([A-Za-z_][A-Za-z0-9_]*)", 1, 0),
        ],
        _ => Vec::new(),
    }
}

/// Compiled matchers cached per extension, mirroring Go's package-level
/// regexes so a large tree does not recompile a matcher per file.
fn cached_matchers(ext: &str) -> Vec<Matcher> {
    use std::sync::Mutex;
    static CACHE: OnceLock<Mutex<HashMap<String, Vec<Matcher>>>> = OnceLock::new();
    let map = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(m) = map.lock().unwrap().get(ext) {
        return m.clone();
    }
    let list = matchers_for(ext);
    map.lock().unwrap().insert(String::from(ext), list.clone());
    list
}

fn extension(path: &str) -> String {
    let name = pathgo::to_slash(path);
    let base = name.rsplit('/').next().unwrap_or(&name);
    match base.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() => format!(".{ext}").to_lowercase(),
        _ => String::new(),
    }
}

/// codeindex.go `supportedCodeIndexFile`.
fn supported_file(path: &str) -> bool {
    matches!(
        extension(path).as_str(),
        ".go"
            | ".js"
            | ".jsx"
            | ".ts"
            | ".tsx"
            | ".py"
            | ".java"
            | ".kt"
            | ".kts"
            | ".cs"
            | ".rs"
            | ".c"
            | ".cc"
            | ".cpp"
            | ".h"
            | ".hpp"
    )
}

/// codeindex.go `skipCodeIndexDir`.
fn skip_dir(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | "node_modules"
            | "vendor"
            | "__pycache__"
            | ".idea"
            | ".vscode"
            | ".next"
            | "dist"
            | "build"
            | "target"
            | "coverage"
    )
}

/// The code_index tool. Scopes mirror Go's `codeIndex` fields.
pub struct CodeIndexTool {
    pub net_roots: Vec<String>,
    pub work_dir: String,
    pub forbid_roots: Vec<String>,
    pub read_roots: Vec<String>,
}

impl CodeIndexTool {
    fn execute_inner(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if action != "outline" && action != "search" {
            return Err(Box::new(std::io::Error::other(
                "action must be outline or search",
            )));
        }
        let mut path = args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .to_owned();
        if path.is_empty() {
            path = String::from(".");
        }
        let query = args.get("query").and_then(Value::as_str).unwrap_or("");
        let kind = args.get("kind").and_then(Value::as_str).unwrap_or("");
        let limit = args.get("limit").and_then(Value::as_i64).unwrap_or(0);
        let mut limit = if limit <= 0 {
            CODE_INDEX_DEFAULT_LIMIT
        } else {
            limit as usize
        };
        if limit > CODE_INDEX_MAX_LIMIT {
            limit = CODE_INDEX_MAX_LIMIT;
        }
        if action == "search" && query.trim().is_empty() {
            return Err(Box::new(std::io::Error::other(
                "query is required for action=search",
            )));
        }

        let rp = resolve_readable_path(&self.work_dir, &path);
        if let Some(r) = rp.refuse_network(&self.net_roots) {
            return Err(Box::new(r));
        }
        if let Some(r) = confine_scope(&self.read_roots, &rp.path) {
            return Err(Box::new(r));
        }
        if confine_read(&self.forbid_roots, &rp.path) {
            return Ok(String::from("(no symbols)"));
        }

        let info = std::fs::metadata(&rp.path)
            .map_err(|e| std::io::Error::other(format!("code_index {}: {e}", rp.path)))?;

        let mut files: Vec<String> = Vec::new();
        if info.is_dir() {
            let confine = WalkConfine::new(&self.forbid_roots, &self.read_roots, &rp.path);
            let root = rp.path.clone();
            let _ = walk_dir(&root, &mut |e| {
                if e.err.is_some() {
                    return Ok(WalkFlow::Continue);
                }
                if e.is_dir {
                    if confine.blocked(&e.path, Some((&e.name, e.is_dir, e.is_regular))) {
                        return Ok(WalkFlow::SkipDir);
                    }
                    if e.path != root && skip_dir(&e.name) {
                        return Ok(WalkFlow::SkipDir);
                    }
                    return Ok(WalkFlow::Continue);
                }
                if !confine.blocked(&e.path, Some((&e.name, e.is_dir, e.is_regular)))
                    && supported_file(&e.path)
                {
                    files.push(e.path.clone());
                    if files.len() >= CODE_INDEX_MAX_FILES {
                        return Ok(WalkFlow::SkipAll);
                    }
                }
                Ok(WalkFlow::Continue)
            });
        } else if supported_file(&rp.path) {
            files.push(rp.path.clone());
        }
        files.sort();

        let mut symbols: Vec<CodeSymbol> = Vec::new();
        for file in files {
            if let Ok(meta) = std::fs::metadata(&file)
                && meta.len() <= CODE_INDEX_MAX_FILE_SIZE
                && let Ok(found) = self.parse_file(&file)
            {
                symbols.extend(found);
            }
        }

        symbols.retain(|s| {
            let kind_ok = kind.trim().is_empty() || s.kind.eq_ignore_ascii_case(kind.trim());
            if !kind_ok {
                return false;
            }
            if query.trim().is_empty() {
                return true;
            }
            let hay = format!("{} {} {}", s.name, s.parent, s.signature).to_lowercase();
            hay.contains(&query.trim().to_lowercase())
        });

        symbols.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then(a.line.cmp(&b.line))
                .then(a.name.cmp(&b.name))
        });
        let truncated = symbols.len() > limit;
        if truncated {
            symbols.truncate(limit);
        }
        Ok(format_symbols(&symbols, truncated))
    }

    fn parse_file(&self, path: &str) -> std::io::Result<Vec<CodeSymbol>> {
        let ext = extension(path);
        let matchers = cached_matchers(&ext);
        if matchers.is_empty() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(path)?;
        let mut out = Vec::new();
        for (line_no, line) in text.lines().enumerate() {
            for m in &matchers {
                if let Some(mut sym) = m.match_line(line) {
                    sym.file = self.display_path(path);
                    sym.line = line_no + 1;
                    if m.kind == "method" && ext == ".go" {
                        // The Go method matcher captures the receiver type in
                        // group 1; the name is group 2.
                        if let Some(caps) = m.re.captures(line)
                            && let Some(parent) = caps.get(1)
                        {
                            sym.parent = parent.as_str().trim().to_owned();
                        }
                    }
                    out.push(sym);
                    break;
                }
            }
        }
        Ok(out)
    }

    /// codeindex.go `displayPath`: a path under the work dir renders relative,
    /// slash-normalized; otherwise the absolute path.
    fn display_path(&self, path: &str) -> String {
        if !self.work_dir.is_empty()
            && let Ok(rel) = pathgo::rel(&self.work_dir, path)
            && rel != "."
            && !rel.starts_with("..")
        {
            return pathgo::to_slash(&rel);
        }
        pathgo::to_slash(path)
    }
}

/// codeindex.go `formatCodeSymbols`.
fn format_symbols(symbols: &[CodeSymbol], truncated: bool) -> String {
    if symbols.is_empty() {
        return String::from("(no symbols)");
    }
    let mut b = String::new();
    for s in symbols {
        let name = if s.parent.is_empty() {
            s.name.clone()
        } else {
            format!("{}.{}", s.parent, s.name)
        };
        if s.signature.is_empty() {
            b.push_str(&format!("{}:{}: {} {}\n", s.file, s.line, s.kind, name));
        } else {
            b.push_str(&format!(
                "{}:{}: {} {} — {}\n",
                s.file, s.line, s.kind, name, s.signature
            ));
        }
    }
    if truncated {
        b.push_str("... (truncated; narrow path/query/kind or raise limit)\n");
    }
    b.trim_end_matches('\n').to_owned()
}

impl Tool for CodeIndexTool {
    fn name(&self) -> &str {
        "code_index"
    }

    fn description(&self) -> &str {
        "Lightweight built-in code symbol index. Prefer lsp_* for language semantics and installed code graph MCP tools for call graph, impact, and architecture relationships; use this as the local fallback for file outlines and symbol definition candidates, then verify with read_file or grep."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["outline", "search"], "description": "outline lists symbols under path; search finds symbol definition candidates by name."},
                "path": {"type": "string", "description": "File or directory path to inspect (default \".\")."},
                "query": {"type": "string", "description": "Symbol name or substring for action=search."},
                "kind": {"type": "string", "description": "Optional symbol kind filter, such as func, method, class, type, interface, const, var, struct, enum, trait."},
                "limit": {"type": "integer", "description": "Maximum symbols to return (default 100, max 200).", "minimum": 1}
            },
            "required": ["action"]
        })
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        self.execute_inner(ctx, args)
    }

    fn read_only(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
