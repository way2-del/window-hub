//! CapGate-backed hub commands: storage, shortcuts pins, capability checks.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

use crate::plugin_install::{find_installed_plugin, list_installed_plugins_sync};
use crate::win32::enum_windows::{focus_window, parse_window_id, WindowInfo};
use crate::windows_service::WindowsService;

#[derive(Clone, Default)]
pub struct ShortcutsPinStore {
    inner: Arc<Mutex<HashMap<String, Vec<ShortcutPin>>>>,
}

impl ShortcutsPinStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load_all_from_db(&self) {
        let mut map = self.inner.lock();
        map.clear();
        let Ok(all) = crate::db::with_conn(|c| crate::db::pins_list_all(c)) else {
            return;
        };
        let enabled: HashMap<String, bool> = list_installed_plugins_sync()
            .into_iter()
            .map(|p| (p.id, p.enabled))
            .collect();
        for (plugin_id, val) in all {
            if !enabled.get(&plugin_id).copied().unwrap_or(false) {
                continue;
            }
            if let Ok(pins) = serde_json::from_value::<Vec<ShortcutPin>>(val) {
                map.insert(plugin_id, pins);
            }
        }
    }

    /// Reload pins for one enabled plugin from DB (e.g. after re-enable).
    pub fn reload_plugin(&self, plugin_id: &str) {
        let Ok(Some(val)) = crate::db::with_conn(|c| crate::db::pins_get(c, plugin_id)) else {
            self.inner.lock().remove(plugin_id);
            return;
        };
        if let Ok(pins) = serde_json::from_value::<Vec<ShortcutPin>>(val) {
            self.inner.lock().insert(plugin_id.to_string(), pins);
        }
    }

    pub fn inner_remove(&self, plugin_id: &str) {
        self.inner.lock().remove(plugin_id);
    }

    pub fn all_flat(&self) -> Vec<ShortcutPinView> {
        let map = self.inner.lock();
        let mut out = Vec::new();
        for (plugin_id, pins) in map.iter() {
            for pin in pins {
                out.push(ShortcutPinView {
                    plugin_id: plugin_id.clone(),
                    id: pin.id.clone(),
                    label: pin.label.clone(),
                    badge: pin.badge.clone(),
                    width: pin.width,
                    action: pin.action.clone(),
                    window_id: pin.window_id.clone(),
                    prefer_group_id: pin.prefer_group_id.clone(),
                });
            }
        }
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutPin {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub badge: Option<Value>,
    #[serde(default = "default_pin_width")]
    pub width: f64,
    /// `popup.open` | `focus.window` | `noop`
    pub action: String,
    #[serde(default)]
    pub window_id: Option<String>,
    #[serde(default)]
    pub prefer_group_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutPinView {
    pub plugin_id: String,
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub badge: Option<Value>,
    pub width: f64,
    pub action: String,
    #[serde(default)]
    pub window_id: Option<String>,
    #[serde(default)]
    pub prefer_group_id: Option<String>,
}

fn default_pin_width() -> f64 {
    96.0
}

pub fn assert_capability(plugin_id: &str, cap: &str) -> Result<(), String> {
    let p = find_installed_plugin(plugin_id).ok_or_else(|| "plugin not installed".to_string())?;
    if !p.enabled {
        return Err("plugin disabled".into());
    }
    if !p.capabilities.iter().any(|c| c == cap) {
        return Err(format!("plugin {plugin_id} missing capability \"{cap}\""));
    }
    Ok(())
}

fn write_pins_db(plugin_id: &str, pins: &[ShortcutPin]) -> Result<(), String> {
    let val = serde_json::to_value(pins).map_err(|e| e.to_string())?;
    crate::db::with_conn(|c| crate::db::pins_set(c, plugin_id, &val))
}

fn migrate_legacy_window_groups(plugin_id: &str) -> Option<Value> {
    if plugin_id != "com.window-hub.window-groups" {
        return None;
    }
    let appdata = std::env::var_os("APPDATA")?;
    let mut path = PathBuf::from(appdata);
    path.push("window-hub");
    path.push("window-groups.json");
    let text = fs::read_to_string(&path).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let _ = crate::db::with_conn(|c| crate::db::plugin_set(c, plugin_id, "store", &value));
    let bak = PathBuf::from(format!("{}.bak", path.display()));
    let _ = fs::rename(&path, bak);
    Some(value)
}

#[tauri::command]
pub fn hub_windows_list(
    app: AppHandle,
    plugin_id: String,
    svc: State<'_, WindowsService>,
) -> Result<Vec<WindowInfo>, String> {
    assert_capability(&plugin_id, "windows.read")?;
    let list = svc.list();
    if list.is_empty() {
        return Ok(svc.refresh_now(&app));
    }
    Ok(list)
}

#[tauri::command]
pub fn hub_windows_get(
    plugin_id: String,
    id: String,
    svc: State<'_, WindowsService>,
) -> Result<Option<WindowInfo>, String> {
    assert_capability(&plugin_id, "windows.read")?;
    svc.get(&id)
}

#[tauri::command]
pub fn hub_windows_focus(plugin_id: String, id: String) -> Result<(), String> {
    assert_capability(&plugin_id, "windows.focus")?;
    let hwnd = parse_window_id(&id)?;
    focus_window(hwnd)
}

#[tauri::command]
pub fn hub_storage_get(plugin_id: String, key: String) -> Result<Option<Value>, String> {
    assert_capability(&plugin_id, "storage")?;
    let found = crate::db::with_conn(|c| crate::db::plugin_get(c, &plugin_id, &key))?;
    if found.is_some() {
        return Ok(found);
    }
    if key == "store" {
        if let Some(migrated) = migrate_legacy_window_groups(&plugin_id) {
            return Ok(Some(migrated));
        }
    }
    Ok(None)
}

#[tauri::command]
pub fn hub_storage_set(plugin_id: String, key: String, value: Value) -> Result<(), String> {
    assert_capability(&plugin_id, "storage")?;
    crate::db::with_conn(|c| crate::db::plugin_set(c, &plugin_id, &key, &value))
}

#[tauri::command]
pub fn hub_storage_remove(plugin_id: String, key: String) -> Result<bool, String> {
    assert_capability(&plugin_id, "storage")?;
    crate::db::with_conn(|c| crate::db::plugin_remove(c, &plugin_id, &key))
}

#[tauri::command]
pub fn hub_storage_list_keys(plugin_id: String) -> Result<Vec<String>, String> {
    assert_capability(&plugin_id, "storage")?;
    crate::db::with_conn(|c| crate::db::plugin_list_keys(c, &plugin_id))
}

#[tauri::command]
pub fn hub_shortcuts_set_pins(
    app: AppHandle,
    plugin_id: String,
    pins: Vec<ShortcutPin>,
    store: State<'_, ShortcutsPinStore>,
) -> Result<(), String> {
    assert_capability(&plugin_id, "shortcuts")?;
    let mut cleaned = Vec::with_capacity(pins.len());
    for mut p in pins {
        p.width = p.width.clamp(56.0, 220.0);
        if p.action.is_empty() {
            p.action = "popup.open".into();
        }
        cleaned.push(p);
    }
    write_pins_db(&plugin_id, &cleaned)?;
    store.inner.lock().insert(plugin_id.clone(), cleaned);
    let _ = app.emit("shortcuts-pins-changed", store.all_flat());
    Ok(())
}

#[tauri::command]
pub fn hub_shortcuts_clear_pins(
    app: AppHandle,
    plugin_id: String,
    store: State<'_, ShortcutsPinStore>,
) -> Result<(), String> {
    assert_capability(&plugin_id, "shortcuts")?;
    crate::db::with_conn(|c| crate::db::pins_clear(c, &plugin_id))?;
    store.inner.lock().remove(&plugin_id);
    let _ = app.emit("shortcuts-pins-changed", store.all_flat());
    Ok(())
}

#[tauri::command]
pub fn hub_shortcuts_list_pins(store: State<'_, ShortcutsPinStore>) -> Vec<ShortcutPinView> {
    store.all_flat()
}

#[tauri::command]
pub fn hub_shortcuts_set_badge(
    app: AppHandle,
    plugin_id: String,
    badge: Option<Value>,
) -> Result<(), String> {
    assert_capability(&plugin_id, "shortcuts")?;
    let _ = app.emit(
        "shortcuts-badge-changed",
        serde_json::json!({ "pluginId": plugin_id, "badge": badge }),
    );
    Ok(())
}

#[tauri::command]
pub fn hub_shortcuts_resize_pin(
    app: AppHandle,
    plugin_id: String,
    pin_id: String,
    width: f64,
    store: State<'_, ShortcutsPinStore>,
) -> Result<(), String> {
    assert_capability(&plugin_id, "shortcuts")?;
    let width = width.clamp(56.0, 220.0);
    {
        let mut map = store.inner.lock();
        let Some(pins) = map.get_mut(&plugin_id) else {
            return Err("no pins for plugin".into());
        };
        let Some(pin) = pins.iter_mut().find(|p| p.id == pin_id) else {
            return Err("pin not found".into());
        };
        pin.width = width;
        write_pins_db(&plugin_id, pins)?;
    }
    let _ = app.emit("shortcuts-pins-changed", store.all_flat());
    Ok(())
}

#[tauri::command]
pub fn hub_plugin_read_text(plugin_id: String, relative_path: String) -> Result<String, String> {
    let record =
        find_installed_plugin(&plugin_id).ok_or_else(|| "plugin not installed".to_string())?;
    if !record.enabled {
        return Err("plugin disabled".into());
    }
    // allow popup assets without requiring a specific capability beyond install
    let rel = relative_path.replace('\\', "/");
    if rel.is_empty()
        || rel.contains("..")
        || rel.starts_with('/')
        || rel.chars().any(|c| c == '\0')
    {
        return Err("invalid relative path".into());
    }
    let path = PathBuf::from(&record.path).join(&rel);
    let canon_root = PathBuf::from(&record.path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&record.path));
    let canon_file = path
        .canonicalize()
        .map_err(|e| format!("asset not found: {e}"))?;
    if !canon_file.starts_with(&canon_root) {
        return Err("path escapes plugin root".into());
    }
    fs::read_to_string(&canon_file).map_err(|e| format!("read asset: {e}"))
}

/// Build `window.hub` injection for plugin popups (trusted pluginId).
pub fn hub_init_script(plugin_id: &str) -> String {
    format!(
        r#"
(function () {{
  const PLUGIN_ID = {plugin_id:?};
  window.__WH_PLUGIN_ID__ = PLUGIN_ID;
  window.__WH_IS_PLUGIN_POPUP__ = true;

  function invoke(cmd, args) {{
    const core = window.__TAURI__ && window.__TAURI__.core;
    if (!core || !core.invoke) return Promise.reject(new Error("Tauri core missing"));
    return core.invoke(cmd, args || {{}});
  }}

  function withPlugin(args) {{
    return Object.assign({{ pluginId: PLUGIN_ID }}, args || {{}});
  }}

  window.hub = {{
    pluginId: PLUGIN_ID,
    windows: {{
      list: () => invoke("hub_windows_list", withPlugin()),
      get: (id) => invoke("hub_windows_get", withPlugin({{ id }})),
      focus: (id) => invoke("hub_windows_focus", withPlugin({{ id }})),
      subscribe: (cb) => {{
        const listen = window.__TAURI__ && window.__TAURI__.event && window.__TAURI__.event.listen;
        if (!listen) {{
          const t = setInterval(() => {{
            window.hub.windows.list().then(cb).catch(() => undefined);
          }}, 1200);
          window.hub.windows.list().then(cb).catch(() => undefined);
          return () => clearInterval(t);
        }}
        let un = () => {{}};
        listen("hub-windows-changed", (ev) => {{
          const wins = ev && ev.payload && ev.payload.windows;
          if (wins) cb(wins);
        }}).then((fn) => {{ un = fn; }});
        window.hub.windows.list().then(cb).catch(() => undefined);
        return () => un();
      }},
    }},
    storage: {{
      get: (key) => invoke("hub_storage_get", withPlugin({{ key }})),
      set: (key, value) => invoke("hub_storage_set", withPlugin({{ key, value }})),
      remove: (key) => invoke("hub_storage_remove", withPlugin({{ key }})),
      listKeys: () => invoke("hub_storage_list_keys", withPlugin()),
    }},
    shortcuts: {{
      setPins: (pins) => invoke("hub_shortcuts_set_pins", withPlugin({{ pins }})),
      clearPins: () => invoke("hub_shortcuts_clear_pins", withPlugin()),
      setBadge: (badge) => invoke("hub_shortcuts_set_badge", withPlugin({{ badge }})),
    }},
    popup: {{
      close: () => invoke("close_plugin_popup"),
    }},
    applyEffect: (material) =>
      material
        ? invoke("apply_window_effect", {{ material }})
        : invoke("apply_window_effect", {{}}),
  }};

  document.addEventListener("keydown", function (e) {{
    if (e.key === "Escape") {{
      try {{ window.hub.popup.close(); }} catch (_) {{}}
    }}
  }});
}})();
"#
    )
}
