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
    /// Edge/Chrome PWA `--app-id` (or derived from window AppUserModelID).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
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

/// PKEY_AppUserModel_ID = {9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3}, 5
#[cfg(windows)]
fn window_aumid(hwnd: windows::Win32::Foundation::HWND) -> Option<String> {
    use windows::core::{GUID, PCWSTR};
    use windows::Win32::System::Variant::VT_LPWSTR;
    use windows::Win32::UI::Shell::PropertiesSystem::{
        IPropertyStore, PROPERTYKEY, SHGetPropertyStoreForWindow,
    };

    const PKEY_APPUSERMODEL_ID: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3),
        pid: 5,
    };

    unsafe {
        let store: IPropertyStore = SHGetPropertyStoreForWindow(hwnd).ok()?;
        let pv = store.GetValue(&PKEY_APPUSERMODEL_ID).ok()?;
        let raw = pv.as_raw();
        let vt = raw.Anonymous.Anonymous.vt;
        if vt != VT_LPWSTR.0 {
            return None;
        }
        let p = raw.Anonymous.Anonymous.Anonymous.pwszVal;
        if p.is_null() {
            return None;
        }
        let s = PCWSTR(p).to_string().ok()?;
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}

/// Read process command line via PEB (best-effort; fails for elevated / protected).
#[cfg(windows)]
fn process_command_line(pid: u32) -> Option<String> {
    use std::mem::{size_of, zeroed};
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows::Win32::System::Threading::{
        OpenProcess, PEB, PROCESS_BASIC_INFORMATION, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
        RTL_USER_PROCESS_PARAMETERS,
    };

    #[link(name = "ntdll")]
    extern "system" {
        fn NtQueryInformationProcess(
            process: HANDLE,
            info_class: u32,
            info: *mut core::ffi::c_void,
            info_len: u32,
            ret_len: *mut u32,
        ) -> i32;
    }

    const ProcessBasicInformation: u32 = 0;

    if pid == 0 {
        return None;
    }
    unsafe {
        let Ok(proc) = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid) else {
            return None;
        };
        let mut pbi: PROCESS_BASIC_INFORMATION = zeroed();
        let mut ret = 0u32;
        let status = NtQueryInformationProcess(
            proc,
            ProcessBasicInformation,
            &mut pbi as *mut _ as *mut _,
            size_of::<PROCESS_BASIC_INFORMATION>() as u32,
            &mut ret,
        );
        if status < 0 || pbi.PebBaseAddress.is_null() {
            let _ = CloseHandle(proc);
            return None;
        }
        let mut peb: PEB = zeroed();
        let mut read = 0usize;
        if ReadProcessMemory(
            proc,
            pbi.PebBaseAddress as *const _,
            &mut peb as *mut _ as *mut _,
            size_of::<PEB>(),
            Some(&mut read),
        )
        .is_err()
            || peb.ProcessParameters.is_null()
        {
            let _ = CloseHandle(proc);
            return None;
        }
        let mut params: RTL_USER_PROCESS_PARAMETERS = zeroed();
        if ReadProcessMemory(
            proc,
            peb.ProcessParameters as *const _,
            &mut params as *mut _ as *mut _,
            size_of::<RTL_USER_PROCESS_PARAMETERS>(),
            Some(&mut read),
        )
        .is_err()
        {
            let _ = CloseHandle(proc);
            return None;
        }
        let byte_len = params.CommandLine.Length as usize;
        if byte_len == 0 || params.CommandLine.Buffer.0.is_null() {
            let _ = CloseHandle(proc);
            return None;
        }
        let mut buf = vec![0u16; (byte_len / 2).saturating_add(1)];
        if ReadProcessMemory(
            proc,
            params.CommandLine.Buffer.0 as *const _,
            buf.as_mut_ptr() as *mut _,
            byte_len,
            Some(&mut read),
        )
        .is_err()
        {
            let _ = CloseHandle(proc);
            return None;
        }
        let _ = CloseHandle(proc);
        let n = (byte_len / 2).min(buf.len());
        Some(String::from_utf16_lossy(&buf[..n]))
    }
}

