//! TOOL-040: skill tools + five dedicated sub-agent wrappers
//! (Go `internal/ext/skill/tools.go`; docs/contracts/tools.md §3.5).
//!
//! Behavior-level evidence: the four tool cards and read-only flags, name
//! cleaning, invocation rendering, host-decision boundary guard, inline vs
//! subagent dispatch, install validation and conditional frontmatter, wrapper
//! skip/bounce/validation paths, and seam constructors for the integration slice.

use std::sync::Arc;

use reasonix_tools::abi::{CallContext, Tool};
use reasonix_tools::builtin::skill::{
    INSTALL_SKILL_TOOL_NAME, ProfileMeta, ProfileResolver, READ_ONLY_SKILL_TOOL_NAME,
    READ_SKILL_TOOL_NAME, RUN_SKILL_TOOL_NAME, SUBAGENT_HOST_DECISION_NOTICE, SkillRecord,
    SkillRunAs, SkillScope, SkillStore, SubagentRunOptions, SubagentRunner, available_names,
    clean_skill_name, collapse_spaces, guard_subagent_host_decision_text, render_invocation,
    render_skill_file,
};
use reasonix_tools::builtin::skill_tools::{
    InstallSkillTool, InstalledHook, ReadOnlySkillTool, ReadSkillTool, RunSkillTool,
    SubagentSkillTool, builtin_subagent_tools,
};
use serde_json::{Value, json};

#[derive(Default)]
struct MemoryStore {
    skills: Vec<SkillRecord>,
    project_scope: bool,
}

impl SkillStore for MemoryStore {
    fn read(&self, name: &str) -> Option<SkillRecord> {
        self.skills.iter().find(|s| s.name == name).cloned()
    }
    fn list_enabled(&self) -> Vec<SkillRecord> {
        self.skills.clone()
    }
    fn has_project_scope(&self) -> bool {
        self.project_scope
    }
    fn create_with_content(
        &self,
        name: &str,
        scope: &str,
        _content: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok(format!("skills/{scope}/{name}"))
    }
}

struct StubRunner {
    answer: String,
    last_profile: std::sync::Mutex<Option<ProfileMeta>>,
}

impl StubRunner {
    fn new(answer: &str) -> Self {
        Self {
            answer: answer.to_owned(),
            last_profile: std::sync::Mutex::new(None),
        }
    }
}

impl SubagentRunner for StubRunner {
    fn run(
        &self,
        _skill: &SkillRecord,
        _task: &str,
        _opts: &SubagentRunOptions,
        profile: &ProfileMeta,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        *self.last_profile.lock().unwrap() = Some(profile.clone());
        Ok(self.answer.clone())
    }
}

struct EchoResolver;
impl ProfileResolver for EchoResolver {
    fn resolve(&self, skill: &SkillRecord) -> Option<ProfileMeta> {
        Some(ProfileMeta {
            name: skill.name.clone(),
            count: 1,
            label: String::new(),
            model: skill.model.clone(),
            effort: skill.effort.clone(),
        })
    }
}

fn inline_skill() -> SkillRecord {
    SkillRecord {
        name: "commit".to_owned(),
        description: "Commit playbook".to_owned(),
        body: "Stage and commit.".to_owned(),
        scope: SkillScope::Builtin,
        path: "builtin/commit/SKILL.md".to_owned(),
        ..Default::default()
    }
}

fn subagent_skill() -> SkillRecord {
    SkillRecord {
        name: "explore".to_owned(),
        description: "Survey the codebase".to_owned(),
        body: "Survey.".to_owned(),
        scope: SkillScope::Builtin,
        path: "builtin/explore/SKILL.md".to_owned(),
        run_as: SkillRunAs::Subagent,
        model: "mini".to_owned(),
        effort: "low".to_owned(),
        ..Default::default()
    }
}

