//! Handshake schema cache (MCP-014).
//!
//! Oracle: `internal/ext/plugin/cache.go`, contract `docs/contracts/mcp.md` §9.
//! The cache is purely an optimisation: a missing directory, unparseable
//! file, stale version, or key mismatch silently degrades to a fresh
//! handshake. The cache root is injected by the platform/config slice;
//! this module never reads environment variables or resolves a home dir.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use reasonix_core::fileutil::atomic_write_file;
use reasonix_core::secrets::credential_key;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::lifecycle::Spec;
use super::naming::validate_mcp_tool_names;

/// Bumped whenever the persisted schema shape changes incompatibly; files
/// with another version are treated as a miss.
const CACHE_VERSION: i32 = 2;

/// Per-name bound on retained per-spec entries; oldest entries are pruned
/// past it, and the entry just written is never pruned.
const MAX_SPEC_CACHES_PER_NAME: usize = 8;

/// Fixed placeholder replacing credential material inside identity URLs.
const IDENTITY_REDACTED: &str = "__redacted__";

/// Windows-reserved DOS device stems, matched case-insensitively; naming a
/// file after one would address a device instead of a file.
const WINDOWS_RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Persisted snapshot of one server's handshake result. `cache_key` gates
/// reuse: capabilities and tools are trusted only when the digest of the
/// current Spec matches, so renaming an env var or swapping a command never
/// serves stale tools.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CachedSchema {
    #[serde(rename = "version")]
    pub version: i32,
    /// Historical JSON key `spec_hash`, kept so older releases reuse the same
    /// best-effort cache across rolling upgrades.
    #[serde(rename = "spec_hash")]
    pub cache_key: String,
    #[serde(default)]
    pub capabilities: BTreeMap<String, bool>,
    /// Server self-description; optional by design, an older file simply
    /// has none.
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub instructions: String,
    #[serde(default)]
    pub tools: Vec<CachedTool>,
    #[serde(
        rename = "last_validated",
        default,
        serialize_with = "serialize_last_validated",
        deserialize_with = "deserialize_last_validated"
    )]
    pub last_validated: Option<DateTime<Utc>>,
}

/// The subset of an MCP tool definition kept to register a placeholder
/// before the real handshake completes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CachedTool {
    /// Raw, server-local name.
    pub name: String,
    /// Model-visible description.
    #[serde(default)]
    pub description: String,
    /// Raw input schema JSON; null becomes the empty object schema on read.
    #[serde(default)]
    pub schema: Value,
    #[serde(
        rename = "output_schema",
        skip_serializing_if = "Value::is_null",
        default
    )]
    pub output_schema: Value,
    #[serde(rename = "read_only")]
    pub read_only: bool,
    #[serde(skip_serializing_if = "is_false", default)]
    pub destructive: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Digest over the load-bearing, non-secret parts of a Spec. Secret values
/// in env and headers are excluded: credential rotation must not leak
/// through a stable digest or force an unrelated trust review. Their sorted
/// key names remain identity-bearing, so adding or removing a runtime
/// input still invalidates the cached schema.
pub fn schema_cache_key(spec: &Spec) -> String {
    let normalized = normalize_identity_url(&spec.url);
    schema_cache_key_for_url(spec, &normalized)
}

/// Recomputes the cache key with the pre-credential-aware URL normalization,
/// or `None` when the two normalizations cannot differ. Kept so legacy
/// cache entries remain readable during the compatibility window.
pub fn legacy_schema_cache_key(spec: &Spec) -> Option<String> {
    let legacy = legacy_normalize_identity_url(&spec.url);
    if spec.url.trim().is_empty() || legacy == normalize_identity_url(&spec.url) {
        return None;
    }
    Some(schema_cache_key_for_url(spec, &legacy))
}

fn schema_cache_key_for_url(spec: &Spec, url_value: &str) -> String {
    let mut h = Sha256::new();
    write_field(&mut h, "name", &spec.name);
    write_field(&mut h, "type", &spec.transport);
    write_field(&mut h, "command", &spec.command);
    write_field(&mut h, "url", url_value);
    write_field(&mut h, "dir", &spec.dir);
    for arg in &spec.args {
        write_field(&mut h, "arg", arg);
    }
    for disabled in sorted_compact(&spec.disabled_tools) {
        write_field(&mut h, "disabled_tool", &disabled);
    }
    write_keys(&mut h, "env", &spec.env);
    write_keys(&mut h, "headers", &spec.headers);
    hex_encode(&h.finalize())
}

