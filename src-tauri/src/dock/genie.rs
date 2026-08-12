//! macOS-like genie minimize/restore — capture last frame, animate to dock/anchor, park HWND.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{mpsc, LazyLock, Mutex};
use std::time::Duration;
use tauri::utils::config::Color;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State, WebviewUrl,
    WebviewWindowBuilder,
};

use super::{load_dock_prefs, DockItem};
use crate::dock::launch::matching_windows;
use crate::win32::capture::{capture_window_jpeg, capture_window_jpeg_genie, Roi};
use crate::win32::enum_windows::{focus_window, list_windows, process_exe};
use crate::win32::park::{
    minimize_window_os, snapshot_window, unpark_window, unpark_window_under_cover,
    PlacementSnapshot,
};

const OVERLAY_LABEL: &str = "genie-overlay";

/// Last known Dock icon screen rects (logical), reported by DockApp.
static ICON_RECTS: LazyLock<Mutex<HashMap<String, GenieRect>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Hot cache: last good frame of the foreground Dock-tracked window.
/// OS minimize intercept must use this — capturing after MINIMIZESTART is too late.
static PREPARED_MIN: LazyLock<Mutex<Option<PreparedMinimize>>> =
    LazyLock::new(|| Mutex::new(None));
static LAST_PREP_CAP_MS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct PreparedMinimize {
    pub hwnd: isize,
    pub item_id: String,
    pub placement: PlacementSnapshot,
    pub restore_rect: GenieRect,
    pub jpeg: Vec<u8>,
    /// Absolute JPEG path already on disk — overlay loads via convertFileSrc (no fat IPC).
    pub frame_path: String,
    pub at: std::time::Instant,
}

/// Last foreground app that is not Window Hub chrome (dock / island / overlays).
static LAST_USER_FG: Mutex<Option<LastFg>> = Mutex::new(None);

/// Dock pointerdown arms minimize for this item until expiry (survives focus steal).
static MINIMIZE_ARM: Mutex<Option<MinimizeArm>> = Mutex::new(None);

#[derive(Debug, Clone)]
struct LastFg {
    hwnd: isize,
    exe: String,
    name: String,
}

