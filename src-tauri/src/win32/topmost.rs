//! Main island HWND Z-order helpers.
//!
//! Collapsed top bar sits in the **AppBar-reserved** strip — no need for permanent
//! `HWND_TOPMOST` (wallpaper / normal windows already respect `rcWork`).
//! Expanded 灵动岛 overlays into the work area and **does** need TOPMOST.
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
    MAIN_HWND.store(hwnd, Ordering::SeqCst);
}

pub fn overlay_raised() -> bool {
    OVERLAY_RAISED.load(Ordering::SeqCst)
}

/// Island expanded → TOPMOST; collapsed strip → NOTOPMOST (AppBar owns the slot).
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
    // Prefer explicit flag; also treat short HWND as collapsed (FE used to skip
    // settle_overlay on shrink → OVERLAY_RAISED stuck true → forever TOPMOST).
    // COVER_TOPMOST: ChatGPT maximize clamp failed → keep strip above the app.
    let raised = COVER_TOPMOST.load(Ordering::SeqCst)
        || (OVERLAY_RAISED.load(Ordering::SeqCst) && hwnd_looks_expanded(hwnd));
    if raised {
        force_topmost(hwnd);
    } else {
        if OVERLAY_RAISED.load(Ordering::SeqCst) && !hwnd_looks_expanded(hwnd) {
            OVERLAY_RAISED.store(false, Ordering::SeqCst);
        }
        clear_topmost(hwnd);
    }
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
        // Visibility restore only — Z-order follows overlay + HWND height.
        if !is_yielding() {
            reassert_main_zorder();
        }
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
        GetWindowLongW, SetWindowLongW, SetWindowPos, GWL_EXSTYLE, HWND_NOTOPMOST,
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WS_EX_TOPMOST,
    };

    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE);
        let _ = SetWindowLongW(hwnd, GWL_EXSTYLE, ex & !(WS_EX_TOPMOST.0 as i32));
        let _ = SetWindowPos(
            hwnd,
            HWND_NOTOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

#[cfg(not(windows))]
pub fn force_topmost(_hwnd_raw: isize) {}

#[cfg(not(windows))]
pub fn clear_topmost(_hwnd_raw: isize) {}
