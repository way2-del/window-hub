//! Popup backdrop materials — DWMBlurGlass README effects (process-local, no dwm inject).
//!
//! Shared by settings / tray / plugin / dock windows.
//! Ships **system Mica** (`DWMSBT_MAINWINDOW`, Start-menu equivalent).
//! Prefs id remains `mica-alt` for storage compatibility.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::WebviewWindow;

/// Live flag for main AppBar frost (default off — solid ambient tint).
static TOPBAR_FROST: AtomicBool = AtomicBool::new(false);

pub fn set_topbar_frost_enabled(on: bool) {
    TOPBAR_FROST.store(on, Ordering::SeqCst);
}

pub fn topbar_frost_enabled() -> bool {
    TOPBAR_FROST.load(Ordering::SeqCst)
}

/// Apply or clear frosted blur on the main strip according to prefs.
#[cfg(windows)]
pub fn sync_topbar_frost(window: &WebviewWindow) -> Result<(), String> {
    if topbar_frost_enabled() {
        crate::win32::blur_glass::apply_topbar_frost(window)
    } else {
        crate::win32::blur_glass::clear(window)?;
        // Keep WebView clear so opaque CSS ambient paints correctly.
        use tauri::utils::config::Color;
        let _ = window.set_background_color(Some(Color(0, 0, 0, 0)));
        crate::win32::blur_glass::strip_dwm_chrome_border(window);
        Ok(())
    }
}

#[cfg(not(windows))]
pub fn sync_topbar_frost(_window: &WebviewWindow) -> Result<(), String> {
    Ok(())
}

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

/// Clear all backdrop materials (prefer `sync_topbar_frost` for the main strip).
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

#[cfg(windows)]
pub fn apply_prefs_deferred(window: &WebviewWindow, prefs: &MaterialPrefs) {
    let prefs = prefs.clone().normalize();
    let _ = apply_prefs(window, &prefs);
    // Win10 / hard-safe: never spawn deferred DWM retries (freeze source).
    if crate::win32::blur_glass::is_hard_safe() {
        return;
    }
    let win = window.clone();
    std::thread::spawn(move || {
        for ms in [80_u64, 280] {
            std::thread::sleep(std::time::Duration::from_millis(ms));
            if crate::win32::blur_glass::is_hard_safe() {
                let _ = apply_prefs(&win, &prefs);
                return;
            }
            let _ = apply_prefs(&win, &prefs);
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
pub fn apply_prefs_deferred(_window: &WebviewWindow, _prefs: &MaterialPrefs) {}
