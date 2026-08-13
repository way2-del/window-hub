//! Park / unpark HWNDs for ECS staging.
//!
//! Restore MUST use the captured screen rect via SetWindowPos.
//! SetWindowPlacement alone is unreliable after hide/off-screen moves
//! (especially Electron/Chromium) and can leave a 1px strip on the left.

#[derive(Debug, Clone)]
pub struct PlacementSnapshot {
    pub flags: u32,
    pub show_cmd: u32,
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
    pub normal_left: i32,
    pub normal_top: i32,
    pub normal_right: i32,
    pub normal_bottom: i32,
    pub screen_left: i32,
    pub screen_top: i32,
    pub screen_right: i32,
    pub screen_bottom: i32,
    pub style: i32,
    pub ex_style: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct ParkOptions {
    /// When true, force WS_EX_TOOLWINDOW so the HWND leaves Alt+Tab / taskbar.
    pub hide_from_switcher: bool,
}

impl Default for ParkOptions {
    fn default() -> Self {
        Self {
            hide_from_switcher: true,
        }
    }
}

const PARK_X: i32 = -10000;
const PARK_Y: i32 = -10000;

#[cfg(windows)]
pub fn snapshot_window(hwnd_raw: isize) -> Result<PlacementSnapshot, String> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, GetWindowPlacement, GetWindowRect, IsWindow, GWL_EXSTYLE, GWL_STYLE,
        WINDOWPLACEMENT,
    };

    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return Err("Invalid window handle".into());
        }
        let mut wp = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        GetWindowPlacement(hwnd, &mut wp)
            .map_err(|e| format!("GetWindowPlacement failed: {e}"))?;
        let mut screen = RECT::default();
        GetWindowRect(hwnd, &mut screen).map_err(|e| format!("GetWindowRect: {e}"))?;
        let style = GetWindowLongW(hwnd, GWL_STYLE);
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
        Ok(PlacementSnapshot {
            flags: wp.flags.0,
            show_cmd: wp.showCmd,
            min_x: wp.ptMinPosition.x,
            min_y: wp.ptMinPosition.y,
            max_x: wp.ptMaxPosition.x,
            max_y: wp.ptMaxPosition.y,
            normal_left: wp.rcNormalPosition.left,
            normal_top: wp.rcNormalPosition.top,
            normal_right: wp.rcNormalPosition.right,
            normal_bottom: wp.rcNormalPosition.bottom,
            screen_left: screen.left,
            screen_top: screen.top,
            screen_right: screen.right,
            screen_bottom: screen.bottom,
            style,
            ex_style,
        })
    }
}

#[cfg(not(windows))]
pub fn snapshot_window(_hwnd_raw: isize) -> Result<PlacementSnapshot, String> {
    Err("Windows only".into())
}

#[cfg(windows)]
pub fn park_window(hwnd_raw: isize) -> Result<PlacementSnapshot, String> {
    let snap = snapshot_window(hwnd_raw)?;
    park_move_only(hwnd_raw, ParkOptions::default())?;
    Ok(snap)
}

/// Hide / move HWND without rewriting WINDOWPLACEMENT size memory.
#[cfg(windows)]
pub fn park_move_only(hwnd_raw: isize, opts: ParkOptions) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, IsIconic, IsWindow, SetWindowLongW, SetWindowPos, ShowWindow, GWL_EXSTYLE,
        SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_RESTORE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    };

    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return Err("Invalid window handle".into());
        }
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        if opts.hide_from_switcher {
            let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
            let mut new_ex = ex_style as u32;
            new_ex |= WS_EX_TOOLWINDOW.0;
            new_ex &= !WS_EX_APPWINDOW.0;
            SetWindowLongW(hwnd, GWL_EXSTYLE, new_ex as i32);
        }
        // ECS staging: off-screen (+ tool window when hide_from_switcher).
        let _ = SetWindowPos(
            hwnd,
            None,
            PARK_X,
            PARK_Y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        Ok(())
    }
}

#[cfg(windows)]
fn restore_box(snap: &PlacementSnapshot) -> (i32, i32, i32, i32) {
    let mut left = snap.screen_left;
    let mut top = snap.screen_top;
    let mut w = snap.screen_right - snap.screen_left;
    let mut h = snap.screen_bottom - snap.screen_top;

    if w < 120 || h < 80 {
        // Fall back to placement normal box if screen snapshot looks broken.
        left = snap.normal_left;
        top = snap.normal_top;
        w = (snap.normal_right - snap.normal_left).max(400);
        h = (snap.normal_bottom - snap.normal_top).max(300);
    }
    w = w.max(200);
    h = h.max(120);
    (left, top, w, h)
}

#[cfg(windows)]
pub fn unpark_window(hwnd_raw: isize, snap: &PlacementSnapshot) -> Result<(), String> {
    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        IsWindow, SetWindowLongW, SetWindowPlacement, SetWindowPos, ShowWindow, GWL_EXSTYLE,
        GWL_STYLE, HWND_TOP, SWP_FRAMECHANGED, SWP_SHOWWINDOW, SW_RESTORE, SW_SHOW,
        SW_SHOWMAXIMIZED, SW_SHOWNORMAL, WINDOWPLACEMENT, WINDOWPLACEMENT_FLAGS,
    };

    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return Err("Invalid window handle".into());
        }

        SetWindowLongW(hwnd, GWL_STYLE, snap.style);
        SetWindowLongW(hwnd, GWL_EXSTYLE, snap.ex_style);

        let was_max = snap.show_cmd == SW_SHOWMAXIMIZED.0 as u32;
        let (left, top, w, h) = restore_box(snap);
        let pos_flags = SWP_SHOWWINDOW | SWP_FRAMECHANGED;

        // Force the exact on-screen box first — this is what the user saw.
        let _ = SetWindowPos(hwnd, HWND_TOP, left, top, w, h, pos_flags);
        let _ = ShowWindow(hwnd, SW_SHOW);

        if was_max {
            let _ = ShowWindow(hwnd, SW_SHOWMAXIMIZED);
        } else {
            // Keep Windows' remembered "normal" size in sync for the next cycle.
            let show_cmd = SW_SHOWNORMAL.0 as u32;
            let wp = WINDOWPLACEMENT {
                length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
                flags: WINDOWPLACEMENT_FLAGS(snap.flags),
                showCmd: show_cmd,
                ptMinPosition: POINT {
                    x: snap.min_x,
                    y: snap.min_y,
                },
                ptMaxPosition: POINT {
                    x: snap.max_x,
                    y: snap.max_y,
                },
                rcNormalPosition: RECT {
                    left: snap.normal_left,
                    top: snap.normal_top,
                    right: snap.normal_right,
                    bottom: snap.normal_bottom,
                },
            };
            // Best-effort sync only — never rely on this alone for visibility.
            let _ = SetWindowPlacement(hwnd, &wp);
            let _ = SetWindowPos(hwnd, HWND_TOP, left, top, w, h, pos_flags);
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }

        Ok(())
    }
}

#[cfg(not(windows))]
pub fn park_window(_hwnd_raw: isize) -> Result<PlacementSnapshot, String> {
    Err("Windows only".into())
}

#[cfg(not(windows))]
pub fn park_move_only(_hwnd_raw: isize, _opts: ParkOptions) -> Result<(), String> {
    Err("Windows only".into())
}

#[cfg(not(windows))]
pub fn unpark_window(_hwnd_raw: isize, _snap: &PlacementSnapshot) -> Result<(), String> {
    Err("Windows only".into())
}
