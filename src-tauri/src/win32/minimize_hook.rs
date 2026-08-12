//! Intercept OS title-bar minimize → genie suck-to-dock (for Dock-tracked apps).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

static HOOK: Mutex<Option<isize>> = Mutex::new(None);
static APP: Mutex<Option<AppHandle>> = Mutex::new(None);
static ARMED: AtomicBool = AtomicBool::new(false);
static LAST: Mutex<Option<(isize, Instant)>> = Mutex::new(None);

#[cfg(windows)]
pub fn spawn_minimize_interceptor(app: AppHandle) {
    if let Ok(mut g) = APP.lock() {
        *g = Some(app);
    }
    std::thread::spawn(|| {
        // WinEvent hooks need a message pump on the installing thread.
        install_and_pump();
    });
}

#[cfg(not(windows))]
pub fn spawn_minimize_interceptor(_app: AppHandle) {}

#[cfg(windows)]
fn install_and_pump() {
    use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent};
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, TranslateMessage, EVENT_SYSTEM_MINIMIZESTART, MSG,
        WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
    };

    unsafe {
        let hook = SetWinEventHook(
            EVENT_SYSTEM_MINIMIZESTART,
            EVENT_SYSTEM_MINIMIZESTART,
            None,
            Some(minimize_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        if hook.0.is_null() {
            eprintln!("[genie] SetWinEventHook(MINIMIZESTART) failed");
            return;
        }
        if let Ok(mut g) = HOOK.lock() {
            *g = Some(hook.0 as isize);
        }
        ARMED.store(true, Ordering::SeqCst);
        eprintln!("[genie] OS minimize interceptor armed");

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let _ = UnhookWinEvent(hook);
        ARMED.store(false, Ordering::SeqCst);
    }
}

#[cfg(windows)]
unsafe extern "system" fn minimize_event_proc(
    _hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    event: u32,
    hwnd: windows::Win32::Foundation::HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    use windows::Win32::UI::WindowsAndMessaging::{
        ShowWindow, EVENT_SYSTEM_MINIMIZESTART, SW_RESTORE,
    };

    if event != EVENT_SYSTEM_MINIMIZESTART {
        return;
    }
    if !ARMED.load(Ordering::SeqCst) {
        return;
    }
    if hwnd.0.is_null() {
        return;
    }
    // Top-level window object only (OBJID_WINDOW=0, CHILDID_SELF=0).
    if id_object != 0 || id_child != 0 {
        return;
    }

    let hwnd_raw = hwnd.0 as isize;

    // Debounce duplicate events for the same HWND.
    {
        let now = Instant::now();
        if let Ok(mut g) = LAST.lock() {
            if let Some((h, t)) = *g {
                if h == hwnd_raw && now.duration_since(t) < Duration::from_millis(900) {
                    return;
                }
            }
            *g = Some((hwnd_raw, now));
        }
    }

    let Some(item_id) = crate::dock::genie::dock_item_id_for_hwnd(hwnd_raw) else {
        return;
    };

    let app = match APP.lock() {
        Ok(g) => g.clone(),
        Err(_) => None,
    };
    let Some(app) = app else {
        return;
    };

    if let Some(state) = app.try_state::<crate::dock::genie::GenieState>() {
        if state.is_busy_or_parked(&item_id) {
            return;
        }
    }

    let icon = crate::dock::genie::icon_rect_for_item(&item_id).unwrap_or(
        crate::dock::genie::GenieRect {
            x: 0.0,
            y: 0.0,
            w: 40.0,
            h: 40.0,
        },
    );

    // Prefer a frame captured WHILE the window was still foreground — never
    // screenshot after OS minimize has already cleared the pixels.
    let prepared = crate::dock::genie::take_prepared_minimize(hwnd_raw, &item_id);

    // Kill DWM minimize/restore transition so we don't get "顿一下" OS anim,
    // then (if we have a cached freeze) snap the window back to full size without bounce.
    crate::dock::genie::disable_hwnd_transitions(hwnd_raw);
    if prepared.is_some() {
        let _ = ShowWindow(hwnd, SW_RESTORE);
        eprintln!("[genie] intercept OS minimize (cached frame) → {item_id}");
    } else {
        let _ = ShowWindow(hwnd, SW_RESTORE);
        eprintln!("[genie] intercept OS minimize (live capture fallback) → {item_id}");
    }

    let _ = app.emit(
        "genie-os-minimize",
        serde_json::json!({ "itemId": item_id, "hwnd": hwnd_raw }),
    );

    tauri::async_runtime::spawn(async move {
        let result = if let Some(prepared) = prepared {
            crate::dock::genie::minimize_app_from_prepared(app, item_id, icon, prepared).await
        } else {
            crate::dock::genie::minimize_app_inner(app, item_id, icon).await
        };
        if let Err(e) = result {
            eprintln!("[genie] OS minimize genie failed: {e}");
        }
    });
}
