// The one global shortcut that toggles the shell window from anywhere. Its
// binding lives in its own file, not among the page's rx- preferences.
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState as KeyState};

pub const MAX_BINDING_LENGTH: usize = 128;

#[derive(Serialize, Clone)]
pub struct ShortcutInfo {
    binding: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl ShortcutInfo {
    fn new(binding: &str, status: &str) -> Self {
        Self {
            binding: binding.into(),
            status: status.into(),
            error: None,
        }
    }
    fn failed(&self, error: &str) -> Self {
        Self {
            error: Some(error.into()),
            ..self.clone()
        }
    }
}

pub struct ShortcutState {
    info: Mutex<ShortcutInfo>,
    closed: AtomicBool,
}

pub fn file(config_dir: &Path) -> PathBuf {
    config_dir.join("global-shortcut.json")
}

pub fn load(file: &Path) -> String {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|text| serde_json::from_str::<String>(&text).ok())
        .unwrap_or_default()
}

/// Written beside and renamed over, so a crash mid-write leaves the last good
/// copy rather than half of one.
pub fn save(file: &Path, binding: &str) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(binding)?)?;
    std::fs::rename(&tmp, file)
}

/// A binding is modifiers plus one key; a bare key would steal every press of
/// it from every other application.
pub fn valid_binding(value: &str) -> bool {
    if value.len() > MAX_BINDING_LENGTH {
        return false;
    }
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return true;
    }
    let parts: Vec<&str> = trimmed.split('+').collect();
    parts[..parts.len() - 1].iter().any(|part| {
        matches!(
            part.to_ascii_lowercase().as_str(),
            "commandorcontrol"
                | "cmdorctrl"
                | "command"
                | "cmd"
                | "control"
                | "ctrl"
                | "alt"
                | "option"
                | "altgr"
                | "super"
                | "meta"
        )
    })
}

fn press(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(crate::boot::MAIN) {
        let visible = window.is_visible().unwrap_or(false);
        let minimized = window.is_minimized().unwrap_or(false);
        let focused = window.is_focused().unwrap_or(false);
        if visible && !minimized && focused {
            let _ = window.hide();
            return;
        }
    }
    crate::boot::show_main(app);
}

fn register(app: &AppHandle, binding: &str) -> Result<(), &'static str> {
    let Ok(shortcut) = binding.parse::<Shortcut>() else {
        return Err("invalid");
    };
    app.global_shortcut()
        .on_shortcut(shortcut, |app, _, event| {
            if event.state == KeyState::Pressed {
                press(app);
            }
        })
        .map_err(|_| "unavailable")
}

fn apply(app: &AppHandle, state: &ShortcutState, value: &str, persist: bool) -> ShortcutInfo {
    if state.closed.load(Ordering::SeqCst) {
        return state
            .info
            .lock()
            .expect("shortcut state")
            .failed("quitting");
    }
    if value.len() > MAX_BINDING_LENGTH || !valid_binding(value) {
        return state.info.lock().expect("shortcut state").failed("invalid");
    }
    let next = value.trim();
    let current = state.info.lock().expect("shortcut state").clone();
    if next == current.binding && current.status == "registered" {
        return current;
    }
    if !next.is_empty()
        && let Err(error) = register(app, next)
    {
        return current.failed(error);
    }
    if persist {
        let path = file_of(app);
        if save(&path, next).is_err() {
            if !next.is_empty() {
                let _ = app.global_shortcut().unregister(next);
            }
            return current.failed("save");
        }
    }
    if current.status == "registered" && current.binding != next {
        let _ = app.global_shortcut().unregister(current.binding.as_str());
    }
    let info = ShortcutInfo::new(
        next,
        if next.is_empty() {
            "disabled"
        } else {
            "registered"
        },
    );
    *state.info.lock().expect("shortcut state") = info.clone();
    info
}

fn file_of(app: &AppHandle) -> PathBuf {
    let shell = app.state::<crate::commands::Shell>();
    file(shell.prefs_file.parent().unwrap_or(Path::new(".")))
}

/// Loads the saved binding and registers it; a binding that cannot register
/// keeps its text and reports why, the way the saved choice deserves.
pub fn init(app: &AppHandle) {
    let binding = load(&file_of(app));
    let state = ShortcutState {
        info: Mutex::new(ShortcutInfo::new(
            &binding,
            if binding.is_empty() {
                "disabled"
            } else {
                "unavailable"
            },
        )),
        closed: AtomicBool::new(false),
    };
    app.manage(state);
    let state = app.state::<ShortcutState>();
    let result = apply(app, &state, &binding, false);
    if result.error.is_some() {
        *state.info.lock().expect("shortcut state") =
            ShortcutInfo::new(&binding, result.error.as_deref().unwrap_or("unavailable"));
    }
}

pub fn close(app: &AppHandle) {
    let Some(state) = app.try_state::<ShortcutState>() else {
        return;
    };
    state.closed.store(true, Ordering::SeqCst);
    let info = state.info.lock().expect("shortcut state").clone();
    if info.status == "registered" {
        let _ = app.global_shortcut().unregister(info.binding.as_str());
    }
}

#[tauri::command]
pub async fn shell_global_shortcut(app: AppHandle) -> Result<ShortcutInfo, String> {
    Ok(app
        .try_state::<ShortcutState>()
        .map(|s| s.info.lock().expect("shortcut state").clone())
        .unwrap_or_else(|| ShortcutInfo::new("", "unavailable")))
}

#[tauri::command]
pub async fn shell_set_global_shortcut(
    app: AppHandle,
    binding: String,
) -> Result<ShortcutInfo, String> {
    let Some(state) = app.try_state::<ShortcutState>() else {
        return Ok(ShortcutInfo::new("", "unavailable"));
    };
    Ok(apply(&app, &state, &binding, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_binding_needs_a_modifier_before_its_key() {
        assert!(valid_binding("CommandOrControl+Shift+R"));
        assert!(valid_binding("ctrl+alt+d"));
        assert!(valid_binding("Super+Space"));
        assert!(valid_binding(""));
        assert!(valid_binding("   "));
        assert!(!valid_binding("R"));
        assert!(!valid_binding("Shift+R"));
        assert!(!valid_binding(&"x".repeat(MAX_BINDING_LENGTH + 1)));
    }

    #[test]
    fn the_binding_round_trips_through_its_own_file() {
        let dir = crate::scratch("shortcut");
        let path = file(&dir);
        save(&path, "Ctrl+Alt+D").expect("saves");
        assert_eq!(load(&path), "Ctrl+Alt+D");
        assert!(!path.with_extension("json.tmp").exists());
        assert_eq!(load(&dir.join("absent.json")), "");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