/// Tagged field framing: key + 0x00 + value + 0x01, so the boundary between
/// one field and the next cannot collide through concatenation.
fn write_field(h: &mut Sha256, key: &str, val: &str) {
    h.update(key.as_bytes());
    h.update([0x00]);
    h.update(val.as_bytes());
    h.update([0x01]);
}

/// Hashes only sorted map keys. Values never enter the digest; an empty map
/// contributes one framed empty field, matching the Go oracle.
fn write_keys(h: &mut Sha256, field: &str, map: &BTreeMap<String, String>) {
    if map.is_empty() {
        write_field(h, field, "");
        return;
    }
    for key in map.keys() {
        write_field(h, &format!("{field}.{key}"), "present");
    }
}

fn sorted_compact(names: &[String]) -> Vec<String> {
    let mut out: Vec<&str> = names.iter().map(String::as_str).collect();
    out.sort_unstable();
    out.dedup();
    out.iter().map(|s| s.to_string()).collect()
}

/// Loads the cached schema for `name` iff it exists, parses, version-matches,
/// and its key matches `expected_key`. Any failure degrades to a silent miss.
pub fn load_cached_schema(root: &Path, name: &str, expected_key: &str) -> Option<CachedSchema> {
    let (schema, key_ok) = load_cached_schema_any(root, name, expected_key);
    schema.filter(|_| key_ok)
}

/// Catalog view: returns the cached schema regardless of key match, plus
/// whether the key matched `expected_key`. Execution paths must keep using
/// [`load_cached_schema`], which refuses mismatches.
pub fn load_cached_schema_any(
    root: &Path,
    name: &str,
    expected_key: &str,
) -> (Option<CachedSchema>, bool) {
    if let Some(path) = spec_cache_path(root, name, expected_key)
        && let Some(schema) = read_cached_schema(&path)
    {
        let key_ok = schema.cache_key == expected_key;
        return (Some(schema), key_ok);
    }
    let name_level = cache_path(root, name);
    if let Some(schema) = read_cached_schema(&name_level) {
        let key_ok = schema.cache_key == expected_key;
        return (Some(schema), key_ok);
    }
    // A sibling for the same name still shows what the server last offered,
    // as a stale candidate; key_ok stays false.
    if let Some(path) = newest_sibling_cache_path(root, name)
        && let Some(schema) = read_cached_schema(&path)
    {
        return (Some(schema), false);
    }
    (None, false)
}

/// Returns a usable enabled-tool snapshot matching the current Spec. A
/// legacy-URL-keyed entry is upgraded in place: the upgraded key is claimed
/// only once the rewrite reached disk, otherwise the hit keeps the legacy
/// key the file still holds.
pub fn load_cached_schema_for_spec(root: &Path, spec: &Spec) -> Option<CachedSchema> {
    let current = schema_cache_key(spec);
    if let Some(mut schema) = load_cached_schema(root, &spec.name, &current) {
        schema.tools = enabled_cached_tools(spec, schema.tools);
        if !cached_tool_names_usable(spec, &schema.tools) {
            return None;
        }
        return Some(schema);
    }
    let legacy = legacy_schema_cache_key(spec)?;
    let mut schema = load_cached_schema(root, &spec.name, &legacy)?;
    schema.tools = enabled_cached_tools(spec, schema.tools);
    if !cached_tool_names_usable(spec, &schema.tools) {
        return None;
    }
    let mut upgraded = schema.clone();
    upgraded.cache_key = current.clone();
    if save_cached_schema(root, &spec.name, upgraded).is_ok() {
        schema.cache_key = current;
    }
    Some(schema)
}

/// Persists `schema` under `name`. Best-effort: the error is returned for
/// the caller that records it; a missing cache root simply no-ops.
pub fn save_cached_schema(root: &Path, name: &str, mut schema: CachedSchema) -> io::Result<()> {
    let path =
        spec_cache_path(root, name, &schema.cache_key).unwrap_or_else(|| cache_path(root, name));
    schema.version = CACHE_VERSION;
    if schema.last_validated.is_none() {
        schema.last_validated = Some(Utc::now());
    }
    let body = serde_json::to_vec_pretty(&schema)?;
    atomic_write_file(&path, &body, 0o600)?;
    prune_spec_caches(root, name, &path);
    Ok(())
}

