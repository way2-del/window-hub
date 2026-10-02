//! Autostart Windows Service: launches the GUI into an interactive user session.
//! Install/remove is driven from `app_launch.rs`; entry flag: `--autostart-svc`.

#![cfg(windows)]

use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{
    DuplicateTokenEx, SecurityImpersonation, TOKEN_ALL_ACCESS, TokenPrimary,
};
use windows::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows::Win32::System::RemoteDesktop::{
    WTSEnumerateSessionsW, WTSFreeMemory, WTSGetActiveConsoleSessionId, WTSQueryUserToken,
    WTS_CONNECTSTATE_CLASS, WTS_CURRENT_SERVER_HANDLE, WTS_SESSION_INFOW,
};
use windows::Win32::System::Services::{
    RegisterServiceCtrlHandlerW, SetServiceStatus, StartServiceCtrlDispatcherW, SERVICE_ACCEPT_STOP,
    SERVICE_CONTROL_STOP, SERVICE_RUNNING, SERVICE_START_PENDING, SERVICE_STATUS,
    SERVICE_STATUS_CURRENT_STATE, SERVICE_STATUS_HANDLE, SERVICE_STOPPED, SERVICE_STOP_PENDING,
    SERVICE_TABLE_ENTRYW, SERVICE_WIN32_OWN_PROCESS,
};
use windows::Win32::System::Threading::{
    CreateProcessAsUserW, GetCurrentProcessId, CREATE_UNICODE_ENVIRONMENT, NORMAL_PRIORITY_CLASS,
    PROCESS_INFORMATION, STARTUPINFOW,
};

pub const SERVICE_NAME: &str = "WindowHubAutoStart";
pub const SERVICE_DISPLAY: &str = "Window Hub Auto Start";
/// Manual-reset Global event: set by GUI “退出 window-hub” (`exit_app`), cleared on
/// service start, new interactive login, manual GUI start, or restart.
/// Keeps the SCM worker from treating intentional quit as a crash to relaunch
/// *within the same login*; Fast Startup / logoff must not leave this sticky.
pub const USER_QUIT_EVENT: &str = "Global\\com.xushi.window-hub.user-quit.v2";
const LAUNCH_RETRY_SECS: u64 = 3;
/// After CreateProcessAsUser, wait this long before treating the GUI as "up".
const LAUNCH_VERIFY_SECS: u64 = 2;

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
static STATUS_HANDLE: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
/// Service holds this so the named event survives across GUI process lifetimes.
static QUIT_EVENT: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn status_handle() -> SERVICE_STATUS_HANDLE {
    SERVICE_STATUS_HANDLE(STATUS_HANDLE.load(Ordering::SeqCst) as *mut _)
}

fn svc_log(msg: &str) {
    let line = format!(
        "[{}] {}\n",
        chrono_like_now(),
        msg
    );
    eprintln!("[autostart-svc] {msg}");
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }
}

fn chrono_like_now() -> String {
    // Avoid extra crate: local rough stamp via system time.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
}

fn log_path() -> PathBuf {
    let mut p = std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    p.push("WindowHub");
    p.push("autostart-svc.log");
    p
}

fn service_exe_path() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("window-hub.exe"))
}

fn quit_event_handle() -> Option<HANDLE> {
    let raw = QUIT_EVENT.load(Ordering::SeqCst);
    if raw == 0 {
        None
    } else {
        Some(HANDLE(raw as _))
    }
}

/// SDDL: Authenticated Users can signal / wait / reset the Global event.
/// Without this, Session-0 (SYSTEM) creates the event and the interactive GUI
/// gets ACCESS_DENIED on SetEvent — service then treats every quit as a crash.
fn quit_event_security_attributes() -> Option<(
    windows::Win32::Security::SECURITY_ATTRIBUTES,
    windows::Win32::Security::PSECURITY_DESCRIPTOR,
)> {
    use windows::core::PCWSTR;
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

    // GA = generic all for Authenticated Users (AU).
    let sddl = wide("D:(A;;GA;;;AU)");
    let mut sd = PSECURITY_DESCRIPTOR::default();
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )
    };
    if ok.is_err() || sd.is_invalid() {
        return None;
    }
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: false.into(),
    };
    Some((sa, sd))
}

