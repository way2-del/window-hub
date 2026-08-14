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
    let rest = &s[pos + "--app-id".len()..];
    let id = if let Some(stripped) = rest.trim_start().strip_prefix('=') {
        token_until_ws(stripped.trim_start())
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
///
/// Edge “安装的应用” often share `msedge.exe` and expose a site AUMID like
/// `chatgpt.com-DFCB3CE4_ch69rtgtz055j!App` (no `--app-id` on the process).
/// Those resolve to the Chromium `--app-id` via Start Menu / Desktop `.lnk` when possible,
/// otherwise fall back to `site:{host}` so Dock can still split them from the browser.
pub fn normalize_window_app_id(cmdline_app_id: Option<&str>, aumid: Option<&str>) -> Option<String> {
    if let Some(id) = cmdline_app_id.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(id.to_string());
    }
    let aumid = aumid.map(str::trim).filter(|s| !s.is_empty())?;
    let lower = aumid.to_ascii_lowercase();
    // Bare browser identity — not an installed app.
    if lower == "msedge" || lower == "chrome" || lower == "brave" {
        return None;
    }
    // Chromium / Edge PWA: `…_crx_<appId>` / `_crx__<appId>` or `MSEdgePWA.<appId>!App`
    if let Some(idx) = lower.find("_crx_") {
        let id = clean_app_id_token(&aumid[idx + 5..]);
        if !id.is_empty() {
            return Some(id);
        }
    }
    for prefix in ["msedgepwa.", "chrome._crx_"] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let id = clean_app_id_token(&aumid[aumid.len() - rest.len()..]);
            if !id.is_empty() {
                return Some(id);
            }
        }
    }
    // Site / installed-web-app AUMID → map to shortcut --app-id when possible.
    if let Some(host) = parse_site_app_host(aumid) {
        if let Some(id) = lookup_app_id_by_host(&host) {
            return Some(id);
        }
        return Some(format!("site:{host}"));
    }
    None
}

/// `chatgpt.com-DFCB3CE4_ch69rtgtz055j!App` → `chatgpt.com`
pub fn parse_site_app_host(aumid: &str) -> Option<String> {
    let stem = aumid
        .split('!')
        .next()
        .unwrap_or(aumid)
        .trim();
    if stem.is_empty() {
        return None;
    }
    let lower = stem.to_ascii_lowercase();
    if lower == "msedge" || lower == "chrome" || lower.starts_with("msedgepwa.") {
        return None;
    }
    // `{host}-{8 hex}_{suffix}` — host may contain dots (chatgpt.com).
    let bytes = stem.as_bytes();
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        if bytes[i] != b'-' {
            continue;
        }
        let after = &stem[i + 1..];
        let hex = after.split('_').next().unwrap_or("");
        if hex.len() == 8 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            let host = stem[..i].trim().to_ascii_lowercase();
            if host.contains('.') || host.contains("localhost") {
                return Some(host);
            }
        }
    }
    None
}

/// Extract host from `--app-url=https://chatgpt.com/…`.
pub fn parse_app_url_host(args_or_cmd: &str) -> Option<String> {
    let s = args_or_cmd.trim();
    if s.is_empty() {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    let Some(pos) = lower.find("--app-url") else {
        return None;
    };
    let rest = s[pos + "--app-url".len()..].trim_start();
    let raw = if let Some(stripped) = rest.strip_prefix('=') {
        token_until_ws(stripped)
    } else if rest.starts_with(|c: char| c.is_whitespace()) {
        token_until_ws(rest.trim_start())
    } else {
        return None;
    };
    let url = raw.trim_matches('"').trim_matches('\'').trim();
    host_from_url(url)
}

fn host_from_url(url: &str) -> Option<String> {
    let u = url.trim();
    let rest = if let Some(r) = u
        .strip_prefix("https://")
        .or_else(|| u.strip_prefix("http://"))
        .or_else(|| u.strip_prefix("HTTPS://"))
        .or_else(|| u.strip_prefix("HTTP://"))
    {
        r
    } else {
        u
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("").trim();
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    // drop userinfo / port
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = if host.matches(':').count() == 1 && !host.contains(']') {
        host.split(':').next().unwrap_or(host)
    } else {
        host
    };
    let host = host.trim().trim_start_matches("www.").to_ascii_lowercase();
    if host.is_empty() || !host.contains('.') && host != "localhost" {
        None
    } else {
        Some(host)
    }
}

fn clean_app_id_token(raw: &str) -> String {
    // Drop `!App` / profile suffixes: `abc123!App` → `abc123`
    // Folder names use `_crx__id` (double underscore) — strip leading `_`.
    let head = raw.split(['!', '.']).next().unwrap_or(raw);
    token_until_ws(head)
        .trim_matches(|c| c == '!' || c == '.' || c == '"' || c == '\'' || c == '_')
        .to_string()
}

/// Cached Start Menu / Desktop / Web Applications scan: host → chromium app-id.
fn lookup_app_id_by_host(host: &str) -> Option<String> {
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let map = installed_app_host_index();
    if let Some(id) = map.get(&host) {
        return Some(id.clone());
    }
    // www-stripped keys already; also try with www. for odd shortcuts.
    map.get(&format!("www.{host}")).cloned()
}

fn installed_app_host_index() -> std::collections::HashMap<String, String> {
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    struct Cache {
        built_at: Instant,
        map: std::collections::HashMap<String, String>,
    }
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let lock = CACHE.get_or_init(|| {
        Mutex::new(Cache {
            built_at: Instant::now()
                .checked_sub(Duration::from_secs(3600))
                .unwrap_or_else(Instant::now),
            map: std::collections::HashMap::new(),
        })
    });
    let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    if guard.built_at.elapsed() < Duration::from_secs(30) && !guard.map.is_empty() {
        return guard.map.clone();
    }
    let map = scan_installed_app_hosts();
    guard.built_at = Instant::now();
    guard.map = map.clone();
    map
}

fn scan_installed_app_hosts() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for root in browser_app_lnk_roots() {
        walk_lnk_hosts(&root, &mut map, 0);
    }
    map
}

fn browser_app_lnk_roots() -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    if let Some(u) = std::env::var_os("USERPROFILE") {
        roots.push(std::path::PathBuf::from(u).join("Desktop"));
    }
    if let Some(a) = std::env::var_os("APPDATA") {
        let a = std::path::PathBuf::from(a);
        roots.push(
            a.join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs"),
        );
        roots.push(
            a.join("Microsoft")
                .join("Internet Explorer")
                .join("Quick Launch")
                .join("User Pinned")
                .join("TaskBar"),
        );
    }
    if let Some(a) = std::env::var_os("ProgramData") {
        roots.push(
            std::path::PathBuf::from(a)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs"),
        );
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let local = std::path::PathBuf::from(local);
        for browser in ["Microsoft\\Edge", "Google\\Chrome", "BraveSoftware\\Brave-Browser"] {
            let base = local.join(browser).join("User Data");
            if let Ok(profiles) = std::fs::read_dir(&base) {
                for ent in profiles.flatten() {
                    let p = ent.path();
                    if !p.is_dir() {
                        continue;
                    }
                    let name = ent.file_name().to_string_lossy().to_ascii_lowercase();
                    if name == "system profile" || name.starts_with("crashpad") {
                        continue;
                    }
                    roots.push(p.join("Web Applications"));
                }
            }
        }
    }
    roots
}

