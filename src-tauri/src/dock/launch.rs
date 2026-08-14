//! Launch / focus dock items.

use super::shortcut::{
    file_name_lower, is_browser_exe_key, normalize_exe_key, normalize_path_key,
    resolve_launch_target,
};
use super::DockItem;
use crate::win32::enum_windows::{focus_or_minimize_group, list_windows, WindowInfo};
use std::path::{Component, Path};

/// Suites that intentionally share one dock pin (launcher ≠ editor process).
const SUITE_FAMILIES: &[&[&str]] = &[
    // WPS / 金山
    &[
        "wps.exe",
        "wpp.exe",
        "et.exe",
        "wpspdf.exe",
        "ksolaunch.exe",
        "wpsoffice.exe",
        "wpscloudsvr.exe",
        "ksomisc.exe",
        "wpscenter.exe",
        "wpsofficeboot.exe",
    ],
    // 腾讯会议
    &["wemeetapp.exe", "wemeetapp_new.exe", "tencentmeeting.exe"],
    // 钉钉
    &["dingtalk.exe", "dingtalk_launcher.exe"],
    // 飞书
    &["feishu.exe", "lark.exe", "feishulauncher.exe"],
    // Spotify
    &["spotify.exe", "spotifylauncher.exe"],
    // Discord
    &["discord.exe", "updat.exe"],
];

/// Same folder / tree but must stay separate dock pins (Word ≠ Excel, GoLand ≠ IDEA…).
const DISTINCT_APPS: &[&[&str]] = &[
    &[
        "winword.exe",
        "excel.exe",
        "powerpnt.exe",
        "outlook.exe",
        "onenote.exe",
        "onenoteim.exe",
        "msaccess.exe",
        "mspub.exe",
        "lync.exe",
        "teams.exe",
        "ms-teams.exe",
    ],
    &[
        "idea64.exe",
        "idea.exe",
        "goland64.exe",
        "goland.exe",
        "webstorm64.exe",
        "webstorm.exe",
        "pycharm64.exe",
        "pycharm.exe",
        "clion64.exe",
        "clion.exe",
        "phpstorm64.exe",
        "phpstorm.exe",
        "rider64.exe",
        "rider.exe",
        "datagrip64.exe",
        "datagrip.exe",
        "rubymine64.exe",
        "rubymine.exe",
        "studio64.exe",
    ],
    &["photoshop.exe", "illustrator.exe", "afterfx.exe", "premiere pro.exe", "acrobat.exe"],
    &["chrome.exe", "msedge.exe", "firefox.exe", "brave.exe", "opera.exe"],
];

/// Path markers that identify a product tree across subfolders.
const PRODUCT_MARKERS: &[&str] = &[
    "\\kingsoft\\",
    "\\wps office\\",
    "\\dingding\\",
    "\\dingtalk\\",
    "\\feishu\\",
    "\\lark\\",
    "\\wemeet\\",
    "\\spotify\\",
    "\\discord\\",
];

/// Generic roots that are too broad to count as a "product tree".
const GENERIC_ROOTS: &[&str] = &[
    "program files",
    "program files (x86)",
    "programfiles",
    "programfiles(x86)",
    "windows",
    "system32",
    "syswow64",
    "users",
    "appdata",
    "local",
    "roaming",
    "locallow",
    "common files",
    "programdata",
    "programs",
];

fn candidate_exe_keys(item: &DockItem) -> Vec<String> {
    let mut keys = Vec::new();
    let push = |keys: &mut Vec<String>, raw: &str| {
        let k = normalize_exe_key(raw);
        if !k.is_empty() && !keys.iter().any(|e| e == &k) {
            keys.push(k);
        }
    };
    push(&mut keys, &item.match_exe);
    if !item.real_path.is_empty() {
        push(&mut keys, &file_name_lower(&item.real_path));
    }
    if !item.launch_path.is_empty() {
        let lp = item.launch_path.to_ascii_lowercase();
        if lp.ends_with(".exe") {
            push(&mut keys, &file_name_lower(&item.launch_path));
        } else if lp.ends_with(".lnk") {
            if let Some(target) = resolve_launch_target(&item.launch_path) {
                push(&mut keys, &file_name_lower(&target));
            }
        }
    }
    expand_suite_keys(&mut keys, item);
    keys
}

