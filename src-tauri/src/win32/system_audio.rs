//! Default render endpoint volume + output device switching for island flyout.

#![cfg(windows)]
// IPolicyConfig COM vtable must keep Windows PascalCase method names.
#![allow(non_snake_case)]

use std::ffi::c_void;

use serde::Serialize;
use windows::core::{interface, IUnknown, IUnknown_Vtbl, GUID, HRESULT, PCWSTR, PROPVARIANT};
use windows::Win32::Foundation::BOOL;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eCommunications, eConsole, eMultimedia, eRender, ERole, IMMDevice, IMMDeviceEnumerator,
    MMDeviceEnumerator, DEVICE_STATE_ACTIVE, WAVEFORMATEX,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeSnapshot {
    pub level: u8,
    pub muted: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioOutputDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// Undocumented PolicyConfig CLSID (Windows Sound CPL).
const CLSID_POLICY_CONFIG: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

/// `{a45c254e-df1c-4efd-8020-67d146a850e0},14` — PKEY_Device_FriendlyName
const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 14,
};

#[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
unsafe trait IPolicyConfig: IUnknown {
    unsafe fn GetMixFormat(
        &self,
        device_name: PCWSTR,
        format: *mut *mut WAVEFORMATEX,
    ) -> HRESULT;
    unsafe fn GetDeviceFormat(
        &self,
        device_name: PCWSTR,
        default: i32,
        format: *mut *mut WAVEFORMATEX,
    ) -> HRESULT;
    unsafe fn ResetDeviceFormat(&self, device_name: PCWSTR) -> HRESULT;
    unsafe fn SetDeviceFormat(
        &self,
        device_name: PCWSTR,
        endpoint: *mut WAVEFORMATEX,
        mix: *mut WAVEFORMATEX,
    ) -> HRESULT;
    unsafe fn GetProcessingPeriod(
        &self,
        device_name: PCWSTR,
        default: i32,
        default_period: *mut i64,
        min_period: *mut i64,
    ) -> HRESULT;
    unsafe fn SetProcessingPeriod(&self, device_name: PCWSTR, period: *mut i64) -> HRESULT;
    unsafe fn GetShareMode(&self, device_name: PCWSTR, mode: *mut c_void) -> HRESULT;
    unsafe fn SetShareMode(&self, device_name: PCWSTR, mode: *mut c_void) -> HRESULT;
    unsafe fn GetPropertyValue(
        &self,
        device_name: PCWSTR,
        fx_store: i32,
        key: *const PROPERTYKEY,
        value: *mut PROPVARIANT,
    ) -> HRESULT;
    unsafe fn SetPropertyValue(
        &self,
        device_name: PCWSTR,
        fx_store: i32,
        key: *const PROPERTYKEY,
        value: *mut PROPVARIANT,
    ) -> HRESULT;
    unsafe fn SetDefaultEndpoint(&self, device_name: PCWSTR, role: ERole) -> HRESULT;
    unsafe fn SetEndpointVisibility(&self, device_name: PCWSTR, visible: i32) -> HRESULT;
}

fn ensure_com() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

fn enumerator() -> Result<IMMDeviceEnumerator, String> {
    ensure_com();
    unsafe {
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
            .map_err(|e| format!("MMDeviceEnumerator: {e}"))
    }
}

fn with_endpoint_volume<T>(
    f: impl FnOnce(&IAudioEndpointVolume) -> Result<T, String>,
) -> Result<T, String> {
    unsafe {
        let enumerator = enumerator()?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|e| format!("GetDefaultAudioEndpoint: {e}"))?;
        let volume: IAudioEndpointVolume = device
            .Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
            .map_err(|e| format!("Activate IAudioEndpointVolume: {e}"))?;
        f(&volume)
    }
}

fn pwstr_to_string(ptr: windows::core::PWSTR) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { ptr.to_string().unwrap_or_default() }
}

fn device_friendly_name(device: &IMMDevice) -> String {
    unsafe {
        let store = match device.OpenPropertyStore(STGM_READ) {
            Ok(s) => s,
            Err(_) => return "音频设备".into(),
        };
        match store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME) {
            Ok(pv) => {
                let name = pv.to_string();
                if name.trim().is_empty() {
                    "音频设备".into()
                } else {
                    name
                }
            }
            Err(_) => "音频设备".into(),
        }
    }
}

fn device_id_string(device: &IMMDevice) -> Result<String, String> {
    unsafe {
        let id = device.GetId().map_err(|e| format!("GetId: {e}"))?;
        let s = pwstr_to_string(id);
        CoTaskMemFree(Some(id.0 as *const c_void));
        if s.is_empty() {
            Err("empty device id".into())
        } else {
            Ok(s)
        }
    }
}

