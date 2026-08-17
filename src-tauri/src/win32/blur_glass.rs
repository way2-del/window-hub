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
    DwmSetWindowAttribute, DWMSBT_MAINWINDOW, DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW,
    DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_COLOR_NONE, DWMWA_SYSTEMBACKDROP_TYPE,
    DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_VISIBLE_FRAME_BORDER_THICKNESS,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND, DWMWCP_ROUNDSMALL,
    DWM_SYSTEMBACKDROP_TYPE, DWM_WINDOW_CORNER_PREFERENCE,
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
        // Maximized + ROUND often forces an opaque black caption; match Win11 apps.
        let corner = if windows::Win32::UI::WindowsAndMessaging::IsZoomed(hwnd).as_bool() {
            DWMWCP_DONOTROUND
        } else {
            DWMWCP_ROUND
        };
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

/// Caption color for framed Mica windows — always `COLOR_NONE` so SYSTEMBACKDROP
/// owns the title strip. (Solid wallpaper tints look “吸错色”; plugin-window uses
/// a frameless Host chrome so mica shows in the client caption row.)
fn framed_caption_color(_hwnd: HWND, _is_dark: bool) -> u32 {
    DWMWA_COLOR_NONE
}

fn dwm_frame_changed(hwnd: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    };
    unsafe {
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

/// Kept for reference; `plugin-window` now uses settings-frame Mica like settings.
#[allow(dead_code)]
pub fn apply_plugin_window_glass(window: &WebviewWindow, dark: Option<bool>) -> Result<(), String> {
    let hwnd = hwnd_of(window)?;
    prepare_hwnd_for_system_backdrop(hwnd);
    clear_webview_fill(window);
    disable_system_backdrop(hwnd);
    let tint = if dark == Some(false) {
        pack_gradient(248, 248, 250, 72)
    } else {
        // Keep alpha low — wallpaper must remain visible in the chrome strip.
        pack_gradient(32, 34, 38, 58)
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
            return Err("apply plugin-window glass failed".into());
        }
    }
    apply_mica_chrome(hwnd, dark);
    clear_webview_fill(window);
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

/// Native OS plugin window: dedicated label `plugin-window`, or legacy
/// `plugin-popup` with `nativeFrame=1` / decorated frame.
pub fn is_native_frame_plugin_popup(window: &WebviewWindow) -> bool {
    match window.label() {
        "plugin-window" => true,
        "plugin-popup" => {
            if window
                .url()
                .ok()
                .map(|u| {
                    let s = u.as_str();
                    s.contains("nativeFrame=1") || s.contains("nativeFrame%3D1")
                })
                .unwrap_or(false)
            {
                return true;
            }
            window.is_decorated().unwrap_or(false)
        }
        _ => false,
    }
}

/// Settings window (decorated): true system Mica so **title bar + client** share
/// the same theme backdrop. Right pane stays solid via CSS (`--glass-main-bg`).
///
/// Frameless popups keep SWCA acrylic (`apply_system_mica`). Decorated frames
/// (`settings` / `dock-icon-editor` / `plugin-window`) use
/// `DWMSBT_MAINWINDOW` + caption `COLOR_NONE` (windowed) or wallpaper-tinted
/// caption when maximized (Win11 drops live Mica on zoomed frames).
pub fn apply_settings_frame_mica(window: &WebviewWindow, dark: Option<bool>) -> Result<(), String> {
    let hwnd = hwnd_of(window)?;
    let is_dark = dark.unwrap_or(true);

    // Dark + backdrop FIRST — clearing SWCA before this paints a white frame flash.
    apply_mica_chrome(hwnd, Some(is_dark));
    set_system_backdrop(hwnd, DWMSBT_MAINWINDOW);

    // Drop SWCA so SYSTEMBACKDROP owns the full frame (caption included).
    let _ = set_window_composition_attribute(hwnd, ACCENT_DISABLED, 0, 0);
    let _ = window_vibrancy::clear_acrylic(window);
    let _ = window_vibrancy::clear_blur(window);
    clear_webview_fill(window);

    // Re-assert after clear (some builds drop immersive mode when SWCA is disabled).
    apply_mica_chrome(hwnd, Some(is_dark));
    set_system_backdrop(hwnd, DWMSBT_MAINWINDOW);

    unsafe {
        // Windowed: COLOR_NONE → live Mica caption. Maximized: wallpaper-tinted solid.
        let caption = framed_caption_color(hwnd, is_dark);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR,
            &caption as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
        // COLORREF 0x00BBGGRR
        let text: u32 = if is_dark {
            0x00_F5_F4_F4
        } else {
            0x00_1E_1C_1C
        };
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TEXT_COLOR,
            &text as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
        let thickness: u32 = 0;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_VISIBLE_FRAME_BORDER_THICKNESS,
            &thickness as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
    }

    dwm_frame_changed(hwnd);
    clear_webview_fill(window);
    Ok(())
}

/// Re-hit dark + system Mica without clear/SWCA teardown (no white flash).
pub fn reassert_settings_frame_mica(window: &WebviewWindow, dark: Option<bool>) -> Result<(), String> {
    let hwnd = hwnd_of(window)?;
    let is_dark = dark.unwrap_or(true);
    apply_mica_chrome(hwnd, Some(is_dark));
    set_system_backdrop(hwnd, DWMSBT_MAINWINDOW);
    unsafe {
        let caption = framed_caption_color(hwnd, is_dark);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR,
            &caption as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
        let text: u32 = if is_dark {
            0x00_F5_F4_F4
        } else {
            0x00_1E_1C_1C
        };
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TEXT_COLOR,
            &text as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
    }
    dwm_frame_changed(hwnd);
    clear_webview_fill(window);
    Ok(())
}

/// Dock glass strip.
///
/// - Radius 0–8: SWCA acrylic + system DWM corners.
/// - Radius ≥ 9: Windows.UI.Composition HostBackdrop under WebView2 with
///   RectangleClip radii (pixel-true capsule; same system backdrop sampling).
pub fn apply_dock_glass_layer(window: &WebviewWindow, dark: Option<bool>) -> Result<(), String> {
    let hwnd = hwnd_of(window)?;
    prepare_hwnd_for_system_backdrop(hwnd);
    clear_webview_fill(window);
    crate::win32::dock_comp::remember_glass_window(window);

    let radius = dock_corner_radius_px();
    apply_dock_glass_frost(window, hwnd, dark, radius)?;
    let _ = window.set_shadow(false);
    strip_class_drop_shadow(hwnd);

    let win = window.clone();
    let dark_c = dark;
    // Two deferred refreshes is enough for WebView2 reparent; the old 5-hit
    // loop stacked with hover resize and could stall the UI thread.
    std::thread::spawn(move || {
        for ms in [60_u64, 220] {
            std::thread::sleep(std::time::Duration::from_millis(ms));
            let w = win.clone();
            let dark_inner = dark_c;
            let w2 = w.clone();
            let _ = w.run_on_main_thread(move || {
                if let Ok(h) = hwnd_of(&w2) {
                    let r = dock_corner_radius_px();
                    let _ = apply_dock_glass_frost(&w2, h, dark_inner, r);
                    let _ = w2.set_shadow(false);
                    strip_class_drop_shadow(h);
                    clear_webview_fill(&w2);
                }
            });
        }
    });
    clear_webview_fill(window);
    Ok(())
}

const DOCK_NC_SUBCLASS_ID: usize = 0xD0C4_0001;
/// DWMWA_NCRENDERING_POLICY / DWMNCRP_DISABLED (not always in windows crate).
use windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE;
const DWMWA_NCRENDERING_POLICY: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(2);
const DWMNCRP_DISABLED: u32 = 1;

type SubclassProc = unsafe extern "system" fn(
    HWND,
    u32,
    windows::Win32::Foundation::WPARAM,
    windows::Win32::Foundation::LPARAM,
    usize,
    usize,
) -> windows::Win32::Foundation::LRESULT;
type SetWindowSubclassFn = unsafe extern "system" fn(HWND, SubclassProc, usize, usize) -> BOOL;
type RemoveWindowSubclassFn = unsafe extern "system" fn(HWND, SubclassProc, usize) -> BOOL;
type DefSubclassProcFn = unsafe extern "system" fn(
    HWND,
    u32,
    windows::Win32::Foundation::WPARAM,
    windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT;

struct ComctlSubclass {
    set: SetWindowSubclassFn,
    remove: RemoveWindowSubclassFn,
    def: DefSubclassProcFn,
}

fn comctl_subclass() -> Option<&'static ComctlSubclass> {
    static CELL: std::sync::OnceLock<Option<ComctlSubclass>> = std::sync::OnceLock::new();
    CELL.get_or_init(|| unsafe {
        let module = LoadLibraryA(s!("comctl32.dll")).ok()?;
        let set = GetProcAddress(module, s!("SetWindowSubclass"))?;
        let remove = GetProcAddress(module, s!("RemoveWindowSubclass"))?;
        let def = GetProcAddress(module, s!("DefSubclassProc"))?;
        Some(ComctlSubclass {
            set: std::mem::transmute(set),
            remove: std::mem::transmute(remove),
            def: std::mem::transmute(def),
        })
    })
    .as_ref()
}

unsafe extern "system" fn dock_nc_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
    _id: usize,
    _data: usize,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, WM_MOUSEACTIVATE, WM_NCACTIVATE, WM_NCCALCSIZE, WM_NCHITTEST, WM_NCPAINT,
        HTCLIENT, HTTRANSPARENT, MA_NOACTIVATE,
    };
    if msg == WM_NCCALCSIZE && wparam.0 != 0 {
        return windows::Win32::Foundation::LRESULT(0);
    }
    if msg == WM_NCPAINT {
        return windows::Win32::Foundation::LRESULT(0);
    }
    // Activation must not paint a caption strip into headroom (right-click menu).
    if msg == WM_NCACTIVATE {
        // Quietly re-assert DWM attrs without FRAMECHANGED flash.
        quiet_reassert_dock_dwm(hwnd);
        // Only the icons layer is taller than chrome — never SetWindowRgn on glass.
        let mut rc = RECT::default();
        if GetClientRect(hwnd, &mut rc).is_ok() {
            let h = (rc.bottom - rc.top).max(1);
            let dpi = GetDpiForWindow(hwnd);
            let scale = if dpi > 0 {
                dpi as f64 / 96.0
            } else {
                1.0
            };
            let glass_h = (crate::dock::DOCK_H * scale).round().max(1.0) as i32;
            if h > glass_h + 2 {
                crate::dock::reclip_dock_icons_hwnd(hwnd.0 as isize);
            }
        }
        return windows::Win32::Foundation::LRESULT(1);
    }
    if msg == WM_NCHITTEST {
        let mut pt = POINT {
            x: (lparam.0 as u32 & 0xFFFF) as i16 as i32,
            y: ((lparam.0 as u32 >> 16) & 0xFFFF) as i16 as i32,
        };
        if ScreenToClient(hwnd, &mut pt).as_bool() {
            let mut rc = RECT::default();
            if GetClientRect(hwnd, &mut rc).is_ok() {
                let h = (rc.bottom - rc.top).max(1);
                let dpi = GetDpiForWindow(hwnd);
                let scale = if dpi > 0 {
                    dpi as f64 / 96.0
                } else {
                    1.0
                };
                let glass_h = (crate::dock::DOCK_H * scale).round().max(1.0) as i32;
                let chrome_top = (h - glass_h).max(0);
                // Same caption-band inset as icons region — never hit-test there.
                let headroom_top = crate::dock::dock_caption_band_px(scale, chrome_top);
                if pt.y < headroom_top {
                    return windows::Win32::Foundation::LRESULT(HTTRANSPARENT as isize);
                }
                let fan_open = crate::dock::dock_hover_expanded()
                    || crate::win32::dock_comp::width_tween_active();
                // Rest: empty headroom above chrome passes through.
                // Fan: headroom stays solid so magnified icons remain clickable.
                if !fan_open && pt.y < chrome_top {
                    return windows::Win32::Foundation::LRESULT(HTTRANSPARENT as isize);
                }
            }
        }
        return windows::Win32::Foundation::LRESULT(HTCLIENT as isize);
    }
    if msg == WM_MOUSEACTIVATE {
        return windows::Win32::Foundation::LRESULT(MA_NOACTIVATE as isize);
    }
    if let Some(api) = comctl_subclass() {
        return (api.def)(hwnd, msg, wparam, lparam);
    }
    windows::Win32::Foundation::LRESULT(0)
}

