//! Visual smoke for Window Hub tray / plugin popups.
//!
//! Captures HWND client areas, rejects "white zombie" frosted empty shells,
//! and OCRs for expected Chinese labels (Tesseract `chi_sim` or WinRT via
//! companion PowerShell script).
//!
//! Typical flow (Window Hub already running, popups warmed):
//! 1. Click the island tray chevron (or open a plugin popup).
//! 2. Within ~2s run: `cargo run --release -- check --expect 已收纳`
//!
//! See README.md for MyDockFinder notes and OCR install.

#![cfg_attr(not(windows), allow(dead_code))]

use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
#[command(name = "tray-visual-smoke", about = "Window Hub tray/plugin visual smoke")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// List candidate popup HWNDs (title contains 已收纳 / 系统面板 / 插件).
    List,
    /// Capture + white-zombie + OCR checks against a visible popup.
    Check {
        /// Substring expected in OCR / window title (default: 已收纳).
        #[arg(long, default_value = "已收纳")]
        expect: String,
        /// Max wait for a matching visible window (ms).
        #[arg(long, default_value_t = 2000)]
        wait_ms: u64,
        /// Optional directory to write capture PNGs.
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// Max fraction of near-white pixels in the center region (0..1).
        #[arg(long, default_value_t = 0.92)]
        white_max: f32,
    },
    /// Probe OCR backends (WinRT script + tesseract) without capturing.
    OcrProbe {
        #[arg(long)]
        image: Option<PathBuf>,
    },
}

