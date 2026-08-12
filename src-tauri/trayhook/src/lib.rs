//! Explorer-side WH_CALLWNDPROC hook that forwards tray WM_COPYDATA to the host.
//!
//! Aligns with MyDockFinder's `GetMsgProc_TRAY` idea: observe tray traffic inside
//! explorer, then IPC out. We use WH_CALLWNDPROC because Shell_NotifyIcon uses
//! SendMessage(WM_COPYDATA), which does not go through GetMessage.
//!
//! Hot path rules (Win10 + MyDockFinder):
//! - Never full-size GetDIBits / heap alloc inside CallWndProc
//! - DrawIconEx into a fixed 32×32 DIB
//! - Persist SHM/Event mapping across messages
//! - Skip RGBA copy when icon_handle is unchanged for the same hwnd:uid

mod ipc;

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering};

use ipc::*;
use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, DIB_RGB_COLORS, HBRUSH, HGDIOBJ,
};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
use windows::Win32::System::Memory::{
    MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_ALL_ACCESS,
};
use windows::Win32::System::Threading::{OpenEventW, SetEvent, EVENT_MODIFY_STATE};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DrawIconEx, CWPSTRUCT, DI_NORMAL, HHOOK, HICON, WM_COPYDATA,
};

const CAPTURE_PX: i32 = 32;

static H_INSTANCE: AtomicIsize = AtomicIsize::new(0);
static SEQ: AtomicU32 = AtomicU32::new(1);

/// Persistent IPC — open once, reuse (re-open on failure).
static SHM_MAPPING: AtomicIsize = AtomicIsize::new(0);
static SHM_VIEW: AtomicIsize = AtomicIsize::new(0);
static EVENT_HANDLE: AtomicIsize = AtomicIsize::new(0);
static IPC_LOCK: AtomicBool = AtomicBool::new(false);

/// icon_handle dedup: key = (hwnd << 32) | uid → last handle.
const DEDUP_SLOTS: usize = 128;
static DEDUP_KEY: [AtomicU64; DEDUP_SLOTS] = [const { AtomicU64::new(0) }; DEDUP_SLOTS];
static DEDUP_HANDLE: [AtomicU32; DEDUP_SLOTS] = [const { AtomicU32::new(0) }; DEDUP_SLOTS];

#[no_mangle]
pub unsafe extern "system" fn DllMain(
    module: HINSTANCE,
    reason: u32,
    _reserved: *mut core::ffi::c_void,
) -> i32 {
    const DLL_PROCESS_ATTACH: u32 = 1;
    const DLL_PROCESS_DETACH: u32 = 0;
    if reason == DLL_PROCESS_ATTACH {
        H_INSTANCE.store(module.0 as isize, Ordering::SeqCst);
        let _ = DisableThreadLibraryCalls(module);
    } else if reason == DLL_PROCESS_DETACH {
        release_ipc();
    }
    1
}

/// Exported hook procedure (install with WH_CALLWNDPROC).
#[no_mangle]
pub unsafe extern "system" fn GetMsgProc_Tray(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 {
        let cwp = &*(lparam.0 as *const CWPSTRUCT);
        if cwp.message == WM_COPYDATA {
            handle_copydata(cwp.hwnd, cwp.lParam);
        }
    }
    CallNextHookEx(HHOOK(std::ptr::null_mut()), code, wparam, lparam)
}

fn spin_lock() {
    while IPC_LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        std::hint::spin_loop();
    }
}

fn spin_unlock() {
    IPC_LOCK.store(false, Ordering::Release);
}

unsafe fn release_ipc() {
    spin_lock();
    let view = SHM_VIEW.swap(0, Ordering::SeqCst);
    if view != 0 {
        let _ = UnmapViewOfFile(windows::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS {
            Value: view as *mut _,
        });
    }
    let mapping = SHM_MAPPING.swap(0, Ordering::SeqCst);
    if mapping != 0 {
        let _ = windows::Win32::Foundation::CloseHandle(
            windows::Win32::Foundation::HANDLE(mapping as *mut _),
        );
    }
    let ev = EVENT_HANDLE.swap(0, Ordering::SeqCst);
    if ev != 0 {
        let _ = windows::Win32::Foundation::CloseHandle(
            windows::Win32::Foundation::HANDLE(ev as *mut _),
        );
    }
    spin_unlock();
}

