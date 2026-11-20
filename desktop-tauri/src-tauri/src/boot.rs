// The launch: spawn the kernel, read its handshake, grant its origin the
// shell's verbs, and only then open the window on it. The credential cookie is
// set before the first navigation — a page that loads without it opens on a
// refusal — and the window may only ever be that origin.
use crate::client::{KernelClient, TOKEN_COOKIE};
use crate::logs::Logs;
use crate::{bridge, commands, graphics, host, layout, prefs, theme};
use std::process::{Child, ChildStdin};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use tauri::ipc::CapabilityBuilder;
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

pub const MAIN: &str = "main";
const DEFAULT_SIZE: LogicalSize<f64> = LogicalSize {
    width: 1440.0,
    height: 900.0,
};
const MIN_SIZE: LogicalSize<f64> = LogicalSize {
    width: 760.0,
    height: 480.0,
};
const DRAIN: Duration = Duration::from_secs(3);
/// How long the exit watcher waits for the quit act to land before reading
/// the host's death as an early one.
const QUIT_BEAT: Duration = Duration::from_secs(1);
const DETAIL_LIMIT: usize = 1800;

/// The lease on the kernel: its stdin, held open for as long as the shell runs.
#[derive(Default)]
pub struct Lease {
    stdin: Mutex<Option<ChildStdin>>,
    ended: Arc<(Mutex<bool>, Condvar)>,
    quitting: AtomicBool,
    relaunch: AtomicBool,
}

impl Lease {
    /// Closing this end is what tells the kernel to drain; without it a session
    /// file is left being written by a process nobody holds open.
    pub fn release(&self) {
        self.quitting.store(true, Ordering::SeqCst);
        drop(self.stdin.lock().ok().and_then(|mut stdin| stdin.take()));
        let (lock, ended) = &*self.ended;
        if let Ok(guard) = lock.lock() {
            let _ = ended.wait_timeout_while(guard, DRAIN, |done| !*done);
        }
    }
}

pub fn start(app: AppHandle) {
    std::thread::spawn(move || run(&app));
}

/// Brings the window back from wherever it went; show alone does nothing for
/// a window that is merely buried.
pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn run(app: &AppHandle) {
    let layout = match layout::resolve(app) {
        Ok(layout) => layout,
        Err(err) => return fail(app, None, &format!("locating the shell's files: {err}")),
    };
    let logs = Arc::new(Logs::open(layout.logs.clone()));
    app.manage(logs.clone());
    logs.line(&format!(
        "shell: start version={} dev={} platform={} pid={}",
        app.package_info().version,
        tauri::is_dev(),
        bridge::platform(),
        std::process::id()
    ));
    if !layout.host.exists() {
        let hint = if tauri::is_dev() {
            " — build it with `pnpm host`"
        } else {
            ""
        };
        return fail(
            app,
            Some(&logs),
            &format!(
                "the kernel host is missing at {}{hint}",
                layout.host.display()
            ),
        );
    }
    let running = match launch(app, &layout, &logs) {
        Ok(running) => running,
        Err(err) => return fail(app, Some(&logs), &err.to_string()),
    };
    let host::Running {
        child,
        stdin,
        stdout,
        handshake,
    } = running;
    logs.line(&format!("host: handshake from {}", handshake.origin));
    let lease = app.state::<Lease>();
    if let Ok(mut held) = lease.stdin.lock() {
        *held = Some(stdin);
    }
    watch(app.clone(), child, lease.ended.clone(), logs.clone());
    let acts = app.clone();
    host::read_acts(stdout, move |act| perform(&acts, &act));
    app.manage(commands::Shell {
        client: KernelClient::new(handshake.origin.clone(), handshake.token.clone()),
        logs_dir: layout.logs.clone(),
        prefs_file: prefs::file(&layout.config),
        painted: Mutex::new(None),
    });
    if let Err(err) = open_window(app, &handshake, &layout) {
        fail(app, Some(&logs), &format!("opening the window: {err}"));
    }
}

fn launch(
    app: &AppHandle,
    layout: &layout::Layout,
    logs: &Arc<Logs>,
) -> Result<host::Running, host::StartError> {
    let path = |p: &std::path::Path| p.to_string_lossy().into_owned();
    let mut args = vec![
        "-page".into(),
        path(&layout.page),
        "-crash-dir".into(),
        path(&layout.logs.join("crash")),
    ];
    // Which build this is belongs to the shell: inside the bundle the kernel's
    // own executable names the host binary, not the application around it.
    if !tauri::is_dev()
        && let Ok(exe) = std::env::current_exe()
    {
        args.extend([
            "-studio-version".into(),
            app.package_info().version.to_string(),
            "-studio-app".into(),
            path(&exe),
            "-studio-app-pid".into(),
            std::process::id().to_string(),
        ]);
    }
    let sink = logs.clone();
    let launch = host::Launch {
        binary: &layout.host,
        args,
        system_language: sys_locale::get_locale(),
        timeout: handshake_timeout(),
        stderr: Arc::new(move |bytes| sink.host(bytes)),
    };
    logs.line(&format!("host: starting {}", layout.host.display()));
    host::start(&launch, |err| {
        logs.line(&format!("host: {err}; retrying once"))
    })
}

