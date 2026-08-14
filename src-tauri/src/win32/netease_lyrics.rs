//! 网易云「正在播放 / 当前歌词」轻量读取。
//!
//! 不做进程堆扫 / RVA 探测 / UIA / SMTC WinRT（后者易与 UI COM 死锁导致整窗未响应）。
//! 策略：
//! 1. 检测 `DesktopLyrics` + 主窗口标题（含隐藏主窗）
//! 2. 官方 LRC + 本地播放时钟选句
//! 3. HTTP 拉 LRC 只在后台线程，热路径绝不阻塞

use serde::Serialize;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GetClassNameW, GetWindowTextLengthW, GetWindowTextW,
    IsWindowVisible,
};

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseNowPlaying {
    pub active: bool,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub lyric: Option<String>,
    pub source: Option<String>,
    /// 是否检测到桌面歌词窗口（无需置顶）
    pub desktop_lyrics: bool,
}

struct ApiLyricCache {
    key: String,
    lines: Vec<(u64, String)>,
    at: Instant,
}

struct StickyLyric {
    song_key: String,
    text: String,
    at: Instant,
}

struct SmtcClock {
    song_key: String,
    origin_ms: u64,
    synced_at: Instant,
    /// 暂停时冻结的进度（ms）；Some 则不再用墙钟推进
    frozen_ms: Option<u64>,
}

static API_LYRIC_CACHE: Mutex<Option<ApiLyricCache>> = Mutex::new(None);
static STICKY_LYRIC: Mutex<Option<StickyLyric>> = Mutex::new(None);
static SMTC_CLOCK: Mutex<Option<SmtcClock>> = Mutex::new(None);
static LAST_SONG_KEY: Mutex<String> = Mutex::new(String::new());
/// 切歌后作废进行中的旧 LRC 拉取，避免把上一首歌词写进缓存。
static LRC_FETCH_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static LRC_FETCH_BUSY: AtomicBool = AtomicBool::new(false);
/// 乐观播放态：面板点暂停时冻结本地 LRC 钟；切歌 / 下一首会恢复。
static PLAYING: AtomicBool = AtomicBool::new(true);

fn wide_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    OsString::from_wide(&buf[..len])
        .to_string_lossy()
        .trim()
        .to_string()
}

fn hwnd_class(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
    if n == 0 {
        return String::new();
    }
    wide_to_string(&buf[..n])
}

fn hwnd_title(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; (len as usize) + 1];
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) } as usize;
    if n == 0 {
        return String::new();
    }
    wide_to_string(&buf[..n])
}

fn parse_title_artist(raw: &str) -> (Option<String>, Option<String>) {
    let t = raw.trim();
    if t.is_empty() || t == "网易云音乐" || t.eq_ignore_ascii_case("cloudmusic") {
        return (None, None);
    }
    for sep in [" - ", " – ", " — ", "-", "–", "—"] {
        if let Some((a, b)) = t.split_once(sep) {
            let title = a.trim();
            let artist = b.trim();
            if !title.is_empty() {
                return (
                    Some(title.to_string()),
                    if artist.is_empty() {
                        None
                    } else {
                        Some(artist.to_string())
                    },
                );
            }
        }
    }
    (Some(t.to_string()), None)
}

struct EnumCtx {
    main_visible: Option<HWND>,
    main_any: Option<HWND>,
    lyric: Option<HWND>,
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut EnumCtx);
    let class = hwnd_class(hwnd);
    let class_l = class.to_ascii_lowercase();
    let visible = IsWindowVisible(hwnd).as_bool();

    if class == "OrpheusBrowserHost" {
        if visible && ctx.main_visible.is_none() {
            ctx.main_visible = Some(hwnd);
        }
        if ctx.main_any.is_none() {
            ctx.main_any = Some(hwnd);
        }
    } else if class == "DesktopLyrics"
        || class_l == "desktoplyrics"
        || class_l.contains("desktoplyric")
    {
        // 必须可见：用户关掉桌面歌词后窗体常仍存活但隐藏，不能再当「开着」
        if visible {
            ctx.lyric = Some(hwnd);
        }
    }
    BOOL(1)
}

