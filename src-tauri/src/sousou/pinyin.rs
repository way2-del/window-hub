//! Lightweight Chinese → pinyin initials for app fuzzy match (jsq → 计算器).

use std::collections::HashMap;
use std::sync::OnceLock;

fn table() -> &'static HashMap<char, char> {
    static T: OnceLock<HashMap<char, char>> = OnceLock::new();
    T.get_or_init(|| {
        [
            ('计', 'j'),
            ('算', 's'),
            ('器', 'q'),
            ('记', 'j'),
            ('事', 's'),
            ('本', 'b'),
            ('画', 'h'),
            ('图', 't'),
            ('写', 'x'),
            ('字', 'z'),
            ('板', 'b'),
            ('微', 'w'),
            ('信', 'x'),
            ('网', 'w'),
            ('易', 'y'),
            ('云', 'y'),
            ('音', 'y'),
            ('乐', 'y'),
            ('浏', 'l'),
            ('览', 'l'),
            ('谷', 'g'),
            ('歌', 'g'),
            ('码', 'm'),
            ('猫', 'm'),
            ('钉', 'd'),
            ('飞', 'f'),
            ('书', 's'),
            ('桌', 'z'),
            ('面', 'm'),
            ('控', 'k'),
            ('制', 'z'),
            ('设', 's'),
            ('置', 'z'),
            ('资', 'z'),
            ('源', 'y'),
            ('管', 'g'),
            ('理', 'l'),
            ('应', 'y'),
            ('用', 'y'),
            ('程', 'c'),
            ('序', 'x'),
            ('编', 'b'),
            ('辑', 'j'),
            ('录', 'l'),
            ('屏', 'p'),
            ('转', 'z'),
            ('换', 'h'),
            ('文', 'w'),
            ('档', 'd'),
            ('件', 'j'),
            ('夹', 'j'),
            ('压', 'y'),
            ('缩', 's'),
            ('工', 'g'),
            ('具', 'j'),
            ('优', 'y'),
            ('化', 'h'),
            ('笔', 'b'),
            ('电', 'd'),
            ('脑', 'n'),
            ('我', 'w'),
            ('的', 'd'),
            ('系', 'x'),
            ('统', 't'),
            ('欧', 'o'),
            ('路', 'l'),
            ('词', 'c'),
            ('典', 'd'),
            ('聊', 'l'),
            ('天', 't'),
            ('游', 'y'),
            ('戏', 'x'),
            ('影', 'y'),
            ('视', 's'),
            ('播', 'b'),
            ('放', 'f'),
            ('下', 'x'),
            ('载', 'z'),
            ('输', 's'),
            ('入', 'r'),
            ('法', 'f'),
            ('邮', 'y'),
            ('箱', 'x'),
            ('办', 'b'),
            ('公', 'g'),
            ('软', 'r'),
            ('搜', 's'),
            ('索', 's'),
            ('快', 'k'),
            ('捷', 'j'),
            ('启', 'q'),
            ('动', 'd'),
            ('窗', 'c'),
            ('口', 'k'),
            ('组', 'z'),
            ('中', 'z'),
            ('站', 'z'),
            ('小', 'x'),
            ('智', 'z'),
            ('助', 'z'),
            ('手', 's'),
            ('机', 'j'),
            ('开', 'k'),
            ('发', 'f'),
            ('者', 'z'),
            ('包', 'b'),
            ('安', 'a'),
            ('装', 'z'),
            ('清', 'q'),
            ('卫', 'w'),
            ('士', 's'),
            ('杀', 's'),
            ('毒', 'd'),
            ('火', 'h'),
            ('狐', 'h'),
            ('狸', 'l'),
            ('边', 'b'),
            ('缘', 'y'),
            ('操', 'c'),
            ('作', 'z'),
            ('任', 'r'),
            ('务', 'w'),
            ('片', 'p'),
            ('频', 'p'),
            ('最', 'z'),
            ('近', 'j'),
            ('访', 'f'),
            ('问', 'w'),
            ('佳', 'j'),
            ('匹', 'p'),
            ('配', 'p'),
            ('主', 'z'),
            ('页', 'y'),
            ('物', 'w'),
            ('社', 's'),
            ('商', 's'),
            ('登', 'd'),
            ('添', 't'),
            ('加', 'j'),
            ('自', 'z'),
            ('定', 'd'),
            ('义', 'y'),
            ('选', 'x'),
            ('择', 'z'),
            ('址', 'z'),
            ('更', 'g'),
            ('新', 'x'),
        ]
        .into_iter()
        .collect()
    })
}

pub fn initials(s: &str) -> String {
    let t = table();
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if let Some(&py) = t.get(&ch) {
            out.push(py);
        } else if let Some(py) = cjk_initial_approx(ch) {
            out.push(py);
        }
    }
    out
}

pub fn matches_query(name: &str, query: &str) -> bool {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return true;
    }
    if name.to_ascii_lowercase().contains(&q) {
        return true;
    }
    let ini = initials(name);
    if ini.contains(&q) {
        return true;
    }
    let compact: String = q.chars().filter(|c| !c.is_whitespace()).collect();
    ini.contains(&compact)
}

fn cjk_initial_approx(ch: char) -> Option<char> {
    let u = ch as u32;
    if !(0x4E00..=0x9FFF).contains(&u) {
        return None;
    }
    let idx = ((u - 0x4E00) % 26) as u8;
    Some((b'a' + idx) as char)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsq_matches_calculator() {
        assert!(matches_query("计算器", "jsq"));
        assert!(matches_query("记事本", "jsb"));
    }
}
