//! Host bottom dock — MyDockFinder-style icons, visibility modes, ini import.

mod file_drop;
mod icon;
mod ini;
mod launch;
mod visibility;

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
    /// Custom icon draw scale (1.0 = 100%). Applied in Dock UI only.
    #[serde(default = "default_icon_scale")]
    pub icon_scale: f64,
    /// Horizontal offset in CSS px at resting 32px icon size.
    #[serde(default)]
    pub icon_offset_x: f64,
    /// Vertical offset in CSS px at resting 32px icon size.
    #[serde(default)]
    pub icon_offset_y: f64,
    /// Solid plate behind the glyph (`#RRGGBB` / `transparent`). Empty = auto.
    #[serde(default)]
    pub icon_bg: String,
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
    /// Logical px corner radius for the glass strip (0 = square; max 30 ≈ half of ~DOCK_H).
    /// ≤8: SWCA + DWM corners; ≥9: Composition HostBackdrop + RectangleClip under WebView2.
    #[serde(default = "default_corner_radius_px")]
    pub corner_radius_px: u32,
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
    20
}

fn default_icon_scale() -> f64 {
    1.0
}

fn clamp_icon_scale(v: f64) -> f64 {
    if !v.is_finite() {
        return 1.0;
    }
    v.clamp(0.5, 2.0)
}

fn clamp_icon_offset(v: f64) -> f64 {
    if !v.is_finite() {
        return 0.0;
    }
    v.clamp(-24.0, 24.0)
}

fn normalize_icon_bg(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return String::new();
    }
    let lower = s.to_ascii_lowercase();
    if lower == "transparent" || lower == "none" || lower == "auto" {
        return if lower == "auto" {
            String::new()
        } else {
            "transparent".into()
        };
    }
    // #RGB / #RRGGBB / #RRGGBBAA
    if let Some(hex) = lower.strip_prefix('#') {
        if matches!(hex.len(), 3 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return format!("#{hex}");
        }
    }
    String::new()
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
        self.corner_radius_px = self.corner_radius_px.min(30);
        for it in &mut self.items {
            it.icon_scale = clamp_icon_scale(it.icon_scale);
            it.icon_offset_x = clamp_icon_offset(it.icon_offset_x);
            it.icon_offset_y = clamp_icon_offset(it.icon_offset_y);
            it.icon_bg = normalize_icon_bg(&it.icon_bg);
        }
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
    // Icon files live under `%APPDATA%\window-hub\dock-icons\`; prefs keep paths only.
    let mut stored = prefs.clone();
    for item in &mut stored.items {
        item.icon_png = None;
    }
    let v = serde_json::to_value(stored).map_err(|e| e.to_string())?;
    crate::db::with_conn(|c| crate::db::dock_set(c, &v))
}

/// Persist item list changes (pin / unpin / import) after materializing owned icons.
fn commit_dock_item_prefs(app: &AppHandle, mut prefs: DockPrefs) -> Result<DockPrefs, String> {
    prefs = prefs.normalize();
    icon::ensure_prefs_icons_cached(&mut prefs);
    prefs
        .hidden_item_ids
        .retain(|id| prefs.items.iter().any(|it| it.id == *id));
    let prefs = with_icons(prefs);
    save_dock_prefs(&prefs)?;
    invalidate_dock_layout_cache();
    let mut live = prefs.clone();
    // Strip runtime PNG before compact mutate path re-saves.
    for it in &mut live.items {
        it.icon_png = None;
    }
    if live.enabled {
        dock_compact_and_notify(app, &mut live);
        position_dock_window(app, &live);
    }
    let out = with_icons(live);
    let _ = app.emit("dock-prefs", &out);
    Ok(out)
}

