//! Cross-source merging by name, and the positional-decode isolation that
//! keeps one vendor's defaults off another's entry (load.go mergeTOMLProviders
//! / mergeTOMLPlugins / decode.go isolateDecodedProviders).
//!
//! Two independent problems meet here. Merging: TOML decoding replaces
//! `[[providers]]` and `[[plugins]]` wholesale, so the last file read would
//! otherwise drop every entry the earlier file declared. Isolation: array-of-
//! tables decoding is positional, so an entry written by the user would inherit
//! whatever occupied its index in the accumulated list.

use std::collections::BTreeMap;

use crate::model::Config;
use crate::plugin::{
    PluginEntry, PluginSource, merge_disabled_tool_policies, normalize_plugin_command_line,
};
use crate::provider::ProviderEntry;

/// Which layer a provider or plugin entry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrySource {
    /// The user's own config.toml.
    User,
    /// A workspace reasonix.toml.
    Project,
}

impl EntrySource {
    /// The scope a path belongs to (load.go providerSourceForPath): the user
    /// layer is the first path the loader was given, every later one a project.
    pub fn for_layer(index: usize) -> EntrySource {
        if index == 0 {
            EntrySource::User
        } else {
            EntrySource::Project
        }
    }
}

/// The outcome of merging providers across sources.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MergedProviders {
    pub providers: Vec<ProviderEntry>,
    /// The winning source per provider name.
    pub sources: BTreeMap<String, EntrySource>,
    /// Project entries a user entry of the same name displaced.
    pub shadowed_project: Vec<ProviderEntry>,
}

/// Merges `[[providers]]` across sources by name (load.go mergeTOMLProviders).
///
/// A user-global provider wins over a same-named project provider; project
/// providers only fill names the user config does not define. Official legacy
/// aliases stay distinct here: they can carry different default models and
/// effort capabilities.
pub fn merge_providers(sources: &[(EntrySource, Vec<ProviderEntry>)]) -> MergedProviders {
    let mut merged = MergedProviders::default();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    for (source, entries) in sources {
        for entry in entries {
            let key = merge_key(&entry.name);
            if let Some(existing) = index.get(&key).copied() {
                let current = merged.sources[&key];
                match (current, source) {
                    // A user entry displaces the project entry it shadowed.
                    (EntrySource::Project, EntrySource::User) => {
                        merged
                            .shadowed_project
                            .push(merged.providers[existing].clone());
                        merged.providers[existing] = entry.clone();
                        merged.sources.insert(key, EntrySource::User);
                    }
                    // A project entry of a user-global name never lands.
                    (EntrySource::User, EntrySource::Project) => {
                        merged.shadowed_project.push(entry.clone());
                    }
                    // Same source twice: the later declaration wins in place.
                    _ => {
                        merged
                            .shadowed_project
                            .push(merged.providers[existing].clone());
                        merged.providers[existing] = entry.clone();
                    }
                }
                continue;
            }
            index.insert(key.clone(), merged.providers.len());
            merged.providers.push(entry.clone());
            merged.sources.insert(key, *source);
        }
    }
    merged
}

/// The identity two entries merge on: the provider name, trimmed.
fn merge_key(name: &str) -> String {
    name.trim().to_owned()
}

/// Merges `[[plugins]]` across sources by name (load.go mergeTOMLPlugins):
/// later sources win, and the earlier source's disabled-tool additions are kept
/// because hiding a tool is a declaration, not an override.
///
/// Each entry's command line is normalized on the way in, exactly once, as the
/// reference does before any consumer sees it.
pub fn merge_plugins(sources: &[(EntrySource, Vec<PluginEntry>)]) -> Vec<PluginEntry> {
    let mut merged: Vec<PluginEntry> = Vec::new();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    let mut disabled: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (layer, entries) in sources {
        for entry in entries {
            let (mut entry, _) = normalize_plugin_command_line(entry);
            // load.go tags each TOML entry by its file layer before merge.
            entry.source = match layer {
                EntrySource::User => PluginSource::UserConfig,
                EntrySource::Project => PluginSource::ProjectConfig,
            };
            let key = merge_key(&entry.name);
            if let Some(existing) = index.get(&key).copied() {
                let base = disabled.remove(&key).unwrap_or_default();
                entry.disabled_tools = merge_disabled_tool_policies(&base, &entry.disabled_tools);
                disabled.insert(key, entry.disabled_tools.clone());
                merged[existing] = entry;
                continue;
            }
            disabled.insert(key.clone(), entry.disabled_tools.clone());
            index.insert(key, merged.len());
            merged.push(entry);
        }
    }
    merged
}

