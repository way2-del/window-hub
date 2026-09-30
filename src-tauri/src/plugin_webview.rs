//! Host-managed external WebView2 sessions for plugins with capability `webview`.
//!
//! Interactive browse windows share a per-plugin WebView2 profile so cookies
//! persist across open/close. A hidden per-plugin watch window polls selectors
//! on a timer and emits `webview-watch-changed` for the plugin to notify.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::commands::async_delay_ms;

const MIN_INTERVAL_MS: u64 = 30_000;
const MAX_INTERVAL_MS: u64 = 3_600_000;
const MAX_WATCHES_PER_PLUGIN: usize = 20;
const PICK_TIMEOUT_MS: u64 = 120_000;
const LOAD_WAIT_MS: u64 = 12_000;
/// SPA / XHR content often appears well after document.readyState.
const SELECTOR_WAIT_MS: u64 = 18_000;
const EVAL_TIMEOUT_MS: u64 = 8_000;

static RUNNER_STARTED: AtomicBool = AtomicBool::new(false);
/// While >0, Host must not destroy plugin-popup (user is picking in plugin WebView).
static PICK_HOLD: AtomicUsize = AtomicUsize::new(0);

pub fn pick_hold_active() -> bool {
    PICK_HOLD.load(Ordering::SeqCst) > 0
}

fn begin_pick_hold() {
    PICK_HOLD.fetch_add(1, Ordering::SeqCst);
}

fn end_pick_hold() {
    let _ = PICK_HOLD.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
        Some(n.saturating_sub(1))
    });
}

struct PickHoldGuard;
impl Drop for PickHoldGuard {
    fn drop(&mut self) {
        end_pick_hold();
    }
}

fn refocus_plugin_popup(app: &AppHandle, plugin_id: &str) {
    if let Some(w) = app.get_webview_window("plugin-popup") {
        let _ = w.show();
        let _ = w.unminimize();
        crate::commands::mark_popup_visible("plugin-popup", true);
        let _ = w.set_focus();
        let _ = app.emit("plugin-popup-opened", plugin_id.to_string());
    }
}

fn state() -> &'static Mutex<WebviewState> {
    static CELL: OnceLock<Mutex<WebviewState>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(WebviewState::default()))
}

#[derive(Default)]
struct WebviewState {
    /// session_id → meta
    sessions: HashMap<String, SessionMeta>,
    /// (plugin_id, watch_id) → watch
    watches: HashMap<(String, String), WatchMeta>,
    /// last snapshot text hash per watch key
    last_text: HashMap<(String, String), String>,
    /// last successful pick per plugin (survives popup tear-down)
    last_pick: HashMap<String, LastPick>,
}

#[derive(Clone)]
struct LastPick {
    session_id: String,
    result: PickResult,
}

#[derive(Clone)]
struct SessionMeta {
    plugin_id: String,
    label: String,
}

