use crate::commands::{
    apply_saved_material_pub as apply_saved_material, async_delay_ms, close_sibling_popups,
    create_watchdog, finish_watchdog, hide_chrome_popup, lock_webview_create, mark_popup_visible,
    popup_visible, MaterialState,
};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::window::Color;
use tauri::{
    AppHandle, Emitter, LogicalPosition, Manager, State, WebviewUrl, WebviewWindowBuilder,
};

static TOGGLING: AtomicBool = AtomicBool::new(false);
struct ToggleGuard;
impl Drop for ToggleGuard {
    fn drop(&mut self) {
        TOGGLING.store(false, Ordering::SeqCst);
    }
}

/// One IPC handles visibility and anchoring. Do not round-trip through several
/// UI-thread window getters before a click can dismiss or reopen the popup.
#[tauri::command]
pub async fn toggle_control_center(
    app: AppHandle,
    state: State<'_, MaterialState>,
    window: tauri::WebviewWindow,
    right: f64,
    bottom: f64,
) -> Result<(), String> {
    if TOGGLING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let _guard = ToggleGuard;
    let start = std::time::Instant::now();
    crate::win32::click_trace::log("control", "toggle enter");
    if let Some(popup) = app.get_webview_window("control-center-popup") {
        let hwnd =
            windows::Win32::Foundation::HWND(popup.hwnd().map_err(|e| e.to_string())?.0 as *mut _);
        if unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd).as_bool() } {
            hide_chrome_popup(&app, "control-center-popup");
            crate::win32::click_trace::log("control", "toggle hidden");
            return Ok(());
        }
    }
    let (x, y) = popup_anchor(&window, right, bottom)?;
    let result = open_control_center(app, state, x, y).await;
    crate::win32::click_trace::log(
        "control",
        &format!(
            "toggle finished {}ms ok={}",
            start.elapsed().as_millis(),
            result.is_ok()
        ),
    );
    result
}

fn popup_anchor(
    window: &tauri::WebviewWindow,
    right: f64,
    bottom: f64,
) -> Result<(f64, f64), String> {
    use windows::Win32::{
        Foundation::RECT,
        Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        },
        UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::GetWindowRect},
    };
    if !right.is_finite() || !bottom.is_finite() {
        return Err("Invalid popup anchor".into());
    }
    let hwnd =
        windows::Win32::Foundation::HWND(window.hwnd().map_err(|e| e.to_string())?.0 as *mut _);
    unsafe {
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect).map_err(|e| e.to_string())?;
        let scale = GetDpiForWindow(hwnd).max(96) as f64 / 96.0;
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(
            MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        )
        .ok()
        .map_err(|e| e.to_string())?;
        let left = monitor.rcMonitor.left as f64 / scale + 8.0;
        let limit = (monitor.rcMonitor.right as f64 / scale - 382.0).max(left);
        Ok((
            (rect.left as f64 / scale + right - 374.0).clamp(left, limit),
            rect.top as f64 / scale + bottom + 8.0,
        ))
    }
}

/// Host control center; shares the WLAN popup material and lifecycle.
#[tauri::command]
pub async fn open_control_center(
    app: AppHandle,
    state: State<'_, MaterialState>,
    x: f64,
    y: f64,
) -> Result<(), String> {
    close_sibling_popups(&app, "control-center-popup");

    if let Some(existing) = app.get_webview_window("control-center-popup") {
        // Material and content size survive hide. Reapplying Mica schedules
        // multiple backdrop resets and resize messages on every toggle.
        existing
            .set_position(LogicalPosition::new(x, y))
            .map_err(|e| e.to_string())?;
        existing.show().map_err(|e| e.to_string())?;
        existing.set_focus().map_err(|e| e.to_string())?;
        mark_popup_visible("control-center-popup", true);
        let _ = app.emit("control-center-popup-opened", ());
        return Ok(());
    }

    crate::win32::click_trace::log("rust", "open_control_center NEED_CREATE async-delay");
    async_delay_ms(100).await;
    if app.get_webview_window("control-center-popup").is_some() {
        return Ok(());
    }
    crate::win32::click_trace::log("rust", "open_control_center after delay, build");
    let init = "window.__WH_IS_CONTROL_CENTER__ = true;";
    let _guard = lock_webview_create("control-center-popup");
    if app.get_webview_window("control-center-popup").is_some() {
        return Ok(());
    }
    let wd = create_watchdog("control-center-popup");
    let built = WebviewWindowBuilder::new(
        &app,
        "control-center-popup",
        WebviewUrl::App("index.html?window=control-center".into()),
    )
    .title("控制中心")
    .inner_size(374.0, 464.0)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .closable(true)
    .decorations(false)
    .transparent(true)
    .background_color(Color(0, 0, 0, 0))
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(true)
    .visible(false)
    .initialization_script(init)
    .build();
    finish_watchdog(&wd);
    let win = built.map_err(|e| format!("control-center-popup create failed: {e}"))?;
    let _ = win.set_position(LogicalPosition::new(x, y));
    apply_saved_material(&win, &state);
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
    }
    let _ = win.show();
    let _ = win.set_focus();
    mark_popup_visible("control-center-popup", true);
    let _ = app.emit("control-center-popup-opened", ());
    crate::win32::click_trace::log("rust", "open_control_center build DONE");
    Ok(())
}

#[tauri::command]
pub async fn close_control_center(app: AppHandle) -> Result<(), String> {
    hide_chrome_popup(&app, "control-center-popup");
    Ok(())
}

#[tauri::command]
pub fn is_control_center_open(_app: AppHandle) -> bool {
    popup_visible("control-center-popup")
}

#[tauri::command]
pub async fn control_center_audio(
    window: tauri::WebviewWindow,
) -> Result<crate::win32::control_center::AudioState, String> {
    require_control_center(&window)?;
    tauri::async_runtime::spawn_blocking(crate::win32::control_center::audio_state)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn control_center_brightness(
    window: tauri::WebviewWindow,
    value: Option<u8>,
) -> Result<Option<u8>, String> {
    require_control_center(&window)?;
    tauri::async_runtime::spawn_blocking(move || crate::win32::control_center::brightness(value))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn control_center_action(
    window: tauri::WebviewWindow,
    action: String,
    value: Option<u8>,
) -> Result<(), String> {
    require_control_center(&window)?;
    tauri::async_runtime::spawn_blocking(move || match action.as_str() {
        "volume" => crate::win32::control_center::set_volume(value.ok_or("Missing volume")?),
        "mic" => {
            crate::win32::control_center::set_mic_muted(value.ok_or("Missing mute state")? != 0)
        }
        "play_pause" => crate::commands::send_media_virtual_key(0xB3),
        "next" => crate::commands::send_media_virtual_key(0xB0),
        "cast" => crate::win32::input::send_shell_shortcut(
            windows::Win32::UI::Input::KeyboardAndMouse::VK_K,
        ),
        page => crate::win32::control_center::open_settings(page),
    })
    .await
    .map_err(|e| e.to_string())?
}

fn require_control_center(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() != "control-center-popup" {
        return Err("Device controls are reserved for the host control center".into());
    }
    Ok(())
}
