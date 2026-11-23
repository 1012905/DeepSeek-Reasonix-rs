//! Credential redaction for text that crosses a diagnostic boundary.
//!
//! Parity target: `internal/base/secrets.RedactCredentials`, the stronger
//! scrub the host applies to sidecar stderr, peer error messages, and startup
//! failures. The workspace already owns the credential vocabulary and endpoint
//! projection in `reasonix-core::secrets`, so this module composes those rather
//! than declaring a second table: a key shape one side calls a credential and
//! the other does not is exactly the drift that leaks.
//!
//! A sentence stays readable; only values a credential carrier names are
//! masked, matching the Go pass (which also refuses to re-redact its own mask).

use reasonix_core::secrets::{credential_key, redact_endpoint};

/// The mask a redacted value becomes.
pub const REDACTED: &str = "[redacted]";
/// The mask an opaque or already-partially-masked token becomes.
pub const MASK: &str = "****";
/// How long a bare token must be before the final fallback looks at it.
const OPAQUE_MIN_LEN: usize = 16;
/// How long a token after a credential word must be before it is masked.
const PROSE_MIN_LEN: usize = 12;

/// Redacts credential-shaped values from free text.
///
/// Four passes, in the order the Go oracle uses: endpoints first, so a URL's
/// userinfo cannot survive as a bare token; then `key=value` / `key: value`
/// pairs whose key names a credential; then prose forms (`api key: <token>`)
/// whose credential word is separated only by space; then a conservative
/// fallback for opaque runs. Every pass leaves an existing mask alone, so the
/// function is idempotent.
pub fn redact_credentials(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    let endpoints = redact_endpoints(input);
    let pairs = redact_credential_pairs(&endpoints);
    let prose = redact_prose_forms(&pairs);
    let bearer = redact_bearer(&prose);
    let masked = redact_masked_runs(&bearer);
    redact_opaque_runs(&masked)
}

/// Redacts an error's rendered text for an external log or diagnostic.
pub fn redact_error_text(message: &str) -> String {
    redact_credentials(message.trim())
}

/// Replaces every absolute URL with its credential-projected form.
fn redact_endpoints(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = find_scheme(rest) {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\''))
            .unwrap_or(tail.len());
        let (url, remainder) = tail.split_at(end);
        // Trailing sentence punctuation belongs to the prose, not the URL.
        let trimmed = url.trim_end_matches([',', '.', ')', ';', ':']);
        let punctuation = &url[trimmed.len()..];
        out.push_str(&redact_endpoint(trimmed));
        out.push_str(punctuation);
        rest = remainder;
    }
    out.push_str(rest);
    out
}

/// Finds the start of the next `scheme://` run, if any.
fn find_scheme(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let starts_scheme =
            bytes[index].is_ascii_lowercase() && (index == 0 || !is_token_byte(bytes[index - 1]));
        if starts_scheme {
            let mut cursor = index;
            while cursor < bytes.len()
                && (bytes[cursor].is_ascii_alphanumeric()
                    || matches!(bytes[cursor], b'+' | b'.' | b'-'))
            {
                cursor += 1;
            }
            if text[cursor..].starts_with("://") {
                return Some(index);
            }
        }
        index += 1;
    }
    None
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
}

/// Masks the value of every `key=value`, `key: value`, or `key="value"` pair
/// whose key names a credential.
fn redact_credential_pairs(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;
    let mut last = 0;
    while cursor < bytes.len() {
        if !matches!(bytes[cursor], b'=' | b':') {
            cursor += 1;
            continue;
        }
        let Some(key) = read_pair_key(input, cursor) else {
            cursor += 1;
            continue;
        };
        if !credential_key(key) {
            cursor += 1;
            continue;
        }
        let Some((value_start, value_end)) = read_pair_value(input, cursor + 1) else {
            cursor += 1;
            continue;
        };
        out.push_str(&input[last..value_start]);
        out.push_str(&mask_replacement(&input[value_start..value_end], REDACTED));
        last = value_end;
        cursor = value_end;
    }
    if last == 0 {
        return input.to_string();
    }
    out.push_str(&input[last..]);
    out
}

