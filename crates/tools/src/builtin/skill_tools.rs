//! TOOL-040: skill tool cards and the five dedicated sub-agent wrappers
//! (docs/contracts/tools.md §3.5, Go `internal/ext/skill/tools.go`).
//!
//! This file holds the four tool implementations and the built-in wrapper
//! registry; records, seams and rendering helpers live in `super::skill`.

#![forbid(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use serde_json::Value;

use super::skill::*;
use crate::abi::{CallContext, Capability, CapabilitySet, PlanModeClassifier, Tool, ToolResult};

const READ_ONLY_TASK_NOTICE: &str = "read-only";

// ---------------------------------------------------------------------------
// Invocation argument parsing shared by the four tools
// ---------------------------------------------------------------------------

#[derive(Default)]
struct InvocationArgs {
    name: String,
    arguments: String,
    continue_from: String,
    fork_from: String,
}

fn parse_invocation(args: &Value, accept_continuation: bool) -> Result<InvocationArgs, String> {
    let root = args
        .as_object()
        .ok_or_else(|| "invalid args: not an object".to_owned())?;
    let allowed: &[&str] = if accept_continuation {
        &["name", "arguments", "continue_from", "fork_from"]
    } else {
        &["name", "arguments"]
    };
    for key in root.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!("invalid args: unknown field {key:?}"));
        }
    }
    let name = root.get("name").and_then(Value::as_str).unwrap_or_default();
    let arguments = root
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let continue_from = root
        .get("continue_from")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let fork_from = root
        .get("fork_from")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Ok(InvocationArgs {
        name: name.to_owned(),
        arguments: arguments.to_owned(),
        continue_from: continue_from.trim().to_owned(),
        fork_from: fork_from.trim().to_owned(),
    })
}

fn profile_for_skill(
    skill: &SkillRecord,
    resolver: Option<&Arc<dyn ProfileResolver>>,
    task: &str,
) -> ProfileMeta {
    let mut profile = resolver
        .as_ref()
        .and_then(|r| r.resolve(skill))
        .unwrap_or_default();
    if profile.model.is_empty() && profile.effort.is_empty() {
        profile.model = skill.model.trim().to_owned();
        profile.effort = skill.effort.trim().to_owned();
    }
    profile.name = skill.name.clone();
    profile.count = 1;
    profile.label = task.trim().to_owned();
    profile
}

// ---------------------------------------------------------------------------
// run_skill
// ---------------------------------------------------------------------------

/// The `run_skill` tool.
#[derive(Clone, Default)]
pub struct RunSkillTool {
    store: Option<Arc<dyn SkillStore>>,
    runner: Option<Arc<dyn SubagentRunner>>,
    profile_resolver: Option<Arc<dyn ProfileResolver>>,
}

impl RunSkillTool {
    /// Builds the general skill-invocation tool over optional seams.
    pub fn new(
        store: Option<Arc<dyn SkillStore>>,
        runner: Option<Arc<dyn SubagentRunner>>,
        profile_resolver: Option<Arc<dyn ProfileResolver>>,
    ) -> Self {
        Self {
            store,
            runner,
            profile_resolver,
        }
    }
}

const RUN_SKILL_SCHEMA: &str = r#"{
"type":"object",
"properties":{
  "name":{"type":"string","description":"Skill identifier as it appears in the pinned Skills index (e.g. 'explore', 'review'). Case-sensitive. Just the identifier, not the [🧬 subagent] tag."},
  "arguments":{"type":"string","description":"Free-form arguments. For inline skills: appended as an 'Arguments:' line; the skill's own instructions decide how to use them. For subagent skills: REQUIRED — becomes the entire task the subagent receives."},
  "continue_from":{"type":"string","description":"Continue a prior compatible subagent transcript in the current conversation context. Only valid for runAs=subagent skills. Pass only the 'sa_...' value from the prior result's 'Subagent reference: ...' line."}
},
"required":["name"]
}"#;

