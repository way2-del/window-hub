//! DWMBlurGlass-inspired materials for a single HWND (no dwm.exe inject).
//!
//! Maps README effects to process-local APIs:
//! - **Blur** → `ACCENT_ENABLE_BLURBEHIND`
//! - **Aero** → `ACCENT_ENABLE_ACRYLICBLURBEHIND` with light tint
//! - **Acrylic** → Win11 `DWMSBT_TRANSIENTWINDOW`, else SWCA acrylic
//! - **Mica** (prefs id `mica-alt` for compat) → system `DWMSBT_MAINWINDOW`
//!   (same Start-menu backdrop — not MicaAlt/tabbed)

#![cfg(windows)]

use std::ffi::c_void;
use tauri::WebviewWindow;
use windows::core::s;
use windows::Win32::Foundation::{BOOL, HWND};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW,
    DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_SYSTEMBACKDROP_TYPE,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    DWMWCP_ROUND, DWM_SYSTEMBACKDROP_TYPE, DWM_WINDOW_CORNER_PREFERENCE,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

use super::material::WindowMaterial;

/// DWMBlurGlass AccentBlur uses nFlags ≈ 3584 for full-client blur regions.
const ACCENT_FLAGS_BLUR_FULL: u32 = 3584;

#[repr(C)]
struct AccentPolicy {
    accent_state: u32,
    accent_flags: u32,
    gradient_color: u32,
    animation_id: u32,
}

#[repr(C)]
struct WindowCompositionAttribData {
    attrib: u32,
    pv_data: *mut c_void,
    cb_data: usize,
}

const WCA_ACCENT_POLICY: u32 = 19;
const ACCENT_DISABLED: u32 = 0;
const ACCENT_ENABLE_BLURBEHIND: u32 = 3;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

type SetWindowCompositionAttributeFn =
    unsafe extern "system" fn(HWND, *mut WindowCompositionAttribData) -> BOOL;

fn hwnd_of(window: &WebviewWindow) -> Result<HWND, String> {
    let raw = window
        .hwnd()
        .map_err(|e| format!("material hwnd: {e}"))?
        .0 as isize;
    Ok(HWND(raw as *mut c_void))
}

fn set_window_composition_attribute(
    hwnd: HWND,
    accent_state: u32,
    accent_flags: u32,
    color_abgr: u32,
) -> bool {
    unsafe {
        let Ok(module) = LoadLibraryA(s!("user32.dll")) else {
            return false;
        };
        let Some(proc) = GetProcAddress(module, s!("SetWindowCompositionAttribute")) else {
            return false;
        };
        let set_attr: SetWindowCompositionAttributeFn = std::mem::transmute(proc);
        let mut policy = AccentPolicy {
            accent_state,
            accent_flags,
            gradient_color: color_abgr,
            animation_id: 0,
        };
        let mut data = WindowCompositionAttribData {
            attrib: WCA_ACCENT_POLICY,
            pv_data: &mut policy as *mut _ as *mut c_void,
            cb_data: std::mem::size_of::<AccentPolicy>(),
        };
        set_attr(hwnd, &mut data).as_bool()
    }
}

fn set_system_backdrop(hwnd: HWND, kind: DWM_SYSTEMBACKDROP_TYPE) {
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &kind as *const DWM_SYSTEMBACKDROP_TYPE as *const c_void,
            std::mem::size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
        );
    }
}

fn disable_system_backdrop(hwnd: HWND) {
    set_system_backdrop(hwnd, DWMSBT_NONE);
}

/// Pack `(r,g,b,a)` → SWCA GradientColor (`R | G<<8 | B<<16 | A<<24`).
fn pack_gradient(r: u8, g: u8, b: u8, a: u8) -> u32 {
    u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16) | (u32::from(a) << 24)
}

fn clear_vibrancy(window: &WebviewWindow) {
    let _ = window_vibrancy::clear_mica(window);
    let _ = window_vibrancy::clear_tabbed(window);
    let _ = window_vibrancy::clear_acrylic(window);
    let _ = window_vibrancy::clear_blur(window);
    if let Ok(hwnd) = hwnd_of(window) {
        let _ = set_window_composition_attribute(hwnd, ACCENT_DISABLED, 0, 0);
        disable_system_backdrop(hwnd);
    }
}

pub fn clear(window: &WebviewWindow) -> Result<(), String> {
    clear_vibrancy(window);
    Ok(())
}

/// `WS_EX_TOOLWINDOW` blocks SYSTEMBACKDROP on many Win11 builds — drop it and
/// use ITaskbarList::DeleteTab so popups stay off the taskbar.
fn prepare_hwnd_for_system_backdrop(hwnd: HWND) {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongW, SetWindowLongW, SetWindowPos, GWL_EXSTYLE, SWP_FRAMECHANGED,
            SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_EX_TOOLWINDOW,
        };
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TOOLWINDOW.0 != 0 {
            let new_ex = ex & !WS_EX_TOOLWINDOW.0;
            SetWindowLongW(hwnd, GWL_EXSTYLE, new_ex as i32);
            let _ = SetWindowPos(
                hwnd,
                HWND::default(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }

    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{ITaskbarList, TaskbarList};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if let Ok(taskbar) =
            CoCreateInstance::<_, ITaskbarList>(&TaskbarList, None, CLSCTX_INPROC_SERVER)
        {
            let _ = taskbar.HrInit();
            let _ = taskbar.DeleteTab(hwnd);
        }
    }
}

fn apply_mica_chrome(hwnd: HWND, dark: Option<bool>) {
    unsafe {
        if let Some(d) = dark {
            let v: u32 = u32::from(d);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &v as *const u32 as *const c_void,
                std::mem::size_of::<u32>() as u32,
            );
        }
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const DWM_WINDOW_CORNER_PREFERENCE as *const c_void,
            std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
        let border = DWMWA_COLOR_NONE;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &border as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
    }
}

