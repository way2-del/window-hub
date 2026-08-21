//! Capture a window as JPEG bytes (prefer on-screen pixels so GPU apps aren't blank).

#[derive(Debug, Clone, Copy)]
pub struct Roi {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// When true, ignore x/y/w/h and use full window area.
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

#[cfg(windows)]
fn bgra_is_blank(bgra: &[u8]) -> bool {
    if bgra.len() < 16 {
        return true;
    }
    // Sample a grid; treat near-white OR all-zero as failed capture.
    let px = bgra.len() / 4;
    let step = (px / 64).max(1);
    let mut sum = 0u64;
    let mut nonzero = 0u32;
    let mut n = 0u32;
    for i in (0..px).step_by(step) {
        let o = i * 4;
        let b = bgra[o] as u64;
        let g = bgra[o + 1] as u64;
        let r = bgra[o + 2] as u64;
        sum += r + g + b;
        if r | g | b != 0 {
            nonzero += 1;
        }
        n += 1;
    }
    if n == 0 || nonzero < n / 20 {
        return true;
    }
    let avg = (sum / (n as u64 * 3)) as u32;
    // PrintWindow often yields flat white (~255) on GPU-composited HWNDs.
    avg >= 248
}

/// GetWindowDC / StretchBlt on DWM-GPU windows often returns a near-black frame
/// that is not "blank" by the white/zero checks above.
#[cfg(windows)]
fn bgra_is_mostly_black(bgra: &[u8]) -> bool {
    if bgra.len() < 16 {
        return true;
    }
    let px = bgra.len() / 4;
    let step = (px / 96).max(1);
    let mut sum = 0u64;
    let mut bright = 0u32;
    let mut n = 0u32;
    for i in (0..px).step_by(step) {
        let o = i * 4;
        let b = bgra[o] as u64;
        let g = bgra[o + 1] as u64;
        let r = bgra[o + 2] as u64;
        // Rec.601-ish luma without divide until the end.
        let y = (r * 30 + g * 59 + b * 11) / 100;
        sum += y;
        if y >= 40 {
            bright += 1;
        }
        n += 1;
    }
    if n == 0 {
        return true;
    }
    let avg = sum / n as u64;
    // Near-black with almost no mid/highlight samples → failed GPU blit.
    avg <= 14 && bright * 20 < n
}

#[cfg(windows)]
fn bgra_looks_usable(bgra: &[u8]) -> bool {
    !bgra_is_blank(bgra) && !bgra_is_mostly_black(bgra)
}

#[cfg(windows)]
unsafe fn dibits_bgra(
    hdc: windows::Win32::Graphics::Gdi::HDC,
    hbmp: windows::Win32::Graphics::Gdi::HBITMAP,
    w: i32,
    h: i32,
) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::Gdi::{
        GetDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
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
        hdc,
        hbmp,
        0,
        h as u32,
        Some(bgra.as_mut_ptr() as *mut _),
        &mut bmi,
        DIB_RGB_COLORS,
    );
    if lines == 0 {
        None
    } else {
        Some(bgra)
    }
}

#[cfg(windows)]
unsafe fn capture_screen_bgra(left: i32, top: i32, w: i32, h: i32) -> Option<Vec<u8>> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        ReleaseDC, SelectObject, CAPTUREBLT, SRCCOPY,
    };
    let hdc_screen = GetDC(HWND::default());
    if hdc_screen.is_invalid() {
        return None;
    }
    let hdc_mem = CreateCompatibleDC(hdc_screen);
    if hdc_mem.is_invalid() {
        ReleaseDC(HWND::default(), hdc_screen);
        return None;
    }
    let hbmp = CreateCompatibleBitmap(hdc_screen, w, h);
    if hbmp.is_invalid() {
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(HWND::default(), hdc_screen);
        return None;
    }
    let old = SelectObject(hdc_mem, hbmp);
    let rop = SRCCOPY | CAPTUREBLT;
    let ok = BitBlt(hdc_mem, 0, 0, w, h, hdc_screen, left, top, rop).is_ok();
    let bgra = if ok {
        dibits_bgra(hdc_mem, hbmp, w, h)
    } else {
        None
    };
    let _ = SelectObject(hdc_mem, old);
    let _ = DeleteObject(hbmp);
    let _ = DeleteDC(hdc_mem);
    ReleaseDC(HWND::default(), hdc_screen);
    bgra
}

