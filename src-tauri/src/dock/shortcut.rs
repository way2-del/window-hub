//! Resolve `.lnk` → target exe path (for dock matching / real_path).

use std::path::Path;

/// Best-effort: follow a shell shortcut to its target file path.
/// Non-Windows / failure → `None`.
pub fn resolve_lnk_target(lnk_path: &str) -> Option<String> {
    let lnk_path = lnk_path.trim();
    if lnk_path.is_empty() {
        return None;
    }
    let p = Path::new(lnk_path);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext != "lnk" {
        return None;
    }
    if !p.exists() {
        return None;
    }
    #[cfg(windows)]
    {
        resolve_lnk_windows(lnk_path)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// If `path` is `.lnk`, resolve; if `.exe`, return as-is; else `None`.
pub fn resolve_launch_target(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "exe" {
        return Some(path.to_string());
    }
    if ext == "lnk" {
        return resolve_lnk_target(path);
    }
    None
}

pub fn file_name_lower(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Normalize an exe identity for comparison (`Foo` / `foo.exe` / path → `foo.exe`).
pub fn normalize_exe_key(raw: &str) -> String {
    let s = raw.trim().to_ascii_lowercase().replace('/', "\\");
    if s.is_empty() {
        return s;
    }
    let base = if let Some(idx) = s.rfind('\\') {
        &s[idx + 1..]
    } else {
        s.as_str()
    };
    if base.is_empty() {
        return String::new();
    }
    if base.ends_with(".exe") {
        base.to_string()
    } else if base.contains('.') {
        // e.g. weird names — keep as-is lowercased
        base.to_string()
    } else {
        format!("{base}.exe")
    }
}

pub fn normalize_path_key(path: &str) -> String {
    path.trim().to_ascii_lowercase().replace('/', "\\")
}

#[cfg(windows)]
fn resolve_lnk_windows(lnk_path: &str) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{Interface, PCWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED, IPersistFile, STGM_READ,
    };
    use windows::Win32::UI::Shell::{
        IShellLinkW, ShellLink, SLGP_UNCPRIORITY, SLR_NOLINKINFO, SLR_NOSEARCH, SLR_NOTRACK,
        SLR_NOUPDATE, SLR_NO_UI,
    };

    let wide: Vec<u16> = std::ffi::OsStr::new(lnk_path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let owned_com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = (|| {
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
            let persist: IPersistFile = link.cast().ok()?;
            persist.Load(PCWSTR(wide.as_ptr()), STGM_READ).ok()?;
            // Don't hang / show UI when the target is missing.
            let flags = (SLR_NO_UI.0
                | SLR_NOSEARCH.0
                | SLR_NOTRACK.0
                | SLR_NOLINKINFO.0
                | SLR_NOUPDATE.0) as u32;
            let _ = link.Resolve(HWND::default(), flags);

            let mut buf = [0u16; 260];
            link.GetPath(&mut buf, std::ptr::null_mut(), SLGP_UNCPRIORITY.0 as u32)
                .ok()?;
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            if len == 0 {
                return None;
            }
            let path = String::from_utf16_lossy(&buf[..len]);
            if path.is_empty() {
                None
            } else {
                Some(path)
            }
        })();
        if owned_com {
            CoUninitialize();
        }
        result
    }
}