/// Masks a token that follows a credential word with no separator at all
/// (`provider rejected api key: relaykey...` is covered by the pair pass; this
/// covers `rejected token ****ae54` and `api key hunter2-swordfish`).
fn redact_prose_forms(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut tokens: Vec<(usize, usize)> = Vec::new();
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if is_token_byte(bytes[index]) {
            let start = index;
            while index < bytes.len() && is_token_byte(bytes[index]) {
                index += 1;
            }
            tokens.push((start, index));
        } else {
            index += 1;
        }
    }
    let mut last = 0;
    for (position, &(start, end)) in tokens.iter().enumerate() {
        if !credential_key(&input[start..end]) {
            continue;
        }
        let Some(&(value_start, value_end)) = tokens.get(position + 1) else {
            continue;
        };
        // Only words (not a following sentence) and only when the gap between
        // them holds nothing but space or a single separator character.
        let gap = &input[end..value_start];
        let trimmed_gap = gap.trim();
        if !trimmed_gap.is_empty() && !matches!(trimmed_gap, ":" | "=" | "\"" | "'") {
            continue;
        }
        let value = &input[value_start..value_end];
        if !is_prose_credential(value) {
            continue;
        }
        out.push_str(&input[last..value_start]);
        out.push_str(&mask_replacement(value, MASK));
        last = value_end;
    }
    if last == 0 {
        return input.to_string();
    }
    out.push_str(&input[last..]);
    out
}

fn is_prose_credential(value: &str) -> bool {
    if value.contains(MASK) {
        return true;
    }
    value.len() >= PROSE_MIN_LEN && !is_common_word(value)
}

/// Keeps prose from being shredded: a long ordinary word after a credential
/// word is still a word, but a credential shaped one carries digits or case
/// churn or an underscore.
fn is_common_word(value: &str) -> bool {
    let has_underscore = value.contains('_');
    let digit = value.bytes().any(|b| b.is_ascii_digit());
    let upper = value.bytes().any(|b| b.is_ascii_uppercase());
    let lower = value.bytes().any(|b| b.is_ascii_lowercase());
    !(has_underscore || digit || (upper && lower))
}

/// Masks a token carried by a `Bearer` scheme, mirroring the oracle's
/// bearer pattern: the scheme word, whitespace, then a 16+ char token.
fn redact_bearer(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if is_bearer_token_byte(bytes[index]) {
            let start = index;
            while index < bytes.len() && is_bearer_token_byte(bytes[index]) {
                index += 1;
            }
            spans.push((start, index));
        } else {
            index += 1;
        }
    }
    let mut out = String::with_capacity(input.len());
    let mut last = 0;
    for (position, &(start, end)) in spans.iter().enumerate() {
        if !input[start..end].eq_ignore_ascii_case("bearer") {
            continue;
        }
        let Some(&(value_start, value_end)) = spans.get(position + 1) else {
            continue;
        };
        let gap = &input[end..value_start];
        if !gap.bytes().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        let value = &input[value_start..value_end];
        if value.len() < 16 {
            continue;
        }
        out.push_str(&input[last..value_start]);
        out.push_str(MASK);
        last = value_end;
    }
    if last == 0 {
        return input.to_string();
    }
    out.push_str(&input[last..]);
    out
}

fn is_bearer_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'~' | b'+' | b'/' | b'-' | b'=')
}

/// Collapses a partially masked credential and any surviving prefix or
/// suffix around its stars, the way the oracle's masked-credential pattern
/// does: `****ae54` and `sk-ab****` leave no fragment a reader could use.
fn redact_masked_runs(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut index = 0;
    let mut last = 0;
    while index < bytes.len() {
        if bytes[index] != b'*' {
            index += 1;
            continue;
        }
        let star_start = index;
        while index < bytes.len() && bytes[index] == b'*' {
            index += 1;
        }
        if index - star_start < 2 {
            continue;
        }
        let mut start = star_start;
        while start > last && is_mask_edge(bytes[start - 1]) {
            start -= 1;
        }
        let mut end = index;
        while end < bytes.len() && is_mask_edge(bytes[end]) {
            end += 1;
        }
        out.push_str(&input[last..start]);
        out.push_str(MASK);
        last = end;
        index = end;
    }
    if last == 0 {
        return input.to_string();
    }
    out.push_str(&input[last..]);
    out
}

fn is_mask_edge(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')
}

