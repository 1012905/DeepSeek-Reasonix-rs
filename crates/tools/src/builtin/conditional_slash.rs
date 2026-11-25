//! TOOL-043: slash_command and install_source conditional surfaces
//! (docs/contracts/tools.md §3.8; Go `internal/ext/command/slashtool.go`,
//! `internal/ext/installsource/install_source.go`).
//!
//! The slash catalog/gate/expansion and the install-source argument gate,
//! approval and connector seams are pure here; the live command source, the
//! real source resolution/download/persist and the rollback flow stay behind
//! seams the integration slice binds.

#![forbid(unsafe_code)]

use std::any::Any;
use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::abi::{CallContext, Tool, ToolResult};
use crate::refusal::Refusal;

// --- slash_command ------------------------------------------------------------

/// The `slash_command` tool name.
pub const SLASH_COMMAND_TOOL_NAME: &str = "slash_command";

const SLASH_DESCRIPTION: &str = "Invoke a project slash command (a reusable prompt template or skill) by name. Returns the command's expanded prompt text for you to act on in this turn — it does not run on its own. Call with an empty command (or \"list\") to see what is available.";

const SLASH_SCHEMA: &str = r#"{"type":"object","properties":{"command":{"type":"string","description":"Slash command name (with or without a leading slash). Empty or \"list\" returns the available commands."},"arguments":{"type":"string","description":"Arguments passed to the command, as you'd type them after the name (space-separated)."}}}"#;

/// Turns positional args into the expanded prompt (Go `SlashEntry.Render`).
pub trait SlashRenderer: Send + Sync {
    fn render(&self, args: &[String]) -> String;
}

/// Keeps an entry out of the listing without refusing a call that names it.
pub trait SlashVisibility: Send + Sync {
    fn hidden(&self) -> bool;
}

/// One invocable slash command (Go `command.SlashEntry`).
#[derive(Clone)]
pub struct SlashEntry {
    #[allow(missing_docs)]
    pub name: String,
    #[allow(missing_docs)]
    pub description: String,
    #[allow(missing_docs)]
    pub arg_hint: String,
    #[allow(missing_docs)]
    pub skill: bool,
    #[allow(missing_docs)]
    pub visibility: Option<Arc<dyn SlashVisibility>>,
    #[allow(missing_docs)]
    pub renderer: Option<Arc<dyn SlashRenderer>>,
}

impl std::fmt::Debug for SlashEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlashEntry")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("arg_hint", &self.arg_hint)
            .field("skill", &self.skill)
            .finish_non_exhaustive()
    }
}

/// Normalizes a command spelling: trim space and one leading slash (Go
/// `TrimPrefix(TrimSpace(name), "/")`).
pub fn normalize_slash_name(raw: &str) -> String {
    let trimmed = raw.trim();
    trimmed.strip_prefix('/').unwrap_or(trimmed).to_owned()
}

/// The loaded command set: later entries win a name clash, names sorted for
/// a stable listing (Go `NewSlashCommandTool`).
#[derive(Debug, Clone, Default)]
pub struct SlashCatalog {
    entries: BTreeMap<String, SlashEntry>,
}

impl SlashCatalog {
    /// Builds the catalog from raw entries.
    pub fn build(entries: Vec<SlashEntry>) -> Self {
        let mut map: BTreeMap<String, SlashEntry> = BTreeMap::new();
        for mut entry in entries {
            let name = normalize_slash_name(&entry.name);
            if name.is_empty() {
                continue;
            }
            entry.name = name;
            map.insert(entry.name.clone(), entry);
        }
        Self { entries: map }
    }

    /// The sorted, visible entry names for one gate snapshot.
    pub fn allowed_names(&self, gate: Option<&dyn SlashGate>) -> Vec<String> {
        self.entries
            .keys()
            .filter(|name| {
                let Some(entry) = self.entries.get(*name) else {
                    return false;
                };
                if let Some(visibility) = &entry.visibility
                    && visibility.hidden()
                {
                    return false;
                }
                match gate {
                    Some(gate) if entry.skill => gate.admit(name).is_ok(),
                    _ => true,
                }
            })
            .cloned()
            .collect()
    }

    /// Looks one entry up by normalized name.
    pub fn get(&self, name: &str) -> Option<&SlashEntry> {
        self.entries.get(name)
    }
}

