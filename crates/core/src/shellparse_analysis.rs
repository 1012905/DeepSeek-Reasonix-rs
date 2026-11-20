//! Statement-level analysis over the bash parse core — the port of the
//! reference `shellparse` consumers the permission layer drives:
//! `SplitTopLevel`, `CompoundLeaves`, `PeelWrappers`, and
//! `AnalyzeApprovalFeatures`. The lexing core (quoting, heredocs, per-statement
//! joins and spans) lives in [`super::shellparse`]; this module reads its
//! output the way the reference reads the mvdan AST.

use super::shellparse::{Join, WordPart, parse, static_word};

/// Why a statement could not be read statically (the reference's
/// `StaticRejectReason`; the core spells the same set).
pub use super::shellparse::StaticReason;

/// The placeholder standing in for an argument whose value is only known at
/// run time (reference: `dynamicArgPlaceholder`). It never occupies argv[0].
pub const DYNAMIC_ARG: &str = "\u{0}dynamic";

pub fn is_dynamic_arg(field: &str) -> bool {
    field == DYNAMIC_ARG
}

/// One simple command the parse read: its source span (byte offsets, the
/// trailing separator excluded), its static argv with dynamic arguments
/// folded to [`DYNAMIC_ARG`], and the word parts per slot for analyses that
/// read quoting (unquoted globs, network-path skeletons).
/// One redirect: (fd, operator, target word parts, source span).
pub type Redirect = (Option<String>, &'static str, Vec<WordPart>, (usize, usize));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leaf {
    pub start: usize,
    pub end: usize,
    pub background: bool,
    pub negated: bool,
    /// Redirections the statement carries.
    pub redirs: Vec<Redirect>,
    /// The statement reads its program from a stdin here-doc (fd unset or 0).
    pub stdin_heredoc: bool,
    pub argv: Vec<String>,
    pub parts: Vec<Vec<WordPart>>,
}

impl Leaf {
    pub fn source<'a>(&self, source: &'a str) -> &'a str {
        let s = self.start.min(source.len());
        let e = self.end.clamp(s, source.len());
        source[s..e].trim()
    }
}

/// The parse lifted into the shape the consumers read: leaves with their
/// spans plus the join that connected each to its successor. Control syntax
/// the core folded into `ctrl` (reserved words, subshells, groups) surfaces
/// as `Some(reason)` markers so callers fail closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lifted {
    pub leaves: Vec<Leaf>,
    pub opaque: Vec<Option<StaticReason>>,
    pub joins: Vec<Join>,
    pub heredoc: bool,
}

/// Parses and lifts. `Err` is a parse failure (reference: `ParseBash`).
pub fn lift(source: &str) -> Result<Lifted, String> {
    let parsed = parse(source)?;
    let mut leaves = Vec::with_capacity(parsed.stmts.len());
    let mut opaque = Vec::with_capacity(parsed.stmts.len());
    let mut joins = Vec::with_capacity(parsed.stmts.len());
    for raw in &parsed.stmts {
        joins.push(raw.join);
        if raw.ctrl {
            opaque.push(Some(StaticReason::Control));
            leaves.push(Leaf {
                start: raw.span.0,
                end: raw.span.1,
                background: raw.background,
                negated: raw.negated,
                redirs: Vec::new(),
                stdin_heredoc: raw.stdin_heredoc,
                argv: Vec::new(),
                parts: Vec::new(),
            });
            continue;
        }
        opaque.push(None);
        let mut argv = Vec::with_capacity(raw.words.len());
        for (i, word) in raw.words.iter().enumerate() {
            match static_word(word) {
                Some(text) => argv.push(text),
                None => {
                    if i == 0 {
                        // Dynamic command name: the statement is not a leaf the
                        // classifier can name; argv stays empty and the
                        // consumers refuse it.
                        argv.push(DYNAMIC_ARG.to_string());
                        break;
                    }
                    argv.push(DYNAMIC_ARG.to_string());
                }
            }
        }
        let truncated = argv.len() < raw.words.len();
        leaves.push(Leaf {
            start: raw.span.0,
            end: raw.span.1,
            background: raw.background,
            negated: raw.negated,
            redirs: raw
                .redirs
                .iter()
                .map(|r| (r.fd.clone(), r.op, r.target.clone(), r.span))
                .collect(),
            stdin_heredoc: raw.stdin_heredoc,
            argv,
            parts: if truncated {
                Vec::new()
            } else {
                raw.words.clone()
            },
        });
    }
    Ok(Lifted {
        leaves,
        opaque,
        joins,
        heredoc: parsed.heredoc,
    })
}

