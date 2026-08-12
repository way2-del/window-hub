//! Read now-playing / desktop lyric text from NetEase Cloud Music (cloudmusic).
//!
//! Strategy (stable → fragile):
//! 1. Desktop lyrics HWND (`DesktopLyrics`) via window text + UI Automation
//! 2. Main window title (`OrpheusBrowserHost`) → "歌名 - 歌手"
//! 3. Optional memory pointer chain for known client versions (desktop lyric buffer)

use serde::Serialize;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE, HWND, LPARAM, MAX_PATH};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::ProcessStatus::{
    EnumProcessModules, GetModuleBaseNameW, GetModuleInformation, MODULEINFO,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, TreeScope_Children, TreeScope_Subtree,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    IsWindowVisible,
};

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseNowPlaying {
    pub active: bool,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub lyric: Option<String>,
    pub source: Option<String>,
}

/// Version → (module RVA, then pointer offsets). Final offset 0 = string at resolved addr.
const VERSION_OFFSETS: &[(&str, &[usize])] = &[
    ("3.1.32", &[0x01DF44D0, 0x120, 0x8, 0x0]),
    ("3.1.30", &[0x01DF44D0, 0x120, 0x8, 0x0]),
    ("3.1.29", &[0x01DEB4D0, 0x120, 0x8, 0x0]),
    ("3.1.28", &[0x01DDF290, 0x120, 0x8, 0x0]),
];

struct MemCache {
    pid: u32,
    lyric_addr: usize,
}

static MEM_CACHE: Mutex<Option<MemCache>> = Mutex::new(None);

struct ComGuard;
impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

fn with_com<T>(f: impl FnOnce() -> T) -> T {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let _guard = ComGuard;
    f()
}

fn wide_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    OsString::from_wide(&buf[..len])
        .to_string_lossy()
        .trim()
        .to_string()
}

fn hwnd_class(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
    if n == 0 {
        return String::new();
    }
    wide_to_string(&buf[..n])
}

fn hwnd_title(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; (len as usize) + 1];
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) } as usize;
    if n == 0 {
        return String::new();
    }
    wide_to_string(&buf[..n])
}

fn parse_title_artist(raw: &str) -> (Option<String>, Option<String>) {
    let t = raw.trim();
    if t.is_empty() || t == "网易云音乐" || t.eq_ignore_ascii_case("cloudmusic") {
        return (None, None);
    }
    for sep in [" - ", " – ", " — ", "-", "–", "—"] {
        if let Some((a, b)) = t.split_once(sep) {
            let title = a.trim();
            let artist = b.trim();
            if !title.is_empty() {
                return (
                    Some(title.to_string()),
                    if artist.is_empty() {
                        None
                    } else {
                        Some(artist.to_string())
                    },
                );
            }
        }
    }
    (Some(t.to_string()), None)
}

struct EnumCtx {
    main: Option<HWND>,
    lyric: Option<HWND>,
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut EnumCtx);
    let class = hwnd_class(hwnd);
    if class == "OrpheusBrowserHost" {
        if ctx.main.is_none() {
            ctx.main = Some(hwnd);
        }
    } else if class == "DesktopLyrics" {
        ctx.lyric = Some(hwnd);
    } else if class.to_ascii_lowercase().contains("lyric") {
        let title = hwnd_title(hwnd);
        if title.contains("歌词") || title.to_ascii_lowercase().contains("lyric") {
            if ctx.lyric.is_none() {
                ctx.lyric = Some(hwnd);
            }
        }
    }
    BOOL(1)
}

fn find_netease_hwnds() -> (Option<HWND>, Option<HWND>) {
    let mut ctx = EnumCtx {
        main: None,
        lyric: None,
    };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut _ as isize));
    }
    (ctx.main, ctx.lyric)
}

fn create_automation() -> Result<IUIAutomation, String> {
    unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string()) }
}

fn uia_name(el: &IUIAutomationElement) -> Option<String> {
    unsafe {
        let s = el.CurrentName().ok()?.to_string();
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    }
}

