//! Work-area quiet + island fullscreen hide flag.
//!
//! Quiet blocks *feedback* (`ABN_POSCHANGED` / non-forced SETPOS / redundant sync),
//! not the first AppBar claim. Startup marks quiet so top + bottom + taskbar
//! settle without SETPOS↔ABN thrash on maximized windows.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static QUIET_UNTIL_MS: AtomicU64 = AtomicU64::new(0);
static ISLAND_HIDDEN_FOR_FULLSCREEN: AtomicBool = AtomicBool::new(false);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Ignore non-forced SETPOS and `ABN_POSCHANGED` until `ms` from now elapses.
pub fn mark_work_area_quiet(ms: u64) {
    let until = now_ms().saturating_add(ms);
    let _ = QUIET_UNTIL_MS.fetch_max(until, Ordering::SeqCst);
}

pub fn work_area_quiet() -> bool {
    now_ms() < QUIET_UNTIL_MS.load(Ordering::SeqCst)
}

pub fn set_island_hidden_for_fullscreen(v: bool) {
    ISLAND_HIDDEN_FOR_FULLSCREEN.store(v, Ordering::SeqCst);
}

pub fn island_hidden_for_fullscreen() -> bool {
    ISLAND_HIDDEN_FOR_FULLSCREEN.load(Ordering::SeqCst)
}
