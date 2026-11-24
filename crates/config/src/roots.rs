//! The storage-root table and its resolution chain (storage_roots.go,
//! docs/contracts/config.md §11).
//!
//! The root set is closed: every directory the runtime writes belongs to
//! exactly one root, so usage accounting, the storage panel, and relocation
//! enumerate roots instead of each keeping its own list of places to look.
//! One precedence chain serves all five — record pin, then configuration, then
//! the root's own default — so relocation and isolation compose the same way.

use std::path::{Path, PathBuf};

use crate::paths::{
    DirEnv, clean_env_dir_in, default_home_dir_in, os_cache_base_in, user_config_dir_in,
    user_home_dir_in,
};

/// Every directory the runtime writes belongs to exactly one root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub enum RootId {
    /// config.toml and the credentials .env. Not relocatable: nothing can name
    /// the location of the file that names locations.
    Home,
    /// Transcripts, archives, stats, and per-project memory.
    State,
    /// Listing catalogs and every other derived artefact.
    Cache,
    /// The isolated git worktrees Delivery checks out.
    Worktrees,
    /// Cross-process locks two isolated runtimes must both find. Not
    /// relocatable: it is pinned to the OS user.
    Locks,
}

impl RootId {
    /// The id spelled in `[storage]` and diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            RootId::Home => "home",
            RootId::State => "state",
            RootId::Cache => "cache",
            RootId::Worktrees => "worktrees",
            RootId::Locks => "locks",
        }
    }

    /// The root with this id, if the table declares one.
    pub fn parse(raw: &str) -> Option<RootId> {
        ROOT_TABLE
            .iter()
            .find(|root| root.id.as_str() == raw.trim())
            .map(|root| root.id)
    }
}

/// One declared root: what pins it, whether a user may move it, and where it
/// lands when nothing says.
#[derive(Debug, Clone, Copy)]
pub struct StorageRoot {
    pub id: RootId,
    /// The variable pinning this root; "" when none names it.
    pub env: &'static str,
    pub relocatable: bool,
    /// Entries this root may claim inside a directory it shares.
    pub owns: &'static [&'static str],
}

/// The locks root's three subdirectories. Two isolated runtimes share the OS
/// cache, so a move and a size report must not assume the root owns
/// everything beneath it.
pub const LOCK_DIRS: [&str; 3] = [
    "workspace-leases",
    "repair-mutation-locks",
    "memory-store-locks",
];

/// Everything written at the top of the state root. An entry missing here is
/// stranded by a relocation while the code reads only the new root.
pub const STATE_ROOT_ENTRIES: [&str; 25] = [
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

/// What relocation owned when it shipped.
const STATE_ROOT_ENTRIES_FIRST_MOVE: [&str; 4] = ["sessions", "archive", "stats", "projects"];

/// The declared table, in a stable order so listings and diagnostics read the
/// same way every time.
pub const ROOT_TABLE: [StorageRoot; 5] = [
    StorageRoot {
        id: RootId::Home,
        env: "REASONIX_HOME",
        relocatable: false,
        owns: &[],
    },
    StorageRoot {
        id: RootId::State,
        env: "REASONIX_STATE_HOME",
        relocatable: true,
        owns: &STATE_ROOT_ENTRIES,
    },
    StorageRoot {
        id: RootId::Cache,
        env: "REASONIX_CACHE_HOME",
        relocatable: true,
        owns: &[],
    },
    StorageRoot {
        id: RootId::Worktrees,
        env: "",
        relocatable: true,
        owns: &[],
    },
    StorageRoot {
        id: RootId::Locks,
        env: "",
        relocatable: false,
        owns: &LOCK_DIRS,
    },
];

/// Every declared root id in table order.
pub fn root_ids() -> Vec<RootId> {
    ROOT_TABLE.iter().map(|root| root.id).collect()
}

/// The declared root, if the id is in the table.
pub fn lookup_root(id: RootId) -> Option<&'static StorageRoot> {
    ROOT_TABLE.iter().find(|root| root.id == id)
}

