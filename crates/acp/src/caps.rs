//! `initialize` handshake and `authenticate` (handshake.go:14-91; protocol.go:
//! 51-193).
//!
//! The reference performs no protocol-version negotiation: the client's
//! `protocolVersion` is parsed but never compared, and the reply always
//! advertises version 1. Client capabilities are recorded tolerantly — a
//! malformed params object leaves them at their zero defaults instead of
//! failing the handshake.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::RpcError;
use crate::methods::{
    SESSION_EXTENSION_SUBMIT_FORM, SESSION_GOAL_PAUSE, SESSION_GOAL_RESUME, SESSION_INFO,
    SESSION_MCP_DISCONNECT, SESSION_MCP_RECONNECT, SESSION_MCP_SERVERS, SESSION_MCP_STATUS,
    SESSION_MCP_STATUS_UPDATE, SESSION_RELOAD_EXTENSIONS, SESSION_STATUS, SESSION_STATUS_UPDATE,
    SESSION_STEER,
};

/// ACP protocol version this agent implements (protocol.go:24).
pub const PROTOCOL_VERSION: i32 = 1;

/// Inbox schema version advertised in the vendor capability block
/// (protocol.go:495).
pub const INBOX_SCHEMA_VERSION: i32 = 1;

/// Vendor extension-surface schema version (protocol.go:136).
pub const EXTENSION_SURFACE_SCHEMA_VERSION: i32 = 1;

// ---- Client → agent handshake ---------------------------------------------

/// Names a participant on the wire (protocol.go:78-82).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Implementation {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Filesystem proxy capabilities the client offers (protocol.go:72-75).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FsCapabilities {
    #[serde(rename = "readTextFile", default, skip_serializing_if = "is_false")]
    pub read_text_file: bool,
    #[serde(rename = "writeTextFile", default, skip_serializing_if = "is_false")]
    pub write_text_file: bool,
}

/// Elicitation modes the client can show (elicitation.go:211-215). A mode
/// counts only when its key is present and not `null`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ElicitationCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Value>,
}

/// What the client declared at initialize (protocol.go:62-69).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ClientCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fs: Option<FsCapabilities>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub terminal: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elicitation: Option<ElicitationCapabilities>,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<BTreeMap<String, Value>>,
}

fn is_false(b: &bool) -> bool {
    !b
}

fn present(raw: Option<&Value>) -> bool {
    let Some(raw) = raw else { return false };
    !raw.is_null()
}

impl ClientCapabilities {
    /// `fs.readTextFile` offered.
    pub fn read_text_file(&self) -> bool {
        self.fs.as_ref().is_some_and(|fs| fs.read_text_file)
    }

    /// `fs.writeTextFile` offered.
    pub fn write_text_file(&self) -> bool {
        self.fs.as_ref().is_some_and(|fs| fs.write_text_file)
    }

    /// Host-owned terminals offered.
    pub fn terminal(&self) -> bool {
        self.terminal
    }

    /// Elicitation form mode advertised (key present and non-null).
    pub fn form_mode(&self) -> bool {
        self.elicitation
            .as_ref()
            .is_some_and(|e| present(e.form.as_ref()))
    }

    /// Elicitation external-page mode advertised.
    pub fn url_mode(&self) -> bool {
        self.elicitation
            .as_ref()
            .is_some_and(|e| present(e.url.as_ref()))
    }

    /// Tolerant vendor boolean: `_meta["reasonix.io"][name]["supported"]` must
    /// be an explicit `true`; absent keys, wrong shapes, and malformed blocks
    /// all mean unsupported (vendor_caps.go:26-37).
    pub fn vendor_bool(&self, name: &str) -> bool {
        let Some(meta) = &self.meta else { return false };
        let Some(Value::Object(vendor)) = meta.get("reasonix.io") else {
            return false;
        };
        let Some(Value::Object(capability)) = vendor.get(name) else {
            return false;
        };
        match capability.get("supported") {
            Some(Value::Bool(flag)) => *flag,
            _ => false,
        }
    }

    /// Structured extension-surface support negotiated at initialize.
    pub fn extension_surface_supported(&self) -> bool {
        self.vendor_bool("extensionSurface")
    }

    /// Opt-in for the unsolicited status/mcpStatus vendor notifications.
    pub fn status_updates_supported(&self) -> bool {
        self.vendor_bool("statusUpdates")
    }
}

/// `initialize` request params (protocol.go:51-55).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct InitializeParams {
    #[serde(rename = "protocolVersion", default)]
    pub protocol_version: i32,
    #[serde(rename = "clientInfo", skip_serializing_if = "Option::is_none")]
    pub client_info: Option<Implementation>,
    #[serde(rename = "clientCapabilities", skip_serializing_if = "Option::is_none")]
    pub client_capabilities: Option<ClientCapabilities>,
}

