//! TOOL-035: the misc built-ins (docs/contracts/tools.md §2.12):
//! `present_files` and `system_one`.
//!
//! Go oracles: `internal/tools/builtin/present_files.go`,
//! `internal/tools/builtin/systemone.go`. The model-visible card — name,
//! verbatim description, schema, read-only classification — is frozen here.
//! The host-backed resolution (workspace file service, System One decision
//! client) stays behind a seam the integration slice binds; this slice ships
//! the argument contract and the pure selection behavior.

#![forbid(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use crate::abi::{
    CallContext, Capability, CapabilitySet, ContextualReasoner, ContextualTool, PlanModeClassifier,
    Tool, ToolResult,
};
use crate::refusal::Refusal;

/// `present_files`, the delivery card tool name (Go `delivery.ToolName`).
pub const PRESENT_FILES_TOOL_NAME: &str = "present_files";

/// Go `presentFiles.Description()`, verbatim.
pub const PRESENT_FILES_DESCRIPTION: &str = "Present existing final files as clickable cards, including shell-created documents. Paths must be in the workspace or authorized folders. Opens current files, not snapshots; does not certify correctness.";

/// Go `presentFiles.Schema()`, verbatim.
const PRESENT_FILES_SCHEMA: &str = r#"{"type":"object","properties":{"files":{"type":"array","minItems":1,"maxItems":4,"items":{"type":"object","properties":{"path":{"type":"string","maxLength":4096},"description":{"type":"string","maxLength":256}},"required":["path"],"additionalProperties":false}}},"required":["files"],"additionalProperties":false}"#;

/// Go `delivery.MaxFiles`: a card carries at most four files.
pub const MAX_PRESENT_FILES: usize = 4;
/// Go execute-side path ceiling (schema `maxLength:4096`).
const MAX_PATH_LEN: usize = 4096;
/// Go execute-side description ceiling (the schema advertises 256; the
/// execution check keeps the looser 1024 the Go source enforces).
const MAX_PRESENT_DESCRIPTION_LEN: usize = 1024;
/// Go execute-side delivery body ceiling: 32 KiB.
const MAX_PRESENT_BYTES: usize = 32 << 10;

/// One file the card presents (Go `delivery.File`; mirrors
/// `reasonix_contract::delivery::File`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PresentedFile {
    pub path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub root: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// The delivery card payload the host renders (Go `delivery.Result`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PresentResult {
    pub version: u32,
    pub files: Vec<PresentedFile>,
}

/// Resolves one model-supplied path against the workspace and read roots, and
/// returns the presentation record. The host owns the workspacefile service and
/// the read-target access check this seam replaces.
pub trait PresentFilesHost: Send + Sync {
    fn present(
        &self,
        path: &str,
    ) -> Result<PresentedFile, Box<dyn std::error::Error + Send + Sync>>;
}

#[derive(Deserialize, Default)]
struct PresentInput {
    #[serde(default)]
    files: Vec<PresentInputFile>,
}

#[derive(Deserialize, Default)]
struct PresentInputFile {
    #[serde(default)]
    path: String,
    #[serde(default)]
    description: String,
}

/// The `present_files` tool.
#[derive(Clone, Default)]
pub struct PresentFilesTool {
    host: Option<Arc<dyn PresentFilesHost>>,
}

impl PresentFilesTool {
    /// Binds the optional host seam. `None` keeps the card registered; the
    /// execution path reports the seam as unwired until the integration slice
    /// provides one.
    pub fn new(host: Option<Arc<dyn PresentFilesHost>>) -> Self {
        Self { host }
    }
}

/// The one instance a composition root registers, when no host seam is bound.
pub fn present_files_tool() -> Arc<dyn Tool> {
    Arc::new(PresentFilesTool::default())
}

impl Tool for PresentFilesTool {
    fn name(&self) -> &str {
        PRESENT_FILES_TOOL_NAME
    }

    fn description(&self) -> &str {
        PRESENT_FILES_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(PRESENT_FILES_SCHEMA).expect("the present_files schema is valid JSON")
    }