/// Whether a user may move this root. Callers that offer relocation read it
/// rather than listing the immovable roots themselves.
pub fn root_relocatable(id: RootId) -> bool {
    lookup_root(id).is_some_and(|root| root.relocatable)
}

/// The entries a root may claim inside its directory, empty when it has the
/// directory to itself.
pub fn root_owns(id: RootId) -> Vec<String> {
    lookup_root(id)
        .map(|root| root.owns.iter().map(|name| (*name).to_owned()).collect())
        .unwrap_or_default()
}

/// What a move made by an earlier version left in the previous root: every
/// state entry the first relocation did not own.
pub fn state_root_entries_early_moves_left() -> Vec<String> {
    STATE_ROOT_ENTRIES
        .iter()
        .filter(|name| !STATE_ROOT_ENTRIES_FIRST_MOVE.contains(name))
        .map(|name| (*name).to_owned())
        .collect()
}

/// The environment one root chain resolves against. Real callers snapshot the
/// process environment; tests inject one.
#[derive(Debug, Clone, Default)]
pub struct RootEnv {
    pub env: DirEnv,
    /// A stated home. It outranks REASONIX_HOME: once a host has worked out
    /// which home to use, it says so, and one process may serve a second
    /// session that says something else.
    pub stated_home: PathBuf,
    /// `[storage]` relocations keyed by root id, already expanded.
    pub configured: Vec<(RootId, PathBuf)>,
}

impl RootEnv {
    /// The binding of what the environment says, with nothing stated and no
    /// configuration applied.
    pub fn process() -> Self {
        Self {
            env: DirEnv::process(),
            stated_home: PathBuf::new(),
            configured: Vec::new(),
        }
    }

    /// States a home for this binding. An empty `home` follows the
    /// environment, so a caller with nothing to say passes on what it was given.
    pub fn for_home(home: &str) -> Self {
        let mut binding = Self::process();
        binding.stated_home = clean_env_dir_in_values(home, &binding.env);
        binding
    }

    /// Applies `[storage]` relocations. Only relocatable roots move, and a
    /// value expands the way an environment pin does.
    pub fn with_storage(mut self, storage: &std::collections::BTreeMap<String, String>) -> Self {
        for (key, dir) in storage {
            let Some(id) = RootId::parse(key) else {
                continue;
            };
            if !root_relocatable(id) {
                continue;
            }
            let expanded = clean_env_dir_in_values(dir, &self.env);
            if !expanded.as_os_str().is_empty() {
                self.configured.push((id, expanded));
            }
        }
        self
    }

    /// The whole chain for one root: what holds it in place, then what the
    /// configuration chose, then the root's own default.
    pub fn dir(&self, id: RootId) -> PathBuf {
        let Some(root) = lookup_root(id) else {
            return PathBuf::new();
        };
        let pinned = self.pinned_root_dir(root);
        if !pinned.as_os_str().is_empty() {
            return pinned;
        }
        let configured = self.configured_root_dir(id);
        if !configured.as_os_str().is_empty() {
            return configured;
        }
        self.default_dir(root)
    }

    /// The folder a first launch opens when nobody has chosen a project: a
    /// small one under the state root, empty when that root cannot resolve
    /// (paths.go AutoWorkspaceDir). The caller creates it.
    pub fn auto_workspace_dir(&self) -> PathBuf {
        let base = self.dir(RootId::State);
        if base.as_os_str().is_empty() {
            return PathBuf::new();
        }
        base.join("workspaces").join("auto")
    }

    /// What holds a root in place before the configuration is read.
    fn pinned_root_dir(&self, root: &StorageRoot) -> PathBuf {
        if root.id == RootId::Home && !self.stated_home.as_os_str().is_empty() {
            return self.stated_home.clone();
        }
        if root.env.is_empty() {
            return PathBuf::new();
        }
        clean_env_dir_in(root.env, &self.env)
    }

