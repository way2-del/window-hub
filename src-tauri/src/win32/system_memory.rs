//! Process memory ranking + working-set trim (MemMeter flyout).

#![cfg(windows)]

use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemProcessRow {
    pub pid: u32,
    pub name: String,
    /// Working set, MiB.
    pub working_set_mb: u64,
    /// Exe shell icon as PNG base64 (no data: prefix).
    pub icon_png: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemTopSnapshot {
    pub mem_percent: u8,
    pub used_mb: u64,
    pub total_mb: u64,
    pub avail_mb: u64,
    /// Commit charge used (approx. “虚拟内存/提交”), MiB.
    pub commit_used_mb: u64,
    /// Commit limit (RAM + pagefile), MiB.
    pub commit_total_mb: u64,
    pub commit_percent: u8,
    pub processes: Vec<MemProcessRow>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemPurgeResult {
    pub before_percent: u8,
    pub after_percent: u8,
    pub before_avail_mb: u64,
    pub after_avail_mb: u64,
    /// Physical memory freed (approx), MiB.
    pub freed_mb: u64,
    pub before_commit_used_mb: u64,
    pub after_commit_used_mb: u64,
    /// Commit charge reduced (approx), MiB.
    pub commit_freed_mb: u64,
    /// Processes where EmptyWorkingSet succeeded.
    pub trimmed: u32,
}

struct MemStatus {
    percent: u8,
    used_mb: u64,
    total_mb: u64,
    avail_mb: u64,
    commit_used_mb: u64,
    commit_total_mb: u64,
    commit_percent: u8,
}

fn mem_status() -> MemStatus {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    unsafe {
        let mut st = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        if GlobalMemoryStatusEx(&mut st).is_err() {
            return MemStatus {
                percent: 0,
                used_mb: 0,
                total_mb: 0,
                avail_mb: 0,
                commit_used_mb: 0,
                commit_total_mb: 0,
                commit_percent: 0,
            };
        }
        let total = st.ullTotalPhys / (1024 * 1024);
        let avail = st.ullAvailPhys / (1024 * 1024);
        let used = total.saturating_sub(avail);
        let commit_total = st.ullTotalPageFile / (1024 * 1024);
        let commit_avail = st.ullAvailPageFile / (1024 * 1024);
        let commit_used = commit_total.saturating_sub(commit_avail);
        let commit_percent = if commit_total == 0 {
            0
        } else {
            ((commit_used * 100) / commit_total).min(100) as u8
        };
        MemStatus {
            percent: st.dwMemoryLoad.min(100) as u8,
            used_mb: used,
            total_mb: total,
            avail_mb: avail,
            commit_used_mb: commit_used,
            commit_total_mb: commit_total,
            commit_percent,
        }
    }
}

/// Cheap physical mem % for background pressure throttling (no alloc).
pub fn physical_mem_percent() -> u8 {
    mem_status().percent
}

fn process_image(pid: u32) -> Option<(String, String)> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let Ok(proc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return None;
        };
        let mut buf = [0u16; 512];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            proc,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut size,
        )
        .is_ok();
        let _ = CloseHandle(proc);
        if !ok || size == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..size as usize]);
        let name = Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        Some((name, path))
    }
}

fn icon_for_exe(path: &str) -> Option<String> {
    static CACHE: LazyLock<Mutex<HashMap<String, Option<String>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    {
        let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(hit) = cache.get(path) {
            return hit.clone();
        }
    }
    // Small icon only — 256px ShellItem factory is too heavy for ranking lists.
    let png = crate::dock::resolve_small_icon_png(path);
    if let Ok(mut cache) = CACHE.lock() {
        cache.insert(path.to_string(), png.clone());
    }
    png
}

fn process_working_set(pid: u32) -> Option<u64> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let Ok(proc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return None;
        };
        let mut counters = PROCESS_MEMORY_COUNTERS::default();
        let ok = GetProcessMemoryInfo(
            proc,
            &mut counters,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
        .is_ok();
        let _ = CloseHandle(proc);
        if !ok {
            return None;
        }
        Some(counters.WorkingSetSize as u64)
    }
}

