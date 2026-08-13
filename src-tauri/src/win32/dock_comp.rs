//! Dock glass via Windows.UI.Composition under WebView2.
//!
//! DesktopWindowTarget(`isTopmost=false`) paints **below** the WebView2 host.
//! HostBackdropBrush (+ tint) samples the desktop; RoundedRectangleGeometry clip
//! gives pixel-true capsule corners (beyond DWM 8px).
//!
//! Note: `RectangleClip` Left/Right/Top/Bottom are **insets**, not absolute size —
//! we use `CompositionRoundedRectangleGeometry` instead.

#![cfg(windows)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use parking_lot::Mutex as ParkingMutex;
use tauri::WebviewWindow;
use windows::core::Interface;
use windows::Foundation::Numerics::{Vector2, Vector3};
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{
    CompositionGeometricClip, CompositionRoundedRectangleGeometry, Compositor, ContainerVisual,
    SpriteVisual,
};
use windows::UI::Color;
use windows::Win32::Foundation::{BOOL, HWND};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_USE_HOSTBACKDROPBRUSH, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_DONOTROUND, DWM_WINDOW_CORNER_PREFERENCE,
};
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::System::WinRT::{
    CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_NONE,
    DQTYPE_THREAD_CURRENT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetWindowLongW, SetWindowLongW, GWL_EXSTYLE, WS_EX_NOREDIRECTIONBITMAP,
};
use windows::System::DispatcherQueueController;

/// Above this, Composition owns the silhouette (DWM ROUND is only ~8px).
pub const DOCK_COMP_RADIUS_MIN: u32 = 9;
pub const DOCK_CORNER_RADIUS_MAX: u32 = 30;

fn pack_tint_color(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color {
        A: a,
        R: r,
        G: g,
        B: b,
    }
}

static DISPATCHER: Mutex<Option<DispatcherQueueController>> = Mutex::new(None);
static COMP_THREAD: ParkingMutex<Option<std::thread::ThreadId>> = ParkingMutex::new(None);
static SESSION: ParkingMutex<Option<DockCompSession>> = ParkingMutex::new(None);
static GLASS_WIN: ParkingMutex<Option<WebviewWindow>> = ParkingMutex::new(None);
/// While true, ignore client-size auto layout (resize hooks would flash full-width capsule).
static WIDTH_TWEEN_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Last intended visual capsule inside the (always host-sized) glass HWND.
/// Rest = content width + centered offset; hover = full host. Frost re-attach
/// must honor this — otherwise deferred material refresh paints “already wide”.
#[derive(Clone, Copy)]
struct CapsulePose {
    w: f32,
    h: f32,
    ox: f32,
}
static PREFERRED_CAPSULE: ParkingMutex<Option<CapsulePose>> = ParkingMutex::new(None);

pub fn begin_width_tween() {
    WIDTH_TWEEN_ACTIVE.store(true, Ordering::SeqCst);
}

pub fn end_width_tween() {
    WIDTH_TWEEN_ACTIVE.store(false, Ordering::SeqCst);
}

pub fn width_tween_active() -> bool {
    WIDTH_TWEEN_ACTIVE.load(Ordering::SeqCst)
}

/// Remember the visual capsule pose so later `attach_or_update` (frost retries)
/// does not snap back to a full-bleed host (= looks pre-widened at rest).
pub fn remember_capsule(width_px: f32, height_px: f32, offset_x: f32) {
    *PREFERRED_CAPSULE.lock() = Some(CapsulePose {
        w: width_px.max(1.0),
        h: height_px.max(1.0),
        ox: offset_x.max(0.0),
    });
}

fn preferred_capsule() -> Option<CapsulePose> {
    *PREFERRED_CAPSULE.lock()
}

struct DockCompSession {
    hwnd_raw: isize,
    _target: DesktopWindowTarget,
    root: ContainerVisual,
    blur: SpriteVisual,
    tint: SpriteVisual,
    round_geom: CompositionRoundedRectangleGeometry,
    _clip: CompositionGeometricClip,
    compositor: Compositor,
}

