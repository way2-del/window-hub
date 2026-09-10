mod commands;
mod companion_scripts;
mod db;
mod dock;
mod ecs;
#[cfg(windows)]
mod everything;
mod hub_fetch_guard;
mod plugin_hub;
mod plugin_install;
mod staging;
mod sysmon;
mod win32;
mod windows_service;

use std::time::{Duration, Instant};

use tauri::{Emitter, Manager};

use crate::commands::initial_material_state;
use crate::ecs::{spawn_ecs_thread, EcsHandle};
use crate::plugin_hub::ShortcutsPinStore;
use crate::win32::appbar;
use crate::win32::work_area;
use crate::windows_service::WindowsService;

/// Windows Service entry for `--autostart-svc` (no Tauri UI).
#[cfg(windows)]
pub fn run_autostart_service() {
    crate::win32::autostart_svc::run_autostart_service();
}

/// One-shot elevated helper: install SCM autostart then exit.
#[cfg(windows)]
pub fn install_autostart_service_once() -> Result<(), String> {
    crate::win32::app_launch::install_service_elevated_helper()
}

/// One-shot elevated helper: uninstall SCM autostart then exit.
#[cfg(windows)]
pub fn uninstall_autostart_service_once() -> Result<(), String> {
    crate::win32::app_launch::uninstall_service_elevated_helper()
}

/// Block duplicate GUI launches (MessageBox + exit). No-op for the service entry.
#[cfg(windows)]
pub fn ensure_single_instance() {
    crate::win32::single_instance::ensure_single_instance_or_exit();
}

/// 因全屏游戏隐藏顶栏时为 true；watchdog 期间勿重挂 AppBar / 几何。
fn island_hidden_for_fullscreen() -> bool {
    work_area::island_hidden_for_fullscreen()
}

fn hwnd_of(window: &tauri::WebviewWindow) -> Option<isize> {
    window.hwnd().ok().map(|h| h.0 as isize)
}

/// Pin island HWND to monitor top / full width — pure Win32 (safe from any thread).
/// Never call `WebviewWindow::set_size` / `set_position` from background or sync IPC.
#[cfg(windows)]
fn pin_top_bar_hwnd(hwnd_raw: isize) {
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
        let mon = info.rcMonitor;
        let w = (mon.right - mon.left).max(1);
        if wr.left == mon.left && wr.top == mon.top && (wr.right - wr.left) == w {
            return;
        }
        let _ = SetWindowPos(
            hwnd,
            None,
            mon.left,
            mon.top,
            w,
            h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

#[cfg(not(windows))]
fn pin_top_bar_hwnd(_hwnd_raw: isize) {}

fn pin_top_bar(window: &tauri::WebviewWindow) {
    if let Some(hwnd) = hwnd_of(window) {
        pin_top_bar_hwnd(hwnd);
    }
}

/// Reassert island visibility / Z-order with Win32 only.
/// Calling `window.show()` / `set_always_on_top` from a worker or sync command
/// deadlocks WebView2 on Windows (click → 未响应).
fn reassert_window(window: &tauri::WebviewWindow) {
    if island_hidden_for_fullscreen() {
        return;
    }
    let Some(hwnd) = hwnd_of(window) else {
        return;
    };
    crate::win32::switcher::exclude_from_switcher(hwnd);
    crate::win32::topmost::set_main_hwnd(hwnd);
    let _ = crate::win32::topmost::ensure_main_visible();
    crate::win32::topmost::reassert_main_zorder();
    pin_top_bar_hwnd(hwnd);
    // Never apply bar-glass / DWM here — Moved/Resized fires on every click/drag
    // and `island_bar_glass::sync` can spawn a WebView (deadlock → 未响应).
}

#[cfg(windows)]
fn show_hwnd(hwnd_raw: isize, show: bool) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE, SW_SHOWNOACTIVATE};
    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        let _ = ShowWindow(hwnd, if show { SW_SHOWNOACTIVATE } else { SW_HIDE });
    }
}

fn boot_log(step: &str, detail: &str) {
    eprintln!("[boot] {step}: {detail}");
}

#[cfg(windows)]
fn restack_island_bar_after_settings(app: &tauri::AppHandle) {
    crate::win32::island_bar_glass::refresh_layout(app);
}

