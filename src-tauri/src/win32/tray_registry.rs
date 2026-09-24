//! HKCU\Control Panel\NotifyIconSettings — Win10/11 tray identity & order.
//!
//! Used for **enrichment only** (tooltip / snapshot / promoted vs overflow area)
//! and **soft stubs** for processes still alive when the explorer hook never
//! delivered NIM_ADD (common for WeChat / Wallpaper Engine / ACE after TaskbarCreated).
//! Clicks still come from the hook/spy hwnd+callback — stubs are display-only until then.

#![cfg(windows)]

use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use windows::core::GUID;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{Shell_NotifyIconGetRect, NOTIFYICONIDENTIFIER};
use winreg::enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS, KEY_READ};
use winreg::RegKey;

#[derive(Debug, Clone)]
pub struct RegTrayIcon {
    pub key: String,
    pub executable_path: PathBuf,
    pub process: String,
    pub icon_snapshot: Vec<u8>,
    pub initial_tooltip: Option<String>,
    pub icon_guid: Option<String>,
    pub icon_uid: Option<u32>,
    /// Windows `IsPromoted`: 1 = taskbar strip, 0 = overflow.
    pub is_promoted: bool,
}

fn normalize_guid(s: &str) -> String {
    s.trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .to_ascii_lowercase()
}

pub fn guid_key(guid: &str) -> String {
    normalize_guid(guid)
}

/// `windows::core::GUID::from(&str)` asserts len==36 (no braces) and panics on
/// bad input — never call it with `{guid}` or untrusted registry strings.
fn parse_guid(guid_str: &str) -> Option<GUID> {
    let g = normalize_guid(guid_str);
    if g.len() != 36 {
        return None;
    }
    // Digits + hyphens only, exact "8-4-4-4-12" shape.
    let ok = g.as_bytes().iter().enumerate().all(|(i, &b)| match i {
        8 | 13 | 18 | 23 => b == b'-',
        _ => b.is_ascii_hexdigit(),
    });
    if !ok {
        return None;
    }
    // Still use catch_unwind: From<&str> asserts on unexpected bytes.
    std::panic::catch_unwind(|| GUID::from(g.as_str())).ok()
}

fn process_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string()
}

/// Strip `\\?\`, unify separators, lowercase — registry often uses `\\?\Volume{…}\…`
/// while `QueryFullProcessImageNameW` returns `C:\…`.
pub fn normalize_exe_path(s: &str) -> String {
    let mut p = s.trim().replace('/', "\\").to_ascii_lowercase();
    if let Some(rest) = p.strip_prefix(r"\\?\") {
        p = rest.to_string();
    }
    p
}