#[test]
fn four_cards_carry_the_go_names_and_read_only_flags() {
    let run = RunSkillTool::default();
    let read_only = ReadOnlySkillTool::default();
    let read = ReadSkillTool::default();
    let install = InstallSkillTool::default();

    assert_eq!(run.name(), RUN_SKILL_TOOL_NAME);
    assert_eq!(read_only.name(), READ_ONLY_SKILL_TOOL_NAME);
    assert_eq!(read.name(), READ_SKILL_TOOL_NAME);
    assert_eq!(install.name(), INSTALL_SKILL_TOOL_NAME);

    assert!(!run.read_only());
    assert!(read_only.read_only());
    assert!(read.read_only());
    assert!(!install.read_only());

    for tool in [&run as &dyn Tool, &read_only, &read, &install] {
        assert!(!tool.description().is_empty());
        let schema = tool.schema();
        assert_eq!(schema["type"], "object");
    }
}

#[test]
fn clean_name_strips_bracket_tags_and_picks_the_alphanumeric_token() {
    assert_eq!(clean_skill_name("explore [🧬 subagent]"), "explore");
    assert_eq!(clean_skill_name("  Review  "), "Review");
    assert_eq!(clean_skill_name("[tag]"), "");
    assert_eq!(clean_skill_name(""), "");
}

#[test]
fn collapse_spaces_folds_newlines_and_runs() {
    assert_eq!(collapse_spaces("  a\n b\t c  "), "a b c");
}

#[test]
fn invocation_pins_the_skill_body_and_arguments() {
    let rendered = render_invocation(&inline_skill(), "what to commit");
    assert!(rendered.starts_with("<skill-pin name=\"commit\">"));
    assert!(rendered.contains("# Skill: commit"));
    assert!(rendered.contains("Arguments: what to commit"));
    assert!(rendered.ends_with("</skill-pin>"));
}

#[test]
fn guard_appends_the_boundary_only_for_host_decision_text_once() {
    assert_eq!(
        guard_subagent_host_decision_text("Found three matches in src/"),
        "Found three matches in src/"
    );
    let guarded = guard_subagent_host_decision_text("the user approved the change");
    assert!(guarded.contains(SUBAGENT_HOST_DECISION_NOTICE));
    assert_eq!(guard_subagent_host_decision_text(&guarded), guarded);
}

#[test]
fn run_skill_inline_renders_body_and_subagent_runs_through_runner_with_profile() {
    let store = Arc::new(MemoryStore {
        skills: vec![inline_skill(), subagent_skill()],
        ..Default::default()
    });
    let runner = Arc::new(StubRunner::new("Survey complete"));
    let resolver = Arc::new(EchoResolver);
    let tool = RunSkillTool::new(Some(store.clone()), Some(runner.clone()), Some(resolver));

    let inline = tool
        .execute(
            &CallContext::new(),
            &json!({ "name": "commit", "arguments": "all files" }),
        )
        .unwrap();
    assert!(inline.contains("# Skill: commit"));

    let sub = tool
        .execute(
            &CallContext::new(),
            &json!({ "name": "explore", "arguments": "find X" }),
        )
        .unwrap();
    assert_eq!(sub, "Survey complete");
    let profile = runner.last_profile.lock().unwrap().clone().unwrap();
    assert_eq!(profile.model, "mini");
    assert_eq!(profile.effort, "low");
}

