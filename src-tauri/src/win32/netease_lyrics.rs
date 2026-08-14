//! 网易云桌面歌词 → 灵动岛镜像（对齐 MyDockFinder「捕捉 / 彩色映射」）。
//!
//! 网易云 DesktopLyrics 多为自绘分层窗，OCR 会错字且易超前。
//! 本模块只做：检测可见桌面歌词窗 → 截取画面 → 交岛栏缩小显示。
//! 不做 OCR / 本地 LRC 选句；不在热路径调 SMTC。

use serde::Serialize;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    IsWindowVisible,
};

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseNowPlaying {
    pub active: bool,
    pub title: Option<String>,
    pub artist: Option<String>,
    /// 兼容旧插件：有镜像时放 "♪"
    pub lyric: Option<String>,
    pub source: Option<String>,
    pub desktop_lyrics: bool,
    /// `data:image/png;base64,...` 桌面歌词有字区域裁切（透明底）
    pub lyric_image: Option<String>,
    /// DWM 实时映射已接通（岛栏可走 live thumb，不必依赖截屏）
    #[serde(default)]
    pub mirror_live: bool,
}

struct ImageCache {
    hwnd: isize,
    data_url: String,
    at: Instant,
}

static IMAGE_CACHE: Mutex<Option<ImageCache>> = Mutex::new(None);

fn wide_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    OsString::from_wide(&buf[..len])
        .to_string_lossy()
        .trim()
        .to_string()
}

fn hwnd_class(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
    if n == 0 {
        return String::new();
    }
    wide_to_string(&buf[..n])
}

fn hwnd_title(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; (len as usize) + 1];
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) } as usize;
    if n == 0 {
        return String::new();
    }
    wide_to_string(&buf[..n])
}

fn parse_title_artist(raw: &str) -> (Option<String>, Option<String>) {
    let t = raw.trim();
    if t.is_empty() || t == "网易云音乐" || t.eq_ignore_ascii_case("cloudmusic") {
        return (None, None);
    }
    for sep in [" - ", " – ", " — ", "-", "–", "—"] {
        if let Some((a, b)) = t.split_once(sep) {
            let title = a.trim();
            let artist = b.trim();
            if !title.is_empty() {
                return (
                    Some(title.to_string()),
                    if artist.is_empty() {
                        None
                    } else {
                        Some(artist.to_string())
                    },
                );
            }
        }
    }
    (Some(t.to_string()), None)
}

struct EnumCtx {
    main_visible: Option<HWND>,
    main_any: Option<HWND>,
    lyric: Option<HWND>,
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut EnumCtx);
    let class = hwnd_class(hwnd);
    let visible = IsWindowVisible(hwnd).as_bool();

    if class == "OrpheusBrowserHost" {
        if visible && ctx.main_visible.is_none() {
            ctx.main_visible = Some(hwnd);
        }
        if ctx.main_any.is_none() {
            ctx.main_any = Some(hwnd);
        }
    } else if class == "DesktopLyrics" {
        // 只要正式桌面歌词窗；Unlock 小钮 / 模糊匹配易截错
        if visible {
            ctx.lyric = Some(hwnd);
        }
    }
    BOOL(1)
}

fn find_netease_hwnds() -> (Option<HWND>, Option<HWND>) {
    let mut ctx = EnumCtx {
        main_visible: None,
        main_any: None,
        lyric: None,
    };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut _ as isize));
    }
    (ctx.main_visible.or(ctx.main_any), ctx.lyric)
}

/// 面板多媒体键入口（Host 仍会调用）。镜像不依赖本地钟，空操作。
pub fn note_media_transport(_action: &str) {}

/// 亮度阈值：低于此视为「透明底 / 黑底」（PrintWindow 常把分层透明填成黑）。
const INK_LUMA: u32 = 28;

fn luma(r: u8, g: u8, b: u8) -> u32 {
    // 粗略感知亮度
    (r as u32 * 3 + g as u32 * 6 + b as u32) / 10
}

