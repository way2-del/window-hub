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
/// Under RAM pressure, slow light refresh to cut IPC + UI churn.
const TTL_LIGHT_PRESSURE: Duration = Duration::from_secs(28);
/// Heavy state: WiFi list / Bluetooth devices.
const TTL_HEAVY: Duration = Duration::from_secs(45);
const TTL_HEAVY_PRESSURE: Duration = Duration::from_secs(75);
/// Temperature (PowerShell / nvidia-smi) — isolated, low frequency.
const TTL_TEMP: Duration = Duration::from_secs(30);
const TTL_TEMP_PRESSURE: Duration = Duration::from_secs(60);
/// IME / Caps — cheap; keep near-instant so 中/英/A·a 切换跟手。
const TTL_IME: Duration = Duration::from_millis(500);
const TTL_IME_PRESSURE: Duration = Duration::from_millis(1200);

const TICK: Duration = Duration::from_millis(400);
const TICK_PRESSURE: Duration = Duration::from_millis(800);

fn under_mem_pressure() -> bool {
    crate::win32::system_memory::physical_mem_percent() >= 88
}

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
                    down_bps: 0,
                    up_bps: 0,
                    session_rx_bytes: 0,
                    session_tx_bytes: 0,
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
    std::thread::Builder::new()
        .name("wh-sysmon-net".into())
        .spawn(|| net_loop())
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
        let pressure = under_mem_pressure();
        let ttl_light = if pressure { TTL_LIGHT_PRESSURE } else { TTL_LIGHT };
        let ttl_ime = if pressure { TTL_IME_PRESSURE } else { TTL_IME };
        let (need_light, need_ime) = {
            let c = s.caches.lock();
            let want = s.want_light.swap(false, Ordering::SeqCst);
            let light = want
                || due(c.volume.at, ttl_light)
                || due(c.power.at, ttl_light)
                || due(c.perf.at, ttl_light);
            let ime = want || due(c.ime.at, ttl_ime);
            (light, ime)
        };
        if need_light && !s.busy_light.swap(true, Ordering::SeqCst) {
            refresh_light(need_ime);
            s.busy_light.store(false, Ordering::SeqCst);
        } else if need_ime {
            refresh_ime_only();
        }
        std::thread::sleep(if pressure { TICK_PRESSURE } else { TICK });
    }
}

fn wifi_loop() {
    let s = state();
    loop {
        let pressure = under_mem_pressure();
        let ttl = if pressure { TTL_HEAVY_PRESSURE } else { TTL_HEAVY };
        let need = {
            let c = s.caches.lock();
            s.want_wifi.swap(false, Ordering::SeqCst) || due(c.wifi.at, ttl)
        };
        if need && !s.busy_wifi.swap(true, Ordering::SeqCst) {
            refresh_wifi();
            s.busy_wifi.store(false, Ordering::SeqCst);
        }
        std::thread::sleep(if pressure { TICK_PRESSURE } else { TICK });
    }
}

fn bluetooth_loop() {
    let s = state();
    loop {
        let pressure = under_mem_pressure();
        let ttl = if pressure { TTL_HEAVY_PRESSURE } else { TTL_HEAVY };
        let need = {
            let c = s.caches.lock();
            s.want_bt.swap(false, Ordering::SeqCst) || due(c.bluetooth.at, ttl)
        };
        if need && !s.busy_bt.swap(true, Ordering::SeqCst) {
            refresh_bluetooth();
            s.busy_bt.store(false, Ordering::SeqCst);
        }
        std::thread::sleep(if pressure { TICK_PRESSURE } else { TICK });
    }
}

fn temperature_loop() {
    let s = state();
    loop {
        let pressure = under_mem_pressure();
        let ttl = if pressure { TTL_TEMP_PRESSURE } else { TTL_TEMP };
        let need = {
            let c = s.caches.lock();
            s.want_temp.swap(false, Ordering::SeqCst) || due(c.temp_at, ttl)
        };
        if need && !s.busy_temp.swap(true, Ordering::SeqCst) {
            refresh_temperature();
            s.busy_temp.store(false, Ordering::SeqCst);
        }
        std::thread::sleep(Duration::from_secs(if pressure { 2 } else { 1 }));
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
        // Skip emit when nothing chip-visible changed — cuts UI thrash under load.
        let same_vol = c.volume.value.level == volume.level && c.volume.value.muted == volume.muted;
        let same_pwr = c.power.value.percent == power.percent
            && c.power.value.charging == power.charging
            && c.power.value.ac_line == power.ac_line;
        // net 字段已由 collect_cpu_mem → NET_LAST 填好；切勿用旧 cache 盖掉，否则会把网速抹成 0
        let same_perf = c.perf.value.mem_percent == perf.mem_percent
            && c.perf.value.cpu_percent.abs_diff(perf.cpu_percent) < 2
            && c.perf.value.down_bps.abs_diff(perf.down_bps) < 2048
            && c.perf.value.up_bps.abs_diff(perf.up_bps) < 2048;
        let same_ime = match &ime {
            Some(i) => {
                c.ime.value.mark == i.mark
                    && c.ime.value.mode == i.mode
                    && c.ime.value.caps == i.caps
            }
            None => true,
        };
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
        if same_vol && same_pwr && same_perf && same_ime {
            return;
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

fn net_loop() {
    // 第一次只建基线；再按 ~1s 推速率
    let _ = system_perf::collect_net();
    loop {
        let pressure = under_mem_pressure();
        std::thread::sleep(if pressure {
            Duration::from_millis(1800)
        } else {
            Duration::from_millis(1000)
        });
        refresh_net();
    }
}

fn refresh_net() {
    let (down_bps, up_bps, session_rx, session_tx) = system_perf::collect_net();
    let snap = {
        let mut c = state().caches.lock();
        let prev_down = c.perf.value.down_bps;
        let prev_up = c.perf.value.up_bps;
        let prev_rx = c.perf.value.session_rx_bytes;
        let prev_tx = c.perf.value.session_tx_bytes;
        let rate_delta =
            prev_down.abs_diff(down_bps) >= 512 || prev_up.abs_diff(up_bps) >= 512;
        let idle_flip = ((down_bps + up_bps) < 1024) != ((prev_down + prev_up) < 1024);
        let busy = down_bps >= 512 || up_bps >= 512 || prev_down >= 512 || prev_up >= 512;
        let sess_tick = session_rx
            .saturating_sub(prev_rx)
            .saturating_add(session_tx.saturating_sub(prev_tx))
            >= 8 * 1024;
        c.perf.value.down_bps = down_bps;
        c.perf.value.up_bps = up_bps;
        c.perf.value.session_rx_bytes = session_rx;
        c.perf.value.session_tx_bytes = session_tx;
        // 空闲跳过；有流量时每秒推一次（面板会话累计 / 芯片速率）
        if !rate_delta && !idle_flip && !busy && !sess_tick {
            return;
        }
        c.to_snapshot()
    };
    emit_updated(&snap);
}
