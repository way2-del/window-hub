//! Per-process network rate + Windows Firewall block for network flyout.
//! Prefers TCP ESTATS when collection is enabled; otherwise shares live NIC
//! rates by ESTAB TCP / UDP socket weight (no elevation required).

#![cfg(windows)]

use parking_lot::Mutex;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::LazyLock;
use std::time::Instant;

const RULE_GROUP: &str = "Window Hub";
const META_KEY: &str = "net_blocked_paths";
const MAX_ESTATS: usize = 480;
const MIB_TCP_STATE_ESTAB: u32 = 5;
/// Hard cap for displayed process rates — above this is almost certainly bad ESTATS data.
const MAX_PROCESS_BPS: u64 = 2_500_000_000; // 2.5 GB/s

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetProcessRow {
    pub pid: u32,
    pub name: String,
    pub path: String,
    pub down_bps: u64,
    pub up_bps: u64,
    pub connections: u32,
    pub blocked: bool,
    pub icon_png: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetProcSnapshot {
    pub processes: Vec<NetProcessRow>,
    /// True after at least one prior sample — rates need a second tick.
    pub warmed: bool,
}

/// Stable key for one TCP 4-tuple (v4 / v6).
#[derive(Clone, PartialEq, Eq, Hash)]
enum ConnKey {
    V4 {
        local_addr: u32,
        local_port: u32,
        remote_addr: u32,
        remote_port: u32,
    },
    V6 {
        local: [u8; 16],
        local_port: u32,
        local_scope: u32,
        remote: [u8; 16],
        remote_port: u32,
        remote_scope: u32,
    },
}

struct PrevAgg {
    at: Instant,
    /// Per-connection octet counters from last sample (only when ESTATS was valid).
    by_conn: HashMap<ConnKey, (u64, u64)>,
}

static PREV: Mutex<Option<PrevAgg>> = Mutex::new(None);
static ICON_CACHE: LazyLock<Mutex<HashMap<String, Option<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn path_key(path: &str) -> String {
    path.trim().replace('/', "\\").to_ascii_lowercase()
}

fn path_hash(path: &str) -> u32 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path_key(path).hash(&mut h);
    h.finish() as u32
}

fn rule_names(path: &str) -> (String, String) {
    let file = Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "app.exe".into());
    let hash = path_hash(path);
    (
        format!("Window Hub · Block Out · {file} · {hash:08x}"),
        format!("Window Hub · Block In · {file} · {hash:08x}"),
    )
}

fn process_image(pid: u32) -> Option<(String, String)> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let Ok(proc) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return None;
        };
        let mut buf = [0u16; 512];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            proc,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut size,
        )
        .is_ok();
        let _ = CloseHandle(proc);
        if !ok || size == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..size as usize]);
        let name = Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        Some((name, path))
    }
}

fn icon_for_exe(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    {
        let cache = ICON_CACHE.lock();
        if let Some(hit) = cache.get(path) {
            return hit.clone();
        }
    }
    let png = crate::dock::resolve_small_icon_png(path);
    ICON_CACHE.lock().insert(path.to_string(), png.clone());
    png
}

fn self_exe_key() -> Option<String> {
    std::env::current_exe()
        .ok()
        .map(|p| path_key(&p.to_string_lossy()))
}

fn load_blocked_paths() -> HashSet<String> {
    let Ok(Some(v)) = crate::db::with_conn(|c| crate::db::meta_get(c, META_KEY)) else {
        return HashSet::new();
    };
    let Some(arr) = v.as_array() else {
        return HashSet::new();
    };
    arr.iter()
        .filter_map(|x| x.as_str().map(path_key))
        .filter(|s| !s.is_empty())
        .collect()
}

fn save_blocked_paths(set: &HashSet<String>) -> Result<(), String> {
    let mut list: Vec<String> = set.iter().cloned().collect();
    list.sort();
    let value = serde_json::Value::Array(
        list.into_iter()
            .map(serde_json::Value::String)
            .collect(),
    );
    crate::db::with_conn(|c| crate::db::meta_set(c, META_KEY, &value))
}

