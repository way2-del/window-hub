//! Host bottom dock — MyDockFinder-style icons, visibility modes, ini import.

mod icon;
mod ini;
mod launch;
mod preview_dwm;
mod shortcut;
mod visibility;

pub use icon::resolve_item_icon_png;
pub use icon::resolve_small_icon_png;
pub use ini::parse_dockico_ini;
pub use launch::launch_or_focus;
pub use visibility::DockVisibility;

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State, WebviewUrl,
    WebviewWindowBuilder,
};

use crate::commands::{apply_saved_material_pub, MaterialState};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DockDisplayMode {
    Default,
    Layered,
    AutoHide,
    SmartHide,
    Always,
    Hotkey,
    AlwaysFullscreen,
    Desktop,
}

impl DockDisplayMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Layered => "layered",
            Self::AutoHide => "autoHide",
            Self::SmartHide => "smartHide",
            Self::Always => "always",
            Self::Hotkey => "hotkey",
            Self::AlwaysFullscreen => "alwaysFullscreen",
            Self::Desktop => "desktop",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "layered" => Self::Layered,
            "autoHide" | "auto_hide" => Self::AutoHide,
            "smartHide" | "smart_hide" => Self::SmartHide,
            "always" => Self::Always,
            "hotkey" => Self::Hotkey,
            "alwaysFullscreen" | "always_fullscreen" => Self::AlwaysFullscreen,
            "desktop" => Self::Desktop,
            _ => Self::Default,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockItem {
    pub id: String,
    /// app | separator | startmenu | trash
    pub kind: String,
    pub label: String,
    pub match_exe: String,
    pub launch_path: String,
    pub real_path: String,
    pub virtual_path: String,
    pub icon_path: String,
    pub uwp: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_png: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockPrefs {
    pub enabled: bool,
    pub display_mode: String,
    pub hide_system_taskbar: bool,
    pub items: Vec<DockItem>,
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    /// Auto-show hot zone: `screenBottom` (default, full monitor edge) or `dockBottom` (dock width only).
    #[serde(default = "default_activation_position")]
    pub activation_position: String,
    /// Logical px height of the bottom hot zone (default 12).
    #[serde(default = "default_activation_thickness_px")]
    pub activation_thickness_px: u32,
    /// Logical px gap between dock bottom and monitor bottom (default 0 = flush).
    #[serde(default)]
    pub bottom_offset_px: u32,
    /// After pointer leaves Dock/activation strip, wait this many ms before hiding.
    #[serde(default = "default_hide_linger_ms")]
    pub hide_linger_ms: u32,
    /// Max icon scale on hover (1.0 = off). Neighbors fall off symmetrically.
    #[serde(default = "default_magnification")]
    pub magnification: f64,
    /// Hover window thumbnails above running apps (MyDockFinder / macOS).
    #[serde(default = "default_true")]
    pub show_preview: bool,
    /// Floating label above the hovered icon.
    #[serde(default = "default_true")]
    pub show_hover_label: bool,
    /// Chrome corner radius in logical px (Apple pill).
    #[serde(default = "default_corner_radius")]
    pub corner_radius: u32,
    /// Bounce the icon once after launch / focus.
    #[serde(default = "default_true")]
    pub bounce_on_click: bool,
    /// Icon slot size in logical px (28–56). Chrome height follows.
    #[serde(default = "default_icon_size")]
    pub icon_size: u32,
    /// When auto/smart-hide tucks the dock, show a thin Apple-style peek strip.
    #[serde(default = "default_true")]
    pub show_trigger_strip: bool,
    /// Show unpinned running apps after a separator (macOS Dock behavior).
    #[serde(default = "default_true")]
    pub show_running_apps: bool,
    /// Running indicator: `bar` (Apple white dash) or `dot`.
    #[serde(default = "default_indicator_style")]
    pub indicator_style: String,
}

fn default_hotkey() -> String {
    "Ctrl+Alt+D".into()
}

fn default_activation_position() -> String {
    "screenBottom".into()
}

fn default_activation_thickness_px() -> u32 {
    20
}

fn default_hide_linger_ms() -> u32 {
    800
}

fn default_magnification() -> f64 {
    1.6
}

fn default_true() -> bool {
    true
}

fn default_corner_radius() -> u32 {
    16
}

fn default_icon_size() -> u32 {
    40
}

fn default_indicator_style() -> String {
    "bar".into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockActivationPosition {
    /// Full monitor bottom edge (default).
    ScreenBottom,
    /// Only the horizontal span of the dock (or where it would sit).
    DockBottom,
}

impl DockActivationPosition {
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "dockBottom" | "dock" | "dock-bottom" => Self::DockBottom,
            _ => Self::ScreenBottom,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ScreenBottom => "screenBottom",
            Self::DockBottom => "dockBottom",
        }
    }
}

impl Default for DockPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            display_mode: DockDisplayMode::Default.as_str().into(),
            hide_system_taskbar: true,
            items: Vec::new(),
            hotkey: default_hotkey(),
            activation_position: default_activation_position(),
            activation_thickness_px: default_activation_thickness_px(),
            bottom_offset_px: 0,
            hide_linger_ms: default_hide_linger_ms(),
            magnification: default_magnification(),
            show_preview: true,
            show_hover_label: false,
            corner_radius: default_corner_radius(),
            bounce_on_click: true,
            icon_size: default_icon_size(),
            show_trigger_strip: true,
            show_running_apps: true,
            indicator_style: default_indicator_style(),
        }
    }
}

impl DockPrefs {
    pub fn mode(&self) -> DockDisplayMode {
        DockDisplayMode::parse(&self.display_mode)
    }

    pub fn activation(&self) -> DockActivationPosition {
        DockActivationPosition::parse(&self.activation_position)
    }

    pub fn icon_slot(&self) -> f64 {
        self.icon_size.clamp(28, 56) as f64
    }

    /// Visible pill height (icon + padding + running dot).
    pub fn chrome_height(&self) -> f64 {
        self.icon_slot() + 16.0
    }

    fn normalize(mut self) -> Self {
        self.display_mode = self.mode().as_str().into();
        self.activation_position = self.activation().as_str().into();
        self.activation_thickness_px = self.activation_thickness_px.clamp(4, 64);
        self.bottom_offset_px = self.bottom_offset_px.min(400);
        self.hide_linger_ms = self.hide_linger_ms.clamp(200, 10_000);
        self.magnification = clamp_magnification(self.magnification);
        self.corner_radius = self.corner_radius.clamp(8, 28);
        self.icon_size = self.icon_size.clamp(28, 56);
        let style = self.indicator_style.trim().to_ascii_lowercase();
        self.indicator_style = if style == "dot" {
            "dot".into()
        } else {
            "bar".into()
        };
        self
    }
}

pub fn load_dock_prefs() -> DockPrefs {
    let raw = crate::db::with_conn(|c| crate::db::dock_get(c))
        .ok()
        .flatten();
    let Some(v) = raw else {
        return DockPrefs::default();
    };
    serde_json::from_value::<DockPrefs>(v)
        .unwrap_or_default()
        .normalize()
}

/// Fill `real_path` / correct `match_exe` for `.lnk` pins so running apps merge into pinned slots.
fn heal_dock_item_targets(prefs: &mut DockPrefs) -> bool {
    let mut changed = false;
    for item in &mut prefs.items {
        if item.kind != "app" {
            continue;
        }
        let launch = item.launch_path.clone();
        let launch_l = launch.to_ascii_lowercase();
        if item.real_path.is_empty() {
            if launch_l.ends_with(".exe") {
                item.real_path = launch.clone();
                changed = true;
            } else if launch_l.ends_with(".lnk") {
                if let Some(target) = shortcut::resolve_lnk_target(&launch) {
                    item.real_path = target;
                    changed = true;
                }
            }
        }
        if !item.real_path.is_empty() {
            let key = shortcut::normalize_exe_key(&shortcut::file_name_lower(&item.real_path));
            if !key.is_empty() && key != shortcut::normalize_exe_key(&item.match_exe) {
                item.match_exe = key;
                changed = true;
            }
        }
    }
    changed
}

pub fn save_dock_prefs(prefs: &DockPrefs) -> Result<(), String> {
    // Do not persist PNG rasters (large + would pin stale low-res forever).
    let mut stored = prefs.clone();
    for item in &mut stored.items {
        item.icon_png = None;
    }
    let v = serde_json::to_value(stored).map_err(|e| e.to_string())?;
    crate::db::with_conn(|c| crate::db::dock_set(c, &v))
}

fn with_icons(mut prefs: DockPrefs) -> DockPrefs {
    // Once per UI load: repair `.lnk` pins so running apps attach to the pinned slot.
    if heal_dock_item_targets(&mut prefs) {
        let _ = save_dock_prefs(&prefs);
    }
    for item in &mut prefs.items {
        if item.kind == "separator" {
            item.icon_png = None;
            continue;
        }
        // Always re-extract — never keep a stale low-res raster from an older Host.
        item.icon_png = icon::resolve_dock_item_icon(item);
    }
    prefs
}

