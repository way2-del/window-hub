//! DWMBlurGlass-inspired materials for a single HWND (no dwm.exe inject).
//!
//! Maps README effects to process-local APIs:
//! - **Blur** → `ACCENT_ENABLE_BLURBEHIND`
//! - **Aero** → `ACCENT_ENABLE_ACRYLICBLURBEHIND` with light tint
//! - **Acrylic** → Win11 `DWMSBT_TRANSIENTWINDOW`, else SWCA acrylic
//! - **Mica** (prefs id `mica-alt` for compat) → SWCA, with **opaque solid
//!   fallback** on Win10 / slim builds where acrylic hangs DWM.

#![cfg(windows)]

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;
use tauri::WebviewWindow;
use windows::core::s;
use windows::Win32::Foundation::{BOOL, HWND};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW, DWMWA_BORDER_COLOR,
    DWMWA_CAPTION_COLOR, DWMWA_COLOR_NONE, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_TEXT_COLOR,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    DWMWCP_ROUND, DWM_SYSTEMBACKDROP_TYPE, DWM_WINDOW_CORNER_PREFERENCE,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

use super::material::WindowMaterial;

/// Sticky compat mode: opaque solid popups (no SWCA acrylic).
/// Auto-on for Win10 (build < 22000) and after any SWCA failure — slim Win10
/// acrylic often freezes DWM into a white/dead shell.
static HARD_SAFE: AtomicBool = AtomicBool::new(false);
static HARD_SAFE_INIT: Once = Once::new();

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

fn os_build() -> u32 {
    #[repr(C)]
    struct OsVersionInfo {
        dw_os_version_info_size: u32,
        dw_major_version: u32,
        dw_minor_version: u32,
        dw_build_number: u32,
        dw_platform_id: u32,
        sz_csd_version: [u16; 128],
    }
    type RtlGetVersionFn = unsafe extern "system" fn(*mut OsVersionInfo) -> i32;
    unsafe {
        let Ok(lib) = LoadLibraryA(s!("ntdll.dll")) else {
            return 0;
        };
        let Some(proc) = GetProcAddress(lib, s!("RtlGetVersion")) else {
            return 0;
        };
        let rtl: RtlGetVersionFn = std::mem::transmute(proc);
        let mut info = OsVersionInfo {
            dw_os_version_info_size: std::mem::size_of::<OsVersionInfo>() as u32,
            dw_major_version: 0,
            dw_minor_version: 0,
            dw_build_number: 0,
            dw_platform_id: 0,
            sz_csd_version: [0; 128],
        };
        if rtl(&mut info) != 0 {
            return 0;
        }
        info.dw_build_number
    }
}

fn ensure_hard_safe_detected() {
    HARD_SAFE_INIT.call_once(|| {
        let build = os_build();
        // Win11 starts at 22000. Win10 (including 精简版) → opaque by default.
        // build==0: RtlGetVersion failed (broken ntdll on some slim packs) → safe.
        if build == 0 || (build > 0 && build < 22000) {
            HARD_SAFE.store(true, Ordering::SeqCst);
            eprintln!(
                "[glass] Win10/unknown build {build} — opaque HWND popups (no transparent WebView2)"
            );
        }
        // User turned off Settings → Personalization → Transparency effects.
        // Transparent WebView2 + acrylic is unusable in that state.
        if !windows_transparency_enabled() {
            HARD_SAFE.store(true, Ordering::SeqCst);
            eprintln!(
                "[glass] EnableTransparency=0 — forcing opaque popups (system transparency off)"
            );
        }
        // Composition off → acrylic/mica paint as white slabs.
        if !dwm_composition_enabled() {
            HARD_SAFE.store(true, Ordering::SeqCst);
            eprintln!("[glass] DWM composition off — forcing opaque popups");
        }
    });
}

fn dwm_composition_enabled() -> bool {
    use windows::Win32::Graphics::Dwm::DwmIsCompositionEnabled;
    unsafe { DwmIsCompositionEnabled().ok().map(|b| b.as_bool()).unwrap_or(true) }
}

