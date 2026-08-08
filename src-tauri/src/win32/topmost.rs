//! Keep the island above other always-on-top shells (e.g. MyDockFinder).
//! Temporarily yield TOPMOST so tray context menus / flyouts are not covered.

use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Main island HWND (set once from setup).
static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);

/// Deadline (ms since unix epoch) while we must not steal Z-order.
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

/// Pause forced topmost for `ms` so another app's popup can stay above the island.
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

#[cfg(windows)]
pub fn force_topmost(hwnd_raw: isize) {
    if is_yielding() {
        return;
    }
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
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
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// Drop TOPMOST briefly so tray menus can stack above the island.
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
