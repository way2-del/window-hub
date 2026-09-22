//! Click-hang tracer: append-only log so we can see the last step before 未响应.
//!
//! Log file: `%TEMP%\window-hub-click-trace.log`
//!
//! Channels (all append the same file):
//! - Rust `log()` / `click_trace!` — any thread, never touches WebView
//! - HTTP `127.0.0.1:38765` POST `/t` — FE bypasses Tauri IPC (survives pump hang)
//! - Hang watchdog — `IsHungAppWindow(main)` every 250ms; dumps ring + HUNG line
//! - WH_MOUSE_LL — native click over our HWNDs even if WebView is dead

use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static SEQ: AtomicU64 = AtomicU64::new(0);
static CREATE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static HEARTBEAT_STARTED: AtomicBool = AtomicBool::new(false);
static HANG_WATCH_STARTED: AtomicBool = AtomicBool::new(false);
static HTTP_STARTED: AtomicBool = AtomicBool::new(false);
static MOUSE_HOOK_STARTED: AtomicBool = AtomicBool::new(false);
static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
static LAST_HUNG_LOG_MS: AtomicU64 = AtomicU64::new(0);

const RING_CAP: usize = 96;
const HTTP_PORT: u16 = 38765;

fn ring() -> &'static Mutex<VecDeque<String>> {
    static RING: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
    RING.get_or_init(|| Mutex::new(VecDeque::with_capacity(RING_CAP)))
}

fn log_path() -> PathBuf {
    std::env::temp_dir().join("window-hub-click-trace.log")
}

pub fn path_string() -> String {
    log_path().to_string_lossy().into_owned()
}

pub fn http_endpoint() -> String {
    format!("http://127.0.0.1:{HTTP_PORT}/t")
}

pub fn clear() {
    SEQ.store(0, Ordering::Relaxed);
    CREATE_IN_PROGRESS.store(false, Ordering::SeqCst);
    if let Ok(mut g) = ring().lock() {
        g.clear();
    }
    let _ = std::fs::remove_file(log_path());
}

pub fn set_main_hwnd(hwnd: isize) {
    MAIN_HWND.store(hwnd, Ordering::SeqCst);
    ensure_hang_watchdog();
    ensure_http_sink();
    ensure_mouse_hook();
}

pub fn mark_create_in_progress(active: bool) {
    CREATE_IN_PROGRESS.store(active, Ordering::SeqCst);
    ensure_heartbeat();
}

fn ensure_heartbeat() {
    if HEARTBEAT_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("click-trace-hb".into())
        .spawn(|| loop {
            std::thread::sleep(Duration::from_millis(1000));
            if CREATE_IN_PROGRESS.load(Ordering::SeqCst) {
                log("heartbeat", "CREATE_IN_PROGRESS still true");
            }
        })
        .ok();
}

fn ensure_hang_watchdog() {
    if HANG_WATCH_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("click-trace-hang".into())
        .spawn(|| {
            log("watchdog", "hang poller started (IsHungAppWindow @250ms)");
            let mut was_hung = false;
            loop {
                std::thread::sleep(Duration::from_millis(250));
                let hwnd = MAIN_HWND.load(Ordering::SeqCst);
                let hung = is_hung(hwnd);
                if hung && !was_hung {
                    dump_hung("became HUNG (IsHungAppWindow=true)");
                } else if hung {
                    let now = unix_ms();
                    let last = LAST_HUNG_LOG_MS.load(Ordering::Relaxed);
                    if now.saturating_sub(last) >= 2000 {
                        dump_hung("still HUNG");
                    }
                } else if was_hung {
                    log("watchdog", "RECOVERED (IsHungAppWindow=false)");
                }
                was_hung = hung;
            }
        })
        .ok();
}

fn is_hung(hwnd_raw: isize) -> bool {
    if hwnd_raw == 0 {
        return false;
    }
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{IsHungAppWindow, IsWindow};
        let hwnd = HWND(hwnd_raw as *mut _);
        unsafe {
            if !IsWindow(hwnd).as_bool() {
                return false;
            }
            IsHungAppWindow(hwnd).as_bool()
        }
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd_raw;
        false
    }
}

