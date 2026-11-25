//! The computer-use face (Go `internal/tools/builtin/computer.go`,
//! `internal/platform/computer/`): `computer_read`, `computer_act`.
//!
//! The app/snapshot/effect renderers, argument validation and refusal
//! identities are pure here; the CUA Driver, window capture, element refs
//! and input dispatch are host seams behind [`ComputerHost`]. A missing seam
//! fails closed with the unavailable refusal.

#![forbid(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use crate::abi::{CallContext, Capability, CapabilitySet, SequentialTool, Tool, ToolResult};
use crate::refusal::Refusal;

/// `computer_read`, the tool name the model calls.
pub const COMPUTER_READ_TOOL_NAME: &str = "computer_read";
/// `computer_act`, the tool name the model calls.
pub const COMPUTER_ACT_TOOL_NAME: &str = "computer_act";

// --- refusal identities (Go `platform/computer/failure.go`) -----------------

pub const CODE_COMPUTER_UNAVAILABLE: &str = "computer.unavailable";
pub const CODE_COMPUTER_UNSUPPORTED: &str = "computer.unsupported";
pub const CODE_COMPUTER_PERMISSION_MISSING: &str = "computer.permission_missing";
pub const CODE_COMPUTER_NO_APP: &str = "computer.no_app";
pub const CODE_COMPUTER_NO_WINDOW: &str = "computer.no_window";
pub const CODE_COMPUTER_APP_REFUSED: &str = "computer.app_refused";
pub const CODE_COMPUTER_UNKNOWN_REF: &str = "computer.unknown_ref";
pub const CODE_COMPUTER_STALE_REF: &str = "computer.stale_ref";
pub const CODE_COMPUTER_NO_ACTION: &str = "computer.no_action";
pub const CODE_COMPUTER_NEEDS_FRONT: &str = "computer.needs_front";
pub const CODE_COMPUTER_BLOCKED: &str = "computer.blocked";
pub const CODE_COMPUTER_ELEVATED: &str = "computer.elevated";
pub const CODE_COMPUTER_NO_ELEMENT: &str = "computer.no_element";
pub const CODE_COMPUTER_NEEDS_SCREENSHOT: &str = "computer.needs_screenshot";
pub const CODE_COMPUTER_BAD_STEP: &str = "computer.bad_step";
pub const CODE_COMPUTER_STOPPED: &str = "computer.stopped";
pub const CODE_COMPUTER_CAPTURE_FAILED: &str = "computer.capture_failed";
pub const CODE_COMPUTER_FAILED: &str = "computer.failed";
pub const CODE_COMPUTER_DRIVER_REFUSED: &str = "computer.driver_refused";
pub const CODE_COMPUTER_EFFECT_UNKNOWN: &str = "computer.effect_unknown";

/// Go `errNoComputer.Detail`, verbatim.
pub const MSG_NO_COMPUTER: &str =
    "computer use requires Reasonix Studio with its pinned CUA Driver";

const COMPUTER_SNAPSHOT_LINES: usize = 300;
const COMPUTER_MAX_STEPS: usize = 64;

/// Go `computerRead.Description()`, verbatim.
pub const COMPUTER_READ_DESCRIPTION: &str = "See an approved application listed by what=apps. Installed entries without a targetable window provide an exact launch identity for a separate computer_act launch step. Select window by id when it has multiple windows. snapshot returns session-bound element refs in brackets for computer_act. screenshot captures that exact window; computer_act x/y use pixels of this latest displayed image. Old tokens and captures expire on refresh or reconnect.";

/// Go `computerAct.Description()`, verbatim.
pub const COMPUTER_ACT_DESCRIPTION: &str = "Operate an approved application through CUA Driver. launch must be one separate step using an exact installed identity from computer_read what=apps; no window selector or extra arguments. Select window when the app has multiple windows. Use refs from its latest snapshot or coordinates from its latest screenshot. Background clicks, set_value, type/paste, key and scroll never silently escalate. pointer_click explicitly takes foreground control and requires separate approval. wait is bounded. Inspect fresh state after an unverified effect; never assume an action landed. Returns a snapshot afterwards except launch, which returns the verified running identity for a separately approved read.";

