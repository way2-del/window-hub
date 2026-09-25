//! Bluetooth radio toggle + paired device list for the host control center.
use serde::Serialize;
use windows::core::HSTRING;
use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothDevice};
use windows::Devices::Enumeration::DeviceInformation;
use windows::Devices::Radios::{Radio, RadioKind, RadioState};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BluetoothDeviceInfo {
    pub id: String,
    pub name: String,
    pub connected: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BluetoothState {
    pub enabled: bool,
    pub available: bool,
    pub devices: Vec<BluetoothDeviceInfo>,
}

fn with_com<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let result = f();
        CoUninitialize();
        result
    }
}

fn bluetooth_radio() -> Result<Option<Radio>, String> {
    let op = Radio::GetRadiosAsync().map_err(|e| e.to_string())?;
    let radios = op.get().map_err(|e| e.to_string())?;
    let count = radios.Size().map_err(|e| e.to_string())?;
    for i in 0..count {
        let radio = radios.GetAt(i).map_err(|e| e.to_string())?;
        if radio.Kind().map_err(|e| e.to_string())? == RadioKind::Bluetooth {
            return Ok(Some(radio));
        }
    }
    Ok(None)
}

fn list_paired_devices() -> Vec<BluetoothDeviceInfo> {
    let Ok(selector) = BluetoothDevice::GetDeviceSelectorFromPairingState(true) else {
        return Vec::new();
    };
    let Ok(op) = DeviceInformation::FindAllAsyncAqsFilter(&selector) else {
        return Vec::new();
    };
    let Ok(collection) = op.get() else {
        return Vec::new();
    };
    let Ok(count) = collection.Size() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let Ok(info) = collection.GetAt(i) else {
            continue;
        };
        let id = info.Id().map(|s| s.to_string()).unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let name = info
            .Name()
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "蓝牙设备".into());
        let connected = BluetoothDevice::FromIdAsync(&HSTRING::from(id.as_str()))
            .ok()
            .and_then(|op| op.get().ok())
            .and_then(|dev| {
                dev.ConnectionStatus()
                    .ok()
                    .map(|s| s == BluetoothConnectionStatus::Connected)
            })
            .unwrap_or(false);
        out.push(BluetoothDeviceInfo { id, name, connected });
    }
    out.sort_by(|a, b| match (a.connected, b.connected) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    out
}

pub fn bluetooth_state() -> Result<BluetoothState, String> {
    with_com(|| {
        let radio = bluetooth_radio()?;
        let Some(radio) = radio else {
            return Ok(BluetoothState {
                enabled: false,
                available: false,
                devices: Vec::new(),
            });
        };
        let enabled = radio.State().map_err(|e| e.to_string())? == RadioState::On;
        let devices = if enabled {
            list_paired_devices()
        } else {
            Vec::new()
        };
        Ok(BluetoothState {
            enabled,
            available: true,
            devices,
        })
    })
}

pub fn set_bluetooth_enabled(enabled: bool) -> Result<BluetoothState, String> {
    with_com(|| {
        let radio = bluetooth_radio()?.ok_or_else(|| "未找到蓝牙适配器".to_string())?;
        let target = if enabled {
            RadioState::On
        } else {
            RadioState::Off
        };
        if radio.State().map_err(|e| e.to_string())? != target {
            radio
                .SetStateAsync(target)
                .map_err(|e| e.to_string())?
                .get()
                .map_err(|e| e.to_string())?;
        }
        // Radio state can lag briefly after SetStateAsync.
        std::thread::sleep(std::time::Duration::from_millis(120));
        let enabled_now = radio.State().map_err(|e| e.to_string())? == RadioState::On;
        let devices = if enabled_now {
            list_paired_devices()
        } else {
            Vec::new()
        };
        Ok(BluetoothState {
            enabled: enabled_now,
            available: true,
            devices,
        })
    })
}
