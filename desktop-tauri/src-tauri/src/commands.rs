// The verbs the kernel page may ask of this shell. Each is reachable only from
// the main window on this launch's kernel origin: the capability granting them
// is bound to both at boot, so no handler re-derives who is asking.
use crate::reveal::{self, Refusal, refused};
use crate::{badge, links, prefs, theme};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

pub const NAMES: &[&str] = &[
    "window_minimise",
    "window_toggle_maximise",
    "window_is_maximised",
    "window_close",
    "open_external",
    "reveal_path",
    "open_file",
    "reveal_workspace",
    "open_workspace",
    "reveal_logs",
    "pick_folder",
    "save_text",
    "save_bytes",
    "prefs_save",
    "sync_theme",
    "set_unread_badge",
    "window_capture",
    "shell_graphics",
    "shell_global_shortcut",
    "shell_set_global_shortcut",
    "shell_process_memory",
    "text_context_menu",
    "browser_show",
    "browser_hide",
    "browser_freeze",
    "browser_control",
    "browser_navigate",
    "browser_trust_certificate",
    "browser_answer_login",
    "browser_pick",
    "browser_pick_cancel",
];

/// A person needs longer than this to read a panel and choose in it. On Linux
/// a missing file-chooser portal answers like a dismissal, only sooner.
const UNDRAWN: Duration = Duration::from_millis(400);
const NAME_HEADER: &str = "x-reasonix-name";

pub struct Shell {
    pub client: crate::client::KernelClient,
    pub logs_dir: PathBuf,
    pub prefs_file: PathBuf,
    pub painted: Mutex<Option<theme::Paint>>,
}

fn text(err: impl std::fmt::Display) -> String {
    err.to_string()
}

pub fn apply_theme(window: &WebviewWindow, shell: &Shell) {
    let saved = prefs::load(&shell.prefs_file);
    let painted = shell.painted.lock().ok().and_then(|p| p.clone());
    let source = theme::source(&saved, painted.as_ref());
    let _ = window.set_theme(theme::theme(source));
    let system_dark = matches!(window.theme(), Ok(tauri::Theme::Dark));
    let _ = window.set_background_color(Some(theme::ground(source, system_dark)));
}

#[tauri::command]
pub fn window_minimise(window: WebviewWindow) -> Result<(), String> {
    window.minimize().map_err(text)
}

#[tauri::command]
pub fn window_toggle_maximise(window: WebviewWindow) -> Result<(), String> {
    if window.is_maximized().map_err(text)? {
        window.unmaximize()
    } else {
        window.maximize()
    }
    .map_err(text)
}

#[tauri::command]
pub fn window_is_maximised(window: WebviewWindow) -> bool {
    window.is_maximized().unwrap_or(false)
}

#[tauri::command]
pub fn window_close(window: WebviewWindow) -> Result<(), String> {
    window.close().map_err(text)
}

#[tauri::command]
pub fn open_external(url: String) -> Result<(), String> {
    match links::external_target(&url) {
        Some(target) => tauri_plugin_opener::open_url(target, None::<&str>).map_err(text),
        None => Ok(()),
    }
}

fn shown(result: Result<(), Refusal>) -> Option<Refusal> {
    result.err()
}

fn select(target: &str) -> Result<(), Refusal> {
    tauri_plugin_opener::reveal_item_in_dir(target).map_err(|e| refused(e.to_string()))
}

fn open(target: &Path) -> Result<(), Refusal> {
    tauri_plugin_opener::open_path(target, None::<&str>).map_err(|e| refused(e.to_string()))
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(text)
}

#[tauri::command]
pub async fn reveal_path(
    shell: State<'_, Shell>,
    base: String,
    rel: String,
) -> Result<Option<Refusal>, String> {
    let client = shell.client.clone();
    blocking(move || {
        shown(reveal::locate_pane(&client, &base, &rel, cfg!(windows)).and_then(|t| select(&t)))
    })
    .await
}

#[tauri::command]
pub async fn open_file(
    shell: State<'_, Shell>,
    base: String,
    rel: String,
) -> Result<Option<Refusal>, String> {
    let client = shell.client.clone();
    blocking(move || {
        shown(
            reveal::locate_pane(&client, &base, &rel, cfg!(windows)).and_then(|target| {
                let target = PathBuf::from(target);
                reveal::openable_document(&target)?;
                open(&target)
            }),
        )
    })
    .await
}

#[tauri::command]
pub async fn reveal_workspace(
    shell: State<'_, Shell>,
    root: String,
) -> Result<Option<Refusal>, String> {
    let client = shell.client.clone();
    blocking(move || {
        shown(reveal::locate_workspace(&client, &root, cfg!(windows)).and_then(|t| select(&t)))
    })
    .await
}