fn dump_hung(reason: &str) {
    LAST_HUNG_LOG_MS.store(unix_ms(), Ordering::Relaxed);
    let hwnd = MAIN_HWND.load(Ordering::SeqCst);
    let create = CREATE_IN_PROGRESS.load(Ordering::SeqCst);
    log(
        "HUNG",
        &format!("{reason} hwnd={hwnd:#x} CREATE_IN_PROGRESS={create}"),
    );
    if let Ok(g) = ring().lock() {
        log("HUNG", &format!("--- ring dump ({} lines) ---", g.len()));
        for line in g.iter() {
            let _ = append_raw(&format!("HUNG-RING\t{line}"));
        }
        log("HUNG", "--- end ring dump ---");
    }
}

fn ensure_http_sink() {
    if HTTP_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("click-trace-http".into())
        .spawn(|| {
            let addr = format!("127.0.0.1:{HTTP_PORT}");
            let listener = match TcpListener::bind(&addr) {
                Ok(l) => l,
                Err(e) => {
                    log("http", &format!("bind {addr} failed: {e}"));
                    return;
                }
            };
            log(
                "http",
                &format!("listening on http://{addr}/t (FE IPC bypass)"),
            );
            for stream in listener.incoming() {
                match stream {
                    Ok(s) => {
                        let _ = std::thread::Builder::new()
                            .name("click-trace-http-req".into())
                            .spawn(move || handle_http(s));
                    }
                    Err(_) => break,
                }
            }
        })
        .ok();
}

fn handle_http(mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(800)));
    let mut buf = [0u8; 8192];
    let n = match stream.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return,
    };
    let req = String::from_utf8_lossy(&buf[..n]);
    let (head, body) = match req.split_once("\r\n\r\n") {
        Some(p) => p,
        None => match req.split_once("\n\n") {
            Some(p) => p,
            None => ("", ""),
        },
    };
    let first = head.lines().next().unwrap_or("");
    if first.starts_with("OPTIONS ") {
        let _ = stream.write_all(
            b"HTTP/1.0 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, GET, OPTIONS\r\nAccess-Control-Allow-Headers: *\r\nContent-Length: 0\r\n\r\n",
        );
        return;
    }
    if first.starts_with("GET /health") {
        let _ = stream.write_all(
            b"HTTP/1.0 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\nok",
        );
        return;
    }
    if first.starts_with("POST /t") || first.starts_with("POST /trace") {
        let body = body.trim();
        if !body.is_empty() {
            if body.starts_with('{') {
                let o = json_str(body, "o").or_else(|| json_str(body, "origin"));
                let m = json_str(body, "m").or_else(|| json_str(body, "msg"));
                if let (Some(o), Some(m)) = (o, m) {
                    log(&o, &m);
                } else {
                    log("fe-http", body);
                }
            } else if let Some((o, m)) = body.split_once('|').or_else(|| body.split_once('\t')) {
                log(o.trim(), m.trim());
            } else {
                log("fe-http", body);
            }
        }
        let _ = stream.write_all(
            b"HTTP/1.0 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 0\r\n\r\n",
        );
        return;
    }
    let _ = stream.write_all(b"HTTP/1.0 404 Not Found\r\nContent-Length: 0\r\n\r\n");
}

fn json_str(s: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let i = s.find(&pat)?;
    let after = &s[i + pat.len()..];
    let colon = after.find(':')?;
    let rest = after[colon + 1..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let rest = &rest[1..];
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else if c == '"' {
            break;
        } else {
            out.push(c);
        }
    }
    Some(out)
}