#[cfg(windows)]
unsafe fn capture_windowdc_bgra(hwnd: windows::Win32::Foundation::HWND, w: i32, h: i32) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetWindowDC,
        ReleaseDC, SelectObject, SRCCOPY,
    };
    let hdc_win = GetWindowDC(hwnd);
    if hdc_win.is_invalid() {
        return None;
    }
    let hdc_mem = CreateCompatibleDC(hdc_win);
    if hdc_mem.is_invalid() {
        ReleaseDC(hwnd, hdc_win);
        return None;
    }
    let hbmp = CreateCompatibleBitmap(hdc_win, w, h);
    if hbmp.is_invalid() {
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(hwnd, hdc_win);
        return None;
    }
    let old = SelectObject(hdc_mem, hbmp);
    let ok = BitBlt(hdc_mem, 0, 0, w, h, hdc_win, 0, 0, SRCCOPY).is_ok();
    let bgra = if ok {
        dibits_bgra(hdc_mem, hbmp, w, h)
    } else {
        None
    };
    let _ = SelectObject(hdc_mem, old);
    let _ = DeleteObject(hbmp);
    let _ = DeleteDC(hdc_mem);
    ReleaseDC(hwnd, hdc_win);
    bgra
}

#[cfg(windows)]
unsafe fn capture_printwindow_bgra(
    hwnd: windows::Win32::Foundation::HWND,
    w: i32,
    h: i32,
) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, ReleaseDC,
        SelectObject,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    const PW_CLIENTONLY: u32 = 0x1;
    const PW_RENDERFULLCONTENT: u32 = 0x2;

    let try_flags = |flags: u32| -> Option<Vec<u8>> {
        let hdc_win = GetDC(hwnd);
        if hdc_win.is_invalid() {
            return None;
        }
        let hdc_mem = CreateCompatibleDC(hdc_win);
        if hdc_mem.is_invalid() {
            ReleaseDC(hwnd, hdc_win);
            return None;
        }
        let hbmp = CreateCompatibleBitmap(hdc_win, w, h);
        if hbmp.is_invalid() {
            let _ = DeleteDC(hdc_mem);
            ReleaseDC(hwnd, hdc_win);
            return None;
        }
        let old = SelectObject(hdc_mem, hbmp);
        let printed = PrintWindow(hwnd, hdc_mem, PRINT_WINDOW_FLAGS(flags)).as_bool();
        let bgra = if printed {
            dibits_bgra(hdc_mem, hbmp, w, h)
        } else {
            None
        };
        let _ = SelectObject(hdc_mem, old);
        let _ = DeleteObject(hbmp);
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(hwnd, hdc_win);
        bgra
    };

    // Full content first (GPU / Chromium); client-only as second attempt.
    // Never return an unfiltered PrintWindow buffer — flat white looks "successful".
    try_flags(PW_RENDERFULLCONTENT)
        .filter(|b| bgra_looks_usable(b))
        .or_else(|| {
            try_flags(PW_CLIENTONLY | PW_RENDERFULLCONTENT).filter(|b| bgra_looks_usable(b))
        })
        .or_else(|| try_flags(0).filter(|b| bgra_looks_usable(b)))
}

/// PrintWindow can block forever on a hung target. Run it off-thread with a short
/// timeout so Dock/ECS/ambient never freeze Window Hub's IPC (UI "未响应").
#[cfg(windows)]
fn capture_printwindow_bgra_timed(
    hwnd: windows::Win32::Foundation::HWND,
    w: i32,
    h: i32,
) -> Option<Vec<u8>> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;
    use windows::Win32::Foundation::HWND;

    static PW_INFLIGHT: AtomicUsize = AtomicUsize::new(0);
    const PW_MAX: usize = 2;
    const PW_TIMEOUT_MS: u64 = 180;

    let raw = hwnd.0 as isize;
    if crate::win32::hang::is_hung_hwnd(raw) {
        return None;
    }
    if PW_INFLIGHT.load(Ordering::SeqCst) >= PW_MAX {
        return None;
    }
    PW_INFLIGHT.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("pw-cap".into())
        .spawn(move || {
            let result = unsafe { capture_printwindow_bgra(HWND(raw as *mut _), w, h) };
            let _ = tx.send(result);
            PW_INFLIGHT.fetch_sub(1, Ordering::SeqCst);
        })
        .is_ok();
    if !spawned {
        PW_INFLIGHT.fetch_sub(1, Ordering::SeqCst);
        return None;
    }
    match rx.recv_timeout(Duration::from_millis(PW_TIMEOUT_MS)) {
        Ok(v) => v,
        Err(_) => {
            // Worker still blocked on PrintWindow — slot stays until it returns.
            None
        }
    }
}