#[derive(Clone)]
struct WatchMeta {
    plugin_id: String,
    id: String,
    url: String,
    selector: String,
    interval_ms: u64,
    title: String,
    next_due: Instant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenOpts {
    pub url: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionIdOpts {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigateOpts {
    pub session_id: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotOpts {
    pub session_id: Option<String>,
    pub url: Option<String>,
    pub selector: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchStartOpts {
    pub id: String,
    pub url: String,
    pub selector: String,
    pub interval_ms: Option<u64>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchIdOpts {
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickResult {
    pub selector: String,
    pub text_preview: String,
    pub outer_html: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotResult {
    pub text: String,
    pub html: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchInfo {
    pub id: String,
    pub url: String,
    pub selector: String,
    pub interval_ms: u64,
    pub title: String,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn sanitize_part(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            out.push('-');
        }
    }
    if out.is_empty() {
        out.push('x');
    }
    out.truncate(48);
    out
}

fn profile_dir(plugin_id: &str) -> Result<PathBuf, String> {
    let appdata = std::env::var_os("APPDATA").ok_or_else(|| "APPDATA missing".to_string())?;
    let dir = PathBuf::from(appdata)
        .join("window-hub")
        .join("webview-profiles")
        .join(sanitize_part(plugin_id));
    std::fs::create_dir_all(&dir).map_err(|e| format!("create profile dir: {e}"))?;
    Ok(dir)
}

fn parse_http_url(raw: &str) -> Result<url::Url, String> {
    let u = url::Url::parse(raw.trim()).map_err(|e| format!("invalid url: {e}"))?;
    match u.scheme() {
        "http" | "https" => Ok(u),
        other => Err(format!("only http/https allowed, got {other}")),
    }
}

fn session_label(plugin_id: &str, session_id: &str) -> String {
    format!(
        "plugin-wv-{}-{}",
        sanitize_part(plugin_id),
        sanitize_part(session_id)
    )
}

fn watch_label(plugin_id: &str) -> String {
    format!("plugin-wv-watch-{}", sanitize_part(plugin_id))
}

fn eval_string(win: &WebviewWindow, js: &str) -> Result<String, String> {
    let (tx, rx) = mpsc::channel::<String>();
    win.eval_with_callback(js.to_string(), move |s| {
        let _ = tx.send(s);
    })
    .map_err(|e| format!("eval failed: {e}"))?;
    rx.recv_timeout(Duration::from_millis(EVAL_TIMEOUT_MS))
        .map_err(|_| "eval timed out".to_string())
}

/// Fire eval on the UI thread, wait for the callback on the caller thread.
/// Never block inside `run_on_main_thread` — WebView2 needs the UI pump to deliver the result.
fn eval_string_on_main(app: &AppHandle, win: &WebviewWindow, js: &str) -> Result<String, String> {
    let (tx, rx) = mpsc::channel::<String>();
    let win = win.clone();
    let js = js.to_string();
    app.run_on_main_thread(move || {
        let _ = win.eval_with_callback(js, move |s| {
            let _ = tx.send(s);
        });
    })
    .map_err(|e| format!("run_on_main_thread: {e}"))?;
    rx.recv_timeout(Duration::from_millis(EVAL_TIMEOUT_MS))
        .map_err(|_| "eval timed out".to_string())
}

fn json_unescape(raw: &str) -> String {
    let t = raw.trim();
    if t == "null" || t.is_empty() {
        return String::new();
    }
    serde_json::from_str::<String>(t).unwrap_or_else(|_| t.trim_matches('"').to_string())
}

fn wait_document_ready(win: &WebviewWindow, timeout: Duration) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        let state = eval_string(win, "document.readyState").unwrap_or_default();
        let s = json_unescape(&state);
        if s == "complete" || s == "interactive" {
            // Give SPA a short settle window.
            std::thread::sleep(Duration::from_millis(400));
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err("page load timed out".into())
}

fn snapshot_selector(win: &WebviewWindow, selector: &str) -> Result<SnapshotResult, String> {
    let sel = serde_json::to_string(selector).map_err(|e| e.to_string())?;
    let js = format!(
        r#"(function(){{
  try {{
    var el = document.querySelector({sel});
    if (!el) return JSON.stringify({{ text: "", html: null, missing: true }});
    var text = (el.innerText || el.textContent || "").replace(/\s+/g, " ").trim();
    var html = (el.outerHTML || "").slice(0, 4000);
    return JSON.stringify({{ text: text, html: html, missing: false }});
  }} catch (e) {{
    return JSON.stringify({{ text: "", html: null, error: String(e) }});
  }}
}})()"#
    );
    let raw = eval_string(win, &js)?;
    let raw = json_unescape(&raw);
    let v: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("snapshot parse: {e} / {raw}"))?;
    if v.get("missing").and_then(|x| x.as_bool()) == Some(true) {
        return Err("selector not found".into());
    }
    if let Some(err) = v.get("error").and_then(|x| x.as_str()) {
        return Err(err.to_string());
    }
    Ok(SnapshotResult {
        text: v
            .get("text")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        html: v
            .get("html")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),
    })
}

/// Try full selector, then progressively shorter `> `-suffixes (layout wrappers often change).
fn selector_candidates(selector: &str) -> Vec<String> {
    let s = selector.trim();
    if s.is_empty() {
        return Vec::new();
    }
    let mut out = vec![s.to_string()];
    let parts: Vec<&str> = s.split(" > ").filter(|p| !p.trim().is_empty()).collect();
    if parts.len() > 1 {
        for i in 1..parts.len() {
            let suffix = parts[i..].join(" > ");
            if !suffix.is_empty() && !out.iter().any(|x| x == &suffix) {
                out.push(suffix);
            }
        }
    }
    // Also try last segment alone when it looks specific (class/id).
    if let Some(last) = parts.last() {
        let last = last.trim();
        if (last.contains('.') || last.contains('#')) && !out.iter().any(|x| x == last) {
            out.push(last.to_string());
        }
    }
    out
}

fn snapshot_selector_resilient(win: &WebviewWindow, selector: &str) -> Result<SnapshotResult, String> {
    let mut last = "selector not found".to_string();
    for cand in selector_candidates(selector) {
        match snapshot_selector(win, &cand) {
            Ok(s) => return Ok(s),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// After navigation, poll until the selector exists or timeout (SPA-friendly).
fn wait_for_selector(
    win: &WebviewWindow,
    selector: &str,
    timeout: Duration,
) -> Result<SnapshotResult, String> {
    let start = Instant::now();
    let mut last = "selector not found".to_string();
    while start.elapsed() < timeout {
        match snapshot_selector_resilient(win, selector) {
            Ok(s) => return Ok(s),
            Err(e) => last = e,
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(last)
}

const PICK_TITLE_MARKER: &str = "\u{200b}WHPICK:";
const PICK_HASH_MARKER: &str = "__WHPICK__";

/// Fullscreen shield + elementFromPoint — avoids fighting page capture listeners.
/// Signals via URL hash (Host polls win.url()), sessionStorage, title, and JS global.
const PICKER_JS: &str = r##"(function(){
  try {
    if (window.__WH_PICK_CLEANUP__) { try { window.__WH_PICK_CLEANUP__(); } catch(_){} }
  } catch(_){}
  window.__WH_PICK_ACTIVE__ = true;
  window.__WH_PICK_RESULT_JSON__ = null;
  try { sessionStorage.removeItem("__WH_PICK__"); } catch(_){}
  if (window.__WH_PICK_OLD_TITLE__ == null) {
    try { window.__WH_PICK_OLD_TITLE__ = document.title; } catch(_) { window.__WH_PICK_OLD_TITLE__ = ""; }
  }
  if (window.__WH_PICK_OLD_HASH__ == null) {
    try { window.__WH_PICK_OLD_HASH__ = location.hash || ""; } catch(_) { window.__WH_PICK_OLD_HASH__ = ""; }
  }
  function cssEscape(s){
    try { return CSS.escape(s); } catch(_) {
      return String(s).replace(/[^a-zA-Z0-9_-]/g, "\\$&");
    }
  }
  function cssPath(el){
    if (!el || el.nodeType !== 1) return "";
    if (el === document.documentElement) return "html";
    if (el === document.body) return "body";
    function unique(sel){
      try {
        var all = document.querySelectorAll(sel);
        return all.length === 1 && all[0] === el;
      } catch(_) { return false; }
    }
    if (el.id) {
      var idSel = "#" + cssEscape(el.id);
      if (unique(idSel)) return idSel;
    }
    // Prefer data-* attributes when unique.
    try {
      if (el.attributes) {
        for (var ai = 0; ai < el.attributes.length; ai++) {
          var at = el.attributes[ai];
          if (!at || !at.name || at.name.indexOf("data-") !== 0) continue;
          if (!at.value || at.value.length > 64) continue;
          var v = String(at.value).replace(/\\/g, "\\\\").replace(/"/g, '\\"');
          var dataSel = el.tagName.toLowerCase() + "[" + at.name + "=\"" + v + "\"]";
          if (unique(dataSel)) return dataSel;
        }
      }
    } catch(_){}
    var parts = [];
    var cur = el;
    while (cur && cur.nodeType === 1 && parts.length < 8) {
      if (cur === document.body) { parts.unshift("body"); break; }
      if (cur === document.documentElement) { parts.unshift("html"); break; }
      var name = cur.tagName.toLowerCase();
      if (cur.id) {
        parts.unshift("#" + cssEscape(cur.id));
        break;
      }
      var classes = [];
      if (cur.classList && cur.classList.length) {
        for (var i = 0; i < cur.classList.length && classes.length < 3; i++) {
          var c = cur.classList[i];
          if (!c || c.length > 48) continue;
          if (/^(js-|is-|has-|active|hover|open|show|hide|css-|ng-|v-)/i.test(c)) continue;
          classes.push("." + cssEscape(c));
        }
      }
      var part = name + classes.join("");
      var parent = cur.parentElement;
      if (parent) {
        var matched = 0;
        var idxAmong = 0;
        for (var j = 0; j < parent.children.length; j++) {
          var ch = parent.children[j];
          if (ch.tagName !== cur.tagName) continue;
          var ok = true;
          if (classes.length) {
            for (var k = 0; k < classes.length; k++) {
              var cn = classes[k].slice(1);
              if (!ch.classList || !ch.classList.contains(cn)) { ok = false; break; }
            }
          }
          if (!ok) continue;
          matched++;
          if (ch === cur) idxAmong = matched;
        }
        if (matched > 1) {
          // nth-of-type among same tag (CSS standard), not among class matches
          var sameTag = 0;
          var nth = 0;
          for (var t = 0; t < parent.children.length; t++) {
            if (parent.children[t].tagName === cur.tagName) {
              sameTag++;
              if (parent.children[t] === cur) nth = sameTag;
            }
          }
          if (nth > 0) part += ":nth-of-type(" + nth + ")";
        }
      }
      parts.unshift(part);
      var test = parts.join(" > ");
      if (unique(test)) break;
      cur = parent;
    }
    var path = parts.join(" > ");
    // Prefer shortest unique suffix.
    var segs = path.split(" > ");
    for (var si = 0; si < segs.length; si++) {
      var suffix = segs.slice(si).join(" > ");
      if (unique(suffix)) return suffix;
    }
    return path;
  }
  function isHud(el){
    if (!el || el.nodeType !== 1) return true;
    var id = el.id || "";
    return id === "__wh_pick_tip" || id === "__wh_pick_hl" || id === "__wh_pick_shield";
  }
  function underPoint(x, y){
    var shield = document.getElementById("__wh_pick_shield");
    var hl = document.getElementById("__wh_pick_hl");
    var tip = document.getElementById("__wh_pick_tip");
    var prevS = shield ? shield.style.pointerEvents : "";
    var prevH = hl ? hl.style.pointerEvents : "";
    var prevT = tip ? tip.style.pointerEvents : "";
    if (shield) shield.style.pointerEvents = "none";
    if (hl) hl.style.pointerEvents = "none";
    if (tip) tip.style.pointerEvents = "none";
    var el = document.elementFromPoint(x, y);
    if (shield) shield.style.pointerEvents = prevS || "auto";
    if (hl) hl.style.pointerEvents = prevH || "none";
    if (tip) tip.style.pointerEvents = prevT || "none";
    while (el && el.shadowRoot) {
      try {
        var inner = el.shadowRoot.elementFromPoint(x, y);
        if (!inner || inner === el) break;
        el = inner;
      } catch(_) { break; }
    }
    if (isHud(el)) return null;
    return el && el.nodeType === 1 ? el : null;
  }
  var tip = document.createElement("div");
  tip.id = "__wh_pick_tip";
  tip.textContent = "点击要监测的元素 · Esc 取消";
  tip.style.cssText = "position:fixed;z-index:2147483647;left:50%;top:12px;transform:translateX(-50%);background:#111;color:#fff;padding:8px 14px;border-radius:10px;font:600 13px/1.3 system-ui,sans-serif;pointer-events:none;box-shadow:0 8px 24px rgba(0,0,0,.35)";
  var overlay = document.createElement("div");
  overlay.id = "__wh_pick_hl";
  overlay.style.cssText = "position:fixed;z-index:2147483646;pointer-events:none;border:2px solid #0a84ff;background:rgba(10,132,255,.12);display:none";
  var shield = document.createElement("div");
  shield.id = "__wh_pick_shield";
  shield.style.cssText = "position:fixed;inset:0;z-index:2147483645;cursor:crosshair;background:rgba(0,0,0,.01)";
  document.documentElement.appendChild(shield);
  document.documentElement.appendChild(overlay);
  document.documentElement.appendChild(tip);
  function cleanup(){
    window.__WH_PICK_ACTIVE__ = false;
    window.__WH_PICK_CLEANUP__ = null;
    try { shield.removeEventListener("pointermove", onMove); } catch(_){}
    try { shield.removeEventListener("pointerdown", onDown); } catch(_){}
    try { shield.removeEventListener("click", onClick); } catch(_){}
    try { window.removeEventListener("keydown", onKey, true); } catch(_){}
    try { tip.remove(); } catch(_){}
    try { overlay.remove(); } catch(_){}
    try { shield.remove(); } catch(_){}
  }
  window.__WH_PICK_CLEANUP__ = cleanup;
  function signal(obj){
    var full = {
      selector: obj.selector || "",
      textPreview: (obj.textPreview || "").slice(0, 120),
      cancelled: !!obj.cancelled
    };
    if (obj.outerHtml) full.outerHtml = String(obj.outerHtml).slice(0, 2000);
    var json = JSON.stringify(full);
    window.__WH_PICK_RESULT_JSON__ = json;
    try { sessionStorage.setItem("__WH_PICK__", json); } catch(_){}
    // Hash/title stay slim (URL/title length limits).
    try {
      var slim = JSON.stringify({
        selector: String(full.selector || "").slice(0, 420),
        textPreview: full.textPreview || "",
        cancelled: !!full.cancelled
      });
      var b64 = btoa(unescape(encodeURIComponent(slim)));
      var base = String(location.href).replace(/#.*$/, "");
      history.replaceState(null, "", base + "#__WHPICK__" + b64);
      document.title = "\u200bWHPICK:" + b64;
    } catch(_){}
    try {
      if (window.chrome && window.chrome.webview && window.chrome.webview.postMessage) {
        window.chrome.webview.postMessage(full);
      }
    } catch(_){}
  }
  function highlight(el){
    if (!el || !el.getBoundingClientRect) {
      overlay.style.display = "none";
      return;
    }
    var r = el.getBoundingClientRect();
    overlay.style.display = "block";
    overlay.style.left = r.left + "px";
    overlay.style.top = r.top + "px";
    overlay.style.width = Math.max(1, r.width) + "px";
    overlay.style.height = Math.max(1, r.height) + "px";
  }
  function finish(el){
    if (!el) return;
    var selector = cssPath(el);
    if (!selector) selector = el.tagName ? el.tagName.toLowerCase() : "body";
    var text = ((el.innerText || el.textContent) || "").replace(/\s+/g," ").trim().slice(0, 240);
    var html = (el.outerHTML || "").slice(0, 2000);
    signal({ selector: selector, textPreview: text, outerHtml: html });
    cleanup();
  }
  function onMove(e){
    highlight(underPoint(e.clientX, e.clientY));
  }
  function onDown(e){
    try { e.preventDefault(); e.stopPropagation(); e.stopImmediatePropagation(); } catch(_){}
    finish(underPoint(e.clientX, e.clientY));
  }
  function onClick(e){
    try { e.preventDefault(); e.stopPropagation(); e.stopImmediatePropagation(); } catch(_){}
    if (window.__WH_PICK_RESULT_JSON__) return;
    finish(underPoint(e.clientX, e.clientY));
  }
  function onKey(e){
    if (e.key === "Escape") {
      try { e.preventDefault(); } catch(_){}
      signal({ cancelled: true });
      cleanup();
    }
  }
  shield.addEventListener("pointermove", onMove, true);
  shield.addEventListener("pointerdown", onDown, true);
  shield.addEventListener("click", onClick, true);
  window.addEventListener("keydown", onKey, true);
  return "ok";
})()"##;

const POLL_PICK_JS: &str = r#"(function(){
  try {
    var r = window.__WH_PICK_RESULT_JSON__;
    if (r == null || r === "") {
      try { r = sessionStorage.getItem("__WH_PICK__"); } catch(_){}
    }
    if (r == null || r === "") return null;
    if (typeof r === "string") return r;
    return JSON.stringify(r);
  } catch (e) {
    return null;
  }
})()"#;

const RESTORE_AFTER_PICK_JS: &str = r#"(function(){
  try {
    if (window.__WH_PICK_OLD_TITLE__ != null) {
      document.title = window.__WH_PICK_OLD_TITLE__;
      window.__WH_PICK_OLD_TITLE__ = null;
    }
  } catch(_){}
  try {
    var oldHash = window.__WH_PICK_OLD_HASH__;
    if (oldHash != null) {
      var base = String(location.href).replace(/#.*$/, "");
      history.replaceState(null, "", base + (oldHash || ""));
      window.__WH_PICK_OLD_HASH__ = null;
    }
  } catch(_){}
  try { sessionStorage.removeItem("__WH_PICK__"); } catch(_){}
  window.__WH_PICK_RESULT_JSON__ = null;
  return true;
})()"#;

const VERIFY_PICK_ACTIVE_JS: &str = r#"(function(){
  try { return window.__WH_PICK_ACTIVE__ === true ? "1" : "0"; } catch(e) { return "0"; }
})()"#;

/// True when the OS foreground HWND belongs to a Host-managed `plugin-wv-*` window.
pub fn foreground_is_plugin_webview(app: &AppHandle) -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        let fg = unsafe { GetForegroundWindow().0 as isize };
        if fg == 0 {
            return false;
        }
        for (label, win) in app.webview_windows() {
            if !label.starts_with("plugin-wv-") {
                continue;
            }
            if let Ok(hwnd) = win.hwnd() {
                if hwnd.0 as isize == fg {
                    return true;
                }
            }
        }
        false
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        false
    }
}

fn parse_pick_payload(raw: &str) -> Result<Result<PickResult, String>, String> {
    let raw = raw.trim();
    if raw.is_empty() || raw == "null" {
        return Err("empty".into());
    }
    // eval_with_callback JSON-encodes the value. Object → `{...}`; string → `"..."`.
    let decoded = json_unescape(raw);
    let v: serde_json::Value = serde_json::from_str(&decoded)
        .or_else(|_| serde_json::from_str(raw))
        .or_else(|_| {
            let inner: String = serde_json::from_str(raw).unwrap_or_default();
            if inner.is_empty() {
                Err(serde::de::Error::custom("empty inner"))
            } else {
                serde_json::from_str(&inner)
            }
        })
        .map_err(|e| format!("pick parse: {e} / {raw}"))?;
    if v.get("cancelled").and_then(|x| x.as_bool()) == Some(true) {
        return Ok(Err("pick cancelled".into()));
    }
    let selector = v
        .get("selector")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if selector.is_empty() {
        return Ok(Err("empty selector".into()));
    }
    Ok(Ok(PickResult {
        selector,
        text_preview: v
            .get("textPreview")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        outer_html: v
            .get("outerHtml")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),
    }))
}

