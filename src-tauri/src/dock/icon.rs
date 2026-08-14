//! Resolve dock icon PNG base64 from icopath or exe file icon.
//!
//! Prefer high-DPI shells (`IShellItemImageFactory` @ 256px) so 32 CSS px
//! icons stay sharp on 150%/200% displays. Legacy SHGFI_LARGEICON is 32px only.
//! Honor `path,index` (MyDockFinder / shell icon locations) and UWP AUMIDs.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use std::path::Path;

use super::DockItem;

/// Source raster size — enough headroom for 200–300% DPI (UI draws at 32 CSS px).
const SHELL_ICON_PX: i32 = 256;

/// Resolve PNG for a dock item (all path / kind fallbacks).
pub fn resolve_dock_item_icon(item: &DockItem) -> Option<String> {
    match item.kind.as_str() {
        "separator" => None,
        "startmenu" => resolve_startmenu_icon()
            .or_else(|| try_icon_location(&item.icon_path))
            .or_else(|| try_file_path(&item.launch_path))
            .or_else(|| try_file_path(&item.real_path)),
        "trash" => resolve_trash_icon()
            .or_else(|| try_icon_location(&item.icon_path))
            .or_else(|| try_file_path(&item.launch_path)),
        _ => try_icon_location(&item.icon_path)
            .or_else(|| try_file_path(&item.launch_path))
            .or_else(|| try_file_path(&item.real_path))
            .or_else(|| try_uwp_aumid(&item.virtual_path)),
    }
}

/// Back-compat for callers that only have path strings.
pub fn resolve_item_icon_png(icon_path: &str, launch_path: &str) -> Option<String> {
    try_icon_location(icon_path).or_else(|| try_file_path(launch_path))
}

/// Cheap 32px icon for dense lists (memory ranking). Avoids 256px ShellItem factory.
pub fn resolve_small_icon_png(launch_path: &str) -> Option<String> {
    let launch = launch_path.trim();
    if launch.is_empty() {
        return None;
    }
    let (path, idx) = split_icon_location(launch);
    let p = Path::new(path);
    if !p.exists() {
        return None;
    }
    #[cfg(windows)]
    {
        if idx != 0 {
            return extract_via_shdef(p, idx, 32);
        }
        extract_via_shgfi_sized(p, 32)
    }
    #[cfg(not(windows))]
    {
        let _ = (p, idx);
        None
    }
}

fn try_icon_location(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (path, idx) = split_icon_location(raw);
    let p = Path::new(path);

    // Raster / ICO files: load pixels (ignore index).
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
        // Explicit resource index (dll/exe,N) — SHDefExtractIcon.
        if idx != 0 || matches!(ext.as_str(), "dll" | "exe" | "cpl" | "ocx" | "scr") {
            if let Some(b) = extract_via_shdef(p, idx, SHELL_ICON_PX) {
                return Some(b);
            }
        }
        if p.exists() {
            if let Some(b) = extract_shell_icon_png(p) {
                return Some(b);
            }
        }
        // Non-existent custom icopath: do not invent a generic shell icon.
        None
    }
    #[cfg(not(windows))]
    {
        let _ = idx;
        None
    }
}

fn try_file_path(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (path, idx) = split_icon_location(raw);
    let p = Path::new(path);
    if !p.exists() {
        // May still be a shell: URI (rare in filepath) — try parsing name as-is.
        #[cfg(windows)]
        {
            if path.starts_with("shell:") {
                return extract_via_shell_item_str(path);
            }
        }
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
        if idx != 0 {
            if let Some(b) = extract_via_shdef(p, idx, SHELL_ICON_PX) {
                return Some(b);
            }
        }
        extract_shell_icon_png(p)
    }
    #[cfg(not(windows))]
    {
        let _ = idx;
        None
    }
}

