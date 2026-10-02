use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, PhysicalPosition, PhysicalSize,
    State, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};
use tauri::window::Color;

use crate::dock::DockVisibility;
use crate::ecs::components::CaptureRoi;
use crate::ecs::resources::{HubCommand, KeyKindDto, PointerKindDto};
use crate::ecs::EcsHandle;
use crate::plugin_hub::hub_init_script;
use crate::win32::enum_windows::{focus_window, parse_window_id, WindowInfo};
use crate::windows_service::WindowsService;

fn set_dock_menu_hold(app: &AppHandle, hold: bool) {
    if let Some(vis) = app.try_state::<Arc<DockVisibility>>() {
        vis.set_interaction_hold(hold);
    }
}

fn set_dock_preview_tip_keep(app: &AppHandle, keep: bool) {
    if let Some(vis) = app.try_state::<Arc<DockVisibility>>() {
        vis.set_preview_tip_keep(keep);
    }
}

/// Chrome popups that must be **hidden and reused**, never destroyed on dismiss.
/// Destroying + rebuilding WebView2 from a click IPC path deadlocks the UI pump
/// (click → 未响应). Frontend already listens for `*-opened` and re-fits on reuse.
const REUSABLE_CHROME_POPUPS: &[&str] = &[
    "tray-popup",
    "status-menu-popup",
    "dock-add-icon-popup",
    "input-lang-popup",
    "control-center-popup",
    "wifi-popup",
    "wifi-auth-popup",
];

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex as StdMutex;

static TRAY_POPUP_VISIBLE: AtomicBool = AtomicBool::new(false);
static STATUS_MENU_POPUP_VISIBLE: AtomicBool = AtomicBool::new(false);
static DOCK_ADD_ICON_POPUP_VISIBLE: AtomicBool = AtomicBool::new(false);
static INPUT_LANG_POPUP_VISIBLE: AtomicBool = AtomicBool::new(false);
static CONTROL_CENTER_VISIBLE: AtomicBool = AtomicBool::new(false);
static WIFI_POPUP_VISIBLE: AtomicBool = AtomicBool::new(false);
static WIFI_AUTH_POPUP_VISIBLE: AtomicBool = AtomicBool::new(false);
static PLUGIN_POPUP_VISIBLE: AtomicBool = AtomicBool::new(false);
/// Serialize every WebviewWindowBuilder::build — concurrent creates freeze the UI pump
/// (proven by click-trace: tip NEED_BUILD + status-menu open racing → 未响应).
static WEBVIEW_CREATE_LOCK: StdMutex<()> = StdMutex::new(());
#[allow(dead_code)]
static TIP_PREWARM_REQUESTED: AtomicBool = AtomicBool::new(false);

pub(crate) struct WebviewCreateGuard {
    _guard: std::sync::MutexGuard<'static, ()>,
    label: &'static str,
}

impl Drop for WebviewCreateGuard {
    fn drop(&mut self) {
        crate::win32::click_trace::log_lock_released(self.label);
    }
}

/// Acquire create lock with wait/hold timing in the click-trace log.
pub(crate) fn lock_webview_create(label: &'static str) -> WebviewCreateGuard {
    crate::win32::click_trace::log_lock_wait(label);
    let t0 = std::time::Instant::now();
    let guard = WEBVIEW_CREATE_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    crate::win32::click_trace::log_lock_acquired(label, t0.elapsed().as_millis());
    WebviewCreateGuard {
        _guard: guard,
        label,
    }
}

pub(crate) fn mark_popup_visible(label: &str, visible: bool) {
    let flag = match label {
        "tray-popup" => &TRAY_POPUP_VISIBLE,
        "status-menu-popup" => &STATUS_MENU_POPUP_VISIBLE,
        "dock-add-icon-popup" => &DOCK_ADD_ICON_POPUP_VISIBLE,
        "input-lang-popup" => &INPUT_LANG_POPUP_VISIBLE,
        "control-center-popup" => &CONTROL_CENTER_VISIBLE,
        "wifi-popup" => &WIFI_POPUP_VISIBLE,
        "wifi-auth-popup" => &WIFI_AUTH_POPUP_VISIBLE,
        "plugin-popup" | "plugin-window" => &PLUGIN_POPUP_VISIBLE,
        _ => return,
    };
    flag.store(visible, Ordering::SeqCst);
}

pub(crate) fn popup_visible(label: &str) -> bool {
    match label {
        "tray-popup" => TRAY_POPUP_VISIBLE.load(Ordering::SeqCst),
        "status-menu-popup" => STATUS_MENU_POPUP_VISIBLE.load(Ordering::SeqCst),
        "dock-add-icon-popup" => DOCK_ADD_ICON_POPUP_VISIBLE.load(Ordering::SeqCst),
        "input-lang-popup" => INPUT_LANG_POPUP_VISIBLE.load(Ordering::SeqCst),
        "control-center-popup" => CONTROL_CENTER_VISIBLE.load(Ordering::SeqCst),
        "wifi-popup" => WIFI_POPUP_VISIBLE.load(Ordering::SeqCst),
        "wifi-auth-popup" => WIFI_AUTH_POPUP_VISIBLE.load(Ordering::SeqCst),
        "plugin-popup" | "plugin-window" => PLUGIN_POPUP_VISIBLE.load(Ordering::SeqCst),
        _ => false,
    }
}

fn emit_chrome_popup_closed(app: &AppHandle, label: &str) {
    mark_popup_visible(label, false);
    match label {
        "tray-popup" => {
            let _ = app.emit("tray-popup-closed", ());
        }
        "plugin-popup" => {
            let _ = app.emit("plugin-popup-closed", ());
        }
        "status-menu-popup" => {
            // Add-icon picker may take over — keep AutoHide hold in that case.
            if !popup_visible("dock-add-icon-popup") {
                set_dock_menu_hold(app, false);
            }
            let _ = app.emit("status-menu-popup-closed", ());
        }
        "dock-add-icon-popup" => {
            set_dock_menu_hold(app, false);
            let _ = app.emit("dock-add-icon-popup-closed", ());
        }
        "input-lang-popup" => {
            let _ = app.emit("input-lang-popup-closed", ());
        }
        "control-center-popup" => { let _ = app.emit("control-center-popup-closed", ()); }
        "wifi-popup" => {
            let _ = app.emit("wifi-popup-closed", ());
        }
        "wifi-auth-popup" => {
            let _ = app.emit("wifi-auth-popup-closed", ());
        }
        _ => {}
    }
}

/// Hide (do not destroy) a reusable chrome popup and emit its closed event.
pub fn hide_chrome_popup(app: &AppHandle, label: &str) {
    crate::win32::click_trace::log("popup", &format!("hide {label}"));
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.hide();
    }
    emit_chrome_popup_closed(app, label);
}

/// Win32 hide from a worker / focus-loss thread — avoids Tauri `hide()`/`close()`
/// on a non-UI thread while still keeping the HWND for reuse.
pub fn hide_chrome_popup_hwnd(app: &AppHandle, label: &str, hwnd_raw: Option<isize>) {
    crate::win32::click_trace::log("popup", &format!("blur hide {label}"));
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
        if let Some(raw) = hwnd_raw.filter(|h| *h != 0) {
            unsafe {
                let _ = ShowWindow(HWND(raw as *mut _), SW_HIDE);
            }
        } else if let Some(w) = app.get_webview_window(label) {
            let _ = w.hide();
        }
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd_raw;
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.hide();
        }
    }
    emit_chrome_popup_closed(app, label);
}

/// Yield the async command past the sync IPC reply path before WebView create.
/// Never use bare `std::thread` + `WebviewWindowBuilder::build` (click-trace hung
/// at chrome-prewarm status-menu build → Responding=False / 穿透).
pub(crate) async fn async_delay_ms(ms: u64) {
    let _ = tauri::async_runtime::spawn_blocking(move || {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    })
    .await;
}

pub(crate) fn create_watchdog(label: &'static str) -> Arc<AtomicBool> {
    crate::win32::click_trace::mark_create_in_progress(true);
    let done = Arc::new(AtomicBool::new(false));
    let flag = done.clone();
    std::thread::Builder::new()
        .name("create-watchdog".into())
        .spawn(move || {
            for i in 1..=20 {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if flag.load(Ordering::SeqCst) {
                    return;
                }
                crate::win32::click_trace::log(
                    "watchdog",
                    &format!("{label} STILL_IN_BUILD after {}ms — UI may freeze / 穿透", i * 500),
                );
            }
        })
        .ok();
    done
}

pub(crate) fn finish_watchdog(done: &Arc<AtomicBool>) {
    done.store(true, Ordering::SeqCst);
    crate::win32::click_trace::mark_create_in_progress(false);
}

#[derive(Deserialize)]
pub struct AttachArgs {
    pub hwnd: isize,
    pub slot: u8,
    pub title: String,
    pub class_name: String,
    pub pid: u32,
}

#[derive(Deserialize)]
pub struct RoiArgs {
    pub slot: u8,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub use_full: bool,
}

#[derive(Deserialize)]
pub struct SwapArgs {
    pub a: u8,
    pub b: u8,
}

#[derive(Deserialize)]
pub struct PointerArgs {
    pub slot: u8,
    pub kind: PointerKindDto,
    pub norm_x: f64,
    pub norm_y: f64,
    pub buttons: u32,
    pub delta_y: i32,
}

#[derive(Deserialize)]
pub struct KeyArgs {
    pub slot: u8,
    pub kind: KeyKindDto,
    pub vk: u16,
    pub scan: u16,
    pub text: Option<String>,
}

#[tauri::command]
pub fn list_open_windows(
    app: AppHandle,
    svc: State<'_, WindowsService>,
) -> Result<Vec<WindowInfo>, String> {
    Ok(svc.refresh_now(&app))
}

#[tauri::command]
pub fn get_open_window(
    id: String,
    svc: State<'_, WindowsService>,
) -> Result<Option<WindowInfo>, String> {
    svc.get(&id)
}

#[tauri::command]
pub fn focus_open_window(id: String) -> Result<(), String> {
    let hwnd = parse_window_id(&id)?;
    focus_window(hwnd)
}

#[tauri::command]
pub fn attach_window(ecs: State<'_, EcsHandle>, args: AttachArgs) -> Result<(), String> {
    if args.slot > 2 {
        return Err("slot must be 0..2".into());
    }
    ecs.send(HubCommand::Attach {
        hwnd: args.hwnd,
        slot: args.slot,
        title: args.title,
        class_name: args.class_name,
        pid: args.pid,
    });
    Ok(())
}

#[tauri::command]
pub fn detach_window(ecs: State<'_, EcsHandle>, slot: u8) -> Result<(), String> {
    if slot > 2 {
        return Err("slot must be 0..2".into());
    }
    ecs.send(HubCommand::Detach { slot });
    Ok(())
}

#[tauri::command]
pub fn set_roi(ecs: State<'_, EcsHandle>, args: RoiArgs) -> Result<(), String> {
    if args.slot > 2 {
        return Err("slot must be 0..2".into());
    }
    ecs.send(HubCommand::SetRoi {
        slot: args.slot,
        roi: CaptureRoi {
            x: args.x,
            y: args.y,
            w: args.w,
            h: args.h,
            use_full: args.use_full,
        },
    });
    Ok(())
}

#[tauri::command]
pub fn swap_slots(ecs: State<'_, EcsHandle>, args: SwapArgs) -> Result<(), String> {
    if args.a > 2 || args.b > 2 {
        return Err("slots must be 0..2".into());
    }
    ecs.send(HubCommand::SwapSlots {
        a: args.a,
        b: args.b,
    });
    Ok(())
}

#[tauri::command]
pub fn forward_pointer(ecs: State<'_, EcsHandle>, args: PointerArgs) -> Result<(), String> {
    ecs.send(HubCommand::Pointer {
        slot: args.slot,
        kind: args.kind,
        norm_x: args.norm_x,
        norm_y: args.norm_y,
        buttons: args.buttons,
        delta_y: args.delta_y,
    });
    Ok(())
}

#[tauri::command]
pub fn forward_key(ecs: State<'_, EcsHandle>, args: KeyArgs) -> Result<(), String> {
    ecs.send(HubCommand::Key {
        slot: args.slot,
        kind: args.kind,
        vk: args.vk,
        scan: args.scan,
        text: args.text,
    });
    Ok(())
}

#[tauri::command]
pub fn self_hwnd(window: WebviewWindow) -> Result<isize, String> {
    window
        .hwnd()
        .map(|h| h.0 as isize)
        .map_err(|e| e.to_string())
}

/// 兼容旧调用：忽略高度，绝不改 AppBar / 工作区。
#[tauri::command]
pub fn dock_set_visual_height(_window: WebviewWindow, _height: i32) -> Result<(), String> {
    Ok(())
}

/// 岛展开 / 拉高：面板伸进工作区，需要 TOPMOST 盖住普通窗口。
///
/// **Must stay Win32-only / async** — sync `window.show()` / `set_always_on_top`
/// from an IPC handler deadlocks WebView2 on Windows (click → 未响应).
#[tauri::command]
pub async fn float_overlay(window: WebviewWindow) -> Result<(), String> {
    crate::win32::click_trace::log("rust", "float_overlay enter");
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let raw = hwnd.0 as isize;
    crate::win32::switcher::exclude_from_switcher(raw);
    crate::win32::topmost::set_main_hwnd(raw);
    crate::win32::topmost::set_overlay_raised(true);
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{
            IsIconic, ShowWindow, SW_RESTORE, SW_SHOWNOACTIVATE,
        };
        let h = HWND(raw as *mut _);
        unsafe {
            if IsIconic(h).as_bool() {
                let _ = ShowWindow(h, SW_RESTORE);
            }
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
        }
        crate::win32::topmost::force_topmost(raw);
    }
    #[cfg(not(windows))]
    {
        let _ = window.set_skip_taskbar(true);
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_always_on_top(true);
    }
    Ok(())
}

/// 热键呼出岛栏搜索：激活主窗以便键盘直达输入框（async，勿在 sync IPC 里 set_focus）。
#[tauri::command]
pub async fn activate_main_island(window: WebviewWindow) -> Result<(), String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let raw = hwnd.0 as isize;
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            AllowSetForegroundWindow, ASFW_ANY,
        };
        unsafe {
            let _ = AllowSetForegroundWindow(ASFW_ANY);
        }
        let _ = crate::win32::enum_windows::focus_window(raw);
    }
    let _ = window.set_focus();
    Ok(())
}

/// Resize the main island HWND (full monitor width × logical height).
/// Bypasses `resizable: false` — Tauri `set_size` is unreliable during pull gestures.
#[tauri::command]
pub async fn resize_main_island(
    window: WebviewWindow,
    window_height: f64,
) -> Result<(), String> {
    crate::win32::click_trace::log(
        "rust",
        &format!("resize_main_island enter h={window_height:.1}"),
    );
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let raw = hwnd.0 as isize;
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        };
        use windows::Win32::UI::HiDpi::GetDpiForWindow;
        use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER};

        // BEFORE SetWindowPos — Moved/Resized must not pin/bar_comp re-enter.
        crate::win32::island_bar_glass::suppress_refresh_ms(600);

        let hwnd = HWND(raw as *mut _);
        unsafe {
            let dpi = GetDpiForWindow(hwnd).max(96) as f64;
            let scale = dpi / 96.0;
            let h_px = (window_height * scale).round().max(1.0) as i32;

            let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(mon, &mut info).as_bool() {
                crate::win32::click_trace::log("rust", "resize_main_island FAIL monitor");
                return Err("resize_main_island: monitor".into());
            }
            let mon = info.rcMonitor;
            let w = (mon.right - mon.left).max(1);
            SetWindowPos(
                hwnd,
                None,
                mon.left,
                mon.top,
                w,
                h_px,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .map_err(|e| {
                crate::win32::click_trace::log(
                    "rust",
                    &format!("resize_main_island FAIL SetWindowPos: {e}"),
                );
                format!("resize_main_island SetWindowPos: {e}")
            })?;
        }
        // Do NOT call set_overlay_raised / ensure_main_visible / reassert_main_zorder
        // here — they SetWindowPos again and re-enter Moved while WebView2 is mid
        // resize → HUNG (click-trace #82 enter h=28 → Moved done → never leave ok).
        crate::win32::click_trace::log("rust", "resize_main_island leave ok");
    }
    #[cfg(not(windows))]
    {
        let scale = window.scale_factor().map_err(|e| e.to_string())?;
        let inner = window.inner_size().map_err(|e| e.to_string())?;
        let w = inner.width as f64 / scale;
        window
            .set_size(tauri::LogicalSize::new(w, window_height))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// FE collapse path: skip Moved → bar_comp while Tauri setSize shrinks the island.
#[tauri::command]
pub fn suppress_island_bar_refresh(ms: Option<u64>) -> Result<(), String> {
    let ms = ms.unwrap_or(800).clamp(100, 5_000);
    #[cfg(windows)]
    {
        crate::win32::island_bar_glass::suppress_refresh_ms(ms);
        crate::win32::click_trace::log("rust", &format!("suppress_island_bar_refresh {ms}ms"));
    }
    Ok(())
}

/// 岛收回折叠条：恢复当前场景层级（桌面保留 TOPMOST，不激活窗口）。
#[tauri::command]
pub async fn settle_overlay(window: WebviewWindow) -> Result<(), String> {
    crate::win32::click_trace::log("rust", "settle_overlay enter");
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let raw = hwnd.0 as isize;
    crate::win32::switcher::exclude_from_switcher(raw);
    crate::win32::topmost::set_main_hwnd(raw);
    crate::win32::topmost::set_overlay_raised(false);
    #[cfg(windows)]
    {
        crate::win32::island_bar_glass::suppress_refresh_ms(400);
        crate::win32::topmost::reassert_main_zorder();
    }
    #[cfg(not(windows))]
    {
        let _ = window.set_always_on_top(false);
    }
    crate::win32::click_trace::log("rust", "settle_overlay leave");
    Ok(())
}

/// 打开独立设置窗口。
///
/// Windows + WebView2：在同步 `#[tauri::command]` 里调用 `WebviewWindowBuilder::build`
/// 会死锁（官方文档明确要求用 async command），表现为白屏且标题栏关闭无效。
/// 见：https://docs.rs/tauri/latest/tauri/webview/struct.WebviewWindowBuilder.html
#[tauri::command]
pub async fn open_settings_window(
    app: AppHandle,
    state: State<'_, MaterialState>,
    plugin_id: Option<String>,
    focus_nav: Option<String>,
) -> Result<(), String> {
    let focus = plugin_id
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let nav = focus_nav
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(existing) = app.get_webview_window("settings") {
        // Already painted — soft reassert only (full deferred clear flashes white).
        reassert_saved_material(&existing, &state);
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_focus();
        if let Some(pid) = focus {
            let _ = app.emit("settings-focus-plugin", pid);
        }
        if let Some(n) = nav {
            let _ = app.emit("settings-focus-nav", n);
        }
        return Ok(());
    }

    // First create: tear down sibling chrome WebViews (hide alone left status-menu
    // alive while settings built → pump hang). Creating settings while another
    // chrome WebView is settling hung IsHungAppWindow ~3s after build DONE.
    for label in ["status-menu-popup", "tray-popup", "chrome-hover-tip"] {
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.hide();
            let _ = w.close();
        }
        emit_chrome_popup_closed(&app, label);
    }
    crate::win32::click_trace::log("rust", "open_settings NEED_CREATE async-delay");
    async_delay_ms(300).await;
    if app.get_webview_window("settings").is_some() {
        return Ok(());
    }
    crate::win32::click_trace::log("rust", "open_settings after delay, build");
    let focus_js = focus
        .as_ref()
        .map(|pid| {
            format!(
                "window.__WH_SETTINGS_FOCUS_PLUGIN__ = {};",
                serde_json::to_string(pid).unwrap_or_else(|_| "null".into())
            )
        })
        .unwrap_or_default();
    let nav_js = nav
        .as_ref()
        .map(|n| {
            format!(
                "window.__WH_SETTINGS_FOCUS_NAV__ = {};",
                serde_json::to_string(n).unwrap_or_else(|_| "null".into())
            )
        })
        .unwrap_or_default();
    let init = format!(
        r#"
      window.__WH_IS_SETTINGS__ = true;
      {focus_js}
      {nav_js}
      document.addEventListener('keydown', function (e) {{
        if (e.key === 'Escape') {{
          try {{ window.__TAURI__.core.invoke('close_settings_window'); }} catch (_) {{}}
        }}
      }});
    "#
    );
    let win = {
        let _guard = lock_webview_create("settings");
        if app.get_webview_window("settings").is_some() {
            return Ok(());
        }
        let wd = create_watchdog("settings");
        let built = WebviewWindowBuilder::new(
            &app,
            "settings",
            WebviewUrl::App("index.html?window=settings".into()),
        )
        .title("灵动岛设置")
        .inner_size(820.0, 560.0)
        .min_inner_size(720.0, 480.0)
        .resizable(true)
        .maximizable(true)
        .minimizable(true)
        .closable(true)
        .decorations(true)
        .transparent(true)
        .background_color(Color(0, 0, 0, 0))
        .always_on_top(false)
        .skip_taskbar(false)
        .center()
        .focused(true)
        .visible(false)
        .initialization_script(init)
        .build();
        finish_watchdog(&wd);
        built.map_err(|e| format!("settings create failed: {e}"))?
        // _guard dropped here — must not hold MutexGuard across await
    };
    // Defer show/focus past create. One-shot Mica only — no apply_prefs_deferred
    // multi-pass (that + Settings FE invoke previously hung ~3s after open).
    async_delay_ms(80).await;
    {
        let prefs = read_material_prefs(&state);
        let _ = crate::win32::material::apply_prefs(&win, &prefs);
    }
    let _ = win.show();
    let _ = win.set_focus();
    if let Some(pid) = focus {
        let _ = app.emit("settings-focus-plugin", pid);
    }
    if let Some(n) = nav {
        let _ = app.emit("settings-focus-nav", n);
    }
    crate::win32::click_trace::log("rust", "open_settings build DONE");
    Ok(())
}