fn watch(app: AppHandle, mut child: Child, ended: Arc<(Mutex<bool>, Condvar)>, logs: Arc<Logs>) {
    std::thread::spawn(move || {
        let code = child.wait().ok().and_then(|status| status.code());
        let (lock, cv) = &*ended;
        if let Ok(mut done) = lock.lock() {
            *done = true;
        }
        cv.notify_all();
        if app.state::<Lease>().quitting.load(Ordering::SeqCst) {
            return;
        }
        logs.line(&format!("host: exited code={code:?}"));
        // The quit act races this thread, landing a beat after the child
        // dies. Past the beat, any host exit without an orderly quit closes
        // the shell instead of stranding the window on a dead origin.
        std::thread::sleep(QUIT_BEAT);
        if app.state::<Lease>().quitting.load(Ordering::SeqCst) {
            return;
        }
        app.exit(1);
    });
}

/// The acts the kernel asks of the application, because only this process can
/// do them. relaunch and quit are a handover and arrive in that order, so the
/// restart is armed before the end rather than by coincidence.
fn perform(app: &AppHandle, act: &str) {
    if let Some(logs) = app.try_state::<Arc<Logs>>() {
        logs.line(&format!("kernel act: {act}"));
    }
    let lease = app.state::<Lease>();
    match act {
        "show" => show_main(app),
        "relaunch" => lease.relaunch.store(true, Ordering::SeqCst),
        "quit" if lease.relaunch.load(Ordering::SeqCst) => {
            lease.quitting.store(true, Ordering::SeqCst);
            app.request_restart();
        }
        "quit" => {
            lease.quitting.store(true, Ordering::SeqCst);
            app.exit(0);
        }
        _ => {}
    }
}

fn credential(token: &str) -> cookie::Cookie<'static> {
    // HttpOnly because nothing in the page reads it, Strict because no
    // cross-site navigation needs it, no expiry so it dies with this launch.
    cookie::Cookie::build((TOKEN_COOKIE, token.to_string()))
        .domain("127.0.0.1")
        .path("/")
        .http_only(true)
        .secure(false)
        .same_site(cookie::SameSite::Strict)
        .build()
}

fn grant(app: &AppHandle, origin: &str) -> tauri::Result<()> {
    app.add_capability(kernel_page_capability(origin))
}

// The kernel-page capability. Bound to the main webview alone: tauri
// OR-matches a capability's window labels against webview labels, so a
// window arm would carry every guest view added to this window, and a
// frame inside one that lands on the kernel origin would reach the shell's
// verbs and its events.
fn kernel_page_capability(origin: &str) -> CapabilityBuilder {
    let mut capability = CapabilityBuilder::new("kernel-page")
        .remote(format!("{origin}/*"))
        .local(false)
        .webview(MAIN)
        .permission("core:event:default");
    for name in commands::NAMES {
        capability = capability.permission(format!("allow-{}", name.replace('_', "-")));
    }
    capability
}

// host.md freezes REASONIX_STUDIO_HANDSHAKE_TIMEOUT_MS as the desktop
// override; a mangled value keeps the default instead of guessing, and the
// range stays inside what a scanner-held binary needs versus an abandoned
// launch.
fn handshake_timeout_from(raw: Option<&str>) -> Duration {
    let ms = raw
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(host::HANDSHAKE_TIMEOUT.as_millis() as u64);
    Duration::from_millis(ms.clamp(1_000, 600_000))
}

fn handshake_timeout() -> Duration {
    handshake_timeout_from(
        std::env::var("REASONIX_STUDIO_HANDSHAKE_TIMEOUT_MS")
            .ok()
            .as_deref(),
    )
}

// A window larger than the display it opens on puts its own title bar off
// screen, and a frameless one takes the only way to move it along with it.
fn fitted(app: &AppHandle) -> LogicalSize<f64> {
    let Ok(Some(monitor)) = app.primary_monitor() else {
        return DEFAULT_SIZE;
    };
    let area = monitor
        .work_area()
        .size
        .to_logical::<f64>(monitor.scale_factor());
    LogicalSize {
        width: DEFAULT_SIZE.width.min(area.width).max(MIN_SIZE.width),
        height: DEFAULT_SIZE.height.min(area.height).max(MIN_SIZE.height),
    }
}

