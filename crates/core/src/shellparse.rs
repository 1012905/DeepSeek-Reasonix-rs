//! Static shell command analysis — a port of the reference Go package
//! `internal/base/shellparse` (static-command subset).
//!
//! The reference delegates parsing to `mvdan.cc/sh/v3/syntax` (bash
//! variant) and classifies the resulting AST. This module implements a
//! byte-level bash-subset lexer that reproduces the same classification for
//! the functions the kernel consumes here: [`parse_static_command`],
//! [`static_fields`], [`static_word`], [`contains_shell_syntax`], and
//! [`heredoc_present`]. Pipelines, compound statements, subshells, and any
//! expansion are detected and rejected with the reference reason strings.

use std::fmt;

/// Why a command was rejected (reference: `StaticRejectReason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaticReason {
    Parse,
    HereDoc,
    Control,
    Redirection,
    Assignment,
    Expansion,
}

impl StaticReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            StaticReason::Parse => "parse error",
            StaticReason::HereDoc => "here document",
            StaticReason::Control => "shell control syntax",
            StaticReason::Redirection => "shell redirection",
            StaticReason::Assignment => "shell assignment",
            StaticReason::Expansion => "shell expansion",
        }
    }
}

impl fmt::Display for StaticReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned by [`parse_static_command`] (reference: `StaticRejectError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticRejectError {
    pub reason: StaticReason,
    pub detail: String,
}

impl StaticRejectError {
    fn new(reason: StaticReason) -> Self {
        StaticRejectError {
            reason,
            detail: String::new(),
        }
    }

    fn with_detail(reason: StaticReason, detail: impl Into<String>) -> Self {
        StaticRejectError {
            reason,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for StaticRejectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.detail.is_empty() {
            f.write_str(&self.detail)
        } else {
            f.write_str(self.reason.as_str())
        }
    }
}

impl std::error::Error for StaticRejectError {}

/// Policy switches (reference: `StaticCommandPolicy`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StaticCommandPolicy {
    pub allow_env_assignments: bool,
    pub allow_stderr_to_stdout: bool,
}

/// A parsed simple command (reference: `StaticCommand`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StaticCommand {
    pub argv: Vec<String>,
    pub env: Vec<String>,
    pub merge_stderr: bool,
}

/// One part of a bash word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WordPart {
    /// Unquoted literal run; raw bytes, escapes not yet interpreted.
    Lit(String),
    /// Single-quoted content (no escapes; also ANSI-C strings).
    SglQuoted(String),
    /// Double-quoted nested parts.
    DblQuoted(Vec<WordPart>),
    /// Any expansion or process substitution — never static.
    Expansion,
    /// A `$(...)` command substitution; the inner text is kept for the
    /// resolved-substitution analysis (double-quoted, one static statement).
    CmdSubst(String),
}

pub(crate) struct RawRedir {
    pub(crate) fd: Option<String>,
    pub(crate) op: &'static str,
    pub(crate) target: Vec<WordPart>,
    /// Byte span of the whole redirect spelling (io number included),
    /// the shape the redirect matcher strips.
    pub(crate) span: (usize, usize),
}

impl RawRedir {
    fn is_stderr_to_stdout(&self) -> bool {
        self.op == ">&"
            && self.fd.as_deref() == Some("2")
            && static_word(&self.target).as_deref() == Some("1")
    }
}

/// The operator that joined this statement to the next one, recorded by
/// the parser as it pushes (the reference reads it off `BinaryCmd.Op` from
/// the left operand). `Join::None` on the last statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Join {
    #[default]
    None,
    Sequence,
    And,
    Or,
    Pipe,
    PipeAll,
}

#[derive(Default)]
pub(crate) struct RawStmt {
    pub(crate) words: Vec<Vec<WordPart>>,
    pub(crate) redirs: Vec<RawRedir>,
    pub(crate) ctrl: bool,
    pub(crate) background: bool,
    pub(crate) negated: bool,
    /// Byte span of the statement's own text; the trailing separator is
    /// excluded the way `sourceForStmt` trims a `;` terminator.
    pub(crate) span: (usize, usize),
    pub(crate) join: Join,
    /// The statement reads its program from a stdin here-doc (fd unset or 0).
    pub(crate) stdin_heredoc: bool,
}

impl RawStmt {
    fn is_empty(&self) -> bool {
        self.words.is_empty() && self.redirs.is_empty() && !self.ctrl
    }

    fn starts_at(&mut self, pos: usize) {
        self.span = (pos, pos);
    }

    fn ends_at(&mut self, end: usize) {
        self.span.1 = self.span.1.max(end);
    }
}

#[derive(Default)]
pub(crate) struct Parsed {
    pub(crate) stmts: Vec<RawStmt>,
    pub(crate) heredoc: bool,
}

pub(crate) fn parse(input: &str) -> Result<Parsed, String> {
    P::new(input).parse_file()
}

struct P<'a> {
    text: &'a str,
    src: &'a [u8],
    pos: usize,
    heredoc_seen: bool,
    stdin_heredoc_pending: bool,
}

impl<'a> P<'a> {
    fn new(text: &'a str) -> Self {
        P {
            text,
            src: text.as_bytes(),
            pos: 0,
            heredoc_seen: false,
            stdin_heredoc_pending: false,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, off: usize) -> Option<u8> {
        self.src.get(self.pos + off).copied()
    }

    fn char_at(&self, i: usize) -> char {
        self.text[i..].chars().next().unwrap()
    }

    fn skip_ws(&mut self) {
        loop {
            match self.peek() {
                Some(b' ') | Some(b'\t') => self.pos += 1,
                Some(b'\\') if self.peek_at(1) == Some(b'\n') => self.pos += 2,
                _ => break,
            }
        }
    }

    fn parse_file(&mut self) -> Result<Parsed, String> {
        let mut parsed = Parsed::default();
        let mut cur = RawStmt::default();
        let mut pending: Vec<(String, bool)> = Vec::new();
        loop {
            self.skip_ws();
            let Some(c) = self.peek() else {
                if let Some((delim, _)) = pending.first() {
                    return Err(format!("unclosed here-doc '{delim}'"));
                }
                if !cur.is_empty() {
                    cur.ends_at(self.pos);
                    parsed.stmts.push(std::mem::take(&mut cur));
                }
                parsed.heredoc = self.heredoc_seen;
                return Ok(parsed);
            };
            match c {
                b'\n' => {
                    self.pos += 1;
                    let stdin_heredoc = self.stdin_heredoc_pending;
                    self.stdin_heredoc_pending = false;
                    self.consume_heredoc_bodies(&mut pending)?;
                    if !cur.is_empty() {
                        cur.stdin_heredoc = stdin_heredoc;
                        cur.ends_at(self.pos - 1);
                        cur.join = Join::Sequence;
                        parsed.stmts.push(std::mem::take(&mut cur));
                    }
                }
                b';' => {
                    if cur.is_empty() {
                        return Err("unexpected ';'".to_string());
                    }
                    cur.ends_at(self.pos);
                    cur.join = Join::Sequence;
                    parsed.stmts.push(std::mem::take(&mut cur));
                    self.pos += 1;
                    if self.peek() == Some(b';') {
                        return Err("unexpected ';;'".to_string());
                    }
                }
                b'&' => match self.peek_at(1) {
                    Some(b'&') => {
                        self.pos += 2;
                        cur.ends_at(self.pos - 2);
                        cur.join = Join::And;
                        parsed.stmts.push(std::mem::take(&mut cur));
                    }
                    Some(b'>') => {
                        let op_start = self.pos;
                        self.pos += 1;
                        let op = if self.peek_at(1) == Some(b'>') {
                            self.pos += 2;
                            "&>>"
                        } else {
                            self.pos += 1;
                            "&>"
                        };
                        let target = self.lex_redirect_target()?;
                        cur.redirs.push(RawRedir {
                            fd: None,
                            op,
                            target,
                            span: (op_start, self.pos),
                        });
                    }
                    _ => {
                        if cur.is_empty() {
                            return Err("unexpected '&'".to_string());
                        }
                        cur.ends_at(self.pos);
                        cur.background = true;
                        cur.join = Join::Sequence;
                        parsed.stmts.push(std::mem::take(&mut cur));
                    }
                },
                b'|' => {
                    let (join, width) = if self.peek_at(1) == Some(b'|') {
                        (Join::Or, 2)
                    } else if self.peek_at(1) == Some(b'&') {
                        (Join::PipeAll, 2)
                    } else {
                        (Join::Pipe, 1)
                    };
                    if cur.is_empty() {
                        return Err("unexpected '|'".to_string());
                    }
                    self.pos += width;
                    cur.ends_at(self.pos - width);
                    cur.join = join;
                    parsed.stmts.push(std::mem::take(&mut cur));
                }
                b'(' => {
                    if !cur.is_empty() {
                        return Err("unexpected '('".to_string());
                    }
                    self.consume_balanced_paren()?;
                    cur.ctrl = true;
                }
                b')' => return Err("unexpected ')'".to_string()),
                b'#' => {
                    while let Some(c) = self.peek() {
                        if c == b'\n' {
                            break;
                        }
                        self.pos += 1;
                    }
                }
                b'<' | b'>' => {
                    if cur.is_empty() {
                        cur.starts_at(self.pos);
                    }
                    if self.peek_at(1) == Some(b'(') {
                        self.pos += 1;
                        self.consume_balanced_paren()?;
                        cur.words.push(vec![WordPart::Expansion]);
                    } else {
                        let redir = self.lex_redirect_and_heredoc(None, &mut pending)?;
                        cur.redirs.push(redir);
                    }
                    cur.ends_at(self.pos);
                }
                _ => {
                    if c.is_ascii_digit()
                        && let Some(fd) = self.try_io_number()
                    {
                        if cur.is_empty() {
                            cur.starts_at(self.pos);
                        }
                        let redir = self.lex_redirect_and_heredoc(Some(fd), &mut pending)?;
                        cur.redirs.push(redir);
                        cur.ends_at(self.pos);
                        continue;
                    }
                    let word_start = self.pos;
                    let word = self.lex_word()?;
                    if cur.words.is_empty()
                        && cur.redirs.is_empty()
                        && !cur.ctrl
                        && is_reserved(&word)
                    {
                        cur.ctrl = true;
                        if matches!(&word[..], [WordPart::Lit(w)] if w == "!") {
                            cur.negated = true;
                        }
                        continue;
                    }
                    if cur.is_empty() {
                        cur.starts_at(word_start);
                    }
                    cur.words.push(word);
                    cur.ends_at(self.pos);
                }
            }
        }
    }

    fn try_io_number(&mut self) -> Option<String> {
        let start = self.pos;
        let mut end = self.pos;
        while matches!(self.src.get(end), Some(b'0'..=b'9')) {
            end += 1;
        }
        if end > start && matches!(self.src.get(end), Some(b'<') | Some(b'>')) {
            self.pos = end;
            Some(self.text[start..end].to_string())
        } else {
            None
        }
    }

    fn lex_redirect_and_heredoc(
        &mut self,
        fd: Option<String>,
        pending: &mut Vec<(String, bool)>,
    ) -> Result<RawRedir, String> {
        let op_start = match &fd {
            Some(fd) => self.pos - fd.len(),
            None => self.pos,
        };
        let c = self.peek().unwrap();
        let op: &'static str = match c {
            b'<' => match self.peek_at(1) {
                Some(b'<') => match self.peek_at(2) {
                    Some(b'-') => {
                        self.pos += 3;
                        "<<-"
                    }
                    Some(b'<') => {
                        self.pos += 3;
                        "<<<"
                    }
                    _ => {
                        self.pos += 2;
                        "<<"
                    }
                },
                Some(b'&') => {
                    self.pos += 2;
                    "<&"
                }
                _ => {
                    self.pos += 1;
                    "<"
                }
            },
            _ => match self.peek_at(1) {
                Some(b'>') => {
                    self.pos += 2;
                    ">>"
                }
                Some(b'&') => {
                    self.pos += 2;
                    ">&"
                }
                _ => {
                    self.pos += 1;
                    ">"
                }
            },
        };
        let target = if matches!(op, "<<" | "<<-") {
            self.lex_heredoc_delim()?
        } else {
            self.lex_redirect_target()?
        };
        if matches!(op, "<<" | "<<-") {
            let delim = static_word(&target)
                .filter(|d| !d.is_empty())
                .ok_or_else(|| "here-document delimiter".to_string())?;
            pending.push((delim, op == "<<-"));
            self.heredoc_seen = true;
            self.stdin_heredoc_pending = fd.is_none() || fd.as_deref() == Some("0");
        }
        Ok(RawRedir {
            fd,
            op,
            target,
            span: (op_start, self.pos),
        })
    }

    fn lex_redirect_target(&mut self) -> Result<Vec<WordPart>, String> {
        while matches!(self.peek(), Some(b' ') | Some(b'\t')) {
            self.pos += 1;
        }
        match self.peek() {
            None | Some(b'\n') | Some(b';') | Some(b'|') | Some(b'&') | Some(b'(') | Some(b')')
            | Some(b'<') | Some(b'>') | Some(b'#') => Err("missing redirection target".to_string()),
            _ => self.lex_word(),
        }
    }

    fn lex_heredoc_delim(&mut self) -> Result<Vec<WordPart>, String> {
        while matches!(self.peek(), Some(b' ') | Some(b'\t')) {
            self.pos += 1;
        }
        match self.peek() {
            None | Some(b'\n') | Some(b';') | Some(b'|') | Some(b'&') | Some(b'(') | Some(b')')
            | Some(b'<') | Some(b'>') | Some(b'#') => {
                Err("missing here-document delimiter".to_string())
            }
            _ => self.lex_word(),
        }
    }

    fn lex_word(&mut self) -> Result<Vec<WordPart>, String> {
        let mut parts: Vec<WordPart> = Vec::new();
        let mut lit = String::new();
        while let Some(c) = self.peek() {
            match c {
                b' ' | b'\t' | b'\n' | b';' | b'|' | b'&' | b')' | b'<' | b'>' => break,
                b'(' => {
                    // extglob opener: the byte just consumed sits in the literal
                    if self.pos > 0
                        && matches!(self.src[self.pos - 1], b'*' | b'@' | b'+' | b'?' | b'!')
                    {
                        flush_lit(&mut parts, &mut lit);
                        self.consume_balanced_paren()?;
                        parts.push(WordPart::Expansion);
                    } else {
                        break;
                    }
                }
                b'\\' => match self.peek_at(1) {
                    None => return Err("unterminated escape".to_string()),
                    Some(b'\n') => {
                        lit.push('\\');
                        lit.push('\n');
                        self.pos += 2;
                    }
                    Some(_) => {
                        let n = self.char_at(self.pos + 1);
                        lit.push('\\');
                        lit.push(n);
                        self.pos += 1 + n.len_utf8();
                    }
                },
                b'\'' => {
                    flush_lit(&mut parts, &mut lit);
                    self.pos += 1;
                    let val = self.consume_single_quoted()?;
                    parts.push(WordPart::SglQuoted(val));
                }
                b'"' => {
                    flush_lit(&mut parts, &mut lit);
                    self.pos += 1;
                    let dq = self.consume_dq()?;
                    parts.push(WordPart::DblQuoted(dq));
                }
                b'`' => {
                    flush_lit(&mut parts, &mut lit);
                    self.pos += 1;
                    self.consume_backquote()?;
                    parts.push(WordPart::Expansion);
                }
                b'$' => match self.peek_at(1) {
                    Some(b'\'') => {
                        flush_lit(&mut parts, &mut lit);
                        self.pos += 2;
                        let decoded = self.consume_ansic()?;
                        parts.push(WordPart::SglQuoted(decoded));
                    }
                    Some(b'"') => {
                        flush_lit(&mut parts, &mut lit);
                        self.pos += 2;
                        let dq = self.consume_dq()?;
                        parts.push(WordPart::DblQuoted(dq));
                    }
                    Some(b'{') => {
                        flush_lit(&mut parts, &mut lit);
                        self.pos += 2;
                        self.consume_brace_param()?;
                        parts.push(WordPart::Expansion);
                    }
                    Some(b'(') => {
                        flush_lit(&mut parts, &mut lit);
                        self.pos += 1;
                        let inner = self.consume_balanced_paren_inner(true)?;
                        parts.push(WordPart::CmdSubst(inner));
                    }
                    Some(b'[') => {
                        flush_lit(&mut parts, &mut lit);
                        self.pos += 2;
                        self.consume_arithm_bracket()?;
                        parts.push(WordPart::Expansion);
                    }
                    Some(c2) if is_param_char(c2) => {
                        flush_lit(&mut parts, &mut lit);
                        self.pos += 1;
                        self.consume_param(c2);
                        parts.push(WordPart::Expansion);
                    }
                    _ => {
                        lit.push('$');
                        self.pos += 1;
                    }
                },
                _ => {
                    let ch = self.char_at(self.pos);
                    lit.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
        flush_lit(&mut parts, &mut lit);
        Ok(parts)
    }

    fn consume_param(&mut self, first: u8) {
        self.pos += 1;
        if !is_special_param(first) {
            while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == b'_') {
                self.pos += 1;
            }
        }
    }

    fn consume_single_quoted(&mut self) -> Result<String, String> {
        match self.src[self.pos..].iter().position(|&b| b == b'\'') {
            None => {
                self.pos = self.src.len();
                Err("unterminated single quote".to_string())
            }
            Some(i) => {
                let val = self.text[self.pos..self.pos + i].to_string();
                self.pos += i + 1;
                Ok(val)
            }
        }
    }

    fn consume_dq(&mut self) -> Result<Vec<WordPart>, String> {
        let mut parts: Vec<WordPart> = Vec::new();
        let mut lit = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err("unterminated double quote".to_string());
            };
            match c {
                b'"' => {
                    self.pos += 1;
                    flush_lit(&mut parts, &mut lit);
                    return Ok(parts);
                }
                b'\\' => match self.peek_at(1) {
                    None => return Err("unterminated double quote".to_string()),
                    Some(b'\n') => {
                        lit.push('\\');
                        lit.push('\n');
                        self.pos += 2;
                    }
                    Some(_) => {
                        let n = self.char_at(self.pos + 1);
                        lit.push('\\');
                        lit.push(n);
                        self.pos += 1 + n.len_utf8();
                    }
                },
                b'$' => match self.peek_at(1) {
                    Some(b'{') => {
                        flush_lit(&mut parts, &mut lit);
                        self.pos += 2;
                        self.consume_brace_param()?;
                        parts.push(WordPart::Expansion);
                    }
                    Some(b'(') => {
                        flush_lit(&mut parts, &mut lit);
                        self.pos += 1;
                        let inner = self.consume_balanced_paren_inner(true)?;
                        parts.push(WordPart::CmdSubst(inner));
                    }
                    Some(c2) if is_param_char(c2) => {
                        flush_lit(&mut parts, &mut lit);
                        self.pos += 1;
                        self.consume_param(c2);
                        parts.push(WordPart::Expansion);
                    }
                    _ => {
                        lit.push('$');
                        self.pos += 1;
                    }
                },
                b'`' => {
                    flush_lit(&mut parts, &mut lit);
                    self.pos += 1;
                    self.consume_backquote()?;
                    parts.push(WordPart::Expansion);
                }
                _ => {
                    let ch = self.char_at(self.pos);
                    lit.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn consume_ansic(&mut self) -> Result<String, String> {
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err("unterminated ANSI-C string".to_string());
            };
            match c {
                b'\'' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    let esc = match self.peek_at(1) {
                        None => return Err("unterminated ANSI-C string".to_string()),
                        Some(b'\n') => {
                            self.pos += 2;
                            continue;
                        }
                        Some(b'\\') => {
                            self.pos += 2;
                            '\\'.to_string()
                        }
                        Some(b'\'') => {
                            self.pos += 2;
                            "'".to_string()
                        }
                        Some(b'"') => {
                            self.pos += 2;
                            "\"".to_string()
                        }
                        Some(b'a') => {
                            self.pos += 2;
                            "\x07".to_string()
                        }
                        Some(b'b') => {
                            self.pos += 2;
                            "\x08".to_string()
                        }
                        Some(b'e') => {
                            self.pos += 2;
                            "\x1b".to_string()
                        }
                        Some(b'f') => {
                            self.pos += 2;
                            "\x0c".to_string()
                        }
                        Some(b'n') => {
                            self.pos += 2;
                            "\n".to_string()
                        }
                        Some(b'r') => {
                            self.pos += 2;
                            "\r".to_string()
                        }
                        Some(b't') => {
                            self.pos += 2;
                            "\t".to_string()
                        }
                        Some(b'v') => {
                            self.pos += 2;
                            "\x0b".to_string()
                        }
                        Some(b'c') => {
                            let ctrl = self
                                .peek_at(2)
                                .ok_or_else(|| "unterminated ANSI-C string".to_string())?;
                            self.pos += 3;
                            ((ctrl & 0x1f) as char).to_string()
                        }
                        Some(b'x') => {
                            self.pos += 2;
                            let (val, _) = self.take_hex(2);
                            char::from_u32(val & 0xff).unwrap_or('\u{fffd}').to_string()
                        }
                        Some(u @ (b'u' | b'U')) => {
                            let want = if u == b'U' { 8 } else { 4 };
                            self.pos += 2;
                            let (val, n) = self.take_hex(want);
                            if n == want {
                                char::from_u32(val).unwrap_or('\u{fffd}').to_string()
                            } else {
                                '\u{fffd}'.to_string()
                            }
                        }
                        Some(d) if d.is_ascii_digit() => {
                            self.pos += 1;
                            let mut val: u32 = 0;
                            let mut n = 0;
                            while n < 3 {
                                match self.peek() {
                                    Some(o) if (b'0'..=b'7').contains(&o) => {
                                        val = val * 8 + (o - b'0') as u32;
                                        self.pos += 1;
                                        n += 1;
                                    }
                                    _ => break,
                                }
                            }
                            char::from_u32(val & 0xff).unwrap_or('\u{fffd}').to_string()
                        }
                        Some(_) => {
                            let n = self.char_at(self.pos + 1);
                            self.pos += 1 + n.len_utf8();
                            format!("\\{n}")
                        }
                    };
                    out.push_str(&esc);
                }
                _ => {
                    let ch = self.char_at(self.pos);
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn take_hex(&mut self, want: usize) -> (u32, usize) {
        let mut val: u32 = 0;
        let mut n = 0;
        while n < want {
            match self.peek() {
                Some(h) if h.is_ascii_hexdigit() => {
                    val = val
                        .wrapping_mul(16)
                        .wrapping_add((h as char).to_digit(16).unwrap());
                    self.pos += 1;
                    n += 1;
                }
                _ => break,
            }
        }
        (val, n)
    }

    fn consume_brace_param(&mut self) -> Result<(), String> {
        let mut depth = 1usize;
        loop {
            let Some(c) = self.peek() else {
                return Err("unterminated parameter expansion".to_string());
            };
            match c {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        self.pos += 1;
                        return Ok(());
                    }
                }
                _ => {}
            }
            self.pos += 1;
        }
    }

    fn consume_arithm_bracket(&mut self) -> Result<(), String> {
        loop {
            let Some(c) = self.peek() else {
                return Err("unterminated arithmetic expansion".to_string());
            };
            self.pos += 1;
            if c == b']' {
                return Ok(());
            }
        }
    }

    fn consume_balanced_paren(&mut self) -> Result<(), String> {
        self.consume_balanced_paren_inner(false).map(|_| ())
    }

    /// Consumes to the matching `)`, optionally returning the inner text.
    fn consume_balanced_paren_inner(&mut self, capture: bool) -> Result<String, String> {
        // Position is at the opening paren.
        let mut depth: usize = 1;
        let mut in_squote = false;
        let mut in_dq = false;
        let mut dq_floor: Vec<usize> = Vec::new();
        self.pos += 1;
        let inner_start = self.pos;
        loop {
            let Some(c) = self.peek() else {
                return Err("unbalanced parenthesis".to_string());
            };
            if in_squote {
                if c == b'\'' {
                    in_squote = false;
                }
                self.pos += 1;
                continue;
            }
            match c {
                b'\\' => {
                    self.pos += 1;
                    if self.peek().is_some() {
                        self.pos += 1;
                    }
                }
                b'\'' if !in_dq => {
                    in_squote = true;
                    self.pos += 1;
                }
                b'"' => {
                    if in_dq {
                        dq_floor.pop();
                        in_dq = !dq_floor.is_empty();
                    } else {
                        dq_floor.push(depth);
                        in_dq = true;
                    }
                    self.pos += 1;
                }
                b'(' => {
                    if !in_dq || (self.pos > 0 && self.src[self.pos - 1] == b'$') {
                        depth += 1;
                    }
                    self.pos += 1;
                }
                b')' => {
                    let closes = if in_dq {
                        depth > *dq_floor.last().unwrap_or(&0)
                    } else {
                        true
                    };
                    if closes {
                        depth -= 1;
                        if depth == 0 {
                            let inner_end = self.pos;
                            self.pos += 1;
                            return Ok(if capture {
                                self.text[inner_start..inner_end].to_string()
                            } else {
                                String::new()
                            });
                        }
                    }
                    self.pos += 1;
                }
                _ => self.pos += 1,
            }
        }
    }

    fn consume_backquote(&mut self) -> Result<(), String> {
        loop {
            let Some(c) = self.peek() else {
                return Err("unterminated backquote".to_string());
            };
            match c {
                b'\\' => {
                    self.pos += 1;
                    if self.peek().is_some() {
                        self.pos += 1;
                    }
                }
                b'`' => {
                    self.pos += 1;
                    return Ok(());
                }
                _ => self.pos += 1,
            }
        }
    }

    fn consume_heredoc_bodies(&mut self, pending: &mut Vec<(String, bool)>) -> Result<(), String> {
        for (delim, strip) in pending.drain(..) {
            loop {
                if self.pos >= self.src.len() {
                    return Err(format!("unclosed here-doc '{delim}'"));
                }
                let nl = self.src[self.pos..].iter().position(|&b| b == b'\n');
                let line_end = match nl {
                    Some(i) => self.pos + i,
                    None => self.src.len(),
                };
                let mut line = &self.src[self.pos..line_end];
                if strip {
                    while line.first() == Some(&b'\t') {
                        line = &line[1..];
                    }
                }
                let matched = line == delim.as_bytes();
                self.pos = if nl.is_some() { line_end + 1 } else { line_end };
                if matched {
                    break;
                }
            }
        }
        Ok(())
    }
}

