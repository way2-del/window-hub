//! Persisted 搜搜 prefs — SQLite `prefs_sousou` (included in Hub backup).
//! Legacy `%APPDATA%/window-hub/sousou.json` is migrated once then left unused.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

static CACHE: Mutex<Option<SousouConfig>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SousouShortcut {
    pub id: String,
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub kind: String, // app | url | file | folder
    #[serde(default)]
    pub icon_png: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SousouTab {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub items: Vec<SousouShortcut>,
    /// Bound folder: tab panel lists this directory's entries.
    #[serde(default)]
    pub folder_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchFilter {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub whole_word: bool,
    #[serde(default)]
    pub path: String,
    #[serde(default = "default_true")]
    pub include_subfolders: bool,
    #[serde(default)]
    pub ext_presets: Vec<String>,
    #[serde(default)]
    pub ext_custom: String,
    /// today | yesterday | last3 | last7 | last30 | ""
    #[serde(default)]
    pub modified: String,
    /// tiny | medium | large | ""
    #[serde(default)]
    pub size_preset: String,
    #[serde(default)]
    pub size_min: String,
    #[serde(default)]
    pub size_max: String,
    #[serde(default = "default_mb")]
    pub size_min_unit: String,
    #[serde(default = "default_mb")]
    pub size_max_unit: String,
}

fn default_true() -> bool {
    true
}
fn default_mb() -> String {
    "MB".into()
}

impl Default for SearchFilter {
    fn default() -> Self {
        Self {
            enabled: true,
            whole_word: false,
            path: String::new(),
            include_subfolders: true,
            ext_presets: Vec::new(),
            ext_custom: String::new(),
            modified: String::new(),
            size_preset: String::new(),
            size_min: String::new(),
            size_max: String::new(),
            size_min_unit: default_mb(),
            size_max_unit: default_mb(),
        }
    }
}

impl SearchFilter {
    /// Build Everything search modifiers (path / date / size). Extensions handled separately.
    pub fn to_everything_suffix(&self) -> String {
        if !self.enabled {
            return String::new();
        }
        let mut parts: Vec<String> = Vec::new();
        let path = self.path.trim();
        if !path.is_empty() {
            let p = path.trim_end_matches(['/', '\\']);
            if self.include_subfolders {
                parts.push(format!("\"{p}\""));
            } else {
                parts.push(format!("parent:\"{p}\""));
            }
        }
        match self.modified.as_str() {
            "today" => parts.push("dm:today".into()),
            "yesterday" => parts.push("dm:yesterday".into()),
            "last3" => parts.push("dm:last3days".into()),
            "last7" => parts.push("dm:last7days".into()),
            "last30" => parts.push("dm:last30days".into()),
            _ => {}
        }
        match self.size_preset.as_str() {
            "tiny" => parts.push("size:<=20kb".into()),
            "medium" => parts.push("size:1mb..128mb".into()),
            "large" => parts.push("size:>=128mb".into()),
            _ => {
                let min = parse_size_token(&self.size_min, &self.size_min_unit);
                let max = parse_size_token(&self.size_max, &self.size_max_unit);
                match (min, max) {
                    (Some(a), Some(b)) => parts.push(format!("size:{a}..{b}")),
                    (Some(a), None) => parts.push(format!("size:>={a}")),
                    (None, Some(b)) => parts.push(format!("size:<={b}")),
                    _ => {}
                }
            }
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!(" {}", parts.join(" "))
        }
    }

    pub fn collected_exts(&self) -> Vec<String> {
        if !self.enabled {
            return Vec::new();
        }
        let mut exts: Vec<String> = self
            .ext_presets
            .iter()
            .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        for e in self.ext_custom.split(',') {
            let e = e.trim().trim_start_matches('.').to_ascii_lowercase();
            if !e.is_empty() && !exts.iter().any(|x| x == &e) {
                exts.push(e);
            }
        }
        exts
    }
}

fn parse_size_token(raw: &str, unit: &str) -> Option<String> {
    let n = raw.trim().parse::<f64>().ok()?;
    if n < 0.0 {
        return None;
    }
    let u = unit.trim().to_ascii_uppercase();
    let suffix = match u.as_str() {
        "KB" | "K" => "kb",
        "GB" | "G" => "gb",
        _ => "mb",
    };
    if (n - n.round()).abs() < f64::EPSILON {
        Some(format!("{}{suffix}", n as u64))
    } else {
        Some(format!("{n}{suffix}"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SousouConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_everything_exe")]
    pub everything_exe: String,
    #[serde(default = "default_es_exe")]
    pub es_exe: String,
    /// Max ms between two Ctrl key-ups to count as double-tap.
    #[serde(default = "default_hotkey_ms")]
    pub double_ctrl_ms: u64,
    #[serde(default = "default_hotkey_enabled")]
    pub hotkey_enabled: bool,
    #[serde(default)]
    pub window_width: f64,
    #[serde(default)]
    pub window_height: f64,
    #[serde(default)]
    pub active_tab_id: String,
    #[serde(default = "default_tabs")]
    pub tabs: Vec<SousouTab>,
    /// User-pinned apps on the home「应用」card.
    #[serde(default)]
    pub home_apps: Vec<SousouShortcut>,
    #[serde(default)]
    pub search_filter: SearchFilter,
    /// Hover tab bar to switch (no click required).
    #[serde(default = "default_true")]
    pub hover_switch_tabs: bool,
    /// Hide 搜搜 after launching an app / file / system tool.
    #[serde(default = "default_true")]
    pub close_after_open: bool,
}

fn default_enabled() -> bool {
    true
}
fn default_hotkey_enabled() -> bool {
    true
}
fn default_hotkey_ms() -> u64 {
    350
}
fn default_everything_exe() -> String {
    r"D:\app\Everything\Everything.exe".into()
}
fn default_es_exe() -> String {
    r"D:\app\Everything\es.exe".into()
}

fn default_tabs() -> Vec<SousouTab> {
    vec![
        tab("home", "主页", "home"),
        tab("apps", "应用", "apps"),
        tab("code", "编程", "code"),
        tab("work", "工作", "work"),
        tab("notes", "笔记", "notes"),
        tab("xinwu", "信物社", "community"),
        tab("shop", "电商", "shop"),
        tab("tools", "小工具", "tools"),
        tab("optimize", "优化", "optimize"),
    ]
}

fn tab(id: &str, name: &str, icon: &str) -> SousouTab {
    SousouTab {
        id: id.into(),
        name: name.into(),
        icon: icon.into(),
        items: Vec::new(),
        folder_path: String::new(),
    }
}

impl Default for SousouConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            everything_exe: default_everything_exe(),
            es_exe: default_es_exe(),
            double_ctrl_ms: default_hotkey_ms(),
            hotkey_enabled: true,
            window_width: 960.0,
            window_height: 640.0,
            active_tab_id: "home".into(),
            tabs: default_tabs(),
            home_apps: Vec::new(),
            search_filter: SearchFilter::default(),
            hover_switch_tabs: true,
            close_after_open: true,
        }
    }
}

fn legacy_json_path() -> Result<PathBuf, String> {
    Ok(crate::db::app_data_root()?.join("sousou.json"))
}

pub fn load() -> SousouConfig {
    if let Ok(g) = CACHE.lock() {
        if let Some(ref c) = *g {
            return c.clone();
        }
    }
    let cfg = load_from_db().unwrap_or_else(|_| {
        match migrate_legacy_json() {
            Ok(c) => c,
            Err(_) => {
                let d = SousouConfig::default();
                let _ = save(&d);
                d
            }
        }
    });
    let cfg = clear_home_apps_once(cfg);
    if let Ok(mut g) = CACHE.lock() {
        *g = Some(cfg.clone());
    }
    cfg
}

fn load_from_db() -> Result<SousouConfig, String> {
    let raw = crate::db::with_conn(|c| crate::db::sousou_get(c))?;
    let Some(v) = raw else {
        return Err("empty".into());
    };
    serde_json::from_value(v).map_err(|e| e.to_string())
}

fn migrate_legacy_json() -> Result<SousouConfig, String> {
    let path = legacy_json_path()?;
    if !path.exists() {
        return Err("no legacy file".into());
    }
    let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let cfg: SousouConfig = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    save(&cfg)?;
    let _ = fs::rename(&path, path.with_extension("json.bak"));
    Ok(cfg)
}

/// One-shot wipe of auto-filled home pins — users add apps themselves via +.
fn clear_home_apps_once(mut cfg: SousouConfig) -> SousouConfig {
    let already = crate::db::with_conn(|c| crate::db::meta_get(c, "sousou_home_manual_v1"))
        .ok()
        .flatten()
        .is_some();
    if already {
        return cfg;
    }
    cfg.home_apps.clear();
    let _ = save(&cfg);
    let _ = crate::db::with_conn(|c| {
        crate::db::meta_set(c, "sousou_home_manual_v1", &serde_json::json!(true))
    });
    cfg
}

pub fn save(cfg: &SousouConfig) -> Result<(), String> {
    let v = serde_json::to_value(cfg).map_err(|e| e.to_string())?;
    crate::db::with_conn(|c| crate::db::sousou_set(c, &v))?;
    if let Ok(mut g) = CACHE.lock() {
        *g = Some(cfg.clone());
    }
    Ok(())
}

pub fn update(mut patch: SousouConfig) -> Result<SousouConfig, String> {
    if patch.tabs.is_empty() {
        patch.tabs = load().tabs;
    }
    if patch.everything_exe.trim().is_empty() {
        patch.everything_exe = default_everything_exe();
    }
    if patch.es_exe.trim().is_empty() {
        patch.es_exe = default_es_exe();
    }
    if patch.double_ctrl_ms < 100 {
        patch.double_ctrl_ms = 100;
    }
    if patch.double_ctrl_ms > 2000 {
        patch.double_ctrl_ms = 2000;
    }
    save(&patch)?;
    Ok(patch)
}