fn find_netease_hwnds() -> (Option<HWND>, Option<HWND>) {
    let mut ctx = EnumCtx {
        main_visible: None,
        main_any: None,
        lyric: None,
    };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut _ as isize));
    }
    (ctx.main_visible.or(ctx.main_any), ctx.lyric)
}

struct TextCollectCtx {
    texts: Vec<String>,
}

unsafe extern "system" fn text_collect_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut TextCollectCtx);
    let t = hwnd_title(hwnd);
    if !t.is_empty() {
        ctx.texts.push(t);
    }
    BOOL(1)
}

fn collect_hwnd_texts(root: HWND) -> Vec<String> {
    let mut out = Vec::new();
    let root_t = hwnd_title(root);
    if !root_t.is_empty() {
        out.push(root_t);
    }
    let mut ctx = TextCollectCtx { texts: Vec::new() };
    unsafe {
        let _ = EnumChildWindows(root, Some(text_collect_proc), LPARAM(&mut ctx as *mut _ as isize));
    }
    out.extend(ctx.texts);
    out
}

fn is_desktop_lyric_chrome(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    lower == "desktoplyrics"
        || t == "桌面歌词"
        || t == "网易云音乐"
        || lower == "cloudmusic"
        || is_unlock_noise(t)
}

/// 从桌面歌词窗读当前行（暂停时网易云会停住标题/子控件文本，不会自己往前跑）。
fn read_desktop_lyric_line(lyric_hwnd: HWND) -> Option<String> {
    let mut best: Option<String> = None;
    for t in collect_hwnd_texts(lyric_hwnd) {
        let t = t.trim().to_string();
        if is_desktop_lyric_chrome(&t) || is_lrc_credit_line(&t) {
            continue;
        }
        // 偏好更长的一行（主歌词通常比进度/副标长）
        if best.as_ref().map(|b| t.chars().count() > b.chars().count()).unwrap_or(true) {
            best = Some(t);
        }
    }
    best
}

/// 面板多媒体键回调：暂停冻结本地 LRC 钟，播放/切歌解冻。
pub fn note_media_transport(action: &str) {
    let a = action.trim().to_ascii_lowercase();
    match a.as_str() {
        "prev" | "previous" | "previoustrack" | "next" | "nexttrack" => {
            PLAYING.store(true, Ordering::Release);
            unfreeze_clock();
        }
        "play" => {
            PLAYING.store(true, Ordering::Release);
            unfreeze_clock();
        }
        "pause" => {
            freeze_clock_now();
            PLAYING.store(false, Ordering::Release);
        }
        "play-pause" | "playpause" | "toggle" => {
            if PLAYING.load(Ordering::Acquire) {
                freeze_clock_now();
                PLAYING.store(false, Ordering::Release);
            } else {
                PLAYING.store(true, Ordering::Release);
                unfreeze_clock();
            }
        }
        _ => {}
    }
}

fn freeze_clock_now() {
    if let Ok(mut guard) = SMTC_CLOCK.lock() {
        if let Some(clock) = guard.as_mut() {
            if clock.frozen_ms.is_none() {
                let pos = clock
                    .origin_ms
                    .saturating_add(clock.synced_at.elapsed().as_millis() as u64);
                clock.frozen_ms = Some(pos);
            }
        }
    }
}

fn unfreeze_clock() {
    if let Ok(mut guard) = SMTC_CLOCK.lock() {
        if let Some(clock) = guard.as_mut() {
            if let Some(pos) = clock.frozen_ms.take() {
                clock.origin_ms = pos;
                clock.synced_at = Instant::now();
            }
        }
    }
}

fn song_key(title: Option<&str>, artist: Option<&str>) -> String {
    format!("{}|{}", title.unwrap_or(""), artist.unwrap_or(""))
}

