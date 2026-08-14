//! Per-app Jump List destinations for the dock item context menu.
//!
//! Sources (best-effort, Windows only):
//! 1. `IApplicationDocumentLists` Recent / Frequent (needs AppUserModelID)
//! 2. `%APPDATA%\…\CustomDestinations\*.customDestinations-ms` (tasks / pinned projects)
//! 3. JetBrains `recentProjects.xml` when the pin looks like an IDE

use super::shortcut::{file_name_lower, normalize_exe_key, resolve_launch_target};
use super::DockItem;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const MAX_ITEMS: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockJumpItem {
    pub label: String,
    pub path: String,
    /// `recent` | `frequent` | `custom`
    pub kind: String,
}

/// Collect jump-list style destinations for a dock pin / ephemeral slot.
pub fn list_jump_items(item: &DockItem, hwnd: Option<isize>) -> Vec<DockJumpItem> {
    #[cfg(windows)]
    {
        list_jump_items_windows(item, hwnd)
    }
    #[cfg(not(windows))]
    {
        let _ = (item, hwnd);
        Vec::new()
    }
}

#[cfg(windows)]
fn list_jump_items_windows(item: &DockItem, hwnd: Option<isize>) -> Vec<DockJumpItem> {
    let mut out: Vec<DockJumpItem> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let push = |out: &mut Vec<DockJumpItem>, seen: &mut HashSet<String>, mut it: DockJumpItem| {
        if out.len() >= MAX_ITEMS {
            return;
        }
        let path = it.path.trim();
        if path.is_empty() {
            return;
        }
        let key = path.to_ascii_lowercase().replace('/', "\\");
        if !seen.insert(key) {
            return;
        }
        if it.label.trim().is_empty() {
            it.label = label_for_path(path);
        }
        out.push(it);
    };

    let aumids = collect_aumids(item, hwnd);
    for aumid in &aumids {
        for (list_kind, kind) in [("recent", "recent"), ("frequent", "frequent")] {
            for it in query_document_lists(aumid, list_kind) {
                push(&mut out, &mut seen, DockJumpItem {
                    label: it.0,
                    path: it.1,
                    kind: kind.into(),
                });
            }
        }
    }

    let exe_keys = exe_keys_for_item(item);
    if !exe_keys.is_empty() {
        for it in scan_custom_destinations(&exe_keys) {
            push(&mut out, &mut seen, it);
        }
    }

    if out.len() < MAX_ITEMS {
        for it in jetbrains_recent_projects(item) {
            push(&mut out, &mut seen, it);
        }
    }

    out
}

fn label_for_path(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn exe_keys_for_item(item: &DockItem) -> Vec<String> {
    let mut keys = Vec::new();
    let push_key = |keys: &mut Vec<String>, raw: &str| {
        let k = normalize_exe_key(&file_name_lower(raw));
        if !k.is_empty() && !keys.iter().any(|e| e == &k) {
            keys.push(k);
        }
    };
    push_key(&mut keys, &item.match_exe);
    push_key(&mut keys, &item.real_path);
    push_key(&mut keys, &item.launch_path);
    if let Some(t) = resolve_launch_target(&item.launch_path) {
        push_key(&mut keys, &t);
    }
    if let Some(t) = resolve_launch_target(&item.real_path) {
        push_key(&mut keys, &t);
    }
    keys
}

#[cfg(windows)]
fn collect_aumids(item: &DockItem, hwnd: Option<isize>) -> Vec<String> {
    let mut out = Vec::new();
    let push = |out: &mut Vec<String>, s: String| {
        let t = s.trim().to_string();
        if t.is_empty() {
            return;
        }
        if !out.iter().any(|e| e.eq_ignore_ascii_case(&t)) {
            out.push(t);
        }
    };

    if let Some(h) = hwnd.filter(|h| *h != 0) {
        if let Some(id) = aumid_from_hwnd(h) {
            push(&mut out, id);
        }
    }

    for path in [&item.launch_path, &item.real_path] {
        let p = path.trim();
        if p.is_empty() {
            continue;
        }
        if p.to_ascii_lowercase().ends_with(".lnk") {
            if let Some(id) = aumid_from_path(p) {
                push(&mut out, id);
            }
        } else if let Some(id) = aumid_from_path(p) {
            push(&mut out, id);
        }
    }

    for id in known_aumid_hints(item) {
        push(&mut out, id);
    }

    out
}

#[cfg(windows)]
fn known_aumid_hints(item: &DockItem) -> Vec<String> {
    let blob = format!(
        "{} {} {} {}",
        item.match_exe, item.real_path, item.launch_path, item.label
    )
    .to_ascii_lowercase();
    let mut v = Vec::new();
    if blob.contains("code.exe") || blob.contains("\\cursor\\") || blob.contains("cursor.exe") {
        if blob.contains("cursor") {
            // Cursor often inherits VS Code jump-list plumbing; AUMID varies by install.
        } else {
            v.push("Microsoft.VisualStudioCode".into());
        }
    }
    if blob.contains("msedge.exe") {
        v.push("MSEdge".into());
    }
    if blob.contains("chrome.exe") {
        v.push("Chrome".into());
    }
    if blob.contains("firefox.exe") {
        v.push("Firefox".into());
    }
    v
}

#[cfg(windows)]
fn aumid_from_hwnd(hwnd: isize) -> Option<String> {
    use windows::core::GUID;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::PropertiesSystem::{
        IPropertyStore, PROPERTYKEY, SHGetPropertyStoreForWindow,
    };

    const PKEY_APPUSERMODEL_ID: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3),
        pid: 5,
    };

    unsafe {
        let store: IPropertyStore =
            SHGetPropertyStoreForWindow(HWND(hwnd as *mut _)).ok()?;
        read_aumid_prop(&store, &PKEY_APPUSERMODEL_ID)
    }
}

