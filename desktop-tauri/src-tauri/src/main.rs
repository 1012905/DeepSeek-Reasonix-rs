// The shell owns OS capability and no business state; the kernel is a sidecar
// this process spawns and holds a lease on. Business surface arrives in later
// phases — the empty window is the Phase 1 deliverable.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
