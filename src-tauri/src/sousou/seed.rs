//! One-shot seed: match local Start Menu apps into tabs (from 小智搜搜 screenshots).
//! Results are written into SQLite `prefs_sousou` — not hardcoded UI lists.

use super::apps;
use super::config::{self, SousouConfig, SousouShortcut};

/// (tab_id, name keywords to match against local shortcuts)
/// Keywords come from user screenshots (应用 / 编程 / 工作 / 优化 …).
const SEED: &[(&str, &[&str])] = &[
    (
        "apps",
        &[
            "WPS",
            "Watt",
            "OBS Studio",
            "OBS",
            "EchoMusic",
            "网易云",
            "CloudMusic",
            "vivo办公",
            "HECATE",
            "游戏加加",
            "BetterGI",
            "欧路词典",
            "Eudic",
            "灾殃",
            "Telegram",
            "GameSir",
            "GTA5",
            "forzahorizon",
            "SteamTools",
            "Dock_64",
            "Steam",
            "微信输入法",
            "MCHOSE",
            "小智桌面日历",
            "QQ",
            "紫鸟",
            "PotPlayer",
            "Motrix",
            "Mem Reduct",
            "夸克",
            "DeskPins",
            "PVZ",
            "抖音",
            "ImageGlass",
            "微信",
            "WeChat",
            "Weixin",
            "Microsoft Edge",
            "Clash Verge",
            "Clash",
            "Math Input",
            "数学输入",
        ],
    ),
    (
        "code",
        &[
            "GoLand",
            "DataGrip",
            "IntelliJ",
            "IDEA",
            "WebStorm",
            "PyCharm",
            "Visual Studio Code",
            "VS Code",
            "VSCode",
            "Cursor",
            "Trae",
            "HBuilder",
            "Typora",
            "Navicat",
            "Redis Desktop",
            "Another Redis",
            "Apifox",
            "DockerPull",
            "Docker Desktop",
            "Git Bash",
            "phpstudy",
            "VMware",
            "Fiddler",
            "MobaXterm",
            "MQTTX",
            "Ollama",
            "Wireshark",
            "ProxyPin",
            "natapp",
            "cpolar",
            "milvus",
            "vectordb",
            "Postman",
            "CLion",
            "Rider",
            "Android Studio",
            "Windows Terminal",
            "GitHub Desktop",
            "code.exe",
        ],
    ),
    (
        "work",
        &[
            "豆包",
            "Doubao",
            "Kimi",
            "Cherry Studio",
            "Cherry",
            "企业微信",
            "WXWork",
            "WeCom",
            "MuMu",
            "GameViewer",
            "向日葵",
            "Sunlogin",
            "ToDesk",
            "BOSS直聘",
            "BOSS",
            "Acrobat",
            "Photoshop",
            "Illustrator",
            "RealVNC",
            "剪映",
            "CapCut",
            "飞书",
            "Lark",
            "Feishu",
            "draw.io",
            "drawio",
            "XMind",
            "Xmind",
            "绘世",
            "blender",
            "有道翻译",
            "Youdao",
            "泉州师院",
        ],
    ),
    (
        "notes",
        &[
            "OneNote",
            "印象笔记",
            "Evernote",
            "Obsidian",
            "Notion",
            "为知笔记",
            "有道云笔记",
            "思源",
            "Logseq",
            "便签",
            "Typora",
        ],
    ),
    (
        "tools",
        &[
            "7-Zip",
            "Bandizip",
            "WinRAR",
            "Notepad++",
            "ShareX",
            "Snipaste",
            "Listary",
            "FastStone",
            "格式工厂",
            "录屏",
            "Everything",
        ],
    ),
    (
        "optimize",
        &[
            "Standalone",
            "CCleaner",
            "LiteMonitor",
            "ContextMenu",
            "Dism++",
            "Dism",
            "geek",
            "Geek",
            "Windows Update",
            "MiniRename",
            "MiniRenamer",
            "OpenArk",
            "WizTree",
            "鼠标点击",
            "图吧",
            "鲁大师",
            "PC-LuDaShi",
            "Ludashi",
            "Mem Reduct",
            "SpaceSniffer",
            "TreeSize",
            "Glary",
            "IObit",
            "Unlocker",
        ],
    ),
    (
        "shop",
        &[
            "淘宝",
            "京东",
            "拼多多",
            "闲鱼",
            "抖音",
            "快手",
            "美团",
            "饿了么",
        ],
    ),
];

const SEED_META: &str = "sousou_tabs_seeded_v2";