/// The join the reference reads off a `BinaryCmd` whose `X` is the statement
/// at `index`: [`Join::None`] on the last statement.
pub fn join_of(lifted: &Lifted, index: usize) -> Join {
    lifted.joins.get(index).copied().unwrap_or(Join::None)
}

fn has_expansion_word(parts: &[Vec<WordPart>]) -> bool {
    parts
        .iter()
        .flatten()
        .any(|p| matches!(p, WordPart::Expansion | WordPart::CmdSubst(_)))
}

/// `AnalyzeApprovalFeatures` (reference bash.go:215): a single non-background,
/// non-negated statement's static prefix plus the features that change how it
/// can be approved. `None` is the reference's `ok=false`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApprovalFeatures {
    pub nested_execution: bool,
    pub expansion: bool,
    pub assignment: bool,
    pub redirection: bool,
    pub stdin_heredoc: bool,
    pub dynamic_command_name: bool,
    pub command_prefix: Vec<String>,
}

pub fn analyze_approval_features(command: &str) -> Option<ApprovalFeatures> {
    let lifted = lift(command).ok()?;
    if lifted.heredoc {
        return None;
    }
    if lifted.leaves.len() != 1 || lifted.opaque.iter().any(|o| o.is_some()) {
        return None;
    }
    let leaf = &lifted.leaves[0];
    if leaf.background || leaf.negated || leaf.parts.is_empty() {
        return None;
    }
    let dynamic_command_name = static_word(&leaf.parts[0]).is_none();
    let expansion = has_expansion_word(&leaf.parts);
    let assignment = leaf
        .parts
        .iter()
        .any(|w| super::shellparse::word_is_assignment(w));
    let command_prefix: Vec<String> = leaf
        .parts
        .iter()
        .map(|word| static_word(word).unwrap_or_else(|| DYNAMIC_ARG.to_string()))
        .collect();
    Some(ApprovalFeatures {
        nested_execution: expansion,
        expansion,
        assignment,
        redirection: !leaf.redirs.is_empty(),
        stdin_heredoc: leaf.stdin_heredoc,
        dynamic_command_name,
        command_prefix,
    })
}

/// `SplitTopLevel` (reference bash.go:441): simple-command segments split at
/// top-level control operators, each kept as its original source text.
/// `None` is the reference's `ok=false` — a shape that must not be
/// decomposed (negation, heredocs, opaque statements, a backgrounded chain).
pub fn split_top_level(command: &str) -> Option<(Vec<String>, bool)> {
    if command.trim().is_empty() {
        return Some((Vec::new(), false));
    }
    let lifted = lift(command).ok()?;
    if lifted.heredoc || lifted.opaque.iter().any(|o| o.is_some()) {
        return None;
    }
    let mut segments = Vec::new();
    let mut split = lifted.leaves.len() > 1;
    for (i, leaf) in lifted.leaves.iter().enumerate() {
        if leaf.negated {
            return None;
        }
        if leaf.background {
            // A backgrounded simple command is its own segment; a background
            // flag reached through a chain refuses decomposition.
            if i > 0 && join_of(&lifted, i - 1) == Join::And {
                return None;
            }
            split = true;
        }
        let text = leaf.source(command);
        if !text.is_empty() {
            segments.push(text.to_string());
        }
    }
    segments.retain(|s| {
        let t = s.trim();
        !t.is_empty() && !t.starts_with('#')
    });
    Some((segments, split))
}

/// `CompoundLeaves` (reference bash.go:620): the static argv of every command
/// a compound statement would run, or the reason one of them was unreadable.
/// `Ok(None)` mirrors the reference's `ok=false` with an empty reason — not a
/// compound at all, the single-command classifier owns the answer.
pub fn compound_leaves(command: &str) -> Result<Option<Vec<Vec<String>>>, StaticReason> {
    let lifted = match lift(command) {
        Ok(l) => l,
        Err(_) => return Err(StaticReason::Parse),
    };
    if lifted.leaves.is_empty() {
        return Ok(None);
    }
    let compound = lifted.leaves.len() > 1 || lifted.leaves.iter().any(|l| l.background);
    let mut out = Vec::new();
    for (i, leaf) in lifted.leaves.iter().enumerate() {
        if leaf.negated || leaf.background {
            return Err(StaticReason::Control);
        }
        if let Some((_, op, ..)) = leaf.redirs.first() {
            if matches!(*op, "<<" | "<<-") {
                return Err(StaticReason::HereDoc);
            }
            return Err(StaticReason::Redirection);
        }
        if let Some(reason) = lifted.opaque[i] {
            return Err(reason);
        }
        if leaf.stdin_heredoc {
            return Err(StaticReason::HereDoc);
        }
        if leaf.argv.is_empty() {
            continue;
        }
        out.push(leaf.argv.clone());
    }
    if !compound || out.is_empty() {
        return Ok(None);
    }
    Ok(Some(out))
}