impl Tool for RunSkillTool {
    fn name(&self) -> &str {
        RUN_SKILL_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Invoke a playbook from the Skills index pinned in the system prompt. For the built-in subagent skills (explore / research / review / security_review), prefer the dedicated top-level tools of the same name — they're easier to pick and do the same thing. Pass `name` as the BARE identifier (e.g. 'explore'), NOT the `[🧬 subagent]` tag that follows it in the index. `[🧬 subagent]` skills spawn an isolated subagent — only the final distilled answer returns; supply `arguments` describing the concrete task since the subagent has no other context. Untagged skills are inlined: the body becomes a tool result you read and follow."
    }

    fn schema(&self) -> Value {
        serde_json::from_str(RUN_SKILL_SCHEMA).expect("the run_skill schema is valid JSON")
    }

    fn execute(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let store = self.store.as_ref().ok_or_else(|| {
            Box::new(std::io::Error::other(
                "skill store seam is not wired in this session",
            )) as Box<dyn std::error::Error + Send + Sync>
        })?;
        let parsed = parse_invocation(args, true).map_err(std::io::Error::other)?;
        let raw_name = parsed.name.clone();
        let name = clean_skill_name(&raw_name);
        if name.is_empty() {
            return Err(Box::new(std::io::Error::other(format!(
                "run_skill requires a 'name' argument (got {raw_name:?}, which is just a marker/tag)"
            ))));
        }
        let Some(mut skill) = store.read(&name) else {
            return Err(Box::new(std::io::Error::other(format!(
                "unknown skill {name:?} — available: {}",
                available_names(store.as_ref())
            ))));
        };
        store
            .validate_invocation(&skill)
            .map_err(|e| std::io::Error::other(format!("run_skill: {e}")))?;
        skill = store.prepare(skill);
        let raw_args = parsed.arguments.trim().to_owned();
        let opts = SubagentRunOptions {
            continue_from: parsed.continue_from,
            fork_from: parsed.fork_from,
            ..Default::default()
        };
        if !opts.continue_from.is_empty() && !opts.fork_from.is_empty() {
            return Err(Box::new(std::io::Error::other(
                "run_skill: continue_from and fork_from are mutually exclusive; pass only continue_from",
            )));
        }
        if skill.run_as == SkillRunAs::Subagent {
            let Some(runner) = &self.runner else {
                return Err(Box::new(std::io::Error::other(format!(
                    "run_skill: skill {name:?} is runAs=subagent but no subagent runner is configured in this session"
                ))));
            };
            if raw_args.is_empty() {
                return Err(Box::new(std::io::Error::other(format!(
                    "run_skill: skill {name:?} is a subagent and requires 'arguments' — the subagent has no other context, so describe the concrete task"
                ))));
            }
            let profile = profile_for_skill(&skill, self.profile_resolver.as_ref(), &raw_args);
            let out = runner.run(&skill, &raw_args, &opts, &profile)?;
            return Ok(guard_subagent_host_decision_text(&out));
        }
        if !opts.continue_from.is_empty() || !opts.fork_from.is_empty() {
            return Err(Box::new(std::io::Error::other(
                "run_skill: subagent continuation is only valid for runAs=subagent skills",
            )));
        }
        Ok(render_invocation(&skill, &raw_args))
    }