fn read_tcp_v4() -> Vec<windows::Win32::NetworkManagement::IpHelper::MIB_TCPROW_OWNER_PID> {
    use windows::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_CONNECTIONS,
    };
    use windows::Win32::Networking::WinSock::AF_INET;

    unsafe {
        let mut size = 0u32;
        let _ = GetExtendedTcpTable(
            None,
            &mut size,
            true,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_CONNECTIONS,
            0,
        );
        if size == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u8; size as usize];
        let err = GetExtendedTcpTable(
            Some(buf.as_mut_ptr().cast()),
            &mut size,
            true,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_CONNECTIONS,
            0,
        );
        if err != 0 || size < std::mem::size_of::<MIB_TCPTABLE_OWNER_PID>() as u32 {
            return Vec::new();
        }
        let table = &*(buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID);
        let n = table.dwNumEntries as usize;
        if n == 0 {
            return Vec::new();
        }
        let rows = std::slice::from_raw_parts(table.table.as_ptr(), n);
        rows.to_vec()
    }
}

fn read_tcp_v6() -> Vec<windows::Win32::NetworkManagement::IpHelper::MIB_TCP6ROW_OWNER_PID> {
    use windows::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP6TABLE_OWNER_PID, TCP_TABLE_OWNER_PID_CONNECTIONS,
    };
    use windows::Win32::Networking::WinSock::AF_INET6;

    unsafe {
        let mut size = 0u32;
        let _ = GetExtendedTcpTable(
            None,
            &mut size,
            true,
            AF_INET6.0 as u32,
            TCP_TABLE_OWNER_PID_CONNECTIONS,
            0,
        );
        if size == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u8; size as usize];
        let err = GetExtendedTcpTable(
            Some(buf.as_mut_ptr().cast()),
            &mut size,
            true,
            AF_INET6.0 as u32,
            TCP_TABLE_OWNER_PID_CONNECTIONS,
            0,
        );
        if err != 0 || size < std::mem::size_of::<MIB_TCP6TABLE_OWNER_PID>() as u32 {
            return Vec::new();
        }
        let table = &*(buf.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID);
        let n = table.dwNumEntries as usize;
        if n == 0 {
            return Vec::new();
        }
        let rows = std::slice::from_raw_parts(table.table.as_ptr(), n);
        rows.to_vec()
    }
}

fn estats_v4(
    row: &windows::Win32::NetworkManagement::IpHelper::MIB_TCPROW_OWNER_PID,
) -> Option<(u64, u64)> {
    use windows::Win32::Foundation::BOOLEAN;
    use windows::Win32::NetworkManagement::IpHelper::{
        GetPerTcpConnectionEStats, SetPerTcpConnectionEStats, MIB_TCPROW_LH, MIB_TCPROW_LH_0,
        TCP_ESTATS_DATA_ROD_v0, TCP_ESTATS_DATA_RW_v0, TcpConnectionEstatsData,
    };

    if row.dwState != MIB_TCP_STATE_ESTAB {
        return None;
    }
    unsafe {
        let lh = MIB_TCPROW_LH {
            Anonymous: MIB_TCPROW_LH_0 {
                dwState: row.dwState,
            },
            dwLocalAddr: row.dwLocalAddr,
            dwLocalPort: row.dwLocalPort,
            dwRemoteAddr: row.dwRemoteAddr,
            dwRemotePort: row.dwRemotePort,
        };
        let rw = TCP_ESTATS_DATA_RW_v0 {
            EnableCollection: BOOLEAN(1),
        };
        let rw_bytes = std::slice::from_raw_parts(
            (&rw as *const TCP_ESTATS_DATA_RW_v0).cast::<u8>(),
            std::mem::size_of::<TCP_ESTATS_DATA_RW_v0>(),
        );
        // Best-effort enable; may fail without elevation. Still read — collection may
        // already be on. Never trust Rod unless EnableCollection comes back true.
        let _ = SetPerTcpConnectionEStats(&lh, TcpConnectionEstatsData, rw_bytes, 0, 0);

        let mut rw_out = TCP_ESTATS_DATA_RW_v0::default();
        let rw_out_bytes = std::slice::from_raw_parts_mut(
            (&mut rw_out as *mut TCP_ESTATS_DATA_RW_v0).cast::<u8>(),
            std::mem::size_of::<TCP_ESTATS_DATA_RW_v0>(),
        );
        let mut rod = TCP_ESTATS_DATA_ROD_v0::default();
        let rod_bytes = std::slice::from_raw_parts_mut(
            (&mut rod as *mut TCP_ESTATS_DATA_ROD_v0).cast::<u8>(),
            std::mem::size_of::<TCP_ESTATS_DATA_ROD_v0>(),
        );
        let err = GetPerTcpConnectionEStats(
            &lh,
            TcpConnectionEstatsData,
            Some(rw_out_bytes),
            0,
            None,
            0,
            Some(rod_bytes),
            0,
        );
        // MSDN: if EnableCollection is FALSE, Ros/Rod are undefined (random garbage).
        if err != 0 || rw_out.EnableCollection.0 == 0 {
            return None;
        }
        Some((rod.DataBytesIn, rod.DataBytesOut))
    }
}

