//! Unified global hotkey registry (system + plugin bindings).
//!
//! One Win32 message loop registers all chords via `RegisterHotKey` and emits
//! `hotkey-action` when fired. System chords live in `prefs_hotkeys`; plugin
//! chords come from enabled plugins' `settings[]` with `type: "hotkey"`.
//! Action `island.search.toggle` is owned by the system binding — plugin fields
//! with that action alias it (no double RegisterHotKey).

#![cfg(windows)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
    MOD_SHIFT, MOD_WIN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, PeekMessageW, PostThreadMessageW, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, MSG, PM_REMOVE,
    WH_KEYBOARD_LL, WM_HOTKEY, WM_KEYDOWN, WM_NULL, WM_QUIT, WM_SYSKEYDOWN,
};

const ACTION_ISLAND_SEARCH: &str = "island.search.toggle";
const ACTION_DOCK_TOGGLE: &str = "dock.toggle";
const ID_ISLAND_SEARCH: &str = "system.islandSearch";
const ID_DOCK_TOGGLE: &str = "system.dockToggle";
/// Temporary RegisterHotKey while recording — eats Space+mod so OS / IME
/// cannot steal them (Alt system menu, Ctrl/Shift IME, Win language switch).
const RECORDING_SHIELD_ALT_SPACE: i32 = 0x15EE;
const RECORDING_SHIELD_CTRL_SPACE: i32 = 0x15EF;
const RECORDING_SHIELD_SHIFT_SPACE: i32 = 0x15F0;
const RECORDING_SHIELD_WIN_SPACE: i32 = 0x15F1;
const VK_SPACE: u32 = 0x20;

static RUNNING: AtomicBool = AtomicBool::new(false);
static WORKER_TID: AtomicI32 = AtomicI32::new(0);
static TX: Mutex<Option<Sender<Cmd>>> = Mutex::new(None);
/// Recording: LL hook eats **Space while any modifier is down** (blocks system
/// menu / IME). Alt/Ctrl/Shift/Win themselves are never swallowed.
static RECORDING_ACTIVE: AtomicBool = AtomicBool::new(false);
static RECORDING_HOOK: AtomicIsize = AtomicIsize::new(0);
static RECORDING_APP: Mutex<Option<AppHandle>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyBindingDto {
    pub id: String,
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_name: Option<String>,
    pub key: String,
    pub label: String,
    pub action: String,
    pub chord: String,
    /// True when this row edits the system island-search chord.
    #[serde(default)]
    pub aliases_system_search: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct HotkeyPrefs {
    #[serde(default)]
    system: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct ParsedChord {
    mods: u32,
    vk: u32,
    /// 1 = normal; 2 = hold mods and tap the key twice (e.g. Alt+Space+Space).
    taps: u8,
    display: String,
}

#[derive(Debug, Clone)]
struct LiveBinding {
    id: String,
    action: String,
    plugin_id: Option<String>,
    chord: ParsedChord,
    /// OS RegisterHotKey id — shared by bindings with the same mods+vk base.
    hotkey_id: i32,
}

enum Cmd {
    Reload { app: AppHandle },
    SuspendRecording,
    ResumeRecording,
    Shutdown,
}

fn load_prefs() -> HotkeyPrefs {
    let raw = crate::db::with_conn(|c| crate::db::hotkeys_get(c))
        .ok()
        .flatten();
    let mut prefs = match raw {
        Some(v) => serde_json::from_value::<HotkeyPrefs>(v).unwrap_or_default(),
        None => HotkeyPrefs::default(),
    };
    prefs
        .system
        .entry("islandSearch".into())
        .or_insert_with(|| "Alt+Space".into());
    prefs
        .system
        .entry("dockToggle".into())
        .or_insert_with(|| "Ctrl+Alt+D".into());
    prefs
}

fn save_prefs(prefs: &HotkeyPrefs) -> Result<(), String> {
    let v = serde_json::to_value(prefs).map_err(|e| e.to_string())?;
    crate::db::with_conn(|c| crate::db::hotkeys_set(c, &v))
}

/// Normalize user / stored chord text → canonical display form.
pub fn normalize_chord(raw: &str) -> Result<String, String> {
    if raw.trim().is_empty() {
        return Ok(String::new());
    }
    let p = parse_chord(raw)?;
    Ok(p.display)
}

fn parse_chord(raw: &str) -> Result<ParsedChord, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("空快捷键".into());
    }
    // Alt+Space×2 / Alt+Space*2
    let (body, mul_taps) = if let Some((a, b)) = s.rsplit_once('×') {
        let n: u8 = b
            .trim()
            .parse()
            .map_err(|_| format!("无效快捷键: {raw}"))?;
        if !(2..=4).contains(&n) {
            return Err(format!("连按次数须为 2–4: {raw}"));
        }
        (a.trim(), Some(n))
    } else if let Some((a, b)) = s.rsplit_once('*') {
        if b.trim().chars().all(|c| c.is_ascii_digit()) {
            let n: u8 = b
                .trim()
                .parse()
                .map_err(|_| format!("无效快捷键: {raw}"))?;
            if !(2..=4).contains(&n) {
                return Err(format!("连按次数须为 2–4: {raw}"));
            }
            (a.trim(), Some(n))
        } else {
            (s, None)
        }
    } else {
        (s, None)
    };

    let mut mods = 0u32;
    let mut key_toks: Vec<String> = Vec::new();
    for part in body.split('+') {
        let t = part.trim();
        if t.is_empty() {
            continue;
        }
        let lower = t.to_ascii_lowercase();
        match lower.as_str() {
            "ctrl" | "control" => mods |= MOD_CONTROL.0,
            "alt" | "option" => mods |= MOD_ALT.0,
            "shift" => mods |= MOD_SHIFT.0,
            "win" | "super" | "meta" | "cmd" => mods |= MOD_WIN.0,
            _ => key_toks.push(t.to_string()),
        }
    }
    if key_toks.is_empty() {
        return Err(format!("缺少主键: {raw}"));
    }
    let first = canonical_key_token(&key_toks[0]);
    for k in &key_toks {
        if canonical_key_token(k) != first {
            return Err(format!(
                "连按快捷键主键必须相同（如 Alt+Space+Space）: {raw}"
            ));
        }
    }
    let taps = mul_taps.unwrap_or(key_toks.len() as u8).max(1);
    if taps > 4 {
        return Err(format!("连按次数过多: {raw}"));
    }
    let vk = vk_from_token(&first)?;
    if mods == 0 {
        return Err("全局快捷键至少需要一个修饰键（Ctrl/Alt/Shift/Win）".into());
    }
    let display = format_chord(mods, &first, taps);
    Ok(ParsedChord {
        mods,
        vk,
        taps,
        display,
    })
}

