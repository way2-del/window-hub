//! Local .whpx / directory plugin install under %APPDATA%/window-hub/plugins

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager};

const WINDOW_GROUPS_EXAMPLE_ID: &str = "com.window-hub.window-groups";
const TRANSFER_EXAMPLE_ID: &str = "com.window-hub.transfer-station";
const WEATHER_EXAMPLE_ID: &str = "com.window-hub.weather";
const MIRROR_EXAMPLE_ID: &str = "com.window-hub.mirror";
const APP_LIBRARY_EXAMPLE_ID: &str = "com.window-hub.app-library";
const IDIOM_EXAMPLE_ID: &str = "com.window-hub.idiom";
const NOW_PLAYING_EXAMPLE_ID: &str = "com.window-hub.now-playing";
const FILE_SEARCH_EXAMPLE_ID: &str = "com.window-hub.file-search";
const SYSMON_EXAMPLE_ID: &str = "com.window-hub.sysmon";
const EXCALIDRAW_EXAMPLE_ID: &str = "com.window-hub.excalidraw";
const WORLD_CLOCK_EXAMPLE_ID: &str = "com.window-hub.world-clock";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPluginRecord {
    pub id: String,
    pub name: String,
    pub version: String,
    pub path: String,
    pub enabled: bool,
    pub is_dev: bool,
    /// Absolute path of the folder the user imported (directory install only).
    /// On startup we re-copy from here so edits don't require re-import.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev_source: Option<String>,
    pub capabilities: Vec<String>,
    pub manifest: Value,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RegistryFile {
    plugins: Vec<InstalledPluginRecord>,
    #[serde(default, rename = "officialSeeded")]
    official_seeded: bool,
    #[serde(default, rename = "transferStationSeeded")]
    transfer_station_seeded: bool,
}

fn plugins_root() -> Result<PathBuf, String> {
    let appdata = std::env::var_os("APPDATA").ok_or_else(|| "APPDATA missing".to_string())?;
    let mut dir = PathBuf::from(appdata);
    dir.push("window-hub");
    dir.push("plugins");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn registry_path() -> Result<PathBuf, String> {
    let mut p = plugins_root()?;
    p.push("registry.json");
    Ok(p)
}

fn load_registry() -> RegistryFile {
    let Ok(path) = registry_path() else {
        return RegistryFile::default();
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_registry(reg: &RegistryFile) -> Result<(), String> {
    let path = registry_path()?;
    let text = serde_json::to_string_pretty(reg).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| e.to_string())
}

fn validate_manifest(v: &Value) -> Result<(String, String, String, Vec<String>), String> {
    let id = v
        .get("id")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "plugin.json missing id".to_string())?
        .to_string();
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
        || !id.contains('.')
    {
        return Err(format!("invalid plugin id: {id}"));
    }
    let name = v
        .get("name")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "plugin.json missing name".to_string())?
        .to_string();
    let version = v
        .get("version")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "plugin.json missing version".to_string())?
        .to_string();
    let capabilities = v
        .get("capabilities")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let slots = v.get("slots");
    let has_slot = slots
        .map(|s| {
            s.get("shortcuts").is_some()
                || s.get("island.notify").is_some()
                || s.get("island.panel").is_some()
        })
        .unwrap_or(false);
    let has_entry = v
        .get("entry")
        .map(|e| e.get("panel").is_some() || e.get("popup").is_some())
        .unwrap_or(false);
    if !has_slot && !has_entry {
        return Err("plugin must declare slots or entry".into());
    }
    Ok((id, name, version, capabilities))
}

fn strip_utf8_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

