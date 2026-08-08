//! Companion script launchers — path / runtime env / plugin association.
//! Scripts run as separate OS processes (never LoadLibrary into the host).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptLauncher {
    pub id: String,
    /// Display name (defaults to file stem).
    pub name: String,
    /// Absolute path to script or executable.
    pub script_path: String,
    /// Runtime: python | node | powershell | cmd | exe | custom
    pub environment: String,
    /// Optional interpreter / runtime binary (e.g. C:\\Python311\\python.exe).
    #[serde(default)]
    pub env_path: String,
    /// Extra CLI args (space-separated, simple split).
    #[serde(default)]
    pub args: String,
    /// Associated plugin id (empty = none).
    #[serde(default)]
    pub plugin_id: String,
    /// Launch when Window Hub starts.
    #[serde(default)]
    pub start_with_hub: bool,
    /// Register a Startup-folder shortcut for user logon.
    #[serde(default)]
    pub start_on_boot: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LaunchersFile {
    launchers: Vec<ScriptLauncher>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptLauncherStatus {
    #[serde(flatten)]
    pub launcher: ScriptLauncher,
    pub running: bool,
    pub pid: Option<u32>,
}

static CHILDREN: Mutex<Option<HashMap<String, Child>>> = Mutex::new(None);

fn with_children<F, R>(f: F) -> R
where
    F: FnOnce(&mut HashMap<String, Child>) -> R,
{
    let mut guard = CHILDREN.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
    f(guard.as_mut().unwrap())
}

fn load_file() -> LaunchersFile {
    if let Ok(rows) = crate::db::with_conn(|c| crate::db::launchers_list(c)) {
        let launchers = rows
            .into_iter()
            .map(|r| ScriptLauncher {
                id: r.id,
                name: r.name,
                script_path: r.script_path,
                environment: r.environment,
                env_path: r.env_path,
                args: r.args,
                plugin_id: r.plugin_id,
                start_with_hub: r.start_with_hub,
                start_on_boot: r.start_on_boot,
                enabled: r.enabled,
            })
            .collect();
        return LaunchersFile { launchers };
    }
    LaunchersFile::default()
}

fn save_file(file: &LaunchersFile) -> Result<(), String> {
    let rows: Vec<crate::db::LauncherRow> = file
        .launchers
        .iter()
        .map(|r| crate::db::LauncherRow {
            id: r.id.clone(),
            name: r.name.clone(),
            script_path: r.script_path.clone(),
            environment: r.environment.clone(),
            env_path: r.env_path.clone(),
            args: r.args.clone(),
            plugin_id: r.plugin_id.clone(),
            start_with_hub: r.start_with_hub,
            start_on_boot: r.start_on_boot,
            enabled: r.enabled,
        })
        .collect();
    crate::db::with_conn(|c| crate::db::launchers_replace_all(c, &rows))
}

fn new_id() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("sc-{ms}")
}

fn normalize_env(env: &str) -> String {
    match env.trim().to_ascii_lowercase().as_str() {
        "python" | "py" => "python".into(),
        "node" | "nodejs" => "node".into(),
        "powershell" | "pwsh" | "ps1" => "powershell".into(),
        "cmd" | "bat" => "cmd".into(),
        "exe" | "bin" => "exe".into(),
        "custom" => "custom".into(),
        other if !other.is_empty() => other.to_string(),
        _ => "exe".into(),
    }
}

