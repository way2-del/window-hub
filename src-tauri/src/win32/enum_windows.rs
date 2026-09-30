use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    /// Stable runtime id: `hwnd:{hwnd}`
    pub id: String,
    pub hwnd: isize,
    pub title: String,
    pub class_name: String,
    pub pid: u32,
    /// Process image path when available
    #[serde(default)]
    pub exe: Option<String>,
    /// File stem of exe (e.g. wechatdevtools) for bind keys
    #[serde(default)]
    pub exe_name: Option<String>,
    /// AppUserModelID when available (UWP / packaged / ApplicationFrameHost).
    #[serde(default)]
    pub aumid: Option<String>,
}

fn window_id(hwnd: isize) -> String {
    format!("hwnd:{hwnd}")
}

/// Read `PKEY_AppUserModel_ID` from a top-level HWND (Settings / Security / Store apps).
#[cfg(windows)]
pub fn window_aumid(hwnd: isize) -> Option<String> {
    use windows::core::GUID;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::PropertiesSystem::{
        IPropertyStore, PROPERTYKEY, SHGetPropertyStoreForWindow,
    };

    // Same GUID as Windows SDK PKEY_AppUserModel_ID.
    const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
        pid: 5,
    };

    if hwnd == 0 {
        return None;
    }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let store: IPropertyStore =
            SHGetPropertyStoreForWindow(HWND(hwnd as *mut _)).ok()?;
        let value = store.GetValue(&PKEY_APP_USER_MODEL_ID).ok()?;
        let s = value.to_string();
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    }
}

#[cfg(not(windows))]
pub fn window_aumid(_hwnd: isize) -> Option<String> {
    None
}

#[cfg(windows)]
pub fn process_exe(pid: u32) -> (Option<String>, Option<String>) {
    use std::path::Path;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    if pid == 0 {
        return (None, None);
    }
    unsafe {
        let Ok(proc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return (None, None);
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
            return (None, None);
        }
        let path = String::from_utf16_lossy(&buf[..size as usize]);
        let name = Path::new(&path)
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string());
        (Some(path), name)
    }
}

