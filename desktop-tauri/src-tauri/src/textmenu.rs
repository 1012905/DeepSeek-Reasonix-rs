// The native text context menu the page asks for over a selection. Edit roles
// do the clipboard work through the page's own commands; the labels follow
// the interface language, as the platform catalogue would not.
use serde_json::Value;
use std::sync::Mutex;
use tauri::menu::{ContextMenu, Menu, MenuItemBuilder, PredefinedMenuItem};
use tauri::{AppHandle, LogicalPosition, Manager, WebviewWindow};

const FLAGS: [&str; 5] = ["editable", "selected", "canUndo", "canRedo", "quote"];

#[derive(Clone, Copy)]
pub struct Request {
    pub editable: bool,
    pub selected: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub quote: bool,
    pub x: f64,
    pub y: f64,
}

pub fn valid_request(value: &Value) -> Option<Request> {
    let map = value.as_object()?;
    if map.len() != FLAGS.len() + 2 {
        return None;
    }
    let flag = |key: &str| map.get(key).and_then(Value::as_bool);
    let finite = |key: &str| {
        map.get(key)
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
    };
    Some(Request {
        editable: flag("editable")?,
        selected: flag("selected")?,
        can_undo: flag("canUndo")?,
        can_redo: flag("canRedo")?,
        quote: flag("quote")?,
        x: finite("x")?,
        y: finite("y")?,
    })
}

pub struct Entry {
    pub id: &'static str,
    pub label: String,
    pub enabled: bool,
    pub accelerator: Option<&'static str>,
}

fn words(lang: &str) -> [&'static str; 7] {
    if lang.to_ascii_lowercase().starts_with("zh") {
        ["撤销", "重做", "剪切", "复制", "粘贴", "全选", "加入对话"]
    } else {
        [
            "Undo",
            "Redo",
            "Cut",
            "Copy",
            "Paste",
            "Select All",
            "Add to conversation",
        ]
    }
}

/// What to offer, decided without a platform: the page's own account of the
/// click is the only input. An empty list means no menu at all.
pub fn template(request: &Request, lang: &str) -> Vec<Entry> {
    if !request.editable && !request.selected {
        return Vec::new();
    }
    let w = words(lang);
    let redo_keys = if cfg!(target_os = "macos") {
        "CommandOrControl+Shift+Z"
    } else {
        "CommandOrControl+Y"
    };
    let mut items = vec![
        Entry {
            id: "undo",
            label: w[0].into(),
            enabled: request.can_undo,
            accelerator: Some("CommandOrControl+Z"),
        },
        Entry {
            id: "redo",
            label: w[1].into(),
            enabled: request.can_redo,
            accelerator: Some(redo_keys),
        },
        Entry {
            id: "cut",
            label: w[2].into(),
            enabled: request.selected,
            accelerator: None,
        },
        Entry {
            id: "copy",
            label: w[3].into(),
            enabled: request.selected,
            accelerator: None,
        },
        Entry {
            id: "paste",
            label: w[4].into(),
            enabled: request.editable,
            accelerator: None,
        },
        Entry {
            id: "selectAll",
            label: w[5].into(),
            enabled: true,
            accelerator: Some("CommandOrControl+A"),
        },
    ];
    if !request.editable {
        items.retain(|item| item.id == "copy" || item.id == "selectAll");
    }
    if request.quote && request.selected {
        items.push(Entry {
            id: "quote",
            label: w[6].into(),
            enabled: true,
            accelerator: None,
        });
    }
    items
}

/// The edit roles answer with their id; the clipboard roles answer "", a
/// standard native action. Both still do the work.
fn outcome(id: &str) -> &'static str {
    match id {
        "undo" => "undo",
        "redo" => "redo",
        "selectAll" => "selectAll",
        "quote" => "quote",
        _ => "",
    }
}

fn edit_command(id: &str) -> Option<&'static str> {
    match id {
        "undo" | "redo" | "cut" | "copy" | "paste" | "selectAll" => Some(match id {
            "undo" => "undo",
            "redo" => "redo",
            "cut" => "cut",
            "copy" => "copy",
            "paste" => "paste",
            _ => "selectAll",
        }),
        _ => None,
    }
}

pub struct TextMenuState {
    pending: Mutex<Option<tauri::async_runtime::Sender<String>>>,
}

impl Default for TextMenuState {
    fn default() -> Self {
        Self {
            pending: Mutex::new(None),
        }
    }
}