fn create_quit_event_shared(initial_signaled: bool) -> Result<HANDLE, windows::core::Error> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::System::Threading::CreateEventW;

    let name = wide(USER_QUIT_EVENT);
    if let Some((sa, sd)) = quit_event_security_attributes() {
        let result = unsafe {
            CreateEventW(
                Some(&sa),
                true,
                initial_signaled,
                PCWSTR(name.as_ptr()),
            )
        };
        unsafe {
            let _ = LocalFree(HLOCAL(sd.0 as _));
        }
        return result;
    }
    unsafe { CreateEventW(None, true, initial_signaled, PCWSTR(name.as_ptr())) }
}

/// Create (or open) the Global quit event and keep a handle in `QUIT_EVENT`.
fn ensure_quit_event_held() -> Option<HANDLE> {
    if let Some(h) = quit_event_handle() {
        return Some(h);
    }
    match create_quit_event_shared(false) {
        Ok(h) => {
            QUIT_EVENT.store(h.0 as isize, Ordering::SeqCst);
            Some(h)
        }
        Err(e) => {
            svc_log(&format!("CreateEvent user-quit: {e}"));
            None
        }
    }
}

fn user_quit_signaled() -> bool {
    use windows::Win32::Foundation::WAIT_OBJECT_0;
    use windows::Win32::System::Threading::WaitForSingleObject;
    let Some(h) = quit_event_handle().or_else(ensure_quit_event_held) else {
        return false;
    };
    unsafe { WaitForSingleObject(h, 0) == WAIT_OBJECT_0 }
}

/// GUI “退出 window-hub”: tell the autostart service not to relaunch this session.
pub fn signal_user_quit() {
    use windows::Win32::System::Threading::{OpenEventW, SetEvent, EVENT_MODIFY_STATE};
    let name = wide(USER_QUIT_EVENT);
    unsafe {
        let handle = OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(name.as_ptr()))
            .or_else(|_| create_quit_event_shared(false));
        match handle {
            Ok(h) => {
                if SetEvent(h).is_err() {
                    eprintln!("[autostart] SetEvent user-quit failed");
                }
                if QUIT_EVENT.load(Ordering::SeqCst) == 0 {
                    let _ = CloseHandle(h);
                }
            }
            Err(e) => {
                eprintln!("[autostart] Open/Create user-quit event failed: {e}");
            }
        }
    }
}

/// Allow autostart relaunch again (GUI start, restart, new login, or fresh service boot).
pub fn clear_user_quit() {
    use windows::Win32::System::Threading::{OpenEventW, ResetEvent, EVENT_MODIFY_STATE};
    let name = wide(USER_QUIT_EVENT);
    unsafe {
        let handle = OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(name.as_ptr()))
            .or_else(|_| create_quit_event_shared(false));
        match handle {
            Ok(h) => {
                let _ = ResetEvent(h);
                if QUIT_EVENT.load(Ordering::SeqCst) == 0 {
                    let _ = CloseHandle(h);
                }
            }
            Err(e) => {
                eprintln!("[autostart] clear user-quit failed: {e}");
            }
        }
    }
}

unsafe fn set_status(state: SERVICE_STATUS_CURRENT_STATE, accept_stop: bool, wait_hint_ms: u32) {
    let handle = status_handle();
    if handle.0.is_null() {
        return;
    }
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: if accept_stop {
            SERVICE_ACCEPT_STOP
        } else {
            Default::default()
        },
        dwWin32ExitCode: 0,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: wait_hint_ms,
    };
    let _ = SetServiceStatus(handle, &status);
}

unsafe extern "system" fn service_ctrl(ctrl: u32) {
    if ctrl == SERVICE_CONTROL_STOP {
        STOP_REQUESTED.store(true, Ordering::SeqCst);
        set_status(SERVICE_STOP_PENDING, false, 3000);
    }
}

/// Prefer an active/connected session; fall back to console session id.
fn resolve_interactive_session() -> Option<u32> {
    unsafe {
        let mut info: *mut WTS_SESSION_INFOW = std::ptr::null_mut();
        let mut count = 0u32;
        if WTSEnumerateSessionsW(WTS_CURRENT_SERVER_HANDLE, 0, 1, &mut info, &mut count).is_ok()
            && !info.is_null()
        {
            let sessions = std::slice::from_raw_parts(info, count as usize);
            // Prefer Active, then Connected.
            let mut pick = None;
            for s in sessions {
                if s.State == WTS_CONNECTSTATE_CLASS(0) {
                    // WTSActive = 0
                    pick = Some(s.SessionId);
                    break;
                }
            }
            if pick.is_none() {
                for s in sessions {
                    // WTSConnected = 1
                    if s.State == WTS_CONNECTSTATE_CLASS(1) {
                        pick = Some(s.SessionId);
                        break;
                    }
                }
            }
            WTSFreeMemory(info as *mut _);
            if let Some(id) = pick {
                if id != 0 {
                    return Some(id);
                }
            }
        }
    }
    let console = unsafe { WTSGetActiveConsoleSessionId() };
    if console == 0xFFFF_FFFF || console == 0 {
        None
    } else {
        Some(console)
    }
}

