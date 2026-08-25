//! Popup backdrop materials — DWMBlurGlass README effects (process-local, no dwm inject).
//!
//! Shared by settings / tray / plugin / dock windows.
//! Ships **system Mica** (`DWMSBT_MAINWINDOW`, Start-menu equivalent).
//! Prefs id remains `mica-alt` for storage compatibility.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tauri::WebviewWindow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WindowMaterial {
    /// Basic pure blur (DWMBlurGlass Blur).
    Blur,
    /// Windows 7 glass-like (DWMBlurGlass Aero).
    Aero,
    /// Acrylic frost with noise (DWMBlurGlass Acrylic).
    Acrylic,
    /// System Mica (Start menu). Prefs key still `mica-alt`.
    #[default]
    MicaAlt,
}

impl WindowMaterial {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blur => "blur",
            Self::Aero => "aero",
            Self::Acrylic => "acrylic",
            // Keep wire id stable; visual is system Mica.
            Self::MicaAlt => "mica-alt",
        }
    }

    /// Parse + migrate legacy kinds from older material.json / material.txt.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "blur" | "frost" | "blur-glass" | "blurglass" | "dwm-blur" => Self::Blur,
            "aero" => Self::Aero,
            "acrylic" | "none" => Self::Acrylic,
            "mica-alt" | "mica_alt" | "tabbed" | "micaalt" | "mica" => Self::MicaAlt,
            _ => Self::MicaAlt,
        }
    }
}