/// Ensure WebView2 / window clear pixels so SYSTEMBACKDROP (or CSS blur) can show through.
fn clear_webview_fill(window: &WebviewWindow) {
    use tauri::utils::config::Color;
    // Alpha must be 0 — any non-zero A becomes 255 on Win8+ WebView2.
    let _ = window.set_background_color(Some(Color(0, 0, 0, 0)));
}

/// Thin always-on-top / popup glass: SYSTEMBACKDROP often paints a dead
/// charcoal slab under WebView2. SWCA acrylic samples the desktop reliably.
fn apply_swca_acrylic(hwnd: HWND, dark: Option<bool>) -> Result<(), String> {
    disable_system_backdrop(hwnd);
    let tint = if dark == Some(false) {
        pack_gradient(245, 245, 250, 120)
    } else {
        // Keep tint lighter than CSS wash so wallpaper still bleeds through.
        pack_gradient(28, 28, 30, 110)
    };
    if !set_window_composition_attribute(
        hwnd,
        ACCENT_ENABLE_ACRYLICBLURBEHIND,
        ACCENT_FLAGS_BLUR_FULL,
        tint,
    ) {
        if !set_window_composition_attribute(
            hwnd,
            ACCENT_ENABLE_BLURBEHIND,
            ACCENT_FLAGS_BLUR_FULL,
            tint,
        ) {
            return Err("apply SWCA acrylic failed".into());
        }
    }
    Ok(())
}

/// Frosted system backdrop — Acrylic blurs desktop more like Start/flyouts;
/// Mica alone often reads as a flat slab under WebView2.
pub fn apply_system_mica(window: &WebviewWindow, dark: Option<bool>) -> Result<(), String> {
    let hwnd = hwnd_of(window)?;
    prepare_hwnd_for_system_backdrop(hwnd);
    clear_webview_fill(window);

    // Unified SWCA acrylic for dock + popups. Mixing SYSTEMBACKDROP on one HWND
    // with SWCA on another can wipe blur when a sibling window opens.
    apply_swca_acrylic(hwnd, dark)?;
    apply_mica_chrome(hwnd, dark);
    clear_webview_fill(window);
    Ok(())
}

/// Dock icons layer: no SWCA — a sibling `dock-glass` window owns the material
/// on the 60px strip so magnification headroom stays fully clear.
pub fn apply_dock_icons_layer(window: &WebviewWindow, dark: Option<bool>) -> Result<(), String> {
    clear_vibrancy(window);
    let hwnd = hwnd_of(window)?;
    disable_system_backdrop(hwnd);
    unsafe {
        if let Some(d) = dark {
            let v: u32 = u32::from(d);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &v as *const u32 as *const c_void,
                std::mem::size_of::<u32>() as u32,
            );
        }
        let corner = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const DWM_WINDOW_CORNER_PREFERENCE as *const c_void,
            std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
        let border = DWMWA_COLOR_NONE;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &border as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
    }
    clear_webview_fill(window);
    Ok(())
}

/// Apply one README-aligned effect to the window.
pub fn apply_effect(
    window: &WebviewWindow,
    kind: WindowMaterial,
    dark: Option<bool>,
    alpha: u8,
) -> Result<(), String> {
    // Icons layer must stay fully clear above the bar; glass is `dock-glass`.
    if window.label() == "dock" {
        return apply_dock_icons_layer(window, dark);
    }

    let hwnd = hwnd_of(window)?;
    let a = alpha.max(1);

    match kind {
        WindowMaterial::Blur => {
            clear_vibrancy(window);
            disable_system_backdrop(hwnd);
            let color = pack_gradient(18, 18, 20, a.min(120));
            if !set_window_composition_attribute(
                hwnd,
                ACCENT_ENABLE_BLURBEHIND,
                ACCENT_FLAGS_BLUR_FULL,
                color,
            ) {
                return Err("apply Blur (SWCA blurbehind) failed".into());
            }
        }
        WindowMaterial::Aero => {
            clear_vibrancy(window);
            disable_system_backdrop(hwnd);
            let color = pack_gradient(200, 210, 230, a.min(140).max(40));
            if !set_window_composition_attribute(
                hwnd,
                ACCENT_ENABLE_ACRYLICBLURBEHIND,
                ACCENT_FLAGS_BLUR_FULL,
                color,
            ) {
                if !set_window_composition_attribute(
                    hwnd,
                    ACCENT_ENABLE_BLURBEHIND,
                    ACCENT_FLAGS_BLUR_FULL,
                    color,
                ) {
                    return Err("apply Aero failed".into());
                }
            }
        }
        WindowMaterial::Acrylic => {
            clear_vibrancy(window);
            set_system_backdrop(hwnd, DWMSBT_TRANSIENTWINDOW);
            let applied = window_vibrancy::apply_acrylic(window, None).is_ok();
            if !applied {
                disable_system_backdrop(hwnd);
                let color = pack_gradient(18, 18, 20, a);
                if !set_window_composition_attribute(
                    hwnd,
                    ACCENT_ENABLE_ACRYLICBLURBEHIND,
                    ACCENT_FLAGS_BLUR_FULL,
                    color,
                ) {
                    return Err("apply Acrylic failed".into());
                }
            }
        }
        // Prefs still say "mica-alt" for storage compat; visuals are system glass.
        // Do NOT clear_vibrancy first — deferred/open retries would flash “blur deleted”.
        WindowMaterial::MicaAlt => {
            apply_system_mica(window, dark)?;
        }
    }
    Ok(())
}
