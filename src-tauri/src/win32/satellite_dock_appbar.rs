//! Per-monitor bottom AppBar for secondary docks.
//!
//! Uses a **dedicated invisible host HWND** per dock-sat (same idea as
//! `dock_appbar.rs`) so `ABM_SETPOS` never moves the Dock WebView — and never
//! nudges the top chrome AppBar down by one strip height.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[cfg(windows)]
mod win {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    use windows::core::w;
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::Shell::{
        SHAppBarMessage, ABE_BOTTOM, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, APPBARDATA,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, IsWindow, LoadCursorW, MoveWindow,
        RegisterClassW, SetLayeredWindowAttributes, ShowWindow, CS_HREDRAW, CS_VREDRAW, IDC_ARROW,
        LWA_ALPHA, SW_SHOWNOACTIVATE, WM_CREATE, WM_DESTROY, WNDCLASSW, WS_EX_LAYERED,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
    };

    /// Match `dock_appbar::STRIP_LOGICAL_H` / `dock::DOCK_H`.
    const STRIP_LOGICAL_H: i32 = 52;
    const CLASS_NAME: windows::core::PCWSTR = w!("WindowHubDockSatAppBarHost");

    /// dock-sat hwnd → host hwnd
    fn hosts() -> &'static Mutex<HashMap<isize, isize>> {
        static MAP: OnceLock<Mutex<HashMap<isize, isize>>> = OnceLock::new();
        MAP.get_or_init(|| Mutex::new(HashMap::new()))
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

    fn desired_strip(anchor: HWND, bottom_offset_px: u32) -> Option<RECT> {
        let mon = monitor_rect(anchor)?;
        let scale = dpi_scale(anchor);
        let h = (STRIP_LOGICAL_H as f64 * scale).round().max(1.0) as i32;
        let margin = (bottom_offset_px as f64 * scale).round().max(0.0) as i32;
        Some(RECT {
            left: mon.left,
            top: mon.bottom - margin - h,
            right: mon.right,
            bottom: mon.bottom - margin,
        })
    }

    fn abd(hwnd: HWND, rc: RECT) -> APPBARDATA {
        APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            hWnd: hwnd,
            uCallbackMessage: 0,
            uEdge: ABE_BOTTOM,
            rc,
            lParam: LPARAM(0),
        }
    }

    unsafe extern "system" fn host_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_DESTROY || msg == WM_CREATE {
            return if msg == WM_CREATE {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            };
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

    fn create_host(anchor: HWND, rc: RECT) -> Option<HWND> {
        ensure_class();
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
                CLASS_NAME,
                w!(""),
                WS_POPUP,
                rc.left,
                rc.top,
                (rc.right - rc.left).max(1),
                (rc.bottom - rc.top).max(1),
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
            let mut data = abd(hwnd, RECT::default());
            let ok = SHAppBarMessage(ABM_NEW, &mut data);
            if ok == 0 {
                let _ = DestroyWindow(hwnd);
                return None;
            }
        }
        let _ = anchor;
        Some(hwnd)
    }

    fn apply_pos(host: HWND, anchor: HWND, bottom_offset_px: u32) {
        let Some(desired) = desired_strip(anchor, bottom_offset_px) else {
            return;
        };
        let h = (desired.bottom - desired.top).max(1);
        let mut data = abd(host, desired);
        unsafe {
            SHAppBarMessage(ABM_QUERYPOS, &mut data);
            // Keep the physical bottom edge. QUERYPOS can report rcWork after a
            // top AppBar claim and would otherwise grow the bottom strip upward.
            data.rc.left = desired.left;
            data.rc.right = desired.right;
            data.rc.bottom = desired.bottom;
            data.rc.top = desired.bottom - h;
            SHAppBarMessage(ABM_SETPOS, &mut data);
            let _ = MoveWindow(
                host,
                desired.left,
                desired.bottom - h,
                (desired.right - desired.left).max(1),
                h,
                false,
            );
        }
    }

    pub fn register_and_sync(dock_hwnd_raw: isize, bottom_offset_px: u32) {
        let anchor = HWND(dock_hwnd_raw as *mut _);
        unsafe {
            if !IsWindow(anchor).as_bool() {
                return;
            }
        }
        let Some(desired) = desired_strip(anchor, bottom_offset_px) else {
            return;
        };
        let mut map = hosts().lock().unwrap_or_else(|e| e.into_inner());
        let host = if let Some(&raw) = map.get(&dock_hwnd_raw) {
            let h = HWND(raw as *mut _);
            if unsafe { IsWindow(h).as_bool() } {
                h
            } else {
                map.remove(&dock_hwnd_raw);
                let Some(created) = create_host(anchor, desired) else {
                    return;
                };
                map.insert(dock_hwnd_raw, created.0 as isize);
                created
            }
        } else {
            let Some(created) = create_host(anchor, desired) else {
                return;
            };
            map.insert(dock_hwnd_raw, created.0 as isize);
            created
        };
        apply_pos(host, anchor, bottom_offset_px);
    }

    pub fn unregister(dock_hwnd_raw: isize) {
        let mut map = hosts().lock().unwrap_or_else(|e| e.into_inner());
        let Some(raw) = map.remove(&dock_hwnd_raw) else {
            return;
        };
        let host = HWND(raw as *mut _);
        unsafe {
            if IsWindow(host).as_bool() {
                let mut data = abd(host, RECT::default());
                let _ = SHAppBarMessage(ABM_REMOVE, &mut data);
                let _ = DestroyWindow(host);
            }
        }
    }

    pub fn unregister_all() {
        let mut map = hosts().lock().unwrap_or_else(|e| e.into_inner());
        let pairs: Vec<(isize, isize)> = map.drain().collect();
        for (_dock, host_raw) in pairs {
            let host = HWND(host_raw as *mut _);
            unsafe {
                if IsWindow(host).as_bool() {
                    let mut data = abd(host, RECT::default());
                    let _ = SHAppBarMessage(ABM_REMOVE, &mut data);
                    let _ = DestroyWindow(host);
                }
            }
        }
    }
}

#[cfg(windows)]
pub use win::{register_and_sync, unregister, unregister_all};

#[cfg(not(windows))]
pub fn register_and_sync(_hwnd_raw: isize, _bottom_offset_px: u32) {}

#[cfg(not(windows))]
pub fn unregister(_hwnd_raw: isize) {}

#[cfg(not(windows))]
pub fn unregister_all() {}
