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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPluginRecord {
    pub id: String,
    pub name: String,
    pub version: String,
    pub path: String,
    pub enabled: bool,
    pub is_dev: bool,
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

fn read_manifest_file(dir: &Path) -> Result<Value, String> {
    let path = dir.join("plugin.json");
    let text = fs::read_to_string(&path).map_err(|e| format!("read plugin.json: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("parse plugin.json: {e}"))
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
    load_registry()
        .plugins
        .into_iter()
        .find(|p| p.id == id)
}

fn close_plugin_popup(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("plugin-popup") {
        let _ = win.close();
        let _ = app.emit("plugin-popup-closed", ());
    }
}

pub fn list_installed_plugins_sync() -> Vec<InstalledPluginRecord> {
    // Transfer-station / window-groups: normal plugins (no embed seed / no overwrite-on-launch).
    load_registry().plugins
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
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| e.to_string())?;
    }
    copy_dir_recursive(src, &dest)?;
    // rewrite manifest with possibly rewritten id
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(dest.join("plugin.json"), text).map_err(|e| e.to_string())?;

    let record = InstalledPluginRecord {
        id: install_id,
        name,
        version,
        path: dest.to_string_lossy().to_string(),
        enabled: true,
        is_dev,
        capabilities,
        manifest,
    };
    let mut reg = load_registry();
    upsert_record(&mut reg, record.clone());
    save_registry(&reg)?;
    emit_plugins(app, &reg);
    Ok(record)
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
    if path.exists() {
        fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
    }
    close_plugin_popup(&app);
    let _ = crate::db::with_conn(|c| crate::db::plugin_clear_all(c, &id));
    pins.inner_remove(&id);
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
    save_registry(&reg)?;
    if !enabled {
        close_plugin_popup(&app);
        pins.inner_remove(&id);
    } else {
        pins.reload_plugin(&id);
        crate::companion_scripts::start_launchers_for_plugin(&id);
    }
    let _ = app.emit("shortcuts-pins-changed", pins.all_flat());
    emit_plugins(&app, &reg);
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

/// Import a bundled example via the same install path as any .whpx / directory.
/// Does not overwrite on every app launch — only when the user invokes this.
#[tauri::command]
pub fn install_example_plugin(
    app: AppHandle,
    example_id: String,
) -> Result<InstalledPluginRecord, String> {
    let id = example_id.trim();
    let folder = if id == "window-groups" || id == WINDOW_GROUPS_EXAMPLE_ID {
        "window-groups"
    } else if id == "transfer-station" || id == TRANSFER_EXAMPLE_ID {
        "transfer-station"
    } else {
        return Err(format!("unknown example plugin: {example_id}"));
    };
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