fn dock_root_for_strip(hwnd: HWND) -> HWND {
    use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GA_ROOT};
    unsafe {
        let root = GetAncestor(hwnd, GA_ROOT);
        if root.0.is_null() {
            hwnd
        } else {
            root
        }
    }
}

/// Re-apply DWM caption kill without FRAMECHANGED / redraw (no flash).
fn quiet_reassert_dock_dwm(hwnd: HWND) {
    unsafe {
        let policy = DWMNCRP_DISABLED;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            &policy as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
        let none = DWMWA_COLOR_NONE;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR,
            &none as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &none as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
        let thickness: u32 = 0;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_VISIBLE_FRAME_BORDER_THICKNESS,
            &thickness as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
    }
}

/// Drop caption chrome + keep a comctl subclass first in the chain so Win11
/// cannot paint a light title-bar strip into dock headroom.
pub fn strip_dock_native_titlebar(hwnd: HWND) {
    strip_dock_native_titlebar_inner(hwnd, true);
}

/// Quiet path for focus / context-menu: only FRAMECHANGED when styles actually
/// change. Blind redraw was flashing a light “window form” above the glass.
pub fn ensure_dock_titlebar_stripped(hwnd: HWND) {
    strip_dock_native_titlebar_inner(hwnd, false);
}

