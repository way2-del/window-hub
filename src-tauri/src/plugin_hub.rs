//! CapGate-backed hub commands: storage, shortcuts pins/badge, capability checks.

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

fn write_pins_db(plugin_id: &str, pins: &[ShortcutPin]) -> Result<(), String> {
    let val = serde_json::to_value(pins).map_err(|e| e.to_string())?;
    crate::db::with_conn(|c| crate::db::pins_set(c, plugin_id, &val))
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

/// Require plugin installed, enabled, and declaring `slots[slot]`.
pub fn assert_plugin_slot(plugin_id: &str, slot: &str) -> Result<(), String> {
    let p = find_installed_plugin(plugin_id).ok_or_else(|| "plugin not installed".to_string())?;
    if !p.enabled {
        return Err("plugin disabled".into());
    }
    let has = p
        .manifest
        .get("slots")
        .and_then(|s| s.as_object())
        .map(|s| s.contains_key(slot))
        .unwrap_or(false);
    if !has {
        return Err(format!("plugin {plugin_id} missing slot \"{slot}\""));
    }
    Ok(())
}

/// `permissions.network` allowlist from installed plugin manifest.
pub fn network_allowlist(plugin_id: &str) -> Result<Vec<String>, String> {
    let p = find_installed_plugin(plugin_id).ok_or_else(|| "plugin not installed".to_string())?;
    let list = p
        .manifest
        .get("permissions")
        .and_then(|x| x.get("network"))
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Ok(list)
}

/// Match URL against allowlist entries (full URL prefix, host, or `*.domain.tld`).
pub fn url_allowed_by_network_list(url: &str, allow: &[String]) -> bool {
    let Some((scheme, host)) = parse_http_url_host(url) else {
        return false;
    };
    let origin = format!("{scheme}://{host}");
    let host_l = host.to_ascii_lowercase();
    for entry in allow {
        let e = entry.trim();
        if e.is_empty() {
            continue;
        }
        if e == "*" {
            return true;
        }
        if url.starts_with(e) || origin.eq_ignore_ascii_case(e) {
            return true;
        }
        if let Some(rest) = e.strip_prefix("*.") {
            let rest_l = rest.to_ascii_lowercase();
            if host_l == rest_l || host_l.ends_with(&format!(".{rest_l}")) {
                return true;
            }
            continue;
        }
        if let Some((ps, ph)) = parse_http_url_host(e) {
            if scheme.eq_ignore_ascii_case(ps) && host_l == ph.to_ascii_lowercase() {
                return true;
            }
            continue;
        }
        if host_l == e.to_ascii_lowercase() {
            return true;
        }
    }
    false
}

fn parse_http_url_host(url: &str) -> Option<(&str, &str)> {
    let url = url.trim();
    let (scheme, rest) = if let Some(r) = url.strip_prefix("https://") {
        ("https", r)
    } else if let Some(r) = url.strip_prefix("http://") {
        ("http", r)
    } else {
        return None;
    };
    let host_port = rest.split('/').next().unwrap_or("");
    let host = host_port.split('@').next_back()?.split(':').next()?;
    if host.is_empty() {
        return None;
    }
    Some((scheme, host))
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
    crate::db::with_conn(|c| crate::db::plugin_get(c, &plugin_id, &key))
}

#[tauri::command]
pub fn hub_storage_set(
    app: AppHandle,
    plugin_id: String,
    key: String,
    value: Value,
) -> Result<(), String> {
    assert_capability(&plugin_id, "storage")?;
    crate::db::with_conn(|c| crate::db::plugin_set(c, &plugin_id, &key, &value))?;
    let _ = app.emit(
        "plugin-storage-changed",
        serde_json::json!({
            "pluginId": plugin_id,
            "key": key,
            "value": value,
            "removed": false,
        }),
    );
    Ok(())
}

#[tauri::command]
pub fn hub_storage_remove(app: AppHandle, plugin_id: String, key: String) -> Result<bool, String> {
    assert_capability(&plugin_id, "storage")?;
    let removed = crate::db::with_conn(|c| crate::db::plugin_remove(c, &plugin_id, &key))?;
    if removed {
        let _ = app.emit(
            "plugin-storage-changed",
            serde_json::json!({
                "pluginId": plugin_id,
                "key": key,
                "value": Value::Null,
                "removed": true,
            }),
        );
    }
    Ok(removed)
}

#[tauri::command]
pub fn hub_storage_list_keys(plugin_id: String) -> Result<Vec<String>, String> {
    assert_capability(&plugin_id, "storage")?;
    crate::db::with_conn(|c| crate::db::plugin_list_keys(c, &plugin_id))
}

const SETTINGS_KEY: &str = crate::db::KEY_SETTINGS;

fn settings_fields(plugin_id: &str) -> Result<Vec<Value>, String> {
    let record =
        find_installed_plugin(plugin_id).ok_or_else(|| "plugin not installed".to_string())?;
    let arr = record
        .manifest
        .get("settings")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(arr)
}

fn option_values(field: &Value) -> Vec<Value> {
    field
        .get("options")
        .and_then(|o| o.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|o| o.get("value").cloned())
                .collect()
        })
        .unwrap_or_default()
}

fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            x.as_f64().unwrap_or(f64::NAN) == y.as_f64().unwrap_or(f64::NAN)
        }
        _ => a == b,
    }
}

fn validate_setting_value(field: &Value, value: &Value) -> Result<(), String> {
    let key = field
        .get("key")
        .and_then(|k| k.as_str())
        .unwrap_or("?");
    let ty = field
        .get("type")
        .and_then(|t| t.as_str())
        .ok_or_else(|| format!("settings field {key} missing type"))?;
    match ty {
        "boolean" => {
            if !value.is_boolean() {
                return Err(format!("settings.{key} must be boolean"));
            }
        }
        "string" => {
            let s = value
                .as_str()
                .ok_or_else(|| format!("settings.{key} must be string"))?;
            if let Some(max) = field.get("maxLength").and_then(|m| m.as_u64()) {
                if s.len() as u64 > max {
                    return Err(format!("settings.{key} exceeds maxLength"));
                }
            }
        }
        "number" => {
            let n = value
                .as_f64()
                .ok_or_else(|| format!("settings.{key} must be number"))?;
            if let Some(min) = field.get("min").and_then(|m| m.as_f64()) {
                if n < min {
                    return Err(format!("settings.{key} below min"));
                }
            }
            if let Some(max) = field.get("max").and_then(|m| m.as_f64()) {
                if n > max {
                    return Err(format!("settings.{key} above max"));
                }
            }
        }
        "select" | "radio" => {
            let opts = option_values(field);
            if opts.is_empty() {
                return Err(format!("settings.{key} has no options"));
            }
            if !opts.iter().any(|o| values_equal(o, value)) {
                return Err(format!("settings.{key} value not in options"));
            }
        }
        "multiSelect" => {
            let arr = value
                .as_array()
                .ok_or_else(|| format!("settings.{key} must be array"))?;
            let opts = option_values(field);
            for item in arr {
                if !opts.iter().any(|o| values_equal(o, item)) {
                    return Err(format!("settings.{key} contains invalid option"));
                }
            }
        }
        other => return Err(format!("unsupported settings type: {other}")),
    }
    Ok(())
}

fn merge_settings_with_defaults(fields: &[Value], stored: &Value) -> Value {
    let mut out = serde_json::Map::new();
    let stored_obj = stored.as_object();
    for field in fields {
        let Some(key) = field.get("key").and_then(|k| k.as_str()) else {
            continue;
        };
        if let Some(obj) = stored_obj {
            if let Some(v) = obj.get(key) {
                if validate_setting_value(field, v).is_ok() {
                    out.insert(key.to_string(), v.clone());
                    continue;
                }
            }
        }
        if let Some(def) = field.get("default") {
            out.insert(key.to_string(), def.clone());
        }
    }
    Value::Object(out)
}

fn read_settings_raw(plugin_id: &str) -> Result<Value, String> {
    let found = crate::db::with_conn(|c| {
        crate::db::plugin_get_system(c, plugin_id, SETTINGS_KEY)
    })?;
    Ok(found.unwrap_or_else(|| Value::Object(serde_json::Map::new())))
}

/// Resolved settings (defaults + stored). Requires `storage`.
#[tauri::command]
pub fn hub_settings_get_all(plugin_id: String) -> Result<Value, String> {
    assert_capability(&plugin_id, "storage")?;
    let fields = settings_fields(&plugin_id)?;
    let stored = read_settings_raw(&plugin_id)?;
    let merged = merge_settings_with_defaults(&fields, &stored);
    // Persist defaults fill so Host & plugins share one snapshot
    if merged != stored {
        let _ = crate::db::with_conn(|c| {
            crate::db::plugin_set_system(c, &plugin_id, SETTINGS_KEY, &merged)
        });
    }
    Ok(merged)
}

