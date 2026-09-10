//! Everything SDK client (Voidtools) — Host CapGate surface.
//!
//! Loads `Everything64.dll` at runtime and talks to the running Everything search
//! client via IPC. SDK state is process-global and not thread-safe → serialize with a mutex.

#![cfg(windows)]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{FreeLibrary, HMODULE, HWND, MAX_PATH};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_WITH_ALTERED_SEARCH_PATH,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const EVERYTHING_OK: u32 = 0;
const EVERYTHING_ERROR_IPC: u32 = 2;
const EVERYTHING_REQUEST_FILE_NAME: u32 = 0x0000_0001;
const EVERYTHING_REQUEST_PATH: u32 = 0x0000_0002;
const EVERYTHING_REQUEST_FULL_PATH_AND_FILE_NAME: u32 = 0x0000_0004;
const EVERYTHING_REQUEST_SIZE: u32 = 0x0000_0010;
const EVERYTHING_REQUEST_DATE_MODIFIED: u32 = 0x0000_0040;
const DEFAULT_REQUEST_FLAGS: u32 = EVERYTHING_REQUEST_FILE_NAME
    | EVERYTHING_REQUEST_PATH
    | EVERYTHING_REQUEST_FULL_PATH_AND_FILE_NAME
    | EVERYTHING_REQUEST_SIZE
    | EVERYTHING_REQUEST_DATE_MODIFIED;

type FnSetSearchW = unsafe extern "system" fn(*const u16);
type FnSetRequestFlags = unsafe extern "system" fn(u32);
type FnSetMax = unsafe extern "system" fn(u32);
type FnSetOffset = unsafe extern "system" fn(u32);
type FnSetMatchCase = unsafe extern "system" fn(i32);
type FnSetMatchWholeWord = unsafe extern "system" fn(i32);
type FnSetMatchPath = unsafe extern "system" fn(i32);
type FnSetRegex = unsafe extern "system" fn(i32);
type FnQueryW = unsafe extern "system" fn(i32) -> i32;
type FnGetLastError = unsafe extern "system" fn() -> u32;
type FnGetNumResults = unsafe extern "system" fn() -> u32;
type FnGetTotResults = unsafe extern "system" fn() -> u32;
type FnIsFolderResult = unsafe extern "system" fn(u32) -> i32;
type FnIsFileResult = unsafe extern "system" fn(u32) -> i32;
type FnGetResultFullPathNameW = unsafe extern "system" fn(u32, *mut u16, u32) -> u32;
type FnGetResultFileNameW = unsafe extern "system" fn(u32) -> *const u16;
type FnGetResultPathW = unsafe extern "system" fn(u32) -> *const u16;
type FnGetResultSize = unsafe extern "system" fn(u32, *mut i64) -> i32;
type FnIsDBLoaded = unsafe extern "system" fn() -> i32;
type FnGetMajorVersion = unsafe extern "system" fn() -> u32;
type FnGetMinorVersion = unsafe extern "system" fn() -> u32;
type FnGetRevision = unsafe extern "system" fn() -> u32;
type FnReset = unsafe extern "system" fn();

/// Hard cap so a stuck Everything IPC cannot freeze Hub / leave the panel on 搜索中 forever.
const QUERY_TIMEOUT_MS: u64 = 4_000;

struct EverythingApi {
    _module: HMODULE,
    set_search_w: FnSetSearchW,
    set_request_flags: FnSetRequestFlags,
    set_max: FnSetMax,
    set_offset: FnSetOffset,
    set_match_case: FnSetMatchCase,
    set_match_whole_word: FnSetMatchWholeWord,
    set_match_path: FnSetMatchPath,
    set_regex: FnSetRegex,
    query_w: FnQueryW,
    get_last_error: FnGetLastError,
    get_num_results: FnGetNumResults,
    get_tot_results: FnGetTotResults,
    is_folder_result: FnIsFolderResult,
    is_file_result: FnIsFileResult,
    get_result_full_path_name_w: FnGetResultFullPathNameW,
    get_result_file_name_w: FnGetResultFileNameW,
    get_result_path_w: FnGetResultPathW,
    get_result_size: FnGetResultSize,
    is_db_loaded: FnIsDBLoaded,
    get_major_version: FnGetMajorVersion,
    get_minor_version: FnGetMinorVersion,
    get_revision: FnGetRevision,
    reset: FnReset,
}

// HMODULE is a raw handle; we only load once for process lifetime.
unsafe impl Send for EverythingApi {}
unsafe impl Sync for EverythingApi {}