    fn read_only(&self) -> bool {
        false
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// read_only_skill
// ---------------------------------------------------------------------------

/// The `read_only_skill` tool.
#[derive(Clone, Default)]
pub struct ReadOnlySkillTool {
    store: Option<Arc<dyn SkillStore>>,
    runner: Option<Arc<dyn SubagentRunner>>,
    profile_resolver: Option<Arc<dyn ProfileResolver>>,
}

impl ReadOnlySkillTool {
    /// Builds the explicitly read-only skill entry point.
    pub fn new(
        store: Option<Arc<dyn SkillStore>>,
        runner: Option<Arc<dyn SubagentRunner>>,
        profile_resolver: Option<Arc<dyn ProfileResolver>>,
    ) -> Self {
        Self {
            store,
            runner,
            profile_resolver,
        }
    }
}

const READ_ONLY_SKILL_SCHEMA: &str = r#"{
"type":"object",
"properties":{
  "name":{"type":"string","description":"Skill identifier as it appears in the pinned Skills index. Just the identifier, not the [🧬 subagent] tag."},
  "arguments":{"type":"string","description":"Free-form arguments. For inline skills: appended as an 'Arguments:' line. For subagent skills: REQUIRED — becomes the read-only subagent's entire task."}
},
"required":["name"]
}"#;

impl Tool for ReadOnlySkillTool {
    fn name(&self) -> &str {
        READ_ONLY_SKILL_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Invoke a skill in read-only mode. Inline skills are loaded into context like read_skill. `[🧬 subagent]` skills run in an isolated ephemeral read-only subagent with only read-only research tools and safe foreground bash; no writes, installers, memory mutation, continuation/fork, background jobs, or writer-capable delegation are available. Read-only nested delegation may be available until max_subagent_depth is reached. Pass `name` as the bare skill identifier and `arguments` as the concrete task."
    }

    fn schema(&self) -> Value {
        serde_json::from_str(READ_ONLY_SKILL_SCHEMA)
            .expect("the read_only_skill schema is valid JSON")
    }

