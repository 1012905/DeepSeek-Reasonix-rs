//! Secrets endpoint projection: redacts credentials from endpoints,
//! configuration values and command arguments before they reach logs,
//! errors or telemetry (reference: internal/base/secrets/endpoint.go).
//!
//! The projection is deliberately conservative: anything that looks like
//! a credential carrier is redacted, and only values that survive the
//! whole pipeline unchanged are passed through. Redaction never widens
//! access; false positives are acceptable, leaks are not.

use std::collections::BTreeMap;

use caseless::default_case_fold_str;
use unicode_general_category::{GeneralCategory, get_general_category};
use unicode_normalization::UnicodeNormalization;

use crate::secrets::url;
use crate::shellparse::{StaticCommandPolicy, parse_static_command};

/// EndpointRedacted is the replacement marker for projected secrets.
pub const ENDPOINT_REDACTED: &str = "<redacted>";

const PROJECTION_DEPTH: usize = 32;

/// Vocabulary of credential-bearing words; a folded word from a key name
/// matching any entry marks the key as a credential.
const CREDENTIAL_WORDS: [&str; 19] = [
    "token",
    "password",
    "passwd",
    "pwd",
    "secret",
    "key",
    "auth",
    "authorization",
    "credential",
    "credentials",
    "signature",
    "sig",
    "session",
    "cookie",
    "bearer",
    "jwt",
    "apikey",
    "access",
    "private",
];

/// CredentialKey reports whether key looks like the name of a credential.
/// Encoded key names are decoded first (failing closed), the name is NFKC
/// normalized with spaces inserted at lower-to-upper boundaries, and the
/// case-folded words are matched against the credential vocabulary. PWD
/// and OLDPWD carry the working directory, not a credential, and are
/// explicitly excluded.
pub fn credential_key(key: &str) -> bool {
    // Query-unescape in a loop: encoded key names must still be
    // recognized, and undecodable input fails closed.
    let mut key = key.to_string();
    let mut depth = 0;
    loop {
        if depth >= PROJECTION_DEPTH {
            return true;
        }
        match url::query_unescape(&key) {
            Err(_) => return true,
            Ok(decoded) => {
                if decoded == key {
                    break;
                }
                key = decoded;
            }
        }
        depth += 1;
    }
    // Insert spaces at lower-to-upper boundaries before folding, so
    // "AccessToken" and "ACCESS_TOKEN" reduce to the same words.
    let normalized: String = key.nfkc().collect();
    let chars: Vec<char> = normalized.chars().collect();
    let mut spaced = String::with_capacity(normalized.len());
    for (i, &r) in chars.iter().enumerate() {
        if is_upper(r) && i > 0 {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|&next| is_lower(next));
            if is_lower(prev) || next_lower {
                spaced.push(' ');
            }
        }
        spaced.push(r);
    }
    let folded = default_case_fold_str(&spaced);
    if folded == "pwd" || folded == "oldpwd" {
        return false;
    }
    folded
        .split(|c: char| !is_letter(c) && !is_digit(c))
        .any(|part| CREDENTIAL_WORDS.contains(&part))
}

/// Exact Go unicode category checks. Rust std's char::is_uppercase /
/// is_alphabetic / is_numeric diverge from Go on Other_Alphabetic and
/// the Nl/No categories, so the general-category crate is used instead.
fn is_upper(c: char) -> bool {
    get_general_category(c) == GeneralCategory::UppercaseLetter
}

fn is_lower(c: char) -> bool {
    get_general_category(c) == GeneralCategory::LowercaseLetter
}

fn is_letter(c: char) -> bool {
    matches!(
        get_general_category(c),
        GeneralCategory::UppercaseLetter
            | GeneralCategory::LowercaseLetter
            | GeneralCategory::TitlecaseLetter
            | GeneralCategory::ModifierLetter
            | GeneralCategory::OtherLetter
    )
}

fn is_digit(c: char) -> bool {
    get_general_category(c) == GeneralCategory::DecimalNumber
}

/// RedactEndpoint projects an endpoint URL, removing credentials from the
/// userinfo, credential-shaped path segments, query parameters and
/// fragments. Values that need no change round-trip byte-exactly.
pub fn redact_endpoint(raw: &str) -> String {
    project_endpoint(raw, 0)
}