    /// The home only insofar as something deliberate holds it there: stated,
    /// or named by REASONIX_HOME. A caller telling an isolated runtime from a
    /// default one reads this, never the resolved home.
    pub fn pinned_home_dir(&self) -> PathBuf {
        if !self.stated_home.as_os_str().is_empty() {
            return self.stated_home.clone();
        }
        self.env
            .var_clean("REASONIX_HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
    }

    /// The state root only insofar as a variable holds it there: its own, or
    /// the home root it otherwise follows.
    pub fn pinned_state_dir(&self) -> PathBuf {
        self.env
            .var_clean("REASONIX_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.pinned_home_dir())
    }

    /// What the configuration chose for a root, empty when it chose nothing.
    /// An immovable root always answers empty.
    pub fn configured_root_dir(&self, id: RootId) -> PathBuf {
        if !root_relocatable(id) {
            return PathBuf::new();
        }
        self.configured
            .iter()
            .find(|(configured, _)| *configured == id)
            .map(|(_, dir)| dir.clone())
            .unwrap_or_default()
    }

    /// The variable holding this root in place, or "" when none does. A
    /// surface offering to move a root shows this instead of accepting an edit
    /// the environment would silently override.
    pub fn pinned_by(&self, id: RootId) -> &'static str {
        let Some(root) = lookup_root(id) else {
            return "";
        };
        if root.env.is_empty() || clean_env_dir_in(root.env, &self.env).as_os_str().is_empty() {
            return "";
        }
        root.env
    }

    fn default_dir(&self, root: &StorageRoot) -> PathBuf {
        match root.id {
            RootId::Home => default_home_dir_in(&self.env),
            RootId::State => self.dir(RootId::Home),
            RootId::Cache => self.default_cache_dir(),
            RootId::Worktrees => self.default_worktree_dir(),
            RootId::Locks => os_cache_base_in(&self.env),
        }
    }

    /// A pinned home takes its cache along as a subdirectory; otherwise the
    /// cache is the OS location, which on Windows does not roam.
    fn default_cache_dir(&self) -> PathBuf {
        let home = self.pinned_home_dir();
        if !home.as_os_str().is_empty() {
            return home.join("cache");
        }
        os_cache_base_in(&self.env)
    }

    /// A deliberately pinned state root takes the worktrees along; otherwise
    /// Windows keeps them out of the roaming profile.
    fn default_worktree_dir(&self) -> PathBuf {
        let pinned = self.pinned_state_dir();
        if !pinned.as_os_str().is_empty() {
            return pinned.join("worktrees");
        }
        if cfg!(windows) {
            return join_root(os_cache_base_in(&self.env), "worktrees");
        }
        join_root(self.dir(RootId::State), "worktrees")
    }

    /// The user config directory: the home root.
    pub fn user_config_dir(&self) -> PathBuf {
        self.dir(RootId::Home)
    }

    /// `<home>/config.toml`, empty when the home does not resolve.
    pub fn user_config_path(&self) -> PathBuf {
        let home = self.user_config_dir();
        if home.as_os_str().is_empty() {
            return PathBuf::new();
        }
        home.join("config.toml")
    }

    /// This binding's credentials `.env`.
    pub fn user_credentials_path(&self) -> PathBuf {
        let home = self.dir(RootId::Home);
        if home.as_os_str().is_empty() {
            return PathBuf::new();
        }
        home.join(".env")
    }

    fn legacy_os_support_dir(&self) -> PathBuf {
        if !self.pinned_home_dir().as_os_str().is_empty() {
            return PathBuf::new();
        }
        let Some(dir) = user_config_dir_in(&self.env) else {
            return PathBuf::new();
        };
        let path = dir.join("reasonix");
        let current = self.dir(RootId::Home);
        if !current.as_os_str().is_empty() && same_path(&path, &current) {
            return PathBuf::new();
        }
        path
    }

    fn legacy_xdg_config_paths(&self) -> Vec<PathBuf> {
        if !self.pinned_home_dir().as_os_str().is_empty() || cfg!(windows) {
            return Vec::new();
        }
        let mut paths: Vec<PathBuf> = Vec::new();
        let mut add = |path: PathBuf| {
            if path.as_os_str().is_empty() || paths.iter().any(|seen| same_path(seen, &path)) {
                return;
            }
            paths.push(path);
        };
        if let Some(dir) = self.env.var_clean("XDG_CONFIG_HOME") {
            add(PathBuf::from(dir).join("reasonix").join("config.toml"));
        }
        if let Some(home) = user_home_dir_in(&self.env) {
            add(home.join(".config").join("reasonix").join("config.toml"));
        }
        paths
    }

    fn legacy_user_config_path(&self) -> PathBuf {
        let dir = self.legacy_os_support_dir();
        if dir.as_os_str().is_empty() {
            return PathBuf::new();
        }
        let path = dir.join("config.toml");
        let primary = self.user_config_path();
        if !primary.as_os_str().is_empty() && same_path(&path, &primary) {
            return PathBuf::new();
        }
        path
    }

    /// Every candidate a load might read, deduplicated: primary, the legacy
    /// OS-support copy, then the legacy XDG paths.
    pub fn user_config_candidate_paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = Vec::new();
        let primary = self.user_config_path();
        if !primary.as_os_str().is_empty() {
            paths.push(primary.clone());
        }
        let legacy = self.legacy_user_config_path();
        if !legacy.as_os_str().is_empty() {
            paths.push(legacy);
        }
        for path in self.legacy_xdg_config_paths() {
            if !paths.iter().any(|seen| same_path(seen, &path)) {
                paths.push(path);
            }
        }
        paths
    }