fn format_chord(mods: u32, key_token: &str, taps: u8) -> String {
    let mut out: Vec<String> = Vec::new();
    if mods & MOD_CONTROL.0 != 0 {
        out.push("Ctrl".into());
    }
    if mods & MOD_ALT.0 != 0 {
        out.push("Alt".into());
    }
    if mods & MOD_SHIFT.0 != 0 {
        out.push("Shift".into());
    }
    if mods & MOD_WIN.0 != 0 {
        out.push("Win".into());
    }
    let key = canonical_key_token(key_token);
    for _ in 0..taps.max(1) {
        out.push(key.clone());
    }
    out.join("+")
}

fn canonical_key_token(key: &str) -> String {
    let lower = key.to_ascii_lowercase();
    match lower.as_str() {
        " " | "space" | "spacebar" => "Space".into(),
        "esc" | "escape" => "Esc".into(),
        "return" | "enter" => "Enter".into(),
        "tab" => "Tab".into(),
        "backspace" | "bksp" => "Backspace".into(),
        "delete" | "del" => "Delete".into(),
        "insert" | "ins" => "Insert".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "pageup" | "pgup" => "PageUp".into(),
        "pagedown" | "pgdn" => "PageDown".into(),
        "up" | "arrowup" => "Up".into(),
        "down" | "arrowdown" => "Down".into(),
        "left" | "arrowleft" => "Left".into(),
        "right" | "arrowright" => "Right".into(),
        "plus" => "Plus".into(),
        "minus" | "-" => "-".into(),
        "=" => "=".into(),
        k if k.len() == 1 => k.to_ascii_uppercase(),
        k if k.starts_with('f')
            && k[1..]
                .parse::<u8>()
                .ok()
                .is_some_and(|n| (1..=24).contains(&n)) =>
        {
            format!("F{}", &k[1..])
        }
        _ => {
            let mut c = key.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => key.to_string(),
            }
        }
    }
}

