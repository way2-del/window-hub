//! Dock show/hide policy for the 8 display modes.
//!
//! AutoHide / SmartHide (pointer part) 鈥?three rules only:
//! 1. Hidden 鈫?bottom reveal strip on the **dock鈥檚 monitor** shows the dock
//! 2. Shown  鈫?pointer inside the **exact dock window rect** (or dock HWND) keeps it
//! 3. Outside that area for `hide_linger_ms` 鈫?hide
//!
//! AutoHide additionally stays visible on the desktop / when nothing covers the
//! dock bar (same clear-area idea as SmartHide), and `tick` honors that via `want`.
//!
//! Geometry always uses the dock window鈥檚 monitor (not the cursor鈥檚) so a stacked
//! upper display鈥檚 bottom edge (often y=0) cannot drive primary-dock reveal/hide.
//! Animation is never cancelled mid-slide (`busy`).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

use super::{DockActivationPosition, DockDisplayMode};

const POLL_MS: u64 = 50;
/// Brief settle after show anim 鈥?blocks leave while the pointer settles onto the bar.
const SETTLE_MS: u64 = 900;
/// Hidden-state reveal: logical px at the monitor bottom edge (prefs may be thinner).
/// Kept tiny so 鈥渟lightly above the bottom鈥?does not show the dock.
const REVEAL_THICK_MAX: u32 = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockVisibilityState {
    pub visible: bool,
    pub reason: String,
}

#[derive(Debug, Clone)]
struct SurfaceVis {
    desired: bool,
    shown: bool,
    busy: bool,
    hide_deadline: Option<Instant>,
    shown_at: Option<Instant>,
    near_streak: u32,
    away_streak: u32,
    mouse_near: bool,
    last_reason: String,
}

impl Default for SurfaceVis {
    fn default() -> Self {
        Self {
            desired: true,
            shown: true,
            busy: false,
            hide_deadline: None,
            shown_at: None,
            near_streak: 0,
            away_streak: 0,
            mouse_near: false,
            last_reason: "init".into(),
        }
    }
}

struct VisInner {
    mode: DockDisplayMode,
    force_show: bool,
    activation_position: DockActivationPosition,
    activation_thickness_px: u32,
    bottom_offset_px: u32,
    hide_linger_ms: u32,
    /// Frontend drag / modal 鈥?keep AutoHide from collapsing the bar.
    interaction_hold: bool,
    /// Interactive window-preview tip is open 鈥?keep AutoHide while cursor moves onto it.
    preview_tip_keep: bool,
    /// Per dock HWND label (`dock`, `dock-sat-*`) 鈥?same FSM, per-monitor inputs.
    surfaces: HashMap<String, SurfaceVis>,
}

pub struct DockVisibility {
    inner: Mutex<VisInner>,
    running: AtomicBool,
}

impl DockVisibility {
    pub fn new() -> Arc<Self> {
        let mut surfaces = HashMap::new();
        surfaces.insert("dock".into(), SurfaceVis::default());
        Arc::new(Self {
            inner: Mutex::new(VisInner {
                mode: DockDisplayMode::Default,
                force_show: false,
                activation_position: DockActivationPosition::ScreenBottom,
                activation_thickness_px: 2,
                bottom_offset_px: 0,
                hide_linger_ms: 800,
                interaction_hold: false,
                preview_tip_keep: false,
                surfaces,
            }),
            running: AtomicBool::new(false),
        })
    }

    pub fn ui_shown(&self) -> bool {
        self.inner
            .lock()
            .ok()
            .and_then(|g| g.surfaces.get("dock").map(|s| s.shown))
            .unwrap_or(true)
    }

    /// Last applied visibility for a `dock-sat-*` label (None = not tracked yet).
    pub fn sat_is_shown(&self, label: &str) -> Option<bool> {
        self.inner
            .lock()
            .ok()
            .and_then(|g| g.surfaces.get(label).map(|s| s.shown))
    }

    pub fn is_busy(&self) -> bool {
        self.inner
            .lock()
            .ok()
            .and_then(|g| g.surfaces.get("dock").map(|s| s.busy))
            .unwrap_or(false)
    }

    pub fn set_mode(&self, mode: DockDisplayMode) {
        if let Ok(mut g) = self.inner.lock() {
            g.mode = mode;
            if mode != DockDisplayMode::Hotkey {
                g.force_show = false;
            }
        }
    }