fn project_endpoint(raw: &str, depth: usize) -> String {
    if raw.trim().is_empty() || raw == ENDPOINT_REDACTED {
        return raw.to_string();
    }
    if depth >= PROJECTION_DEPTH {
        return ENDPOINT_REDACTED.to_string();
    }
    let trimmed = raw.trim();
    let mut u = match url::parse(trimmed) {
        Ok(u) => u,
        Err(_) => return ENDPOINT_REDACTED.to_string(),
    };
    if u.host.is_empty() || u.scheme.is_empty() || !u.opaque.is_empty() {
        return ENDPOINT_REDACTED.to_string();
    }
    let mut changed = u.user.is_some();
    u.user = None;
    let segments: Vec<&str> = u.path.split('/').collect();
    for (i, segment) in segments.iter().enumerate() {
        if credential_key(segment) && i + 1 < segments.len() {
            // A credential-shaped segment truncates the path: everything
            // after it could be the secret itself.
            u.path = format!("{}/{}", segments[..i + 1].join("/"), ENDPOINT_REDACTED);
            u.raw_path = String::new();
            changed = true;
            break;
        }
    }
    if u.raw_query.contains(';') {
        // ParseQuery rejects semicolons; keep the origin but mask the rest.
        return format!("{}://{}/{}", u.scheme, u.host, ENDPOINT_REDACTED);
    }
    match project_query(&u.raw_query, depth + 1) {
        None => return ENDPOINT_REDACTED.to_string(),
        Some(query) => {
            if query != u.raw_query {
                u.raw_query = query;
                changed = true;
            }
        }
    }
    if !u.fragment.is_empty() {
        let mut fragment = ENDPOINT_REDACTED.to_string();
        if u.fragment.contains('=') {
            match project_query(&u.fragment, depth + 1) {
                None => return ENDPOINT_REDACTED.to_string(),
                Some(f) => fragment = f,
            }
        } else if u.fragment.contains("://") {
            fragment = project_endpoint(&u.fragment, depth + 1);
        }
        if fragment != u.fragment {
            u.fragment = fragment;
            u.raw_fragment = String::new();
            changed = true;
        }
    }
    if !changed {
        return raw.to_string();
    }
    u.to_go_string()
}

/// projectQuery projects query parameters; None means the query could not
/// be parsed safely (the caller then fails closed).
fn project_query(raw: &str, depth: usize) -> Option<String> {
    if raw.is_empty() {
        return Some(raw.to_string());
    }
    if depth >= PROJECTION_DEPTH {
        return Some(ENDPOINT_REDACTED.to_string());
    }
    let query = url::parse_query(raw).ok()?;
    let mut changed = false;
    let mut out = url::Values::new();
    for (key, values) in &query {
        let projected_key = project_value(key, depth + 1);
        for value in values {
            // A credential key always masks its value; the value itself
            // is never projected in that case.
            let mut projected = ENDPOINT_REDACTED.to_string();
            if !credential_key(key) {
                projected = project_value(value, depth + 1);
            }
            if value.is_empty() {
                projected = String::new();
            }
            changed = changed || projected != *value || projected_key != *key;
            out.entry(projected_key.clone())
                .or_default()
                .push(projected);
        }
    }
    if !changed {
        return Some(raw.to_string());
    }
    Some(url::encode_values(&out))
}

/// CredentialValue reports whether value carries a credential anywhere.
pub fn credential_value(value: &str) -> bool {
    redact_config_value("", value) != value
}

/// RedactConfigValue projects a single configuration value; a credential
/// key always masks its value entirely.
pub fn redact_config_value(key: &str, value: &str) -> String {
    if credential_key(key) {
        return ENDPOINT_REDACTED.to_string();
    }
    project_value(value, 0)
}

