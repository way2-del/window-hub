//! Watch only the visible tray banner, reusing the ambient foreground probe.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Dwell {
    foreground: isize,
    since: Option<Instant>,
}

impl Dwell {
    fn observe(&mut self, foreground: isize, matches: bool, now: Instant) -> bool {
        if !matches || foreground == 0 {
            self.since = None;
            self.foreground = 0;
            return false;
        }
        if self.foreground != foreground || self.since.is_none() {
            self.foreground = foreground;
            self.since = Some(now);
        }
        now.duration_since(self.since.unwrap()) >= Duration::from_millis(240)
    }
}

struct Watch {
    token: String,
    hwnd: isize,
    pid: u32,
    dwell: Dwell,
}
static ACTIVE: AtomicBool = AtomicBool::new(false);
static WATCH: Mutex<Option<Watch>> = Mutex::new(None);

#[cfg(windows)]
fn owner_pid(hwnd: isize) -> u32 {
    let mut pid = 0;
    if hwnd != 0 {
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
                windows::Win32::Foundation::HWND(hwnd as *mut _),
                Some(&mut pid),
            );
        }
    }
    pid
}
#[cfg(not(windows))]
fn owner_pid(_hwnd: isize) -> u32 {
    0
}

/// A per-effect token prevents cleanup for an old banner clearing a newer watch.
#[tauri::command]
pub fn watch_tray_notification(window: tauri::WebviewWindow, token: String, hwnd: Option<isize>) {
    if window.label() != "main" {
        return;
    }
    let Ok(mut watch) = WATCH.lock() else {
        return;
    };
    if let Some(hwnd) = hwnd {
        let pid = owner_pid(hwnd);
        *watch = (pid != 0 && pid != std::process::id()).then(|| Watch {
            token,
            hwnd,
            pid,
            dwell: Dwell::default(),
        });
    } else if watch.as_ref().is_some_and(|current| current.token == token) {
        *watch = None;
    }
    ACTIVE.store(watch.is_some(), Ordering::Release);
}

/// No active tray banner: one atomic read. Active: two HWND → PID lookups,
/// no process enumeration, image capture, process handles, or frontend polling.
pub fn poll(foreground: isize) -> Option<String> {
    if !ACTIVE.load(Ordering::Acquire) {
        return None;
    }
    let Ok(mut watch) = WATCH.try_lock() else {
        return None;
    };
    let current = watch.as_mut()?;
    let matches = owner_pid(foreground) == current.pid && owner_pid(current.hwnd) == current.pid;
    if !current.dwell.observe(foreground, matches, Instant::now()) {
        return None;
    }
    let token = current.token.clone();
    *watch = None;
    ACTIVE.store(false, Ordering::Release);
    Some(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(windows)]
    fn native_owner_check_and_idle_cost() {
        let foreground = super::super::ambient::foreground_key();
        let pid = owner_pid(foreground);
        if pid == 0 { return; }
        let make_watch = |pid| Watch {
            token: "test-only".into(), hwnd: foreground, pid,
            dwell: Dwell { foreground, since: Some(Instant::now() - Duration::from_secs(1)) },
        };
        *WATCH.lock().unwrap() = Some(make_watch(pid.wrapping_add(1)));
        ACTIVE.store(true, Ordering::Release);
        let start = Instant::now();
        for _ in 0..10_000 { assert!(poll(foreground).is_none()); }
        println!("active native check (debug): {:.2} us/call", start.elapsed().as_secs_f64() * 100.0);
        *WATCH.lock().unwrap() = Some(make_watch(pid));
        assert_eq!(poll(foreground).as_deref(), Some("test-only"));
        assert!(poll(foreground).is_none());
        assert!(!ACTIVE.load(Ordering::Acquire));
    }

    #[test]
    fn only_a_stable_matching_foreground_is_acknowledged() {
        let mut dwell = Dwell::default();
        let start = Instant::now();
        assert!(!dwell.observe(10, true, start));
        assert!(!dwell.observe(10, true, start + Duration::from_millis(239)));
        assert!(dwell.observe(10, true, start + Duration::from_millis(240)));
        assert!(!dwell.observe(20, true, start + Duration::from_millis(250)));
        assert!(!dwell.observe(20, false, start + Duration::from_millis(500)));
        assert!(!dwell.observe(20, true, start + Duration::from_millis(600)));
        assert!(!dwell.observe(0, true, start + Duration::from_millis(900)));
    }
}
