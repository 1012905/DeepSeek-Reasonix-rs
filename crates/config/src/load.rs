//! Layered loading: built-in defaults, then the given files lowest priority
//! first (docs/contracts/config.md §0), with the reference's per-layer
//! tolerance — missing files are skipped and a file that cannot be read or
//! parsed is skipped with a load warning instead of failing the load.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use reasonix_core::fileencoding::read_file_utf8;
use serde::Deserialize;

use crate::desktop::DesktopSection;
use crate::model::Config;
use crate::serve::{AuthMode, ServeSection};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// A caller-supplied layer path is empty or whitespace-only. The
    /// reference refuses such a path before any file access
    /// (path_access.go: "config path is empty").
    #[error("config path is empty")]
    EmptyPath,
}

impl Config {
    /// Loads layered configuration files, lowest priority first: built-in
    /// defaults, then each file's declared keys over what came before. The
    /// first path is the user layer; later paths are project layers, which
    /// the user-globals rule keeps away from [serve] and [storage]
    /// (load_project.go). Missing files are skipped; a file that fails to
    /// read or parse is skipped with a load warning — like the reference,
    /// bad TOML never fails the load.
    ///
    /// Each file is parsed exactly once, straight into an all-Optional delta
    /// and applied field-wise. The OPT-016 alternative (Value trees merged
    /// before one final decode) measured slower: the parser builds every
    /// unknown key the model ignores, while a delta decode skips them
    /// (benches/load.rs holds both numbers).
    pub fn load<P: AsRef<Path>>(layered_paths: &[P]) -> Result<Config, ConfigError> {
        for path in layered_paths {
            if is_blank_path(path.as_ref()) {
                return Err(ConfigError::EmptyPath);
            }
        }
        let mut config = Config::builtin_defaults();
        for (index, path) in layered_paths.iter().enumerate() {
            let path = path.as_ref();
            match decode_layer(path) {
                LayerOutcome::Missing => {}
                LayerOutcome::Invalid(error) => {
                    config
                        .load_warnings
                        .push(invalid_layer_warning(index, path, &error));
                }
                LayerOutcome::Decoded(delta) => {
                    apply_layer(&mut config, delta, index > 0);
                }
            }
        }
        Ok(config)
    }
}

fn is_blank_path(path: &Path) -> bool {
    match path.as_os_str().to_str() {
        Some(text) => text.trim().is_empty(),
        None => path.as_os_str().is_empty(),
    }
}

enum LayerOutcome {
    Missing,
    Invalid(String),
    Decoded(LayerDelta),
}

fn decode_layer(path: &Path) -> LayerOutcome {
    // statConfigPath: a missing file contributes nothing, silently.
    let bytes = match read_file_utf8(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return LayerOutcome::Missing,
        Err(error) => return LayerOutcome::Invalid(error.to_string()),
    };
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => return LayerOutcome::Invalid(error.to_string()),
    };
    match toml::from_str(&text) {
        Ok(delta) => LayerOutcome::Decoded(delta),
        Err(error) => LayerOutcome::Invalid(error.to_string()),
    }
}

fn invalid_layer_warning(index: usize, path: &Path, error: &str) -> String {
    // load.go warning texts: a broken user layer falls back to defaults; a
    // later layer is a project file and is dropped for this workspace.
    if index == 0 {
        format!(
            "user config {} is invalid ({}); using built-in defaults in memory without modifying the original file",
            path.display(),
            error
        )
    } else {
        format!(
            "project config {} is invalid ({}); ignored for this workspace",
            path.display(),
            error
        )
    }
}