fn with_icons(mut prefs: DockPrefs) -> DockPrefs {
    for item in &mut prefs.items {
        if item.kind == "separator" {
            item.icon_png = None;
            continue;
        }
        // Builtin Start / Trash: Host draws SVG unless the user picked a custom icon_path.
        if (item.kind == "startmenu" || item.kind == "trash") && item.icon_path.trim().is_empty() {
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
            icon_scale: 1.0,
            icon_offset_x: 0.0,
            icon_offset_y: 0.0,
            icon_bg: String::new(),
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
                icon_scale: 1.0,
                icon_offset_x: 0.0,
                icon_offset_y: 0.0,
                icon_bg: String::new(),
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
        if dock_expanded_width(&layout, prefs.corner_radius_px) <= budget {
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

pub(crate) const DOCK_H: f64 = 52.0;
const DOCK_ICON: f64 = 40.0;
const DOCK_GAP: f64 = 6.0;
const DOCK_PAD_X_MIN: f64 = 2.0;
/// Hit/layout width for separators (1px rule centered). Keep in sync with DockApp.css.
const DOCK_SEP: f64 = 16.0;
/// Fixed hover magnification (not user-configurable).
pub(crate) const DOCK_MAG_SCALE: f64 = 1.6;
/// Fixed total extra logical width when hovering (not dynamically measured).
const DOCK_FAN_EXTRA: f64 = 48.0;
const DOCK_GLASS_LABEL: &str = "dock-glass";

/// Horizontal inset so icon plates stay inside the capsule flat (large radius
/// otherwise clips through the rounded glass silhouette).
pub(crate) fn dock_pad_x(corner_radius_px: u32) -> f64 {
    ((corner_radius_px as f64) * 0.5)
        .clamp(DOCK_PAD_X_MIN, 16.0)
}

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
pub(crate) fn dock_content_width(items: &[DockItem], corner_radius_px: u32) -> f64 {
    let mut w = dock_pad_x(corner_radius_px) * 2.0;
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
pub(crate) fn dock_expanded_width(items: &[DockItem], corner_radius_px: u32) -> f64 {
    dock_content_width(items, corner_radius_px) + DOCK_FAN_EXTRA
}

/// Outer HWND width for icons/glass.
///
/// Composition path: always `content + FAN_EXTRA` so hover widen never calls
/// `SetWindowPos` (DWM size-then-x causes left-then-recenter). Rest/expand is
/// only the frosted capsule Size/Offset inside that fixed host.
pub(crate) fn dock_window_width(
    items: &[DockItem],
    corner_radius_px: u32,
    expanded: bool,
) -> f64 {
    #[cfg(windows)]
    if crate::win32::dock_comp::uses_composition(corner_radius_px) {
        let _ = expanded;
        return dock_expanded_width(items, corner_radius_px);
    }
    if expanded {
        dock_expanded_width(items, corner_radius_px)
    } else {
        dock_content_width(items, corner_radius_px)
    }
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
    // Keep icons + glass the same width; honor hover-expand so place/relayout
    // does not yank the bar back to rest mid-hover (icons leaked past glass).
    let width = dock_window_width(&layout, prefs.corner_radius_px, dock_hover_expanded());
    let glass_w = width;
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

/// Generation for in-flight width tweens — a newer expand/collapse cancels the old one.
#[cfg(windows)]
fn width_tween_gen() -> &'static std::sync::atomic::AtomicU64 {
    static GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    &GEN
}

/// Keep AutoHide from hiding while the dock UI holds an interaction (icon dnd).
#[tauri::command]
pub fn dock_set_interaction_hold(vis: State<'_, Arc<DockVisibility>>, hold: bool) {
    vis.set_interaction_hold(hold);
}

/// Toggle hover pad: Composition animates capsule only (HWND already host-sized).
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
    let prefs = load_dock_prefs();
    let layout = dock_layout_items(&prefs);
    let content_w = dock_content_width(&layout, prefs.corner_radius_px);
    let host_w = dock_expanded_width(&layout, prefs.corner_radius_px);
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
            let raw = hwnd.0 as isize;
            // Skip (and do not bump gen) when already settled at the target pose.
            if let Some((mi, scale, _y, _cur_w)) = win32_dock_width_context(raw) {
                let _ = mi;
                let content_px = (content_w * scale).round().max(1.0) as f32;
                let host_px = (host_w * scale).round().max(1.0) as f32;
                if dock_hover_expanded() == expanded
                    && !crate::win32::dock_comp::width_tween_active()
                    && crate::win32::dock_comp::capsule_near_target(expanded, content_px, host_px)
                {
                    return true;
                }
            }
            let gen = width_tween_gen().fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            set_hover_expanded(expanded);
            let _ = win32_dock_tween_pair_width(
                raw,
                glass_hwnd,
                content_w,
                host_w,
                logical_h,
                prefs.corner_radius_px,
                expanded,
                gen,
            );
            return true;
        }
    }

    #[cfg(not(windows))]
    if dock_hover_expanded() == expanded {
        return true;
    }

    // Non-Windows / no hwnd: fall back to resizing to host or content.
    let logical_w = if expanded { host_w } else { content_w };
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
    let rest = dock_content_width(&layout, prefs.corner_radius_px);
    let expanded = width.is_finite() && width > rest + DOCK_FAN_EXTRA * 0.5;
    dock_set_hover_expand(app, vis, expanded)
}

/// Cursor position in the dock icons webview client space (CSS px), or `None`
/// if the cursor is outside the dock HWND. Used to resume fan after AutoHide
/// show (window slides under a stationary pointer — no pointerenter).
#[tauri::command]
pub fn dock_pointer_client_xy(app: AppHandle) -> Option<(f64, f64)> {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{POINT, RECT};
        use windows::Win32::Graphics::Gdi::ScreenToClient;
        use windows::Win32::UI::HiDpi::GetDpiForWindow;
        use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetCursorPos};

        let win = app.get_webview_window("dock")?;
        let hwnd = win.hwnd().ok()?;
        let hwnd = dock_root_hwnd(hwnd.0 as isize);
        unsafe {
            let mut pt = POINT::default();
            if GetCursorPos(&mut pt).is_err() {
                return None;
            }
            let mut client = pt;
            if !ScreenToClient(hwnd, &mut client).as_bool() {
                return None;
            }
            let mut rc = RECT::default();
            if GetClientRect(hwnd, &mut rc).is_err() {
                return None;
            }
            if client.x < rc.left
                || client.y < rc.top
                || client.x >= rc.right
                || client.y >= rc.bottom
            {
                return None;
            }
            let dpi = GetDpiForWindow(hwnd).max(96) as f64;
            let scale = dpi / 96.0;
            return Some((client.x as f64 / scale, client.y as f64 / scale));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        None
    }
}

/// Read monitor + scale + current Y for width place/tween.
#[cfg(windows)]
fn win32_dock_width_context(
    icons_hwnd_raw: isize,
) -> Option<(
    windows::Win32::Graphics::Gdi::MONITORINFO,
    f64,
    i32,
    i32,
)> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    let icons = dock_root_hwnd(icons_hwnd_raw);
    unsafe {
        let mut wr = RECT::default();
        if GetWindowRect(icons, &mut wr).is_err() {
            return None;
        }
        let mon = MonitorFromWindow(icons, MONITOR_DEFAULTTONEAREST);
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
        let cur_w = (wr.right - wr.left).max(1);
        Some((mi, scale, wr.top, cur_w))
    }
}

