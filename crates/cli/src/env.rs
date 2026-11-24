//! Process-start environment facts: the legacy-config migration trigger set
//! and the `REASONIX_DNS` server list.
//!
//! Parity targets: `internal/frontend/cli/cli.go:182-189`
//! (`shouldMigrateLegacyConfigForCLI`) and `internal/base/netdns` (`Parse`).
//! Address validity itself belongs to the resolver backend; this layer only
//! splits and trims the raw value the process inherited.

/// Commands that migrate a legacy config before anything else reads it
/// (cli.go:184). The bare invocation is the empty command string.
pub fn should_migrate_legacy_config(command: &str) -> bool {
    matches!(
        command,
        "" | "run"
            | "serve"
            | "web"
            | "setup"
            | "config"
            | "init"
            | "acp"
            | "mcp"
            | "remote"
            | "plugin"
            | "subagent"
            | "doctor"
            | "upgrade"
            | "update"
            | "login"
            | "whoami"
            | "logout"
    )
}

/// Splits a `REASONIX_DNS` value on commas, spaces, semicolons, newlines, tabs,
/// and pipes; trims each field and drops empties. `netdns.Parse` additionally
/// validates each entry as an address/`host:port` and defaults the port; that
/// validation lives with the resolver backend.
pub fn parse_dns_servers(raw: &str) -> Vec<String> {
    raw.split(|c: char| [',', ' ', ';', '\n', '\t', '|'].contains(&c))
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_trigger_set_matches_reference() {
        let migrate = [
            "", "run", "serve", "web", "setup", "config", "init", "acp", "mcp", "remote", "plugin",
            "subagent", "doctor", "upgrade", "update", "login", "whoami", "logout",
        ];
        for cmd in migrate {
            assert!(should_migrate_legacy_config(cmd), "{cmd}");
        }
        let skip = [
            "tui",
            "chat",
            "code",
            "review",
            "report",
            "session",
            "hook",
            "trust",
            "task",
            "completion",
            "version",
            "schedule",
            "sandbox",
        ];
        for cmd in skip {
            assert!(!should_migrate_legacy_config(cmd), "{cmd}");
        }
    }

    #[test]
    fn dns_split_on_all_separators_and_trims() {
        assert_eq!(
            parse_dns_servers("1.1.1.1, 8.8.8.8;9.9.9.9\teno|example.net"),
            vec![
                "1.1.1.1".to_owned(),
                "8.8.8.8".to_owned(),
                "9.9.9.9".to_owned(),
                "eno".to_owned(),
                "example.net".to_owned(),
            ]
        );
        assert!(parse_dns_servers("  , ; ").is_empty());
        assert!(parse_dns_servers("").is_empty());
    }
}
