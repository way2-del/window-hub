//! Host bottom dock — MyDockFinder-style icons, visibility modes, ini import.

mod icon;
mod ini;
mod launch;
mod visibility;
pub mod genie;

pub use ini::parse_dockico_ini;
pub use launch::launch_or_focus;
pub use visibility::DockVisibility;

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::utils::config::Color;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, State, WebviewUrl, WebviewWindowBuilder,
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
    /// Hover icon scale is fixed in Host (`DOCK_MAG_SCALE`); kept for serde compat.
    #[serde(default = "default_magnification")]
    pub magnification: f64,
    /// Logical px corner radius for the glass strip (0 = square). Uses SetWindowRgn
    /// so Win11 DWM rounded-shadow chrome is not involved.
    #[serde(default = "default_corner_radius_px")]
    pub corner_radius_px: u32,
    /// Genie suck/expand duration in milliseconds (200–1500).
    #[serde(default = "default_genie_duration_ms")]
    pub genie_duration_ms: u32,
    /// Pinned item ids temporarily omitted when the dock overflows the monitor.
    /// Running matches still show; restore via status-menu right-click.
    #[serde(default)]
    pub hidden_item_ids: Vec<String>,
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
    1.6 // == DOCK_MAG_SCALE (const defined below with geometry)
}

fn default_corner_radius_px() -> u32 {
    12
}