fn is_ink(r: u8, g: u8, b: u8) -> bool {
    luma(r, g, b) > INK_LUMA
}

/// 裁切包围盒：要字不要底板。低饱和底板（棕/灰）排除；亮笔/绿描边保留（含字顶细笔）。
fn is_ink_core(r: u8, g: u8, b: u8) -> bool {
    let y = luma(r, g, b);
    let maxc = r.max(g).max(b) as i32;
    let minc = r.min(g).min(b) as i32;
    let sat = maxc - minc;
    // 网易云桌面歌词底板：中低亮 + 低饱和
    if sat < 28 && y < 105 {
        return false;
    }
    if y >= 88 {
        return true;
    }
    if g as i32 > r as i32 + 12 && g as i32 > b as i32 + 6 && y >= 48 {
        return true;
    }
    false
}

/// 截取桌面歌词窗 → BGRA（含透明区被填成黑的情况）。
fn capture_lyric_bgra(hwnd: HWND) -> Option<(Vec<u8>, u32, u32)> {
    unsafe {
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
        if fw < 24 || fh < 10 {
            return None;
        }

        let read_dib = |hdc_mem, hbmp, w: i32, h: i32| -> Option<(Vec<u8>, u32, u32)> {
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
            let n = (w as usize) * (h as usize);
            let mut ink = 0usize;
            for i in 0..n {
                let si = i * 4;
                let b = bgra[si];
                let g = bgra[si + 1];
                let r = bgra[si + 2];
                if is_ink(r, g, b) {
                    ink += 1;
                }
            }
            if ink < 12 {
                return None;
            }
            Some((bgra, w as u32, h as u32))
        };

        // 只用 PrintWindow。屏上 BitBlt 会透过分层透明歌词窗，截到背后的网易云主界面（专辑墙等）。
        let hdc_win = GetDC(hwnd);
        if hdc_win.is_invalid() {
            return None;
        }
        let hdc_mem = CreateCompatibleDC(hdc_win);
        if hdc_mem.is_invalid() {
            ReleaseDC(hwnd, hdc_win);
            return None;
        }
        let hbmp = CreateCompatibleBitmap(hdc_win, fw, fh);
        if hbmp.is_invalid() {
            let _ = DeleteDC(hdc_mem);
            ReleaseDC(hwnd, hdc_win);
            return None;
        }
        let old = SelectObject(hdc_mem, hbmp);
        let ok = PrintWindow(hwnd, hdc_mem, PRINT_WINDOW_FLAGS(0x2)).as_bool()
            || PrintWindow(hwnd, hdc_mem, PRINT_WINDOW_FLAGS(0)).as_bool();
        let out = if ok {
            read_dib(hdc_mem, hbmp, fw, fh)
        } else {
            None
        };
        SelectObject(hdc_mem, old);
        let _ = DeleteObject(hbmp);
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(hwnd, hdc_win);
        out
    }
}

