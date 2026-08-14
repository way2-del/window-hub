//! Isolate plugin `hub.fetch` so a dead local backend cannot stall Host.
//!
//! - Concurrent in-flight cap (global)
//! - Per plugin+origin circuit breaker (fast-fail while open)

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

const MAX_IN_FLIGHT: usize = 2;
const TRIP_AFTER_FAILS: u32 = 3;
const OPEN_FOR: Duration = Duration::from_secs(45);
const HALF_OPEN_PROBE: Duration = Duration::from_secs(15);

static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone)]
struct Breaker {
    fails: u32,
    /// While `Instant::now() < open_until`, reject immediately.
    open_until: Option<Instant>,
}

impl Default for Breaker {
    fn default() -> Self {
        Self {
            fails: 0,
            open_until: None,
        }
    }
}

static BREAKERS: LazyLock<Mutex<HashMap<String, Breaker>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn breaker_key(plugin_id: &str, url: &str) -> String {
    let origin = url_origin(url).unwrap_or_else(|| url.to_string());
    format!("{plugin_id}|{origin}")
}

fn url_origin(url: &str) -> Option<String> {
    let u = url.trim();
    // http://host:port/... → http://host:port
    let rest = u
        .strip_prefix("http://")
        .or_else(|| u.strip_prefix("https://"))?;
    let hostport = rest.split('/').next().unwrap_or(rest);
    if hostport.is_empty() {
        return None;
    }
    let scheme = if u.starts_with("https://") {
        "https"
    } else {
        "http"
    };
    Some(format!("{scheme}://{hostport}"))
}

/// Acquire a fetch slot. Err if Host is already saturated.
pub fn try_acquire_slot() -> Result<FetchSlot, String> {
    loop {
        let cur = IN_FLIGHT.load(Ordering::SeqCst);
        if cur >= MAX_IN_FLIGHT {
            return Err(
                "hub.fetch busy: too many in-flight plugin network calls (retry later)".into(),
            );
        }
        if IN_FLIGHT
            .compare_exchange(cur, cur + 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return Ok(FetchSlot);
        }
    }
}

pub struct FetchSlot;

impl Drop for FetchSlot {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Fast-fail if this plugin+origin circuit is open.
pub fn check_circuit(plugin_id: &str, url: &str) -> Result<(), String> {
    let key = breaker_key(plugin_id, url);
    let mut map = BREAKERS.lock();
    let entry = map.entry(key.clone()).or_default();
    if let Some(until) = entry.open_until {
        if Instant::now() < until {
            return Err(format!(
                "hub.fetch circuit open for {key} (backend unreachable; backing off)"
            ));
        }
        // Half-open: allow one probe; keep open_until short so failures re-trip quickly.
        entry.open_until = Some(Instant::now() + HALF_OPEN_PROBE);
    }
    Ok(())
}

pub fn record_success(plugin_id: &str, url: &str) {
    let key = breaker_key(plugin_id, url);
    BREAKERS.lock().remove(&key);
}

pub fn record_failure(plugin_id: &str, url: &str) {
    let key = breaker_key(plugin_id, url);
    let mut map = BREAKERS.lock();
    let entry = map.entry(key).or_default();
    entry.fails = entry.fails.saturating_add(1);
    if entry.fails >= TRIP_AFTER_FAILS {
        entry.open_until = Some(Instant::now() + OPEN_FOR);
    }
}

/// True if the error looks like transport / connect failure (not HTTP 4xx/5xx body).
pub fn is_transport_error(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("connection")
        || e.contains("timed out")
        || e.contains("timeout")
        || e.contains("refused")
        || e.contains("reset")
        || e.contains("unreachable")
        || e.contains("dns")
        || e.contains("name resolution")
        || e.contains("fetch failed")
        || e.contains("i/o error")
        || e.contains("os error")
}