fn flush_lit(parts: &mut Vec<WordPart>, lit: &mut String) {
    if !lit.is_empty() {
        parts.push(WordPart::Lit(std::mem::take(lit)));
    }
}

const RESERVED: &[&str] = &[
    "{", "}", "if", "then", "elif", "else", "fi", "for", "while", "until", "do", "done", "case",
    "esac", "function", "select", "coproc", "[[", "]]", "!",
];

fn is_reserved(parts: &[WordPart]) -> bool {
    match parts {
        [WordPart::Lit(s)] => RESERVED.contains(&s.as_str()),
        _ => false,
    }
}

fn is_param_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'@' | b'*' | b'#' | b'?' | b'-' | b'$' | b'!')
}

fn is_special_param(c: u8) -> bool {
    matches!(c, b'@' | b'*' | b'#' | b'?' | b'-' | b'$' | b'!')
}

enum AssignShape {
    Simple {
        name: String,
        value_parts: Vec<WordPart>,
    },
    Reject,
    Word,
}

/// Whether a word's parts spell a `NAME=value` assignment (the analysis
/// module reads this where Go inspects `Assign` nodes).
pub(crate) fn word_is_assignment(parts: &[WordPart]) -> bool {
    matches!(assignment_shape(parts), AssignShape::Simple { .. })
}