#[cfg(windows)]
fn aumid_from_path(path: &str) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{GUID, PCWSTR};
    use windows::Win32::UI::Shell::PropertiesSystem::{
        IPropertyStore, PROPERTYKEY, SHGetPropertyStoreFromParsingName, GPS_DEFAULT,
    };

    const PKEY_APPUSERMODEL_ID: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3),
        pid: 5,
    };

    let wide: Vec<u16> = std::ffi::OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let store: IPropertyStore =
            SHGetPropertyStoreFromParsingName(PCWSTR(wide.as_ptr()), None, GPS_DEFAULT).ok()?;
        read_aumid_prop(&store, &PKEY_APPUSERMODEL_ID)
    }
}

#[cfg(windows)]
fn read_aumid_prop(
    store: &windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore,
    key: &windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY,
) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Variant::VT_LPWSTR;

    unsafe {
        let pv = store.GetValue(key).ok()?;
        let raw = pv.as_raw();
        let vt = raw.Anonymous.Anonymous.vt;
        if vt != VT_LPWSTR.0 {
            return None;
        }
        let p = raw.Anonymous.Anonymous.Anonymous.pwszVal;
        if p.is_null() {
            return None;
        }
        let s = PCWSTR(p).to_string().ok()?;
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}

#[cfg(windows)]
fn query_document_lists(aumid: &str, which: &str) -> Vec<(String, String)> {
    use windows::core::{Interface, HSTRING};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::Common::IObjectArray;
    use windows::Win32::UI::Shell::{
        IApplicationDocumentLists, IShellItem, IShellLinkW, ApplicationDocumentLists,
        ADLT_FREQUENT, ADLT_RECENT,
    };

    let list_type = if which == "frequent" {
        ADLT_FREQUENT
    } else {
        ADLT_RECENT
    };

    unsafe {
        let owned = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = (|| -> Option<Vec<(String, String)>> {
            let lists: IApplicationDocumentLists =
                CoCreateInstance(&ApplicationDocumentLists, None, CLSCTX_INPROC_SERVER).ok()?;
            lists.SetAppID(&HSTRING::from(aumid)).ok()?;
            let arr: IObjectArray = lists.GetList(list_type, MAX_ITEMS as u32).ok()?;
            let count = arr.GetCount().ok()? as usize;
            let mut out = Vec::new();
            for i in 0..count.min(MAX_ITEMS) {
                let unk = arr.GetAt::<windows::core::IUnknown>(i as u32).ok()?;
                if let Ok(item) = unk.cast::<IShellItem>() {
                    if let Some((label, path)) = shell_item_dest(&item) {
                        out.push((label, path));
                        continue;
                    }
                }
                if let Ok(link) = unk.cast::<IShellLinkW>() {
                    if let Some((label, path)) = shell_link_dest(&link) {
                        out.push((label, path));
                    }
                }
            }
            Some(out)
        })();
        if owned {
            CoUninitialize();
        }
        result.unwrap_or_default()
    }
}

#[cfg(windows)]
fn shell_item_dest(
    item: &windows::Win32::UI::Shell::IShellItem,
) -> Option<(String, String)> {
    use windows::Win32::UI::Shell::SIGDN_FILESYSPATH;
    unsafe {
        let path_bstr = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = path_bstr.to_string().ok()?;
        if path.is_empty() || !path_exists_or_unc(&path) {
            return None;
        }
        let label = label_for_path(&path);
        Some((label, path))
    }
}

