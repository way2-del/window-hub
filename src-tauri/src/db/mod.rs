//! SQLite-backed local storage for host prefs, plugin KV, staging index, and shortcuts display cache.

pub mod admin;
mod host;
mod migrate;

use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub use host::{
    ambient_get, ambient_set, dock_get, dock_set, island_get, island_set, launchers_list,
    launchers_replace_all, material_get, material_set, meta_get, meta_set, shortcuts_get,
    shortcuts_set, tray_get, tray_set, IslandPrefsRow, LauncherRow,
};
pub use migrate::migrate_legacy_files;

/// Official weather plugin id (settings / storage live in `plugin_kv`).
pub const WEATHER_PLUGIN_ID: &str = "com.window-hub.weather";

const SCHEMA_VERSION: i32 = 9;
const PLUGIN_KEY_MAX_BYTES: usize = 512 * 1024;
const PLUGIN_TOTAL_MAX_BYTES: usize = 8 * 1024 * 1024;
const SYSTEM_KEY_MAX_BYTES: usize = 8 * 1024 * 1024;

/// CapGate-owned keys in `plugin_kv` (hub.storage cannot read/write these).
/// `__shortcuts_pins` = Host 快捷区壳的展示缓存（插件业务 pins 仍在 `hub.storage`，如窗口组 `store.pins`）。
pub const KEY_SHORTCUTS_PINS: &str = "__shortcuts_pins";
pub const KEY_STAGING_ITEMS: &str = "__staging_items";
/// Declarative plugin settings (manifest.settings); accessed via hub.settings.*
pub const KEY_SETTINGS: &str = "__settings";

static DB: OnceLock<DbState> = OnceLock::new();

#[derive(Clone)]
pub struct DbState {
    pub conn: Arc<Mutex<Connection>>,
}

pub fn global() -> Option<&'static DbState> {
    DB.get()
}

pub fn with_conn<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce(&Connection) -> Result<R, String>,
{
    let db = global().ok_or_else(|| "database not initialized".to_string())?;
    let guard = db.conn.lock();
    f(&guard)
}

