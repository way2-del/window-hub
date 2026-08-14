mod autostart;
mod commands;
mod companion_scripts;
mod db;
mod dock;
mod ecs;
mod plugin_hub;
mod plugin_install;
mod sousou;
mod staging;
mod win32;
mod windows_service;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{Emitter, Manager, PhysicalPosition, PhysicalSize};

use crate::commands::initial_material_state;
use crate::ecs::{spawn_ecs_thread, EcsHandle};
use crate::plugin_hub::ShortcutsPinStore;
use crate::win32::appbar;
use crate::win32::topmost::force_topmost;
use crate::windows_service::WindowsService;

/// 因全屏游戏隐藏顶栏时为 true；watchdog 期间勿强制置顶/重挂 AppBar。
static HIDDEN_FOR_FULLSCREEN: AtomicBool = AtomicBool::new(false);

fn hwnd_of(window: &tauri::WebviewWindow) -> Option<isize> {
    window.hwnd().ok().map(|h| h.0 as isize)
}

/// 顶栏贴齐当前显示器顶部，并强制铺满整屏宽度（左右留给系统材质）。
fn pin_top_bar(window: &tauri::WebviewWindow) {
    let Ok(Some(monitor)) = window.current_monitor() else {
        return;
    };
    let Ok(size) = window.outer_size() else {
        return;
    };
    let Ok(pos) = window.outer_position() else {
        return;
    };

    let screen = monitor.size();
    let origin = monitor.position();
    let x = origin.x;
    let y = origin.y;

    if size.width != screen.width {
        let _ = window.set_size(PhysicalSize::new(screen.width, size.height));
    }
    if pos.x != x || pos.y != y {
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
}

fn reassert_window(window: &tauri::WebviewWindow) {
    if HIDDEN_FOR_FULLSCREEN.load(Ordering::SeqCst) {
        return;
    }
    if let Some(hwnd) = hwnd_of(window) {
        // 持续排除 Alt+Tab / Win+Tab，防止样式被重置后又出现在窗口切换里
        crate::win32::switcher::exclude_from_switcher(hwnd);
    }
    #[cfg(windows)]
    crate::win32::blur_glass::strip_dwm_chrome_border(window);
    if crate::win32::topmost::is_yielding() {
        // Keep geometry pinned, but don't steal Z-order over tray menus.
        pin_top_bar(window);
        return;
    }
    let _ = window.set_always_on_top(true);
    if let Some(hwnd) = hwnd_of(window) {
        force_topmost(hwnd);
    }
    pin_top_bar(window);
}

fn spawn_watchdog(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        if let Some(window) = app.get_webview_window("main") {
            if let Some(hwnd) = hwnd_of(&window) {
                appbar::register(hwnd);
            }
            // 顶栏磨砂：按 prefs.topbarFrost（默认关）
            let _ = crate::win32::material::sync_topbar_frost(&window);
            reassert_window(&window);
        }

        let mut ticks: u32 = 0;
        loop {
            std::thread::sleep(Duration::from_millis(2000));
            if HIDDEN_FOR_FULLSCREEN.load(Ordering::SeqCst) {
                continue;
            }
            let Some(window) = app.get_webview_window("main") else {
                break;
            };
            reassert_window(&window);
            ticks = ticks.wrapping_add(1);
            if ticks % 5 == 0 {
                if let Some(hwnd) = hwnd_of(&window) {
                    appbar::sync(hwnd);
                }
                let _ = crate::win32::material::sync_topbar_frost(&window);
            }
        }
    });
}

/// 前台为独占/无边框全屏（游戏）时隐藏岛并释放工作区；退出全屏后再显示。
fn spawn_fullscreen_watcher(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(800));
        let mut hidden = false;

        loop {
            std::thread::sleep(Duration::from_millis(350));
            let Some(window) = app.get_webview_window("main") else {
                break;
            };
            let self_hwnd = hwnd_of(&window);
            let should_hide = crate::win32::fullscreen::should_hide_strip(self_hwnd);

            if should_hide && !hidden {
                HIDDEN_FOR_FULLSCREEN.store(true, Ordering::SeqCst);
                appbar::suspend();
                let _ = window.hide();
                // 设置窗若开着一并藏起，避免盖在游戏上
                if let Some(settings) = app.get_webview_window("settings") {
                    let _ = settings.hide();
                }
                hidden = true;
            } else if !should_hide && hidden {
                let _ = window.show();
                if let Some(hwnd) = self_hwnd {
                    appbar::register(hwnd);
                }
                HIDDEN_FOR_FULLSCREEN.store(false, Ordering::SeqCst);
                reassert_window(&window);
                hidden = false;
            }
        }
    });
}

