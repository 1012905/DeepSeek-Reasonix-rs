//! The computer-use face at the tools boundary (Go `internal/tools/builtin/computer.go`,
//! `internal/platform/computer/`): tool cards, argument validation, app/snapshot
//! renderers, and the unavailable seam refusal.

use std::sync::Arc;

use reasonix_tools::abi::{CallContext, Tool};
use reasonix_tools::builtin::{
    App, COMPUTER_ACT_TOOL_NAME, COMPUTER_READ_TOOL_NAME, ComputerActResult, ComputerActTool,
    ComputerHost, ComputerReadTool, ComputerSnapshot, ComputerStep, Effect, Rect, StepResult,
    Window,
};
use serde_json::json;

struct FakeComputer {
    apps: Vec<App>,
    snapshot: ComputerSnapshot,
}

impl Default for FakeComputer {
    fn default() -> Self {
        let app = App {
            pid: 42,
            bundle: "com.example.app".into(),
            name: "Example".into(),
            active: true,
            shown_bundle: "com.example.app".into(),
            shown_name: "Example".into(),
            windows: vec![Window {
                id: 1,
                title: "Main".into(),
                bounds: Rect {
                    width: 800.0,
                    height: 600.0,
                    ..Default::default()
                },
            }],
            ..Default::default()
        };
        let snapshot = ComputerSnapshot {
            app: app.clone(),
            lines: vec!["[r1] Name field".into(), "[r2] Save".into()],
            ..Default::default()
        };
        Self {
            apps: vec![app],
            snapshot,
        }
    }
}

impl ComputerHost for FakeComputer {
    fn apps(&self) -> Result<Vec<App>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.apps.clone())
    }
    fn snapshot(
        &self,
        _app: &str,
        _window: i64,
    ) -> Result<ComputerSnapshot, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.snapshot.clone())
    }
    fn screenshot(
        &self,
        _app: &str,
        _window: i64,
    ) -> Result<(String, App), Box<dyn std::error::Error + Send + Sync>> {
        Ok(("shot.png".into(), self.apps[0].clone()))
    }
    fn act(
        &self,
        _app: &str,
        _window: i64,
        _steps: Vec<ComputerStep>,
    ) -> Result<ComputerActResult, Box<dyn std::error::Error + Send + Sync>> {
        Ok(ComputerActResult {
            app: self.apps[0].clone(),
            done: 1,
            steps: vec![StepResult {
                note: "clicked Save".into(),
                effect: Effect {
                    class: "confirmed".into(),
                    ..Default::default()
                },
            }],
            ..Default::default()
        })
    }
}

fn ctx_with(host: Arc<FakeComputer>) -> CallContext {
    let bound: Arc<Arc<dyn ComputerHost>> = Arc::new(host as Arc<dyn ComputerHost>);
    CallContext::new().with_value(bound)
}

#[test]
fn computer_read_card_is_writing() {
    let tool = ComputerReadTool;
    assert_eq!(tool.name(), COMPUTER_READ_TOOL_NAME);
    assert!(tool.description().contains("approved application"));
    assert!(tool.schema().is_object());
    assert!(!tool.read_only());
}

#[test]
fn computer_act_card_writes_and_rejects_empty_steps() {
    let tool = ComputerActTool;
    assert_eq!(tool.name(), COMPUTER_ACT_TOOL_NAME);
    assert!(tool.description().contains("CUA Driver"));
    assert!(tool.schema().is_object());
    assert!(!tool.read_only());

    let err = tool
        .execute(
            &CallContext::new(),
            &json!({"app": "com.example.app", "steps": []}),
        )
        .expect_err("empty steps refused");
    assert!(err.to_string().contains("steps is empty"));
}

#[test]
fn computer_read_apps_lists_running_windows() {
    let host = Arc::new(FakeComputer::default());
    let out = ComputerReadTool
        .execute(&ctx_with(host), &json!({"what": "apps"}))
        .expect("apps render");
    assert!(out.starts_with("* com.example.app — Example"));
    assert!(out.contains("window 1 \"Main\" 800×600"));
}

#[test]
fn computer_read_snapshot_passes_through_the_bound_host() {
    let host = Arc::new(FakeComputer::default());
    let out = ComputerReadTool
        .execute(&ctx_with(host), &json!({"what": "snapshot"}))
        .expect("snapshot renders");
    assert!(out.contains("Example (com.example.app)"));
    assert!(out.contains("[r2] Save"));
}

#[test]
fn computer_read_unknown_what_is_a_bad_step() {
    let host = Arc::new(FakeComputer::default());
    let err = ComputerReadTool
        .execute(&ctx_with(host), &json!({"what": "desktop"}))
        .expect_err("unknown what refused");
    assert!(
        err.to_string()
            .contains("is not one of apps, snapshot, screenshot")
    );
}

#[test]
fn computer_act_reports_completed_step_and_effect() {
    let host = Arc::new(FakeComputer::default());
    let out = ComputerActTool
        .execute(
            &ctx_with(host),
            &json!({"app": "com.example.app", "steps": [{"action": "click", "ref": "r2"}]}),
        )
        .expect("act runs against the seam");
    assert!(out.contains("Completed 1 of 1 step(s)."));
    assert!(out.contains("1. clicked Save — took effect"));
}

#[test]
fn computer_without_host_fails_closed_as_unavailable() {
    let err = ComputerReadTool
        .execute(&CallContext::new(), &json!({"what": "apps"}))
        .expect_err("seam absent refuses");
    assert!(err.to_string().contains("computer.unavailable"));
    assert!(err.to_string().contains("CUA Driver"));
}
