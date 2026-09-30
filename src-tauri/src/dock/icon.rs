//! Resolve dock icon PNG base64 from icopath or exe file icon.
//!
//! Prefer high-DPI shells (`IShellItemImageFactory` @ 256px) so 32 CSS px
//! icons stay sharp on 150%/200% displays. Legacy SHGFI_LARGEICON is 32px only.
//!
//! Hot cache: process-local HashMap of base64 by source path.
//! Cold store: `%APPDATA%\window-hub\dock-icons\{itemId}.png` — prefs only keep
//! the owned path (MyDockFinder imports / picks / pins all materialize here).

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Source raster size — enough headroom for 200–300% DPI (UI draws at 32 CSS px).
const SHELL_ICON_PX: i32 = 256;

fn icon_cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Drop cached rasters (call when dock prefs / icon paths change).
pub fn clear_icon_cache() {
    icon_cache().lock().clear();
}

/// `%APPDATA%\window-hub\dock-icons`
pub fn icons_dir() -> Result<PathBuf, String> {
    let dir = crate::db::app_data_root()?.join("dock-icons");
    std::fs::create_dir_all(&dir).map_err(|e| format!("dock-icons dir: {e}"))?;
    Ok(dir)
}

fn safe_icon_stem(item_id: &str) -> String {
    let mut out = String::with_capacity(item_id.len());
    for ch in item_id.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "icon".into()
    } else {
        out
    }
}

fn path_is_under_icons_dir(path: &Path) -> bool {
    let Ok(dir) = icons_dir() else {
        return false;
    };
    let Ok(canon_dir) = dunce_canonicalize(&dir) else {
        return false;
    };
    let Ok(canon_path) = dunce_canonicalize(path) else {
        return path.starts_with(&dir);
    };
    canon_path.starts_with(&canon_dir)
}

fn dunce_canonicalize(path: &Path) -> Result<PathBuf, std::io::Error> {
    let c = std::fs::canonicalize(path)?;
    let s = c.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        Ok(PathBuf::from(stripped))
    } else {
        Ok(c)
    }
}

fn owned_icon_path(item_id: &str) -> Result<PathBuf, String> {
    Ok(icons_dir()?.join(format!("{}.png", safe_icon_stem(item_id))))
}

/// True when `icon_path` already points at our on-disk cache and the file exists.
pub fn is_cached_icon_path(icon_path: &str) -> bool {
    let p = Path::new(icon_path.trim());
    !icon_path.trim().is_empty() && p.is_file() && path_is_under_icons_dir(p)
}

/// First known AppsFolder AUMID for a packaged system exe path (if any).
pub fn preferred_aumid_for_path(path: &str) -> Option<String> {
    aumid_candidates_for_paths(path, path)
        .into_iter()
        .next()
        .map(|s| s.to_string())
}

/// True when `aumid` is a known AppsFolder id for this exe / launch path.
pub fn path_matches_aumid(path: &str, aumid: &str) -> bool {
    let id = aumid.trim();
    if id.is_empty() {
        return false;
    }
    aumid_candidates_for_paths(path, path)
        .iter()
        .any(|c| c.eq_ignore_ascii_case(id))
}

/// Ensure each pin stores an owned `icon_path` under `dock-icons` (when a raster exists).
pub fn ensure_item_icon_cached(item: &mut super::DockItem) {
    if item.kind == "separator" {
        return;
    }
    // Builtin Start / Trash keep empty icon_path → Host SVG (until user picks).
    if item.kind == "startmenu" && item.icon_path.trim().is_empty() {
        return;
    }
    if item.kind == "trash" {
        if !item.icon_path.trim().is_empty() && !is_cached_icon_path(&item.icon_path) {
            if let Ok(Some(path)) = materialize_item_icon(&item.id, &item.icon_path, "") {
                item.icon_path = path;
            }
        }
        if !item.icon_path_full.trim().is_empty() && !is_cached_icon_path(&item.icon_path_full) {
            let full_id = format!("{}-full", item.id);
            if let Ok(Some(path)) = materialize_item_icon(&full_id, &item.icon_path_full, "") {
                item.icon_path_full = path;
            }
        }
        return;
    }
    if is_cached_icon_path(&item.icon_path) {
        return;
    }
    // Prefer AppsFolder when we already know the AUMID (UWP / system settings).
    let aumid = item.virtual_path.trim();
    if !aumid.is_empty() {
        if let Some(png) = resolve_item_icon_png_bytes("", "", aumid) {
            if let Ok(dest) = owned_icon_path(&item.id) {
                if std::fs::write(&dest, &png).is_ok() {
                    clear_icon_cache();
                    item.icon_path = normalize_path_string(&dest.to_string_lossy());
                    return;
                }
            }
        }
    }
    if item.icon_path.trim().is_empty() && aumid.is_empty() {
        return;
    }
    if let Ok(Some(path)) =
        materialize_item_icon_with_aumid(&item.id, &item.icon_path, &item.launch_path, aumid)
    {
        item.icon_path = path;
    }
}