/// projectValue is the general single-value projection: URL-shaped values
/// project their endpoint, key:nested and key=value shapes check their
/// keys, query-shaped values project their parameters, percent-encoded
/// values recurse on their decoded form, and shell-shaped values are
/// parsed with their argument vector projected. Values that survive every
/// check unchanged round-trip exactly.
fn project_value(value: &str, depth: usize) -> String {
    if value.is_empty() || value == ENDPOINT_REDACTED {
        return value.to_string();
    }
    if depth >= PROJECTION_DEPTH {
        return ENDPOINT_REDACTED.to_string();
    }
    let trimmed = value.trim();
    // URL-shaped values project their endpoint. The "://" must precede
    // any "=" so that query-shaped or key=value values do not match.
    let eq_index = trimmed.find('=').unwrap_or(trimmed.len());
    if trimmed.contains("://")
        && !trimmed.chars().any(char::is_whitespace)
        && trimmed.find("://").is_some_and(|i| i < eq_index)
    {
        return project_endpoint(value, depth + 1);
    }
    // A "key:nested" shape redacts when the key is a credential or the
    // nested value itself changes under projection.
    if let Some((key, nested)) = cut(trimmed, ':')
        && (credential_key(key) || project_value(nested, depth + 1) != nested)
    {
        return ENDPOINT_REDACTED.to_string();
    }
    // An attached "key=value" shape redacts when the key is a credential.
    if let Some((key, _)) = cut(trimmed, '=')
        && credential_key(key)
        && !key.contains("://")
    {
        return ENDPOINT_REDACTED.to_string();
    }
    // Query-shaped values project their parameters.
    if trimmed.contains('=') && !trimmed.chars().any(char::is_whitespace) {
        match project_query(trimmed, depth + 1) {
            None => return ENDPOINT_REDACTED.to_string(),
            Some(projected) => {
                if projected != trimmed {
                    return projected;
                }
            }
        }
    }
    // Percent-encoded values recurse on their decoded form.
    match url::query_unescape(value) {
        Err(_) => return ENDPOINT_REDACTED.to_string(),
        Ok(decoded) => {
            if decoded != value && value.contains('%') {
                let projected = project_value(&decoded, depth + 1);
                if projected != decoded {
                    return projected;
                }
            }
        }
    }
    // Shell-shaped values are parsed and their argv projected; a command
    // that only survives projection unchanged round-trips exactly.
    if trimmed.chars().any(char::is_whitespace) {
        let normalized: String = trimmed
            .chars()
            .map(|r| if r.is_whitespace() { ' ' } else { r })
            .collect();
        let command = match parse_static_command(&normalized, StaticCommandPolicy::default()) {
            Ok(c) => c,
            Err(_) => return ENDPOINT_REDACTED.to_string(),
        };
        if command.argv.len() > 1 {
            let projected = project_args(&command.argv, depth + 1);
            if projected.iter().zip(&command.argv).any(|(p, a)| p != a) {
                return ENDPOINT_REDACTED.to_string();
            }
        }
    }
    value.to_string()
}

/// RedactConfigMap projects a configuration map without mutating the
/// input; None in, None out (Go nil map semantics).
pub fn redact_config_map(
    fields: Option<&BTreeMap<String, String>>,
) -> Option<BTreeMap<String, String>> {
    let fields = fields?;
    let mut out = BTreeMap::new();
    for (key, value) in fields {
        out.insert(key.clone(), redact_config_value(key, value));
    }
    Some(out)
}

/// RedactArgs projects a command argument vector, masking credential
/// carrier flags, headers, environment references and authorization
/// schemes. The input vector is never mutated.
pub fn redact_args(args: &[String]) -> Vec<String> {
    project_args(args, 0)
}

