//! Installed-plugin state: the only source a sidecar may be launched from.
//!
//! Parity target: `internal/ext/pluginpkg` (installed state, manifest, runtime
//! spec). The authorization invariant lives here rather than in a doc comment:
//! enumeration reads `<Reasonix home>/plugin-packages.json`, keeps only
//! entries that are enabled and declare a `runtime` block, and resolves each
//! root beneath that home. Nothing in a project configuration can contribute a
//! runtime, because no API here accepts one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// File holding the installed-plugin state beneath the Reasonix home.
pub const STATE_FILE: &str = "plugin-packages.json";
/// Native plugin manifest name.
pub const NATIVE_MANIFEST: &str = "reasonix-plugin.json";
/// The one supported manifest API version.
pub const MANIFEST_API_VERSION_V2: &str = "reasonix.io/plugin/v2";
/// Directory installed packages live in, beneath the Reasonix home.
pub const PLUGINS_DIR: &str = "plugins";

/// One entry of the persisted installed state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct InstalledPlugin {
    pub name: String,
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, rename = "manifestKind")]
    pub manifest_kind: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub generation: u64,
}

/// The persisted state document.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct InstalledState {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub plugins: Vec<InstalledPlugin>,
}

/// The manifest's runtime declaration: what to run and what it may claim.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct RuntimeSpec {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default, rename = "tightenOnly")]
    pub tighten_only: bool,
    #[serde(default, rename = "dynamicTools")]
    pub dynamic_tools: bool,
    #[serde(default, rename = "killOnTimeout")]
    pub kill_on_timeout: bool,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub intercepts: Vec<String>,
    #[serde(default)]
    pub replaces: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default, rename = "timeoutMillis")]
    pub timeout_millis: u64,
    #[serde(default)]
    pub tools: Vec<RuntimeTool>,
}

impl RuntimeSpec {
    /// The tool names this runtime declares, in manifest order.
    pub fn tool_names(&self) -> Vec<String> {
        self.tools.iter().map(|tool| tool.name.clone()).collect()
    }

    /// Reports whether the runtime declares one capability string.
    pub fn has_capability(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|have| have == capability)
    }
}

/// One model-callable tool the runtime serves.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct RuntimeTool {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "readOnly")]
    pub read_only: bool,
}

/// A capability reference in a v2 manifest, used for both requires and provides.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct CapabilityRef {
    #[serde(default)]
    pub namespace: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub version: String,
    #[serde(default, rename = "versionRange")]
    pub version_range: String,
    #[serde(default, rename = "schemaHash")]
    pub schema_hash: String,
    #[serde(default)]
    pub optional: bool,
}

impl CapabilityRef {
    /// The `namespace/kind/id` ceiling key.
    pub fn key(&self) -> String {
        format!(
            "{}/{}/{}",
            self.namespace.trim(),
            self.kind.trim(),
            self.id.trim()
        )
    }
}

/// The manifest fields this slice reads.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RuntimeManifest {
    #[serde(default, rename = "apiVersion")]
    pub api_version: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub runtime: Option<RuntimeSpec>,
    #[serde(default)]
    pub requires: Vec<CapabilityRef>,
    #[serde(default)]
    pub provides: Vec<CapabilityRef>,
}

/// An installed, enabled package that declares a runtime.
#[derive(Debug, Clone)]
pub struct InstalledRuntimePackage {
    pub installed: InstalledPlugin,
    pub manifest: RuntimeManifest,
    pub root: PathBuf,
}

impl InstalledRuntimePackage {
    /// The plugin ID every protocol call and contribution uses.
    pub fn plugin_id(&self) -> &str {
        &self.installed.name
    }

    /// The runtime declaration. Enumeration only yields packages with one.
    pub fn runtime(&self) -> &RuntimeSpec {
        self.manifest
            .runtime
            .as_ref()
            .expect("enumerated packages declare a runtime")
    }

    /// The version a sidecar is told it is: installed first, manifest second.
    pub fn version(&self) -> &str {
        let installed = self.installed.version.trim();
        if installed.is_empty() {
            self.manifest.version.trim()
        } else {
            installed
        }
    }

    /// The identity block a sidecar sees in its environment.
    pub fn runtime_env(&self) -> RuntimeEnv {
        RuntimeEnv {
            root: self.root.display().to_string(),
            name: self.installed.name.clone(),
            version: self.installed.version.clone(),
            manifest_version: self.manifest.version.clone(),
        }
    }
}

/// The env identity triple handed to a spawned sidecar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeEnv {
    pub root: String,
    pub name: String,
    pub version: String,
    pub manifest_version: String,
}

/// Why enumeration could not read the installed state.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstallStateError {
    #[error("install state {path}: {message}")]
    Unreadable { path: String, message: String },
}

/// A package that was skipped, with the reason the host can report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedPackage {
    pub name: String,
    pub reason: String,
}

/// The enumeration result: launch candidates plus diagnostics.
#[derive(Debug, Clone, Default)]
pub struct RuntimePackages {
    pub packages: Vec<InstalledRuntimePackage>,
    pub warnings: Vec<String>,
}

/// Reads the installed state beneath `home`.
pub fn load_installed_state(home: &Path) -> Result<InstalledState, InstallStateError> {
    let path = home.join(STATE_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(InstalledState {
                version: 1,
                plugins: Vec::new(),
            });
        }
        Err(err) => {
            return Err(InstallStateError::Unreadable {
                path: path.display().to_string(),
                message: err.to_string(),
            });
        }
    };
    serde_json::from_slice(&bytes).map_err(|err| InstallStateError::Unreadable {
        path: path.display().to_string(),
        message: err.to_string(),
    })
}

