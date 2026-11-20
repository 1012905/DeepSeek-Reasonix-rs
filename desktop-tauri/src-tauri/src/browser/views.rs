// The pages the agent's browser opens in this window, each a child webview on
// its own partition, never the profile the window's credential lives in. An
// unwatched page is parked out of sight; an idle one sleeps whole and keeps its
// handle, so the kernel's target outlives the sleep and the next call wakes it.
use crate::boot::MAIN;
use crate::browser::geometry::{
    self, Rect, VIEWPORT_HEIGHT, VIEWPORT_WIDTH, placement, scale_input,
};
use crate::browser::protocol::Views as _;
use crate::browser::{cdp, certs, pick, policy, protocol};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};
use tauri::webview::NewWindowResponse;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Webview, WebviewUrl, WebviewWindow,
};

const SWEEP: Duration = Duration::from_secs(60);
const SLEPT_LIMIT: usize = 18;
const ENABLE_SUFFIX: &str = ".enable";

/// A partition names a storage profile; only a safe directory name may be one.
pub fn valid_partition(partition: &str) -> bool {
    !partition.is_empty()
        && partition
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The kernel enables a domain once at attach; a sleep throws away the session
/// it was said on, so the set is kept per page and said again to every wake.
fn remember_enable(enables: &mut Vec<(String, Value)>, method: &str, params: &Value) {
    if method.ends_with(ENABLE_SUFFIX) || method == "Page.setLifecycleEventsEnabled" {
        if let Some(known) = enables.iter_mut().find(|(m, _)| m == method) {
            known.1 = params.clone();
        } else {
            enables.push((method.to_string(), params.clone()));
        }
    } else if let Some(enable) = method.strip_suffix(".disable") {
        let enable = format!("{enable}{ENABLE_SUFFIX}");
        enables.retain(|(m, _)| *m != enable);
    }
}

/// CDP names a net error "net::ERR_…"; the window reads the bare ERR_ name.
fn net_reason(error_text: &str) -> String {
    error_text
        .strip_prefix("net::")
        .unwrap_or(error_text)
        .to_string()
}

struct Tombstone {
    url: String,
    picture: String,
}

/// What settles a pick: the page's answer, or a reason it ended without one.
enum PickMsg {
    Answer(String),
    End(&'static str),
}

struct Entry {
    target: String,
    partition: String,
    url: String,
    webview: Option<Webview>,
    hooks: protocol::Hooks,
    main_frame: String,
    enables: Vec<(String, Value)>,
    /// The viewport the agent told the page to lay out in; survives a sleep.
    emulated: Option<Value>,
    override_: Option<Value>,
    scale: f64,
    panel: Option<Rect>,
    last_touch: Instant,
    slept: Option<Tombstone>,
    certificate: Option<certs::CertInfo>,
    dialog: bool,
    picking: bool,
    pick_tx: Option<std::sync::mpsc::Sender<PickMsg>>,
    closed: bool,
    typed_turn: u64,
}

impl Entry {
    fn awake(&self) -> bool {
        self.slept.is_none() && !self.closed
    }
}

struct Inner {
    entries: HashMap<String, Entry>,
    shown: String,
    placed: u64,
    sleep_after: Option<Duration>,
    max_awake: usize,
}

pub struct BrowserViews {
    inner: Mutex<Inner>,
    app: AppHandle,
    kernel_origin: String,
    data_root: PathBuf,
    me: OnceLock<Weak<BrowserViews>>,
}

impl BrowserViews {
    pub fn new(
        app: AppHandle,
        kernel_origin: String,
        data_root: PathBuf,
        sleep_after: Option<Duration>,
        max_awake: usize,
    ) -> Arc<Self> {
        let views = Arc::new(Self {
            inner: Mutex::new(Inner {
                entries: HashMap::new(),
                shown: String::new(),
                placed: 0,
                sleep_after,
                max_awake,
            }),
            app,
            kernel_origin,
            data_root,
            me: OnceLock::new(),
        });
        let _ = views.me.set(Arc::downgrade(&views));
        let sweeper = views.clone();
        std::thread::spawn(move || sweeper.sweep_loop());
        views
    }

    fn arc(&self) -> Option<Arc<BrowserViews>> {
        self.me.get().and_then(Weak::upgrade)
    }

    fn window(&self) -> Option<tauri::Window> {
        self.app.get_window(MAIN)
    }

    fn webview_window(&self) -> Option<WebviewWindow> {
        self.app.get_webview_window(MAIN)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn webview_of(&self, target: &str) -> Option<Webview> {
        self.lock()
            .entries
            .get(target)
            .and_then(|e| e.webview.clone())
    }

    fn touch(&self, target: &str) {
        if let Some(entry) = self.lock().entries.get_mut(target) {
            entry.last_touch = Instant::now();
        }
    }

    // ---- lifecycle -------------------------------------------------------

    /// Builds the child webview a page lives in and wires its events. Runs off
    /// the main thread and holds no lock across the build, which waits on it.
    fn spawn(
        &self,
        target: &str,
        partition: &str,
        url: &str,
        hooks: protocol::Hooks,
    ) -> Result<Webview, String> {
        let window = self.window().ok_or("the window is gone")?;
        let parsed: url::Url = url.parse().map_err(|_| format!("cannot load {url}"))?;
        let origin = self.kernel_origin.clone();
        let popup_origin = self.kernel_origin.clone();
        let popup_hooks = hooks.clone();
        let data_dir = self.data_root.join(partition);
        let builder = tauri::webview::WebviewBuilder::new(
            format!("browser-{target}"),
            WebviewUrl::External(parsed),
        )
        .data_directory(data_dir)
        .incognito(false)
        .on_navigation(move |to| policy::guest_navigation_allowed(to.as_str(), &origin))
        .on_new_window(move |to, _| {
            if policy::guest_navigation_allowed(to.as_str(), &popup_origin) {
                popup_hooks.popup(to.as_str());
            }
            NewWindowResponse::Deny
        });
        let parked = LogicalPosition::new(1.0 - VIEWPORT_WIDTH, 0.0);
        let size = LogicalSize::new(VIEWPORT_WIDTH, VIEWPORT_HEIGHT);
        let webview = window
            .add_child(builder, parked, size)
            .map_err(|e| format!("building the page: {e}"))?;
        let _ = webview.hide();
        self.wire(&webview, target, &hooks);
        Ok(webview)
    }

    /// Subscribes the page's events. Each handler updates state briefly and
    /// never waits on the main thread, so the lock is never held across a call
    /// that the main thread must answer.
    fn wire(&self, webview: &Webview, target: &str, hooks: &protocol::Hooks) {
        let Some(views) = self.arc() else {
            return;
        };
        let target = target.to_string();
        let hooks = hooks.clone();
        let nav_views = views.clone();
        let nav_target = target.clone();
        let nav_hooks = hooks.clone();
        let _ = cdp::subscribe(webview, "Page.frameNavigated", move |params| {
            let frame = &params["frame"];
            if frame.get("parentId").is_some() {
                return;
            }
            if let Some(id) = frame.get("id").and_then(Value::as_str)
                && let Some(entry) = nav_views.lock().entries.get_mut(&nav_target)
            {
                entry.main_frame = id.to_string();
                entry.typed_turn = 0;
            }
            nav_hooks.event("Page.frameNavigated", params);
            nav_views.report(&nav_target, None);
        });
        let fail_views = views.clone();
        let fail_target = target.clone();
        let fail_hooks = hooks.clone();
        let _ = cdp::subscribe(webview, "Page.loadingFailed", move |params| {
            let reason = net_reason(
                params
                    .get("errorText")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            );
            if reason == "ERR_ABORTED" {
                return;
            }
            fail_hooks.event("Page.loadingFailed", params.clone());
            // CDP names no numeric code and no url here; the window reads the
            // bare ERR_ name and the address from the page's own state.
            fail_views.report(
                &fail_target,
                Some(json!({ "url": "", "code": 0, "reason": reason })),
            );
        });
        let dialog_views = views.clone();
        let dialog_target = target.clone();
        let dialog_hooks = hooks.clone();
        let _ = cdp::subscribe(webview, "Page.javascriptDialogOpening", move |params| {
            if let Some(entry) = dialog_views.lock().entries.get_mut(&dialog_target) {
                entry.dialog = true;
            }
            dialog_hooks.event("Page.javascriptDialogOpening", params);
        });
        let closed_views = views.clone();
        let closed_target = target.clone();
        let closed_hooks = hooks.clone();
        let _ = cdp::subscribe(webview, "Page.javascriptDialogClosed", move |params| {
            if let Some(entry) = closed_views.lock().entries.get_mut(&closed_target) {
                entry.dialog = false;
            }
            closed_hooks.event("Page.javascriptDialogClosed", params);
        });
        let cert_views = views.clone();
        let cert_target = target.clone();
        let _ = certs::watch_certificates(webview, move |info| {
            if let Some(entry) = cert_views.lock().entries.get_mut(&cert_target) {
                entry.certificate = Some(info);
            }
        });
        let login_views = views.clone();
        let login_target = target.clone();
        let _ = cdp::watch_logins(webview, &target, move |ask| {
            login_views.emit_login(&login_target, ask);
        });
        let dl_views = views.clone();
        let dl_target = target.clone();
        let dl_hooks = hooks.clone();
        let _ = cdp::subscribe(webview, "Browser.downloadWillBegin", move |params| {
            let url = params.get("url").and_then(Value::as_str).unwrap_or("");
            let name = params
                .get("suggestedFilename")
                .and_then(Value::as_str)
                .unwrap_or("");
            dl_hooks.download(url, name, &dl_views.main_frame(&dl_target));
        });
        // The kernel enables Page at attach; the shell switches it on now so
        // the dialog and navigation events above reach it from the first load.
        let _ = cdp::call(webview, "Page.enable", &json!({}));
        let _ = cdp::call(webview, "Network.enable", &json!({}));
        let blocked = json!({ "urls": [format!("{}/*", self.kernel_origin)] });
        let _ = cdp::call(webview, "Network.setBlockedURLs", &blocked);
        let _ = cdp::call(
            webview,
            "Browser.setDownloadBehavior",
            &json!({ "behavior": "deny", "eventsEnabled": true }),
        );
        for permission in [
            "geolocation",
            "notifications",
            "camera",
            "microphone",
            "clipboard-read",
            "clipboard-write",
        ] {
            let _ = cdp::call(
                webview,
                "Browser.setPermission",
                &json!({ "permission": { "name": permission }, "setting": "denied" }),
            );
        }
    }

    fn report(&self, target: &str, failure: Option<Value>) {
        let failure = {
            let inner = self.lock();
            let Some(entry) = inner.entries.get(target) else {
                return;
            };
            match (failure, &entry.certificate) {
                (Some(mut f), cert) => {
                    let reason = f
                        .get("reason")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let url = f
                        .get("url")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .or_else(|| cert.as_ref().map(|c| c.url.clone()))
                        .unwrap_or_else(|| entry.url.clone());
                    f["url"] = json!(url);
                    if let Some(cert) = cert
                        && reason.starts_with("ERR_CERT_")
                        && host_of(&url) == cert.host
                    {
                        f["certificate"] = json!({
                            "host": cert.host, "error": cert.error, "fingerprint": cert.fingerprint,
                            "subject": cert.subject, "issuer": cert.issuer, "expiry": cert.expiry,
                        });
                    }
                    Some(f)
                }
                (None, _) => None,
            }
        };
        let _ = self.app.emit_to(
            MAIN,
            "browser://load-state",
            json!({ "targetId": target, "failure": failure }),
        );
    }

    fn emit_login(&self, target: &str, ask: cdp::LoginAsk) {
        let _ = self.app.emit_to(
            MAIN,
            "browser://login",
            json!({
                "id": ask.id, "targetId": target, "url": ask.url, "host": ask.host,
                "port": ask.port, "realm": ask.realm, "proxy": false,
            }),
        );
    }

    // ---- sleep / wake ----------------------------------------------------

    fn may_sleep(&self, entry: &Entry, shown: &str) -> bool {
        entry.target != shown && !entry.picking && entry.certificate.is_none() && !entry.dialog
    }

    fn sweep_loop(self: Arc<Self>) {
        loop {
            std::thread::sleep(SWEEP);
            let sleep_after = self.lock().sleep_after;
            let Some(sleep_after) = sleep_after else {
                continue;
            };
            let candidates: Vec<String> = {
                let inner = self.lock();
                inner
                    .entries
                    .values()
                    .filter(|e| {
                        e.awake()
                            && e.last_touch.elapsed() > sleep_after
                            && self.may_sleep(e, &inner.shown)
                    })
                    .map(|e| e.target.clone())
                    .collect()
            };
            for target in candidates {
                self.sleep_target(&target);
            }
        }
    }

    /// Puts a page away whole: a picture and its address are kept, the renderer
    /// is closed, and the handle survives for the next call to wake.
    fn sleep_target(&self, target: &str) {
        let (webview, url) = {
            let inner = self.lock();
            let Some(entry) = inner.entries.get(target) else {
                return;
            };
            if !entry.awake() || !self.may_sleep(entry, &inner.shown) {
                return;
            }
            (entry.webview.clone(), entry.url.clone())
        };
        let Some(webview) = webview else {
            return;
        };
        let picture = screenshot(&webview).unwrap_or_default();
        let url = current_url(&webview).unwrap_or(url);
        let oldest = {
            let mut inner = self.lock();
            let shown = inner.shown.clone();
            let Some(entry) = inner.entries.get_mut(target) else {
                return;
            };
            if !entry.awake() || !self.may_sleep(entry, &shown) {
                return;
            }
            entry.slept = Some(Tombstone { url, picture });
            entry.webview = None;
            entry.panel = None;
            entry.override_ = None;
            entry.scale = 1.0;
            entry.main_frame = String::new();
            let mut tombs = 0;
            let mut oldest: Option<(String, Instant)> = None;
            for other in inner.entries.values() {
                if other.slept.is_some() {
                    tombs += 1;
                    if oldest.as_ref().is_none_or(|(_, t)| other.last_touch < *t) {
                        oldest = Some((other.target.clone(), other.last_touch));
                    }
                }
            }
            if tombs > SLEPT_LIMIT {
                oldest.map(|(t, _)| t)
            } else {
                None
            }
        };
        let _ = webview.close();
        if let Some(oldest) = oldest {
            self.close(&oldest);
        }
    }

    /// Raises a slept page on the address it dozed off on, saying its viewport
    /// and enabled domains again to the new session.
    fn wake(&self, target: &str) {
        let (partition, url, enables, emulated, hooks) = {
            let mut inner = self.lock();
            let Some(entry) = inner.entries.get_mut(target) else {
                return;
            };
            let Some(tomb) = entry.slept.take() else {
                return;
            };
            entry.url = tomb.url.clone();
            (
                entry.partition.clone(),
                tomb.url,
                entry.enables.clone(),
                entry.emulated.clone(),
                entry.hooks.clone(),
            )
        };
        let Ok(webview) = self.spawn(target, &partition, &url, hooks) else {
            return;
        };
        if let Some(emulated) = &emulated {
            let _ = cdp::call(&webview, "Emulation.setDeviceMetricsOverride", emulated);
        }
        for (method, params) in enables {
            let _ = cdp::call(&webview, &method, &params);
        }
        let mut inner = self.lock();
        if let Some(entry) = inner.entries.get_mut(target) {
            entry.webview = Some(webview);
            entry.override_ = emulated;
            entry.last_touch = Instant::now();
        }
    }

    // ---- placement -------------------------------------------------------

    fn put_away(&self, webview: &Webview) {
        let _ = webview.set_position(LogicalPosition::new(1.0 - VIEWPORT_WIDTH, 0.0));
        let _ = webview.set_size(LogicalSize::new(VIEWPORT_WIDTH, VIEWPORT_HEIGHT));
        let _ = webview.hide();
    }

    fn fit(&self, target: &str) {
        let (webview, panel, override_) = {
            let inner = self.lock();
            let Some(entry) = inner.entries.get(target) else {
                return;
            };
            (entry.webview.clone(), entry.panel, entry.override_.clone())
        };
        let (Some(webview), Some(panel)) = (webview, panel) else {
            return;
        };
        let (at, scale) = placement(panel, override_.as_ref());
        let _ = webview.set_position(LogicalPosition::new(at.x, at.y));
        let _ = webview.set_size(LogicalSize::new(at.width, at.height));
        self.set_scale(target, &webview, scale, false);
    }

    fn set_scale(&self, target: &str, webview: &Webview, scale: f64, force: bool) {
        let override_ = {
            let mut inner = self.lock();
            let Some(entry) = inner.entries.get_mut(target) else {
                return;
            };
            if entry.override_.is_none() || (entry.scale == scale && !force) {
                return;
            }
            entry.scale = scale;
            entry.override_.clone()
        };
        if let Some(mut override_) = override_ {
            override_["scale"] = json!(scale);
            let _ = cdp::call(webview, "Emulation.setDeviceMetricsOverride", &override_);
        }
    }

    /// Draws one page over the rectangle the panel reserved, in window
    /// coordinates, and puts every other page away.
    pub fn show(&self, target: &str, rect: Rect) {
        if !geometry::valid_rect(&rect) {
            return self.hide();
        }
        self.touch(target);
        let slept = {
            let inner = self.lock();
            match inner.entries.get(target) {
                Some(entry) => entry.slept.is_some(),
                None => return self.hide(),
            }
        };
        if slept {
            self.wake(target);
        }
        let at = geometry::rounded(&rect);
        let webview = {
            let mut inner = self.lock();
            if inner.shown == target
                && geometry::same_rect(inner.entries.get(target).and_then(|e| e.panel), at)
            {
                return;
            }
            inner.placed += 1;
            inner.shown = target.to_string();
            let others: Vec<Webview> = inner
                .entries
                .values()
                .filter(|e| e.target != target)
                .filter_map(|e| e.webview.clone())
                .collect();
            for other in others {
                self.put_away(&other);
            }
            if let Some(entry) = inner.entries.get_mut(target) {
                entry.panel = Some(at);
            }
            inner.entries.get(target).and_then(|e| e.webview.clone())
        };
        if let Some(webview) = webview {
            let _ = webview.show();
            let _ = webview.set_focus();
        }
        self.fit(target);
    }

    pub fn hide(&self) {
        let (webviews, picking) = {
            let mut inner = self.lock();
            inner.placed += 1;
            inner.shown = String::new();
            let webviews: Vec<Webview> = inner
                .entries
                .values()
                .filter_map(|e| e.webview.clone())
                .collect();
            let picking: Vec<String> = inner
                .entries
                .values()
                .filter(|e| e.picking)
                .map(|e| e.target.clone())
                .collect();
            for entry in inner.entries.values_mut() {
                entry.panel = None;
            }
            (webviews, picking)
        };
        for webview in webviews {
            self.put_away(&webview);
        }
        for target in picking {
            self.cancel_pick(&target, "hidden");
        }
    }
}

fn screenshot(webview: &Webview) -> Option<String> {
    let answer = cdp::call(
        webview,
        "Page.captureScreenshot",
        &json!({ "format": "jpeg", "quality": 85 }),
    )
    .ok()?;
    let data = answer.get("data").and_then(Value::as_str)?;
    if data.is_empty() {
        return None;
    }
    Some(format!("data:image/jpeg;base64,{data}"))
}

fn current_url(webview: &Webview) -> Option<String> {
    let answer = cdp::call(webview, "Page.getNavigationHistory", &json!({})).ok()?;
    let index = answer.get("currentIndex").and_then(Value::as_i64)? as usize;
    let entries = answer.get("entries").and_then(Value::as_array)?;
    entries
        .get(index)?
        .get("url")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn host_of(raw: &str) -> String {
    let Ok(url) = url::Url::parse(raw) else {
        return String::new();
    };
    let Some(host) = url.host_str() else {
        return String::new();
    };
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    }
}

impl protocol::Views for BrowserViews {
    fn create(
        &self,
        target: &str,
        partition: &str,
        url: &str,
        hooks: protocol::Hooks,
    ) -> Result<(), String> {
        if !valid_partition(partition) {
            return Err(format!("invalid partition {partition}"));
        }
        if !policy::guest_navigation_allowed(url, &self.kernel_origin) {
            return Err(format!("cannot open {url}"));
        }
        let victim = {
            let mut inner = self.lock();
            let mut awake = 0;
            let mut victim: Option<(String, Instant)> = None;
            for entry in inner.entries.values() {
                if !entry.awake() {
                    continue;
                }
                awake += 1;
                if entry.target != inner.shown
                    && victim.as_ref().is_none_or(|(_, t)| entry.last_touch < *t)
                {
                    victim = Some((entry.target.clone(), entry.last_touch));
                }
            }
            let cap = inner.sleep_after.is_some() && awake >= inner.max_awake;
            inner.entries.insert(
                target.to_string(),
                Entry {
                    target: target.to_string(),
                    partition: partition.to_string(),
                    url: url.to_string(),
                    webview: None,
                    hooks: hooks.clone(),
                    main_frame: String::new(),
                    enables: Vec::new(),
                    emulated: None,
                    override_: None,
                    scale: 1.0,
                    panel: None,
                    last_touch: Instant::now(),
                    slept: None,
                    certificate: None,
                    dialog: false,
                    picking: false,
                    pick_tx: None,
                    closed: false,
                    typed_turn: 0,
                },
            );
            if cap { victim.map(|(t, _)| t) } else { None }
        };
        if let Some(victim) = victim {
            self.sleep_target(&victim);
        }
        match self.spawn(target, partition, url, hooks) {
            Ok(webview) => {
                self.lock().entries.get_mut(target).unwrap().webview = Some(webview);
                Ok(())
            }
            Err(err) => {
                self.lock().entries.remove(target);
                Err(err)
            }
        }
    }

    fn send(&self, target: &str, method: &str, params: Value) -> Result<Value, String> {
        self.touch(target);
        let slept = self
            .lock()
            .entries
            .get(target)
            .is_some_and(|e| e.slept.is_some());
        if slept {
            self.wake(target);
        }
        let (webview, scale) = {
            let mut inner = self.lock();
            let entry = inner.entries.get_mut(target).ok_or("no such page")?;
            remember_enable(&mut entry.enables, method, &params);
            (
                entry.webview.clone().ok_or("the page is gone")?,
                entry.scale,
            )
        };
        if method == "Emulation.setDeviceMetricsOverride"
            || method == "Emulation.clearDeviceMetricsOverride"
        {
            return self.emulate(target, &webview, method, &params);
        }
        let params = scale_input(method, params, scale)?;
        let answer = cdp::call(&webview, method, &params);
        if method == "Page.captureScreenshot" {
            self.fit(target);
        }
        answer
    }

    fn close(&self, target: &str) {
        let (webview, hooks) = {
            let mut inner = self.lock();
            let Some(entry) = inner.entries.remove(target) else {
                return;
            };
            if inner.shown == target {
                inner.shown = String::new();
            }
            (entry.webview, entry.hooks)
        };
        if let Some(webview) = webview {
            let _ = webview.close();
        }
        hooks.closed();
    }

    fn main_frame(&self, target: &str) -> String {
        self.lock()
            .entries
            .get(target)
            .filter(|e| e.slept.is_none())
            .map(|e| e.main_frame.clone())
            .unwrap_or_default()
    }

    fn activate(&self, target: &str) {
        if let Some(webview) = self.webview_of(target) {
            let _ = webview.set_focus();
        }
    }
}

impl BrowserViews {
    fn emulate(
        &self,
        target: &str,
        webview: &Webview,
        method: &str,
        params: &Value,
    ) -> Result<Value, String> {
        let clearing =
            method == "Emulation.clearDeviceMetricsOverride" || !geometry::is_sized(Some(params));
        let panel = {
            let mut inner = self.lock();
            let entry = inner.entries.get_mut(target).ok_or("no such page")?;
            if clearing {
                entry.override_ = None;
                entry.emulated = None;
                entry.scale = 1.0;
            } else {
                entry.override_ = Some(params.clone());
                entry.emulated = Some(params.clone());
                entry.scale = 1.0;
            }
            entry.panel
        };
        if clearing {
            if panel.is_some() {
                self.fit(target);
            }
            return cdp::call(webview, "Emulation.clearDeviceMetricsOverride", &json!({}));
        }
        if let Some(panel) = panel {
            let (at, scale) = placement(panel, Some(params));
            let _ = webview.set_position(LogicalPosition::new(at.x, at.y));
            let _ = webview.set_size(LogicalSize::new(at.width, at.height));
            self.lock().entries.get_mut(target).unwrap().scale = scale;
        }
        let mut override_ = params.clone();
        let scale = self
            .lock()
            .entries
            .get(target)
            .map(|e| e.scale)
            .unwrap_or(1.0);
        override_["scale"] = json!(scale);
        cdp::call(webview, "Emulation.setDeviceMetricsOverride", &override_)
    }
}

impl BrowserViews {
    /// Starts loading what the person typed and answers "", or the code of why
    /// the window will not: "scheme" or "network_file".
    pub fn navigate(&self, target: &str, address: &str) -> String {
        let typed = policy::typed(address, &self.kernel_origin);
        let slept = {
            let inner = self.lock();
            match inner.entries.get(target) {
                None => return "scheme".to_string(),
                Some(entry) => entry.slept.is_some(),
            }
        };
        if !typed.refusal.is_empty() {
            return typed.refusal.to_string();
        }
        if slept {
            self.wake(target);
        }
        self.load_typed(target, &typed.url, &typed.fallback);
        String::new()
    }

    fn load_typed(&self, target: &str, url: &str, fallback: &str) {
        let webview = {
            let mut inner = self.lock();
            let Some(entry) = inner.entries.get_mut(target) else {
                return;
            };
            entry.typed_turn += 1;
            entry.webview.clone()
        };
        let Some(webview) = webview else {
            return;
        };
        if let Ok(parsed) = url.parse::<url::Url>()
            && webview.navigate(parsed).is_err()
            && !fallback.is_empty()
            && let Ok(fb) = fallback.parse::<url::Url>()
        {
            let _ = webview.navigate(fb);
        }
    }

    /// Steers a page: back, forward, reload, stop. A slept page is woken first
    /// so the action lands on the page it comes back as.
    pub fn control(&self, target: &str, action: &str) {
        let slept = self
            .lock()
            .entries
            .get(target)
            .is_some_and(|e| e.slept.is_some());
        if slept {
            self.wake(target);
        }
        let Some(webview) = self.webview_of(target) else {
            return;
        };
        match action {
            "reload" => {
                let _ = webview.eval("location.reload()");
            }
            "stop" => {
                let _ = webview.eval("window.stop()");
            }
            "back" | "forward" => {
                if let Ok(history) = cdp::call(&webview, "Page.getNavigationHistory", &json!({})) {
                    let index = history["currentIndex"].as_i64().unwrap_or(0);
                    let entries = history["entries"].as_array();
                    let count = entries.map(|a| a.len() as i64).unwrap_or(0);
                    let want = if action == "back" {
                        index - 1
                    } else {
                        index + 1
                    };
                    if want >= 0
                        && want < count
                        && let Some(id) = entries
                            .and_then(|a| a.get(want as usize))
                            .and_then(|e| e.get("id"))
                            .cloned()
                    {
                        let _ = cdp::call(
                            &webview,
                            "Page.navigateToHistoryEntry",
                            &json!({ "entryId": id }),
                        );
                    }
                    return;
                }
                let _ = webview.eval(if action == "back" {
                    "history.back()"
                } else {
                    "history.forward()"
                });
            }
            _ => {}
        }
    }

    /// A picture of the page on screen, taken before it is put away. Only the
    /// page already shown is taken; a slept page answers with its own picture.
    pub fn freeze(&self) -> String {
        enum Shot {
            Slept(String),
            Live(String, Box<Webview>, u64),
            None,
        }
        let shot = {
            let inner = self.lock();
            let id = inner.shown.clone();
            let turn = inner.placed;
            match inner.entries.get(&id) {
                Some(entry) if entry.slept.is_some() => Shot::Slept(
                    entry
                        .slept
                        .as_ref()
                        .map(|t| t.picture.clone())
                        .unwrap_or_default(),
                ),
                Some(entry) => match entry.webview.clone() {
                    Some(webview) => Shot::Live(id, Box::new(webview), turn),
                    None => Shot::None,
                },
                None => Shot::None,
            }
        };
        match shot {
            Shot::Slept(picture) => picture,
            Shot::None => {
                self.hide();
                String::new()
            }
            Shot::Live(id, webview, turn) => {
                let picture = screenshot(&webview).unwrap_or_default();
                let unchanged = {
                    let inner = self.lock();
                    inner.placed == turn && inner.shown == id
                };
                if unchanged {
                    self.hide();
                }
                picture
            }
        }
    }

    /// Arms one page's element picker and answers what the person pointed at,
    /// or why none was. A second pick dismisses the first; a pick behind a page
    /// nobody sees is refused, and one left too long times out.
    pub fn pick(&self, target: &str, labels: Value) -> Value {
        let gone = json!({ "cancelled": true, "reason": "gone" });
        let shown = {
            let inner = self.lock();
            match inner.entries.get(target) {
                None => return gone,
                Some(_) if inner.shown != target => {
                    return json!({ "cancelled": true, "reason": "hidden" });
                }
                Some(entry) => entry.slept.is_some(),
            }
        };
        if shown {
            self.wake(target);
        }
        let Some(webview) = self.webview_of(target) else {
            return gone;
        };
        self.cancel_pick(target, "dismissed");
        let (tx, rx) = std::sync::mpsc::channel::<PickMsg>();
        {
            let mut inner = self.lock();
            if let Some(entry) = inner.entries.get_mut(target) {
                entry.picking = true;
                entry.pick_tx = Some(tx.clone());
            }
        }
        let armed = webview.eval_with_callback(pick::arm(&labels), move |answer| {
            let _ = tx.send(PickMsg::Answer(answer));
        });
        if armed.is_err() {
            self.end_pick(target);
            return json!({ "cancelled": true, "reason": "navigation" });
        }
        let answer = match rx.recv_timeout(pick::PICK_LIMIT) {
            Ok(PickMsg::Answer(answer)) => answer,
            Ok(PickMsg::End(reason)) => {
                self.end_pick(target);
                return json!({ "cancelled": true, "reason": reason });
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let _ = webview.eval(pick::STOP);
                self.end_pick(target);
                return json!({ "cancelled": true, "reason": "timeout" });
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                self.end_pick(target);
                return json!({ "cancelled": true, "reason": "navigation" });
            }
        };
        self.end_pick(target);
        let cleaned = pick::clean(&serde_json::from_str::<Value>(&answer).unwrap_or(Value::Null));
        if cleaned["cancelled"].as_bool().unwrap_or(false) {
            return cleaned;
        }
        let selector = cleaned["element"]["selector"].as_str().unwrap_or("");
        let tag = cleaned["element"]["tagName"].as_str().unwrap_or("");
        if !self.lands(&webview, selector, tag) {
            return gone;
        }
        let mut cleaned = cleaned;
        if let Some(shot) = self.shot(target, &webview, &cleaned["element"]["rect"].clone()) {
            cleaned["shot"] = json!(shot);
        }
        cleaned
    }

    fn end_pick(&self, target: &str) {
        if let Some(entry) = self.lock().entries.get_mut(target) {
            entry.picking = false;
            entry.pick_tx = None;
        }
    }

    /// Disarms the picker, settling a pick still waiting on the page.
    pub fn cancel_pick(&self, target: &str, reason: &'static str) {
        let (tx, webview) = {
            let mut inner = self.lock();
            let Some(entry) = inner.entries.get_mut(target) else {
                return;
            };
            entry.picking = false;
            (entry.pick_tx.take(), entry.webview.clone())
        };
        if let Some(tx) = tx {
            let _ = tx.send(PickMsg::End(reason));
        }
        if let Some(webview) = webview {
            let _ = webview.eval(pick::STOP);
        }
    }

    /// Whether a selector still names an element of the tag the pick reported.
    /// The page cannot invent an element: the shell re-finds it before trusting
    /// the answer. Without a native debugger this runs in the page's own world.
    fn lands(&self, webview: &Webview, selector: &str, tag: &str) -> bool {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        if webview
            .eval_with_callback(pick::lands_script(selector), move |answer| {
                let _ = tx.send(answer);
            })
            .is_err()
        {
            return false;
        }
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(answer) => pick::lands_match(&answer, tag),
            Err(_) => false,
        }
    }

    /// Captures the element as the view drew it, scaled and kept inside the
    /// view; an element scrolled out of sight has no picture.
    fn shot(&self, target: &str, webview: &Webview, rect: &Value) -> Option<String> {
        if !rect.is_object() {
            return None;
        }
        let scale = self
            .lock()
            .entries
            .get(target)
            .map(|e| e.scale)
            .unwrap_or(1.0);
        let size = webview.size().ok()?;
        let x = (rect["x"].as_f64().unwrap_or(0.0) * scale).round().max(0.0);
        let y = (rect["y"].as_f64().unwrap_or(0.0) * scale).round().max(0.0);
        let mut width = (rect["width"].as_f64().unwrap_or(0.0) * scale).round();
        let mut height = (rect["height"].as_f64().unwrap_or(0.0) * scale).round();
        width = width.min(size.width as f64 - x);
        height = height.min(size.height as f64 - y);
        if width < 1.0 || height < 1.0 {
            return None;
        }
        let clip = json!({ "x": x, "y": y, "width": width, "height": height, "scale": 1 });
        let answer = cdp::call(
            webview,
            "Page.captureScreenshot",
            &json!({ "format": "png", "clip": clip }),
        )
        .ok()?;
        let data = answer.get("data").and_then(Value::as_str)?;
        if data.is_empty() {
            return None;
        }
        Some(format!("data:image/png;base64,{data}"))
    }

    /// Proceeds past the certificate a page's last load was refused for. Only
    /// the window's own page reaches this, so neither the kernel nor the agent
    /// driving the browser can trust one.
    pub fn trust_certificate(&self, target: &str) -> bool {
        let (cert, slept) = {
            let mut inner = self.lock();
            let Some(entry) = inner.entries.get_mut(target) else {
                return false;
            };
            (entry.certificate.take(), entry.slept.is_some())
        };
        let Some(cert) = cert else {
            return false;
        };
        if let Some(window) = self.webview_window() {
            certs::trust(&window, &cert.host, &cert.fingerprint);
        }
        if slept {
            self.wake(target);
        }
        self.load_typed(target, &cert.url, "");
        true
    }

    pub fn answer_login(&self, id: &str, username: &str, password: &str) {
        if let Some(window) = self.webview_window() {
            cdp::answer_login(&window, id, username, password);
        }
    }

    pub fn close_all(&self) {
        if let Some(window) = self.webview_window() {
            cdp::cancel_all_logins(&window);
        }
        let targets: Vec<String> = self.lock().entries.keys().cloned().collect();
        for target in targets {
            self.close(&target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partition_is_a_safe_directory_name() {
        assert!(valid_partition("agent-1"));
        assert!(valid_partition("A_b-2"));
        assert!(!valid_partition(""));
        assert!(!valid_partition("../escape"));
        assert!(!valid_partition("a/b"));
        assert!(!valid_partition("a b"));
    }

    #[test]
    fn enables_are_remembered_latest_first_seen_order_and_disabled_removed() {
        let mut enables: Vec<(String, Value)> = Vec::new();
        remember_enable(&mut enables, "Page.enable", &json!({}));
        remember_enable(&mut enables, "Network.enable", &json!({ "a": 1 }));
        remember_enable(&mut enables, "Page.enable", &json!({ "b": 2 }));
        assert_eq!(enables.len(), 2);
        assert_eq!(enables[0].0, "Page.enable");
        assert_eq!(enables[0].1["b"], 2);
        remember_enable(
            &mut enables,
            "Page.setLifecycleEventsEnabled",
            &json!({ "enabled": true }),
        );
        assert_eq!(enables.len(), 3);
        remember_enable(&mut enables, "Network.disable", &json!({}));
        assert!(enables.iter().all(|(m, _)| m != "Network.enable"));
        remember_enable(&mut enables, "Runtime.evaluate", &json!({}));
        assert_eq!(enables.len(), 2);
    }

    #[test]
    fn a_net_error_keeps_its_bare_name() {
        assert_eq!(
            net_reason("net::ERR_CERT_AUTHORITY_INVALID"),
            "ERR_CERT_AUTHORITY_INVALID"
        );
        assert_eq!(net_reason("ERR_ABORTED"), "ERR_ABORTED");
        assert_eq!(net_reason(""), "");
    }

    #[test]
    fn the_host_carries_the_port_the_url_spells() {
        assert_eq!(host_of("https://example.com:8443/a"), "example.com:8443");
        assert_eq!(host_of("https://example.com/a"), "example.com");
        assert_eq!(host_of("nonsense"), "");
    }
}