/// `ContainsUnquotedGlob` (reference bash.go:271): whether any word carries
/// `*`/`?`/`[` outside quotes and escapes. A parse failure reads as true
/// (the reference answers true when it cannot prove otherwise).
pub fn contains_unquoted_glob(command: &str) -> bool {
    let lifted = match lift(command) {
        Ok(l) => l,
        Err(_) => return true,
    };
    for leaf in &lifted.leaves {
        for word in &leaf.parts {
            if lit_has_unescaped_meta(word, b"*?[") {
                return true;
            }
        }
    }
    false
}

/// Scans a word's literal runs for unescaped metacharacters (reference
/// `hasUnescapedMeta`); quoted parts cannot smuggle one in.
fn lit_has_unescaped_meta(parts: &[WordPart], meta: &[u8]) -> bool {
    for part in parts {
        if let WordPart::Lit(value) = part {
            let mut escaped = false;
            for b in value.bytes() {
                if escaped {
                    escaped = false;
                    continue;
                }
                if b == b'\\' {
                    escaped = true;
                    continue;
                }
                if meta.contains(&b) {
                    return true;
                }
            }
        }
    }
    false
}

/// `CanMaskEarlierFailure` (reference bash.go:341): whether a later part of
/// the command can hide an earlier failure. Only `&&` chains are exempt;
/// `Err` is the reference's `ok=false` (unanalyzable).
pub fn can_mask_earlier_failure(command: &str) -> Result<bool, StaticReason> {
    if command.trim().is_empty() {
        return Ok(false);
    }
    let lifted = lift(command).map_err(|_: String| StaticReason::Parse)?;
    if lifted.heredoc {
        return Err(StaticReason::HereDoc);
    }
    for leaf in &lifted.leaves {
        if leaf.negated || leaf.stdin_heredoc {
            return Err(StaticReason::Control);
        }
    }
    for (i, leaf) in lifted.leaves.iter().enumerate() {
        if leaf.background {
            return Ok(true);
        }
        match join_of(&lifted, i) {
            Join::None | Join::And => {}
            // `;`, newline, `||`, `|`, `|&` all let a later stage decide.
            Join::Sequence | Join::Or | Join::Pipe | Join::PipeAll => return Ok(true),
        }
    }
    Ok(false)
}

/// `PeelWrappers` (reference wrapper.go): follows the transparent wrappers in
/// [`WRAPPERS`] down to the program a single simple command runs. `Err(())`
/// mirrors the reference's `ok=false` — compound lists, pipelines, groups and
/// parse errors stay with their per-segment callers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Invocation {
    /// The static argv prefix of the command, then of each command a wrapper
    /// hands the call to; a path-spelled program is named by its base.
    pub layers: Vec<Vec<String>>,
    /// A wrapper whose target cannot be named statically.
    pub opaque: bool,
}

#[derive(Debug, Clone, Copy)]
struct WrapperSpec {
    short_flag: &'static str,
    short_arg: &'static str,
    long_flag: &'static [&'static str],
    long_arg: &'static [&'static str],
    assigns: bool,
    any_equals: bool,
    numeric: bool,
    reports_on: &'static str,
}

