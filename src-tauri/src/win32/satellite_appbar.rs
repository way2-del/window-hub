//! Per-monitor AppBar registration for secondary chrome satellite windows.
//! Main island AppBar stays in `appbar.rs` (singleton). Satellites use this map.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[cfg(windows)]
mod win {
    use super::*;
    use windows::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::Shell::{
        SHAppBarMessage, ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, APPBARDATA,
    };
    use windows::Win32::UI::WindowsAndMessaging::IsWindow;

    const STRIP_LOGICAL_H: i32 = 28;

    fn strip_logical_h() -> i32 {
        crate::chrome_prefs::bar_height_logical().max(STRIP_LOGICAL_H)
    }

    fn registry() -> &'static Mutex<HashMap<isize, bool>> {
        static REG: OnceLock<Mutex<HashMap<isize, bool>>> = OnceLock::new();
        REG.get_or_init(|| Mutex::new(HashMap::new()))
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

    fn desired_strip(hwnd: HWND) -> Option<RECT> {
        let mon = monitor_rect(hwnd)?;
        let h = (strip_logical_h() as f64 * dpi_scale(hwnd)).round().max(1.0) as i32;
        Some(RECT {
            left: mon.left,
            top: mon.top,
            right: mon.right,
            bottom: mon.top + h,
        })
    }

    fn abd(hwnd: HWND, rc: RECT) -> APPBARDATA {
        APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            hWnd: hwnd,
            uCallbackMessage: 0,
            uEdge: ABE_TOP,
            rc,
            lParam: LPARAM(0),
        }
    }

    pub fn register_and_sync(hwnd_raw: isize) {
        let hwnd = HWND(hwnd_raw as *mut _);
        unsafe {
            if !IsWindow(hwnd).as_bool() {
                return;
            }
            let Some(desired) = desired_strip(hwnd) else {
                return;
            };
            let mut map = registry().lock().unwrap_or_else(|e| e.into_inner());
            let already = map.get(&hwnd_raw).copied().unwrap_or(false);
            if !already {
                let mut data = abd(hwnd, desired);
                let _ = SHAppBarMessage(ABM_NEW, &mut data);
                map.insert(hwnd_raw, true);
            }
            let mut data = abd(hwnd, desired);
            let _ = SHAppBarMessage(ABM_QUERYPOS, &mut data);
            let _ = SHAppBarMessage(ABM_SETPOS, &mut data);
        }
    }

    pub fn unregister(hwnd_raw: isize) {
        let hwnd = HWND(hwnd_raw as *mut _);
        let mut map = registry().lock().unwrap_or_else(|e| e.into_inner());
        if map.remove(&hwnd_raw).is_none() {
            return;
        }
        unsafe {
            if !IsWindow(hwnd).as_bool() {
                return;
            }
            let mut data = abd(hwnd, RECT::default());
            let _ = SHAppBarMessage(ABM_REMOVE, &mut data);
        }
    }
}

#[cfg(windows)]
pub use win::{register_and_sync, unregister};

#[cfg(not(windows))]
pub fn register_and_sync(_hwnd_raw: isize) {}

#[cfg(not(windows))]
pub fn unregister(_hwnd_raw: isize) {}
