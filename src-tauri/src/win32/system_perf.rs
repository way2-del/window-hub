//! CPU / memory collectors + isolated temperature collector.
//! Temperature (LHM / ACPI / nvidia-smi) must never run on the UI/IPC path.
//! Prefer LibreHardwareMonitor (HTTP :8085 or WMI) for real CPU die/package temps;
//! MSAcpi zones are a last-resort fallback and often need elevation or are EC-only.

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
    // LHM first (real package/die). ACPI zones are often EC chassis and may need admin.
    let (lhm_cpu, lhm_gpu) = thermal_via_lhm();
    let gpu = gpu_temp_nvidia_smi().or(lhm_gpu);
    let cpu = lhm_cpu.or_else(cpu_temp_acpi_powershell);
    (cpu, gpu)
}

fn parse_temp_c(raw: &str) -> Option<u8> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    // LHM HTTP values look like "45.0 °C"
    let cleaned = t.trim_start_matches(|c: char| !(c.is_ascii_digit() || c == '-' || c == '.' || c == ','));
    let head = cleaned
        .split(|c: char| c == '.' || c == ',' || c == ' ' || c == '°')
        .next()
        .unwrap_or(cleaned);
    let c: u8 = head.trim().parse().ok()?;
    if c > 120 {
        None
    } else {
        Some(c)
    }
}

fn powershell_exe() -> &'static str {
    r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
}

/// Score LHM/OHM sensor names — higher wins (package/die over ambient/distance).
fn cpu_sensor_score(name: &str) -> i32 {
    let low = name.to_ascii_lowercase();
    if low.contains("distance")
        || low.contains("ambient")
        || low.contains("motherboard")
        || low.contains("chipset")
        || low.contains("sodimm")
        || low.contains("dimm")
        || low.contains("hdd")
        || low.contains("ssd")
        || low.contains("nvme")
        || low.contains("drive")
    {
        return -1;
    }
    if low.contains("package")
        || low.contains("tctl")
        || low.contains("tdie")
        || low.contains("cpu die")
    {
        return 100;
    }
    if low.contains("ccd") || low.contains("cpu (tctl") {
        return 90;
    }
    if low.contains("cpu") && low.contains("core") {
        return 70;
    }
    if low.contains("cpu") {
        return 60;
    }
    if low.contains("core") {
        return 40;
    }
    -1
}

fn gpu_sensor_score(name: &str) -> i32 {
    let low = name.to_ascii_lowercase();
    if low.contains("hot spot") || low.contains("hotspot") || low.contains("junction") {
        return 80;
    }
    if low.contains("gpu core") || low.contains("gpu temperature") {
        return 90;
    }
    if low.contains("gpu") || low.contains("gfx") || low.contains("video") {
        return 60;
    }
    -1
}

fn pick_best_temp(samples: &[(i32, u8)]) -> Option<u8> {
    samples
        .iter()
        .filter(|(score, _)| *score >= 0)
        .max_by_key(|(score, _)| *score)
        .map(|(_, c)| *c)
}

/// LibreHardwareMonitor remote JSON (Options → Remote Web Server, default :8085).
fn thermal_via_lhm_http() -> (Option<u8>, Option<u8>) {
    let mut cpu_samples: Vec<(i32, u8)> = Vec::new();
    let mut gpu_samples: Vec<(i32, u8)> = Vec::new();

    for port in [8085u16, 8086, 8090] {
        let url = format!("http://127.0.0.1:{port}/data.json");
        let Ok(resp) = ureq::get(&url)
            .timeout(std::time::Duration::from_millis(400))
            .call()
        else {
            continue;
        };
        let Ok(v) = resp.into_json::<serde_json::Value>() else {
            continue;
        };
        walk_lhm_json(&v, &mut cpu_samples, &mut gpu_samples);
        if !cpu_samples.is_empty() || !gpu_samples.is_empty() {
            break;
        }
    }

    (pick_best_temp(&cpu_samples), pick_best_temp(&gpu_samples))
}

