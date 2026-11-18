//! Agent composition contract (Go: internal/contract/agentcomposition).
//!
//! Wire shapes, digests, live bindings, parameter discipline and execution
//! authority are frozen in docs/contracts/provider-message.md section 6.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest as _, Sha256};

use reasonix_core::secrets::{redact_args, redact_endpoint};

use crate::gojson::{
    go_opt_vec, go_raw, go_struct, is_zero_bool, is_zero_i64, marshal, null_default,
};

/// Definition schema recognized by this kernel (Go: schemaVersion const).
pub const SCHEMA_VERSION: i64 = 1;
/// Selection mode that keeps the ambient module set (Go: Inherit).
pub const INHERIT: &str = "inherit";
/// Selection mode that replaces the module set (Go: Select).
pub const SELECT: &str = "select";
/// Format version of a live resource binding (Go: liveBindingVersion).
pub const LIVE_BINDING_VERSION: i64 = 2;
/// Resource mode of a live resource binding (Go: liveResources).
pub const LIVE_RESOURCES: &str = "live";

/// Errors surfaced by composition decoding and validation. The texts mirror
/// the Go sentinel errors; Invalid carries the wrapped cause, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompositionError {
    /// Go ErrInvalid, optionally wrapped with a short cause.
    Invalid { detail: Option<Cow<'static, str>> },
    /// Go ErrCompositionUnavailable.
    Unavailable,
    /// Go ErrCompositionLocked.
    Locked,
    /// Go ErrCompositionWorkspaceBound.
    WorkspaceBound,
}

impl CompositionError {
    /// Bare invalid composition (Go: ErrInvalid itself).
    pub fn invalid() -> Self {
        CompositionError::Invalid { detail: None }
    }

    /// Invalid composition with a cause
    /// (Go: fmt.Errorf with %w and a cause string).
    pub fn invalid_detail(detail: impl Into<Cow<'static, str>>) -> Self {
        CompositionError::Invalid {
            detail: Some(detail.into()),
        }
    }

    /// True when this error is (or wraps) Go ErrInvalid.
    pub fn is_invalid(&self) -> bool {
        matches!(self, CompositionError::Invalid { .. })
    }
}

impl fmt::Display for CompositionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompositionError::Invalid {
                detail: Some(detail),
            } => {
                write!(f, "invalid agent composition: {detail}")
            }
            CompositionError::Invalid { detail: None } => write!(f, "invalid agent composition"),
            CompositionError::Unavailable => write!(f, "agent composition unavailable"),
            CompositionError::Locked => write!(f, "agent composition locked"),
            CompositionError::WorkspaceBound => {
                write!(f, "agent composition is bound to its workspace")
            }
        }
    }
}

impl std::error::Error for CompositionError {}

go_struct! {
    /// Persona prompt fragments (Go: composition.go Persona).
    #[derive(PartialEq)]
    pub struct Persona : PersonaWire {
        #[serde(rename = "prefix", default, deserialize_with = "null_default")]
        pub prefix: String,
        #[serde(rename = "suffix", default, deserialize_with = "null_default")]
        pub suffix: String,
        #[serde(rename = "include_runtime_context", default, deserialize_with = "null_default")]
        pub include_runtime_context: bool,
    }
}

go_struct! {
    /// One selected module inside a selection (Go: composition.go Module).
    pub struct Module : ModuleWire {
        #[serde(rename = "id", default, deserialize_with = "null_default")]
        pub id: String,
        #[serde(
            rename = "parameters",
            default,
            deserialize_with = "go_raw",
            skip_serializing_if = "Option::is_none"
        )]
        pub parameters: Option<Box<RawValue>>,
        #[serde(
            rename = "optional",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_bool"
        )]
        pub optional: bool,
    }
}

go_struct! {
    /// A module group selection (Go: composition.go Selection).
    pub struct Selection : SelectionWire {
        #[serde(rename = "mode", default, deserialize_with = "null_default")]
        pub mode: String,
        #[serde(rename = "members", default, deserialize_with = "go_opt_vec")]
        pub members: Option<Vec<Module>>,
    }
}