/// 裁切有墨迹的包围盒，并把近黑像素打成透明 → RGBA。
fn crop_ink_to_rgba(bgra: &[u8], w: u32, h: u32) -> Option<(Vec<u8>, u32, u32)> {
    let w = w as usize;
    let h = h as usize;
    if w == 0 || h == 0 || bgra.len() < w * h * 4 {
        return None;
    }
    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    let mut found = false;
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let b = bgra[i];
            let g = bgra[i + 1];
            let r = bgra[i + 2];
            if is_ink(r, g, b) {
                found = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if !found {
        return None;
    }
    // 留一点描边余量，避免切掉发光字边
    let pad = 4usize;
    let x0 = min_x.saturating_sub(pad);
    let y0 = min_y.saturating_sub(pad);
    let x1 = (max_x + pad + 1).min(w);
    let y1 = (max_y + pad + 1).min(h);
    let cw = x1 - x0;
    let ch = y1 - y0;
    if cw < 4 || ch < 4 {
        return None;
    }
    // 单行歌词应偏扁；接近方块/大色块多半是截到了主界面
    if ch > 140 || (cw as f64 / ch as f64) < 2.2 {
        return None;
    }

    let mut rgba = vec![0u8; cw * ch * 4];
    let mut ink_n = 0usize;
    let mut color_bins = [0u8; 512]; // 粗量化色桶占用标记
    let mut distinct = 0usize;
    for y in 0..ch {
        for x in 0..cw {
            let si = ((y0 + y) * w + (x0 + x)) * 4;
            let di = (y * cw + x) * 4;
            let b = bgra[si];
            let g = bgra[si + 1];
            let r = bgra[si + 2];
            if is_ink(r, g, b) {
                ink_n += 1;
                let bin = (((r as usize) >> 5) << 6) | (((g as usize) >> 5) << 3) | ((b as usize) >> 5);
                if color_bins[bin] == 0 {
                    color_bins[bin] = 1;
                    distinct += 1;
                }
                rgba[di] = r;
                rgba[di + 1] = g;
                rgba[di + 2] = b;
                let a = ((luma(r, g, b) - INK_LUMA).min(200) as u8).saturating_add(55);
                rgba[di + 3] = a.max(90);
            } else {
                rgba[di] = 0;
                rgba[di + 1] = 0;
                rgba[di + 2] = 0;
                rgba[di + 3] = 0;
            }
        }
    }
    // 专辑封面/推荐墙颜色很杂；单行歌词通常色相很少
    if distinct > 56 || ink_n < 20 {
        return None;
    }
    Some((rgba, cw as u32, ch as u32))
}

/// 目标显示约 22px；编码保留 2×～2.5× 清晰度，交给 CSS 缩小。
fn rgba_to_png_data_url(rgba: &[u8], w: u32, h: u32) -> Option<String> {
    use image::codecs::png::PngEncoder;
    use image::{ImageBuffer, ImageEncoder, Rgba};
    use std::io::Cursor;

    let img: ImageBuffer<Rgba<u8>, _> = ImageBuffer::from_raw(w, h, rgba.to_vec())?;

    // 岛栏显示高 ~26；源图高度压到 52～64，避免整窗竖条却又够清晰
    const TARGET_H: u32 = 56;
    const MAX_W: u32 = 720;
    let (out_img, out_w, out_h) = if h > TARGET_H || w > MAX_W {
        let scale_h = TARGET_H as f64 / h as f64;
        let scale_w = MAX_W as f64 / w as f64;
        let scale = scale_h.min(scale_w).min(1.0);
        let nw = ((w as f64) * scale).round().max(1.0) as u32;
        let nh = ((h as f64) * scale).round().max(1.0) as u32;
        let resized =
            image::imageops::resize(&img, nw, nh, image::imageops::FilterType::CatmullRom);
        (resized, nw, nh)
    } else if h < 28 && h > 0 {
        // 源太矮：适度放大，减轻 CSS 再缩时的糊感
        let scale = (36.0 / h as f64).min(2.0);
        let nw = ((w as f64) * scale).round().max(1.0) as u32;
        let nh = ((h as f64) * scale).round().max(1.0) as u32;
        let resized =
            image::imageops::resize(&img, nw, nh, image::imageops::FilterType::CatmullRom);
        (resized, nw, nh)
    } else {
        (img, w, h)
    };

    let mut cursor = Cursor::new(Vec::new());
    {
        let enc = PngEncoder::new(&mut cursor);
        enc.write_image(
            out_img.as_raw(),
            out_w,
            out_h,
            image::ExtendedColorType::Rgba8,
        )
        .ok()?;
    }
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, cursor.into_inner());
    Some(format!("data:image/png;base64,{b64}"))
}

