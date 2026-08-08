//! One-time migration from JSON / text files into SQLite business tables.

use super::{
    ambient_set, launchers_replace_all, material_set, meta_get, meta_set, plugin_set,
    staging_replace_all, tray_set, LauncherRow, StagingRow,
};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

fn root() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    let mut dir = PathBuf::from(appdata);
    dir.push("window-hub");
    Some(dir)
}

fn rename_bak(path: &Path) {
    if !path.is_file() {
        return;
    }
    let bak = PathBuf::from(format!("{}.bak", path.display()));
    let _ = fs::rename(path, &bak);
}

pub fn migrate_legacy_files(conn: &Connection) -> Result<(), String> {
    if meta_get(conn, "migrated_v1")?.is_some() {
        return Ok(());
    }
    let Some(root) = root() else {
        meta_set(conn, "migrated_v1", &json!(true))?;
        return Ok(());
    };

    // material.json
    let material = root.join("material.json");
    if material.is_file() {
        if let Ok(raw) = fs::read_to_string(&material) {
            if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                let _ = material_set(conn, &v);
            }
        }
        rename_bak(&material);
    }

    // ambient-mode.txt
    let ambient = root.join("ambient-mode.txt");
    if ambient.is_file() {
        if let Ok(raw) = fs::read_to_string(&ambient) {
            let mode = raw.trim();
            if !mode.is_empty() {
                let _ = ambient_set(conn, mode);
            }
        }
        rename_bak(&ambient);
    }

    // tray-prefs.json
    let tray = root.join("tray-prefs.json");
    if tray.is_file() {
        if let Ok(raw) = fs::read_to_string(&tray) {
            if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                let _ = tray_set(conn, &v);
            }
        }
        rename_bak(&tray);
    }

    // companions/launchers.json
    let launchers = root.join("companions").join("launchers.json");
    if launchers.is_file() {
        if let Ok(raw) = fs::read_to_string(&launchers) {
            if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                let list = v
                    .get("launchers")
                    .and_then(|x| x.as_array())
                    .cloned()
                    .or_else(|| v.as_array().cloned())
                    .unwrap_or_default();
                let mut rows = Vec::new();
                for item in list {
                    if let Ok(r) = serde_json::from_value::<LauncherRow>(item) {
                        rows.push(r);
                    }
                }
                let _ = launchers_replace_all(conn, &rows);
            }
        }
        rename_bak(&launchers);
    }

    // plugins/*/data/*.json → plugin_kv / shortcut_pins
    let plugins = root.join("plugins");
    if plugins.is_dir() {
        if let Ok(entries) = fs::read_dir(&plugins) {
            for entry in entries.flatten() {
                let plugin_dir = entry.path();
                if !plugin_dir.is_dir() {
                    continue;
                }
                let plugin_id = match plugin_dir.file_name().and_then(|s| s.to_str()) {
                    Some(id) => id.to_string(),
                    None => continue,
                };
                let data = plugin_dir.join("data");
                if !data.is_dir() {
                    continue;
                }
                if let Ok(files) = fs::read_dir(&data) {
                    for f in files.flatten() {
                        let path = f.path();
                        if path.extension().and_then(|e| e.to_str()) != Some("json") {
                            continue;
                        }
                        let stem = match path.file_stem().and_then(|s| s.to_str()) {
                            Some(s) => s.to_string(),
                            None => continue,
                        };
                        let Ok(raw) = fs::read_to_string(&path) else {
                            continue;
                        };
                        let Ok(v) = serde_json::from_str::<Value>(&raw) else {
                            continue;
                        };
                        // Legacy host pin file — discarded (pins live in plugin `store` now).
                        if stem != "shortcuts-pins" {
                            let _ = plugin_set(conn, &plugin_id, &stem, &v);
                        }
                        rename_bak(&path);
                    }
                }
            }
        }
    }

    // plugins/<id>/staging/index.json → that plugin's plugin_kv __staging_items
    if plugins.is_dir() {
        if let Ok(entries) = fs::read_dir(&plugins) {
            for entry in entries.flatten() {
                let plugin_dir = entry.path();
                if !plugin_dir.is_dir() {
                    continue;
                }
                let Some(plugin_id) = plugin_dir.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                let staging_index = plugin_dir.join("staging").join("index.json");
                if !staging_index.is_file() {
                    continue;
                }
                if let Ok(raw) = fs::read_to_string(&staging_index) {
                    if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                        let items = v
                            .get("items")
                            .and_then(|x| x.as_array())
                            .cloned()
                            .unwrap_or_default();
                        let mut rows = Vec::new();
                        for it in items {
                            let id = it
                                .get("id")
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_string();
                            if id.is_empty() {
                                continue;
                            }
                            let kind = it
                                .get("kind")
                                .and_then(|x| x.as_str())
                                .unwrap_or("file")
                                .to_string();
                            let label = it
                                .get("label")
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_string();
                            let path = it
                                .get("path")
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_string();
                            let created_at = it
                                .get("createdAt")
                                .and_then(|x| x.as_u64())
                                .or_else(|| it.get("created_at").and_then(|x| x.as_u64()))
                                .unwrap_or(0);
                            rows.push(StagingRow {
                                id,
                                kind,
                                label,
                                path,
                                created_at,
                            });
                        }
                        let _ = staging_replace_all(conn, plugin_id, &rows);
                    }
                }
                rename_bak(&staging_index);
            }
        }
    }

    meta_set(conn, "migrated_v1", &json!(true))?;
    Ok(())
}