#[cfg(windows)]
fn encode_jpeg_bgra(bgra: &[u8], full_w: i32, full_h: i32, roi: Roi) -> Result<CapturedFrame, String> {
    use image::{ImageBuffer, ImageFormat, Rgb};
    use std::io::Cursor;

    let (cx, cy, cw, ch) = if roi.use_full || roi.w <= 0 || roi.h <= 0 {
        (0, 0, full_w, full_h)
    } else {
        let x = roi.x.clamp(0, full_w - 1);
        let y = roi.y.clamp(0, full_h - 1);
        let w = roi.w.clamp(1, full_w - x);
        let h = roi.h.clamp(1, full_h - y);
        (x, y, w, h)
    };

    let mut rgb = vec![0u8; (cw as usize) * (ch as usize) * 3];
    for row in 0..ch as usize {
        let src_y = (cy as usize) + row;
        for col in 0..cw as usize {
            let src_x = (cx as usize) + col;
            let si = (src_y * full_w as usize + src_x) * 4;
            let di = (row * cw as usize + col) * 3;
            rgb[di] = bgra[si + 2];
            rgb[di + 1] = bgra[si + 1];
            rgb[di + 2] = bgra[si];
        }
    }

    let img: ImageBuffer<Rgb<u8>, _> =
        ImageBuffer::from_raw(cw as u32, ch as u32, rgb).ok_or("ImageBuffer failed")?;
    let mut cursor = Cursor::new(Vec::new());
    img.write_to(&mut cursor, ImageFormat::Jpeg)
        .map_err(|e| format!("JPEG encode: {e}"))?;

    Ok(CapturedFrame {
        jpeg: cursor.into_inner(),
        width: cw as u32,
        height: ch as u32,
    })
}

/// Convert BGRA → RGB, optionally downscale, encode JPEG once (no decode round-trip).
#[cfg(windows)]
fn encode_jpeg_bgra_thumb(
    bgra: &[u8],
    full_w: i32,
    full_h: i32,
    max_w: u32,
    max_h: u32,
) -> Result<CapturedFrame, String> {
    use image::{imageops, ImageBuffer, Rgb};
    use std::io::Cursor;

    let cw = full_w.max(1) as u32;
    let ch = full_h.max(1) as u32;
    let mut rgb = vec![0u8; (cw as usize) * (ch as usize) * 3];
    for row in 0..ch as usize {
        for col in 0..cw as usize {
            let si = (row * cw as usize + col) * 4;
            let di = (row * cw as usize + col) * 3;
            rgb[di] = bgra[si + 2];
            rgb[di + 1] = bgra[si + 1];
            rgb[di + 2] = bgra[si];
        }
    }
    let img: ImageBuffer<Rgb<u8>, _> =
        ImageBuffer::from_raw(cw, ch, rgb).ok_or("ImageBuffer failed")?;
    let max_w = max_w.max(1);
    let max_h = max_h.max(1);
    let scale = (max_w as f64 / cw as f64)
        .min(max_h as f64 / ch as f64)
        .min(1.0);
    let tw = ((cw as f64) * scale).round().max(1.0) as u32;
    let th = ((ch as f64) * scale).round().max(1.0) as u32;
    // Lanczos stays sharper than `thumbnail` (triangle) when downscaling large windows.
    let thumb = if tw == cw && th == ch {
        img
    } else {
        imageops::resize(&img, tw, th, imageops::FilterType::Lanczos3)
    };
    let (tw, th) = (thumb.width(), thumb.height());
    let mut cursor = Cursor::new(Vec::new());
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, 88);
    enc.encode(thumb.as_raw(), tw, th, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("JPEG encode: {e}"))?;
    Ok(CapturedFrame {
        jpeg: cursor.into_inner(),
        width: tw,
        height: th,
    })
}

