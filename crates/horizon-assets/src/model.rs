//! The 26.3 cuboid model format, before parent resolution and quad baking.
//!
//! Absent geometry differs from an explicit empty element list. Explicit null
//! elements, faces and display transforms are retained as the native decoder
//! does; render preparation must diagnose these before baking.
use crate::json::*;
use serde_json::value::RawValue;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Direction {
    Down,
    Up,
    North,
    South,
    West,
    East,
}
impl Direction {
    pub const ALL: [Self; 6] = [
        Self::Down,
        Self::Up,
        Self::North,
        Self::South,
        Self::West,
        Self::East,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Down => "down",
            Self::Up => "up",
            Self::North => "north",
            Self::South => "south",
            Self::West => "west",
            Self::East => "east",
        }
    }
    pub fn by_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|direction| direction.name() == name)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuiLight {
    Front,
    Side,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Material {
    pub sprite: String,
    pub force_translucent: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextureSlot {
    Reference(String),
    Material(Material),
}
#[derive(Clone, Debug, PartialEq)]
pub struct CuboidModel {
    pub parent: Option<String>,
    pub loader: Option<Loader>,
    pub ambient_occlusion: Option<bool>,
    pub gui_light: Option<GuiLight>,
    pub textures: BTreeMap<String, TextureSlot>,
    pub elements: Option<Vec<Option<Element>>>,
    pub display: Option<ItemTransforms>,
    /// NeoForge's `render_type`, without a `minecraft:` prefix.
    pub render_type: Option<String>,
    /// A model's `visibility`: a composite child name to shown; absent is shown.
    pub visibility: BTreeMap<String, bool>,
}
/// A custom model loader's payload; a model inherits its parent's loader unless it names its own.
#[derive(Clone, Debug, PartialEq)]
pub enum Loader {
    /// `neoforge:composite`: children in document order.
    Composite {
        children: Vec<(String, Arc<CuboidModel>)>,
        /// Child name to shown; a child absent from the map is shown.
        visibility: BTreeMap<String, bool>,
    },
    /// A loader this decoder does not know: it adds no geometry.
    Unknown(String),
}
/// NeoForge's `neoforge_data` of an element or a face; a face's keys win.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NeoforgeData {
    pub block_light: Option<u8>,
    pub sky_light: Option<u8>,
    /// ARGB.
    pub color: Option<i32>,
    pub ambient_occlusion: Option<bool>,
}
impl NeoforgeData {
    /// `self` over `base`, key by key.
    pub fn over(self, base: Self) -> Self {
        Self {
            block_light: self.block_light.or(base.block_light),
            sky_light: self.sky_light.or(base.sky_light),
            color: self.color.or(base.color),
            ambient_occlusion: self.ambient_occlusion.or(base.ambient_occlusion),
        }
    }
    fn decode(raw: &RawValue) -> Result<Self> {
        let obj = object(raw)?;
        let light = |key: &str| -> Result<Option<u8>> {
            Ok(obj
                .get(key)
                .map(|r| integer(r))
                .transpose()?
                .map(|v| v.clamp(0, 15) as u8))
        };
        // Some mods spell both lights as one `light_emission`.
        let emission = light("light_emission")?;
        let color = match obj.get("color") {
            Some(raw) if raw.get().starts_with('"') => {
                let text = string(raw)?;
                let text = text.trim();
                let hex = text
                    .strip_prefix('#')
                    .or_else(|| text.strip_prefix("0x"))
                    .or_else(|| text.strip_prefix("0X"));
                match hex {
                    Some(hex) => u32::from_str_radix(hex, 16).ok().map(|v| v as i32),
                    None => text.parse::<u32>().ok().map(|v| v as i32),
                }
            }
            Some(raw) => Some(integer(raw)?),
            None => None,
        };
        Ok(Self {
            block_light: light("block_light")?.or(emission),
            sky_light: light("sky_light")?.or(emission),
            color,
            ambient_occlusion: obj
                .get("ambient_occlusion")
                .map(|r| boolean(r))
                .transpose()?,
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub faces: BTreeMap<Direction, Option<Face>>,
    pub rotation: Option<ElementRotation>,
    pub shade_direction_override: Option<Direction>,
    pub light_emission: u8,
    pub neoforge_data: Option<NeoforgeData>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub cull: Option<Direction>,
    pub tint_index: i32,
    pub texture: String,
    pub uv: Option<[f32; 4]>,
    /// Normalized counterclockwise quadrants, in the range 0..4.
    pub rotation: u8,
    /// A face-level `light_emission`, which wins over its element's.
    pub light_emission: Option<u8>,
    pub neoforge_data: Option<NeoforgeData>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}
#[derive(Clone, Debug, PartialEq)]
pub enum RotationValue {
    SingleAxis { axis: Axis, angle: f32 },
    EulerXyz([f32; 3]),
}
#[derive(Clone, Debug, PartialEq)]
pub struct ElementRotation {
    /// Block units, converted from model coordinates during decoding.
    pub origin: [f32; 3],
    pub value: RotationValue,
    pub rescale: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ItemTransform {
    pub rotation: [f32; 3],
    pub translation: [f32; 3],
    pub scale: [f32; 3],
}
/// Missing transforms inherit later. Explicit identity and null values do not.
#[derive(Clone, Debug, PartialEq)]
pub enum DisplayTransform {
    Inherit,
    Explicit(Option<ItemTransform>),
}
pub const DISPLAY_CONTEXTS: [&str; 9] = [
    "thirdperson_lefthand",
    "thirdperson_righthand",
    "firstperson_lefthand",
    "firstperson_righthand",
    "head",
    "gui",
    "ground",
    "fixed",
    "on_shelf",
];
#[derive(Clone, Debug, PartialEq)]
pub struct ItemTransforms {
    pub contexts: [DisplayTransform; 9],
}

impl CuboidModel {
    /// Decode strict JSON, preserving the native parser's primitive coercions,
    /// optional inheritance fields, nullable children and f32 conversion.
    /// Like the native stream entry point, this reads one value and leaves
    /// trailing input unread. The pinned native entry point uses strict JSON.
    pub fn from_json(json: &str) -> Result<Self> {
        let raw = serde_json::Deserializer::from_str(json.strip_prefix('\u{feff}').unwrap_or(json))
            .into_iter::<&RawValue>()
            .next()
            .ok_or_else(|| error("empty model input"))??;
        validate_number_tokens(raw)?;
        Self::decode(raw)
    }
    fn decode(raw: &RawValue) -> Result<Self> {
        let obj = object(raw)?;
        let parent = obj
            .get("parent")
            .map(|r| primitive_string(r))
            .transpose()?
            .unwrap_or_default();
        let parent = if parent.is_empty() {
            None
        } else {
            Some(identifier(&parent)?)
        };
        let loader = obj
            .get("loader")
            .filter(|raw| raw.get() != "null")
            .map(|raw| Loader::decode(raw, &obj))
            .transpose()?;
        let ambient_occlusion = obj
            .get("ambientocclusion")
            .map(|r| boolean(r))
            .transpose()?;
        let gui_light = obj
            .get("gui_light")
            .map(|raw| match primitive_string(raw)?.as_str() {
                "front" => Ok(GuiLight::Front),
                "side" => Ok(GuiLight::Side),
                _ => Err(error("invalid gui light")),
            })
            .transpose()?;
        let mut textures = BTreeMap::new();
        if let Some(raw) = obj.get("textures") {
            for (key, raw) in object(raw)? {
                textures.insert(key, texture(raw)?);
            }
        }
        let elements = obj
            .get("elements")
            .map(|raw| {
                array(raw)?
                    .into_iter()
                    .map(|raw| nullable(raw, Element::decode))
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?;
        let display = obj
            .get("display")
            .map(|r| ItemTransforms::decode(r))
            .transpose()?;
        let render_type = obj
            .get("render_type")
            .filter(|raw| raw.get().starts_with('"'))
            .map(|raw| string(raw))
            .transpose()?
            .map(|name| name.strip_prefix("minecraft:").unwrap_or(&name).to_owned());
        Ok(Self {
            parent,
            loader,
            ambient_occlusion,
            gui_light,
            textures,
            elements,
            display,
            render_type,
            visibility: visibility(&obj)?,
        })
    }
    /// The models this one needs besides itself: its parent and its composite children's parents.
    pub fn referenced_models(&self) -> Vec<String> {
        let mut names: Vec<String> = self.parent.iter().cloned().collect();
        if let Some(Loader::Composite { children, .. }) = &self.loader {
            for (_, child) in children {
                names.extend(child.referenced_models());
            }
        }
        names
    }
}
fn visibility(obj: &Object<'_>) -> Result<BTreeMap<String, bool>> {
    let mut visibility = BTreeMap::new();
    if let Some(raw) = obj.get("visibility") {
        for (key, raw) in object(raw)? {
            visibility.insert(key, boolean(raw)?);
        }
    }
    Ok(visibility)
}
impl Loader {
    fn decode(raw: &RawValue, obj: &Object<'_>) -> Result<Self> {
        let name = primitive_string(raw)?;
        match name.as_str() {
            "neoforge:composite" => {
                let mut children = Vec::new();
                if let Some(raw) = obj.get("children") {
                    for (name, raw) in ordered_object(raw)? {
                        children.push((name, Arc::new(CuboidModel::decode(raw)?)));
                    }
                }
                Ok(Self::Composite {
                    children,
                    visibility: visibility(obj)?,
                })
            }
            _ => Ok(Self::Unknown(name)),
        }
    }
}
fn nullable<T>(raw: &RawValue, decode: impl FnOnce(&RawValue) -> Result<T>) -> Result<Option<T>> {
    if raw.get() == "null" {
        Ok(None)
    } else {
        decode(raw).map(Some)
    }
}
fn texture(raw: &RawValue) -> Result<TextureSlot> {
    if raw.get().starts_with('"') {
        let name = string(raw)?;
        if name.is_empty() {
            return Err(error("empty texture slot value"));
        }
        if let Some(target) = name.strip_prefix('#') {
            return Ok(TextureSlot::Reference(target.to_owned()));
        }
        return Ok(TextureSlot::Material(Material {
            sprite: identifier(&name)?,
            force_translucent: false,
        }));
    }
    let obj = object(raw)?;
    let sprite = identifier(&string(required(&obj, "sprite")?)?)?;
    let force_translucent = match obj.get("force_translucent") {
        None => false,
        Some(raw) if raw.get() == "null" => false,
        Some(raw) if raw.get() == "true" => true,
        Some(raw) if raw.get() == "false" => false,
        Some(_) => return Err(error("invalid material boolean")),
    };
    Ok(TextureSlot::Material(Material {
        sprite,
        force_translucent,
    }))
}
impl Element {
    fn decode(raw: &RawValue) -> Result<Self> {
        let obj = object(raw)?;
        let from = vector(required(&obj, "from")?)?;
        let to = vector(required(&obj, "to")?)?;
        if from.iter().chain(&to).any(|v| !(-16.0..=32.0).contains(v)) {
            return Err(error("element outside model bounds"));
        }
        let rotation = obj
            .get("rotation")
            .map(|r| ElementRotation::decode(r))
            .transpose()?;
        let mut faces = BTreeMap::new();
        for (name, raw) in object(required(&obj, "faces")?)? {
            let direction =
                Direction::by_name(&name).ok_or_else(|| error("invalid face direction"))?;
            faces.insert(direction, nullable(raw, Face::decode)?);
        }
        if faces.is_empty() {
            return Err(error("element requires a face"));
        }
        let shade_direction_override = obj
            .get("shade_direction_override")
            .map(|raw| {
                Direction::by_name(&primitive_string(raw)?)
                    .ok_or_else(|| error("invalid shade direction"))
            })
            .transpose()?;
        let light_emission = obj
            .get("light_emission")
            .map(|r| integer(r))
            .transpose()?
            .unwrap_or(0);
        if !(0..=15).contains(&light_emission) {
            return Err(error("invalid light emission"));
        }
        Ok(Self {
            from,
            to,
            faces,
            rotation,
            shade_direction_override,
            light_emission: light_emission as u8,
            neoforge_data: obj
                .get("neoforge_data")
                .map(|r| NeoforgeData::decode(r))
                .transpose()?,
        })
    }
}
impl Face {
    fn decode(raw: &RawValue) -> Result<Self> {
        let obj = object(raw)?;
        let cull = obj
            .get("cullface")
            .map(|r| primitive_string(r))
            .transpose()?
            .and_then(|s| Direction::by_name(&s));
        let texture = primitive_string(required(&obj, "texture")?)?;
        let tint_index = obj
            .get("tintindex")
            .map(|r| integer(r))
            .transpose()?
            .unwrap_or(-1);
        let rotation = obj
            .get("rotation")
            .map(|r| integer(r))
            .transpose()?
            .unwrap_or(0)
            .rem_euclid(360);
        if rotation % 90 != 0 {
            return Err(error("face rotation must be a multiple of 90"));
        }
        let uv = obj.get("uv").map(|r| vector(r)).transpose()?;
        Ok(Self {
            cull,
            tint_index,
            texture,
            uv,
            rotation: (rotation / 90) as u8,
            light_emission: obj
                .get("light_emission")
                .map(|r| integer(r))
                .transpose()?
                .map(|v| v.clamp(0, 15) as u8),
            neoforge_data: obj
                .get("neoforge_data")
                .map(|r| NeoforgeData::decode(r))
                .transpose()?,
        })
    }
}
impl ElementRotation {
    fn decode(raw: &RawValue) -> Result<Self> {
        let obj = object(raw)?;
        let origin = vector::<3>(required(&obj, "origin")?)?.map(|v| v * 0.0625);
        let value = if obj.contains_key("axis") || obj.contains_key("angle") {
            let axis = match primitive_string(required(&obj, "axis")?)?
                .to_lowercase()
                .as_str()
            {
                "x" => Axis::X,
                "y" => Axis::Y,
                "z" => Axis::Z,
                _ => return Err(error("invalid rotation axis")),
            };
            RotationValue::SingleAxis {
                axis,
                angle: float(required(&obj, "angle")?)?,
            }
        } else if ["x", "y", "z"].iter().any(|key| obj.contains_key(*key)) {
            let mut angles = [0.0; 3];
            for (angle, key) in angles.iter_mut().zip(["x", "y", "z"]) {
                *angle = obj.get(key).map(|r| float(r)).transpose()?.unwrap_or(0.0);
            }
            RotationValue::EulerXyz(angles)
        } else {
            return Err(error("missing rotation value"));
        };
        let rescale = obj
            .get("rescale")
            .map(|r| boolean(r))
            .transpose()?
            .unwrap_or(false);
        Ok(Self {
            origin,
            value,
            rescale,
        })
    }
}
impl ItemTransform {
    fn decode(raw: &RawValue) -> Result<Self> {
        let obj = object(raw)?;
        let rotation = obj
            .get("rotation")
            .map(|r| vector(r))
            .transpose()?
            .unwrap_or([0.0; 3]);
        let translation = obj
            .get("translation")
            .map(|r| vector(r))
            .transpose()?
            .unwrap_or([0.0; 3])
            .map(|v| (v * 0.0625).clamp(-5.0, 5.0));
        let scale = obj
            .get("scale")
            .map(|r| vector(r))
            .transpose()?
            .unwrap_or([1.0; 3])
            .map(|v| v.clamp(-4.0, 4.0));
        Ok(Self {
            rotation,
            translation,
            scale,
        })
    }
}
impl ItemTransforms {
    fn decode(raw: &RawValue) -> Result<Self> {
        let obj = object(raw)?;
        let mut contexts = std::array::from_fn(|_| DisplayTransform::Inherit);
        for (transform, key) in contexts.iter_mut().zip(DISPLAY_CONTEXTS) {
            if let Some(raw) = obj.get(key) {
                *transform = DisplayTransform::Explicit(nullable(raw, ItemTransform::decode)?);
            }
        }
        for (left, right) in [(0, 1), (2, 3)] {
            if contexts[left] == DisplayTransform::Inherit {
                contexts[left] = contexts[right].clone();
            }
        }
        Ok(Self { contexts })
    }
}
