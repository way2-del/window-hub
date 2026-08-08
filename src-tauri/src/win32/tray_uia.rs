//! UI Automation helpers for the notification area (Win10 + Win11).
//!
//! **Retired from the tray hot path** (MyDockFinder parity): list + click now go
//! through the explorer `WH_CALLWNDPROC` hook and `SendNotifyMessage`. These
//! helpers remain available for diagnostics only — do not call demote/capture
//! from startup or click handlers (they hide the menubar via shell churn).

#![cfg(windows)]
#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};

use windows::core::s;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationInvokePattern,
    TreeScope_Children, TreeScope_Subtree, UIA_InvokePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowA, FindWindowExA, IsWindow, ShowWindow, SW_HIDE, SW_SHOWNA,
};

use super::tray_registry;

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
        let lib = windows::Win32::System::LibraryLoader::LoadLibraryA(s!("ntdll.dll"));
        let Ok(lib) = lib else {
            return 0;
        };
        let proc = windows::Win32::System::LibraryLoader::GetProcAddress(lib, s!("RtlGetVersion"));
        let Some(proc) = proc else {
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
        let _ = rtl(&mut info);
        info.dw_build_number
    }
}

pub fn is_windows_11() -> bool {
    os_build() >= 22000
}

pub fn get_tray_overflow_handle() -> Option<HWND> {
    unsafe {
        if is_windows_11() {
            FindWindowA(s!("TopLevelWindowForOverflowXamlIsland"), None)
                .ok()
                .or_else(|| FindWindowA(s!("NotifyIconOverflowWindow"), None).ok())
        } else {
            FindWindowA(s!("NotifyIconOverflowWindow"), None).ok()
        }
    }
}

pub fn get_tray_overflow_content_handle() -> Option<HWND> {
    let tray_overflow = get_tray_overflow_handle()?;
    unsafe {
        if is_windows_11() {
            FindWindowExA(
                tray_overflow,
                None,
                s!("Windows.UI.Composition.DesktopWindowContentBridge"),
                None,
            )
            .ok()
            .or_else(|| FindWindowExA(tray_overflow, None, s!("ToolbarWindow32"), None).ok())
        } else {
            FindWindowExA(tray_overflow, None, s!("ToolbarWindow32"), None).ok()
        }
    }
}

fn shell_tray_hwnd() -> Option<HWND> {
    // Never use bare FindWindow("Shell_TrayWnd") — our spy uses that class too.
    unsafe {
        let mut hwnd = FindWindowA(s!("Shell_TrayWnd"), None).ok()?;
        loop {
            if FindWindowExA(hwnd, None, s!("TrayNotifyWnd"), None).is_ok() {
                return Some(hwnd);
            }
            hwnd = FindWindowExA(HWND::default(), hwnd, s!("Shell_TrayWnd"), None).ok()?;
        }
    }
}

struct ComGuard;
impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

fn with_com<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| e.to_string())?;
    }
    let _guard = ComGuard;
    f()
}

fn create_automation() -> Result<IUIAutomation, String> {
    unsafe {
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())
    }
}

fn uia_children(
    root: &IUIAutomationElement,
    subtree: bool,
) -> Result<Vec<IUIAutomationElement>, String> {
    let automation = create_automation()?;
    let condition = unsafe { automation.CreateTrueCondition().map_err(|e| e.to_string())? };
    let scope = if subtree {
        TreeScope_Subtree
    } else {
        TreeScope_Children
    };
    let mut elements = Vec::new();
    unsafe {
        let arr = root.FindAll(scope, &condition).map_err(|e| e.to_string())?;
        let len = arr.Length().unwrap_or(0);
        for i in 0..len {
            if let Ok(el) = arr.GetElement(i) {
                elements.push(el);
            }
        }
    }
    Ok(elements)
}

fn is_chevron_element(el: &IUIAutomationElement) -> bool {
    unsafe {
        let id = el.CurrentAutomationId().map(|s| s.to_string()).unwrap_or_default();
        let class = el.CurrentClassName().map(|s| s.to_string()).unwrap_or_default();
        let name = el.CurrentName().map(|s| s.to_string()).unwrap_or_default();
        (id == "SystemTrayIcon" && class.contains("SystemTray"))
            || id.eq_ignore_ascii_case("NotifyIconOverflow")
            || id.eq_ignore_ascii_case("SystemTrayCorner")
            || class.contains("Chevron")
            || class.contains("NotifyIcon")
            || name.contains("溢出")
            || name.to_ascii_lowercase().contains("overflow")
            || name.contains("显示隐藏的图标")
            || name.contains("隐藏的图标")
            || name.contains("Show hidden")
            || name.contains("Hidden icons")
    }
}

