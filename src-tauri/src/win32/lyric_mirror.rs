//! 桌面歌词窗 → 灵动岛：DWM 实时缩略图映射（对齐 MyDockFinder「彩色映射」）。
//!
//! `DwmRegisterThumbnail(岛栏 HWND, DesktopLyrics)`，由 DWM 合成源窗口画面。
//! 可选 `rcSource` 裁切有字区域，避免整窗竖条被压扁。

use std::sync::Mutex;

struct MirrorState {
    thumb_id: Option<isize>,
    src_hwnd: isize,
    dest_hwnd: isize,
    /// 目的矩形：相对 dest 客户区，物理像素 (l,t,r,b)
    dest: (i32, i32, i32, i32),
    /// 源裁切：相对源客户区，物理像素；None = 整窗
    src_crop: Option<(i32, i32, i32, i32)>,
}

fn state() -> &'static Mutex<MirrorState> {
    static S: std::sync::OnceLock<Mutex<MirrorState>> = std::sync::OnceLock::new();
    S.get_or_init(|| {
        Mutex::new(MirrorState {
            thumb_id: None,
            src_hwnd: 0,
            dest_hwnd: 0,
            dest: (0, 0, 0, 0),
            src_crop: None,
        })
    })
}

/// 前端测好岛栏歌词槽位后写入（物理像素，相对岛窗客户区）。
pub fn set_dest_slot(dest_hwnd: isize, x: i32, y: i32, w: i32, h: i32) {
    if dest_hwnd == 0 || w < 4 || h < 4 {
        return;
    }
    let dest = (x, y, x + w, y + h);
    let (thumb_id, src, crop) = {
        let Ok(mut g) = state().lock() else {
            return;
        };
        g.dest_hwnd = dest_hwnd;
        g.dest = dest;
        (g.thumb_id, g.src_hwnd, g.src_crop)
    };
    if let Some(id) = thumb_id {
        if src != 0 {
            let _ = update_props(id, src, dest, crop);
        }
    }
}

pub fn clear() {
    #[cfg(windows)]
    {
        use windows::Win32::Graphics::Dwm::DwmUnregisterThumbnail;
        if let Ok(mut g) = state().lock() {
            if let Some(id) = g.thumb_id.take() {
                unsafe {
                    let _ = DwmUnregisterThumbnail(id);
                }
            }
            g.src_hwnd = 0;
            g.src_crop = None;
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(mut g) = state().lock() {
            g.thumb_id = None;
            g.src_hwnd = 0;
            g.src_crop = None;
        }
    }
}

pub fn has_dest_slot() -> bool {
    state()
        .lock()
        .map(|g| {
            g.dest_hwnd != 0 && (g.dest.2 - g.dest.0) >= 4 && (g.dest.3 - g.dest.1) >= 4
        })
        .unwrap_or(false)
}

pub fn is_live() -> bool {
    state()
        .lock()
        .map(|g| g.thumb_id.is_some() && g.src_hwnd != 0)
        .unwrap_or(false)
}

/// 把 DesktopLyrics 实时映到已设置的岛栏槽位。`src_crop` 为源窗客户区裁切。
pub fn sync(src_hwnd: isize, src_crop: Option<(i32, i32, i32, i32)>) -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::Graphics::Dwm::{DwmRegisterThumbnail, DwmUnregisterThumbnail};
        use windows::Win32::UI::WindowsAndMessaging::IsWindow;

        if src_hwnd == 0 {
            clear();
            return false;
        }
        let src = HWND(src_hwnd as _);
        unsafe {
            if !IsWindow(src).as_bool() {
                clear();
                return false;
            }
        }

        let (dest_hwnd, dest, same, old_id) = {
            let Ok(mut g) = state().lock() else {
                return false;
            };
            if g.dest_hwnd == 0 {
                return false;
            }
            let (l, t, r, b) = g.dest;
            if r - l < 4 || b - t < 4 {
                return false;
            }
            g.src_crop = src_crop;
            if g.thumb_id.is_some() && g.src_hwnd == src_hwnd {
                let id = g.thumb_id.unwrap();
                let d = g.dest;
                let crop = g.src_crop;
                drop(g);
                return update_props(id, src_hwnd, d, crop);
            }
            let old = g.thumb_id.take();
            g.src_hwnd = src_hwnd;
            (g.dest_hwnd, g.dest, false, old)
        };
        let _ = same;

        if let Some(id) = old_id {
            unsafe {
                let _ = DwmUnregisterThumbnail(id);
            }
        }

        let dest_h = HWND(dest_hwnd as _);
        let thumb = unsafe { DwmRegisterThumbnail(dest_h, src) };
        let Ok(id) = thumb else {
            return false;
        };
        if !update_props(id, src_hwnd, dest, src_crop) {
            unsafe {
                let _ = DwmUnregisterThumbnail(id);
            }
            return false;
        }
        if let Ok(mut g) = state().lock() {
            g.thumb_id = Some(id);
            g.src_hwnd = src_hwnd;
            g.dest_hwnd = dest_hwnd;
            g.src_crop = src_crop;
            g.dest = dest;
        }
        true
    }
    #[cfg(not(windows))]
    {
        let _ = (src_hwnd, src_crop);
        false
    }
}

