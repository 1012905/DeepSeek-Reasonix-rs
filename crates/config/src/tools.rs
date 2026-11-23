//! The `[tools]` section and its timeout defaults (config.go ToolsConfig).

use std::collections::BTreeMap;

use serde::Deserialize;

/// Built-in bash timeout cap in seconds (defaultBashTimeoutSeconds).
pub const DEFAULT_BASH_TIMEOUT_SECONDS: i64 = 120;
/// Built-in MCP startup cap (defaultMCPStartupTimeoutSeconds).
pub const DEFAULT_MCP_STARTUP_TIMEOUT_SECONDS: i64 = 30;
/// Built-in MCP per-call cap (defaultMCPCallTimeoutSeconds).
pub const DEFAULT_MCP_CALL_TIMEOUT_SECONDS: i64 = 300;

/// [tools]: the tool catalogue switches and the timeout ceilings.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct ToolsSection {
    /// Empty = every built-in tool.
    pub enabled: Vec<String>,
    /// A project file can only add to this list.
    pub disabled: Vec<String>,
    /// Bash cap; 0 disables the tool-local cap, negative keeps the default.
    pub bash_timeout_seconds: Option<i64>,
    /// Startup + tools/list cap; <= 0 keeps the default.
    pub mcp_startup_timeout_seconds: Option<i64>,
    /// Per-call cap; <= 0 keeps the default.
    pub mcp_call_timeout_seconds: Option<i64>,
    pub protect_changed_files: Option<bool>,
    pub browser_tools: Option<bool>,
    pub setting_tool: Option<bool>,
    /// User file only; default off.
    pub mcp_resources: bool,
    /// User file only; default on.
    pub present_files: bool,
    /// User file only; default off.
    pub video_url_passthrough: bool,
    /// Per-server `always`|`deferred` loading.
    pub mcp_load: BTreeMap<String, String>,
}

impl Default for ToolsSection {
    fn default() -> Self {
        // PresentFiles is set explicitly here, not left to the zero value: an
        // absent [tools] still presents files (config.go Default).
        Self {
            enabled: Vec::new(),
            disabled: Vec::new(),
            bash_timeout_seconds: None,
            mcp_startup_timeout_seconds: None,
            mcp_call_timeout_seconds: None,
            protect_changed_files: None,
            browser_tools: None,
            setting_tool: None,
            mcp_resources: false,
            present_files: true,
            video_url_passthrough: false,
            mcp_load: BTreeMap::new(),
        }
    }
}

impl ToolsSection {
    /// The bash timeout: an absent or negative value keeps the built-in cap; 0
    /// turns the tool-local cap off (config.go BashTimeoutSeconds).
    pub fn bash_timeout_seconds(&self) -> i64 {
        match self.bash_timeout_seconds {
            None | Some(..0) => DEFAULT_BASH_TIMEOUT_SECONDS,
            Some(seconds) => seconds,
        }
    }

    /// The MCP startup cap: absent, zero, and negative keep the built-in cap
    /// so a slow but healthy server can outlive the short interactive wait
    /// without running forever.
    pub fn mcp_startup_timeout_seconds(&self) -> i64 {
        match self.mcp_startup_timeout_seconds {
            Some(seconds) if seconds > 0 => seconds,
            _ => DEFAULT_MCP_STARTUP_TIMEOUT_SECONDS,
        }
    }

    /// The MCP per-call cap: absent, zero, and negative keep the built-in cap
    /// so a hung server cannot block a turn indefinitely.
    pub fn mcp_call_timeout_seconds(&self) -> i64 {
        match self.mcp_call_timeout_seconds {
            Some(seconds) if seconds > 0 => seconds,
            _ => DEFAULT_MCP_CALL_TIMEOUT_SECONDS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_BASH_TIMEOUT_SECONDS, DEFAULT_MCP_CALL_TIMEOUT_SECONDS,
        DEFAULT_MCP_STARTUP_TIMEOUT_SECONDS, ToolsSection,
    };

    #[test]
    fn the_bash_cap_keeps_the_default_for_absent_and_negative() {
        // config.go BashTimeoutSeconds: nil and negative → 120; 0 disables.
        let mut tools = ToolsSection::default();
        assert_eq!(tools.bash_timeout_seconds(), DEFAULT_BASH_TIMEOUT_SECONDS);
        tools.bash_timeout_seconds = Some(-5);
        assert_eq!(tools.bash_timeout_seconds(), DEFAULT_BASH_TIMEOUT_SECONDS);
        tools.bash_timeout_seconds = Some(0);
        assert_eq!(tools.bash_timeout_seconds(), 0);
        tools.bash_timeout_seconds = Some(30);
        assert_eq!(tools.bash_timeout_seconds(), 30);
    }

    #[test]
    fn the_mcp_caps_keep_the_default_until_a_positive_value() {
        // config.go MCP{Startup,Call}TimeoutSeconds: <= 0 keeps the default.
        let mut tools = ToolsSection::default();
        assert_eq!(
            tools.mcp_startup_timeout_seconds(),
            DEFAULT_MCP_STARTUP_TIMEOUT_SECONDS
        );
        assert_eq!(
            tools.mcp_call_timeout_seconds(),
            DEFAULT_MCP_CALL_TIMEOUT_SECONDS
        );
        tools.mcp_startup_timeout_seconds = Some(0);
        tools.mcp_call_timeout_seconds = Some(-1);
        assert_eq!(
            tools.mcp_startup_timeout_seconds(),
            DEFAULT_MCP_STARTUP_TIMEOUT_SECONDS
        );
        assert_eq!(
            tools.mcp_call_timeout_seconds(),
            DEFAULT_MCP_CALL_TIMEOUT_SECONDS
        );
        tools.mcp_startup_timeout_seconds = Some(60);
        tools.mcp_call_timeout_seconds = Some(600);
        assert_eq!(tools.mcp_startup_timeout_seconds(), 60);
        assert_eq!(tools.mcp_call_timeout_seconds(), 600);
    }

    #[test]
    fn the_section_defaults_carry_present_files() {
        // config.md §2: Tools{PresentFiles: true}; everything else is zero.
        let tools = ToolsSection::default();
        assert!(tools.present_files);
        assert!(!tools.mcp_resources);
        assert!(!tools.video_url_passthrough);
        assert!(tools.enabled.is_empty());
        assert!(tools.disabled.is_empty());
        assert!(tools.protect_changed_files.is_none());
        assert!(tools.browser_tools.is_none());
        assert!(tools.setting_tool.is_none());
        assert!(tools.mcp_load.is_empty());
    }
}