const COMPUTER_READ_SCHEMA: &str = r#"{"type":"object","properties":{"window":{"type":"integer","minimum":1},"what":{"type":"string","enum":["apps","snapshot","screenshot"]},"app":{"type":"string"}},"required":["what"]}"#;
const COMPUTER_ACT_SCHEMA: &str = r#"{"type":"object","properties":{"app":{"type":"string"},"window":{"type":"integer","minimum":1},"steps":{"type":"array","maxItems":64,"items":{"type":"object","properties":{"action":{"type":"string","enum":["launch","click","right_click","set_value","type","paste","key","scroll","wait","pointer_click"]},"ref":{"type":"string"},"text":{"type":"string"},"key":{"type":"string"},"x":{"type":"number"},"y":{"type":"number"},"amount":{"type":"number"},"ms":{"type":"integer"},"to_x":{"type":"number"},"to_y":{"type":"number"},"button":{"type":"string","enum":["left","right","middle"]},"times":{"type":"integer"},"seconds":{"type":"number"}},"required":["action"]}}},"required":["app","steps"]}"#;

// --- platform types (Go `platform/computer`) --------------------------------

/// A window rectangle in screen pixels (Go `computer.Rect`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// One visible window of an application (Go `computer.Window`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Window {
    pub id: i64,
    pub title: String,
    pub bounds: Rect,
}

/// One installed or running application (Go `computer.App`). The host seam
/// fills `shown_name`/`shown_bundle` after the bound-literal sanitization; the
/// tool layer renders them as handed over.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct App {
    pub pid: i32,
    pub bundle: String,
    pub exe: String,
    pub name: String,
    pub active: bool,
    pub launchable: bool,
    pub windows: Vec<Window>,
    /// Why the app may never be operated (Go `computer.Refused`), empty when
    /// allowed.
    pub refused_reason: String,
    /// Sanitized display name (Go `ShownName`).
    pub shown_name: String,
    /// Sanitized bundle identity (Go `ShownBundle`).
    pub shown_bundle: String,
}

/// A modal holding the application's input (Go `computer.Modal`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Modal {
    pub r#ref: String,
    pub title: String,
    pub blocks: bool,
}

impl std::fmt::Display for Modal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.title)
    }
}

/// One effect evidence note (Go `computer.Evidence`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    ValueReadback,
    ValueUnchanged,
    Unknown,
}

impl Evidence {
    fn as_str(self) -> &'static str {
        match self {
            Evidence::ValueReadback => "value read back",
            Evidence::ValueUnchanged => "value unchanged",
            Evidence::Unknown => "",
        }
    }
}

/// The driver's own account of an effect (Go `computer.DriverAction`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DriverAction {
    pub effect: String,
    pub route: String,
    pub path: String,
    pub summary: String,
    pub delivery_count: Option<i64>,
}

/// What one step did, as the driver observed it (Go `computer.Effect`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Effect {
    pub class: String,
    pub evidence: Vec<Evidence>,
    pub blocked_by: Option<Modal>,
    pub driver: Option<DriverAction>,
}

/// One completed step's note and observed effect (Go `computer.StepResult`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepResult {
    pub note: String,
    pub effect: Effect,
}

/// The outcome of an act call (Go `computer.ActResult`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ActResult {
    pub app: App,
    pub done: usize,
    pub steps: Vec<StepResult>,
    pub failed_at: isize,
}

/// The page/snapshot the host read back (Go `computer.Snapshot`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub app: App,
    pub modals: Vec<Modal>,
    pub lines: Vec<String>,
    pub truncated: bool,
    pub note: String,
}

/// One input step (Go `computer.Step`).
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct ComputerStep {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub r#ref: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub amount: Option<f64>,
    #[serde(default)]
    pub ms: i64,
    #[serde(default)]
    pub to_x: Option<f64>,
    #[serde(default)]
    pub to_y: Option<f64>,
    #[serde(default)]
    pub button: String,
    #[serde(default)]
    pub times: i64,
    #[serde(default)]
    pub seconds: Option<f64>,
}

