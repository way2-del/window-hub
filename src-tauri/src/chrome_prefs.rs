//! Top-chrome visibility prefs (menubar chips + system tray rail).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromePrefs {
    /// System tray icons + overflow chevron / dropdown.
    #[serde(default = "default_true")]
    pub show_tray: bool,
    /// WLAN / ethernet chip.
    #[serde(default = "default_true")]
    pub show_wifi: bool,
    /// Clock / date chip.
    #[serde(default = "default_true")]
    pub show_clock: bool,
    /// Input language + IME chips.
    #[serde(default = "default_true")]
    pub show_ime: bool,
    /// Control center button + popup.
    #[serde(default = "default_true")]
    pub show_control_center: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ChromePrefs {
    fn default() -> Self {
        Self {
            show_tray: true,
            show_wifi: true,
            show_clock: true,
            show_ime: true,
            show_control_center: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromeRailTier {
    /// All modules off: left+right shortcuts.
    Dual,
    /// Tray icons off, ≥1 system chip: far-right chips + inner right shortcuts.
    Hybrid,
    /// Tray icons on: full tray, no right shortcuts.
    Tray,
}

fn has_system_chrome_chips(prefs: &ChromePrefs) -> bool {
    prefs.show_wifi || prefs.show_clock || prefs.show_ime || prefs.show_control_center
}

pub fn chrome_rail_tier(prefs: &ChromePrefs) -> ChromeRailTier {
    if prefs.show_tray {
        ChromeRailTier::Tray
    } else if has_system_chrome_chips(prefs) {
        ChromeRailTier::Hybrid
    } else {
        ChromeRailTier::Dual
    }
}

/// Right shortcuts wing exists in Dual and Hybrid.
pub fn has_right_shortcuts_wing(prefs: &ChromePrefs) -> bool {
    chrome_rail_tier(prefs) != ChromeRailTier::Tray
}

/// All right-rail modules off → island left+right become shortcut strips.
pub fn is_dual_shortcuts_mode(prefs: &ChromePrefs) -> bool {
    chrome_rail_tier(prefs) == ChromeRailTier::Dual
}

static TRAY_STARTED: AtomicBool = AtomicBool::new(false);
static WIFI_STARTED: AtomicBool = AtomicBool::new(false);
static IME_STARTED: AtomicBool = AtomicBool::new(false);

pub fn load() -> ChromePrefs {
    match crate::db::with_conn(|c| crate::db::chrome_get(c)) {
        Ok(Some(v)) => serde_json::from_value(v).unwrap_or_default(),
        _ => ChromePrefs::default(),
    }
}

fn save(prefs: &ChromePrefs) -> Result<(), String> {
    let v = serde_json::to_value(prefs).map_err(|e| e.to_string())?;
    crate::db::with_conn(|c| {
        // Existing installs may predate prefs_chrome; create before write.
        c.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS prefs_chrome (
              id INTEGER PRIMARY KEY CHECK (id = 1),
              data_json TEXT NOT NULL,
              updated_at INTEGER NOT NULL
            );
            "#,
        )
        .map_err(|e| e.to_string())?;
        crate::db::chrome_set(c, &v)
    })
}

fn close_related_popups(app: &AppHandle, prefs: &ChromePrefs) {
    if !prefs.show_tray {
        crate::commands::hide_chrome_popup(app, "tray-popup");
        crate::win32::tray::set_emit_paused(true);
    } else {
        crate::win32::tray::set_emit_paused(false);
    }
    if !prefs.show_wifi {
        crate::commands::hide_chrome_popup(app, "wifi-popup");
        crate::commands::hide_chrome_popup(app, "wifi-auth-popup");
    }
    if !prefs.show_ime {
        crate::commands::hide_chrome_popup(app, "input-lang-popup");
    }
    if !prefs.show_control_center {
        crate::commands::hide_chrome_popup(app, "control-center-popup");
    }
}

/// Start tray hook once (when enabled). Safe to call repeatedly.
pub fn ensure_tray_started(app: &AppHandle) {
    if !load().show_tray {
        return;
    }
    if !crate::win32::tray::tray_boot_enabled() {
        return;
    }
    if TRAY_STARTED.swap(true, Ordering::SeqCst) {
        crate::win32::tray::set_emit_paused(false);
        return;
    }
    let app_icons = app.clone();
    let app_attn = app.clone();
    let app_prefs = app.clone();
    crate::win32::tray::start(
        move |icons| {
            if load().show_tray {
                let _ = app_icons.emit("tray-icons", &icons);
            }
        },
        move |attn| {
            if load().show_tray {
                let _ = app_attn.emit("tray-attention", &attn);
            }
        },
        move |prefs| {
            if load().show_tray {
                let _ = app_prefs.emit("tray-prefs", &prefs);
            }
        },
    );
    crate::win32::tray::set_emit_paused(false);
    crate::win32::tray::flush_publish();
}

pub fn ensure_wifi_started(app: &AppHandle) {
    if !load().show_wifi {
        return;
    }
    if WIFI_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::Builder::new()
        .name("wifi".into())
        .spawn(move || {
            crate::win32::wifi::start(move |state| {
                if load().show_wifi {
                    let _ = app.emit("wifi-state", &state);
                }
            });
        })
        .ok();
}

pub fn ensure_ime_started(app: &AppHandle) {
    if !load().show_ime {
        return;
    }
    if IME_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::Builder::new()
        .name("input-lang".into())
        .spawn(move || {
            crate::win32::input_lang::start(move |state| {
                if load().show_ime {
                    let _ = app.emit("input-lang", &state);
                }
            });
        })
        .ok();
}

#[tauri::command]
pub fn get_chrome_prefs() -> ChromePrefs {
    load()
}

#[tauri::command]
pub fn set_chrome_prefs(app: AppHandle, prefs: ChromePrefs) -> Result<ChromePrefs, String> {
    let prev = load();
    let next = ChromePrefs {
        show_tray: prefs.show_tray,
        show_wifi: prefs.show_wifi,
        show_clock: prefs.show_clock,
        show_ime: prefs.show_ime,
        show_control_center: prefs.show_control_center,
    };
    let prev_tier = chrome_rail_tier(&prev);
    let next_tier = chrome_rail_tier(&next);
    save(&next)?;
    close_related_popups(&app, &next);
    if next.show_tray {
        ensure_tray_started(&app);
    }
    if next.show_wifi {
        ensure_wifi_started(&app);
    }
    if next.show_ime {
        ensure_ime_started(&app);
    }
    // Only entering full tray yields the right shortcuts wing.
    if prev_tier != ChromeRailTier::Tray && next_tier == ChromeRailTier::Tray {
        let n = disable_right_side_shortcut_plugins(&app);
        let _ = app.emit(
            "chrome-dual-shortcuts",
            json!({ "transition": "exit", "tier": "tray", "disabledCount": n }),
        );
    } else if prev_tier != ChromeRailTier::Dual && next_tier == ChromeRailTier::Dual {
        let _ = app.emit(
            "chrome-dual-shortcuts",
            json!({ "transition": "enter", "tier": "dual", "disabledCount": 0 }),
        );
    } else if prev_tier != ChromeRailTier::Hybrid && next_tier == ChromeRailTier::Hybrid {
        let _ = app.emit(
            "chrome-dual-shortcuts",
            json!({ "transition": "hybrid", "tier": "hybrid", "disabledCount": 0 }),
        );
    }
    let _ = app.emit("chrome-prefs", &next);
    Ok(next)
}

fn load_shortcuts_plugin_sides() -> HashMap<String, String> {
    let Ok(Some(v)) = crate::db::with_conn(|c| crate::db::shortcuts_get(c)) else {
        return HashMap::new();
    };
    let Some(obj) = v.as_object() else {
        return HashMap::new();
    };
    let sides = obj
        .get("pluginSides")
        .or_else(|| obj.get("plugin_sides"))
        .and_then(|x| x.as_object());
    let Some(sides) = sides else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    for (pid, side) in sides {
        let id = pid.trim();
        if id.is_empty() {
            continue;
        }
        if side.as_str().unwrap_or("").eq_ignore_ascii_case("right") {
            out.insert(id.to_string(), "right".into());
        }
    }
    out
}

fn write_shortcuts_plugin_sides(app: &AppHandle, sides: &HashMap<String, String>) -> Result<(), String> {
    let mut root = match crate::db::with_conn(|c| crate::db::shortcuts_get(c))? {
        Some(Value::Object(map)) => Value::Object(map),
        _ => json!({}),
    };
    if let Some(obj) = root.as_object_mut() {
        let mut cleaned = serde_json::Map::new();
        for (id, side) in sides {
            if side.eq_ignore_ascii_case("right") {
                cleaned.insert(id.clone(), Value::String("right".into()));
            }
        }
        obj.insert("pluginSides".into(), Value::Object(cleaned));
    }
    crate::db::with_conn(|c| crate::db::shortcuts_set(c, &root))?;
    let _ = app.emit("shortcuts-prefs", &root);
    Ok(())
}

/// When re-enabling a plugin: if it was on the right but no right wing (tray tier), move to left.
pub(crate) fn resolve_shortcuts_side_on_enable(app: &AppHandle, plugin_id: &str) -> bool {
    let id = plugin_id.trim();
    if id.is_empty() || has_right_shortcuts_wing(&load()) {
        return false;
    }
    let mut sides = load_shortcuts_plugin_sides();
    if !sides.contains_key(id) {
        return false;
    }
    sides.remove(id);
    let _ = write_shortcuts_plugin_sides(app, &sides);
    true
}

/// Leaving dual shortcuts: disable every plugin assigned to the right bar (keep side=right).
pub(crate) fn disable_right_side_shortcut_plugins(app: &AppHandle) -> usize {
    let right_ids: Vec<String> = load_shortcuts_plugin_sides()
        .into_iter()
        .filter(|(_, side)| side.eq_ignore_ascii_case("right"))
        .map(|(id, _)| id)
        .collect();
    if right_ids.is_empty() {
        return 0;
    }
    let Some(pins) = app.try_state::<crate::plugin_hub::ShortcutsPinStore>() else {
        return 0;
    };
    let mut n = 0;
    for id in right_ids {
        match crate::plugin_install::apply_plugin_enabled(app, &id, false, pins.inner()) {
            Ok(true) => n += 1,
            Ok(false) => {}
            Err(err) => {
                eprintln!("[chrome] disable right shortcuts plugin {id}: {err}");
            }
        }
    }
    n
}

/// Boot: start only the producers the user left enabled.
pub fn start_enabled_producers(app: &AppHandle) {
    let prefs = load();
    if prefs.show_ime {
        ensure_ime_started(app);
    }
    if prefs.show_wifi {
        ensure_wifi_started(app);
    }
    if prefs.show_tray {
        ensure_tray_started(app);
    }
}
