// The page is the kernel's own origin, which Tauri counts as remote: it may
// reach only the commands this manifest declares, and only through the
// capability the shell grants that one origin at launch.
fn main() {
    let manifest = tauri_build::AppManifest::new().commands(&[
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
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .expect("running tauri-build");
}