#[tauri::command]
pub async fn close_settings_window(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("settings") {
        // 正常关闭即可；不要在同步路径里 hide+destroy 连环调用
        w.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

const TRAY_POPUP_W: f64 = 280.0;
/// Placeholder only — frontend measures + slide-reveals while still hidden.
const TRAY_POPUP_H: f64 = 320.0;

fn main_hwnd_raw(app: &AppHandle) -> isize {
    app.get_webview_window("main")
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as isize)
        .unwrap_or(0)
}

/// Keep chrome/plugin popups inside the work area (flip/clamp near screen edges).
fn fit_popup_xy(app: &AppHandle, x: f64, y: f64, w: f64, h: f64) -> (f64, f64) {
    crate::win32::popup_fit::fit_popup_origin(main_hwnd_raw(app), x, y, w, h)
}

const TRAY_POPUP_INIT: &str = r#"
  window.__WH_IS_TRAY_POPUP__ = true;
  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') {
      try { window.__TAURI__.core.invoke('close_tray_popup'); } catch (_) {}
    }
  });
"#;

fn ensure_tray_popup_window(
    app: &AppHandle,
    state: &MaterialState,
) -> Result<WebviewWindow, String> {
    if let Some(existing) = app.get_webview_window("tray-popup") {
        return Ok(existing);
    }
    let _guard = lock_webview_create("tray-popup");
    if let Some(existing) = app.get_webview_window("tray-popup") {
        return Ok(existing);
    }
    crate::win32::click_trace::log("rust", "ensure_tray_popup_window build START");
    let wd = create_watchdog("tray-popup");
    let built = WebviewWindowBuilder::new(
        app,
        "tray-popup",
        WebviewUrl::App("index.html?window=tray".into()),
    )
    .title("已收纳")
    .inner_size(TRAY_POPUP_W, TRAY_POPUP_H)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .closable(true)
    .decorations(false)
    .transparent(true)
    .background_color(Color(0, 0, 0, 0))
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .visible(false)
    .initialization_script(TRAY_POPUP_INIT)
    .build();
    finish_watchdog(&wd);
    let win = built.map_err(|e| format!("open tray popup failed: {e}"))?;

    // One-shot material — never apply_prefs_deferred (5× mica hung the pump on chevron).
    {
        let prefs = read_material_prefs(state);
        let _ = crate::win32::material::apply_prefs(&win, &prefs);
    }
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
    }
    crate::win32::click_trace::log("rust", "ensure_tray_popup_window build DONE");
    Ok(win)
}

/// Optional: create hidden tray-popup after READY (not on boot critical path).
#[allow(dead_code)]
pub fn prewarm_tray_popup(app: &AppHandle) {
    if app.get_webview_window("tray-popup").is_some() {
        eprintln!("[boot] tray-popup: already exists");
        return;
    }
    let Some(state) = app.try_state::<MaterialState>() else {
        eprintln!("[boot] tray-popup: SKIP (MaterialState missing)");
        return;
    };
    match ensure_tray_popup_window(app, &*state) {
        Ok(_) => eprintln!("[boot] tray-popup: prewarmed"),
        Err(e) => eprintln!("[boot] tray-popup: prewarm failed: {e}"),
    }
}

#[allow(dead_code)]
fn ensure_chrome_hover_tip_window(app: &AppHandle, state: &MaterialState) -> Result<(), String> {
    if app.get_webview_window("chrome-hover-tip").is_some() {
        return Ok(());
    }
    let _guard = lock_webview_create("chrome-hover-tip");
    if app.get_webview_window("chrome-hover-tip").is_some() {
        return Ok(());
    }
    crate::win32::click_trace::log("rust", "ensure_chrome_hover_tip_window build START");
    let wd = create_watchdog("chrome-hover-tip");
    let init = r#"
      window.__WH_IS_CHROME_HOVER_TIP__ = true;
    "#;
    let built = WebviewWindowBuilder::new(
        app,
        "chrome-hover-tip",
        WebviewUrl::App("index.html?window=chrome-tip".into()),
    )
    .title("提示")
    .inner_size(CHROME_HOVER_TIP_MEASURE_W, CHROME_HOVER_TIP_MEASURE_H)
    .position(CHROME_HOVER_TIP_PARK_X, CHROME_HOVER_TIP_PARK_Y)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .closable(false)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .background_color(Color(0, 0, 0, 0))
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .visible(false)
    .initialization_script(init)
    .build();
    finish_watchdog(&wd);
    let win = built.map_err(|e| format!("prewarm chrome-hover-tip failed: {e}"))?;
    park_chrome_hover_tip(&win);
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
    }
    let _ = win.set_shadow(false);
    apply_chrome_hover_tip_material(&win, state);
    // Always click-through until commit decides interactive — avoids covering the
    // desktop with a transparent tip HWND that eats / passes hits.
    let _ = win.set_ignore_cursor_events(true);
    let _ = win.hide();
    crate::win32::click_trace::log("rust", "ensure_chrome_hover_tip_window build DONE");
    Ok(())
}

/// DISABLED: background-thread WebView create freezes the UI pump.
/// Proven by click-trace: hang at ensure_status_menu_popup_window build
/// (Responding=False → 穿透). Chrome popups create on first click via async await.
#[allow(dead_code)]
pub fn spawn_chrome_popup_prewarm(_app: AppHandle) {
    crate::win32::click_trace::log("boot", "chrome-prewarm DISABLED (avoids create hang)");
}

/// 独立窄高托盘弹窗：与设置/插件共用材质配置。
/// Kept invisible until the webview fits content — same path as status menu.
/// HWND is reused across opens — never destroy on dismiss (WebView2 deadlock).
/// First create is deferred off the IPC reply path.
#[tauri::command]
pub async fn open_tray_popup(
    app: AppHandle,
    state: State<'_, MaterialState>,
    x: f64,
    y: f64,
) -> Result<(), String> {
    if !crate::win32::tray::tray_boot_enabled() {
        return Err("tray disabled (hang A/B)".into());
    }
    // Chevron open: kick light catch-up so late/admin trays (PixPin…) appear without waiting.
    std::thread::Builder::new()
        .name("tray-refresh-on-open".into())
        .spawn(|| crate::win32::tray::request_refresh())
        .ok();
    let (x, y) = fit_popup_xy(&app, x, y, TRAY_POPUP_W, TRAY_POPUP_H);
    crate::win32::click_trace::log("rust", &format!("open_tray_popup enter x={x:.0} y={y:.0}"));
    close_sibling_popups(&app, "tray-popup");

    if let Some(win) = app.get_webview_window("tray-popup") {
        // Soft reassert only — no deferred mica storm on every chevron click.
        reassert_saved_material(&win, &state);
        let _ = win.hide();
        let _ = win.set_size(LogicalSize::new(TRAY_POPUP_W, TRAY_POPUP_H));
        let _ = win.set_position(LogicalPosition::new(x, y));
        let _ = win.unminimize();
        mark_popup_visible("tray-popup", true);
        let _ = app.emit("tray-popup-opened", ());
        crate::win32::click_trace::log("rust", "open_tray_popup REUSE done");
        return Ok(());
    }

    // First create: yield past IPC, then build once (same class as settings hang).
    crate::win32::click_trace::log("rust", "open_tray_popup NEED_CREATE async-delay");
    async_delay_ms(250).await;
    if let Some(win) = app.get_webview_window("tray-popup") {
        reassert_saved_material(&win, &state);
        let _ = win.hide();
        let _ = win.set_size(LogicalSize::new(TRAY_POPUP_W, TRAY_POPUP_H));
        let _ = win.set_position(LogicalPosition::new(x, y));
        let _ = win.unminimize();
        mark_popup_visible("tray-popup", true);
        let _ = app.emit("tray-popup-opened", ());
        return Ok(());
    }
    crate::win32::click_trace::log("rust", "open_tray_popup after delay, ensure");
    let win = ensure_tray_popup_window(&app, &state)?;
    async_delay_ms(60).await;
    let _ = win.hide();
    let _ = win.set_size(LogicalSize::new(TRAY_POPUP_W, TRAY_POPUP_H));
    let _ = win.set_position(LogicalPosition::new(x, y));
    let _ = win.unminimize();
    mark_popup_visible("tray-popup", true);
    let _ = app.emit("tray-popup-opened", ());
    crate::win32::click_trace::log("rust", "open_tray_popup CREATE done");
    Ok(())
}

#[tauri::command]
pub async fn close_tray_popup(app: AppHandle) -> Result<(), String> {
    hide_chrome_popup(&app, "tray-popup");
    Ok(())
}

#[tauri::command]
pub fn is_tray_popup_open(_app: AppHandle) -> bool {
    let _s = crate::win32::click_trace::Scope::enter("rust", "is_tray_popup_open");
    popup_visible("tray-popup")
}

const STATUS_MENU_POPUP_W: f64 = 200.0;
/// Placeholder only — frontend measures + fits while still hidden, then shows.
const STATUS_MENU_POPUP_H: f64 = 340.0;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusMenuFoldItem {
    pub plugin_id: String,
    pub label: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusMenuOpenPayload {
    from_dock: bool,
    item_id: Option<String>,
    /// Pinned tile id to insert a separator after (gap / “在右侧”).
    after_item_id: Option<String>,
    pin_bottom: Option<f64>,
    /// Shortcuts ⋯ overflow list (cross-webview; in-memory bus does not share).
    fold_items: Vec<StatusMenuFoldItem>,
    /// Which shortcuts wing opened the fold menu (`left` | `right`).
    fold_side: Option<String>,
    /// Dual shortcuts mode — fold menu can drag items to the other wing.
    fold_dual: bool,
}

fn status_menu_init_script(payload: &StatusMenuOpenPayload) -> String {
    let from_dock = if payload.from_dock { "true" } else { "false" };
    let item_id = payload
        .item_id
        .as_ref()
        .map(|id| serde_json::to_string(id).unwrap_or_else(|_| "null".into()))
        .unwrap_or_else(|| "null".into());
    let after_item_id = payload
        .after_item_id
        .as_ref()
        .map(|id| serde_json::to_string(id).unwrap_or_else(|_| "null".into()))
        .unwrap_or_else(|| "null".into());
    let pin_bottom = payload
        .pin_bottom
        .map(|n| n.to_string())
        .unwrap_or_else(|| "null".into());
    let fold_items = serde_json::to_string(&payload.fold_items).unwrap_or_else(|_| "[]".into());
    let fold_side = payload
        .fold_side
        .as_ref()
        .map(|s| serde_json::to_string(s).unwrap_or_else(|_| "null".into()))
        .unwrap_or_else(|| "null".into());
    let fold_dual = if payload.fold_dual { "true" } else { "false" };
    format!(
        r#"
      window.__WH_IS_STATUS_MENU_POPUP__ = true;
      window.__WH_STATUS_MENU_FROM_DOCK__ = {from_dock};
      window.__WH_STATUS_MENU_ITEM_ID__ = {item_id};
      window.__WH_STATUS_MENU_AFTER_ITEM_ID__ = {after_item_id};
      window.__WH_STATUS_MENU_PIN_BOTTOM__ = {pin_bottom};
      window.__WH_STATUS_MENU_FOLD_ITEMS__ = {fold_items};
      window.__WH_STATUS_MENU_FOLD_SIDE__ = {fold_side};
      window.__WH_STATUS_MENU_FOLD_DUAL__ = {fold_dual};
      document.addEventListener('keydown', function (e) {{
        if (e.key === 'Escape') {{
          try {{ window.__TAURI__.core.invoke('close_status_menu_popup'); }} catch (_) {{}}
        }}
      }});
    "#
    )
}

fn apply_status_menu_payload(win: &WebviewWindow, payload: &StatusMenuOpenPayload) {
    let from_dock = if payload.from_dock { "true" } else { "false" };
    let item_id = payload
        .item_id
        .as_ref()
        .map(|id| serde_json::to_string(id).unwrap_or_else(|_| "null".into()))
        .unwrap_or_else(|| "null".into());
    let after_item_id = payload
        .after_item_id
        .as_ref()
        .map(|id| serde_json::to_string(id).unwrap_or_else(|_| "null".into()))
        .unwrap_or_else(|| "null".into());
    let pin_bottom = payload
        .pin_bottom
        .map(|n| n.to_string())
        .unwrap_or_else(|| "null".into());
    let fold_items = serde_json::to_string(&payload.fold_items).unwrap_or_else(|_| "[]".into());
    let fold_side = payload
        .fold_side
        .as_ref()
        .map(|s| serde_json::to_string(s).unwrap_or_else(|_| "null".into()))
        .unwrap_or_else(|| "null".into());
    let fold_dual = if payload.fold_dual { "true" } else { "false" };
    let _ = win.eval(&format!(
        "window.__WH_STATUS_MENU_FROM_DOCK__ = {from_dock}; window.__WH_STATUS_MENU_ITEM_ID__ = {item_id}; window.__WH_STATUS_MENU_AFTER_ITEM_ID__ = {after_item_id}; window.__WH_STATUS_MENU_PIN_BOTTOM__ = {pin_bottom}; window.__WH_STATUS_MENU_FOLD_ITEMS__ = {fold_items}; window.__WH_STATUS_MENU_FOLD_SIDE__ = {fold_side}; window.__WH_STATUS_MENU_FOLD_DUAL__ = {fold_dual};"
    ));
}

fn ensure_status_menu_popup_window(
    app: &AppHandle,
    state: &MaterialState,
    payload: &StatusMenuOpenPayload,
) -> Result<WebviewWindow, String> {
    if let Some(existing) = app.get_webview_window("status-menu-popup") {
        return Ok(existing);
    }
    let _guard = lock_webview_create("status-menu-popup");
    if let Some(existing) = app.get_webview_window("status-menu-popup") {
        return Ok(existing);
    }
    crate::win32::click_trace::log("rust", "ensure_status_menu_popup_window build START");
    let wd = create_watchdog("status-menu-popup");
    let built = WebviewWindowBuilder::new(
        app,
        "status-menu-popup",
        WebviewUrl::App("index.html?window=status-menu".into()),
    )
    .title("")
    .inner_size(STATUS_MENU_POPUP_W, STATUS_MENU_POPUP_H)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .closable(true)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .background_color(Color(0, 0, 0, 0))
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .visible(false)
    .initialization_script(status_menu_init_script(payload))
    .build();
    finish_watchdog(&wd);
    let win = built.map_err(|e| format!("open status menu popup failed: {e}"))?;

    let _ = win.set_shadow(false);
    apply_saved_material(&win, state);
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
        #[cfg(windows)]
        crate::win32::blur_glass::strip_frameless_popup_titlebar(hwnd.0 as isize);
    }
    crate::win32::click_trace::log("rust", "ensure_status_menu_popup_window build DONE");
    Ok(win)
}

/// 左侧状态菜单弹窗：与插件/托盘共用 MicaAlt 材质与深浅色。
/// Kept invisible until the webview fits content — avoids 80→full height stutter.
/// First create is deferred off the IPC reply path (inline build → 未响应).
#[tauri::command]
pub async fn open_status_menu_popup(
    app: AppHandle,
    state: State<'_, MaterialState>,
    x: f64,
    y: f64,
    from_dock: Option<bool>,
    item_id: Option<String>,
    after_item_id: Option<String>,
    pin_bottom: Option<f64>,
    fold_items: Option<Vec<StatusMenuFoldItem>>,
    fold_side: Option<String>,
    fold_dual: Option<bool>,
) -> Result<(), String> {
    let (x, y) = fit_popup_xy(&app, x, y, STATUS_MENU_POPUP_W, STATUS_MENU_POPUP_H);
    crate::win32::click_trace::log(
        "rust",
        &format!(
            "open_status_menu_popup enter x={x:.0} y={y:.0} from_dock={}",
            from_dock.unwrap_or(false)
        ),
    );
    // Hold BEFORE closing siblings / focus moves — AutoHide leave must not win.
    let from_dock = from_dock.unwrap_or(false);
    if from_dock {
        set_dock_menu_hold(&app, true);
    }

    close_sibling_popups(&app, "status-menu-popup");
    #[cfg(windows)]
    crate::win32::blur_glass::strip_dock_windows(&app);

    let fold_items = fold_items
        .unwrap_or_default()
        .into_iter()
        .filter(|it| !it.plugin_id.trim().is_empty() && !it.label.trim().is_empty())
        .map(|it| StatusMenuFoldItem {
            plugin_id: it.plugin_id.trim().to_string(),
            label: it.label.trim().to_string(),
        })
        .collect();
    let fold_side = fold_side
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| s == "left" || s == "right");
    let fold_dual = fold_dual.unwrap_or(false) && fold_side.is_some();

    let payload = StatusMenuOpenPayload {
        from_dock,
        item_id: item_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        after_item_id: after_item_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        pin_bottom,
        fold_items,
        fold_side,
        fold_dual,
    };

    if let Some(existing) = app.get_webview_window("status-menu-popup") {
        apply_saved_material(&existing, &state);
        let _ = existing.hide();
        let _ = existing.set_size(LogicalSize::new(STATUS_MENU_POPUP_W, STATUS_MENU_POPUP_H));
        let _ = existing.set_position(LogicalPosition::new(x, y));
        let _ = existing.unminimize();
        #[cfg(windows)]
        if let Ok(hwnd) = existing.hwnd() {
            crate::win32::blur_glass::strip_frameless_popup_titlebar(hwnd.0 as isize);
        }
        apply_status_menu_payload(&existing, &payload);
        mark_popup_visible("status-menu-popup", true);
        let _ = app.emit("status-menu-popup-opened", &payload);
        return Ok(());
    }

    crate::win32::click_trace::log("rust", "open_status_menu_popup NEED_CREATE async-delay");
    async_delay_ms(100).await;
    crate::win32::click_trace::log("rust", "open_status_menu_popup after delay, ensure");
    let win = ensure_status_menu_popup_window(&app, &state, &payload)?;
    let _ = win.set_position(LogicalPosition::new(x, y));
    let _ = win.set_size(LogicalSize::new(STATUS_MENU_POPUP_W, STATUS_MENU_POPUP_H));
    apply_status_menu_payload(&win, &payload);
    mark_popup_visible("status-menu-popup", true);
    let _ = app.emit("status-menu-popup-opened", &payload);
    Ok(())
}