fn estats_v6(
    row: &windows::Win32::NetworkManagement::IpHelper::MIB_TCP6ROW_OWNER_PID,
) -> Option<(u64, u64)> {
    use windows::Win32::Foundation::BOOLEAN;
    use windows::Win32::NetworkManagement::IpHelper::{
        GetPerTcp6ConnectionEStats, SetPerTcp6ConnectionEStats, MIB_TCP6ROW, MIB_TCP_STATE,
        TCP_ESTATS_DATA_ROD_v0, TCP_ESTATS_DATA_RW_v0, TcpConnectionEstatsData,
    };
    use windows::Win32::Networking::WinSock::{IN6_ADDR, IN6_ADDR_0};

    if row.dwState != MIB_TCP_STATE_ESTAB {
        return None;
    }
    unsafe {
        let tcp6 = MIB_TCP6ROW {
            State: MIB_TCP_STATE(row.dwState as i32),
            LocalAddr: IN6_ADDR {
                u: IN6_ADDR_0 {
                    Byte: row.ucLocalAddr,
                },
            },
            dwLocalScopeId: row.dwLocalScopeId,
            dwLocalPort: row.dwLocalPort,
            RemoteAddr: IN6_ADDR {
                u: IN6_ADDR_0 {
                    Byte: row.ucRemoteAddr,
                },
            },
            dwRemoteScopeId: row.dwRemoteScopeId,
            dwRemotePort: row.dwRemotePort,
        };
        let rw = TCP_ESTATS_DATA_RW_v0 {
            EnableCollection: BOOLEAN(1),
        };
        let rw_bytes = std::slice::from_raw_parts(
            (&rw as *const TCP_ESTATS_DATA_RW_v0).cast::<u8>(),
            std::mem::size_of::<TCP_ESTATS_DATA_RW_v0>(),
        );
        let _ = SetPerTcp6ConnectionEStats(&tcp6, TcpConnectionEstatsData, rw_bytes, 0, 0);

        let mut rw_out = TCP_ESTATS_DATA_RW_v0::default();
        let rw_out_bytes = std::slice::from_raw_parts_mut(
            (&mut rw_out as *mut TCP_ESTATS_DATA_RW_v0).cast::<u8>(),
            std::mem::size_of::<TCP_ESTATS_DATA_RW_v0>(),
        );
        let mut rod = TCP_ESTATS_DATA_ROD_v0::default();
        let rod_bytes = std::slice::from_raw_parts_mut(
            (&mut rod as *mut TCP_ESTATS_DATA_ROD_v0).cast::<u8>(),
            std::mem::size_of::<TCP_ESTATS_DATA_ROD_v0>(),
        );
        let err = GetPerTcp6ConnectionEStats(
            &tcp6,
            TcpConnectionEstatsData,
            Some(rw_out_bytes),
            0,
            None,
            0,
            Some(rod_bytes),
            0,
        );
        if err != 0 || rw_out.EnableCollection.0 == 0 {
            return None;
        }
        Some((rod.DataBytesIn, rod.DataBytesOut))
    }
}