fn resolve_window_app_id(
    hwnd: windows::Win32::Foundation::HWND,
    pid: u32,
    exe_name: Option<&str>,
    cmdline_cache: &mut std::collections::HashMap<u32, Option<String>>,
) -> Option<String> {
    use crate::dock::shortcut::{
        is_browser_exe_key, normalize_window_app_id, parse_browser_app_id,
    };

    let exe = exe_name.unwrap_or("");
    if !is_browser_exe_key(exe) {
        return None;
    }
    let cmdline = cmdline_cache
        .entry(pid)
        .or_insert_with(|| process_command_line(pid))
        .as_deref();
    let from_cmd = cmdline.and_then(parse_browser_app_id);
    let aumid = window_aumid(hwnd);
    normalize_window_app_id(from_cmd.as_deref(), aumid.as_deref())
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
                app_id: None,
            });
        }
        BOOL(1)
    }

    unsafe {
        let _ = EnumWindows(Some(enum_cb), LPARAM(ctx_ptr as isize));
        let ctx = Box::from_raw(ctx_ptr);
        let mut out = ctx.out.into_inner().unwrap_or_default();
        let mut cmdline_cache = std::collections::HashMap::<u32, Option<String>>::new();
        for w in &mut out {
            w.app_id = resolve_window_app_id(
                HWND(w.hwnd as _),
                w.pid,
                w.exe_name.as_deref(),
                &mut cmdline_cache,
            );
        }
        out
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

/// Minimize like the taskbar. Electron/Chromium often no-ops `ShowWindow(SW_MINIMIZE)`;
/// `WM_SYSCOMMAND/SC_MINIMIZE` matches the title-bar minimize path.
#[cfg(windows)]
fn minimize_hwnd(hwnd: isize) -> bool {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, IsWindow, PostMessageW, SendMessageW, ShowWindow, SC_MINIMIZE, SW_MINIMIZE,
        WM_SYSCOMMAND,
    };

    unsafe {
        let h = HWND(hwnd as *mut _);
        if !IsWindow(h).as_bool() || IsIconic(h).as_bool() {
            return false;
        }
        let _ = SendMessageW(
            h,
            WM_SYSCOMMAND,
            WPARAM(SC_MINIMIZE as usize),
            LPARAM(0),
        );
        if IsIconic(h).as_bool() {
            return true;
        }
        let _ = PostMessageW(
            h,
            WM_SYSCOMMAND,
            WPARAM(SC_MINIMIZE as usize),
            LPARAM(0),
        );
        let _ = ShowWindow(h, SW_MINIMIZE);
        // Do not require IsIconic yet — Electron may apply SC_MINIMIZE asynchronously;
        // falling through to focus_window would undo the minimize.
        true
    }
}

/// True when `hwnd` is a visible, non-iconic top-level that is safe to minimize.
#[cfg(windows)]
fn is_minimizable_top_level(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindow, GetWindowLongW, IsIconic, IsWindow, IsWindowVisible, GW_OWNER, GWL_EXSTYLE,
        WS_EX_TOOLWINDOW,
    };
    unsafe {
        if !IsWindow(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return false;
        }
        if !IsWindowVisible(hwnd).as_bool() {
            return false;
        }
        if let Ok(owner) = GetWindow(hwnd, GW_OWNER) {
            if !owner.0.is_null() {
                return false;
            }
        }
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        if is_dwm_cloaked(hwnd) {
            return false;
        }
        has_nonzero_client_area(hwnd)
    }
}

/// Largest visible non-iconic candidate (stable pick when FG maps to the app but not a HWND).
#[cfg(windows)]
fn best_minimizable_candidate(candidates: &[isize]) -> Option<isize> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

    let mut best: Option<(isize, i64)> = None;
    unsafe {
        for &raw in candidates {
            let h = HWND(raw as *mut _);
            if !is_minimizable_top_level(h) {
                continue;
            }
            let mut rc = RECT::default();
            let area = if GetClientRect(h, &mut rc).is_ok() {
                (rc.right - rc.left) as i64 * (rc.bottom - rc.top) as i64
            } else {
                0
            };
            if best.map(|(_, a)| area > a).unwrap_or(true) {
                best = Some((raw, area));
            }
        }
    }
    best.map(|(h, _)| h)
}

/// Topmost taskbar-style window, skipping Host/Dock PIDs (Z-order walk).
#[cfg(windows)]
fn topmost_taskbar_hwnd(skip_pids: &[u32]) -> Option<isize> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetTopWindow, GetWindow, GetWindowThreadProcessId, GW_HWNDNEXT,
    };

    unsafe {
        let mut cur = GetTopWindow(HWND::default()).unwrap_or_default();
        for _ in 0..512 {
            if cur.0.is_null() {
                break;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(cur, Some(&mut pid));
            if pid != 0
                && !skip_pids.contains(&pid)
                && is_minimizable_top_level(cur)
            {
                let mut class_buf = [0u16; 256];
                let cn = GetClassNameW(cur, &mut class_buf);
                let class_name = String::from_utf16_lossy(&class_buf[..cn as usize]);
                if !is_shell_noise_class(&class_name) {
                    let (_, exe_name) = process_exe(pid);
                    if !is_explorer_shell_only(&class_name, exe_name.as_deref())
                        && !is_suite_helper_exe(exe_name.as_deref())
                    {
                        return Some(cur.0 as isize);
                    }
                }
            }
            cur = GetWindow(cur, GW_HWNDNEXT).unwrap_or_default();
        }
    }
    None
}

