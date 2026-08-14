//! Everything IPC via es.exe (CSV export).

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::config;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static CSV_SEQ: AtomicU64 = AtomicU64::new(1);

/// Always hide the console — es.exe is a CLI and otherwise flashes a black cmd window.
fn es_cmd(es_exe: &str) -> Command {
    let mut cmd = Command::new(es_exe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

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
    let out = es_cmd(&cfg.es_exe)
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

/// Category / user-ext filters as extra es.exe search tokens (separate argv entries).
fn build_query(raw: &str, category: &str) -> (String, Vec<String>, bool) {
    let filt = config::load().search_filter;
    let mut q = raw.trim().to_string();
    if filt.enabled && filt.whole_word {
        q = format!("ww:{q}");
    }
    q.push_str(&filt.to_everything_suffix());

    let mut extras: Vec<String> = Vec::new();
    let user_exts = filt.collected_exts();
    if !user_exts.is_empty() {
        // One OR-group: *.zip|*.rar — es accepts this as a single token.
        let wild = user_exts
            .iter()
            .map(|e| format!("*.{e}"))
            .collect::<Vec<_>>()
            .join("|");
        extras.push(wild);
        return (q, extras, false);
    }

    let is_dir = match category {
        "folder" | "folders" => {
            extras.push("/ad".into());
            true
        }
        "doc" | "docs" | "documents" => {
            extras.push(
                "*.doc|*.docx|*.xls|*.xlsx|*.ppt|*.pptx|*.pdf|*.txt|*.md|*.rtf".into(),
            );
            false
        }
        "image" | "images" | "pic" => {
            extras.push("*.png|*.jpg|*.jpeg|*.gif|*.bmp|*.webp|*.ico|*.svg".into());
            false
        }
        "archive" | "zip" | "compressed" => {
            extras.push("*.zip|*.rar|*.7z|*.tar|*.gz|*.iso".into());
            false
        }
        "media" | "av" | "audio" | "video" => {
            extras.push("*.mp3|*.wav|*.flac|*.aac|*.mp4|*.mkv|*.avi|*.mov|*.wmv|*.webm".into());
            false
        }
        "other" => {
            // Files only; typed buckets cover the common exts above.
            extras.push("/a-d".into());
            false
        }
        _ => false, // all
    };
    (q, extras, is_dir)
}

fn search_category_checked(
    query: &str,
    category: &str,
    limit: usize,
) -> Result<(u64, Vec<FileHit>), String> {
    let q0 = query.trim();
    if q0.is_empty() {
        return Ok((0, Vec::new()));
    }
    let (q, extras, is_dir) = build_query(q0, category);
    let total = get_result_count(&q, &extras).unwrap_or(0);
    let items = query_csv(&q, &extras, limit, is_dir, category).unwrap_or_default();
    Ok((total, items))
}

fn apply_path_scope(cmd: &mut Command) {
    let filt = config::load().search_filter;
    if let Some((include_sub, path)) = filt.path_scope() {
        if include_sub {
            cmd.arg("-path").arg(path);
        } else {
            cmd.arg("-parent").arg(path);
        }
    }
}

fn get_result_count(query: &str, extras: &[String]) -> Result<u64, String> {
    let cfg = config::load();
    let mut cmd = es_cmd(&cfg.es_exe);
    // Name-only match (no -p). Full-path match pulls every file under a folder
    // named like the query (e.g. 逆战 → all files in …\逆战\…).
    apply_path_scope(&mut cmd);
    cmd.arg("-get-result-count");
    cmd.arg(query);
    for e in extras {
        cmd.arg(e);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    let s = decode_es_bytes(&out.stdout);
    Ok(s.trim().parse::<u64>().unwrap_or(0))
}

fn unique_csv_path() -> std::path::PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let seq = CSV_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("wh-sousou-{stamp}-{seq}.csv"))
}

fn query_csv(
    query: &str,
    extras: &[String],
    limit: usize,
    is_dir_hint: bool,
    category: &str,
) -> Result<Vec<FileHit>, String> {
    let cfg = config::load();
    let tmp = unique_csv_path();
    let tmp_str = tmp.to_string_lossy().to_string();

    let mut cmd = es_cmd(&cfg.es_exe);
    // Match file/folder name only — keep -path-column for CSV export, not -p.
    cmd.arg("-n")
        .arg(limit.to_string())
        .arg("-export-csv")
        .arg(&tmp_str)
        .arg("-utf8-bom")
        .arg("-name")
        .arg("-path-column")
        .arg("-size")
        .arg("-date-modified")
        .arg("-date-format")
        .arg("1");
    apply_path_scope(&mut cmd);
    cmd.arg(query);
    for e in extras {
        cmd.arg(e);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;

    if !tmp.exists() {
        if out.status.success() && !out.stdout.is_empty() {
            let raw = decode_es_bytes(&out.stdout);
            return Ok(parse_csv(&raw, is_dir_hint, category));
        }
        return Err(format!(
            "es.exe 查询失败 (code {:?}): {}",
            out.status.code(),
            decode_es_bytes(&out.stderr)
        ));
    }

    let bytes = fs::read(&tmp).unwrap_or_default();
    let _ = fs::remove_file(&tmp);
    if bytes.is_empty() && !out.status.success() {
        return Err(format!("es.exe 查询失败 (code {:?})", out.status.code()));
    }
    let raw = decode_es_bytes(&bytes);
    Ok(parse_csv(&raw, is_dir_hint, category))
}

/// es.exe may write UTF-8 (export -utf8-bom) or system ANSI (stdout / older builds).
fn decode_es_bytes(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    let body = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        return decode_utf16_le(&bytes[2..]);
    } else {
        bytes
    };
    if let Ok(s) = std::str::from_utf8(body) {
        return s.to_string();
    }
    #[cfg(windows)]
    {
        if let Some(s) = decode_acp(body) {
            return s;
        }
    }
    String::from_utf8_lossy(body).into_owned()
}

fn decode_utf16_le(bytes: &[u8]) -> String {
    let mut u16s = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i + 1 < bytes.len() {
        u16s.push(u16::from_le_bytes([bytes[i], bytes[i + 1]]));
        i += 2;
    }
    String::from_utf16_lossy(&u16s)
}

#[cfg(windows)]
fn decode_acp(bytes: &[u8]) -> Option<String> {
    use windows::Win32::Globalization::{MultiByteToWideChar, CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};
    unsafe {
        let need = MultiByteToWideChar(CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, None);
        if need <= 0 {
            return None;
        }
        let mut wide = vec![0u16; need as usize];
        let written = MultiByteToWideChar(
            CP_ACP,
            MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0),
            bytes,
            Some(wide.as_mut_slice()),
        );
        if written <= 0 {
            return None;
        }
        wide.truncate(written as usize);
        Some(String::from_utf16_lossy(&wide))
    }
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
            || (size.is_none() && Path::new(&full_path).is_dir())
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
                cols.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    cols.push(cur);
    cols
}

/// Multi-category search for the search results page.
pub fn search_all(query: &str, per_cat: usize) -> Result<Vec<CategoryBucket>, String> {
    if !is_everything_running() {
        let _ = ensure_running();
        if !is_everything_running() {
            return Err("Everything 未运行".into());
        }
    }
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
        let (total, items) =
            search_category_checked(query, id, per_cat).unwrap_or((0, Vec::new()));
        buckets.push(CategoryBucket {
            id: (*id).into(),
            label: (*label).into(),
            total,
            items,
        });
    }
    Ok(buckets)
}
