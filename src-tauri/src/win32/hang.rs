//! Detect hung foreign HWNDs so PrintWindow / GetWindowDC never block our process.

#[cfg(windows)]
pub fn is_hung_hwnd(hwnd_raw: isize) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{IsHungAppWindow, IsWindow};
    if hwnd_raw == 0 {
        return false;
    }
    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return false;
        }
        IsHungAppWindow(hwnd).as_bool()
    }
}

#[cfg(not(windows))]
pub fn is_hung_hwnd(_hwnd_raw: isize) -> bool {
    false
}