/// Expand/collapse visual width. Composition: GPU keyframe capsule Size/Offset
/// (HWND stays host-sized). SWCA: one centered HWND snap (no mid-flight SetWindowPos).
#[cfg(windows)]
fn win32_dock_tween_pair_width(
    icons_hwnd_raw: isize,
    glass_hwnd_raw: Option<isize>,
    content_logical: f64,
    host_logical: f64,
    logical_h: f64,
    corner_radius_px: u32,
    expanded: bool,
    gen: u64,
) -> bool {
    let Some((mi, scale, y, cur_phys_w)) = win32_dock_width_context(icons_hwnd_raw) else {
        return false;
    };
    let content_px = (content_logical * scale).round().max(1.0) as i32;
    let host_px = (host_logical * scale).round().max(1.0) as i32;
    let phys_h = (logical_h * scale).round().max(1.0) as i32;
    let glass_h = (DOCK_H * scale).round().max(1.0) as i32;
    let use_comp = crate::win32::dock_comp::uses_composition(corner_radius_px);

    if use_comp {
        let Some(raw) = glass_hwnd_raw else {
            return false;
        };
        // Ensure host HWND is already content+pad (relayout may have left it short).
        if (cur_phys_w - host_px).abs() > 2 {
            let _ = win32_dock_place_pair_width(
                icons_hwnd_raw,
                glass_hwnd_raw,
                host_px,
                phys_h,
                glass_h,
                corner_radius_px,
                &mi,
                y,
                false,
            );
            // Seed resting capsule immediately so we never paint a full-bleed host.
            win32_dock_set_capsule(
                raw,
                content_px as f32,
                glass_h as f32,
                ((host_px - content_px) as f32 * 0.5).max(0.0),
                corner_radius_px,
                true,
            );
        }

        let to_w = if expanded {
            host_px as f32
        } else {
            content_px as f32
        };
        let host_f = host_px as f32;

        crate::win32::dock_comp::begin_width_tween();
        let gh = dock_root_hwnd(raw);
        // Animate from live visual Size → to_w (no assumed from snap).
        let started = crate::win32::dock_comp::sync_animate_capsule_width(
            gh,
            to_w,
            glass_h as f32,
            host_f,
            corner_radius_px,
        );
        let tween_ms = match &started {
            Ok(ms) => *ms,
            Err(_) => {
                let ox = ((host_f - to_w) * 0.5).max(0.0);
                win32_dock_set_capsule(raw, to_w, glass_h as f32, ox, corner_radius_px, true);
                win32_dock_icons_set_round(icons_hwnd_raw, corner_radius_px);
                crate::win32::dock_comp::end_width_tween();
                return true;
            }
        };
        if tween_ms == 0 {
            win32_dock_icons_set_round(icons_hwnd_raw, corner_radius_px);
            crate::win32::dock_comp::end_width_tween();
            return true;
        }

        let icons = icons_hwnd_raw;
        let glass_raw = raw;
        let to = to_w;
        let gh_f = glass_h as f32;
        let host = host_f;
        let radius = corner_radius_px;
        std::thread::spawn(move || {
            let slice = std::time::Duration::from_millis(16);
            let deadline =
                std::time::Instant::now() + std::time::Duration::from_millis(tween_ms);
            while std::time::Instant::now() < deadline {
                if width_tween_gen().load(std::sync::atomic::Ordering::Relaxed) != gen {
                    return;
                }
                std::thread::sleep(slice);
            }
            if width_tween_gen().load(std::sync::atomic::Ordering::Relaxed) != gen {
                return;
            }
            let ox = ((host - to) * 0.5).max(0.0);
            win32_dock_set_capsule(glass_raw, to, gh_f, ox, radius, true);
            win32_dock_icons_set_round(icons, radius);
            crate::win32::dock_comp::end_width_tween();
        });
        return true;
    }

    // SWCA: HWND must change — one centered snap only.
    let target = if expanded { host_px } else { content_px };
    win32_dock_place_pair_width(
        icons_hwnd_raw,
        glass_hwnd_raw,
        target,
        phys_h,
        glass_h,
        corner_radius_px,
        &mi,
        y,
        true,
    )
}

/// Set Composition capsule size/offset. `wait` seeds on the UI thread before paint.
#[cfg(windows)]
fn win32_dock_set_capsule(
    glass_hwnd_raw: isize,
    width_px: f32,
    height_px: f32,
    offset_x: f32,
    corner_radius_px: u32,
    wait: bool,
) {
    // Persist before paint so frost re-attach cannot wipe rest → full-bleed.
    crate::win32::dock_comp::remember_capsule(width_px, height_px, offset_x);
    let gh = dock_root_hwnd(glass_hwnd_raw);
    if wait {
        let _ = crate::win32::dock_comp::sync_layout_tween_frame_wait(
            gh,
            width_px,
            height_px,
            offset_x,
            corner_radius_px,
        );
    } else {
        let _ = crate::win32::dock_comp::sync_layout_tween_frame(
            gh,
            width_px,
            height_px,
            offset_x,
            corner_radius_px,
        );
    }
}

