//! The subprocess environment the bash tool builds (parity targets:
//! `bash_shellenv.go`'s env fold, `shellrun/loginenv.go`'s SetEnvValue /
//! envKeyEqual, and `internal/base/secrets/redact.go`'s env half:
//! ProcessEnv, FilterEnv, the credential-key registry, and the agent-child
//! marker). The login-shell PATH probe is POSIX-only host machinery and
//! lands with the shellrun slice; the merge primitive is here so the env
//! the tool builds has one spelling.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// Marks every subprocess the host starts for the agent. Commands that need
/// a person at a terminal refuse when it is set. A child can unset it, so
/// it is friction against accidents, not a boundary.
pub const AGENT_CHILD_ENV: &str = "REASONIX_AGENT_CHILD";

static FILTER_SUBPROCESS_ENV: AtomicBool = AtomicBool::new(false);

/// Enables stripping credential-like variables from tool subprocess
/// environments ([secrets] filter_subprocess_env). Set once by the
/// composition root.
pub fn set_filter_subprocess_env(enabled: bool) {
    FILTER_SUBPROCESS_ENV.store(enabled, Ordering::SeqCst);
}

/// Reports whether credential-like variables are stripped from tool
/// subprocess environments. A caller that would launch a command in an
/// environment it cannot filter (a host-owned terminal) must check this and
/// keep execution local.
pub fn filter_subprocess_env() -> bool {
    FILTER_SUBPROCESS_ENV.load(Ordering::SeqCst)
}

fn registered_credential_env_keys() -> &'static Mutex<Vec<String>> {
    static KEYS: Mutex<Vec<String>> = Mutex::new(Vec::new());
    &KEYS
}

/// Permanently marks names whose values came from Reasonix's credential
/// store. Registration is a process-lifetime union so two concurrent
/// workspaces cannot make each other's saved keys visible to tools.
pub fn register_credential_env_keys(keys: &[String]) {
    if let Ok(mut store) = registered_credential_env_keys().lock() {
        for key in keys {
            let key = key.trim().to_ascii_uppercase();
            if !key.is_empty() && !store.contains(&key) {
                store.push(key);
            }
        }
    }
}

fn registered_credential_env_key(key: &str) -> bool {
    registered_credential_env_keys()
        .lock()
        .map(|store| store.iter().any(|k| k == key))
        .unwrap_or(false)
}

/// Whether an environment variable name is likely to carry credentials. It
/// keys off the name, not the value, so child processes do not inherit
/// saved provider secrets when filtering is enabled.
pub fn env_key_sensitive(key: &str) -> bool {
    let key = key.trim();
    !key.is_empty() && reasonix_core::secrets::credential_key(key)
}

/// Removes sensitive KEY=value assignments from an environment vector.
/// An entry that is not KEY=value is not an environment entry and drops.
pub fn filter_env(env: &[String]) -> Vec<String> {
    env.iter()
        .filter(|item| match item.split_once('=') {
            None => false,
            Some((key, _)) => !env_key_sensitive(key) && !registered_credential_env_key(key),
        })
        .cloned()
        .collect()
}

fn filter_registered_credential_env(env: &[String]) -> Vec<String> {
    env.iter()
        .filter(|item| match item.split_once('=') {
            None => false,
            Some((key, _)) => !registered_credential_env_key(key),
        })
        .cloned()
        .collect()
}

/// The environment for shell/tool subprocesses. Values loaded from
/// Reasonix's credential store are always removed. Other credential-like
/// inherited variables are removed only when the user opted into
/// filter_subprocess_env, preserving gh/git/npm workflows by default.
pub fn process_env() -> Vec<String> {
    let env: Vec<String> = std::env::vars_os()
        .map(|(k, v)| {
            let k = k.to_string_lossy().into_owned();
            let v = v.to_string_lossy().into_owned();
            format!("{k}={v}")
        })
        .collect();
    let filtered = if filter_subprocess_env() {
        filter_env(&env)
    } else {
        filter_registered_credential_env(&env)
    };
    mark_agent_child(&filtered)
}

fn mark_agent_child(env: &[String]) -> Vec<String> {
    let mut out: Vec<String> = env
        .iter()
        .filter(|item| {
            let Some((key, _)) = item.split_once('=') else {
                return true;
            };
            key != AGENT_CHILD_ENV
        })
        .cloned()
        .collect();
    out.push(format!("{AGENT_CHILD_ENV}=1"));
    out
}

/// The environment every shell command runs in. Windows returns the process
/// env unchanged; the login-shell PATH merge is the shellrun slice's
/// machine and rides in later.
pub fn command_env() -> Vec<String> {
    process_env()
}

/// Renders the configured `[tools.shell]` env as sorted KEY=value pairs.
/// Sorted so identical config yields identical environment bytes; a key
/// that cannot appear in an environment entry is dropped rather than
/// mangled.
pub fn shell_env_overrides(env: &BTreeMap<String, String>) -> Vec<String> {
    let mut keys: Vec<&String> = env
        .keys()
        .filter(|k| !k.is_empty() && !k.contains(['=', '\0']))
        .collect();
    keys.sort();
    keys.iter().map(|k| format!("{k}={}", env[*k])).collect()
}

