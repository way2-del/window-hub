//! Per-app volume mixer and output routing for the host control center.
use serde::Serialize;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};
use windows::core::{Interface, GUID, HRESULT, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, MAX_PATH};
use windows::Win32::Media::Audio::{
    eCommunications, eConsole, eMultimedia, eRender, AudioSessionState, IAudioSessionControl,
    IAudioSessionControl2, IAudioSessionManager2, ISimpleAudioVolume, IMMDevice,
    IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE, EDataFlow, ERole,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// Remembers app volume per output device so switching restores the matching level.
fn volume_store() -> &'static Mutex<HashMap<(String, String), u8>> {
    static STORE: OnceLock<Mutex<HashMap<(String, String), u8>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OutputDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MixerApp {
    pub app_key: String,
    pub process_id: u32,
    pub name: String,
    pub volume: u8,
    pub muted: bool,
    pub device_id: String,
    pub is_system: bool,
    /// PNG base64 without data: prefix; None when unavailable.
    pub icon_png_base64: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SoundMixerState {
    pub devices: Vec<OutputDevice>,
    pub apps: Vec<MixerApp>,
    pub default_device_id: String,
}

fn with_com<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|e| e.to_string())?;
        let result = f();
        CoUninitialize();
        result
    }
}

fn enumerator() -> windows::core::Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
}

fn pwstr_to_string(ptr: PWSTR) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe {
        let s = ptr.to_string().unwrap_or_default();
        CoTaskMemFree(Some(ptr.0 as *const _));
        s
    }
}

fn device_id(device: &IMMDevice) -> windows::core::Result<String> {
    unsafe { Ok(pwstr_to_string(device.GetId()?)) }
}

fn device_name(device: &IMMDevice) -> String {
    unsafe {
        let Ok(store) = device.OpenPropertyStore(STGM_READ) else {
            return "音频设备".into();
        };
        let Ok(value) = store.GetValue(&PKEY_Device_FriendlyName) else {
            return "音频设备".into();
        };
        let name = value.to_string();
        if name.is_empty() {
            "音频设备".into()
        } else {
            name
        }
    }
}

fn process_path(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; MAX_PATH as usize];
        let mut size = buf.len() as u32;
        let ok =
            QueryFullProcessImageNameW(handle, Default::default(), PWSTR(buf.as_mut_ptr()), &mut size);
        let _ = CloseHandle(handle);
        if ok.is_err() {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..size as usize]))
    }
}

