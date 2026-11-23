//! Environment and `${VAR}` expansion, plus the path helpers built on the root
//! chain (paths.go / expand.go, docs/contracts/config.md §11).
//!
//! Everything here resolves against a [`DirEnv`] rather than the live process
//! environment: Go reads `os.Getenv` live, and the tests that pin an isolated
//! home need a snapshot (mutating process env is unsafe under edition 2024).

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Component, MAIN_SEPARATOR_STR, Path, PathBuf};

use crate::model::Config;
use crate::roots::{RootEnv, RootId, join_root};

/// The variables the root chain reads (storage_roots.go cleanEnvDir plus the
/// Go `os.User*Dir` probes).
const ROOT_ENV_KEYS: [&str; 9] = [
    "REASONIX_HOME",
    "REASONIX_STATE_HOME",
    "REASONIX_CACHE_HOME",
    "USERPROFILE",
    "HOME",
    "APPDATA",
    "LOCALAPPDATA",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
];

/// An environment snapshot the root chain resolves against.
#[derive(Debug, Clone, Default)]
pub struct DirEnv {
    vars: BTreeMap<String, String>,
    cwd: PathBuf,
}

impl DirEnv {
    /// A snapshot of the live process environment.
    pub fn process() -> Self {
        let mut vars = BTreeMap::new();
        for key in ROOT_ENV_KEYS {
            if let Ok(value) = std::env::var(key) {
                vars.insert((*key).to_owned(), value);
            }
        }
        Self {
            vars,
            cwd: std::env::current_dir().unwrap_or_default(),
        }
    }

    /// A snapshot built from named variables, for tests and stated bindings.
    pub fn with_vars(vars: BTreeMap<String, String>, cwd: PathBuf) -> Self {
        Self { vars, cwd }
    }

    /// One variable's raw value, empty when unset.
    pub fn var(&self, name: &str) -> &str {
        self.vars.get(name).map(String::as_str).unwrap_or_default()
    }

    /// One variable read as a directory value: expanded, absolutised, cleaned;
    /// `None` when it is unset, empty, or expands to nothing.
    pub fn var_clean(&self, name: &str) -> Option<String> {
        let path = clean_env_dir_in(name, self);
        if path.as_os_str().is_empty() {
            return None;
        }
        Some(path.to_string_lossy().into_owned())
    }

    /// The user home: `USERPROFILE` on Windows, `HOME` elsewhere; an empty
    /// value counts as unset (Go os.UserHomeDir).
    pub fn home(&self) -> Option<PathBuf> {
        user_home_dir_in(self)
    }

    /// Go `os.UserConfigDir`.
    pub fn user_config_dir(&self) -> Option<PathBuf> {
        user_config_dir_in(self)
    }

    /// Go `os.UserCacheDir`.
    pub fn user_cache_dir(&self) -> Option<PathBuf> {
        os_user_cache_dir_in(self)
    }

    /// The working directory this snapshot resolves relative paths against.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }
}

/// Go `os.UserHomeDir`: `%USERPROFILE%` on Windows, `$HOME` elsewhere; empty
/// counts as unset.
pub fn user_home_dir_in(env: &DirEnv) -> Option<PathBuf> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let value = env.var(key);
    if value.is_empty() {
        None
    } else {
        Some(PathBuf::from(value))
    }
}

/// Go `os.UserConfigDir`.
pub fn user_config_dir_in(env: &DirEnv) -> Option<PathBuf> {
    if cfg!(windows) {
        let value = env.var("APPDATA");
        if value.is_empty() {
            None
        } else {
            Some(PathBuf::from(value))
        }
    } else if cfg!(target_os = "macos") {
        Some(env.home()?.join("Library").join("Application Support"))
    } else {
        let xdg = env.var("XDG_CONFIG_HOME");
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg));
        }
        Some(env.home()?.join(".config"))
    }
}

/// Go `os.UserCacheDir`: `%LOCALAPPDATA%` on Windows, `~/Library/Caches` on
/// macOS, `$XDG_CACHE_HOME` or `~/.cache` elsewhere.
pub fn os_user_cache_dir_in(env: &DirEnv) -> Option<PathBuf> {
    if cfg!(windows) {
        let value = env.var("LOCALAPPDATA");
        if value.is_empty() {
            None
        } else {
            Some(PathBuf::from(value))
        }
    } else if cfg!(target_os = "macos") {
        Some(env.home()?.join("Library").join("Caches"))
    } else {
        let xdg = env.var("XDG_CACHE_HOME");
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg));
        }
        Some(env.home()?.join(".cache"))
    }
}