fn is_unlock_noise(s: &str) -> bool {
    let t = s.trim();
    t.contains("桌面歌词解锁") || t.contains("解锁桌面歌词")
}

fn is_lrc_credit_line(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    let keys = [
        "原唱",
        "作曲",
        "作词",
        "编曲",
        "制作人",
        "混音",
        "母带",
        "后期",
        "mastering",
        "producer",
        "composer",
        "lyricist",
        "正版授权",
    ];
    keys.iter().any(|k| t.contains(k) || lower.contains(k))
}

/// 切歌：丢掉上一首的 sticky / 进度钟，并作废进行中的 LRC 请求。
fn on_song_changed(new_key: &str) {
    let mut changed = false;
    if let Ok(mut last) = LAST_SONG_KEY.lock() {
        if last.as_str() != new_key {
            *last = new_key.to_string();
            changed = true;
        }
    }
    if !changed {
        return;
    }
    LRC_FETCH_GEN.fetch_add(1, Ordering::AcqRel);
    PLAYING.store(true, Ordering::Release);
    let _ = STICKY_LYRIC.lock().map(|mut g| *g = None);
    let _ = SMTC_CLOCK.lock().map(|mut g| *g = None);
    // 旧歌 LRC 缓存可留着（按 key 区分）；但若当前缓存 key 不是新歌则不影响 peek
}

fn apply_sticky(out: &mut NeteaseNowPlaying) {
    let key = song_key(out.title.as_deref(), out.artist.as_deref());
    if let Some(lyric) = out.lyric.as_ref().filter(|s| !s.trim().is_empty()) {
        if !is_lrc_credit_line(lyric) && !is_unlock_noise(lyric) {
            if let Ok(mut g) = STICKY_LYRIC.lock() {
                *g = Some(StickyLyric {
                    song_key: key,
                    text: lyric.clone(),
                    at: Instant::now(),
                });
            }
        }
        return;
    }
    // 桌面歌词已关：立刻丢掉 sticky，禁止关窗后继续「自己播」
    if !out.desktop_lyrics {
        let _ = STICKY_LYRIC.lock().map(|mut g| *g = None);
        return;
    }
    if !out.active {
        let _ = STICKY_LYRIC.lock().map(|mut g| *g = None);
        return;
    }
    if let Ok(g) = STICKY_LYRIC.lock() {
        if let Some(s) = g.as_ref() {
            if s.song_key == key && s.at.elapsed() < Duration::from_millis(800) {
                out.lyric = Some(s.text.clone());
                if out.source.as_deref() == Some("window-title") || out.source.is_none() {
                    out.source = Some("sticky".into());
                }
            }
        }
    }
}

fn normalize_title(s: &str) -> String {
    let s = s.split('（').next().unwrap_or(s);
    let s = s.split('(').next().unwrap_or(s);
    let s = s.split('[').next().unwrap_or(s);
    s.trim().to_ascii_lowercase()
}

/// 必须标题相关，避免搜索落到完全另一首歌（串词主因）。
fn pick_search_song_id(songs: &[serde_json::Value], title: &str, artist: Option<&str>) -> Option<u64> {
    let want_t = normalize_title(title);
    if want_t.is_empty() {
        return None;
    }
    let want_a = artist.map(normalize_title).unwrap_or_default();
    let mut best: Option<(i32, u64)> = None;
    for s in songs {
        let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let nt = normalize_title(name);
        if nt.is_empty() {
            continue;
        }
        let mut score = 0i32;
        if nt == want_t {
            score += 100;
        } else if nt.contains(&want_t) || want_t.contains(&nt) {
            // 太短的包含易误伤
            if want_t.chars().count() >= 3 && nt.chars().count() >= 3 {
                score += 55;
            } else {
                continue;
            }
        } else {
            continue;
        }
        if !want_a.is_empty() {
            let artists = s
                .get("artists")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                        .map(normalize_title)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            if artists.contains(&want_a) || want_a.contains(&artists) {
                score += 40;
            }
        }
        let Some(id) = s.get("id").and_then(|v| v.as_u64()) else {
            continue;
        };
        if best.map(|(sc, _)| score > sc).unwrap_or(true) {
            best = Some((score, id));
        }
    }
    best.and_then(|(sc, id)| if sc >= 55 { Some(id) } else { None })
}