fn process_display_name(pid: u32) -> String {
    if pid == 0 {
        return "系统声音".into();
    }
    process_path(pid)
        .and_then(|path| {
            std::path::Path::new(&path)
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| format!("应用 {pid}"))
}

fn app_key_for(pid: u32, is_system: bool) -> String {
    if is_system || pid == 0 {
        return "system".into();
    }
    process_path(pid)
        .map(|p| p.to_ascii_lowercase())
        .unwrap_or_else(|| format!("pid:{pid}"))
}

fn app_icon_png(pid: u32, ctl2: &IAudioSessionControl2) -> Option<String> {
    // Prefer the session's own icon path (may be dll,index or an .ico/.exe).
    if let Ok(icon) = unsafe { ctl2.GetIconPath() } {
        let path = pwstr_to_string(icon);
        if !path.is_empty() {
            if let Some(png) = crate::dock::icon::resolve_item_icon_png(&path, "") {
                return Some(png);
            }
            // "%SystemRoot%\system32\mmres.dll,-3004" style — try file part only.
            if let Some(file) = path.split(',').next() {
                let file = file.trim().trim_matches('"');
                if !file.is_empty() {
                    if let Some(png) = crate::dock::icon::resolve_item_icon_png(file, "") {
                        return Some(png);
                    }
                }
            }
        }
    }
    let exe = process_path(pid)?;
    crate::dock::icon::resolve_item_icon_png("", &exe)
}

fn remember_volume(app_key: &str, device_id: &str, volume: u8) {
    if device_id.is_empty() {
        return;
    }
    if let Ok(mut map) = volume_store().lock() {
        map.insert((app_key.to_string(), device_id.to_string()), volume.min(100));
    }
}

fn recalled_volume(app_key: &str, device_id: &str) -> Option<u8> {
    volume_store()
        .lock()
        .ok()
        .and_then(|map| map.get(&(app_key.to_string(), device_id.to_string())).copied())
}

fn is_system_sounds(ctl2: &IAudioSessionControl2) -> bool {
    unsafe {
        // S_OK = system sounds, S_FALSE = not. Both are "success" for Result.
        let hr = (Interface::vtable(ctl2).IsSystemSoundsSession)(Interface::as_raw(ctl2));
        hr == HRESULT(0)
    }
}

fn session_volume(ctl: &IAudioSessionControl) -> windows::core::Result<(u8, bool)> {
    let vol: ISimpleAudioVolume = ctl.cast()?;
    unsafe {
        let level = vol.GetMasterVolume()?;
        let muted = vol.GetMute()?.as_bool();
        Ok(((level * 100.0).round().clamp(0.0, 100.0) as u8, muted))
    }
}

fn set_session_volume(ctl: &IAudioSessionControl, volume: u8) -> windows::core::Result<()> {
    let vol: ISimpleAudioVolume = ctl.cast()?;
    unsafe {
        vol.SetMasterVolume(f32::from(volume.min(100)) / 100.0, std::ptr::null())?;
        if volume > 0 {
            let _ = vol.SetMute(false, std::ptr::null());
        }
        Ok(())
    }
}

fn find_session_on_device(
    device: &IMMDevice,
    process_id: u32,
    system: bool,
) -> windows::core::Result<Option<IAudioSessionControl>> {
    unsafe {
        let mgr: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
        let enumerator = mgr.GetSessionEnumerator()?;
        let count = enumerator.GetCount()?;
        for i in 0..count {
            let ctl = enumerator.GetSession(i)?;
            let ctl2: IAudioSessionControl2 = ctl.cast()?;
            let state = ctl.GetState().unwrap_or(AudioSessionState(2));
            if state.0 == 2 {
                continue;
            }
            let system_session = is_system_sounds(&ctl2);
            let pid = ctl2.GetProcessId().unwrap_or(0);
            if system {
                if system_session {
                    return Ok(Some(ctl));
                }
            } else if pid == process_id && !system_session {
                return Ok(Some(ctl));
            }
        }
        Ok(None)
    }
}

fn find_session_anywhere(
    devices: &IMMDeviceEnumerator,
    process_id: u32,
    system: bool,
) -> Result<(String, IAudioSessionControl), String> {
    unsafe {
        let collection = devices
            .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
            .map_err(|e| e.to_string())?;
        let count = collection.GetCount().map_err(|e| e.to_string())?;
        for i in 0..count {
            let device = collection.Item(i).map_err(|e| e.to_string())?;
            if let Ok(Some(ctl)) = find_session_on_device(&device, process_id, system) {
                return Ok((device_id(&device).unwrap_or_default(), ctl));
            }
        }
    }
    Err("未找到该应用的音频会话".into())
}

pub fn sound_mixer_state() -> Result<SoundMixerState, String> {
    with_com(|| unsafe {
        let devices = enumerator().map_err(|e| e.to_string())?;
        let default = devices
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|e| e.to_string())?;
        let default_id = device_id(&default).unwrap_or_default();
        let collection = devices
            .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
            .map_err(|e| e.to_string())?;
        let count = collection.GetCount().map_err(|e| e.to_string())?;
        let mut outs = Vec::with_capacity(count as usize);
        let mut apps: HashMap<String, MixerApp> = HashMap::new();
        for i in 0..count {
            let device = collection.Item(i).map_err(|e| e.to_string())?;
            let id = device_id(&device).unwrap_or_default();
            outs.push(OutputDevice {
                is_default: id == default_id,
                name: device_name(&device),
                id: id.clone(),
            });
            let Ok(mgr) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                continue;
            };
            let Ok(sessions) = mgr.GetSessionEnumerator() else {
                continue;
            };
            let Ok(session_count) = sessions.GetCount() else {
                continue;
            };
            for s in 0..session_count {
                let Ok(ctl) = sessions.GetSession(s) else {
                    continue;
                };
                let Ok(ctl2) = ctl.cast::<IAudioSessionControl2>() else {
                    continue;
                };
                let state = ctl.GetState().unwrap_or(AudioSessionState(2));
                if state.0 == 2 {
                    continue;
                }
                let is_system = is_system_sounds(&ctl2);
                let pid = ctl2.GetProcessId().unwrap_or(0);
                if !is_system && pid == 0 {
                    continue;
                }
                let Ok((volume, muted)) = session_volume(&ctl) else {
                    continue;
                };
                let key = app_key_for(pid, is_system);
                if apps.contains_key(&key) {
                    continue;
                }
                let persisted = policy_get_endpoint(pid).unwrap_or_default();
                let device_for_app = if persisted.is_empty() {
                    id.clone()
                } else {
                    persisted
                };
                let display = if is_system {
                    "系统声音".into()
                } else {
                    let mut name = String::new();
                    if let Ok(dn) = ctl.GetDisplayName() {
                        name = pwstr_to_string(dn);
                    }
                    if name.is_empty() || name.starts_with('@') {
                        name = process_display_name(pid);
                    }
                    name
                };
                let icon_png_base64 = if is_system {
                    None
                } else {
                    app_icon_png(pid, &ctl2)
                };
                apps.insert(
                    key.clone(),
                    MixerApp {
                        app_key: key,
                        process_id: pid,
                        name: display,
                        volume,
                        muted,
                        device_id: device_for_app,
                        is_system,
                        icon_png_base64,
                    },
                );
            }
        }
        outs.sort_by(|a, b| a.name.cmp(&b.name));
        let mut apps: Vec<_> = apps.into_values().collect();
        apps.sort_by(|a, b| match (a.is_system, b.is_system) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });
        Ok(SoundMixerState {
            devices: outs,
            apps,
            default_device_id: default_id,
        })
    })
}

