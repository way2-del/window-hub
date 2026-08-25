//! Top status-bar frost via Windows.UI.Composition **under** WebView2 on `main`.
//!
//! Replaces the sibling `island-bar-glass` HWND (DWM Z-order flicker when framed
//! windows close). DesktopWindowTarget paints only the top ~28px strip; expanded
//! panel sides stay fully clear.

#![cfg(windows)]

use parking_lot::Mutex;
use tauri::WebviewWindow;
use windows::core::Interface;
use windows::Foundation::Numerics::{Vector2, Vector3};
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{
    CompositionGeometricClip, CompositionRoundedRectangleGeometry, Compositor, ContainerVisual,
    SpriteVisual,
};
use windows::UI::Color;
use windows::Win32::Foundation::{BOOL, HWND, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_USE_HOSTBACKDROPBRUSH, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_DONOTROUND, DWM_WINDOW_CORNER_PREFERENCE,
};
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetWindowLongW, SetWindowLongW, GWL_EXSTYLE, WS_EX_NOREDIRECTIONBITMAP,
};

const BAR_H_LOGICAL: f32 = 28.0;

static SESSION: Mutex<Option<BarCompSession>> = Mutex::new(None);

struct BarCompSession {
    hwnd_raw: isize,
    _target: DesktopWindowTarget,
    root: ContainerVisual,
    blur: SpriteVisual,
    tint: SpriteVisual,
    round_geom: CompositionRoundedRectangleGeometry,
    _clip: CompositionGeometricClip,
    compositor: Compositor,
}

fn pack_tint_color(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color {
        A: a,
        R: r,
        G: g,
        B: b,
    }
}

fn tint_for(dark: Option<bool>) -> Color {
    if crate::win32::dock_comp::resolve_theme_dark(dark) {
        pack_tint_color(28, 28, 30, 110)
    } else {
        pack_tint_color(245, 245, 250, 120)
    }
}

fn enable_host_backdrop_attr(hwnd: HWND, on: bool) {
    let v: BOOL = on.into();
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_HOSTBACKDROPBRUSH,
            &v as *const BOOL as *const _,
            std::mem::size_of::<BOOL>() as u32,
        );
    }
}

fn prime_host_backdrop_accent(hwnd: HWND) {
    use std::ffi::c_void;
    use windows::core::s;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

    #[repr(C)]
    struct AccentPolicy {
        accent_state: u32,
        accent_flags: u32,
        gradient_color: u32,
        animation_id: u32,
    }
    #[repr(C)]
    struct AttribData {
        attrib: u32,
        pv_data: *mut c_void,
        cb_data: usize,
    }
    type SetFn = unsafe extern "system" fn(HWND, *mut AttribData) -> BOOL;
    const WCA_ACCENT_POLICY: u32 = 19;
    const ACCENT_ENABLE_HOSTBACKDROP: u32 = 5;

    unsafe {
        let Ok(module) = LoadLibraryA(s!("user32.dll")) else {
            return;
        };
        let Some(proc) = GetProcAddress(module, s!("SetWindowCompositionAttribute")) else {
            return;
        };
        let set_attr: SetFn = std::mem::transmute(proc);
        let mut policy = AccentPolicy {
            accent_state: ACCENT_ENABLE_HOSTBACKDROP,
            accent_flags: 0,
            gradient_color: 0,
            animation_id: 0,
        };
        let mut data = AttribData {
            attrib: WCA_ACCENT_POLICY,
            pv_data: &mut policy as *mut _ as *mut c_void,
            cb_data: std::mem::size_of::<AccentPolicy>(),
        };
        let _ = set_attr(hwnd, &mut data);
    }
}

fn clear_noredirectionbitmap(hwnd: HWND) {
    unsafe {
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex & WS_EX_NOREDIRECTIONBITMAP.0 != 0 {
            SetWindowLongW(
                hwnd,
                GWL_EXSTYLE,
                (ex & !WS_EX_NOREDIRECTIONBITMAP.0) as i32,
            );
        }
    }
}

