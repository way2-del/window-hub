//! Reserve the top work area via the official AppBar API (`SHAppBarMessage`).
//!
//! The reserved strip is ALWAYS `STRIP_LOGICAL_H` (collapsed island height).
//! Expanding the island / settings popup must NEVER enlarge the AppBar rect or
//! call `SPI_SETWORKAREA`. The visible main window does **not** fight TOPMOST —
//! the strip is owned by work-area reservation, not Z-order.
//!
//! Stability rules:
//! - Claim once at reveal; do **not** periodically re-SETPOS / SPI-rewrite.
//! - Re-SETPOS only when desired geometry changes (monitor/DPI) or `force_sync`
//!   (boot/dock settle) / exclusive-fullscreen suspend→restore.
//! - Never poll `SPI_SETWORKAREA` on the quiet path.
//! - Helper HWND owns the claim; the visible Tauri window may be taller.

#[cfg(windows)]
mod win {
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
    use std::sync::mpsc::{self, Sender};
    use std::sync::Mutex;
    use std::thread;
    use std::time::Duration;

    use crate::win32::work_area;

    use windows::core::w;
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::Shell::{
        SHAppBarMessage, ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, ABN_POSCHANGED,
        APPBARDATA,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, LoadCursorW,
        MoveWindow, PeekMessageW, RegisterClassW, SetLayeredWindowAttributes, SetWindowPos,
        ShowWindow, SystemParametersInfoW, TranslateMessage, CS_HREDRAW, CS_VREDRAW,
        HWND_TOPMOST, IDC_ARROW, LWA_ALPHA, MSG, PM_REMOVE, SPI_GETWORKAREA, SPI_SETWORKAREA,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_SHOWNOACTIVATE,
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WM_CREATE, WM_DESTROY, WM_QUIT, WM_USER, WNDCLASSW,
        WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
    };

    /// Logical island strip height (= capsule / bar height; flush to screen top).
    pub const STRIP_LOGICAL_H: i32 = 28;

    const APPBAR_CALLBACK: u32 = WM_USER + 77;
    const CLASS_NAME: windows::core::PCWSTR = w!("WindowHubAppBarHost");

    enum Cmd {
        /// Anchor HWND used to pick monitor / DPI.
        Ensure { anchor: isize },
        Sync { anchor: isize },
        /// Re-SETPOS even when LAST_DESIRED matches (post-dock / taskbar wipe reclaim).
        ForceSync { anchor: isize },
        /// Temporarily drop AppBar claim (e.g. exclusive fullscreen game) without killing worker.
        Suspend,
        Shutdown,
    }

    static TX: Mutex<Option<Sender<Cmd>>> = Mutex::new(None);
    static REGISTERED: AtomicBool = AtomicBool::new(false);
    /// Last *intended* strip (pre-negotiation). Early-out must use this — comparing
    /// shell-tweaked `LAST_RC` never matches `desired_strip` and loops SETPOS↔ABN.
    static LAST_DESIRED: Mutex<Option<RECT>> = Mutex::new(None);
    static LAST_RC: Mutex<Option<RECT>> = Mutex::new(None);
    static STRIP_PX: AtomicI32 = AtomicI32::new(0);
    static SPI_CLEARED: AtomicBool = AtomicBool::new(false);
    /// Rate-limit hard reclaim (SETPOS + notified SPI) when shell drops the inset.
    static LAST_HARD_RECLAIM_MS: AtomicU64 = AtomicU64::new(0);

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn dpi_scale(hwnd: HWND) -> f64 {
        unsafe {
            let dpi = GetDpiForWindow(hwnd);
            if dpi == 0 {
                1.0
            } else {
                dpi as f64 / 96.0
            }
        }
    }

    fn monitor_rect(hwnd: HWND) -> Option<RECT> {
        unsafe {
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(monitor, &mut info).as_bool() {
                Some(info.rcMonitor)
            } else {
                None
            }
        }
    }

    fn rect_eq(a: &RECT, b: &RECT) -> bool {
        a.left == b.left && a.top == b.top && a.right == b.right && a.bottom == b.bottom
    }

    fn desired_strip(anchor: HWND) -> Option<RECT> {
        let mon = monitor_rect(anchor)?;
        let h = (STRIP_LOGICAL_H as f64 * dpi_scale(anchor))
            .round()
            .max(1.0) as i32;
        STRIP_PX.store(h, Ordering::SeqCst);
        Some(RECT {
            left: mon.left,
            top: mon.top,
            right: mon.right,
            bottom: mon.top + h,
        })
    }

