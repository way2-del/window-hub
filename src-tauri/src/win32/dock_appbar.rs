//! Reserve the bottom work area via `SHAppBarMessage` (ABE_BOTTOM).
//!
//! Strip height is resting Dock chrome (`DOCK_H`) only — hover magnification
//! must NEVER enlarge the AppBar / work area (same rule as the top island strip).

#[cfg(windows)]
mod win {
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use std::sync::mpsc::{self, Sender};
    use std::sync::Mutex;
    use std::thread;
    use std::time::Duration;

    use windows::core::w;
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::Shell::{
        SHAppBarMessage, ABE_BOTTOM, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS,
        ABM_WINDOWPOSCHANGED, ABN_POSCHANGED, APPBARDATA,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, LoadCursorW, MoveWindow,
        PeekMessageW, RegisterClassW, SetLayeredWindowAttributes, SetWindowPos, ShowWindow,
        TranslateMessage, CS_HREDRAW, CS_VREDRAW, HWND_TOPMOST, IDC_ARROW, LWA_ALPHA, MSG,
        PM_REMOVE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_SHOWNOACTIVATE,
        WM_CREATE, WM_DESTROY, WM_QUIT, WM_USER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
    };

    /// Logical Dock chrome height (= `dock::DOCK_H`).
    pub const STRIP_LOGICAL_H: i32 = 52;

    const APPBAR_CALLBACK: u32 = WM_USER + 78;
    const CLASS_NAME: windows::core::PCWSTR = w!("WindowHubDockAppBarHost");

    enum Cmd {
        Ensure { anchor: isize },
        Sync { anchor: isize },
        Suspend,
        Shutdown,
    }

    static TX: Mutex<Option<Sender<Cmd>>> = Mutex::new(None);
    static REGISTERED: AtomicBool = AtomicBool::new(false);
    static LAST_RC: Mutex<Option<RECT>> = Mutex::new(None);
    static STRIP_PX: AtomicI32 = AtomicI32::new(0);

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
            top: mon.bottom - h,
            right: mon.right,
            bottom: mon.bottom,
        })
    }

    unsafe extern "system" fn host_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == APPBAR_CALLBACK {
            if lparam.0 as u32 == ABN_POSCHANGED {
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
            uEdge: ABE_BOTTOM,
            rc,
            lParam: LPARAM(0),
        }
    }

    fn apply_pos(host: HWND, anchor: Option<HWND>, force: bool) -> bool {
        let probe = anchor.unwrap_or(host);
        let Some(mut rc) = desired_strip(probe) else {
            return false;
        };

        if !force {
            if let Ok(guard) = LAST_RC.lock() {
                if let Some(prev) = *guard {
                    if rect_eq(&prev, &rc) {
                        return true;
                    }
                }
            }
        }

        let mut data = abd_for(host, rc);
        unsafe {
            SHAppBarMessage(ABM_QUERYPOS, &mut data);
            rc = data.rc;
            let h = STRIP_PX.load(Ordering::SeqCst).max(1);
            // Fixed chrome height — shell negotiation must not grow/shrink forever.
            rc.top = rc.bottom - h;
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
            let mut changed = abd_for(host, rc);
            SHAppBarMessage(ABM_WINDOWPOSCHANGED, &mut changed);
        }

        if let Ok(mut guard) = LAST_RC.lock() {
            *guard = Some(rc);
        }
        true
    }

    fn create_and_register(anchor: HWND) -> Option<HWND> {
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
            return;
        }
        send(Cmd::Sync {
            anchor: anchor_hwnd_raw,
        });
    }

    pub fn suspend() {
        send(Cmd::Suspend);
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

    pub fn strip_height_px() -> i32 {
        let h = STRIP_PX.load(Ordering::SeqCst);
        if h > 0 {
            h
        } else {
            STRIP_LOGICAL_H
        }
    }

    pub fn is_registered() -> bool {
        REGISTERED.load(Ordering::SeqCst)
    }
}

#[cfg(windows)]
pub use win::*;

#[cfg(not(windows))]
pub const STRIP_LOGICAL_H: i32 = 52;

#[cfg(not(windows))]
pub fn register(_hwnd_raw: isize) {}

#[cfg(not(windows))]
pub fn sync(_hwnd_raw: isize) {}

#[cfg(not(windows))]
pub fn suspend() {}

#[cfg(not(windows))]
pub fn restore() {}

#[cfg(not(windows))]
pub fn strip_height_px() -> i32 {
    STRIP_LOGICAL_H
}

#[cfg(not(windows))]
pub fn is_registered() -> bool {
    false
}