/// Returns shared memory pointer or null. Holds no lock after return (view is stable).
unsafe fn ensure_shm() -> *mut TrayHookShared {
    let view = SHM_VIEW.load(Ordering::Acquire);
    if view != 0 {
        let shared = &*(view as *const TrayHookShared);
        if shared.magic == MAGIC {
            return view as *mut TrayHookShared;
        }
    }

    spin_lock();
    // Re-check under lock.
    let view = SHM_VIEW.load(Ordering::Acquire);
    if view != 0 {
        let shared = &*(view as *const TrayHookShared);
        if shared.magic == MAGIC {
            spin_unlock();
            return view as *mut TrayHookShared;
        }
        // Stale — tear down.
        let _ = UnmapViewOfFile(windows::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS {
            Value: view as *mut _,
        });
        SHM_VIEW.store(0, Ordering::Release);
        let mapping = SHM_MAPPING.swap(0, Ordering::SeqCst);
        if mapping != 0 {
            let _ = windows::Win32::Foundation::CloseHandle(
                windows::Win32::Foundation::HANDLE(mapping as *mut _),
            );
        }
    }

    let mapping = match OpenFileMappingW(FILE_MAP_ALL_ACCESS.0, false, w!("Local\\WindowHubTrayHookV2"))
    {
        Ok(h) => h,
        Err(_) => {
            spin_unlock();
            return std::ptr::null_mut();
        }
    };
    let mapped = MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, SHM_SIZE);
    if mapped.Value.is_null() {
        let _ = windows::Win32::Foundation::CloseHandle(mapping);
        spin_unlock();
        return std::ptr::null_mut();
    }
    let shared = &*(mapped.Value as *const TrayHookShared);
    if shared.magic != MAGIC {
        let _ = UnmapViewOfFile(mapped);
        let _ = windows::Win32::Foundation::CloseHandle(mapping);
        spin_unlock();
        return std::ptr::null_mut();
    }
    SHM_MAPPING.store(mapping.0 as isize, Ordering::Release);
    SHM_VIEW.store(mapped.Value as isize, Ordering::Release);
    spin_unlock();
    mapped.Value as *mut TrayHookShared
}

unsafe fn signal_event() {
    let mut ev = EVENT_HANDLE.load(Ordering::Acquire);
    if ev == 0 {
        spin_lock();
        ev = EVENT_HANDLE.load(Ordering::Acquire);
        if ev == 0 {
            if let Ok(h) = OpenEventW(EVENT_MODIFY_STATE, false, w!("Local\\WindowHubTrayHookEventV2"))
            {
                EVENT_HANDLE.store(h.0 as isize, Ordering::Release);
                ev = h.0 as isize;
            }
        }
        spin_unlock();
    }
    if ev != 0 {
        let _ = SetEvent(windows::Win32::Foundation::HANDLE(ev as *mut _));
    }
}

fn dedup_key(hwnd: u64, uid: u32) -> u64 {
    (hwnd << 32) | (uid as u64)
}

fn icon_handle_unchanged(key: u64, handle: u32) -> bool {
    let slot = (key as usize) % DEDUP_SLOTS;
    let existing_key = DEDUP_KEY[slot].load(Ordering::Acquire);
    let existing_handle = DEDUP_HANDLE[slot].load(Ordering::Acquire);
    existing_key == key && existing_handle == handle && handle != 0
}

fn remember_icon_handle(key: u64, handle: u32) {
    let slot = (key as usize) % DEDUP_SLOTS;
    DEDUP_KEY[slot].store(key, Ordering::Release);
    DEDUP_HANDLE[slot].store(handle, Ordering::Release);
}

fn forget_icon_handle(key: u64) {
    let slot = (key as usize) % DEDUP_SLOTS;
    if DEDUP_KEY[slot].load(Ordering::Acquire) == key {
        DEDUP_KEY[slot].store(0, Ordering::Release);
        DEDUP_HANDLE[slot].store(0, Ordering::Release);
    }
}