fn collect_uia_names(root: &IUIAutomationElement, subtree: bool) -> Vec<String> {
    let Ok(auto) = create_automation() else {
        return Vec::new();
    };
    let Ok(cond) = (unsafe { auto.CreateTrueCondition() }) else {
        return Vec::new();
    };
    let scope = if subtree {
        TreeScope_Subtree
    } else {
        TreeScope_Children
    };
    let Ok(finder) = (unsafe { root.FindAll(scope, &cond) }) else {
        return Vec::new();
    };
    let Ok(len) = (unsafe { finder.Length() }) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for i in 0..len {
        let Ok(el) = (unsafe { finder.GetElement(i) }) else {
            continue;
        };
        if let Some(n) = uia_name(&el) {
            if n.len() > 1
                && !n.contains("网易云")
                && n != "桌面歌词"
                && !n.eq_ignore_ascii_case("DesktopLyrics")
            {
                out.push(n);
            }
        }
    }
    out
}

fn read_desktop_lyric(hwnd: HWND) -> Option<String> {
    let title = hwnd_title(hwnd);
    if !title.is_empty() && title != "桌面歌词" && !title.contains("网易云音乐") {
        return Some(title);
    }
    with_com(|| {
        let Ok(auto) = create_automation() else {
            return None;
        };
        let Ok(el) = (unsafe { auto.ElementFromHandle(hwnd) }) else {
            return None;
        };
        if let Some(n) = uia_name(&el) {
            if n != "桌面歌词" && !n.contains("网易云音乐") {
                return Some(n);
            }
        }
        collect_uia_names(&el, false)
            .into_iter()
            .chain(collect_uia_names(&el, true))
            .find(|s| {
                let n = s.chars().count();
                (1..=80).contains(&n)
            })
    })
}

fn pe_file_version(path: &str) -> Option<(u16, u16, u16)> {
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let mut handle = 0u32;
        let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), Some(&mut handle));
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        if GetFileVersionInfoW(PCWSTR(wide.as_ptr()), 0, size, buf.as_mut_ptr() as *mut _)
            .is_err()
        {
            return None;
        }
        let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        if !VerQueryValueW(
            buf.as_ptr() as *const _,
            windows::core::w!("\\"),
            &mut ptr,
            &mut len,
        )
        .as_bool()
            || ptr.is_null()
            || len < 52
        {
            return None;
        }
        let info = &*(ptr as *const [u32; 13]);
        if info[0] != 0xFEEF_04BD {
            return None;
        }
        let ms = info[2];
        let ls = info[3];
        Some((
            ((ms >> 16) & 0xffff) as u16,
            (ms & 0xffff) as u16,
            ((ls >> 16) & 0xffff) as u16,
        ))
    }
}

fn process_exe_path(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; MAX_PATH as usize];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(handle);
        if ok.is_err() {
            return None;
        }
        Some(wide_to_string(&buf[..size as usize]))
    }
}

fn find_module_base(process: HANDLE, names: &[&str]) -> Option<usize> {
    unsafe {
        let mut needed = 0u32;
        let mut mods = [windows::Win32::Foundation::HMODULE::default(); 512];
        if EnumProcessModules(
            process,
            mods.as_mut_ptr(),
            (std::mem::size_of_val(&mods)) as u32,
            &mut needed,
        )
        .is_err()
        {
            return None;
        }
        let count = (needed as usize) / std::mem::size_of::<windows::Win32::Foundation::HMODULE>();
        for m in mods.iter().take(count) {
            let mut name_buf = [0u16; 256];
            let n = GetModuleBaseNameW(process, *m, &mut name_buf);
            if n == 0 {
                continue;
            }
            let name = wide_to_string(&name_buf[..n as usize]).to_ascii_lowercase();
            if names.iter().any(|want| name == *want) {
                let mut info = MODULEINFO::default();
                if GetModuleInformation(
                    process,
                    *m,
                    &mut info,
                    std::mem::size_of::<MODULEINFO>() as u32,
                )
                .is_ok()
                {
                    return Some(info.lpBaseOfDll as usize);
                }
            }
        }
        None
    }
}

fn read_qword(process: HANDLE, addr: usize) -> Option<usize> {
    let mut buf = [0u8; 8];
    let mut read = 0usize;
    unsafe {
        if ReadProcessMemory(
            process,
            addr as *const _,
            buf.as_mut_ptr() as *mut _,
            8,
            Some(&mut read),
        )
        .is_err()
            || read != 8
        {
            return None;
        }
    }
    Some(usize::from_le_bytes(buf))
}

