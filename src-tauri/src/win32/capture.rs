//! Capture a window client area (with optional ROI) as JPEG bytes.

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

#[cfg(windows)]
pub fn capture_window_jpeg(hwnd_raw: isize, roi: Roi) -> Result<CapturedFrame, String> {
    use image::{ImageBuffer, ImageFormat, Rgb};
    use std::io::Cursor;
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        SRCCOPY,
    };
    use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
    use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, IsWindow};

    const PW_CLIENTONLY: u32 = 0x1;
    const PW_RENDERFULLCONTENT: u32 = 0x2;

    let hwnd = HWND(hwnd_raw as *mut _);
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return Err("Invalid window".into());
        }

        let mut rect = RECT::default();
        GetClientRect(hwnd, &mut rect).map_err(|e| format!("GetClientRect: {e}"))?;
        let full_w = (rect.right - rect.left).max(1);
        let full_h = (rect.bottom - rect.top).max(1);

        let (cx, cy, cw, ch) = if roi.use_full || roi.w <= 0 || roi.h <= 0 {
            (0, 0, full_w, full_h)
        } else {
            let x = roi.x.clamp(0, full_w - 1);
            let y = roi.y.clamp(0, full_h - 1);
            let w = roi.w.clamp(1, full_w - x);
            let h = roi.h.clamp(1, full_h - y);
            (x, y, w, h)
        };

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
            // Fallback: BitBlt from window DC (works when window is visible/composited)
            let _ = BitBlt(hdc_mem, 0, 0, full_w, full_h, hdc_win, 0, 0, SRCCOPY);
        }

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: full_w,
                biHeight: -full_h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bgra = vec![0u8; (full_w as usize) * (full_h as usize) * 4];
        let lines = GetDIBits(
            hdc_mem,
            hbmp,
            0,
            full_h as u32,
            Some(bgra.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        SelectObject(hdc_mem, old);
        let _ = DeleteObject(hbmp);
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(hwnd, hdc_win);

        if lines == 0 {
            return Err("GetDIBits failed".into());
        }

        // Crop ROI and convert BGRA -> RGB (JPEG does not support RGBA)
        let mut rgb = vec![0u8; (cw as usize) * (ch as usize) * 3];
        for row in 0..ch as usize {
            let src_y = (cy as usize) + row;
            for col in 0..cw as usize {
                let src_x = (cx as usize) + col;
                let si = (src_y * full_w as usize + src_x) * 4;
                let di = (row * cw as usize + col) * 3;
                rgb[di] = bgra[si + 2]; // R
                rgb[di + 1] = bgra[si + 1]; // G
                rgb[di + 2] = bgra[si]; // B
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
}

#[cfg(not(windows))]
pub fn capture_window_jpeg(_hwnd_raw: isize, _roi: Roi) -> Result<CapturedFrame, String> {
    Err("Windows only".into())
}