/// Reads and validates one cache file; a missing, unreadable, or bad file is
/// a miss.
fn read_cached_schema(path: &Path) -> Option<CachedSchema> {
    let bytes = fs::read(path).ok()?;
    let mut schema: CachedSchema = serde_json::from_slice(&bytes).ok()?;
    if schema.version != CACHE_VERSION {
        return None;
    }
    schema.tools = filter_valid_cached_tools(schema.tools);
    Some(schema)
}

/// Keeps tools whose input schema is a JSON object; a null schema becomes
/// the empty object schema. Non-object schemas are quarantined, matching
/// the oracle's per-tool validation gate.
fn filter_valid_cached_tools(tools: Vec<CachedTool>) -> Vec<CachedTool> {
    tools
        .into_iter()
        .filter_map(|mut tool| {
            if tool.schema.is_null() {
                tool.schema = empty_object_schema();
            } else if !tool.schema.is_object() {
                return None;
            }
            Some(tool)
        })
        .collect()
}

fn empty_object_schema() -> Value {
    serde_json::json!({"properties": {}, "required": [], "type": "object"})
}

fn enabled_cached_tools(spec: &Spec, tools: Vec<CachedTool>) -> Vec<CachedTool> {
    if spec.disabled_tools.is_empty() {
        return tools;
    }
    tools
        .into_iter()
        .filter(|tool| spec.tool_enabled(&tool.name))
        .collect()
}

fn cached_tool_names_usable(spec: &Spec, tools: &[CachedTool]) -> bool {
    let names: Vec<String> = tools.iter().map(|tool| tool.name.clone()).collect();
    validate_mcp_tool_names(&spec.name, &spec.strip_raw_prefix, &names).is_ok()
}

/// Per-spec entry: `<root>/mcp/<slug(name)>.<key tag>.json`. `None` when
/// there is no key to tell specs apart.
fn spec_cache_path(root: &Path, name: &str, key: &str) -> Option<PathBuf> {
    let tag = cache_key_tag(key);
    if tag.is_empty() {
        return None;
    }
    Some(mcp_dir(root).join(format!("{}.{tag}.json", slug(name))))
}

/// Name-level legacy entry: `<root>/mcp/<slug(name)>.json`.
fn cache_path(root: &Path, name: &str) -> PathBuf {
    mcp_dir(root).join(format!("{}.json", slug(name)))
}

fn mcp_dir(root: &Path) -> PathBuf {
    root.join("mcp")
}

/// Tag is the lowercase hex-only prefix of the key, at most 16 characters.
fn cache_key_tag(key: &str) -> String {
    let filtered: String = key
        .chars()
        .filter(|c| c.is_ascii_digit() || ('a'..='f').contains(c))
        .collect();
    filtered.chars().take(16).collect()
}

/// Every per-spec entry of `name`, newest first. The name-level legacy file
/// does not match the `<slug>.` prefix and is excluded.
fn sibling_cache_paths(root: &Path, name: &str) -> Vec<PathBuf> {
    let dir = mcp_dir(root);
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let prefix = format!("{}.", slug(name));
    let mut dated: Vec<(PathBuf, SystemTime)> = Vec::new();
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            continue;
        }
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if !file_name.starts_with(&prefix) || !file_name.ends_with(".json") {
            continue;
        }
        let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
        dated.push((entry.path(), modified));
    }
    dated.sort_by_key(|(_, modified)| Reverse(*modified));
    dated.into_iter().map(|(path, _)| path).collect()
}

fn newest_sibling_cache_path(root: &Path, name: &str) -> Option<PathBuf> {
    sibling_cache_paths(root, name).into_iter().next()
}

/// Drops the oldest entries of `name` past the bound, never the one just
/// written.
fn prune_spec_caches(root: &Path, name: &str, keep: &Path) {
    for (index, path) in sibling_cache_paths(root, name).iter().enumerate() {
        if index >= MAX_SPEC_CACHES_PER_NAME && path != keep {
            let _ = fs::remove_file(path);
        }
    }
}

