//! Background SystemMonitorService — UI only reads caches; collectors never run on IPC/UI path.
//!
//! Domains refresh independently with separate TTLs. Heavy work (WiFi / BT / temperature)
//! cannot stall volume/power/IME chip reads.

#![cfg(windows)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter};

use crate::win32::system_audio;
use crate::win32::system_perf;
use crate::win32::system_power;
use crate::win32::system_radio::{
    self, BluetoothSnapshot, ImeSnapshot, PeripheralIcon, SystemRadioSnapshot, WifiSnapshot,
};

/// Light state: volume / power / IME / CPU·mem — background cadence.
const TTL_LIGHT: Duration = Duration::from_secs(15);
/// Heavy state: WiFi list / Bluetooth devices.
const TTL_HEAVY: Duration = Duration::from_secs(45);
/// Temperature (PowerShell / nvidia-smi) — isolated, low frequency.
const TTL_TEMP: Duration = Duration::from_secs(30);
/// IME / Caps — cheap; keep near-instant so 中/英/A·a 切换跟手。
const TTL_IME: Duration = Duration::from_millis(500);

const TICK: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    Wifi,
    Bluetooth,
    Audio,
    Power,
    Perf,
    Temperature,
    Ime,
}

struct Timed<T> {
    at: Instant,
    value: T,
}

struct Caches {
    wifi: Timed<WifiSnapshot>,
    bluetooth: Timed<BluetoothSnapshot>,
    ime: Timed<ImeSnapshot>,
    volume: Timed<system_audio::VolumeSnapshot>,
    power: Timed<system_power::PowerSnapshot>,
    perf: Timed<system_perf::PerfSnapshot>,
    /// Independent of `perf.at` — light CPU/mem refresh must not reset temp TTL.
    temp_at: Instant,
    peripherals: Timed<Vec<PeripheralIcon>>,
}

impl Caches {
    fn empty() -> Self {
        let now = Instant::now()
            .checked_sub(Duration::from_secs(3600))
            .unwrap_or_else(Instant::now);
        Self {
            wifi: Timed {
                at: now,
                value: system_radio::empty_wifi(),
            },
            bluetooth: Timed {
                at: now,
                value: system_radio::empty_bt(),
            },
            ime: Timed {
                at: now,
                value: system_radio::empty_ime(),
            },
            volume: Timed {
                at: now,
                value: system_audio::VolumeSnapshot {
                    level: 0,
                    muted: true,
                },
            },
            power: Timed {
                at: now,
                value: system_power::PowerSnapshot {
                    ac_line: "unknown".into(),
                    percent: None,
                    charging: false,
                    life_sec: None,
                    has_battery: false,
                },
            },
            perf: Timed {
                at: now,
                value: system_perf::PerfSnapshot {
                    mem_percent: 0,
                    cpu_percent: 0,
                    cpu_temp_c: None,
                    gpu_temp_c: None,
                    gpu_mem_percent: None,
                },
            },
            temp_at: now,
            peripherals: Timed {
                at: now,
                value: Vec::new(),
            },
        }
    }

    fn to_snapshot(&self) -> SystemRadioSnapshot {
        SystemRadioSnapshot {
            wifi: self.wifi.value.clone(),
            bluetooth: self.bluetooth.value.clone(),
            ime: self.ime.value.clone(),
            volume: self.volume.value.clone(),
            power: self.power.value.clone(),
            perf: self.perf.value.clone(),
            peripherals: self.peripherals.value.clone(),
        }
    }
}

struct MonitorState {
    caches: Mutex<Caches>,
    app: Mutex<Option<AppHandle>>,
    started: AtomicBool,
    /// Priority refresh requests from UI (non-blocking).
    want_wifi: AtomicBool,
    want_bt: AtomicBool,
    want_light: AtomicBool,
    want_temp: AtomicBool,
    /// Collectors in flight — UI still reads last good cache.
    busy_wifi: AtomicBool,
    busy_bt: AtomicBool,
    busy_temp: AtomicBool,
    busy_light: AtomicBool,
}

static MONITOR: OnceLock<MonitorState> = OnceLock::new();

