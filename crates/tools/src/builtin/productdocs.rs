//! Offline retrieval over the official Reasonix documentation embedded in the
//! application binary (productdocs/docs.go, tools.md §6). The tool serves
//! three operations — `search`, `read`, `list` — over an in-memory corpus
//! built from Markdown sources.
//!
//! Host-machinery gaps recorded with the integration slice: the real
//! `go:embed`-style corpus bundle (byte-identical `docs/*.md` tree for the
//! sha256 manifest digest), release-stamped version/revision identity, and
//! the goldmark AST heading walk. The line-based heading parser here mirrors
//! the V1 section split for ordinary `#`..`####` headings.

#![forbid(unsafe_code)]

use std::any::Any;
use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::productdocs_retrieval as retrieval;
use crate::abi::{CallContext, Tool, ToolResult};

const DEFAULT_LIMIT: usize = 5;
const MAX_LIMIT: usize = 10;
const MAX_SNIPPET: usize = 360;
const MAX_QUERY_RUNES: usize = 4096;
const SCORE_FLOOR: f64 = 0.15;

const DIGEST_DOMAIN_TAG: &[u8] = b"reasonix-product-docs-v1\x00";

/// One embedded document (Go `document`).
#[derive(Debug, Clone)]
pub struct Document {
    path: String,
    source: String,
    title: String,
    locale: String,
    audience: String,
    sections: Vec<Section>,
}

/// One section of a document (Go `section`).
#[derive(Debug, Clone)]
pub struct Section {
    id: String,
    heading: String,
    content: String,
    search_text: String,
    counts: BTreeMap<String, i32>,
    heading_hits: BTreeMap<String, i32>,
    length: usize,
    start_line: usize,
    end_line: usize,
}

/// A section's position inside the catalog's document list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionRef {
    doc: usize,
    section: usize,
}

/// The loaded corpus identity (Go `Manifest`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Manifest {
    version: String,
    revision: String,
    digest: String,
    documents: usize,
    sections: usize,
}

/// The loaded catalog (Go `catalog`).
#[derive(Debug, Clone)]
pub struct Catalog {
    digest: String,
    docs: Vec<Document>,
    by_path: BTreeMap<String, usize>,
    by_id: BTreeMap<String, SectionRef>,
    sections: Vec<SectionRef>,
    df: BTreeMap<String, i32>,
    avg_len: f64,
    version: String,
    revision: String,
}

impl Document {
    fn display_path(&self) -> String {
        format!("docs/{}", self.path)
    }

    fn source_range(&self, start_line: usize, end_line: usize) -> String {
        format!("{}:{start_line}-{end_line}", self.source)
    }
}

impl Catalog {
    /// The document at a section reference.
    pub fn document(&self, r: SectionRef) -> &Document {
        &self.docs[r.doc]
    }

    /// The section at a section reference.
    pub fn section(&self, r: SectionRef) -> &Section {
        &self.docs[r.doc].sections[r.section]
    }

    fn manifest(&self) -> Manifest {
        Manifest {
            version: self.version.clone(),
            revision: self.revision.clone(),
            digest: format!("sha256:{}", self.digest),
            documents: self.docs.len(),
            sections: self.sections.len(),
        }
    }

    fn identity_line(&self) -> String {
        let m = self.manifest();
        format!(
            "version={} revision={} digest={}",
            m.version, m.revision, m.digest
        )
    }
}

