//! The browser face at the tools boundary (Go `internal/tools/builtin/browser.go`,
//! `internal/platform/browser/`): tool cards and read-only/sequential decisions,
//! the argument validation order, renderer passthrough, and the engine-missing
//! seam refusal.

use std::sync::Arc;

use reasonix_tools::abi::{CallContext, Tool};
use reasonix_tools::builtin::{
    ActResult, BROWSER_ACT_TOOL_NAME, BROWSER_OPEN_TOOL_NAME, BROWSER_READ_TOOL_NAME,
    BrowserActTool, BrowserFailure, BrowserHost, BrowserOpenTool, BrowserReadTool, BrowserStep,
    Changes, LogEntry, Request, Snapshot, TabInfo,
};
use serde_json::json;

struct FakeBrowser {
    tabs: Vec<TabInfo>,
    snapshot: Snapshot,
}

impl Default for FakeBrowser {
    fn default() -> Self {
        let tab = TabInfo {
            id: "t1".into(),
            url: "https://example.com".into(),
            title: "Example".into(),
            active: true,
            ..Default::default()
        };
        Self {
            tabs: vec![tab.clone()],
            snapshot: Snapshot {
                tab,
                lines: vec!["[e1] Search box".into(), "[e2] Submit".into()],
                ..Default::default()
            },
        }
    }
}

impl BrowserHost for FakeBrowser {
    fn close_tab(&self, _tab: &str) -> Result<Vec<TabInfo>, BrowserFailure> {
        Ok(self.tabs.clone())
    }
    fn switch(&self, _tab: &str) -> Result<TabInfo, BrowserFailure> {
        Ok(self.tabs[0].clone())
    }
    fn open(&self, _url: &str, _tab: &str, _new_tab: bool) -> Result<TabInfo, BrowserFailure> {
        Ok(self.tabs[0].clone())
    }
    fn snapshot(&self, _tab: &str, _reference: &str) -> Result<Snapshot, BrowserFailure> {
        Ok(self.snapshot.clone())
    }
    fn screenshot(&self, _tab: &str) -> Result<(String, TabInfo), BrowserFailure> {
        Ok(("screenshot.png".into(), self.tabs[0].clone()))
    }
    fn logs(&self, _tab: &str) -> Result<(Vec<LogEntry>, TabInfo), BrowserFailure> {
        Ok((Vec::new(), self.tabs[0].clone()))
    }
    fn requests(&self, _tab: &str) -> Result<(Vec<Request>, TabInfo), BrowserFailure> {
        Ok((Vec::new(), self.tabs[0].clone()))
    }
    fn tabs(&self) -> Vec<TabInfo> {
        self.tabs.clone()
    }
    fn act(&self, _tab: &str, _steps: Vec<BrowserStep>) -> Result<ActResult, BrowserFailure> {
        Ok(ActResult {
            tab: self.tabs[0].clone(),
            done: 1,
            notes: vec!["clicked submit".into()],
            ..Default::default()
        })
    }
    fn refresh(&self, _tab_id: &str) -> Result<(Snapshot, Changes), BrowserFailure> {
        Ok((self.snapshot.clone(), Changes::default()))
    }
}

fn ctx_with(host: Arc<FakeBrowser>) -> CallContext {
    let bound: Arc<Arc<dyn BrowserHost>> = Arc::new(host as Arc<dyn BrowserHost>);
    CallContext::new().with_value(bound)
}

#[test]
fn browser_open_card_writes_and_is_sequential() {
    let tool = BrowserOpenTool;
    assert_eq!(tool.name(), BROWSER_OPEN_TOOL_NAME);
    assert!(tool.description().contains("snapshot"));
    assert!(tool.schema().is_object());
    assert!(!tool.read_only());
}

#[test]
fn browser_read_card_is_read_only() {
    let tool = BrowserReadTool;
    assert_eq!(tool.name(), BROWSER_READ_TOOL_NAME);
    assert!(tool.description().contains("snapshot"));
    assert!(tool.schema().is_object());
    assert!(tool.read_only());
}

#[test]
fn browser_act_card_writes_and_rejects_empty_steps() {
    let tool = BrowserActTool;
    assert_eq!(tool.name(), BROWSER_ACT_TOOL_NAME);
    assert!(tool.description().contains("real input"));
    assert!(tool.schema().is_object());
    assert!(!tool.read_only());

    let err = tool
        .execute(&CallContext::new(), &json!({"steps": []}))
        .expect_err("empty steps refused");
    assert!(err.to_string().contains("steps is empty"));
}

#[test]
fn browser_read_snapshot_passes_through_the_bound_host() {
    let host = Arc::new(FakeBrowser::default());
    let out = BrowserReadTool
        .execute(&ctx_with(host), &json!({"what": "snapshot"}))
        .expect("snapshot renders");
    assert!(out.contains("Tab t1: Example — https://example.com"));
    assert!(out.contains("[e1] Search box"));
}

#[test]
fn browser_read_unknown_what_is_a_bad_step() {
    let host = Arc::new(FakeBrowser::default());
    let err = BrowserReadTool
        .execute(&ctx_with(host), &json!({"what": "screen"}))
        .expect_err("unknown what refused");
    assert!(
        err.to_string()
            .contains("is not one of snapshot, find, screenshot, logs, network, tabs")
    );
}

#[test]
fn browser_read_find_requires_text() {
    let host = Arc::new(FakeBrowser::default());
    let err = BrowserReadTool
        .execute(&ctx_with(host), &json!({"what": "find", "text": ""}))
        .expect_err("find text required");
    assert!(err.to_string().contains("find needs the text to look for"));
}

#[test]
fn browser_act_reports_completed_steps() {
    let host = Arc::new(FakeBrowser::default());
    let out = BrowserActTool
        .execute(
            &ctx_with(host),
            &json!({"steps": [{"action": "click", "ref": "e2"}]}),
        )
        .expect("act runs against the seam");
    assert!(out.contains("Completed 1 of 1 step(s)."));
    assert!(out.contains("1. clicked submit"));
}

#[test]
fn browser_without_host_fails_closed_as_engine_missing() {
    let err = BrowserReadTool
        .execute(&CallContext::new(), &json!({"what": "tabs"}))
        .expect_err("seam absent refuses");
    assert!(err.to_string().contains("browser.engine_missing"));
    assert!(err.to_string().contains("no browser is available"));
}
