//! Native OLE file-drop onto the Dock icons HWND.
//!
//! Tauri/wry's default drag-drop handler fights HTML5 DnD and often leaves a
//! "no drop" cursor on our frameless dock. We disable it on the dock window and
//! register our own `IDropTarget`.
//!
//! Accepts both `CF_HDROP` (Explorer file paths) and Shell IDList / `IShellItem`
//! payloads (Start menu, many shortcuts, long paths).

use std::path::PathBuf;
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter};

#[cfg(windows)]
use parking_lot::Mutex;
#[cfg(windows)]
use windows::core::implement;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DockFileDragPayload {
    phase: &'static str,
    count: usize,
}

#[cfg(windows)]
#[allow(dead_code)]
struct DropTargetKeepAlive(Vec<windows::Win32::System::Ole::IDropTarget>);

#[cfg(windows)]
unsafe impl Send for DropTargetKeepAlive {}

#[cfg(windows)]
static DROP_KEEPALIVE: OnceLock<Mutex<DropTargetKeepAlive>> = OnceLock::new();

#[cfg(windows)]
static DROP_APP: OnceLock<Mutex<Option<AppHandle>>> = OnceLock::new();

/// Install after the dock webview exists. Safe to call repeatedly (rebinds).
#[cfg(windows)]
pub fn install_dock_file_drop(app: &AppHandle, hwnd_raw: isize) {
    use std::ffi::c_void;
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    use windows::Win32::System::Ole::{
        IDropTarget, OleInitialize, RegisterDragDrop, RevokeDragDrop,
    };
    use windows::Win32::UI::WindowsAndMessaging::EnumChildWindows;

    unsafe {
        let _ = OleInitialize(None);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }

    if crate::win32::app_launch::is_process_elevated() {
        eprintln!(
            "[dock] file-drop: process is elevated — Explorer OLE drops are blocked by UIPI; run without admin"
        );
    }

    *DROP_APP.get_or_init(|| Mutex::new(None)).lock() = Some(app.clone());

    let root = HWND(hwnd_raw as *mut _);
    let mut targets: Vec<IDropTarget> = Vec::new();

    let mut inject = |hwnd: HWND| {
        let target: IDropTarget = DockPinDropTarget::new(hwnd).into();
        let _ = unsafe { RevokeDragDrop(hwnd) };
        if unsafe { RegisterDragDrop(hwnd, &target) }.is_ok() {
            targets.push(target);
        }
        true
    };

    inject(root);
    {
        let mut callback = |hwnd: HWND| inject(hwnd);
        let mut trait_obj: &mut dyn FnMut(HWND) -> bool = &mut callback;
        let closure_ptr: *mut c_void = unsafe { std::mem::transmute(&mut trait_obj) };
        let lparam = LPARAM(closure_ptr as isize);
        unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let closure = &mut *(lparam.0 as *mut c_void as *mut &mut dyn FnMut(HWND) -> bool);
            closure(hwnd).into()
        }
        let _ = unsafe { EnumChildWindows(root, Some(enum_cb), lparam) };
    }

    if targets.is_empty() {
        eprintln!("[dock] file-drop: RegisterDragDrop failed on all HWNDs");
        return;
    }
    *DROP_KEEPALIVE
        .get_or_init(|| Mutex::new(DropTargetKeepAlive(Vec::new())))
        .lock() = DropTargetKeepAlive(targets);
    eprintln!(
        "[dock] file-drop: OLE target installed ({} hwnds)",
        DROP_KEEPALIVE.get().map(|m| m.lock().0.len()).unwrap_or(0)
    );
}

#[cfg(not(windows))]
pub fn install_dock_file_drop(_app: &AppHandle, _hwnd_raw: isize) {}

/// Schedule a couple of rebinds so WebView2 child HWNDs get the drop target.
#[cfg(windows)]
pub fn schedule_dock_file_drop_rebind(app: &AppHandle, hwnd_raw: isize) {
    let app = app.clone();
    std::thread::spawn(move || {
        for ms in [400u64, 1200, 2500] {
            std::thread::sleep(std::time::Duration::from_millis(ms));
            install_dock_file_drop(&app, hwnd_raw);
        }
    });
}

