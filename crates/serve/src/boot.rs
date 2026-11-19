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
    Config::load(&layered)
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
