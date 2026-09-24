//! Native startup supervision must remain usable even when WebView/UI is stuck.
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::Duration;

static STOPPING: AtomicBool = AtomicBool::new(false);
static MAIN_PAINTED: AtomicBool = AtomicBool::new(false);
static DOCK_PAINTED: AtomicBool = AtomicBool::new(false);
static REVEALED: AtomicBool = AtomicBool::new(false);
static DOCK_DONE: AtomicBool = AtomicBool::new(false);
static FAILURE_SHOWN: AtomicBool = AtomicBool::new(false);
static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);

pub fn register_main_window(hwnd: isize) {
    MAIN_HWND.store(hwnd, Ordering::Release);
}

pub fn stopping() -> bool {
    STOPPING.load(Ordering::Acquire)
}
pub fn revealed() {
    REVEALED.store(true, Ordering::Release);
}
pub fn dock_done() {
    DOCK_DONE.store(true, Ordering::Release);
}

#[tauri::command]
pub fn startup_surface_ready(window: tauri::WebviewWindow) {
    match window.label() {
        "main" => MAIN_PAINTED.store(true, Ordering::Release),
        "dock" => DOCK_PAINTED.store(true, Ordering::Release),
        _ => (),
    }
}

pub fn fail(detail: &str) {
    if stopping() || FAILURE_SHOWN.swap(true, Ordering::AcqRel) {
        return;
    }
    crate::boot_log("failure", detail);
    let detail = detail.to_owned();
    std::thread::spawn(move || {
        let log = std::env::temp_dir().join("window-hub-boot.log");
        let quit = rfd::MessageDialog::new()
            .set_title("Window Hub 启动失败")
            .set_level(rfd::MessageLevel::Error)
            .set_description(format!("{detail}\n\n顶栏或 Dock 未完成启动。\n日志：{}\n\n是否退出 Window Hub？退出后可重新打开，无需在任务管理器结束进程。选择“否”可继续等待。", log.display()))
            .set_buttons(rfd::MessageButtons::YesNo)
            .show();
        if quit == rfd::MessageDialogResult::Yes {
            #[cfg(windows)]
            crate::win32::autostart_svc::signal_user_quit();
            begin_shutdown(None);
        }
    });
}

pub fn supervise_startup() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(45));
        if stopping() {
            return;
        }
        let mut missing = Vec::new();
        #[cfg(windows)]
        unsafe {
            let hwnd = MAIN_HWND.load(Ordering::Acquire);
            if hwnd != 0 && windows::Win32::UI::WindowsAndMessaging::IsHungAppWindow(
                windows::Win32::Foundation::HWND(hwnd as *mut _),
            ).as_bool() {
                missing.push("主窗口消息循环无响应");
            }
        }
        if !REVEALED.load(Ordering::Acquire) {
            missing.push("顶栏原生窗口显示");
        }
        if !MAIN_PAINTED.load(Ordering::Acquire) {
            missing.push("顶栏网页渲染");
        }
        if !DOCK_DONE.load(Ordering::Acquire) {
            missing.push("Dock 初始化");
        }
        // Disabled Dock is accounted for by the pipeline without creating a WebView.
        if DOCK_REQUIRED.load(Ordering::Acquire) && !DOCK_PAINTED.load(Ordering::Acquire) {
            missing.push("Dock 网页渲染");
        }
        if !missing.is_empty() {
            fail(&format!("启动超过 45 秒，未完成：{}", missing.join("、")));
        }
    });
}

static DOCK_REQUIRED: AtomicBool = AtomicBool::new(false);
pub fn require_dock() {
    DOCK_REQUIRED.store(true, Ordering::Release);
}

/// Arm BEFORE app.exit: event-loop teardown itself can hang.
pub fn begin_shutdown(app: Option<tauri::AppHandle>) {
    if STOPPING.swap(true, Ordering::AcqRel) {
        return;
    }
    // Separate deadline from shell cleanup, which can itself wait on Explorer.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(6));
        std::process::exit(0);
    });
    std::thread::spawn(move || {
        crate::win32::appbar::restore();
        crate::win32::dock_appbar::restore();
        let _ = crate::win32::status_menu::set_taskbar_visible(true);
        // Give the AppBar owner threads time to process their shutdown messages.
        std::thread::sleep(Duration::from_millis(250));
        if let Some(app) = app {
            app.exit(0);
        }
    });
}
