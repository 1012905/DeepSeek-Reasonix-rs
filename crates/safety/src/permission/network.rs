//! Network-path and redirect-normalization judgements a bash subject feeds.
//!
//! The reference reads full bash ASTs (`shellsafe.OperandsNameNetworkPath`,
//! `NormalizeBashSafeRedirectsForMatch` walk the mvdan tree). The static
//! engine behind this slice cannot see redirects or nested words, so it
//! answers fail-closed from the spelling: any Windows network-path spelling
//! (`//host/...`, `\\host\...`, `\??\`) in the raw subject names a network
//! path, and no redirect is ever stripped as safe. The shellsafe slice
//! replaces these bodies with the AST ports without changing signatures.

/// Reports whether any operand of a command is or could expand to a Windows
/// network path. Spelling-only: two leading separators and a host, with `/`
/// and `\` both as separators. On a non-Windows host `//x` is a local path,
/// so the answer is no — the host flag is fixed at compile time here, while
/// the Go test toggles it at runtime.
pub fn operands_name_network_path(command: &str) -> bool {
    operands_name_network_path_on(command, cfg!(windows))
}

/// The same answer with the host named, so the oracle test can pin the
/// Windows table on every platform the way the reference's test toggles it
/// at runtime.
fn operands_name_network_path_on(command: &str, windows: bool) -> bool {
    if !windows {
        return false;
    }
    // The reference walks every word of the parse (nested included); a parse
    // failure reads as true (a command nothing can read is not proven local).
    let lifted = match reasonix_core::shellparse_analysis::lift(command) {
        Ok(l) => l,
        Err(_) => return true,
    };
    for leaf in &lifted.leaves {
        for word in &leaf.parts {
            if word_may_be_network_path(word) {
                return true;
            }
        }
    }
    false
}

/// The dynamic placeholder standing in for a word part whose value the host
/// cannot know; it may expand to separators (reference `dynamicPart`).
const DYNAMIC_PART: char = '\u{0}';

/// Renders a word's lexical text with every dynamic part replaced by
/// [`DYNAMIC_PART`] (reference `wordSkeletons`).
fn word_skeletons(parts: &[reasonix_core::shellparse::WordPart]) -> Vec<String> {
    use reasonix_core::shellparse::WordPart;
    const MAX_VARIANTS: usize = 64;
    let mut out = vec![String::new()];
    for part in parts {
        let pieces: Vec<String> = match part {
            WordPart::Lit(v) => vec![v.clone()],
            WordPart::SglQuoted(v) => vec![v.clone()],
            WordPart::DblQuoted(nested) => {
                let inner = word_skeletons(nested);
                let mut next = Vec::with_capacity(out.len() * inner.len());
                for a in &out {
                    for b in &inner {
                        next.push(format!("{a}{b}"));
                    }
                }
                if next.len() > MAX_VARIANTS {
                    vec![DYNAMIC_PART.to_string()]
                } else {
                    next
                }
            }
            WordPart::Expansion | WordPart::CmdSubst(_) => vec![DYNAMIC_PART.to_string()],
        };
        for skeleton in out.iter_mut() {
            let first = pieces.first().cloned().unwrap_or_default();
            skeleton.push_str(&first);
        }
        // The reference appends every alternative; subsequent pieces ride as
        // their own skeletons.
        for piece in pieces.iter().skip(1) {
            out.push(piece.clone());
        }
    }
    out
}

/// The offsets a path may begin at: the word's start and after each `,`
/// or `{` (reference `candidateStarts`).
fn candidate_starts(skeleton: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    let b = skeleton.as_bytes();
    for i in 1..b.len() {
        if b[i - 1] == b'{' || b[i - 1] == b',' {
            starts.push(i);
        }
    }
    starts
}