    pub fn set_activation(
        &self,
        position: DockActivationPosition,
        thickness_px: u32,
        bottom_offset_px: u32,
        hide_linger_ms: u32,
    ) {
        if let Ok(mut g) = self.inner.lock() {
            g.activation_position = position;
            g.activation_thickness_px = thickness_px.clamp(1, 64);
            g.bottom_offset_px = bottom_offset_px.min(400);
            g.hide_linger_ms = hide_linger_ms.clamp(200, 10_000);
        }
    }

    pub fn apply_prefs(&self, prefs: &super::DockPrefs) {
        self.set_mode(prefs.mode());
        self.set_activation(
            prefs.activation(),
            prefs.activation_thickness_px,
            prefs.bottom_offset_px,
            prefs.hide_linger_ms,
        );
    }

    pub fn toggle_hotkey(&self) {
        if let Ok(mut g) = self.inner.lock() {
            if g.mode == DockDisplayMode::Hotkey {
                g.force_show = !g.force_show;
            }
        }
    }

    /// Global hotkey path (from `hotkey_registry`).
    pub fn apply_hotkey_toggle(self: &Arc<Self>, app: &AppHandle) {
        self.toggle_hotkey();
        self.tick(app);
    }

    /// Kept for IPC compatibility 鈥?AutoHide ignores this (native geometry only).
    pub fn set_mouse_near_bottom(&self, _near: bool) {}

    /// Hold AutoHide open (e.g. icon reorder drag leaving the chrome strip).
    pub fn set_interaction_hold(&self, hold: bool) {
        if let Ok(mut g) = self.inner.lock() {
            g.interaction_hold = hold;
            if hold {
                for s in g.surfaces.values_mut() {
                    s.hide_deadline = None;
                    s.desired = true;
                }
            }
        }
    }

    pub fn is_interaction_hold(&self) -> bool {
        self.inner
            .lock()
            .map(|g| g.interaction_hold)
            .unwrap_or(false)
    }

    /// Keep AutoHide while an interactive Dock window-preview tip is visible
    /// (cursor must leave the dock HWND to reach the tip above it).
    pub fn set_preview_tip_keep(&self, keep: bool) {
        if let Ok(mut g) = self.inner.lock() {
            g.preview_tip_keep = keep;
            if keep {
                for s in g.surfaces.values_mut() {
                    s.hide_deadline = None;
                    s.desired = true;
                }
            }
        }
    }

