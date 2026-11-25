//! The browser face (Go `internal/tools/builtin/browser.go`,
//! `internal/platform/browser/`): `browser_open`, `browser_read`,
//! `browser_act`.
//!
//! The snapshot/log/request/change renderers, the argument validation and the
//! refusal identities are pure here; the browser process, tabs, element refs,
//! screenshots and input dispatch are host seams behind [`BrowserHost`]. A
//! missing seam fails closed with the engine-missing refusal.

#![forbid(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use crate::abi::{
    CallContext, Capability, CapabilitySet, PlanModeClassifier, SequentialTool, Tool, ToolResult,
};
use crate::refusal::Refusal;

/// `browser_open`, the tool name the model calls.
pub const BROWSER_OPEN_TOOL_NAME: &str = "browser_open";
/// `browser_read`, the tool name the model calls.
pub const BROWSER_READ_TOOL_NAME: &str = "browser_read";
/// `browser_act`, the tool name the model calls.
pub const BROWSER_ACT_TOOL_NAME: &str = "browser_act";

// --- refusal identities (Go `platform/browser/failure.go`) ------------------

pub const CODE_BROWSER_ENGINE_MISSING: &str = "browser.engine_missing";
pub const CODE_BROWSER_ENGINE_FAILED: &str = "browser.engine_failed";
pub const CODE_BROWSER_PROFILE_BUSY: &str = "browser.profile_busy";
pub const CODE_BROWSER_URL_REFUSED: &str = "browser.url_refused";
pub const CODE_BROWSER_NETWORK_PATH: &str = "browser.network_path";
pub const CODE_BROWSER_NAVIGATION_FAILED: &str = "browser.navigation_failed";
pub const CODE_BROWSER_NAVIGATION_TIMEOUT: &str = "browser.navigation_timeout";
pub const CODE_BROWSER_NO_TAB: &str = "browser.no_tab";
pub const CODE_BROWSER_TOO_MANY_TABS: &str = "browser.too_many_tabs";
pub const CODE_BROWSER_TAB_CLOSED: &str = "browser.tab_closed";
pub const CODE_BROWSER_UNKNOWN_REF: &str = "browser.unknown_ref";
pub const CODE_BROWSER_STALE_REF: &str = "browser.stale_ref";
pub const CODE_BROWSER_NOT_VISIBLE: &str = "browser.not_visible";
pub const CODE_BROWSER_COVERED: &str = "browser.covered";
pub const CODE_BROWSER_NOT_SELECTABLE: &str = "browser.not_selectable";
pub const CODE_BROWSER_NO_SUCH_OPTION: &str = "browser.no_such_option";
pub const CODE_BROWSER_DIALOG_OPEN: &str = "browser.dialog_open";
pub const CODE_BROWSER_NO_DIALOG: &str = "browser.no_dialog";
pub const CODE_BROWSER_WAIT_TIMEOUT: &str = "browser.wait_timeout";
pub const CODE_BROWSER_BAD_STEP: &str = "browser.bad_step";
pub const CODE_BROWSER_SCRIPT_FAILED: &str = "browser.script_failed";
pub const CODE_BROWSER_SCRIPT_TIMEOUT: &str = "browser.script_timeout";
pub const CODE_BROWSER_ORIGIN_CHANGED: &str = "browser.origin_changed";
pub const CODE_BROWSER_UNCONFIRMED_SECRET: &str = "browser.unconfirmed_secret";

/// Go `errNoBrowser.Detail`, verbatim.
pub const MSG_NO_BROWSER: &str = "no browser is available in this runtime";

const BROWSER_FIND_LINES: usize = 40;
const BROWSER_REQUEST_LINES: usize = 40;
const BROWSER_SNAPSHOT_LINES: usize = 250;
const BROWSER_MAX_LINES: usize = 1000;
const BROWSER_CHANGE_LINES: usize = 80;

/// Go `browserOpen.Description()`, verbatim.
pub const BROWSER_OPEN_DESCRIPTION: &str = "Load a page and return its snapshot, one element per line with [eN] refs. url: http(s), about:blank or a workspace file; no url switches to tab; close closes. The user sees tabs: web_fetch reads static text, close tabs when done.";

