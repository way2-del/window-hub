//! Install `window_hub_trayhook.dll` into explorer via `SetWindowsHookEx(WH_CALLWNDPROC)`.
//!
//! Shell_NotifyIcon delivers tray updates with `SendMessage(WM_COPYDATA)` to
//! `Shell_TrayWnd`, so `WH_CALLWNDPROC` (not `WH_GETMESSAGE`) is the correct hook.

use crate::win32::tray_hook_ipc::{
    TrayHookShared, EVENT_NAME, MAGIC, SHM_NAME, SHM_SIZE, SLOT_COUNT,
};
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use windows::core::{s, w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HMODULE, HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, UnmapViewOfFile, FILE_MAP_ALL_ACCESS, MEMORY_MAPPED_VIEW_ADDRESS,
    PAGE_READWRITE,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetWindowThreadProcessId, SetWindowsHookExW, UnhookWindowsHookEx,
    HHOOK, WH_CALLWNDPROC,
};

type HookProc =
    unsafe extern "system" fn(i32, WPARAM, LPARAM) -> windows::Win32::Foundation::LRESULT;

struct HostState {
    mapping: HANDLE,
    view: *mut TrayHookShared,
    event: HANDLE,
    hook: HHOOK,
    dll: HMODULE,
    explorer_tid: u32,
    #[allow(dead_code)]
    tray_hwnd: isize,
}

// SAFETY: host owns the mapping for process lifetime; access is behind Mutex.
unsafe impl Send for HostState {}

static HOST: Mutex<Option<HostState>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);
static DLL_PATH: OnceLock<PathBuf> = OnceLock::new();

fn wide_path(path: &std::path::Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Resolve `window_hub_trayhook.dll` next to the executable (or CARGO_MANIFEST_DIR target).
pub fn resolve_dll_path() -> Option<PathBuf> {
    if let Some(p) = DLL_PATH.get() {
        return Some(p.clone());
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("window_hub_trayhook.dll"));
            candidates.push(dir.join("resources").join("window_hub_trayhook.dll"));
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    candidates.push(manifest.join("resources").join("window_hub_trayhook.dll"));
    for profile in ["debug", "release", "release-fast"] {
        candidates.push(manifest.join("target").join(profile).join("window_hub_trayhook.dll"));
        // Preferred: trayhook artifacts live under package target/ (tauri-dev safe).
        candidates.push(
            manifest
                .join("target")
                .join("trayhook")
                .join(profile)
                .join("window_hub_trayhook.dll"),
        );
        // Legacy path (avoid using in new builds — triggers tauri watch loops).
        candidates.push(
            manifest
                .join("trayhook")
                .join("target")
                .join(profile)
                .join("window_hub_trayhook.dll"),
        );
    }
    for c in candidates {
        if c.is_file() {
            let _ = DLL_PATH.set(c.clone());
            return Some(c);
        }
    }
    None
}

fn find_explorer_tray() -> Option<(HWND, u32)> {
    unsafe {
        let mut hwnd = FindWindowW(w!("Shell_TrayWnd"), None).ok()?;
        // The spy can precede Explorer in Z-order. Keep enumerating instead of
        // treating the first matching class as the real taskbar.
        loop {
            if FindWindowExW(hwnd, HWND::default(), w!("TrayNotifyWnd"), None).is_ok() { break; }
            hwnd = FindWindowExW(HWND::default(), hwnd, w!("Shell_TrayWnd"), None).ok()?;
        }
        let mut pid = 0u32;
        let tid = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if tid == 0 {
            return None;
        }
        Some((hwnd, tid))
    }
}

/// True when explorer's tray notify tree is present (safe to install WH_CALLWNDPROC).
pub fn shell_tray_ready() -> bool {
    find_explorer_tray().is_some()
}

unsafe fn create_ipc() -> Result<(HANDLE, *mut TrayHookShared, HANDLE), String> {
    let mapping = CreateFileMappingW(
        HANDLE(usize::MAX as *mut _),
        None,
        PAGE_READWRITE,
        0,
        SHM_SIZE as u32,
        w!("Local\\WindowHubTrayHookV2"),
    )
    .map_err(|e| format!("CreateFileMappingW: {e}"))?;

    let view = MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, SHM_SIZE);
    if view.Value.is_null() {
        let _ = CloseHandle(mapping);
        return Err("MapViewOfFile returned null".into());
    }

    let shared = view.Value as *mut TrayHookShared;
    if (*shared).magic != MAGIC {
        (*shared).init();
    }

    let event =
        CreateEventW(None, false, false, w!("Local\\WindowHubTrayHookEventV2")).map_err(|e| {
            let _ = UnmapViewOfFile(view);
            let _ = CloseHandle(mapping);
            format!("CreateEventW: {e}")
        })?;

    let _ = (SHM_NAME, EVENT_NAME, MAGIC, SLOT_COUNT);
    Ok((mapping, shared, event))
}

unsafe fn install_hook(dll_path: &std::path::Path, tid: u32) -> Result<(HMODULE, HHOOK), String> {
    let wide = wide_path(dll_path);
    let dll = LoadLibraryW(PCWSTR(wide.as_ptr())).map_err(|e| format!("LoadLibraryW: {e}"))?;
    let proc = GetProcAddress(dll, s!("GetMsgProc_Tray")).ok_or_else(|| {
        "GetMsgProc_Tray export missing".to_string()
    })?;
    let hook_proc: HookProc = std::mem::transmute(proc);
    let hook = SetWindowsHookExW(WH_CALLWNDPROC, Some(hook_proc), dll, tid)
        .map_err(|e| format!("SetWindowsHookExW: {e}"))?;
    Ok((dll, hook))
}

