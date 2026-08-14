//! Host-first 搜搜 launcher — Everything-backed search + app tabs.

mod apps;
mod config;
mod dir;
mod drop_ingest;
mod everything;
mod hotkey;
mod icon_cache;
mod open;
mod pinyin;
mod recent;
mod seed;
mod window;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

pub use config::SousouConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub query: String,
    pub apps: Vec<apps::AppEntry>,
    pub files: Vec<everything::CategoryBucket>,
    pub everything: everything::EverythingStatus,
}

pub fn bootstrap(app: &AppHandle) {
    let cfg = config::load();
    if cfg.enabled {
        // Drop any previous warm/hidden instance — its OLE drop target is often dead.
        window::recreate_next_open(app);
        std::thread::spawn(|| {
            let _ = everything::ensure_running();
            let _ = apps::list_apps(false, 0);
            // Match screenshot categories into tabs (one-shot).
            let _ = seed::seed_tabs_if_needed();
        });
        hotkey::start(app.clone());
        // Do not warm a hidden sousou window: WebView2 drag-drop RegisterDragDrop
        // often fails before child HWNDs exist → permanent "no drop" cursor.
    }
}

#[tauri::command]
pub async fn sousou_toggle(app: AppHandle) -> Result<(), String> {
    window::toggle(app).await
}

#[tauri::command]
pub async fn sousou_open(app: AppHandle) -> Result<(), String> {
    window::open(app).await
}

#[tauri::command]
pub fn sousou_hide(app: AppHandle) -> Result<(), String> {
    window::hide(&app)
}

#[tauri::command]
pub fn sousou_get_config() -> SousouConfig {
    config::load()
}

#[tauri::command]
pub fn sousou_set_config(prefs: SousouConfig) -> Result<SousouConfig, String> {
    let saved = config::update(prefs)?;
    hotkey::configure(saved.hotkey_enabled && saved.enabled, saved.double_ctrl_ms);
    Ok(saved)
}

#[tauri::command]
pub fn sousou_ensure_everything() -> Result<everything::EverythingStatus, String> {
    everything::ensure_running()
}

#[tauri::command]
pub fn sousou_everything_status() -> everything::EverythingStatus {
    everything::status()
}

#[tauri::command]
pub async fn sousou_list_apps(
    with_icons: Option<bool>,
    limit: Option<usize>,
) -> Result<Vec<apps::AppEntry>, String> {
    let with_icons = with_icons.unwrap_or(false);
    let limit = limit.unwrap_or(0);
    tauri::async_runtime::spawn_blocking(move || apps::list_apps(with_icons, limit))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sousou_list_recent(
    limit: Option<usize>,
    with_icons: Option<bool>,
) -> Result<Vec<recent::RecentEntry>, String> {
    let limit = limit.unwrap_or(24);
    let with_icons = with_icons.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || recent::list_recent(limit, with_icons))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sousou_search(
    query: String,
    per_category: Option<usize>,
) -> Result<SearchResponse, String> {
    let per = per_category.unwrap_or(24);
    tauri::async_runtime::spawn_blocking(move || {
        let q = query.trim().to_string();
        let apps_hits = if q.is_empty() {
            Vec::new()
        } else {
            apps::search_apps(&q, 40)
        };
        let ev = everything::status();
        let files = if q.is_empty() {
            Vec::new()
        } else {
            match everything::search_all(&q, per) {
                Ok(buckets) => buckets,
                Err(e) => {
                    // Keep UI usable — still return status so banner can explain.
                    let mut st = ev.clone();
                    if st.message == "就绪" {
                        st.message = e;
                    }
                    return Ok(SearchResponse {
                        query: q,
                        apps: apps_hits,
                        files: Vec::new(),
                        everything: st,
                    });
                }
            }
        };
        Ok(SearchResponse {
            query: q,
            apps: apps_hits,
            files,
            everything: ev,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn sousou_refresh_apps() -> Result<Vec<apps::AppEntry>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        apps::invalidate_cache();
        apps::list_apps(true, 120)
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn sousou_open_path(path: String) -> Result<(), String> {
    open::open_path(&path)
}

#[tauri::command]
pub fn sousou_reveal_path(path: String) -> Result<(), String> {
    open::reveal_in_folder(&path)
}

#[tauri::command]
pub fn sousou_open_system(id: String) -> Result<(), String> {
    open::open_system(&id)
}

#[tauri::command]
pub fn sousou_pick_folder() -> Result<Option<String>, String> {
    let folder = rfd::FileDialog::new()
        .set_title("选择文件夹路径")
        .pick_folder();
    Ok(folder.map(|p| p.to_string_lossy().to_string()))
}

#[tauri::command]
pub fn sousou_first_folder(paths: Vec<String>) -> Option<String> {
    dir::first_folder(&paths)
}

#[tauri::command]
pub async fn sousou_paths_to_shortcuts(
    paths: Vec<String>,
    with_icons: Option<bool>,
) -> Result<Vec<config::SousouShortcut>, String> {
    let with_icons = with_icons.unwrap_or(true);
    tauri::async_runtime::spawn_blocking(move || dir::paths_to_shortcuts(&paths, with_icons))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sousou_list_dir(
    path: String,
    with_icons: Option<bool>,
    limit: Option<usize>,
) -> Result<Vec<dir::DirEntry>, String> {
    let with_icons = with_icons.unwrap_or(false);
    let limit = limit.unwrap_or(0);
    tauri::async_runtime::spawn_blocking(move || dir::list_dir(&path, with_icons, limit))
        .await
        .map_err(|e| e.to_string())?
}

/// Copy dropped paths into a bound-folder tab's real directory.
#[tauri::command]
pub async fn sousou_import_into_folder(
    dest: String,
    paths: Vec<String>,
) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || dir::import_paths_into_dir(&dest, &paths))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn sousou_resolve_icon(path: String) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = path.trim().to_string();
        if p.is_empty() {
            return None;
        }
        icon_cache::get_or_resolve(&p)
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn sousou_icon_cache_stats() -> icon_cache::IconCacheStats {
    icon_cache::stats()
}

#[tauri::command]
pub fn sousou_clear_icon_cache(app: AppHandle) -> Result<icon_cache::IconCacheStats, String> {
    let cleared = icon_cache::clear()?;
    let _ = app.emit("sousou-icon-cache-cleared", ());
    Ok(cleared)
}

#[tauri::command]
pub async fn sousou_seed_tabs() -> Result<SousouConfig, String> {
    tauri::async_runtime::spawn_blocking(|| seed::reseeds_merge())
        .await
        .map_err(|e| e.to_string())?
}
