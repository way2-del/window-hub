//! 网易云「正在播放 / 当前歌词」轻量读取。
//!
//! 不做进程堆扫 / RVA 探测 / UIA（3.1.36 上又慢又卡 UI）。
//! 策略：
//! 1. 检测 `DesktopLyrics` + 主窗口标题（歌名 - 歌手）
//! 2. 官方 LRC + 本地播放时钟选句（SMTC Position 在 3.1.x 常卡 0）
//! 3. HTTP 拉 LRC 只在后台线程，热路径绝不阻塞

use serde::Serialize;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowTextLengthW, GetWindowTextW, IsWindowVisible,
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
    last_raw_ms: u64,
}

static API_LYRIC_CACHE: Mutex<Option<ApiLyricCache>> = Mutex::new(None);
static STICKY_LYRIC: Mutex<Option<StickyLyric>> = Mutex::new(None);
static SMTC_CLOCK: Mutex<Option<SmtcClock>> = Mutex::new(None);
static LAST_SONG_KEY: Mutex<String> = Mutex::new(String::new());
/// 切歌后作废进行中的旧 LRC 拉取，避免把上一首歌词写进缓存。
static LRC_FETCH_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static LRC_FETCH_BUSY: AtomicBool = AtomicBool::new(false);

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
    main: Option<HWND>,
    lyric: Option<HWND>,
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut EnumCtx);
    let class = hwnd_class(hwnd);
    let class_l = class.to_ascii_lowercase();
    let visible = IsWindowVisible(hwnd).as_bool();

    if class == "OrpheusBrowserHost" {
        if visible && ctx.main.is_none() {
            ctx.main = Some(hwnd);
        }
    } else if class == "DesktopLyrics"
        || class_l == "desktoplyrics"
        || class_l.contains("desktoplyric")
    {
        // 桌面歌词可不开置顶；隐藏窗也算「开了桌面歌词」
        ctx.lyric = Some(hwnd);
    }
    BOOL(1)
}