fn decode_pick_b64(b64: &str) -> Option<Result<PickResult, String>> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .ok()?;
    let decoded = String::from_utf8(bytes).ok()?;
    match parse_pick_payload(&decoded) {
        Ok(inner) => Some(inner),
        Err(e) => {
            eprintln!("[plugin-webview] pick b64 parse failed: {e} / {decoded}");
            None
        }
    }
}

fn read_pick_from_title(win: &WebviewWindow) -> Option<Result<PickResult, String>> {
    let title = win.title().ok()?;
    // Window title may drop the leading ZWSP; accept both forms.
    let rest = title
        .strip_prefix(PICK_TITLE_MARKER)
        .or_else(|| title.strip_prefix("WHPICK:"))
        .or_else(|| {
            title
                .find("WHPICK:")
                .map(|i| title[i + "WHPICK:".len()..].trim())
        })?;
    decode_pick_b64(rest)
}

fn read_pick_from_url(win: &WebviewWindow) -> Option<Result<PickResult, String>> {
    let url = win.url().ok()?;
    let frag = url.fragment()?;
    let rest = frag.strip_prefix(PICK_HASH_MARKER)?;
    decode_pick_b64(rest)
}

fn restore_after_pick(win: &WebviewWindow) {
    let _ = win.eval(RESTORE_AFTER_PICK_JS);
}

