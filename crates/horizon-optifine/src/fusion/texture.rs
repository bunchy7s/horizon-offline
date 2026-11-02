//! The `fusion` section of a texture's `.mcmeta` (beside vanilla's `animation`): the texture's
//! type and its options.

use super::{Connection, Layout, bool_field, int_field, kind, str_field};
use crate::cem::json::Json;

/// A texture's Fusion metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct Meta {
    pub base: Base,
    pub kind: Kind,
}

/// The options every type takes (`base` on its own).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Base {
    /// Drawn fully bright, as at light level 15.
    pub emissive: bool,
    pub render_type: Option<RenderType>,
    /// Tinted as this, whatever the block tints.
    pub tinting: Option<Tinting>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderType {
    Opaque,
    Cutout,
    Translucent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tinting {
    Grass,
    Foliage,
    Water,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Base,
    Connecting {
        layout: Layout,
        /// The texture's own; a `fusion:connecting` model's replace them.
        connections: Connection,
        /// Each tile's own metadata, as if it were a texture.
        sub_texture: Option<Box<Meta>>,
        /// Each tile holds its animation frames, instead of each frame holding every tile.
        per_tile_animation: bool,
    },
    Random {
        columns: u32,
        rows: u32,
        source: RandomSource,
        seed: Option<i64>,
        sub_texture: Option<Box<Meta>>,
        per_tile_animation: bool,
    },
    /// One image stretched over `columns` x `rows` blocks.
    Continuous {
        columns: u32,
        rows: u32,
    },
    Scrolling(Scrolling),
}

/// What picks a random texture's tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RandomSource {
    Position,
    PositionFacing,
    PositionAxis,
}

/// An animation whose frame, `frame_width` x `frame_height` pixels, moves across the image from
/// one corner to another, each step shown for `frame_time` ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scrolling {
    pub from: Corner,
    pub to: Corner,
    pub frame_width: u32,
    pub frame_height: u32,
    pub frame_time: u32,
    pub loop_type: LoopType,
    /// Ticks between loops.
    pub loop_pause: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopType {
    /// Around the image to the start.
    Wrap,
    /// Back to the start at once.
    Reset,
    /// Back the way it came.
    Reverse,
}

/// `key`'s value, which must be one of `options`.
fn choice<T: Copy>(json: &Json, key: &str, options: &[(&str, T)]) -> Result<Option<T>, String> {
    match str_field(json, key)? {
        None => Ok(None),
        Some(value) => options
            .iter()
            .find(|(name, _)| *name == value)
            .map(|&(_, v)| Some(v))
            .ok_or_else(|| format!("`{key}` can't be `{value}`")),
    }
}

const CORNERS: [(&str, Corner); 4] = [
    ("top_left", Corner::TopLeft),
    ("top_right", Corner::TopRight),
    ("bottom_left", Corner::BottomLeft),
    ("bottom_right", Corner::BottomRight),
];

impl Meta {
    /// A `.mcmeta` file's Fusion metadata: `None` when it has no `fusion` section.
    pub fn from_mcmeta(text: &str) -> Result<Option<Self>, String> {
        let json = Json::parse(text)?;
        json.get("fusion").map(Self::parse).transpose()
    }

