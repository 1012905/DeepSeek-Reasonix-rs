//! The configuration the desktop host's boot reads, reached through this
//! frontend because the composition root reaches config only through
//! frontend crates (ADR-0001 key choice 3).

use std::path::{Path, PathBuf};

pub use reasonix_config::{self, Config, ConfigError, auto_workspace_dir, user_config_path};

/// Go `config.Load()`: the user config, then the project candidates in
/// priority order. Missing files are skipped by the loader; the layering
/// treats `paths[0]` as the user layer and the rest as project layers.
pub fn load_config(cwd: &Path) -> Result<Config, ConfigError> {
    let project = [
        cwd.join("reasonix.toml"),
        cwd.join(".reasonix").join("reasonix.toml"),
    ];
    let layered: Vec<PathBuf> = std::iter::once(user_config_path()).chain(project).collect();
    Config::load(&layered)
}

/// Go `config.ReasonixHomeDir()`, re-exported for the host's `-instance-id`
/// mode: the data home the root chain resolves, empty when none can be.
pub fn reasonix_home_dir() -> PathBuf {
    reasonix_config::reasonix_home_dir()
}
