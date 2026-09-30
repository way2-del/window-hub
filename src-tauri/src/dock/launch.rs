//! Launch / focus dock items.

use super::DockItem;
use crate::win32::enum_windows::{focus_window, list_windows, WindowInfo};

pub fn matching_windows(item: &DockItem, windows: &[WindowInfo]) -> Vec<WindowInfo> {
    windows
        .iter()
        .filter(|w| item_matches_window(item, w))
        .cloned()
        .collect()
}

#[cfg(windows)]
fn window_preview_score(w: &WindowInfo) -> (i32, i64) {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, IsIconic, IsWindow, IsWindowVisible,
    };
    let hwnd = HWND(w.hwnd as *mut _);
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return (0, 0);
        }
        let iconic = IsIconic(hwnd).as_bool();
        let visible = IsWindowVisible(hwnd).as_bool();
        let mut rect = RECT::default();
        let area = if GetWindowRect(hwnd, &mut rect).is_ok() {
            let ww = (rect.right - rect.left).max(0) as i64;
            let hh = (rect.bottom - rect.top).max(0) as i64;
            ww.saturating_mul(hh)
        } else {
            0
        };
        let rank = if !iconic && visible {
            3
        } else if visible {
            2
        } else if iconic {
            1
        } else {
            0
        };
        (rank, area)
    }
}

/// Matching windows, best thumbnail/focus candidates first.
pub fn matching_windows_ranked(item: &DockItem, windows: &[WindowInfo]) -> Vec<WindowInfo> {
    let mut matched = matching_windows(item, windows);
    #[cfg(windows)]
    {
        matched.sort_by(|a, b| window_preview_score(b).cmp(&window_preview_score(a)));
    }
    matched
}

/// Prefer a visible, non-minimized top-level match for thumbnails / focus.
pub fn best_matching_window(item: &DockItem, windows: &[WindowInfo]) -> Option<WindowInfo> {
    matching_windows_ranked(item, windows).into_iter().next()
}

pub fn item_matches_window(item: &DockItem, w: &WindowInfo) -> bool {
    if item.kind != "app" {
        return false;
    }
    let win_aumid = w.aumid.as_deref().unwrap_or("").trim();
    let item_aumid = item.virtual_path.trim();
    if !win_aumid.is_empty() {
        if !item_aumid.is_empty() && item_aumid.eq_ignore_ascii_case(win_aumid) {
            return true;
        }
        if crate::dock::icon::path_matches_aumid(&item.real_path, win_aumid)
            || crate::dock::icon::path_matches_aumid(&item.launch_path, win_aumid)
        {
            return true;
        }
    }
    let want = item.match_exe.to_ascii_lowercase();
    if want.is_empty() {
        return false;
    }
    let real = item.real_path.to_ascii_lowercase();
    let exe_name = w
        .exe_name
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    let exe_path = w.exe.as_deref().unwrap_or("").to_ascii_lowercase();
    if !exe_name.is_empty()
        && (exe_name == want
            || format!("{exe_name}.exe") == want
            || exe_name == want.trim_end_matches(".exe"))
    {
        return true;
    }
    if !real.is_empty() && !exe_path.is_empty() && exe_path == real {
        return true;
    }
    false
}

pub fn launch_or_focus(item: &DockItem) -> Result<(), String> {
    match item.kind.as_str() {
        "startmenu" => open_start_menu(),
        "trash" => open_trash(),
        "separator" => Ok(()),
        _ => {
            let wins = list_windows(None);
            let matched = matching_windows(item, &wins);
            if let Some(w) = best_matching_window(item, &wins).or_else(|| matched.first().cloned()) {
                focus_window(w.hwnd)
            } else {
                launch_app(item)
            }
        }
    }
}

fn open_start_menu() -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            keybd_event, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VK_LWIN,
        };
        unsafe {
            keybd_event(VK_LWIN.0 as u8, 0, KEYEVENTF_EXTENDEDKEY, 0);
            keybd_event(
                VK_LWIN.0 as u8,
                0,
                KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP,
                0,
            );
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err("start menu only on Windows".into())
    }
}

fn open_trash() -> Result<(), String> {
    shell_open("shell:RecycleBinFolder", None)
}

/// Empty the system Recycle Bin (shows the OS confirmation UI).
pub fn empty_recycle_bin() -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::Shell::SHEmptyRecycleBinW;

        // Flags 0 → keep Windows confirmation / progress / sound.
        match unsafe { SHEmptyRecycleBinW(HWND::default(), PCWSTR::null(), 0) } {
            Ok(()) => Ok(()),
            Err(err) => {
                let code = err.code().0;
                // HRESULT_FROM_WIN32(ERROR_CANCELLED) — user dismissed the prompt.
                if code == -2147023673 || code == 1223 {
                    return Ok(());
                }
                // Already empty / nothing to delete.
                if code == 0 || code == 1 {
                    return Ok(());
                }
                Err(format!("清空回收站失败 (0x{:08X})", code as u32))
            }
        }
    }
    #[cfg(not(windows))]
    {
        Err("仅支持 Windows".into())
    }
}

fn launch_app(item: &DockItem) -> Result<(), String> {
    if item.uwp && !item.virtual_path.is_empty() {
        let uri = format!("shell:AppsFolder\\{}", item.virtual_path);
        return shell_open(&uri, None);
    }
    let path = if !item.launch_path.is_empty() {
        item.launch_path.as_str()
    } else if !item.real_path.is_empty() {
        item.real_path.as_str()
    } else {
        return Err("no launch path".into());
    };
    // .lnk and exe both work via ShellExecute
    shell_open(path, None)
}

pub(crate) fn shell_open(file: &str, params: Option<&str>) -> Result<(), String> {
    shell_execute(file, params, "open")
}

pub(crate) fn shell_runas(file: &str, params: Option<&str>) -> Result<(), String> {
    shell_execute(file, params, "runas")
}

fn shell_execute(file: &str, params: Option<&str>, verb: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let file_w: Vec<u16> = std::ffi::OsStr::new(file)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let params_w: Option<Vec<u16>> = params.map(|p| {
            std::ffi::OsStr::new(p)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        });
        let op: Vec<u16> = std::ffi::OsStr::new(verb)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            let ret = ShellExecuteW(
                HWND::default(),
                PCWSTR(op.as_ptr()),
                PCWSTR(file_w.as_ptr()),
                params_w
                    .as_ref()
                    .map(|p| PCWSTR(p.as_ptr()))
                    .unwrap_or(PCWSTR::null()),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
            // HINSTANCE > 32 means success
            if ret.0 as isize <= 32 {
                return Err(format!(
                    "ShellExecute({verb}) failed for {file} (code {})",
                    ret.0 as isize
                ));
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (file, params, verb);
        Err("launch only on Windows".into())
    }
}
