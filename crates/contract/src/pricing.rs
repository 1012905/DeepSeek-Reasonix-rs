//! Mirrors the `internal/contract/pricing` wire types the event projection carries.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Money {
    pub amount: String,
    pub currency: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostQuote {
    pub original: Money,
    #[serde(
        rename = "originalTotals",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub original_totals: Vec<Money>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub valuations: HashMap<String, Valuation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<Money>,
    #[serde(
        rename = "billingMode",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub billing_mode: String,
    pub estimated: bool,
    #[serde(rename = "costComplete", default)]
    pub cost_complete: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub coverage: String,
    #[serde(rename = "displayComplete", default)]
    pub display_complete: bool,
    pub complete: bool,
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
    #[serde(rename = "modelRef", default, skip_serializing_if = "String::is_empty")]
    pub model_ref: String,
    #[serde(
        rename = "usageSource",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub usage_source: String,
    #[serde(
        rename = "pricingFingerprint",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub pricing_fingerprint: String,
    #[serde(rename = "rateDate", default, skip_serializing_if = "String::is_empty")]
    pub rate_date: String,
    #[serde(
        rename = "incompleteReason",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub incomplete_reason: String,
    #[serde(
        rename = "legacyEstimate",
        default,
        skip_serializing_if = "crate::eventwire::util::is_false"
    )]
    pub legacy_estimate: bool,
    #[serde(
        rename = "catalogSource",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub catalog_source: String,
    #[serde(rename = "rateBand", default, skip_serializing_if = "String::is_empty")]
    pub rate_band: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Valuation {
    pub money: Money,
    pub basis: String,
    pub source: String,
    #[serde(rename = "asOf")]
    pub as_of: String,
    #[serde(
        rename = "rateSnapshot",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub rate_snapshot: Option<RateSnapshot>,
    #[serde(default, skip_serializing_if = "crate::eventwire::util::is_false")]
    pub stale: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateSnapshot {
    pub base: String,
    pub quote: String,
    pub rate: f64,
    pub source: String,
    #[serde(rename = "asOf")]
    pub as_of: String,
    #[serde(default, skip_serializing_if = "crate::eventwire::util::is_false")]
    pub stale: bool,
}