#[derive(Debug, Clone)]
struct MinimizeArm {
    item_id: String,
    until: std::time::Instant,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenieRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeniePlayPayload {
    /// Absolute path for convertFileSrc — preferred.
    #[serde(default)]
    pub frame_path: String,
    /// Tiny legacy fallback (usually empty).
    #[serde(default)]
    pub jpeg_base64: String,
    pub from: GenieRect,
    pub to: GenieRect,
    pub direction: String,
    pub duration_ms: u32,
    pub request_id: String,
}

struct GenieSlot {
    hwnd: isize,
    placement: PlacementSnapshot,
    frame_jpeg: Vec<u8>,
    /// Logical screen rect of the window before park.
    restore_rect: GenieRect,
    #[allow(dead_code)]
    kind: GenieKind,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum GenieKind {
    DockApp,
    Popup { label: String },
}

pub struct GenieState {
    inner: Mutex<GenieInner>,
}

struct GenieInner {
    slots: HashMap<String, GenieSlot>,
    pending_done: Option<(String, mpsc::Sender<()>)>,
    /// Fired once the overlay has painted the freeze frame (so we can park without a gap).
    pending_painted: Option<(String, mpsc::Sender<()>)>,
    busy: bool,
    /// HWNDs currently expanding under the genie overlay — block switcher auto-unpark.
    restoring: std::collections::HashSet<isize>,
}

impl Default for GenieState {
    fn default() -> Self {
        Self {
            inner: Mutex::new(GenieInner {
                slots: HashMap::new(),
                pending_done: None,
                pending_painted: None,
                busy: false,
                restoring: std::collections::HashSet::new(),
            }),
        }
    }
}

impl GenieState {
    pub fn is_busy_or_parked(&self, item_id: &str) -> bool {
        self.inner
            .lock()
            .map(|g| g.busy || g.slots.contains_key(item_id))
            .unwrap_or(true)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockIconRectReport {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[tauri::command]
pub fn dock_report_icon_rects(rects: Vec<DockIconRectReport>) {
    if let Ok(mut g) = ICON_RECTS.lock() {
        g.clear();
        for r in rects {
            if r.id.is_empty() {
                continue;
            }
            g.insert(
                r.id,
                GenieRect {
                    x: r.x,
                    y: r.y,
                    w: r.w.max(8.0),
                    h: r.h.max(8.0),
                },
            );
        }
    }
}

pub fn icon_rect_for_item(item_id: &str) -> Option<GenieRect> {
    ICON_RECTS
        .lock()
        .ok()
        .and_then(|g| g.get(item_id).copied())
}

/// Disable DWM open/close/minimize transitions on a HWND (instant show/hide).
pub fn disable_hwnd_transitions(hwnd_raw: isize) {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
        // DWMWA_TRANSITIONS_FORCEDISABLED = 3
        let hwnd = HWND(hwnd_raw as *mut _);
        let disable: i32 = 1;
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWINDOWATTRIBUTE(3),
                &disable as *const i32 as *const _,
                std::mem::size_of::<i32>() as u32,
            );
        }
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd_raw;
    }
}

/// Take a still-fresh prepared frame for this HWND (or matching item).
pub fn take_prepared_minimize(hwnd: isize, item_id: &str) -> Option<PreparedMinimize> {
    let mut g = PREPARED_MIN.lock().ok()?;
    let Some(p) = g.as_ref() else {
        return None;
    };
    // Keep warm enough that Dock click-minimize usually hits cache (no live capture hitch).
    if p.at.elapsed() > Duration::from_millis(1600) {
        *g = None;
        return None;
    }
    if p.hwnd != hwnd && p.item_id != item_id {
        return None;
    }
    g.take()
}

fn refresh_prepared_minimize(hwnd: isize) {
    let Some(item_id) = dock_item_id_for_hwnd(hwnd) else {
        return;
    };
    let Ok(placement) = snapshot_window(hwnd) else {
        return;
    };
    let Ok(restore_rect) = hwnd_screen_rect_logical(hwnd) else {
        return;
    };
    // Skip tiny/iconic boxes — not a usable genie source.
    if restore_rect.w < 80.0 || restore_rect.h < 60.0 {
        return;
    }
    // PrintWindow-first — never screen BitBlt (leaks windows underneath).
    let Ok(frame) = capture_window_jpeg_genie(hwnd) else {
        return;
    };
    if frame.jpeg.is_empty() || frame.jpeg.len() < 64 {
        return;
    }
    let frame_path = write_genie_frame_bytes(&item_id, &frame.jpeg).unwrap_or_default();
    if frame_path.is_empty() {
        return;
    }
    if let Ok(mut g) = PREPARED_MIN.lock() {
        *g = Some(PreparedMinimize {
            hwnd,
            item_id,
            placement,
            restore_rect,
            jpeg: frame.jpeg,
            frame_path,
            at: std::time::Instant::now(),
        });
    }
}

fn genie_cache_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .map(|p| p.join("window-hub").join("cache"))
        .unwrap_or_else(|| std::env::temp_dir().join("window-hub-genie"));
    let dir = if base.ends_with("cache") {
        base.join("genie")
    } else {
        base
    };
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn write_genie_frame_bytes(tag: &str, bytes: &[u8]) -> Option<String> {
    let dir = genie_cache_dir()?;
    let safe: String = tag
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(48)
        .collect();
    let ext = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "png"
    } else {
        "jpg"
    };
    let name = format!(
        "f-{}-{}.{}",
        safe,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
        ext
    );
    let path = dir.join(name);
    std::fs::write(&path, bytes).ok()?;
    Some(path.to_string_lossy().to_string())
}

fn allow_genie_asset_scope(app: &AppHandle, path: &str) {
    let p = std::path::Path::new(path);
    if let Some(parent) = p.parent() {
        let _ = app.asset_protocol_scope().allow_directory(parent, false);
    }
    let _ = app.asset_protocol_scope().allow_file(p);
}

/// Map an HWND to a Dock app item id (exe / path match).
pub fn dock_item_id_for_hwnd(hwnd: isize) -> Option<String> {
    if hwnd == 0 || is_hub_chrome_hwnd(hwnd) {
        return None;
    }
    let prefs = load_dock_prefs();
    let items = super::dock_merge_running(&prefs, false);
    let wins = list_windows(None);
    for item in &items {
        if item.kind != "app" {
            continue;
        }
        if matching_windows(item, &wins).iter().any(|w| w.hwnd == hwnd) {
            return Some(item.id.clone());
        }
    }
    // Window may already be iconic / filtered from enum — match by process.
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(HWND(hwnd as *mut _), Some(&mut pid));
        }
        if pid == 0 || pid == std::process::id() {
            return None;
        }
        let (exe, name) = process_exe(pid);
        let exe = exe.unwrap_or_default();
        let name = name.unwrap_or_default();
        for item in &items {
            if item.kind != "app" {
                continue;
            }
            if item_matches_exe(item, &exe, &name) {
                return Some(item.id.clone());
            }
        }
    }
    None
}

fn find_dock_item(item_id: &str) -> Result<DockItem, String> {
    let prefs = load_dock_prefs();
    let items = super::dock_merge_running(&prefs, false);
    items
        .into_iter()
        .find(|i| i.id == item_id)
        .ok_or_else(|| format!("dock item not found: {item_id}"))
}

#[cfg(windows)]
fn hwnd_scale(hwnd_raw: isize) -> f64 {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        if dpi == 0 {
            1.0
        } else {
            dpi as f64 / 96.0
        }
    }
}

#[cfg(not(windows))]
fn hwnd_scale(_hwnd_raw: isize) -> f64 {
    1.0
}

#[cfg(windows)]
fn hwnd_screen_rect_logical(hwnd_raw: isize) -> Result<GenieRect, String> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsWindow};
    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return Err("Invalid window".into());
        }
        // Prefer visible frame — matches cropped genie capture (no shadow margin).
        let mut r = RECT::default();
        GetWindowRect(hwnd, &mut r).map_err(|e| format!("GetWindowRect: {e}"))?;
        let mut visible = r;
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut visible as *mut RECT as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        )
        .is_ok()
            && visible.right > visible.left
            && visible.bottom > visible.top
        {
            r = visible;
        }
        let scale = hwnd_scale(hwnd_raw).max(0.5);
        Ok(GenieRect {
            x: r.left as f64 / scale,
            y: r.top as f64 / scale,
            w: (r.right - r.left).max(1) as f64 / scale,
            h: (r.bottom - r.top).max(1) as f64 / scale,
        })
    }
}