    /// The user file a load actually reads (paths.go userConfigLoadPath): the
    /// primary when it exists, else the legacy OS-support copy, else the first
    /// legacy XDG candidate that exists. A reader of one table must use this so
    /// it and the full loader agree which file is authoritative.
    pub fn user_config_load_path(&self) -> PathBuf {
        let primary = self.user_config_path();
        if primary.as_os_str().is_empty() {
            return self.legacy_user_config_path();
        }
        if primary.exists() {
            return primary;
        }
        let legacy = self.legacy_user_config_path();
        if !legacy.as_os_str().is_empty() && legacy.exists() {
            return legacy;
        }
        for candidate in self.legacy_xdg_config_paths() {
            if same_path(&candidate, &primary) {
                continue;
            }
            if candidate.exists() {
                return candidate;
            }
        }
        primary
    }

    /// The Reasonix-owned user configuration FILES a repair may touch: the
    /// current config.toml, the compatibility TOMLs, and the legacy v0.x
    /// `~/.reasonix/config.json`. Files only — credentials, hooks, skills and
    /// stores never ride a config repair.
    pub fn managed_config_paths(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        let mut add = |path: PathBuf| {
            if path.as_os_str().is_empty() {
                return;
            }
            let path = crate::paths::clean_path(&path);
            if out.iter().any(|seen| same_path(seen, &path)) {
                return;
            }
            out.push(path);
        };
        add(self.user_config_path());
        add(self.legacy_user_config_path());
        for path in self.legacy_xdg_config_paths() {
            add(path);
        }
        let home = self.dir(RootId::Home);
        if !home.as_os_str().is_empty() {
            add(home.join("config.json"));
        }
        out
    }

    /// Whether `path` names one of the user config candidates (edit.go
    /// isUserConfigPath).
    pub fn is_user_config_path(&self, path: &Path) -> bool {
        if path.as_os_str().is_empty() {
            return false;
        }
        self.user_config_candidate_paths()
            .iter()
            .any(|candidate| same_path(candidate, path))
    }
}

impl crate::model::Config {
    /// The configured location for a root, "" when the configuration names
    /// none and the root falls back to its default.
    pub fn storage_dir(&self, id: RootId) -> Option<&str> {
        self.storage
            .get(id.as_str())
            .map(|dir| dir.trim())
            .filter(|dir| !dir.is_empty())
    }