fn state() -> &'static MonitorState {
    MONITOR.get_or_init(|| MonitorState {
        caches: Mutex::new(Caches::empty()),
        app: Mutex::new(None),
        started: AtomicBool::new(false),
        want_wifi: AtomicBool::new(false),
        want_bt: AtomicBool::new(false),
        want_light: AtomicBool::new(false),
        want_temp: AtomicBool::new(false),
        busy_wifi: AtomicBool::new(false),
        busy_bt: AtomicBool::new(false),
        busy_temp: AtomicBool::new(false),
        busy_light: AtomicBool::new(false),
    })
}

/// Start background collectors. Safe to call multiple times.
pub fn start(app: AppHandle) {
    let s = state();
    *s.app.lock() = Some(app);
    if s.started.swap(true, Ordering::SeqCst) {
        // Already running — kick a soft light refresh for warm start.
        request_refresh(&[Domain::Audio, Domain::Power, Domain::Perf, Domain::Ime]);
        return;
    }

    // Bootstrap light domains first so chips aren't empty; heavy/temp async.
    s.want_light.store(true, Ordering::SeqCst);
    s.want_wifi.store(true, Ordering::SeqCst);
    s.want_bt.store(true, Ordering::SeqCst);
    s.want_temp.store(true, Ordering::SeqCst);

    std::thread::Builder::new()
        .name("wh-sysmon-light".into())
        .spawn(|| light_loop())
        .ok();
    std::thread::Builder::new()
        .name("wh-sysmon-wifi".into())
        .spawn(|| wifi_loop())
        .ok();
    std::thread::Builder::new()
        .name("wh-sysmon-bt".into())
        .spawn(|| bluetooth_loop())
        .ok();
    std::thread::Builder::new()
        .name("wh-sysmon-temp".into())
        .spawn(|| temperature_loop())
        .ok();
}

/// Instant cache read — never calls WLAN / BT / WMI / PowerShell.
pub fn cached_snapshot() -> SystemRadioSnapshot {
    state().caches.lock().to_snapshot()
}

/// Schedule background refresh; returns immediately. Does not wait for collectors.
pub fn request_refresh(domains: &[Domain]) {
    let s = state();
    for d in domains {
        match d {
            Domain::Wifi => s.want_wifi.store(true, Ordering::SeqCst),
            Domain::Bluetooth => s.want_bt.store(true, Ordering::SeqCst),
            Domain::Audio | Domain::Power | Domain::Perf | Domain::Ime => {
                s.want_light.store(true, Ordering::SeqCst);
            }
            Domain::Temperature => s.want_temp.store(true, Ordering::SeqCst),
        }
    }
}

/// Soft "force" from legacy API: kick heavy + light without blocking.
pub fn request_full_refresh() {
    request_refresh(&[
        Domain::Wifi,
        Domain::Bluetooth,
        Domain::Audio,
        Domain::Power,
        Domain::Perf,
        Domain::Ime,
        Domain::Temperature,
    ]);
}

pub fn patch_volume(volume: system_audio::VolumeSnapshot) {
    let mut c = state().caches.lock();
    c.volume = Timed {
        at: Instant::now(),
        value: volume,
    };
}

pub fn invalidate_wifi() {
    request_refresh(&[Domain::Wifi]);
}

pub fn invalidate_bluetooth() {
    request_refresh(&[Domain::Bluetooth]);
}

fn emit_updated(snap: &SystemRadioSnapshot) {
    if let Some(app) = state().app.lock().clone() {
        let _ = app.emit("system-status-updated", snap.clone());
    }
}

fn due(at: Instant, ttl: Duration) -> bool {
    at.elapsed() >= ttl
}

fn light_loop() {
    let s = state();
    loop {
        let (need_light, need_ime) = {
            let c = s.caches.lock();
            let want = s.want_light.swap(false, Ordering::SeqCst);
            let light = want
                || due(c.volume.at, TTL_LIGHT)
                || due(c.power.at, TTL_LIGHT)
                || due(c.perf.at, TTL_LIGHT);
            // want_light also pulls IME so UI soft-refresh stays coherent.
            let ime = want || due(c.ime.at, TTL_IME);
            (light, ime)
        };
        if need_light && !s.busy_light.swap(true, Ordering::SeqCst) {
            refresh_light(need_ime);
            s.busy_light.store(false, Ordering::SeqCst);
        } else if need_ime {
            refresh_ime_only();
        }
        std::thread::sleep(TICK);
    }
}

