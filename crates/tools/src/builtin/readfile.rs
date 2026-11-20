//! The read_file tool (readfile.go): numbered-window reads for the model,
//! verbatim reads for calls another tool made, the encoding cascade, and the
//! binary/image refusals. Parity target: internal/tools/builtin/readfile.go.

#![forbid(unsafe_code)]

use std::io::{BufReader, Chain, Cursor, Read};

use serde_json::Value;

use super::confine::{confine_opened, confine_read, confine_scope};
use super::network::{ResolvedPath, resolve_readable_path};
use crate::abi::{CallContext, Tool, ToolResult};

const READ_FILE_BINARY_PEEK: usize = 8 * 1024;
const READ_FILE_DETECT_SAMPLE: usize = 256 * 1024;
pub const READ_FILE_DEFAULT_LIMIT: usize = 2000;
const SCAN_MAX_LINE: usize = 1024 * 1024;
const SCAN_BUF: usize = 64 * 1024;

/// The read_file tool. `work_dir` resolves relative paths; `net_roots` /
/// `forbid_roots` / `read_roots` carry the network and confinement scopes;
/// `paths` anchors the §7.7 token-alias resolver (not migrated). overlay and
/// views (§7.5) are likewise unmigrated machines — nil on the Go side reads
/// as absent.
pub struct ReadFileTool {
    pub net_roots: Vec<String>,
    pub work_dir: String,
    pub forbid_roots: Vec<String>,
    pub read_roots: Vec<String>,
}

impl ReadFileTool {
    /// The read boundary: outside the read roots, or a forbidden path that is
    /// answered as if it did not exist (readfile.go `refuse`).
    fn refuse(&self, rp: &ResolvedPath) -> Option<Box<dyn std::error::Error + Send + Sync>> {
        if let Some(r) = rp.refuse_network(&self.net_roots) {
            return Some(Box::new(r));
        }
        if let Some(r) = confine_scope(&self.read_roots, &rp.path) {
            return Some(Box::new(r));
        }
        if !confine_read(&self.forbid_roots, &rp.path) {
            return None;
        }
        let err = std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("open {}: file does not exist", rp.path),
        );
        if rp.external {
            return Some(Box::new(std::io::Error::new(
                err.kind(),
                format!("read {}: {}", rp.display_path, rp.error_text(&err)),
            )));
        }
        Some(Box::new(err))
    }

    fn read(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let path_arg = args.get("path").and_then(Value::as_str).unwrap_or("");
        let offset = args.get("offset").and_then(Value::as_i64).unwrap_or(0);
        let limit = args.get("limit").and_then(Value::as_i64).unwrap_or(0);
        if path_arg.is_empty() {
            return Err(Box::new(std::io::Error::other("path is required")));
        }
        let rp = resolve_readable_path(&self.work_dir, path_arg);
        let display_path = rp.display_path.clone();
        if let Some(err) = self.refuse(&rp) {
            return Err(err);
        }
        let offset = offset.max(0) as usize;
        let (emit, limit) = render(ctx, limit);

        // A directory can be opened but not read as text — an actionable
        // message up front so the model switches to the ls tool.
        if let Ok(info) = std::fs::metadata(&rp.path)
            && info.is_dir()
        {
            return Err(Box::new(std::io::Error::other(format!(
                "{} is a directory, not a file — use the ls tool to list it, or read a specific file inside it",
                display_path
            ))));
        }

        let mut file = match std::fs::File::open(&rp.path) {
            Ok(f) => f,
            Err(err) => return Err(read_error(display_path, &rp, err)),
        };
        // confine_opened owns the re-resolution and its fallback spelling;
        // a resolve failure here must recheck the opened path, not an empty
        // one that reads as outside every root.
        if let Some(r) = confine_opened(&self.read_roots, &rp.path) {
            return Err(Box::new(r));
        }

        // Peek the first 8 KiB: a NUL byte rejects binaries cheaply before
        // any further read, keeping a multi-GB archive from being slurped.
        let mut peek = vec![0u8; READ_FILE_BINARY_PEEK];
        let pn = read_full(&mut file, &mut peek)?;
        peek.truncate(pn);
        let peek_eof = pn < READ_FILE_BINARY_PEEK;

        // BOM check first: UTF-16 files hold a 0x00 per ASCII character, so a
        // naive NUL check would misidentify them as binary.
        match reasonix_core::fileencoding::detect_quick(&peek) {
            reasonix_core::fileencoding::Kind::Utf16Le
            | reasonix_core::fileencoding::Kind::Utf16Be => {
                let mut rest = Vec::new();
                file.read_to_end(&mut rest)
                    .map_err(|e| read_error(display_path.clone(), &rp, e))?;
                let mut all = peek.clone();
                all.extend_from_slice(&rest);
                let kind = reasonix_core::fileencoding::detect_quick(&all);
                return emit(
                    &mut Cursor::new(reasonix_core::fileencoding::decode(&all, kind).into_owned()),
                    offset,
                    limit,
                );
            }
            reasonix_core::fileencoding::Kind::Utf8Bom => {
                let body = if peek.len() >= 3 {
                    peek[3..].to_vec()
                } else {
                    peek.clone()
                };
                let src: Chain<Cursor<Vec<u8>>, std::fs::File> = Cursor::new(body).chain(file);
                return emit(&mut BufReader::new(src), offset, limit);
            }
            _ => {}
        }

        // BOM-less UTF-16 (Windows source files) reaches here; recognise it
        // by its NUL pattern and decode rather than reject as binary.
        if let Some(kind) = reasonix_core::fileencoding::detect_utf16_no_bom(&peek) {
            let mut rest = Vec::new();
            file.read_to_end(&mut rest)
                .map_err(|e| read_error(display_path.clone(), &rp, e))?;
            let mut all = peek.clone();
            all.extend_from_slice(&rest);
            return emit(
                &mut Cursor::new(reasonix_core::fileencoding::decode(&all, kind).into_owned()),
                offset,
                limit,
            );
        }

        if peek.contains(&0) {
            return Err(Box::new(std::io::Error::other(binary_refusal(
                &display_path,
                &peek,
                rp.external,
            ))));
        }

        // A bounded sample decides the encoding; the rest streams.
        let mut head = peek.clone();
        if !peek_eof {
            let mut more = vec![0u8; READ_FILE_DETECT_SAMPLE - head.len()];
            let mn = read_full(&mut file, &mut more)?;
            more.truncate(mn);
            head.extend_from_slice(&more);
        }
        // When more file follows, the sample can end inside a character.
        let enc = if peek_eof {
            reasonix_core::fileencoding::detect_and_decode(&head).0
        } else {
            reasonix_core::fileencoding::detect_fragment(&head).0
        };
        match enc {
            reasonix_core::fileencoding::Kind::Gb18030 | reasonix_core::fileencoding::Kind::Gbk => {
                let mut rest = Vec::new();
                file.read_to_end(&mut rest)
                    .map_err(|e| read_error(display_path.clone(), &rp, e))?;
                let mut all = head.clone();
                all.extend_from_slice(&rest);
                emit(
                    &mut Cursor::new(reasonix_core::fileencoding::decode(&all, enc).into_owned()),
                    offset,
                    limit,
                )
            }
            _ => {
                let src: Chain<Cursor<Vec<u8>>, std::fs::File> = Cursor::new(head).chain(file);
                emit(&mut BufReader::new(src), offset, limit)
            }
        }
    }
}