/// Current overflow button names (UIA), in visual order.
pub fn scan_overflow_names() -> Vec<String> {
    let Some(content) = get_tray_overflow_content_handle() else {
        return Vec::new();
    };
    with_com(|| {
        let automation = create_automation()?;
        let element = unsafe {
            automation
                .ElementFromHandle(content)
                .map_err(|e| e.to_string())?
        };
        // Prefer direct children; if empty, fall back to subtree buttons/names.
        let mut children = uia_children(&element, false)?;
        if children.is_empty() {
            children = uia_children(&element, true)?;
        }
        let mut names = Vec::new();
        for child in children {
            let name =
                unsafe { child.CurrentName().map(|s| s.to_string()).unwrap_or_default() };
            let name = name.trim().to_string();
            if name.is_empty() {
                continue;
            }
            // Skip chrome / chevron chrome inside the flyout.
            if name.contains("溢出") || name.to_ascii_lowercase().contains("overflow") {
                continue;
            }
            names.push(name);
        }
        Ok(names)
    })
    .unwrap_or_default()
}

static TRAY_CREATION_ATTEMPTS: AtomicU32 = AtomicU32::new(0);

fn open_chevron_via_uia() -> Result<bool, String> {
    with_com(|| unsafe {
        let Some(tray_hwnd) = shell_tray_hwnd() else {
            return Err("real Shell_TrayWnd (explorer) not found".into());
        };
        if !IsWindow(tray_hwnd).as_bool() {
            return Err("Shell_TrayWnd invalid".into());
        }

        let automation = create_automation()?;
        let element = automation
            .ElementFromHandle(tray_hwnd)
            .map_err(|e| e.to_string())?;
        let children = uia_children(&element, true)?;

        for el in children {
            if !is_chevron_element(&el) {
                continue;
            }
            let Ok(invoker) =
                el.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
            else {
                continue;
            };
            let _ = invoker.Invoke();
            return Ok(true);
        }
        Ok(false)
    })
}

/// Demote → open overflow → scan names → close → restore IsPromoted.
/// This is the Seelen-style path that can see *all* overflow icons (not just GetRect hits).
pub fn capture_overflow_names() -> Result<Vec<String>, String> {
    if let Some(_) = get_tray_overflow_content_handle() {
        let names = scan_overflow_names();
        if !names.is_empty() {
            return Ok(names);
        }
    }

    let attempts = TRAY_CREATION_ATTEMPTS.fetch_add(1, Ordering::AcqRel) + 1;
    if attempts > 8 {
        return Err("maximum tray overflow creation attempts reached".into());
    }

    let _ = tray_registry::enable_chevron();
    let promoted_snap = tray_registry::snapshot_promoted();
    let _ = tray_registry::demote_all_to_overflow();
    std::thread::sleep(std::time::Duration::from_millis(450));

    let opened = open_chevron_via_uia().unwrap_or(false);
    if !opened {
        if let Some(overflow) = get_tray_overflow_handle() {
            unsafe {
                let _ = ShowWindow(overflow, SW_SHOWNA);
            }
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(280));

    let names = scan_overflow_names();
    eprintln!(
        "[tray] overflow capture: opened={opened} names={} handle={:?}",
        names.len(),
        get_tray_overflow_content_handle().map(|h| h.0 as isize)
    );

    // Close flyout if we opened it.
    if let Some(overflow) = get_tray_overflow_handle() {
        unsafe {
            let _ = ShowWindow(overflow, SW_HIDE);
        }
    } else {
        let _ = open_chevron_via_uia(); // toggle close best-effort
    }

    std::thread::sleep(std::time::Duration::from_millis(80));
    tray_registry::restore_promoted(&promoted_snap);

    if names.is_empty() {
        return Err("failed to capture tray overflow names".into());
    }
    Ok(names)
}

#[allow(dead_code)]
pub fn ensure_tray_overflow_creation() -> Result<(), String> {
    if get_tray_overflow_content_handle().is_some() {
        return Ok(());
    }
    let names = capture_overflow_names()?;
    if names.is_empty() {
        return Err("failed to create tray overflow".into());
    }
    Ok(())
}

/// Open overflow (without demoting every icon), Invoke child by visible name, hide.
pub fn invoke_overflow_by_name(name: &str, right_click: bool) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("empty overflow name".into());
    }
    let _ = tray_registry::enable_chevron();
    let _ = open_chevron_via_uia();
    std::thread::sleep(std::time::Duration::from_millis(180));

    let Some(overflow) = get_tray_overflow_handle() else {
        return Err("overflow window missing".into());
    };
    let Some(content) = get_tray_overflow_content_handle() else {
        return Err("overflow content missing".into());
    };

    let want = name.trim().to_ascii_lowercase();
    let result = with_com(|| {
        unsafe {
            let _ = ShowWindow(overflow, SW_SHOWNA);
        }
        std::thread::sleep(std::time::Duration::from_millis(40));

        let automation = create_automation()?;
        let element = unsafe {
            automation
                .ElementFromHandle(content)
                .map_err(|e| e.to_string())?
        };
        let mut children = uia_children(&element, false)?;
        if children.is_empty() {
            children = uia_children(&element, true)?;
        }

        let mut target = None;
        for child in &children {
            let n = unsafe { child.CurrentName().map(|s| s.to_string()).unwrap_or_default() };
            let n = n.trim().to_string();
            if n.is_empty() {
                continue;
            }
            let nl = n.to_ascii_lowercase();
            if nl == want || nl.contains(&want) || want.contains(&nl) {
                target = Some(child.clone());
                break;
            }
        }
        let Some(child) = target else {
            unsafe {
                let _ = ShowWindow(overflow, SW_HIDE);
            }
            return Err(format!("overflow child named `{name}` not found"));
        };

        unsafe {
            if right_click {
                use windows::core::Interface;
                use windows::Win32::UI::Accessibility::IUIAutomationElement3;
                if let Ok(el3) = child.cast::<IUIAutomationElement3>() {
                    let _ = el3.ShowContextMenu();
                } else if let Ok(invoker) = child
                    .GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
                {
                    let _ = invoker.Invoke();
                }
            } else if let Ok(invoker) = child
                .GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
            {
                let _ = invoker.Invoke();
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(60));
        unsafe {
            let _ = ShowWindow(overflow, SW_HIDE);
        }
        Ok(())
    });

    result
}

