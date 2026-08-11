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
fn process_exe(pid: u32) -> (Option<String>, Option<String>) {
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
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW,
        GetWindowThreadProcessId, IsWindowVisible, GWL_EXSTYLE, WS_EX_TOOLWINDOW,
    };

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

        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }

        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
            return BOOL(1);
        }

        let title_len = GetWindowTextLengthW(hwnd);
        if title_len == 0 {
            return BOOL(1);
        }

        let mut title_buf = vec![0u16; (title_len + 1) as usize];
        let n = GetWindowTextW(hwnd, &mut title_buf);
        title_buf.truncate(n as usize);
        let title = String::from_utf16_lossy(&title_buf);
        if title.trim().is_empty() {
            return BOOL(1);
        }

        let mut class_buf = [0u16; 256];
        let cn = GetClassNameW(hwnd, &mut class_buf);
        let class_name = String::from_utf16_lossy(&class_buf[..cn as usize]);

        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let (exe, exe_name) = process_exe(pid);
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

/// Restore / focus the largest visible top-level window owned by `pid`
/// (used after left-clicking a tray icon for apps that don't activate themselves).
#[cfg(windows)]
pub fn focus_main_for_pid(pid: u32) -> Result<(), String> {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, IsIconic,
        IsWindowVisible, GWL_EXSTYLE, GWL_STYLE, WS_EX_TOOLWINDOW, WS_VISIBLE,
    };

    if pid == 0 {
        return Err("pid is 0".into());
    }

    struct Ctx {
        pid: u32,
        best: Mutex<(isize, i64)>,
    }
    use std::sync::Mutex;

    let ctx = Box::new(Ctx {
        pid,
        best: Mutex::new((0, -1)),
    });
    let ctx_ptr = Box::into_raw(ctx);

    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &*(lparam.0 as *const Ctx);
        let mut wpid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut wpid));
        if wpid != ctx.pid {
            return BOOL(1);
        }
        if !IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() {
            return BOOL(1);
        }
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        if style & WS_VISIBLE.0 == 0 && !IsIconic(hwnd).as_bool() {
            return BOOL(1);
        }
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TOOLWINDOW.0 != 0 {
            return BOOL(1);
        }
        let mut rc = RECT::default();
        if GetWindowRect(hwnd, &mut rc).is_err() {
            return BOOL(1);
        }
        let area = ((rc.right - rc.left) as i64).max(0) * ((rc.bottom - rc.top) as i64).max(0);
        if let Ok(mut best) = ctx.best.lock() {
            if area > best.1 {
                *best = (hwnd.0 as isize, area);
            }
        }
        BOOL(1)
    }

    let hwnd = unsafe {
        let _ = EnumWindows(Some(enum_cb), LPARAM(ctx_ptr as isize));
        let ctx = Box::from_raw(ctx_ptr);
        ctx.best.into_inner().map(|b| b.0).unwrap_or(0)
    };
    if hwnd == 0 {
        return Err("no main window for pid".into());
    }
    focus_window(hwnd)
}

#[cfg(not(windows))]
pub fn focus_main_for_pid(_pid: u32) -> Result<(), String> {
    Err("focus_main_for_pid is only available on Windows".into())
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
