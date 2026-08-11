//! System tray icon tracking for Windows 10/11.
//!
//! Primary pipeline (MyDockFinder-aligned):
//! 1. **Explorer hook** (`window_hub_trayhook.dll` via `WH_CALLWNDPROC`) —
//!    intercepts tray `WM_COPYDATA` inside explorer → shared-memory slots
//! 2. **Notify callback clicks** — `SendNotifyMessage` only (no demote / UIA)
//! 3. **Registry** — tooltip / snapshot / pin enrichment only (not existence)
//!
//! Fallback: vendored `systray-util` spy when the hook fails to install.
//! UIA / demote_all are **not** on the click or startup hot path.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrayIconInfo {
    /// Runtime-unique id (`guid` or `hwnd:uid`). HWND changes across reboots.
    pub id: String,
    /// Reboot-stable key for 常显 / menu height prefs (`guid` or `exe:path:uid`).
    #[serde(default)]
    pub pin_key: String,
    /// Human-readable label for UI (tooltip / process / known system name).
    pub tooltip: String,
    pub process: String,
    pub uid: u32,
    pub hwnd: isize,
    pub callback_msg: u32,
    /// NOTIFYICON version (0..=4). Affects click message packing.
    pub version: u32,
    /// PNG as base64 (may be empty if icon copy failed).
    pub icon_png_base64: String,
    /// `taskbar` (visible) or `overflow` (hidden / overflow flyout).
    pub area: String,
    /// Attention / blink (e.g. WeChat new message). Cleared on click.
    #[serde(default)]
    pub flashing: bool,
    /// Always keep on the menubar rail (IME / input language). Auto-pinned.
    #[serde(default)]
    pub resident: bool,
}

/// Rising-edge tray blink for island notification UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrayAttention {
    pub id: String,
    pub tooltip: String,
    pub process: String,
    pub icon_png_base64: String,
    pub hwnd: isize,
    pub uid: u32,
    pub callback_msg: u32,
    pub version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrayPrefs {
    /// Icon ids that stay visible on the bar (outside the chevron).
    pub pinned: Vec<String>,
    /// Per-icon right-click menu height (px). Missing id = auto measure.
    #[serde(default)]
    pub menu_heights: std::collections::HashMap<String, i32>,
    /// Legacy global height (migrated into `menu_heights` on load if present).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu_height_px: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayClick {
    Left,
    Right,
}