    /// Relocates one root, or with "" returns it to its default
    /// (storage_config.go SetStorageDir).
    ///
    /// It rules on what the configuration can rule on — that the root exists,
    /// may be moved, and is not held by the environment — and leaves whether
    /// the target fits, is reachable, and is not inside the source to the move
    /// that acts on it. The id is a string because that is what a storage
    /// panel or a CLI verb passes in; an unknown one is a refusal a caller must
    /// be able to tell apart from a successful no-op.
    pub fn set_storage_dir(&mut self, id: &str, dir: &str) -> Result<(), StorageError> {
        let Some(root_id) = RootId::parse(id) else {
            return Err(StorageError::UnknownRoot {
                root: id.trim().to_owned(),
            });
        };
        let Some(root) = lookup_root(root_id) else {
            return Err(StorageError::UnknownRoot {
                root: root_id.as_str().to_owned(),
            });
        };
        if !root.relocatable {
            return Err(StorageError::Immovable {
                root: root.id.as_str().to_owned(),
            });
        }
        let binding = RootEnv::process();
        let pin = binding.pinned_by(root_id);
        if !pin.is_empty() {
            return Err(StorageError::HeldByEnvironment {
                root: root.id.as_str().to_owned(),
                pin: pin.to_owned(),
            });
        }
        let expanded = crate::paths::expand_dir_value_in(dir, &binding.env);
        if expanded.as_os_str().is_empty() {
            self.storage.remove(root.id.as_str());
            return Ok(());
        }
        self.storage.insert(
            root.id.as_str().to_owned(),
            expanded.to_string_lossy().into_owned(),
        );
        Ok(())
    }
}

/// Why a relocation was refused (storage_config.go SetStorageDir).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StorageError {
    /// The id names no declared root.
    #[error("set storage: no root named {root:?}")]
    UnknownRoot { root: String },
    /// The root is not relocatable.
    #[error("set storage: {root} cannot be moved")]
    Immovable { root: String },
    /// The environment holds the root in place.
    #[error("set storage: {root} is held by {pin}; unset it to choose a location here")]
    HeldByEnvironment { root: String, pin: String },
}

/// Expands and absolutises a directory value against one environment.
fn clean_env_dir_in_values(raw: &str, env: &DirEnv) -> PathBuf {
    crate::paths::expand_dir_value_in(raw, env)
}

/// Appends `sub` to the first base that resolves, and answers empty when none
/// does: `Path::join("", "x")` is the relative path "x", which would land
/// runtime state in the working directory.
pub fn join_root(base: PathBuf, sub: &str) -> PathBuf {
    if base.as_os_str().is_empty() {
        return PathBuf::new();
    }
    base.join(sub)
}

/// `filepath.Abs` + `filepath.Clean` comparison (paths.go samePath).
fn same_path(a: &Path, b: &Path) -> bool {
    if a.as_os_str().is_empty() || b.as_os_str().is_empty() {
        return false;
    }
    let abs = |path: &Path| match std::path::absolute(path) {
        Ok(abs) => crate::paths::clean_path(&abs),
        Err(_) => crate::paths::clean_path(path),
    };
    abs(a) == abs(b)
}

#[cfg(test)]
mod tests {
    use super::{
        RootEnv, RootId, STATE_ROOT_ENTRIES, StorageError, join_root, lookup_root, root_ids,
        root_owns, root_relocatable, state_root_entries_early_moves_left,
    };
    use crate::paths::DirEnv;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn env_with(vars: &[(&str, &str)]) -> DirEnv {
        DirEnv::with_vars(
            vars.iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            PathBuf::from(if cfg!(windows) { "C:\\wd" } else { "/wd" }),
        )
    }

    fn binding(vars: &[(&str, &str)]) -> RootEnv {
        RootEnv {
            env: env_with(vars),
            stated_home: PathBuf::new(),
            configured: Vec::new(),
        }
    }