const DOCK_GAP: f64 = 5.0;
const DOCK_PAD_X: f64 = 8.0;
const DOCK_SEP: f64 = 8.0;
pub(crate) const DOCK_TRIGGER_H: f64 = 3.0;
const DOCK_GLASS_LABEL: &str = "dock-glass";
const DOCK_TRIGGER_LABEL: &str = "dock-trigger";
const DOCK_ITEM_MENU_LABEL: &str = "dock-item-menu";

/// Extra transparent px above chrome (preview). Not persisted — runtime only.
fn extra_headroom() -> &'static std::sync::atomic::AtomicU32 {
    static V: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    &V
}

/// Extra content width for ephemeral running-app icons (frontend layout hint).
fn runtime_extra_width() -> &'static std::sync::atomic::AtomicU32 {
    static V: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    &V
}

fn clamp_magnification(m: f64) -> f64 {
    if m.is_finite() {
        m.clamp(1.0, 2.5)
    } else {
        default_magnification()
    }
}

/// Base content width (unscaled icon slots).
pub(crate) fn dock_content_width(items: &[DockItem], icon: f64) -> f64 {
    let mut w = DOCK_PAD_X * 2.0;
    for (i, it) in items.iter().enumerate() {
        if i > 0 {
            w += DOCK_GAP;
        }
        if it.kind == "separator" {
            w += DOCK_SEP;
        } else {
            w += icon;
        }
    }
    w.max(100.0)
}

/// Window width including fan-out room when magnification is on.
pub(crate) fn dock_window_width(prefs: &DockPrefs) -> f64 {
    let icon = prefs.icon_slot();
    let base = dock_content_width(&prefs.items, icon)
        + f64::from(runtime_extra_width().load(std::sync::atomic::Ordering::Relaxed));
    // Win10: keep fan pad modest — wide transparent frames look like a slab.
    #[cfg(windows)]
    if crate::win32::blur_glass::is_hard_safe() {
        let mag = clamp_magnification(prefs.magnification);
        let fan = if mag > 1.001 {
            icon * (mag - 1.0) * 2.0
        } else {
            0.0
        };
        return (base + fan).max(100.0);
    }
    let mag = clamp_magnification(prefs.magnification);
    let fan = icon * (mag - 1.0) * 4.0;
    (base + fan).max(100.0)
}

/// Window height = chrome + fan headroom only.
/// Preview / context menu use separate popups — never a permanent tall outlined box.
///
/// At rest (no extra_headroom): chrome-only on Win10 so no floating light band.
/// While magnifying, frontend may set extra_headroom; chrome CSS fills that band
/// (opaque), so icons can enlarge without a hollow "条子".
pub(crate) fn dock_window_height(prefs: &DockPrefs) -> f64 {
    let chrome = prefs.chrome_height();
    let extra = f64::from(extra_headroom().load(std::sync::atomic::Ordering::Relaxed));
    #[cfg(windows)]
    if crate::win32::blur_glass::is_hard_safe() {
        return chrome + extra;
    }
    let icon = prefs.icon_slot();
    let mag = clamp_magnification(prefs.magnification);
    let mut head = 0.0_f64;
    if mag > 1.001 {
        head = icon * (mag - 1.0);
    }
    head += extra;
    chrome + head
}

/// Full pill width including runtime running-app extras (glass strip).
pub(crate) fn dock_glass_width(prefs: &DockPrefs) -> f64 {
    dock_content_width(&prefs.items, prefs.icon_slot())
        + f64::from(runtime_extra_width().load(std::sync::atomic::Ordering::Relaxed))
}

fn dock_place_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

pub fn position_dock_window(app: &AppHandle, prefs: &DockPrefs) {
    if let Some(vis) = app.try_state::<Arc<DockVisibility>>() {
        if vis.is_busy() {
            return;
        }
    }
    let shown = app
        .try_state::<Arc<DockVisibility>>()
        .map(|v| v.ui_shown())
        .unwrap_or(true);
    place_dock_window(app, prefs, shown, false);
}

/// Move the **entire outer HWND** (glass + WebView content as one unit).
///
/// - Rest: bottom edge = monitor.bottom - offset
/// - Hidden: fully below the screen, then `SW_HIDE`
/// - Animate: `SetWindowPos` Y tween — never cancelled mid-slide
///
/// Material lives on sibling `dock-glass` (chrome height); icons window may be
/// taller for magnification headroom only.
///
/// **Lock rule:** `dock_place_lock` may only wrap pure Win32 geom. Any Tauri
/// window API (`hide`/`show`/`set_ignore_cursor_events`/`build`/…) marshals to
/// the UI thread and will deadlock if called while this lock is held.
pub fn place_dock_window(app: &AppHandle, prefs: &DockPrefs, shown: bool, animate: bool) {
    let Some(win) = app.get_webview_window("dock") else {
        return;
    };
    let width = dock_window_width(prefs);
    let height = dock_window_height(prefs);
    let chrome_h = prefs.chrome_height();
    // Glass must match the CSS pill (content width) — NOT the icons HWND fan width,
    // or a rectangular material slab peeks past the rounded chrome.
    let glass_w = dock_glass_width(prefs);
    let glass = app.get_webview_window(DOCK_GLASS_LABEL);

    #[cfg(windows)]
    let glass_disabled = crate::win32::blur_glass::is_hard_safe();
    #[cfg(not(windows))]
    let glass_disabled = false;

    // Win10: never sleep on a place path that can contend with UI/WebView2 init —
    // animated slides previously painted the whole app as "未响应".
    #[cfg(windows)]
    let animate = animate && !glass_disabled;
    #[cfg(not(windows))]
    let animate = animate;

    // --- Hit-test prep: prefer Win32 (Tauri set_ignore_cursor_events can self-deadlock
    // when place runs on / blocks the UI thread). ---
    #[cfg(windows)]
    {
        let dock_hwnd = win.hwnd().ok().map(|h| h.0 as isize);
        let glass_hwnd_raw = glass.as_ref().and_then(|g| g.hwnd().ok()).map(|h| h.0 as isize);

        if let Some(gh) = glass_hwnd_raw {
            // Glass never receives hits.
            win32_dock_set_click_through(gh, true);
            win32_dock_clear_frame(gh);
        }

        let glass_hwnd = if glass_disabled {
            if let Some(gh) = glass_hwnd_raw {
                win32_dock_show(gh, false);
            }
            None
        } else {
            glass_hwnd_raw
        };

        if let Some(hwnd) = dock_hwnd {
            if shown {
                win32_dock_set_click_through(hwnd, false);
                win32_dock_clear_frame(hwnd);
            } else {
                win32_dock_set_click_through(hwnd, true);
            }

            // Snap holds the place lock; animated slides run unlocked (busy serializes).
            let slid = if animate {
                win32_dock_slide_root(
                    hwnd,
                    glass_hwnd,
                    width,
                    height,
                    glass_w,
                    chrome_h,
                    prefs.bottom_offset_px,
                    shown,
                    true,
                )
            } else {
                let _guard = dock_place_lock().lock().unwrap_or_else(|e| e.into_inner());
                win32_dock_slide_root(
                    hwnd,
                    glass_hwnd,
                    width,
                    height,
                    glass_w,
                    chrome_h,
                    prefs.bottom_offset_px,
                    shown,
                    false,
                )
            };

            if slid {
                if shown {
                    win32_dock_set_click_through(hwnd, false);
                    win32_dock_clear_frame(hwnd);
                    if let Some(gh) = glass_hwnd {
                        win32_dock_clear_frame(gh);
                    }
                }
                // Trigger strip uses Tauri build — must stay outside the place lock.
                sync_trigger_strip(app, prefs, shown);
                return;
            }
        }
    }

    #[cfg(not(windows))]
    {
        let _ = win.set_ignore_cursor_events(!shown);
        if let Some(g) = &glass {
            let _ = g.set_ignore_cursor_events(true);
        }
    }

    // Non-Windows / Win32 geom failure: Tauri fallback (still outside place lock).
    let _ = win.set_size(LogicalSize::new(width, height));
    if !glass_disabled {
        if let Some(g) = &glass {
            let _ = g.set_size(LogicalSize::new(glass_w, chrome_h));
        }
    }

    let Ok(Some(monitor)) = win.current_monitor() else {
        #[cfg(not(windows))]
        let _ = win.set_ignore_cursor_events(!shown);
        sync_trigger_strip(app, prefs, shown);
        return;
    };
    let scale = monitor.scale_factor();
    let screen = monitor.size();
    let origin = monitor.position();
    let phys_w = (width * scale).round().max(1.0) as i32;
    let phys_h = (height * scale).round().max(1.0) as i32;
    let glass_phys_w = (glass_w * scale).round().max(1.0) as i32;
    let glass_h = (chrome_h * scale).round().max(1.0) as i32;
    let margin = (prefs.bottom_offset_px as f64 * scale).round() as i32;
    let x = origin.x + ((screen.width as i32 - phys_w) / 2).max(0);
    let y_shown = origin.y + (screen.height as i32 - phys_h - margin).max(0);
    let y = if shown {
        y_shown
    } else {
        y_shown + phys_h + 4
    };
    let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
    if !glass_disabled {
        if let Some(g) = &glass {
            let gx = x + ((phys_w - glass_phys_w) / 2).max(0);
            let gy = y + phys_h - glass_h;
            let _ = g.set_position(tauri::PhysicalPosition::new(gx, gy));
            #[cfg(windows)]
            if let Ok(hwnd) = g.hwnd() {
                win32_dock_show(hwnd.0 as isize, shown);
            }
            #[cfg(not(windows))]
            {
                if shown {
                    let _ = g.show();
                } else {
                    let _ = g.hide();
                }
            }
        }
    }
    #[cfg(windows)]
    if let Ok(hwnd) = win.hwnd() {
        win32_dock_show(hwnd.0 as isize, shown);
        win32_dock_set_click_through(hwnd.0 as isize, !shown);
    }
    #[cfg(not(windows))]
    {
        if shown {
            let _ = win.show();
        } else {
            let _ = win.hide();
        }
        let _ = win.set_ignore_cursor_events(!shown);
    }
    sync_trigger_strip(app, prefs, shown);
}