/// 选句提前量（相对切歌后本地钟）。
const LYRIC_LEAD_MS: u64 = 2800;

/// 播放进度（ms）。只用本地钟——禁止在热路径调 WinRT SMTC（易与 WebView2 COM 死锁 → 整窗未响应）。
fn playback_position_ms(expect_title: Option<&str>) -> u64 {
    let key = expect_title
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    if key.is_empty() {
        return 0;
    }
    let playing = PLAYING.load(Ordering::Acquire);
    if let Ok(mut guard) = SMTC_CLOCK.lock() {
        match guard.as_mut() {
            Some(clock) if clock.song_key == key => {
                if !playing && clock.frozen_ms.is_none() {
                    let pos = clock
                        .origin_ms
                        .saturating_add(clock.synced_at.elapsed().as_millis() as u64);
                    clock.frozen_ms = Some(pos);
                }
            }
            _ => {
                *guard = Some(SmtcClock {
                    song_key: key,
                    origin_ms: 0,
                    synced_at: Instant::now(),
                    frozen_ms: if playing { None } else { Some(0) },
                });
            }
        }
        let Some(clock) = guard.as_ref() else {
            return 0;
        };
        if let Some(pos) = clock.frozen_ms {
            return pos;
        }
        return clock
            .origin_ms
            .saturating_add(clock.synced_at.elapsed().as_millis() as u64);
    }
    0
}

fn parse_lrc(raw: &str) -> Vec<(u64, String)> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if !line.starts_with('[') {
            continue;
        }
        let mut rest = line;
        let mut times: Vec<u64> = Vec::new();
        while rest.starts_with('[') {
            let Some(end) = rest.find(']') else {
                break;
            };
            let tag = &rest[1..end];
            rest = &rest[end + 1..];
            let mut parts = tag.split(':');
            let Some(mm) = parts.next() else {
                continue;
            };
            let Some(ss) = parts.next() else {
                continue;
            };
            if parts.next().is_some() {
                continue;
            }
            let Ok(m) = mm.parse::<u64>() else {
                continue;
            };
            let (sec_s, frac_s) = match ss.split_once('.') {
                Some((a, b)) => (a, b),
                None => (ss, "0"),
            };
            let Ok(sec) = sec_s.parse::<u64>() else {
                continue;
            };
            let frac = frac_s.chars().take(3).collect::<String>();
            let frac_ms = match frac.len() {
                0 => 0u64,
                1 => frac.parse::<u64>().unwrap_or(0) * 100,
                2 => frac.parse::<u64>().unwrap_or(0) * 10,
                _ => frac.parse::<u64>().unwrap_or(0),
            };
            times.push(m * 60_000 + sec * 1000 + frac_ms);
        }
        let text = rest.trim();
        if text.is_empty() || is_lrc_credit_line(text) {
            continue;
        }
        for ms in times {
            out.push((ms, text.to_string()));
        }
    }
    out.sort_by_key(|(t, _)| *t);
    out
}

fn lyric_line_at(lines: &[(u64, String)], pos_ms: u64) -> Option<String> {
    if lines.is_empty() {
        return None;
    }
    let mut cur = None;
    for (t, s) in lines {
        if *t <= pos_ms {
            cur = Some(s.clone());
        } else {
            break;
        }
    }
    cur.or_else(|| lines.first().map(|(_, s)| s.clone()))
}

fn http_get_json(url: &str) -> Option<serde_json::Value> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_millis(800))
        .timeout_read(std::time::Duration::from_millis(1500))
        .build();
    let resp = agent
        .get(url)
        .set(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) WindowHub/1.0",
        )
        .set("Referer", "https://music.163.com/")
        .call()
        .ok()?;
    resp.into_json().ok()
}

