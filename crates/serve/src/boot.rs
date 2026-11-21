//! The configuration the desktop host's boot reads, reached through this
//! frontend because the composition root reaches config only through
//! frontend crates (ADR-0001 key choice 3).

use std::path::{Path, PathBuf};

pub use reasonix_config::{self, Config, ConfigError, auto_workspace_dir, user_config_path};

/// Go `config.Load()`: the user config, then the project config. The project
/// layer is ONE file selected by Go `ProjectConfigPath` (paths.md §2): the
/// root `reasonix.toml` wins whenever a stat on it says anything but
/// IsNotExist — a broken link counts as present, like Go's Lstat — and only
/// absence falls through to probing `.reasonix/reasonix.toml`. When neither
/// exists the root path stays, as the creation target. It is a fallback, not
/// a merge: two project files never both contribute.
fn project_config_path(cwd: &Path) -> PathBuf {
    let candidates = [
        cwd.join("reasonix.toml"),
        cwd.join(".reasonix").join("reasonix.toml"),
    ];
    candidates
        .iter()
        .find(|candidate| {
            !matches!(
                std::fs::symlink_metadata(candidate),
                Err(ref err) if err.kind() == std::io::ErrorKind::NotFound
            )
        })
        .unwrap_or(&candidates[0])
        .to_path_buf()
}

/// Go `config.Load()`: the user config, then the selected project config.
/// Missing files are skipped by the loader; the layering treats `paths[0]`
/// as the user layer and the rest as project layers.
pub fn load_config(cwd: &Path) -> Result<Config, ConfigError> {
    let layered: Vec<PathBuf> = std::iter::once(user_config_path())
        .chain(std::iter::once(project_config_path(cwd)))
        .collect();
    let config = Config::load(&layered)?;
    arm_secrets(&config);
    Ok(config)
}

/// Arms the read-tool denylists from the loaded config. [secrets] is
/// user-global, so arming package globals right after the load — before any
/// subprocess can spawn — is correct for every concurrent workspace (Go
/// build.go arms the same globals).
pub fn arm_secrets(config: &Config) {
    reasonix_tools::builtin::set_protect_sensitive_files(config.secrets.protect_sensitive_files);
}

/// How long an approval or ask prompt blocks when the configuration names a
/// posture. The reference leaves the value to each frontend's `boot.Options`
/// (`ApprovalTimeout`), and this build has no interactive frontend of its
/// own; the desktop's window is the one that would set it, so a resolved
/// posture here waits a day rather than forever — a walked-away user must
/// not wedge the runtime. A configuration that named no posture keeps the
/// headless posture, where nothing waits at all.
pub const APPROVAL_PROMPT_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(24 * 60 * 60);

/// The runtime approval surface the resolved configuration asks for, or
/// `None` when it asks for none (Go: `withWindowPosture` + the publish-time
/// `EnableInteractiveApproval`).
///
/// The posture is read through [Config]'s own accessor, which already
/// normalizes aliases and keeps an unrecognized value at ask. Absent means
/// absent: this function never invents an approval the user did not
/// configure, and it never widens one they did.
pub fn runtime_approval(config: &Config) -> Option<reasonix_agent::approval::ApprovalConfig> {
    config.desktop_default_tool_approval_mode().map(|mode| {
        reasonix_agent::approval::ApprovalConfig {
            mode,
            timeout: APPROVAL_PROMPT_TIMEOUT,
        }
    })
}

/// Go `config.ReasonixHomeDir()`, re-exported for the host's `-instance-id`
/// mode: the data home the root chain resolves, empty when none can be.
pub fn reasonix_home_dir() -> PathBuf {
    reasonix_config::reasonix_home_dir()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> tempfile::TempDir {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("target")
            .join("test-tmp");
        fs::create_dir_all(&base).expect("create test-tmp base");
        tempfile::Builder::new()
            .prefix(name)
            .tempdir_in(&base)
            .expect("create boot test temp dir")
    }

    // [secrets] arms the read-tool denylists at boot (Go build.go:230): the
    // loaded value reaches the tools global, default off.
    #[test]
    fn a_configured_secrets_toggle_arms_the_read_denylists() {
        reasonix_tools::builtin::set_protect_sensitive_files(false);
        arm_secrets(&Config {
            secrets: reasonix_config::SecretsSection {
                protect_sensitive_files: true,
            },
            ..Config::default()
        });
        assert!(reasonix_tools::builtin::protect_sensitive_files());
        arm_secrets(&Config::default());
        assert!(!reasonix_tools::builtin::protect_sensitive_files());
    }

    // M-2: two coexisting project files must read the root one only — the
    // project layer is a fallback chain, not a merge (config.md §12.6).
    #[test]
    fn coexisting_project_files_read_the_root_one_only() {
        let cwd = scratch(".bootboth");
        fs::write(
            cwd.path().join("reasonix.toml"),
            "language = \"root-marker\"\n",
        )
        .unwrap();
        fs::create_dir_all(cwd.path().join(".reasonix")).unwrap();
        fs::write(
            cwd.path().join(".reasonix").join("reasonix.toml"),
            "language = \"nested-marker\"\n",
        )
        .unwrap();
        let config = load_config(cwd.path()).unwrap();
        assert_eq!(config.language, "root-marker");
    }

    #[test]
    fn a_missing_root_falls_back_to_the_nested_file() {
        let cwd = scratch(".bootnested");
        fs::create_dir_all(cwd.path().join(".reasonix")).unwrap();
        fs::write(
            cwd.path().join(".reasonix").join("reasonix.toml"),
            "language = \"nested-marker\"\n",
        )
        .unwrap();
        let config = load_config(cwd.path()).unwrap();
        assert_eq!(config.language, "nested-marker");
    }

    #[test]
    fn neither_file_present_loads_with_no_project_layer() {
        let cwd = scratch(".bootnone");
        let config = load_config(cwd.path()).unwrap();
        // Whatever the user layer holds, no project marker can appear.
        assert_ne!(config.language, "root-marker");
        assert_ne!(config.language, "nested-marker");
    }
}