/// Open overflow flyout, Invoke the child at `overflow_index`, then hide.
/// Does **not** demote all icons (that was collapsing the menubar via shell churn).
pub fn invoke_overflow_child(overflow_index: usize, right_click: bool) -> Result<(), String> {
    let _ = tray_registry::enable_chevron();
    let _ = open_chevron_via_uia();
    std::thread::sleep(std::time::Duration::from_millis(180));

    let Some(overflow) = get_tray_overflow_handle() else {
        return Err("overflow window missing".into());
    };
    let Some(content) = get_tray_overflow_content_handle() else {
        return Err("overflow content missing".into());
    };

    with_com(|| {
        unsafe {
            let _ = ShowWindow(overflow, SW_SHOWNA);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));

        let automation = create_automation()?;
        let element = unsafe {
            automation
                .ElementFromHandle(content)
                .map_err(|e| e.to_string())?
        };
        let mut children = uia_children(&element, false)?;
        if children.is_empty() {
            children = uia_children(&element, true)?;
        }
        let children: Vec<_> = children
            .into_iter()
            .filter(|c| {
                let name = unsafe { c.CurrentName().map(|s| s.to_string()).unwrap_or_default() };
                let name = name.trim();
                !name.is_empty()
                    && !name.contains("溢出")
                    && !name.to_ascii_lowercase().contains("overflow")
            })
            .collect();
        let Some(child) = children.get(overflow_index) else {
            unsafe {
                let _ = ShowWindow(overflow, SW_HIDE);
            }
            return Err(format!("overflow child {overflow_index} out of range"));
        };

        unsafe {
            if right_click {
                use windows::core::Interface;
                use windows::Win32::UI::Accessibility::IUIAutomationElement3;
                if let Ok(el3) = child.cast::<IUIAutomationElement3>() {
                    let _ = el3.ShowContextMenu();
                } else if let Ok(invoker) = child
                    .GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
                {
                    let _ = invoker.Invoke();
                }
            } else if let Ok(invoker) = child
                .GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
            {
                let _ = invoker.Invoke();
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(80));
        unsafe {
            let _ = ShowWindow(overflow, SW_HIDE);
        }
        Ok(())
    })
}

/// Briefly show the overflow window so Win10 ToolbarWindow32 is populated.
#[allow(dead_code)]
pub fn poke_overflow_window() {
    if let Some(hwnd) = get_tray_overflow_handle() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNA);
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}