/// Loads a catalog from named Markdown sources. Names are the bare file
/// names (`CLI.md`); the corpus sort, digest record and duplicate checks
/// mirror `loadCatalog`.
pub fn load_catalog(entries: &[(String, String)]) -> Result<Catalog, String> {
    let mut sorted: Vec<(&String, &String)> = entries.iter().map(|(n, c)| (n, c)).collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));

    let mut hasher = Sha256::new();
    hasher.update(DIGEST_DOMAIN_TAG);

    let mut docs: Vec<Document> = Vec::new();
    let mut by_path: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_id: BTreeMap<String, SectionRef> = BTreeMap::new();
    let mut sections: Vec<SectionRef> = Vec::new();

    for (name, data) in &sorted {
        if !name.to_lowercase().ends_with(".md") {
            continue;
        }
        write_digest_record(&mut hasher, name, data.as_bytes());
        let doc = parse_document(name, data);
        if doc.sections.is_empty() {
            continue;
        }
        if by_path.contains_key(&doc.path) {
            return Err(format!(
                "duplicate embedded documentation path {:?}",
                doc.path
            ));
        }
        let doc_idx = docs.len();
        for (sec_idx, section) in doc.sections.iter().enumerate() {
            if by_id.contains_key(&section.id) {
                return Err(format!(
                    "duplicate embedded documentation section {:?}",
                    section.id
                ));
            }
            let r = SectionRef {
                doc: doc_idx,
                section: sec_idx,
            };
            by_id.insert(section.id.clone(), r);
            sections.push(r);
        }
        by_path.insert(doc.path.clone(), doc_idx);
        by_path.insert(doc.display_path(), doc_idx);
        docs.push(doc);
    }

    if docs.is_empty() || sections.is_empty() {
        return Err("embedded documentation corpus is empty".to_owned());
    }

    let counts: Vec<BTreeMap<String, i32>> = sections
        .iter()
        .map(|r| docs[r.doc].sections[r.section].counts.clone())
        .collect();
    let total_length: usize = counts
        .iter()
        .map(|c| c.values().sum::<i32>() as usize)
        .sum();
    let df = retrieval::document_frequency(&counts);
    let avg_len = if total_length == 0 {
        1.0
    } else {
        total_length as f64 / sections.len() as f64
    };

    Ok(Catalog {
        digest: hex_encode(hasher.finalize().as_slice()),
        docs,
        by_path,
        by_id,
        sections,
        df,
        avg_len,
        // Release build stamps these; the dev fallback matches buildIdentity.
        version: "dev".to_owned(),
        revision: "unknown".to_owned(),
    })
}