fn exe_file_name(s: &str) -> String {
    Path::new(s)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// True when two ExecutablePath / image-path strings refer to the same binary.
pub fn exe_paths_equivalent(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let na = normalize_exe_path(a);
    let nb = normalize_exe_path(b);
    if na == nb {
        return true;
    }
    let fa = exe_file_name(&na);
    let fb = exe_file_name(&nb);
    !fa.is_empty() && fa == fb
}

/// Serialize registry + process/window enum — concurrent calls can hitch the shell.
static REGISTRY_ENUM_LOCK: Mutex<()> = Mutex::new(());

/// All running process image paths (PID-based — catches tray-only / no visible HWND).
///
/// Elevated apps (管理员): `OpenProcess` from a medium-IL host often fails (UIPI).
/// Fall back to Toolhelp `szExeFile` so stem / file-name matching still sees them
/// (PixPin / other admin trays would otherwise vanish from registry soft-seed).
fn enum_process_image_paths_unlocked() -> Vec<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let mut out: Vec<String> = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return out;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut ok = Process32FirstW(snap, &mut entry).is_ok();
        while ok {
            let pid = entry.th32ProcessID;
            if pid > 4 {
                let mut got_full = false;
                if let Ok(proc) =
                    OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
                {
                    let mut buf16 = [0u16; 520];
                    let mut size = buf16.len() as u32;
                    let qok = QueryFullProcessImageNameW(
                        proc,
                        PROCESS_NAME_WIN32,
                        windows::core::PWSTR(buf16.as_mut_ptr()),
                        &mut size,
                    );
                    let _ = CloseHandle(proc);
                    if qok.is_ok() && size > 0 {
                        let path =
                            String::from_utf16_lossy(&buf16[..size as usize]).to_ascii_lowercase();
                        out.push(path);
                        got_full = true;
                    }
                }
                if !got_full {
                    // Elevated / protected: keep file name so exe_paths_equivalent / stem match work.
                    let raw = &entry.szExeFile;
                    let len = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
                    if len > 0 {
                        let name = String::from_utf16_lossy(&raw[..len]).to_ascii_lowercase();
                        if !name.is_empty() {
                            out.push(name);
                        }
                    }
                }
            }
            ok = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// Cheap running-set fingerprint (Toolhelp names only — no OpenProcess).
/// Used to wake tray reconcile when a new process appears (e.g. PixPin just launched).
pub fn process_set_fingerprint() -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let mut names: Vec<String> = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return 0;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut ok = Process32FirstW(snap, &mut entry).is_ok();
        while ok {
            if entry.th32ProcessID > 4 {
                let raw = &entry.szExeFile;
                let len = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
                if len > 0 {
                    names.push(String::from_utf16_lossy(&raw[..len]).to_ascii_lowercase());
                }
            }
            ok = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    names.sort_unstable();
    let mut h = DefaultHasher::new();
    names.len().hash(&mut h);
    for n in &names {
        n.hash(&mut h);
    }
    h.finish()
}

/// HWND → image path for GetRect(uid) probes. Includes **hidden** top-level windows
/// (tray-only hosts often have no visible main window).
fn enum_exe_hwnds_unlocked() -> Vec<(isize, String)> {
    use windows::Win32::Foundation::{BOOL, LPARAM};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId};

    let mut out: Vec<(isize, String)> = Vec::new();

    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let buf = &mut *(lparam.0 as *mut Vec<(isize, String)>);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 || pid <= 4 {
            return BOOL(1);
        }
        let Ok(proc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return BOOL(1);
        };
        let mut buf16 = [0u16; 520];
        let mut size = buf16.len() as u32;
        let ok = QueryFullProcessImageNameW(
            proc,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf16.as_mut_ptr()),
            &mut size,
        );
        let _ = windows::Win32::Foundation::CloseHandle(proc);
        if ok.is_ok() && size > 0 {
            let path = String::from_utf16_lossy(&buf16[..size as usize]).to_ascii_lowercase();
            buf.push((hwnd.0 as isize, path));
        }
        BOOL(1)
    }

    unsafe {
        let ptr = &mut out as *mut Vec<(isize, String)> as isize;
        let _ = EnumWindows(Some(callback), LPARAM(ptr));
    }
    out
}

fn notify_rect_ok(ident: &NOTIFYICONIDENTIFIER) -> bool {
    unsafe { Shell_NotifyIconGetRect(ident).is_ok() }
}

fn is_running_guid(guid_str: &str) -> bool {
    let Some(guid) = parse_guid(guid_str) else {
        return false;
    };
    let identifier = NOTIFYICONIDENTIFIER {
        cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
        guidItem: guid,
        ..Default::default()
    };
    notify_rect_ok(&identifier)
}

fn is_running_uid(uid: u32, exe_path: &str, windows: &[(isize, String)]) -> bool {
    for (hwnd, path) in windows {
        if !exe_paths_equivalent(path, exe_path) {
            continue;
        }
        let identifier = NOTIFYICONIDENTIFIER {
            cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: HWND(*hwnd as _),
            uID: uid,
            ..Default::default()
        };
        if notify_rect_ok(&identifier) {
            return true;
        }
    }
    false
}