unsafe fn teardown(state: &mut HostState) {
    if !state.hook.0.is_null() {
        let _ = UnhookWindowsHookEx(state.hook);
        state.hook = HHOOK(std::ptr::null_mut());
    }
    if !state.view.is_null() {
        let _ = UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
            Value: state.view as *mut _,
        });
        state.view = std::ptr::null_mut();
    }
    if !state.mapping.is_invalid() {
        let _ = CloseHandle(state.mapping);
        state.mapping = HANDLE::default();
    }
    if !state.event.is_invalid() {
        let _ = CloseHandle(state.event);
        state.event = HANDLE::default();
    }
    // Keep the DLL loaded for the process lifetime while any remote hook may
    // still reference it; Windows unloads on process exit.
    state.dll = HMODULE::default();
}

/// Create IPC + hook explorer. Returns Ok(true) when the hook is live.
pub fn start_host() -> Result<bool, String> {
    let dll = resolve_dll_path().ok_or_else(|| {
        "window_hub_trayhook.dll not found (build trayhook crate / copy next to exe)".to_string()
    })?;
    let (tray, tid) =
        find_explorer_tray().ok_or_else(|| "Shell_TrayWnd not found".to_string())?;

    let mut guard = HOST.lock();
    if let Some(ref mut st) = *guard {
        if st.explorer_tid == tid && !st.hook.0.is_null() {
            return Ok(true);
        }
        unsafe { teardown(st) };
        *guard = None;
    }

    unsafe {
        let (mapping, view, event) = create_ipc()?;
        let (dll_h, hook) = match install_hook(&dll, tid) {
            Ok(v) => v,
            Err(e) => {
                let _ = UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: view as *mut _,
                });
                let _ = CloseHandle(mapping);
                let _ = CloseHandle(event);
                return Err(e);
            }
        };
        *guard = Some(HostState {
            mapping,
            view,
            event,
            hook,
            dll: dll_h,
            explorer_tid: tid,
            tray_hwnd: tray.0 as isize,
        });
    }
    RUNNING.store(true, Ordering::SeqCst);
    eprintln!(
        "[tray-hook] installed WH_CALLWNDPROC on explorer tid={tid} dll={}",
        dll.display()
    );
    Ok(true)
}

#[allow(dead_code)]
pub fn stop_host() {
    RUNNING.store(false, Ordering::SeqCst);
    let mut guard = HOST.lock();
    if let Some(ref mut st) = *guard {
        unsafe { teardown(st) };
    }
    *guard = None;
}

pub fn is_running() -> bool {
    RUNNING.load(Ordering::SeqCst) && HOST.lock().as_ref().is_some_and(|s| !s.hook.0.is_null())
}

/// Re-install if explorer restarted (new Shell_TrayWnd thread).
pub fn ensure_hook() -> Result<bool, String> {
    let Some((_, tid)) = find_explorer_tray() else {
        return Ok(false);
    };
    let need = {
        let guard = HOST.lock();
        match &*guard {
            Some(st) => st.explorer_tid != tid || st.hook.0.is_null(),
            None => true,
        }
    };
    if need {
        start_host()?;
    }
    Ok(is_running())
}

/// Drain ready slots into `out`. Returns number consumed.
pub fn drain_slots(out: &mut Vec<crate::win32::tray_hook_ipc::TrayHookSlot>) -> usize {
    let guard = HOST.lock();
    let Some(st) = guard.as_ref() else {
        return 0;
    };
    if st.view.is_null() {
        return 0;
    }
    unsafe {
        let shared = &mut *st.view;
        let mut n = 0usize;
        for _ in 0..SLOT_COUNT {
            if shared.read_idx == shared.write_idx {
                break;
            }
            let idx = (shared.read_idx as usize) % SLOT_COUNT;
            let slot = &mut shared.slots[idx];
            // Acquire before reading payload.
            std::sync::atomic::fence(Ordering::Acquire);
            if slot.ready == 0 {
                break;
            }
            let seq = slot.seq;
            let copy = *slot;
            // Torn write? skip and wait for next publish.
            std::sync::atomic::fence(Ordering::Acquire);
            if slot.ready == 0 || slot.seq != seq || copy.seq != seq {
                break;
            }
            slot.ready = 0;
            shared.read_idx = shared.read_idx.wrapping_add(1);
            out.push(copy);
            n += 1;
        }
        n
    }
}

/// Wait briefly for the explorer-side event, then return.
pub fn wait_event(timeout_ms: u32) -> bool {
    let event = {
        let guard = HOST.lock();
        match guard.as_ref() {
            Some(st) if !st.event.is_invalid() => st.event,
            _ => return false,
        }
    };
    unsafe { WaitForSingleObject(event, timeout_ms) == windows::Win32::Foundation::WAIT_OBJECT_0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_install_and_refill() {
        let dll = resolve_dll_path();
        assert!(dll.is_some(), "window_hub_trayhook.dll must be built");
        start_host().expect("start_host");
        assert!(is_running());

        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{
            RegisterWindowMessageW, SendNotifyMessageW, HWND_BROADCAST,
        };
        unsafe {
            let msg = RegisterWindowMessageW(w!("TaskbarCreated"));
            assert_ne!(msg, 0);
            let _ = SendNotifyMessageW(HWND_BROADCAST, msg, None, None);
        }

        let mut total = 0usize;
        let mut buf = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            let _ = wait_event(250);
            buf.clear();
            total += drain_slots(&mut buf);
        }
        eprintln!("[tray-hook test] drained {total} slot(s)");
        // Soft assert: environment may have zero tray apps, but hook must stay live.
        assert!(is_running(), "hook should remain installed");
        assert!(
            ensure_hook().unwrap_or(false),
            "ensure_hook should keep hook alive"
        );
        stop_host();
    }
}