#[tauri::command]
pub async fn close_status_menu_popup(app: AppHandle) -> Result<(), String> {
    hide_chrome_popup(&app, "status-menu-popup");
    Ok(())
}

#[tauri::command]
pub fn is_status_menu_popup_open(_app: AppHandle) -> bool {
    let _s = crate::win32::click_trace::Scope::enter("rust", "is_status_menu_popup_open");
    popup_visible("status-menu-popup")
}

const DOCK_ADD_ICON_POPUP_W: f64 = 280.0;
/// Placeholder only — frontend `fitPopupToContent` resizes to content.
const DOCK_ADD_ICON_POPUP_H: f64 = 120.0;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DockAddIconOpenPayload {
    after_item_id: Option<String>,
    pin_bottom: Option<f64>,
}

fn dock_add_icon_init_script(payload: &DockAddIconOpenPayload) -> String {
    let after_item_id = payload
        .after_item_id
        .as_ref()
        .map(|id| serde_json::to_string(id).unwrap_or_else(|_| "null".into()))
        .unwrap_or_else(|| "null".into());
    let pin_bottom = payload
        .pin_bottom
        .map(|n| n.to_string())
        .unwrap_or_else(|| "null".into());
    format!(
        r#"
      window.__WH_IS_DOCK_ADD_ICON_POPUP__ = true;
      window.__WH_DOCK_ADD_ICON_AFTER_ITEM_ID__ = {after_item_id};
      window.__WH_DOCK_ADD_ICON_PIN_BOTTOM__ = {pin_bottom};
      document.addEventListener('keydown', function (e) {{
        if (e.key === 'Escape') {{
          try {{ window.__TAURI__.core.invoke('close_dock_add_icon_popup'); }} catch (_) {{}}
        }}
      }});
    "#
    )
}

fn apply_dock_add_icon_payload(win: &WebviewWindow, payload: &DockAddIconOpenPayload) {
    let after_item_id = payload
        .after_item_id
        .as_ref()
        .map(|id| serde_json::to_string(id).unwrap_or_else(|_| "null".into()))
        .unwrap_or_else(|| "null".into());
    let pin_bottom = payload
        .pin_bottom
        .map(|n| n.to_string())
        .unwrap_or_else(|| "null".into());
    let _ = win.eval(&format!(
        "window.__WH_DOCK_ADD_ICON_AFTER_ITEM_ID__ = {after_item_id}; window.__WH_DOCK_ADD_ICON_PIN_BOTTOM__ = {pin_bottom};"
    ));
}

fn ensure_dock_add_icon_popup_window(
    app: &AppHandle,
    state: &MaterialState,
    payload: &DockAddIconOpenPayload,
) -> Result<WebviewWindow, String> {
    if let Some(existing) = app.get_webview_window("dock-add-icon-popup") {
        return Ok(existing);
    }
    let _guard = lock_webview_create("dock-add-icon-popup");
    if let Some(existing) = app.get_webview_window("dock-add-icon-popup") {
        return Ok(existing);
    }
    let wd = create_watchdog("dock-add-icon-popup");
    let built = WebviewWindowBuilder::new(
        app,
        "dock-add-icon-popup",
        WebviewUrl::App("index.html?window=dock-add-icon".into()),
    )
    .title("")
    .inner_size(DOCK_ADD_ICON_POPUP_W, DOCK_ADD_ICON_POPUP_H)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .closable(true)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .background_color(Color(0, 0, 0, 0))
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(true)
    .visible(false)
    .initialization_script(dock_add_icon_init_script(payload))
    .build();
    finish_watchdog(&wd);
    let win = built.map_err(|e| format!("dock-add-icon create failed: {e}"))?;
    apply_saved_material(&win, state);
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
        #[cfg(windows)]
        crate::win32::blur_glass::strip_frameless_popup_titlebar(hwnd.0 as isize);
    }
    Ok(win)
}

/// Dock blank-space “添加图标” picker — same MicaAlt chrome shell as Wi‑Fi / IME.
#[tauri::command]
pub async fn open_dock_add_icon_popup(
    app: AppHandle,
    state: State<'_, MaterialState>,
    x: f64,
    y: f64,
    after_item_id: Option<String>,
    pin_bottom: Option<f64>,
) -> Result<(), String> {
    set_dock_menu_hold(&app, true);
    // Mark early so status-menu close (sibling) does not drop AutoHide hold.
    mark_popup_visible("dock-add-icon-popup", true);

    close_sibling_popups(&app, "dock-add-icon-popup");
    #[cfg(windows)]
    crate::win32::blur_glass::strip_dock_windows(&app);

    let payload = DockAddIconOpenPayload {
        after_item_id: after_item_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        pin_bottom,
    };

    let (x, y) = fit_popup_xy(&app, x, y, DOCK_ADD_ICON_POPUP_W, DOCK_ADD_ICON_POPUP_H);

    if let Some(existing) = app.get_webview_window("dock-add-icon-popup") {
        apply_saved_material(&existing, &state);
        let _ = existing.hide();
        let _ = existing.set_size(LogicalSize::new(DOCK_ADD_ICON_POPUP_W, DOCK_ADD_ICON_POPUP_H));
        let _ = existing.set_position(LogicalPosition::new(x, y));
        let _ = existing.unminimize();
        #[cfg(windows)]
        if let Ok(hwnd) = existing.hwnd() {
            crate::win32::blur_glass::strip_frameless_popup_titlebar(hwnd.0 as isize);
        }
        apply_dock_add_icon_payload(&existing, &payload);
        mark_popup_visible("dock-add-icon-popup", true);
        let _ = app.emit("dock-add-icon-popup-opened", &payload);
        return Ok(());
    }

    async_delay_ms(100).await;
    let win = ensure_dock_add_icon_popup_window(&app, &state, &payload)?;
    let _ = win.set_position(LogicalPosition::new(x, y));
    let _ = win.set_size(LogicalSize::new(DOCK_ADD_ICON_POPUP_W, DOCK_ADD_ICON_POPUP_H));
    apply_dock_add_icon_payload(&win, &payload);
    mark_popup_visible("dock-add-icon-popup", true);
    let _ = app.emit("dock-add-icon-popup-opened", &payload);
    Ok(())
}

#[tauri::command]
pub async fn close_dock_add_icon_popup(app: AppHandle) -> Result<(), String> {
    hide_chrome_popup(&app, "dock-add-icon-popup");
    Ok(())
}

#[tauri::command]
pub fn is_dock_add_icon_popup_open(_app: AppHandle) -> bool {
    popup_visible("dock-add-icon-popup")
}

const PLUGIN_POPUP_W: f64 = 320.0;
const PLUGIN_POPUP_H: f64 = 480.0;
const PLUGIN_POPUP_W_MIN: f64 = 280.0;
/// Large canvas plugins (e.g. Excalidraw) need room beyond the old 720 cap.
const PLUGIN_POPUP_W_MAX: f64 = 2400.0;
const PLUGIN_POPUP_H_MIN: f64 = 320.0;
const PLUGIN_POPUP_H_MAX: f64 = 1600.0;

fn clamp_popup_size(w: f64, h: f64) -> (f64, f64) {
    (
        w.clamp(PLUGIN_POPUP_W_MIN, PLUGIN_POPUP_W_MAX),
        h.clamp(PLUGIN_POPUP_H_MIN, PLUGIN_POPUP_H_MAX),
    )
}

/// Last normal (non–windowed-fullscreen) popup geometry for restore.
static PLUGIN_POPUP_RESTORE: Mutex<Option<(f64, f64, f64, f64)>> = Mutex::new(None);

/// Fit plugin popup to the monitor work area (taskbar-safe = 窗口化全屏, not exclusive).
#[cfg(windows)]
fn apply_plugin_popup_work_area(win: &WebviewWindow) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };

    let hwnd = win
        .hwnd()
        .map_err(|e| format!("plugin popup hwnd: {e}"))?;
    unsafe {
        let mon = MonitorFromWindow(HWND(hwnd.0 as *mut _), MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(mon, &mut info).as_bool() {
            return Err("GetMonitorInfoW failed".into());
        }
        let r = info.rcWork;
        let w = (r.right - r.left).max(320);
        let h = (r.bottom - r.top).max(240);
        win.set_position(PhysicalPosition::new(r.left, r.top))
            .map_err(|e| e.to_string())?;
        win.set_size(PhysicalSize::new(w as u32, h as u32))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn apply_plugin_popup_work_area(win: &WebviewWindow) -> Result<(), String> {
    let mon = win
        .current_monitor()
        .ok()
        .flatten()
        .ok_or_else(|| "no monitor".to_string())?;
    let scale = mon.scale_factor();
    let size = mon.size();
    let pos = mon.position();
    let w = ((size.width as f64) / scale).round().max(320.0);
    let h = ((size.height as f64) / scale).round().max(240.0);
    let x = (pos.x as f64) / scale;
    let y = (pos.y as f64) / scale;
    win.set_position(LogicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    win.set_size(LogicalSize::new(w, h))
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn remember_plugin_popup_geometry(win: &WebviewWindow) {
    let Ok(size) = win.inner_size() else {
        return;
    };
    let Ok(pos) = win.outer_position() else {
        return;
    };
    let scale = win.scale_factor().unwrap_or(1.0);
    let w = size.width as f64 / scale;
    let h = size.height as f64 / scale;
    let x = pos.x as f64 / scale;
    let y = pos.y as f64 / scale;
    if let Ok(mut g) = PLUGIN_POPUP_RESTORE.lock() {
        *g = Some((x, y, w, h));
    }
}

fn restore_plugin_popup_geometry(app: &AppHandle, win: &WebviewWindow, plugin_id: &str) {
    let saved = PLUGIN_POPUP_RESTORE
        .lock()
        .ok()
        .and_then(|g| *g);
    if let Some((x, y, w, h)) = saved {
        let (cw, ch) = clamp_popup_size(w, h);
        let (x, y) = fit_popup_xy(app, x, y, cw, ch);
        let _ = win.set_size(LogicalSize::new(cw, ch));
        let _ = win.set_position(LogicalPosition::new(x, y));
        return;
    }
    let (cw, ch) = resolve_plugin_popup_size(plugin_id, None, None);
    let _ = win.set_size(LogicalSize::new(cw, ch));
}

fn number_from_settings(v: &serde_json::Value, key: &str) -> Option<f64> {
    let n = v.get(key)?;
    if let Some(x) = n.as_f64() {
        return Some(x);
    }
    if let Some(x) = n.as_i64() {
        return Some(x as f64);
    }
    if let Some(s) = n.as_str() {
        return s.parse().ok();
    }
    None
}

/// Resolve popup size: invoke args → plugin settings popupWidth/Height → defaults.
fn resolve_plugin_popup_size(
    plugin_id: &str,
    width: Option<f64>,
    height: Option<f64>,
) -> (f64, f64) {
    let mut w = width.filter(|x| x.is_finite() && *x > 0.0);
    let mut h = height.filter(|x| x.is_finite() && *x > 0.0);
    if w.is_none() || h.is_none() {
        if let Ok(Some(raw)) =
            crate::db::with_conn(|c| crate::db::plugin_get_system(c, plugin_id, "__settings"))
        {
            if w.is_none() {
                w = number_from_settings(&raw, "popupWidth");
            }
            if h.is_none() {
                h = number_from_settings(&raw, "popupHeight");
            }
        }
        // Fill from manifest defaults when still missing
        if w.is_none() || h.is_none() {
            if let Some(rec) = crate::plugin_install::find_installed_plugin(plugin_id) {
                if let Some(settings) = rec.manifest.get("settings").and_then(|s| s.as_array()) {
                    for field in settings {
                        let key = field.get("key").and_then(|k| k.as_str()).unwrap_or("");
                        if w.is_none() && key == "popupWidth" {
                            w = field.get("default").and_then(|d| {
                                d.as_f64()
                                    .or_else(|| d.as_i64().map(|i| i as f64))
                                    .or_else(|| d.as_str().and_then(|s| s.parse().ok()))
                            });
                        }
                        if h.is_none() && key == "popupHeight" {
                            h = field.get("default").and_then(|d| {
                                d.as_f64()
                                    .or_else(|| d.as_i64().map(|i| i as f64))
                                    .or_else(|| d.as_str().and_then(|s| s.parse().ok()))
                            });
                        }
                    }
                }
            }
        }
    }
    clamp_popup_size(
        w.unwrap_or(PLUGIN_POPUP_W),
        h.unwrap_or(PLUGIN_POPUP_H),
    )
}

fn popup_plugin_id_of(win: &WebviewWindow) -> Option<String> {
    let url = win.url().ok()?;
    let s = url.as_str();
    for part in s.split(['?', '&']) {
        if let Some(id) = part.strip_prefix("plugin=") {
            let id = id.split('#').next().unwrap_or(id);
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
    }
    None
}

fn urlencoding_minimal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn plugin_popup_path(
    record: &crate::plugin_install::InstalledPluginRecord,
) -> Result<PathBuf, String> {
    let entry = record
        .manifest
        .get("entry")
        .and_then(|e| e.get("popup"))
        .and_then(|s| s.as_str())
        .ok_or_else(|| "plugin has no popup entry".to_string())?;
    let popup = PathBuf::from(&record.path).join(entry);
    if !popup.is_file() {
        return Err(format!("plugin popup missing: {}", popup.to_string_lossy()));
    }
    Ok(popup)
}

const PLUGIN_POPUP_LABEL: &str = "plugin-popup";
/// Decorated OS window (settings-like Mica caption). Separate label so material
/// routing never depends on `is_decorated()` / URL parsing quirks.
const PLUGIN_WINDOW_LABEL: &str = "plugin-window";

fn close_plugin_surfaces(app: &AppHandle) {
    for label in [PLUGIN_POPUP_LABEL, PLUGIN_WINDOW_LABEL] {
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.hide();
            let _ = w.close();
        }
    }
    #[cfg(windows)]
    crate::win32::ambient::set_ambient_sample_target(None, 0);
    mark_popup_visible("plugin-popup", false);
    let _ = app.emit("plugin-popup-closed", ());
}

fn plugin_surface_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(PLUGIN_WINDOW_LABEL)
        .or_else(|| app.get_webview_window(PLUGIN_POPUP_LABEL))
}

/// 通用插件弹窗：宿主 App 壳（有 Tauri IPC）+ 注入 window.hub，再由前端加载插件静态资源。
#[tauri::command]
pub async fn open_plugin_popup(
    app: AppHandle,
    state: State<'_, MaterialState>,
    plugin_id: String,
    x: f64,
    y: f64,
    prefer_group_id: Option<String>,
    width: Option<f64>,
    height: Option<f64>,
    windowed_fullscreen: Option<bool>,
    resizable: Option<bool>,
    // Like settings: OS title bar / min / max / close → label `plugin-window`.
    native_frame: Option<bool>,
) -> Result<(), String> {
    close_sibling_popups(&app, PLUGIN_POPUP_LABEL);

    let record = crate::plugin_install::find_installed_plugin(&plugin_id)
        .ok_or_else(|| "plugin not installed".to_string())?;
    if !record.enabled {
        return Err("plugin disabled".into());
    }
    crate::plugin_hub::assert_capability(&plugin_id, "popup")?;

    let want_native = native_frame.unwrap_or(false);
    let want_fs = windowed_fullscreen.unwrap_or(false);
    let want_resize = resizable.unwrap_or(want_fs || want_native);
    let (popup_w, popup_h) = resolve_plugin_popup_size(&plugin_id, width, height);
    // Framed / fullscreen surfaces manage their own geometry; floating popups
    // must stay inside the monitor work area (right shortcuts / screen corners).
    let (x, y) = if want_native || want_fs {
        (x, y)
    } else {
        fit_popup_xy(&app, x, y, popup_w, popup_h)
    };

    let popup = plugin_popup_path(&record)?;
    let parent = popup
        .parent()
        .ok_or_else(|| "plugin popup has no parent directory".to_string())?;
    let _ = app.asset_protocol_scope().allow_directory(parent, true);

    let title = record
        .manifest
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("插件")
        .to_string();

    let target_label = if want_native {
        PLUGIN_WINDOW_LABEL
    } else {
        PLUGIN_POPUP_LABEL
    };

    // Idempotent: same surface + same plugin already visible.
    if let Some(existing) = app.get_webview_window(target_label) {
        let already = popup_visible("plugin-popup")
            && popup_plugin_id_of(&existing) == Some(plugin_id.clone());
        if already {
            if let Some(gid) = prefer_group_id.as_ref().filter(|s| !s.is_empty()) {
                let _ = app.emit("plugin-popup-prefer-group", gid);
            }
            let _ = existing.set_resizable(want_resize);
            if want_native {
                let _ = existing.set_always_on_top(false);
                let _ = existing.set_skip_taskbar(false);
                if want_fs {
                    let _ = existing.maximize();
                }
                reassert_saved_material(&existing, &state);
                schedule_plugin_window_mica_refresh(&app);
                #[cfg(windows)]
                if let Ok(hwnd) = existing.hwnd() {
                    // Sample OS title bar (same as Chrome/VS Code), not canvas.
                    crate::win32::ambient::set_ambient_sample_target(
                        Some(hwnd.0 as isize),
                        0,
                    );
                }
            } else if want_fs {
                remember_plugin_popup_geometry(&existing);
                apply_plugin_popup_work_area(&existing)?;
                let _ = existing.set_always_on_top(false);
                let _ = existing.set_skip_taskbar(false);
            } else {
                let _ = existing.set_size(LogicalSize::new(popup_w, popup_h));
                let _ = existing.set_position(LogicalPosition::new(x, y));
                let _ = existing.set_always_on_top(true);
                let _ = existing.set_skip_taskbar(true);
            }
            let _ = existing.unminimize();
            let _ = existing.set_focus();
            mark_popup_visible("plugin-popup", true);
            let _ = app.emit("plugin-popup-opened", &plugin_id);
            return Ok(());
        }
    }

    // Switching popup ↔ window (or different plugin): tear down both surfaces.
    close_plugin_surfaces(&app);
    std::thread::sleep(std::time::Duration::from_millis(48));

    let mut url_s = if want_native {
        format!("index.html?window=plugin-window&plugin={plugin_id}")
    } else {
        format!("index.html?window=plugin-popup&plugin={plugin_id}")
    };
    if let Some(gid) = prefer_group_id.as_ref().filter(|s| !s.is_empty()) {
        url_s.push_str("&preferGroup=");
        url_s.push_str(&urlencoding_minimal(gid));
    }
    let url = WebviewUrl::App(url_s.into());
    let init = hub_init_script(&plugin_id);

    let win = if want_native {
        // Same builder + material path as settings (OS caption + system Mica 吸色).
        WebviewWindowBuilder::new(&app, PLUGIN_WINDOW_LABEL, url)
            .title(&title)
            .inner_size(popup_w.max(640.0), popup_h.max(420.0))
            .min_inner_size(640.0, 420.0)
            .resizable(true)
            .maximizable(true)
            .minimizable(true)
            .closable(true)
            .decorations(true)
            .transparent(true)
            .background_color(Color(0, 0, 0, 0))
            .always_on_top(false)
            .skip_taskbar(false)
            .center()
            .focused(true)
            .visible(false)
            .initialization_script(init)
            .build()
            .map_err(|e| format!("open plugin window failed: {e}"))?
    } else {
        WebviewWindowBuilder::new(&app, PLUGIN_POPUP_LABEL, url)
            .title(&title)
            .inner_size(popup_w, popup_h)
            .resizable(want_resize)
            .maximizable(false)
            .minimizable(false)
            .closable(true)
            .decorations(false)
            .transparent(true)
            .background_color(Color(0, 0, 0, 0))
            .always_on_top(!want_fs)
            .skip_taskbar(!want_fs)
            .focused(true)
            .visible(false)
            .initialization_script(init)
            .build()
            .map_err(|e| format!("open plugin popup failed: {e}"))?
    };

    if want_native {
        if want_fs {
            let _ = win.maximize();
        }
    } else if want_fs {
        remember_plugin_popup_geometry(&win);
        apply_plugin_popup_work_area(&win)?;
    } else {
        let _ = win.set_position(LogicalPosition::new(x, y));
    }
    apply_saved_material(&win, &state);
    if !want_native {
        if let Ok(hwnd) = win.hwnd() {
            crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
        }
    }
    let _ = win.show();
    let _ = win.set_focus();
    if want_native {
        reassert_saved_material(&win, &state);
        schedule_plugin_window_mica_refresh(&app);
        // Allow island ambient to sample this HWND when maximized (same-process
        // windows are otherwise excluded). Skip ~36px Host chrome → canvas.
        #[cfg(windows)]
        if let Ok(hwnd) = win.hwnd() {
            // Sample OS title bar (窗体顶栏), not canvas below caption.
            crate::win32::ambient::set_ambient_sample_target(Some(hwnd.0 as isize), 0);
            // Kick ambient watcher so island picks up canvas colors immediately.
            if let Some(main) = app.get_webview_window("main") {
                if let Ok(mh) = main.hwnd() {
                    let strip = crate::win32::ambient::sample(Some(mh.0 as isize));
                    let _ = app.emit("ambient-color", &strip);
                }
            }
            // WebView2 may reparent shortly after show — re-bind root HWND.
            let app_amb = app.clone();
            std::thread::spawn(move || {
                for ms in [120_u64, 320, 700] {
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                    let Some(w) = app_amb.get_webview_window(PLUGIN_WINDOW_LABEL) else {
                        return;
                    };
                    let Ok(hwnd) = w.hwnd() else {
                        continue;
                    };
                    crate::win32::ambient::set_ambient_sample_target(Some(hwnd.0 as isize), 0);
                    if let Some(main) = app_amb.get_webview_window("main") {
                        if let Ok(mh) = main.hwnd() {
                            let strip = crate::win32::ambient::sample(Some(mh.0 as isize));
                            let _ = app_amb.emit("ambient-color", &strip);
                        }
                    }
                }
            });
        }
    } else {
        #[cfg(windows)]
        crate::win32::ambient::set_ambient_sample_target(None, 0);
    }
    mark_popup_visible("plugin-popup", true);
    let _ = app.emit("plugin-popup-opened", &plugin_id);
    Ok(())
}

/// Detach frameless popup → native OS window without racing the closing webview's IPC reply.
/// Returns Ok immediately so the popup can finish the invoke; recreate runs on a short delay.
#[tauri::command]
pub async fn schedule_plugin_popup_as_window(
    app: AppHandle,
    plugin_id: String,
    width: Option<f64>,
    height: Option<f64>,
    windowed_fullscreen: Option<bool>,
) -> Result<(), String> {
    crate::plugin_hub::assert_capability(&plugin_id, "popup")?;
    let app2 = app.clone();
    let plugin_id2 = plugin_id;
    tauri::async_runtime::spawn(async move {
        // Deliver invoke result to the still-alive popup first.
        std::thread::sleep(std::time::Duration::from_millis(40));
        let state = app2.state::<MaterialState>();
        let _ = open_plugin_popup(
            app2.clone(),
            state,
            plugin_id2,
            0.0,
            0.0,
            None,
            width,
            height,
            windowed_fullscreen,
            Some(true),
            Some(true),
        )
        .await;
    });
    Ok(())
}

#[tauri::command]
pub async fn close_plugin_popup(app: AppHandle) -> Result<(), String> {
    close_plugin_surfaces(&app);
    Ok(())
}

#[tauri::command]
pub fn is_plugin_popup_open(_app: AppHandle) -> bool {
    popup_visible("plugin-popup")
}

/// Resize the open plugin popup (clamped). Used by canvas plugins.
#[tauri::command]
pub async fn resize_plugin_popup(
    app: AppHandle,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let win = plugin_surface_window(&app)
        .ok_or_else(|| "plugin popup not open".to_string())?;
    let (w, h) = clamp_popup_size(width, height);
    win.set_size(LogicalSize::new(w, h))
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Toggle 窗口化全屏：原生窗体用 maximize；无边框弹窗则铺满工作区。
#[tauri::command]
pub async fn set_plugin_popup_windowed_fullscreen(
    app: AppHandle,
    state: State<'_, MaterialState>,
    enabled: bool,
) -> Result<(), String> {
    let win = plugin_surface_window(&app)
        .ok_or_else(|| "plugin popup not open".to_string())?;
    let plugin_id = popup_plugin_id_of(&win).unwrap_or_default();
    let native = win.label() == PLUGIN_WINDOW_LABEL
        || crate::win32::blur_glass::is_native_frame_plugin_popup(&win);

    if native {
        if enabled {
            let _ = win.maximize();
        } else {
            let _ = win.unmaximize();
        }
        // Maximize repaints caption — keep settings-frame Mica.
        reassert_saved_material(&win, &state);
        if win.label() == PLUGIN_WINDOW_LABEL {
            schedule_plugin_window_mica_refresh(&app);
            #[cfg(windows)]
            if let Ok(hwnd) = win.hwnd() {
                crate::win32::ambient::set_ambient_sample_target(Some(hwnd.0 as isize), 0);
                if let Some(main) = app.get_webview_window("main") {
                    if let Ok(mh) = main.hwnd() {
                        let strip = crate::win32::ambient::sample(Some(mh.0 as isize));
                        let _ = app.emit("ambient-color", &strip);
                    }
                }
            }
        }
    } else if enabled {
        remember_plugin_popup_geometry(&win);
        apply_plugin_popup_work_area(&win)?;
        let _ = win.set_resizable(true);
        let _ = win.set_always_on_top(false);
        let _ = win.set_skip_taskbar(false);
    } else {
        restore_plugin_popup_geometry(&app, &win, &plugin_id);
        let _ = win.set_always_on_top(true);
        let _ = win.set_skip_taskbar(true);
    }
    let _ = win.set_focus();
    let _ = app.emit(
        "plugin-popup-windowed-fullscreen",
        serde_json::json!({ "enabled": enabled }),
    );
    Ok(())
}

pub(crate) fn load_material_prefs() -> crate::win32::material::MaterialPrefs {
    use crate::win32::material::MaterialPrefs;
    if let Ok(Some(v)) = crate::db::with_conn(|c| crate::db::material_get(c)) {
        if let Ok(prefs) = serde_json::from_value::<MaterialPrefs>(v) {
            return prefs.normalize();
        }
    }
    MaterialPrefs::default()
}

fn save_material_prefs(prefs: &crate::win32::material::MaterialPrefs) -> Result<(), String> {
    let v = serde_json::to_value(prefs).map_err(|e| e.to_string())?;
    crate::db::with_conn(|c| crate::db::material_set(c, &v))
}

pub struct MaterialState(pub std::sync::Mutex<crate::win32::material::MaterialPrefs>);

pub fn initial_material_state() -> MaterialState {
    MaterialState(std::sync::Mutex::new(load_material_prefs()))
}

fn read_material_prefs(state: &MaterialState) -> crate::win32::material::MaterialPrefs {
    state
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// Apply saved material to settings / tray / plugin / dock windows (unified).
fn apply_saved_material(window: &tauri::WebviewWindow, state: &MaterialState) {
    let prefs = read_material_prefs(state);
    crate::win32::material::apply_prefs_deferred(window, &prefs);
}

/// Soft reassert (settings / icon editor) — no clear cycle, avoids open/focus flash.
fn reassert_saved_material(window: &tauri::WebviewWindow, state: &MaterialState) {
    let prefs = read_material_prefs(state);
    let _ = crate::win32::material::reassert_prefs(window, &prefs);
}

pub fn apply_saved_material_pub(window: &tauri::WebviewWindow, state: &MaterialState) {
    apply_saved_material(window, state);
}

pub fn reassert_saved_material_pub(window: &tauri::WebviewWindow, state: &MaterialState) {
    reassert_saved_material(window, state);
}

/// Full DWM apply (not soft) — used after maximize when caption attrs are reset.
pub fn force_apply_saved_material_pub(window: &tauri::WebviewWindow, state: &MaterialState) {
    let prefs = read_material_prefs(state);
    let _ = crate::win32::material::apply_prefs(window, &prefs);
}

/// Debounced multi-pass refresh for `plugin-window` after maximize/restore.
pub fn schedule_plugin_window_mica_refresh(app: &AppHandle) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static GEN: AtomicU64 = AtomicU64::new(0);
    let gen = GEN.fetch_add(1, Ordering::Relaxed) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        for ms in [40_u64, 120, 280, 520] {
            std::thread::sleep(std::time::Duration::from_millis(ms));
            if GEN.load(Ordering::Relaxed) != gen {
                return;
            }
            let Some(w) = app.get_webview_window("plugin-window") else {
                return;
            };
            let Some(state) = app.try_state::<MaterialState>() else {
                return;
            };
            force_apply_saved_material_pub(&w, &*state);
        }
    });
}

/// Re-apply materials to every open popup after prefs change.
fn reapply_material_to_popups(app: &AppHandle, prefs: &crate::win32::material::MaterialPrefs) {
    for label in [
        "main",
        "island-bar-glass",
        "settings",
        "dock-icon-editor",
        "tray-popup",
        "plugin-popup",
        "plugin-window",
        "status-menu-popup",
        "input-lang-popup",
        "control-center-popup",
        "wifi-popup",
        "wifi-auth-popup",
        "chrome-hover-tip",
        "dock",
        "dock-glass",
    ] {
        if let Some(w) = app.get_webview_window(label) {
            if label == "main" || label == "island-bar-glass" {
                continue;
            }
            let _ = crate::win32::material::apply_prefs(&w, prefs);
        }
    }
    #[cfg(windows)]
    apply_main_window_material(app);
}

/// Top status bar Win32 glass (`bar_comp`, strip-clipped):
/// - Desktop: always on when `barGlass`
/// - Maximized: on until live ambient (`hwnd≠0`) owns chrome, then detach
///
/// Never full-HWND acrylic — island hover expands the client and would paint a slab.
pub fn apply_main_window_material(app: &AppHandle) {
    schedule_main_window_material(app, 120);
}

/// Setup / tests — caller MUST already be on the UI thread.
/// Do not use `run_on_main_thread` here (deadlocks if called from setup).
#[allow(dead_code)]
pub fn apply_main_window_material_now(app: &AppHandle) {
    apply_main_window_material_inner(app);
}

fn apply_main_window_material_inner(app: &AppHandle) {
    let prefs = load_material_prefs();
    let bar_glass = get_island_prefs().bar_glass;
    #[cfg(windows)]
    {
        // Per-window desktop scene is decided inside sync_inner — do not gate
        // all strips on primary's scene (secondary must keep the same frost).
        // Quiet only pauses main bar_comp attach; chrome-sat still syncs.
        let quiet = crate::win32::work_area::work_area_quiet();
        crate::win32::island_bar_glass::sync_inner(app, bar_glass, quiet, &prefs);
    }
    #[cfg(not(windows))]
    {
        let _ = (bar_glass, prefs, app);
    }
}

#[cfg(windows)]
fn schedule_main_window_material(app: &AppHandle, debounce_ms: u64) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static GEN: AtomicU64 = AtomicU64::new(0);
    let debounce_ms = if crate::win32::work_area::work_area_quiet() {
        debounce_ms.max(800)
    } else {
        debounce_ms
    };
    let gen = GEN.fetch_add(1, Ordering::Relaxed) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        if debounce_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(debounce_ms));
        }
        if GEN.load(Ordering::Relaxed) != gen {
            return;
        }
        let Some(main) = app.get_webview_window("main") else {
            return;
        };
        let _ = main.run_on_main_thread(move || {
            if GEN.load(Ordering::Relaxed) != gen {
                return;
            }
            apply_main_window_material_inner(&app);
        });
    });
}