fn vk_from_token(key: &str) -> Result<u32, String> {
    let lower = key.to_ascii_lowercase();
    let vk = match lower.as_str() {
        " " | "space" | "spacebar" => 0x20,
        "esc" | "escape" => 0x1B,
        "enter" | "return" => 0x0D,
        "tab" => 0x09,
        "backspace" | "bksp" => 0x08,
        "delete" | "del" => 0x2E,
        "insert" | "ins" => 0x2D,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" | "pgup" => 0x21,
        "pagedown" | "pgdn" => 0x22,
        "up" | "arrowup" => 0x26,
        "down" | "arrowdown" => 0x28,
        "left" | "arrowleft" => 0x25,
        "right" | "arrowright" => 0x27,
        "plus" => 0xBB,
        "minus" | "-" => 0xBD,
        "=" => 0xBB,
        k if k.len() == 1 => {
            let c = k.chars().next().unwrap();
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase() as u32
            } else {
                return Err(format!("不支持的主键: {key}"));
            }
        }
        k if k.starts_with('f') => {
            let n: u8 = k[1..]
                .parse()
                .map_err(|_| format!("不支持的主键: {key}"))?;
            if !(1..=24).contains(&n) {
                return Err(format!("不支持的主键: {key}"));
            }
            0x70 + (n as u32 - 1)
        }
        _ => return Err(format!("不支持的主键: {key}")),
    };
    Ok(vk)
}

fn collect_bindings() -> Result<(Vec<LiveBinding>, Vec<HotkeyBindingDto>), String> {
    let prefs = load_prefs();
    let mut dtos: Vec<HotkeyBindingDto> = Vec::new();
    let mut live: Vec<LiveBinding> = Vec::new();
    let mut next_id: i32 = 0x1600;
    let mut used_chords: HashMap<String, String> = HashMap::new();
    let mut base_hotkey_id: HashMap<(u32, u32), i32> = HashMap::new();

    let island_chord = prefs
        .system
        .get("islandSearch")
        .cloned()
        .unwrap_or_else(|| "Alt+Space".into());
    let dock_chord = prefs
        .system
        .get("dockToggle")
        .cloned()
        .unwrap_or_else(|| "Ctrl+Alt+D".into());

    dtos.push(HotkeyBindingDto {
        id: ID_ISLAND_SEARCH.into(),
        scope: "system".into(),
        plugin_id: None,
        plugin_name: None,
        key: "islandSearch".into(),
        label: "打开岛栏搜索".into(),
        action: ACTION_ISLAND_SEARCH.into(),
        chord: island_chord.clone(),
        aliases_system_search: false,
    });
    dtos.push(HotkeyBindingDto {
        id: ID_DOCK_TOGGLE.into(),
        scope: "system".into(),
        plugin_id: None,
        plugin_name: None,
        key: "dockToggle".into(),
        label: "切换 Dock 显隐".into(),
        action: ACTION_DOCK_TOGGLE.into(),
        chord: dock_chord.clone(),
        aliases_system_search: false,
    });

    let push_live =
        |live: &mut Vec<LiveBinding>,
         used: &mut HashMap<String, String>,
         base_ids: &mut HashMap<(u32, u32), i32>,
         next: &mut i32,
         bind_id: String,
         action: String,
         plugin_id: Option<String>,
         chord_raw: &str|
         -> Result<(), String> {
            if chord_raw.trim().is_empty() {
                return Ok(());
            }
            let parsed = parse_chord(chord_raw)?;
            if let Some(other) = used.get(&parsed.display) {
                return Err(format!("快捷键 {} 与 {} 冲突", parsed.display, other));
            }
            used.insert(parsed.display.clone(), bind_id.clone());
            let base = (parsed.mods, parsed.vk);
            let hid = *base_ids.entry(base).or_insert_with(|| {
                let id = *next;
                *next += 1;
                id
            });
            live.push(LiveBinding {
                id: bind_id,
                action,
                plugin_id,
                chord: parsed,
                hotkey_id: hid,
            });
            Ok(())
        };

    for (chord_raw, action, bind_id) in [
        (island_chord.as_str(), ACTION_ISLAND_SEARCH, ID_ISLAND_SEARCH),
        (dock_chord.as_str(), ACTION_DOCK_TOGGLE, ID_DOCK_TOGGLE),
    ] {
        push_live(
            &mut live,
            &mut used_chords,
            &mut base_hotkey_id,
            &mut next_id,
            bind_id.to_string(),
            action.to_string(),
            None,
            chord_raw,
        )?;
    }

    let plugins = crate::plugin_install::list_installed_plugins_sync();
    for p in plugins {
        if !p.enabled {
            continue;
        }
        let fields = p
            .manifest
            .get("settings")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let settings = crate::plugin_hub::hub_settings_get_all(p.id.clone())
            .unwrap_or_else(|_| serde_json::json!({}));
        for field in fields {
            let ty = field.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if ty != "hotkey" {
                continue;
            }
            let key = field
                .get("key")
                .and_then(|k| k.as_str())
                .unwrap_or("")
                .to_string();
            if key.is_empty() {
                continue;
            }
            let label = field
                .get("label")
                .and_then(|l| l.as_str())
                .unwrap_or(&key)
                .to_string();
            let action = field
                .get("action")
                .and_then(|a| a.as_str())
                .unwrap_or(&key)
                .to_string();
            let aliases = action == ACTION_ISLAND_SEARCH;
            let chord = if aliases {
                island_chord.clone()
            } else {
                settings
                    .get(&key)
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| {
                        field
                            .get("default")
                            .and_then(|d| d.as_str())
                            .map(|s| s.to_string())
                    })
                    .unwrap_or_default()
            };
            let id = format!("plugin.{}:{}", p.id, key);
            dtos.push(HotkeyBindingDto {
                id: id.clone(),
                scope: "plugin".into(),
                plugin_id: Some(p.id.clone()),
                plugin_name: Some(p.name.clone()),
                key: key.clone(),
                label,
                action: action.clone(),
                chord: chord.clone(),
                aliases_system_search: aliases,
            });
            if aliases {
                continue;
            }
            if chord.trim().is_empty() {
                continue;
            }
            if let Err(e) = push_live(
                &mut live,
                &mut used_chords,
                &mut base_hotkey_id,
                &mut next_id,
                id,
                action,
                Some(p.id.clone()),
                &chord,
            ) {
                eprintln!("[hotkey] skip {}.{}: {e}", p.id, key);
            }
        }
    }

    Ok((live, dtos))
}

