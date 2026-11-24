//! Modern-era request headers and `x-mcp-header` tool-argument annotations
//! (MCP-011).
//!
//! Oracle: `internal/ext/plugin/modern_headers.go`. A 2026-07-28 request over
//! Streamable HTTP mirrors parts of its body into headers for gateways; they
//! are derived from the body here, so the two cannot disagree. An annotation
//! that breaks the rules disqualifies only its own tool — the catalog caller
//! drops that tool rather than failing the whole server.

use serde_json::Value;

use super::version::META_PROTOCOL_VERSION;

/// Largest integer a header may carry as an exact JSON number (Go
/// `maxSafeInteger`, 2^53 - 1).
const MAX_SAFE_INTEGER: i64 = (1 << 53) - 1;

/// RFC 9110 tchar characters beyond ASCII letters and digits.
const TCHAR_EXTRA: &[u8] = b"!#$%&'*+-.^_`|~";

/// One tool argument mirrored into an `Mcp-Param-{Name}` header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamHeader {
    pub name: String,
    pub path: Vec<String>,
}

/// Typed failure for a bad `x-mcp-header` annotation. The code string is the
/// identity a catalog caller matches on; message text never varies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid x-mcp-header annotation")]
pub struct HeaderAnnotationError;

/// Encodes a header value: visible ASCII with no surrounding space passes
/// through; anything else uses the base64 sentinel form (Go `encodeHeaderValue`).
pub fn encode_header_value(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    let plain = input == input.trim()
        && !(input.starts_with("=?base64?") && input.ends_with("?="))
        && input
            .chars()
            .all(|r| ('\u{20}'..='\u{7e}').contains(&r) || r == '\t');
    if plain {
        return input.to_string();
    }
    format!("=?base64?{}?=", base64_encode(input.as_bytes()))
}

/// Reads a tool's `x-mcp-header` annotations from its input schema.
///
/// Only annotations reachable through a plain `properties` property path are
/// legal; an invalid annotation returns the typed error so the catalog caller
/// can drop just that tool.
pub fn tool_param_headers(schema: &Value) -> Result<Vec<ParamHeader>, HeaderAnnotationError> {
    let mut out = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    walk(schema, &[], true, &mut seen, &mut out)?;
    Ok(out)
}

fn walk(
    node: &Value,
    path: &[String],
    reachable: bool,
    seen: &mut Vec<String>,
    out: &mut Vec<ParamHeader>,
) -> Result<(), HeaderAnnotationError> {
    let Some(map) = node.as_object() else {
        if let Some(array) = node.as_array() {
            for child in array {
                walk(child, path, false, seen, out)?;
            }
        }
        return Ok(());
    };
    if let Some(raw) = map.get("x-mcp-header") {
        if !reachable || path.is_empty() {
            return Err(HeaderAnnotationError);
        }
        let name = raw.as_str().ok_or(HeaderAnnotationError)?;
        if !valid_header_token(name) {
            return Err(HeaderAnnotationError);
        }
        let lower = name.to_ascii_lowercase();
        if seen.iter().any(|existing| existing == &lower) {
            return Err(HeaderAnnotationError);
        }
        let ty = map.get("type").and_then(Value::as_str).unwrap_or_default();
        if !matches!(ty, "string" | "integer" | "boolean") {
            return Err(HeaderAnnotationError);
        }
        seen.push(lower);
        out.push(ParamHeader {
            name: name.to_string(),
            path: path.to_vec(),
        });
    }
    for (key, child) in map {
        if key == "properties" {
            if let Some(props) = child.as_object() {
                for (prop, sub) in props {
                    let mut child_path = path.to_vec();
                    child_path.push(prop.clone());
                    walk(sub, &child_path, true, seen, out)?;
                }
            }
        } else {
            walk(child, path, false, seen, out)?;
        }
    }
    Ok(())
}

/// RFC 9110 field-name token: 1*tchar.
fn valid_header_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || TCHAR_EXTRA.contains(&b))
}

/// The header form of an argument, or `None` when it is absent, null, or not
/// a value a header may carry.
pub fn header_scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Bool(flag) => Some(if *flag { "true" } else { "false" }.to_string()),
        Value::Number(number) => {
            if let Some(int) = number.as_i64() {
                return (int.unsigned_abs() <= MAX_SAFE_INTEGER as u64).then(|| int.to_string());
            }
            if let Some(uint) = number.as_u64() {
                return (uint <= MAX_SAFE_INTEGER as u64).then(|| uint.to_string());
            }
            let float = number.as_f64()?;
            if !float.is_finite() || float.fract() != 0.0 || float.abs() > MAX_SAFE_INTEGER as f64 {
                return None;
            }
            Some((float as i64).to_string())
        }
        _ => None,
    }
}

/// Reads one annotated argument through its property path, if present.
pub fn param_header_value(args: &Value, path: &[String]) -> Option<String> {
    let mut current = args;
    for key in path {
        current = current.get(key)?;
    }
    header_scalar(current)
}

/// Builds the `Mcp-Param-*` headers for one `tools/call` argument set.
pub fn param_headers_for_args(headers: &[ParamHeader], args: &Value) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|header| {
            let value = param_header_value(args, &header.path)?;
            Some((
                format!("Mcp-Param-{}", header.name),
                encode_header_value(&value),
            ))
        })
        .collect()
}

/// Modern request headers derived from the body. A request without a modern
/// `_meta` protocol version gets none, keeping legacy behavior intact.
pub fn modern_request_headers(method: &str, params: Option<&Value>) -> Vec<(String, String)> {
    let Some(params) = params.and_then(Value::as_object) else {
        return Vec::new();
    };
    let Some(meta) = params.get("_meta").and_then(Value::as_object) else {
        return Vec::new();
    };
    let Some(version) = meta.get(META_PROTOCOL_VERSION).and_then(Value::as_str) else {
        return Vec::new();
    };
    if version.is_empty() {
        return Vec::new();
    }
    let mut headers = vec![
        ("MCP-Protocol-Version".to_string(), version.to_string()),
        ("Mcp-Method".to_string(), method.to_string()),
    ];
    let name = match method {
        "tools/call" | "prompts/get" => params.get("name").and_then(Value::as_str),
        "tasks/get" | "tasks/update" | "tasks/cancel" => {
            params.get("taskId").and_then(Value::as_str)
        }
        "resources/read" => params.get("uri").and_then(Value::as_str),
        _ => None,
    };
    if let Some(name) = name {
        headers.push(("Mcp-Name".to_string(), encode_header_value(name)));
    }
    headers
}

/// Standard alphabet only; the workspace pins no base64 crate, and the
/// sentinel format needs std-encoding padding.
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((triple >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((triple >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(triple & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}