pub fn app_data_root() -> Result<PathBuf, String> {
    let appdata = std::env::var_os("APPDATA").ok_or_else(|| "APPDATA missing".to_string())?;
    let mut dir = PathBuf::from(appdata);
    dir.push("window-hub");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

pub fn db_path() -> Result<PathBuf, String> {
    Ok(app_data_root()?.join("window-hub.db"))
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn init() -> Result<DbState, String> {
    let path = db_path()?;
    let conn = Connection::open(&path).map_err(|e| e.to_string())?;
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
        .map_err(|e| e.to_string())?;
    migrate_schema(&conn)?;
    let state = DbState {
        conn: Arc::new(Mutex::new(conn)),
    };
    {
        let guard = state.conn.lock();
        migrate_legacy_files(&guard)?;
    }
    let _ = DB.set(state.clone());
    Ok(state)
}

fn migrate_schema(conn: &Connection) -> Result<(), String> {
    let ver: i32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(0);

    if ver < 1 {
        // Fresh: only plugin_kv (+ host tables). Pins/staging live as reserved keys.
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS plugin_kv (
              plugin_id TEXT NOT NULL,
              key TEXT NOT NULL,
              value_json TEXT NOT NULL,
              updated_at INTEGER NOT NULL,
              PRIMARY KEY (plugin_id, key)
            );
            "#,
        )
        .map_err(|e| e.to_string())?;
        host::create_host_tables(conn)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    if ver < 2 {
        host::create_host_tables(conn)?;
        host::migrate_from_host_kv(conn)?;
        // leave user_version bump to end of chain
    }

    if ver < 3 {
        // Ensure plugin_kv exists (v1/v2 already have it)
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS plugin_kv (
              plugin_id TEXT NOT NULL,
              key TEXT NOT NULL,
              value_json TEXT NOT NULL,
              updated_at INTEGER NOT NULL,
              PRIMARY KEY (plugin_id, key)
            );
            "#,
        )
        .map_err(|e| e.to_string())?;
        migrate_plugin_side_tables_into_kv(conn)?;
    }

    if ver < 4 {
        drop_prefs_island_staging_panel_w(conn)?;
    }

    if ver < 5 {
        conn.execute_batch("DROP TABLE IF EXISTS todo_items;")
            .map_err(|e| e.to_string())?;
    }

    if ver < 6 {
        add_prefs_island_bar_resident(conn)?;
    }

    if ver < 7 {
        migrate_weather_host_tables_into_plugin_kv(conn)?;
    }

    if ver < 8 {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS prefs_dock (
              id INTEGER PRIMARY KEY CHECK (id = 1),
              data_json TEXT NOT NULL,
              updated_at INTEGER NOT NULL
            );
            "#,
        )
        .map_err(|e| e.to_string())?;
    }

    if ver < 9 {
        add_prefs_island_volume_preview(conn)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|e| e.to_string())?;
    }

    // Additive host tables for installs already past schema bumps.
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS prefs_shortcuts (
          id INTEGER PRIMARY KEY CHECK (id = 1),
          data_json TEXT NOT NULL,
          updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS prefs_dock (
          id INTEGER PRIMARY KEY CHECK (id = 1),
          data_json TEXT NOT NULL,
          updated_at INTEGER NOT NULL
        );
        "#,
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Move Host `weather_api` / `weather_cache` → weather plugin `plugin_kv`, then DROP tables.
fn migrate_weather_host_tables_into_plugin_kv(conn: &Connection) -> Result<(), String> {
    let has_api: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='weather_api'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has_api > 0 {
        let row = conn
            .query_row(
                "SELECT api_id, api_key FROM weather_api WHERE id = 1",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some((api_id, api_key)) = row {
            let id = api_id.trim();
            let key = api_key.trim();
            if !id.is_empty()
                && !key.is_empty()
                && !(id == "88888888" && key == "88888888")
            {
                let existing = plugin_get_system(conn, WEATHER_PLUGIN_ID, KEY_SETTINGS)?;
                let mut map = match existing {
                    Some(Value::Object(m)) => m,
                    _ => serde_json::Map::new(),
                };
                let has_custom = map
                    .get("apiId")
                    .and_then(|v| v.as_str())
                    .map(|s| !s.is_empty() && s != "88888888")
                    .unwrap_or(false);
                if !has_custom {
                    map.insert("apiId".into(), Value::String(id.to_string()));
                    map.insert("apiKey".into(), Value::String(key.to_string()));
                    plugin_set_system(conn, WEATHER_PLUGIN_ID, KEY_SETTINGS, &Value::Object(map))?;
                }
            }
        }
    }

    let has_cache: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='weather_cache'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has_cache > 0 {
        let cache_raw = conn
            .query_row(
                "SELECT data_json FROM weather_cache WHERE id = 1",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(text) = cache_raw {
            if let Ok(cache) = serde_json::from_str::<Value>(&text) {
                let existing = plugin_get(conn, WEATHER_PLUGIN_ID, "cache")?;
                if existing.is_none() {
                    let payload = if cache.get("info").is_some() {
                        cache
                    } else {
                        serde_json::json!({ "info": cache, "savedAt": now_ms() })
                    };
                    let _ = plugin_set(conn, WEATHER_PLUGIN_ID, "cache", &payload);
                }
            }
        }
    }

    conn.execute_batch(
        r#"
        DROP TABLE IF EXISTS weather_api;
        DROP TABLE IF EXISTS weather_cache;
        "#,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn add_prefs_island_bar_resident(conn: &Connection) -> Result<(), String> {
    let has: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='prefs_island'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has == 0 {
        return Ok(());
    }
    let has_col: i64 = conn
        .prepare("PRAGMA table_info(prefs_island)")
        .and_then(|mut stmt| {
            let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
            let mut n = 0i64;
            for row in rows {
                if row.ok().as_deref() == Some("bar_resident") {
                    n = 1;
                    break;
                }
            }
            Ok(n)
        })
        .unwrap_or(0);
    if has_col != 0 {
        return Ok(());
    }
    conn.execute_batch(
        r#"
        ALTER TABLE prefs_island ADD COLUMN bar_resident TEXT NOT NULL DEFAULT 'com.window-hub.weather';
        "#,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn add_prefs_island_volume_preview(conn: &Connection) -> Result<(), String> {
    let has: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='prefs_island'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has == 0 {
        return Ok(());
    }
    let has_col: i64 = conn
        .prepare("PRAGMA table_info(prefs_island)")
        .and_then(|mut stmt| {
            let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
            let mut n = 0i64;
            for row in rows {
                if row.ok().as_deref() == Some("volume_preview_sound") {
                    n = 1;
                    break;
                }
            }
            Ok(n)
        })
        .unwrap_or(0);
    if has_col != 0 {
        return Ok(());
    }
    conn.execute_batch(
        r#"
        ALTER TABLE prefs_island ADD COLUMN volume_preview_sound INTEGER NOT NULL DEFAULT 1;
        "#,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Remove deprecated prefs_island.staging_panel_w (width lives in plugin __settings).
fn drop_prefs_island_staging_panel_w(conn: &Connection) -> Result<(), String> {
    let has: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='prefs_island'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has == 0 {
        return Ok(());
    }
    let has_col: i64 = conn
        .prepare("PRAGMA table_info(prefs_island)")
        .and_then(|mut stmt| {
            let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
            let mut n = 0i64;
            for row in rows {
                if row.ok().as_deref() == Some("staging_panel_w") {
                    n = 1;
                    break;
                }
            }
            Ok(n)
        })
        .unwrap_or(0);
    if has_col == 0 {
        return Ok(());
    }
    conn.execute_batch(
        r#"
        CREATE TABLE prefs_island__v4 (
          id INTEGER PRIMARY KEY CHECK (id = 1),
          auto_immerse INTEGER NOT NULL,
          immerse_idle_sec INTEGER NOT NULL,
          pull_content TEXT NOT NULL,
          msg_notify INTEGER NOT NULL,
          msg_notify_text TEXT NOT NULL,
          msg_notify_sec INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        );
        INSERT INTO prefs_island__v4 (
          id, auto_immerse, immerse_idle_sec, pull_content,
          msg_notify, msg_notify_text, msg_notify_sec, updated_at
        )
        SELECT
          id, auto_immerse, immerse_idle_sec, pull_content,
          msg_notify, msg_notify_text, msg_notify_sec, updated_at
        FROM prefs_island;
        DROP TABLE prefs_island;
        ALTER TABLE prefs_island__v4 RENAME TO prefs_island;
        "#,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Move `shortcut_pins` / `staging_items` → `plugin_kv` reserved keys, then DROP.
pub(crate) fn migrate_plugin_side_tables_into_kv(conn: &Connection) -> Result<(), String> {
    let has_pins: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='shortcut_pins'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has_pins > 0 {
        let mut stmt = conn
            .prepare("SELECT plugin_id, pins_json FROM shortcut_pins")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (plugin_id, pins_json) = row.map_err(|e| e.to_string())?;
            if let Ok(v) = serde_json::from_str::<Value>(&pins_json) {
                let _ = plugin_set_system(conn, &plugin_id, KEY_SHORTCUTS_PINS, &v);
            }
        }
        conn.execute_batch("DROP TABLE IF EXISTS shortcut_pins;")
            .map_err(|e| e.to_string())?;
    }


    // Pre-scoped global staging_items had no plugin_id — drop without inventing a host id.
    // Live staging index is plugin_kv (__staging_items) per plugin.
    conn.execute_batch("DROP TABLE IF EXISTS staging_items;")
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── plugin_kv (hub.storage) ──────────────────────────────────────────

pub fn validate_plugin_storage_key(key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() || key.len() > 128 {
        return Err("storage key length must be 1..=128".into());
    }
    if key.starts_with("__") {
        return Err("storage keys starting with __ are reserved".into());
    }
    if key.contains("..") || key.contains('/') || key.contains('\\') || key.contains('\0') {
        return Err("storage key must not contain path separators".into());
    }
    let ok = key
        .chars()
        .enumerate()
        .all(|(i, c)| match c {
            'a'..='z' | '0'..='9' => true,
            'A'..='Z' => true,
            '.' | '_' | '-' if i > 0 => true,
            _ => false,
        });
    if !ok {
        return Err("storage key must match [a-zA-Z0-9][a-zA-Z0-9._-]*".into());
    }
    Ok(())
}

fn plugin_total_bytes(conn: &Connection, plugin_id: &str) -> Result<usize, String> {
    let sum: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(LENGTH(value_json)), 0) FROM plugin_kv WHERE plugin_id = ?1",
            params![plugin_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(sum as usize)
}

pub fn plugin_get(conn: &Connection, plugin_id: &str, key: &str) -> Result<Option<Value>, String> {
    validate_plugin_storage_key(key)?;
    conn.query_row(
        "SELECT value_json FROM plugin_kv WHERE plugin_id = ?1 AND key = ?2",
        params![plugin_id, key],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| e.to_string())?
    .map(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
    .transpose()
}

pub fn plugin_set(conn: &Connection, plugin_id: &str, key: &str, value: &Value) -> Result<(), String> {
    validate_plugin_storage_key(key)?;
    let text = serde_json::to_string(value).map_err(|e| e.to_string())?;
    if text.len() > PLUGIN_KEY_MAX_BYTES {
        return Err(format!(
            "storage value too large (max {}KB per key)",
            PLUGIN_KEY_MAX_BYTES / 1024
        ));
    }
    let existing_len: usize = conn
        .query_row(
            "SELECT LENGTH(value_json) FROM plugin_kv WHERE plugin_id = ?1 AND key = ?2",
            params![plugin_id, key],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .map(|n| n as usize)
        .unwrap_or(0);
    let total = plugin_total_bytes(conn, plugin_id)?;
    let next_total = total.saturating_sub(existing_len).saturating_add(text.len());
    if next_total > PLUGIN_TOTAL_MAX_BYTES {
        return Err(format!(
            "plugin storage quota exceeded (max {}MB)",
            PLUGIN_TOTAL_MAX_BYTES / (1024 * 1024)
        ));
    }
    conn.execute(
        "INSERT INTO plugin_kv(plugin_id, key, value_json, updated_at) VALUES(?1,?2,?3,?4)
         ON CONFLICT(plugin_id, key) DO UPDATE SET value_json=excluded.value_json, updated_at=excluded.updated_at",
        params![plugin_id, key, text, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn plugin_remove(conn: &Connection, plugin_id: &str, key: &str) -> Result<bool, String> {
    validate_plugin_storage_key(key)?;
    let n = conn
        .execute(
            "DELETE FROM plugin_kv WHERE plugin_id = ?1 AND key = ?2",
            params![plugin_id, key],
        )
        .map_err(|e| e.to_string())?;
    Ok(n > 0)
}

pub fn plugin_list_keys(conn: &Connection, plugin_id: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT key FROM plugin_kv WHERE plugin_id = ?1 AND key NOT LIKE '__%' ORDER BY key",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![plugin_id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

pub fn plugin_clear_all(conn: &Connection, plugin_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM plugin_kv WHERE plugin_id = ?1",
        params![plugin_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Host/CapGate write for reserved `__*` keys (not exposed via hub.storage).
pub fn plugin_set_system(
    conn: &Connection,
    plugin_id: &str,
    key: &str,
    value: &Value,
) -> Result<(), String> {
    if !key.starts_with("__") {
        return Err("system keys must start with __".into());
    }
    let text = serde_json::to_string(value).map_err(|e| e.to_string())?;
    if text.len() > SYSTEM_KEY_MAX_BYTES {
        return Err(format!(
            "system value too large (max {}MB)",
            SYSTEM_KEY_MAX_BYTES / (1024 * 1024)
        ));
    }
    conn.execute(
        "INSERT INTO plugin_kv(plugin_id, key, value_json, updated_at) VALUES(?1,?2,?3,?4)
         ON CONFLICT(plugin_id, key) DO UPDATE SET value_json=excluded.value_json, updated_at=excluded.updated_at",
        params![plugin_id, key, text, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn plugin_get_system(
    conn: &Connection,
    plugin_id: &str,
    key: &str,
) -> Result<Option<Value>, String> {
    conn.query_row(
        "SELECT value_json FROM plugin_kv WHERE plugin_id = ?1 AND key = ?2",
        params![plugin_id, key],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| e.to_string())?
    .map(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
    .transpose()
}

pub fn plugin_remove_system(conn: &Connection, plugin_id: &str, key: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM plugin_kv WHERE plugin_id = ?1 AND key = ?2",
        params![plugin_id, key],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ── Staging via plugin_kv reserved keys ──────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StagingRow {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub path: String,
    pub created_at: u64,
}

pub fn staging_list(conn: &Connection, plugin_id: &str) -> Result<Vec<StagingRow>, String> {
    match plugin_get_system(conn, plugin_id, KEY_STAGING_ITEMS)? {
        None => Ok(Vec::new()),
        Some(v) => serde_json::from_value(v).map_err(|e| e.to_string()),
    }
}

pub fn staging_replace_all(
    conn: &Connection,
    plugin_id: &str,
    items: &[StagingRow],
) -> Result<(), String> {
    let v = serde_json::to_value(items).map_err(|e| e.to_string())?;
    plugin_set_system(conn, plugin_id, KEY_STAGING_ITEMS, &v)
}

pub fn staging_insert(conn: &Connection, plugin_id: &str, item: &StagingRow) -> Result<(), String> {
    let mut items = staging_list(conn, plugin_id)?;
    items.retain(|x| x.id != item.id);
    items.insert(0, item.clone());
    staging_replace_all(conn, plugin_id, &items)
}

pub fn staging_remove(conn: &Connection, plugin_id: &str, id: &str) -> Result<(), String> {
    let mut items = staging_list(conn, plugin_id)?;
    let before = items.len();
    items.retain(|x| x.id != id);
    if items.len() == before {
        return Err("item not found".into());
    }
    staging_replace_all(conn, plugin_id, &items)
}

pub fn staging_clear(conn: &Connection, plugin_id: &str) -> Result<(), String> {
    staging_replace_all(conn, plugin_id, &[])
}

pub fn pins_get(conn: &Connection, plugin_id: &str) -> Result<Option<Value>, String> {
    plugin_get_system(conn, plugin_id, KEY_SHORTCUTS_PINS)
}

pub fn pins_set(conn: &Connection, plugin_id: &str, pins: &Value) -> Result<(), String> {
    plugin_set_system(conn, plugin_id, KEY_SHORTCUTS_PINS, pins)
}

pub fn pins_clear(conn: &Connection, plugin_id: &str) -> Result<(), String> {
    plugin_remove_system(conn, plugin_id, KEY_SHORTCUTS_PINS)
}

pub fn pins_list_all(conn: &Connection) -> Result<Vec<(String, Value)>, String> {
    let mut stmt = conn
        .prepare("SELECT plugin_id, value_json FROM plugin_kv WHERE key = ?1")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![KEY_SHORTCUTS_PINS], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (id, json) = row.map_err(|e| e.to_string())?;
        let val: Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        out.push((id, val));
    }
    Ok(out)
}

/// Plugin ids that have a staging index row in plugin_kv.
pub fn staging_list_plugin_ids(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT plugin_id FROM plugin_kv WHERE key = ?1 ORDER BY plugin_id")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![KEY_STAGING_ITEMS], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}