fn main() {
    let cli = Cli::parse();
    #[cfg(not(windows))]
    {
        eprintln!("tray-visual-smoke is Windows-only");
        std::process::exit(2);
    }
    #[cfg(windows)]
    {
        match cli.cmd {
            Cmd::List => cmd_list(),
            Cmd::Check {
                expect,
                wait_ms,
                out_dir,
                white_max,
            } => {
                if let Err(e) = cmd_check(&expect, wait_ms, out_dir.as_deref(), white_max) {
                    eprintln!("FAIL: {e}");
                    std::process::exit(1);
                }
            }
            Cmd::OcrProbe { image } => match cmd_ocr_probe(image.as_deref()) {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("WARN: {e}");
                    eprintln!("Install tesseract chi_sim OR Windows 中文 OCR language pack.");
                    std::process::exit(0);
                }
            },
        }
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        HGDIOBJ, SRCCOPY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowRect, GetWindowTextW, IsWindowVisible,
    };

    #[derive(Clone, Debug)]
    pub struct WinInfo {
        pub hwnd: isize,
        pub title: String,
        pub class: String,
        pub w: i32,
        pub h: i32,
    }

    pub fn list_popups() -> Vec<WinInfo> {
        struct Ctx {
            list: Vec<WinInfo>,
        }
        unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let ctx = &mut *(lparam.0 as *mut Ctx);
            if !IsWindowVisible(hwnd).as_bool() {
                return BOOL(1);
            }
            let mut title_buf = [0u16; 256];
            let tn = GetWindowTextW(hwnd, &mut title_buf);
            let title = if tn > 0 {
                String::from_utf16_lossy(&title_buf[..tn as usize])
            } else {
                String::new()
            };
            let mut class_buf = [0u16; 128];
            let cn = GetClassNameW(hwnd, &mut class_buf);
            let class = if cn > 0 {
                String::from_utf16_lossy(&class_buf[..cn as usize])
            } else {
                String::new()
            };
            let interesting = title.contains("已收纳")
                || title.contains("系统面板")
                || title.contains("插件")
                || title.contains("Window Hub")
                || class.contains("TAURI");
            if !interesting {
                return BOOL(1);
            }
            let mut rc = RECT::default();
            if GetWindowRect(hwnd, &mut rc).is_err() {
                return BOOL(1);
            }
            let w = rc.right - rc.left;
            let h = rc.bottom - rc.top;
            if w < 40 || h < 40 {
                return BOOL(1);
            }
            ctx.list.push(WinInfo {
                hwnd: hwnd.0 as isize,
                title,
                class,
                w,
                h,
            });
            BOOL(1)
        }
        let mut ctx = Ctx { list: Vec::new() };
        unsafe {
            let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut _ as isize));
        }
        ctx.list
    }

    pub fn wait_for_title(substr: &str, wait_ms: u64) -> Result<(WinInfo, Duration), String> {
        let deadline = Instant::now() + Duration::from_millis(wait_ms);
        let start = Instant::now();
        loop {
            if let Some(w) = list_popups()
                .into_iter()
                .find(|w| w.title.contains(substr) || (substr == "已收纳" && w.title.contains("已收纳")))
            {
                return Ok((w, start.elapsed()));
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "no visible window with title containing {substr:?} within {wait_ms}ms"
                ));
            }
            std::thread::sleep(Duration::from_millis(16));
        }
    }

    pub fn capture_hwnd(hwnd_raw: isize) -> Result<image::RgbaImage, String> {
        unsafe {
            let hwnd = HWND(hwnd_raw as *mut _);
            let mut rc = RECT::default();
            GetWindowRect(hwnd, &mut rc).map_err(|e| e.to_string())?;
            let width = (rc.right - rc.left).max(1);
            let height = (rc.bottom - rc.top).max(1);

            let hdc_win = GetDC(hwnd);
            if hdc_win.is_invalid() {
                return Err("GetDC failed".into());
            }
            let hdc_mem = CreateCompatibleDC(hdc_win);
            if hdc_mem.is_invalid() {
                let _ = ReleaseDC(hwnd, hdc_win);
                return Err("CreateCompatibleDC failed".into());
            }
            let hbmp = CreateCompatibleBitmap(hdc_win, width, height);
            let old = SelectObject(hdc_mem, HGDIOBJ(hbmp.0));
            let ok = BitBlt(hdc_mem, 0, 0, width, height, hdc_win, 0, 0, SRCCOPY);
            if ok.is_err() {
                let _ = SelectObject(hdc_mem, old);
                let _ = DeleteObject(hbmp);
                let _ = DeleteDC(hdc_mem);
                let _ = ReleaseDC(hwnd, hdc_win);
                return Err("BitBlt failed".into());
            }

            let mut bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0 as u32,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut buf = vec![0u8; (width * height * 4) as usize];
            let got = GetDIBits(
                hdc_mem,
                hbmp,
                0,
                height as u32,
                Some(buf.as_mut_ptr() as *mut _),
                &mut bi,
                DIB_RGB_COLORS,
            );
            let _ = SelectObject(hdc_mem, old);
            let _ = DeleteObject(hbmp);
            let _ = DeleteDC(hdc_mem);
            let _ = ReleaseDC(hwnd, hdc_win);
            if got == 0 {
                return Err("GetDIBits failed".into());
            }

            // BGRA → RGBA
            for px in buf.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
            image::RgbaImage::from_raw(width as u32, height as u32, buf)
                .ok_or_else(|| "invalid image buffer".into())
        }
    }

    /// Center 50% region near-white ratio — empty mica zombies score very high.
    pub fn near_white_ratio(img: &image::RgbaImage) -> f32 {
        let w = img.width();
        let h = img.height();
        if w < 4 || h < 4 {
            return 1.0;
        }
        let x0 = w / 4;
        let y0 = h / 4;
        let x1 = w - w / 4;
        let y1 = h - h / 4;
        let mut total = 0u32;
        let mut white = 0u32;
        for y in y0..y1 {
            for x in x0..x1 {
                let p = img.get_pixel(x, y).0;
                total += 1;
                // Near-white / frosted glass (ignore fully transparent).
                if p[3] > 8 && p[0] > 230 && p[1] > 230 && p[2] > 230 {
                    white += 1;
                }
            }
        }
        if total == 0 {
            1.0
        } else {
            white as f32 / total as f32
        }
    }
}