impl TrayClick {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "right" | "context" | "contextmenu" => Self::Right,
            _ => Self::Left,
        }
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use base64::Engine;
    use parking_lot::Mutex;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{LazyLock, OnceLock};
    use systray_util::{ImageFormat, Systray, SystrayEvent, SystrayIcon};

    static PREFS: LazyLock<Mutex<TrayPrefs>> =
        LazyLock::new(|| Mutex::new(TrayPrefs::default()));

    static ICONS: LazyLock<Mutex<HashMap<String, TrayIconInfo>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    /// Last OS-level fingerprint per icon (`hash` / `__blank__` / png).
    /// Used so blank flash frames still arm `flashing` even when we retain
    /// the previous PNG for CSS blink display.
    static OS_FINGERPRINT: LazyLock<Mutex<HashMap<String, String>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    /// Recent fingerprint-change timestamps — rapid swaps ≈ blink without blank frames.
    static FP_CHANGES: LazyLock<Mutex<HashMap<String, Vec<std::time::Instant>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    /// User acknowledged island/tray attention — ignore leftover blink frames for a bit.
    /// Key: tray icon id. Value: ack Instant.
    static ATTENTION_ACK: LazyLock<Mutex<HashMap<String, std::time::Instant>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    /// Suppress leftover blink frames right after click (not long — new msgs must re-arm).
    const ATTENTION_ACK_MS: u128 = 700;

    type EmitFn = Box<dyn Fn(Vec<TrayIconInfo>) + Send + Sync>;
    type AttentionFn = Box<dyn Fn(TrayAttention) + Send + Sync>;
    type PrefsFn = Box<dyn Fn(TrayPrefs) + Send + Sync>;
    static EMIT: OnceLock<EmitFn> = OnceLock::new();
    static ATTENTION: OnceLock<AttentionFn> = OnceLock::new();
    static PREFS_EMIT: OnceLock<PrefsFn> = OnceLock::new();

    /// Rate-limit optional cold-start TaskbarCreated (hook path only).
    static LAST_TASKBAR_CREATED: Mutex<Option<std::time::Instant>> = Mutex::new(None);

    /// icon id → NotifyIconSettings subkey (enrichment / pin only).
    static REG_KEY_BY_ID: LazyLock<Mutex<HashMap<String, String>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    /// True when explorer hook is the list source (spy disabled).
    static HOOK_PRIMARY: AtomicBool = AtomicBool::new(false);

    pub fn get_prefs() -> TrayPrefs {
        PREFS.lock().clone()
    }

    pub fn set_prefs(mut prefs: TrayPrefs) {
        let icons: Vec<TrayIconInfo> = ICONS.lock().values().cloned().collect();
        normalize_prefs_keys(&mut prefs, &icons);
        *PREFS.lock() = prefs;
    }

    /// Persist current in-memory tray prefs (after pin_key migration).
    fn persist_prefs_disk() {
        let prefs = get_prefs();
        if let Ok(v) = serde_json::to_value(&prefs) {
            let _ = crate::db::with_conn(|c| crate::db::tray_set(c, &v));
        }
    }

    fn looks_like_guid_id(id: &str) -> bool {
        let g = id
            .trim()
            .trim_start_matches('{')
            .trim_end_matches('}')
            .to_ascii_lowercase();
        if g.len() != 36 {
            return false;
        }
        g.as_bytes().iter().enumerate().all(|(i, &b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
    }

    /// Legacy runtime id `hwnd:uid` (digits only) — not reboot-stable.
    fn is_legacy_hwnd_uid(id: &str) -> bool {
        let Some((h, u)) = id.split_once(':') else {
            return false;
        };
        if id.starts_with("exe:") || id.starts_with("proc:") || id.starts_with("hwnd:") {
            return false;
        }
        h.parse::<isize>().is_ok() && u.parse::<u32>().is_ok()
    }

    /// Reboot-stable pin key: GUID → full exe+uid → process stem+uid → runtime id.
    fn compute_pin_key(guid_id: Option<&str>, hwnd: isize, uid: u32, process_stem: &str) -> String {
        if let Some(g) = guid_id {
            let g = g
                .trim()
                .trim_start_matches('{')
                .trim_end_matches('}')
                .to_ascii_lowercase();
            if looks_like_guid_id(&g) && g != "00000000-0000-0000-0000-000000000000" {
                return g;
            }
        }
        let path = process_image_path(hwnd);
        if !path.is_empty() {
            return format!("exe:{path}:{uid}");
        }
        let stem = process_stem.trim().to_ascii_lowercase();
        if !stem.is_empty() {
            return format!("proc:{stem}:{uid}");
        }
        if hwnd != 0 {
            return format!("hwnd:{hwnd}:{uid}");
        }
        format!("uid:{uid}")
    }

    fn pin_key_of(info: &TrayIconInfo) -> String {
        if !info.pin_key.is_empty() {
            info.pin_key.clone()
        } else if looks_like_guid_id(&info.id) {
            info.id.clone()
        } else {
            compute_pin_key(None, info.hwnd, info.uid, &info.process)
        }
    }

    /// Rewrite pinned / menu_heights onto reboot-stable `pin_key`s. Returns true if changed.
    fn normalize_prefs_keys(prefs: &mut TrayPrefs, icons: &[TrayIconInfo]) -> bool {
        let mut remap: HashMap<String, String> = HashMap::new();
        for icon in icons {
            let pk = pin_key_of(icon);
            remap.insert(icon.id.clone(), pk.clone());
            if !icon.pin_key.is_empty() {
                remap.insert(icon.pin_key.clone(), pk.clone());
            }
            if icon.hwnd != 0 {
                remap.insert(format!("{}:{}", icon.hwnd, icon.uid), pk.clone());
                remap.insert(format!("hwnd:{}:{}", icon.hwnd, icon.uid), pk.clone());
            }
            let path = process_image_path(icon.hwnd);
            if !path.is_empty() {
                remap.insert(format!("exe:{path}:{}", icon.uid), pk.clone());
            }
            let stem = icon.process.trim().to_ascii_lowercase();
            if !stem.is_empty() {
                remap.insert(format!("proc:{stem}:{}", icon.uid), pk.clone());
            }
        }

        let resolve = |raw: &str| -> String {
            if let Some(pk) = remap.get(raw) {
                return pk.clone();
            }
            if is_legacy_hwnd_uid(raw) {
                let uid = raw
                    .split_once(':')
                    .and_then(|(_, u)| u.parse::<u32>().ok())
                    .unwrap_or(0);
                let hits: Vec<&TrayIconInfo> = icons
                    .iter()
                    .filter(|i| i.uid == uid && (i.hwnd != 0 || looks_like_guid_id(&i.id)))
                    .collect();
                if hits.len() == 1 {
                    return pin_key_of(hits[0]);
                }
            }
            raw.to_string()
        };

        let mut changed = false;
        let mut new_pinned = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for p in &prefs.pinned {
            let n = resolve(p);
            if n != *p {
                changed = true;
            }
            if seen.insert(n.clone()) {
                new_pinned.push(n);
            } else {
                changed = true;
            }
        }
        prefs.pinned = new_pinned;

        let old_heights = std::mem::take(&mut prefs.menu_heights);
        let mut new_heights = std::collections::HashMap::new();
        for (k, h) in old_heights {
            let nk = resolve(&k);
            if nk != k {
                changed = true;
            }
            new_heights.insert(nk, h);
        }
        prefs.menu_heights = new_heights;
        changed
    }

    fn rewrite_prefs_against_live_icons() -> bool {
        let icons: Vec<TrayIconInfo> = ICONS.lock().values().cloned().collect();
        let mut prefs = PREFS.lock();
        let mut changed = normalize_prefs_keys(&mut prefs, &icons);
        // IME / input-language indicators must stay in 常显.
        for icon in &icons {
            if !icon.resident {
                continue;
            }
            let pk = pin_key_of(icon);
            if pk.is_empty() {
                continue;
            }
            if !prefs.pinned.iter().any(|p| p == &pk || p == &icon.id) {
                prefs.pinned.push(pk);
                changed = true;
            }
        }
        changed
    }

    /// Resolve height: per-icon custom → WeChat/QQ default custom → measured cache → 160.
    fn effective_menu_height(icon_id: &str, pin_key: &str, process: &str, tip: &str) -> i32 {
        let prefs = get_prefs();
        if let Some(h) = prefs
            .menu_heights
            .get(pin_key)
            .or_else(|| prefs.menu_heights.get(icon_id))
            .copied()
        {
            if h > 0 {
                return h.clamp(48, 640);
            }
        }
        if is_tencent_im(process, tip) {
            return DEFAULT_TENCENT_MENU_HEIGHT;
        }
        cached_menu_height(icon_id)
    }

    pub fn list_icons() -> Vec<TrayIconInfo> {
        let _ = sweep_icons();
        let mut v: Vec<_> = ICONS.lock().values().cloned().collect();
        v.sort_by(|a, b| {
            a.tooltip
                .to_lowercase()
                .cmp(&b.tooltip.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        v
    }

    fn clean_text(s: &str) -> String {
        s.replace('\r', " ")
            .replace('\n', " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn looks_like_raw_id(s: &str) -> bool {
        let t = s.trim();
        if t.is_empty() {
            return true;
        }
        // hwnd:uid
        if let Some((a, b)) = t.split_once(':') {
            if a.chars().all(|c| c.is_ascii_digit()) && b.chars().all(|c| c.is_ascii_digit()) {
                return true;
            }
        }
        // GUID
        let hex = t.chars().filter(|c| c.is_ascii_hexdigit()).count();
        t.contains('-') && hex >= 32 && t.len() >= 36
    }

    fn known_system_name(id: &str) -> Option<&'static str> {
        let g = id.trim().to_ascii_lowercase();
        match g.as_str() {
            "7820ae72-23e3-4229-82c1-e41cb67d5b9c" => Some("时钟"),
            "7820ae73-23e3-4229-82c1-e41cb67d5b9c" => Some("扬声器"),
            "7820ae74-23e3-4229-82c1-e41cb67d5b9c" => Some("网络"),
            "7820ae75-23e3-4229-82c1-e41cb67d5b9c" => Some("电源"),
            "7820ae76-23e3-4229-82c1-e41cb67d5b9c" => Some("操作中心"),
            "7820ae78-23e3-4229-82c1-e41cb67d5b9c" => Some("安全删除硬件"),
            "6da68f06-00f1-4e6e-8158-7f1ffd4f9db9" => Some("蓝牙"),
            // Shell Input Indicator / language + IME branding
            "a59b00b9-f6cd-4fed-a1dc-0f4064a12831" => Some("输入法"),
            // GUID_LBI_INPUTMODE — IME on/off / mode switch glyph
            "2c77a81e-41cc-4178-a3a7-5f8a987568e6" => Some("输入法切换"),
            _ => None,
        }
    }

    /// Input language abbreviation + IME mode/branding — always 常显 on the rail.
    fn is_language_ime_icon(id: &str, process: &str, tip: &str) -> bool {
        let g = id
            .trim()
            .trim_start_matches('{')
            .trim_end_matches('}')
            .to_ascii_lowercase();
        if matches!(
            g.as_str(),
            "a59b00b9-f6cd-4fed-a1dc-0f4064a12831"
                | "2c77a81e-41cc-4178-a3a7-5f8a987568e6"
        ) {
            return true;
        }
        if known_system_name(id).is_some_and(|n| n.contains("输入法")) {
            return true;
        }

        let proc = process.trim().to_ascii_lowercase();
        if matches!(
            proc.as_str(),
            "textinputhost"
                | "ctfmon"
                | "tabtip"
                | "inputapp"
                | "msctfmonitor"
                | "chsime"
                | "chtime"
        ) || proc.contains("sogou")
            || proc.contains("baiduinput")
            || proc.contains("qqpinyin")
            || proc.contains("rime")
            || proc.contains("weasel")
            || proc.contains("inputmethod")
        {
            return true;
        }

        let tip_raw = tip.trim();
        let tip_l = tip_raw.to_ascii_lowercase();
        if tip_l.contains("输入法")
            || tip_l.contains("语言")
            || tip_l.contains("ime")
            || tip_l.contains("language")
            || tip_l.contains("微软拼音")
            || tip_l.contains("搜狗")
            || tip_l.contains("微信输入法")
            || tip_l.contains("chinese")
            || tip_l.contains("中文")
        {
            return true;
        }

        // Shell language abbreviation tile: "中" / "英" / "EN" / "CHS" …
        let chars: Vec<char> = tip_raw.chars().collect();
        if chars.len() == 1 {
            let c = chars[0];
            if ('\u{4e00}'..='\u{9fff}').contains(&c) {
                return true;
            }
        }
        matches!(
            tip_l.as_str(),
            "en" | "eng" | "chs" | "cht" | "jp" | "jpn" | "kr" | "kor" | "中" | "英" | "日" | "韩"
        )
    }

    fn window_title(hwnd: isize) -> String {
        if hwnd == 0 {
            return String::new();
        }
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW};
        unsafe {
            let h = HWND(hwnd as *mut _);
            let len = GetWindowTextLengthW(h);
            if len <= 0 {
                return String::new();
            }
            let mut buf = vec![0u16; (len + 1) as usize];
            let n = GetWindowTextW(h, &mut buf);
            if n <= 0 {
                return String::new();
            }
            clean_text(&String::from_utf16_lossy(&buf[..n as usize]))
        }
    }

    fn process_label(hwnd: isize) -> String {
        if hwnd == 0 {
            return String::new();
        }
        use windows::Win32::Foundation::{CloseHandle, HWND};
        use windows::Win32::System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        };
        use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

        unsafe {
            let mut pid = 0u32;
            GetWindowThreadProcessId(HWND(hwnd as *mut _), Some(&mut pid));
            if pid == 0 {
                return String::new();
            }
            let Ok(proc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return String::new();
            };
            let mut buf = [0u16; 520];
            let mut size = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(
                proc,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(buf.as_mut_ptr()),
                &mut size,
            );
            let _ = CloseHandle(proc);
            if ok.is_err() || size == 0 {
                return String::new();
            }
            let path = String::from_utf16_lossy(&buf[..size as usize]);
            let stem = Path::new(&path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            stem
        }
    }

    fn resolve_label(id: &str, raw_tip: &str, process: &str, hwnd: isize) -> String {
        let tip = clean_text(raw_tip);
        if !tip.is_empty() && !looks_like_raw_id(&tip) {
            return tip;
        }
        if let Some(name) = known_system_name(id) {
            return name.to_string();
        }
        if !process.is_empty() {
            return process.to_string();
        }
        let title = window_title(hwnd);
        if !title.is_empty() && !looks_like_raw_id(&title) {
            return title;
        }
        if !tip.is_empty() {
            return tip;
        }
        "未知应用".to_string()
    }

    fn os_fingerprint(icon: &SystrayIcon, blank_frame: bool) -> String {
        if blank_frame {
            return "__blank__".to_string();
        }
        if let Some(hash) = &icon.icon_image_hash {
            return hash.clone();
        }
        icon.to_image_format(ImageFormat::Png)
            .ok()
            .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
            .unwrap_or_default()
    }

    fn to_info(icon: &SystrayIcon, is_update: bool) -> TrayIconInfo {
        let id = icon.stable_id.to_string();
        let hwnd = icon.window_handle.unwrap_or(0);
        let uid = icon.uid.unwrap_or(0);
        let callback_msg = icon.callback_message.unwrap_or(0);
        let version = icon.version.unwrap_or(0);
        let process = process_label(hwnd);
        let mut tooltip = resolve_label(&id, &icon.tooltip, &process, hwnd);

        let prev = ICONS.lock().get(&id).cloned();

        // Keep a previously resolved friendly name if this update has no tip.
        if looks_like_raw_id(&tooltip) || tooltip == "未知应用" {
            if let Some(ref prev) = prev {
                if !looks_like_raw_id(&prev.tooltip) && prev.tooltip != "未知应用" {
                    tooltip = prev.tooltip.clone();
                }
            }
        }

        let os_png = icon
            .to_image_format(ImageFormat::Png)
            .ok()
            .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
            .unwrap_or_default();
        let blank_frame = os_png.is_empty();
        let fingerprint = os_fingerprint(icon, blank_frame);

        let area = if icon.is_visible {
            "taskbar".to_string()
        } else {
            "overflow".to_string()
        };

        // Blank flash frames clear the HICON; keep the last glyph so CSS can blink.
        let mut icon_png_base64 = os_png;
        if blank_frame {
            if let Some(ref prev) = prev {
                if !prev.icon_png_base64.is_empty() {
                    icon_png_base64 = prev.icon_png_base64.clone();
                }
            }
        }

        let mut flashing = prev.as_ref().map(|p| p.flashing).unwrap_or(false);
        if is_update {
            let prev_fp = OS_FINGERPRINT.lock().get(&id).cloned();
            let fp_changed = prev_fp.as_ref().is_some_and(|p| p != &fingerprint);
            let vis_changed = prev.as_ref().map(|p| p.area != area).unwrap_or(false);
            // Classic tray blink (WeChat etc.): blank HICON frames and/or
            // NIS_HIDDEN toggles. Also arm on rapid glyph oscillation.
            let from_or_to_blank =
                blank_frame || prev_fp.as_deref() == Some("__blank__");
            let rapid_swap = if fp_changed {
                let now = std::time::Instant::now();
                let mut map = FP_CHANGES.lock();
                let times = map.entry(id.clone()).or_default();
                times.push(now);
                times.retain(|t| now.duration_since(*t).as_millis() < 2500);
                times.len() >= 2
            } else {
                false
            };
            if blank_frame || vis_changed || from_or_to_blank || rapid_swap {
                flashing = true;
            }
        }
        if flashing && attention_suppressed(&id) {
            flashing = false;
        }

        OS_FINGERPRINT.lock().insert(id.clone(), fingerprint);

        let pin_key = {
            let guid = if looks_like_guid_id(&id) {
                Some(id.as_str())
            } else {
                None
            };
            compute_pin_key(guid, hwnd, uid, &process)
        };
        let resident = is_language_ime_icon(&id, &process, &tooltip);

        TrayIconInfo {
            id: id.clone(),
            pin_key,
            tooltip,
            process,
            uid,
            hwnd,
            callback_msg,
            version,
            icon_png_base64,
            area,
            flashing,
            resident,
        }
    }

    fn hwnd_alive(hwnd: isize) -> bool {
        // Registry stubs use hwnd=0 until spy delivers a real target.
        if hwnd == 0 {
            return true;
        }
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::IsWindow;
        unsafe { IsWindow(HWND(hwnd as *mut _)).as_bool() }
    }

    fn is_clickable(icon: &TrayIconInfo) -> bool {
        icon.hwnd != 0 && icon.callback_msg != 0 && hwnd_alive(icon.hwnd)
    }

    /// Drop dead owner windows and replace stubs with clickable spy icons.
    /// Never merge two live tray icons (WeChat multi-account shares process+tooltip).
    fn sweep_icons() -> bool {
        let mut icons = ICONS.lock();
        let before = icons.len();

        let dead: Vec<String> = icons
            .iter()
            .filter(|(_, info)| info.hwnd != 0 && !hwnd_alive(info.hwnd))
            .map(|(id, _)| id.clone())
            .collect();
        for id in &dead {
            icons.remove(id);
            REG_KEY_BY_ID.lock().remove(id);
        }

        // Drop non-clickable stubs when a clickable twin exists (same uid+process, or same guid id).
        let clickable: Vec<TrayIconInfo> = icons
            .values()
            .filter(|i| is_clickable(i))
            .cloned()
            .collect();
        let mut drop_ids: Vec<String> = Vec::new();
        for (id, info) in icons.iter() {
            if is_clickable(info) {
                continue;
            }
            let proc = info.process.trim().to_ascii_lowercase();
            let covered = clickable.iter().any(|c| {
                if c.id == *id {
                    return false;
                }
                let cproc = c.process.trim().to_ascii_lowercase();
                if !proc.is_empty() && proc == cproc && info.uid != 0 && info.uid == c.uid {
                    return true;
                }
                if !info.id.contains(':') && c.id == info.id {
                    return true;
                }
                false
            });
            if covered {
                drop_ids.push(id.clone());
            }
        }
        for (id, info) in icons.iter() {
            if is_clickable(info) || drop_ids.contains(id) {
                continue;
            }
            let proc = info.process.trim().to_ascii_lowercase();
            let tip = info.tooltip.trim().to_ascii_lowercase();
            if proc.is_empty() {
                continue;
            }
            if info.uid != 0
                && clickable.iter().any(|c| {
                    c.uid == info.uid && c.process.trim().eq_ignore_ascii_case(&proc)
                })
            {
                drop_ids.push(id.clone());
            }
            let _ = tip;
        }

        // Task Manager floods the tray with ghost copies while open — keep at most
        // one "stats" icon (tooltip has %) and one generic Taskmgr entry.
        let taskmgr_ids: Vec<(String, TrayIconInfo)> = icons
            .iter()
            .filter(|(_, info)| info.process.trim().eq_ignore_ascii_case("taskmgr"))
            .map(|(id, info)| (id.clone(), info.clone()))
            .collect();
        if taskmgr_ids.len() > 1 {
            let mut stats: Option<(String, i32)> = None;
            let mut generic: Option<(String, i32)> = None;
            for (id, info) in &taskmgr_ids {
                let tip = info.tooltip.clone();
                let score = (if is_clickable(info) { 10 } else { 0 })
                    + (if !info.icon_png_base64.is_empty() { 1 } else { 0 });
                if tip.contains('%') || tip.to_ascii_lowercase().contains("cpu") {
                    if stats.as_ref().map(|(_, s)| score > *s).unwrap_or(true) {
                        stats = Some((id.clone(), score));
                    }
                } else if generic.as_ref().map(|(_, s)| score > *s).unwrap_or(true) {
                    generic = Some((id.clone(), score));
                }
            }
            let keep: std::collections::HashSet<String> = [stats, generic]
                .into_iter()
                .flatten()
                .map(|(id, _)| id)
                .collect();
            for (id, _) in &taskmgr_ids {
                if !keep.contains(id) {
                    drop_ids.push(id.clone());
                }
            }
        }

        for id in &drop_ids {
            icons.remove(id);
            REG_KEY_BY_ID.lock().remove(id);
        }

        let changed = icons.len() != before || !dead.is_empty() || !drop_ids.is_empty();
        if changed {
            let mut fps = OS_FINGERPRINT.lock();
            let mut changes = FP_CHANGES.lock();
            for id in dead.iter().chain(drop_ids.iter()) {
                fps.remove(id);
                changes.remove(id);
            }
        }
        changed
    }

    fn upsert_icon(info: TrayIconInfo) {
        let proc = info.process.trim().to_ascii_lowercase();
        let id = info.id.clone();
        let uid = info.uid;
        let clickable_new = info.hwnd != 0 && info.callback_msg != 0;

        // Taskmgr: ignore extra ghosts once we already track one clickable / stub.
        if proc == "taskmgr" {
            let icons = ICONS.lock();
            let tip = info.tooltip.clone();
            let is_stats = tip.contains('%') || tip.to_ascii_lowercase().contains("cpu");
            let already = icons.values().any(|p| {
                p.process.trim().eq_ignore_ascii_case("taskmgr")
                    && p.id != id
                    && {
                        let pt = p.tooltip.clone();
                        let p_stats = pt.contains('%') || pt.to_ascii_lowercase().contains("cpu");
                        is_stats == p_stats
                    }
            });
            if already {
                return;
            }
        }

        let mut armed_attention: Option<TrayAttention> = None;
        {
            let mut icons = ICONS.lock();
            // When spy delivers a real icon, drop matching registry stubs only — never
            // another live icon (dual WeChat / multi-instance).
            if clickable_new {
                let stale: Vec<String> = icons
                    .iter()
                    .filter(|(other_id, prev)| {
                        if *other_id == &id || is_clickable(prev) {
                            return false;
                        }
                        let same_proc =
                            !proc.is_empty() && prev.process.trim().eq_ignore_ascii_case(&proc);
                        if !same_proc {
                            return false;
                        }
                        // Same uid → definite twin stub; uid 0 stub → drop when process matches.
                        prev.uid == uid || prev.uid == 0
                    })
                    .map(|(other_id, _)| other_id.clone())
                    .collect();
                for other_id in &stale {
                    icons.remove(other_id);
                    REG_KEY_BY_ID.lock().remove(other_id);
                }
                if !stale.is_empty() {
                    let mut fps = OS_FINGERPRINT.lock();
                    let mut changes = FP_CHANGES.lock();
                    for other_id in &stale {
                        fps.remove(other_id);
                        changes.remove(other_id);
                    }
                }
            }
            let was_flashing = icons.get(&id).map(|p| p.flashing).unwrap_or(false);
            if info.flashing && !was_flashing && !attention_suppressed(&id) {
                armed_attention = Some(TrayAttention {
                    id: info.id.clone(),
                    tooltip: info.tooltip.clone(),
                    process: info.process.clone(),
                    icon_png_base64: info.icon_png_base64.clone(),
                    hwnd: info.hwnd,
                    uid: info.uid,
                    callback_msg: info.callback_msg,
                    version: info.version,
                });
            }
            icons.insert(id, info);
        }
        let _ = sweep_icons();
        publish();
        if let Some(att) = armed_attention {
            if let Some(emit) = ATTENTION.get() {
                emit(att);
            }
        }
    }

    fn attention_suppressed(id: &str) -> bool {
        let mut ack = ATTENTION_ACK.lock();
        match ack.get(id) {
            Some(t) if t.elapsed().as_millis() < ATTENTION_ACK_MS => true,
            Some(_) => {
                ack.remove(id);
                false
            }
            None => false,
        }
    }

    fn acknowledge_attention(ids: impl IntoIterator<Item = String>) {
        let now = std::time::Instant::now();
        let mut ack = ATTENTION_ACK.lock();
        let mut fps = FP_CHANGES.lock();
        for id in ids {
            ack.insert(id.clone(), now);
            fps.remove(&id);
        }
    }

    /// Point-open / tray click: flashing → 0 and suppress leftover blink rising-edges.
    fn clear_flashing(id: Option<&str>, hwnd: isize, uid: u32) {
        let mut icons = ICONS.lock();
        let mut cleared: Vec<String> = Vec::new();
        for (icon_id, icon) in icons.iter_mut() {
            let by_id = id.is_some_and(|i| i == icon_id.as_str());
            let by_hwnd = hwnd != 0 && icon.hwnd == hwnd && icon.uid == uid;
            if !by_id && !by_hwnd {
                continue;
            }
            if icon.flashing {
                icon.flashing = false;
            }
            cleared.push(icon_id.clone());
        }
        drop(icons);
        if cleared.is_empty() {
            return;
        }
        acknowledge_attention(cleared);
        publish();
    }

    fn publish() {
        if rewrite_prefs_against_live_icons() {
            persist_prefs_disk();
            if let Some(emit) = PREFS_EMIT.get() {
                emit(get_prefs());
            }
        }
        let list = list_icons();
        if let Some(emit) = EMIT.get() {
            emit(list);
        }
    }

    fn process_image_path(hwnd: isize) -> String {
        if hwnd == 0 {
            return String::new();
        }
        use windows::Win32::Foundation::{CloseHandle, HWND};
        use windows::Win32::System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        };
        use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
        unsafe {
            let mut pid = 0u32;
            GetWindowThreadProcessId(HWND(hwnd as *mut _), Some(&mut pid));
            if pid == 0 {
                return String::new();
            }
            let Ok(proc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return String::new();
            };
            let mut buf = [0u16; 520];
            let mut size = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(
                proc,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(buf.as_mut_ptr()),
                &mut size,
            );
            let _ = CloseHandle(proc);
            if ok.is_err() || size == 0 {
                return String::new();
            }
            String::from_utf16_lossy(&buf[..size as usize]).to_ascii_lowercase()
        }
    }

    fn snapshot_to_png_b64(bytes: &[u8]) -> String {
        if bytes.is_empty() {
            return String::new();
        }
        // IconSnapShot is often PNG already; otherwise try ICO/BMP via `image`.
        if bytes.len() >= 8 && &bytes[..8] == b"\x89PNG\r\n\x1a\n" {
            return base64::engine::general_purpose::STANDARD.encode(bytes);
        }
        match image::load_from_memory(bytes) {
            Ok(img) => {
                let mut out = Vec::new();
                let rgba = img.to_rgba8();
                let enc = image::codecs::png::PngEncoder::new(&mut out);
                use image::ImageEncoder;
                if enc
                    .write_image(
                        rgba.as_raw(),
                        rgba.width(),
                        rgba.height(),
                        image::ExtendedColorType::Rgba8,
                    )
                    .is_ok()
                {
                    base64::engine::general_purpose::STANDARD.encode(out)
                } else {
                    String::new()
                }
            }
            Err(_) => {
                // Last resort: treat as raw PNG/ICO bytes the frontend may still decode.
                base64::engine::general_purpose::STANDARD.encode(bytes)
            }
        }
    }

    fn match_reg_to_spy(
        icons: &HashMap<String, TrayIconInfo>,
        reg: &crate::win32::tray_registry::RegTrayIcon,
    ) -> Option<String> {
        if let Some(ref guid) = reg.icon_guid {
            let g = crate::win32::tray_registry::guid_key(guid);
            if icons.contains_key(&g) {
                return Some(g);
            }
        }
        if let Some(uid) = reg.icon_uid {
            let exe = reg.executable_path.to_string_lossy().to_ascii_lowercase();
            let stem = reg.process.trim().to_ascii_lowercase();
            // Prefer clickable matches; collect all uid hits then pick best.
            let mut candidates: Vec<(i32, String)> = Vec::new();
            for (id, info) in icons {
                if info.uid != uid {
                    continue;
                }
                let path = process_image_path(info.hwnd);
                let stem_ok =
                    !stem.is_empty() && info.process.trim().eq_ignore_ascii_case(&stem);
                let path_ok = !path.is_empty() && !exe.is_empty() && path == exe;
                // Stem-only match: registry ExecutablePath often uses volume GUIDs.
                if !(path_ok || stem_ok) {
                    continue;
                }
                let score = if is_clickable(info) { 2 } else { 0 }
                    + if path_ok { 1 } else { 0 };
                candidates.push((score, id.clone()));
            }
            candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            if let Some((_, id)) = candidates.into_iter().next() {
                return Some(id);
            }
        }
        // Last resort: unique clickable with same process stem.
        let stem = reg.process.trim().to_ascii_lowercase();
        if !stem.is_empty() {
            let hits: Vec<String> = icons
                .iter()
                .filter(|(_, info)| {
                    is_clickable(info) && info.process.trim().eq_ignore_ascii_case(&stem)
                })
                .map(|(id, _)| id.clone())
                .collect();
            if hits.len() == 1 {
                return Some(hits[0].clone());
            }
        }
        None
    }

    fn broadcast_taskbar_created() {
        {
            let mut last = LAST_TASKBAR_CREATED.lock();
            if let Some(t) = *last {
                if t.elapsed() < std::time::Duration::from_secs(20) {
                    return;
                }
            }
            *last = Some(std::time::Instant::now());
        }
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{
            RegisterWindowMessageW, SendNotifyMessageW, HWND_BROADCAST,
        };
        unsafe {
            let msg = RegisterWindowMessageW(w!("TaskbarCreated"));
            if msg != 0 {
                let _ = SendNotifyMessageW(HWND_BROADCAST, msg, None, None);
            }
        }
    }

    fn guid_bytes_to_id(bytes: &[u8; 16]) -> String {
        let d1 = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let d2 = u16::from_le_bytes([bytes[4], bytes[5]]);
        let d3 = u16::from_le_bytes([bytes[6], bytes[7]]);
        format!(
            "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            d1,
            d2,
            d3,
            bytes[8],
            bytes[9],
            bytes[10],
            bytes[11],
            bytes[12],
            bytes[13],
            bytes[14],
            bytes[15]
        )
    }

    fn tooltip_from_u16(buf: &[u16]) -> String {
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        clean_text(&String::from_utf16_lossy(&buf[..len]))
    }

    fn rgba_to_png_b64(rgba: &[u8], width: u32, height: u32) -> String {
        if width == 0 || height == 0 {
            return String::new();
        }
        let expect = (width as usize).saturating_mul(height as usize).saturating_mul(4);
        if rgba.len() < expect {
            return String::new();
        }
        let mut out = Vec::new();
        let enc = image::codecs::png::PngEncoder::new(&mut out);
        use image::ImageEncoder;
        if enc
            .write_image(
                &rgba[..expect],
                width,
                height,
                image::ExtendedColorType::Rgba8,
            )
            .is_err()
        {
            return String::new();
        }
        base64::engine::general_purpose::STANDARD.encode(out)
    }

    fn stable_id_from_slot(slot: &crate::win32::tray_hook_ipc::TrayHookSlot) -> String {
        use crate::win32::tray_hook_ipc::NIF_GUID;
        if slot.has_guid != 0 && slot.flags & NIF_GUID != 0 {
            let id = guid_bytes_to_id(&slot.guid);
            if id != "00000000-0000-0000-0000-000000000000" {
                return id;
            }
        }
        format!("{}:{}", slot.hwnd, slot.uid)
    }

    fn slot_to_info(
        slot: &crate::win32::tray_hook_ipc::TrayHookSlot,
        is_update: bool,
    ) -> TrayIconInfo {
        use crate::win32::tray_hook_ipc::{
            NIF_ICON, NIF_MESSAGE, NIF_STATE, NIF_TIP, NIS_HIDDEN,
        };

        let id = stable_id_from_slot(slot);
        let prev = ICONS.lock().get(&id).cloned();

        // hwnd/uid identify the icon and are present even without NIF_* on MODIFY.
        let hwnd = if slot.hwnd != 0 {
            slot.hwnd as isize
        } else {
            prev.as_ref().map(|p| p.hwnd).unwrap_or(0)
        };
        let uid = if slot.uid != 0 || prev.is_none() {
            slot.uid
        } else {
            prev.as_ref().map(|p| p.uid).unwrap_or(0)
        };

        // CRITICAL: MODIFY often omits NIF_MESSAGE and leaves callback=0. Overwriting
        // would make clicks fall through to uid-twin matching (Clash uid=2 → AI助手).
        let callback_msg = if !is_update || slot.flags & NIF_MESSAGE != 0 || slot.callback_msg != 0
        {
            if slot.callback_msg != 0 || !is_update {
                slot.callback_msg
            } else {
                prev.as_ref().map(|p| p.callback_msg).unwrap_or(0)
            }
        } else {
            prev.as_ref().map(|p| p.callback_msg).unwrap_or(0)
        };

        let version = if slot.version > 0 {
            slot.version
        } else {
            prev.as_ref().map(|p| p.version).unwrap_or(0)
        };

        let process = {
            let p = process_label(hwnd);
            if p.is_empty() {
                prev.as_ref().map(|x| x.process.clone()).unwrap_or_default()
            } else {
                p
            }
        };

        let raw_tip = if slot.flags & NIF_TIP != 0 {
            tooltip_from_u16(&slot.tooltip)
        } else {
            String::new()
        };
        let mut tooltip = resolve_label(&id, &raw_tip, &process, hwnd);
        if raw_tip.is_empty() || looks_like_raw_id(&tooltip) || tooltip == "未知应用" {
            if let Some(ref prev) = prev {
                if !looks_like_raw_id(&prev.tooltip) && prev.tooltip != "未知应用" {
                    tooltip = prev.tooltip.clone();
                }
            }
        }

        let blank_frame = slot.flags & NIF_ICON != 0 && slot.icon_w == 0;
        let os_png = if slot.flags & NIF_ICON != 0 && slot.icon_w > 0 && slot.icon_h > 0 {
            rgba_to_png_b64(&slot.icon_rgba, slot.icon_w, slot.icon_h)
        } else {
            String::new()
        };
        let fingerprint = if blank_frame {
            "__blank__".to_string()
        } else if !os_png.is_empty() {
            os_png.clone()
        } else {
            prev.as_ref()
                .map(|p| p.icon_png_base64.clone())
                .unwrap_or_default()
        };

        let hidden = slot.flags & NIF_STATE != 0 && slot.state & NIS_HIDDEN != 0;
        let area = if slot.flags & NIF_STATE != 0 {
            if hidden {
                "overflow".to_string()
            } else {
                "taskbar".to_string()
            }
        } else {
            prev.as_ref()
                .map(|p| p.area.clone())
                .unwrap_or_else(|| "overflow".to_string())
        };

        // Only replace glyph when this update carries NIF_ICON (or first ADD).
        let mut icon_png_base64 = if slot.flags & NIF_ICON != 0 {
            os_png
        } else {
            String::new()
        };
        if icon_png_base64.is_empty() {
            if let Some(ref prev) = prev {
                if !prev.icon_png_base64.is_empty() {
                    icon_png_base64 = prev.icon_png_base64.clone();
                }
            }
        }

        let mut flashing = prev.as_ref().map(|p| p.flashing).unwrap_or(false);
        if is_update {
            let prev_fp = OS_FINGERPRINT.lock().get(&id).cloned();
            let fp_changed = prev_fp.as_ref().is_some_and(|p| p != &fingerprint);
            let from_or_to_blank = blank_frame || prev_fp.as_deref() == Some("__blank__");
            let rapid_swap = if fp_changed {
                let now = std::time::Instant::now();
                let mut map = FP_CHANGES.lock();
                let times = map.entry(id.clone()).or_default();
                times.push(now);
                times.retain(|t| now.duration_since(*t).as_millis() < 2500);
                times.len() >= 2
            } else {
                false
            };
            if blank_frame || from_or_to_blank || rapid_swap {
                flashing = true;
            }
        }
        if flashing && attention_suppressed(&id) {
            flashing = false;
        }
        OS_FINGERPRINT.lock().insert(id.clone(), fingerprint);

        let pin_key = {
            let guid = if looks_like_guid_id(&id) {
                Some(id.as_str())
            } else {
                None
            };
            compute_pin_key(guid, hwnd, uid, &process)
        };
        let resident = is_language_ime_icon(&id, &process, &tooltip);

        TrayIconInfo {
            id,
            pin_key,
            tooltip,
            process,
            uid,
            hwnd,
            callback_msg,
            version,
            icon_png_base64,
            area,
            flashing,
            resident,
        }
    }

    fn apply_hook_slot(slot: &crate::win32::tray_hook_ipc::TrayHookSlot) {
        use crate::win32::tray_hook_ipc::{NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION};
        match slot.message_type {
            NIM_ADD => {
                let info = slot_to_info(slot, false);
                eprintln!("[tray] hook IconAdd {} tip={:?}", info.id, info.tooltip);
                upsert_icon(info);
            }
            NIM_MODIFY | NIM_SETVERSION => {
                upsert_icon(slot_to_info(slot, true));
            }
            NIM_DELETE => {
                let id = stable_id_from_slot(slot);
                eprintln!("[tray] hook IconRemove {id}");
                ICONS.lock().remove(&id);
                OS_FINGERPRINT.lock().remove(&id);
                FP_CHANGES.lock().remove(&id);
                ATTENTION_ACK.lock().remove(&id);
                REG_KEY_BY_ID.lock().remove(&id);
                let _ = sweep_icons();
                publish();
            }
            _ => {}
        }
    }

    /// Prefer a stable id for a registry row.
        fn reg_icon_id(item: &crate::win32::tray_registry::RegTrayIcon) -> String {
            item.icon_guid
                .as_ref()
                .map(|g| crate::win32::tray_registry::guid_key(g))
                .unwrap_or_else(|| format!("reg:{}", item.key))
        }

        fn tip_from_reg(item: &crate::win32::tray_registry::RegTrayIcon, uia_name: Option<&str>) -> String {
            if let Some(n) = uia_name {
                let n = clean_text(n);
                if !n.is_empty() {
                    return n;
                }
            }
            item.initial_tooltip
                .as_ref()
                .map(|t| clean_text(t))
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| {
                    if !item.process.is_empty() {
                        item.process.clone()
                    } else {
                        "未知应用".to_string()
                    }
                })
        }

        fn upsert_stub_from_reg(
            icons: &mut HashMap<String, TrayIconInfo>,
            reg_map: &mut HashMap<String, String>,
            item: &crate::win32::tray_registry::RegTrayIcon,
            uia_name: Option<&str>,
            area: &str,
        ) -> bool {
            if let Some(id) = match_reg_to_spy(icons, item) {
                reg_map.insert(id.clone(), item.key.clone());
                let Some(info) = icons.get_mut(&id) else {
                    return false;
                };
                let mut changed = false;
                if info.area != area {
                    info.area = area.to_string();
                    changed = true;
                }
                let tip = tip_from_reg(item, uia_name);
                if (looks_like_raw_id(&info.tooltip) || info.tooltip == "未知应用") && !tip.is_empty() {
                    info.tooltip = tip;
                    changed = true;
                }
                if info.icon_png_base64.is_empty() && !item.icon_snapshot.is_empty() {
                    let b64 = snapshot_to_png_b64(&item.icon_snapshot);
                    if !b64.is_empty() {
                        info.icon_png_base64 = b64;
                        changed = true;
                    }
                }
                if info.process.is_empty() && !item.process.is_empty() {
                    info.process = item.process.clone();
                    changed = true;
                }
                let resident = is_language_ime_icon(&info.id, &info.process, &info.tooltip);
                if info.resident != resident {
                    info.resident = resident;
                    changed = true;
                }
                return changed;
            }

            let id = reg_icon_id(item);
            reg_map.insert(id.clone(), item.key.clone());
            if let Some(info) = icons.get_mut(&id) {
                let mut changed = false;
                let tip = tip_from_reg(item, uia_name);
                if info.tooltip != tip && !tip.is_empty() {
                    info.tooltip = tip;
                    changed = true;
                }
                if info.area != area {
                    info.area = area.to_string();
                    changed = true;
                }
                let resident = is_language_ime_icon(&info.id, &info.process, &info.tooltip);
                if info.resident != resident {
                    info.resident = resident;
                    changed = true;
                }
                return changed;
            }

            icons.insert(
                id.clone(),
                {
                    let tip = tip_from_reg(item, uia_name);
                    let process = item.process.clone();
                    let pin_key = {
                        let guid = item
                            .icon_guid
                            .as_ref()
                            .map(|g| crate::win32::tray_registry::guid_key(g));
                        let path = item
                            .executable_path
                            .to_string_lossy()
                            .to_ascii_lowercase();
                        let uid = item.icon_uid.unwrap_or(0);
                        if let Some(ref g) = guid {
                            if looks_like_guid_id(g) {
                                g.clone()
                            } else if !path.is_empty() {
                                format!("exe:{path}:{uid}")
                            } else {
                                compute_pin_key(None, 0, uid, &item.process)
                            }
                        } else if !path.is_empty() {
                            format!("exe:{path}:{uid}")
                        } else {
                            compute_pin_key(None, 0, uid, &item.process)
                        }
                    };
                    let resident = is_language_ime_icon(&id, &process, &tip);
                    TrayIconInfo {
                        id: id.clone(),
                        pin_key,
                        tooltip: tip,
                        process,
                        uid: item.icon_uid.unwrap_or(0),
                        hwnd: 0,
                        callback_msg: 0,
                        version: 0,
                        icon_png_base64: snapshot_to_png_b64(&item.icon_snapshot),
                        area: area.to_string(),
                        flashing: false,
                        resident,
                    }
                },
            );
            true
        }

    /// Apply registry enrichment into ICONS (tooltips / snapshots / area). Does not
    /// invent existence: stubs are only seeded when the hook list is still empty
    /// (cold start), and never go through UIA / demote.
    fn apply_registry_snapshot(seed_stubs: bool) -> (bool, usize) {
        use crate::win32::tray_registry;

        let pool = tray_registry::enum_match_pool();
        let mut changed = false;
        let mut missing = 0usize;
        let mut keep_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

        {
            let mut icons = ICONS.lock();
            let mut reg_map = REG_KEY_BY_ID.lock();

            for (id, info) in icons.iter() {
                if is_clickable(info) {
                    keep_ids.insert(id.clone());
                }
            }

            for item in &pool {
                let area = if item.is_promoted {
                    "taskbar"
                } else {
                    "overflow"
                };
                if let Some(id) = match_reg_to_spy(&icons, item) {
                    reg_map.insert(id.clone(), item.key.clone());
                    keep_ids.insert(id.clone());
                    if let Some(info) = icons.get_mut(&id) {
                        if info.area != area {
                            info.area = area.to_string();
                            changed = true;
                        }
                        let tip = tip_from_reg(item, None);
                        if (looks_like_raw_id(&info.tooltip) || info.tooltip == "未知应用")
                            && !tip.is_empty()
                        {
                            info.tooltip = tip;
                            changed = true;
                        }
                        if info.icon_png_base64.is_empty() && !item.icon_snapshot.is_empty() {
                            let b64 = snapshot_to_png_b64(&item.icon_snapshot);
                            if !b64.is_empty() {
                                info.icon_png_base64 = b64;
                                changed = true;
                            }
                        }
                        if info.process.is_empty() && !item.process.is_empty() {
                            info.process = item.process.clone();
                            changed = true;
                        }
                    }
                } else if seed_stubs && !HOOK_PRIMARY.load(Ordering::SeqCst) {
                    // Spy-fallback cold seed only — hook mode waits for COPYDATA.
                    missing += 1;
                    if upsert_stub_from_reg(&mut icons, &mut reg_map, item, None, area) {
                        changed = true;
                    }
                    keep_ids.insert(reg_icon_id(item));
                } else if seed_stubs {
                    missing += 1;
                }
            }

            // Drop stale non-clickable stubs not matched this pass.
            if seed_stubs {
                let drop_ids: Vec<String> = icons
                    .iter()
                    .filter(|(id, info)| !is_clickable(info) && !keep_ids.contains(*id))
                    .map(|(id, _)| id.clone())
                    .collect();
                for id in &drop_ids {
                    icons.remove(id);
                    reg_map.remove(id);
                    changed = true;
                }
            }
        }

        if sweep_icons() {
            changed = true;
        }
        (changed, missing)
    }

    /// Enrich from registry only — never TaskbarCreated / UIA.
    fn reconcile_once() -> bool {
        let (changed, _) = apply_registry_snapshot(false);
        changed
    }

    fn start_reconcile_loop() {
        std::thread::Builder::new()
            .name("tray-reconcile".into())
            .spawn(|| {
                let seed_stubs = !HOOK_PRIMARY.load(Ordering::SeqCst);
                let seed = std::panic::catch_unwind(|| apply_registry_snapshot(seed_stubs));
                match seed {
                    Ok((changed, missing)) => {
                        eprintln!(
                            "[tray] startup enrich: changed={changed} missing={missing} icons={} hook={}",
                            ICONS.lock().len(),
                            HOOK_PRIMARY.load(Ordering::SeqCst)
                        );
                        let _ = changed;
                        publish();
                    }
                    Err(err) => {
                        eprintln!("[tray] startup enrich panicked: {err:?}");
                    }
                }

                let started = std::time::Instant::now();
                loop {
                    let interval = if started.elapsed().as_secs() < 30 {
                        std::time::Duration::from_secs(2)
                    } else {
                        std::time::Duration::from_secs(15)
                    };
                    std::thread::sleep(interval);
                    match std::panic::catch_unwind(|| reconcile_once()) {
                        Ok(true) => publish(),
                        Ok(false) => {}
                        Err(err) => eprintln!("[tray] reconcile panicked: {err:?}"),
                    }
                }
            })
            .expect("spawn tray-reconcile");
    }

    fn start_hook_loop() -> bool {
        match crate::win32::tray_hook_host::start_host() {
            Ok(true) => {}
            Ok(false) => {
                eprintln!("[tray] hook host returned false");
                return false;
            }
            Err(err) => {
                eprintln!("[tray] hook install failed: {err}");
                return false;
            }
        }
        HOOK_PRIMARY.store(true, Ordering::SeqCst);

        std::thread::Builder::new()
            .name("tray-hook".into())
            .spawn(|| {
                // One cold-start refill after the hook is live so NIM_ADDs hit explorer
                // (and our CALLWNDPROC) — not a spy FindWindow race.
                std::thread::sleep(std::time::Duration::from_millis(300));
                broadcast_taskbar_created();

                let mut buf = Vec::with_capacity(16);
                loop {
                    let _ = crate::win32::tray_hook_host::wait_event(500);
                    if let Err(err) = crate::win32::tray_hook_host::ensure_hook() {
                        eprintln!("[tray] hook rehang: {err}");
                        std::thread::sleep(std::time::Duration::from_secs(2));
                        continue;
                    }
                    buf.clear();
                    let n = crate::win32::tray_hook_host::drain_slots(&mut buf);
                    if n == 0 {
                        continue;
                    }
                    for slot in &buf {
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            apply_hook_slot(slot);
                        }));
                    }
                }
            })
            .expect("spawn tray-hook");
        true
    }

    fn start_spy_fallback() {
        eprintln!("[tray] starting systray-util spy fallback");
        HOOK_PRIMARY.store(false, Ordering::SeqCst);
        std::thread::Builder::new()
            .name("tray-spy".into())
            .spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(200));
                let mut systray = match Systray::new() {
                    Ok(s) => s,
                    Err(err) => {
                        eprintln!("[tray] Systray::new failed: {err:?}");
                        return;
                    }
                };
                eprintln!("[tray] spy online (fallback)");
                std::thread::sleep(std::time::Duration::from_millis(400));
                broadcast_taskbar_created();

                while let Some(event) = systray.events_blocking() {
                    match event {
                        SystrayEvent::IconAdd(icon) => {
                            eprintln!(
                                "[tray] IconAdd {} tip={:?}",
                                icon.stable_id, icon.tooltip
                            );
                            upsert_icon(to_info(&icon, false));
                        }
                        SystrayEvent::IconUpdate(icon) => {
                            upsert_icon(to_info(&icon, true));
                        }
                        SystrayEvent::IconRemove(id) => {
                            eprintln!("[tray] IconRemove {id}");
                            let key = id.to_string();
                            ICONS.lock().remove(&key);
                            OS_FINGERPRINT.lock().remove(&key);
                            FP_CHANGES.lock().remove(&key);
                            REG_KEY_BY_ID.lock().remove(&key);
                            let _ = sweep_icons();
                            publish();
                        }
                    }
                }
            })
            .expect("spawn tray-spy");
    }

    /// Start tray tracking: explorer hook first, spy only if hook fails.
    pub fn start<F, A, P>(on_change: F, on_attention: A, on_prefs: P)
    where
        F: Fn(Vec<TrayIconInfo>) + Send + Sync + 'static,
        A: Fn(TrayAttention) + Send + Sync + 'static,
        P: Fn(TrayPrefs) + Send + Sync + 'static,
    {
        let _ = EMIT.set(Box::new(on_change));
        let _ = ATTENTION.set(Box::new(on_attention));
        let _ = PREFS_EMIT.set(Box::new(on_prefs));

        // Recover if a previous build left the cursor hidden via ShowCursor.
        #[cfg(windows)]
        {
            use windows::Win32::UI::WindowsAndMessaging::ShowCursor;
            unsafe {
                for _ in 0..16 {
                    if ShowCursor(true) >= 0 {
                        break;
                    }
                }
            }
        }

        if start_hook_loop() {
            start_reconcile_loop();
        } else {
            start_reconcile_loop();
            start_spy_fallback();
        }
    }

    /// Last measured popup-menu height per icon id (auto mode).
    static MENU_HEIGHT_CACHE: LazyLock<Mutex<HashMap<String, i32>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    fn pack_i32(low: i16, high: i16) -> i32 {
        ((high as u32) << 16 | (low as u16) as u32) as i32
    }

    fn cursor_pos() -> (i32, i32) {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        (pt.x, pt.y)
    }

    /// WeChat / QQ default custom menu height when user has not set one.
    const DEFAULT_TENCENT_MENU_HEIGHT: i32 = 200;

    /// WeChat / QQ — may need forced VERSION_4 CONTEXTMENU when version stays 0.
    fn is_tencent_im(process: &str, tip: &str) -> bool {
        let p = process.trim().to_ascii_lowercase();
        if matches!(
            p.as_str(),
            "weixin" | "wechat" | "wechatappex" | "qq" | "qqnt" | "tim"
        ) || p.starts_with("weixin")
            || p.starts_with("wechat")
            || p.starts_with("qqnt")
            || p.starts_with("qq")
        {
            return true;
        }
        let t = tip.trim();
        t == "微信" || t == "QQ" || t.starts_with("微信")
    }

    fn process_stem_for_pid(pid: u32) -> String {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        };
        if pid == 0 {
            return String::new();
        }
        unsafe {
            let Ok(proc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return String::new();
            };
            let mut buf = [0u16; 520];
            let mut size = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(
                proc,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(buf.as_mut_ptr()),
                &mut size,
            );
            let _ = CloseHandle(proc);
            if ok.is_err() || size == 0 {
                return String::new();
            }
            let path = String::from_utf16_lossy(&buf[..size as usize]);
            std::path::Path::new(&path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
        }
    }

    fn cached_menu_height(icon_id: &str) -> i32 {
        MENU_HEIGHT_CACHE
            .lock()
            .get(icon_id)
            .copied()
            .unwrap_or(160)
            .clamp(48, 640)
    }

    fn remember_menu_height(icon_id: &str, height: i32) {
        if !(48..=640).contains(&height) || icon_id.is_empty() {
            return;
        }
        // Don't overwrite a user-fixed height.
        if get_prefs()
            .menu_heights
            .get(icon_id)
            .copied()
            .is_some_and(|h| h > 0)
        {
            return;
        }
        MENU_HEIGHT_CACHE
            .lock()
            .insert(icon_id.to_string(), height);
    }

    fn snapshot_popup_menus() -> Vec<isize> {
        use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
        use windows::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GetClassNameW, GetWindowLongW, GetWindowRect, IsWindowVisible,
            GWL_STYLE, WS_POPUP,
        };

        struct Ctx {
            list: Vec<isize>,
        }

        unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let ctx = &mut *(lparam.0 as *mut Ctx);
            if !IsWindowVisible(hwnd).as_bool() {
                return BOOL(1);
            }
            let mut class = [0u16; 64];
            let n = GetClassNameW(hwnd, &mut class);
            let name = if n > 0 {
                String::from_utf16_lossy(&class[..n as usize])
            } else {
                String::new()
            };
            let mut rc = RECT::default();
            if GetWindowRect(hwnd, &mut rc).is_err() {
                return BOOL(1);
            }
            let w = rc.right - rc.left;
            let h = rc.bottom - rc.top;
            let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
            let popup = (style & WS_POPUP.0) != 0;
            let menu_class = name == "#32768";
            let size_ok = (80..720).contains(&w) && (40..900).contains(&h);
            if menu_class || (popup && size_ok) {
                ctx.list.push(hwnd.0 as isize);
            }
            BOOL(1)
        }

        let mut ctx = Ctx { list: Vec::new() };
        unsafe {
            let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut _ as isize));
        }
        ctx.list
    }

    /// Find a newly shown menu-like popup. Accepts `#32768` and same-process popups
    /// (WeChat / QQ NT often use custom classes, not only `#32768`).
    fn find_new_popup_menu(owner_pid: u32, before: &[isize], tencent: bool) -> Option<isize> {
        use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
        use windows::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GetClassNameW, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId,
            IsWindowVisible, GWL_STYLE, WS_POPUP,
        };

        #[derive(Clone)]
        struct Cand {
            hwnd: isize,
            pid: u32,
            class: String,
            popup: bool,
        }

        struct Ctx {
            before: Vec<isize>,
            cands: Vec<Cand>,
        }

        unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let ctx = &mut *(lparam.0 as *mut Ctx);
            if !IsWindowVisible(hwnd).as_bool() {
                return BOOL(1);
            }
            let h = hwnd.0 as isize;
            if ctx.before.contains(&h) {
                return BOOL(1);
            }
            let mut class = [0u16; 64];
            let n = GetClassNameW(hwnd, &mut class);
            let name = if n > 0 {
                String::from_utf16_lossy(&class[..n as usize])
            } else {
                String::new()
            };
            if name == "Shell_TrayWnd" || name == "Progman" || name == "WorkerW" {
                return BOOL(1);
            }

            let mut rc = RECT::default();
            if GetWindowRect(hwnd, &mut rc).is_err() {
                return BOOL(1);
            }
            let w = rc.right - rc.left;
            let hgt = rc.bottom - rc.top;
            if !(60..800).contains(&w) || !(36..900).contains(&hgt) {
                return BOOL(1);
            }

            let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
            let popup = (style & WS_POPUP.0) != 0;
            let menu_class = name == "#32768";
            if !(menu_class || popup) {
                return BOOL(1);
            }

            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            ctx.cands.push(Cand {
                hwnd: h,
                pid,
                class: name,
                popup,
            });
            BOOL(1)
        }

        let mut ctx = Ctx {
            before: before.to_vec(),
            cands: Vec::new(),
        };
        unsafe {
            let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut _ as isize));
        }

        let mut best: Option<(i32, isize)> = None;
        for c in ctx.cands {
            let mut score = 0i32;
            if c.class == "#32768" {
                score += 100;
            }
            if c.popup {
                score += 20;
            }
            if c.pid == owner_pid {
                score += 40;
            } else if tencent {
                let stem = process_stem_for_pid(c.pid);
                if stem.contains("weixin")
                    || stem.contains("wechat")
                    || stem == "qq"
                    || stem.starts_with("qq")
                    || stem == "tim"
                {
                    score += 35;
                }
            }
            // QQ NT / Electron-style menus
            if tencent
                && (c.class.contains("Chrome_WidgetWin")
                    || c.class.contains("WeChat")
                    || c.class.contains("Weixin")
                    || c.class.contains("TXGui"))
            {
                score += 25;
            }
            if score >= 40 {
                match best {
                    Some((s, _)) if s >= score => {}
                    _ => best = Some((score, c.hwnd)),
                }
            }
        }
        best.map(|(_, h)| h)
    }

    /// Move an already-open menu so its top-left sits at `anchor` (below the strip/click).
    fn reposition_popup_menu(menu: isize, anchor: (i32, i32)) -> Option<i32> {
        use windows::Win32::Foundation::{HWND, RECT};
        use windows::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, GetWindowRect, SetWindowPos, HWND_TOPMOST, SM_CXSCREEN, SM_CYSCREEN,
            SWP_NOACTIVATE, SWP_NOSIZE,
        };

        let hwnd = HWND(menu as *mut _);
        let mut rc = RECT::default();
        unsafe {
            if GetWindowRect(hwnd, &mut rc).is_err() {
                return None;
            }
        }
        let height = rc.bottom - rc.top;
        let width = rc.right - rc.left;
        if height <= 0 || width <= 0 {
            return None;
        }

        let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
        let mut x = anchor.0;
        let mut y = anchor.1;
        if x + width > screen_w {
            x = (screen_w - width).max(0);
        }
        if y + height > screen_h {
            y = (screen_h - height).max(0);
        }
        x = x.max(0);
        y = y.max(0);

        unsafe {
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        Some(height)
    }

    /// Logical notify coords for BOTTOMALIGN menus — pointer stays put.
    fn menu_msg_anchor(click: (i32, i32), menu_height: i32) -> (i32, i32) {
        let strip = crate::win32::appbar::strip_height_px().saturating_add(6);
        let top = (click.1 + 4).max(strip);
        (click.0, top + menu_height.max(48))
    }

    fn notify_icon_at(
        hwnd: isize,
        callback: u32,
        uid: u32,
        version: u32,
        message: u32,
        cursor: (i32, i32),
    ) -> Result<(), String> {
        use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::SendNotifyMessageW;

        // NOTIFYICON_VERSION_4+: wParam = cursor, lParam = MAKEWPARAM(msg, uid)
        // older: wParam = uid, lParam = MAKEWPARAM(msg, 0)
        let (wparam, lparam) = if version > 3 {
            let (x, y) = cursor;
            (
                WPARAM(pack_i32(x as i16, y as i16) as usize),
                LPARAM(pack_i32(message as i16, uid as i16) as isize),
            )
        } else {
            (
                WPARAM(uid as usize),
                LPARAM(pack_i32(message as i16, 0) as isize),
            )
        };

        unsafe {
            SendNotifyMessageW(HWND(hwnd as *mut _), callback, wparam, lparam)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn invoke_icon_by_id(
        id: Option<String>,
        hwnd: isize,
        callback_msg: u32,
        uid: u32,
        version: u32,
        click: TrayClick,
    ) -> Result<(), String> {
        // Resolve from cache when frontend passes an id (registry stubs have hwnd=0).
        let resolved = {
            let icons = ICONS.lock();
            if let Some(ref id) = id {
                icons.get(id).cloned()
            } else if hwnd != 0 {
                icons
                    .values()
                    .find(|i| i.hwnd == hwnd && i.uid == uid)
                    .cloned()
            } else {
                None
            }
        };

        let mut hwnd = hwnd;
        let mut callback_msg = callback_msg;
        let mut uid = uid;
        let mut version = version;
        let mut icon_id = id.clone();

        let mut process = String::new();
        let mut tip = String::new();
        if let Some(info) = resolved {
            process = info.process.clone();
            tip = info.tooltip.clone();
            // Prefer live cache over frontend payload — UI can show a correct tip
            // while holding a stale hwnd from a previous merge bug.
            if is_clickable(&info) {
                hwnd = info.hwnd;
                callback_msg = info.callback_msg;
                uid = info.uid;
                version = info.version;
                icon_id = Some(info.id.clone());
            } else {
                if hwnd == 0 {
                    hwnd = info.hwnd;
                }
                if callback_msg == 0 {
                    callback_msg = info.callback_msg;
                }
                if uid == 0 {
                    uid = info.uid;
                }
                if version == 0 {
                    version = info.version;
                }
                icon_id = Some(info.id.clone());

                // Stub / incomplete → find a clickable twin. NEVER match by uid alone
                // (Clash / AI助手 / many apps reuse uid=2 across processes).
                if hwnd == 0 || callback_msg == 0 {
                    let proc = info.process.trim().to_ascii_lowercase();
                    let icons = ICONS.lock();
                    let twin = if proc.is_empty() {
                        None
                    } else {
                        icons
                            .values()
                            .find(|c| {
                                is_clickable(c)
                                    && c.id != info.id
                                    && c.process.trim().eq_ignore_ascii_case(&proc)
                                    && (info.uid == 0 || c.uid == info.uid)
                            })
                            .cloned()
                            .or_else(|| {
                                let hits: Vec<_> = icons
                                    .values()
                                    .filter(|c| {
                                        is_clickable(c)
                                            && c.process.trim().eq_ignore_ascii_case(&proc)
                                    })
                                    .cloned()
                                    .collect();
                                if hits.len() == 1 {
                                    Some(hits[0].clone())
                                } else {
                                    None
                                }
                            })
                    };
                    if let Some(c) = twin {
                        hwnd = c.hwnd;
                        callback_msg = c.callback_msg;
                        uid = c.uid;
                        version = c.version;
                        icon_id = Some(c.id);
                        if process.is_empty() {
                            process = c.process;
                        }
                    }
                }
            }
        }
        if process.is_empty() && hwnd != 0 {
            process = process_label(hwnd);
        }

        if hwnd != 0 && callback_msg != 0 {
            crate::win32::fullscreen::note_tray_interaction();
            let id_str = icon_id.clone().unwrap_or_default();
            // 点开即复位 flashing→0（先于回调），下次新消息才能再弹岛通知
            clear_flashing(
                if id_str.is_empty() {
                    None
                } else {
                    Some(id_str.as_str())
                },
                hwnd,
                uid,
            );
            return invoke_via_notify(
                hwnd,
                callback_msg,
                uid,
                version,
                click,
                &process,
                &tip,
                &id_str,
            );
        }

        Err(format!(
            "托盘点击失败：缺少 hwnd/callback（id={:?}）— 仅转发 Shell_NotifyIcon 回调，不走 UIA/demote",
            icon_id
        ))
    }

    fn invoke_via_notify(
        hwnd: isize,
        callback_msg: u32,
        uid: u32,
        version: u32,
        click: TrayClick,
        process: &str,
        tip: &str,
        icon_id: &str,
    ) -> Result<(), String> {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{
            AllowSetForegroundWindow, GetWindowThreadProcessId, IsWindow, WM_CONTEXTMENU,
            WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_USER,
        };
        const NIN_SELECT: u32 = WM_USER + 0;

        let h = HWND(hwnd as *mut _);
        let mut owner_pid = 0u32;
        unsafe {
            if !IsWindow(h).as_bool() {
                return Err("tray owner window gone".into());
            }

            GetWindowThreadProcessId(h, Some(&mut owner_pid));
            if owner_pid != 0 {
                let _ = AllowSetForegroundWindow(owner_pid);
            }
        }

        crate::win32::topmost::yield_for(1_800);

        let click_pt = cursor_pos();
        let adapt_menu = matches!(click, TrayClick::Right);
        let tencent = is_tencent_im(process, tip);
        let pin_key = {
            let guid = if looks_like_guid_id(icon_id) {
                Some(icon_id)
            } else {
                None
            };
            compute_pin_key(guid, hwnd, uid, process)
        };

        let est_h = if adapt_menu {
            effective_menu_height(icon_id, &pin_key, process, tip)
        } else {
            160
        };
        // Message coords only — never SetCursorPos / ShowCursor (pointer stays put for aiming).
        let (msg_x, msg_y) = if adapt_menu {
            menu_msg_anchor(click_pt, est_h)
        } else {
            let strip = crate::win32::appbar::strip_height_px().saturating_add(8);
            (click_pt.0, (click_pt.1 + 8).max(strip))
        };
        let place_top = {
            let strip_top = crate::win32::appbar::strip_height_px().saturating_add(6);
            (click_pt.1 + 4).max(strip_top)
        };
        let place_x = click_pt.0;

        let menus_before = if adapt_menu {
            snapshot_popup_menus()
        } else {
            Vec::new()
        };

        // Tencent: always pack as VERSION_4 so wParam carries screen coords.
        let pack_ver = if adapt_menu && tencent {
            version.max(4)
        } else {
            version
        };

        let messages: &[u32] = match click {
            TrayClick::Left => &[WM_LBUTTONDOWN, WM_LBUTTONUP],
            TrayClick::Right => &[WM_RBUTTONDOWN, WM_RBUTTONUP],
        };

        for &msg in messages {
            notify_icon_at(
                hwnd,
                callback_msg,
                uid,
                pack_ver,
                msg,
                (msg_x, msg_y),
            )?;
        }

        let extra = match click {
            TrayClick::Left => NIN_SELECT,
            TrayClick::Right => WM_CONTEXTMENU,
        };
        if pack_ver >= 3 || (adapt_menu && tencent) {
            notify_icon_at(
                hwnd,
                callback_msg,
                uid,
                pack_ver.max(4),
                extra,
                (msg_x, msg_y),
            )?;
        }

        if adapt_menu {
            let id_key = icon_id.to_string();
            std::thread::spawn(move || {
                let deadline =
                    std::time::Instant::now() + std::time::Duration::from_millis(1_200);
                let mut last_menu = 0isize;
                while std::time::Instant::now() < deadline {
                    if let Some(menu) =
                        find_new_popup_menu(owner_pid, &menus_before, tencent)
                    {
                        last_menu = menu;
                        if let Some(h) =
                            reposition_popup_menu(menu, (place_x, place_top))
                        {
                            remember_menu_height(&id_key, h);
                        }
                        // Keep fighting TrackPopupMenu / Electron layout for a bit.
                        for _ in 0..16 {
                            std::thread::sleep(std::time::Duration::from_millis(16));
                            let _ = reposition_popup_menu(menu, (place_x, place_top));
                        }
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(16));
                }
                if last_menu != 0 {
                    let _ = reposition_popup_menu(last_menu, (place_x, place_top));
                }
            });
        }

        let _ = WM_LBUTTONDBLCLK;
        Ok(())
    }

    /// Frontend / island: reset tray blink attention so the next message can notify again.
    pub fn acknowledge_icon_attention(id: Option<String>, hwnd: isize, uid: u32) {
        clear_flashing(id.as_deref(), hwnd, uid);
    }
}

#[cfg(windows)]
pub use win::{
    acknowledge_icon_attention, get_prefs, invoke_icon_by_id, list_icons, set_prefs, start,
};

#[cfg(not(windows))]
pub fn list_icons() -> Vec<TrayIconInfo> {
    Vec::new()
}

#[cfg(not(windows))]
pub fn get_prefs() -> TrayPrefs {
    TrayPrefs::default()
}

#[cfg(not(windows))]
pub fn set_prefs(_prefs: TrayPrefs) {}

#[cfg(not(windows))]
pub fn invoke_icon_by_id(
    _id: Option<String>,
    _hwnd: isize,
    _callback_msg: u32,
    _uid: u32,
    _version: u32,
    _click: TrayClick,
) -> Result<(), String> {
    Err("tray only on Windows".into())
}

#[cfg(not(windows))]
pub fn acknowledge_icon_attention(_id: Option<String>, _hwnd: isize, _uid: u32) {}

#[cfg(not(windows))]
pub fn start<F, A, P>(_on_change: F, _on_attention: A, _on_prefs: P)
where
    F: Fn(Vec<TrayIconInfo>) + Send + Sync + 'static,
    A: Fn(TrayAttention) + Send + Sync + 'static,
    P: Fn(TrayPrefs) + Send + Sync + 'static,
{
}
