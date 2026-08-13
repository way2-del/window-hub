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
    const PW_RENDERFULLCONTENT: u32 = 0x2;
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
    let flags = PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT);
    let printed = PrintWindow(hwnd, hdc_mem, flags).as_bool();
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

#[cfg(windows)]
pub fn capture_window_jpeg(hwnd_raw: isize, roi: Roi) -> Result<CapturedFrame, String> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        ReleaseDC, SelectObject, SRCCOPY,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetWindowRect, IsWindow};

    const PW_CLIENTONLY: u32 = 0x1;
    const PW_RENDERFULLCONTENT: u32 = 0x2;

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
                bgra = capture_printwindow_bgra(hwnd, full_w, full_h).filter(|b| !bgra_is_blank(b));
            }
            let bgra = bgra.ok_or_else(|| "window capture blank".to_string())?;
            return encode_jpeg_bgra(&bgra, full_w, full_h, Roi::default());
        }

        // ROI path (ECS): client-area PrintWindow / BitBlt, coords relative to client.
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
        let flags = PRINT_WINDOW_FLAGS(PW_CLIENTONLY | PW_RENDERFULLCONTENT);
        let printed = PrintWindow(hwnd, hdc_mem, flags).as_bool();
        if !printed {
            let _ = BitBlt(hdc_mem, 0, 0, full_w, full_h, hdc_win, 0, 0, SRCCOPY);
        }
        let bgra = dibits_bgra(hdc_mem, hbmp, full_w, full_h);
        let _ = SelectObject(hdc_mem, old);
        let _ = DeleteObject(hbmp);
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(hwnd, hdc_win);
        let bgra = bgra.ok_or_else(|| "GetDIBits failed".to_string())?;
        encode_jpeg_bgra(&bgra, full_w, full_h, roi)
    }
}

#[cfg(not(windows))]
pub fn capture_window_jpeg(_hwnd_raw: isize, _roi: Roi) -> Result<CapturedFrame, String> {
    Err("Windows only".into())
}
