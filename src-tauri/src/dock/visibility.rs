//! Dock show/hide policy for the 8 display modes.
//!
//! AutoHide / SmartHide (pointer part) — three rules only:
//! 1. Hidden → bottom reveal strip on the **dock’s monitor** shows the dock
//! 2. Shown  → pointer inside the **exact dock window rect** (or dock HWND) keeps it
//! 3. Outside that area for `hide_linger_ms` → hide
//!
//! AutoHide additionally stays visible on the desktop / when nothing covers the
//! dock bar (same clear-area idea as SmartHide), and `tick` honors that via `want`.
//!
//! Geometry always uses the dock window’s monitor (not the cursor’s) so a stacked
//! upper display’s bottom edge (often y=0) cannot drive primary-dock reveal/hide.
//! Animation is never cancelled mid-slide (`busy`).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

use super::{DockActivationPosition, DockDisplayMode};

const POLL_MS: u64 = 50;
/// Brief settle after show anim — blocks leave while the pointer settles onto the bar.
const SETTLE_MS: u64 = 900;
/// Hidden-state reveal: logical px at the monitor bottom edge (prefs may be thinner).
/// Kept tiny so “slightly above the bottom” does not show the dock.
const REVEAL_THICK_MAX: u32 = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockVisibilityState {
    pub visible: bool,
    pub reason: String,
}

struct VisInner {
    mode: DockDisplayMode,
    force_show: bool,
    mouse_near: bool,
    activation_position: DockActivationPosition,
    activation_thickness_px: u32,
    bottom_offset_px: u32,
    hide_linger_ms: u32,
    /// Latest wanted visibility (may differ from `shown` while animating).
    desired: bool,
    /// Matches HWND rest pose after place completes.
    shown: bool,
    busy: bool,
    /// Frontend drag / modal — keep AutoHide from collapsing the bar.
    interaction_hold: bool,
    /// Interactive window-preview tip is open — keep AutoHide while cursor moves onto it.
    preview_tip_keep: bool,
    /// When `Some`, hide only after this instant if still unwanted.
    hide_deadline: Option<Instant>,
    /// When the HWND last finished a show transition.
    shown_at: Option<Instant>,
    /// Consecutive polls with pointer in/out of chrome (hysteresis).
    near_streak: u32,
    away_streak: u32,
    last_reason: String,
    /// Per `dock-sat-*` label visibility (same policy as primary, that monitor).
    sat_shown: HashMap<String, bool>,
}

pub struct DockVisibility {
    inner: Mutex<VisInner>,
    running: AtomicBool,
}

