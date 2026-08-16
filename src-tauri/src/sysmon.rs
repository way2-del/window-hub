//! Host system monitor snapshot (CPU / memory / disks / temperatures).
//! Temperatures are best-effort on Windows; `effectiveTempC` = max(CPU, GPU).

use parking_lot::Mutex;
use serde::Serialize;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use sysinfo::{Components, Disks, RefreshKind, System};

const MIN_REFRESH: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuInfo {
    pub usage_pct: f32,
    pub frequency_mhz: u64,
    pub brand: String,
    pub core_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemInfo {
    pub used_bytes: u64,
    pub total_bytes: u64,
    pub usage_pct: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskInfo {
    pub name: String,
    pub mount: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
    pub usage_pct: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TempReading {
    pub label: String,
    pub celsius: f32,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SysmonSnapshot {
    pub cpu: CpuInfo,
    pub memory: MemInfo,
    pub disks: Vec<DiskInfo>,
    pub temperatures: Vec<TempReading>,
    /// CPU package / die estimate (°C), if available.
    pub cpu_temp_c: Option<f32>,
    /// GPU estimate (°C), if available.
    pub gpu_temp_c: Option<f32>,
    /// `max(cpuTempC, gpuTempC)` for fan mapping.
    pub effective_temp_c: Option<f32>,
    pub updated_at_ms: u64,
}

struct SysmonState {
    system: System,
    components: Components,
    disks: Disks,
    last: Instant,
    cached: Option<SysmonSnapshot>,
}

fn state() -> &'static Mutex<SysmonState> {
    static STATE: OnceLock<Mutex<SysmonState>> = OnceLock::new();
    STATE.get_or_init(|| {
        let refresh = RefreshKind::everything();
        Mutex::new(SysmonState {
            system: System::new_with_specifics(refresh),
            components: Components::new_with_refreshed_list(),
            disks: Disks::new_with_refreshed_list(),
            last: Instant::now()
                .checked_sub(Duration::from_secs(10))
                .unwrap_or_else(Instant::now),
            cached: None,
        })
    })
}

fn classify_temp_kind(label: &str) -> &'static str {
    let l = label.to_ascii_lowercase();
    if l.contains("gpu")
        || l.contains("nvidia")
        || l.contains("radeon")
        || l.contains("geforce")
        || l.contains("amd radeon")
        || l.contains("intel arc")
    {
        "gpu"
    } else if l.contains("cpu")
        || l.contains("package")
        || l.contains("core")
        || l.contains("tdie")
        || l.contains("tctl")
        || l.contains("processor")
    {
        "cpu"
    } else if l.contains("nvme") || l.contains("ssd") || l.contains("hdd") || l.contains("drive")
    {
        "disk"
    } else if l.contains("motherboard") || l.contains("system") || l.contains("acpi")
    {
        "board"
    } else {
        "other"
    }
}

fn pick_max(temps: &[TempReading], kind: &str) -> Option<f32> {
    temps
        .iter()
        .filter(|t| t.kind == kind)
        .map(|t| t.celsius)
        .filter(|c| c.is_finite() && *c > -20.0 && *c < 150.0)
        .fold(None, |acc: Option<f32>, c| {
            Some(acc.map_or(c, |a| a.max(c)))
        })
}

#[cfg(windows)]
fn nvidia_gpu_temp_c() -> Option<f32> {
    // Optional NVML — present when NVIDIA driver is installed.
    use std::ffi::c_void;
    use windows::core::s;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

    type NvmlInit = unsafe extern "C" fn() -> i32;
    type NvmlShutdown = unsafe extern "C" fn() -> i32;
    type NvmlDeviceGetHandleByIndex = unsafe extern "C" fn(u32, *mut *mut c_void) -> i32;
    type NvmlDeviceGetTemperature = unsafe extern "C" fn(*mut c_void, u32, *mut u32) -> i32;

    unsafe {
        let Ok(lib) = LoadLibraryA(s!("nvml.dll")) else {
            return None;
        };
        let init: NvmlInit = std::mem::transmute(GetProcAddress(lib, s!("nvmlInit_v2"))?);
        let shutdown: NvmlShutdown =
            std::mem::transmute(GetProcAddress(lib, s!("nvmlShutdown"))?);
        let by_index: NvmlDeviceGetHandleByIndex =
            std::mem::transmute(GetProcAddress(lib, s!("nvmlDeviceGetHandleByIndex_v2"))?);
        let get_temp: NvmlDeviceGetTemperature =
            std::mem::transmute(GetProcAddress(lib, s!("nvmlDeviceGetTemperature"))?);

        if init() != 0 {
            return None;
        }
        let mut device: *mut c_void = std::ptr::null_mut();
        let ok = by_index(0, &mut device) == 0 && !device.is_null();
        let mut out = None;
        if ok {
            let mut temp = 0u32;
            // 0 = NVML_TEMPERATURE_GPU
            if get_temp(device, 0, &mut temp) == 0 {
                out = Some(temp as f32);
            }
        }
        let _ = shutdown();
        // Keep nvml.dll loaded for subsequent samples (FreeLibrary not linked in this crate).
        let _ = lib;
        out
    }
}

#[cfg(not(windows))]
fn nvidia_gpu_temp_c() -> Option<f32> {
    None
}

fn build_snapshot(st: &mut SysmonState) -> SysmonSnapshot {
    // First CPU refresh seeds; second yields a meaningful usage %.
    st.system.refresh_cpu_all();
    st.system.refresh_memory();
    st.disks.refresh(true);
    st.components.refresh(true);

    let cpus = st.system.cpus();
    let usage_pct = if cpus.is_empty() {
        0.0
    } else {
        cpus.iter().map(|c| c.cpu_usage()).sum::<f32>() / cpus.len() as f32
    };
    let frequency_mhz = cpus.first().map(|c| c.frequency()).unwrap_or(0);
    let brand = cpus
        .first()
        .map(|c| c.brand().trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "CPU".into());

    let total = st.system.total_memory();
    let used = st.system.used_memory();
    let mem_pct = if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64 * 100.0) as f32
    };

    let mut disks = Vec::new();
    for d in st.disks.list() {
        let total_b = d.total_space();
        let avail = d.available_space();
        if total_b == 0 {
            continue;
        }
        let used_b = total_b.saturating_sub(avail);
        let mount = d.mount_point().to_string_lossy().to_string();
        // Skip pseudo mounts.
        if mount.starts_with("\\\\") && !mount.chars().nth(2).is_some_and(|c| c == '?') {
            // keep UNC if needed; skip empty
        }
        let name = d.name().to_string_lossy().to_string();
        let label = if name.trim().is_empty() {
            mount.clone()
        } else {
            name
        };
        disks.push(DiskInfo {
            name: label,
            mount,
            total_bytes: total_b,
            available_bytes: avail,
            used_bytes: used_b,
            usage_pct: (used_b as f64 / total_b as f64 * 100.0) as f32,
        });
    }
    disks.sort_by(|a, b| a.mount.cmp(&b.mount));

    let mut temperatures = Vec::new();
    for c in st.components.list() {
        let Some(temp) = c.temperature() else {
            continue;
        };
        if !temp.is_finite() || temp <= 0.0 || temp > 150.0 {
            continue;
        }
        let label = c.label().trim().to_string();
        if label.is_empty() {
            continue;
        }
        let kind = classify_temp_kind(&label).to_string();
        temperatures.push(TempReading {
            label,
            celsius: temp,
            kind,
        });
    }

    if let Some(gpu) = nvidia_gpu_temp_c() {
        if temperatures
            .iter()
            .filter(|t| t.kind == "gpu")
            .all(|t| (t.celsius - gpu).abs() > 0.5)
        {
            temperatures.push(TempReading {
                label: "NVIDIA GPU".into(),
                celsius: gpu,
                kind: "gpu".into(),
            });
        }
    }

    let cpu_temp_c = pick_max(&temperatures, "cpu").or_else(|| {
        // Fall back to any board/acpi reading when CPU sensors are missing.
        pick_max(&temperatures, "board").or_else(|| pick_max(&temperatures, "other"))
    });
    let gpu_temp_c = pick_max(&temperatures, "gpu");
    let effective_temp_c = match (cpu_temp_c, gpu_temp_c) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };

    let updated_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    SysmonSnapshot {
        cpu: CpuInfo {
            usage_pct,
            frequency_mhz,
            brand,
            core_count: cpus.len(),
        },
        memory: MemInfo {
            used_bytes: used,
            total_bytes: total,
            usage_pct: mem_pct,
        },
        disks,
        temperatures,
        cpu_temp_c,
        gpu_temp_c,
        effective_temp_c,
        updated_at_ms,
    }
}

/// Refresh (throttled) and return a snapshot for CapGate.
pub fn snapshot() -> SysmonSnapshot {
    let mut g = state().lock();
    if let Some(ref cached) = g.cached {
        if g.last.elapsed() < MIN_REFRESH {
            return cached.clone();
        }
    }
    // Seed usage on first call, then sample again after a short sleep.
    if g.cached.is_none() {
        g.system.refresh_cpu_all();
        drop(g);
        std::thread::sleep(Duration::from_millis(120));
        g = state().lock();
    }
    let snap = build_snapshot(&mut g);
    g.last = Instant::now();
    g.cached = Some(snap.clone());
    snap
}