fn force_dwm_donotround(hwnd: HWND) {
    let corner = DWMWCP_DONOTROUND;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const DWM_WINDOW_CORNER_PREFERENCE as *const _,
            std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
    }
}

fn bar_height_px(hwnd: HWND) -> f32 {
    let dpi = unsafe { GetDpiForWindow(hwnd).max(96) } as f32;
    BAR_H_LOGICAL * dpi / 96.0
}

fn client_width_px(hwnd: HWND) -> Option<f32> {
    unsafe {
        let mut rc = RECT::default();
        if GetClientRect(hwnd, &mut rc).is_err() || rc.right <= 0 {
            return None;
        }
        Some(rc.right as f32)
    }
}

fn build_session(hwnd: HWND, dark: Option<bool>) -> Result<BarCompSession, String> {
    crate::win32::dock_comp::ensure_shared_dispatcher()?;
    clear_noredirectionbitmap(hwnd);
    enable_host_backdrop_attr(hwnd, true);
    prime_host_backdrop_accent(hwnd);
    force_dwm_donotround(hwnd);

    let compositor = Compositor::new().map_err(|e| format!("Compositor::new: {e}"))?;
    let interop: ICompositorDesktopInterop = compositor
        .cast()
        .map_err(|e| format!("ICompositorDesktopInterop: {e}"))?;
    let target = unsafe { interop.CreateDesktopWindowTarget(hwnd, false) }
        .map_err(|e| format!("CreateDesktopWindowTarget: {e}"))?;

    let root = compositor
        .CreateContainerVisual()
        .map_err(|e| format!("CreateContainerVisual: {e}"))?;
    let blur = compositor
        .CreateSpriteVisual()
        .map_err(|e| format!("blur SpriteVisual: {e}"))?;
    let tint = compositor
        .CreateSpriteVisual()
        .map_err(|e| format!("tint SpriteVisual: {e}"))?;

    let host_brush = compositor
        .CreateHostBackdropBrush()
        .map_err(|e| format!("CreateHostBackdropBrush: {e}"))?;
    blur
        .SetBrush(&host_brush)
        .map_err(|e| format!("blur SetBrush: {e}"))?;

    let color_brush = compositor
        .CreateColorBrushWithColor(tint_for(dark))
        .map_err(|e| format!("CreateColorBrush: {e}"))?;
    tint
        .SetBrush(&color_brush)
        .map_err(|e| format!("tint SetBrush: {e}"))?;

    let children = root.Children().map_err(|e| format!("Children: {e}"))?;
    children
        .InsertAtTop(&blur)
        .map_err(|e| format!("Insert blur: {e}"))?;
    children
        .InsertAtTop(&tint)
        .map_err(|e| format!("Insert tint: {e}"))?;

    let round_geom = compositor
        .CreateRoundedRectangleGeometry()
        .map_err(|e| format!("CreateRoundedRectangleGeometry: {e}"))?;
    let clip = compositor
        .CreateGeometricClipWithGeometry(&round_geom)
        .map_err(|e| format!("CreateGeometricClip: {e}"))?;
    root.SetClip(&clip)
        .map_err(|e| format!("SetClip: {e}"))?;

    target
        .SetRoot(&root)
        .map_err(|e| format!("SetRoot: {e}"))?;

    Ok(BarCompSession {
        hwnd_raw: hwnd.0 as isize,
        _target: target,
        root,
        blur,
        tint,
        round_geom,
        _clip: clip,
        compositor,
    })
}

