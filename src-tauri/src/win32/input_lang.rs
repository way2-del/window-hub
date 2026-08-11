//! Foreground keyboard layout / IME indicator.
//!
//! Windows Input Indicator (“中” + IME mode) is Shell chrome, **not** a
//! `Shell_NotifyIcon`. When Hub hides `Shell_TrayWnd`, those tiles vanish and
//! never appear in the tray hook — so we host our own chips.
//!
//! Listing / switching uses TSF (`ITfInputProcessorProfileMgr`). Classic HKL
//! APIs alone collapse multiple Chinese IMEs (微信 / 微软拼音) into one layout
//! and often miss en-US when Preload only has `00000804`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct InputLangState {
    /// Short label for the language chip, e.g. "中" / "英" / "EN".
    pub lang_abbr: String,
    /// Longer language name for tooltip.
    pub lang_name: String,
    /// IME / layout name when available.
    pub ime_name: String,
    /// Whether an East-Asian IME context is open (typing CJK vs Latin).
    pub ime_open: bool,
    /// True when current layout is typically paired with an IME (zh/ja/ko).
    pub ime_capable: bool,
    /// Raw LANGID (low word of HKL).
    pub lang_id: u16,
    /// Current HKL as usize bits (for menu selection).
    #[serde(default)]
    pub hkl: u64,
    /// Active TSF profile type (1=IME, 2=keyboard layout).
    #[serde(default)]
    pub profile_type: u32,
    #[serde(default)]
    pub clsid: String,
    #[serde(default)]
    pub guid_profile: String,
}

/// One installed keyboard layout / IME for the self-drawn picker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InputLayoutItem {
    /// Stable id: `type:lang:clsid:profile` or `type:lang:hkl`.
    pub id: String,
    pub profile_type: u32,
    pub lang_id: u16,
    pub clsid: String,
    pub guid_profile: String,
    pub hkl: u64,
    pub lang_abbr: String,
    pub lang_name: String,
    pub ime_name: String,
    pub display_name: String,
    pub mark: String,
    pub active: bool,
}