async fn await_pick_result(
    app: &AppHandle,
    win: &WebviewWindow,
    timeout: Duration,
) -> Result<PickResult, String> {
    let deadline = Instant::now() + timeout;
    let mut loops: u64 = 0;
    while Instant::now() < deadline {
        loops += 1;
        // 1) URL hash — no eval, survives click→navigate races better than JS globals
        if let Some(parsed) = read_pick_from_url(win) {
            restore_after_pick(win);
            return parsed;
        }
        // 2) Title channel
        if let Some(parsed) = read_pick_from_title(win) {
            restore_after_pick(win);
            return parsed;
        }
        // 3) Occasional eval (every ~500ms) — avoid flooding WebView2 / starving UI
        if loops % 5 == 0 {
            let raw = eval_string(win, POLL_PICK_JS).unwrap_or_else(|_| "null".into());
            match parse_pick_payload(&raw) {
                Err(_) => {}
                Ok(Ok(pick)) => {
                    restore_after_pick(win);
                    return Ok(pick);
                }
                Ok(Err(e)) => {
                    restore_after_pick(win);
                    return Err(e);
                }
            }
            if loops % 20 == 0 {
                let raw2 =
                    eval_string_on_main(app, win, POLL_PICK_JS).unwrap_or_else(|_| "null".into());
                match parse_pick_payload(&raw2) {
                    Err(_) => {}
                    Ok(Ok(pick)) => {
                        restore_after_pick(win);
                        return Ok(pick);
                    }
                    Ok(Err(e)) => {
                        restore_after_pick(win);
                        return Err(e);
                    }
                }
            }
        }
        async_delay_ms(100).await;
    }
    Err("pick timed out — 未读到点选结果，可改用「整页」或手填选择器".into())
}

