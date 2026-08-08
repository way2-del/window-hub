//! Explorer-side WH_CALLWNDPROC hook that forwards tray WM_COPYDATA to the host.
//!
//! Aligns with MyDockFinder's `GetMsgProc_TRAY` idea: observe tray traffic inside
//! explorer, then IPC out. We use WH_CALLWNDPROC because Shell_NotifyIcon uses
//! SendMessage(WM_COPYDATA), which does not go through GetMessage.

mod ipc;

use std::sync::atomic::{AtomicIsize, Ordering};

use ipc::*;
use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
use windows::Win32::System::Memory::{
    MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_ALL_ACCESS,
};
use windows::Win32::System::Threading::{OpenEventW, SetEvent, EVENT_MODIFY_STATE};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetIconInfo, ICONINFO, CWPSTRUCT, HHOOK, HICON, WM_COPYDATA,
};

static H_INSTANCE: AtomicIsize = AtomicIsize::new(0);
static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

#[no_mangle]
pub unsafe extern "system" fn DllMain(
    module: HINSTANCE,
    reason: u32,
    _reserved: *mut core::ffi::c_void,
) -> i32 {
    const DLL_PROCESS_ATTACH: u32 = 1;
    if reason == DLL_PROCESS_ATTACH {
        H_INSTANCE.store(module.0 as isize, Ordering::SeqCst);
        let _ = DisableThreadLibraryCalls(module);
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

    let mapping = match OpenFileMappingW(FILE_MAP_ALL_ACCESS.0, false, w!("Local\\WindowHubTrayHookV2")) {
        Ok(h) => h,
        Err(_) => return,
    };
    let view = MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, SHM_SIZE);
    if view.Value.is_null() {
        let _ = windows::Win32::Foundation::CloseHandle(mapping);
        return;
    }
    let shared = &mut *(view.Value as *mut TrayHookShared);
    if shared.magic != MAGIC {
        let _ = UnmapViewOfFile(view);
        let _ = windows::Win32::Foundation::CloseHandle(mapping);
        return;
    }

    // Drop if ring full — never overwrite an unread slot (avoids tip/hwnd tearing).
    let pending = shared.write_idx.wrapping_sub(shared.read_idx);
    if pending as usize >= SLOT_COUNT {
        let _ = UnmapViewOfFile(view);
        let _ = windows::Win32::Foundation::CloseHandle(mapping);
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
    if msg.icon_data.flags & NIF_ICON != 0 && msg.icon_data.icon_handle != 0 {
        copy_icon_rgba(msg.icon_data.icon_handle as isize, slot);
    }

    slot.seq = SEQ.fetch_add(1, Ordering::Relaxed);
    std::sync::atomic::fence(Ordering::Release);
    slot.ready = 1;
    shared.write_idx = shared.write_idx.wrapping_add(1);

    let _ = UnmapViewOfFile(view);
    let _ = windows::Win32::Foundation::CloseHandle(mapping);

    if let Ok(ev) = OpenEventW(EVENT_MODIFY_STATE, false, w!("Local\\WindowHubTrayHookEventV2")) {
        let _ = SetEvent(ev);
        let _ = windows::Win32::Foundation::CloseHandle(ev);
    }
}

unsafe fn copy_icon_rgba(icon: isize, slot: &mut TrayHookSlot) {
    let mut info = ICONINFO::default();
    if GetIconInfo(HICON(icon as *mut _), &mut info).is_err() {
        return;
    }
    if info.hbmColor.is_invalid() {
        let _ = DeleteObject(info.hbmMask);
        return;
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
        return;
    }

    // Use FULL bitmap size for GetDIBits. Clamping biWidth/Height to 48 made
    // GetDIBits return only the top-left corner of 128/256px icons (Clash, etc.).
    let width = bm.bmWidth;
    let height = bm.bmHeight.abs();
    if width <= 0 || height <= 0 || width > 512 || height > 512 {
        let _ = DeleteObject(info.hbmColor);
        let _ = DeleteObject(info.hbmMask);
        return;
    }

    let buffer_size = (width as usize).saturating_mul(height as usize).saturating_mul(4);
    let mut color_buffer = vec![0u8; buffer_size];
    let mut mask_buffer = vec![0u8; buffer_size];

    let dc = GetDC(None);
    if dc.is_invalid() {
        let _ = DeleteObject(info.hbmColor);
        let _ = DeleteObject(info.hbmMask);
        return;
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
        // bi may be mutated by GetDIBits — reset size fields.
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
        return;
    }

    let is_mask_based = color_buffer.chunks_exact(4).all(|chunk| chunk[3] == 0);
    for (index, chunk) in color_buffer.chunks_exact_mut(4).enumerate() {
        chunk.swap(0, 2); // BGR → RGB
        if is_mask_based && mask_ok != 0 {
            let mask_alpha = mask_buffer[index * 4];
            chunk[3] = if mask_alpha == 255 { 0 } else { 255 };
        }
    }

    // Downscale into the fixed 48×48 slot buffer (nearest-neighbor).
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
}

/// Host helper: module base for SetWindowsHookEx.
#[no_mangle]
pub unsafe extern "system" fn TrayHook_GetModule() -> HINSTANCE {
    HINSTANCE(H_INSTANCE.load(Ordering::SeqCst) as *mut _)
}