/// Stretch-blit window DC into a small bitmap (much faster than full-size PrintWindow).
#[cfg(windows)]
unsafe fn capture_windowdc_thumb_bgra(
    hwnd: windows::Win32::Foundation::HWND,
    src_w: i32,
    src_h: i32,
    dst_w: i32,
    dst_h: i32,
) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetWindowDC,
        ReleaseDC, SelectObject, SetStretchBltMode, StretchBlt, HALFTONE, SRCCOPY,
    };
    let hdc_win = GetWindowDC(hwnd);
    if hdc_win.is_invalid() {
        return None;
    }
    let hdc_mem = CreateCompatibleDC(hdc_win);
    if hdc_mem.is_invalid() {
        ReleaseDC(hwnd, hdc_win);
        return None;
    }
    let hbmp = CreateCompatibleBitmap(hdc_win, dst_w, dst_h);
    if hbmp.is_invalid() {
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(hwnd, hdc_win);
        return None;
    }
    let old = SelectObject(hdc_mem, hbmp);
    let _ = SetStretchBltMode(hdc_mem, HALFTONE);
    let ok = StretchBlt(
        hdc_mem,
        0,
        0,
        dst_w,
        dst_h,
        hdc_win,
        0,
        0,
        src_w,
        src_h,
        SRCCOPY,
    )
    .as_bool();
    let bgra = if ok {
        dibits_bgra(hdc_mem, hbmp, dst_w, dst_h)
    } else {
        None
    };
    let _ = SelectObject(hdc_mem, old);
    let _ = DeleteObject(hbmp);
    let _ = DeleteDC(hdc_mem);
    ReleaseDC(hwnd, hdc_win);
    bgra
}

#[cfg(windows)]
pub fn capture_window_jpeg(hwnd_raw: isize, roi: Roi) -> Result<CapturedFrame, String> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        ReleaseDC, SelectObject, SRCCOPY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetWindowRect, IsWindow};

    if crate::win32::hang::is_hung_hwnd(hwnd_raw) {
        return Err("window hung".into());
    }

    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return Err("Invalid window".into());
        }

        // Full-frame: capture on-screen window chrome+client (matches GetWindowRect).
        if roi.use_full || roi.w <= 0 || roi.h <= 0 {
            let mut rect = RECT::default();
            GetWindowRect(hwnd, &mut rect).map_err(|e| format!("GetWindowRect: {e}"))?;
            let full_w = (rect.right - rect.left).max(1);
            let full_h = (rect.bottom - rect.top).max(1);

            let mut bgra = capture_screen_bgra(rect.left, rect.top, full_w, full_h)
                .filter(|b| !bgra_is_blank(b));
            if bgra.is_none() {
                bgra = capture_windowdc_bgra(hwnd, full_w, full_h).filter(|b| !bgra_is_blank(b));
            }
            if bgra.is_none() {
                bgra =
                    capture_printwindow_bgra_timed(hwnd, full_w, full_h).filter(|b| !bgra_is_blank(b));
            }
            let bgra = bgra.ok_or_else(|| "window capture blank".to_string())?;
            return encode_jpeg_bgra(&bgra, full_w, full_h, Roi::default());
        }

        // ROI path (ECS): prefer BitBlt; timed PrintWindow only as fallback.
        let mut rect = RECT::default();
        GetClientRect(hwnd, &mut rect).map_err(|e| format!("GetClientRect: {e}"))?;
        let full_w = (rect.right - rect.left).max(1);
        let full_h = (rect.bottom - rect.top).max(1);

        let hdc_win = GetDC(hwnd);
        if hdc_win.is_invalid() {
            return Err("GetDC failed".into());
        }
        let hdc_mem = CreateCompatibleDC(hdc_win);
        if hdc_mem.is_invalid() {
            ReleaseDC(hwnd, hdc_win);
            return Err("CreateCompatibleDC failed".into());
        }
        let hbmp = CreateCompatibleBitmap(hdc_win, full_w, full_h);
        if hbmp.is_invalid() {
            let _ = DeleteDC(hdc_mem);
            ReleaseDC(hwnd, hdc_win);
            return Err("CreateCompatibleBitmap failed".into());
        }
        let old = SelectObject(hdc_mem, hbmp);
        let blitted = BitBlt(hdc_mem, 0, 0, full_w, full_h, hdc_win, 0, 0, SRCCOPY).is_ok();
        let mut bgra = if blitted {
            dibits_bgra(hdc_mem, hbmp, full_w, full_h).filter(|b| !bgra_is_blank(b))
        } else {
            None
        };
        let _ = SelectObject(hdc_mem, old);
        let _ = DeleteObject(hbmp);
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(hwnd, hdc_win);

        if bgra.is_none() {
            // Timed PW — never block ECS thread forever on hung/GPU targets.
            bgra = capture_printwindow_bgra_timed(hwnd, full_w, full_h).filter(|b| !bgra_is_blank(b));
        }
        let bgra = bgra.ok_or_else(|| "GetDIBits failed".to_string())?;
        encode_jpeg_bgra(&bgra, full_w, full_h, roi)
    }
}