fn peek_cached_lrc(title: &str, artist: Option<&str>) -> Option<Vec<(u64, String)>> {
    let key = format!(
        "{}|{}",
        title.trim(),
        artist.map(|a| a.trim()).unwrap_or("")
    );
    let guard = API_LYRIC_CACHE.lock().ok()?;
    let c = guard.as_ref()?;
    if c.key == key && !c.lines.is_empty() && c.at.elapsed() < Duration::from_secs(600) {
        Some(c.lines.clone())
    } else {
        None
    }
}

fn fetch_lrc_into_cache(title: &str, artist: Option<&str>, gen: u64) {
    let key = format!(
        "{}|{}",
        title.trim(),
        artist.map(|a| a.trim()).unwrap_or("")
    );
    if peek_cached_lrc(title, artist).is_some() {
        return;
    }
    let bare = normalize_title(title);
    let q = if let Some(a) = artist.filter(|s| !s.is_empty()) {
        format!("{bare} {}", a.trim())
    } else {
        bare
    };
    let enc: String =
        percent_encoding::utf8_percent_encode(&q, percent_encoding::NON_ALPHANUMERIC).to_string();
    let search_url = format!(
        "https://music.163.com/api/search/get/web?s={enc}&type=1&offset=0&total=true&limit=8"
    );
    let Some(search) = http_get_json(&search_url) else {
        return;
    };
    if LRC_FETCH_GEN.load(Ordering::Acquire) != gen {
        return;
    }
    let Some(songs) = search.pointer("/result/songs").and_then(|v| v.as_array()) else {
        return;
    };
    let Some(id) = pick_search_song_id(songs, title, artist) else {
        return;
    };
    let lyric_url = format!("https://music.163.com/api/song/lyric?id={id}&lv=-1&kv=-1&tv=-1");
    let Some(lyric_json) = http_get_json(&lyric_url) else {
        return;
    };
    if LRC_FETCH_GEN.load(Ordering::Acquire) != gen {
        return;
    }
    let lrc = lyric_json
        .pointer("/lrc/lyric")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let lines = parse_lrc(lrc);
    if lines.is_empty() {
        return;
    }
    if LRC_FETCH_GEN.load(Ordering::Acquire) != gen {
        return;
    }
    if let Ok(mut guard) = API_LYRIC_CACHE.lock() {
        *guard = Some(ApiLyricCache {
            key,
            lines,
            at: Instant::now(),
        });
    }
}

fn schedule_lrc_fetch(title: String, artist: Option<String>) {
    if peek_cached_lrc(&title, artist.as_deref()).is_some() {
        return;
    }
    // 允许切歌打断：busy 时若 gen 已变，仍可再开一枪
    if LRC_FETCH_BUSY.load(Ordering::Acquire) {
        return;
    }
    if LRC_FETCH_BUSY.swap(true, Ordering::AcqRel) {
        return;
    }
    let gen = LRC_FETCH_GEN.load(Ordering::Acquire);
    let _ = std::thread::Builder::new()
        .name("netease-lrc-fetch".into())
        .spawn(move || {
            fetch_lrc_into_cache(&title, artist.as_deref(), gen);
            LRC_FETCH_BUSY.store(false, Ordering::Release);
            // 若拉取期间又切歌且缓存仍空，下次 snapshot 会再 schedule
        });
}

fn lyric_from_cached_lrc(title: &str, artist: Option<&str>) -> Option<String> {
    let lines = peek_cached_lrc(title, artist)?;
    let pos = playback_position_ms(Some(title)).saturating_add(LYRIC_LEAD_MS);
    lyric_line_at(&lines, pos)
}

/// 热路径快照：只 EnumWindows + 读缓存 LRC，绝不 HTTP / 扫内存。
pub fn snapshot() -> NeteaseNowPlaying {
    static CACHE: Mutex<Option<(Instant, NeteaseNowPlaying)>> = Mutex::new(None);
    if let Ok(guard) = CACHE.lock() {
        if let Some((at, snap)) = guard.as_ref() {
            if at.elapsed() < Duration::from_millis(400) {
                return snap.clone();
            }
        }
    }
    let snap = snapshot_uncached();
    if let Ok(mut guard) = CACHE.lock() {
        *guard = Some((Instant::now(), snap.clone()));
    }
    snap
}

