//! Open files / apps / system locations via ShellExecute.

use std::path::Path;

pub fn open_path(path: &str) -> Result<(), String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("empty path".into());
    }
    // URL
    if path.starts_with("http://") || path.starts_with("https://") || path.starts_with("ms-settings:")
    {
        return shell_open(path, None);
    }
    // shell: special folders
    if path.starts_with("shell:") {
        return shell_open(path, None);
    }
    let p = Path::new(path);
    if !p.exists() {
        // Still try — Some shell URIs / relative
        return shell_open(path, None);
    }
    shell_open(path, None)
}

/// Open Explorer with the item selected (`explorer /select,path`).
pub fn reveal_in_folder(path: &str) -> Result<(), String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("empty path".into());
    }
    if path.starts_with("http://")
        || path.starts_with("https://")
        || path.starts_with("ms-settings:")
        || path.starts_with("shell:")
    {
        return Err("not a filesystem path".into());
    }
    std::process::Command::new("explorer")
        .arg(format!("/select,{path}"))
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn open_system(id: &str) -> Result<(), String> {
    let k = id.trim().to_ascii_lowercase();
    match k.as_str() {
        "settings" | "windows-settings" | "ms-settings" => {
            crate::win32::status_menu::open_system_tool("settings")
        }
        "control" | "control-panel" => crate::win32::status_menu::open_system_tool("control"),
        "computer" | "this-pc" | "my-computer" => shell_open("shell:MyComputerFolder", None),
        "calc" | "calculator" => shell_open("calc.exe", None),
        "mspaint" | "paint" => shell_open("mspaint.exe", None),
        "wordpad" => shell_open("write.exe", None),
        "notepad" => shell_open("notepad.exe", None),
        other => Err(format!("unknown system target: {other}")),
    }
}

fn shell_open(file: &str, params: Option<&str>) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::{w, PCWSTR};
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
        unsafe {
            let rc = ShellExecuteW(
                HWND::default(),
                w!("open"),
                PCWSTR::from_raw(file_w.as_ptr()),
                params_w
                    .as_ref()
                    .map(|v| PCWSTR::from_raw(v.as_ptr()))
                    .unwrap_or(PCWSTR::null()),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
            let code = rc.0 as isize;
            if code <= 32 {
                return Err(format!("ShellExecute failed ({code}) for {file}"));
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (file, params);
        Err("Windows only".into())
    }
}
