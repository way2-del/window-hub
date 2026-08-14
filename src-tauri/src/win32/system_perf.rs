//! CPU / memory collectors + isolated temperature collector.
//! Temperature (PowerShell CIM / nvidia-smi) must never run on the UI/IPC path.

#![cfg(windows)]

use parking_lot::Mutex;
use serde::Serialize;
use std::time::Instant;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfSnapshot {
    /// Physical memory in use, 0–100.
    pub mem_percent: u8,
    /// Approx CPU utilization 0–100 (delta of GetSystemTimes).
    pub cpu_percent: u8,
    /// Best-effort CPU / ACPI zone °C (filled by TemperatureService).
    pub cpu_temp_c: Option<u8>,
    /// Best-effort GPU °C when reported.
    pub gpu_temp_c: Option<u8>,
    /// Dedicated GPU memory used percent (optional).
    pub gpu_mem_percent: Option<u8>,
    /// Instant download rate (bytes/sec), all non-loopback adapters.
    pub down_bps: u64,
    /// Instant upload rate (bytes/sec).
    pub up_bps: u64,
    /// Bytes received since Window Hub started (sum of positive deltas).
    pub session_rx_bytes: u64,
    /// Bytes sent since Window Hub started.
    pub session_tx_bytes: u64,
}

static CPU_TIMES: Mutex<Option<(Instant, (u64, u64))>> = Mutex::new(None);

struct NetSample {
    at: Instant,
    rx: u64,
    tx: u64,
}

static NET_PREV: Mutex<Option<NetSample>> = Mutex::new(None);
static NET_SESSION: Mutex<(u64, u64)> = Mutex::new((0, 0));
static NET_LAST: Mutex<(u64, u64, u64, u64)> = Mutex::new((0, 0, 0, 0));

fn filetime_u64(ft: windows::Win32::Foundation::FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

fn mem_percent() -> u8 {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    unsafe {
        let mut st = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        if GlobalMemoryStatusEx(&mut st).is_err() {
            return 0;
        }
        st.dwMemoryLoad.min(100) as u8
    }
}

fn cpu_times() -> Option<(u64, u64)> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::GetSystemTimes;
    unsafe {
        let mut idle = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        if GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).is_err() {
            return None;
        }
        let idle_t = filetime_u64(idle);
        let total = filetime_u64(kernel).saturating_add(filetime_u64(user));
        Some((idle_t, total))
    }
}

fn cpu_percent_from_delta(prev: (u64, u64), now: (u64, u64)) -> u8 {
    let idle_d = now.0.saturating_sub(prev.0);
    let total_d = now.1.saturating_sub(prev.1);
    if total_d == 0 {
        return 0;
    }
    let busy = total_d.saturating_sub(idle_d) as f64;
    ((busy / total_d as f64) * 100.0).round().clamp(0.0, 100.0) as u8
}

/// CPU + memory only — safe for light background loop. No temperature.
/// Net fields are filled from the last `collect_net` sample (zeros until first net tick).
pub fn collect_cpu_mem() -> PerfSnapshot {
    let mem = mem_percent();
    let now_times = cpu_times();
    let mut cpu = 0u8;
    {
        let mut guard = CPU_TIMES.lock();
        if let Some(now) = now_times {
            if let Some((_, prev)) = guard.as_ref() {
                if prev.1 > 0 {
                    cpu = cpu_percent_from_delta(*prev, now);
                }
            }
            *guard = Some((Instant::now(), now));
        }
    }

    let (down_bps, up_bps, session_rx_bytes, session_tx_bytes) = *NET_LAST.lock();

    PerfSnapshot {
        mem_percent: mem,
        cpu_percent: cpu,
        cpu_temp_c: None,
        gpu_temp_c: None,
        gpu_mem_percent: None,
        down_bps,
        up_bps,
        session_rx_bytes,
        session_tx_bytes,
    }
}

fn iface_octets_total() -> Option<(u64, u64)> {
    use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;

    // IF_TYPE_SOFTWARE_LOOPBACK
    const IF_LOOPBACK: u32 = 24;
    // MIB_IF_ROW2.InterfaceAndOperStatusFlags.FilterInterface
    const FLAG_FILTER_IF: u8 = 0x02;

    // 不经 catch_unwind：GetIfTable2 是常规 FFI， unwind 穿过会 UB；失败用返回值判断即可
    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut table).is_err() || table.is_null() {
            return None;
        }
        let t = &*table;
        let n = (t.NumEntries as usize).min(512);
        let rows = std::slice::from_raw_parts(t.Table.as_ptr(), n);
        let mut rx = 0u64;
        let mut tx = 0u64;
        for row in rows {
            if row.Type == IF_LOOPBACK {
                continue;
            }
            // 跳过 NDIS 过滤驱动层，避免同一块 WLAN 被加 5～6 次
            if (row.InterfaceAndOperStatusFlags._bitfield & FLAG_FILTER_IF) != 0 {
                continue;
            }
            if row.OperStatus != IfOperStatusUp {
                continue;
            }
            rx = rx.saturating_add(row.InOctets);
            tx = tx.saturating_add(row.OutOctets);
        }
        FreeMibTable(table as *const _);
        Some((rx, tx))
    }
}