#[cfg(not(windows))]
fn hwnd_screen_rect_logical(_hwnd_raw: isize) -> Result<GenieRect, String> {
    Err("Windows only".into())
}

#[cfg(windows)]
fn foreground_hwnd() -> Option<isize> {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    unsafe {
        let h = GetForegroundWindow();
        if h.0.is_null() {
            None
        } else {
            Some(h.0 as isize)
        }
    }
}

#[cfg(not(windows))]
fn foreground_hwnd() -> Option<isize> {
    None
}

fn ensure_overlay(app: &AppHandle, cover: &GenieRect) -> Result<bool, String> {
    let cover = clamp_overlay_cover(cover);
    let w = cover.w.max(100.0);
    let h = cover.h.max(100.0);

    if let Some(existing) = app.get_webview_window(OVERLAY_LABEL) {
        // Prefer Win32 place: Tauri set_size on transparent windows reallocates
        // softbuffer; a bad/racy size panics with `!bitmap.is_null()`.
        #[cfg(windows)]
        let placed = existing
            .hwnd()
            .ok()
            .map(|hwnd| place_overlay_hwnd(hwnd.0 as isize, cover.x, cover.y, w, h))
            .unwrap_or(false);
        #[cfg(not(windows))]
        let placed = false;
        if !placed {
            let _ = existing.set_size(LogicalSize::new(w, h));
            let _ = existing.set_position(LogicalPosition::new(cover.x, cover.y));
        }
        let _ = existing.set_ignore_cursor_events(true);
        let _ = existing.set_shadow(false);
        #[cfg(windows)]
        crate::win32::blur_glass::strip_transparent_overlay_chrome(&existing);
        let _ = existing.unminimize();
        let _ = existing.show();
        return Ok(false);
    }

    let init = r#"
      window.__WH_IS_GENIE_OVERLAY__ = true;
    "#;
    let win = WebviewWindowBuilder::new(
        app,
        OVERLAY_LABEL,
        WebviewUrl::App("index.html?window=genie-overlay".into()),
    )
    .title("Genie")
    .inner_size(w, h)
    .position(cover.x, cover.y)
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
    .visible(true)
    .initialization_script(init)
    .build()
    .map_err(|e| format!("open genie overlay failed: {e}"))?;

    let _ = win.set_ignore_cursor_events(true);
    let _ = win.set_shadow(false);
    if let Ok(hwnd) = win.hwnd() {
        crate::win32::switcher::exclude_from_switcher(hwnd.0 as isize);
        #[cfg(windows)]
        let _ = place_overlay_hwnd(hwnd.0 as isize, cover.x, cover.y, w, h);
    }
    #[cfg(windows)]
    crate::win32::blur_glass::strip_transparent_overlay_chrome(&win);
    Ok(true)
}

fn hide_overlay(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(OVERLAY_LABEL) {
        let _ = w.hide();
    }
}

/// Cover rect large enough for from+to (union), snapped to include both.
fn cover_for(from: &GenieRect, to: &GenieRect) -> GenieRect {
    let x0 = from.x.min(to.x);
    let y0 = from.y.min(to.y);
    let x1 = (from.x + from.w).max(to.x + to.w);
    let y1 = (from.y + from.h).max(to.y + to.h);
    // Pad so mesh edges aren't clipped.
    let pad = 24.0;
    clamp_overlay_cover(&GenieRect {
        x: x0 - pad,
        y: y0 - pad,
        w: (x1 - x0) + pad * 2.0,
        h: (y1 - y0) + pad * 2.0,
    })
}

/// Cap overlay size so softbuffer's CreateDIBSection cannot OOM/assert.
/// Transparent Tauri windows allocate a full ARGB softbuffer for the client area.
fn clamp_overlay_cover(cover: &GenieRect) -> GenieRect {
    // Softbuffer uses i32 dims; GDI often fails well before that. Keep headroom.
    const MAX_LOGICAL: f64 = 4096.0;
    let mut w = cover.w.max(100.0).min(MAX_LOGICAL);
    let mut h = cover.h.max(100.0).min(MAX_LOGICAL);
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
        };
        unsafe {
            let vx = GetSystemMetrics(SM_CXVIRTUALSCREEN) as f64;
            let vy = GetSystemMetrics(SM_CYVIRTUALSCREEN) as f64;
            // Virtual metrics are physical px; treat as upper bound in logical too
            // (covers 100% DPI; at higher DPI softbuffer is smaller than this cap).
            if vx > 0.0 {
                w = w.min(vx + 48.0).min(MAX_LOGICAL);
            }
            if vy > 0.0 {
                h = h.min(vy + 48.0).min(MAX_LOGICAL);
            }
        }
    }
    if !w.is_finite() || w < 100.0 {
        w = 100.0;
    }
    if !h.is_finite() || h < 100.0 {
        h = 100.0;
    }
    let x = if cover.x.is_finite() { cover.x } else { 0.0 };
    let y = if cover.y.is_finite() { cover.y } else { 0.0 };
    GenieRect { x, y, w, h }
}