fn strip_dock_native_titlebar_inner(hwnd: HWND, force_frame: bool) {
    use windows::Win32::Graphics::Gdi::{RedrawWindow, RDW_FRAME, RDW_INVALIDATE, RDW_UPDATENOW};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, SetWindowLongW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_BORDER, WS_CAPTION,
        WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_MAXIMIZEBOX, WS_MINIMIZEBOX,
        WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
    };
    unsafe {
        let hwnd = dock_root_for_strip(hwnd);
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let kill = WS_CAPTION.0
            | WS_THICKFRAME.0
            | WS_SYSMENU.0
            | WS_MINIMIZEBOX.0
            | WS_MAXIMIZEBOX.0
            | WS_BORDER.0;
        // Force popup frame — overlapped styles reintroduce a Win11 caption band.
        let new_style = (style & !kill) | WS_POPUP.0;
        let style_changed = new_style != style;
        if style_changed {
            SetWindowLongW(hwnd, GWL_STYLE, new_style as i32);
        }
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        let new_ex = (ex | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) & !WS_EX_APPWINDOW.0;
        let ex_changed = new_ex != ex;
        if ex_changed {
            SetWindowLongW(hwnd, GWL_EXSTYLE, new_ex as i32);
        }
        // Remove+re-add so we stay the *newest* subclass after WebView2/Tao hooks.
        if let Some(api) = comctl_subclass() {
            let _ = (api.remove)(hwnd, dock_nc_subclass_proc, DOCK_NC_SUBCLASS_ID);
            let _ = (api.set)(hwnd, dock_nc_subclass_proc, DOCK_NC_SUBCLASS_ID, 0);
        }
        let policy = DWMNCRP_DISABLED;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            &policy as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
        let none = DWMWA_COLOR_NONE;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR,
            &none as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TEXT_COLOR,
            &none as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &none as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
        let thickness: u32 = 0;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_VISIBLE_FRAME_BORDER_THICKNESS,
            &thickness as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
        // FRAMECHANGED + sync redraw is what flashes the light strip on right-click.
        if force_frame || style_changed || ex_changed {
            let _ = SetWindowPos(
                hwnd,
                HWND::default(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
            let _ = RedrawWindow(
                hwnd,
                None,
                None,
                RDW_FRAME | RDW_INVALIDATE | RDW_UPDATENOW,
            );
        }
    }
}

/// Tauri may depend on a newer `windows` crate — accept raw HWND bits.
pub fn strip_dock_native_titlebar_raw(hwnd_raw: isize) {
    strip_dock_native_titlebar(HWND(hwnd_raw as _));
}

pub fn ensure_dock_titlebar_stripped_raw(hwnd_raw: isize) {
    ensure_dock_titlebar_stripped(HWND(hwnd_raw as _));
}

/// WebView2 often re-subclasses after first show — re-strip on a short schedule
/// so the light caption bar never sticks until the user clicks a few times.
pub fn schedule_dock_titlebar_strip(hwnd_raw: isize) {
    strip_dock_native_titlebar_raw(hwnd_raw);
    std::thread::spawn(move || {
        use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
        for ms in [16_u64, 50, 120, 300, 700, 1500] {
            std::thread::sleep(std::time::Duration::from_millis(ms));
            let hwnd = dock_root_for_strip(HWND(hwnd_raw as _));
            // Don't poke a tucked AutoHide dock — FRAMECHANGED can flash a strip.
            unsafe {
                if !IsWindowVisible(hwnd).as_bool() {
                    continue;
                }
            }
            // Delayed passes stay quiet unless styles drifted back.
            ensure_dock_titlebar_stripped_raw(hwnd_raw);
        }
    });
}

/// Strip dock + dock-glass (and clear titles). Call when menus open / focus shifts.
pub fn strip_dock_windows(app: &tauri::AppHandle) {
    use tauri::Manager;
    for label in ["dock", "dock-glass"] {
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.set_decorations(false);
            let _ = w.set_title("");
            if let Ok(hwnd) = w.hwnd() {
                // Quiet: right-click menu must not flash caption into headroom.
                ensure_dock_titlebar_stripped_raw(hwnd.0 as isize);
                // Icons: chrome-only region so residual light shell is clipped.
                if label == "dock" {
                    crate::dock::reclip_dock_icons_hwnd(hwnd.0 as isize);
                }
            }
        }
    }
}

