//! `[[providers]]`: the provider entry, its model list, and model resolution.
//!
//! `ResolveModel` accepts the reference's three forms — `provider/model`, a
//! provider name, and a bare model name — and `Validate` answers the
//! reference's four error texts verbatim (docs/contracts/config.md §5).

use std::collections::BTreeMap;

use serde::Deserialize;

/// One provider instance (provider_entry.go). Keys this crate does not model
/// decode through untouched, like every other table.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ProviderEntry {
    /// Identity refs point at.
    pub name: String,
    /// What the UI calls this entry; `name` stays the identity.
    pub display_name: String,
    /// Wire format: openai|anthropic|responses|gemini|typesafe.
    pub kind: String,
    pub base_url: String,
    pub chat_url: String,
    pub request_url: String,
    /// A single model, back-compat with one-model entries.
    pub model: String,
    /// A vendor's model list behind one address and key.
    pub models: Vec<String>,
    pub model_aliases: BTreeMap<String, String>,
    pub hide_model_prefix: bool,
    pub models_url: String,
    /// Default model when `models` is set, else the first of them.
    pub default: String,
    pub api_key_env: String,
    pub preset_id: String,
    pub preset_version: i64,
    pub headers: BTreeMap<String, String>,
    pub extra_body: BTreeMap<String, toml::Value>,
    pub auth_header: bool,
    pub responses_mode: String,
    pub responses_stateful: Option<bool>,
    pub balance_format: String,
    pub balance_url: String,
    pub context_window: i64,
    pub max_output_tokens: i64,
    pub perseveration_retries: Option<i64>,
    pub price: Option<Pricing>,
    pub prices: BTreeMap<String, Pricing>,
    pub billing_currency: String,
    pub billing_mode: String,
    pub thinking: String,
    pub effort: String,
    pub vision: bool,
    pub vision_models: Vec<String>,
    pub video_models: Vec<String>,
    pub video_protocol: String,
    pub vision_detail: String,
    /// Provider-executed web search; None keeps the built-in default.
    pub web_search: Option<bool>,
    pub reasoning_protocol: String,
    pub supported_efforts: Vec<String>,
    pub default_effort: String,
    pub model_overrides: BTreeMap<String, ProviderModelOverride>,
    pub no_proxy: bool,
    pub cache_ttl_minutes: i64,
    pub idle_timeout_seconds: i64,
}

/// The per-1M-token list price of one model (provider.Pricing). The rates
/// themselves live in `reasonix_contract::pricing`, never here.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Pricing {
    pub cache_hit: f64,
    pub input: f64,
    pub output: f64,
    pub currency: String,
    pub source_url: String,
    pub checked: String,
}

/// Capability metadata one model declares on a gateway serving several
/// vendors' models behind one address (ProviderModelOverride).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ProviderModelOverride {
    pub reasoning_protocol: String,
    pub supported_efforts: Vec<String>,
    pub default_effort: String,
    pub vision: Option<bool>,
    pub context_window: i64,
    pub max_output_tokens: i64,
}

impl ProviderEntry {
    /// The models this provider exposes: the explicit list, else the single
    /// `model` as a one-element list (provider_entry.go ModelList).
    pub fn model_list(&self) -> Vec<String> {
        if !self.models.is_empty() {
            return self.models.clone();
        }
        if !self.model.is_empty() {
            return vec![self.model.clone()];
        }
        Vec::new()
    }

    /// The provider's default model: the explicit `default`, else the first of
    /// the model list.
    pub fn default_model(&self) -> String {
        if !self.default.is_empty() {
            return self.default.clone();
        }
        self.model_list().into_iter().next().unwrap_or_default()
    }

    /// Whether `model` is one of the provider's models.
    pub fn has_model(&self, model: &str) -> bool {
        self.model_list().iter().any(|listed| listed == model)
    }

    /// The per-model price, else the legacy provider-wide price
    /// (provider_entry_methods.go PriceForModel).
    pub fn price_for_model(&self, model: &str) -> Option<Pricing> {
        if let Some(price) = self.prices.get(model.trim()) {
            return Some(price.clone());
        }
        self.price.clone()
    }

