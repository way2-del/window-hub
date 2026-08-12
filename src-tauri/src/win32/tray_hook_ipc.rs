//! Shared IPC layout between `window_hub_trayhook.dll` (in explorer) and the host.
//!
//! Keep this file free of Windows API deps so both sides can include the same
//! `#[repr(C)]` layout (duplicated constants in the cdylib for isolation).

#![allow(dead_code)]

pub const SHM_NAME: &str = "Local\\WindowHubTrayHookV2";
pub const EVENT_NAME: &str = "Local\\WindowHubTrayHookEventV2";
pub const MAGIC: u32 = 0x5748_5452; // 'WHTR'
pub const SLOT_COUNT: usize = 256;
pub const ICON_PIXELS: usize = 48 * 48;
/// Slot buffer is 48×48; trayhook currently captures **32×32** via DrawIconEx
/// and sets `icon_w`/`icon_h` accordingly (packed at the start of `icon_rgba`).
pub const ICON_BYTES: usize = ICON_PIXELS * 4;
pub const TOOLTIP_LEN: usize = 128;

/// NIM_* (shellapi)
pub const NIM_ADD: u32 = 0;
pub const NIM_MODIFY: u32 = 1;
pub const NIM_DELETE: u32 = 2;
pub const NIM_SETVERSION: u32 = 4;

pub const NIF_MESSAGE: u32 = 0x0000_0001;
pub const NIF_ICON: u32 = 0x0000_0002;
pub const NIF_TIP: u32 = 0x0000_0004;
pub const NIF_STATE: u32 = 0x0000_0008;
pub const NIF_GUID: u32 = 0x0000_0020;
pub const NIS_HIDDEN: u32 = 0x0000_0001;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TrayHookSlot {
    pub seq: u32,
    pub ready: u32,
    pub message_type: u32,
    pub flags: u32,
    pub state: u32,
    pub version: u32,
    pub uid: u32,
    pub callback_msg: u32,
    pub hwnd: u64,
    pub icon_w: u32,
    pub icon_h: u32,
    pub guid: [u8; 16],
    pub has_guid: u32,
    pub tooltip: [u16; TOOLTIP_LEN],
    pub icon_rgba: [u8; ICON_BYTES],
}

impl TrayHookSlot {
    pub const fn empty() -> Self {
        Self {
            seq: 0,
            ready: 0,
            message_type: 0,
            flags: 0,
            state: 0,
            version: 0,
            uid: 0,
            callback_msg: 0,
            hwnd: 0,
            icon_w: 0,
            icon_h: 0,
            guid: [0; 16],
            has_guid: 0,
            tooltip: [0; TOOLTIP_LEN],
            icon_rgba: [0; ICON_BYTES],
        }
    }
}

#[repr(C)]
pub struct TrayHookShared {
    pub magic: u32,
    pub write_idx: u32,
    pub read_idx: u32,
    pub slots: [TrayHookSlot; SLOT_COUNT],
}

impl TrayHookShared {
    pub fn init(&mut self) {
        self.magic = MAGIC;
        self.write_idx = 0;
        self.read_idx = 0;
        for s in &mut self.slots {
            *s = TrayHookSlot::empty();
        }
    }
}

pub const SHM_SIZE: usize = std::mem::size_of::<TrayHookShared>();