/// What render hands back: the line formatter and the effective limit.
type Emitter = fn(&mut dyn Read, usize, i64) -> ToolResult;

fn render(ctx: &CallContext, limit: i64) -> (Emitter, i64) {
    if ctx.is_nested() {
        return (verbatim, limit);
    }
    let limit = if limit <= 0 {
        READ_FILE_DEFAULT_LIMIT as i64
    } else {
        limit
    };
    (scan, limit)
}

fn read_error(
    display_path: String,
    rp: &ResolvedPath,
    err: std::io::Error,
) -> Box<dyn std::error::Error + Send + Sync> {
    if rp.external {
        Box::new(std::io::Error::new(
            err.kind(),
            format!("read {}: {}", display_path, rp.error_text(&err)),
        ))
    } else {
        Box::new(std::io::Error::new(
            err.kind(),
            format!("read {display_path}: {err}"),
        ))
    }
}

/// Reads until buf is full or EOF (Go io.ReadFull).
fn read_full(file: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0usize;
    while n < buf.len() {
        match file.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(m) => n += m,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// readfile.go `scan`: numbered lines paged at the window, stopping at the
/// first line past it rather than counting the remainder.
pub fn scan(src: &mut dyn Read, offset: usize, limit: i64) -> ToolResult {
    let mut collected: Vec<String> = Vec::new();
    let mut line_no = 0usize;
    let mut has_more = false;
    let mut lines = ChunkLines::new(src);
    loop {
        match lines
            .next_line()
            .map_err(|e| std::io::Error::new(e.kind(), format!("scan: {e}")))?
        {
            None => break,
            Some(line) => {
                line_no += 1;
                if line_no <= offset {
                    continue;
                }
                if (collected.len() as i64) < limit {
                    collected.push(line);
                    continue;
                }
                has_more = true;
                break;
            }
        }
    }
    if line_no == 0 {
        return Ok(String::from("(empty file)"));
    }
    if collected.is_empty() {
        return Ok(format!(
            "(offset {offset} is past EOF — file has {line_no} lines)"
        ));
    }
    let max_shown = offset + collected.len();
    let w = max_showed_width(max_shown);
    let mut b = String::new();
    for (i, line) in collected.iter().enumerate() {
        b.push_str(&format!("{:>w$}→{}\n", offset + i + 1, line));
    }
    if has_more {
        b.push_str(&format!(
            "\n[more lines below; pass offset={} to continue]\n",
            offset + collected.len()
        ));
    }
    Ok(b)
}

fn max_showed_width(n: usize) -> usize {
    let mut w = 1;
    let mut v = n;
    while v >= 10 {
        v /= 10;
        w += 1;
    }
    w
}

/// Line reader mirroring bufio.Scanner + ScanLines: split on \n, strip a
/// trailing \r, 1 MiB line ceiling. Reads in bounded chunks so a small
/// windowed read never drains the file.
pub struct ChunkLines<'a> {
    src: &'a mut dyn Read,
    pending: Vec<u8>,
    eof: bool,
}

impl<'a> ChunkLines<'a> {
    pub fn new(src: &'a mut dyn Read) -> Self {
        Self {
            src,
            pending: Vec::with_capacity(SCAN_BUF),
            eof: false,
        }
    }

    /// The next line, or None at end of input.
    pub fn next_line(&mut self) -> std::io::Result<Option<String>> {
        loop {
            if let Some(i) = self.pending.iter().position(|&b| b == b'\n') {
                let mut line: Vec<u8> = self.pending.drain(..=i).collect();
                line.truncate(line.len() - 1);
                if line.last() == Some(&b'\r') {
                    line.truncate(line.len() - 1);
                }
                return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
            }
            if self.eof {
                if self.pending.is_empty() {
                    return Ok(None);
                }
                let mut line = std::mem::take(&mut self.pending);
                if line.last() == Some(&b'\r') {
                    line.truncate(line.len() - 1);
                }
                return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
            }
            let mut chunk = [0u8; SCAN_BUF];
            let n = read_chunk(self.src, &mut chunk)?;
            if n == 0 {
                self.eof = true;
                continue;
            }
            self.pending.extend_from_slice(&chunk[..n]);
            if self.pending.len() > SCAN_MAX_LINE {
                return Err(std::io::Error::other("bufio.Scanner: token too long"));
            }
        }
    }
}

fn read_chunk(src: &mut dyn Read, buf: &mut [u8]) -> std::io::Result<usize> {
    loop {
        match src.read(buf) {
            Ok(0) => return Ok(0),
            Ok(n) => return Ok(n),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
}

/// readfile.go `verbatim` — the text itself for a nested call, line endings
/// and final newline included; offset/limit select whole lines.
pub fn verbatim(src: &mut dyn Read, offset: usize, limit: i64) -> ToolResult {
    let mut data = Vec::new();
    src.read_to_end(&mut data)
        .map_err(|e| std::io::Error::new(e.kind(), format!("read: {e}")))?;
    let text = String::from_utf8_lossy(&data).into_owned();
    if offset == 0 && limit <= 0 {
        return Ok(text);
    }
    let mut lines: Vec<String> = text.split_inclusive('\n').map(String::from).collect();
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    if offset >= lines.len() {
        return Ok(String::new());
    }
    let mut end = lines.len();
    if limit > 0 && (offset + limit as usize) < end {
        end = offset + limit as usize;
    }
    Ok(lines[offset..end].concat())
}

/// readfile.go `binaryRefusal`: an image is told where pictures actually
/// travel; anything else is pointed at hexdump.
fn binary_refusal(display_path: &str, head: &[u8], external: bool) -> String {
    if let Some(kind) = image_kind(head) {
        return format!(
            "{display_path} is a {kind} image; read_file reads text. If view_image is among your tools, use it to see the picture; otherwise it is viewable only when the user's message references it as @{display_path} — writing that yourself attaches nothing, so ask them to"
        );
    }
    if external {
        return format!("binary file {display_path} (NUL byte detected); not shown by read_file");
    }
    format!("binary file {display_path} (NUL byte detected); use `bash hexdump` or another tool")
}

/// The formats the vision path accepts; anything else is binary like any
/// other (readfile.go `imageKind`).
pub fn image_kind(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("PNG");
    }
    if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("JPEG");
    }
    if head.starts_with(b"GIF8") {
        return Some("GIF");
    }
    if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        return Some("WebP");
    }
    None
}

impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read a text file with optional line offset/limit. Output prefixes each line with its 1-based number (e.g. `   42→...`) so subsequent edit_file calls can target exact lines. Use `offset` and `limit` to page through large files; the tool reports total length and pagination hints in a trailer."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path"},
                "offset": {"type": "integer", "description": "0-based line offset to start reading from (default 0)", "minimum": 0},
                "limit": {"type": "integer", "description": "Maximum lines to return (default 2000)", "minimum": 1}
            },
            "required": ["path"]
        })
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        self.read(ctx, args)
    }

    fn read_only(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
