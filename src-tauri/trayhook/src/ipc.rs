//! IPC layout (must stay in sync with `window_hub` `tray_hook_ipc.rs`).

pub const SHM_NAME: &str = "Local\\WindowHubTrayHookV2";
pub const EVENT_NAME: &str = "Local\\WindowHubTrayHookEventV2";
pub const MAGIC: u32 = 0x5748_5452;
pub const SLOT_COUNT: usize = 256;
pub const ICON_PIXELS: usize = 48 * 48;
pub const ICON_BYTES: usize = ICON_PIXELS * 4;
pub const TOOLTIP_LEN: usize = 128;

pub const NIM_ADD: u32 = 0;
pub const NIM_MODIFY: u32 = 1;
pub const NIM_DELETE: u32 = 2;
pub const NIM_SETVERSION: u32 = 4;

pub const NIF_MESSAGE: u32 = 0x1;
pub const NIF_ICON: u32 = 0x2;
pub const NIF_TIP: u32 = 0x4;
pub const NIF_STATE: u32 = 0x8;
pub const NIF_GUID: u32 = 0x20;

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

pub const SHM_SIZE: usize = core::mem::size_of::<TrayHookShared>();

/// Shell tray COPYDATA payload (32-bit HWND fields), as used by Shell_NotifyIcon.
#[repr(C)]
pub struct ShellTrayMessage {
    pub magic_number: i32,
    pub message_type: u32,
    pub icon_data: NotifyIconData32,
    pub version: u32,
}

#[repr(C)]
pub struct NotifyIconData32 {
    pub callback_size: u32,
    pub window_handle: u32,
    pub uid: u32,
    pub flags: u32,
    pub callback_message: u32,
    pub icon_handle: u32,
    pub tooltip: [u16; 128],
    pub state: u32,
    pub state_mask: u32,
    pub size_info: [u16; 256],
    pub uversion_or_timeout: u32,
    pub info_title: [u16; 64],
    pub info_flags: u32,
    pub guid_item: [u8; 16],
    pub balloon_icon_handle: u32,
}