fn assignment_shape(parts: &[WordPart]) -> AssignShape {
    let [WordPart::Lit(head), rest @ ..] = parts else {
        return AssignShape::Word;
    };
    let Some(eq) = head.find('=') else {
        return AssignShape::Word;
    };
    let (name_raw, after_eq) = head.split_at(eq);
    let after_eq = &after_eq[1..];
    let (name, append) = match name_raw.strip_suffix('+') {
        Some(n) => (n, true),
        None => (name_raw, false),
    };
    let is_array = match name.find('[') {
        Some(open) => {
            if !name.ends_with(']') || open + 2 > name.len() {
                return AssignShape::Word;
            }
            let (base, _) = name.split_at(open);
            if !is_valid_ident(base) {
                return AssignShape::Word;
            }
            true
        }
        None => {
            if !is_valid_ident(name) {
                return AssignShape::Word;
            }
            false
        }
    };
    if is_array || append {
        return AssignShape::Reject;
    }
    let mut value_parts: Vec<WordPart> = Vec::new();
    if !after_eq.is_empty() {
        value_parts.push(WordPart::Lit(after_eq.to_string()));
    }
    value_parts.extend(rest.iter().cloned());
    AssignShape::Simple {
        name: name.to_string(),
        value_parts,
    }
}

fn is_valid_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Concatenates the static parts of a word (reference: `StaticWord`).
/// Returns `None` when the word contains any expansion.
pub fn static_word(parts: &[WordPart]) -> Option<String> {
    let mut out = String::new();
    for part in parts {
        out.push_str(&static_word_part(part, false)?);
    }
    Some(out)
}

