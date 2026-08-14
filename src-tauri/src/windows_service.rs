//! Shared OS window enumerator — one poller, many CapGate consumers.

use parking_lot::Mutex;
use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

use crate::win32::enum_windows::{get_window, list_windows, parse_window_id, WindowInfo};

/// Base poll — was 700ms and thrashed plugins on every title blink.
const POLL_MS: u64 = 1100;
const POLL_MS_PRESSURE: u64 = 2200;
/// Title-only changes (Chrome tab text…) — coalesce emits.
const TITLE_EMIT_MIN: Duration = Duration::from_millis(2500);

#[derive(Clone)]
pub struct WindowsService {
    inner: Arc<Inner>,
}

struct Inner {
    snapshot: Mutex<Vec<WindowInfo>>,
    exclude: Mutex<Option<isize>>,
    last_struct_fp: Mutex<String>,
    last_title_fp: Mutex<String>,
    last_title_emit: Mutex<Instant>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsChangedPayload {
    pub windows: Vec<WindowInfo>,
}

fn structural_fp(wins: &[WindowInfo]) -> String {
    // id + exe — ignore title flicker (browsers / editors).
    let mut parts: Vec<String> = wins
        .iter()
        .map(|w| format!("{}:{}", w.id, w.exe_name.as_deref().unwrap_or("")))
        .collect();
    parts.sort();
    parts.join("|")
}

fn title_fp(wins: &[WindowInfo]) -> String {
    wins.iter()
        .map(|w| w.title.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn under_mem_pressure() -> bool {
    #[cfg(windows)]
    {
        crate::win32::system_memory::physical_mem_percent() >= 88
    }
    #[cfg(not(windows))]
    {
        false
    }
}

impl WindowsService {
    pub fn start(app: AppHandle) -> Self {
        let svc = Self {
            inner: Arc::new(Inner {
                snapshot: Mutex::new(Vec::new()),
                exclude: Mutex::new(None),
                last_struct_fp: Mutex::new(String::new()),
                last_title_fp: Mutex::new(String::new()),
                last_title_emit: Mutex::new(
                    Instant::now()
                        .checked_sub(Duration::from_secs(60))
                        .unwrap_or_else(Instant::now),
                ),
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
                // list_windows already drops our PID; also strip known host labels by hwnd.
                let mut next = list_windows(exclude);
                const HOST_LABELS: &[&str] = &[
                    "dock",
                    "dock-glass",
                    "dock-trigger",
                    "dock-preview",
                    "dock-item-menu",
                    "main",
                    "settings",
                    "status-menu",
                    "system-flyout",
                ];
                for label in HOST_LABELS {
                    if let Some(w) = app.get_webview_window(label) {
                        if let Ok(hwnd) = w.hwnd() {
                            let h = hwnd.0 as isize;
                            next.retain(|x| x.hwnd != h);
                        }
                    }
                }

                let struct_fp = structural_fp(&next);
                let titles = title_fp(&next);
                let emit = {
                    let mut snap = poller.inner.snapshot.lock();
                    let mut last_s = poller.inner.last_struct_fp.lock();
                    let mut last_t = poller.inner.last_title_fp.lock();
                    let mut last_te = poller.inner.last_title_emit.lock();

                    let struct_changed = *last_s != struct_fp;
                    let title_changed = *last_t != titles;
                    let should = if struct_changed {
                        true
                    } else if title_changed {
                        last_te.elapsed() >= TITLE_EMIT_MIN
                    } else {
                        false
                    };
                    if should {
                        *snap = next.clone();
                        *last_s = struct_fp;
                        *last_t = titles;
                        *last_te = Instant::now();
                    } else if !next.is_empty() {
                        // Keep cache fresh for list() even when not emitting.
                        *snap = next.clone();
                    }
                    should
                };
                if emit {
                    let _ = app.emit(
                        "hub-windows-changed",
                        WindowsChangedPayload { windows: next },
                    );
                }
                let ms = if under_mem_pressure() {
                    POLL_MS_PRESSURE
                } else {
                    POLL_MS
                };
                std::thread::sleep(Duration::from_millis(ms));
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
        *self.inner.last_struct_fp.lock() = structural_fp(&next);
        *self.inner.last_title_fp.lock() = title_fp(&next);
        next
    }
}