    fn execute(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let store = self.store.as_ref().ok_or_else(|| {
            Box::new(std::io::Error::other(
                "skill store seam is not wired in this session",
            )) as Box<dyn std::error::Error + Send + Sync>
        })?;
        let parsed = parse_invocation(args, false).map_err(std::io::Error::other)?;
        let raw_name = parsed.name.clone();
        let name = clean_skill_name(&raw_name);
        if name.is_empty() {
            return Err(Box::new(std::io::Error::other(format!(
                "read_only_skill requires a 'name' argument (got {raw_name:?}, which is just a marker/tag)"
            ))));
        }
        let Some(mut skill) = store.read(&name) else {
            return Err(Box::new(std::io::Error::other(format!(
                "unknown skill {name:?} — available: {}",
                available_names(store.as_ref())
            ))));
        };
        store
            .validate_invocation(&skill)
            .map_err(|e| std::io::Error::other(format!("read_only_skill: {e}")))?;
        skill = store.prepare(skill);
        let raw_args = parsed.arguments.trim().to_owned();
        if skill.run_as == SkillRunAs::Subagent {
            let Some(runner) = &self.runner else {
                return Err(Box::new(std::io::Error::other(format!(
                    "read_only_skill: skill {name:?} is runAs=subagent but no read-only subagent runner is configured in this session"
                ))));
            };
            if raw_args.is_empty() {
                return Err(Box::new(std::io::Error::other(format!(
                    "read_only_skill: skill {name:?} is a subagent and requires 'arguments' — the subagent has no other context, so describe the concrete {READ_ONLY_TASK_NOTICE} task"
                ))));
            }
            let profile = profile_for_skill(&skill, self.profile_resolver.as_ref(), &raw_args);
            let out = runner.run(&skill, &raw_args, &SubagentRunOptions::default(), &profile)?;
            return Ok(guard_subagent_host_decision_text(&out));
        }
        Ok(render_invocation(&skill, &raw_args))
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default().with(
            "plan_mode_safe",
            Arc::new(ReadOnlyPlanSafe) as Arc<dyn PlanModeClassifier>,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct ReadOnlyPlanSafe;
impl Capability for ReadOnlyPlanSafe {}
impl PlanModeClassifier for ReadOnlyPlanSafe {
    fn plan_mode_safe(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// read_skill
// ---------------------------------------------------------------------------

/// The `read_skill` tool: renders an inline skill body without running anything.
#[derive(Clone, Default)]
pub struct ReadSkillTool {
    store: Option<Arc<dyn SkillStore>>,
}

impl ReadSkillTool {
    /// Builds the read-only inline-skill loader.
    pub fn new(store: Option<Arc<dyn SkillStore>>) -> Self {
        Self { store }
    }
}

const READ_SKILL_SCHEMA: &str = r#"{
"type":"object",
"properties":{
  "name":{"type":"string","description":"Inline skill identifier as it appears in the pinned Skills index. Just the identifier, not the [🧬 subagent] tag."},
  "arguments":{"type":"string","description":"Optional free-form arguments, appended as an 'Arguments:' line; the skill's own instructions decide how to use them."}
},
"required":["name"]
}"#;

impl Tool for ReadSkillTool {
    fn name(&self) -> &str {
        READ_SKILL_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Load an inline playbook from the Skills index into your context WITHOUT running anything — the skill body returns as a tool result you read and follow. This is the read-only alternative when no subagent execution is needed. Pass `name` as the BARE identifier (e.g. 'commit'), NOT the `[🧬 subagent]` tag. Subagent-tagged skills are rejected: use run_skill (or the dedicated tool) for those, since they execute work."
    }

    fn schema(&self) -> Value {
        serde_json::from_str(READ_SKILL_SCHEMA).expect("the read_skill schema is valid JSON")
    }

    fn execute(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let store = self.store.as_ref().ok_or_else(|| {
            Box::new(std::io::Error::other(
                "skill store seam is not wired in this session",
            )) as Box<dyn std::error::Error + Send + Sync>
        })?;
        let parsed = parse_invocation(args, false).map_err(std::io::Error::other)?;
        let raw_name = parsed.name.clone();
        let name = clean_skill_name(&raw_name);
        if name.is_empty() {
            return Err(Box::new(std::io::Error::other(format!(
                "read_skill requires a 'name' argument (got {raw_name:?}, which is just a marker/tag)"
            ))));
        }
        let Some(mut skill) = store.read(&name) else {
            return Err(Box::new(std::io::Error::other(format!(
                "unknown skill {name:?} — available: {}",
                available_names(store.as_ref())
            ))));
        };
        store
            .validate_invocation(&skill)
            .map_err(|e| std::io::Error::other(format!("read_skill: {e}")))?;
        skill = store.prepare(skill);
        if skill.run_as == SkillRunAs::Subagent {
            return Err(Box::new(std::io::Error::other(format!(
                "read_skill: skill {name:?} is a subagent and must be executed, not read — use run_skill (or the dedicated {name} tool)"
            ))));
        }
        Ok(render_invocation(&skill, parsed.arguments.trim()))
    }

    fn read_only(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// install_skill
// ---------------------------------------------------------------------------

/// Hook fired after install_skill writes a new file, so a host can refresh UI
/// without a reload (Go `InstalledHook`).
pub type InstalledHook = Arc<dyn Fn(&str, &str, &str) + Send + Sync>;

/// The `install_skill` authoring tool.
#[derive(Clone)]
pub struct InstallSkillTool {
    store: Option<Arc<dyn SkillStore>>,
    on_installed: Option<InstalledHook>,
    description: String,
}

impl Default for InstallSkillTool {
    fn default() -> Self {
        Self::new(None, None)
    }
}

impl InstallSkillTool {
    /// Builds the authoring tool over an optional store seam and hook.
    pub fn new(store: Option<Arc<dyn SkillStore>>, on_installed: Option<InstalledHook>) -> Self {
        let has_project = store
            .as_ref()
            .map(|s| s.has_project_scope())
            .unwrap_or(false);
        let description = Self::description_for(has_project);
        Self {
            store,
            on_installed,
            description,
        }
    }

    fn description_for(has_project: bool) -> String {
        let scope = if has_project {
            "'project' (default) writes to <repo>/.reasonix/skills/ (this workspace only); 'global' writes to the Reasonix home skills directory (every project)."
        } else {
            "'global' (only option — no project workspace) writes to the Reasonix home skills directory."
        };
        format!(
            "Author and save a new skill — a reusable playbook future turns invoke via run_skill (or /<name>). Runnable immediately this turn; appears in the pinned Skills index on the next launch. {scope}"
        )
    }
}

const INSTALL_SKILL_SCHEMA: &str = r#"{
"type":"object",
"properties":{
  "name":{"type":"string","description":"Identifier — letters/digits/_/-/., 1-64 chars, starts alphanumeric. Becomes the skill folder name under the selected skills directory."},
  "description":{"type":"string","description":"≤120-char one-liner shown in the pinned Skills index — future agents read it to decide whether to invoke."},
  "body":{"type":"string","description":"Markdown playbook. For subagent skills, write the subagent's persona/rules — it gets no context besides 'arguments' at runtime."},
  "scope":{"type":"string","enum":["project","global"],"description":"Where to write. Defaults to project when a workspace exists, else global."},
  "runAs":{"type":"string","enum":["inline","subagent"],"description":"inline (default) folds the body into the parent turn; subagent spawns an isolated child loop returning only its final answer (use for context-heavy work)."},
  "model":{"type":"string","description":"Optional model override for runAs=subagent (a configured provider/model name). Ignored otherwise."},
  "effort":{"type":"string","description":"Optional effort for runAs=subagent (e.g. high, max). Ignored otherwise."},
  "allowedTools":{"type":"array","items":{"type":"string"},"description":"Optional tool allowlist for runAs=subagent (e.g. ['read_file','grep'])."}
},
"required":["name","description","body"]
}"#;

impl Tool for InstallSkillTool {
    fn name(&self) -> &str {
        INSTALL_SKILL_TOOL_NAME
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> Value {
        serde_json::from_str(INSTALL_SKILL_SCHEMA).expect("the install_skill schema is valid JSON")
    }

    fn execute(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let store = self.store.as_ref().ok_or_else(|| {
            Box::new(std::io::Error::other(
                "skill store seam is not wired in this session",
            )) as Box<dyn std::error::Error + Send + Sync>
        })?;
        let root = args
            .as_object()
            .ok_or_else(|| std::io::Error::other("invalid args: not an object"))?;
        for key in root.keys() {
            if ![
                "name",
                "description",
                "body",
                "scope",
                "runAs",
                "model",
                "effort",
                "allowedTools",
            ]
            .contains(&key.as_str())
            {
                return Err(Box::new(std::io::Error::other(format!(
                    "invalid args: unknown field {key:?}"
                ))));
            }
        }
        let name = root
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if name.is_empty() {
            return Err(Box::new(std::io::Error::other(
                "install_skill requires a non-empty 'name'",
            )));
        }
        let description = collapse_spaces(
            root.get("description")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
        if description.is_empty() {
            return Err(Box::new(std::io::Error::other(
                "install_skill requires a non-empty 'description' — it is what appears in the Skills index",
            )));
        }
        let body = root.get("body").and_then(Value::as_str).unwrap_or_default();
        if body.trim().is_empty() {
            return Err(Box::new(std::io::Error::other(
                "install_skill requires a non-empty 'body' — the playbook the skill executes",
            )));
        }

        let scope_raw = root
            .get("scope")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let scope = match scope_raw.trim() {
            "global" => "global",
            "project" => "project",
            _ if store.has_project_scope() => "project",
            _ => "global",
        };
        if scope == "project" && !store.has_project_scope() {
            return Err(Box::new(std::io::Error::other(
                "install_skill: scope='project' requires a workspace — use scope='global'",
            )));
        }

        let run_as = SkillRunAs::parse_run_as(
            root.get("runAs")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
        let model = root
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let effort = root
            .get("effort")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let allowed_tools: Vec<String> = root
            .get("allowedTools")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();

        let content = render_skill_file(&SkillFileOptions {
            name: name.clone(),
            description,
            body: body.to_owned(),
            run_as,
            model: model.to_owned(),
            effort: effort.to_owned(),
            allowed_tools,
            ..Default::default()
        });
        let path = store.create_with_content(&name, scope, &content)?;
        if let Some(hook) = &self.on_installed {
            hook(&name, &path, scope);
        }
        let result = serde_json::json!({
            "ok": true,
            "name": name,
            "scope": scope,
            "path": path,
            "runAs": run_as.as_str(),
            "note": format!("Callable now via run_skill({{name}}) or /{name}. Appears in the pinned Skills index on the next launch."),
        });
        Ok(serde_json::to_string(&result)
            .map_err(|e| Box::new(std::io::Error::other(format!("encode install result: {e}"))))?)
    }

    fn read_only(&self) -> bool {
        false
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Dedicated sub-agent wrappers (Go: builtinSubagentTools / subagentSkillTool)
// ---------------------------------------------------------------------------

struct WrapperSpec {
    tool_name: &'static str,
    skill_name: &'static str,
    description: &'static str,
    task_desc: &'static str,
    max_steps: usize,
}

const WRAPPER_SPECS: &[WrapperSpec] = &[
    WrapperSpec {
        tool_name: "explore",
        skill_name: "explore",
        description: "Run a focused read-only codebase investigation in an isolated subagent. Use for broad survey questions across many files — 'find all places that X', 'how does Y work across the project', 'audit Z'. Returns one distilled answer with file:line citations. Its reads + reasoning never enter your context, unlike chained read_file.",
        task_desc: "Concrete investigation question. The subagent has none of your context — write a self-contained prompt naming the symbol / pattern / behavior to survey.",
        max_steps: 0,
    },
    WrapperSpec {
        tool_name: "locate",
        skill_name: "locate",
        description: "Find where something lives in the codebase: a read-only subagent that runs its searches in parallel within four rounds and returns only file:line ranges, one reason each. Use before read_file or edits when you do not yet know which files matter — 'where is X defined / handled / configured'. Cheaper and faster than explore; use explore when you need an explanation, not locations.",
        task_desc: "What to find. The subagent has none of your context — name the symbol, error text, config key or behavior, and any spelling variants you know.",
        max_steps: 4,
    },
    WrapperSpec {
        tool_name: "research",
        skill_name: "research",
        description: "Combine web_fetch + code reading in an isolated subagent. Use when the answer needs both an external reference and local verification — 'is X supported by lib Y', 'compare our impl against the spec'. Returns one synthesis citing code (file:line) and web (URL).",
        task_desc: "Concrete research question. The subagent has none of your context — name the external thing to look up and the local code to compare against.",
        max_steps: 0,
    },
    WrapperSpec {
        tool_name: "review",
        skill_name: "review",
        description: "Review the pending changes (current branch diff) in an isolated subagent — flags correctness / security / missing-tests / hidden behavior per file:line. Read-only; you decide what to act on. Use before suggesting a PR-shaped change or after finishing a multi-step edit.",
        task_desc: "What to focus the review on (e.g. 'focus on the auth changes' or 'general'). The subagent reads the diff itself.",
        max_steps: 0,
    },
    WrapperSpec {
        tool_name: "security_review",
        skill_name: "security-review",
        description: "Security-focused review of the current branch diff in an isolated subagent — injection / authz / secrets / deserialization / path-traversal / crypto, severity-tagged. Read-only. Use when shipping changes that touch auth, input parsing, file IO, or external requests.",
        task_desc: "Optional scope hint (e.g. 'focus on token handling in internal/auth/') or 'full' for everything in the diff.",
        max_steps: 0,
    },
];

/// The five dedicated top-level wrapper tools (Go `BuiltinSubagentTools`).
/// Each is skipped when its underlying skill is not registered, so the tool
/// set never advertises a phantom skill.
pub fn builtin_subagent_tools(
    store: Option<Arc<dyn SkillStore>>,
    runner: Option<Arc<dyn SubagentRunner>>,
    profile_resolver: Option<Arc<dyn ProfileResolver>>,
) -> Vec<Arc<dyn Tool>> {
    let Some(store) = store else {
        return Vec::new();
    };
    let present: std::collections::HashSet<String> =
        store.list_enabled().into_iter().map(|s| s.name).collect();
    let mut out: Vec<Arc<dyn Tool>> = Vec::new();
    for spec in WRAPPER_SPECS {
        if !present.contains(spec.skill_name) {
            continue;
        }
        out.push(Arc::new(SubagentSkillTool {
            spec,
            store: store.clone(),
            runner: runner.clone(),
            profile: profile_resolver.clone(),
        }));
    }
    out
}

/// One dedicated wrapper bound to a built-in sub-agent skill.
pub struct SubagentSkillTool {
    spec: &'static WrapperSpec,
    store: Arc<dyn SkillStore>,
    runner: Option<Arc<dyn SubagentRunner>>,
    profile: Option<Arc<dyn ProfileResolver>>,
}

impl Tool for SubagentSkillTool {
    fn name(&self) -> &str {
        self.spec.tool_name
    }

    fn description(&self) -> &str {
        self.spec.description
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": { "type": "string", "description": self.spec.task_desc },
                "continue_from": { "type": "string", "description": "Continue a prior compatible subagent transcript in the current conversation context. Pass only the 'sa_...' value from the prior result's 'Subagent reference: ...' line." }
            },
            "required": ["task"]
        })
    }

    fn execute(&self, _ctx: &CallContext, args: &Value) -> ToolResult {
        let root = args
            .as_object()
            .ok_or_else(|| std::io::Error::other("invalid args: not an object"))?;
        for key in root.keys() {
            if !["task", "continue_from", "fork_from"].contains(&key.as_str()) {
                return Err(Box::new(std::io::Error::other(format!(
                    "invalid args: unknown field {key:?}"
                ))));
            }
        }
        let task = root
            .get("task")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if task.is_empty() {
            return Err(Box::new(std::io::Error::other(format!(
                "{} requires a non-empty 'task' argument — describe the concrete question",
                self.spec.tool_name
            ))));
        }
        let Some(mut skill) = self.store.read(self.spec.skill_name) else {
            return Err(Box::new(std::io::Error::other(format!(
                "{}: built-in skill {:?} is not registered",
                self.spec.tool_name, self.spec.skill_name
            ))));
        };
        self.store
            .validate_invocation(&skill)
            .map_err(|e| std::io::Error::other(format!("{}: {e}", self.spec.tool_name)))?;
        skill = self.store.prepare(skill);
        if skill.run_as != SkillRunAs::Subagent {
            return Err(Box::new(std::io::Error::other(format!(
                "{}: skill {:?} is overridden as inline; invoke it via run_skill instead",
                self.spec.tool_name, self.spec.skill_name
            ))));
        }
        let Some(runner) = &self.runner else {
            return Err(Box::new(std::io::Error::other(format!(
                "{}: no subagent runner is configured in this session",
                self.spec.tool_name
            ))));
        };
        let continue_from = root
            .get("continue_from")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        let fork_from = root
            .get("fork_from")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if !continue_from.is_empty() && !fork_from.is_empty() {
            return Err(Box::new(std::io::Error::other(format!(
                "{}: continue_from and fork_from are mutually exclusive; pass only continue_from",
                self.spec.tool_name
            ))));
        }
        let opts = SubagentRunOptions {
            continue_from,
            fork_from,
            max_steps: self.spec.max_steps,
            ..Default::default()
        };
        let profile = profile_for_skill(&skill, self.profile.as_ref(), &task);
        let out = runner.run(&skill, &task, &opts, &profile)?;
        Ok(guard_subagent_host_decision_text(&out))
    }

    fn read_only(&self) -> bool {
        false
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests;
