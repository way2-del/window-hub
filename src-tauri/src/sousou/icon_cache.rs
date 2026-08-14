//! Disk + memory cache for sousou launcher icons (PNG base64).
//!
//! Keys include path + mtime so a rebuilt exe gets a fresh extract.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::UNIX_EPOCH;

const MEM_CAP: usize = 400;

static MEM: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IconCacheStats {
    pub entries: usize,
    pub bytes: u64,
}

fn cache_dir() -> Result<PathBuf, String> {
    let dir = crate::db::app_data_root()?.join("sousou-icon-cache");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn normalize_path(path: &str) -> String {
    path.trim().replace('/', "\\").to_lowercase()
}

fn file_mtime_secs(path: &str) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn cache_key(path: &str) -> String {
    let norm = normalize_path(path);
    let mtime = file_mtime_secs(path);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    norm.hash(&mut h);
    mtime.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn disk_path(key: &str) -> Result<PathBuf, String> {
    Ok(cache_dir()?.join(format!("{key}.png")))
}

fn mem_get(key: &str) -> Option<String> {
    MEM.lock().get(key).cloned()
}

fn mem_put(key: String, b64: String) {
    let mut g = MEM.lock();
    if g.len() >= MEM_CAP && !g.contains_key(&key) {
        let drop_n = MEM_CAP / 2;
        let keys: Vec<String> = g.keys().take(drop_n).cloned().collect();
        for k in keys {
            g.remove(&k);
        }
    }
    g.insert(key, b64);
}

fn disk_get(key: &str) -> Option<String> {
    let p = disk_path(key).ok()?;
    let bytes = fs::read(p).ok()?;
    if bytes.is_empty() {
        return None;
    }
    Some(B64.encode(bytes))
}

fn disk_put(key: &str, b64: &str) {
    let Ok(bytes) = B64.decode(b64) else {
        return;
    };
    let Ok(p) = disk_path(key) else {
        return;
    };
    let _ = fs::write(p, bytes);
}

/// Resolve launcher icon with memory → disk → Shell extract.
pub fn get_or_resolve(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    let key = cache_key(path);
    if let Some(hit) = mem_get(&key) {
        return Some(hit);
    }
    if let Some(hit) = disk_get(&key) {
        mem_put(key, hit.clone());
        return Some(hit);
    }
    let b64 = crate::dock::resolve_launcher_icon_png(path)?;
    disk_put(&key, &b64);
    mem_put(key, b64.clone());
    Some(b64)
}

pub fn stats() -> IconCacheStats {
    let mut entries = 0usize;
    let mut bytes = 0u64;
    if let Ok(dir) = cache_dir() {
        if let Ok(rd) = fs::read_dir(dir) {
            for ent in rd.flatten() {
                let p = ent.path();
                if p.extension().and_then(|e| e.to_str()) != Some("png") {
                    continue;
                }
                entries += 1;
                if let Ok(meta) = ent.metadata() {
                    bytes = bytes.saturating_add(meta.len());
                }
            }
        }
    }
    let mem_n = MEM.lock().len();
    if mem_n > entries {
        entries = mem_n;
    }
    IconCacheStats { entries, bytes }
}

pub fn clear() -> Result<IconCacheStats, String> {
    MEM.lock().clear();
    let dir = cache_dir()?;
    if dir.is_dir() {
        for ent in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let ent = ent.map_err(|e| e.to_string())?;
            let p = ent.path();
            if p.extension().and_then(|e| e.to_str()) != Some("png") {
                continue;
            }
            let _ = fs::remove_file(&p);
        }
    }
    Ok(IconCacheStats {
        entries: 0,
        bytes: 0,
    })
}