go_struct! {
    /// A whole composition document (Go: composition.go Definition).
    pub struct Definition : DefinitionWire {
        #[serde(rename = "schemaVersion", default, deserialize_with = "null_default")]
        pub schema_version: i64,
        #[serde(rename = "id", default, deserialize_with = "null_default")]
        pub id: String,
        #[serde(rename = "name", default, deserialize_with = "null_default")]
        pub name: String,
        #[serde(rename = "description", default, deserialize_with = "null_default")]
        pub description: String,
        #[serde(rename = "persona", default, deserialize_with = "null_default")]
        pub persona: Persona,
        #[serde(rename = "tools", default, deserialize_with = "null_default")]
        pub tools: Selection,
        #[serde(rename = "skills", default, deserialize_with = "null_default")]
        pub skills: Selection,
        #[serde(rename = "mcp", default, deserialize_with = "null_default")]
        pub mcp: Selection,
        #[serde(rename = "extensions", default, deserialize_with = "null_default")]
        pub extensions: Selection,
        #[serde(rename = "hooks", default, deserialize_with = "null_default")]
        pub hooks: Selection,
        #[serde(rename = "commands", default, deserialize_with = "null_default")]
        pub commands: Selection,
    }
}

go_struct! {
    /// A pinned definition revision (Go: composition.go Version).
    pub struct Version : VersionWire {
        #[serde(rename = "definition", default, deserialize_with = "null_default")]
        pub definition: Definition,
        #[serde(rename = "revision", default, deserialize_with = "null_default")]
        pub revision: String,
    }
}

go_struct! {
    /// One retained live resource (Go: composition.go Resource). Data is raw
    /// JSON: absent data fails validation (Go json.Valid of nil is false)
    /// while explicit null passes; both marshal as data:null.
    pub struct Resource : ResourceWire {
        #[serde(rename = "id", default, deserialize_with = "null_default")]
        pub id: String,
        #[serde(rename = "kind", default, deserialize_with = "null_default")]
        pub kind: String,
        #[serde(rename = "data", default, deserialize_with = "go_raw")]
        pub data: Option<Box<RawValue>>,
    }
}

go_struct! {
    /// A sealed composition binding (Go: composition.go Binding).
    pub struct Binding : BindingWire {
        #[serde(
            rename = "formatVersion",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_i64"
        )]
        pub format_version: i64,
        #[serde(
            rename = "resourceMode",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub resource_mode: String,
        #[serde(rename = "resolved", default, deserialize_with = "null_default")]
        pub resolved: Definition,
        #[serde(
            rename = "started",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "is_zero_bool"
        )]
        pub started: bool,
        #[serde(rename = "version", default, deserialize_with = "null_default")]
        pub version: Version,
        #[serde(rename = "digest", default, deserialize_with = "null_default")]
        pub digest: String,
        #[serde(
            rename = "workspace",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub workspace: String,
        #[serde(rename = "resources", default, deserialize_with = "go_opt_vec")]
        pub resources: Option<Vec<Resource>>,
        #[serde(
            rename = "systemPrompt",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub system_prompt: String,
        #[serde(
            rename = "toolSchemaDigest",
            default,
            deserialize_with = "null_default",
            skip_serializing_if = "String::is_empty"
        )]
        pub tool_schema_digest: String,
        #[serde(rename = "toolSurface", default, deserialize_with = "go_opt_vec")]
        pub tool_surface: Option<Vec<String>>,
    }
}

impl Binding {
    /// Projects the binding to its persisted summary (Go: Binding.Summary;
    /// the Go nil-receiver case maps to None at the call site, where the
    /// enclosing Option already carries it).
    pub fn summary(&self) -> Summary {
        Summary {
            id: self.version.definition.id.clone(),
            name: self.version.definition.name.clone(),
            revision: self.version.revision.clone(),
            digest: self.digest.clone(),
            started: self.started,
            format_version: self.format_version,
            resource_mode: self.resource_mode.clone(),
        }
    }
}

/// The bounded projection of a [Binding] used by listings.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Summary {
    #[serde(rename = "id", default, deserialize_with = "null_default")]
    pub id: String,
    #[serde(rename = "name", default, deserialize_with = "null_default")]
    pub name: String,
    #[serde(rename = "revision", default, deserialize_with = "null_default")]
    pub revision: String,
    #[serde(rename = "digest", default, deserialize_with = "null_default")]
    pub digest: String,
    #[serde(rename = "started", default, deserialize_with = "null_default")]
    pub started: bool,
    #[serde(
        rename = "formatVersion",
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero_i64"
    )]
    pub format_version: i64,
    #[serde(
        rename = "resourceMode",
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub resource_mode: String,
}

