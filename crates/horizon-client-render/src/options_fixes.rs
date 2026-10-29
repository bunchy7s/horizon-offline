//! The `OPTIONS` data fixers of 26.3 (`DataFixTypes.OPTIONS`), applied to the
//! raw key/value strings of an `options.txt` older than the current version.
use std::collections::HashMap;

/// A fixer threw an exception; vanilla abandons loading the file.
#[derive(Debug)]
pub struct FixFailed;

pub type Raw = HashMap<String, String>;

/// OptionsKeyLwjgl3Fix.MAP: GLFW 2 key codes to key names.
const LWJGL3: &[(i32, &str)] = &[
    (0, "key.unknown"),
    (11, "key.0"),
    (2, "key.1"),
    (3, "key.2"),
    (4, "key.3"),
    (5, "key.4"),
    (6, "key.5"),
    (7, "key.6"),
    (8, "key.7"),
    (9, "key.8"),
    (10, "key.9"),
    (30, "key.a"),
    (40, "key.apostrophe"),
    (48, "key.b"),
    (43, "key.backslash"),
    (14, "key.backspace"),
    (46, "key.c"),
    (58, "key.caps.lock"),
    (51, "key.comma"),
    (32, "key.d"),
    (211, "key.delete"),
    (208, "key.down"),
    (18, "key.e"),
    (207, "key.end"),
    (28, "key.enter"),
    (13, "key.equal"),
    (1, "key.escape"),
    (33, "key.f"),
    (59, "key.f1"),
    (68, "key.f10"),
    (87, "key.f11"),
    (88, "key.f12"),
    (100, "key.f13"),
    (101, "key.f14"),
    (102, "key.f15"),
    (103, "key.f16"),
    (104, "key.f17"),
    (105, "key.f18"),
    (113, "key.f19"),
    (60, "key.f2"),
    (61, "key.f3"),
    (62, "key.f4"),
    (63, "key.f5"),
    (64, "key.f6"),
    (65, "key.f7"),
    (66, "key.f8"),
    (67, "key.f9"),
    (34, "key.g"),
    (41, "key.grave.accent"),
    (35, "key.h"),
    (199, "key.home"),
    (23, "key.i"),
    (210, "key.insert"),
    (36, "key.j"),
    (37, "key.k"),
    (82, "key.keypad.0"),
    (79, "key.keypad.1"),
    (80, "key.keypad.2"),
    (81, "key.keypad.3"),
    (75, "key.keypad.4"),
    (76, "key.keypad.5"),
    (77, "key.keypad.6"),
    (71, "key.keypad.7"),
    (72, "key.keypad.8"),
    (73, "key.keypad.9"),
    (78, "key.keypad.add"),
    (83, "key.keypad.decimal"),
    (181, "key.keypad.divide"),
    (156, "key.keypad.enter"),
    (141, "key.keypad.equal"),
    (55, "key.keypad.multiply"),
    (74, "key.keypad.subtract"),
    (38, "key.l"),
    (203, "key.left"),
    (56, "key.left.alt"),
    (26, "key.left.bracket"),
    (29, "key.left.control"),
    (42, "key.left.shift"),
    (219, "key.left.win"),
    (50, "key.m"),
    (12, "key.minus"),
    (49, "key.n"),
    (69, "key.num.lock"),
    (24, "key.o"),
    (25, "key.p"),
    (209, "key.page.down"),
    (201, "key.page.up"),
    (197, "key.pause"),
    (52, "key.period"),
    (183, "key.print.screen"),
    (16, "key.q"),
    (19, "key.r"),
    (205, "key.right"),
    (184, "key.right.alt"),
    (27, "key.right.bracket"),
    (157, "key.right.control"),
    (54, "key.right.shift"),
    (220, "key.right.win"),
    (31, "key.s"),
    (70, "key.scroll.lock"),
    (39, "key.semicolon"),
    (53, "key.slash"),
    (57, "key.space"),
    (20, "key.t"),
    (15, "key.tab"),
    (22, "key.u"),
    (200, "key.up"),
    (47, "key.v"),
    (17, "key.w"),
    (45, "key.x"),
    (21, "key.y"),
    (44, "key.z"),
];

/// `Integer.parseInt(String)`.
pub fn java_parse_int(text: &str) -> Option<i32> {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    text.parse::<i32>().ok()
}

enum Number<'a> {
    Special(f64),
    Decimal(bool, &'a str),
}

fn java_number(text: &str) -> Option<Number<'_>> {
    let t = text.trim_matches(|c: char| c <= ' ');
    let (negative, body) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    match body {
        "NaN" => return Some(Number::Special(f64::NAN)),
        "Infinity" => {
            return Some(Number::Special(if negative {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }));
        }
        _ => {}
    }
    let body = body.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(body);
    if body.is_empty()
        || !body
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'))
        || !body.chars().any(|c| c.is_ascii_digit())
    {
        return None;
    }
    Some(Number::Decimal(negative, body))
}

