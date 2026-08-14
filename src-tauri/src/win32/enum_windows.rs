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

/// Shell / desktop HWNDs that are visible but are not real taskbar apps.
#[cfg(windows)]
fn is_shell_noise_class(class_name: &str) -> bool {
    matches!(
        class_name,
        "Progman"
            | "WorkerW"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "NotifyIconOverflowWindow"
            | "TopLevelWindowForOverflowXamlIsland"
            | "Windows.UI.Core.CoreWindow"
            | "ForegroundStaging"
            | "ApplicationManager_DesktopShellWindow"
            | "XamlExplorerHostIslandWindow"
            | "ImmersiveLauncher"
            | "DV2ControlHost"
            | "MsgrIMEWindowClass"
            | "SysShadow"
            | "ThumbnailDeviceHelperWnd"
            | "EdgeUiInputTopWndClass"
    )
}

/// Explorer desktop / tray hosts — not a folder window.
#[cfg(windows)]
fn is_explorer_shell_only(class_name: &str, exe_name: Option<&str>) -> bool {
    let exe = exe_name.unwrap_or("").to_ascii_lowercase();
    if exe != "explorer.exe" {
        return false;
    }
    // Real folder windows.
    if matches!(class_name, "CabinetWClass" | "ExploreWClass") {
        return false;
    }
    true
}

/// Suite helper processes that must not light a dock pin by themselves.
#[cfg(windows)]
fn is_suite_helper_exe(exe_name: Option<&str>) -> bool {
    let exe = exe_name.unwrap_or("").to_ascii_lowercase();
    matches!(
        exe.as_str(),
        "wpscloudsvr.exe"
            | "ksolaunch.exe"
            | "ksomisc.exe"
            | "wpscenter.exe"
            | "wpsofficeboot.exe"
            | "spotifylauncher.exe"
            | "dingtalk_launcher.exe"
            | "feishulauncher.exe"
            | "updat.exe"
            | "crashpad_handler.exe"
            | "msedgewebview2.exe"
            | "widgetservice.exe"
            | "widgets.exe"
            | "phoneexperiencehost.exe"
            | "gamebar.exe"
            | "gamebarftserver.exe"
            | "xboxgamebar.exe"
    )
}

#[cfg(windows)]
fn is_dwm_cloaked(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use std::ffi::c_void;
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    unsafe {
        let mut cloaked: u32 = 0;
        let ok = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            std::mem::size_of::<u32>() as u32,
        );
        ok.is_ok() && cloaked != 0
    }
}

#[cfg(windows)]
fn has_nonzero_client_area(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
    unsafe {
        let mut rc = RECT::default();
        if GetClientRect(hwnd, &mut rc).is_err() {
            return false;
        }
        (rc.right - rc.left) > 0 && (rc.bottom - rc.top) > 0
    }
}