fn static_word_part(part: &WordPart, in_double_quotes: bool) -> Option<String> {
    match part {
        WordPart::Lit(raw) => Some(unescape_lit(raw, in_double_quotes)),
        WordPart::SglQuoted(v) => Some(v.clone()),
        WordPart::DblQuoted(parts) => {
            let mut out = String::new();
            for p in parts {
                out.push_str(&static_word_part(p, true)?);
            }
            Some(out)
        }
        WordPart::Expansion | WordPart::CmdSubst(_) => None,
    }
}

/// Interprets escape sequences in an unquoted literal (reference:
/// `unescapeLit`). Outside quotes a backslash quotes the next byte; in
/// double quotes only dollar, backquote, double quote, and backslash are
/// special and the backslash is kept otherwise; backslash-newline is a line
/// continuation.
fn unescape_lit(s: &str, in_double_quotes: bool) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut it = s.char_indices().peekable();
    while let Some((_, c)) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.peek() {
            None => out.push('\\'),
            Some((_, '\n')) => {
                it.next();
            }
            Some((_, n)) => {
                if !in_double_quotes || matches!(n, '$' | '`' | '"' | '\\') {
                    out.push(*n);
                    it.next();
                } else {
                    out.push('\\');
                }
            }
        }
    }
    out
}

