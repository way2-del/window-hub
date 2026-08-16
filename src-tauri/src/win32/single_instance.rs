//! Single-instance guard for the GUI process.
//! Service entry (`--autostart-svc`) must not take this lock.
//!
//! Uses a **Global\\** mutex so Session-0 (the autostart service) and the user
//! session agree on “GUI is up”. Process-name scans are intentionally avoided:
//! the service is the same `window-hub.exe` and often cannot be distinguished
//! when command-line queries fail across integrity levels.

#![cfg(windows)]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::{CreateMutexW, OpenMutexW, MUTEX_MODIFY_STATE};
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
};

/// Cross-session; service OpenMutex + GUI CreateMutex share this name.
pub const UI_MUTEX_NAME: &str = "Global\\com.xushi.window-hub.ui";

/// Kept alive for the process lifetime (raw HANDLE as isize for Sync).
static INSTANCE_MUTEX: AtomicIsize = AtomicIsize::new(0);

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn show_already_running_dialog() {
    let text = wide("Window Hub 已在运行，请勿重复打开。");
    let caption = wide("Window Hub");
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(caption.as_ptr()),
            MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND | MB_TOPMOST,
        );
    }
}

fn try_acquire_instance_mutex() -> bool {
    let name = wide(UI_MUTEX_NAME);
    unsafe {
        let handle = match CreateMutexW(None, true, PCWSTR(name.as_ptr())) {
            Ok(h) => h,
            Err(_) => return false,
        };
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(handle);
            return false;
        }
        INSTANCE_MUTEX.store(handle.0 as isize, Ordering::SeqCst);
        true
    }
}

/// True when the interactive GUI holds the Global UI mutex (service-safe check).
pub fn any_gui_instance_running() -> bool {
    let name = wide(UI_MUTEX_NAME);
    unsafe {
        match OpenMutexW(MUTEX_MODIFY_STATE, false, PCWSTR(name.as_ptr())) {
            Ok(h) => {
                let _ = CloseHandle(h);
                true
            }
            Err(_) => false,
        }
    }
}

/// Call once from the GUI entry (`main` without `--autostart-svc`).
/// If another UI instance exists: show a dialog and exit the process.
pub fn ensure_single_instance_or_exit() {
    // Mutex only — never treat the Session-0 service exe as a second GUI.
    if !try_acquire_instance_mutex() {
        show_already_running_dialog();
        std::process::exit(0);
    }
    // Manual launch after “退出” should clear the suppress so a later crash can
    // still be recovered by the autostart service (quit stays until then / reboot).
    crate::win32::autostart_svc::clear_user_quit();
}
