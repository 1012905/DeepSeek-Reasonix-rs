//! Lifecycle handshake DTOs and validation (contract §2, §12.5).
//!
//! The host sends `extension/initialize` first. The sidecar answers with its
//! declaration; the host validates the protocol version and that every
//! activated capability is a subset of the installed manifest.

use crate::enums::UIHostKind;
use crate::error::{ErrorReason, ProtocolError};
use crate::provider::ProviderDescriptor;
use crate::rawjson::RawJson;
use crate::ui::UIActionDecl;
use serde::{Deserialize, Serialize};

/// Revision of the host/sidecar shared dependency identity schema.
pub const DEPENDENCY_SCHEMA_VERSION: u32 = 1;

/// A namespaced capability identity on the wire.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityWire {
    pub namespace: String,
    pub kind: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    #[serde(
        rename = "schemaHash",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub schema_hash: String,
}

impl CapabilityWire {
    /// The `namespace/kind/id` key used for subset checks.
    pub fn key(&self) -> String {
        format!("{}/{}/{}", self.namespace, self.kind, self.id)
    }
}

/// A dependency requirement on the wire.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementWire {
    pub namespace: String,
    pub kind: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    #[serde(
        rename = "schemaHash",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub schema_hash: String,
    #[serde(
        rename = "versionRange",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub version_range: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

/// The host's opening handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitializeParams {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    #[serde(rename = "protocolId")]
    pub protocol_id: String,
    pub manifest: ManifestExpectation,
    pub session: SessionContext,
    pub capabilities: HostCapabilities,
    #[serde(
        rename = "dependencySchemaVersion",
        default,
        skip_serializing_if = "is_zero_u32"
    )]
    pub dependency_schema_version: u32,
}

/// What the host will accept from this extension, derived from its manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestExpectation {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intercepts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replaces: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<String>,
    #[serde(rename = "uiActions", default, skip_serializing_if = "Vec::is_empty")]
    pub ui_actions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<RequirementWire>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provides: Vec<CapabilityWire>,
}

/// The session the extension serves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionContext {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "workspaceRoot")]
    pub workspace_root: String,
    pub generation: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub epoch: String,
}

/// What the host supports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostCapabilities {
    #[serde(rename = "contentRefs")]
    pub content_refs: bool,
    #[serde(rename = "uiHost")]
    pub ui_host: UIHostKind,
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    #[serde(
        rename = "dependencySchemaVersion",
        default,
        skip_serializing_if = "is_zero_u32"
    )]
    pub dependency_schema_version: u32,
}

/// The sidecar's handshake answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    pub name: String,
    pub version: String,
    #[serde(
        rename = "componentId",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub component_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subscriptions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replaces: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<ProviderDescriptor>,
    #[serde(rename = "uiActions", default, skip_serializing_if = "Vec::is_empty")]
    pub ui_actions: Vec<UIActionDecl>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    #[serde(rename = "toolSpecs", default, skip_serializing_if = "Vec::is_empty")]
    pub tool_specs: Vec<ToolSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<RequirementWire>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provides: Vec<CapabilityWire>,
    #[serde(rename = "stateSchemaVersion")]
    pub state_schema_version: u32,
}

/// A tool a dynamic runtime registered at startup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: RawJson,
    #[serde(
        rename = "readOnly",
        default,
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub read_only: bool,
}

/// No-payload notification: the sidecar may start receiving work.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitializedParams {}

/// Graceful shutdown request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShutdownParams {
    #[serde(rename = "timeoutMillis")]
    pub timeout_millis: i64,
}

/// Shutdown acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShutdownResult {
    pub accepted: bool,
}

/// Notification that watched resources changed on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourcesChangedParams {
    pub paths: Vec<String>,
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

/// Validates a sidecar's declaration against the host's manifest expectation.
///
/// Enforces the exact protocol major and the subset rule: every subscription,
/// replacement slot, provider ref, UI action, and tool must be declared.
pub fn validate_handshake_result(
    plugin_id: &str,
    result: &InitializeResult,
    expectation: &ManifestExpectation,
) -> Result<(), ProtocolError> {
    let capability_error = || ProtocolError::new(ErrorReason::CapabilityNotDeclared);
    crate::error::compare_protocol_version(crate::enums::PROTOCOL_ID, &result.protocol_version)?;

    for point in &result.subscriptions {
        if !expectation.intercepts.contains(point) {
            return Err(capability_error());
        }
    }
    for slot in &result.replaces {
        if !expectation.replaces.contains(slot) {
            return Err(capability_error());
        }
    }
    if !result.providers.is_empty() {
        if !expectation.capabilities.iter().any(|c| c == "providers") {
            return Err(capability_error());
        }
        let prefix = format!("plugin/{plugin_id}/");
        for desc in &result.providers {
            if !desc.ref_.starts_with(&prefix) {
                return Err(capability_error());
            }
        }
    }
    if !result.ui_actions.is_empty() && !expectation.capabilities.iter().any(|c| c == "ui") {
        return Err(capability_error());
    }
    for tool in result
        .tools
        .iter()
        .chain(result.tool_specs.iter().map(|t| &t.name))
    {
        if !expectation.tools.contains(tool) {
            return Err(capability_error());
        }
    }
    validate_provides_ceiling(&expectation.provides, &result.provides)?;
    Ok(())
}