fn publish_pick(
    app: &AppHandle,
    plugin_id: &str,
    session_id: &str,
    result: &Result<PickResult, String>,
) {
    match result {
        Ok(pick) => {
            state().lock().last_pick.insert(
                plugin_id.to_string(),
                LastPick {
                    session_id: session_id.to_string(),
                    result: pick.clone(),
                },
            );
            let _ = app.emit(
                "webview-pick-result",
                serde_json::json!({
                    "pluginId": plugin_id,
                    "sessionId": session_id,
                    "cancelled": false,
                    "selector": pick.selector,
                    "textPreview": pick.text_preview,
                    "outerHtml": pick.outer_html,
                }),
            );
        }
        Err(e) if e == "pick cancelled" => {
            let _ = app.emit(
                "webview-pick-result",
                serde_json::json!({
                    "pluginId": plugin_id,
                    "sessionId": session_id,
                    "cancelled": true,
                }),
            );
        }
        Err(_) => {}
    }
}

fn build_external_window(
    app: &AppHandle,
    label: &str,
    url: url::Url,
    title: &str,
    plugin_id: &str,
    visible: bool,
) -> Result<WebviewWindow, String> {
    if let Some(existing) = app.get_webview_window(label) {
        let _ = existing.navigate(url);
        if visible {
            let _ = existing.show();
            let _ = existing.set_focus();
        } else {
            let _ = existing.hide();
        }
        return Ok(existing);
    }

    let profile = profile_dir(plugin_id)?;
    let _guard = crate::commands::lock_webview_create("plugin-webview");

    let builder = WebviewWindowBuilder::new(app, label, WebviewUrl::External(url))
        .title(title)
        .inner_size(1100.0, 720.0)
        .min_inner_size(480.0, 320.0)
        .resizable(true)
        .maximizable(true)
        .minimizable(true)
        .closable(true)
        .decorations(true)
        .always_on_top(false)
        .skip_taskbar(!visible)
        .focused(visible)
        .visible(visible)
        .data_directory(profile);

    let win = builder
        .build()
        .map_err(|e| format!("open webview failed: {e}"))?;

    if !visible {
        let _ = win.hide();
        let _ = win.set_skip_taskbar(true);
    } else {
        let _ = win.show();
        let _ = win.set_focus();
    }
    Ok(win)
}