/// One file's declared keys over the accumulated config. Every field is
/// optional so "declared" and "built-in default" stay distinct — the direct
/// translation of the reference decoding each file onto the live config.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LayerDelta {
    language: Option<String>,
    serve: Option<PartialServe>,
    desktop: Option<PartialDesktop>,
    secrets: Option<PartialSecrets>,
    storage: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PartialServe {
    auth_mode: Option<String>,
    token: Option<String>,
    password_hash: Option<String>,
    behind_proxy: Option<bool>,
    share_port: Option<i64>,
    remember_paired_devices: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PartialSecrets {
    protect_sensitive_files: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PartialDesktop {
    telemetry: Option<bool>,
    metrics: Option<bool>,
    tray: Option<String>,
    default_tool_approval_mode: Option<String>,
}

fn apply_layer(config: &mut Config, delta: LayerDelta, project_scope: bool) {
    // load_project.go userGlobals: a workspace file never gets to set these.
    // The rest of the user-globals set is unmodeled in this slice.
    if let Some(language) = delta.language {
        config.language = language;
    }
    if !project_scope {
        if let Some(serve) = delta.serve {
            apply_serve(&mut config.serve, serve);
        }
        if let Some(secrets) = delta.secrets {
            // A workspace file never arms or disarms the denylists
            // (load_project.go userGlobals holds SecretsConfig).
            if let Some(value) = secrets.protect_sensitive_files {
                config.secrets.protect_sensitive_files = value;
            }
        }
        if let Some(storage) = delta.storage {
            config.storage = storage;
        }
    }
    if let Some(desktop) = delta.desktop {
        apply_desktop(&mut config.desktop, desktop, project_scope);
    }
}

fn apply_serve(serve: &mut ServeSection, partial: PartialServe) {
    if let Some(raw) = partial.auth_mode {
        serve.auth_mode = AuthMode::normalize(&raw);
    }
    if let Some(value) = partial.token {
        serve.token = value;
    }
    if let Some(value) = partial.password_hash {
        serve.password_hash = value;
    }
    if let Some(value) = partial.behind_proxy {
        serve.behind_proxy = value;
    }
    if let Some(value) = partial.share_port {
        serve.share_port = value;
    }
    if let Some(value) = partial.remember_paired_devices {
        serve.remember_paired_devices = value;
    }
}

fn apply_desktop(desktop: &mut DesktopSection, partial: PartialDesktop, project_scope: bool) {
    if let Some(value) = partial.telemetry {
        desktop.telemetry = Some(value);
    }
    if let Some(value) = partial.metrics {
        desktop.metrics = Some(value);
    }
    if let Some(value) = partial.tray {
        desktop.tray = value;
    }
    // load_project.go's narrowPermissions keeps the approval posture
    // user-scope: a workspace file never decides what runs in it unattended.
    if !project_scope && let Some(value) = partial.default_tool_approval_mode {
        desktop.default_tool_approval_mode = value;
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, ConfigError, invalid_layer_warning};
    use crate::desktop::DesktopSection;
    use crate::serve::{AuthMode, ServeSection, host_serve_section};
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// Temp dirs live under target/ because the system temp path is denied in
    /// some environments (same approach as the session/serve test support).
    fn temp_dir() -> TempDir {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        std::fs::create_dir_all(&base).expect("create test-tmp base");
        tempfile::Builder::new()
            .prefix(".tmpconfig")
            .tempdir_in(&base)
            .expect("create config test temp dir")
    }
    fn write(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, content).expect("test fixture write");
        path
    }

    fn no_paths() -> Vec<PathBuf> {
        Vec::new()
    }

    #[test]
    fn empty_layer_list_yields_defaults() {
        // config.md §0: built-in defaults are the bottom layer.
        let loaded = Config::load(&no_paths()).expect("empty load");
        assert_eq!(loaded, Config::builtin_defaults());
    }

    #[test]
    fn missing_files_are_silent_defaults() {
        // statConfigPath: absence is not an error and not a warning.
        let dir = temp_dir();
        let absent_user = dir.path().join("absent-user.toml");
        let absent_project = dir.path().join("absent-project.toml");
        let loaded = Config::load(&[absent_user, absent_project]).expect("missing files load");
        assert_eq!(loaded, Config::builtin_defaults());
        assert!(loaded.load_warnings.is_empty());
    }

    #[test]
    fn unknown_keys_pass_through_silently() {
        // config.md §1 / CONFIG-002: unknown fields are ignored without
        // warning or refusal, at the top level and inside every table.
        let dir = temp_dir();
        let user = write(
            &dir,
            "user.toml",
            r#"totally_unknown = "x"
language = "zh"
[serve]
auth_mode = "token"
also_unknown = 3
[desktop]
telemetry = false
[agent]
compact_ratio = 0.85
"#,
        );
        let loaded = Config::load(&[user]).expect("unknown keys load");
        assert!(loaded.load_warnings.is_empty());
        assert_eq!(loaded.language, "zh");
        assert_eq!(loaded.serve.auth_mode, AuthMode::Token);
        assert!(!loaded.desktop_telemetry());
    }

    #[test]
    fn project_layer_overrides_user_layer() {
        // config.md §0: project reasonix.toml outranks the user config.
        let dir = temp_dir();
        let user = write(
            &dir,
            "user.toml",
            r#"language = "zh"
[desktop]
tray = "off"
metrics = false
"#,
        );
        let project = write(
            &dir,
            "project.toml",
            r#"language = "en"
[desktop]
metrics = true
"#,
        );
        let loaded = Config::load(&[user, project]).expect("layered load");
        assert!(loaded.load_warnings.is_empty());
        assert_eq!(loaded.language, "en");
        assert_eq!(loaded.desktop.tray, "off");
        assert!(loaded.desktop_metrics());
    }

    #[test]
    fn explicit_default_equal_value_still_overrides() {
        // Decoding onto the live config: a later layer that explicitly sets
        // an empty value wins over an earlier non-empty one.
        let dir = temp_dir();
        let user = write(&dir, "user.toml", "language = \"zh\"\n");
        let project = write(&dir, "project.toml", "language = \"\"\n");
        let loaded = Config::load(&[user, project]).expect("layered load");
        assert_eq!(loaded.language, "");
    }

    #[test]
    fn project_layer_cannot_set_serve_or_storage() {
        // load_project.go userGlobals: serve authentication and relocation
        // stay the user's, whatever a checked-in file claims.
        let dir = temp_dir();
        let user = write(
            &dir,
            "user.toml",
            r#"
[serve]
auth_mode = "token"
token = "user-token"
share_port = 8090
[storage]
state = "C:/state"
"#,
        );
        let project = write(
            &dir,
            "project.toml",
            r#"
[serve]
auth_mode = "none"
token = "project-token"
behind_proxy = true
[storage]
state = "D:/elsewhere"
"#,
        );
        let loaded = Config::load(&[user, project]).expect("layered load");
        assert!(loaded.load_warnings.is_empty());
        assert_eq!(loaded.serve.auth_mode, AuthMode::Token);
        assert_eq!(loaded.serve.token, "user-token");
        assert!(!loaded.serve.behind_proxy);
        assert_eq!(loaded.serve.share_port, 8090);
        assert_eq!(
            loaded.storage.get("state").map(String::as_str),
            Some("C:/state")
        );
    }

    #[test]
    fn the_user_layer_arms_secrets_and_the_project_layer_cannot() {
        // load_project.go userGlobals: SecretsConfig is the user's — a
        // cloned repository neither arms nor disarms the denylists.
        let dir = temp_dir();
        let user = write(
            &dir,
            "user.toml",
            "[secrets]
protect_sensitive_files = true
",
        );
        let loaded = Config::load(&[&user]).expect("user secrets load");
        assert!(loaded.secrets.protect_sensitive_files);
        let project = write(
            &dir,
            "project.toml",
            "[secrets]
protect_sensitive_files = false
",
        );
        let loaded = Config::load(&[&user, &project]).expect("layered secrets load");
        assert!(loaded.secrets.protect_sensitive_files);
        let absent_user = dir.path().join("absent-user.toml");
        let project_only = write(
            &dir,
            "project-only.toml",
            "[secrets]
protect_sensitive_files = true
",
        );
        let loaded = Config::load(&[absent_user, project_only]).expect("project-only load");
        assert!(!loaded.secrets.protect_sensitive_files);
    }

    #[test]
    fn project_serve_alone_yields_defaults() {
        // With no user layer, a project file's [serve] still cannot set it:
        // the held value is the built-in default.
        let dir = temp_dir();
        let absent_user = dir.path().join("absent-user.toml");
        let project = write(
            &dir,
            "project.toml",
            r#"
[serve]
auth_mode = "password"
password_hash = "hash"
"#,
        );
        let loaded = Config::load(&[absent_user, project]).expect("project-only load");
        assert_eq!(loaded.serve, ServeSection::default());
        assert!(loaded.storage.is_empty());
    }

    #[test]
    fn project_layer_may_set_desktop_keys() {
        // desktop telemetry/metrics/tray are not user-globals in the
        // reference (load_project.go userGlobals does not hold them).
        let dir = temp_dir();
        let project = write(
            &dir,
            "project.toml",
            r#"
[desktop]
telemetry = false
tray = "off"
"#,
        );
        let loaded = Config::load(&[project]).expect("project-only load");
        assert!(!loaded.desktop_telemetry());
        assert!(loaded.desktop_metrics());
        assert_eq!(loaded.desktop_tray(), "off");
    }

    #[test]
    fn auth_mode_normalizes_through_the_toml_layer() {
        let dir = temp_dir();
        let user = write(
            &dir,
            "user.toml",
            r#"
[serve]
auth_mode = "  PASSWORD  "
token = "  padded  "
"#,
        );
        let loaded = Config::load(&[user]).expect("auth load");
        assert_eq!(loaded.serve.auth_mode, AuthMode::Password);
        // The token is kept raw; the gate trims it.
        assert_eq!(loaded.serve.token, "  padded  ");
    }

    #[test]
    fn wrong_typed_known_key_invalidates_its_layer() {
        // The reference decodes transactionally: one mistyped value drops
        // the whole file, and earlier valid keys in it do not survive either.
        let dir = temp_dir();
        let user = write(
            &dir,
            "user.toml",
            "language = \"zh\"\n[serve]\nauth_mode = 5\n",
        );
        let project = write(&dir, "project.toml", "language = \"en\"\n");
        let loaded = Config::load(&[user, project]).expect("layered load");
        assert_eq!(loaded.language, "en");
        assert_eq!(loaded.serve.auth_mode, AuthMode::None);
        assert_eq!(loaded.load_warnings.len(), 1);
        assert!(
            loaded.load_warnings[0].starts_with("user config"),
            "warning = {}",
            loaded.load_warnings[0]
        );
    }

    #[test]
    fn mistyped_key_drops_its_layer_even_without_a_later_override() {
        // Nothing later overrides the fault, so the layer is dropped whole:
        // the delta decode of one file is transactional on its own.
        let dir = temp_dir();
        let user = write(
            &dir,
            "user.toml",
            "language = \"zh\"\n[serve]\nbehind_proxy = \"yes\"\n",
        );
        let loaded = Config::load(&[user]).expect("isolated load");
        assert_eq!(loaded.language, "");
        assert_eq!(loaded.serve, ServeSection::default());
        assert_eq!(loaded.load_warnings.len(), 1);
    }

    #[test]
    fn malformed_user_layer_falls_back_to_defaults() {
        let dir = temp_dir();
        let user = write(&dir, "user.toml", "language = [not closed\n");
        let loaded = Config::load(&[user]).expect("malformed load");
        assert_eq!(loaded.language, "");
        assert_eq!(loaded.serve, ServeSection::default());
        assert_eq!(loaded.desktop, DesktopSection::default());
        assert!(loaded.storage.is_empty());
        assert_eq!(loaded.load_warnings.len(), 1);
        assert!(
            loaded.load_warnings[0].starts_with("user config")
                && loaded.load_warnings[0].contains("invalid"),
            "warning = {}",
            loaded.load_warnings[0]
        );
    }

    #[test]
    fn malformed_project_layer_is_ignored_for_this_workspace() {
        // load.go: the project file's warning leaves the user values in force.
        let dir = temp_dir();
        let user = write(&dir, "user.toml", "language = \"zh\"\n");
        let project = write(&dir, "project.toml", "[desktop\nbroken");
        let loaded = Config::load(&[user, project]).expect("layered load");
        assert_eq!(loaded.language, "zh");
        assert_eq!(loaded.load_warnings.len(), 1);
        assert!(
            loaded.load_warnings[0].starts_with("project config"),
            "warning = {}",
            loaded.load_warnings[0]
        );
    }

    #[test]
    fn warning_texts_match_the_reference_shape() {
        let warning = invalid_layer_warning(0, Path::new("u.toml"), "boom");
        assert_eq!(
            warning,
            "user config u.toml is invalid (boom); using built-in defaults in memory without modifying the original file"
        );
        let warning = invalid_layer_warning(1, Path::new("p.toml"), "boom");
        assert_eq!(
            warning,
            "project config p.toml is invalid (boom); ignored for this workspace"
        );
    }

    #[test]
    fn blank_layer_path_is_refused() {
        // path_access.go: "config path is empty".
        assert_eq!(
            Config::load(&[Path::new("")]).err(),
            Some(ConfigError::EmptyPath)
        );
        assert_eq!(
            Config::load(&[Path::new("  ")]).err(),
            Some(ConfigError::EmptyPath)
        );
    }

    #[test]
    fn host_serve_view_over_a_layered_load() {
        // The host flow: layered load, then the credential-free serve view.
        let dir = temp_dir();
        let user = write(
            &dir,
            "user.toml",
            r#"
[serve]
auth_mode = "token"
token = "secret"
behind_proxy = true
share_port = 8090
"#,
        );
        let project = write(&dir, "project.toml", "language = \"en\"\n");
        let loaded = Config::load(&[user, project]).expect("layered load");
        let serve = host_serve_section(&loaded);
        assert_eq!(serve.auth_mode, AuthMode::None);
        assert_eq!(serve.token, "");
        assert_eq!(serve.password_hash, "");
        assert!(!serve.behind_proxy);
        assert_eq!(serve.share_port, 8090);
        assert_eq!(loaded.language, "en");
    }

    #[test]
    fn storage_roundtrip_shapes() {
        // [storage] decodes as a plain string map keyed by root id.
        let dir = temp_dir();
        let user = write(
            &dir,
            "user.toml",
            r#"
[storage]
state = "~/state"
cache = "C:/cache"
"#,
        );
        let loaded = Config::load(&[user]).expect("storage load");
        let mut expected = BTreeMap::new();
        expected.insert("state".to_owned(), "~/state".to_owned());
        expected.insert("cache".to_owned(), "C:/cache".to_owned());
        assert_eq!(loaded.storage, expected);
    }

    #[test]
    fn desktop_section_shape() {
        let section = DesktopSection::default();
        assert!(section.telemetry.is_none());
        assert!(section.metrics.is_none());
        assert_eq!(section.tray, "");
    }
}