/// Whether FG (or its process image) belongs to the candidate dock app.
#[cfg(windows)]
fn foreground_belongs_to_candidates(
    candidates: &[isize],
    fg_pid: u32,
    fg_exe: Option<&str>,
) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow};

    if fg_pid == 0 {
        return false;
    }
    let fg_exe_l = fg_exe.map(|s| s.to_ascii_lowercase());
    unsafe {
        for &c in candidates {
            let h = HWND(c as *mut _);
            if !IsWindow(h).as_bool() {
                continue;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(h, Some(&mut pid));
            if pid == fg_pid {
                return true;
            }
            if let Some(ref want) = fg_exe_l {
                if !want.is_empty() {
                    let (_, name) = process_exe(pid);
                    if name
                        .as_deref()
                        .map(|n| n.eq_ignore_ascii_case(want))
                        .unwrap_or(false)
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Map the OS foreground window onto a dock candidate HWND to minimize.
///
/// Electron / Cursor often reports FG as a child or owned popup that never
/// appears in [`list_windows`]. Bare same-PID → "first pin window" used to
/// false-minimize; we only minimize the actual FG top-level, its owner in the
/// candidate set, or (when FG is Dock itself) the topmost Z-order candidate.
#[cfg(windows)]
fn minimize_target_for_foreground(candidates: &[isize]) -> Option<isize> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetForegroundWindow, GetWindow, GetWindowThreadProcessId, IsIconic, IsWindow,
        GA_ROOT, GA_ROOTOWNER, GW_OWNER,
    };

    if candidates.is_empty() {
        return None;
    }

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

        let mut fg_pid = 0u32;
        GetWindowThreadProcessId(fg, Some(&mut fg_pid));
        let self_pid = GetCurrentProcessId();
        let (_, fg_exe) = process_exe(fg_pid);

        // Dock is WS_EX_NOACTIVATE, but WebView2 can still briefly become FG on click.
        // Then use Z-order: if the topmost real window is this dock app → minimize it.
        if fg_pid != 0 && fg_pid == self_pid {
            if let Some(top) = topmost_taskbar_hwnd(&[self_pid]) {
                if let Some(h) = pick(top) {
                    return Some(h);
                }
                // Topmost shares exe with candidates (Electron multi-PID).
                let mut top_pid = 0u32;
                GetWindowThreadProcessId(HWND(top as *mut _), Some(&mut top_pid));
                let (_, top_exe) = process_exe(top_pid);
                if foreground_belongs_to_candidates(candidates, top_pid, top_exe.as_deref()) {
                    return best_minimizable_candidate(candidates);
                }
            }
            return None;
        }

        if !foreground_belongs_to_candidates(candidates, fg_pid, fg_exe.as_deref()) {
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

        let top_raw = top.0 as isize;
        if candidates.contains(&top_raw) && !IsIconic(top).as_bool() {
            return Some(top_raw);
        }
        if let Some(h) = pick(owner_raw) {
            return Some(h);
        }
        // FG helper chrome (tool / zero-client / cloaked): minimize the real candidate.
        if is_minimizable_top_level(top) {
            // Prefer minimizing the actual FG top-level when it is a real window
            // (even if EnumWindows dropped it) — avoids picking a sibling.
            return Some(top_raw);
        }
        best_minimizable_candidate(candidates)
    }
}

/// Dock / taskbar toggle: if `hwnd` is already foreground → minimize;
/// if minimized → restore; otherwise focus.
#[cfg(windows)]
pub fn focus_or_minimize_window(hwnd: isize) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindow};

    unsafe {
        let h = HWND(hwnd as *mut _);
        if !IsWindow(h).as_bool() {
            return Err("window no longer exists".into());
        }
        if IsIconic(h).as_bool() {
            return focus_window(hwnd);
        }

        if let Some(target) = minimize_target_for_foreground(&[hwnd]) {
            if minimize_hwnd(target) {
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
    use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindow};

    if hwnds.is_empty() {
        return Err("no windows".into());
    }

    unsafe {
        if let Some(target) = minimize_target_for_foreground(hwnds) {
            if minimize_hwnd(target) {
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
