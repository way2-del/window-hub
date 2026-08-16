use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State, WebviewUrl, WebviewWindow,
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

/// 岛展开 / 拉高：面板伸进工作区，保持 TOPMOST。
#[tauri::command]
pub fn float_overlay(window: WebviewWindow) -> Result<(), String> {
    let _ = window.set_skip_taskbar(true);
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let raw = hwnd.0 as isize;
    crate::win32::switcher::exclude_from_switcher(raw);
    crate::win32::topmost::set_main_hwnd(raw);
    crate::win32::topmost::set_overlay_raised(true);
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_always_on_top(true);
    Ok(())
}

/// 岛收回折叠条：仍保持 TOPMOST（防壁纸软件 / 显示桌面埋掉顶栏）。
#[tauri::command]
pub fn settle_overlay(window: WebviewWindow) -> Result<(), String> {
    let _ = window.set_skip_taskbar(true);
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let raw = hwnd.0 as isize;
    crate::win32::switcher::exclude_from_switcher(raw);
    crate::win32::topmost::set_main_hwnd(raw);
    crate::win32::topmost::set_overlay_raised(false);
    let _ = crate::win32::topmost::ensure_main_visible();
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_always_on_top(true);
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
) -> Result<(), String> {
    let focus = plugin_id
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
        return Ok(());
    }

    let focus_js = focus
        .as_ref()
        .map(|pid| {
            format!(
                "window.__WH_SETTINGS_FOCUS_PLUGIN__ = {};",
                serde_json::to_string(pid).unwrap_or_else(|_| "null".into())
            )
        })
        .unwrap_or_default();

    let init = format!(
        r#"
      window.__WH_IS_SETTINGS__ = true;
      {focus_js}
      document.addEventListener('keydown', function (e) {{
        if (e.key === 'Escape') {{
          try {{ window.__TAURI__.core.invoke('close_settings_window'); }} catch (_) {{}}
        }}
      }});
    "#
    );

    // async command 会把创建窗口挪出 IPC 同步路径，避免 WebView2 死锁
    let win = WebviewWindowBuilder::new(
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
    .build()
    .map_err(|e| format!("open settings failed: {e}"))?;

    // Mica before show — avoids a white undecorated frame on first paint.
    apply_saved_material(&win, &state);
    let _ = win.show();
    let _ = win.set_focus();
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

/// 独立窄高托盘弹窗：与设置/插件共用材质配置。
/// Kept invisible until the webview fits content — same path as status menu.
#[tauri::command]
pub async fn open_tray_popup(
    app: AppHandle,
    state: State<'_, MaterialState>,
    x: f64,
    y: f64,
) -> Result<(), String> {
    close_sibling_popups(&app, "tray-popup");

    if let Some(existing) = app.get_webview_window("tray-popup") {
        apply_saved_material(&existing, &state);
        let _ = existing.hide();
        let _ = existing.set_size(LogicalSize::new(TRAY_POPUP_W, TRAY_POPUP_H));
        let _ = existing.set_position(LogicalPosition::new(x, y));
        let _ = existing.unminimize();
        let _ = app.emit("tray-popup-opened", ());
        return Ok(());
    }

    let init = r#"
      window.__WH_IS_TRAY_POPUP__ = true;
      document.addEventListener('keydown', function (e) {
        if (e.key === 'Escape') {
          try { window.__TAURI__.core.invoke('close_tray_popup'); } catch (_) {}
        }
      });
    "#;

    let win = WebviewWindowBuilder::new(
        &app,
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
    .initialization_script(init)
    .build()
    .map_err(|e| format!("open tray popup failed: {e}"))?;

    let _ = win.set_position(LogicalPosition::new(x, y));
    apply_saved_material(&win, &state);
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
    }
    // Do not show yet — TrayPopupApp fits height, then slide-reveal.
    let _ = app.emit("tray-popup-opened", ());
    Ok(())
}

#[tauri::command]
pub async fn close_tray_popup(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("tray-popup") {
        w.close().map_err(|e| e.to_string())?;
    }
    let _ = app.emit("tray-popup-closed", ());
    Ok(())
}

#[tauri::command]
pub fn is_tray_popup_open(app: AppHandle) -> bool {
    app.get_webview_window("tray-popup")
        .map(|w| w.is_visible().unwrap_or(false))
        .unwrap_or(false)
}

const STATUS_MENU_POPUP_W: f64 = 200.0;
/// Placeholder only — frontend measures + fits while still hidden, then shows.
const STATUS_MENU_POPUP_H: f64 = 340.0;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusMenuOpenPayload {
    from_dock: bool,
    item_id: Option<String>,
    /// Pinned tile id to insert a separator after (gap / “在右侧”).
    after_item_id: Option<String>,
    pin_bottom: Option<f64>,
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
    format!(
        r#"
      window.__WH_IS_STATUS_MENU_POPUP__ = true;
      window.__WH_STATUS_MENU_FROM_DOCK__ = {from_dock};
      window.__WH_STATUS_MENU_ITEM_ID__ = {item_id};
      window.__WH_STATUS_MENU_AFTER_ITEM_ID__ = {after_item_id};
      window.__WH_STATUS_MENU_PIN_BOTTOM__ = {pin_bottom};
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
    let _ = win.eval(&format!(
        "window.__WH_STATUS_MENU_FROM_DOCK__ = {from_dock}; window.__WH_STATUS_MENU_ITEM_ID__ = {item_id}; window.__WH_STATUS_MENU_AFTER_ITEM_ID__ = {after_item_id}; window.__WH_STATUS_MENU_PIN_BOTTOM__ = {pin_bottom};"
    ));
}

/// 左侧状态菜单弹窗：与插件/托盘共用 MicaAlt 材质与深浅色。
/// Kept invisible until the webview fits content — avoids 80→full height stutter.
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
) -> Result<(), String> {
    // Hold BEFORE closing siblings / focus moves — AutoHide leave must not win.
    let from_dock = from_dock.unwrap_or(false);
    if from_dock {
        set_dock_menu_hold(&app, true);
    }

    close_sibling_popups(&app, "status-menu-popup");
    #[cfg(windows)]
    crate::win32::blur_glass::strip_dock_windows(&app);

    let payload = StatusMenuOpenPayload {
        from_dock,
        item_id: item_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        after_item_id: after_item_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        pin_bottom,
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
        let _ = app.emit("status-menu-popup-opened", &payload);
        return Ok(());
    }

    let win = WebviewWindowBuilder::new(
        &app,
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
    .initialization_script(status_menu_init_script(&payload))
    .build()
    .map_err(|e| format!("open status menu popup failed: {e}"))?;

    let _ = win.set_position(LogicalPosition::new(x, y));
    let _ = win.set_shadow(false);
    apply_saved_material(&win, &state);
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
        #[cfg(windows)]
        crate::win32::blur_glass::strip_frameless_popup_titlebar(hwnd.0 as isize);
    }
    // Do not show yet — StatusMenuPopupApp fits height, then show().
    let _ = app.emit("status-menu-popup-opened", &payload);
    Ok(())
}

#[tauri::command]
pub async fn close_status_menu_popup(app: AppHandle) -> Result<(), String> {
    set_dock_menu_hold(&app, false);
    if let Some(w) = app.get_webview_window("status-menu-popup") {
        w.close().map_err(|e| e.to_string())?;
    }
    let _ = app.emit("status-menu-popup-closed", ());
    Ok(())
}

#[tauri::command]
pub fn is_status_menu_popup_open(app: AppHandle) -> bool {
    app.get_webview_window("status-menu-popup")
        .map(|w| w.is_visible().unwrap_or(false))
        .unwrap_or(false)
}

const PLUGIN_POPUP_W: f64 = 320.0;
const PLUGIN_POPUP_H: f64 = 480.0;
const PLUGIN_POPUP_W_MIN: f64 = 280.0;
const PLUGIN_POPUP_W_MAX: f64 = 720.0;
const PLUGIN_POPUP_H_MIN: f64 = 320.0;
const PLUGIN_POPUP_H_MAX: f64 = 900.0;

fn clamp_popup_size(w: f64, h: f64) -> (f64, f64) {
    (
        w.clamp(PLUGIN_POPUP_W_MIN, PLUGIN_POPUP_W_MAX),
        h.clamp(PLUGIN_POPUP_H_MIN, PLUGIN_POPUP_H_MAX),
    )
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
) -> Result<(), String> {
    close_sibling_popups(&app, "plugin-popup");

    let record = crate::plugin_install::find_installed_plugin(&plugin_id)
        .ok_or_else(|| "plugin not installed".to_string())?;
    if !record.enabled {
        return Err("plugin disabled".into());
    }
    crate::plugin_hub::assert_capability(&plugin_id, "popup")?;

    let (popup_w, popup_h) = resolve_plugin_popup_size(&plugin_id, width, height);

    // Ensure popup entry exists (and asset scope for any future direct loads)
    let popup = plugin_popup_path(&record)?;
    let parent = popup
        .parent()
        .ok_or_else(|| "plugin popup has no parent directory".to_string())?;
    let _ = app.asset_protocol_scope().allow_directory(parent, true);

    // Idempotent: same plugin popup already visible → do not recreate (avoids hover/slide spam).
    if let Some(existing) = app.get_webview_window("plugin-popup") {
        let already =
            existing.is_visible().unwrap_or(false) && popup_plugin_id_of(&existing) == Some(plugin_id.clone());
        if already {
            if let Some(gid) = prefer_group_id.as_ref().filter(|s| !s.is_empty()) {
                let _ = app.emit("plugin-popup-prefer-group", gid);
            }
            let _ = existing.set_size(LogicalSize::new(popup_w, popup_h));
            let _ = existing.set_position(LogicalPosition::new(x, y));
            let _ = existing.set_focus();
            let _ = app.emit("plugin-popup-opened", &plugin_id);
            return Ok(());
        }
        let _ = existing.close();
        let _ = app.emit("plugin-popup-closed", ());
        std::thread::sleep(std::time::Duration::from_millis(40));
    }

    let mut url_s = format!("index.html?window=plugin-popup&plugin={plugin_id}");
    if let Some(gid) = prefer_group_id.as_ref().filter(|s| !s.is_empty()) {
        url_s.push_str("&preferGroup=");
        url_s.push_str(&urlencoding_minimal(gid));
    }
    let url = WebviewUrl::App(url_s.into());
    let init = hub_init_script(&plugin_id);

    let win = WebviewWindowBuilder::new(&app, "plugin-popup", url)
        .title(
            record
                .manifest
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("插件"),
        )
        .inner_size(popup_w, popup_h)
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
        .build()
        .map_err(|e| format!("open plugin popup failed: {e}"))?;

    let _ = win.set_position(LogicalPosition::new(x, y));
    apply_saved_material(&win, &state);
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
    }
    let _ = win.show();
    let _ = win.set_focus();
    let _ = app.emit("plugin-popup-opened", &plugin_id);
    Ok(())
}

