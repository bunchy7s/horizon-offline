//! Inline atlas glyph geometry for `tools/test_inline_object_parity.py`.
use horizon_client::{
    component,
    hud::{HudAtlas, HudBatch, Rect},
    text::Text,
};
use serde_json::Value;
use std::{error::Error, fs, io::Write};

pub fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = crate::args().collect();
    let mut atlas = HudAtlas::boot();
    atlas.sprites.insert(
        "item/test".into(),
        Rect {
            u0: 0.125,
            v0: 0.25,
            u1: 0.375,
            v1: 0.5,
            w: 16,
            h: 16,
        },
    );
    let mut out = fs::File::create(&args[2])?;
    for line in fs::read_to_string(&args[1])?.lines() {
        let row: Value = serde_json::from_str(line)?;
        let mut input = serde_json::json!({"object":"atlas", "sprite":"minecraft:item/test"});
        for key in ["shadow_color", "color", "bold", "italic"] {
            if let Some(value) = row.get(key) {
                input[key] = value.clone();
            }
        }
        let text = component::to_text(&input, &mut |_| Text::empty());
        let mut batch = HudBatch::new([320, 240]);
        let (x, y) = (
            row["x"].as_f64().unwrap() as f32,
            row["y"].as_f64().unwrap() as f32,
        );
        let argb = row["base"].as_u64().unwrap() as u32;
        let color = horizon_client::hud::rgba(
            (argb >> 16) as u8,
            (argb >> 8) as u8,
            argb as u8,
            (argb >> 24) as u8,
        );
        let shadow = row["shadow"].as_bool().unwrap();
        if row["world"].as_bool().unwrap() {
            batch.text_styled_world(&atlas, &text, x, y, color, shadow);
        } else {
            batch.text_styled(&atlas, &text, x, y, color, shadow);
        }
        writeln!(out, "{}", batch.vertices.len() / 6)?;
        for quad in batch.vertices.as_chunks::<6>().0 {
            let v = quad[0];
            let [r, g, b, a] = v.color.to_le_bytes().map(u32::from);
            writeln!(
                out,
                "{:.3} {:.3} {:.3} {:.3} {:.3} {:08x}",
                v.pos[0],
                v.pos[1],
                quad[2].pos[0],
                quad[2].pos[1],
                v.depth,
                a << 24 | r << 16 | g << 8 | b
            )?;
        }
    }
    Ok(())
}
