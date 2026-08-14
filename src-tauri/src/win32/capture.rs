//! Capture a window as JPEG bytes (for dock previews / ECS).

#[derive(Debug, Clone, Copy)]
pub struct Roi {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// When true, ignore x/y/w/h and use full client area.
    pub use_full: bool,
}

impl Default for Roi {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            use_full: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CapturedFrame {
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Capture window as JPEG. Pass `max_edge = 0` to keep native size.
#[cfg(windows)]
pub fn capture_window_jpeg(hwnd_raw: isize, roi: Roi) -> Result<CapturedFrame, String> {
    capture_window_jpeg_scaled(hwnd_raw, roi, 0)
}

/// Capture window as JPEG, optionally downscaling so the long edge ≤ `max_edge`.
///
/// Prefer **screen BitBlt** of the visible window bounds — PrintWindow often
/// returns a successful black frame for Chromium / GPU-composited apps.
/// Falls back to a brief topmost peek when covered Chromium windows refuse both.
#[cfg(windows)]
pub fn capture_window_jpeg_scaled(
    hwnd_raw: isize,
    roi: Roi,
    max_edge: u32,
) -> Result<CapturedFrame, String> {
    capture_window_jpeg_scaled_ex(hwnd_raw, roi, max_edge, true)
}

/// Dock preview fast path: optional peek + short-lived cache.
#[cfg(windows)]
pub fn capture_window_jpeg_cached(
    hwnd_raw: isize,
    max_edge: u32,
    allow_peek: bool,
) -> Result<CapturedFrame, String> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    struct Entry {
        at: Instant,
        frame: CapturedFrame,
    }
    static CACHE: std::sync::OnceLock<Mutex<HashMap<isize, Entry>>> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    if let Ok(guard) = cache.lock() {
        if let Some(e) = guard.get(&hwnd_raw) {
            if e.at.elapsed() < Duration::from_millis(1200) {
                return Ok(e.frame.clone());
            }
        }
    }

    let frame = capture_window_jpeg_scaled_ex(
        hwnd_raw,
        Roi {
            use_full: true,
            ..Default::default()
        },
        max_edge,
        allow_peek,
    )?;
    if let Ok(mut guard) = cache.lock() {
        guard.insert(
            hwnd_raw,
            Entry {
                at: Instant::now(),
                frame: frame.clone(),
            },
        );
        // Bound cache size.
        if guard.len() > 48 {
            let cutoff = Instant::now() - Duration::from_secs(3);
            guard.retain(|_, e| e.at > cutoff);
        }
    }
    Ok(frame)
}

#[cfg(windows)]
fn capture_window_jpeg_scaled_ex(
    hwnd_raw: isize,
    _roi: Roi,
    max_edge: u32,
    allow_peek: bool,
) -> Result<CapturedFrame, String> {
    use image::{ImageBuffer, Rgb};
    use std::io::Cursor;
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        ReleaseDC, SelectObject, SRCCOPY,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetWindowRect, IsIconic, IsWindow, IsWindowVisible, SetWindowPos,
        HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };

    const PW_CLIENTONLY: u32 = 0x1;
    const PW_RENDERFULLCONTENT: u32 = 0x2;

    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return Err("Invalid window".into());
        }
        if IsIconic(hwnd).as_bool() {
            return Err("window minimized".into());
        }
        if !IsWindowVisible(hwnd).as_bool() {
            return Err("window not visible".into());
        }

        // Prefer extended frame bounds (excludes DWM shadow) for screen blit.
        let mut frame = RECT::default();
        let frame_ok = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut frame as *mut RECT as *mut _,
            std::mem::size_of::<RECT>() as u32,
        )
        .is_ok();
        if !frame_ok {
            let _ = GetWindowRect(hwnd, &mut frame);
        }
        let fw = (frame.right - frame.left).max(1);
        let fh = (frame.bottom - frame.top).max(1);

        let fg = windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow();
        let target_is_fg = fg == hwnd || {
            use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
            let mut a = 0u32;
            let mut b = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut a));
            GetWindowThreadProcessId(fg, Some(&mut b));
            a != 0 && a == b
        };

        let try_bitblt = || -> Option<(Vec<u8>, u32, u32)> {
            let hdc_screen = GetDC(HWND::default());
            if hdc_screen.is_invalid() {
                return None;
            }
            let hdc_mem = CreateCompatibleDC(hdc_screen);
            if hdc_mem.is_invalid() {
                ReleaseDC(HWND::default(), hdc_screen);
                return None;
            }
            let hbmp = CreateCompatibleBitmap(hdc_screen, fw, fh);
            if hbmp.is_invalid() {
                let _ = DeleteDC(hdc_mem);
                ReleaseDC(HWND::default(), hdc_screen);
                return None;
            }
            let old = SelectObject(hdc_mem, hbmp);
            let ok = BitBlt(
                hdc_mem,
                0,
                0,
                fw,
                fh,
                hdc_screen,
                frame.left,
                frame.top,
                SRCCOPY,
            )
            .is_ok();
            let mut out = None;
            if ok {
                if let Some(rgb) = dib_to_rgb(hdc_mem, hbmp, fw, fh) {
                    if !is_mostly_blank(&rgb, fw as u32, fh as u32) {
                        out = Some((rgb, fw as u32, fh as u32));
                    }
                }
            }
            SelectObject(hdc_mem, old);
            let _ = DeleteObject(hbmp);
            let _ = DeleteDC(hdc_mem);
            ReleaseDC(HWND::default(), hdc_screen);
            out
        };

        let try_print = || -> Option<(Vec<u8>, u32, u32)> {
            let mut rect = RECT::default();
            if GetClientRect(hwnd, &mut rect).is_err() {
                return None;
            }
            let cw = (rect.right - rect.left).max(1);
            let ch = (rect.bottom - rect.top).max(1);
            let hdc_win = GetDC(hwnd);
            if hdc_win.is_invalid() {
                return None;
            }
            let hdc_mem = CreateCompatibleDC(hdc_win);
            if hdc_mem.is_invalid() {
                ReleaseDC(hwnd, hdc_win);
                return None;
            }
            let hbmp = CreateCompatibleBitmap(hdc_win, cw, ch);
            if hbmp.is_invalid() {
                let _ = DeleteDC(hdc_mem);
                ReleaseDC(hwnd, hdc_win);
                return None;
            }
            let old = SelectObject(hdc_mem, hbmp);
            let ok =
                PrintWindow(hwnd, hdc_mem, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool();
            if !ok {
                let _ = PrintWindow(
                    hwnd,
                    hdc_mem,
                    PRINT_WINDOW_FLAGS(PW_CLIENTONLY | PW_RENDERFULLCONTENT),
                );
            }
            let mut out = None;
            if let Some(rgb) = dib_to_rgb(hdc_mem, hbmp, cw, ch) {
                if !is_mostly_blank(&rgb, cw as u32, ch as u32) {
                    out = Some((rgb, cw as u32, ch as u32));
                }
            }
            SelectObject(hdc_mem, old);
            let _ = DeleteObject(hbmp);
            let _ = DeleteDC(hdc_mem);
            ReleaseDC(hwnd, hdc_win);
            out
        };

        // Prefer BitBlt first (fast, matches taskbar-ish screen snapshot for visible wins).
        let mut chosen = try_bitblt().or_else(try_print);

        // Optional peek — slow; dock preview skips this by default.
        if chosen.is_none() && allow_peek && !target_is_fg {
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            std::thread::sleep(std::time::Duration::from_millis(16));
            chosen = try_bitblt().or_else(try_print);
            let _ = SetWindowPos(
                hwnd,
                HWND_NOTOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }

        let (rgb, w, h) = chosen.ok_or_else(|| "capture failed".to_string())?;
        if is_mostly_blank(&rgb, w, h) {
            return Err("capture blank".into());
        }

        let img: ImageBuffer<Rgb<u8>, _> =
            ImageBuffer::from_raw(w, h, rgb).ok_or("ImageBuffer failed")?;

        let (out_img, out_w, out_h) = if max_edge > 0 {
            let src_max = w.max(h);
            if src_max > max_edge {
                let scale = max_edge as f64 / src_max as f64;
                let nw = ((w as f64) * scale).round().max(1.0) as u32;
                let nh = ((h as f64) * scale).round().max(1.0) as u32;
                let resized =
                    image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle);
                (resized, nw, nh)
            } else {
                (img, w, h)
            }
        } else {
            (img, w, h)
        };

        let mut cursor = Cursor::new(Vec::new());
        {
            use image::codecs::jpeg::JpegEncoder;
            use image::ImageEncoder;
            let q = if max_edge > 0 && max_edge <= 280 { 72 } else { 62 };
            let enc = JpegEncoder::new_with_quality(&mut cursor, q);
            enc.write_image(
                out_img.as_raw(),
                out_w,
                out_h,
                image::ExtendedColorType::Rgb8,
            )
            .map_err(|e| format!("JPEG encode: {e}"))?;
        }

        Ok(CapturedFrame {
            jpeg: cursor.into_inner(),
            width: out_w,
            height: out_h,
        })
    }
}