    /// `SPI_GETWORKAREA` top inset for `anchor`'s monitor (physical px).
    fn work_area_top_inset(anchor: HWND) -> Option<i32> {
        let mon = monitor_rect(anchor)?;
        let mut wa = RECT::default();
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETWORKAREA,
                0,
                Some(&mut wa as *mut RECT as *mut _),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        };
        if ok.is_err() {
            return None;
        }
        Some(wa.top - mon.top)
    }

    /// True when Explorer actually reserved our strip (taskbar autohide / dock
    /// can wipe rcWork after a successful SETPOS while LAST_DESIRED still matches).
    fn shell_honors_strip(anchor: HWND) -> bool {
        let h = STRIP_PX
            .load(Ordering::SeqCst)
            .max(STRIP_LOGICAL_H)
            .max(1);
        match work_area_top_inset(anchor) {
            Some(inset) => inset >= h.saturating_sub(4) && inset <= h + 20,
            None => false,
        }
    }

    /// Win11 + auto-hide taskbar often leaves `rcWork.top` at monitor top even
    /// after a successful `ABM_SETPOS`. Fall back to SPI inset.
    /// `notify=true` sends `SPIF_SENDCHANGE` so maximized windows re-layout under the strip.
    fn apply_spi_top_inset(anchor: HWND, strip_bottom: i32, notify: bool) {
        use windows::Win32::UI::WindowsAndMessaging::SPIF_SENDCHANGE;

        let Some(mon) = monitor_rect(anchor) else {
            return;
        };
        let mut wa = RECT::default();
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETWORKAREA,
                0,
                Some(&mut wa as *mut RECT as *mut _),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        };
        if ok.is_err() {
            return;
        }
        let target_top = strip_bottom.clamp(mon.top + 1, mon.bottom - 100);
        if (wa.top - target_top).abs() <= 2 {
            return;
        }
        wa.top = target_top;
        if wa.bottom - wa.top < 100 {
            return;
        }
        let flags = if notify {
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(SPIF_SENDCHANGE.0)
        } else {
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)
        };
        let _ = unsafe {
            SystemParametersInfoW(
                SPI_SETWORKAREA,
                0,
                Some(&mut wa as *mut RECT as *mut _),
                flags,
            )
        };
    }

    /// One-shot: undo leftover SPI top inset from older builds.
    /// No `SPIF_SENDCHANGE` — broadcasting here then immediately `ABM_SETPOS`
    /// makes maximized windows resize twice (startup flicker).
    fn clear_legacy_spi_inset(anchor: HWND) {
        if SPI_CLEARED.swap(true, Ordering::SeqCst) {
            return;
        }
        let Some(mon) = monitor_rect(anchor) else {
            return;
        };
        let mut wa = RECT::default();
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETWORKAREA,
                0,
                Some(&mut wa as *mut RECT as *mut _),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        };
        if ok.is_err() {
            return;
        }
        let our = (STRIP_LOGICAL_H as f64 * dpi_scale(anchor)).round() as i32;
        let inset = wa.top - mon.top;
        if inset < our.saturating_sub(8) || inset > our + 12 {
            return;
        }
        wa.top = mon.top;
        if wa.bottom - wa.top < 100 {
            return;
        }
        let _ = unsafe {
            SystemParametersInfoW(
                SPI_SETWORKAREA,
                0,
                Some(&mut wa as *mut RECT as *mut _),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        };
    }

    unsafe extern "system" fn host_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == APPBAR_CALLBACK {
            if lparam.0 as u32 == ABN_POSCHANGED {
                if work_area::work_area_quiet() {
                    return LRESULT(0);
                }
                // Other AppBars / taskbar moved — only SETPOS if *our* desired
                // strip changed. Unconditional apply_pos caused SETPOS↔ABN loops
                // that thrash maximized windows' work area.
                let _ = apply_pos(hwnd, None, false);
            }
            return LRESULT(0);
        }
        if msg == WM_DESTROY {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        if msg == WM_CREATE {
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    fn ensure_class() {
        static ONCE: AtomicBool = AtomicBool::new(false);
        if ONCE.swap(true, Ordering::SeqCst) {
            return;
        }
        unsafe {
            let wc = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(host_wnd_proc),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: CLASS_NAME,
                ..Default::default()
            };
            let _ = RegisterClassW(&wc);
        }
    }

    fn abd_for(hwnd: HWND, rc: RECT) -> APPBARDATA {
        APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            hWnd: hwnd,
            uCallbackMessage: APPBAR_CALLBACK,
            uEdge: ABE_TOP,
            rc,
            lParam: LPARAM(0),
        }
    }

    fn apply_pos(host: HWND, anchor: Option<HWND>, force: bool) -> bool {
        let probe = anchor.unwrap_or(host);
        let Some(desired) = desired_strip(probe) else {
            return false;
        };

        if !force {
            if work_area::work_area_quiet() {
                return true;
            }
            if let Ok(guard) = LAST_DESIRED.lock() {
                if let Some(prev) = *guard {
                    if rect_eq(&prev, &desired) {
                        drop(guard);
                        if shell_honors_strip(probe) {
                            return true;
                        }
                        // Shell dropped the inset (common after maximize / autohide).
                        // Soft SPI first; rate-limited hard SETPOS+notify so placeholder
                        // returns without permanent thrash.
                        apply_spi_top_inset(probe, desired.bottom, true);
                        if shell_honors_strip(probe) {
                            return true;
                        }
                        let now = now_ms();
                        let last = LAST_HARD_RECLAIM_MS.load(Ordering::SeqCst);
                        if now.saturating_sub(last) < 2_500 {
                            return true;
                        }
                        LAST_HARD_RECLAIM_MS.store(now, Ordering::SeqCst);
                        // Fall through to ABM_SETPOS once.
                    }
                }
            }
        }

        let mut rc = desired;
        let mut data = abd_for(host, rc);
        unsafe {
            SHAppBarMessage(ABM_QUERYPOS, &mut data);
            rc = data.rc;
            let h = STRIP_PX.load(Ordering::SeqCst).max(1);
            // Keep a fixed strip height so shell negotiation cannot enlarge/shrink forever.
            rc.bottom = rc.top + h;
            data.rc = rc;
            SHAppBarMessage(ABM_SETPOS, &mut data);
            rc = data.rc;

            let _ = MoveWindow(
                host,
                rc.left,
                rc.top,
                (rc.right - rc.left).max(1),
                (rc.bottom - rc.top).max(1),
                false,
            );
            // Do NOT call ABM_WINDOWPOSCHANGED here — it re-broadcasts ABN_POSCHANGED
            // to every AppBar (including dock) and amplifies work-area flicker.
        }

        // Prefer SPI inset whenever shell failed to honor ABM.
        // Force / hard-reclaim path notifies so maximized windows reflow under the strip.
        if !shell_honors_strip(probe) {
            apply_spi_top_inset(probe, desired.bottom, force);
        }

        if let Ok(mut guard) = LAST_DESIRED.lock() {
            *guard = Some(desired);
        }
        if let Ok(mut guard) = LAST_RC.lock() {
            *guard = Some(rc);
        }
        true
    }

    fn create_and_register(anchor: HWND) -> Option<HWND> {
        // Do NOT clear_legacy_spi_inset here — wiping a working SPI strip before
        // ABM on Win11 autohide leaves maximized windows with no top inset.
        ensure_class();
        let rc = desired_strip(anchor)?;
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
                CLASS_NAME,
                w!(""),
                WS_POPUP,
                rc.left,
                rc.top,
                rc.right - rc.left,
                rc.bottom - rc.top,
                None,
                None,
                None,
                None,
            )
        }
        .ok()?;

        unsafe {
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 1, LWA_ALPHA);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }

        let mut data = abd_for(hwnd, RECT::default());
        let ok = unsafe { SHAppBarMessage(ABM_NEW, &mut data) };
        if ok == 0 {
            let _ = unsafe { DestroyWindow(hwnd) };
            return None;
        }

        REGISTERED.store(true, Ordering::SeqCst);
        let _ = apply_pos(hwnd, Some(anchor), true);
        Some(hwnd)
    }

    fn remove_host(host: HWND) {
        if !REGISTERED.swap(false, Ordering::SeqCst) {
            let _ = unsafe { DestroyWindow(host) };
            return;
        }
        let mut data = abd_for(host, RECT::default());
        unsafe {
            SHAppBarMessage(ABM_REMOVE, &mut data);
            let _ = DestroyWindow(host);
        }
        if let Ok(mut guard) = LAST_RC.lock() {
            *guard = None;
        }
        if let Ok(mut guard) = LAST_DESIRED.lock() {
            *guard = None;
        }
    }

    fn pump_once() {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    break;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    fn worker_main(rx: mpsc::Receiver<Cmd>) {
        let mut host: Option<HWND> = None;

        loop {
            // Drain commands + pump so ABN_* arrives without busy SPI rewriting.
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Cmd::Ensure { anchor }) => {
                    let anchor = HWND(anchor as *mut _);
                    if host.is_none() {
                        host = create_and_register(anchor);
                    } else if let Some(h) = host {
                        let _ = apply_pos(h, Some(anchor), false);
                    }
                }
                Ok(Cmd::Sync { anchor }) => {
                    if let Some(h) = host {
                        let _ = apply_pos(h, Some(HWND(anchor as *mut _)), false);
                    }
                }
                Ok(Cmd::ForceSync { anchor }) => {
                    if let Some(h) = host {
                        let _ = apply_pos(h, Some(HWND(anchor as *mut _)), true);
                    }
                }
                Ok(Cmd::Suspend) => {
                    if let Some(h) = host.take() {
                        remove_host(h);
                    }
                }
                Ok(Cmd::Shutdown) => {
                    if let Some(h) = host.take() {
                        remove_host(h);
                    }
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if let Some(h) = host.take() {
                        remove_host(h);
                    }
                    break;
                }
            }
            pump_once();
        }
    }

    fn ensure_worker() -> Sender<Cmd> {
        let mut guard = TX.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tx) = guard.as_ref() {
            return tx.clone();
        }
        let (tx, rx) = mpsc::channel();
        *guard = Some(tx.clone());
        thread::spawn(move || worker_main(rx));
        tx
    }

    fn send(cmd: Cmd) {
        let _ = ensure_worker().send(cmd);
    }

    pub fn register(anchor_hwnd_raw: isize) {
        send(Cmd::Ensure {
            anchor: anchor_hwnd_raw,
        });
    }

    pub fn sync(anchor_hwnd_raw: isize) {
        if !REGISTERED.load(Ordering::SeqCst) {
            // Not claimed yet — Ensure instead of no-op (boot race).
            register(anchor_hwnd_raw);
            return;
        }
        send(Cmd::Sync {
            anchor: anchor_hwnd_raw,
        });
    }

    /// Force re-SETPOS (ignores LAST_DESIRED early-out). Use after dock/taskbar
    /// settle — they can wipe `rcWork` without changing our desired strip.
    pub fn force_sync(anchor_hwnd_raw: isize) {
        if !REGISTERED.load(Ordering::SeqCst) {
            register(anchor_hwnd_raw);
            return;
        }
        send(Cmd::ForceSync {
            anchor: anchor_hwnd_raw,
        });
    }

    /// Release work-area strip while a fullscreen game is active; call [`register`] to restore.
    pub fn suspend() {
        send(Cmd::Suspend);
    }

    #[allow(dead_code)]
    pub fn set_visual_height_logical(_hwnd_raw: isize, _logical_h: i32) {
        // Intentionally ignored: overlay height must not change AppBar strip.
    }

    #[allow(dead_code)]
    pub fn unregister(_hwnd_raw: isize) {
        restore();
    }

    pub fn restore() {
        let tx = TX
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(tx) = tx {
            let _ = tx.send(Cmd::Shutdown);
        }
    }

    /// Physical pixel height of the reserved top strip (DPI-scaled).
    pub fn strip_height_px() -> i32 {
        let h = STRIP_PX.load(Ordering::SeqCst);
        if h > 0 {
            h
        } else {
            STRIP_LOGICAL_H
        }
    }
}

#[cfg(windows)]
pub use win::*;

#[cfg(not(windows))]
pub const STRIP_LOGICAL_H: i32 = 28;

#[cfg(not(windows))]
pub fn register(_hwnd_raw: isize) {}

#[cfg(not(windows))]
pub fn set_visual_height_logical(_hwnd_raw: isize, _logical_h: i32) {}

#[cfg(not(windows))]
pub fn sync(_hwnd_raw: isize) {}

#[cfg(not(windows))]
pub fn force_sync(_hwnd_raw: isize) {}

#[cfg(not(windows))]
pub fn suspend() {}

#[cfg(not(windows))]
pub fn unregister(_hwnd_raw: isize) {}

#[cfg(not(windows))]
pub fn restore() {}

#[cfg(not(windows))]
pub fn strip_height_px() -> i32 {
    STRIP_LOGICAL_H
}