impl DockVisibility {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(VisInner {
                mode: DockDisplayMode::Default,
                force_show: false,
                mouse_near: false,
                activation_position: DockActivationPosition::ScreenBottom,
                activation_thickness_px: 2,
                bottom_offset_px: 0,
                hide_linger_ms: 800,
                desired: false,
                shown: false,
                busy: false,
                interaction_hold: false,
                preview_tip_keep: false,
                hide_deadline: None,
                shown_at: None,
                near_streak: 0,
                away_streak: 0,
                last_reason: "init".into(),
                sat_shown: HashMap::new(),
            }),
            running: AtomicBool::new(false),
        })
    }

    pub fn ui_shown(&self) -> bool {
        self.inner
            .lock()
            .map(|g| g.shown)
            .unwrap_or(true)
    }

    /// Last applied visibility for a `dock-sat-*` label (None = not tracked yet).
    pub fn sat_is_shown(&self, label: &str) -> Option<bool> {
        self.inner
            .lock()
            .ok()
            .and_then(|g| g.sat_shown.get(label).copied())
    }

    pub fn is_busy(&self) -> bool {
        self.inner.lock().map(|g| g.busy).unwrap_or(false)
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

    /// Kept for IPC compatibility — AutoHide ignores this (native geometry only).
    pub fn set_mouse_near_bottom(&self, _near: bool) {}

    /// Hold AutoHide open (e.g. icon reorder drag leaving the chrome strip).
    pub fn set_interaction_hold(&self, hold: bool) {
        if let Ok(mut g) = self.inner.lock() {
            g.interaction_hold = hold;
            if hold {
                g.hide_deadline = None;
                g.desired = true;
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
                g.hide_deadline = None;
                g.desired = true;
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
        let visible = match g.mode {
            DockDisplayMode::AutoHide | DockDisplayMode::SmartHide => g.shown || g.desired,
            _ => want,
        };
        let reason = if visible && !want {
            g.last_reason.clone()
        } else {
            reason
        };
        DockVisibilityState { visible, reason }
    }

    /// Snap/animate to a visibility target (window create / prefs apply / external sync).
    pub fn apply_dock_shown(self: &Arc<Self>, app: &AppHandle, visible: bool, animate: bool) {
        let prefs = super::load_dock_prefs();
        {
            let Ok(mut g) = self.inner.lock() else {
                return;
            };
            g.desired = visible;
            if g.busy {
                return;
            }
            g.busy = true;
        }
        eprintln!(
            "[dock-vis] apply place visible={} animate={}",
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
        super::place_dock_window(app, &prefs, visible, animate);
        if let Ok(mut g) = self.inner.lock() {
            g.shown = visible;
            g.busy = false;
            if visible {
                g.desired = true;
                g.hide_deadline = None;
                g.shown_at = Some(Instant::now());
            } else {
                g.shown_at = None;
            }
            g.last_reason = if animate { "apply-anim" } else { "apply-snap" }.into();
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
        self.sync_satellite_docks(app);
    }

    fn tick(self: &Arc<Self>, app: &AppHandle) {
        let near_raw = self.poll_pointer(app);
        let (want, reason) = self.compute_want(app, near_raw);

        let mut should_place: Option<(bool, bool, String)> = None;
        let mut emit: Option<DockVisibilityState> = None;
        let mut leave_timer_started = None;
        let mut leave_timer_fired = false;

        // State only while locked. HWND getters dispatch to the UI thread,
        // which can be waiting here in set_preview_tip_keep (chrome clicks).
        if let Ok(mut g) = self.inner.lock() {
            if near_raw {
                g.near_streak = g.near_streak.saturating_add(1);
                g.away_streak = 0;
            } else {
                g.away_streak = g.away_streak.saturating_add(1);
                g.near_streak = 0;
            }
            // ~100ms stable at 50ms poll — ignore 1-frame keep-zone flicker.
            let near = g.near_streak >= 2;
            let away = g.away_streak >= 2;
            g.mouse_near = near;
            let shown = g.shown;
            let busy = g.busy;
            let linger = Duration::from_millis(g.hide_linger_ms as u64);
            let settling = g
                .shown_at
                .is_some_and(|t| t.elapsed() < Duration::from_millis(SETTLE_MS));

            let uses_linger = matches!(
                g.mode,
                DockDisplayMode::AutoHide | DockDisplayMode::SmartHide
            );

            if uses_linger {
                // Exclusive FS: never keep dock via bottom-edge `near` (games cursors sit there).
                let fullscreen_hide = reason == "fullscreen";
                // AutoHide/SmartHide: policy `want` (desktop / clear / edge) plus
                // pointer hysteresis. AutoHide previously used `near` only, which
                // ignored on-desktop and made that preference a no-op.
                let want_eff = if fullscreen_hide {
                    false
                } else {
                    want || near
                };

                if busy {
                    // In-flight slide — do not change desired / leave.
                } else if fullscreen_hide {
                    // Drop immediately — skip settle / linger while a game owns the display.
                    g.hide_deadline = None;
                    g.desired = false;
                    g.shown_at = None;
                } else if shown && settling {
                    g.hide_deadline = None;
                    g.desired = true;
                } else if want_eff {
                    // Policy `want` (desktop / clear) cancels leave immediately.
                    // Pointer-only keep still needs a stronger near streak so
                    // edge flicker does not reset the linger timer.
                    if want || g.hide_deadline.is_none() || g.near_streak >= 4 {
                        g.hide_deadline = None;
                    }
                    g.desired = true;
                } else if shown && away {
                    match g.hide_deadline {
                        None => {
                            g.hide_deadline = Some(Instant::now() + linger);
                            g.desired = true;
                            let shown_ms = g
                                .shown_at
                                .map(|t| t.elapsed().as_millis())
                                .unwrap_or(0);
                            leave_timer_started = Some((
                                g.hide_linger_ms,
                                shown_ms,
                                g.bottom_offset_px,
                            ));
                        }
                        Some(deadline) if Instant::now() >= deadline => {
                            g.desired = false;
                            g.hide_deadline = None;
                            leave_timer_fired = true;
                        }
                        Some(_) => {
                            g.desired = true;
                        }
                    }
                } else if shown {
                    // Flicker / not yet stably away — keep visible, don't reset leave timer.
                    g.desired = true;
                } else {
                    g.desired = false;
                    g.hide_deadline = None;
                }
            } else {
                g.desired = want;
                g.hide_deadline = None;
            }

            let desired = g.desired;
            g.last_reason = reason.clone();

            if !busy && desired != shown {
                g.busy = true;
                should_place = Some((desired, true, reason.clone()));
            }

            if should_place.is_some() {
                emit = Some(DockVisibilityState {
                    visible: desired,
                    reason: reason.clone(),
                });
            }
        }

        // Diagnostics must not hold inner across native/Tauri calls or I/O.
        if let Some((linger_ms, shown_ms, bottom_offset_px)) = leave_timer_started {
            let dock_area = dock_area_rect_px(app, bottom_offset_px);
            eprintln!(
                "[dock-vis] leave-timer start linger={}ms shown_for={}ms cursor={:?} dock_area={:?} reason={}",
                linger_ms,
                shown_ms,
                cursor_pos_px(),
                dock_area,
                reason
            );
        }
        if leave_timer_fired {
            eprintln!(
                "[dock-vis] leave-timer fired → hide cursor={:?}",
                cursor_pos_px()
            );
        }

        if let Some((target, animate, why)) = should_place {
            let prev_shown = self.ui_shown();
            let near_log = self
                .inner
                .lock()
                .map(|g| g.mouse_near)
                .unwrap_or(false);
            eprintln!(
                "[dock-vis] mode={} near={} want={} shown={}→{} reason={}",
                self.mode_str(),
                u8::from(near_log),
                u8::from(want),
                u8::from(prev_shown),
                u8::from(target),
                why
            );
            let prefs = super::load_dock_prefs();
            // Hide: notify FE to snap fan *before* the slide. This poll thread is not
            // the UI thread, so WebView can commit rest scales during a short wait —
            // otherwise magnified tiles paint through the tween and freeze mid-CSS
            // when the HWND hides.
            if !target {
                super::set_hover_expanded_pub(false);
                if let Some(ref state) = emit {
                    let _ = app.emit("dock-visibility", state);
                }
                std::thread::sleep(Duration::from_millis(48));
            }
            super::place_dock_window(app, &prefs, target, animate);
            if let Ok(mut g) = self.inner.lock() {
                g.shown = target;
                g.busy = false;
                if target {
                    g.desired = true;
                    g.hide_deadline = None;
                    g.shown_at = Some(Instant::now());
                } else {
                    g.shown_at = None;
                    super::set_hover_expanded_pub(false);
                }
            }
            // Show: emit after place (settle). Hide already emitted above.
            if target {
                if let Some(state) = emit {
                    let _ = app.emit("dock-visibility", &state);
                }
            }
        }
        // Secondary docks: same show/hide policy as primary, scoped to each monitor.
        self.sync_satellite_docks(app);
    }

    /// Evaluate + place each `dock-sat-*` independently (fullscreen / AutoHide / …).
    fn sync_satellite_docks(self: &Arc<Self>, app: &AppHandle) {
        let prefs = super::load_dock_prefs();
        if !prefs.enabled {
            return;
        }
        let (mode, force, thick, bottom, linger, hold) = {
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
        let snap = crate::display_placement::snapshot();
        let mut places: Vec<(String, bool)> = Vec::new();
        for r in &snap.resolved {
            if r.is_primary || !r.dock {
                continue;
            }
            let label = crate::display_placement::dock_sat_label(&r.id);
            let Some(win) = app.get_webview_window(&label) else {
                continue;
            };
            let Ok(hwnd) = win.hwnd() else {
                continue;
            };
            let hwnd_raw = hwnd.0 as isize;
            let was_shown = self
                .inner
                .lock()
                .ok()
                .and_then(|g| g.sat_shown.get(&label).copied())
                .unwrap_or(true);
            let near = if hold {
                true
            } else {
                pointer_near_dock_hwnd(app, hwnd_raw, was_shown, thick, bottom)
            };
            let (want, reason) =
                compute_want_for_dock_hwnd(app, hwnd_raw, mode, force, near);
            // Soft linger: when leaving AutoHide/SmartHide keep shown briefly via
            // was_shown + !near — callers place every tick; only animate on flip.
            let uses_linger = matches!(
                mode,
                DockDisplayMode::AutoHide | DockDisplayMode::SmartHide
            );
            let target = if reason == "fullscreen" {
                false
            } else if uses_linger {
                want || near
            } else {
                want
            };
            let _ = linger; // reserved — primary owns timed leave; sat uses near hysteresis
            if target != was_shown {
                eprintln!(
                    "[dock-vis] sat {} want={} shown={}→{} reason={} near={}",
                    label,
                    u8::from(want),
                    u8::from(was_shown),
                    u8::from(target),
                    reason,
                    u8::from(near)
                );
                places.push((label.clone(), target));
                if let Ok(mut g) = self.inner.lock() {
                    g.sat_shown.insert(label, target);
                }
            }
        }
        for (label, shown) in places {
            super::place_one_dock_satellite(app, &prefs, &label, shown, true);
        }
    }

    fn mode_str(&self) -> &'static str {
        self.inner
            .lock()
            .map(|g| g.mode.as_str())
            .unwrap_or("default")
    }

    /// `want` = policy wants dock visible (before leave-linger).
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
            use windows::Win32::Foundation::POINT;
            use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

            let (thick_log, bottom_off, shown, hold) = {
                let Ok(g) = self.inner.lock() else {
                    return false;
                };
                (
                    g.activation_thickness_px,
                    g.bottom_offset_px,
                    g.shown,
                    g.interaction_hold || g.preview_tip_keep,
                )
            };
            if hold {
                return true;
            }

            unsafe {
                let mut pt = POINT::default();
                if GetCursorPos(&mut pt).is_err() {
                    return false;
                }

                let Some((mi, scale)) = dock_monitor_info(app) else {
                    return false;
                };
                if !point_in_monitor(&mi, pt.x, pt.y) {
                    return false;
                }

                if shown {
                    // Rest: chrome strip only (empty headroom must not block hide).
                    // Hover-expanded: chrome + fan headroom — magnified icon hits
                    // extend there; excluding them hid the dock while fan stayed on.
                    if pointer_in_dock_chrome(app, &mi, scale, bottom_off, pt.x, pt.y) {
                        return true;
                    }
                    // Preview tip sits above the icons HWND — treat as keep zone.
                    pointer_in_chrome_hover_tip(app, pt.x, pt.y)
                } else {
                    // Hidden: razor strip on the monitor bottom edge only.
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
        {
            let _ = app;
            false
        }
    }
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
    // rcMonitor.bottom is exclusive — last visible row is bottom - 1.
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
/// Never trust the glass HWND width — live resize bugs made it span almost the
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

    let on_desktop = is_desktop_foreground();
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