fn ensure_watch_window_sync(app: &AppHandle, plugin_id: &str) -> Result<WebviewWindow, String> {
    let label = watch_label(plugin_id);
    if let Some(w) = app.get_webview_window(&label) {
        return Ok(w);
    }
    let warm: url::Url = "https://example.com/"
        .parse()
        .map_err(|e| format!("{e}"))?;
    build_external_window(
        app,
        &label,
        warm,
        &format!("Watch · {plugin_id}"),
        plugin_id,
        false,
    )
}

fn start_runner(app: AppHandle) {
    if RUNNER_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("plugin-webview-watch".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_secs(1));
            let due: Vec<WatchMeta> = {
                let st = state().lock();
                let now = Instant::now();
                st.watches
                    .values()
                    .filter(|w| w.next_due <= now)
                    .cloned()
                    .collect()
            };
            if due.is_empty() {
                continue;
            }
            // Process one at a time to share the hidden webview.
            for w in due {
                if let Err(e) = tick_watch(&app, &w) {
                    eprintln!(
                        "[plugin-webview] watch {} / {} failed: {e}",
                        w.plugin_id, w.id
                    );
                }
                let mut st = state().lock();
                if let Some(entry) = st.watches.get_mut(&(w.plugin_id.clone(), w.id.clone())) {
                    entry.next_due =
                        Instant::now() + Duration::from_millis(entry.interval_ms.max(MIN_INTERVAL_MS));
                }
            }
        })
        .ok();
}

fn tick_watch(app: &AppHandle, w: &WatchMeta) -> Result<(), String> {
    let win = ensure_watch_window_sync(app, &w.plugin_id)?;
    let url = parse_http_url(&w.url)?;
    win.navigate(url).map_err(|e| format!("navigate: {e}"))?;
    // readyState alone is not enough for SPA waybill panels — poll for selector.
    let _ = wait_document_ready(&win, Duration::from_millis(LOAD_WAIT_MS));
    let snap = match wait_for_selector(&win, &w.selector, Duration::from_millis(SELECTOR_WAIT_MS)) {
        Ok(s) => s,
        Err(e) => {
            let _ = app.emit(
                "webview-watch-scanned",
                serde_json::json!({
                    "pluginId": w.plugin_id,
                    "watchId": w.id,
                    "title": w.title,
                    "url": w.url,
                    "selector": w.selector,
                    "atMs": now_ms(),
                    "ok": false,
                    "error": e,
                    "text": "",
                    "prevText": "",
                    "changed": false,
                    "baseline": false,
                }),
            );
            return Ok(());
        }
    };
    let key = (w.plugin_id.clone(), w.id.clone());
    let mut st = state().lock();
    let prev = st.last_text.get(&key).cloned();
    let is_first = prev.is_none();
    let unchanged = prev.as_ref() == Some(&snap.text);
    if !unchanged {
        st.last_text.insert(key, snap.text.clone());
    }
    drop(st);

    let changed = !is_first && !unchanged;
    let prev_text = prev.clone().unwrap_or_default();
    let _ = app.emit(
        "webview-watch-scanned",
        serde_json::json!({
            "pluginId": w.plugin_id,
            "watchId": w.id,
            "title": w.title,
            "url": w.url,
            "selector": w.selector,
            "atMs": now_ms(),
            "ok": true,
            "error": null,
            "text": snap.text,
            "prevText": prev_text,
            "changed": changed,
            "baseline": is_first,
        }),
    );

    // First successful sample only baselines — do not notify.
    if is_first || unchanged {
        return Ok(());
    }
    let _ = app.emit(
        "webview-watch-changed",
        serde_json::json!({
            "pluginId": w.plugin_id,
            "watchId": w.id,
            "title": w.title,
            "url": w.url,
            "selector": w.selector,
            "text": snap.text,
            "prevText": prev_text,
            "atMs": now_ms(),
        }),
    );
    Ok(())
}