/// A single validation or resolution failure of a composition.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Diagnostic {
    #[serde(rename = "code", default, deserialize_with = "null_default")]
    pub code: String,
    #[serde(
        rename = "module",
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub module: String,
    #[serde(rename = "detail", default, deserialize_with = "null_default")]
    pub detail: String,
}

/// Carries module-level diagnostics across the availability boundary so a
/// caller can attribute an unavailability to its modules (Go:
/// DiagnosticError).
#[derive(Debug)]
pub struct DiagnosticError {
    pub diagnostics: Vec<Diagnostic>,
}

impl fmt::Display for DiagnosticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", CompositionError::Unavailable)?;
        for d in &self.diagnostics {
            write!(f, "; {}: {}", d.module, d.detail)?;
        }
        Ok(())
    }
}

impl std::error::Error for DiagnosticError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        static UNAVAILABLE: CompositionError = CompositionError::Unavailable;
        Some(&UNAVAILABLE)
    }
}

/// Builds the single-diagnostic availability error (Go: Unavailable).
pub fn unavailable(module: &str, detail: &str) -> DiagnosticError {
    DiagnosticError {
        diagnostics: vec![Diagnostic {
            code: "agent_composition.unavailable".to_owned(),
            module: module.to_owned(),
            detail: detail.to_owned(),
        }],
    }
}

