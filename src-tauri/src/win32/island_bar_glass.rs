//! Desktop top-bar frost on `main` via Composition under WebView2 (`bar_comp`).
//!
//! The legacy sibling `island-bar-glass` HWND is retired — it fought DWM Z-order
//! when framed windows closed/minimized (material-only flash).

#![cfg(windows)]

use tauri::{AppHandle, Manager, WebviewWindow};
use windows::Win32::Foundation::HWND;

use super::material::MaterialPrefs;
use std::sync::atomic::{AtomicU64, Ordering};

pub const LABEL: &str = "island-bar-glass";

/// Skip Moved/Resized → bar_comp until this unix-ms (set after island SetWindowPos).
static SUPPRESS_REFRESH_UNTIL_MS: AtomicU64 = AtomicU64::new(0);
static REFRESH_GEN: AtomicU64 = AtomicU64::new(0);

fn main_hwnd(window: &WebviewWindow) -> Option<HWND> {
    window
        .hwnd()
        .ok()
        .map(|h| HWND(h.0 as *mut core::ffi::c_void))
}

fn hide_legacy_glass(app: &AppHandle) {
    if let Some(glass) = app.get_webview_window(LABEL) {
        let _ = glass.hide();
    }
}

fn theme_dark_from_prefs(prefs: &MaterialPrefs) -> Option<bool> {
    prefs.dark.or_else(|| Some(crate::win32::material::system_apps_dark()))
}

/// Must run on the UI / main thread only.
///
/// `bar_glass`: island pref. `quiet`: main AppBar settle — do **not** wipe
/// chrome-sat SWCA during quiet (that left secondary bars empty until a manual refresh).
pub(crate) fn sync_inner(app: &AppHandle, bar_glass: bool, quiet: bool, prefs: &MaterialPrefs) {
    hide_legacy_glass(app);

    if !bar_glass {
        crate::win32::bar_comp::detach();
        if let Some(main) = app.get_webview_window("main") {
            let _ = crate::win32::material::clear(&main);
            if let Ok(hwnd) = main.hwnd() {
                crate::win32::blur_glass::clear_hwnd_composition(hwnd.0 as isize);
            }
            crate::win32::blur_glass::clear_webview_fill(&main);
        }
        for (label, win) in app.webview_windows() {
            if label.starts_with("chrome-sat-") {
                let _ = crate::win32::material::clear(&win);
                if let Ok(hwnd) = win.hwnd() {
                    crate::win32::blur_glass::clear_hwnd_composition(hwnd.0 as isize);
                }
                crate::win32::blur_glass::clear_webview_fill(&win);
            }
        }
        return;
    }

    let dark = theme_dark_from_prefs(prefs);

    if let Some(main) = app.get_webview_window("main") {
        let me = main.hwnd().ok().map(|h| h.0 as isize);
        if quiet {
            // AppBar settle: leave main composition alone (attach races DWM).
        } else {
            let desktop = crate::win32::ambient::is_desktop_scene(me);
            // Glass first: keep strip-clipped HostBackdrop until live AmbientStrip has
            // covered it for ~2s (`ambient_owns_main_chrome`), then detach.
            let want_glass = desktop || !crate::win32::ambient::ambient_owns_main_chrome();
            let _ = crate::win32::material::clear(&main);
            if let Some(raw) = me {
                crate::win32::blur_glass::clear_hwnd_composition(raw);
            }
            crate::win32::blur_glass::clear_webview_fill(&main);
            if want_glass {
                if let Some(hwnd) = main_hwnd(&main) {
                    if let Err(e) = crate::win32::bar_comp::attach_or_update(hwnd, dark) {
                        eprintln!("[bar-comp] attach failed: {e}; keep transparent strip");
                        crate::win32::bar_comp::detach();
                    }
                }
                crate::win32::blur_glass::clear_webview_fill(&main);
            } else if let Some(raw) = me {
                crate::win32::bar_comp::detach_hwnd(raw);
                crate::win32::blur_glass::clear_webview_fill(&main);
            }
        }
    }

    // Secondary chrome: always sync per-monitor scene (even during main quiet).
    for (label, win) in app.webview_windows() {
        if !label.starts_with("chrome-sat-") {
            continue;
        }
        sync_one_sat(&win, &label, dark);
    }
}

