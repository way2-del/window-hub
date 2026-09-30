//! Win32 helpers for window enumeration, parking, capture, and input.

pub mod ambient;
pub mod notification_focus;
pub mod app_launch;
#[cfg(windows)]
pub mod autostart_svc;
#[cfg(windows)]
pub mod single_instance;
pub mod appbar;
#[cfg(windows)]
pub mod appbar_window;
pub mod dock_appbar;
#[cfg(windows)]
pub mod blur_glass;
#[cfg(windows)]
pub mod bar_comp;
#[cfg(windows)]
pub mod dock_comp;
#[cfg(windows)]
pub mod island_bar_glass;
pub mod capture;
pub mod enum_windows;
pub mod fullscreen;
pub mod hang;
pub mod click_trace;
pub mod input;
pub mod material;
pub mod park;
pub mod status_menu;
pub mod switcher;
pub mod topmost;
pub mod tray;
pub mod popup_fit;
pub mod work_area;
#[cfg(windows)]
pub mod satellite_appbar;
#[cfg(not(windows))]
pub mod satellite_appbar {
    pub fn register_and_sync(_hwnd_raw: isize) {}
    pub fn unregister(_hwnd_raw: isize) {}
    pub fn unregister_all() {}
}
#[cfg(windows)]
pub mod satellite_dock_appbar;
#[cfg(not(windows))]
pub mod satellite_dock_appbar {
    pub fn register_and_sync(_hwnd_raw: isize, _bottom_offset_px: u32) {}
    pub fn unregister(_hwnd_raw: isize) {}
    pub fn unregister_all() {}
}
#[cfg(windows)]
pub mod max_clamp;
#[cfg(not(windows))]
pub mod max_clamp {
    pub fn tick(_self_hwnd: Option<isize>) {}
}
#[cfg(windows)]
pub mod tray_hook_ipc;
#[cfg(windows)]
pub mod tray_hook_host;
#[cfg(windows)]
pub mod tray_registry;
#[cfg(windows)]
pub mod tray_icon_cache;
#[cfg(windows)]
pub mod tray_uia;
#[cfg(windows)]
mod tray_native;
#[cfg(windows)]
pub mod tray_shell_click;
#[cfg(windows)]
pub mod tray_toolbar;
pub mod input_lang;
pub mod wifi;
#[cfg(windows)]
pub mod webview_camera;
#[cfg(windows)]
pub mod hotkey_registry;
#[cfg(not(windows))]
pub mod hotkey_registry {
    use serde::{Deserialize, Serialize};
    use tauri::AppHandle;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct HotkeyBindingDto {
        pub id: String,
        pub scope: String,
        pub plugin_id: Option<String>,
        pub plugin_name: Option<String>,
        pub key: String,
        pub label: String,
        pub action: String,
        pub chord: String,
        pub aliases_system_search: bool,
    }

    pub fn spawn(_app: AppHandle) {}
    pub fn reload(_app: &AppHandle) {}
    pub fn suspend_for_recording() {}
    pub fn resume_after_recording() {}
    pub fn list_bindings() -> Result<Vec<HotkeyBindingDto>, String> {
        Ok(vec![])
    }
    pub fn set_binding(
        _app: &AppHandle,
        _id: &str,
        _chord: &str,
    ) -> Result<Vec<HotkeyBindingDto>, String> {
        Ok(vec![])
    }
    pub fn validate_chord_available(_id: &str, chord: &str) -> Result<String, String> {
        Ok(chord.to_string())
    }
    pub fn normalize_chord(raw: &str) -> Result<String, String> {
        Ok(raw.trim().to_string())
    }
}

#[cfg(windows)]
pub mod audio_mixer;
#[cfg(windows)]
pub mod bluetooth;
#[cfg(windows)]
pub mod control_center;