/// Resolve Tauri/WebView HWND → outer top-level window (never SetWindowPos the child).
#[cfg(windows)]
fn dock_root_hwnd(hwnd_raw: isize) -> windows::Win32::Foundation::HWND {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GA_ROOT};
    unsafe {
        let h = HWND(hwnd_raw as _);
        let root = GetAncestor(h, GA_ROOT);
        if root.0.is_null() {
            h
        } else {
            root
        }
    }
}

/// Ensure the dock can be hit-tested (auto-hide keep-alive + clicks).
#[cfg(windows)]
#[allow(dead_code)]
fn win32_dock_clear_transparent(hwnd_raw: isize) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, SetWindowLongW, SetWindowPos, GWL_EXSTYLE, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_EX_TRANSPARENT,
    };
    let hwnd = dock_root_hwnd(hwnd_raw);
    unsafe {
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE);
        if ex & WS_EX_TRANSPARENT.0 as i32 != 0 {
            let _ = SetWindowLongW(hwnd, GWL_EXSTYLE, ex & !(WS_EX_TRANSPARENT.0 as i32));
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }
}

/// Show/hide without Tauri — avoids event-loop re-entry under `dock_place_lock`.
#[cfg(windows)]
fn win32_dock_show(hwnd_raw: isize, shown: bool) {
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE, SW_SHOWNOACTIVATE};
    let hwnd = dock_root_hwnd(hwnd_raw);
    unsafe {
        let _ = ShowWindow(hwnd, if shown { SW_SHOWNOACTIVATE } else { SW_HIDE });
    }
}

/// Toggle WS_EX_TRANSPARENT (click-through) without Tauri marshal.
#[cfg(windows)]
fn win32_dock_set_click_through(hwnd_raw: isize, through: bool) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, SetWindowLongW, SetWindowPos, GWL_EXSTYLE, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_EX_TRANSPARENT,
    };
    let hwnd = dock_root_hwnd(hwnd_raw);
    unsafe {
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE);
        let flag = WS_EX_TRANSPARENT.0 as i32;
        let next = if through { ex | flag } else { ex & !flag };
        if next != ex {
            let _ = SetWindowLongW(hwnd, GWL_EXSTYLE, next);
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }
}

/// Kill the rectangular DWM / classic frame outline around dock HWNDs.
/// Also strips caption so focus never reveals a "Dock" title bar / blue accent ring.
/// Sets WS_EX_NOACTIVATE so clicks don't steal activation (blue focus chrome + flaky launch).
#[cfg(windows)]
pub fn reclear_dock_frame(hwnd_raw: isize) {
    win32_dock_clear_frame(hwnd_raw);
}

#[cfg(not(windows))]
pub fn reclear_dock_frame(_hwnd_raw: isize) {}

/// Kill the rectangular DWM / classic frame outline around dock HWNDs.
/// Also strips caption so focus never reveals a "Dock" title bar / blue accent ring.
/// Sets WS_EX_NOACTIVATE so clicks don't steal activation (blue focus chrome + flaky launch).
#[cfg(windows)]
fn win32_dock_clear_frame(hwnd_raw: isize) {
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_COLOR_NONE,
        DWMWA_TEXT_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
        DWM_WINDOW_CORNER_PREFERENCE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, SetWindowLongW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_BORDER, WS_CAPTION,
        WS_DLGFRAME, WS_EX_NOACTIVATE, WS_SYSMENU, WS_THICKFRAME,
    };
    use std::ffi::c_void;
    let hwnd = dock_root_hwnd(hwnd_raw);
    unsafe {
        let style = GetWindowLongW(hwnd, GWL_STYLE);
        let strip = (WS_BORDER.0 | WS_DLGFRAME.0 | WS_THICKFRAME.0 | WS_CAPTION.0 | WS_SYSMENU.0)
            as i32;
        if style & strip != 0 {
            let _ = SetWindowLongW(hwnd, GWL_STYLE, style & !strip);
        }
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE);
        let ex_next = ex | (WS_EX_NOACTIVATE.0 as i32);
        if ex_next != ex {
            let _ = SetWindowLongW(hwnd, GWL_EXSTYLE, ex_next);
        }
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
        let none = DWMWA_COLOR_NONE;
        for attr in [DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR] {
            let _ = DwmSetWindowAttribute(
                hwnd,
                attr,
                &none as *const u32 as *const c_void,
                std::mem::size_of::<u32>() as u32,
            );
        }
        let corner = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const DWM_WINDOW_CORNER_PREFERENCE as *const c_void,
            std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
    }
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct DockGeom {
    x: i32,
    y_shown: i32,
    y_hidden: i32,
    w: i32,
    h: i32,
}

#[cfg(windows)]
fn win32_dock_geom(
    hwnd_raw: isize,
    logical_w: f64,
    logical_h: f64,
    bottom_offset_px: u32,
) -> Option<DockGeom> {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

    let hwnd = dock_root_hwnd(hwnd_raw);
    unsafe {
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return None;
        }
        let mut dpi_x = 96u32;
        let mut dpi_y = 96u32;
        if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_err() {
            dpi_x = 96;
        }
        let scale = if dpi_x > 0 {
            dpi_x as f64 / 96.0
        } else {
            1.0
        };
        let w = (logical_w * scale).round().max(1.0) as i32;
        let h = (logical_h * scale).round().max(1.0) as i32;
        let margin = (bottom_offset_px as f64 * scale).round() as i32;
        let rc = mi.rcMonitor;
        let want_bottom = rc.bottom - margin;
        let x = rc.left + ((rc.right - rc.left - w) / 2).max(0);
        let y_shown = want_bottom - h;
        // Park fully below the visible edge (top past the bottom by a full height).
        let y_hidden = want_bottom + h + 8;
        Some(DockGeom {
            x,
            y_shown,
            y_hidden,
            w,
            h,
        })
    }
}