pub fn set_default_output_device(device_id: &str) -> Result<(), String> {
    with_com(|| set_default_endpoint(device_id))
}

pub fn set_app_volume(
    app_key: &str,
    process_id: u32,
    is_system: bool,
    volume: u8,
) -> Result<(), String> {
    with_com(|| {
        let devices = enumerator().map_err(|e| e.to_string())?;
        let (device_id, ctl) = find_session_anywhere(&devices, process_id, is_system)?;
        set_session_volume(&ctl, volume).map_err(|e| e.to_string())?;
        remember_volume(app_key, &device_id, volume);
        Ok(())
    })
}

pub fn set_app_output_device(
    app_key: &str,
    process_id: u32,
    is_system: bool,
    device_id: &str,
) -> Result<(), String> {
    if is_system || process_id == 0 {
        return Err("系统声音无法单独指定输出设备".into());
    }
    with_com(|| {
        let devices = enumerator().map_err(|e| e.to_string())?;
        if let Ok((old_id, ctl)) = find_session_anywhere(&devices, process_id, false) {
            if let Ok((vol, _)) = session_volume(&ctl) {
                remember_volume(app_key, &old_id, vol);
            }
        }
        policy_set_endpoint(process_id, device_id)?;
        std::thread::sleep(std::time::Duration::from_millis(100));
        if let Some(vol) = recalled_volume(app_key, device_id) {
            if let Ok((_, ctl)) = find_session_anywhere(&devices, process_id, false) {
                let _ = set_session_volume(&ctl, vol);
            }
        } else if let Ok((new_id, ctl)) = find_session_anywhere(&devices, process_id, false) {
            if let Ok((vol, _)) = session_volume(&ctl) {
                remember_volume(app_key, &new_id, vol);
            }
        }
        Ok(())
    })
}

// ── Undocumented PolicyConfig / AudioPolicyConfig ───────────────────

const POLICY_CONFIG_CLSID: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
const IPOLICY_CONFIG_IID: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);
const AUDIO_POLICY_FACTORY_IID: GUID = GUID::from_u128(0xab3d4648_e242_459f_b02f_541c70306324);
const AUDIO_POLICY_FACTORY_DOWNLEVEL: GUID =
    GUID::from_u128(0x2a59116d_6c4f_45e0_a74f_707e3fef9258);

type SetDefaultEndpointFn = unsafe extern "system" fn(*mut c_void, PCWSTR, ERole) -> HRESULT;
type SetPersistedFn =
    unsafe extern "system" fn(*mut c_void, u32, EDataFlow, ERole, HSTRING) -> HRESULT;
type GetPersistedFn =
    unsafe extern "system" fn(*mut c_void, u32, EDataFlow, ERole, *mut HSTRING) -> HRESULT;
