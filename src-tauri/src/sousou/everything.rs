//! Everything IPC via es.exe (CSV export).

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use super::config;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHit {
    pub name: String,
    pub path: String,
    pub full_path: String,
    pub size: Option<u64>,
    pub modified: Option<String>,
    pub is_dir: bool,
    pub category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryBucket {
    pub id: String,
    pub label: String,
    pub total: u64,
    pub items: Vec<FileHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EverythingStatus {
    pub running: bool,
    pub everything_exe: String,
    pub es_exe: String,
    pub message: String,
}

pub fn status() -> EverythingStatus {
    let cfg = config::load();
    let es_ok = Path::new(&cfg.es_exe).exists();
    let ev_ok = Path::new(&cfg.everything_exe).exists();
    let running = is_everything_running();
    let message = if !ev_ok {
        format!("未找到 Everything：{}", cfg.everything_exe)
    } else if !es_ok {
        format!("未找到 es.exe：{}", cfg.es_exe)
    } else if !running {
        "Everything 未运行，点击启动索引服务".into()
    } else {
        "就绪".into()
    };
    EverythingStatus {
        running,
        everything_exe: cfg.everything_exe,
        es_exe: cfg.es_exe,
        message,
    }
}

pub fn is_everything_running() -> bool {
    let cfg = config::load();
    if !Path::new(&cfg.es_exe).exists() {
        return false;
    }
    let out = Command::new(&cfg.es_exe)
        .arg("-get-everything-version")
        .output();
    match out {
        Ok(o) => o.status.success(),
        Err(_) => false,
    }
}

pub fn ensure_running() -> Result<EverythingStatus, String> {
    let cfg = config::load();
    if !Path::new(&cfg.everything_exe).exists() {
        return Err(format!("Everything.exe 不存在：{}", cfg.everything_exe));
    }
    if is_everything_running() {
        return Ok(status());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        Command::new(&cfg.everything_exe)
            .arg("-startup")
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("启动 Everything 失败: {e}"))?;
    }
    #[cfg(not(windows))]
    {
        return Err("Windows only".into());
    }
    // Wait briefly for IPC window
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(250));
        if is_everything_running() {
            return Ok(status());
        }
    }
    Ok(status())
}

fn category_filter(category: &str) -> (Vec<String>, bool) {
    match category {
        "folder" | "folders" => (vec!["/ad".into()], true),
        "doc" | "docs" | "documents" => (
            vec![], // handled via multi-query in search_category
            false,
        ),
        "image" | "images" | "pic" => (vec![], false),
        "archive" | "zip" | "compressed" => (vec![], false),
        "media" | "av" | "audio" | "video" => (vec![], false),
        "other" => (vec!["/a-d".into()], false),
        _ => (Vec::new(), false), // all / best
    }
}

fn category_exts(category: &str) -> Option<&'static [&'static str]> {
    match category {
        "doc" | "docs" | "documents" => Some(&[
            "*.doc", "*.docx", "*.xls", "*.xlsx", "*.ppt", "*.pptx", "*.pdf", "*.txt", "*.md",
            "*.rtf",
        ]),
        "image" | "images" | "pic" => {
            Some(&["*.png", "*.jpg", "*.jpeg", "*.gif", "*.bmp", "*.webp", "*.ico", "*.svg"])
        }
        "archive" | "zip" | "compressed" => {
            Some(&["*.zip", "*.rar", "*.7z", "*.tar", "*.gz", "*.iso"])
        }
        "media" | "av" | "audio" | "video" => Some(&[
            "*.mp3", "*.wav", "*.flac", "*.aac", "*.mp4", "*.mkv", "*.avi", "*.mov", "*.wmv",
            "*.webm",
        ]),
        _ => None,
    }
}