#[cfg(not(windows))]
fn schedule_main_window_material(app: &AppHandle, _debounce_ms: u64) {
    apply_main_window_material_inner(app);
}

/// Keep glass strip geometry in sync after main HWND / island resize.
///
/// **Must be async** — sync invoke right after `resize_main_island` / SetWindowPos
/// deadlocks WebView2 (click-trace #106 before reassert → never ENTER → HUNG).
#[tauri::command]
pub async fn reassert_main_bar_geometry(
    app: AppHandle,
    island_width: Option<f64>,
    island_height: Option<f64>,
) -> Result<(), String> {
    crate::win32::click_trace::log(
        "rust",
        &format!(
            "reassert_main_bar_geometry enter w={:?} h={:?}",
            island_width, island_height
        ),
    );
    let _ = (island_width, island_height);
    // Yield past the resize/Moved reply path before scheduling Composition work.
    async_delay_ms(50).await;
    apply_main_window_material(&app);
    crate::win32::click_trace::log("rust", "reassert_main_bar_geometry leave");
    Ok(())
}

#[tauri::command]
pub fn get_material_prefs(state: State<'_, MaterialState>) -> crate::win32::material::MaterialPrefs {
    read_material_prefs(&state).normalize()
}

/// Concrete Windows Apps dark/light (used when material prefs `dark` is null / 跟随系统).
#[tauri::command]
pub fn system_apps_dark() -> bool {
    crate::win32::material::system_apps_dark()
}

#[tauri::command]
pub fn set_material_prefs(
    app: AppHandle,
    state: State<'_, MaterialState>,
    prefs: crate::win32::material::MaterialPrefs,
) -> Result<crate::win32::material::MaterialPrefs, String> {
    let prefs = prefs.normalize();
    let prev = read_material_prefs(&state);
    // Re-hit DWM when kind/dark changes, or when SWCA tint alpha changes (Blur/Aero).
    // Acrylic SYSTEMBACKDROP ignores tint — alpha is CSS-only there.
    let backdrop_changed = prev.kind != prefs.kind || prev.dark != prefs.dark;
    let swca_tint_changed = prev.acrylic_alpha != prefs.acrylic_alpha
        && matches!(
            prefs.kind,
            crate::win32::material::WindowMaterial::Blur
                | crate::win32::material::WindowMaterial::Aero
        );
    if let Ok(mut guard) = state.0.lock() {
        *guard = prefs.clone();
    }
    let _ = save_material_prefs(&prefs);
    if backdrop_changed || swca_tint_changed {
        reapply_material_to_popups(&app, &prefs);
    }
    let _ = app.emit("material-prefs", &prefs);
    Ok(prefs)
}

/// Apply material to the calling window (settings / tray / plugin / dock — shared prefs).
#[tauri::command]
pub fn apply_window_effect(
    window: WebviewWindow,
    state: State<'_, MaterialState>,
    material: Option<String>,
) -> Result<String, String> {
    // Tip HWND: never run the normal path's side effects mid-show; commit owns geometry.
    if window.label() == "chrome-hover-tip" {
        apply_chrome_hover_tip_material(&window, &state);
        return Ok(read_material_prefs(&state).kind.as_str().to_string());
    }
    let base = read_material_prefs(&state);
    let prefs = if let Some(m) = material {
        let mut p = base;
        p.kind = crate::win32::material::WindowMaterial::parse(&m);
        p
    } else {
        base
    };
    // Main / chrome-sat / island-bar-glass: shared bar material path.
    if window.label() == "main" || window.label() == "island-bar-glass" {
        apply_main_window_material(window.app_handle());
        return Ok(if get_island_prefs().bar_glass {
            prefs.kind.as_str().to_string()
        } else {
            "none".into()
        });
    }
    // chrome-sat: same material path as main (bar_comp via sync_inner), plus an
    // immediate per-HWND sync so desktop frost does not wait on debounce.
    if window.label().starts_with("chrome-sat-") {
        #[cfg(windows)]
        {
            crate::win32::island_bar_glass::sync_sat_window_now(&window, &prefs);
        }
        apply_main_window_material(window.app_handle());
        return Ok(if get_island_prefs().bar_glass {
            prefs.kind.as_str().to_string()
        } else {
            "none".into()
        });
    }
    // Settings / framed windows: soft reassert only. Full apply_prefs clears SWCA
    // then re-Mica — stacking with apply_prefs_deferred hung the pump after open
    // (and any later invoke from a settings button could re-enter the same path).
    if matches!(
        window.label(),
        "settings" | "dock-icon-editor" | "plugin-window" | "tray-popup" | "status-menu-popup" | "dock-add-icon-popup" | "control-center-popup" | "wifi-popup" | "wifi-auth-popup" | "input-lang-popup"
    ) {
        let _ = crate::win32::material::reassert_prefs(&window, &prefs);
        return Ok(prefs.kind.as_str().to_string());
    }
    crate::win32::material::apply_prefs(&window, &prefs)?;
    Ok(prefs.kind.as_str().to_string())
}

#[tauri::command]
pub fn get_window_material(state: State<'_, MaterialState>) -> String {
    read_material_prefs(&state).kind.as_str().to_string()
}

#[tauri::command]
pub fn set_window_material(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, MaterialState>,
    material: String,
) -> Result<String, String> {
    // Island / main: clear this HWND only — never rewrite shared popup prefs.
    if matches!(
        material.trim().to_ascii_lowercase().as_str(),
        "none" | "clear"
    ) {
        crate::win32::material::clear(&window)?;
        return Ok("none".into());
    }

    let mut prefs = read_material_prefs(&state);
    prefs.kind = crate::win32::material::WindowMaterial::parse(&material);
    prefs = prefs.normalize();
    if let Ok(mut guard) = state.0.lock() {
        *guard = prefs.clone();
    }
    let _ = save_material_prefs(&prefs);
    reapply_material_to_popups(&app, &prefs);
    Ok(prefs.kind.as_str().to_string())
}

fn main_hwnd(app: &AppHandle) -> Option<isize> {
    app.get_webview_window("main")
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as isize)
}

/// Fast path for UI: never block on a live BitBlt of a foreign HWND.
/// Prefer locked/last strip; otherwise wallpaper / fallback.
#[tauri::command]
pub fn sample_ambient_color(app: AppHandle) -> crate::win32::ambient::AmbientStrip {
    let hwnd = main_hwnd(&app);
    crate::win32::ambient::sample_nonblocking(hwnd)
}

/// Calling window sample — chrome-sat uses per-monitor cache (watcher fills it).
/// FE poll must stay BitBlt-free so dual-monitor auto-switch cannot hang IPC.
#[tauri::command]
pub fn sample_ambient_for_window(
    window: WebviewWindow,
) -> crate::win32::ambient::AmbientStrip {
    let label = window.label().to_string();
    let hwnd = window.hwnd().ok().map(|h| h.0 as isize);
    if label.starts_with("chrome-sat-") {
        return crate::win32::ambient::sample_sat_for_ipc(&label, hwnd);
    }
    crate::win32::ambient::sample_nonblocking(hwnd).with_label("main")
}

#[tauri::command]
pub fn get_ambient_mode() -> String {
    crate::win32::ambient::get_mode().as_str().to_string()
}

/// 切换采样模式并立刻重采一次（会推送新色带）。
#[tauri::command]
pub fn set_ambient_mode(
    app: AppHandle,
    mode: String,
) -> Result<crate::win32::ambient::AmbientStrip, String> {
    let kind = crate::win32::ambient::SampleMode::parse(&mode);
    crate::win32::ambient::set_mode(kind);
    let _ = save_ambient_mode(kind);
    let hwnd = main_hwnd(&app);
    let strip = crate::win32::ambient::poll_changed(hwnd)
        .unwrap_or_else(|| crate::win32::ambient::sample(hwnd));
    let _ = app.emit("ambient-color", &strip);
    Ok(strip)
}

fn save_ambient_mode(mode: crate::win32::ambient::SampleMode) -> Result<(), String> {
    crate::db::with_conn(|c| crate::db::ambient_set(c, mode.as_str()))
}

pub fn load_ambient_mode() -> crate::win32::ambient::SampleMode {
    if let Ok(Some(s)) = crate::db::with_conn(|c| crate::db::ambient_get(c)) {
        return crate::win32::ambient::SampleMode::parse(&s);
    }
    crate::win32::ambient::SampleMode::default()
}

#[derive(Serialize)]
pub struct Health {
    pub ok: bool,
    pub platform: &'static str,
}

