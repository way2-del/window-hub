use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};
use tauri::window::Color;

use crate::ecs::components::CaptureRoi;
use crate::ecs::resources::{HubCommand, KeyKindDto, PointerKindDto};
use crate::ecs::EcsHandle;
use crate::plugin_hub::hub_init_script;
use crate::win32::enum_windows::{focus_window, parse_window_id, WindowInfo};
use crate::windows_service::WindowsService;

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

/// 弹窗/岛展开时仅置顶悬浮，不改工作区预留。
#[tauri::command]
pub fn float_overlay(window: WebviewWindow) -> Result<(), String> {
    let _ = window.set_always_on_top(true);
    let _ = window.set_skip_taskbar(true);
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
    crate::win32::topmost::force_topmost(hwnd.0 as isize);
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
) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window("settings") {
        apply_saved_material(&existing, &state);
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }

    let init = r#"
      window.__WH_IS_SETTINGS__ = true;
      document.addEventListener('keydown', function (e) {
        if (e.key === 'Escape') {
          try { window.__TAURI__.core.invoke('close_settings_window'); } catch (_) {}
        }
      });
    "#;

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

    // Show first, then apply DWM mica (HWND/WebView ready). Deferred retries cover first-open race.
    let _ = win.show();
    apply_saved_material(&win, &state);
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
const TRAY_POPUP_H: f64 = 520.0;

/// 独立窄高托盘弹窗：与设置/插件共用材质配置。
#[tauri::command]
pub async fn open_tray_popup(
    app: AppHandle,
    state: State<'_, MaterialState>,
    x: f64,
    y: f64,
) -> Result<(), String> {
    if let Some(wg) = app.get_webview_window("plugin-popup") {
        let _ = wg.close();
        let _ = app.emit("plugin-popup-closed", ());
    }
    if let Some(status) = app.get_webview_window("status-menu-popup") {
        let _ = status.close();
        let _ = app.emit("status-menu-popup-closed", ());
    }

    if let Some(existing) = app.get_webview_window("tray-popup") {
        apply_saved_material(&existing, &state);
        let _ = existing.set_size(LogicalSize::new(TRAY_POPUP_W, TRAY_POPUP_H));
        let _ = existing.set_position(LogicalPosition::new(x, y));
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_focus();
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
    .focused(true)
    .visible(false)
    .initialization_script(init)
    .build()
    .map_err(|e| format!("open tray popup failed: {e}"))?;

    let _ = win.set_position(LogicalPosition::new(x, y));
    apply_saved_material(&win, &state);
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
    }
    let _ = win.show();
    let _ = win.set_focus();
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
const STATUS_MENU_POPUP_H: f64 = 248.0;

/// 左侧状态菜单弹窗：与插件/托盘共用 MicaAlt 材质与深浅色。
#[tauri::command]
pub async fn open_status_menu_popup(
    app: AppHandle,
    state: State<'_, MaterialState>,
    x: f64,
    y: f64,
) -> Result<(), String> {
    if let Some(tray) = app.get_webview_window("tray-popup") {
        let _ = tray.close();
        let _ = app.emit("tray-popup-closed", ());
    }
    if let Some(plugin) = app.get_webview_window("plugin-popup") {
        let _ = plugin.close();
        let _ = app.emit("plugin-popup-closed", ());
    }

    if let Some(existing) = app.get_webview_window("status-menu-popup") {
        apply_saved_material(&existing, &state);
        let _ = existing.set_size(LogicalSize::new(STATUS_MENU_POPUP_W, STATUS_MENU_POPUP_H));
        let _ = existing.set_position(LogicalPosition::new(x, y));
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_focus();
        let _ = app.emit("status-menu-popup-opened", ());
        return Ok(());
    }

    let init = r#"
      window.__WH_IS_STATUS_MENU_POPUP__ = true;
      document.addEventListener('keydown', function (e) {
        if (e.key === 'Escape') {
          try { window.__TAURI__.core.invoke('close_status_menu_popup'); } catch (_) {}
        }
      });
    "#;

    let win = WebviewWindowBuilder::new(
        &app,
        "status-menu-popup",
        WebviewUrl::App("index.html?window=status-menu".into()),
    )
    .title("状态菜单")
    .inner_size(STATUS_MENU_POPUP_W, STATUS_MENU_POPUP_H)
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
    .map_err(|e| format!("open status menu popup failed: {e}"))?;

    let _ = win.set_position(LogicalPosition::new(x, y));
    apply_saved_material(&win, &state);
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
    }
    let _ = win.show();
    let _ = win.set_focus();
    let _ = app.emit("status-menu-popup-opened", ());
    Ok(())
}