#[cfg(windows)]
pub fn list_windows(exclude_hwnd: Option<isize>) -> Vec<WindowInfo> {
    use std::sync::Mutex;
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindow, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW,
        GetWindowThreadProcessId, IsWindowVisible, GWL_EXSTYLE, GW_OWNER, WS_EX_APPWINDOW,
        WS_EX_TOOLWINDOW,
    };

    /// Task Manager "应用" / taskbar-button windows — not tray-only background processes.
    unsafe fn is_app_window(hwnd: HWND) -> bool {
        use windows::Win32::UI::WindowsAndMessaging::IsIconic;
        // Minimized apps are not "visible" but must still show in Dock / Task View matching.
        let visible = IsWindowVisible(hwnd).as_bool();
        let iconic = IsIconic(hwnd).as_bool();
        if !visible && !iconic {
            return false;
        }

        // UWP hosts often stay "visible" while cloaked — those are not foreground apps.
        // Skip cloaked unless iconic (some shells cloak while minimized).
        if !iconic {
            let mut cloaked: u32 = 0;
            if DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                &mut cloaked as *mut _ as *mut _,
                std::mem::size_of::<u32>() as u32,
            )
            .is_ok()
                && cloaked != 0
            {
                return false;
            }
        }

        let mut class_buf = [0u16; 256];
        let cn = GetClassNameW(hwnd, &mut class_buf);
        let class_name = String::from_utf16_lossy(&class_buf[..cn as usize]);
        if matches!(
            class_name.as_str(),
            "Progman"
                | "WorkerW"
                | "Shell_TrayWnd"
                | "Shell_SecondaryTrayWnd"
                | "NotifyIconOverflowWindow"
                | "Windows.UI.Core.CoreWindow"
                | "ForegroundStaging"
                | "XamlExplorerHostIslandWindow"
                | "WindowHubAppBarHost"
                | "WindowHubDockAppBarHost"
        ) {
            return false;
        }

        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        let is_tool = ex_style & WS_EX_TOOLWINDOW.0 != 0;
        let is_app = ex_style & WS_EX_APPWINDOW.0 != 0;
        // Tool windows are tray/utility chrome unless they force a taskbar button.
        if is_tool && !is_app {
            return false;
        }
        // Owned popups (dialogs, floaters) do not get their own taskbar entry
        // unless WS_EX_APPWINDOW is set — same rule Taskbar / Alt-Tab use.
        let owner = GetWindow(hwnd, GW_OWNER).unwrap_or(HWND::default());
        if !owner.0.is_null() && !is_app {
            return false;
        }

        let title_len = GetWindowTextLengthW(hwnd);
        if title_len <= 0 {
            return false;
        }

        true
    }

    struct Ctx {
        exclude: Option<isize>,
        out: Mutex<Vec<WindowInfo>>,
    }

    let ctx = Box::new(Ctx {
        exclude: exclude_hwnd,
        out: Mutex::new(Vec::new()),
    });
    let ctx_ptr = Box::into_raw(ctx);

    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &*(lparam.0 as *const Ctx);
        if let Some(ex) = ctx.exclude {
            if hwnd.0 as isize == ex {
                return BOOL(1);
            }
        }

        if !is_app_window(hwnd) {
            return BOOL(1);
        }

        let title_len = GetWindowTextLengthW(hwnd);
        let mut title_buf = vec![0u16; (title_len + 1) as usize];
        let n = GetWindowTextW(hwnd, &mut title_buf);
        title_buf.truncate(n as usize);
        let title = String::from_utf16_lossy(&title_buf);
        let title_trim = title.trim();
        if title_trim.is_empty() {
            return BOOL(1);
        }
        // Desktop Window Manager internal HWND — not a user app (leaks into dock running).
        if title_trim.eq_ignore_ascii_case("DWM Notification Window") {
            return BOOL(1);
        }

        let mut class_buf = [0u16; 256];
        let cn = GetClassNameW(hwnd, &mut class_buf);
        let class_name = String::from_utf16_lossy(&class_buf[..cn as usize]);

        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let (exe, exe_name) = process_exe(pid);
        if exe_name
            .as_deref()
            .is_some_and(|n| n.eq_ignore_ascii_case("dwm.exe"))
        {
            return BOOL(1);
        }
        let hwnd_i = hwnd.0 as isize;
        let aumid = window_aumid(hwnd_i);

        if let Ok(mut out) = ctx.out.lock() {
            out.push(WindowInfo {
                id: window_id(hwnd_i),
                hwnd: hwnd_i,
                title,
                class_name,
                pid,
                exe,
                exe_name,
                aumid,
            });
        }
        BOOL(1)
    }

    unsafe {
        let _ = EnumWindows(Some(enum_cb), LPARAM(ctx_ptr as isize));
        let ctx = Box::from_raw(ctx_ptr);
        ctx.out.into_inner().unwrap_or_default()
    }
}

#[cfg(not(windows))]
pub fn list_windows(_exclude_hwnd: Option<isize>) -> Vec<WindowInfo> {
    Vec::new()
}

pub fn get_window(hwnd: isize, exclude_hwnd: Option<isize>) -> Option<WindowInfo> {
    list_windows(exclude_hwnd)
        .into_iter()
        .find(|w| w.hwnd == hwnd)
}

#[cfg(windows)]
pub fn focus_window(hwnd: isize) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, IsWindow, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    unsafe {
        let h = HWND(hwnd as *mut _);
        if !IsWindow(h).as_bool() {
            return Err("window no longer exists".into());
        }
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        if SetForegroundWindow(h).as_bool() {
            Ok(())
        } else {
            // Soft-fail: still restored; foreground may be blocked by OS policy
            Err("SetForegroundWindow was denied by the OS".into())
        }
    }
}

