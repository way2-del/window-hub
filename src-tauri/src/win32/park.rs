//! Park windows off-screen so they leave the desktop but stay capturable.

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
    pub style: i32,
    pub ex_style: i32,
}

const PARK_X: i32 = -10000;
const PARK_Y: i32 = -10000;

#[cfg(windows)]
pub fn park_window(hwnd_raw: isize) -> Result<PlacementSnapshot, String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, GetWindowPlacement, IsIconic, IsWindow, SetWindowLongW, SetWindowPlacement,
        SetWindowPos, ShowWindow, GWL_EXSTYLE, GWL_STYLE, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
        SW_RESTORE, WINDOWPLACEMENT, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
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

        let style = GetWindowLongW(hwnd, GWL_STYLE);
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);

        let snap = PlacementSnapshot {
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
            style,
            ex_style,
        };

        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }

        // Hide from taskbar while parked
        let mut new_ex = ex_style as u32;
        new_ex |= WS_EX_TOOLWINDOW.0;
        new_ex &= !WS_EX_APPWINDOW.0;
        SetWindowLongW(hwnd, GWL_EXSTYLE, new_ex as i32);

        let _ = SetWindowPos(
            hwnd,
            None,
            PARK_X,
            PARK_Y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );

        // Keep restored placement size but parked; avoid leaving as minimized.
        // (No DWM cloak — cloaking stops composition and breaks PrintWindow.)
        let mut parked = wp;
        parked.showCmd = windows::Win32::UI::WindowsAndMessaging::SW_SHOWNOACTIVATE.0 as u32;
        parked.rcNormalPosition.left = PARK_X;
        parked.rcNormalPosition.top = PARK_Y;
        parked.rcNormalPosition.right =
            PARK_X + (snap.normal_right - snap.normal_left).max(100);
        parked.rcNormalPosition.bottom =
            PARK_Y + (snap.normal_bottom - snap.normal_top).max(100);
        let _ = SetWindowPlacement(hwnd, &parked);

        Ok(snap)
    }
}

#[cfg(windows)]
pub fn unpark_window(hwnd_raw: isize, snap: &PlacementSnapshot) -> Result<(), String> {
    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        IsWindow, SetWindowLongW, SetWindowPlacement, SetWindowPos, ShowWindow, GWL_EXSTYLE,
        GWL_STYLE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_RESTORE, WINDOWPLACEMENT,
        WINDOWPLACEMENT_FLAGS,
    };

    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return Err("Invalid window handle".into());
        }

        SetWindowLongW(hwnd, GWL_STYLE, snap.style);
        SetWindowLongW(hwnd, GWL_EXSTYLE, snap.ex_style);

        let wp = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            flags: WINDOWPLACEMENT_FLAGS(snap.flags),
            showCmd: snap.show_cmd,
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
        SetWindowPlacement(hwnd, &wp).map_err(|e| format!("SetWindowPlacement failed: {e}"))?;

        let w = (snap.normal_right - snap.normal_left).max(100);
        let h = (snap.normal_bottom - snap.normal_top).max(100);
        let _ = SetWindowPos(
            hwnd,
            None,
            snap.normal_left,
            snap.normal_top,
            w,
            h,
            SWP_SHOWWINDOW | SWP_FRAMECHANGED | SWP_NOACTIVATE,
        );
        let _ = ShowWindow(hwnd, SW_RESTORE);
        Ok(())
    }
}

#[cfg(not(windows))]
pub fn park_window(_hwnd_raw: isize) -> Result<PlacementSnapshot, String> {
    Err("Windows only".into())
}

#[cfg(not(windows))]
pub fn unpark_window(_hwnd_raw: isize, _snap: &PlacementSnapshot) -> Result<(), String> {
    Err("Windows only".into())
}
