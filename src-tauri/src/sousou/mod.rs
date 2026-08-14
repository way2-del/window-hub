//! Host-first 搜搜 launcher — Everything-backed search + app tabs.

mod apps;
mod config;
mod dir;
mod everything;
mod hotkey;
mod open;
mod pinyin;
mod recent;
mod seed;
mod window;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

pub use config::{SearchFilter, SousouConfig};

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
        std::thread::spawn(|| {
            let _ = everything::ensure_running();
            let _ = apps::list_apps(false, 0);
            // Match screenshot categories into tabs (one-shot).
            let _ = seed::seed_tabs_if_needed();
        });
        hotkey::start(app.clone());
        let app2 = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(1800));
            tauri::async_runtime::block_on(async move {
                let _ = window::warm(app2).await;
            });
        });
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
        let files = if q.is_empty() {
            Vec::new()
        } else {
            everything::search_all(&q, per).unwrap_or_default()
        };
        Ok(SearchResponse {
            query: q,
            apps: apps_hits,
            files,
            everything: everything::status(),
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

#[tauri::command]
pub async fn sousou_resolve_icon(path: String) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = path.trim().to_string();
        if p.is_empty() {
            return None;
        }
        crate::dock::resolve_launcher_icon_png(&p)
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sousou_seed_tabs() -> Result<SousouConfig, String> {
    tauri::async_runtime::spawn_blocking(|| seed::reseeds_merge())
        .await
        .map_err(|e| e.to_string())?
}