/// What follows an option name, an `=`, and a PowerShell provider prefix,
/// besides the text itself (reference `operandForms`).
fn operand_forms(text: &str) -> Vec<String> {
    let mut forms = vec![text.to_string()];
    if text.starts_with('-') {
        let b = text.as_bytes();
        let mut i = 0;
        while i < b.len() && b[i] == b'-' {
            i += 1;
        }
        while i < b.len()
            && (b[i] == b'_'
                || b[i] == b'-'
                || b[i].is_ascii_digit()
                || b[i].to_ascii_lowercase().is_ascii_lowercase())
        {
            i += 1;
        }
        if i < b.len() && (b[i] == b'=' || b[i] == b':') {
            i += 1;
        }
        forms.push(text[i..].to_string());
    }
    if let Some((_, after)) = text.split_once('=') {
        forms.push(after.to_string());
    }
    for form in forms.clone() {
        if let Some(rest) = without_provider_prefix(&form) {
            forms.push(rest);
        }
    }
    forms
}

fn without_provider_prefix(text: &str) -> Option<String> {
    let mut rest = text;
    const CORE: &str = r"Microsoft.PowerShell.Core\";
    if rest.len() >= CORE.len() && rest[..CORE.len()].eq_ignore_ascii_case(CORE) {
        rest = &rest[CORE.len()..];
    }
    const FS: &str = "FileSystem::";
    if rest.len() >= FS.len() && rest[..FS.len()].eq_ignore_ascii_case(FS) {
        return Some(rest[FS.len()..].to_string());
    }
    if rest != text {
        Some(rest.to_string())
    } else {
        None
    }
}

fn is_sep(c: u8) -> bool {
    c == b'/' || c == b'\\'
}

/// Tests the leading run of separators and dynamic parts
/// (reference `mayBeNetworkOperand`).
fn may_be_network_operand(operand: &str) -> bool {
    let b = operand.as_bytes();
    let mut seps = 0usize;
    let mut dynamics = 0usize;
    let mut i = 0usize;
    while i < b.len() {
        if is_sep(b[i]) {
            seps += 1;
        } else if b[i] == DYNAMIC_PART as u8 {
            dynamics += 1;
        } else {
            break;
        }
        i += 1;
    }
    if dynamics > 0 {
        return seps >= 1;
    }
    if seps < 2 || i >= b.len() || b[i] == b' ' {
        return false;
    }
    let head: String = operand[..i].replace('/', "\\");
    is_network_path(&format!("{head}{}", &operand[i..]))
}

/// The spelling-only Windows network/device path test
/// (reference `fileutil.IsNetworkPath`).
fn is_network_path(path: &str) -> bool {
    let b = path.as_bytes();
    if let Some(rest) = nt_namespace(b) {
        return !starts_with_drive(rest);
    }
    if b.len() < 2 || !is_sep(b[0]) || !is_sep(b[1]) {
        return false;
    }
    let rest = &b[2..];
    if rest.len() >= 2 && (rest[0] == b'?' || rest[0] == b'.') && is_sep(rest[1]) {
        return !starts_with_drive(&rest[2..]);
    }
    true
}

fn nt_namespace(b: &[u8]) -> Option<&[u8]> {
    if b.len() >= 4 && is_sep(b[0]) && b[1] == b'?' && b[2] == b'?' && is_sep(b[3]) {
        Some(&b[4..])
    } else {
        None
    }
}

fn starts_with_drive(s: &[u8]) -> bool {
    s.len() >= 2
        && s[1] == b':'
        && s[0].to_ascii_lowercase().is_ascii_lowercase()
        && (s.len() == 2 || is_sep(s[2]))
}

fn word_may_be_network_path(parts: &[reasonix_core::shellparse::WordPart]) -> bool {
    for skeleton in word_skeletons(parts) {
        for start in candidate_starts(&skeleton) {
            let tail = &skeleton[start..];
            if operand_forms(tail)
                .iter()
                .any(|f| may_be_network_operand(f))
            {
                return true;
            }
        }
    }
    false
}

