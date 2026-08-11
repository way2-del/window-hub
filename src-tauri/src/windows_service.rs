//! Shared OS window enumerator — one poller, many CapGate consumers.

use parking_lot::Mutex;
use serde::Serialize;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

use crate::win32::enum_windows::{get_window, list_windows, parse_window_id, WindowInfo};

const POLL_MS: u64 = 250;

fn shared_snapshot() -> &'static Mutex<Vec<WindowInfo>> {
    static SNAP: OnceLock<Mutex<Vec<WindowInfo>>> = OnceLock::new();
    SNAP.get_or_init(|| Mutex::new(Vec::new()))
}

/// Latest poller snapshot for callers without `AppHandle` (e.g. dock layout).
/// Falls back to a fresh EnumWindows if the poller has not filled yet.
pub fn cached_windows() -> Vec<WindowInfo> {
    let snap = shared_snapshot().lock().clone();
    if !snap.is_empty() {
        return snap;
    }
    list_windows(None)
}

#[derive(Clone)]
pub struct WindowsService {
    inner: Arc<Inner>,
}

struct Inner {
    snapshot: Mutex<Vec<WindowInfo>>,
    exclude: Mutex<Option<isize>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsChangedPayload {
    pub windows: Vec<WindowInfo>,
}

impl WindowsService {
    pub fn start(app: AppHandle) -> Self {
        let svc = Self {
            inner: Arc::new(Inner {
                snapshot: Mutex::new(Vec::new()),
                exclude: Mutex::new(None),
            }),
        };
        let poller = svc.clone();
        std::thread::spawn(move || {
            loop {
                if let Some(main) = app.get_webview_window("main") {
                    if let Ok(hwnd) = main.hwnd() {
                        *poller.inner.exclude.lock() = Some(hwnd.0 as isize);
                    }
                }
                let exclude = *poller.inner.exclude.lock();
                let mut next = list_windows(exclude);
                // Never treat dock chrome as a running app for indicator dots
                if let Some(dock) = app.get_webview_window("dock") {
                    if let Ok(hwnd) = dock.hwnd() {
                        let dh = hwnd.0 as isize;
                        next.retain(|w| w.hwnd != dh);
                    }
                }
                *shared_snapshot().lock() = next.clone();
                let changed = {
                    let mut snap = poller.inner.snapshot.lock();
                    let key = |w: &WindowInfo| format!("{}:{}", w.id, w.title);
                    let prev_key: String = snap.iter().map(key).collect::<Vec<_>>().join("|");
                    let next_key: String = next.iter().map(key).collect::<Vec<_>>().join("|");
                    if prev_key != next_key {
                        *snap = next.clone();
                        true
                    } else {
                        false
                    }
                };
                if changed {
                    let _ = app.emit(
                        "hub-windows-changed",
                        WindowsChangedPayload { windows: next },
                    );
                }
                std::thread::sleep(Duration::from_millis(POLL_MS));
            }
        });
        svc
    }

    pub fn list(&self) -> Vec<WindowInfo> {
        self.inner.snapshot.lock().clone()
    }

    pub fn get(&self, id: &str) -> Result<Option<WindowInfo>, String> {
        let hwnd = parse_window_id(id)?;
        if let Some(hit) = self.inner.snapshot.lock().iter().find(|w| w.hwnd == hwnd) {
            return Ok(Some(hit.clone()));
        }
        let exclude = *self.inner.exclude.lock();
        Ok(get_window(hwnd, exclude))
    }

    pub fn refresh_now(&self, app: &AppHandle) -> Vec<WindowInfo> {
        let exclude = app
            .get_webview_window("main")
            .and_then(|w| w.hwnd().ok())
            .map(|h| h.0 as isize);
        *self.inner.exclude.lock() = exclude;
        let next = list_windows(exclude);
        *self.inner.snapshot.lock() = next.clone();
        *shared_snapshot().lock() = next.clone();
        next
    }
}