fn static_redirections(
    redirs: &[RawRedir],
    policy: StaticCommandPolicy,
) -> Result<bool, StaticRejectError> {
    let mut merge_stderr = false;
    for redir in redirs {
        if !policy.allow_stderr_to_stdout || !redir.is_stderr_to_stdout() || merge_stderr {
            return Err(StaticRejectError::new(StaticReason::Redirection));
        }
        merge_stderr = true;
    }
    Ok(merge_stderr)
}

/// Parses a single static command (reference: `ParseStaticCommand`).
pub fn parse_static_command(
    command: &str,
    policy: StaticCommandPolicy,
) -> Result<StaticCommand, StaticRejectError> {
    if command.trim().is_empty() {
        return Ok(StaticCommand::default());
    }
    let parsed = parse(command)
        .map_err(|detail| StaticRejectError::with_detail(StaticReason::Parse, detail))?;
    if parsed.heredoc {
        return Err(StaticRejectError::new(StaticReason::HereDoc));
    }
    if parsed.stmts.len() != 1 {
        return Err(StaticRejectError::new(StaticReason::Control));
    }
    let stmt = &parsed.stmts[0];
    if stmt.ctrl || stmt.background {
        return Err(StaticRejectError::new(StaticReason::Control));
    }
    let merge_stderr = static_redirections(&stmt.redirs, policy)?;
    let mut env: Vec<String> = Vec::new();
    let mut idx = 0;
    while idx < stmt.words.len() {
        match assignment_shape(&stmt.words[idx]) {
            AssignShape::Word => break,
            AssignShape::Reject => return Err(StaticRejectError::new(StaticReason::Assignment)),
            AssignShape::Simple { name, value_parts } => {
                if !policy.allow_env_assignments {
                    return Err(StaticRejectError::new(StaticReason::Assignment));
                }
                let value = static_word(&value_parts)
                    .ok_or_else(|| StaticRejectError::new(StaticReason::Expansion))?;
                env.push(format!("{name}={value}"));
                idx += 1;
            }
        }
    }
    let mut argv: Vec<String> = Vec::new();
    for word in &stmt.words[idx..] {
        argv.push(
            static_word(word).ok_or_else(|| StaticRejectError::new(StaticReason::Expansion))?,
        );
    }
    if argv.is_empty() && !env.is_empty() {
        return Err(StaticRejectError::with_detail(
            StaticReason::Assignment,
            "shell assignment without command",
        ));
    }
    Ok(StaticCommand {
        argv,
        env,
        merge_stderr,
    })
}