fn expand_suite_keys(keys: &mut Vec<String>, item: &DockItem) {
    let label = item.label.to_ascii_lowercase();
    let blob = format!(
        "{} {} {} {}",
        item.match_exe, item.real_path, item.launch_path, item.label
    )
    .to_ascii_lowercase();

    for family in SUITE_FAMILIES {
        let hit = keys.iter().any(|k| family.iter().any(|e| e == &k.as_str()))
            || family.iter().any(|e| {
                let stem = e.trim_end_matches(".exe");
                label.contains(stem) || blob.contains(stem)
            });
        // WPS / 金山 by Chinese label
        let hit = hit
            || (family[0] == "wps.exe" && (label.contains("wps") || label.contains("金山") || blob.contains("kingsoft")));
        if hit {
            for e in *family {
                let s = (*e).to_string();
                if !keys.iter().any(|k| k == &s) {
                    keys.push(s);
                }
            }
        }
    }
}

fn effective_real_paths(item: &DockItem) -> Vec<String> {
    let mut out = Vec::new();
    let push = |out: &mut Vec<String>, raw: &str| {
        let p = normalize_path_key(raw);
        if !p.is_empty() && !out.iter().any(|e| e == &p) {
            out.push(p);
        }
    };
    if !item.real_path.is_empty() {
        push(&mut out, &item.real_path);
    }
    if !item.launch_path.is_empty() {
        let lp = item.launch_path.to_ascii_lowercase();
        if lp.ends_with(".exe") {
            push(&mut out, &item.launch_path);
        } else if lp.ends_with(".lnk") {
            if let Some(target) = resolve_launch_target(&item.launch_path) {
                push(&mut out, &target);
            }
        }
    }
    out
}

fn is_system_dir(path: &str) -> bool {
    let l = path.to_ascii_lowercase().replace('/', "\\");
    l.contains("\\windows\\")
        || l.contains("\\system32")
        || l.contains("\\syswow64")
        || l.contains("\\windowsapps\\")
        || l.ends_with("\\windows")
}

fn same_install_dir(a: &str, b: &str) -> bool {
    let pa = Path::new(a)
        .parent()
        .map(|p| p.to_string_lossy().to_ascii_lowercase());
    let pb = Path::new(b)
        .parent()
        .map(|p| p.to_string_lossy().to_ascii_lowercase());
    match (pa, pb) {
        (Some(a), Some(b)) if !a.is_empty() && a == b => !is_system_dir(&a),
        _ => false,
    }
}