fn unregister_all(ids: &[i32]) {
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        if !seen.insert(*id) {
            continue;
        }
        unsafe {
            let _ = UnregisterHotKey(None, *id);
        }
    }
}

fn register_live_set(live: &[LiveBinding]) {
    let mut seen = std::collections::HashSet::new();
    for b in live {
        if !seen.insert(b.hotkey_id) {
            continue;
        }
        // Multi-tap needs repeated WM_HOTKEY — never use MOD_NOREPEAT for that base.
        let needs_repeat = live
            .iter()
            .any(|x| x.hotkey_id == b.hotkey_id && x.chord.taps > 1);
        unsafe {
            let _ = UnregisterHotKey(None, b.hotkey_id);
            let ok = if needs_repeat {
                RegisterHotKey(
                    None,
                    b.hotkey_id,
                    HOT_KEY_MODIFIERS(b.chord.mods),
                    b.chord.vk,
                )
                .is_ok()
            } else {
                let mods_nr = HOT_KEY_MODIFIERS(b.chord.mods | MOD_NOREPEAT.0);
                RegisterHotKey(None, b.hotkey_id, mods_nr, b.chord.vk).is_ok()
                    || RegisterHotKey(
                        None,
                        b.hotkey_id,
                        HOT_KEY_MODIFIERS(b.chord.mods),
                        b.chord.vk,
                    )
                    .is_ok()
            };
            if ok {
                eprintln!(
                    "[hotkey] registered base id={} mods={} vk={} ({} binding(s))",
                    b.hotkey_id,
                    b.chord.mods,
                    b.chord.vk,
                    live.iter().filter(|x| x.hotkey_id == b.hotkey_id).count()
                );
            } else {
                eprintln!(
                    "[hotkey] RegisterHotKey failed for {} (id={})",
                    b.chord.display, b.hotkey_id
                );
            }
        }
    }
}

