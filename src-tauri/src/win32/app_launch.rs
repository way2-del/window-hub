//! Host launch prefs: Service or Scheduled Task autostart (or off).
//! Never keeps the GUI elevated — SCM install/uninstall uses a one-shot UAC helper.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::AppHandle;

const LAYERS_KEY: &str =
    r"Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE_NAME: &str = "WindowHub";
const TASK_NAME: &str = "WindowHubAutoStart";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AutostartBackend {
    Service,
    Task,
    None,
}

impl Default for AutostartBackend {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralPrefs {
    /// Derived from `start_on_boot_backend` on apply; kept for older JSON rows.
    #[serde(default)]
    pub start_on_boot: bool,
    /// Legacy field — always forced off (no persistent admin GUI).
    #[serde(default)]
    pub run_as_admin: bool,
    /// User-selected autostart mechanism (`none` = off).
    #[serde(default)]
    pub start_on_boot_backend: AutostartBackend,
}

impl Default for GeneralPrefs {
    fn default() -> Self {
        Self {
            start_on_boot: false,
            run_as_admin: false,
            start_on_boot_backend: AutostartBackend::None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralPrefsView {
    pub start_on_boot: bool,
    /// Always false — kept for older frontends.
    pub run_as_admin: bool,
    pub is_elevated: bool,
    /// Always false — kept for older frontends.
    pub needs_relaunch: bool,
    pub start_on_boot_backend: AutostartBackend,
    /// Set when a one-shot UAC helper just ran (install/uninstall service).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

pub fn current_exe_path() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| e.to_string())
}

#[cfg(windows)]
pub fn is_process_elevated() -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = Default::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elev = TOKEN_ELEVATION::default();
        let mut ret = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elev as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret,
        );
        let _ = CloseHandle(token);
        ok.is_ok() && elev.TokenIsElevated != 0
    }
}

#[cfg(not(windows))]
pub fn is_process_elevated() -> bool {
    false
}

pub fn load_prefs() -> GeneralPrefs {
    if let Ok(Some(v)) = crate::db::with_conn(|c| crate::db::general_get(c)) {
        if let Ok(mut p) = serde_json::from_value::<GeneralPrefs>(v) {
            // Prefer explicit backend; migrate older boolean-only rows to task.
            if p.start_on_boot_backend != AutostartBackend::None {
                p.start_on_boot = true;
            } else if p.start_on_boot {
                p.start_on_boot_backend = AutostartBackend::Task;
            } else {
                p.start_on_boot = false;
            }
            return p;
        }
    }
    GeneralPrefs::default()
}

fn save_prefs(prefs: &GeneralPrefs) -> Result<(), String> {
    let v = serde_json::to_value(prefs).map_err(|e| e.to_string())?;
    crate::db::with_conn(|c| crate::db::general_set(c, &v))
}

fn normalize_path_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

fn strip_quotes(s: &str) -> &str {
    let t = s.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        &t[1..t.len() - 1]
    } else {
        t
    }
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    normalize_path_key(a).eq_ignore_ascii_case(&normalize_path_key(b))
}

fn is_our_exe_command(cmd: &str, our_exe: &Path) -> bool {
    let path_part = strip_quotes(cmd.split_whitespace().next().unwrap_or(cmd));
    let p = PathBuf::from(path_part);
    if paths_equal(&p, our_exe) {
        return true;
    }
    p.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.eq_ignore_ascii_case("window-hub.exe"))
        .unwrap_or(false)
}

