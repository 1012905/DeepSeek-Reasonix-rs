//! Effort vocabulary and per-request depth resolution (Go:
//! `openai/effort.go`, `glm_effort.go`, `minimax.go`, `kimi_k3.go`, and
//! `contract/provider/zhipu_effort.go`).

use crate::error::ProviderError;

/// The documented `reasoning_effort` contract for one GLM depth model (Go:
/// `ZhipuEffort`).
#[derive(Debug, Clone, PartialEq)]
pub struct ZhipuEffort {
    /// Depth levels the model accepts, in menu order.
    pub levels: &'static [&'static str],
    /// The level the API applies when the request omits the field.
    pub default: &'static str,
    /// Levels that switch thinking off via `thinking.type=disabled`; empty
    /// means the model cannot disable thinking.
    pub thinking_off: &'static [&'static str],
    /// Where a stored binary `disabled` choice lands.
    pub disabled_to: &'static str,
}

impl ZhipuEffort {
    /// Whether the model runs thinking unconditionally (Go:
    /// `ForcesThinking`).
    pub fn forces_thinking(&self) -> bool {
        self.thinking_off.is_empty()
    }
}

/// Keyed by exact model id; the ladder is per model, not per host (Go:
/// `zhipuEffortContracts`).
pub fn zhipu_effort_contract(model: &str) -> Option<ZhipuEffort> {
    match model.trim().to_ascii_lowercase().as_str() {
        "glm-5.2" => Some(ZhipuEffort {
            levels: &["none", "minimal", "low", "medium", "high", "xhigh", "max"],
            default: "max",
            thinking_off: &["none", "minimal"],
            disabled_to: "none",
        }),
        "glm-5.3" | "glm-5.3-flash" => Some(ZhipuEffort {
            levels: &["low", "high", "max"],
            default: "max",
            thinking_off: &[],
            disabled_to: "low",
        }),
        _ => None,
    }
}

/// Model ids whose effort contract is documented; an unrelated GLM variant
/// must not inherit one by resemblance (Go: `ZhipuDepthModel`).
pub fn zhipu_depth_model(model: &str) -> String {
    let model = model.trim().to_ascii_lowercase();
    if zhipu_effort_contract(&model).is_some() {
        model
    } else {
        String::new()
    }
}

/// The optional vendor-agnostic `thinking` config field: enabled|disabled are
/// kept case-insensitively, everything else is ignored so a typo never breaks
/// a request (Go: `configuredThinkingType`).
pub fn configured_thinking_type(thinking: &str) -> String {
    let t = thinking.trim().to_ascii_lowercase();
    if t == "enabled" || t == "disabled" {
        t
    } else {
        String::new()
    }
}

/// Normalizes `reasoning_protocol`; unknown values fall back to auto (Go:
/// `normalizeReasoningProtocol`).
pub fn normalize_reasoning_protocol(raw: &str) -> String {
    let t = raw.trim().to_ascii_lowercase();
    if matches!(
        t.as_str(),
        "deepseek" | "glm" | "kimi-k3" | "openai" | "none"
    ) {
        t
    } else {
        String::new()
    }
}

const KIMI_K3_EFFORT_VOCABULARY: [&str; 3] = ["low", "high", "max"];

/// Whether the K3 contract applies: explicit protocol, or a kimi-k3 model on
/// Moonshot's direct API (Go: `usesKimiK3Contract`).
pub fn uses_kimi_k3_contract(protocol: &str, base_url: &str, model: &str) -> bool {
    protocol == "kimi-k3"
        || (crate::vendor::is_kimi(base_url) && model.trim().eq_ignore_ascii_case("kimi-k3"))
}

/// (vocabulary, has-explicit-vocabulary) an endpoint's efforts resolve to (Go:
/// `reasoningEffortVocabulary`).
pub fn reasoning_effort_vocabulary(kimi_k3: bool, configured: &[String]) -> (Vec<String>, bool) {
    if kimi_k3 {
        return (
            KIMI_K3_EFFORT_VOCABULARY
                .iter()
                .map(|s| s.to_string())
                .collect(),
            true,
        );
    }
    let explicit = has_explicit_supported_efforts(configured);
    (configured.to_vec(), explicit)
}