fn explorer_ready(session: u32) -> bool {
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;

    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return false;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut ok = Process32FirstW(snap, &mut entry).is_ok();
        let mut found = false;
        while ok {
            let name = String::from_utf16_lossy(
                &entry.szExeFile[..entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len())],
            )
            .to_ascii_lowercase();
            if name == "explorer.exe" {
                let mut sid = 0u32;
                if ProcessIdToSessionId(entry.th32ProcessID, &mut sid).is_ok() && sid == session {
                    // Confirm image exists (process still alive).
                    if let Ok(proc) =
                        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, entry.th32ProcessID)
                    {
                        let mut buf = [0u16; 8];
                        let mut size = buf.len() as u32;
                        let _ = QueryFullProcessImageNameW(
                            proc,
                            PROCESS_NAME_WIN32,
                            PWSTR(buf.as_mut_ptr()),
                            &mut size,
                        );
                        let _ = CloseHandle(proc);
                    }
                    found = true;
                    break;
                }
            }
            ok = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
        found
    }
}

fn launch_gui_in_session(session: u32) -> Result<HANDLE, String> {
    unsafe {
        let mut user_token = HANDLE::default();
        WTSQueryUserToken(session, &mut user_token)
            .map_err(|e| format!("WTSQueryUserToken({session}): {e}"))?;

        let mut primary = HANDLE::default();
        let dup = DuplicateTokenEx(
            user_token,
            TOKEN_ALL_ACCESS,
            None,
            SecurityImpersonation,
            TokenPrimary,
            &mut primary,
        );
        let _ = CloseHandle(user_token);
        dup.map_err(|e| format!("DuplicateTokenEx: {e}"))?;

        let mut env = std::ptr::null_mut();
        if CreateEnvironmentBlock(&mut env, primary, false).is_err() {
            let _ = CloseHandle(primary);
            return Err("CreateEnvironmentBlock 失败".into());
        }

        let exe = service_exe_path();
        let cwd = exe
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let mut cmd = wide(&format!("\"{}\"", exe.to_string_lossy()));
        let mut desktop = wide("winsta0\\default");
        let dir = wide(&cwd.to_string_lossy());
        let si = STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            lpDesktop: PWSTR(desktop.as_mut_ptr()),
            ..Default::default()
        };
        let mut pi = PROCESS_INFORMATION::default();
        let created = CreateProcessAsUserW(
            primary,
            None,
            PWSTR(cmd.as_mut_ptr()),
            None,
            None,
            false,
            CREATE_UNICODE_ENVIRONMENT | NORMAL_PRIORITY_CLASS,
            Some(env),
            PCWSTR(dir.as_ptr()),
            &si,
            &mut pi,
        );
        let _ = DestroyEnvironmentBlock(env);
        let _ = CloseHandle(primary);
        let _ = desktop;
        let _ = dir;
        if created.is_err() {
            return Err(format!("CreateProcessAsUser 失败: {created:?}"));
        }
        let pid = pi.dwProcessId;
        let _ = CloseHandle(pi.hThread);
        // Caller owns hProcess — used to avoid double-launch while child lives.
        svc_log(&format!(
            "CreateProcessAsUser ok pid={pid} session={session} exe={}",
            exe.display()
        ));
        Ok(pi.hProcess)
    }
}