#[test]
fn run_skill_refuses_missing_name_unknown_skill_empty_args_and_both_continuations() {
    let store = Arc::new(MemoryStore {
        skills: vec![subagent_skill()],
        ..Default::default()
    });
    let runner = Arc::new(StubRunner::new("ok"));
    let tool = RunSkillTool::new(Some(store), Some(runner), None);

    let no_name = tool
        .execute(&CallContext::new(), &json!({ "name": "[🧬 subagent]" }))
        .unwrap_err();
    assert!(no_name.to_string().contains("requires a 'name'"));

    let unknown = tool
        .execute(&CallContext::new(), &json!({ "name": "ghost" }))
        .unwrap_err();
    assert!(unknown.to_string().contains("unknown skill"));
    assert!(unknown.to_string().contains("explore"));

    let empty_args = tool
        .execute(
            &CallContext::new(),
            &json!({ "name": "explore", "arguments": "  " }),
        )
        .unwrap_err();
    assert!(empty_args.to_string().contains("requires 'arguments'"));

    let both = tool
        .execute(
            &CallContext::new(),
            &json!({ "name": "explore", "arguments": "x", "continue_from": "sa_1", "fork_from": "sa_2" }),
        )
        .unwrap_err();
    assert!(both.to_string().contains("mutually exclusive"));

    let unknown_field = tool
        .execute(
            &CallContext::new(),
            &json!({ "name": "explore", "arguments": "x", "nope": 1 }),
        )
        .unwrap_err();
    assert!(unknown_field.to_string().contains("unknown field"));
}

#[test]
fn read_skill_rejects_subagent_skills_and_read_only_tool_runs_its_own_path() {
    let store = Arc::new(MemoryStore {
        skills: vec![subagent_skill()],
        ..Default::default()
    });
    let read = ReadSkillTool::new(Some(store.clone()));
    let err = read
        .execute(&CallContext::new(), &json!({ "name": "explore" }))
        .unwrap_err();
    assert!(err.to_string().contains("must be executed, not read"));

    let runner = Arc::new(StubRunner::new("read-only answer"));
    let ro = ReadOnlySkillTool::new(Some(store), Some(runner), None);
    let out = ro
        .execute(
            &CallContext::new(),
            &json!({ "name": "explore", "arguments": "survey safely" }),
        )
        .unwrap();
    assert_eq!(out, "read-only answer");
}

#[test]
fn install_skill_validates_fields_writes_frontmatter_and_fires_the_hook() {
    let store = Arc::new(MemoryStore {
        skills: vec![],
        project_scope: true,
    });
    let hook_called: Arc<std::sync::Mutex<Option<(String, String, String)>>> = Arc::default();
    let hook: InstalledHook = {
        let called = hook_called.clone();
        Arc::new(move |name, path, scope| {
            *called.lock().unwrap() = Some((name.to_owned(), path.to_owned(), scope.to_owned()));
        })
    };
    let tool = InstallSkillTool::new(Some(store), Some(hook));

    let no_name = tool
        .execute(
            &CallContext::new(),
            &json!({ "description": "d", "body": "b" }),
        )
        .unwrap_err();
    assert!(no_name.to_string().contains("non-empty 'name'"));
    let no_desc = tool
        .execute(&CallContext::new(), &json!({ "name": "s", "body": "b" }))
        .unwrap_err();
    assert!(no_desc.to_string().contains("non-empty 'description'"));
    let no_body = tool
        .execute(
            &CallContext::new(),
            &json!({ "name": "s", "description": "d" }),
        )
        .unwrap_err();
    assert!(no_body.to_string().contains("non-empty 'body'"));

    let ok = tool
        .execute(
            &CallContext::new(),
            &json!({
                "name": "demo",
                "description": "One-liner",
                "body": "Playbook body",
                "runAs": "subagent",
                "allowedTools": ["read_file", "grep"]
            }),
        )
        .unwrap();
    let body: Value = serde_json::from_str(&ok).unwrap();
    assert_eq!(body["ok"], true);
    assert_eq!(body["runAs"], "subagent");
    assert_eq!(body["path"], "skills/project/demo");
    let called = hook_called.lock().unwrap().clone().unwrap();
    assert_eq!(called.0, "demo");
    assert_eq!(called.2, "project");
}

#[test]
fn install_project_scope_without_workspace_is_refused() {
    let store = Arc::new(MemoryStore {
        skills: vec![],
        project_scope: false,
    });
    let tool = InstallSkillTool::new(Some(store), None);
    let err = tool
        .execute(
            &CallContext::new(),
            &json!({ "name": "s", "description": "d", "body": "b", "scope": "project" }),
        )
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("scope='project' requires a workspace")
    );
}