#[cfg(windows)]
fn place_overlay_hwnd(hwnd_raw: isize, x: f64, y: f64, w: f64, h: f64) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE,
    };
    let hwnd = HWND(hwnd_raw as *mut _);
    let scale = hwnd_scale(hwnd_raw).max(0.5);
    let px = (x * scale).round() as i32;
    let py = (y * scale).round() as i32;
    let pw = (w * scale).round().clamp(100.0, 8192.0) as i32;
    let ph = (h * scale).round().clamp(100.0, 8192.0) as i32;
    unsafe {
        // Must raise above the target app — SWP_NOZORDER left the freeze under the real window.
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            px,
            py,
            pw,
            ph,
            SWP_NOACTIVATE,
        )
        .is_ok()
    }
}

fn relative(rect: &GenieRect, cover: &GenieRect) -> GenieRect {
    GenieRect {
        x: rect.x - cover.x,
        y: rect.y - cover.y,
        w: rect.w,
        h: rect.h,
    }
}

async fn play_and_wait(
    app: &AppHandle,
    state: &GenieState,
    jpeg: &[u8],
    from: GenieRect,
    to: GenieRect,
    direction: &str,
) -> Result<(), String> {
    play_and_wait_ex(app, state, jpeg, from, to, direction, true).await
}

/// Show overlay, emit play, wait until freeze is painted (or timeout).
/// Returns whether the freeze frame was acknowledged.
async fn emit_genie_play(
    app: &AppHandle,
    state: &GenieState,
    jpeg: &[u8],
    frame_path: Option<&str>,
    from: &GenieRect,
    to: &GenieRect,
    direction: &str,
) -> Result<(String, mpsc::Receiver<()>, bool, bool), String> {
    let cover = cover_for(from, to);
    let created = ensure_overlay(app, &cover)?;

    let request_id = uuid_like();
    let (tx_done, rx_done) = mpsc::channel::<()>();
    let (tx_paint, rx_paint) = mpsc::channel::<()>();
    {
        let mut g = state.inner.lock().map_err(|e| e.to_string())?;
        if g.busy {
            return Err("genie busy".into());
        }
        g.busy = true;
        g.pending_done = Some((request_id.clone(), tx_done));
        g.pending_painted = Some((request_id.clone(), tx_paint));
    }

    let frame_path = match frame_path {
        Some(p) if !p.is_empty() && std::path::Path::new(p).is_file() => p.to_string(),
        _ => write_genie_frame_bytes(&request_id, jpeg)
            .ok_or_else(|| "write genie frame failed".to_string())?,
    };
    allow_genie_asset_scope(app, &frame_path);

    let payload = GeniePlayPayload {
        frame_path,
        jpeg_base64: String::new(),
        from: relative(from, &cover),
        to: relative(to, &cover),
        direction: direction.to_string(),
        duration_ms: genie_duration_ms(),
        request_id: request_id.clone(),
    };

    // Cold create: give WebView one tick to mount the listener.
    if created {
        tauri::async_runtime::spawn_blocking(|| std::thread::sleep(Duration::from_millis(40)))
            .await
            .ok();
    }

    if let Some(w) = app.get_webview_window(OVERLAY_LABEL) {
        let _ = w.set_always_on_top(true);
        let _ = w.show();
        #[cfg(windows)]
        if let Ok(hwnd) = w.hwnd() {
            let _ = place_overlay_above(hwnd.0 as isize);
        }
        let _ = w.emit("genie-play", &payload);
    } else {
        let mut g = state.inner.lock().map_err(|e| e.to_string())?;
        g.busy = false;
        g.pending_done = None;
        g.pending_painted = None;
        return Err("genie overlay missing".into());
    }

    // File load is fast; keep this short so minimize button doesn't "顿一下".
    let paint_ms = if created { 280 } else { 160 };
    let painted = tauri::async_runtime::spawn_blocking(move || {
        rx_paint
            .recv_timeout(Duration::from_millis(paint_ms))
            .is_ok()
    })
    .await
    .unwrap_or(false);

    if !painted {
        eprintln!("[genie] freeze paint timeout ({paint_ms}ms) — continuing");
    }

    Ok((request_id, rx_done, painted, created))
}

async fn wait_genie_done(rx_done: mpsc::Receiver<()>, state: &GenieState) {
    let wait_ms = (genie_duration_ms() as u64).saturating_add(400).max(800);
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let _ = rx_done.recv_timeout(Duration::from_millis(wait_ms));
    })
    .await;
    if let Ok(mut g) = state.inner.lock() {
        g.busy = false;
        g.pending_done = None;
        g.pending_painted = None;
    }
}

async fn play_and_wait_ex(
    app: &AppHandle,
    state: &GenieState,
    jpeg: &[u8],
    from: GenieRect,
    to: GenieRect,
    direction: &str,
    hide_when_done: bool,
) -> Result<(), String> {
    let (_id, rx_done, _painted, _created) =
        emit_genie_play(app, state, jpeg, None, &from, &to, direction).await?;
    wait_genie_done(rx_done, state).await;
    if hide_when_done {
        hide_overlay(app);
    }
    Ok(())
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("g-{t}")
}

fn genie_duration_ms() -> u32 {
    load_dock_prefs().genie_duration_ms.clamp(200, 1500)
}

