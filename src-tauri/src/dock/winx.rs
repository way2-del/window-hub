//! Win+X-style power-user actions for the dock Start tile context menu.

use super::launch::{shell_open, shell_runas};

fn windows_dir() -> std::path::PathBuf {
    std::env::var_os("WINDIR")
        .or_else(|| std::env::var_os("SystemRoot"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"))
}

fn system32(name: &str) -> String {
    windows_dir()
        .join("System32")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn open_msc(name: &str) -> Result<(), String> {
    shell_open(&system32(name), None)
}

fn open_cpl(name: &str) -> Result<(), String> {
    shell_open(&system32("control.exe"), Some(name)).or_else(|_| shell_open(name, None))
}

fn send_win_chord(vk: u16) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            keybd_event, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VK_LWIN,
        };
        unsafe {
            keybd_event(VK_LWIN.0 as u8, 0, KEYEVENTF_EXTENDEDKEY, 0);
            keybd_event(vk as u8, 0, Default::default(), 0);
            keybd_event(vk as u8, 0, KEYEVENTF_KEYUP, 0);
            keybd_event(
                VK_LWIN.0 as u8,
                0,
                KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP,
                0,
            );
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = vk;
        Err("仅支持 Windows".into())
    }
}

fn resolve_terminal() -> String {
    // Prefer Windows Terminal from PATH / Apps folder symlink.
    if let Ok(path) = which_wt() {
        return path;
    }
    let local = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let candidate = local
        .join("Microsoft")
        .join("WindowsApps")
        .join("wt.exe");
    if candidate.is_file() {
        return candidate.to_string_lossy().into_owned();
    }
    system32("WindowsPowerShell\\v1.0\\powershell.exe")
}

fn which_wt() -> Result<String, ()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let out = std::process::Command::new("where.exe")
            .arg("wt.exe")
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|_| ())?;
        if !out.status.success() {
            return Err(());
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.lines().next().unwrap_or("").trim();
        if line.is_empty() {
            return Err(());
        }
        Ok(line.to_string())
    }
    #[cfg(not(windows))]
    {
        Err(())
    }
}

fn open_terminal(as_admin: bool) -> Result<(), String> {
    let path = resolve_terminal();
    if as_admin {
        shell_runas(&path, None)
    } else {
        shell_open(&path, None)
    }
}

fn run_shutdown_flag(flag: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new(system32("shutdown.exe"))
            .args([flag, "/t", "0"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("shutdown failed: {e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = flag;
        Err("仅支持 Windows".into())
    }
}

fn power_sleep(hibernate: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        // Prefer rundll32 powrprof — avoids linking PowrProf when unavailable.
        let args = if hibernate {
            "PowrProf.dll,SetSuspendState 1,1,0"
        } else {
            "PowrProf.dll,SetSuspendState 0,1,0"
        };
        shell_open(&system32("rundll32.exe"), Some(args))
    }
    #[cfg(not(windows))]
    {
        let _ = hibernate;
        Err("仅支持 Windows".into())
    }
}

fn power_sign_out() -> Result<(), String> {
    // /l = log off current session
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new(system32("shutdown.exe"))
            .args(["/l"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("logoff failed: {e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err("仅支持 Windows".into())
    }
}

/// Execute a Win+X-style action id from the Start dock tile menu.
pub fn run_action(action: &str) -> Result<(), String> {
    let id = action.trim().to_ascii_lowercase();
    match id.as_str() {
        "apps" | "installed-apps" => shell_open("ms-settings:appsfeatures", None),
        "mobility" | "mobility-center" => shell_open(&system32("mblctr.exe"), None),
        "power" | "power-options" => open_cpl("powercfg.cpl")
            .or_else(|_| shell_open("ms-settings:powersleep", None)),
        "eventvwr" | "event-viewer" => open_msc("eventvwr.msc"),
        "system" => shell_open("ms-settings:about", None)
            .or_else(|_| open_cpl("sysdm.cpl")),
        "devmgmt" | "device-manager" => open_msc("devmgmt.msc"),
        "network" | "network-connections" => open_cpl("ncpa.cpl"),
        "diskmgmt" | "disk-management" => open_msc("diskmgmt.msc"),
        "compmgmt" | "computer-management" => open_msc("compmgmt.msc"),
        "terminal" => open_terminal(false),
        "terminal-admin" => open_terminal(true),
        "taskmgr" | "task-manager" => shell_open(&system32("taskmgr.exe"), None),
        "settings" => shell_open("ms-settings:", None),
        "explorer" | "file-explorer" => shell_open(&windows_dir().join("explorer.exe").to_string_lossy(), None),
        "search" => send_win_chord(0x53), // VK_S
        "run" => send_win_chord(0x52),    // VK_R
        "desktop" => crate::win32::status_menu::show_desktop(),
        "sign-out" | "logoff" => power_sign_out(),
        "sleep" => power_sleep(false),
        "hibernate" => power_sleep(true),
        "shutdown" => run_shutdown_flag("/s"),
        "restart" | "reboot" => run_shutdown_flag("/r"),
        other => Err(format!("未知 Win+X 动作: {other}")),
    }
}