type QiFn = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
type DllGetActivationFactoryFn =
    unsafe extern "system" fn(HSTRING, *mut *mut c_void) -> HRESULT;

unsafe fn vtable_slot(obj: *mut c_void, index: usize) -> *const c_void {
    let vtbl = *(obj as *const *const *const c_void);
    *vtbl.add(index)
}

fn set_default_endpoint(id: &str) -> Result<(), String> {
    unsafe {
        let unknown: windows::core::IUnknown =
            CoCreateInstance(&POLICY_CONFIG_CLSID, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let mut cfg: *mut c_void = std::ptr::null_mut();
        unknown
            .query(&IPOLICY_CONFIG_IID, &mut cfg)
            .ok()
            .map_err(|e| e.to_string())?;
        if cfg.is_null() {
            return Err("无法创建设备策略接口".into());
        }
        // IUnknown(3) + 10 methods → SetDefaultEndpoint at index 13.
        let set: SetDefaultEndpointFn = std::mem::transmute(vtable_slot(cfg, 13));
        let wide: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
        for role in [eConsole, eMultimedia, eCommunications] {
            set(cfg, PCWSTR(wide.as_ptr()), role)
                .ok()
                .map_err(|e| format!("切换默认输出失败: {e}"))?;
        }
        let _ = windows::core::IUnknown::from_raw(cfg);
        Ok(())
    }
}

fn activate_audio_policy_factory() -> Result<*mut c_void, String> {
    unsafe {
        let class_id = HSTRING::from("Windows.Media.Internal.AudioPolicyConfig");
        let lib = LoadLibraryW(windows::core::w!("AudioSes.dll")).map_err(|e| e.to_string())?;
        let proc = GetProcAddress(lib, windows::core::s!("DllGetActivationFactory"))
            .ok_or_else(|| "DllGetActivationFactory missing".to_string())?;
        let get_factory: DllGetActivationFactoryFn = std::mem::transmute(proc);
        let mut factory: *mut c_void = std::ptr::null_mut();
        get_factory(std::mem::transmute_copy(&class_id), &mut factory)
            .ok()
            .map_err(|e| e.to_string())?;
        if factory.is_null() {
            return Err("AudioPolicyConfig 工厂为空".into());
        }
        let qi: QiFn = std::mem::transmute(vtable_slot(factory, 0));
        for iid in [AUDIO_POLICY_FACTORY_IID, AUDIO_POLICY_FACTORY_DOWNLEVEL] {
            let mut typed: *mut c_void = std::ptr::null_mut();
            if qi(factory, &iid, &mut typed).is_ok() && !typed.is_null() {
                let _ = windows::core::IUnknown::from_raw(factory);
                return Ok(typed);
            }
        }
        Ok(factory)
    }
}

fn policy_set_endpoint(process_id: u32, device_id: &str) -> Result<(), String> {
    unsafe {
        let factory = activate_audio_policy_factory()?;
        let set: SetPersistedFn = std::mem::transmute(vtable_slot(factory, 25));
        let hs = HSTRING::from(device_id);
        let mut any_ok = false;
        for role in [eConsole, eMultimedia, eCommunications] {
            let hr = set(factory, process_id, eRender, role, std::mem::transmute_copy(&hs));
            if hr.is_ok() {
                any_ok = true;
            } else if hr != HRESULT(0x80070057u32 as i32) {
                let _ = windows::core::IUnknown::from_raw(factory);
                return Err(format!("无法切换应用输出设备 ({hr:?})"));
            }
        }
        let _ = windows::core::IUnknown::from_raw(factory);
        if !any_ok {
            return Err("该应用当前没有活动音频流，无法切换输出设备".into());
        }
        Ok(())
    }
}

fn policy_get_endpoint(process_id: u32) -> Result<String, String> {
    if process_id == 0 {
        return Ok(String::new());
    }
    unsafe {
        let factory = activate_audio_policy_factory()?;
        let get: GetPersistedFn = std::mem::transmute(vtable_slot(factory, 26));
        let mut hs = HSTRING::new();
        let hr = get(factory, process_id, eRender, eMultimedia, &mut hs);
        let _ = windows::core::IUnknown::from_raw(factory);
        if hr.is_err() {
            return Ok(String::new());
        }
        Ok(hs.to_string())
    }
}
