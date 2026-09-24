//! Tests against real HWNDs. The shell test is opt-in because it shows desktop.
use super::*;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::WindowsAndMessaging::*;

struct TestWindow(HWND);
unsafe extern "system" fn default_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    DefWindowProcW(hwnd, msg, wp, lp)
}
impl TestWindow {
    fn new(title: windows::core::PCWSTR, ex: WINDOW_EX_STYLE) -> Self {
        Self(unsafe {
            static CLASS: OnceLock<u16> = OnceLock::new();
            CLASS.get_or_init(|| RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(default_proc),
                lpszClassName: w!("WindowHubNativeAppBarRegression"),
                ..Default::default()
            }));
            CreateWindowExW(ex, w!("WindowHubNativeAppBarRegression"), title,
                WS_POPUP | WS_MINIMIZEBOX, 80, 80, 360, 28, None, None, None, None).unwrap()
        })
    }
}
impl Drop for TestWindow {
    fn drop(&mut self) { unsafe { let _ = DestroyWindow(self.0); } }
}

fn pump(ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn minimize_is_rejected_before_default_window_processing() {
    let window = TestWindow::new(w!("Window Hub minimize regression"), WS_EX_TOOLWINDOW);
    unsafe {
        let _ = ShowWindow(window.0, SW_SHOWNOACTIVATE);
        SendMessageW(window.0, WM_SYSCOMMAND, WPARAM(SC_MINIMIZE as usize), LPARAM(0));
        assert!(IsIconic(window.0).as_bool(), "unprotected native window must minimize");
        let _ = ShowWindow(window.0, SW_SHOWNOACTIVATE);
    }
    attach(window.0.0 as isize).unwrap();
    unsafe {
        let _ = ShowWindow(window.0, SW_SHOWNOACTIVATE);
        SendMessageW(window.0, WM_SYSCOMMAND, WPARAM(SC_MINIMIZE as usize | 2), LPARAM(0));
        assert!(!IsIconic(window.0).as_bool());
        assert!(IsWindowVisible(window.0).as_bool());
        // Reproduce a later framework style refresh.
        let style = GetWindowLongPtrW(window.0, GWL_STYLE);
        SetWindowLongPtrW(window.0, GWL_STYLE, style | WS_MINIMIZEBOX.0 as isize);
        assert_eq!(GetWindowLongPtrW(window.0, GWL_STYLE) & WS_MINIMIZEBOX.0 as isize, 0);
        // Explicit application hiding (fullscreen/shutdown) must still work.
        let _ = ShowWindow(window.0, SW_HIDE);
        assert!(!IsWindowVisible(window.0).as_bool());
    }
}

fn work_rect(hwnd: HWND) -> RECT {
    unsafe {
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        assert!(GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info).as_bool());
        info.rcWork
    }
}

#[test]
#[ignore = "interactive Windows shell: temporarily shows desktop and restores it"]
fn shell_show_desktop_preserves_visible_appbar_without_watchdog() {
    use windows::core::GUID;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{IShellDispatch4, SHAppBarMessage, APPBARDATA, ABM_NEW, ABM_REMOVE};
    let bar = TestWindow::new(w!("Window Hub shell regression"), WS_EX_TOOLWINDOW | WS_EX_TOPMOST);
    let ordinary = TestWindow::new(w!("Window Hub ordinary control"), WS_EX_APPWINDOW);
    unsafe {
        SetWindowLongPtrW(ordinary.0, GWL_STYLE, WS_OVERLAPPEDWINDOW.0 as isize);
        SetWindowLongPtrW(ordinary.0, GWL_EXSTYLE, WS_EX_APPWINDOW.0 as isize);
        SetWindowPos(ordinary.0, None, 80, 150, 480, 300,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED).unwrap();
    }
    attach(bar.0.0 as isize).unwrap();
    crate::win32::topmost::set_main_hwnd(bar.0.0 as isize);
    crate::win32::topmost::force_topmost(bar.0.0 as isize);
    crate::win32::appbar::register(bar.0.0 as isize);
    pump(800);
    crate::win32::topmost::reassert_main_zorder();
    let mut data = APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        hWnd: bar.0, uCallbackMessage: callback_message(), ..Default::default()
    };
    // Duplicate registration must fail: the visible HWND already owns the claim.
    let registered = unsafe { SHAppBarMessage(ABM_NEW, &mut data) } == 0;
    let collapsed = work_rect(bar.0);
    unsafe { SetWindowPos(bar.0, None, 0, 0, 360, 280, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE).unwrap(); }
    pump(400);
    let expanded = work_rect(bar.0);
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).unwrap();
        let shell: IShellDispatch4 = CoCreateInstance(
            &GUID::from_u128(0x13709620_C279_11CE_A49E_444553540000), None, CLSCTX_INPROC_SERVER).unwrap();
        let _ = ShowWindow(ordinary.0, SW_SHOW);
        let activated = SetForegroundWindow(ordinary.0).as_bool();
        pump(200);
        shell.MinimizeAll().unwrap();
        pump(700);
        // There is deliberately no watchdog/foreground-recovery loop in this test.
        let stayed_visible = IsWindowVisible(bar.0).as_bool() && !IsIconic(bar.0).as_bool();
        let stayed_topmost = GetWindowLongPtrW(bar.0, GWL_EXSTYLE) & WS_EX_TOPMOST.0 as isize != 0;
        let ordinary_minimized = IsIconic(ordinary.0).as_bool();
        eprintln!("shell regression: activated={activated} registered={registered} control_minimized={ordinary_minimized} bar_visible={stayed_visible} topmost={stayed_topmost} reserved_top={}/{}", collapsed.top, expanded.top);
        let _ = shell.UndoMinimizeALL();
        pump(500);
        let mut desktop_survived = true;
        for _ in 0..2 {
            shell.ToggleDesktop().unwrap();
            pump(500);
            let mut rect = RECT::default();
            GetWindowRect(bar.0, &mut rect).unwrap();
            desktop_survived &= IsWindowVisible(bar.0).as_bool()
                && !IsIconic(bar.0).as_bool()
                && GetAncestor(WindowFromPoint(POINT { x: rect.left + 10, y: rect.top + 10 }), GA_ROOT) == bar.0;
            let _ = shell.ToggleDesktop();
            pump(400);
        }
        drop(shell);
        CoUninitialize();
        crate::win32::appbar::restore();
        pump(300);
        SHAppBarMessage(ABM_REMOVE, &mut data);
        assert!(registered, "visible HWND must own shell registration");
        assert_eq!(collapsed.top, expanded.top, "opening overlay must not enlarge reservation");
        assert!(ordinary_minimized, "control window proves Shell Show Desktop ran");
        assert!(stayed_visible && stayed_topmost, "bar must survive without being restored");
        assert!(desktop_survived, "bar must remain on screen through repeated Show Desktop");
    }
}