/// Mild chrome for frameless menus/popups. Do **not** reuse the dock NC nuke
/// (`DWMNCRP_DISABLED` / forced `WS_POPUP`) — that leaves a light window frame
/// beside the dark menu shell.
pub fn strip_frameless_popup_titlebar(hwnd_raw: isize) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, SetWindowLongW, SetWindowPos, GWL_STYLE, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_CAPTION, WS_MAXIMIZEBOX,
        WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
    };
    let hwnd = dock_root_for_strip(HWND(hwnd_raw as _));
    unsafe {
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let kill =
            WS_CAPTION.0 | WS_THICKFRAME.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0;
        let new_style = style & !kill;
        if new_style != style {
            SetWindowLongW(hwnd, GWL_STYLE, new_style as i32);
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
    apply_mica_chrome(hwnd, None);
    let thickness: u32 = 0;
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_VISIBLE_FRAME_BORDER_THICKNESS,
            &thickness as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        )
    };
}

fn apply_dock_glass_chrome(hwnd: HWND, dark: Option<bool>, corner_radius_logical: u32) {
    strip_dock_native_titlebar(hwnd);
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
        let corner = dwm_corner_for_radius(corner_radius_logical);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const DWM_WINDOW_CORNER_PREFERENCE as *const c_void,
            std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
    }
}