    /// This entry resolved onto one of its models: the model's price and its
    /// overrides applied (model_effort.go forModel).
    pub fn for_model(&self, model: &str) -> ProviderEntry {
        let mut entry = self.clone();
        entry.model = model.to_owned();
        entry.price = entry.price_for_model(model);
        entry.apply_model_override();
        entry
    }

    fn apply_model_override(&mut self) {
        let Some(over) = self.model_override_for(&self.model) else {
            return;
        };
        if !over.reasoning_protocol.is_empty() {
            self.reasoning_protocol = over.reasoning_protocol.clone();
        }
        if !over.supported_efforts.is_empty() {
            self.supported_efforts = over.supported_efforts.clone();
        }
        if !over.default_effort.is_empty() || !over.supported_efforts.is_empty() {
            self.default_effort = over.default_effort.clone();
        }
        if over.context_window > 0 {
            self.context_window = over.context_window;
        }
        if over.max_output_tokens != 0 {
            self.max_output_tokens = over.max_output_tokens;
        }
    }

    fn model_override_for(&self, model: &str) -> Option<ProviderModelOverride> {
        let key = self.model_override_key(model)?;
        self.model_overrides.get(key).cloned()
    }

    /// The stored key for `model`: an exact match, else a case-insensitive one
    /// (model_effort.go modelOverrideKey).
    fn model_override_key(&self, model: &str) -> Option<&str> {
        let model = model.trim();
        if model.is_empty() {
            return None;
        }
        if let Some((key, _)) = self.model_overrides.get_key_value(model) {
            return Some(key.as_str());
        }
        self.model_overrides
            .keys()
            .find(|key| key.trim().eq_ignore_ascii_case(model))
            .map(String::as_str)
    }

    /// Whether selecting this provider needs a key: an entry naming no
    /// `api_key_env` is an intentional no-auth provider, and a loopback or
    /// private endpoint may run without one (provider_entry_methods.go
    /// RequiresAPIKey).
    pub fn requires_api_key(&self) -> bool {
        if self.api_key_env.trim().is_empty() {
            return provider_base_url_requires_api_key(&self.base_url);
        }
        !provider_base_url_allows_missing_api_key(&self.base_url)
    }

    /// Whether the provider is selectable: no key needed, or one resolved.
    /// `resolved_key` is the credential the caller read for `api_key_env`.
    pub fn configured(&self, resolved_key: &str) -> bool {
        !self.requires_api_key() || !resolved_key.is_empty()
    }
}

/// Hosts that need a key even with no `api_key_env` declared.
fn provider_base_url_requires_api_key(raw: &str) -> bool {
    matches!(
        official_provider_host(raw).as_str(),
        "api.deepseek.com"
            | "api.xiaomimimo.com"
            | "token-plan-cn.xiaomimimo.com"
            | "api.minimaxi.com"
            | "api.openai.com"
    )
}

/// Loopback, private, and link-local endpoints run without a key: local
/// gateways often keep a legacy `api_key_env` while accepting anonymous
/// requests (provider_entry_methods.go).
fn provider_base_url_allows_missing_api_key(raw: &str) -> bool {
    let raw = raw.trim();
    let Some((_, after_scheme)) = raw.split_once("://") else {
        return false;
    };
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) => host,
        _ => authority,
    };
    let host = host
        .trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(addr) => match addr {
            std::net::IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
            std::net::IpAddr::V6(v6) => {
                // An IPv4-mapped address answers as the address it maps.
                if let Some(v4) = v6.to_ipv4_mapped() {
                    return v4.is_loopback() || v4.is_private() || v4.is_link_local();
                }
                v6.is_loopback() || v6.is_unique_local() || (v6.segments()[0] & 0xffc0) == 0xfe80
            }
        },
        Err(_) => false,
    }
}

/// The lowercased host of a base URL, empty when it does not parse
/// (load.go officialProviderHost).
pub fn official_provider_host(base_url: &str) -> String {
    let raw = base_url.trim();
    let Some((_, after_scheme)) = raw.split_once("://") else {
        return String::new();
    };
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = match authority.rsplit_once(':') {
        Some((host, _)) => host,
        None => authority,
    };
    host.trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase()
}