#[tauri::command]
pub async fn close_plugin_popup(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("plugin-popup") {
        w.close().map_err(|e| e.to_string())?;
    }
    let _ = app.emit("plugin-popup-closed", ());
    Ok(())
}

#[tauri::command]
pub fn is_plugin_popup_open(app: AppHandle) -> bool {
    app.get_webview_window("plugin-popup")
        .map(|w| w.is_visible().unwrap_or(false))
        .unwrap_or(false)
}

fn load_material_prefs() -> crate::win32::material::MaterialPrefs {
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

/// Re-apply materials to every open popup after prefs change.
fn reapply_material_to_popups(app: &AppHandle, prefs: &crate::win32::material::MaterialPrefs) {
    for label in [
        "settings",
        "dock-icon-editor",
        "tray-popup",
        "plugin-popup",
        "status-menu-popup",
        "input-lang-popup",
        "wifi-popup",
        "wifi-auth-popup",
        "chrome-hover-tip",
        "dock",
        "dock-glass",
    ] {
        if let Some(w) = app.get_webview_window(label) {
            let _ = crate::win32::material::apply_prefs(&w, prefs);
        }
    }
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
    let base = read_material_prefs(&state);
    let prefs = if let Some(m) = material {
        let mut p = base;
        p.kind = crate::win32::material::WindowMaterial::parse(&m);
        p
    } else {
        base
    };
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

/// 采样当前窗口顶边整条色带（左右可变色）。
#[tauri::command]
pub fn sample_ambient_color(app: AppHandle) -> crate::win32::ambient::AmbientStrip {
    crate::win32::ambient::sample(main_hwnd(&app))
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

#[tauri::command]
pub fn list_tray_icons() -> Vec<crate::win32::tray::TrayIconInfo> {
    crate::win32::tray::list_icons()
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

    if let Some(existing) = app.get_webview_window("input-lang-popup") {
        apply_saved_material(&existing, &state);
        let _ = existing.set_size(LogicalSize::new(INPUT_LANG_POPUP_W, INPUT_LANG_POPUP_H));
        let _ = existing.set_position(LogicalPosition::new(x, y));
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_focus();
        let _ = app.emit("input-lang-popup-opened", ());
        return Ok(());
    }

    let init = r#"
      window.__WH_IS_INPUT_LANG_POPUP__ = true;
      document.addEventListener('keydown', function (e) {
        if (e.key === 'Escape') {
          try { window.__TAURI__.core.invoke('close_input_lang_popup'); } catch (_) {}
        }
      });
    "#;

    let win = WebviewWindowBuilder::new(
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
    .build()
    .map_err(|e| format!("open input-lang popup failed: {e}"))?;

    let _ = win.set_position(LogicalPosition::new(x, y));
    apply_saved_material(&win, &state);
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
    }
    let _ = win.show();
    let _ = win.set_focus();
    let _ = app.emit("input-lang-popup-opened", ());
    Ok(())
}

#[tauri::command]
pub async fn close_input_lang_popup(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("input-lang-popup") {
        w.close().map_err(|e| e.to_string())?;
    }
    let _ = app.emit("input-lang-popup-closed", ());
    Ok(())
}

#[tauri::command]
pub fn is_input_lang_popup_open(app: AppHandle) -> bool {
    app.get_webview_window("input-lang-popup")
        .map(|w| w.is_visible().unwrap_or(false))
        .unwrap_or(false)
}

static CHROME_HOVER_TIP: Mutex<Option<ChromeHoverTipPayload>> = Mutex::new(None);
/// Backend-owned generation. Any show/close bumps this so in-flight work can detect supersession.
/// Do NOT trust per-webview JS counters — main/dock/tray each have their own tipEpoch and
/// desync permanently rejects shows (tip works once, then never again).
static CHROME_HOVER_TIP_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_jpeg_base64: Option<String>,
    /// When set with a preview image, tip is interactive (close button).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hwnd: Option<i64>,
    /// Dock item id — click preview launches/focuses like clicking the icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
}

const CHROME_HOVER_TIP_W: f64 = 160.0;
const CHROME_HOVER_TIP_H: f64 = 48.0;

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
    set_dock_preview_tip_keep(app, false);
    if let Some(w) = app.get_webview_window("chrome-hover-tip") {
        let _ = w.hide();
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
            // Two samples (~200ms) outside Host → dismiss (covers fast flick to desktop).
            if misses >= 2 {
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
    #[allow(unused_variables)] epoch: Option<u64>,
) -> Result<(), String> {
    let lines: Vec<String> = lines
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .take(8)
        .collect();
    let image = image_jpeg_base64
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if lines.is_empty() && image.is_none() {
        return close_chrome_hover_tip(app, None).await;
    }

    // Claim a generation for this show; close/newer show will bump past it.
    let seq = CHROME_HOVER_TIP_EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;

    let placement = placement
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| s == "above" || s == "below");
    let interactive = image.is_some() && hwnd.map(|h| h != 0).unwrap_or(false);
    let item_id = item_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let payload = ChromeHoverTipPayload {
        lines,
        x,
        y,
        placement,
        image_jpeg_base64: image,
        hwnd: if interactive { hwnd } else { None },
        item_id: if interactive { item_id } else { None },
    };
    if let Ok(mut g) = CHROME_HOVER_TIP.lock() {
        *g = Some(payload.clone());
    }
    // Hold AutoHide for interactive previews. Only arm here — never clear on a
    // non-interactive refresh (progressive title-first tip must not drop keep).
    if interactive {
        set_dock_preview_tip_keep(&app, true);
    }

    let still_current =
        || CHROME_HOVER_TIP_EPOCH.load(std::sync::atomic::Ordering::SeqCst) == seq;

    if let Some(existing) = app.get_webview_window("chrome-hover-tip") {
        if !still_current() {
            return Ok(());
        }
        apply_saved_material(&existing, &state);
        let _ = existing.set_size(LogicalSize::new(CHROME_HOVER_TIP_W, CHROME_HOVER_TIP_H));
        let _ = existing.set_position(LogicalPosition::new(x, y));
        let _ = existing.set_always_on_top(true);
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_ignore_cursor_events(!interactive);
        if !still_current() {
            let _ = existing.hide();
            return Ok(());
        }
        let _ = app.emit("chrome-hover-tip-show", &payload);
        spawn_chrome_tip_leave_watch(app.clone(), seq);
        return Ok(());
    }

    let init = r#"
      window.__WH_IS_CHROME_HOVER_TIP__ = true;
    "#;

    let win = WebviewWindowBuilder::new(
        &app,
        "chrome-hover-tip",
        WebviewUrl::App("index.html?window=chrome-tip".into()),
    )
    .title("提示")
    .inner_size(CHROME_HOVER_TIP_W, CHROME_HOVER_TIP_H)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .closable(false)
    .decorations(false)
    .transparent(true)
    .background_color(Color(0, 0, 0, 0))
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .visible(false)
    .initialization_script(init)
    .build()
    .map_err(|e| format!("open chrome hover tip failed: {e}"))?;

    if !still_current() {
        let _ = win.close();
        return Ok(());
    }

    let _ = win.set_position(LogicalPosition::new(x, y));
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
    }
    apply_saved_material(&win, &state);
    let _ = win.set_ignore_cursor_events(!interactive);
    let _ = win.set_always_on_top(true);
    let _ = win.show();
    apply_saved_material(&win, &state);
    if !still_current() {
        let _ = win.hide();
        return Ok(());
    }
    let _ = app.emit("chrome-hover-tip-show", &payload);
    let app2 = app.clone();
    let payload2 = payload.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        if CHROME_HOVER_TIP_EPOCH.load(std::sync::atomic::Ordering::SeqCst) != seq {
            return;
        }
        if CHROME_HOVER_TIP
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .is_none()
        {
            return;
        }
        let _ = app2.emit("chrome-hover-tip-show", &payload2);
    });
    spawn_chrome_tip_leave_watch(app.clone(), seq);
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