/// Resolves an installed root: absolute paths stay, relative ones join `home`.
pub fn resolve_root(home: &Path, root: &str) -> PathBuf {
    let path = Path::new(root);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        home.join(path)
    }
}

/// Enumerates the installed, enabled packages that declare a runtime.
///
/// Sorted by plugin name so status output and warning order are stable, which
/// is also what makes the parallel start's result order deterministic.
pub fn load_runtime_packages(home: &Path) -> Result<RuntimePackages, InstallStateError> {
    let state = load_installed_state(home)?;
    let mut result = RuntimePackages::default();
    let mut out: Vec<InstalledRuntimePackage> = Vec::new();
    for installed in state.plugins {
        if !installed.enabled {
            continue;
        }
        let root = resolve_root(home, &installed.root);
        let manifest_path = root.join(NATIVE_MANIFEST);
        let manifest = match read_manifest(&manifest_path) {
            Ok(manifest) => manifest,
            Err(reason) => {
                result
                    .warnings
                    .push(format!("{}: {reason}", installed.name));
                continue;
            }
        };
        if manifest.runtime.is_none() {
            continue;
        }
        out.push(InstalledRuntimePackage {
            installed,
            manifest,
            root,
        });
    }
    out.sort_by(|a, b| a.installed.name.cmp(&b.installed.name));
    result.packages = out;
    Ok(result)
}

/// Reads and validates one package manifest.
fn read_manifest(path: &Path) -> Result<RuntimeManifest, String> {
    let bytes = std::fs::read(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let manifest: RuntimeManifest =
        serde_json::from_slice(&bytes).map_err(|err| format!("parse {}: {err}", path.display()))?;
    if manifest.api_version != MANIFEST_API_VERSION_V2 {
        return Err(format!(
            "manifest {} declares apiVersion {:?}; only {MANIFEST_API_VERSION_V2} is supported",
            path.display(),
            manifest.api_version
        ));
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn write(home: &Path, name: &str, manifest: &str, enabled: bool) {
        let root = home.join(PLUGINS_DIR).join(name);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(NATIVE_MANIFEST), manifest).unwrap();
        let state_path = home.join(STATE_FILE);
        let mut state: Value = std::fs::read(&state_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_else(|| json!({"version": 1, "plugins": []}));
        let plugins = state["plugins"].as_array_mut().expect("plugins array");
        plugins.retain(|entry| entry["name"] != json!(name));
        plugins.push(json!({
            "name": name,
            "root": format!("{PLUGINS_DIR}/{name}"),
            "version": "1.0.0",
            "enabled": enabled
        }));
        std::fs::write(
            &state_path,
            serde_json::to_vec_pretty(&state).expect("state"),
        )
        .unwrap();
    }

    fn manifest(name: &str, runtime: Option<&str>) -> String {
        let runtime = match runtime {
            Some(runtime) => format!(r#","runtime":{runtime}"#),
            None => String::new(),
        };
        format!(
            r#"{{"apiVersion":"{MANIFEST_API_VERSION_V2}","name":"{name}","version":"2.0.0"{runtime}}}"#
        )
    }

    #[test]
    fn missing_state_yields_no_packages() {
        let dir = std::env::temp_dir().join(format!("reasonix-ext-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let packages = load_runtime_packages(&dir).unwrap();
        assert!(packages.packages.is_empty());
        assert!(packages.warnings.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_enabled_runtime_packages_are_launch_candidates() {
        let dir = std::env::temp_dir().join(format!("reasonix-ext-enabled-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write(
            &dir,
            "alpha",
            &manifest("alpha", Some(r#"{"command":"/opt/a"}"#)),
            true,
        );
        write(&dir, "beta", &manifest("beta", None), true);
        let packages = load_runtime_packages(&dir).unwrap();
        assert_eq!(
            packages
                .packages
                .iter()
                .map(|p| p.plugin_id().to_string())
                .collect::<Vec<_>>(),
            ["alpha"]
        );
        assert!(packages.warnings.is_empty());

        write(
            &dir,
            "alpha",
            &manifest("alpha", Some(r#"{"command":"/opt/a"}"#)),
            false,
        );
        assert!(load_runtime_packages(&dir).unwrap().packages.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wrong_api_version_is_a_warning_not_a_launch() {
        let dir = std::env::temp_dir().join(format!("reasonix-ext-api-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write(
            &dir,
            "legacy",
            r#"{"apiVersion":"reasonix.io/plugin/v1","name":"legacy","runtime":{"command":"/opt/l"}}"#,
            true,
        );
        let packages = load_runtime_packages(&dir).unwrap();
        assert!(packages.packages.is_empty());
        assert_eq!(packages.warnings.len(), 1);
        assert!(packages.warnings[0].contains("only reasonix.io/plugin/v2"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn installed_version_wins_over_the_manifest() {
        let package = InstalledRuntimePackage {
            installed: InstalledPlugin {
                name: "alpha".into(),
                version: "1.0.0".into(),
                enabled: true,
                ..InstalledPlugin::default()
            },
            manifest: RuntimeManifest {
                name: "alpha".into(),
                version: "2.0.0".into(),
                runtime: Some(RuntimeSpec::default()),
                ..RuntimeManifest::default()
            },
            root: PathBuf::from("/opt/alpha"),
        };
        assert_eq!(package.version(), "1.0.0");
        assert_eq!(package.runtime_env().root, "/opt/alpha");
    }
}