fn default_genie_duration_ms() -> u32 {
    560
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
            corner_radius_px: default_corner_radius_px(),
            genie_duration_ms: default_genie_duration_ms(),
            hidden_item_ids: Vec::new(),
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

    fn normalize(mut self) -> Self {
        self.display_mode = self.mode().as_str().into();
        self.activation_position = self.activation().as_str().into();
        self.activation_thickness_px = self.activation_thickness_px.clamp(4, 64);
        self.bottom_offset_px = self.bottom_offset_px.min(400);
        self.hide_linger_ms = self.hide_linger_ms.clamp(200, 10_000);
        self.magnification = clamp_magnification(self.magnification);
        self.corner_radius_px = self.corner_radius_px.min(28);
        self.genie_duration_ms = self.genie_duration_ms.clamp(200, 1500);
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
    for item in &mut prefs.items {
        if item.kind == "separator" {
            item.icon_png = None;
            continue;
        }
        item.icon_png = icon::resolve_item_icon_png(&item.icon_path, &item.launch_path);
    }
    prefs
}

/// Merge pinned dock items with running apps that are not on the dock.
/// Running extras sit before trash, behind an auto separator.
pub(crate) fn dock_layout_items(prefs: &DockPrefs) -> Vec<DockItem> {
    dock_merge_running(prefs, false)
}

fn windows_exe_sig(windows: &[crate::win32::enum_windows::WindowInfo]) -> String {
    let mut exes: Vec<String> = windows
        .iter()
        .filter_map(|w| w.exe.as_ref())
        .map(|e| e.to_ascii_lowercase())
        .collect();
    exes.sort();
    exes.dedup();
    exes.join("|")
}

fn pinned_layout_sig(items: &[DockItem]) -> String {
    items
        .iter()
        .map(|i| format!("{}:{}", i.id, i.kind))
        .collect::<Vec<_>>()
        .join("|")
}

fn hidden_sig(ids: &[String]) -> String {
    let mut v = ids.to_vec();
    v.sort();
    v.join("|")
}

fn item_is_running(item: &DockItem, windows: &[crate::win32::enum_windows::WindowInfo]) -> bool {
    if item.kind != "app" {
        return false;
    }
    windows.iter().any(|w| launch::item_matches_window(item, w))
}

/// Overflow-hidden pins stay omitted unless they currently have a window.
fn item_is_overflow_hidden(
    item: &DockItem,
    hidden: &[String],
    windows: &[crate::win32::enum_windows::WindowInfo],
) -> bool {
    if hidden.is_empty() || !hidden.iter().any(|id| id == &item.id) {
        return false;
    }
    // Keep showing if the app is open — frees space was for running clicks.
    !item_is_running(item, windows)
}

pub(crate) fn dock_merge_running(prefs: &DockPrefs, with_icons: bool) -> Vec<DockItem> {
    use std::collections::HashSet;

    let items = &prefs.items;
    let windows = crate::windows_service::cached_windows();
    let key = format!(
        "{}::{}::{}",
        pinned_layout_sig(items),
        hidden_sig(&prefs.hidden_item_ids),
        windows_exe_sig(&windows)
    );
    {
        let guard = merge_cache().lock();
        if let Some(hit) = guard.as_ref() {
            if hit.key == key && hit.with_icons == with_icons {
                return hit.items.clone();
            }
            if hit.key == key && hit.with_icons && !with_icons {
                let mut stripped = hit.items.clone();
                for it in &mut stripped {
                    if it.id.starts_with("running:") {
                        it.icon_png = None;
                    }
                }
                return stripped;
            }
        }
    }

    let self_exe = std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    let mut head: Vec<DockItem> = Vec::new();
    let mut trash: Option<DockItem> = None;
    for it in items {
        if it.kind == "trash" {
            if trash.is_none() {
                trash = Some(it.clone());
            }
            continue;
        }
        if item_is_overflow_hidden(it, &prefs.hidden_item_ids, &windows) {
            continue;
        }
        head.push(it.clone());
    }

    let mut seen_exe: HashSet<String> = HashSet::new();
    let mut extras: Vec<DockItem> = Vec::new();
    for w in &windows {
        let exe = w.exe.as_deref().unwrap_or("").trim();
        if exe.is_empty() {
            continue;
        }
        let exe_key = exe.to_ascii_lowercase();
        if !self_exe.is_empty() && exe_key == self_exe {
            continue;
        }
        if seen_exe.contains(&exe_key) {
            continue;
        }
        if items.iter().any(|it| launch::item_matches_window(it, w)) {
            continue;
        }
        seen_exe.insert(exe_key.clone());
        let stem = w
            .exe_name
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                std::path::Path::new(exe)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("app")
                    .to_string()
            });
        let match_exe = if stem.to_ascii_lowercase().ends_with(".exe") {
            stem.clone()
        } else {
            format!("{stem}.exe")
        };
        let icon_png = if with_icons {
            icon::resolve_item_icon_png("", exe)
        } else {
            None
        };
        extras.push(DockItem {
            id: format!("running:{exe_key}"),
            kind: "app".into(),
            label: if w.title.trim().is_empty() {
                stem
            } else {
                w.title.clone()
            },
            match_exe,
            launch_path: exe.to_string(),
            real_path: exe.to_string(),
            virtual_path: String::new(),
            icon_path: exe.to_string(),
            uwp: false,
            icon_png,
        });
    }

    let mut out = head;
    if !extras.is_empty() {
        let needs_sep = out
            .last()
            .map(|i| i.kind != "separator")
            .unwrap_or(true);
        if needs_sep {
            out.push(DockItem {
                id: "running-sep".into(),
                kind: "separator".into(),
                label: String::new(),
                match_exe: String::new(),
                launch_path: String::new(),
                real_path: String::new(),
                virtual_path: String::new(),
                icon_path: String::new(),
                uwp: false,
                icon_png: None,
            });
        }
        out.extend(extras);
    }
    if let Some(t) = trash {
        out.push(t);
    }

    *merge_cache().lock() = Some(MergeCache {
        key,
        with_icons,
        items: out.clone(),
    });
    out
}

struct MergeCache {
    key: String,
    with_icons: bool,
    items: Vec<DockItem>,
}

fn merge_cache() -> &'static parking_lot::Mutex<Option<MergeCache>> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<parking_lot::Mutex<Option<MergeCache>>> = OnceLock::new();
    CACHE.get_or_init(|| parking_lot::Mutex::new(None))
}

fn invalidate_dock_layout_cache() {
    icon::clear_icon_cache();
    *merge_cache().lock() = None;
}

/// Max logical width the dock may occupy (leave side margins + hover fan pad).
fn dock_width_budget(monitor_logical_w: f64) -> f64 {
    (monitor_logical_w - 48.0).max(200.0)
}

fn dock_monitor_logical_width(app: &AppHandle) -> Option<f64> {
    let win = app.get_webview_window("dock")?;
    let mon = win.current_monitor().ok().flatten()?;
    Some(mon.size().width as f64 / mon.scale_factor())
}

