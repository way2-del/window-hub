//! Ingest Explorer/Desktop drops into the active sousou tab.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use super::config::{self, SousouConfig, SousouShortcut};
use super::dir;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DropResult {
    pub ok: bool,
    pub added: usize,
    pub message: String,
    pub prefs: Option<SousouConfig>,
}

fn merge_shortcuts(existing: &[SousouShortcut], incoming: Vec<SousouShortcut>) -> (Vec<SousouShortcut>, usize) {
    let mut out = existing.to_vec();
    let mut seen: std::collections::HashSet<String> = out
        .iter()
        .map(|i| i.path.to_ascii_lowercase())
        .collect();
    let mut n = 0usize;
    for it in incoming {
        let key = it.path.to_ascii_lowercase();
        if seen.insert(key) {
            out.push(it);
            n += 1;
        }
    }
    (out, n)
}

/// Add dropped paths to the active tab (or into its bound folder).
pub fn ingest_dropped_paths(paths: Vec<String>) -> DropResult {
    let paths: Vec<String> = paths
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if paths.is_empty() {
        return DropResult {
            ok: false,
            added: 0,
            message: "未读到文件路径".into(),
            prefs: None,
        };
    }

    let mut cfg = config::load();
    let tab_id = if cfg.active_tab_id.trim().is_empty() {
        "home".into()
    } else {
        cfg.active_tab_id.clone()
    };

    // Folder-bound tab → copy into real directory
    if let Some(tab) = cfg.tabs.iter().find(|t| t.id == tab_id) {
        let bound = tab.folder_path.trim();
        if !bound.is_empty() {
            return match dir::import_paths_into_dir(bound, &paths) {
                Ok(n) => DropResult {
                    ok: true,
                    added: n,
                    message: if n > 0 {
                        format!("已放入文件夹 {n} 项")
                    } else {
                        "没有可放入的项目".into()
                    },
                    prefs: Some(cfg),
                },
                Err(e) => DropResult {
                    ok: false,
                    added: 0,
                    message: e,
                    prefs: None,
                },
            };
        }
    }

    let items = dir::paths_to_shortcuts(&paths, true);
    if items.is_empty() {
        return DropResult {
            ok: false,
            added: 0,
            message: "没有可添加的项目".into(),
            prefs: None,
        };
    }

    let mut added = 0usize;
    if tab_id == "home" {
        let (home_apps, n) = merge_shortcuts(&cfg.home_apps, items.clone());
        added = n;
        cfg.home_apps = home_apps;
        cfg.tabs = cfg
            .tabs
            .into_iter()
            .map(|mut t| {
                if t.id == "home" {
                    let (next, _) = merge_shortcuts(&t.items, items.clone());
                    t.items = next;
                }
                t
            })
            .collect();
    } else {
        cfg.tabs = cfg
            .tabs
            .into_iter()
            .map(|mut t| {
                if t.id == tab_id {
                    let (next, n) = merge_shortcuts(&t.items, items.clone());
                    added = n;
                    t.items = next;
                }
                t
            })
            .collect();
    }

    match config::update(cfg) {
        Ok(prefs) => DropResult {
            ok: true,
            added,
            message: if added > 0 {
                format!("已添加 {added} 项")
            } else {
                "已存在，未重复添加".into()
            },
            prefs: Some(prefs),
        },
        Err(e) => DropResult {
            ok: false,
            added: 0,
            message: e,
            prefs: None,
        },
    }
}

pub fn emit_drop_result(app: &AppHandle, result: DropResult) {
    let _ = app.emit("sousou-drop-result", &result);
}