    pub fn start(self: &Arc<Self>, app: AppHandle) {
        if self
            .running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let this = Arc::clone(self);
        let app_poll = app.clone();
        std::thread::spawn(move || {
            while this.running.load(Ordering::SeqCst) {
                this.tick(&app_poll);
                std::thread::sleep(Duration::from_millis(POLL_MS));
            }
        });
        // Global Dock hotkey: `hotkey_registry` system.dockToggle.
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Current policy snapshot (for UI). AutoHide reports hysteresis, not raw edge.
    pub fn snapshot(&self, app: &AppHandle) -> DockVisibilityState {
        let near = self.poll_pointer(app);
        let (want, reason) = self.compute_want(app, near);
        let Ok(g) = self.inner.lock() else {
            return DockVisibilityState {
                visible: want,
                reason,
            };
        };
        let prim = g.surfaces.get("dock");
        let visible = match g.mode {
            DockDisplayMode::AutoHide | DockDisplayMode::SmartHide => prim
                .map(|s| s.shown || s.desired)
                .unwrap_or(want),
            _ => want,
        };
        let reason = if visible && !want {
            prim.map(|s| s.last_reason.clone()).unwrap_or(reason)
        } else {
            reason
        };
        DockVisibilityState { visible, reason }
    }

    /// Snap/animate to a visibility target (window create / prefs apply / external sync).
    /// Applies the **same** target to every dock surface (primary + satellites).
    pub fn apply_dock_shown(self: &Arc<Self>, app: &AppHandle, visible: bool, animate: bool) {
        let prefs = super::load_dock_prefs();
        {
            let Ok(mut g) = self.inner.lock() else {
                return;
            };
            for (label, _) in list_dock_surfaces(app) {
                g.surfaces.entry(label).or_default();
            }
            if g.surfaces.values().any(|s| s.busy) {
                return;
            }
            for s in g.surfaces.values_mut() {
                s.desired = visible;
                s.busy = true;
            }
        }
        eprintln!(
            "[dock-vis] apply place visible={} animate={} (all surfaces)",
            u8::from(visible),
            u8::from(animate)
        );
        if !visible {
            super::set_hover_expanded_pub(false);
            let _ = app.emit(
                "dock-visibility",
                &DockVisibilityState {
                    visible: false,
                    reason: if animate { "apply-anim" } else { "apply-snap" }.into(),
                },
            );
            std::thread::sleep(Duration::from_millis(48));
        }
        super::place_dock_window_primary(app, &prefs, visible, animate);
        for (label, _) in list_dock_surfaces(app) {
            if label == "dock" {
                continue;
            }
            super::place_one_dock_satellite(app, &prefs, &label, visible, animate);
        }
        if let Ok(mut g) = self.inner.lock() {
            let reason = if animate { "apply-anim" } else { "apply-snap" };
            for s in g.surfaces.values_mut() {
                s.shown = visible;
                s.busy = false;
                s.desired = visible;
                s.hide_deadline = None;
                s.shown_at = if visible {
                    Some(Instant::now())
                } else {
                    None
                };
                s.last_reason = reason.into();
            }
        }
        if visible {
            let _ = app.emit(
                "dock-visibility",
                &DockVisibilityState {
                    visible,
                    reason: if animate { "apply-anim" } else { "apply-snap" }.into(),
                },
            );
        }
    }

    fn tick(self: &Arc<Self>, app: &AppHandle) {
        let prefs = super::load_dock_prefs();
        if !prefs.enabled {
            return;
        }
        let (mode, force, thick, bottom, linger_ms, hold) = {
            let Ok(g) = self.inner.lock() else {
                return;
            };
            (
                g.mode,
                g.force_show,
                g.activation_thickness_px,
                g.bottom_offset_px,
                g.hide_linger_ms,
                g.interaction_hold || g.preview_tip_keep,
            )
        };
        let linger = Duration::from_millis(linger_ms as u64);
        let surfaces = list_dock_surfaces(app);
        if let Ok(mut g) = self.inner.lock() {
            for (label, _) in &surfaces {
                g.surfaces.entry(label.clone()).or_default();
            }
        }

        let mut places: Vec<(String, bool, String)> = Vec::new();
        let mut leave_logs: Vec<(String, u32, u128)> = Vec::new();
        let mut leave_fired: Vec<String> = Vec::new();

        for (label, hwnd) in &surfaces {
            let was_shown = self
                .inner
                .lock()
                .ok()
                .and_then(|g| g.surfaces.get(label).map(|s| s.shown))
                .unwrap_or(true);
            let near_raw = if hold {
                true
            } else {
                pointer_near_dock_hwnd(app, *hwnd, was_shown, thick, bottom)
            };
            let (want, reason) =
                compute_want_for_dock_hwnd(app, *hwnd, mode, force, near_raw);

            let Ok(mut g) = self.inner.lock() else {
                continue;
            };
            let Some(surf) = g.surfaces.get_mut(label) else {
                continue;
            };
            if near_raw {
                surf.near_streak = surf.near_streak.saturating_add(1);
                surf.away_streak = 0;
            } else {
                surf.away_streak = surf.away_streak.saturating_add(1);
                surf.near_streak = 0;
            }
            let near = surf.near_streak >= 2;
            let away = surf.away_streak >= 2;
            surf.mouse_near = near;

            let uses_linger = matches!(
                mode,
                DockDisplayMode::AutoHide | DockDisplayMode::SmartHide
            );
            let fullscreen_hide = reason == "fullscreen";
            let settling = surf
                .shown_at
                .is_some_and(|t| t.elapsed() < Duration::from_millis(SETTLE_MS));

            if uses_linger {
                let want_eff = if fullscreen_hide {
                    false
                } else {
                    want || near
                };
                if surf.busy {
                    // In-flight slide.
                } else if fullscreen_hide {
                    surf.hide_deadline = None;
                    surf.desired = false;
                    surf.shown_at = None;
                } else if surf.shown && settling {
                    surf.hide_deadline = None;
                    surf.desired = true;
                } else if want_eff {
                    if want || surf.hide_deadline.is_none() || surf.near_streak >= 4 {
                        surf.hide_deadline = None;
                    }
                    surf.desired = true;
                } else if surf.shown && away {
                    match surf.hide_deadline {
                        None => {
                            surf.hide_deadline = Some(Instant::now() + linger);
                            surf.desired = true;
                            let shown_ms = surf
                                .shown_at
                                .map(|t| t.elapsed().as_millis())
                                .unwrap_or(0);
                            leave_logs.push((label.clone(), linger_ms, shown_ms));
                        }
                        Some(deadline) if Instant::now() >= deadline => {
                            surf.desired = false;
                            surf.hide_deadline = None;
                            leave_fired.push(label.clone());
                        }
                        Some(_) => {
                            surf.desired = true;
                        }
                    }
                } else if surf.shown {
                    surf.desired = true;
                } else {
                    surf.desired = false;
                    surf.hide_deadline = None;
                }
            } else {
                surf.desired = want;
                surf.hide_deadline = None;
            }

            surf.last_reason = reason.clone();
            let desired = surf.desired;
            let shown = surf.shown;
            let busy = surf.busy;
            if !busy && desired != shown {
                surf.busy = true;
                places.push((label.clone(), desired, reason));
            }
        }

        for (label, linger_ms, shown_ms) in leave_logs {
            eprintln!(
                "[dock-vis] {} leave-timer start linger={}ms shown_for={}ms cursor={:?}",
                label,
                linger_ms,
                shown_ms,
                cursor_pos_px()
            );
        }
        for label in leave_fired {
            eprintln!(
                "[dock-vis] {} leave-timer fired → hide cursor={:?}",
                label,
                cursor_pos_px()
            );
        }

        for (label, target, why) in places {
            let near_log = self
                .inner
                .lock()
                .ok()
                .and_then(|g| g.surfaces.get(&label).map(|s| s.mouse_near))
                .unwrap_or(false);
            eprintln!(
                "[dock-vis] {} mode={} near={} shown→{} reason={}",
                label,
                mode.as_str(),
                u8::from(near_log),
                u8::from(target),
                why
            );
            if label == "dock" && !target {
                super::set_hover_expanded_pub(false);
                let _ = app.emit(
                    "dock-visibility",
                    &DockVisibilityState {
                        visible: false,
                        reason: why.clone(),
                    },
                );
                std::thread::sleep(Duration::from_millis(48));
            }
            if label == "dock" {
                super::place_dock_window_primary(app, &prefs, target, true);
                if target {
                    let _ = app.emit(
                        "dock-visibility",
                        &DockVisibilityState {
                            visible: true,
                            reason: why,
                        },
                    );
                }
            } else {
                super::place_one_dock_satellite(app, &prefs, &label, target, true);
            }
            if let Ok(mut g) = self.inner.lock() {
                if let Some(surf) = g.surfaces.get_mut(&label) {
                    surf.shown = target;
                    surf.busy = false;
                    if target {
                        surf.desired = true;
                        surf.hide_deadline = None;
                        surf.shown_at = Some(Instant::now());
                    } else {
                        surf.shown_at = None;
                    }
                }
            }
        }
    }

    #[allow(dead_code)]
    fn mode_str(&self) -> &'static str {
        self.inner
            .lock()
            .map(|g| g.mode.as_str())
            .unwrap_or("default")
    }

