//! Desktop top-bar frost on `main` via Composition under WebView2 (`bar_comp`).
//!
//! The legacy sibling `island-bar-glass` HWND is retired — it fought DWM Z-order
//! when framed windows closed/minimized (material-only flash).

#![cfg(windows)]

use tauri::{AppHandle, Manager, WebviewWindow};
use windows::Win32::Foundation::HWND;

use super::material::MaterialPrefs;

pub const LABEL: &str = "island-bar-glass";

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
pub fn refresh_layout(app: &AppHandle) {
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
    if let Some(hwnd) = main_hwnd(&main) {
        let _ = crate::win32::bar_comp::refresh_layout(hwnd, dark);
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
