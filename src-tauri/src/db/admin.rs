//! Developer SQLite browser: list / upsert / delete + backup / restore.

use rusqlite::{params, types::ValueRef, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, State};

use super::{db_path, with_conn};
use crate::plugin_hub::ShortcutsPinStore;

const ALLOWED_TABLES: &[&str] = &[
    "schema_meta",
    "prefs_material",
    "prefs_tray",
    "prefs_shortcuts",
    "prefs_ambient",
    "prefs_island",
    "weather_api",
    "weather_cache",
    "script_launchers",
    "plugin_kv",
];

fn assert_table(table: &str) -> Result<(), String> {
    if ALLOWED_TABLES.contains(&table) {
        Ok(())
    } else {
        Err(format!("table not allowed: {table}"))
    }
}

fn table_pks(table: &str) -> &'static [&'static str] {
    match table {
        "schema_meta" => &["key"],
        "prefs_material" | "prefs_tray" | "prefs_shortcuts" | "prefs_ambient" | "prefs_island"
        | "weather_api" | "weather_cache" => &["id"],
        "script_launchers" => &["id"],
        "plugin_kv" => &["plugin_id", "key"],
        _ => &[],
    }
}

fn table_columns(table: &str) -> &'static [&'static str] {
    match table {
        "schema_meta" => &["key", "value_json", "updated_at"],
        "prefs_material" | "prefs_tray" | "prefs_shortcuts" | "weather_cache" => {
            &["id", "data_json", "updated_at"]
        }
        "prefs_ambient" => &["id", "mode", "updated_at"],
        "prefs_island" => &[
            "id",
            "auto_immerse",
            "immerse_idle_sec",
            "pull_content",
            "msg_notify",
            "msg_notify_text",
            "msg_notify_sec",
            "updated_at",
        ],
        "weather_api" => &["id", "api_id", "api_key", "updated_at"],
        "script_launchers" => &[
            "id",
            "name",
            "script_path",
            "environment",
            "env_path",
            "args",
            "plugin_id",
            "start_with_hub",
            "start_on_boot",
            "enabled",
            "updated_at",
        ],
        "plugin_kv" => &["plugin_id", "key", "value_json", "updated_at"],
        _ => &[],
    }
}

fn value_ref_to_json(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(f) => json!(f),
        ValueRef::Text(t) => {
            let s = String::from_utf8_lossy(t).into_owned();
            // Prefer parsing JSON blobs for *_json columns when valid
            serde_json::from_str(&s).unwrap_or(Value::String(s))
        }
        ValueRef::Blob(b) => Value::String(format!("<blob {} bytes>", b.len())),
    }
}