/// Snapshot current IsPromoted values (for temporary demote / restore).
pub fn snapshot_promoted() -> Vec<(String, u32)> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(settings) =
        hkcu.open_subkey_with_flags(r"Control Panel\NotifyIconSettings", KEY_READ)
    else {
        return Vec::new();
    };
    let Ok(raw) = settings.get_raw_value("UIOrderList") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for chunk in raw.bytes.chunks(8) {
        if chunk.len() < 8 {
            break;
        }
        let key = u64::from_le_bytes(chunk.try_into().unwrap()).to_string();
        let Ok(regkey) = settings.open_subkey_with_flags(&key, KEY_READ) else {
            continue;
        };
        let promoted: u32 = regkey.get_value("IsPromoted").unwrap_or(0);
        out.push((key, promoted));
    }
    out
}

pub fn set_promoted(key: &str, value: u32) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let settings = hkcu
        .open_subkey_with_flags(r"Control Panel\NotifyIconSettings", KEY_ALL_ACCESS)
        .map_err(|e| e.to_string())?;
    let regkey = settings
        .open_subkey_with_flags(key, KEY_ALL_ACCESS)
        .map_err(|e| e.to_string())?;
    regkey.set_value("IsPromoted", &value).map_err(|e| e.to_string())
}

/// Demote every icon into overflow so the Win11 chevron / overflow island can exist.
pub fn demote_all_to_overflow() -> Result<(), String> {
    let snap = snapshot_promoted();
    for (key, _) in snap {
        let _ = set_promoted(&key, 0);
    }
    Ok(())
}

pub fn restore_promoted(snap: &[(String, u32)]) {
    for (key, value) in snap {
        let _ = set_promoted(key, *value);
    }
}

/// Ensure the overflow chevron is not force-hidden by policy/settings.
pub fn enable_chevron() -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let settings = hkcu
        .open_subkey_with_flags(
            r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\TrayNotify",
            KEY_ALL_ACCESS,
        )
        .map_err(|e| e.to_string())?;
    settings
        .set_value("SystemTrayChevronVisibility", &1u32)
        .map_err(|e| e.to_string())
}

fn process_alive_by_path(exe_path: &str, processes: &[String]) -> bool {
    if exe_path.is_empty() {
        return false;
    }
    processes.iter().any(|p| exe_paths_equivalent(p, exe_path))
}

fn process_alive_by_stem(stem: &str, processes: &[String]) -> bool {
    if stem.is_empty() {
        return false;
    }
    processes.iter().any(|p| {
        Path::new(p)
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case(stem))
    })
}

/// Icons Windows still considers present / process still running.
/// `fast`: skip `Shell_NotifyIconGetRect` — can hang while Explorer/AppBar settles.
pub fn enum_running(fast: bool) -> Vec<RegTrayIcon> {
    let _guard = REGISTRY_ENUM_LOCK.lock();
    let processes = enum_process_image_paths_unlocked();
    let windows = if fast {
        Vec::new()
    } else {
        enum_exe_hwnds_unlocked()
    };
    enum_running_with_processes(fast, &processes, &windows)
}