pub fn search_category(query: &str, category: &str, limit: usize) -> Result<(u64, Vec<FileHit>), String> {
    let q0 = query.trim();
    if q0.is_empty() {
        return Ok((0, Vec::new()));
    }
    if !is_everything_running() {
        let _ = ensure_running();
        if !is_everything_running() {
            return Err("Everything 未运行".into());
        }
    }

    let filt = config::load().search_filter;
    let mut q = q0.to_string();
    if filt.enabled && filt.whole_word {
        q = format!("ww:{q}");
    }
    q.push_str(&filt.to_everything_suffix());

    let filter_exts = filt.collected_exts();
    if !filter_exts.is_empty() {
        let mut total: u64 = 0;
        let mut items: Vec<FileHit> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for e in &filter_exts {
            let filter = vec![format!("*.{e}")];
            total = total.saturating_add(get_result_count(&q, &filter).unwrap_or(0));
            for h in query_csv(&q, &filter, limit, false, category).unwrap_or_default() {
                if seen.insert(h.full_path.clone()) {
                    items.push(h);
                }
                if items.len() >= limit {
                    break;
                }
            }
            if items.len() >= limit {
                break;
            }
        }
        return Ok((total, items));
    }

    if let Some(exts) = category_exts(category) {
        let mut total: u64 = 0;
        let mut items: Vec<FileHit> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for ext in exts {
            let (t, hits) = {
                let filter = vec![(*ext).to_string()];
                let t = get_result_count(&q, &filter).unwrap_or(0);
                let hits = query_csv(&q, &filter, limit, false, category).unwrap_or_default();
                (t, hits)
            };
            total = total.saturating_add(t);
            for h in hits {
                if seen.insert(h.full_path.clone()) {
                    items.push(h);
                }
                if items.len() >= limit {
                    break;
                }
            }
            if items.len() >= limit {
                break;
            }
        }
        return Ok((total, items));
    }

    let (filter_args, is_dir) = category_filter(category);
    let total = get_result_count(&q, &filter_args)?;
    let items = query_csv(&q, &filter_args, limit, is_dir, category)?;
    Ok((total, items))
}

fn get_result_count(query: &str, filter_args: &[String]) -> Result<u64, String> {
    let cfg = config::load();
    let mut cmd = Command::new(&cfg.es_exe);
    cmd.arg("-get-result-count");
    for f in filter_args {
        cmd.arg(f);
    }
    cmd.arg(query);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(s.parse::<u64>().unwrap_or(0))
}

fn query_csv(
    query: &str,
    filter_args: &[String],
    limit: usize,
    is_dir_hint: bool,
    category: &str,
) -> Result<Vec<FileHit>, String> {
    let cfg = config::load();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("wh-sousou-{stamp}.csv"));
    let tmp_str = tmp.to_string_lossy().to_string();

    let mut cmd = Command::new(&cfg.es_exe);
    cmd.arg("-n")
        .arg(limit.to_string())
        .arg("-export-csv")
        .arg(&tmp_str)
        .arg("-name")
        .arg("-path-column")
        .arg("-size")
        .arg("-date-modified")
        .arg("-date-format")
        .arg("1");
    for f in filter_args {
        cmd.arg(f);
    }
    cmd.arg(query);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let status = cmd.status().map_err(|e| e.to_string())?;
    if !status.success() && !tmp.exists() {
        return Err(format!("es.exe 查询失败 (code {:?})", status.code()));
    }
    let raw = fs::read_to_string(&tmp).unwrap_or_default();
    let _ = fs::remove_file(&tmp);
    Ok(parse_csv(&raw, is_dir_hint, category))
}

fn parse_csv(raw: &str, is_dir_hint: bool, category: &str) -> Vec<FileHit> {
    let mut out = Vec::new();
    let mut lines = raw.lines();
    let _header = lines.next();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let cols = split_csv_line(line);
        if cols.len() < 2 {
            continue;
        }
        let name = cols[0].clone();
        let parent = cols.get(1).cloned().unwrap_or_default();
        let size = cols
            .get(2)
            .and_then(|s| s.replace(',', "").parse::<u64>().ok());
        let modified = cols.get(3).cloned().filter(|s| !s.is_empty());
        let full_path = if parent.is_empty() {
            name.clone()
        } else if parent.ends_with('\\') || parent.ends_with('/') {
            format!("{parent}{name}")
        } else {
            format!("{parent}\\{name}")
        };
        let is_dir = is_dir_hint
            || size.is_none() && Path::new(&full_path).is_dir()
            || category == "folder";
        out.push(FileHit {
            name,
            path: parent,
            full_path,
            size,
            modified,
            is_dir,
            category: category.to_string(),
        });
    }
    out
}

fn split_csv_line(line: &str) -> Vec<String> {
    let mut cols = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if in_q {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        cur.push('"');
                    } else {
                        in_q = false;
                    }
                } else {
                    in_q = true;
                }
            }
            ',' if !in_q => {
                cols.push(cur.clone());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    cols.push(cur);
    cols
}

/// Multi-category search for the search results page.
pub fn search_all(query: &str, per_cat: usize) -> Result<Vec<CategoryBucket>, String> {
    let cats: &[(&str, &str)] = &[
        ("all", "全部"),
        ("folder", "文件夹"),
        ("doc", "文档"),
        ("image", "图片"),
        ("archive", "压缩"),
        ("media", "音视频"),
        ("other", "其他"),
    ];
    let mut buckets = Vec::new();
    for (id, label) in cats {
        let (total, items) = search_category(query, id, per_cat).unwrap_or((0, Vec::new()));
        buckets.push(CategoryBucket {
            id: (*id).into(),
            label: (*label).into(),
            total,
            items,
        });
    }
    Ok(buckets)
}
