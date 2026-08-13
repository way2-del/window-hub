//! Parse MyDockFinder `.dockico.ini` → DockItem list.

use super::DockItem;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub fn parse_dockico_ini(path: &Path) -> Result<Vec<DockItem>, String> {
    let bytes = fs::read(path).map_err(|e| format!("read ini: {e}"))?;
    let text = decode_ini_bytes(&bytes)?;
    parse_dockico_text(&text)
}

/// MyDockFinder writes `.dockico.ini` as UTF-16 LE (often with BOM).
fn decode_ini_bytes(bytes: &[u8]) -> Result<String, String> {
    if bytes.is_empty() {
        return Ok(String::new());
    }
    // UTF-8 BOM
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(bytes[3..].to_vec()).map_err(|e| format!("ini utf-8: {e}"));
    }
    // UTF-16 LE BOM
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return decode_utf16_le(&bytes[2..]);
    }
    // UTF-16 BE BOM
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return decode_utf16_be(&bytes[2..]);
    }
    // No BOM: prefer UTF-8, then UTF-16 LE (common on Windows exports)
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Ok(s.to_string());
    }
    if bytes.len() >= 2 && bytes.len() % 2 == 0 {
        // Heuristic: many NULs in odd positions → UTF-16 LE without BOM
        let nul_odds = bytes.iter().skip(1).step_by(2).filter(|&&b| b == 0).count();
        if nul_odds * 2 >= bytes.len() / 2 {
            return decode_utf16_le(bytes);
        }
    }
    // Last resort: lossy Windows ANSI-ish via UTF-8 lossy (keeps ASCII keys)
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

fn decode_utf16_le(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() % 2 != 0 {
        return Err("ini utf-16 le: odd length".into());
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16(&units).map_err(|e| format!("ini utf-16 le: {e}"))
}

fn decode_utf16_be(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() % 2 != 0 {
        return Err("ini utf-16 be: odd length".into());
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16(&units).map_err(|e| format!("ini utf-16 be: {e}"))
}

pub fn parse_dockico_text(text: &str) -> Result<Vec<DockItem>, String> {
    let mut sections: BTreeMap<u32, BTreeMap<String, String>> = BTreeMap::new();
    let mut cur: Option<u32> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = &line[1..line.len() - 1];
            let num = name
                .strip_prefix("ico")
                .or_else(|| name.strip_prefix("ICO"))
                .and_then(|s| s.parse::<u32>().ok());
            cur = num;
            if let Some(n) = num {
                sections.entry(n).or_default();
            }
            continue;
        }
        let Some(n) = cur else { continue };
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        sections
            .entry(n)
            .or_default()
            .insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
    }

    let mut out = Vec::new();
    for (idx, map) in sections {
        let id = format!("ico{idx}");
        if map.get("delimiter").map(|s| s == "1").unwrap_or(false) {
            out.push(DockItem {
                id,
                kind: "separator".into(),
                label: String::new(),
                match_exe: String::new(),
                launch_path: String::new(),
                real_path: String::new(),
                virtual_path: String::new(),
                icon_path: String::new(),
                uwp: false,
                icon_png: None,
                icon_scale: 1.0,
                icon_offset_x: 0.0,
                icon_offset_y: 0.0,
                icon_bg: String::new(),
            });
            continue;
        }

        let appname = map.get("appname").cloned().unwrap_or_default();
        let tag = map.get("tag").cloned().unwrap_or_else(|| appname.clone());
        let filepath = map.get("filepath").cloned().unwrap_or_default();
        let realpath = map.get("realpath").cloned().unwrap_or_default();
        let virtualpath = map.get("virtualpath").cloned().unwrap_or_default();
        let icopath = map.get("icopath").cloned().unwrap_or_default();
        let uwp = map.get("uwp").map(|s| s == "1").unwrap_or(false);

        let app_l = appname.to_ascii_lowercase();
        let kind = if app_l == "startmenu" || app_l.starts_with("startmenu") {
            "startmenu"
        } else if app_l.starts_with("trash") {
            "trash"
        } else {
            "app"
        };

        let match_exe = if kind == "app" {
            // strip trailing |
            let base = appname.trim_end_matches('|');
            if base.to_ascii_lowercase().ends_with(".exe") {
                base.to_ascii_lowercase()
            } else if !base.is_empty() {
                format!("{}.exe", base.to_ascii_lowercase())
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let launch_path = if !realpath.is_empty() {
            realpath.clone()
        } else {
            filepath.clone()
        };

        out.push(DockItem {
            id,
            kind: kind.into(),
            label: tag,
            match_exe,
            launch_path,
            real_path: realpath,
            virtual_path: virtualpath,
            icon_path: icopath,
            uwp,
            icon_png: None,
            icon_scale: 1.0,
            icon_offset_x: 0.0,
            icon_offset_y: 0.0,
            icon_bg: String::new(),
        });
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_separator_and_app() {
        let text = r#"
[ico1]
tag=开始菜单
appname=startmenu
[ico2]
delimiter=1
[ico3]
tag=Code
appname=code.exe
realpath=d:\app\code.exe
"#;
        let items = parse_dockico_text(text).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].kind, "startmenu");
        assert_eq!(items[1].kind, "separator");
        assert_eq!(items[2].match_exe, "code.exe");
        assert_eq!(items[2].launch_path, "d:\\app\\code.exe");
    }

    #[test]
    fn decodes_utf16_le_bom() {
        let text = "[ico1]\r\ntag=开始\r\nappname=startmenu\r\n";
        let mut bytes = vec![0xFF, 0xFE];
        for u in text.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let decoded = decode_ini_bytes(&bytes).unwrap();
        let items = parse_dockico_text(&decoded).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "startmenu");
        assert_eq!(items[0].label, "开始");
    }
}