fn capture_lyric_image(hwnd: HWND) -> Option<String> {
    let raw = hwnd.0 as isize;
    if let Ok(guard) = IMAGE_CACHE.lock() {
        if let Some(c) = guard.as_ref() {
            if c.hwnd == raw && c.at.elapsed() < Duration::from_millis(160) {
                return Some(c.data_url.clone());
            }
        }
    }
    let (bgra, w, h) = capture_lyric_bgra(hwnd)?;
    let (rgba, cw, ch) = crop_ink_to_rgba(&bgra, w, h)?;
    let data_url = rgba_to_png_data_url(&rgba, cw, ch)?;
    if let Ok(mut guard) = IMAGE_CACHE.lock() {
        *guard = Some(ImageCache {
            hwnd: raw,
            data_url: data_url.clone(),
            at: Instant::now(),
        });
    }
    Some(data_url)
}

/// 有字区域相对源画面的归一化裁切 `(l,t,r,b)`（约 0..1），供 DWM `rcSource`。
/// 用比例而非像素，避免 PrintWindow 与 DWM 缩略图源尺寸不一致时错位。
/// 只取「墨迹最多的一行带」，避免上下两句叠进岛栏。
pub fn ink_source_crop(hwnd: HWND) -> Option<(f64, f64, f64, f64)> {
    let (bgra, w0, h0) = capture_lyric_bgra(hwnd)?;
    let w = w0 as usize;
    let h = h0 as usize;
    if w == 0 || h == 0 || bgra.len() < w * h * 4 {
        return None;
    }
    let mut row_ink = vec![0usize; h];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            if is_ink(bgra[i + 2], bgra[i + 1], bgra[i]) {
                row_ink[y] += 1;
            }
        }
    }
    let thr = (w / 80).max(3);
    // 连续有墨迹 → 若干行带；优先「最矮且够密」的单行（当前句）
    let mut bands: Vec<(usize, usize, usize, f64)> = Vec::new(); // y0,y1,ink,density
    let mut y = 0usize;
    while y < h {
        if row_ink[y] < thr {
            y += 1;
            continue;
        }
        let y0 = y;
        let mut ink = 0usize;
        while y < h && row_ink[y] >= thr {
            ink += row_ink[y];
            y += 1;
        }
        let y1 = y;
        let hh = (y1 - y0).max(1);
        let density = ink as f64 / hh as f64;
        bands.push((y0, y1, ink, density));
    }
    if bands.is_empty() {
        return None;
    }
    let (band0, band1) = {
        let mut best = &bands[0];
        for b in &bands[1..] {
            let best_tall = (best.1 - best.0) > 52;
            let b_tall = (b.1 - b.0) > 52;
            if best_tall != b_tall {
                if !b_tall {
                    best = b;
                }
                continue;
            }
            if b.3 > best.3 * 1.05 || (b.3 >= best.3 * 0.95 && b.2 > best.2) {
                best = b;
            }
        }
        let (mut y0, mut y1, _, _) = *best;
        if y1 - y0 > 56 {
            let mut peak_y = y0;
            let mut peak_v = 0usize;
            for yy in y0..y1 {
                if row_ink[yy] > peak_v {
                    peak_v = row_ink[yy];
                    peak_y = yy;
                }
            }
            let half = 22usize;
            y0 = peak_y.saturating_sub(half).max(y0);
            y1 = (peak_y + half + 1).min(y1);
        }
        (y0, y1)
    };
    let mut min_x = w;
    let mut max_x = 0usize;
    let mut min_y = h;
    let mut max_y = 0usize;
    let mut found = false;
    for y in band0..band1 {
        for x in 0..w {
            let i = (y * w + x) * 4;
            if is_ink_core(bgra[i + 2], bgra[i + 1], bgra[i]) {
                found = true;
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    if !found {
        return None;
    }
    // 网易云桌面歌词字在胶囊里偏上；底边多留、顶边少留，映到岛栏后更接近视觉居中
    let pad_x = 5usize;
    let pad_top = 3usize;
    let pad_bot = 7usize;
    let x0 = min_x.saturating_sub(pad_x);
    let y0 = min_y.saturating_sub(pad_top);
    let x1 = (max_x + pad_x + 1).min(w);
    let y1 = (max_y + pad_bot + 1).min(h);
    if x1 <= x0 + 3 || y1 <= y0 + 3 {
        return None;
    }
    let fw = w as f64;
    let fh = h as f64;
    Some((
        x0 as f64 / fw,
        y0 as f64 / fh,
        x1 as f64 / fw,
        y1 as f64 / fh,
    ))
}

/// 当前可见 DesktopLyrics 窗句柄（0 = 无）。
pub fn desktop_lyrics_hwnd() -> isize {
    find_netease_hwnds()
        .1
        .map(|h| h.0 as isize)
        .unwrap_or(0)
}

pub fn snapshot() -> NeteaseNowPlaying {
    static CACHE: Mutex<Option<(Instant, NeteaseNowPlaying)>> = Mutex::new(None);
    if let Ok(guard) = CACHE.lock() {
        if let Some((at, snap)) = guard.as_ref() {
            if at.elapsed() < Duration::from_millis(120) {
                return snap.clone();
            }
        }
    }
    let snap = snapshot_uncached();
    if let Ok(mut guard) = CACHE.lock() {
        *guard = Some((Instant::now(), snap.clone()));
    }
    snap
}

fn snapshot_uncached() -> NeteaseNowPlaying {
    let (main, lyric_hwnd) = find_netease_hwnds();
    let mut out = NeteaseNowPlaying::default();
    out.desktop_lyrics = lyric_hwnd.is_some();

    if let Some(hwnd) = main {
        out.active = true;
        let title_raw = hwnd_title(hwnd);
        let (title, artist) = parse_title_artist(&title_raw);
        out.title = title;
        out.artist = artist;
        if out.title.is_some() {
            out.source = Some("window-title".into());
        }
    }

    if out.desktop_lyrics {
        out.active = true;
        if let Some(hwnd) = lyric_hwnd {
            let raw = hwnd.0 as isize;
            let crop = ink_source_crop(hwnd);
            // 折叠岛栏：DWM 实时映射（对齐 MyDockFinder）；截屏仅 PrintWindow，禁止 BitBlt 透底
            let dwm_ok = crate::win32::lyric_mirror::has_dest_slot()
                && crate::win32::lyric_mirror::sync(raw, crop);
            out.mirror_live = dwm_ok;
            out.lyric_image = capture_lyric_image(hwnd);
            if dwm_ok || out.lyric_image.is_some() {
                out.lyric = Some("♪".into());
                out.source = Some(if dwm_ok {
                    "desktop-dwm".into()
                } else {
                    "desktop-mirror".into()
                });
            } else if !dwm_ok {
                crate::win32::lyric_mirror::clear();
            }
        }
    } else {
        let _ = IMAGE_CACHE.lock().map(|mut g| *g = None);
        crate::win32::lyric_mirror::clear();
    }

    if out.title.is_some() || out.lyric_image.is_some() || out.desktop_lyrics {
        out.active = true;
    }
    out
}

pub fn open_or_focus() -> Result<(), String> {
    let (main, _) = find_netease_hwnds();
    if let Some(hwnd) = main {
        let raw = hwnd.0 as isize;
        if raw != 0 {
            return crate::win32::enum_windows::focus_window(raw);
        }
    }
    launch_cloudmusic()
}

fn launch_cloudmusic() -> Result<(), String> {
    use std::path::PathBuf;

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(local)
                .join("NetEase")
                .join("CloudMusic")
                .join("cloudmusic.exe"),
        );
    }
    for key in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(pf) = std::env::var(key) {
            candidates.push(
                PathBuf::from(pf)
                    .join("NetEase")
                    .join("CloudMusic")
                    .join("cloudmusic.exe"),
            );
        }
    }
    for path in &candidates {
        if path.is_file() {
            return crate::dock::shell_open_path(&path.to_string_lossy());
        }
    }
    crate::dock::shell_open_path("cloudmusic.exe")
        .map_err(|_| "未找到网易云音乐，请先安装或手动打开".into())
}
