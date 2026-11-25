use super::*;

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
}

impl SubagentRunner for StubRunner {
    fn run(
        &self,
        _skill: &SkillRecord,
        _task: &str,
        _opts: &SubagentRunOptions,
        _profile: &ProfileMeta,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.answer.clone())
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
        ..Default::default()
    }
}

#[test]
fn clean_name_strips_bracket_tags_and_picks_alphanumeric_token() {
    assert_eq!(clean_skill_name("explore [🧬 subagent]"), "explore");
    assert_eq!(clean_skill_name("  Review  "), "Review");
    assert_eq!(clean_skill_name("[tag]"), "");
    assert_eq!(clean_skill_name(""), "");
}

#[test]
fn guard_appends_boundary_only_for_host_decision_text() {
    let ordinary = "Found three matches in src/.";
    assert_eq!(guard_subagent_host_decision_text(ordinary), ordinary);
    let guarded = guard_subagent_host_decision_text("the user approved the change");
    assert!(guarded.contains(SUBAGENT_HOST_DECISION_NOTICE));
    // Never guard twice.
    assert_eq!(guard_subagent_host_decision_text(&guarded), guarded);
}

#[test]
fn render_invocation_wraps_body_in_skill_pin() {
    let rendered = render_invocation(&inline_skill(), "what to commit");
    assert!(rendered.starts_with("<skill-pin name=\"commit\">"));
    assert!(rendered.contains("# Skill: commit"));
    assert!(rendered.contains("Arguments: what to commit"));
    assert!(rendered.ends_with("</skill-pin>"));
}

#[test]
fn run_skill_inline_renders_body_and_subagent_runs_through_runner() {
    let store = Arc::new(MemoryStore {
        skills: vec![inline_skill(), subagent_skill()],
        ..Default::default()
    });
    let runner = Arc::new(StubRunner {
        answer: "Survey complete".to_owned(),
    });
    let tool = RunSkillTool::new(Some(store.clone()), Some(runner), None);

    let inline = tool
        .execute(
            &CallContext::new(),
            &serde_json::json!({ "name": "commit", "arguments": "all files" }),
        )
        .unwrap();
    assert!(inline.contains("# Skill: commit"));

    let sub = tool
        .execute(
            &CallContext::new(),
            &serde_json::json!({ "name": "explore", "arguments": "find X" }),
        )
        .unwrap();
    assert_eq!(sub, "Survey complete");
}

#[test]
fn run_skill_refuses_missing_name_unknown_skill_and_empty_subagent_args() {
    let store = Arc::new(MemoryStore {
        skills: vec![subagent_skill()],
        ..Default::default()
    });
    let runner = Arc::new(StubRunner {
        answer: "ok".to_owned(),
    });
    let tool = RunSkillTool::new(Some(store), Some(runner), None);

    let no_name = tool.execute(
        &CallContext::new(),
        &serde_json::json!({ "name": "[🧬 subagent]" }),
    );
    assert!(
        no_name
            .unwrap_err()
            .to_string()
            .contains("requires a 'name'")
    );

    let unknown = tool.execute(&CallContext::new(), &serde_json::json!({ "name": "ghost" }));
    assert!(unknown.unwrap_err().to_string().contains("unknown skill"));

    let empty_args = tool.execute(
        &CallContext::new(),
        &serde_json::json!({ "name": "explore", "arguments": "  " }),
    );
    assert!(
        empty_args
            .unwrap_err()
            .to_string()
            .contains("requires 'arguments'")
    );

    let both = tool.execute(
        &CallContext::new(),
        &serde_json::json!({ "name": "explore", "arguments": "x", "continue_from": "sa_1", "fork_from": "sa_2" }),
    );
    assert!(both.unwrap_err().to_string().contains("mutually exclusive"));
}

