//! The `setting` tool (Go `internal/tools/builtin/setting.go`): look up the
//! user's own settings and ask for a change. A change is only ever applied
//! after the user's own answer to a host-worded card; the tool never writes
//! on the model's say-so.

use std::any::Any;
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use crate::abi::{
    CallContext, Capability, CapabilitySet, DecisionCallBarrier, ReadOnlyHostMutation, Tool,
    ToolResult,
};
use crate::builtin::peer::PrefixedError;
use crate::refusal::Refusal;

/// `setting.unavailable` — the session cannot change settings.
pub const CODE_SETTING_UNAVAILABLE: &str = "setting.unavailable";
/// `setting.invalid_value` — propose named no id or no value.
pub const CODE_SETTING_INVALID_VALUE: &str = "setting.invalid_value";

/// The list cap (Go `settingListMax`).
pub const SETTING_LIST_MAX: usize = 40;

/// `setting`, the tool name the model calls.
pub const SETTING_TOOL_NAME: &str = "setting";

/// Go `settingTool.Description()`, verbatim.
pub const SETTING_DESCRIPTION: &str = "Find and change the user's own settings. op=list finds settings by words of their id (for example \"peers dispatch\"). op=propose asks the user to confirm one change: give the id, the new value and a short reason; nothing changes until the user answers, and the result says what changed or that they declined. op=open points the user at a setting that only they may change. Use it when a refusal ends with (fix: tool:setting ...), or when a tool you need is not offered because a switch is off. Do not change a setting the task did not call for.";

const GO_SCHEMA: &str = r#"{"type":"object","properties":{"op":{"type":"string","enum":["list","propose","open"]},"query":{"type":"string","description":"Words of the setting's id, for op=list"},"id":{"type":"string","description":"The setting's id, for propose and open"},"value":{"description":"The new value as JSON, for op=propose"},"reason":{"type":"string","description":"Why the change helps the user, shown on the card, for op=propose"}},"required":["op"],"additionalProperties":false}"#;

/// One row of what the model may learn about a setting (Go `tool.SettingInfo`).
/// `value` is absent for a secret or a setting edited in its own screen.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SettingInfo {
    pub id: String,
    pub class: String,
    pub effect: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value: Option<Value>,
}

/// What a confirmed change did (Go `tool.SettingReceipt`). `via` is `click`
/// for a fresh answer and `session_grant` for one given earlier in the
/// session for this setting.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SettingReceipt {
    pub id: String,
    pub from: Value,
    pub to: Value,
    pub effect: String,
    pub via: String,
}