/// Last instant rates from background `collect_net` (does not advance the sampler).
pub fn last_net_rates() -> (u64, u64) {
    let (down, up, _, _) = *NET_LAST.lock();
    (down, up)
}

/// Instant rates + session totals. Call ~1 Hz from background net loop.
pub fn collect_net() -> (u64, u64, u64, u64) {
    let Some((rx, tx)) = iface_octets_total() else {
        let last = *NET_LAST.lock();
        return last;
    };
    let mut down_bps = 0u64;
    let mut up_bps = 0u64;
    let mut d_rx = 0u64;
    let mut d_tx = 0u64;
    {
        let mut prev = NET_PREV.lock();
        if let Some(p) = prev.as_ref() {
            let dt = p.at.elapsed().as_secs_f64().max(0.05);
            if rx >= p.rx {
                d_rx = rx - p.rx;
                down_bps = (d_rx as f64 / dt).round() as u64;
            }
            if tx >= p.tx {
                d_tx = tx - p.tx;
                up_bps = (d_tx as f64 / dt).round() as u64;
            }
        }
        *prev = Some(NetSample {
            at: Instant::now(),
            rx,
            tx,
        });
    }
    let (session_rx, session_tx) = {
        let mut s = NET_SESSION.lock();
        s.0 = s.0.saturating_add(d_rx);
        s.1 = s.1.saturating_add(d_tx);
        *s
    };
    let out = (down_bps, up_bps, session_rx, session_tx);
    *NET_LAST.lock() = out;
    out
}

/// Heavy thermal probe — only from TemperatureService background thread.
pub fn collect_temperatures() -> (Option<u8>, Option<u8>) {
    let (cpu, gpu_fallback) = thermal_via_powershell();
    let gpu = gpu_temp_nvidia_smi().or(gpu_fallback);
    (cpu, gpu)
}

fn parse_temp_c(raw: &str) -> Option<u8> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    let head = t.split(|c: char| c == '.' || c == ',').next().unwrap_or(t);
    let c: u8 = head.trim().parse().ok()?;
    if c > 120 {
        None
    } else {
        Some(c)
    }
}

fn gpu_temp_nvidia_smi() -> Option<u8> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=temperature.gpu",
            "--format=csv,noheader,nounits",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if let Some(c) = parse_temp_c(line) {
            return Some(c);
        }
    }
    None
}

fn thermal_via_powershell() -> (Option<u8>, Option<u8>) {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let script = r#"
$ErrorActionPreference='SilentlyContinue'
$out = @()
$zones = Get-CimInstance -Namespace root/wmi -ClassName MSAcpi_ThermalZoneTemperature
foreach ($z in $zones) {
  $k10 = [int]$z.CurrentTemperature
  if ($k10 -gt 2730) {
    $c = [int](($k10 / 10) - 273)
    if ($c -ge 0 -and $c -le 120) {
      $out += ("ACPI|{0}|{1}" -f ([string]$z.InstanceName), $c)
    }
  }
}
foreach ($ns in @('root/LibreHardwareMonitor','root/OpenHardwareMonitor')) {
  $sensors = Get-CimInstance -Namespace $ns -ClassName Sensor |
    Where-Object { $_.SensorType -eq 'Temperature' }
  foreach ($s in $sensors) {
    $c = [int][math]::Round([double]$s.Value)
    if ($c -ge 0 -and $c -le 120) {
      $out += ("LHM|{0}|{1}" -f ([string]$s.Name), $c)
    }
  }
}
$out -join "`n"
"#;

    let Ok(output) = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return (None, None);
    };
    if !output.status.success() {
        return (None, None);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut cpu = None;
    let mut gpu = None;
    for line in text.lines() {
        let mut parts = line.splitn(3, '|');
        let Some(kind) = parts.next() else { continue };
        let Some(name) = parts.next() else { continue };
        let Some(temp) = parts.next() else { continue };
        let Some(c) = parse_temp_c(temp) else { continue };
        let low = name.to_ascii_lowercase();
        let looks_gpu = low.contains("gpu")
            || low.contains("gfx")
            || low.contains("video")
            || low.contains("hot spot")
            || low.contains("hotspot");
        // LHM / OHM: package / Tctl / CCD / die are common CPU package sensors.
        let looks_cpu = low.contains("cpu")
            || low.contains("core")
            || low.contains("package")
            || low.contains("tctl")
            || low.contains("tdie")
            || low.contains("ccd")
            || low.contains("cpu die")
            || low.contains("cpu (tctl");
        match kind {
            "LHM" if looks_gpu && gpu.is_none() => gpu = Some(c),
            "LHM" if !looks_gpu && cpu.is_none() && looks_cpu => {
                cpu = Some(c);
            }
            "ACPI" if looks_gpu && gpu.is_none() => gpu = Some(c),
            "ACPI" if !looks_gpu && cpu.is_none() => cpu = Some(c),
            _ => {}
        }
    }
    if cpu.is_none() {
        for line in text.lines() {
            let mut parts = line.splitn(3, '|');
            let Some(kind) = parts.next() else { continue };
            if kind != "ACPI" {
                continue;
            }
            let _ = parts.next();
            if let Some(temp) = parts.next() {
                if let Some(c) = parse_temp_c(temp) {
                    cpu = Some(c);
                    break;
                }
            }
        }
    }
    (cpu, gpu)
}