#[cfg(not(windows))]
pub fn capture_window_jpeg_cached(
    _hwnd_raw: isize,
    _max_edge: u32,
    _allow_peek: bool,
) -> Result<CapturedFrame, String> {
    Err("Windows only".into())
}

#[cfg(windows)]
unsafe fn dib_to_rgb(
    hdc_mem: windows::Win32::Graphics::Gdi::HDC,
    hbmp: windows::Win32::Graphics::Gdi::HBITMAP,
    w: i32,
    h: i32,
) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::Gdi::{
        GetDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    unsafe {
        let mut bmi = BITMAPINFO {
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
        let lines = GetDIBits(
            hdc_mem,
            hbmp,
            0,
            h as u32,
            Some(bgra.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );
        if lines == 0 {
            return None;
        }
        let mut rgb = vec![0u8; (w as usize) * (h as usize) * 3];
        for i in 0..(w as usize * h as usize) {
            let si = i * 4;
            let di = i * 3;
            rgb[di] = bgra[si + 2];
            rgb[di + 1] = bgra[si + 1];
            rgb[di + 2] = bgra[si];
        }
        Some(rgb)
    }
}

/// True when the frame is almost entirely black or white (failed / empty blit).
/// Near-white matters: BitBlt of a covered region or failed path often yields a
/// washed-out slab that used to ship as a blank dock preview card.
fn is_mostly_blank(rgb: &[u8], w: u32, h: u32) -> bool {
    let n = (w as usize).saturating_mul(h as usize);
    if n == 0 || rgb.len() < n * 3 {
        return true;
    }
    let step = ((n / 400).max(1)) * 3;
    let mut dark = 0usize;
    let mut bright = 0usize;
    let mut counted = 0usize;
    let mut i = 0usize;
    while i + 2 < rgb.len() {
        let r = rgb[i] as u32;
        let g = rgb[i + 1] as u32;
        let b = rgb[i + 2] as u32;
        let sum = r + g + b;
        if sum < 36 {
            dark += 1;
        } else if sum > 750 {
            bright += 1;
        }
        counted += 1;
        i += step;
    }
    counted > 0 && (dark * 100 / counted >= 92 || bright * 100 / counted >= 92)
}

#[cfg(not(windows))]
pub fn capture_window_jpeg(_hwnd_raw: isize, _roi: Roi) -> Result<CapturedFrame, String> {
    Err("Windows only".into())
}

#[cfg(not(windows))]
pub fn capture_window_jpeg_scaled(
    _hwnd_raw: isize,
    _roi: Roi,
    _max_edge: u32,
) -> Result<CapturedFrame, String> {
    Err("Windows only".into())
}
