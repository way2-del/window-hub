//! Double-Ctrl global hotkey via low-level keyboard hook.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter};

static HOOK_STARTED: AtomicBool = AtomicBool::new(false);
static LAST_CTRL_UP_MS: AtomicU64 = AtomicU64::new(0);
static ENABLED: AtomicBool = AtomicBool::new(true);
static INTERVAL_MS: AtomicU64 = AtomicU64::new(350);
static APP: OnceLock<AppHandle> = OnceLock::new();

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn configure(enabled: bool, interval_ms: u64) {
    ENABLED.store(enabled, Ordering::SeqCst);
    INTERVAL_MS.store(interval_ms.clamp(100, 2000), Ordering::SeqCst);
}

pub fn start(app: AppHandle) {
    let _ = APP.set(app);
    let cfg = super::config::load();
    configure(cfg.hotkey_enabled && cfg.enabled, cfg.double_ctrl_ms);
    if HOOK_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    #[cfg(windows)]
    std::thread::spawn(|| {
        if let Err(e) = run_hook_thread() {
            eprintln!("[sousou] hotkey hook failed: {e}");
            HOOK_STARTED.store(false, Ordering::SeqCst);
        }
    });
}

#[cfg(windows)]
fn run_hook_thread() -> Result<(), String> {
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_CONTROL;
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
        UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYUP, WM_SYSKEYUP,
    };

    unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let msg = wparam.0 as u32;
            if msg == WM_KEYUP || msg == WM_SYSKEYUP {
                let kb = *(lparam.0 as *const KBDLLHOOKSTRUCT);
                // VK_LCONTROL / VK_RCONTROL / VK_CONTROL
                if kb.vkCode == 0x11 || kb.vkCode == 0xA2 || kb.vkCode == 0xA3 {
                    let _ = VK_CONTROL;
                    on_ctrl_up();
                }
            }
        }
        unsafe { CallNextHookEx(HHOOK::default(), code, wparam, lparam) }
    }

    unsafe {
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), HINSTANCE::default(), 0)
            .map_err(|e| e.to_string())?;
        let mut msg = MSG::default();
        loop {
            let ret = GetMessageW(&mut msg, HWND::default(), 0, 0);
            if ret.0 == 0 || ret.0 == -1 {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = UnhookWindowsHookEx(hook);
    }
    Ok(())
}

fn on_ctrl_up() {
    if !ENABLED.load(Ordering::SeqCst) {
        return;
    }
    let now = now_ms();
    let last = LAST_CTRL_UP_MS.swap(now, Ordering::SeqCst);
    let interval = INTERVAL_MS.load(Ordering::SeqCst);
    if last == 0 || now.saturating_sub(last) > interval {
        return;
    }
    LAST_CTRL_UP_MS.store(0, Ordering::SeqCst);
    if let Some(app) = APP.get() {
        let app = app.clone();
        std::thread::spawn(move || {
            let handle = app.clone();
            tauri::async_runtime::block_on(async move {
                let _ = super::window::toggle(handle).await;
            });
            let _ = app.emit("sousou-toggled", ());
        });
    }
}