fn read_utf16_z(process: HANDLE, addr: usize, max_chars: usize) -> Option<String> {
    let bytes = max_chars.saturating_mul(2).min(1024);
    let mut buf = vec![0u8; bytes];
    let mut read = 0usize;
    unsafe {
        if ReadProcessMemory(
            process,
            addr as *const _,
            buf.as_mut_ptr() as *mut _,
            bytes,
            Some(&mut read),
        )
        .is_err()
            || read < 2
        {
            return None;
        }
    }
    let words: Vec<u16> = buf[..read]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect();
    if words.is_empty() {
        return None;
    }
    let s = String::from_utf16_lossy(&words).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn resolve_lyric_addr(process: HANDLE, base: usize, offsets: &[usize]) -> Option<usize> {
    if offsets.is_empty() {
        return None;
    }
    let mut addr = base.checked_add(offsets[0])?;
    for off in offsets.iter().skip(1) {
        let ptr = read_qword(process, addr)?;
        addr = ptr.checked_add(*off)?;
        if addr <= 0x10000 || addr >= 0x7FFF_FFFF_0000 {
            return None;
        }
    }
    Some(addr)
}

fn read_memory_lyric(pid: u32) -> Option<String> {
    if let Ok(guard) = MEM_CACHE.lock() {
        if let Some(cache) = guard.as_ref() {
            if cache.pid == pid {
                if let Ok(handle) =
                    unsafe { OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, false, pid) }
                {
                    let text = read_utf16_z(handle, cache.lyric_addr, 256);
                    unsafe {
                        let _ = CloseHandle(handle);
                    }
                    if text.is_some() {
                        return text;
                    }
                }
            }
        }
    }

    let exe = process_exe_path(pid)?;
    let (maj, min, patch) = pe_file_version(&exe).unwrap_or((3, 1, 30));
    let ver = format!("{maj}.{min}.{patch}");
    let offsets = VERSION_OFFSETS
        .iter()
        .find(|(v, _)| *v == ver)
        .or_else(|| VERSION_OFFSETS.first())
        .map(|(_, o)| *o)?;

    unsafe {
        let handle = OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, false, pid).ok()?;
        let base = find_module_base(handle, &["cloudmusic.dll", "cloudmusic.exe"]);
        let Some(base) = base else {
            let _ = CloseHandle(handle);
            return None;
        };
        let addr = resolve_lyric_addr(handle, base, offsets);
        let text = addr.and_then(|a| read_utf16_z(handle, a, 256));
        if let Some(a) = addr {
            if let Ok(mut guard) = MEM_CACHE.lock() {
                *guard = Some(MemCache {
                    pid,
                    lyric_addr: a,
                });
            }
        }
        let _ = CloseHandle(handle);
        text
    }
}

/// Snapshot current NetEase Cloud Music playback / lyric line.
/// Cached ~800ms so lyrics plugin ticks don't EnumWindows/RPM every call.
pub fn snapshot() -> NeteaseNowPlaying {
    static CACHE: Mutex<Option<(Instant, NeteaseNowPlaying)>> = Mutex::new(None);
    if let Ok(guard) = CACHE.lock() {
        if let Some((at, snap)) = guard.as_ref() {
            if at.elapsed() < Duration::from_millis(800) {
                return snap.clone();
            }
        }
    }
    let snap = snapshot_uncached();
    if let Ok(mut guard) = CACHE.lock() {
        *guard = Some((Instant::now(), snap.clone()));
    }
    snap
}

fn snapshot_uncached() -> NeteaseNowPlaying {
    let (main, lyric_hwnd) = find_netease_hwnds();
    let mut out = NeteaseNowPlaying::default();

    if let Some(hwnd) = main {
        let _ = unsafe { IsWindowVisible(hwnd) };
        out.active = true;
        let title_raw = hwnd_title(hwnd);
        let (title, artist) = parse_title_artist(&title_raw);
        out.title = title;
        out.artist = artist;
        out.source = Some("window-title".into());

        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
        }

        if let Some(hwnd_l) = lyric_hwnd {
            if let Some(line) = read_desktop_lyric(hwnd_l) {
                out.lyric = Some(line);
                out.source = Some("desktop-lyrics".into());
            }
        }

        if out.lyric.is_none() && pid != 0 {
            if let Some(line) = read_memory_lyric(pid) {
                let same_as_title = out.title.as_ref().map(|t| t == &line).unwrap_or(false);
                if !same_as_title {
                    out.lyric = Some(line);
                    out.source = Some("memory".into());
                }
            }
        }
    } else if let Some(hwnd_l) = lyric_hwnd {
        out.active = true;
        if let Some(line) = read_desktop_lyric(hwnd_l) {
            out.lyric = Some(line);
            out.source = Some("desktop-lyrics".into());
        }
    }

    if out.title.is_some() || out.lyric.is_some() {
        out.active = true;
    }

    out
}
