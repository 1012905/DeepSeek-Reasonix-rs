//! Legacy-config migration trigger set and `REASONIX_DNS` splitting:
//! CLI-049, CLI-050. Address validity and the resolver install are backend
//! anchors; this layer only splits and trims the inherited raw value.

use reasonix_cli::env::{parse_dns_servers, should_migrate_legacy_config};

/// The 17 reference commands trigger the legacy migration; the bare invocation
/// is the empty command string.
#[test]
fn migration_trigger_set_matches_reference() {
    let migrate = [
        "", "run", "serve", "web", "setup", "config", "init", "acp", "mcp", "remote", "plugin",
        "subagent", "doctor", "upgrade", "update", "login", "whoami", "logout",
    ];
    for cmd in migrate {
        assert!(should_migrate_legacy_config(cmd), "{cmd}");
    }
}

/// Non-triggering commands (including the `doctor repair` and `schedule`
/// exclusions) do not request the migration.
#[test]
fn migration_skip_set_matches_reference() {
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
        "repair",
    ];
    for cmd in skip {
        assert!(!should_migrate_legacy_config(cmd), "{cmd}");
    }
}

/// DNS values split on commas, spaces, semicolons, newlines, tabs, and pipes;
/// fields are trimmed and empties dropped.
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