/// Slide the **root** HWND as one unit from below the screen ↔ bottom rest pose.
///
/// Show path (always):
///   1. Park at `y_hidden` while still not shown (or re-park)
///   2. `SW_SHOWNOACTIVATE`
///   3. Ease-out tween → `y_shown`
///   4. Snap rest + re-assert show
///
/// Hide path:
///   1. Ensure visible at current Y (clamped to [y_shown, y_hidden])
///   2. Ease-in tween → `y_hidden`
///   3. Snap + `SW_HIDE`
///
/// Returns `true` on success. Never cancelled mid-slide (caller holds place lock).
///
/// When `glass_hwnd_raw` is set, the glass strip stays bottom-aligned with the
/// icons window for the whole tween (material without frosted headroom).
#[cfg(windows)]
fn win32_dock_slide_root(
    hwnd_raw: isize,
    glass_hwnd_raw: Option<isize>,
    logical_w: f64,
    logical_h: f64,
    glass_logical_w: f64,
    chrome_h: f64,
    bottom_offset_px: u32,
    shown: bool,
    animate: bool,
) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE, SW_SHOWNOACTIVATE};

    let Some(geom) = win32_dock_geom(hwnd_raw, logical_w, logical_h, bottom_offset_px) else {
        return false;
    };
    let hwnd = dock_root_hwnd(hwnd_raw);
    let glass = glass_hwnd_raw.map(dock_root_hwnd);
    let scale = if geom.h > 0 && logical_h > 0.0 {
        geom.h as f64 / logical_h
    } else {
        1.0
    };
    let glass_h = (chrome_h * scale).round().max(1.0) as i32;
    let glass_w = (glass_logical_w * scale).round().max(1.0) as i32;

    unsafe fn set_pair_pos(
        hwnd: windows::Win32::Foundation::HWND,
        glass: Option<windows::Win32::Foundation::HWND>,
        geom: DockGeom,
        y: i32,
        glass_w: i32,
        glass_h: i32,
    ) {
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER,
        };
        let _ = SetWindowPos(
            hwnd,
            None,
            geom.x,
            y,
            geom.w,
            geom.h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        if let Some(gh) = glass {
            let gx = geom.x + ((geom.w - glass_w) / 2).max(0);
            let gy = y + geom.h - glass_h;
            let _ = SetWindowPos(
                gh,
                None,
                gx,
                gy,
                glass_w,
                glass_h,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            // Keep glass under the icons layer (hWndInsertAfter = icons ⇒ glass below).
            let _ = SetWindowPos(
                gh,
                hwnd,
                0,
                0,
                0,
                0,
                windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                    | windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE
                    | windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
            );
        }
    }

    unsafe fn set_pair_y(
        hwnd: windows::Win32::Foundation::HWND,
        glass: Option<windows::Win32::Foundation::HWND>,
        geom: DockGeom,
        y: i32,
        glass_w: i32,
        glass_h: i32,
    ) {
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
        };
        let _ = SetWindowPos(
            hwnd,
            None,
            geom.x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        if let Some(gh) = glass {
            let gx = geom.x + ((geom.w - glass_w) / 2).max(0);
            let gy = y + geom.h - glass_h;
            let _ = SetWindowPos(
                gh,
                None,
                gx,
                gy,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    unsafe fn read_y(hwnd: windows::Win32::Foundation::HWND) -> Option<i32> {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
        let mut wr = RECT::default();
        if GetWindowRect(hwnd, &mut wr).is_ok() {
            Some(wr.top)
        } else {
            None
        }
    }

    /// Show: ease-out-quint with slight overshoot settle. Hide: ease-in-cubic.
    fn ease(t: f64, show: bool) -> f64 {
        let t = t.clamp(0.0, 1.0);
        if show {
            // Soft landing — fast leave from below, gentle settle at rest.
            1.0 - (1.0 - t).powi(4)
        } else {
            // Accelerate into the hide so the tuck feels intentional.
            t * t * t * t
        }
    }

    unsafe fn tween_y(
        hwnd: windows::Win32::Foundation::HWND,
        glass: Option<windows::Win32::Foundation::HWND>,
        geom: DockGeom,
        y_from: i32,
        y_to: i32,
        rising: bool,
        glass_w: i32,
        glass_h: i32,
    ) {
        if (y_from - y_to).abs() <= 1 {
            set_pair_pos(hwnd, glass, geom, y_to, glass_w, glass_h);
            return;
        }
        // ~280ms — smoother Apple-like tuck without feeling sluggish.
        const FRAMES: u32 = 24;
        const FRAME_MS: u64 = 12;
        for i in 1..=FRAMES {
            let t = i as f64 / FRAMES as f64;
            let e = ease(t, rising);
            let y = (y_from as f64 + (y_to as f64 - y_from as f64) * e).round() as i32;
            set_pair_y(hwnd, glass, geom, y, glass_w, glass_h);
            std::thread::sleep(std::time::Duration::from_millis(FRAME_MS));
        }
        set_pair_pos(hwnd, glass, geom, y_to, glass_w, glass_h);
    }

    unsafe {
        if !animate {
            set_pair_pos(
                hwnd,
                glass,
                geom,
                if shown { geom.y_shown } else { geom.y_hidden },
                glass_w,
                glass_h,
            );
            let cmd = if shown {
                SW_SHOWNOACTIVATE
            } else {
                SW_HIDE
            };
            let _ = ShowWindow(hwnd, cmd);
            if let Some(gh) = glass {
                let _ = ShowWindow(gh, cmd);
            }
            eprintln!(
                "[dock-place] snap shown={} y={}",
                u8::from(shown),
                if shown { geom.y_shown } else { geom.y_hidden }
            );
            return true;
        }

        if shown {
            // Enable hit-testing as soon as the window is shown for the rise —
            // waiting until tween end left WindowFromPoint blind and false-leaved.
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            if let Some(gh) = glass {
                let _ = ShowWindow(gh, SW_SHOWNOACTIVATE);
            }
            set_pair_pos(hwnd, glass, geom, geom.y_hidden, glass_w, glass_h);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            if let Some(gh) = glass {
                let _ = ShowWindow(gh, SW_SHOWNOACTIVATE);
            }
            tween_y(
                hwnd,
                glass,
                geom,
                geom.y_hidden,
                geom.y_shown,
                true,
                glass_w,
                glass_h,
            );
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            if let Some(gh) = glass {
                let _ = ShowWindow(gh, SW_SHOWNOACTIVATE);
            }
            eprintln!(
                "[dock-place] show anim y {}→{} (now={:?})",
                geom.y_hidden,
                geom.y_shown,
                read_y(hwnd)
            );
        } else {
            let y_now = read_y(hwnd).unwrap_or(geom.y_shown);
            let y_from = y_now.clamp(geom.y_shown, geom.y_hidden);
            // Must be visible to tween out; if already tucked, just hide.
            if (y_from - geom.y_hidden).abs() <= 2 {
                set_pair_pos(hwnd, glass, geom, geom.y_hidden, glass_w, glass_h);
                let _ = ShowWindow(hwnd, SW_HIDE);
                if let Some(gh) = glass {
                    let _ = ShowWindow(gh, SW_HIDE);
                }
            } else {
                set_pair_pos(hwnd, glass, geom, y_from, glass_w, glass_h);
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                if let Some(gh) = glass {
                    let _ = ShowWindow(gh, SW_SHOWNOACTIVATE);
                }
                tween_y(
                    hwnd,
                    glass,
                    geom,
                    y_from,
                    geom.y_hidden,
                    false,
                    glass_w,
                    glass_h,
                );
                let _ = ShowWindow(hwnd, SW_HIDE);
                if let Some(gh) = glass {
                    let _ = ShowWindow(gh, SW_HIDE);
                }
            }
            eprintln!(
                "[dock-place] hide anim y {}→{} (now={:?})",
                y_from,
                geom.y_hidden,
                read_y(hwnd)
            );
        }
        true
    }
}

#[tauri::command]
pub async fn get_dock_prefs() -> DockPrefs {
    // Shell icon extract (256px) is slow on Win10 — never run it on the UI/async
    // worker inline or the whole app paints as "未响应".
    match tauri::async_runtime::spawn_blocking(|| with_icons(load_dock_prefs())).await {
        Ok(prefs) => prefs,
        Err(_) => load_dock_prefs(),
    }
}

#[tauri::command]
pub async fn set_dock_prefs(
    app: AppHandle,
    state: State<'_, MaterialState>,
    vis: State<'_, Arc<DockVisibility>>,
    prefs: DockPrefs,
) -> Result<DockPrefs, String> {
    let mut next = prefs.normalize();
    // Dock on ⇒ system taskbar stays hidden (product rule).
    if next.enabled {
        next.hide_system_taskbar = true;
    }
    next = with_icons(next);
    save_dock_prefs(&next)?;
    vis.apply_prefs(&next);
    let _ = app.emit("dock-prefs", &next);

    if next.enabled {
        ensure_dock_window_inner(&app, &state, &vis, &next).await?;
        apply_taskbar_for_dock(true);
        // Snap once only — looping reposition races auto-hide slides and causes flash-hide.
        if !matches!(
            next.mode(),
            DockDisplayMode::AutoHide | DockDisplayMode::SmartHide
        ) {
            position_dock_window(&app, &next);
        } else {
            // Still size/place to current policy without fighting the vis thread later.
            let vis_shown = vis.ui_shown();
            place_dock_window(&app, &next, vis_shown, false);
        }
        if let Some(w) = app.get_webview_window("dock") {
            let top = !matches!(next.mode(), DockDisplayMode::Hotkey | DockDisplayMode::Desktop);
            let _ = w.set_always_on_top(top);
        }
        if let Some(g) = app.get_webview_window(DOCK_GLASS_LABEL) {
            let top = !matches!(next.mode(), DockDisplayMode::Hotkey | DockDisplayMode::Desktop);
            let _ = g.set_always_on_top(top);
            let _ = g.set_ignore_cursor_events(true);
        }
    } else {
        vis.stop();
        if let Some(w) = app.get_webview_window("dock") {
            let _ = w.close();
        }
        if let Some(g) = app.get_webview_window(DOCK_GLASS_LABEL) {
            let _ = g.close();
        }
        if let Some(t) = app.get_webview_window(DOCK_TRIGGER_LABEL) {
            let _ = t.close();
        }
        if let Some(m) = app.get_webview_window(DOCK_ITEM_MENU_LABEL) {
            let _ = m.close();
        }
        if let Some(p) = app.get_webview_window(DOCK_PREVIEW_LABEL) {
            let _ = p.close();
        }
        apply_taskbar_for_dock(false);
    }
    Ok(next)
}

fn apply_taskbar_for_dock(hide: bool) {
    let _ = crate::commands::set_system_taskbar_visible(!hide);
}

#[tauri::command]
pub fn import_dockico_ini(app: AppHandle, path: String) -> Result<DockPrefs, String> {
    let items = parse_dockico_ini(std::path::Path::new(path.trim()))?;
    let mut prefs = load_dock_prefs();
    prefs.items = items;
    prefs = with_icons(prefs);
    save_dock_prefs(&prefs)?;
    let _ = app.emit("dock-prefs", &prefs);
    position_dock_window(&app, &prefs);
    Ok(prefs)
}

#[tauri::command]
pub fn pick_dockico_file() -> Result<Option<String>, String> {
    let file = rfd::FileDialog::new()
        .add_filter("MyDockFinder Dock", &["ini"])
        .set_title("导入 MyDockFinder .dockico.ini")
        .pick_file();
    Ok(file.map(|p| p.to_string_lossy().to_string()))
}

#[tauri::command]
pub fn dock_launch_item(item_id: String) -> Result<(), String> {
    let prefs = load_dock_prefs();
    let item = prefs
        .items
        .iter()
        .find(|i| i.id == item_id)
        .ok_or_else(|| format!("dock item not found: {item_id}"))?;
    launch_or_focus(item)
}

fn persist_and_emit(app: &AppHandle, mut prefs: DockPrefs) -> Result<DockPrefs, String> {
    prefs = prefs.normalize();
    prefs = with_icons(prefs);
    save_dock_prefs(&prefs)?;
    let _ = app.emit("dock-prefs", &prefs);
    position_dock_window(app, &prefs);
    Ok(prefs)
}

fn new_item_id(prefix: &str) -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{prefix}-{ms}")
}

fn dock_item_from_path(path: &str) -> Result<DockItem, String> {
    use std::path::Path;
    let path = path.trim();
    if path.is_empty() {
        return Err("empty path".into());
    }
    let p = Path::new(path);
    if !p.exists() {
        return Err(format!("file not found: {path}"));
    }
    let stem = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("App")
        .to_string();
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    // Prefer real target exe so pinned slots merge with running apps (`.lnk` stem ≠ process name).
    let (match_exe, real) = if ext == "exe" {
        let name = p
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        (name, path.to_string())
    } else if ext == "lnk" {
        if let Some(target) = shortcut::resolve_lnk_target(path) {
            let name = shortcut::normalize_exe_key(&shortcut::file_name_lower(&target));
            let name = if name.is_empty() {
                format!("{}.exe", stem.to_ascii_lowercase())
            } else {
                name
            };
            (name, target)
        } else {
            (
                format!("{}.exe", stem.to_ascii_lowercase()),
                String::new(),
            )
        }
    } else {
        (
            format!("{}.exe", stem.to_ascii_lowercase()),
            String::new(),
        )
    };
    Ok(DockItem {
        id: new_item_id("app"),
        kind: "app".into(),
        label: stem,
        match_exe,
        launch_path: path.to_string(),
        real_path: real,
        virtual_path: String::new(),
        icon_path: path.to_string(),
        uwp: false,
        icon_png: None,
    })
}

#[tauri::command]
pub fn pick_dock_app_file() -> Result<Option<String>, String> {
    let file = rfd::FileDialog::new()
        .add_filter("应用程序", &["exe", "lnk"])
        .add_filter("全部", &["*"])
        .set_title("添加到 Dock")
        .pick_file();
    Ok(file.map(|p| p.to_string_lossy().to_string()))
}

#[tauri::command]
pub fn dock_add_app(
    app: AppHandle,
    path: String,
    after_id: Option<String>,
) -> Result<DockPrefs, String> {
    let item = dock_item_from_path(&path)?;
    let mut prefs = load_dock_prefs();
    if let Some(aid) = after_id.as_deref().filter(|s| !s.is_empty()) {
        if let Some(idx) = prefs.items.iter().position(|i| i.id == aid) {
            prefs.items.insert(idx + 1, item);
        } else {
            prefs.items.push(item);
        }
    } else {
        prefs.items.push(item);
    }
    persist_and_emit(&app, prefs)
}

#[tauri::command]
pub fn dock_add_separator(
    app: AppHandle,
    after_id: Option<String>,
) -> Result<DockPrefs, String> {
    let item = DockItem {
        id: new_item_id("sep"),
        kind: "separator".into(),
        label: String::new(),
        match_exe: String::new(),
        launch_path: String::new(),
        real_path: String::new(),
        virtual_path: String::new(),
        icon_path: String::new(),
        uwp: false,
        icon_png: None,
    };
    let mut prefs = load_dock_prefs();
    if let Some(aid) = after_id.as_deref().filter(|s| !s.is_empty()) {
        if let Some(idx) = prefs.items.iter().position(|i| i.id == aid) {
            prefs.items.insert(idx + 1, item);
        } else {
            prefs.items.push(item);
        }
    } else {
        prefs.items.push(item);
    }
    persist_and_emit(&app, prefs)
}

#[tauri::command]
pub fn dock_remove_item(app: AppHandle, item_id: String) -> Result<DockPrefs, String> {
    let mut prefs = load_dock_prefs();
    let before = prefs.items.len();
    prefs.items.retain(|i| i.id != item_id);
    if prefs.items.len() == before {
        return Err(format!("dock item not found: {item_id}"));
    }
    persist_and_emit(&app, prefs)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockWindowLite {
    pub id: String,
    pub hwnd: isize,
    pub title: String,
}

#[tauri::command]
pub fn dock_list_item_windows(item_id: String) -> Result<Vec<DockWindowLite>, String> {
    let prefs = load_dock_prefs();
    let item = prefs
        .items
        .iter()
        .find(|i| i.id == item_id)
        .ok_or_else(|| format!("dock item not found: {item_id}"))?;
    let wins = crate::win32::enum_windows::list_windows(None);
    Ok(launch::matching_windows(item, &wins)
        .into_iter()
        .map(|w| DockWindowLite {
            id: w.id,
            hwnd: w.hwnd,
            title: w.title,
        })
        .collect())
}

#[tauri::command]
pub fn dock_close_item_windows(item_id: String) -> Result<u32, String> {
    let prefs = load_dock_prefs();
    let item = prefs
        .items
        .iter()
        .find(|i| i.id == item_id)
        .ok_or_else(|| format!("dock item not found: {item_id}"))?;
    let wins = crate::win32::enum_windows::list_windows(None);
    let matched = launch::matching_windows(item, &wins);
    let mut n = 0u32;
    for w in matched {
        if close_hwnd(w.hwnd) {
            n += 1;
        }
    }
    Ok(n)
}

fn close_hwnd(hwnd_raw: isize) -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
        let hwnd = HWND(hwnd_raw as _);
        unsafe { PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok() }
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd_raw;
        false
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockPreviewFrame {
    pub hwnd: isize,
    pub title: String,
    pub jpeg_base64: String,
    pub width: u32,
    pub height: u32,
}

fn capture_previews_for_match(
    match_exe: &str,
    real_path: &str,
) -> Result<Vec<DockPreviewFrame>, String> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    let prefs = load_dock_prefs();
    if !prefs.show_preview {
        return Ok(Vec::new());
    }
    let probe = DockItem {
        id: String::new(),
        kind: "app".into(),
        label: String::new(),
        match_exe: match_exe.to_string(),
        launch_path: real_path.to_string(),
        real_path: real_path.to_string(),
        virtual_path: String::new(),
        icon_path: String::new(),
        uwp: false,
        icon_png: None,
    };
    let wins = crate::win32::enum_windows::list_windows(None);
    let matched = launch::matching_windows(&probe, &wins);
    let mut out = Vec::new();
    for w in matched.into_iter().take(6) {
        // Fast path: small edge + cache; skip topmost peek (Win10 taskbar uses DWM — we approximate).
        match crate::win32::capture::capture_window_jpeg_cached(w.hwnd, 240, false) {
            Ok(frame) => {
                out.push(DockPreviewFrame {
                    hwnd: w.hwnd,
                    title: w.title,
                    jpeg_base64: B64.encode(&frame.jpeg),
                    width: frame.width,
                    height: frame.height,
                });
            }
            Err(e) => {
                eprintln!("[dock] preview capture failed hwnd={}: {e}", w.hwnd);
                out.push(DockPreviewFrame {
                    hwnd: w.hwnd,
                    title: w.title,
                    jpeg_base64: String::new(),
                    width: 0,
                    height: 0,
                });
            }
        }
    }
    Ok(out)
}

/// Window list only (no capture) — used to open the preview popup instantly.
fn list_preview_targets(match_exe: &str, real_path: &str) -> Vec<DockPreviewFrame> {
    let probe = DockItem {
        id: String::new(),
        kind: "app".into(),
        label: String::new(),
        match_exe: match_exe.to_string(),
        launch_path: real_path.to_string(),
        real_path: real_path.to_string(),
        virtual_path: String::new(),
        icon_path: String::new(),
        uwp: false,
        icon_png: None,
    };
    let wins = crate::win32::enum_windows::list_windows(None);
    launch::matching_windows(&probe, &wins)
        .into_iter()
        .take(6)
        .map(|w| DockPreviewFrame {
            hwnd: w.hwnd,
            title: w.title,
            jpeg_base64: String::new(),
            width: 0,
            height: 0,
        })
        .collect()
}

fn dock_capture_item_previews_sync(item_id: String) -> Result<Vec<DockPreviewFrame>, String> {
    let prefs = load_dock_prefs();
    if let Some(item) = prefs.items.iter().find(|i| i.id == item_id) {
        return capture_previews_for_match(&item.match_exe, &item.real_path);
    }
    // Ephemeral running:* items from the frontend.
    if let Some(rest) = item_id.strip_prefix("running:") {
        let match_exe = rest.to_string();
        let real = prefs
            .items
            .iter()
            .find(|i| i.match_exe.eq_ignore_ascii_case(&match_exe))
            .map(|i| i.real_path.clone())
            .unwrap_or_default();
        return capture_previews_for_match(&match_exe, &real);
    }
    Err(format!("dock item not found: {item_id}"))
}

const DOCK_PREVIEW_LABEL: &str = "dock-preview";
const DOCK_PREVIEW_PAD: f64 = 10.0;
const DOCK_PREVIEW_GAP: f64 = 8.0;
const DOCK_PREVIEW_CARD_W: f64 = 168.0;
const DOCK_PREVIEW_CARD_H: f64 = 118.0; // thumb ~100 + title strip

fn apply_preview_live_thumbs(app: &AppHandle, frames: &[DockPreviewFrame]) -> usize {
    #[cfg(windows)]
    {
        let Some(win) = app.get_webview_window(DOCK_PREVIEW_LABEL) else {
            return 0;
        };
        let Ok(hwnd) = win.hwnd() else {
            return 0;
        };
        let scale = win
            .current_monitor()
            .ok()
            .flatten()
            .map(|m| m.scale_factor())
            .unwrap_or(1.0);
        let sources: Vec<isize> = frames.iter().map(|f| f.hwnd).collect();
        preview_dwm::set_thumbnails(hwnd.0 as isize, &sources, scale)
    }
    #[cfg(not(windows))]
    {
        let _ = (app, frames);
        0
    }
}

/// Bumped on every close / open start — drops stale `open_dock_preview` that finish after click.
fn preview_epoch() -> &'static std::sync::atomic::AtomicU64 {
    static E: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    &E
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockPreviewPayload {
    pub item_id: String,
    pub label: String,
    pub frames: Vec<DockPreviewFrame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_png: Option<String>,
}

fn preview_window_size(n: usize) -> (f64, f64) {
    let count = n.max(1).min(6) as f64;
    let w = DOCK_PREVIEW_PAD * 2.0 + count * DOCK_PREVIEW_CARD_W + (count - 1.0).max(0.0) * DOCK_PREVIEW_GAP;
    let h = DOCK_PREVIEW_PAD * 2.0 + DOCK_PREVIEW_CARD_H;
    (w.clamp(160.0, 920.0), h.clamp(120.0, 160.0))
}

/// Keep the preview popup on the same monitor as the dock, clamped to edges.
fn clamp_preview_pos(app: &AppHandle, x: f64, y: f64, menu_w: f64, menu_h: f64) -> (f64, f64) {
    const MARGIN: f64 = 4.0;
    let mon = app
        .get_webview_window("dock")
        .and_then(|w| w.current_monitor().ok().flatten())
        .or_else(|| {
            app.get_webview_window(DOCK_PREVIEW_LABEL)
                .and_then(|w| w.current_monitor().ok().flatten())
        });
    let Some(monitor) = mon else {
        return (x.max(MARGIN), y.max(MARGIN));
    };
    let scale = monitor.scale_factor();
    let origin = monitor.position();
    let size = monitor.size();
    let mx = f64::from(origin.x) / scale;
    let my = f64::from(origin.y) / scale;
    let mw = f64::from(size.width) / scale;
    let mh = f64::from(size.height) / scale;
    let max_x = (mx + mw - menu_w - MARGIN).max(mx + MARGIN);
    let max_y = (my + mh - menu_h - MARGIN).max(my + MARGIN);
    (
        x.clamp(mx + MARGIN, max_x),
        y.clamp(my + MARGIN, max_y),
    )
}

#[tauri::command]
pub async fn open_dock_preview(
    app: AppHandle,
    state: State<'_, MaterialState>,
    item_id: String,
    anchor_x: f64,
    anchor_y: f64,
    match_exe: Option<String>,
    real_path: Option<String>,
    label: Option<String>,
    icon_png: Option<String>,
) -> Result<(), String> {
    // Fast: no with_icons — that re-extracted every dock icon and made preview feel stuck.
    let prefs = load_dock_prefs();
    if !prefs.show_preview {
        return Ok(());
    }

    let pinned = prefs.items.iter().find(|i| i.id == item_id).cloned();
    let (match_exe, real_path, label, icon_png) = if let Some(item) = pinned {
        if item.kind != "app" {
            return Ok(());
        }
        (
            item.match_exe.clone(),
            item.real_path.clone(),
            if !item.label.is_empty() {
                item.label
            } else {
                item.match_exe.clone()
            },
            icon_png.or(item.icon_png),
        )
    } else {
        let mex = match_exe.unwrap_or_else(|| {
            item_id
                .strip_prefix("running:")
                .unwrap_or(item_id.as_str())
                .to_string()
        });
        if mex.is_empty() {
            return Ok(());
        }
        let rp = real_path.unwrap_or_default();
        let lb = label.unwrap_or_else(|| mex.clone());
        (mex, rp, lb, icon_png)
    };

    let epoch = preview_epoch().load(std::sync::atomic::Ordering::SeqCst);

    // Instant shell: window titles first (taskbar-like snappiness), thumbs fill in after.
    let match_exe_list = match_exe.clone();
    let real_path_list = real_path.clone();
    let mut frames = tauri::async_runtime::spawn_blocking(move || {
        list_preview_targets(&match_exe_list, &real_path_list)
    })
    .await
    .map_err(|e| e.to_string())?;
    if frames.is_empty() {
        return Ok(());
    }
    if preview_epoch().load(std::sync::atomic::Ordering::SeqCst) != epoch {
        return Ok(());
    }

    let payload = DockPreviewPayload {
        item_id: item_id.clone(),
        label: label.clone(),
        frames: frames.clone(),
        icon_png: icon_png.clone(),
    };
    let (menu_w, menu_h) = preview_window_size(frames.len());
    let (x, y) = clamp_preview_pos(
        &app,
        anchor_x - menu_w / 2.0,
        anchor_y - menu_h - 10.0,
        menu_w,
        menu_h,
    );

    if let Some(existing) = app.get_webview_window(DOCK_PREVIEW_LABEL) {
        let _ = app.emit("dock-preview", &payload);
        let _ = existing.set_size(LogicalSize::new(menu_w, menu_h));
        let _ = existing.set_position(LogicalPosition::new(x, y));
        if preview_epoch().load(std::sync::atomic::Ordering::SeqCst) != epoch {
            let _ = existing.hide();
            return Ok(());
        }
        let _ = existing.unminimize();
        let _ = existing.show();
    } else {
        let init = r#"
      window.__WH_IS_DOCK_PREVIEW__ = true;
      document.addEventListener('keydown', function (e) {
        if (e.key === 'Escape') {
          try { window.__TAURI__.core.invoke('close_dock_preview'); } catch (_) {}
        }
      });
    "#;

        let win = WebviewWindowBuilder::new(
            &app,
            DOCK_PREVIEW_LABEL,
            WebviewUrl::App("index.html?window=dock-preview".into()),
        )
        .title("Dock Preview")
        .inner_size(menu_w, menu_h)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .closable(true)
        .decorations(false)
        .transparent(crate::win32::blur_glass::popup_is_transparent())
        .background_color({
            use tauri::utils::config::Color;
            if crate::win32::blur_glass::popup_is_transparent() {
                Color(0, 0, 0, 0)
            } else {
                Color(28, 28, 30, 255)
            }
        })
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .initialization_script(init)
        .build()
        .map_err(|e| format!("open dock-preview failed: {e}"))?;

        let _ = win.set_position(LogicalPosition::new(x, y));
        apply_saved_material_pub(&win, &state);
        if let Ok(hwnd) = win.hwnd() {
            crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
            win32_dock_clear_frame(hwnd.0 as isize);
        }
        let _ = app.emit("dock-preview", &payload);
        if preview_epoch().load(std::sync::atomic::Ordering::SeqCst) != epoch {
            let _ = win.hide();
            return Ok(());
        }
        let _ = win.show();
    }

    // Windows taskbar style: DWM live thumbnails (instant, correct). JPEG only if DWM fails.
    let live_n = apply_preview_live_thumbs(&app, &frames);
    if live_n > 0 {
        return Ok(());
    }

    // Fallback: BitBlt/PrintWindow snapshots (slow — covered / GPU windows often blank).
    let app2 = app.clone();
    let item_id2 = item_id.clone();
    let label2 = label;
    let icon2 = icon_png;
    let match_exe2 = match_exe;
    let real_path2 = real_path;
    tauri::async_runtime::spawn(async move {
        let captured = tauri::async_runtime::spawn_blocking(move || {
            capture_previews_for_match(&match_exe2, &real_path2)
        })
        .await;
        let Ok(Ok(filled)) = captured else {
            return;
        };
        if preview_epoch().load(std::sync::atomic::Ordering::SeqCst) != epoch {
            return;
        }
        for (i, f) in frames.iter_mut().enumerate() {
            if let Some(nf) = filled.get(i) {
                if !nf.jpeg_base64.is_empty() {
                    f.jpeg_base64 = nf.jpeg_base64.clone();
                    f.width = nf.width;
                    f.height = nf.height;
                }
            } else if let Some(nf) = filled.iter().find(|x| x.hwnd == f.hwnd) {
                if !nf.jpeg_base64.is_empty() {
                    f.jpeg_base64 = nf.jpeg_base64.clone();
                    f.width = nf.width;
                    f.height = nf.height;
                }
            }
        }
        let _ = app2.emit(
            "dock-preview",
            &DockPreviewPayload {
                item_id: item_id2,
                label: label2,
                frames,
                icon_png: icon2,
            },
        );
    });

    Ok(())
}

#[tauri::command]
pub async fn close_dock_preview(app: AppHandle) -> Result<(), String> {
    preview_epoch().fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    preview_dwm::clear_thumbnails();
    if let Some(w) = app.get_webview_window(DOCK_PREVIEW_LABEL) {
        let _ = w.hide();
    }
    let _ = app.emit("dock-preview-closed", ());
    Ok(())
}

#[tauri::command]
pub async fn dock_set_extra_headroom(app: AppHandle, px: u32) -> Result<(), String> {
    let next = px.min(120);
    let prev = extra_headroom().swap(next, std::sync::atomic::Ordering::Relaxed);
    if prev == next {
        return Ok(());
    }
    let prefs = load_dock_prefs();
    if prefs.enabled {
        let app2 = app.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            position_dock_window(&app2, &prefs);
        })
        .await;
    }
    Ok(())
}

