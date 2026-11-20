//! Token accounting (Go: `contract/provider/usage.go` subset and
//! `openai/openai.go` `normaliseUsage`).

use serde::Deserialize;

/// What a completion cost (Go: `Usage`, provider-accounting subset).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    /// Prompt tokens served from cache.
    pub cache_hit_tokens: i64,
    /// Prompt tokens not cached, including cache writes.
    pub cache_miss_tokens: i64,
    /// Subset of cache misses used to create provider cache entries.
    pub cache_write_tokens: i64,
    /// Cache-write charge in ordinary input-token equivalents.
    pub cache_write_billed_tokens: f64,
    /// Subset of completion tokens spent on chain-of-thought.
    pub reasoning_tokens: i64,
    /// The model's last reported `choices[0].finish_reason`.
    pub finish_reason: String,
    /// Reconstructed counts or partial interrupted-response usage.
    pub estimated: bool,
    /// Provider requests represented by this aggregate; zero means one.
    pub request_count: i64,
}

/// The usage shape providers actually put on the wire: DeepSeek's top-level
/// cache fields, OpenAI/MiMo's nested details, and Anthropic-style fallbacks
/// returned by compatible gateways (Go: `wireUsage`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WireUsage {
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default)]
    pub prompt_cache_hit_tokens: i64,
    #[serde(default)]
    pub prompt_cache_miss_tokens: i64,
    #[serde(default)]
    pub cache_creation_input_tokens: i64,
    #[serde(default)]
    pub cache_read_input_tokens: i64,
    #[serde(default)]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(default)]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct PromptTokensDetails {
    #[serde(default)]
    pub cached_tokens: i64,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct CompletionTokensDetails {
    #[serde(default)]
    pub reasoning_tokens: i64,
}

/// Folds the cache shapes OpenAI-compatible providers use into one [`Usage`]
/// (Go: `normaliseUsage`). PromptTokens is the complete input; a details
/// object with cached=0 is an explicit full miss; a hit above prompt is a
/// broken report clamped to it.
pub fn normalize_usage(u: &WireUsage) -> Usage {
    let mut prompt = u.prompt_tokens;
    let anthropic_prompt = prompt == 0
        && (u.input_tokens != 0
            || u.cache_creation_input_tokens != 0
            || u.cache_read_input_tokens != 0);
    if anthropic_prompt {
        // Anthropic input_tokens excludes cache reads and cache writes;
        // PromptTokens is the complete input.
        prompt = u.input_tokens + u.cache_creation_input_tokens + u.cache_read_input_tokens;
    }
    let mut completion = u.completion_tokens;
    if completion == 0 {
        completion = u.output_tokens;
    }
    let mut total = u.total_tokens;
    if total == 0 && (prompt != 0 || completion != 0) {
        total = prompt + completion;
    }

    let mut hit = u.prompt_cache_hit_tokens;
    if hit == 0 {
        hit = u.prompt_tokens_details.map_or(0, |d| d.cached_tokens);
    }
    if hit == 0 {
        hit = u.cache_read_input_tokens;
    }
    if prompt > 0 && hit > prompt {
        hit = prompt;
    }
    let mut miss = u.prompt_cache_miss_tokens;
    if miss == 0 {
        if anthropic_prompt {
            // Cache writes stay uncached input for pricing and cache-ratio.
            miss = u.input_tokens + u.cache_creation_input_tokens;
        } else if u.prompt_tokens_details.is_some() {
            miss = (prompt - hit).max(0);
        } else if hit > 0 && prompt > hit {
            miss = prompt - hit;
        }
    }
    let reasoning = u
        .completion_tokens_details
        .map_or(0, |d| d.reasoning_tokens);
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        cache_hit_tokens: hit,
        cache_miss_tokens: miss,
        cache_write_tokens: 0,
        cache_write_billed_tokens: 0.0,
        reasoning_tokens: reasoning,
        finish_reason: String::new(),
        estimated: false,
        request_count: 0,
    }
}
