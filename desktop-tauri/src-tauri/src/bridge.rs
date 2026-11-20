// The page's window onto this shell: `window.reasonixHost`, defined before any
// page script runs and only on the kernel's own origin. Verbs only — the
// credential never crosses here, and the page's origin is already its own.
//
// localStorage is keyed by origin and the kernel's port is new each launch, so
// the page's saved preferences ride in the script itself and are restored
// before the page reads them; a key the page already holds wins.
use crate::prefs::Prefs;

pub struct Facts<'a> {
    pub origin: &'a str,
    pub platform: &'a str,
    pub title_bar: bool,
    pub saved: &'a Prefs,
}

pub fn platform() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

const TEMPLATE: &str = r#"(() => {
  "use strict";
  const ORIGIN = __ORIGIN__;
  if (window.location.origin !== ORIGIN || window.top !== window) return;
  const SAVED = __SAVED__;
  const PREFIX = "rx-";
  const SEEDED = "reasonix-shell-seeded";
  const invoke = (cmd, args, options) => {
    const ipc = window.__TAURI_INTERNALS__;
    if (!ipc) return Promise.reject(new Error("the shell bridge is not ready"));
    return ipc.invoke(cmd, args, options);
  };
  const text = (value) => String(value ?? "");
  const bytesOf = (value) => (value instanceof Uint8Array ? value : new Uint8Array(value));
  const LISTEN = "plugin:event|listen";
  const UNLISTEN = "plugin:event|unlisten";
  const onEvent = (event, handler) => {
    const ipc = window.__TAURI_INTERNALS__;
    if (!ipc) return () => {};
    const handlerId = ipc.transformCallback(handler);
    let eventId = null;
    let active = true;
    invoke(LISTEN, { event, target: { kind: "Any" }, handler: handlerId })
      .then((id) => { eventId = id; if (!active) invoke(UNLISTEN, { event, eventId: id }); })
      .catch(() => {});
    return () => {
      active = false;
      if (eventId === null) return;
      const internals = window.__TAURI_EVENT_PLUGIN_INTERNALS__;
      if (internals) internals.unregisterListener(event, eventId);
      invoke(UNLISTEN, { event, eventId }).catch(() => {});
    };
  };

  // The native drop target hands the window paths and coordinates, never File
  // objects; the page's drop zones read DOM DragEvents, so each native event is
  // rebuilt as the DOM event the zone already listens for, at the point given.
  const fireDrag = (type, position, transfer) => {
    const x = position && typeof position.x === "number" ? position.x : 0;
    const y = position && typeof position.y === "number" ? position.y : 0;
    const el = document.elementFromPoint(x, y) || document.body;
    el.dispatchEvent(new DragEvent(type, {
      bubbles: true, cancelable: true, clientX: x, clientY: y, dataTransfer: transfer,
    }));
  };
  const typed = (types) => {
    const dt = new DataTransfer();
    for (const t of types) dt.setData(t, "");
    return dt;
  };
  const dropListeners = new Set();
  onEvent("tauri://drag-enter", (e) => fireDrag("dragover", e.payload.position, typed(["Files"])));
  onEvent("tauri://drag-over", (e) => fireDrag("dragover", e.payload.position, typed(["Files"])));
  onEvent("tauri://drag-leave", () => fireDrag("dragleave", null, typed(["Files"])));
  onEvent("tauri://drag-drop", (e) => {
    const paths = Array.isArray(e.payload.paths) ? e.payload.paths : [];
    fireDrag("drop", e.payload.position, typed(["Files"]));
    for (const listener of dropListeners) listener(paths);
  });
  onEvent("reasonix://drag-enter", (e) => fireDrag("dragover", e.payload.position, typed(e.payload.types || [])));
  onEvent("reasonix://drag-over", (e) => fireDrag("dragover", e.payload.position, typed(e.payload.types || [])));
  onEvent("reasonix://drag-leave", () => fireDrag("dragleave", null, typed(["Files"])));
  onEvent("reasonix://drag-drop", (e) => {
    const dt = new DataTransfer();
    for (const entry of e.payload.entries || []) dt.setData(entry[0], entry[1]);
    fireDrag("drop", e.payload.position, dt);
  });

  const host = {
    shell: "tauri",
    platform: __PLATFORM__,
    titleBar: __TITLEBAR__,
    setUnreadBadge: (unread) => invoke("set_unread_badge", { unread: unread === true }),
    syncTheme: (mode, scheme) => {
      invoke("sync_theme", { mode: text(mode), scheme: text(scheme) }).catch(() => {});
    },
    minimiseWindow: () => invoke("window_minimise"),
    toggleMaximiseWindow: () => invoke("window_toggle_maximise"),
    isWindowMaximised: () => invoke("window_is_maximised"),
    closeWindow: () => invoke("window_close"),
    openExternal: (url) => invoke("open_external", { url: text(url) }),
    revealPath: (base, rel) => invoke("reveal_path", { base: text(base), rel: text(rel) }),
    openFile: (base, rel) => invoke("open_file", { base: text(base), rel: text(rel) }),
    revealWorkspace: (root) => invoke("reveal_workspace", { root: text(root) }),
    revealLogs: () => invoke("reveal_logs"),
    openWorkspace: (root) => invoke("open_workspace", { root: text(root) }),
    saveText: (name, content) => invoke("save_text", { name: text(name), content: text(content) }),
    saveBytes: (name, bytes) =>
      invoke("save_bytes", bytesOf(bytes), { headers: { "x-reasonix-name": encodeURIComponent(text(name)) } }),
    pickFolder: (startIn) => invoke("pick_folder", { startIn: text(startIn) }),
    captureWindow: () =>
      invoke("window_capture").then((b) => (b && b.byteLength ? new Uint8Array(b) : null)).catch(() => null),
    graphics: () => invoke("shell_graphics"),
    globalShortcut: () => invoke("shell_global_shortcut"),
    setGlobalShortcut: (binding) => invoke("shell_set_global_shortcut", { binding: text(binding) }),
    processMemory: () => invoke("shell_process_memory"),
    showTextContextMenu: (request) => invoke("text_context_menu", { request }),
    onDropPaths: (listener) => {
      dropListeners.add(listener);
      return () => dropListeners.delete(listener);
    },
    showBrowserView: (target, rect) => invoke("browser_show", { target: text(target), rect }),
    hideBrowserView: () => invoke("browser_hide"),
    freezeBrowserView: () => invoke("browser_freeze"),
    controlBrowserView: (target, action) => invoke("browser_control", { target: text(target), action: text(action) }),
    navigateBrowserView: (target, address) => invoke("browser_navigate", { target: text(target), address: text(address) }),
    onBrowserLoadState: (listener) => onEvent("browser://load-state", (e) => listener(e.payload)),
    onBrowserLogin: (listener) => onEvent("browser://login", (e) => listener(e.payload)),
    answerBrowserLogin: (id, username, password) =>
      invoke("browser_answer_login", { id: text(id), username: text(username), password: text(password) }),
    trustBrowserCertificate: (target) => invoke("browser_trust_certificate", { target: text(target) }),
    pickBrowserElement: (target, labels) => invoke("browser_pick", { target: text(target), labels }),
    cancelBrowserPick: (target) => invoke("browser_pick_cancel", { target: text(target) }),
  };
  Object.defineProperty(window, "reasonixHost", { value: Object.freeze(host), enumerable: false, configurable: false, writable: false });

  try {
    if (sessionStorage.getItem(SEEDED) !== "1") {
      for (const [key, value] of Object.entries(SAVED)) {
        if (key.startsWith(PREFIX) && localStorage.getItem(key) === null) localStorage.setItem(key, String(value));
      }
      sessionStorage.setItem(SEEDED, "1");
    }
  } catch {}
  const own = () => {
    const out = {};
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      if (key && key.startsWith(PREFIX)) out[key] = localStorage.getItem(key) ?? "";
    }
    return out;
  };
  let sent = "";
  const flush = () => {
    try {
      const prefs = own();
      const next = JSON.stringify(prefs);
      if (next === sent) return;
      sent = next;
      invoke("prefs_save", { prefs }).catch(() => { sent = ""; });
    } catch {}
  };
  setInterval(flush, 1000);
  window.addEventListener("pagehide", flush);
  document.addEventListener("visibilitychange", () => { if (document.visibilityState === "hidden") flush(); });
})();
"#;

