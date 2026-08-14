//! List directory entries for tab-bound folders / resolve dropped paths.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::config::SousouShortcut;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    #[serde(default)]
    pub icon_png: Option<String>,
    pub size: Option<u64>,
    pub modified_ms: u64,
}

/// First existing directory among `paths` (Explorer drop → bind folder).
pub fn first_folder(paths: &[String]) -> Option<String> {
    for p in paths {
        let t = p.trim();
        if t.is_empty() {
            continue;
        }
        let path = Path::new(t);
        if path.is_dir() {
            return Some(path.to_string_lossy().to_string());
        }
    }
    None
}

/// Turn Explorer-dropped paths into pinable shortcuts (.lnk / .exe / folder / file).
pub fn paths_to_shortcuts(paths: &[String], with_icons: bool) -> Vec<SousouShortcut> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);

    for (i, raw) in paths.iter().enumerate() {
        let src = raw.trim();
        if src.is_empty() {
            continue;
        }
        let src_path = Path::new(src);
        if !src_path.exists() {
            continue;
        }

        let ext = src_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let (launch, kind, display_name) = if ext == "lnk" {
            let target = crate::dock::resolve_lnk_target(src).unwrap_or_else(|| src.to_string());
            let name = src_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("快捷方式")
                .to_string();
            let tpath = Path::new(&target);
            let kind = if tpath.is_dir() {
                "folder"
            } else if tpath
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .eq_ignore_ascii_case("exe")
            {
                "app"
            } else {
                "file"
            };
            (target, kind, name)
        } else if src_path.is_dir() {
            let name = src_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(src)
                .to_string();
            (src.to_string(), "folder", name)
        } else if ext == "exe" {
            let name = src_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(src)
                .to_string();
            (src.to_string(), "app", name)
        } else {
            let name = src_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(src)
                .to_string();
            (src.to_string(), "file", name)
        };

        let key = launch.to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }

        let icon_png = if with_icons {
            crate::dock::resolve_launcher_icon_png(&launch)
                .or_else(|| crate::dock::resolve_launcher_icon_png(src))
        } else {
            None
        };

        out.push(SousouShortcut {
            id: format!("drop-{now}-{i}"),
            name: display_name,
            path: launch,
            kind: kind.into(),
            icon_png,
        });
    }
    out
}

pub fn list_dir(path: &str, with_icons: bool, limit: usize) -> Result<Vec<DirEntry>, String> {
    let root = PathBuf::from(path.trim());
    if root.as_os_str().is_empty() {
        return Err("empty path".into());
    }
    if !root.is_dir() {
        return Err(format!("not a folder: {}", root.display()));
    }
    let rd = fs::read_dir(&root).map_err(|e| format!("read_dir {}: {e}", root.display()))?;
    let mut entries: Vec<DirEntry> = Vec::new();
    for e in rd.filter_map(|x| x.ok()) {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name == "." || name == ".." {
            continue;
        }
        // Skip hidden / system-ish names on Windows.
        if name.starts_with('.') || name.eq_ignore_ascii_case("desktop.ini") {
            continue;
        }
        let meta = e.metadata().ok();
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or_else(|| p.is_dir());
        let size = meta
            .as_ref()
            .filter(|m| m.is_file())
            .map(|m| m.len());
        let modified_ms = meta
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let path_str = p.to_string_lossy().to_string();
        entries.push(DirEntry {
            name,
            path: path_str,
            is_dir,
            icon_png: None,
            size,
            modified_ms,
        });
    }
    // Folders first, then name (case-insensitive).
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()),
    });
    if limit > 0 && entries.len() > limit {
        entries.truncate(limit);
    }
    if with_icons {
        for it in &mut entries {
            it.icon_png = crate::dock::resolve_launcher_icon_png(&it.path);
        }
    }
    Ok(entries)
}