/// The OS cache location for this app (storage_roots.go osCacheBase). It reads
/// no Reasonix variable on purpose: the locks root has to converge there even
/// when two instances run under different profiles.
pub fn os_cache_base_in(env: &DirEnv) -> PathBuf {
    if let Some(dir) = os_user_cache_dir_in(env) {
        return dir.join("reasonix");
    }
    if cfg!(windows)
        && let Some(home) = env.home()
    {
        return home.join("AppData").join("Local").join("reasonix");
    }
    PathBuf::new()
}

/// Where configuration lands when REASONIX_HOME is unset: the roaming profile
/// on Windows, a dotfile directory elsewhere (storage_roots.go defaultHomeDir).
pub fn default_home_dir_in(env: &DirEnv) -> PathBuf {
    if cfg!(windows) {
        if let Some(dir) = user_config_dir_in(env) {
            return dir.join("reasonix");
        }
        return match env.home() {
            Some(home) => home.join("AppData").join("Roaming").join("reasonix"),
            None => PathBuf::new(),
        };
    }
    if let Some(home) = env.home() {
        return home.join(".reasonix");
    }
    match user_config_dir_in(env) {
        Some(dir) => dir.join("reasonix"),
        None => PathBuf::new(),
    }
}

/// Go `config.cleanEnvDir`: a variable read as a directory value.
pub fn clean_env_dir_in(name: &str, env: &DirEnv) -> PathBuf {
    expand_dir_value_in(env.var(name), env)
}

/// Turns a directory someone wrote — in the environment or in config.toml —
/// into an absolute path (paths.go expandDirValue): trim, `${VAR}` expansion,
/// `~` into the user home, relative into absolute, then lexically cleaned.
/// Both layers of the root chain share it, so a location is spelled the same
/// way wherever it is set.
pub fn expand_dir_value_in(dir: &str, env: &DirEnv) -> PathBuf {
    let dir = dir.trim();
    if dir.is_empty() {
        return PathBuf::new();
    }
    let expanded = expand_vars_lookup(dir, &|name| env.var(name).to_owned());
    let path = match home_prefixed(&expanded, env) {
        Some(path) => path,
        None => PathBuf::from(&expanded),
    };
    if is_absolute_path(&path) {
        return clean_path(&path);
    }
    clean_path(&env.cwd().join(&path))
}

fn home_prefixed(expanded: &str, env: &DirEnv) -> Option<PathBuf> {
    let home = env.home()?;
    if expanded == "~" {
        return Some(home);
    }
    expanded
        .strip_prefix("~/")
        .or_else(|| expanded.strip_prefix("~\\"))
        .map(|rest| home.join(rest))
}

/// `filepath.IsAbs` on Windows also accepts rooted paths ("\\foo"); plain Rust
/// `is_absolute` demands a volume prefix too.
fn is_absolute_path(path: &Path) -> bool {
    path.is_absolute() || path.has_root()
}

/// `filepath.Clean` over components. Only called on absolutized paths, so a
/// volume-relative spelling never reaches the separator reconstruction.
pub fn clean_path(path: &Path) -> PathBuf {
    let mut parts: Vec<Component> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match parts.last() {
                Some(Component::Normal(_)) => {
                    parts.pop();
                }
                // Leading and chained ".." survive; one directly after a root
                // is dropped (`filepath.Clean` turns "/.." into "/").
                Some(Component::ParentDir) | None => parts.push(component),
                _ => {}
            },
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return PathBuf::from(".");
    }
    let mut out = OsString::new();
    for component in parts {
        match component {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(MAIN_SEPARATOR_STR),
            Component::Normal(name) => {
                if !out.is_empty() && !ends_with_separator(&out) {
                    out.push(MAIN_SEPARATOR_STR);
                }
                out.push(name);
            }
            Component::ParentDir => {
                if !out.is_empty() && !ends_with_separator(&out) {
                    out.push(MAIN_SEPARATOR_STR);
                }
                out.push("..");
            }
            Component::CurDir => {}
        }
    }
    PathBuf::from(out)
}

