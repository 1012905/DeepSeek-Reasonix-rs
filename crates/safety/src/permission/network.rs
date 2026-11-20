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
    if !cfg!(windows) {
        return false;
    }
    scan_network_spelling(command.as_bytes())
}

fn scan_network_spelling(b: &[u8]) -> bool {
    let mut i = 0;
    while i + 1 < b.len() {
        let two_seps = (b[i] == b'/' || b[i] == b'\\') && (b[i + 1] == b'/' || b[i + 1] == b'\\');
        let nt = b[i] == b'\\' && b[i + 1] == b'?';
        if two_seps || nt {
            // A drive following the separators ("//c:/x") is a local path.
            if let Some(rest) = b.get(i + 2..)
                && looks_like_drive(rest)
            {
                i += 2;
                continue;
            }
            if two_seps {
                return true;
            }
            // \??\ namespace: a drive after \??\ is local.
            if b.get(i + 2) == Some(&b'?')
                && b.get(i + 3).is_some_and(|c| *c == b'\\' || *c == b'/')
            {
                if let Some(rest) = b.get(i + 4..)
                    && looks_like_drive(rest)
                {
                    i += 4;
                    continue;
                }
                return true;
            }
        }
        i += 1;
    }
    false
}

fn looks_like_drive(rest: &[u8]) -> bool {
    rest.len() >= 2 && rest[1] == b':' && rest[0].to_ascii_lowercase().is_ascii_lowercase()
}

/// Returns a copy of subject with redirect syntax removed only when the
/// redirect cannot write to a real file. The static engine proves no
/// redirect safe, so it strips nothing and reports success only for a
/// subject with no redirect spelling at all; callers keep the usual
/// shell-syntax guard as the conservative backstop.
pub fn normalize_bash_safe_redirects_for_match(subject: &str) -> Option<String> {
    if has_redirect_spelling(subject) {
        return None;
    }
    Some(subject.to_string())
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_spellings() {
        assert!(operands_name_network_path(r"cat \\evil\x"));
        assert!(operands_name_network_path("cat //evil/x"));
        assert!(operands_name_network_path("grep -r x //evil/x"));
        assert!(operands_name_network_path(r"Get-Content \\evil\x"));
        assert!(operands_name_network_path(r"\\evil\share\a.exe -v"));
        assert!(!operands_name_network_path("cat C:\\x\\y"));
        assert!(!operands_name_network_path("echo a,b"));
        assert!(!operands_name_network_path("cat a.txt"));
    }

    #[test]
    fn redirects_never_normalize_off_the_static_engine() {
        assert_eq!(
            normalize_bash_safe_redirects_for_match("git log 2>/dev/null"),
            None
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