fn pick_target_hwnd(item: &DockItem) -> Result<isize, String> {
    let wins = list_windows(None);
    let matched = matching_windows(item, &wins);
    matched
        .first()
        .map(|w| w.hwnd)
        .ok_or_else(|| "no matching window".into())
}

#[cfg(windows)]
fn is_hub_chrome_hwnd(hwnd_raw: isize) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(HWND(hwnd_raw as *mut _), Some(&mut pid));
    }
    pid != 0 && pid == std::process::id()
}

#[cfg(not(windows))]
fn is_hub_chrome_hwnd(_hwnd_raw: isize) -> bool {
    false
}

/// Record a user app HWND as the last foreground (call after Dock focus/launch).
pub fn note_user_foreground(hwnd: isize) {
    if hwnd == 0 || is_hub_chrome_hwnd(hwnd) {
        return;
    }
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(HWND(hwnd as *mut _), Some(&mut pid));
        }
        if pid == 0 || pid == std::process::id() {
            return;
        }
        let (exe, name) = process_exe(pid);
        let exe = exe.unwrap_or_default();
        let name = name.unwrap_or_default();
        if exe.is_empty() && name.is_empty() {
            return;
        }
        if let Ok(mut g) = LAST_USER_FG.lock() {
            *g = Some(LastFg { hwnd, exe, name });
        }
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
    }
}

/// Pre-create a tiny always-on-top overlay so the first suck isn't a cold start.
pub fn prewarm_overlay(app: &AppHandle) {
    let cover = GenieRect {
        x: -320.0,
        y: -320.0,
        w: 160.0,
        h: 120.0,
    };
    let _ = ensure_overlay(app, &cover);
    hide_overlay(app);
}

/// Background sampler so Dock click (which steals FG) still knows the prior app.
/// Also: if Alt+Tab focuses a genie-parked HWND, restore it (minimize ≠ close).
pub fn spawn_foreground_tracker(app: AppHandle) {
    #[cfg(windows)]
    std::thread::spawn(move || {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
        loop {
            std::thread::sleep(Duration::from_millis(50));
            let Some(fg) = foreground_hwnd() else {
                continue;
            };

            // Alt+Tab / task switcher focused a parked app → bring it back on-screen.
            // IMPORTANT: right after our SW_MINIMIZE the hwnd can still be foreground while
            // iconic — treating that as "switcher restore" unparks immediately and the
            // real window flashes before (or instead of) the genie animation.
            if let Some(state) = app.try_state::<GenieState>() {
                if let Ok(mut g) = state.inner.lock() {
                    if g.busy || g.restoring.contains(&fg) {
                        // Dock restore / suck in flight — never race-unpark.
                    } else {
                        let hit = g
                            .slots
                            .iter()
                            .find(|(_, s)| {
                                matches!(s.kind, GenieKind::DockApp) && s.hwnd == fg
                            })
                            .map(|(id, s)| (id.clone(), s.placement.clone(), s.hwnd));
                        if let Some((item_id, placement, hwnd)) = hit {
                            let iconic = unsafe {
                                use windows::Win32::UI::WindowsAndMessaging::IsIconic;
                                IsIconic(HWND(hwnd as *mut _)).as_bool()
                            };
                            if iconic {
                                // Still minimized — not a real switcher restore yet.
                            } else {
                                g.slots.remove(&item_id);
                                note_parked(&item_id, false);
                                drop(g);
                                let _ = unpark_window(hwnd, &placement);
                                let _ = focus_window(hwnd);
                                note_user_foreground(hwnd);
                                eprintln!(
                                    "[genie] restored parked hwnd via switcher focus ({item_id})"
                                );
                                continue;
                            }
                        }
                    }
                }
            }

            if is_hub_chrome_hwnd(fg) {
                continue;
            }
            let mut pid = 0u32;
            unsafe {
                GetWindowThreadProcessId(HWND(fg as *mut _), Some(&mut pid));
            }
            if pid == 0 || pid == std::process::id() {
                continue;
            }
            let (exe, name) = process_exe(pid);
            let exe = exe.unwrap_or_default();
            let name = name.unwrap_or_default();
            if exe.is_empty() && name.is_empty() {
                continue;
            }
            if let Ok(mut g) = LAST_USER_FG.lock() {
                *g = Some(LastFg { hwnd: fg, exe, name });
            }

            // Keep a ready JPEG for OS title-bar minimize (capture-before-gone).
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let last = LAST_PREP_CAP_MS.load(AtomicOrdering::Relaxed);
            if now_ms.saturating_sub(last) >= 120 && dock_item_id_for_hwnd(fg).is_some() {
                LAST_PREP_CAP_MS.store(now_ms, AtomicOrdering::Relaxed);
                refresh_prepared_minimize(fg);
            }
        }
    });
    #[cfg(not(windows))]
    {
        let _ = app;
    }
}

fn item_matches_exe(item: &DockItem, exe: &str, name: &str) -> bool {
    let want = item.match_exe.to_ascii_lowercase();
    let real = item.real_path.to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    let exe = exe.to_ascii_lowercase();
    if !want.is_empty()
        && (name == want
            || format!("{name}.exe") == want
            || name == want.trim_end_matches(".exe")
            || exe.ends_with(&want)
            || (!want.ends_with(".exe") && exe.ends_with(&format!("{want}.exe"))))
    {
        return true;
    }
    if !real.is_empty() && !exe.is_empty() && (exe == real || exe.ends_with(&real)) {
        return true;
    }
    false
}

