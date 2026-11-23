//! Inventory of entries written at the top of the state root (paths.md §1.2,
//! config.md §12.4).
//!
//! The relocation guard reads this list so an entry added later is not stranded
//! in the previous root when the state root moves. Root resolution itself
//! belongs to reasonix-config; this slice owns the inventory and its subsets.

/// Every entry written at the top of the state root, in reference order
/// (Go `StateRootEntries`).
pub const STATE_ROOT_ENTRIES: &[&str] = &[
    "sessions",
    "archive",
    "stats",
    "projects",
    "appearance",
    "themes",
    "repair",
    "serve-workspaces.json",
    "memory",
    "schedules",
    "peers",
    "trusted",
    "machine-id.key",
    "mcp-global-migration-v1",
    "commands",
    "rules",
    "agent-presets",
    "sandbox",
    "workspaces",
    "REASONIX.md",
    "AGENTS.md",
    "CLAUDE.md",
    "REASONIX.local.md",
    "AGENTS.local.md",
    "CLAUDE.local.md",
];

/// The subset the first relocation owned; the remainder stays in the earlier
/// root until recovery adopts it (Go `stateRootEntriesFirstMove`).
pub const STATE_ROOT_ENTRIES_FIRST_MOVE: &[&str] = &["sessions", "archive", "stats", "projects"];

/// Entries the first move left in the previous root (Go
/// `StateRootEntriesEarlyMovesLeft`). The recovery routine and the storage-panel
/// notice read this list.
pub fn early_moves_left() -> Vec<&'static str> {
    STATE_ROOT_ENTRIES
        .iter()
        .copied()
        .filter(|name| !STATE_ROOT_ENTRIES_FIRST_MOVE.contains(name))
        .collect()
}

/// Whether `name` is a declared state-root entry.
pub fn is_state_root_entry(name: &str) -> bool {
    STATE_ROOT_ENTRIES.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_list_matches_reference() {
        let expected: &[&str] = &[
            "sessions",
            "archive",
            "stats",
            "projects",
            "appearance",
            "themes",
            "repair",
            "serve-workspaces.json",
            "memory",
            "schedules",
            "peers",
            "trusted",
            "machine-id.key",
            "mcp-global-migration-v1",
            "commands",
            "rules",
            "agent-presets",
            "sandbox",
            "workspaces",
            "REASONIX.md",
            "AGENTS.md",
            "CLAUDE.md",
            "REASONIX.local.md",
            "AGENTS.local.md",
            "CLAUDE.local.md",
        ];
        assert_eq!(STATE_ROOT_ENTRIES, expected);
    }

    #[test]
    fn first_move_subset_is_declared_entries() {
        for name in STATE_ROOT_ENTRIES_FIRST_MOVE {
            assert!(
                STATE_ROOT_ENTRIES.contains(name),
                "first-move entry {name} is not in the inventory"
            );
        }
    }

    #[test]
    fn early_moves_left_is_inventory_minus_first_move() {
        let left = early_moves_left();
        assert_eq!(
            left.len(),
            STATE_ROOT_ENTRIES.len() - STATE_ROOT_ENTRIES_FIRST_MOVE.len()
        );
        for name in &left {
            assert!(!STATE_ROOT_ENTRIES_FIRST_MOVE.contains(name));
            assert!(STATE_ROOT_ENTRIES.contains(name));
        }
        for name in STATE_ROOT_ENTRIES_FIRST_MOVE {
            assert!(!left.contains(name));
        }
    }

    #[test]
    fn membership_predicate() {
        assert!(is_state_root_entry("sessions"));
        assert!(is_state_root_entry("CLAUDE.local.md"));
        assert!(!is_state_root_entry("config.toml"));
        assert!(!is_state_root_entry(""));
    }
}