pub fn seed_tabs_if_needed() -> SousouConfig {
    let mut cfg = config::load();
    let already = crate::db::with_conn(|c| crate::db::meta_get(c, SEED_META))
        .ok()
        .flatten()
        .is_some();
    if already {
        return cfg;
    }
    cfg = seed_into(cfg, false);
    let _ = config::save(&cfg);
    let _ = crate::db::with_conn(|c| {
        crate::db::meta_set(c, SEED_META, &serde_json::json!(true))
    });
    cfg
}

/// `replace_seeded`: if true, clear items on seeded tabs before filling (screenshot refresh).
pub fn seed_into(mut cfg: SousouConfig, replace_seeded: bool) -> SousouConfig {
    let seeded_ids: std::collections::HashSet<&str> = SEED.iter().map(|(id, _)| *id).collect();
    if replace_seeded {
        for tab in &mut cfg.tabs {
            if seeded_ids.contains(tab.id.as_str()) {
                tab.items.clear();
            }
        }
    }

    let mut all = apps::list_apps(false, 0);
    // Drop docs / help / uninstallers / plain text noise.
    all.retain(|a| {
        let p = a.target.to_ascii_lowercase();
        let n = a.name.to_ascii_lowercase();
        if n.contains("卸载")
            || n.contains("uninstall")
            || n.contains("help")
            || n.contains("installer")
            || p.contains("installer")
            || p.contains("msedge_proxy")
        {
            return false;
        }
        if p.ends_with(".txt")
            || p.ends_with(".chm")
            || p.ends_with(".html")
            || p.ends_with(".htm")
            || p.ends_with(".pdf")
            || p.ends_with(".url")
        {
            return false;
        }
        true
    });

    let mut matched_idxs: Vec<usize> = Vec::new();

    for (tab_id, keys) in SEED {
        let Some(tab) = cfg.tabs.iter_mut().find(|t| t.id == *tab_id) else {
            continue;
        };
        let mut exist: std::collections::HashSet<String> = tab
            .items
            .iter()
            .map(|i| i.path.to_ascii_lowercase())
            .collect();
        for (i, app) in all.iter().enumerate() {
            let name_l = app.name.to_ascii_lowercase();
            let target_l = app.target.to_ascii_lowercase();
            let hit = keys.iter().any(|k| {
                let k = k.to_ascii_lowercase();
                // Short keys (< 4) only match display name to avoid path false positives.
                if k.len() < 4 {
                    return name_l.contains(&k);
                }
                // Prefer name match; allow target filename match for exe stems.
                if name_l.contains(&k) {
                    return true;
                }
                let file = std::path::Path::new(&target_l)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                file.contains(&k)
            });
            if !hit {
                continue;
            }
            let path = if !app.target.is_empty() {
                app.target.clone()
            } else {
                app.path.clone()
            };
            let key = path.to_ascii_lowercase();
            if !exist.insert(key) {
                continue;
            }
            matched_idxs.push(i);
            tab.items.push(SousouShortcut {
                id: app.id.clone(),
                name: app.name.clone(),
                path,
                kind: "app".into(),
                icon_png: None,
            });
        }
    }

    matched_idxs.sort_unstable();
    matched_idxs.dedup();
    for i in matched_idxs {
        if let Some(app) = all.get_mut(i) {
            if app.icon_png.is_none() {
                app.icon_png = crate::dock::resolve_launcher_icon_png(&app.target)
                    .or_else(|| crate::dock::resolve_launcher_icon_png(&app.path));
            }
        }
    }
    for tab in &mut cfg.tabs {
        for item in &mut tab.items {
            if item.icon_png.is_some() {
                continue;
            }
            if let Some(app) = all.iter().find(|a| {
                a.target.eq_ignore_ascii_case(&item.path) || a.path.eq_ignore_ascii_case(&item.path)
            }) {
                item.icon_png = app.icon_png.clone();
            }
        }
    }
    cfg
}

/// Force re-seed from screenshots into DB (replace seeded tab items).
pub fn reseeds_merge() -> Result<SousouConfig, String> {
    let cfg = seed_into(config::load(), true);
    config::save(&cfg)?;
    let _ = crate::db::with_conn(|c| {
        crate::db::meta_set(c, SEED_META, &serde_json::json!(true))
    });
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_into_user_db() {
        crate::db::init().expect("db init");
        let cfg = reseeds_merge().expect("seed");
        for t in &cfg.tabs {
            eprintln!("tab {} ({}) => {} items", t.id, t.name, t.items.len());
            for it in t.items.iter().take(8) {
                eprintln!("  - {} | {}", it.name, it.path);
            }
        }
    }
}