/// The call-time gate behind skill entries (Go `gate` snapshot).
pub trait SlashGate: Send + Sync {
    fn admit(&self, name: &str) -> Result<(), Refusal>;
}

/// Renders the listing the model reads without naming a command.
pub fn list_slash_commands(catalog: &SlashCatalog, gate: Option<&dyn SlashGate>) -> String {
    let names = catalog.allowed_names(gate);
    if names.is_empty() {
        return "No slash commands are configured in this project.".to_owned();
    }
    let mut body = String::from("Available slash commands:\n");
    for name in names {
        let Some(entry) = catalog.get(&name) else {
            continue;
        };
        let mut line = format!("- /{name}");
        if !entry.arg_hint.is_empty() {
            line.push(' ');
            line.push_str(&entry.arg_hint);
        }
        if !entry.description.is_empty() {
            line.push_str(" — ");
            line.push_str(&entry.description);
        }
        body.push_str(&line);
        body.push('\n');
    }
    body.trim_end_matches('\n').to_owned()
}

/// Frames an expansion so the model treats it as an instruction now (Go
/// `Expanded /%s — follow these instructions now:`).
pub fn expand_frame(name: &str, expanded: &str) -> String {
    format!("Expanded /{name} — follow these instructions now:\n\n{expanded}")
}

/// The `slash_command` tool card over a snapshot catalog and optional gate.
#[derive(Clone)]
pub struct SlashCommandTool {
    catalog: SlashCatalog,
    gate: Option<Arc<dyn SlashGate>>,
}

impl SlashCommandTool {
    /// Builds the card.
    pub fn new(catalog: SlashCatalog, gate: Option<Arc<dyn SlashGate>>) -> Self {
        Self { catalog, gate }
    }
}

#[derive(Deserialize, Default)]
struct SlashArgs {
    #[serde(default)]
    command: String,
    #[serde(default)]
    arguments: String,
}

impl Tool for SlashCommandTool {
    fn name(&self) -> &str {
        SLASH_COMMAND_TOOL_NAME
    }

    fn description(&self) -> &str {
        SLASH_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(SLASH_SCHEMA).expect("the slash_command schema is valid JSON")
    }

    fn execute(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let parsed: SlashArgs = serde_json::from_value(args.clone())
            .map_err(|e| invalid_err(format!("invalid args: {e}")))?;
        let name = normalize_slash_name(&parsed.command);
        let gate = self.gate.as_deref();
        if name.is_empty() || name.eq_ignore_ascii_case("list") {
            return Ok(list_slash_commands(&self.catalog, gate));
        }
        let Some(entry) = self.catalog.get(&name) else {
            let available = self.catalog.allowed_names(gate).join(", ");
            return Err(invalid_err(format!(
                "no slash command {name:?}; available: {available}"
            )));
        };
        if entry.skill
            && let Some(gate) = gate
        {
            gate.admit(&name)?;
        }
        let positional: Vec<String> = parsed
            .arguments
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        let Some(renderer) = &entry.renderer else {
            return Err(invalid_err(format!(
                "slash_command: /{name} has no renderer wired in this slice"
            )));
        };
        let expanded = renderer.render(&positional);
        if entry.skill && expanded.is_empty() {
            return Err(invalid_err(format!(
                "slash_command: /{name} changed while it was being expanded; call it again"
            )));
        }
        Ok(expand_frame(&name, &expanded))
    }

