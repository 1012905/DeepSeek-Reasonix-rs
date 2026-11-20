//! Client configuration and factory resolution (Go: `openai/openai.go`
//! `New`, `cleanCustomHeaders`, `cleanExtraBody`, and the idle-timeout
//! clamps in `contract/provider/streaminterrupt.go`).

use std::collections::BTreeMap;
use std::time::Duration;

use crate::error::ProviderError;
use crate::request::Dialect;
use crate::vendor;
use crate::{client::Client, effort};

/// How long a model call may send nothing before it is read as dropped (Go:
/// `StreamIdleTimeout`).
pub const DEFAULT_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// Bounds for a provider's `idle_timeout_seconds` override (Go:
/// `MinIdleTimeoutSeconds`/`MaxIdleTimeoutSeconds`).
pub const MIN_IDLE_TIMEOUT_SECONDS: u64 = 1;
pub const MAX_IDLE_TIMEOUT_SECONDS: u64 = 32767;

/// Config Extra keys this client reads (Go: `IdleTimeoutSecondsKey`).
pub const IDLE_TIMEOUT_SECONDS_KEY: &str = "idle_timeout_seconds";

/// A resolved provider configuration (Go: `provider.Config` for the openai
/// kind). Plain struct; the caller owns how it is filled.
#[derive(Debug, Clone, Default)]
pub struct ClientConfig {
    /// Provider instance name surfaced in errors.
    pub name: String,
    /// Base endpoint, e.g. `https://api.deepseek.com/v1`.
    pub base_url: String,
    /// Model id.
    pub model: String,
    /// Resolved API key; empty means none is sent.
    pub api_key: String,
    /// api_key_env name, surfaced in auth errors.
    pub key_env: String,
    /// Human-readable source of key_env, surfaced in auth errors.
    pub key_source: String,
    /// Configured reasoning depth; empty/auto means the provider default.
    pub effort: String,
    /// Explicit reasoning protocol (deepseek|glm|kimi-k3|openai|none).
    pub reasoning_protocol: String,
    /// Explicit `thinking` config override (enabled|disabled).
    pub thinking: String,
    /// The endpoint's declared effort vocabulary, when it declares one.
    pub supported_efforts: Vec<String>,
    /// Custom headers; reserved ones are dropped at build time.
    pub headers: BTreeMap<String, String>,
    /// Extra top-level request body fields (Go: `extra_body`).
    pub extra_body: serde_json::Map<String, serde_json::Value>,
    /// The model accepts image input.
    pub vision: bool,
    /// image_url detail hint (low|high; original on official DeepSeek).
    pub vision_detail: String,
    /// Resolved total output budget; 0 means automatic, <0 disables.
    pub max_output_tokens: i64,
    /// Per-provider stream idle override in seconds; 0 keeps the default.
    pub idle_timeout_seconds: u64,
    /// Legacy endpoint override (Go: `chat_url`).
    pub chat_url: String,
    /// Exact POST target override (Go: `request_url`).
    pub request_url: String,
}

impl ClientConfig {
    /// A config with the three required identities filled.
    pub fn new(name: &str, base_url: &str, model: &str) -> ClientConfig {
        ClientConfig {
            name: name.to_owned(),
            base_url: base_url.to_owned(),
            model: model.to_owned(),
            ..ClientConfig::default()
        }
    }
}

/// Reserved custom headers a config cannot override (Go:
/// `reservedCustomHeader`).
fn reserved_custom_header(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "authorization" | "content-type" | "accept" | "host"
    )
}

/// Reserved request-body fields extra_body cannot override (Go:
/// `reservedExtraBodyField`).
fn reserved_extra_body_field(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "model"
            | "messages"
            | "tools"
            | "stream"
            | "stream_options"
            | "temperature"
            | "max_tokens"
            | "max_completion_tokens"
            | "max_output_tokens"
            | "reasoning_effort"
            | "thinking"
    )
}

/// Trims and drops empty/reserved header entries (Go: `cleanCustomHeaders`).
pub fn clean_custom_headers(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(raw_name, raw_value)| {
            let name = raw_name.trim();
            let value = raw_value.trim();
            if name.is_empty() || value.is_empty() || reserved_custom_header(name) {
                return None;
            }
            Some((name.to_owned(), value.to_owned()))
        })
        .collect()
}

/// Trims and drops empty/reserved extra-body entries (Go: `cleanExtraBody`).
pub fn clean_extra_body(
    extra: &serde_json::Map<String, serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    extra
        .iter()
        .filter(|(name, _)| {
            let name = name.trim();
            !name.is_empty() && !reserved_extra_body_field(name)
        })
        .map(|(name, value)| (name.trim().to_owned(), value.clone()))
        .collect()
}