/// Returns a copy of subject with redirect syntax removed only when the
/// redirect cannot write to a real file. The static engine proves no
/// redirect safe, so it strips nothing and reports success only for a
/// subject with no redirect spelling at all; callers keep the usual
/// shell-syntax guard as the conservative backstop.
pub fn normalize_bash_safe_redirects_for_match(subject: &str) -> Option<String> {
    // The reference parses and strips only spans whose redirect cannot write
    // to a real file. The spelling gate stays as the fast path; anything with
    // redirect spelling goes through the AST spans.
    if !has_redirect_spelling(subject) {
        return Some(subject.to_string());
    }
    let lifted = reasonix_core::shellparse_analysis::lift(subject).ok()?;
    if lifted.heredoc {
        return None;
    }
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for leaf in &lifted.leaves {
        for (_, op, _, span) in &leaf.redirs {
            // The reference reads the redirect word's raw spelling
            // (redirectWordSource), which survives words the static word
            // walk cannot name ($null and friends).
            let raw = subject
                .get(span.0.min(subject.len())..span.1.min(subject.len()))
                .unwrap_or("");
            if !redirect_is_safe(op, raw) {
                return None;
            }
            spans.push(*span);
        }
        if leaf.stdin_heredoc {
            return None;
        }
    }
    if spans.is_empty() {
        return Some(subject.to_string());
    }
    spans.sort_unstable();
    let mut out = String::with_capacity(subject.len());
    let mut last = 0usize;
    for (start, end) in spans {
        if start < last || end > subject.len() {
            return None;
        }
        out.push_str(&subject[last..start]);
        last = end;
    }
    out.push_str(&subject[last..]);
    Some(out.trim().to_string())
}

/// Whether one redirect cannot write to a real file: fd duplication/close
/// (`2>&1`, `>&2`, `2>&-`, `0<&1`) or output to a null sink
/// (`/dev/null`, `$null`, `nul`) (reference `safeRedirectSpan`).
fn has_redirect_spelling(subject: &str) -> bool {
    let mut quote: Option<char> = None;
    for c in subject.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '\'' | '"' => quote = Some(c),
                '<' | '>' => return true,
                _ => {}
            },
        }
    }
    false
}

fn redirect_is_safe(op: &str, raw: &str) -> bool {
    // The raw spelling is the whole redirect; the value is what follows the
    // operator (and its leading fd), read off the original bytes so
    // expansions like $null keep their spelling.
    let value = raw
        .trim_start_matches(|c: char| c.is_ascii_digit())
        .trim_start_matches(op)
        .trim();
    match op {
        "<&" | ">&" => {
            if value == "-" {
                return true;
            }
            !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
        }
        ">" | ">>" | "&>" | "&>>" | ">|" => {
            value == "/dev/null"
                || value.eq_ignore_ascii_case("$null")
                || value.eq_ignore_ascii_case("nul")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_spellings() {
        // The Windows table pinned through the named-host seam, the way the
        // reference's test toggles the host at runtime.
        let on = |command: &str| operands_name_network_path_on(command, true);
        assert!(on(r"cat \\evil\x"));
        assert!(on("cat //evil/x"));
        assert!(on("grep -r x //evil/x"));
        assert!(on(r"Get-Content \\evil\x"));
        assert!(on(r"\\evil\share\a.exe -v"));
        assert!(!on("cat C:\\x\\y"));
        assert!(!on("echo a,b"));
        assert!(!on("cat a.txt"));
    }

    // The public answer is compile-time fixed: on a non-Windows host `//x`
    // is a local path and nothing reads as network.
    #[test]
    #[cfg(not(windows))]
    fn a_non_windows_host_answers_no() {
        assert!(!operands_name_network_path(r"cat \\evil\x"));
        assert!(!operands_name_network_path("cat //evil/x"));
    }

    #[test]
    fn safe_redirects_normalize_file_redirects_never_do() {
        // The AST engine strips only what cannot write to a real file.
        assert_eq!(
            normalize_bash_safe_redirects_for_match("git log 2>/dev/null"),
            Some("git log".to_string())
        );
        assert_eq!(
            normalize_bash_safe_redirects_for_match("git log > out.txt"),
            None
        );
        assert_eq!(
            normalize_bash_safe_redirects_for_match("git log"),
            Some("git log".to_string())
        );
    }
}