#[cfg(not(windows))]
pub fn schedule_dock_file_drop_rebind(_app: &AppHandle, _hwnd_raw: isize) {}

#[cfg(windows)]
fn emit_phase(phase: &'static str, count: usize) {
    let Some(app) = DROP_APP.get().and_then(|m| m.lock().clone()) else {
        return;
    };
    let _ = app.emit("dock-file-drag", DockFileDragPayload { phase, count });
}

#[cfg(windows)]
fn pin_paths(paths: Vec<PathBuf>) {
    let Some(app) = DROP_APP.get().and_then(|m| m.lock().clone()) else {
        return;
    };
    if paths.is_empty() {
        emit_phase("error", 0);
        return;
    }
    let list: Vec<String> = paths
        .into_iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let count = list.len();
    eprintln!("[dock] file-drop: pinning {count} path(s)");
    tauri::async_runtime::spawn(async move {
        match super::dock_pin_paths(app.clone(), list) {
            Ok(prefs) => {
                let _ = app.emit("dock-prefs", &prefs);
                emit_phase("drop", count);
            }
            Err(e) => {
                eprintln!("[dock] pin drop failed: {e}");
                let _ = app.emit(
                    "dock-file-drag",
                    DockFileDragPayload {
                        phase: "error",
                        count: 0,
                    },
                );
            }
        }
    });
}

#[cfg(windows)]
fn shell_idlist_format() -> u16 {
    use windows::core::PCWSTR;
    use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
    let name: Vec<u16> = "Shell IDList Array\0".encode_utf16().collect();
    unsafe { RegisterClipboardFormatW(PCWSTR(name.as_ptr())) as u16 }
}

#[cfg(windows)]
fn formatetc(cf: u16) -> windows::Win32::System::Com::FORMATETC {
    use std::ptr;
    use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
    FORMATETC {
        cfFormat: cf,
        ptd: ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

#[cfg(windows)]
fn data_object_looks_pinnable(
    data_obj: Option<&windows::Win32::System::Com::IDataObject>,
) -> bool {
    use windows::Win32::System::Ole::CF_HDROP;
    let Some(obj) = data_obj else {
        return false;
    };
    let hdrop = formatetc(CF_HDROP.0);
    if unsafe { obj.QueryGetData(&hdrop) }.is_ok() {
        return true;
    }
    let idlist = formatetc(shell_idlist_format());
    if unsafe { obj.QueryGetData(&idlist) }.is_ok() {
        return true;
    }
    // Last resort: shell can often synthesize items even when QueryGetData is quirky.
    collect_paths_from_data_object(Some(obj))
        .map(|p| !p.is_empty())
        .unwrap_or(false)
}

#[cfg(windows)]
fn paths_from_shell_items(
    data_obj: &windows::Win32::System::Com::IDataObject,
) -> Option<Vec<PathBuf>> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{
        IShellItemArray, SHCreateShellItemArrayFromDataObject, SIGDN_FILESYSPATH,
        SIGDN_DESKTOPABSOLUTEPARSING,
    };

    let array =
        unsafe { SHCreateShellItemArrayFromDataObject::<_, IShellItemArray>(data_obj) }.ok()?;
    let count = unsafe { array.GetCount() }.ok()?;
    if count == 0 {
        return None;
    }
    let mut paths = Vec::with_capacity(count as usize);
    for i in 0..count {
        let Ok(item) = (unsafe { array.GetItemAt(i) }) else {
            continue;
        };
        let mut resolved: Option<PathBuf> = None;
        for sigdn in [SIGDN_FILESYSPATH, SIGDN_DESKTOPABSOLUTEPARSING] {
            if let Ok(pw) = unsafe { item.GetDisplayName(sigdn) } {
                if !pw.is_null() {
                    let s = unsafe { pw.to_string().unwrap_or_default() };
                    unsafe { CoTaskMemFree(Some(pw.0 as _)) };
                    let t = s.trim();
                    if !t.is_empty() {
                        resolved = Some(PathBuf::from(t));
                        break;
                    }
                }
            }
        }
        if let Some(p) = resolved {
            paths.push(p);
        }
    }
    if paths.is_empty() {
        None
    } else {
        Some(paths)
    }
}

#[cfg(windows)]
fn paths_from_hdrop(
    data_obj: &windows::Win32::System::Com::IDataObject,
) -> Option<Vec<PathBuf>> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::System::Ole::{ReleaseStgMedium, CF_HDROP};
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

    let fmt = formatetc(CF_HDROP.0);
    let mut medium = unsafe { data_obj.GetData(&fmt) }.ok()?;
    let paths = unsafe {
        let hdrop = HDROP(medium.u.hGlobal.0 as _);
        let item_count = DragQueryFileW(hdrop, 0xFFFFFFFF, None);
        let mut out = Vec::with_capacity(item_count as usize);
        for i in 0..item_count {
            let character_count = DragQueryFileW(hdrop, i, None) as usize;
            let mut path_buf = vec![0u16; character_count + 1];
            DragQueryFileW(hdrop, i, Some(&mut path_buf));
            out.push(OsString::from_wide(&path_buf[0..character_count]).into());
        }
        let _ = ReleaseStgMedium(&mut medium);
        out
    };
    if paths.is_empty() {
        None
    } else {
        Some(paths)
    }
}