    /// `want` = policy wants primary dock visible (before leave-linger).
    fn compute_want(&self, app: &AppHandle, near: bool) -> (bool, String) {
        let Ok(g) = self.inner.lock() else {
            return (true, "lock".into());
        };
        let mode = g.mode;
        let force = g.force_show;
        drop(g);

        let self_hwnd = app
            .get_webview_window("dock")
            .and_then(|w| w.hwnd().ok().map(|h| h.0 as isize))
            .or_else(|| {
                app.get_webview_window("main")
                    .and_then(|w| w.hwnd().ok().map(|h| h.0 as isize))
            });
        let Some(hwnd) = self_hwnd else {
            return (true, "no-hwnd".into());
        };
        compute_want_for_dock_hwnd(app, hwnd, mode, force, near)
    }

    fn poll_pointer(&self, app: &AppHandle) -> bool {
        #[cfg(windows)]
        {
            let (thick_log, bottom_off, shown, hold) = {
                let Ok(g) = self.inner.lock() else {
                    return false;
                };
                let shown = g
                    .surfaces
                    .get("dock")
                    .map(|s| s.shown)
                    .unwrap_or(false);
                (
                    g.activation_thickness_px,
                    g.bottom_offset_px,
                    shown,
                    g.interaction_hold || g.preview_tip_keep,
                )
            };
            if hold {
                return true;
            }
            let Some(hwnd) = app
                .get_webview_window("dock")
                .and_then(|w| w.hwnd().ok().map(|h| h.0 as isize))
            else {
                return false;
            };
            pointer_near_dock_hwnd(app, hwnd, shown, thick_log, bottom_off)
        }
        #[cfg(not(windows))]
        {
            let _ = app;
            false
        }
    }
}