struct PathMeta {
    pid: u32,
    name: String,
    path: String,
    conns: u32,
    /// Weight for NIC-rate sharing (ESTAB TCP + UDP sockets).
    weight: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EstatsMode {
    /// Not probed yet (or no ESTAB sockets on last probe).
    Unknown,
    /// At least one connection returned valid EnableCollection.
    Available,
    /// Probed ESTAB sockets; none had collection enabled (typical without elevation).
    Unavailable,
}

static ESTATS_MODE: Mutex<EstatsMode> = Mutex::new(EstatsMode::Unknown);

fn read_udp_v4() -> Vec<windows::Win32::NetworkManagement::IpHelper::MIB_UDPROW_OWNER_PID> {
    use windows::Win32::NetworkManagement::IpHelper::{
        GetExtendedUdpTable, MIB_UDPTABLE_OWNER_PID, UDP_TABLE_OWNER_PID,
    };
    use windows::Win32::Networking::WinSock::AF_INET;

    unsafe {
        let mut size = 0u32;
        let _ = GetExtendedUdpTable(
            None,
            &mut size,
            true,
            AF_INET.0 as u32,
            UDP_TABLE_OWNER_PID,
            0,
        );
        if size == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u8; size as usize];
        let err = GetExtendedUdpTable(
            Some(buf.as_mut_ptr().cast()),
            &mut size,
            true,
            AF_INET.0 as u32,
            UDP_TABLE_OWNER_PID,
            0,
        );
        if err != 0 || size < std::mem::size_of::<MIB_UDPTABLE_OWNER_PID>() as u32 {
            return Vec::new();
        }
        let table = &*(buf.as_ptr() as *const MIB_UDPTABLE_OWNER_PID);
        let n = table.dwNumEntries as usize;
        if n == 0 {
            return Vec::new();
        }
        let rows = std::slice::from_raw_parts(table.table.as_ptr(), n);
        rows.to_vec()
    }
}

fn read_udp_v6() -> Vec<windows::Win32::NetworkManagement::IpHelper::MIB_UDP6ROW_OWNER_PID> {
    use windows::Win32::NetworkManagement::IpHelper::{
        GetExtendedUdpTable, MIB_UDP6TABLE_OWNER_PID, UDP_TABLE_OWNER_PID,
    };
    use windows::Win32::Networking::WinSock::AF_INET6;

    unsafe {
        let mut size = 0u32;
        let _ = GetExtendedUdpTable(
            None,
            &mut size,
            true,
            AF_INET6.0 as u32,
            UDP_TABLE_OWNER_PID,
            0,
        );
        if size == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u8; size as usize];
        let err = GetExtendedUdpTable(
            Some(buf.as_mut_ptr().cast()),
            &mut size,
            true,
            AF_INET6.0 as u32,
            UDP_TABLE_OWNER_PID,
            0,
        );
        if err != 0 || size < std::mem::size_of::<MIB_UDP6TABLE_OWNER_PID>() as u32 {
            return Vec::new();
        }
        let table = &*(buf.as_ptr() as *const MIB_UDP6TABLE_OWNER_PID);
        let n = table.dwNumEntries as usize;
        if n == 0 {
            return Vec::new();
        }
        let rows = std::slice::from_raw_parts(table.table.as_ptr(), n);
        rows.to_vec()
    }
}

