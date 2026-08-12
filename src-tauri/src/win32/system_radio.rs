//! WiFi / Bluetooth / IME / peripheral collectors.
//! Heavy work is scheduled by `system_monitor` — do not call scan_* from UI/IPC paths.

#![cfg(windows)]

use serde::Serialize;
use std::collections::HashSet;
use std::net::{Ipv4Addr, Ipv6Addr};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WifiNetwork {
    pub ssid: String,
    pub signal: u32,
    pub secured: bool,
    pub connected: bool,
    /// Profile exists (password saved on this PC).
    pub saved: bool,
    /// Always None in list scans — use `read_wifi_password_for_ssid` on demand.
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WifiSnapshot {
    pub radio_on: bool,
    pub connected_ssid: Option<String>,
    pub ipv4: Vec<String>,
    pub ipv6: Vec<String>,
    pub networks: Vec<WifiNetwork>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BtDevice {
    pub id: String,
    pub name: String,
    pub connected: bool,
    pub kind: String,
    /// Battery 0–100 when reported by the OS / device; None if unknown.
    pub battery: Option<u8>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BluetoothSnapshot {
    pub radio_on: bool,
    pub devices: Vec<BtDevice>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PeripheralIcon {
    pub id: String,
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImeSnapshot {
    pub name: String,
    pub layout: String,
    /// Compact chip: 中 / a / A / あ / 한 …
    pub mark: String,
    /// zh | en | other — Chinese IME conversion mode when applicable
    pub mode: String,
    pub caps: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemRadioSnapshot {
    pub wifi: WifiSnapshot,
    pub bluetooth: BluetoothSnapshot,
    pub ime: ImeSnapshot,
    pub volume: crate::win32::system_audio::VolumeSnapshot,
    pub power: crate::win32::system_power::PowerSnapshot,
    pub perf: crate::win32::system_perf::PerfSnapshot,
    pub peripherals: Vec<PeripheralIcon>,
}

pub fn empty_wifi() -> WifiSnapshot {
    WifiSnapshot {
        radio_on: false,
        connected_ssid: None,
        ipv4: Vec::new(),
        ipv6: Vec::new(),
        networks: Vec::new(),
    }
}

pub fn empty_bt() -> BluetoothSnapshot {
    BluetoothSnapshot {
        radio_on: false,
        devices: Vec::new(),
    }
}

pub fn empty_ime() -> ImeSnapshot {
    ImeSnapshot {
        name: "输入法".into(),
        layout: String::new(),
        mark: "—".into(),
        mode: "en".into(),
        caps: false,
    }
}

fn wide_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
        .trim()
        .trim_end_matches('\0')
        .to_string()
}

fn ssid_to_string(ssid: &windows::Win32::NetworkManagement::WiFi::DOT11_SSID) -> String {
    let len = ssid.uSSIDLength.min(32) as usize;
    String::from_utf8_lossy(&ssid.ucSSID[..len])
        .trim()
        .to_string()
}

fn adapter_ips() -> (Vec<String>, Vec<String>) {
    use windows::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_INCLUDE_PREFIX, GAA_FLAG_SKIP_ANYCAST,
        GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IF_TYPE_IEEE80211,
        IP_ADAPTER_ADDRESSES_LH,
    };
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows::Win32::Networking::WinSock::{
        AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR_IN, SOCKADDR_IN6,
    };

    let mut ipv4 = Vec::new();
    let mut ipv6 = Vec::new();

    unsafe {
        let mut size = 0u32;
        let flags =
            GAA_FLAG_INCLUDE_PREFIX | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
        let first = GetAdaptersAddresses(AF_UNSPEC.0 as u32, flags, None, None, &mut size);
        // ERROR_BUFFER_OVERFLOW == 111
        if first != 111 {
            return (ipv4, ipv6);
        }
        let mut buf = vec![0u8; size as usize];
        let head = buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
        let rc = GetAdaptersAddresses(AF_UNSPEC.0 as u32, flags, None, Some(head), &mut size);
        if rc != 0 {
            return (ipv4, ipv6);
        }

        let mut cur = head;
        while !cur.is_null() {
            let adapter = &*cur;
            // WiFi flyout: only active IEEE 802.11 adapters (skip Ethernet / VPN / Hyper-V / etc.)
            if adapter.IfType != IF_TYPE_IEEE80211 || adapter.OperStatus != IfOperStatusUp {
                cur = adapter.Next;
                continue;
            }
            let mut uni = adapter.FirstUnicastAddress;
            while !uni.is_null() {
                let ua = &*uni;
                if !ua.Address.lpSockaddr.is_null() {
                    let sa = ua.Address.lpSockaddr;
                    match (*sa).sa_family {
                        AF_INET => {
                            let sin = &*(sa as *const SOCKADDR_IN);
                            let octets = u32::from_be(sin.sin_addr.S_un.S_addr).to_be_bytes();
                            let addr = Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]);
                            if !addr.is_loopback() && !addr.is_link_local() && !addr.is_unspecified() {
                                let s = addr.to_string();
                                if !ipv4.contains(&s) {
                                    ipv4.push(s);
                                }
                            }
                        }
                        AF_INET6 => {
                            let sin6 = &*(sa as *const SOCKADDR_IN6);
                            let bytes = sin6.sin6_addr.u.Byte;
                            let addr = Ipv6Addr::from(bytes);
                            if !addr.is_loopback()
                                && !addr.is_unspecified()
                                && !addr.is_unicast_link_local()
                            {
                                let s = addr.to_string();
                                if !ipv6.contains(&s) {
                                    ipv6.push(s);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                uni = ua.Next;
            }
            cur = adapter.Next;
        }
    }

    (ipv4, ipv6)
}

/// Scan visible + saved SSIDs. Never reads plaintext passwords (no WlanGetProfile key).
/// `trigger_scan` issues WlanScan but does **not** sleep waiting for results.
pub fn scan_wifi_list(trigger_scan: bool) -> WifiSnapshot {
    use windows::core::GUID;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::NetworkManagement::WiFi::{
        WlanCloseHandle, WlanEnumInterfaces, WlanFreeMemory, WlanGetAvailableNetworkList,
        WlanGetProfileList, WlanOpenHandle, WlanScan, WLAN_AVAILABLE_NETWORK,
        WLAN_AVAILABLE_NETWORK_LIST, WLAN_INTERFACE_INFO_LIST, WLAN_INTERFACE_STATE,
        WLAN_PROFILE_INFO_LIST,
    };

    let (ipv4, ipv6) = adapter_ips();

    unsafe {
        let mut ver = 0u32;
        let mut client = HANDLE::default();
        if WlanOpenHandle(2, None, &mut ver, &mut client) != 0 {
            return WifiSnapshot {
                radio_on: false,
                connected_ssid: None,
                ipv4,
                ipv6,
                networks: Vec::new(),
            };
        }

        let mut list_ptr: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        let enum_rc = WlanEnumInterfaces(client, None, &mut list_ptr);
        if enum_rc != 0 || list_ptr.is_null() {
            let _ = WlanCloseHandle(client, None);
            return WifiSnapshot {
                radio_on: false,
                connected_ssid: None,
                ipv4,
                ipv6,
                networks: Vec::new(),
            };
        }

        let list = &*list_ptr;
        if list.dwNumberOfItems == 0 {
            WlanFreeMemory(list_ptr as *mut _);
            let _ = WlanCloseHandle(client, None);
            return WifiSnapshot {
                radio_on: false,
                connected_ssid: None,
                ipv4,
                ipv6,
                networks: Vec::new(),
            };
        }

        let iface = &list.InterfaceInfo[0];
        let guid: GUID = iface.InterfaceGuid;
        let radio_on = iface.isState != WLAN_INTERFACE_STATE(0);
        if trigger_scan {
            let _ = WlanScan(client, &guid, None, None, None);
        }

        // Saved profiles (including those not currently in range) — names only.
        let mut saved_names: HashSet<String> = HashSet::new();
        let mut profile_ptr: *mut WLAN_PROFILE_INFO_LIST = std::ptr::null_mut();
        if WlanGetProfileList(client, &guid, None, &mut profile_ptr) == 0 && !profile_ptr.is_null()
        {
            let profiles = &*profile_ptr;
            let count = profiles.dwNumberOfItems as usize;
            let first = profiles.ProfileInfo.as_ptr();
            for i in 0..count {
                let p = &*first.add(i);
                let name = wide_to_string(&p.strProfileName);
                if !name.is_empty() {
                    saved_names.insert(name);
                }
            }
            WlanFreeMemory(profile_ptr as *mut _);
        }

        let mut net_ptr: *mut WLAN_AVAILABLE_NETWORK_LIST = std::ptr::null_mut();
        let net_rc = WlanGetAvailableNetworkList(client, &guid, 0, None, &mut net_ptr);
        let mut networks = Vec::new();
        let mut connected_ssid = None::<String>;
        let mut seen = HashSet::new();

        if net_rc == 0 && !net_ptr.is_null() {
            let nets = &*net_ptr;
            let count = nets.dwNumberOfItems as usize;
            let first = nets.Network.as_ptr();
            for i in 0..count {
                let n: WLAN_AVAILABLE_NETWORK = *first.add(i);
                let ssid = ssid_to_string(&n.dot11Ssid);
                if ssid.is_empty() || !seen.insert(ssid.clone()) {
                    continue;
                }
                let connected = n.dwFlags & 1 != 0;
                // WLAN_AVAILABLE_NETWORK_HAS_PROFILE = 0x2
                let has_profile_flag = n.dwFlags & 2 != 0;
                let saved = has_profile_flag || saved_names.contains(&ssid);
                if connected {
                    connected_ssid = Some(ssid.clone());
                }
                networks.push(WifiNetwork {
                    ssid,
                    signal: n.wlanSignalQuality.min(100),
                    secured: n.bSecurityEnabled.as_bool(),
                    connected,
                    saved,
                    password: None,
                });
            }
            WlanFreeMemory(net_ptr as *mut _);
        }

        // Only nearby / currently visible networks — do not append out-of-range saved profiles.

        networks.sort_by(|a, b| {
            b.connected
                .cmp(&a.connected)
                .then(b.saved.cmp(&a.saved))
                .then(b.signal.cmp(&a.signal))
                .then(a.ssid.to_lowercase().cmp(&b.ssid.to_lowercase()))
        });

        WlanFreeMemory(list_ptr as *mut _);
        let _ = WlanCloseHandle(client, None);

        WifiSnapshot {
            radio_on: radio_on || connected_ssid.is_some() || !networks.is_empty(),
            connected_ssid,
            ipv4,
            ipv6,
            networks,
        }
    }
}

/// On-demand plaintext key for a single saved profile. Call only from explicit user action.
pub fn read_wifi_password_for_ssid(ssid: &str) -> Option<String> {
    use windows::core::GUID;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::NetworkManagement::WiFi::{
        WlanCloseHandle, WlanEnumInterfaces, WlanFreeMemory, WlanOpenHandle,
        WLAN_INTERFACE_INFO_LIST,
    };

    let ssid = ssid.trim();
    if ssid.is_empty() {
        return None;
    }

    unsafe {
        let mut ver = 0u32;
        let mut client = HANDLE::default();
        if WlanOpenHandle(2, None, &mut ver, &mut client) != 0 {
            return None;
        }
        let mut list_ptr: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        if WlanEnumInterfaces(client, None, &mut list_ptr) != 0 || list_ptr.is_null() {
            let _ = WlanCloseHandle(client, None);
            return None;
        }
        let list = &*list_ptr;
        if list.dwNumberOfItems == 0 {
            WlanFreeMemory(list_ptr as *mut _);
            let _ = WlanCloseHandle(client, None);
            return None;
        }
        let guid: GUID = list.InterfaceInfo[0].InterfaceGuid;
        let pw = read_wifi_password(client, &guid, ssid);
        WlanFreeMemory(list_ptr as *mut _);
        let _ = WlanCloseHandle(client, None);
        pw
    }
}

fn read_wifi_password(
    client: windows::Win32::Foundation::HANDLE,
    guid: &windows::core::GUID,
    profile: &str,
) -> Option<String> {
    use windows::core::{HSTRING, PCWSTR, PWSTR};
    use windows::Win32::NetworkManagement::WiFi::{WlanFreeMemory, WlanGetProfile, WLAN_PROFILE_GET_PLAINTEXT_KEY};

    unsafe {
        let name = HSTRING::from(profile);
        let mut xml: PWSTR = PWSTR::null();
        let mut flags = WLAN_PROFILE_GET_PLAINTEXT_KEY;
        let rc = WlanGetProfile(
            client,
            guid,
            PCWSTR(name.as_ptr()),
            None,
            &mut xml,
            Some(&mut flags),
            None,
        );
        if rc != 0 || xml.is_null() {
            // Retry without plaintext (marks saved but no key).
            return None;
        }
        let wide = {
            let mut len = 0usize;
            while *xml.0.add(len) != 0 {
                len += 1;
                if len > 1_000_000 {
                    break;
                }
            }
            std::slice::from_raw_parts(xml.0, len)
        };
        let text = String::from_utf16_lossy(wide);
        WlanFreeMemory(xml.0 as *mut _);
        extract_key_material(&text)
    }
}

fn extract_key_material(xml: &str) -> Option<String> {
    // <keyMaterial>...</keyMaterial>
    let start = xml.find("<keyMaterial>")? + "<keyMaterial>".len();
    let end = xml[start..].find("</keyMaterial>")? + start;
    let key = xml[start..end].trim();
    if key.is_empty() {
        return None;
    }
    // Encrypted blob when not elevated — usually long hex; still show if short-ish passphrase.
    if key.len() > 128 && key.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(key.to_string())
}

pub fn scan_bluetooth() -> BluetoothSnapshot {
    use windows::Win32::Devices::Bluetooth::{
        BluetoothFindDeviceClose, BluetoothFindFirstDevice, BluetoothFindNextDevice,
        BLUETOOTH_DEVICE_INFO, BLUETOOTH_DEVICE_SEARCH_PARAMS,
    };
    use windows::Win32::Foundation::HANDLE;

    unsafe {
        let params = BLUETOOTH_DEVICE_SEARCH_PARAMS {
            dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32,
            fReturnAuthenticated: true.into(),
            fReturnRemembered: true.into(),
            fReturnUnknown: false.into(),
            fReturnConnected: true.into(),
            fIssueInquiry: false.into(),
            cTimeoutMultiplier: 0,
            hRadio: HANDLE::default(),
        };

        let mut info = BLUETOOTH_DEVICE_INFO {
            dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
            ..Default::default()
        };

        let find = match BluetoothFindFirstDevice(&params, &mut info) {
            Ok(h) => h,
            Err(_) => {
                return empty_bt();
            }
        };

        let mut devices = Vec::new();
        let mut any = true;
        while any {
            let name = wide_to_string(&info.szName);
            let addr = unsafe_bt_addr(&info);
            let connected = info.fConnected.as_bool();
            let kind = classify_bt_cod(info.ulClassofDevice);
            if !name.is_empty() {
                let battery = read_bt_battery(&addr);
                devices.push(BtDevice {
                    id: addr,
                    name,
                    connected,
                    kind,
                    battery,
                });
            }
            info = BLUETOOTH_DEVICE_INFO {
                dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
                ..Default::default()
            };
            any = BluetoothFindNextDevice(find, &mut info).is_ok();
        }
        let _ = BluetoothFindDeviceClose(find);

        // Deduplicate by address (stacks sometimes return the same device twice).
        let mut seen = HashSet::new();
        devices.retain(|d| seen.insert(d.id.clone()));

        devices.sort_by(|a, b| {
            b.connected
                .cmp(&a.connected)
                .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        BluetoothSnapshot {
            radio_on: true,
            devices,
        }
    }
}

fn read_bt_battery(addr_hex: &str) -> Option<u8> {
    // Best-effort: OEM / stack may store a percent under BTHPORT Devices\<addr>.
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let key_path = format!(
        r"SYSTEM\CurrentControlSet\Services\BTHPORT\Parameters\Devices\{}",
        addr_hex.to_ascii_lowercase()
    );
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let Ok(key) = hklm.open_subkey(key_path) else {
        return None;
    };
    for name in ["BatteryPercent", "Battery", "batteryPercent", "LastBattery"] {
        if let Ok(v) = key.get_value::<u32, _>(name) {
            if v <= 100 {
                return Some(v as u8);
            }
        }
    }
    None
}

fn bt_device_connected(addr_hex: &str) -> Option<bool> {
    use windows::Win32::Devices::Bluetooth::{
        BluetoothFindDeviceClose, BluetoothFindFirstDevice, BluetoothFindNextDevice,
        BLUETOOTH_DEVICE_INFO, BLUETOOTH_DEVICE_SEARCH_PARAMS,
    };
    use windows::Win32::Foundation::HANDLE;

    let target = addr_hex.trim().to_ascii_uppercase();
    unsafe {
        let params = BLUETOOTH_DEVICE_SEARCH_PARAMS {
            dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32,
            fReturnAuthenticated: true.into(),
            fReturnRemembered: true.into(),
            fReturnUnknown: false.into(),
            fReturnConnected: true.into(),
            fIssueInquiry: false.into(),
            cTimeoutMultiplier: 0,
            hRadio: HANDLE::default(),
        };
        let mut info = BLUETOOTH_DEVICE_INFO {
            dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
            ..Default::default()
        };
        let find = BluetoothFindFirstDevice(&params, &mut info).ok()?;
        let mut any = true;
        while any {
            if unsafe_bt_addr(&info).eq_ignore_ascii_case(&target) {
                let connected = info.fConnected.as_bool();
                let _ = BluetoothFindDeviceClose(find);
                return Some(connected);
            }
            info = BLUETOOTH_DEVICE_INFO {
                dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
                ..Default::default()
            };
            any = BluetoothFindNextDevice(find, &mut info).is_ok();
        }
        let _ = BluetoothFindDeviceClose(find);
        None
    }
}

fn bt_service_state_ok(rc: u32) -> bool {
    // ERROR_SUCCESS, or already in requested state (E_INVALIDARG / ERROR_INVALID_PARAMETER).
    rc == 0 || rc == 0x8007_0057 || rc == 87
}

/// Prefer A2DP / Handsfree / HID for reconnect nudge — toggling every installed
/// service DISABLE→ENABLE causes multi-second PnP storms (flyout goes white).
fn bt_nudge_services(services: &[windows::core::GUID]) -> Vec<windows::core::GUID> {
    use windows::core::GUID;
    const PRIORITY: [u128; 4] = [
        0x0000110B_0000_1000_8000_00805F9B34FB, // AudioSink
        0x0000111E_0000_1000_8000_00805F9B34FB, // Handsfree
        0x00001108_0000_1000_8000_00805F9B34FB, // Headset
        0x00001124_0000_1000_8000_00805F9B34FB, // HID
    ];
    let mut out = Vec::new();
    for p in PRIORITY {
        let want = GUID::from_u128(p);
        if let Some(g) = services.iter().find(|s| **s == want) {
            out.push(*g);
            if out.len() >= 2 {
                break;
            }
        }
    }
    if out.is_empty() {
        if let Some(first) = services.first() {
            out.push(*first);
        }
    }
    out
}

/// Connect or disconnect a remembered Bluetooth device via common service GUIDs.
pub fn set_bluetooth_device(addr_hex: &str, connect: bool) -> Result<(), String> {
    use windows::core::GUID;
    use windows::Win32::Devices::Bluetooth::{
        BluetoothEnumerateInstalledServices, BluetoothFindDeviceClose, BluetoothFindFirstDevice,
        BluetoothFindFirstRadio, BluetoothFindNextDevice, BluetoothFindRadioClose,
        BluetoothGetDeviceInfo, BluetoothSetServiceState, BLUETOOTH_DEVICE_INFO,
        BLUETOOTH_DEVICE_SEARCH_PARAMS, BLUETOOTH_FIND_RADIO_PARAMS, BLUETOOTH_SERVICE_DISABLE,
        BLUETOOTH_SERVICE_ENABLE,
    };
    use windows::Win32::Foundation::{CloseHandle, HANDLE};

    let target = addr_hex.trim().to_ascii_uppercase();
    if target.is_empty() {
        return Err("device id empty".into());
    }

    // Classic BT profiles commonly bound by Windows audio / HID stacks.
    const SERVICES: [GUID; 8] = [
        GUID::from_u128(0x0000110B_0000_1000_8000_00805F9B34FB), // AudioSink (A2DP)
        GUID::from_u128(0x0000110A_0000_1000_8000_00805F9B34FB), // AudioSource
        GUID::from_u128(0x0000111E_0000_1000_8000_00805F9B34FB), // Handsfree
        GUID::from_u128(0x00001108_0000_1000_8000_00805F9B34FB), // Headset
        GUID::from_u128(0x0000110C_0000_1000_8000_00805F9B34FB), // AVRCP Target
        GUID::from_u128(0x0000110E_0000_1000_8000_00805F9B34FB), // AVRCP
        GUID::from_u128(0x00001124_0000_1000_8000_00805F9B34FB), // HID
        GUID::from_u128(0x00001200_0000_1000_8000_00805F9B34FB), // PnP
    ];

    unsafe {
        let radio_params = BLUETOOTH_FIND_RADIO_PARAMS {
            dwSize: std::mem::size_of::<BLUETOOTH_FIND_RADIO_PARAMS>() as u32,
        };
        let mut radio = HANDLE::default();
        let radio_find = BluetoothFindFirstRadio(&radio_params, &mut radio).ok();
        if radio.is_invalid() {
            if let Some(rf) = radio_find {
                let _ = BluetoothFindRadioClose(rf);
            }
            return Err("未找到蓝牙适配器".into());
        }
        let radio_handle = radio;

        let params = BLUETOOTH_DEVICE_SEARCH_PARAMS {
            dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32,
            fReturnAuthenticated: true.into(),
            fReturnRemembered: true.into(),
            fReturnUnknown: false.into(),
            fReturnConnected: true.into(),
            fIssueInquiry: false.into(),
            cTimeoutMultiplier: 0,
            hRadio: radio_handle,
        };
        let mut info = BLUETOOTH_DEVICE_INFO {
            dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
            ..Default::default()
        };
        let find = match BluetoothFindFirstDevice(&params, &mut info) {
            Ok(h) => h,
            Err(e) => {
                if let Some(rf) = radio_find {
                    let _ = BluetoothFindRadioClose(rf);
                }
                let _ = CloseHandle(radio_handle);
                return Err(format!("BluetoothFindFirstDevice: {e}"));
            }
        };

        let mut found_info: Option<BLUETOOTH_DEVICE_INFO> = None;
        let mut any = true;
        while any {
            let addr = unsafe_bt_addr(&info);
            if addr.eq_ignore_ascii_case(&target) {
                found_info = Some(info);
                break;
            }
            info = BLUETOOTH_DEVICE_INFO {
                dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
                ..Default::default()
            };
            any = BluetoothFindNextDevice(find, &mut info).is_ok();
        }
        let _ = BluetoothFindDeviceClose(find);

        let Some(mut dev) = found_info else {
            if let Some(rf) = radio_find {
                let _ = BluetoothFindRadioClose(rf);
            }
            let _ = CloseHandle(radio_handle);
            return Err("未找到该蓝牙设备".into());
        };

        // Refresh cached fields — SetServiceState is picky about a full DEVICE_INFO.
        let _ = BluetoothGetDeviceInfo(radio_handle, &mut dev);

        // Prefer services Windows already mapped for this device; fall back to common profiles.
        let mut services: Vec<GUID> = {
            let mut count: u32 = 0;
            let rc = BluetoothEnumerateInstalledServices(
                radio_handle,
                &dev,
                &mut count,
                None,
            );
            if (rc == 0 || rc == 234 /* ERROR_MORE_DATA */) && count > 0 && count < 64 {
                let mut buf = vec![GUID::default(); count as usize];
                let mut n = count;
                let rc2 = BluetoothEnumerateInstalledServices(
                    radio_handle,
                    &dev,
                    &mut n,
                    Some(buf.as_mut_ptr()),
                );
                if rc2 == 0 || rc2 == 234 {
                    buf.truncate(n as usize);
                    buf
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        };
        if services.is_empty() {
            services.extend_from_slice(&SERVICES);
        }

        let mut touched = false;
        if connect {
            // Fast path: ENABLE only (no driver tear-down).
            for svc in &services {
                let rc = BluetoothSetServiceState(
                    radio_handle,
                    &dev,
                    svc,
                    BLUETOOTH_SERVICE_ENABLE,
                );
                if bt_service_state_ok(rc) {
                    touched = true;
                }
            }
            // Already-enabled-but-disconnected: nudge at most 2 primary profiles.
            // Full-list DISABLE→ENABLE freezes DWM / turns the mica flyout white.
            let need_nudge = bt_device_connected(&target) != Some(true);
            if need_nudge {
                for svc in bt_nudge_services(&services) {
                    let _ = BluetoothSetServiceState(
                        radio_handle,
                        &dev,
                        &svc,
                        BLUETOOTH_SERVICE_DISABLE,
                    );
                    let rc = BluetoothSetServiceState(
                        radio_handle,
                        &dev,
                        &svc,
                        BLUETOOTH_SERVICE_ENABLE,
                    );
                    if bt_service_state_ok(rc) {
                        touched = true;
                    }
                }
            }
        } else {
            // Disconnect: disable mapped services only; if we fell back to the full
            // profile list, only touch the primary ones to avoid PnP white-out.
            let to_disable = if services.len() > 4 {
                bt_nudge_services(&services)
            } else {
                services.clone()
            };
            for svc in &to_disable {
                let rc = BluetoothSetServiceState(
                    radio_handle,
                    &dev,
                    svc,
                    BLUETOOTH_SERVICE_DISABLE,
                );
                if bt_service_state_ok(rc) {
                    touched = true;
                }
            }
        }

        if let Some(rf) = radio_find {
            let _ = BluetoothFindRadioClose(rf);
        }
        let _ = CloseHandle(radio_handle);

        crate::win32::system_monitor::invalidate_bluetooth();
        if touched {
            return Ok(());
        }
        if let Some(now) = bt_device_connected(&target) {
            if now == connect {
                return Ok(());
            }
        }

        Err(if connect {
            "无法连接（可在系统蓝牙设置中操作）".into()
        } else {
            "无法断开（可在系统蓝牙设置中操作）".into()
        })
    }
}

fn unsafe_bt_addr(info: &windows::Win32::Devices::Bluetooth::BLUETOOTH_DEVICE_INFO) -> String {
    // BLUETOOTH_ADDRESS is a union; prefer ullLong when available.
    let raw = unsafe { info.Address.Anonymous.ullLong };
    format!("{raw:012X}")
}

fn classify_bt_cod(cod: u32) -> String {
    let major = (cod >> 8) & 0x1f;
    match major {
        0x01 => "computer".into(),
        0x02 => "phone".into(),
        0x04 => "audio".into(),
        0x05 => "peripheral".into(),
        0x06 => "imaging".into(),
        0x07 => "wearable".into(),
        0x08 => "toy".into(),
        _ => "other".into(),
    }
}

pub fn collect_peripherals(bt: &BluetoothSnapshot) -> Vec<PeripheralIcon> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for d in &bt.devices {
        if !d.connected {
            continue;
        }
        let low = d.name.to_ascii_lowercase();
        let kind = if d.kind == "audio"
            || low.contains("headphone")
            || low.contains("headset")
            || low.contains("airpods")
            || d.name.contains("耳机")
            || d.name.contains("耳麦")
        {
            "headphones"
        } else if d.kind == "peripheral"
            || low.contains("controller")
            || low.contains("gamepad")
            || low.contains("xbox")
            || d.name.contains("手柄")
        {
            "gamepad"
        } else {
            continue;
        };
        let id = format!("bt-{kind}-{}", d.id);
        if seen.insert(id.clone()) {
            out.push(PeripheralIcon {
                id,
                kind: kind.into(),
                name: d.name.clone(),
            });
        }
    }

    for (i, name) in xinput_pads().into_iter().enumerate() {
        let id = format!("xinput-{i}");
        if seen.insert(id.clone()) {
            out.push(PeripheralIcon {
                id,
                kind: "gamepad".into(),
                name,
            });
        }
    }

    out
}

fn xinput_pads() -> Vec<String> {
    use windows::Win32::UI::Input::XboxController::{XInputGetState, XINPUT_STATE};

    let mut names = Vec::new();
    unsafe {
        for i in 0..4u32 {
            let mut state = XINPUT_STATE::default();
            if XInputGetState(i, &mut state) == 0 {
                names.push(format!("Xbox 控制器 {}", i + 1));
            }
        }
    }
    names
}

fn lang_id(layout_hex: &str) -> u16 {
    u16::from_str_radix(&layout_hex[layout_hex.len().saturating_sub(4)..], 16).unwrap_or(0)
}

fn ime_caps_on() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CAPITAL};
    unsafe { (GetKeyState(VK_CAPITAL.0 as i32) as u16 & 1) != 0 }
}

/// Keyboard layout of the *foreground* thread (not our sysmon worker thread).
fn foreground_keyboard_layout() -> windows::Win32::UI::Input::KeyboardAndMouse::HKL {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return GetKeyboardLayout(0);
        }
        let tid = GetWindowThreadProcessId(hwnd, None);
        if tid == 0 {
            return GetKeyboardLayout(0);
        }
        GetKeyboardLayout(tid)
    }
}

const WM_IME_CONTROL: u32 = 0x0283;
const IMC_GETCONVERSIONMODE: usize = 0x0001;
const IMC_GETOPENSTATUS: usize = 0x0005;

/// True when foreground Chinese IME is in native (中) conversion mode.
/// Prefer ImmGetDefaultIMEWnd + WM_IME_CONTROL — MS Pinyin/TSF often fails ImmGetContext.
fn ime_native_mode(fg: windows::Win32::Foundation::HWND) -> Option<bool> {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::Input::Ime::{
        ImmGetContext, ImmGetConversionStatus, ImmGetDefaultIMEWnd, ImmGetOpenStatus,
        ImmReleaseContext, IME_CMODE_NATIVE, IME_CONVERSION_MODE, IME_SENTENCE_MODE,
    };
    use windows::Win32::UI::WindowsAndMessaging::SendMessageW;

    unsafe {
        if fg.0.is_null() {
            return None;
        }

        let ime_hwnd = ImmGetDefaultIMEWnd(fg);
        if !ime_hwnd.0.is_null() {
            let open = SendMessageW(
                ime_hwnd,
                WM_IME_CONTROL,
                WPARAM(IMC_GETOPENSTATUS),
                LPARAM(0),
            )
            .0;
            let conv = SendMessageW(
                ime_hwnd,
                WM_IME_CONTROL,
                WPARAM(IMC_GETCONVERSIONMODE),
                LPARAM(0),
            )
            .0 as u32;
            // Closed IME / English mode → not native. Open + NATIVE → 中.
            if open == 0 {
                return Some(false);
            }
            return Some((conv & IME_CMODE_NATIVE.0) != 0);
        }

        let himc = ImmGetContext(fg);
        if himc.0.is_null() {
            return None;
        }
        let open = ImmGetOpenStatus(himc).as_bool();
        let mut conv = IME_CONVERSION_MODE(0);
        let mut sentence = IME_SENTENCE_MODE(0);
        let ok = ImmGetConversionStatus(himc, Some(&mut conv), Some(&mut sentence)).as_bool();
        let _ = ImmReleaseContext(fg, himc);
        if !ok {
            return None;
        }
        Some(open && (conv.0 & IME_CMODE_NATIVE.0) != 0)
    }
}

fn en_mark(caps: bool) -> String {
    if caps {
        "A".into()
    } else {
        "a".into()
    }
}

pub fn current_ime() -> ImeSnapshot {
    use windows::Win32::UI::Input::Ime::ImmGetDescriptionW;
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    unsafe {
        let fg = GetForegroundWindow();
        let hkl = foreground_keyboard_layout();
        let layout = format!("{:08X}", hkl.0 as usize as u32);
        let mut buf = [0u16; 128];
        let n = ImmGetDescriptionW(hkl, Some(&mut buf));
        let name = if n > 0 {
            wide_to_string(&buf[..n as usize])
        } else {
            String::new()
        };
        let name = if name.is_empty() {
            match lang_id(&layout) {
                0x0804 => "中文(简体)".into(),
                0x0404 | 0x0C04 | 0x1404 => "中文(繁體)".into(),
                0x0409 => "English (US)".into(),
                0x0809 => "English (UK)".into(),
                0x0411 => "日本語".into(),
                0x0412 => "한국어".into(),
                _ => "输入法".into(),
            }
        } else {
            name
        };

        let lid = lang_id(&layout);
        let caps = ime_caps_on();
        // Chip: 中 / a / A（英文区分大小写）；勿在检测失败时默认「中」
        let (mode, mark) = match lid {
            0x0804 | 0x0404 | 0x0C04 | 0x1404 => {
                let native = ime_native_mode(fg).unwrap_or(false);
                if native {
                    ("zh".into(), "中".into())
                } else {
                    ("en".into(), en_mark(caps))
                }
            }
            0x0411 => ("other".into(), "あ".into()),
            0x0412 => ("other".into(), "한".into()),
            _ => ("en".into(), en_mark(caps)),
        };

        ImeSnapshot {
            name,
            layout,
            mark,
            mode,
            caps,
        }
    }
}

pub fn open_ime_picker() -> Result<(), String> {
    // Win+Space cycles / opens language input switcher on Win10/11.
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
        VK_LWIN, VK_SPACE,
    };

    unsafe fn stroke(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        Default::default()
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    unsafe {
        let inputs = [
            stroke(VK_LWIN, false),
            stroke(VK_SPACE, false),
            stroke(VK_SPACE, true),
            stroke(VK_LWIN, true),
        ];
        let sent = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        if sent as usize != inputs.len() {
            return Err(format!("SendInput Win+Space failed ({sent})"));
        }
    }
    Ok(())
}

pub fn connect_wifi_profile(ssid: &str) -> Result<(), String> {
    use windows::core::GUID;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::NetworkManagement::WiFi::{
        WlanCloseHandle, WlanConnect, WlanEnumInterfaces, WlanFreeMemory, WlanOpenHandle,
        DOT11_SSID, WLAN_CONNECTION_PARAMETERS, WLAN_INTERFACE_INFO_LIST,
        dot11_BSS_type_infrastructure, wlan_connection_mode_profile,
    };

    let ssid = ssid.trim();
    if ssid.is_empty() {
        return Err("ssid empty".into());
    }

    unsafe {
        let mut ver = 0u32;
        let mut client = HANDLE::default();
        if WlanOpenHandle(2, None, &mut ver, &mut client) != 0 {
            return Err("WlanOpenHandle failed".into());
        }
        let mut list_ptr: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        if WlanEnumInterfaces(client, None, &mut list_ptr) != 0 || list_ptr.is_null() {
            let _ = WlanCloseHandle(client, None);
            return Err("no wifi interface".into());
        }
        let list = &*list_ptr;
        if list.dwNumberOfItems == 0 {
            WlanFreeMemory(list_ptr as *mut _);
            let _ = WlanCloseHandle(client, None);
            return Err("no wifi interface".into());
        }
        let guid: GUID = list.InterfaceInfo[0].InterfaceGuid;

        let profile: Vec<u16> = ssid.encode_utf16().chain(std::iter::once(0)).collect();
        let mut dot11 = DOT11_SSID::default();
        let bytes = ssid.as_bytes();
        let len = bytes.len().min(32);
        dot11.uSSIDLength = len as u32;
        dot11.ucSSID[..len].copy_from_slice(&bytes[..len]);

        let params = WLAN_CONNECTION_PARAMETERS {
            wlanConnectionMode: wlan_connection_mode_profile,
            strProfile: windows::core::PCWSTR(profile.as_ptr()),
            pDot11Ssid: std::ptr::from_ref(&dot11) as *mut _,
            pDesiredBssidList: std::ptr::null_mut(),
            dot11BssType: dot11_BSS_type_infrastructure,
            dwFlags: 0,
        };

        let rc = WlanConnect(client, &guid, &params, None);
        WlanFreeMemory(list_ptr as *mut _);
        let _ = WlanCloseHandle(client, None);
        if rc != 0 {
            return Err(format!(
                "连接失败（可能尚未保存配置）。请到系统设置完成首次连接。code={rc}"
            ));
        }
        crate::win32::system_monitor::invalidate_wifi();
        Ok(())
    }
}

pub fn open_wifi_settings() -> Result<(), String> {
    open_uri("ms-settings:network-wifi")
}

pub fn open_bluetooth_settings() -> Result<(), String> {
    open_uri("ms-settings:bluetooth")
}

pub fn open_ime_settings() -> Result<(), String> {
    open_uri("ms-settings:regionlanguage")
}

fn open_uri(uri: &str) -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    unsafe {
        let rc = ShellExecuteW(
            HWND::default(),
            windows::core::w!("open"),
            &HSTRING::from(uri),
            None,
            None,
            SW_SHOWNORMAL,
        );
        if (rc.0 as isize) <= 32 {
            return Err(format!("ShellExecute failed for {uri}"));
        }
    }
    Ok(())
}