    /// A `fusion` section (or a `sub_texture`).
    pub fn parse(json: &Json) -> Result<Self, String> {
        if json.as_object().is_none() {
            return Err("a texture's Fusion metadata that isn't an object".into());
        }
        let base = Base {
            emissive: bool_field(json, "emissive")?.unwrap_or(false),
            render_type: choice(
                json,
                "render_type",
                &[
                    ("opaque", RenderType::Opaque),
                    ("cutout", RenderType::Cutout),
                    ("translucent", RenderType::Translucent),
                ],
            )?,
            tinting: choice(
                json,
                "tinting",
                &[
                    ("biome_grass", Tinting::Grass),
                    ("biome_foliage", Tinting::Foliage),
                    ("biome_water", Tinting::Water),
                ],
            )?,
        };
        let sub_texture = || {
            json.get("sub_texture")
                .map(|s| Self::parse(s).map(Box::new))
                .transpose()
        };
        let per_tile_animation =
            || Ok::<_, String>(bool_field(json, "per_tile_animation")?.unwrap_or(false));
        // Without a type the options stand alone (Fusion Emissive Ores' overlays: only
        // `emissive` and `render_type`).
        let ty = str_field(json, "type")?.unwrap_or("base");
        let kind = match kind(ty) {
            "base" => Kind::Base,
            "connecting" => Kind::Connecting {
                layout: match str_field(json, "layout")? {
                    None => Layout::Full,
                    Some(l) => Layout::parse(l).ok_or_else(|| format!("unknown layout `{l}`"))?,
                },
                connections: match json.get("connections") {
                    None => Connection::SameState,
                    Some(c) => Connection::parse(c)?,
                },
                sub_texture: sub_texture()?,
                per_tile_animation: per_tile_animation()?,
            },
            "random" => Kind::Random {
                columns: int_field(json, "columns", 1..=100)?.unwrap_or(1) as u32,
                rows: int_field(json, "rows", 1..=100)?.unwrap_or(1) as u32,
                source: choice(
                    json,
                    "randomness_source",
                    &[
                        ("position", RandomSource::Position),
                        ("position_facing", RandomSource::PositionFacing),
                        ("position_axis", RandomSource::PositionAxis),
                    ],
                )?
                .unwrap_or(RandomSource::PositionFacing),
                // JSON numbers here are `f64`s: from 2^53 a seed may not be the one written.
                seed: int_field(json, "seed", -(1 << 53) + 1..=(1 << 53) - 1)?,
                sub_texture: sub_texture()?,
                per_tile_animation: per_tile_animation()?,
            },
            "continuous" => Kind::Continuous {
                columns: int_field(json, "columns", 1..=32)?.unwrap_or(1) as u32,
                rows: int_field(json, "rows", 1..=32)?.unwrap_or(1) as u32,
            },
            "scrolling" => {
                let positive = |key, default| {
                    Ok::<_, String>(
                        int_field(json, key, 1..=i64::from(u32::MAX))?.unwrap_or(default) as u32,
                    )
                };
                Kind::Scrolling(Scrolling {
                    from: choice(json, "from", &CORNERS)?.unwrap_or(Corner::TopLeft),
                    to: choice(json, "to", &CORNERS)?.unwrap_or(Corner::BottomLeft),
                    frame_width: positive("frame_width", 16)?,
                    frame_height: positive("frame_height", 16)?,
                    frame_time: positive("frame_time", 10)?,
                    loop_type: choice(
                        json,
                        "loop_type",
                        &[
                            ("wrap", LoopType::Wrap),
                            ("reset", LoopType::Reset),
                            ("reverse", LoopType::Reverse),
                        ],
                    )?
                    .unwrap_or(LoopType::Wrap),
                    loop_pause: int_field(json, "loop_pause", 0..=i64::from(u32::MAX))?.unwrap_or(0)
                        as u32,
                })
            }
            other => return Err(format!("unknown texture type `{other}`")),
        };
        Ok(Self { base, kind })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BlockMatch;

    fn meta(text: &str) -> Meta {
        Meta::from_mcmeta(text).unwrap().unwrap()
    }

    #[test]
    fn vanilla_mcmeta_has_none() {
        assert_eq!(
            Meta::from_mcmeta(r#"{"animation": {"frametime": 2}}"#),
            Ok(None)
        );
    }

    #[test]
    fn options_without_a_type_are_base() {
        assert_eq!(
            meta(r#"{"fusion": {"emissive": true, "render_type": "cutout"}}"#),
            Meta {
                base: Base {
                    emissive: true,
                    render_type: Some(RenderType::Cutout),
                    tinting: None,
                },
                kind: Kind::Base,
            }
        );
    }

    #[test]
    fn connecting_with_defaults_and_base_options() {
        let m = meta(
            r#"{"fusion": {"type": "connecting", "render_type": "cutout", "tinting": "biome_grass"},
                "animation": {"frametime": 10}}"#,
        );
        assert_eq!(
            m,
            Meta {
                base: Base {
                    emissive: false,
                    render_type: Some(RenderType::Cutout),
                    tinting: Some(Tinting::Grass),
                },
                kind: Kind::Connecting {
                    layout: Layout::Full,
                    connections: Connection::SameState,
                    sub_texture: None,
                    per_tile_animation: false,
                },
            }
        );
    }

    #[test]
    fn connecting_with_a_random_sub_texture() {
        let m = meta(
            r#"{"fusion": {"type": "fusion:connecting", "layout": "pieced", "emissive": true,
                "connections": [{"type": "is_same_block"}, {"type": "match_block", "block": "iron_block"}],
                "sub_texture": {"type": "random", "rows": 5, "columns": 4, "seed": -3},
                "per_tile_animation": true}}"#,
        );
        assert!(m.base.emissive);
        let Kind::Connecting {
            layout,
            connections,
            sub_texture,
            per_tile_animation,
        } = m.kind
        else {
            panic!("{m:?}");
        };
        assert_eq!(layout, Layout::Pieced);
        assert!(per_tile_animation);
        assert_eq!(
            connections,
            Connection::Or(vec![
                Connection::SameBlock,
                Connection::Neighbour(vec![BlockMatch {
                    id: "minecraft:iron_block".into(),
                    states: vec![],
                }]),
            ])
        );
        assert_eq!(
            sub_texture.unwrap().kind,
            Kind::Random {
                columns: 4,
                rows: 5,
                source: RandomSource::PositionFacing,
                seed: Some(-3),
                sub_texture: None,
                per_tile_animation: false,
            }
        );
    }