fn wifi_loop() {
    let s = state();
    loop {
        let need = {
            let c = s.caches.lock();
            s.want_wifi.swap(false, Ordering::SeqCst) || due(c.wifi.at, TTL_HEAVY)
        };
        if need && !s.busy_wifi.swap(true, Ordering::SeqCst) {
            refresh_wifi();
            s.busy_wifi.store(false, Ordering::SeqCst);
        }
        std::thread::sleep(TICK);
    }
}

fn bluetooth_loop() {
    let s = state();
    loop {
        let need = {
            let c = s.caches.lock();
            s.want_bt.swap(false, Ordering::SeqCst) || due(c.bluetooth.at, TTL_HEAVY)
        };
        if need && !s.busy_bt.swap(true, Ordering::SeqCst) {
            refresh_bluetooth();
            s.busy_bt.store(false, Ordering::SeqCst);
        }
        std::thread::sleep(TICK);
    }
}

fn temperature_loop() {
    let s = state();
    loop {
        let need = {
            let c = s.caches.lock();
            s.want_temp.swap(false, Ordering::SeqCst) || due(c.temp_at, TTL_TEMP)
        };
        if need && !s.busy_temp.swap(true, Ordering::SeqCst) {
            refresh_temperature();
            s.busy_temp.store(false, Ordering::SeqCst);
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn refresh_light(also_ime: bool) {
    let volume = system_audio::snapshot();
    let power = system_power::snapshot();
    let mut perf = system_perf::collect_cpu_mem();
    // Preserve last known temperatures — never call PowerShell here.
    {
        let c = state().caches.lock();
        perf.cpu_temp_c = c.perf.value.cpu_temp_c;
        perf.gpu_temp_c = c.perf.value.gpu_temp_c;
        perf.gpu_mem_percent = c.perf.value.gpu_mem_percent;
    }
    let ime = if also_ime {
        Some(system_radio::current_ime())
    } else {
        None
    };
    let now = Instant::now();
    let snap = {
        let mut c = state().caches.lock();
        c.volume = Timed {
            at: now,
            value: volume,
        };
        c.power = Timed {
            at: now,
            value: power,
        };
        c.perf = Timed {
            at: now,
            value: perf,
        };
        if let Some(ime) = ime {
            c.ime = Timed {
                at: now,
                value: ime,
            };
        }
        c.to_snapshot()
    };
    emit_updated(&snap);
}

fn refresh_ime_only() {
    let ime = system_radio::current_ime();
    let now = Instant::now();
    let snap = {
        let mut c = state().caches.lock();
        // Skip emit if chip text unchanged — avoid flooding UI.
        if c.ime.value.mark == ime.mark
            && c.ime.value.mode == ime.mode
            && c.ime.value.caps == ime.caps
            && c.ime.value.name == ime.name
            && c.ime.value.layout == ime.layout
        {
            c.ime.at = now;
            return;
        }
        c.ime = Timed {
            at: now,
            value: ime,
        };
        c.to_snapshot()
    };
    emit_updated(&snap);
}

fn refresh_wifi() {
    // Fire WlanScan without sleeping — next cycle / subsequent refresh picks results.
    let wifi = system_radio::scan_wifi_list(true);
    let now = Instant::now();
    let snap = {
        let mut c = state().caches.lock();
        c.wifi = Timed {
            at: now,
            value: wifi,
        };
        c.to_snapshot()
    };
    emit_updated(&snap);
}

fn refresh_bluetooth() {
    let bluetooth = system_radio::scan_bluetooth();
    let peripherals = system_radio::collect_peripherals(&bluetooth);
    let now = Instant::now();
    let snap = {
        let mut c = state().caches.lock();
        c.bluetooth = Timed {
            at: now,
            value: bluetooth,
        };
        c.peripherals = Timed {
            at: now,
            value: peripherals,
        };
        c.to_snapshot()
    };
    emit_updated(&snap);
}

fn refresh_temperature() {
    let (cpu_temp_c, gpu_temp_c) = system_perf::collect_temperatures();
    // On failure keep previous readings; always advance temp_at to avoid tight retry storms.
    let snap = {
        let mut c = state().caches.lock();
        if cpu_temp_c.is_some() {
            c.perf.value.cpu_temp_c = cpu_temp_c;
        }
        if gpu_temp_c.is_some() {
            c.perf.value.gpu_temp_c = gpu_temp_c;
        }
        c.temp_at = Instant::now();
        c.to_snapshot()
    };
    emit_updated(&snap);
}