/// The key identity the env fold uses. It is a seam so a test can stand in
/// for Windows, whose environment block is case-insensitive.
pub fn env_key_equal(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// Folds the configured shell env under the host-owned session variables,
/// which win, so a preset value cannot displace TMPDIR. A session key also
/// displaces a preset one differing only in case: the child sees one entry.
pub fn shell_env_map(
    shell_env: &BTreeMap<String, String>,
    session_temp: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    if shell_env.is_empty() {
        return session_temp.clone();
    }
    let mut merged = shell_env.clone();
    for (k, v) in session_temp {
        let displaced: Vec<String> = merged
            .keys()
            .filter(|existing| *existing != k && env_key_equal(existing, k))
            .cloned()
            .collect();
        for existing in displaced {
            merged.remove(&existing);
        }
        merged.insert(k.clone(), v.clone());
    }
    merged
}

/// Applies KEY=value overrides over env, replacing any spelling of a key
/// exactly once (Go `SetEnvValue` via `applyEnvOverrides`).
pub fn apply_env_overrides(mut env: Vec<String>, overrides: &[String]) -> Vec<String> {
    for kv in overrides {
        let Some((key, value)) = kv.split_once('=') else {
            continue;
        };
        if key.is_empty() {
            continue;
        }
        env = set_env_value(env, key, value);
    }
    env
}

/// Replaces every entry of `key` with one `key=value`, appended when absent.
pub fn set_env_value(env: Vec<String>, key: &str, value: &str) -> Vec<String> {
    let mut out = Vec::with_capacity(env.len() + 1);
    let mut replaced = false;
    for kv in env {
        let Some((k, _)) = kv.split_once('=') else {
            out.push(kv);
            continue;
        };
        if env_key_equal(k, key) {
            if !replaced {
                out.push(format!("{key}={value}"));
                replaced = true;
            }
            continue;
        }
        out.push(kv);
    }
    if !replaced {
        out.push(format!("{key}={value}"));
    }
    out
}

/// The value of `key` as the child will see it: the last matching entry.
pub fn env_value(env: &[String], key: &str) -> Option<String> {
    env.iter()
        .rev()
        .find(|entry| {
            entry
                .split_once('=')
                .is_some_and(|(k, _)| env_key_equal(k, key))
        })
        .and_then(|entry| entry.split_once('=').map(|(_, v)| v.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_env_overrides_sorts_and_drops_invalid_keys() {
        let mut env = BTreeMap::new();
        env.insert(String::from("B"), String::from("2"));
        env.insert(String::from("A"), String::from("1"));
        env.insert(String::from("BAD=KEY"), String::from("x"));
        env.insert(String::new(), String::from("y"));
        let got = shell_env_overrides(&env);
        assert_eq!(got, vec![String::from("A=1"), String::from("B=2")]);
    }

    // Go TestShellEnvMapFoldsKeysCaseInsensitivelyOnWindows: a Windows
    // environment block is case-insensitive, so the session's TMPDIR must
    // displace a preset tmpdir rather than leave two entries the child
    // cannot keep.
    #[test]
    fn shell_env_map_folds_keys_case_insensitively_on_windows() {
        let mut shell = BTreeMap::new();
        shell.insert(String::from("tmpdir"), String::from("/preset"));
        shell.insert(String::from("CI"), String::from("1"));
        let mut session = BTreeMap::new();
        session.insert(String::from("TMPDIR"), String::from("/session"));
        let got = shell_env_map(&shell, &session);
        if cfg!(windows) {
            assert_eq!(got.len(), 2, "the preset casing must not survive: {got:?}");
            assert!(!got.contains_key("tmpdir"));
        } else {
            assert_eq!(got.len(), 3, "case-sensitive identity keeps both casings");
            assert_eq!(got.get("tmpdir").map(String::as_str), Some("/preset"));
        }
        assert_eq!(got.get("TMPDIR").map(String::as_str), Some("/session"));
        assert_eq!(got.get("CI").map(String::as_str), Some("1"));
    }

    #[test]
    fn apply_env_overrides_replaces_one_spelling() {
        let env = vec![String::from("A=1"), String::from("PATH=/usr/bin")];
        let got = apply_env_overrides(
            env,
            &[
                String::from("A=2"),
                String::from("TMPDIR=/t"),
                String::from("BROKEN"),
            ],
        );
        assert!(got.contains(&String::from("A=2")));
        assert!(got.contains(&String::from("TMPDIR=/t")));
        assert!(got.contains(&String::from("PATH=/usr/bin")));
        assert!(!got.contains(&String::from("A=1")));
        assert_eq!(got.len(), 3, "a KEY=value without = adds nothing: {got:?}");
    }

    #[test]
    fn set_env_value_replaces_any_spelling_once() {
        let env = vec![
            String::from("PATH=/a"),
            String::from("OTHER=1"),
            String::from("path=/b"),
        ];
        let got = set_env_value(env, "PATH", "/c");
        assert_eq!(got, vec![String::from("PATH=/c"), String::from("OTHER=1")]);
    }

    #[test]
    fn process_env_marks_the_agent_child() {
        let env = process_env();
        let marker = format!("{AGENT_CHILD_ENV}=1");
        assert!(
            env.contains(&marker),
            "the marker rides every subprocess env"
        );
        assert_eq!(
            env.iter()
                .filter(|kv| kv.starts_with(&format!("{AGENT_CHILD_ENV}=")))
                .count(),
            1,
            "one marker entry only"
        );
    }

    #[test]
    fn filter_env_drops_credential_named_entries() {
        let env = vec![
            String::from("HOME=/home/u"),
            String::from("AWS_SECRET_ACCESS_KEY=abcd"),
            String::from("PWD=/work"),
        ];
        let got = filter_env(&env);
        assert!(got.contains(&String::from("HOME=/home/u")));
        assert!(got.contains(&String::from("PWD=/work")));
        assert!(!got.contains(&String::from("AWS_SECRET_ACCESS_KEY=abcd")));
    }
}