fn js(value: &impl serde::Serialize) -> String {
    serde_json::to_string(value).expect("a string map serialises")
}

pub fn script(facts: &Facts<'_>) -> String {
    TEMPLATE
        .replace("__ORIGIN__", &js(&facts.origin))
        .replace("__SAVED__", &js(facts.saved))
        .replace("__PLATFORM__", &js(&facts.platform))
        .replace(
            "__TITLEBAR__",
            if facts.title_bar { "true" } else { "false" },
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_names_its_origin_and_carries_no_placeholder() {
        let mut saved = Prefs::new();
        saved.insert("rx-theme".into(), "dark\"</script>".into());
        let script = script(&Facts {
            origin: "http://127.0.0.1:5281",
            platform: "windows",
            title_bar: true,
            saved: &saved,
        });
        assert!(script.contains(r#"const ORIGIN = "http://127.0.0.1:5281";"#));
        assert!(script.contains(r#"platform: "windows""#));
        assert!(script.contains("titleBar: true"));
        assert!(script.contains(r#""rx-theme":"dark\"</script>""#));
        for placeholder in ["__ORIGIN__", "__SAVED__", "__PLATFORM__", "__TITLEBAR__"] {
            assert!(!script.contains(placeholder), "{placeholder} is filled");
        }
    }

    #[test]
    fn the_bridge_names_exactly_the_declared_commands() {
        let script = script(&Facts {
            origin: "o",
            platform: "linux",
            title_bar: false,
            saved: &Prefs::new(),
        });
        let mut named: Vec<&str> = script
            .split("invoke(\"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .collect();
        named.sort_unstable();
        named.dedup();
        let mut declared = crate::commands::NAMES.to_vec();
        declared.sort_unstable();
        assert_eq!(named, declared);
    }
}