fn recording_shield_ids() -> [i32; 4] {
    [
        RECORDING_SHIELD_ALT_SPACE,
        RECORDING_SHIELD_CTRL_SPACE,
        RECORDING_SHIELD_SHIFT_SPACE,
        RECORDING_SHIELD_WIN_SPACE,
    ]
}

fn is_recording_shield(hid: i32) -> bool {
    recording_shield_ids().contains(&hid)
}

fn install_recording_shield() {
    unsafe {
        for id in recording_shield_ids() {
            let _ = UnregisterHotKey(None, id);
        }
        let _ = RegisterHotKey(
            None,
            RECORDING_SHIELD_ALT_SPACE,
            HOT_KEY_MODIFIERS(MOD_ALT.0),
            VK_SPACE,
        );
        let _ = RegisterHotKey(
            None,
            RECORDING_SHIELD_CTRL_SPACE,
            HOT_KEY_MODIFIERS(MOD_CONTROL.0),
            VK_SPACE,
        );
        let _ = RegisterHotKey(
            None,
            RECORDING_SHIELD_SHIFT_SPACE,
            HOT_KEY_MODIFIERS(MOD_SHIFT.0),
            VK_SPACE,
        );
        let _ = RegisterHotKey(
            None,
            RECORDING_SHIELD_WIN_SPACE,
            HOT_KEY_MODIFIERS(MOD_WIN.0),
            VK_SPACE,
        );
    }
    install_recording_ll_hook();
}

fn remove_recording_shield() {
    remove_recording_ll_hook();
    unsafe {
        for id in recording_shield_ids() {
            let _ = UnregisterHotKey(None, id);
        }
    }
}

fn read_mod_async() -> (bool, bool, bool, bool) {
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        let down = |vk: i32| GetAsyncKeyState(vk) as u16 & 0x8000 != 0;
        // Check left/right variants — VK_SHIFT alone can briefly read false on some IMEs.
        let ctrl = down(0x11) || down(0xA2) || down(0xA3);
        let alt = down(0x12) || down(0xA4) || down(0xA5);
        let shift = down(0x10) || down(0xA0) || down(0xA1);
        let win = down(0x5B) || down(0x5C);
        (ctrl, alt, shift, win)
    }
}

fn emit_space_tap(ctrl: bool, alt: bool, shift: bool, win: bool) {
    if !(ctrl || alt || shift || win) {
        return;
    }
    if let Ok(guard) = RECORDING_APP.lock() {
        if let Some(app) = guard.as_ref() {
            let _ = app.emit(
                "hotkey-record-key",
                serde_json::json!({
                    "key": "Space",
                    "down": true,
                    "alt": alt,
                    "ctrl": ctrl,
                    "shift": shift,
                    "win": win,
                }),
            );
        }
    }
}

fn emit_space_tap_from_shield(hid: i32) {
    let (mut ctrl, mut alt, mut shift, mut win) = read_mod_async();
    // RegisterHotKey id guarantees at least that modifier even if async lags.
    match hid {
        RECORDING_SHIELD_ALT_SPACE => alt = true,
        RECORDING_SHIELD_CTRL_SPACE => ctrl = true,
        RECORDING_SHIELD_SHIFT_SPACE => shift = true,
        RECORDING_SHIELD_WIN_SPACE => win = true,
        _ => {}
    }
    emit_space_tap(ctrl, alt, shift, win);
}

/// Intercept Space while any modifier is held. Never eat modifiers themselves.
unsafe extern "system" fn recording_ll_proc(
    code: i32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::LRESULT;
    if code < 0 || !RECORDING_ACTIVE.load(Ordering::SeqCst) {
        return CallNextHookEx(
            HHOOK(RECORDING_HOOK.load(Ordering::SeqCst) as _),
            code,
            wparam,
            lparam,
        );
    }
    let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
    let vk = info.vkCode;
    let wp = wparam.0 as u32;
    let is_down = wp == WM_KEYDOWN || wp == WM_SYSKEYDOWN;
    let injected = (info.flags.0 & 0x10) != 0;
    if injected || !is_down {
        return CallNextHookEx(
            HHOOK(RECORDING_HOOK.load(Ordering::SeqCst) as _),
            code,
            wparam,
            lparam,
        );
    }

    // Space + Ctrl/Alt/Shift/Win — Host owns these during recording.
    if vk == VK_SPACE {
        let (ctrl, alt_async, shift, win) = read_mod_async();
        let alt = (info.flags.0 & 0x20) != 0 || alt_async;
        if ctrl || alt || shift || win {
            emit_space_tap(ctrl, alt, shift, win);
            return LRESULT(1);
        }
    }

    CallNextHookEx(
        HHOOK(RECORDING_HOOK.load(Ordering::SeqCst) as _),
        code,
        wparam,
        lparam,
    )
}