fn try_uwp_aumid(aumid: &str) -> Option<String> {
    let aumid = aumid.trim();
    if aumid.is_empty() {
        return None;
    }
    #[cfg(windows)]
    {
        let uri = format!("shell:AppsFolder\\{aumid}");
        extract_via_shell_item_str(&uri)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(windows)]
fn resolve_startmenu_icon() -> Option<String> {
    // Prefer a recognizable Windows / Start glyph from system binaries.
    let windir = std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into());
    let candidates = [
        Path::new(&windir).join(
            r"SystemApps\Microsoft.Windows.StartMenuExperienceHost_cw5n1h2txyewy\StartMenuExperienceHost.exe",
        ),
        Path::new(&windir).join("explorer.exe"),
        Path::new(&windir).join(r"System32\shell32.dll"),
    ];
    for p in &candidates {
        if p.exists() {
            if let Some(b) = extract_shell_icon_png(p) {
                return Some(b);
            }
        }
    }
    // Folder icon for the Start Menu directory as last resort.
    extract_via_shell_item_str("shell:StartMenu")
}

#[cfg(not(windows))]
fn resolve_startmenu_icon() -> Option<String> {
    None
}

#[cfg(windows)]
fn resolve_trash_icon() -> Option<String> {
    use windows::Win32::UI::Shell::{
        SHGetStockIconInfo, SHGSI_ICON, SHGSI_LARGEICON, SHSTOCKICONINFO, SIID_RECYCLER,
    };
    use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;

    unsafe {
        let mut info = SHSTOCKICONINFO {
            cbSize: std::mem::size_of::<SHSTOCKICONINFO>() as u32,
            ..Default::default()
        };
        if SHGetStockIconInfo(SIID_RECYCLER, SHGSI_ICON | SHGSI_LARGEICON, &mut info).is_err() {
            return extract_via_shell_item_str("shell:RecycleBinFolder");
        }
        let hicon = info.hIcon;
        if hicon.is_invalid() {
            return extract_via_shell_item_str("shell:RecycleBinFolder");
        }
        let png = hicon_to_png_b64(hicon, 128);
        let _ = DestroyIcon(hicon);
        png.or_else(|| extract_via_shell_item_str("shell:RecycleBinFolder"))
    }
}

#[cfg(not(windows))]
fn resolve_trash_icon() -> Option<String> {
    None
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
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    extract_via_shell_item_wide(&wide)
}

#[cfg(windows)]
fn extract_via_shell_item_str(s: &str) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    extract_via_shell_item_wide(&wide)
}

#[cfg(windows)]
fn extract_via_shell_item_wide(wide: &[u16]) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::DeleteObject;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{
        IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
    };

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

/// Extract icon by resource index (`dll,N` / `exe,N`).
#[cfg(windows)]
fn extract_via_shdef(path: &Path, index: i32, size: i32) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::SHDefExtractIconW;
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let size = size.clamp(16, 256);
    unsafe {
        let mut large = HICON::default();
        let mut small = HICON::default();
        let hr = SHDefExtractIconW(
            PCWSTR(wide.as_ptr()),
            index,
            0,
            Some(&mut large as *mut _),
            Some(&mut small as *mut _),
            size as u32,
        );
        if hr.is_err() || large.is_invalid() {
            if !small.is_invalid() {
                let _ = DestroyIcon(small);
            }
            return None;
        }
        let png = hicon_to_png_b64(large, size);
        let _ = DestroyIcon(large);
        if !small.is_invalid() {
            let _ = DestroyIcon(small);
        }
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
    use windows::Win32::UI::Shell::{
        SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_SMALLICON, SHGetFileInfoW,
    };
    use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
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
        let png = hicon_to_png_b64(hicon, size);
        let _ = DestroyIcon(hicon);
        png
    }
}

#[cfg(windows)]
fn hicon_to_png_b64(hicon: windows::Win32::UI::WindowsAndMessaging::HICON, size: i32) -> Option<String> {
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, SelectObject,
        BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::UI::WindowsAndMessaging::{DrawIconEx, DI_NORMAL};

    let size = size.clamp(16, 256);
    unsafe {
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
