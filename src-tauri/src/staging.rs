//! Plugin-scoped staging — path refs for files; text/image bytes under each plugin's staging/.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

const MAX_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StagingKind {
    File,
    Text,
    Image,
    Folder,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StagingItem {
    pub id: String,
    pub kind: StagingKind,
    pub label: String,
    pub created_at: u64,
    /// File/image/folder path ref (original absolute path), or staging-owned .txt / dropped image bytes.
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StagingSummary {
    pub files: u32,
    pub texts: u32,
    pub images: u32,
    #[serde(default)]
    pub folders: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagingChangedPayload {
    pub plugin_id: String,
    #[serde(flatten)]
    pub summary: StagingSummary,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct IndexFile {
    items: Vec<StagingItem>,
}

struct Store {
    by_plugin: Mutex<HashMap<String, IndexFile>>,
}

static STORE: OnceLock<Store> = OnceLock::new();

fn store() -> &'static Store {
    STORE.get_or_init(|| Store {
        by_plugin: Mutex::new(HashMap::new()),
    })
}

fn plugin_root(plugin_id: &str) -> Result<PathBuf, String> {
    let appdata = std::env::var_os("APPDATA").ok_or_else(|| "APPDATA missing".to_string())?;
    let mut dir = PathBuf::from(appdata);
    dir.push("window-hub");
    dir.push("plugins");
    dir.push(plugin_id);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn staging_dir(plugin_id: &str) -> Result<PathBuf, String> {
    let mut dir = plugin_root(plugin_id)?;
    dir.push("staging");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn kind_str(k: &StagingKind) -> &'static str {
    match k {
        StagingKind::File => "file",
        StagingKind::Text => "text",
        StagingKind::Image => "image",
        StagingKind::Folder => "folder",
    }
}

fn parse_kind(s: &str) -> StagingKind {
    match s {
        "text" => StagingKind::Text,
        "image" => StagingKind::Image,
        "folder" => StagingKind::Folder,
        _ => StagingKind::File,
    }
}

fn to_row(item: &StagingItem) -> crate::db::StagingRow {
    crate::db::StagingRow {
        id: item.id.clone(),
        kind: kind_str(&item.kind).to_string(),
        label: item.label.clone(),
        path: item.path.clone(),
        created_at: item.created_at,
    }
}

fn from_row(r: crate::db::StagingRow) -> StagingItem {
    StagingItem {
        id: r.id,
        kind: parse_kind(&r.kind),
        label: r.label,
        path: r.path,
        created_at: r.created_at,
    }
}

fn try_remove_owned_payload(plugin_id: &str, path: &str) {
    let Ok(dir) = staging_dir(plugin_id) else {
        return;
    };
    let p = Path::new(path);
    if p.starts_with(&dir) {
        let _ = fs::remove_file(p);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn new_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    format!("{:x}-{:x}", now_ms(), SEQ.fetch_add(1, Ordering::Relaxed))
}

fn load_index(plugin_id: &str) -> IndexFile {
    match crate::db::with_conn(|c| crate::db::staging_list(c, plugin_id)) {
        Ok(rows) => IndexFile {
            items: rows.into_iter().map(from_row).collect(),
        },
        Err(_) => IndexFile::default(),
    }
}

fn ensure_loaded(map: &mut HashMap<String, IndexFile>, plugin_id: &str) {
    if !map.contains_key(plugin_id) {
        map.insert(plugin_id.to_string(), load_index(plugin_id));
    }
}

/// Reload in-memory index after DB restore / external mutation.
/// Sources plugin ids from plugin_kv `__staging_items` (no hardcoded plugin id).
pub fn reload_from_db() {
    let mut map = store().by_plugin.lock();
    let mut ids = crate::db::with_conn(|c| crate::db::staging_list_plugin_ids(c)).unwrap_or_default();
    if ids.is_empty() {
        ids = map.keys().cloned().collect();
    }
    map.clear();
    for id in ids {
        map.insert(id.clone(), load_index(&id));
    }
}

/// Plugin ids currently holding a staging index (for restore notifications).
pub fn plugin_ids_with_staging() -> Vec<String> {
    crate::db::with_conn(|c| crate::db::staging_list_plugin_ids(c)).unwrap_or_default()
}

fn emit_changed(app: Option<&AppHandle>, plugin_id: &str) {
    let summary = summary(plugin_id);
    if let Some(app) = app {
        let _ = app.emit(
            "staging-changed",
            StagingChangedPayload {
                plugin_id: plugin_id.to_string(),
                summary,
            },
        );
    }
}

fn summarize(items: &[StagingItem]) -> StagingSummary {
    let mut s = StagingSummary::default();
    for it in items {
        match it.kind {
            StagingKind::File => s.files += 1,
            StagingKind::Text => s.texts += 1,
            StagingKind::Image => s.images += 1,
            StagingKind::Folder => s.folders += 1,
        }
        s.total += 1;
    }
    s
}

/// Strip Windows `\\?\` / `\\?\UNC\` prefixes from canonicalize() for clipboard / Explorer.
fn path_display_string(path: &Path) -> String {
    let s = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{rest}");
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return rest.to_string();
        }
    }
    s.into_owned()
}

pub fn summary(plugin_id: &str) -> StagingSummary {
    let mut map = store().by_plugin.lock();
    ensure_loaded(&mut map, plugin_id);
    summarize(&map.get(plugin_id).map(|i| i.items.as_slice()).unwrap_or(&[]))
}

pub fn list(plugin_id: &str) -> Vec<StagingItem> {
    let mut map = store().by_plugin.lock();
    ensure_loaded(&mut map, plugin_id);
    map.get(plugin_id)
        .map(|i| i.items.clone())
        .unwrap_or_default()
}

fn push_item(
    app: Option<&AppHandle>,
    plugin_id: &str,
    item: StagingItem,
) -> Result<StagingItem, String> {
    let mut map = store().by_plugin.lock();
    ensure_loaded(&mut map, plugin_id);
    let index = map.get_mut(plugin_id).unwrap();
    index.items.insert(0, item.clone());
    if let Err(e) =
        crate::db::with_conn(|c| crate::db::staging_insert(c, plugin_id, &to_row(&item)))
    {
        index.items.remove(0);
        return Err(e);
    }
    drop(map);
    emit_changed(app, plugin_id);
    Ok(item)
}

pub fn add_text(
    app: Option<&AppHandle>,
    plugin_id: &str,
    text: String,
) -> Result<StagingItem, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("empty text".into());
    }
    if text.len() as u64 > MAX_BYTES {
        return Err("text too large (max 50MB)".into());
    }
    let id = new_id();
    let path = staging_dir(plugin_id)?.join(format!("{id}.txt"));
    fs::write(&path, text.as_bytes()).map_err(|e| e.to_string())?;
    let label: String = text.chars().take(24).collect();
    let label = if text.chars().count() > 24 {
        format!("{label}…")
    } else {
        label
    };
    push_item(
        app,
        plugin_id,
        StagingItem {
            id,
            kind: StagingKind::Text,
            label,
            created_at: now_ms(),
            path: path.to_string_lossy().into_owned(),
        },
    )
}

pub fn add_image_bytes(
    app: Option<&AppHandle>,
    plugin_id: &str,
    label: String,
    bytes: Vec<u8>,
    ext: Option<String>,
) -> Result<StagingItem, String> {
    if bytes.is_empty() {
        return Err("empty image".into());
    }
    if bytes.len() as u64 > MAX_BYTES {
        return Err("image too large (max 50MB)".into());
    }
    let id = new_id();
    let ext = ext
        .unwrap_or_else(|| "png".into())
        .trim_start_matches('.')
        .to_ascii_lowercase();
    let ext = if ext.is_empty() {
        "png".to_string()
    } else {
        ext
    };
    let path = staging_dir(plugin_id)?.join(format!("{id}.{ext}"));
    fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    let label = if label.trim().is_empty() {
        format!("图片.{ext}")
    } else {
        label.trim().to_string()
    };
    push_item(
        app,
        plugin_id,
        StagingItem {
            id,
            kind: StagingKind::Image,
            label,
            created_at: now_ms(),
            path: path.to_string_lossy().into_owned(),
        },
    )
}

/// Prefer Explorer-style absolute paths as-is; `canonicalize` is expensive
/// (AV / network) and unnecessary for typical CF_HDROP payloads.
fn resolve_drop_path(src: PathBuf) -> PathBuf {
    let s = src.to_string_lossy();
    let needs_canon = !src.is_absolute()
        || s.contains("..")
        || s.starts_with(r"\\?\");
    if needs_canon {
        fs::canonicalize(&src).unwrap_or(src)
    } else {
        src
    }
}

/// Store absolute path reference only — do not copy into staging/.
/// Accepts regular files and directories (`folder` kind).
///
/// Batches: one DB write + one `staging-changed` for the whole drop
/// (avoids UI freeze when dragging many files).
pub fn add_paths(
    app: Option<&AppHandle>,
    plugin_id: &str,
    paths: Vec<String>,
) -> Result<Vec<StagingItem>, String> {
    // FS resolve outside the store lock so we don't block other staging ops.
    let mut candidates: Vec<(String, String, StagingKind)> = Vec::new();
    for p in paths {
        let src = PathBuf::from(&p);
        let meta = match fs::symlink_metadata(&src) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let is_dir = meta.is_dir();
        let is_file = meta.is_file();
        if !is_file && !is_dir {
            continue;
        }
        let abs = resolve_drop_path(src);
        let path_str = path_display_string(&abs);
        let name = abs
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(if is_dir { "folder" } else { "file" })
            .to_string();
        let kind = if is_dir {
            StagingKind::Folder
        } else if is_image_name(&name) {
            StagingKind::Image
        } else {
            StagingKind::File
        };
        candidates.push((path_str, name, kind));
    }

    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let mut map = store().by_plugin.lock();
    ensure_loaded(&mut map, plugin_id);
    let index = map.get_mut(plugin_id).unwrap();

    let mut out = Vec::new();
    let mut seen_new: Vec<String> = Vec::new();
    for (path_str, name, kind) in candidates {
        let dup_existing = index.items.iter().any(|it| {
            it.kind != StagingKind::Text && it.path.eq_ignore_ascii_case(&path_str)
        });
        let dup_batch = seen_new
            .iter()
            .any(|p| p.eq_ignore_ascii_case(&path_str));
        if dup_existing || dup_batch {
            continue;
        }
        seen_new.push(path_str.clone());
        out.push(StagingItem {
            id: new_id(),
            kind,
            label: name,
            created_at: now_ms(),
            path: path_str,
        });
    }

    if out.is_empty() {
        return Ok(out);
    }

    // Preserve prior per-item insert(0) order: last path ends up on top.
    let mut prepended = out.clone();
    prepended.reverse();
    prepended.append(&mut index.items);
    index.items = prepended;

    let rows: Vec<_> = index.items.iter().map(to_row).collect();
    if let Err(e) = crate::db::with_conn(|c| crate::db::staging_replace_all(c, plugin_id, &rows)) {
        // Roll back memory to last good DB snapshot.
        *index = load_index(plugin_id);
        return Err(e);
    }
    drop(map);
    emit_changed(app, plugin_id);
    Ok(out)
}

fn is_image_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
        || lower.ends_with(".webp")
        || lower.ends_with(".bmp")
}

pub fn remove(app: Option<&AppHandle>, plugin_id: &str, id: &str) -> Result<(), String> {
    remove_many(app, plugin_id, &[id.to_string()]).map(|_| ())
}

/// Batch remove — one DB write + one `staging-changed`.
pub fn remove_many(
    app: Option<&AppHandle>,
    plugin_id: &str,
    ids: &[String],
) -> Result<u32, String> {
    if ids.is_empty() {
        return Ok(0);
    }
    let id_set: std::collections::HashSet<&str> = ids.iter().map(|s| s.as_str()).collect();
    let mut map = store().by_plugin.lock();
    ensure_loaded(&mut map, plugin_id);
    let index = map.get_mut(plugin_id).unwrap();
    let mut kept = Vec::with_capacity(index.items.len());
    let mut removed = 0u32;
    for it in index.items.drain(..) {
        if id_set.contains(it.id.as_str()) {
            try_remove_owned_payload(plugin_id, &it.path);
            removed += 1;
        } else {
            kept.push(it);
        }
    }
    if removed == 0 {
        index.items = kept;
        return Err("item not found".into());
    }
    index.items = kept;
    let rows: Vec<_> = index.items.iter().map(to_row).collect();
    crate::db::with_conn(|c| crate::db::staging_replace_all(c, plugin_id, &rows))?;
    drop(map);
    emit_changed(app, plugin_id);
    Ok(removed)
}

pub fn clear(app: Option<&AppHandle>, plugin_id: &str) -> Result<(), String> {
    let mut map = store().by_plugin.lock();
    ensure_loaded(&mut map, plugin_id);
    let index = map.get_mut(plugin_id).unwrap();
    for it in index.items.drain(..) {
        try_remove_owned_payload(plugin_id, &it.path);
    }
    crate::db::with_conn(|c| crate::db::staging_clear(c, plugin_id))?;
    drop(map);
    emit_changed(app, plugin_id);
    Ok(())
}

pub fn reveal(plugin_id: &str, id: &str) -> Result<(), String> {
    let mut map = store().by_plugin.lock();
    ensure_loaded(&mut map, plugin_id);
    let item = map
        .get(plugin_id)
        .and_then(|idx| idx.items.iter().find(|i| i.id == id))
        .ok_or_else(|| "item not found".to_string())?;
    let path = item.path.clone();
    drop(map);
    // Same Explorer /select quoting rules as sousou (see open::reveal_in_folder).
    crate::sousou::open::reveal_in_folder(&path)
}

/// Open / run with the default shell association (exe/bat run, docs open, folders browse).
pub fn open_item(plugin_id: &str, id: &str) -> Result<(), String> {
    let mut map = store().by_plugin.lock();
    ensure_loaded(&mut map, plugin_id);
    let item = map
        .get(plugin_id)
        .and_then(|idx| idx.items.iter().find(|i| i.id == id))
        .ok_or_else(|| "item not found".to_string())?
        .clone();
    drop(map);

    let path = item.path.trim();
    if path.is_empty() {
        return Err("item has no path".into());
    }
    let path_buf = PathBuf::from(path);
    if !path_buf.exists() {
        return Err(format!("path missing: {path}"));
    }

    #[cfg(windows)]
    {
        // `start "" <path>`：可靠打开/运行；空标题避免路径被当成窗口标题。
        // 工作目录设为文件所在目录，方便 bat/exe 找相对资源。
        // 不要 CREATE_NO_WINDOW：.bat/.cmd 需要控制台窗口。
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "start", "", path]);
        if let Some(dir) = path_buf.parent() {
            if !dir.as_os_str().is_empty() {
                cmd.current_dir(dir);
            }
        }
        cmd.spawn().map_err(|e| format!("open failed: {e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = item;
        Err("open staging item is Windows-only".into())
    }
}

pub fn paths_for_drag(plugin_id: &str, ids: &[String]) -> Result<Vec<PathBuf>, String> {
    let mut map = store().by_plugin.lock();
    ensure_loaded(&mut map, plugin_id);
    let guard = map.get(plugin_id).unwrap();
    let mut out = Vec::new();
    for id in ids {
        let Some(item) = guard.items.iter().find(|i| i.id == *id) else {
            continue;
        };
        if matches!(item.kind, StagingKind::Text) {
            continue;
        }
        let p = PathBuf::from(&item.path);
        if !p.is_file() && !p.is_dir() {
            continue;
        }
        out.push(fs::canonicalize(&p).unwrap_or(p));
    }
    if out.is_empty() {
        return Err("no files to drag".into());
    }
    Ok(out)
}

const DRAG_PREVIEW_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

pub fn start_drag_out(
    window: &tauri::WebviewWindow,
    plugin_id: &str,
    ids: &[String],
) -> Result<(), String> {
    let paths = paths_for_drag(plugin_id, ids)?;
    let preview = {
        let first = &paths[0];
        let name = first
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if is_image_name(&name) {
            drag::Image::File(first.clone())
        } else {
            drag::Image::Raw(DRAG_PREVIEW_PNG.to_vec())
        }
    };
    drag::start_drag(
        window,
        drag::DragItem::Files(paths),
        preview,
        |_result, _pos| {},
        drag::Options {
            mode: drag::DragMode::Copy,
            skip_animatation_on_cancel_or_failure: true,
        },
    )
    .map_err(|e| e.to_string())
}

#[cfg(windows)]
pub fn copy_to_clipboard(plugin_id: &str, id: &str) -> Result<(), String> {
    let mut map = store().by_plugin.lock();
    ensure_loaded(&mut map, plugin_id);
    let item = map
        .get(plugin_id)
        .and_then(|idx| idx.items.iter().find(|i| i.id == id))
        .cloned()
        .ok_or_else(|| "item not found".to_string())?;
    drop(map);
    let text = match item.kind {
        StagingKind::Text => fs::read_to_string(&item.path).map_err(|e| e.to_string())?,
        StagingKind::File | StagingKind::Image | StagingKind::Folder => item.path,
    };
    set_clipboard_text(&text)
}

#[cfg(windows)]
pub fn copy_all_paths(plugin_id: &str) -> Result<u32, String> {
    let mut map = store().by_plugin.lock();
    ensure_loaded(&mut map, plugin_id);
    let paths: Vec<String> = map
        .get(plugin_id)
        .map(|idx| {
            idx.items
                .iter()
                .filter(|i| {
                    matches!(
                        i.kind,
                        StagingKind::File | StagingKind::Image | StagingKind::Folder
                    )
                })
                .map(|i| i.path.clone())
                .collect()
        })
        .unwrap_or_default();
    drop(map);
    if paths.is_empty() {
        return Err("no file paths".into());
    }
    let n = paths.len() as u32;
    set_clipboard_text(&paths.join("\n"))?;
    Ok(n)
}

#[cfg(not(windows))]
pub fn copy_all_paths(_plugin_id: &str) -> Result<u32, String> {
    Err("clipboard only on Windows".into())
}

/// Decode full image pixels only below this size (popup cards are ~56×40).
const IMAGE_THUMB_MAX_BYTES: u64 = 2 * 1024 * 1024;

fn shell_thumb_cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Cheap 32px shell icon with path cache — staging UI is small; avoid 256px factory storms.
fn cached_shell_thumb_data_url(path: &str) -> Option<String> {
    let key = path.to_ascii_lowercase();
    {
        let cache = shell_thumb_cache().lock();
        if let Some(hit) = cache.get(&key) {
            return hit.clone();
        }
    }
    let data = crate::dock::resolve_small_icon_png(path)
        .map(|b64| format!("data:image/png;base64,{b64}"));
    let mut cache = shell_thumb_cache().lock();
    if cache.len() > 512 {
        cache.clear();
    }
    cache.insert(key, data.clone());
    data
}

pub fn thumb_data_url(plugin_id: &str, id: &str) -> Result<Option<String>, String> {
    let mut map = store().by_plugin.lock();
    ensure_loaded(&mut map, plugin_id);
    let item = map
        .get(plugin_id)
        .and_then(|idx| idx.items.iter().find(|i| i.id == id))
        .cloned()
        .ok_or_else(|| "item not found".to_string())?;
    drop(map);
    match item.kind {
        StagingKind::Image => {
            let meta_len = fs::metadata(&item.path).map(|m| m.len()).unwrap_or(0);
            // Large images: shell icon instead of full decode (multi-drop freeze root cause).
            if meta_len == 0 || meta_len > IMAGE_THUMB_MAX_BYTES {
                return Ok(cached_shell_thumb_data_url(&item.path));
            }
            let bytes = fs::read(&item.path).map_err(|e| e.to_string())?;
            if bytes.is_empty() {
                return Ok(None);
            }
            let img = match image::load_from_memory(&bytes) {
                Ok(i) => i,
                Err(_) => return Ok(cached_shell_thumb_data_url(&item.path)),
            };
            let thumb = img.thumbnail(96, 96);
            let mut out = Vec::new();
            thumb
                .write_to(
                    &mut std::io::Cursor::new(&mut out),
                    image::ImageFormat::Png,
                )
                .map_err(|e| e.to_string())?;
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&out);
            Ok(Some(format!("data:image/png;base64,{b64}")))
        }
        StagingKind::File | StagingKind::Folder => Ok(cached_shell_thumb_data_url(&item.path)),
        StagingKind::Text => Ok(None),
    }
}

