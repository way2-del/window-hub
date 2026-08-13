//! Native OLE file-drop onto the Dock icons HWND.
//!
//! Tauri/wry's default drag-drop handler fights HTML5 DnD and often leaves a
//! "no drop" cursor on our frameless dock. We disable it on the dock window and
//! register our own `IDropTarget` that always accepts `CF_HDROP`.

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
    eprintln!("[dock] file-drop: OLE target installed");
}

#[cfg(not(windows))]
pub fn install_dock_file_drop(_app: &AppHandle, _hwnd_raw: isize) {}

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
        return;
    }
    let list: Vec<String> = paths
        .into_iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let count = list.len();
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
fn collect_hdrop_paths(
    data_obj: Option<&windows::Win32::System::Com::IDataObject>,
) -> Option<(Vec<PathBuf>, windows::Win32::UI::Shell::HDROP)> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::ptr;
    use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
    use windows::Win32::System::Ole::CF_HDROP;
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

    let obj = data_obj?;
    let drop_format = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    unsafe {
        let medium = obj.GetData(&drop_format).ok()?;
        let hdrop = HDROP(medium.u.hGlobal.0 as _);
        let item_count = DragQueryFileW(hdrop, 0xFFFFFFFF, None);
        let mut paths = Vec::with_capacity(item_count as usize);
        for i in 0..item_count {
            let character_count = DragQueryFileW(hdrop, i, None) as usize;
            let mut path_buf = vec![0u16; character_count + 1];
            DragQueryFileW(hdrop, i, Some(&mut path_buf));
            paths.push(OsString::from_wide(&path_buf[0..character_count]).into());
        }
        Some((paths, hdrop))
    }
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
        let paths = collect_hdrop_paths(pdataobj).map(|(p, _)| p);
        let valid = paths.as_ref().map(|p| !p.is_empty()).unwrap_or(false);
        unsafe {
            *pdweffect = if valid {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT_NONE
            };
        }
        if valid {
            emit_phase("enter", paths.map(|p| p.len()).unwrap_or(0));
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
        use windows::Win32::UI::Shell::DragFinish;

        unsafe {
            *pdweffect = DROPEFFECT_COPY;
        }
        if let Some((paths, hdrop)) = collect_hdrop_paths(pdataobj) {
            pin_paths(paths);
            unsafe {
                DragFinish(hdrop);
            }
        } else {
            emit_phase("leave", 0);
        }
        Ok(())
    }
}
