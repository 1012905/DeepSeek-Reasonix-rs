// What the launch actually applied for GPU compositing, and what is saved.
// This shell has no readable compositing status, so it reports none: the page
// renders "" as "not reported yet", which is the truth here.
use crate::prefs::Prefs;
use serde::Serialize;
use tauri::{AppHandle, Manager};

pub const PREF_KEY: &str = "rx-hw-accel";

/// Managed at boot with what the launch applied; only Windows can apply it.
pub struct GraphicsState {
    pub launched_off: bool,
}

pub fn acceleration_off(saved: &Prefs) -> bool {
    saved.get(PREF_KEY).is_some_and(|v| v == "off")
}

/// A launch-only override for a window that will not draw; neither source is
/// written back to the preferences.
pub fn disable_gpu(saved: &Prefs) -> bool {
    acceleration_off(saved)
        || std::env::var("REASONIX_DISABLE_GPU").is_ok_and(|v| v == "1")
        || std::env::args().any(|a| a == "--disable-gpu")
}

#[derive(Serialize)]
pub struct GraphicsInfo {
    #[serde(rename = "launchedOff")]
    launched_off: bool,
    #[serde(rename = "savedOff")]
    saved_off: bool,
    compositing: String,
}

#[tauri::command]
pub async fn shell_graphics(app: AppHandle) -> Result<GraphicsInfo, String> {
    let shell = app.state::<crate::commands::Shell>();
    let saved = crate::prefs::load(&shell.prefs_file);
    let launched_off = app
        .try_state::<GraphicsState>()
        .is_some_and(|s| s.launched_off);
    Ok(GraphicsInfo {
        launched_off,
        saved_off: acceleration_off(&saved),
        compositing: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_explicit_off_turns_acceleration_off() {
        let mut saved = Prefs::new();
        assert!(!acceleration_off(&saved));
        saved.insert(PREF_KEY.into(), "on".into());
        assert!(!acceleration_off(&saved));
        saved.insert(PREF_KEY.into(), "off".into());
        assert!(acceleration_off(&saved));
    }

    #[test]
    fn a_saved_off_reaches_the_launch_decision() {
        let mut saved = Prefs::new();
        saved.insert(PREF_KEY.into(), "off".into());
        assert!(disable_gpu(&saved));
    }
}
