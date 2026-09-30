//! Dock glass via Windows.UI.Composition under WebView2.
//!
//! DesktopWindowTarget(`isTopmost=false`) paints **below** the WebView2 host.
//! HostBackdropBrush (+ tint) samples the desktop; RoundedRectangleGeometry clip
//! gives pixel-true capsule corners (beyond DWM 8px).
//!
//! Note: `RectangleClip` Left/Right/Top/Bottom are **insets**, not absolute size —
//! we use `CompositionRoundedRectangleGeometry` instead.

#![cfg(windows)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI8, Ordering};
use std::sync::{Mutex, OnceLock};

use parking_lot::Mutex as ParkingMutex;
use tauri::WebviewWindow;
use windows::core::{Interface, HSTRING};
use windows::Foundation::Numerics::{Vector2, Vector3};
use windows::Foundation::TimeSpan;
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
/// Rest ↔ hover capsule width (must match FE chrome `width` transition).
pub const DOCK_WIDTH_TWEEN_MS: u64 = 240;

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

fn sessions() -> &'static ParkingMutex<HashMap<isize, DockCompSession>> {
    static S: OnceLock<ParkingMutex<HashMap<isize, DockCompSession>>> = OnceLock::new();
    S.get_or_init(|| ParkingMutex::new(HashMap::new()))
}

fn glass_wins() -> &'static ParkingMutex<HashMap<isize, WebviewWindow>> {
    static S: OnceLock<ParkingMutex<HashMap<isize, WebviewWindow>>> = OnceLock::new();
    S.get_or_init(|| ParkingMutex::new(HashMap::new()))
}

fn preferred_capsules() -> &'static ParkingMutex<HashMap<isize, CapsulePose>> {
    static S: OnceLock<ParkingMutex<HashMap<isize, CapsulePose>>> = OnceLock::new();
    S.get_or_init(|| ParkingMutex::new(HashMap::new()))
}

/// While true, ignore client-size auto layout (resize hooks would flash full-width capsule).
static WIDTH_TWEEN_ACTIVE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
struct CapsulePose {
    w: f32,
    h: f32,
    ox: f32,
}

/// Last resolved dock theme: -1 unknown, 0 light, 1 dark.
/// Place/resize frost refresh often passes `dark: None` — must not fall back to dark tint.
static LAST_THEME_DARK: AtomicI8 = AtomicI8::new(-1);

pub fn set_theme_dark(dark: bool) {
    LAST_THEME_DARK.store(if dark { 1 } else { 0 }, Ordering::SeqCst);
}