#[cfg(not(windows))]
fn restack_island_bar_after_settings(_app: &tauri::AppHandle) {}

fn spawn_watchdog(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("watchdog".into())
        .spawn(move || {
            boot_log("watchdog", "thread running");
            // Let the main HWND exist, then claim strips once under quiet.
            std::thread::sleep(Duration::from_millis(120));
            if let Some(window) = app.get_webview_window("main") {
                // Dock on: hide taskbar *before* top AppBar so shell only settles once
                // toward the final (top+bottom reserved) work area, not expand-then-shrink.
                #[cfg(windows)]
                {
                    let prefs = crate::dock::load_dock_prefs();
                    if prefs.enabled && crate::dock::mode_reserves_bottom_work_area(prefs.mode()) {
                        let _ = crate::commands::set_system_taskbar_visible(false);
                        std::thread::sleep(Duration::from_millis(60));
                    }
                }
                if let Some(hwnd) = hwnd_of(&window) {
                    appbar::register(hwnd);
                    boot_log("watchdog", "appbar registered");
                }
                crate::commands::apply_main_window_material(&app);
                reassert_window(&window);
            }

            let mut ticks: u32 = 0;
            loop {
                // Sparse reassert — every 500ms (was 2s; bar-glass watcher handles Z-order).
                std::thread::sleep(Duration::from_millis(500));
                if island_hidden_for_fullscreen() {
                    continue;
                }
                let Some(window) = app.get_webview_window("main") else {
                    break;
                };
                reassert_window(&window);
                ticks = ticks.wrapping_add(1);
                if ticks % 5 == 0 && !work_area::work_area_quiet() {
                    if let Some(hwnd) = hwnd_of(&window) {
                        appbar::sync(hwnd);
                    }
                }
            }
        })
        .expect("spawn watchdog");
}

/// 前台为独占/无边框全屏（游戏）时只藏岛/设置窗 UI；**不** ABM_REMOVE / 重挂 AppBar。
///
/// 顶栏 + Dock 各一次 SETPOS 会让最大化窗 resize 两次；quiet 只能砍掉 ABN 互踢，
/// 无法把两次合成一次。全屏期间保持工作区不变 → 退出时 0 次 work-area 抖动，判定一次成功。
fn spawn_fullscreen_watcher(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("fullscreen".into())
        .spawn(move || {
            boot_log("fullscreen", "thread running");
            let mut hidden = false;
            let mut hide_streak = 0u32;
            let mut show_streak = 0u32;
            let mut restore_grace_until: Option<Instant> = None;
            // 工作区不再随全屏抖动，2×350ms 即可；grace 防短闪误藏。
            const NEED: u32 = 2;

            loop {
                std::thread::sleep(Duration::from_millis(350));
                let Some(window) = app.get_webview_window("main") else {
                    break;
                };
                let self_hwnd = hwnd_of(&window);
                let mut should_hide = crate::win32::fullscreen::should_hide_strip(self_hwnd);

                if restore_grace_until
                    .map(|until| Instant::now() < until)
                    .unwrap_or(false)
                {
                    should_hide = false;
                    hide_streak = 0;
                } else {
                    restore_grace_until = None;
                }

                if should_hide {
                    show_streak = 0;
                    hide_streak = hide_streak.saturating_add(1);
                } else {
                    hide_streak = 0;
                    show_streak = show_streak.saturating_add(1);
                }

                if should_hide && hide_streak >= NEED && !hidden {
                    // Freeze dock AppBar sync so Default 模式藏条时不会 ABM_REMOVE 底边。
                    work_area::set_island_hidden_for_fullscreen(true);
                    if let Some(hwnd) = hwnd_of(&window) {
                        #[cfg(windows)]
                        show_hwnd(hwnd, false);
                        #[cfg(not(windows))]
                        {
                            let _ = window.hide();
                        }
                    }
                    #[cfg(windows)]
                    crate::win32::island_bar_glass::hide(&app);
                    if let Some(settings) = app.get_webview_window("settings") {
                        if let Some(hwnd) = hwnd_of(&settings) {
                            #[cfg(windows)]
                            show_hwnd(hwnd, false);
                            #[cfg(not(windows))]
                            {
                                let _ = settings.hide();
                            }
                        }
                    }
                    hidden = true;
                } else if !should_hide && show_streak >= NEED && hidden {
                    if let Some(hwnd) = hwnd_of(&window) {
                        #[cfg(windows)]
                        show_hwnd(hwnd, true);
                        #[cfg(not(windows))]
                        {
                            let _ = window.show();
                        }
                    }
                    work_area::set_island_hidden_for_fullscreen(false);
                    reassert_window(&window);
                    crate::commands::apply_main_window_material(&app);
                    restore_grace_until = Some(Instant::now() + Duration::from_secs(2));
                    hidden = false;
                }
            }
        })
        .expect("spawn fullscreen");
}

