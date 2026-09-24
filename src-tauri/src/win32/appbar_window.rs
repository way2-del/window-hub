//! UI-thread window contract for the visible shell AppBar.
//! Only main uses this subclass; settings, Dock and plugin windows keep their
//! existing behavior. Reservation calculations remain in appbar.rs.

use std::sync::OnceLock;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{
    DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass, ABM_ACTIVATE,
    ABM_WINDOWPOSCHANGED, ABN_POSCHANGED,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, RegisterWindowMessageW, SetWindowLongPtrW, SetWindowPos,
    GWL_STYLE, SC_MINIMIZE, STYLESTRUCT, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WINDOWPOS, WM_ACTIVATE, WM_NCDESTROY,
    WM_STYLECHANGING, WM_SYSCOMMAND, WM_WINDOWPOSCHANGED, WS_MINIMIZEBOX,
};

const SUBCLASS_ID: usize = 0x5748_4241;

pub fn callback_message() -> u32 {
    static MESSAGE: OnceLock<u32> = OnceLock::new();
    *MESSAGE.get_or_init(|| unsafe { RegisterWindowMessageW(w!("WindowHub.VisibleAppBar")) })
}

fn taskbar_created() -> u32 {
    static MESSAGE: OnceLock<u32> = OnceLock::new();
    *MESSAGE.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) })
}

/// Must be attached on main's owning UI thread before first reveal.
pub fn attach(raw: isize) -> Result<(), String> {
    if callback_message() == 0 || taskbar_created() == 0 {
        return Err("无法注册顶栏 Shell 消息".into());
    }
    let hwnd = HWND(raw as *mut _);
    unsafe {
        if !SetWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID, 0).as_bool() {
            return Err("无法安装顶栏最小化保护".into());
        }
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        SetWindowLongPtrW(hwnd, GWL_STYLE, style & !(WS_MINIMIZEBOX.0 as isize));
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
    }
    Ok(())
}

unsafe extern "system" fn window_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize,
) -> LRESULT {
    if msg == WM_SYSCOMMAND && (wparam.0 as u32 & 0xfff0) == SC_MINIMIZE {
        // Reject before DefWindowProc starts the minimize animation. This is
        // not an after-the-fact restore and never activates the bar.
        return LRESULT(0);
    }
    if msg == WM_STYLECHANGING && wparam.0 as i32 == GWL_STYLE.0 && lparam.0 != 0 {
        // Framework style refreshes must not reintroduce minimizability.
        let styles = &mut *(lparam.0 as *mut STYLESTRUCT);
        styles.styleNew &= !WS_MINIMIZEBOX.0;
    }
    if msg == callback_message() {
        // The notification code is wParam, not lParam (the latter is payload).
        if wparam.0 as u32 == ABN_POSCHANGED {
            super::appbar::sync(hwnd.0 as isize);
        }
        // Fullscreen visibility has one owner: spawn_fullscreen_watcher.
        // Do not hide the shell bar for window arrangement notifications.
        return LRESULT(0);
    }
    if msg == taskbar_created() {
        super::appbar::shell_restarted(hwnd.0 as isize);
    } else if msg == WM_ACTIVATE {
        super::appbar::notify(hwnd.0 as isize, ABM_ACTIVATE);
    } else if msg == WM_WINDOWPOSCHANGED && lparam.0 != 0 {
        let pos = &*(lparam.0 as *const WINDOWPOS);
        if !pos.flags.contains(SWP_NOMOVE | SWP_NOSIZE) {
            super::appbar::notify(hwnd.0 as isize, ABM_WINDOWPOSCHANGED);
        }
    } else if msg == WM_NCDESTROY {
        super::appbar::restore();
        let _ = RemoveWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID);
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

#[cfg(test)]
#[path = "../../../tests/native-appbar.rs"]
mod tests;