    #[test]
    fn the_root_table_is_the_closed_five() {
        // storage_roots.go: five roots, three relocatable, two pinned by a
        // variable.
        assert_eq!(
            root_ids(),
            vec![
                RootId::Home,
                RootId::State,
                RootId::Cache,
                RootId::Worktrees,
                RootId::Locks
            ]
        );
        assert_eq!(RootId::parse(" home "), Some(RootId::Home));
        assert_eq!(RootId::parse("state"), Some(RootId::State));
        assert_eq!(RootId::parse("nowhere"), None);
        assert!(!root_relocatable(RootId::Home));
        assert!(!root_relocatable(RootId::Locks));
        for id in [RootId::State, RootId::Cache, RootId::Worktrees] {
            assert!(root_relocatable(id), "{id:?} should be relocatable");
        }
        assert_eq!(lookup_root(RootId::Home).unwrap().env, "REASONIX_HOME");
        assert_eq!(
            lookup_root(RootId::State).unwrap().env,
            "REASONIX_STATE_HOME"
        );
        assert_eq!(
            lookup_root(RootId::Cache).unwrap().env,
            "REASONIX_CACHE_HOME"
        );
        assert_eq!(lookup_root(RootId::Worktrees).unwrap().env, "");
        assert_eq!(lookup_root(RootId::Locks).unwrap().env, "");
    }

    #[test]
    fn the_chain_prefers_the_environment_then_the_configuration_then_the_default() {
        // storage_roots_test.go TestRootChainPrefersEnvironmentThenConfigurationThenDefault.
        let mut b = binding(&[("REASONIX_HOME", "/home/u/.reasonix")]);
        assert_eq!(b.dir(RootId::State), PathBuf::from("/home/u/.reasonix"));
        let mut storage = BTreeMap::new();
        storage.insert("state".to_owned(), "/moved/state".to_owned());
        b = binding(&[("REASONIX_HOME", "/home/u/.reasonix")]).with_storage(&storage);
        assert_eq!(b.dir(RootId::State), PathBuf::from("/moved/state"));
        b = binding(&[
            ("REASONIX_HOME", "/home/u/.reasonix"),
            ("REASONIX_STATE_HOME", "/pinned/state"),
        ])
        .with_storage(&storage);
        assert_eq!(b.dir(RootId::State), PathBuf::from("/pinned/state"));
    }

    #[test]
    fn configuration_cannot_move_an_immovable_root() {
        // storage_roots_test.go TestConfigurationCannotMoveAnImmovableRoot.
        let mut storage = BTreeMap::new();
        storage.insert("home".to_owned(), "/elsewhere".to_owned());
        storage.insert("locks".to_owned(), "/elsewhere".to_owned());
        let b = binding(&[("REASONIX_HOME", "/home/u/.reasonix")]).with_storage(&storage);
        assert_eq!(b.dir(RootId::Home), PathBuf::from("/home/u/.reasonix"));
        assert_ne!(b.dir(RootId::Locks), PathBuf::from("/elsewhere"));
        assert_eq!(b.configured_root_dir(RootId::Home), PathBuf::new());
        assert_eq!(b.configured_root_dir(RootId::Locks), PathBuf::new());
    }

    #[test]
    fn a_pinned_root_names_the_variable_holding_it() {
        // storage_roots_test.go TestPinnedRootNamesTheVariableHoldingIt.
        let mut b = binding(&[("REASONIX_HOME", "/home/u/.reasonix")]);
        assert_eq!(b.pinned_by(RootId::State), "");
        b = binding(&[
            ("REASONIX_HOME", "/home/u/.reasonix"),
            ("REASONIX_STATE_HOME", "/pinned"),
        ]);
        assert_eq!(b.pinned_by(RootId::State), "REASONIX_STATE_HOME");
        assert_eq!(b.pinned_by(RootId::Worktrees), "");
        assert_eq!(b.pinned_by(RootId::Home), "REASONIX_HOME");
    }

    #[test]
    fn a_configured_root_expands_like_an_environment_override() {
        // storage_roots_test.go TestConfiguredRootExpandsLikeAnEnvironmentOverride:
        // both layers accept the same spellings.
        let mut storage = BTreeMap::new();
        storage.insert(
            "cache".to_owned(),
            "${STORAGE_TEST_TARGET}/derived".to_owned(),
        );
        let b = binding(&[
            ("REASONIX_HOME", "/home/u/.reasonix"),
            ("STORAGE_TEST_TARGET", "/target"),
        ])
        .with_storage(&storage);
        assert_eq!(b.dir(RootId::Cache), PathBuf::from("/target/derived"));
        let mut storage = BTreeMap::new();
        storage.insert("cache".to_owned(), "~/derived".to_owned());
        let b = binding(&[
            ("REASONIX_HOME", "/home/u/.reasonix"),
            ("HOME", "/users/u"),
            ("USERPROFILE", "C:\\Users\\u"),
        ])
        .with_storage(&storage);
        let home = if cfg!(windows) {
            PathBuf::from("C:\\Users\\u")
        } else {
            PathBuf::from("/users/u")
        };
        assert_eq!(b.dir(RootId::Cache), home.join("derived"));
    }