fn sleep_interruptible(total_ms: u64) {
    let steps = total_ms / 100;
    for _ in 0..steps {
        if STOP_REQUESTED.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn process_alive(handle: HANDLE) -> bool {
    use windows::Win32::Foundation::WAIT_TIMEOUT;
    use windows::Win32::System::Threading::WaitForSingleObject;
    unsafe { WaitForSingleObject(handle, 0) == WAIT_TIMEOUT }
}

fn service_worker() {
    unsafe {
        set_status(SERVICE_RUNNING, true, 0);
    }
    svc_log(&format!(
        "service worker start pid={} exe={}",
        unsafe { GetCurrentProcessId() },
        service_exe_path().display()
    ));

    // Hold the named event for the service lifetime; clear so login/boot can launch.
    let _ = ensure_quit_event_held();
    clear_user_quit();

    let mut child: Option<HANDLE> = None;
    let mut last_err: Option<String> = None;
    let mut shell_settled = false;
    // Track interactive session so Fast Startup / logoff→logon can relaunch.
    // Under hybrid shutdown the SCM worker often keeps running; a stale
    // user-quit from the previous session must not block the next login.
    let mut last_session: Option<u32> = None;

    while !STOP_REQUESTED.load(Ordering::SeqCst) {
        // Drop dead child handle.
        if let Some(h) = child {
            if !process_alive(h) {
                unsafe {
                    let _ = CloseHandle(h);
                }
                child = None;
                svc_log("previous GUI process exited");
            }
        }

        // GUI holds Global UI mutex, or our child is still alive → do not relaunch.
        if crate::win32::single_instance::any_gui_instance_running() {
            if last_err.take().is_some() {
                svc_log("GUI mutex held (UI running)");
            }
            // Keep session tracking warm while UI is up.
            if let Some(sid) = resolve_interactive_session() {
                last_session = Some(sid);
            }
            sleep_interruptible(LAUNCH_RETRY_SECS * 1000);
            continue;
        }
        if child.is_some() {
            // Child alive but mutex not yet created (still starting) — wait.
            sleep_interruptible(500);
            continue;
        }

        // Session transitions must be observed *before* the user-quit gate:
        // otherwise a quit from the previous login (or EndSession teardown)
        // permanently blocks relaunch across Fast Startup.
        let session_now = resolve_interactive_session();
        match (last_session, session_now) {
            (_, None) => {
                if last_session.is_some() {
                    svc_log("interactive session ended");
                    last_err = None;
                }
                last_session = None;
                shell_settled = false;
            }
            (prev, Some(sid)) if prev != Some(sid) => {
                clear_user_quit();
                svc_log(&format!(
                    "session {sid}: new interactive session — cleared user-quit for login relaunch"
                ));
                last_session = Some(sid);
                shell_settled = false;
                last_err = None;
            }
            _ => {}
        }

        // User chose “退出 window-hub” — stay down for this login only.
        // New session (above) or manual GUI start clears the event.
        if user_quit_signaled() {
            if last_err.as_deref() != Some("user-quit") {
                svc_log("user quit signaled — not relaunching until next login / manual start");
                last_err = Some("user-quit".into());
            }
            sleep_interruptible(LAUNCH_RETRY_SECS * 1000);
            continue;
        }

        let Some(session) = session_now else {
            if last_err.as_deref() != Some("waiting-session") {
                svc_log("waiting for interactive session…");
                last_err = Some("waiting-session".into());
            }
            sleep_interruptible(LAUNCH_RETRY_SECS * 1000);
            continue;
        };

        if !explorer_ready(session) {
            if last_err.as_deref() != Some("waiting-explorer") {
                svc_log(&format!("session {session}: waiting for explorer.exe…"));
                last_err = Some("waiting-explorer".into());
            }
            sleep_interruptible(LAUNCH_RETRY_SECS * 1000);
            continue;
        }

        // explorer.exe can exist before Shell_TrayWnd / TrayNotifyWnd is live.
        // Settle once per session so the GUI hook does not race an empty tray.
        if !shell_settled {
            shell_settled = true;
            svc_log(&format!(
                "session {session}: explorer up — settling shell tray before GUI launch"
            ));
            sleep_interruptible(2500);
        }

        match launch_gui_in_session(session) {
            Ok(h) => {
                child = Some(h);
                // Give the GUI time to take the Global mutex.
                sleep_interruptible(LAUNCH_VERIFY_SECS * 1000);
                if crate::win32::single_instance::any_gui_instance_running() {
                    svc_log("GUI verified (mutex acquired)");
                    last_err = None;
                } else if child.map(process_alive).unwrap_or(false) {
                    svc_log("GUI process still starting (no mutex yet)");
                    last_err = None;
                } else {
                    if let Some(h) = child.take() {
                        unsafe {
                            let _ = CloseHandle(h);
                        }
                    }
                    svc_log("GUI exited before taking mutex; will retry");
                    last_err = Some("gui-died".into());
                }
            }
            Err(e) => {
                if last_err.as_deref() != Some(e.as_str()) {
                    svc_log(&format!("launch: {e}"));
                    last_err = Some(e);
                }
            }
        }
        sleep_interruptible(LAUNCH_RETRY_SECS * 1000);
    }

    if let Some(h) = child.take() {
        unsafe {
            let _ = CloseHandle(h);
        }
    }
    unsafe {
        set_status(SERVICE_STOPPED, false, 0);
    }
    svc_log("service worker stop");
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut PWSTR) {
    STOP_REQUESTED.store(false, Ordering::SeqCst);
    let name = wide(SERVICE_NAME);
    match RegisterServiceCtrlHandlerW(PCWSTR(name.as_ptr()), Some(service_ctrl)) {
        Ok(h) => STATUS_HANDLE.store(h.0 as isize, Ordering::SeqCst),
        Err(e) => {
            svc_log(&format!("RegisterServiceCtrlHandler: {e}"));
            return;
        }
    }
    set_status(SERVICE_START_PENDING, false, 3000);
    service_worker();
}

/// Block as a Windows service (argv contains `--autostart-svc`).
pub fn run_autostart_service() {
    let mut name = wide(SERVICE_NAME);
    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: PWSTR(name.as_mut_ptr()),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW {
            lpServiceName: PWSTR::null(),
            lpServiceProc: None,
        },
    ];
    unsafe {
        if let Err(e) = StartServiceCtrlDispatcherW(table.as_ptr()) {
            svc_log(&format!("StartServiceCtrlDispatcher: {e}"));
        }
    }
}

pub fn uninstall_service() -> Result<(), String> {
    use windows::Win32::System::Services::{
        CloseServiceHandle, ControlService, DeleteService, OpenSCManagerW, OpenServiceW,
        SC_MANAGER_ALL_ACCESS, SERVICE_ALL_ACCESS, SERVICE_CONTROL_STOP, SERVICE_STATUS,
    };

    unsafe {
        let scm = match OpenSCManagerW(None, None, SC_MANAGER_ALL_ACCESS) {
            Ok(h) => h,
            Err(_) => return Ok(()), // no rights / SCM down — treat as cleared
        };
        let name = wide(SERVICE_NAME);
        if let Ok(svc) = OpenServiceW(scm, PCWSTR(name.as_ptr()), SERVICE_ALL_ACCESS) {
            let mut st = SERVICE_STATUS::default();
            let _ = ControlService(svc, SERVICE_CONTROL_STOP, &mut st);
            std::thread::sleep(Duration::from_millis(500));
            let _ = DeleteService(svc);
            let _ = CloseServiceHandle(svc);
        }
        let _ = CloseServiceHandle(scm);
    }
    Ok(())
}

pub fn install_service(exe: &std::path::Path) -> Result<(), String> {
    use windows::Win32::System::Services::{
        CloseServiceHandle, CreateServiceW, OpenSCManagerW, StartServiceW, SC_MANAGER_ALL_ACCESS,
        SERVICE_ALL_ACCESS, SERVICE_AUTO_START, SERVICE_ERROR_NORMAL, SERVICE_WIN32_OWN_PROCESS,
    };

    let _ = uninstall_service();
    std::thread::sleep(Duration::from_millis(200));

    let bin = format!("\"{}\" --autostart-svc", exe.to_string_lossy());
    let bin_w = wide(&bin);
    let name_w = wide(SERVICE_NAME);
    let disp_w = wide(SERVICE_DISPLAY);

    unsafe {
        let scm = OpenSCManagerW(None, None, SC_MANAGER_ALL_ACCESS)
            .map_err(|e| format!("OpenSCManager 失败: {e}（安装服务需要管理员）"))?;

        let svc = CreateServiceW(
            scm,
            PCWSTR(name_w.as_ptr()),
            PCWSTR(disp_w.as_ptr()),
            SERVICE_ALL_ACCESS,
            SERVICE_WIN32_OWN_PROCESS,
            SERVICE_AUTO_START,
            SERVICE_ERROR_NORMAL,
            PCWSTR(bin_w.as_ptr()),
            None,
            None,
            None,
            None,
            None,
        );
        match svc {
            Ok(s) => {
                let _ = StartServiceW(s, None);
                let _ = CloseServiceHandle(s);
                let _ = CloseServiceHandle(scm);
                svc_log(&format!("installed service bin={bin}"));
                Ok(())
            }
            Err(e) => {
                let _ = CloseServiceHandle(scm);
                Err(format!("CreateService 失败: {e}"))
            }
        }
    }
}