/// 最大化窗口顶栏取色；切窗后采 ~3s 再锁定，锁定后只侦测窗口切换。
fn spawn_ambient_watcher(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(700));
        let mut synced = false;

        // 启动时采一次
        if let Some(window) = app.get_webview_window("main") {
            let _ = crate::win32::material::sync_topbar_frost(&window);
            synced = true;
            if let Some(strip) = crate::win32::ambient::poll_changed(hwnd_of(&window)) {
                let _ = app.emit("ambient-color", strip);
            } else {
                let strip = crate::win32::ambient::sample(hwnd_of(&window));
                let _ = app.emit("ambient-color", strip);
            }
        }

        loop {
            // 稳定期适度密采；锁定后低频侦测切窗。内存吃紧时大幅降频，避免跟色拖垮机器。
            let pressure = crate::win32::system_memory::physical_mem_percent() >= 88;
            let ms = if pressure {
                if crate::win32::ambient::is_settling() {
                    1200
                } else {
                    3200
                }
            } else if crate::win32::ambient::is_settling() {
                450
            } else {
                1400
            };
            std::thread::sleep(Duration::from_millis(ms));
            let Some(window) = app.get_webview_window("main") else {
                break;
            };

            if !synced {
                let _ = crate::win32::material::sync_topbar_frost(&window);
                synced = true;
            }

            if let Some(strip) = crate::win32::ambient::poll_changed(hwnd_of(&window)) {
                let _ = app.emit("ambient-color", strip);
            }
        }
    });
}