/// Declares a strict decode type: a wire struct with Go
/// json.Decoder.DisallowUnknownFields semantics (unknown keys are errors)
/// behind a map-only outer guard so JSON arrays are rejected exactly like
/// Go unmarshalling into a struct.
macro_rules! strict_struct {
    (
        $(#[$outer:meta])*
        $name:ident : $wire:ident {
            $( $(#[$fmeta:meta])* $field:ident : $ty:ty ),* $(,)?
        }
    ) => {
        $(#[$outer])*
        #[derive(Default, serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct $wire {
            $( $(#[$fmeta])* $field: $ty, )*
        }

        #[derive(Default)]
        struct $name {
            $( $field: $ty, )*
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct V;
                impl<'de> serde::de::Visitor<'de> for V {
                    type Value = $name;
                    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(f, "struct {} (a JSON object)", stringify!($name))
                    }
                    fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
                    where
                        A: serde::de::MapAccess<'de>,
                    {
                        let wire = <$wire as serde::Deserialize<'de>>::deserialize(
                            serde::de::value::MapAccessDeserializer::new(map),
                        )?;
                        Ok($name { $( $field: wire.$field, )* })
                    }
                }
                deserializer.deserialize_map(V)
            }
        }
    };
}

strict_struct! {
    /// Strict persona decode (Go: json.Decoder DisallowUnknownFields).
    PersonaStrict : PersonaStrictWire {
        #[serde(rename = "prefix", default, deserialize_with = "null_default")]
        prefix: String,
        #[serde(rename = "suffix", default, deserialize_with = "null_default")]
        suffix: String,
        #[serde(rename = "include_runtime_context", default, deserialize_with = "null_default")]
        include_runtime_context: bool,
    }
}

strict_struct! {
    /// Strict module decode (Go: json.Decoder DisallowUnknownFields).
    ModuleStrict : ModuleStrictWire {
        #[serde(rename = "id", default, deserialize_with = "null_default")]
        id: String,
        #[serde(rename = "parameters", default, deserialize_with = "go_raw")]
        parameters: Option<Box<RawValue>>,
        #[serde(rename = "optional", default, deserialize_with = "null_default")]
        optional: bool,
    }
}

strict_struct! {
    /// Strict selection decode (Go: json.Decoder DisallowUnknownFields).
    SelectionStrict : SelectionStrictWire {
        #[serde(rename = "mode", default, deserialize_with = "null_default")]
        mode: String,
        #[serde(rename = "members", default, deserialize_with = "go_opt_vec")]
        members: Option<Vec<ModuleStrict>>,
    }
}

strict_struct! {
    /// Strict definition decode (Go: json.Decoder DisallowUnknownFields).
    DefinitionStrict : DefinitionStrictWire {
        #[serde(rename = "schemaVersion", default, deserialize_with = "null_default")]
        schema_version: i64,
        #[serde(rename = "id", default, deserialize_with = "null_default")]
        id: String,
        #[serde(rename = "name", default, deserialize_with = "null_default")]
        name: String,
        #[serde(rename = "description", default, deserialize_with = "null_default")]
        description: String,
        #[serde(rename = "persona", default, deserialize_with = "null_default")]
        persona: PersonaStrict,
        #[serde(rename = "tools", default, deserialize_with = "null_default")]
        tools: SelectionStrict,
        #[serde(rename = "skills", default, deserialize_with = "null_default")]
        skills: SelectionStrict,
        #[serde(rename = "mcp", default, deserialize_with = "null_default")]
        mcp: SelectionStrict,
        #[serde(rename = "extensions", default, deserialize_with = "null_default")]
        extensions: SelectionStrict,
        #[serde(rename = "hooks", default, deserialize_with = "null_default")]
        hooks: SelectionStrict,
        #[serde(rename = "commands", default, deserialize_with = "null_default")]
        commands: SelectionStrict,
    }
}

strict_struct! {
    /// Strict parameters decode (Go: json.Decoder DisallowUnknownFields).
    ParametersStrict : ParametersStrictWire {
        #[serde(rename = "args", default)]
        args: Option<Vec<String>>,
        #[serde(rename = "env", default)]
        env: Option<BTreeMap<String, String>>,
        #[serde(rename = "headers", default)]
        headers: Option<BTreeMap<String, String>>,
        #[serde(rename = "url", default)]
        url: Option<String>,
        #[serde(rename = "callTimeoutSeconds", default)]
        call_timeout_seconds: Option<i64>,
        #[serde(rename = "startupTimeoutSeconds", default)]
        startup_timeout_seconds: Option<i64>,
        #[serde(rename = "timeoutMillis", default)]
        timeout_millis: Option<i64>,
    }
}

impl From<PersonaStrict> for Persona {
    fn from(w: PersonaStrict) -> Self {
        Persona {
            prefix: w.prefix,
            suffix: w.suffix,
            include_runtime_context: w.include_runtime_context,
        }
    }
}

impl From<ModuleStrict> for Module {
    fn from(w: ModuleStrict) -> Self {
        Module {
            id: w.id,
            parameters: w.parameters,
            optional: w.optional,
        }
    }
}

impl From<SelectionStrict> for Selection {
    fn from(w: SelectionStrict) -> Self {
        Selection {
            mode: w.mode,
            members: w
                .members
                .map(|members| members.into_iter().map(Module::from).collect()),
        }
    }
}

impl From<DefinitionStrict> for Definition {
    fn from(w: DefinitionStrict) -> Self {
        Definition {
            schema_version: w.schema_version,
            id: w.id,
            name: w.name,
            description: w.description,
            persona: w.persona.into(),
            tools: w.tools.into(),
            skills: w.skills.into(),
            mcp: w.mcp.into(),
            extensions: w.extensions.into(),
            hooks: w.hooks.into(),
            commands: w.commands.into(),
        }
    }
}

/// A module's runtime parameter overrides (Go: parameters.go Parameters).
/// The Go pointers become Options so explicit null and absent stay distinct.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Parameters {
    #[serde(rename = "args", skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(rename = "env", skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
    #[serde(rename = "headers", skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,
    #[serde(rename = "url", skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(rename = "callTimeoutSeconds", skip_serializing_if = "Option::is_none")]
    pub call_timeout_seconds: Option<i64>,
    #[serde(
        rename = "startupTimeoutSeconds",
        skip_serializing_if = "Option::is_none"
    )]
    pub startup_timeout_seconds: Option<i64>,
    #[serde(rename = "timeoutMillis", skip_serializing_if = "Option::is_none")]
    pub timeout_millis: Option<i64>,
}

impl From<ParametersStrict> for Parameters {
    fn from(w: ParametersStrict) -> Self {
        Parameters {
            args: w.args,
            env: w.env,
            headers: w.headers,
            url: w.url,
            call_timeout_seconds: w.call_timeout_seconds,
            startup_timeout_seconds: w.startup_timeout_seconds,
            timeout_millis: w.timeout_millis,
        }
    }
}