fn close_sibling_popups(app: &AppHandle, except: &str) {
    for label in [
        "tray-popup",
        "plugin-popup",
        "status-menu-popup",
        "input-lang-popup",
        "wifi-popup",
        "wifi-auth-popup",
    ] {
        if label == except {
            continue;
        }
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.close();
            match label {
                "tray-popup" => {
                    let _ = app.emit("tray-popup-closed", ());
                }
                "plugin-popup" => {
                    let _ = app.emit("plugin-popup-closed", ());
                }
                "status-menu-popup" => {
                    set_dock_menu_hold(app, false);
                    let _ = app.emit("status-menu-popup-closed", ());
                }
                "input-lang-popup" => {
                    let _ = app.emit("input-lang-popup-closed", ());
                }
                "wifi-popup" => {
                    let _ = app.emit("wifi-popup-closed", ());
                }
                "wifi-auth-popup" => {
                    let _ = app.emit("wifi-auth-popup-closed", ());
                }
                _ => {}
            }
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

    if let Some(existing) = app.get_webview_window("wifi-popup") {
        apply_saved_material(&existing, &state);
        let _ = existing.set_size(LogicalSize::new(WIFI_POPUP_W, WIFI_POPUP_H));
        let _ = existing.set_position(LogicalPosition::new(x, y));
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_focus();
        let _ = app.emit("wifi-popup-opened", ());
        return Ok(());
    }

    let init = r#"
      window.__WH_IS_WIFI_POPUP__ = true;
      document.addEventListener('keydown', function (e) {
        if (e.key === 'Escape') {
          try { window.__TAURI__.core.invoke('close_wifi_popup'); } catch (_) {}
        }
      });
    "#;

    let win = WebviewWindowBuilder::new(
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
    .build()
    .map_err(|e| format!("open wifi popup failed: {e}"))?;

    let _ = win.set_position(LogicalPosition::new(x, y));
    apply_saved_material(&win, &state);
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
    }
    let _ = win.show();
    let _ = win.set_focus();
    let _ = app.emit("wifi-popup-opened", ());
    Ok(())
}

#[tauri::command]
pub async fn close_wifi_popup(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("wifi-popup") {
        w.close().map_err(|e| e.to_string())?;
    }
    let _ = app.emit("wifi-popup-closed", ());
    Ok(())
}

#[tauri::command]
pub fn is_wifi_popup_open(app: AppHandle) -> bool {
    app.get_webview_window("wifi-popup")
        .map(|w| w.is_visible().unwrap_or(false))
        .unwrap_or(false)
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

    // Close the WLAN menu but keep other chrome; auth is the focused modal.
    if let Some(w) = app.get_webview_window("wifi-popup") {
        let _ = w.close();
        let _ = app.emit("wifi-popup-closed", ());
    }
    for label in ["tray-popup", "plugin-popup", "status-menu-popup", "input-lang-popup"] {
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.close();
            match label {
                "tray-popup" => {
                    let _ = app.emit("tray-popup-closed", ());
                }
                "plugin-popup" => {
                    let _ = app.emit("plugin-popup-closed", ());
                }
                "status-menu-popup" => {
                    set_dock_menu_hold(&app, false);
                    let _ = app.emit("status-menu-popup-closed", ());
                }
                "input-lang-popup" => {
                    let _ = app.emit("input-lang-popup-closed", ());
                }
                _ => {}
            }
        }
    }

    let ssid_js = serde_json::to_string(&ssid).unwrap_or_else(|_| "\"\"".into());
    let init = format!(
        r#"
      window.__WH_IS_WIFI_AUTH_POPUP__ = true;
      window.__WH_WIFI_AUTH_SSID__ = {ssid_js};
      document.addEventListener('keydown', function (e) {{
        if (e.key === 'Escape') {{
          try {{ window.__TAURI__.core.invoke('close_wifi_auth_popup'); }} catch (_) {{}}
        }}
      }});
    "#
    );

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
        let _ = app.emit("wifi-auth-popup-opened", &ssid);
        return Ok(());
    }

    let win = WebviewWindowBuilder::new(
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
    .build()
    .map_err(|e| format!("open wifi auth popup failed: {e}"))?;

    let _ = win.set_position(LogicalPosition::new(pos_x, pos_y));
    apply_saved_material(&win, &state);
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
    }
    let _ = win.show();
    let _ = win.set_focus();
    let _ = app.emit("wifi-auth-popup-opened", &ssid);
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
    if let Some(w) = app.get_webview_window("wifi-auth-popup") {
        w.close().map_err(|e| e.to_string())?;
    }
    let _ = app.emit("wifi-auth-popup-closed", ());
    Ok(())
}