/// Masks opaque key-shaped runs: long and either digit-bearing or mixed case.
fn redact_opaque_runs(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0;
    let mut last = 0;
    while index < bytes.len() {
        if !is_token_byte(bytes[index]) {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && is_token_byte(bytes[index]) {
            index += 1;
        }
        if !should_mask_run(&input[start..index]) {
            continue;
        }
        out.push_str(&input[last..start]);
        out.push_str(MASK);
        last = index;
    }
    if last == 0 {
        return input.to_string();
    }
    out.push_str(&input[last..]);
    out
}

fn should_mask_run(run: &str) -> bool {
    if run.len() < OPAQUE_MIN_LEN || run.contains(MASK) {
        return false;
    }
    let digit = run.bytes().any(|b| b.is_ascii_digit());
    let upper = run.bytes().any(|b| b.is_ascii_uppercase());
    let lower = run.bytes().any(|b| b.is_ascii_lowercase());
    digit || (upper && lower)
}

fn mask_replacement(value: &str, fallback: &'static str) -> String {
    if value.contains(MASK) {
        // A partially masked credential's surviving prefix and suffix are the
        // parts a reader could use, so the whole token collapses.
        return MASK.to_string();
    }
    if value == REDACTED || value.starts_with("<redacted>") {
        return value.to_string();
    }
    fallback.to_string()
}

/// Reads the key that ends at a separator, skipping surrounding ASCII space
/// and one optional quote on each side.
fn read_pair_key(input: &str, separator: usize) -> Option<&str> {
    let bytes = input.as_bytes();
    let mut end = separator;
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    if end > 0 && matches!(bytes[end - 1], b'"' | b'\'') {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && is_token_byte(bytes[start - 1]) {
        start -= 1;
    }
    (start < end).then(|| &input[start..end])
}

/// Reads the value that follows a separator: leading space and one optional
/// quote are skipped, and the value ends at space, quote, comma, or semicolon.
fn read_pair_value(input: &str, from: usize) -> Option<(usize, usize)> {
    let bytes = input.as_bytes();
    let mut start = from;
    while start < bytes.len() && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    if start < bytes.len() && matches!(bytes[start], b'"' | b'\'') {
        start += 1;
    }
    let mut end = start;
    while end < bytes.len()
        && !bytes[end].is_ascii_whitespace()
        && !matches!(bytes[end], b'"' | b'\'' | b',' | b';')
    {
        end += 1;
    }
    (end > start).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_is_empty() {
        assert_eq!(redact_credentials(""), "");
        assert_eq!(redact_error_text("   "), "");
    }

    #[test]
    fn key_value_credentials_are_masked_but_the_sentence_survives() {
        let redacted =
            redact_credentials("boot failed: api_key=sk-abcdef1234567890SECRETKEY is invalid");
        assert!(
            !redacted.contains("sk-abcdef1234567890SECRETKEY"),
            "{redacted}"
        );
        assert!(redacted.contains("boot failed:"), "{redacted}");
        assert!(redacted.contains(REDACTED), "{redacted}");
    }

    #[test]
    fn prose_forms_and_bearer_tokens_are_masked() {
        for (input, leaked) in [
            (
                "provider rejected api key: relaykey_abcdefghijklmn",
                "relaykey_abcdefghijklmn",
            ),
            (
                "upstream returned Authorization: Bearer abcdef0123456789abcdef",
                "abcdef0123456789abcdef",
            ),
            ("provider rejected token ****ae54", "ae54"),
        ] {
            let redacted = redact_credentials(input);
            assert!(!redacted.contains(leaked), "{input} -> {redacted}");
        }
    }

    #[test]
    fn endpoint_userinfo_is_projected() {
        let redacted =
            redact_credentials("dial https://proxy-user:pa@ss@proxy.example.com:8443: refused");
        assert!(!redacted.contains("proxy-user"), "{redacted}");
        assert!(!redacted.contains("pa@ss"), "{redacted}");
        assert!(redacted.contains("proxy.example.com:8443"), "{redacted}");
    }

    #[test]
    fn redaction_is_idempotent() {
        let input = "DEEPSEEK_API_KEY=sk-real-secret-value-123456\nAuthorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz\nplain PWD=/home/user/project";
        let once = redact_credentials(input);
        assert_eq!(redact_credentials(&once), once);
        assert!(once.contains("PWD=/home/user/project"), "{once}");
    }

    #[test]
    fn ordinary_identifiers_survive() {
        for input in [
            "tools/list returned ReadOnlyHint for server filesystem",
            "intercept timeout after 5000ms",
            "the sidecar exited with status 1",
        ] {
            assert_eq!(redact_credentials(input), input);
        }
    }
}