fn project_args(args: &[String], depth: usize) -> Vec<String> {
    let mut out = args.to_vec();
    let mut i = 0;
    while i < out.len() {
        if depth >= PROJECTION_DEPTH {
            out[i] = ENDPOINT_REDACTED.to_string();
            i += 1;
            continue;
        }
        let arg = out[i].clone();
        // Attached short carriers (-eKEY=..., -HHeader:...) are masked
        // after the flag prefix.
        if arg.len() > 2 && (arg.starts_with("-H") || arg.starts_with("-e")) {
            out[i] = format!("{}{}", &arg[..2], ENDPOINT_REDACTED);
            i += 1;
            continue;
        }
        let cut_eq = cut(&arg, '=');
        let key = cut_eq.map_or(arg.as_str(), |(k, _)| k);
        let inline = cut_eq.is_some();
        let carrier =
            key == "-H" || key == "--header" || key == "--headers" || key == "--env" || key == "-e";
        // A trailing bare header name redacts everything from it onward:
        // what follows is the header block.
        if let Some((header, _)) = cut(&arg, ':')
            && credential_key(header)
            && arg.ends_with(':')
        {
            for slot in &mut out[i..] {
                *slot = ENDPOINT_REDACTED.to_string();
            }
            break;
        }
        let flag = key.starts_with('-') && credential_key(key.trim_start_matches('-'));
        // HTTP authentication schemes mask their following token. Full
        // case folding matches Go strings.EqualFold for these targets;
        // the theoretical sharp-s corner would only redact more, never
        // less, which is the safe direction.
        let folded_key = default_case_fold_str(key);
        let scheme = folded_key == "basic" || folded_key == "bearer";
        if carrier || flag || scheme {
            if arg.chars().any(char::is_whitespace) {
                out[i] = ENDPOINT_REDACTED.to_string();
            }
            if inline {
                out[i] = format!("{}={}", key, ENDPOINT_REDACTED);
            } else if i + 1 < out.len() {
                // The value follows as its own argument.
                i += 1;
                out[i] = ENDPOINT_REDACTED.to_string();
            }
            i += 1;
            continue;
        }
        out[i] = project_value(&arg, depth + 1);
        i += 1;
    }
    out
}