/// Call from the **RegisterHotKey message thread** right when WM_HOTKEY fires.
/// That thread briefly has foreground rights; FE/IPC activate later is often denied,
/// which feels like "hotkey only works after I hover/click the island".
#[cfg(windows)]
pub fn activate_for_hotkey(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, BringWindowToTop, GetForegroundWindow, GetWindowLongW,
        GetWindowThreadProcessId, IsIconic, IsWindow, SetForegroundWindow, SetWindowLongW,
        SetWindowPos, ShowWindow, ASFW_ANY, GWL_EXSTYLE, SWP_FRAMECHANGED, SWP_NOMOVE,
        SWP_NOSIZE, SWP_NOZORDER, SW_RESTORE, SW_SHOW, WINDOW_EX_STYLE, WS_EX_TRANSPARENT,
    };

    unsafe {
        let h = HWND(hwnd as *mut _);
        if hwnd == 0 || !IsWindow(h).as_bool() {
            return;
        }

        // Hotkey path must receive clicks/keys — clear OS click-through if set.
        let ex = WINDOW_EX_STYLE(GetWindowLongW(h, GWL_EXSTYLE) as u32);
        if ex.contains(WS_EX_TRANSPARENT) {
            let cleared = WINDOW_EX_STYLE(ex.0 & !WS_EX_TRANSPARENT.0);
            SetWindowLongW(h, GWL_EXSTYLE, cleared.0 as i32);
            let _ = SetWindowPos(
                h,
                HWND::default(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
            );
        }

        let _ = AllowSetForegroundWindow(ASFW_ANY);
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        } else {
            let _ = ShowWindow(h, SW_SHOW);
        }

        let fg = GetForegroundWindow();
        let our_tid = GetCurrentThreadId();
        let mut fg_pid = 0u32;
        let fg_tid = if !fg.is_invalid() {
            GetWindowThreadProcessId(fg, Some(&mut fg_pid))
        } else {
            0
        };
        let attached = fg_tid != 0 && fg_tid != our_tid && AttachThreadInput(our_tid, fg_tid, true).as_bool();

        let _ = BringWindowToTop(h);
        let ok = SetForegroundWindow(h).as_bool();
        if attached {
            let _ = AttachThreadInput(our_tid, fg_tid, false);
        }
        if !ok {
            // Last resort: keybd event trick is avoided (side effects); soft log only.
            eprintln!("[hotkey] SetForegroundWindow soft-fail hwnd={hwnd:#x}");
        }
    }
}

#[cfg(not(windows))]
pub fn activate_for_hotkey(_hwnd: isize) {}

/// Ask a top-level window to close (WM_CLOSE). Does not force-kill.
#[cfg(windows)]
pub fn close_window(hwnd: isize) -> Result<(), String> {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{IsWindow, PostMessageW, WM_CLOSE};

    unsafe {
        let h = HWND(hwnd as *mut _);
        if !IsWindow(h).as_bool() {
            return Err("window no longer exists".into());
        }
        PostMessageW(h, WM_CLOSE, WPARAM(0), LPARAM(0))
            .map_err(|e| format!("PostMessage WM_CLOSE: {e}"))?;
        Ok(())
    }
}

#[cfg(not(windows))]
pub fn focus_window(_hwnd: isize) -> Result<(), String> {
    Err("focus_window is only available on Windows".into())
}

#[cfg(not(windows))]
pub fn close_window(_hwnd: isize) -> Result<(), String> {
    Err("close_window is only available on Windows".into())
}

/// Stable window bind key: `exe:C:\\path\\app.exe` or `proc:appname` (matches Host FE).
#[cfg(windows)]
pub fn window_key_for_pid(pid: u32) -> Option<String> {
    let (exe, name) = process_exe(pid);
    if let Some(path) = exe {
        let norm = path.replace('/', "\\").to_lowercase();
        if !norm.is_empty() {
            return Some(format!("exe:{norm}"));
        }
    }
    if let Some(name) = name {
        let stem = name
            .trim()
            .trim_end_matches(".exe")
            .trim_end_matches(".EXE")
            .to_lowercase();
        if !stem.is_empty() {
            return Some(format!("proc:{stem}"));
        }
    }
    None
}

#[cfg(windows)]
pub fn window_key_for_hwnd(hwnd: isize) -> Option<String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    unsafe {
        let h = HWND(hwnd as *mut _);
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        window_key_for_pid(pid)
    }
}

#[cfg(not(windows))]
pub fn window_key_for_pid(_pid: u32) -> Option<String> {
    None
}

#[cfg(not(windows))]
pub fn window_key_for_hwnd(_hwnd: isize) -> Option<String> {
    None
}

/// Parse `hwnd:123` or raw numeric string into hwnd.
pub fn parse_window_id(id: &str) -> Result<isize, String> {
    let raw = id.strip_prefix("hwnd:").unwrap_or(id);
    raw.parse::<isize>()
        .map_err(|_| format!("invalid window id: {id}"))
}
