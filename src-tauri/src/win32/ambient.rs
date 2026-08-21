//! Ambient strip color.
//! - Maximized / fullscreen window:
//!   - **edge**: GetWindowDC at junction (+1px), then screen BitBlt if DC empty —
//!     screen path skipped while Win11 Snap Layouts is visible.
//!   - **center**: solid color from that same junction band
//! - Windowed → desktop wallpaper (edge may use wallpaper top-row strip).
//! - After a target switch: live-sample ~5s then lock until next switch.
//! - Never PrintWindow on the hot path (full-frame PW can hang the process).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SampleMode {
    #[default]
    Edge,
    Center,
}

impl SampleMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Edge => "edge",
            Self::Center => "center",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "center" => Self::Center,
            _ => Self::Edge,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AmbientStrip {
    /// Fallback / average tone.
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// Strip pixel width (encoded image).
    pub width: u32,
    /// Horizontal offset from our window's left (logical CSS px approx physical).
    pub offset_x: i32,
    /// How wide to paint the ribbon inside our bar (physical px).
    pub span_width: i32,
    /// PNG (1×width RGB) as base64 — stretch across the ambient bar.
    pub png_base64: String,
    pub hwnd: isize,
    pub mode: SampleMode,
}

impl AmbientStrip {
    pub fn fallback() -> Self {
        Self {
            r: 32,
            g: 32,
            b: 34,
            width: 1,
            offset_x: 0,
            span_width: 0,
            png_base64: solid_png_b64(32, 32, 34),
            hwnd: 0,
            mode: SampleMode::Edge,
        }
    }
}

fn solid_png_b64(r: u8, g: u8, b: u8) -> String {
    use base64::Engine;
    let img = image::RgbImage::from_pixel(1, 1, image::Rgb([r, g, b]));
    let mut buf = Vec::new();
    if image::DynamicImage::ImageRgb8(img)
        .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .is_err()
    {
        return String::new();
    }
    base64::engine::general_purpose::STANDARD.encode(buf)
}

/// Kept for docs / future avg-only API.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Serialize)]
pub struct AmbientColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub hwnd: isize,
}

#[allow(dead_code)]
impl From<&AmbientStrip> for AmbientColor {
    fn from(s: &AmbientStrip) -> Self {
        Self {
            r: s.r,
            g: s.g,
            b: s.b,
            hwnd: s.hwnd,
        }
    }
}