const WRAPPERS: &[(&str, WrapperSpec)] = &[
    (
        "env",
        WrapperSpec {
            short_flag: "iv",
            short_arg: "uC",
            long_flag: &["-", "--ignore-environment", "--debug"],
            long_arg: &["--unset", "--chdir"],
            assigns: true,
            any_equals: true,
            numeric: false,
            reports_on: "",
        },
    ),
    (
        "sudo",
        WrapperSpec {
            short_flag: "AbBEHknPS",
            short_arg: "ugphCDRTrtU",
            long_flag: &[
                "--askpass",
                "--background",
                "--bell",
                "--preserve-env",
                "--non-interactive",
                "--stdin",
                "--set-home",
                "--reset-timestamp",
            ],
            long_arg: &[
                "--user",
                "--group",
                "--host",
                "--prompt",
                "--close-from",
                "--chdir",
                "--chroot",
                "--role",
                "--type",
                "--other-user",
                "--command-timeout",
            ],
            assigns: true,
            any_equals: false,
            numeric: false,
            reports_on: "",
        },
    ),
    (
        "doas",
        WrapperSpec {
            short_flag: "nL",
            short_arg: "uC",
            long_flag: &[],
            long_arg: &[],
            assigns: false,
            any_equals: false,
            numeric: false,
            reports_on: "",
        },
    ),
    (
        "command",
        WrapperSpec {
            short_flag: "p",
            short_arg: "",
            long_flag: &[],
            long_arg: &[],
            assigns: false,
            any_equals: false,
            numeric: false,
            reports_on: "vV",
        },
    ),
    (
        "builtin",
        WrapperSpec {
            short_flag: "",
            short_arg: "",
            long_flag: &[],
            long_arg: &[],
            assigns: false,
            any_equals: false,
            numeric: false,
            reports_on: "",
        },
    ),
    (
        "exec",
        WrapperSpec {
            short_flag: "cl",
            short_arg: "a",
            long_flag: &[],
            long_arg: &[],
            assigns: false,
            any_equals: false,
            numeric: false,
            reports_on: "",
        },
    ),
    (
        "nohup",
        WrapperSpec {
            short_flag: "",
            short_arg: "",
            long_flag: &[],
            long_arg: &[],
            assigns: false,
            any_equals: false,
            numeric: false,
            reports_on: "",
        },
    ),
    (
        "time",
        WrapperSpec {
            short_flag: "pvalh",
            short_arg: "of",
            long_flag: &["--portability", "--verbose", "--append"],
            long_arg: &["--output", "--format"],
            assigns: false,
            any_equals: false,
            numeric: false,
            reports_on: "",
        },
    ),
    (
        "nice",
        WrapperSpec {
            short_flag: "",
            short_arg: "n",
            long_flag: &[],
            long_arg: &["--adjustment"],
            assigns: false,
            any_equals: false,
            numeric: true,
            reports_on: "",
        },
    ),
];

const MAX_WRAPPER_DEPTH: usize = 16;

/// The word facts `PeelWrappers` reads (reference `argWord`).
#[derive(Debug, Clone)]
struct ArgWord {
    text: String,
    r#static: bool,
    assign: bool,
    equals: bool,
    lead: bool,
    dash: bool,
}

pub fn peel_wrappers(command: &str) -> Result<Invocation, StaticReason> {
    let lifted = lift(command).map_err(|_: String| StaticReason::Parse)?;
    if lifted.leaves.len() != 1 || lifted.opaque.iter().any(|o| o.is_some()) {
        return Err(StaticReason::Control);
    }
    let leaf = &lifted.leaves[0];
    if leaf.background || leaf.negated || leaf.parts.is_empty() {
        return Err(StaticReason::Control);
    }
    let mut words: Vec<ArgWord> = leaf
        .parts
        .iter()
        .map(|word| {
            let text = static_word(word);
            let prefix: String = word
                .iter()
                .map_while(|p| match p {
                    WordPart::Lit(v) => Some(v.clone()),
                    _ => None,
                })
                .collect();
            ArgWord {
                r#static: text.is_some(),
                text: text.clone().unwrap_or_default(),
                assign: prefix.contains('=') && super::shellparse::word_is_assignment(word),
                equals: prefix.find('=').is_some_and(|i| i > 0),
                lead: prefix.starts_with('='),
                dash: prefix.starts_with('-'),
            }
        })
        .collect();
    if words.is_empty() {
        return Err(StaticReason::Control);
    }
    let mut inv = Invocation::default();
    for depth in 0.. {
        if depth >= MAX_WRAPPER_DEPTH {
            inv.opaque = true;
            return Ok(inv);
        }
        if !words[0].r#static {
            inv.opaque = depth > 0;
            return Ok(inv);
        }
        let mut layer: Vec<String> = Vec::new();
        for w in &words {
            if !w.r#static {
                break;
            }
            layer.push(w.text.clone());
        }
        layer[0] = program_name(&layer[0]);
        inv.layers.push(layer);
        let program = inv.layers.last().unwrap()[0].clone();
        let Some((_, spec)) = WRAPPERS.iter().find(|(name, _)| *name == program) else {
            return Ok(inv);
        };
        let (rest, status) = unwrap_words(spec, &words[1..]);
        match status {
            Unwrap::Next => {
                if rest.is_empty() {
                    return Ok(inv);
                }
                words = rest;
            }
            Unwrap::None => return Ok(inv),
            Unwrap::Opaque => {
                inv.opaque = true;
                return Ok(inv);
            }
        }
    }
    unreachable!("wrapper loop always returns");
}