#[cfg(windows)]
fn update_props(
    thumb_id: isize,
    _src: isize,
    dest: (i32, i32, i32, i32),
    src_crop: Option<(i32, i32, i32, i32)>,
) -> bool {
    use windows::Win32::Foundation::{BOOL, RECT};
    use windows::Win32::Graphics::Dwm::{
        DwmQueryThumbnailSourceSize, DwmUpdateThumbnailProperties, DWM_THUMBNAIL_PROPERTIES,
        DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION, DWM_TNP_RECTSOURCE, DWM_TNP_SOURCECLIENTAREAONLY,
        DWM_TNP_VISIBLE,
    };

    let (l, t, r, b) = dest;
    let dw = (r - l).max(1) as f64;
    let dh = (b - t).max(1) as f64;

    let thumb_sz = unsafe { DwmQueryThumbnailSourceSize(thumb_id) };
    let (full_w, full_h) = match thumb_sz {
        Ok(sz) if sz.cx > 0 && sz.cy > 0 => (sz.cx as f64, sz.cy as f64),
        _ => {
            let props = DWM_THUMBNAIL_PROPERTIES {
                dwFlags: DWM_TNP_VISIBLE
                    | DWM_TNP_RECTDESTINATION
                    | DWM_TNP_OPACITY
                    | DWM_TNP_SOURCECLIENTAREAONLY,
                rcDestination: RECT {
                    left: l,
                    top: t,
                    right: r,
                    bottom: b,
                },
                rcSource: RECT::default(),
                opacity: 255,
                fVisible: BOOL(1),
                fSourceClientAreaOnly: BOOL(0),
            };
            return unsafe { DwmUpdateThumbnailProperties(thumb_id, &props).is_ok() };
        }
    };

    // 有字裁切；失败则取源窗底部分行带（桌面歌词字常贴底，整窗映射会又小又偏下）
    let (origin_l, origin_t, sw0, sh0) = match src_crop {
        Some((sl, st, sr, sb)) if sr - sl >= 4 && sb - st >= 4 => {
            let mut cw = (sr - sl) as f64;
            let mut ch = (sb - st) as f64;
            let mut ol = sl as f64;
            let mut ot = st as f64;
            // PrintWindow 尺寸与 DWM 源尺寸不一致时按比例映射
            if (cw > full_w + 2.0 || ch > full_h + 2.0) && full_w > 0.0 && full_h > 0.0 {
                // crop 可能相对更大的 frame；若明显超出则夹紧
                ol = ol.clamp(0.0, full_w - 1.0);
                ot = ot.clamp(0.0, full_h - 1.0);
                cw = cw.min(full_w - ol);
                ch = ch.min(full_h - ot);
            }
            (ol, ot, cw.max(1.0), ch.max(1.0))
        }
        _ => {
            // DesktopLyrics 常把字画在窗底部；整窗映射会又小又贴底
            let band = (dh * 1.4).clamp(28.0, 72.0).min(full_h).round().max(16.0);
            let ot = (full_h - band).max(0.0);
            (0.0, ot, full_w, band)
        }
    };

    // 始终撑满槽位高度；长句过宽则水平居中裁源
    let scale = dh / sh0.max(1.0);
    let max_sw = dw / scale;
    let (draw_sw, trim_x) = if sw0 > max_sw + 0.5 {
        (max_sw, ((sw0 - max_sw) / 2.0).round().max(0.0))
    } else {
        (sw0, 0.0)
    };
    let tw = (draw_sw * scale).round().max(1.0).min(dw);
    let th = dh;
    let ox = ((dw - tw) / 2.0).round() as i32;
    // 垂直：目的矩形铺满槽位（oy=0），源已裁成单行，视觉上相对岛栏居中
    let oy = 0i32;

    let src_l = (origin_l + trim_x).round() as i32;
    let src_r = (origin_l + trim_x + draw_sw).round() as i32;
    let src_t = origin_t.round() as i32;
    let src_b = (origin_t + sh0).round() as i32;

    let props = DWM_THUMBNAIL_PROPERTIES {
        dwFlags: DWM_TNP_VISIBLE
            | DWM_TNP_RECTDESTINATION
            | DWM_TNP_OPACITY
            | DWM_TNP_SOURCECLIENTAREAONLY
            | DWM_TNP_RECTSOURCE,
        rcDestination: RECT {
            left: l + ox,
            top: t + oy,
            right: l + ox + tw as i32,
            bottom: t + oy + th as i32,
        },
        rcSource: RECT {
            left: src_l,
            top: src_t,
            right: src_r,
            bottom: src_b,
        },
        opacity: 255,
        fVisible: BOOL(1),
        fSourceClientAreaOnly: BOOL(0),
    };
    unsafe { DwmUpdateThumbnailProperties(thumb_id, &props).is_ok() }
}