/// Primary + every enabled `dock-sat-*` on the placement snapshot.
fn list_dock_surfaces(app: &AppHandle) -> Vec<(String, isize)> {
    let mut out = Vec::new();
    if let Some(dock) = app.get_webview_window("dock") {
        if let Ok(hwnd) = dock.hwnd() {
            out.push(("dock".into(), hwnd.0 as isize));
        }
    }
    let snap = crate::display_placement::snapshot();
    for r in &snap.resolved {
        if r.is_primary || !r.dock {
            continue;
        }
        let label = crate::display_placement::dock_sat_label(&r.id);
        if let Some(win) = app.get_webview_window(&label) {
            if let Ok(hwnd) = win.hwnd() {
                out.push((label, hwnd.0 as isize));
            }
        }
    }
    out
}

fn cursor_pos_px() -> Option<(i32, i32)> {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        unsafe {
            let mut pt = POINT::default();
            if GetCursorPos(&mut pt).is_err() {
                None
            } else {
                Some((pt.x, pt.y))
            }
        }
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Monitor that hosts the dock HWND (placement + activation geometry).
#[cfg(windows)]
fn dock_monitor_info(
    app: &AppHandle,
) -> Option<(windows::Win32::Graphics::Gdi::MONITORINFO, f64)> {
    use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST};

    let root = dock_root_hwnd_of(app)?;
    unsafe { monitor_info_from(MonitorFromWindow(root, MONITOR_DEFAULTTONEAREST)) }
}

#[cfg(windows)]
unsafe fn monitor_info_from(
    mon: windows::Win32::Graphics::Gdi::HMONITOR,
) -> Option<(windows::Win32::Graphics::Gdi::MONITORINFO, f64)> {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

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
    Some((mi, scale))
}

#[cfg(windows)]
fn point_in_monitor(mi: &windows::Win32::Graphics::Gdi::MONITORINFO, x: i32, y: i32) -> bool {
    x >= mi.rcMonitor.left
        && x < mi.rcMonitor.right
        && y >= mi.rcMonitor.top
        && y < mi.rcMonitor.bottom
}

#[cfg(windows)]
fn point_on_activation_strip(
    app: &AppHandle,
    mi: &windows::Win32::Graphics::Gdi::MONITORINFO,
    scale: f64,
    activation: DockActivationPosition,
    thick_log: u32,
    bottom_off: u32,
    x: i32,
    y: i32,
) -> bool {
    let _ = app;
    let _ = activation;
    let thick = ((thick_log as f64) * scale).round().clamp(1.0, 16.0) as i32;
    let margin = ((bottom_off as f64) * scale).round() as i32;
    // rcMonitor.bottom is exclusive 鈥?last visible row is bottom - 1.
    let zone_bottom = mi.rcMonitor.bottom - 1 - margin;
    let zone_top = zone_bottom - thick + 1;
    // Inclusive [zone_top, zone_bottom] on the last physical pixel rows.
    if y < zone_top || y > zone_bottom {
        return false;
    }
    x >= mi.rcMonitor.left && x < mi.rcMonitor.right
}

/// Dock keep area while shown: resting **content** width, centered in the icons
/// HWND. Height is chrome-only at rest; when hover-expanded (fan live), include
/// magnification headroom so moving onto a peaked icon does not start AutoHide.
/// Never trust the glass HWND width 鈥?live resize bugs made it span almost the
/// full monitor and AutoHide could not leave.
#[cfg(windows)]
fn dock_chrome_keep_rect(
    app: &AppHandle,
    mi: &windows::Win32::Graphics::Gdi::MONITORINFO,
    scale: f64,
    bottom_off: u32,
) -> (i32, i32, i32, i32) {
    let prefs = super::load_dock_prefs();
    let expanded = super::dock_hover_expanded();
    let chrome_h = (super::dock_chrome_height() * scale).round().max(1.0) as i32;
    let keep_h = if expanded {
        (super::dock_window_height(prefs.magnification) * scale)
            .round()
            .max(chrome_h as f64) as i32
    } else {
        chrome_h
    };
    let layout = super::dock_layout_items(&prefs);
    let logical_keep = super::dock_window_width(
        &layout,
        prefs.corner_radius_px,
        prefs.magnification,
        expanded,
    );
    let content_w = (logical_keep * scale).round().max(1.0) as i32;

    if let Some((l, wt, r, b)) = dock_root_screen_rect(app).filter(|(_l, t, _r, _b)| {
        *t < mi.rcMonitor.bottom - 4
    }) {
        let win_w = r - l;
        let left = l + ((win_w - content_w) / 2).max(0);
        // Prefer live HWND top when expanded (exact headroom); else chrome strip.
        let top = if expanded {
            wt.max(mi.rcMonitor.top)
        } else {
            (b - keep_h).max(mi.rcMonitor.top)
        };
        return (left, top, left + content_w, b);
    }
    if let Some((gl, _gt, gr, gb)) = dock_glass_screen_rect(app) {
        if gb > mi.rcMonitor.top + 4 {
            let mid = (gl + gr) / 2;
            let left = mid - content_w / 2;
            let top = (gb - keep_h).max(mi.rcMonitor.top);
            return (left, top, left + content_w, gb);
        }
    }
    dock_rest_pose_rect(
        mi,
        scale,
        bottom_off,
        logical_keep,
        if expanded {
            super::dock_window_height(prefs.magnification)
        } else {
            super::dock_chrome_height()
        },
    )
}

