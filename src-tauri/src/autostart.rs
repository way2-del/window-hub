//! Window Hub 开机自启（当前用户 Startup 文件夹）。

use std::fs;
use std::path::PathBuf;

const STARTUP_CMD_NAME: &str = "WindowHub.cmd";

fn startup_dir() -> Result<PathBuf, String> {
    let appdata = std::env::var("APPDATA").map_err(|_| "APPDATA 未设置".to_string())?;
    let mut dir = PathBuf::from(appdata);
    dir.push("Microsoft");
    dir.push("Windows");
    dir.push("Start Menu");
    dir.push("Programs");
    dir.push("Startup");
    if !dir.is_dir() {
        return Err("找不到用户 Startup 文件夹".into());
    }
    Ok(dir)
}

fn startup_cmd_path() -> Result<PathBuf, String> {
    let mut p = startup_dir()?;
    p.push(STARTUP_CMD_NAME);
    Ok(p)
}

fn current_exe_path() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| format!("无法获取程序路径: {e}"))
}

/// 是否已在用户 Startup 中注册（以快捷脚本是否存在为准）。
#[tauri::command]
pub fn get_open_at_login() -> bool {
    startup_cmd_path()
        .map(|p| p.is_file())
        .unwrap_or(false)
}

/// 开启/关闭开机自启。开启时写入 Startup 下的 .cmd，指向当前 exe。
#[tauri::command]
pub fn set_open_at_login(enabled: bool) -> Result<bool, String> {
    let path = startup_cmd_path()?;
    if enabled {
        let exe = current_exe_path()?;
        let exe_s = exe.to_string_lossy();
        // start "" keeps the cmd window from lingering; quote path for spaces.
        let body = format!("@echo off\r\nstart \"\" \"{exe_s}\"\r\n");
        fs::write(&path, body).map_err(|e| format!("写入开机自启失败: {e}"))?;
    } else if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("移除开机自启失败: {e}"))?;
    }
    Ok(path.is_file())
}
