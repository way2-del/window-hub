//! Status-menu shell actions: foreground label, taskbar, show-desktop.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForegroundApp {
    pub title: String,
    pub exe_name: Option<String>,
    /// Short label for the status chip (exe stem or title).
    pub label: String,
    /// Foreground is this process — UI should keep the previous chip label.
    pub is_self: bool,
    /// Raw HWND (0 if unknown / desktop / self).
    pub hwnd: isize,
    /// Stable id `hwnd:{hwnd}` for matching shortcut pins.
    pub window_id: Option<String>,
}

#[cfg(windows)]
mod win {
    use super::ForegroundApp;
    use windows::core::{s, GUID};
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Shell::{
        IShellDispatch4, SHAppBarMessage, ABS_AUTOHIDE, ABM_GETSTATE, ABM_SETSTATE, APPBARDATA,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowA, FindWindowExA, GetAncestor, GetClassNameW, GetForegroundWindow,
        GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, ShowWindow, GA_ROOT,
        SW_HIDE, SW_SHOWNA,
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    /// Shell.Application
    const CLSID_SHELL: GUID = GUID::from_u128(0x13709620_C279_11CE_A49E_444553540000);

    fn class_name(hwnd: HWND) -> String {
        unsafe {
            let mut buf = [0u16; 256];
            let n = GetClassNameW(hwnd, &mut buf);
            if n <= 0 {
                return String::new();
            }
            String::from_utf16_lossy(&buf[..n as usize])
        }
    }

    fn window_title(hwnd: HWND) -> String {
        unsafe {
            let len = GetWindowTextLengthW(hwnd);
            if len <= 0 {
                return String::new();
            }
            let mut buf = vec![0u16; (len + 1) as usize];
            let n = GetWindowTextW(hwnd, &mut buf);
            if n <= 0 {
                return String::new();
            }
            String::from_utf16_lossy(&buf[..n as usize])
        }
    }

    fn process_exe_stem(pid: u32) -> Option<String> {
        if pid == 0 {
            return None;
        }
        unsafe {
            let Ok(proc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return None;
            };
            let mut buf = [0u16; 520];
            let mut size = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(
                proc,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(buf.as_mut_ptr()),
                &mut size,
            );
            let _ = CloseHandle(proc);
            if ok.is_err() || size == 0 {
                return None;
            }
            let path = String::from_utf16_lossy(&buf[..size as usize]);
            std::path::Path::new(&path)
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        }
    }

    fn is_desktop(hwnd: HWND) -> bool {
        matches!(class_name(hwnd).as_str(), "Progman" | "WorkerW")
    }

    fn make_label(title: &str, exe: Option<&str>) -> String {
        if let Some(stem) = exe {
            let pretty = stem.trim();
            if !pretty.is_empty()
                && !pretty.eq_ignore_ascii_case("explorer")
                && !pretty.eq_ignore_ascii_case("ApplicationFrameHost")
            {
                // Cursor.exe → Cursor; keep mixed case from stem
                let mut chars = pretty.chars();
                if let Some(c0) = chars.next() {
                    return format!("{}{}", c0.to_uppercase(), chars.as_str());
                }
            }
        }
        let t = title.trim();
        if t.is_empty() {
            return "桌面".into();
        }
        // "App — doc" / "App - doc" → App
        for sep in [" — ", " – ", " - ", " | "] {
            if let Some((head, _)) = t.split_once(sep) {
                let h = head.trim();
                if !h.is_empty() && h.chars().count() <= 24 {
                    return h.to_string();
                }
            }
        }
        t.chars().take(24).collect()
    }

    pub fn foreground_app(self_hwnd: Option<isize>) -> ForegroundApp {
        fn empty(is_self: bool, label: &str) -> ForegroundApp {
            ForegroundApp {
                title: String::new(),
                exe_name: None,
                label: label.into(),
                is_self,
                hwnd: 0,
                window_id: None,
            }
        }

        unsafe {
            let raw = GetForegroundWindow();
            if raw.0.is_null() {
                return empty(false, "桌面");
            }
            let fg = GetAncestor(raw, GA_ROOT);
            let fg = if fg.0.is_null() { raw } else { fg };
            let hwnd = fg.0 as isize;
            let window_id = Some(format!("hwnd:{hwnd}"));
            if let Some(me) = self_hwnd {
                if hwnd == me {
                    return ForegroundApp {
                        title: String::new(),
                        exe_name: None,
                        label: String::new(),
                        is_self: true,
                        hwnd,
                        window_id,
                    };
                }
            }
            if is_desktop(fg) {
                return empty(false, "桌面");
            }
            let cls = class_name(fg);
            if matches!(
                cls.as_str(),
                "Shell_TrayWnd"
                    | "Shell_SecondaryTrayWnd"
                    | "Windows.UI.Core.CoreWindow"
                    | "XamlExplorerHostIslandWindow"
            ) {
                return ForegroundApp {
                    title: String::new(),
                    exe_name: None,
                    label: String::new(),
                    is_self: true,
                    hwnd,
                    window_id,
                };
            }
            let title = window_title(fg);
            let mut pid = 0u32;
            GetWindowThreadProcessId(fg, Some(&mut pid));
            let exe = process_exe_stem(pid);
            // Same process (settings / tray popup) — keep previous chip
            if let Some(me) = self_hwnd {
                let mut my_pid = 0u32;
                GetWindowThreadProcessId(HWND(me as *mut _), Some(&mut my_pid));
                if my_pid != 0 && pid == my_pid {
                    return ForegroundApp {
                        title,
                        exe_name: exe,
                        label: String::new(),
                        is_self: true,
                        hwnd,
                        window_id,
                    };
                }
            }
            let label = make_label(&title, exe.as_deref());
            ForegroundApp {
                title,
                exe_name: exe,
                label,
                is_self: false,
                hwnd,
                window_id,
            }
        }
    }

    fn shell_tray_hwnd() -> Option<HWND> {
        unsafe {
            let mut hwnd = FindWindowA(s!("Shell_TrayWnd"), None).ok()?;
            loop {
                if FindWindowExA(hwnd, None, s!("TrayNotifyWnd"), None).is_ok() {
                    return Some(hwnd);
                }
                hwnd = FindWindowExA(HWND::default(), hwnd, s!("Shell_TrayWnd"), None).ok()?;
            }
        }
    }

    fn for_each_taskbar(mut f: impl FnMut(HWND)) {
        if let Some(primary) = shell_tray_hwnd() {
            f(primary);
        }
        unsafe {
            let mut hwnd = FindWindowA(s!("Shell_SecondaryTrayWnd"), None).unwrap_or_default();
            while !hwnd.0.is_null() {
                f(hwnd);
                hwnd =
                    FindWindowExA(HWND::default(), hwnd, s!("Shell_SecondaryTrayWnd"), None)
                        .unwrap_or_default();
            }
        }
    }

    /// Remembers shell auto-hide state so we can restore it.
    struct TaskbarOverride {
        prev_abm: u32,
    }

    static TASKBAR_OVERRIDE: Mutex<Option<TaskbarOverride>> = Mutex::new(None);
    static TASKBAR_KEEP_HIDDEN: AtomicBool = AtomicBool::new(false);

    fn appbar_get_state() -> u32 {
        let mut data = APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            ..Default::default()
        };
        unsafe { SHAppBarMessage(ABM_GETSTATE, &mut data) as u32 }
    }