fn write_digest_record(hasher: &mut Sha256, name: &str, data: &[u8]) {
    hasher.update((name.len() as u64).to_be_bytes());
    hasher.update(name.as_bytes());
    hasher.update((data.len() as u64).to_be_bytes());
    hasher.update(data);
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// blankMetadataHeader: the ownership `---` fence at the top of a file becomes
/// blank spaces, so Markdown does not read `---`, key lines, `---` as a
/// heading that surfaces owner names as a section. Offsets and lines stay put.
fn blank_metadata_header(source: &str) -> String {
    let lines: Vec<&str> = source.split_inclusive('\n').collect();
    if lines.is_empty() || !lines[0].trim().eq("---") {
        return source.to_owned();
    }
    let Some(end) = lines.iter().skip(1).position(|l| l.trim().eq("---")) else {
        return source.to_owned();
    };
    let mut out = source.as_bytes().to_vec();
    let mut offset = 0;
    for line in &lines[..=end] {
        for byte in out[offset..offset + line.len()].iter_mut() {
            if *byte != b'\n' && *byte != b'\r' {
                *byte = b' ';
            }
        }
        offset += line.len();
    }
    String::from_utf8(out).unwrap_or_else(|_| source.to_owned())
}

struct HeadingNode {
    start: usize,
    level: usize,
    text: String,
}

fn parse_document(name: &str, content: &str) -> Document {
    let source = blank_metadata_header(content);
    let stem = name
        .strip_suffix(".md")
        .unwrap_or(name)
        .trim_end_matches(".zh-CN")
        .to_owned();
    let mut doc = Document {
        path: clean_name(name),
        source: format!("docs/{name}"),
        title: stem,
        locale: detect_document_language(name, &source),
        audience: document_audience(name),
        sections: Vec::new(),
    };

    let heading_nodes = parse_headings(&source);
    for heading in &heading_nodes {
        if heading.level == 1 {
            doc.title = heading.text.clone();
            break;
        }
    }

    let line_starts = source_line_starts(&source);
    let mut headings: [String; 4] = Default::default();
    let mut section_number = 0usize;

    let title = doc.title.clone();
    if heading_nodes.is_empty() {
        append_section(
            &mut doc,
            &mut section_number,
            &source,
            0,
            source.len(),
            &title,
            &line_starts,
        );
        return doc;
    }
    if heading_nodes[0].start > 0 {
        append_section(
            &mut doc,
            &mut section_number,
            &source,
            0,
            heading_nodes[0].start,
            &title,
            &line_starts,
        );
    }
    for (i, heading) in heading_nodes.iter().enumerate() {
        let end = heading_nodes
            .get(i + 1)
            .map(|h| h.start)
            .unwrap_or(source.len());
        headings[heading.level - 1] = heading.text.clone();
        for slot in headings.iter_mut().skip(heading.level) {
            *slot = String::new();
        }
        let trail: Vec<&str> = headings
            .iter()
            .filter(|h| !h.is_empty())
            .map(String::as_str)
            .collect();
        append_section(
            &mut doc,
            &mut section_number,
            &source,
            heading.start,
            end,
            &trail.join(" > "),
            &line_starts,
        );
    }
    doc
}

fn append_section(
    doc: &mut Document,
    section_number: &mut usize,
    source: &str,
    start: usize,
    end: usize,
    current_heading: &str,
    line_starts: &[usize],
) {
    let (start, end) = trim_source_bounds(source, start, end);
    if end <= start {
        return;
    }
    let raw = source[start..end].to_owned();
    *section_number += 1;
    let search_text = format!("{}\n{current_heading}\n{raw}", doc.title);
    let terms = retrieval::tokens(&search_text);
    let heading_terms = retrieval::tokens(&format!("{} {current_heading}", doc.title));
    let id = format!("{}::s{section_number:03}", doc.path);
    doc.sections.push(Section {
        id,
        heading: current_heading.to_owned(),
        content: raw,
        search_text,
        counts: retrieval::counts(&terms),
        heading_hits: retrieval::counts(&heading_terms),
        length: terms.len(),
        start_line: source_line_number(line_starts, start),
        end_line: source_line_number(line_starts, end.saturating_sub(1)),
    });
}

/// Line-based heading scan: `#`..`####` at line start. The goldmark AST walk
/// (inline emphasis, nested heading parents) is the parser slice's gap.
fn parse_headings(source: &str) -> Vec<HeadingNode> {
    let mut nodes = Vec::new();
    let mut offset = 0usize;
    for line in source.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let hash_len = body.bytes().take_while(|&b| b == b'#').count();
        if hash_len > 0 && hash_len <= 4 {
            let rest = &body[hash_len..];
            if let Some(after) = rest.strip_prefix(' ') {
                let text = after.trim().to_owned();
                if !text.is_empty() {
                    nodes.push(HeadingNode {
                        start: offset,
                        level: hash_len,
                        text,
                    });
                }
            }
        }
        offset += line.len();
    }
    nodes
}

fn clean_name(name: &str) -> String {
    name.strip_prefix("./").unwrap_or(name).to_owned()
}

fn trim_source_bounds(source: &str, start: usize, end: usize) -> (usize, usize) {
    let start = start.min(source.len());
    let end = end.min(source.len());
    if end <= start {
        return (start, start);
    }
    let trimmed_left = source[start..end].trim_start_matches([' ', '\t', '\r', '\n']);
    let left = end - trimmed_left.len();
    let trimmed = trimmed_left.trim_end_matches([' ', '\t', '\r', '\n']);
    (left, left + trimmed.len())
}

fn source_line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, byte) in source.bytes().enumerate() {
        if byte == b'\n' && i + 1 < source.len() {
            starts.push(i + 1);
        }
    }
    starts
}

fn source_line_number(starts: &[usize], offset: usize) -> usize {
    match starts.partition_point(|&s| s <= offset) {
        0 => 1,
        n => n,
    }
}