#[cfg(windows)]
fn shell_link_dest(
    link: &windows::Win32::UI::Shell::IShellLinkW,
) -> Option<(String, String)> {
    use windows::Win32::UI::Shell::SLGP_UNCPRIORITY;
    unsafe {
        let mut path_buf = [0u16; 1024];
        let _ = link.GetPath(&mut path_buf, std::ptr::null_mut(), SLGP_UNCPRIORITY.0 as u32);
        let plen = path_buf.iter().position(|&c| c == 0).unwrap_or(0);
        let target = String::from_utf16_lossy(&path_buf[..plen]);

        let mut args_buf = [0u16; 2048];
        let args = if link.GetArguments(&mut args_buf).is_ok() {
            let alen = args_buf.iter().position(|&c| c == 0).unwrap_or(0);
            String::from_utf16_lossy(&args_buf[..alen])
        } else {
            String::new()
        };

        let mut desc_buf = [0u16; 512];
        let desc = if link.GetDescription(&mut desc_buf).is_ok() {
            let dlen = desc_buf.iter().position(|&c| c == 0).unwrap_or(0);
            String::from_utf16_lossy(&desc_buf[..dlen])
        } else {
            String::new()
        };

        // Tasks: exe + path argument (IDEA / VS Code / Cursor "Open recent").
        if let Some(p) = first_existing_path_arg(&args) {
            let label = if !desc.trim().is_empty() {
                desc.trim().to_string()
            } else {
                label_for_path(&p)
            };
            return Some((label, p));
        }

        if !target.is_empty() && path_exists_or_unc(&target) {
            let label = if !desc.trim().is_empty() {
                desc.trim().to_string()
            } else {
                label_for_path(&target)
            };
            return Some((label, target));
        }
        None
    }
}

fn path_exists_or_unc(path: &str) -> bool {
    let p = path.trim();
    if p.is_empty() {
        return false;
    }
    // Allow missing network paths to still show; local must exist.
    if p.starts_with("\\\\") {
        return true;
    }
    Path::new(p).exists()
}

fn first_existing_path_arg(args: &str) -> Option<String> {
    let args = args.trim();
    if args.is_empty() {
        return None;
    }
    // Prefer quoted segments, then bare tokens that look like paths.
    let mut candidates: Vec<String> = Vec::new();
    let mut rest = args;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        if let Some(end) = after.find('"') {
            let inner = after[..end].trim();
            if !inner.is_empty() {
                candidates.push(inner.to_string());
            }
            rest = &after[end + 1..];
        } else {
            break;
        }
    }
    for tok in args.split_whitespace() {
        let t = tok.trim_matches('"');
        if t.len() >= 3 && (t.contains('\\') || t.contains('/')) {
            candidates.push(t.to_string());
        }
    }
    for c in candidates {
        let expanded = expand_env_path(&c);
        if path_exists_or_unc(&expanded) {
            return Some(expanded);
        }
    }
    None
}

fn expand_env_path(s: &str) -> String {
    let mut out = s.to_string();
    for (key, val) in [
        ("%USERPROFILE%", std::env::var("USERPROFILE").ok()),
        ("%APPDATA%", std::env::var("APPDATA").ok()),
        ("%LOCALAPPDATA%", std::env::var("LOCALAPPDATA").ok()),
        ("%ProgramFiles%", std::env::var("ProgramFiles").ok()),
        (
            "%ProgramFiles(x86)%",
            std::env::var("ProgramFiles(x86)").ok(),
        ),
    ] {
        if let Some(v) = val {
            out = out.replace(key, &v);
        }
    }
    out
}