fn item_is_user_fg_pure(item: &DockItem) -> bool {
    if let Some(fg) = foreground_hwnd() {
        if !is_hub_chrome_hwnd(fg) {
            let wins = list_windows(None);
            if matching_windows(item, &wins).iter().any(|w| w.hwnd == fg) {
                return true;
            }
            #[cfg(windows)]
            {
                use windows::Win32::Foundation::HWND;
                use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
                let mut pid = 0u32;
                unsafe {
                    GetWindowThreadProcessId(HWND(fg as *mut _), Some(&mut pid));
                }
                if pid != 0 {
                    let (exe, name) = process_exe(pid);
                    if item_matches_exe(
                        item,
                        exe.as_deref().unwrap_or(""),
                        name.as_deref().unwrap_or(""),
                    ) {
                        return true;
                    }
                }
            }
        }
    }

    if let Ok(g) = LAST_USER_FG.lock() {
        if let Some(fg) = g.as_ref() {
            if item_matches_exe(item, &fg.exe, &fg.name) {
                return true;
            }
            let wins = list_windows(None);
            if matching_windows(item, &wins)
                .iter()
                .any(|w| w.hwnd == fg.hwnd)
            {
                return true;
            }
        }
    }
    false
}

/// True when this dock tile's window is (or was just) the user foreground app.
pub fn dock_item_is_foreground(item_id: &str) -> bool {
    let Ok(item) = find_dock_item(item_id) else {
        return false;
    };

    if let Ok(g) = MINIMIZE_ARM.lock() {
        if let Some(arm) = g.as_ref() {
            if arm.item_id == item_id && std::time::Instant::now() < arm.until {
                return true;
            }
        }
    }

    item_is_user_fg_pure(&item)
}

/// Sample FG on pointerdown and arm minimize for ~1.2s so click still sees it.
#[tauri::command]
pub fn genie_arm_minimize_intent(item_id: String) -> bool {
    let Ok(item) = find_dock_item(&item_id) else {
        return false;
    };
    let live = item_is_user_fg_pure(&item);
    if live {
        if let Ok(mut g) = MINIMIZE_ARM.lock() {
            *g = Some(MinimizeArm {
                item_id: item_id.clone(),
                until: std::time::Instant::now() + Duration::from_millis(1200),
            });
        }
        eprintln!("[genie] arm minimize intent for {item_id}");
    }
    live
}

#[tauri::command]
pub fn genie_is_parked(state: State<'_, GenieState>, item_id: String) -> bool {
    state
        .inner
        .lock()
        .map(|g| g.slots.contains_key(&item_id))
        .unwrap_or(false)
}

/// Non-command check for launch_or_focus — avoid SW_RESTORE flashing parked apps.
pub fn item_is_genie_parked(item_id: &str) -> bool {
    PARKED_IDS
        .lock()
        .map(|g| g.contains(item_id))
        .unwrap_or(false)
}

static PARKED_IDS: LazyLock<Mutex<std::collections::HashSet<String>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

fn note_parked(item_id: &str, parked: bool) {
    if let Ok(mut g) = PARKED_IDS.lock() {
        if parked {
            g.insert(item_id.to_string());
        } else {
            g.remove(item_id);
        }
    }
}

#[tauri::command]
pub fn genie_item_is_foreground(item_id: String) -> bool {
    dock_item_is_foreground(&item_id)
}

#[tauri::command]
pub fn genie_overlay_painted(state: State<'_, GenieState>, request_id: String) -> Result<(), String> {
    let mut g = state.inner.lock().map_err(|e| e.to_string())?;
    if let Some((id, tx)) = g.pending_painted.take() {
        if id == request_id {
            let _ = tx.send(());
        } else {
            g.pending_painted = Some((id, tx));
        }
    }
    Ok(())
}