fn dwm_corner_for_radius(radius_logical: u32) -> DWM_WINDOW_CORNER_PREFERENCE {
    match radius_logical {
        0 => DWMWCP_DONOTROUND,
        1..=4 => DWMWCP_ROUNDSMALL,
        5..=8 => DWMWCP_ROUND,
        // Composition owns silhouette past system ROUND.
        _ => DWMWCP_DONOTROUND,
    }
}

fn apply_dock_glass_frost(
    window: &WebviewWindow,
    hwnd: HWND,
    dark: Option<bool>,
    corner_radius_logical: u32,
) -> Result<(), String> {
    let r = corner_radius_logical.min(crate::win32::dock_comp::DOCK_CORNER_RADIUS_MAX);
    apply_dock_glass_chrome(hwnd, dark, r);
    // Composition owns the smooth capsule — a GDI SetWindowRgn here makes
    // widened corners look broken/jagged (rest looks fine because frost is inset).
    clear_window_region(hwnd);
    disable_blur_behind(hwnd);

    if crate::win32::dock_comp::uses_composition(r) {
        // Drop SWCA acrylic slab only — HostBackdrop accent is primed inside dock_comp.
        // Do NOT leave ACCENT_DISABLED on: that turns HostBackdrop into a black fill.
        disable_system_backdrop(hwnd);
        clear_webview_fill(window);
        let win = window.clone();
        let dark_c = dark;
        let win2 = win.clone();
        let _ = win.run_on_main_thread(move || {
            if let Ok(h) = hwnd_of(&win2) {
                if let Err(e) = crate::win32::dock_comp::attach_or_update(h, r, dark_c) {
                    eprintln!("[dock-comp] attach failed: {e}");
                    let _ = apply_swca_acrylic(h, dark_c);
                    apply_dock_glass_chrome(h, dark_c, 8);
                }
            }
        });
        return Ok(());
    }

    crate::win32::dock_comp::detach();
    apply_swca_acrylic(hwnd, dark)?;
    Ok(())
}