/// Go `browserRead.Description()`, verbatim.
pub const BROWSER_READ_DESCRIPTION: &str = "Read a browser page: snapshot (default; page with offset/limit, or ref for one subtree), find (lines carrying text, with their refs), screenshot (browser_act x/y use its pixels, and the person is shown it too), logs (errors since you last read them), network (what it requested and what came back; text narrows by URL), tabs.";

/// Go `browserAct.Description()`, verbatim.
pub const BROWSER_ACT_DESCRIPTION: &str = "Operate a page with real input. Ordered steps stop at the first failure; target by ref, or screenshot x/y. Returns results, page errors and changes. fill replaces, type inserts; press takes keys like Enter or Control+a; select sets a <select>; scroll takes delta_y or delta_x; drag ends at to_ref or to_x/to_y; upload gives a file input workspace files; dialog answers prompts; eval executes arbitrary page JavaScript from script and awaits promises; resize sets the viewport to width×height; back, forward and reload move this tab.";

const BROWSER_OPEN_SCHEMA: &str = r#"{"type":"object","properties":{"url":{"type":"string"},"tab":{"type":"string","description":"t1, t2...; default active"},"new_tab":{"type":"boolean"},"close":{"type":"boolean"}}}"#;
const BROWSER_READ_SCHEMA: &str = r#"{"type":"object","properties":{"what":{"type":"string","enum":["snapshot","find","screenshot","logs","network","tabs"]},"tab":{"type":"string"},"ref":{"type":"string"},"text":{"type":"string"},"offset":{"type":"integer"},"limit":{"type":"integer"}}}"#;
pub const BROWSER_ACT_SCHEMA: &str = r#"{"type":"object","properties":{"tab":{"type":"string"},"steps":{"type":"array","items":{"type":"object","properties":{"action":{"type":"string","enum":["click","double_click","hover","fill","type","press","select","scroll","drag","upload","wait","wait_for","dialog","eval","resize","back","forward","reload"]},"ref":{"type":"string"},"text":{"type":"string"},"script":{"type":"string"},"key":{"type":"string"},"values":{"type":"array","items":{"type":"string"}},"x":{"type":"number"},"y":{"type":"number"},"delta_x":{"type":"number"},"delta_y":{"type":"number"},"to_ref":{"type":"string"},"to_x":{"type":"number"},"to_y":{"type":"number"},"files":{"type":"array","items":{"type":"string"}},"ms":{"type":"integer"},"width":{"type":"integer"},"height":{"type":"integer"},"accept":{"type":"boolean"}},"required":["action"]}}},"required":["steps"]}"#;

// --- platform types (Go `platform/browser`) --------------------------------

/// One open tab (Go `browser.TabInfo`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TabInfo {
    pub id: String,
    pub target: String,
    pub url: String,
    pub title: String,
    pub active: bool,
}

/// A page snapshot: the lines the accessibility tree exposed, plus any open
/// dialog (Go `browser.Snapshot`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub tab: TabInfo,
    pub lines: Vec<String>,
    pub dialog: Option<Dialog>,
}

/// A page dialog the input must answer before the page takes more input (Go
/// `browser.Dialog`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dialog {
    pub kind: String,
    pub message: String,
}

/// One console/error entry (Go `browser.LogEntry`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogEntry {
    pub seq: i64,
    pub kind: String,
    pub level: String,
    pub text: String,
}

/// One network request the page made (Go `browser.Request`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub kind: String,
    pub status: i64,
    pub mime: String,
    pub bytes: i64,
    pub failed: String,
    pub millis: i64,
    pub page: i64,
}

/// What changed between two snapshots (Go `browser.Changes`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    pub new_document: bool,
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

/// One input step (Go `browser.Step`).
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct BrowserStep {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub r#ref: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub script: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub values: Vec<String>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub delta_x: Option<f64>,
    #[serde(default)]
    pub delta_y: Option<f64>,
    #[serde(rename = "to_ref", default)]
    pub to_ref: String,
    #[serde(default)]
    pub to_x: Option<f64>,
    #[serde(default)]
    pub to_y: Option<f64>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub ms: i64,
    #[serde(default)]
    pub width: i64,
    #[serde(default)]
    pub height: i64,
    #[serde(default)]
    pub accept: bool,
}