fn row_value_as_sql(v: &Value) -> Result<rusqlite::types::Value, String> {
    match v {
        Value::Null => Ok(rusqlite::types::Value::Null),
        Value::Bool(b) => Ok(rusqlite::types::Value::Integer(if *b { 1 } else { 0 })),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(rusqlite::types::Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(rusqlite::types::Value::Real(f))
            } else {
                Err("unsupported number".into())
            }
        }
        Value::String(s) => Ok(rusqlite::types::Value::Text(s.clone())),
        other => Ok(rusqlite::types::Value::Text(
            serde_json::to_string(other).map_err(|e| e.to_string())?,
        )),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbTableInfo {
    pub name: String,
    pub row_count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbDevInfo {
    pub path: String,
    pub size_bytes: u64,
    pub schema_version: i32,
    pub tables: Vec<DbTableInfo>,
}

#[tauri::command]
pub fn db_dev_info() -> Result<DbDevInfo, String> {
    let path = db_path()?;
    let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    with_conn(|conn| {
        let schema_version: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0);
        let mut tables = Vec::new();
        for name in ALLOWED_TABLES {
            let row_count: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {name}"), [], |r| r.get(0))
                .unwrap_or(0);
            tables.push(DbTableInfo {
                name: (*name).to_string(),
                row_count,
            });
        }
        Ok(DbDevInfo {
            path: path.display().to_string(),
            size_bytes,
            schema_version,
            tables,
        })
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbDevRows {
    pub columns: Vec<String>,
    pub rows: Vec<Map<String, Value>>,
    pub total: i64,
}

#[tauri::command]
pub fn db_dev_list_rows(
    table: String,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<DbDevRows, String> {
    assert_table(&table)?;
    let limit = limit.unwrap_or(200).clamp(1, 1000);
    let offset = offset.unwrap_or(0).max(0);
    let cols = table_columns(&table);
    let col_sql = cols.join(", ");
    with_conn(|conn| {
        let total: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let sql = format!("SELECT {col_sql} FROM {table} LIMIT ?1 OFFSET ?2");
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let mut rows_out = Vec::new();
        let mut rows = stmt
            .query(params![limit, offset])
            .map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let mut map = Map::new();
            for (i, name) in cols.iter().enumerate() {
                let v = row.get_ref(i).map_err(|e| e.to_string())?;
                let mut json_v = value_ref_to_json(v);
                // Keep raw string for json columns so editor can edit text
                if name.ends_with("_json") || *name == "value_json" || *name == "pins_json" {
                    if let ValueRef::Text(t) = v {
                        json_v = Value::String(String::from_utf8_lossy(t).into_owned());
                    }
                }
                map.insert((*name).to_string(), json_v);
            }
            rows_out.push(map);
        }
        Ok(DbDevRows {
            columns: cols.iter().map(|s| (*s).to_string()).collect(),
            rows: rows_out,
            total,
        })
    })
}

#[tauri::command]
pub fn db_dev_upsert_row(table: String, row: Map<String, Value>) -> Result<(), String> {
    assert_table(&table)?;
    let cols = table_columns(&table);
    for c in cols {
        if !row.contains_key(*c) {
            return Err(format!("missing column: {c}"));
        }
    }
    let pks = table_pks(&table);
    let placeholders: Vec<String> = (1..=cols.len()).map(|i| format!("?{i}")).collect();
    let col_sql = cols.join(", ");
    let ph_sql = placeholders.join(", ");
    let updates: Vec<String> = cols
        .iter()
        .filter(|c| !pks.contains(*c))
        .map(|c| format!("{c}=excluded.{c}"))
        .collect();
    let conflict = pks.join(", ");
    let sql = if updates.is_empty() {
        format!("INSERT OR REPLACE INTO {table} ({col_sql}) VALUES ({ph_sql})")
    } else {
        format!(
            "INSERT INTO {table} ({col_sql}) VALUES ({ph_sql}) ON CONFLICT({conflict}) DO UPDATE SET {}",
            updates.join(", ")
        )
    };
    with_conn(|conn| {
        let mut vals = Vec::with_capacity(cols.len());
        for c in cols {
            vals.push(row_value_as_sql(row.get(*c).unwrap_or(&Value::Null))?);
        }
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        stmt.execute(rusqlite::params_from_iter(vals.iter()))
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

#[tauri::command]
pub fn db_dev_delete_row(table: String, keys: Map<String, Value>) -> Result<(), String> {
    assert_table(&table)?;
    let pks = table_pks(&table);
    if pks.is_empty() {
        return Err("no primary key".into());
    }
    let mut wheres = Vec::new();
    let mut vals = Vec::new();
    for (i, pk) in pks.iter().enumerate() {
        wheres.push(format!("{pk}=?{}", i + 1));
        let Some(v) = keys.get(*pk) else {
            return Err(format!("missing pk: {pk}"));
        };
        vals.push(row_value_as_sql(v)?);
    }
    let sql = format!("DELETE FROM {table} WHERE {}", wheres.join(" AND "));
    with_conn(|conn| {
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let n = stmt
            .execute(rusqlite::params_from_iter(vals.iter()))
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("row not found".into());
        }
        Ok(())
    })
}

#[tauri::command]
pub fn db_dev_clear_table(table: String) -> Result<i64, String> {
    assert_table(&table)?;
    with_conn(|conn| {
        let n = conn
            .execute(&format!("DELETE FROM {table}"), [])
            .map_err(|e| e.to_string())?;
        Ok(n as i64)
    })
}

#[tauri::command]
pub fn db_dev_backup() -> Result<Option<String>, String> {
    let stamp = chrono_like_stamp();
    let default_name = format!("window-hub-backup-{stamp}.db");
    let dest = rfd::FileDialog::new()
        .set_title("备份 Window Hub 数据库")
        .set_file_name(&default_name)
        .add_filter("SQLite", &["db", "sqlite"])
        .save_file();
    let Some(dest) = dest else {
        return Ok(None);
    };
    // VACUUM INTO needs absolute path string
    let dest_str = dest
        .to_str()
        .ok_or_else(|| "invalid backup path".to_string())?
        .replace('\'', "''");
    with_conn(|conn| {
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        conn.execute(&format!("VACUUM INTO '{dest_str}'"), [])
            .map_err(|e| e.to_string())?;
        Ok(())
    })?;
    Ok(Some(dest.display().to_string()))
}

#[tauri::command]
pub fn db_dev_pick_restore_file() -> Result<Option<String>, String> {
    let file = rfd::FileDialog::new()
        .set_title("选择要恢复的备份")
        .add_filter("SQLite", &["db", "sqlite"])
        .pick_file();
    Ok(file.map(|p| p.display().to_string()))
}

#[tauri::command]
pub fn db_dev_restore(
    app: AppHandle,
    path: String,
    pins: State<'_, ShortcutsPinStore>,
) -> Result<(), String> {
    let src = PathBuf::from(path.trim());
    if !src.is_file() {
        return Err("备份文件不存在".into());
    }
    // Validate it's sqlite by opening read-only
    {
        let probe = Connection::open_with_flags(
            &src,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| format!("无法打开备份: {e}"))?;
        let ver: i32 = probe
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .optional()
            .map_err(|e| e.to_string())?
            .unwrap_or(0);
        let _ = ver;
        // Ensure at least one known table exists
        let has: i64 = probe
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='plugin_kv'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if has == 0 {
            return Err("备份中缺少 plugin_kv，可能不是 Window Hub 数据库".into());
        }
    }

    let src_str = src
        .to_str()
        .ok_or_else(|| "invalid path".to_string())?
        .replace('\'', "''");

    with_conn(|conn| {
        conn.execute_batch("PRAGMA foreign_keys = OFF;")
            .map_err(|e| e.to_string())?;
        conn.execute(&format!("ATTACH DATABASE '{src_str}' AS bak"), [])
            .map_err(|e| format!("ATTACH failed: {e}"))?;
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for table in ALLOWED_TABLES {
            // Only copy if table exists in backup
            let exists: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM bak.sqlite_master WHERE type='table' AND name=?1",
                    params![*table],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if exists == 0 {
                continue;
            }
            tx.execute(&format!("DELETE FROM main.{table}"), [])
                .map_err(|e| e.to_string())?;
            // Named columns so older backups with dropped cols (e.g. staging_panel_w) still restore.
            let cols = table_columns(table);
            if cols.is_empty() {
                tx.execute(
                    &format!("INSERT INTO main.{table} SELECT * FROM bak.{table}"),
                    [],
                )
                .map_err(|e| format!("restore {table}: {e}"))?;
            } else {
                let col_sql = cols.join(", ");
                tx.execute(
                    &format!(
                        "INSERT INTO main.{table} ({col_sql}) SELECT {col_sql} FROM bak.{table}"
                    ),
                    [],
                )
                .map_err(|e| format!("restore {table}: {e}"))?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;

        // v1 backup: host_kv → entity tables
        let has_host_kv: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bak.sqlite_master WHERE type='table' AND name='host_kv'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if has_host_kv > 0 {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS main.host_kv (
                  namespace TEXT NOT NULL,
                  key TEXT NOT NULL,
                  value_json TEXT NOT NULL,
                  updated_at INTEGER NOT NULL,
                  PRIMARY KEY (namespace, key)
                );
                DELETE FROM main.host_kv;
                INSERT INTO main.host_kv SELECT * FROM bak.host_kv;
                "#,
            )
            .map_err(|e| format!("restore host_kv bridge: {e}"))?;
            crate::db::host::migrate_from_host_kv(conn)?;
        }

        // Pre-v3 backup: staging_items / shortcut_pins → plugin_kv reserved keys
        let has_side: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bak.sqlite_master WHERE type='table' AND name IN ('staging_items','shortcut_pins')",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if has_side > 0 {
            let _ = conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS main.shortcut_pins (
                  plugin_id TEXT PRIMARY KEY,
                  pins_json TEXT NOT NULL,
                  updated_at INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS main.staging_items (
                  id TEXT PRIMARY KEY,
                  kind TEXT NOT NULL,
                  label TEXT NOT NULL,
                  path TEXT NOT NULL,
                  created_at INTEGER NOT NULL
                );
                "#,
            );
            let has_pins: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM bak.sqlite_master WHERE type='table' AND name='shortcut_pins'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if has_pins > 0 {
                let _ = conn.execute_batch(
                    "DELETE FROM main.shortcut_pins; INSERT INTO main.shortcut_pins SELECT * FROM bak.shortcut_pins;",
                );
            }
            let has_st: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM bak.sqlite_master WHERE type='table' AND name='staging_items'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if has_st > 0 {
                let _ = conn.execute_batch(
                    "DELETE FROM main.staging_items; INSERT INTO main.staging_items SELECT * FROM bak.staging_items;",
                );
            }
            crate::db::migrate_plugin_side_tables_into_kv(conn)?;
        }

        conn.execute("DETACH DATABASE bak", [])
            .map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|e| e.to_string())?;
        Ok(())
    })?;

    // Refresh in-memory caches
    crate::staging::reload_from_db();
    pins.load_all_from_db();
    for id in crate::staging::plugin_ids_with_staging() {
        let _ = app.emit(
            "staging-changed",
            crate::staging::StagingChangedPayload {
                plugin_id: id.clone(),
                summary: crate::staging::summary(&id),
            },
        );
    }
    let _ = app.emit("shortcuts-pins-changed", pins.all_flat());
    let _ = app.emit("db-restored", ());
    Ok(())
}

fn chrono_like_stamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}