fn walk_lhm_json(
    node: &serde_json::Value,
    cpu_samples: &mut Vec<(i32, u8)>,
    gpu_samples: &mut Vec<(i32, u8)>,
) {
    let sensor_type = node
        .get("SensorType")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let text = node
        .get("Text")
        .or_else(|| node.get("text"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let value = node
        .get("Value")
        .or_else(|| node.get("value"))
        .and_then(|x| x.as_str())
        .unwrap_or("");

    if sensor_type.eq_ignore_ascii_case("Temperature") {
        if let Some(c) = parse_temp_c(value) {
            let cs = cpu_sensor_score(text);
            if cs >= 0 {
                cpu_samples.push((cs, c));
            }
            let gs = gpu_sensor_score(text);
            if gs >= 0 {
                gpu_samples.push((gs, c));
            }
        }
    }

    if let Some(children) = node.get("Children").and_then(|c| c.as_array()) {
        for child in children {
            walk_lhm_json(child, cpu_samples, gpu_samples);
        }
    }
}

fn thermal_via_lhm() -> (Option<u8>, Option<u8>) {
    let http = thermal_via_lhm_http();
    if http.0.is_some() && http.1.is_some() {
        return http;
    }
    let wmi = thermal_via_lhm_powershell();
    (http.0.or(wmi.0), http.1.or(wmi.1))
}

/// MSAcpi thermal zones → °C (max zone). Often empty without elevation; not true package temp.
fn cpu_temp_acpi_powershell() -> Option<u8> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let script = concat!(
        "$ErrorActionPreference='SilentlyContinue';",
        "$vals=@();",
        "Get-CimInstance -Namespace root/wmi -ClassName MSAcpi_ThermalZoneTemperature | ",
        "ForEach-Object {",
        "  $c=[int]($_.CurrentTemperature/10-273);",
        "  if($c -ge 20 -and $c -le 120){$vals+=$c}",
        "};",
        "if($vals.Count -gt 0){($vals|Measure-Object -Maximum).Maximum}",
    );

    let output = Command::new(powershell_exe())
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_temp_c(&String::from_utf8_lossy(&output.stdout))
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

/// Optional LibreHardwareMonitor / OpenHardwareMonitor WMI sensors.
fn thermal_via_lhm_powershell() -> (Option<u8>, Option<u8>) {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let script = r#"
$ErrorActionPreference='SilentlyContinue'
$out = @()
foreach ($ns in @('root/LibreHardwareMonitor','root/OpenHardwareMonitor')) {
  try {
    $sensors = Get-CimInstance -Namespace $ns -ClassName Sensor -ErrorAction Stop |
      Where-Object { $_.SensorType -eq 'Temperature' }
    foreach ($s in $sensors) {
      $c = [int][math]::Round([double]$s.Value)
      if ($c -ge 0 -and $c -le 120) {
        $out += ("LHM|{0}|{1}" -f ([string]$s.Name), $c)
      }
    }
  } catch {}
}
$out -join "`n"
"#;

    let Ok(output) = Command::new(powershell_exe())
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
    let mut cpu_samples: Vec<(i32, u8)> = Vec::new();
    let mut gpu_samples: Vec<(i32, u8)> = Vec::new();
    for line in text.lines() {
        let mut parts = line.splitn(3, '|');
        let Some(kind) = parts.next() else { continue };
        if kind != "LHM" {
            continue;
        }
        let Some(name) = parts.next() else { continue };
        let Some(temp) = parts.next() else { continue };
        let Some(c) = parse_temp_c(temp) else { continue };
        let cs = cpu_sensor_score(name);
        if cs >= 0 {
            cpu_samples.push((cs, c));
        }
        let gs = gpu_sensor_score(name);
        if gs >= 0 {
            gpu_samples.push((gs, c));
        }
    }
    (pick_best_temp(&cpu_samples), pick_best_temp(&gpu_samples))
}