/// One file's declared provider and plugin lists, plus whether the file
/// declared `providers` at all.
#[derive(Debug, Default)]
pub struct DeclaredLists {
    pub providers: Vec<ProviderEntry>,
    pub plugins: Vec<PluginEntry>,
    /// `[[providers]]` was present. An absent key means "no opinion", an
    /// explicit empty list means "every provider removed".
    pub providers_defined: bool,
}

impl DeclaredLists {
    /// The lists one decoded layer declares. The delta's `Option` already
    /// answers the absent-versus-empty question, which is why the layer decode
    /// carries them as options rather than as plain vectors.
    pub fn from_delta(delta: &crate::delta::LayerDelta) -> DeclaredLists {
        DeclaredLists {
            providers: delta.providers.clone().unwrap_or_default(),
            plugins: delta.plugins.clone().unwrap_or_default(),
            providers_defined: delta.providers.is_some(),
        }
    }
}

/// Decodes one TOML text's provider and plugin lists without touching any
/// accumulated config.
///
/// This is the isolation boundary the reference names in
/// `isolateDecodedProviders`: array-of-tables decoding is positional, so a
/// caller that decodes onto an accumulated config lets a declared entry inherit
/// the fields of whatever occupied its index. Decoding onto a fresh value means
/// there is nothing to inherit; [`isolate_providers`] is what a caller that
/// must decode onto a live config calls afterwards.
pub fn decode_declared_lists(text: &str) -> Result<DeclaredLists, toml::de::Error> {
    let delta: crate::delta::LayerDelta = toml::from_str(text)?;
    Ok(DeclaredLists::from_delta(&delta))
}

