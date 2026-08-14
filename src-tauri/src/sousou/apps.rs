//! Enumerate Start Menu + Desktop shortcuts as installable apps.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::pinyin;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub path: String,
    pub target: String,
    #[serde(default)]
    pub icon_png: Option<String>,
    #[serde(default)]
    pub initials: String,
}

/// Index without icons (fast). Icons filled on demand.
static INDEX: Mutex<Option<(Instant, Vec<AppEntry>)>> = Mutex::new(None);
const CACHE_TTL: Duration = Duration::from_secs(180);

fn ensure_index() -> Vec<AppEntry> {
    if let Ok(g) = INDEX.lock() {
        if let Some((at, ref apps)) = *g {
            if at.elapsed() < CACHE_TTL {
                return apps.clone();
            }
        }
    }
    let mut apps = scan_all();
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    if let Ok(mut g) = INDEX.lock() {
        *g = Some((Instant::now(), apps.clone()));
    }
    apps
}

fn fill_icons(apps: &mut [AppEntry], max: usize) {
    for a in apps.iter_mut().take(max) {
        if a.icon_png.is_some() {
            continue;
        }
        // Always prefer 256px shell icons — 32px sources look blurry at 48px UI.
        a.icon_png = super::icon_cache::get_or_resolve(&a.target)
            .or_else(|| super::icon_cache::get_or_resolve(&a.path));
    }
}

/// Fast list. Icons only when `with_icons` and capped by `limit` (0 = all names, icons up to 120).
pub fn list_apps(with_icons: bool, limit: usize) -> Vec<AppEntry> {
    let mut apps = ensure_index();
    if limit > 0 && apps.len() > limit {
        apps.truncate(limit);
    }
    if with_icons {
        let icon_cap = if limit > 0 { limit.min(120) } else { 120 };
        fill_icons(&mut apps, icon_cap);
    }
    apps
}

pub fn search_apps(query: &str, limit: usize) -> Vec<AppEntry> {
    let all = ensure_index();
    let mut matched: Vec<AppEntry> = all
        .into_iter()
        .filter(|a| pinyin::matches_query(&a.name, query))
        .collect();
    let cap = if limit > 0 { limit } else { 40 };
    if matched.len() > cap {
        matched.truncate(cap);
    }
    let n = matched.len();
    fill_icons(&mut matched, n);
    matched
}

pub fn invalidate_cache() {
    if let Ok(mut g) = INDEX.lock() {
        *g = None;
    }
}

fn scan_all() -> Vec<AppEntry> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for dir in start_menu_dirs() {
        walk_lnks(&dir, &mut seen, &mut out);
    }
    for dir in desktop_dirs() {
        walk_lnks(&dir, &mut seen, &mut out);
    }
    push_builtin(&mut seen, &mut out, "计算器", "calc.exe");
    push_builtin(&mut seen, &mut out, "画图", "mspaint.exe");
    push_builtin(&mut seen, &mut out, "写字板", "write.exe");
    push_builtin(&mut seen, &mut out, "记事本", "notepad.exe");
    out
}

fn push_builtin(seen: &mut HashSet<String>, out: &mut Vec<AppEntry>, name: &str, exe: &str) {
    let key = exe.to_ascii_lowercase();
    if !seen.insert(key.clone()) {
        return;
    }
    out.push(AppEntry {
        id: format!("builtin:{key}"),
        name: name.into(),
        path: exe.into(),
        target: exe.into(),
        icon_png: None,
        initials: pinyin::initials(name),
    });
}

fn start_menu_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        dirs.push(PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if let Ok(prog) = std::env::var("ProgramData") {
        dirs.push(PathBuf::from(prog).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    dirs
}

fn desktop_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(user) = std::env::var("USERPROFILE") {
        dirs.push(PathBuf::from(user).join("Desktop"));
    }
    if let Ok(pub_dir) = std::env::var("PUBLIC") {
        dirs.push(PathBuf::from(pub_dir).join("Desktop"));
    }
    dirs
}

fn walk_lnks(root: &Path, seen: &mut HashSet<String>, out: &mut Vec<AppEntry>) {
    let Ok(rd) = fs::read_dir(root) else {
        return;
    };
    let mut stack: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    while let Some(p) = stack.pop() {
        if p.is_dir() {
            if let Ok(rd) = fs::read_dir(&p) {
                for e in rd.flatten() {
                    stack.push(e.path());
                }
            }
            continue;
        }
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext != "lnk" && ext != "exe" {
            continue;
        }
        let name = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty()
            || name.starts_with("卸载")
            || name.to_ascii_lowercase().contains("uninstall")
        {
            continue;
        }
        let path_str = p.to_string_lossy().to_string();
        let target = if ext == "lnk" {
            crate::dock::resolve_lnk_target(&path_str).unwrap_or_else(|| path_str.clone())
        } else {
            path_str.clone()
        };
        let t_lower = target.to_ascii_lowercase();
        if t_lower.ends_with(".exe") {
            if !seen.insert(t_lower) {
                continue;
            }
        } else if !seen.insert(path_str.to_ascii_lowercase()) {
            continue;
        }
        out.push(AppEntry {
            id: path_str.clone(),
            name: name.clone(),
            path: path_str,
            target,
            icon_png: None,
            initials: pinyin::initials(&name),
        });
    }
}
