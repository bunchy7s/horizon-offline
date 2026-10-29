//! The resource pack screen for tools/test_pack_screen_parity.py: reads one
//! JSON case per line (packs, selection, GUI size, actions, mouse), runs
//! `horizon_client::pack_screen` through the actions and writes the shown
//! screen's GUI calls in the oracle's JSON form.
//!
//! usage: pack_screen_fixture RESOURCES CASES OUTPUT
use horizon_client::{
    hud::{HudAtlas, PACK_ICON_SIZE},
    lang,
    pack_screen::{Compat, PackInfo, PackScreen, Source},
    packs::{self, Position},
    screen_draw::{Draw, Measure},
    text::Text,
};
use serde_json::{Value, json};
use std::{fmt::Write as _, fs, path::PathBuf, sync::Arc};

/// Styled runs: [text, rgb or null, bold, italic, underlined, strikethrough, obfuscated].
pub fn runs(t: &Text) -> Value {
    let mut out: Vec<(String, Option<u32>, [bool; 5])> = Vec::new();
    for (c, s) in t.chars() {
        let f = [
            s.bold,
            s.italic,
            s.underlined,
            s.strikethrough,
            s.obfuscated,
        ]
        .map(|b| b == Some(true));
        match out.last_mut() {
            Some(last) if last.1 == s.color && last.2 == f => last.0.push(c),
            _ => out.push((c.to_string(), s.color, f)),
        }
    }
    Value::Array(
        out.into_iter()
            .map(|(t, c, f)| json!([t, c, f[0], f[1], f[2], f[3], f[4]]))
            .collect(),
    )
}
pub fn encode(draws: &[Draw]) -> Value {
    Value::Array(
        draws
            .iter()
            .map(|d| match d {
                Draw::Sprite { sprite, x, y, w, h, color } => {
                    json!({"type": "sprite", "sprite": sprite, "x": x, "y": y, "w": w, "h": h, "color": color})
                }
                Draw::Blit { texture, x, y, w, h, u, v, src, tex, color } => json!({"type": "blit", "texture": texture,
                    "x": x, "y": y, "w": w, "h": h, "u": u, "v": v, "srcW": src[0], "srcH": src[1], "texW": tex[0],
                    "texH": tex[1], "color": color}),
                Draw::Fill { x0, y0, x1, y1, color } => {
                    json!({"type": "fill", "x0": x0, "y0": y0, "x1": x1, "y1": y1, "color": color})
                }
                Draw::Highlight { x0, y0, x1, y1 } => json!({"type": "highlight", "x0": x0, "y0": y0, "x1": x1, "y1": y1}),
                Draw::Text { text, x, y, color, shadow } => {
                    json!({"type": "text", "runs": runs(text), "x": x, "y": y, "color": color, "shadow": shadow})
                }
                Draw::Scissor { x0, y0, x1, y1 } => json!({"type": "scissor", "x0": x0, "y0": y0, "x1": x1, "y1": y1}),
                Draw::NoScissor => json!({"type": "noscissor"}),
                Draw::Tooltip { lines, x, y } => {
                    json!({"type": "tooltip", "lines": lines.iter().map(runs).collect::<Vec<_>>(), "x": x, "y": y})
                }
            })
            .collect(),
    )
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<PathBuf> = crate::args_os().skip(1).map(PathBuf::from).collect();
    let [root, inputs, output] = args.as_slice() else {
        return Err("usage: pack_screen_fixture RESOURCES CASES OUTPUT".into());
    };
    let atlas = HudAtlas::build(root)?;
    lang::set(lang::Language::load(&[root], lang::DEFAULT));
    let m = Measure(&atlas.font);
    let mut out = String::new();
    for line in fs::read_to_string(inputs)?
        .lines()
        .filter(|l| !l.is_empty())
    {
        let c: Value = serde_json::from_str(line)?;
        let int = |v: &Value| v.as_i64().unwrap_or(0) as i32;
        let packs: Vec<PackInfo> = c["packs"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| {
                let (description, description_style) = packs::component_root(&p["description"]);
                PackInfo {
                    id: p["id"].as_str().unwrap_or_default().to_owned(),
                    title: packs::component_root(&p["title"]).0,
                    description,
                    description_style,
                    source: match p["source"].as_str() {
                        Some("builtin") => Source::BuiltIn,
                        Some("world") => Source::World,
                        Some("server") => Source::Server,
                        Some("feature") => Source::Feature,
                        _ => Source::Default,
                    },
                    compat: match p["compat"].as_str() {
                        Some("old") => Compat::TooOld,
                        Some("new") => Compat::TooNew,
                        Some("unknown") => Compat::Unknown,
                        _ => Compat::Compatible,
                    },
                    icon: p["icon"].as_bool().unwrap_or(false).then(|| {
                        Arc::from(vec![0; (PACK_ICON_SIZE * PACK_ICON_SIZE * 4) as usize])
                    }),
                    required: p["required"].as_bool().unwrap_or(false),
                    position: if p["position"] == "top" {
                        Position::Top
                    } else {
                        Position::Bottom
                    },
                    fixed: p["fixed"].as_bool().unwrap_or(false),
                }
            })
            .collect();
        let selected: Vec<String> = c["selected"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect();
        let mut screen = PackScreen::new(
            &m,
            Text::translate(c["title"].as_str().unwrap_or("resourcePack.title"), &[]),
            int(&c["width"]),
            int(&c["height"]),
            c["inWorld"] == true,
            packs,
            &selected,
        );
        for a in c["actions"].as_array().into_iter().flatten() {
            let f = |k: &str| a[k].as_f64().unwrap_or(0.0);
            match a["type"].as_str().unwrap_or_default() {
                "click" => {
                    screen.click(&m, f("x"), f("y"));
                }
                "scroll" => screen.scroll(f("x"), f("y"), f("amount")),
                "type" => screen.type_text(&m, a["text"].as_str().unwrap_or_default()),
                "erase" => screen.erase(&m, f("count") as isize),
                _ => {}
            }
            if std::env::var_os("PACK_DEBUG").is_some() {
                eprintln!("{a} -> {}", screen.debug());
            }
        }
        let mouse = (int(&c["mouse"][0]), int(&c["mouse"][1]));
        let draws = screen.draw(&m, mouse, c["blink"].as_u64().unwrap_or(0));
        let screen_name = if screen.confirming() {
            "confirm"
        } else {
            "packs"
        };
        writeln!(
            out,
            "{}",
            json!({"screen": screen_name, "draws": encode(&draws)})
        )?;
    }
    fs::write(output, out)?;
    Ok(())
}
