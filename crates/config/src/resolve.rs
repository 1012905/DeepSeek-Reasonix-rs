//! Model resolution and validation (config.go ResolveModel / Validate,
//! docs/contracts/config.md §5).
//!
//! `ResolveModel` accepts three reference forms and hands back a copy resolved
//! onto the selected model, so a vendor with many models yields one instance
//! per model without duplicating `base_url`/`api_key_env`.

use crate::model::Config;
use crate::provider::{ProviderEntry, is_valid_credential_key};

/// A resolved model reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModel {
    /// The provider name the reference resolved through.
    pub provider: String,
    /// The concrete model id.
    pub model: String,
}

impl ResolvedModel {
    /// The canonical `provider/model` reference.
    pub fn reference(&self) -> String {
        format!("{}/{}", self.provider, self.model)
    }
}

impl Config {
    /// The named provider entry, exact-name lookup.
    pub fn provider(&self, name: &str) -> Option<&ProviderEntry> {
        self.providers.iter().find(|entry| entry.name == name)
    }

    /// Resolves a model reference to a provider entry whose `model` is the
    /// selected model. Accepts, in order:
    ///
    /// - `provider/model` — that exact model under that provider;
    /// - a provider name — the provider's default model;
    /// - a bare model name — the first provider that lists it.
    pub fn resolve_model(&self, reference: &str) -> Option<ProviderEntry> {
        // "provider/model": cut at the FIRST slash, like the reference, so a
        // model id carrying slashes stays whole.
        if let Some((provider, model)) = reference.split_once('/')
            && let Some(entry) = self.provider(provider)
            && entry.has_model(model)
        {
            return Some(entry.for_model(model));
        }
        // A provider name resolves to its default model.
        if let Some(entry) = self.provider(reference) {
            let model = entry.default_model();
            return Some(entry.for_model(&model));
        }
        // A bare model name: the first provider that lists it.
        for entry in &self.providers {
            if entry.has_model(reference) {
                return Some(entry.for_model(reference));
            }
        }
        None
    }