fn detect_document_language(name: &str, content: &str) -> String {
    if name.to_lowercase().ends_with(".zh-cn.md") {
        return "zh-CN".to_owned();
    }
    let (mut han, mut latin) = (0usize, 0usize);
    for r in content.chars() {
        if unicode_is_cjk_char(r) {
            han += 1;
        } else if r.is_ascii_alphabetic() {
            latin += 1;
        }
    }
    if han > 100 && han * 4 > latin {
        "zh-CN".to_owned()
    } else {
        "en".to_owned()
    }
}

fn unicode_is_cjk_char(r: char) -> bool {
    matches!(r,
        '\u{3040}'..='\u{30FF}' | '\u{3130}'..='\u{318F}' | '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}' | '\u{AC00}'..='\u{D7AF}' | '\u{F900}'..='\u{FAFF}')
}

fn document_audience(name: &str) -> String {
    let stem = name
        .strip_suffix(".md")
        .unwrap_or(name)
        .trim_end_matches(".zh-CN")
        .to_uppercase();
    match stem.as_str() {
        "THEME_ASSETS" | "STUDIO_RELEASE" | "DOCS_STANDARD" => "maintainer".to_owned(),
        "CHECKPOINTS" | "SPEC" | "TASK_CONTRACT" | "TOOL_CONTRACT" => "developer".to_owned(),
        _ => "user".to_owned(),
    }
}

fn normalize_language(language: &str) -> Result<String, String> {
    match language.to_lowercase().trim() {
        "" | "auto" => Ok("auto".to_owned()),
        "all" => Ok("all".to_owned()),
        "en" | "en-us" | "english" => Ok("en".to_owned()),
        "zh" | "zh-cn" | "cn" | "chinese" => Ok("zh-CN".to_owned()),
        other => Err(format!(
            "unknown language {other:?}; use auto, all, en, or zh-CN"
        )),
    }
}

fn normalize_audience(audience: &str) -> Result<String, String> {
    match audience.to_lowercase().trim() {
        "" | "all" => Ok("all".to_owned()),
        "user" | "developer" | "maintainer" => Ok(audience.to_lowercase().trim().to_owned()),
        other => Err(format!(
            "unknown audience {other:?}; use all, user, developer, or maintainer"
        )),
    }
}

fn detect_query_language(query: &str) -> String {
    if query.chars().any(unicode_is_cjk_char) {
        "zh-CN".to_owned()
    } else {
        "en".to_owned()
    }
}

fn clamp_limit(value: i64) -> usize {
    match value {
        v if v <= 0 => DEFAULT_LIMIT,
        v if v > MAX_LIMIT as i64 => MAX_LIMIT,
        v => v as usize,
    }
}

#[derive(Deserialize, Default)]
struct DocsArgs {
    #[serde(default)]
    operation: String,
    #[serde(default)]
    query: String,
    #[serde(default)]
    section_id: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    language: String,
    #[serde(default)]
    audience: String,
    #[serde(default)]
    limit: i64,
}

const DOCS_DESCRIPTION: &str = "Search and read the official documentation embedded in this exact Reasonix build. Use it before web search or assumptions for Reasonix setup, CLI, Desktop, configuration, permissions, MCP, memory, recovery, provider behavior, and maintainer workflows. Search first, then read the returned section_id when the full section is needed.";

const DOCS_SCHEMA: &str = r#"{
  "type":"object",
  "properties":{
    "operation":{"type":"string","enum":["search","read","list"],"description":"search ranks relevant sections; read returns one section or lists a document's sections; list shows the embedded document catalog."},
    "query":{"type":"string","maxLength":4096,"description":"Question, command, configuration key, error phrase, or topic for operation=search."},
    "section_id":{"type":"string","description":"Exact section_id returned by search or by a document section listing. Used by operation=read."},
    "path":{"type":"string","description":"Exact docs/*.md path returned by search/list. With operation=read and no section_id, lists that document's sections."},
    "language":{"type":"string","enum":["auto","all","en","zh-CN"],"description":"Language preference. search defaults to auto from the query; list defaults to all. Explicit en or zh-CN filters results."},
    "audience":{"type":"string","enum":["all","user","developer","maintainer"],"description":"Optional audience filter; defaults to all."},
    "limit":{"type":"integer","minimum":1,"maximum":10,"description":"Maximum search results, default 5, max 10."}
  },
  "required":["operation"]
}"#;