/// Remember + apply rest capsule from current glass client size (pre-frost).
#[cfg(windows)]
fn win32_dock_seed_rest_capsule(glass_hwnd_raw: isize, corner_radius_px: u32) {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
    let hwnd = dock_root_hwnd(glass_hwnd_raw);
    let mut rc = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut rc) }.is_err() {
        return;
    }
    let hw = (rc.right - rc.left).max(1);
    let hh = (rc.bottom - rc.top).max(1);
    // Force rest pose for seed (open must not look pre-widened).
    let was = dock_hover_expanded();
    set_hover_expanded(false);
    win32_dock_sync_capsule_after_place(glass_hwnd_raw, hw, hh, corner_radius_px);
    set_hover_expanded(was);
}

/// After placing the host HWND, sync capsule to rest (inset) or hover (full).
#[cfg(windows)]
fn win32_dock_sync_capsule_after_place(
    glass_hwnd_raw: isize,
    host_phys_w: i32,
    glass_h: i32,
    corner_radius_px: u32,
) {
    use windows::Win32::UI::HiDpi::GetDpiForWindow;

    if !crate::win32::dock_comp::uses_composition(corner_radius_px) {
        return;
    }
    let gh = dock_root_hwnd(glass_hwnd_raw);
    let scale = unsafe {
        let dpi = GetDpiForWindow(gh);
        if dpi > 0 {
            dpi as f64 / 96.0
        } else {
            1.0
        }
    };
    let prefs = load_dock_prefs();
    let layout = dock_layout_items(&prefs);
    let content_px =
        (dock_content_width(&layout, corner_radius_px) * scale).round().max(1.0) as f32;
    let host_f = host_phys_w as f32;
    if dock_hover_expanded() {
        win32_dock_set_capsule(
            glass_hwnd_raw,
            host_f,
            glass_h as f32,
            0.0,
            corner_radius_px,
            false,
        );
    } else {
        let ox = ((host_f - content_px) * 0.5).max(0.0);
        win32_dock_set_capsule(
            glass_hwnd_raw,
            content_px,
            glass_h as f32,
            ox,
            corner_radius_px,
            false,
        );
    }
}

