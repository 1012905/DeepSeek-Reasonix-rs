//! `.mcp.json` load, merge, upsert and remove (docs/contracts/config.md §10.21;
//! mcpjson.go).
//!
//! The project-root `.mcp.json` is Claude Code compatible: an `mcpServers` map
//! whose specs map field-for-field onto `PluginEntry`. This slice parses the
//! document, merges it over the TOML-declared plugins, and rewrites one server
//! while preserving unrelated top-level fields and unknown per-server fields.
//! File IO, the symlink access check, the temp-file atomic replace and the
//! boot call site are wiring left to the integration layer (CONFIG-012 wip).

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::plugin::{
    PluginEntry, merge_disabled_tool_policies, normalize_plugin_command_line,
    validate_disabled_tools,
};

/// The project-root file Claude Code calls `.mcp.json`.
pub const MCP_JSON_FILE: &str = ".mcp.json";

/// Why a `.mcp.json` document could not be parsed or rewritten.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum McpJsonError {
    /// The body is not valid JSON, or is not an object where one was expected.
    #[error("mcp config: {message}")]
    Parse { message: String },
    /// `mcpServers` is present but is not an object.
    #[error("mcp config: mcpServers must be an object")]
    ServersNotObject,
    /// One named server entry is present but is not an object.
    #[error("mcp config: server {name:?} is not an object")]
    ServerNotObject { name: String },
    /// The entry fails transport or field validation.
    #[error("plugin {name:?}: {message}")]
    Plugin { name: String, message: String },
}

/// One `mcpServers` spec, mirroring the subset of `PluginEntry` the JSON file
/// carries (mcpjson.go mcpServerSpec).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
struct McpServerSpec {
    #[serde(rename = "type")]
    kind: String,
    command: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    url: String,
    headers: BTreeMap<String, String>,
    startup_timeout_seconds: i64,
    call_timeout_seconds: i64,
    tool_timeout_seconds: BTreeMap<String, i64>,
    disabled_tools: Vec<String>,
    auto_start: Option<bool>,
    #[serde(rename = "alwaysLoad")]
    always_load: bool,
}

fn spec_to_entry(name: &str, spec: McpServerSpec) -> PluginEntry {
    let mut entry = PluginEntry {
        name: name.to_owned(),
        kind: spec.kind,
        command: spec.command,
        args: spec.args,
        env: spec.env,
        url: spec.url,
        headers: spec.headers,
        startup_timeout_seconds: spec.startup_timeout_seconds,
        call_timeout_seconds: spec.call_timeout_seconds,
        tool_timeout_seconds: spec.tool_timeout_seconds,
        disabled_tools: spec.disabled_tools,
        auto_start: spec.auto_start,
        ..PluginEntry::default()
    };
    if spec.always_load {
        entry.load = "always".to_owned();
    }
    let (normalized, _) = normalize_plugin_command_line(&entry);
    normalized
}

/// Validates an entry by transport (edit.go validatePlugin). Empty type means
/// stdio.
pub fn validate_plugin(entry: &PluginEntry) -> Result<(), McpJsonError> {
    let name = entry.name.trim().to_owned();
    if name.is_empty() {
        return Err(McpJsonError::Plugin {
            name: String::new(),
            message: "name is required".to_owned(),
        });
    }
    if entry.startup_timeout_seconds < 0 {
        return Err(McpJsonError::Plugin {
            name: name.clone(),
            message: format!(
                "startup_timeout_seconds must be >= 0, got {}",
                entry.startup_timeout_seconds
            ),
        });
    }
    if entry.call_timeout_seconds < 0 {
        return Err(McpJsonError::Plugin {
            name: name.clone(),
            message: format!(
                "call_timeout_seconds must be >= 0, got {}",
                entry.call_timeout_seconds
            ),
        });
    }
    for (tool, seconds) in &entry.tool_timeout_seconds {
        if tool.trim().is_empty() {
            return Err(McpJsonError::Plugin {
                name: name.clone(),
                message: "tool_timeout_seconds contains an empty tool name".to_owned(),
            });
        }
        if *seconds < 0 {
            return Err(McpJsonError::Plugin {
                name: name.clone(),
                message: format!("tool_timeout_seconds[{tool:?}] must be >= 0, got {seconds}"),
            });
        }
    }
    match entry.kind.trim().to_lowercase().as_str() {
        "" | "stdio" => {
            if entry.command.trim().is_empty() {
                return Err(McpJsonError::Plugin {
                    name,
                    message: "command is required for a stdio server".to_owned(),
                });
            }
        }
        "http" | "sse" | "streamable-http" => {
            if entry.url.trim().is_empty() {
                return Err(McpJsonError::Plugin {
                    name,
                    message: format!("url is required for a {} server", entry.kind.trim()),
                });
            }
        }
        other => {
            return Err(McpJsonError::Plugin {
                name,
                message: format!("unknown type {other:?} (want stdio|http|sse)"),
            });
        }
    }
    if let Some(message) = validate_disabled_tools(entry) {
        return Err(McpJsonError::Plugin { name, message });
    }
    Ok(())
}