#[cfg(windows)]
fn dock_primary_monitor_logical_width() -> Option<f64> {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN};
    unsafe {
        let px = GetSystemMetrics(SM_CXSCREEN);
        if px <= 0 {
            return None;
        }
        // Approximate 1.0 scale when dock window is not up yet.
        Some(px as f64)
    }
}

#[cfg(not(windows))]
fn dock_primary_monitor_logical_width() -> Option<f64> {
    None
}

/// Hide unopened pinned apps (right-to-left) until hover width fits the monitor.
/// Returns how many ids were newly added to `hidden_item_ids`.
fn dock_auto_compact(prefs: &mut DockPrefs, monitor_logical_w: f64) -> usize {
    let budget = dock_width_budget(monitor_logical_w);
    let windows = crate::windows_service::cached_windows();
    let mut newly = 0usize;

    loop {
        let layout = dock_merge_running(prefs, false);
        if dock_expanded_width(&layout) <= budget {
            break;
        }
        // Prefer hiding unopened pins near trash (end of pin list).
        let candidate = prefs
            .items
            .iter()
            .rev()
            .find(|it| {
                if it.kind != "app" {
                    return false;
                }
                if prefs.hidden_item_ids.iter().any(|id| id == &it.id) {
                    return false;
                }
                !item_is_running(it, &windows)
            })
            .map(|it| it.id.clone());
        let Some(id) = candidate else {
            break;
        };
        prefs.hidden_item_ids.push(id);
        newly += 1;
        *merge_cache().lock() = None;
    }
    newly
}

/// Run compact if needed; persist + notify UI when something was newly hidden.
pub(crate) fn dock_compact_and_notify(app: &AppHandle, prefs: &mut DockPrefs) {
    let Some(mon_w) = dock_monitor_logical_width(app).or_else(dock_primary_monitor_logical_width)
    else {
        return;
    };
    let newly = dock_auto_compact(prefs, mon_w);
    if newly == 0 {
        return;
    }
    let _ = save_dock_prefs(prefs);
    invalidate_dock_layout_cache();
    let payload = serde_json::json!({
        "newlyHidden": newly,
        "totalHidden": prefs.hidden_item_ids.len(),
    });
    let _ = app.emit("dock-compacted", &payload);
    let _ = app.emit("dock-prefs", &with_icons(prefs.clone()));
}

pub(crate) const DOCK_H: f64 = 60.0;
const DOCK_ICON: f64 = 40.0;
const DOCK_GAP: f64 = 6.0;
const DOCK_PAD_X: f64 = 2.0;
const DOCK_SEP: f64 = 10.0;
/// Fixed hover magnification (not user-configurable).
pub(crate) const DOCK_MAG_SCALE: f64 = 1.6;
/// Fixed total extra logical width when hovering (not dynamically measured).
const DOCK_FAN_EXTRA: f64 = 48.0;
const DOCK_GLASS_LABEL: &str = "dock-glass";

/// True while the dock HWND is in the fixed expanded hover size.
fn hover_expanded_flag() -> &'static std::sync::atomic::AtomicBool {
    static FLAG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    &FLAG
}

pub(crate) fn dock_hover_expanded() -> bool {
    hover_expanded_flag().load(std::sync::atomic::Ordering::Relaxed)
}

