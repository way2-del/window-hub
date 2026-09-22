//! Win32 helpers for window enumeration, parking, capture, and input.

pub mod ambient;
pub mod app_launch;
#[cfg(windows)]
pub mod autostart_svc;
#[cfg(windows)]
pub mod single_instance;
pub mod appbar;
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
pub mod work_area;
#[cfg(windows)]
pub mod tray_hook_ipc;
#[cfg(windows)]
pub mod tray_hook_host;
#[cfg(windows)]
pub mod tray_registry;
#[cfg(windows)]
pub mod tray_uia;
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