unsafe fn handle_copydata(_hwnd: HWND, lparam: LPARAM) {
    let cds = match (lparam.0 as *const COPYDATASTRUCT).as_ref() {
        Some(c) => c,
        None => return,
    };
    // Zebar/Seelen: dwData == 1 carries ShellTrayMessage (NIM_*).
    if cds.dwData != 1 || cds.lpData.is_null() || cds.cbData == 0 {
        return;
    }
    if (cds.cbData as usize) < std::mem::size_of::<ShellTrayMessage>() {
        return;
    }
    let msg = &*(cds.lpData as *const ShellTrayMessage);
    match msg.message_type {
        NIM_ADD | NIM_MODIFY | NIM_DELETE | NIM_SETVERSION => {}
        _ => return,
    }

    let shared = ensure_shm();
    if shared.is_null() {
        return;
    }
    let shared = &mut *shared;

    // Drop if ring full — never overwrite an unread slot (avoids tip/hwnd tearing).
    let pending = shared.write_idx.wrapping_sub(shared.read_idx);
    if pending as usize >= SLOT_COUNT {
        return;
    }

    let idx = (shared.write_idx as usize) % SLOT_COUNT;
    let slot = &mut shared.slots[idx];
    slot.ready = 0;
    slot.message_type = msg.message_type;
    slot.flags = msg.icon_data.flags;
    slot.state = msg.icon_data.state;
    slot.version = if msg.version > 0 && msg.version <= 4 {
        msg.version
    } else {
        msg.icon_data.uversion_or_timeout
    };
    slot.uid = msg.icon_data.uid;
    slot.callback_msg = msg.icon_data.callback_message;
    slot.hwnd = msg.icon_data.window_handle as u64;
    slot.icon_w = 0;
    slot.icon_h = 0;
    slot.has_guid = 0;
    slot.guid = [0; 16];
    slot.tooltip = [0; TOOLTIP_LEN];

    if msg.icon_data.flags & NIF_TIP != 0 {
        slot.tooltip = msg.icon_data.tooltip;
    }
    if msg.icon_data.flags & NIF_GUID != 0 {
        slot.guid = msg.icon_data.guid_item;
        slot.has_guid = 1;
    }

    let key = dedup_key(slot.hwnd, slot.uid);
    if msg.message_type == NIM_DELETE {
        forget_icon_handle(key);
    } else if msg.icon_data.flags & NIF_ICON != 0 && msg.icon_data.icon_handle != 0 {
        let handle = msg.icon_data.icon_handle;
        if icon_handle_unchanged(key, handle) {
            // Same glyph — strip NIF_ICON so host keeps previous PNG and does not
            // treat this as a blank flash frame (icon_w==0 + NIF_ICON).
            slot.flags &= !NIF_ICON;
        } else if copy_icon_rgba(msg.icon_data.icon_handle as isize, slot) {
            remember_icon_handle(key, handle);
        } else {
            // Failed capture with NIF_ICON left set + icon_w=0 → blank flash (intentional).
            forget_icon_handle(key);
        }
    } else if msg.icon_data.flags & NIF_ICON != 0 && msg.icon_data.icon_handle == 0 {
        // Explicit blank icon frame (blink).
        forget_icon_handle(key);
    }

    slot.seq = SEQ.fetch_add(1, Ordering::Relaxed);
    std::sync::atomic::fence(Ordering::Release);
    slot.ready = 1;
    shared.write_idx = shared.write_idx.wrapping_add(1);

    signal_event();
}

/// Capture tray glyph. Prefer small GetIconInfo (WeChat/QQ mask icons); DrawIconEx
/// for large sources. Never allocate >128² in explorer.
unsafe fn copy_icon_rgba(icon: isize, slot: &mut TrayHookSlot) -> bool {
    if copy_icon_via_info(icon, slot) {
        return true;
    }
    copy_icon_via_draw(icon, slot)
}

fn rgba_has_visible_pixels(rgba: &[u8], w: u32, h: u32) -> bool {
    let n = (w as usize).saturating_mul(h as usize).saturating_mul(4);
    if rgba.len() < n || n == 0 {
        return false;
    }
    rgba[..n].chunks_exact(4).any(|px| px[3] > 8 || px[0] > 8 || px[1] > 8 || px[2] > 8)
}