fn layout_strip(session: &BarCompSession, hwnd: HWND, dark: Option<bool>) -> Result<(), String> {
    let w = client_width_px(hwnd).ok_or("bar-comp: empty client")?;
    let h = bar_height_px(hwnd).max(1.0);
    let size = Vector2 { X: w.max(1.0), Y: h };
    let zero = Vector3 {
        X: 0.0,
        Y: 0.0,
        Z: 0.0,
    };

    session
        .root
        .SetSize(size)
        .map_err(|e| format!("root SetSize: {e}"))?;
    session
        .root
        .SetOffset(zero)
        .map_err(|e| format!("root SetOffset: {e}"))?;
    session
        .blur
        .SetSize(size)
        .map_err(|e| format!("blur SetSize: {e}"))?;
    session
        .blur
        .SetOffset(zero)
        .map_err(|e| format!("blur SetOffset: {e}"))?;
    session
        .tint
        .SetSize(size)
        .map_err(|e| format!("tint SetSize: {e}"))?;
    session
        .tint
        .SetOffset(zero)
        .map_err(|e| format!("tint SetOffset: {e}"))?;
    session
        .round_geom
        .SetSize(size)
        .map_err(|e| format!("geom SetSize: {e}"))?;
    session
        .round_geom
        .SetCornerRadius(Vector2 { X: 0.0, Y: 0.0 })
        .map_err(|e| format!("geom SetCornerRadius: {e}"))?;

    let color_brush = session
        .compositor
        .CreateColorBrushWithColor(tint_for(dark))
        .map_err(|e| format!("retint: {e}"))?;
    session
        .tint
        .SetBrush(&color_brush)
        .map_err(|e| format!("retint SetBrush: {e}"))?;

    let _ = session.compositor.RequestCommitAsync();
    Ok(())
}

/// Tear down bar Composition (e.g. bar glass off or over maximized window).
pub fn detach() {
    let mut slot = SESSION.lock();
    if let Some(session) = slot.take() {
        enable_host_backdrop_attr(HWND(session.hwnd_raw as *mut _), false);
    }
}

pub fn is_attached_to(hwnd_raw: isize) -> bool {
    SESSION
        .lock()
        .as_ref()
        .is_some_and(|s| s.hwnd_raw == hwnd_raw)
}

/// Attach or refresh the top-strip HostBackdrop under WebView2. UI thread only.
pub fn attach_or_update(hwnd: HWND, dark: Option<bool>) -> Result<(), String> {
    let dark = Some(crate::win32::dock_comp::resolve_theme_dark(dark));
    let raw = hwnd.0 as isize;
    let needs_rebuild = match SESSION.lock().as_ref() {
        None => true,
        Some(s) => s.hwnd_raw != raw,
    };
    if needs_rebuild {
        detach();
        let session = build_session(hwnd, dark)?;
        layout_strip(&session, hwnd, dark)?;
        *SESSION.lock() = Some(session);
        return Ok(());
    }
    if let Some(session) = SESSION.lock().as_ref() {
        layout_strip(session, hwnd, dark)?;
    }
    Ok(())
}

/// Relayout after monitor move / width change — no session rebuild.
pub fn refresh_layout(hwnd: HWND, dark: Option<bool>) -> Result<(), String> {
    let raw = hwnd.0 as isize;
    let slot = SESSION.lock();
    let Some(session) = slot.as_ref() else {
        return Ok(());
    };
    if session.hwnd_raw != raw {
        return Ok(());
    }
    layout_strip(session, hwnd, Some(crate::win32::dock_comp::resolve_theme_dark(dark)))
}

/// Marshals to the main WebView thread when called off-UI (ambient debounce).
pub fn sync_attach_or_update(window: &WebviewWindow, dark: Option<bool>) -> Result<(), String> {
    let raw = window
        .hwnd()
        .map_err(|e| format!("bar-comp hwnd: {e}"))?
        .0 as isize;
    let dark_c = dark;
    if window.label() == "main" {
        return attach_or_update(HWND(raw as *mut _), dark_c);
    }
    let w = window.clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let _ = w.run_on_main_thread(move || {
        let h = HWND(raw as *mut _);
        let r = attach_or_update(h, dark_c);
        let _ = tx.send(r);
    });
    rx.recv_timeout(std::time::Duration::from_millis(500))
        .map_err(|_| "bar-comp attach timeout".to_string())?
}