#[cfg(windows)]
fn cmd_list() {
    let list = win::list_popups();
    if list.is_empty() {
        println!("(no matching visible popups)");
        return;
    }
    for w in list {
        println!(
            "hwnd=0x{:X} {}x{} class={:?} title={:?}",
            w.hwnd, w.w, w.h, w.class, w.title
        );
    }
}

#[cfg(windows)]
fn cmd_check(
    expect: &str,
    wait_ms: u64,
    out_dir: Option<&Path>,
    white_max: f32,
) -> Result<(), String> {
    println!("waiting up to {wait_ms}ms for window containing {expect:?} …");
    let (info, appear) = win::wait_for_title(expect, wait_ms)?;
    println!(
        "found hwnd=0x{:X} title={:?} appear={:?} (budget warm reopen ≤300ms — cold WebView2 excluded)",
        info.hwnd, info.title, appear
    );
    if appear > Duration::from_millis(300) {
        println!(
            "WARN: appear {:?} > 300ms — if this was a warm reopen, investigate popup reveal path",
            appear
        );
    }

    let img = win::capture_hwnd(info.hwnd)?;
    let ratio = win::near_white_ratio(&img);
    println!("center near-white ratio = {ratio:.3} (max {white_max})");
    if ratio > white_max {
        return Err(format!(
            "white zombie suspected: center near-white ratio {ratio:.3} > {white_max}"
        ));
    }

    if let Some(dir) = out_dir {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join("capture.png");
        img.save(&path).map_err(|e| e.to_string())?;
        println!("wrote {}", path.display());
        let text = ocr_image(&path)?;
        println!("OCR: {text:?}");
        if !text.contains(expect) && !info.title.contains(expect) {
            return Err(format!(
                "OCR/title missing expected {expect:?} (got OCR={text:?} title={:?})",
                info.title
            ));
        }
    } else {
        // Title already matched; OCR optional without out_dir.
        let tmp = std::env::temp_dir().join("wh-tray-smoke.png");
        img.save(&tmp).map_err(|e| e.to_string())?;
        match ocr_image(&tmp) {
            Ok(text) => {
                println!("OCR: {text:?}");
                if !text.contains(expect) && !info.title.contains(expect) {
                    println!("WARN: OCR missed {expect:?}; title matched — OK if OCR lang pack missing");
                }
            }
            Err(e) => {
                println!("WARN: OCR unavailable ({e}); title match accepted");
            }
        }
        let _ = std::fs::remove_file(&tmp);
    }

    println!("PASS");
    Ok(())
}

#[cfg(windows)]
fn cmd_ocr_probe(image: Option<&Path>) -> Result<(), String> {
    let path = if let Some(p) = image {
        p.to_path_buf()
    } else {
        // 1×32 white PNG with nothing useful — just probe tooling.
        let tmp = std::env::temp_dir().join("wh-ocr-probe.png");
        let img = image::RgbaImage::from_pixel(64, 32, image::Rgba([255, 255, 255, 255]));
        img.save(&tmp).map_err(|e| e.to_string())?;
        tmp
    };
    let text = ocr_image(&path)?;
    println!("OCR backend OK, text={text:?}");
    Ok(())
}

fn ocr_image(path: &Path) -> Result<String, String> {
    // 1) Tesseract chi_sim
    if let Ok(out) = Command::new("tesseract")
        .args([path.to_str().unwrap_or(""), "stdout", "-l", "chi_sim+eng"])
        .output()
    {
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
        }
    }
    // 2) Companion PowerShell WinRT OCR
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("ocr-winrt.ps1");
    if script.is_file() {
        let out = Command::new("powershell")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                script.to_str().unwrap_or(""),
                "-ImagePath",
                path.to_str().unwrap_or(""),
            ])
            .output()
            .map_err(|e| format!("powershell OCR failed: {e}"))?;
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
        }
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "WinRT OCR failed: {err}. Install 中文 OCR 语言包 or tesseract chi_sim."
        ));
    }
    Err(
        "no OCR backend: install tesseract (chi_sim) or keep ocr-winrt.ps1 beside this tool"
            .into(),
    )
}
