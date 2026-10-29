//! A text display's text for differential tests against vanilla's
//! `TextDisplayRenderer.submitInner` (tools/test_text_display_render_parity.py):
//! decodes each case's entity data from vanilla's wire bytes, lays the text out
//! with `horizon_client::text_display` and prints the pose and every quad in
//! text units [x, y, z, ARGB], as VanillaTextDisplayRenderOracle does.
//!
//! usage: text_display_render_fixture RESOURCES INPUTS OUTPUT
use horizon_client::{
    hud::{HudAtlas, HudBatch},
    hud_layout::background_color,
    text_display::{self, Lines, TextDisplayData},
};
use horizon_protocol::{ids, packets::play::Clientbound};
use serde_json::{Value, json};
use std::{error::Error, io::Write};

pub fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = crate::args().collect();
    let [_, resources, inputs, output] = args.as_slice() else {
        return Err("usage: text_display_render_fixture RESOURCES INPUTS OUTPUT".into());
    };
    let atlas = HudAtlas::build(std::path::Path::new(resources))?;
    let mut out = std::io::BufWriter::new(std::fs::File::create(output)?);
    for line in std::fs::read_to_string(inputs)?.lines() {
        let input: Value = serde_json::from_str(line)?;
        let hex = input["wire"].as_str().unwrap_or_default();
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
            .collect::<Result<_, _>>()?;
        let Clientbound::SetEntityData(packet) =
            Clientbound::decode(ids::play::clientbound::SET_ENTITY_DATA, &bytes)?
        else {
            return Err("not entity data".into());
        };
        let mut d = TextDisplayData::default();
        d.apply(&packet.values);
        let lines = Lines::split(&atlas.font, &d.text, d.line_width);
        let mut batch = HudBatch::new([1, 1]);
        let default_background = background_color(
            input["chat_only"].as_bool().unwrap_or(false),
            input["background_opacity"].as_f64().unwrap_or(0.0),
            0.25,
        );
        text_display::draw(&mut batch, &atlas, &d, &lines, default_background);
        let quads: Vec<Value> = batch
            .vertices
            .as_chunks::<6>()
            .0
            .iter()
            .map(|q| {
                let corners: Vec<Value> = [0, 1, 2, 5]
                    .map(|i| {
                        let v = q[i];
                        let [r, g, b, a] = v.color.to_le_bytes().map(u32::from);
                        let argb = (a << 24 | r << 16 | g << 8 | b) as i32;
                        json!([v.pos[0], v.pos[1], v.depth, argb])
                    })
                    .into();
                corners.into()
            })
            .collect();
        writeln!(
            out,
            "{}",
            json!({"pose": lines.pose().to_cols_array(), "quads": quads})
        )?;
    }
    Ok(())
}