// ---- Agent → client handshake reply ---------------------------------------

/// Empty capability flag serializes to `{}` (protocol.go:150).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EmptyCapability {}

/// Optional session lifecycle methods the agent supports (protocol.go:153-160).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list: Option<EmptyCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume: Option<EmptyCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close: Option<EmptyCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete: Option<EmptyCapability>,
    #[serde(
        rename = "additionalDirectories",
        skip_serializing_if = "Option::is_none"
    )]
    pub additional_directories: Option<EmptyCapability>,
}

/// Content-block kinds prompts may carry (protocol.go:163-167).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PromptCapabilities {
    #[serde(default)]
    pub image: bool,
    #[serde(default)]
    pub audio: bool,
    #[serde(rename = "embeddedContext", default)]
    pub embedded_context: bool,
}

/// MCP transports session/new accepts (protocol.go:170-173).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct McpCapabilities {
    #[serde(default)]
    pub http: bool,
    #[serde(default)]
    pub sse: bool,
}

/// Vendor-namespaced steering method advertisement (protocol.go:117-119).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSteerCapability {
    pub method: String,
}

/// Durable inbox methods advertisement (protocol.go:122-125).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInboxCapability {
    #[serde(rename = "schemaVersion")]
    pub schema_version: i32,
    pub methods: BTreeMap<String, String>,
}

/// Runtime-reload method advertisement (protocol.go:129-131).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionReloadExtensionsCapability {
    pub method: String,
}

/// Structured extension-UI surface advertisement (protocol.go:144-147).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionSurfaceCapability {
    pub supported: bool,
    #[serde(rename = "schemaVersion")]
    pub schema_version: i32,
}

/// The `reasonix.io` vendor capability block (protocol.go:105-114).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReasonixExtensionCapabilities {
    #[serde(rename = "sessionSteer", skip_serializing_if = "Option::is_none")]
    pub session_steer: Option<SessionSteerCapability>,
    #[serde(rename = "sessionInbox", skip_serializing_if = "Option::is_none")]
    pub session_inbox: Option<SessionInboxCapability>,
    #[serde(
        rename = "sessionReloadExtensions",
        skip_serializing_if = "Option::is_none"
    )]
    pub session_reload_extensions: Option<SessionReloadExtensionsCapability>,
    #[serde(rename = "extensionSurface", skip_serializing_if = "Option::is_none")]
    pub extension_surface: Option<ExtensionSurfaceCapability>,
}

/// Versioned vendor extension advertised by method-name direct key
/// (status.go:29-31).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasonixSchemaCapability {
    #[serde(rename = "schemaVersion")]
    pub schema_version: i32,
}

/// `agentCapabilities` object in the initialize reply (protocol.go:95-101).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AgentCapabilities {
    #[serde(rename = "loadSession")]
    pub load_session: bool,
    #[serde(
        rename = "sessionCapabilities",
        skip_serializing_if = "Option::is_none"
    )]
    pub session_capabilities: Option<SessionCapabilities>,
    #[serde(rename = "promptCapabilities")]
    pub prompt_capabilities: PromptCapabilities,
    #[serde(rename = "mcpCapabilities")]
    pub mcp_capabilities: McpCapabilities,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<BTreeMap<String, Value>>,
}

/// One advertised auth method (protocol.go:176-183).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuthMethod {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
}

/// `initialize` reply (protocol.go:87-92).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: i32,
    #[serde(rename = "agentCapabilities")]
    pub agent_capabilities: AgentCapabilities,
    #[serde(rename = "agentInfo")]
    pub agent_info: Implementation,
    #[serde(rename = "authMethods")]
    pub auth_methods: Vec<AuthMethod>,
}

/// The only advertised auth method (handshake.go:72-80).
pub fn reasonix_setup_auth_method() -> AuthMethod {
    AuthMethod {
        id: "reasonix-setup".to_owned(),
        name: "Reasonix setup".to_owned(),
        description: Some("Configure Reasonix providers and credentials in a terminal".to_owned()),
        r#type: Some("terminal".to_owned()),
        args: Some(vec!["setup".to_owned()]),
        env: None,
    }
}

fn inbox_methods() -> BTreeMap<String, String> {
    crate::methods::INBOX_METHODS
        .iter()
        .map(|(verb, method)| ((*verb).to_owned(), (*method).to_owned()))
        .collect()
}

