//! Hide the island overlay from Alt+Tab / Win+Tab (Task View).

#[cfg(windows)]
pub fn exclude_from_switcher(hwnd_raw: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{ITaskbarList, TaskbarList};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GA_ROOT, GWL_EXSTYLE,
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_EX_APPWINDOW,
        WS_EX_TOOLWINDOW,
    };

    if hwnd_raw == 0 {
        return;
    }

    unsafe {
        let hwnd = HWND(hwnd_raw as *mut _);
        // Prefer the top-level frame HWND (WebView2 child would not affect Alt+Tab).
        let root = GetAncestor(hwnd, GA_ROOT);
        let target = if root.0.is_null() { hwnd } else { root };

        let ex = GetWindowLongPtrW(target, GWL_EXSTYLE) as u32;
        let mut new_ex = ex | WS_EX_TOOLWINDOW.0;
        new_ex &= !WS_EX_APPWINDOW.0;
        if new_ex != ex {
            SetWindowLongPtrW(target, GWL_EXSTYLE, new_ex as isize);
            let _ = SetWindowPos(
                target,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }

        // Also drop any taskbar/switcher tab registration.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if let Ok(taskbar) =
            CoCreateInstance::<_, ITaskbarList>(&TaskbarList, None, CLSCTX_INPROC_SERVER)
        {
            let _ = taskbar.HrInit();
            let _ = taskbar.DeleteTab(target);
        }
    }
}

#[cfg(not(windows))]
pub fn exclude_from_switcher(_hwnd_raw: isize) {}