fn ensure_dispatcher_queue() -> Result<(), String> {
    let mut slot = DISPATCHER
        .lock()
        .map_err(|_| "dispatcher lock poisoned".to_string())?;
    if slot.is_some() {
        return Ok(());
    }
    let opts = DispatcherQueueOptions {
        dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
        threadType: DQTYPE_THREAD_CURRENT,
        apartmentType: DQTAT_COM_NONE,
    };
    let controller = unsafe { CreateDispatcherQueueController(opts) }
        .map_err(|e| format!("CreateDispatcherQueueController: {e}"))?;
    *slot = Some(controller);
    *COMP_THREAD.lock() = Some(std::thread::current().id());
    Ok(())
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

/// Win32 callers need this accent so HostBackdropBrush is not a black slab.
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

fn client_size_px(hwnd: HWND) -> Option<(f32, f32)> {
    use windows::Win32::Foundation::RECT;
    unsafe {
        let mut cr = RECT::default();
        if GetClientRect(hwnd, &mut cr).is_err() || cr.right <= 0 || cr.bottom <= 0 {
            return None;
        }
        Some((cr.right as f32, cr.bottom as f32))
    }
}

fn radius_px(hwnd: HWND, radius_logical: u32, height_px: f32) -> f32 {
    let dpi = unsafe { GetDpiForWindow(hwnd).max(96) } as f32;
    let r = (radius_logical as f32) * dpi / 96.0;
    r.clamp(1.0, height_px * 0.5)
}

fn tint_for(dark: Option<bool>) -> Color {
    if dark == Some(false) {
        pack_tint_color(245, 245, 250, 120)
    } else {
        pack_tint_color(28, 28, 30, 110)
    }
}

fn build_session(hwnd: HWND, dark: Option<bool>) -> Result<DockCompSession, String> {
    ensure_dispatcher_queue()?;
    clear_noredirectionbitmap(hwnd);
    enable_host_backdrop_attr(hwnd, true);
    prime_host_backdrop_accent(hwnd);
    force_dwm_donotround(hwnd);

    let compositor =
        Compositor::new().map_err(|e| format!("Compositor::new: {e}"))?;
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

    let children = root
        .Children()
        .map_err(|e| format!("Children: {e}"))?;
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

    Ok(DockCompSession {
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

fn layout_session(
    session: &DockCompSession,
    hwnd: HWND,
    radius_logical: u32,
    dark: Option<bool>,
) -> Result<(), String> {
    if let Some(p) = preferred_capsule() {
        return layout_capsule(session, hwnd, p.w, p.h, p.ox, radius_logical, dark);
    }
    let (w, h) = client_size_px(hwnd).ok_or_else(|| "dock-comp: empty client".to_string())?;
    layout_session_size(session, hwnd, w, h, radius_logical, dark)
}

fn layout_session_size(
    session: &DockCompSession,
    hwnd: HWND,
    w: f32,
    h: f32,
    radius_logical: u32,
    dark: Option<bool>,
) -> Result<(), String> {
    // Prefer remembered rest/hover pose over a full-host size hint from place.
    if let Some(p) = preferred_capsule() {
        return layout_capsule(session, hwnd, p.w, p.h, p.ox, radius_logical, dark);
    }
    layout_capsule(session, hwnd, w, h, 0.0, radius_logical, dark)
}

/// Place the frosted capsule inside the (possibly wider) glass HWND.
/// `offset_x` centers a shrinking/growing capsule while the HWND stays put.
fn layout_capsule(
    session: &DockCompSession,
    hwnd: HWND,
    w: f32,
    h: f32,
    offset_x: f32,
    radius_logical: u32,
    dark: Option<bool>,
) -> Result<(), String> {
    layout_capsule_inner(session, hwnd, w, h, offset_x, radius_logical, dark, true)
}

fn layout_capsule_inner(
    session: &DockCompSession,
    hwnd: HWND,
    w: f32,
    h: f32,
    offset_x: f32,
    radius_logical: u32,
    dark: Option<bool>,
    retint: bool,
) -> Result<(), String> {
    let r = radius_px(hwnd, radius_logical, h);
    let size = Vector2 {
        X: w.max(1.0),
        Y: h.max(1.0),
    };
    let origin = Vector3 {
        X: offset_x,
        Y: 0.0,
        Z: 0.0,
    };
    let zero = Vector3 {
        X: 0.0,
        Y: 0.0,
        Z: 0.0,
    };
    let corner = Vector2 { X: r, Y: r };

    session
        .root
        .SetSize(size)
        .map_err(|e| format!("root SetSize: {e}"))?;
    session
        .root
        .SetOffset(origin)
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
        .SetCornerRadius(corner)
        .map_err(|e| format!("geom SetCornerRadius: {e}"))?;

    if retint {
        let color_brush = session
            .compositor
            .CreateColorBrushWithColor(tint_for(dark))
            .map_err(|e| format!("retint: {e}"))?;
        session
            .tint
            .SetBrush(&color_brush)
            .map_err(|e| format!("retint SetBrush: {e}"))?;
    }

    let _ = session.compositor.RequestCommitAsync();
    Ok(())
}

/// Geometry-only update for width tween frames (no accent / chrome / retint churn).
pub fn layout_tween_frame(
    hwnd: HWND,
    width_px: f32,
    height_px: f32,
    offset_x: f32,
    radius_logical: u32,
) -> Result<(), String> {
    let r = radius_logical.min(DOCK_CORNER_RADIUS_MAX);
    if r < DOCK_COMP_RADIUS_MIN {
        return Ok(());
    }
    let raw = hwnd.0 as isize;
    let slot = SESSION.lock();
    let Some(session) = slot.as_ref() else {
        return Ok(());
    };
    if session.hwnd_raw != raw {
        return Ok(());
    }
    remember_capsule(width_px, height_px, offset_x);
    layout_capsule_inner(session, hwnd, width_px, height_px, offset_x, r, None, false)
}

pub fn sync_layout_tween_frame(
    hwnd: HWND,
    width_px: f32,
    height_px: f32,
    offset_x: f32,
    radius_logical: u32,
) -> Result<(), String> {
    if on_composition_thread() || COMP_THREAD.lock().is_none() {
        return layout_tween_frame(hwnd, width_px, height_px, offset_x, radius_logical);
    }
    let raw = hwnd.0 as isize;
    let win = GLASS_WIN.lock().clone();
    if let Some(w) = win {
        let _ = w.run_on_main_thread(move || {
            let h = HWND(raw as *mut _);
            let _ = layout_tween_frame(h, width_px, height_px, offset_x, radius_logical);
        });
        Ok(())
    } else {
        layout_tween_frame(hwnd, width_px, height_px, offset_x, radius_logical)
    }
}

/// One-shot seed for capsule pose — waits briefly so the first painted frame is
/// already centered (avoids a left-aligned flash after host pin).
pub fn sync_layout_tween_frame_wait(
    hwnd: HWND,
    width_px: f32,
    height_px: f32,
    offset_x: f32,
    radius_logical: u32,
) -> Result<(), String> {
    if on_composition_thread() || COMP_THREAD.lock().is_none() {
        return layout_tween_frame(hwnd, width_px, height_px, offset_x, radius_logical);
    }
    let raw = hwnd.0 as isize;
    let win = GLASS_WIN.lock().clone();
    let Some(w) = win else {
        return layout_tween_frame(hwnd, width_px, height_px, offset_x, radius_logical);
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = w.run_on_main_thread(move || {
        let h = HWND(raw as *mut _);
        let r = layout_tween_frame(h, width_px, height_px, offset_x, radius_logical);
        let _ = tx.send(r);
    });
    match rx.recv_timeout(std::time::Duration::from_millis(80)) {
        Ok(r) => r,
        Err(_) => Ok(()),
    }
}

/// Tear down Composition session (restore SWCA path caller responsibility).
pub fn detach() {
    let mut slot = SESSION.lock();
    if let Some(session) = slot.take() {
        enable_host_backdrop_attr(HWND(session.hwnd_raw as *mut _), false);
    }
}

/// Attach or refresh Composition acrylic under WebView2. Call on UI thread.
pub fn attach_or_update(hwnd: HWND, radius_logical: u32, dark: Option<bool>) -> Result<(), String> {
    attach_or_update_sized(hwnd, None, radius_logical, dark)
}

pub fn attach_or_update_sized(
    hwnd: HWND,
    size_px: Option<(f32, f32)>,
    radius_logical: u32,
    dark: Option<bool>,
) -> Result<(), String> {
    let r = radius_logical.min(DOCK_CORNER_RADIUS_MAX);
    if r < DOCK_COMP_RADIUS_MIN {
        detach();
        return Ok(());
    }

    // During capsule width tween, never re-layout from full client size — that
    // flashes the expanded right edge before the L/R grow animation.
    if width_tween_active() && size_px.is_none() {
        return Ok(());
    }

    let raw = hwnd.0 as isize;
    let slot = SESSION.lock();
    let needs_rebuild = match slot.as_ref() {
        None => true,
        Some(s) => s.hwnd_raw != raw,
    };
    if needs_rebuild {
        drop(slot);
        detach();
        let session = build_session(hwnd, dark)?;
        match size_px {
            Some((w, h)) => layout_session_size(&session, hwnd, w, h, r, dark)?,
            None => layout_session(&session, hwnd, r, dark)?,
        }
        *SESSION.lock() = Some(session);
        return Ok(());
    }

    if let Some(session) = slot.as_ref() {
        match size_px {
            Some((w, h)) => layout_session_size(session, hwnd, w, h, r, dark)?,
            None => layout_session(session, hwnd, r, dark)?,
        }
    }
    Ok(())
}

pub fn remember_glass_window(window: &WebviewWindow) {
    *GLASS_WIN.lock() = Some(window.clone());
}

fn on_composition_thread() -> bool {
    COMP_THREAD.lock().as_ref() == Some(&std::thread::current().id())
}

pub fn sync_attach_or_update_sized(
    hwnd: HWND,
    size_px: Option<(f32, f32)>,
    radius_logical: u32,
    dark: Option<bool>,
) -> Result<(), String> {
    // Never block-wait on run_on_main_thread — that deadlocks when the caller is
    // already the UI thread (Tauri sync commands) and freezes the whole app.
    if on_composition_thread() || COMP_THREAD.lock().is_none() {
        return attach_or_update_sized(hwnd, size_px, radius_logical, dark);
    }
    let raw = hwnd.0 as isize;
    let win = GLASS_WIN.lock().clone();
    if let Some(w) = win {
        let _ = w.run_on_main_thread(move || {
            let h = HWND(raw as *mut _);
            let _ = attach_or_update_sized(h, size_px, radius_logical, dark);
        });
        Ok(())
    } else {
        attach_or_update_sized(hwnd, size_px, radius_logical, dark)
    }
}

pub fn uses_composition(radius_logical: u32) -> bool {
    radius_logical >= DOCK_COMP_RADIUS_MIN
}
