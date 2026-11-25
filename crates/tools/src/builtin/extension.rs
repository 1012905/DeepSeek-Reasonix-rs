//! TOOL-041: extension runtime tools (docs/contracts/tools.md §3.6; Go
//! `internal/assembly/boot/extension_tools.go`,
//! `internal/assembly/boot/extension_discovery_tool.go`).
//!
//! The `ext__` namespace qualifies a runtime's tool with its plugin so two
//! plugins can each serve a "search" and neither can take a built-in's name.
//! The card, the name minting, the contract digest recheck and the over-long
//! offer filter are pure here; the sidecar process lifecycle, the ready
//! handshake and the per-call transport stay behind the [`ExtensionHost`]
//! seam the integration slice binds. A runtime that failed to start contributes
//! none, so the catalog never lists a tool nothing can run.

#![forbid(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::abi::{CallContext, Capability, CapabilitySet, Tool, ToolResult};
use crate::registry::EXTENSION_NAME_PREFIX;

/// The on-demand discovery tool namespace: `connect_ext__<slug>_<digest8>`.
pub const CONNECT_EXTENSION_PREFIX: &str = "connect_ext__";

/// The longest name a model tool may carry (Go `len(name) <= 64`).
pub const MAX_TOOL_NAME_LEN: usize = 64;

/// The plugin slug kept in a discovery tool name before the digest suffix.
const DISCOVERY_SLUG_LEN: usize = 42;
/// Digest prefix carried in the discovery tool name.
const DISCOVERY_DIGEST_LEN: usize = 8;

/// One extension call's result (Go `protocol.ToolCallResult`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionCall {
    /// Whether the sidecar reported the call as an error.
    pub is_error: bool,
    /// The content the sidecar returned (error text when `is_error`).
    pub content: String,
}

/// A tool declaration a plugin runtime serves (Go `pluginpkg.RuntimeTool`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct RuntimeTool {
    /// The tool's bare name inside the plugin.
    #[serde(rename = "name")]
    pub name: String,
    /// The author-written description.
    #[serde(default, rename = "description")]
    pub description: String,
    /// The input schema, as a JSON value.
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Value,
    /// The author's read-only claim; never inferred.
    #[serde(default, rename = "readOnly")]
    pub read_only: bool,
}

impl RuntimeTool {
    /// The contract object this runtime declares, in the wire shape the
    /// digest hashes.
    fn digest_value(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": self.input_schema,
            "readOnly": self.read_only,
        })
    }
}

/// Lowercase hex SHA-256 of bytes (Go `sha256.Sum256` + `%x`).
fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap_or('0'));
    }
    out
}

/// Normalized-JSON digest (Go `agentcomposition.Digest`): marshal, unmarshal
/// into a generic value so object keys and number forms canonicalize,
/// remarshal, then SHA-256 hex. An unparseable value digests to the empty
/// string, matching the Go failure arm.
pub fn digest(value: &Value) -> String {
    let Ok(normalized) = serde_json::from_value::<Value>(value.clone()) else {
        return String::new();
    };
    let Ok(bytes) = serde_json::to_vec(&normalized) else {
        return String::new();
    };
    hex_lower(&Sha256::digest(bytes))
}

/// The digest of one tool declaration's contract.
pub fn decl_digest(decl: &RuntimeTool) -> String {
    digest(&decl.digest_value())
}

/// Whether the live declaration still matches the contract registered when
/// the tool was minted (Go recheck before dispatch).
pub fn contract_matches(current: &RuntimeTool, registered: &RuntimeTool) -> bool {
    decl_digest(current) == decl_digest(registered)
}

/// Qualifies a runtime's tool with its plugin
/// (`extensionToolName`): `ext__<pluginID with .→_>__<toolName>`.
pub fn extension_tool_name(plugin_id: &str, tool_name: &str) -> String {
    let slug = plugin_id.replace('.', "_");
    format!("{EXTENSION_NAME_PREFIX}{slug}__{tool_name}")
}