/// Re-assert WS_EX_NOACTIVATE + clear icon-layer fill. Call on every icon press so
/// Win10 cannot paint a light-blue activation slab over transparent headroom.
#[tauri::command]
pub fn dock_touch_noactivate(app: AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        if let Some(dock) = app.get_webview_window("dock") {
            let _ = dock.set_focusable(false);
            if let Ok(hwnd) = dock.hwnd() {
                win32_dock_clear_frame(hwnd.0 as isize);
            }
            let _ = crate::win32::blur_glass::apply_dock_icons_layer(&dock, None);
        }
        if let Some(glass) = app.get_webview_window(DOCK_GLASS_LABEL) {
            let _ = glass.set_focusable(false);
            if let Ok(hwnd) = glass.hwnd() {
                win32_dock_clear_frame(hwnd.0 as isize);
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = app;
    }
    Ok(())
}

/// Frontend reports extra content width for ephemeral running-app icons.
#[tauri::command]
pub async fn dock_set_runtime_extra_width(app: AppHandle, extra: f64) -> Result<(), String> {
    let px = extra.round().clamp(0.0, 2400.0) as u32;
    let prev = runtime_extra_width().swap(px, std::sync::atomic::Ordering::Relaxed);
    if prev == px {
        return Ok(());
    }
    let prefs = load_dock_prefs();
    if prefs.enabled {
        let app2 = app.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            position_dock_window(&app2, &prefs);
        })
        .await;
    }
    Ok(())
}