fn read_manifest_file(dir: &Path) -> Result<Value, String> {
    let path = dir.join("plugin.json");
    let text = fs::read_to_string(&path).map_err(|e| format!("read plugin.json: {e}"))?;
    serde_json::from_str(strip_utf8_bom(text.trim_start()))
        .map_err(|e| format!("parse plugin.json: {e}"))
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &to)?;
        } else {
            fs::copy(entry.path(), to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn extract_whpx(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("open whpx: {e}"))?;
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry
            .enclosed_name()
            .ok_or_else(|| "illegal path in archive".to_string())?
            .to_owned();
        let out = dest.join(&name);
        if entry.is_dir() {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut outfile = fs::File::create(&out).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut outfile).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn upsert_record(reg: &mut RegistryFile, record: InstalledPluginRecord) {
    if let Some(existing) = reg.plugins.iter_mut().find(|p| p.id == record.id) {
        *existing = record;
    } else {
        reg.plugins.push(record);
    }
}

fn emit_plugins(app: &AppHandle, reg: &RegistryFile) {
    let _ = app.emit("plugins-changed", &reg.plugins);
}

pub(crate) fn find_installed_plugin(id: &str) -> Option<InstalledPluginRecord> {
    let reg = load_registry();
    if let Some(p) = reg.plugins.iter().find(|p| p.id == id).cloned() {
        return Some(p);
    }
    // Host / prefs may still reference the official id while only `__dev` is installed.
    if !id.ends_with("__dev") {
        let dev = format!("{id}__dev");
        return reg.plugins.into_iter().find(|p| p.id == dev);
    }
    None
}

fn close_plugin_popup(app: &AppHandle) {
    for label in ["plugin-popup", "plugin-window"] {
        if let Some(win) = app.get_webview_window(label) {
            let _ = win.close();
        }
    }
    let _ = app.emit("plugin-popup-closed", ());
}

pub fn list_installed_plugins_sync() -> Vec<InstalledPluginRecord> {
    // Transfer-station / window-groups: normal plugins (no embed seed / no overwrite-on-launch).
    // Weather / mirror: ensured separately on app setup (missing-only).
    load_registry().plugins
}

fn plugin_already_installed(reg: &RegistryFile, id: &str) -> bool {
    let dev = format!("{id}__dev");
    reg.plugins.iter().any(|p| p.id == id || p.id == dev)
}

/// Install or bump official weather / mirror / transfer from resources when missing or version differs.
/// Does not touch `__dev` installs.
pub fn ensure_official_plugins(app: &AppHandle) {
    for (folder, id) in [
        ("weather", WEATHER_EXAMPLE_ID),
        ("mirror", MIRROR_EXAMPLE_ID),
        ("transfer-station", TRANSFER_EXAMPLE_ID),
        ("idiom", IDIOM_EXAMPLE_ID),
        ("now-playing", NOW_PLAYING_EXAMPLE_ID),
        ("file-search", FILE_SEARCH_EXAMPLE_ID),
        ("world-clock", WORLD_CLOCK_EXAMPLE_ID),
    ] {
        let reg = load_registry();
        let bundled = match resolve_example_plugin_dir(app, folder) {
            Ok(src) => src,
            Err(e) => {
                eprintln!("[plugins] ensure {id}: {e}");
                continue;
            }
        };
        let bundled_ver = read_manifest_file(&bundled)
            .ok()
            .and_then(|m| {
                m.get("version")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default();
        let installed = reg.plugins.iter().find(|p| p.id == id);
        let dev_present = reg.plugins.iter().any(|p| p.id == format!("{id}__dev"));
        if let Some(rec) = installed {
            if rec.version == bundled_ver || bundled_ver.is_empty() {
                continue;
            }
            // Official release bump only (skip if user has a parallel __dev copy as source of truth)
            if dev_present {
                continue;
            }
            if let Err(e) = install_from_dir(app, &bundled, false) {
                eprintln!("[plugins] upgrade {id} → {bundled_ver} failed: {e}");
            }
            continue;
        }
        // Production id missing: always install it (even if a __dev copy exists).
        // Previously `plugin_already_installed` treated __dev as enough and left
        // Host code that hard-codes the official id without a panel to load.
        if let Err(e) = install_from_dir(app, &bundled, false) {
            eprintln!("[plugins] ensure {id} failed: {e}");
        }
    }
}

#[tauri::command]
pub fn list_installed_plugins() -> Result<Vec<InstalledPluginRecord>, String> {
    Ok(list_installed_plugins_sync())
}

#[tauri::command]
pub fn pick_whpx_file() -> Result<Option<String>, String> {
    let file = rfd::FileDialog::new()
        .add_filter("Window Hub Plugin", &["whpx", "zip"])
        .set_title("选择插件包 (.whpx)")
        .pick_file();
    Ok(file.map(|p| p.to_string_lossy().to_string()))
}

#[tauri::command]
pub fn pick_plugin_directory() -> Result<Option<String>, String> {
    let dir = rfd::FileDialog::new()
        .set_title("选择插件开发目录（含 plugin.json）")
        .pick_folder();
    Ok(dir.map(|p| p.to_string_lossy().to_string()))
}

fn remove_plugin_dir(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    fs::remove_dir_all(path).map_err(|e| format!("删除插件目录失败: {e}"))
}

fn install_from_dir(app: &AppHandle, src: &Path, is_dev: bool) -> Result<InstalledPluginRecord, String> {
    let manifest = read_manifest_file(src)?;
    let (id, name, version, capabilities) = validate_manifest(&manifest)?;
    let install_id = if is_dev && !id.ends_with("__dev") {
        format!("{id}__dev")
    } else {
        id.clone()
    };
    let mut manifest = manifest;
    if is_dev {
        if let Some(obj) = manifest.as_object_mut() {
            obj.insert("id".into(), Value::String(install_id.clone()));
        }
    }

    let root = plugins_root()?;
    let dest = root.join(&install_id);
    remove_plugin_dir(&dest)?;
    copy_dir_recursive(src, &dest)?;
    // rewrite manifest with possibly rewritten id
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(dest.join("plugin.json"), text).map_err(|e| e.to_string())?;

    let dev_source = if is_dev {
        Some(
            src.canonicalize()
                .unwrap_or_else(|_| src.to_path_buf())
                .to_string_lossy()
                .to_string(),
        )
    } else {
        None
    };

    let prev_enabled = {
        let reg = load_registry();
        reg.plugins
            .iter()
            .find(|p| p.id == install_id)
            .map(|p| p.enabled)
    };

    let record = InstalledPluginRecord {
        id: install_id,
        name,
        version,
        path: dest.to_string_lossy().to_string(),
        // Preserve user toggle across resync / upgrade (never force-enable).
        enabled: prev_enabled.unwrap_or(true),
        is_dev,
        dev_source,
        capabilities,
        manifest,
    };
    let mut reg = load_registry();
    upsert_record(&mut reg, record.clone());
    save_registry(&reg)?;
    emit_plugins(app, &reg);
    Ok(record)
}

pub fn resync_dev_plugins(app: &AppHandle) {
    let reg = load_registry();
    let jobs: Vec<(String, String)> = reg
        .plugins
        .iter()
        .filter(|p| p.is_dev)
        .filter_map(|p| {
            p.dev_source
                .as_ref()
                .map(|s| (p.id.clone(), s.clone()))
        })
        .collect();

    for (id, src_s) in jobs {
        let src = PathBuf::from(&src_s);
        if !src.is_dir() || !src.join("plugin.json").is_file() {
            eprintln!("[plugins] dev {id} source missing: {src_s}");
            continue;
        }
        match install_from_dir(app, &src, true) {
            Ok(_) => eprintln!("[plugins] resynced dev {id} ← {src_s}"),
            Err(e) => eprintln!("[plugins] resync {id} failed: {e}"),
        }
    }

    for rec in load_registry().plugins.iter().filter(|p| p.is_dev && p.dev_source.is_none()) {
        eprintln!(
            "[plugins] dev {} has no source path — re-import the folder once to enable auto-sync on restart",
            rec.id
        );
    }
}

#[tauri::command]
pub fn install_plugin_from_path(app: AppHandle, path: String) -> Result<InstalledPluginRecord, String> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err("path does not exist".into());
    }
    if p.is_dir() {
        return install_from_dir(&app, &p, true);
    }
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext != "whpx" && ext != "zip" {
        return Err("expected .whpx / .zip or a directory".into());
    }

    let root = plugins_root()?;
    let staging = root.join(format!(".staging-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    extract_whpx(&p, &staging)?;

    // support archives with nested single root folder
    let manifest_dir = if staging.join("plugin.json").is_file() {
        staging.clone()
    } else {
        let mut found = None;
        for entry in fs::read_dir(&staging).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.path().join("plugin.json").is_file() {
                found = Some(entry.path());
                break;
            }
        }
        found.ok_or_else(|| "plugin.json not found in archive".to_string())?
    };

    let result = install_from_dir(&app, &manifest_dir, false);
    let _ = fs::remove_dir_all(&staging);
    result
}

#[tauri::command]
pub fn uninstall_plugin(
    app: AppHandle,
    id: String,
    pins: tauri::State<'_, crate::plugin_hub::ShortcutsPinStore>,
) -> Result<(), String> {
    let mut reg = load_registry();
    let Some(idx) = reg.plugins.iter().position(|p| p.id == id) else {
        return Err("plugin not installed".into());
    };
    let record = reg.plugins.remove(idx);
    save_registry(&reg)?;
    let path = PathBuf::from(&record.path);
    remove_plugin_dir(&path)?;
    close_plugin_popup(&app);
    let _ = crate::db::with_conn(|c| crate::db::plugin_clear_all(c, &id));
    pins.inner_remove(&id);
    crate::commands::detach_plugin_from_island_prefs(&app, &id);
    let _ = app.emit("shortcuts-pins-changed", pins.all_flat());
    emit_plugins(&app, &reg);
    Ok(())
}

#[tauri::command]
pub fn set_plugin_enabled(
    app: AppHandle,
    id: String,
    enabled: bool,
    pins: tauri::State<'_, crate::plugin_hub::ShortcutsPinStore>,
) -> Result<(), String> {
    let mut reg = load_registry();
    let Some(p) = reg.plugins.iter_mut().find(|p| p.id == id) else {
        return Err("plugin not installed".into());
    };
    p.enabled = enabled;
    let has_everything = p.capabilities.iter().any(|c| c == "everything.search");
    save_registry(&reg)?;
    if !enabled {
        close_plugin_popup(&app);
        pins.inner_remove(&id);
        #[cfg(windows)]
        if has_everything {
            crate::everything::reset_if_idle();
        }
    } else {
        pins.reload_plugin(&id);
        crate::companion_scripts::start_launchers_for_plugin(&id);
    }
    let _ = app.emit("shortcuts-pins-changed", pins.all_flat());
    emit_plugins(&app, &reg);
    #[cfg(windows)]
    crate::win32::hotkey_registry::reload(&app);
    Ok(())
}

/// Resolve a bundled example directory (docs in dev, resources when packaged).
fn resolve_example_plugin_dir(app: &AppHandle, folder: &str) -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    candidates.push(manifest_dir.join(format!("../docs/plugins/examples/{folder}")));
    candidates.push(manifest_dir.join(format!("resources/plugins/{folder}")));

    if let Ok(p) = app.path().resolve(
        format!("resources/plugins/{folder}/plugin.json"),
        BaseDirectory::Resource,
    ) {
        if p.is_file() {
            if let Some(parent) = p.parent() {
                candidates.push(parent.to_path_buf());
            }
        }
    }

    for c in candidates {
        let dir = c.canonicalize().unwrap_or(c);
        if dir.join("plugin.json").is_file() {
            return Ok(dir);
        }
    }
    Err(format!(
        "找不到示例包「{folder}」：请用「安装 .whpx / 添加开发目录」，或确认 resources/plugins/{folder} 已打包"
    ))
}

fn example_folder(example_id: &str) -> Result<&'static str, String> {
    let id = example_id.trim();
    if id == "window-groups" || id == WINDOW_GROUPS_EXAMPLE_ID {
        Ok("window-groups")
    } else if id == "transfer-station" || id == TRANSFER_EXAMPLE_ID {
        Ok("transfer-station")
    } else if id == "weather" || id == WEATHER_EXAMPLE_ID {
        Ok("weather")
    } else if id == "mirror" || id == MIRROR_EXAMPLE_ID {
        Ok("mirror")
    } else if id == "app-library" || id == APP_LIBRARY_EXAMPLE_ID {
        Ok("app-library")
    } else if id == "idiom" || id == IDIOM_EXAMPLE_ID {
        Ok("idiom")
    } else if id == "now-playing" || id == NOW_PLAYING_EXAMPLE_ID {
        Ok("now-playing")
    } else if id == "file-search" || id == FILE_SEARCH_EXAMPLE_ID {
        Ok("file-search")
    } else if id == "sysmon" || id == SYSMON_EXAMPLE_ID {
        Ok("sysmon")
    } else if id == "excalidraw" || id == EXCALIDRAW_EXAMPLE_ID {
        Ok("excalidraw")
    } else if id == "world-clock" || id == WORLD_CLOCK_EXAMPLE_ID {
        Ok("world-clock")
    } else {
        Err(format!("unknown example plugin: {example_id}"))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSurfacePreview {
    pub id: String,
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPreviewDto {
    pub id: String,
    pub name: String,
    pub version: String,
    pub capabilities: Vec<String>,
    pub surfaces: Vec<PluginSurfacePreview>,
    pub network_hosts: Vec<String>,
    pub description: Option<String>,
}

fn surfaces_from_manifest(manifest: &Value) -> Vec<PluginSurfacePreview> {
    let slots = manifest.get("slots");
    let entry = manifest.get("entry");
    let mut out = Vec::new();

    if let Some(sc) = slots.and_then(|s| s.get("shortcuts")) {
        let label = sc
            .get("label")
            .and_then(|x| x.as_str())
            .unwrap_or("快捷区");
        let action = sc.get("action").and_then(|x| x.as_str()).unwrap_or("expand");
        let has_popup_entry = entry.and_then(|e| e.get("popup")).is_some();
        let has_shortcuts_entry = entry.and_then(|e| e.get("shortcuts")).is_some();
        let mut detail = format!("入口「{label}」");
        if has_shortcuts_entry {
            detail.push_str(" · 自画快捷条");
        }
        match action {
            "popup.open" => detail.push_str(if has_popup_entry {
                " · 点击打开弹窗"
            } else {
                " · 声明 popup.open（缺 entry.popup）"
            }),
            "panel.open" => detail.push_str(" · 点击打开岛面板"),
            "command" => detail.push_str(" · 自定义命令"),
            _ => detail.push_str(" · 展开/激活"),
        }
        out.push(PluginSurfacePreview {
            id: "shortcuts".into(),
            label: "快捷区".into(),
            detail,
        });
    }

    if entry.and_then(|e| e.get("popup")).is_some()
        && !out.iter().any(|s| s.id == "shortcuts" && s.detail.contains("弹窗"))
    {
        // Standalone popup without shortcuts slot, or shortcuts without popup.open
        if slots.and_then(|s| s.get("shortcuts")).is_none() {
            out.push(PluginSurfacePreview {
                id: "popup".into(),
                label: "托管弹窗".into(),
                detail: "有 entry.popup，可由其它入口打开".into(),
            });
        } else if slots
            .and_then(|s| s.get("shortcuts"))
            .and_then(|sc| sc.get("action"))
            .and_then(|a| a.as_str())
            != Some("popup.open")
        {
            out.push(PluginSurfacePreview {
                id: "popup".into(),
                label: "托管弹窗".into(),
                detail: "有 entry.popup（快捷区 action 非 popup.open）".into(),
            });
        }
    }

    if slots.and_then(|s| s.get("island.notify")).is_some() {
        out.push(PluginSurfacePreview {
            id: "island.notify".into(),
            label: "灵动岛通知".into(),
            detail: "可推送岛栏通知横幅".into(),
        });
    }
    if slots.and_then(|s| s.get("island.bar")).is_some() {
        out.push(PluginSurfacePreview {
            id: "island.bar".into(),
            label: "灵动岛摘要栏".into(),
            detail: "可占用折叠岛中间摘要文案".into(),
        });
    }
    if slots.and_then(|s| s.get("island.scenario")).is_some() {
        out.push(PluginSurfacePreview {
            id: "island.scenario".into(),
            label: "情景临时".into(),
            detail: "健康时可暂代岛栏与下拉，结束后归还常驻/下拉设置".into(),
        });
    }
    if slots.and_then(|s| s.get("island.drop")).is_some() {
        out.push(PluginSurfacePreview {
            id: "island.drop".into(),
            label: "灵动岛拖放".into(),
            detail: "可接收拖到岛上的文件/文字".into(),
        });
    }
    if let Some(panel) = slots.and_then(|s| s.get("island.panel")) {
        let excl = panel
            .get("excludeFromPullContent")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        let detail = if excl {
            "岛下拉面板（不出现在「下拉内容」列表；会话/拖放打开）".into()
        } else {
            "岛下拉面板（可选入「下拉内容」）".into()
        };
        out.push(PluginSurfacePreview {
            id: "island.panel".into(),
            label: "灵动岛下拉".into(),
            detail,
        });
    }

    if out.is_empty() {
        out.push(PluginSurfacePreview {
            id: "none".into(),
            label: "未声明 UI 表面".into(),
            detail: "仅 capabilities / 后台能力".into(),
        });
    }
    out
}

fn preview_from_manifest(manifest: Value) -> Result<PluginPreviewDto, String> {
    let (id, name, version, capabilities) = validate_manifest(&manifest)?;
    let network_hosts = manifest
        .get("permissions")
        .and_then(|p| p.get("network"))
        .and_then(|n| n.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let description = manifest
        .get("description")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    let surfaces = surfaces_from_manifest(&manifest);
    Ok(PluginPreviewDto {
        id,
        name,
        version,
        capabilities,
        surfaces,
        network_hosts,
        description,
    })
}

fn read_manifest_from_path(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Err("path does not exist".into());
    }
    if path.is_dir() {
        return read_manifest_file(path);
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext != "whpx" && ext != "zip" {
        return Err("expected .whpx / .zip or a directory".into());
    }
    let staging = std::env::temp_dir().join(format!(
        "window-hub-preview-{}-{}",
        std::process::id(),
        now_ms_preview()
    ));
    let _ = fs::remove_dir_all(&staging);
    extract_whpx(path, &staging)?;
    let manifest_dir = if staging.join("plugin.json").is_file() {
        staging.clone()
    } else {
        let mut found = None;
        if let Ok(entries) = fs::read_dir(&staging) {
            for entry in entries.flatten() {
                if entry.path().join("plugin.json").is_file() {
                    found = Some(entry.path());
                    break;
                }
            }
        }
        match found {
            Some(p) => p,
            None => {
                let _ = fs::remove_dir_all(&staging);
                return Err("plugin.json not found in archive".into());
            }
        }
    };
    let result = read_manifest_file(&manifest_dir);
    let _ = fs::remove_dir_all(&staging);
    result
}

fn now_ms_preview() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Read plugin.json from .whpx / directory without installing.
#[tauri::command]
pub fn preview_plugin_from_path(path: String) -> Result<PluginPreviewDto, String> {
    let manifest = read_manifest_from_path(Path::new(path.trim()))?;
    preview_from_manifest(manifest)
}

/// Preview a bundled example before import.
#[tauri::command]
pub fn preview_example_plugin(
    app: AppHandle,
    example_id: String,
) -> Result<PluginPreviewDto, String> {
    let folder = example_folder(&example_id)?;
    let src = resolve_example_plugin_dir(&app, folder)?;
    let manifest = read_manifest_file(&src)?;
    preview_from_manifest(manifest)
}

/// Import a bundled example via the same install path as any .whpx / directory.
/// Does not overwrite on every app launch — only when the user invokes this.
#[tauri::command]
pub fn install_example_plugin(
    app: AppHandle,
    example_id: String,
) -> Result<InstalledPluginRecord, String> {
    let folder = example_folder(&example_id)?;
    let src = resolve_example_plugin_dir(&app, folder)?;
    install_from_dir(&app, &src, false)
}

/// Pack a plugin directory into .whpx (zip).
#[tauri::command]
pub fn pack_plugin_directory(dir: String, out_path: Option<String>) -> Result<String, String> {
    let src = PathBuf::from(&dir);
    if !src.join("plugin.json").is_file() {
        return Err("directory must contain plugin.json".into());
    }
    // Fail at pack time instead of producing an .whpx that cannot be installed.
    read_manifest_file(&src)?;
    let out = match out_path {
        Some(p) => PathBuf::from(p),
        None => {
            let name = src
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("plugin");
            src.parent()
                .unwrap_or(Path::new("."))
                .join(format!("{name}.whpx"))
        }
    };
    let file = fs::File::create(&out).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    fn add_dir(
        zip: &mut zip::ZipWriter<fs::File>,
        opts: zip::write::SimpleFileOptions,
        base: &Path,
        dir: &Path,
    ) -> Result<(), String> {
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let rel = path
                .strip_prefix(base)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                let _ = zip.add_directory(format!("{rel}/"), opts);
                add_dir(zip, opts, base, &path)?;
            } else {
                zip.start_file(rel, opts).map_err(|e| e.to_string())?;
                let mut f = fs::File::open(&path).map_err(|e| e.to_string())?;
                let mut buf = Vec::new();
                f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
                zip.write_all(&buf).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    add_dir(&mut zip, opts, &src, &src)?;
    zip.finish().map_err(|e| e.to_string())?;
    Ok(out.to_string_lossy().to_string())
}