/// Sanitizes `name` for use as a filename. Names changed by sanitization,
/// and Windows-reserved device stems, get a strong suffix so confusable names
/// cannot make one server consume another's cache. Ordinary safe names
/// stay byte-identical.
pub fn slug(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let mut cleaned = String::with_capacity(lower.len());
    let mut dash_run = false;
    for c in lower.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            cleaned.push(c);
            dash_run = false;
        } else if !dash_run {
            cleaned.push('-');
            dash_run = true;
        }
    }
    let stem = cleaned.trim_matches('-');
    let mut slug = if stem.is_empty() {
        "_".to_string()
    } else {
        stem.to_string()
    };
    if slug != name || WINDOWS_RESERVED.contains(&slug.as_str()) {
        let sum = Sha256::digest(name.as_bytes());
        slug.push_str(&format!("-{}", hex_encode(&sum[..6])));
    }
    slug
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Canonicalizes an endpoint for identity and cache keys: scheme/host case
/// and default ports fold, the fragment drops, query keys sort stably, and
/// credential material (userinfo, credential query values) becomes a fixed
/// placeholder so rotation never invalidates the digest. Network requests
/// always use the raw configured URL, never this form.
///
/// Known overlap: the workspace has no shared identity-URL normalizer yet
/// (MCP-034 / §22); this minimal equivalent lives here until that slice
/// converges. Query values are not percent-re-encoded, which only affects
/// non-ASCII or escaped resource values, never the ASCII test corpus.
fn normalize_identity_url(raw: &str) -> String {
    normalize_url(raw, true)
}

/// Pre-credential-aware normalization kept only to read legacy cache keys.
fn legacy_normalize_identity_url(raw: &str) -> String {
    normalize_url(raw, false)
}

fn normalize_url(raw: &str, redact: bool) -> String {
    let trimmed = raw.trim();
    let Some((scheme, rest)) = trimmed.split_once("://") else {
        return trimmed.to_string();
    };
    let scheme = scheme.to_ascii_lowercase();
    let authority_end = rest
        .bytes()
        .position(|b| matches!(b, b'/' | b'?' | b'#'))
        .unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let tail = &rest[authority_end..];

    let (userinfo, hostport) = match authority.rfind('@') {
        Some(i) => (Some(&authority[..i]), &authority[i + 1..]),
        None => (None, authority),
    };
    let (host, port) = split_host_port(hostport);
    let host = host.to_ascii_lowercase();
    let port = strip_default_port(&scheme, port);

    let mut out = scheme;
    out.push_str("://");
    if let Some(userinfo) = userinfo {
        if redact {
            if userinfo.contains(':') {
                out.push_str(IDENTITY_REDACTED);
                out.push(':');
                out.push_str(IDENTITY_REDACTED);
            } else {
                out.push_str(IDENTITY_REDACTED);
            }
        } else {
            out.push_str(userinfo);
        }
        out.push('@');
    }
    out.push_str(&host);
    if let Some(port) = port {
        out.push(':');
        out.push_str(&port);
    }

    let path_end = tail.find('?').unwrap_or(tail.len());
    out.push_str(&tail[..path_end]);
    let query = tail[path_end..].strip_prefix('?').unwrap_or("");
    let query = query.split('#').next().unwrap_or("");
    if !query.is_empty() {
        out.push('?');
        out.push_str(&normalize_query(query, redact));
    }
    out
}

fn split_host_port(hostport: &str) -> (String, Option<String>) {
    if let Some(rest) = hostport.strip_prefix('[') {
        let Some(close) = rest.find(']') else {
            return (hostport.to_string(), None);
        };
        let host = &rest[..close];
        let after = &rest[close + 1..];
        let port = after.strip_prefix(':').filter(|p| !p.is_empty());
        return (format!("[{host}]"), port.map(str::to_string));
    }
    match hostport.rfind(':') {
        Some(i) => (
            hostport[..i].to_string(),
            Some(hostport[i + 1..].to_string()),
        ),
        None => (hostport.to_string(), None),
    }
}

fn strip_default_port(scheme: &str, port: Option<String>) -> Option<String> {
    match (scheme, port.as_deref()) {
        ("https", Some("443")) | ("http", Some("80")) => None,
        _ => port,
    }
}

