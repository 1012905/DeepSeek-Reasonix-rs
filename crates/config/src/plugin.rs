//! `[[plugins]]`: external MCP servers and their command-line normalization.

use std::collections::BTreeMap;

use serde::Deserialize;

use reasonix_core::shellparse::static_fields;

/// One external MCP server (plugin_entry.go). The fields mirror Claude Code's
/// `mcpServers` spec so an entry can come from either file.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct PluginEntry {
    pub name: String,
    /// `stdio` (default) | `http` (a.k.a. streamable-http) | `sse`.
    pub kind: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    /// Raw tool names from this server that stay hidden.
    pub disabled_tools: Vec<String>,
    pub startup_timeout_seconds: i64,
    pub call_timeout_seconds: i64,
    pub tool_timeout_seconds: BTreeMap<String, i64>,
    pub oauth_allow_missing_pkce_metadata: bool,
    /// `parallel` (default) | `serial`.
    pub concurrency: String,
    /// None preserves the historical behavior: a configured server starts.
    pub auto_start: Option<bool>,
    /// `always` puts the tools in the provider schema from session start.
    pub load: String,
    /// Retired startup tier; a load strips it.
    pub tier: String,
}

impl PluginEntry {
    /// The transport this entry speaks, defaulting to stdio
    /// (mcp_command.go pluginEntryTransport).
    pub fn transport(&self) -> String {
        match self.kind.trim().to_lowercase().as_str() {
            "" | "stdio" => "stdio".to_owned(),
            "http" | "streamable-http" => "http".to_owned(),
            "sse" => "sse".to_owned(),
            other => other.to_owned(),
        }
    }
}

/// Repairs the common copy/paste mistake where a tutorial's whole command line
/// lands in `command` while `args` stays empty (mcp_command.go
/// NormalizePluginCommandLine). Reports whether it changed anything.
pub fn normalize_plugin_command_line(entry: &PluginEntry) -> (PluginEntry, bool) {
    let mut entry = entry.clone();
    if entry.transport() != "stdio" || !entry.args.is_empty() {
        entry.command = entry.command.trim().to_owned();
        return (entry, false);
    }
    let command = entry.command.trim().to_owned();
    entry.command = command.clone();
    if !command.contains([' ', '\t', '\r', '\n']) {
        return (entry, false);
    }
    let Some(parts) = split_plugin_command_line(&command) else {
        return (entry, false);
    };
    if parts.len() < 2 || !should_split_plugin_command(&command, &parts[0]) {
        return (entry, false);
    }
    entry.command = parts[0].clone();
    entry.args = parts[1..].to_vec();
    (entry, true)
}

fn should_split_plugin_command(original: &str, first: &str) -> bool {
    let trimmed = original.trim_start();
    if trimmed.starts_with('"') || trimmed.starts_with('\'') {
        return true;
    }
    known_mcp_command_runner(first) || !first.contains(['/', '\\'])
}

fn known_mcp_command_runner(command: &str) -> bool {
    let base = command.replace('\\', "/");
    let base = base.rsplit('/').next().unwrap_or_default();
    let mut base = base.to_lowercase();
    for extension in [".exe", ".cmd", ".bat", ".ps1"] {
        if let Some(stripped) = base.strip_suffix(extension) {
            base = stripped.to_owned();
        }
    }
    matches!(
        base.as_str(),
        "npx"
            | "npm"
            | "node"
            | "pnpm"
            | "yarn"
            | "bun"
            | "uvx"
            | "uv"
            | "python"
            | "python3"
            | "py"
            | "docker"
            | "deno"
            | "go"
            | "cmd"
            | "powershell"
            | "pwsh"
    )
}

/// Splits a static command line, refusing an unquoted Windows path that
/// happens to carry spaces (mcp_command.go splitPluginCommandLine).
fn split_plugin_command_line(command: &str) -> Option<Vec<String>> {
    if preserves_unquoted_windows_path(command) {
        return None;
    }
    let (fields, malformed) = static_fields(command);
    if !malformed.is_empty() {
        return None;
    }
    Some(fields)
}

fn preserves_unquoted_windows_path(command: &str) -> bool {
    let trimmed = command.trim_start();
    if trimmed.starts_with('"') || trimmed.starts_with('\'') {
        return false;
    }
    let first = trimmed.split([' ', '\t']).next().unwrap_or_default();
    first.contains('\\') && trimmed.contains([' ', '\t', '\r', '\n'])
}

/// True when any `disabled_tools` entry is blank (disabled_tools_merge.go
/// validateDisabledTools), phrased as the reference states it.
pub fn validate_disabled_tools(entry: &PluginEntry) -> Option<String> {
    if entry
        .disabled_tools
        .iter()
        .any(|name| name.trim().is_empty())
    {
        return Some(format!(
            "plugin {:?}: disabled_tools contains an empty tool name",
            entry.name
        ));
    }
    None
}

