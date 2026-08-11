//! HKCU\Control Panel\NotifyIconSettings — Win10/11 tray identity & order.
//!
//! Used for **enrichment only** (tooltip / snapshot / promoted vs overflow area).
//! Existence and clicks come from the explorer tray hook, not registry demote.

#![cfg(windows)]

use std::path::{Path, PathBuf};

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

fn enum_top_level_exe_hwnds() -> Vec<(isize, String)> {
    use parking_lot::Mutex;
    use windows::Win32::Foundation::{BOOL, LPARAM};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible,
    };

    static BUF: Mutex<Vec<(isize, String)>> = Mutex::new(Vec::new());
    {
        BUF.lock().clear();
    }

    unsafe extern "system" fn callback(hwnd: HWND, _: LPARAM) -> BOOL {
        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return BOOL(1);
        }
        let Ok(proc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return BOOL(1);
        };
        let mut buf = [0u16; 520];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            proc,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut size,
        );
        let _ = windows::Win32::Foundation::CloseHandle(proc);
        if ok.is_ok() && size > 0 {
            let path = String::from_utf16_lossy(&buf[..size as usize]).to_ascii_lowercase();
            BUF.lock().push((hwnd.0 as isize, path));
        }
        BOOL(1)
    }

    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(0));
    }
    BUF.lock().clone()
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

fn is_running_uid(uid: u32, exe_lower: &str, windows: &[(isize, String)]) -> bool {
    for (hwnd, path) in windows {
        if path != exe_lower {
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

fn exe_process_alive(exe_lower: &str, windows: &[(isize, String)]) -> bool {
    if exe_lower.is_empty() {
        return false;
    }
    windows.iter().any(|(_, path)| path == exe_lower)
}

/// Icons Windows still considers present (GetRect / process / soft heuristics).
/// Order follows `UIOrderList` (taskbar promoted first, then overflow) — same as Seelen.
pub fn enum_running() -> Vec<RegTrayIcon> {
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

    let windows = enum_top_level_exe_hwnds();
    let mut registers = Vec::new();

    for chunk in raw.bytes.chunks(8) {
        if chunk.len() < 8 {
            break;
        }
        let key = u64::from_le_bytes(chunk.try_into().unwrap()).to_string();
        let Ok(regkey) = settings.open_subkey_with_flags(&key, KEY_READ) else {
            continue;
        };

        // ExecutablePath may be missing for some shell / system icons.
        let path_with_guid: String = regkey.get_value("ExecutablePath").unwrap_or_default();
        let executable_path = PathBuf::from(&path_with_guid);
        let exe_lower = path_with_guid.to_ascii_lowercase();
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

        let rect_ok = if let Some(ref guid) = icon_guid {
            is_running_guid(guid)
        } else if let Some(uid) = icon_uid {
            if exe_lower.is_empty() {
                false
            } else {
                is_running_uid(uid, &exe_lower, &windows)
            }
        } else {
            false
        };
        let stem_alive = !process.is_empty()
            && windows.iter().any(|(_, p)| {
                Path::new(p)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.eq_ignore_ascii_case(&process))
            });
        let is_running = rect_ok || exe_process_alive(&exe_lower, &windows) || stem_alive;

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

    // Hot path — quiet by default (reconcile can run often at startup).
    if std::env::var_os("WH_TRAY_REGISTRY_LOG").is_some() {
        eprintln!(
            "[tray] registry running icons: {} (of {} order entries)",
            registers.len(),
            raw.bytes.len() / 8
        );
    }
    registers
}

/// Soft pool for matching UIA overflow names when GetRect under-counts.
pub fn enum_match_pool() -> Vec<RegTrayIcon> {
    let mut base = enum_running();
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
    let windows = enum_top_level_exe_hwnds();

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
        let stem_alive = !process.is_empty()
            && windows.iter().any(|(_, p)| {
                Path::new(p)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.eq_ignore_ascii_case(&process))
            });
        if !stem_alive && !exe_process_alive(&path_with_guid.to_ascii_lowercase(), &windows) {
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