/// Place icons + glass at the same centered physical width (atomic DeferWindowPos).
/// `finalize`: refresh Composition frost + icon region (skip on tween frames).
#[cfg(windows)]
fn win32_dock_place_pair_width(
    icons_hwnd_raw: isize,
    glass_hwnd_raw: Option<isize>,
    phys_w: i32,
    phys_h: i32,
    glass_h: i32,
    corner_radius_px: u32,
    mi: &windows::Win32::Graphics::Gdi::MONITORINFO,
    y: i32,
    finalize: bool,
) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        BeginDeferWindowPos, DeferWindowPos, EndDeferWindowPos, SetWindowPos, SWP_NOACTIVATE,
        SWP_NOCOPYBITS, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    };
    let icons = dock_root_hwnd(icons_hwnd_raw);
    let mon_w = mi.rcMonitor.right - mi.rcMonitor.left;
    let x = mi.rcMonitor.left + ((mon_w - phys_w) / 2).max(0);
    let flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS;
    unsafe {
        if let Some(raw) = glass_hwnd_raw {
            let gh = dock_root_hwnd(raw);
            let gy = y + phys_h - glass_h;
            // Apply x+w together so DWM cannot flash “widen right, then shift”.
            if let Ok(hdwp) = BeginDeferWindowPos(2) {
                let hdwp = DeferWindowPos(hdwp, icons, None, x, y, phys_w, phys_h, flags)
                    .unwrap_or(hdwp);
                let hdwp = DeferWindowPos(hdwp, gh, None, x, gy, phys_w, glass_h, flags)
                    .unwrap_or(hdwp);
                let _ = EndDeferWindowPos(hdwp);
            } else {
                let _ = SetWindowPos(icons, None, x, y, phys_w, phys_h, flags);
                let _ = SetWindowPos(gh, None, x, gy, phys_w, glass_h, flags);
            }
            let _ = SetWindowPos(
                gh,
                icons,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            if finalize {
                // set_round seeds rest/hover capsule before+after frost attach.
                win32_dock_glass_set_round_sized(
                    raw,
                    corner_radius_px,
                    Some((phys_w as f32, glass_h as f32)),
                );
                win32_dock_icons_set_round(icons_hwnd_raw, corner_radius_px);
                crate::win32::blur_glass::schedule_dock_titlebar_strip(icons.0 as isize);
                crate::win32::blur_glass::schedule_dock_titlebar_strip(gh.0 as isize);
            }
        } else {
            let _ = SetWindowPos(icons, None, x, y, phys_w, phys_h, flags);
            if finalize {
                win32_dock_icons_set_round(icons_hwnd_raw, corner_radius_px);
            }
        }
    }
    true
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

/// Clip the icons HWND to a capsule matching glass (bottom corners only so
/// fan headroom is not shaved by top rounding). Prevents glyphs poking past
/// large Composition radii on a square transparent window.
#[cfg(windows)]
fn win32_dock_icons_set_round(hwnd_raw: isize, corner_radius_px: u32) {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{
        CombineRgn, CreateRectRgn, CreateRoundRectRgn, DeleteObject, SetWindowRgn, RGN_ERROR,
        RGN_OR,
    };
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

    let hwnd = dock_root_hwnd(hwnd_raw);
    unsafe {
        if corner_radius_px == 0 {
            let _ = SetWindowRgn(hwnd, None, true);
            return;
        }
        let mut rc = RECT::default();
        if GetClientRect(hwnd, &mut rc).is_err() {
            return;
        }
        let w = rc.right - rc.left;
        let h = rc.bottom - rc.top;
        if w <= 1 || h <= 1 {
            return;
        }
        let dpi = GetDpiForWindow(hwnd);
        let scale = if dpi > 0 {
            dpi as f64 / 96.0
        } else {
            1.0
        };
        let glass_h = (DOCK_H * scale).round().max(1.0) as i32;
        let r = ((corner_radius_px as f64) * scale).round().max(1.0) as i32;
        let ell = (r * 2).clamp(2, w.min(glass_h).max(2));
        let chrome_top = (h - glass_h).max(0);
        // Square headroom + upper chrome, OR bottom rounded chrome strip.
        let top = CreateRectRgn(0, 0, w + 1, chrome_top + r);
        let bottom = CreateRoundRectRgn(0, chrome_top, w + 1, h + 1, ell, ell);
        let combined = CreateRectRgn(0, 0, 0, 0);
        if top.is_invalid() || bottom.is_invalid() || combined.is_invalid() {
            if !top.is_invalid() {
                let _ = DeleteObject(top);
            }
            if !bottom.is_invalid() {
                let _ = DeleteObject(bottom);
            }
            if !combined.is_invalid() {
                let _ = DeleteObject(combined);
            }
            return;
        }
        if CombineRgn(combined, top, bottom, RGN_OR) == RGN_ERROR {
            let _ = DeleteObject(top);
            let _ = DeleteObject(bottom);
            let _ = DeleteObject(combined);
            return;
        }
        let _ = DeleteObject(top);
        let _ = DeleteObject(bottom);
        // SetWindowRgn takes ownership of `combined`.
        let _ = SetWindowRgn(hwnd, combined, true);
    }
}

/// Refresh glass corners after place/resize (Composition clip tracks HWND size).
#[cfg(windows)]
fn win32_dock_glass_set_round(hwnd_raw: isize, corner_radius_px: u32) {
    win32_dock_glass_set_round_sized(hwnd_raw, corner_radius_px, None);
}

#[cfg(windows)]
fn win32_dock_glass_set_round_sized(
    hwnd_raw: isize,
    corner_radius_px: u32,
    size_px: Option<(f32, f32)>,
) {
    let hwnd = dock_root_hwnd(hwnd_raw);
    // Prefer seeding rest pose before frost when host size is known.
    if let Some((hw, hh)) = size_px {
        win32_dock_sync_capsule_after_place(hwnd_raw, hw.round() as i32, hh.round() as i32, corner_radius_px);
    }
    crate::win32::blur_glass::apply_dock_glass_round_frost_sized_pub(
        hwnd,
        corner_radius_px,
        size_px,
    );
    // Frost attach may run async — re-assert rest/hover capsule afterward.
    if let Some((hw, hh)) = size_px {
        win32_dock_sync_capsule_after_place(hwnd_raw, hw.round() as i32, hh.round() as i32, corner_radius_px);
    } else {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
        let mut rc = RECT::default();
        if unsafe { GetClientRect(hwnd, &mut rc) }.is_ok() {
            let hw = (rc.right - rc.left).max(1);
            let hh = (rc.bottom - rc.top).max(1);
            win32_dock_sync_capsule_after_place(hwnd_raw, hw, hh, corner_radius_px);
        }
    }
}

/// Ensure the dock can be hit-tested (auto-hide keep-alive + clicks).
#[cfg(windows)]
fn win32_dock_clear_transparent(hwnd_raw: isize) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, SetWindowLongW, SetWindowPos, GWL_EXSTYLE, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_EX_TRANSPARENT,
    };
    let hwnd = dock_root_hwnd(hwnd_raw);
    // Focus / long-press can reintroduce a native caption into headroom.
    crate::win32::blur_glass::schedule_dock_titlebar_strip(hwnd.0 as isize);
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
        // Always match icons silhouette to current glass radius.
        // (icons hwnd is `hwnd` here — root of the icons webview.)
        win32_dock_icons_set_round(hwnd.0 as isize, corner_radius_px);
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
            crate::win32::blur_glass::schedule_dock_titlebar_strip(hwnd.0 as isize);
            if let Some(gh) = glass {
                crate::win32::blur_glass::schedule_dock_titlebar_strip(gh.0 as isize);
            }
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
    let mut prefs = load_dock_prefs();
    // Lazy migrate: older imports only stored MyDockFinder paths — copy into dock-icons/.
    let before: Vec<String> = prefs.items.iter().map(|i| i.icon_path.clone()).collect();
    icon::ensure_prefs_icons_cached(&mut prefs);
    if prefs
        .items
        .iter()
        .zip(before.iter())
        .any(|(i, b)| i.icon_path != *b)
    {
        let _ = save_dock_prefs(&prefs);
        invalidate_dock_layout_cache();
    }
    with_icons(prefs)
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
    // Materialize any external / MyDockFinder paths into owned dock-icons/.
    icon::ensure_prefs_icons_cached(&mut next);
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
    // Copy icopath / shell icons into `%APPDATA%\window-hub\dock-icons` so prefs
    // no longer depend on MyDockFinder backup paths.
    commit_dock_item_prefs(&app, prefs)
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
pub fn pick_dock_icon_file() -> Result<Option<String>, String> {
    let file = rfd::FileDialog::new()
        .add_filter("图标 / 图片", &["ico", "png", "jpg", "jpeg", "bmp", "webp", "exe", "dll"])
        .set_title("选择 Dock 图标")
        .pick_file();
    Ok(file.map(|p| p.to_string_lossy().to_string()))
}