fn windows_transparency_enabled() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(key) = hkcu.open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
    else {
        return true;
    };
    match key.get_value::<u32, _>("EnableTransparency") {
        Ok(0) => false,
        Ok(_) => true,
        Err(_) => true,
    }
}

/// Whether popups use solid opaque fill (no transparent WebView fill / acrylic).
pub fn is_hard_safe() -> bool {
    ensure_hard_safe_detected();
    HARD_SAFE.load(Ordering::SeqCst)
}

/// Sync JS injected **before** any page paint — sets `data-glass-compat` + solid
/// CSS so React/`main.tsx` cannot race a transparent body over an opaque HWND
/// (classic white zombie on slim Win10).
pub fn glass_compat_boot_script() -> String {
    if !is_hard_safe() {
        return String::new();
    }
    let dark = super::material::system_apps_dark();
    let (bg, fg, muted, theme, scheme) = if dark {
        (
            "rgb(28, 28, 30)",
            "#f4f4f5",
            "#a1a1aa",
            "dark",
            "dark",
        )
    } else {
        (
            "rgb(245, 245, 247)",
            "#1c1c1e",
            "#3f3f46",
            "light",
            "light",
        )
    };
    format!(
        r#"(function(){{
  try {{
    var r = document.documentElement;
    r.dataset.glassCompat = "1";
    r.dataset.glass = "mica";
    r.dataset.theme = {theme:?};
    r.style.colorScheme = {scheme:?};
    r.style.setProperty("--glass-panel-bg", {bg:?});
    r.style.setProperty("--glass-fg", {fg:?});
    r.style.setProperty("--glass-fg-muted", {muted:?});
    r.style.background = {bg:?};
    window.__WH_GLASS_COMPAT__ = true;
    var s = document.getElementById("wh-glass-compat-boot");
    if (!s) {{
      s = document.createElement("style");
      s.id = "wh-glass-compat-boot";
      (document.head || r).appendChild(s);
    }}
    s.textContent =
      "html,body,#root{{background:" + {bg:?} + " !important;color:" + {fg:?} + " !important;}}" +
      "html[data-glass-compat='1'] body,html[data-glass-compat='1'] body #root," +
      "html[data-glass-compat='1'] .plugin-popup-root,html[data-glass-compat='1'] .system-flyout-shell," +
      "html[data-glass-compat='1'] .tray-popup-shell,html[data-glass-compat='1'] .wg-shell{{" +
      "background:" + {bg:?} + " !important;}}";
  }} catch (e) {{}}
}})();"#,
        theme = theme,
        scheme = scheme,
        bg = bg,
        fg = fg,
        muted = muted,
    )
}

/// Prepend opaque-compat boot JS to a window `initialization_script`.
pub fn prepend_glass_compat_boot(user_js: &str) -> String {
    let boot = glass_compat_boot_script();
    if boot.is_empty() {
        return user_js.to_string();
    }
    if user_js.trim().is_empty() {
        return boot;
    }
    format!("{boot}\n{user_js}")
}

/// Win10 hard-safe: **never** create transparent HWNDs (EnableTransparency=0 +
/// transparent WebView2 = white freeze). Win11 keeps layered glass.
pub fn popup_is_transparent() -> bool {
    !is_hard_safe()
}

/// Background for popup builders — opaque RGB on Win10, clear on Win11 glass.
pub fn popup_background_color() -> tauri::utils::config::Color {
    use tauri::utils::config::Color;
    if is_hard_safe() {
        if super::material::system_apps_dark() {
            Color(28, 28, 30, 255)
        } else {
            Color(245, 245, 247, 255)
        }
    } else {
        Color(0, 0, 0, 0)
    }
}

fn arm_hard_safe(reason: &str) {
    if !HARD_SAFE.swap(true, Ordering::SeqCst) {
        eprintln!("[glass] enabling opaque compat mode: {reason}");
    }
}

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