fn path_components(path: &str) -> Vec<String> {
    Path::new(path)
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

fn is_generic_component(s: &str) -> bool {
    GENERIC_ROOTS.iter().any(|g| g == &s)
}

/// Shared product directory tree (launcher in root, app in subfolder) — any vendor.
fn same_install_tree(pin_path: &str, run_path: &str) -> bool {
    let pin_exe = normalize_exe_key(&file_name_lower(pin_path));
    let run_exe = normalize_exe_key(&file_name_lower(run_path));
    if !pin_exe.is_empty() && !run_exe.is_empty() && pin_exe != run_exe {
        for group in DISTINCT_APPS {
            let pin_hit = group.iter().any(|e| *e == pin_exe.as_str());
            let run_hit = group.iter().any(|e| *e == run_exe.as_str());
            if pin_hit && run_hit {
                // Word vs Excel etc. — never merge via tree.
                return false;
            }
        }
        // Different exe names only merge when they belong to a known suite
        // (WPS editor ↔ launcher). Otherwise Local\Programs\A vs B falsely match.
        let mut in_suite = false;
        for family in SUITE_FAMILIES {
            let pin_hit = family.iter().any(|e| *e == pin_exe.as_str());
            let run_hit = family.iter().any(|e| *e == run_exe.as_str());
            if pin_hit && run_hit {
                in_suite = true;
                break;
            }
        }
        if !in_suite {
            return false;
        }
    }

    let a = path_components(pin_path);
    let b = path_components(run_path);
    if a.len() < 2 || b.len() < 2 {
        return false;
    }
    // Drop file names.
    let a_dirs = &a[..a.len() - 1];
    let b_dirs = &b[..b.len() - 1];
    let mut n = 0usize;
    for (x, y) in a_dirs.iter().zip(b_dirs.iter()) {
        if x == y {
            n += 1;
        } else {
            break;
        }
    }
    if n == 0 {
        return false;
    }
    let common: Vec<&str> = a_dirs.iter().take(n).map(|s| s.as_str()).collect();
    // Need a non-generic product segment (e.g. Kingsoft, MyApp).
    let meaningful = common
        .iter()
        .filter(|c| !is_generic_component(c))
        .count();
    // Require 2 meaningful segments so "Users\foo\AppData\Local\Programs" alone
    // cannot merge unrelated apps.
    meaningful >= 2 && n >= 3
}

fn same_product_marker(a: &str, b: &str) -> bool {
    let al = a.to_ascii_lowercase().replace('/', "\\");
    let bl = b.to_ascii_lowercase().replace('/', "\\");
    let a_exe = normalize_exe_key(&file_name_lower(a));
    let b_exe = normalize_exe_key(&file_name_lower(b));
    // Marker alone is too weak across vendors; require same exe or shared suite.
    if !a_exe.is_empty() && !b_exe.is_empty() && a_exe != b_exe {
        let mut in_suite = false;
        for family in SUITE_FAMILIES {
            if in_suite_family(&a_exe, family) && in_suite_family(&b_exe, family) {
                in_suite = true;
                break;
            }
        }
        if !in_suite {
            return false;
        }
    }
    for m in PRODUCT_MARKERS {
        if al.contains(m) && bl.contains(m) {
            return true;
        }
    }
    false
}

fn in_suite_family(exe: &str, family: &[&str]) -> bool {
    let k = normalize_exe_key(exe);
    family.iter().any(|e| *e == k.as_str())
}

fn item_suite_families(item: &DockItem, keys: &[String]) -> Vec<&'static [&'static str]> {
    let mut out = Vec::new();
    let label = item.label.to_ascii_lowercase();
    let blob = format!(
        "{} {} {} {}",
        item.match_exe, item.real_path, item.launch_path, item.label
    )
    .to_ascii_lowercase();
    for family in SUITE_FAMILIES {
        let hit = keys.iter().any(|k| in_suite_family(k, family))
            || family.iter().any(|e| {
                let stem = e.trim_end_matches(".exe");
                label.contains(stem) || blob.contains(stem)
            })
            || (family[0] == "wps.exe"
                && (label.contains("wps") || label.contains("金山") || blob.contains("kingsoft")));
        if hit {
            out.push(*family);
        }
    }
    out
}