#[cfg(windows)]
fn set_clipboard_text(text: &str) -> Result<(), String> {
    use windows::Win32::Foundation::{HANDLE, HWND};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

    const CF_UNICODETEXT: u32 = 13;

    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = wide.len() * 2;
    unsafe {
        OpenClipboard(HWND::default()).map_err(|e| e.to_string())?;
        let _ = EmptyClipboard();
        let hmem = GlobalAlloc(GMEM_MOVEABLE, bytes).map_err(|e| e.to_string())?;
        let ptr = GlobalLock(hmem) as *mut u16;
        if ptr.is_null() {
            let _ = CloseClipboard();
            return Err("GlobalLock failed".into());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
        let _ = GlobalUnlock(hmem);
        if SetClipboardData(CF_UNICODETEXT, HANDLE(hmem.0 as _)).is_err() {
            let _ = CloseClipboard();
            return Err("SetClipboardData failed".into());
        }
        let _ = CloseClipboard();
    }
    Ok(())
}

/// Put file paths on the clipboard as `CF_HDROP` so Explorer paste works.
/// Accepts one or more staging item ids (file / image / folder).
#[cfg(windows)]
pub fn copy_files_to_clipboard(plugin_id: &str, ids: &[String]) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::{HANDLE, HWND, POINT};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::UI::Shell::DROPFILES;

    const CF_HDROP: u32 = 15;

    if ids.is_empty() {
        return Err("no items".into());
    }

    let mut map = store().by_plugin.lock();
    ensure_loaded(&mut map, plugin_id);
    let guard = map.get(plugin_id).ok_or_else(|| "item not found".to_string())?;
    let mut paths: Vec<PathBuf> = Vec::new();
    for id in ids {
        let Some(item) = guard.items.iter().find(|i| i.id == *id) else {
            continue;
        };
        if !matches!(
            item.kind,
            StagingKind::File | StagingKind::Image | StagingKind::Folder
        ) {
            continue;
        }
        let path = PathBuf::from(&item.path);
        if path.exists() {
            paths.push(path);
        }
    }
    drop(map);

    if paths.is_empty() {
        return Err("no files to copy".into());
    }

    let mut path_wide: Vec<u16> = Vec::new();
    for path in &paths {
        path_wide.extend(path.as_os_str().encode_wide());
        path_wide.push(0);
    }
    path_wide.push(0); // double-null terminator for HDROP list

    let header_size = std::mem::size_of::<DROPFILES>();
    let total = header_size + path_wide.len() * 2;
    unsafe {
        OpenClipboard(HWND::default()).map_err(|e| e.to_string())?;
        let _ = EmptyClipboard();
        let hmem = GlobalAlloc(GMEM_MOVEABLE, total).map_err(|e| e.to_string())?;
        let ptr = GlobalLock(hmem) as *mut u8;
        if ptr.is_null() {
            let _ = CloseClipboard();
            return Err("GlobalLock failed".into());
        }
        let dropfiles = DROPFILES {
            pFiles: header_size as u32,
            pt: POINT { x: 0, y: 0 },
            fNC: windows::Win32::Foundation::BOOL(0),
            fWide: windows::Win32::Foundation::BOOL(1),
        };
        std::ptr::copy_nonoverlapping(
            &dropfiles as *const DROPFILES as *const u8,
            ptr,
            header_size,
        );
        std::ptr::copy_nonoverlapping(
            path_wide.as_ptr() as *const u8,
            ptr.add(header_size),
            path_wide.len() * 2,
        );
        let _ = GlobalUnlock(hmem);
        if SetClipboardData(CF_HDROP, HANDLE(hmem.0 as _)).is_err() {
            let _ = CloseClipboard();
            return Err("SetClipboardData CF_HDROP failed".into());
        }
        let _ = CloseClipboard();
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn copy_files_to_clipboard(_plugin_id: &str, _ids: &[String]) -> Result<(), String> {
    Err("clipboard only on Windows".into())
}

/// Copy selected file/folder/image paths as newline-joined text.
#[cfg(windows)]
pub fn copy_selected_paths(plugin_id: &str, ids: &[String]) -> Result<u32, String> {
    if ids.is_empty() {
        return Err("no items".into());
    }
    let id_set: std::collections::HashSet<&str> = ids.iter().map(|s| s.as_str()).collect();
    let mut map = store().by_plugin.lock();
    ensure_loaded(&mut map, plugin_id);
    let paths: Vec<String> = map
        .get(plugin_id)
        .map(|idx| {
            idx.items
                .iter()
                .filter(|i| {
                    id_set.contains(i.id.as_str())
                        && matches!(
                            i.kind,
                            StagingKind::File | StagingKind::Image | StagingKind::Folder
                        )
                })
                .map(|i| i.path.clone())
                .collect()
        })
        .unwrap_or_default();
    drop(map);
    if paths.is_empty() {
        return Err("no file paths".into());
    }
    let n = paths.len() as u32;
    set_clipboard_text(&paths.join("\n"))?;
    Ok(n)
}

#[cfg(not(windows))]
pub fn copy_selected_paths(_plugin_id: &str, _ids: &[String]) -> Result<u32, String> {
    Err("clipboard only on Windows".into())
}

#[cfg(not(windows))]
pub fn copy_to_clipboard(_plugin_id: &str, _id: &str) -> Result<(), String> {
    Err("clipboard only on Windows".into())
}
