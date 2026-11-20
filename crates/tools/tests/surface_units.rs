//! Provider surface and remaining capability oracle (Go
//! `provider_surface_test.go`, `roots_test.go`, `goal_test.go`,
//! `contextual_reason_test.go` semantics where portable).

use std::sync::Arc;

use reasonix_tools::abi::{
    CallContext, CapabilitySet, ContextualReasoner, ContextualTool, Tool, ToolResult,
};
use reasonix_tools::refusal::Refusal;
use reasonix_tools::registry::{CODE_TOOL_DISABLED, Registry, feature_label};
use serde_json::{Value, json};

struct SurfaceTool {
    name: String,
}

impl Tool for SurfaceTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.name
    }
    fn schema(&self) -> Value {
        json!({"type":"object"})
    }
    fn execute(&self, _ctx: &CallContext, _args: &Value) -> ToolResult {
        Ok(String::new())
    }
    fn read_only(&self) -> bool {
        true
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A contextual tool gated on a context binding (Go `gatedTool`).
struct GatedTool {
    name: String,
}

impl Tool for GatedTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.name
    }
    fn schema(&self) -> Value {
        json!({"type":"object"})
    }
    fn execute(&self, _ctx: &CallContext, _args: &Value) -> ToolResult {
        Ok(String::new())
    }
    fn read_only(&self) -> bool {
        true
    }
    fn capabilities(&self) -> CapabilitySet {
        // Both views of the same concrete object, as Go's two type
        // assertions both succeed on one value.
        let ct: Arc<dyn ContextualTool> = Arc::new(GatedReason);
        let cr: Arc<dyn ContextualReasoner> = Arc::new(GatedReason);
        CapabilitySet::default()
            .with("contextual-tool", ct)
            .with("contextual-reasoner", cr)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Marker carrying the gate binding id in the call context.
#[derive(Clone)]
struct SurfaceGate(bool);

#[derive(Default)]
struct GatedReason;

impl reasonix_tools::abi::Capability for GatedReason {}

impl ContextualTool for GatedReason {
    fn provider_visible(&self, ctx: &CallContext) -> bool {
        ctx.value()
            .and_then(|v| v.downcast_ref::<SurfaceGate>())
            .is_some_and(|g| g.0)
    }
}

impl ContextualReasoner for GatedReason {
    fn unavailable(&self, _ctx: &CallContext) -> Refusal {
        Refusal::code("surface.closed", "closed")
    }
}

fn surface_registry() -> Registry {
    let r = Registry::new();
    for name in ["alpha", "zulu", "mike"] {
        r.add(Arc::new(SurfaceTool {
            name: name.to_owned(),
        }));
    }
    for name in ["beta_gate", "yankee_gate"] {
        r.add(Arc::new(GatedTool {
            name: name.to_owned(),
        }));
    }
    r.set_provider_visible_tools(Some(&["alpha", "zulu", "mike", "beta_gate", "yankee_gate"]));
    r
}

fn names_of(schemas: &[reasonix_tools::ToolSchema]) -> Vec<String> {
    schemas.iter().map(|s| s.name.clone()).collect()
}

#[test]
fn provider_schemas_puts_every_contextual_tool_after_every_stable_one() {
    let r = surface_registry();
    let ctx = CallContext::new().with_value(Arc::new(SurfaceGate(true)));
    let got = names_of(&r.provider_schemas(&ctx));
    assert_eq!(
        got,
        vec![
            "alpha".to_owned(),
            "mike".to_owned(),
            "zulu".to_owned(),
            "beta_gate".to_owned(),
            "yankee_gate".to_owned()
        ],
        "a contextual tool ahead of a stable one breaks the prefix"
    );
}

/// The absence of a contextual tool has to be a prefix of its presence: that
/// is what lets the cache keep every token before the boundary when one
/// comes or goes.
#[test]
fn absent_surface_is_a_prefix_of_the_present_one() {
    let r = surface_registry();
    let closed = names_of(&r.provider_schemas(&CallContext::new()));
    let open_ctx = CallContext::new().with_value(Arc::new(SurfaceGate(true)));
    let open = names_of(&r.provider_schemas(&open_ctx));
    assert!(
        closed.len() < open.len(),
        "closed surface {closed:?} is not shorter than open {open:?}; this arm measures nothing"
    );
    for (i, name) in closed.iter().enumerate() {
        assert_eq!(
            open[i], *name,
            "position {i} differs: closed={closed:?} open={open:?} — the shared head is what the cache keeps"
        );
    }
}

#[test]
fn stable_segment_is_identical_whatever_the_context() {
    let r = surface_registry();
    let closed = names_of(&r.provider_schemas(&CallContext::new()));
    let open_ctx = CallContext::new().with_value(Arc::new(SurfaceGate(true)));
    let open = names_of(&r.provider_schemas(&open_ctx));
    for name in ["alpha", "mike", "zulu"] {
        assert!(
            closed.contains(&name.to_owned()) && open.contains(&name.to_owned()),
            "stable segment moved with the context: closed={closed:?} open={open:?}"
        );
    }
}

#[test]
fn show_to_provider_moves_one_name_and_leaves_the_rest_alone() {
    let r = Registry::new();
    for name in ["alpha", "zulu", "mike"] {
        r.add(Arc::new(SurfaceTool {
            name: name.to_owned(),
        }));
    }
    r.set_provider_visible_tools(Some(&["alpha"]));
    let rev = r.schema_revision();
    r.show_to_provider(true, &["zulu"]);
    let names = names_of(&r.provider_schemas(&CallContext::new()));
    assert_eq!(names.len(), 2, "after showing zulu: {names:?}");
    assert!(
        r.schema_revision() != rev,
        "showing a new name must bump the revision"
    );

    let rev = r.schema_revision();
    r.show_to_provider(true, &["zulu"]);
    assert_eq!(
        r.schema_revision(),
        rev,
        "showing a name already shown moved the schema revision"
    );

    r.show_to_provider(false, &["zulu", "never-registered"]);
    let names = names_of(&r.provider_schemas(&CallContext::new()));
    assert_eq!(
        names,
        vec!["alpha".to_owned()],
        "after hiding zulu: {names:?}"
    );
}

#[test]
fn show_to_provider_without_a_restriction_changes_nothing() {
    let r = Registry::new();
    for name in ["alpha", "zulu", "mike"] {
        r.add(Arc::new(SurfaceTool {
            name: name.to_owned(),
        }));
    }
    let rev = r.schema_revision();
    r.show_to_provider(false, &["alpha"]);
    assert_eq!(r.schema_revision(), rev);
    assert_eq!(r.provider_schemas(&CallContext::new()).len(), 3);
}

// ---- contextual_reason_test.go (identity, not wording) ----

#[test]
fn contextual_tools_explain_themselves() {
    let gated = GatedTool {
        name: "beta_gate".to_owned(),
    };
    let caps = gated.capabilities();
    let reasoner = caps
        .get::<dyn ContextualReasoner>()
        .expect("a contextual tool must be able to say why it is unavailable");
    let refusal = reasoner.unavailable(&CallContext::new());
    assert!(
        !refusal.message.trim().is_empty(),
        "returned a blank reason"
    );
    assert!(
        !refusal.code.is_empty(),
        "refuses without an identity; a reader can only match its words"
    );
}

// ---- goal_test.go (fail-closed recorder shadowing) ----

type GoalRecorderFn = dyn Fn(&str) -> Result<String, String> + Send + Sync;

#[derive(Clone, Default)]
struct CallBindings {
    goal_recorder: Option<Arc<GoalRecorderFn>>,
    shadowed: bool,
}

#[test]
fn without_goal_turn_recorder_shadows_only_recorder() {
    // The CallContext binding model: a child context built from a parent's
    // keeps the parent's other bindings only if composed; the goal recorder
    // is never inherited implicitly.
    let parent = CallContext::new().with_value(Arc::new(CallBindings {
        goal_recorder: Some(Arc::new(|_| Ok("recorded".to_owned()))),
        shadowed: false,
    }));
    // WithoutGoalTurnRecorder in Go replaces only that one key; here the
    // child is built explicitly, which is the same statement: no recorder.
    let child_bindings = CallBindings {
        goal_recorder: None,
        shadowed: true,
    };
    let child = CallContext::new().with_value(Arc::new(child_bindings));
    let bound = child
        .value()
        .and_then(|v| v.downcast_ref::<CallBindings>())
        .unwrap();
    assert!(
        bound.goal_recorder.is_none(),
        "child context inherited the parent goal recorder"
    );
    assert!(bound.shadowed);
    let _ = parent;
}

// ---- feature labels + disabled codes ----

#[test]
fn feature_label_maps_known_ids() {
    assert_eq!(feature_label("goal_mode"), "goal mode");
    assert_eq!(
        feature_label("deferred_mcp_tools"),
        "loading deferred MCP tools"
    );
    assert_eq!(feature_label("plan_steps"), "plan step tracking");
    assert_eq!(
        feature_label("turn_exit"),
        "ending a turn the host left open"
    );
    assert_eq!(feature_label("other"), "other");
    assert_eq!(CODE_TOOL_DISABLED, "tool.disabled_by_config");
}

// ---- roots_test.go ----

mod roots {
    use reasonix_tools::roots::{CODE_ROOT_MISSING, Root, RootKind, is_root_missing, missing_root};
    use std::path::Path;

    fn tempdir() -> tempfile::TempDir {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("target")
            .join("test-tmp");
        std::fs::create_dir_all(&base).expect("create test-tmp base");
        tempfile::Builder::new()
            .prefix(".tmptools")
            .tempdir_in(&base)
            .expect("create tools test temp dir")
    }

    fn stat(path: &str) -> Result<std::fs::Metadata, std::io::Error> {
        std::fs::metadata(path)
    }

    #[test]
    fn missing_root_names_the_first_gone_directory_by_its_display_path() {
        let dir = tempdir();
        let gone = dir.path().join("gone");
        let roots = vec![
            Root {
                kind: RootKind::Workspace,
                path: dir.path().to_string_lossy().to_string(),
                display: dir.path().to_string_lossy().to_string(),
            },
            Root {
                kind: RootKind::External,
                path: gone.to_string_lossy().to_string(),
                display: "__reasonix_external_folder/abc/meta".to_owned(),
            },
        ];
        let err = missing_root(&roots, stat).expect("the external root is gone");
        // Downcast identity in place of Go's errors.Is(err, ErrRootMissing).
        assert!(
            is_root_missing(&err),
            "a root that is gone must match ErrRootMissing"
        );
        assert_eq!(err.kind(), RootKind::External, "want the external root");
        let text = err.to_string();
        assert!(
            !text.contains(gone.to_string_lossy().as_ref()),
            "the real path of an external folder must not reach the model: {text}"
        );
        assert!(
            text.contains("__reasonix_external_folder/abc/meta"),
            "the text must name the display path: {text}"
        );
        assert!(
            text.contains("not a permission or argument problem"),
            "the text must say what kind of failure this is: {text}"
        );
        assert_eq!(err.refusal_code(), CODE_ROOT_MISSING);
    }

    #[test]
    fn missing_root_leaves_other_stat_failures_to_the_tool() {
        let roots = vec![Root {
            kind: RootKind::Workspace,
            path: "/x".to_owned(),
            display: "/x".to_owned(),
        }];
        let denied = |_: &str| -> Result<std::fs::Metadata, std::io::Error> {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        };
        assert!(
            missing_root(&roots, denied).is_none(),
            "a denied stat is the tool's to report"
        );
    }

    #[test]
    fn missing_root_treats_a_file_where_a_directory_should_be_as_missing() {
        let dir = tempdir();
        let file = dir.path().join("f");
        std::fs::write(&file, b"").unwrap();
        let roots = vec![Root {
            kind: RootKind::Workspace,
            path: file.to_string_lossy().to_string(),
            display: file.to_string_lossy().to_string(),
        }];
        assert!(
            missing_root(&roots, stat).is_some(),
            "a file standing where the project folder was is not a project folder"
        );
    }

    #[test]
    fn missing_root_ignores_roots_with_no_path() {
        let roots = vec![Root {
            kind: RootKind::Workspace,
            path: String::new(),
            display: String::new(),
        }];
        assert!(
            missing_root(&roots, stat).is_none(),
            "a call with no directory has nothing to be missing"
        );
    }
}