/// Resolve source → write `%APPDATA%\window-hub\dock-icons\{itemId}.png` → return path.
pub fn materialize_item_icon(
    item_id: &str,
    icon_path: &str,
    launch_path: &str,
) -> Result<Option<String>, String> {
    materialize_item_icon_with_aumid(item_id, icon_path, launch_path, "")
}

pub fn materialize_item_icon_with_aumid(
    item_id: &str,
    icon_path: &str,
    launch_path: &str,
    aumid: &str,
) -> Result<Option<String>, String> {
    let id = item_id.trim();
    if id.is_empty() {
        return Err("item_id required".into());
    }
    if is_cached_icon_path(icon_path) {
        return Ok(Some(normalize_path_string(icon_path.trim())));
    }
    let Some(png) = resolve_item_icon_png_bytes(icon_path, launch_path, aumid) else {
        return Ok(None);
    };
    let dest = owned_icon_path(id)?;
    std::fs::write(&dest, &png).map_err(|e| format!("write dock icon: {e}"))?;
    // Invalidate hot cache entries that may still point at the old source.
    clear_icon_cache();
    Ok(Some(normalize_path_string(&dest.to_string_lossy())))
}

/// Copy / extract a user-picked file into the owned icon store for `item_id`.
pub fn cache_icon_from_source(item_id: &str, source_path: &str) -> Result<String, String> {
    let src = source_path.trim();
    if src.is_empty() {
        return Err("source_path required".into());
    }
    let cached = materialize_item_icon(item_id, src, src)?
        .ok_or_else(|| "无法解析图标".to_string())?;
    Ok(cached)
}

pub fn ensure_prefs_icons_cached(prefs: &mut super::DockPrefs) {
    for item in &mut prefs.items {
        ensure_item_icon_cached(item);
    }
}

fn normalize_path_string(s: &str) -> String {
    s.replace('/', "\\")
}

pub fn resolve_item_icon_png(icon_path: &str, launch_path: &str) -> Option<String> {
    resolve_item_icon_png_with_aumid(icon_path, launch_path, "")
}

/// Same as [`resolve_item_icon_png`], but prefer `shell:AppsFolder\{aumid}` for UWP /
/// packaged apps (Settings, Security Center, Store hosts, etc.).
pub fn resolve_item_icon_png_with_aumid(
    icon_path: &str,
    launch_path: &str,
    aumid: &str,
) -> Option<String> {
    let key = format!(
        "{}||{}||{}",
        icon_path.trim().to_ascii_lowercase(),
        launch_path.trim().to_ascii_lowercase(),
        aumid.trim().to_ascii_lowercase()
    );
    {
        let cache = icon_cache().lock();
        if let Some(hit) = cache.get(&key) {
            return hit.clone();
        }
    }
    let resolved =
        resolve_item_icon_png_bytes(icon_path, launch_path, aumid).map(|b| B64.encode(b));
    icon_cache().lock().insert(key, resolved.clone());
    resolved
}