fn open_window(
    app: &AppHandle,
    handshake: &host::Handshake,
    layout: &layout::Layout,
) -> tauri::Result<()> {
    let origin = handshake.origin.as_str();
    let saved = prefs::load(&prefs::file(&layout.config));
    let source = theme::source(&saved, None);
    let title_bar = cfg!(any(windows, target_os = "macos"));
    let script = bridge::script(&bridge::Facts {
        origin,
        platform: bridge::platform(),
        title_bar,
        saved: &saved,
    });
    let expected = origin.to_string();
    let size = fitted(app);
    #[allow(unused_mut)]
    let mut builder = WebviewWindowBuilder::new(
        app,
        MAIN,
        WebviewUrl::External("about:blank".parse().expect("about:blank parses")),
    )
    .title("Reasonix Studio")
    .inner_size(size.width, size.height)
    .min_inner_size(MIN_SIZE.width, MIN_SIZE.height)
    .center()
    .visible(false)
    .theme(theme::theme(source))
    .background_color(theme::ground(source, true))
    .initialization_script(script)
    .on_navigation(move |url| {
        url.as_str() == "about:blank" || url.origin().ascii_serialization() == expected
    })
    .on_new_window(|_, _| NewWindowResponse::Deny)
    .disable_drag_drop_handler();
    #[cfg(windows)]
    {
        builder = builder.decorations(false);
    }
    #[cfg(target_os = "macos")]
    {
        builder = builder
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true)
            .traffic_light_position(tauri::LogicalPosition::new(11.0, 20.0));
    }
    // Only Windows can apply the choice; elsewhere the launch keeps its GPU and
    // says so, rather than reporting an override it never made.
    let off = graphics::disable_gpu(&saved);
    #[cfg(windows)]
    if off {
        builder = builder.additional_browser_args("--disable-gpu");
    }
    app.manage(graphics::GraphicsState {
        launched_off: off && cfg!(windows),
    });
    grant(app, origin)?;
    let window = builder.build()?;
    window.set_cookie(credential(&handshake.token))?;
    window.navigate(
        format!("{origin}/")
            .parse()
            .expect("a loopback origin parses"),
    )?;
    commands::apply_theme(&window, &app.state::<commands::Shell>());
    window.show()?;
    let _ = window.set_focus();
    crate::shortcut::init(app);
    crate::browser::start(
        app.clone(),
        app.state::<commands::Shell>().client.clone(),
        origin.to_string(),
        layout.config.join("browser"),
    );
    #[cfg(windows)]
    crate::dropwin::install(app, &window);
    Ok(())
}

fn chinese() -> bool {
    sys_locale::get_locale().is_some_and(|l| l.to_ascii_lowercase().starts_with("zh"))
}

fn fail(app: &AppHandle, logs: Option<&Logs>, why: &str) {
    let words = logs.map(Logs::host_tail).unwrap_or_default();
    if let Some(logs) = logs {
        logs.line(&format!("startup failed: {why}"));
    }
    let mut detail = if words.is_empty() {
        why.to_string()
    } else {
        format!("{why}\n\n{words}")
    };
    if detail.len() > DETAIL_LIMIT {
        let cut = (0..=DETAIL_LIMIT)
            .rev()
            .find(|&i| detail.is_char_boundary(i))
            .unwrap_or(0);
        detail.truncate(cut);
        detail.push('…');
    }
    let title = if chinese() {
        "Reasonix Studio 无法启动"
    } else {
        "Reasonix Studio could not start"
    };
    app.dialog()
        .message(detail)
        .title(title)
        .kind(MessageDialogKind::Error)
        .blocking_show();
    app.exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_credential_cookie_is_host_bound_and_never_readable() {
        let cookie = credential("t3");
        assert_eq!(cookie.name(), TOKEN_COOKIE);
        assert_eq!(cookie.value(), "t3");
        assert_eq!(cookie.domain(), Some("127.0.0.1"));
        assert_eq!(cookie.path(), Some("/"));
        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.same_site(), Some(cookie::SameSite::Strict));
        assert!(cookie.expires().is_none() && cookie.max_age().is_none());
    }

    #[test]
    fn the_capability_names_only_the_main_webview() {
        use tauri::ipc::RuntimeCapability;
        let tauri::utils::acl::capability::CapabilityFile::Capability(cap) =
            kernel_page_capability("http://127.0.0.1:9/").build()
        else {
            panic!("the kernel page grants through a single capability");
        };
        assert!(
            cap.windows.is_empty(),
            "a window match carries every guest webview on the main window: tauri OR-matches the webview label against the window label"
        );
        assert_eq!(cap.webviews, vec![MAIN.to_string()]);
        assert!(!cap.local);
    }

    #[test]
    fn the_handshake_timeout_override_is_bounded() {
        assert_eq!(
            handshake_timeout_from(None),
            host::HANDSHAKE_TIMEOUT,
            "no override keeps the frozen default"
        );
        assert_eq!(
            handshake_timeout_from(Some("  junk  ")),
            host::HANDSHAKE_TIMEOUT,
            "a mangled override keeps the default"
        );
        assert_eq!(handshake_timeout_from(Some("250")), Duration::from_secs(1));
        assert_eq!(
            handshake_timeout_from(Some("999999999999")),
            Duration::from_secs(600)
        );
        assert_eq!(
            handshake_timeout_from(Some("120000")),
            Duration::from_secs(120)
        );
    }

    #[test]
    fn a_released_lease_waits_only_as_long_as_the_drain() {
        let lease = Lease::default();
        let began = std::time::Instant::now();
        {
            let (lock, cv) = &*lease.ended;
            *lock.lock().expect("lock") = true;
            cv.notify_all();
        }
        lease.release();
        assert!(began.elapsed() < DRAIN);
        assert!(lease.quitting.load(Ordering::SeqCst));
    }
}