#[test]
fn read_skill_rejects_subagent_skills() {
    let store = Arc::new(MemoryStore {
        skills: vec![subagent_skill()],
        ..Default::default()
    });
    let tool = ReadSkillTool::new(Some(store));
    let err = tool
        .execute(
            &CallContext::new(),
            &serde_json::json!({ "name": "explore" }),
        )
        .unwrap_err();
    assert!(err.to_string().contains("must be executed, not read"));
}

#[test]
fn wrappers_skip_unregistered_skills_and_bounce_overridden_inline() {
    let store = Arc::new(MemoryStore {
        skills: vec![subagent_skill()],
        ..Default::default()
    });
    let runner = Arc::new(StubRunner {
        answer: "done".to_owned(),
    });
    let tools = builtin_subagent_tools(Some(store.clone()), Some(runner.clone()), None);
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert_eq!(names, vec!["explore"]);

    // Override explore back to inline: the wrapper bounces to run_skill.
    let mut inline = subagent_skill();
    inline.run_as = SkillRunAs::Inline;
    let store = Arc::new(MemoryStore {
        skills: vec![inline],
        ..Default::default()
    });
    let tools = builtin_subagent_tools(Some(store), Some(runner), None);
    let err = tools[0]
        .execute(
            &CallContext::new(),
            &serde_json::json!({ "task": "survey" }),
        )
        .unwrap_err();
    assert!(err.to_string().contains("overridden as inline"));
}

#[test]
fn install_skill_validates_fields_and_renders_frontmatter() {
    let store = Arc::new(MemoryStore {
        skills: vec![],
        project_scope: true,
    });
    let tool = InstallSkillTool::new(Some(store), None);

    let no_name = tool.execute(
        &CallContext::new(),
        &serde_json::json!({ "description": "d", "body": "b" }),
    );
    assert!(
        no_name
            .unwrap_err()
            .to_string()
            .contains("non-empty 'name'")
    );
    let no_desc = tool.execute(
        &CallContext::new(),
        &serde_json::json!({ "name": "s", "body": "b" }),
    );
    assert!(
        no_desc
            .unwrap_err()
            .to_string()
            .contains("non-empty 'description'")
    );
    let no_body = tool.execute(
        &CallContext::new(),
        &serde_json::json!({ "name": "s", "description": "d" }),
    );
    assert!(
        no_body
            .unwrap_err()
            .to_string()
            .contains("non-empty 'body'")
    );

    let ok = tool
        .execute(
            &CallContext::new(),
            &serde_json::json!({
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
            &serde_json::json!({ "name": "s", "description": "d", "body": "b", "scope": "project" }),
        )
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("scope='project' requires a workspace")
    );
}

#[test]
fn skill_file_frontmatter_emits_subagent_only_fields_conditionially() {
    let inline = render_skill_file(&SkillFileOptions {
        name: "commit".to_owned(),
        description: "d".to_owned(),
        body: "body".to_owned(),
        run_as: SkillRunAs::Inline,
        model: "m".to_owned(),
        effort: "high".to_owned(),
        allowed_tools: vec!["read_file".to_owned()],
        ..Default::default()
    });
    assert!(!inline.contains("model:"));
    assert!(!inline.contains("allowed-tools:"));

    let sub = render_skill_file(&SkillFileOptions {
        name: "explore".to_owned(),
        description: "d".to_owned(),
        body: "body".to_owned(),
        run_as: SkillRunAs::Subagent,
        model: "m".to_owned(),
        effort: "high".to_owned(),
        allowed_tools: vec!["read_file".to_owned(), "grep".to_owned()],
        read_only: true,
        color: "blue".to_owned(),
        invocation: "manual".to_owned(),
    });
    assert!(sub.contains("runAs: \"subagent\""));
    assert!(sub.contains("model: \"m\""));
    assert!(sub.contains("read-only: true"));
    assert!(sub.contains("allowed-tools: [\"read_file\", \"grep\"]"));
    assert!(sub.contains("color: \"blue\""));
    assert!(sub.contains("invocation: \"manual\""));
}