/// Mints the on-demand discovery tool name
/// (`extensionDiscoveryTool.Name`): `connect_ext__` + slug (`.`→`_`, truncated
/// to 42 bytes) + `_` + first 8 digest chars of the plugin id.
pub fn discovery_tool_name(plugin_id: &str) -> String {
    let slug = plugin_id.replace('.', "_");
    let keep = slug.bytes().take(DISCOVERY_SLUG_LEN).collect::<Vec<u8>>();
    let keep = String::from_utf8_lossy(&keep).into_owned();
    let digest = digest(&Value::String(plugin_id.to_owned()));
    let suffix = digest
        .get(..DISCOVERY_DIGEST_LEN)
        .unwrap_or(&digest)
        .to_owned();
    format!("{CONNECT_EXTENSION_PREFIX}{keep}_{suffix}")
}

/// The discovery tool's fixed description (Go verbatim).
pub fn discovery_description(plugin_id: &str) -> String {
    format!("Connect extension {plugin_id} and list its available tools.")
}

/// Whether a minted name may be offered to the model (Go `len(name) <= 64`).
pub fn name_offered(name: &str) -> bool {
    name.len() <= MAX_TOOL_NAME_LEN
}

/// The over-long offer warning (Go verbatim format).
pub fn overlong_warning(plugin_id: &str, tool_name: &str, name: &str) -> String {
    format!(
        "extension {plugin_id}: tool {tool_name} is not offered: {name} is over the 64 characters a model tool name may have"
    )
}

/// One minted extension tool kept by the assembly slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintedExtension {
    /// The model-visible minted name.
    pub name: String,
    /// The owning plugin id.
    pub plugin_id: String,
    /// The declaration the tool was minted from.
    pub decl: RuntimeTool,
}

/// Mints names for a plugin's declarations, dropping over-long names the Go
/// side silently does not offer.
pub fn mint_declared(plugin_id: &str, decls: &[RuntimeTool]) -> Vec<MintedExtension> {
    decls
        .iter()
        .map(|decl| {
            let name = extension_tool_name(plugin_id, &decl.name);
            MintedExtension {
                name,
                plugin_id: plugin_id.to_owned(),
                decl: decl.clone(),
            }
        })
        .filter(|minted| name_offered(&minted.name))
        .collect()
}

/// The host seam behind an extension runtime. Real sidecar process loading,
/// the ready handshake and the JSON-RPC transport live in the integration
/// slice; this slice ships the card and the pure decision behavior.
pub trait ExtensionHost: Send + Sync {
    /// Whether the runtime currently serves the named tool.
    fn serves_tool(&self, plugin_id: &str, tool_name: &str) -> bool;
    /// The declarations the runtime currently advertises.
    fn declared_tools(&self, plugin_id: &str) -> Vec<RuntimeTool>;
    /// Calls one served tool and returns its result.
    fn call_tool(
        &self,
        plugin_id: &str,
        tool_name: &str,
        args: &Value,
    ) -> Result<ExtensionCall, Box<dyn std::error::Error + Send + Sync>>;
    /// Brings the runtime up on demand and returns its advertised declarations.
    fn ensure_ready(
        &self,
        plugin_id: &str,
    ) -> Result<Vec<RuntimeTool>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Typed extension failure identities.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExtensionError {
    /// The runtime no longer serves the tool the registry still lists.
    #[error("extension {plugin_id} no longer serves tool {tool_name}")]
    NoLongerServes {
        plugin_id: String,
        tool_name: String,
    },
    /// The declared contract changed between registration and dispatch.
    #[error(
        "extension:{plugin_id}: Tool contract changed before dispatch; refresh the capability directory and retry."
    )]
    ContractChanged { plugin_id: String },
    /// The sidecar call wrapped its own error.
    #[error("the {plugin_id} extension could not run {tool_name}: {inner}")]
    CallFailed {
        plugin_id: String,
        tool_name: String,
        inner: String,
    },
    /// The sidecar reported the call as an error and returned its own text.
    #[error("extension {plugin_id} tool {tool_name} reported an error: {content}")]
    CallReportedError {
        plugin_id: String,
        tool_name: String,
        content: String,
    },
    /// The host seam is not wired in this slice.
    #[error("extension host seam is not wired in this slice")]
    SeamUnwired,
}

/// Capability declaring the package a tool belongs to (Go
/// `ExtensionPackageName`).
pub trait ExtensionPackageMetadata: Capability {
    fn extension_package_name(&self) -> String;
}