#[tauri::command]
pub fn genie_overlay_done(state: State<'_, GenieState>, request_id: String) -> Result<(), String> {
    let mut g = state.inner.lock().map_err(|e| e.to_string())?;
    if let Some((id, tx)) = g.pending_done.take() {
        if id == request_id {
            let _ = tx.send(());
        } else {
            // Stale / mismatched — put back if still relevant
            g.pending_done = Some((id, tx));
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn genie_minimize_app(
    app: AppHandle,
    state: State<'_, GenieState>,
    item_id: String,
    icon: GenieRect,
) -> Result<(), String> {
    let _ = &state;
    minimize_app_inner(app, item_id, icon).await
}

/// OS minimize path: use a pre-cached frame so we never screenshot after the window is gone.
pub async fn minimize_app_from_prepared(
    app: AppHandle,
    item_id: String,
    icon: GenieRect,
    prepared: PreparedMinimize,
) -> Result<(), String> {
    let Some(state) = app.try_state::<GenieState>() else {
        return Err("genie state missing".into());
    };
    if state.is_busy_or_parked(&item_id) {
        return Ok(());
    }

    let hwnd = prepared.hwnd;
    let placement = prepared.placement;
    let restore_rect = prepared.restore_rect;
    let jpeg = prepared.jpeg;
    let frame_path = prepared.frame_path;
    if jpeg.is_empty() {
        return Err("empty prepared capture".into());
    }

    let icon_rect = GenieRect {
        x: icon.x,
        y: icon.y,
        w: icon.w.max(8.0),
        h: icon.h.max(8.0),
    };

    let (_id, rx_done, painted, _created) = emit_genie_play(
        &app,
        &state,
        &jpeg,
        Some(frame_path.as_str()),
        &restore_rect,
        &icon_rect,
        "suck",
    )
    .await?;

    // Even if paint is slightly late, overlay HWND is already up — minimize under it.
    // Waiting longer here is what made the title-bar minimize button feel "顿一下".
    let _ = painted;
    minimize_window_os(hwnd)?;
    {
        let mut g = state.inner.lock().map_err(|e| e.to_string())?;
        g.slots.insert(
            item_id.clone(),
            GenieSlot {
                hwnd,
                placement,
                frame_jpeg: jpeg,
                restore_rect,
                kind: GenieKind::DockApp,
            },
        );
        note_parked(&item_id, true);
        if let Ok(mut arm) = MINIMIZE_ARM.lock() {
            *arm = None;
        }
    }

    wait_genie_done(rx_done, &state).await;
    hide_overlay(&app);
    Ok(())
}

/// Shared by Dock click (live capture while window still visible).
pub async fn minimize_app_inner(
    app: AppHandle,
    item_id: String,
    icon: GenieRect,
) -> Result<(), String> {
    let Some(state) = app.try_state::<GenieState>() else {
        return Err("genie state missing".into());
    };
    if state.is_busy_or_parked(&item_id) {
        return Ok(());
    }

    let item = find_dock_item(&item_id)?;
    let hwnd = pick_target_hwnd(&item)?;

    // Prefer hot cache — live capture on click is a hitch.
    if let Some(prepared) = take_prepared_minimize(hwnd, &item_id) {
        return minimize_app_from_prepared(app, item_id, icon, prepared).await;
    }

    // Freeze geometry BEFORE any overlay delay so restore size matches what user sees.
    let placement = snapshot_window(hwnd)?;
    let restore_rect = hwnd_screen_rect_logical(hwnd)?;
    let frame = capture_window_jpeg_genie(hwnd).map_err(|e| format!("capture failed: {e}"))?;
    if frame.jpeg.is_empty() {
        return Err("empty capture".into());
    }
    let frame_path = write_genie_frame_bytes(&item_id, &frame.jpeg)
        .ok_or_else(|| "write genie frame failed".to_string())?;

    if let Ok(mut g) = PREPARED_MIN.lock() {
        *g = Some(PreparedMinimize {
            hwnd,
            item_id: item_id.clone(),
            placement: placement.clone(),
            restore_rect,
            jpeg: frame.jpeg.clone(),
            frame_path: frame_path.clone(),
            at: std::time::Instant::now(),
        });
    }

    minimize_app_from_prepared(
        app,
        item_id,
        icon,
        PreparedMinimize {
            hwnd,
            item_id: String::new(),
            placement,
            restore_rect,
            jpeg: frame.jpeg,
            frame_path,
            at: std::time::Instant::now(),
        },
    )
    .await
}

#[tauri::command]
pub async fn genie_restore_app(
    app: AppHandle,
    state: State<'_, GenieState>,
    item_id: String,
    icon: GenieRect,
) -> Result<(), String> {
    let slot = {
        let mut g = state.inner.lock().map_err(|e| e.to_string())?;
        let s = g.slots.remove(&item_id);
        if s.is_some() {
            note_parked(&item_id, false);
        }
        s
    };
    let Some(slot) = slot else {
        // Not parked — fall back to normal launch/focus
        return super::dock_launch_item(item_id);
    };

    let icon_rect = GenieRect {
        x: icon.x,
        y: icon.y,
        w: icon.w.max(8.0),
        h: icon.h.max(8.0),
    };

    {
        let mut g = state.inner.lock().map_err(|e| e.to_string())?;
        g.restoring.insert(slot.hwnd);
    }

    // Keep the real HWND iconic until the freeze has covered it — otherwise a
    // concurrent focus/restore shows the live window under/before the mesh.
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindow, ShowWindow, SW_MINIMIZE};
        let hwnd = HWND(slot.hwnd as *mut _);
        unsafe {
            if IsWindow(hwnd).as_bool() && !IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_MINIMIZE);
            }
        }
    }

    let play = play_and_wait_ex(
        &app,
        &state,
        &slot.frame_jpeg,
        slot.restore_rect,
        icon_rect,
        "expand",
        false, // keep overlay up until HWND is shown underneath
    )
    .await;

    // Only reveal the real window after the expand mesh finished (overlay still up).
    let unpark = if play.is_ok() {
        unpark_window_under_cover(slot.hwnd, &slot.placement)
    } else {
        // Animation failed — still restore, but without pretending we covered it.
        unpark_window(slot.hwnd, &slot.placement)
    };
    if play.is_ok() {
        if let Some(w) = app.get_webview_window(OVERLAY_LABEL) {
            let _ = w.set_always_on_top(true);
            #[cfg(windows)]
            if let Ok(hwnd) = w.hwnd() {
                let _ = place_overlay_above(hwnd.0 as isize);
            }
        }
        tauri::async_runtime::spawn_blocking(|| std::thread::sleep(Duration::from_millis(48)))
            .await
            .ok();
    }
    hide_overlay(&app);
    let _ = focus_window(slot.hwnd);

    {
        if let Ok(mut g) = state.inner.lock() {
            g.restoring.remove(&slot.hwnd);
        }
    }
    play?;
    unpark?;
    Ok(())
}