    fn execute(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let input: PresentInput = serde_json::from_value(args.clone())
            .map_err(|e| Box::new(std::io::Error::other(format!("invalid args: {e}"))))?;
        if input.files.is_empty() || input.files.len() > MAX_PRESENT_FILES {
            return Err(Box::new(std::io::Error::other(format!(
                "present_files requires 1 to {MAX_PRESENT_FILES} files"
            ))));
        }
        for file in &input.files {
            if file.path.len() > MAX_PATH_LEN
                || file.description.len() > MAX_PRESENT_DESCRIPTION_LEN
            {
                return Err(Box::new(std::io::Error::other(
                    "file path or description is too long",
                )));
            }
        }
        let Some(host) = &self.host else {
            // TODO(wave39-b): the workspacefile service and read-target access
            // check live in the host assembly; this slice ships the card and
            // argument contract only.
            return Err(Box::new(std::io::Error::other(
                "present_files host seam is not wired in this slice",
            )));
        };
        let mut files = Vec::with_capacity(input.files.len());
        for file in &input.files {
            let mut resolved = host.present(&file.path)?;
            if resolved.description.is_empty() {
                resolved.description = file.description.clone();
            }
            files.push(resolved);
        }
        let result = PresentResult { version: 1, files };
        let body = serde_json::to_string(&result)
            .map_err(|e| Box::new(std::io::Error::other(format!("encode present result: {e}"))))?;
        if body.len() > MAX_PRESENT_BYTES {
            return Err(Box::new(std::io::Error::other(
                "file declaration exceeds 32 KiB",
            )));
        }
        Ok(body)
    }

    fn read_only(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `system_one`, the decision-backend tool name.
pub const SYSTEM_ONE_TOOL_NAME: &str = "system_one";

/// `decision.not_configured` — no model is assigned to the decision role.
pub const CODE_SYSTEM_ONE_NOT_CONFIGURED: &str = "decision.not_configured";

/// Go `localBackend`: the built-in local decision client.
pub const LOCAL_BACKEND: &str = "laya-local";

/// The configured decision backends the model may ask for (Go `SystemOneSpec`
/// reduced to the surface this slice owns). `endpoint` is the named provider
/// entry; `local` marks the built-in local client present.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SystemOneSpec {
    pub endpoint: Option<String>,
    pub local: bool,
}

impl SystemOneSpec {
    /// Backends by the names settings shows: the configured endpoint first,
    /// then the local client. An option the model can read but never reach is
    /// a refusal it has to discover.
    pub fn backends(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(endpoint) = self
            .endpoint
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            out.push(endpoint.to_owned());
        }
        if self.local {
            out.push(LOCAL_BACKEND.to_owned());
        }
        out
    }

    /// Whether any decision backend is configured at all (Go
    /// `SystemOneConfigured`).
    pub fn configured(&self) -> bool {
        !self.backends().is_empty()
    }

    /// Picks the requested backend, defaulting to the single configured one
    /// when the request is empty.
    pub fn select_backend(&self, requested: &str) -> Result<String, SystemOneError> {
        let available = self.backends();
        let requested = requested.trim();
        if requested.is_empty() && available.len() == 1 {
            return Ok(available[0].clone());
        }
        if let Some(matched) = available.iter().find(|b| b == &requested) {
            return Ok(matched.clone());
        }
        if requested.is_empty() {
            return Err(SystemOneError::BackendRequired {
                available: available.join(", "),
            });
        }
        Err(SystemOneError::BackendNotConfigured {
            requested: requested.to_owned(),
            available: available.join(", "),
        })
    }
}

/// Pure backend selection failure (Go `systemOne.backend` errors).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SystemOneError {
    /// No backend was requested and more than one is available.
    #[error("system_one backend is required; available: {available}")]
    BackendRequired { available: String },
    /// The requested backend is not among the configured ones.
    #[error("system_one backend {requested:?} is not configured; available: {available}")]
    BackendNotConfigured {
        requested: String,
        available: String,
    },
}