/// Tear down sessions + watches for a plugin (disable / uninstall).
pub fn cleanup_plugin(app: &AppHandle, plugin_id: &str) {
    let (session_labels, watch_lbl) = {
        let mut st = state().lock();
        let sessions: Vec<(String, String)> = st
            .sessions
            .iter()
            .filter(|(_, m)| m.plugin_id == plugin_id)
            .map(|(id, m)| (id.clone(), m.label.clone()))
            .collect();
        for (id, _) in &sessions {
            st.sessions.remove(id);
        }
        st.watches.retain(|(pid, _), _| pid != plugin_id);
        st.last_text.retain(|(pid, _), _| pid != plugin_id);
        (sessions, watch_label(plugin_id))
    };
    for (_, label) in session_labels {
        if let Some(w) = app.get_webview_window(&label) {
            let _ = w.close();
        }
    }
    if let Some(w) = app.get_webview_window(&watch_lbl) {
        let _ = w.close();
    }
}

pub async fn open(app: AppHandle, plugin_id: String, opts: OpenOpts) -> Result<serde_json::Value, String> {
    crate::plugin_hub::assert_capability(&plugin_id, "webview")?;
    let url = parse_http_url(&opts.url)?;
    let session_id = format!("s{}", now_ms());
    let label = session_label(&plugin_id, &session_id);
    let title = opts
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("网页")
        .to_string();

    let win = build_external_window(&app, &label, url, &title, &plugin_id, true)?;
    let label_owned = label.clone();
    let plugin_for_close = plugin_id.clone();
    let session_for_close = session_id.clone();
    let app_close = app.clone();
    win.on_window_event(move |ev| {
        if let tauri::WindowEvent::Destroyed = ev {
            state().lock().sessions.remove(&session_for_close);
            let _ = app_close.emit(
                "webview-session-closed",
                serde_json::json!({
                    "pluginId": plugin_for_close,
                    "sessionId": session_for_close,
                    "label": label_owned,
                }),
            );
        }
    });

    state().lock().sessions.insert(
        session_id.clone(),
        SessionMeta {
            plugin_id,
            label,
        },
    );

    Ok(serde_json::json!({ "sessionId": session_id }))
}

pub fn close(app: AppHandle, plugin_id: String, opts: SessionIdOpts) -> Result<(), String> {
    crate::plugin_hub::assert_capability(&plugin_id, "webview")?;
    let meta = {
        let st = state().lock();
        st.sessions
            .get(&opts.session_id)
            .cloned()
            .ok_or_else(|| "session not found".to_string())?
    };
    if meta.plugin_id != plugin_id {
        return Err("session belongs to another plugin".into());
    }
    if let Some(w) = app.get_webview_window(&meta.label) {
        let _ = w.close();
    }
    state().lock().sessions.remove(&opts.session_id);
    Ok(())
}

pub fn navigate(app: AppHandle, plugin_id: String, opts: NavigateOpts) -> Result<(), String> {
    crate::plugin_hub::assert_capability(&plugin_id, "webview")?;
    let url = parse_http_url(&opts.url)?;
    let meta = {
        let st = state().lock();
        st.sessions
            .get(&opts.session_id)
            .cloned()
            .ok_or_else(|| "session not found".to_string())?
    };
    if meta.plugin_id != plugin_id {
        return Err("session belongs to another plugin".into());
    }
    let win = app
        .get_webview_window(&meta.label)
        .ok_or_else(|| "webview window missing".to_string())?;
    win.navigate(url).map_err(|e| format!("navigate: {e}"))?;
    Ok(())
}

pub async fn start_pick(
    app: AppHandle,
    plugin_id: String,
    opts: SessionIdOpts,
) -> Result<PickResult, String> {
    crate::plugin_hub::assert_capability(&plugin_id, "webview")?;
    let meta = {
        let st = state().lock();
        st.sessions
            .get(&opts.session_id)
            .cloned()
            .ok_or_else(|| "session not found".to_string())?
    };
    if meta.plugin_id != plugin_id {
        return Err("session belongs to another plugin".into());
    }
    let win = app
        .get_webview_window(&meta.label)
        .ok_or_else(|| "webview window missing".to_string())?;

    // Hold BEFORE focusing the browse window — otherwise plugin-popup is destroyed
    // on blur and the invoke/JS form never receives the selector.
    begin_pick_hold();
    let _hold = PickHoldGuard;
    let _ = win.set_focus();

    // Fire-and-forget inject (don't block on callback — WebView2 needs UI pump for clicks).
    if let Err(e) = win.eval(PICKER_JS) {
        eprintln!("[plugin-webview] pick eval() failed ({e}), trying eval_with_callback");
        eval_string(&win, PICKER_JS).map_err(|e2| format!("inject picker failed: {e} / {e2}"))?;
    }

    // Verify shield is active (short polls).
    let mut active = false;
    for _ in 0..25 {
        let raw = eval_string(&win, VERIFY_PICK_ACTIVE_JS).unwrap_or_else(|_| "0".into());
        if json_unescape(&raw).trim() == "1" {
            active = true;
            break;
        }
        async_delay_ms(80).await;
    }
    if !active {
        eprintln!(
            "[plugin-webview] pick inject not confirmed via eval — continuing (hash/title may still work)"
        );
    } else {
        eprintln!("[plugin-webview] pick injected OK");
    }

    let outcome = await_pick_result(&app, &win, Duration::from_millis(PICK_TIMEOUT_MS)).await;
    if let Err(ref e) = outcome {
        eprintln!("[plugin-webview] pick failed: {e}");
    } else {
        eprintln!("[plugin-webview] pick ok");
    }
    publish_pick(&app, &plugin_id, &opts.session_id, &outcome);

    refocus_plugin_popup(&app, &plugin_id);
    async_delay_ms(150).await;
    outcome
}