/// The union that keeps an earlier source's disabled tools when a later source
/// declares the same server name (disabled_tools_merge.go
/// mergeDisabledToolPolicies).
pub fn merge_disabled_tool_policies(base: &[String], over: &[String]) -> Vec<String> {
    let mut merged = base.to_vec();
    for name in over {
        if !merged.iter().any(|existing| existing == name) {
            merged.push(name.clone());
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::{
        PluginEntry, merge_disabled_tool_policies, normalize_plugin_command_line,
        validate_disabled_tools,
    };

    fn stdio(command: &str) -> PluginEntry {
        PluginEntry {
            name: "srv".to_owned(),
            command: command.to_owned(),
            ..PluginEntry::default()
        }
    }

    #[test]
    fn a_plain_command_stays_whole() {
        let (entry, changed) = normalize_plugin_command_line(&stdio("npx"));
        assert!(!changed);
        assert_eq!(entry.command, "npx");
        assert!(entry.args.is_empty());
    }

    #[test]
    fn a_full_command_line_splits_into_command_and_args() {
        // The tutorial copy/paste: runner in `command`, nothing in `args`.
        let (entry, changed) =
            normalize_plugin_command_line(&stdio("npx -y @modelcontextprotocol/server-memory"));
        assert!(changed);
        assert_eq!(entry.command, "npx");
        assert_eq!(
            entry.args,
            vec![
                "-y".to_owned(),
                "@modelcontextprotocol/server-memory".to_owned()
            ]
        );
        let (entry, changed) = normalize_plugin_command_line(&stdio("  custom-mcp --stdio  "));
        assert!(changed);
        assert_eq!(entry.command, "custom-mcp");
        assert_eq!(entry.args, vec!["--stdio".to_owned()]);
    }

    #[test]
    fn declared_args_are_never_reshaped() {
        let mut entry = stdio("npx -y pkg");
        entry.args = vec!["existing".to_owned()];
        let (got, changed) = normalize_plugin_command_line(&entry);
        assert!(!changed);
        assert_eq!(got.command, "npx -y pkg");
        assert_eq!(got.args, vec!["existing".to_owned()]);
    }

    #[test]
    fn an_unquoted_windows_path_keeps_its_spaces() {
        // mcp_command.go preservesUnquotedWindowsPath: a path-like first token
        // with a space after it is the command itself, not a runner line.
        let raw = r"C:\Program Files\srv\srv.exe --stdio";
        let (entry, changed) = normalize_plugin_command_line(&stdio(raw));
        assert!(!changed, "entry = {entry:?}");
        assert_eq!(entry.command, raw);
        assert!(entry.args.is_empty());
    }

    #[test]
    fn a_quoted_windows_path_splits() {
        let raw = r#""C:\Program Files\srv\srv.exe" --stdio"#;
        let (entry, changed) = normalize_plugin_command_line(&stdio(raw));
        assert!(changed, "entry = {entry:?}");
        assert_eq!(entry.command, r"C:\Program Files\srv\srv.exe");
        assert_eq!(entry.args, vec!["--stdio".to_owned()]);
    }

    #[test]
    fn a_remote_transport_only_trims() {
        let entry = PluginEntry {
            name: "remote".to_owned(),
            kind: "sse".to_owned(),
            command: "  npx -y pkg  ".to_owned(),
            ..PluginEntry::default()
        };
        let (got, changed) = normalize_plugin_command_line(&entry);
        assert!(!changed);
        assert_eq!(got.command, "npx -y pkg");
        assert!(got.args.is_empty());
    }

    #[test]
    fn shell_syntax_is_left_alone() {
        let (entry, changed) = normalize_plugin_command_line(&stdio("npx -y pkg | tee out"));
        assert!(!changed, "entry = {entry:?}");
    }

    #[test]
    fn transports_normalize_as_the_loader_expects() {
        for (raw, want) in [
            ("", "stdio"),
            (" STDIO ", "stdio"),
            ("http", "http"),
            ("streamable-http", "http"),
            ("SSE", "sse"),
            ("weird", "weird"),
        ] {
            let entry = PluginEntry {
                kind: raw.to_owned(),
                ..PluginEntry::default()
            };
            assert_eq!(entry.transport(), want, "kind = {raw:?}");
        }
    }

    #[test]
    fn disabled_tool_lists_merge_and_refuse_blanks() {
        // disabled_tools_merge.go: union preserving the earlier source's order.
        let merged = merge_disabled_tool_policies(
            &["a".to_owned(), "b".to_owned()],
            &["b".to_owned(), "c".to_owned()],
        );
        assert_eq!(merged, vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]);
        let entry = PluginEntry {
            name: "srv".to_owned(),
            disabled_tools: vec![" ".to_owned()],
            ..PluginEntry::default()
        };
        assert_eq!(
            validate_disabled_tools(&entry).as_deref(),
            Some("plugin \"srv\": disabled_tools contains an empty tool name")
        );
        assert!(validate_disabled_tools(&PluginEntry::default()).is_none());
    }
}
