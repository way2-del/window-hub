//! Host-owned Wi‑Fi indicator (Shell WLAN chrome vanishes with the taskbar).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct WifiState {
    /// Radio / software Wi‑Fi enabled.
    pub enabled: bool,
    pub connected: bool,
    pub ssid: String,
    /// 0–100 link quality.
    pub signal: u32,
    pub ip: String,
    /// Approximate link speed in Mbps.
    pub link_mbps: u32,
    pub mac: String,
    pub secured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WifiNetwork {
    pub ssid: String,
    pub signal: u32,
    pub secured: bool,
    pub connected: bool,
    /// Saved profile exists — can connect without a password prompt.
    pub has_profile: bool,
    /// Windows profile name when known (may differ from SSID).
    #[serde(default)]
    pub profile_name: String,
    /// Profile auth hint: `open` | `wpapsk` | `wpa2psk` | `wpa3sae`.
    #[serde(default)]
    pub auth: String,
}

#[cfg(windows)]
mod win {
    use super::{WifiNetwork, WifiState};
    use parking_lot::Mutex;
    use std::sync::{LazyLock, OnceLock};
    use std::time::Duration;
    use windows::core::{GUID, HSTRING, PCWSTR};
    use windows::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS, BOOL, HANDLE, HWND};
    use windows::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
        GAA_FLAG_SKIP_MULTICAST, IF_TYPE_IEEE80211, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows::Win32::NetworkManagement::WiFi::{
        dot11_BSS_type_infrastructure, dot11_radio_state_off, dot11_radio_state_on,
        wlan_connection_mode_discovery_unsecure, wlan_connection_mode_profile,
        wlan_intf_opcode_current_connection, wlan_intf_opcode_radio_state,
        wlan_interface_state_connected, DOT11_AUTH_ALGO_80211_OPEN, DOT11_AUTH_ALGO_RSNA_PSK,
        DOT11_AUTH_ALGO_WPA3_SAE, DOT11_AUTH_ALGO_WPA_PSK, DOT11_SSID, WLAN_AVAILABLE_NETWORK,
        WLAN_AVAILABLE_NETWORK_CONNECTED, WLAN_AVAILABLE_NETWORK_HAS_PROFILE,
        WLAN_AVAILABLE_NETWORK_LIST, WLAN_CONNECTION_ATTRIBUTES, WLAN_CONNECTION_PARAMETERS,
        WLAN_INTERFACE_INFO, WLAN_INTERFACE_INFO_LIST, WLAN_PHY_RADIO_STATE, WLAN_RADIO_STATE,
        WlanCloseHandle, WlanConnect, WlanDisconnect, WlanEnumInterfaces, WlanFreeMemory,
        WlanGetAvailableNetworkList, WlanOpenHandle, WlanQueryInterface, WlanScan, WlanSetInterface,
        WlanSetProfile,
    };
    use windows::Win32::Networking::WinSock::{AF_INET, SOCKADDR, SOCKADDR_IN};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    static STATE: LazyLock<Mutex<WifiState>> = LazyLock::new(|| Mutex::new(WifiState::default()));
    static EMIT: OnceLock<Box<dyn Fn(WifiState) + Send + Sync>> = OnceLock::new();

    fn wchar_to_string(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end]).trim().to_string()
    }

    fn ssid_to_string(ssid: &DOT11_SSID) -> String {
        let len = (ssid.uSSIDLength as usize).min(ssid.ucSSID.len());
        String::from_utf8_lossy(&ssid.ucSSID[..len]).trim().to_string()
    }

    struct WlanClient {
        handle: HANDLE,
    }

    impl WlanClient {
        fn open() -> Result<Self, String> {
            unsafe {
                let mut negotiated = 0u32;
                let mut handle = HANDLE::default();
                let rc = WlanOpenHandle(2, None, &mut negotiated, &mut handle);
                if rc != ERROR_SUCCESS.0 {
                    return Err(format!("WlanOpenHandle failed: {rc}"));
                }
                Ok(Self { handle })
            }
        }
    }

    impl Drop for WlanClient {
        fn drop(&mut self) {
            unsafe {
                let _ = WlanCloseHandle(self.handle, None);
            }
        }
    }

    unsafe fn first_interface(client: &WlanClient) -> Result<(GUID, WLAN_INTERFACE_INFO), String> {
        let mut list_ptr: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        let rc = WlanEnumInterfaces(client.handle, None, &mut list_ptr);
        if rc != ERROR_SUCCESS.0 || list_ptr.is_null() {
            return Err(format!("WlanEnumInterfaces failed: {rc}"));
        }
        let list = &*list_ptr;
        if list.dwNumberOfItems == 0 {
            WlanFreeMemory(list_ptr as *const _);
            return Err("未找到无线网卡".into());
        }
        let base = list.InterfaceInfo.as_ptr();
        let info = *base;
        let guid = info.InterfaceGuid;
        WlanFreeMemory(list_ptr as *const _);
        Ok((guid, info))
    }

    unsafe fn query_radio_enabled(client: &WlanClient, guid: &GUID) -> bool {
        let mut data_size = 0u32;
        let mut data: *mut core::ffi::c_void = std::ptr::null_mut();
        let rc = WlanQueryInterface(
            client.handle,
            guid,
            wlan_intf_opcode_radio_state,
            None,
            &mut data_size,
            &mut data,
            None,
        );
        if rc != ERROR_SUCCESS.0 || data.is_null() {
            return true;
        }
        let radio = &*(data as *const WLAN_RADIO_STATE);
        let mut on = false;
        let n = radio.dwNumberOfPhys.min(64);
        for i in 0..n {
            let phy = radio.PhyRadioState[i as usize];
            if phy.dot11SoftwareRadioState == dot11_radio_state_on
                && phy.dot11HardwareRadioState != dot11_radio_state_off
            {
                on = true;
                break;
            }
        }
        WlanFreeMemory(data);
        on
    }

    unsafe fn query_connection(
        client: &WlanClient,
        guid: &GUID,
    ) -> Option<WLAN_CONNECTION_ATTRIBUTES> {
        let mut data_size = 0u32;
        let mut data: *mut core::ffi::c_void = std::ptr::null_mut();
        let rc = WlanQueryInterface(
            client.handle,
            guid,
            wlan_intf_opcode_current_connection,
            None,
            &mut data_size,
            &mut data,
            None,
        );
        if rc != ERROR_SUCCESS.0 || data.is_null() {
            return None;
        }
        let attrs = *(data as *const WLAN_CONNECTION_ATTRIBUTES);
        WlanFreeMemory(data);
        Some(attrs)
    }

    unsafe fn adapter_ip_mac_speed() -> (String, String, u32) {
        let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
        let mut size = 0u32;
        let probe = GetAdaptersAddresses(AF_INET.0 as u32, flags, None, None, &mut size);
        if size == 0
            && probe != ERROR_BUFFER_OVERFLOW.0
            && probe != ERROR_SUCCESS.0
        {
            return (String::new(), String::new(), 0);
        }
        if size == 0 {
            return (String::new(), String::new(), 0);
        }
        let mut buf = vec![0u8; size as usize];
        let head = buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
        let rc = GetAdaptersAddresses(AF_INET.0 as u32, flags, None, Some(head), &mut size);
        if rc != ERROR_SUCCESS.0 {
            return (String::new(), String::new(), 0);
        }

        let mut cur = head;
        while !cur.is_null() {
            let a = &*cur;
            if a.IfType == IF_TYPE_IEEE80211 && a.OperStatus == IfOperStatusUp {
                let mac = if a.PhysicalAddressLength >= 6 {
                    format!(
                        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                        a.PhysicalAddress[0],
                        a.PhysicalAddress[1],
                        a.PhysicalAddress[2],
                        a.PhysicalAddress[3],
                        a.PhysicalAddress[4],
                        a.PhysicalAddress[5]
                    )
                } else {
                    String::new()
                };
                let mbps = (a.TransmitLinkSpeed / 1_000_000) as u32;
                let mut ip = String::new();
                let mut ua = a.FirstUnicastAddress;
                while !ua.is_null() {
                    let u = &*ua;
                    let sa_ptr = u.Address.lpSockaddr;
                    if !sa_ptr.is_null() {
                        let sa = &*(sa_ptr as *const SOCKADDR);
                        if sa.sa_family == AF_INET {
                            let sin = &*(sa_ptr as *const SOCKADDR_IN);
                            let b = sin.sin_addr.S_un.S_un_b;
                            ip = format!("{}.{}.{}.{}", b.s_b1, b.s_b2, b.s_b3, b.s_b4);
                            break;
                        }
                    }
                    ua = u.Next;
                }
                return (ip, mac, mbps);
            }
            cur = a.Next;
        }
        (String::new(), String::new(), 0)
    }

    fn poll_state() -> WifiState {
        let Ok(client) = WlanClient::open() else {
            return WifiState::default();
        };
        unsafe {
            let Ok((guid, info)) = first_interface(&client) else {
                return WifiState::default();
            };
            let enabled = query_radio_enabled(&client, &guid);
            let mut state = WifiState {
                enabled,
                ..Default::default()
            };
            if !enabled {
                return state;
            }
            if let Some(conn) = query_connection(&client, &guid) {
                if conn.isState == wlan_interface_state_connected
                    || info.isState == wlan_interface_state_connected
                {
                    state.connected = true;
                    state.ssid = ssid_to_string(&conn.wlanAssociationAttributes.dot11Ssid);
                    if state.ssid.is_empty() {
                        state.ssid = wchar_to_string(&conn.strProfileName);
                    }
                    state.signal = conn.wlanAssociationAttributes.wlanSignalQuality.min(100);
                    state.secured = conn.wlanSecurityAttributes.bSecurityEnabled.as_bool();
                    let rx = conn.wlanAssociationAttributes.ulRxRate;
                    let tx = conn.wlanAssociationAttributes.ulTxRate;
                    // Rates are in Kbps — prefer RX (matches netsh “Receive rate”).
                    let kbps = if rx > 0 { rx } else { tx };
                    state.link_mbps = (kbps / 1000).max(1);
                    let bssid = conn.wlanAssociationAttributes.dot11Bssid;
                    state.mac = format!(
                        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                        bssid[0], bssid[1], bssid[2], bssid[3], bssid[4], bssid[5]
                    );
                }
            } else if info.isState == wlan_interface_state_connected {
                state.connected = true;
            }
            if state.connected {
                let (ip, _adapter_mac, mbps) = adapter_ip_mac_speed();
                if !ip.is_empty() {
                    state.ip = ip;
                }
                if mbps > 0 && state.link_mbps == 0 {
                    state.link_mbps = mbps;
                }
            }
            state
        }
    }

    fn store_and_emit(state: WifiState) -> WifiState {
        *STATE.lock() = state.clone();
        if let Some(emit) = EMIT.get() {
            emit(state.clone());
        }
        state
    }

    pub fn get() -> WifiState {
        STATE.lock().clone()
    }

    pub fn start(emit: impl Fn(WifiState) + Send + Sync + 'static) {
        let _ = EMIT.set(Box::new(emit));
        let initial = poll_state();
        store_and_emit(initial);
        std::thread::Builder::new()
            .name("wifi-watcher".into())
            .spawn(|| loop {
                std::thread::sleep(Duration::from_millis(1500));
                let next = poll_state();
                let prev = STATE.lock().clone();
                if next != prev {
                    store_and_emit(next);
                }
            })
            .ok();
    }

    pub fn refresh() -> WifiState {
        store_and_emit(poll_state())
    }

    pub fn list_networks() -> Result<Vec<WifiNetwork>, String> {
        let client = WlanClient::open()?;
        unsafe {
            let (guid, _) = first_interface(&client)?;
            let _ = WlanScan(client.handle, &guid, None, None, None);
            // Scan is async; brief wait so the list isn't stale/empty.
            std::thread::sleep(Duration::from_millis(600));

            let mut list_ptr: *mut WLAN_AVAILABLE_NETWORK_LIST = std::ptr::null_mut();
            let rc = WlanGetAvailableNetworkList(client.handle, &guid, 0, None, &mut list_ptr);
            if rc != ERROR_SUCCESS.0 || list_ptr.is_null() {
                return Err(format!("WlanGetAvailableNetworkList failed: {rc}"));
            }

            let list = &*list_ptr;
            let count = list.dwNumberOfItems as usize;
            let base = list.Network.as_ptr();
            let mut out: Vec<WifiNetwork> = Vec::with_capacity(count);
            let mut seen = std::collections::HashSet::<String>::new();

            for i in 0..count {
                let net: WLAN_AVAILABLE_NETWORK = *base.add(i);
                let ssid = ssid_to_string(&net.dot11Ssid);
                if ssid.is_empty() {
                    continue;
                }
                let connected = (net.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED) != 0;
                let profile_name = wchar_to_string(&net.strProfileName);
                let has_profile =
                    (net.dwFlags & WLAN_AVAILABLE_NETWORK_HAS_PROFILE) != 0 || !profile_name.is_empty();
                let signal = net.wlanSignalQuality.min(100);
                let secured = net.bSecurityEnabled.as_bool();
                let auth = auth_from_algo(net.dot11DefaultAuthAlgorithm);

                if let Some(existing) = out.iter_mut().find(|n| n.ssid == ssid) {
                    if signal > existing.signal {
                        existing.signal = signal;
                    }
                    existing.connected |= connected;
                    existing.has_profile |= has_profile;
                    existing.secured |= secured;
                    if existing.profile_name.is_empty() && !profile_name.is_empty() {
                        existing.profile_name = profile_name;
                    }
                    if existing.auth.is_empty() && !auth.is_empty() {
                        existing.auth = auth;
                    }
                    continue;
                }
                if !seen.insert(ssid.clone()) {
                    continue;
                }
                out.push(WifiNetwork {
                    ssid,
                    signal,
                    secured,
                    connected,
                    has_profile,
                    profile_name,
                    auth,
                });
            }
            WlanFreeMemory(list_ptr as *const _);

            out.sort_by(|a, b| {
                b.connected
                    .cmp(&a.connected)
                    .then(b.signal.cmp(&a.signal))
                    .then(a.ssid.cmp(&b.ssid))
            });
            Ok(out)
        }
    }

    pub fn set_enabled(enabled: bool) -> Result<WifiState, String> {
        let client = WlanClient::open()?;
        unsafe {
            let (guid, _) = first_interface(&client)?;
            // Query current radio to pick a phy index.
            let mut data_size = 0u32;
            let mut data: *mut core::ffi::c_void = std::ptr::null_mut();
            let rc = WlanQueryInterface(
                client.handle,
                &guid,
                wlan_intf_opcode_radio_state,
                None,
                &mut data_size,
                &mut data,
                None,
            );
            let phy_index = if rc == ERROR_SUCCESS.0 && !data.is_null() {
                let radio = &*(data as *const WLAN_RADIO_STATE);
                let idx = if radio.dwNumberOfPhys > 0 {
                    radio.PhyRadioState[0].dwPhyIndex
                } else {
                    0
                };
                WlanFreeMemory(data);
                idx
            } else {
                0
            };

            let phy = WLAN_PHY_RADIO_STATE {
                dwPhyIndex: phy_index,
                dot11SoftwareRadioState: if enabled {
                    dot11_radio_state_on
                } else {
                    dot11_radio_state_off
                },
                dot11HardwareRadioState: if enabled {
                    dot11_radio_state_on
                } else {
                    dot11_radio_state_off
                },
            };
            let set_rc = WlanSetInterface(
                client.handle,
                &guid,
                wlan_intf_opcode_radio_state,
                std::mem::size_of::<WLAN_PHY_RADIO_STATE>() as u32,
                &phy as *const _ as *const core::ffi::c_void,
                None,
            );
            if set_rc != ERROR_SUCCESS.0 {
                return Err(format!("无法切换 Wi‑Fi：{set_rc}"));
            }
        }
        // Radio state can lag briefly.
        std::thread::sleep(Duration::from_millis(200));
        Ok(refresh())
    }

    fn auth_from_algo(algo: windows::Win32::NetworkManagement::WiFi::DOT11_AUTH_ALGORITHM) -> String {
        if algo == DOT11_AUTH_ALGO_WPA3_SAE {
            "wpa3sae".into()
        } else if algo == DOT11_AUTH_ALGO_RSNA_PSK {
            "wpa2psk".into()
        } else if algo == DOT11_AUTH_ALGO_WPA_PSK {
            "wpapsk".into()
        } else if algo == DOT11_AUTH_ALGO_80211_OPEN {
            "open".into()
        } else {
            // Default to WPA2-PSK for unknown secured networks.
            "wpa2psk".into()
        }
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    fn build_profile_xml(ssid: &str, password: &str, auth: &str) -> String {
        let name = xml_escape(ssid);
        let key = xml_escape(password);
        let (authentication, encryption) = match auth {
            "open" => ("open", "none"),
            "wpapsk" => ("WPAPSK", "TKIP"),
            "wpa3sae" => ("WPA3SAE", "AES"),
            _ => ("WPA2PSK", "AES"),
        };
        if authentication == "open" {
            format!(
                r#"<?xml version="1.0"?>
<WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1">
	<name>{name}</name>
	<SSIDConfig>
		<SSID>
			<name>{name}</name>
		</SSID>
	</SSIDConfig>
	<connectionType>ESS</connectionType>
	<connectionMode>auto</connectionMode>
	<MSM>
		<security>
			<authEncryption>
				<authentication>open</authentication>
				<encryption>none</encryption>
				<useOneX>false</useOneX>
			</authEncryption>
		</security>
	</MSM>
</WLANProfile>"#
            )
        } else {
            format!(
                r#"<?xml version="1.0"?>
<WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1">
	<name>{name}</name>
	<SSIDConfig>
		<SSID>
			<name>{name}</name>
		</SSID>
	</SSIDConfig>
	<connectionType>ESS</connectionType>
	<connectionMode>auto</connectionMode>
	<MSM>
		<security>
			<authEncryption>
				<authentication>{authentication}</authentication>
				<encryption>{encryption}</encryption>
				<useOneX>false</useOneX>
			</authEncryption>
			<sharedKey>
				<keyType>passPhrase</keyType>
				<protected>false</protected>
				<keyMaterial>{key}</keyMaterial>
			</sharedKey>
		</security>
	</MSM>
</WLANProfile>"#
            )
        }
    }

    pub fn connect(ssid: &str, password: Option<&str>) -> Result<WifiState, String> {
        let ssid = ssid.trim();
        if ssid.is_empty() {
            return Err("SSID 为空".into());
        }
        let networks = list_networks().unwrap_or_default();
        let target = networks.iter().find(|n| n.ssid == ssid);
        let has_profile = target.map(|n| n.has_profile).unwrap_or(false);
        let secured = target.map(|n| n.secured).unwrap_or(true);
        let auth = target
            .map(|n| n.auth.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| if secured { "wpa2psk".into() } else { "open".into() });
        let profile_name = target
            .map(|n| {
                if !n.profile_name.is_empty() {
                    n.profile_name.clone()
                } else {
                    n.ssid.clone()
                }
            })
            .unwrap_or_else(|| ssid.to_string());

        let password = password.map(str::trim).filter(|s| !s.is_empty());

        // Secured network without profile needs a password from the auth dialog.
        if !has_profile && secured && password.is_none() {
            return Err("NEED_PASSWORD".into());
        }

        let client = WlanClient::open()?;
        unsafe {
            let (guid, _) = first_interface(&client)?;

            if let Some(pwd) = password {
                let xml = build_profile_xml(ssid, pwd, &auth);
                let xml_hs = HSTRING::from(xml.as_str());
                let mut reason = 0u32;
                let set_rc = WlanSetProfile(
                    client.handle,
                    &guid,
                    0,
                    PCWSTR(xml_hs.as_ptr()),
                    PCWSTR::null(),
                    BOOL(1),
                    None,
                    &mut reason,
                );
                if set_rc != ERROR_SUCCESS.0 {
                    return Err(format!("保存 Wi‑Fi 配置失败：{set_rc} (reason {reason})"));
                }
            }

            let connect_name = if password.is_some() {
                ssid.to_string()
            } else {
                profile_name
            };
            let profile = HSTRING::from(connect_name.as_str());
            let mut dot11 = DOT11_SSID {
                uSSIDLength: ssid.len().min(32) as u32,
                ucSSID: [0; 32],
            };
            let bytes = ssid.as_bytes();
            let n = bytes.len().min(32);
            dot11.ucSSID[..n].copy_from_slice(&bytes[..n]);

            let params = if has_profile || password.is_some() {
                WLAN_CONNECTION_PARAMETERS {
                    wlanConnectionMode: wlan_connection_mode_profile,
                    strProfile: PCWSTR(profile.as_ptr()),
                    pDot11Ssid: std::ptr::null_mut(),
                    pDesiredBssidList: std::ptr::null_mut(),
                    dot11BssType: dot11_BSS_type_infrastructure,
                    dwFlags: 0,
                }
            } else {
                WLAN_CONNECTION_PARAMETERS {
                    wlanConnectionMode: wlan_connection_mode_discovery_unsecure,
                    strProfile: PCWSTR::null(),
                    pDot11Ssid: &mut dot11,
                    pDesiredBssidList: std::ptr::null_mut(),
                    dot11BssType: dot11_BSS_type_infrastructure,
                    dwFlags: 0,
                }
            };

            let rc = WlanConnect(client.handle, &guid, &params, None);
            if rc != ERROR_SUCCESS.0 {
                return Err(format!("连接失败：{rc}"));
            }
        }
        std::thread::sleep(Duration::from_millis(500));
        Ok(refresh())
    }

    pub fn disconnect() -> Result<WifiState, String> {
        let client = WlanClient::open()?;
        unsafe {
            let (guid, _) = first_interface(&client)?;
            let rc = WlanDisconnect(client.handle, &guid, None);
            if rc != ERROR_SUCCESS.0 {
                return Err(format!("断开失败：{rc}"));
            }
        }
        std::thread::sleep(Duration::from_millis(200));
        Ok(refresh())
    }

    pub fn open_network_settings() -> Result<(), String> {
        unsafe {
            let op = HSTRING::from("open");
            let file = HSTRING::from("ms-settings:network-wifi");
            let ret = ShellExecuteW(
                HWND::default(),
                PCWSTR(op.as_ptr()),
                PCWSTR(file.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
            if (ret.0 as isize) <= 32 {
                return Err("无法打开网络设置".into());
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
pub use win::{
    connect, disconnect, get, list_networks, open_network_settings, refresh, set_enabled, start,
};

#[cfg(not(windows))]
pub fn get() -> WifiState {
    WifiState::default()
}

#[cfg(not(windows))]
pub fn start(_emit: impl Fn(WifiState) + Send + Sync + 'static) {}

#[cfg(not(windows))]
pub fn refresh() -> WifiState {
    WifiState::default()
}

#[cfg(not(windows))]
pub fn list_networks() -> Result<Vec<WifiNetwork>, String> {
    Ok(Vec::new())
}

#[cfg(not(windows))]
pub fn set_enabled(_enabled: bool) -> Result<WifiState, String> {
    Ok(WifiState::default())
}

#[cfg(not(windows))]
pub fn connect(_ssid: &str, _password: Option<&str>) -> Result<WifiState, String> {
    Ok(WifiState::default())
}

#[cfg(not(windows))]
pub fn disconnect() -> Result<WifiState, String> {
    Ok(WifiState::default())
}

#[cfg(not(windows))]
pub fn open_network_settings() -> Result<(), String> {
    Ok(())
}
