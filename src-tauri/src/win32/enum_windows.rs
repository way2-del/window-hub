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
}

fn window_id(hwnd: isize) -> String {
    format!("hwnd:{hwnd}")
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

        if let Ok(mut out) = ctx.out.lock() {
            out.push(WindowInfo {
                id: window_id(hwnd_i),
                hwnd: hwnd_i,
                title,
                class_name,
                pid,
                exe,
                exe_name,
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

#[cfg(not(windows))]
pub fn focus_window(_hwnd: isize) -> Result<(), String> {
    Err("focus_window is only available on Windows".into())
}

/// Parse `hwnd:123` or raw numeric string into hwnd.
pub fn parse_window_id(id: &str) -> Result<isize, String> {
    let raw = id.strip_prefix("hwnd:").unwrap_or(id);
    raw.parse::<isize>()
        .map_err(|_| format!("invalid window id: {id}"))
}