/// The controller side of the setting tool (Go `tool.SettingsHost`). The
/// tool holds no setting id whitelist: the list of keys comes from the host.
pub trait SettingsHost: Send + Sync {
    fn settings_list(&self, query: &str) -> Vec<SettingInfo>;
    fn propose_setting(
        &self,
        id: &str,
        value: &Value,
        reason: &str,
    ) -> Result<SettingReceipt, Box<dyn std::error::Error + Send + Sync>>;
    fn open_setting(&self, id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// Joins the setting tool, built with the registry, to the controller that
/// exists only afterwards (Go `tool.SettingsLink`).
#[derive(Default)]
pub struct SettingsLink {
    host: Mutex<Option<Arc<dyn SettingsHost>>>,
}

impl SettingsLink {
    /// Binds the host the tool calls through.
    pub fn bind(&self, host: Arc<dyn SettingsHost>) {
        *self.host.lock().unwrap_or_else(|e| e.into_inner()) = Some(host);
    }

    /// The bound host, if any.
    pub fn host(&self) -> Option<Arc<dyn SettingsHost>> {
        self.host.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// The `setting` tool (Go `settingTool`).
#[derive(Clone)]
pub struct SettingTool {
    link: Arc<SettingsLink>,
}

/// Go `SettingTool(link)`.
pub fn setting_tool(link: &Arc<SettingsLink>) -> Arc<dyn Tool> {
    Arc::new(SettingTool { link: link.clone() })
}

#[derive(Deserialize, Default)]
struct Args {
    #[serde(default)]
    op: String,
    #[serde(default)]
    query: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    value: Option<Value>,
    #[serde(default)]
    reason: String,
}

impl Tool for SettingTool {
    fn name(&self) -> &str {
        SETTING_TOOL_NAME
    }

    fn description(&self) -> &str {
        SETTING_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(GO_SCHEMA).expect("the setting schema is valid JSON")
    }

    fn execute(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let parsed: Args = serde_json::from_value(args.clone()).map_err(|e| {
            std::io::Error::other(format!(
                "setting needs {{\"op\": list|propose|open, ...}}: {e}"
            ))
        })?;
        let Some(host) = self.link.host() else {
            return Err(Refusal::code(
                CODE_SETTING_UNAVAILABLE,
                "Settings cannot be changed from this session.",
            )
            .into());
        };
        match parsed.op.as_str() {
            "list" => list_settings(host.settings_list(&parsed.query)),
            "propose" => {
                // Go treats an explicit JSON null as a present value; only a
                // missing key counts as len(in.Value) == 0.
                let value_present = args.get("value").is_some();
                if parsed.id.trim().is_empty() || !value_present {
                    return Err(Box::new(PrefixedError::new(
                        CODE_SETTING_INVALID_VALUE,
                        "op=propose needs an id and a value",
                    )));
                }
                let value = parsed.value.unwrap_or(Value::Null);
                let receipt = host.propose_setting(&parsed.id, &value, &parsed.reason)?;
                let out = serde_json::to_string(&receipt)
                    .map_err(|e| std::io::Error::other(e.to_string()))?;
                Ok(out)
            }
            "open" => {
                host.open_setting(&parsed.id)?;
                Ok(format!(
                    "Pointed the user at {}. Only they can change it; carry on without it and say what it blocks.",
                    parsed.id
                ))
            }
            _ => Err(Box::new(std::io::Error::other(
                "setting needs op list, propose or open",
            ))),
        }
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default()
            .with(
                "host_mutation",
                Arc::new(HostMutation) as Arc<dyn ReadOnlyHostMutation>,
            )
            .with(
                "decision_call_barrier",
                Arc::new(SettingBarrier) as Arc<dyn DecisionCallBarrier>,
            )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn op_is_propose(args: &Value) -> bool {
    #[derive(Deserialize, Default)]
    struct Op {
        #[serde(default)]
        op: String,
    }
    serde_json::from_value::<Op>(args.clone()).is_ok_and(|op| op.op == "propose")
}

struct SettingBarrier;
impl Capability for SettingBarrier {}
impl DecisionCallBarrier for SettingBarrier {
    fn decision_barrier_call(&self, args: &Value) -> bool {
        op_is_propose(args)
    }
}

struct HostMutation;
impl Capability for HostMutation {}
impl ReadOnlyHostMutation for HostMutation {
    fn read_only_execution_host_mutation(&self) -> bool {
        true
    }
}

/// Renders the settings list, capped at [`SETTING_LIST_MAX`]; `truncated` is
/// omitted when every row fits (Go `listSettings`).
pub fn list_settings(rows: Vec<SettingInfo>) -> ToolResult {
    let truncated = rows.len() > SETTING_LIST_MAX;
    let kept: Vec<SettingInfo> = if truncated {
        rows.into_iter().take(SETTING_LIST_MAX).collect()
    } else {
        rows
    };
    #[derive(Serialize)]
    struct Out {
        settings: Vec<SettingInfo>,
        #[serde(skip_serializing_if = "Option::is_none")]
        truncated: Option<bool>,
    }
    let out = Out {
        settings: kept,
        truncated: truncated.then_some(true),
    };
    let text = serde_json::to_string(&out).map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(text)
}

/// The schema as one JSON value, for tests and contract reads.
pub fn schema_value() -> Value {
    serde_json::from_str(GO_SCHEMA).expect("the setting schema is valid JSON")
}