fn walk_lnk_hosts(
    dir: &std::path::Path,
    map: &mut std::collections::HashMap<String, String>,
    depth: u8,
) {
    if depth > 5 || !dir.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for ent in entries.flatten() {
        let p = ent.path();
        if p.is_dir() {
            walk_lnk_hosts(&p, map, depth + 1);
            continue;
        }
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext != "lnk" {
            continue;
        }
        let Some(info) = resolve_lnk_info(&p.to_string_lossy()) else {
            continue;
        };
        let Some(app_id) = parse_browser_app_id(&info.args) else {
            continue;
        };
        if let Some(host) = parse_app_url_host(&info.args) {
            map.entry(host).or_insert(app_id);
        }
    }
}

fn token_until_ws(s: &str) -> &str {
    s.split_whitespace().next().unwrap_or("").trim()
}

/// Upgrade `site:{host}` → chromium `--app-id` when a matching `.lnk` exists.
pub fn canonicalize_browser_app_id(app_id: &str) -> String {
    let id = app_id.trim();
    if let Some(host) = id.strip_prefix("site:") {
        if let Some(real) = lookup_app_id_by_host(host) {
            return real;
        }
    }
    id.to_string()
}

/// Args to launch an Edge/Chrome installed app (chromium id or `site:{host}`).
pub fn launch_args_for_browser_app(app_id: &str) -> String {
    let id = canonicalize_browser_app_id(app_id);
    if let Some(host) = id.strip_prefix("site:") {
        return format!("--profile-directory=Default --app=https://{host}/");
    }
    format!("--profile-directory=Default --app-id={id}")
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

    #[test]
    fn parses_site_aumid_host() {
        assert_eq!(
            parse_site_app_host("chatgpt.com-DFCB3CE4_ch69rtgtz055j!App"),
            Some("chatgpt.com".into())
        );
        assert_eq!(
            parse_site_app_host("gemini.google.com-D0A8E439_vn3jms8s81tkg!App"),
            Some("gemini.google.com".into())
        );
        assert_eq!(parse_site_app_host("MSEdge"), None);
    }

    #[test]
    fn parses_app_url_host() {
        assert_eq!(
            parse_app_url_host(
                "--profile-directory=Default --app-id=abc --app-url=https://chatgpt.com/"
            ),
            Some("chatgpt.com".into())
        );
        assert_eq!(
            parse_app_url_host("--app-url=https://www.gemini.google.com/app"),
            Some("gemini.google.com".into())
        );
    }

    #[test]
    fn strips_double_underscore_crx() {
        assert_eq!(
            normalize_window_app_id(None, Some("MSEdge._crx__cadlkienfkclaiaibeoongdcgmdikeeg")),
            Some("cadlkienfkclaiaibeoongdcgmdikeeg".into())
        );
    }

    #[test]
    fn site_aumid_maps_chatgpt_lnk_when_present() {
        // Developer machine with Edge ChatGPT install — otherwise skip.
        let host = parse_site_app_host("chatgpt.com-DFCB3CE4_ch69rtgtz055j!App");
        assert_eq!(host.as_deref(), Some("chatgpt.com"));
        let id = normalize_window_app_id(
            None,
            Some("chatgpt.com-DFCB3CE4_ch69rtgtz055j!App"),
        );
        let id = id.expect("chatgpt aumid");
        if std::path::Path::new(
            &(std::env::var("APPDATA").unwrap_or_default()
                + r"\Microsoft\Windows\Start Menu\Programs\ChatGPT.lnk"),
        )
        .exists()
        {
            assert_eq!(id, "cadlkienfkclaiaibeoongdcgmdikeeg");
        } else {
            assert!(id == "site:chatgpt.com" || id == "cadlkienfkclaiaibeoongdcgmdikeeg");
        }
    }
}