#[cfg(windows)]
fn dock_glass_screen_rect(app: &AppHandle) -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GetWindowRect, GA_ROOT};

    let glass = app.get_webview_window("dock-glass")?;
    let hwnd = glass.hwnd().ok()?;
    unsafe {
        let h = HWND(hwnd.0 as _);
        let root = GetAncestor(h, GA_ROOT);
        let root = if root.0.is_null() { h } else { root };
        let mut wr = RECT::default();
        GetWindowRect(root, &mut wr).ok()?;
        if wr.right <= wr.left || wr.bottom <= wr.top {
            return None;
        }
        Some((wr.left, wr.top, wr.right, wr.bottom))
    }
}

#[cfg(windows)]
fn dock_area_rect_px(app: &AppHandle, bottom_off: u32) -> Option<(i32, i32, i32, i32)> {
    let (mi, scale) = dock_monitor_info(app)?;
    Some(dock_chrome_keep_rect(app, &mi, scale, bottom_off))
}

#[cfg(not(windows))]
fn dock_area_rect_px(_app: &AppHandle, _bottom_off: u32) -> Option<(i32, i32, i32, i32)> {
    None
}

#[cfg(windows)]
fn pointer_in_dock_chrome(
    app: &AppHandle,
    mi: &windows::Win32::Graphics::Gdi::MONITORINFO,
    scale: f64,
    bottom_off: u32,
    x: i32,
    y: i32,
) -> bool {
    let (l, t, r, b) = dock_chrome_keep_rect(app, mi, scale, bottom_off);
    x >= l && x < r && y >= t && y < b
}

#[cfg(windows)]
fn pointer_in_dock_chrome_for_hwnd(
    dock_hwnd: isize,
    mi: &windows::Win32::Graphics::Gdi::MONITORINFO,
    scale: f64,
    bottom_off: u32,
    x: i32,
    y: i32,
) -> bool {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    let prefs = super::load_dock_prefs();
    let expanded = super::hover_expanded_hwnd(dock_hwnd);
    let chrome_h = (super::dock_chrome_height() * scale).round().max(1.0) as i32;
    let keep_h = if expanded {
        (super::dock_window_height(prefs.magnification) * scale)
            .round()
            .max(chrome_h as f64) as i32
    } else {
        chrome_h
    };
    let layout = super::dock_layout_items(&prefs);
    let logical_keep = super::dock_window_width(
        &layout,
        prefs.corner_radius_px,
        prefs.magnification,
        expanded,
    );
    let content_w = (logical_keep * scale).round().max(1.0) as i32;
    let _ = bottom_off;

    unsafe {
        let mut wr = RECT::default();
        let h = HWND(dock_hwnd as *mut _);
        if GetWindowRect(h, &mut wr).is_err() {
            // Fall back to rest pose on this monitor.
            let logical_h = super::dock_window_height(prefs.magnification);
            let (l, t, r, b) =
                dock_rest_pose_rect(mi, scale, prefs.bottom_offset_px, logical_keep, logical_h);
            return x >= l && x < r && y >= t && y < b;
        }
        let win_w = wr.right - wr.left;
        let left = wr.left + ((win_w - content_w) / 2).max(0);
        let top = if expanded {
            wr.top.max(mi.rcMonitor.top)
        } else {
            (wr.bottom - keep_h).max(mi.rcMonitor.top)
        };
        x >= left && x < left + content_w && y >= top && y < wr.bottom
    }
}