fn install_recording_ll_hook() {
    if RECORDING_HOOK.load(Ordering::SeqCst) != 0 {
        RECORDING_ACTIVE.store(true, Ordering::SeqCst);
        return;
    }
    unsafe {
        match SetWindowsHookExW(WH_KEYBOARD_LL, Some(recording_ll_proc), None, 0) {
            Ok(h) => {
                RECORDING_HOOK.store(h.0 as isize, Ordering::SeqCst);
                RECORDING_ACTIVE.store(true, Ordering::SeqCst);
                eprintln!("[hotkey] recording Space+mod LL shield on");
            }
            Err(e) => {
                eprintln!("[hotkey] WH_KEYBOARD_LL failed: {e}");
                RECORDING_ACTIVE.store(true, Ordering::SeqCst);
            }
        }
    }
}

fn remove_recording_ll_hook() {
    RECORDING_ACTIVE.store(false, Ordering::SeqCst);
    let raw = RECORDING_HOOK.swap(0, Ordering::SeqCst);
    if raw != 0 {
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(raw as _));
        }
        eprintln!("[hotkey] recording Space+mod LL shield off");
    }
}

fn fire_binding(app: &AppHandle, b: &LiveBinding) {
    if b.action == ACTION_DOCK_TOGGLE {
        if let Some(vis) = app.try_state::<std::sync::Arc<crate::dock::DockVisibility>>() {
            vis.apply_hotkey_toggle(app);
        }
    }
    let payload = serde_json::json!({
        "id": b.id,
        "scope": if b.plugin_id.is_some() { "plugin" } else { "system" },
        "pluginId": b.plugin_id,
        "action": b.action,
        "chord": b.chord.display,
    });
    let _ = app.emit("hotkey-action", payload);
}

struct PendingTap {
    hotkey_id: i32,
    count: u8,
    deadline: std::time::Instant,
}

const TAP_WINDOW: Duration = Duration::from_millis(480);

