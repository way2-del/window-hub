//! 桌面歌词窗 → 灵动岛：DWM 实时缩略图映射（对齐 MyDockFinder「彩色映射」）。
//!
//! `DwmRegisterThumbnail(岛栏 HWND, DesktopLyrics)`，由 DWM 合成源窗口画面。
//! 可选 `rcSource` 裁切有字区域，避免整窗竖条被压扁。
//!
//! - `offset_y`：只挪位置（改源裁切），不改大小
//! - `user_scale`：只改显示大小（相对铺满槽位），不因偏移而缩放

use std::sync::Mutex;

struct MirrorState {
    thumb_id: Option<isize>,
    src_hwnd: isize,
    dest_hwnd: isize,
    /// 目的矩形：相对 dest 客户区，物理像素 (l,t,r,b)
    dest: (i32, i32, i32, i32),
    /// 源裁切：相对源画面归一化 (l,t,r,b)∈[0,1]；None = 底部带 fallback
    src_crop: Option<(f64, f64, f64, f64)>,
    /// 槽内垂直微调（物理像素，正数下移）
    offset_y: i32,
    /// 显示缩放（1.0 = 铺满可用高度）
    user_scale: f64,
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
            offset_y: 0,
            user_scale: 1.0,
        })
    })
}

/// 前端测好岛栏歌词槽位后写入（物理像素，相对岛窗客户区）。
pub fn set_dest_slot(
    dest_hwnd: isize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    offset_y: i32,
    user_scale: f64,
) {
    if dest_hwnd == 0 || w < 4 || h < 4 {
        return;
    }
    let dest = (x, y, x + w, y + h);
    let scale = user_scale.clamp(0.5, 1.5);
    let (thumb_id, src, crop, oy, us) = {
        let Ok(mut g) = state().lock() else {
            return;
        };
        g.dest_hwnd = dest_hwnd;
        g.dest = dest;
        g.offset_y = offset_y;
        g.user_scale = scale;
        (g.thumb_id, g.src_hwnd, g.src_crop, g.offset_y, g.user_scale)
    };
    if let Some(id) = thumb_id {
        if src != 0 {
            let _ = update_props(id, src, dest, crop, oy, us);
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
            g.dest_hwnd = 0;
            g.dest = (0, 0, 0, 0);
            g.src_crop = None;
            g.offset_y = 0;
            g.user_scale = 1.0;
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(mut g) = state().lock() {
            g.thumb_id = None;
            g.src_hwnd = 0;
            g.dest_hwnd = 0;
            g.dest = (0, 0, 0, 0);
            g.src_crop = None;
            g.offset_y = 0;
            g.user_scale = 1.0;
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

/// 把 DesktopLyrics 实时映到已设置的岛栏槽位。
pub fn sync(src_hwnd: isize, src_crop: Option<(f64, f64, f64, f64)>) -> bool {
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

        let (dest_hwnd, dest, same, old_id, oy, us) = {
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
                let oy = g.offset_y;
                let us = g.user_scale;
                drop(g);
                return update_props(id, src_hwnd, d, crop, oy, us);
            }
            let old = g.thumb_id.take();
            g.src_hwnd = src_hwnd;
            (
                g.dest_hwnd,
                g.dest,
                false,
                old,
                g.offset_y,
                g.user_scale,
            )
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
        if !update_props(id, src_hwnd, dest, src_crop, oy, us) {
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
    src_crop: Option<(f64, f64, f64, f64)>,
    offset_y: i32,
    user_scale: f64,
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
    let user_scale = user_scale.clamp(0.5, 1.5);

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

    // 归一化裁切 → 像素；失败则取源窗底部带
    let (origin_l, mut origin_t, sw0, sh0) = match src_crop {
        Some((fl, ft, fr, fb)) if fr > fl + 0.01 && fb > ft + 0.01 => {
            let ol = (fl.clamp(0.0, 1.0) * full_w).round().clamp(0.0, full_w - 1.0);
            let ot = (ft.clamp(0.0, 1.0) * full_h).round().clamp(0.0, full_h - 1.0);
            let or_ = (fr.clamp(0.0, 1.0) * full_w).round().clamp(ol + 1.0, full_w);
            let ob = (fb.clamp(0.0, 1.0) * full_h).round().clamp(ot + 1.0, full_h);
            (ol, ot, (or_ - ol).max(1.0), (ob - ot).max(1.0))
        }
        _ => {
            let band = (dh * 1.6).clamp(32.0, 64.0).min(full_h).round().max(20.0);
            let ot = (full_h - band).max(0.0);
            (0.0, ot, full_w, band)
        }
    };

    // 大小：只由槽位 + user_scale 决定（与偏移无关）
    let edge = if dh < 20.0 { 1.0 } else { 2.0 };
    let inner_h = (dh - edge * 2.0).max(8.0);
    let inner_w = dw.max(8.0);
    let fit = (inner_h / sh0.max(1.0)).min(inner_w / sw0.max(1.0));
    let scale = (fit * user_scale).max(0.05);
    let tw = (sw0 * scale).round().max(1.0);
    let th = (sh0 * scale).round().max(1.0);
    // 允许略超出槽位（放大时裁切边缘），水平仍居中
    let ox = ((dw - tw) / 2.0).round() as i32;
    let oy = ((dh - th) / 2.0).round() as i32;

    // 位置：只挪源裁切，正数下移 → 源窗口上移取样（字在画面里显得更靠下）
    // 换算：目的像素 / 当前缩放 ≈ 源像素
    if offset_y != 0 && scale > 0.01 {
        let src_shift = -(offset_y as f64) / scale;
        let max_t = (full_h - sh0).max(0.0);
        origin_t = (origin_t + src_shift).clamp(0.0, max_t);
    }

    let src_l = origin_l.round() as i32;
    let src_r = (origin_l + sw0).round() as i32;
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
