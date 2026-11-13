//! Usage telemetry and cache diagnostics, mirroring `eventwire/wire.go`.

use crate::eventwire::util;
use serde::{Deserialize, Serialize};

/// Provider usage telemetry. The token totals and session-cumulative cache
/// counts always arrive; the rest are optional.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    #[serde(rename = "promptTokens", default)]
    pub prompt_tokens: i64,
    #[serde(rename = "completionTokens", default)]
    pub completion_tokens: i64,
    #[serde(rename = "totalTokens", default)]
    pub total_tokens: i64,
    #[serde(rename = "cacheHitTokens", default)]
    pub cache_hit_tokens: i64,
    #[serde(rename = "cacheMissTokens", default)]
    pub cache_miss_tokens: i64,
    #[serde(
        rename = "reasoningTokens",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub reasoning_tokens: i64,
    #[serde(default, skip_serializing_if = "util::is_false")]
    pub estimated: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    #[serde(
        rename = "attemptId",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub attempt_id: String,
    #[serde(
        rename = "cacheDiagnostics",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub cache_diagnostics: Option<CacheDiagnostics>,
    #[serde(rename = "sessionCacheHitTokens", default)]
    pub session_cache_hit_tokens: i64,
    #[serde(rename = "sessionCacheMissTokens", default)]
    pub session_cache_miss_tokens: i64,
    #[serde(
        rename = "contextPromptTokens",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub context_prompt_tokens: i64,
    #[serde(
        rename = "contextCompletionTokens",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub context_completion_tokens: i64,
    #[serde(
        rename = "contextReasoningTokens",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub context_reasoning_tokens: i64,
    #[serde(
        rename = "contextCacheHitTokens",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub context_cache_hit_tokens: i64,
    #[serde(
        rename = "contextCacheMissTokens",
        default,
        skip_serializing_if = "util::is_zero"
    )]
    pub context_cache_miss_tokens: i64,
    #[serde(default, skip_serializing_if = "util::is_zero")]
    pub cost: f64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub currency: String,
    #[serde(
        rename = "currencyCode",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub currency_code: String,
    #[serde(rename = "costUsd", default, skip_serializing_if = "util::is_zero")]
    pub cost_usd: f64,
    #[serde(rename = "costQuote", default, skip_serializing_if = "Option::is_none")]
    pub cost_quote: Option<crate::pricing::CostQuote>,
    #[serde(
        rename = "costComplete",
        default,
        skip_serializing_if = "util::is_false"
    )]
    pub cost_complete: bool,
    #[serde(
        rename = "displayComplete",
        default,
        skip_serializing_if = "util::is_false"
    )]
    pub display_complete: bool,
    #[serde(
        rename = "displayStatus",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub display_status: String,
    #[serde(
        rename = "aggregateMode",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub aggregate_mode: String,
    #[serde(
        rename = "originalTotals",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub original_totals: Vec<crate::pricing::Money>,
}

/// Cache prefix diagnostics. Only `prefixChangeReasons` is omitempty on the
/// wire; the rest always arrive.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheDiagnostics {
    #[serde(rename = "prefixHash", default)]
    pub prefix_hash: String,
    #[serde(rename = "prefixChanged", default)]
    pub prefix_changed: bool,
    #[serde(
        rename = "prefixChangeReasons",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub prefix_change_reasons: Vec<String>,
    #[serde(rename = "systemHash", default)]
    pub system_hash: String,
    #[serde(rename = "toolsHash", default)]
    pub tools_hash: String,
    #[serde(rename = "logRewriteVersion", default)]
    pub log_rewrite_version: i64,
    #[serde(rename = "toolSchemaTokens", default)]
    pub tool_schema_tokens: i64,
    #[serde(rename = "cacheMissTokens", default)]
    pub cache_miss_tokens: i64,
    #[serde(rename = "cacheHitTokens", default)]
    pub cache_hit_tokens: i64,
    #[serde(rename = "carriedMessages", default)]
    pub carried_messages: i64,
    #[serde(rename = "bodyChanged", default)]
    pub body_changed: bool,
    #[serde(rename = "bodyHash", default)]
    pub body_hash: String,
}
