//! Device controls used only by the host control center.
use serde::Serialize;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator,
    MMDeviceEnumerator,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioState {
    volume: Option<u8>,
    muted: bool,
    mic_muted: Option<bool>,
}

fn with_endpoint<T>(
    capture: bool,
    f: impl FnOnce(IAudioEndpointVolume) -> windows::core::Result<T>,
) -> Result<T, String> {
    unsafe {
        // Commands run on blocking workers; balance COM even when device access fails.
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|e| e.to_string())?;
        let result = (|| {
            let devices: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = devices
                .GetDefaultAudioEndpoint(if capture { eCapture } else { eRender }, eConsole)?;
            f(device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)?)
        })();
        CoUninitialize();
        result.map_err(|e: windows::core::Error| e.to_string())
    }
}

pub fn audio_state() -> AudioState {
    let output = with_endpoint(false, |e| unsafe {
        Ok((e.GetMasterVolumeLevelScalar()?, e.GetMute()?.as_bool()))
    });
    AudioState {
        volume: output.as_ref().ok().map(|(v, _)| (v * 100.0).round() as u8),
        muted: output.map(|(_, m)| m).unwrap_or(false),
        mic_muted: with_endpoint(true, |e| unsafe { Ok(e.GetMute()?.as_bool()) }).ok(),
    }
}

pub fn set_volume(value: u8) -> Result<(), String> {
    with_endpoint(false, |e| unsafe {
        e.SetMasterVolumeLevelScalar(f32::from(value.min(100)) / 100.0, std::ptr::null())?;
        e.SetMute(false, std::ptr::null())
    })
}

pub fn set_mic_muted(muted: bool) -> Result<(), String> {
    with_endpoint(true, |e| unsafe { e.SetMute(muted, std::ptr::null()) })
}

// WMI exposes laptop panel brightness. External displays without this provider
// return None: the UI disables the slider instead of presenting a fictitious value.
pub fn brightness(value: Option<u8>) -> Result<Option<u8>, String> {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let script = match value {
        Some(v) => format!("$ErrorActionPreference='Stop'; $m=Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods | Where-Object Active | Select-Object -First 1; if(!$m){{throw 'Brightness unavailable'}}; $r=Invoke-CimMethod -InputObject $m -MethodName WmiSetBrightness -Arguments @{{Timeout=[uint32]0;Brightness=[byte]{}}}; if($r.ReturnValue -ne 0){{throw 'Brightness update failed'}}; Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightness | Where-Object Active | Select-Object -First 1 -ExpandProperty CurrentBrightness", v.min(100)),
        None => "$ErrorActionPreference='Stop'; Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightness | Where-Object Active | Select-Object -First 1 -ExpandProperty CurrentBrightness".into(),
    };
    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    loop {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("读取显示器亮度超时".into());
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("此显示器不支持系统亮度调节".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u8>()
        .ok())
}

pub fn open_settings(page: &str) -> Result<(), String> {
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::HWND,
            UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        },
    };
    let uri = match page {
        "bluetooth" => "ms-settings:bluetooth",
        "hotspot" => "ms-settings:network-mobilehotspot",
        "display" => "ms-settings:display",
        "sound" => "ms-settings:sound",
        _ => return Err("Unknown control center destination".into()),
    };
    let uri: Vec<u16> = uri.encode_utf16().chain(Some(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            HWND::default(),
            windows::core::w!("open"),
            PCWSTR(uri.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        return Err("无法打开系统设置".into());
    }
    Ok(())
}
