//! Detect exclusive / borderless fullscreen (games) so the island can hide.

#[cfg(windows)]
mod win {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetAncestor, GetClassNameW, GetForegroundWindow, GetWindowLongW,
        GetWindowRect, IsIconic, IsWindow, IsWindowVisible, GA_ROOT, GWL_EXSTYLE,
        GWL_STYLE, WS_CAPTION, WS_EX_TOOLWINDOW,
    };

    fn class_name(hwnd: HWND) -> String {
        let mut buf = [0u16; 256];
        let n = unsafe { GetClassNameW(hwnd, &mut buf) };
        String::from_utf16_lossy(&buf[..n as usize])
    }

    fn root_of(hwnd: HWND) -> HWND {
        unsafe { GetAncestor(hwnd, GA_ROOT) }
    }

    fn is_excluded(hwnd: HWND, self_hwnd: Option<isize>) -> bool {
        if hwnd.0.is_null() {
            return true;
        }
        if let Some(me) = self_hwnd {
            if hwnd.0 as isize == me {
                return true;
            }
        }
        unsafe {
            if !IsWindow(hwnd).as_bool()
                || !IsWindowVisible(hwnd).as_bool()
                || IsIconic(hwnd).as_bool()
            {
                return true;
            }
        }
        matches!(
            class_name(hwnd).as_str(),
            "WindowHubAppBarHost"
            | "WindowHubDockAppBarHost"
                | "Shell_TrayWnd"
                | "Shell_SecondaryTrayWnd"
                | "Progman"
                | "WorkerW"
                | "ForegroundStaging"
                | "Windows.UI.Core.CoreWindow"
                // Native tray overflow flyout — opening it on tray click must NOT
                // look like a game fullscreen and hide our menubar.
                | "NotifyIconOverflowWindow"
                | "TopLevelWindowForOverflowXamlIsland"
        ) || {
            let c = class_name(hwnd);
            c.contains("Overflow") || c.contains("NotifyIcon")
        }
    }

    /// True fullscreen ≈ covers the physical monitor (`rcMonitor`), not just work area.
    /// Maximized apps usually stop at the taskbar (`rcWork`) and should NOT hide the island.
    fn covers_rect(hwnd: HWND, target: RECT) -> bool {
        unsafe {
            let mut wr = RECT::default();
            if GetWindowRect(hwnd, &mut wr).is_err() {
                return false;
            }
            let tw = (target.right - target.left).max(1);
            let th = (target.bottom - target.top).max(1);
            let ww = (wr.right - wr.left).max(0);
            let wh = (wr.bottom - wr.top).max(0);

            // Near full coverage (95% — some borderless games leave a thin inset).
            if ww * 100 < tw * 95 || wh * 100 < th * 95 {
                return false;
            }
            // Anchored to target origin (allow a few px for exclusive-mode quirks).
            if (wr.left - target.left).abs() > 8 || (wr.top - target.top).abs() > 8 {
                return false;
            }
            true
        }
    }

    fn covers_physical_monitor(hwnd: HWND) -> bool {
        unsafe {
            let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(mon, &mut info).as_bool() {
                return false;
            }
            covers_rect(hwnd, info.rcMonitor)
        }
    }

    /// Game-like fullscreen: fills the **physical** monitor (`rcMonitor`).
    ///
    /// Maximized apps (WPS / browsers / IDEs — often borderless custom chrome) stop at
    /// `rcWork` under our top AppBar and must NOT hide the island/dock.
    /// Exclusive games often also set `WS_MAXIMIZE` / `IsZoomed` while covering
    /// `rcMonitor` without a caption — those still count as game FS.
    fn is_game_fullscreen(hwnd: HWND) -> bool {
        unsafe {
            if !covers_physical_monitor(hwnd) {
                return false;
            }
            let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
            let has_caption = style & WS_CAPTION.0 != 0;
            // Titled maximize that reaches rcMonitor (auto-hidden taskbar) — keep chrome.
            if has_caption {
                return false;
            }
            // Borderless covering the physical monitor = exclusive / game FS.
            true
        }
    }

    fn same_monitor(a: HWND, b: HWND) -> bool {
        unsafe {
            MonitorFromWindow(a, MONITOR_DEFAULTTONEAREST).0
                == MonitorFromWindow(b, MONITOR_DEFAULTTONEAREST).0
        }
    }

    /// Grace period after menubar tray clicks — overflow / app popups must not
    /// trip the fullscreen hide path (status bar vanishing).
    static TRAY_CLICK_GRACE_UNTIL_MS: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    pub fn note_tray_interaction() {
        let until = now_ms().saturating_add(2_500);
        let _ = TRAY_CLICK_GRACE_UNTIL_MS.fetch_max(until, std::sync::atomic::Ordering::SeqCst);
    }

    fn in_tray_click_grace() -> bool {
        now_ms() < TRAY_CLICK_GRACE_UNTIL_MS.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Foreground window is game-like fullscreen on the same monitor as our strip.
    pub fn should_hide_strip(self_hwnd: Option<isize>) -> bool {
        if in_tray_click_grace() {
            return false;
        }
        unsafe {
            let fg = root_of(GetForegroundWindow());
            if is_excluded(fg, self_hwnd) {
                return false;
            }
            let ex = GetWindowLongW(fg, GWL_EXSTYLE) as u32;
            if ex & WS_EX_TOOLWINDOW.0 != 0 {
                return false;
            }
            if let Some(me) = self_hwnd {
                let mine = HWND(me as *mut _);
                if !same_monitor(fg, mine) {
                    return false;
                }
            }
            is_game_fullscreen(fg)
        }
    }

    /// Also true if any top z-order window is exclusive FS on our monitor while focused there.
    /// Kept for potential future use; foreground check is the primary path.
    #[allow(dead_code)]
    pub fn any_exclusive_fullscreen(self_hwnd: Option<isize>) -> bool {
        struct Ctx {
            self_hwnd: Option<isize>,
            found: bool,
        }
        let ctx = Box::new(Ctx {
            self_hwnd,
            found: false,
        });
        let ptr = Box::into_raw(ctx);

        unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let ctx = &mut *(lparam.0 as *mut Ctx);
            if ctx.found {
                return BOOL(0);
            }
            if is_excluded(hwnd, ctx.self_hwnd) {
                return BOOL(1);
            }
            let root = root_of(hwnd);
            if is_excluded(root, ctx.self_hwnd) {
                return BOOL(1);
            }
            if let Some(me) = ctx.self_hwnd {
                if !same_monitor(root, HWND(me as *mut _)) {
                    return BOOL(1);
                }
            }
            if is_game_fullscreen(root) {
                ctx.found = true;
                return BOOL(0);
            }
            BOOL(1)
        }

        unsafe {
            let _ = EnumWindows(Some(cb), LPARAM(ptr as isize));
            let ctx = Box::from_raw(ptr);
            ctx.found
        }
    }
}

#[cfg(windows)]
pub use win::{note_tray_interaction, should_hide_strip};

#[cfg(not(windows))]
pub fn should_hide_strip(_self_hwnd: Option<isize>) -> bool {
    false
}

#[cfg(not(windows))]
pub fn note_tray_interaction() {}