#[tauri::command]
pub async fn text_context_menu(
    app: AppHandle,
    window: WebviewWindow,
    request: Value,
) -> Result<String, String> {
    let Some(request) = valid_request(&request) else {
        return Ok(String::new());
    };
    let lang = {
        let shell = app.state::<crate::commands::Shell>();
        crate::prefs::load(&shell.prefs_file)
            .get("rx-lang")
            .cloned()
            .unwrap_or_default()
    };
    let entries = template(&request, &lang);
    if entries.is_empty() {
        return Ok(String::new());
    }

    let state = app.state::<TextMenuState>();
    let (tx, mut rx) = tauri::async_runtime::channel::<String>(1);
    if let Some(previous) = state.pending.lock().expect("text menu").replace(tx.clone()) {
        let _ = previous.blocking_send(String::new());
    }

    let menu = (|| -> tauri::Result<Menu<tauri::Wry>> {
        let menu = Menu::new(&app)?;
        for (at, entry) in entries.iter().enumerate() {
            let separated = entry.id == "quote" || (request.editable && (at == 2 || at == 5));
            if separated {
                menu.append(&PredefinedMenuItem::separator(&app)?)?;
            }
            let mut builder =
                MenuItemBuilder::with_id(entry.id, &entry.label).enabled(entry.enabled);
            if let Some(keys) = entry.accelerator {
                builder = builder.accelerator(keys);
            }
            menu.append(&builder.build(&app)?)?;
        }
        Ok(menu)
    })()
    .map_err(|e| e.to_string())?;

    let acting = app.clone();
    window.on_menu_event(move |_, event| {
        let id = event.id().0.clone();
        if let Some(command) = edit_command(&id)
            && let Some(page) = acting.get_webview_window(crate::boot::MAIN)
        {
            let _ = page.eval(format!("document.execCommand(\"{command}\")"));
        }
        let _ = tx.blocking_send(outcome(&id).to_string());
    });

    let popup_window = window.as_ref().window().clone();
    let position = LogicalPosition::new(request.x, request.y);
    tauri::async_runtime::spawn_blocking(move || menu.popup_at(popup_window, position))
        .await
        .map_err(|e| e.to_string())
        .and_then(|shown| shown.map_err(|e| e.to_string()))?;

    // Windows and macOS hold the popup open inside the call above, so a
    // choice has already landed or never will; Linux returns with the menu
    // still open and no dismissal signal, so the wait is the person's.
    let wait = if cfg!(any(windows, target_os = "macos")) {
        std::time::Duration::from_millis(500)
    } else {
        std::time::Duration::from_secs(60)
    };
    let answer = tokio::time::timeout(wait, rx.recv())
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(editable: bool, selected: bool) -> Request {
        Request {
            editable,
            selected,
            can_undo: true,
            can_redo: false,
            quote: true,
            x: 0.0,
            y: 0.0,
        }
    }

    #[test]
    fn a_request_is_five_booleans_and_two_finite_numbers() {
        let good = json!({"editable":true,"selected":true,"canUndo":true,"canRedo":false,"quote":false,"x":10,"y":20.5});
        assert!(valid_request(&good).is_some());
        assert!(valid_request(&json!({"editable":1,"selected":true,"canUndo":true,"canRedo":false,"quote":false,"x":0,"y":0})).is_none());
        assert!(valid_request(&json!({"editable":true,"selected":true,"canUndo":true,"canRedo":false,"quote":false,"x":0})).is_none());
        let null_x = json!({"editable":true,"selected":true,"canUndo":true,"canRedo":false,"quote":false,"x":null,"y":0});
        assert!(valid_request(&null_x).is_none());
        assert!(valid_request(&json!([])).is_none());
    }

    #[test]
    fn nothing_to_edit_and_nothing_selected_offers_no_menu() {
        assert!(template(&request(false, false), "en").is_empty());
    }

    #[test]
    fn a_read_only_selection_offers_copy_and_select_all() {
        let items = template(&request(false, true), "en");
        let ids: Vec<&str> = items.iter().map(|i| i.id).collect();
        assert_eq!(ids, ["copy", "selectAll", "quote"]);
        assert!(items.iter().all(|i| i.enabled));
    }

    #[test]
    fn an_editable_click_offers_the_edit_roles_with_the_pages_flags() {
        let items = template(&request(true, true), "en");
        let ids: Vec<&str> = items.iter().map(|i| i.id).collect();
        assert_eq!(
            ids,
            ["undo", "redo", "cut", "copy", "paste", "selectAll", "quote"]
        );
        assert!(items[0].enabled && !items[1].enabled);
    }

    #[test]
    fn quote_needs_a_selection_and_the_ask() {
        let mut ask = request(true, false);
        ask.quote = true;
        let ids: Vec<&str> = template(&ask, "en").iter().map(|i| i.id).collect();
        assert!(!ids.contains(&"quote"));
    }

    #[test]
    fn labels_follow_the_interface_language() {
        let zh = template(&request(true, true), "zh");
        assert_eq!(zh[0].label, "撤销");
        assert_eq!(zh.last().expect("quote").label, "加入对话");
    }

    #[test]
    fn clipboard_roles_answer_empty_and_edit_roles_answer_their_id() {
        assert_eq!(outcome("copy"), "");
        assert_eq!(outcome("paste"), "");
        assert_eq!(outcome("undo"), "undo");
        assert_eq!(outcome("selectAll"), "selectAll");
        assert_eq!(outcome("quote"), "quote");
        assert_eq!(edit_command("quote"), None);
        assert_eq!(edit_command("selectAll"), Some("selectAll"));
    }
}