/// Pointer actions that take foreground control and name a separate approval
/// subject (Go `computer.PointerSteps`).
pub fn pointer_steps(steps: &[ComputerStep]) -> bool {
    steps.iter().any(|s| {
        matches!(
            s.action.as_str(),
            "pointer_move" | "pointer_click" | "pointer_drag" | "pointer_position"
        )
    })
}

// --- host seam ---------------------------------------------------------------

/// The CUA driver session the tools drive (Go `*computer.Session`). Every
/// method runs against the host's real driver; the tool layer never invents
/// windows, refs or effects.
pub trait ComputerHost: Send + Sync {
    fn apps(&self) -> Result<Vec<App>, Box<dyn std::error::Error + Send + Sync>>;
    fn snapshot(
        &self,
        app: &str,
        window: i64,
    ) -> Result<Snapshot, Box<dyn std::error::Error + Send + Sync>>;
    fn screenshot(
        &self,
        app: &str,
        window: i64,
    ) -> Result<(String, App), Box<dyn std::error::Error + Send + Sync>>;
    fn act(
        &self,
        app: &str,
        window: i64,
        steps: Vec<ComputerStep>,
    ) -> Result<ActResult, Box<dyn std::error::Error + Send + Sync>>;
}

/// The computer host bound to one call, when installed.
pub fn computer_host_from(ctx: &CallContext) -> Option<Arc<dyn ComputerHost>> {
    ctx.value()?
        .downcast_ref::<Arc<dyn ComputerHost>>()
        .cloned()
}

fn no_computer() -> Refusal {
    Refusal::code(CODE_COMPUTER_UNAVAILABLE, MSG_NO_COMPUTER)
}

fn invalid_err(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(std::io::Error::other(message.into()))
}

fn bad_step(detail: impl Into<String>) -> Refusal {
    Refusal::code(CODE_COMPUTER_BAD_STEP, detail)
}

// --- pure renderers (Go `computer.go:187–273`) ------------------------------

const EFFECT_SAID: [(&str, &str); 3] = [
    ("confirmed", "took effect"),
    ("suspected_noop", "no effect seen"),
    ("unverifiable", "effect not verified"),
];

/// The app list as model text (Go `renderApps`).
pub fn render_apps(apps: &[App]) -> String {
    let mut b = String::new();
    for app in apps {
        let mark = if app.active { "*" } else { " " };
        b.push_str(&format!(
            "{} {} — {}",
            mark, app.shown_bundle, app.shown_name
        ));
        if !app.refused_reason.is_empty() {
            b.push_str(&format!(" (never operated: {})", app.refused_reason));
        }
        if app.pid == 0 && app.windows.is_empty() {
            if app.launchable {
                b.push_str(" (installed; no targetable window; launch separately)");
            } else {
                b.push_str(" (installed; no targetable window; launch unsupported)");
            }
        }
        b.push('\n');
        for w in &app.windows {
            let title = if w.title.is_empty() {
                "\"(untitled)\"".to_owned()
            } else {
                format!("{:?}", w.title)
            };
            b.push_str(&format!(
                "    window {} {} {:.0}×{:.0}\n",
                w.id, title, w.bounds.width, w.bounds.height
            ));
        }
    }
    if b.is_empty() {
        return "No applications are running with windows a person can see.\n".to_owned();
    }
    b
}

/// One step's observed effect as a trailing clause (Go `renderEffect`).
pub fn render_effect(e: &Effect) -> String {
    if e.class.is_empty() {
        return String::new();
    }
    let mut b = String::from(" — ");
    let said = EFFECT_SAID
        .iter()
        .find(|(k, _)| *k == e.class)
        .map(|(_, v)| *v)
        .unwrap_or(&e.class);
    b.push_str(said);
    for ev in &e.evidence {
        let text = ev.as_str();
        if !text.is_empty() {
            b.push_str(&format!(" ({text})"));
        }
    }
    if let Some(blocked) = &e.blocked_by {
        b.push_str(&format!(", into {blocked}"));
    }
    if let Some(driver) = &e.driver {
        b.push_str(&format!(
            " [CUA effect={} route={}]",
            driver.effect, driver.route
        ));
        if let Some(count) = driver.delivery_count {
            b.push_str(&format!(" delivered={count}"));
        }
    }
    b
}