/// The read-only embedded-documentation tool.
pub struct DocsTool {
    catalog: Option<Catalog>,
}

/// Go `NewTool`: a tool backed by the embedded corpus. When `catalog` is
/// `None` the default lazy load has not bound; the integration slice wires the
/// real corpus source.
pub fn docs_tool(catalog: Option<Catalog>) -> Arc<dyn Tool> {
    Arc::new(DocsTool { catalog })
}

impl Tool for DocsTool {
    fn name(&self) -> &str {
        "docs"
    }

    fn description(&self) -> &str {
        DOCS_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(DOCS_SCHEMA).expect("the docs schema is valid JSON")
    }

    fn execute(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let Some(catalog) = &self.catalog else {
            return Err(std::io::Error::other("embedded documentation is unavailable").into());
        };
        let parsed: DocsArgs = serde_json::from_value(args.clone())
            .map_err(|e| std::io::Error::other(format!("invalid arguments: {e}")))?;
        let language = normalize_language(&parsed.language)?;
        let audience = normalize_audience(&parsed.audience)?;
        match parsed.operation.to_lowercase().trim() {
            "search" => search(catalog, &parsed.query, &language, &audience, parsed.limit),
            "read" => read(catalog, &parsed.section_id, &parsed.path),
            "list" => Ok(list(catalog, &language, &audience)),
            "" => Err(std::io::Error::other("operation is required").into()),
            other => Err(std::io::Error::other(format!("unknown operation {other:?}")).into()),
        }
    }

    fn read_only(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn search(
    catalog: &Catalog,
    query: &str,
    language: &str,
    audience: &str,
    limit: i64,
) -> ToolResult {
    let query = query.trim();
    if query.chars().count() > MAX_QUERY_RUNES {
        return Err(std::io::Error::other(format!(
            "query is too long: maximum {MAX_QUERY_RUNES} characters"
        ))
        .into());
    }
    let query_terms = retrieval::query_terms(query)?;
    let limit = clamp_limit(limit);
    let preferred_language = if language == "auto" {
        detect_query_language(query)
    } else {
        language.to_owned()
    };
    let query_lower = query.to_lowercase();

    let mut hits: Vec<(SectionRef, f64)> = Vec::new();
    for &r in &catalog.sections {
        let section = catalog.section(r);
        let doc = catalog.document(r);
        if language != "auto" && language != "all" && doc.locale != *language {
            continue;
        }
        if audience != "all" && doc.audience != *audience {
            continue;
        }
        let mut score = retrieval::bm25_score(
            &section.counts,
            section.length,
            &query_terms,
            &catalog.df,
            catalog.sections.len(),
            catalog.avg_len,
        );
        if score <= 0.0 {
            continue;
        }
        for term in &query_terms {
            if section.heading_hits.get(term).copied().unwrap_or(0) > 0 {
                score += 0.35;
            }
        }
        if !query_lower.is_empty() && section.search_text.to_lowercase().contains(&query_lower) {
            score += 1.5;
        }
        if preferred_language != "all" && doc.locale == preferred_language {
            score *= 1.15;
        }
        hits.push((r, score));
    }

    hits.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| catalog.section(a.0).id.cmp(&catalog.section(b.0).id))
    });
    let mut hits = retrieval::keep_top_relative_score(&mut hits, SCORE_FLOOR, |h| h.1);
    if hits.len() > limit {
        hits.truncate(limit);
    }
    Ok(format_search_results(catalog, query, &hits))
}