pub fn snapshot() -> VolumeSnapshot {
    with_endpoint_volume(|vol| unsafe {
        let level = vol
            .GetMasterVolumeLevelScalar()
            .map_err(|e| format!("GetMasterVolumeLevelScalar: {e}"))?;
        let muted = vol
            .GetMute()
            .map_err(|e| format!("GetMute: {e}"))?
            .as_bool();
        let pct = (level * 100.0).round().clamp(0.0, 100.0) as u8;
        Ok(VolumeSnapshot {
            level: pct,
            muted,
        })
    })
    .unwrap_or(VolumeSnapshot {
        level: 0,
        muted: true,
    })
}

pub fn set_level(level: u8) -> Result<VolumeSnapshot, String> {
    let scalar = (level.min(100) as f32) / 100.0;
    with_endpoint_volume(|vol| unsafe {
        vol.SetMasterVolumeLevelScalar(scalar, std::ptr::null::<GUID>())
            .map_err(|e| format!("SetMasterVolumeLevelScalar: {e}"))?;
        Ok(())
    })?;
    let snap = snapshot();
    crate::win32::system_monitor::patch_volume(snap.clone());
    Ok(snap)
}

pub fn set_muted(muted: bool) -> Result<VolumeSnapshot, String> {
    with_endpoint_volume(|vol| unsafe {
        vol.SetMute(BOOL::from(muted), std::ptr::null::<GUID>())
            .map_err(|e| format!("SetMute: {e}"))?;
        Ok(())
    })?;
    let snap = snapshot();
    crate::win32::system_monitor::patch_volume(snap.clone());
    Ok(snap)
}

pub fn list_output_devices() -> Result<Vec<AudioOutputDevice>, String> {
    unsafe {
        let enumerator = enumerator()?;
        let default_id = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .ok()
            .and_then(|d| device_id_string(&d).ok());

        let collection = enumerator
            .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
            .map_err(|e| format!("EnumAudioEndpoints: {e}"))?;
        let count = collection.GetCount().map_err(|e| format!("GetCount: {e}"))?;

        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count {
            let device = match collection.Item(i) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let id = match device_id_string(&device) {
                Ok(id) => id,
                Err(_) => continue,
            };
            let name = device_friendly_name(&device);
            let is_default = default_id.as_ref() == Some(&id);
            out.push(AudioOutputDevice {
                id,
                name,
                is_default,
            });
        }
        out.sort_by(|a, b| match (a.is_default, b.is_default) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });
        Ok(out)
    }
}

pub fn set_default_output(device_id: &str) -> Result<VolumeSnapshot, String> {
    if device_id.trim().is_empty() {
        return Err("device id required".into());
    }
    ensure_com();
    let wide: Vec<u16> = device_id
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let pcw = PCWSTR(wide.as_ptr());

    let policy: IPolicyConfig = unsafe {
        CoCreateInstance(&CLSID_POLICY_CONFIG, None, CLSCTX_ALL)
            .map_err(|e| format!("PolicyConfigClient: {e}"))?
    };

    // Match Sound settings: Console + Multimedia + Communications.
    for role in [eConsole, eMultimedia, eCommunications] {
        unsafe {
            policy
                .SetDefaultEndpoint(pcw, role)
                .ok()
                .map_err(|e| format!("SetDefaultEndpoint: {e}"))?;
        }
    }

    std::thread::sleep(std::time::Duration::from_millis(40));
    let snap = snapshot();
    crate::win32::system_monitor::patch_volume(snap.clone());
    Ok(snap)
}

/// Soft system beep so the user can hear the current level.
pub fn play_preview() -> Result<(), String> {
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC, SND_NODEFAULT};
    unsafe {
        let ok = PlaySoundW(
            windows::core::w!("SystemDefault"),
            None,
            SND_ALIAS | SND_ASYNC | SND_NODEFAULT,
        );
        if !ok.as_bool() {
            let _ = PlaySoundW(
                windows::core::w!("SystemAsterisk"),
                None,
                SND_ALIAS | SND_ASYNC | SND_NODEFAULT,
            );
        }
    }
    Ok(())
}

pub fn open_sound_settings() -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    unsafe {
        let r = ShellExecuteW(
            HWND::default(),
            windows::core::w!("open"),
            &HSTRING::from("ms-settings:sound"),
            None,
            None,
            SW_SHOWNORMAL,
        );
        if (r.0 as isize) <= 32 {
            return Err(format!(
                "ShellExecute sound settings failed ({})",
                r.0 as isize
            ));
        }
    }
    Ok(())
}