pub fn matching_windows(item: &DockItem, windows: &[WindowInfo]) -> Vec<WindowInfo> {
    if item.kind != "app" {
        return Vec::new();
    }
    let pin_app_id = item.app_id.trim();
    // Edge/Chrome installed apps: match strictly by --app-id (never absorb the browser).
    if !pin_app_id.is_empty() {
        return windows
            .iter()
            .filter(|w| {
                w.app_id
                    .as_deref()
                    .map(|id| id.eq_ignore_ascii_case(pin_app_id))
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
    }

    let keys = candidate_exe_keys(item);
    if keys.is_empty() && item.real_path.is_empty() && item.launch_path.is_empty() {
        return Vec::new();
    }
    let pin_is_browser = keys.iter().any(|k| is_browser_exe_key(k))
        || is_browser_exe_key(&item.match_exe)
        || is_browser_exe_key(&file_name_lower(&item.real_path));
    let reals = effective_real_paths(item);
    let suites = item_suite_families(item, &keys);
    windows
        .iter()
        .filter(|w| {
            // Plain browser pin must not claim PWA windows (those have app_id).
            if pin_is_browser {
                if w.app_id.as_ref().map(|s| !s.is_empty()).unwrap_or(false) {
                    return false;
                }
            }
            let exe_name = normalize_exe_key(w.exe_name.as_deref().unwrap_or(""));
            let exe_path = normalize_path_key(w.exe.as_deref().unwrap_or(""));
            if !exe_name.is_empty() && keys.iter().any(|k| k == &exe_name) {
                return true;
            }
            if !exe_path.is_empty() {
                if reals.iter().any(|r| r == &exe_path) {
                    return true;
                }
                let win_base = file_name_lower(&exe_path);
                if !win_base.is_empty() && reals.iter().any(|r| file_name_lower(r) == win_base) {
                    return true;
                }
                if reals.iter().any(|r| same_install_dir(r, &exe_path)) {
                    return true;
                }
                if reals.iter().any(|r| same_product_marker(r, &exe_path)) {
                    return true;
                }
                // Generic: any pin whose install tree contains the running exe.
                if reals.iter().any(|r| same_install_tree(r, &exe_path)) {
                    return true;
                }
            }
            // Known suite families (WPS / 钉钉 / 飞书 / …).
            for family in &suites {
                if !exe_name.is_empty() && in_suite_family(&exe_name, family) {
                    return true;
                }
                if !exe_path.is_empty() && in_suite_family(&file_name_lower(&exe_path), family) {
                    return true;
                }
            }
            // Soft title fallback for WPS suite only (e.g. "Java.docx - WPS Office").
            // Generic label⊂title wrongly merges unrelated windows into a pin.
            if !suites.is_empty() {
                let title = w.title.to_ascii_lowercase();
                if suites.iter().any(|f| f[0] == "wps.exe")
                    && (title.contains("wps") || title.contains("金山"))
                {
                    return true;
                }
            }
            false
        })
        .cloned()
        .collect()
}

pub fn launch_or_focus(item: &DockItem) -> Result<(), String> {
    match item.kind.as_str() {
        "startmenu" => open_start_menu(),
        "trash" => open_trash(),
        "separator" => Ok(()),
        _ => {
            let wins = list_windows(None);
            let mut matched = matching_windows(item, &wins);
            // Broader fallback: exe filename only (helps Cursor when pin path/heal lags).
            // Never for Edge/Chrome PWAs — stem match would steal the whole browser.
            if matched.is_empty() && item.app_id.trim().is_empty() {
                matched = windows_by_exe_stem(item, &wins);
            }
            if !matched.is_empty() {
                // Taskbar toggle: frontmost → minimize; else restore/focus.
                let hwnds: Vec<isize> = matched.iter().map(|w| w.hwnd).collect();
                return focus_or_minimize_group(&hwnds);
            }
            launch_app(item)
        }
    }
}

/// Match running windows by exe stem from pin paths / match_exe / label.
fn windows_by_exe_stem(item: &DockItem, windows: &[WindowInfo]) -> Vec<WindowInfo> {
    let mut stems = Vec::new();
    let push = |out: &mut Vec<String>, raw: &str| {
        let k = normalize_exe_key(&file_name_lower(raw));
        if !k.is_empty() && !out.iter().any(|e| e == &k) {
            out.push(k);
        }
    };
    push(&mut stems, &item.match_exe);
    push(&mut stems, &item.real_path);
    push(&mut stems, &item.launch_path);
    let label = item.label.trim().to_ascii_lowercase();
    if !label.is_empty() {
        // "Cursor" → cursor.exe
        let guess = format!("{label}.exe");
        push(&mut stems, &guess);
    }
    if stems.is_empty() {
        return Vec::new();
    }
    windows
        .iter()
        .filter(|w| {
            let n = normalize_exe_key(w.exe_name.as_deref().unwrap_or(""));
            let base = normalize_exe_key(&file_name_lower(w.exe.as_deref().unwrap_or("")));
            stems.iter().any(|s| s == &n || s == &base)
        })
        .cloned()
        .collect()
}

pub fn open_start_menu() -> Result<(), String> {
    #[cfg(windows)]
    {
        crate::win32::input::tap_win_key()
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
    // `.lnk` already embeds args — ShellExecute the shortcut as-is.
    let path_l = path.to_ascii_lowercase();
    if path_l.ends_with(".lnk") {
        return shell_open(path, None);
    }
    let params = if item.launch_args.trim().is_empty() {
        None
    } else {
        Some(item.launch_args.as_str())
    };
    shell_open(path, params)
}

pub fn shell_open_path(path: &str) -> Result<(), String> {
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
            if ret.0 as isize <= 32 {
                return Err(format!(
                    "ShellExecute failed for {file} (code {})",
                    ret.0 as isize
                ));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_tree_matches_nested_wps() {
        let pin = r"C:\Program Files\Kingsoft\WPS Office\ksolaunch.exe";
        let run = r"C:\Program Files\Kingsoft\WPS Office\12.1.0\office6\wps.exe";
        assert!(same_install_tree(pin, run));
    }

    #[test]
    fn install_tree_rejects_office_cross_app() {
        let word = r"C:\Program Files\Microsoft Office\root\Office16\WINWORD.EXE";
        let excel = r"C:\Program Files\Microsoft Office\root\Office16\EXCEL.EXE";
        assert!(!same_install_tree(word, excel));
    }

    #[test]
    fn install_tree_rejects_program_files_only() {
        let a = r"C:\Program Files\Foo\a.exe";
        let b = r"C:\Program Files\Bar\b.exe";
        assert!(!same_install_tree(a, b));
    }
}
