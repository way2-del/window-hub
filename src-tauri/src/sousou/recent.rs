//! Recent files from `%APPDATA%\Microsoft\Windows\Recent\*.lnk`.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    #[serde(default)]
    pub icon_png: Option<String>,
    pub modified_ms: u64,
}

pub fn list_recent(limit: usize, with_icons: bool) -> Vec<RecentEntry> {
    let mut entries = Vec::new();
    let Ok(appdata) = std::env::var("APPDATA") else {
        return entries;
    };
    let recent = PathBuf::from(appdata).join(r"Microsoft\Windows\Recent");
    let Ok(rd) = fs::read_dir(&recent) else {
        return entries;
    };
    let mut files: Vec<_> = rd.filter_map(|e| e.ok()).collect();
    files.sort_by_key(|e| {
        e.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    });
    files.reverse();

    for e in files {
        if entries.len() >= limit.max(1) {
            break;
        }
        let p = e.path();
        let ext = p
            .extension()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext != "lnk" {
            continue;
        }
        let lnk = p.to_string_lossy().to_string();
        let Some(target) = crate::dock::resolve_lnk_target(&lnk) else {
            continue;
        };
        if target.is_empty() || !std::path::Path::new(&target).exists() {
            continue;
        }
        let name = std::path::Path::new(&target)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&target)
            .to_string();
        let is_dir = std::path::Path::new(&target).is_dir();
        let modified_ms = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let icon_png = if with_icons {
            crate::dock::resolve_launcher_icon_png(&target)
        } else {
            None
        };
        entries.push(RecentEntry {
            name,
            path: target,
            is_dir,
            icon_png,
            modified_ms,
        });
    }
    entries
}
