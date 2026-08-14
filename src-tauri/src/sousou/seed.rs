//! One-shot seed: match local Start Menu / Desktop shortcuts into tabs by name
//! keywords. Results are written into SQLite `prefs_sousou` (JSON) — never
//! hardcode machine paths here.

use super::apps;
use super::config::{self, SousouConfig, SousouShortcut, SousouTab};

/// (tab_id, name keywords to match against local shortcuts)
/// Games → `game` shelf; other categories follow screenshots loosely.
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
            "欧路词典",
            "Eudic",
            "Telegram",
            "微信输入法",
            "WeType",
            "MCHOSE",
            "小智桌面日历",
            "QQ",
            "紫鸟",
            "PotPlayer",
            "Motrix",
            "夸克",
            "DeskPins",
            "ImageGlass",
            "微信",
            "WeChat",
            "Weixin",
            "Microsoft Edge",
            "Clash Verge",
            "Clash",
            "Math Input",
            "数学输入",
            "抖音",
        ],
    ),
    (
        "game",
        &[
            "Steam",
            "SteamTools",
            "游戏加加",
            "GamePP",
            "BetterGI",
            "灾殃",
            "Scourge",
            "TheScourge",
            "GameSir",
            "GTA5",
            "GTAV",
            "Grand Theft Auto",
            "forzahorizon",
            "Forza Horizon",
            "PVZ",
            "植物大战僵尸",
            "WeGame",
            "米哈游",
            "miHoYo",
            "原神",
            "我的世界",
            "Minecraft",
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
            "TRAE",
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
            "花生壳",
            "milvus",
            "vectordb",
            "VectorDB",
            "Attu",
            "Axure",
            "cosbrowser",
            "DBX",
            "QtScrcpy",
            "宝塔",
            "Kafka-King",
            "Kafka",
            "若依",
            "New-API",
            "ApacheJMeter",
            "JMeter",
            "Kiro",
            "CursorLogin",
            "微信开发者",
            "抖音开发者",
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
            "UU远程",
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
            "EasyConnect",
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
            "ShExView",
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
            "快手",
            "美团",
            "饿了么",
        ],
    ),
];

/// Tab id → display name (used when creating a missing shelf).
const TAB_NAMES: &[(&str, &str, &str)] = &[
    ("home", "主页", "home"),
    ("apps", "应用", "apps"),
    ("game", "游戏", "game"),
    ("code", "编程", "code"),
    ("work", "工作", "work"),
    ("notes", "笔记", "notes"),
    ("xinwu", "信物社", "community"),
    ("shop", "电商", "shop"),
    ("tools", "小工具", "tools"),
    ("optimize", "优化", "optimize"),
];

const SEED_META: &str = "sousou_tabs_seeded_v3";

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
    ensure_seed_tabs(&mut cfg);

    let seeded_ids: std::collections::HashSet<&str> = SEED.iter().map(|(id, _)| *id).collect();
    if replace_seeded {
        for tab in &mut cfg.tabs {
            if seeded_ids.contains(tab.id.as_str()) || tab_alias_id(tab).is_some_and(|id| seeded_ids.contains(id))
            {
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
            || n.contains("website")
            || p.contains("installer")
            || p.contains("msedge_proxy")
            || p.contains("node_modules")
            || p.contains("\\test\\")
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
        let Some(tab) = find_tab_mut(&mut cfg.tabs, tab_id) else {
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
                if name_l.contains(&k) {
                    return true;
                }
                let file = std::path::Path::new(&target_l)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                // "steam" must not match Watt Toolkit / Steam++.exe
                if k == "steam" {
                    return file == "steam";
                }
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

    // Deduplicate cross-tab: prefer game shelf for game keywords, code for 开发者工具.
    dedupe_prefer(&mut cfg, "game", &["steam", "bettergi", "游戏加加", "gamespp", "games sir", "gta", "forza", "灾殃", "scourge", "pvz", "steamtools"]);
    dedupe_prefer(&mut cfg, "code", &["开发者工具", "hbuilder", "axure", "jmeter", "kafka", "natapp", "cpolar"]);
    dedupe_prefer(&mut cfg, "work", &["企业微信", "wxwork"]);
    dedupe_prefer(&mut cfg, "optimize", &["mem reduct", "wiztree", "图吧"]);

    matched_idxs.sort_unstable();
    matched_idxs.dedup();
    for i in matched_idxs {
        if let Some(app) = all.get_mut(i) {
            if app.icon_png.is_none() {
                app.icon_png = super::icon_cache::get_or_resolve(&app.target)
                    .or_else(|| super::icon_cache::get_or_resolve(&app.path));
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

fn ensure_seed_tabs(cfg: &mut SousouConfig) {
    let needed: std::collections::HashSet<&str> = SEED.iter().map(|(id, _)| *id).collect();
    for (id, name, icon) in TAB_NAMES {
        if !needed.contains(id) {
            continue;
        }
        if find_tab_mut(&mut cfg.tabs, id).is_some() {
            continue;
        }
        // Insert game after apps when possible.
        let insert_at = if *id == "game" {
            cfg.tabs
                .iter()
                .position(|t| t.id == "apps" || t.name == "应用")
                .map(|i| i + 1)
                .unwrap_or(cfg.tabs.len())
        } else {
            cfg.tabs.len()
        };
        cfg.tabs.insert(
            insert_at,
            SousouTab {
                id: (*id).into(),
                name: (*name).into(),
                icon: (*icon).into(),
                items: Vec::new(),
                folder_path: String::new(),
            },
        );
    }
}

fn tab_alias_id(tab: &SousouTab) -> Option<&'static str> {
    for (id, name, _) in TAB_NAMES {
        if tab.id == *id || tab.name == *name {
            return Some(*id);
        }
    }
    None
}

fn find_tab_mut<'a>(tabs: &'a mut [SousouTab], want: &str) -> Option<&'a mut SousouTab> {
    let want_name = TAB_NAMES
        .iter()
        .find(|(id, _, _)| *id == want)
        .map(|(_, name, _)| *name);
    tabs.iter_mut().find(|t| {
        t.id == want || want_name.is_some_and(|n| t.name == n) || tab_alias_id(t) == Some(want)
    })
}

/// Keep needle matches only on `prefer` tab; strip them from other shelves.
fn dedupe_prefer(cfg: &mut SousouConfig, prefer: &str, needles: &[&str]) {
    for tab in &mut cfg.tabs {
        let is_prefer = tab.id == prefer
            || tab_alias_id(tab) == Some(prefer)
            || TAB_NAMES
                .iter()
                .any(|(id, name, _)| *id == prefer && tab.name == *name);
        if is_prefer {
            continue;
        }
        tab.items.retain(|i| {
            let hay = format!("{} {}", i.name, i.path).to_ascii_lowercase();
            !needle_hit(&hay, needles)
        });
    }
}

fn needle_hit(hay: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| {
        let n = n.to_ascii_lowercase();
        // Watt Toolkit is Steam++ — not the Steam client.
        if n == "steam" {
            if hay.contains("steam++") || hay.contains("watt") {
                return false;
            }
            return hay.contains("steam.exe")
                || hay.contains("\\steam\\")
                || hay.contains("/steam/")
                || hay.contains(" steam");
        }
        hay.contains(&n)
    })
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
            for it in t.items.iter().take(12) {
                eprintln!("  - {} | {}", it.name, it.path);
            }
        }
    }
}