    #[test]
    fn continuous_and_scrolling() {
        assert_eq!(
            meta(r#"{"fusion": {"type": "continuous", "rows": 2, "columns": 3}}"#).kind,
            Kind::Continuous {
                columns: 3,
                rows: 2,
            }
        );
        assert_eq!(
            meta(
                r#"{"fusion": {"type": "scrolling", "to": "top_right", "frame_time": 4,
                    "frame_height": 8, "loop_type": "reverse", "loop_pause": 20}}"#
            )
            .kind,
            Kind::Scrolling(Scrolling {
                from: Corner::TopLeft,
                to: Corner::TopRight,
                frame_width: 16,
                frame_height: 8,
                frame_time: 4,
                loop_type: LoopType::Reverse,
                loop_pause: 20,
            })
        );
    }

    #[test]
    fn defaults() {
        assert_eq!(
            meta(r#"{"fusion": {"type": "random", "randomness_source": "position_axis"}}"#).kind,
            Kind::Random {
                columns: 1,
                rows: 1,
                source: RandomSource::PositionAxis,
                seed: None,
                sub_texture: None,
                per_tile_animation: false,
            }
        );
        assert_eq!(
            meta(r#"{"fusion": {"type": "continuous"}}"#).kind,
            Kind::Continuous {
                columns: 1,
                rows: 1
            }
        );
        assert_eq!(
            meta(r#"{"fusion": {"type": "scrolling"}}"#).kind,
            Kind::Scrolling(Scrolling {
                from: Corner::TopLeft,
                to: Corner::BottomLeft,
                frame_width: 16,
                frame_height: 16,
                frame_time: 10,
                loop_type: LoopType::Wrap,
                loop_pause: 0,
            })
        );
    }

    #[test]
    fn refuses_bad_values() {
        for bad in [
            r#"{"fusion": {"type": "random", "seed": 9007199254740993}}"#,
            r#"{"fusion": {"type": "random", "rows": 101}}"#,
            r#"{"fusion": {"type": "scrolling", "frame_time": 0}}"#,
            r#"{"fusion": {"type": "scrolling", "loop_pause": -1}}"#,
            r#"{"fusion": {"type": "connecting", "sub_texture": {"type": "random", "rows": 0}}}"#,
            r#"{"fusion": {"type": "glowing"}}"#,
            r#"{"fusion": "connecting"}"#,
            r#"{"fusion": {"type": "connecting", "sub_texture": "random"}}"#,
            r#"{"fusion": {"type": "connecting", "layout": "diagonal"}}"#,
            r#"{"fusion": {"type": "base", "emissive": "yes"}}"#,
            r#"{"fusion": {"type": "base", "render_type": "solid"}}"#,
            r#"{"fusion": {"type": "random", "rows": 0}}"#,
            r#"{"fusion": {"type": "continuous", "columns": 33}}"#,
            r#"{"fusion": {"type": "scrolling", "frame_width": 1.5}}"#,
        ] {
            assert!(Meta::from_mcmeta(bad).is_err(), "{bad}");
        }
    }
}
