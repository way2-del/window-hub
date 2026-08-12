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

pub fn item_matches_window(item: &DockItem, w: &WindowInfo) -> bool {
    if item.kind != "app" {
        return false;
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
            // Genie-parked apps must go through genie_restore_app — focus_window's
            // SW_RESTORE would flash the live window before the expand mesh.
            if crate::dock::genie::item_is_genie_parked(&item.id) {
                return Ok(());
            }
            let wins = list_windows(None);
            let matched = matching_windows(item, &wins);
            if let Some(w) = matched.first() {
                let hwnd = w.hwnd;
                let _ = focus_window(hwnd);
                crate::dock::genie::note_user_foreground(hwnd);
                Ok(())
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

fn shell_open(file: &str, params: Option<&str>) -> Result<(), String> {
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
        let op: Vec<u16> = std::ffi::OsStr::new("open")
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
                return Err(format!("ShellExecute failed for {file} (code {})", ret.0 as isize));
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (file, params);
        Err("launch only on Windows".into())
    }
}
