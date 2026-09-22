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
pub(crate) fn sync_inner(app: &AppHandle, enabled: bool, prefs: &MaterialPrefs) {
    let Some(main) = app.get_webview_window("main") else {
        return;
    };

    hide_legacy_glass(app);

    if !enabled {
        crate::win32::bar_comp::detach();
        let _ = crate::win32::material::clear(&main);
        return;
    }

    let _ = crate::win32::material::clear(&main);
    let dark = theme_dark_from_prefs(prefs);
    if let Some(hwnd) = main_hwnd(&main) {
        if let Err(e) = crate::win32::bar_comp::attach_or_update(hwnd, dark) {
            eprintln!("[bar-comp] attach failed: {e}");
        }
    }
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
    let app_for_thread = app.clone();
    schedule_on_main(app_for_thread, move || {
        sync_inner(&app, enabled, &prefs);
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
    let Some(main) = app.get_webview_window("main") else {
        return;
    };
    let me = main.hwnd().ok().map(|h| h.0 as isize);
    if !crate::win32::ambient::is_desktop_scene(me) {
        return;
    }
    let prefs = crate::commands::load_material_prefs();
    let dark = theme_dark_from_prefs(&prefs);
    let Some(hwnd) = main_hwnd(&main) else {
        return;
    };
    let hwnd_raw = hwnd.0 as isize;
    crate::win32::click_trace::log("bar-comp", "refresh_layout_now → main thread");
    let _ = main.run_on_main_thread(move || {
        let h = HWND(hwnd_raw as *mut _);
        let _ = crate::win32::bar_comp::refresh_layout(h, dark);
        crate::win32::click_trace::log("bar-comp", "refresh_layout_now done");
    });
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
