//! Click the **real** Explorer notify-icon instance.
//!
//! Does not need hwnd/callback. Identity is:
//! - GUID (`NOTIFYICONIDENTIFIER.guidItem`), or
//! - hwnd + uid
//!
//! Flow: `Shell_NotifyIconGetRect` → `SendInput` at the rect center.
//! If the icon lives in overflow (no rect), temporarily `IsPromoted=1`, click, restore.

#![cfg(windows)]

use windows::core::GUID;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEINPUT,
};
use windows::Win32::UI::Shell::{Shell_NotifyIconGetRect, NOTIFYICONIDENTIFIER};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

fn normalize_guid(s: &str) -> String {
    s.trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .to_ascii_lowercase()
}

fn parse_guid(guid_str: &str) -> Option<GUID> {
    let g = normalize_guid(guid_str);
    if g.len() != 36 {
        return None;
    }
    let ok = g.as_bytes().iter().enumerate().all(|(i, &b)| match i {
        8 | 13 | 18 | 23 => b == b'-',
        _ => b.is_ascii_hexdigit(),
    });
    if !ok {
        return None;
    }
    std::panic::catch_unwind(|| GUID::from(g.as_str())).ok()
}

fn identifier(guid: Option<&str>, hwnd: isize, uid: u32) -> Option<NOTIFYICONIDENTIFIER> {
    if let Some(g) = guid.and_then(parse_guid) {
        return Some(NOTIFYICONIDENTIFIER {
            cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
            guidItem: g,
            ..Default::default()
        });
    }
    if hwnd != 0 {
        return Some(NOTIFYICONIDENTIFIER {
            cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: HWND(hwnd as _),
            uID: uid,
            ..Default::default()
        });
    }
    None
}

fn get_rect(ident: &NOTIFYICONIDENTIFIER) -> Option<RECT> {
    unsafe { Shell_NotifyIconGetRect(ident).ok() }
}

fn rect_usable(r: &RECT) -> bool {
    let w = r.right.saturating_sub(r.left);
    let h = r.bottom.saturating_sub(r.top);
    w > 0 && h > 0 && w < 400 && h < 400
}

fn to_absolute(x: i32, y: i32) -> (i32, i32) {
    let (cx, cy) = unsafe {
        (
            GetSystemMetrics(SM_CXSCREEN).max(1),
            GetSystemMetrics(SM_CYSCREEN).max(1),
        )
    };
    // SendInput absolute: 0..65535 maps across the primary virtual screen.
    let ax = ((x as i64) * 65535 / cx as i64) as i32;
    let ay = ((y as i64) * 65535 / cy as i64) as i32;
    (ax, ay)
}

fn mouse_input(flags: u32, dx: i32, dy: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS(flags),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send_button_at(x: i32, y: i32, right: bool, double: bool) -> Result<(), String> {
    let mut saved = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut saved);
    }
    let (ax, ay) = to_absolute(x, y);
    let (down, up) = if right {
        (MOUSEEVENTF_RIGHTDOWN.0, MOUSEEVENTF_RIGHTUP.0)
    } else {
        (MOUSEEVENTF_LEFTDOWN.0, MOUSEEVENTF_LEFTUP.0)
    };
    let move_f = MOUSEEVENTF_MOVE.0 | MOUSEEVENTF_ABSOLUTE.0;

    let mut inputs = vec![
        mouse_input(move_f, ax, ay),
        mouse_input(down, 0, 0),
        mouse_input(up, 0, 0),
    ];
    if double && !right {
        inputs.push(mouse_input(down, 0, 0));
        inputs.push(mouse_input(up, 0, 0));
    }
    // Restore cursor so the user pointer does not stick on the tray strip.
    let (sx, sy) = to_absolute(saved.x, saved.y);
    inputs.push(mouse_input(move_f, sx, sy));

    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        return Err(format!(
            "SendInput shell-rect click failed ({sent}/{})",
            inputs.len()
        ));
    }
    Ok(())
}

fn click_rect(r: &RECT, right: bool, double: bool) -> Result<(), String> {
    let x = r.left + (r.right - r.left) / 2;
    let y = r.top + (r.bottom - r.top) / 2;
    crate::win32::click_trace::log(
        "tray",
        &format!("shell-rect click at=({x},{y}) right={right} double={double} rect=[{},{} {}x{}]",
            r.left, r.top, r.right - r.left, r.bottom - r.top),
    );
    send_button_at(x, y, right, double)
}

/// Click the shell's live notify-icon instance by GUID and/or hwnd+uid.
///
/// `reg_key`: NotifyIconSettings subkey for temporary promote when the icon is
/// in overflow (GetRect fails until promoted / overflow open).
pub fn invoke(
    guid: Option<&str>,
    hwnd: isize,
    uid: u32,
    reg_key: Option<&str>,
    right: bool,
    double: bool,
) -> Result<(), String> {
    let Some(ident) = identifier(guid, hwnd, uid) else {
        return Err("shell-rect: no GUID and no hwnd for identity".into());
    };

    if let Some(r) = get_rect(&ident).filter(rect_usable) {
        return click_rect(&r, right, double);
    }

    // Overflow / hidden: temporarily promote so Explorer exposes a screen rect.
    let Some(key) = reg_key.filter(|k| !k.is_empty()) else {
        return Err("shell-rect: GetRect failed and no reg key to promote".into());
    };

    let snap = crate::win32::tray_registry::snapshot_promoted();
    let prev = snap
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| *v)
        .unwrap_or(0);

    crate::win32::click_trace::log(
        "tray",
        &format!("shell-rect promote key={key} prev={prev}"),
    );
    crate::win32::tray_registry::set_promoted(key, 1)?;
    // Explorer needs a beat to re-layout the notify strip.
    std::thread::sleep(std::time::Duration::from_millis(180));

    let result = (|| {
        let r = get_rect(&ident)
            .filter(rect_usable)
            .ok_or_else(|| "shell-rect: GetRect still empty after promote".to_string())?;
        click_rect(&r, right, double)
    })();

    let _ = crate::win32::tray_registry::set_promoted(key, prev);
    result
}