#[test]
fn skill_file_frontmatter_emits_subagent_only_fields_conditionially() {
    let inline = render_skill_file(&render_opts(SkillRunAs::Inline));
    assert!(!inline.contains("model:"));
    assert!(!inline.contains("allowed-tools:"));

    let sub = render_skill_file(&render_opts(SkillRunAs::Subagent));
    assert!(sub.contains("runAs: \"subagent\""));
    assert!(sub.contains("model: \"m\""));
    assert!(sub.contains("read-only: true"));
    assert!(sub.contains("allowed-tools: [\"read_file\", \"grep\"]"));
    assert!(sub.contains("color: \"blue\""));
    assert!(sub.contains("invocation: \"manual\""));
}

fn render_opts(run_as: SkillRunAs) -> reasonix_tools::builtin::skill::SkillFileOptions {
    reasonix_tools::builtin::skill::SkillFileOptions {
        name: "explore".to_owned(),
        description: "d".to_owned(),
        body: "body".to_owned(),
        run_as,
        model: "m".to_owned(),
        effort: "high".to_owned(),
        allowed_tools: vec!["read_file".to_owned(), "grep".to_owned()],
        read_only: true,
        color: "blue".to_owned(),
        invocation: "manual".to_owned(),
    }
}

#[test]
fn wrappers_skip_unregistered_skills_and_bounce_overridden_inline() {
    let store = Arc::new(MemoryStore {
        skills: vec![subagent_skill()],
        ..Default::default()
    });
    let runner = Arc::new(StubRunner::new("done"));
    let tools = builtin_subagent_tools(Some(store.clone()), Some(runner.clone()), None);
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert_eq!(names, vec!["explore"]);

    let mut inline = subagent_skill();
    inline.run_as = SkillRunAs::Inline;
    let store = Arc::new(MemoryStore {
        skills: vec![inline],
        ..Default::default()
    });
    let tools = builtin_subagent_tools(Some(store), Some(runner), None);
    let err = tools[0]
        .execute(&CallContext::new(), &json!({ "task": "survey" }))
        .unwrap_err();
    assert!(err.to_string().contains("overridden as inline"));
}

#[test]
fn wrapper_requires_a_task_and_rejects_unknown_fields() {
    let store = Arc::new(MemoryStore {
        skills: vec![subagent_skill()],
        ..Default::default()
    });
    let runner = Arc::new(StubRunner::new("done"));
    let tools = builtin_subagent_tools(Some(store), Some(runner), None);
    let wrapper: &SubagentSkillTool = tools[0].as_any().downcast_ref().unwrap();

    let no_task = wrapper
        .execute(&CallContext::new(), &json!({ "task": "  " }))
        .unwrap_err();
    assert!(no_task.to_string().contains("non-empty 'task'"));

    let unknown = wrapper
        .execute(&CallContext::new(), &json!({ "task": "survey", "nope": 1 }))
        .unwrap_err();
    assert!(unknown.to_string().contains("unknown field"));
}

#[test]
fn available_names_list_empty_store_and_registered_skills() {
    let empty = MemoryStore::default();
    assert_eq!(available_names(&empty), "(none — no skills defined)");

    let store = MemoryStore {
        skills: vec![subagent_skill()],
        ..Default::default()
    };
    assert!(available_names(&store).contains("explore"));
}

#[test]
fn skill_constructors_accept_empty_seams_for_assembly() {
    let _run = RunSkillTool::new(None, None, None);
    let _ro = ReadOnlySkillTool::new(None, None, None);
    let _read = ReadSkillTool::new(None);
    let _install = InstallSkillTool::new(None, None);
    let _wrappers = builtin_subagent_tools(None, None, None);
}