fn resolve_item_icon_png_bytes(icon_path: &str, launch_path: &str, aumid: &str) -> Option<Vec<u8>> {
    #[cfg(windows)]
    {
        let aumid = aumid.trim();
        if !aumid.is_empty() {
            if let Some(b) = extract_via_apps_folder(aumid) {
                return Some(b);
            }
        }
        for cand in aumid_candidates_for_paths(icon_path, launch_path) {
            if cand.eq_ignore_ascii_case(aumid) {
                continue;
            }
            if let Some(b) = extract_via_apps_folder(cand) {
                return Some(b);
            }
        }
    }

    if !icon_path.trim().is_empty() {
        let (path, _idx) = split_icon_location(icon_path.trim());
        if let Some(b) = load_image_file_png_bytes(Path::new(path)) {
            return Some(b);
        }
        #[cfg(windows)]
        if let Some(b) = extract_shell_icon_png_bytes(Path::new(path)) {
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
        if let Some(b) = load_image_file_png_bytes(p) {
            return Some(b);
        }
    }
    #[cfg(windows)]
    {
        return extract_shell_icon_png_bytes(p);
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Known packaged-app AUMIDs when HWND property store is missing (pinned exe path only).
fn aumid_candidates_for_paths(icon_path: &str, launch_path: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    for raw in [icon_path, launch_path] {
        let name = Path::new(raw.trim())
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match name.as_str() {
            "systemsettings.exe" => {
                out.push(
                    "windows.immersivecontrolpanel_cw5n1h2txyewy!microsoft.windows.immersivecontrolpanel",
                );
                out.push("Microsoft.Windows.Settings_cw5n1h2txyewy!Settings");
            }
            "sechealthui.exe" | "securityhealthsystray.exe" | "securityhealthhost.exe" => {
                out.push("Microsoft.Windows.SecHealthUI_cw5n1h2txyewy!SecHealthUI");
            }
            _ => {}
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(windows)]
fn extract_via_apps_folder(aumid: &str) -> Option<Vec<u8>> {
    let id = aumid.trim();
    if id.is_empty() {
        return None;
    }
    // Prefer AppsFolder parsing name — this is where branded UWP icons live.
    let uri = format!("shell:AppsFolder\\{id}");
    extract_via_parsing_name(&uri)
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

fn load_image_file_png_bytes(path: &Path) -> Option<Vec<u8>> {
    let img = image::open(path).ok()?;
    // Prefer the largest frame for multi-size ICO.
    let rgba = img.to_rgba8();
    encode_rgba_png_bytes(rgba.as_raw(), rgba.width(), rgba.height())
}

fn encode_rgba_png_bytes(pixels: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    let enc = image::codecs::png::PngEncoder::new(&mut buf);
    use image::ImageEncoder;
    enc.write_image(pixels, w, h, image::ExtendedColorType::Rgba8)
        .ok()?;
    Some(buf)
}

#[cfg(windows)]
fn extract_shell_icon_png_bytes(path: &Path) -> Option<Vec<u8>> {
    extract_via_parsing_name(&path.to_string_lossy()).or_else(|| extract_via_shgfi_fallback(path))
}

/// High-quality path: shell image factory for file paths **or** `shell:AppsFolder\{AUMID}`.
#[cfg(windows)]
fn extract_via_parsing_name(name: &str) -> Option<Vec<u8>> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::DeleteObject;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{
        IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
    };

    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    let wide: Vec<u16> = std::ffi::OsStr::new(trimmed)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
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
        let png = hbitmap_to_png_bytes(hbmp);
        let _ = DeleteObject(hbmp);
        png
    }
}

/// Fallback when shell item factory fails (rare / very old paths).
#[cfg(windows)]
fn extract_via_shgfi_fallback(path: &Path) -> Option<Vec<u8>> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, SelectObject,
        BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::UI::Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGetFileInfoW};
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, DrawIconEx, DI_NORMAL};

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    // Draw into 128 so HiDPI still has some headroom even from a 32px HICON.
    let size = 128i32;
    unsafe {
        let mut fi = SHFILEINFOW::default();
        let ok = SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            Default::default(),
            Some(&mut fi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
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
        encode_rgba_png_bytes(&pixels, size as u32, size as u32)
    }
}

#[cfg(windows)]
fn hbitmap_to_png_bytes(hbmp: windows::Win32::Graphics::Gdi::HBITMAP) -> Option<Vec<u8>> {
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
        encode_rgba_png_bytes(&pixels, w as u32, h as u32)
    }
}

#[cfg(windows)]
fn bgra_to_rgba(pixels: &mut [u8]) {
    for chunk in pixels.chunks_exact_mut(4) {
        chunk.swap(0, 2);
    }
}