/// Rejects handshake `provides` entries outside the manifest ceiling.
///
/// An empty ceiling means the runtime.capabilities string checks above are
/// authoritative; declared-but-missing capabilities stay unavailable.
pub fn validate_provides_ceiling(
    manifest: &[CapabilityWire],
    wire: &[CapabilityWire],
) -> Result<(), ProtocolError> {
    if wire.is_empty() || manifest.is_empty() {
        return Ok(());
    }
    let allowed: std::collections::HashSet<String> =
        manifest.iter().map(CapabilityWire::key).collect();
    for entry in wire {
        if !allowed.contains(&entry.key()) {
            return Err(ProtocolError::new(ErrorReason::CapabilityNotDeclared));
        }
    }
    Ok(())
}

/// Extracts the plugin ID from `plugin/<pluginID>/<rest...>`.
///
/// The two-segment `plugin/<model>` shape stays an ordinary host ref and
/// returns an empty string.
pub fn plugin_ref_owner(reference: &str) -> &str {
    let Some(rest) = reference.strip_prefix("plugin/") else {
        return "";
    };
    let Some((plugin_id, remainder)) = rest.split_once('/') else {
        return "";
    };
    if plugin_id.is_empty() || remainder.is_empty() {
        return "";
    }
    plugin_id
}

/// The ordered handshake sequence the host and sidecar must follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakePhase {
    /// Before `extension/initialize`.
    New,
    /// `extension/initialize` sent; awaiting the declaration.
    Initializing,
    /// `extension/initialized` sent; ready for work.
    Ready,
    /// Any extension-to-host traffic before Ready poisons the connection.
    Poisoned,
    /// `extension/shutdown` accepted.
    Shutdown,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expectation() -> ManifestExpectation {
        ManifestExpectation {
            intercepts: vec!["tool.before".into()],
            replaces: vec!["tool:bash".into()],
            capabilities: vec!["providers".into(), "ui".into()],
            tools: vec!["lookup".into()],
            ..Default::default()
        }
    }

    fn result() -> InitializeResult {
        InitializeResult {
            protocol_version: "2".into(),
            name: "acme".into(),
            version: "1.2.3".into(),
            component_id: String::new(),
            subscriptions: vec!["tool.before".into()],
            replaces: vec!["tool:bash".into()],
            providers: vec![crate::provider::ProviderDescriptor {
                ref_: "plugin/acme/model".into(),
                ..Default::default()
            }],
            ui_actions: vec![UIActionDecl {
                action_id: "acme.refresh".into(),
                label: "Refresh".into(),
            }],
            tools: vec!["lookup".into()],
            tool_specs: vec![],
            requires: vec![],
            provides: vec![],
            state_schema_version: 2,
        }
    }

    #[test]
    fn valid_handshake_passes() {
        assert!(validate_handshake_result("acme", &result(), &expectation()).is_ok());
    }

    #[test]
    fn undeclared_capabilities_fail_with_typed_reason() {
        let mut expect = expectation();
        expect.intercepts.clear();
        let err = validate_handshake_result("acme", &result(), &expect).unwrap_err();
        assert_eq!(err.reason, ErrorReason::CapabilityNotDeclared);

        let mut expect = expectation();
        expect.capabilities.clear();
        let err = validate_handshake_result("acme", &result(), &expect).unwrap_err();
        assert_eq!(err.reason, ErrorReason::CapabilityNotDeclared);
    }

    #[test]
    fn provider_ref_must_be_in_plugin_namespace() {
        let mut result = result();
        result.providers[0].ref_ = "plugin/other/model".into();
        let err = validate_handshake_result("acme", &result, &expectation()).unwrap_err();
        assert_eq!(err.reason, ErrorReason::CapabilityNotDeclared);
    }

    #[test]
    fn wrong_protocol_version_fails_handshake() {
        let mut result = result();
        result.protocol_version = "3".into();
        let err = validate_handshake_result("acme", &result, &expectation()).unwrap_err();
        assert_eq!(err.reason, ErrorReason::UnsupportedVersion);
    }

    #[test]
    fn provides_ceiling_is_a_subset_check() {
        let manifest = vec![CapabilityWire {
            namespace: "acme".into(),
            kind: "provider".into(),
            id: "model".into(),
            ..Default::default()
        }];
        let wire = manifest.clone();
        assert!(validate_provides_ceiling(&manifest, &wire).is_ok());
        let mut extra = wire;
        extra[0].id = "secret".into();
        assert_eq!(
            validate_provides_ceiling(&manifest, &extra)
                .unwrap_err()
                .reason,
            ErrorReason::CapabilityNotDeclared
        );
    }

    #[test]
    fn plugin_ref_owner_oracle() {
        assert_eq!(plugin_ref_owner("plugin/acme/model"), "acme");
        assert_eq!(plugin_ref_owner("plugin/model"), "");
        assert_eq!(plugin_ref_owner("host/model"), "");
    }

    #[test]
    fn handshake_dtos_round_trip_byte_exact() {
        let params = InitializeParams {
            protocol_version: "2".into(),
            protocol_id: crate::enums::PROTOCOL_ID.into(),
            manifest: expectation(),
            session: SessionContext {
                session_id: "s-1".into(),
                workspace_root: "/repo".into(),
                generation: 3,
                epoch: String::new(),
            },
            capabilities: HostCapabilities {
                content_refs: true,
                ui_host: UIHostKind::Desktop,
                protocol_version: "2".into(),
                dependency_schema_version: 1,
            },
            dependency_schema_version: 1,
        };
        let json = serde_json::to_string(&params).unwrap();
        assert!(json.contains("\"protocolVersion\":\"2\""));
        assert!(json.contains("\"uiHost\":\"desktop\""));
        let decoded: InitializeParams = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, params);
        let decoded: Result<InitializeParams, _> =
            serde_json::from_str(&json.replace("\"session\"", "\"sesion\""));
        assert!(decoded.is_err(), "strict DTO must reject unknown members");
    }
}
