//! Missing callback fallback. Ask Explorer's own accessible tray button to act;
//! never guess an application's callback, change IsPromoted, or inject a hook.
use super::tray::{TrayClick, TrayIconInfo};
use parking_lot::Mutex;
use windows::core::{w, Interface};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation, IUIAutomation2, IUIAutomationElement, IUIAutomationElement3,
    IUIAutomationInvokePattern, TreeScope_Subtree, UIA_ButtonControlTypeId, UIA_InvokePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ChildWindowFromPointEx, FindWindowExW, GetAncestor, IsWindowVisible, PostMessageW,
    ShowWindowAsync, CWP_SKIPDISABLED, CWP_SKIPINVISIBLE, GA_ROOT, SW_HIDE, SW_SHOWNOACTIVATE,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP,
};

static INVOCATION: Mutex<()> = Mutex::new(());

struct ComGuard;
impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() }
    }
}

fn normalized(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

// A tooltip may gain a second line (status/message count). Never substring-match
// process names: "QQ" must not select "QQ Music", and duplicate names must fail.
fn matches_name(name: &str, tip: &str, process: &str) -> bool {
    let first_name = normalized(name.lines().next().unwrap_or_default());
    let first_tip = normalized(tip.lines().next().unwrap_or_default());
    let name = normalized(name);
    let tip = normalized(tip);
    let process = normalized(process);
    if name.is_empty() {
        return false;
    }
    if !tip.is_empty()
        && (name == tip || first_name == tip || first_tip == name || first_name == first_tip)
    {
        return true;
    }
    // Process stem only when it is the entire accessible name (never substring).
    !process.is_empty() && (name == process || first_name == process)
}

fn shell_tray() -> Result<HWND, String> {
    unsafe {
        let mut after = HWND::default();
        loop {
            let hwnd = FindWindowExW(HWND::default(), after, w!("Shell_TrayWnd"), None)
                .map_err(|_| "找不到 Windows 原生托盘".to_string())?;
            // Exclude the systray-util spy, which uses the same top-level class.
            if FindWindowExW(hwnd, HWND::default(), w!("TrayNotifyWnd"), None).is_ok() {
                return Ok(hwnd);
            }
            after = hwnd;
        }
    }
}

fn descendants(
    automation: &IUIAutomation,
    hwnd: HWND,
) -> Result<Vec<IUIAutomationElement>, String> {
    unsafe {
        let root = automation
            .ElementFromHandle(hwnd)
            .map_err(|e| e.to_string())?;
        let condition = automation
            .CreateTrueCondition()
            .map_err(|e| e.to_string())?;
        let all = root
            .FindAll(TreeScope_Subtree, &condition)
            .map_err(|e| e.to_string())?;
        let mut result = Vec::new();
        for i in 0..all.Length().map_err(|e| e.to_string())? {
            let element = all.GetElement(i).map_err(|e| e.to_string())?;
            if element.CurrentControlType().ok() == Some(UIA_ButtonControlTypeId) {
                result.push(element);
            }
        }
        Ok(result)
    }
}

fn find_icon(
    elements: &[IUIAutomationElement],
    info: &TrayIconInfo,
) -> Result<Option<IUIAutomationElement>, String> {
    let mut found = None;
    for el in elements {
        let name = unsafe { el.CurrentName() }
            .map(|s| s.to_string())
            .unwrap_or_default();
        if matches_name(&name, &info.tooltip, &info.process) {
            if found.is_some() {
                return Err(format!("原生托盘存在多个同名图标：{}", info.tooltip));
            }
            found = Some(el.clone());
        }
    }
    Ok(found)
}

fn shell_buttons(
    automation: &IUIAutomation,
    tray: HWND,
) -> Result<Vec<IUIAutomationElement>, String> {
    // Win11's XAML notification area can be a sibling of TrayNotifyWnd. Scope
    // by SystemTray classes so a same-named taskbar app button cannot be chosen.
    let modern: Vec<_> = descendants(automation, tray)?
        .into_iter()
        .filter(|el| unsafe {
            el.CurrentClassName()
                .map(|s| s.to_string().starts_with("SystemTray."))
                .unwrap_or(false)
        })
        .collect();
    if !modern.is_empty() {
        return Ok(modern);
    }
    let notify = unsafe { FindWindowExW(tray, HWND::default(), w!("TrayNotifyWnd"), None) }
        .map_err(|e| e.to_string())?;
    descendants(automation, notify)
}

fn is_overflow_chevron(el: &IUIAutomationElement) -> bool {
    unsafe {
        let id = el
            .CurrentAutomationId()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let class = el
            .CurrentClassName()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let name = el.CurrentName().map(|s| s.to_string()).unwrap_or_default();
        id.eq_ignore_ascii_case("NotifyIconOverflow")
            || id.eq_ignore_ascii_case("SystemTrayCorner")
            || (id == "SystemTrayIcon" && class.contains("SystemTray") && name.contains("隐藏"))
            || class.contains("Chevron")
            || name.contains("溢出")
            || name.to_ascii_lowercase().contains("overflow")
            || name.contains("显示隐藏的图标")
            || name.contains("显示隐藏图标")
            || name.contains("隐藏的图标")
            || name.contains("Show hidden")
            || name.contains("Hidden icons")
    }
}

fn open_overflow(automation: &IUIAutomation, tray: HWND) -> Result<(), String> {
    // Search the FULL tray tree — Win11 SystemTray.* app-icon lists often omit the chevron.
    let all = descendants(automation, tray)?;
    for el in &all {
        if !is_overflow_chevron(el) {
            continue;
        }
        unsafe {
            if let Ok(pattern) =
                el.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
            {
                return pattern
                    .Invoke()
                    .map_err(|e| format!("展开 Windows 托盘失败：{e}"));
            }
        }
    }
    // Fall back to the broader tray_uia chevron walk (enable registry chevron first).
    let _ = super::tray_registry::enable_chevron();
    match super::tray_uia::open_chevron_via_uia() {
        Ok(true) => Ok(()),
        Ok(false) => Err("找不到 Windows 托盘展开按钮".into()),
        Err(e) => Err(format!("找不到 Windows 托盘展开按钮：{e}")),
    }
}

/// Post to the actual Explorer input child, not the application's unknown HWND.
/// Used for double-click and providers without a context-menu pattern. No global
/// SendInput or cursor movement; the hit point comes from the matched UIA button.
fn post_mouse(root: HWND, el: &IUIAutomationElement, click: TrayClick) -> Result<(), String> {
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    unsafe {
        let rect = el.CurrentBoundingRectangle().map_err(|e| e.to_string())?;
        if rect.right <= rect.left || rect.bottom <= rect.top || !IsWindowVisible(root).as_bool() {
            return Err("原生托盘图标尚不可点击".into());
        }
        let screen = POINT {
            x: rect.left + (rect.right - rect.left) / 2,
            y: rect.top + (rect.bottom - rect.top) / 2,
        };
        let mut target = root;
        let mut point;
        loop {
            point = screen;
            if !ScreenToClient(target, &mut point).as_bool() {
                return Err("托盘坐标转换失败".into());
            }
            let child = ChildWindowFromPointEx(target, point, CWP_SKIPINVISIBLE | CWP_SKIPDISABLED);
            if child.0.is_null() || child == target {
                break;
            }
            target = child;
        }
        let pos = LPARAM((((point.y as u32 & 0xffff) << 16) | (point.x as u32 & 0xffff)) as isize);
        let messages: &[(u32, usize)] = match click {
            TrayClick::Left => &[(WM_LBUTTONDOWN, 1), (WM_LBUTTONUP, 0)],
            TrayClick::LeftDouble => &[
                (WM_LBUTTONDOWN, 1),
                (WM_LBUTTONUP, 0),
                (WM_LBUTTONDBLCLK, 1),
                (WM_LBUTTONUP, 0),
            ],
            TrayClick::Right => &[(WM_RBUTTONDOWN, 2), (WM_RBUTTONUP, 0)],
        };
        for &(message, flags) in messages {
            PostMessageW(target, message, WPARAM(flags), pos)
                .map_err(|e| format!("Windows 托盘点击失败：{e}"))?;
        }
        Ok(())
    }
}

fn activate(root: HWND, el: &IUIAutomationElement, click: TrayClick) -> Result<(), String> {
    unsafe {
        match click {
            TrayClick::Left => {
                if let Ok(pattern) =
                    el.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
                {
                    return pattern
                        .Invoke()
                        .map_err(|e| format!("原生托盘打开失败：{e}"));
                }
            }
            TrayClick::Right => {
                if let Ok(el3) = el.cast::<IUIAutomationElement3>() {
                    if el3.ShowContextMenu().is_ok() {
                        return Ok(());
                    }
                }
            }
            TrayClick::LeftDouble => {}
        }
    }
    post_mouse(root, el, click)
}

pub fn invoke(info: &TrayIconInfo, click: TrayClick) -> Result<(), String> {
    // Queue behind an in-flight shell click instead of rejecting the user.
    let _lock = INVOCATION.lock();
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| e.to_string())?;
    }
    let _com = ComGuard;
    let automation: IUIAutomation = unsafe {
        CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?
    };
    if let Ok(settings) = automation.cast::<IUIAutomation2>() {
        unsafe {
            settings
                .SetConnectionTimeout(500)
                .map_err(|e| e.to_string())?;
            settings
                .SetTransactionTimeout(700)
                .map_err(|e| e.to_string())?;
        }
    }
    let tray = shell_tray()?;
    let hold = super::status_menu::hold_taskbar_for_tray();
    // Give Explorer a beat to rebuild the notification-area tree after ShowWindow.
    std::thread::sleep(std::time::Duration::from_millis(80));
    let root = unsafe { GetAncestor(tray, GA_ROOT) };
    if !unsafe { IsWindowVisible(root).as_bool() } {
        unsafe {
            let _ = ShowWindowAsync(root, SW_SHOWNOACTIVATE);
        }
        std::thread::sleep(std::time::Duration::from_millis(80));
    }
    let result = invoke_in_shell(&automation, tray, info, click);
    if result.is_ok() {
        // Posted mouse events and app-owned menus need time to acquire focus.
        // Restoration still observes the latest user taskbar visibility setting.
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            drop(hold);
        });
    }
    result
}