/// Digest of any JSON-serializable value, computed the way Go Digest does:
/// marshal, decode into a generic value (normalizing away key order and
/// number formats), marshal again, SHA-256, lowercase hex. Any failure
/// yields the empty string, matching Go.
pub fn digest<T>(value: &T) -> String
where
    T: ?Sized + Serialize,
{
    let Ok(first) = marshal(value) else {
        return String::new();
    };
    let Ok(normalized) = serde_json::from_str::<serde_json::Value>(&first) else {
        return String::new();
    };
    let Ok(second) = marshal(&normalized) else {
        return String::new();
    };
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let sum = Sha256::digest(second.as_bytes());
    let mut out = String::with_capacity(sum.len() * 2);
    for byte in sum {
        out.push(HEX[usize::from(byte >> 4)] as char);
        out.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    out
}

/// Canonical revision of a definition (Go: Revision).
pub fn revision(d: &Definition) -> String {
    digest(d)
}

/// Decodes the embedded builtin catalog, ignoring errors like Go
/// (Builtins over go:embed).
pub fn builtins() -> Vec<Definition> {
    serde_json::from_str::<Vec<Definition>>(include_str!("builtins.json")).unwrap_or_default()
}

impl Selection {
    /// True when the selection keeps or explicitly lists the module
    /// (Go: Selection.Allows).
    pub fn allows(&self, id: &str) -> bool {
        self.mode == INHERIT
            || (self.mode == SELECT
                && self
                    .members
                    .as_deref()
                    .unwrap_or(&[])
                    .iter()
                    .any(|m| m.id == id))
    }
}

impl Definition {
    /// Structural validation (Go: Definition.Validate).
    pub fn validate(&self) -> Result<(), CompositionError> {
        if self.schema_version != SCHEMA_VERSION
            || !is_identifier(&self.id)
            || self.name.trim().is_empty()
            || self.name.len() > 1024
            || self.description.len() > 16384
        {
            return Err(CompositionError::invalid_detail(
                "schemaVersion, id or name",
            ));
        }
        if self.persona.prefix.len() > 32768 || self.persona.suffix.len() > 32768 {
            return Err(CompositionError::invalid_detail("persona"));
        }
        let groups = [
            &self.tools,
            &self.skills,
            &self.mcp,
            &self.extensions,
            &self.hooks,
            &self.commands,
        ];
        for (group, selection) in groups.into_iter().enumerate() {
            let members = selection.members.as_deref().unwrap_or(&[]);
            if members.len() > 10000 {
                return Err(CompositionError::invalid());
            }
            if selection.mode != INHERIT && selection.mode != SELECT
                || selection.mode == INHERIT && !members.is_empty()
            {
                return Err(CompositionError::invalid_detail("selection mode"));
            }
            let mut seen: HashSet<&str> = HashSet::new();
            for member in members {
                if member.optional && group != 0 {
                    return Err(CompositionError::invalid_detail(
                        "optional is only supported for tools",
                    ));
                }
                if member.id.is_empty() || member.id.len() > 1024 || !seen.insert(&member.id) {
                    return Err(CompositionError::invalid_detail(format!(
                        "duplicate or empty module {}",
                        go_quote(&member.id)
                    )));
                }
                if let Some(parameters) = &member.parameters {
                    let parsed = serde_json::from_str::<
                        Option<serde_json::Map<String, serde_json::Value>>,
                    >(parameters.get());
                    if !parsed.is_ok_and(|m| m.is_some()) {
                        return Err(CompositionError::invalid_detail("module parameters"));
                    }
                }
            }
        }
        self.validate_module_parameters()
    }

    /// Group-specific parameter rules (Go: validateModuleParameters).
    fn validate_module_parameters(&self) -> Result<(), CompositionError> {
        for selection in [&self.mcp, &self.extensions] {
            for member in selection.members.as_deref().unwrap_or(&[]) {
                parse_parameters(member.parameters.as_deref())?;
            }
        }
        for member in self.extensions.members.as_deref().unwrap_or(&[]) {
            let p = parse_parameters(member.parameters.as_deref())?;
            if p.url.is_some()
                || p.headers.is_some()
                || p.call_timeout_seconds.is_some()
                || p.startup_timeout_seconds.is_some()
            {
                return Err(CompositionError::invalid_detail(
                    "unsupported extension parameter",
                ));
            }
        }
        for member in self.mcp.members.as_deref().unwrap_or(&[]) {
            let p = parse_parameters(member.parameters.as_deref())?;
            if p.timeout_millis.is_some() {
                return Err(CompositionError::invalid_detail(
                    "unsupported MCP parameter",
                ));
            }
        }
        for selection in [&self.tools, &self.skills, &self.hooks, &self.commands] {
            for member in selection.members.as_deref().unwrap_or(&[]) {
                if let Some(parameters) = &member.parameters
                    && !parameters.get().is_empty()
                    && parameters.get() != "{}"
                {
                    return Err(CompositionError::invalid_detail(format!(
                        "parameters unsupported for module {}",
                        member.id
                    )));
                }
            }
        }
        Ok(())
    }

    /// Validation plus project-source authority
    /// (Go: Definition.ValidateForExecution).
    pub fn validate_for_execution(&self) -> Result<(), CompositionError> {
        self.validate()?;
        if !self.id.starts_with("project:") {
            return Ok(());
        }
        for selection in [&self.mcp, &self.extensions] {
            for module in selection.members.as_deref().unwrap_or(&[]) {
                let p = parse_parameters(module.parameters.as_deref())?;
                if p.args.is_some() || p.url.is_some() || p.env.is_some() || p.headers.is_some() {
                    return Err(CompositionError::invalid_detail(format!(
                        "project module {} may override only timeouts",
                        module.id
                    )));
                }
            }
        }
        Ok(())
    }
}

impl Binding {
    /// True for live-bound compositions (Go: Binding.Live; the Go nil
    /// receiver maps to None at the call site).
    pub fn live(&self) -> bool {
        self.format_version == LIVE_BINDING_VERSION && self.resource_mode == LIVE_RESOURCES
    }

    /// Recomputes the digest with started forced false (Go: Seal) so the
    /// digest never covers the started flag.
    pub fn seal(&mut self) {
        self.digest = String::new();
        let started = self.started;
        self.started = false;
        self.digest = digest(self);
        self.started = started;
    }

    /// Live-resource format rule (Go: validateResourceFormat).
    fn validate_resource_format(&self) -> Result<(), CompositionError> {
        if self.format_version == 0 && self.resource_mode.is_empty() {
            return Ok(());
        }
        if !self.live() || self.resources.as_deref().is_some_and(|r| !r.is_empty()) {
            return Err(CompositionError::invalid_detail(
                "unsupported binding resource format",
            ));
        }
        Ok(())
    }

    /// Structural validation (Go: Binding.Validate).
    pub fn validate(&self) -> Result<(), CompositionError> {
        self.validate_resource_format()?;
        self.version.definition.validate()?;
        if self.version.revision != revision(&self.version.definition) {
            return Err(CompositionError::invalid());
        }
        self.resolved.validate()?;
        if self.resolved.id != self.version.definition.id {
            return Err(CompositionError::invalid());
        }
        let resources = self.resources.as_deref().unwrap_or(&[]);
        let mut total = self.system_prompt.len();
        let mut seen: HashSet<String> = HashSet::new();
        for resource in resources {
            let key = format!("{}:{}", resource.kind, resource.id);
            if resource.id.is_empty()
                || resource.kind.is_empty()
                || seen.contains(&key)
                || !valid_resource_data(resource.data.as_deref())
            {
                return Err(CompositionError::invalid());
            }
            seen.insert(key);
            total += resource_data_len(resource.data.as_deref());
            if total > 128 << 20 || resources.len() > 20000 {
                return Err(CompositionError::invalid_detail("retained resource limit"));
            }
        }
        let mut sealed = self.clone();
        sealed.seal();
        if sealed.digest != self.digest {
            return Err(CompositionError::invalid_detail("binding digest"));
        }
        Ok(())
    }

    /// Validation plus project authority for both definitions
    /// (Go: Binding.ValidateForExecution).
    pub fn validate_for_execution(&self) -> Result<(), CompositionError> {
        self.validate()?;
        self.version.definition.validate_for_execution()?;
        self.resolved.validate_for_execution()
    }
}

/// Strict decode of a definition plus execution validation (Go: Decode).
pub fn decode(data: &[u8]) -> Result<Definition, CompositionError> {
    let mut de = serde_json::Deserializer::from_slice(data);
    let strict = Option::<DefinitionStrict>::deserialize(&mut de)
        .map_err(|e| CompositionError::invalid_detail(e.to_string()))?;
    de.end()
        .map_err(|_| CompositionError::invalid_detail("trailing data"))?;
    let d = Definition::from(strict.unwrap_or_default());
    d.validate_for_execution()?;
    Ok(d)
}

/// Parses and enforces reference discipline on module parameters
/// (Go: ParseParameters). Null raw decodes to the zero value, matching the
/// Go null-into-struct no-op.
pub fn parse_parameters(raw: Option<&RawValue>) -> Result<Parameters, CompositionError> {
    let Some(raw) = raw else {
        return Ok(Parameters::default());
    };
    let text = raw.get();
    if text.is_empty() {
        return Ok(Parameters::default());
    }
    let mut de = serde_json::Deserializer::from_str(text);
    let parsed = Option::<ParametersStrict>::deserialize(&mut de)
        .map_err(|e| CompositionError::invalid_detail(format!("module parameters: {e}")))?;
    let p = Parameters::from(parsed.unwrap_or_default());
    for map in [&p.env, &p.headers] {
        for value in map.iter().flat_map(|m| m.values()) {
            if !is_environment_reference(value) {
                return Err(CompositionError::invalid_detail(
                    "environment and header values must be ${NAME} references",
                ));
            }
        }
    }
    if p.call_timeout_seconds.is_some_and(|v| v < 0)
        || p.startup_timeout_seconds.is_some_and(|v| v < 0)
        || p.timeout_millis.is_some_and(|v| v < 0)
    {
        return Err(CompositionError::invalid_detail("negative timeout"));
    }
    if let Some(args) = &p.args {
        let projected = redact_args(args);
        for (i, arg) in args.iter().enumerate() {
            let (value, inline) = match arg.split_once('=') {
                Some((_, value)) => (value, true),
                None => ("", false),
            };
            if projected[i] != *arg
                && !is_environment_reference(arg)
                && !(inline && is_environment_reference(value))
            {
                return Err(CompositionError::invalid_detail(
                    "credentials require references",
                ));
            }
        }
    }
    if let Some(url) = &p.url
        && !is_environment_reference(url)
        && redact_endpoint(url) != *url
    {
        return Err(CompositionError::invalid_detail(
            "endpoint credentials require references",
        ));
    }
    Ok(p)
}

/// Go json.Valid(r.Data): absent data is invalid (nil fails); captured raw
/// JSON is valid by construction but still re-checked here.
fn valid_resource_data(data: Option<&RawValue>) -> bool {
    data.is_some_and(|d| serde_json::from_str::<serde::de::IgnoredAny>(d.get()).is_ok())
}

fn resource_data_len(data: Option<&RawValue>) -> usize {
    data.map_or(0, |d| d.get().len())
}

/// Go identifier regex ^(?:builtin|user|project):[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$
/// expressed byte-wise (RE2 anchors, so no trailing-newline tolerance).
fn is_identifier(id: &str) -> bool {
    let Some((prefix, rest)) = id.split_once(':') else {
        return false;
    };
    if !matches!(prefix, "builtin" | "user" | "project") {
        return false;
    }
    let bytes = rest.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_alphanumeric() || bytes.len() > 64 {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Go environmentReference regex ^${[a-zA-Z_][a-zA-Z0-9_]*}$.
fn is_environment_reference(s: &str) -> bool {
    let Some(inner) = s.strip_prefix("${") else {
        return false;
    };
    let Some(inner) = inner.strip_suffix('}') else {
        return false;
    };
    let bytes = inner.as_bytes();
    if bytes.is_empty() || !(bytes[0].is_ascii_alphabetic() || bytes[0] == b'_') {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Go %q (strconv.Quote) for the duplicate-module error detail: double
/// quotes, ASCII specials, \xNN for other ASCII control bytes.
fn go_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{b}' => out.push_str("\\v"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c if c.is_control() => {
                let n = c as u32;
                if n < 0x10000 {
                    out.push_str(&format!("\\u{n:04x}"));
                } else {
                    out.push_str(&format!("\\U{n:08x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
