//! Main island HWND Z-order helpers.
//!
//! The visible main HWND is a shell AppBar. Both collapsed and expanded states
//! stay TOPMOST so Show Desktop cannot cover the bar before a foreground poll.
//! Fullscreen hiding is owned by the existing fullscreen watcher.
//! Tray flyouts briefly call [`yield_for`] to drop TOPMOST while menus show.

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Main island HWND (set once from setup).
static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);

/// True while the island panel is expanded over the work area (`float_overlay`).
static OVERLAY_RAISED: AtomicBool = AtomicBool::new(false);

/// Temporary TOPMOST while a ChatGPT-family maximize overlaps the strip (clamp failed).
static COVER_TOPMOST: AtomicBool = AtomicBool::new(false);

/// Deadline (ms since unix epoch) while we must not re-apply TOPMOST (tray yield).
static YIELD_UNTIL_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn set_main_hwnd(hwnd: isize) {
    if MAIN_HWND.swap(hwnd, Ordering::SeqCst) != hwnd && hwnd != 0 {
        exclude_main_from_peek(hwnd);
    }
}

/// Run once for the main HWND, not for settings, Dock or popup windows.
#[cfg(windows)]
fn exclude_main_from_peek(raw: isize) {
    use windows::Win32::Foundation::{BOOL, HWND};
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_EXCLUDED_FROM_PEEK};
    let enabled = BOOL(1);
    unsafe {
        let _ = DwmSetWindowAttribute(
            HWND(raw as *mut _),
            DWMWA_EXCLUDED_FROM_PEEK,
            &enabled as *const BOOL as *const _,
            std::mem::size_of::<BOOL>() as u32,
        );
    }
}

#[cfg(not(windows))]
fn exclude_main_from_peek(_raw: isize) {}

pub fn overlay_raised() -> bool {
    OVERLAY_RAISED.load(Ordering::SeqCst)
}

/// Track overlay geometry; the collapsed shell bar also remains TOPMOST.
/// Does **not** SetWindowPos when called mid-resize — only stores the flag.
/// Call [`reassert_main_zorder`] / [`force_topmost`] after geometry settles.
pub fn set_overlay_raised(raised: bool) {
    OVERLAY_RAISED.store(raised, Ordering::SeqCst);
}

/// Keep the collapsed strip above a stubborn maximized app (ChatGPT/Codex).
pub fn set_cover_topmost(need: bool) {
    COVER_TOPMOST.store(need, Ordering::SeqCst);
    reassert_main_zorder();
}

pub fn cover_topmost() -> bool {
    COVER_TOPMOST.load(Ordering::SeqCst)
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

/// Apply the correct Z-order for the current island mode (skip during tray yield).
pub fn reassert_main_zorder() {
    let hwnd = MAIN_HWND.load(Ordering::SeqCst);
    if hwnd == 0 || is_yielding() {
        return;
    }
    if OVERLAY_RAISED.load(Ordering::SeqCst) && !hwnd_looks_expanded(hwnd) {
        OVERLAY_RAISED.store(false, Ordering::SeqCst);
    }
    // Do not show/activate here: fullscreen and startup own visibility.
    set_topmost(hwnd, false);
}

#[cfg(windows)]
fn hwnd_looks_expanded(hwnd_raw: isize) -> bool {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let hwnd = HWND(hwnd_raw as *mut _);
    let mut wr = RECT::default();
    unsafe {
        if GetWindowRect(hwnd, &mut wr).is_err() {
            return OVERLAY_RAISED.load(Ordering::SeqCst);
        }
    }
    let h = (wr.bottom - wr.top).max(0);
    // Collapsed strip ≈ 28 logical ≈ 28–56 phys depending on DPI; expanded panel is taller.
    h > 72
}

#[cfg(not(windows))]
fn hwnd_looks_expanded(_hwnd_raw: isize) -> bool {
    OVERLAY_RAISED.load(Ordering::SeqCst)
}

/// Win11 三指下滑「显示桌面」会把岛窗最小化/隐去；非全屏隐藏期间必须拉回。
#[cfg(windows)]
pub fn ensure_main_visible() -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, IsWindow, IsWindowVisible, ShowWindowAsync, SW_SHOWNOACTIVATE,
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
        // Shell cloaking alone can mean another virtual desktop. ShowWindow
        // cannot uncloak that and must not pull the user out of their desktop.
        if !needs_visibility_restore(iconic, visible, cloaked) {
            return false;
        }
        // SW_RESTORE activates the bar and can cancel Explorer's Show Desktop.
        // Queue to the owning UI thread; never block the foreground probe.
        let restored = ShowWindowAsync(hwnd, SW_SHOWNOACTIVATE).as_bool();
        // Visibility restore only — Z-order follows overlay + HWND height.
        if !is_yielding() {
            reassert_main_zorder();
        }
        restored
    }
}

#[cfg(not(windows))]
pub fn ensure_main_visible() -> bool {
    false
}

#[cfg(windows)]
pub fn force_topmost(hwnd_raw: isize) {
    set_topmost(hwnd_raw, true);
}

#[cfg(windows)]
fn set_topmost(hwnd_raw: isize, show: bool) {
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
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE
                | if show { SWP_SHOWWINDOW } else { Default::default() },
        );
    }
}

#[cfg(windows)]
pub fn clear_topmost(hwnd_raw: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, SetWindowPos, GWL_EXSTYLE, HWND_NOTOPMOST,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WS_EX_TOPMOST,
    };

    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE);
        if ex & WS_EX_TOPMOST.0 as i32 == 0 {
            return;
        }
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
fn set_topmost(_hwnd_raw: isize, _show: bool) {}

#[cfg(not(windows))]
pub fn clear_topmost(_hwnd_raw: isize) {}

/// Cloaking is owned by DWM/Shell (e.g. virtual desktop switches); ShowWindow
/// only repairs hidden/minimized windows, not a shell-cloaked surface.
fn needs_visibility_restore(iconic: bool, visible: bool, cloaked: u32) -> bool {
    cloaked == 0 && (iconic || !visible)
}

#[cfg(test)]
mod tests {
    use super::needs_visibility_restore;

    #[test]
    fn minimized_or_hidden_bar_is_restored_but_occlusion_needs_zorder() {
        assert!(needs_visibility_restore(true, true, 0));
        assert!(needs_visibility_restore(false, false, 0));
        // Show Desktop can cover a visible HWND without minimizing it.
        assert!(!needs_visibility_restore(false, true, 0));
    }

    #[test]
    fn shell_cloaking_does_not_trigger_repeated_show_requests() {
        for cloak in [1, 2, 4] {
            assert!(!needs_visibility_restore(false, true, cloak));
            assert!(!needs_visibility_restore(false, false, cloak));
            assert!(!needs_visibility_restore(true, true, cloak));
        }
    }
}