fn find_netease_hwnds() -> (Option<HWND>, Option<HWND>) {
    let mut ctx = EnumCtx {
        main: None,
        lyric: None,
    };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut _ as isize));
    }
    (ctx.main, ctx.lyric)
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
    if !out.active || !out.desktop_lyrics {
        let _ = STICKY_LYRIC.lock().map(|mut g| *g = None);
        return;
    }
    if let Ok(g) = STICKY_LYRIC.lock() {
        if let Some(s) = g.as_ref() {
            // 仅同曲且短窗口；切歌后 on_song_changed 已清空
            if s.song_key == key && s.at.elapsed() < Duration::from_secs(6) {
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

fn filetime_now_ticks() -> i64 {
    use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    let ft = unsafe { GetSystemTimeAsFileTime() };
    ((ft.dwHighDateTime as i64) << 32) | (ft.dwLowDateTime as i64)
}

fn ticks_to_ms(ticks: i64) -> u64 {
    (ticks.max(0) as u64) / 10_000
}

struct SmtcSample {
    song_key: String,
    raw_ms: u64,
    end_ms: Option<u64>,
    playing: bool,
}

fn read_smtc_sample(expect_title: Option<&str>) -> Option<SmtcSample> {
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSessionManager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus,
    };

    let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
        .ok()?
        .get()
        .ok()?;

    let mut best: Option<(i32, SmtcSample)> = None;
    if let Ok(sessions) = manager.GetSessions() {
        let n = sessions.Size().unwrap_or(0);
        for i in 0..n {
            let Ok(session) = sessions.GetAt(i) else {
                continue;
            };
            let app = session
                .SourceAppUserModelId()
                .map(|s| s.to_string())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let mut score = 0;
            if app.contains("cloudmusic") || app.contains("netease") {
                score += 100;
            }
            let media_title = session
                .TryGetMediaPropertiesAsync()
                .ok()
                .and_then(|op| op.get().ok())
                .and_then(|p| p.Title().ok())
                .map(|s| s.to_string())
                .unwrap_or_default();
            if let Some(want) = expect_title.filter(|s| !s.is_empty()) {
                let w = want.to_ascii_lowercase();
                let t = media_title.to_ascii_lowercase();
                if !t.is_empty() && (t.contains(&w) || w.contains(&t)) {
                    score += 40;
                }
            }
            if score <= 0 {
                continue;
            }
            let Ok(timeline) = session.GetTimelineProperties() else {
                continue;
            };
            let Ok(pos) = timeline.Position() else {
                continue;
            };
            let raw_ms = ticks_to_ms(pos.Duration);
            let end_ms = timeline
                .EndTime()
                .ok()
                .map(|e| ticks_to_ms(e.Duration))
                .filter(|&ms| ms > 0);
            // 轻外推：Position 卡 0 时仍可能靠 LastUpdated 无效，后面用本地钟
            let _ = timeline.LastUpdatedTime().map(|last| {
                let _ = filetime_now_ticks().saturating_sub(last.UniversalTime);
            });
            let playing = session
                .GetPlaybackInfo()
                .ok()
                .and_then(|info| info.PlaybackStatus().ok())
                .map(|s| s == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing)
                .unwrap_or(true);
            let song_key = expect_title
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or(media_title);
            let sample = SmtcSample {
                song_key,
                raw_ms,
                end_ms,
                playing,
            };
            if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
                best = Some((score, sample));
            }
        }
    }
    best.map(|(_, s)| s)
}

/// 播放进度（ms）。SMTC 卡 0 时用本地时钟推进。
/// WinRT 采样最多约 2s 一次，热路径多数只做 Instant 加法，避免卡 UI。
fn playback_position_ms(expect_title: Option<&str>) -> u64 {
    static LAST_SMTC_POLL: Mutex<Option<Instant>> = Mutex::new(None);
    let need_poll = LAST_SMTC_POLL
        .lock()
        .ok()
        .and_then(|g| g.map(|t| t.elapsed() >= Duration::from_secs(2)))
        .unwrap_or(true);

    if need_poll {
        if let Some(sample) = read_smtc_sample(expect_title) {
            if let Ok(mut guard) = SMTC_CLOCK.lock() {
                let reported = sample.raw_ms;
                match guard.as_mut() {
                    Some(clock) if clock.song_key == sample.song_key => {
                        let delta = reported as i64 - clock.last_raw_ms as i64;
                        if delta.abs() >= 1200
                            || (reported > 0 && reported != clock.last_raw_ms && delta >= 400)
                        {
                            clock.origin_ms = reported;
                            clock.synced_at = Instant::now();
                        }
                        clock.last_raw_ms = reported;
                        if !sample.playing {
                            let frozen = clock
                                .origin_ms
                                .saturating_add(clock.synced_at.elapsed().as_millis() as u64);
                            let frozen = sample.end_ms.map(|e| frozen.min(e)).unwrap_or(frozen);
                            clock.origin_ms = frozen;
                            clock.synced_at = Instant::now();
                        }
                    }
                    _ => {
                        *guard = Some(SmtcClock {
                            song_key: sample.song_key,
                            origin_ms: reported,
                            synced_at: Instant::now(),
                            last_raw_ms: reported,
                        });
                    }
                }
            }
            if let Ok(mut g) = LAST_SMTC_POLL.lock() {
                *g = Some(Instant::now());
            }
        }
    }

    let Ok(guard) = SMTC_CLOCK.lock() else {
        return 0;
    };
    let Some(clock) = guard.as_ref() else {
        return 0;
    };
    clock
        .origin_ms
        .saturating_add(clock.synced_at.elapsed().as_millis() as u64)
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
    let pos = playback_position_ms(Some(title));
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
    out.desktop_lyrics = lyric_hwnd.is_some();

    if let Some(hwnd) = main {
        let _ = unsafe { IsWindowVisible(hwnd) };
        out.active = true;
        let title_raw = hwnd_title(hwnd);
        let (title, artist) = parse_title_artist(&title_raw);
        out.title = title;
        out.artist = artist;
        out.source = Some("window-title".into());
    } else if lyric_hwnd.is_some() {
        out.active = true;
    }

    let key = song_key(out.title.as_deref(), out.artist.as_deref());
    if !key.is_empty() && key != "|" {
        on_song_changed(&key);
    }

    // 不再用桌面歌词窗标题当歌词（置顶时是「解锁」，也易串）

    if out.desktop_lyrics {
        if let Some(title) = out.title.clone() {
            if let Some(line) = lyric_from_cached_lrc(&title, out.artist.as_deref()) {
                out.lyric = Some(line);
                out.source = Some("api-lrc".into());
            } else {
                schedule_lrc_fetch(title, out.artist.clone());
            }
        }
    }

    if out.title.is_some() || out.lyric.is_some() || out.desktop_lyrics {
        out.active = true;
    }
    apply_sticky(&mut out);
    out
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