/// GetIconInfo + GetDIBits for icons up to 128px, scaled into ≤48 slot.
unsafe fn copy_icon_via_info(icon: isize, slot: &mut TrayHookSlot) -> bool {
    use windows::Win32::Graphics::Gdi::{
        DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, DIB_RGB_COLORS, HGDIOBJ,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, ICONINFO, HICON};

    let mut info = ICONINFO::default();
    if GetIconInfo(HICON(icon as *mut _), &mut info).is_err() {
        return false;
    }
    if info.hbmColor.is_invalid() {
        let _ = DeleteObject(info.hbmMask);
        return false;
    }

    let mut bm = BITMAP::default();
    if GetObjectW(
        HGDIOBJ(info.hbmColor.0),
        std::mem::size_of::<BITMAP>() as i32,
        Some(&mut bm as *mut _ as *mut _),
    ) == 0
    {
        let _ = DeleteObject(info.hbmColor);
        let _ = DeleteObject(info.hbmMask);
        return false;
    }

    let width = bm.bmWidth;
    let height = bm.bmHeight.abs();
    // Bound explorer work — larger icons go through DrawIconEx instead.
    if width <= 0 || height <= 0 || width > 128 || height > 128 {
        let _ = DeleteObject(info.hbmColor);
        let _ = DeleteObject(info.hbmMask);
        return false;
    }

    let buffer_size = (width as usize).saturating_mul(height as usize).saturating_mul(4);
    let mut color_buffer = vec![0u8; buffer_size];
    let mut mask_buffer = vec![0u8; buffer_size];

    let dc = GetDC(None);
    if dc.is_invalid() {
        let _ = DeleteObject(info.hbmColor);
        let _ = DeleteObject(info.hbmMask);
        return false;
    }

    let mut bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0,
            ..Default::default()
        },
        ..Default::default()
    };

    let color_ok = GetDIBits(
        dc,
        info.hbmColor,
        0,
        height as u32,
        Some(color_buffer.as_mut_ptr() as *mut _),
        &mut bi,
        DIB_RGB_COLORS,
    );
    let mask_ok = if !info.hbmMask.is_invalid() {
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = width;
        bi.bmiHeader.biHeight = -height;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = 0;
        GetDIBits(
            dc,
            info.hbmMask,
            0,
            height as u32,
            Some(mask_buffer.as_mut_ptr() as *mut _),
            &mut bi,
            DIB_RGB_COLORS,
        )
    } else {
        0
    };

    let _ = ReleaseDC(None, dc);
    let _ = DeleteObject(info.hbmColor);
    let _ = DeleteObject(info.hbmMask);

    if color_ok == 0 {
        return false;
    }

    let is_mask_based = color_buffer.chunks_exact(4).all(|chunk| chunk[3] == 0);
    for (index, chunk) in color_buffer.chunks_exact_mut(4).enumerate() {
        chunk.swap(0, 2); // BGR → RGB
        if is_mask_based && mask_ok != 0 {
            let mask_alpha = mask_buffer[index * 4];
            chunk[3] = if mask_alpha == 255 { 0 } else { 255 };
        }
    }

    let max_dim = 48i32;
    let scale = (width.max(height) as f32 / max_dim as f32).max(1.0);
    let out_w = ((width as f32) / scale).round().max(1.0) as i32;
    let out_h = ((height as f32) / scale).round().max(1.0) as i32;
    let out_w = out_w.min(max_dim);
    let out_h = out_h.min(max_dim);

    slot.icon_rgba = [0; ICON_BYTES];
    for y in 0..out_h {
        for x in 0..out_w {
            let sx = ((x as f32 + 0.5) * scale).floor() as i32;
            let sy = ((y as f32 + 0.5) * scale).floor() as i32;
            let sx = sx.clamp(0, width - 1) as usize;
            let sy = sy.clamp(0, height - 1) as usize;
            let src = (sy * width as usize + sx) * 4;
            let dst = (y as usize * out_w as usize + x as usize) * 4;
            if src + 4 <= color_buffer.len() && dst + 4 <= ICON_BYTES {
                slot.icon_rgba[dst..dst + 4].copy_from_slice(&color_buffer[src..src + 4]);
            }
        }
    }
    slot.icon_w = out_w as u32;
    slot.icon_h = out_h as u32;
    rgba_has_visible_pixels(&slot.icon_rgba, slot.icon_w, slot.icon_h)
}