#[cfg(windows)]
pub fn list_windows(exclude_hwnd: Option<isize>) -> Vec<WindowInfo> {
    use std::sync::Mutex;
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW,
        GetWindowThreadProcessId, IsWindowVisible, GWL_EXSTYLE, GWL_STYLE, WS_EX_TOOLWINDOW,
        WS_DISABLED,
    };

    struct Ctx {
        exclude: Option<isize>,
        /// Host process — dock / island / preview must never appear as taskbar apps.
        self_pid: u32,
        out: Mutex<Vec<WindowInfo>>,
    }

    let ctx = Box::new(Ctx {
        exclude: exclude_hwnd,
        self_pid: unsafe { GetCurrentProcessId() },
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
        // Do NOT filter WS_EX_NOACTIVATE here — some real apps keep it on their
        // top-level HWND and would never appear as unpinned dock extras.

        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        if style & WS_DISABLED.0 != 0 {
            return BOOL(1);
        }

        // Taskbar-like: skip owned windows (dialogs / floaters).
        {
            use windows::Win32::UI::WindowsAndMessaging::{GetWindow, GW_OWNER};
            if let Ok(owner) = GetWindow(hwnd, GW_OWNER) {
                if !owner.0.is_null() {
                    return BOOL(1);
                }
            }
        }

        if is_dwm_cloaked(hwnd) {
            return BOOL(1);
        }
        // Minimized windows often report 0×0 client — still taskbar-worthy.
        {
            use windows::Win32::UI::WindowsAndMessaging::IsIconic;
            if !IsIconic(hwnd).as_bool() && !has_nonzero_client_area(hwnd) {
                return BOOL(1);
            }
        }

        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        // Drop every top-level HWND we own (dock, preview, settings, glass, …).
        if pid != 0 && pid == ctx.self_pid {
            return BOOL(1);
        }

        let title_len = GetWindowTextLengthW(hwnd);
        let title = if title_len > 0 {
            let mut title_buf = vec![0u16; (title_len + 1) as usize];
            let n = GetWindowTextW(hwnd, &mut title_buf);
            title_buf.truncate(n as usize);
            String::from_utf16_lossy(&title_buf)
        } else {
            String::new()
        };

        let mut class_buf = [0u16; 256];
        let cn = GetClassNameW(hwnd, &mut class_buf);
        let class_name = String::from_utf16_lossy(&class_buf[..cn as usize]);

        if is_shell_noise_class(&class_name) {
            return BOOL(1);
        }

        let (exe, exe_name) = process_exe(pid);

        if is_explorer_shell_only(&class_name, exe_name.as_deref()) {
            return BOOL(1);
        }
        if is_suite_helper_exe(exe_name.as_deref()) {
            return BOOL(1);
        }

        // Taskbar-like: allow empty title when we still have a real process image.
        // (Previously title_len==0 dropped many legitimate top-level apps.)
        if title.trim().is_empty() && exe.as_ref().map(|e| e.is_empty()).unwrap_or(true) {
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
    use windows::Win32::Foundation::{BOOL, HWND};
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        keybd_event, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VK_MENU,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId,
        IsIconic, IsWindow, SetForegroundWindow, SetWindowPos, ShowWindow, HWND_TOP, SWP_NOMOVE,
        SWP_NOSIZE, SWP_SHOWWINDOW, SW_RESTORE, SW_SHOW,
    };

    // user32 SwitchToThisWindow — more reliable than SetForegroundWindow on Win10.
    #[link(name = "user32")]
    extern "system" {
        fn SwitchToThisWindow(hwnd: HWND, f_alt_tab: BOOL);
    }

    unsafe {
        let h = HWND(hwnd as *mut _);
        if !IsWindow(h).as_bool() {
            return Err("window no longer exists".into());
        }
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        } else {
            let _ = ShowWindow(h, SW_SHOW);
        }

        // ASFW_ANY — required when the caller is a WS_EX_NOACTIVATE dock on Win10.
        let _ = AllowSetForegroundWindow(u32::MAX);

        let mut target_pid = 0u32;
        let target_tid = GetWindowThreadProcessId(h, Some(&mut target_pid));
        if target_pid != 0 {
            let _ = AllowSetForegroundWindow(target_pid);
        }

        let fg = GetForegroundWindow();
        let mut fg_pid = 0u32;
        let fg_tid = GetWindowThreadProcessId(fg, Some(&mut fg_pid));
        let cur_tid = GetCurrentThreadId();

        let attached_fg =
            fg_tid != 0 && fg_tid != cur_tid && AttachThreadInput(cur_tid, fg_tid, true).as_bool();
        let attached_tg = target_tid != 0
            && target_tid != cur_tid
            && AttachThreadInput(cur_tid, target_tid, true).as_bool();

        // Synthetic Alt unlocks the foreground lock (Win10 is stricter than Win11).
        keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_EXTENDEDKEY, 0);
        keybd_event(
            VK_MENU.0 as u8,
            0,
            KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP,
            0,
        );

        let _ = BringWindowToTop(h);
        let _ = SetWindowPos(
            h,
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        );
        SwitchToThisWindow(h, BOOL(1));
        let _ = SetForegroundWindow(h);

        if attached_tg {
            let _ = AttachThreadInput(cur_tid, target_tid, false);
        }
        if attached_fg {
            let _ = AttachThreadInput(cur_tid, fg_tid, false);
        }

        Ok(())
    }
}