/// Evaluates one decision request against a configured backend (Go the
/// `typesafe.Client` / `laya.LocalClient` pair). The real decision clients
/// live in the provider slice; this seam binds them at assembly time.
pub trait SystemOneEvaluator: Send + Sync {
    fn evaluate(
        &self,
        backend: &str,
        state: &Value,
        questions: &Value,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>>;
}

/// Go `systemOne.Description()` with the dynamic backend list filled in.
pub fn system_one_description(spec: &SystemOneSpec) -> String {
    format!(
        "Evaluate state with the configured System One decision backend ({}). Ask typed Noul, Choice, or Score questions and receive complete probabilities, confidence, routing metadata, and usage.",
        spec.backends().join(", ")
    )
}

/// Go `systemOne.Schema()` with the backend enum filled in from the spec.
pub fn system_one_schema(spec: &SystemOneSpec) -> Value {
    let enum_backends = spec.backends();
    serde_json::json!({
        "type": "object",
        "properties": {
            "backend": {
                "type": "string",
                "enum": enum_backends,
                "description": "Configured decision backend; optional only when exactly one is available"
            },
            "state": { "description": "Text or structured JSON state to evaluate" },
            "questions": {
                "type": "object",
                "description": "Named questions; answer keys match these ids",
                "additionalProperties": {
                    "type": "object",
                    "properties": {
                        "type": { "type": "string", "enum": ["noul", "choice", "score"] },
                        "instructions": { "description": "Question text or structured instructions" },
                        "criteria": { "description": "Noul: optional true/false object; Choice: 2-255 option map; Score: ordered array of 2-10 levels" }
                    },
                    "required": ["type", "instructions"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["state", "questions"],
        "additionalProperties": false
    })
}

#[derive(Deserialize, Default)]
struct SystemOneInput {
    #[serde(default)]
    backend: String,
    #[serde(default)]
    state: Value,
    #[serde(default)]
    questions: Value,
}

struct PlanSafe;
impl Capability for PlanSafe {}
impl PlanModeClassifier for PlanSafe {
    fn plan_mode_safe(&self) -> bool {
        true
    }
}

/// Contextual declaration carried by the tool's own spec (Go
/// `systemOne.ProviderVisible` / `Unavailable`).
struct SystemOneContextual {
    configured: bool,
}
impl Capability for SystemOneContextual {}
impl ContextualTool for SystemOneContextual {
    fn provider_visible(&self, _ctx: &CallContext) -> bool {
        self.configured
    }
}

struct SystemOneReasoner {
    configured: bool,
}
impl Capability for SystemOneReasoner {}
impl ContextualTool for SystemOneReasoner {
    fn provider_visible(&self, _ctx: &CallContext) -> bool {
        self.configured
    }
}
impl ContextualReasoner for SystemOneReasoner {
    fn unavailable(&self, _ctx: &CallContext) -> Refusal {
        system_one_not_configured()
    }
}

/// The frozen not-configured refusal (Go `systemOne.Unavailable`).
pub fn system_one_not_configured() -> Refusal {
    Refusal::code(
        CODE_SYSTEM_ONE_NOT_CONFIGURED,
        "no model is assigned to the decision role",
    )
}

/// The `system_one` tool.
#[derive(Clone, Default)]
pub struct SystemOneTool {
    spec: SystemOneSpec,
    evaluator: Option<Arc<dyn SystemOneEvaluator>>,
    description: String,
}

impl SystemOneTool {
    /// Builds the tool over a configured spec and optional evaluator seam.
    pub fn new(spec: SystemOneSpec, evaluator: Option<Arc<dyn SystemOneEvaluator>>) -> Self {
        let description = system_one_description(&spec);
        Self {
            spec,
            evaluator,
            description,
        }
    }

    /// The configured backends, exposed for the host's visibility check.
    pub fn backends(&self) -> Vec<String> {
        self.spec.backends()
    }
}

impl Tool for SystemOneTool {
    fn name(&self) -> &str {
        SYSTEM_ONE_TOOL_NAME
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> Value {
        system_one_schema(&self.spec)
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        if !self.spec.configured() {
            return Err(Box::new(system_one_not_configured()));
        }
        let input: SystemOneInput = serde_json::from_value(args.clone())
            .map_err(|e| Box::new(std::io::Error::other(format!("invalid args: {e}"))))?;
        let backend = self.spec.select_backend(&input.backend)?;
        let Some(evaluator) = &self.evaluator else {
            // TODO(wave39-b): the typesafe/laya clients land with the provider
            // slice; this slice ships selection and the card only.
            return Err(Box::new(std::io::Error::other(
                "system_one evaluator seam is not wired in this slice",
            )));
        };
        let _ = ctx;
        evaluator.evaluate(&backend, &input.state, &input.questions)
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> CapabilitySet {
        let configured = self.spec.configured();
        CapabilitySet::default()
            .with(
                "plan_mode_safe",
                Arc::new(PlanSafe) as Arc<dyn PlanModeClassifier>,
            )
            .with(
                "contextual",
                Arc::new(SystemOneContextual { configured }) as Arc<dyn ContextualTool>,
            )
            .with(
                "contextual_reasoner",
                Arc::new(SystemOneReasoner { configured }) as Arc<dyn ContextualReasoner>,
            )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