    fn appbar_set_state(tray: HWND, state: u32) {
        let mut data = APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            hWnd: tray,
            lParam: LPARAM(state as isize),
            ..Default::default()
        };
        unsafe {
            let _ = SHAppBarMessage(ABM_SETSTATE, &mut data);
        }
    }

    fn hide_taskbars_once() {
        use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
        for_each_taskbar(|hwnd| unsafe {
            // Only hide when visible — avoids needless Show/Hide churn.
            if IsWindowVisible(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        });
    }

    fn start_keep_hidden() {
        if TASKBAR_KEEP_HIDDEN
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        std::thread::spawn(|| {
            while TASKBAR_KEEP_HIDDEN.load(Ordering::SeqCst) {
                hide_taskbars_once();
                // Re-assert auto-hide so Explorer keeps the work-area gap gone.
                if let Some(primary) = shell_tray_hwnd() {
                    let st = appbar_get_state();
                    if st & ABS_AUTOHIDE == 0 {
                        appbar_set_state(primary, st | ABS_AUTOHIDE);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
        });
    }

    fn stop_keep_hidden() {
        TASKBAR_KEEP_HIDDEN.store(false, Ordering::SeqCst);
    }

    /// Hide system taskbar while Dock owns the bottom edge.
    /// Auto-hide reclaims work area; a light keep-hidden loop only calls
    /// `SW_HIDE` when Explorer re-shows the bar (no SetWindowPos exile).
    pub fn set_taskbar_visible(visible: bool) -> Result<(), String> {
        if !visible && crate::lifecycle::stopping() { return Ok(()); }
        let primary = shell_tray_hwnd().ok_or_else(|| "找不到系统任务栏".to_string())?;

        if visible {
            stop_keep_hidden();
            std::thread::sleep(std::time::Duration::from_millis(50));
            if let Ok(mut g) = TASKBAR_OVERRIDE.lock() {
                if let Some(prev) = g.take() {
                    appbar_set_state(primary, prev.prev_abm);
                }
            }
            for_each_taskbar(|hwnd| unsafe {
                let _ = ShowWindow(hwnd, SW_SHOWNA);
            });
            return Ok(());
        }

        let before = appbar_get_state();
        let already = TASKBAR_OVERRIDE
            .lock()
            .ok()
            .map(|g| g.is_some())
            .unwrap_or(false);
        if !already {
            if before & ABS_AUTOHIDE == 0 {
                appbar_set_state(primary, before | ABS_AUTOHIDE);
                std::thread::sleep(std::time::Duration::from_millis(80));
            }
            if let Ok(mut g) = TASKBAR_OVERRIDE.lock() {
                *g = Some(TaskbarOverride { prev_abm: before });
            }
        } else if before & ABS_AUTOHIDE == 0 {
            appbar_set_state(primary, before | ABS_AUTOHIDE);
        }

        hide_taskbars_once();
        start_keep_hidden();
        Ok(())
    }

    pub fn show_desktop() -> Result<(), String> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|e| e.to_string())?;
        }
        struct Guard;
        impl Drop for Guard {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _guard = Guard;

        unsafe {
            let shell: IShellDispatch4 = CoCreateInstance(&CLSID_SHELL, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| format!("Shell.Application: {e}"))?;
            shell.ToggleDesktop().map_err(|e| format!("ToggleDesktop: {e}"))?;
        }
        Ok(())
    }
}

#[cfg(windows)]
pub use win::{foreground_app, set_taskbar_visible, show_desktop};

#[cfg(not(windows))]
pub fn foreground_app(_self_hwnd: Option<isize>) -> ForegroundApp {
    ForegroundApp {
        title: String::new(),
        exe_name: None,
        label: "Desktop".into(),
        is_self: false,
        hwnd: 0,
        window_id: None,
    }
}

#[cfg(not(windows))]
pub fn set_taskbar_visible(_visible: bool) -> Result<(), String> {
    Err("Windows only".into())
}

#[cfg(not(windows))]
pub fn show_desktop() -> Result<(), String> {
    Err("Windows only".into())
}