/// Resolve a PNG icon for an arbitrary exe path (running-app dock entries).
/// Always off the UI thread — Shell icon extract on Win10 freezes the app otherwise.
#[tauri::command]
pub async fn dock_resolve_exe_icon(path: String) -> Option<String> {
    let path = path.trim().to_string();
    if path.is_empty() {
        return None;
    }
    tauri::async_runtime::spawn_blocking(move || {
        // Same 256px shell path as pinned icons — 32px looked soft next to them.
        icon::resolve_item_icon_png("", &path).or_else(|| icon::resolve_small_icon_png(&path))
    })
    .await
    .ok()
    .flatten()
}

/// Capture previews off the UI thread (PrintWindow / BitBlt can stall for hundreds of ms).
#[tauri::command]
pub async fn dock_capture_item_previews(item_id: String) -> Result<Vec<DockPreviewFrame>, String> {
    tauri::async_runtime::spawn_blocking(move || dock_capture_item_previews_sync(item_id))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn dock_capture_exe_previews(
    match_exe: String,
    real_path: Option<String>,
) -> Result<Vec<DockPreviewFrame>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        capture_previews_for_match(&match_exe, real_path.as_deref().unwrap_or(""))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockItemMenuPayload {
    pub item_id: String,
    pub label: String,
    pub kind: String,
    pub windows: Vec<DockWindowLite>,
}

#[tauri::command]
pub async fn open_dock_item_menu(
    app: AppHandle,
    state: State<'_, MaterialState>,
    item_id: String,
    x: f64,
    y: f64,
) -> Result<(), String> {
    let _ = close_dock_preview(app.clone()).await;
    let prefs = with_icons(load_dock_prefs());
    let item = prefs
        .items
        .iter()
        .find(|i| i.id == item_id)
        .ok_or_else(|| format!("dock item not found: {item_id}"))?
        .clone();
    let wins = crate::win32::enum_windows::list_windows(None);
    let windows: Vec<DockWindowLite> = launch::matching_windows(&item, &wins)
        .into_iter()
        .map(|w| DockWindowLite {
            id: w.id,
            hwnd: w.hwnd,
            title: w.title,
        })
        .collect();
    let label = if item.kind == "startmenu" {
        "开始".into()
    } else if item.kind == "trash" {
        "回收站".into()
    } else if !item.label.is_empty() {
        item.label.clone()
    } else {
        item.match_exe.clone()
    };
    let payload = DockItemMenuPayload {
        item_id: item.id.clone(),
        label,
        kind: item.kind.clone(),
        windows: windows.clone(),
    };

    let menu_w = 200.0_f64;
    let menu_h = (56.0 + windows.len().min(6) as f64 * 28.0 + 160.0).clamp(200.0, 360.0);

    if let Some(existing) = app.get_webview_window(DOCK_ITEM_MENU_LABEL) {
        let _ = existing.set_size(LogicalSize::new(menu_w, menu_h));
        let _ = existing.set_position(LogicalPosition::new(x, y));
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_focus();
        let _ = app.emit("dock-item-menu", &payload);
        return Ok(());
    }

    let init = r#"
      window.__WH_IS_DOCK_ITEM_MENU__ = true;
      document.addEventListener('keydown', function (e) {
        if (e.key === 'Escape') {
          try { window.__TAURI__.core.invoke('close_dock_item_menu'); } catch (_) {}
        }
      });
    "#;

    let win = WebviewWindowBuilder::new(
        &app,
        DOCK_ITEM_MENU_LABEL,
        WebviewUrl::App("index.html?window=dock-item-menu".into()),
    )
    .title("Dock Menu")
    .inner_size(menu_w, menu_h)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .closable(true)
    .decorations(false)
    .transparent(crate::win32::blur_glass::popup_is_transparent())
    .background_color(crate::win32::blur_glass::popup_background_color())
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(true)
    .visible(false)
    .initialization_script(init)
    .build()
    .map_err(|e| format!("open dock-item-menu failed: {e}"))?;

    let _ = win.set_position(LogicalPosition::new(x, y));
    apply_saved_material_pub(&win, &state);
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
    }
    let _ = win.show();
    let _ = win.set_focus();
    let _ = app.emit("dock-item-menu", &payload);
    Ok(())
}

