//! `special` item models: the block-entity style renderers an item can ask
//! for (`SpecialModelRenderers`), their codec fields and what each reads from
//! the stack (`extractArgument`).
use super::{Result, float, get, ident, object, required, string};
use crate::json::error as err;
use serde_json::{Map, Value, json};

#[derive(Clone, Debug, PartialEq)]
pub enum SpecialKind {
    Bell,
    Banner {
        color: String,
        wall: bool,
    },
    Book {
        open_angle: f32,
        page1: f32,
        page2: f32,
    },
    Conduit,
    Chest {
        texture: String,
        openness: f32,
        chest_type: String,
    },
    CopperGolemStatue {
        texture: String,
        pose: String,
    },
    DecoratedPot,
    EndCube {
        effect: String,
    },
    Head {
        kind: String,
        texture: Option<String>,
        animation: f32,
    },
    PlayerHead,
    ShulkerBox {
        texture: String,
        openness: f32,
    },
    Shield,
    Trident,
}
#[derive(Clone, Debug, PartialEq)]
pub struct SpecialModel {
    pub kind: SpecialKind,
}

const DYES: [&str; 16] = [
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];
fn one_of(v: &Value, names: &[&str]) -> Result<String> {
    let s = string(v)?;
    if names.contains(&s.as_str()) {
        Ok(s)
    } else {
        Err(err("invalid enum name"))
    }
}

impl SpecialModel {
    /// A special model type of another namespace, which Horizon has no renderer for.
    pub fn is_foreign(v: &Value) -> bool {
        v.get("type")
            .and_then(Value::as_str)
            .and_then(|t| super::identifier(t).ok())
            .is_some_and(|t| super::foreign::is_foreign(&t))
    }
    pub fn decode(v: &Value) -> Result<Self> {
        let m = object(v)?;
        let float_or = |key: &str, default: f32| get(m, key).map_or(Ok(default), float);
        let kind = match ident(required(m, "type")?)?.as_str() {
            "minecraft:bell" => SpecialKind::Bell,
            "minecraft:banner" => SpecialKind::Banner {
                color: one_of(required(m, "color")?, &DYES)?,
                wall: match get(m, "attachment") {
                    None => false,
                    Some(a) => one_of(a, &["wall", "ground"])? == "wall",
                },
            },
            "minecraft:book" => SpecialKind::Book {
                open_angle: float(required(m, "open_angle")?)?,
                page1: float(required(m, "page1")?)?,
                page2: float(required(m, "page2")?)?,
            },
            "minecraft:conduit" => SpecialKind::Conduit,
            "minecraft:chest" => SpecialKind::Chest {
                texture: ident(required(m, "texture")?)?,
                openness: float_or("openness", 0.0)?,
                chest_type: match get(m, "chest_type") {
                    None => "single".to_owned(),
                    Some(t) => one_of(t, &["single", "left", "right"])?,
                },
            },
            "minecraft:copper_golem_statue" => SpecialKind::CopperGolemStatue {
                texture: ident(required(m, "texture")?)?,
                pose: one_of(
                    required(m, "pose")?,
                    &["standing", "sitting", "running", "star"],
                )?,
            },
            "minecraft:decorated_pot" => SpecialKind::DecoratedPot,
            "minecraft:end_cube" => SpecialKind::EndCube {
                effect: one_of(required(m, "effect")?, &["portal", "gateway"])?,
            },
            "minecraft:head" => SpecialKind::Head {
                kind: one_of(
                    required(m, "kind")?,
                    &[
                        "skeleton",
                        "wither_skeleton",
                        "player",
                        "zombie",
                        "creeper",
                        "piglin",
                        "dragon",
                    ],
                )?,
                texture: get(m, "texture").map(ident).transpose()?,
                animation: float_or("animation", 0.0)?,
            },
            "minecraft:player_head" => SpecialKind::PlayerHead,
            "minecraft:shulker_box" => SpecialKind::ShulkerBox {
                texture: ident(required(m, "texture")?)?,
                openness: float_or("openness", 0.0)?,
            },
            "minecraft:shield" => SpecialKind::Shield,
            "minecraft:trident" => SpecialKind::Trident,
            _ => return Err(err("unknown special model type")),
        };
        Ok(Self { kind })
    }
    /// The codec's encoding: optional fields equal to their default are omitted.
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        let mut put = |k: &str, v: Value| {
            m.insert(k.to_owned(), v);
        };
        let ty = match &self.kind {
            SpecialKind::Bell => "bell",
            SpecialKind::Banner { color, wall } => {
                put("color", json!(color));
                if *wall {
                    put("attachment", json!("wall"));
                }
                "banner"
            }
            SpecialKind::Book {
                open_angle,
                page1,
                page2,
            } => {
                put("open_angle", json!(open_angle));
                put("page1", json!(page1));
                put("page2", json!(page2));
                "book"
            }
            SpecialKind::Conduit => "conduit",
            SpecialKind::Chest {
                texture,
                openness,
                chest_type,
            } => {
                put("texture", json!(texture));
                if *openness != 0.0 {
                    put("openness", json!(openness));
                }
                if chest_type != "single" {
                    put("chest_type", json!(chest_type));
                }
                "chest"
            }
            SpecialKind::CopperGolemStatue { texture, pose } => {
                put("texture", json!(texture));
                put("pose", json!(pose));
                "copper_golem_statue"
            }
            SpecialKind::DecoratedPot => "decorated_pot",
            SpecialKind::EndCube { effect } => {
                put("effect", json!(effect));
                "end_cube"
            }
            SpecialKind::Head {
                kind,
                texture,
                animation,
            } => {
                put("kind", json!(kind));
                if let Some(t) = texture {
                    put("texture", json!(t));
                }
                if *animation != 0.0 {
                    put("animation", json!(animation));
                }
                "head"
            }
            SpecialKind::PlayerHead => "player_head",
            SpecialKind::ShulkerBox { texture, openness } => {
                put("texture", json!(texture));
                if *openness != 0.0 {
                    put("openness", json!(openness));
                }
                "shulker_box"
            }
            SpecialKind::Shield => "shield",
            SpecialKind::Trident => "trident",
        };
        m.insert("type".to_owned(), json!(format!("minecraft:{ty}")));
        Value::Object(m)
    }
    pub fn type_name(&self) -> String {
        self.to_json()["type"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }
    /// What `extractArgument` reads: banners their pattern layers, shields
    /// their base colour and layers, decorated pots their sherds and player
    /// heads their profile. Absent components give `null`.
    pub fn argument(&self, component: &dyn Fn(&str) -> Option<Value>) -> Value {
        match &self.kind {
            SpecialKind::Banner { .. } => {
                component("minecraft:banner_patterns").unwrap_or(Value::Null)
            }
            SpecialKind::DecoratedPot => {
                component("minecraft:pot_decorations").unwrap_or(Value::Null)
            }
            SpecialKind::PlayerHead => component("minecraft:profile").unwrap_or(Value::Null),
            SpecialKind::Shield => {
                let mut m = Map::new();
                for key in ["minecraft:banner_patterns", "minecraft:base_color"] {
                    if let Some(v) = component(key) {
                        m.insert(key.to_owned(), v);
                    }
                }
                Value::Object(m)
            }
            _ => Value::Null,
        }
    }
}