    #[test]
    fn the_defaults_are_where_they_always_were() {
        // storage_roots_test.go TestDefaultRootDirectoriesAreWhereTheyAlwaysWere.
        let vars: &[(&str, &str)] = if cfg!(windows) {
            &[
                ("USERPROFILE", "C:\\Users\\u"),
                ("APPDATA", "C:\\Users\\u\\AppData\\Roaming"),
                ("LOCALAPPDATA", "C:\\Users\\u\\AppData\\Local"),
            ]
        } else {
            &[("HOME", "/users/u"), ("XDG_CACHE_HOME", "/users/u/.cache")]
        };
        let b = binding(vars);
        let cache_base = if cfg!(windows) {
            PathBuf::from("C:\\Users\\u\\AppData\\Local\\reasonix")
        } else {
            PathBuf::from("/users/u/.cache/reasonix")
        };
        assert_eq!(b.dir(RootId::Cache), cache_base);
        assert_eq!(b.dir(RootId::Locks), cache_base);
        assert_eq!(b.dir(RootId::State), b.dir(RootId::Home));
        // A pinned home is the one case that takes the cache along.
        let pinned = binding(&[("REASONIX_HOME", "/pinned/home")]);
        assert_eq!(
            pinned.dir(RootId::Cache),
            PathBuf::from("/pinned/home/cache")
        );
        // A pinned state root takes the worktrees along.
        let state_pinned = binding(&[("REASONIX_STATE_HOME", "/pinned/state")]);
        assert_eq!(
            state_pinned.dir(RootId::Worktrees),
            PathBuf::from("/pinned/state/worktrees")
        );
    }

    #[test]
    fn the_locks_root_owns_only_its_lock_directories() {
        // storage_roots_test.go TestLocksRootOwnsOnlyItsLockDirectories: the
        // locks root shares the OS cache with the cache root.
        let owned = root_owns(RootId::Locks);
        assert_eq!(
            owned,
            vec![
                "workspace-leases",
                "repair-mutation-locks",
                "memory-store-locks"
            ]
        );
        assert!(root_owns(RootId::Home).is_empty());
        assert_eq!(root_owns(RootId::State).len(), STATE_ROOT_ENTRIES.len());
    }

    #[test]
    fn the_state_root_entry_list_is_the_frozen_one() {
        // storage_roots.go StateRootEntries, in declaration order.
        assert_eq!(STATE_ROOT_ENTRIES.len(), 25);
        for name in [
            "sessions",
            "archive",
            "stats",
            "projects",
            "serve-workspaces.json",
            "machine-id.key",
            "CLAUDE.local.md",
        ] {
            assert!(STATE_ROOT_ENTRIES.contains(&name), "missing {name:?}");
        }
        // The early-move leftovers are everything the first relocation did
        // not own, and the first-move set itself is gone from the list.
        let left = state_root_entries_early_moves_left();
        assert_eq!(left.len(), STATE_ROOT_ENTRIES.len() - 4);
        for name in ["sessions", "archive", "stats", "projects"] {
            assert!(!left.iter().any(|entry| entry == name));
            assert!(STATE_ROOT_ENTRIES.contains(&name));
        }
    }

    #[test]
    fn join_root_never_yields_a_relative_path() {
        // storage_roots.go joinRoot guard.
        assert_eq!(join_root(PathBuf::new(), "x"), PathBuf::new());
        assert_eq!(
            join_root(PathBuf::from("/base"), "x"),
            PathBuf::from("/base/x")
        );
    }

