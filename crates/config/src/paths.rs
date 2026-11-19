//! Path resolution for this slice: the auto workspace directory and the
//! environment/`${VAR}` expansion the reference applies to directory values.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Component, MAIN_SEPARATOR_STR, Path, PathBuf};

use crate::model::Config;

/// Environment the root chain resolves against. Real callers snapshot the
/// process environment (Go reads it live); tests inject a snapshot because
/// mutating process env is unsafe under edition 2024 and tests run threaded.
#[derive(Debug, Clone, Default)]
struct RootEnv {
    vars: BTreeMap<String, String>,
    cwd: PathBuf,
}

/// Only the variables the root chain reads (storage_roots.go cleanEnvDir +
/// the Go os.User*Dir probes).
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

impl RootEnv {
    fn process() -> Self {
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

    fn var(&self, name: &str) -> &str {
        self.vars.get(name).map(String::as_str).unwrap_or_default()
    }

    fn home(&self) -> Option<PathBuf> {
        // Go os.UserHomeDir: USERPROFILE on Windows, HOME elsewhere; an empty
        // value counts as unset.
        let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let value = self.var(key);
        if value.is_empty() {
            None
        } else {
            Some(PathBuf::from(value))
        }
    }

    fn user_config_dir(&self) -> Option<PathBuf> {
        if cfg!(windows) {
            let value = self.var("APPDATA");
            if value.is_empty() {
                None
            } else {
                Some(PathBuf::from(value))
            }
        } else if cfg!(target_os = "macos") {
            Some(self.home()?.join("Library").join("Application Support"))
        } else {
            let xdg = self.var("XDG_CONFIG_HOME");
            if !xdg.is_empty() {
                return Some(PathBuf::from(xdg));
            }
            Some(self.home()?.join(".config"))
        }
    }

    fn cwd(&self) -> &Path {
        &self.cwd
    }
}

/// The folder a first launch opens when nobody has chosen a project: a small
/// one the app owns under the user state root, never the home directory
/// (paths.go AutoWorkspaceDir). The caller creates it. Empty when the state
/// root cannot be resolved.
pub fn auto_workspace_dir(config: &Config) -> PathBuf {
    auto_workspace_dir_in(&config.storage, &RootEnv::process())
}

fn auto_workspace_dir_in(storage: &BTreeMap<String, String>, env: &RootEnv) -> PathBuf {
    // The state-root chain (storage_roots.go Dir): the REASONIX_STATE_HOME
    // pin, then the [storage] relocation, then the home root.
    let pinned = expand_dir_value_in(env.var("REASONIX_STATE_HOME"), env);
    if !pinned.as_os_str().is_empty() {
        return under_auto(pinned);
    }
    let home = home_root_in(env);
    if home.as_os_str().is_empty() {
        return PathBuf::new();
    }
    if let Some(configured) = configured_state_root(storage, env) {
        return under_auto(configured);
    }
    under_auto(home)
}

fn under_auto(base: PathBuf) -> PathBuf {
    base.join("workspaces").join("auto")
}

/// Go `config.ReasonixHomeDir()`: the data home the root chain resolves
/// (the REASONIX_HOME pin, else the platform default). Empty when none can.
pub fn reasonix_home_dir() -> PathBuf {
    home_root_in(&RootEnv::process())
}

/// The user config file a load reads (paths.md §2.1 userConfigLoadPath): the
/// primary `<home>/config.toml` when it exists, else the legacy OS-support
/// copy, else the legacy XDG candidates. Empty when the home cannot resolve.
pub fn user_config_path() -> PathBuf {
    user_config_path_in(&RootEnv::process())
}

fn user_config_path_in(env: &RootEnv) -> PathBuf {
    let home = home_root_in(env);
    if home.as_os_str().is_empty() {
        return PathBuf::new();
    }
    let primary = home.join("config.toml");
    if primary.exists() {
        return primary;
    }
    // The legacy dirs name the same tree as a pinned home, so only the
    // primary counts there.
    if env.var("REASONIX_HOME").trim().is_empty() {
        if let Some(dir) = env.user_config_dir() {
            let legacy = dir.join("reasonix").join("config.toml");
            if legacy != primary && legacy.exists() {
                return legacy;
            }
        }
        if cfg!(unix) {
            let xdg = env.var("XDG_CONFIG_HOME");
            if !xdg.is_empty() {
                let candidate = PathBuf::from(xdg).join("reasonix").join("config.toml");
                if candidate.exists() {
                    return candidate;
                }
            }
            if let Some(home) = env.home() {
                let candidate = home.join(".config").join("reasonix").join("config.toml");
                if candidate.exists() {
                    return candidate;
                }
            }
        }
    }
    primary
}

fn home_root_in(env: &RootEnv) -> PathBuf {
    let pinned = expand_dir_value_in(env.var("REASONIX_HOME"), env);
    if !pinned.as_os_str().is_empty() {
        return pinned;
    }
    default_home_dir_in(env)
}

fn configured_state_root(storage: &BTreeMap<String, String>, env: &RootEnv) -> Option<PathBuf> {
    // readStorageDirs: only relocatable roots move, and a value expands the
    // same way an environment pin does.
    let dir = storage.get("state")?.trim();
    if dir.is_empty() {
        return None;
    }
    let expanded = expand_dir_value_in(dir, env);
    if expanded.as_os_str().is_empty() {
        None
    } else {
        Some(expanded)
    }
}

fn default_home_dir_in(env: &RootEnv) -> PathBuf {
    if cfg!(windows) {
        if let Some(dir) = env.user_config_dir() {
            return dir.join("reasonix");
        }
        match env.home() {
            Some(home) => home.join("AppData").join("Roaming").join("reasonix"),
            None => PathBuf::new(),
        }
    } else {
        match env.home() {
            Some(home) => home.join(".reasonix"),
            None => match env.user_config_dir() {
                Some(dir) => dir.join("reasonix"),
                None => PathBuf::new(),
            },
        }
    }
}

/// Turns a directory someone wrote — in the environment or in config.toml —
/// into an absolute path (paths.go expandDirValue): trim, `${VAR}` expansion,
/// `~` into the user home, relative into absolute, then lexically cleaned.
fn expand_dir_value_in(dir: &str, env: &RootEnv) -> PathBuf {
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

fn home_prefixed(expanded: &str, env: &RootEnv) -> Option<PathBuf> {
    let home = env.home()?;
    if expanded == "~" {
        return Some(home);
    }
    expanded
        .strip_prefix("~/")
        .or_else(|| expanded.strip_prefix("~\\"))
        .map(|rest| home.join(rest))
}

/// filepath.IsAbs on Windows also accepts rooted paths ("\\foo"); plain Rust
/// `is_absolute` demands a volume prefix too.
fn is_absolute_path(path: &Path) -> bool {
    path.is_absolute() || path.has_root()
}

/// filepath.Clean over components. Only called on absolutized paths, so a
/// volume-relative spelling never reaches the separator reconstruction.
fn clean_path(path: &Path) -> PathBuf {
    let mut parts: Vec<Component> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match parts.last() {
                Some(Component::Normal(_)) => {
                    parts.pop();
                }
                // Leading and chained ".." survive; one directly after a root
                // is dropped (filepath.Clean turns "/.." into "/").
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
/// (expand.go ExpandVars). An unset or empty variable with no default
/// expands to "", so a missing value never leaks a literal `${TOKEN}`.
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

/// Splits a reference at the start of `rest`: `${NAME}` or
/// `${NAME:-default}` with NAME matching [A-Za-z_][A-Za-z0-9_]* (expand.go
/// varRef); anything else is not a reference. Returns the name, the default,
/// and the consumed byte count.
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

#[cfg(test)]
mod tests {
    use super::{
        RootEnv, auto_workspace_dir_in, clean_path, expand_dir_value_in, expand_vars_lookup,
        user_config_path_in,
    };
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    fn env_with(vars: &[(&str, &str)]) -> RootEnv {
        RootEnv {
            vars: vars
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            cwd: PathBuf::from(if cfg!(windows) { "C:\\wd" } else { "/wd" }),
        }
    }

    // The selection chain touches the filesystem, so the tests root their
    // homes under target/ — the system Temp is not writable under this
    // environment's antivirus policy.
    fn home_dir(name: &str) -> PathBuf {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/userconfig");
        let dir = base.join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_primary_config_file_wins_when_it_exists() {
        let home = home_dir("primary");
        std::fs::write(home.join("config.toml"), "language = \"en\"\n").unwrap();
        let got = user_config_path_in(&env_with(&[("REASONIX_HOME", &home.to_string_lossy())]));
        assert_eq!(got, clean_path(&home.join("config.toml")));
    }

    #[test]
    // Windows has no distinct legacy copy: the default home IS
    // osUserConfigDir()/reasonix, so the "different from primary" exclusion
    // makes the fallback unreachable there — same as the reference.
    #[cfg(unix)]
    fn a_missing_primary_falls_back_to_the_legacy_os_support_copy() {
        let home = home_dir("legacy");
        let support = home_dir("legacy-support");
        let env = env_with(&[
            ("REASONIX_HOME", &home.to_string_lossy()),
            ("USERPROFILE", &support.to_string_lossy()),
            ("HOME", &support.to_string_lossy()),
            ("APPDATA", &support.to_string_lossy()),
        ]);
        let legacy = env
            .user_config_dir()
            .unwrap()
            .join("reasonix")
            .join("config.toml");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "language = \"en\"\n").unwrap();
        assert_eq!(user_config_path_in(&env), legacy);
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
        let legacy = env
            .user_config_dir()
            .unwrap()
            .join("reasonix")
            .join("config.toml");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "language = \"en\"\n").unwrap();
        // The legacy copy exists, but a pinned home makes it invisible: the
        // primary path is the answer even though nothing is there.
        let got = user_config_path_in(&env);
        assert_eq!(got, clean_path(&home.join("config.toml")));
        assert!(!got.exists());
    }

    #[test]
    fn with_no_home_at_all_the_path_is_empty() {
        assert_eq!(user_config_path_in(&env_with(&[])), PathBuf::new());
    }

    #[test]
    fn expand_vars_substitutes_and_defaults() {
        // expand.go: unset/empty counts as unset; the default applies then.
        let env = env_with(&[("SET", "v"), ("EMPTY", "")]);
        assert_eq!(
            expand_vars_lookup("${SET}", &|n| env.var(n).to_owned()),
            "v"
        );
        assert_eq!(
            expand_vars_lookup("a${SET}b", &|n| env.var(n).to_owned()),
            "avb"
        );
        assert_eq!(
            expand_vars_lookup("${NOPE:-d}", &|n| env.var(n).to_owned()),
            "d"
        );
        assert_eq!(
            expand_vars_lookup("${EMPTY:-d}", &|n| env.var(n).to_owned()),
            "d"
        );
        assert_eq!(
            expand_vars_lookup("${NOPE}", &|n| env.var(n).to_owned()),
            ""
        );
        // Non-references stay literal (varRef requires a valid NAME).
        assert_eq!(
            expand_vars_lookup("${1A}", &|n| env.var(n).to_owned()),
            "${1A}"
        );
        assert_eq!(
            expand_vars_lookup("${NOPE", &|n| env.var(n).to_owned()),
            "${NOPE"
        );
        assert_eq!(
            expand_vars_lookup("plain", &|n| env.var(n).to_owned()),
            "plain"
        );
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
    fn auto_workspace_prefers_the_state_pin() {
        let env = env_with(&[("REASONIX_STATE_HOME", "/state")]);
        let expected = clean_path(Path::new("/state"))
            .join("workspaces")
            .join("auto");
        assert_eq!(auto_workspace_dir_in(&BTreeMap::new(), &env), expected);
    }

    #[test]
    fn auto_workspace_falls_to_the_configured_root_then_home() {
        // storage_roots.go: env pin > [storage] state > home root.
        let env = env_with(&[("HOME", "/h"), ("USERPROFILE", "C:\\Users\\x")]);
        let mut storage = BTreeMap::new();
        storage.insert("state".to_owned(), "~/relocated".to_owned());
        let relocated = env.home().unwrap_or_default().join("relocated");
        assert_eq!(
            auto_workspace_dir_in(&storage, &env),
            relocated.join("workspaces").join("auto")
        );
        let default_state_root = if cfg!(windows) {
            PathBuf::from("C:\\Users\\x")
                .join("AppData")
                .join("Roaming")
                .join("reasonix")
        } else {
            PathBuf::from("/h").join(".reasonix")
        };
        assert_eq!(
            auto_workspace_dir_in(&BTreeMap::new(), &env),
            default_state_root.join("workspaces").join("auto")
        );
    }

    #[test]
    fn auto_workspace_is_empty_when_the_state_root_cannot_resolve() {
        // No home and no pin: the reference answers "" and the caller degrades.
        let env = env_with(&[]);
        assert_eq!(
            auto_workspace_dir_in(&BTreeMap::new(), &env),
            PathBuf::new()
        );
    }

    #[test]
    fn clean_follows_filepath_clean_rules() {
        assert_eq!(clean_path(Path::new("a/b/")), PathBuf::from("a/b"));
        assert_eq!(clean_path(Path::new("/..")), PathBuf::from("/"));
        assert_eq!(clean_path(Path::new("a/../../b")), PathBuf::from("../b"));
        assert_eq!(clean_path(Path::new(".")), PathBuf::from("."));
    }
}