/// Capture the window's own pixels (PrintWindow / window DC) — never screen blit.
/// Used for Dock hover previews so overlapping chrome / other apps don't become "the screen".
#[cfg(windows)]
pub fn capture_window_owned_jpeg(hwnd_raw: isize) -> Result<CapturedFrame, String> {
    capture_window_owned_thumb_jpeg(hwnd_raw, 280, 168)
}

/// Dock / owned preview: prefer PrintWindow (works for many GPU apps), reject black frames.
///
/// **Never** soft-restores minimized windows — a background refresher doing
/// ShowWindow(SW_SHOWNOACTIVATE)↔minimize flashes apps (e.g. Cursor) onto the
/// desktop and poisons ambient chrome sampling.
#[cfg(windows)]
pub fn capture_window_owned_thumb_jpeg(
    hwnd_raw: isize,
    max_w: u32,
    max_h: u32,
) -> Result<CapturedFrame, String> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, IsIconic, IsWindow, IsWindowVisible,
    };

    if crate::win32::hang::is_hung_hwnd(hwnd_raw) {
        return Err("window hung".into());
    }

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

        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect).map_err(|e| format!("GetWindowRect: {e}"))?;
        let full_w = (rect.right - rect.left).max(1);
        let full_h = (rect.bottom - rect.top).max(1);
        if full_w > 8192 || full_h > 8192 {
            return Err("window size out of range".into());
        }

        let max_w = max_w.max(1);
        let max_h = max_h.max(1);

        let try_capture = |hwnd: HWND, full_w: i32, full_h: i32| -> Option<CapturedFrame> {
            let scale = (max_w as f64 / full_w as f64)
                .min(max_h as f64 / full_h as f64)
                .min(1.0);
            let dst_w = ((full_w as f64) * scale).round().max(1.0) as i32;
            let dst_h = ((full_h as f64) * scale).round().max(1.0) as i32;

            // WindowDC / stretch first — PrintWindow last with timeout (hang-safe).
            if let Some(buf) = capture_windowdc_thumb_bgra(hwnd, full_w, full_h, dst_w, dst_h)
                .filter(|b| bgra_looks_usable(b))
            {
                return encode_jpeg_bgra_thumb(&buf, dst_w, dst_h, max_w, max_h).ok();
            }
            if let Some(buf) =
                capture_windowdc_bgra(hwnd, full_w, full_h).filter(|b| bgra_looks_usable(b))
            {
                return encode_jpeg_bgra_thumb(&buf, full_w, full_h, max_w, max_h).ok();
            }
            if let Some(buf) =
                capture_printwindow_bgra_timed(hwnd, full_w, full_h).filter(|b| bgra_looks_usable(b))
            {
                return encode_jpeg_bgra_thumb(&buf, full_w, full_h, max_w, max_h).ok();
            }
            None
        };

        // First attempt, then one short retry — DWM/GPU apps often need a composed frame.
        if let Some(frame) = try_capture(hwnd, full_w, full_h) {
            return Ok(frame);
        }
        std::thread::sleep(std::time::Duration::from_millis(28));
        if let Some(frame) = try_capture(hwnd, full_w, full_h) {
            return Ok(frame);
        }
        // On-screen blit last: PrintWindow/WindowDC often blank on Chromium/GPU,
        // but the visible pixels are already composed.
        if let Some(buf) =
            capture_screen_bgra(rect.left, rect.top, full_w, full_h).filter(|b| bgra_looks_usable(b))
        {
            if let Ok(frame) = encode_jpeg_bgra_thumb(&buf, full_w, full_h, max_w, max_h) {
                return Ok(frame);
            }
        }

        Err("owned window capture blank/black".into())
    }
}

#[cfg(not(windows))]
pub fn capture_window_jpeg(_hwnd_raw: isize, _roi: Roi) -> Result<CapturedFrame, String> {
    Err("Windows only".into())
}

#[cfg(not(windows))]
pub fn capture_window_owned_jpeg(_hwnd_raw: isize) -> Result<CapturedFrame, String> {
    Err("Windows only".into())
}

#[cfg(not(windows))]
pub fn capture_window_owned_thumb_jpeg(
    _hwnd_raw: isize,
    _max_w: u32,
    _max_h: u32,
) -> Result<CapturedFrame, String> {
    Err("Windows only".into())
}
