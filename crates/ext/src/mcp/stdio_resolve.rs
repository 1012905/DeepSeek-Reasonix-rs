//! Stdio launch resolution: working directory, environment fold, and executable
//! lookup (contract §4; Go `transport_stdio.go` `stdioWorkingDir`, `mergeEnv`,
//! `resolveStdioExecutable`).

use std::path::{Path, PathBuf};

use super::stdio::StdioError;

/// Every subprocess the host starts carries this marker; a manifest cannot unset
/// or spoof it. A child may unset it, so it is friction against accidents, not a
/// boundary (Go `secrets.AgentChildEnv`).
pub const AGENT_CHILD_ENV: &str = "REASONIX_AGENT_CHILD";
/// Extensions tried for a bare command when PATHEXT is unset (Go `defaultPathExt`).
#[cfg(windows)]
const DEFAULT_PATH_EXT: &str = ".COM;.EXE;.BAT;.CMD";

/// Resolves the child working directory (Go `stdioWorkingDir`): explicit `Dir`
/// wins; a repository-declared server falls back to the workspace root; a
/// user-installed server keeps the inherited host cwd.
pub fn working_dir(dir: &str, workspace_root: &str, require_approval: bool) -> Option<String> {
    let dir = dir.trim();
    if !dir.is_empty() {
        return Some(dir.to_string());
    }
    if require_approval {
        let root = workspace_root.trim();
        if !root.is_empty() {
            return Some(root.to_string());
        }
    }
    None
}

/// Builds the subprocess environment: inherited base, manifest overrides
/// layered in sorted key order, then the forced identity marker last, so a
/// manifest can tune the environment but cannot forge its own identity.
pub fn build_env(overrides: &[(String, String)]) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = std::env::vars().collect();
    let mut sorted: Vec<&(String, String)> = overrides.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    for (key, value) in sorted {
        set_env(&mut env, key, value);
    }
    env.retain(|(key, _)| !env_key_equal(key, AGENT_CHILD_ENV));
    env.push((AGENT_CHILD_ENV.to_string(), "1".to_string()));
    env
}

fn set_env(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    if let Some(existing) = env.iter_mut().find(|(k, _)| env_key_equal(k, key)) {
        existing.1 = value.to_string();
    } else {
        env.push((key.to_string(), value.to_string()));
    }
}