/// The outcome of an act call (Go `browser.ActResult`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActResult {
    pub tab: TabInfo,
    pub done: usize,
    pub notes: Vec<String>,
    pub logs: Vec<LogEntry>,
    pub opened: Vec<String>,
    pub dialog: Option<Dialog>,
    pub failed_at: isize,
}

// --- host seam ---------------------------------------------------------------

/// A host-side browser failure carrying its typed code (Go `browser.Failure`).
#[derive(Debug, Clone, Default)]
pub struct BrowserFailure {
    pub code: String,
    pub detail: String,
}

impl std::fmt::Display for BrowserFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for BrowserFailure {}

/// The browser session the tools drive (Go `*browser.Session`). Every method
/// runs against the host's real browser process; the tool layer never invents
/// snapshots or refs.
pub trait BrowserHost: Send + Sync {
    fn close_tab(&self, tab: &str) -> Result<Vec<TabInfo>, BrowserFailure>;
    fn switch(&self, tab: &str) -> Result<TabInfo, BrowserFailure>;
    fn open(&self, url: &str, tab: &str, new_tab: bool) -> Result<TabInfo, BrowserFailure>;
    fn snapshot(&self, tab: &str, r#ref: &str) -> Result<Snapshot, BrowserFailure>;
    fn screenshot(&self, tab: &str) -> Result<(String, TabInfo), BrowserFailure>;
    fn logs(&self, tab: &str) -> Result<(Vec<LogEntry>, TabInfo), BrowserFailure>;
    fn requests(&self, tab: &str) -> Result<(Vec<Request>, TabInfo), BrowserFailure>;
    fn tabs(&self) -> Vec<TabInfo>;
    fn act(&self, tab: &str, steps: Vec<BrowserStep>) -> Result<ActResult, BrowserFailure>;
    fn refresh(&self, tab_id: &str) -> Result<(Snapshot, Changes), BrowserFailure>;
}

/// The browser host bound to one call, when installed.
pub fn browser_host_from(ctx: &CallContext) -> Option<Arc<dyn BrowserHost>> {
    ctx.value()?.downcast_ref::<Arc<dyn BrowserHost>>().cloned()
}

fn no_browser() -> Refusal {
    Refusal::code(CODE_BROWSER_ENGINE_MISSING, MSG_NO_BROWSER)
}

fn invalid_err(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(std::io::Error::other(message.into()))
}

fn bad_step(detail: impl Into<String>) -> Refusal {
    Refusal::code(CODE_BROWSER_BAD_STEP, detail)
}

// --- pure renderers (Go `browser.go:383–557`) -------------------------------

/// One line naming a tab: id, title, URL (Go `tabLine`).
pub fn tab_line(info: &TabInfo) -> String {
    let title = if info.title.is_empty() {
        "(untitled)"
    } else {
        &info.title
    };
    format!("Tab {}: {} — {}", info.id, title, info.url)
}

/// Byte size for a person or a model to read, 1024 base, rounded down (Go
/// `textutil.HumanBytes`).
pub fn human_bytes(n: i64) -> String {
    const UNIT: i64 = 1024;
    if n < UNIT {
        return format!("{n} B");
    }
    let mut div = UNIT;
    let mut exp = 0usize;
    let mut v = n / UNIT;
    while v >= UNIT && exp < 4 {
        div *= UNIT;
        exp += 1;
        v /= UNIT;
    }
    let value = n as f64 / div as f64;
    let unit = "KMGTP".chars().nth(exp).unwrap_or('P');
    if value >= 100.0 {
        format!("{value:.0} {unit}B")
    } else {
        format!("{value:.1} {unit}B")
    }
}

/// Where the wanted text sits on a long page: matching lines with refs and
/// positions (Go `renderFound`).
pub fn render_found(snap: &Snapshot, text: &str, limit: i64) -> String {
    let limit = if limit <= 0 {
        BROWSER_FIND_LINES
    } else {
        limit as usize
    };
    let mut b = String::new();
    b.push_str(&tab_line(&snap.tab));
    b.push('\n');
    let want = text.to_lowercase();
    let mut found = 0usize;
    for (i, line) in snap.lines.iter().enumerate() {
        if !line.to_lowercase().contains(&want) {
            continue;
        }
        found += 1;
        if found > limit {
            continue;
        }
        b.push_str(&format!("{}: {}\n", i + 1, line.trim_end_matches(' ')));
    }
    match found {
        0 => b.push_str(&format!(
            "No line of this page's {} carries {text:?}.\n",
            snap.lines.len()
        )),
        n if n > limit => b.push_str(&format!(
            "{limit} of {n} matching lines; the numbers are this page's own, so snapshot offset=0 reads around the first.\n"
        )),
        n => b.push_str(&format!("{n} line(s) of this page's {}.\n", snap.lines.len())),
    }
    b
}

/// The page snapshot as model text, paged by offset/limit (Go `renderSnapshot`).
pub fn render_snapshot(snap: &Snapshot, offset: i64, limit: i64) -> String {
    let mut b = String::new();
    b.push_str(&tab_line(&snap.tab));
    b.push('\n');
    if let Some(dialog) = &snap.dialog {
        b.push_str(&format!(
            "A {} dialog is open: {:?}. Answer it with a browser_act dialog step.\n",
            dialog.kind, dialog.message
        ));
    }
    let total = snap.lines.len();
    if total == 0 {
        b.push_str("The page shows nothing.\n");
        return b;
    }
    let start = (offset.max(0) as usize).min(total);
    let end = (start + (limit.min(BROWSER_MAX_LINES as i64) as usize)).min(total);
    if start > 0 || end < total {
        b.push_str(&format!("Lines {}-{} of {}.", start + 1, end, total));
        if end < total {
            b.push_str(&format!(" Continue with offset={end}."));
        }
        b.push('\n');
    }
    for line in &snap.lines[start..end] {
        b.push_str(line);
        b.push('\n');
    }
    b
}

/// The diff between two snapshots (Go `renderChanges`).
pub fn render_changes(snap: &Snapshot, c: &Changes) -> String {
    if c.new_document {
        let prefix = format!("{}\n", tab_line(&snap.tab));
        let body = render_snapshot(snap, 0, BROWSER_SNAPSHOT_LINES as i64);
        return format!("New page:\n{}", body.strip_prefix(&prefix).unwrap_or(&body));
    }
    if c.added.is_empty() && c.removed.is_empty() {
        return "The page's accessibility tree did not change.\n".to_owned();
    }
    let mut b = String::from("Changes since your last snapshot (+ appeared, - gone):\n");
    let mut shown = 0usize;
    for (mark, lines) in [("+ ", &c.added), ("- ", &c.removed)] {
        for line in lines.iter() {
            if shown == BROWSER_CHANGE_LINES {
                b.push_str(&format!(
                    "... {} more changed line(s); read a snapshot for the whole page\n",
                    c.added.len() + c.removed.len() - shown
                ));
                return b;
            }
            b.push_str(mark);
            b.push_str(line.trim_start_matches(' '));
            b.push('\n');
            shown += 1;
        }
    }
    b
}

/// Console entries as indented lines (Go `renderLogs`).
pub fn render_logs(entries: &[LogEntry]) -> String {
    let mut b = String::new();
    for e in entries {
        b.push_str(&format!("  [{}] {}: {}\n", e.level, e.kind, e.text));
    }
    b
}

/// Open tabs, active one starred (Go `renderTabs`).
pub fn render_tabs(tabs: &[TabInfo]) -> String {
    if tabs.is_empty() {
        return "No tabs are open.\n".to_owned();
    }
    let mut b = String::from("Tabs:\n");
    for t in tabs {
        let mark = if t.active { "*" } else { " " };
        b.push_str(&format!("{} {}\n", mark, tab_line(t)));
    }
    b
}

/// The network list, newest first, narrowed by URL text (Go `renderRequests`).
pub fn render_requests(requests: &[Request], match_text: &str, limit: i64) -> String {
    let limit = if limit <= 0 {
        BROWSER_REQUEST_LINES
    } else {
        limit as usize
    };
    let m = match_text.trim().to_lowercase();
    let mut lines: Vec<String> = Vec::new();
    let mut kept = 0usize;
    let mut page: i64 = -1;
    for r in requests.iter().rev() {
        if kept >= limit {
            break;
        }
        if !m.is_empty() && !r.url.to_lowercase().contains(&m) {
            continue;
        }
        if page >= 0 && r.page != page {
            lines.push("Earlier, before the page changed:".to_owned());
        }
        page = r.page;
        lines.push(format!("- {}", request_line(r)));
        kept += 1;
    }
    if lines.is_empty() {
        if !m.is_empty() {
            return format!("No request carried {match_text:?}.");
        }
        return "The page has requested nothing.".to_owned();
    }
    lines.join("\n")
}

/// One request line (Go `requestLine`).
pub fn request_line(r: &Request) -> String {
    let mut b = format!("{} {}", r.method, r.url);
    if !r.kind.is_empty() {
        b.push_str(&format!(" [{}]", r.kind));
    }
    if !r.failed.is_empty() {
        b.push_str(&format!(" → failed: {}", r.failed));
    } else if r.status == 0 {
        b.push_str(" → in flight");
    } else {
        b.push_str(&format!(" → {}", r.status));
        if !r.mime.is_empty() {
            b.push_str(&format!(" {}", r.mime));
        }
        if r.bytes > 0 {
            b.push_str(&format!(" {}", human_bytes(r.bytes)));
        }
    }
    if r.millis > 0 {
        b.push_str(&format!(" {}ms", r.millis));
    }
    b
}

// --- argument shapes ---------------------------------------------------------

#[derive(Deserialize, Default)]
struct OpenArgs {
    #[serde(default)]
    url: String,
    #[serde(default)]
    tab: String,
    #[serde(rename = "new_tab", default)]
    new_tab: bool,
    #[serde(default)]
    close: bool,
}

#[derive(Deserialize, Default)]
struct ReadArgs {
    #[serde(default)]
    what: String,
    #[serde(default)]
    tab: String,
    #[serde(default)]
    r#ref: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    offset: i64,
    #[serde(default)]
    limit: i64,
}

#[derive(Deserialize, Default)]
struct ActArgs {
    #[serde(default)]
    tab: String,
    #[serde(default)]
    steps: Vec<BrowserStep>,
}

// --- tools -------------------------------------------------------------------

/// The `browser_open` tool (Go `browserOpen`).
#[derive(Debug, Default, Clone, Copy)]
pub struct BrowserOpenTool;

pub fn browser_open_tool() -> Arc<dyn Tool> {
    Arc::new(BrowserOpenTool)
}

impl Tool for BrowserOpenTool {
    fn name(&self) -> &str {
        BROWSER_OPEN_TOOL_NAME
    }

