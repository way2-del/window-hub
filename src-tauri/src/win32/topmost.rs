//! Main island HWND helpers. The strip is reserved via AppBar — do **not** fight
//! other shells for `HWND_TOPMOST`; yielding clears leftover TOPMOST if any.

use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Main island HWND (set once from setup).
static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);

/// Deadline (ms since unix epoch) while we must not re-apply TOPMOST (legacy).
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

/// No-op: island no longer claims TOPMOST (AppBar owns the strip).
#[cfg(windows)]
pub fn force_topmost(_hwnd_raw: isize) {}

/// Drop TOPMOST if still set (e.g. leftover from older builds / tray yield).
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