#[cfg(windows)]
mod win {
    use super::{InputLangState, InputLayoutItem};
    use parking_lot::Mutex;
    use std::sync::{LazyLock, OnceLock};
    use std::time::Duration;
    use windows::core::{Interface, GUID, HSTRING, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::Ime::{
        ImmGetContext, ImmGetConversionStatus, ImmGetDefaultIMEWnd, ImmGetDescriptionW,
        ImmGetOpenStatus, ImmReleaseContext, ImmSetConversionStatus, ImmSetOpenStatus,
        IME_CONVERSION_MODE, IME_SENTENCE_MODE,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        ActivateKeyboardLayout, GetKeyboardLayout, GetKeyboardLayoutList, LoadKeyboardLayoutW,
        SendInput, HKL, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
        KLF_NOTELLSHELL, KLF_SETFORPROCESS, VIRTUAL_KEY, VK_LWIN, VK_SPACE,
    };
    use windows::Win32::UI::TextServices::{
        CLSID_TF_InputProcessorProfiles, ITfInputProcessorProfileMgr, ITfInputProcessorProfiles,
        GUID_TFCAT_TIP_KEYBOARD, TF_INPUTPROCESSORPROFILE, TF_IPPMF_FORSESSION,
        TF_IPP_FLAG_ACTIVE, TF_IPP_FLAG_ENABLED, TF_PROFILETYPE_INPUTPROCESSOR,
        TF_PROFILETYPE_KEYBOARDLAYOUT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId,
        IsWindow, PostMessageW, SendMessageTimeoutW, SendMessageW, SetForegroundWindow, ASFW_ANY,
        SMTO_ABORTIFHUNG,
    };

    static LAST: LazyLock<Mutex<InputLangState>> =
        LazyLock::new(|| Mutex::new(InputLangState::default()));
    /// Last non-Hub foreground hwnd — clicks steal focus to the menubar.
    static LAST_FG: LazyLock<Mutex<isize>> = LazyLock::new(|| Mutex::new(0));
    /// Last successfully activated HKL (optimistic UI when FG query lags).
    static LAST_HKL: LazyLock<Mutex<u64>> = LazyLock::new(|| Mutex::new(0));
    static LAST_PROFILE_ID: LazyLock<Mutex<String>> = LazyLock::new(|| Mutex::new(String::new()));
    /// Last Chinese IME profile id — restore target when leaving EN/英.
    static LAST_ZH_PROFILE_ID: LazyLock<Mutex<String>> = LazyLock::new(|| Mutex::new(String::new()));

    type EmitFn = Box<dyn Fn(InputLangState) + Send + Sync>;
    static EMIT: OnceLock<EmitFn> = OnceLock::new();

    const WM_INPUTLANGCHANGEREQUEST: u32 = 0x0050;
    const WM_IME_CONTROL: u32 = 0x0283;
    const IMC_GETCONVERSIONMODE: usize = 0x0001;
    const IMC_SETCONVERSIONMODE: usize = 0x0002;
    const IMC_GETOPENSTATUS: usize = 0x0005;
    const IMC_SETOPENSTATUS: usize = 0x0006;
    const IME_CMODE_NATIVE: u32 = 0x0001;

    struct ComGuard {
        uninit: bool,
    }
    impl ComGuard {
        fn enter() -> Self {
            let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            // S_OK → we own uninit; S_FALSE already init on this thread.
            let uninit = hr.is_ok() && hr.0 == 0;
            Self { uninit }
        }
    }
    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.uninit {
                unsafe { CoUninitialize() };
            }
        }
    }

    fn our_pid() -> u32 {
        unsafe { windows::Win32::System::Threading::GetCurrentProcessId() }
    }

    fn hwnd_pid(hwnd: HWND) -> u32 {
        unsafe {
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            pid
        }
    }

    fn is_our_window(hwnd: HWND) -> bool {
        if hwnd.0.is_null() || hwnd.is_invalid() {
            return true;
        }
        let pid = hwnd_pid(hwnd);
        pid != 0 && pid == our_pid()
    }

    /// Prefer the real app under the cursor/IME, not Hub after chip click.
    fn target_hwnd() -> HWND {
        unsafe {
            let fg = GetForegroundWindow();
            if !fg.is_invalid() && !is_our_window(fg) {
                *LAST_FG.lock() = fg.0 as isize;
                return fg;
            }
            let last = *LAST_FG.lock();
            if last != 0 {
                let h = HWND(last as *mut _);
                if IsWindow(h).as_bool() && !is_our_window(h) {
                    return h;
                }
            }
            fg
        }
    }

    fn focus_target(hwnd: HWND) {
        if hwnd.is_invalid() || is_our_window(hwnd) {
            return;
        }
        unsafe {
            let _ = AllowSetForegroundWindow(ASFW_ANY);
            let mut pid = 0u32;
            let target_tid = GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid != 0 {
                let _ = AllowSetForegroundWindow(pid);
            }
            let our_tid = GetCurrentThreadId();
            let attached = if target_tid != 0 && target_tid != our_tid {
                AttachThreadInput(our_tid, target_tid, true).as_bool()
            } else {
                false
            };
            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
            if attached {
                let _ = AttachThreadInput(our_tid, target_tid, false);
            }
        }
    }

    fn primary_lang_id(hkl: HKL) -> u16 {
        (hkl.0 as usize as u32 & 0xFFFF) as u16
    }

    fn lang_meta(lang_id: u16) -> (&'static str, &'static str, bool) {
        match lang_id {
            0x0804 | 0x0004 => ("中", "中文(简体)", true),
            0x0404 | 0x0C04 | 0x1404 | 0x1004 => ("繁", "中文(繁体)", true),
            0x0411 => ("あ", "日本語", true),
            0x0412 => ("가", "한국어", true),
            0x0409 => ("EN", "English (US)", false),
            0x0809 => ("EN", "English (UK)", false),
            0x0C09 => ("EN", "English (AU)", false),
            0x0407 => ("DE", "Deutsch", false),
            0x040C => ("FR", "Français", false),
            0x0410 => ("IT", "Italiano", false),
            0x0419 => ("RU", "Русский", false),
            0x041F => ("TR", "Türkçe", false),
            0x041E => ("TH", "ไทย", false),
            0x042A => ("VI", "Tiếng Việt", false),
            _ => ("IN", "Input language", false),
        }
    }

    fn fallback_abbr(lang_id: u16) -> String {
        let (abbr, _, _) = lang_meta(lang_id);
        if abbr != "IN" {
            return abbr.to_string();
        }
        if lang_id == 0 {
            return "中".to_string();
        }
        "Aa".to_string()
    }

    fn layout_mark(lang_abbr: &str, ime_name: &str) -> String {
        let name = ime_name.trim();
        if name.contains("搜狗") {
            return "搜".into();
        }
        if name.contains("微信") {
            return "P".into();
        }
        if name.contains("微软") || name.contains("拼音") {
            return "拼".into();
        }
        if !name.is_empty() {
            return name.chars().next().unwrap_or('拼').to_string();
        }
        if lang_abbr == "あ" {
            return "あ".into();
        }
        if lang_abbr == "가" {
            return "가".into();
        }
        if lang_abbr == "EN" || lang_abbr == "英" {
            return "A".into();
        }
        "拼".into()
    }

    fn hkl_bits(hkl: HKL) -> u64 {
        hkl.0 as usize as u64
    }

    fn hkl_from_bits(bits: u64) -> HKL {
        HKL(bits as usize as *mut _)
    }

    fn hkl_valid(hkl: HKL) -> bool {
        primary_lang_id(hkl) != 0
    }

    fn guid_str(g: &GUID) -> String {
        format!(
            "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
            g.data1,
            g.data2,
            g.data3,
            g.data4[0],
            g.data4[1],
            g.data4[2],
            g.data4[3],
            g.data4[4],
            g.data4[5],
            g.data4[6],
            g.data4[7]
        )
    }

    fn parse_guid(s: &str) -> Result<GUID, String> {
        let t = s
            .trim()
            .trim_start_matches('{')
            .trim_end_matches('}')
            .trim();
        if t.is_empty() || t == "00000000-0000-0000-0000-000000000000" {
            return Ok(GUID::default());
        }
        // windows::GUID::from(&str) panics unless length == 36 (no braces).
        if t.len() != 36 {
            return Err(format!("无效 GUID: {s}"));
        }
        // Avoid From<&str> (assert panic) — parse hex groups ourselves.
        let b = t.as_bytes();
        let hex = |i: usize, n: usize| -> Result<u64, String> {
            let slice = std::str::from_utf8(&b[i..i + n]).map_err(|_| format!("无效 GUID: {s}"))?;
            u64::from_str_radix(slice, 16).map_err(|_| format!("无效 GUID: {s}"))
        };
        if b[8] != b'-' || b[13] != b'-' || b[18] != b'-' || b[23] != b'-' {
            return Err(format!("无效 GUID: {s}"));
        }
        let data1 = hex(0, 8)? as u32;
        let data2 = hex(9, 4)? as u16;
        let data3 = hex(14, 4)? as u16;
        let d0 = hex(19, 2)? as u8;
        let d1 = hex(21, 2)? as u8;
        let d2 = hex(24, 2)? as u8;
        let d3 = hex(26, 2)? as u8;
        let d4 = hex(28, 2)? as u8;
        let d5 = hex(30, 2)? as u8;
        let d6 = hex(32, 2)? as u8;
        let d7 = hex(34, 2)? as u8;
        Ok(GUID::from_values(data1, data2, data3, [d0, d1, d2, d3, d4, d5, d6, d7]))
    }

    fn profile_id(profile_type: u32, lang_id: u16, clsid: &str, guid_profile: &str, hkl: u64) -> String {
        if profile_type == TF_PROFILETYPE_KEYBOARDLAYOUT || (clsid.is_empty() && guid_profile.is_empty())
        {
            format!("kb:{lang_id:04x}:{hkl:x}")
        } else {
            format!("ime:{lang_id:04x}:{clsid}:{guid_profile}")
        }
    }

    fn tip_profiles() -> Result<ITfInputProcessorProfileMgr, String> {
        let profiles: ITfInputProcessorProfiles = unsafe {
            CoCreateInstance(
                &CLSID_TF_InputProcessorProfiles,
                None,
                CLSCTX_INPROC_SERVER,
            )
        }
        .map_err(|e| format!("TSF profiles: {e}"))?;
        profiles
            .cast::<ITfInputProcessorProfileMgr>()
            .map_err(|e| format!("TSF profile mgr: {e}"))
    }

    fn enum_enabled_profiles() -> Vec<TF_INPUTPROCESSORPROFILE> {
        let _com = ComGuard::enter();
        let Ok(mgr) = tip_profiles() else {
            return Vec::new();
        };
        let Ok(enumerator) = (unsafe { mgr.EnumProfiles(0) }) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        loop {
            let mut buf = [TF_INPUTPROCESSORPROFILE::default(); 8];
            let mut fetched = 0u32;
            let hr = unsafe { enumerator.Next(&mut buf, &mut fetched) };
            if fetched == 0 {
                break;
            }
            for p in buf.iter().take(fetched as usize) {
                if p.dwFlags & TF_IPP_FLAG_ENABLED == 0 {
                    continue;
                }
                // Keyboard layouts + keyboard TIPs only (skip speech etc.).
                if p.dwProfileType == TF_PROFILETYPE_INPUTPROCESSOR
                    && p.catid != GUID_TFCAT_TIP_KEYBOARD
                {
                    continue;
                }
                out.push(*p);
            }
            if hr.is_err() {
                break;
            }
        }
        out
    }

    fn active_tsf_profile() -> Option<TF_INPUTPROCESSORPROFILE> {
        let _com = ComGuard::enter();
        let mgr = tip_profiles().ok()?;
        let mut p = TF_INPUTPROCESSORPROFILE::default();
        unsafe { mgr.GetActiveProfile(&GUID_TFCAT_TIP_KEYBOARD, &mut p) }.ok()?;
        if p.langid == 0 && p.dwProfileType == 0 {
            return None;
        }
        Some(p)
    }

    fn profile_description(p: &TF_INPUTPROCESSORPROFILE) -> String {
        if p.dwProfileType == TF_PROFILETYPE_KEYBOARDLAYOUT {
            let (abbr, name, _) = lang_meta(p.langid);
            if abbr == "IN" {
                return format!("Language 0x{:04X}", p.langid);
            }
            return name.to_string();
        }
        let _com = ComGuard::enter();
        if let Ok(profiles) = (|| -> Result<ITfInputProcessorProfiles, String> {
            unsafe {
                CoCreateInstance(
                    &CLSID_TF_InputProcessorProfiles,
                    None,
                    CLSCTX_INPROC_SERVER,
                )
            }
            .map_err(|e| e.to_string())
        })() {
            if let Ok(desc) = unsafe {
                profiles.GetLanguageProfileDescription(&p.clsid, p.langid, &p.guidProfile)
            } {
                let s = desc.to_string();
                if !s.trim().is_empty() {
                    return s;
                }
            }
        }
        let ime = ime_description(if hkl_valid(p.hkl) {
            p.hkl
        } else {
            p.hklSubstitute
        });
        if !ime.is_empty() {
            return ime;
        }
        let (_, name, _) = lang_meta(p.langid);
        name.to_string()
    }

    fn item_from_profile(p: &TF_INPUTPROCESSORPROFILE, active_id: &str) -> InputLayoutItem {
        let (abbr, name, _) = lang_meta(p.langid);
        let lang_abbr = if abbr == "IN" {
            fallback_abbr(p.langid)
        } else {
            abbr.to_string()
        };
        let lang_name = if name == "Input language" {
            format!("Language 0x{:04X}", p.langid)
        } else {
            name.to_string()
        };
        let ime_name = profile_description(p);
        let display_name = if !ime_name.is_empty() {
            ime_name.clone()
        } else {
            lang_name.clone()
        };
        let mark = layout_mark(&lang_abbr, &ime_name);
        let clsid = if p.dwProfileType == TF_PROFILETYPE_INPUTPROCESSOR {
            guid_str(&p.clsid)
        } else {
            String::new()
        };
        let guid_profile = if p.dwProfileType == TF_PROFILETYPE_INPUTPROCESSOR {
            guid_str(&p.guidProfile)
        } else {
            String::new()
        };
        let hkl = if hkl_valid(p.hkl) {
            hkl_bits(p.hkl)
        } else if hkl_valid(p.hklSubstitute) {
            hkl_bits(p.hklSubstitute)
        } else {
            0
        };
        let id = profile_id(p.dwProfileType, p.langid, &clsid, &guid_profile, hkl);
        let preferred = LAST_PROFILE_ID.lock().clone();
        let active = (p.dwFlags & TF_IPP_FLAG_ACTIVE) != 0
            || (!active_id.is_empty() && id == active_id)
            || (!preferred.is_empty() && id == preferred);
        InputLayoutItem {
            id,
            profile_type: p.dwProfileType,
            lang_id: p.langid,
            clsid,
            guid_profile,
            hkl,
            lang_abbr,
            lang_name,
            ime_name,
            display_name,
            mark,
            active,
        }
    }

    fn fallback_hkl_items(active_hkl: HKL) -> Vec<InputLayoutItem> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for h in loaded_hkls() {
            if !hkl_valid(h) {
                continue;
            }
            let bits = hkl_bits(h);
            if !seen.insert(bits) {
                continue;
            }
            let lang_id = primary_lang_id(h);
            let (abbr, name, _) = lang_meta(lang_id);
            let lang_abbr = if abbr == "IN" {
                fallback_abbr(lang_id)
            } else {
                abbr.to_string()
            };
            let lang_name = name.to_string();
            let ime_name = ime_description(h);
            let display_name = if ime_name.is_empty() {
                lang_name.clone()
            } else {
                ime_name.clone()
            };
            let id = profile_id(TF_PROFILETYPE_KEYBOARDLAYOUT, lang_id, "", "", bits);
            out.push(InputLayoutItem {
                id,
                profile_type: TF_PROFILETYPE_KEYBOARDLAYOUT,
                lang_id,
                clsid: String::new(),
                guid_profile: String::new(),
                hkl: bits,
                lang_abbr: lang_abbr.clone(),
                lang_name,
                ime_name: ime_name.clone(),
                display_name,
                mark: layout_mark(&lang_abbr, &ime_name),
                active: hkl_bits(h) == hkl_bits(active_hkl),
            });
        }
        out
    }

    fn foreground_hkl() -> HKL {
        unsafe {
            let hwnd = target_hwnd();
            let mut tid = 0u32;
            if !hwnd.is_invalid() {
                GetWindowThreadProcessId(hwnd, Some(&mut tid));
            }
            let hkl = GetKeyboardLayout(tid);
            if hkl_valid(hkl) {
                return hkl;
            }
            let preferred = *LAST_HKL.lock();
            if preferred != 0 {
                return hkl_from_bits(preferred);
            }
            let sys = GetKeyboardLayout(0);
            if hkl_valid(sys) {
                return sys;
            }
            let live = loaded_hkls();
            if let Some(first) = live.first() {
                return *first;
            }
            hkl
        }
    }

    fn ime_description(hkl: HKL) -> String {
        unsafe {
            let mut buf = [0u16; 128];
            let n = ImmGetDescriptionW(hkl, Some(&mut buf));
            if n == 0 {
                return String::new();
            }
            let end = (n as usize).min(buf.len());
            String::from_utf16_lossy(&buf[..end])
                .trim()
                .trim_end_matches('\0')
                .to_string()
        }
    }

    fn ime_open_for(hwnd: HWND) -> bool {
        unsafe {
            if hwnd.is_invalid() {
                return false;
            }
            let himc = ImmGetContext(hwnd);
            if himc.0.is_null() {
                return false;
            }
            let open = ImmGetOpenStatus(himc).as_bool();
            let _ = ImmReleaseContext(hwnd, himc);
            open
        }
    }

    fn loaded_hkls() -> Vec<HKL> {
        unsafe {
            let count = GetKeyboardLayoutList(None);
            if count <= 0 {
                return Vec::new();
            }
            let mut buf = vec![HKL::default(); count as usize];
            let filled = GetKeyboardLayoutList(Some(&mut buf));
            if filled <= 0 {
                return Vec::new();
            }
            buf.truncate(filled as usize);
            buf
        }
    }

    fn emit_now(state: &InputLangState) {
        if let Some(emit) = EMIT.get() {
            emit(state.clone());
        }
    }

    fn store_and_emit(state: InputLangState) -> InputLangState {
        if state.lang_id != 0 && state.lang_abbr != "0000" {
            *LAST.lock() = state.clone();
        }
        if state.hkl != 0 {
            *LAST_HKL.lock() = state.hkl;
        }
        let id = profile_id(
            state.profile_type,
            state.lang_id,
            &state.clsid,
            &state.guid_profile,
            state.hkl,
        );
        if !state.clsid.is_empty() || state.hkl != 0 {
            *LAST_PROFILE_ID.lock() = id;
        }
        emit_now(&state);
        state
    }

    fn is_cjk_lang(lang_id: u16) -> bool {
        matches!(
            lang_id,
            0x0804 | 0x0004 | 0x0404 | 0x0C04 | 0x1404 | 0x1004 | 0x0411 | 0x0412
        )
    }

    fn is_zh_lang(lang_id: u16) -> bool {
        matches!(
            lang_id,
            0x0804 | 0x0004 | 0x0404 | 0x0C04 | 0x1404 | 0x1004
        )
    }

    /// Query 中/英 via the window's default IME hwnd (works cross-process for many IMEs).
    /// Does not Activate ITfThreadMgr — safe for the IME's Shift hotkey.
    fn query_ime_zh_via_ime_wnd(hwnd: HWND) -> Option<(bool /*open*/, bool /*native*/)> {
        unsafe {
            if hwnd.is_invalid() {
                return None;
            }
            let ime = ImmGetDefaultIMEWnd(hwnd);
            if ime.is_invalid() || ime.0.is_null() {
                return None;
            }
            let open = SendMessageW(ime, WM_IME_CONTROL, WPARAM(IMC_GETOPENSTATUS), LPARAM(0)).0
                != 0;
            let conv =
                SendMessageW(ime, WM_IME_CONTROL, WPARAM(IMC_GETCONVERSIONMODE), LPARAM(0)).0
                    as u32;
            let native = conv & IME_CMODE_NATIVE != 0;
            // Some IMEs leave conversion at 0; then open-status alone is the 中/英 bit.
            let zh = if conv != 0 { open && native } else { open };
            Some((open, zh))
        }
    }

    fn query_ime_zh_via_imm_attach(hwnd: HWND) -> Option<(bool, bool)> {
        unsafe {
            if hwnd.is_invalid() {
                return None;
            }
            let mut pid = 0u32;
            let target_tid = GetWindowThreadProcessId(hwnd, Some(&mut pid));
            let our_tid = GetCurrentThreadId();
            let attached = if target_tid != 0 && target_tid != our_tid {
                AttachThreadInput(our_tid, target_tid, true).as_bool()
            } else {
                false
            };
            let himc = ImmGetContext(hwnd);
            let result = if himc.0.is_null() {
                None
            } else {
                let open = ImmGetOpenStatus(himc).as_bool();
                let mut conv = IME_CONVERSION_MODE(0);
                let mut sentence = IME_SENTENCE_MODE(0);
            let native = if ImmGetConversionStatus(himc, Some(&mut conv), Some(&mut sentence))
                    .as_bool()
                {
                    Some(conv.0 & IME_CMODE_NATIVE != 0)
                } else {
                    None
                };
                let _ = ImmReleaseContext(hwnd, himc);
                // (open, is_chinese)
                match native {
                    Some(n) => Some((open, open && n)),
                    None => Some((open, open)),
                }
            };
            if attached {
                let _ = AttachThreadInput(our_tid, target_tid, false);
            }
            result
        }
    }

    fn set_ime_zh_via_ime_wnd(hwnd: HWND, want_chinese: bool) -> bool {
        unsafe {
            let ime = ImmGetDefaultIMEWnd(hwnd);
            if ime.is_invalid() || ime.0.is_null() {
                return false;
            }
            let open = if want_chinese { 1isize } else { 0isize };
            let _ = SendMessageW(ime, WM_IME_CONTROL, WPARAM(IMC_SETOPENSTATUS), LPARAM(open));
            let mode = if want_chinese {
                IME_CMODE_NATIVE as isize
            } else {
                0
            };
            let _ = SendMessageW(ime, WM_IME_CONTROL, WPARAM(IMC_SETCONVERSIONMODE), LPARAM(mode));
            true
        }
    }

    fn set_ime_zh_via_imm_attach(hwnd: HWND, want_chinese: bool) -> bool {
        unsafe {
            let mut pid = 0u32;
            let target_tid = GetWindowThreadProcessId(hwnd, Some(&mut pid));
            let our_tid = GetCurrentThreadId();
            let attached = if target_tid != 0 && target_tid != our_tid {
                AttachThreadInput(our_tid, target_tid, true).as_bool()
            } else {
                false
            };
            let mut ok = false;
            let himc = ImmGetContext(hwnd);
            if !himc.0.is_null() {
                if ImmSetOpenStatus(himc, want_chinese).as_bool() {
                    ok = true;
                }
                let mut conv = IME_CONVERSION_MODE(0);
                let mut sentence = IME_SENTENCE_MODE(0);
                if ImmGetConversionStatus(himc, Some(&mut conv), Some(&mut sentence)).as_bool() {
                    let next = if want_chinese {
                        IME_CONVERSION_MODE(conv.0 | IME_CMODE_NATIVE)
                    } else {
                        IME_CONVERSION_MODE(conv.0 & !IME_CMODE_NATIVE)
                    };
                    if ImmSetConversionStatus(himc, next, sentence).as_bool() {
                        ok = true;
                    }
                }
                let _ = ImmReleaseContext(hwnd, himc);
            }
            if attached {
                let _ = AttachThreadInput(our_tid, target_tid, false);
            }
            ok
        }
    }

    /// Resolve 中/英 for a Chinese language profile (read-only, no TSF ThreadMgr).
    fn resolve_zh_en_abbr(hwnd: HWND) -> (String, bool) {
        if let Some((open, is_zh)) = query_ime_zh_via_ime_wnd(hwnd)
            .or_else(|| query_ime_zh_via_imm_attach(hwnd))
        {
            if is_zh {
                return ("中".into(), true);
            }
            return ("英".into(), open);
        }
        // Query failed — keep last 中/英 while still on a Chinese IME (don't snap to 中).
        let last = LAST.lock().clone();
        if is_zh_lang(last.lang_id) && matches!(last.lang_abbr.as_str(), "中" | "英") {
            return (last.lang_abbr.clone(), last.ime_open || last.lang_abbr == "中");
        }
        ("中".into(), true)
    }

    fn query_zh_en_mode(hwnd: HWND) -> (bool /*zh*/, bool /*open*/) {
        let (abbr, open) = resolve_zh_en_abbr(hwnd);
        (abbr == "中", open)
    }

    fn remember_zh_profile(state: &InputLangState) {
        if is_zh_lang(state.lang_id) && state.profile_type == TF_PROFILETYPE_INPUTPROCESSOR {
            let id = profile_id(
                state.profile_type,
                state.lang_id,
                &state.clsid,
                &state.guid_profile,
                state.hkl,
            );
            if !id.is_empty() {
                *LAST_ZH_PROFILE_ID.lock() = id;
            }
        }
    }

    fn state_from_profile(p: &TF_INPUTPROCESSORPROFILE) -> InputLangState {
        let item = item_from_profile(p, "");
        let hwnd = target_hwnd();
        let mut lang_abbr = item.lang_abbr.clone();
        let ime_capable = lang_meta(p.langid).2 || p.dwProfileType == TF_PROFILETYPE_INPUTPROCESSOR;
        let mut ime_open = false;

        if p.dwProfileType == TF_PROFILETYPE_KEYBOARDLAYOUT && !is_cjk_lang(p.langid) {
            // Real English (or other) keyboard — show EN/DE/… from lang_meta, not 英.
            ime_open = false;
        } else if is_zh_lang(p.langid) {
            let (abbr, open) = resolve_zh_en_abbr(hwnd);
            lang_abbr = abbr;
            ime_open = open;
        } else if ime_capable {
            ime_open = ime_open_for(hwnd);
        }

        let state = InputLangState {
            lang_abbr,
            lang_name: item.lang_name,
            ime_name: item.ime_name,
            ime_open,
            ime_capable: ime_capable && is_cjk_lang(p.langid),
            lang_id: p.langid,
            hkl: item.hkl,
            profile_type: p.dwProfileType,
            clsid: item.clsid,
            guid_profile: item.guid_profile,
        };
        remember_zh_profile(&state);
        state
    }

    /// Activate HKL for the target app thread (not just our process).
    fn activate_hkl_on_target(hwnd: HWND, hkl: HKL) -> Result<(), String> {
        if !hkl_valid(hkl) {
            return Err("无效的键盘布局".into());
        }
        unsafe {
            let mut pid = 0u32;
            let target_tid = GetWindowThreadProcessId(hwnd, Some(&mut pid));
            let our_tid = GetCurrentThreadId();
            let attached = if target_tid != 0 && target_tid != our_tid {
                AttachThreadInput(our_tid, target_tid, true).as_bool()
            } else {
                false
            };

            let _ = ActivateKeyboardLayout(hkl, KLF_SETFORPROCESS);

            let mut result = 0usize;
            let _ = SendMessageTimeoutW(
                hwnd,
                WM_INPUTLANGCHANGEREQUEST,
                WPARAM(0),
                LPARAM(hkl.0 as isize),
                SMTO_ABORTIFHUNG,
                200,
                Some(&mut result),
            );
            let _ = PostMessageW(
                hwnd,
                WM_INPUTLANGCHANGEREQUEST,
                WPARAM(0),
                LPARAM(hkl.0 as isize),
            );

            if attached {
                let _ = AttachThreadInput(our_tid, target_tid, false);
            }
        }
        *LAST_HKL.lock() = hkl_bits(hkl);
        Ok(())
    }

    fn activate_tsf_profile(
        profile_type: u32,
        lang_id: u16,
        clsid: &GUID,
        guid_profile: &GUID,
        hkl: HKL,
    ) -> Result<(), String> {
        let _com = ComGuard::enter();
        let mgr = tip_profiles()?;
        let hwnd = target_hwnd();
        if !hwnd.is_invalid() {
            focus_target(hwnd);
        }
        unsafe {
            mgr.ActivateProfile(
                profile_type,
                lang_id,
                clsid,
                guid_profile,
                hkl,
                TF_IPPMF_FORSESSION,
            )
        }
        .map_err(|e| format!("切换输入法失败: {e}"))?;

        // Reinforce classic layout change on the target window when HKL known.
        if !hwnd.is_invalid() && hkl_valid(hkl) {
            let _ = activate_hkl_on_target(hwnd, hkl);
        } else if !hwnd.is_invalid() && profile_type == TF_PROFILETYPE_KEYBOARDLAYOUT {
            let klid = format!("{lang_id:08X}");
            if let Ok(loaded) = (|| {
                let hs = HSTRING::from(klid.as_str());
                unsafe { LoadKeyboardLayoutW(PCWSTR(hs.as_ptr()), KLF_NOTELLSHELL) }
            })() {
                let _ = activate_hkl_on_target(hwnd, loaded);
            }
        }
        Ok(())
    }

    fn activate_layout_item(item: &InputLayoutItem) -> Result<(), String> {
        activate_tsf_profile(
            item.profile_type,
            item.lang_id,
            &parse_guid(&item.clsid).unwrap_or_default(),
            &parse_guid(&item.guid_profile).unwrap_or_default(),
            if item.hkl != 0 {
                hkl_from_bits(item.hkl)
            } else {
                HKL::default()
            },
        )?;
        *LAST_PROFILE_ID.lock() = item.id.clone();
        if is_zh_lang(item.lang_id) && item.profile_type == TF_PROFILETYPE_INPUTPROCESSOR {
            *LAST_ZH_PROFILE_ID.lock() = item.id.clone();
        }
        Ok(())
    }

    /// Simulate the IME's Shift 中/英 hotkey (once) toward the focused app.
    fn send_ime_shift_hotkey() -> Result<(), String> {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            MapVirtualKeyW, KEYEVENTF_SCANCODE, MAPVK_VK_TO_VSC, VK_LSHIFT,
        };
        unsafe {
            let scan = MapVirtualKeyW(VK_LSHIFT.0 as u32, MAPVK_VK_TO_VSC) as u16;
            let scan = if scan == 0 { 0x2A } else { scan };
            let down = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: scan,
                        dwFlags: KEYEVENTF_SCANCODE,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            let up = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: scan,
                        dwFlags: KEYEVENTF_SCANCODE | KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            if SendInput(&[down], std::mem::size_of::<INPUT>() as i32) == 0 {
                return Err("SendInput Shift down 失败".into());
            }
            std::thread::sleep(Duration::from_millis(40));
            if SendInput(&[up], std::mem::size_of::<INPUT>() as i32) == 0 {
                return Err("SendInput Shift up 失败".into());
            }
        }
        Ok(())
    }

    #[allow(dead_code)]
    fn find_english_layout() -> Option<InputLayoutItem> {
        let list = list_layouts();
        list.iter()
            .find(|l| {
                l.profile_type == TF_PROFILETYPE_KEYBOARDLAYOUT
                    && matches!(l.lang_id, 0x0409 | 0x0809 | 0x0C09)
            })
            .cloned()
            .or_else(|| {
                list.into_iter().find(|l| {
                    matches!(l.lang_id, 0x0409 | 0x0809 | 0x0C09) || l.lang_abbr == "EN"
                })
            })
    }

    fn find_chinese_ime() -> Option<InputLayoutItem> {
        let preferred = LAST_ZH_PROFILE_ID.lock().clone();
        let list = list_layouts();
        if let Some(hit) = list.iter().find(|l| !preferred.is_empty() && l.id == preferred) {
            return Some(hit.clone());
        }
        list.iter()
            .find(|l| {
                is_zh_lang(l.lang_id) && l.profile_type == TF_PROFILETYPE_INPUTPROCESSOR
            })
            .cloned()
            .or_else(|| list.into_iter().find(|l| is_zh_lang(l.lang_id)))
    }

    fn send_win_space() -> Result<(), String> {
        unsafe {
            let inputs = [
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_LWIN,
                            wScan: 0,
                            dwFlags: Default::default(),
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(VK_SPACE.0),
                            wScan: 0,
                            dwFlags: Default::default(),
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(VK_SPACE.0),
                            wScan: 0,
                            dwFlags: KEYEVENTF_KEYUP,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_LWIN,
                            wScan: 0,
                            dwFlags: KEYEVENTF_KEYUP,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
            ];
            let sent = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
            if sent == 0 {
                return Err("SendInput Win+Space 失败".into());
            }
        }
        Ok(())
    }

    pub fn snapshot() -> InputLangState {
        if let Some(p) = active_tsf_profile() {
            return state_from_profile(&p);
        }

        let hkl = foreground_hkl();
        let lang_id = primary_lang_id(hkl);
        if lang_id == 0 {
            let last = LAST.lock().clone();
            if !last.lang_abbr.is_empty()
                && last.lang_abbr != "0000"
                && !(last.lang_abbr.len() == 4
                    && last.lang_abbr.chars().all(|c| c.is_ascii_hexdigit()))
            {
                return last;
            }
            return InputLangState {
                lang_abbr: "中".into(),
                lang_name: "中文(简体)".into(),
                ime_name: String::new(),
                ime_open: true,
                ime_capable: true,
                lang_id: 0x0804,
                hkl: 0,
                profile_type: 0,
                clsid: String::new(),
                guid_profile: String::new(),
            };
        }
        let (abbr, name, ime_capable_meta) = lang_meta(lang_id);
        let mut lang_abbr = if abbr == "IN" {
            fallback_abbr(lang_id)
        } else {
            abbr.to_string()
        };
        let lang_name = if name == "Input language" {
            format!("Language 0x{lang_id:04X}")
        } else {
            name.to_string()
        };
        let ime_name = ime_description(hkl);
        let ime_capable = ime_capable_meta || !ime_name.is_empty();
        let hwnd = target_hwnd();
        let mut ime_open = false;
        if is_zh_lang(lang_id) {
            let (abbr, open) = resolve_zh_en_abbr(hwnd);
            lang_abbr = abbr;
            ime_open = open;
        } else if ime_capable {
            ime_open = ime_open_for(hwnd);
        }
        let state = InputLangState {
            lang_abbr,
            lang_name,
            ime_name,
            ime_open,
            ime_capable: ime_capable && is_cjk_lang(lang_id),
            lang_id,
            hkl: hkl_bits(hkl),
            profile_type: if ime_capable && is_cjk_lang(lang_id) {
                TF_PROFILETYPE_INPUTPROCESSOR
            } else {
                TF_PROFILETYPE_KEYBOARDLAYOUT
            },
            clsid: String::new(),
            guid_profile: String::new(),
        };
        remember_zh_profile(&state);
        state
    }

    pub fn get() -> InputLangState {
        let cur = snapshot();
        if cur.lang_id != 0 && cur.lang_abbr != "0000" {
            *LAST.lock() = cur.clone();
        }
        if cur.hkl != 0 {
            *LAST_HKL.lock() = cur.hkl;
        }
        remember_zh_profile(&cur);
        cur
    }

    pub fn list_layouts() -> Vec<InputLayoutItem> {
        let profiles = enum_enabled_profiles();
        if profiles.is_empty() {
            return fallback_hkl_items(foreground_hkl());
        }
        let active = active_tsf_profile()
            .map(|p| item_from_profile(&p, "").id)
            .unwrap_or_default();
        let mut items: Vec<_> = profiles
            .iter()
            .map(|p| item_from_profile(p, &active))
            .collect();
        // Deduplicate by id while preferring active.
        let mut seen = std::collections::HashSet::new();
        items.retain(|it| seen.insert(it.id.clone()));
        if items.is_empty() {
            return fallback_hkl_items(foreground_hkl());
        }
        items
    }

    pub fn select_layout(
        profile_type: u32,
        lang_id: u16,
        clsid: Option<String>,
        guid_profile: Option<String>,
        hkl_bits_v: u64,
    ) -> Result<InputLangState, String> {
        let hwnd = target_hwnd();
        if hwnd.is_invalid() {
            return Err("没有前台窗口，请先点一下要输入的窗口".into());
        }

        let clsid_s = clsid.unwrap_or_default();
        let guid_s = guid_profile.unwrap_or_default();
        let ptype = if profile_type == 0 {
            if clsid_s.is_empty() {
                TF_PROFILETYPE_KEYBOARDLAYOUT
            } else {
                TF_PROFILETYPE_INPUTPROCESSOR
            }
        } else {
            profile_type
        };
        let clsid_g = parse_guid(&clsid_s)?;
        let profile_g = parse_guid(&guid_s)?;
        let hkl = if hkl_bits_v != 0 {
            hkl_from_bits(hkl_bits_v)
        } else {
            HKL::default()
        };

        activate_tsf_profile(ptype, lang_id, &clsid_g, &profile_g, hkl)?;
        *LAST_PROFILE_ID.lock() = profile_id(ptype, lang_id, &clsid_s, &guid_s, hkl_bits_v);
        let before_lang = LAST.lock().lang_id;

        std::thread::sleep(std::time::Duration::from_millis(150));
        let mut after = get();
        // Always refresh identity from the selection (IME name may lag).
        after.profile_type = ptype;
        after.clsid = clsid_s.clone();
        after.guid_profile = guid_s.clone();
        if hkl_bits_v != 0 {
            after.hkl = hkl_bits_v;
        }
        if lang_id != 0 {
            let (abbr, name, capable) = lang_meta(lang_id);
            after.lang_id = lang_id;
            after.lang_name = name.to_string();
            after.ime_capable = capable || ptype == TF_PROFILETYPE_INPUTPROCESSOR;
            if is_zh_lang(lang_id) && ptype == TF_PROFILETYPE_INPUTPROCESSOR {
                // Switching to a Chinese IME → chip「中」+ green mark.
                after.lang_abbr = "中".into();
                after.ime_open = true;
                after.ime_capable = true;
                let _ = set_ime_zh_via_ime_wnd(hwnd, true);
                let _ = set_ime_zh_via_imm_attach(hwnd, true);
            } else if !after.ime_capable {
                after.lang_abbr = if abbr == "IN" {
                    fallback_abbr(lang_id)
                } else {
                    abbr.to_string()
                };
                after.ime_open = false;
            } else if before_lang != lang_id || !matches!(after.lang_abbr.as_str(), "中" | "英") {
                after.lang_abbr = if abbr == "IN" {
                    fallback_abbr(lang_id)
                } else {
                    abbr.to_string()
                };
            }
        }
        if let Some(item) = list_layouts().into_iter().find(|it| {
            it.profile_type == ptype
                && it.lang_id == lang_id
                && (clsid_s.is_empty() || it.clsid.eq_ignore_ascii_case(&clsid_s))
                && (guid_s.is_empty() || it.guid_profile.eq_ignore_ascii_case(&guid_s))
        }) {
            if !item.ime_name.is_empty() {
                after.ime_name = item.ime_name;
            }
        }
        Ok(store_and_emit(after))
    }

    /// Cycle to next installed keyboard / IME (also used as hotkey path).
    pub fn cycle_layout() -> Result<InputLangState, String> {
        let layouts = list_layouts();
        if layouts.is_empty() {
            send_win_space()?;
            std::thread::sleep(std::time::Duration::from_millis(150));
            return Ok(store_and_emit(get()));
        }
        let preferred = LAST_PROFILE_ID.lock().clone();
        let mut idx = layouts.iter().position(|l| l.active).unwrap_or(0);
        if let Some(i) = layouts.iter().position(|l| l.id == preferred) {
            idx = i;
        }
        let next = &layouts[(idx + 1) % layouts.len()];
        select_layout(
            next.profile_type,
            next.lang_id,
            Some(next.clsid.clone()),
            Some(next.guid_profile.clone()),
            next.hkl,
        )
    }

    /// Toggle 中英文 on the current Chinese IME (same idea as keyboard Shift).
    /// If current layout is EN keyboard, switch back to the last Chinese IME.
    pub fn toggle_ime() -> Result<InputLangState, String> {
        let hwnd = target_hwnd();
        if hwnd.is_invalid() {
            return Err("没有前台窗口，请先点一下要输入的窗口".into());
        }

        let before = snapshot();
        focus_target(hwnd);
        std::thread::sleep(Duration::from_millis(30));
        focus_target(hwnd);

        // EN keyboard → restore Chinese IME (same as menu pick).
        if matches!(before.lang_id, 0x0409 | 0x0809 | 0x0C09)
            || (before.profile_type == TF_PROFILETYPE_KEYBOARDLAYOUT && !is_cjk_lang(before.lang_id))
        {
            let zh = find_chinese_ime().ok_or_else(|| "未找到已安装的中文输入法".to_string())?;
            activate_layout_item(&zh)?;
            std::thread::sleep(Duration::from_millis(150));
            return Ok(store_and_emit(get()));
        }

        // Chinese IME: go to the opposite of what we currently read.
        let (is_zh, _) = query_zh_en_mode(hwnd);
        let want_chinese = !is_zh;

        // 1) Directional set (no Shift yet — Shift would undo a successful set).
        let _ = set_ime_zh_via_ime_wnd(hwnd, want_chinese);
        let _ = set_ime_zh_via_imm_attach(hwnd, want_chinese);
        std::thread::sleep(Duration::from_millis(80));
        let (zh_mid, _) = query_zh_en_mode(hwnd);
        if zh_mid != want_chinese {
            // 2) IMC didn't stick — one Shift toggle (same as keyboard hotkey).
            let _ = send_ime_shift_hotkey();
            std::thread::sleep(Duration::from_millis(100));
        }

        let mut after = get();
        if is_zh_lang(after.lang_id) {
            let (zh_now, open_now) = query_zh_en_mode(hwnd);
            if zh_now == want_chinese {
                after.lang_abbr = if zh_now { "中".into() } else { "英".into() };
                after.ime_open = open_now || zh_now;
            } else {
                // Query still lagging — trust the requested direction so tray matches typing.
                after.lang_abbr = if want_chinese { "中".into() } else { "英".into() };
                after.ime_open = want_chinese;
            }
        }
        Ok(store_and_emit(after))
    }

    pub fn open_language_settings() -> Result<(), String> {
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        unsafe {
            let op = HSTRING::from("open");
            let file = HSTRING::from("ms-settings:regionlanguage");
            let ret = ShellExecuteW(
                HWND::default(),
                PCWSTR(op.as_ptr()),
                PCWSTR(file.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
            if (ret.0 as isize) <= 32 {
                return Err("无法打开语言设置".into());
            }
        }
        Ok(())
    }

    pub fn open_keyboard_settings() -> Result<(), String> {
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        unsafe {
            let op = HSTRING::from("open");
            let file = HSTRING::from("ms-settings:typing");
            let ret = ShellExecuteW(
                HWND::default(),
                PCWSTR(op.as_ptr()),
                PCWSTR(file.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
            if (ret.0 as isize) <= 32 {
                return open_language_settings();
            }
        }
        Ok(())
    }

    pub fn open_emoji_panel() -> Result<(), String> {
        unsafe {
            let inputs = [
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_LWIN,
                            wScan: 0,
                            dwFlags: Default::default(),
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(0xBE),
                            wScan: 0,
                            dwFlags: Default::default(),
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(0xBE),
                            wScan: 0,
                            dwFlags: KEYEVENTF_KEYUP,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_LWIN,
                            wScan: 0,
                            dwFlags: KEYEVENTF_KEYUP,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
            ];
            let sent = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
            if sent == 0 {
                return Err("无法打开表情面板".into());
            }
        }
        Ok(())
    }

    pub fn open_touch_keyboard() -> Result<(), String> {
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        unsafe {
            let op = HSTRING::from("open");
            for path in [
                r"C:\Program Files\Common Files\microsoft shared\ink\TabTip.exe",
                "osk.exe",
            ] {
                let file = HSTRING::from(path);
                let ret = ShellExecuteW(
                    HWND::default(),
                    PCWSTR(op.as_ptr()),
                    PCWSTR(file.as_ptr()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    SW_SHOWNORMAL,
                );
                if (ret.0 as isize) > 32 {
                    return Ok(());
                }
            }
        }
        Err("无法打开虚拟键盘".into())
    }

    pub fn start<F>(on_change: F)
    where
        F: Fn(InputLangState) + Send + Sync + 'static,
    {
        let _ = EMIT.set(Box::new(on_change));
        if let Some(emit) = EMIT.get() {
            emit(get());
        }
        std::thread::Builder::new()
            .name("input-lang".into())
            .spawn(|| {
                let _com = ComGuard::enter();
                let mut last = LAST.lock().clone();
                loop {
                    let _ = target_hwnd();
                    let cur = snapshot();
                    if cur != last {
                        last = cur.clone();
                        if cur.lang_id != 0 && cur.lang_abbr != "0000" {
                            *LAST.lock() = cur.clone();
                        }
                        if cur.hkl != 0 {
                            *LAST_HKL.lock() = cur.hkl;
                        }
                        remember_zh_profile(&cur);
                        if let Some(emit) = EMIT.get() {
                            emit(cur);
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(120));
                }
            })
            .expect("spawn input-lang");
    }

    // Silence unused import if CoTaskMemFree not needed with EnumProfiles path.
    #[allow(dead_code)]
    fn _keep_cotaskmemfree(ptr: *mut std::ffi::c_void) {
        unsafe { CoTaskMemFree(Some(ptr)) };
    }
}

#[cfg(windows)]
pub use win::{
    cycle_layout, get, list_layouts, open_emoji_panel, open_keyboard_settings,
    open_language_settings, open_touch_keyboard, select_layout, start, toggle_ime,
};

#[cfg(not(windows))]
pub fn get() -> InputLangState {
    InputLangState {
        lang_abbr: "EN".into(),
        lang_name: "English".into(),
        ..Default::default()
    }
}

#[cfg(not(windows))]
pub fn cycle_layout() -> Result<InputLangState, String> {
    Ok(get())
}

#[cfg(not(windows))]
pub fn toggle_ime() -> Result<InputLangState, String> {
    Ok(get())
}

#[cfg(not(windows))]
pub fn open_language_settings() -> Result<(), String> {
    Ok(())
}

#[cfg(not(windows))]
pub fn open_keyboard_settings() -> Result<(), String> {
    Ok(())
}

#[cfg(not(windows))]
pub fn open_emoji_panel() -> Result<(), String> {
    Ok(())
}

#[cfg(not(windows))]
pub fn open_touch_keyboard() -> Result<(), String> {
    Ok(())
}

#[cfg(not(windows))]
pub fn list_layouts() -> Vec<InputLayoutItem> {
    Vec::new()
}

#[cfg(not(windows))]
pub fn select_layout(
    _profile_type: u32,
    _lang_id: u16,
    _clsid: Option<String>,
    _guid_profile: Option<String>,
    _hkl: u64,
) -> Result<InputLangState, String> {
    Ok(get())
}

#[cfg(not(windows))]
pub fn start<F>(_on_change: F)
where
    F: Fn(InputLangState) + Send + Sync + 'static,
{
}
