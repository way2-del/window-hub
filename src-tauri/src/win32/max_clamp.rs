//! ChatGPT / Codex desktop: clamp maximize to `rcWork`, or raise the island TOPMOST.
//!
//! ChatGPT ships as many processes (`ChatGPT.exe`, `Codex.exe`, …). Matching only
//! `chatgpt` missed the real HWND. When clamp fails (Electron fights back), keep the
//! top bar above the app via [`crate::win32::topmost::set_cover_topmost`].

#[cfg(windows)]
mod win {
    use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};

    use windows::Win32::Foundation::{CloseHandle, HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
        IsWindow, IsZoomed, SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER,
    };

    /// Process stems for the ChatGPT desktop family (Task Manager shows both).
    const CLAMP_STEMS: &[&str] = &["chatgpt", "codex"];

    static LAST_CLAMPED: AtomicIsize = AtomicIsize::new(0);
    static LAST_CLAMP_MS: AtomicU64 = AtomicU64::new(0);
    static COVER_ARMED: AtomicBool = AtomicBool::new(false);

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn process_stem(hwnd: HWND) -> String {
        unsafe {
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid <= 4 {
                return String::new();
            }
            let Ok(proc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return String::new();
            };
            let mut buf = [0u16; 520];
            let mut size = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(
                proc,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(buf.as_mut_ptr()),
                &mut size,
            );
            let _ = CloseHandle(proc);
            if ok.is_err() || size == 0 {
                return String::new();
            }
            let path = String::from_utf16_lossy(&buf[..size as usize]);
            std::path::Path::new(&path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
        }
    }

    fn window_title(hwnd: HWND) -> String {
        let mut buf = [0u16; 256];
        let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
        if n <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..n as usize])
    }

    fn is_chatgpt_family(hwnd: HWND) -> bool {
        let stem = process_stem(hwnd);
        if CLAMP_STEMS.iter().any(|s| stem == *s || stem.starts_with(s)) {
            return true;
        }
        // Title fallback — renderer may be named oddly.
        let tip = window_title(hwnd).to_ascii_lowercase();
        tip.contains("chatgpt")
    }

    fn work_and_monitor(hwnd: HWND) -> Option<(RECT, RECT)> {
        unsafe {
            let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(mon, &mut info).as_bool() {
                return None;
            }
            Some((info.rcWork, info.rcMonitor))
        }
    }

    fn rect_close(a: &RECT, b: &RECT, tol: i32) -> bool {
        (a.left - b.left).abs() <= tol
            && (a.top - b.top).abs() <= tol
            && (a.right - b.right).abs() <= tol
            && (a.bottom - b.bottom).abs() <= tol
    }

    /// Maximized or borderless fill that should respect the AppBar strip.
    fn looks_maximized(hwnd: HWND, wa: &RECT, mon: &RECT) -> bool {
        unsafe {
            if IsZoomed(hwnd).as_bool() {
                return true;
            }
            let mut wr = RECT::default();
            if GetWindowRect(hwnd, &mut wr).is_err() {
                return false;
            }
            let ww = (wr.right - wr.left).max(0);
            let wh = (wr.bottom - wr.top).max(0);
            let mw = (mon.right - mon.left).max(1);
            let mh = (mon.bottom - mon.top).max(1);
            let fills_mon = ww * 100 >= mw * 92 && wh * 100 >= mh * 92;
            let fills_work = rect_close(&wr, wa, 24);
            fills_mon || fills_work
        }
    }

    fn overlaps_strip(wr: &RECT, wa: &RECT, mon: &RECT) -> bool {
        // Top into reserved strip, or taller than work area (classic Electron bug).
        wr.top < wa.top - 2
            || wr.bottom > wa.bottom + 4
            || (wr.bottom - wr.top) > (wa.bottom - wa.top) + 8
            || (wr.top <= mon.top + 2 && (wr.bottom - wr.top) >= (mon.bottom - mon.top) - 8)
    }

    fn arm_cover(on: bool) {
        let prev = COVER_ARMED.swap(on, Ordering::SeqCst);
        if prev != on {
            crate::win32::topmost::set_cover_topmost(on);
            eprintln!(
                "[max-clamp] cover topmost {}",
                if on { "ON (ChatGPT family)" } else { "off" }
            );
        }
    }

    /// Try clamp; if still wrong / Electron fights, raise the island TOPMOST.
    pub fn tick(self_hwnd: Option<isize>) {
        unsafe {
            let fg = GetForegroundWindow();
            if fg.0.is_null() || !IsWindow(fg).as_bool() || IsIconic(fg).as_bool() {
                arm_cover(false);
                return;
            }
            if let Some(me) = self_hwnd {
                if fg.0 as isize == me {
                    return;
                }
            }

            if !is_chatgpt_family(fg) {
                arm_cover(false);
                return;
            }

            let Some((wa, mon)) = work_and_monitor(fg) else {
                return;
            };
            // No AppBar inset yet — nothing to protect.
            if wa.top <= mon.top + 4 {
                arm_cover(false);
                return;
            }

            if !looks_maximized(fg, &wa, &mon) {
                if LAST_CLAMPED.load(Ordering::Relaxed) == fg.0 as isize {
                    LAST_CLAMPED.store(0, Ordering::Relaxed);
                }
                arm_cover(false);
                return;
            }

            let mut wr = RECT::default();
            if GetWindowRect(fg, &mut wr).is_err() {
                return;
            }

            // Already perfect — no cover needed.
            if rect_close(&wr, &wa, 3) {
                arm_cover(false);
                return;
            }

            // Attempt clamp (rate-limited).
            let now = now_ms();
            let same = LAST_CLAMPED.load(Ordering::Relaxed) == fg.0 as isize;
            let last = LAST_CLAMP_MS.load(Ordering::Relaxed);
            if !(same && now.saturating_sub(last) < 400) {
                let w = (wa.right - wa.left).max(1);
                let h = (wa.bottom - wa.top).max(1);
                let _ = SetWindowPos(
                    fg,
                    None,
                    wa.left,
                    wa.top,
                    w,
                    h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                LAST_CLAMPED.store(fg.0 as isize, Ordering::Relaxed);
                LAST_CLAMP_MS.store(now, Ordering::Relaxed);
                eprintln!(
                    "[max-clamp] clamp → {}x{} @({},{}) stem={}",
                    w,
                    h,
                    wa.left,
                    wa.top,
                    process_stem(fg)
                );
                // Re-read after clamp.
                let _ = GetWindowRect(fg, &mut wr);
            }

            if overlaps_strip(&wr, &wa, &mon) || !rect_close(&wr, &wa, 6) {
                // Clamp didn't stick — keep top bar above ChatGPT.
                arm_cover(true);
            } else {
                arm_cover(false);
            }
        }
    }
}

#[cfg(windows)]
pub use win::tick;

#[cfg(not(windows))]
pub fn tick(_self_hwnd: Option<isize>) {}
