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
#[cfg(windows)]
pub mod netease_lyrics;
pub mod park;
pub mod status_menu;
pub mod switcher;
pub mod topmost;
pub mod tray;
#[cfg(windows)]
pub mod system_audio;
#[cfg(windows)]
pub mod system_monitor;
#[cfg(windows)]
pub mod system_perf;
#[cfg(windows)]
pub mod system_memory;
#[cfg(windows)]
pub mod system_net_procs;
#[cfg(windows)]
pub mod system_power;
#[cfg(windows)]
pub mod system_radio;
#[cfg(windows)]
pub mod tray_hook_ipc;
#[cfg(windows)]
pub mod tray_hook_host;
#[cfg(windows)]
pub mod tray_registry;
#[cfg(windows)]
pub mod tray_uia;