fn worker_main(rx: mpsc::Receiver<Cmd>) {
    let tid = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
    WORKER_TID.store(tid as i32, Ordering::SeqCst);

    let mut live: Vec<LiveBinding> = Vec::new();
    let mut app_handle: Option<AppHandle> = None;
    let mut suspended = false;
    let mut pending: Option<PendingTap> = None;

    loop {
        match rx.recv_timeout(Duration::from_millis(30)) {
            Ok(Cmd::Shutdown) => break,
            Ok(Cmd::SuspendRecording) => {
                suspended = true;
                pending = None;
                let ids: Vec<i32> = live.iter().map(|b| b.hotkey_id).collect();
                unregister_all(&ids);
                if let Some(app) = &app_handle {
                    if let Ok(mut g) = RECORDING_APP.lock() {
                        *g = Some(app.clone());
                    }
                }
                install_recording_shield();
                eprintln!("[hotkey] suspended for recording (+ Space+mod shield)");
            }
            Ok(Cmd::ResumeRecording) => {
                remove_recording_shield();
                if let Ok(mut g) = RECORDING_APP.lock() {
                    *g = None;
                }
                suspended = false;
                pending = None;
                if app_handle.is_some() {
                    register_live_set(&live);
                }
                eprintln!("[hotkey] resumed after recording");
            }
            Ok(Cmd::Reload { app }) => {
                app_handle = Some(app);
                pending = None;
                let ids: Vec<i32> = live.iter().map(|b| b.hotkey_id).collect();
                unregister_all(&ids);
                live.clear();
                match collect_bindings() {
                    Ok((next, _)) => {
                        for b in &next {
                            eprintln!(
                                "[hotkey] binding {} → {} ({})",
                                b.chord.display, b.action, b.id
                            );
                        }
                        live = next;
                        if suspended {
                            install_recording_shield();
                        } else {
                            remove_recording_shield();
                            register_live_set(&live);
                        }
                    }
                    Err(e) => eprintln!("[hotkey] reload failed: {e}"),
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        if let Some(p) = &pending {
            if std::time::Instant::now() >= p.deadline {
                let hid = p.hotkey_id;
                pending = None;
                if !suspended {
                    if let Some(app) = &app_handle {
                        if let Some(b) = live
                            .iter()
                            .find(|x| x.hotkey_id == hid && x.chord.taps <= 1)
                        {
                            fire_binding(app, b);
                        }
                    }
                }
            }
        }

        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    RUNNING.store(false, Ordering::SeqCst);
                    break;
                }
                if msg.message == WM_HOTKEY {
                    let hid = msg.wParam.0 as i32;
                    let is_shield = is_recording_shield(hid);
                    if suspended || is_shield {
                        // Recording: Space chords stolen here — forward tap to UI.
                        if is_shield && suspended {
                            emit_space_tap_from_shield(hid);
                        }
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                        continue;
                    }
                    if let Some(app) = &app_handle {
                        let group: Vec<LiveBinding> = live
                            .iter()
                            .filter(|x| x.hotkey_id == hid)
                            .cloned()
                            .collect();
                        if !group.is_empty() {
                            let has_multi = group.iter().any(|b| b.chord.taps > 1);
                            let single = group.iter().find(|b| b.chord.taps <= 1).cloned();
                            let multi2 = group.iter().find(|b| b.chord.taps == 2).cloned();

                            if !has_multi {
                                if let Some(b) = single {
                                    fire_binding(app, &b);
                                }
                            } else if let Some(p) =
                                pending.as_mut().filter(|p| p.hotkey_id == hid)
                            {
                                p.count = p.count.saturating_add(1);
                                if p.count >= 2 {
                                    pending = None;
                                    if let Some(b) = multi2 {
                                        fire_binding(app, &b);
                                    }
                                }
                            } else {
                                pending = Some(PendingTap {
                                    hotkey_id: hid,
                                    count: 1,
                                    deadline: std::time::Instant::now() + TAP_WINDOW,
                                });
                            }
                        }
                    }
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if !RUNNING.load(Ordering::SeqCst) {
            break;
        }
    }

    remove_recording_shield();
    let ids: Vec<i32> = live.iter().map(|b| b.hotkey_id).collect();
    unregister_all(&ids);
    WORKER_TID.store(0, Ordering::SeqCst);
    RUNNING.store(false, Ordering::SeqCst);
}

fn ensure_worker() -> Sender<Cmd> {
    let mut guard = TX.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(tx) = guard.as_ref() {
        return tx.clone();
    }
    let (tx, rx) = mpsc::channel();
    *guard = Some(tx.clone());
    RUNNING.store(true, Ordering::SeqCst);
    thread::spawn(move || worker_main(rx));
    tx
}

fn send_cmd(cmd: Cmd) {
    let _ = ensure_worker().send(cmd);
    let tid = WORKER_TID.load(Ordering::SeqCst);
    if tid != 0 {
        unsafe {
            let _ = PostThreadMessageW(
                tid as u32,
                WM_NULL,
                windows::Win32::Foundation::WPARAM(0),
                windows::Win32::Foundation::LPARAM(0),
            );
        }
    }
}

fn send_reload(app: AppHandle) {
    send_cmd(Cmd::Reload { app });
}

/// Start registry and load bindings (call once from app setup).
pub fn spawn(app: AppHandle) {
    send_reload(app);
}

/// Re-scan system prefs + plugin hotkey settings.
pub fn reload(app: &AppHandle) {
    send_reload(app.clone());
}

/// Unregister all OS hotkeys while the settings recorder is active.
pub fn suspend_for_recording() {
    send_cmd(Cmd::SuspendRecording);
}

/// Re-register after recorder exits (Esc / commit / unmount).
pub fn resume_after_recording() {
    send_cmd(Cmd::ResumeRecording);
}

pub fn list_bindings() -> Result<Vec<HotkeyBindingDto>, String> {
    let (_, dtos) = collect_bindings()?;
    Ok(dtos)
}

pub fn set_binding(
    app: &AppHandle,
    id: &str,
    chord_raw: &str,
) -> Result<Vec<HotkeyBindingDto>, String> {
    let chord = normalize_chord(chord_raw)?;
    let (_, dtos) = collect_bindings()?;
    if !chord.is_empty() {
        for b in &dtos {
            if b.id == id || b.aliases_system_search {
                continue;
            }
            if !b.chord.is_empty() && b.chord == chord {
                return Err(format!("快捷键 {chord} 已被「{}」占用", b.label));
            }
        }
        let editing_aliases = dtos
            .iter()
            .find(|b| b.id == id)
            .map(|b| b.aliases_system_search)
            .unwrap_or(false);
        if !editing_aliases && id != ID_ISLAND_SEARCH {
            if let Some(sys) = dtos.iter().find(|b| b.id == ID_ISLAND_SEARCH) {
                if !sys.chord.is_empty() && sys.chord == chord {
                    return Err(format!("快捷键 {chord} 已被「{}」占用", sys.label));
                }
            }
        }
        if id != ID_DOCK_TOGGLE {
            if let Some(sys) = dtos.iter().find(|b| b.id == ID_DOCK_TOGGLE) {
                if !sys.chord.is_empty() && sys.chord == chord {
                    return Err(format!("快捷键 {chord} 已被「{}」占用", sys.label));
                }
            }
        }
    }

    if id == ID_ISLAND_SEARCH
        || dtos
            .iter()
            .any(|b| b.id == id && b.aliases_system_search)
    {
        let mut prefs = load_prefs();
        prefs.system.insert("islandSearch".into(), chord.clone());
        save_prefs(&prefs)?;
        for b in &dtos {
            if b.aliases_system_search {
                if let Some(pid) = &b.plugin_id {
                    let _ = crate::plugin_hub::write_setting_value(
                        pid,
                        &b.key,
                        serde_json::Value::String(chord.clone()),
                    );
                }
            }
        }
    } else if id == ID_DOCK_TOGGLE {
        let mut prefs = load_prefs();
        prefs.system.insert("dockToggle".into(), chord.clone());
        save_prefs(&prefs)?;
        let mut dock = crate::dock::load_dock_prefs();
        if !chord.is_empty() {
            dock.hotkey = chord.clone();
            let _ = crate::dock::save_dock_prefs(&dock);
        }
    } else if let Some(rest) = id.strip_prefix("plugin.") {
        let (plugin_id, key) = rest
            .split_once(':')
            .ok_or_else(|| format!("无效绑定 id: {id}"))?;
        crate::plugin_hub::assert_capability(plugin_id, "storage")?;
        let _ = crate::plugin_hub::hub_settings_set(
            app.clone(),
            plugin_id.to_string(),
            key.to_string(),
            serde_json::Value::String(chord.clone()),
        )?;
    } else {
        return Err(format!("未知快捷键: {id}"));
    }

    reload(app);
    let _ = app.emit("hotkeys-changed", true);
    list_bindings()
}

/// Validate a chord string without saving (for recorder UI).
pub fn validate_chord_available(id: &str, chord_raw: &str) -> Result<String, String> {
    let chord = normalize_chord(chord_raw)?;
    if chord.is_empty() {
        return Ok(chord);
    }
    let (_, dtos) = collect_bindings()?;
    let editing_aliases = dtos
        .iter()
        .find(|b| b.id == id)
        .map(|b| b.aliases_system_search)
        .unwrap_or(false);
    for b in &dtos {
        if b.id == id || b.aliases_system_search {
            continue;
        }
        if b.chord == chord {
            return Err(format!("快捷键 {chord} 已被「{}」占用", b.label));
        }
    }
    if !editing_aliases && id != ID_ISLAND_SEARCH {
        if let Some(sys) = dtos.iter().find(|b| b.id == ID_ISLAND_SEARCH) {
            if sys.chord == chord {
                return Err(format!("快捷键 {chord} 已被「{}」占用", sys.label));
            }
        }
    }
    if id != ID_DOCK_TOGGLE {
        if let Some(sys) = dtos.iter().find(|b| b.id == ID_DOCK_TOGGLE) {
            if sys.chord == chord {
                return Err(format!("快捷键 {chord} 已被「{}」占用", sys.label));
            }
        }
    }
    Ok(chord)
}