/// Solid opaque fill — safe on slim Win10 where transparent+acrylic freezes DWM.
pub fn apply_opaque_solid(window: &WebviewWindow, dark: Option<bool>) -> Result<(), String> {
    use tauri::utils::config::Color;
    // Skip clear_vibrancy thrash — on Win10 it can white-flash even for already-opaque HWNDs.
    let hwnd = hwnd_of(window)?;
    let _ = set_window_composition_attribute(hwnd, ACCENT_DISABLED, 0, 0);
    disable_system_backdrop(hwnd);
    let is_dark = dark.unwrap_or(true);
    let fill = if is_dark {
        Color(28, 28, 30, 255)
    } else {
        Color(245, 245, 247, 255)
    };
    let _ = window.set_background_color(Some(fill));
    unsafe {
        let v: u32 = u32::from(is_dark);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &v as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
        let corner = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const DWM_WINDOW_CORNER_PREFERENCE as *const c_void,
            std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
    }
    Ok(())
}

fn prepare_hwnd_for_system_backdrop(hwnd: HWND) {
    // Skip COM thrash in compat mode (rapid open/close was CoInitialize storm).
    if is_hard_safe() {
        return;
    }
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
        let corner = if is_hard_safe() {
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
        // Light theme + ROUND leaves a default white border unless all chrome
        // color attrs are cleared (classic L-shaped frame around dock-glass).
        let border = DWMWA_COLOR_NONE;
        for attr in [DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR] {
            let _ = DwmSetWindowAttribute(
                hwnd,
                attr,
                &border as *const u32 as *const c_void,
                std::mem::size_of::<u32>() as u32,
            );
        }
    }
}

fn clear_webview_fill(window: &WebviewWindow) {
    use tauri::utils::config::Color;
    let _ = window.set_background_color(Some(Color(0, 0, 0, 0)));
}

fn apply_swca_acrylic(hwnd: HWND, dark: Option<bool>) -> Result<(), String> {
    disable_system_backdrop(hwnd);
    let tint = if dark == Some(false) {
        pack_gradient(245, 245, 250, 120)
    } else {
        pack_gradient(28, 28, 30, 110)
    };
    // Prefer blurbehind — acrylic (state 4) is the hang risk on slim Win10.
    if set_window_composition_attribute(
        hwnd,
        ACCENT_ENABLE_BLURBEHIND,
        ACCENT_FLAGS_BLUR_FULL,
        tint,
    ) {
        return Ok(());
    }
    if set_window_composition_attribute(
        hwnd,
        ACCENT_ENABLE_ACRYLICBLURBEHIND,
        ACCENT_FLAGS_BLUR_FULL,
        tint,
    ) {
        return Ok(());
    }
    Err("apply SWCA acrylic failed".into())
}

/// Frosted system backdrop — or opaque solid in hard-safe / Win10 compat mode.
pub fn apply_system_mica(window: &WebviewWindow, dark: Option<bool>) -> Result<(), String> {
    ensure_hard_safe_detected();
    if is_hard_safe() {
        return apply_opaque_solid(window, dark);
    }

    let hwnd = hwnd_of(window)?;
    prepare_hwnd_for_system_backdrop(hwnd);

    match apply_swca_acrylic(hwnd, dark) {
        Ok(()) => {
            // Only clear WebView fill AFTER composition succeeds.
            clear_webview_fill(window);
            apply_mica_chrome(hwnd, dark);
            Ok(())
        }
        Err(e) => {
            arm_hard_safe(&e);
            apply_opaque_solid(window, dark)
        }
    }
}