#[tauri::command]
pub fn health() -> Health {
    Health {
        ok: true,
        platform: if cfg!(windows) { "windows" } else { "other" },
    }
}

/// FE / tests: append a line to the click-hang trace (file-only, no HWND).
#[tauri::command]
pub fn debug_click_trace(origin: String, msg: String) -> String {
    crate::win32::click_trace::log(&origin, &msg);
    crate::win32::click_trace::path_string()
}

#[tauri::command]
pub fn debug_click_trace_path() -> String {
    crate::win32::click_trace::path_string()
}

#[tauri::command]
pub fn debug_click_trace_http() -> String {
    crate::win32::click_trace::http_endpoint()
}

#[tauri::command]
pub fn debug_click_trace_clear() {
    crate::win32::click_trace::clear();
    crate::win32::click_trace::log("rust", "trace cleared");
}

/// Legacy shim removed — plugins use hub.storage.

pub fn load_tray_prefs() -> crate::win32::tray::TrayPrefs {
    let mut prefs: crate::win32::tray::TrayPrefs =
        if let Ok(Some(v)) = crate::db::with_conn(|c| crate::db::tray_get(c)) {
            serde_json::from_value(v).unwrap_or_default()
        } else {
            crate::win32::tray::TrayPrefs::default()
        };
    prefs.menu_height_px = None;
    for h in prefs.menu_heights.values_mut() {
        *h = (*h).clamp(48, 640);
    }
    prefs.menu_heights.retain(|_, h| *h > 0);
    prefs
}

fn save_tray_prefs(prefs: &crate::win32::tray::TrayPrefs) -> Result<(), String> {
    let mut to_save = prefs.clone();
    to_save.menu_height_px = None;
    let v = serde_json::to_value(&to_save).map_err(|e| e.to_string())?;
    crate::db::with_conn(|c| crate::db::tray_set(c, &v))
}

#[tauri::command(async)]
pub fn list_tray_icons() -> Vec<crate::win32::tray::TrayIconInfo> {
    crate::win32::tray::list_icons()
}

/// Force a light tray catch-up (registry soft-seed + TaskbarCreated when needed).
/// Safe to call after launching an app whose tray icon did not appear yet.
#[tauri::command]
pub fn refresh_tray_icons() {
    crate::win32::tray::request_refresh();
}

/// On-demand PNG glyphs (rail / popup). Never push these on every NIM_* emit.
#[tauri::command(async)]
pub fn get_tray_icon_glyphs(
    ids: Vec<String>,
) -> std::collections::HashMap<String, String> {
    crate::win32::tray::glyphs_for_ids(&ids)
}

/// Pause tray-icons FE emits while the island is morphing / resizing.
#[tauri::command]
pub fn set_tray_ui_paused(paused: bool) {
    crate::win32::tray::set_emit_paused(paused);
}

/// True after boot pipeline reveals chrome (`host-boot-ready`).
#[tauri::command]
pub fn is_host_boot_ready() -> bool {
    crate::host_boot_ready()
}

#[tauri::command]
pub fn get_tray_prefs() -> crate::win32::tray::TrayPrefs {
    crate::win32::tray::get_prefs()
}

#[tauri::command]
pub fn set_tray_prefs(
    app: AppHandle,
    pinned: Vec<String>,
    menu_heights: Option<std::collections::HashMap<String, i32>>,
    flash_notify: Option<std::collections::HashMap<String, bool>>,
) -> Result<crate::win32::tray::TrayPrefs, String> {
    let mut heights = menu_heights.unwrap_or_default();
    heights.retain(|_, h| *h > 0);
    for h in heights.values_mut() {
        *h = (*h).clamp(48, 640);
    }
    // Keep IME / language icons pinned even if the UI omitted them.
    let mut pinned = pinned;
    for icon in crate::win32::tray::list_icons() {
        if !icon.resident {
            continue;
        }
        let pk = if !icon.pin_key.is_empty() {
            icon.pin_key.clone()
        } else {
            icon.id.clone()
        };
        if !pinned.iter().any(|p| p == &pk || p == &icon.id) {
            pinned.push(pk);
        }
    }
    // Missing key = notify on (default). Omit arg → keep existing map.
    let flash = match flash_notify {
        Some(m) => m,
        None => crate::win32::tray::get_prefs().flash_notify,
    };
    let prefs = crate::win32::tray::TrayPrefs {
        pinned,
        menu_heights: heights,
        flash_notify: flash,
        menu_height_px: None,
    };
    crate::win32::tray::set_prefs(prefs.clone());
    // Re-read after normalize (pin_key rewrite).
    let prefs = crate::win32::tray::get_prefs();
    save_tray_prefs(&prefs)?;
    let _ = app.emit("tray-prefs", &prefs);
    Ok(prefs)
}

#[tauri::command]
pub fn get_input_lang() -> crate::win32::input_lang::InputLangState {
    crate::win32::input_lang::get()
}

#[tauri::command]
pub fn cycle_input_lang() -> Result<crate::win32::input_lang::InputLangState, String> {
    crate::win32::input_lang::cycle_layout()
}

#[tauri::command]
pub fn toggle_input_ime() -> Result<crate::win32::input_lang::InputLangState, String> {
    crate::win32::input_lang::toggle_ime()
}

#[tauri::command]
pub fn open_input_lang_settings() -> Result<(), String> {
    crate::win32::input_lang::open_language_settings()
}

#[tauri::command]
pub fn list_input_layouts() -> Vec<crate::win32::input_lang::InputLayoutItem> {
    crate::win32::input_lang::list_layouts()
}

#[tauri::command]
pub fn select_input_layout(
    profile_type: Option<u32>,
    lang_id: Option<u16>,
    clsid: Option<String>,
    guid_profile: Option<String>,
    hkl: Option<u64>,
) -> Result<crate::win32::input_lang::InputLangState, String> {
    crate::win32::input_lang::select_layout(
        profile_type.unwrap_or(0),
        lang_id.unwrap_or(0),
        clsid,
        guid_profile,
        hkl.unwrap_or(0),
    )
}

#[tauri::command]
pub fn open_input_emoji_panel() -> Result<(), String> {
    crate::win32::input_lang::open_emoji_panel()
}

#[tauri::command]
pub fn open_touch_keyboard() -> Result<(), String> {
    crate::win32::input_lang::open_touch_keyboard()
}

#[tauri::command]
pub fn open_keyboard_settings() -> Result<(), String> {
    crate::win32::input_lang::open_keyboard_settings()
}

const INPUT_LANG_POPUP_W: f64 = 240.0;
/// Placeholder only — frontend `fitPopupToContent` resizes to content.
const INPUT_LANG_POPUP_H: f64 = 80.0;

/// Self-drawn IME / language picker (figure-2 style).
#[tauri::command]
pub async fn open_input_lang_popup(
    app: AppHandle,
    state: State<'_, MaterialState>,
    x: f64,
    y: f64,
) -> Result<(), String> {
    close_sibling_popups(&app, "input-lang-popup");
    let (x, y) = fit_popup_xy(&app, x, y, INPUT_LANG_POPUP_W, INPUT_LANG_POPUP_H);

    if let Some(existing) = app.get_webview_window("input-lang-popup") {
        apply_saved_material(&existing, &state);
        let _ = existing.set_size(LogicalSize::new(INPUT_LANG_POPUP_W, INPUT_LANG_POPUP_H));
        let _ = existing.set_position(LogicalPosition::new(x, y));
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_focus();
        mark_popup_visible("input-lang-popup", true);
        let _ = app.emit("input-lang-popup-opened", ());
        return Ok(());
    }

    crate::win32::click_trace::log("rust", "open_input_lang_popup NEED_CREATE async-delay");
    async_delay_ms(100).await;
    if app.get_webview_window("input-lang-popup").is_some() {
        return Ok(());
    }
    crate::win32::click_trace::log("rust", "open_input_lang_popup after delay, build");
    let init = r#"
      window.__WH_IS_INPUT_LANG_POPUP__ = true;
      document.addEventListener('keydown', function (e) {
        if (e.key === 'Escape') {
          try { window.__TAURI__.core.invoke('close_input_lang_popup'); } catch (_) {}
        }
      });
    "#;
    let _guard = lock_webview_create("input-lang-popup");
    if app.get_webview_window("input-lang-popup").is_some() {
        return Ok(());
    }
    let wd = create_watchdog("input-lang-popup");
    let built = WebviewWindowBuilder::new(
        &app,
        "input-lang-popup",
        WebviewUrl::App("index.html?window=input-lang".into()),
    )
    .title("输入法")
    .inner_size(INPUT_LANG_POPUP_W, INPUT_LANG_POPUP_H)
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
    let win = built.map_err(|e| format!("input-lang create failed: {e}"))?;
    let _ = win.set_position(LogicalPosition::new(x, y));
    apply_saved_material(&win, &state);
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
    }
    let _ = win.show();
    let _ = win.set_focus();
    mark_popup_visible("input-lang-popup", true);
    let _ = app.emit("input-lang-popup-opened", ());
    crate::win32::click_trace::log("rust", "open_input_lang_popup build DONE");
    Ok(())
}

#[tauri::command]
pub async fn close_input_lang_popup(app: AppHandle) -> Result<(), String> {
    hide_chrome_popup(&app, "input-lang-popup");
    Ok(())
}

#[tauri::command]
pub fn is_input_lang_popup_open(_app: AppHandle) -> bool {
    popup_visible("input-lang-popup")
}

static CHROME_HOVER_TIP: Mutex<Option<ChromeHoverTipPayload>> = Mutex::new(None);
/// Backend-owned generation. Any show/close bumps this so in-flight work can detect supersession.
/// Do NOT trust per-webview JS counters — main/dock/tray each have their own tipEpoch and
/// desync permanently rejects shows (tip works once, then never again).
static CHROME_HOVER_TIP_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Last committed tip window box (logical): left, top, width, height — for position-only nudges.
static CHROME_HOVER_TIP_BOX: Mutex<Option<(f64, f64, f64, f64)>> = Mutex::new(None);

/// Chrome hover tip payload (status-bar tip must be a separate window — main is ~28px tall).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromeHoverTipPreview {
    pub jpeg_base64: String,
    pub title: String,
    pub hwnd: i64,
}

/// Chrome hover tip payload (status-bar tip must be a separate window — main is ~28px tall).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromeHoverTipPayload {
    pub lines: Vec<String>,
    pub x: f64,
    pub y: f64,
    /// `above` | `below` (default). Dock tips sit above the icon.
    #[serde(default)]
    pub placement: Option<String>,
    /// Optional live window thumbnail (JPEG base64, no data: prefix).
    /// Prefer `previews` when multiple windows; kept as first-frame compat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_jpeg_base64: Option<String>,
    /// When set with a preview image, tip is interactive (close button).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hwnd: Option<i64>,
    /// Dock item id — click preview launches/focuses like clicking the icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    /// Dock / app icon (PNG base64, no data: prefix) — title row leading glyph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_png_base64: Option<String>,
    /// Multi-instance window thumbnails (side-by-side). Empty/omitted = single `imageJpegBase64`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previews: Vec<ChromeHoverTipPreview>,
    /// Thumbnail CSS height (logical px). Default 160 when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_height_px: Option<u32>,
    /// Backend generation — FE must pass this to `commit_chrome_hover_tip`.
    #[serde(default)]
    pub epoch: u64,
}

/// Park the tip HWND off-screen while measuring so DWM never paints a stub frame.
const CHROME_HOVER_TIP_PARK_X: f64 = -32000.0;
const CHROME_HOVER_TIP_PARK_Y: f64 = -32000.0;
const CHROME_HOVER_TIP_MEASURE_W: f64 = 720.0;
const CHROME_HOVER_TIP_MEASURE_H: f64 = 420.0;

#[cfg(windows)]
fn chrome_hover_tip_hwnd(win: &WebviewWindow) -> Option<windows::Win32::Foundation::HWND> {
    let hwnd_raw = win.hwnd().ok()?;
    Some(windows::Win32::Foundation::HWND(hwnd_raw.0 as *mut _))
}

/// Kill Win11 show/move/resize animations on the tip HWND (they read as a drag).
/// Does NOT touch corner preference — that is owned by `apply_chrome_hover_tip_chrome`.
fn disable_chrome_hover_tip_transitions(win: &WebviewWindow) {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::BOOL;
        use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
        const DWMWA_TRANSITIONS_FORCEDISABLED: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(3);
        let Some(hwnd) = chrome_hover_tip_hwnd(win) else {
            return;
        };
        let disable = BOOL(1);
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_TRANSITIONS_FORCEDISABLED,
                &disable as *const _ as *const _,
                std::mem::size_of::<BOOL>() as u32,
            );
        }
    }
    #[cfg(not(windows))]
    {
        let _ = win;
    }
}

/// Frameless popup chrome — identical path to `status-menu-popup`
/// (`strip_frameless_popup_titlebar` → `apply_mica_chrome` with DWM `ROUND`).
/// Do not force `ROUNDSMALL`: it clips less than CSS 10px and leaves a light HWND fringe.
fn apply_chrome_hover_tip_chrome(win: &WebviewWindow, dark: Option<bool>) {
    #[cfg(windows)]
    {
        use std::ffi::c_void;
        use windows::Win32::Graphics::Dwm::{
            DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE,
        };
        let Some(hwnd) = chrome_hover_tip_hwnd(win) else {
            return;
        };
        crate::win32::blur_glass::strip_frameless_popup_titlebar(hwnd.0 as isize);
        unsafe {
            if let Some(d) = dark {
                let v: u32 = u32::from(d);
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_USE_IMMERSIVE_DARK_MODE,
                    &v as *const u32 as *const c_void,
                    std::mem::size_of::<u32>() as u32,
                );
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = dark;
    }
    let _ = win.set_shadow(false);
}

fn park_chrome_hover_tip(win: &WebviewWindow) {
    if let Ok(mut g) = CHROME_HOVER_TIP_BOX.lock() {
        *g = None;
    }
    // Off-screen measure box — never use Tauri set_size (async → races with show).
    set_chrome_hover_tip_rect(
        win,
        CHROME_HOVER_TIP_PARK_X,
        CHROME_HOVER_TIP_PARK_Y,
        CHROME_HOVER_TIP_MEASURE_W,
        CHROME_HOVER_TIP_MEASURE_H,
        false,
        true,
    );
}

fn chrome_hover_tip_same(a: &ChromeHoverTipPayload, b: &ChromeHoverTipPayload) -> bool {
    chrome_hover_tip_same_content(a, b)
        && (a.x - b.x).abs() < 0.75
        && (a.y - b.y).abs() < 0.75
}

/// Same tip body (ignore anchor) — used to slide without park/remeasure flash.
fn chrome_hover_tip_same_content(a: &ChromeHoverTipPayload, b: &ChromeHoverTipPayload) -> bool {
    if a.lines != b.lines {
        return false;
    }
    if a.image_jpeg_base64 != b.image_jpeg_base64 {
        return false;
    }
    if a.icon_png_base64 != b.icon_png_base64 {
        return false;
    }
    if a.hwnd != b.hwnd || a.item_id != b.item_id {
        return false;
    }
    if a.preview_height_px != b.preview_height_px {
        return false;
    }
    if a.previews.len() != b.previews.len() {
        return false;
    }
    for (pa, pb) in a.previews.iter().zip(b.previews.iter()) {
        if pa.jpeg_base64 != pb.jpeg_base64 || pa.title != pb.title || pa.hwnd != pb.hwnd {
            return false;
        }
    }
    a.placement == b.placement
}

fn chrome_hover_tip_window_origin(payload: &ChromeHoverTipPayload, w: f64, h: f64) -> (f64, f64) {
    let left = (payload.x - w * 0.5).max(4.0);
    let above = payload
        .placement
        .as_deref()
        .is_some_and(|p| p.eq_ignore_ascii_case("above"));
    let top = if above {
        (payload.y - h).max(0.0)
    } else {
        payload.y.max(0.0)
    };
    (left, top)
}

/// Low-level tip HWND box. Never call Tauri `set_size`/`set_position` here —
/// those are async and resize *after* ShowWindow (reads as bottom-right drag).
fn set_chrome_hover_tip_rect(
    win: &WebviewWindow,
    left: f64,
    top: f64,
    w: f64,
    h: f64,
    show: bool,
    resize: bool,
) {
    disable_chrome_hover_tip_transitions(win);
    let scale = win.scale_factor().unwrap_or(1.0).max(0.1);
    let px = (left * scale).round() as i32;
    let py = (top * scale).round() as i32;
    let pw = (w.max(1.0) * scale).round().max(1.0) as i32;
    let ph = (h.max(1.0) * scale).round().max(1.0) as i32;
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, ShowWindow, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOCOPYBITS, SWP_NOSIZE,
            SWP_NOZORDER, SW_HIDE, SW_SHOWNA,
        };
        let Some(hwnd) = chrome_hover_tip_hwnd(win) else {
            return;
        };
        // Never SWP_SHOWWINDOW / SWP_FRAMECHANGED with a size change — that is the
        // DWM "drag from bottom-right" paint. Size while hidden; ShowWindow alone.
        let mut flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS;
        if !resize {
            flags |= SWP_NOSIZE;
        }
        if !show {
            flags |= SWP_HIDEWINDOW;
        }
        unsafe {
            if resize {
                let _ = SetWindowPos(hwnd, None, px, py, pw, ph, flags);
            } else {
                let _ = SetWindowPos(hwnd, None, px, py, 0, 0, flags | SWP_NOSIZE);
            }
            if show {
                let _ = ShowWindow(hwnd, SW_SHOWNA);
            } else {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
    }
    #[cfg(not(windows))]
    {
        if resize {
            let _ = win.set_size(LogicalSize::new(w.max(1.0), h.max(1.0)));
        }
        let _ = win.set_position(LogicalPosition::new(left, top));
        if show {
            let _ = win.show();
        } else {
            let _ = win.hide();
        }
        let _ = (px, py, pw, ph);
    }
}

/// Final box while HIDDEN (Tauri + Win32 agree on size), then ShowWindow only.
///
/// Skipping Tauri `set_size` left Wry's cached size at the measure box (720×420);
/// ShowWindow then restored that — tip looked like a huge empty mica slab.
fn reveal_chrome_hover_tip(win: &WebviewWindow, left: f64, top: f64, w: f64, h: f64) {
    let w = w.max(1.0);
    let h = h.max(1.0);
    disable_chrome_hover_tip_transitions(win);
    // 1) Hide + size/position while invisible. Prefer Tauri first so WebView2
    //    client size matches; then re-assert with SetWindowPos (no SHOW flag).
    let _ = win.hide();
    let _ = win.set_size(LogicalSize::new(w, h));
    let _ = win.set_position(LogicalPosition::new(left, top));
    set_chrome_hover_tip_rect(win, left, top, w, h, false, true);
    // 2) Show only — never resize in the same call as becoming visible.
    set_chrome_hover_tip_rect(win, left, top, w, h, true, false);
}

fn move_chrome_hover_tip_hwnd(win: &WebviewWindow, left: f64, top: f64, w: f64, h: f64, show: bool) {
    // Visible nudge: move only (no resize).
    set_chrome_hover_tip_rect(win, left, top, w, h, show, false);
}

/// Tip material: sync once, never deferred (deferred SetWindowPos races with show).
fn apply_chrome_hover_tip_material(win: &WebviewWindow, state: &MaterialState) {
    disable_chrome_hover_tip_transitions(win);
    let prefs = read_material_prefs(state);
    let dark = crate::win32::material::resolve_dark(prefs.dark);
    let _ = crate::win32::material::apply_prefs(win, &prefs);
    apply_chrome_hover_tip_chrome(win, Some(dark));
}

/// Windows that may own chrome tips — cursor must stay over one of these or tip auto-hides.
const CHROME_TIP_HOST_LABELS: &[&str] = &[
    "main",
    "dock",
    "tray-popup",
    "status-menu-popup",
    "chrome-hover-tip",
];

#[cfg(windows)]
fn chrome_tip_cursor_pos() -> Option<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut pt = POINT::default();
    unsafe {
        if GetCursorPos(&mut pt).is_err() {
            return None;
        }
    }
    Some((pt.x, pt.y))
}

