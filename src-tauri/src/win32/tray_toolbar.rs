//! Read live Shell_NotifyIcon hwnd/callback from Explorer ToolbarWindow32.
//! Used to upgrade soft-seed stubs so Hub can synthesize clicks like PixPin.
//!
//! Win10: main + overflow toolbars expose TbButton payloads.
//! Win11: overflow may be XAML (no TB_BUTTONCOUNT); main notify toolbar still works
//! for promoted icons. Hidden Shell_TrayWnd is briefly shown before the scan.

#![cfg(windows)]

use std::ffi::c_void;

use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_ALL_ACCESS};
use windows::Win32::System::Memory::{
    VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, PAGE_READWRITE,
};
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::UI::Controls::{TBSTATE_HIDDEN, TB_BUTTONCOUNT, TB_GETBUTTON, TBBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetWindowThreadProcessId, IsWindowVisible, SendMessageW,
    ShowWindow, SW_SHOWNA,
};

/// One live tray icon resolved from Explorer's toolbar shared memory.
#[derive(Debug, Clone)]
pub struct ToolbarIcon {
    pub hwnd: isize,
    pub uid: u32,
    pub callback_msg: u32,
    pub version: u32,
    pub guid: Option<String>,
    pub tooltip: String,
    pub exe_name: String,
    pub is_visible: bool,
}

#[repr(C)]
struct TbButtonItem {
    window_handle: isize,
    uid: u32,
    callback_message: u32,
    state: u32,
    version: u32,
    icon_handle: isize,
    icon_demote_timer_id: isize,
    user_pref: u32,
    last_sound_time: u32,
    exe_name: [u16; 260],
    icon_text: [u16; 260],
    num_seconds: u32,
    guid_item: windows::core::GUID,
}

fn u16_zstring(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
        .replace('\r', "")
        .trim()
        .to_string()
}

fn guid_string(g: &windows::core::GUID) -> Option<String> {
    if g == &windows::core::GUID::default() {
        return None;
    }
    // GUID Display format lowercase without braces — matches Hub pin keys.
    let s = format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        g.data1,
        g.data2,
        g.data3,
        g.data4[0],
        g.data4[1],
        g.data4[2],
        g.data4[3],
        g.data4[4],
        g.data4[5],
        g.data4[6],
        g.data4[7],
    );
    if s == "00000000-0000-0000-0000-000000000000" {
        None
    } else {
        Some(s)
    }
}

fn find_shell_tray() -> Option<HWND> {
    unsafe {
        let mut after = HWND::default();
        loop {
            let hwnd = FindWindowExW(HWND::default(), after, w!("Shell_TrayWnd"), None).ok()?;
            if FindWindowExW(hwnd, HWND::default(), w!("TrayNotifyWnd"), None).is_ok() {
                return Some(hwnd);
            }
            after = hwnd;
        }
    }
}

fn find_main_toolbar(tray: HWND) -> Option<HWND> {
    unsafe {
        let notify = FindWindowExW(tray, None, w!("TrayNotifyWnd"), None).ok()?;
        let pager = FindWindowExW(notify, None, w!("SysPager"), None).ok()?;
        FindWindowExW(pager, None, w!("ToolbarWindow32"), None).ok()
    }
}

fn find_overflow_toolbar() -> Option<HWND> {
    unsafe {
        let notify = FindWindowW(w!("NotifyIconOverflowWindow"), None).ok()?;
        FindWindowExW(notify, None, w!("ToolbarWindow32"), None).ok()
    }
}

fn read_button(
    process: HANDLE,
    buffer: *mut c_void,
    toolbar: HWND,
    index: usize,
) -> Option<ToolbarIcon> {
    unsafe {
        SendMessageW(
            toolbar,
            TB_GETBUTTON,
            WPARAM(index),
            LPARAM(buffer as isize),
        );
        let mut button: TBBUTTON = std::mem::zeroed();
        ReadProcessMemory(
            process,
            buffer,
            &mut button as *mut _ as _,
            std::mem::size_of::<TBBUTTON>(),
            None,
        )
        .ok()?;
        if button.dwData == 0 {
            return None;
        }
        let mut item: TbButtonItem = std::mem::zeroed();
        ReadProcessMemory(
            process,
            button.dwData as _,
            &mut item as *mut _ as _,
            std::mem::size_of::<TbButtonItem>(),
            None,
        )
        .ok()?;
        if item.window_handle == 0 || item.callback_message == 0 {
            return None;
        }
        let version = if (1..=4).contains(&item.version) {
            item.version
        } else {
            0
        };
        Some(ToolbarIcon {
            hwnd: item.window_handle,
            uid: item.uid,
            callback_msg: item.callback_message,
            version,
            guid: guid_string(&item.guid_item),
            tooltip: u16_zstring(&item.icon_text),
            exe_name: u16_zstring(&item.exe_name),
            is_visible: button.fsState & TBSTATE_HIDDEN as u8 == 0,
        })
    }
}

fn scan_toolbar(process: HANDLE, buffer: *mut c_void, toolbar: HWND, out: &mut Vec<ToolbarIcon>) {
    let count = unsafe { SendMessageW(toolbar, TB_BUTTONCOUNT, None, None) }.0;
    if count <= 0 {
        return;
    }
    for index in 0..count as usize {
        if let Some(icon) = read_button(process, buffer, toolbar, index) {
            out.push(icon);
        }
    }
}

/// Enumerate clickable tray icons from Explorer toolbars (main + Win10 overflow).
/// Briefly reveals the shell tray so ToolbarWindow32 is reachable while Hub hides it.
pub fn enumerate_live_icons() -> Result<Vec<ToolbarIcon>, String> {
    let tray = find_shell_tray().ok_or_else(|| "Shell_TrayWnd not found".to_string())?;
    let was_visible = unsafe { IsWindowVisible(tray).as_bool() };
    if !was_visible {
        unsafe {
            let _ = ShowWindow(tray, SW_SHOWNA);
        }
        std::thread::sleep(std::time::Duration::from_millis(60));
    }

    let result = (|| {
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(tray, Some(&mut pid));
        }
        if pid == 0 {
            return Err("explorer pid unknown".into());
        }
        let process = unsafe { OpenProcess(PROCESS_ALL_ACCESS, false, pid) }
            .map_err(|e| e.to_string())?;

        let buffer = unsafe {
            VirtualAllocEx(
                process,
                None,
                std::mem::size_of::<TBBUTTON>(),
                MEM_COMMIT,
                PAGE_READWRITE,
            )
        };
        if buffer.is_null() {
            let _ = unsafe { CloseHandle(process) };
            return Err("VirtualAllocEx failed".into());
        }

        let mut icons = Vec::new();
        if let Some(tb) = find_main_toolbar(tray) {
            scan_toolbar(process, buffer, tb, &mut icons);
        }
        if let Some(tb) = find_overflow_toolbar() {
            scan_toolbar(process, buffer, tb, &mut icons);
        }

        unsafe {
            let _ = VirtualFreeEx(process, buffer, 0, MEM_RELEASE);
            let _ = CloseHandle(process);
        }
        Ok(icons)
    })();

    if !was_visible {
        // Keep-hidden loop will re-hide; do not force SW_HIDE here if a click
        // hold_taskbar_for_tray lease is active — status_menu owns that policy.
    }

    result
}
