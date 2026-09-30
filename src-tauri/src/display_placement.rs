//! Multi-monitor placement for top chrome and Dock.
//!
//! Presets: primaryOnly | allDisplays | custom.
//! Island (灵动岛) only on primary when that screen's topBar is `full`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{
    AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder,
};
use tauri::utils::config::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TopBarMode {
    Full,
    Shortcuts,
    None,
}

impl TopBarMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Shortcuts => "shortcuts",
            Self::None => "none",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim() {
            "shortcuts" => Self::Shortcuts,
            "none" => Self::None,
            _ => Self::Full,
        }
    }
}

impl Default for TopBarMode {
    fn default() -> Self {
        Self::Full
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlacementPreset {
    PrimaryOnly,
    AllDisplays,
    Custom,
}

impl PlacementPreset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PrimaryOnly => "primaryOnly",
            Self::AllDisplays => "allDisplays",
            Self::Custom => "custom",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim() {
            "allDisplays" | "all_displays" => Self::AllDisplays,
            "custom" => Self::Custom,
            _ => Self::PrimaryOnly,
        }
    }
}

impl Default for PlacementPreset {
    fn default() -> Self {
        Self::PrimaryOnly
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorPlacement {
    pub id: String,
    #[serde(default)]
    pub top_bar: TopBarMode,
    #[serde(default = "default_true")]
    pub dock: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayPlacementPrefs {
    #[serde(default)]
    pub preset: PlacementPreset,
    /// Persisted only for `custom`; other presets derive at apply time.
    #[serde(default)]
    pub monitors: Vec<MonitorPlacement>,
}

impl Default for DisplayPlacementPrefs {
    fn default() -> Self {
        Self {
            preset: PlacementPreset::PrimaryOnly,
            monitors: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayInfo {
    pub id: String,
    pub name: String,
    pub is_primary: bool,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedPlacement {
    pub id: String,
    pub name: String,
    pub is_primary: bool,
    pub top_bar: TopBarMode,
    pub dock: bool,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlacementSnapshot {
    pub prefs: DisplayPlacementPrefs,
    pub displays: Vec<DisplayInfo>,
    pub resolved: Vec<ResolvedPlacement>,
    pub primary_top_bar: TopBarMode,
}

static APPLYING: AtomicBool = AtomicBool::new(false);
static LAST_SAT_CHROME: Mutex<Vec<String>> = Mutex::new(Vec::new());
static LAST_SAT_DOCK: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn label_hash(id: &str) -> String {
    let mut h: u32 = 2166136261;
    for b in id.as_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(16777619);
    }
    format!("{h:08x}")
}

pub fn chrome_sat_label(id: &str) -> String {
    format!("chrome-sat-{}", label_hash(id))
}

pub fn dock_sat_label(id: &str) -> String {
    format!("dock-sat-{}", label_hash(id))
}

pub fn dock_sat_glass_label(id: &str) -> String {
    format!("dock-sat-glass-{}", label_hash(id))
}

pub fn is_chrome_sat_label(label: &str) -> bool {
    label.starts_with("chrome-sat-")
}

pub fn is_dock_sat_label(label: &str) -> bool {
    label.starts_with("dock-sat-") && !label.starts_with("dock-sat-glass-")
}

pub fn is_dock_sat_glass_label(label: &str) -> bool {
    label.starts_with("dock-sat-glass-")
}

pub fn load() -> DisplayPlacementPrefs {
    let mut prefs = match crate::db::with_conn(|c| crate::db::display_placement_get(c)) {
        Ok(Some(v)) => serde_json::from_value(v).unwrap_or_default(),
        _ => DisplayPlacementPrefs::default(),
    };
    prefs.preset = PlacementPreset::parse(prefs.preset.as_str());
    for m in &mut prefs.monitors {
        m.top_bar = TopBarMode::parse(m.top_bar.as_str());
    }
    prefs
}

/// One monitor: allDisplays/custom are meaningless — force primaryOnly.
fn coerce_single_display(prefs: &mut DisplayPlacementPrefs, display_count: usize) -> bool {
    if display_count <= 1 && prefs.preset != PlacementPreset::PrimaryOnly {
        prefs.preset = PlacementPreset::PrimaryOnly;
        prefs.monitors.clear();
        return true;
    }
    false
}

fn save(prefs: &DisplayPlacementPrefs) -> Result<(), String> {
    let v = serde_json::to_value(prefs).map_err(|e| e.to_string())?;
    crate::db::with_conn(|c| {
        c.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS prefs_display_placement (
              id INTEGER PRIMARY KEY CHECK (id = 1),
              data_json TEXT NOT NULL,
              updated_at INTEGER NOT NULL
            );
            "#,
        )
        .map_err(|e| e.to_string())?;
        crate::db::display_placement_set(c, &v)
    })
}

#[cfg(windows)]
fn utf16_z(buf: &[u16]) -> String {
    String::from_utf16_lossy(&buf.iter().copied().take_while(|c| *c != 0).collect::<Vec<_>>())
        .trim()
        .to_string()
}

/// Extract EDID product id like `LEN9053` from paths such as
/// `\\?\DISPLAY#LEN9053#4&...` or `MONITOR\LEN9053\...`.
#[cfg(windows)]
fn edid_code_from_path(path: &str) -> Option<String> {
    let upper = path.to_ascii_uppercase();
    for marker in ["DISPLAY#", "MONITOR\\", "MONITOR/"] {
        if let Some(rest) = upper.split(marker).nth(1) {
            let code: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            if code.len() >= 3 {
                // Preserve original casing from path when possible.
                let start = path.to_ascii_uppercase().find(marker)? + marker.len();
                let end = start + code.len();
                if end <= path.len() {
                    return Some(path[start..end].to_string());
                }
                return Some(code);
            }
        }
    }
    None
}

fn is_useless_monitor_label(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    lower == "generic pnp monitor"
        || lower.starts_with("generic ")
        || lower == "default monitor"
        || t.starts_with("显示器 ")
        || t.eq_ignore_ascii_case("DISPLAY")
}

/// GDI `\\.\DISPLAYn` → EDID-style friendly name (`LEN9053`, …).
#[cfg(windows)]
fn gdi_monitor_name_map() -> std::collections::HashMap<String, String> {
    use windows::Win32::Devices::Display::{
        DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
        DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
        DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
        DISPLAYCONFIG_TARGET_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
    };

    let mut map = std::collections::HashMap::new();
    unsafe {
        let mut path_count = 0u32;
        let mut mode_count = 0u32;
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
            .is_err()
            || path_count == 0
        {
            return map;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        if QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        )
        .is_err()
        {
            return map;
        }
        paths.truncate(path_count as usize);
        for path in &paths {
            let mut src = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
            src.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            src.header.adapterId = path.sourceInfo.adapterId;
            src.header.id = path.sourceInfo.id;
            src.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            if DisplayConfigGetDeviceInfo(&mut src.header) != 0 {
                continue;
            }
            let gdi = utf16_z(&src.viewGdiDeviceName);
            if gdi.is_empty() {
                continue;
            }

            let mut tgt = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
            tgt.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            tgt.header.adapterId = path.targetInfo.adapterId;
            tgt.header.id = path.targetInfo.id;
            tgt.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            if DisplayConfigGetDeviceInfo(&mut tgt.header) != 0 {
                continue;
            }
            let friendly = utf16_z(&tgt.monitorFriendlyDeviceName);
            let device_path = utf16_z(&tgt.monitorDevicePath);
            let name = if !is_useless_monitor_label(&friendly) {
                friendly
            } else if let Some(code) = edid_code_from_path(&device_path) {
                code
            } else {
                continue;
            };
            map.insert(gdi, name);
        }
    }
    map
}

/// Prefer EDID / DisplayConfig product name (e.g. LEN9053); never「显示器 N」.
#[cfg(windows)]
fn monitor_display_name(
    device: &str,
    w: u32,
    h: u32,
    ccd_names: &std::collections::HashMap<String, String>,
) -> String {
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{EnumDisplayDevicesW, DISPLAY_DEVICEW};

    let fallback = format!("{w}×{h}");
    if device.trim().is_empty() {
        return fallback;
    }
    if let Some(n) = ccd_names.get(device) {
        if !is_useless_monitor_label(n) {
            return n.clone();
        }
    }
    let wide: Vec<u16> = device.encode_utf16().chain(std::iter::once(0)).collect();
    for i in 0..8u32 {
        let mut dd = DISPLAY_DEVICEW {
            cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        let ok = unsafe { EnumDisplayDevicesW(PCWSTR(wide.as_ptr()), i, &mut dd, 0) };
        if !ok.as_bool() {
            break;
        }
        let device_id = utf16_z(&dd.DeviceID);
        if let Some(code) = edid_code_from_path(&device_id) {
            return code;
        }
        let label = utf16_z(&dd.DeviceString);
        if !is_useless_monitor_label(&label) {
            return label;
        }
    }
    fallback
}

#[cfg(windows)]
pub fn list_displays() -> Vec<DisplayInfo> {
    use std::sync::Mutex;
    use windows::Win32::Foundation::{BOOL, LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, HDC, HMONITOR, MONITORINFOEXW,
        MONITOR_DEFAULTTOPRIMARY,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

    struct Acc(Vec<DisplayInfo>);
    static ACC: Mutex<Option<Acc>> = Mutex::new(None);

    unsafe extern "system" fn enum_proc(
        mon: HMONITOR,
        _hdc: HDC,
        _rc: *mut RECT,
        _lp: LPARAM,
    ) -> BOOL {
        let mut info = MONITORINFOEXW {
            monitorInfo: windows::Win32::Graphics::Gdi::MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFOEXW>() as u32,
                ..Default::default()
            },
            ..Default::default()
        };
        // MONITORINFOEXW.cbSize must cover the full EX struct.
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if !GetMonitorInfoW(mon, &mut info as *mut _ as *mut _).as_bool() {
            return BOOL(1);
        }
        let rc = info.monitorInfo.rcMonitor;
        let w = (rc.right - rc.left).max(1) as u32;
        let h = (rc.bottom - rc.top).max(1) as u32;
        let mut dpi_x = 96u32;
        let mut dpi_y = 96u32;
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let scale = if dpi_x > 0 {
            dpi_x as f64 / 96.0
        } else {
            1.0
        };
        let device = String::from_utf16_lossy(
            &info
                .szDevice
                .iter()
                .copied()
                .take_while(|c| *c != 0)
                .collect::<Vec<_>>(),
        );
        let id = if device.trim().is_empty() {
            format!("{},{}@{}x{}@{scale:.2}", rc.left, rc.top, w, h)
        } else {
            device.clone()
        };
        // Name filled after enum from DisplayConfig / EDID (see below).
        let name = format!("{w}×{h}");
        let primary = (info.monitorInfo.dwFlags & 1) != 0;
        if let Ok(mut g) = ACC.lock() {
            if let Some(acc) = g.as_mut() {
                acc.0.push(DisplayInfo {
                    id,
                    name,
                    is_primary: primary,
                    x: rc.left,
                    y: rc.top,
                    width: w,
                    height: h,
                    scale_factor: scale,
                });
            }
        }
        BOOL(1)
    }

    {
        let mut g = ACC.lock().unwrap_or_else(|e| e.into_inner());
        *g = Some(Acc(Vec::new()));
    }
    unsafe {
        let _ = EnumDisplayMonitors(HDC::default(), None, Some(enum_proc), LPARAM(0));
        // Ensure primary flag even if dwFlags missed.
        let primary_mon = MonitorFromPoint(
            windows::Win32::Foundation::POINT { x: 0, y: 0 },
            MONITOR_DEFAULTTOPRIMARY,
        );
        let mut info = MONITORINFOEXW {
            monitorInfo: windows::Win32::Graphics::Gdi::MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFOEXW>() as u32,
                ..Default::default()
            },
            ..Default::default()
        };
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(primary_mon, &mut info as *mut _ as *mut _).as_bool() {
            let device = String::from_utf16_lossy(
                &info
                    .szDevice
                    .iter()
                    .copied()
                    .take_while(|c| *c != 0)
                    .collect::<Vec<_>>(),
            );
            if let Ok(mut g) = ACC.lock() {
                if let Some(acc) = g.as_mut() {
                    for d in &mut acc.0 {
                        if d.id == device {
                            d.is_primary = true;
                        }
                    }
                    if !acc.0.iter().any(|d| d.is_primary) {
                        if let Some(first) = acc.0.first_mut() {
                            first.is_primary = true;
                        }
                    }
                }
            }
        }
    }
    let mut out = ACC
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .map(|a| a.0)
        .unwrap_or_default();
    let ccd_names = gdi_monitor_name_map();
    for d in &mut out {
        d.name = monitor_display_name(&d.id, d.width, d.height, &ccd_names);
    }
    // Same model on two ports → append resolution so rows stay distinct.
    {
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for d in &out {
            *counts.entry(d.name.clone()).or_insert(0) += 1;
        }
        for d in &mut out {
            if counts.get(&d.name).copied().unwrap_or(0) > 1 {
                d.name = format!("{} · {}×{}", d.name, d.width, d.height);
            }
        }
    }
    out.sort_by(|a, b| {
        b.is_primary
            .cmp(&a.is_primary)
            .then(a.x.cmp(&b.x))
            .then(a.y.cmp(&b.y))
    });
    out
}

#[cfg(not(windows))]
pub fn list_displays() -> Vec<DisplayInfo> {
    Vec::new()
}

pub fn resolve_placements(
    prefs: &DisplayPlacementPrefs,
    displays: &[DisplayInfo],
) -> Vec<ResolvedPlacement> {
    let by_id: std::collections::HashMap<&str, &MonitorPlacement> = prefs
        .monitors
        .iter()
        .map(|m| (m.id.as_str(), m))
        .collect();

    // Single display always behaves as primaryOnly (ignore stale custom/allDisplays).
    let effective = if displays.len() <= 1 {
        PlacementPreset::PrimaryOnly
    } else {
        prefs.preset
    };

    displays
        .iter()
        .map(|d| {
            let (top_bar, dock) = match effective {
                PlacementPreset::PrimaryOnly => {
                    if d.is_primary {
                        (TopBarMode::Full, true)
                    } else {
                        (TopBarMode::None, false)
                    }
                }
                PlacementPreset::AllDisplays => {
                    if d.is_primary {
                        (TopBarMode::Full, true)
                    } else {
                        (TopBarMode::Shortcuts, true)
                    }
                }
                PlacementPreset::Custom => {
                    if let Some(m) = by_id.get(d.id.as_str()) {
                        let top = if !d.is_primary && m.top_bar == TopBarMode::Full {
                            // Secondary "full" = chrome without island (still Full mode for UI).
                            TopBarMode::Full
                        } else {
                            m.top_bar
                        };
                        (top, m.dock)
                    } else if d.is_primary {
                        (TopBarMode::Full, true)
                    } else {
                        (TopBarMode::None, false)
                    }
                }
            };
            ResolvedPlacement {
                id: d.id.clone(),
                name: d.name.clone(),
                is_primary: d.is_primary,
                top_bar,
                dock,
                x: d.x,
                y: d.y,
                width: d.width,
                height: d.height,
                scale_factor: d.scale_factor,
            }
        })
        .collect()
}

pub fn snapshot() -> PlacementSnapshot {
    let mut prefs = load();
    let displays = list_displays();
    if coerce_single_display(&mut prefs, displays.len()) {
        let _ = save(&prefs);
    }
    let resolved = resolve_placements(&prefs, &displays);
    let primary_top_bar = resolved
        .iter()
        .find(|r| r.is_primary)
        .map(|r| r.top_bar)
        .unwrap_or(TopBarMode::Full);
    PlacementSnapshot {
        prefs,
        displays,
        resolved,
        primary_top_bar,
    }
}

#[cfg(windows)]
fn pin_hwnd_to_monitor_rect(hwnd_raw: isize, x: i32, y: i32, width: i32, height: i32) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER};
    let hwnd = HWND(hwnd_raw as *mut _);
    // Always use the requested physical size — keeping GetWindowRect height left
    // satellites at the wrong strip size / DPI after create.
    let w = width.max(1);
    let h = height.max(1);
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            x,
            y,
            w,
            h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

#[cfg(windows)]
pub(crate) fn pin_hwnd_to_monitor_top(hwnd_raw: isize, d: &ResolvedPlacement, logical_h: i32) {
    let scale = d.scale_factor.max(0.5);
    let phys_h = ((logical_h as f64) * scale).round().max(1.0) as i32;
    pin_hwnd_to_monitor_rect(hwnd_raw, d.x, d.y, d.width as i32, phys_h);
}

/// Re-pin any top-bar HWND to its monitor's physical top (after bottom AppBar churn).
#[cfg(windows)]
pub(crate) fn pin_top_bar_hwnd_public(hwnd_raw: isize) {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER,
    };
    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(mon, &mut info).as_bool() {
            return;
        }
        let mut wr = RECT::default();
        if GetWindowRect(hwnd, &mut wr).is_err() {
            return;
        }
        let h = (wr.bottom - wr.top).max(1);
        let m = info.rcMonitor;
        let w = (m.right - m.left).max(1);
        if wr.left == m.left && wr.top == m.top && (wr.right - wr.left) == w {
            return;
        }
        let _ = SetWindowPos(
            hwnd,
            None,
            m.left,
            m.top,
            w,
            h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

#[cfg(windows)]
pub(crate) fn pin_chrome_sat_to_monitor(
    hwnd_raw: isize,
    d: &ResolvedPlacement,
    logical_h: i32,
) {
    pin_hwnd_to_monitor_top(hwnd_raw, d, logical_h);
}

#[cfg(windows)]
fn move_hwnd_onto_monitor_bottom(hwnd_raw: isize, d: &ResolvedPlacement) {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    };
    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        let mut wr = RECT::default();
        if GetWindowRect(hwnd, &mut wr).is_err() {
            return;
        }
        let w = (wr.right - wr.left).max(1);
        let h = (wr.bottom - wr.top).max(1);
        let x = d.x + ((d.width as i32 - w) / 2).max(0);
        let y = d.y + (d.height as i32 - h).max(0);
        let _ = SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

fn emit_snapshot(app: &AppHandle, snap: &PlacementSnapshot) {
    let _ = app.emit("display-placement", snap);
}

fn destroy_window(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        #[cfg(windows)]
        {
            if is_chrome_sat_label(label) {
                if let Ok(hwnd) = w.hwnd() {
                    let raw = hwnd.0 as isize;
                    crate::win32::bar_comp::detach_hwnd(raw);
                    crate::win32::satellite_appbar::unregister(raw);
                }
            }
            if label.starts_with("dock-sat-glass-") {
                if let Ok(hwnd) = w.hwnd() {
                    crate::win32::dock_comp::detach_hwnd(hwnd.0 as isize);
                }
            }
        }
        let _ = w.close();
        let _ = w.destroy();
    }
}

async fn ensure_chrome_satellite(
    app: &AppHandle,
    d: &ResolvedPlacement,
) -> Result<(), String> {
    let label = chrome_sat_label(&d.id);
    let mode = d.top_bar.as_str();
    let bar_h = crate::chrome_prefs::bar_height_logical().max(24) as f64;
    let width = (d.width as f64 / d.scale_factor.max(0.5)).max(200.0);

    if let Some(existing) = app.get_webview_window(&label) {
        #[cfg(windows)]
        if let Ok(hwnd) = existing.hwnd() {
            pin_hwnd_to_monitor_top(hwnd.0 as isize, d, bar_h as i32);
            crate::win32::satellite_appbar::register_and_sync(hwnd.0 as isize);
        }
        let _ = existing.show();
        let _ = existing.set_ignore_cursor_events(false);
        let _ = app.emit(
            "chrome-sat-mode",
            serde_json::json!({ "label": label, "mode": mode, "monitorId": d.id }),
        );
        // Re-hit material + seed ambient for this sat (cache + emit_to).
        crate::commands::apply_main_window_material(app);
        #[cfg(windows)]
        if let Ok(hwnd) = existing.hwnd() {
            let raw = hwnd.0 as isize;
            let strip = if crate::win32::ambient::is_desktop_scene(Some(raw)) {
                crate::win32::ambient::sample_wallpaper_only(Some(raw))
            } else {
                crate::win32::ambient::sample_for_satellite(Some(raw))
            };
            let labeled = strip.with_label(&label);
            crate::win32::ambient::remember_sat_strip(&label, labeled.clone());
            let _ = app.emit_to(&label, "ambient-color", &labeled);
            let _ = app.emit("ambient-color", &labeled);
        }
        return Ok(());
    }

    let mat_prefs = app
        .try_state::<crate::commands::MaterialState>()
        .map(|s| {
            s.0.lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
                .normalize()
        });
    let theme_boot = mat_prefs
        .as_ref()
        .map(|p| crate::win32::material::theme_bootstrap_script(p))
        .unwrap_or_default();
    let init = format!(
        "window.__WH_IS_CHROME_SAT__ = true;\nwindow.__WH_CHROME_SAT_MODE__ = \"{mode}\";\nwindow.__WH_CHROME_SAT_MONITOR__ = {};\n{theme_boot}",
        serde_json::to_string(&d.id).unwrap_or_else(|_| "\"\"".into())
    );
    let win = WebviewWindowBuilder::new(
        app,
        &label,
        WebviewUrl::App(format!("index.html?window=chrome-sat&mode={mode}").into()),
    )
    .title("")
    .inner_size(width, bar_h)
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
    .initialization_script(&init)
    .build()
    .map_err(|e| format!("open chrome satellite failed: {e}"))?;

    #[cfg(windows)]
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
        pin_hwnd_to_monitor_top(hwnd.0 as isize, d, bar_h as i32);
        crate::win32::blur_glass::clear_webview_fill(&win);
        // Second pin after WebView DPI settle + apply desktop glass.
        let hwnd_raw = hwnd.0 as isize;
        let d2 = d.clone();
        let bar_h2 = bar_h as i32;
        let app2 = app.clone();
        let label2 = label.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            pin_hwnd_to_monitor_top(hwnd_raw, &d2, bar_h2);
            crate::win32::satellite_appbar::register_and_sync(hwnd_raw);
            let prefs = crate::commands::load_material_prefs();
            if let Some(w) = app2.get_webview_window(&label2) {
                crate::win32::island_bar_glass::sync_sat_window_now(&w, &prefs);
            }
            let strip = if crate::win32::ambient::is_desktop_scene(Some(hwnd_raw)) {
                crate::win32::ambient::sample_wallpaper_only(Some(hwnd_raw))
            } else {
                crate::win32::ambient::sample_for_satellite(Some(hwnd_raw))
            };
            let labeled = strip.with_label(&label2);
            crate::win32::ambient::remember_sat_strip(&label2, labeled.clone());
            let _ = app2.emit_to(&label2, "ambient-color", &labeled);
            let _ = app2.emit("ambient-color", &labeled);
            // Second hit after first paint (same cadence as main material settle).
            std::thread::sleep(std::time::Duration::from_millis(400));
            let prefs = crate::commands::load_material_prefs();
            if let Some(w) = app2.get_webview_window(&label2) {
                crate::win32::island_bar_glass::sync_sat_window_now(&w, &prefs);
            }
            crate::commands::apply_main_window_material(&app2);
        });
        crate::win32::satellite_appbar::register_and_sync(hwnd.0 as isize);
    }
    let _ = win.show();
    Ok(())
}

async fn ensure_dock_satellite(
    app: &AppHandle,
    d: &ResolvedPlacement,
    prefs: &crate::dock::DockPrefs,
) -> Result<(), String> {
    let label = dock_sat_label(&d.id);
    let glass_label = dock_sat_glass_label(&d.id);
    let layout = crate::dock::dock_layout_items(prefs);
    let width = crate::dock::dock_window_width(
        &layout,
        prefs.corner_radius_px,
        prefs.magnification,
        false,
    );
    let height = crate::dock::dock_window_height(prefs.magnification);
    let glass_h = crate::dock::DOCK_H;

    if app.get_webview_window(&glass_label).is_none() {
        let mat_prefs = app
            .try_state::<crate::commands::MaterialState>()
            .map(|s| {
                s.0.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
                    .normalize()
            });
        let theme_boot = mat_prefs
            .as_ref()
            .map(|p| crate::win32::material::theme_bootstrap_script(p))
            .unwrap_or_default();
        let glass_init = format!(
            "window.__WH_IS_DOCK_GLASS__ = true;\nwindow.__WH_IS_DOCK_SAT__ = true;\n{theme_boot}"
        );
        let glass = WebviewWindowBuilder::new(
            app,
            &glass_label,
            WebviewUrl::App("index.html?window=dock-glass".into()),
        )
        .title("")
        .inner_size(width, glass_h)
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
        .map_err(|e| format!("open dock-sat glass failed: {e}"))?;
        let _ = glass.set_ignore_cursor_events(true);
        #[cfg(windows)]
        if let Ok(hwnd) = glass.hwnd() {
            crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
            move_hwnd_onto_monitor_bottom(hwnd.0 as isize, d);
        }
    }

    if app.get_webview_window(&label).is_none() {
        let init = format!(
            "window.__WH_IS_DOCK__ = true;\nwindow.__WH_IS_DOCK_SAT__ = true;\nwindow.__WH_DOCK_SAT_MONITOR__ = {};\n",
            serde_json::to_string(&d.id).unwrap_or_else(|_| "\"\"".into())
        );
        let win = WebviewWindowBuilder::new(
            app,
            &label,
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
        .disable_drag_drop_handler()
        .initialization_script(&init)
        .build()
        .map_err(|e| format!("open dock-sat failed: {e}"))?;
        #[cfg(windows)]
        if let Ok(hwnd) = win.hwnd() {
            crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
            move_hwnd_onto_monitor_bottom(hwnd.0 as isize, d);
        }
        let _ = win.show();
        if let Some(g) = app.get_webview_window(&glass_label) {
            // Same HostBackdrop / SWCA path as primary dock-glass.
            if let Some(state) = app.try_state::<crate::commands::MaterialState>() {
                crate::commands::apply_saved_material_pub(&g, &state);
            }
            let _ = g.show();
        }
        if let Some(state) = app.try_state::<crate::commands::MaterialState>() {
            crate::commands::apply_saved_material_pub(&win, &state);
        }
        // Soft reassert primary glass after sat attach.
        if let Some(state) = app.try_state::<crate::commands::MaterialState>() {
            if let Some(g) = app.get_webview_window("dock-glass") {
                crate::commands::reassert_saved_material_pub(&g, &state);
            }
        }
        // Same place path as primary (glass under icons + headroom).
        crate::dock::place_dock_satellites(app, prefs, true, false);
    } else if let Some(_existing) = app.get_webview_window(&label) {
        crate::dock::place_dock_satellites(app, prefs, true, false);
    }
    Ok(())
}

#[cfg(windows)]
pub fn move_dock_sat_onto_monitor(hwnd_raw: isize, d: &ResolvedPlacement) {
    move_hwnd_onto_monitor_bottom(hwnd_raw, d);
}

#[cfg(not(windows))]
pub fn move_dock_sat_onto_monitor(_hwnd_raw: isize, _d: &ResolvedPlacement) {}

/// Re-pin existing dock satellites after primary `position_dock_window`.
pub fn reposition_dock_satellites(app: &AppHandle) {
    let prefs = crate::dock::load_dock_prefs();
    if !prefs.enabled {
        return;
    }
    crate::dock::place_dock_satellites(app, &prefs, true, false);
}

/// Apply placement: move main/dock, create/destroy satellites.
pub async fn apply(app: &AppHandle) -> Result<PlacementSnapshot, String> {
    if APPLYING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Ok(snapshot());
    }
    let result = apply_inner(app).await;
    APPLYING.store(false, Ordering::SeqCst);
    result
}

async fn apply_inner(app: &AppHandle) -> Result<PlacementSnapshot, String> {
    let snap = snapshot();
    let primary = snap.resolved.iter().find(|r| r.is_primary).cloned();
    let primary_top = snap.primary_top_bar;
    let needs_secondary_chrome = snap
        .resolved
        .iter()
        .any(|r| !r.is_primary && r.top_bar != TopBarMode::None);
    let need_extra_dock = snap.resolved.iter().any(|r| !r.is_primary && r.dock)
        && crate::dock::load_dock_prefs().enabled;

    // ── Phase 1: primary only (never touch secondary in this step) ──
    apply_primary_only(app, primary.as_ref(), primary_top).await;

    // Clean leftover sat docks; recreate in phase 3 after primary settles.
    destroy_all_dock_satellites(app);

    if !needs_secondary_chrome {
        destroy_all_chrome_satellites(app);
    } else {
        // ── Phase 2: secondary chrome, after primary has settled ──
        tauri::async_runtime::spawn_blocking(|| {
            std::thread::sleep(std::time::Duration::from_millis(1500));
        })
        .await
        .ok();
        if crate::lifecycle::stopping() {
            emit_snapshot(app, &snap);
            return Ok(snap);
        }
        apply_secondary_chrome(app, &snap).await;
    }

    // ── Phase 3: secondary docks (independent of chrome), one monitor at a time ──
    if need_extra_dock {
        let settle_ms = if needs_secondary_chrome { 2000 } else { 1500 };
        tauri::async_runtime::spawn_blocking(move || {
            std::thread::sleep(std::time::Duration::from_millis(settle_ms));
        })
        .await
        .ok();
        if !crate::lifecycle::stopping() {
            apply_secondary_docks(app, &snap).await;
        }
    }

    emit_snapshot(app, &snap);
    Ok(snap)
}

async fn apply_primary_only(
    app: &AppHandle,
    primary: Option<&ResolvedPlacement>,
    primary_top: TopBarMode,
) {
    let Some(p) = primary else {
        return;
    };

    // Main island / top bar — pin to primary monitor only.
    if let Some(main) = app.get_webview_window("main") {
        match primary_top {
            TopBarMode::None => {
                #[cfg(windows)]
                if let Ok(hwnd) = main.hwnd() {
                    crate::win32::appbar::unregister(hwnd.0 as isize);
                }
                let _ = main.hide();
            }
            TopBarMode::Full | TopBarMode::Shortcuts => {
                #[cfg(windows)]
                if let Ok(hwnd) = main.hwnd() {
                    let bar_h = crate::chrome_prefs::bar_height_logical();
                    pin_hwnd_to_monitor_top(hwnd.0 as isize, p, bar_h);
                    crate::win32::appbar::register(hwnd.0 as isize);
                    crate::win32::appbar::force_sync(hwnd.0 as isize);
                }
                if crate::host_boot_ready() {
                    let _ = main.show();
                }
            }
        }
    }

    // Dock — only on primary when that screen wants dock; never create clones here.
    let dock_prefs = crate::dock::load_dock_prefs();
    if !dock_prefs.enabled {
        return;
    }
    if !p.dock {
        // Prefs say no dock on primary: still keep the single dock window on primary
        // bottom so it is not lost on a secondary from an earlier move.
    }
    if let Some(dock) = app.get_webview_window("dock") {
        #[cfg(windows)]
        if let Ok(hwnd) = dock.hwnd() {
            move_hwnd_onto_monitor_bottom(hwnd.0 as isize, p);
        }
    }
    if let Some(glass) = app.get_webview_window("dock-glass") {
        #[cfg(windows)]
        if let Ok(hwnd) = glass.hwnd() {
            move_hwnd_onto_monitor_bottom(hwnd.0 as isize, p);
        }
    }
    // Brief pause so SetWindowPos settles before place animation.
    tauri::async_runtime::spawn_blocking(|| {
        std::thread::sleep(std::time::Duration::from_millis(200));
    })
    .await
    .ok();
    crate::dock::position_dock_window(app, &dock_prefs);
    if let Some(state) = app.try_state::<crate::commands::MaterialState>() {
        if let Some(g) = app.get_webview_window("dock-glass") {
            crate::commands::reassert_saved_material_pub(&g, &state);
        }
    }
}

async fn apply_secondary_chrome(app: &AppHandle, snap: &PlacementSnapshot) {
    let mut want_chrome: HashSet<String> = HashSet::new();
    for r in &snap.resolved {
        if r.is_primary || r.top_bar == TopBarMode::None {
            continue;
        }
        want_chrome.insert(chrome_sat_label(&r.id));
        if let Err(e) = ensure_chrome_satellite(app, r).await {
            eprintln!("[display-placement] chrome sat {}: {e}", r.id);
        }
        tauri::async_runtime::spawn_blocking(|| {
            std::thread::sleep(std::time::Duration::from_millis(600));
        })
        .await
        .ok();
    }
    {
        let mut prev = LAST_SAT_CHROME.lock().unwrap_or_else(|e| e.into_inner());
        for old in prev.iter() {
            if !want_chrome.contains(old) {
                destroy_window(app, old);
            }
        }
        *prev = want_chrome.into_iter().collect();
    }
}

async fn apply_secondary_docks(app: &AppHandle, snap: &PlacementSnapshot) {
    let prefs = crate::dock::load_dock_prefs();
    if !prefs.enabled {
        destroy_all_dock_satellites(app);
        return;
    }
    let mut want: HashSet<String> = HashSet::new();
    for r in &snap.resolved {
        if r.is_primary || !r.dock {
            continue;
        }
        want.insert(dock_sat_label(&r.id));
        want.insert(dock_sat_glass_label(&r.id));
        if let Err(e) = ensure_dock_satellite(app, r, &prefs).await {
            eprintln!("[display-placement] dock sat {}: {e}", r.id);
        }
        // Reassert primary glass after each sat (sat uses SWCA only).
        if let Some(state) = app.try_state::<crate::commands::MaterialState>() {
            if let Some(g) = app.get_webview_window("dock-glass") {
                crate::commands::reassert_saved_material_pub(&g, &state);
            }
        }
        tauri::async_runtime::spawn_blocking(|| {
            std::thread::sleep(std::time::Duration::from_millis(800));
        })
        .await
        .ok();
    }
    {
        let mut prev = LAST_SAT_DOCK.lock().unwrap_or_else(|e| e.into_inner());
        for old in prev.iter() {
            if !want.contains(old) {
                destroy_window(app, old);
            }
        }
        *prev = want.into_iter().collect();
    }
}

fn destroy_all_chrome_satellites(app: &AppHandle) {
    let sweep: Vec<String> = app
        .webview_windows()
        .keys()
        .filter(|l| l.starts_with("chrome-sat-"))
        .cloned()
        .collect();
    for old in sweep {
        destroy_window(app, &old);
    }
    *LAST_SAT_CHROME.lock().unwrap_or_else(|e| e.into_inner()) = Vec::new();
}

fn destroy_all_dock_satellites(app: &AppHandle) {
    let leftover: Vec<String> = {
        let prev = LAST_SAT_DOCK.lock().unwrap_or_else(|e| e.into_inner());
        prev.clone()
    };
    for old in &leftover {
        if is_dock_sat_label(old) {
            if let Some(w) = app.get_webview_window(old) {
                if let Ok(h) = w.hwnd() {
                    crate::win32::satellite_dock_appbar::unregister(h.0 as isize);
                }
            }
        }
        destroy_window(app, old);
    }
    let sweep: Vec<String> = app
        .webview_windows()
        .keys()
        .filter(|l| l.starts_with("dock-sat-"))
        .cloned()
        .collect();
    for old in sweep {
        if is_dock_sat_label(&old) {
            if let Some(w) = app.get_webview_window(&old) {
                if let Ok(h) = w.hwnd() {
                    crate::win32::satellite_dock_appbar::unregister(h.0 as isize);
                }
            }
        }
        destroy_window(app, &old);
    }
    crate::win32::satellite_dock_appbar::unregister_all();
    *LAST_SAT_DOCK.lock().unwrap_or_else(|e| e.into_inner()) = Vec::new();
}

pub fn apply_sync(app: &AppHandle) {
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = apply(&app2).await;
    });
}