/// Copy a picked / dropped icon into the owned `dock-icons` store for `item_id`.
#[tauri::command]
pub fn dock_cache_icon(item_id: String, source_path: String) -> Result<String, String> {
    icon::cache_icon_from_source(&item_id, &source_path)
}

fn path_pin_id(path: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.trim().to_ascii_lowercase().hash(&mut h);
    format!("pin-{:016x}", h.finish())
}

fn match_exe_from_path(path: &std::path::Path) -> String {
    let stem = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("app")
        .to_string();
    let lower = stem.to_ascii_lowercase();
    if lower.ends_with(".exe") {
        lower
    } else if lower.ends_with(".lnk") {
        format!(
            "{}.exe",
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("app")
                .to_ascii_lowercase()
        )
    } else {
        format!("{lower}.exe")
    }
}

fn label_from_path(path: &std::path::Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("App")
        .to_string()
}

#[cfg(windows)]
fn resolve_shortcut_target(path: &std::path::Path) -> Option<std::path::PathBuf> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{Interface, PCWSTR};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED, STGM_READ,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext != "lnk" {
        return None;
    }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let persist: IPersistFile = link.cast().ok()?;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        persist.Load(PCWSTR(wide.as_ptr()), STGM_READ).ok()?;
        let mut buf = [0u16; 520];
        link.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        if len == 0 {
            return None;
        }
        let s = String::from_utf16_lossy(&buf[..len]);
        let p = std::path::PathBuf::from(s.trim());
        if p.as_os_str().is_empty() {
            None
        } else {
            Some(p)
        }
    }
}

#[cfg(not(windows))]
fn resolve_shortcut_target(_path: &std::path::Path) -> Option<std::path::PathBuf> {
    None
}