/// Peek the last successful pick for this plugin (does not clear — survives remount).
pub fn take_last_pick(plugin_id: String) -> Result<Option<serde_json::Value>, String> {
    crate::plugin_hub::assert_capability(&plugin_id, "webview")?;
    let last = state().lock().last_pick.get(&plugin_id).cloned();
    Ok(last.map(|p| {
        serde_json::json!({
            "sessionId": p.session_id,
            "selector": p.result.selector,
            "textPreview": p.result.text_preview,
            "outerHtml": p.result.outer_html,
        })
    }))
}

pub fn clear_last_pick(plugin_id: String) -> Result<(), String> {
    crate::plugin_hub::assert_capability(&plugin_id, "webview")?;
    state().lock().last_pick.remove(&plugin_id);
    Ok(())
}

pub async fn snapshot(
    app: AppHandle,
    plugin_id: String,
    opts: SnapshotOpts,
) -> Result<SnapshotResult, String> {
    crate::plugin_hub::assert_capability(&plugin_id, "webview")?;
    let selector = opts.selector.trim();
    if selector.is_empty() {
        return Err("selector required".into());
    }

    let win = if let Some(sid) = opts.session_id.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty())
    {
        let meta = {
            let st = state().lock();
            st.sessions
                .get(sid)
                .cloned()
                .ok_or_else(|| "session not found".to_string())?
        };
        if meta.plugin_id != plugin_id {
            return Err("session belongs to another plugin".into());
        }
        app.get_webview_window(&meta.label)
            .ok_or_else(|| "webview window missing".to_string())?
    } else {
        let url_raw = opts
            .url
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "url or sessionId required".to_string())?;
        let url = parse_http_url(&url_raw)?;
        let win = ensure_watch_window_sync(&app, &plugin_id)?;
        win.navigate(url).map_err(|e| format!("navigate: {e}"))?;
        let _ = wait_document_ready(&win, Duration::from_millis(LOAD_WAIT_MS));
        return wait_for_selector(&win, selector, Duration::from_millis(SELECTOR_WAIT_MS));
    };

    snapshot_selector_resilient(&win, selector)
}

pub async fn watch_start(
    app: AppHandle,
    plugin_id: String,
    opts: WatchStartOpts,
) -> Result<(), String> {
    crate::plugin_hub::assert_capability(&plugin_id, "webview")?;
    let id = opts.id.trim().to_string();
    if id.is_empty() {
        return Err("watch id required".into());
    }
    let url = parse_http_url(&opts.url)?.to_string();
    let selector = opts.selector.trim().to_string();
    if selector.is_empty() {
        return Err("selector required".into());
    }
    let interval_ms = opts
        .interval_ms
        .unwrap_or(MIN_INTERVAL_MS)
        .clamp(MIN_INTERVAL_MS, MAX_INTERVAL_MS);
    let title = opts
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("网页监测")
        .to_string();

    {
        let st = state().lock();
        let count = st.watches.keys().filter(|(p, _)| p == &plugin_id).count();
        if count >= MAX_WATCHES_PER_PLUGIN && !st.watches.contains_key(&(plugin_id.clone(), id.clone()))
        {
            return Err(format!("max {MAX_WATCHES_PER_PLUGIN} watches per plugin"));
        }
    }

    let _ = ensure_watch_window_sync(&app, &plugin_id)?;

    let next_due = {
        let st = state().lock();
        match st.watches.get(&(plugin_id.clone(), id.clone())) {
            Some(existing)
                if existing.url == url
                    && existing.selector == selector
                    && existing.interval_ms == interval_ms =>
            {
                // Popup remount / syncWatches must not reset the schedule to "now+2s".
                existing.next_due
            }
            _ => Instant::now() + Duration::from_secs(2),
        }
    };

    state().lock().watches.insert(
        (plugin_id.clone(), id.clone()),
        WatchMeta {
            plugin_id,
            id,
            url,
            selector,
            interval_ms,
            title,
            next_due,
        },
    );
    start_runner(app);
    Ok(())
}

pub fn watch_stop(plugin_id: String, opts: WatchIdOpts) -> Result<(), String> {
    crate::plugin_hub::assert_capability(&plugin_id, "webview")?;
    let id = opts.id.trim().to_string();
    let mut st = state().lock();
    st.watches.remove(&(plugin_id.clone(), id.clone()));
    st.last_text.remove(&(plugin_id, id));
    Ok(())
}

pub fn watch_list(plugin_id: String) -> Result<Vec<WatchInfo>, String> {
    crate::plugin_hub::assert_capability(&plugin_id, "webview")?;
    let st = state().lock();
    let mut list: Vec<WatchInfo> = st
        .watches
        .values()
        .filter(|w| w.plugin_id == plugin_id)
        .map(|w| WatchInfo {
            id: w.id.clone(),
            url: w.url.clone(),
            selector: w.selector.clone(),
            interval_ms: w.interval_ms,
            title: w.title.clone(),
        })
        .collect();
    list.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(list)
}
