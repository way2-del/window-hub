//! WebView2 camera permission helpers.
//!
//! Once the user clicks Deny on a getUserMedia prompt, WebView2 persists DENY and
//! never re-prompts. Reset to ALLOW via ICoreWebView2Profile4 so the mirror plugin
//! can recover without wiping EBWebView.

#![cfg(windows)]

use std::sync::OnceLock;
use std::time::Duration;

use tauri::{Manager, WebviewWindow};
use webview2_com::{
    Microsoft::Web::WebView2::Win32::{
        ICoreWebView2PermissionRequestedEventArgs, ICoreWebView2Profile4, ICoreWebView2_13,
        COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_CAMERA,
        COREWEBVIEW2_PERMISSION_STATE_ALLOW,
    },
    PermissionRequestedEventHandler, SetPermissionStateCompletedHandler,
};
// wry / webview2-com bind to windows-core 0.61 — not the app's windows 0.58.
use windows_core_wv::{Interface, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

fn wide_nul(s: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn origin_from_window(win: &WebviewWindow) -> Result<String, String> {
    let u = win.url().map_err(|e| format!("webview url: {e}"))?;
    let scheme = u.scheme();
    let host = u
        .host_str()
        .ok_or_else(|| format!("webview origin has no host: {u}"))?;
    Ok(match u.port() {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    })
}

/// Auto-allow camera PermissionRequested so WebView2 never stores a sticky DENY.
/// OS privacy settings still gate real device access.
pub fn install_camera_auto_allow(win: &WebviewWindow) -> Result<(), String> {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    if INSTALLED.get().is_some() {
        return Ok(());
    }
    win.with_webview(|platform| {
        unsafe {
            let Ok(core) = platform.controller().CoreWebView2() else {
                return;
            };
            let mut token = 0i64;
            let handler = PermissionRequestedEventHandler::create(Box::new(
                move |_sender, args: Option<ICoreWebView2PermissionRequestedEventArgs>| {
                    if let Some(args) = args {
                        let mut kind = COREWEBVIEW2_PERMISSION_KIND(0);
                        args.PermissionKind(&mut kind)?;
                        if kind == COREWEBVIEW2_PERMISSION_KIND_CAMERA {
                            args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW)?;
                        }
                    }
                    Ok(())
                },
            ));
            let _ = core.add_PermissionRequested(&handler, &mut token);
        }
    })
    .map_err(|e| format!("with_webview: {e}"))?;
    let _ = INSTALLED.set(());
    Ok(())
}

/// Persist ALLOW for camera on the main webview origin (clears a prior DENY).
pub fn allow_camera_permission(win: &WebviewWindow) -> Result<(), String> {
    let origin = origin_from_window(win)?;
    let origin_wide = wide_nul(&origin);
    win.with_webview(move |platform| {
        let result = unsafe {
            (|| -> Result<(), String> {
                let core = platform
                    .controller()
                    .CoreWebView2()
                    .map_err(|e| format!("CoreWebView2: {e}"))?;
                let core13: ICoreWebView2_13 = core
                    .cast()
                    .map_err(|e| format!("ICoreWebView2_13: {e}"))?;
                let profile = core13
                    .Profile()
                    .map_err(|e| format!("Profile: {e}"))?;
                let profile4: ICoreWebView2Profile4 = profile
                    .cast()
                    .map_err(|e| format!("ICoreWebView2Profile4: {e}"))?;
                let origin_pcw = PCWSTR(origin_wide.as_ptr());
                SetPermissionStateCompletedHandler::wait_for_async_operation(
                    Box::new(move |handler| {
                        profile4
                            .SetPermissionState(
                                COREWEBVIEW2_PERMISSION_KIND_CAMERA,
                                origin_pcw,
                                COREWEBVIEW2_PERMISSION_STATE_ALLOW,
                                &handler,
                            )
                            .map_err(|e| webview2_com::Error::WindowsError(e))?;
                        Ok(())
                    }),
                    Box::new(|result| {
                        result?;
                        Ok(())
                    }),
                )
                .map_err(|e| format!("SetPermissionState: {e}"))?;
                Ok(())
            })()
        };
        if let Err(e) = result {
            eprintln!("[webview_camera] allow_camera_permission failed: {e}");
        }
    })
    .map_err(|e| format!("with_webview: {e}"))?;
    std::thread::sleep(Duration::from_millis(50));
    Ok(())
}

pub fn open_windows_camera_privacy() -> Result<(), String> {
    use windows::core::PCWSTR as ShellPCWSTR;
    let target = wide_nul("ms-settings:privacy-webcam");
    let op = wide_nul("open");
    let rc = unsafe {
        ShellExecuteW(
            HWND::default(),
            ShellPCWSTR(op.as_ptr()),
            ShellPCWSTR(target.as_ptr()),
            ShellPCWSTR::null(),
            ShellPCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if (rc.0 as isize) <= 32 {
        return Err(format!("ShellExecute failed ({})", rc.0 as isize));
    }
    Ok(())
}

pub fn main_webview(app: &tauri::AppHandle) -> Result<WebviewWindow, String> {
    app.get_webview_window("main")
        .ok_or_else(|| "main webview missing".into())
}