fn ends_with_separator(value: &OsStr) -> bool {
    // WTF-8 continuation bytes are all >= 0x80, so a trailing ASCII byte is
    // always a whole character.
    matches!(value.as_encoded_bytes().last(), Some(b'/') | Some(b'\\'))
}

/// Substitutes `${VAR}` / `${VAR:-default}` from the process environment
/// (expand.go ExpandVars). An unset or empty variable with no default expands
/// to "", so a missing value never leaks a literal `${TOKEN}`.
pub fn expand_vars(input: &str) -> String {
    expand_vars_lookup(input, &|name| std::env::var(name).unwrap_or_default())
}

fn expand_vars_lookup(input: &str, lookup: &dyn Fn(&str) -> String) -> String {
    if !input.contains("${") {
        return input.to_owned();
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        match var_ref(rest) {
            Some((name, default, consumed)) => {
                let value = lookup(name);
                if value.is_empty() {
                    out.push_str(default);
                } else {
                    out.push_str(&value);
                }
                rest = &rest[consumed..];
            }
            None => {
                out.push_str("${");
                rest = &rest[2..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Splits a reference at the start of `rest`: `${NAME}` or `${NAME:-default}`
/// with NAME matching `[A-Za-z_][A-Za-z0-9_]*` (expand.go varRef); anything
/// else is not a reference. Returns the name, the default, and the consumed
/// byte count.
fn var_ref(rest: &str) -> Option<(&str, &str, usize)> {
    let bytes = rest.as_bytes();
    // "${" + one name byte + "}" is the shortest well-formed reference.
    if bytes.len() < 4 || !is_name_start(bytes[2]) {
        return None;
    }
    let mut i = 3;
    while i < bytes.len() && is_name_byte(bytes[i]) {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let name = &rest[2..i];
    if bytes[i] == b'}' {
        return Some((name, "", i + 1));
    }
    if bytes[i] == b':' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
        let start = i + 2;
        let end = rest[start..].find('}')? + start;
        return Some((name, &rest[start..end], end + 1));
    }
    None
}

fn is_name_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn is_name_byte(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

impl Config {
    /// The root binding this configuration resolves against: the process
    /// environment plus its own `[storage]` relocations.
    pub fn root_env(&self) -> RootEnv {
        RootEnv::process().with_storage(&self.storage)
    }

    /// One root's absolute directory, empty when it cannot be resolved.
    pub fn root_dir(&self, id: RootId) -> PathBuf {
        self.root_env().dir(id)
    }

    /// The folder a first launch opens when nobody has chosen a project: a
    /// small one the app owns under the state root, never the home directory
    /// (paths.go AutoWorkspaceDir). The caller creates it.
    pub fn auto_workspace_dir(&self) -> PathBuf {
        let base = self.root_dir(RootId::State);
        if base.as_os_str().is_empty() {
            return PathBuf::new();
        }
        base.join("workspaces").join("auto")
    }
}

/// Go `config.AutoWorkspaceDir()`: the auto workspace under the state root the
/// config resolves against.
pub fn auto_workspace_dir(config: &Config) -> PathBuf {
    config.auto_workspace_dir()
}

/// Go `config.ReasonixHomeDir()`: the data home the root chain resolves (the
/// REASONIX_HOME pin, else the platform default). Empty when none can.
pub fn reasonix_home_dir() -> PathBuf {
    RootEnv::process().dir(RootId::Home)
}

/// The state root: REASONIX_STATE_HOME, else the home root.
pub fn reasonix_state_dir() -> PathBuf {
    RootEnv::process().dir(RootId::State)
}

/// The cache root: REASONIX_CACHE_HOME, else `<pinned home>/cache`, else the OS
/// cache location.
pub fn cache_dir() -> PathBuf {
    RootEnv::process().dir(RootId::Cache)
}

/// The locks root. It converges on the OS cache even under an isolated home,
/// so two runtimes find the same lock.
pub fn locks_dir() -> PathBuf {
    RootEnv::process().dir(RootId::Locks)
}

/// The delivery worktree root.
pub fn delivery_worktree_dir() -> PathBuf {
    RootEnv::process().dir(RootId::Worktrees)
}

/// Cross-process Delivery writer locks, outside user workspaces.
pub fn workspace_lease_dir() -> PathBuf {
    join_root(locks_dir(), "workspace-leases")
}

/// Target-path repair locks in the OS-user cache.
pub fn repair_mutation_lock_dir() -> PathBuf {
    join_root(locks_dir(), "repair-mutation-locks")
}

/// One mutation lock per memory store directory.
pub fn memory_lock_dir() -> PathBuf {
    join_root(locks_dir(), "memory-store-locks")
}

/// The user config file a load reads (paths.go userConfigLoadPath): the primary
/// `<home>/config.toml` when it exists, else the legacy OS-support copy, else
/// the legacy XDG candidates. Empty when the home cannot resolve.
pub fn user_config_path() -> PathBuf {
    RootEnv::process().user_config_load_path()
}

#[cfg(test)]
mod tests {
    use super::{
        DirEnv, clean_path, expand_dir_value_in, expand_vars_lookup, os_cache_base_in,
        user_config_dir_in,
    };
    use crate::model::Config;
    use crate::roots::{RootEnv, RootId};
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    fn env_with(vars: &[(&str, &str)]) -> DirEnv {
        DirEnv::with_vars(
            vars.iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            PathBuf::from(if cfg!(windows) { "C:\\wd" } else { "/wd" }),
        )
    }

    // The selection chain touches the filesystem, so the tests root their
    // homes under target/ — the system Temp is denied under this
    // environment's antivirus policy.
    fn home_dir(name: &str) -> PathBuf {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/userconfig");
        let dir = base.join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn binding(env: DirEnv) -> RootEnv {
        RootEnv {
            env,
            stated_home: PathBuf::new(),
            configured: Vec::new(),
        }
    }

    #[test]
    fn the_primary_config_file_wins_when_it_exists() {
        let home = home_dir("primary");
        std::fs::write(home.join("config.toml"), "language = \"en\"\n").unwrap();
        let binding = binding(env_with(&[("REASONIX_HOME", &home.to_string_lossy())]));
        assert_eq!(
            binding.user_config_load_path(),
            clean_path(&home.join("config.toml"))
        );
    }

    #[test]
    // Windows has no distinct legacy copy: the default home IS
    // osUserConfigDir()/reasonix, so the "different from primary" exclusion
    // makes the fallback unreachable there — same as the reference.
    #[cfg(unix)]
    fn a_missing_primary_falls_back_to_the_legacy_os_support_copy() {
        let support = home_dir("legacy-support");
        let env = env_with(&[
            ("USERPROFILE", &support.to_string_lossy()),
            ("HOME", &support.to_string_lossy()),
            ("APPDATA", &support.to_string_lossy()),
        ]);
        let legacy = user_config_dir_in(&env)
            .unwrap()
            .join("reasonix")
            .join("config.toml");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "language = \"en\"\n").unwrap();
        assert_eq!(binding(env).user_config_load_path(), legacy);
    }

    #[test]
    fn a_pinned_home_skips_the_legacy_chain() {
        let home = home_dir("pinned");
        let support = home_dir("pinned-support");
        let env = env_with(&[
            ("REASONIX_HOME", &home.to_string_lossy()),
            ("USERPROFILE", &support.to_string_lossy()),
            ("HOME", &support.to_string_lossy()),
            ("APPDATA", &support.to_string_lossy()),
        ]);
        let legacy = user_config_dir_in(&env)
            .unwrap()
            .join("reasonix")
            .join("config.toml");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "language = \"en\"\n").unwrap();
        // The legacy copy exists, but a pinned home makes it invisible: the
        // primary path is the answer even though nothing is there.
        let got = binding(env).user_config_load_path();
        assert_eq!(got, clean_path(&home.join("config.toml")));
        assert!(!got.exists());
    }

    #[test]
    fn with_no_home_at_all_the_path_is_empty() {
        assert_eq!(binding(env_with(&[])).user_config_path(), PathBuf::new());
        assert_eq!(binding(env_with(&[])).dir(RootId::State), PathBuf::new());
    }

    #[test]
    fn expand_vars_substitutes_and_defaults() {
        // expand.go: unset/empty counts as unset; the default applies then.
        let env = env_with(&[("SET", "v"), ("EMPTY", "")]);
        let lookup = |n: &str| env.var(n).to_owned();
        assert_eq!(expand_vars_lookup("${SET}", &lookup), "v");
        assert_eq!(expand_vars_lookup("a${SET}b", &lookup), "avb");
        assert_eq!(expand_vars_lookup("${NOPE:-d}", &lookup), "d");
        assert_eq!(expand_vars_lookup("${EMPTY:-d}", &lookup), "d");
        assert_eq!(expand_vars_lookup("${NOPE}", &lookup), "");
        // Non-references stay literal (varRef requires a valid NAME).
        assert_eq!(expand_vars_lookup("${1A}", &lookup), "${1A}");
        assert_eq!(expand_vars_lookup("${NOPE", &lookup), "${NOPE");
        assert_eq!(expand_vars_lookup("plain", &lookup), "plain");
    }

    #[test]
    fn expand_dir_value_expands_home_and_absolutizes() {
        // paths.go expandDirValue: ~ into home, relative into absolute, cleaned.
        let env = env_with(&[("HOME", "/Users/x"), ("USERPROFILE", "C:\\Users\\x")]);
        let home = env.home().unwrap_or_default();
        assert_eq!(expand_dir_value_in("~", &env), home.clone());
        assert_eq!(expand_dir_value_in("~/sub/", &env), home.join("sub"));
        assert_eq!(
            expand_dir_value_in("a//b/./c/../d", &env),
            env.cwd().join("a").join("b").join("d")
        );
        assert_eq!(expand_dir_value_in("   ", &env), PathBuf::new());
    }

    #[test]
    fn the_auto_workspace_is_empty_without_a_state_root() {
        // No home and no pin: the reference answers "" and the caller degrades
        // rather than joining a relative "workspaces/auto" into the cwd.
        let mut config = Config::default();
        config.storage.clear();
        let empty = binding(env_with(&[]));
        assert_eq!(empty.dir(RootId::State), PathBuf::new());
        // A stated home always resolves one.
        let stated = RootEnv {
            env: env_with(&[]),
            stated_home: PathBuf::from("/stated/home"),
            configured: Vec::new(),
        };
        assert_eq!(
            stated.dir(RootId::State).join("workspaces").join("auto"),
            PathBuf::from("/stated/home/workspaces/auto")
        );
    }

    #[test]
    fn the_os_cache_base_reads_no_reasonix_variable() {
        // storage_roots.go osCacheBase: locks converge on the OS user's cache,
        // so a profile variable in the same snapshot changes nothing.
        let env = env_with(&[
            ("LOCALAPPDATA", "C:\\Users\\u\\AppData\\Local"),
            ("XDG_CACHE_HOME", "/users/u/.cache"),
            ("REASONIX_HOME", "/ignored"),
            ("REASONIX_CACHE_HOME", "/ignored"),
        ]);
        let expected = if cfg!(windows) {
            PathBuf::from("C:\\Users\\u\\AppData\\Local").join("reasonix")
        } else if cfg!(target_os = "macos") {
            // macOS reads ~/Library/Caches and the snapshot names no home.
            PathBuf::new()
        } else {
            PathBuf::from("/users/u/.cache").join("reasonix")
        };
        assert_eq!(os_cache_base_in(&env), expected);
    }

    #[test]
    fn clean_follows_filepath_clean_rules() {
        assert_eq!(clean_path(Path::new("a/b/")), PathBuf::from("a/b"));
        assert_eq!(clean_path(Path::new("/..")), PathBuf::from("/"));
        assert_eq!(clean_path(Path::new("a/../../b")), PathBuf::from("../b"));
        assert_eq!(clean_path(Path::new(".")), PathBuf::from("."));
    }

    #[test]
    fn the_env_snapshot_keeps_the_variables_the_root_chain_reads() {
        // DirEnv reads one variable through the same directory-value rules a
        // pin does, so a relative value lands absolute.
        let vars: BTreeMap<String, String> = [("UNRELATED", "x")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        let cwd = PathBuf::from(if cfg!(windows) { "C:\\wd" } else { "/wd" });
        let env = DirEnv::with_vars(vars, cwd.clone());
        assert_eq!(env.var("UNRELATED"), "x");
        assert_eq!(env.var("REASONIX_HOME"), "");
        assert_eq!(env.var_clean("REASONIX_HOME"), None);
        assert_eq!(
            env.var_clean("UNRELATED"),
            Some(cwd.join("x").to_string_lossy().into_owned())
        );
    }
}