fn validate_launcher(mut row: ScriptLauncher) -> Result<ScriptLauncher, String> {
    row.script_path = row.script_path.trim().to_string();
    if row.script_path.is_empty() {
        return Err("脚本路径不能为空".into());
    }
    if !Path::new(&row.script_path).exists() {
        return Err(format!("脚本不存在: {}", row.script_path));
    }
    row.environment = normalize_env(&row.environment);
    row.env_path = row.env_path.trim().to_string();
    row.plugin_id = row.plugin_id.trim().to_string();
    row.args = row.args.trim().to_string();
    if row.name.trim().is_empty() {
        row.name = Path::new(&row.script_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("脚本")
            .to_string();
    }
    if row.id.trim().is_empty() {
        row.id = new_id();
    }
    Ok(row)
}

fn split_args(s: &str) -> Vec<String> {
    s.split_whitespace()
        .filter(|p| !p.is_empty())
        .map(|p| p.to_string())
        .collect()
}

fn build_command(row: &ScriptLauncher) -> Result<Command, String> {
    let script = &row.script_path;
    let extra = split_args(&row.args);
    let mut cmd = match row.environment.as_str() {
        "python" => {
            let bin = if row.env_path.is_empty() {
                "python".to_string()
            } else {
                row.env_path.clone()
            };
            let mut c = Command::new(bin);
            c.arg(script);
            for a in extra {
                c.arg(a);
            }
            c
        }
        "node" => {
            let bin = if row.env_path.is_empty() {
                "node".to_string()
            } else {
                row.env_path.clone()
            };
            let mut c = Command::new(bin);
            c.arg(script);
            for a in extra {
                c.arg(a);
            }
            c
        }
        "powershell" => {
            let bin = if row.env_path.is_empty() {
                "powershell".to_string()
            } else {
                row.env_path.clone()
            };
            let mut c = Command::new(bin);
            c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script]);
            for a in extra {
                c.arg(a);
            }
            c
        }
        "cmd" => {
            let mut c = Command::new("cmd");
            c.args(["/C", script]);
            for a in extra {
                c.arg(a);
            }
            c
        }
        "custom" => {
            if row.env_path.is_empty() {
                return Err("custom 环境需要填写运行时路径".into());
            }
            let mut c = Command::new(&row.env_path);
            c.arg(script);
            for a in extra {
                c.arg(a);
            }
            c
        }
        _ => {
            // exe — run script_path directly
            let mut c = Command::new(script);
            for a in extra {
                c.arg(a);
            }
            c
        }
    };
    if let Some(parent) = Path::new(script).parent() {
        let _ = cmd.current_dir(parent);
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Detached so closing Hub does not kill the companion by default.
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    Ok(cmd)
}

fn reaping_running(id: &str) -> (bool, Option<u32>) {
    with_children(|map| {
        if let Some(child) = map.get_mut(id) {
            match child.try_wait() {
                Ok(Some(_)) => {
                    map.remove(id);
                    (false, None)
                }
                Ok(None) => (true, Some(child.id())),
                Err(_) => {
                    map.remove(id);
                    (false, None)
                }
            }
        } else {
            (false, None)
        }
    })
}