pub fn theme_dark() -> Option<bool> {
    match LAST_THEME_DARK.load(Ordering::SeqCst) {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

/// Prefer explicit arg; else last dock theme; else Windows Apps theme.
pub fn resolve_theme_dark(dark: Option<bool>) -> bool {
    if let Some(d) = dark {
        set_theme_dark(d);
        return d;
    }
    if let Some(d) = theme_dark() {
        return d;
    }
    let d = crate::win32::material::system_apps_dark();
    set_theme_dark(d);
    d
}

fn preferred_capsule_for(hwnd_raw: isize) -> Option<CapsulePose> {
    preferred_capsules().lock().get(&hwnd_raw).copied()
}

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
pub fn remember_capsule(hwnd_raw: isize, width_px: f32, height_px: f32, offset_x: f32) {
    preferred_capsules().lock().insert(
        hwnd_raw,
        CapsulePose {
            w: width_px.max(1.0),
            h: height_px.max(1.0),
            ox: offset_x.max(0.0),
        },
    );
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

/// Shared WinRT dispatcher for all Composition surfaces (dock-glass + main bar).
pub fn ensure_shared_dispatcher() -> Result<(), String> {
    ensure_dispatcher_queue()
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
    if resolve_theme_dark(dark) {
        pack_tint_color(28, 28, 30, 110)
    } else {
        pack_tint_color(245, 245, 250, 120)
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
    if let Some(p) = preferred_capsule_for(session.hwnd_raw) {
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
    if let Some(p) = preferred_capsule_for(session.hwnd_raw) {
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

fn stop_capsule_anims(session: &DockCompSession) {
    let size = HSTRING::from("Size");
    let offset = HSTRING::from("Offset");
    let _ = session.root.StopAnimation(&size);
    let _ = session.root.StopAnimation(&offset);
    let _ = session.blur.StopAnimation(&size);
    let _ = session.tint.StopAnimation(&size);
    let _ = session.round_geom.StopAnimation(&size);
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
    stop_capsule_anims(session);

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

/// One Vector2 size animation instance (Composition binds 1 anim → 1 property).
fn make_size_anim(
    compositor: &Compositor,
    from: Vector2,
    to: Vector2,
    duration: TimeSpan,
    ease: &windows::UI::Composition::CubicBezierEasingFunction,
) -> Result<windows::UI::Composition::Vector2KeyFrameAnimation, String> {
    let anim = compositor
        .CreateVector2KeyFrameAnimation()
        .map_err(|e| format!("CreateVector2KeyFrameAnimation: {e}"))?;
    anim.InsertKeyFrame(0.0, from)
        .map_err(|e| format!("size InsertKeyFrame 0: {e}"))?;
    anim.InsertKeyFrameWithEasingFunction(1.0, to, ease)
        .map_err(|e| format!("size InsertKeyFrame 1: {e}"))?;
    anim.SetDuration(duration)
        .map_err(|e| format!("size SetDuration: {e}"))?;
    Ok(anim)
}

fn make_offset_anim(
    compositor: &Compositor,
    from: Vector3,
    to: Vector3,
    duration: TimeSpan,
    ease: &windows::UI::Composition::CubicBezierEasingFunction,
) -> Result<windows::UI::Composition::Vector3KeyFrameAnimation, String> {
    let anim = compositor
        .CreateVector3KeyFrameAnimation()
        .map_err(|e| format!("CreateVector3KeyFrameAnimation: {e}"))?;
    anim.InsertKeyFrame(0.0, from)
        .map_err(|e| format!("offset InsertKeyFrame 0: {e}"))?;
    anim.InsertKeyFrameWithEasingFunction(1.0, to, ease)
        .map_err(|e| format!("offset InsertKeyFrame 1: {e}"))?;
    anim.SetDuration(duration)
        .map_err(|e| format!("offset SetDuration: {e}"))?;
    Ok(anim)
}

/// GPU rest↔hover capsule width. Animates from the **current** visual Size
/// (after StopAnimation) so interrupted tweens never snap to a stale from-width
/// (that flash is what left transparent left/right gaps).
/// Returns effective duration ms (0 if already at target).
pub fn animate_capsule_width(
    hwnd: HWND,
    to_w: f32,
    height_px: f32,
    host_w: f32,
    radius_logical: u32,
) -> Result<u64, String> {
    let r = radius_logical.min(DOCK_CORNER_RADIUS_MAX);
    if r < DOCK_COMP_RADIUS_MIN {
        return Err("composition radius too small".into());
    }
    let raw = hwnd.0 as isize;
    let map = sessions().lock();
    let Some(session) = map.get(&raw) else {
        return Err("no composition session".into());
    };

    let h = height_px.max(1.0);
    let to = to_w.max(1.0);
    let host = host_w.max(1.0);
    let to_ox = ((host - to) * 0.5).max(0.0);
    let corner_r = radius_px(hwnd, r, h);

    stop_capsule_anims(session);

    let cur_size = session
        .root
        .Size()
        .unwrap_or(Vector2 { X: to, Y: h });
    let from = cur_size.X.max(1.0);
    let cur_off = session.root.Offset().unwrap_or(Vector3 {
        X: ((host - from) * 0.5).max(0.0),
        Y: 0.0,
        Z: 0.0,
    });
    let from_ox = cur_off.X.clamp(0.0, (host - 1.0).max(0.0));

    // Target pose for frost re-attach while tweening / after settle.
    remember_capsule(raw, to, h, to_ox);

    // Already there — commit pose, no keyframes.
    if (from - to).abs() < 1.5 {
        drop(map);
        layout_tween_frame(hwnd, to, h, to_ox, radius_logical)?;
        return Ok(0);
    }

    let from_size = Vector2 { X: from, Y: h };
    let to_size = Vector2 { X: to, Y: h };
    let from_off = Vector3 {
        X: from_ox,
        Y: 0.0,
        Z: 0.0,
    };
    let to_off = Vector3 {
        X: to_ox,
        Y: 0.0,
        Z: 0.0,
    };
    let zero = Vector3 {
        X: 0.0,
        Y: 0.0,
        Z: 0.0,
    };
    let corner = Vector2 {
        X: corner_r,
        Y: corner_r,
    };

    // Keep current visual as the animation start (no snap).
    session
        .blur
        .SetSize(from_size)
        .map_err(|e| format!("anim seed blur Size: {e}"))?;
    session
        .blur
        .SetOffset(zero)
        .map_err(|e| format!("anim seed blur Offset: {e}"))?;
    session
        .tint
        .SetSize(from_size)
        .map_err(|e| format!("anim seed tint Size: {e}"))?;
    session
        .tint
        .SetOffset(zero)
        .map_err(|e| format!("anim seed tint Offset: {e}"))?;
    session
        .round_geom
        .SetSize(from_size)
        .map_err(|e| format!("anim seed geom Size: {e}"))?;
    session
        .round_geom
        .SetCornerRadius(corner)
        .map_err(|e| format!("anim seed CornerRadius: {e}"))?;

    let ease = session
        .compositor
        .CreateCubicBezierEasingFunction(
            Vector2 { X: 0.22, Y: 1.0 },
            Vector2 { X: 0.36, Y: 1.0 },
        )
        .map_err(|e| format!("CreateCubicBezierEasingFunction: {e}"))?;
    // Shorter travel → shorter tween (fast reverse feels stable, less overlap).
    let travel = ((to - from).abs() / host).clamp(0.35, 1.0);
    let ms = ((DOCK_WIDTH_TWEEN_MS as f32) * travel).round().max(90.0) as u64;
    let duration = TimeSpan {
        Duration: (ms as i64) * 10_000,
    };

    let size_prop = HSTRING::from("Size");
    let offset_prop = HSTRING::from("Offset");
    let root_size = make_size_anim(&session.compositor, from_size, to_size, duration, &ease)?;
    let blur_size = make_size_anim(&session.compositor, from_size, to_size, duration, &ease)?;
    let tint_size = make_size_anim(&session.compositor, from_size, to_size, duration, &ease)?;
    let geom_size = make_size_anim(&session.compositor, from_size, to_size, duration, &ease)?;
    let root_off = make_offset_anim(&session.compositor, from_off, to_off, duration, &ease)?;

    session
        .root
        .StartAnimation(&size_prop, &root_size)
        .map_err(|e| format!("root StartAnimation Size: {e}"))?;
    session
        .root
        .StartAnimation(&offset_prop, &root_off)
        .map_err(|e| format!("root StartAnimation Offset: {e}"))?;
    session
        .blur
        .StartAnimation(&size_prop, &blur_size)
        .map_err(|e| format!("blur StartAnimation Size: {e}"))?;
    session
        .tint
        .StartAnimation(&size_prop, &tint_size)
        .map_err(|e| format!("tint StartAnimation Size: {e}"))?;
    session
        .round_geom
        .StartAnimation(&size_prop, &geom_size)
        .map_err(|e| format!("geom StartAnimation Size: {e}"))?;

    let _ = session.compositor.RequestCommitAsync();
    Ok(ms)
}

/// Start GPU width tween on the UI thread; returns once animations are committed.
/// Returns the effective duration ms used (scaled by travel).
pub fn sync_animate_capsule_width(
    hwnd: HWND,
    to_w: f32,
    height_px: f32,
    host_w: f32,
    radius_logical: u32,
) -> Result<u64, String> {
    if on_composition_thread() || COMP_THREAD.lock().is_none() {
        return animate_capsule_width(hwnd, to_w, height_px, host_w, radius_logical);
    }
    let raw = hwnd.0 as isize;
    let win = glass_win_for(raw);
    let Some(w) = win else {
        return animate_capsule_width(hwnd, to_w, height_px, host_w, radius_logical);
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = w.run_on_main_thread(move || {
        let h = HWND(raw as *mut _);
        let r = animate_capsule_width(h, to_w, height_px, host_w, radius_logical);
        let _ = tx.send(r);
    });
    match rx.recv_timeout(std::time::Duration::from_millis(80)) {
        Ok(r) => r,
        Err(_) => Ok(DOCK_WIDTH_TWEEN_MS),
    }
}

/// True when preferred capsule is already within ~2px of the expected rest/hover width.
pub fn capsule_near_target(hwnd_raw: isize, expanded: bool, content_px: f32, host_px: f32) -> bool {
    let Some(p) = preferred_capsule_for(hwnd_raw) else {
        return false;
    };
    let expect = if expanded {
        host_px.max(1.0)
    } else {
        content_px.max(1.0)
    };
    (p.w - expect).abs() < 2.5
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
    let map = sessions().lock();
    let Some(session) = map.get(&raw) else {
        return Ok(());
    };
    remember_capsule(raw, width_px, height_px, offset_x);
    layout_capsule_inner(session, hwnd, width_px, height_px, offset_x, r, None, false)
}

fn glass_win_for(hwnd_raw: isize) -> Option<WebviewWindow> {
    glass_wins().lock().get(&hwnd_raw).cloned()
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
    if let Some(w) = glass_win_for(raw) {
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
    let Some(w) = glass_win_for(raw) else {
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

/// Tear down all Composition sessions.
pub fn detach() {
    let mut map = sessions().lock();
    for (_, session) in map.drain() {
        enable_host_backdrop_attr(HWND(session.hwnd_raw as *mut _), false);
    }
    preferred_capsules().lock().clear();
}

/// Tear down one glass HWND (satellite destroy / radius demote).
pub fn detach_hwnd(hwnd_raw: isize) {
    if let Some(session) = sessions().lock().remove(&hwnd_raw) {
        enable_host_backdrop_attr(HWND(session.hwnd_raw as *mut _), false);
    }
    preferred_capsules().lock().remove(&hwnd_raw);
    glass_wins().lock().remove(&hwnd_raw);
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
    let dark = Some(resolve_theme_dark(dark));
    let r = radius_logical.min(DOCK_CORNER_RADIUS_MAX);
    let raw = hwnd.0 as isize;
    if r < DOCK_COMP_RADIUS_MIN {
        detach_hwnd(raw);
        return Ok(());
    }

    // During capsule width tween, never re-layout from full client size — that
    // flashes the expanded right edge before the L/R grow animation.
    if width_tween_active() && size_px.is_none() {
        return Ok(());
    }

    let needs_rebuild = !sessions().lock().contains_key(&raw);
    if needs_rebuild {
        detach_hwnd(raw);
        let session = build_session(hwnd, dark)?;
        match size_px {
            Some((w, h)) => layout_session_size(&session, hwnd, w, h, r, dark)?,
            None => layout_session(&session, hwnd, r, dark)?,
        }
        sessions().lock().insert(raw, session);
        return Ok(());
    }

    if let Some(session) = sessions().lock().get(&raw) {
        match size_px {
            Some((w, h)) => layout_session_size(session, hwnd, w, h, r, dark)?,
            None => layout_session(session, hwnd, r, dark)?,
        }
    }
    Ok(())
}

pub fn remember_glass_window(window: &WebviewWindow) {
    if let Ok(hwnd) = window.hwnd() {
        glass_wins().lock().insert(hwnd.0 as isize, window.clone());
    }
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
    if let Some(w) = glass_win_for(raw) {
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
