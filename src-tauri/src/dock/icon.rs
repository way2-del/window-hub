//! Resolve dock icon PNG base64 from icopath or exe file icon.
//!
//! Prefer high-DPI shells (`IShellItemImageFactory` @ 256px) so 32 CSS px
//! icons stay sharp on 150%/200% displays. Legacy SHGFI_LARGEICON is 32px only.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use std::path::Path;

/// Source raster size — enough headroom for 200–300% DPI (UI draws at 32 CSS px).
const SHELL_ICON_PX: i32 = 256;

pub fn resolve_item_icon_png(icon_path: &str, launch_path: &str) -> Option<String> {
    if !icon_path.trim().is_empty() {
        let (path, _idx) = split_icon_location(icon_path.trim());
        if let Some(b) = load_image_file_png(Path::new(path)) {
            return Some(b);
        }
        #[cfg(windows)]
        if let Some(b) = extract_shell_icon_png(Path::new(path)) {
            return Some(b);
        }
    }
    let launch = launch_path.trim();
    if launch.is_empty() {
        return None;
    }
    let (path, _idx) = split_icon_location(launch);
    let p = Path::new(path);
    if !p.exists() {
        return None;
    }
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "ico" | "bmp" | "webp") {
        if let Some(b) = load_image_file_png(p) {
            return Some(b);
        }
    }
    #[cfg(windows)]
    {
        return extract_shell_icon_png(p);
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Cheap 32px icon for dense lists (memory ranking). Avoids 256px ShellItem factory.
pub fn resolve_small_icon_png(launch_path: &str) -> Option<String> {
    let launch = launch_path.trim();
    if launch.is_empty() {
        return None;
    }
    let (path, _idx) = split_icon_location(launch);
    let p = Path::new(path);
    if !p.exists() {
        return None;
    }
    #[cfg(windows)]
    {
        extract_via_shgfi_sized(p, 32)
    }
    #[cfg(not(windows))]
    {
        let _ = p;
        None
    }
}

/// Windows icon location: `C:\App\app.exe,0` or plain path.
fn split_icon_location(s: &str) -> (&str, i32) {
    if let Some((path, idx)) = s.rsplit_once(',') {
        if let Ok(n) = idx.trim().parse::<i32>() {
            let path = path.trim().trim_matches('"');
            if !path.is_empty() {
                return (path, n);
            }
        }
    }
    (s.trim().trim_matches('"'), 0)
}

fn load_image_file_png(path: &Path) -> Option<String> {
    let img = image::open(path).ok()?;
    // Prefer the largest frame for multi-size ICO.
    let rgba = img.to_rgba8();
    encode_rgba_png(rgba.as_raw(), rgba.width(), rgba.height())
}

fn encode_rgba_png(pixels: &[u8], w: u32, h: u32) -> Option<String> {
    let mut buf = Vec::new();
    let enc = image::codecs::png::PngEncoder::new(&mut buf);
    use image::ImageEncoder;
    enc.write_image(pixels, w, h, image::ExtendedColorType::Rgba8)
        .ok()?;
    Some(B64.encode(buf))
}

#[cfg(windows)]
fn extract_shell_icon_png(path: &Path) -> Option<String> {
    extract_via_shell_item(path).or_else(|| extract_via_shgfi_fallback(path))
}

/// High-quality path: shell image factory (jumbo / scaled icon, not 32px SHGFI).
#[cfg(windows)]
fn extract_via_shell_item(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::DeleteObject;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{
        SHCreateItemFromParsingName, IShellItemImageFactory, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
    };

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None).ok()?;
        let hbmp = factory
            .GetImage(
                SIZE {
                    cx: SHELL_ICON_PX,
                    cy: SHELL_ICON_PX,
                },
                SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
            )
            .ok()?;
        let png = hbitmap_to_png_b64(hbmp);
        let _ = DeleteObject(hbmp);
        png
    }
}

/// Fallback when shell item factory fails (rare / very old paths).
#[cfg(windows)]
fn extract_via_shgfi_fallback(path: &Path) -> Option<String> {
    extract_via_shgfi_sized(path, 128)
}

#[cfg(windows)]
fn extract_via_shgfi_sized(path: &Path, size: i32) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, SelectObject,
        BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::UI::Shell::{
        SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_SMALLICON, SHGetFileInfoW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, DrawIconEx, DI_NORMAL};

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let size = size.clamp(16, 128);
    let flags = if size <= 24 {
        SHGFI_ICON | SHGFI_SMALLICON
    } else {
        SHGFI_ICON | SHGFI_LARGEICON
    };
    unsafe {
        let mut fi = SHFILEINFOW::default();
        let ok = SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            Default::default(),
            Some(&mut fi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        );
        if ok == 0 || fi.hIcon.is_invalid() {
            return None;
        }
        let hicon = fi.hIcon;
        let hdc_screen = windows::Win32::Graphics::Gdi::GetDC(None);
        let hdc = CreateCompatibleDC(hdc_screen);
        let hbmp = CreateCompatibleBitmap(hdc_screen, size, size);
        let old = SelectObject(hdc, hbmp);
        let _ = DrawIconEx(hdc, 0, 0, hicon, size, size, 0, None, DI_NORMAL);

        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; (size * size * 4) as usize];
        let got = GetDIBits(
            hdc,
            hbmp,
            0,
            size as u32,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut bi,
            DIB_RGB_COLORS,
        );

        let _ = SelectObject(hdc, old);
        let _ = DeleteObject(hbmp);
        let _ = DeleteDC(hdc);
        let _ = windows::Win32::Graphics::Gdi::ReleaseDC(None, hdc_screen);
        let _ = DestroyIcon(hicon);

        if got == 0 {
            return None;
        }
        bgra_to_rgba(&mut pixels);
        encode_rgba_png(&pixels, size as u32, size as u32)
    }
}

#[cfg(windows)]
fn hbitmap_to_png_b64(hbmp: windows::Win32::Graphics::Gdi::HBITMAP) -> Option<String> {
    use windows::Win32::Graphics::Gdi::{
        GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        DIB_RGB_COLORS,
    };

    unsafe {
        let mut bm = BITMAP::default();
        if GetObjectW(
            hbmp,
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        ) == 0
        {
            return None;
        }
        let w = bm.bmWidth;
        let h = bm.bmHeight.unsigned_abs() as i32;
        if w <= 0 || h <= 0 {
            return None;
        }

        let hdc = GetDC(None);
        let mut bi = BITMAPINFO {
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
        let mut pixels = vec![0u8; (w as usize) * (h as usize) * 4];
        let got = GetDIBits(
            hdc,
            hbmp,
            0,
            h as u32,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut bi,
            DIB_RGB_COLORS,
        );
        let _ = ReleaseDC(None, hdc);
        if got == 0 {
            return None;
        }
        bgra_to_rgba(&mut pixels);
        encode_rgba_png(&pixels, w as u32, h as u32)
    }
}

#[cfg(windows)]
fn bgra_to_rgba(pixels: &mut [u8]) {
    for chunk in pixels.chunks_exact_mut(4) {
        chunk.swap(0, 2);
    }
}