/// Whether `key` can be stored in the credentials dotenv and exported as an
/// environment variable (credentials.go isCredentialKey): underscores,
/// letters anywhere, and digits from the second character on.
pub fn is_valid_credential_key(key: &str) -> bool {
    let key = key.trim();
    if key.is_empty() {
        return false;
    }
    key.chars().enumerate().all(|(index, ch)| {
        ch == '_' || ch.is_ascii_alphabetic() || (index > 0 && ch.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::{ProviderEntry, is_valid_credential_key, official_provider_host};

    fn provider(name: &str, models: &[&str], default: &str) -> ProviderEntry {
        ProviderEntry {
            name: name.to_owned(),
            kind: "openai".to_owned(),
            base_url: "https://example.com/v1".to_owned(),
            models: models.iter().map(|m| (*m).to_owned()).collect(),
            default: default.to_owned(),
            ..ProviderEntry::default()
        }
    }

    #[test]
    fn model_list_falls_back_to_the_single_model() {
        // provider_entry.go ModelList: `models` wins, else `model` as one.
        let mut entry = provider("p", &[], "");
        assert!(entry.model_list().is_empty());
        assert_eq!(entry.default_model(), "");
        entry.model = "only".to_owned();
        assert_eq!(entry.model_list(), vec!["only".to_owned()]);
        assert_eq!(entry.default_model(), "only");
        entry.models = vec!["a".to_owned(), "b".to_owned()];
        assert_eq!(entry.model_list(), vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(entry.default_model(), "a");
        entry.default = "b".to_owned();
        assert_eq!(entry.default_model(), "b");
    }

    #[test]
    fn a_provider_without_a_key_env_is_configured_unless_the_host_demands_one() {
        // configured_test.go: no api_key_env is an explicit no-auth provider;
        // the five vendor hosts still require one.
        let mut entry = provider("local", &["m"], "m");
        entry.base_url = "http://127.0.0.1:23333/v1".to_owned();
        assert!(!entry.requires_api_key());
        assert!(entry.configured(""));
        entry.api_key_env = "LOCAL_API_KEY".to_owned();
        assert!(!entry.requires_api_key());
        assert!(entry.configured(""));
        entry.base_url = "https://api.deepseek.com".to_owned();
        assert!(entry.requires_api_key());
        assert!(!entry.configured(""));
        assert!(entry.configured("sk-test"));
    }

    #[test]
    fn private_and_link_local_endpoints_need_no_key() {
        for base in [
            "http://localhost:8080/v1",
            "http://app.localhost/v1",
            "http://10.1.2.3/v1",
            "http://172.16.0.9/v1",
            "http://192.168.1.2/v1",
            "http://169.254.1.1/v1",
            "http://[::1]:8080/v1",
            "http://[fd00::1]/v1",
            "http://[fe80::1]/v1",
            "http://[::ffff:127.0.0.1]/v1",
        ] {
            let mut entry = provider("local", &["m"], "m");
            entry.base_url = base.to_owned();
            entry.api_key_env = "SOME_API_KEY".to_owned();
            assert!(!entry.requires_api_key(), "base_url = {base}");
        }
        let mut entry = provider("remote", &["m"], "m");
        entry.base_url = "https://api.example.com/v1".to_owned();
        entry.api_key_env = "SOME_API_KEY".to_owned();
        assert!(entry.requires_api_key());
    }

    #[test]
    fn credential_keys_follow_the_dotenv_rule() {
        // credentials.go isCredentialKey: digits only from index 1.
        for key in ["A", "AB", "A_1", "_A", "A1B2", "deepseek_API_KEY", "A1"] {
            assert!(is_valid_credential_key(key), "key = {key:?}");
        }
        for key in ["", " ", "1A", "A-B", "A.B", "A B", "A$", "A/1", "-A"] {
            assert!(
                !is_valid_credential_key(key),
                "key = {key:?} must be refused"
            );
        }
        // Surrounding whitespace is trimmed before the check.
        assert!(is_valid_credential_key("  KEY  "));
    }

    #[test]
    fn the_official_host_reads_the_url_authority() {
        assert_eq!(
            official_provider_host("https://api.deepseek.com/v1"),
            "api.deepseek.com"
        );
        assert_eq!(
            official_provider_host("  HTTPS://API.DeepSeek.com  "),
            "api.deepseek.com"
        );
        assert_eq!(
            official_provider_host("http://127.0.0.1:23333/v1"),
            "127.0.0.1"
        );
        assert_eq!(official_provider_host("not a url"), "");
    }
}
