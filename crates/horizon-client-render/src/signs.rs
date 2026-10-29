//! Sign block entities (`SignBlockEntity`, `SignText`): two four-line text
//! sides with a dye colour and glow, saved in vanilla's `front_text` /
//! `back_text` NBT layout.
pub use crate::chunk::block_entity::{SignData, SignText};
use horizon_world::nbt::Tag;

/// `DyeColor` names and `getTextColor` values by id.
pub const DYE_TEXT: [(&str, u32); 16] = [
    ("white", 0xFFFFFF),
    ("orange", 0xFF681F),
    ("magenta", 0xFF00FF),
    ("light_blue", 0x9AC0CD),
    ("yellow", 0xFFFF00),
    ("lime", 0xBFFF00),
    ("pink", 0xFF69B4),
    ("gray", 0x808080),
    ("light_gray", 0xD3D3D3),
    ("cyan", 0x00FFFF),
    ("purple", 0xA020F0),
    ("blue", 0x0000FF),
    ("brown", 0x8B4513),
    ("green", 0x00FF00),
    ("red", 0xFF0000),
    ("black", 0x000000),
];
const BLACK: u8 = 15;

impl Default for SignText {
    fn default() -> Self {
        Self {
            lines: Default::default(),
            color: BLACK,
            glowing: false,
        }
    }
}
impl SignText {
    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(String::is_empty)
    }
    /// `DyeColor.getTextColor` of this side.
    pub fn text_color(&self) -> u32 {
        DYE_TEXT[usize::from(self.color.min(15))].1
    }
    /// `AbstractSignRenderer.getDarkColor`.
    pub fn dark_color(&self) -> u32 {
        let c = self.text_color();
        if c == 0 && self.glowing {
            0xF0EBCC
        } else {
            let s = |shift: u32| ((((c >> shift) & 255) as f32 * 0.4) as u32) << shift;
            s(16) | s(8) | s(0)
        }
    }
}

impl SignData {
    pub fn id(&self) -> &'static str {
        if self.hanging {
            "minecraft:hanging_sign"
        } else {
            "minecraft:sign"
        }
    }
    /// `SignBlockEntity.getMaxTextLineWidth`.
    pub fn max_line_width(&self) -> f32 {
        if self.hanging { 60.0 } else { 90.0 }
    }
    /// `SignBlockEntity.getTextLineHeight`.
    pub fn line_height(&self) -> f32 {
        if self.hanging { 9.0 } else { 10.0 }
    }
    /// NBT entries after the common block-entity header.
    pub fn encode(&self) -> Vec<(&'static str, Tag)> {
        let side = |t: &SignText| {
            Tag::compound([
                (
                    "messages",
                    Tag::List(
                        t.lines
                            .iter()
                            .map(|l| Tag::String(l.as_str().into()))
                            .collect(),
                    ),
                ),
                (
                    "color",
                    Tag::String(DYE_TEXT[usize::from(t.color.min(15))].0.into()),
                ),
                ("has_glowing_text", Tag::Byte(i8::from(t.glowing))),
            ])
        };
        vec![
            ("front_text", side(&self.front)),
            ("back_text", side(&self.back)),
            ("is_waxed", Tag::Byte(i8::from(self.waxed))),
        ]
    }
    pub fn decode(&mut self, tag: &Tag) {
        let side = |t: Option<&Tag>| -> SignText {
            let mut out = SignText::default();
            let Some(t) = t else { return out };
            if let Some(Tag::List(messages)) = t.get("messages") {
                for (line, m) in out.lines.iter_mut().zip(messages) {
                    *line = component_text(m);
                }
            }
            if let Some(Tag::String(c)) = t.get("color") {
                let name = c.to_string_lossy();
                if let Some(i) = DYE_TEXT.iter().position(|(n, _)| *n == name) {
                    out.color = i as u8;
                }
            }
            out.glowing = matches!(t.get("has_glowing_text"), Some(Tag::Byte(b)) if *b != 0);
            out
        };
        self.front = side(tag.get("front_text"));
        self.back = side(tag.get("back_text"));
        self.waxed = matches!(tag.get("is_waxed"), Some(Tag::Byte(b)) if *b != 0);
    }
}
/// Plain text of a stored text component (string, `{text: ...}` or JSON string).
fn component_text(tag: &Tag) -> String {
    match tag {
        Tag::String(s) => {
            let s = s.to_string_lossy();
            match serde_json::from_str::<serde_json::Value>(&s) {
                Ok(serde_json::Value::String(text)) => text,
                Ok(v) if v.get("text").is_some() => {
                    v["text"].as_str().unwrap_or_default().to_owned()
                }
                _ => s,
            }
        }
        // Component.getString: translations and key binds resolved.
        Tag::Compound(_) => {
            crate::scoreboard::comp_text(&crate::scoreboard::tag_to_json(tag)).plain()
        }
        _ => String::new(),
    }
}
/// Whether a block name is a sign (standing, wall, hanging or wall hanging).
pub fn sign_kind(name: &str) -> Option<bool> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    if name == "hanging_sign" || name.ends_with("_hanging_sign") {
        Some(true)
    } else if name == "sign" || name.ends_with("_sign") {
        Some(false)
    } else {
        None
    }
}
/// Wood type of a sign block (`SignBlock.getWoodType`), for the editor texture.
pub fn wood_type(name: &str) -> &str {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    for suffix in ["_wall_hanging_sign", "_hanging_sign", "_wall_sign", "_sign"] {
        if let Some(wood) = name.strip_suffix(suffix) {
            return wood;
        }
    }
    "oak"
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trips_nbt() {
        let mut data = SignData {
            hanging: false,
            ..SignData::default()
        };
        data.front.lines[0] = "Hello".into();
        data.front.color = 14;
        data.back.glowing = true;
        let tag = Tag::compound(data.encode());
        let mut decoded = SignData::default();
        decoded.decode(&tag);
        assert_eq!(decoded, data);
    }
    #[test]
    fn names_and_colors() {
        assert_eq!(sign_kind("minecraft:oak_wall_hanging_sign"), Some(true));
        assert_eq!(sign_kind("minecraft:sign"), Some(false));
        assert_eq!(sign_kind("minecraft:spruce_wall_sign"), Some(false));
        assert_eq!(sign_kind("minecraft:stone"), None);
        assert_eq!(wood_type("minecraft:dark_oak_wall_sign"), "dark_oak");
        assert_eq!(SignText::default().dark_color(), 0);
        let red = SignText {
            color: 14,
            ..SignText::default()
        };
        assert_eq!(red.dark_color(), 0x660000);
    }
}