/// Whether an image_url detail hint reaches the wire (Go:
/// `detailAccepted`); "original" is DeepSeek's fourth level only.
fn detail_accepted(detail: &str, official_deepseek: bool) -> bool {
    match detail {
        "low" | "high" => true,
        "original" => official_deepseek,
        _ => false,
    }
}

/// Builds a [`Client`] from a resolved config (Go: `New`).
pub fn build_client(cfg: &ClientConfig) -> Result<Client, ProviderError> {
    if cfg.base_url.is_empty() {
        return Err(ProviderError::Config {
            message: format!("openai: base_url is required for provider {:?}", cfg.name),
        });
    }
    if cfg.model.is_empty() {
        return Err(ProviderError::Config {
            message: format!("openai: model is required for provider {:?}", cfg.name),
        });
    }
    let name = if cfg.name.is_empty() {
        "openai".to_owned()
    } else {
        cfg.name.clone()
    };
    let mut effort_value = cfg.effort.trim().to_ascii_lowercase();
    if effort_value == "auto" {
        effort_value = String::new();
    }
    let protocol = effort::normalize_reasoning_protocol(&cfg.reasoning_protocol);
    let kimi_k3 = effort::uses_kimi_k3_contract(&protocol, &cfg.base_url, &cfg.model);
    let (supported_efforts, has_explicit_efforts) =
        effort::reasoning_effort_vocabulary(kimi_k3, &cfg.supported_efforts);
    let chat_url = vendor::resolve_chat_url(&cfg.base_url, &cfg.request_url, &cfg.chat_url);
    let prefix_chat_url = vendor::deepseek_prefix_chat_url(&chat_url);
    let headers = clean_custom_headers(&cfg.headers);
    let extra_body = clean_extra_body(&cfg.extra_body);
    let model = cfg.model.trim().trim_start_matches("models/").to_owned();
    let official_deepseek = vendor::is_deepseek(&cfg.base_url);
    let vision = cfg.vision && (!official_deepseek || vendor::deepseek_takes_images(&cfg.model));
    let mut vision_detail = cfg.vision_detail.trim().to_ascii_lowercase();
    if !detail_accepted(&vision_detail, official_deepseek) {
        vision_detail = String::new();
    }
    let deepseek = protocol == "deepseek" || (protocol.is_empty() && official_deepseek);
    let mut max_output_tokens = cfg.max_output_tokens;
    let deepseek_v4_flash = model.trim().eq_ignore_ascii_case("deepseek-v4-flash");
    let minimax = protocol.is_empty() && vendor::is_minimax(&cfg.base_url);
    let zhipu = protocol == "glm" || (protocol.is_empty() && vendor::is_zhipu(&cfg.base_url));
    let zhipu_depth = if zhipu {
        effort::zhipu_depth_model(&cfg.model)
    } else {
        String::new()
    };
    let longcat = protocol.is_empty() && vendor::is_longcat(&cfg.base_url);
    let ollama_cloud = protocol.is_empty() && vendor::is_ollama_cloud(&cfg.base_url);
    let mut thinking_type = effort::configured_thinking_type(&cfg.thinking);
    match () {
        _ if protocol == "none" => effort_value = String::new(),
        _ if deepseek => {
            if thinking_type == "disabled" {
                effort_value = String::new();
            } else {
                match effort_value.as_str() {
                    // "off" is a retired level (disabled thinking); fall back
                    // to the default depth.
                    "" | "off" => effort_value = "high".to_owned(),
                    "disabled" => {
                        if has_explicit_efforts
                            && !effort::supports_effort_owned(&supported_efforts, "disabled")
                        {
                            return Err(effort_refused(&name, &effort_value, &supported_efforts));
                        }
                        // DeepSeek can turn thinking off too; route through
                        // thinking.type and drop the depth hint.
                        effort_value = String::new();
                        thinking_type = "disabled".to_owned();
                    }
                    _ => {
                        if has_explicit_efforts {
                            if !effort::supports_effort_owned(&supported_efforts, &effort_value) {
                                return Err(effort_refused(
                                    &name,
                                    &effort_value,
                                    &supported_efforts,
                                ));
                            }
                        } else {
                            match effort_value.as_str() {
                                "low" => {
                                    if !deepseek_v4_flash {
                                        return Err(ProviderError::Config {
                                            message: format!(
                                                "openai: provider {name:?} uses DeepSeek thinking; \
                                                 effort low requires deepseek-v4-flash or explicit \
                                                 supported_efforts"
                                            ),
                                        });
                                    }
                                }
                                "high" | "max" => {}
                                _ => {
                                    return Err(ProviderError::Config {
                                        message: format!(
                                            "openai: provider {name:?} uses DeepSeek thinking; \
                                             effort must be low, high, max, or disabled"
                                        ),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        _ if minimax => {
            effort_value =
                effort::resolve_minimax_effort(&name, &cfg.model, &effort_value, &cfg.thinking)?;
        }
        _ if zhipu => {
            effort_value = effort::resolve_zhipu_effort(&name, &zhipu_depth, &effort_value)?;
        }
        _ if longcat => {
            if !matches!(effort_value.as_str(), "" | "enabled" | "disabled") {
                return Err(ProviderError::Config {
                    message: format!(
                        "openai: provider {name:?} uses LongCat thinking; effort must be enabled or disabled"
                    ),
                });
            }
        }
        _ if ollama_cloud => match effort_value.as_str() {
            "" | "none" | "disabled" | "off" => effort_value = String::new(),
            "xhigh" | "max" => effort_value = "max".to_owned(),
            "low" | "medium" | "high" => {}
            _ => {
                return Err(ProviderError::Config {
                    message: format!(
                        "openai: provider {name:?} uses Ollama Cloud thinking; effort must be \
                         none, low, medium, high, or max"
                    ),
                });
            }
        },
        _ if !effort_value.is_empty() => {
            if has_explicit_efforts {
                if !effort::supports_effort_owned(&supported_efforts, &effort_value) {
                    return Err(effort_refused(&name, &effort_value, &supported_efforts));
                }
            } else {
                match effort_value.as_str() {
                    // max remains clamped to the OpenAI ceiling because MiMo
                    // and similar backends reject it.
                    "max" => effort_value = "high".to_owned(),
                    "low" | "medium" | "high" => {}
                    _ => {
                        return Err(ProviderError::Config {
                            message: format!(
                                "openai: provider {name:?}: effort must be low, medium, or high: \
                                 effort not supported by this endpoint"
                            ),
                        });
                    }
                }
            }
        }
        _ => {}
    }
    let mut request_efforts = effort::request_effort_vocabulary(&effort::EffortEndpoint {
        protocol: protocol.clone(),
        thinking_type: thinking_type.clone(),
        effort: effort_value.clone(),
        deepseek,
        flash: deepseek_v4_flash,
        minimax,
        zhipu,
        zhipu_depth: zhipu_depth.clone(),
        longcat,
        ollama_cloud,
        explicit: has_explicit_efforts,
        supported: supported_efforts.clone(),
    });
    if minimax && vendor::is_minimax_flash(&cfg.model) {
        request_efforts = vendor::MINIMAX_FLASH_EFFORTS
            .iter()
            .map(|s| s.to_string())
            .collect();
    }
    // max_output_tokens=0 means automatic. DeepSeek reasoning uses 32K /
    // high-max 64K; thinking-disabled stays ordinary 16K.
    let auto_max_output = max_output_tokens == 0 && official_deepseek;
    if auto_max_output {
        let reasoning_on = thinking_type != "disabled"
            && effort_value != "disabled"
            && effort_value != "off"
            && effort_value != "none";
        max_output_tokens = effort::auto_output_budget(reasoning_on, &effort_value);
    }
    let dialect = Dialect {
        name: name.clone(),
        model,
        base_url: cfg.base_url.trim_end_matches('/').to_owned(),
        deepseek,
        minimax,
        zhipu,
        zhipu_depth,
        longcat,
        kimi_k3,
        thinking_type: thinking_type.clone(),
        effort: effort_value.clone(),
        request_efforts,
        vision,
        vision_detail,
        max_output_tokens,
        auto_max_output,
        extra_body,
        open_code_go: vendor::is_opencode_go_endpoint(&chat_url),
    };
    let transport = crate::transport::Transport::new()?;
    Ok(Client::new(
        dialect,
        chat_url,
        prefix_chat_url,
        headers,
        cfg.api_key.trim().to_owned(),
        cfg.key_env.clone(),
        cfg.key_source.clone(),
        idle_timeout(cfg.idle_timeout_seconds),
        transport,
    ))
}

fn effort_refused(name: &str, effort: &str, supported: &[String]) -> ProviderError {
    ProviderError::Config {
        message: format!(
            "openai: provider {name:?}: effort {effort:?} is not listed in supported_efforts: \
             {supported:?}: effort not supported by this endpoint"
        ),
    }
}

/// Resolves the per-provider stream idle timeout (Go:
/// `IdleTimeoutFromExtra`): unset and non-positive keep the default; a value
/// past the ceiling clamps to it.
pub(crate) fn idle_timeout(seconds: u64) -> Duration {
    if seconds == 0 {
        return DEFAULT_STREAM_IDLE_TIMEOUT;
    }
    Duration::from_secs(seconds.clamp(MIN_IDLE_TIMEOUT_SECONDS, MAX_IDLE_TIMEOUT_SECONDS))
}