#[tauri::command]
pub fn is_wifi_auth_popup_open(app: AppHandle) -> bool {
    app.get_webview_window("wifi-auth-popup")
        .map(|w| w.is_visible().unwrap_or(false))
        .unwrap_or(false)
}

#[tauri::command]
pub fn invoke_tray_icon(
    hwnd: isize,
    callback_msg: u32,
    uid: u32,
    version: Option<u32>,
    action: Option<String>,
    id: Option<String>,
) -> Result<(), String> {
    let click = crate::win32::tray::TrayClick::parse(action.as_deref().unwrap_or("left"));
    crate::win32::tray::invoke_icon_by_id(
        id,
        hwnd,
        callback_msg,
        uid,
        version.unwrap_or(0),
        click,
    )
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
    // Allow SCM autostart to treat this as a normal handoff, not a user quit.
    #[cfg(windows)]
    crate::win32::autostart_svc::clear_user_quit();
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(exe);
    if let Ok(cwd) = std::env::current_dir() {
        cmd.current_dir(cwd);
    }
    cmd.spawn().map_err(|e| format!("restart spawn failed: {e}"))?;
    app.exit(0);
    Ok(())
}

#[tauri::command]
pub fn exit_app(app: AppHandle) {
    // Stop WindowHubAutoStart from immediately relaunching the GUI.
    #[cfg(windows)]
    crate::win32::autostart_svc::signal_user_quit();
    app.exit(0);
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
    let icons = crate::win32::tray::list_icons();
    let icon = icons
        .iter()
        .find(|i| {
            let pk = i.pin_key.trim();
            (!pk.is_empty() && pk == key) || i.id == key
        })
        .ok_or_else(|| "绑定的托盘当前不在系统托盘中".to_string())?;
    crate::win32::tray::invoke_icon_by_id(
        Some(icon.id.clone()),
        icon.hwnd,
        icon.callback_msg,
        icon.uid,
        icon.version,
        crate::win32::tray::TrayClick::Left,
    )
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
    pub msg_notify: bool,
    pub msg_notify_text: String,
    pub msg_notify_sec: u32,
    /// pluginId → presence gates (stable tray pin_key / window exe key)
    #[serde(default)]
    pub scenario_gates: std::collections::HashMap<String, ScenarioGateDto>,
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

fn normalize_island_prefs(mut p: IslandPrefsDto) -> IslandPrefsDto {
    p.immerse_idle_sec = p.immerse_idle_sec.clamp(2, 300);
    p.msg_notify_sec = p.msg_notify_sec.clamp(2, 30);
    p.pull_content = normalize_pull_content(&p.pull_content);
    p.bar_resident = normalize_bar_resident(&p.bar_resident);
    p.scenario_gates = normalize_scenario_gates(p.scenario_gates);
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
    let row = crate::db::IslandPrefsRow::from(&next);
    crate::db::with_conn(|c| crate::db::island_set(c, &row))?;
    let _ = app.emit("island-prefs", &next);
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
fn send_media_virtual_key(vk: u16) -> Result<(), String> {
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
fn send_media_virtual_key(_vk: u16) -> Result<(), String> {
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
pub struct ShortcutsPrefsDto {
    /// When set, shortcuts bar only shows this plugin entry + its pins.
    #[serde(default)]
    pub exclusive_plugin_id: Option<String>,
    /// User order of shortcuts plugin ids (Ctrl+drag). `None` = leave unchanged on patch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_order: Option<Vec<String>>,
}

impl Default for ShortcutsPrefsDto {
    fn default() -> Self {
        Self {
            exclusive_plugin_id: None,
            plugin_order: None,
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
    let val = serde_json::to_value(&next).map_err(|e| e.to_string())?;
    crate::db::with_conn(|c| crate::db::shortcuts_set(c, &val))?;
    let _ = app.emit("shortcuts-prefs", &next);
    Ok(next)
}