#[tauri::command]
pub async fn close_dock_item_menu(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(DOCK_ITEM_MENU_LABEL) {
        let _ = w.hide();
    }
    let _ = app.emit("dock-item-menu-closed", ());
    Ok(())
}

/// Apple-style thin peek strip while the dock is auto-hidden.
fn sync_trigger_strip(app: &AppHandle, prefs: &DockPrefs, dock_shown: bool) {
    let want = prefs.enabled
        && prefs.show_trigger_strip
        && !dock_shown
        && matches!(
            prefs.mode(),
            DockDisplayMode::AutoHide | DockDisplayMode::SmartHide
        );

    if !want {
        if let Some(t) = app.get_webview_window(DOCK_TRIGGER_LABEL) {
            #[cfg(windows)]
            if let Ok(hwnd) = t.hwnd() {
                win32_dock_show(hwnd.0 as isize, false);
            } else {
                let _ = t.hide();
            }
            #[cfg(not(windows))]
            {
                let _ = t.hide();
            }
        }
        return;
    }

    let width = match prefs.activation() {
        DockActivationPosition::DockBottom => dock_window_width(prefs).max(80.0),
        DockActivationPosition::ScreenBottom => {
            // Full monitor width looks like macOS edge highlight.
            if let Some(dock) = app.get_webview_window("dock") {
                if let Ok(Some(mon)) = dock.current_monitor() {
                    mon.size().width as f64 / mon.scale_factor()
                } else {
                    dock_window_width(prefs).max(200.0)
                }
            } else {
                dock_window_width(prefs).max(200.0)
            }
        }
    };

    let trigger = if let Some(t) = app.get_webview_window(DOCK_TRIGGER_LABEL) {
        t
    } else {
        let init = r#"window.__WH_IS_DOCK_TRIGGER__ = true;"#;
        match WebviewWindowBuilder::new(
            app,
            DOCK_TRIGGER_LABEL,
            WebviewUrl::App("index.html?window=dock-trigger".into()),
        )
        .title("Dock Trigger")
        .inner_size(width, DOCK_TRIGGER_H)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .closable(false)
        .decorations(false)
        .transparent(true)
        .background_color(tauri::utils::config::Color(0, 0, 0, 0))
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .initialization_script(init)
        .build()
        {
            Ok(w) => w,
            Err(e) => {
                eprintln!("[dock] trigger strip open failed: {e}");
                return;
            }
        }
    };

    let _ = trigger.set_ignore_cursor_events(true);
    let _ = trigger.set_size(LogicalSize::new(width, DOCK_TRIGGER_H));

    if let Some(dock) = app.get_webview_window("dock") {
        if let Ok(Some(mon)) = dock.current_monitor() {
            let scale = mon.scale_factor();
            let screen = mon.size();
            let origin = mon.position();
            let phys_w = (width * scale).round().max(1.0) as i32;
            let phys_h = (DOCK_TRIGGER_H * scale).round().max(1.0) as i32;
            let margin = (prefs.bottom_offset_px as f64 * scale).round() as i32;
            let x = origin.x + ((screen.width as i32 - phys_w) / 2).max(0);
            let y = origin.y + (screen.height as i32 - phys_h - margin).max(0);
            let _ = trigger.set_position(tauri::PhysicalPosition::new(x, y));
        }
    }
    #[cfg(windows)]
    if let Ok(hwnd) = trigger.hwnd() {
        win32_dock_show(hwnd.0 as isize, true);
    } else {
        let _ = trigger.show();
    }
    #[cfg(not(windows))]
    {
        let _ = trigger.show();
    }
}