/// Screen-space hit of the chrome-hover-tip HWND (window preview sits above Dock).
#[cfg(windows)]
fn pointer_in_chrome_hover_tip(app: &AppHandle, x: i32, y: i32) -> bool {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GetWindowRect, GA_ROOT};

    let Some(tip) = app.get_webview_window("chrome-hover-tip") else {
        return false;
    };
    let Ok(visible) = tip.is_visible() else {
        return false;
    };
    if !visible {
        return false;
    }
    let Ok(hwnd) = tip.hwnd() else {
        return false;
    };
    unsafe {
        let h = HWND(hwnd.0 as _);
        let root = GetAncestor(h, GA_ROOT);
        let root = if root.0.is_null() { h } else { root };
        let mut wr = RECT::default();
        if GetWindowRect(root, &mut wr).is_err() {
            return false;
        }
        if wr.right <= wr.left || wr.bottom <= wr.top {
            return false;
        }
        // Small pad so the gap between dock top and tip bottom does not start leave.
        const PAD: i32 = 10;
        x >= wr.left.saturating_sub(PAD)
            && x < wr.right.saturating_add(PAD)
            && y >= wr.top.saturating_sub(PAD)
            && y < wr.bottom.saturating_add(PAD)
    }
}

/// Bottom-anchored rest pose in screen px (overlap tests).
#[cfg(windows)]
fn dock_rest_pose_rect(
    mi: &windows::Win32::Graphics::Gdi::MONITORINFO,
    scale: f64,
    bottom_offset_px: u32,
    logical_w: f64,
    logical_h: f64,
) -> (i32, i32, i32, i32) {
    let w = (logical_w * scale).round().max(1.0) as i32;
    let h = (logical_h * scale).round().max(1.0) as i32;
    let margin = (bottom_offset_px as f64 * scale).round() as i32;
    let left = mi.rcMonitor.left + ((mi.rcMonitor.right - mi.rcMonitor.left - w) / 2).max(0);
    let top = mi.rcMonitor.bottom - margin - h;
    (left, top, left + w, top + h)
}

#[cfg(windows)]
fn dock_root_hwnd_of(app: &AppHandle) -> Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GA_ROOT};

    let dock = app.get_webview_window("dock")?;
    let hwnd = dock.hwnd().ok()?;
    unsafe {
        let h = HWND(hwnd.0 as _);
        let root = GetAncestor(h, GA_ROOT);
        Some(if root.0.is_null() { h } else { root })
    }
}

#[cfg(windows)]
fn dock_root_screen_rect(app: &AppHandle) -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    let root = dock_root_hwnd_of(app)?;
    unsafe {
        let mut wr = RECT::default();
        GetWindowRect(root, &mut wr).ok()?;
        if wr.right <= wr.left || wr.bottom <= wr.top {
            return None;
        }
        Some((wr.left, wr.top, wr.right, wr.bottom))
    }
}



#[cfg(windows)]
fn is_desktop_foreground() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetClassNameW, GetForegroundWindow, GetWindowTextW, GA_ROOT,
    };
    unsafe {
        let raw = GetForegroundWindow();
        if raw.0.is_null() {
            return true;
        }
        let fg = {
            let root = GetAncestor(raw, GA_ROOT);
            if root.0.is_null() {
                raw
            } else {
                root
            }
        };
        let mut cls = [0u16; 64];
        let n = GetClassNameW(fg, &mut cls);
        let class = String::from_utf16_lossy(&cls[..n as usize]);
        if matches!(
            class.as_str(),
            "Progman" | "WorkerW" | "SHELLDLL_DefView"
        ) {
            return true;
        }
        let mut title = [0u16; 64];
        let tn = GetWindowTextW(fg, &mut title);
        let t = String::from_utf16_lossy(&title[..tn as usize]);
        t.is_empty() && (class == "Shell_TrayWnd" || class.contains("Desktop"))
    }
}

#[cfg(not(windows))]
fn is_desktop_foreground() -> bool {
    false
}

fn is_dock_overlapped(app: &AppHandle) -> bool {
    #[cfg(windows)]
    {
        let Some(dock) = app.get_webview_window("dock") else {
            return false;
        };
        let Ok(hwnd) = dock.hwnd() else {
            return false;
        };
        is_dock_overlapped_hwnd(hwnd.0 as isize)
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        false
    }
}