    fn read_only(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// --- install_source ------------------------------------------------------------

/// The `install_source` tool name.
pub const INSTALL_SOURCE_TOOL_NAME: &str = "install_source";

pub const INSTALL_SOURCE_DESCRIPTION: &str = "Plan, install, or uninstall a Reasonix skill, MCP server, or plugin package from a URL, local file/folder, .mcp.json, executable, or package name. Two-phase: with apply=false (default) returns a deterministic plan with per-action risk level and a planId; with apply=true, passing back that planId, it copies/registers skills, connects/persists MCP servers, or installs plugin packages. An apply without the planId of a plan you have read installs nothing and returns the plan instead. op='uninstall' removes a previously installed skill, MCP server, or plugin package by name.";

const INSTALL_SOURCE_SCHEMA: &str = r#"{
"type":"object",
"properties":{
  "op":{"type":"string","enum":["install","uninstall"],"description":"Whether to install (default) or uninstall."},
  "source":{"type":"string","description":"URL, local file/folder path, .mcp.json path, or package name to install from. Ignored when op=uninstall (use name instead)."},
  "kind":{"type":"string","enum":["auto","skill","mcp","plugin"],"description":"Capability kind. Defaults to auto."},
  "apply":{"type":"boolean","description":"false (default) only returns an install plan; true performs the planned writes/connects. Ignored for op=uninstall."},
  "scope":{"type":"string","enum":["project","global"],"description":"Where to persist config or copy skills. MCP installs and skills default to global; project-root .mcp.json imports default to project. Set project explicitly to install only for this workspace."},
  "mode":{"type":"string","enum":["auto","copy","link","register"],"description":"Skill install mode. auto registers multi-skill roots and copies single skills into the canonical <skill-name>/SKILL.md layout; copy copies skill files/folders; link creates symlinks; register adds a skill root to [skills].paths."},
  "name":{"type":"string","description":"Optional override for the installed MCP server or single skill name. Required for op=uninstall when removing by name."},
  "transport":{"type":"string","enum":["auto","stdio","http","sse"],"description":"MCP transport override. URL sources default to http unless --sse-like; package sources default to stdio."},
  "tier":{"type":"string","enum":["background","eager"],"description":"Persisted MCP startup tier. Defaults to background."},
  "planId":{"type":"string","description":"The planId from the plan being applied. Required with apply=true: it proves the plan was read, and its leading risk level (low/medium/high) is what the permission layer decides on."}
},
"required":[]
}"#;

/// One raw install request, as the model posted it.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct InstallRequest {
    #[serde(default)]
    pub op: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub apply: Option<bool>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub transport: Option<String>,
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(rename = "planId", default)]
    pub plan_id: Option<String>,
}

/// Typed install-source failure identities.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstallError {
    #[error("install_source: op {op:?} is not supported (want install|uninstall)")]
    OpUnsupported { op: String },
    #[error("install_source requires a non-empty source")]
    MissingSource,
    #[error("install_source: op=uninstall requires a non-empty name")]
    MissingUninstallName,
    #[error("planId mismatch (got {got}, expected {expected}); re-plan and re-approve")]
    PlanIdMismatch { got: String, expected: String },
    #[error("install_source connector seam is not wired in this slice")]
    SeamUnwired,
}

/// Normalizes the operation; empty defaults to `install`.
pub fn normalize_install_op(raw: &str) -> Result<&'static str, InstallError> {
    match raw.trim() {
        "" | "install" => Ok("install"),
        "uninstall" => Ok("uninstall"),
        other => Err(InstallError::OpUnsupported {
            op: other.to_owned(),
        }),
    }
}

/// Normalizes the capability kind; unknown or empty reads `auto`.
pub fn normalize_kind(raw: &str) -> &'static str {
    match raw.trim() {
        "skill" => "skill",
        "mcp" => "mcp",
        "plugin" => "plugin",
        _ => "auto",
    }
}

/// Normalizes the persistence scope; unknown reads `None` (host decides).
pub fn normalize_scope(raw: &str) -> Option<&'static str> {
    match raw.trim() {
        "project" => Some("project"),
        "global" => Some("global"),
        _ => None,
    }
}

/// Normalizes the skill install mode; unknown reads `auto`.
pub fn normalize_mode(raw: &str) -> &'static str {
    match raw.trim() {
        "copy" => "copy",
        "link" => "link",
        "register" => "register",
        _ => "auto",
    }
}

/// Normalizes the MCP transport; unknown reads `auto`.
pub fn normalize_transport(raw: &str) -> &'static str {
    match raw.trim() {
        "stdio" => "stdio",
        "http" => "http",
        "sse" => "sse",
        _ => "auto",
    }
}

/// Normalizes the startup tier; unknown reads `None` (host default).
pub fn normalize_tier(raw: &str) -> Option<&'static str> {
    match raw.trim() {
        "background" => Some("background"),
        "eager" => Some("eager"),
        _ => None,
    }
}

/// A validated, normalized install request ready for the plan seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedRequest {
    pub op: &'static str,
    pub source: String,
    pub kind: &'static str,
    pub apply: bool,
    pub scope: Option<&'static str>,
    pub mode: &'static str,
    pub name: String,
    pub transport: &'static str,
    pub tier: Option<&'static str>,
    pub plan_id: String,
}