fn wide_nul(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

fn path_wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

unsafe fn load_fn<T>(module: HMODULE, name: &[u8]) -> Result<T, String> {
    let p = GetProcAddress(module, windows::core::PCSTR::from_raw(name.as_ptr()));
    let Some(p) = p else {
        let label = String::from_utf8_lossy(&name[..name.len().saturating_sub(1)]);
        return Err(format!("Everything SDK missing export: {label}"));
    };
    Ok(std::mem::transmute_copy(&p))
}

fn candidate_dll_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("Everything64.dll"));
            out.push(dir.join("resources").join("Everything64.dll"));
        }
    }
    // Dev / cargo run: next to crate resources & vendor
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    out.push(manifest.join("resources").join("Everything64.dll"));
    out.push(
        manifest
            .join("vendor")
            .join("everything-sdk")
            .join("dll")
            .join("Everything64.dll"),
    );
    out
}

fn load_api() -> Result<&'static EverythingApi, String> {
    static API: OnceLock<Result<EverythingApi, String>> = OnceLock::new();
    match API.get_or_init(|| {
        let mut last_err = "Everything64.dll not found".to_string();
        for path in candidate_dll_paths() {
            if !path.is_file() {
                continue;
            }
            let wide = path_wide(&path);
            let module = unsafe {
                LoadLibraryExW(
                    PCWSTR(wide.as_ptr()),
                    None,
                    LOAD_WITH_ALTERED_SEARCH_PATH,
                )
            };
            let Ok(module) = module else {
                last_err = format!("LoadLibrary failed: {}", path.display());
                continue;
            };
            let loaded = unsafe {
                (|| -> Result<EverythingApi, String> {
                    Ok(EverythingApi {
                        _module: module,
                        set_search_w: load_fn(module, b"Everything_SetSearchW\0")?,
                        set_request_flags: load_fn(module, b"Everything_SetRequestFlags\0")?,
                        set_max: load_fn(module, b"Everything_SetMax\0")?,
                        set_offset: load_fn(module, b"Everything_SetOffset\0")?,
                        set_match_case: load_fn(module, b"Everything_SetMatchCase\0")?,
                        set_match_whole_word: load_fn(module, b"Everything_SetMatchWholeWord\0")?,
                        set_match_path: load_fn(module, b"Everything_SetMatchPath\0")?,
                        set_regex: load_fn(module, b"Everything_SetRegex\0")?,
                        query_w: load_fn(module, b"Everything_QueryW\0")?,
                        get_last_error: load_fn(module, b"Everything_GetLastError\0")?,
                        get_num_results: load_fn(module, b"Everything_GetNumResults\0")?,
                        get_tot_results: load_fn(module, b"Everything_GetTotResults\0")?,
                        is_folder_result: load_fn(module, b"Everything_IsFolderResult\0")?,
                        is_file_result: load_fn(module, b"Everything_IsFileResult\0")?,
                        get_result_full_path_name_w: load_fn(
                            module,
                            b"Everything_GetResultFullPathNameW\0",
                        )?,
                        get_result_file_name_w: load_fn(module, b"Everything_GetResultFileNameW\0")?,
                        get_result_path_w: load_fn(module, b"Everything_GetResultPathW\0")?,
                        get_result_size: load_fn(module, b"Everything_GetResultSize\0")?,
                        is_db_loaded: load_fn(module, b"Everything_IsDBLoaded\0")?,
                        get_major_version: load_fn(module, b"Everything_GetMajorVersion\0")?,
                        get_minor_version: load_fn(module, b"Everything_GetMinorVersion\0")?,
                        get_revision: load_fn(module, b"Everything_GetRevision\0")?,
                        reset: load_fn(module, b"Everything_Reset\0")?,
                    })
                })()
            };
            match loaded {
                Ok(api) => return Ok(api),
                Err(e) => {
                    let _ = unsafe { FreeLibrary(module) };
                    last_err = e;
                }
            }
        }
        Err(last_err)
    }) {
        Ok(api) => Ok(api),
        Err(e) => Err(e.clone()),
    }
}

fn api_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// True while a QueryW (or status probe) owns the SDK. Survives timed-out callers
/// until the worker finishes — prevents overlapping Reset/Query corruption.
fn sdk_busy() -> &'static AtomicBool {
    static BUSY: OnceLock<AtomicBool> = OnceLock::new();
    BUSY.get_or_init(|| AtomicBool::new(false))
}