/// Schema-versioned vendor direct keys advertised under `_meta`
/// (handshake.go:56-64; session_goal.go:45-46).
fn schema_capability_keys() -> BTreeMap<String, ReasonixSchemaCapability> {
    [
        SESSION_STATUS,
        SESSION_STATUS_UPDATE,
        SESSION_MCP_STATUS,
        SESSION_MCP_STATUS_UPDATE,
        SESSION_INFO,
        SESSION_MCP_SERVERS,
        SESSION_MCP_RECONNECT,
        SESSION_MCP_DISCONNECT,
        SESSION_EXTENSION_SUBMIT_FORM,
        SESSION_GOAL_RESUME,
        SESSION_GOAL_PAUSE,
    ]
    .into_iter()
    .map(|method| {
        (
            method.to_owned(),
            ReasonixSchemaCapability {
                schema_version: INBOX_SCHEMA_VERSION,
            },
        )
    })
    .collect()
}

/// Builds the fixed initialize capability tree. `agent_name` / `agent_version`
/// are the ldflags-injected service identity (handshake.go:67).
pub fn initialize_result(agent_name: &str, agent_version: &str) -> InitializeResult {
    let vendor = ReasonixExtensionCapabilities {
        session_steer: Some(SessionSteerCapability {
            method: SESSION_STEER.to_owned(),
        }),
        session_inbox: Some(SessionInboxCapability {
            schema_version: INBOX_SCHEMA_VERSION,
            methods: inbox_methods(),
        }),
        session_reload_extensions: Some(SessionReloadExtensionsCapability {
            method: SESSION_RELOAD_EXTENSIONS.to_owned(),
        }),
        extension_surface: Some(ExtensionSurfaceCapability {
            supported: true,
            schema_version: EXTENSION_SURFACE_SCHEMA_VERSION,
        }),
    };

    let mut meta: BTreeMap<String, Value> = BTreeMap::new();
    meta.insert(
        "reasonix.io".to_owned(),
        serde_json::to_value(&vendor).expect("vendor capabilities serialize"),
    );
    for (method, capability) in schema_capability_keys() {
        meta.insert(
            method,
            serde_json::to_value(&capability).expect("schema capability serializes"),
        );
    }

    InitializeResult {
        protocol_version: PROTOCOL_VERSION,
        agent_capabilities: AgentCapabilities {
            load_session: true,
            session_capabilities: Some(SessionCapabilities {
                list: Some(EmptyCapability {}),
                resume: Some(EmptyCapability {}),
                close: Some(EmptyCapability {}),
                delete: Some(EmptyCapability {}),
                additional_directories: Some(EmptyCapability {}),
            }),
            prompt_capabilities: PromptCapabilities {
                image: true,
                audio: false,
                embedded_context: true,
            },
            mcp_capabilities: McpCapabilities {
                http: true,
                sse: false,
            },
            meta: Some(meta),
        },
        agent_info: Implementation {
            name: agent_name.to_owned(),
            title: None,
            version: Some(agent_version.to_owned()),
        },
        auth_methods: vec![reasonix_setup_auth_method()],
    }
}

/// Protocol version negotiation: the reference ignores the peer version and
/// always answers 1 (matrix §3.5, conflict C3).
pub fn negotiate_protocol_version(_client: i32) -> i32 {
    PROTOCOL_VERSION
}

// ---- authenticate ----------------------------------------------------------

/// `authenticate` params (protocol.go:188-190).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuthenticateParams {
    #[serde(rename = "methodId")]
    pub method_id: String,
}

/// Empty authentication ack (protocol.go:193).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuthenticateResult {}