fn enum_running_with_processes(
    fast: bool,
    processes: &[String],
    windows: &[(isize, String)],
) -> Vec<RegTrayIcon> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(settings) =
        hkcu.open_subkey_with_flags(r"Control Panel\NotifyIconSettings", KEY_READ)
    else {
        eprintln!("[tray] NotifyIconSettings not readable");
        return Vec::new();
    };
    let Ok(raw) = settings.get_raw_value("UIOrderList") else {
        eprintln!("[tray] UIOrderList missing");
        return Vec::new();
    };

    let mut registers = Vec::new();

    for chunk in raw.bytes.chunks(8) {
        if chunk.len() < 8 {
            break;
        }
        let key = u64::from_le_bytes(chunk.try_into().unwrap()).to_string();
        let Ok(regkey) = settings.open_subkey_with_flags(&key, KEY_READ) else {
            continue;
        };

        let path_with_guid: String = regkey.get_value("ExecutablePath").unwrap_or_default();
        let executable_path = PathBuf::from(&path_with_guid);
        let process = process_stem(&executable_path);
        let icon_snapshot = regkey
            .get_raw_value("IconSnapShot")
            .map(|v| v.bytes)
            .unwrap_or_default();
        let initial_tooltip: Option<String> = regkey.get_value("InitialTooltip").ok();
        let icon_guid: Option<String> = regkey
            .get_value("IconGuid")
            .ok()
            .map(|s: String| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let icon_uid: Option<u32> = regkey.get_value("UID").ok();
        let is_promoted: u32 = regkey.get_value("IsPromoted").unwrap_or(0);

        let rect_ok = if fast {
            false
        } else if let Some(ref guid) = icon_guid {
            is_running_guid(guid)
        } else if let Some(uid) = icon_uid {
            if path_with_guid.is_empty() {
                false
            } else {
                is_running_uid(uid, &path_with_guid, windows)
            }
        } else {
            false
        };
        let stem_alive = process_alive_by_stem(&process, processes);
        let path_alive = process_alive_by_path(&path_with_guid, processes);
        let is_running = if fast {
            stem_alive || path_alive
        } else {
            rect_ok || stem_alive || path_alive
        };

        if !is_running {
            continue;
        }

        registers.push(RegTrayIcon {
            key,
            executable_path,
            process,
            icon_snapshot,
            initial_tooltip,
            icon_guid,
            icon_uid,
            is_promoted: is_promoted == 1,
        });
    }

    if std::env::var_os("WH_TRAY_REGISTRY_LOG").is_some() {
        eprintln!(
            "[tray] registry running icons: {} (of {} order entries, processes={})",
            registers.len(),
            raw.bytes.len() / 8,
            processes.len()
        );
    }
    registers
}

/// Soft pool for matching / seeding when GetRect under-counts or hook missed NIM_ADD.
pub fn enum_match_pool(fast: bool) -> Vec<RegTrayIcon> {
    let _guard = REGISTRY_ENUM_LOCK.lock();
    let processes = enum_process_image_paths_unlocked();
    let windows = if fast {
        Vec::new()
    } else {
        enum_exe_hwnds_unlocked()
    };
    let mut base = enum_running_with_processes(fast, &processes, &windows);
    let seen: std::collections::HashSet<String> = base.iter().map(|r| r.key.clone()).collect();

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(settings) =
        hkcu.open_subkey_with_flags(r"Control Panel\NotifyIconSettings", KEY_READ)
    else {
        return base;
    };
    let Ok(raw) = settings.get_raw_value("UIOrderList") else {
        return base;
    };

    for chunk in raw.bytes.chunks(8) {
        if chunk.len() < 8 {
            break;
        }
        let key = u64::from_le_bytes(chunk.try_into().unwrap()).to_string();
        if seen.contains(&key) {
            continue;
        }
        let Ok(regkey) = settings.open_subkey_with_flags(&key, KEY_READ) else {
            continue;
        };
        let path_with_guid: String = regkey.get_value("ExecutablePath").unwrap_or_default();
        let executable_path = PathBuf::from(&path_with_guid);
        let process = process_stem(&executable_path);
        let icon_snapshot = regkey
            .get_raw_value("IconSnapShot")
            .map(|v| v.bytes)
            .unwrap_or_default();
        let initial_tooltip: Option<String> = regkey.get_value("InitialTooltip").ok();
        if icon_snapshot.is_empty()
            && initial_tooltip.as_ref().map(|t| t.trim().is_empty()).unwrap_or(true)
        {
            continue;
        }
        if !process_alive_by_stem(&process, &processes)
            && !process_alive_by_path(&path_with_guid, &processes)
        {
            continue;
        }
        let icon_guid: Option<String> = regkey
            .get_value("IconGuid")
            .ok()
            .map(|s: String| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let icon_uid: Option<u32> = regkey.get_value("UID").ok();
        let is_promoted: u32 = regkey.get_value("IsPromoted").unwrap_or(0);
        base.push(RegTrayIcon {
            key,
            executable_path,
            process,
            icon_snapshot,
            initial_tooltip,
            icon_guid,
            icon_uid,
            is_promoted: is_promoted == 1,
        });
    }
    base
}