/// DrawIconEx into fixed 32×32 — for large icons / when GetIconInfo is unavailable.
unsafe fn copy_icon_via_draw(icon: isize, slot: &mut TrayHookSlot) -> bool {
    let hdc_screen = windows::Win32::Graphics::Gdi::GetDC(None);
    if hdc_screen.is_invalid() {
        return false;
    }
    let hdc = CreateCompatibleDC(hdc_screen);
    if hdc.is_invalid() {
        let _ = windows::Win32::Graphics::Gdi::ReleaseDC(None, hdc_screen);
        return false;
    }

    let mut bits_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
    let bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: CAPTURE_PX,
            biHeight: -CAPTURE_PX, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0,
            ..Default::default()
        },
        ..Default::default()
    };

    let hbmp = CreateDIBSection(hdc, &bi, DIB_RGB_COLORS, &mut bits_ptr, None, 0);
    if hbmp.is_err() || bits_ptr.is_null() {
        let _ = DeleteDC(hdc);
        let _ = windows::Win32::Graphics::Gdi::ReleaseDC(None, hdc_screen);
        return false;
    }
    let hbmp = hbmp.unwrap();
    let old = SelectObject(hdc, HGDIOBJ(hbmp.0));

    let pixel_count = (CAPTURE_PX * CAPTURE_PX) as usize;
    let bits = std::slice::from_raw_parts_mut(bits_ptr as *mut u8, pixel_count * 4);
    bits.fill(0);

    let drawn = DrawIconEx(
        hdc,
        0,
        0,
        HICON(icon as *mut _),
        CAPTURE_PX,
        CAPTURE_PX,
        0,
        HBRUSH(std::ptr::null_mut()),
        DI_NORMAL,
    )
    .is_ok();

    slot.icon_rgba = [0; ICON_BYTES];
    let mut ok = false;
    if drawn {
        for i in 0..pixel_count {
            let src = i * 4;
            let dst = i * 4;
            slot.icon_rgba[dst] = bits[src + 2];
            slot.icon_rgba[dst + 1] = bits[src + 1];
            slot.icon_rgba[dst + 2] = bits[src];
            slot.icon_rgba[dst + 3] = bits[src + 3];
        }
        slot.icon_w = CAPTURE_PX as u32;
        slot.icon_h = CAPTURE_PX as u32;
        ok = rgba_has_visible_pixels(&slot.icon_rgba, slot.icon_w, slot.icon_h);
        if !ok {
            slot.icon_w = 0;
            slot.icon_h = 0;
        }
    }

    let _ = SelectObject(hdc, old);
    let _ = DeleteObject(hbmp);
    let _ = DeleteDC(hdc);
    let _ = windows::Win32::Graphics::Gdi::ReleaseDC(None, hdc_screen);

    ok
}

/// Host helper: module base for SetWindowsHookEx.
#[no_mangle]
pub unsafe extern "system" fn TrayHook_GetModule() -> HINSTANCE {
    HINSTANCE(H_INSTANCE.load(Ordering::SeqCst) as *mut _)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_key_packs_hwnd_uid() {
        assert_eq!(dedup_key(0xAABB, 0x11), (0xAABBu64 << 32) | 0x11);
    }

    #[test]
    fn icon_handle_dedup_roundtrip() {
        let key = dedup_key(42, 7);
        forget_icon_handle(key);
        assert!(!icon_handle_unchanged(key, 0x100));
        remember_icon_handle(key, 0x100);
        assert!(icon_handle_unchanged(key, 0x100));
        assert!(!icon_handle_unchanged(key, 0x200));
        remember_icon_handle(key, 0x200);
        assert!(icon_handle_unchanged(key, 0x200));
        forget_icon_handle(key);
        assert!(!icon_handle_unchanged(key, 0x200));
    }
}