/// Validates and normalizes one request (Go `execute` argument gate).
pub fn validate_install_request(req: &InstallRequest) -> Result<NormalizedRequest, InstallError> {
    let op = normalize_install_op(req.op.as_deref().unwrap_or_default())?;
    let source = req.source.as_deref().unwrap_or_default().trim().to_owned();
    let name = req.name.as_deref().unwrap_or_default().trim().to_owned();
    if op == "install" && source.is_empty() {
        return Err(InstallError::MissingSource);
    }
    if op == "uninstall" && name.is_empty() {
        return Err(InstallError::MissingUninstallName);
    }
    Ok(NormalizedRequest {
        op,
        source,
        kind: normalize_kind(req.kind.as_deref().unwrap_or_default()),
        apply: req.apply.unwrap_or(false),
        scope: normalize_scope(req.scope.as_deref().unwrap_or_default()),
        mode: normalize_mode(req.mode.as_deref().unwrap_or_default()),
        name,
        transport: normalize_transport(req.transport.as_deref().unwrap_or_default()),
        tier: normalize_tier(req.tier.as_deref().unwrap_or_default()),
        plan_id: req.plan_id.as_deref().unwrap_or_default().trim().to_owned(),
    })
}

/// Whether an apply call carries no plan ticket (Go `unticketed`): apply on,
/// the plan gate required, and an empty plan id. Such a call returns the plan
/// instead of installing anything.
pub fn unticketed_apply(apply: bool, require_approved_plan: bool, plan_id: &str) -> bool {
    apply && require_approved_plan && plan_id.trim().is_empty()
}

/// The connect seam behind an install. Real source resolution, download,
/// launch grant and persist live in the ext/assembly slices.
pub trait InstallConnector: Send + Sync {
    fn connect(
        &self,
        req: &NormalizedRequest,
    ) -> Result<InstallReceipt, Box<dyn std::error::Error + Send + Sync>>;
}

/// The approval seam between plan and apply.
pub trait InstallApprover: Send + Sync {
    fn approve(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// What one connect returned (Go `MCPConnectResult`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallReceipt {
    pub tool_count: usize,
}

/// The `install_source` tool card.
#[derive(Clone)]
pub struct InstallSourceTool {
    connector: Option<Arc<dyn InstallConnector>>,
    approver: Option<Arc<dyn InstallApprover>>,
    require_approved_plan: bool,
}

impl InstallSourceTool {
    /// Builds the card. `require_approved_plan` mirrors boot: on for the model,
    /// off for hosts whose reaching the tool already was the user's decision.
    pub fn new(
        connector: Option<Arc<dyn InstallConnector>>,
        approver: Option<Arc<dyn InstallApprover>>,
        require_approved_plan: bool,
    ) -> Self {
        Self {
            connector,
            approver,
            require_approved_plan,
        }
    }
}

impl Tool for InstallSourceTool {
    fn name(&self) -> &str {
        INSTALL_SOURCE_TOOL_NAME
    }

    fn description(&self) -> &str {
        INSTALL_SOURCE_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(INSTALL_SOURCE_SCHEMA)
            .expect("the install_source schema is valid JSON")
    }

    fn execute(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let req: InstallRequest = serde_json::from_value(args.clone())
            .map_err(|e| invalid_err(format!("install_source: invalid args: {e}")))?;
        let normalized = validate_install_request(&req)?;
        if unticketed_apply(
            normalized.apply,
            self.require_approved_plan,
            &normalized.plan_id,
        ) {
            // Unticketed apply answers with the plan instead of an install; the
            // real plan body is produced by the connector seam in the integration
            // slice. This slice returns the planned-status receipt shape.
            let wire = json!({"ok": false, "status": "planned", "applied": false, "op": normalized.op, "planId": ""});
            return Ok(wire.to_string());
        }
        let Some(connector) = &self.connector else {
            return Err(Box::new(InstallError::SeamUnwired));
        };
        if let Some(approver) = &self.approver {
            approver.approve()?;
        }
        let receipt = connector.connect(&normalized)?;
        let wire = json!({"ok": true, "applied": true, "toolCount": receipt.tool_count});
        Ok(wire.to_string())
    }

    fn read_only(&self) -> bool {
        false
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn invalid_err(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(std::io::Error::other(message.into()))
}