/// Icons layer must NEVER carry SystemBackdrop / SWCA / DWM border.
/// HWND size matches the CSS pill; any NC/DWM fringe paints the classic white frame.
pub fn apply_dock_icons_layer(window: &WebviewWindow, dark: Option<bool>) -> Result<(), String> {
    clear_vibrancy(window);
    let hwnd = hwnd_of(window)?;
    disable_system_backdrop(hwnd);
    let _ = set_window_composition_attribute(hwnd, ACCENT_DISABLED, 0, 0);
    clear_webview_fill(window);
    let _ = window.set_shadow(false);
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongW, SetWindowLongW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE, SWP_FRAMECHANGED,
            SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_BORDER, WS_CAPTION,
            WS_DLGFRAME, WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_EX_STATICEDGE, WS_EX_WINDOWEDGE,
            WS_SYSMENU, WS_THICKFRAME,
        };
        if let Some(d) = dark {
            let v: u32 = u32::from(d);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &v as *const u32 as *const c_void,
                std::mem::size_of::<u32>() as u32,
            );
        }
        // Never call DwmExtendFrameIntoClientArea(-1) here — on transparent
        // WebView2 it paints the exact white rectangular outline around the dock.
        let style = GetWindowLongW(hwnd, GWL_STYLE);
        let strip = (WS_BORDER.0 | WS_DLGFRAME.0 | WS_THICKFRAME.0 | WS_CAPTION.0 | WS_SYSMENU.0)
            as i32;
        if style & strip != 0 {
            let _ = SetWindowLongW(hwnd, GWL_STYLE, style & !strip);
        }
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE);
        let ex_strip = (WS_EX_WINDOWEDGE.0
            | WS_EX_CLIENTEDGE.0
            | WS_EX_STATICEDGE.0
            | WS_EX_DLGMODALFRAME.0) as i32;
        if ex & ex_strip != 0 {
            let _ = SetWindowLongW(hwnd, GWL_EXSTYLE, ex & !ex_strip);
        }
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
        // Undo ExtendFrame(-1) if a previous build applied it (white outline).
        {
            use windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
            use windows::Win32::UI::Controls::MARGINS;
            let margins = MARGINS {
                cxLeftWidth: 0,
                cxRightWidth: 0,
                cyTopHeight: 0,
                cyBottomHeight: 0,
            };
            let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
        }
        let corner = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const DWM_WINDOW_CORNER_PREFERENCE as *const c_void,
            std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
        let border = DWMWA_COLOR_NONE;
        for attr in [DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR] {
            let _ = DwmSetWindowAttribute(
                hwnd,
                attr,
                &border as *const u32 as *const c_void,
                std::mem::size_of::<u32>() as u32,
            );
        }
    }
    let _ = dark;
    Ok(())
}

/// Clip a dock HWND (icons or glass) to the CSS pill so DWM cannot draw a
/// sharp rectangular white outline around transparent bounds.
pub fn apply_dock_pill_region(window: &WebviewWindow, radius_logical: f64) {
    let Ok(hwnd) = hwnd_of(window) else {
        return;
    };
    unsafe {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, SetWindowRgn};
        use windows::Win32::UI::HiDpi::GetDpiForWindow;
        use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

        let mut rc = RECT::default();
        if GetClientRect(hwnd, &mut rc).is_err() {
            return;
        }
        let w = rc.right - rc.left;
        let h = rc.bottom - rc.top;
        if w <= 0 || h <= 0 {
            return;
        }

        let dpi = GetDpiForWindow(hwnd).max(96);
        let scale = f64::from(dpi) / 96.0;
        let radius = radius_logical.clamp(8.0, 28.0);
        let r = ((radius * scale).round() as i32).clamp(4, h.max(8));
        let hrgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, r * 2, r * 2);
        let _ = SetWindowRgn(hwnd, hrgn, true);

        let corner = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const DWM_WINDOW_CORNER_PREFERENCE as *const c_void,
            std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
        let border = DWMWA_COLOR_NONE;
        for attr in [DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR] {
            let _ = DwmSetWindowAttribute(
                hwnd,
                attr,
                &border as *const u32 as *const c_void,
                std::mem::size_of::<u32>() as u32,
            );
        }
    }
}

/// Dock glass strip: clip HWND to the CSS pill radius (legacy helper).
pub fn apply_dock_glass_shape(window: &WebviewWindow, radius_logical: f64) {
    if is_hard_safe() {
        return;
    }
    apply_dock_pill_region(window, radius_logical);
}