/// Splits a static command into argument fields (reference: `StaticFields`).
/// Returns the argv and an empty message on success, or no fields and a
/// rejection message when the command is not a single static command.
pub fn static_fields(command: &str) -> (Vec<String>, String) {
    match parse_static_command(command, StaticCommandPolicy::default()) {
        Ok(cmd) => (cmd.argv, String::new()),
        Err(reject) => (Vec::new(), static_fields_message(&reject)),
    }
}

fn static_fields_message(reject: &StaticRejectError) -> String {
    match reject.reason {
        StaticReason::Parse => reject.to_string(),
        StaticReason::HereDoc => "here document".to_string(),
        StaticReason::Expansion => "shell expansion".to_string(),
        _ => "shell control syntax".to_string(),
    }
}

/// Reports whether the command contains shell syntax beyond a single static
/// command (reference: `ContainsShellSyntax`).
pub fn contains_shell_syntax(command: &str) -> bool {
    if command.trim().is_empty() {
        return false;
    }
    !static_fields(command).1.is_empty()
}

/// Reports whether the parsed command contains a here-document (reference:
/// `HasHereDoc` combined with a parse). Returns `None` when the
/// input fails to parse.
pub fn heredoc_present(command: &str) -> Option<bool> {
    parse(command).ok().map(|parsed| parsed.heredoc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_fields_table() {
        let ok: &[(&str, &[&str])] = &[
            ("git status --short", &["git", "status", "--short"]),
            (
                "grep 'a|b' \"file name.txt\"",
                &["grep", "a|b", "file name.txt"],
            ),
            (
                "find . -name scratch \\-delete",
                &["find", ".", "-name", "scratch", "-delete"],
            ),
        ];
        for (command, want) in ok {
            let (got, malformed) = static_fields(command);
            assert_eq!(malformed, "", "StaticFields({command:?})");
            assert_eq!(got, want.to_vec(), "StaticFields({command:?})");
        }
        let malformed_cases = [
            "git log >/dev/null",
            "git status && rm -rf /tmp/x",
            "git diff $REV",
            "echo $(touch out)",
            "GIT_EXTERNAL_DIFF=cat git diff",
            "echo 'unterminated",
        ];
        for command in malformed_cases {
            let (got, malformed) = static_fields(command);
            assert_ne!(
                malformed, "",
                "StaticFields({command:?}) malformed = empty, want error"
            );
            assert!(got.is_empty(), "StaticFields({command:?}) argv not empty");
        }
    }

    #[test]
    fn parse_static_command_policy() {
        let got = parse_static_command(
            "FOO=bar MESSAGE='hello world' go test ./...",
            StaticCommandPolicy {
                allow_env_assignments: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(got.env, ["FOO=bar", "MESSAGE=hello world"]);
        assert_eq!(got.argv, ["go", "test", "./..."]);

        let err =
            parse_static_command("FOO=bar go test", StaticCommandPolicy::default()).unwrap_err();
        assert_eq!(err.reason, StaticReason::Assignment);

        let err = parse_static_command(
            "FOO=$(whoami) go test",
            StaticCommandPolicy {
                allow_env_assignments: true,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(err.reason, StaticReason::Expansion);

        let err = parse_static_command(
            "go test ./... >out.txt",
            StaticCommandPolicy {
                allow_env_assignments: true,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(err.reason, StaticReason::Redirection);

        let got = parse_static_command(
            "go test ./... 2>&1",
            StaticCommandPolicy {
                allow_stderr_to_stdout: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(got.merge_stderr);
        assert_eq!(got.argv, ["go", "test", "./..."]);

        let err = parse_static_command(
            "go test ./... 2>err.txt",
            StaticCommandPolicy {
                allow_stderr_to_stdout: true,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(err.reason, StaticReason::Redirection);

        let (_, malformed) = static_fields("FOO=bar go test");
        assert_eq!(malformed, "shell control syntax");
        let (_, malformed) = static_fields("go test 2>&1");
        assert_eq!(malformed, "shell control syntax");
    }

    #[test]
    fn contains_shell_syntax_table() {
        for command in [
            "git status && rm -rf /",
            "cat a | tee b",
            "git status > out.txt",
            "echo $(rm x)",
            "echo $HOME",
            "echo `whoami`",
            "sleep 1 &",
        ] {
            assert!(
                contains_shell_syntax(command),
                "ContainsShellSyntax({command:?}) = false, want true"
            );
        }
        for command in [
            "git status",
            "grep 'a|b' file",
            "printf \"%s\\n\" \"a && b\"",
            "find . -name scratch \\-delete",
        ] {
            assert!(
                !contains_shell_syntax(command),
                "ContainsShellSyntax({command:?}) = true, want false"
            );
        }
    }

    #[test]
    fn heredoc_detected_in_parsed_command() {
        let command = "cat <<'EOF'\nnohup sleep 60 >/dev/null 2>&1 &\nEOF";
        assert_eq!(heredoc_present(command), Some(true));
    }

    #[test]
    fn heredoc_variants() {
        assert_eq!(heredoc_present("cat <<EOF\nx\nEOF"), Some(true));
        assert_eq!(heredoc_present("cat <<-EOF\n\tx\n\tEOF"), Some(true));
        assert_eq!(heredoc_present("cat <<EOF\nx"), None);
        assert_eq!(heredoc_present("cat <<EOF"), None);
        assert_eq!(heredoc_present("cat file"), Some(false));
        let err = parse_static_command("cat <<EOF\nbody\nEOF", StaticCommandPolicy::default())
            .unwrap_err();
        assert_eq!(err.reason, StaticReason::HereDoc);
        let (_, malformed) = static_fields("cat <<EOF\nbody\nEOF");
        assert_eq!(malformed, "here document");
    }

    #[test]
    fn extglob_and_brace_words() {
        // extglob makes the word non-static
        let (_, malformed) = static_fields("printf '%s\\n' @(a|b)");
        assert_eq!(malformed, "shell expansion");
        // braces stay literal in the bash grammar
        let (argv, malformed) = static_fields("printf '%s\\n' {a,b}");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["printf", "%s\\n", "{a,b}"]);
    }

    #[test]
    fn ansic_and_locale_strings() {
        let (argv, malformed) = static_fields("echo $'a\\tb'");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["echo", "a\tb"]);
        let (argv, malformed) = static_fields("echo $\"x\"");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["echo", "x"]);
        // $' and $" are not special inside double quotes
        let (argv, malformed) = static_fields("echo \"x$'y'\"");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["echo", "x$'y'"]);
    }

    #[test]
    fn dollar_edge_cases() {
        let (argv, malformed) = static_fields("echo $ x");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["echo", "$", "x"]);
        let (argv, malformed) = static_fields("echo a$");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["echo", "a$"]);
        let (_, malformed) = static_fields("echo $((1+1))");
        assert_eq!(malformed, "shell expansion");
        let (_, malformed) = static_fields("echo $[1+1]");
        assert_eq!(malformed, "shell expansion");
        let (_, malformed) = static_fields("echo ${HOME}");
        assert_eq!(malformed, "shell expansion");
        let (_, malformed) = static_fields("echo $1");
        assert_eq!(malformed, "shell expansion");
        let (_, malformed) = static_fields("echo $$");
        assert_eq!(malformed, "shell expansion");
    }

    #[test]
    fn procsubst_is_opaque_word() {
        let (_, malformed) = static_fields("diff <(a) <(b)");
        assert_eq!(malformed, "shell expansion");
        let (_, malformed) = static_fields("diff >(a)");
        assert_eq!(malformed, "shell expansion");
        // quotes and nested parens inside the substitution body
        let (_, malformed) = static_fields("diff <(echo \"((\") <(echo x)");
        assert_eq!(malformed, "shell expansion");
    }

    #[test]
    fn io_numbers_and_merge() {
        let got = parse_static_command(
            "2>&1 go test",
            StaticCommandPolicy {
                allow_stderr_to_stdout: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(got.merge_stderr);
        assert_eq!(got.argv, ["go", "test"]);
        let (argv, malformed) = static_fields("echo 2 x");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["echo", "2", "x"]);
        // digits glued to a word are not an io number
        let (argv, malformed) = static_fields("echo 2x>log hi");
        assert_eq!(malformed, "shell control syntax");
        assert!(argv.is_empty());
    }

    #[test]
    fn reserved_words_and_negation() {
        let (_, malformed) = static_fields("if true");
        assert_eq!(malformed, "shell control syntax");
        // quoted reserved word is a plain command word
        let (argv, malformed) = static_fields("\"if\" true");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["if", "true"]);
        let (_, malformed) = static_fields("! ls");
        assert_eq!(malformed, "shell control syntax");
        // negation marker as an argument is fine
        let (argv, malformed) = static_fields("echo !");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["echo", "!"]);
        let (_, malformed) = static_fields("[[ -f x ]]");
        assert_eq!(malformed, "shell control syntax");
    }

    #[test]
    fn comments_and_separators() {
        let (argv, malformed) = static_fields("git status # comment");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["git", "status"]);
        let (argv, malformed) = static_fields("git status;");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["git", "status"]);
        // comment-only input parses to zero statements
        let (_, malformed) = static_fields("# just a comment");
        assert_eq!(malformed, "shell control syntax");
        // '#' inside a word is literal
        let (argv, malformed) = static_fields("echo a#b");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["echo", "a#b"]);
        let (_, malformed) = static_fields("cd /tmp; ls");
        assert_eq!(malformed, "shell control syntax");
        let (_, malformed) = static_fields("cd /tmp\nls");
        assert_eq!(malformed, "shell control syntax");
        let (_, malformed) = static_fields("a\n\nb");
        assert_eq!(malformed, "shell control syntax");
        let (_, malformed) = static_fields("a;; b");
        assert_ne!(malformed, "");
        let (_, malformed) = static_fields("; a");
        assert_ne!(malformed, "");
    }

    #[test]
    fn subshell_and_parens() {
        let (_, malformed) = static_fields("(ls)");
        assert_eq!(malformed, "shell control syntax");
        let (_, malformed) = static_fields("(ls; pwd)");
        assert_eq!(malformed, "shell control syntax");
        let (_, malformed) = static_fields("echo a(b");
        assert_ne!(malformed, "");
        // parentheses inside quotes are literal
        let (argv, malformed) = static_fields("echo 'x(y)z'");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["echo", "x(y)z"]);
    }

    #[test]
    fn empty_quotes_and_quoted_glue() {
        let (argv, malformed) = static_fields("echo '' x");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["echo", "", "x"]);
        let (argv, malformed) = static_fields("echo a'b'c\"d\"e");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["echo", "abcde"]);
        // backslash-newline continuation inside a word
        let (argv, malformed) = static_fields("git \\\nstatus");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["git", "status"]);
        // continuation between tokens
        let (argv, malformed) = static_fields("git \\\n status");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["git", "status"]);
        // double-quoted backslash keeps other escapes literal
        let (argv, malformed) = static_fields("echo \"a\\qb\"");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["echo", "a\\qb"]);
    }

    #[test]
    fn assignment_variants() {
        let err = parse_static_command(
            "FOO+=x cmd",
            StaticCommandPolicy {
                allow_env_assignments: true,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(err.reason, StaticReason::Assignment);
        let err = parse_static_command(
            "FOO[0]=x cmd",
            StaticCommandPolicy {
                allow_env_assignments: true,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(err.reason, StaticReason::Assignment);
        let err = parse_static_command(
            "FOO=bar",
            StaticCommandPolicy {
                allow_env_assignments: true,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(err.reason, StaticReason::Assignment);
        assert_eq!(err.detail, "shell assignment without command");
        // assignments after the command word are arguments
        let got = parse_static_command(
            "env FOO=bar go",
            StaticCommandPolicy {
                allow_env_assignments: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(got.env.is_empty());
        assert_eq!(got.argv, ["env", "FOO=bar", "go"]);
        // empty assignment value
        let got = parse_static_command(
            "FOO= go",
            StaticCommandPolicy {
                allow_env_assignments: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(got.env, ["FOO="]);
        assert_eq!(got.argv, ["go"]);
        // invalid identifier stays a word
        let (argv, malformed) = static_fields("1=x cmd");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["1=x", "cmd"]);
        // quoted assignment value
        let got = parse_static_command(
            "FOO='a b' go",
            StaticCommandPolicy {
                allow_env_assignments: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(got.env, ["FOO=a b"]);
    }

    #[test]
    fn redirects_table() {
        let (_, malformed) = static_fields("cat <<< hi");
        assert_eq!(malformed, "shell control syntax");
        let (_, malformed) = static_fields("cat < in");
        assert_eq!(malformed, "shell control syntax");
        let (_, malformed) = static_fields("cat <&0");
        assert_eq!(malformed, "shell control syntax");
        let (_, malformed) = static_fields("go test &> log");
        assert_eq!(malformed, "shell control syntax");
        let (_, malformed) = static_fields("go test &>> log");
        assert_eq!(malformed, "shell control syntax");
        let err =
            parse_static_command("go test >$TMP/log", StaticCommandPolicy::default()).unwrap_err();
        assert_eq!(err.reason, StaticReason::Redirection);
        // missing target is a parse error
        let (_, malformed) = static_fields("go test >");
        assert_ne!(malformed, "");
    }

    #[test]
    fn whitespace_and_edges() {
        assert!(!contains_shell_syntax(""));
        assert!(!contains_shell_syntax("   "));
        let (argv, malformed) = static_fields("   ");
        assert_eq!(malformed, "");
        assert!(argv.is_empty());
        // \r is a word byte, not whitespace
        let (argv, malformed) = static_fields("echo a\r");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["echo", "a\r"]);
        // non-ASCII words survive
        let (argv, malformed) = static_fields("echo café");
        assert_eq!(malformed, "");
        assert_eq!(argv, ["echo", "café"]);
        // backslash at EOF is a parse error
        let (_, malformed) = static_fields("git status \\");
        assert_ne!(malformed, "");
        // multiple heredocs on one line
        assert_eq!(heredoc_present("cat <<A <<B\na\nA\nb\nB"), Some(true));
    }

    #[test]
    fn reject_reason_strings() {
        assert_eq!(StaticReason::Parse.as_str(), "parse error");
        assert_eq!(StaticReason::HereDoc.as_str(), "here document");
        assert_eq!(StaticReason::Control.as_str(), "shell control syntax");
        assert_eq!(StaticReason::Redirection.as_str(), "shell redirection");
        assert_eq!(StaticReason::Assignment.as_str(), "shell assignment");
        assert_eq!(StaticReason::Expansion.as_str(), "shell expansion");
        let err = StaticRejectError::new(StaticReason::Control);
        assert_eq!(err.to_string(), "shell control syntax");
    }
}