/// Sample path metadata + optional per-connection ESTATS counters.
fn sample_totals() -> (
    HashMap<String, PathMeta>,
    HashMap<ConnKey, (String, u64, u64)>,
) {
    let mut by_path: HashMap<String, PathMeta> = HashMap::new();
    let mut by_conn: HashMap<ConnKey, (String, u64, u64)> = HashMap::new();
    let mut pid_cache: HashMap<u32, Option<(String, String)>> = HashMap::new();
    let mut estats_budget = MAX_ESTATS;
    let mut estats_mode = *ESTATS_MODE.lock();
    let try_estats = !matches!(estats_mode, EstatsMode::Unavailable);
    let mut estats_tried = 0u32;
    let mut estats_ok = 0u32;

    let mut bump_meta = |pid: u32, add_conn: u32, add_weight: u32| -> Option<String> {
        if pid == 0 || pid == 4 {
            return None;
        }
        let (name, path) = pid_cache
            .entry(pid)
            .or_insert_with(|| process_image(pid))
            .clone()
            .unwrap_or_else(|| {
                let n = format!("pid-{pid}");
                (n.clone(), String::new())
            });
        let key = if path.is_empty() {
            format!("pid:{pid}")
        } else {
            path_key(&path)
        };
        let e = by_path.entry(key.clone()).or_insert(PathMeta {
            pid,
            name,
            path,
            conns: 0,
            weight: 0,
        });
        e.conns = e.conns.saturating_add(add_conn);
        e.weight = e.weight.saturating_add(add_weight);
        Some(key)
    };

    for row in read_tcp_v4() {
        let estab = row.dwState == MIB_TCP_STATE_ESTAB;
        let w = if estab { 3 } else { 0 };
        let Some(pkey) = bump_meta(row.dwOwningPid, 1, w) else {
            continue;
        };
        if !try_estats || !estab || estats_budget == 0 {
            continue;
        }
        estats_tried = estats_tried.saturating_add(1);
        if let Some((r, t)) = estats_v4(&row) {
            estats_ok = estats_ok.saturating_add(1);
            estats_budget = estats_budget.saturating_sub(1);
            let ck = ConnKey::V4 {
                local_addr: row.dwLocalAddr,
                local_port: row.dwLocalPort,
                remote_addr: row.dwRemoteAddr,
                remote_port: row.dwRemotePort,
            };
            by_conn.insert(ck, (pkey, r, t));
        }
    }
    for row in read_tcp_v6() {
        let estab = row.dwState == MIB_TCP_STATE_ESTAB;
        let w = if estab { 3 } else { 0 };
        let Some(pkey) = bump_meta(row.dwOwningPid, 1, w) else {
            continue;
        };
        if !try_estats || !estab || estats_budget == 0 {
            continue;
        }
        estats_tried = estats_tried.saturating_add(1);
        if let Some((r, t)) = estats_v6(&row) {
            estats_ok = estats_ok.saturating_add(1);
            estats_budget = estats_budget.saturating_sub(1);
            let ck = ConnKey::V6 {
                local: row.ucLocalAddr,
                local_port: row.dwLocalPort,
                local_scope: row.dwLocalScopeId,
                remote: row.ucRemoteAddr,
                remote_port: row.dwRemotePort,
                remote_scope: row.dwRemoteScopeId,
            };
            by_conn.insert(ck, (pkey, r, t));
        }
    }
    for row in read_udp_v4() {
        let _ = bump_meta(row.dwOwningPid, 1, 1);
    }
    for row in read_udp_v6() {
        let _ = bump_meta(row.dwOwningPid, 1, 1);
    }

    if try_estats && estats_tried > 0 {
        estats_mode = if estats_ok > 0 {
            EstatsMode::Available
        } else {
            EstatsMode::Unavailable
        };
        *ESTATS_MODE.lock() = estats_mode;
    }

    (by_path, by_conn)
}

fn clamp_bps(bps: u64) -> u64 {
    bps.min(MAX_PROCESS_BPS)
}

fn share_nic_rates(
    paths: &HashMap<String, PathMeta>,
    nic_down: u64,
    nic_up: u64,
) -> HashMap<String, (u64, u64)> {
    let mut out = HashMap::new();
    let total_w: u64 = paths.values().map(|m| m.weight as u64).sum();
    if total_w == 0 || (nic_down == 0 && nic_up == 0) {
        return out;
    }
    for (key, meta) in paths {
        let w = meta.weight as u64;
        if w == 0 {
            continue;
        }
        out.insert(
            key.clone(),
            (
                clamp_bps(nic_down.saturating_mul(w) / total_w),
                clamp_bps(nic_up.saturating_mul(w) / total_w),
            ),
        );
    }
    out
}