fn snapshot_uncached() -> NeteaseNowPlaying {
    let (main, lyric_hwnd) = find_netease_hwnds();
    let mut out = NeteaseNowPlaying::default();
    // 仅「可见」的 DesktopLyrics 才算开启——监听网易云桌面歌词开关，不自创通道
    out.desktop_lyrics = lyric_hwnd.is_some();

    if let Some(hwnd) = main {
        out.active = true;
        let title_raw = hwnd_title(hwnd);
        let (title, artist) = parse_title_artist(&title_raw);
        out.title = title;
        out.artist = artist;
        if out.title.is_some() {
            out.source = Some("window-title".into());
        }
    }

    let key = song_key(out.title.as_deref(), out.artist.as_deref());
    if !key.is_empty() && key != "|" {
        on_song_changed(&key);
    }

    if out.desktop_lyrics {
        out.active = true;
        // 优先跟听桌面歌词窗文本（暂停时网易云停住，岛栏不会自己往前跑）
        let desk_line = lyric_hwnd.and_then(read_desktop_lyric_line);
        if let Some(line) = desk_line {
            out.lyric = Some(line);
            out.source = Some("desktop-lyrics".into());
            // 用桌面行校准本地钟，避免短暂读空时 api-lrc 乱跳
            if let Some(title) = out.title.as_deref() {
                sync_clock_to_line(title, out.artist.as_deref(), out.lyric.as_deref().unwrap_or(""));
            }
        } else if let Some(title) = out.title.clone() {
            // 桌面窗读不到字时才用官方 LRC + 本地钟（尊重 PLAYING 冻结）
            if let Some(line) = lyric_from_cached_lrc(&title, out.artist.as_deref()) {
                out.lyric = Some(line);
                out.source = Some("api-lrc".into());
            } else {
                schedule_lrc_fetch(title, out.artist.clone());
            }
        }
    } else {
        // 关桌面歌词：清空歌词与 sticky，岛栏应让位
        out.lyric = None;
        let _ = STICKY_LYRIC.lock().map(|mut g| *g = None);
    }

    if out.title.is_some() || out.lyric.is_some() || out.desktop_lyrics {
        out.active = true;
    }
    apply_sticky(&mut out);
    out
}

/// 桌面歌词当前行 → 把本地钟钉到该 LRC 时间戳，暂停/短暂丢字时不乱进。
fn sync_clock_to_line(title: &str, artist: Option<&str>, line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    let Some(lines) = peek_cached_lrc(title, artist) else {
        return;
    };
    let Some((ms, _)) = lines.iter().find(|(_, s)| s.trim() == line) else {
        return;
    };
    let key = title.trim().to_ascii_lowercase();
    if key.is_empty() {
        return;
    }
    // 选句时会再加 LYRIC_LEAD_MS，这里反推 origin
    let origin = ms.saturating_sub(LYRIC_LEAD_MS);
    if let Ok(mut guard) = SMTC_CLOCK.lock() {
        let frozen = if PLAYING.load(Ordering::Acquire) {
            None
        } else {
            Some(origin)
        };
        *guard = Some(SmtcClock {
            song_key: key,
            origin_ms: origin,
            synced_at: Instant::now(),
            frozen_ms: frozen,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credit_filter() {
        assert!(is_lrc_credit_line("母带后期处理：Mastering"));
        assert!(is_lrc_credit_line("【本歌曲已获得正版授权】"));
        assert!(!is_lrc_credit_line("每天一张开眼睛就会想到你"));
    }

    #[test]
    fn unlock_noise() {
        assert!(is_unlock_noise("桌面歌词解锁"));
        assert!(!is_unlock_noise("有没有暂停键可以stop"));
    }
}