/// strings.Cut: split at the first occurrence of sep; None when absent.
fn cut(s: &str, sep: char) -> Option<(&str, &str)> {
    let i = s.find(sep)?;
    Some((&s[..i], &s[i + sep.len_utf8()..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported from endpoint_test.go.

    #[test]
    fn config_projection_does_not_mutate_operational_values() {
        let mut fields = BTreeMap::new();
        fields.insert("PASSWORD".to_string(), "fixture-secret".to_string());
        fields.insert("author".to_string(), "neutral".to_string());
        fields.insert(
            "ENDPOINT".to_string(),
            "https://host/mcp?%74oken=fixture-secret".to_string(),
        );
        let mut want = BTreeMap::new();
        want.insert("PASSWORD".to_string(), ENDPOINT_REDACTED.to_string());
        want.insert("author".to_string(), "neutral".to_string());
        want.insert(
            "ENDPOINT".to_string(),
            "https://host/mcp?token=%3Credacted%3E".to_string(),
        );
        assert_eq!(redact_config_map(Some(&fields)), Some(want));
        assert_eq!(
            fields.get("PASSWORD").map(String::as_str),
            Some("fixture-secret"),
            "operational map mutated"
        );
        assert!(redact_config_map(None).is_none(), "nil projection changed");
        assert!(redact_args(&[]).is_empty(), "nil projection changed");
    }

    #[test]
    fn arguments_mask_credential_carrier_flags() {
        for args in [
            vec!["https://user:fixture-secret'private@host/mcp".to_string()],
            vec!["--endpoint=https://user:fixture-secret'private@host/mcp".to_string()],
            vec![
                "-H".to_string(),
                "Authorization: Basic fixture-secret".to_string(),
            ],
            vec![
                "--header".to_string(),
                "X-Custom: fixture-secret".to_string(),
            ],
            vec!["--env".to_string(), "CUSTOM=fixture-secret".to_string()],
            vec!["--headers={\"X-Custom\":\"fixture-secret\"}".to_string()],
        ] {
            let got = redact_args(&args).join(" ");
            assert!(
                !got.contains("fixture-secret") && !got.contains("private"),
                "flag leaked: {got}"
            );
        }
    }

    #[test]
    fn redact_endpoint_semicolon_query_keeps_origin() {
        assert_eq!(
            redact_endpoint("https://mcp.example.com/sse?a=1;token=abc"),
            "https://mcp.example.com/<redacted>"
        );
    }

    // Ported from endpoint_revision_test.go.

    #[test]
    fn revision_endpoint_path_and_opaque_fragment() {
        for raw in [
            "https://host/token/fixturesecret",
            "https://host/api_key/fixturesecret",
            "https://host/%74oken/fixturesecret",
            "https://host/mcp#fixturesecret",
        ] {
            let got = redact_endpoint(raw);
            assert!(!got.contains("fixturesecret"), "endpoint leaked: {got}");
        }
    }

    #[test]
    fn revision_embedded_url_userinfo_suffix() {
        for quote in ["'", "\""] {
            let got = redact_config_value(
                "ordinary",
                &format!("prefix https://neutral:head{quote}fixturesecret@host/mcp"),
            );
            assert!(
                !got.contains("fixturesecret"),
                "embedded userinfo leaked: {got}"
            );
        }
    }

    #[test]
    fn revision_credential_carriers() {
        for args in [
            vec!["-e".to_string(), "KEY=fixturesecret".to_string()],
            vec!["Authorization: Bearer fixturesecret".to_string()],
            vec!["Authorization: Basic fixturesecret".to_string()],
            vec!["--header 'Authorization: Bearer fixturesecret'".to_string()],
            vec!["--env 'KEY=fixturesecret'".to_string()],
            vec!["--header 'X-Custom: fixturesecret'".to_string()],
            vec!["node --header 'Authorization: Bearer fixturesecret' && true".to_string()],
            vec!["node --env 'KEY=fixturesecret".to_string()],
        ] {
            let got = redact_args(&args).join(" ");
            assert!(!got.contains("fixturesecret"), "argument leaked: {got}");
        }
        let got = redact_config_value("ordinary", "Authorization: Basic fixturesecret");
        assert!(!got.contains("fixturesecret"), "value leaked: {got}");
    }

    // Ported from endpoint_rereview_test.go.

    #[test]
    fn rereview_attached_short_credential_carriers() {
        for arg in [
            "-eKEY=fixturesecret",
            "-HAuthorization:Basic fixturesecret",
            "-HAuthorization:Bearer fixturesecret",
            "-e=KEY=fixturesecret",
            "-H=Authorization:Basic fixturesecret",
            "-HX-Custom:fixturesecret",
        ] {
            let input = vec![
                arg.to_string(),
                "ordinary".to_string(),
                "https://host/clean".to_string(),
            ];
            let original = input.clone();
            let got = redact_args(&input);
            assert!(
                !got[0].contains("fixturesecret") && got[1] == input[1] && got[2] == input[2],
                "argument projection = {got:?}"
            );
            assert_eq!(input, original, "operational argv changed");
        }
        let input = vec![
            "-v".to_string(),
            "--help".to_string(),
            "ordinary=value".to_string(),
            "https://host/clean".to_string(),
            "-e".to_string(),
            "KEY=fixturesecret".to_string(),
            "tail".to_string(),
        ];
        let got = redact_args(&input);
        assert_eq!(&got[..4], &input[..4]);
        assert_eq!(got[4], "-e");
        assert_eq!(got[5], ENDPOINT_REDACTED);
        assert_eq!(got[6], "tail");
    }

    #[test]
    fn rereview_endpoint_prefix_mixed_credentials() {
        for suffix in [
            " Authorization: Bearer fixturesecret",
            " Authorization: Basic fixturesecret",
            " --header 'X-Custom: fixturesecret'",
            " -eKEY=fixturesecret",
            "\tTOKEN=fixturesecret",
            "\nAuthorization: Basic fixturesecret",
        ] {
            let value = format!("https://host/mcp{suffix}");
            let got = redact_config_value("ORDINARY", &value);
            assert!(!got.contains("fixturesecret"), "mixed value leaked: {got}");
            let got = redact_args(&[value]);
            assert!(
                !got[0].contains("fixturesecret"),
                "mixed argument leaked: {got:?}"
            );
        }
        for value in [
            "https://host/mcp",
            "https://host/mcp?author=neutral",
            "https://host/a%20b",
            "https://host/mcp ordinary",
            "ordinary value",
        ] {
            assert_eq!(
                redact_config_value("ORDINARY", value),
                value,
                "harmless value changed"
            );
        }
    }

    #[test]
    fn rereview_url_argument_query_projection() {
        for value in [
            "https://host/mcp?token=fixturesecret",
            "https://host/mcp?%74oken=fixturesecret",
            "https://host/mcp#token=fixturesecret",
        ] {
            let got = redact_args(&[value.to_string()]);
            assert!(
                !got[0].contains("fixturesecret"),
                "URL argument leaked: {got:?}"
            );
        }
    }
}
