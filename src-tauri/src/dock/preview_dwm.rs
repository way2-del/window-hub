//! Live window thumbnails via DwmRegisterThumbnail (Windows taskbar style).
//!
//! Faster and more accurate than BitBlt/PrintWindow JPEG snapshots: DWM composites
//! the source window directly into our preview HWND.

use std::sync::Mutex;

#[derive(Clone, Copy)]
struct ThumbSlot {
    id: isize,
}

fn active_thumbs() -> &'static Mutex<Vec<ThumbSlot>> {
    static T: std::sync::OnceLock<Mutex<Vec<ThumbSlot>>> = std::sync::OnceLock::new();
    T.get_or_init(|| Mutex::new(Vec::new()))
}

/// Layout constants — must match `DockPreviewApp.css` / `DOCK_PREVIEW_*` in mod.rs.
pub const PREVIEW_PAD: f64 = 10.0;
pub const PREVIEW_GAP: f64 = 8.0;
pub const PREVIEW_CARD_W: f64 = 168.0;
pub const PREVIEW_THUMB_H: f64 = 100.0;
/// Reserved top pad — per-card × lives under thumbs (title row), not a global header.
/// Keep in sync with `.dock-preview-shell` padding-top.
pub const PREVIEW_HEADER: f64 = 10.0;

/// Destination rects (physical px) for N cards inside the preview client area.
pub fn thumb_dest_rects(count: usize, scale: f64) -> Vec<(i32, i32, i32, i32)> {
    let n = count.max(1).min(6);
    let mut out = Vec::with_capacity(n);
    let pad = (PREVIEW_PAD * scale).round() as i32;
    let gap = (PREVIEW_GAP * scale).round() as i32;
    let card_w = (PREVIEW_CARD_W * scale).round() as i32;
    let thumb_h = (PREVIEW_THUMB_H * scale).round() as i32;
    let header = (PREVIEW_HEADER * scale).round() as i32;
    let mut x = pad;
    let y = header;
    for _ in 0..n {
        out.push((x, y, x + card_w, y + thumb_h));
        x += card_w + gap;
    }
    out
}

/// Synchronously resize/move the preview HWND (logical px → physical).
/// Tauri `set_size` can lag a frame; DWM dest rects must see the final client size.
#[cfg(windows)]
pub fn sync_set_bounds(hwnd: isize, x: f64, y: f64, w: f64, h: f64, scale: f64) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER};
    if hwnd == 0 || w <= 0.0 || h <= 0.0 {
        return;
    }
    let px = (x * scale).round() as i32;
    let py = (y * scale).round() as i32;
    let pw = (w * scale).round().max(1.0) as i32;
    let ph = (h * scale).round().max(1.0) as i32;
    unsafe {
        let _ = SetWindowPos(
            HWND(hwnd as _),
            HWND::default(),
            px,
            py,
            pw,
            ph,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

#[cfg(not(windows))]
pub fn sync_set_bounds(_hwnd: isize, _x: f64, _y: f64, _w: f64, _h: f64, _scale: f64) {}

/// Unregister every live thumbnail (call on preview close / hide).
pub fn clear_thumbnails() {
    #[cfg(windows)]
    {
        use windows::Win32::Graphics::Dwm::DwmUnregisterThumbnail;
        let mut guard = active_thumbs().lock().unwrap_or_else(|e| e.into_inner());
        for slot in guard.drain(..) {
            unsafe {
                let _ = DwmUnregisterThumbnail(slot.id);
            }
        }
    }
    #[cfg(not(windows))]
    {
        let mut guard = active_thumbs().lock().unwrap_or_else(|e| e.into_inner());
        guard.clear();
    }
}

/// Register live DWM thumbnails from `sources` into `dest_hwnd`.
/// Returns how many succeeded. Clears any previous registration first.
#[cfg(windows)]
pub fn set_thumbnails(dest_hwnd: isize, sources: &[isize], scale: f64) -> usize {
    use windows::Win32::Foundation::{BOOL, HWND, RECT};
    use windows::Win32::Graphics::Dwm::{
        DwmQueryThumbnailSourceSize, DwmRegisterThumbnail, DwmUpdateThumbnailProperties,
        DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION,
        DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE,
    };

    clear_thumbnails();
    if dest_hwnd == 0 || sources.is_empty() {
        return 0;
    }
    let dest = HWND(dest_hwnd as _);
    let rects = thumb_dest_rects(sources.len(), scale);
    let mut ok = 0usize;
    let mut slots = Vec::new();

    for (i, &src_raw) in sources.iter().take(6).enumerate() {
        if src_raw == 0 {
            continue;
        }
        let Some(&(l, t, r, b)) = rects.get(i) else {
            break;
        };
        let src = HWND(src_raw as _);
        let thumb = unsafe { DwmRegisterThumbnail(dest, src) };
        let Ok(id) = thumb else {
            eprintln!("[dock] DwmRegisterThumbnail failed hwnd={src_raw}");
            continue;
        };

        // Fit source into dest while preserving aspect (letterbox inside card).
        let (dst_l, dst_t, dst_r, dst_b) = unsafe {
            match DwmQueryThumbnailSourceSize(id) {
                Ok(sz) if sz.cx > 0 && sz.cy > 0 => {
                    let dw = (r - l).max(1) as f64;
                    let dh = (b - t).max(1) as f64;
                    let sw = sz.cx as f64;
                    let sh = sz.cy as f64;
                    let scale_fit = (dw / sw).min(dh / sh);
                    let tw = (sw * scale_fit).round().max(1.0);
                    let th = (sh * scale_fit).round().max(1.0);
                    let ox = ((dw - tw) / 2.0).round() as i32;
                    let oy = ((dh - th) / 2.0).round() as i32;
                    (l + ox, t + oy, l + ox + tw as i32, t + oy + th as i32)
                }
                _ => (l, t, r, b),
            }
        };

        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_VISIBLE
                | DWM_TNP_RECTDESTINATION
                | DWM_TNP_OPACITY
                | DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination: RECT {
                left: dst_l,
                top: dst_t,
                right: dst_r,
                bottom: dst_b,
            },
            rcSource: RECT::default(),
            opacity: 255,
            fVisible: BOOL(1),
            fSourceClientAreaOnly: BOOL(0),
        };
        if unsafe { DwmUpdateThumbnailProperties(id, &props) }.is_ok() {
            slots.push(ThumbSlot { id });
            ok += 1;
        } else {
            unsafe {
                let _ = windows::Win32::Graphics::Dwm::DwmUnregisterThumbnail(id);
            }
        }
    }

    if let Ok(mut guard) = active_thumbs().lock() {
        *guard = slots;
    }
    ok
}

#[cfg(not(windows))]
pub fn set_thumbnails(_dest_hwnd: isize, _sources: &[isize], _scale: f64) -> usize {
    0
}
