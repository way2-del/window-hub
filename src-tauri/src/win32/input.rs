//! Forward pointer/keyboard events to a parked Win32 window via PostMessage.

use crate::win32::capture::Roi;

#[derive(Debug, Clone)]
pub struct PointerEvent {
    pub kind: PointerKind,
    pub x: i32,
    pub y: i32,
    pub buttons: u32,
    pub delta_y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerKind {
    Down,
    Move,
    Up,
    Wheel,
}

#[derive(Debug, Clone)]
pub struct KeyEvent {
    pub kind: KeyKind,
    pub vk: u16,
    pub scan: u16,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    Down,
    Up,
    Char,
}

fn makelparam(x: i32, y: i32) -> isize {
    ((y as u16 as u32) << 16 | (x as u16 as u32)) as isize
}

#[cfg(windows)]
pub fn forward_pointer(hwnd_raw: isize, ev: &PointerEvent) -> Result<(), String> {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        PostMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE,
        WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP,
    };

    // MK_* values from WinUser.h
    const MK_LBUTTON: usize = 0x0001;
    const MK_RBUTTON: usize = 0x0002;
    const MK_MBUTTON: usize = 0x0010;

    let hwnd = HWND(hwnd_raw as *mut _);
    let lp = LPARAM(makelparam(ev.x, ev.y));

    let mut mk: usize = 0;
    if ev.buttons & 1 != 0 {
        mk |= MK_LBUTTON;
    }
    if ev.buttons & 2 != 0 {
        mk |= MK_RBUTTON;
    }
    if ev.buttons & 4 != 0 {
        mk |= MK_MBUTTON;
    }

    unsafe {
        match ev.kind {
            PointerKind::Move => {
                PostMessageW(hwnd, WM_MOUSEMOVE, WPARAM(mk), lp)
                    .map_err(|e| format!("WM_MOUSEMOVE: {e}"))?;
            }
            PointerKind::Down => {
                let msg = if ev.buttons & 2 != 0 {
                    WM_RBUTTONDOWN
                } else if ev.buttons & 4 != 0 {
                    WM_MBUTTONDOWN
                } else {
                    WM_LBUTTONDOWN
                };
                PostMessageW(hwnd, msg, WPARAM(mk), lp)
                    .map_err(|e| format!("button down: {e}"))?;
            }
            PointerKind::Up => {
                let msg = if ev.buttons & 2 != 0 {
                    WM_RBUTTONUP
                } else if ev.buttons & 4 != 0 {
                    WM_MBUTTONUP
                } else {
                    WM_LBUTTONUP
                };
                // On up, buttons field may already be cleared — prefer left up default
                let msg = if ev.buttons == 0 { WM_LBUTTONUP } else { msg };
                PostMessageW(hwnd, msg, WPARAM(0), lp).map_err(|e| format!("button up: {e}"))?;
            }
            PointerKind::Wheel => {
                // HIWORD = delta (120 units), LOWORD = keys; LPARAM = screen-ish but many apps accept client
                let wp = WPARAM((((ev.delta_y as i16) as u32) << 16) as usize);
                PostMessageW(hwnd, WM_MOUSEWHEEL, wp, lp)
                    .map_err(|e| format!("WM_MOUSEWHEEL: {e}"))?;
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
pub fn forward_key(hwnd_raw: isize, ev: &KeyEvent) -> Result<(), String> {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        PostMessageW, WM_CHAR, WM_KEYDOWN, WM_KEYUP,
    };

    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        match ev.kind {
            KeyKind::Down => {
                let repeat = 1u32;
                let scan = (ev.scan as u32) << 16;
                let lp = LPARAM((repeat | scan) as isize);
                PostMessageW(hwnd, WM_KEYDOWN, WPARAM(ev.vk as usize), lp)
                    .map_err(|e| format!("WM_KEYDOWN: {e}"))?;
            }
            KeyKind::Up => {
                let scan = (ev.scan as u32) << 16;
                let lp = LPARAM((1u32 | scan | (1 << 30) | (1 << 31)) as isize);
                PostMessageW(hwnd, WM_KEYUP, WPARAM(ev.vk as usize), lp)
                    .map_err(|e| format!("WM_KEYUP: {e}"))?;
            }
            KeyKind::Char => {
                if let Some(text) = &ev.text {
                    for ch in text.chars() {
                        PostMessageW(hwnd, WM_CHAR, WPARAM(ch as usize), LPARAM(1))
                            .map_err(|e| format!("WM_CHAR: {e}"))?;
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn forward_pointer(_hwnd_raw: isize, _ev: &PointerEvent) -> Result<(), String> {
    Err("Windows only".into())
}

#[cfg(not(windows))]
pub fn forward_key(_hwnd_raw: isize, _ev: &KeyEvent) -> Result<(), String> {
    Err("Windows only".into())
}

/// Map normalized preview coords [0,1] within the displayed ROI image to client coords.
pub fn map_preview_to_client(
    norm_x: f64,
    norm_y: f64,
    roi: Roi,
    client_w: i32,
    client_h: i32,
) -> (i32, i32) {
    let (ox, oy, ow, oh) = if roi.use_full || roi.w <= 0 || roi.h <= 0 {
        (0, 0, client_w, client_h)
    } else {
        (
            roi.x.clamp(0, client_w.max(1) - 1),
            roi.y.clamp(0, client_h.max(1) - 1),
            roi.w.clamp(1, client_w),
            roi.h.clamp(1, client_h),
        )
    };
    let x = ox + (norm_x.clamp(0.0, 1.0) * ow as f64) as i32;
    let y = oy + (norm_y.clamp(0.0, 1.0) * oh as f64) as i32;
    (x.min(client_w.max(1) - 1), y.min(client_h.max(1) - 1))
}

#[cfg(windows)]
pub fn client_size(hwnd_raw: isize) -> Result<(i32, i32), String> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, IsWindow};
    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return Err("Invalid window".into());
        }
        let mut rect = RECT::default();
        GetClientRect(hwnd, &mut rect).map_err(|e| e.to_string())?;
        Ok((
            (rect.right - rect.left).max(1),
            (rect.bottom - rect.top).max(1),
        ))
    }
}

#[cfg(not(windows))]
pub fn client_size(_hwnd_raw: isize) -> Result<(i32, i32), String> {
    Err("Windows only".into())
}

/// 模拟 Win+N，打开系统通知中心 / 日历面板。
#[cfg(windows)]
pub fn open_notification_center() -> Result<(), String> {
    send_shell_shortcut(windows::Win32::UI::Input::KeyboardAndMouse::VK_N)
}

/// 模拟 Win+A，切换系统控制中心。
#[cfg(windows)]
pub fn open_control_center() -> Result<(), String> {
    send_shell_shortcut(windows::Win32::UI::Input::KeyboardAndMouse::VK_A)
}

#[cfg(not(windows))]
pub fn open_control_center() -> Result<(), String> {
    Err("Windows only".into())
}

#[cfg(windows)]
fn send_shell_shortcut(key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY) -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
        VK_LWIN,
    };

    unsafe fn stroke(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        Default::default()
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    unsafe {
        let inputs = [
            stroke(VK_LWIN, false),
            stroke(key, false),
            stroke(key, true),
            stroke(VK_LWIN, true),
        ];
        let sent = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        if sent as usize != inputs.len() {
            return Err(format!("SendInput shell shortcut failed ({sent}/{})", inputs.len()));
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn open_notification_center() -> Result<(), String> {
    Err("Windows only".into())
}