enum Unwrap {
    Next,
    None,
    Opaque,
}

fn unwrap_words(spec: &WrapperSpec, args: &[ArgWord]) -> (Vec<ArgWord>, Unwrap) {
    let mut i = 0usize;
    while i < args.len() {
        let w = &args[i];
        if spec.assigns && !w.dash {
            if w.lead {
                return (Vec::new(), Unwrap::Opaque);
            }
            if w.assign || (w.equals && spec.any_equals) {
                i += 1;
                continue;
            }
            if w.equals {
                return (Vec::new(), Unwrap::Opaque);
            }
        }
        if !w.r#static
            || !w.text.starts_with('-')
            || (w.text == "-" && !spec.long_flag.contains(&"-"))
        {
            return (args[i..].to_vec(), Unwrap::Next);
        }
        let text = w.text.as_str();
        if text == "--" {
            if i + 1 >= args.len() {
                return (Vec::new(), Unwrap::None);
            }
            return (args[i + 1..].to_vec(), Unwrap::Next);
        }
        if let Some(rest) = text.strip_prefix("--") {
            let (name, _, has_value) = split_flag(rest);
            if spec.long_flag.contains(&name) {
                i += 1;
                continue;
            }
            if spec.long_arg.contains(&name) {
                if !has_value {
                    i += 1;
                    if i >= args.len() {
                        return (Vec::new(), Unwrap::Opaque);
                    }
                }
                i += 1;
                continue;
            }
            return (Vec::new(), Unwrap::Opaque);
        }
        if text == "-" {
            i += 1;
            continue;
        }
        if spec.numeric && text[1..].bytes().all(|b| b.is_ascii_digit()) {
            i += 1;
            continue;
        }
        let (consumed, status) = short_cluster(spec, &text[1..], args.len() > i + 1);
        match status {
            Unwrap::Opaque => return (Vec::new(), Unwrap::Opaque),
            Unwrap::None => return (Vec::new(), Unwrap::None),
            Unwrap::Next => {
                i += 1 + consumed;
            }
        }
    }
    (Vec::new(), Unwrap::None)
}

fn split_flag(rest: &str) -> (&str, &str, bool) {
    match rest.split_once('=') {
        Some((name, value)) => (name, value, true),
        None => (rest, "", false),
    }
}

/// Reads one `-abc` cluster and returns how many following words it consumed
/// as option values (reference `shortCluster`).
fn short_cluster(spec: &WrapperSpec, flags: &str, value_follows: bool) -> (usize, Unwrap) {
    for (i, c) in flags.bytes().enumerate() {
        if spec.reports_on.as_bytes().contains(&c) {
            return (0, Unwrap::None);
        }
        if spec.short_flag.as_bytes().contains(&c) {
            continue;
        }
        if spec.short_arg.as_bytes().contains(&c) {
            if i + 1 < flags.len() {
                return (0, Unwrap::Next);
            }
            if !value_follows {
                return (0, Unwrap::Opaque);
            }
            return (1, Unwrap::Next);
        }
        return (0, Unwrap::Opaque);
    }
    (0, Unwrap::Next)
}

/// The program name a layer carries: the path base, `.exe` stripped and case
/// folded for path spellings (reference `programName`).
pub fn program_name(word: &str) -> String {
    let original = word;
    let mut has_path = word.contains(['/', '\\']);
    let mut name = match word.rfind(['/', '\\']) {
        Some(i) => word[i + 1..].to_string(),
        None => word.to_string(),
    };
    if name.len() > 4 && name.len() >= 4 && name[name.len() - 4..].eq_ignore_ascii_case(".exe") {
        name.truncate(name.len() - 4);
        has_path = true;
    }
    if name.is_empty() {
        return original.to_string();
    }
    if has_path { name.to_lowercase() } else { name }
}