fn specs_map(value: &Value) -> Option<&Map<String, Value>> {
    match value {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// Parses a Claude-compatible `mcpServers` document (mcpjson.go
/// ParseMCPServersJSON). A bare server map (what a user copies out of a larger
/// document) is accepted too. Specs with neither command nor URL are dropped;
/// an empty result is an error. Entries are sorted by name for a stable
/// connection order; `source` is left unset because the caller owns it.
pub fn parse_mcp_servers_json(body: &str) -> Result<Vec<PluginEntry>, McpJsonError> {
    let wrapper: Value = serde_json::from_str(body).map_err(|error| McpJsonError::Parse {
        message: error.to_string(),
    })?;

    let mut specs: Vec<(String, McpServerSpec)> = Vec::new();
    let mut wrapped = false;
    if let Some(servers) = wrapper.get("mcpServers").and_then(specs_map) {
        wrapped = true;
        for (name, raw) in servers {
            if raw.is_null() {
                continue;
            }
            let spec: McpServerSpec =
                serde_json::from_value(raw.clone()).map_err(|error| McpJsonError::Parse {
                    message: error.to_string(),
                })?;
            specs.push((name.clone(), spec));
        }
    }
    if !wrapped {
        let bare: Map<String, Value> =
            serde_json::from_str(body).map_err(|error| McpJsonError::Parse {
                message: error.to_string(),
            })?;
        for (name, raw) in bare {
            if raw.is_null() {
                continue;
            }
            let spec: McpServerSpec =
                serde_json::from_value(raw).map_err(|error| McpJsonError::Parse {
                    message: error.to_string(),
                })?;
            specs.push((name, spec));
        }
    }

    // Either shape needs a command or URL to describe an installable server.
    specs.retain(|(_, spec)| !spec.command.is_empty() || !spec.url.is_empty());
    if specs.is_empty() {
        return Err(McpJsonError::Parse {
            message: "no MCP server found in this JSON".to_owned(),
        });
    }

    specs.sort_by(|(a, _), (b, _)| a.cmp(b));
    Ok(specs
        .into_iter()
        .map(|(name, spec)| spec_to_entry(&name, spec))
        .collect())
}

/// Reads the raw document and its `mcpServers` map, preserving every unknown
/// top-level field (mcpjson.go readMCPJSONRaw). A missing or null `mcpServers`
/// yields an empty map; an object is expected.
pub fn read_mcp_json_raw(body: &str) -> Result<(Value, Map<String, Value>), McpJsonError> {
    let root: Value = serde_json::from_str(body).map_err(|error| McpJsonError::Parse {
        message: error.to_string(),
    })?;
    let root = match root {
        Value::Null => Value::Object(Map::new()),
        Value::Object(_) => root,
        other => {
            return Err(McpJsonError::Parse {
                message: format!("document must be an object, got {other}"),
            });
        }
    };
    let mut servers = Map::new();
    if let Some(raw) = root.get("mcpServers") {
        match raw {
            Value::Null => {}
            Value::Object(map) => servers = map.clone(),
            _ => return Err(McpJsonError::ServersNotObject),
        }
    }
    Ok((root, servers))
}

fn set_string(server: &mut Map<String, Value>, key: &str, value: &str) {
    if value.is_empty() {
        server.remove(key);
        return;
    }
    server.insert(key.to_owned(), Value::String(value.to_owned()));
}

fn set_string_array(server: &mut Map<String, Value>, key: &str, values: &[String]) {
    if values.is_empty() {
        server.remove(key);
        return;
    }
    server.insert(
        key.to_owned(),
        Value::Array(values.iter().map(|v| Value::String(v.clone())).collect()),
    );
}

fn set_string_map(server: &mut Map<String, Value>, key: &str, values: &BTreeMap<String, String>) {
    if values.is_empty() {
        server.remove(key);
        return;
    }
    let map: Map<String, Value> = values
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    server.insert(key.to_owned(), Value::Object(map));
}

fn set_int(server: &mut Map<String, Value>, key: &str, value: i64) {
    if value <= 0 {
        server.remove(key);
        return;
    }
    server.insert(key.to_owned(), Value::from(value));
}

fn set_int_map(server: &mut Map<String, Value>, key: &str, values: &BTreeMap<String, i64>) {
    let clean: Map<String, Value> = values
        .iter()
        .filter(|(k, v)| !k.trim().is_empty() && **v > 0)
        .map(|(k, v)| (k.clone(), Value::from(*v)))
        .collect();
    if clean.is_empty() {
        server.remove(key);
        return;
    }
    server.insert(key.to_owned(), Value::Object(clean));
}

fn set_bool(server: &mut Map<String, Value>, key: &str, value: Option<bool>) {
    match value {
        Some(flag) => {
            server.insert(key.to_owned(), Value::Bool(flag));
        }
        None => {
            server.remove(key);
        }
    }
}

/// Removes Reasonix's retired per-tool `approval_mode` while keeping fields
/// owned by other MCP clients (mcpjson.go removeMCPJSONApprovalModes). A tool
/// whose only field was the approval mode drops out entirely.
fn remove_approval_modes(server: &mut Map<String, Value>) -> Result<(), McpJsonError> {
    let Some(raw_tools) = server.get("tools") else {
        return Ok(());
    };
    let Value::Object(tools) = raw_tools else {
        return Err(McpJsonError::Parse {
            message: "tools must be an object".to_owned(),
        });
    };
    let mut next_tools = Map::new();
    for (name, raw) in tools {
        let Some(fields) = specs_map(raw).cloned() else {
            next_tools.insert(name.clone(), raw.clone());
            continue;
        };
        let mut fields = fields;
        fields.remove("approval_mode");
        if fields.is_empty() {
            continue;
        }
        next_tools.insert(name.clone(), Value::Object(fields));
    }
    if next_tools.is_empty() {
        server.remove("tools");
    } else {
        server.insert("tools".to_owned(), Value::Object(next_tools));
    }
    Ok(())
}

/// Writes one entry into a server object, dropping transport-specific fields
/// and retired approval settings (mcpjson.go applyPluginEntryToMCPJSONServer).
pub fn apply_entry_to_server(
    server: &mut Map<String, Value>,
    entry: &PluginEntry,
) -> Result<(), McpJsonError> {
    let transport = entry.kind.trim().to_lowercase();
    let transport = if transport.is_empty() {
        "stdio"
    } else {
        transport.as_str()
    };

    if transport == "stdio" {
        server.remove("type");
        set_string(server, "command", entry.command.trim());
        set_string_array(server, "args", &entry.args);
        set_string_map(server, "env", &entry.env);
        server.remove("url");
        server.remove("headers");
    } else {
        set_string(server, "type", transport);
        set_string(server, "url", entry.url.trim());
        set_string_map(server, "headers", &entry.headers);
        set_string_map(server, "env", &entry.env);
        server.remove("command");
        server.remove("args");
    }

    set_int(
        server,
        "startup_timeout_seconds",
        entry.startup_timeout_seconds,
    );
    set_int(server, "call_timeout_seconds", entry.call_timeout_seconds);
    set_int_map(server, "tool_timeout_seconds", &entry.tool_timeout_seconds);
    set_string_array(server, "disabled_tools", &entry.disabled_tools);

    // The removed per-tool reader list is accepted on load for compatibility
    // but never persisted; the other retired approval fields disappear here too.
    server.remove("trusted_read_only_tools");
    server.remove("default_tools_approval_mode");
    server.remove("approvals_reviewer");
    set_bool(server, "auto_start", entry.auto_start);
    remove_approval_modes(server)
}

fn write_root(root: &Value) -> Result<String, McpJsonError> {
    let mut out = serde_json::to_string_pretty(root).map_err(|error| McpJsonError::Parse {
        message: error.to_string(),
    })?;
    out.push('\n');
    Ok(out)
}

/// Writes one server into a `.mcp.json` body, preserving unrelated top-level
/// and unknown per-server fields (mcpjson.go UpsertMCPJSONPlugin, in-memory
/// half). Returns the rewritten body and whether the server was new.
pub fn upsert_mcp_json_server(
    body: &str,
    entry: &PluginEntry,
) -> Result<(String, bool), McpJsonError> {
    let (entry, _) = normalize_plugin_command_line(entry);
    validate_plugin(&entry)?;

    let (mut root, mut servers) = read_mcp_json_raw(body)?;
    let existed = servers.contains_key(&entry.name);

    let mut server = match servers.get(&entry.name) {
        Some(raw) => specs_map(raw)
            .cloned()
            .ok_or_else(|| McpJsonError::ServerNotObject {
                name: entry.name.clone(),
            })?,
        None => Map::new(),
    };
    apply_entry_to_server(&mut server, &entry)?;
    servers.insert(entry.name.clone(), Value::Object(server));
    root["mcpServers"] = Value::Object(servers);

    let out = write_root(&root)?;
    Ok((out, !existed))
}

/// Removes one server from a `.mcp.json` body (mcpjson.go RemoveMCPJSONPlugin,
/// in-memory half). A missing entry leaves the body unchanged and reports
/// `false`; only the target server is dropped.
pub fn remove_mcp_json_server(body: &str, name: &str) -> Result<(String, bool), McpJsonError> {
    let (mut root, mut servers) = read_mcp_json_raw(body)?;
    if !servers.contains_key(name) {
        return Ok((body.to_owned(), false));
    }
    servers.remove(name);
    root["mcpServers"] = Value::Object(servers);
    let out = write_root(&root)?;
    Ok((out, true))
}

/// Appends `.mcp.json` servers the TOML config did not declare
/// (mcpjson.go mergeMCPJSON). The TOML plugin wins on a name collision; the
/// `disabled_tools` lists are unioned so a later source cannot re-enable a
/// restriction.
pub fn merge_mcp_json(declared: &[PluginEntry], incoming: Vec<PluginEntry>) -> Vec<PluginEntry> {
    let mut out = declared.to_vec();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    for (i, plugin) in out.iter().enumerate() {
        index.insert(plugin.name.clone(), i);
    }
    for entry in incoming {
        match index.get(&entry.name) {
            Some(&i) => {
                out[i].disabled_tools =
                    merge_disabled_tool_policies(&out[i].disabled_tools, &entry.disabled_tools);
            }
            None => {
                index.insert(entry.name.clone(), out.len());
                out.push(entry);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio_entry(name: &str, command: &str) -> PluginEntry {
        PluginEntry {
            name: name.to_owned(),
            command: command.to_owned(),
            ..PluginEntry::default()
        }
    }

    #[test]
    fn parses_wrapped_and_bare_maps_and_sorts_by_name() {
        let wrapped = r#"{"mcpServers":{"filesystem":{"command":"npx","args":["/tmp"]},"stripe":{"type":"http","url":"https://mcp.stripe.com"}}}"#;
        let entries = parse_mcp_servers_json(wrapped).expect("wrapped");
        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["filesystem", "stripe"]
        );
        assert_eq!(entries[1].kind, "http");
        assert_eq!(entries[1].url, "https://mcp.stripe.com");

        let bare = r#"{"docs":{"command":"docs-mcp"}}"#;
        let entries = parse_mcp_servers_json(bare).expect("bare");
        assert_eq!(entries[0].name, "docs");
    }

    #[test]
    fn drops_specs_with_neither_command_nor_url_and_reports_empty() {
        let body =
            r#"{"empty":{},"null":null,"remote":{"type":"http"},"local":{"command":"ok-mcp"}}"#;
        let entries = parse_mcp_servers_json(body).expect("parses");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "local");

        let empty = r#"{"empty":{},"remote":{"type":"http"}}"#;
        let err = parse_mcp_servers_json(empty).expect_err("no installable server");
        assert!(err.to_string().contains("no MCP server found in this JSON"));
    }

    #[test]
    fn parses_timeout_and_load_fields() {
        let body = r#"{"docs":{"command":"docs-mcp","startup_timeout_seconds":7,"call_timeout_seconds":9,"tool_timeout_seconds":{"list":3},"disabled_tools":["write"],"auto_start":false,"alwaysLoad":true}}"#;
        let entries = parse_mcp_servers_json(body).expect("parses");
        let docs = &entries[0];
        assert_eq!(docs.startup_timeout_seconds, 7);
        assert_eq!(docs.call_timeout_seconds, 9);
        assert_eq!(docs.tool_timeout_seconds.get("list"), Some(&3));
        assert_eq!(docs.disabled_tools, vec!["write".to_owned()]);
        assert_eq!(docs.auto_start, Some(false));
        assert_eq!(docs.load, "always");
    }

    #[test]
    fn upsert_preserves_unknown_top_level_and_per_server_fields() {
        let body = r#"{
  "future_root_field": {"version": 2},
  "mcpServers": {
    "admin": {"command": "old-admin-mcp", "future_server_field": {"nested": true}}
  }
}"#;
        let (out, new) =
            upsert_mcp_json_server(body, &stdio_entry("admin", "admin-mcp")).expect("upsert");
        assert!(!new);

        let (root, servers) = read_mcp_json_raw(&out).expect("re-read");
        assert_eq!(
            root.get("future_root_field"),
            Some(&Value::Object(Map::from_iter([(
                "version".to_owned(),
                Value::Number(2.into())
            )])))
        );
        let admin = servers["admin"].as_object().expect("admin object");
        assert_eq!(
            admin.get("future_server_field"),
            Some(&Value::Object(Map::from_iter([(
                "nested".to_owned(),
                Value::Bool(true)
            )])))
        );
        assert_eq!(
            admin.get("command"),
            Some(&Value::String("admin-mcp".to_owned()))
        );
    }

    #[test]
    fn upsert_strips_retired_approval_fields_and_nested_modes() {
        let body = r#"{
  "mcpServers": {
    "admin": {
      "command": "old-admin-mcp",
      "trusted_read_only_tools": ["issue_read"],
      "default_tools_approval_mode": "prompt",
      "approvals_reviewer": "alice",
      "tools": {
        "wipe": {"approval_mode": "prompt", "enabled": false, "future": {"audit": true}},
        "external_only": {"enabled": false},
        "remove_keep": {"approval_mode": "writes", "enabled": true},
        "remove_entirely": {"approval_mode": "approve"}
      }
    }
  }
}"#;
        let (out, _) =
            upsert_mcp_json_server(body, &stdio_entry("admin", "admin-mcp")).expect("upsert");
        assert!(!out.contains("trusted_read_only_tools"));
        assert!(!out.contains("default_tools_approval_mode"));
        assert!(!out.contains("approvals_reviewer"));

        let (_, servers) = read_mcp_json_raw(&out).expect("re-read");
        let tools = servers["admin"]["tools"].as_object().expect("tools object");
        assert_eq!(tools.len(), 3);
        assert!(tools["wipe"].get("enabled").is_some());
        assert!(tools["wipe"].get("future").is_some());
        assert!(tools["external_only"].get("enabled").is_some());
        assert!(tools["remove_keep"].get("enabled").is_some());
        assert!(tools["remove_keep"].get("approval_mode").is_none());
        assert!(tools.get("remove_entirely").is_none());
    }

    #[test]
    fn stdio_write_drops_remote_fields_and_remote_write_drops_stdio_fields() {
        let body = r#"{"mcpServers":{"legacy":{"command":"old","url":"https://example.com","headers":{"X":"y"}}}}"#;
        let (out, _) =
            upsert_mcp_json_server(body, &stdio_entry("legacy", "new-mcp")).expect("stdio upsert");
        let (_, servers) = read_mcp_json_raw(&out).expect("re-read");
        let server = servers["legacy"].as_object().unwrap();
        assert!(server.get("url").is_none());
        assert!(server.get("headers").is_none());
        assert!(server.get("type").is_none());
        assert_eq!(
            server.get("command"),
            Some(&Value::String("new-mcp".to_owned()))
        );

        let remote = PluginEntry {
            name: "legacy".to_owned(),
            kind: "http".to_owned(),
            url: "https://mcp.example.com".to_owned(),
            ..PluginEntry::default()
        };
        let (out, _) = upsert_mcp_json_server(&out, &remote).expect("remote upsert");
        let (_, servers) = read_mcp_json_raw(&out).expect("re-read");
        let server = servers["legacy"].as_object().unwrap();
        assert!(server.get("command").is_none());
        assert!(server.get("args").is_none());
        assert_eq!(
            server.get("url"),
            Some(&Value::String("https://mcp.example.com".to_owned()))
        );
        assert_eq!(server.get("type"), Some(&Value::String("http".to_owned())));
    }

    #[test]
    fn empty_scalar_maps_and_zero_timeouts_are_not_written() {
        let body = r#"{"mcpServers":{"srv":{"command":"old"}}}"#;
        let entry = PluginEntry {
            name: "srv".to_owned(),
            command: "srv-mcp".to_owned(),
            args: Vec::new(),
            env: BTreeMap::new(),
            headers: BTreeMap::new(),
            startup_timeout_seconds: 0,
            call_timeout_seconds: 0,
            tool_timeout_seconds: BTreeMap::new(),
            disabled_tools: Vec::new(),
            auto_start: None,
            ..PluginEntry::default()
        };
        let (out, _) = upsert_mcp_json_server(body, &entry).expect("upsert");
        assert!(!out.contains("\"args\""));
        assert!(!out.contains("\"env\""));
        assert!(!out.contains("\"headers\""));
        assert!(!out.contains("timeout_seconds"));
        assert!(!out.contains("auto_start"));
        assert!(!out.contains("disabled_tools"));
    }

    #[test]
    fn remove_drops_only_the_target_server_and_keeps_the_rest() {
        let body = r#"{
  "future_root": true,
  "mcpServers": {
    "keep": {"command": "keep-mcp", "unknown": 1},
    "drop": {"command": "drop-mcp"}
  }
}"#;
        let (out, removed) = remove_mcp_json_server(body, "drop").expect("remove");
        assert!(removed);
        let (root, servers) = read_mcp_json_raw(&out).expect("re-read");
        assert!(servers.get("drop").is_none());
        assert!(servers.get("keep").is_some());
        assert_eq!(root.get("future_root"), Some(&Value::Bool(true)));

        // A missing entry leaves the body byte-for-byte unchanged.
        let (unchanged, removed) = remove_mcp_json_server(&out, "absent").expect("missing");
        assert!(!removed);
        assert_eq!(unchanged, out);
    }

    #[test]
    fn merge_keeps_toml_wins_and_unions_disabled_tools() {
        let declared = vec![PluginEntry {
            name: "shared".to_owned(),
            command: "toml-mcp".to_owned(),
            disabled_tools: vec!["a".to_owned()],
            ..PluginEntry::default()
        }];
        let incoming = vec![
            PluginEntry {
                name: "shared".to_owned(),
                command: "json-mcp".to_owned(),
                disabled_tools: vec!["b".to_owned()],
                ..PluginEntry::default()
            },
            PluginEntry {
                name: "json-only".to_owned(),
                command: "json-only-mcp".to_owned(),
                ..PluginEntry::default()
            },
        ];
        let merged = merge_mcp_json(&declared, incoming);
        assert_eq!(merged.len(), 2);
        let shared = &merged[0];
        assert_eq!(shared.command, "toml-mcp");
        assert_eq!(shared.disabled_tools, vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(merged[1].name, "json-only");
    }

    #[test]
    fn validation_refuses_bad_transport_and_timeouts() {
        let missing_command = stdio_entry("srv", "  ");
        assert!(validate_plugin(&missing_command).is_err());

        let remote = PluginEntry {
            name: "srv".to_owned(),
            kind: "http".to_owned(),
            url: String::new(),
            ..PluginEntry::default()
        };
        assert!(validate_plugin(&remote).is_err());

        let bad_timeout = PluginEntry {
            name: "srv".to_owned(),
            command: "mcp".to_owned(),
            call_timeout_seconds: -1,
            ..PluginEntry::default()
        };
        assert!(validate_plugin(&bad_timeout).is_err());

        let ok = stdio_entry("srv", "mcp");
        assert!(validate_plugin(&ok).is_ok());
    }
}