#[tauri::command]
pub async fn open_workspace(
    shell: State<'_, Shell>,
    root: String,
) -> Result<Option<Refusal>, String> {
    let client = shell.client.clone();
    blocking(move || {
        shown(
            reveal::locate_workspace(&client, &root, cfg!(windows)).and_then(|target| {
                let path = PathBuf::from(&target);
                if reveal::plain_folder(&path) {
                    open(&path)
                } else {
                    select(&target)
                }
            }),
        )
    })
    .await
}

#[tauri::command]
pub fn reveal_logs(shell: State<'_, Shell>) -> Option<Refusal> {
    let _ = std::fs::create_dir_all(&shell.logs_dir);
    shown(open(&shell.logs_dir))
}

/// The path chosen; "" for a panel the person closed; null for one that could
/// not be drawn, which the page answers by asking for the path in words.
pub fn folder_answer(picked: Option<PathBuf>, elapsed: Duration, linux: bool) -> Option<String> {
    match picked {
        Some(path) => Some(path.to_string_lossy().into_owned()),
        None if linux && elapsed < UNDRAWN => None,
        None => Some(String::new()),
    }
}

#[tauri::command]
pub async fn pick_folder(
    app: AppHandle,
    window: WebviewWindow,
    start_in: String,
) -> Result<Option<String>, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut dialog = app.dialog().file().set_parent(&window);
    if !start_in.is_empty() && Path::new(&start_in).is_dir() {
        dialog = dialog.set_directory(&start_in);
    }
    let began = Instant::now();
    dialog.pick_folder(move |picked| {
        let _ = tx.send(picked.and_then(|p| p.into_path().ok()));
    });
    let picked = blocking(move || rx.recv().ok().flatten()).await?;
    Ok(folder_answer(
        picked,
        began.elapsed(),
        cfg!(target_os = "linux"),
    ))
}

async fn save_to(
    app: &AppHandle,
    window: &WebviewWindow,
    name: &str,
    bytes: Vec<u8>,
) -> Result<String, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .set_parent(window)
        .set_file_name(name)
        .save_file(move |picked| {
            let _ = tx.send(picked.and_then(|p| p.into_path().ok()));
        });
    let Some(path) = blocking(move || rx.recv().ok().flatten()).await? else {
        return Ok(String::new());
    };
    std::fs::write(&path, bytes).map_err(text)?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn save_text(
    app: AppHandle,
    window: WebviewWindow,
    name: String,
    content: String,
) -> Result<String, String> {
    save_to(&app, &window, &name, content.into_bytes()).await
}

pub fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[tauri::command]
pub async fn save_bytes(
    app: AppHandle,
    window: WebviewWindow,
    request: Request<'_>,
) -> Result<String, String> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("save_bytes takes raw bytes".into());
    };
    let name = request
        .headers()
        .get(NAME_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(percent_decode)
        .unwrap_or_default();
    save_to(&app, &window, &name, bytes.clone()).await
}

#[tauri::command]
pub fn prefs_save(shell: State<'_, Shell>, window: WebviewWindow, prefs: Map<String, Value>) {
    if prefs::save(&shell.prefs_file, &prefs::pick(&prefs)).is_ok() {
        apply_theme(&window, &shell);
    }
}

#[tauri::command]
pub fn sync_theme(shell: State<'_, Shell>, window: WebviewWindow, mode: String, scheme: String) {
    let Some(paint) = theme::validate(&mode, &scheme) else {
        return;
    };
    if let Ok(mut painted) = shell.painted.lock() {
        *painted = Some(paint);
    }
    apply_theme(&window, &shell);
}

#[tauri::command]
pub fn set_unread_badge(window: WebviewWindow, unread: bool) -> Result<(), String> {
    badge::set(&window, unread).map_err(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_is_declared_to_the_acl() {
        let build = include_str!("../build.rs");
        for name in NAMES {
            assert!(
                build.contains(&format!("\"{name}\"")),
                "{name} is missing from build.rs"
            );
        }
        assert_eq!(build.matches("\",").count(), NAMES.len());
    }

    #[test]
    fn a_closed_panel_and_an_undrawn_one_answer_differently() {
        let quick = Duration::from_millis(50);
        let slow = Duration::from_secs(2);
        assert_eq!(
            folder_answer(Some(PathBuf::from("/w")), quick, true).as_deref(),
            Some("/w")
        );
        assert_eq!(folder_answer(None, slow, true).as_deref(), Some(""));
        assert_eq!(folder_answer(None, quick, true), None);
        assert_eq!(folder_answer(None, quick, false).as_deref(), Some(""));
    }

    #[test]
    fn names_decode_from_the_header() {
        assert_eq!(
            percent_decode("report%20%E6%8A%A5%E5%91%8A.png"),
            "report 报告.png"
        );
        assert_eq!(percent_decode("plain.txt"), "plain.txt");
        assert_eq!(percent_decode("bad%zz"), "bad%zz");
        assert_eq!(percent_decode("end%2"), "end%2");
    }
}