fn map_error(code: u32) -> String {
    match code {
        EVERYTHING_OK => "Everything: unknown error".into(),
        EVERYTHING_ERROR_IPC => {
            "Everything 未运行（请先启动 Everything 搜索客户端）".into()
        }
        1 => "Everything: out of memory".into(),
        3 => "Everything: RegisterClassEx failed".into(),
        4 => "Everything: CreateWindow failed".into(),
        5 => "Everything: CreateThread failed".into(),
        6 => "Everything: invalid index".into(),
        7 => "Everything: invalid call".into(),
        8 => "Everything: invalid request".into(),
        9 => "Everything: invalid parameter".into(),
        _ => format!("Everything error code {code}"),
    }
}

unsafe fn read_wstr(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
        if len > 32_768 {
            break;
        }
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EverythingStatus {
    pub available: bool,
    pub running: bool,
    pub db_loaded: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EverythingSearchOpts {
    pub max: Option<u32>,
    pub offset: Option<u32>,
    pub match_case: Option<bool>,
    pub match_whole_word: Option<bool>,
    pub match_path: Option<bool>,
    pub regex: Option<bool>,
    /// When set, prefixes query with Everything path: filter.
    pub path_prefix: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EverythingHit {
    pub name: String,
    pub path: String,
    pub full_path: String,
    pub is_folder: bool,
    pub is_file: bool,
    pub size: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EverythingSearchResult {
    pub query: String,
    pub total: u32,
    pub results: Vec<EverythingHit>,
}

pub fn status() -> EverythingStatus {
    let api = match load_api() {
        Ok(a) => a,
        Err(e) => {
            return EverythingStatus {
                available: false,
                running: false,
                db_loaded: false,
                version: None,
                error: Some(e),
            };
        }
    };
    // Never block UI/status on a hung QueryW — try_lock only.
    let Ok(_guard) = api_lock().try_lock() else {
        return EverythingStatus {
            available: true,
            running: true,
            db_loaded: false,
            version: None,
            error: Some("Everything 搜索进行中…".into()),
        };
    };
    if sdk_busy().load(Ordering::SeqCst) {
        return EverythingStatus {
            available: true,
            running: true,
            db_loaded: false,
            version: None,
            error: Some("Everything 搜索进行中…".into()),
        };
    }
    unsafe {
        // SDK: GetMajorVersion returns 0 when Everything IPC is down.
        // Prefer this over IsDBLoaded / GetLastError (false「未运行」before any query)
        // and over an empty QueryW probe (can hitch the UI lock on large indexes).
        let major = (api.get_major_version)();
        let minor = (api.get_minor_version)();
        let rev = (api.get_revision)();
        let db = (api.is_db_loaded)() != 0;
        let running = major > 0;
        EverythingStatus {
            available: true,
            running,
            db_loaded: db,
            version: if running {
                Some(format!("{major}.{minor}.{rev}"))
            } else {
                None
            },
            error: if !running {
                Some(map_error(EVERYTHING_ERROR_IPC))
            } else {
                None
            },
        }
    }
}

fn search_locked(
    api: &EverythingApi,
    q: String,
    opts: &EverythingSearchOpts,
    max: u32,
    offset: u32,
) -> Result<EverythingSearchResult, String> {
    let _guard = api_lock().lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        (api.reset)();
        let wide = wide_nul(&q);
        (api.set_search_w)(wide.as_ptr());
        (api.set_request_flags)(DEFAULT_REQUEST_FLAGS);
        (api.set_max)(max);
        (api.set_offset)(offset);
        (api.set_match_case)(i32::from(opts.match_case.unwrap_or(false)));
        (api.set_match_whole_word)(i32::from(opts.match_whole_word.unwrap_or(false)));
        (api.set_match_path)(i32::from(opts.match_path.unwrap_or(false)));
        (api.set_regex)(i32::from(opts.regex.unwrap_or(false)));

        let ok = (api.query_w)(1);
        if ok == 0 {
            let code = (api.get_last_error)();
            return Err(map_error(code));
        }

        let n = (api.get_num_results)();
        let total = (api.get_tot_results)();
        let mut results = Vec::with_capacity(n as usize);
        let mut buf = vec![0u16; (MAX_PATH as usize).saturating_mul(4).max(1024)];

        for i in 0..n {
            let written = (api.get_result_full_path_name_w)(i, buf.as_mut_ptr(), buf.len() as u32);
            let full_path = if written > 0 {
                String::from_utf16_lossy(&buf[..written as usize])
            } else {
                String::new()
            };
            let name = read_wstr((api.get_result_file_name_w)(i));
            let path = read_wstr((api.get_result_path_w)(i));
            let is_folder = (api.is_folder_result)(i) != 0;
            let is_file = (api.is_file_result)(i) != 0;
            let mut size: i64 = 0;
            let size = if (api.get_result_size)(i, &mut size) != 0 {
                Some(size)
            } else {
                None
            };
            results.push(EverythingHit {
                name,
                path,
                full_path,
                is_folder,
                is_file,
                size,
            });
        }

        Ok(EverythingSearchResult {
            query: q,
            total,
            results,
        })
    }
}

pub fn search(query: &str, opts: Option<EverythingSearchOpts>) -> Result<EverythingSearchResult, String> {
    let api = load_api()?;
    let opts = opts.unwrap_or(EverythingSearchOpts {
        max: None,
        offset: None,
        match_case: None,
        match_whole_word: None,
        match_path: None,
        regex: None,
        path_prefix: None,
    });
    let max = opts.max.unwrap_or(50).clamp(1, 200);
    let offset = opts.offset.unwrap_or(0);

    let mut q = query.trim().to_string();
    if let Some(prefix) = opts.path_prefix.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty())
    {
        let normalized = prefix.replace('/', "\\");
        if q.is_empty() {
            q = format!("path:\"{normalized}\"");
        } else {
            q = format!("path:\"{normalized}\" {q}");
        }
    }

    if sdk_busy().swap(true, Ordering::SeqCst) {
        return Err("Everything 正忙（上次搜索可能未结束），请稍后再试".into());
    }

    let (tx, rx) = mpsc::channel();
    let q_worker = q.clone();
    let opts_worker = EverythingSearchOpts {
        max: Some(max),
        offset: Some(offset),
        match_case: opts.match_case,
        match_whole_word: opts.match_whole_word,
        match_path: opts.match_path,
        regex: opts.regex,
        path_prefix: None, // already folded into `q`
    };
    let spawned = std::thread::Builder::new()
        .name("everything-q".into())
        .spawn(move || {
            // SAFETY: EverythingApi is process-static; only one worker runs at a time (sdk_busy).
            let result = search_locked(api, q_worker, &opts_worker, max, offset);
            let _ = tx.send(result);
            sdk_busy().store(false, Ordering::SeqCst);
        })
        .is_ok();
    if !spawned {
        sdk_busy().store(false, Ordering::SeqCst);
        return Err("无法启动 Everything 搜索线程".into());
    }

    match rx.recv_timeout(Duration::from_millis(QUERY_TIMEOUT_MS)) {
        Ok(r) => r,
        Err(_) => {
            // Worker may still be inside QueryW — leave sdk_busy true until it returns.
            Err("Everything 搜索超时（客户端无响应）。请确认 Everything 已运行，或重启后再试".into())
        }
    }
}

/// Best-effort: clear SDK request state when a plugin with Everything is disabled.
/// No-op if a query is in flight (avoid Reset during QueryW).
pub fn reset_if_idle() {
    if sdk_busy().load(Ordering::SeqCst) {
        return;
    }
    let Ok(api) = load_api() else {
        return;
    };
    let Ok(_guard) = api_lock().try_lock() else {
        return;
    };
    unsafe {
        (api.reset)();
    }
}

pub fn open_path(path: &str) -> Result<(), String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("empty path".into());
    }
    let wide = wide_nul(path);
    let op = wide_nul("open");
    let ret = unsafe {
        ShellExecuteW(
            HWND::default(),
            PCWSTR(op.as_ptr()),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute returns >32 on success (as HINSTANCE cast).
    if (ret.0 as isize) <= 32 {
        return Err(format!("无法打开: {path}"));
    }
    Ok(())
}

pub fn reveal_path(path: &str) -> Result<(), String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("empty path".into());
    }
    let explorer = wide_nul("explorer.exe");
    let args = wide_nul(&format!("/select,\"{path}\""));
    let op = wide_nul("open");
    let ret = unsafe {
        ShellExecuteW(
            HWND::default(),
            PCWSTR(op.as_ptr()),
            PCWSTR(explorer.as_ptr()),
            PCWSTR(args.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if (ret.0 as isize) <= 32 {
        return Err(format!("无法在资源管理器中显示: {path}"));
    }
    Ok(())
}