/// Same policy as primary `compute_want`, but `dock_hwnd` selects the monitor.
fn compute_want_for_dock_hwnd(
    app: &AppHandle,
    dock_hwnd: isize,
    mode: DockDisplayMode,
    force: bool,
    near: bool,
) -> (bool, String) {
    let fullscreen = crate::win32::fullscreen::should_hide_strip(Some(dock_hwnd));
    let fs_hides = matches!(
        mode,
        DockDisplayMode::Default
            | DockDisplayMode::AutoHide
            | DockDisplayMode::SmartHide
            | DockDisplayMode::Desktop
            | DockDisplayMode::Hotkey
    );
    if fullscreen && fs_hides {
        return (false, "fullscreen".into());
    }

    let on_desktop = crate::win32::ambient::is_desktop_scene(Some(dock_hwnd));
    let overlapped = is_dock_overlapped_hwnd(dock_hwnd);
    let _ = app;

    match mode {
        DockDisplayMode::Default => (true, "default".into()),
        DockDisplayMode::Layered | DockDisplayMode::Always => (true, "always".into()),
        DockDisplayMode::AlwaysFullscreen => (true, "alwaysFullscreen".into()),
        DockDisplayMode::AutoHide => {
            if on_desktop || !overlapped {
                (
                    true,
                    if on_desktop {
                        "autoHideDesktop".into()
                    } else {
                        "autoHideClear".into()
                    },
                )
            } else if near {
                (true, "edge".into())
            } else {
                (false, "leave".into())
            }
        }
        DockDisplayMode::SmartHide => {
            if !overlapped || on_desktop {
                (true, "smartShow".into())
            } else if near {
                (true, "smartEdge".into())
            } else {
                (false, "smartHide".into())
            }
        }
        DockDisplayMode::Hotkey => {
            if force {
                (true, "hotkeyOn".into())
            } else {
                (false, "hotkeyOff".into())
            }
        }
        DockDisplayMode::Desktop => {
            if on_desktop {
                (true, "desktop".into())
            } else {
                (false, "notDesktop".into())
            }
        }
    }
}

#[cfg(windows)]
fn pointer_near_dock_hwnd(
    app: &AppHandle,
    dock_hwnd: isize,
    shown: bool,
    thick_log: u32,
    bottom_off: u32,
) -> bool {
    use windows::Win32::Foundation::{HWND, POINT};
    use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    unsafe {
        let mut pt = POINT::default();
        if GetCursorPos(&mut pt).is_err() {
            return false;
        }
        let h = HWND(dock_hwnd as *mut _);
        let Some((mi, scale)) = monitor_info_from(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST))
        else {
            return false;
        };
        if !point_in_monitor(&mi, pt.x, pt.y) {
            return false;
        }
        if shown {
            if pointer_in_dock_chrome_for_hwnd(dock_hwnd, &mi, scale, bottom_off, pt.x, pt.y) {
                return true;
            }
            pointer_in_chrome_hover_tip(app, pt.x, pt.y)
        } else {
            let reveal_thick = thick_log.clamp(1, REVEAL_THICK_MAX);
            point_on_activation_strip(
                app,
                &mi,
                scale,
                DockActivationPosition::ScreenBottom,
                reveal_thick,
                bottom_off,
                pt.x,
                pt.y,
            )
        }
    }
}

#[cfg(not(windows))]
fn pointer_near_dock_hwnd(
    _app: &AppHandle,
    _dock_hwnd: isize,
    _shown: bool,
    _thick_log: u32,
    _bottom_off: u32,
) -> bool {
    false
}

fn is_dock_overlapped_hwnd(dock_hwnd: isize) -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{HWND, RECT};
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        };
        use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
        use windows::Win32::UI::WindowsAndMessaging::{
            GetAncestor, GetForegroundWindow, GetWindowRect, IsWindowVisible, GA_ROOT,
        };

        unsafe {
            let h = HWND(dock_hwnd as *mut _);
            let dock_root = {
                let root = GetAncestor(h, GA_ROOT);
                if root.0.is_null() {
                    h
                } else {
                    root
                }
            };

            let mon = MonitorFromWindow(dock_root, MONITOR_DEFAULTTONEAREST);
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
            let prefs = super::load_dock_prefs();
            let layout = super::dock_layout_items(&prefs);
            let logical_w = super::dock_window_width(
                &layout,
                prefs.corner_radius_px,
                prefs.magnification,
                false,
            );
            let logical_h = super::dock_window_height(prefs.magnification);
            let (l, t, r, b) =
                dock_rest_pose_rect(&mi, scale, prefs.bottom_offset_px, logical_w, logical_h);

            let fg = GetForegroundWindow();
            if fg.0.is_null() || fg == dock_root || fg == h {
                return false;
            }
            if !IsWindowVisible(fg).as_bool() {
                return false;
            }
            // Foreground must be on this dock's monitor.
            let fg_mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
            if fg_mon != mon {
                return false;
            }
            let mut rc = RECT::default();
            if GetWindowRect(fg, &mut rc).is_err() {
                return false;
            }
            !(rc.right <= l || rc.left >= r || rc.bottom <= t || rc.top >= b)
        }
    }
    #[cfg(not(windows))]
    {
        let _ = dock_hwnd;
        false
    }
}