impl Serialize for WindowMaterial {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for WindowMaterial {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(Self::parse(&s))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MaterialPrefs {
    pub kind: WindowMaterial,
    /// MicaAlt: `None` = system; `Some(true/false)` = force dark/light.
    #[serde(default)]
    pub dark: Option<bool>,
    /// Reserved for Blur / Aero / Acrylic when re-enabled (1–255).
    #[serde(default = "default_acrylic_alpha")]
    pub acrylic_alpha: u8,
}

fn default_acrylic_alpha() -> u8 {
    125
}

impl Default for MaterialPrefs {
    fn default() -> Self {
        Self {
            kind: WindowMaterial::MicaAlt,
            dark: None,
            acrylic_alpha: default_acrylic_alpha(),
        }
    }
}

impl MaterialPrefs {
    pub fn normalize(mut self) -> Self {
        if self.acrylic_alpha == 0 {
            self.acrylic_alpha = 1;
        }
        // Temporarily ship MicaAlt only — coerce any persisted kind.
        self.kind = WindowMaterial::MicaAlt;
        self
    }
}

/// Windows Apps theme (`AppsUseLightTheme`: 0 = dark, 1 = light). Default dark if missing.
#[cfg(windows)]
pub fn system_apps_dark() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(key) = hkcu.open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
    else {
        return true;
    };
    match key.get_value::<u32, _>("AppsUseLightTheme") {
        Ok(0) => true,
        Ok(_) => false,
        Err(_) => true,
    }
}

#[cfg(not(windows))]
pub fn system_apps_dark() -> bool {
    true
}

/// `None` (follow system) → concrete bool matching Windows app theme.
pub fn resolve_dark(dark: Option<bool>) -> bool {
    dark.unwrap_or_else(system_apps_dark)
}

#[allow(dead_code)]
#[cfg(windows)]
pub fn apply(window: &WebviewWindow, material: WindowMaterial) -> Result<(), String> {
    apply_prefs(
        window,
        &MaterialPrefs {
            kind: material,
            ..MaterialPrefs::default()
        },
    )
}

/// Clear all backdrop materials (popups / explicit "none" on main).
#[cfg(windows)]
pub fn clear(window: &WebviewWindow) -> Result<(), String> {
    crate::win32::blur_glass::clear(window)
}

#[cfg(windows)]
pub fn apply_prefs(window: &WebviewWindow, prefs: &MaterialPrefs) -> Result<(), String> {
    let prefs = prefs.clone().normalize();
    // Always pass Some(bool): window-vibrancy ignores immersive mode when dark is None,
    // which leaves the previous force sticky and creates a CSS/DWM hybrid "third" look.
    let dark = Some(resolve_dark(prefs.dark));
    crate::win32::blur_glass::apply_effect(window, prefs.kind, dark, prefs.acrylic_alpha)
}

/// Soft reassert for framed settings (no clear/SWCA teardown → no white flash).
#[cfg(windows)]
pub fn reassert_prefs(window: &WebviewWindow, prefs: &MaterialPrefs) -> Result<(), String> {
    let prefs = prefs.clone().normalize();
    let dark = Some(resolve_dark(prefs.dark));
    let framed_mica = matches!(prefs.kind, WindowMaterial::MicaAlt)
        && match window.label() {
            "settings" | "dock-icon-editor" | "plugin-window" => true,
            "plugin-popup" => {
                crate::win32::blur_glass::is_native_frame_plugin_popup(window)
            }
            _ => false,
        };
    if framed_mica {
        crate::win32::blur_glass::reassert_settings_frame_mica(window, dark)
    } else {
        apply_prefs(window, &prefs)
    }
}

/// Soft dock-glass refresh (no nested deferred frost storms).
#[cfg(windows)]
pub fn reassert_dock_glass(window: &WebviewWindow, prefs: &MaterialPrefs) -> Result<(), String> {
    let dark = Some(resolve_dark(prefs.dark));
    crate::win32::blur_glass::reassert_dock_glass_layer(window, dark)
}

#[cfg(not(windows))]
pub fn reassert_dock_glass(_window: &WebviewWindow, _prefs: &MaterialPrefs) -> Result<(), String> {
    Ok(())
}

/// Early HTML theme tokens for dock / dock-glass (before React paints).
pub fn theme_bootstrap_script(prefs: &MaterialPrefs) -> String {
    let dark = resolve_dark(prefs.dark);
    let theme = if dark { "dark" } else { "light" };
    format!(
        r#"window.__WH_THEME_DARK__={dark};(function(){{var r=document.documentElement;r.dataset.theme="{theme}";r.style.colorScheme="{theme}";}})();"#
    )
}

#[cfg(windows)]
pub fn apply_prefs_deferred(window: &WebviewWindow, prefs: &MaterialPrefs) {
    use tauri::Manager;

    let prefs = prefs.clone().normalize();
    let label = window.label().to_string();
    let app = window.app_handle().clone();
    let hwnd0 = window.hwnd().ok().map(|h| h.0 as isize);
    let framed = label == "settings"
        || label == "dock-icon-editor"
        || label == "plugin-window"
        || (label == "plugin-popup"
            && crate::win32::blur_glass::is_native_frame_plugin_popup(window));
    let dockish = label == "dock" || label == "dock-glass";
    let _ = apply_prefs(window, &prefs);
    // Framed Mica: one late soft retry. Dock: short soft reassert only —
    // full apply_dock_glass_layer stacks deferred frost and flashes dark.
    let delays: &'static [u64] = if framed {
        &[180]
    } else if dockish {
        &[120, 320]
    } else {
        &[40, 100, 220, 450, 800]
    };
    std::thread::spawn(move || {
        for ms in delays {
            std::thread::sleep(std::time::Duration::from_millis(*ms));
            let Some(win) = app.get_webview_window(&label) else {
                return;
            };
            let hwnd1 = win.hwnd().ok().map(|h| h.0 as isize);
            if hwnd0.is_some() && hwnd0 != hwnd1 {
                return;
            }
            if let Some(h) = hwnd1 {
                use windows::Win32::Foundation::HWND;
                use windows::Win32::UI::WindowsAndMessaging::IsWindow;
                if !unsafe { IsWindow(HWND(h as _)) }.as_bool() {
                    return;
                }
            }
            if framed {
                let _ = reassert_prefs(&win, &prefs);
            } else if label == "dock-glass" {
                let _ = reassert_dock_glass(&win, &prefs);
            } else {
                let _ = apply_prefs(&win, &prefs);
            }
        }
    });
}

#[cfg(not(windows))]
pub fn apply(_window: &WebviewWindow, _material: WindowMaterial) -> Result<(), String> {
    Ok(())
}

#[cfg(not(windows))]
pub fn clear(_window: &WebviewWindow) -> Result<(), String> {
    Ok(())
}

#[cfg(not(windows))]
pub fn apply_prefs(_window: &WebviewWindow, _prefs: &MaterialPrefs) -> Result<(), String> {
    Ok(())
}

#[cfg(not(windows))]
pub fn reassert_prefs(_window: &WebviewWindow, _prefs: &MaterialPrefs) -> Result<(), String> {
    Ok(())
}

#[cfg(not(windows))]
pub fn apply_prefs_deferred(_window: &WebviewWindow, _prefs: &MaterialPrefs) {}