#[cfg(windows)]
fn place_overlay_above(hwnd_raw: isize) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };
    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
        .is_ok()
    }
}

/// Hide a Host popup webview with genie suck toward an anchor rect.
#[tauri::command]
pub async fn genie_hide_popup(
    app: AppHandle,
    state: State<'_, GenieState>,
    slot_id: String,
    window_label: String,
    anchor: GenieRect,
) -> Result<(), String> {
    let win = app
        .get_webview_window(&window_label)
        .ok_or_else(|| format!("window not found: {window_label}"))?;
    if !win.is_visible().unwrap_or(false) {
        return Ok(());
    }
    let hwnd = win.hwnd().map(|h| h.0 as isize).map_err(|e| e.to_string())?;
    let restore_rect = hwnd_screen_rect_logical(hwnd).unwrap_or(GenieRect {
        x: anchor.x,
        y: anchor.y - 200.0,
        w: 280.0,
        h: 320.0,
    });
    let frame = capture_window_jpeg(hwnd, Roi::default()).unwrap_or(crate::win32::capture::CapturedFrame {
        jpeg: Vec::new(),
        width: 1,
        height: 1,
    });

    // Prefer hide (keep webview) over park for Host popups.
    let _ = win.hide();

    match window_label.as_str() {
        "plugin-popup" => {
            let _ = app.emit("plugin-popup-closed", ());
        }
        "tray-popup" => {
            let _ = app.emit("tray-popup-closed", ());
        }
        "status-menu-popup" => {
            let _ = app.emit("status-menu-popup-closed", ());
        }
        _ => {}
    }

    if !frame.jpeg.is_empty() {
        {
            let mut g = state.inner.lock().map_err(|e| e.to_string())?;
            g.slots.insert(
                slot_id.clone(),
                GenieSlot {
                    hwnd,
                    placement: PlacementSnapshot {
                        flags: 0,
                        show_cmd: 0,
                        min_x: 0,
                        min_y: 0,
                        max_x: 0,
                        max_y: 0,
                        normal_left: 0,
                        normal_top: 0,
                        normal_right: 0,
                        normal_bottom: 0,
                        screen_left: 0,
                        screen_top: 0,
                        screen_right: 0,
                        screen_bottom: 0,
                        style: 0,
                        ex_style: 0,
                    },
                    frame_jpeg: frame.jpeg.clone(),
                    restore_rect,
                    kind: GenieKind::Popup {
                        label: window_label.clone(),
                    },
                },
            );
        }
        let _ = play_and_wait(
            &app,
            &state,
            &frame.jpeg,
            restore_rect,
            GenieRect {
                x: anchor.x,
                y: anchor.y,
                w: anchor.w.max(8.0),
                h: anchor.h.max(8.0),
            },
            "suck",
        )
        .await;
    }
    Ok(())
}

#[tauri::command]
pub async fn genie_show_popup(
    app: AppHandle,
    state: State<'_, GenieState>,
    slot_id: String,
    window_label: String,
    anchor: GenieRect,
    x: f64,
    y: f64,
) -> Result<bool, String> {
    let slot = {
        let mut g = state.inner.lock().map_err(|e| e.to_string())?;
        g.slots.remove(&slot_id)
    };
    let Some(slot) = slot else {
        return Ok(false);
    };
    let win = match app.get_webview_window(&window_label) {
        Some(w) => w,
        None => return Ok(false),
    };

    let target = GenieRect {
        x,
        y,
        w: slot.restore_rect.w,
        h: slot.restore_rect.h,
    };
    let icon = GenieRect {
        x: anchor.x,
        y: anchor.y,
        w: anchor.w.max(8.0),
        h: anchor.h.max(8.0),
    };

    if !slot.frame_jpeg.is_empty() {
        let _ = play_and_wait(&app, &state, &slot.frame_jpeg, target, icon, "expand").await;
    }

    let _ = win.set_position(LogicalPosition::new(x, y));
    let _ = win.set_size(LogicalSize::new(target.w.max(80.0), target.h.max(40.0)));
    let _ = win.show();
    let _ = win.set_focus();
    match window_label.as_str() {
        "plugin-popup" => {
            let pid = slot_id
                .strip_prefix("plugin:")
                .unwrap_or(slot_id.as_str())
                .to_string();
            let _ = app.emit("plugin-popup-opened", pid);
        }
        "tray-popup" => {
            let _ = app.emit("tray-popup-opened", ());
        }
        "status-menu-popup" => {
            let _ = app.emit("status-menu-popup-opened", ());
        }
        _ => {}
    }
    Ok(true)
}

/// Drop a parked slot without animating (e.g. window died).
#[tauri::command]
pub fn genie_forget(state: State<'_, GenieState>, item_id: String) {
    if let Ok(mut g) = state.inner.lock() {
        g.slots.remove(&item_id);
    }
    note_parked(&item_id, false);
}