#[cfg(not(windows))]
fn chrome_tip_cursor_pos() -> Option<(i32, i32)> {
    None
}

/// Cursor position in the `main` webview's CSS client space (logical px).
/// Used so stacked island notify can click-through beside the capsule.
#[tauri::command]
pub fn main_cursor_client_pos(app: AppHandle) -> Option<(f64, f64)> {
    let (cx, cy) = chrome_tip_cursor_pos()?;
    let win = app.get_webview_window("main")?;
    let pos = win.outer_position().ok()?;
    let scale = win.scale_factor().ok()?.max(0.1);
    Some((
        (cx as f64 - pos.x as f64) / scale,
        (cy as f64 - pos.y as f64) / scale,
    ))
}

fn chrome_tip_cursor_over_host(app: &AppHandle) -> bool {
    let Some((cx, cy)) = chrome_tip_cursor_pos() else {
        // Unknown cursor — keep tip (avoid flicker on transient API failure).
        return true;
    };
    // Thin bars: small pad so edge slips don't false-dismiss.
    const PAD: i32 = 6;
    for label in CHROME_TIP_HOST_LABELS {
        let Some(w) = app.get_webview_window(label) else {
            continue;
        };
        let Ok(visible) = w.is_visible() else {
            continue;
        };
        if !visible {
            continue;
        }
        let Ok(pos) = w.outer_position() else {
            continue;
        };
        let Ok(size) = w.outer_size() else {
            continue;
        };
        let left = pos.x.saturating_sub(PAD);
        let top = pos.y.saturating_sub(PAD);
        let right = pos.x.saturating_add(size.width as i32).saturating_add(PAD);
        let bottom = pos.y.saturating_add(size.height as i32).saturating_add(PAD);
        if cx >= left && cx < right && cy >= top && cy < bottom {
            return true;
        }
    }
    false
}

fn hide_chrome_hover_tip_sync(app: &AppHandle) {
    CHROME_HOVER_TIP_EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if let Ok(mut g) = CHROME_HOVER_TIP.lock() {
        *g = None;
    }
    if let Ok(mut g) = CHROME_HOVER_TIP_BOX.lock() {
        *g = None;
    }
    set_dock_preview_tip_keep(app, false);
    if let Some(w) = app.get_webview_window("chrome-hover-tip") {
        park_chrome_hover_tip(&w);
    }
    let _ = app.emit("chrome-hover-tip-hide", ());
}

/// When tip is visible but the pointer already left Host (no more DOM events), poll cursor and hide.
fn spawn_chrome_tip_leave_watch(app: AppHandle, seq: u64) {
    std::thread::spawn(move || {
        let mut misses = 0u32;
        // ~12s max; normal hover dismisses much sooner via FE or leave watch.
        for _ in 0..120 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if CHROME_HOVER_TIP_EPOCH.load(std::sync::atomic::Ordering::SeqCst) != seq {
                return;
            }
            if chrome_tip_cursor_over_host(&app) {
                misses = 0;
                continue;
            }
            misses = misses.saturating_add(1);
            // Interactive preview: allow crossing the Dock→tip gap without a false dismiss.
            let interactive = CHROME_HOVER_TIP
                .lock()
                .ok()
                .and_then(|g| g.clone())
                .map(|p| {
                    !p.previews.is_empty()
                        || (p.image_jpeg_base64.as_ref().is_some_and(|s| !s.is_empty())
                            && p.hwnd.map(|h| h != 0).unwrap_or(false))
                })
                .unwrap_or(false);
            let need = if interactive { 5 } else { 2 }; // ~500ms vs ~200ms
            if misses >= need {
                if CHROME_HOVER_TIP_EPOCH.load(std::sync::atomic::Ordering::SeqCst) != seq {
                    return;
                }
                hide_chrome_hover_tip_sync(&app);
                return;
            }
        }
        if CHROME_HOVER_TIP_EPOCH.load(std::sync::atomic::Ordering::SeqCst) == seq {
            hide_chrome_hover_tip_sync(&app);
        }
    });
}

#[tauri::command]
pub fn get_chrome_hover_tip() -> Option<ChromeHoverTipPayload> {
    CHROME_HOVER_TIP.lock().ok().and_then(|g| g.clone())
}

#[tauri::command]
pub async fn show_chrome_hover_tip(
    app: AppHandle,
    state: State<'_, MaterialState>,
    lines: Vec<String>,
    x: f64,
    y: f64,
    placement: Option<String>,
    image_jpeg_base64: Option<String>,
    hwnd: Option<i64>,
    item_id: Option<String>,
    icon_png_base64: Option<String>,
    previews: Option<Vec<ChromeHoverTipPreview>>,
    preview_height_px: Option<u32>,
    #[allow(unused_variables)] epoch: Option<u64>,
) -> Result<(), String> {
    let lines: Vec<String> = lines
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .take(8)
        .collect();
    let mut previews: Vec<ChromeHoverTipPreview> = previews
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| {
            let jpeg = p.jpeg_base64.trim().to_string();
            if jpeg.is_empty() || p.hwnd == 0 {
                return None;
            }
            Some(ChromeHoverTipPreview {
                jpeg_base64: jpeg,
                title: p.title.trim().to_string(),
                hwnd: p.hwnd,
            })
        })
        .take(8)
        .collect();
    let image = image_jpeg_base64
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    // Promote legacy single image into previews when needed.
    if previews.is_empty() {
        if let (Some(jpeg), Some(h)) = (image.clone(), hwnd.filter(|h| *h != 0)) {
            previews.push(ChromeHoverTipPreview {
                jpeg_base64: jpeg,
                title: lines.first().cloned().unwrap_or_default(),
                hwnd: h,
            });
        }
    }
    let image = previews
        .first()
        .map(|p| p.jpeg_base64.clone())
        .or(image);
    if lines.is_empty() && image.is_none() && previews.is_empty() {
        return close_chrome_hover_tip(app, None).await;
    }

    let placement = placement
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| s == "above" || s == "below");
    let primary_hwnd = previews.first().map(|p| p.hwnd).or(hwnd.filter(|h| *h != 0));
    let interactive = image.is_some() && primary_hwnd.map(|h| h != 0).unwrap_or(false);
    let item_id = item_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let icon = icon_png_base64
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let preview_height_px = preview_height_px
        .map(|h| h.clamp(96, 320))
        .filter(|&h| h > 0);
    let mut payload = ChromeHoverTipPayload {
        lines,
        x,
        y,
        placement,
        image_jpeg_base64: image,
        hwnd: if interactive { primary_hwnd } else { None },
        // Keep item id for title-first progressive tips (click before thumb arrives).
        item_id,
        icon_png_base64: icon,
        previews,
        preview_height_px,
        epoch: 0,
    };

    // Skip identical re-show (same content + ~same anchor) — prevents tip/preview flicker.
    // Still re-assert cursor hit-testing so an interactive tip never stays click-through.
    // Same body, new anchor (fan arm / chrome widen): slide HWND only — no park flash.
    if let Some(existing) = app.get_webview_window("chrome-hover-tip") {
        let prev = CHROME_HOVER_TIP.lock().ok().and_then(|g| g.clone());
        if let Some(prev) = prev {
            if chrome_hover_tip_same(&prev, &payload) {
                if interactive || payload.item_id.is_some() {
                    set_dock_preview_tip_keep(&app, true);
                }
                let _ = existing.set_ignore_cursor_events(!interactive);
                return Ok(());
            }
            if chrome_hover_tip_same_content(&prev, &payload) {
                let box_wh = CHROME_HOVER_TIP_BOX.lock().ok().and_then(|b| *b);
                if let Some((_, _, bw, bh)) = box_wh {
                    let visible = existing.is_visible().unwrap_or(false);
                    if visible {
                        if interactive || payload.item_id.is_some() {
                            set_dock_preview_tip_keep(&app, true);
                        }
                        let (left, top) = chrome_hover_tip_window_origin(&payload, bw, bh);
                        move_chrome_hover_tip_hwnd(&existing, left, top, bw, bh, true);
                        if let Ok(mut box_g) = CHROME_HOVER_TIP_BOX.lock() {
                            *box_g = Some((left, top, bw, bh));
                        }
                        if let Ok(mut tip_g) = CHROME_HOVER_TIP.lock() {
                            let mut next = payload.clone();
                            next.epoch = tip_g.as_ref().map(|p| p.epoch).unwrap_or(0);
                            *tip_g = Some(next);
                        }
                        let _ = existing.set_ignore_cursor_events(!interactive);
                        return Ok(());
                    }
                }
            }
        }
    }

    // Hide + park BEFORE emit so React never paints a new layout on a visible HWND.
    if let Some(existing) = app.get_webview_window("chrome-hover-tip") {
        park_chrome_hover_tip(&existing);
        apply_chrome_hover_tip_material(&existing, &state);
        let _ = existing.set_always_on_top(true);
        let _ = existing.set_ignore_cursor_events(!interactive);
    }

    // Claim a generation for this show; close/newer show will bump past it.
    let seq = CHROME_HOVER_TIP_EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    payload.epoch = seq;
    if let Ok(mut g) = CHROME_HOVER_TIP.lock() {
        *g = Some(payload.clone());
    }
    // Hold AutoHide for interactive previews / dock preview sessions (title-first included).
    if interactive || payload.item_id.is_some() {
        set_dock_preview_tip_keep(&app, true);
    }

    let still_current =
        || CHROME_HOVER_TIP_EPOCH.load(std::sync::atomic::Ordering::SeqCst) == seq;

    if app.get_webview_window("chrome-hover-tip").is_some() {
        if !still_current() {
            return Ok(());
        }
        let _ = app.emit("chrome-hover-tip-show", &payload);
        spawn_chrome_tip_leave_watch(app.clone(), seq);
        return Ok(());
    }

    // Proven hang (click-trace #10+#13): never build tip HWND on the hover/click
    // path — races with status-menu/tray create and freezes WebView2. Skip until
    // boot prewarm (or a quiet background request) has created it.
    crate::win32::click_trace::log(
        "rust",
        "show_chrome_hover_tip SKIP_NO_HWND (no build on hot path)",
    );
    request_tip_prewarm(&app);
    Ok(())
}

fn request_tip_prewarm(_app: &AppHandle) {
    // DISABLED: same hang class as chrome-prewarm (background WebView::build).
    crate::win32::click_trace::log("rust", "tip-prewarm DISABLED (no background create)");
}

/// Apply measured geometry then show once. No-op if a newer show/hide superseded `epoch`.
///
/// Uses one `SetWindowPos` for size+origin while hidden — separate Tauri
/// `set_size` / `set_position` lets DWM paint a wide stub then slide (reads as
/// drag-from-the-right).
#[tauri::command]
pub async fn commit_chrome_hover_tip(
    app: AppHandle,
    state: State<'_, MaterialState>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    epoch: u64,
) -> Result<(), String> {
    if epoch == 0
        || CHROME_HOVER_TIP_EPOCH.load(std::sync::atomic::Ordering::SeqCst) != epoch
    {
        return Ok(());
    }
    let Some(win) = app.get_webview_window("chrome-hover-tip") else {
        return Ok(());
    };
    let w = width.max(1.0).min(2400.0);
    let h = height.max(1.0).min(560.0);
    let left = x.max(0.0);
    let top = y.max(0.0);
    let interactive = CHROME_HOVER_TIP
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .map(|p| {
            !p.previews.is_empty()
                || (p.image_jpeg_base64.as_ref().is_some_and(|s| !s.is_empty())
                    && p.hwnd.map(|h| h != 0).unwrap_or(false))
        })
        .unwrap_or(false);
    let _ = win.set_ignore_cursor_events(!interactive);
    let _ = win.set_always_on_top(true);
    // Material while hidden, then one hidden SetWindowPos(final) + ShowWindow.
    // Never Tauri set_size / deferred material — those resize after show (= 右下拖拽).
    apply_chrome_hover_tip_material(&win, &state);
    reveal_chrome_hover_tip(&win, left, top, w, h);
    if CHROME_HOVER_TIP_EPOCH.load(std::sync::atomic::Ordering::SeqCst) != epoch {
        park_chrome_hover_tip(&win);
        return Ok(());
    }
    if let Ok(mut g) = CHROME_HOVER_TIP_BOX.lock() {
        *g = Some((left, top, w, h));
    }
    Ok(())
}

#[tauri::command]
pub async fn close_chrome_hover_tip(
    app: AppHandle,
    #[allow(unused_variables)] epoch: Option<u64>,
) -> Result<(), String> {
    hide_chrome_hover_tip_sync(&app);
    Ok(())
}

#[tauri::command]
pub fn get_wifi_state() -> crate::win32::wifi::WifiState {
    crate::win32::wifi::get()
}

#[tauri::command]
pub fn list_wifi_networks() -> Result<Vec<crate::win32::wifi::WifiNetwork>, String> {
    crate::win32::wifi::list_networks()
}

#[tauri::command]
pub fn set_wifi_enabled(enabled: bool) -> Result<crate::win32::wifi::WifiState, String> {
    crate::win32::wifi::set_enabled(enabled)
}

#[tauri::command]
pub fn connect_wifi(
    ssid: String,
    password: Option<String>,
) -> Result<crate::win32::wifi::WifiState, String> {
    crate::win32::wifi::connect(&ssid, password.as_deref())
}

#[tauri::command]
pub fn disconnect_wifi() -> Result<crate::win32::wifi::WifiState, String> {
    crate::win32::wifi::disconnect()
}

#[tauri::command]
pub fn open_network_settings() -> Result<(), String> {
    crate::win32::wifi::open_network_settings()
}

const WIFI_POPUP_W: f64 = 280.0;
/// Placeholder only — frontend `fitPopupToContent` resizes to content.
/// Keep tall enough that clipped layouts still show 首选/其他网络 before fit.
const WIFI_POPUP_H: f64 = 320.0;

const WIFI_AUTH_W: f64 = 420.0;
const WIFI_AUTH_H: f64 = 220.0;

pub(crate) fn close_sibling_popups(app: &AppHandle, except: &str) {
    for label in [
        "tray-popup",
        "plugin-popup",
        "status-menu-popup",
        "dock-add-icon-popup",
        "input-lang-popup",
        "control-center-popup",
        "wifi-popup",
        "wifi-auth-popup",
    ] {
        if label == except {
            continue;
        }
        if app.get_webview_window(label).is_none() {
            continue;
        }
        // Reusable chrome: hide only. Plugin surfaces still tear down (content binds
        // to a single plugin id and must not show a stale surface).
        if REUSABLE_CHROME_POPUPS.contains(&label) {
            hide_chrome_popup(app, label);
        } else if label == "plugin-popup" {
            close_plugin_surfaces(app);
        }
    }
}

/// Self-drawn WLAN menu (resident tray chip).
#[tauri::command]
pub async fn open_wifi_popup(
    app: AppHandle,
    state: State<'_, MaterialState>,
    x: f64,
    y: f64,
) -> Result<(), String> {
    close_sibling_popups(&app, "wifi-popup");
    let _ = crate::win32::wifi::refresh();
    let (x, y) = fit_popup_xy(&app, x, y, WIFI_POPUP_W, WIFI_POPUP_H);

    if let Some(existing) = app.get_webview_window("wifi-popup") {
        apply_saved_material(&existing, &state);
        let _ = existing.set_size(LogicalSize::new(WIFI_POPUP_W, WIFI_POPUP_H));
        let _ = existing.set_position(LogicalPosition::new(x, y));
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_focus();
        mark_popup_visible("wifi-popup", true);
        let _ = app.emit("wifi-popup-opened", ());
        return Ok(());
    }

    crate::win32::click_trace::log("rust", "open_wifi_popup NEED_CREATE async-delay");
    async_delay_ms(100).await;
    let _ = crate::win32::wifi::refresh();
    if app.get_webview_window("wifi-popup").is_some() {
        return Ok(());
    }
    crate::win32::click_trace::log("rust", "open_wifi_popup after delay, build");
    let init = r#"
      window.__WH_IS_WIFI_POPUP__ = true;
      document.addEventListener('keydown', function (e) {
        if (e.key === 'Escape') {
          try { window.__TAURI__.core.invoke('close_wifi_popup'); } catch (_) {}
        }
      });
    "#;
    let _guard = lock_webview_create("wifi-popup");
    if app.get_webview_window("wifi-popup").is_some() {
        return Ok(());
    }
    let wd = create_watchdog("wifi-popup");
    let built = WebviewWindowBuilder::new(
        &app,
        "wifi-popup",
        WebviewUrl::App("index.html?window=wifi".into()),
    )
    .title("WLAN")
    .inner_size(WIFI_POPUP_W, WIFI_POPUP_H)
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
    let win = built.map_err(|e| format!("wifi-popup create failed: {e}"))?;
    let _ = win.set_position(LogicalPosition::new(x, y));
    apply_saved_material(&win, &state);
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
    }
    let _ = win.show();
    let _ = win.set_focus();
    mark_popup_visible("wifi-popup", true);
    let _ = app.emit("wifi-popup-opened", ());
    crate::win32::click_trace::log("rust", "open_wifi_popup build DONE");
    Ok(())
}

#[tauri::command]
pub async fn close_wifi_popup(app: AppHandle) -> Result<(), String> {
    hide_chrome_popup(&app, "wifi-popup");
    Ok(())
}

#[tauri::command]
pub fn is_wifi_popup_open(_app: AppHandle) -> bool {
    popup_visible("wifi-popup")
}

/// Centered global password dialog for joining a secured WLAN.
#[tauri::command]
pub async fn open_wifi_auth_popup(
    app: AppHandle,
    state: State<'_, MaterialState>,
    ssid: String,
) -> Result<(), String> {
    let ssid = ssid.trim().to_string();
    if ssid.is_empty() {
        return Err("SSID 为空".into());
    }

    // Dismiss siblings but keep HWNDs — auth is the focused modal.
    hide_chrome_popup(&app, "wifi-popup");
    for label in ["tray-popup", "status-menu-popup", "input-lang-popup"] {
        if app.get_webview_window(label).is_some() {
            hide_chrome_popup(&app, label);
        }
    }
    if app.get_webview_window("plugin-popup").is_some()
        || app.get_webview_window("plugin-window").is_some()
    {
        close_plugin_surfaces(&app);
    }

    let ssid_js = serde_json::to_string(&ssid).unwrap_or_else(|_| "\"\"".into());
    let (pos_x, pos_y) = {
        let main = app.get_webview_window("main");
        let monitor = main
            .as_ref()
            .and_then(|w| w.current_monitor().ok().flatten())
            .or_else(|| app.primary_monitor().ok().flatten());
        if let Some(m) = monitor {
            let scale = m.scale_factor();
            let size = m.size();
            let pos = m.position();
            let w = size.width as f64 / scale;
            let h = size.height as f64 / scale;
            let x = pos.x as f64 / scale + (w - WIFI_AUTH_W) / 2.0;
            let y = pos.y as f64 / scale + (h - WIFI_AUTH_H) / 2.0;
            (x, y)
        } else {
            (200.0, 200.0)
        }
    };

    if let Some(existing) = app.get_webview_window("wifi-auth-popup") {
        apply_saved_material(&existing, &state);
        let _ = existing.set_size(LogicalSize::new(WIFI_AUTH_W, WIFI_AUTH_H));
        let _ = existing.set_position(LogicalPosition::new(pos_x, pos_y));
        let _ = existing.eval(&format!("window.__WH_WIFI_AUTH_SSID__ = {ssid_js};"));
        let _ = app.emit("wifi-auth-ssid", &ssid);
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_focus();
        mark_popup_visible("wifi-auth-popup", true);
        let _ = app.emit("wifi-auth-popup-opened", &ssid);
        return Ok(());
    }

    crate::win32::click_trace::log("rust", "open_wifi_auth_popup NEED_CREATE async-delay");
    async_delay_ms(100).await;
    if app.get_webview_window("wifi-auth-popup").is_some() {
        return Ok(());
    }
    crate::win32::click_trace::log("rust", "open_wifi_auth_popup after delay, build");
    let ssid_js2 = serde_json::to_string(&ssid).unwrap_or_else(|_| "\"\"".into());
    let init = format!(
        r#"
      window.__WH_IS_WIFI_AUTH_POPUP__ = true;
      window.__WH_WIFI_AUTH_SSID__ = {ssid_js2};
      document.addEventListener('keydown', function (e) {{
        if (e.key === 'Escape') {{
          try {{ window.__TAURI__.core.invoke('close_wifi_auth_popup'); }} catch (_) {{}}
        }}
      }});
    "#
    );
    let _guard = lock_webview_create("wifi-auth-popup");
    if app.get_webview_window("wifi-auth-popup").is_some() {
        return Ok(());
    }
    let wd = create_watchdog("wifi-auth-popup");
    let built = WebviewWindowBuilder::new(
        &app,
        "wifi-auth-popup",
        WebviewUrl::App(
            format!(
                "index.html?window=wifi-auth&ssid={}",
                urlencoding_encode(&ssid)
            )
            .into(),
        ),
    )
    .title("加入网络")
    .inner_size(WIFI_AUTH_W, WIFI_AUTH_H)
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
    .initialization_script(&init)
    .build();
    finish_watchdog(&wd);
    let win = built.map_err(|e| format!("wifi-auth create failed: {e}"))?;
    let _ = win.set_position(LogicalPosition::new(pos_x, pos_y));
    apply_saved_material(&win, &state);
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
    }
    let _ = win.show();
    let _ = win.set_focus();
    mark_popup_visible("wifi-auth-popup", true);
    let _ = app.emit("wifi-auth-popup-opened", &ssid);
    crate::win32::click_trace::log("rust", "open_wifi_auth_popup build DONE");
    Ok(())
}

fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[tauri::command]
pub async fn close_wifi_auth_popup(app: AppHandle) -> Result<(), String> {
    hide_chrome_popup(&app, "wifi-auth-popup");
    Ok(())
}

#[tauri::command]
pub fn is_wifi_auth_popup_open(_app: AppHandle) -> bool {
    popup_visible("wifi-auth-popup")
}

#[tauri::command]
pub async fn invoke_tray_icon(
    app: AppHandle,
    hwnd: isize,
    callback_msg: u32,
    uid: u32,
    version: Option<u32>,
    action: Option<String>,
    id: Option<String>,
    cursor_x: Option<i32>,
    cursor_y: Option<i32>,
) -> Result<(), String> {
    crate::win32::click_trace::log("tray", &format!("invoke action={action:?} hwnd={hwnd:#x} callback={callback_msg:#x} uid={uid} version={version:?} cursor={cursor_x:?},{cursor_y:?}"));
    let click = crate::win32::tray::TrayClick::parse(action.as_deref().unwrap_or("left"));
    let version = version.unwrap_or(0);
    let cursor = match (cursor_x, cursor_y) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => None,
    };
    // Await the worker, never execute Win32/UIA on the UI/runtime thread.
    // Return dispatch failures to the caller instead of acknowledging a no-op.
    if matches!(click, crate::win32::tray::TrayClick::Right) {
        hide_chrome_popup(&app, "control-center-popup");
    }
    tauri::async_runtime::spawn_blocking(move || {
        let result = crate::win32::tray::invoke_icon_by_id(
            id, hwnd, callback_msg, uid, version, click, cursor,
        );
        crate::win32::click_trace::log("tray", &format!("dispatch result={result:?}"));
        result
    })
    .await
    .map_err(|e| format!("托盘点击任务失败：{e}"))?
}

/// Physical screen cursor — capture at pointer-down so deferred left-clicks
/// still pack VERSION_4 coords from the press point (not 280ms later).
#[tauri::command]
pub fn tray_cursor_pos() -> (i32, i32) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    (pt.x, pt.y)
}

/// 点开岛通知 / 确认托盘注意力：flashing → 0，下次新消息可再弹。
#[tauri::command]
pub fn clear_tray_attention(
    id: Option<String>,
    hwnd: Option<isize>,
    uid: Option<u32>,
) -> Result<(), String> {
    crate::win32::tray::acknowledge_icon_attention(id, hwnd.unwrap_or(0), uid.unwrap_or(0));
    Ok(())
}

/// 打开系统通知中心（等同 Win+N）。
#[tauri::command]
pub fn open_notification_center() -> Result<(), String> {
    crate::win32::input::open_notification_center()
}



/// 当前前台窗口短标签（状态菜单左侧 chip）。
#[tauri::command]
pub fn get_foreground_app(window: WebviewWindow) -> crate::win32::status_menu::ForegroundApp {
    let self_hwnd = window.hwnd().ok().map(|h| h.0 as isize);
    crate::win32::status_menu::foreground_app(self_hwnd)
}

#[tauri::command]
pub fn set_system_taskbar_visible(visible: bool) -> Result<(), String> {
    crate::win32::status_menu::set_taskbar_visible(visible)
}

#[tauri::command]
pub fn show_desktop() -> Result<(), String> {
    crate::win32::status_menu::show_desktop()
}

#[tauri::command]
pub fn restart_app(app: AppHandle) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(exe);
    // Child waits for our single-instance lock to disappear before building UI.
    cmd.arg("--wait-for-restart");
    if let Ok(cwd) = std::env::current_dir() {
        cmd.current_dir(cwd);
    }
    cmd.spawn().map_err(|e| format!("restart spawn failed: {e}"))?;
    #[cfg(windows)]
    {
        crate::win32::autostart_svc::clear_user_quit();
    }
    crate::lifecycle::begin_shutdown(Some(app));
    Ok(())
}

#[tauri::command]
pub fn exit_app(app: AppHandle) {
    // Stop WindowHubAutoStart from immediately relaunching the GUI.
    #[cfg(windows)]
    crate::win32::autostart_svc::signal_user_quit();
    crate::lifecycle::begin_shutdown(Some(app));
}

#[tauri::command]
pub fn list_hotkey_bindings() -> Result<Vec<crate::win32::hotkey_registry::HotkeyBindingDto>, String> {
    crate::win32::hotkey_registry::list_bindings()
}

#[tauri::command]
pub fn set_hotkey_binding(
    app: AppHandle,
    id: String,
    chord: String,
) -> Result<Vec<crate::win32::hotkey_registry::HotkeyBindingDto>, String> {
    crate::win32::hotkey_registry::set_binding(&app, &id, &chord)
}

#[tauri::command]
pub fn validate_hotkey_chord(id: String, chord: String) -> Result<String, String> {
    crate::win32::hotkey_registry::validate_chord_available(&id, &chord)
}

#[tauri::command]
pub fn suspend_hotkeys_for_recording() {
    #[cfg(windows)]
    crate::win32::hotkey_registry::suspend_for_recording();
}

#[tauri::command]
pub fn resume_hotkeys_after_recording() {
    #[cfg(windows)]
    crate::win32::hotkey_registry::resume_after_recording();
}

// ── Staging (CapGate only; scoped by pluginId) ─────────────────────

#[tauri::command]
pub fn hub_staging_list(plugin_id: String) -> Result<Vec<crate::staging::StagingItem>, String> {
    crate::plugin_hub::assert_capability(&plugin_id, "staging")?;
    Ok(crate::staging::list(&plugin_id))
}

#[tauri::command]
pub fn hub_staging_summary(plugin_id: String) -> Result<crate::staging::StagingSummary, String> {
    crate::plugin_hub::assert_capability(&plugin_id, "staging")?;
    Ok(crate::staging::summary(&plugin_id))
}

#[tauri::command]
pub fn hub_staging_add_text(
    app: AppHandle,
    plugin_id: String,
    text: String,
) -> Result<crate::staging::StagingItem, String> {
    crate::plugin_hub::assert_capability(&plugin_id, "staging")?;
    crate::staging::add_text(Some(&app), &plugin_id, text)
}

#[tauri::command]
pub fn hub_staging_add_paths(
    app: AppHandle,
    plugin_id: String,
    paths: Vec<String>,
) -> Result<Vec<crate::staging::StagingItem>, String> {
    crate::plugin_hub::assert_capability(&plugin_id, "staging")?;
    crate::staging::add_paths(Some(&app), &plugin_id, paths)
}

#[tauri::command]
pub fn hub_staging_add_image_bytes(
    app: AppHandle,
    plugin_id: String,
    label: String,
    bytes: Vec<u8>,
    ext: Option<String>,
) -> Result<crate::staging::StagingItem, String> {
    crate::plugin_hub::assert_capability(&plugin_id, "staging")?;
    crate::staging::add_image_bytes(Some(&app), &plugin_id, label, bytes, ext)
}

#[tauri::command]
pub fn hub_staging_remove(app: AppHandle, plugin_id: String, id: String) -> Result<(), String> {
    crate::plugin_hub::assert_capability(&plugin_id, "staging")?;
    crate::staging::remove(Some(&app), &plugin_id, &id)
}

#[tauri::command]
pub fn hub_staging_clear(app: AppHandle, plugin_id: String) -> Result<(), String> {
    crate::plugin_hub::assert_capability(&plugin_id, "staging")?;
    crate::staging::clear(Some(&app), &plugin_id)
}

#[tauri::command]
pub fn hub_staging_copy(plugin_id: String, id: String) -> Result<(), String> {
    crate::plugin_hub::assert_capability(&plugin_id, "staging")?;
    crate::staging::copy_to_clipboard(&plugin_id, &id)
}

#[tauri::command]
pub fn hub_staging_copy_all_paths(plugin_id: String) -> Result<u32, String> {
    crate::plugin_hub::assert_capability(&plugin_id, "staging")?;
    crate::staging::copy_all_paths(&plugin_id)
}

#[tauri::command]
pub fn hub_staging_thumb(plugin_id: String, id: String) -> Result<Option<String>, String> {
    crate::plugin_hub::assert_capability(&plugin_id, "staging")?;
    crate::staging::thumb_data_url(&plugin_id, &id)
}

#[tauri::command]
pub fn hub_staging_reveal(plugin_id: String, id: String) -> Result<(), String> {
    crate::plugin_hub::assert_capability(&plugin_id, "staging")?;
    crate::staging::reveal(&plugin_id, &id)
}

#[tauri::command]
pub fn hub_staging_open(plugin_id: String, id: String) -> Result<(), String> {
    crate::plugin_hub::assert_capability(&plugin_id, "staging")?;
    crate::staging::open(&plugin_id, &id)
}

#[tauri::command]
pub fn hub_staging_start_drag(
    window: tauri::WebviewWindow,
    plugin_id: String,
    ids: Vec<String>,
) -> Result<(), String> {
    crate::plugin_hub::assert_capability(&plugin_id, "staging")?;
    crate::staging::start_drag_out(&window, &plugin_id, &ids)
}

// ── Island bar / panel session (platform slots) ────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IslandBarDto {
    pub plugin_id: String,
    pub text: String,
    pub title: Option<String>,
}

#[tauri::command]
pub fn hub_island_set_bar(
    app: AppHandle,
    plugin_id: String,
    text: String,
    title: Option<String>,
) -> Result<(), String> {
    crate::plugin_hub::assert_capability(&plugin_id, "island.bar")?;
    crate::plugin_hub::assert_plugin_slot(&plugin_id, "island.bar")?;
    let text = text.trim().to_string();
    // Always include plugin_id so Host can clear only that plugin's layer
    // (resident vs temporary overlay) without wiping the other.
    let _ = app.emit(
        "island-bar-changed",
        IslandBarDto {
            plugin_id,
            text,
            title,
        },
    );
    Ok(())
}

#[tauri::command]
pub fn hub_island_clear_bar(app: AppHandle, plugin_id: String) -> Result<(), String> {
    crate::plugin_hub::assert_capability(&plugin_id, "island.bar")?;
    crate::plugin_hub::assert_plugin_slot(&plugin_id, "island.bar")?;
    let _ = app.emit(
        "island-bar-changed",
        IslandBarDto {
            plugin_id,
            text: String::new(),
            title: None,
        },
    );
    Ok(())
}

/// Temporary scenario takeover of island bar + pull panel (does not mutate prefs).
#[tauri::command]
pub fn hub_island_claim_scenario(app: AppHandle, plugin_id: String) -> Result<(), String> {
    crate::plugin_hub::assert_capability(&plugin_id, "island.bar")?;
    crate::plugin_hub::assert_capability(&plugin_id, "island.panel")?;
    crate::plugin_hub::assert_plugin_slot(&plugin_id, "island.scenario")?;
    crate::plugin_hub::assert_plugin_slot(&plugin_id, "island.bar")?;
    crate::plugin_hub::assert_plugin_slot(&plugin_id, "island.panel")?;
    let _ = app.emit(
        "island-scenario",
        serde_json::json!({ "action": "claim", "pluginId": plugin_id }),
    );
    Ok(())
}

#[tauri::command]
pub fn hub_island_release_scenario(app: AppHandle, plugin_id: String) -> Result<(), String> {
    crate::plugin_hub::assert_plugin_slot(&plugin_id, "island.scenario")?;
    let _ = app.emit(
        "island-scenario",
        serde_json::json!({ "action": "release", "pluginId": plugin_id }),
    );
    Ok(())
}