/// 最大化窗口顶取色；切窗后动态采 ~0.9s 再锁定。启动稍晚，避免 AppBar 未就绪时自吸发黑。
fn spawn_ambient_watcher(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("ambient".into())
        .spawn(move || {
            boot_log("ambient", "thread running");
            let mut cleared = false;
            let mut last_desktop: Option<bool> = None;
            let mut material_after_quiet = false;

            if let Some(window) = app.get_webview_window("main") {
                // Setup already applied bar material on the UI thread — do not re-queue here
                // while dock WebViews are still settling (was blocking main + freezing dock IPC).
                cleared = true;
                last_desktop = Some(crate::win32::ambient::is_desktop_scene(hwnd_of(&window)));
                // Wallpaper seed only — defer live BitBlt until work-area quiet ends.
                let seed = crate::win32::ambient::sample_nonblocking(hwnd_of(&window));
                let _ = app.emit("ambient-color", seed);
                boot_log("ambient", "seed emitted (live capture deferred)");
            }

            loop {
                let ms = if crate::win32::ambient::is_settling() {
                    450
                } else {
                    1200
                };
                std::thread::sleep(Duration::from_millis(ms));
                let Some(window) = app.get_webview_window("main") else {
                    break;
                };

                if !cleared {
                    crate::commands::apply_main_window_material(&app);
                    cleared = true;
                }

                if !material_after_quiet && !crate::win32::work_area::work_area_quiet() {
                    material_after_quiet = true;
                    crate::commands::apply_main_window_material(&app);
                }

                // BitBlt + bar_comp during AppBar settle → DWM / main-thread 未响应.
                if crate::win32::work_area::work_area_quiet() {
                    continue;
                }

                let desktop_now = crate::win32::ambient::is_desktop_scene(hwnd_of(&window));
                if last_desktop != Some(desktop_now) {
                    last_desktop = Some(desktop_now);
                    // Desktop ↔ window: glass only when desktop && barGlass.
                    crate::commands::apply_main_window_material(&app);
                }

                if let Some(strip) = crate::win32::ambient::poll_changed(hwnd_of(&window)) {
                    let _ = app.emit("ambient-color", strip);
                }
            }
        })
        .expect("spawn ambient");
}

/// Push foreground keyboard layout / IME state to the menubar chips.
fn spawn_input_lang_watcher(app: tauri::AppHandle) {
    // WLAN/IME first poll can block; never run on the Tauri setup (UI) thread.
    std::thread::Builder::new()
        .name("input-lang".into())
        .spawn(move || {
            boot_log("input-lang", "thread running");
            crate::win32::input_lang::start(move |state| {
                let _ = app.emit("input-lang", &state);
            });
            boot_log("input-lang", "listener online");
        })
        .expect("spawn input-lang");
}

/// Push WLAN radio / association state to the menubar chip.
fn spawn_wifi_watcher(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("wifi".into())
        .spawn(move || {
            boot_log("wifi", "thread running");
            crate::win32::wifi::start(move |state| {
                let _ = app.emit("wifi-state", &state);
            });
            boot_log("wifi", "listener online");
        })
        .expect("spawn wifi");
}

