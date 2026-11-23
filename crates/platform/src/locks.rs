//! Derived lock directories under the locks root (paths.md §1.3, config.md
//! §12.1/§12.3).
//!
//! The locks root converges on the OS user cache on purpose: two isolated
//! instances must find the same lock, so the root ignores Reasonix home/cache
//! overrides. Resolving that root — the OS cache base and the convergence rule —
//! is reasonix-config's job; here the resolved root is injected and only the
//! owned subdirectories are derived.

use std::path::{Path, PathBuf};

pub const WORKSPACE_LEASE_DIR_NAME: &str = "workspace-leases";
pub const REPAIR_MUTATION_LOCK_DIR_NAME: &str = "repair-mutation-locks";
pub const MEMORY_STORE_LOCK_DIR_NAME: &str = "memory-store-locks";

/// The entries the locks root claims inside its shared cache parent; a size
/// report must not count the whole cache twice (Go `RootOwns(RootLocks)`).
pub const LOCK_ROOT_OWNED_DIRS: [&str; 3] = [
    WORKSPACE_LEASE_DIR_NAME,
    REPAIR_MUTATION_LOCK_DIR_NAME,
    MEMORY_STORE_LOCK_DIR_NAME,
];

fn join(locks_root: &Path, name: &str) -> Option<PathBuf> {
    // Go `joinRoot`: an empty base yields an empty path, never a relative one.
    if locks_root.as_os_str().is_empty() {
        return None;
    }
    Some(locks_root.join(name))
}

/// `<locks>/workspace-leases` — one lease per live workspace process.
pub fn workspace_lease_dir(locks_root: &Path) -> Option<PathBuf> {
    join(locks_root, WORKSPACE_LEASE_DIR_NAME)
}

/// `<locks>/repair-mutation-locks` — serialized mutation repair.
pub fn repair_mutation_lock_dir(locks_root: &Path) -> Option<PathBuf> {
    join(locks_root, REPAIR_MUTATION_LOCK_DIR_NAME)
}

/// `<locks>/memory-store-locks` — serialized memory-store rewrites.
pub fn memory_lock_dir(locks_root: &Path) -> Option<PathBuf> {
    join(locks_root, MEMORY_STORE_LOCK_DIR_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_dirs_join_under_injected_root() {
        let root = Path::new("/run/cache/reasonix");
        assert_eq!(
            workspace_lease_dir(root),
            Some(root.join(WORKSPACE_LEASE_DIR_NAME))
        );
        assert_eq!(
            repair_mutation_lock_dir(root),
            Some(root.join(REPAIR_MUTATION_LOCK_DIR_NAME))
        );
        assert_eq!(
            memory_lock_dir(root),
            Some(root.join(MEMORY_STORE_LOCK_DIR_NAME))
        );
    }

    #[test]
    fn owned_dirs_match_the_three_lock_subdirs() {
        let root = Path::new("/run/cache/reasonix");
        let owned = [
            workspace_lease_dir(root).unwrap(),
            repair_mutation_lock_dir(root).unwrap(),
            memory_lock_dir(root).unwrap(),
        ];
        for (dir, name) in owned.iter().zip(LOCK_ROOT_OWNED_DIRS) {
            assert_eq!(dir.file_name().unwrap(), name);
        }
        assert_eq!(LOCK_ROOT_OWNED_DIRS.len(), 3);
    }

    #[test]
    fn empty_root_yields_no_dir() {
        let empty = Path::new("");
        assert!(workspace_lease_dir(empty).is_none());
        assert!(repair_mutation_lock_dir(empty).is_none());
        assert!(memory_lock_dir(empty).is_none());
    }
}
