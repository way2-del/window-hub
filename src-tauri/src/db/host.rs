//! Host business tables (not generic KV). Plugin data stays in `plugin_kv`.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::now_ms;

fn singleton_get_json(conn: &Connection, table: &str) -> Result<Option<Value>, String> {
    conn.query_row(
        &format!("SELECT data_json FROM {table} WHERE id = 1"),
        [],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| e.to_string())?
    .map(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
    .transpose()
}

fn singleton_set_json(conn: &Connection, table: &str, value: &Value) -> Result<(), String> {
    let text = serde_json::to_string(value).map_err(|e| e.to_string())?;
    conn.execute(
        &format!(
            "INSERT INTO {table}(id, data_json, updated_at) VALUES(1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET data_json=excluded.data_json, updated_at=excluded.updated_at"
        ),
        params![text, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ── schema_meta ──────────────────────────────────────────────────────

pub fn meta_get(conn: &Connection, key: &str) -> Result<Option<Value>, String> {
    conn.query_row(
        "SELECT value_json FROM schema_meta WHERE key = ?1",
        params![key],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| e.to_string())?
    .map(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
    .transpose()
}

pub fn meta_set(conn: &Connection, key: &str, value: &Value) -> Result<(), String> {
    let text = serde_json::to_string(value).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO schema_meta(key, value_json, updated_at) VALUES(?1,?2,?3)
         ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json, updated_at=excluded.updated_at",
        params![key, text, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ── prefs_material / prefs_tray (document singleton) ─────────────────

pub fn material_get(conn: &Connection) -> Result<Option<Value>, String> {
    singleton_get_json(conn, "prefs_material")
}

pub fn material_set(conn: &Connection, value: &Value) -> Result<(), String> {
    singleton_set_json(conn, "prefs_material", value)
}

pub fn tray_get(conn: &Connection) -> Result<Option<Value>, String> {
    singleton_get_json(conn, "prefs_tray")
}

pub fn tray_set(conn: &Connection, value: &Value) -> Result<(), String> {
    singleton_set_json(conn, "prefs_tray", value)
}

// ── prefs_shortcuts ──────────────────────────────────────────────────

pub fn shortcuts_get(conn: &Connection) -> Result<Option<Value>, String> {
    singleton_get_json(conn, "prefs_shortcuts")
}

pub fn shortcuts_set(conn: &Connection, value: &Value) -> Result<(), String> {
    singleton_set_json(conn, "prefs_shortcuts", value)
}

// ── prefs_ambient ────────────────────────────────────────────────────

pub fn ambient_get(conn: &Connection) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT mode FROM prefs_ambient WHERE id = 1",
        [],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn ambient_set(conn: &Connection, mode: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO prefs_ambient(id, mode, updated_at) VALUES(1, ?1, ?2)
         ON CONFLICT(id) DO UPDATE SET mode=excluded.mode, updated_at=excluded.updated_at",
        params![mode, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ── prefs_island ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IslandPrefsRow {
    pub auto_immerse: bool,
    pub immerse_idle_sec: u32,
    pub pull_content: String,
    pub msg_notify: bool,
    pub msg_notify_text: String,
    pub msg_notify_sec: u32,
}

pub fn island_get(conn: &Connection) -> Result<Option<IslandPrefsRow>, String> {
    conn.query_row(
        "SELECT auto_immerse, immerse_idle_sec, pull_content, msg_notify, msg_notify_text, msg_notify_sec
         FROM prefs_island WHERE id = 1",
        [],
        |r| {
            Ok(IslandPrefsRow {
                auto_immerse: r.get::<_, i64>(0)? != 0,
                immerse_idle_sec: r.get::<_, i64>(1)? as u32,
                pull_content: r.get(2)?,
                msg_notify: r.get::<_, i64>(3)? != 0,
                msg_notify_text: r.get(4)?,
                msg_notify_sec: r.get::<_, i64>(5)? as u32,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn island_set(conn: &Connection, p: &IslandPrefsRow) -> Result<(), String> {
    conn.execute(
        "INSERT INTO prefs_island(
            id, auto_immerse, immerse_idle_sec, pull_content, msg_notify, msg_notify_text, msg_notify_sec, updated_at
         ) VALUES(1,?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(id) DO UPDATE SET
           auto_immerse=excluded.auto_immerse,
           immerse_idle_sec=excluded.immerse_idle_sec,
           pull_content=excluded.pull_content,
           msg_notify=excluded.msg_notify,
           msg_notify_text=excluded.msg_notify_text,
           msg_notify_sec=excluded.msg_notify_sec,
           updated_at=excluded.updated_at",
        params![
            p.auto_immerse as i64,
            p.immerse_idle_sec as i64,
            p.pull_content,
            p.msg_notify as i64,
            p.msg_notify_text,
            p.msg_notify_sec as i64,
            now_ms(),
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ── weather ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherApiRow {
    pub id: String,
    pub key: String,
}

pub fn weather_api_get(conn: &Connection) -> Result<Option<WeatherApiRow>, String> {
    conn.query_row(
        "SELECT api_id, api_key FROM weather_api WHERE id = 1",
        [],
        |r| {
            Ok(WeatherApiRow {
                id: r.get(0)?,
                key: r.get(1)?,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn weather_api_set(conn: &Connection, row: &WeatherApiRow) -> Result<(), String> {
    conn.execute(
        "INSERT INTO weather_api(id, api_id, api_key, updated_at) VALUES(1, ?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET api_id=excluded.api_id, api_key=excluded.api_key, updated_at=excluded.updated_at",
        params![row.id, row.key, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn weather_cache_get(conn: &Connection) -> Result<Option<Value>, String> {
    singleton_get_json(conn, "weather_cache")
}

pub fn weather_cache_set(conn: &Connection, value: &Value) -> Result<(), String> {
    singleton_set_json(conn, "weather_cache", value)
}

// ── script_launchers ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LauncherRow {
    pub id: String,
    pub name: String,
    pub script_path: String,
    pub environment: String,
    #[serde(default)]
    pub env_path: String,
    #[serde(default)]
    pub args: String,
    #[serde(default)]
    pub plugin_id: String,
    #[serde(default)]
    pub start_with_hub: bool,
    #[serde(default)]
    pub start_on_boot: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

pub fn launchers_list(conn: &Connection) -> Result<Vec<LauncherRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, script_path, environment, env_path, args, plugin_id,
                    start_with_hub, start_on_boot, enabled
             FROM script_launchers ORDER BY name COLLATE NOCASE",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(LauncherRow {
                id: r.get(0)?,
                name: r.get(1)?,
                script_path: r.get(2)?,
                environment: r.get(3)?,
                env_path: r.get(4)?,
                args: r.get(5)?,
                plugin_id: r.get(6)?,
                start_with_hub: r.get::<_, i64>(7)? != 0,
                start_on_boot: r.get::<_, i64>(8)? != 0,
                enabled: r.get::<_, i64>(9)? != 0,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

pub fn launchers_replace_all(conn: &Connection, items: &[LauncherRow]) -> Result<(), String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM script_launchers", [])
        .map_err(|e| e.to_string())?;
    let now = now_ms();
    for it in items {
        tx.execute(
            "INSERT INTO script_launchers(
                id, name, script_path, environment, env_path, args, plugin_id,
                start_with_hub, start_on_boot, enabled, updated_at
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                it.id,
                it.name,
                it.script_path,
                it.environment,
                it.env_path,
                it.args,
                it.plugin_id,
                it.start_with_hub as i64,
                it.start_on_boot as i64,
                it.enabled as i64,
                now,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// DDL for host business tables (schema v2+).
pub fn create_host_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS schema_meta (
          key TEXT PRIMARY KEY,
          value_json TEXT NOT NULL,
          updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS prefs_material (
          id INTEGER PRIMARY KEY CHECK (id = 1),
          data_json TEXT NOT NULL,
          updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS prefs_tray (
          id INTEGER PRIMARY KEY CHECK (id = 1),
          data_json TEXT NOT NULL,
          updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS prefs_shortcuts (
          id INTEGER PRIMARY KEY CHECK (id = 1),
          data_json TEXT NOT NULL,
          updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS prefs_ambient (
          id INTEGER PRIMARY KEY CHECK (id = 1),
          mode TEXT NOT NULL,
          updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS prefs_island (
          id INTEGER PRIMARY KEY CHECK (id = 1),
          auto_immerse INTEGER NOT NULL,
          immerse_idle_sec INTEGER NOT NULL,
          pull_content TEXT NOT NULL,
          msg_notify INTEGER NOT NULL,
          msg_notify_text TEXT NOT NULL,
          msg_notify_sec INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS weather_api (
          id INTEGER PRIMARY KEY CHECK (id = 1),
          api_id TEXT NOT NULL,
          api_key TEXT NOT NULL,
          updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS weather_cache (
          id INTEGER PRIMARY KEY CHECK (id = 1),
          data_json TEXT NOT NULL,
          updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS script_launchers (
          id TEXT PRIMARY KEY,
          name TEXT NOT NULL,
          script_path TEXT NOT NULL,
          environment TEXT NOT NULL,
          env_path TEXT NOT NULL DEFAULT '',
          args TEXT NOT NULL DEFAULT '',
          plugin_id TEXT NOT NULL DEFAULT '',
          start_with_hub INTEGER NOT NULL DEFAULT 0,
          start_on_boot INTEGER NOT NULL DEFAULT 0,
          enabled INTEGER NOT NULL DEFAULT 1,
          updated_at INTEGER NOT NULL
        );
        "#,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Copy legacy `host_kv` rows into business tables (schema 1 → 2).
pub fn migrate_from_host_kv(conn: &Connection) -> Result<(), String> {
    let has: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='host_kv'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has == 0 {
        return Ok(());
    }

    let mut stmt = conn
        .prepare("SELECT namespace, key, value_json FROM host_kv")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    for row in rows {
        let (ns, key, raw) = row.map_err(|e| e.to_string())?;
        let Ok(v) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        match (ns.as_str(), key.as_str()) {
            ("meta", k) => {
                let _ = meta_set(conn, k, &v);
            }
            ("prefs", "material") => {
                let _ = material_set(conn, &v);
            }
            ("prefs", "tray") => {
                let _ = tray_set(conn, &v);
            }
            ("prefs", "ambient") => {
                let mode = match &v {
                    Value::String(s) => s.clone(),
                    other => other.as_str().unwrap_or("edge").to_string(),
                };
                let _ = ambient_set(conn, &mode);
            }
            ("prefs", "island") => {
                if let Ok(p) = serde_json::from_value::<IslandPrefsRow>(v) {
                    let _ = island_set(conn, &p);
                }
            }
            ("prefs", "todos") => {
                // Removed host todo_items table; discard legacy host_kv blob.
            }
            ("prefs", "launchers") => {
                // { launchers: [...] } or bare array
                let list = if let Some(arr) = v.get("launchers").and_then(|x| x.as_array()) {
                    arr.clone()
                } else if let Value::Array(arr) = &v {
                    arr.clone()
                } else {
                    Vec::new()
                };
                let mut rows = Vec::new();
                for item in list {
                    if let Ok(r) = serde_json::from_value::<LauncherRow>(item) {
                        rows.push(r);
                    }
                }
                let _ = launchers_replace_all(conn, &rows);
            }
            ("secrets", "weather_api") => {
                let id = v
                    .get("id")
                    .and_then(|x| x.as_str())
                    .unwrap_or("88888888")
                    .to_string();
                let key = v
                    .get("key")
                    .and_then(|x| x.as_str())
                    .unwrap_or("88888888")
                    .to_string();
                let _ = weather_api_set(conn, &WeatherApiRow { id, key });
            }
            ("cache", "weather") => {
                let _ = weather_cache_set(conn, &v);
            }
            _ => {}
        }
    }

    conn.execute_batch("DROP TABLE IF EXISTS host_kv;")
        .map_err(|e| e.to_string())?;
    Ok(())
}