/// Map the OS foreground window onto a dock candidate HWND to minimize.
///
/// Electron / Cursor often reports FG as a child or owned popup that never
/// appears in [`list_windows`]. Bare same-PID → "first pin window" used to
/// false-minimize; we only minimize the actual FG top-level (or its owner in
/// the candidate set).
#[cfg(windows)]
fn minimize_target_for_foreground(candidates: &[isize]) -> Option<isize> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetForegroundWindow, GetWindow, GetWindowLongW, GetWindowThreadProcessId,
        IsIconic, IsWindow, IsWindowVisible, GA_ROOT, GA_ROOTOWNER, GW_OWNER, GWL_EXSTYLE,
        WS_EX_TOOLWINDOW,
    };

    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() || !IsWindow(fg).as_bool() {
            return None;
        }
        let fg_raw = fg.0 as isize;
        let root = GetAncestor(fg, GA_ROOT);
        let root_owner = GetAncestor(fg, GA_ROOTOWNER);
        let root_raw = root.0 as isize;
        let owner_raw = root_owner.0 as isize;

        let pick = |raw: isize| -> Option<isize> {
            if raw == 0 || !candidates.contains(&raw) {
                return None;
            }
            let h = HWND(raw as *mut _);
            if IsWindow(h).as_bool() && !IsIconic(h).as_bool() {
                Some(raw)
            } else {
                None
            }
        };

        if let Some(h) = pick(fg_raw)
            .or_else(|| pick(root_raw))
            .or_else(|| pick(owner_raw))
        {
            return Some(h);
        }

        // Owned dialogs / floaters are dropped by EnumWindows — walk owners.
        let mut cur = fg;
        for _ in 0..8 {
            let Ok(owner) = GetWindow(cur, GW_OWNER) else {
                break;
            };
            if owner.0.is_null() {
                break;
            }
            let o = owner.0 as isize;
            if let Some(h) = pick(o) {
                return Some(h);
            }
            cur = owner;
        }

        // Same-PID fallback: FG belongs to this app, but HWND wasn't enumerated
        // (Electron helper chrome). Minimize the FG top-level itself — never a
        // random sibling pin window (that was the Cursor false-minimize bug).
        let mut fg_pid = 0u32;
        GetWindowThreadProcessId(fg, Some(&mut fg_pid));
        if fg_pid == 0 {
            return None;
        }
        let mut group_has_pid = false;
        for &c in candidates {
            let h = HWND(c as *mut _);
            if !IsWindow(h).as_bool() {
                continue;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(h, Some(&mut pid));
            if pid == fg_pid {
                group_has_pid = true;
                break;
            }
        }
        if !group_has_pid {
            return None;
        }

        let top = if !root.0.is_null() && IsWindow(root).as_bool() {
            root
        } else {
            fg
        };
        if IsIconic(top).as_bool() {
            return None;
        }
        // Ignore invisible / tool / empty helpers that can linger as FG.
        if !IsWindowVisible(top).as_bool() {
            return None;
        }
        let ex = GetWindowLongW(top, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TOOLWINDOW.0 != 0 {
            return None;
        }
        if is_dwm_cloaked(top) {
            return None;
        }
        if !has_nonzero_client_area(top) {
            return None;
        }

        let top_raw = top.0 as isize;
        if candidates.contains(&top_raw) {
            return Some(top_raw);
        }
        // Prefer an owned candidate when FG is a non-enumerated popup.
        if let Some(h) = pick(owner_raw) {
            return Some(h);
        }
        Some(top_raw)
    }
}

/// Dock / taskbar toggle: if `hwnd` is already foreground → minimize;
/// if minimized → restore; otherwise focus.
#[cfg(windows)]
pub fn focus_or_minimize_window(hwnd: isize) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindow, ShowWindow, SW_MINIMIZE};

    unsafe {
        let h = HWND(hwnd as *mut _);
        if !IsWindow(h).as_bool() {
            return Err("window no longer exists".into());
        }
        if IsIconic(h).as_bool() {
            return focus_window(hwnd);
        }

        if let Some(target) = minimize_target_for_foreground(&[hwnd]) {
            let th = HWND(target as *mut _);
            if IsWindow(th).as_bool() && !IsIconic(th).as_bool() {
                let _ = ShowWindow(th, SW_MINIMIZE);
                return Ok(());
            }
        }
    }
    focus_window(hwnd)
}

/// Like [`focus_or_minimize_window`], but for a group of related HWNDs (same dock app).
#[cfg(windows)]
pub fn focus_or_minimize_group(hwnds: &[isize]) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindow, ShowWindow, SW_MINIMIZE};

    if hwnds.is_empty() {
        return Err("no windows".into());
    }

    unsafe {
        if let Some(target) = minimize_target_for_foreground(hwnds) {
            let h = HWND(target as *mut _);
            if IsWindow(h).as_bool() && !IsIconic(h).as_bool() {
                let _ = ShowWindow(h, SW_MINIMIZE);
                return Ok(());
            }
        }

        // Prefer restoring a minimized member; else focus the first valid.
        if let Some(&raw) = hwnds.iter().find(|&&raw| {
            let h = HWND(raw as *mut _);
            IsWindow(h).as_bool() && IsIconic(h).as_bool()
        }) {
            return focus_window(raw);
        }
        for &raw in hwnds {
            let h = HWND(raw as *mut _);
            if IsWindow(h).as_bool() {
                return focus_window(raw);
            }
        }
    }
    Err("no valid window in group".into())
}

/// Focus / restore the best HWND in a group — never minimize (dock activate).
#[cfg(windows)]
pub fn focus_group(hwnds: &[isize]) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindow};

    if hwnds.is_empty() {
        return Err("no windows".into());
    }
    unsafe {
        if let Some(&raw) = hwnds.iter().find(|&&raw| {
            let h = HWND(raw as *mut _);
            IsWindow(h).as_bool() && IsIconic(h).as_bool()
        }) {
            return focus_window(raw);
        }
        for &raw in hwnds {
            let h = HWND(raw as *mut _);
            if IsWindow(h).as_bool() {
                return focus_window(raw);
            }
        }
    }
    Err("no valid window in group".into())
}

#[cfg(not(windows))]
pub fn focus_or_minimize_window(hwnd: isize) -> Result<(), String> {
    focus_window(hwnd)
}

#[cfg(not(windows))]
pub fn focus_or_minimize_group(hwnds: &[isize]) -> Result<(), String> {
    hwnds
        .first()
        .copied()
        .ok_or_else(|| "no windows".into())
        .and_then(focus_window)
}

#[cfg(not(windows))]
pub fn focus_group(hwnds: &[isize]) -> Result<(), String> {
    focus_or_minimize_group(hwnds)
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