/// Apply or clear frost for one chrome-sat HWND — **same as main**:
/// desktop → `bar_comp` HostBackdrop; maximized → detach + clear (AmbientStrip).
/// SWCA only if Composition attach fails on this WebView2.
///
/// Call from the UI thread (or via `run_on_main_thread`). Do **not** call
/// `sync_attach_or_update` here — that deadlocks when already on the UI thread.
pub fn sync_one_sat(win: &WebviewWindow, label: &str, dark: Option<bool>) {
    use tauri::utils::config::Color;
    let Ok(hwnd) = win.hwnd() else {
        return;
    };
    let raw = hwnd.0 as isize;
    let desktop = crate::win32::ambient::is_desktop_scene(Some(raw));
    if !desktop {
        // Floor = last sampled 吸色; never seed charcoal if cache empty.
        // Do NOT apply full-HWND SWCA — chrome-sat can grow and acrylic becomes a slab.
        let (r, g, b) = crate::win32::ambient::last_sat_strip(label)
            .filter(|s| !(s.r == 48 && s.g == 48 && s.b == 52))
            .map(|s| (s.r, s.g, s.b))
            .unwrap_or((72, 72, 76));
        let floor = Color(r, g, b, 255);
        let _ = win.set_background_color(Some(floor));
        crate::win32::bar_comp::detach_hwnd(raw);
        let _ = crate::win32::material::clear(win);
        crate::win32::blur_glass::clear_hwnd_composition(raw);
        let _ = win.set_background_color(Some(floor));
        return;
    }
    // Desktop: clear SWCA leftovers, HostBackdrop under transparent WebView2.
    let _ = crate::win32::material::clear(win);
    crate::win32::blur_glass::clear_webview_fill(win);
    let h = HWND(raw as *mut _);
    match crate::win32::bar_comp::attach_or_update(h, dark) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("[chrome-sat] bar_comp {label}: {e}; SWCA fallback");
            crate::win32::bar_comp::detach_hwnd(raw);
            if let Err(e2) = crate::win32::blur_glass::apply_chrome_bar_swca(raw, dark) {
                eprintln!("[chrome-sat] glass fallback {label}: {e2}");
            }
        }
    }
    crate::win32::blur_glass::clear_webview_fill(win);
}

/// Immediate glass sync for a chrome-sat. Marshals to that window's UI thread
/// when needed (ambient watcher / IPC workers).
pub fn sync_sat_window_now(win: &WebviewWindow, prefs: &MaterialPrefs) {
    let dark = theme_dark_from_prefs(prefs);
    let bar_glass = crate::commands::get_island_prefs().bar_glass;
    let label = win.label().to_string();
    let win2 = win.clone();
    let _ = win.run_on_main_thread(move || {
        if !bar_glass {
            if let Ok(hwnd) = win2.hwnd() {
                crate::win32::bar_comp::detach_hwnd(hwnd.0 as isize);
            }
            let _ = crate::win32::material::clear(&win2);
            crate::win32::blur_glass::clear_webview_fill(&win2);
            return;
        }
        sync_one_sat(&win2, &label, dark);
    });
}

fn schedule_on_main(app: AppHandle, f: impl FnOnce() + Send + 'static) {
    let Some(main) = app.get_webview_window("main") else {
        return;
    };
    let _ = main.run_on_main_thread(f);
}

pub fn sync(app: &AppHandle, enabled: bool, prefs: &MaterialPrefs) {
    let app = app.clone();
    let prefs = prefs.clone();
    let quiet = crate::win32::work_area::work_area_quiet();
    let app_for_thread = app.clone();
    schedule_on_main(app_for_thread, move || {
        sync_inner(&app, enabled, quiet, &prefs);
    });
}