fn startup_dir() -> Result<PathBuf, String> {
    let appdata = std::env::var_os("APPDATA").ok_or_else(|| "APPDATA missing".to_string())?;
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

fn startup_cmd_path(id: &str) -> Result<PathBuf, String> {
    let mut p = startup_dir()?;
    // Sanitize id for filename
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    p.push(format!("window-hub-{safe}.cmd"));
    Ok(p)
}

fn write_boot_shortcut(row: &ScriptLauncher) -> Result<(), String> {
    let path = startup_cmd_path(&row.id)?;
    let line = match row.environment.as_str() {
        "python" => {
            let bin = if row.env_path.is_empty() {
                "python"
            } else {
                row.env_path.as_str()
            };
            format!(
                "@echo off\r\nstart \"\" \"{bin}\" \"{}\" {}\r\n",
                row.script_path, row.args
            )
        }
        "node" => {
            let bin = if row.env_path.is_empty() {
                "node"
            } else {
                row.env_path.as_str()
            };
            format!(
                "@echo off\r\nstart \"\" \"{bin}\" \"{}\" {}\r\n",
                row.script_path, row.args
            )
        }
        "powershell" => format!(
            "@echo off\r\nstart \"\" powershell -NoProfile -ExecutionPolicy Bypass -File \"{}\" {}\r\n",
            row.script_path, row.args
        ),
        "cmd" => format!(
            "@echo off\r\nstart \"\" cmd /C \"{}\" {}\r\n",
            row.script_path, row.args
        ),
        "custom" => format!(
            "@echo off\r\nstart \"\" \"{}\" \"{}\" {}\r\n",
            row.env_path, row.script_path, row.args
        ),
        _ => format!(
            "@echo off\r\nstart \"\" \"{}\" {}\r\n",
            row.script_path, row.args
        ),
    };
    fs::write(path, line).map_err(|e| e.to_string())
}

fn remove_boot_shortcut(id: &str) -> Result<(), String> {
    let path = startup_cmd_path(id)?;
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn sync_boot_shortcut(row: &ScriptLauncher) -> Result<(), String> {
    if row.start_on_boot && row.enabled {
        write_boot_shortcut(row)
    } else {
        remove_boot_shortcut(&row.id)
    }
}

fn emit_changed(app: &AppHandle) {
    let _ = app.emit("script-launchers-changed", ());
}

fn to_status(row: ScriptLauncher) -> ScriptLauncherStatus {
    let (running, pid) = reaping_running(&row.id);
    ScriptLauncherStatus {
        launcher: row,
        running,
        pid,
    }
}

#[tauri::command]
pub fn list_script_launchers() -> Result<Vec<ScriptLauncherStatus>, String> {
    let file = load_file();
    Ok(file.launchers.into_iter().map(to_status).collect())
}

#[tauri::command]
pub fn upsert_script_launcher(
    app: AppHandle,
    launcher: ScriptLauncher,
) -> Result<ScriptLauncherStatus, String> {
    let row = validate_launcher(launcher)?;
    let mut file = load_file();
    if let Some(pos) = file.launchers.iter().position(|x| x.id == row.id) {
        file.launchers[pos] = row.clone();
    } else {
        file.launchers.push(row.clone());
    }
    sync_boot_shortcut(&row)?;
    save_file(&file)?;
    emit_changed(&app);
    Ok(to_status(row))
}

#[tauri::command]
pub fn delete_script_launcher(app: AppHandle, id: String) -> Result<(), String> {
    let mut file = load_file();
    file.launchers.retain(|x| x.id != id);
    save_file(&file)?;
    let _ = remove_boot_shortcut(&id);
    let _ = stop_script_launcher_inner(&id);
    emit_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn pick_script_file() -> Result<Option<String>, String> {
    let file = rfd::FileDialog::new()
        .add_filter(
            "Scripts",
            &["py", "js", "mjs", "cjs", "ts", "ps1", "bat", "cmd", "exe"],
        )
        .add_filter("All", &["*"])
        .pick_file();
    Ok(file.map(|p| p.to_string_lossy().into_owned()))
}

fn stop_script_launcher_inner(id: &str) -> Result<(), String> {
    with_children(|map| {
        if let Some(mut child) = map.remove(id) {
            let _ = child.kill();
            let _ = child.wait();
        }
        Ok(())
    })
}

#[tauri::command]
pub fn start_script_launcher(app: AppHandle, id: String) -> Result<ScriptLauncherStatus, String> {
    let file = load_file();
    let row = file
        .launchers
        .iter()
        .find(|x| x.id == id)
        .cloned()
        .ok_or_else(|| "启动器不存在".to_string())?;
    if !row.enabled {
        return Err("启动器已禁用".into());
    }
    let (running, _) = reaping_running(&id);
    if running {
        return Ok(to_status(row));
    }
    let mut cmd = build_command(&row)?;
    let child = cmd.spawn().map_err(|e| format!("启动失败: {e}"))?;
    with_children(|map| {
        map.insert(id.clone(), child);
    });
    emit_changed(&app);
    Ok(to_status(row))
}

#[tauri::command]
pub fn stop_script_launcher(app: AppHandle, id: String) -> Result<(), String> {
    stop_script_launcher_inner(&id)?;
    emit_changed(&app);
    Ok(())
}

/// Called from host setup: start launchers marked `start_with_hub`.
pub fn start_hub_associated_launchers() {
    let file = load_file();
    for row in file.launchers {
        if !row.enabled || !row.start_with_hub {
            continue;
        }
        let (running, _) = reaping_running(&row.id);
        if running {
            continue;
        }
        if let Ok(mut cmd) = build_command(&row) {
            if let Ok(child) = cmd.spawn() {
                let id = row.id.clone();
                with_children(|map| {
                    map.insert(id, child);
                });
            }
        }
    }
}

/// When a plugin is enabled, start launchers associated with that plugin id.
pub fn start_launchers_for_plugin(plugin_id: &str) {
    if plugin_id.is_empty() {
        return;
    }
    let file = load_file();
    for row in file.launchers {
        if !row.enabled || row.plugin_id != plugin_id {
            continue;
        }
        let (running, _) = reaping_running(&row.id);
        if running {
            continue;
        }
        if let Ok(mut cmd) = build_command(&row) {
            if let Ok(child) = cmd.spawn() {
                let id = row.id.clone();
                with_children(|map| {
                    map.insert(id, child);
                });
            }
        }
    }
}
