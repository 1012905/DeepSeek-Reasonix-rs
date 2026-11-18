// The kernel's agent browser: child webviews it drives over CDP-shaped frames.
// One actor owns the protocol state; the relay carries frames to and from the
// kernel; the window's own page drives the views through the commands below.
//
// Platform reach: the native debugger and certificate events are Windows-only
// today. Elsewhere the views still create, position, navigate and pick through
// the cross-platform webview APIs, but kernel CDP commands, screenshots, login
// and certificate trust answer unsupported rather than pretend — see cdp/certs.
mod cdp;
mod certs;
mod geometry;
mod pick;
mod policy;
mod protocol;
mod relay;
mod views;

use crate::client::KernelClient;
use protocol::{Incoming, Notify, Protocol};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::Duration;
use tauri::{AppHandle, Manager};
use views::BrowserViews;

const SLEEP_AFTER: Duration = Duration::from_secs(15 * 60);
const MAX_AWAKE: usize = 6;

/// Wires the browser to the kernel and the window. Called once the kernel's
/// handshake is known and the window is open on it.
pub fn start(app: AppHandle, client: KernelClient, kernel_origin: String, data_root: PathBuf) {
    let views = BrowserViews::new(
        app.clone(),
        kernel_origin,
        data_root,
        Some(SLEEP_AFTER),
        MAX_AWAKE,
    );
    app.manage(views.clone());

    let (in_tx, in_rx) = mpsc::channel::<Incoming>();
    let (post_tx, post_rx) = mpsc::channel::<Value>();
    let (note_tx, note_rx) = mpsc::channel::<Notify>();
    let mut protocol = Protocol::new(views.clone(), post_tx, note_tx);

    let frame_tx = in_tx.clone();
    let drop_tx = in_tx.clone();
    let relay = relay::start(
        client,
        move |frame| {
            let _ = frame_tx.send(Incoming::Frame(frame));
        },
        move || {
            let _ = drop_tx.send(Incoming::Drop);
        },
    );
    app.manage(relay.clone());
    std::thread::spawn(move || {
        while let Ok(frame) = post_rx.recv() {
            relay.post(frame);
        }
    });
    let note_forward = in_tx.clone();
    std::thread::spawn(move || {
        while let Ok(note) = note_rx.recv() {
            let _ = note_forward.send(Incoming::Notify(note));
        }
    });
    std::thread::spawn(move || {
        while let Ok(msg) = in_rx.recv() {
            match msg {
                Incoming::Frame(frame) => protocol.receive(frame),
                Incoming::Notify(note) => protocol.notify(note),
                Incoming::Drop => protocol.drop_all(),
            }
        }
    });
}

fn views(app: &AppHandle) -> Arc<BrowserViews> {
    app.state::<Arc<BrowserViews>>().inner().clone()
}

#[tauri::command]
pub async fn browser_show(app: AppHandle, target: String, rect: geometry::Rect) {
    let views = views(&app);
    let _ = tauri::async_runtime::spawn_blocking(move || views.show(&target, rect)).await;
}

#[tauri::command]
pub async fn browser_hide(app: AppHandle) {
    let views = views(&app);
    let _ = tauri::async_runtime::spawn_blocking(move || views.hide()).await;
}

#[tauri::command]
pub async fn browser_freeze(app: AppHandle) -> String {
    let views = views(&app);
    tauri::async_runtime::spawn_blocking(move || views.freeze())
        .await
        .unwrap_or_default()
}

#[tauri::command]
pub async fn browser_control(app: AppHandle, target: String, action: String) {
    let views = views(&app);
    let _ = tauri::async_runtime::spawn_blocking(move || views.control(&target, &action)).await;
}

#[tauri::command]
pub async fn browser_navigate(app: AppHandle, target: String, address: String) -> String {
    let views = views(&app);
    tauri::async_runtime::spawn_blocking(move || views.navigate(&target, &address))
        .await
        .unwrap_or_else(|_| "scheme".to_string())
}

#[tauri::command]
pub async fn browser_trust_certificate(app: AppHandle, target: String) -> bool {
    let views = views(&app);
    tauri::async_runtime::spawn_blocking(move || views.trust_certificate(&target))
        .await
        .unwrap_or(false)
}

#[tauri::command]
pub async fn browser_answer_login(app: AppHandle, id: String, username: String, password: String) {
    let views = views(&app);
    let _ =
        tauri::async_runtime::spawn_blocking(move || views.answer_login(&id, &username, &password))
            .await;
}

#[tauri::command]
pub async fn browser_pick(app: AppHandle, target: String, labels: Value) -> Value {
    let views = views(&app);
    tauri::async_runtime::spawn_blocking(move || views.pick(&target, labels))
        .await
        .unwrap_or_else(|_| json!({ "cancelled": true, "reason": "gone" }))
}

#[tauri::command]
pub async fn browser_pick_cancel(app: AppHandle, target: String) {
    let views = views(&app);
    let _ =
        tauri::async_runtime::spawn_blocking(move || views.cancel_pick(&target, "dismissed")).await;
}

/// Ends the browser with the shell: the stream stops reconnecting and every
/// page is buried, its pending logins cancelled rather than left waiting.
pub fn shutdown(app: &AppHandle) {
    if let Some(relay) = app.try_state::<relay::Relay>() {
        relay.stop();
    }
    if let Some(views) = app.try_state::<Arc<BrowserViews>>() {
        views.close_all();
    }
}