/// Relayout top strip after monitor pin / width change.
/// Debounced + main-thread only — calling Composition from a worker after
/// SetWindowPos freezes WebView2 (收起 HUNG after resize leave ok).
pub fn refresh_layout(app: &AppHandle) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    if now < SUPPRESS_REFRESH_UNTIL_MS.load(Ordering::Relaxed) {
        crate::win32::click_trace::log("bar-comp", "refresh_layout suppressed");
        return;
    }

    let gen = REFRESH_GEN.fetch_add(1, Ordering::Relaxed) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        if REFRESH_GEN.load(Ordering::Relaxed) != gen {
            return;
        }
        let now2 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        if now2 < SUPPRESS_REFRESH_UNTIL_MS.load(Ordering::Relaxed) {
            crate::win32::click_trace::log("bar-comp", "refresh_layout suppressed (late)");
            return;
        }
        refresh_layout_now(&app);
    });
}

/// Skip Moved/Resized → bar_comp for `ms` after island HWND resize.
pub fn suppress_refresh_ms(ms: u64) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let until = now.saturating_add(ms);
    SUPPRESS_REFRESH_UNTIL_MS.fetch_max(until, Ordering::Relaxed);
}

pub fn refresh_suppressed() -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    now < SUPPRESS_REFRESH_UNTIL_MS.load(Ordering::Relaxed)
}

fn refresh_layout_now(app: &AppHandle) {
    if crate::win32::work_area::work_area_quiet() {
        return;
    }
    if !crate::commands::get_island_prefs().bar_glass {
        return;
    }
    let prefs = crate::commands::load_material_prefs();
    let dark = theme_dark_from_prefs(&prefs);

    let Some(main) = app.get_webview_window("main") else {
        return;
    };
    let me = main.hwnd().ok().map(|h| h.0 as isize);
    if crate::win32::ambient::is_desktop_scene(me) {
        if let Some(hwnd) = main_hwnd(&main) {
            let hwnd_raw = hwnd.0 as isize;
            crate::win32::click_trace::log("bar-comp", "refresh_layout_now → main thread");
            let _ = main.run_on_main_thread(move || {
                let h = HWND(hwnd_raw as *mut _);
                let _ = crate::win32::bar_comp::refresh_layout(h, dark);
                crate::win32::click_trace::log("bar-comp", "refresh_layout_now done");
            });
        }
    }

    // Relayout secondary strips — same bar_comp refresh as main.
    for (label, win) in app.webview_windows() {
        if !label.starts_with("chrome-sat-") {
            continue;
        }
        let Ok(hwnd) = win.hwnd() else {
            continue;
        };
        let raw = hwnd.0 as isize;
        if !crate::win32::ambient::is_desktop_scene(Some(raw)) {
            // Maximized: keep opaque 吸色 floor — never alpha-0 (shows as black).
            use tauri::utils::config::Color;
            let (r, g, b) = crate::win32::ambient::last_sat_strip(&label)
                .filter(|s| !(s.r == 48 && s.g == 48 && s.b == 52))
                .map(|s| (s.r, s.g, s.b))
                .unwrap_or((72, 72, 76));
            let floor = Color(r, g, b, 255);
            let _ = win.set_background_color(Some(floor));
            crate::win32::bar_comp::detach_hwnd(raw);
            let _ = crate::win32::material::clear(&win);
            let _ = win.set_background_color(Some(floor));
            continue;
        }
        let dark = dark;
        let _ = win.run_on_main_thread(move || {
            let h = HWND(raw as *mut _);
            if crate::win32::bar_comp::is_attached_to(raw) {
                let _ = crate::win32::bar_comp::refresh_layout(h, dark);
            } else if let Err(e) = crate::win32::bar_comp::attach_or_update(h, dark) {
                eprintln!("[chrome-sat] bar_comp refresh: {e}");
                let _ = crate::win32::blur_glass::apply_chrome_bar_swca(raw, dark);
            }
        });
        crate::win32::blur_glass::clear_webview_fill(&win);
    }
}

/// No-op — Composition attaches on first `sync_inner`. Kept for boot call sites.
pub fn ensure_at_boot(_app: &AppHandle, _prefs: &MaterialPrefs) {
    eprintln!("[bar-comp] ready (composition on main, no sibling glass)");
}

pub fn hide(app: &AppHandle) {
    crate::win32::bar_comp::detach();
    hide_legacy_glass(app);
}

/// Retired — single-HWND composition has no Z-order restack.
pub fn reassert_stack(_app: &AppHandle) {}

/// Retired — no sibling glass / WinEvent needed.
pub fn spawn_foreground_restack_watcher(_app: tauri::AppHandle) {}