/// Handles `authenticate`: only `reasonix-setup` (trimmed compare) is
/// accepted; anything else is `-32602` with the verbatim unknown-methodId
/// message. The reply on success is the empty object `{}`.
pub fn authenticate(params: Option<&Value>) -> Result<AuthenticateResult, RpcError> {
    let params = params.cloned().unwrap_or(Value::Null);
    let parsed: AuthenticateParams = serde_json::from_value(params)
        .map_err(|e| RpcError::invalid_params(format!("authenticate: {e}")))?;
    if parsed.method_id.trim() != "reasonix-setup" {
        return Err(RpcError::invalid_params(format!(
            "authenticate: unknown methodId {}",
            parsed.method_id
        )));
    }
    Ok(AuthenticateResult {})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_tree_matches_fixed_capabilities() {
        let result = initialize_result("reasonix", "0.3.0");
        let value = serde_json::to_value(&result).unwrap();

        assert_eq!(value["protocolVersion"], 1);
        assert_eq!(value["agentCapabilities"]["loadSession"], true);
        let sessions = &value["agentCapabilities"]["sessionCapabilities"];
        for key in ["list", "resume", "close", "delete", "additionalDirectories"] {
            assert_eq!(sessions[key], serde_json::json!({}));
        }
        let prompt = &value["agentCapabilities"]["promptCapabilities"];
        assert_eq!(prompt["image"], true);
        assert_eq!(prompt["audio"], false);
        assert_eq!(prompt["embeddedContext"], true);
        let mcp = &value["agentCapabilities"]["mcpCapabilities"];
        assert_eq!(mcp["http"], true);
        assert_eq!(mcp["sse"], false);

        let auth = &value["authMethods"][0];
        assert_eq!(auth["id"], "reasonix-setup");
        assert_eq!(auth["name"], "Reasonix setup");
        assert_eq!(
            auth["description"],
            "Configure Reasonix providers and credentials in a terminal"
        );
        assert_eq!(auth["type"], "terminal");
        assert_eq!(auth["args"], serde_json::json!(["setup"]));
        assert!(auth.get("env").is_none());
    }

    #[test]
    fn initialize_meta_has_vendor_block_and_eleven_schema_keys() {
        let result = initialize_result("reasonix", "0.3.0");
        let meta = &serde_json::to_value(&result).unwrap()["agentCapabilities"]["_meta"];

        let vendor = &meta["reasonix.io"];
        assert_eq!(
            vendor["sessionSteer"]["method"],
            "_reasonix.io/session/steer"
        );
        assert_eq!(vendor["sessionInbox"]["schemaVersion"], 1);
        assert_eq!(
            vendor["sessionInbox"]["methods"].as_object().unwrap().len(),
            9
        );
        assert_eq!(
            vendor["sessionReloadExtensions"]["method"],
            "_reasonix.io/session/reloadExtensions"
        );
        assert_eq!(vendor["extensionSurface"]["supported"], true);
        assert_eq!(vendor["extensionSurface"]["schemaVersion"], 1);

        for key in [
            "_reasonix.io/session/status",
            "_reasonix.io/session/status_update",
            "_reasonix.io/session/mcpStatus",
            "_reasonix.io/session/mcpStatus_update",
            "_reasonix.io/session/info",
            "_reasonix.io/session/mcpServers",
            "_reasonix.io/session/mcp/reconnect",
            "_reasonix.io/session/mcp/disconnect",
            "_reasonix.io/session/extension/submitForm",
            "_reasonix.io/session/goal/resume",
            "_reasonix.io/session/goal/pause",
        ] {
            assert_eq!(meta[key]["schemaVersion"], 1, "missing schema key {key}");
        }
    }

    #[test]
    fn initialize_ignores_client_version_and_replies_one() {
        assert_eq!(negotiate_protocol_version(99), 1);
        assert_eq!(negotiate_protocol_version(0), 1);
    }

    #[test]
    fn vendor_bool_requires_explicit_true_object() {
        let caps: ClientCapabilities = serde_json::from_value(serde_json::json!({
            "_meta": {"reasonix.io": {"extensionSurface": {"supported": true}}}
        }))
        .unwrap();
        assert!(caps.extension_surface_supported());
        assert!(!caps.status_updates_supported());

        let missing: ClientCapabilities = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(!missing.extension_surface_supported());

        let wrong_shape: ClientCapabilities = serde_json::from_value(serde_json::json!({
            "_meta": {"reasonix.io": {"extensionSurface": true}}
        }))
        .unwrap();
        assert!(!wrong_shape.extension_surface_supported());

        let false_value: ClientCapabilities = serde_json::from_value(serde_json::json!({
            "_meta": {"reasonix.io": {"extensionSurface": {"supported": false}}}
        }))
        .unwrap();
        assert!(!false_value.extension_surface_supported());
    }

    #[test]
    fn elicitation_mode_is_present_and_not_null() {
        let yes: ClientCapabilities = serde_json::from_value(serde_json::json!({
            "elicitation": {"form": {}, "url": "https://example.test"}
        }))
        .unwrap();
        assert!(yes.form_mode());
        assert!(yes.url_mode());

        let nulled: ClientCapabilities = serde_json::from_value(serde_json::json!({
            "elicitation": {"form": null, "url": null}
        }))
        .unwrap();
        assert!(!nulled.form_mode());
        assert!(!nulled.url_mode());
    }

    #[test]
    fn authenticate_accepts_only_reasonix_setup() {
        let ok = authenticate(Some(&serde_json::json!({"methodId": "reasonix-setup"})));
        assert!(ok.is_ok());
        assert_eq!(
            serde_json::to_value(ok.unwrap()).unwrap(),
            serde_json::json!({})
        );

        let trimmed = authenticate(Some(&serde_json::json!({"methodId": "  reasonix-setup  "})));
        assert!(trimmed.is_ok());

        let bad = authenticate(Some(&serde_json::json!({"methodId": "other"}))).unwrap_err();
        assert_eq!(bad.code, crate::errors::RpcErrorCode::InvalidParams);
        assert_eq!(bad.message, "authenticate: unknown methodId other");

        let malformed = authenticate(Some(&serde_json::json!({"methodId": 1}))).unwrap_err();
        assert_eq!(malformed.code, crate::errors::RpcErrorCode::InvalidParams);
        assert!(malformed.message.starts_with("authenticate: "));
    }
}