fn dock_item_from_path(path_raw: &str) -> Result<DockItem, String> {
    let trimmed = path_raw.trim().trim_matches('"');
    if trimmed.is_empty() {
        return Err("empty path".into());
    }
    let path = std::path::PathBuf::from(trimmed);
    if !path.exists() {
        return Err(format!("path not found: {trimmed}"));
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(
        ext.as_str(),
        "exe" | "lnk" | "url" | "msc" | "bat" | "cmd" | "com"
    ) {
        // Allow bare files that are applications without extension (rare) only if .exe-like.
        return Err(format!("unsupported pin type: .{ext}"));
    }

    let target = resolve_shortcut_target(&path).unwrap_or_else(|| path.clone());
    let launch = path.to_string_lossy().replace('/', "\\");
    let real = target.to_string_lossy().replace('/', "\\");
    let id = path_pin_id(&real);
    let match_exe = match_exe_from_path(&target);
    let label = label_from_path(if ext == "lnk" { &path } else { &target });

    let mut item = DockItem {
        id,
        kind: "app".into(),
        label,
        match_exe,
        launch_path: launch.clone(),
        real_path: real.clone(),
        virtual_path: String::new(),
        // Prefer shell icon from shortcut / exe; materialize writes owned PNG.
        icon_path: launch,
        uwp: false,
        icon_png: None,
        icon_scale: 1.0,
        icon_offset_x: 0.0,
        icon_offset_y: 0.0,
        icon_bg: String::new(),
    };
    icon::ensure_item_icon_cached(&mut item);
    Ok(item)
}

fn insert_pin_before_trash(items: &mut Vec<DockItem>, item: DockItem) {
    let launch_key = item.launch_path.trim().to_ascii_lowercase();
    let real_key = item.real_path.trim().to_ascii_lowercase();
    let match_key = item.match_exe.trim().to_ascii_lowercase();
    items.retain(|it| {
        if it.kind != "app" {
            return true;
        }
        let l = it.launch_path.trim().to_ascii_lowercase();
        let r = it.real_path.trim().to_ascii_lowercase();
        let m = it.match_exe.trim().to_ascii_lowercase();
        !(l == launch_key
            || (!real_key.is_empty() && (r == real_key || l == real_key))
            || (!match_key.is_empty() && m == match_key && !m.is_empty()))
    });
    if let Some(idx) = items.iter().position(|it| it.kind == "trash") {
        items.insert(idx, item);
    } else {
        items.push(item);
    }
}

/// Drop `.exe` / `.lnk` (and similar) onto the dock to pin them.
#[tauri::command]
pub fn dock_pin_paths(app: AppHandle, paths: Vec<String>) -> Result<DockPrefs, String> {
    let mut prefs = load_dock_prefs();
    let mut added = 0usize;
    for raw in paths {
        match dock_item_from_path(&raw) {
            Ok(item) => {
                insert_pin_before_trash(&mut prefs.items, item);
                added += 1;
            }
            Err(e) => {
                eprintln!("[dock] pin skip {raw}: {e}");
            }
        }
    }
    if added == 0 {
        return Err("没有可固定到 Dock 的程序（请拖入 .exe / .lnk）".into());
    }
    commit_dock_item_prefs(&app, prefs)
}

fn new_dock_separator() -> DockItem {
    let id = format!(
        "sep-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    DockItem {
        id,
        kind: "separator".into(),
        label: String::new(),
        match_exe: String::new(),
        launch_path: String::new(),
        real_path: String::new(),
        virtual_path: String::new(),
        icon_path: String::new(),
        uwp: false,
        icon_png: None,
        icon_scale: 1.0,
        icon_offset_x: 0.0,
        icon_offset_y: 0.0,
        icon_bg: String::new(),
    }
}

fn insert_separator_after(items: &mut Vec<DockItem>, after_item_id: Option<&str>) {
    let sep = new_dock_separator();
    if let Some(aid) = after_item_id.map(str::trim).filter(|s| !s.is_empty()) {
        // Only pinned tiles (never ephemeral running:* / running-sep).
        if !aid.starts_with("running:") && aid != "running-sep" {
            if let Some(idx) = items.iter().position(|it| it.id == aid) {
                // Never place after trash — that looks like “stuck at the end”.
                if items[idx].kind == "trash" {
                    items.insert(idx, sep);
                } else {
                    items.insert((idx + 1).min(items.len()), sep);
                }
                return;
            }
        }
    }
    // No usable anchor → after Start (or front). Avoid defaulting to before-trash
    // (“always appears at the end”).
    if let Some(idx) = items.iter().position(|it| it.kind == "startmenu") {
        items.insert(idx + 1, sep);
    } else if let Some(idx) = items.iter().position(|it| it.kind == "trash") {
        items.insert(idx, sep);
    } else {
        items.insert(0, sep);
    }
}

/// Insert a user separator after a pinned tile (or before trash when `after_item_id` is empty).
#[tauri::command]
pub fn dock_add_separator(
    app: AppHandle,
    after_item_id: Option<String>,
) -> Result<DockPrefs, String> {
    let mut prefs = load_dock_prefs();
    insert_separator_after(&mut prefs.items, after_item_id.as_deref());
    commit_dock_item_prefs(&app, prefs)
}

/// Persist a new pin order (ids of `prefs.items`). Unknown ids ignored; missing pins appended.
#[tauri::command]
pub fn dock_reorder_items(app: AppHandle, ordered_ids: Vec<String>) -> Result<DockPrefs, String> {
    let mut prefs = load_dock_prefs();
    if ordered_ids.is_empty() {
        return Ok(with_icons(prefs));
    }
    let mut by_id: std::collections::HashMap<String, DockItem> = prefs
        .items
        .drain(..)
        .map(|it| (it.id.clone(), it))
        .collect();
    let mut next = Vec::with_capacity(by_id.len());
    for id in ordered_ids {
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        if let Some(item) = by_id.remove(id) {
            next.push(item);
        }
    }
    // Keep any pins the client omitted (should be rare).
    for (_, item) in by_id {
        next.push(item);
    }
    prefs.items = next;
    prefs = prefs.normalize();
    // Reorder does not change pin count / bar width — skip icon encode, HWND
    // place, and compact. Those were hitching the dock on drop.
    save_dock_prefs(&prefs)?;
    invalidate_dock_layout_cache();
    let mut out = prefs;
    for it in &mut out.items {
        it.icon_png = None;
    }
    let _ = app.emit("dock-prefs", &out);
    Ok(out)
}

/// Pin a display tile (typically `running:…`) into the fixed dock list.
#[tauri::command]
pub fn dock_pin_item(app: AppHandle, item_id: String) -> Result<DockPrefs, String> {
    let id = item_id.trim().to_string();
    if id.is_empty() {
        return Err("item_id required".into());
    }
    let prefs = load_dock_prefs();
    let layout = dock_merge_running(&prefs, false);
    let item = layout
        .iter()
        .find(|it| it.id == id)
        .cloned()
        .ok_or_else(|| format!("dock item not found: {id}"))?;
    if item.kind != "app" {
        return Err("只能固定应用程序".into());
    }
    let path = if !item.real_path.trim().is_empty() {
        item.real_path.clone()
    } else if !item.launch_path.trim().is_empty() {
        item.launch_path.clone()
    } else {
        return Err("无法解析程序路径".into());
    };
    // Prefer building from path so icon cache + stable pin-id apply.
    match dock_item_from_path(&path) {
        Ok(mut pin) => {
            if !item.label.trim().is_empty() {
                pin.label = item.label;
            }
            if !item.match_exe.trim().is_empty() {
                pin.match_exe = item.match_exe;
            }
            let mut next = prefs;
            insert_pin_before_trash(&mut next.items, pin);
            commit_dock_item_prefs(&app, next)
        }
        Err(_) => {
            // Fallback: keep the ephemeral tile fields but give a stable pin id.
            let mut pin = item;
            pin.id = path_pin_id(&path);
            icon::ensure_item_icon_cached(&mut pin);
            let mut next = prefs;
            insert_pin_before_trash(&mut next.items, pin);
            commit_dock_item_prefs(&app, next)
        }
    }
}

/// Remove a pinned dock item (not Start / Trash / ephemeral running tiles).
#[tauri::command]
pub fn dock_unpin_item(app: AppHandle, item_id: String) -> Result<DockPrefs, String> {
    let id = item_id.trim().to_string();
    if id.is_empty() {
        return Err("item_id required".into());
    }
    if id.starts_with("running:") {
        return Err("运行中图标未固定，无需移除".into());
    }
    let mut prefs = load_dock_prefs();
    let Some(item) = prefs.items.iter().find(|it| it.id == id).cloned() else {
        return Err(format!("dock item not found: {id}"));
    };
    if item.kind == "startmenu" || item.kind == "trash" {
        return Err("开始菜单 / 回收站不可移除".into());
    }
    prefs.items.retain(|it| it.id != id);
    prefs.hidden_item_ids.retain(|h| h != &id);
    // Best-effort: remove owned icon file for this id.
    if let Ok(dir) = icon::icons_dir() {
        let stem = id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let file = dir.join(format!("{stem}.png"));
        let _ = std::fs::remove_file(file);
    }
    commit_dock_item_prefs(&app, prefs)
}

/// Settings-styled icon editor (left list + right pane).
#[tauri::command]
pub async fn open_dock_icon_editor(
    app: AppHandle,
    state: State<'_, MaterialState>,
    item_id: Option<String>,
) -> Result<(), String> {
    let focus = item_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(existing) = app.get_webview_window("dock-icon-editor") {
        apply_saved_material_pub(&existing, &state);
        let _ = existing.unminimize();
        let _ = existing.show();
        let _ = existing.set_focus();
        if let Some(id) = focus {
            let _ = app.emit("dock-icon-editor-focus", id);
        }
        return Ok(());
    }

    let focus_js = focus
        .as_ref()
        .map(|id| {
            format!(
                "window.__WH_DOCK_ICON_EDITOR_FOCUS__ = {};",
                serde_json::to_string(id).unwrap_or_else(|_| "null".into())
            )
        })
        .unwrap_or_default();

    let init = format!(
        r#"
      window.__WH_IS_DOCK_ICON_EDITOR__ = true;
      {focus_js}
      document.addEventListener('keydown', function (e) {{
        if (e.key === 'Escape') {{
          try {{ window.__TAURI__.core.invoke('close_dock_icon_editor'); }} catch (_) {{}}
        }}
      }});
    "#
    );

    let win = WebviewWindowBuilder::new(
        &app,
        "dock-icon-editor",
        WebviewUrl::App("index.html?window=dock-icon-editor".into()),
    )
    .title("修改图标")
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
    .map_err(|e| format!("open dock icon editor failed: {e}"))?;

    let _ = win.show();
    apply_saved_material_pub(&win, &state);
    let _ = win.set_focus();
    Ok(())
}

#[tauri::command]
pub async fn close_dock_icon_editor(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("dock-icon-editor") {
        w.close().map_err(|e| e.to_string())?;
    }
    Ok(())
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
    let width = dock_window_width(&layout, prefs.corner_radius_px, dock_hover_expanded());
    let glass_w = width;
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
        .title("")
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
        // Seed rest capsule before material frost (host HWND is already wide).
        #[cfg(windows)]
        if let Ok(gh) = glass.hwnd() {
            win32_dock_seed_rest_capsule(gh.0 as isize, prefs.corner_radius_px);
        }
        apply_saved_material_pub(&glass, state);
        #[cfg(windows)]
        if let Ok(gh) = glass.hwnd() {
            win32_dock_glass_set_round(gh.0 as isize, prefs.corner_radius_px);
        }
    } else if let Some(glass) = app.get_webview_window(DOCK_GLASS_LABEL) {
        let _ = glass.set_ignore_cursor_events(true);
        let _ = glass.set_shadow(false);
        #[cfg(windows)]
        if let Ok(gh) = glass.hwnd() {
            win32_dock_seed_rest_capsule(gh.0 as isize, prefs.corner_radius_px);
        }
        apply_saved_material_pub(&glass, state);
        #[cfg(windows)]
        if let Ok(gh) = glass.hwnd() {
            win32_dock_glass_set_round(gh.0 as isize, prefs.corner_radius_px);
        }
    }

    if let Some(existing) = app.get_webview_window("dock") {
        let _ = existing.set_shadow(false);
        apply_saved_material_pub(&existing, state);
        #[cfg(windows)]
        if let Ok(hwnd) = existing.hwnd() {
            file_drop::install_dock_file_drop(app, hwnd.0 as isize);
        }
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
    .title("")
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
    // Use our OLE target in `file_drop` — wry's handler leaves a no-drop cursor
    // on this frameless HWND and blocks reliable HTML5 / pointer DnD.
    .disable_drag_drop_handler()
    .initialization_script(init)
    .build()
    .map_err(|e| format!("open dock failed: {e}"))?;

    // Icons layer: clear material (glass sibling owns acrylic).
    let _ = win.set_shadow(false);
    apply_saved_material_pub(&win, state);
    #[cfg(windows)]
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::blur_glass::schedule_dock_titlebar_strip(hwnd.0 as isize);
        file_drop::install_dock_file_drop(app, hwnd.0 as isize);
        // WebView2 children appear a tick later — re-bind drop targets.
        let app_drop = app.clone();
        let hwnd_raw = hwnd.0 as isize;
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            file_drop::install_dock_file_drop(&app_drop, hwnd_raw);
        });
    }
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
