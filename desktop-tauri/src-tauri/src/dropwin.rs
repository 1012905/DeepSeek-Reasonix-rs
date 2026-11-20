// Windows drop target that serves file paths to the shell and text to the page.
// wry's own handler answers DROPEFFECT_NONE for anything but files, which kills
// the page's internal HTML5 drags; this one reads both kinds off the same
// IDataObject and emits the events each consumer already listens for.
use std::cell::UnsafeCell;
use std::collections::HashSet;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use windows::Win32::Foundation::{DRAGDROP_E_INVALIDHWND, HWND, LPARAM, POINT, POINTL};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::{
    CoTaskMemFree, DATADIR_GET, DVASPECT_CONTENT, FORMATETC, IDataObject, TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::GetClipboardFormatNameW;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{
    CF_HDROP, CF_UNICODETEXT, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, IDropTarget,
    IDropTarget_Impl, RegisterDragDrop, RevokeDragDrop,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{
    DragFinish, DragQueryFileW, HDROP, IShellItemArray, SHCreateShellItemArrayFromDataObject,
    SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{EnumChildWindows, IsWindow};
use windows::core::{BOOL, implement};

const MAX_FORMATS: usize = 32;
const MAX_TEXT: usize = 4 * 1024 * 1024;
const REHOOK: std::time::Duration = std::time::Duration::from_secs(2);

pub fn install(app: &AppHandle, window: &WebviewWindow) {
    let hooked_app = app.clone();
    let hooked_window = window.clone();
    let _ = window.with_webview(move |_| {
        // Runs on the main thread once the controller exists, which is when
        // WebView2's child HWNDs are there to take over.
        if let Ok(hwnd) = hooked_window.hwnd() {
            hook_children(hwnd, &hooked_app);
        }
    });
    let watch_app = app.clone();
    let watch_window = window.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(REHOOK);
            if watch_app.get_webview_window(crate::boot::MAIN).is_none() {
                return;
            }
            let app = watch_app.clone();
            let window = watch_window.clone();
            let inner = window.clone();
            // WebView2 recreates its render-widget HWND across navigations;
            // a drag lands on whichever child exists now.
            let _ = window.run_on_main_thread(move || {
                if let Ok(hwnd) = inner.hwnd() {
                    hook_children(hwnd, &app);
                }
            });
        }
    });
}

