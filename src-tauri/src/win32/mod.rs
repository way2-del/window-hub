//! Win32 helpers for window enumeration, parking, capture, and input.

pub mod ambient;
pub mod appbar;
#[cfg(windows)]
pub mod blur_glass;
pub mod capture;
pub mod enum_windows;
pub mod fullscreen;
pub mod input;
pub mod material;
pub mod park;
pub mod status_menu;
pub mod switcher;
pub mod topmost;
pub mod tray;
#[cfg(windows)]
pub mod tray_hook_ipc;
#[cfg(windows)]
pub mod tray_hook_host;
#[cfg(windows)]
pub mod tray_registry;
#[cfg(windows)]
pub mod tray_uia;