/// The snapshot as model text (Go `renderComputerSnapshot`).
pub fn render_computer_snapshot(snap: &Snapshot) -> String {
    let mut b = String::new();
    b.push_str(&format!(
        "{} ({})\n",
        snap.app.shown_name, snap.app.shown_bundle
    ));
    for m in &snap.modals {
        b.push_str(&format!(
            "Blocked: {} holds this application's input; the window behind it takes none until it is answered.\n",
            m
        ));
    }
    if !snap.note.is_empty() {
        b.push_str(&snap.note);
        b.push('\n');
    }
    let lines: &[String] = if snap.lines.len() > COMPUTER_SNAPSHOT_LINES {
        &snap.lines[..COMPUTER_SNAPSHOT_LINES]
    } else {
        &snap.lines
    };
    for l in lines {
        b.push_str(l);
        b.push('\n');
    }
    if snap.truncated || snap.lines.len() > lines.len() {
        b.push_str("… the tree is longer than this; act on what is shown or take a screenshot\n");
    }
    b
}

/// The screenshot caption naming the front window (Go `screenshotCaption`).
pub fn screenshot_caption(app: &App) -> String {
    format!(
        "{} ({}): front window\n[image: screenshot]",
        app.shown_name, app.shown_bundle
    )
}

// --- argument shapes ---------------------------------------------------------

#[derive(Deserialize, Default)]
struct ReadArgs {
    #[serde(default)]
    window: i64,
    #[serde(default)]
    what: String,
    #[serde(default)]
    app: String,
}

#[derive(Deserialize, Default)]
struct ActArgs {
    #[serde(default)]
    app: String,
    #[serde(default)]
    window: i64,
    #[serde(default)]
    steps: Vec<ComputerStep>,
}

// --- tools -------------------------------------------------------------------

/// The `computer_read` tool (Go `computerRead`).
#[derive(Debug, Default, Clone, Copy)]
pub struct ComputerReadTool;

pub fn computer_read_tool() -> Arc<dyn Tool> {
    Arc::new(ComputerReadTool)
}

impl Tool for ComputerReadTool {
    fn name(&self) -> &str {
        COMPUTER_READ_TOOL_NAME
    }