    /// The provider names in list order, joined as the validation error names
    /// them.
    pub fn provider_names(&self) -> String {
        self.providers
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Checks that the selected model's provider is usable, with the
    /// reference's four error texts verbatim (config.go Validate).
    ///
    /// `resolved_key` is the credential read for the entry's `api_key_env`;
    /// `credentials_location` is the file a missing key is missing from.
    pub fn validate(
        &self,
        model: &str,
        resolved_key: &str,
        credentials_location: &str,
    ) -> Result<(), ModelError> {
        let Some(entry) = self.resolve_model(model) else {
            return Err(ModelError::Unknown {
                model: model.to_owned(),
                configured: self.provider_names(),
            });
        };
        if let Some(message) = kind_error(&entry, model) {
            return Err(message);
        }
        if entry.base_url.is_empty() {
            return Err(ModelError::BaseUrlRequired {
                model: model.to_owned(),
            });
        }
        if !entry.api_key_env.trim().is_empty() && !is_valid_credential_key(&entry.api_key_env) {
            return Err(ModelError::InvalidCredentialKey {
                model: model.to_owned(),
                key: entry.api_key_env.clone(),
            });
        }
        if entry.requires_api_key() && resolved_key.is_empty() {
            return Err(ModelError::MissingKey {
                model: model.to_owned(),
                key_env: entry.api_key_env.clone(),
                location: credentials_location.to_owned(),
            });
        }
        Ok(())
    }
}

/// The stand-in a config that lists no provider answers with, so the kernel
/// can start and the window can ask for a connection (provider_entry.go).
pub const UNCONFIGURED_NAME: &str = "unconfigured";
pub const UNCONFIGURED_KIND: &str = "unconfigured";
pub const UNCONFIGURED_MODEL: &str = "none";

/// The reserved stand-in reference.
pub fn unconfigured_ref() -> String {
    format!("{UNCONFIGURED_NAME}/{UNCONFIGURED_MODEL}")
}

/// The placeholder entry for a config that lists no provider.
pub fn unconfigured_entry() -> ProviderEntry {
    ProviderEntry {
        name: UNCONFIGURED_NAME.to_owned(),
        kind: UNCONFIGURED_KIND.to_owned(),
        model: UNCONFIGURED_MODEL.to_owned(),
        ..ProviderEntry::default()
    }
}

/// Why a selected entry cannot be used (provider_entry.go kindError).
fn kind_error(entry: &ProviderEntry, model: &str) -> Option<ModelError> {
    match entry.kind.as_str() {
        UNCONFIGURED_KIND => Some(ModelError::NoProvider {
            model: model.to_owned(),
        }),
        "" => Some(ModelError::KindRequired {
            model: model.to_owned(),
        }),
        _ => None,
    }
}

/// The validation refusals, each carrying its identity rather than a sentence
/// a caller would have to parse (REASONIX.md error-identity rule).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ModelError {
    /// No provider lists the reference.
    #[error("unknown model {model:?} (configured: {configured})")]
    Unknown { model: String, configured: String },
    /// The reference resolved to the stand-in: nothing is configured.
    #[error("no model provider is configured; run `reasonix setup` to add one ({model:?})")]
    NoProvider { model: String },
    /// The entry names no wire format.
    #[error("provider {model:?}: kind is required")]
    KindRequired { model: String },
    /// The entry names no endpoint.
    #[error("provider {model:?}: base_url is required")]
    BaseUrlRequired { model: String },
    /// `api_key_env` is not a usable environment variable name.
    #[error(
        "provider {model:?}: api_key_env {key:?} is invalid; use letters, numbers, and underscores, not a model name"
    )]
    InvalidCredentialKey { model: String, key: String },
    /// The provider requires a key and none is stored.
    #[error("provider {model:?}: no stored key for {key_env} in {location}; run 'reasonix setup'")]
    MissingKey {
        model: String,
        key_env: String,
        location: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{Config, ModelError, unconfigured_entry, unconfigured_ref};
    use crate::provider::{ProviderEntry, ProviderModelOverride};

    fn providers() -> Vec<ProviderEntry> {
        vec![
            ProviderEntry {
                name: "prov-a".to_owned(),
                kind: "openai".to_owned(),
                base_url: "https://a.example.com".to_owned(),
                models: vec!["model-a1".to_owned(), "model-a2".to_owned()],
                default: "model-a1".to_owned(),
                api_key_env: "PROV_A_KEY".to_owned(),
                effort: "max".to_owned(),
                ..ProviderEntry::default()
            },
            ProviderEntry {
                name: "prov-b".to_owned(),
                kind: "openai".to_owned(),
                base_url: "https://b.example.com".to_owned(),
                model: "model-b1".to_owned(),
                ..ProviderEntry::default()
            },
        ]
    }

    fn config() -> Config {
        Config {
            providers: providers(),
            ..Config::default()
        }
    }

    #[test]
    fn the_three_reference_forms_resolve() {
        // config.go ResolveModel: provider/model, provider name, bare model.
        let config = config();
        let entry = config
            .resolve_model("prov-a/model-a2")
            .expect("provider/model");
        assert_eq!(entry.name, "prov-a");
        assert_eq!(entry.model, "model-a2");
        // The provider's own fields survive the copy: a single list yields one
        // instance per model without duplicating the endpoint.
        assert_eq!(entry.base_url, "https://a.example.com");
        assert_eq!(entry.api_key_env, "PROV_A_KEY");
        // A provider-wide effort survives resolution (edit_test.go
        // TestResolveModelPreservesProviderEffort).
        assert_eq!(entry.effort, "max");

        let entry = config.resolve_model("prov-b").expect("provider name");
        assert_eq!(entry.name, "prov-b");
        assert_eq!(entry.model, "model-b1");

        let entry = config.resolve_model("model-a1").expect("bare model");
        assert_eq!(entry.name, "prov-a");
        assert_eq!(entry.model, "model-a1");

        // A provider name resolves to its `default`, not merely its first.
        let mut defaulted = config.clone();
        defaulted.providers[0].default = "model-a2".to_owned();
        assert_eq!(
            defaulted.resolve_model("prov-a").map(|e| e.model),
            Some("model-a2".to_owned())
        );
    }

    #[test]
    fn unknown_references_resolve_to_nothing() {
        let config = config();
        assert!(config.resolve_model("").is_none());
        assert!(config.resolve_model("prov-a/missing").is_none());
        assert!(config.resolve_model("missing").is_none());
        // A wrong provider with a known model does not fall through to the
        // bare-model search: the "provider/" prefix claims that provider.
        assert!(config.resolve_model("prov-b/model-a1").is_none());
        // A bare model matches the FIRST provider that lists it.
        let mut shared = config.clone();
        shared.providers[1].models = vec!["model-a1".to_owned()];
        assert_eq!(
            shared.resolve_model("model-a1").map(|e| e.name),
            Some("prov-a".to_owned())
        );
    }

    #[test]
    fn the_resolved_copy_carries_the_per_model_price_and_override() {
        // backfill_test.go TestResolveModelUsesPerModelPricing and
        // edit_test.go TestResolveModelAppliesModelOverrides.
        let config = Config {
            providers: vec![ProviderEntry {
                name: "deepseek".to_owned(),
                kind: "openai".to_owned(),
                base_url: "https://api.deepseek.com".to_owned(),
                models: vec!["flash".to_owned(), "pro".to_owned()],
                default: "flash".to_owned(),
                price: Some(crate::provider::Pricing {
                    output: 9.0,
                    ..Default::default()
                }),
                prices: [
                    (
                        "flash".to_owned(),
                        crate::provider::Pricing {
                            output: 4.0,
                            ..Default::default()
                        },
                    ),
                    (
                        "pro".to_owned(),
                        crate::provider::Pricing {
                            output: 13.5,
                            ..Default::default()
                        },
                    ),
                ]
                .into_iter()
                .collect(),
                context_window: 131_072,
                reasoning_protocol: "openai".to_owned(),
                model_overrides: [(
                    "pro".to_owned(),
                    ProviderModelOverride {
                        context_window: 1_000_000,
                        max_output_tokens: 32_768,
                        ..Default::default()
                    },
                )]
                .into_iter()
                .collect(),
                ..ProviderEntry::default()
            }],
            ..Config::default()
        };
        let pro = config.resolve_model("deepseek/pro").expect("pro resolves");
        assert_eq!(pro.price.as_ref().map(|price| price.output), Some(13.5));
        assert_eq!(pro.context_window, 1_000_000);
        assert_eq!(pro.max_output_tokens, 32_768);
        let flash = config.resolve_model("deepseek").expect("default resolves");
        assert_eq!(flash.model, "flash");
        assert_eq!(flash.price.as_ref().map(|price| price.output), Some(4.0));
        assert_eq!(flash.context_window, 131_072);
        assert_eq!(flash.max_output_tokens, 0);
    }

    #[test]
    fn a_validation_success_needs_only_a_usable_entry() {
        let config = config();
        assert_eq!(config.validate("prov-b/model-b1", "", "/r/.env"), Ok(()));
        assert_eq!(
            config.validate("prov-a/model-a1", "sk-test", "/r/.env"),
            Ok(())
        );
    }

    #[test]
    fn the_unknown_model_error_text_is_verbatim() {
        // config.go Validate: `unknown model %q (configured: %s)`.
        let config = config();
        let error = config.validate("missing/x", "", "/r/.env").unwrap_err();
        assert_eq!(
            error,
            ModelError::Unknown {
                model: "missing/x".to_owned(),
                configured: "prov-a, prov-b".to_owned(),
            }
        );
        assert_eq!(
            error.to_string(),
            "unknown model \"missing/x\" (configured: prov-a, prov-b)"
        );
        let empty = Config::default();
        assert_eq!(
            empty.validate("x", "", "/r/.env").unwrap_err().to_string(),
            "unknown model \"x\" (configured: )"
        );
    }

    #[test]
    fn the_base_url_error_text_is_verbatim() {
        // config.go Validate: `provider %q: base_url is required`.
        let mut config = config();
        config.providers[1].base_url.clear();
        let error = config.validate("prov-b", "", "/r/.env").unwrap_err();
        assert_eq!(
            error.to_string(),
            "provider \"prov-b\": base_url is required"
        );
        assert_eq!(
            error,
            ModelError::BaseUrlRequired {
                model: "prov-b".to_owned()
            }
        );
    }

    #[test]
    fn the_credential_key_error_text_is_verbatim() {
        // configured_test.go TestValidateExplainsInvalidCredentialVariableName
        // and config.go Validate's third message.
        let config = Config {
            providers: vec![ProviderEntry {
                name: "relay".to_owned(),
                kind: "openai".to_owned(),
                base_url: "https://api.example.com/v1".to_owned(),
                model: "grok-4.5".to_owned(),
                api_key_env: "grok-4.5".to_owned(),
                ..ProviderEntry::default()
            }],
            ..Config::default()
        };
        let error = config.validate("relay", "", "/r/.env").unwrap_err();
        assert_eq!(
            error.to_string(),
            "provider \"relay\": api_key_env \"grok-4.5\" is invalid; \
             use letters, numbers, and underscores, not a model name"
        );
        assert_eq!(
            error,
            ModelError::InvalidCredentialKey {
                model: "relay".to_owned(),
                key: "grok-4.5".to_owned(),
            }
        );
    }

    #[test]
    fn the_missing_key_error_text_is_verbatim() {
        // config.go Validate: `provider %q: no stored key for %s in %s; run
        // 'reasonix setup'`.
        let config = config();
        let error = config
            .validate("prov-a/model-a1", "", "/home/u/.reasonix/.env")
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "provider \"prov-a/model-a1\": no stored key for PROV_A_KEY in \
             /home/u/.reasonix/.env; run 'reasonix setup'"
        );
        assert_eq!(
            error,
            ModelError::MissingKey {
                model: "prov-a/model-a1".to_owned(),
                key_env: "PROV_A_KEY".to_owned(),
                location: "/home/u/.reasonix/.env".to_owned(),
            }
        );
    }

    #[test]
    fn a_no_auth_local_provider_validates_without_a_key() {
        // configured_test.go TestValidateAllowsNoAuthProvider.
        let config = Config {
            providers: vec![ProviderEntry {
                name: "local".to_owned(),
                kind: "openai".to_owned(),
                base_url: "http://127.0.0.1:23333/v1".to_owned(),
                models: vec!["model-a".to_owned(), "model-b".to_owned()],
                default: "model-a".to_owned(),
                ..ProviderEntry::default()
            }],
            ..Config::default()
        };
        assert_eq!(config.validate("local/model-b", "", "/r/.env"), Ok(()));
        let mut with_key_env = config.clone();
        with_key_env.providers[0].api_key_env = "LOCAL_API_KEY".to_owned();
        assert_eq!(
            with_key_env.validate("local/model-b", "", "/r/.env"),
            Ok(())
        );
    }

    #[test]
    fn a_kind_error_precedes_the_endpoint_check() {
        // provider_entry.go kindError: the stand-in and an empty kind both
        // refuse before base_url is looked at.
        let config = Config {
            providers: vec![
                ProviderEntry {
                    name: "nc".to_owned(),
                    ..ProviderEntry::default()
                },
                ProviderEntry {
                    name: "empty-kind".to_owned(),
                    base_url: "https://x.example.com".to_owned(),
                    model: "m".to_owned(),
                    ..ProviderEntry::default()
                },
            ],
            ..Config::default()
        };
        assert!(matches!(
            config.validate("empty-kind", "", "/r/.env").unwrap_err(),
            ModelError::KindRequired { .. }
        ));
        // A bare provider whose kind is unconfigured is the stand-in identity.
        let mut stand_in = config;
        stand_in.providers[0].kind = "unconfigured".to_owned();
        stand_in.providers[0].model = "none".to_owned();
        assert!(matches!(
            stand_in.validate("nc", "", "/r/.env").unwrap_err(),
            ModelError::NoProvider { .. }
        ));
    }

    #[test]
    fn the_stand_in_reference_is_the_placeholder_identity() {
        // provider_entry.go UnconfiguredRef = "unconfigured/none".
        assert_eq!(unconfigured_ref(), "unconfigured/none");
        let entry = unconfigured_entry();
        assert_eq!(entry.name, "unconfigured");
        assert_eq!(entry.kind, "unconfigured");
        assert_eq!(entry.model, "none");
    }
}