#[tauri::command]
pub fn dock_set_mouse_near_bottom(
    vis: State<'_, Arc<DockVisibility>>,
    near: bool,
) -> Result<(), String> {
    vis.set_mouse_near_bottom(near);
    Ok(())
}

#[tauri::command]
pub fn get_dock_visibility(
    app: AppHandle,
    vis: State<'_, Arc<DockVisibility>>,
) -> visibility::DockVisibilityState {
    vis.snapshot(&app)
}

fn sync_dock_visual(app: &AppHandle, vis: &Arc<DockVisibility>) {
    // Place to the hysteresis state (`shown`/`desired`), never raw edge `want`.
    // Using snapshot().visible (instant near) was snap-hiding AutoHide docks
    // whenever ensure/set_prefs ran while the cursor was off the thin strip —
    // looks like “呼出闪一下就消失” with no leave-timer log.
    let visible = vis.ui_shown();
    vis.apply_dock_shown(app, visible, false);
    let _ = app.emit("dock-visibility", &vis.snapshot(app));
}

async fn ensure_dock_window_inner(
    app: &AppHandle,
    state: &MaterialState,
    vis: &Arc<DockVisibility>,
    prefs: &DockPrefs,
) -> Result<(), String> {
    vis.apply_prefs(prefs);
    // Do NOT start the visibility poller before the dock HWND exists + first place.
    // Early ticks raced bootstrap and deadlocked on dock_place_lock + Tauri marshal.

    let width = dock_window_width(prefs);
    let height = dock_window_height(prefs);
    let chrome_h = prefs.chrome_height();
    let glass_w = dock_glass_width(prefs);

    #[cfg(windows)]
    let skip_glass = crate::win32::blur_glass::is_hard_safe();
    #[cfg(not(windows))]
    let skip_glass = false;

    // Glass strip first (below icons): owns SWCA material at chrome height.
    // Win10: skip entirely — transparent glass HWND still paints a rectangular frame.
    if !skip_glass && app.get_webview_window(DOCK_GLASS_LABEL).is_none() {
        let glass_init = r#"
          window.__WH_IS_DOCK_GLASS__ = true;
        "#;
        let glass = WebviewWindowBuilder::new(
            app,
            DOCK_GLASS_LABEL,
            WebviewUrl::App("index.html?window=dock-glass".into()),
        )
        .title("")
        .inner_size(glass_w, chrome_h)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .closable(false)
        .decorations(false)
        // Always transparent: hard_safe opaque fill became a rectangular slab behind
        // the rounded CSS chrome. Material (when available) still applies via SWCA.
        .transparent(true)
        .background_color(tauri::utils::config::Color(0, 0, 0, 0))
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .focusable(false)
        .visible(false)
        .initialization_script(glass_init)
        .build()
        .map_err(|e| format!("open dock-glass failed: {e}"))?;
        let _ = glass.set_ignore_cursor_events(true);
        apply_saved_material_pub(&glass, state);
        #[cfg(windows)]
        if let Ok(hwnd) = glass.hwnd() {
            win32_dock_clear_frame(hwnd.0 as isize);
        }
    } else if skip_glass {
        // Hide via Win32 only — Tauri hide can deadlock with place_dock_window.
        if let Some(g) = app.get_webview_window(DOCK_GLASS_LABEL) {
            #[cfg(windows)]
            if let Ok(hwnd) = g.hwnd() {
                win32_dock_show(hwnd.0 as isize, false);
            }
            #[cfg(not(windows))]
            {
                let _ = g.hide();
            }
        }
    } else if let Some(glass) = app.get_webview_window(DOCK_GLASS_LABEL) {
        let _ = glass.set_ignore_cursor_events(true);
        apply_saved_material_pub(&glass, state);
    }

    if let Some(existing) = app.get_webview_window("dock") {
        let _ = existing.set_focusable(false);
        apply_saved_material_pub(&existing, state);
        #[cfg(windows)]
        if let Ok(hwnd) = existing.hwnd() {
            win32_dock_clear_frame(hwnd.0 as isize);
        }
        sync_dock_visual(app, vis);
        vis.start(app.clone());
        return Ok(());
    }

    let init = r#"
      window.__WH_IS_DOCK__ = true;
    "#;

    let win = WebviewWindowBuilder::new(
        app,
        "dock",
        WebviewUrl::App("index.html?window=dock".into()),
    )
    .title("")
    .inner_size(width, height)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .closable(false)
    .decorations(false)
    // Icons layer must stay transparent even in hard_safe — opaque fill paints
    // headroom as a solid white slab above the glass strip.
    .transparent(true)
    .background_color(tauri::utils::config::Color(0, 0, 0, 0))
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .focusable(false)
    .visible(false)
    .initialization_script(init)
    .build()
    .map_err(|e| format!("open dock failed: {e}"))?;

    // Icons layer: clear material (glass sibling owns acrylic).
    apply_saved_material_pub(&win, state);
    #[cfg(windows)]
    if let Ok(hwnd) = win.hwnd() {
        win32_dock_clear_frame(hwnd.0 as isize);
    }
    // Snap whole window to shown/hidden rest pose (no CSS half-state).
    sync_dock_visual(app, vis);
    vis.start(app.clone());
    Ok(())
}

#[tauri::command]
pub async fn ensure_dock_window(
    app: AppHandle,
    state: State<'_, MaterialState>,
    vis: State<'_, Arc<DockVisibility>>,
) -> Result<(), String> {
    let prefs = load_dock_prefs();
    if !prefs.enabled {
        return Ok(());
    }
    ensure_dock_window_inner(&app, &state, &vis, &prefs).await?;
    apply_taskbar_for_dock(true);
    Ok(())
}

/// Called from app setup when dock was enabled last session.
pub fn bootstrap_dock(app: &AppHandle) {
    let mut prefs = load_dock_prefs();
    if !prefs.enabled {
        return;
    }
    prefs.hide_system_taskbar = true;
    let _ = save_dock_prefs(&prefs);
    let app2 = app.clone();
    let prefs2 = prefs.clone();
    tauri::async_runtime::spawn(async move {
        let state = app2.state::<MaterialState>();
        let vis = app2.state::<Arc<DockVisibility>>();
        if let Err(e) = ensure_dock_window_inner(&app2, &*state, &*vis, &prefs2).await {
            eprintln!("[dock] bootstrap failed: {e}");
            return;
        }
        apply_taskbar_for_dock(true);
        // One delayed material/place pass — do not loop (races auto-hide animation).
        let app3 = app2.clone();
        let prefs3 = prefs2.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            apply_taskbar_for_dock(true);
            if let Some(vis) = app3.try_state::<Arc<DockVisibility>>() {
                if vis.is_busy() {
                    return;
                }
            }
            position_dock_window(&app3, &prefs3);
            let state = app3.state::<MaterialState>();
            if let Some(g) = app3.get_webview_window(DOCK_GLASS_LABEL) {
                apply_saved_material_pub(&g, &*state);
            }
            if let Some(w) = app3.get_webview_window("dock") {
                apply_saved_material_pub(&w, &*state);
            }
        });
    });
}