/// Applies the isolation rule to an accumulated config after a layer was
/// decoded onto it (decode.go isolateDecodedProviders): when the layer
/// declared `[[providers]]`, the list becomes exactly what the file declared,
/// so no field of a previously accumulated entry survives on it.
pub fn isolate_providers(config: &mut Config, declared: &DeclaredLists) {
    if declared.providers_defined {
        config.providers.clone_from(&declared.providers);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EntrySource, decode_declared_lists, isolate_providers, merge_plugins, merge_providers,
    };
    use crate::model::Config;
    use crate::plugin::{PluginEntry, PluginSource};
    use crate::provider::ProviderEntry;

    fn provider(name: &str, base_url: &str, models: &[&str]) -> ProviderEntry {
        ProviderEntry {
            name: name.to_owned(),
            kind: "openai".to_owned(),
            base_url: base_url.to_owned(),
            models: models.iter().map(|m| (*m).to_owned()).collect(),
            ..ProviderEntry::default()
        }
    }

    fn plugin(name: &str, command: &str) -> PluginEntry {
        PluginEntry {
            name: name.to_owned(),
            command: command.to_owned(),
            ..PluginEntry::default()
        }
    }

    #[test]
    fn a_user_provider_wins_over_a_same_named_project_provider() {
        // load.go mergeTOMLProviders: user-global wins; project entries only
        // fill names the user config does not define.
        let user = vec![provider("gateway", "https://user.example.com", &["m"])];
        let project = vec![
            provider("gateway", "https://project.example.com", &["other"]),
            provider("workspace", "https://ws.example.com", &["w"]),
        ];
        let merged = merge_providers(&[(EntrySource::User, user), (EntrySource::Project, project)]);
        assert_eq!(merged.providers.len(), 2);
        let gateway = merged
            .providers
            .iter()
            .find(|entry| entry.name == "gateway")
            .expect("gateway");
        assert_eq!(gateway.base_url, "https://user.example.com");
        assert_eq!(gateway.models, vec!["m"]);
        assert_eq!(merged.sources.get("gateway"), Some(&EntrySource::User));
        assert_eq!(merged.sources.get("workspace"), Some(&EntrySource::Project));
        // The displaced project entry is kept so a renderer can show it.
        assert_eq!(merged.shadowed_project.len(), 1);
        assert_eq!(
            merged.shadowed_project[0].base_url,
            "https://project.example.com"
        );
    }

    #[test]
    fn a_project_only_provider_lands_in_table_order() {
        let merged = merge_providers(&[
            (EntrySource::User, Vec::new()),
            (
                EntrySource::Project,
                vec![
                    provider("a", "https://a.example.com", &["m"]),
                    provider("b", "https://b.example.com", &["m"]),
                ],
            ),
        ]);
        assert_eq!(
            merged
                .providers
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }

    #[test]
    fn merge_identity_is_the_trimmed_name() {
        // providerMergeKey: TrimSpace(p.Name).
        let merged = merge_providers(&[(
            EntrySource::User,
            vec![
                provider(" gateway ", "https://first.example.com", &["m"]),
                provider("gateway", "https://second.example.com", &["m"]),
            ],
        )]);
        assert_eq!(merged.providers.len(), 1);
        assert_eq!(merged.providers[0].base_url, "https://second.example.com");
    }

    #[test]
    fn plugins_merge_by_name_and_keep_the_earlier_disabled_tools() {
        // load.go mergeTOMLPlugins: later wins, disabled_tools union.
        let mut user_plugin = plugin("memory", "npx -y first");
        user_plugin.disabled_tools = vec!["a".to_owned()];
        let mut project_plugin = plugin("memory", "npx -y second");
        project_plugin.disabled_tools = vec!["b".to_owned()];
        let merged = merge_plugins(&[
            (EntrySource::User, vec![user_plugin]),
            (
                EntrySource::Project,
                vec![project_plugin, plugin("extra", "uvx srv")],
            ),
        ]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].name, "memory");
        // The later declaration wins on the fields it sets.
        assert_eq!(merged[0].command, "npx");
        assert_eq!(merged[0].args, vec!["-y".to_owned(), "second".to_owned()]);
        // Both declarations' hidden tools survive.
        assert_eq!(
            merged[0].disabled_tools,
            vec!["a".to_owned(), "b".to_owned()]
        );
        assert_eq!(merged[1].name, "extra");
    }

    #[test]
    fn a_project_plugin_is_not_dropped_by_a_user_plugin_of_another_name() {
        // The whole reason this merge exists: TOML decoding replaces the list.
        let merged = merge_plugins(&[
            (EntrySource::User, vec![plugin("global", "npx global")]),
            (EntrySource::Project, vec![plugin("local", "npx local")]),
        ]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].name, "global");
        assert_eq!(merged[1].name, "local");
    }

    #[test]
    fn plugin_entries_are_tagged_by_their_config_layer() {
        // load.go: a user file tags UserConfig, every later layer ProjectConfig.
        let merged = merge_plugins(&[
            (EntrySource::User, vec![plugin("global", "npx global")]),
            (EntrySource::Project, vec![plugin("local", "npx local")]),
        ]);
        assert_eq!(merged[0].source, PluginSource::UserConfig);
        assert_eq!(merged[1].source, PluginSource::ProjectConfig);
    }

    #[test]
    fn decoding_declared_lists_carries_only_what_the_file_declares() {
        let declared = decode_declared_lists(
            r#"
[[providers]]
name = "gateway"
kind = "openai"
base_url = "http://localhost:8021/v1"
models = ["my-model"]
api_key_env = "GATEWAY_API_KEY"
"#,
        )
        .expect("decode");
        assert!(declared.providers_defined);
        assert_eq!(declared.providers.len(), 1);
        let entry = &declared.providers[0];
        assert_eq!(entry.name, "gateway");
        assert_eq!(entry.models, vec!["my-model"]);
        assert_eq!(entry.api_key_env, "GATEWAY_API_KEY");
        // Nothing the reference's official default entries carry.
        assert!(entry.balance_url.is_empty());
        assert_eq!(entry.context_window, 0);
        assert!(entry.price.is_none());
    }

    #[test]
    fn positional_decode_isolation_never_inherits_a_neighbour() {
        // provider_isolation_test.go: the first declared entry used to inherit
        // index 0 of the seeded list (balance_url, context_window, prices).
        let mut config = Config {
            providers: vec![
                ProviderEntry {
                    name: "deepseek-flash".to_owned(),
                    kind: "anthropic".to_owned(),
                    base_url: "https://api.deepseek.com/anthropic".to_owned(),
                    model: "deepseek-flash".to_owned(),
                    balance_url: "https://api.deepseek.com/user/balance".to_owned(),
                    context_window: 1_000_000,
                    ..ProviderEntry::default()
                },
                ProviderEntry {
                    name: "deepseek-pro".to_owned(),
                    kind: "anthropic".to_owned(),
                    base_url: "https://api.deepseek.com/anthropic".to_owned(),
                    model: "deepseek-v4-pro".to_owned(),
                    balance_url: "https://api.deepseek.com/user/balance".to_owned(),
                    context_window: 1_000_000,
                    ..ProviderEntry::default()
                },
            ],
            ..Config::default()
        };
        let declared = decode_declared_lists(
            r#"
[[providers]]
name = "gateway"
kind = "openai"
base_url = "http://localhost:8021/v1"
models = ["my-model"]
api_key_env = "GATEWAY_API_KEY"

[[providers]]
name = "second"
kind = "openai"
base_url = "http://localhost:9000/v1"
models = ["m2"]
api_key_env = "SECOND_KEY"
"#,
        )
        .expect("decode");
        isolate_providers(&mut config, &declared);
        assert_eq!(config.providers.len(), 2);
        for entry in &config.providers {
            assert!(
                entry.balance_url.is_empty(),
                "leaked balance_url on {entry:?}"
            );
            assert_eq!(entry.context_window, 0, "leaked window on {entry:?}");
            assert!(entry.price.is_none(), "leaked price on {entry:?}");
            assert!(entry.prices.is_empty(), "leaked prices on {entry:?}");
        }
        assert_eq!(config.providers[0].name, "gateway");
        assert_eq!(config.providers[1].name, "second");
    }

    #[test]
    fn a_file_without_the_key_leaves_the_list_alone() {
        // An absent `[[providers]]` means "no opinion"; an explicit
        // `providers = []` means every provider was removed.
        let mut config = Config {
            providers: vec![provider("keep", "https://keep.example.com", &["m"])],
            ..Config::default()
        };
        let declared = decode_declared_lists("language = \"en\"\n").expect("decode");
        assert!(!declared.providers_defined);
        isolate_providers(&mut config, &declared);
        assert_eq!(config.providers.len(), 1);

        let declared = decode_declared_lists("providers = []\n").expect("decode");
        assert!(declared.providers_defined);
        isolate_providers(&mut config, &declared);
        assert!(config.providers.is_empty());

        let declared = decode_declared_lists("[[providers]]\nname = \"x\"\n").expect("decode");
        assert!(declared.providers_defined);
    }

    #[test]
    fn the_layer_index_decides_the_scope() {
        // loadForRoot: paths[0] is the user layer, every later one a project.
        assert_eq!(EntrySource::for_layer(0), EntrySource::User);
        assert_eq!(EntrySource::for_layer(1), EntrySource::Project);
        assert_eq!(EntrySource::for_layer(9), EntrySource::Project);
    }
}