#[cfg(windows)]
mod win {
    use super::{solid_png_b64, AmbientStrip, SampleMode};
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use base64::Engine;
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Dwm::{
        DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
    };
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, GetMonitorInfoW, GetSysColor, GetWindowDC, MonitorFromWindow, ReleaseDC,
        SelectObject, SetStretchBltMode, StretchBlt, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        COLOR_DESKTOP, DIB_RGB_COLORS, HDC, HALFTONE, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        SRCCOPY,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetAncestor, GetClassNameW, GetForegroundWindow, GetWindowLongW,
        GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, IsZoomed,
        SystemParametersInfoW, GA_ROOT, GWL_EXSTYLE, GWL_STYLE, SPI_GETDESKWALLPAPER,
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WS_CAPTION, WS_EX_TOOLWINDOW,
    };

    const PW_RENDERFULLCONTENT: u32 = 0x2;

    const MODE_EDGE: u8 = 0;
    const MODE_CENTER: u8 = 1;

    /// How long to keep live-sampling after a window/desktop target switch.
    /// Short settle + timed blit — avoid multi-second BitBlt storms (DWM freeze).
    const SETTLE_MS: u64 = 900;
    /// Top rows to average from the target window (window-local Y).
    const TOP_ROWS: i32 = 2;
    /// Hard cap on window-DC / screen ribbon width.
    const MAX_RIBBON_W: i32 = 960;
    /// Abort GDI sample if it does not finish (hung HWND / DWM stall).
    const CAPTURE_TIMEOUT_MS: u64 = 80;

    /// Last live sample — used when focus is on the island (no external target).
    static LAST_STRIP: Mutex<Option<AmbientStrip>> = Mutex::new(None);
    static SAMPLE_MODE: AtomicU8 = AtomicU8::new(MODE_EDGE);
    /// True while still in the post-switch settle window (watcher polls faster).
    static SETTLING: AtomicBool = AtomicBool::new(true);

    struct SampleGate {
        /// 0 = wallpaper / none; else target HWND. `isize::MIN` = uninitialized.
        target_key: isize,
        settle_started: Option<Instant>,
        locked: Option<AmbientStrip>,
        last_avg: Option<(u8, u8, u8)>,
    }

    static GATE: Mutex<SampleGate> = Mutex::new(SampleGate {
        target_key: isize::MIN,
        settle_started: None,
        locked: None,
        last_avg: None,
    });

    pub fn get_mode() -> SampleMode {
        if SAMPLE_MODE.load(Ordering::SeqCst) == MODE_CENTER {
            SampleMode::Center
        } else {
            SampleMode::Edge
        }
    }

    pub fn set_mode(mode: SampleMode) {
        SAMPLE_MODE.store(
            match mode {
                SampleMode::Center => MODE_CENTER,
                SampleMode::Edge => MODE_EDGE,
            },
            Ordering::SeqCst,
        );
        // Mode change → unlock and re-settle.
        if let Ok(mut g) = GATE.lock() {
            g.target_key = isize::MIN;
            g.locked = None;
            g.last_avg = None;
            g.settle_started = None;
        }
        SETTLING.store(true, Ordering::SeqCst);
    }

    /// Watcher: poll often while settling, rarely when locked (only to detect switch).
    pub fn is_settling() -> bool {
        SETTLING.load(Ordering::SeqCst)
    }

    fn target_key(hwnd: Option<HWND>) -> isize {
        hwnd.map(|h| h.0 as isize).unwrap_or(0)
    }

    fn class_name(hwnd: HWND) -> String {
        let mut buf = [0u16; 256];
        let n = unsafe { GetClassNameW(hwnd, &mut buf) };
        String::from_utf16_lossy(&buf[..n as usize])
    }

    fn same_process(hwnd: HWND, other: HWND) -> bool {
        unsafe {
            let mut a = 0u32;
            let mut b = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut a));
            GetWindowThreadProcessId(other, Some(&mut b));
            a != 0 && a == b
        }
    }

    /// In-process HWNDs that ambient MAY sample (plugin OS windows).
    /// Default same-process exclusion skips island/settings/tray — but maximized
    /// `plugin-window` must be a valid 吸色 target like Chrome/VS Code.
    static AMBIENT_ALLOW_HWND: std::sync::atomic::AtomicIsize =
        std::sync::atomic::AtomicIsize::new(0);

    /// Always store / compare **root** HWND (Tauri `.hwnd()` may be a child).
    fn root_hwnd_isize(hwnd_raw: isize) -> isize {
        if hwnd_raw == 0 {
            return 0;
        }
        unsafe {
            let h = HWND(hwnd_raw as *mut _);
            let root = GetAncestor(h, GA_ROOT);
            if root.0.is_null() {
                hwnd_raw
            } else {
                root.0 as isize
            }
        }
    }

    /// Register / clear an in-process window as an ambient sample target.
    /// `top_inset_px` is ignored — all windows use the same top-chrome sample Y.
    pub fn set_ambient_sample_target(hwnd: Option<isize>, _top_inset_px: i32) {
        use std::sync::atomic::Ordering;
        let root = hwnd.map(root_hwnd_isize).filter(|&h| h != 0);
        AMBIENT_ALLOW_HWND.store(root.unwrap_or(0), Ordering::SeqCst);
        // Force settle restart so island leaves a wallpaper lock for this HWND.
        if let Ok(mut gate) = GATE.lock() {
            gate.target_key = isize::MIN;
            gate.settle_started = None;
            gate.locked = None;
            gate.last_avg = None;
        }
        SETTLING.store(true, Ordering::SeqCst);
    }

    fn is_ambient_allowed_hwnd(hwnd: HWND) -> bool {
        use std::sync::atomic::Ordering;
        let allow = AMBIENT_ALLOW_HWND.load(Ordering::SeqCst);
        if allow == 0 {
            return false;
        }
        let raw = hwnd.0 as isize;
        raw == allow || root_hwnd_isize(raw) == allow
    }

    fn is_cloaked(hwnd: HWND) -> bool {
        unsafe {
            let mut cloaked: u32 = 0;
            let ok = DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                &mut cloaked as *mut _ as *mut _,
                std::mem::size_of::<u32>() as u32,
            );
            ok.is_ok() && cloaked != 0
        }
    }

    /// Visible outer bounds (excludes invisible DWM resize borders).
    /// Sampling at GetWindowRect y=0 often hits transparent chrome → wallpaper bleed.
    fn extended_frame_bounds(hwnd: HWND) -> Option<RECT> {
        unsafe {
            let mut r = RECT::default();
            let ok = DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut r as *mut _ as *mut _,
                std::mem::size_of::<RECT>() as u32,
            );
            if ok.is_ok() && r.right > r.left && r.bottom > r.top {
                Some(r)
            } else {
                None
            }
        }
    }

    /// Top of the shell work area (= bottom of island AppBar). Maximized windows
    /// meet the status bar on this screen row.
    fn work_area_top(hint: HWND) -> i32 {
        unsafe {
            let mon = MonitorFromWindow(hint, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(mon, &mut info).as_bool() {
                return info.rcWork.top;
            }
        }
        crate::win32::appbar::strip_height_px().max(1)
    }

    /// AppBar must have reserved the top strip — otherwise screen Y is wrong and
    /// we BitBlt our own (often black/transparent) island → black bar + thrash.
    fn junction_screen_y(island: HWND) -> Option<i32> {
        let strip = crate::win32::appbar::strip_height_px();
        if strip <= 0 {
            return None;
        }
        let meet = work_area_top(island);
        if meet + 2 < strip {
            return None;
        }
        Some(meet + 1)
    }

    fn row_avg_luma(bgra: &[u8]) -> u32 {
        let mut sum = 0u64;
        let mut n = 0u64;
        for c in bgra.chunks_exact(4) {
            // rough luma
            sum += (u64::from(c[2]) * 3 + u64::from(c[1]) * 6 + u64::from(c[0])) / 10;
            n += 1;
        }
        if n == 0 {
            0
        } else {
            (sum / n) as u32
        }
    }

    /// BitBlt/StretchBlt from the desktop DC.
    /// Always covers the full source width, scaled into ≤ MAX_RIBBON_W columns
    /// (left-only crop made icons dominate the ambient strip).
    unsafe fn blit_screen_rows(x: i32, y: i32, src_w: i32, h: i32) -> Option<Vec<u8>> {
        if src_w < 1 || h < 1 {
            return None;
        }
        let src_w = src_w.max(1);
        let dst_w = src_w.min(MAX_RIBBON_W).max(1);
        let hdc_screen = GetDC(None);
        if hdc_screen.is_invalid() {
            return None;
        }
        let hdc_mem = CreateCompatibleDC(hdc_screen);
        if hdc_mem.is_invalid() {
            ReleaseDC(None, hdc_screen);
            return None;
        }
        let hbmp = CreateCompatibleBitmap(hdc_screen, dst_w, h);
        if hbmp.is_invalid() {
            let _ = DeleteDC(hdc_mem);
            ReleaseDC(None, hdc_screen);
            return None;
        }
        let old = SelectObject(hdc_mem, hbmp);
        let ok = if dst_w == src_w {
            BitBlt(hdc_mem, 0, 0, dst_w, h, hdc_screen, x, y, SRCCOPY).is_ok()
        } else {
            let _ = SetStretchBltMode(hdc_mem, HALFTONE);
            StretchBlt(
                hdc_mem,
                0,
                0,
                dst_w,
                h,
                hdc_screen,
                x,
                y,
                src_w,
                h,
                SRCCOPY,
            )
            .as_bool()
        };
        let bgra = if ok {
            dibits_bgra(hdc_mem, hbmp, dst_w, h)
        } else {
            None
        };
        let _ = SelectObject(hdc_mem, old);
        let _ = DeleteObject(hbmp);
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(None, hdc_screen);
        bgra
    }

    fn blit_screen_rows_timed(x: i32, y: i32, src_w: i32, h: i32) -> Option<Vec<u8>> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc;
        static INFLIGHT: AtomicUsize = AtomicUsize::new(0);
        if INFLIGHT.load(Ordering::SeqCst) >= 1 {
            return None;
        }
        INFLIGHT.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("amb-scr".into())
            .spawn(move || {
                let result = unsafe { blit_screen_rows(x, y, src_w, h) };
                let _ = tx.send(result);
                INFLIGHT.fetch_sub(1, Ordering::SeqCst);
            })
            .is_ok();
        if !spawned {
            INFLIGHT.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        match rx.recv_timeout(Duration::from_millis(CAPTURE_TIMEOUT_MS)) {
            Ok(v) => v,
            Err(_) => None,
        }
    }

    /// Optional screen sample at work-area top + 1px.
    /// Skip when Snap Layouts is up (would paint the strip dark).
    unsafe fn capture_junction_row(
        island: HWND,
        target_wr: RECT,
        left_inset: i32,
        vis_w: i32,
    ) -> Option<Vec<u8>> {
        if snap_overlay_visible() {
            return None;
        }
        let screen_y = junction_screen_y(island)?;
        let x0 = (target_wr.left + left_inset).max(target_wr.left);
        let src_w = vis_w.min(target_wr.right - x0).max(1);
        let dst_w = src_w.min(MAX_RIBBON_W).max(1);
        let bgra = blit_screen_rows_timed(x0, screen_y, src_w, TOP_ROWS)?;
        let avg = average_bgra_rows(&bgra, dst_w, TOP_ROWS);
        if is_all_zero(&avg) || row_avg_luma(&avg) < 8 {
            return None;
        }
        Some(avg)
    }

    fn is_snap_class_name(c: &str) -> bool {
        matches!(
            c,
            "XamlExplorerHostIslandWindow" | "SnapAssistFlyout"
        ) || {
            let cl = c.to_ascii_lowercase();
            cl.contains("snapassist") || cl.contains("snap_assist") || cl.contains("snaplayout")
        }
    }

    /// Win11 Snap Layouts / drag-to-top flyout currently on screen.
    fn snap_overlay_visible() -> bool {
        struct Ctx {
            found: bool,
        }
        let ctx = Box::new(Ctx { found: false });
        let ptr = Box::into_raw(ctx);
        unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let ctx = &mut *(lparam.0 as *mut Ctx);
            if ctx.found {
                return BOOL(0);
            }
            if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                return BOOL(1);
            }
            if is_snap_class_name(&class_name(hwnd)) {
                ctx.found = true;
                return BOOL(0);
            }
            BOOL(1)
        }
        unsafe {
            let _ = EnumWindows(Some(cb), LPARAM(ptr as isize));
            let ctx = Box::from_raw(ptr);
            ctx.found
        }
    }

    fn is_excluded(hwnd: HWND, self_hwnd: Option<isize>) -> bool {
        if hwnd.0.is_null() {
            return true;
        }
        if let Some(me) = self_hwnd {
            let mine = HWND(me as *mut _);
            if hwnd.0 as isize == me {
                return true;
            }
            if same_process(hwnd, mine) && !is_ambient_allowed_hwnd(hwnd) {
                return true;
            }
        }
        unsafe {
            if !IsWindow(hwnd).as_bool()
                || !IsWindowVisible(hwnd).as_bool()
                || IsIconic(hwnd).as_bool()
                || is_cloaked(hwnd)
                || crate::win32::hang::is_hung_hwnd(hwnd.0 as isize)
            {
                return true;
            }
        }
        is_snap_class_name(&class_name(hwnd))
            || matches!(
                class_name(hwnd).as_str(),
                "WindowHubAppBarHost"
                    | "WindowHubDockAppBarHost"
                    | "Shell_TrayWnd"
                    | "Shell_SecondaryTrayWnd"
                    | "Progman"
                    | "WorkerW"
                    | "ForegroundStaging"
                    | "Windows.UI.Core.CoreWindow"
            )
    }

    fn root_of(hwnd: HWND) -> HWND {
        unsafe { GetAncestor(hwnd, GA_ROOT) }
    }

    fn covers_monitor(hwnd: HWND) -> bool {
        unsafe {
            let mut wr = RECT::default();
            if GetWindowRect(hwnd, &mut wr).is_err() {
                return false;
            }
            let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(mon, &mut info).as_bool() {
                return false;
            }
            for area in [info.rcMonitor, info.rcWork] {
                let aw = (area.right - area.left).max(1) as i64;
                let ah = (area.bottom - area.top).max(1) as i64;
                let ww = (wr.right - wr.left).max(0) as i64;
                let wh = (wr.bottom - wr.top).max(0) as i64;
                // 85%: borderless "maximize" apps often fall short of 92%.
                if ww * 100 >= aw * 85 && wh * 100 >= ah * 85 {
                    return true;
                }
            }
            false
        }
    }

    /// True fullscreen ≈ fills the physical monitor (`rcMonitor`).
    fn covers_physical_monitor(hwnd: HWND) -> bool {
        unsafe {
            let mut wr = RECT::default();
            if GetWindowRect(hwnd, &mut wr).is_err() {
                return false;
            }
            let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(mon, &mut info).as_bool() {
                return false;
            }
            let m = info.rcMonitor;
            let mw = (m.right - m.left).max(1) as i64;
            let mh = (m.bottom - m.top).max(1) as i64;
            let ww = (wr.right - wr.left).max(0) as i64;
            let wh = (wr.bottom - wr.top).max(0) as i64;
            if ww * 100 < mw * 97 || wh * 100 < mh * 97 {
                return false;
            }
            if (wr.left - m.left).abs() > 4 || (wr.top - m.top).abs() > 4 {
                return false;
            }
            true
        }
    }

    /// Fills the shell work area tightly (borderless maximize under AppBars).
    fn covers_work_area_tight(hwnd: HWND) -> bool {
        unsafe {
            let mut wr = RECT::default();
            if GetWindowRect(hwnd, &mut wr).is_err() {
                return false;
            }
            let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(mon, &mut info).as_bool() {
                return false;
            }
            let w = info.rcWork;
            let aw = (w.right - w.left).max(1) as i64;
            let ah = (w.bottom - w.top).max(1) as i64;
            let ww = (wr.right - wr.left).max(0) as i64;
            let wh = (wr.bottom - wr.top).max(0) as i64;
            if ww * 100 < aw * 97 || wh * 100 < ah * 97 {
                return false;
            }
            if (wr.left - w.left).abs() > 6 || (wr.top - w.top).abs() > 6 {
                return false;
            }
            true
        }
    }

    /// Stable maximize / exclusive fullscreen suitable for ambient sampling.
    /// Excludes Win11 "drag to top" snap preview (large but not IsZoomed).
    fn is_true_maximized_for_ambient(hwnd: HWND) -> bool {
        unsafe {
            if IsZoomed(hwnd).as_bool() {
                return true;
            }
            // Borderless F11 / game-style: no caption, fills monitor or work area.
            let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
            let has_caption = style & WS_CAPTION.0 != 0;
            if has_caption {
                // Caption windows must be IsZoomed — snap-drag looks maximized but isn't.
                return false;
            }
            covers_physical_monitor(hwnd) || covers_work_area_tight(hwnd)
        }
    }

    #[allow(dead_code)]
    fn is_fullscreen_or_maximized(hwnd: HWND) -> bool {
        unsafe { IsZoomed(hwnd).as_bool() || covers_monitor(hwnd) }
    }

    /// Topmost true maximized/fullscreen (EnumWindows = z-order top → bottom).
    /// Skips snap-drag previews so we sample the maximized window underneath.
    fn topmost_fullscreen(self_hwnd: Option<isize>) -> Option<HWND> {
        struct Ctx {
            self_hwnd: Option<isize>,
            /// Prefer IsZoomed; keep first borderless fill as fallback.
            zoomed: Option<HWND>,
            borderless: Option<HWND>,
        }
        let ctx = Box::new(Ctx {
            self_hwnd,
            zoomed: None,
            borderless: None,
        });
        let ptr = Box::into_raw(ctx);

        unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let ctx = &mut *(lparam.0 as *mut Ctx);
            if ctx.zoomed.is_some() {
                return BOOL(0);
            }
            if is_excluded(hwnd, ctx.self_hwnd) {
                return BOOL(1);
            }
            let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            if ex & WS_EX_TOOLWINDOW.0 != 0 {
                return BOOL(1);
            }
            let root = root_of(hwnd);
            if is_excluded(root, ctx.self_hwnd) {
                return BOOL(1);
            }
            if !is_true_maximized_for_ambient(root) {
                return BOOL(1);
            }
            if IsZoomed(root).as_bool() {
                ctx.zoomed = Some(root);
                return BOOL(0);
            }
            if ctx.borderless.is_none() {
                ctx.borderless = Some(root);
            }
            BOOL(1)
        }

        unsafe {
            let _ = EnumWindows(Some(cb), LPARAM(ptr as isize));
            let ctx = Box::from_raw(ptr);
            ctx.zoomed.or(ctx.borderless)
        }
    }

    /// Ambient only tracks true maximize / fullscreen.
    /// Drag-to-top / Snap Layouts → ignore FG, use maximized window below.
    fn pick_target(self_hwnd: Option<isize>) -> Option<HWND> {
        unsafe {
            let fg = root_of(GetForegroundWindow());
            // Snap UI as foreground, or caption window that isn't IsZoomed (drag preview).
            if !is_excluded(fg, self_hwnd) && is_true_maximized_for_ambient(fg) {
                return Some(fg);
            }
        }
        topmost_fullscreen(self_hwnd)
    }

    /// Desktop wallpaper path, or empty if solid-color desktop.
    fn wallpaper_path() -> Option<String> {
        unsafe {
            let mut buf = [0u16; 260];
            let ok = SystemParametersInfoW(
                SPI_GETDESKWALLPAPER,
                buf.len() as u32,
                Some(buf.as_mut_ptr() as *mut _),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
            if ok.is_ok() {
                let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                let path = String::from_utf16_lossy(&buf[..len]);
                if !path.is_empty() && std::path::Path::new(&path).is_file() {
                    return Some(path);
                }
            }
        }
        // Slideshow / some builds expose only the transcoded copy.
        if let Ok(appdata) = std::env::var("APPDATA") {
            let p = std::path::PathBuf::from(appdata)
                .join("Microsoft")
                .join("Windows")
                .join("Themes")
                .join("TranscodedWallpaper");
            if p.is_file() {
                return Some(p.to_string_lossy().into_owned());
            }
        }
        None
    }

    fn desktop_solid_rgb() -> (u8, u8, u8) {
        unsafe {
            let c = GetSysColor(COLOR_DESKTOP);
            ((c & 0xFF) as u8, ((c >> 8) & 0xFF) as u8, ((c >> 16) & 0xFF) as u8)
        }
    }

    struct WallpaperCache {
        key: String,
        rgb: Vec<u8>,
        width: u32,
    }

    static WALLPAPER_CACHE: Mutex<Option<WallpaperCache>> = Mutex::new(None);

    fn load_wallpaper_top_row(path: &str) -> Option<(Vec<u8>, u32)> {
        use image::ImageReader;
        let img = ImageReader::open(path)
            .ok()?
            .with_guessed_format()
            .ok()?
            .decode()
            .ok()?
            .into_rgb8();
        let (iw, ih) = img.dimensions();
        if iw == 0 || ih == 0 {
            return None;
        }
        // Only the top 1–2 image rows (island sits on the desktop top edge).
        let sample_rows = (TOP_ROWS as u32).min(ih).max(1);
        let max_w = 1280u32;
        let step = if iw > max_w {
            ((iw as f64) / max_w as f64).ceil() as u32
        } else {
            1
        };
        let out_w = ((iw + step - 1) / step).max(1);
        let mut rgb = vec![0u8; (out_w as usize) * 3];
        for ox in 0..out_w {
            let x = (ox * step).min(iw - 1);
            let mut sr = 0u32;
            let mut sg = 0u32;
            let mut sb = 0u32;
            for y in 0..sample_rows {
                let p = img.get_pixel(x, y).0;
                sr += p[0] as u32;
                sg += p[1] as u32;
                sb += p[2] as u32;
            }
            let n = sample_rows;
            let o = (ox as usize) * 3;
            rgb[o] = (sr / n) as u8;
            rgb[o + 1] = (sg / n) as u8;
            rgb[o + 2] = (sb / n) as u8;
        }
        Some((rgb, out_w))
    }

    fn wallpaper_cache_key(path: &str) -> String {
        let modified = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("{path}|{modified}")
    }

    /// Sample desktop wallpaper (or solid desktop color) for the island strip.
    fn sample_wallpaper(self_hwnd: Option<isize>) -> Option<AmbientStrip> {
        let me = HWND(self_hwnd? as *mut _);
        let mut mine = RECT::default();
        unsafe {
            GetWindowRect(me, &mut mine).ok()?;
        }

        let mode = get_mode();
        let dpi_scale = unsafe {
            let dpi = GetDpiForWindow(me);
            if dpi == 0 {
                1.0
            } else {
                dpi as f64 / 96.0
            }
        };
        let bar_w = (((mine.right - mine.left) as f64) / dpi_scale).round() as i32;

        let (rgb, width) = if let Some(path) = wallpaper_path() {
            let key = wallpaper_cache_key(&path);
            if let Ok(guard) = WALLPAPER_CACHE.lock() {
                if let Some(c) = guard.as_ref() {
                    if c.key == key {
                        (c.rgb.clone(), c.width)
                    } else {
                        drop(guard);
                        let loaded = load_wallpaper_top_row(&path)?;
                        if let Ok(mut g) = WALLPAPER_CACHE.lock() {
                            *g = Some(WallpaperCache {
                                key,
                                rgb: loaded.0.clone(),
                                width: loaded.1,
                            });
                        }
                        loaded
                    }
                } else {
                    drop(guard);
                    let loaded = load_wallpaper_top_row(&path)?;
                    if let Ok(mut g) = WALLPAPER_CACHE.lock() {
                        *g = Some(WallpaperCache {
                            key,
                            rgb: loaded.0.clone(),
                            width: loaded.1,
                        });
                    }
                    loaded
                }
            } else {
                load_wallpaper_top_row(&path)?
            }
        } else {
            let (r, g, b) = desktop_solid_rgb();
            (vec![r, g, b], 1u32)
        };

        let (avg_r, avg_g, avg_b) = {
            let mut sr = 0u64;
            let mut sg = 0u64;
            let mut sb = 0u64;
            for chunk in rgb.chunks_exact(3) {
                sr += chunk[0] as u64;
                sg += chunk[1] as u64;
                sb += chunk[2] as u64;
            }
            let n = (rgb.len() / 3).max(1) as u64;
            ((sr / n) as u8, (sg / n) as u8, (sb / n) as u8)
        };

        if mode == SampleMode::Center || width <= 1 {
            return Some(AmbientStrip {
                r: avg_r,
                g: avg_g,
                b: avg_b,
                width: 1,
                offset_x: 0,
                span_width: bar_w.max(1),
                png_base64: solid_png_b64(avg_r, avg_g, avg_b),
                hwnd: 0,
                mode,
            });
        }

        // Map island X range onto the wallpaper top-row by monitor fraction.
        let (img_x0, img_x1) = unsafe {
            let mon = MonitorFromWindow(me, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(mon, &mut info).as_bool() {
                let mw = (info.rcMonitor.right - info.rcMonitor.left).max(1) as f64;
                let left = ((mine.left - info.rcMonitor.left) as f64 / mw * width as f64)
                    .floor()
                    .clamp(0.0, (width - 1) as f64) as u32;
                let right = ((mine.right - info.rcMonitor.left) as f64 / mw * width as f64)
                    .ceil()
                    .clamp((left + 1) as f64, width as f64) as u32;
                (left, right)
            } else {
                (0, width)
            }
        };

        let span = (img_x1 - img_x0).max(1);
        let mut slice = vec![0u8; (span as usize) * 3];
        for i in 0..span {
            let src = ((img_x0 + i) as usize) * 3;
            let dst = (i as usize) * 3;
            slice[dst..dst + 3].copy_from_slice(&rgb[src..src + 3]);
        }

        Some(AmbientStrip {
            r: avg_r,
            g: avg_g,
            b: avg_b,
            width: span,
            offset_x: 0,
            span_width: bar_w.max(1),
            png_base64: encode_rgb_row(&slice, span),
            hwnd: 0,
            mode,
        })
    }

    /// After target switch: live-sample for SETTLE_MS (always emit strip), then
    /// lock the last frame. Locked: return `None` until the next window switch.
    pub fn poll_changed(self_hwnd: Option<isize>) -> Option<AmbientStrip> {
        let target = pick_target(self_hwnd);
        let key = target_key(target);

        {
            let mut gate = GATE.lock().ok()?;
            if gate.target_key != key {
                gate.target_key = key;
                gate.settle_started = Some(Instant::now());
                gate.locked = None;
                gate.last_avg = None;
                SETTLING.store(true, Ordering::SeqCst);
            } else if gate.locked.is_some() {
                SETTLING.store(false, Ordering::SeqCst);
                // Locked: no recapture until next switch.
                return None;
            } else {
                SETTLING.store(true, Ordering::SeqCst);
            }
        }

        let strip = if let Some(t) = target {
            capture_edge_ribbon(self_hwnd, Some(t))
        } else {
            sample_wallpaper(self_hwnd)
        };

        let strip = strip?;
        remember(strip.clone());

        let mut gate = GATE.lock().ok()?;
        // Target may have changed mid-sample — only lock if still same.
        if gate.target_key != key {
            return Some(strip);
        }

        gate.last_avg = Some((strip.r, strip.g, strip.b));
        let started = gate.settle_started.get_or_insert_with(Instant::now);
        if started.elapsed() >= Duration::from_millis(SETTLE_MS) {
            // Freeze last live frame — one final emit, then quiet.
            gate.locked = Some(strip.clone());
            SETTLING.store(false, Ordering::SeqCst);
            return Some(strip);
        }

        SETTLING.store(true, Ordering::SeqCst);
        // Settling: always push so the 色带 stays continuously dynamic.
        Some(strip)
    }

    /// Average RGB of the desktop wallpaper top strip (never samples a window HWND).
    #[allow(dead_code)]
    pub fn wallpaper_avg_rgb() -> (u8, u8, u8) {
        if let Some(path) = wallpaper_path() {
            if let Some((rgb, _)) = load_wallpaper_top_row(&path) {
                let mut sr = 0u64;
                let mut sg = 0u64;
                let mut sb = 0u64;
                for chunk in rgb.chunks_exact(3) {
                    sr += u64::from(chunk[0]);
                    sg += u64::from(chunk[1]);
                    sb += u64::from(chunk[2]);
                }
                let n = (rgb.len() / 3).max(1) as u64;
                return ((sr / n) as u8, (sg / n) as u8, (sb / n) as u8);
            }
        }
        desktop_solid_rgb()
    }

    /// Always sample live (settings / first paint). Also seeds the settle gate.
    /// When already locked on the same target, return the frozen strip (no flicker).
    pub fn sample(self_hwnd: Option<isize>) -> AmbientStrip {
        let target = pick_target(self_hwnd);
        let key = target_key(target);
        if let Ok(mut gate) = GATE.lock() {
            if gate.target_key != key {
                gate.target_key = key;
                gate.settle_started = Some(Instant::now());
                gate.locked = None;
                gate.last_avg = None;
            } else if let Some(locked) = gate.locked.clone() {
                SETTLING.store(false, Ordering::SeqCst);
                return locked;
            }
            SETTLING.store(true, Ordering::SeqCst);
        }

        if let Some(t) = target {
            if let Some(strip) = capture_edge_ribbon(self_hwnd, Some(t)) {
                remember(strip.clone());
                if let Ok(mut gate) = GATE.lock() {
                    gate.last_avg = Some((strip.r, strip.g, strip.b));
                }
                return strip;
            }
            return last_strip().unwrap_or_else(AmbientStrip::fallback);
        }
        if let Some(strip) = sample_wallpaper(self_hwnd) {
            remember(strip.clone());
            if let Ok(mut gate) = GATE.lock() {
                gate.last_avg = Some((strip.r, strip.g, strip.b));
            }
            return strip;
        }
        last_strip().unwrap_or_else(AmbientStrip::fallback)
    }

    /// For UI invoke: never BitBlt a foreign HWND on the IPC thread.
    /// Watcher thread owns live capture via `poll_changed` / `sample`.
    pub fn sample_nonblocking(self_hwnd: Option<isize>) -> AmbientStrip {
        let target = pick_target(self_hwnd);
        let key = target_key(target);
        if let Ok(gate) = GATE.lock() {
            if gate.target_key == key {
                if let Some(locked) = gate.locked.clone() {
                    return locked;
                }
            }
        }
        if let Some(strip) = last_strip() {
            return strip;
        }
        // Wallpaper decode is local disk — safe; skip window BitBlt.
        sample_wallpaper(self_hwnd).unwrap_or_else(AmbientStrip::fallback)
    }

    fn encode_rgb_row(rgb: &[u8], width: u32) -> String {
        if width == 0 || rgb.len() < (width as usize) * 3 {
            return solid_png_b64(32, 32, 34);
        }
        let img = match image::RgbImage::from_raw(width, 1, rgb.to_vec()) {
            Some(i) => i,
            None => return solid_png_b64(32, 32, 34),
        };
        let mut buf = Vec::new();
        if image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .is_err()
        {
            return solid_png_b64(32, 32, 34);
        }
        base64::engine::general_purpose::STANDARD.encode(buf)
    }

    /// Read BGRA pixels from an HBITMAP already selected into `hdc_mem`.
    unsafe fn dibits_bgra(hdc_mem: HDC, hbmp: windows::Win32::Graphics::Gdi::HBITMAP, w: i32, h: i32) -> Option<Vec<u8>> {
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bgra = vec![0u8; (w as usize) * (h as usize) * 4];
        let got = GetDIBits(
            hdc_mem,
            hbmp,
            0,
            h as u32,
            Some(bgra.as_mut_ptr() as *mut _),
            &mut info,
            DIB_RGB_COLORS,
        );
        if got == 0 {
            None
        } else {
            Some(bgra)
        }
    }

    /// Window-local top 1–2 px. GetWindowDC only — no PrintWindow (PW of a
    /// maximized frame on the ambient hot path can freeze the process).
    /// Skip hung targets: BitBlt from their window DC can also stall.
    unsafe fn capture_top_pixel_row(
        target: HWND,
        win_w: i32,
        win_h: i32,
        x0: i32,
        ribbon_w: i32,
        y0: i32,
    ) -> Option<Vec<u8>> {
        if crate::win32::hang::is_hung_hwnd(target.0 as isize) {
            return None;
        }
        if win_w < 2 || win_h < 2 || ribbon_w < 1 {
            return None;
        }
        let x0 = x0.clamp(0, win_w - 1);
        let src_w = ribbon_w.min(win_w - x0).max(1);
        let dst_w = src_w.min(MAX_RIBBON_W).max(1);
        let y0 = y0.clamp(0, win_h - 1);
        let rows = TOP_ROWS.min(win_h - y0).max(1);

        let bgra = blit_window_rows_timed(target, x0, y0, src_w, rows, dst_w)?;
        let avg = average_bgra_rows(&bgra, dst_w, rows);
        if is_all_zero(&avg) {
            None
        } else {
            Some(avg)
        }
    }

    /// Average `rows` of BGRA into a single row (per-column).
    fn average_bgra_rows(bgra: &[u8], width: i32, rows: i32) -> Vec<u8> {
        let w = width.max(1) as usize;
        let rows = rows.max(1) as usize;
        let mut out = vec![0u8; w * 4];
        for x in 0..w {
            let mut b = 0u32;
            let mut g = 0u32;
            let mut r = 0u32;
            let mut a = 0u32;
            for row in 0..rows {
                let i = (row * w + x) * 4;
                if i + 3 >= bgra.len() {
                    break;
                }
                b += bgra[i] as u32;
                g += bgra[i + 1] as u32;
                r += bgra[i + 2] as u32;
                a += bgra[i + 3] as u32;
            }
            let n = rows.max(1) as u32;
            let o = x * 4;
            out[o] = (b / n) as u8;
            out[o + 1] = (g / n) as u8;
            out[o + 2] = (r / n) as u8;
            out[o + 3] = (a / n) as u8;
        }
        out
    }

    /// Stretch full `src_w` into `dst_w` columns so the ribbon spans the whole chrome.
    unsafe fn blit_window_rows(
        target: HWND,
        x0: i32,
        y0: i32,
        src_w: i32,
        rows: i32,
        dst_w: i32,
    ) -> Option<Vec<u8>> {
        let hdc_win = GetWindowDC(target);
        if hdc_win.is_invalid() {
            return None;
        }
        let hdc_mem = CreateCompatibleDC(hdc_win);
        if hdc_mem.is_invalid() {
            ReleaseDC(target, hdc_win);
            return None;
        }
        let hbmp = CreateCompatibleBitmap(hdc_win, dst_w, rows);
        if hbmp.is_invalid() {
            let _ = DeleteDC(hdc_mem);
            ReleaseDC(target, hdc_win);
            return None;
        }
        let old = SelectObject(hdc_mem, hbmp);
        let ok = if dst_w == src_w {
            BitBlt(hdc_mem, 0, 0, dst_w, rows, hdc_win, x0, y0, SRCCOPY).is_ok()
        } else {
            let _ = SetStretchBltMode(hdc_mem, HALFTONE);
            StretchBlt(
                hdc_mem,
                0,
                0,
                dst_w,
                rows,
                hdc_win,
                x0,
                y0,
                src_w,
                rows,
                SRCCOPY,
            )
            .as_bool()
        };
        let bgra = if ok {
            dibits_bgra(hdc_mem, hbmp, dst_w, rows)
        } else {
            None
        };
        let _ = SelectObject(hdc_mem, old);
        let _ = DeleteObject(hbmp);
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(target, hdc_win);
        bgra
    }

    /// Run window-DC capture off-thread with a hard timeout so a hung target
    /// cannot freeze ambient (and stall DWM / IPC).
    fn blit_window_rows_timed(
        target: HWND,
        x0: i32,
        y0: i32,
        src_w: i32,
        rows: i32,
        dst_w: i32,
    ) -> Option<Vec<u8>> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc;
        static INFLIGHT: AtomicUsize = AtomicUsize::new(0);
        if crate::win32::hang::is_hung_hwnd(target.0 as isize) {
            return None;
        }
        if INFLIGHT.load(Ordering::SeqCst) >= 1 {
            return None;
        }
        INFLIGHT.fetch_add(1, Ordering::SeqCst);
        let raw = target.0 as isize;
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("amb-blit".into())
            .spawn(move || {
                let result = unsafe {
                    blit_window_rows(HWND(raw as *mut _), x0, y0, src_w, rows, dst_w)
                };
                let _ = tx.send(result);
                INFLIGHT.fetch_sub(1, Ordering::SeqCst);
            })
            .is_ok();
        if !spawned {
            INFLIGHT.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        match rx.recv_timeout(Duration::from_millis(CAPTURE_TIMEOUT_MS)) {
            Ok(v) => v,
            Err(_) => None,
        }
    }

    /// PrintWindow full window; read `rows` starting at window-local `y0`.
    /// Kept for diagnostics — not used on the ambient hot path (can hang).
    #[allow(dead_code)]
    unsafe fn printwindow_top_rows(
        target: HWND,
        win_w: i32,
        win_h: i32,
        x0: i32,
        ribbon_w: i32,
        y0: i32,
        rows: i32,
    ) -> Option<Vec<u8>> {
        const MAX_CAP_W: i32 = 1920;
        let scale_x = if win_w > MAX_CAP_W {
            MAX_CAP_W as f64 / win_w as f64
        } else {
            1.0
        };
        let cap_w = ((win_w as f64) * scale_x).round().max(1.0) as i32;
        let cap_h = win_h;

        let hdc_ref = GetWindowDC(target);
        if hdc_ref.is_invalid() {
            return None;
        }
        let hdc_mem = CreateCompatibleDC(hdc_ref);
        if hdc_mem.is_invalid() {
            ReleaseDC(target, hdc_ref);
            return None;
        }
        let hbmp = CreateCompatibleBitmap(hdc_ref, cap_w, cap_h);
        ReleaseDC(target, hdc_ref);
        if hbmp.is_invalid() {
            let _ = DeleteDC(hdc_mem);
            return None;
        }

        let old = SelectObject(hdc_mem, hbmp);
        // Full window incl. non-client title bar — never PW_CLIENTONLY (that skips chrome).
        let flags_full = PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT);
        let printed = PrintWindow(target, hdc_mem, flags_full).as_bool();

        let full = if printed {
            dibits_bgra(hdc_mem, hbmp, cap_w, cap_h)
        } else {
            None
        };
        let _ = SelectObject(hdc_mem, old);
        let _ = DeleteObject(hbmp);
        let _ = DeleteDC(hdc_mem);
        let full = full?;

        let y0 = y0.clamp(0, win_h.min(cap_h) - 1);
        let rows = rows.clamp(1, (win_h.min(cap_h) - y0).max(1));
        let sx0 = ((x0 as f64) * scale_x).floor() as i32;
        let sx1 = (((x0 + ribbon_w) as f64) * scale_x).ceil() as i32;
        let sx0 = sx0.clamp(0, cap_w - 1);
        let sw = (sx1 - sx0).clamp(1, cap_w - sx0);

        let mut out = vec![0u8; (ribbon_w as usize) * (rows as usize) * 4];
        for row in 0..rows {
            let src_y = (y0 + row).clamp(0, cap_h - 1);
            for col in 0..ribbon_w {
                let src_x = sx0 + (col * sw) / ribbon_w;
                let src_x = src_x.min(sx0 + sw - 1);
                let si = ((src_y * cap_w + src_x) * 4) as usize;
                let di = ((row * ribbon_w + col) * 4) as usize;
                out[di..di + 4].copy_from_slice(&full[si..si + 4]);
            }
        }
        Some(out)
    }

    fn is_all_zero(bgra: &[u8]) -> bool {
        bgra.chunks_exact(4).all(|c| c[0] == 0 && c[1] == 0 && c[2] == 0)
    }

    /// Sample visible top chrome where the window meets the status bar.
    fn capture_edge_ribbon(self_hwnd: Option<isize>, target: Option<HWND>) -> Option<AmbientStrip> {
        unsafe {
            let target = target?;
            let me = HWND(self_hwnd? as *mut _);
            let mut mine = RECT::default();
            GetWindowRect(me, &mut mine).ok()?;

            let mut wr = RECT::default();
            GetWindowRect(target, &mut wr).ok()?;
            let win_w = (wr.right - wr.left).max(1);
            let win_h = (wr.bottom - wr.top).max(1);

            let frame = extended_frame_bounds(target).unwrap_or(wr);
            let top_inset = (frame.top - wr.top).clamp(0, (win_h - 1).max(0));
            let left_inset = (frame.left - wr.left).clamp(0, (win_w - 1).max(0));
            let right_limit = (frame.right - wr.left).clamp(left_inset + 1, win_w);
            let vis_w = (right_limit - left_inset).max(1);

            let dpi_scale = {
                let dpi = GetDpiForWindow(me);
                if dpi == 0 {
                    1.0
                } else {
                    dpi as f64 / 96.0
                }
            };
            let bar_phys = (mine.right - mine.left).max(1);
            let bar_logical = ((bar_phys as f64) / dpi_scale).round() as i32;
            let mode = get_mode();

            let meet = work_area_top(me);
            let y_meet = (meet + 1 - wr.top).clamp(0, win_h - 1);
            let y_inset = (top_inset + 1).clamp(0, win_h - 1);

            let mut bgra: Option<Vec<u8>> = None;
            // At most two Y tries — multi-Y × full-width BitBlt stalls DWM on Alt-Tab.
            for y in [y_meet, y_inset] {
                if let Some(row) =
                    capture_top_pixel_row(target, win_w, win_h, left_inset, vis_w, y)
                {
                    if !is_all_zero(&row) {
                        bgra = Some(row);
                        break;
                    }
                }
            }
            // Timed screen fallback for DWM/GPU chrome (window DC often blank).
            if bgra.is_none() {
                bgra = capture_junction_row(me, wr, left_inset, vis_w);
            }
            let bgra = bgra?;
            let captured_w = ((bgra.len() / 4) as i32).max(1);

            if mode == SampleMode::Center {
                // Sample is already full-width compressed into `captured_w`.
                let pad = ((captured_w as f64) * 0.35).round() as i32;
                let cx0 = pad.clamp(0, captured_w / 3);
                let cx1 = (captured_w - pad).max(cx0 + 1);
                let mut mid = Vec::with_capacity(((cx1 - cx0) as usize) * 4);
                for x in cx0..cx1 {
                    let i = (x as usize) * 4;
                    if i + 3 < bgra.len() {
                        mid.extend_from_slice(&bgra[i..i + 4]);
                    }
                }
                let (r, g, b) = robust_rgb_from_bgra(&mid);
                return Some(AmbientStrip {
                    r,
                    g,
                    b,
                    width: 1,
                    offset_x: 0,
                    span_width: bar_logical.max(1),
                    png_base64: solid_png_b64(r, g, b),
                    hwnd: target.0 as isize,
                    mode,
                });
            }

            // Edge: bar column → proportional X in full-width stretched sample.
            let out_w = bar_logical.clamp(64, 1280) as u32;
            let mut rgb = vec![0u8; (out_w as usize) * 3];
            for ox in 0..out_w {
                let t = (ox as f64 + 0.5) / out_w as f64;
                let screen_x = mine.left as f64 + t * bar_phys as f64;
                let wx = (screen_x - wr.left as f64).round() as i32;
                let local = (wx - left_inset).clamp(0, vis_w - 1);
                let src_x = if vis_w > 1 {
                    ((local as i64 * (captured_w as i64 - 1)) / (vis_w as i64 - 1).max(1)) as i32
                } else {
                    0
                }
                .clamp(0, captured_w - 1);
                let src = (src_x as usize) * 4;
                let dst = (ox as usize) * 3;
                if src + 2 < bgra.len() {
                    rgb[dst] = bgra[src + 2];
                    rgb[dst + 1] = bgra[src + 1];
                    rgb[dst + 2] = bgra[src];
                }
            }

            let (avg_r, avg_g, avg_b) = {
                let mut sr = 0u64;
                let mut sg = 0u64;
                let mut sb = 0u64;
                for chunk in rgb.chunks_exact(3) {
                    sr += chunk[0] as u64;
                    sg += chunk[1] as u64;
                    sb += chunk[2] as u64;
                }
                let n = (rgb.len() / 3).max(1) as u64;
                ((sr / n) as u8, (sg / n) as u8, (sb / n) as u8)
            };

            Some(AmbientStrip {
                r: avg_r,
                g: avg_g,
                b: avg_b,
                width: out_w,
                offset_x: 0,
                span_width: bar_logical.max(1),
                png_base64: encode_rgb_row(&rgb, out_w),
                hwnd: target.0 as isize,
                mode,
            })
        }
    }

    #[allow(dead_code)]
    fn blur_rgb_row_3(rgb: &mut [u8]) {
        let n = rgb.len() / 3;
        if n < 3 {
            return;
        }
        let src = rgb.to_vec();
        for i in 0..n {
            let i0 = i.saturating_sub(1);
            let i2 = (i + 1).min(n - 1);
            for c in 0..3 {
                let v = src[i0 * 3 + c] as u16
                    + src[i * 3 + c] as u16
                    + src[i2 * 3 + c] as u16;
                rgb[i * 3 + c] = (v / 3) as u8;
            }
        }
    }

    /// Median RGB after dropping outer samples — resists edge shadows / outliers.
    fn robust_rgb_from_bgra(bgra: &[u8]) -> (u8, u8, u8) {
        let n = bgra.len() / 4;
        if n == 0 {
            return (32, 32, 34);
        }
        let margin = ((n as f64) * 0.1).round() as usize;
        let start = margin.min(n.saturating_sub(1) / 4);
        let end = (n - margin).max(start + 1);

        let mut rs = Vec::with_capacity(end - start);
        let mut gs = Vec::with_capacity(end - start);
        let mut bs = Vec::with_capacity(end - start);
        for i in start..end {
            let o = i * 4;
            if o + 2 >= bgra.len() {
                break;
            }
            bs.push(bgra[o]);
            gs.push(bgra[o + 1]);
            rs.push(bgra[o + 2]);
        }
        let med = |v: &mut [u8]| -> u8 {
            if v.is_empty() {
                return 32;
            }
            v.sort_unstable();
            v[v.len() / 2]
        };
        (med(&mut rs), med(&mut gs), med(&mut bs))
    }

    fn remember(strip: AmbientStrip) {
        if let Ok(mut g) = LAST_STRIP.lock() {
            *g = Some(strip);
        }
    }

    fn last_strip() -> Option<AmbientStrip> {
        LAST_STRIP.lock().ok().and_then(|g| g.clone())
    }
}

#[cfg(windows)]
pub use win::{
    get_mode, is_settling, poll_changed, sample, sample_nonblocking, set_ambient_sample_target,
    set_mode,
};

#[cfg(not(windows))]
pub fn set_ambient_sample_target(_hwnd: Option<isize>, _top_inset_px: i32) {}

#[cfg(not(windows))]
pub fn sample(_self_hwnd: Option<isize>) -> AmbientStrip {
    AmbientStrip::fallback()
}

#[cfg(not(windows))]
pub fn sample_nonblocking(_self_hwnd: Option<isize>) -> AmbientStrip {
    AmbientStrip::fallback()
}

#[cfg(not(windows))]
pub fn poll_changed(_self_hwnd: Option<isize>) -> Option<AmbientStrip> {
    None
}

#[cfg(not(windows))]
pub fn is_settling() -> bool {
    false
}

#[cfg(not(windows))]
pub fn get_mode() -> SampleMode {
    SampleMode::Edge
}

#[cfg(not(windows))]
pub fn set_mode(_mode: SampleMode) {}