    fn description(&self) -> &str {
        COMPUTER_READ_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(COMPUTER_READ_SCHEMA).expect("the computer_read schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let parsed: ReadArgs = serde_json::from_value(args.clone())
            .map_err(|e| invalid_err(format!("invalid args: {e}")))?;
        let Some(host) = computer_host_from(ctx) else {
            return Err(no_computer().into());
        };

        match parsed.what.as_str() {
            "apps" => {
                let apps = host.apps()?;
                Ok(render_apps(&apps))
            }
            "snapshot" => {
                let snap = host.snapshot(&parsed.app, parsed.window)?;
                Ok(render_computer_snapshot(&snap))
            }
            "screenshot" => {
                let (_shot, app) = host.screenshot(&parsed.app, parsed.window)?;
                // The image rides the host's detailed-result channel; the
                // caption names the window the shot showed.
                Ok(screenshot_caption(&app))
            }
            other => Err(bad_step(format!(
                "what={other:?} is not one of apps, snapshot, screenshot"
            ))
            .into()),
        }
    }

    fn read_only(&self) -> bool {
        false
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default().with(
            "sequential",
            Arc::new(ComputerSequential) as Arc<dyn SequentialTool>,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The `computer_act` tool (Go `computerAct`).
#[derive(Debug, Default, Clone, Copy)]
pub struct ComputerActTool;

pub fn computer_act_tool() -> Arc<dyn Tool> {
    Arc::new(ComputerActTool)
}

impl Tool for ComputerActTool {
    fn name(&self) -> &str {
        COMPUTER_ACT_TOOL_NAME
    }

    fn description(&self) -> &str {
        COMPUTER_ACT_DESCRIPTION
    }

    fn schema(&self) -> Value {
        serde_json::from_str(COMPUTER_ACT_SCHEMA).expect("the computer_act schema is valid JSON")
    }

    fn execute(&self, ctx: &CallContext, args: &Value) -> ToolResult {
        let parsed: ActArgs = serde_json::from_value(args.clone())
            .map_err(|e| invalid_err(format!("invalid args: {e}")))?;
        if parsed.steps.is_empty() {
            return Err(bad_step("steps is empty").into());
        }
        if parsed.steps.len() > COMPUTER_MAX_STEPS {
            return Err(
                bad_step(format!("steps has more than {COMPUTER_MAX_STEPS} entries")).into(),
            );
        }
        let Some(host) = computer_host_from(ctx) else {
            return Err(no_computer().into());
        };

        let res = host.act(&parsed.app, parsed.window, parsed.steps.clone())?;
        let mut out = String::new();
        out.push_str(&format!(
            "Completed {} of {} step(s).\n",
            res.done,
            parsed.steps.len()
        ));
        for (i, step) in res.steps.iter().enumerate() {
            out.push_str(&format!(
                "  {}. {}{}\n",
                i + 1,
                step.note,
                render_effect(&step.effect)
            ));
        }
        if res.failed_at >= 0 {
            out.push_str(&format!("Step {} failed.\n", res.failed_at + 1));
        }
        if res.app.bundle.is_empty() {
            return Ok(out);
        }
        if parsed.steps.len() == 1 && parsed.steps[0].action == "launch" {
            out.push_str(
                "Launched application identity (read separately under its own approval):\n",
            );
            out.push_str(&render_apps(std::slice::from_ref(&res.app)));
            return Ok(out);
        }
        match host.snapshot(&res.app.bundle, parsed.window) {
            Ok(snap) => {
                out.push_str("Now:\n");
                out.push_str(&render_computer_snapshot(&snap));
            }
            Err(err) => out.push_str(&format!(
                "The application could not be read afterwards: {err}\n"
            )),
        }
        Ok(out)
    }

    fn read_only(&self) -> bool {
        false
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::default().with(
            "sequential",
            Arc::new(ComputerSequential) as Arc<dyn SequentialTool>,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct ComputerSequential;
impl Capability for ComputerSequential {}
impl SequentialTool for ComputerSequential {
    fn sequential(&self, _ctx: &CallContext, _args: &Value) -> bool {
        true
    }
}

/// The computer face for one composition root.
pub fn computer_tools() -> Vec<Arc<dyn Tool>> {
    vec![computer_read_tool(), computer_act_tool()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_are_valid_json() {
        let _ = ComputerReadTool.schema();
        let _ = ComputerActTool.schema();
    }

    #[test]
    fn render_apps_marks_active_and_unlaunchable() {
        let apps = vec![
            App {
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
            },
            App {
                bundle: "com.other".into(),
                name: "Other".into(),
                launchable: false,
                shown_bundle: "com.other".into(),
                shown_name: "Other".into(),
                ..Default::default()
            },
        ];
        let out = render_apps(&apps);
        assert!(out.starts_with("* com.example.app — Example"));
        assert!(out.contains("(installed; no targetable window; launch unsupported)"));
    }

    #[test]
    fn render_effect_carries_class_and_evidence() {
        let effect = Effect {
            class: "confirmed".into(),
            evidence: vec![Evidence::ValueReadback],
            driver: Some(DriverAction {
                effect: "click".into(),
                route: "ref".into(),
                delivery_count: Some(1),
                ..Default::default()
            }),
            ..Default::default()
        };
        let out = render_effect(&effect);
        assert!(out.contains("took effect"));
        assert!(out.contains("value read back"));
        assert!(out.contains("[CUA effect=click route=ref] delivered=1"));
    }

    #[test]
    fn pointer_steps_detect_pointer_actions() {
        let steps = vec![ComputerStep {
            action: "type".into(),
            ..Default::default()
        }];
        assert!(!pointer_steps(&steps));
        let steps = vec![ComputerStep {
            action: "pointer_click".into(),
            ..Default::default()
        }];
        assert!(pointer_steps(&steps));
    }
}