/// One coordinator thread: start background services **sequentially** so AppBar /
/// tray hook / dock / ambient do not stampede the UI thread on cold start.
fn spawn_boot_pipeline(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("boot-pipeline".into())
        .spawn(move || {
            let t0 = Instant::now();
            let elapsed = || t0.elapsed().as_millis();
            boot_log("pipeline", "begin (sequential)");

            // Let WebView2 first paint finish before shell / hook work.
            std::thread::sleep(Duration::from_millis(400));
            boot_log("pipeline", &format!("first-paint wait (+{}ms)", elapsed()));

            // 1) AppBar + topmost — must settle before other Win32 work.
            boot_log("watchdog", "starting");
            spawn_watchdog(app.clone());
            std::thread::sleep(Duration::from_millis(500));
            boot_log("watchdog", &format!("handed off (+{}ms)", elapsed()));

            // 2) Window enum poller (Dock dots) — defer from setup.
            boot_log("windows-service", "starting");
            if let Some(svc) = app.try_state::<WindowsService>() {
                svc.start_poller(app.clone());
                boot_log("windows-service", &format!("ok (+{}ms)", elapsed()));
            } else {
                boot_log("windows-service", "SKIP (state missing)");
            }
            std::thread::sleep(Duration::from_millis(250));

            // 3) Fullscreen hide strip (light loop).
            boot_log("fullscreen", "starting");
            spawn_fullscreen_watcher(app.clone());
            boot_log("fullscreen", &format!("ok (+{}ms)", elapsed()));
            std::thread::sleep(Duration::from_millis(300));

            // 4) Menubar chips (IME / Wi‑Fi) — first poll can block.
            boot_log("input-lang", "starting");
            spawn_input_lang_watcher(app.clone());
            std::thread::sleep(Duration::from_millis(350));
            boot_log("input-lang", &format!("ok (+{}ms)", elapsed()));

            boot_log("wifi", "starting");
            spawn_wifi_watcher(app.clone());
            std::thread::sleep(Duration::from_millis(350));
            boot_log("wifi", &format!("ok (+{}ms)", elapsed()));

            // 5) Official plugins copy — disk heavy.
            boot_log("plugins", "ensure/resync starting");
            {
                let step = Instant::now();
                crate::plugin_install::ensure_official_plugins(&app);
                crate::plugin_install::resync_dev_plugins(&app);
                boot_log(
                    "plugins",
                    &format!(
                        "ok step={}ms total={}ms",
                        step.elapsed().as_millis(),
                        elapsed()
                    ),
                );
            }
            std::thread::sleep(Duration::from_millis(350));

            // 6) Companion launchers.
            boot_log("companion", "starting");
            crate::companion_scripts::start_hub_associated_launchers();
            boot_log("companion", &format!("ok (+{}ms)", elapsed()));
            std::thread::sleep(Duration::from_millis(350));

            // 7) Tray hook + registry seed — before dock WebViews (EnumWindows vs WebView2).
            boot_log("tray", "starting");
            {
                let app_icons = app.clone();
                let app_attn = app.clone();
                let app_prefs = app.clone();
                crate::win32::tray::start(
                    move |icons| {
                        let _ = app_icons.emit("tray-icons", &icons);
                    },
                    move |attn| {
                        let _ = app_attn.emit("tray-attention", &attn);
                    },
                    move |prefs| {
                        let _ = app_prefs.emit("tray-prefs", &prefs);
                    },
                );
            }
            boot_log("tray", "waiting for hook icons");
            let seeded = crate::win32::tray::wait_for_tray_seed(Duration::from_secs(3));
            boot_log(
                "tray",
                &format!(
                    "seed done={seeded} total={} clickable={} (+{}ms)",
                    crate::win32::tray::total_icon_count(),
                    crate::win32::tray::clickable_count(),
                    elapsed()
                ),
            );

            if crate::win32::tray::clickable_count() == 0 && seeded {
                boot_log(
                    "tray",
                    &format!(
                        "WARN no clickable yet total={} — UI may show empty chevron (+{}ms)",
                        crate::win32::tray::total_icon_count(),
                        elapsed()
                    ),
                );
            }

            // Do NOT create tray-popup / extra WebViews here — blocks UI pump
            // for 10s+ and leaves the island bar transparent (glass never applied).
            boot_log("tray", &format!("handed off (+{}ms)", elapsed()));
            std::thread::sleep(Duration::from_millis(200));

            // 8) Dock dual-WebView — block until icons HWND exists (ambient/hotkeys after).
            boot_log("dock", "bootstrap starting");
            if let Some(rx) = crate::dock::bootstrap_dock(&app) {
                match rx.recv_timeout(Duration::from_secs(20)) {
                    Ok(Ok(())) => {
                        boot_log("dock", &format!("ready (+{}ms)", elapsed()));
                    }
                    Ok(Err(e)) => {
                        boot_log("dock", &format!("FAIL {e} (+{}ms)", elapsed()));
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        boot_log(
                            "dock",
                            &format!("TIMEOUT — continuing (+{}ms)", elapsed()),
                        );
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        boot_log("dock", &format!("DISCONNECTED (+{}ms)", elapsed()));
                    }
                }
            } else {
                boot_log("dock", &format!("disabled (+{}ms)", elapsed()));
            }
            std::thread::sleep(Duration::from_millis(300));

            // 9) Global hotkeys.
            #[cfg(windows)]
            {
                boot_log("hotkeys", "starting");
                crate::win32::hotkey_registry::spawn(app.clone());
                boot_log("hotkeys", &format!("ok (+{}ms)", elapsed()));
                std::thread::sleep(Duration::from_millis(200));
            }

            // 10) Ambient sampling — BitBlt last so DWM is calm.
            boot_log("ambient", "starting");
            spawn_ambient_watcher(app.clone());
            boot_log("ambient", &format!("ok (+{}ms)", elapsed()));
            std::thread::sleep(Duration::from_millis(200));

            // 11) Dock hover previews — PrintWindow; keep last.
            boot_log("dock-preview", "starting");
            crate::dock::spawn_dock_preview_refresher(app.clone());
            boot_log("dock-preview", &format!("ok (+{}ms)", elapsed()));

            boot_log(
                "pipeline",
                &format!(
                    "READY total={}ms tray_clickable={} — UI should stay responsive",
                    elapsed(),
                    crate::win32::tray::clickable_count()
                ),
            );
        })
        .expect("spawn boot-pipeline");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            boot_log("setup", "enter");
            let db = crate::db::init().map_err(|e| {
                eprintln!("[db] init failed: {e}");
                e
            })?;
            boot_log("setup", "db ok");
            app.manage(db);
            // Drop legacy AppCompat RUNASADMIN so the GUI never stays elevated.
            let _ = crate::win32::app_launch::clear_legacy_admin_flag();
            let handle = spawn_ecs_thread(app.handle().clone());
            app.manage(handle);
            boot_log("setup", "ecs thread ok");
            app.manage(initial_material_state());
            app.manage(WindowsService::new());
            boot_log("setup", "windows service registered (poller deferred)");
            app.manage(crate::dock::DockVisibility::new());
            let pins = ShortcutsPinStore::new();
            pins.load_all_from_db();
            app.manage(pins);
            let _ = crate::plugin_install::list_installed_plugins_sync();
            crate::win32::ambient::set_mode(commands::load_ambient_mode());
            crate::win32::tray::set_prefs(commands::load_tray_prefs());
            #[cfg(windows)]
            crate::win32::ambient::sync_ignore_ambient_apps(
                commands::get_island_prefs().ignore_ambient_apps,
            );

            // Startup: kill AppBar ABN thrash while top + taskbar + dock bottom settle.
            // Maximized windows otherwise resize many times on first launch.
            work_area::mark_work_area_quiet(5_000);

            if let Some(window) = app.get_webview_window("main") {
                if let Some(hwnd) = hwnd_of(&window) {
                    crate::win32::topmost::set_main_hwnd(hwnd);
                }
                reassert_window(&window);
                // Defer bar_comp attach until work-area quiet ends (~5s) — attaching
                // Composition during WebView2 first paint deadlocks the main thread.
                #[cfg(windows)]
                if let Some(main) = app.get_webview_window("main") {
                    let _ = crate::win32::material::clear(&main);
                }
                boot_log("setup", "main hwnd + material (deferred bar-comp)");
            }

            // Everything else: one sequential pipeline (logs: [boot] …).
            spawn_boot_pipeline(app.handle().clone());
            boot_log("setup", "boot pipeline scheduled — leaving setup");

            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { .. } => {
                    // Pre-restack before the framed settings HWND tears down — avoids
                    // ~80ms wrong Z-order + late `clear(main)` flash on the bar.
                    if window.label() == "settings" {
                        restack_island_bar_after_settings(window.app_handle());
                    }
                }
                tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                    if window.label() == "main"
                        && !island_hidden_for_fullscreen()
                    {
                        if let Some(w) = window.app_handle().get_webview_window("main") {
                            pin_top_bar(&w);
                        }
                        #[cfg(windows)]
                        crate::win32::island_bar_glass::refresh_layout(window.app_handle());
                    }
                    // Maximize / restore resets caption — debounce full Mica re-apply.
                    if window.label() == "plugin-window" {
                        crate::commands::schedule_plugin_window_mica_refresh(
                            window.app_handle(),
                        );
                    }
                }
                tauri::WindowEvent::Focused(focused) => {
                    // Dock: re-strip native caption residue on activate (Win11 paints
                    // a light bar into headroom on click / long-press otherwise).
                    if *focused && (window.label() == "dock" || window.label() == "dock-glass") {
                        #[cfg(windows)]
                        if let Ok(hwnd) = window.hwnd() {
                            // Quiet ensure — schedule+FRAMECHANGED on every click flashes
                            // a light caption strip into magnification headroom.
                            crate::win32::blur_glass::ensure_dock_titlebar_stripped_raw(
                                hwnd.0 as isize,
                            );
                        }
                    }
                    if *focused && window.label() == "plugin-window" {
                        crate::commands::schedule_plugin_window_mica_refresh(
                            window.app_handle(),
                        );
                    }
                    // 托盘 / 插件 / 状态菜单弹窗失焦即关（WebView 侧 focus 事件不总是可靠）
                    if (window.label() == "tray-popup"
                        || window.label() == "plugin-popup"
                        || window.label() == "status-menu-popup"
                        || window.label() == "input-lang-popup"
                        || window.label() == "wifi-popup")
                        && !*focused
                    {
                        let label = window.label().to_string();
                        let app = window.app_handle().clone();
                        let popup_hwnd = window.hwnd().ok().map(|h| h.0 as isize);
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(60));
                            #[cfg(windows)]
                            {
                                use windows::Win32::Foundation::HWND;
                                use windows::Win32::UI::WindowsAndMessaging::{
                                    GetForegroundWindow, PostMessageW, WM_CLOSE,
                                };
                                if let Some(raw) = popup_hwnd {
                                    unsafe {
                                        let fg = GetForegroundWindow();
                                        if fg.0 as isize == raw {
                                            return;
                                        }
                                        let _ = PostMessageW(
                                            HWND(raw as *mut _),
                                            WM_CLOSE,
                                            windows::Win32::Foundation::WPARAM(0),
                                            windows::Win32::Foundation::LPARAM(0),
                                        );
                                    }
                                }
                            }
                            #[cfg(not(windows))]
                            if let Some(w) = app.get_webview_window(&label) {
                                if w.is_focused().unwrap_or(false) {
                                    return;
                                }
                                let _ = w.close();
                            }
                            match label.as_str() {
                                "tray-popup" => {
                                    let _ = app.emit("tray-popup-closed", ());
                                }
                                "plugin-popup" => {
                                    let _ = app.emit("plugin-popup-closed", ());
                                }
                                "status-menu-popup" => {
                                    if let Some(vis) =
                                        app.try_state::<std::sync::Arc<crate::dock::DockVisibility>>()
                                    {
                                        vis.set_interaction_hold(false);
                                    }
                                    let _ = app.emit("status-menu-popup-closed", ());
                                }
                                "input-lang-popup" => {
                                    let _ = app.emit("input-lang-popup-closed", ());
                                }
                                "wifi-popup" => {
                                    let _ = app.emit("wifi-popup-closed", ());
                                }
                                _ => {}
                            }
                        });
                    }
                }
                tauri::WindowEvent::Destroyed => {
                    if window.label() == "tray-popup" {
                        let _ = window.app_handle().emit("tray-popup-closed", ());
                    }
                    if window.label() == "plugin-popup" || window.label() == "plugin-window" {
                        #[cfg(windows)]
                        if window.label() == "plugin-window" {
                            crate::win32::ambient::set_ambient_sample_target(None, 0);
                        }
                        let _ = window.app_handle().emit("plugin-popup-closed", ());
                    }
                    if window.label() == "status-menu-popup" {
                        if let Some(vis) = window
                            .app_handle()
                            .try_state::<std::sync::Arc<crate::dock::DockVisibility>>()
                        {
                            vis.set_interaction_hold(false);
                        }
                        let _ = window.app_handle().emit("status-menu-popup-closed", ());
                    }
                    if window.label() == "input-lang-popup" {
                        let _ = window.app_handle().emit("input-lang-popup-closed", ());
                    }
                    if window.label() == "wifi-popup" {
                        let _ = window.app_handle().emit("wifi-popup-closed", ());
                    }
                    if window.label() == "wifi-auth-popup" {
                        let _ = window.app_handle().emit("wifi-auth-popup-closed", ());
                    }
                    if window.label() == "settings" {
                        let app = window.app_handle().clone();
                        let _ = app.emit("settings-closed", ());
                        restack_island_bar_after_settings(&app);
                    }
                    if window.label() == "main" {
                        appbar::restore();
                        if let Some(ecs) = window.app_handle().try_state::<EcsHandle>() {
                            ecs.send(crate::ecs::resources::HubCommand::Shutdown);
                        }
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::health,
            commands::list_open_windows,
            commands::get_open_window,
            commands::focus_open_window,
            commands::attach_window,
            commands::detach_window,
            commands::set_roi,
            commands::swap_slots,
            commands::forward_pointer,
            commands::forward_key,
            commands::self_hwnd,
            commands::main_cursor_client_pos,
            commands::dock_set_visual_height,
            commands::float_overlay,
            commands::activate_main_island,
            commands::resize_main_island,
            commands::settle_overlay,
            commands::open_settings_window,
            commands::close_settings_window,
            commands::open_tray_popup,
            commands::close_tray_popup,
            commands::is_tray_popup_open,
            commands::open_status_menu_popup,
            commands::close_status_menu_popup,
            commands::is_status_menu_popup_open,
            commands::open_plugin_popup,
            commands::schedule_plugin_popup_as_window,
            commands::close_plugin_popup,
            commands::is_plugin_popup_open,
            commands::resize_plugin_popup,
            commands::set_plugin_popup_windowed_fullscreen,
            plugin_hub::hub_windows_list,
            plugin_hub::hub_windows_get,
            plugin_hub::hub_windows_focus,
            plugin_hub::hub_storage_get,
            plugin_hub::hub_storage_set,
            plugin_hub::hub_storage_remove,
            plugin_hub::hub_storage_list_keys,
            plugin_hub::hub_settings_get_all,
            plugin_hub::hub_settings_get,
            plugin_hub::hub_settings_set,
            plugin_hub::hub_shortcuts_set_pins,
            plugin_hub::hub_shortcuts_clear_pins,
            plugin_hub::hub_shortcuts_list_pins,
            plugin_hub::hub_shortcuts_set_badge,
            plugin_hub::hub_plugin_read_text,
            plugin_hub::hub_plugin_asset_path,
            plugin_hub::hub_everything_status,
            plugin_hub::hub_everything_search,
            plugin_hub::hub_everything_open,
            plugin_hub::hub_everything_reveal,
            plugin_hub::hub_camera_prepare,
            plugin_hub::hub_camera_reset_permission,
            plugin_hub::hub_camera_open_privacy_settings,
            plugin_hub::hub_sysmon_snapshot,
            plugin_install::list_installed_plugins,
            plugin_install::pick_whpx_file,
            plugin_install::pick_plugin_directory,
            plugin_install::install_plugin_from_path,
            plugin_install::uninstall_plugin,
            plugin_install::set_plugin_enabled,
            plugin_install::pack_plugin_directory,
            plugin_install::install_example_plugin,
            companion_scripts::list_script_launchers,
            companion_scripts::upsert_script_launcher,
            companion_scripts::delete_script_launcher,
            companion_scripts::pick_script_file,
            companion_scripts::start_script_launcher,
            companion_scripts::stop_script_launcher,
            commands::apply_window_effect,
            commands::reassert_main_bar_geometry,
            commands::get_material_prefs,
            commands::system_apps_dark,
            commands::set_material_prefs,
            commands::sample_ambient_color,
            commands::get_ambient_mode,
            commands::set_ambient_mode,
            commands::get_window_material,
            commands::set_window_material,
            commands::list_tray_icons,
            commands::get_tray_prefs,
            commands::set_tray_prefs,
            commands::get_input_lang,
            commands::cycle_input_lang,
            commands::toggle_input_ime,
            commands::open_input_lang_settings,
            commands::list_input_layouts,
            commands::select_input_layout,
            commands::open_input_emoji_panel,
            commands::open_touch_keyboard,
            commands::open_keyboard_settings,
            commands::open_input_lang_popup,
            commands::show_chrome_hover_tip,
            commands::commit_chrome_hover_tip,
            commands::close_chrome_hover_tip,
            commands::get_chrome_hover_tip,
            commands::close_input_lang_popup,
            commands::is_input_lang_popup_open,
            commands::get_wifi_state,
            commands::list_wifi_networks,
            commands::set_wifi_enabled,
            commands::connect_wifi,
            commands::disconnect_wifi,
            commands::open_network_settings,
            commands::open_wifi_popup,
            commands::close_wifi_popup,
            commands::is_wifi_popup_open,
            commands::open_wifi_auth_popup,
            commands::close_wifi_auth_popup,
            commands::is_wifi_auth_popup_open,
            commands::get_island_prefs,
            commands::set_island_prefs,
            commands::get_shortcuts_prefs,
            commands::set_shortcuts_prefs,
            db::admin::db_dev_info,
            db::admin::db_dev_list_rows,
            db::admin::db_dev_upsert_row,
            db::admin::db_dev_delete_row,
            db::admin::db_dev_clear_table,
            db::admin::db_dev_backup,
            db::admin::db_dev_pick_restore_file,
            db::admin::db_dev_restore,
            commands::invoke_tray_icon,
            commands::clear_tray_attention,
            commands::open_notification_center,
            commands::get_foreground_app,
            commands::set_system_taskbar_visible,
            dock::get_dock_prefs,
            dock::set_dock_prefs,
            dock::dock_preview_magnification,
            dock::dock_end_magnification_preview,
            dock::import_dockico_ini,
            dock::pick_dockico_file,
            dock::pick_dock_icon_file,
            dock::dock_cache_icon,
            dock::dock_pin_paths,
            dock::dock_pin_item,
            dock::dock_add_separator,
            dock::dock_reorder_items,
            dock::dock_unpin_item,
            dock::open_dock_icon_editor,
            dock::close_dock_icon_editor,
            dock::dock_launch_item,
            dock::dock_capture_window_preview,
            dock::dock_item_window_count,
            dock::dock_close_item_windows,
            dock::close_window_hwnd,
            dock::dock_set_mouse_near_bottom,
            dock::dock_set_live_width,
            dock::dock_set_interaction_hold,
            dock::dock_set_preview_tip_keep,
            dock::dock_set_hover_expand,
            dock::dock_pointer_client_xy,
            dock::get_dock_display_items,
            dock::dock_relayout,
            dock::dock_restore_hidden_items,
            dock::get_dock_hidden_count,
            dock::get_dock_visibility,
            dock::ensure_dock_window,
            commands::show_desktop,
            commands::restart_app,
            commands::exit_app,
            commands::list_hotkey_bindings,
            commands::set_hotkey_binding,
            commands::validate_hotkey_chord,
            commands::suspend_hotkeys_for_recording,
            commands::resume_hotkeys_after_recording,
            win32::app_launch::get_general_prefs,
            win32::app_launch::set_general_prefs,
            win32::app_launch::relaunch_app,
            commands::hub_staging_list,
            commands::hub_staging_summary,
            commands::hub_staging_add_text,
            commands::hub_staging_add_paths,
            commands::hub_staging_add_image_bytes,
            commands::hub_staging_remove,
            commands::hub_staging_clear,
            commands::hub_staging_copy,
            commands::hub_staging_copy_all_paths,
            commands::hub_staging_thumb,
            commands::hub_staging_reveal,
            commands::hub_staging_start_drag,
            commands::hub_island_set_bar,
            commands::hub_island_clear_bar,
            commands::hub_island_claim_scenario,
            commands::hub_island_release_scenario,
            commands::hub_island_get_bound_tray,
            commands::hub_island_open_bound_tray,
            commands::hub_panel_open_session,
            commands::hub_panel_close_session,
            commands::hub_media_send_key,
            commands::hub_notify,
            commands::hub_fetch,
            plugin_install::preview_plugin_from_path,
            plugin_install::preview_example_plugin,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Window Hub")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                // Any GUI teardown (菜单退出 / 进程结束) — tell SCM not to treat as crash.
                #[cfg(windows)]
                crate::win32::autostart_svc::signal_user_quit_unless_relaunching();
            }
        });
}