/// Top processes by instant rate (ESTATS when available, else NIC share by sockets).
pub fn list_top(limit: usize) -> NetProcSnapshot {
    let limit = limit.clamp(5, 40);
    let blocked = load_blocked_paths();
    let (now_paths, now_conns) = sample_totals();
    let (nic_down, nic_up) = crate::win32::system_perf::last_net_rates();
    let mut warmed = false;
    let mut rates: HashMap<String, (u64, u64)> = HashMap::new();

    {
        let mut prev = PREV.lock();
        if let Some(p) = prev.as_ref() {
            warmed = true;
            let dt = p.at.elapsed().as_secs_f64().max(0.2);
            for (ck, (pkey, rx, tx)) in &now_conns {
                let Some(&(prx, ptx)) = p.by_conn.get(ck) else {
                    continue;
                };
                let d_rx = rx.saturating_sub(prx);
                let d_tx = tx.saturating_sub(ptx);
                let down = (d_rx as f64 / dt).round() as u64;
                let up = (d_tx as f64 / dt).round() as u64;
                let e = rates.entry(pkey.clone()).or_insert((0, 0));
                e.0 = e.0.saturating_add(down);
                e.1 = e.1.saturating_add(up);
            }
            for (_k, v) in rates.iter_mut() {
                v.0 = clamp_bps(v.0);
                v.1 = clamp_bps(v.1);
            }
        }
        *prev = Some(PrevAgg {
            at: Instant::now(),
            by_conn: now_conns
                .iter()
                .map(|(k, (_, rx, tx))| (k.clone(), (*rx, *tx)))
                .collect(),
        });
    }

    let estats_sum: u64 = rates.values().map(|(d, u)| d.saturating_add(*u)).sum();
    // Without elevation ESTATS usually yields nothing — share live NIC rates by socket weight.
    if estats_sum < 512 && nic_down.saturating_add(nic_up) >= 256 {
        rates = share_nic_rates(&now_paths, nic_down, nic_up);
        warmed = true;
    } else if estats_sum == 0 {
        warmed = true;
    }

    let mut rows: Vec<NetProcessRow> = now_paths
        .into_iter()
        .map(|(key, meta)| {
            let (down, up) = rates.get(&key).copied().unwrap_or((0, 0));
            let is_blocked = !meta.path.is_empty() && blocked.contains(&path_key(&meta.path));
            NetProcessRow {
                pid: meta.pid,
                name: meta.name,
                path: meta.path,
                down_bps: down,
                up_bps: up,
                connections: meta.conns,
                blocked: is_blocked,
                icon_png: None,
            }
        })
        .collect();

    // Include blocked apps with no current sockets.
    for b in &blocked {
        if rows.iter().any(|r| path_key(&r.path) == *b) {
            continue;
        }
        let name = Path::new(b)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| b.clone());
        rows.push(NetProcessRow {
            pid: 0,
            name,
            path: b.clone(),
            down_bps: 0,
            up_bps: 0,
            connections: 0,
            blocked: true,
            icon_png: None,
        });
    }

    rows.sort_by(|a, b| {
        let ta = a.down_bps.saturating_add(a.up_bps);
        let tb = b.down_bps.saturating_add(b.up_bps);
        tb.cmp(&ta)
            .then_with(|| b.connections.cmp(&a.connections))
            .then_with(|| a.name.cmp(&b.name))
    });

    // Prefer showing blocked + active; truncate after sort but keep all blocked visible.
    if rows.len() > limit {
        let mut kept = Vec::with_capacity(limit);
        for r in rows.drain(..) {
            if kept.len() < limit || r.blocked {
                kept.push(r);
            }
        }
        kept.sort_by(|a, b| {
            let ta = a.down_bps.saturating_add(a.up_bps);
            let tb = b.down_bps.saturating_add(b.up_bps);
            tb.cmp(&ta)
                .then_with(|| b.blocked.cmp(&a.blocked))
                .then_with(|| a.name.cmp(&b.name))
        });
        if kept.len() > limit + 8 {
            kept.truncate(limit + 8);
        }
        rows = kept;
    }

    for row in &mut rows {
        row.icon_png = icon_for_exe(&row.path);
    }

    NetProcSnapshot {
        processes: rows,
        warmed,
    }
}

