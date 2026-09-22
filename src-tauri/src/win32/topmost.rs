//! Main island HWND Z-order helpers.
//!
//! The strip must stay `HWND_TOPMOST` on the desktop so wallpaper engines /
//! Show-Desktop churn cannot bury it. Tray flyouts briefly call [`yield_for`].
//! Game fullscreen uses `HIDDEN_FOR_FULLSCREEN` + `hide()` instead of clearing Z-order.

use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Main island HWND (set once from setup).
static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);

/// Deadline (ms since unix epoch) while we must not re-apply TOPMOST (tray yield).
static YIELD_UNTIL_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn set_main_hwnd(hwnd: isize) {
    MAIN_HWND.store(hwnd, Ordering::SeqCst);
}

pub fn set_overlay_raised(_raised: bool) {
    // Flag retained for API compat. Do NOT SetWindowPos here — callers often
    // invoke this in the middle of a resize (deadlocks WebView2).
}

/// Drop TOPMOST for `ms` (and clear immediately) so tray flyouts aren't covered.
pub fn yield_for(ms: u64) {
    let until = now_ms().saturating_add(ms);
    let _ = YIELD_UNTIL_MS.fetch_max(until, Ordering::SeqCst);
    let hwnd = MAIN_HWND.load(Ordering::SeqCst);
    if hwnd != 0 {
        clear_topmost(hwnd);
    }
}

pub fn is_yielding() -> bool {
    now_ms() < YIELD_UNTIL_MS.load(Ordering::SeqCst)
}

/// Re-apply TOPMOST after AppBar / material watchdog ticks (skip during tray yield).
pub fn reassert_main_zorder() {
    let hwnd = MAIN_HWND.load(Ordering::SeqCst);
    if hwnd == 0 || is_yielding() {
        return;
    }
    force_topmost(hwnd);
}

/// Win11 三指下滑「显示桌面」会把岛窗最小化/隐去；非全屏隐藏期间必须拉回。
#[cfg(windows)]
pub fn ensure_main_visible() -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, IsWindow, IsWindowVisible, ShowWindow, SW_RESTORE, SW_SHOWNOACTIVATE,
    };

    let raw = MAIN_HWND.load(Ordering::SeqCst);
    if raw == 0 {
        return false;
    }
    let hwnd = HWND(raw as *mut _);
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return false;
        }
        let iconic = IsIconic(hwnd).as_bool();
        let visible = IsWindowVisible(hwnd).as_bool();
        let mut cloaked: u32 = 0;
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut _,
            std::mem::size_of::<u32>() as u32,
        );
        if !iconic && visible && cloaked == 0 {
            return false;
        }
        if iconic {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        force_topmost(raw);
        true
    }
}

#[cfg(not(windows))]
pub fn ensure_main_visible() -> bool {
    false
}

#[cfg(windows)]
pub fn force_topmost(hwnd_raw: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    };

    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
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
}

#[cfg(windows)]
pub fn clear_topmost(hwnd_raw: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_NOTOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };

    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            HWND_NOTOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

#[cfg(not(windows))]
pub fn force_topmost(_hwnd_raw: isize) {}

#[cfg(not(windows))]
pub fn clear_topmost(_hwnd_raw: isize) {}
