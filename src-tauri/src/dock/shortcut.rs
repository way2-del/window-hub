//! Resolve `.lnk` → target / args (for dock matching / Edge PWA `--app-id`).

use std::path::Path;

/// Resolved shell shortcut.
#[derive(Debug, Clone, Default)]
pub struct LnkInfo {
    pub target: String,
    pub args: String,
}

/// Best-effort: follow a shell shortcut to its target file path.
/// Non-Windows / failure → `None`.
pub fn resolve_lnk_target(lnk_path: &str) -> Option<String> {
    resolve_lnk_info(lnk_path).map(|i| i.target).filter(|t| !t.is_empty())
}

/// Target + arguments (needed for Edge/Chrome PWA shortcuts).
pub fn resolve_lnk_info(lnk_path: &str) -> Option<LnkInfo> {
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

/// Extract Chromium/Edge `--app-id=` from shortcut args or a raw command line.
pub fn parse_browser_app_id(args_or_cmd: &str) -> Option<String> {
    let s = args_or_cmd.trim();
    if s.is_empty() {
        return None;
    }
    // --app-id=xxx  /  --app-id xxx  (case-insensitive flag)
    let lower = s.to_ascii_lowercase();
    let Some(pos) = lower.find("--app-id") else {
        return None;
    };
    let rest = s[pos + "--app-id".len()..].trim_start();
    let id = if let Some(stripped) = rest.strip_prefix('=') {
        token_until_ws(stripped)
    } else if rest.starts_with(|c: char| c.is_whitespace()) {
        token_until_ws(rest.trim_start())
    } else {
        return None;
    };
    let id = id.trim_matches('"').trim_matches('\'').trim();
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

/// Prefer a stable app-id from cmdline; else derive from AppUserModelID (`_crx_XXXX`).
pub fn normalize_window_app_id(cmdline_app_id: Option<&str>, aumid: Option<&str>) -> Option<String> {
    if let Some(id) = cmdline_app_id.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(id.to_string());
    }
    let aumid = aumid.map(str::trim).filter(|s| !s.is_empty())?;
    // Chromium / Edge PWA: `…_crx_<appId>` or `MSEdgePWA.<appId>!App`
    let lower = aumid.to_ascii_lowercase();
    if let Some(idx) = lower.find("_crx_") {
        let id = clean_app_id_token(&aumid[idx + 5..]);
        if !id.is_empty() {
            return Some(id);
        }
    }
    for prefix in ["msedgepwa.", "chrome._crx_"] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            // Use original casing slice of equal length when possible.
            let id = clean_app_id_token(&aumid[aumid.len() - rest.len()..]);
            if !id.is_empty() {
                return Some(id);
            }
        }
    }
    None
}

fn clean_app_id_token(raw: &str) -> String {
    // Drop `!App` / profile suffixes: `abc123!App` → `abc123`
    let head = raw.split(['!', '.']).next().unwrap_or(raw);
    token_until_ws(head)
        .trim_matches(|c| c == '!' || c == '.' || c == '"' || c == '\'')
        .to_string()
}

fn token_until_ws(s: &str) -> &str {
    s.split_whitespace().next().unwrap_or("").trim()
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

pub fn is_browser_exe_key(exe: &str) -> bool {
    matches!(
        normalize_exe_key(exe).as_str(),
        "msedge.exe"
            | "msedge_proxy.exe"
            | "chrome.exe"
            | "chrome_proxy.exe"
            | "brave.exe"
            | "brave_proxy.exe"
            | "firefox.exe"
            | "opera.exe"
            | "opera_proxy.exe"
    )
}

#[cfg(windows)]
fn resolve_lnk_windows(lnk_path: &str) -> Option<LnkInfo> {
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
            let target = if len == 0 {
                String::new()
            } else {
                String::from_utf16_lossy(&buf[..len])
            };

            let mut args_buf = [0u16; 1024];
            let args = if link.GetArguments(&mut args_buf).is_ok() {
                let alen = args_buf.iter().position(|&c| c == 0).unwrap_or(args_buf.len());
                String::from_utf16_lossy(&args_buf[..alen])
            } else {
                String::new()
            };

            if target.is_empty() && args.is_empty() {
                None
            } else {
                Some(LnkInfo { target, args })
            }
        })();
        if owned_com {
            CoUninitialize();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_app_id_equals() {
        assert_eq!(
            parse_browser_app_id("--profile-directory=Default --app-id=abc123XYZ --app-url=https://x"),
            Some("abc123XYZ".into())
        );
    }

    #[test]
    fn parses_app_id_space() {
        assert_eq!(
            parse_browser_app_id("--app-id ehccgonefendpahldcmdgdmbkpbdmhbg"),
            Some("ehccgonefendpahldcmdgdmbkpbdmhbg".into())
        );
    }

    #[test]
    fn normalizes_crx_aumid() {
        assert_eq!(
            normalize_window_app_id(None, Some("MSEdge._crx_abcDEF123")),
            Some("abcDEF123".into())
        );
    }

    #[test]
    fn normalizes_msedgepwa_aumid() {
        assert_eq!(
            normalize_window_app_id(None, Some("MSEdgePWA.abcDEF123!App")),
            Some("abcDEF123".into())
        );
    }
}