fn fw_policy() -> Result<
    windows::Win32::NetworkManagement::WindowsFirewall::INetFwPolicy2,
    String,
> {
    use windows::Win32::NetworkManagement::WindowsFirewall::NetFwPolicy2;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())
    }
}

fn fw_add_rules_com(path: &str) -> Result<(), String> {
    use windows::core::BSTR;
    use windows::Win32::Foundation::VARIANT_BOOL;
    use windows::Win32::NetworkManagement::WindowsFirewall::{
        INetFwRule, NetFwRule, NET_FW_ACTION_BLOCK, NET_FW_IP_PROTOCOL_ANY, NET_FW_PROFILE2_ALL,
        NET_FW_RULE_DIR_IN, NET_FW_RULE_DIR_OUT,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

    let (out_name, in_name) = rule_names(path);
    let policy = fw_policy()?;
    let rules = unsafe { policy.Rules() }.map_err(|e| e.to_string())?;

    let add_one = |name: &str, dir| -> Result<(), String> {
        unsafe {
            let _ = rules.Remove(&BSTR::from(name));
            let rule: INetFwRule =
                CoCreateInstance(&NetFwRule, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
            rule.SetName(&BSTR::from(name)).map_err(|e| e.to_string())?;
            rule.SetDescription(&BSTR::from("Window Hub 禁止程序联网"))
                .map_err(|e| e.to_string())?;
            rule.SetApplicationName(&BSTR::from(path))
                .map_err(|e| e.to_string())?;
            rule.SetProtocol(NET_FW_IP_PROTOCOL_ANY.0)
                .map_err(|e| e.to_string())?;
            rule.SetDirection(dir).map_err(|e| e.to_string())?;
            rule.SetAction(NET_FW_ACTION_BLOCK)
                .map_err(|e| e.to_string())?;
            rule.SetEnabled(VARIANT_BOOL(-1))
                .map_err(|e| e.to_string())?;
            rule.SetGrouping(&BSTR::from(RULE_GROUP))
                .map_err(|e| e.to_string())?;
            rule.SetProfiles(NET_FW_PROFILE2_ALL.0)
                .map_err(|e| e.to_string())?;
            rules.Add(&rule).map_err(|e| e.to_string())?;
        }
        Ok(())
    };

    add_one(&out_name, NET_FW_RULE_DIR_OUT)?;
    add_one(&in_name, NET_FW_RULE_DIR_IN)?;
    Ok(())
}

fn fw_remove_rules_com(path: &str) -> Result<(), String> {
    use windows::core::BSTR;
    let (out_name, in_name) = rule_names(path);
    let policy = fw_policy()?;
    let rules = unsafe { policy.Rules() }.map_err(|e| e.to_string())?;
    unsafe {
        let _ = rules.Remove(&BSTR::from(out_name.as_str()));
        let _ = rules.Remove(&BSTR::from(in_name.as_str()));
    }
    Ok(())
}

fn elevate_cmd_script(script_body: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let bat = std::env::temp_dir().join(format!(
        "wh-netfw-{}-{}.cmd",
        std::process::id(),
        path_hash(script_body)
    ));
    std::fs::write(&bat, script_body).map_err(|e| e.to_string())?;
    let bat_s = bat.to_string_lossy().replace('\'', "''");
    let ps = format!(
        "Start-Process -FilePath cmd.exe -ArgumentList '/c \"\"{bat_s}\"\"' -Verb RunAs -Wait -WindowStyle Hidden"
    );
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&bat);
    if !output.status.success() {
        return Err("需要管理员权限才能修改防火墙（可能已取消 UAC）".into());
    }
    Ok(())
}

fn fw_add_rules_elevated(path: &str) -> Result<(), String> {
    let (out_name, in_name) = rule_names(path);
    let prog = path.replace('"', "");
    let out_n = out_name.replace('"', "");
    let in_n = in_name.replace('"', "");
    let body = format!(
        "@echo off\r\n\
netsh advfirewall firewall delete rule name=\"{out_n}\" >nul 2>&1\r\n\
netsh advfirewall firewall delete rule name=\"{in_n}\" >nul 2>&1\r\n\
netsh advfirewall firewall add rule name=\"{out_n}\" dir=out action=block program=\"{prog}\" enable=yes profile=any\r\n\
netsh advfirewall firewall add rule name=\"{in_n}\" dir=in action=block program=\"{prog}\" enable=yes profile=any\r\n\
if errorlevel 1 exit /b 1\r\n"
    );
    elevate_cmd_script(&body)
}

fn fw_remove_rules_elevated(path: &str) -> Result<(), String> {
    let (out_name, in_name) = rule_names(path);
    let out_n = out_name.replace('"', "");
    let in_n = in_name.replace('"', "");
    let body = format!(
        "@echo off\r\n\
netsh advfirewall firewall delete rule name=\"{out_n}\"\r\n\
netsh advfirewall firewall delete rule name=\"{in_n}\"\r\n"
    );
    elevate_cmd_script(&body)
}

fn validate_block_path(path: &str) -> Result<String, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("缺少程序路径".into());
    }
    let key = path_key(path);
    if let Some(self_key) = self_exe_key() {
        if key == self_key {
            return Err("不能禁止 Window Hub 自身联网".into());
        }
    }
    let low = key.as_str();
    if low.ends_with("\\system\\") || low == "system" || low.ends_with("\\csrss.exe") {
        return Err("不能禁止系统关键进程联网".into());
    }
    Ok(path.to_string())
}