#[cfg(windows)]
fn clear_run_as_admin_flag() -> Result<(), String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS};
    use winreg::RegKey;

    let exe = current_exe_path()?;
    let exe_s = exe.to_string_lossy().to_string();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey_with_flags(LAYERS_KEY, KEY_ALL_ACCESS) {
        let _ = key.delete_value(&exe_s);
        let canon = normalize_path_key(&exe);
        if canon != exe_s {
            let _ = key.delete_value(&canon);
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn clear_run_as_admin_flag() -> Result<(), String> {
    Ok(())
}

/// Public: strip leftover RUNASADMIN so Explorer can drag-drop onto the GUI.
pub fn clear_legacy_admin_flag() -> Result<(), String> {
    clear_run_as_admin_flag()
}

fn startup_dir() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    let mut dir = PathBuf::from(appdata);
    dir.push("Microsoft\\Windows\\Start Menu\\Programs\\Startup");
    dir.is_dir().then_some(dir)
}

fn clear_startup_folder_residuals() {
    let Some(dir) = startup_dir() else {
        return;
    };
    if let Ok(entries) = fs::read_dir(&dir) {
        for ent in entries.flatten() {
            let path = ent.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            if lower.starts_with("window-hub")
                && (lower.ends_with(".cmd") || lower.ends_with(".lnk") || lower.ends_with(".bat"))
            {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

/// Remove legacy HKCU Run entries from the old registry-based autostart.
#[cfg(windows)]
fn clear_legacy_run_residuals() {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS};
    use winreg::RegKey;

    let Ok(our_exe) = current_exe_path() else {
        return;
    };
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(key) = hkcu.open_subkey_with_flags(RUN_KEY, KEY_ALL_ACCESS) else {
        return;
    };
    let _ = key.delete_value(RUN_VALUE_NAME);
    let names: Vec<String> = key
        .enum_values()
        .filter_map(|r| r.ok().map(|(name, _)| name))
        .collect();
    for name in names {
        if let Ok(data) = key.get_value::<String, _>(&name) {
            if is_our_exe_command(&data, &our_exe) {
                let _ = key.delete_value(&name);
            }
        }
    }
}

#[cfg(not(windows))]
fn clear_legacy_run_residuals() {}

#[cfg(windows)]
fn uninstall_task() {
    let _ = Command::new("schtasks")
        .args(["/Delete", "/TN", TASK_NAME, "/F"])
        .output();
}

#[cfg(not(windows))]
fn uninstall_task() {}

#[cfg(windows)]
fn install_task(exe: &Path, highest: bool) -> Result<(), String> {
    uninstall_task();
    let tr = format!("\"{}\"", exe.to_string_lossy());
    let mut args = vec![
        "/Create".into(),
        "/TN".into(),
        TASK_NAME.into(),
        "/TR".into(),
        tr,
        "/SC".into(),
        "ONLOGON".into(),
        "/F".into(),
    ];
    if highest {
        args.push("/RL".into());
        args.push("HIGHEST".into());
    }
    let out = Command::new("schtasks")
        .args(&args)
        .output()
        .map_err(|e| format!("schtasks 启动失败: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        return Err(format!(
            "创建计划任务失败: {} {}",
            err.trim(),
            stdout.trim()
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn install_task(_exe: &Path, _highest: bool) -> Result<(), String> {
    Err("仅支持 Windows".into())
}

/// Tear down non-service autostart leftovers (no admin required).
fn clear_user_autostart() {
    uninstall_task();
    clear_legacy_run_residuals();
    clear_startup_folder_residuals();
}

#[cfg(windows)]
fn service_is_installed() -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::System::Services::{
        CloseServiceHandle, OpenSCManagerW, OpenServiceW, SC_MANAGER_CONNECT, SERVICE_QUERY_STATUS,
    };

    let name_w: Vec<u16> = std::ffi::OsStr::new(crate::win32::autostart_svc::SERVICE_NAME)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let Ok(scm) = OpenSCManagerW(None, None, SC_MANAGER_CONNECT) else {
            return false;
        };
        let found = match OpenServiceW(scm, PCWSTR(name_w.as_ptr()), SERVICE_QUERY_STATUS) {
            Ok(svc) => {
                let _ = CloseServiceHandle(svc);
                true
            }
            Err(_) => false,
        };
        let _ = CloseServiceHandle(scm);
        found
    }
}

#[cfg(not(windows))]
fn service_is_installed() -> bool {
    false
}

/// Ask the user before the Windows UAC prompt (install/uninstall service).
#[cfg(windows)]
fn confirm_temp_admin(action_label: &str) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDCANCEL, MB_ICONINFORMATION, MB_OKCANCEL,
    };

    let title: Vec<u16> = std::ffi::OsStr::new("Window Hub")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let body = format!(
        "{action_label}\n\n接下来将弹出 Windows 用户账户控制（UAC），请点击「是」。\n主程序不会保持管理员身份运行（避免无法从资源管理器拖放文件）。"
    );
    let text: Vec<u16> = std::ffi::OsStr::new(&body)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let ret = unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OKCANCEL | MB_ICONINFORMATION,
        )
    };
    if ret == IDCANCEL {
        return Err("已取消（未请求管理员权限）".into());
    }
    Ok(())
}

#[cfg(not(windows))]
fn confirm_temp_admin(_action_label: &str) -> Result<(), String> {
    Ok(())
}

/// ShellExecuteEx `runas` + wait. Used for one-shot UAC helpers (SCM / …).
#[cfg(windows)]
pub(crate) fn run_elevated_helper_and_wait(arg: &str, action_label: &str) -> Result<(), String> {
    confirm_temp_admin(action_label)?;

    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};
    use windows::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let exe = current_exe_path()?;
    let file_w: Vec<u16> = exe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let op: Vec<u16> = std::ffi::OsStr::new("runas")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let params: Vec<u16> = std::ffi::OsStr::new(arg)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(op.as_ptr()),
        lpFile: PCWSTR(file_w.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0 as i32,
        ..Default::default()
    };

    unsafe {
        ShellExecuteExW(&mut info).map_err(|_| {
            "已取消 UAC，或无法请求管理员权限".to_string()
        })?;
        if info.hProcess.is_invalid() {
            return Err("已取消 UAC，或无法请求管理员权限".into());
        }
        let wait = WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1u32;
        let _ = windows::Win32::System::Threading::GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        if wait != WAIT_OBJECT_0 {
            return Err("等待临时管理员助手结束失败".into());
        }
        if code != 0 {
            return Err(format!("临时管理员操作失败，退出码 {code}"));
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn run_elevated_helper_and_wait(_arg: &str, _action_label: &str) -> Result<(), String> {
    Err("仅支持 Windows".into())
}

/// Returns `Some(notice)` when a one-shot UAC helper ran.
#[cfg(windows)]
fn ensure_service_uninstalled() -> Result<Option<String>, String> {
    if !service_is_installed() {
        return Ok(None);
    }
    if is_process_elevated() {
        crate::win32::autostart_svc::uninstall_service()?;
        return Ok(Some("已卸载系统服务自启。".into()));
    }
    run_elevated_helper_and_wait(
        "--uninstall-autostart-service",
        "卸载系统服务自启需要临时管理员权限。",
    )?;
    Ok(Some(
        "已通过临时管理员权限卸载系统服务；当前窗口仍为普通权限。".into(),
    ))
}

#[cfg(not(windows))]
fn ensure_service_uninstalled() -> Result<Option<String>, String> {
    Ok(None)
}

/// Returns `Some(notice)` when a one-shot UAC helper ran (or refreshed while elevated).
#[cfg(windows)]
fn ensure_service_installed() -> Result<Option<String>, String> {
    let exe = current_exe_path()?;
    if is_process_elevated() {
        crate::win32::autostart_svc::install_service(&exe)?;
        return Ok(Some("已安装系统服务自启。".into()));
    }
    // Already installed: don't re-prompt UAC on every prefs save.
    if service_is_installed() {
        return Ok(None);
    }
    run_elevated_helper_and_wait(
        "--install-autostart-service",
        "安装系统服务自启需要临时管理员权限。",
    )?;
    if !service_is_installed() {
        return Err("服务安装未完成（可能取消了 UAC）".into());
    }
    Ok(Some(
        "已通过临时管理员权限安装系统服务；登录后由服务以普通权限拉起主程序。".into(),
    ))
}

#[cfg(not(windows))]
fn ensure_service_installed() -> Result<Option<String>, String> {
    Err("仅支持 Windows".into())
}

/// One-shot elevated helper body for `--install-autostart-service`.
pub fn install_service_elevated_helper() -> Result<(), String> {
    #[cfg(windows)]
    {
        if !is_process_elevated() {
            return Err("需要管理员权限".into());
        }
        let exe = current_exe_path()?;
        crate::win32::autostart_svc::install_service(&exe)
    }
    #[cfg(not(windows))]
    {
        Err("仅支持 Windows".into())
    }
}

/// One-shot elevated helper body for `--uninstall-autostart-service`.
pub fn uninstall_service_elevated_helper() -> Result<(), String> {
    #[cfg(windows)]
    {
        if !is_process_elevated() {
            return Err("需要管理员权限".into());
        }
        crate::win32::autostart_svc::uninstall_service()
    }
    #[cfg(not(windows))]
    {
        Err("仅支持 Windows".into())
    }
}

/// Tear down every autostart mechanism we may have created.
fn clear_all_autostart() -> Result<Option<String>, String> {
    let notice = ensure_service_uninstalled()?;
    clear_user_autostart();
    Ok(notice)
}

/// Apply exactly the mechanism the UI selected; always clear the other first.
/// Returns `(backend, optional_notice)` when a one-shot UAC helper ran.
fn apply_autostart(want: AutostartBackend) -> Result<(AutostartBackend, Option<String>), String> {
    match want {
        AutostartBackend::None => {
            let notice = clear_all_autostart()?;
            Ok((AutostartBackend::None, notice))
        }
        AutostartBackend::Task => {
            // Drop service first (may one-shot UAC), then install user task.
            let notice = ensure_service_uninstalled()?;
            clear_user_autostart();
            let exe = current_exe_path()?;
            install_task(&exe, false)?;
            Ok((AutostartBackend::Task, notice))
        }
        AutostartBackend::Service => {
            clear_user_autostart();
            let notice = ensure_service_installed()?;
            Ok((AutostartBackend::Service, notice))
        }
    }
}

pub fn apply_launch_flags(prefs: &mut GeneralPrefs) -> Result<Option<String>, String> {
    // Never persist elevated GUI; clear any legacy AppCompat RUNASADMIN.
    prefs.run_as_admin = false;
    clear_run_as_admin_flag()?;
    let want = if prefs.start_on_boot_backend != AutostartBackend::None {
        prefs.start_on_boot_backend.clone()
    } else if prefs.start_on_boot {
        AutostartBackend::Task
    } else {
        AutostartBackend::None
    };
    let (backend, notice) = apply_autostart(want)?;
    prefs.start_on_boot_backend = backend;
    prefs.start_on_boot = prefs.start_on_boot_backend != AutostartBackend::None;
    Ok(notice)
}

pub fn prefs_view(prefs: &GeneralPrefs, notice: Option<String>) -> GeneralPrefsView {
    GeneralPrefsView {
        start_on_boot: prefs.start_on_boot,
        run_as_admin: false,
        is_elevated: is_process_elevated(),
        needs_relaunch: false,
        start_on_boot_backend: prefs.start_on_boot_backend.clone(),
        notice,
    }
}

#[tauri::command]
pub fn get_general_prefs() -> GeneralPrefsView {
    let mut prefs = load_prefs();
    prefs.run_as_admin = false;
    prefs_view(&prefs, None)
}

#[tauri::command]
pub fn set_general_prefs(mut prefs: GeneralPrefs) -> Result<GeneralPrefsView, String> {
    prefs.run_as_admin = false;
    let notice = apply_launch_flags(&mut prefs)?;
    save_prefs(&prefs)?;
    Ok(prefs_view(&prefs, notice))
}

/// Relaunch current exe; `as_admin` uses ShellExecute runas (UAC).
#[cfg(windows)]
pub fn relaunch_now(as_admin: bool) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let exe = current_exe_path()?;
    if as_admin {
        let file_w: Vec<u16> = exe
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let op: Vec<u16> = std::ffi::OsStr::new("runas")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            let ret = ShellExecuteW(
                HWND::default(),
                PCWSTR(op.as_ptr()),
                PCWSTR(file_w.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
            if (ret.0 as isize) <= 32 {
                return Err(format!(
                    "提权重启失败（可能取消了 UAC），代码 {}",
                    ret.0 as isize
                ));
            }
        }
        Ok(())
    } else {
        let mut cmd = std::process::Command::new(&exe);
        if let Ok(cwd) = std::env::current_dir() {
            cmd.current_dir(cwd);
        }
        cmd.spawn()
            .map_err(|e| format!("普通权限重启失败: {e}"))?;
        Ok(())
    }
}

#[cfg(not(windows))]
pub fn relaunch_now(_as_admin: bool) -> Result<(), String> {
    Err("仅支持 Windows".into())
}

#[tauri::command]
pub fn relaunch_app(app: AppHandle, as_admin: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        crate::win32::autostart_svc::note_expect_relaunch();
        crate::win32::autostart_svc::clear_user_quit();
    }
    relaunch_now(as_admin)?;
    app.exit(0);
    Ok(())
}
