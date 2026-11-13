//! The extension UI surfaces, mirroring `eventwire/extension.go`. Kept beside
//! each other rather than in one file: a new surface kind touches every one of
//! them.

use crate::eventwire::util;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionSurface {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub code: String,
    #[serde(rename = "pluginId", default)]
    pub plugin_id: String,
    #[serde(rename = "surfaceId", default)]
    pub surface_id: String,
    #[serde(
        rename = "sessionId",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub session_id: String,
    #[serde(default, skip_serializing_if = "util::is_zero")]
    pub generation: u64,
    #[serde(default)]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ExtensionStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<ExtensionCard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form: Option<ExtensionForm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification: Option<ExtensionNotification>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel: Option<ExtensionPanel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ExtensionView>,
}

/// A view is composed rather than filled in: the extension sends a tree of
/// primitives and this side renders them with its own components.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionView {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub slot: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub anchor: String,
    #[serde(default)]
    pub body: Vec<ExtensionViewNode>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionViewNode {
    #[serde(default)]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub value: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tone: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    #[serde(rename = "actionId", default, skip_serializing_if = "String::is_empty")]
    pub action_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ExtensionViewNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<crate::surfacespec::Field>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<crate::surfacespec::Diff>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset: Option<crate::surfacespec::Dataset>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionStatus {
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub severity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionKeyValue {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionActionRef {
    #[serde(rename = "actionId", default)]
    pub action_id: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionCard {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub markdown: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<ExtensionKeyValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ExtensionActionRef>,
}

/// `default` travels as raw JSON: the value is already protocol-validated JSON
/// on arrival, so no remote schema needs an "any" type.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionFormField {
    #[serde(default)]
    pub key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "util::is_false")]
    pub required: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionForm {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    #[serde(default)]
    pub fields: Vec<ExtensionFormField>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionNotification {
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub severity: String,
}

/// A panel has no markdown on purpose — the side rail is a narrow column, and
/// a rendered document there costs more than it tells.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionPanel {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<ExtensionKeyValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ExtensionActionRef>,
}