/// 用 explorer 托盘钩子（失败则 spy fallback）监听系统托盘；变化时推送前端。
fn spawn_tray_watcher(app: tauri::AppHandle) {
    let app_icons = app.clone();
    let app_attn = app.clone();
    let app_clear = app.clone();
    crate::win32::tray::start(
        move |icons| {
            let _ = app_icons.emit("tray-icons", &icons);
        },
        move |attn| {
            let _ = app_attn.emit("tray-attention", &attn);
        },
        move |id| {
            let _ = app_clear.emit("tray-attention-cleared", &serde_json::json!({ "id": id }));
        },
    );
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let db = crate::db::init().map_err(|e| {
                eprintln!("[db] init failed: {e}");
                e
            })?;
            app.manage(db);
            let handle = spawn_ecs_thread(app.handle().clone());
            app.manage(handle);
            app.manage(initial_material_state());
            // Win10 / 精简包：尽早判定 opaque 弹窗策略（HARD_SAFE）
            #[cfg(windows)]
            {
                let _ = crate::win32::blur_glass::is_hard_safe();
            }
            app.manage(WindowsService::start(app.handle().clone()));
            app.manage(crate::dock::DockVisibility::new());
            let pins = ShortcutsPinStore::new();
            pins.load_all_from_db();
            app.manage(pins);
            let _ = crate::plugin_install::list_installed_plugins_sync();
            crate::plugin_install::ensure_official_plugins(app.handle());
            crate::win32::ambient::set_mode(commands::load_ambient_mode());
            crate::win32::tray::set_prefs(commands::load_tray_prefs());
            {
                let frost = commands::get_island_prefs().topbar_frost;
                crate::win32::material::set_topbar_frost_enabled(frost);
            }

            if let Some(window) = app.get_webview_window("main") {
                if let Some(hwnd) = hwnd_of(&window) {
                    crate::win32::topmost::set_main_hwnd(hwnd);
                }
                reassert_window(&window);
                let _ = crate::win32::material::sync_topbar_frost(&window);
            }

            spawn_watchdog(app.handle().clone());
            spawn_ambient_watcher(app.handle().clone());
            spawn_fullscreen_watcher(app.handle().clone());
            spawn_tray_watcher(app.handle().clone());
            crate::companion_scripts::start_hub_associated_launchers();
            crate::dock::bootstrap_dock(app.handle());
            crate::sousou::bootstrap(app.handle());
            // Warm popup webviews in background so first open isn't a cold create.
            crate::commands::warm_popup_windows(app.handle().clone());
            #[cfg(windows)]
            crate::win32::system_monitor::start(app.handle().clone());
            #[cfg(windows)]
            crate::win32::drag_watch::start(app.handle().clone());

            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                    if window.label() == "main"
                        && !HIDDEN_FOR_FULLSCREEN.load(Ordering::SeqCst)
                    {
                        if let Some(w) = window.app_handle().get_webview_window("main") {
                            pin_top_bar(&w);
                        }
                    }
                }
                tauri::WindowEvent::Focused(focused) => {
                    // Dock must never keep activation chrome — re-clear on any focus pulse.
                    if window.label() == "dock" || window.label() == "dock-glass" {
                        let app = window.app_handle().clone();
                        let label = window.label().to_string();
                        if let Some(w) = app.get_webview_window(&label) {
                            let _ = w.set_focusable(false);
                            if let Ok(hwnd) = w.hwnd() {
                                crate::dock::reclear_dock_frame(hwnd.0 as isize);
                            }
                            if label == "dock" {
                                let _ =
                                    crate::win32::blur_glass::apply_dock_icons_layer(&w, None);
                            }
                        }
                        let _ = focused;
                    }
                    // 主岛获得焦点：关掉仍开着的弹窗（点岛栏空白/其它 chip 等常见关闭方式）
                    if window.label() == "main" && *focused {
                        let app = window.app_handle().clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(8));
                            // 阻塞锁：避免 open 刚结束 try_lock 失败导致弹窗关不掉
                            crate::commands::with_popup_ops_pub(|| {
                                if !crate::commands::plugin_popup_blur_suppressed() {
                                    if let Some(w) = app.get_webview_window("plugin-popup") {
                                        if w.is_visible().unwrap_or(false) {
                                            let id = crate::commands::peek_plugin_popup_id(&w);
                                            crate::commands::cancel_plugin_popup_reveal_fallback();
                                            let _ = w.hide();
                                            crate::commands::note_plugin_popup_focus_close(
                                                id.as_deref(),
                                            );
                                            let _ = app.emit("plugin-popup-closed", ());
                                        }
                                    }
                                }
                                if !crate::commands::tray_popup_blur_suppressed() {
                                    if let Some(w) = app.get_webview_window("tray-popup") {
                                        if w.is_visible().unwrap_or(false) {
                                            let _ = w.hide();
                                            let _ = app.emit("tray-popup-closed", ());
                                        }
                                    }
                                }
                                if !crate::commands::system_flyout_blur_suppressed() {
                                    crate::commands::cancel_system_flyout_reveal_fallback();
                                    if let Some(w) = app.get_webview_window("system-flyout") {
                                        if w.is_visible().unwrap_or(false) {
                                            let _ = w.hide();
                                            let _ = app.emit("system-flyout-closed", ());
                                        }
                                    }
                                }
                                if let Some(w) = app.get_webview_window("status-menu-popup") {
                                    if w.is_visible().unwrap_or(false) {
                                        let _ = w.hide();
                                        let _ = app.emit("status-menu-popup-closed", ());
                                    }
                                }
                            });
                        });
                    }
                    // 弹窗失焦关闭。托盘用 hide（可复用）；勿在 hide 前 eval（拖慢关闭）。
                    if (window.label() == "tray-popup"
                        || window.label() == "plugin-popup"
                        || window.label() == "status-menu-popup"
                        || window.label() == "system-flyout")
                        && !*focused
                    {
                        let label = window.label().to_string();
                        let app = window.app_handle().clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(8));
                            if label == "tray-popup"
                                && crate::commands::tray_popup_blur_suppressed()
                            {
                                return;
                            }
                            if label == "system-flyout"
                                && crate::commands::system_flyout_blur_suppressed()
                            {
                                return;
                            }
                            if label == "plugin-popup"
                                && crate::commands::plugin_popup_blur_suppressed()
                            {
                                return;
                            }
                            crate::commands::with_popup_ops_pub(|| {
                                if let Some(w) = app.get_webview_window(&label) {
                                    if w.is_focused().unwrap_or(false) {
                                        return;
                                    }
                                    let plugin_id = if label == "plugin-popup" {
                                        crate::commands::peek_plugin_popup_id(&w)
                                    } else {
                                        None
                                    };
                                    if label == "plugin-popup" {
                                        crate::commands::cancel_plugin_popup_reveal_fallback();
                                    }
                                    let _ = w.hide();
                                    match label.as_str() {
                                        "tray-popup" => {
                                            let _ = app.emit("tray-popup-closed", ());
                                        }
                                        "system-flyout" => {
                                            let _ = app.emit("system-flyout-closed", ());
                                        }
                                        "plugin-popup" => {
                                            crate::commands::note_plugin_popup_focus_close(
                                                plugin_id.as_deref(),
                                            );
                                            let _ = app.emit("plugin-popup-closed", ());
                                        }
                                        "status-menu-popup" => {
                                            let _ = app.emit("status-menu-popup-closed", ());
                                        }
                                        _ => {}
                                    }
                                } else {
                                    match label.as_str() {
                                        "tray-popup" => {
                                            let _ = app.emit("tray-popup-closed", ());
                                        }
                                        "system-flyout" => {
                                            let _ = app.emit("system-flyout-closed", ());
                                        }
                                        "plugin-popup" => {
                                            let _ = app.emit("plugin-popup-closed", ());
                                        }
                                        "status-menu-popup" => {
                                            let _ = app.emit("status-menu-popup-closed", ());
                                        }
                                        _ => {}
                                    }
                                }
                            });
                        });
                    }
                }
                tauri::WindowEvent::Destroyed => {
                    if window.label() == "tray-popup" {
                        let _ = window.app_handle().emit("tray-popup-closed", ());
                    }
                    if window.label() == "system-flyout" {
                        let _ = window.app_handle().emit("system-flyout-closed", ());
                    }
                    if window.label() == "plugin-popup" {
                        let _ = window.app_handle().emit("plugin-popup-closed", ());
                    }
                    if window.label() == "status-menu-popup" {
                        let _ = window.app_handle().emit("status-menu-popup-closed", ());
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
            commands::focus_or_minimize_open_window,
            commands::attach_window,
            commands::detach_window,
            commands::set_roi,
            commands::swap_slots,
            commands::forward_pointer,
            commands::forward_key,
            commands::self_hwnd,
            commands::dock_set_visual_height,
            commands::float_overlay,
            commands::open_settings_window,
            commands::close_settings_window,
            commands::open_tray_popup,
            commands::reveal_tray_popup,
            commands::close_tray_popup,
            commands::is_tray_popup_open,
            commands::suppress_tray_popup_blur,
            commands::suppress_plugin_popup_blur,
            commands::open_system_flyout,
            commands::reveal_system_flyout,
            commands::close_system_flyout,
            commands::is_system_flyout_open,
            commands::get_system_flyout_kind,
            commands::suppress_system_flyout_blur,
            commands::get_system_radio_snapshot,
            commands::refresh_system_status,
            commands::get_wifi_password,
            commands::connect_wifi_network,
            commands::open_wifi_settings,
            commands::open_network_settings,
            commands::open_bluetooth_settings,
            commands::set_bluetooth_device,
            commands::open_ime_picker,
            commands::open_ime_settings,
            commands::set_system_volume,
            commands::set_system_volume_muted,
            commands::open_sound_settings,
            commands::play_volume_preview,
            commands::list_audio_output_devices,
            commands::set_audio_output_device,
            commands::open_power_settings,
            commands::list_memory_top,
            commands::purge_system_memory,
            commands::open_task_manager,
            commands::list_network_top,
            commands::set_process_net_blocked,
            commands::open_status_menu_popup,
            commands::close_status_menu_popup,
            commands::is_status_menu_popup_open,
            commands::open_plugin_popup,
            commands::reveal_plugin_popup,
            commands::close_plugin_popup,
            commands::is_plugin_popup_open,
            commands::get_plugin_popup_id,
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
            commands::get_material_prefs,
            commands::is_glass_compat_mode,
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
            commands::get_island_prefs,
            commands::set_island_prefs,
            autostart::get_open_at_login,
            autostart::set_open_at_login,
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
            db::admin::export_hub_backup,
            db::admin::pick_hub_backup_file,
            db::admin::import_hub_backup,
            commands::invoke_tray_icon,
            commands::clear_tray_attention,
            commands::open_notification_center,
            commands::get_foreground_app,
            commands::is_system_taskbar_visible,
            commands::set_system_taskbar_visible,
            dock::get_dock_prefs,
            dock::set_dock_prefs,
            dock::import_dockico_ini,
            dock::pick_dockico_file,
            dock::pick_dock_app_file,
            dock::dock_launch_item,
            dock::dock_launch_path,
            dock::dock_open_jump_item,
            dock::dock_add_app,
            dock::dock_pin_running_app,
            dock::dock_add_separator,
            dock::dock_remove_adjacent_separator,
            dock::dock_remove_item,
            dock::dock_list_item_windows,
            dock::dock_close_item_windows,
            dock::dock_close_hwnd,
            dock::refresh_dock_preview,
            dock::dock_capture_item_previews,
            dock::dock_capture_exe_previews,
            dock::open_dock_preview,
            dock::close_dock_preview,
            dock::dock_set_extra_headroom,
            dock::dock_touch_noactivate,
            dock::dock_set_runtime_extra_width,
            dock::dock_resolve_exe_icon,
            dock::open_dock_item_menu,
            dock::close_dock_item_menu,
            dock::reveal_dock_item_menu,
            dock::get_dock_item_menu_payload,
            dock::dock_set_mouse_near_bottom,
            dock::get_dock_visibility,
            dock::ensure_dock_window,
            commands::show_desktop,
            commands::open_system_tool,
            commands::restart_app,
            commands::exit_app,
            sousou::sousou_toggle,
            sousou::sousou_open,
            sousou::sousou_hide,
            sousou::sousou_get_config,
            sousou::sousou_set_config,
            sousou::sousou_ensure_everything,
            sousou::sousou_everything_status,
            sousou::sousou_list_apps,
            sousou::sousou_list_recent,
            sousou::sousou_search,
            sousou::sousou_open_path,
            sousou::sousou_reveal_path,
            sousou::sousou_open_system,
            sousou::sousou_refresh_apps,
            sousou::sousou_pick_folder,
            sousou::sousou_first_folder,
            sousou::sousou_paths_to_shortcuts,
            sousou::sousou_list_dir,
            sousou::sousou_import_into_folder,
            sousou::sousou_resolve_icon,
            sousou::sousou_icon_cache_stats,
            sousou::sousou_clear_icon_cache,
            sousou::sousou_seed_tabs,
            commands::hub_staging_list,
            commands::hub_staging_summary,
            commands::hub_staging_add_text,
            commands::hub_staging_add_paths,
            commands::hub_staging_add_image_bytes,
            commands::hub_staging_remove,
            commands::hub_staging_remove_many,
            commands::hub_staging_clear,
            commands::hub_staging_copy,
            commands::hub_staging_copy_files,
            commands::hub_staging_copy_paths,
            commands::hub_staging_copy_all_paths,
            commands::hub_staging_thumb,
            commands::hub_staging_reveal,
            commands::hub_staging_open,
            commands::hub_staging_start_drag,
            commands::hub_staging_pick_files,
            commands::hub_staging_pick_folders,
            commands::hub_island_set_bar,
            commands::hub_island_clear_bar,
            commands::hub_lyric_mirror_set_slot,
            commands::hub_lyric_mirror_clear,
            commands::hub_lyric_mirror_current_layout,
            commands::hub_lyric_mirror_remember_layout,
            commands::hub_netease_now_playing,
            commands::hub_media_transport,
            commands::hub_media_open_netease,
            commands::hub_panel_open_session,
            commands::hub_panel_close_session,
            commands::hub_notify,
            commands::hub_fetch,
            plugin_install::preview_plugin_from_path,
            plugin_install::preview_example_plugin,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Window Hub");
}