#[tauri::command]
pub fn hub_settings_get(plugin_id: String, key: String) -> Result<Value, String> {
    let all = hub_settings_get_all(plugin_id)?;
    Ok(all.get(&key).cloned().unwrap_or(Value::Null))
}

#[tauri::command]
pub fn hub_settings_set(
    app: AppHandle,
    plugin_id: String,
    key: String,
    value: Value,
) -> Result<Value, String> {
    assert_capability(&plugin_id, "storage")?;
    let fields = settings_fields(&plugin_id)?;
    let field = fields
        .iter()
        .find(|f| f.get("key").and_then(|k| k.as_str()) == Some(key.as_str()))
        .ok_or_else(|| format!("unknown settings key: {key}"))?
        .clone();
    validate_setting_value(&field, &value)?;
    let mut all = hub_settings_get_all(plugin_id.clone())?;
    if let Some(obj) = all.as_object_mut() {
        obj.insert(key, value);
    }
    crate::db::with_conn(|c| crate::db::plugin_set_system(c, &plugin_id, SETTINGS_KEY, &all))?;
    let _ = app.emit(
        "plugin-settings-changed",
        serde_json::json!({ "pluginId": plugin_id, "settings": all }),
    );
    Ok(all)
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
    let body = format!(
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
      subscribe: (cb) => {{
        const listen = window.__TAURI__ && window.__TAURI__.event && window.__TAURI__.event.listen;
        if (!listen || typeof cb !== "function") return () => {{}};
        let un = () => {{}};
        listen("plugin-storage-changed", (ev) => {{
          const p = ev && ev.payload;
          if (!p || p.pluginId !== PLUGIN_ID) return;
          try {{
            cb({{
              key: p.key,
              value: p.removed ? null : p.value,
              removed: !!p.removed
            }});
          }} catch (_) {{}}
        }}).then((fn) => {{ un = fn; }});
        return () => un();
      }},
    }},
    settings: {{
      getAll: () => invoke("hub_settings_get_all", withPlugin()),
      get: (key) => invoke("hub_settings_get", withPlugin({{ key }})),
      set: (key, value) => invoke("hub_settings_set", withPlugin({{ key, value }})),
      subscribe: (cb) => {{
        const listen = window.__TAURI__ && window.__TAURI__.event && window.__TAURI__.event.listen;
        if (!listen) return () => {{}};
        let un = () => {{}};
        listen("plugin-settings-changed", (ev) => {{
          const p = ev && ev.payload;
          if (!p || p.pluginId !== PLUGIN_ID) return;
          try {{ cb(p.settings || {{}}); }} catch (_) {{}}
        }}).then((fn) => {{ un = fn; }});
        window.hub.settings.getAll().then(cb).catch(() => undefined);
        return () => un();
      }},
    }},
    shortcuts: {{
      setPins: (pins) => invoke("hub_shortcuts_set_pins", withPlugin({{ pins }})),
      clearPins: () => invoke("hub_shortcuts_clear_pins", withPlugin()),
      setBadge: (badge) => invoke("hub_shortcuts_set_badge", withPlugin({{ badge }})),
    }},
    staging: {{
      list: () => invoke("hub_staging_list", withPlugin()),
      summary: () => invoke("hub_staging_summary", withPlugin()),
      addText: (text) => invoke("hub_staging_add_text", withPlugin({{ text }})),
      addPaths: (paths) => invoke("hub_staging_add_paths", withPlugin({{ paths }})),
      addImageBytes: (label, bytes, ext) =>
        invoke("hub_staging_add_image_bytes", withPlugin({{ label, bytes, ext }})),
      remove: (idOrIds) => {{
        const ids = Array.isArray(idOrIds) ? idOrIds : [idOrIds];
        if (ids.length === 0) return Promise.resolve();
        if (ids.length === 1) {{
          return invoke("hub_staging_remove", withPlugin({{ id: ids[0] }}));
        }}
        return invoke("hub_staging_remove_many", withPlugin({{ ids }}));
      }},
      clear: () => invoke("hub_staging_clear", withPlugin()),
      copy: (id) => invoke("hub_staging_copy", withPlugin({{ id }})),
      copyFiles: (idOrIds) => {{
        const ids = Array.isArray(idOrIds) ? idOrIds : [idOrIds];
        return invoke("hub_staging_copy_files", withPlugin({{ ids }}));
      }},
      copyPaths: (ids) => invoke("hub_staging_copy_paths", withPlugin({{ ids }})),
      copyAllPaths: () => invoke("hub_staging_copy_all_paths", withPlugin()),
      thumb: (id) => invoke("hub_staging_thumb", withPlugin({{ id }})),
      reveal: (id) => invoke("hub_staging_reveal", withPlugin({{ id }})),
      open: (id) => invoke("hub_staging_open", withPlugin({{ id }})),
      startDrag: (ids) => invoke("hub_staging_start_drag", withPlugin({{ ids }})),
      pickFiles: () => invoke("hub_staging_pick_files", withPlugin()),
      pickFolders: () => invoke("hub_staging_pick_folders", withPlugin()),
      subscribe: (cb) => {{
        const listenFn =
          window.__TAURI__ && window.__TAURI__.event && window.__TAURI__.event.listen;
        if (!listenFn) {{
          window.hub.staging.summary().then(cb).catch(() => undefined);
          return () => {{}};
        }}
        let un = () => {{}};
        listenFn("staging-changed", (ev) => {{
          const p = ev && ev.payload;
          if (!p) return;
          const pid = p.pluginId || p.plugin_id;
          if (pid && pid !== PLUGIN_ID) return;
          const summary = p.summary || {{
            files: p.files || 0,
            texts: p.texts || 0,
            images: p.images || 0,
            total: p.total || 0,
          }};
          try {{ cb(summary); }} catch (_) {{}}
        }}).then((fn) => {{ un = fn; }});
        window.hub.staging.summary().then(cb).catch(() => undefined);
        return () => un();
      }},
    }},
    island: {{
      setBar: (opts) =>
        invoke(
          "hub_island_set_bar",
          withPlugin({{
            text: (opts && opts.text) || "",
            title: opts && opts.title,
            image: opts && opts.image,
            mirror: !!(opts && opts.mirror),
          }}),
        ),
      clearBar: () => invoke("hub_island_clear_bar", withPlugin()),
    }},
    media: {{
      neteaseNowPlaying: () => invoke("hub_netease_now_playing", withPlugin()),
      transport: (action) =>
        invoke("hub_media_transport", withPlugin({{ action: String(action || "") }})),
      openNetease: () => invoke("hub_media_open_netease", withPlugin()),
    }},
    fetch: (url, opts) =>
      invoke(
        "hub_fetch",
        withPlugin({{
          url: url,
          opts: opts || null,
        }}),
      ),
    panel: {{
      close: () => invoke("close_plugin_popup"),
      openSession: () => invoke("hub_panel_open_session", withPlugin()),
      closeSession: () => invoke("hub_panel_close_session", {{}}),
    }},
    popup: {{
      close: () => invoke("close_plugin_popup"),
    }},
    applyEffect: (material) =>
      material
        ? invoke("apply_window_effect", {{ material }})
        : invoke("apply_window_effect", {{}}),
  }};

  const notifyFn = (opts) =>
    invoke(
      "hub_notify",
      withPlugin({{
        opts: {{
          title: (opts && opts.title) || "",
          body: opts && opts.body,
          iconPng: opts && opts.iconPng,
          urgency: opts && opts.urgency,
          ttlMs: opts && opts.ttlMs,
          actions: opts && opts.actions,
          data: opts && opts.data,
        }},
      }}),
    );
  notifyFn.onAction = (cb) => {{
    const listen = window.__TAURI__ && window.__TAURI__.event && window.__TAURI__.event.listen;
    if (!listen) return () => {{}};
    let un = () => {{}};
    listen("island-notify-action", (ev) => {{
      const p = ev && ev.payload;
      if (!p || p.pluginId !== PLUGIN_ID) return;
      try {{
        cb({{
          notifyId: p.notifyId,
          actionId: p.actionId,
          data: p.data,
        }});
      }} catch (_) {{}}
    }}).then((fn) => {{ un = fn; }});
    return () => un();
  }};
  window.hub.notify = notifyFn;

  document.addEventListener("keydown", function (e) {{
    if (e.key === "Escape") {{
      try {{ window.hub.popup.close(); }} catch (_) {{}}
    }}
  }});
}})();
"#
    );
    #[cfg(windows)]
    {
        crate::win32::blur_glass::prepend_glass_compat_boot(&body)
    }
    #[cfg(not(windows))]
    {
        body
    }
}
