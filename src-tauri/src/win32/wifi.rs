//! Host-owned Wi‑Fi / Ethernet indicator (Shell network chrome vanishes with the taskbar).

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
    /// Wired (Ethernet) link is up — tray prefers this over Wi‑Fi glyph.
    #[serde(default)]
    pub ethernet_connected: bool,
    #[serde(default)]
    pub ethernet_name: String,
    #[serde(default)]
    pub ethernet_ip: String,
    #[serde(default)]
    pub ethernet_link_mbps: u32,
    #[serde(default)]
    pub ethernet_mac: String,
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
        GetAdaptersAddresses, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST,
        GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IF_TYPE_ETHERNET_CSMACD,
        IF_TYPE_GIGABITETHERNET, IF_TYPE_IEEE80211, IP_ADAPTER_ADDRESSES_LH,
        IP_ADAPTER_GATEWAY_ADDRESS_LH,
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

    unsafe fn pwstr_to_string(p: windows::core::PWSTR) -> String {
        if p.is_null() {
            return String::new();
        }
        p.to_string().unwrap_or_default().trim().to_string()
    }

    fn is_virtual_adapter(name: &str, desc: &str) -> bool {
        let s = format!("{name} {desc}").to_ascii_lowercase();
        [
            "virtual",
            "hyper-v",
            "vethernet",
            "vmware",
            "virtualbox",
            "vbox",
            "vpn",
            "tap-windows",
            "wintun",
            "wireguard",
            "bluetooth",
            "loopback",
            "pseudo",
            "teredo",
            "isatap",
            "microsoft wi-fi direct",
            "sangfor",
            "atrust",
            "vnic",
            "docker",
            "wsl",
            "npcap",
        ]
        .iter()
        .any(|k| s.contains(k))
    }

    fn is_ethernet_if_type(if_type: u32) -> bool {
        if_type == IF_TYPE_ETHERNET_CSMACD || if_type == IF_TYPE_GIGABITETHERNET
    }

    fn is_apipa(ip: &str) -> bool {
        ip.starts_with("169.254.")
    }

    unsafe fn adapter_ipv4(a: &IP_ADAPTER_ADDRESSES_LH) -> String {
        let mut ua = a.FirstUnicastAddress;
        while !ua.is_null() {
            let u = &*ua;
            let sa_ptr = u.Address.lpSockaddr;
            if !sa_ptr.is_null() {
                let sa = &*(sa_ptr as *const SOCKADDR);
                if sa.sa_family == AF_INET {
                    let sin = &*(sa_ptr as *const SOCKADDR_IN);
                    let b = sin.sin_addr.S_un.S_un_b;
                    return format!("{}.{}.{}.{}", b.s_b1, b.s_b2, b.s_b3, b.s_b4);
                }
            }
            ua = u.Next;
        }
        String::new()
    }

    unsafe fn adapter_has_gateway(a: &IP_ADAPTER_ADDRESSES_LH) -> bool {
        let mut g = a.FirstGatewayAddress;
        while !g.is_null() {
            let gw = &*(g as *const IP_ADAPTER_GATEWAY_ADDRESS_LH);
            let sa_ptr = gw.Address.lpSockaddr;
            if !sa_ptr.is_null() {
                let sa = &*(sa_ptr as *const SOCKADDR);
                if sa.sa_family == AF_INET {
                    return true;
                }
            }
            g = gw.Next;
        }
        false
    }

    unsafe fn adapter_mac(a: &IP_ADAPTER_ADDRESSES_LH) -> String {
        if a.PhysicalAddressLength >= 6 {
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
        }
    }

    /// Walk adapters once; fill Wi‑Fi IP/MAC/speed and Ethernet link details.
    /// Prefers the in-use wired NIC (has gateway / non-APIPA) over VMware/Hyper-V.
    unsafe fn adapter_wifi_and_ethernet() -> ((String, String, u32), EthernetSnapshot) {
        let empty_wifi = (String::new(), String::new(), 0u32);
        let empty_eth = EthernetSnapshot::default();
        let flags = GAA_FLAG_SKIP_ANYCAST
            | GAA_FLAG_SKIP_MULTICAST
            | GAA_FLAG_SKIP_DNS_SERVER
            | GAA_FLAG_INCLUDE_GATEWAYS;
        let mut size = 0u32;
        let probe = GetAdaptersAddresses(AF_INET.0 as u32, flags, None, None, &mut size);
        if size == 0
            && probe != ERROR_BUFFER_OVERFLOW.0
            && probe != ERROR_SUCCESS.0
        {
            return (empty_wifi, empty_eth);
        }
        if size == 0 {
            return (empty_wifi, empty_eth);
        }
        let mut buf = vec![0u8; size as usize];
        let head = buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
        let rc = GetAdaptersAddresses(AF_INET.0 as u32, flags, None, Some(head), &mut size);
        if rc != ERROR_SUCCESS.0 {
            return (empty_wifi, empty_eth);
        }

        let mut wifi = empty_wifi;
        // score: higher wins — gateway(+1000), real IP(+100), lower metric bonus
        let mut best: Option<(i32, EthernetSnapshot)> = None;

        let mut cur = head;
        while !cur.is_null() {
            let a = &*cur;
            if a.OperStatus == IfOperStatusUp {
                if a.IfType == IF_TYPE_IEEE80211 && wifi.0.is_empty() {
                    let mac = adapter_mac(a);
                    let mbps = (a.TransmitLinkSpeed / 1_000_000) as u32;
                    let ip = adapter_ipv4(a);
                    wifi = (ip, mac, mbps);
                } else if is_ethernet_if_type(a.IfType) {
                    let name = pwstr_to_string(a.FriendlyName);
                    let desc = pwstr_to_string(a.Description);
                    if !is_virtual_adapter(&name, &desc) {
                        let ip = adapter_ipv4(a);
                        let has_gw = adapter_has_gateway(a);
                        let snap = EthernetSnapshot {
                            connected: true,
                            name: if name.is_empty() {
                                "以太网".into()
                            } else {
                                name
                            },
                            ip: ip.clone(),
                            link_mbps: (a.TransmitLinkSpeed / 1_000_000) as u32,
                            mac: adapter_mac(a),
                        };
                        let mut score = 0i32;
                        if has_gw {
                            score += 1000;
                        }
                        if !ip.is_empty() && !is_apipa(&ip) {
                            score += 100;
                        } else if !ip.is_empty() {
                            score += 10;
                        }
                        // Prefer lower IPv4 metric (Windows routing preference).
                        score += 1000i32 - (a.Ipv4Metric as i32).min(999);
                        let replace = match &best {
                            None => true,
                            Some((best_score, _)) => score > *best_score,
                        };
                        if replace {
                            best = Some((score, snap));
                        }
                    }
                }
            }
            cur = a.Next;
        }
        (
            wifi,
            best.map(|(_, s)| s).unwrap_or(empty_eth),
        )
    }

    #[derive(Default, Clone)]
    struct EthernetSnapshot {
        connected: bool,
        name: String,
        ip: String,
        link_mbps: u32,
        mac: String,
    }

    fn apply_ethernet(state: &mut WifiState, eth: EthernetSnapshot) {
        state.ethernet_connected = eth.connected;
        state.ethernet_name = eth.name;
        state.ethernet_ip = eth.ip;
        state.ethernet_link_mbps = eth.link_mbps;
        state.ethernet_mac = eth.mac;
    }

    fn poll_state() -> WifiState {
        let (wifi_adapt, eth) = unsafe { adapter_wifi_and_ethernet() };
        let mut state = WifiState::default();
        apply_ethernet(&mut state, eth);

        let Ok(client) = WlanClient::open() else {
            return state;
        };
        unsafe {
            let Ok((guid, info)) = first_interface(&client) else {
                return state;
            };
            let enabled = query_radio_enabled(&client, &guid);
            state.enabled = enabled;
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
                let (ip, _adapter_mac, mbps) = wifi_adapt;
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
        // Always re-poll on IPC read so tray/popup aren't stuck on Default
        // before the boot-pipeline watcher finishes starting.
        let next = poll_state();
        *STATE.lock() = next.clone();
        next
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
            // Status page covers both Wi‑Fi and Ethernet.
            let file = HSTRING::from("ms-settings:network-status");
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