/// Poll monitor topology; re-apply when ids/count change (slow debounce).
pub fn spawn_display_change_watcher(app: AppHandle) {
    std::thread::Builder::new()
        .name("display-placement".into())
        .spawn(move || {
            let mut last: Vec<String> = list_displays().into_iter().map(|d| d.id).collect();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(8));
                if crate::lifecycle::stopping() {
                    break;
                }
                let now: Vec<String> = list_displays().into_iter().map(|d| d.id).collect();
                if now != last {
                    last = now;
                    apply_sync(&app);
                }
            }
        })
        .ok();
}

#[tauri::command]
pub fn list_displays_cmd() -> Vec<DisplayInfo> {
    list_displays()
}

#[tauri::command]
pub fn get_display_placement() -> PlacementSnapshot {
    snapshot()
}

#[tauri::command]
pub async fn set_display_placement(
    app: AppHandle,
    prefs: DisplayPlacementPrefs,
) -> Result<PlacementSnapshot, String> {
    let mut next = prefs;
    next.preset = PlacementPreset::parse(next.preset.as_str());
    for m in &mut next.monitors {
        m.top_bar = TopBarMode::parse(m.top_bar.as_str());
        if m.id.trim().is_empty() {
            return Err("monitor id required".into());
        }
    }
    let display_count = list_displays().len();
    let _ = coerce_single_display(&mut next, display_count);
    if next.preset == PlacementPreset::Custom {
        seed_custom_monitors(&mut next);
    } else {
        next.monitors.clear();
    }
    save(&next)?;
    // Settings save: run apply on a worker so the settings UI does not block;
    // primary settles before secondary chrome (see apply_inner phases).
    let snap = apply(&app).await?;
    Ok(snap)
}

#[tauri::command]
pub async fn apply_display_placement(app: AppHandle) -> Result<PlacementSnapshot, String> {
    apply(&app).await
}

/// Seed custom rows from current displays when entering custom first time.
pub fn seed_custom_monitors(prefs: &mut DisplayPlacementPrefs) {
    if prefs.preset != PlacementPreset::Custom {
        return;
    }
    if !prefs.monitors.is_empty() {
        return;
    }
    let displays = list_displays();
    let derived = resolve_placements(
        &DisplayPlacementPrefs {
            preset: PlacementPreset::PrimaryOnly,
            monitors: Vec::new(),
        },
        &displays,
    );
    prefs.monitors = derived
        .into_iter()
        .map(|r| MonitorPlacement {
            id: r.id,
            top_bar: r.top_bar,
            dock: r.dock,
        })
        .collect();
}

#[allow(dead_code)]
pub fn prefs_to_value(prefs: &DisplayPlacementPrefs) -> Value {
    serde_json::to_value(prefs).unwrap_or(Value::Null)
}