#[tauri::command]
pub async fn close_status_menu_popup(app: AppHandle) -> Result<(), String> {
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
) -> Result<(), String> {
    if let Some(tray) = app.get_webview_window("tray-popup") {
        let _ = tray.close();
        let _ = app.emit("tray-popup-closed", ());
    }
    if let Some(status) = app.get_webview_window("status-menu-popup") {
        let _ = status.close();
        let _ = app.emit("status-menu-popup-closed", ());
    }

    let record = crate::plugin_install::find_installed_plugin(&plugin_id)
        .ok_or_else(|| "plugin not installed".to_string())?;
    if !record.enabled {
        return Err("plugin disabled".into());
    }
    crate::plugin_hub::assert_capability(&plugin_id, "popup")?;

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
        .inner_size(PLUGIN_POPUP_W, PLUGIN_POPUP_H)
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

pub fn apply_saved_material_pub(window: &tauri::WebviewWindow, state: &MaterialState) {
    apply_saved_material(window, state);
}

/// Re-apply materials to every open popup after prefs change.
fn reapply_material_to_popups(app: &AppHandle, prefs: &crate::win32::material::MaterialPrefs) {
    for label in [
        "settings",
        "tray-popup",
        "plugin-popup",
        "status-menu-popup",
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
) -> Result<crate::win32::tray::TrayPrefs, String> {
    let mut heights = menu_heights.unwrap_or_default();
    heights.retain(|_, h| *h > 0);
    for h in heights.values_mut() {
        *h = (*h).clamp(48, 640);
    }
    let prefs = crate::win32::tray::TrayPrefs {
        pinned,
        menu_heights: heights,
        menu_height_px: None,
    };
    crate::win32::tray::set_prefs(prefs.clone());
    save_tray_prefs(&prefs)?;
    let _ = app.emit("tray-prefs", &prefs);
    Ok(prefs)
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
    let timeout = std::time::Duration::from_millis(opts.timeout_ms.unwrap_or(15_000).clamp(1_000, 60_000));
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
    }
    .map_err(|e| format!("fetch failed: {e}"))?;

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
pub struct IslandPrefsDto {
    pub auto_immerse: bool,
    pub immerse_idle_sec: u32,
    pub pull_content: String,
    #[serde(default = "default_bar_resident_pref")]
    pub bar_resident: String,
    pub msg_notify: bool,
    pub msg_notify_text: String,
    pub msg_notify_sec: u32,
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
        }
    }
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
        }
    }
}

fn normalize_pull_content(raw: &str) -> String {
    match raw.trim() {
        "weather" => "plugin:com.window-hub.weather".into(),
        "mirror" => "plugin:com.window-hub.mirror".into(),
        s if s.starts_with("plugin:") && s.len() > "plugin:".len() => s.to_string(),
        _ => "plugin:com.window-hub.weather".into(),
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
    let next = normalize_island_prefs(prefs);
    let row = crate::db::IslandPrefsRow::from(&next);
    crate::db::with_conn(|c| crate::db::island_set(c, &row))?;
    let _ = app.emit("island-prefs", &next);
    Ok(next)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutsPrefsDto {
    /// When set, shortcuts bar only shows this plugin entry + its pins.
    #[serde(default)]
    pub exclusive_plugin_id: Option<String>,
}

impl Default for ShortcutsPrefsDto {
    fn default() -> Self {
        Self {
            exclusive_plugin_id: None,
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
    let next = normalize_shortcuts_prefs(prefs);
    let val = serde_json::to_value(&next).map_err(|e| e.to_string())?;
    crate::db::with_conn(|c| crate::db::shortcuts_set(c, &val))?;
    let _ = app.emit("shortcuts-prefs", &next);
    Ok(next)
}