fn set_hover_expanded(v: bool) {
    hover_expanded_flag().store(v, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn set_hover_expanded_pub(v: bool) {
    set_hover_expanded(v);
}

/// Magnification is fixed — prefs field kept for serde compat only.
fn clamp_magnification(_m: f64) -> f64 {
    DOCK_MAG_SCALE
}

/// Base content width (unscaled icon slots) — rest glass / icons width.
pub(crate) fn dock_content_width(items: &[DockItem]) -> f64 {
    let mut w = DOCK_PAD_X * 2.0;
    for (i, it) in items.iter().enumerate() {
        if i > 0 {
            w += DOCK_GAP;
        }
        if it.kind == "separator" {
            w += DOCK_SEP;
        } else {
            w += DOCK_ICON;
        }
    }
    w.max(120.0)
}

/// Hover width = content + fixed pad (no per-frame fan measurement).
pub(crate) fn dock_expanded_width(items: &[DockItem]) -> f64 {
    dock_content_width(items) + DOCK_FAN_EXTRA
}

/// Icons window at rest is content-sized; expands by fixed pad on hover.
pub(crate) fn dock_window_width(items: &[DockItem], _magnification: f64) -> f64 {
    dock_content_width(items)
}

/// Fixed headroom for `DOCK_MAG_SCALE` (slider removed).
/// Extra 8px slack so the peaked icon is not clipped at the HWND top edge.
pub(crate) fn dock_headroom(_magnification: f64) -> f64 {
    DOCK_ICON * (DOCK_MAG_SCALE - 1.0) + 8.0
}

pub(crate) fn dock_window_height(magnification: f64) -> f64 {
    DOCK_H + dock_headroom(magnification)
}

/// Logical height of the interactive chrome strip (excludes fan headroom).
pub(crate) fn dock_chrome_height() -> f64 {
    DOCK_H
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
/// Material lives on sibling `dock-glass` (fixed `DOCK_H`); icons window may be
/// taller for magnification headroom.
pub fn place_dock_window(app: &AppHandle, prefs: &DockPrefs, shown: bool, animate: bool) {
    let _guard = dock_place_lock().lock().unwrap_or_else(|e| e.into_inner());
    if !shown {
        set_hover_expanded(false);
    }
    // Compact against current monitor before measuring width.
    let mut prefs_owned = prefs.clone();
    let mon_w = dock_monitor_logical_width(app).or_else(dock_primary_monitor_logical_width);
    if let Some(w) = mon_w {
        let newly = dock_auto_compact(&mut prefs_owned, w);
        if newly > 0 {
            let _ = save_dock_prefs(&prefs_owned);
            invalidate_dock_layout_cache();
            let payload = serde_json::json!({
                "newlyHidden": newly,
                "totalHidden": prefs_owned.hidden_item_ids.len(),
            });
            let _ = app.emit("dock-compacted", &payload);
            let _ = app.emit("dock-prefs", &with_icons(prefs_owned.clone()));
        }
    }
    let prefs = &prefs_owned;
    let Some(win) = app.get_webview_window("dock") else {
        return;
    };
    let layout = dock_layout_items(prefs);
    let width = dock_window_width(&layout, prefs.magnification);
    let glass_w = dock_content_width(&layout);
    let height = dock_window_height(prefs.magnification);
    let glass = app.get_webview_window(DOCK_GLASS_LABEL);

    // Click-through only while hiding / tucked. Clear it *before* a show slide so
    // geometry + HWND hit-tests work as soon as the bar is on-screen.
    if shown {
        let _ = win.set_ignore_cursor_events(false);
        #[cfg(windows)]
        if let Ok(hwnd) = win.hwnd() {
            win32_dock_clear_transparent(hwnd.0 as isize);
        }
    } else {
        let _ = win.set_ignore_cursor_events(true);
    }
    if let Some(g) = &glass {
        // Glass is visual-only — never steal hits from the icons layer.
        let _ = g.set_ignore_cursor_events(true);
    }

    #[cfg(windows)]
    {
        let glass_hwnd = glass
            .as_ref()
            .and_then(|g| g.hwnd().ok())
            .map(|h| h.0 as isize);
        if let Ok(hwnd) = win.hwnd() {
            // Size+move only via Win32 on the root HWND. Calling Tauri `set_size` /
            // `set_position` here races DWM and can yank the window back (flash-hide).
            if win32_dock_slide_root(
                hwnd.0 as isize,
                glass_hwnd,
                width,
                glass_w,
                height,
                prefs.bottom_offset_px,
                prefs.corner_radius_px,
                shown,
                animate,
            ) {
                // Always clear click-through after a successful show place.
                let _ = win.set_ignore_cursor_events(!shown);
                if shown {
                    win32_dock_clear_transparent(hwnd.0 as isize);
                }
                return;
            }
        }
    }

    // Non-Windows / Win32 geom failure: Tauri fallback.
    let _ = win.set_size(LogicalSize::new(width, height));
    if let Some(g) = &glass {
        let _ = g.set_size(LogicalSize::new(glass_w, DOCK_H));
    }

    let Ok(Some(monitor)) = win.current_monitor() else {
        let _ = win.set_ignore_cursor_events(!shown);
        return;
    };
    let scale = monitor.scale_factor();
    let screen = monitor.size();
    let origin = monitor.position();
    let phys_w = (width * scale).round().max(1.0) as i32;
    let phys_h = (height * scale).round().max(1.0) as i32;
    let glass_phys_w = (glass_w * scale).round().max(1.0) as i32;
    let glass_h = (DOCK_H * scale).round().max(1.0) as i32;
    let margin = (prefs.bottom_offset_px as f64 * scale).round() as i32;
    let x = origin.x + ((screen.width as i32 - phys_w) / 2).max(0);
    let y_shown = origin.y + (screen.height as i32 - phys_h - margin).max(0);
    let y = if shown {
        y_shown
    } else {
        y_shown + phys_h + 4
    };
    let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
    if let Some(g) = &glass {
        let gx = x + ((phys_w - glass_phys_w) / 2).max(0);
        let gy = y + phys_h - glass_h;
        let _ = g.set_position(tauri::PhysicalPosition::new(gx, gy));
        if shown {
            let _ = g.show();
        } else {
            let _ = g.hide();
        }
        #[cfg(windows)]
        if let Ok(gh) = g.hwnd() {
            win32_dock_glass_set_round(gh.0 as isize, prefs.corner_radius_px);
        }
    }
    if shown {
        let _ = win.show();
    } else {
        let _ = win.hide();
    }
    let _ = win.set_ignore_cursor_events(!shown);
}

/// Toggle between resting content width and content+fixed pad, with a short
/// width tween. Returns whether the size change was applied (or already matched).
#[tauri::command]
pub fn dock_set_hover_expand(
    app: AppHandle,
    vis: State<'_, Arc<DockVisibility>>,
    expanded: bool,
) -> bool {
    // No place-lock / busy gate: first hover often overlaps show settle; callers
    // must be able to widen immediately and retry if the window is missing.
    if !vis.ui_shown() {
        set_hover_expanded(false);
        return false;
    }
    if dock_hover_expanded() == expanded {
        return true;
    }
    let prefs = load_dock_prefs();
    let layout = dock_layout_items(&prefs);
    let logical_w = if expanded {
        dock_expanded_width(&layout)
    } else {
        dock_content_width(&layout)
    };
    let logical_h = dock_window_height(prefs.magnification);
    let Some(win) = app.get_webview_window("dock") else {
        return false;
    };
    let glass = app.get_webview_window(DOCK_GLASS_LABEL);

    #[cfg(windows)]
    {
        let glass_hwnd = glass
            .as_ref()
            .and_then(|g| g.hwnd().ok())
            .map(|h| h.0 as isize);
        if let Ok(hwnd) = win.hwnd() {
            if win32_dock_tween_pair_width(
                hwnd.0 as isize,
                glass_hwnd,
                logical_w,
                logical_h,
                prefs.corner_radius_px,
            ) {
                set_hover_expanded(expanded);
                return true;
            }
        }
    }

    let _ = win.set_size(LogicalSize::new(logical_w, logical_h));
    if let Some(g) = &glass {
        let _ = g.set_size(LogicalSize::new(logical_w, DOCK_H));
    }
    let Ok(Some(monitor)) = win.current_monitor() else {
        set_hover_expanded(expanded);
        return true;
    };
    let scale = monitor.scale_factor();
    let screen = monitor.size();
    let origin = monitor.position();
    let phys_w = (logical_w * scale).round().max(1.0) as i32;
    let phys_h = (logical_h * scale).round().max(1.0) as i32;
    let glass_h = (DOCK_H * scale).round().max(1.0) as i32;
    let x = origin.x + ((screen.width as i32 - phys_w) / 2).max(0);
    let Ok(pos) = win.outer_position() else {
        set_hover_expanded(expanded);
        return true;
    };
    let y = pos.y;
    let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
    if let Some(g) = &glass {
        let gy = y + phys_h - glass_h;
        let _ = g.set_position(tauri::PhysicalPosition::new(x, gy));
        #[cfg(windows)]
        if let Ok(gh) = g.hwnd() {
            win32_dock_glass_set_round(gh.0 as isize, prefs.corner_radius_px);
        }
    }
    set_hover_expanded(expanded);
    true
}

/// Legacy alias — maps any width request to expand/collapse vs content width.
#[tauri::command]
pub fn dock_set_live_width(app: AppHandle, vis: State<'_, Arc<DockVisibility>>, width: f64) -> bool {
    let prefs = load_dock_prefs();
    let layout = dock_layout_items(&prefs);
    let rest = dock_content_width(&layout);
    let expanded = width.is_finite() && width > rest + DOCK_FAN_EXTRA * 0.5;
    dock_set_hover_expand(app, vis, expanded)
}

/// Animate icons + glass width to `logical_w` (~160ms ease-out), keep Y, re-center X.
#[cfg(windows)]
fn win32_dock_tween_pair_width(
    icons_hwnd_raw: isize,
    glass_hwnd_raw: Option<isize>,
    logical_w: f64,
    logical_h: f64,
    corner_radius_px: u32,
) -> bool {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER,
    };

    let icons = dock_root_hwnd(icons_hwnd_raw);
    unsafe {
        let mut wr = RECT::default();
        if GetWindowRect(icons, &mut wr).is_err() {
            return false;
        }
        let mon = MonitorFromWindow(icons, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return false;
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
        let cur_w = (wr.right - wr.left).max(1);
        let from_logical = cur_w as f64 / scale;
        let to_logical = logical_w;
        let phys_h = (logical_h * scale).round().max(1.0) as i32;
        let glass_h = (DOCK_H * scale).round().max(1.0) as i32;
        let mon_w = mi.rcMonitor.right - mi.rcMonitor.left;
        let y = wr.top;

        let apply = |w_logical: f64, round: bool| {
            let phys_w = (w_logical * scale).round().max(1.0) as i32;
            let x = mi.rcMonitor.left + ((mon_w - phys_w) / 2).max(0);
            let _ = SetWindowPos(
                icons,
                None,
                x,
                y,
                phys_w,
                phys_h,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            if let Some(raw) = glass_hwnd_raw {
                let gh = dock_root_hwnd(raw);
                let gy = y + phys_h - glass_h;
                let _ = SetWindowPos(
                    gh,
                    None,
                    x,
                    gy,
                    phys_w,
                    glass_h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                let _ = SetWindowPos(
                    gh,
                    icons,
                    0,
                    0,
                    0,
                    0,
                    windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                        | windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE
                        | windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
                );
                if round {
                    win32_dock_glass_set_round(raw, corner_radius_px);
                }
            }
        };

        if (from_logical - to_logical).abs() < 1.0 {
            apply(to_logical, true);
            return true;
        }

        // ~160ms ease-out — matches icon layout transition.
        const FRAMES: u32 = 12;
        const FRAME_MS: u64 = 13;
        for i in 1..=FRAMES {
            let t = i as f64 / FRAMES as f64;
            let e = 1.0 - (1.0 - t).powi(3);
            let w = from_logical + (to_logical - from_logical) * e;
            apply(w, i == FRAMES);
            if i < FRAMES {
                std::thread::sleep(std::time::Duration::from_millis(FRAME_MS));
            }
        }
        true
    }
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

/// Refresh glass corners after place/resize (SWCA + DWM corner preference).
#[cfg(windows)]
fn win32_dock_glass_set_round(hwnd_raw: isize, corner_radius_px: u32) {
    let hwnd = dock_root_hwnd(hwnd_raw);
    crate::win32::blur_glass::apply_dock_glass_round_frost_pub(hwnd, corner_radius_px);
}

/// Ensure the dock can be hit-tested (auto-hide keep-alive + clicks).
#[cfg(windows)]
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
    glass_logical_w: f64,
    logical_h: f64,
    bottom_offset_px: u32,
    corner_radius_px: u32,
    shown: bool,
    animate: bool,
) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE, SW_SHOWNOACTIVATE};

    let Some(geom) = win32_dock_geom(hwnd_raw, logical_w, logical_h, bottom_offset_px) else {
        return false;
    };
    let hwnd = dock_root_hwnd(hwnd_raw);
    let glass = glass_hwnd_raw.map(dock_root_hwnd);
    let glass_raw = glass_hwnd_raw;
    let scale = if geom.h > 0 && logical_h > 0.0 {
        geom.h as f64 / logical_h
    } else {
        1.0
    };
    let glass_h = (DOCK_H * scale).round().max(1.0) as i32;
    let glass_w = (glass_logical_w * scale).round().max(1.0) as i32;

    unsafe fn set_pair_pos(
        hwnd: windows::Win32::Foundation::HWND,
        glass: Option<windows::Win32::Foundation::HWND>,
        glass_raw: Option<isize>,
        geom: DockGeom,
        y: i32,
        glass_w: i32,
        glass_h: i32,
        corner_radius_px: u32,
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
            if let Some(raw) = glass_raw {
                win32_dock_glass_set_round(raw, corner_radius_px);
            }
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

    /// Ease-out cubic for show (fast start, soft settle); ease-in cubic for hide.
    fn ease(t: f64, show: bool) -> f64 {
        let t = t.clamp(0.0, 1.0);
        if show {
            1.0 - (1.0 - t).powi(3)
        } else {
            t * t * t
        }
    }

    unsafe fn tween_y(
        hwnd: windows::Win32::Foundation::HWND,
        glass: Option<windows::Win32::Foundation::HWND>,
        glass_raw: Option<isize>,
        geom: DockGeom,
        y_from: i32,
        y_to: i32,
        rising: bool,
        glass_w: i32,
        glass_h: i32,
        corner_radius_px: u32,
    ) {
        if (y_from - y_to).abs() <= 1 {
            set_pair_pos(
                hwnd,
                glass,
                glass_raw,
                geom,
                y_to,
                glass_w,
                glass_h,
                corner_radius_px,
            );
            return;
        }
        // ~180ms — readable rise without feeling laggy.
        const FRAMES: u32 = 15;
        const FRAME_MS: u64 = 12;
        for i in 1..=FRAMES {
            let t = i as f64 / FRAMES as f64;
            let e = ease(t, rising);
            let y = (y_from as f64 + (y_to as f64 - y_from as f64) * e).round() as i32;
            set_pair_y(hwnd, glass, geom, y, glass_w, glass_h);
            std::thread::sleep(std::time::Duration::from_millis(FRAME_MS));
        }
        set_pair_pos(
            hwnd,
            glass,
            glass_raw,
            geom,
            y_to,
            glass_w,
            glass_h,
            corner_radius_px,
        );
    }

    unsafe {
        if !animate {
            set_pair_pos(
                hwnd,
                glass,
                glass_raw,
                geom,
                if shown { geom.y_shown } else { geom.y_hidden },
                glass_w,
                glass_h,
                corner_radius_px,
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
            set_pair_pos(
                hwnd,
                glass,
                glass_raw,
                geom,
                geom.y_hidden,
                glass_w,
                glass_h,
                corner_radius_px,
            );
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            if let Some(gh) = glass {
                let _ = ShowWindow(gh, SW_SHOWNOACTIVATE);
            }
            tween_y(
                hwnd,
                glass,
                glass_raw,
                geom,
                geom.y_hidden,
                geom.y_shown,
                true,
                glass_w,
                glass_h,
                corner_radius_px,
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
                set_pair_pos(
                    hwnd,
                    glass,
                    glass_raw,
                    geom,
                    geom.y_hidden,
                    glass_w,
                    glass_h,
                    corner_radius_px,
                );
                let _ = ShowWindow(hwnd, SW_HIDE);
                if let Some(gh) = glass {
                    let _ = ShowWindow(gh, SW_HIDE);
                }
            } else {
                set_pair_pos(
                    hwnd,
                    glass,
                    glass_raw,
                    geom,
                    y_from,
                    glass_w,
                    glass_h,
                    corner_radius_px,
                );
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                if let Some(gh) = glass {
                    let _ = ShowWindow(gh, SW_SHOWNOACTIVATE);
                }
                tween_y(
                    hwnd,
                    glass,
                    glass_raw,
                    geom,
                    y_from,
                    geom.y_hidden,
                    false,
                    glass_w,
                    glass_h,
                    corner_radius_px,
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
pub fn get_dock_prefs() -> DockPrefs {
    with_icons(load_dock_prefs())
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
    // Drop stale hide ids after pin list edits / imports.
    next.hidden_item_ids
        .retain(|id| next.items.iter().any(|it| it.id == *id));
    next = with_icons(next);
    save_dock_prefs(&next)?;
    invalidate_dock_layout_cache();
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
    invalidate_dock_layout_cache();
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
    let items = dock_merge_running(&prefs, false);
    let item = items
        .iter()
        .find(|i| i.id == item_id)
        .ok_or_else(|| format!("dock item not found: {item_id}"))?
        .clone();
    launch_or_focus(&item)
}

/// Pinned items + running apps not on the dock (before trash), with icons.
#[tauri::command]
pub fn get_dock_display_items(app: AppHandle) -> Vec<DockItem> {
    let mut prefs = load_dock_prefs();
    dock_compact_and_notify(&app, &mut prefs);
    let prefs = with_icons(prefs);
    dock_merge_running(&prefs, true)
}

#[tauri::command]
pub fn dock_relayout(app: AppHandle) {
    let mut prefs = load_dock_prefs();
    if !prefs.enabled {
        return;
    }
    dock_compact_and_notify(&app, &mut prefs);
    position_dock_window(&app, &prefs);
}

/// Clear overflow-hidden pins and relayout (status-menu right-click restore).
#[tauri::command]
pub fn dock_restore_hidden_items(app: AppHandle) -> Result<DockPrefs, String> {
    let mut prefs = load_dock_prefs();
    if prefs.hidden_item_ids.is_empty() {
        return Ok(with_icons(prefs));
    }
    prefs.hidden_item_ids.clear();
    invalidate_dock_layout_cache();
    // May immediately re-compact if still overflowing — that re-emits dock-compacted.
    dock_compact_and_notify(&app, &mut prefs);
    save_dock_prefs(&prefs)?;
    let prefs = with_icons(prefs);
    let _ = app.emit("dock-prefs", &prefs);
    if prefs.enabled {
        position_dock_window(&app, &prefs);
    }
    Ok(prefs)
}

#[tauri::command]
pub fn get_dock_hidden_count() -> usize {
    load_dock_prefs().hidden_item_ids.len()
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
    vis.start(app.clone());

    let layout = dock_layout_items(prefs);
    let width = dock_window_width(&layout, prefs.magnification);
    let glass_w = dock_content_width(&layout);
    let height = dock_window_height(prefs.magnification);

    // Glass strip first (below icons): owns SWCA material at fixed DOCK_H.
    if app.get_webview_window(DOCK_GLASS_LABEL).is_none() {
        let glass_init = r#"
          window.__WH_IS_DOCK_GLASS__ = true;
        "#;
        let glass = WebviewWindowBuilder::new(
            app,
            DOCK_GLASS_LABEL,
            WebviewUrl::App("index.html?window=dock-glass".into()),
        )
        .title("Dock Glass")
        .inner_size(glass_w, DOCK_H)
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
        .initialization_script(glass_init)
        .build()
        .map_err(|e| format!("open dock-glass failed: {e}"))?;
        let _ = glass.set_ignore_cursor_events(true);
        let _ = glass.set_shadow(false);
        apply_saved_material_pub(&glass, state);
        #[cfg(windows)]
        if let Ok(gh) = glass.hwnd() {
            win32_dock_glass_set_round(gh.0 as isize, prefs.corner_radius_px);
        }
    } else if let Some(glass) = app.get_webview_window(DOCK_GLASS_LABEL) {
        let _ = glass.set_ignore_cursor_events(true);
        let _ = glass.set_shadow(false);
        apply_saved_material_pub(&glass, state);
        #[cfg(windows)]
        if let Ok(gh) = glass.hwnd() {
            win32_dock_glass_set_round(gh.0 as isize, prefs.corner_radius_px);
        }
    }

    if let Some(existing) = app.get_webview_window("dock") {
        let _ = existing.set_shadow(false);
        apply_saved_material_pub(&existing, state);
        sync_dock_visual(app, vis);
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
    .title("Dock")
    .inner_size(width, height)
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
    .build()
    .map_err(|e| format!("open dock failed: {e}"))?;

    // Icons layer: clear material (glass sibling owns acrylic).
    let _ = win.set_shadow(false);
    apply_saved_material_pub(&win, state);
    // Snap whole window to shown/hidden rest pose (no CSS half-state).
    sync_dock_visual(app, vis);
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