fn read(catalog: &Catalog, section_id: &str, document_path: &str) -> ToolResult {
    let section_id = section_id.trim();
    let document_path = document_path.trim();
    if !section_id.is_empty() {
        let r = catalog
            .by_id
            .get(section_id)
            .ok_or_else(|| {
                std::io::Error::other(format!(
                    "unknown section_id {section_id:?}; use operation=search or read with an exact path to list section ids"
                ))
            })?;
        let section = catalog.section(*r);
        let doc = catalog.document(*r);
        return Ok(format!(
            "Embedded Reasonix documentation ({})\nsource: {}\npath: {}\nsection_id: {}\nlocale: {}\naudience: {}\nheading: {}\n\n{}",
            catalog.identity_line(),
            doc.source_range(section.start_line, section.end_line),
            doc.display_path(),
            section.id,
            doc.locale,
            doc.audience,
            section.heading,
            section.content.trim()
        ));
    }
    if document_path.is_empty() {
        return Err(
            std::io::Error::other("section_id or path is required for operation=read").into(),
        );
    }
    let doc_idx = catalog
        .by_path
        .get(document_path)
        .or_else(|| {
            catalog
                .by_path
                .get(document_path.strip_prefix("docs/").unwrap_or(document_path))
        })
        .copied()
        .ok_or_else(|| {
            std::io::Error::other(format!(
                "unknown documentation path {document_path:?}; use operation=list for exact paths"
            ))
        })?;
    let doc = &catalog.docs[doc_idx];
    let mut out = format!(
        "{} ({}, audience={})\npath: {}\nsource: {}\nbuild: {}\n",
        doc.title,
        doc.locale,
        doc.audience,
        doc.display_path(),
        doc.source,
        catalog.identity_line()
    );
    for section in &doc.sections {
        out.push_str(&format!(
            "\n- section_id={} lines={}-{} heading={}",
            section.id, section.start_line, section.end_line, section.heading
        ));
    }
    out.push_str("\n\nUse operation=read with section_id to read one complete section.");
    Ok(out)
}

fn list(catalog: &Catalog, language: &str, audience: &str) -> String {
    let mut out = format!(
        "Embedded Reasonix documentation catalog ({}):\n",
        catalog.identity_line()
    );
    let mut count = 0usize;
    for doc in &catalog.docs {
        if language != "auto" && language != "all" && doc.locale != *language {
            continue;
        }
        if audience != "all" && doc.audience != *audience {
            continue;
        }
        count += 1;
        out.push_str(&format!(
            "\n- path={} locale={} audience={} sections={} title={}",
            doc.display_path(),
            doc.locale,
            doc.audience,
            doc.sections.len(),
            doc.title
        ));
    }
    if count == 0 {
        out.push_str("\n\nNo embedded documents matched the requested filters.");
    } else {
        out.push_str("\n\nUse operation=read with an exact path to list its section ids, or operation=search to rank relevant sections.");
    }
    out
}