    fn description(&self) -> &str {
        BROWSER_OPEN_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(BROWSER_OPEN_SCHEMA).expect("the browser_open schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let parsed: OpenArgs = serde_json::from_value(args.clone())
            .map_err(|e| invalid_err(format!("invalid args: {e}")))?;
        let Some(host) = browser_host_from(ctx) else {
            return Err(no_browser().into());
        };

        if parsed.close {
            let tabs = host.close_tab(&parsed.tab)?;
            return Ok(render_tabs(&tabs));
        }
        if parsed.url.is_empty() {
            if parsed.tab.is_empty() {
                return Err(bad_step("give a url to load, or a tab to switch to").into());
            }
            host.switch(&parsed.tab)?;
        } else {
            match host.open(&parsed.url, &parsed.tab, parsed.new_tab) {
                Ok(_) => {}
                Err(err) => {
                    // A navigation timeout keeps whatever snapshot the host held,
                    // as Go does; other failures return straight to the model.
                    if err.code != CODE_BROWSER_NAVIGATION_TIMEOUT {
                        return Err(Box::new(err));
                    }
                    let snap = host.snapshot(&parsed.tab, "")?;
                    return Ok(render_snapshot(&snap, 0, BROWSER_SNAPSHOT_LINES as i64));
                }
            }
        }
        let snap = host.snapshot(&parsed.tab, "")?;
        let mut out = render_snapshot(&snap, 0, BROWSER_SNAPSHOT_LINES as i64);
        let tabs = host.tabs();
        if tabs.len() > 1 {
            out.push('\n');
            out.push_str(&render_tabs(&tabs));
        }
        Ok(out)
    }

    fn read_only(&self) -> bool {
        false
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default()
            .with(
                "sequential",
                Arc::new(BrowserSequential) as Arc<dyn SequentialTool>,
            )
            .with(
                "plan_mode_safe",
                Arc::new(BrowserUnsafe) as Arc<dyn PlanModeClassifier>,
            )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The `browser_read` tool (Go `browserRead`).
#[derive(Debug, Default, Clone, Copy)]
pub struct BrowserReadTool;

pub fn browser_read_tool() -> Arc<dyn Tool> {
    Arc::new(BrowserReadTool)
}

impl Tool for BrowserReadTool {
    fn name(&self) -> &str {
        BROWSER_READ_TOOL_NAME
    }

    fn description(&self) -> &str {
        BROWSER_READ_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(BROWSER_READ_SCHEMA).expect("the browser_read schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let parsed: ReadArgs = serde_json::from_value(args.clone())
            .map_err(|e| invalid_err(format!("invalid args: {e}")))?;
        let Some(host) = browser_host_from(ctx) else {
            return Err(no_browser().into());
        };

        match parsed.what.as_str() {
            "" | "snapshot" => {
                let snap = host.snapshot(&parsed.tab, &parsed.r#ref)?;
                let limit = if parsed.limit > 0 {
                    (parsed.limit as usize).min(BROWSER_MAX_LINES)
                } else {
                    BROWSER_SNAPSHOT_LINES
                };
                Ok(render_snapshot(&snap, parsed.offset, limit as i64))
            }
            "screenshot" => {
                let (_url, info) = host.screenshot(&parsed.tab)?;
                // The image itself rides the host's detailed-result channel; the
                // caption names the tab the shot showed.
                Ok(format!("{}\n[image: screenshot]", tab_line(&info)))
            }
            "logs" => {
                let (entries, info) = host.logs(&parsed.tab)?;
                if entries.is_empty() {
                    Ok(format!("{}\nNothing new reported.", tab_line(&info)))
                } else {
                    Ok(format!("{}\n{}", tab_line(&info), render_logs(&entries)))
                }
            }
            "find" => {
                if parsed.text.trim().is_empty() {
                    return Err(bad_step("find needs the text to look for").into());
                }
                let snap = host.snapshot(&parsed.tab, &parsed.r#ref)?;
                Ok(render_found(&snap, &parsed.text, parsed.limit))
            }
            "network" => {
                let (requests, info) = host.requests(&parsed.tab)?;
                Ok(format!(
                    "{}\n{}",
                    tab_line(&info),
                    render_requests(&requests, &parsed.text, parsed.limit)
                ))
            }
            "tabs" => Ok(render_tabs(&host.tabs())),
            other => Err(bad_step(format!(
                "what={other:?} is not one of snapshot, find, screenshot, logs, network, tabs"
            ))
            .into()),
        }
    }

    fn read_only(&self) -> bool {
        true
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default().with(
            "sequential",
            Arc::new(BrowserSequential) as Arc<dyn SequentialTool>,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The `browser_act` tool (Go `browserAct`).
#[derive(Debug, Default, Clone, Copy)]
pub struct BrowserActTool;

pub fn browser_act_tool() -> Arc<dyn Tool> {
    Arc::new(BrowserActTool)
}

impl Tool for BrowserActTool {
    fn name(&self) -> &str {
        BROWSER_ACT_TOOL_NAME
    }

    fn description(&self) -> &str {
        BROWSER_ACT_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(BROWSER_ACT_SCHEMA).expect("the browser_act schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let parsed: ActArgs = serde_json::from_value(args.clone())
            .map_err(|e| invalid_err(format!("invalid args: {e}")))?;
        if parsed.steps.is_empty() {
            return Err(bad_step("steps is empty").into());
        }
        let Some(host) = browser_host_from(ctx) else {
            return Err(no_browser().into());
        };

        let res = host.act(&parsed.tab, parsed.steps.clone())?;
        let mut out = String::new();
        out.push_str(&format!(
            "Completed {} of {} step(s).\n",
            res.done,
            parsed.steps.len()
        ));
        for (i, note) in res.notes.iter().enumerate() {
            out.push_str(&format!("  {}. {note}\n", i + 1));
        }
        if res.failed_at >= 0 {
            out.push_str(&format!("Step {} failed.\n", res.failed_at + 1));
        }
        if !res.logs.is_empty() {
            out.push_str("The page reported:\n");
            out.push_str(&render_logs(&res.logs));
        }
        if !res.opened.is_empty() {
            out.push_str(&format!(
                "Opened tab(s): {} (now active)\n",
                res.opened.join(", ")
            ));
        }
        if res.tab.id.is_empty() {
            return Ok(out);
        }
        out.push_str(&format!("{}\n", tab_line(&res.tab)));
        if let Some(dialog) = &res.dialog {
            out.push_str(&format!(
                "A {} dialog is open: {:?}. Answer it with a dialog step.\n",
                dialog.kind, dialog.message
            ));
            return Ok(out);
        }
        match host.refresh(&res.tab.id) {
            Ok((snap, changes)) => out.push_str(&render_changes(&snap, &changes)),
            Err(err) => out.push_str(&format!("The page could not be read afterwards: {err}\n")),
        }
        Ok(out)
    }

    fn read_only(&self) -> bool {
        false
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default().with(
            "sequential",
            Arc::new(BrowserSequential) as Arc<dyn SequentialTool>,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct BrowserSequential;
impl Capability for BrowserSequential {}
impl SequentialTool for BrowserSequential {
    fn sequential(&self, _ctx: &CallContext, _args: &Value) -> bool {
        true
    }
}

struct BrowserUnsafe;
impl Capability for BrowserUnsafe {}
impl PlanModeClassifier for BrowserUnsafe {
    fn plan_mode_safe(&self) -> bool {
        false
    }
}

/// The browser face for one composition root.
pub fn browser_tools() -> Vec<Arc<dyn Tool>> {
    vec![browser_open_tool(), browser_read_tool(), browser_act_tool()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_are_valid_json() {
        let _ = BrowserOpenTool.schema();
        let _ = BrowserReadTool.schema();
        let _ = BrowserActTool.schema();
    }

    #[test]
    fn tab_line_falls_back_to_untitled() {
        let t = TabInfo {
            id: "t1".into(),
            url: "https://example.com".into(),
            ..Default::default()
        };
        assert_eq!(tab_line(&t), "Tab t1: (untitled) — https://example.com");
    }

    #[test]
    fn human_bytes_1024_base() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(153600), "150 KB");
    }

    #[test]
    fn render_requests_newest_first_and_filters() {
        let reqs = vec![
            Request {
                method: "GET".into(),
                url: "https://a/1".into(),
                status: 200,
                page: 1,
                ..Default::default()
            },
            Request {
                method: "GET".into(),
                url: "https://b/2".into(),
                status: 404,
                page: 1,
                ..Default::default()
            },
        ];
        let out = render_requests(&reqs, "b", 40);
        assert!(out.starts_with("- GET https://b/2 → 404"));
        assert!(!out.contains("a/1"));
    }

    #[test]
    fn render_found_reports_missing_text() {
        let snap = Snapshot {
            tab: TabInfo {
                id: "t1".into(),
                title: "T".into(),
                url: "u".into(),
                ..Default::default()
            },
            lines: vec!["one".into(), "two".into()],
            ..Default::default()
        };
        let out = render_found(&snap, "nope", 40);
        assert!(out.contains("No line of this page's 2 carries \"nope\"."));
    }
}