struct PackageMeta {
    plugin_id: String,
}
impl Capability for PackageMeta {}
impl ExtensionPackageMetadata for PackageMeta {
    fn extension_package_name(&self) -> String {
        self.plugin_id.clone()
    }
}

/// The `ext__` extension tool card (Go `extensionTool`).
#[derive(Clone)]
pub struct ExtensionTool {
    name: String,
    plugin_id: String,
    decl: RuntimeTool,
    host: Option<Arc<dyn ExtensionHost>>,
}

impl ExtensionTool {
    /// Builds the card over a minted name, plugin id, declaration and optional
    /// host seam.
    pub fn new(
        name: String,
        plugin_id: String,
        decl: RuntimeTool,
        host: Option<Arc<dyn ExtensionHost>>,
    ) -> Self {
        Self {
            name,
            plugin_id,
            decl,
            host,
        }
    }

    /// The owning plugin id, exposed for the registry's package metadata.
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }
}

impl Tool for ExtensionTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.decl.description
    }

    fn schema(&self) -> Value {
        self.decl.input_schema.clone()
    }

    fn execute(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let Some(host) = &self.host else {
            return Err(Box::new(ExtensionError::SeamUnwired));
        };
        if !host.serves_tool(&self.plugin_id, &self.decl.name) {
            return Err(Box::new(ExtensionError::NoLongerServes {
                plugin_id: self.plugin_id.clone(),
                tool_name: self.decl.name.clone(),
            }));
        }
        for current in host.declared_tools(&self.plugin_id) {
            if current.name == self.decl.name && !contract_matches(&current, &self.decl) {
                return Err(Box::new(ExtensionError::ContractChanged {
                    plugin_id: self.plugin_id.clone(),
                }));
            }
        }
        let result = host
            .call_tool(&self.plugin_id, &self.decl.name, args)
            .map_err(|inner| ExtensionError::CallFailed {
                plugin_id: self.plugin_id.clone(),
                tool_name: self.decl.name.clone(),
                inner: inner.to_string(),
            })?;
        if result.is_error {
            return Err(Box::new(ExtensionError::CallReportedError {
                plugin_id: self.plugin_id.clone(),
                tool_name: self.decl.name.clone(),
                content: result.content,
            }));
        }
        Ok(result.content)
    }

    fn read_only(&self) -> bool {
        self.decl.read_only
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default().with(
            "extension_package",
            Arc::new(PackageMeta {
                plugin_id: self.plugin_id.clone(),
            }) as Arc<dyn ExtensionPackageMetadata>,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The `connect_ext__` on-demand discovery tool card (Go
/// `extensionDiscoveryTool`). The registry keeps it even when the plugin is
/// not yet connected; calling it connects the sidecar and registers the
/// concrete `ext__` tools.
#[derive(Clone)]
pub struct ExtensionDiscoveryTool {
    plugin_id: String,
    name: String,
    description: String,
    host: Option<Arc<dyn ExtensionHost>>,
}

impl ExtensionDiscoveryTool {
    /// Builds the discovery card for one plugin id.
    pub fn new(plugin_id: String, host: Option<Arc<dyn ExtensionHost>>) -> Self {
        let name = discovery_tool_name(&plugin_id);
        let description = discovery_description(&plugin_id);
        Self {
            plugin_id,
            name,
            description,
            host,
        }
    }

    /// The owning plugin id.
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }
}

impl Tool for ExtensionDiscoveryTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> Value {
        json!({"type":"object","properties":{},"additionalProperties":false})
    }

    fn execute(&self, _ctx: &CallContext, _args: &Value) -> ToolResult {
        let Some(host) = &self.host else {
            return Err(Box::new(ExtensionError::SeamUnwired));
        };
        let declared = host.ensure_ready(&self.plugin_id)?;
        let wire = serde_json::to_value(&declared).map_err(|e| {
            Box::new(std::io::Error::other(format!("encode declared tools: {e}")))
                as Box<dyn std::error::Error + Send + Sync>
        })?;
        Ok(wire.to_string())
    }

    fn read_only(&self) -> bool {
        false
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default().with(
            "extension_package",
            Arc::new(PackageMeta {
                plugin_id: self.plugin_id.clone(),
            }) as Arc<dyn ExtensionPackageMetadata>,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