    #[test]
    fn the_user_config_selection_chain_prefers_the_primary() {
        // paths.go userConfigLoadPath: primary, legacy OS-support, legacy XDG.
        let b = binding(&[
            ("REASONIX_HOME", "/pinned/home"),
            ("USERPROFILE", "C:\\Users\\u"),
            ("HOME", "/users/u"),
            ("APPDATA", "C:\\Users\\u\\AppData\\Roaming"),
        ]);
        // Nothing exists yet: the primary path is the answer even so.
        assert_eq!(
            b.user_config_load_path(),
            PathBuf::from("/pinned/home/config.toml")
        );
        // A pinned home makes the legacy chain invisible.
        assert!(
            b.user_config_candidate_paths()
                .iter()
                .all(|path| { path.starts_with("/pinned/home") })
        );
        assert!(b.is_user_config_path(std::path::Path::new("/pinned/home/config.toml")));
        assert!(!b.is_user_config_path(std::path::Path::new("/pinned/home/other.toml")));
        assert!(!b.is_user_config_path(std::path::Path::new("")));
    }

    #[test]
    fn the_storage_setter_refuses_what_it_cannot_honour() {
        // storage_roots_test.go TestSetStorageDirRefusesWhatItCannotHonour:
        // an immovable root, an undeclared id, and an environment pin each
        // answer their own message.
        let mut config = crate::model::Config::default();
        assert_eq!(
            config.set_storage_dir("locks", "/anywhere").unwrap_err(),
            StorageError::Immovable {
                root: "locks".to_owned()
            }
        );
        assert_eq!(
            config.set_storage_dir("home", "/anywhere").unwrap_err(),
            StorageError::Immovable {
                root: "home".to_owned()
            }
        );
        assert_eq!(
            config.set_storage_dir("nowhere", "/anywhere").unwrap_err(),
            StorageError::UnknownRoot {
                root: "nowhere".to_owned()
            }
        );
        assert_eq!(
            StorageError::UnknownRoot {
                root: "nowhere".to_owned()
            }
            .to_string(),
            "set storage: no root named \"nowhere\""
        );
        assert_eq!(
            StorageError::Immovable {
                root: "locks".to_owned()
            }
            .to_string(),
            "set storage: locks cannot be moved"
        );
        // An environment pin outranks the configuration, so the setter refuses
        // rather than writing a line resolution would ignore. The pin names
        // whichever variable the ambient environment actually carries.
        for (id, pin) in [
            ("state", "REASONIX_STATE_HOME"),
            ("cache", "REASONIX_CACHE_HOME"),
        ] {
            if std::env::var(pin)
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false)
            {
                assert_eq!(
                    config.set_storage_dir(id, "/anywhere").unwrap_err(),
                    StorageError::HeldByEnvironment {
                        root: id.to_owned(),
                        pin: pin.to_owned(),
                    }
                );
            }
        }
    }

    #[test]
    fn a_relocation_and_its_clear_are_round_trips() {
        // storage_roots_test.go TestClearingAStorageDirRestoresTheDefault.
        if ["REASONIX_STATE_HOME", "REASONIX_HOME"].iter().any(|name| {
            std::env::var(name)
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false)
        }) {
            // Under a pinned home the setters refuse by design; that path has
            // its own test above.
            return;
        }
        let mut config = crate::model::Config::default();
        let target = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-tmp/relocated-state");
        config
            .set_storage_dir("state", &target.to_string_lossy())
            .expect("relocate state");
        assert_eq!(
            config.storage_dir(RootId::State),
            Some(
                crate::paths::clean_path(&target)
                    .to_string_lossy()
                    .into_owned()
                    .as_str()
            )
        );
        config.set_storage_dir("state", "").expect("clear");
        assert_eq!(config.storage_dir(RootId::State), None);
        assert!(config.storage.is_empty());
    }

    #[test]
    fn managed_paths_are_files_only() {
        // paths.go ReasonixManagedConfigPaths.
        let b = binding(&[("REASONIX_HOME", "/pinned/home")]);
        let managed = b.managed_config_paths();
        assert_eq!(
            managed.first(),
            Some(&PathBuf::from("/pinned/home/config.toml"))
        );
        assert!(managed.iter().any(|path| path.ends_with("config.json")));
    }
}