fn ensure_mouse_hook() {
    if MOUSE_HOOK_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    #[cfg(windows)]
    {
        std::thread::Builder::new()
            .name("click-trace-mouse".into())
            .spawn(|| {
                use std::sync::atomic::AtomicPtr;
                use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
                use windows::Win32::UI::WindowsAndMessaging::{
                    CallNextHookEx, GetMessageW, GetWindowThreadProcessId, SetWindowsHookExW,
                    UnhookWindowsHookEx, WindowFromPoint, HHOOK, MSG, MSLLHOOKSTRUCT, WH_MOUSE_LL,
                    WM_LBUTTONDOWN, WM_RBUTTONDOWN,
                };

                static HOOK: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(core::ptr::null_mut());
                static OUR_PID: AtomicU64 = AtomicU64::new(0);

                unsafe extern "system" fn hook_proc(
                    code: i32,
                    wparam: WPARAM,
                    lparam: LPARAM,
                ) -> LRESULT {
                    if code >= 0
                        && (wparam.0 == WM_LBUTTONDOWN as usize
                            || wparam.0 == WM_RBUTTONDOWN as usize)
                    {
                        let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
                        let pt = info.pt;
                        let under = WindowFromPoint(pt);
                        let under_raw = under.0 as isize;
                        if under_raw != 0 && hwnd_is_our_process(under) {
                            let main = MAIN_HWND.load(Ordering::SeqCst);
                            let btn = if wparam.0 == WM_LBUTTONDOWN as usize {
                                "L"
                            } else {
                                "R"
                            };
                            let hung = is_hung(main);
                            log(
                                "native-click",
                                &format!(
                                    "{btn} at {},{} under={under_raw:#x} main={main:#x} hung={hung}",
                                    pt.x, pt.y
                                ),
                            );
                        }
                    }
                    let h = HOOK.load(Ordering::SeqCst);
                    CallNextHookEx(HHOOK(h), code, wparam, lparam)
                }

                unsafe fn hwnd_is_our_process(hwnd: HWND) -> bool {
                    let mut pid = 0u32;
                    let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
                    let ours = OUR_PID.load(Ordering::Relaxed) as u32;
                    ours != 0 && pid == ours
                }

                OUR_PID.store(
                    std::process::id() as u64,
                    Ordering::Relaxed,
                );
                log("mouse", "WH_MOUSE_LL installing");
                unsafe {
                    let hook = SetWindowsHookExW(
                        WH_MOUSE_LL,
                        Some(hook_proc),
                        HINSTANCE(core::ptr::null_mut()),
                        0,
                    );
                    match hook {
                        Ok(h) => {
                            HOOK.store(h.0, Ordering::SeqCst);
                            log("mouse", "WH_MOUSE_LL ok — clicks on our HWND will log");
                            let mut msg = MSG::default();
                            while GetMessageW(&mut msg, HWND(core::ptr::null_mut()), 0, 0)
                                .as_bool()
                            {}
                            let _ = UnhookWindowsHookEx(h);
                        }
                        Err(e) => {
                            log("mouse", &format!("WH_MOUSE_LL failed: {e}"));
                        }
                    }
                }
            })
            .ok();
    }
    #[cfg(not(windows))]
    {
        log("mouse", "skipped (non-windows)");
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn append_raw(line: &str) -> std::io::Result<()> {
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())?;
    f.write_all(line.as_bytes())?;
    f.write_all(b"\n")?;
    f.flush()?;
    Ok(())
}

/// Append one line. Never calls WebView / HWND APIs — safe from any thread / sync IPC.
pub fn log(origin: &str, msg: &str) {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    let ms = unix_ms();
    let create = if CREATE_IN_PROGRESS.load(Ordering::SeqCst) {
        " CREATE"
    } else {
        ""
    };
    let line = format!("{ms}\t#{seq}\t{origin}{create}\t{msg}");
    eprint!("[click-trace] {origin}: {msg}\n");
    if let Ok(mut g) = ring().lock() {
        if g.len() >= RING_CAP {
            g.pop_front();
        }
        g.push_back(line.clone());
    }
    let _ = append_raw(&line);
}

/// Timed scope helper — logs enter immediately and leave (+elapsed) on drop.
pub struct Scope {
    origin: &'static str,
    label: String,
    start: Instant,
}

impl Scope {
    pub fn enter(origin: &'static str, label: impl Into<String>) -> Self {
        let label = label.into();
        log(origin, &format!("{label} ENTER"));
        Self {
            origin,
            label,
            start: Instant::now(),
        }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        log(
            self.origin,
            &format!(
                "{} LEAVE {}ms",
                self.label,
                self.start.elapsed().as_millis()
            ),
        );
    }
}

pub fn log_lock_wait(label: &str) {
    log("lock", &format!("WEBVIEW_CREATE wait {label}"));
}

pub fn log_lock_acquired(label: &str, waited_ms: u128) {
    log(
        "lock",
        &format!("WEBVIEW_CREATE acquired {label} after {waited_ms}ms"),
    );
}

pub fn log_lock_released(label: &str) {
    log("lock", &format!("WEBVIEW_CREATE released {label}"));
}

#[macro_export]
macro_rules! click_trace {
    ($origin:expr, $($arg:tt)*) => {{
        $crate::win32::click_trace::log($origin, &format!($($arg)*));
    }};
}
