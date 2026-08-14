//! Watch system shell drag (Explorer file drag) via SysDragImage window lifecycle.
//! Emits `system-drag-start` / `system-drag-end` so Host can reveal an in-window
//! staging catcher without creating a new HWND mid-drag (OLE would reject drops).

#[cfg(windows)]
mod win {
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;

    use tauri::{AppHandle, Emitter};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Accessibility::{
        SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK, WINEVENTPROC,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, FindWindowW, GetClassNameW, GetMessageW, IsWindow, TranslateMessage,
        EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, MSG, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT,
    };

    const VK_LBUTTON: i32 = 0x01;

    static ACTIVE: AtomicBool = AtomicBool::new(false);
    static DRAG_HWND: AtomicIsize = AtomicIsize::new(0);
    static HOOK_CREATE: AtomicIsize = AtomicIsize::new(0);
    static HOOK_DESTROY: AtomicIsize = AtomicIsize::new(0);
    static APP: Mutex<Option<AppHandle>> = Mutex::new(None);

    fn class_name(hwnd: HWND) -> String {
        let mut buf = [0u16; 64];
        let n = unsafe { GetClassNameW(hwnd, &mut buf) };
        if n <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..n as usize])
    }

    fn is_shell_drag_image(hwnd: HWND) -> bool {
        if hwnd.0.is_null() {
            return false;
        }
        class_name(hwnd) == "SysDragImage"
    }

    fn find_shell_drag_image() -> Option<HWND> {
        let hwnd = unsafe { FindWindowW(windows::core::w!("SysDragImage"), None) }.ok()?;
        if hwnd.0.is_null() {
            None
        } else {
            Some(hwnd)
        }
    }

    fn drag_hwnd_alive() -> bool {
        let raw = DRAG_HWND.load(Ordering::SeqCst);
        if raw != 0 {
            let hwnd = HWND(raw as *mut _);
            if unsafe { IsWindow(hwnd).as_bool() } && is_shell_drag_image(hwnd) {
                return true;
            }
        }
        find_shell_drag_image().is_some()
    }

    fn lbutton_down() -> bool {
        unsafe { GetAsyncKeyState(VK_LBUTTON) as u16 & 0x8000 != 0 }
    }

    fn emit_start() {
        if ACTIVE.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Ok(guard) = APP.lock() {
            if let Some(app) = guard.as_ref() {
                let _ = app.emit("system-drag-start", ());
            }
        }
        // Escape cancel often skips DESTROY; poll until image gone + button up.
        std::thread::spawn(|| {
            loop {
                std::thread::sleep(Duration::from_millis(90));
                if !ACTIVE.load(Ordering::SeqCst) {
                    break;
                }
                if !drag_hwnd_alive() && !lbutton_down() {
                    emit_end();
                    break;
                }
            }
        });
    }

    fn emit_end() {
        if !ACTIVE.swap(false, Ordering::SeqCst) {
            return;
        }
        DRAG_HWND.store(0, Ordering::SeqCst);
        if let Ok(guard) = APP.lock() {
            if let Some(app) = guard.as_ref() {
                let _ = app.emit("system-drag-end", ());
            }
        }
    }

    unsafe extern "system" fn on_win_event(
        _hook: HWINEVENTHOOK,
        event: u32,
        hwnd: HWND,
        id_object: i32,
        _id_child: i32,
        _thread: u32,
        _time: u32,
    ) {
        if id_object != OBJID_WINDOW.0 {
            return;
        }
        if !is_shell_drag_image(hwnd) {
            return;
        }
        match event {
            EVENT_OBJECT_CREATE => {
                DRAG_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
                if lbutton_down() {
                    emit_start();
                }
            }
            EVENT_OBJECT_DESTROY => {
                let raw = hwnd.0 as isize;
                if DRAG_HWND.load(Ordering::SeqCst) == raw {
                    DRAG_HWND.store(0, Ordering::SeqCst);
                }
                if !drag_hwnd_alive() {
                    emit_end();
                }
            }
            _ => {}
        }
    }

    pub fn start(app: AppHandle) {
        {
            let mut g = APP.lock().unwrap_or_else(|e| e.into_inner());
            *g = Some(app);
        }
        let _ = std::thread::Builder::new()
            .name("wh-drag-watch".into())
            .spawn(|| {
                let proc: WINEVENTPROC = Some(on_win_event);
                let create = unsafe {
                    SetWinEventHook(
                        EVENT_OBJECT_CREATE,
                        EVENT_OBJECT_CREATE,
                        None,
                        proc,
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT,
                    )
                };
                let destroy = unsafe {
                    SetWinEventHook(
                        EVENT_OBJECT_DESTROY,
                        EVENT_OBJECT_DESTROY,
                        None,
                        proc,
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT,
                    )
                };
                if create.is_invalid() || destroy.is_invalid() {
                    eprintln!("[drag_watch] SetWinEventHook failed");
                    return;
                }
                HOOK_CREATE.store(create.0 as isize, Ordering::SeqCst);
                HOOK_DESTROY.store(destroy.0 as isize, Ordering::SeqCst);

                let mut msg = MSG::default();
                while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                    unsafe {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }

                let c = HOOK_CREATE.swap(0, Ordering::SeqCst);
                if c != 0 {
                    let _ = unsafe { UnhookWinEvent(HWINEVENTHOOK(c as *mut _)) };
                }
                let d = HOOK_DESTROY.swap(0, Ordering::SeqCst);
                if d != 0 {
                    let _ = unsafe { UnhookWinEvent(HWINEVENTHOOK(d as *mut _)) };
                }
            });
    }
}

#[cfg(windows)]
pub use win::start;

#[cfg(not(windows))]
pub fn start(_app: tauri::AppHandle) {}