#[cfg(windows)]
fn scan_custom_destinations(exe_keys: &[String]) -> Vec<DockJumpItem> {
    let Ok(appdata) = std::env::var("APPDATA") else {
        return Vec::new();
    };
    let dir = PathBuf::from(appdata).join(r"Microsoft\Windows\Recent\CustomDestinations");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let needles: Vec<Vec<u8>> = exe_keys
        .iter()
        .flat_map(|k| {
            let name = if k.ends_with(".exe") {
                k.clone()
            } else {
                format!("{k}.exe")
            };
            vec![
                name.as_bytes().to_vec(),
                name.encode_utf16()
                    .flat_map(|c| c.to_le_bytes())
                    .collect::<Vec<u8>>(),
            ]
        })
        .collect();

    let mut out = Vec::new();
    for ent in rd.flatten() {
        if out.len() >= MAX_ITEMS {
            break;
        }
        let p = ent.path();
        let name = p
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !name.ends_with(".customdestinations-ms") {
            continue;
        }
        let Ok(bytes) = std::fs::read(&p) else {
            continue;
        };
        if !needles.iter().any(|n| !n.is_empty() && find_bytes(&bytes, n)) {
            continue;
        }
        for (label, path) in extract_custom_dest_links(&bytes, exe_keys) {
            if out.len() >= MAX_ITEMS {
                break;
            }
            out.push(DockJumpItem {
                label,
                path,
                kind: "custom".into(),
            });
        }
    }
    out
}

fn find_bytes(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || hay.len() < needle.len() {
        return false;
    }
    hay.windows(needle.len()).any(|w| w == needle)
}

#[cfg(windows)]
fn extract_custom_dest_links(bytes: &[u8], exe_keys: &[String]) -> Vec<(String, String)> {
    use windows::core::Interface;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistStream, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, SHCreateMemStream, ShellLink};

    let mut starts = Vec::new();
    let mut i = 0usize;
    while i + 4 <= bytes.len() {
        if bytes[i] == 0x4C && bytes[i + 1] == 0 && bytes[i + 2] == 0 && bytes[i + 3] == 0 {
            starts.push(i);
            i += 4;
        } else {
            i += 1;
        }
    }

    unsafe {
        let owned = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let mut out = Vec::new();
        for (idx, start) in starts.iter().enumerate() {
            if out.len() >= MAX_ITEMS {
                break;
            }
            let end = starts.get(idx + 1).copied().unwrap_or(bytes.len());
            // Category footer / padding — skip tiny blobs.
            if end.saturating_sub(*start) < 64 {
                continue;
            }
            let slice = &bytes[*start..end.min(bytes.len())];
            // Trim trailing category marker `AB FB BF BA` if present.
            let slice = trim_custom_dest_footer(slice);
            let Some(stream) = SHCreateMemStream(Some(slice)) else {
                continue;
            };
            let Ok(link) = CoCreateInstance::<_, IShellLinkW>(&ShellLink, None, CLSCTX_INPROC_SERVER)
            else {
                continue;
            };
            let Ok(persist) = link.cast::<IPersistStream>() else {
                continue;
            };
            if persist.Load(&stream).is_err() {
                continue;
            }

            let mut path_buf = [0u16; 1024];
            let _ = link.GetPath(
                &mut path_buf,
                std::ptr::null_mut(),
                windows::Win32::UI::Shell::SLGP_UNCPRIORITY.0 as u32,
            );
            let plen = path_buf.iter().position(|&c| c == 0).unwrap_or(0);
            let target = expand_env_path(&String::from_utf16_lossy(&path_buf[..plen]));
            let target_key = normalize_exe_key(&file_name_lower(&target));

            // Only keep entries launched *by* this app (tasks), not random LNKs in the file.
            let is_our_exe = exe_keys.iter().any(|k| k == &target_key);
            if !is_our_exe {
                continue;
            }

            if let Some((label, path)) = shell_link_dest(&link) {
                out.push((label, path));
            }
        }
        if owned {
            CoUninitialize();
        }
        out
    }
}

fn trim_custom_dest_footer(slice: &[u8]) -> &[u8] {
    const MARK: [u8; 4] = [0xAB, 0xFB, 0xBF, 0xBA];
    if let Some(pos) = slice.windows(4).rposition(|w| w == MARK) {
        return &slice[..pos];
    }
    slice
}

