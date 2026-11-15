// The shell owns OS capability and no business state: the kernel is a sidecar
// it spawns and holds a lease on, and the window is that kernel's own page.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod badge;
mod boot;
mod bridge;
mod client;
mod commands;
mod host;
mod layout;
mod links;
mod logs;
mod prefs;
mod reveal;
mod theme;

/// A per-test directory under target/, which every runner can write to.
#[cfg(test)]
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test-scratch")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

use tauri::Manager;
use tauri_plugin_window_state::StateFlags;

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            boot::show_main(app)
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
                .build(),
        )
        .manage(boot::Lease::default())
        .invoke_handler(tauri::generate_handler![
            commands::window_minimise,
            commands::window_toggle_maximise,
            commands::window_is_maximised,
            commands::window_close,
            commands::open_external,
            commands::reveal_path,
            commands::open_file,
            commands::reveal_workspace,
            commands::open_workspace,
            commands::reveal_logs,
            commands::pick_folder,
            commands::save_text,
            commands::save_bytes,
            commands::prefs_save,
            commands::sync_theme,
            commands::set_unread_badge,
        ])
        .setup(|app| {
            boot::start(app.handle().clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("building the Reasonix Studio shell");
    app.run(|app, event| match event {
        tauri::RunEvent::Exit => app.state::<boot::Lease>().release(),
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => boot::show_main(app),
        _ => {}
    });
}
