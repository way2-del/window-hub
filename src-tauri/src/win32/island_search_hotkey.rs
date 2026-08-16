//! Global Alt+Space → emit `island-search-hotkey` for Host island search session.

#![cfg(windows)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Start a dedicated message-loop thread for Alt+Space (VK_SPACE).
pub fn spawn_island_search_hotkey(app: AppHandle) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    thread::spawn(move || {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_NOREPEAT,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, TranslateMessage, MSG, WM_HOTKEY,
        };

        const HOTKEY_ID: i32 = 0x15E1; // island search
        const VK_SPACE: u32 = 0x20;

        unsafe {
            // Prefer MOD_NOREPEAT; fall back to plain Alt if that fails (some shells reject it).
            let mut registered = false;
            for mods in [
                HOT_KEY_MODIFIERS(MOD_ALT.0 | MOD_NOREPEAT.0),
                HOT_KEY_MODIFIERS(MOD_ALT.0),
            ] {
                let _ = UnregisterHotKey(None, HOTKEY_ID);
                if RegisterHotKey(None, HOTKEY_ID, mods, VK_SPACE).is_ok() {
                    registered = true;
                    eprintln!("[island-search] Alt+Space hotkey registered");
                    break;
                }
            }
            if !registered {
                eprintln!("[island-search] RegisterHotKey Alt+Space failed (maybe taken by another app)");
                RUNNING.store(false, Ordering::SeqCst);
                return;
            }

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                if !RUNNING.load(Ordering::SeqCst) {
                    break;
                }
                if msg.message == WM_HOTKEY && msg.wParam.0 == HOTKEY_ID as usize {
                    // Non-unit payload — more reliable across Tauri event bridge.
                    let _ = app.emit("island-search-hotkey", true);
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = UnregisterHotKey(None, HOTKEY_ID);
            RUNNING.store(false, Ordering::SeqCst);
        }
    });
    thread::sleep(Duration::from_millis(10));
}