fn env_key_equal(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

fn env_value(env: &[(String, String)], key: &str) -> Option<String> {
    env.iter()
        .rev()
        .find(|(k, _)| env_key_equal(k, key))
        .map(|(_, v)| v.clone())
}

/// Resolves the launchable executable (Go `resolveStdioExecutable`).
///
/// A command carrying a path separator resolves against the working directory;
/// a bare command is looked up on the environment PATH.
pub fn resolve_executable(
    plugin: &str,
    command: &str,
    work_dir: Option<&str>,
    env: &[(String, String)],
) -> Result<PathBuf, StdioError> {
    let trimmed = command.trim();
    if has_path_separator(trimmed) {
        let relative = Path::new(trimmed);
        let absolute = if relative.is_absolute() {
            relative.to_path_buf()
        } else {
            let base = work_dir
                .map(PathBuf::from)
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_default();
            base.join(relative)
        };
        return Ok(absolute);
    }
    let path_list = env_value(env, "PATH").unwrap_or_default();
    let pathext = env_value(env, "PATHEXT").unwrap_or_default();
    if let Some(found) = look_path(trimmed, &path_list, &pathext) {
        return Ok(found);
    }
    Err(StdioError::CommandMissing {
        plugin: plugin.to_string(),
        command: trimmed.to_string(),
    })
}

fn has_path_separator(command: &str) -> bool {
    command.contains('/') || command.contains('\\')
}

fn look_path(command: &str, path_list: &str, pathext: &str) -> Option<PathBuf> {
    for dir in split_path_list(path_list) {
        if dir.is_empty() || !Path::new(&dir).is_absolute() {
            continue;
        }
        for name in command_names(command, pathext) {
            let candidate = Path::new(&dir).join(&name);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(windows)]
fn split_path_list(path_list: &str) -> Vec<String> {
    path_list
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(unix)]
fn split_path_list(path_list: &str) -> Vec<String> {
    path_list
        .split(':')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(not(any(windows, unix)))]
fn split_path_list(_path_list: &str) -> Vec<String> {
    Vec::new()
}

#[cfg(windows)]
fn command_names(command: &str, pathext: &str) -> Vec<String> {
    let pathext = if pathext.trim().is_empty() {
        DEFAULT_PATH_EXT
    } else {
        pathext
    };
    let mut names = vec![command.to_string()];
    let mut seen = std::collections::HashSet::new();
    seen.insert(command.to_ascii_lowercase());
    for ext in pathext.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let ext = match ext.strip_prefix('.') {
            Some(rest) => format!(".{rest}"),
            None => format!(".{ext}"),
        };
        let name = format!("{command}{ext}");
        if seen.insert(name.to_ascii_lowercase()) {
            names.push(name);
        }
    }
    names
}

#[cfg(unix)]
fn command_names(command: &str, _pathext: &str) -> Vec<String> {
    vec![command.to_string()]
}

#[cfg(not(any(windows, unix)))]
fn command_names(command: &str, _pathext: &str) -> Vec<String> {
    vec![command.to_string()]
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(info) = std::fs::metadata(path) else {
        return false;
    };
    if !info.is_file() {
        return false;
    }
    #[cfg(windows)]
    {
        true
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        info.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(any(windows, unix)))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_dir_follows_the_go_precedence() {
        assert_eq!(
            working_dir("/explicit", "/workspace", false),
            Some("/explicit".to_string())
        );
        assert_eq!(working_dir("", "/workspace", false), None);
        assert_eq!(
            working_dir("", "/workspace", true),
            Some("/workspace".to_string())
        );
        assert_eq!(working_dir("", "", true), None);
    }

    #[test]
    fn env_overrides_win_and_identity_marker_cannot_be_forged() {
        let overrides = vec![
            ("REASONIX_TEST_INJECTED".into(), "manifest".into()),
            (AGENT_CHILD_ENV.into(), "spoofed".into()),
        ];
        let env = build_env(&overrides);
        let lookup = |key: &str| {
            env.iter()
                .find(|(k, _)| env_key_equal(k, key))
                .map(|(_, v)| v.clone())
        };
        assert_eq!(
            lookup("REASONIX_TEST_INJECTED").as_deref(),
            Some("manifest")
        );
        assert_eq!(lookup(AGENT_CHILD_ENV).as_deref(), Some("1"));
        assert_eq!(
            env.iter()
                .filter(|(k, _)| env_key_equal(k, AGENT_CHILD_ENV))
                .count(),
            1,
            "one marker entry only"
        );
        assert_eq!(env.last().map(|(k, _)| k.as_str()), Some(AGENT_CHILD_ENV));
    }

    #[test]
    fn bare_command_missing_from_path_is_a_typed_error() {
        let env = vec![("PATH".into(), "/usr/bin:/bin".into())];
        let err = resolve_executable("demo", "no-such-binary-xyzzy", None, &env).unwrap_err();
        assert_eq!(
            err,
            StdioError::CommandMissing {
                plugin: "demo".into(),
                command: "no-such-binary-xyzzy".into()
            }
        );
    }

    #[cfg(windows)]
    #[test]
    fn env_keys_fold_case_insensitively_on_windows() {
        assert!(env_key_equal("Path", "PATH"));
        let mut env = vec![("Path".into(), "/old".into())];
        set_env(&mut env, "PATH", "/new");
        assert_eq!(env.len(), 1);
        assert_eq!(env[0].1, "/new");
    }
}