/// JetBrains IDEs store recent projects outside the shell Jump List.
fn jetbrains_recent_projects(item: &DockItem) -> Vec<DockJumpItem> {
    let blob = format!(
        "{} {} {} {}",
        item.match_exe, item.real_path, item.launch_path, item.label
    )
    .to_ascii_lowercase();
    let product = if blob.contains("idea") {
        Some("idea")
    } else if blob.contains("webstorm") {
        Some("webstorm")
    } else if blob.contains("pycharm") {
        Some("pycharm")
    } else if blob.contains("clion") {
        Some("clion")
    } else if blob.contains("goland") {
        Some("goland")
    } else if blob.contains("rider") {
        Some("rider")
    } else if blob.contains("phpstorm") {
        Some("phpstorm")
    } else if blob.contains("rubymine") {
        Some("rubymine")
    } else if blob.contains("datagrip") {
        Some("datagrip")
    } else if blob.contains("androidstudio") || blob.contains("studio64") {
        Some("androidstudio")
    } else {
        None
    };
    let Some(product) = product else {
        return Vec::new();
    };

    let Ok(appdata) = std::env::var("APPDATA") else {
        return Vec::new();
    };
    let root = PathBuf::from(&appdata).join("JetBrains");
    let Ok(rd) = std::fs::read_dir(&root) else {
        // Android Studio may live under Google\
        return android_studio_recent(&appdata);
    };

    let mut dirs: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            match product {
                "idea" => n.contains("intellijidea") || n.starts_with("idea"),
                "androidstudio" => n.contains("androidstudio"),
                other => n.contains(other),
            }
        })
        .collect();
    // Newest config dir first (name sorts by year.version roughly).
    dirs.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

    for dir in dirs {
        let xml = dir.join("options").join("recentProjects.xml");
        if !xml.is_file() {
            continue;
        }
        let items = parse_jetbrains_recent_xml(&xml);
        if !items.is_empty() {
            return items;
        }
    }
    if product == "androidstudio" {
        return android_studio_recent(&appdata);
    }
    Vec::new()
}

fn android_studio_recent(appdata: &str) -> Vec<DockJumpItem> {
    let root = PathBuf::from(appdata).join("Google");
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
                .contains("androidstudio")
        })
        .collect();
    dirs.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    for dir in dirs {
        let xml = dir.join("options").join("recentProjects.xml");
        if xml.is_file() {
            let items = parse_jetbrains_recent_xml(&xml);
            if !items.is_empty() {
                return items;
            }
        }
    }
    Vec::new()
}

fn parse_jetbrains_recent_xml(path: &Path) -> Vec<DockJumpItem> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    // Entries look like: <entry key="$USER_HOME$/work/foo"> or key="C:/work/foo"
    for (idx, _) in text.match_indices("<entry key=\"") {
        let rest = &text[idx + "<entry key=\"".len()..];
        let Some(end) = rest.find('"') else {
            continue;
        };
        let raw = rest[..end].trim();
        if raw.is_empty() {
            continue;
        }
        let expanded = expand_jetbrains_path(raw);
        if !path_exists_or_unc(&expanded) {
            continue;
        }
        out.push(DockJumpItem {
            label: label_for_path(&expanded),
            path: expanded,
            kind: "recent".into(),
        });
        if out.len() >= MAX_ITEMS {
            break;
        }
    }
    out
}

fn expand_jetbrains_path(s: &str) -> String {
    let mut out = s.replace('/', "\\");
    if let Ok(home) = std::env::var("USERPROFILE") {
        out = out.replace("$USER_HOME$", &home);
    }
    if let Ok(home) = std::env::var("HOME") {
        out = out.replace("$USER_HOME$", &home);
    }
    expand_env_path(&out)
}

/// Open a jump-list destination with the dock app when helpful (folders / projects).
pub fn open_jump_item(item: &DockItem, target: &str) -> Result<(), String> {
    let target = target.trim();
    if target.is_empty() {
        return Err("empty jump path".into());
    }
    let target_path = Path::new(target);
    let app = if !item.launch_path.is_empty() {
        item.launch_path.as_str()
    } else {
        item.real_path.as_str()
    };

    // Folders / extensionless project roots → open via the pinned app.
    let open_via_app = target_path.is_dir()
        || target_path.extension().is_none()
        || is_project_file(target);

    if open_via_app && !app.is_empty() {
        let exe = resolve_launch_target(app).unwrap_or_else(|| app.to_string());
        let exe_l = exe.to_ascii_lowercase();
        if exe_l.ends_with(".exe") {
            let args = if !item.launch_args.is_empty() && !exe_l.contains("idea") {
                // Keep PWA / special args, append path.
                format!("{} \"{}\"", item.launch_args.trim(), target)
            } else {
                format!("\"{target}\"")
            };
            return super::launch::shell_open_path_with_params(&exe, Some(&args));
        }
        // .lnk: ShellExecute the shortcut with the path as params (often ignored) —
        // fall through to resolving again via real_path if needed.
        if !item.real_path.is_empty() && item.real_path.to_ascii_lowercase().ends_with(".exe") {
            return super::launch::shell_open_path_with_params(
                &item.real_path,
                Some(&format!("\"{target}\"")),
            );
        }
    }

    super::launch::shell_open_path(target)
}

fn is_project_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".sln")
        || lower.ends_with(".code-workspace")
        || lower.ends_with(".csproj")
        || lower.ends_with(".ipr")
}
