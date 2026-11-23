//! The `[permissions]` and `[sandbox]` sections (docs/contracts/config.md §4).

use serde::Deserialize;

/// The host authorities an entry grants by default (config.go Default:
/// `host_authorities: ["ssh_agent"]`).
pub const DEFAULT_HOST_AUTHORITIES: [&str; 1] = ["ssh_agent"];

/// [permissions]: the per-call permission policy. Every rule is
/// `ToolName` or `ToolName(glob)`; precedence is deny > ask > allow >
/// fallback.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct PermissionsSection {
    /// The fallback decision for writer tools: ask|allow|deny. Read-only tools
    /// always fall back to allow.
    pub mode: String,
    pub allow: Vec<String>,
    pub ask: Vec<String>,
    pub deny: Vec<String>,
    /// User file only.
    pub allow_dynamic_bash: bool,
    /// User file only.
    pub ask_on_external_origin: bool,
}

impl Default for PermissionsSection {
    fn default() -> Self {
        // Mode "ask" with no rules keeps `reasonix run` autonomous (no TTY →
        // ask resolves to allow) while the TUI prompts before writers.
        Self {
            mode: "ask".to_owned(),
            allow: Vec::new(),
            ask: Vec::new(),
            deny: Vec::new(),
            allow_dynamic_bash: false,
            ask_on_external_origin: false,
        }
    }
}

impl PermissionsSection {
    /// The normalized fallback mode: ask|allow|deny, unknown reads ask.
    pub fn mode(&self) -> &'static str {
        match self.mode.trim().to_lowercase().as_str() {
            "allow" => "allow",
            "deny" => "deny",
            _ => "ask",
        }
    }
}

/// [sandbox]: the workspace jail. Values support `${VAR}` expansion but not
/// `~`; the platform decides whether bash can be jailed at all.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct SandboxSection {
    /// Empty = the current working directory.
    pub workspace_root: String,
    pub allow_write: Vec<String>,
    pub forbid_read: Vec<String>,
    /// enforce|off; empty = the platform default.
    pub bash: String,
    /// User file only: the Windows write-restricted token.
    pub windows_integrity: bool,
    /// Egress; the built-in default is true so an absent [sandbox] keeps it.
    pub network: bool,
    /// User-global.
    pub host_authorities: Vec<String>,
    /// User-global.
    pub allow_devices: Vec<String>,
    /// A repo may set these only where the user has not.
    pub allowed_domains: Vec<String>,
    /// A repo's additions join the user's.
    pub denied_domains: Vec<String>,
    /// User file only: "allow" stops the unlisted-host question.
    pub egress_unlisted: String,
}

impl Default for SandboxSection {
    fn default() -> Self {
        // Network=true here so an absent [sandbox] keeps egress; the zero value
        // would wrongly deny it.
        Self {
            workspace_root: String::new(),
            allow_write: Vec::new(),
            forbid_read: Vec::new(),
            bash: String::new(),
            windows_integrity: false,
            network: true,
            host_authorities: DEFAULT_HOST_AUTHORITIES
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            allow_devices: Vec::new(),
            allowed_domains: Vec::new(),
            denied_domains: Vec::new(),
            egress_unlisted: String::new(),
        }
    }
}

impl SandboxSection {
    /// The jail mode this platform can honour (config.go BashModeForGOOS):
    /// Windows has no OS-level bash sandbox, so it enforces only under
    /// windows_integrity; elsewhere enforce|off, empty meaning enforce.
    pub fn bash_mode(&self) -> &'static str {
        if cfg!(windows) {
            if self.windows_integrity {
                "enforce"
            } else {
                "off"
            }
        } else {
            match self.bash.trim().to_lowercase().as_str() {
                "off" => "off",
                _ => "enforce",
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_HOST_AUTHORITIES, PermissionsSection, SandboxSection};

    #[test]
    fn permissions_default_to_ask_with_no_rules() {
        // config.md §2: Permissions{Mode: "ask"}.
        let permissions = PermissionsSection::default();
        assert_eq!(permissions.mode, "ask");
        assert_eq!(permissions.mode(), "ask");
        assert!(permissions.allow.is_empty());
        assert!(permissions.ask.is_empty());
        assert!(permissions.deny.is_empty());
        assert!(!permissions.allow_dynamic_bash);
        assert!(!permissions.ask_on_external_origin);
    }

    #[test]
    fn the_permission_mode_matrix() {
        let mut permissions = PermissionsSection::default();
        for (raw, want) in [
            ("ALLOW", "allow"),
            (" deny ", "deny"),
            ("ask", "ask"),
            ("maybe", "ask"),
            ("", "ask"),
        ] {
            permissions.mode = raw.to_owned();
            assert_eq!(permissions.mode(), want, "mode = {raw:?}");
        }
    }

    #[test]
    fn the_sandbox_defaults_keep_egress_and_ssh_agent() {
        // config.md §2: Sandbox{Network: true, HostAuthorities: ["ssh_agent"]}.
        let sandbox = SandboxSection::default();
        assert!(sandbox.network);
        assert_eq!(
            sandbox.host_authorities,
            DEFAULT_HOST_AUTHORITIES
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>()
        );
        assert!(sandbox.workspace_root.is_empty());
        assert!(sandbox.allow_write.is_empty());
        assert!(!sandbox.windows_integrity);
        assert!(sandbox.egress_unlisted.is_empty());
    }

    #[test]
    fn bash_mode_follows_the_platform() {
        // config.md §4 BashModeForGOOS: Windows enforces only under
        // windows_integrity; elsewhere off is the one way out.
        let mut sandbox = SandboxSection::default();
        if cfg!(windows) {
            assert_eq!(sandbox.bash_mode(), "off");
            sandbox.bash = "enforce".to_owned();
            assert_eq!(sandbox.bash_mode(), "off");
            sandbox.windows_integrity = true;
            assert_eq!(sandbox.bash_mode(), "enforce");
        } else {
            assert_eq!(sandbox.bash_mode(), "enforce");
            sandbox.bash = " enforce ".to_owned();
            assert_eq!(sandbox.bash_mode(), "enforce");
            sandbox.bash = "OFF".to_owned();
            assert_eq!(sandbox.bash_mode(), "off");
            sandbox.bash = "unrecognized".to_owned();
            assert_eq!(sandbox.bash_mode(), "enforce");
        }
    }
}
