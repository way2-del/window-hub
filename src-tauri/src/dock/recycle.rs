//! Recycle Bin empty/full detection for the dock Trash tile.
//!
//! Never call the live Shell query on the UI / sync-IPC path —
//! `SHQueryRecycleBinW(NULL)` can stall for a long time on network /
//! unavailable volumes and freezes the whole process.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

static LAST_FULL: AtomicBool = AtomicBool::new(false);
static WATCHER_STARTED: AtomicBool = AtomicBool::new(false);

/// Live Shell query — **background threads only**.
///
/// Queries fixed drive letters one-by-one (never a NULL root), so a stuck
/// network volume cannot block forever as easily as `SHQueryRecycleBinW(NULL)`.
pub fn recycle_bin_is_full() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::GetDriveTypeW;
        use windows::Win32::UI::Shell::{SHQueryRecycleBinW, SHQUERYRBINFO};

        // Win32 DRIVE_* constants (avoid WindowsProgramming feature just for these).
        const DRIVE_REMOVABLE: u32 = 2;
        const DRIVE_FIXED: u32 = 3;

        for letter in b'A'..=b'Z' {
            let root = format!("{}:\\", letter as char);
            let root_w: Vec<u16> = std::ffi::OsStr::new(&root)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let dtype = unsafe { GetDriveTypeW(PCWSTR(root_w.as_ptr())) };
            if dtype != DRIVE_FIXED && dtype != DRIVE_REMOVABLE {
                continue;
            }
            let mut info = SHQUERYRBINFO {
                cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
                ..Default::default()
            };
            let ok = unsafe { SHQueryRecycleBinW(PCWSTR(root_w.as_ptr()), &mut info) }.is_ok();
            if ok && info.i64NumItems > 0 {
                return true;
            }
        }
        false
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Cached fullness for UI / `with_icons` (never hits Shell).
pub fn last_known_full() -> bool {
    LAST_FULL.load(Ordering::Relaxed)
}

fn publish_state(app: &AppHandle, full: bool) {
    LAST_FULL.store(full, Ordering::Relaxed);
    // Do not clear the PNG hot cache on every empty/full flip.
    super::invalidate_dock_merge_cache();
    let _ = app.emit("dock-trash-state", serde_json::json!({ "full": full }));
    // Use cached fullness inside with_icons — do not re-enter Shell here.
    let prefs = super::with_icons(super::load_dock_prefs());
    let _ = app.emit("dock-prefs", &prefs);
}

/// Poll Recycle Bin fullness; emit `dock-trash-state` + refreshed `dock-prefs` on change.
pub fn spawn_trash_watcher(app: AppHandle) {
    if WATCHER_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("dock-trash-watch".into())
        .spawn(move || {
            let initial = recycle_bin_is_full();
            LAST_FULL.store(initial, Ordering::Relaxed);
            // First paint may have used the default `false` cache — push once.
            publish_state(&app, initial);
            loop {
                std::thread::sleep(Duration::from_secs(3));
                if !super::load_dock_prefs().enabled {
                    continue;
                }
                let full = recycle_bin_is_full();
                let prev = LAST_FULL.load(Ordering::Relaxed);
                if prev == full {
                    continue;
                }
                publish_state(&app, full);
            }
        })
        .ok();
}

/// Schedule a background re-query after empty / other mutations (never blocks caller).
pub fn notify_trash_changed(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("dock-trash-notify".into())
        .spawn(move || {
            // Let the OS confirmation / FS settle, then sample twice.
            for delay_ms in [200u64, 900] {
                std::thread::sleep(Duration::from_millis(delay_ms));
                let full = recycle_bin_is_full();
                publish_state(&app, full);
            }
        })
        .ok();
}
