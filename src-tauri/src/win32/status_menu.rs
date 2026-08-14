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
    /// Exe icon as PNG base64 (empty / None when desktop or unavailable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_png: Option<String>,
}

#[cfg(windows)]
mod win {
    use super::ForegroundApp;
    use windows::core::{s, GUID};
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT, WPARAM};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::Threading::{
        AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
        PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Shell::{
        IShellDispatch4, SHAppBarMessage, ABS_AUTOHIDE, ABM_GETSTATE, ABM_SETSTATE, APPBARDATA,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, FindWindowA, FindWindowExA, GetAncestor, GetClassNameW,
        GetForegroundWindow, GetSystemMetrics, GetWindowRect, GetWindowTextLengthW,
        GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, SendMessageW,
        SetForegroundWindow, SetWindowPos, ShowWindow, GA_ROOT, HWND_BOTTOM, SC_TASKLIST,
        SM_CYVIRTUALSCREEN, SM_YVIRTUALSCREEN, SWP_NOACTIVATE, SWP_NOSENDCHANGING, SW_HIDE,
        SW_SHOWNA, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_SYSCOMMAND,
    };
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{LazyLock, Mutex};

    /// Shell.Application
    const CLSID_SHELL: GUID = GUID::from_u128(0x13709620_C279_11CE_A49E_444553540000);

    static ICON_CACHE: LazyLock<Mutex<HashMap<String, Option<String>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

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

    fn process_exe_path(pid: u32) -> Option<String> {
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
            Some(String::from_utf16_lossy(&buf[..size as usize]))
        }
    }

    fn icon_for_exe(exe_path: Option<&str>) -> Option<String> {
        let path = exe_path?.trim();
        if path.is_empty() {
            return None;
        }
        {
            let cache = ICON_CACHE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(hit) = cache.get(path) {
                return hit.clone();
            }
        }
        let png = crate::dock::resolve_item_icon_png("", path);
        if let Ok(mut cache) = ICON_CACHE.lock() {
            cache.insert(path.to_string(), png.clone());
        }
        png
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
                icon_png: None,
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
                        icon_png: None,
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
                    icon_png: None,
                };
            }
            let title = window_title(fg);
            let mut pid = 0u32;
            GetWindowThreadProcessId(fg, Some(&mut pid));
            let exe_path = process_exe_path(pid);
            let exe = exe_path.as_ref().and_then(|path| {
                std::path::Path::new(path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
            });
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
                        icon_png: None,
                    };
                }
            }
            let label = make_label(&title, exe.as_deref());
            let icon_png = icon_for_exe(exe_path.as_deref());
            ForegroundApp {
                title,
                exe_name: exe,
                label,
                is_self: false,
                hwnd,
                window_id,
                icon_png,
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

    /// BM_CLICK — start button.
    const BM_CLICK: u32 = 0x00F5;

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

    /// Park the taskbar below the virtual desktop so tray-flash / Explorer
    /// re-shows land off-screen (plain SW_HIDE fights blinking icons → white bar flicker).
    fn exile_taskbar(hwnd: HWND) {
        unsafe {
            let mut rc = RECT::default();
            if GetWindowRect(hwnd, &mut rc).is_err() {
                let _ = ShowWindow(hwnd, SW_HIDE);
                return;
            }
            let w = (rc.right - rc.left).max(1);
            let h = (rc.bottom - rc.top).max(1);
            let y = GetSystemMetrics(SM_YVIRTUALSCREEN)
                + GetSystemMetrics(SM_CYVIRTUALSCREEN)
                + 120;
            let _ = SetWindowPos(
                hwnd,
                HWND_BOTTOM,
                rc.left,
                y,
                w,
                h,
                SWP_NOACTIVATE | SWP_NOSENDCHANGING,
            );
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }

    fn hide_taskbars_once() {
        for_each_taskbar(|hwnd| {
            // Always re-exile: Explorer may SW_SHOW for tray blink while still "visible".
            exile_taskbar(hwnd);
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
                // Faster than 400ms — tray attention blinks ~2Hz and otherwise flashes a white bar.
                std::thread::sleep(std::time::Duration::from_millis(80));
            }
        });
    }

    fn stop_keep_hidden() {
        TASKBAR_KEEP_HIDDEN.store(false, Ordering::SeqCst);
    }

    /// Whether we are currently forcing the system taskbar hidden.
    pub fn is_taskbar_visible() -> bool {
        if TASKBAR_KEEP_HIDDEN.load(Ordering::SeqCst) {
            return false;
        }
        shell_tray_hwnd()
            .map(|hwnd| unsafe { IsWindowVisible(hwnd).as_bool() })
            .unwrap_or(true)
    }

    /// Hide system taskbar while Dock owns the bottom edge.
    /// Auto-hide reclaims work area; keep-hidden loop exiles the bar off-screen
    /// so tray flash / Explorer re-show cannot paint a white strip at the bottom.
    pub fn set_taskbar_visible(visible: bool) -> Result<(), String> {
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
                // Explorer repositions on show — no need to restore our exile coords.
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

    /// Open Start — background dock poller is not foreground, so Win key is filtered.
    /// Prefer shell tray Start button / SC_TASKLIST, then AttachThreadInput + Win.
    pub fn open_start_menu() -> Result<(), String> {
        run_shell_action(|| {
            unsafe {
                let _ = AllowSetForegroundWindow(u32::MAX);
            }
            if click_start_button().is_ok() {
                return Ok(());
            }
            if post_tasklist().is_ok() {
                return Ok(());
            }
            tap_win_attached()
        })
    }

    pub fn show_desktop() -> Result<(), String> {
        // Prefer shell COM. SendInput Win+D from the dock visibility poller is often filtered.
        run_shell_action(|| {
            unsafe {
                let _ = AllowSetForegroundWindow(u32::MAX);
            }
            if toggle_desktop_com().is_ok() {
                return Ok(());
            }
            if click_show_desktop_button().is_ok() {
                return Ok(());
            }
            chord_win_d_attached()
        })
    }

    fn run_shell_action(f: impl FnOnce() -> Result<(), String> + Send + 'static) -> Result<(), String> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv()
            .unwrap_or_else(|_| Err("shell action worker died".into()))
    }

    fn post_tasklist() -> Result<(), String> {
        let tray = shell_tray_hwnd().ok_or_else(|| "no explorer tray".to_string())?;
        unsafe {
            // SendMessage is more reliable than Post when the tray is SW_HIDE / exiled.
            let _ = SendMessageW(
                tray,
                WM_SYSCOMMAND,
                WPARAM(SC_TASKLIST as usize),
                LPARAM(0),
            );
            let _ = PostMessageW(
                tray,
                WM_SYSCOMMAND,
                WPARAM(SC_TASKLIST as usize),
                LPARAM(0),
            );
        }
        Ok(())
    }

    fn click_start_button() -> Result<(), String> {
        let tray = shell_tray_hwnd().ok_or_else(|| "no explorer tray".to_string())?;
        unsafe {
            let start = FindWindowExA(tray, None, s!("Start"), None)
                .or_else(|_| FindWindowA(s!("Button"), s!("Start")))
                .map_err(|_| "no Start button".to_string())?;
            let _ = AllowSetForegroundWindow(u32::MAX);
            let _ = SetForegroundWindow(tray);
            let _ = SendMessageW(start, BM_CLICK, WPARAM(0), LPARAM(0));
        }
        Ok(())
    }

    /// Attach to explorer's input queue so SendInput is not UIPI-filtered.
    fn with_explorer_input<F>(f: F) -> Result<(), String>
    where
        F: FnOnce() -> Result<(), String>,
    {
        let tray = shell_tray_hwnd().ok_or_else(|| "no explorer tray".to_string())?;
        unsafe {
            let mut explorer_pid = 0u32;
            let explorer_tid = GetWindowThreadProcessId(tray, Some(&mut explorer_pid));
            let our_tid = GetCurrentThreadId();
            if explorer_pid != 0 {
                let _ = AllowSetForegroundWindow(explorer_pid);
            } else {
                let _ = AllowSetForegroundWindow(u32::MAX);
            }
            let attached = explorer_tid != 0
                && explorer_tid != our_tid
                && AttachThreadInput(our_tid, explorer_tid, true).as_bool();
            let _ = SetForegroundWindow(tray);
            let result = f();
            if attached {
                let _ = AttachThreadInput(our_tid, explorer_tid, false);
            }
            result
        }
    }

    fn tap_win_attached() -> Result<(), String> {
        with_explorer_input(|| crate::win32::input::tap_win_key())
    }

    fn chord_win_d_attached() -> Result<(), String> {
        with_explorer_input(|| crate::win32::input::chord_win_d())
    }

    fn toggle_desktop_com() -> Result<(), String> {
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
            shell
                .ToggleDesktop()
                .map_err(|e| format!("ToggleDesktop: {e}"))?;
        }
        Ok(())
    }

    /// Click the taskbar "Show desktop" peek button when present (Win10).
    fn click_show_desktop_button() -> Result<(), String> {
        let tray = shell_tray_hwnd().ok_or_else(|| "no explorer tray".to_string())?;
        unsafe {
            let btn = FindWindowExA(tray, None, s!("TrayShowDesktopButtonWClass"), None)
                .map_err(|_| "no Show Desktop button".to_string())?;
            let _ = SendMessageW(btn, WM_LBUTTONDOWN, WPARAM(0), LPARAM(0));
            let _ = SendMessageW(btn, WM_LBUTTONUP, WPARAM(0), LPARAM(0));
        }
        Ok(())
    }

    /// 状态菜单常用系统工具：taskmgr / shells(runas) / control-panel …
    pub fn open_system_tool(kind: &str) -> Result<(), String> {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        use windows::core::{w, HSTRING, PCWSTR};
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // ShellExecute 错误码：用户取消 UAC 常见为 SE_ERR_ACCESSDENIED(5)
        const SE_ERR_ACCESSDENIED: isize = 5;
        let k = kind.trim().to_ascii_lowercase();

        let shell_exec = |verb: PCWSTR, file: &str| -> Result<(), String> {
            unsafe {
                let rc = ShellExecuteW(
                    HWND::default(),
                    verb,
                    &HSTRING::from(file),
                    None,
                    None,
                    SW_SHOWNORMAL,
                );
                let code = rc.0 as isize;
                if code <= 32 {
                    if code == SE_ERR_ACCESSDENIED {
                        // 用户取消提权，不当错误抛
                        return Ok(());
                    }
                    return Err(format!("打开失败 ({code})"));
                }
            }
            Ok(())
        };

        let shell_open = |file: &str| -> Result<(), String> { shell_exec(w!("open"), file) };
        let shell_runas = |file: &str| -> Result<(), String> { shell_exec(w!("runas"), file) };

        match k.as_str() {
            "taskmgr" | "task-manager" => {
                Command::new("taskmgr.exe")
                    .creation_flags(CREATE_NO_WINDOW)
                    .spawn()
                    .map_err(|e| e.to_string())?;
                Ok(())
            }
            "device-manager" | "devmgmt" => shell_open("devmgmt.msc"),
            "control-panel" | "control" => {
                Command::new("control.exe")
                    .creation_flags(CREATE_NO_WINDOW)
                    .spawn()
                    .map_err(|e| e.to_string())?;
                Ok(())
            }
            "windows-settings" | "settings" | "ms-settings" => shell_open("ms-settings:"),
            "env-vars" | "environment-variables" | "env" => {
                // 直接打开「环境变量」对话框
                Command::new("rundll32.exe")
                    .arg("sysdm.cpl,EditEnvironmentVariables")
                    .creation_flags(CREATE_NO_WINDOW)
                    .spawn()
                    .map_err(|e| e.to_string())?;
                Ok(())
            }
            "cmd-admin" | "admin-cmd" => shell_runas("cmd.exe"),
            "powershell-admin" | "admin-powershell" | "admin-ps" => {
                shell_runas("powershell.exe")
            }
            // 普通权限 Windows Terminal；未安装则回退普通 PowerShell
            "terminal" | "wt" | "windows-terminal" => {
                match shell_open("wt.exe") {
                    Ok(()) => Ok(()),
                    Err(_) => shell_open("powershell.exe"),
                }
            }
            other => Err(format!("unknown system tool: {other}")),
        }
    }
}

#[cfg(windows)]
pub use win::{
    foreground_app, is_taskbar_visible, open_start_menu, open_system_tool, set_taskbar_visible,
    show_desktop,
};

#[cfg(not(windows))]
pub fn foreground_app(_self_hwnd: Option<isize>) -> ForegroundApp {
    ForegroundApp {
        title: String::new(),
        exe_name: None,
        label: "Desktop".into(),
        is_self: false,
        hwnd: 0,
        window_id: None,
        icon_png: None,
    }
}

#[cfg(not(windows))]
pub fn is_taskbar_visible() -> bool {
    true
}

#[cfg(not(windows))]
pub fn set_taskbar_visible(_visible: bool) -> Result<(), String> {
    Err("Windows only".into())
}

#[cfg(not(windows))]
pub fn show_desktop() -> Result<(), String> {
    Err("Windows only".into())
}

#[cfg(not(windows))]
pub fn open_start_menu() -> Result<(), String> {
    Err("Windows only".into())
}

#[cfg(not(windows))]
pub fn open_system_tool(_kind: &str) -> Result<(), String> {
    Err("Windows only".into())
}