fn strip_class_drop_shadow(hwnd: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassLongPtrW, SetClassLongPtrW, CS_DROPSHADOW, GCL_STYLE,
    };
    unsafe {
        let style = GetClassLongPtrW(hwnd, GCL_STYLE);
        let drop = CS_DROPSHADOW.0 as usize;
        if style & drop != 0 {
            let _ = SetClassLongPtrW(hwnd, GCL_STYLE, (style & !drop) as isize);
        }
    }
}

fn clear_window_region(hwnd: HWND) {
    use windows::Win32::Graphics::Gdi::SetWindowRgn;
    unsafe {
        let _ = SetWindowRgn(hwnd, None, true);
    }
}

fn disable_blur_behind(hwnd: HWND) {
    use windows::Win32::Graphics::Dwm::{
        DwmEnableBlurBehindWindow, DWM_BB_ENABLE, DWM_BLURBEHIND,
    };
    use windows::Win32::Graphics::Gdi::HRGN;
    let bb = DWM_BLURBEHIND {
        dwFlags: DWM_BB_ENABLE,
        fEnable: false.into(),
        hRgnBlur: HRGN::default(),
        fTransitionOnMaximized: false.into(),
    };
    unsafe {
        let _ = DwmEnableBlurBehindWindow(hwnd, &bb);
    }
}

fn dock_corner_radius_px() -> u32 {
    crate::db::with_conn(|c| crate::db::dock_get(c))
        .ok()
        .flatten()
        .and_then(|v| {
            v.get("cornerRadiusPx")
                .and_then(|x| x.as_u64())
                .or_else(|| v.get("corner_radius_px").and_then(|x| x.as_u64()))
        })
        .map(|n| n.min(crate::win32::dock_comp::DOCK_CORNER_RADIUS_MAX as u64) as u32)
        .unwrap_or(20)
}

/// Place/resize hook — refreshes Composition clip or DWM corners.
pub fn apply_dock_glass_round_frost_sized_pub(
    hwnd: HWND,
    corner_radius_logical: u32,
    size_px: Option<(f32, f32)>,
) {
    let r = corner_radius_logical.min(crate::win32::dock_comp::DOCK_CORNER_RADIUS_MAX);
    apply_dock_glass_chrome(hwnd, None, r);
    // Same as frost attach: leave region clear so Composition silhouette stays smooth when wide.
    clear_window_region(hwnd);
    disable_blur_behind(hwnd);
    if crate::win32::dock_comp::uses_composition(r) {
        // Layout-only refresh: never ACCENT_DISABLED (that blacks out HostBackdrop).
        disable_system_backdrop(hwnd);
        let _ = crate::win32::dock_comp::sync_attach_or_update_sized(hwnd, size_px, r, None);
    } else {
        crate::win32::dock_comp::detach();
    }
}

/// Dock icons layer: no SWCA — a sibling `dock-glass` window owns the material
/// on the chrome strip so magnification headroom stays fully clear.
pub fn apply_dock_icons_layer(window: &WebviewWindow, dark: Option<bool>) -> Result<(), String> {
    clear_vibrancy(window);
    let hwnd = hwnd_of(window)?;
    disable_system_backdrop(hwnd);
    let _ = window.set_decorations(false);
    strip_dock_native_titlebar(hwnd);
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
    }
    let _ = window.set_shadow(false);
    clear_webview_fill(window);
    // Clip to chrome capsule so headroom cannot host a light caption band.
    crate::dock::reclip_dock_icons_hwnd(hwnd.0 as isize);
    Ok(())
}

/// Apply one README-aligned effect to the window.
pub fn apply_effect(
    window: &WebviewWindow,
    kind: WindowMaterial,
    dark: Option<bool>,
    alpha: u8,
) -> Result<(), String> {
    match window.label() {
        // Icons layer must stay fully clear above the bar.
        "dock" => return apply_dock_icons_layer(window, dark),
        // Glass strip: acrylic without rounded DWM chrome/shadow.
        "dock-glass" => return apply_dock_glass_layer(window, dark),
        // Decorated settings / icon editor / plugin OS window: Mica on caption.
        "settings" | "dock-icon-editor" | "plugin-window" => {
            if matches!(kind, WindowMaterial::MicaAlt) {
                return apply_settings_frame_mica(window, dark);
            }
        }
        "plugin-popup" => {
            // Legacy nativeFrame on same label only (decorated OS caption).
            if matches!(kind, WindowMaterial::MicaAlt) && is_native_frame_plugin_popup(window) {
                return apply_settings_frame_mica(window, dark);
            }
        }
        _ => {}
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