/// `Double.parseDouble(String)` for decimal and special forms.
pub fn java_parse_double(text: &str) -> Option<f64> {
    match java_number(text)? {
        Number::Special(v) => Some(v),
        Number::Decimal(negative, body) => body
            .parse::<f64>()
            .ok()
            .map(|v| if negative { -v } else { v }),
    }
}

/// `Float.parseFloat(String)`.
pub fn java_parse_float(text: &str) -> Option<f32> {
    match java_number(text)? {
        Number::Special(v) => Some(v as f32),
        Number::Decimal(negative, body) => body
            .parse::<f32>()
            .ok()
            .map(|v| if negative { -v } else { v }),
    }
}

fn rename(raw: &mut Raw, from: &str, to: &str, fix: impl FnOnce(&str) -> String) {
    if let Some(old) = raw.remove(from) {
        let value = fix(&old);
        raw.insert(to.to_owned(), value);
    }
}

/// `OPTIONS` fixers from `version` up to the current data version.
pub fn update(raw: &mut Raw, version: i32) -> Result<(), FixFailed> {
    if version < 816
        && let Some(lang) = raw.get_mut("lang")
    {
        *lang = lang.to_lowercase();
    }
    if version < 1344 {
        let keys: Vec<String> = raw
            .keys()
            .filter(|k| k.starts_with("key_"))
            .cloned()
            .collect();
        for key in keys {
            let old = java_parse_int(&raw[&key]).ok_or(FixFailed)?;
            let name = if old < 0 {
                match old + 100 {
                    0 => "key.mouse.left".to_owned(),
                    1 => "key.mouse.right".to_owned(),
                    2 => "key.mouse.middle".to_owned(),
                    button => format!("key.mouse.{}", button + 1),
                }
            } else {
                LWJGL3
                    .iter()
                    .find(|(v, _)| *v == old)
                    .map_or("key.unknown", |(_, n)| n)
                    .to_owned()
            };
            raw.insert(key, name);
        }
    }
    if version < 1446 {
        let keys: Vec<String> = raw
            .keys()
            .filter(|k| k.starts_with("key_"))
            .cloned()
            .collect();
        for key in keys {
            let old = raw[&key].clone();
            if !old.starts_with("key.mouse") && !old.starts_with("scancode.") {
                // substring(4) on a shorter value throws.
                let rest: String = old.chars().skip(4).collect();
                if old.chars().count() < 4 {
                    return Err(FixFailed);
                }
                raw.insert(key, format!("key.keyboard.{rest}"));
            }
        }
    }
    if version < 1936
        && let Some(opacity) = raw.get("chatOpacity")
    {
        let background = java_parse_double(opacity).map_or(0.5, |v| (0.9 * v + 0.1) / 2.0);
        raw.insert(
            "textBackgroundOpacity".into(),
            crate::java_text::double_to_string(background),
        );
    }
    if version < 2556 {
        rename(raw, "fancyGraphics", "graphicsMode", |v| {
            if v == "true" { "1" } else { "0" }.to_owned()
        });
    }
    if version < 2558
        && let Some(v) = raw.remove("key_key.swapHands")
    {
        raw.insert("key_key.swapOffhand".into(), v);
    }
    if version < 3201 {
        for key in ["resourcePacks", "incompatibleResourcePacks"] {
            if let Some(v) = raw.get_mut(key) {
                *v = v.replace("\"programer_art\"", "\"programmer_art\"");
            }
        }
    }
    if version < 3214
        && let Some(ao) = raw.get_mut("ao")
    {
        match ao.as_str() {
            "0" => *ao = "false".into(),
            "1" | "2" => *ao = "true".into(),
            _ => {}
        }
    }
    if version < 3319 {
        raw.insert("onboardAccessibility".into(), "false".into());
    }
    if version < 3943
        && let Some(v) = raw.get_mut("menuBackgroundBlurriness")
    {
        let value = java_parse_float(v).map_or(5, |d| ((d * 10.0) + 0.5).floor() as i32);
        *v = value.to_string();
    }
    if version < 4651 {
        let mode = raw.get("graphicsMode").cloned();
        for (name, fast, fancy, fabulous) in [
            ("cutoutLeaves", "false", "true", "true"),
            ("weatherRadius", "5", "10", "10"),
            ("vignette", "false", "true", "true"),
            ("improvedTransparency", "false", "false", "true"),
        ] {
            let value = match mode.as_deref() {
                Some("2") => fabulous,
                Some("0") => fast,
                _ => fancy,
            };
            raw.insert(name.into(), value.into());
        }
        raw.insert("graphicsPreset".into(), "custom".into());
    }
    if version < 4661 {
        rename(raw, "showNowPlayingToast", "musicToast", |v| {
            if v == "false" {
                "never"
            } else {
                "pause_and_toast"
            }
            .to_owned()
        });
    }
    if version < 4892 {
        raw.insert("preferredGraphicsBackend".into(), "default".into());
    }
    Ok(())
}