/// Top processes by working set (aggregated by exe name).
pub fn list_top(limit: usize) -> MemTopSnapshot {
    use windows::Win32::System::ProcessStatus::EnumProcesses;

    let st = mem_status();
    let limit = limit.clamp(5, 40);

    let mut pids = vec![0u32; 2048];
    let mut needed = 0u32;
    unsafe {
        if EnumProcesses(pids.as_mut_ptr(), (pids.len() * 4) as u32, &mut needed).is_err() {
            return MemTopSnapshot {
                mem_percent: st.percent,
                used_mb: st.used_mb,
                total_mb: st.total_mb,
                avail_mb: st.avail_mb,
                commit_used_mb: st.commit_used_mb,
                commit_total_mb: st.commit_total_mb,
                commit_percent: st.commit_percent,
                processes: Vec::new(),
            };
        }
    }
    let count = (needed as usize / 4).min(pids.len());
    pids.truncate(count);

    let mut by_name: HashMap<String, (u32, u64, String)> = HashMap::new();
    for pid in pids {
        if pid == 0 {
            continue;
        }
        let Some(ws) = process_working_set(pid) else {
            continue;
        };
        if ws < 2 * 1024 * 1024 {
            continue;
        }
        let (name, path) = process_image(pid).unwrap_or_else(|| {
            let n = format!("pid-{pid}");
            (n.clone(), String::new())
        });
        let entry = by_name.entry(name).or_insert((pid, 0, path));
        entry.1 = entry.1.saturating_add(ws);
    }

    let mut rows: Vec<(MemProcessRow, String)> = by_name
        .into_iter()
        .map(|(name, (pid, ws, path))| {
            (
                MemProcessRow {
                    pid,
                    name,
                    working_set_mb: ws / (1024 * 1024),
                    icon_png: None,
                },
                path,
            )
        })
        .collect();
    rows.sort_by(|a, b| b.0.working_set_mb.cmp(&a.0.working_set_mb));
    rows.truncate(limit);
    let processes: Vec<MemProcessRow> = rows
        .into_iter()
        .map(|(mut row, path)| {
            row.icon_png = icon_for_exe(&path);
            row
        })
        .collect();

    MemTopSnapshot {
        mem_percent: st.percent,
        used_mb: st.used_mb,
        total_mb: st.total_mb,
        avail_mb: st.avail_mb,
        commit_used_mb: st.commit_used_mb,
        commit_total_mb: st.commit_total_mb,
        commit_percent: st.commit_percent,
        processes,
    }
}

fn trim_process(pid: u32) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::ProcessStatus::EmptyWorkingSet;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SET_QUOTA,
    };
    unsafe {
        let access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_QUOTA;
        let proc = match OpenProcess(access, false, pid) {
            Ok(h) => h,
            Err(_) => {
                let access2 = PROCESS_QUERY_INFORMATION | PROCESS_SET_QUOTA;
                match OpenProcess(access2, false, pid) {
                    Ok(h) => h,
                    Err(_) => return false,
                }
            }
        };
        let ok = EmptyWorkingSet(proc).is_ok();
        let _ = CloseHandle(proc);
        ok
    }
}

fn should_skip_trim(pid: u32, self_pid: u32, self_name_lower: &str) -> bool {
    if pid == 0 || pid == self_pid {
        return true;
    }
    let Some((name, _)) = process_image(pid) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    if !self_name_lower.is_empty() && lower == self_name_lower {
        return true;
    }
    matches!(
        lower.as_str(),
        "msedgewebview2.exe"
            | "webview2.exe"
            | "dwm.exe"
            | "csrss.exe"
            | "winlogon.exe"
            | "smss.exe"
            | "services.exe"
            | "lsass.exe"
            | "svchost.exe"
            | "fontdrvhost.exe"
            | "sihost.exe"
            | "explorer.exe"
            | "system"
            | "registry"
            | "memory compression"
            | "secure system"
    )
}

/// Trim working sets of accessible processes (best-effort “clean memory”).
pub fn purge() -> MemPurgeResult {
    use windows::Win32::System::ProcessStatus::EnumProcesses;
    use windows::Win32::System::Threading::GetCurrentProcessId;

    let before = mem_status();
    let self_pid = unsafe { GetCurrentProcessId() };
    let self_name_lower = process_image(self_pid)
        .map(|(name, _)| name.to_ascii_lowercase())
        .unwrap_or_default();

    let mut pids = vec![0u32; 2048];
    let mut needed = 0u32;
    let mut trimmed = 0u32;
    unsafe {
        if EnumProcesses(pids.as_mut_ptr(), (pids.len() * 4) as u32, &mut needed).is_ok() {
            let count = (needed as usize / 4).min(pids.len());
            for &pid in &pids[..count] {
                if should_skip_trim(pid, self_pid, &self_name_lower) {
                    continue;
                }
                if process_working_set(pid).unwrap_or(0) < 8 * 1024 * 1024 {
                    continue;
                }
                if trim_process(pid) {
                    trimmed += 1;
                }
            }
        }
    }

    std::thread::sleep(std::time::Duration::from_millis(80));
    let after = mem_status();
    let freed_mb = after.avail_mb.saturating_sub(before.avail_mb);
    let commit_freed_mb = before
        .commit_used_mb
        .saturating_sub(after.commit_used_mb);

    MemPurgeResult {
        before_percent: before.percent,
        after_percent: after.percent,
        before_avail_mb: before.avail_mb,
        after_avail_mb: after.avail_mb,
        freed_mb,
        before_commit_used_mb: before.commit_used_mb,
        after_commit_used_mb: after.commit_used_mb,
        commit_freed_mb,
        trimmed,
    }
}

pub fn open_task_manager() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Command::new("taskmgr.exe")
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}