// The registered interfaces are STA objects: they may only be touched on the
// thread that registered them, which is where every access below happens.
thread_local! {
    static HOOKED: std::cell::RefCell<HashSet<isize>> = std::cell::RefCell::new(HashSet::new());
    static TARGETS: std::cell::RefCell<Vec<IDropTarget>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn hook_children(parent: HWND, app: &AppHandle) {
    let mut callback = |hwnd: HWND| {
        let key = hwnd.0 as isize;
        // SAFETY: IsWindow answers for a handle this enumeration just produced
        // on the thread that owns it; a destroyed child is simply skipped.
        if unsafe { IsWindow(Some(hwnd)) }.as_bool() && HOOKED.with(|h| h.borrow().contains(&key)) {
            return true;
        }
        let target: IDropTarget = DropTarget::new(hwnd, app.clone()).into();
        // SAFETY: both calls take a live HWND owned by this STA thread; the
        // interface is kept alive in TARGETS for as long as it is registered.
        let revoked = unsafe { RevokeDragDrop(hwnd) };
        if revoked != Err(DRAGDROP_E_INVALIDHWND.into())
            && unsafe { RegisterDragDrop(hwnd, &target) }.is_ok()
        {
            HOOKED.with(|h| h.borrow_mut().insert(key));
            TARGETS.with(|t| t.borrow_mut().push(target));
        }
        true
    };
    let mut trait_obj: &mut dyn FnMut(HWND) -> bool = &mut callback;
    // SAFETY: the fat pointer is round-tripped through LPARAM unchanged and
    // used only inside this call, while `callback` is alive on this stack.
    let closure_pointer_pointer: *mut std::ffi::c_void =
        unsafe { std::mem::transmute(&mut trait_obj) };
    let lparam = LPARAM(closure_pointer_pointer as _);
    unsafe extern "system" fn enumerate(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam carries the fat pointer stored by the caller above,
        // valid for the duration of this enumeration.
        unsafe {
            let closure =
                &mut *(lparam.0 as *mut std::ffi::c_void as *mut &mut dyn FnMut(HWND) -> bool);
            closure(hwnd).into()
        }
    }
    // SAFETY: parent is the shell window's live HWND on its owning thread.
    let _ = unsafe { EnumChildWindows(Some(parent), Some(enumerate), lparam) };
    // SAFETY: each stored key came from this thread's enumeration; IsWindow
    // only reads the handle's liveness.
    HOOKED.with(|h| {
        h.borrow_mut()
            .retain(|key| unsafe { IsWindow(Some(HWND(*key as _))) }.as_bool())
    });
}

enum Mode {
    Nothing,
    Files,
    Data(Vec<(u16, String)>),
}

#[implement(IDropTarget)]
struct DropTarget {
    hwnd: HWND,
    app: AppHandle,
    mode: UnsafeCell<Mode>,
    effect: UnsafeCell<DROPEFFECT>,
}

impl DropTarget {
    fn new(hwnd: HWND, app: AppHandle) -> Self {
        Self {
            hwnd,
            app,
            mode: UnsafeCell::new(Mode::Nothing),
            effect: UnsafeCell::new(DROPEFFECT_NONE),
        }
    }

    fn point(&self, pt: &POINTL) -> POINT {
        let mut point = POINT { x: pt.x, y: pt.y };
        // SAFETY: hwnd is the child this target was registered on, alive for
        // the duration of the drag that called in.
        let _ = unsafe { ScreenToClient(self.hwnd, &mut point) };
        point
    }

    fn emit(&self, event: &str, payload: serde_json::Value) {
        let _ = self.app.emit_to(crate::boot::MAIN, event, payload);
    }
}

fn position(point: POINT) -> serde_json::Value {
    serde_json::json!({ "x": point.x, "y": point.y })
}

#[allow(non_snake_case)]
impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(
        &self,
        data: windows::core::Ref<'_, IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        // SAFETY: the cells are written only here and in the other drag
        // callbacks, which OLE serializes on this STA thread.
        unsafe {
            *self.mode.get() = Mode::Nothing;
            *self.effect.get() = DROPEFFECT_NONE;
        }
        let point = self.point(pt);
        let Some(data) = data.as_ref() else {
            // SAFETY: effect is the out-pointer OLE guarantees for this call.
            unsafe { *effect = DROPEFFECT_NONE };
            return Ok(());
        };
        if let Some(paths) = file_paths(data) {
            // SAFETY: as above, single-threaded OLE callbacks.
            unsafe { *self.mode.get() = Mode::Files };
            self.emit(
                "tauri://drag-enter",
                serde_json::json!({ "paths": paths, "position": position(point) }),
            );
        } else {
            let formats = text_formats(data);
            if formats.is_empty() {
                // SAFETY: as above.
                unsafe { *effect = DROPEFFECT_NONE };
                return Ok(());
            }
            let types: Vec<String> = formats.iter().map(|(_, name)| name.clone()).collect();
            // SAFETY: as above.
            unsafe { *self.mode.get() = Mode::Data(formats) };
            self.emit(
                "reasonix://drag-enter",
                serde_json::json!({ "types": types, "position": position(point) }),
            );
        }
        // SAFETY: as above.
        unsafe {
            *self.effect.get() = DROPEFFECT_COPY;
            *effect = DROPEFFECT_COPY;
        }
        Ok(())
    }

    fn DragOver(
        &self,
        _keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let point = self.point(pt);
        // SAFETY: mode and effect are only touched by OLE's serialized drag
        // callbacks on this thread.
        unsafe {
            match &*self.mode.get() {
                Mode::Files => self.emit(
                    "tauri://drag-over",
                    serde_json::json!({ "position": position(point) }),
                ),
                Mode::Data(_) => self.emit(
                    "reasonix://drag-over",
                    serde_json::json!({ "position": position(point) }),
                ),
                Mode::Nothing => {}
            }
            *effect = *self.effect.get();
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        // SAFETY: as above.
        match unsafe { &*self.mode.get() } {
            Mode::Files => self.emit("tauri://drag-leave", serde_json::Value::Null),
            Mode::Data(_) => self.emit("reasonix://drag-leave", serde_json::Value::Null),
            Mode::Nothing => {}
        }
        Ok(())
    }

    fn Drop(
        &self,
        data: windows::core::Ref<'_, IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        _effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let point = self.point(pt);
        let Some(data) = data.as_ref() else {
            return Ok(());
        };
        // SAFETY: as above.
        let mode = unsafe { std::mem::replace(&mut *self.mode.get(), Mode::Nothing) };
        match mode {
            Mode::Files => {
                let paths = file_paths(data).unwrap_or_default();
                self.emit(
                    "tauri://drag-drop",
                    serde_json::json!({ "paths": paths, "position": position(point) }),
                );
                release_hdrop(data);
            }
            Mode::Data(formats) => {
                let entries: Vec<(String, String)> = formats
                    .iter()
                    .filter_map(|(format, name)| {
                        read_text(data, *format).map(|text| (name.clone(), text))
                    })
                    .collect();
                self.emit(
                    "reasonix://drag-drop",
                    serde_json::json!({ "entries": entries, "position": position(point) }),
                );
            }
            Mode::Nothing => {}
        }
        Ok(())
    }
}

/// The drag's file paths, or None when the data holds none. CF_HDROP first;
/// Explorer cannot build it for paths at or past MAX_PATH, and the shell item
/// array has no such limit.
fn file_paths(data: &IDataObject) -> Option<Vec<String>> {
    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    // SAFETY: data is the live interface OLE passed for this callback; the
    // medium it answers with is freed by release_hdrop after the drop.
    let medium = unsafe { data.GetData(&format) };
    if let Ok(medium) = medium {
        // SAFETY: the union holds the HGLOBAL GetData answered with for
        // TYMED_HGLOBAL; the queries read only what that handle holds.
        unsafe {
            let hdrop = HDROP(medium.u.hGlobal.0 as _);
            let mut paths = Vec::new();
            let count = DragQueryFileW(hdrop, u32::MAX, None);
            for i in 0..count {
                let len = DragQueryFileW(hdrop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                DragQueryFileW(hdrop, i, Some(&mut buf));
                paths.push(
                    OsString::from_wide(&buf[..len])
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            return Some(paths);
        }
    }
    shell_paths(data)
}

fn shell_paths(data: &IDataObject) -> Option<Vec<String>> {
    // SAFETY: data is live for this callback; the array and each display name
    // are released below, the name memory with CoTaskMemFree as the shell asks.
    unsafe {
        let items = SHCreateShellItemArrayFromDataObject::<_, IShellItemArray>(data).ok()?;
        let count = items.GetCount().unwrap_or(0);
        let mut paths = Vec::new();
        for i in 0..count {
            let Ok(item) = items.GetItemAt(i) else {
                continue;
            };
            let Ok(name) = item.GetDisplayName(SIGDN_FILESYSPATH) else {
                continue;
            };
            paths.push(
                OsString::from_wide(name.as_wide())
                    .to_string_lossy()
                    .into_owned(),
            );
            CoTaskMemFree(Some(name.0 as _));
        }
        if paths.is_empty() { None } else { Some(paths) }
    }
}

fn release_hdrop(data: &IDataObject) {
    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    // SAFETY: a second GetData for the same drag yields the same HDROP;
    // DragFinish is the release the shell pairs with DragQueryFileW.
    if let Ok(medium) = unsafe { data.GetData(&format) } {
        unsafe { DragFinish(HDROP(medium.u.hGlobal.0 as _)) };
    }
}

/// MIME-looking registered formats plus plain text, in the order the data
/// object offers them. Chromium registers a format per HTML5 drag MIME type,
/// which is how the page's internal path drags arrive here.
fn text_formats(data: &IDataObject) -> Vec<(u16, String)> {
    let mut out: Vec<(u16, String)> = Vec::new();
    // SAFETY: data is live for this callback; the enumerator it answers with
    // is consumed here and dropped.
    let Ok(enumerator) = (unsafe { data.EnumFormatEtc(DATADIR_GET.0 as u32) }) else {
        return out;
    };
    loop {
        let mut formats = [FORMATETC::default(); 1];
        let mut fetched = 0u32;
        // SAFETY: the array and count outlive the call; Next fills at most one.
        let hr = unsafe { enumerator.Next(&mut formats, Some(&mut fetched)) };
        if hr.is_err() || fetched == 0 {
            break;
        }
        let cf = formats[0].cfFormat;
        if cf == CF_UNICODETEXT.0 {
            out.push((cf, "text/plain".into()));
        } else if let Some(name) = format_name(cf)
            && name.contains('/')
        {
            out.push((cf, name));
        }
        if out.len() >= MAX_FORMATS {
            break;
        }
    }
    out
}

fn format_name(cf: u16) -> Option<String> {
    let mut buf = [0u16; 256];
    // SAFETY: buf is a live writable slice of the length passed; the return
    // is the number of characters written, excluding the null.
    let len = unsafe { GetClipboardFormatNameW(cf as u32, &mut buf) };
    if len <= 0 {
        return None;
    }
    Some(
        OsString::from_wide(&buf[..len as usize])
            .to_string_lossy()
            .into_owned(),
    )
}

fn read_text(data: &IDataObject, cf: u16) -> Option<String> {
    let format = FORMATETC {
        cfFormat: cf,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    // SAFETY: data is live for this callback; the medium's global is locked
    // for the read and unlocked before return, and released by the medium's
    // own Drop when it goes out of scope.
    unsafe {
        let medium = data.GetData(&format).ok()?;
        let handle = medium.u.hGlobal;
        let size = GlobalSize(handle).min(MAX_TEXT);
        if size == 0 {
            return None;
        }
        let ptr = GlobalLock(handle) as *const u8;
        if ptr.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(ptr, size);
        let text = decode(bytes);
        let _ = GlobalUnlock(handle);
        text
    }
}

/// Custom drag formats carry UTF-8 text; the standard text format carries
/// UTF-16. A trailing null belongs to neither.
fn decode(bytes: &[u8]) -> Option<String> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Some(text.trim_end_matches('\0').to_string());
    }
    if bytes.len().is_multiple_of(2) {
        let wide: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        let text = OsString::from_wide(&wide).to_string_lossy().into_owned();
        return Some(text.trim_end_matches('\0').to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_text_decodes_without_its_null() {
        assert_eq!(decode(b"/w/a.txt\0").as_deref(), Some("/w/a.txt"));
    }

    #[test]
    fn utf16_text_decodes_when_utf8_cannot() {
        let wide: Vec<u16> = "C:\\工作\\a.txt\0".encode_utf16().collect();
        let bytes: Vec<u8> = wide.iter().flat_map(|c| c.to_le_bytes()).collect();
        assert_eq!(decode(&bytes).as_deref(), Some("C:\\工作\\a.txt"));
    }

    #[test]
    fn odd_bytes_are_not_text() {
        assert_eq!(decode(&[0xff, 0xfe, 0x00]), None);
    }

    #[test]
    fn positions_serialize_as_the_page_reads_them() {
        let value = position(POINT { x: 12, y: 34 });
        assert_eq!(value["x"], 12);
        assert_eq!(value["y"], 34);
    }
}