/// Bound tray pin_key from plugin settings `openTrayKey` (scenario panel open-app).
fn plugin_open_tray_key(plugin_id: &str) -> Result<Option<String>, String> {
    let declares = crate::plugin_hub::plugin_declares_setting(plugin_id, "openTrayKey");
    let from_settings = crate::plugin_hub::hub_settings_get_all(plugin_id.to_string())
        .ok()
        .and_then(|all| {
            all.get("openTrayKey")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
    if let Some(key) = from_settings {
        crate::plugin_hub::clear_legacy_open_tray_key(plugin_id);
        return Ok(Some(key));
    }

    let prefs = get_island_prefs();
    let legacy = prefs
        .scenario_gates
        .get(plugin_id)
        .map(|g| g.open_tray_key.trim().to_string())
        .filter(|k| !k.is_empty());

    if declares {
        if let Some(key) = legacy {
            // One-shot migrate into settings so empty settings = unbound afterwards.
            let _ = crate::plugin_hub::write_setting_value(
                plugin_id,
                "openTrayKey",
                serde_json::Value::String(key.clone()),
            );
            return Ok(Some(key));
        }
        return Ok(None);
    }

    Ok(legacy)
}

/// Bound tray pin_key for this scenario plugin (plugin settings `openTrayKey`).
#[tauri::command]
pub fn hub_island_get_bound_tray(plugin_id: String) -> Result<Option<String>, String> {
    crate::plugin_hub::assert_plugin_slot(&plugin_id, "island.scenario")?;
    plugin_open_tray_key(&plugin_id)
}

/// Left-click the tray icon bound in plugin settings (`openTrayKey`).
#[tauri::command]
pub fn hub_island_open_bound_tray(plugin_id: String) -> Result<(), String> {
    crate::plugin_hub::assert_plugin_slot(&plugin_id, "island.scenario")?;
    let key = plugin_open_tray_key(&plugin_id)?
        .ok_or_else(|| "未绑定打开用托盘（插件详情 → 打开应用）".to_string())?;
    // Fuzzy resolve (same rules as 常显 pin / OpenTraySetting): uid may change after reboot.
    let icon = crate::win32::tray::find_icon_by_bind_key(&key)
        .ok_or_else(|| "绑定的托盘当前不在系统托盘中".to_string())?;
    crate::win32::tray::invoke_icon_by_id(
        Some(icon.id.clone()),
        icon.hwnd,
        icon.callback_msg,
        icon.uid,
        icon.version,
        crate::win32::tray::TrayClick::Left,
        None,
    )
}

/// Capture the island window (bar + expanded dashboard) to the clipboard.
#[tauri::command]
pub fn capture_island_screenshot(app: AppHandle) -> Result<(), String> {
    let hwnd = main_hwnd_raw(&app);
    if hwnd == 0 {
        return Err("island window missing".into());
    }
    let (png, _w, _h) = crate::win32::capture::capture_window_png(hwnd)?;
    crate::win32::capture::copy_png_to_clipboard(&png)
}

#[tauri::command]
pub fn hub_panel_open_session(app: AppHandle, plugin_id: String) -> Result<(), String> {
    crate::plugin_hub::assert_capability(&plugin_id, "island.panel")?;
    let _ = app.emit(
        "island-session",
        serde_json::json!({ "action": "open", "pluginId": plugin_id }),
    );
    Ok(())
}

#[tauri::command]
pub fn hub_panel_close_session(app: AppHandle) -> Result<(), String> {
    let _ = app.emit(
        "island-session",
        serde_json::json!({ "action": "close" }),
    );
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubNotifyArgs {
    pub title: String,
    pub body: Option<String>,
    pub icon_png: Option<String>,
    pub urgency: Option<String>,
    pub ttl_ms: Option<u64>,
    pub actions: Option<Vec<serde_json::Value>>,
    /// Body click → fire `onAction` with this id (else dismiss / open panel).
    pub default_action_id: Option<String>,
    pub data: Option<serde_json::Value>,
}

#[tauri::command]
pub fn hub_notify(
    app: AppHandle,
    plugin_id: String,
    opts: HubNotifyArgs,
) -> Result<serde_json::Value, String> {
    crate::plugin_hub::assert_capability(&plugin_id, "notify")?;
    crate::plugin_hub::assert_plugin_slot(&plugin_id, "island.notify")?;
    let title = opts.title.trim().to_string();
    if title.is_empty() {
        return Err("notify title required".into());
    }
    let max_per_minute = crate::plugin_install::find_installed_plugin(&plugin_id)
        .and_then(|p| {
            p.manifest
                .get("slots")
                .and_then(|s| s.get("island.notify"))
                .and_then(|n| n.get("maxPerMinute"))
                .and_then(|v| v.as_u64())
        })
        .unwrap_or(6)
        .clamp(1, 60);
    let id = format!("n-{}-{}", plugin_id, chrono_like_notify_id());
    let urgency = match opts.urgency.as_deref() {
        Some("passive") | Some("critical") | Some("active") => {
            opts.urgency.clone().unwrap_or_else(|| "active".into())
        }
        _ => "active".into(),
    };
    let _ = app.emit(
        "island-notify",
        serde_json::json!({
            "id": id,
            "pluginId": plugin_id,
            "maxPerMinute": max_per_minute,
            "title": title,
            "body": opts.body,
            "iconPng": opts.icon_png,
            "urgency": urgency,
            "ttlMs": opts.ttl_ms,
            "actions": opts.actions.unwrap_or_default(),
            "defaultActionId": opts.default_action_id,
            "data": opts.data,
        }),
    );
    Ok(serde_json::json!({ "id": id }))
}

fn chrono_like_notify_id() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubFetchOpts {
    pub method: Option<String>,
    pub headers: Option<serde_json::Map<String, serde_json::Value>>,
    pub body: Option<String>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubFetchResponse {
    pub status: u16,
    pub ok: bool,
    pub headers: serde_json::Map<String, serde_json::Value>,
    pub body: String,
}

#[tauri::command]
pub fn hub_fetch(
    plugin_id: String,
    url: String,
    opts: Option<HubFetchOpts>,
) -> Result<HubFetchResponse, String> {
    crate::plugin_hub::assert_capability(&plugin_id, "network")?;
    let url = url.trim().to_string();
    if url.is_empty() {
        return Err("url required".into());
    }
    let allow = crate::plugin_hub::network_allowlist(&plugin_id)?;
    if allow.is_empty() {
        return Err("permissions.network is empty — declare allowed hosts".into());
    }
    if !crate::plugin_hub::url_allowed_by_network_list(&url, &allow) {
        return Err(format!("url not allowed by permissions.network: {url}"));
    }

    // Isolate dead backends: concurrency cap + per-origin circuit breaker.
    crate::hub_fetch_guard::check_circuit(&plugin_id, &url)?;
    let _slot = crate::hub_fetch_guard::try_acquire_slot()?;

    let opts = opts.unwrap_or(HubFetchOpts {
        method: None,
        headers: None,
        body: None,
        timeout_ms: None,
    });
    let method = opts
        .method
        .as_deref()
        .unwrap_or("GET")
        .trim()
        .to_ascii_uppercase();
    if !matches!(
        method.as_str(),
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD"
    ) {
        return Err(format!("unsupported method: {method}"));
    }
    // Cap default timeout lower so a hung localhost cannot hold invoke threads for 15s.
    let timeout = std::time::Duration::from_millis(
        opts.timeout_ms.unwrap_or(5_000).clamp(200, 30_000),
    );
    let agent = ureq::AgentBuilder::new().timeout(timeout).build();
    let mut req = match method.as_str() {
        "GET" => agent.get(&url),
        "POST" => agent.post(&url),
        "PUT" => agent.put(&url),
        "PATCH" => agent.request("PATCH", &url),
        "DELETE" => agent.delete(&url),
        "HEAD" => agent.request("HEAD", &url),
        _ => unreachable!(),
    };
    if let Some(headers) = &opts.headers {
        for (k, v) in headers {
            if let Some(s) = v.as_str() {
                req = req.set(k, s);
            }
        }
    }
    let resp = if matches!(method.as_str(), "POST" | "PUT" | "PATCH") {
        let body = opts.body.unwrap_or_default();
        req.send_string(&body)
    } else {
        req.call()
    };
    let resp = match resp {
        Ok(r) => {
            crate::hub_fetch_guard::record_success(&plugin_id, &url);
            r
        }
        Err(e) => {
            let msg = format!("fetch failed: {e}");
            if crate::hub_fetch_guard::is_transport_error(&msg) {
                crate::hub_fetch_guard::record_failure(&plugin_id, &url);
            }
            return Err(msg);
        }
    };

    let status = resp.status();
    let mut headers = serde_json::Map::new();
    for name in resp.headers_names() {
        if let Some(val) = resp.header(&name) {
            headers.insert(name, serde_json::Value::String(val.to_string()));
        }
    }
    let body = if method == "HEAD" {
        String::new()
    } else {
        resp.into_string()
            .map_err(|e| format!("read body: {e}"))?
    };
    Ok(HubFetchResponse {
        status,
        ok: (200..300).contains(&status),
        headers,
        body,
    })
}

// ── Plugin WebView (capability: webview) ────────────

#[tauri::command]
pub async fn hub_webview_open(
    app: AppHandle,
    plugin_id: String,
    opts: crate::plugin_webview::OpenOpts,
) -> Result<serde_json::Value, String> {
    crate::plugin_webview::open(app, plugin_id, opts).await
}

#[tauri::command]
pub fn hub_webview_close(
    app: AppHandle,
    plugin_id: String,
    opts: crate::plugin_webview::SessionIdOpts,
) -> Result<(), String> {
    crate::plugin_webview::close(app, plugin_id, opts)
}

#[tauri::command]
pub fn hub_webview_navigate(
    app: AppHandle,
    plugin_id: String,
    opts: crate::plugin_webview::NavigateOpts,
) -> Result<(), String> {
    crate::plugin_webview::navigate(app, plugin_id, opts)
}

#[tauri::command]
pub async fn hub_webview_start_pick(
    app: AppHandle,
    plugin_id: String,
    opts: crate::plugin_webview::SessionIdOpts,
) -> Result<crate::plugin_webview::PickResult, String> {
    crate::plugin_webview::start_pick(app, plugin_id, opts).await
}

#[tauri::command]
pub fn hub_webview_take_last_pick(
    plugin_id: String,
) -> Result<Option<serde_json::Value>, String> {
    crate::plugin_webview::take_last_pick(plugin_id)
}

#[tauri::command]
pub async fn hub_webview_snapshot(
    app: AppHandle,
    plugin_id: String,
    opts: crate::plugin_webview::SnapshotOpts,
) -> Result<crate::plugin_webview::SnapshotResult, String> {
    crate::plugin_webview::snapshot(app, plugin_id, opts).await
}

#[tauri::command]
pub async fn hub_webview_watch_start(
    app: AppHandle,
    plugin_id: String,
    opts: crate::plugin_webview::WatchStartOpts,
) -> Result<(), String> {
    crate::plugin_webview::watch_start(app, plugin_id, opts).await
}

#[tauri::command]
pub fn hub_webview_watch_stop(
    plugin_id: String,
    opts: crate::plugin_webview::WatchIdOpts,
) -> Result<(), String> {
    crate::plugin_webview::watch_stop(plugin_id, opts)
}

#[tauri::command]
pub fn hub_webview_watch_list(plugin_id: String) -> Result<Vec<crate::plugin_webview::WatchInfo>, String> {
    crate::plugin_webview::watch_list(plugin_id)
}

// ── Island prefs (host business tables) ────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioGateDto {
    #[serde(default)]
    pub tray_keys: Vec<String>,
    #[serde(default)]
    pub window_keys: Vec<String>,
    /// Stable tray pin_key for `hub.island.openBoundTray` (legacy; prefer plugin settings `openTrayKey`).
    #[serde(default)]
    pub open_tray_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IslandPrefsDto {
    pub auto_immerse: bool,
    pub immerse_idle_sec: u32,
    pub pull_content: String,
    #[serde(default = "default_bar_resident_pref")]
    pub bar_resident: String,
    /// Top bar themed blur material (sample + glass overlay, or desktop blur only).
    #[serde(default = "default_bar_glass_pref")]
    pub bar_glass: bool,
    /// Never sample these apps' window chrome (`exe:` / `proc:` bind keys).
    #[serde(default)]
    pub ignore_ambient_apps: Vec<String>,
    pub msg_notify: bool,
    pub msg_notify_text: String,
    pub msg_notify_sec: u32,
    /// pluginId → presence gates (stable tray pin_key / window exe key)
    #[serde(default)]
    pub scenario_gates: std::collections::HashMap<String, ScenarioGateDto>,
}

fn default_bar_glass_pref() -> bool {
    true
}

fn default_bar_resident_pref() -> String {
    "com.window-hub.weather".into()
}

impl Default for IslandPrefsDto {
    fn default() -> Self {
        Self {
            auto_immerse: true,
            immerse_idle_sec: 8,
            pull_content: "plugin:com.window-hub.weather".into(),
            bar_resident: default_bar_resident_pref(),
            bar_glass: true,
            ignore_ambient_apps: Vec::new(),
            msg_notify: true,
            msg_notify_text: "收到一条消息".into(),
            msg_notify_sec: 4,
            scenario_gates: std::collections::HashMap::new(),
        }
    }
}

fn parse_scenario_gates_json(raw: &str) -> std::collections::HashMap<String, ScenarioGateDto> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn scenario_gates_to_json(gates: &std::collections::HashMap<String, ScenarioGateDto>) -> String {
    serde_json::to_string(gates).unwrap_or_else(|_| "{}".into())
}

fn normalize_scenario_gates(
    gates: std::collections::HashMap<String, ScenarioGateDto>,
) -> std::collections::HashMap<String, ScenarioGateDto> {
    let mut out = std::collections::HashMap::new();
    for (pid, g) in gates {
        let plugin_id = pid.trim().to_string();
        if plugin_id.is_empty() {
            continue;
        }
        let mut tray = Vec::new();
        let mut seen_t = std::collections::HashSet::new();
        for k in g.tray_keys {
            let t = k.trim().to_string();
            if t.is_empty() || !seen_t.insert(t.clone()) {
                continue;
            }
            tray.push(t);
        }
        let mut win = Vec::new();
        let mut seen_w = std::collections::HashSet::new();
        for k in g.window_keys {
            let t = k.trim().to_string();
            if t.is_empty() || !seen_w.insert(t.clone()) {
                continue;
            }
            win.push(t);
        }
        let open_tray_key = g.open_tray_key.trim().to_string();
        if tray.is_empty() && win.is_empty() && open_tray_key.is_empty() {
            continue;
        }
        out.insert(
            plugin_id,
            ScenarioGateDto {
                tray_keys: tray,
                window_keys: win,
                open_tray_key,
            },
        );
    }
    out
}

impl From<crate::db::IslandPrefsRow> for IslandPrefsDto {
    fn from(p: crate::db::IslandPrefsRow) -> Self {
        Self {
            auto_immerse: p.auto_immerse,
            immerse_idle_sec: p.immerse_idle_sec,
            pull_content: p.pull_content,
            bar_resident: p.bar_resident,
            bar_glass: p.bar_glass,
            ignore_ambient_apps: parse_ignore_ambient_apps_json(&p.ignore_ambient_apps_json),
            msg_notify: p.msg_notify,
            msg_notify_text: p.msg_notify_text,
            msg_notify_sec: p.msg_notify_sec,
            scenario_gates: parse_scenario_gates_json(&p.scenario_gates_json),
        }
    }
}

impl From<&IslandPrefsDto> for crate::db::IslandPrefsRow {
    fn from(p: &IslandPrefsDto) -> Self {
        Self {
            auto_immerse: p.auto_immerse,
            immerse_idle_sec: p.immerse_idle_sec,
            pull_content: p.pull_content.clone(),
            bar_resident: p.bar_resident.clone(),
            bar_glass: p.bar_glass,
            ignore_ambient_apps_json: ignore_ambient_apps_to_json(&p.ignore_ambient_apps),
            msg_notify: p.msg_notify,
            msg_notify_text: p.msg_notify_text.clone(),
            msg_notify_sec: p.msg_notify_sec,
            scenario_gates_json: scenario_gates_to_json(&p.scenario_gates),
        }
    }
}

fn normalize_pull_content(raw: &str) -> String {
    match raw.trim() {
        "" | "none" | "off" => String::new(),
        "weather" => "plugin:com.window-hub.weather".into(),
        "mirror" => "plugin:com.window-hub.mirror".into(),
        s if s.starts_with("plugin:") && s.len() > "plugin:".len() => s.to_string(),
        // Do not force-default to weather — Host must not keep a disabled plugin as pull target.
        _ => String::new(),
    }
}

fn normalize_bar_resident(raw: &str) -> String {
    let t = raw.trim();
    if t.is_empty() || t == "none" || t == "off" {
        return String::new();
    }
    if let Some(rest) = t.strip_prefix("plugin:") {
        return rest.to_string();
    }
    t.to_string()
}

fn parse_ignore_ambient_apps_json(raw: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Vec<String>>(raw) else {
        return Vec::new();
    };
    normalize_ignore_ambient_apps(v)
}

fn ignore_ambient_apps_to_json(apps: &[String]) -> String {
    serde_json::to_string(&normalize_ignore_ambient_apps(apps.to_vec()))
        .unwrap_or_else(|_| "[]".into())
}

fn normalize_ignore_ambient_apps(apps: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for k in apps {
        let t = k.trim().to_string();
        if t.is_empty() || !seen.insert(t.clone()) {
            continue;
        }
        out.push(t);
    }
    out
}

fn normalize_island_prefs(mut p: IslandPrefsDto) -> IslandPrefsDto {
    p.immerse_idle_sec = p.immerse_idle_sec.clamp(2, 300);
    p.msg_notify_sec = p.msg_notify_sec.clamp(2, 30);
    p.pull_content = normalize_pull_content(&p.pull_content);
    p.bar_resident = normalize_bar_resident(&p.bar_resident);
    p.scenario_gates = normalize_scenario_gates(p.scenario_gates);
    p.ignore_ambient_apps = normalize_ignore_ambient_apps(p.ignore_ambient_apps);
    p.msg_notify_text = {
        let t = p.msg_notify_text.trim().to_string();
        if t.is_empty() {
            "收到一条消息".into()
        } else {
            t
        }
    };
    p
}

#[tauri::command]
pub fn get_island_prefs() -> IslandPrefsDto {
    if let Ok(Some(p)) = crate::db::with_conn(|c| crate::db::island_get(c)) {
        let raw = IslandPrefsDto::from(p);
        let next = normalize_island_prefs(raw.clone());
        // Persist legacy weather|mirror → plugin:* once
        if next.pull_content != raw.pull_content {
            let row = crate::db::IslandPrefsRow::from(&next);
            let _ = crate::db::with_conn(|c| crate::db::island_set(c, &row));
        }
        return next;
    }
    IslandPrefsDto::default()
}

#[tauri::command]
pub fn set_island_prefs(app: AppHandle, prefs: IslandPrefsDto) -> Result<IslandPrefsDto, String> {
    // FE no longer writes openTrayKey (plugin settings). Migrate any leftover Host
    // values into settings before dropping them from island prefs.
    let prev = get_island_prefs();
    for (pid, old) in &prev.scenario_gates {
        let legacy = old.open_tray_key.trim();
        if legacy.is_empty() {
            continue;
        }
        if !crate::plugin_hub::plugin_declares_setting(pid, "openTrayKey") {
            continue;
        }
        let already = crate::plugin_hub::hub_settings_get_all(pid.clone())
            .ok()
            .and_then(|all| {
                all.get("openTrayKey")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            });
        if already.is_some() {
            continue;
        }
        let _ = crate::plugin_hub::write_setting_value(
            pid,
            "openTrayKey",
            serde_json::Value::String(legacy.to_string()),
        );
    }
    let mut prefs = prefs;
    for g in prefs.scenario_gates.values_mut() {
        g.open_tray_key.clear();
    }
    let next = normalize_island_prefs(prefs);
    let bar_glass_changed = prev.bar_glass != next.bar_glass;
    let ignore_ambient_apps_changed = prev.ignore_ambient_apps != next.ignore_ambient_apps;
    let row = crate::db::IslandPrefsRow::from(&next);
    crate::db::with_conn(|c| crate::db::island_set(c, &row))?;
    let _ = app.emit("island-prefs", &next);
    if bar_glass_changed || ignore_ambient_apps_changed {
        apply_main_window_material(&app);
    }
    if ignore_ambient_apps_changed {
        #[cfg(windows)]
        {
            crate::win32::ambient::sync_ignore_ambient_apps(next.ignore_ambient_apps.clone());
            crate::win32::ambient::reset_sampling_gate();
            let hwnd = main_hwnd(&app);
            let strip = crate::win32::ambient::sample(hwnd);
            let _ = app.emit("ambient-color", &strip);
        }
    }
    Ok(next)
}

/// When a plugin is disabled/uninstalled, drop it from island pull + bar resident.
pub fn detach_plugin_from_island_prefs(app: &AppHandle, plugin_id: &str) {
    let cur = get_island_prefs();
    let pull_id = format!("plugin:{plugin_id}");
    let mut next = cur;
    let mut changed = false;
    if next.pull_content == pull_id {
        next.pull_content.clear();
        changed = true;
    }
    if next.bar_resident == plugin_id {
        next.bar_resident.clear();
        changed = true;
    }
    if next.scenario_gates.remove(plugin_id).is_some() {
        changed = true;
    }
    if !changed {
        return;
    }
    let _ = set_island_prefs(app.clone(), next);
}

#[cfg(windows)]
pub(crate) fn send_media_virtual_key(vk: u16) -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        keybd_event, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
    };
    unsafe {
        keybd_event(vk as u8, 0, KEYEVENTF_EXTENDEDKEY, 0);
        keybd_event(vk as u8, 0, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP, 0);
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn send_media_virtual_key(_vk: u16) -> Result<(), String> {
    Err("media.keys only available on Windows".into())
}

/// Send a system media key. Requires `media.keys`.
/// `action`: play_pause | next | previous | stop
#[tauri::command]
pub fn hub_media_send_key(plugin_id: String, action: String) -> Result<(), String> {
    crate::plugin_hub::assert_capability(&plugin_id, "media.keys")?;
    let act = action.trim().to_ascii_lowercase().replace('-', "_");
    // VK_MEDIA_* 
    let vk: u16 = match act.as_str() {
        "next" | "next_track" => 0xB0,
        "previous" | "prev" | "previous_track" => 0xB1,
        "stop" => 0xB2,
        "play_pause" | "playpause" | "toggle" => 0xB3,
        _ => return Err(format!("unknown media action: {action}")),
    };
    send_media_virtual_key(vk)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutsPluginScopeDto {
    /// `all` | `apps` — default all when missing.
    #[serde(default = "default_shortcuts_scope_mode")]
    pub mode: String,
    #[serde(default)]
    pub window_keys: Vec<String>,
}

fn default_shortcuts_scope_mode() -> String {
    "all".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutsPrefsDto {
    /// When set, shortcuts bar only shows this plugin entry + its pins.
    #[serde(default)]
    pub exclusive_plugin_id: Option<String>,
    /// User order of shortcuts plugin ids (Ctrl+drag). `None` = leave unchanged on patch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_order: Option<Vec<String>>,
    /// Per-plugin visibility: all programs vs selected window keys.
    /// `None` on patch = leave unchanged; `Some({})` clears custom scopes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scopes: Option<std::collections::HashMap<String, ShortcutsPluginScopeDto>>,
    /// pluginId → `"left"` | `"right"`. Missing = left. `None` on patch = leave unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_sides: Option<std::collections::HashMap<String, String>>,
}

impl Default for ShortcutsPrefsDto {
    fn default() -> Self {
        Self {
            exclusive_plugin_id: None,
            plugin_order: None,
            scopes: None,
            plugin_sides: None,
        }
    }
}

fn normalize_shortcuts_prefs(mut p: ShortcutsPrefsDto) -> ShortcutsPrefsDto {
    if let Some(id) = p.exclusive_plugin_id.as_mut() {
        let t = id.trim().to_string();
        if t.is_empty() {
            p.exclusive_plugin_id = None;
        } else {
            *id = t;
        }
    }
    if let Some(order) = p.plugin_order.as_mut() {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::with_capacity(order.len());
        for id in order.drain(..) {
            let t = id.trim().to_string();
            if t.is_empty() || !seen.insert(t.clone()) {
                continue;
            }
            out.push(t);
        }
        *order = out;
    }
    if let Some(scopes) = p.scopes.as_mut() {
        let mut cleaned = std::collections::HashMap::new();
        for (pid, scope) in scopes.drain() {
            let id = pid.trim().to_string();
            if id.is_empty() {
                continue;
            }
            let mode = if scope.mode.trim().eq_ignore_ascii_case("apps") {
                "apps"
            } else {
                "all"
            };
            let mut seen = std::collections::HashSet::new();
            let mut keys = Vec::new();
            for k in scope.window_keys {
                let t = k.trim().to_string();
                if t.is_empty() || !seen.insert(t.clone()) {
                    continue;
                }
                keys.push(t);
            }
            if mode == "all" && keys.is_empty() {
                continue;
            }
            cleaned.insert(
                id,
                ShortcutsPluginScopeDto {
                    mode: mode.into(),
                    window_keys: keys,
                },
            );
        }
        *scopes = cleaned;
    }
    if let Some(sides) = p.plugin_sides.as_mut() {
        let mut cleaned = std::collections::HashMap::new();
        for (pid, side) in sides.drain() {
            let id = pid.trim().to_string();
            if id.is_empty() {
                continue;
            }
            if side.trim().eq_ignore_ascii_case("right") {
                cleaned.insert(id, "right".into());
            }
            // omit "left" defaults
        }
        *sides = cleaned;
    }
    p
}

#[tauri::command]
pub fn get_shortcuts_prefs() -> ShortcutsPrefsDto {
    if let Ok(Some(v)) = crate::db::with_conn(|c| crate::db::shortcuts_get(c)) {
        if let Ok(p) = serde_json::from_value::<ShortcutsPrefsDto>(v) {
            return normalize_shortcuts_prefs(p);
        }
    }
    ShortcutsPrefsDto::default()
}

#[tauri::command]
pub fn set_shortcuts_prefs(
    app: AppHandle,
    prefs: ShortcutsPrefsDto,
) -> Result<ShortcutsPrefsDto, String> {
    let patch = normalize_shortcuts_prefs(prefs);
    let mut next = get_shortcuts_prefs();
    next.exclusive_plugin_id = patch.exclusive_plugin_id;
    if let Some(order) = patch.plugin_order {
        next.plugin_order = Some(order);
    }
    if let Some(scopes) = patch.scopes {
        next.scopes = Some(scopes);
    }
    if let Some(sides) = patch.plugin_sides {
        next.plugin_sides = Some(sides);
    }
    let val = serde_json::to_value(&next).map_err(|e| e.to_string())?;
    crate::db::with_conn(|c| crate::db::shortcuts_set(c, &val))?;
    let _ = app.emit("shortcuts-prefs", &next);
    Ok(next)
}