/// Best-effort: tear down IPv4 TCP for this exe so block takes effect immediately.
fn drop_tcp_for_path(path: &str) {
    use windows::Win32::NetworkManagement::IpHelper::{
        SetTcpEntry, MIB_TCPROW_LH, MIB_TCPROW_LH_0, MIB_TCP_STATE_DELETE_TCB,
    };
    let key = path_key(path);
    let mut pid_ok: HashMap<u32, bool> = HashMap::new();
    for row in read_tcp_v4() {
        let match_pid = *pid_ok.entry(row.dwOwningPid).or_insert_with(|| {
            process_image(row.dwOwningPid)
                .map(|(_, p)| path_key(&p) == key)
                .unwrap_or(false)
        });
        if !match_pid {
            continue;
        }
        unsafe {
            let lh = MIB_TCPROW_LH {
                Anonymous: MIB_TCPROW_LH_0 {
                    State: MIB_TCP_STATE_DELETE_TCB,
                },
                dwLocalAddr: row.dwLocalAddr,
                dwLocalPort: row.dwLocalPort,
                dwRemoteAddr: row.dwRemoteAddr,
                dwRemotePort: row.dwRemotePort,
            };
            let _ = SetTcpEntry(&lh);
        }
    }
}

/// Block or unblock an exe via Windows Firewall (may prompt UAC).
pub fn set_blocked(path: &str, blocked: bool) -> Result<NetProcessRow, String> {
    let path = validate_block_path(path)?;
    let name = Path::new(&path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());

    if blocked {
        if let Err(e) = fw_add_rules_com(&path) {
            // Access denied / not elevated → one UAC for both rules.
            let _ = e;
            fw_add_rules_elevated(&path)?;
        }
        drop_tcp_for_path(&path);
        let mut set = load_blocked_paths();
        set.insert(path_key(&path));
        save_blocked_paths(&set)?;
    } else {
        if fw_remove_rules_com(&path).is_err() {
            let _ = fw_remove_rules_elevated(&path);
        }
        let mut set = load_blocked_paths();
        set.remove(&path_key(&path));
        save_blocked_paths(&set)?;
    }

    Ok(NetProcessRow {
        pid: 0,
        name,
        path: path.clone(),
        down_bps: 0,
        up_bps: 0,
        connections: 0,
        blocked,
        icon_png: icon_for_exe(&path),
    })
}