fn normalize_query(raw: &str, redact: bool) -> String {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = match pair.find('=') {
            Some(i) => (&pair[..i], &pair[i + 1..]),
            None => (pair, ""),
        };
        map.entry(key.to_string())
            .or_default()
            .push(value.to_string());
    }
    let mut out = String::new();
    for (key, mut values) in map {
        if redact && credential_key(&key) {
            values = vec![IDENTITY_REDACTED.to_string()];
        } else {
            values.sort();
        }
        for value in values {
            if !out.is_empty() {
                out.push('&');
            }
            out.push_str(&key);
            out.push('=');
            out.push_str(&value);
        }
    }
    out
}

fn serialize_last_validated<S>(
    value: &Option<DateTime<Utc>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match value {
        Some(dt) => serializer.serialize_str(&dt.to_rfc3339()),
        None => serializer.serialize_none(),
    }
}

fn deserialize_last_validated<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    raw.map(|raw| {
        DateTime::parse_from_rfc3339(&raw)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(serde::de::Error::custom)
    })
    .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn sample_spec() -> Spec {
        let mut env = BTreeMap::new();
        env.insert("FOO".to_string(), "1".to_string());
        env.insert("BAR".to_string(), "2".to_string());
        let mut headers = BTreeMap::new();
        headers.insert("X-Custom".to_string(), "ok".to_string());
        Spec {
            name: "my-server".to_string(),
            transport: "stdio".to_string(),
            command: "/usr/bin/example".to_string(),
            args: vec!["--flag".to_string(), "x".to_string()],
            env,
            headers,
            dir: "/work".to_string(),
            ..Spec::default()
        }
    }

    fn sample_schema(key: &str, tools: Vec<CachedTool>) -> CachedSchema {
        let mut capabilities = BTreeMap::new();
        capabilities.insert("prompts".to_string(), true);
        capabilities.insert("resources".to_string(), false);
        CachedSchema {
            cache_key: key.to_string(),
            capabilities,
            instructions: "how the server is used".to_string(),
            tools,
            ..CachedSchema::default()
        }
    }

    fn one_tool(name: &str, read_only: bool, destructive: bool) -> CachedTool {
        CachedTool {
            name: name.to_string(),
            description: "does a thing".to_string(),
            schema: serde_json::json!({"type": "object"}),
            output_schema: Value::Null,
            read_only,
            destructive,
        }
    }

    #[test]
    fn round_trip_persists_schema_and_shape_on_disk() {
        let root = tempfile::tempdir().expect("temp cache root");
        let spec = sample_spec();
        let key = schema_cache_key(&spec);
        let schema = sample_schema(&key, vec![one_tool("do_thing", true, true)]);

        save_cached_schema(root.path(), &spec.name, schema).expect("save");

        let path = spec_cache_path(root.path(), &spec.name, &key).expect("spec path");
        let body = fs::read(&path).expect("read cache file");
        let text = String::from_utf8(body).expect("utf8 cache file");
        assert!(
            text.contains("\"spec_hash\""),
            "historical key missing: {text}"
        );
        assert!(
            !text.contains("\"cache_key\""),
            "renamed historical key: {text}"
        );
        assert!(text.contains("\"version\": 2"), "version shape: {text}");
        assert!(
            text.contains("\"read_only\": true"),
            "tool flag shape: {text}"
        );
        assert!(
            text.contains("\"last_validated\""),
            "validation timestamp: {text}"
        );
        // 2-space indent, matching MarshalIndent's two spaces.
        assert!(
            text.lines()
                .any(|line| line.starts_with("  \"capabilities\""))
        );

        let loaded = load_cached_schema(root.path(), &spec.name, &key).expect("load");
        assert_eq!(loaded.cache_key, key);
        assert_eq!(loaded.version, CACHE_VERSION);
        assert_eq!(loaded.tools.len(), 1);
        assert_eq!(loaded.tools[0].name, "do_thing");
        assert!(loaded.tools[0].read_only);
        assert!(loaded.tools[0].destructive);
        assert!(loaded.capabilities["prompts"]);
        assert!(!loaded.capabilities["resources"]);
        assert_eq!(loaded.instructions, "how the server is used");
        assert!(loaded.last_validated.is_some());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).expect("metadata").permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "cache file must be 0600");
        }
    }

    #[test]
    fn cache_key_ignores_secret_values_but_tracks_names_and_scope() {
        let base = sample_spec();
        let base_key = schema_cache_key(&base);

        // Rotating an env or header credential value must not move the key.
        let mut rotated_env = base.clone();
        rotated_env
            .env
            .insert("BAR".to_string(), "different".to_string());
        assert_eq!(schema_cache_key(&rotated_env), base_key);
        let mut rotated_header = base.clone();
        rotated_header
            .headers
            .insert("X-Custom".to_string(), "rotated-secret".to_string());
        assert_eq!(schema_cache_key(&rotated_header), base_key);

        // Adding or renaming a key name, or disabling a tool, invalidates.
        let mut new_env_key = base.clone();
        new_env_key
            .env
            .insert("NEW_KEY".to_string(), "2".to_string());
        assert_ne!(schema_cache_key(&new_env_key), base_key);
        let mut new_header_key = base.clone();
        new_header_key
            .headers
            .insert("Authorization".to_string(), "secret".to_string());
        assert_ne!(schema_cache_key(&new_header_key), base_key);
        let mut disabled = base.clone();
        disabled.disabled_tools = vec!["hidden".to_string()];
        assert_ne!(schema_cache_key(&disabled), base_key);

        // Name, command, and args are identity-bearing.
        let mut renamed = base.clone();
        renamed.name = "other-server".to_string();
        assert_ne!(schema_cache_key(&renamed), base_key);
        let mut command_changed = base.clone();
        command_changed.command = "/usr/local/bin/other".to_string();
        assert_ne!(schema_cache_key(&command_changed), base_key);
        let mut arg_changed = base.clone();
        arg_changed.args = vec!["--different".to_string(), "x".to_string()];
        assert_ne!(schema_cache_key(&arg_changed), base_key);
    }

    #[test]
    fn url_spellings_normalize_to_one_key() {
        let mut a = sample_spec();
        a.transport = "http".to_string();
        a.url = "https://user:first@example.com/mcp?access_token=one&workspace=alpha&tenant=t1"
            .to_string();
        let key_a = schema_cache_key(&a);

        // Reordered query and rotated credential values fold together.
        let mut reordered = a.clone();
        reordered.url =
            "https://user:second@example.com/mcp?tenant=t1&workspace=alpha&access_token=two"
                .to_string();
        assert_eq!(schema_cache_key(&reordered), key_a);

        // Case and default port fold.
        let mut cased = a.clone();
        cased.url =
            "HTTPS://User:Secret@Example.COM:443/mcp?access_token=one&workspace=alpha&tenant=t1"
                .to_string();
        assert_eq!(schema_cache_key(&cased), key_a);

        // A non-sensitive scope change still moves the key.
        let mut moved = a.clone();
        moved.url = "https://user:first@example.com/mcp?access_token=one&workspace=beta&tenant=t1"
            .to_string();
        assert_ne!(schema_cache_key(&moved), key_a);
    }

    #[test]
    fn load_misses_on_version_mismatch() {
        let root = tempfile::tempdir().expect("temp cache root");
        let spec = sample_spec();
        let key = schema_cache_key(&spec);
        let path = cache_path(root.path(), &spec.name);
        fs::create_dir_all(path.parent().expect("mcp dir")).expect("create dir");
        let stale = format!(
            "{{\"version\":3,\"spec_hash\":\"{key}\",\"capabilities\":{{}},\"tools\":[],\"last_validated\":\"2026-01-01T00:00:00Z\"}}"
        );
        fs::write(&path, stale).expect("write stale file");

        assert!(load_cached_schema(root.path(), &spec.name, &key).is_none());
    }

    #[test]
    fn load_misses_on_corrupt_file_without_panic() {
        let root = tempfile::tempdir().expect("temp cache root");
        let path = cache_path(root.path(), "broken");
        fs::create_dir_all(path.parent().expect("mcp dir")).expect("create dir");
        fs::write(&path, "{this is not json").expect("write corrupt file");

        let result =
            std::panic::catch_unwind(|| load_cached_schema(root.path(), "broken", "any-key"));
        assert!(result.is_ok(), "corrupt file must not panic");
        assert!(result.expect("no panic").is_none());
    }

    #[test]
    fn key_mismatch_is_an_any_hit_with_key_ok_false() {
        let root = tempfile::tempdir().expect("temp cache root");
        let saved = sample_spec();
        let saved_key = schema_cache_key(&saved);
        save_cached_schema(
            root.path(),
            &saved.name,
            sample_schema(&saved_key, vec![one_tool("x", false, false)]),
        )
        .expect("save");

        let mut asked = sample_spec();
        asked.command = "/usr/local/bin/other".to_string();
        let asked_key = schema_cache_key(&asked);
        let (any, key_ok) = load_cached_schema_any(root.path(), &saved.name, &asked_key);
        assert!(any.is_some());
        assert!(!key_ok);
        // The execution path refuses the sibling's entry.
        assert!(load_cached_schema(root.path(), &saved.name, &asked_key).is_none());
    }

    #[test]
    fn legacy_entry_upgrades_only_after_successful_save() {
        let root = tempfile::tempdir().expect("temp cache root");
        let mut spec = sample_spec();
        spec.transport = "http".to_string();
        spec.command = String::new();
        spec.url = "https://user:pw@example.com/mcp?api_key=secret".to_string();
        let current = schema_cache_key(&spec);
        let legacy = legacy_schema_cache_key(&spec).expect("legacy key variant");
        assert_ne!(legacy, current);

        save_cached_schema(
            root.path(),
            &spec.name,
            sample_schema(&legacy, vec![one_tool("echo", false, false)]),
        )
        .expect("save legacy entry");

        let loaded = load_cached_schema_for_spec(root.path(), &spec).expect("legacy hit");
        assert_eq!(loaded.cache_key, current);
        assert_eq!(loaded.tools.len(), 1);
        assert_eq!(loaded.tools[0].name, "echo");

        // The upgrade reached disk: a plain current-key load now succeeds.
        assert!(load_cached_schema(root.path(), &spec.name, &current).is_some());
        let current_path =
            spec_cache_path(root.path(), &spec.name, &current).expect("current path");
        assert!(current_path.exists());
    }

    #[test]
    fn failed_upgrade_keeps_legacy_key_on_disk() {
        let root = tempfile::tempdir().expect("temp cache root");
        let mut spec = sample_spec();
        spec.transport = "http".to_string();
        spec.command = String::new();
        spec.url = "https://user:pw@example.com/mcp?api_key=secret".to_string();
        let current = schema_cache_key(&spec);
        let legacy = legacy_schema_cache_key(&spec).expect("legacy key variant");

        save_cached_schema(
            root.path(),
            &spec.name,
            sample_schema(&legacy, vec![one_tool("echo", false, false)]),
        )
        .expect("save legacy entry");

        // Block the upgrade publication: a directory occupies the would-be
        // current-key file, so rename onto it fails on both Unix and Windows.
        let current_path =
            spec_cache_path(root.path(), &spec.name, &current).expect("current path");
        fs::create_dir_all(&current_path).expect("block upgrade target");

        let loaded = load_cached_schema_for_spec(root.path(), &spec).expect("legacy hit still");
        assert_eq!(
            loaded.cache_key, legacy,
            "failed upgrade must keep the on-disk legacy key"
        );

        // Re-reading with the returned legacy key agrees; the current key misses.
        assert!(load_cached_schema(root.path(), &spec.name, &legacy).is_some());
        assert!(load_cached_schema(root.path(), &spec.name, &current).is_none());
    }

    #[test]
    fn prune_keeps_eight_newest_entries_per_name() {
        let root = tempfile::tempdir().expect("temp cache root");
        let mut specs = Vec::new();
        for i in 0..=MAX_SPEC_CACHES_PER_NAME + 1 {
            let mut spec = sample_spec();
            spec.name = "docs".to_string();
            spec.command = format!("./{i}.sh");
            specs.push(spec.clone());
            let key = schema_cache_key(&spec);
            save_cached_schema(
                root.path(),
                &spec.name,
                sample_schema(&key, vec![one_tool(&format!("t{i}"), false, false)]),
            )
            .expect("save entry");
        }
        // A name that merely shares the prefix must survive pruning.
        let mut neighbor = sample_spec();
        neighbor.name = "docs-extra".to_string();
        let neighbor_key = schema_cache_key(&neighbor);
        save_cached_schema(
            root.path(),
            &neighbor.name,
            sample_schema(&neighbor_key, vec![one_tool("n", false, false)]),
        )
        .expect("save neighbor");

        let entries = sibling_cache_paths(root.path(), "docs");
        assert_eq!(entries.len(), MAX_SPEC_CACHES_PER_NAME);

        let last = specs.last().expect("last spec");
        assert!(
            load_cached_schema_for_spec(root.path(), last).is_some(),
            "just-written entry pruned"
        );
        let first = &specs[0];
        assert!(
            load_cached_schema_for_spec(root.path(), first).is_none(),
            "oldest entry survived"
        );
        assert!(
            load_cached_schema_for_spec(root.path(), &neighbor).is_some(),
            "neighbour pruned"
        );
    }

    #[test]
    fn slug_is_filesystem_safe_and_reserved_names_get_hash_suffix() {
        assert_eq!(slug("a_b-c"), "a_b-c");
        assert_eq!(slug("github"), "github");
        assert_eq!(slug("console"), "console");
        assert_eq!(slug("com10"), "com10");
        assert_eq!(slug("naux"), "naux");

        for reserved in [
            "con", "CON", "prn", "aux", "nul", "com1", "COM9", "lpt1", "LPT9",
        ] {
            let got = slug(reserved);
            let lowered = reserved.to_ascii_lowercase();
            assert!(
                got.starts_with(&format!("{lowered}-")),
                "slug({reserved:?}) = {got}"
            );
            let suffix = got
                .strip_prefix(&format!("{lowered}-"))
                .expect("hash suffix");
            assert_eq!(suffix.len(), 12, "six-byte sha256 hex: {suffix}");
            assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
        }

        // Sanitization changed the name, so a hash suffix is appended.
        let cleaned = slug("My Server!");
        assert!(cleaned.starts_with("my-server-"));
        assert!(!cleaned.contains('/') && !cleaned.contains('\\') && !cleaned.contains(':'));

        let empty = slug("");
        assert!(empty.starts_with("_-"));

        // Confusable spellings must not collide.
        let confusable = ["foo", "Foo", "foo bar", "foo-bar", "foo/bar", "foo\\bar"];
        let slugs: Vec<String> = confusable.iter().map(|n| slug(n)).collect();
        for (i, a) in slugs.iter().enumerate() {
            for b in &slugs[..i] {
                assert_ne!(a, b, "slug collision {a}");
            }
        }
    }

    #[test]
    fn disabled_tools_are_filtered_from_spec_result() {
        let root = tempfile::tempdir().expect("temp cache root");
        let mut spec = sample_spec();
        spec.disabled_tools = vec!["hidden".to_string()];
        let key = schema_cache_key(&spec);
        save_cached_schema(
            root.path(),
            &spec.name,
            sample_schema(
                &key,
                vec![
                    one_tool("visible", false, false),
                    one_tool("hidden", false, false),
                ],
            ),
        )
        .expect("save");

        let loaded = load_cached_schema_for_spec(root.path(), &spec).expect("hit");
        let names: Vec<&str> = loaded.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["visible"]);
    }

    #[test]
    fn invalid_tool_name_makes_spec_hit_a_miss() {
        let root = tempfile::tempdir().expect("temp cache root");
        let spec = sample_spec();
        let key = schema_cache_key(&spec);
        save_cached_schema(
            root.path(),
            &spec.name,
            sample_schema(&key, vec![one_tool("", false, false)]),
        )
        .expect("save");

        assert!(load_cached_schema_for_spec(root.path(), &spec).is_none());
    }

    #[test]
    fn name_level_legacy_entry_reads_only_for_its_own_spec() {
        let root = tempfile::tempdir().expect("temp cache root");
        let a = sample_spec();
        let a_key = schema_cache_key(&a);
        let path = cache_path(root.path(), &a.name);
        fs::create_dir_all(path.parent().expect("mcp dir")).expect("create dir");
        let legacy = format!(
            "{{\"version\":2,\"spec_hash\":\"{a_key}\",\"capabilities\":{{}},\"tools\":[{{\"name\":\"echo\",\"description\":\"\",\"schema\":{{\"type\":\"object\"}},\"read_only\":false}}],\"last_validated\":\"2026-01-01T00:00:00Z\"}}"
        );
        fs::write(&path, legacy).expect("write name-level entry");

        assert!(load_cached_schema_for_spec(root.path(), &a).is_some());
        let mut other = a.clone();
        other.command = "/usr/local/bin/other".to_string();
        assert!(load_cached_schema_for_spec(root.path(), &other).is_none());
    }
}