/// Re-assert icons-clear after place/resize.
/// Do NOT SetWindowRgn on the icons HWND — a round clip makes edge icons
/// (esp. the rightmost ephemeral app) ignore clicks in the clipped corners.
pub fn refresh_dock_layers(
    icons: &WebviewWindow,
    glass: Option<&WebviewWindow>,
    radius_logical: f64,
) {
    let _ = apply_dock_icons_layer(icons, None);
    if let Some(g) = glass {
        if !is_hard_safe() {
            apply_dock_glass_shape(g, radius_logical);
        }
    }
}

/// Main top bar: MyDockFinder-like frosted blur under the ambient tint strip.
/// Never falls back to opaque solid (that would kill ambient painting).
pub fn apply_topbar_frost(window: &WebviewWindow) -> Result<(), String> {
    ensure_hard_safe_detected();
    // Win10 / transparency-off: keep HWND clear so CSS ambient + soft charcoal show.
    if is_hard_safe() {
        clear_webview_fill(window);
        return Ok(());
    }
    let hwnd = hwnd_of(window)?;
    disable_system_backdrop(hwnd);
    // Soft dark frost tint — ambient strip paints translucent color on top.
    let tint = pack_gradient(36, 36, 40, 72);
    let ok = set_window_composition_attribute(
        hwnd,
        ACCENT_ENABLE_BLURBEHIND,
        ACCENT_FLAGS_BLUR_FULL,
        tint,
    ) || set_window_composition_attribute(
        hwnd,
        ACCENT_ENABLE_ACRYLICBLURBEHIND,
        ACCENT_FLAGS_BLUR_FULL,
        tint,
    );
    clear_webview_fill(window);
    unsafe {
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
    if !ok {
        // Soft-fail: transparent bar + CSS tint still looks better than dead black.
        return Ok(());
    }
    Ok(())
}

pub fn apply_effect(
    window: &WebviewWindow,
    kind: WindowMaterial,
    dark: Option<bool>,
    alpha: u8,
) -> Result<(), String> {
    ensure_hard_safe_detected();
    // REGRESSION: never apply mica/acrylic to the icons HWND — it is larger than
    // the CSS pill; SystemBackdrop paints a rectangular white frame around the dock.
    if window.label() == "dock" {
        return apply_dock_icons_layer(window, dark);
    }

    // Dock glass: never apply Win11 SWCA / mica / acrylic.
    // Material rectangle edges show as white frames beside the CSS pill.
    // Frost is painted by `.dock-chrome` on the icons layer instead.
    if window.label() == "dock-glass" {
        clear_vibrancy(window);
        let hwnd = hwnd_of(window)?;
        let _ = set_window_composition_attribute(hwnd, ACCENT_DISABLED, 0, 0);
        disable_system_backdrop(hwnd);
        clear_webview_fill(window);
        return Ok(());
    }

    if is_hard_safe() {
        // Dock preview: always dark opaque — light system theme painted a blank white box.
        if window.label() == "dock-preview" {
            return apply_opaque_solid(window, Some(true));
        }
        return apply_opaque_solid(window, dark);
    }

    let hwnd = hwnd_of(window)?;
    let a = alpha.max(1);

    let result = match kind {
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
                Err("apply Blur (SWCA blurbehind) failed".into())
            } else {
                Ok(())
            }
        }
        WindowMaterial::Aero => {
            clear_vibrancy(window);
            disable_system_backdrop(hwnd);
            let color = pack_gradient(200, 210, 230, a.min(140).max(40));
            if !set_window_composition_attribute(
                hwnd,
                ACCENT_ENABLE_BLURBEHIND,
                ACCENT_FLAGS_BLUR_FULL,
                color,
            ) {
                Err("apply Aero failed".into())
            } else {
                Ok(())
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
                    ACCENT_ENABLE_BLURBEHIND,
                    ACCENT_FLAGS_BLUR_FULL,
                    color,
                ) {
                    Err("apply Acrylic failed".into())
                } else {
                    Ok(())
                }
            } else {
                Ok(())
            }
        }
        WindowMaterial::MicaAlt => apply_system_mica(window, dark),
    };

    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            arm_hard_safe(&e);
            apply_opaque_solid(window, dark)
        }
    }
}