/// K3 default depth is max (Go: `kimiK3ReasoningEffort`).
pub fn kimi_k3_reasoning_effort(kimi_k3: bool, effort: &str) -> String {
    if kimi_k3 && effort.is_empty() {
        return "max".to_owned();
    }
    effort.to_owned()
}

/// Facts New resolved that decide which depth levels a per-request override
/// may take (Go: `effortEndpoint`).
#[derive(Debug, Default)]
pub struct EffortEndpoint {
    pub protocol: String,
    pub thinking_type: String,
    pub effort: String,
    pub deepseek: bool,
    pub flash: bool,
    pub minimax: bool,
    pub zhipu: bool,
    pub zhipu_depth: String,
    pub longcat: bool,
    pub ollama_cloud: bool,
    pub explicit: bool,
    pub supported: Vec<String>,
}

/// Depth levels a per-request `EffortOverride` may take; empty means
/// overrides are ignored (Go: `requestEffortVocabulary`). Binary thinking
/// knobs and disabled thinking yield nothing — an override adjusts depth,
/// never whether thinking runs.
pub fn request_effort_vocabulary(e: &EffortEndpoint) -> Vec<String> {
    if e.thinking_type == "disabled" || e.protocol == "none" || e.minimax || e.longcat {
        return Vec::new();
    }
    if e.zhipu {
        return zhipu_request_effort_vocabulary(&e.zhipu_depth, &e.effort);
    }
    let levels: Vec<String> = if e.explicit {
        e.supported.clone()
    } else if e.deepseek {
        let mut levels = vec!["high".to_owned(), "max".to_owned()];
        if e.flash {
            levels.push("low".to_owned());
        }
        levels
    } else if e.ollama_cloud {
        ["low", "medium", "high", "max"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else if !e.effort.is_empty() {
        ["low", "medium", "high"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        Vec::new()
    };
    depth_only(levels)
}

/// Depth vocabulary a Zhipu request may carry (Go:
/// `zhipuRequestEffortVocabulary`); thinking-off levels ride thinking.type.
pub fn zhipu_request_effort_vocabulary(depth: &str, effort: &str) -> Vec<String> {
    let Some(contract) = zhipu_effort_contract(depth) else {
        return Vec::new();
    };
    if supports_effort(contract.thinking_off, effort) {
        return Vec::new();
    }
    let out: Vec<String> = contract
        .levels
        .iter()
        .filter(|level| !supports_effort(contract.thinking_off, level))
        .map(|s| s.to_string())
        .collect();
    depth_only(out)
}

/// Strips thinking on/off toggles from an effort vocabulary (Go:
/// `depthOnly`).
pub fn depth_only(levels: Vec<String>) -> Vec<String> {
    levels
        .into_iter()
        .filter(|level| {
            !matches!(
                level.trim().to_ascii_lowercase().as_str(),
                "" | "auto" | "enabled" | "adaptive" | "disabled" | "none" | "off"
            )
        })
        .collect()
}

/// Whether a level is listed in a vocabulary (case-insensitive, trimmed).
pub fn supports_effort(levels: &[&str], want: &str) -> bool {
    let want = want.trim().to_ascii_lowercase();
    levels
        .iter()
        .any(|level| level.trim().to_ascii_lowercase() == want)
}

pub fn supports_effort_owned(levels: &[String], want: &str) -> bool {
    supports_effort(&levels.iter().map(|s| s.as_str()).collect::<Vec<_>>(), want)
}

/// A meaningful explicit list is the endpoint's declared effort vocabulary;
/// auto remains implicit and is ignored (Go: `hasExplicitSupportedEfforts`).
pub fn has_explicit_supported_efforts(levels: &[String]) -> bool {
    levels.iter().any(|level| {
        let level = level.trim().to_ascii_lowercase();
        !level.is_empty() && level != "auto"
    })
}

/// Resolves one request's reasoning depth: a vocabulary-approved
/// `EffortOverride` wins, anything else falls back to the configured effort
/// (Go: `client.requestEffort`).
pub fn request_effort(
    configured: &str,
    request_efforts: &[String],
    effort_override: &str,
) -> String {
    let want = effort_override.trim().to_ascii_lowercase();
    if want.is_empty() || !supports_effort_owned(request_efforts, &want) {
        return configured.to_owned();
    }
    want
}

/// Boot-time validation for the MiniMax path (Go: `resolveMiniMaxEffort`).
pub fn resolve_minimax_effort(
    name: &str,
    model: &str,
    effort: &str,
    thinking: &str,
) -> Result<String, ProviderError> {
    if crate::vendor::is_minimax_flash(model) {
        return minimax_flash_effort(effort, thinking);
    }
    let effort = effort.trim().to_ascii_lowercase();
    match effort.as_str() {
        "" | "adaptive" | "disabled" => Ok(effort),
        _ => Err(ProviderError::Config {
            message: format!(
                "openai: provider {name:?} uses MiniMax thinking; effort must be adaptive or disabled"
            ),
        }),
    }
}

/// The shared MiniMax-Flash depth contract (Go: `MiniMaxFlashEffort`).
pub fn minimax_flash_effort(effort: &str, thinking: &str) -> Result<String, ProviderError> {
    let effort = effort.trim().to_ascii_lowercase();
    let thinking = thinking.trim().to_ascii_lowercase();
    if matches!(thinking.as_str(), "disabled" | "off" | "none") {
        return Err(ProviderError::Config {
            message:
                "MiniMax-M3.1-Flash cannot disable thinking: effort not supported by this endpoint"
                    .to_owned(),
        });
    }
    if effort.is_empty() || effort == "auto" {
        return Ok("max".to_owned());
    }
    if !crate::vendor::MINIMAX_FLASH_EFFORTS.contains(&effort.as_str()) {
        return Err(ProviderError::Config {
            message: "MiniMax-M3.1-Flash effort must be low, medium, high, xhigh, or max: effort not supported by this endpoint".to_owned(),
        });
    }
    Ok(effort)
}

/// Boot-time validation for the Zhipu path (Go: `resolveZhipuEffort`), folding
/// the legacy binary spellings onto the documented contract.
pub fn resolve_zhipu_effort(
    name: &str,
    model: &str,
    effort: &str,
) -> Result<String, ProviderError> {
    let Some(contract) = zhipu_effort_contract(model) else {
        return match effort {
            "" | "enabled" | "disabled" => Ok(effort.to_owned()),
            _ => Err(ProviderError::Config {
                message: format!(
                    "openai: provider {name:?} uses Zhipu thinking; effort must be enabled or disabled"
                ),
            }),
        };
    };
    match effort {
        "" => Ok(String::new()),
        "enabled" => Ok(contract.default.to_owned()),
        "disabled" => Ok(contract.disabled_to.to_owned()),
        _ => {
            if supports_effort(contract.levels, effort) {
                Ok(effort.to_owned())
            } else {
                Err(ProviderError::Config {
                    message: format!(
                        "openai: provider {name:?}: effort must be {}",
                        contract.levels.join(", ")
                    ),
                })
            }
        }
    }
}

/// Default total output budgets (Go: `AutoOutputBudget`): 16K non-reasoning,
/// 32K ordinary reasoning, 64K high/max reasoning.
pub fn auto_output_budget(reasoning_enabled: bool, effort: &str) -> i64 {
    const DEFAULT_ORDINARY_OUTPUT_TOKENS: i64 = 16 * 1024;
    const DEFAULT_REASONING_OUTPUT_TOKENS: i64 = 32 * 1024;
    const DEFAULT_HIGH_REASONING_OUTPUT_TOKENS: i64 = 64 * 1024;
    if !reasoning_enabled {
        return DEFAULT_ORDINARY_OUTPUT_TOKENS;
    }
    match effort.trim().to_ascii_lowercase().as_str() {
        "high" | "max" => DEFAULT_HIGH_REASONING_OUTPUT_TOKENS,
        _ => DEFAULT_REASONING_OUTPUT_TOKENS,
    }
}
