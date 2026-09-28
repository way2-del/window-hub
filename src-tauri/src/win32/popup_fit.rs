//! Keep chrome / plugin popups inside the monitor work area (logical px).

const MARGIN: f64 = 8.0;
const FLIP_GAP: f64 = 8.0;

/// Fit popup top-left `(x, y)` for size `(w, h)` into the work area of the
/// monitor nearest `hwnd` (or primary if hwnd is 0 / invalid).
#[cfg(windows)]
pub fn fit_popup_origin(hwnd_raw: isize, x: f64, y: f64, w: f64, h: f64) -> (f64, f64) {
    use windows::Win32::{
        Foundation::HWND,
        Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
            MONITOR_DEFAULTTOPRIMARY,
        },
        UI::HiDpi::GetDpiForWindow,
    };

    if !x.is_finite() || !y.is_finite() || !w.is_finite() || !h.is_finite() {
        return (x, y);
    }
    let width = w.max(1.0);
    let height = h.max(1.0);

    unsafe {
        let hwnd = HWND(hwnd_raw as *mut _);
        let mon = if hwnd_raw != 0 {
            MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST)
        } else {
            MonitorFromWindow(HWND(std::ptr::null_mut()), MONITOR_DEFAULTTOPRIMARY)
        };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(mon, &mut info).as_bool() {
            return (x, y);
        }

        let scale = if hwnd_raw != 0 {
            GetDpiForWindow(hwnd).max(96) as f64 / 96.0
        } else {
            1.0
        };

        let work = info.rcWork;
        let left = work.left as f64 / scale + MARGIN;
        let top = work.top as f64 / scale + MARGIN;
        let right = work.right as f64 / scale - MARGIN;
        let bottom = work.bottom as f64 / scale - MARGIN;

        let mut next_x = x;
        let mut next_y = y;
        let max_x = (right - width).max(left);
        if next_x + width > right {
            next_x = max_x;
        }
        if next_x < left {
            next_x = left;
        }

        if next_y + height > bottom {
            // Requested y is typically just below the trigger; flip above that band.
            let trigger_bottom = y - FLIP_GAP;
            let flip_y = trigger_bottom - FLIP_GAP - height;
            if flip_y >= top {
                next_y = flip_y;
            } else {
                next_y = (bottom - height).max(top);
            }
        }
        if next_y < top {
            next_y = top;
        }

        (next_x, next_y)
    }
}

#[cfg(not(windows))]
pub fn fit_popup_origin(_hwnd_raw: isize, x: f64, y: f64, _w: f64, _h: f64) -> (f64, f64) {
    (x, y)
}