#[cfg(windows)]
fn collect_paths_from_data_object(
    data_obj: Option<&windows::Win32::System::Com::IDataObject>,
) -> Option<Vec<PathBuf>> {
    let obj = data_obj?;
    if let Some(paths) = paths_from_shell_items(obj) {
        return Some(paths);
    }
    paths_from_hdrop(obj)
}

#[cfg(windows)]
#[implement(windows::Win32::System::Ole::IDropTarget)]
struct DockPinDropTarget {
    hwnd: windows::Win32::Foundation::HWND,
}

#[cfg(windows)]
impl DockPinDropTarget {
    fn new(hwnd: windows::Win32::Foundation::HWND) -> Self {
        Self { hwnd }
    }
}

#[cfg(windows)]
#[allow(non_snake_case)]
impl windows::Win32::System::Ole::IDropTarget_Impl for DockPinDropTarget_Impl {
    fn DragEnter(
        &self,
        pdataobj: Option<&windows::Win32::System::Com::IDataObject>,
        _grfkeystate: windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS,
        pt: &windows::Win32::Foundation::POINTL,
        pdweffect: *mut windows::Win32::System::Ole::DROPEFFECT,
    ) -> windows_core::Result<()> {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::Graphics::Gdi::ScreenToClient;
        use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_NONE};

        let mut cpt = POINT { x: pt.x, y: pt.y };
        let _ = unsafe { ScreenToClient(self.hwnd, &mut cpt) };
        let valid = data_object_looks_pinnable(pdataobj);
        unsafe {
            *pdweffect = if valid {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT_NONE
            };
        }
        if valid {
            let count = collect_paths_from_data_object(pdataobj)
                .map(|p| p.len())
                .unwrap_or(1);
            emit_phase("enter", count);
        }
        Ok(())
    }

    fn DragOver(
        &self,
        _grfkeystate: windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS,
        _pt: &windows::Win32::Foundation::POINTL,
        pdweffect: *mut windows::Win32::System::Ole::DROPEFFECT,
    ) -> windows_core::Result<()> {
        use windows::Win32::System::Ole::DROPEFFECT_COPY;
        unsafe {
            *pdweffect = DROPEFFECT_COPY;
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows_core::Result<()> {
        emit_phase("leave", 0);
        Ok(())
    }

    fn Drop(
        &self,
        pdataobj: Option<&windows::Win32::System::Com::IDataObject>,
        _grfkeystate: windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS,
        _pt: &windows::Win32::Foundation::POINTL,
        pdweffect: *mut windows::Win32::System::Ole::DROPEFFECT,
    ) -> windows_core::Result<()> {
        use windows::Win32::System::Ole::DROPEFFECT_COPY;

        unsafe {
            *pdweffect = DROPEFFECT_COPY;
        }
        if let Some(paths) = collect_paths_from_data_object(pdataobj) {
            pin_paths(paths);
        } else {
            eprintln!("[dock] file-drop: Drop with no resolvable paths");
            emit_phase("error", 0);
        }
        Ok(())
    }
}