fn invoke_in_shell(
    automation: &IUIAutomation,
    tray: HWND,
    info: &TrayIconInfo,
    click: TrayClick,
) -> Result<(), String> {
    let mut buttons = Vec::new();
    for _ in 0..12 {
        if !unsafe { IsWindowVisible(tray).as_bool() } {
            unsafe {
                let _ = ShowWindowAsync(tray, SW_SHOWNOACTIVATE);
            }
        }
        buttons = shell_buttons(automation, tray)?;
        if !buttons.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // A Hub pin does not change the icon's Windows area. Respect that area to
    // avoid selecting a visible same-named instance for a hidden icon.
    if info.area != "overflow" {
        if let Some(el) = find_icon(&buttons, info)? {
            return activate(tray, &el, click);
        }
    }

    let existing = super::tray_uia::get_tray_overflow_handle();
    let already_open = existing.is_some_and(|h| unsafe { IsWindowVisible(h).as_bool() });
    if !already_open {
        open_overflow(automation, tray)?;
    }
    let result = (|| {
        // Opening the flyout is asynchronous. Re-query instead of assuming its
        // HWND or accessibility tree exists immediately (also after Explorer restart).
        for _ in 0..12 {
            if let Some(overflow) = super::tray_uia::get_tray_overflow_handle() {
                let buttons = descendants(automation, overflow)?;
                if let Some(el) = find_icon(&buttons, info)? {
                    return activate(overflow, &el, click);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        Err(format!(
            "Windows 原生托盘中未找到“{}”，图标可能已退出或提示文字已改变",
            info.tooltip
        ))
    })();
    // On success Explorer owns dismissal: hiding immediately can cancel the menu.
    // Only clean up a failed flyout that this invocation opened.
    if result.is_err() && !already_open {
        if let Some(overflow) = super::tray_uia::get_tray_overflow_handle() {
            unsafe {
                let _ = ShowWindowAsync(overflow, SW_HIDE);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_tooltip_with_status_line() {
        assert!(matches_name(
            "Clash Verge\n已连接",
            "Clash Verge",
            "clash-verge"
        ));
        assert!(matches_name("微信", "微信\n新消息", "weixin"));
        assert!(matches_name(" PixPin ", "pixpin", ""));
    }
    #[test]
    fn never_uses_partial_or_empty_app_names() {
        assert!(!matches_name("QQ音乐", "QQ", "qq"));
        assert!(!matches_name("Clash Verge", "", "clash"));
        assert!(!matches_name("任意应用", "", ""));
        assert!(!matches_name("", "", ""));
    }

    #[test]
    fn host_flattened_tooltips_match_native_multiline_text() {
        assert!(matches_name(
            "Clash Verge\r\n已连接",
            "Clash Verge 已连接",
            ""
        ));
        assert!(!matches_name("Clash Verge\n断开", "Clash Verge 已连接", ""));
    }

    #[test]
    fn mouse_fallback_posts_the_requested_button_to_the_native_child() {
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, PeekMessageW, MSG, PM_REMOVE, WINDOW_EX_STYLE,
            WS_CHILD, WS_POPUP, WS_VISIBLE,
        };
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
            let _com = ComGuard;
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).unwrap();
            let root = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Tray dispatch test"),
                WS_POPUP | WS_VISIBLE,
                80,
                80,
                80,
                80,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            struct WindowGuard(HWND);
            impl Drop for WindowGuard {
                fn drop(&mut self) {
                    unsafe {
                        let _ = DestroyWindow(self.0);
                    }
                }
            }
            let _window = WindowGuard(root);
            let child = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Fixture"),
                WS_CHILD | WS_VISIBLE,
                10,
                10,
                40,
                40,
                root,
                None,
                None,
                None,
            )
            .unwrap();
            let el = automation.ElementFromHandle(child).unwrap();
            for (action, expected) in [
                (TrayClick::Left, vec![WM_LBUTTONDOWN, WM_LBUTTONUP]),
                (TrayClick::Right, vec![WM_RBUTTONDOWN, WM_RBUTTONUP]),
                (
                    TrayClick::LeftDouble,
                    vec![WM_LBUTTONDOWN, WM_LBUTTONUP, WM_LBUTTONDBLCLK, WM_LBUTTONUP],
                ),
            ] {
                post_mouse(root, &el, action).unwrap();
                let mut msg = MSG::default();
                let mut received = Vec::new();
                while PeekMessageW(&mut msg, child, WM_LBUTTONDOWN, WM_RBUTTONUP, PM_REMOVE)
                    .as_bool()
                {
                    assert_eq!(msg.hwnd, child);
                    assert_eq!(msg.lParam.0, 20 | (20 << 16));
                    received.push(msg.message);
                }
                assert_eq!(received, expected);
            }
        }
    }

    #[test]
    #[ignore = "read-only diagnostic requiring an interactive Windows Explorer session"]
    fn inspect_native_tray() {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
        }
        let _com = ComGuard;
        let automation: IUIAutomation =
            unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).unwrap() };
        let mut roots = vec![("taskbar", shell_tray().unwrap())];
        if let Some(overflow) = super::super::tray_uia::get_tray_overflow_handle() {
            roots.push(("overflow", overflow));
        }
        for (area, hwnd) in roots {
            let buttons = if area == "taskbar" {
                shell_buttons(&automation, hwnd)
            } else {
                descendants(&automation, hwnd)
            }
            .unwrap();
            println!(
                "{area}: {} native buttons, visible={}",
                buttons.len(),
                unsafe { IsWindowVisible(hwnd).as_bool() }
            );
            for button in buttons {
                unsafe {
                    println!(
                        "  name={:?} id={:?} class={:?} invoke={}",
                        button.CurrentName().unwrap_or_default().to_string(),
                        button.CurrentAutomationId().unwrap_or_default().to_string(),
                        button.CurrentClassName().unwrap_or_default().to_string(),
                        button
                            .GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
                            .is_ok()
                    );
                }
            }
        }
    }
}