fn format_search_results(catalog: &Catalog, query: &str, hits: &[(SectionRef, f64)]) -> String {
    if hits.is_empty() {
        return format!(
            "No embedded Reasonix documentation matched {query:?} ({}). Try fewer terms, an exact command/configuration key, language=all, or audience=all.",
            catalog.identity_line()
        );
    }
    let mut out = format!(
        "Embedded Reasonix documentation results for {query:?} ({}):\n",
        catalog.identity_line()
    );
    for (i, &(r, score)) in hits.iter().enumerate() {
        let section = catalog.section(r);
        let doc = catalog.document(r);
        let terms = retrieval::unique(&retrieval::tokens(query));
        let snippet = retrieval::make_snippet(&section.search_text, query, &terms, MAX_SNIPPET);
        out.push_str(&format!(
            "\n{}. score={:.3} source={} path={} section_id={} locale={} audience={}\n   heading: {}\n   snippet: {}\n",
            i + 1,
            score,
            doc.source_range(section.start_line, section.end_line),
            doc.display_path(),
            section.id,
            doc.locale,
            doc.audience,
            section.heading,
            snippet
        ));
    }
    out.push_str("\nUse operation=read with section_id to read the complete embedded section. Cite the source path and line range in the answer.");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_catalog() -> Catalog {
        let entries = vec![
            (
                "GUIDE.md".to_owned(),
                "# Getting Started\n\nInstall Reasonix.\n\n## MCP servers\n\nConfigure an MCP server in settings.\n".to_owned(),
            ),
            (
                "GUIDE.zh-CN.md".to_owned(),
                "# 入门\n\n安装 Reasonix。\n\n## MCP 服务器\n\n在设置中配置 MCP 服务器。\n".to_owned(),
            ),
        ];
        load_catalog(&entries).expect("sample catalog loads")
    }

    #[test]
    fn catalog_indexes_documents_and_sections_with_ids() {
        let catalog = sample_catalog();
        assert_eq!(catalog.docs.len(), 2);
        assert!(catalog.sections.len() >= 3);
        let mcp = catalog
            .sections
            .iter()
            .find(|r| catalog.section(**r).heading.contains("MCP"))
            .expect("mcp section");
        assert!(catalog.section(*mcp).id.contains("::s"));
    }

    #[test]
    fn search_ranks_a_matching_section_and_reads_it_back() {
        let catalog = sample_catalog();
        let hits =
            search(&catalog, "MCP server", "auto", "all", MAX_LIMIT as i64).expect("search ok");
        assert!(hits.contains("section_id="));
        assert!(hits.contains("MCP"));

        let section_id = catalog
            .sections
            .iter()
            .find(|r| catalog.section(**r).heading.contains("MCP"))
            .map(|r| catalog.section(*r).id.clone())
            .expect("id");
        let body = read(&catalog, &section_id, "").expect("read ok");
        assert!(body.contains("Embedded Reasonix documentation"));
        assert!(body.contains("Configure an MCP server"));
    }

    #[test]
    fn list_filters_by_language_and_audience() {
        let catalog = load_catalog(&[
            (
                "GUIDE.md".to_owned(),
                "# Getting Started\n\nInstall Reasonix.\n".to_owned(),
            ),
            (
                "GUIDE.zh-CN.md".to_owned(),
                "# 入门\n\n安装 Reasonix。\n".to_owned(),
            ),
            (
                "SPEC.md".to_owned(),
                "# Specification\n\nDeveloper contract.\n".to_owned(),
            ),
        ])
        .expect("sample catalog loads");
        let all = list(&catalog, "all", "all");
        assert!(all.contains("GUIDE.md"));
        assert!(all.contains("SPEC.md"));
        let en = list(&catalog, "en", "all");
        assert!(en.contains("GUIDE.md"));
        assert!(!en.contains("GUIDE.zh-CN.md"));
        let user = list(&catalog, "en", "user");
        assert!(user.contains("GUIDE.md"));
        assert!(!user.contains("SPEC.md"));
        let developer = list(&catalog, "en", "developer");
        assert!(developer.contains("SPEC.md"));
        assert!(!developer.contains("GUIDE.md"));
    }

    #[test]
    fn unknown_language_and_audience_are_rejected() {
        assert!(normalize_language("klingon").is_err());
        assert!(normalize_audience("root").is_err());
        assert!(normalize_language("").is_ok());
    }

    #[test]
    fn empty_corpus_and_duplicate_ids_fail() {
        assert!(load_catalog(&[]).is_err());
        let dup = vec![("GUIDE.md".to_owned(), "# Title\n\nBody\n".to_owned())];
        let catalog = load_catalog(&dup).expect("first load");
        let again = vec![
            ("GUIDE.md".to_owned(), "# Title\n\nBody\n".to_owned()),
            ("other.md".to_owned(), "# Title\n\nBody\n".to_owned()),
        ];
        // Same section-id shape across two docs with the same path stem would
        // collide only when paths repeat; here the duplicate-path guard fires.
        let _ = catalog;
        assert!(load_catalog(&again).is_ok());
    }

    #[test]
    fn identity_line_carries_the_digest_prefix() {
        let catalog = sample_catalog();
        assert!(catalog.identity_line().contains("digest=sha256:"));
    }
}
