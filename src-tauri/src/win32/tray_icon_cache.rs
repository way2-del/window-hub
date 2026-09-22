//! Persistent tray glyph cache — keyed by reboot-stable identity (pin_key / process).
//!
//! Hot-path rules (avoid 未响应):
//! - Lookups are memory-only and clone only on hit for empty PNGs.
//! - Writes are rare (new glyph only), keyed by content hash (no multi-copy base64).
//! - Disk flush is debounced on a background thread; never from publish/list_meta.

#![cfg(windows)]

use parking_lot::Mutex;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

#[derive(Default)]
struct CacheState {
    /// content hash → png base64 (single copy per unique image)
    blobs: HashMap<u64, String>,
    /// stable key → content hash
    keys: HashMap<String, u64>,
}

static MEM: OnceLock<Mutex<CacheState>> = OnceLock::new();
static DIRTY: AtomicBool = AtomicBool::new(false);
static FLUSH_SCHEDULED: AtomicBool = AtomicBool::new(false);

fn mem() -> &'static Mutex<CacheState> {
    MEM.get_or_init(|| Mutex::new(CacheState::default()))
}

fn cache_dir() -> Option<PathBuf> {
    let root = crate::db::app_data_root().ok()?;
    let dir = root.join("tray-icon-cache");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn index_path() -> Option<PathBuf> {
    Some(cache_dir()?.join("index-v2.json"))
}

fn sanitize_key(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

fn hash_b64(png: &str) -> u64 {
    let mut h = DefaultHasher::new();
    png.hash(&mut h);
    h.finish()
}

/// Keys used to look up / store a glyph for one tray row.
pub fn keys_for(pin_key: &str, process: &str, id: &str) -> Vec<String> {
    let mut out = Vec::with_capacity(4);
    let mut push = |s: String| {
        let s = sanitize_key(&s);
        if s.is_empty() || s == "未知应用" {
            return;
        }
        if !out.iter().any(|e| e == &s) {
            out.push(s);
        }
    };
    let pk = pin_key.trim();
    if !pk.is_empty() {
        push(pk.to_string());
        if let Some(rest) = pk.strip_prefix("exe:") {
            if let Some(colon) = rest.rfind(':') {
                let path = &rest[..colon];
                let uid = &rest[colon + 1..];
                if let Some(stem) = std::path::Path::new(path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                {
                    push(format!("proc:{stem}:{uid}"));
                    push(stem.to_string());
                }
            }
        }
        if let Some(rest) = pk.strip_prefix("proc:") {
            if let Some((stem, _)) = rest.rsplit_once(':') {
                push(stem.to_string());
            }
        }
    }
    let proc = process.trim();
    if !proc.is_empty() {
        push(format!("proc:{proc}"));
        push(proc.to_string());
    }
    let id = id.trim();
    if !id.is_empty() && !id.contains(':') {
        push(id.to_string());
    }
    out
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct DiskIndex {
    /// key → hash as hex string
    keys: HashMap<String, String>,
    /// hash hex → png base64
    blobs: HashMap<String, String>,
}

/// Load index into memory once.
pub fn load() {
    let Some(path) = index_path() else {
        return;
    };
    // Migrate / ignore legacy v1 (was unbounded duplicated base64 — can freeze load).
    let _ = std::fs::remove_file(cache_dir().map(|d| d.join("index.json")).unwrap_or_default());

    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    // Hard cap: refuse monstrous files that would hang the process.
    if text.len() > 12 * 1024 * 1024 {
        eprintln!(
            "[tray] icon cache index too large ({} bytes) — deleting",
            text.len()
        );
        let _ = std::fs::remove_file(&path);
        return;
    }
    let Ok(disk) = serde_json::from_str::<DiskIndex>(&text) else {
        return;
    };
    let mut guard = mem().lock();
    for (hex, b64) in disk.blobs {
        if b64.len() < 32 {
            continue;
        }
        if let Ok(h) = u64::from_str_radix(&hex, 16) {
            guard.blobs.insert(h, b64);
        }
    }
    for (k, hex) in disk.keys {
        if let Ok(h) = u64::from_str_radix(&hex, 16) {
            if guard.blobs.contains_key(&h) {
                guard.keys.insert(sanitize_key(&k), h);
            }
        }
    }
    eprintln!(
        "[tray] icon cache loaded keys={} blobs={}",
        guard.keys.len(),
        guard.blobs.len()
    );
}

fn schedule_flush() {
    DIRTY.store(true, Ordering::SeqCst);
    if FLUSH_SCHEDULED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("tray-icon-cache-flush".into())
        .spawn(|| {
            std::thread::sleep(Duration::from_secs(5));
            FLUSH_SCHEDULED.store(false, Ordering::SeqCst);
            flush_now();
        })
        .ok();
}

fn flush_now() {
    if !DIRTY.swap(false, Ordering::SeqCst) {
        return;
    }
    let Some(path) = index_path() else {
        return;
    };
    let disk = {
        let guard = mem().lock();
        let mut keys = HashMap::new();
        let mut blobs = HashMap::new();
        // Cap: keep at most 120 unique glyphs.
        let mut blob_list: Vec<(u64, &String)> = guard.blobs.iter().map(|(h, b)| (*h, b)).collect();
        blob_list.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
        blob_list.truncate(120);
        let keep: std::collections::HashSet<u64> = blob_list.iter().map(|(h, _)| *h).collect();
        for (h, b) in blob_list {
            blobs.insert(format!("{h:016x}"), b.clone());
        }
        for (k, h) in &guard.keys {
            if keep.contains(h) {
                keys.insert(k.clone(), format!("{h:016x}"));
            }
        }
        DiskIndex { keys, blobs }
    };
    let Ok(text) = serde_json::to_string(&disk) else {
        DIRTY.store(true, Ordering::SeqCst);
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, text.as_bytes()).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    } else {
        DIRTY.store(true, Ordering::SeqCst);
    }
}

/// Lookup first hit among keys (clones one blob on hit).
pub fn get(keys: &[String]) -> Option<String> {
    if keys.is_empty() {
        return None;
    }
    let guard = mem().lock();
    for k in keys {
        if let Some(h) = guard.keys.get(k) {
            if let Some(v) = guard.blobs.get(h) {
                if v.len() >= 32 {
                    return Some(v.clone());
                }
            }
        }
    }
    None
}

/// Store PNG once by content hash; keys only store the hash (no duplicated base64).
pub fn put(keys: &[String], png_b64: &str) {
    let png = png_b64.trim();
    if png.len() < 32 || keys.is_empty() {
        return;
    }
    let h = hash_b64(png);
    let mut changed = false;
    {
        let mut guard = mem().lock();
        if !guard.blobs.contains_key(&h) {
            guard.blobs.insert(h, png.to_string());
            changed = true;
        }
        for k in keys {
            match guard.keys.get(k) {
                Some(old) if *old == h => {}
                _ => {
                    guard.keys.insert(k.clone(), h);
                    changed = true;
                }
            }
        }
    }
    if changed {
        schedule_flush();
    }
}

/// Fill empty PNG from cache only — safe on publish / list hot paths.
pub fn hydrate(pin_key: &str, process: &str, id: &str, png: &mut String) {
    if !png.trim().is_empty() {
        return;
    }
    let keys = keys_for(pin_key, process, id);
    if let Some(cached) = get(&keys) {
        *png = cached;
    }
}

/// Persist a newly observed glyph — skip if keys already cached (no re-hash).
pub fn remember(pin_key: &str, process: &str, id: &str, png: &str) {
    if png.trim().len() < 32 {
        return;
    }
    let keys = keys_for(pin_key, process, id);
    if keys.is_empty() {
        return;
    }
    {
        let guard = mem().lock();
        // Already indexed under every alias — do not re-hash multi-KB base64 on NIM_MODIFY.
        if keys.iter().all(|k| guard.keys.contains_key(k)) {
            return;
        }
    }
    // Clone off the caller thread so upsert/hook never blocks on hash+insert.
    let keys = keys;
    let png = png.to_string();
    let _ = std::thread::Builder::new()
        .name("tray-icon-cache-put".into())
        .spawn(move || {
            put(&keys, &png);
        });
}
