//! The 26.3 client item definitions (`assets/<ns>/items/*.json`): the model
//! tree (`model`, `composite`, `condition`, `select`, `range_dispatch`,
//! `special`, `bundle/selected_item`, `empty`), its properties and tints, and
//! the resolver that turns one stack into render layers
//! (`ItemModelResolver.updateForTopItem`).
//!
//! Baked geometry lives in [`crate::item_bake`]; this module decides *which*
//! model, tints, foil and transformation each layer gets.
use crate::{
    json::{DecodeError, error as err},
    math::{Mat4, normalize_quaternion, quaternion_from_axis_angle},
};
use serde_json::{Map, Value};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

mod component_predicate;
mod special;
pub use special::{SpecialKind, SpecialModel};

type Result<T> = std::result::Result<T, DecodeError>;

/// `BuiltInRegistries.DATA_COMPONENT_TYPE`, which `has_component` and the
/// `component` select validate against (checked by the harness).
pub const DATA_COMPONENT_TYPES: [&str; 122] = [
    "minecraft:additional_trade_cost",
    "minecraft:attack_animation",
    "minecraft:attack_range",
    "minecraft:attribute_modifiers",
    "minecraft:axolotl/variant",
    "minecraft:banner_patterns",
    "minecraft:base_color",
    "minecraft:bees",
    "minecraft:block_entity_data",
    "minecraft:block_state",
    "minecraft:block_transformer",
    "minecraft:blocks_attacks",
    "minecraft:break_sound",
    "minecraft:brewing_fuel",
    "minecraft:bucket_entity_data",
    "minecraft:bundle_contents",
    "minecraft:can_break",
    "minecraft:can_place_on",
    "minecraft:cat/collar",
    "minecraft:cat/sound_variant",
    "minecraft:cat/variant",
    "minecraft:charged_projectiles",
    "minecraft:chicken/sound_variant",
    "minecraft:chicken/variant",
    "minecraft:compostable",
    "minecraft:consumable",
    "minecraft:container",
    "minecraft:container_loot",
    "minecraft:cooking_fuel",
    "minecraft:cow/sound_variant",
    "minecraft:cow/variant",
    "minecraft:creative_slot_lock",
    "minecraft:cushion/color",
    "minecraft:custom_data",
    "minecraft:custom_model_data",
    "minecraft:custom_name",
    "minecraft:damage",
    "minecraft:damage_resistant",
    "minecraft:damage_type",
    "minecraft:death_protection",
    "minecraft:debug_stick_state",
    "minecraft:dye",
    "minecraft:dyed_color",
    "minecraft:enchantable",
    "minecraft:enchantment_glint_override",
    "minecraft:enchantments",
    "minecraft:entity_data",
    "minecraft:equippable",
    "minecraft:firework_explosion",
    "minecraft:fireworks",
    "minecraft:food",
    "minecraft:fox/variant",
    "minecraft:frog/variant",
    "minecraft:glider",
    "minecraft:horse/variant",
    "minecraft:instrument",
    "minecraft:intangible_projectile",
    "minecraft:interact_animation",
    "minecraft:item_model",
    "minecraft:item_name",
    "minecraft:jukebox_playable",
    "minecraft:kinetic_weapon",
    "minecraft:llama/variant",
    "minecraft:lock",
    "minecraft:lodestone_tracker",
    "minecraft:lore",
    "minecraft:map_decorations",
    "minecraft:map_id",
    "minecraft:map_post_processing",
    "minecraft:max_damage",
    "minecraft:max_stack_size",
    "minecraft:minimum_attack_charge",
    "minecraft:mob_visibility",
    "minecraft:mooshroom/variant",
    "minecraft:note_block_sound",
    "minecraft:ominous_bottle_amplifier",
    "minecraft:painting/variant",
    "minecraft:parrot/variant",
    "minecraft:piercing_weapon",
    "minecraft:pig/sound_variant",
    "minecraft:pig/variant",
    "minecraft:pot_decorations",
    "minecraft:potion_contents",
    "minecraft:potion_duration_scale",
    "minecraft:profile",
    "minecraft:provides_banner_patterns",
    "minecraft:provides_pottery_pattern",
    "minecraft:provides_trim_material",
    "minecraft:rabbit/variant",
    "minecraft:rarity",
    "minecraft:recipes",
    "minecraft:repair_cost",
    "minecraft:repairable",
    "minecraft:salmon/size",
    "minecraft:sheep/color",
    "minecraft:shulker/color",
    "minecraft:sign_text_back",
    "minecraft:sign_text_front",
    "minecraft:stored_enchantments",
    "minecraft:sulfur_cube_content",
    "minecraft:suspicious_stew_effects",
    "minecraft:tool",
    "minecraft:tooltip_display",
    "minecraft:tooltip_style",
    "minecraft:trim",
    "minecraft:tropical_fish/base_color",
    "minecraft:tropical_fish/pattern",
    "minecraft:tropical_fish/pattern_color",
    "minecraft:unbreakable",
    "minecraft:use_cooldown",
    "minecraft:use_effects",
    "minecraft:use_remainder",
    "minecraft:villager/variant",
    "minecraft:villager_food",
    "minecraft:waxed",
    "minecraft:weapon",
    "minecraft:wolf/collar",
    "minecraft:wolf/sound_variant",
    "minecraft:wolf/variant",
    "minecraft:writable_book_content",
    "minecraft:written_book_content",
    "minecraft:zombie_nautilus/variant",
];

// ---------------------------------------------------------------------------
// Display contexts
// ---------------------------------------------------------------------------

/// `ItemDisplayContext`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DisplayContext {
    None,
    ThirdPersonLeftHand,
    ThirdPersonRightHand,
    FirstPersonLeftHand,
    FirstPersonRightHand,
    Head,
    Gui,
    Ground,
    Fixed,
    OnShelf,
}
impl DisplayContext {
    pub const ALL: [Self; 10] = [
        Self::None,
        Self::ThirdPersonLeftHand,
        Self::ThirdPersonRightHand,
        Self::FirstPersonLeftHand,
        Self::FirstPersonRightHand,
        Self::Head,
        Self::Gui,
        Self::Ground,
        Self::Fixed,
        Self::OnShelf,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ThirdPersonLeftHand => "thirdperson_lefthand",
            Self::ThirdPersonRightHand => "thirdperson_righthand",
            Self::FirstPersonLeftHand => "firstperson_lefthand",
            Self::FirstPersonRightHand => "firstperson_righthand",
            Self::Head => "head",
            Self::Gui => "gui",
            Self::Ground => "ground",
            Self::Fixed => "fixed",
            Self::OnShelf => "on_shelf",
        }
    }
    pub fn by_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }
    /// `ItemDisplayContext.leftHand`.
    pub fn left_hand(self) -> bool {
        matches!(self, Self::ThirdPersonLeftHand | Self::FirstPersonLeftHand)
    }
}

// ---------------------------------------------------------------------------
// Decoding helpers (DataFixerUpper `JsonOps` semantics)
// ---------------------------------------------------------------------------

fn object(v: &Value) -> Result<&Map<String, Value>> {
    v.as_object().ok_or_else(|| err("expected object"))
}
/// A present, non-null field (`JsonOps` maps `null` to an absent key).
fn get<'a>(m: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    m.get(key).filter(|v| !v.is_null())
}
fn required<'a>(m: &'a Map<String, Value>, key: &str) -> Result<&'a Value> {
    get(m, key).ok_or_else(|| err(&format!("missing {key}")))
}
/// `Codec.FLOAT`: numbers, and booleans as 1 and 0.
fn float(v: &Value) -> Result<f32> {
    match v {
        Value::Number(n) => n
            .to_string()
            .parse()
            .or_else(|_| n.as_f64().map(|f| f as f32).ok_or(()))
            .map_err(|()| err("invalid float")),
        _ => Err(err("expected float")),
    }
}
/// `Codec.INT`: `Number.intValue`, truncating toward zero and saturating
/// like a Java narrowing of a double (longs keep their low 32 bits).
fn int(v: &Value) -> Result<i32> {
    match v {
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i as i32)
            } else if let Some(u) = n.as_u64() {
                Ok(u as i32)
            } else {
                Ok(n.as_f64().ok_or_else(|| err("invalid int"))? as i32)
            }
        }
        _ => Err(err("expected int")),
    }
}
fn boolean(v: &Value) -> Result<bool> {
    v.as_bool().ok_or_else(|| err("expected boolean"))
}
fn string(v: &Value) -> Result<String> {
    v.as_str()
        .map(str::to_owned)
        .ok_or_else(|| err("expected string"))
}
fn list(v: &Value) -> Result<&Vec<Value>> {
    v.as_array().ok_or_else(|| err("expected list"))
}
/// `Identifier.CODEC`: a lowercase `namespace:path`, defaulting the namespace.
pub fn identifier(name: &str) -> Result<String> {
    let (namespace, path) = match name.split_once(':') {
        Some((n, p)) => (if n.is_empty() { "minecraft" } else { n }, p),
        None => ("minecraft", name),
    };
    let valid = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c);
    if path.contains(':')
        || !namespace.bytes().all(valid)
        || !path.bytes().all(|c| valid(c) || c == b'/')
    {
        return Err(err("invalid resource identifier"));
    }
    Ok(format!("{namespace}:{path}"))
}
fn ident(v: &Value) -> Result<String> {
    identifier(&string(v)?)
}
fn component_type(v: &Value) -> Result<String> {
    let id = ident(v)?;
    if DATA_COMPONENT_TYPES.contains(&id.as_str()) {
        Ok(id)
    } else {
        Err(err("unknown data component type"))
    }
}
fn vector<const N: usize>(v: &Value) -> Result<[f32; N]> {
    let values = list(v)?;
    if values.len() != N {
        return Err(err("wrong vector length"));
    }
    let mut out = [0.0; N];
    for (o, v) in out.iter_mut().zip(values) {
        *o = float(v)?;
    }
    Ok(out)
}
fn non_negative(v: &Value) -> Result<i32> {
    let i = int(v)?;
    if i < 0 {
        return Err(err("value must be non-negative"));
    }
    Ok(i)
}
/// `ExtraCodecs.RGB_COLOR_CODEC`: an int, or three floats in 0..1.
fn rgb(v: &Value) -> Result<i32> {
    if v.is_array() {
        let [r, g, b] = vector::<3>(v)?;
        return Ok(argb(255, channel(r), channel(g), channel(b)));
    }
    int(v)
}
fn argb(a: i32, r: i32, g: i32, b: i32) -> i32 {
    (a & 255) << 24 | (r & 255) << 16 | (g & 255) << 8 | (b & 255)
}
fn opaque(color: i32) -> i32 {
    color | (0xFF00_0000u32 as i32)
}

/// `Transformation.EXTENDED_CODEC`.
fn transformation(v: &Value) -> Result<Mat4> {
    if v.is_array() {
        return Ok(Mat4::from_row_major(vector::<16>(v)?));
    }
    let m = object(v)?;
    let quaternion = |v: &Value| -> Result<[f32; 4]> {
        if v.is_array() {
            return Ok(normalize_quaternion(vector::<4>(v)?));
        }
        let o = object(v)?;
        Ok(quaternion_from_axis_angle(
            float(required(o, "angle")?)?,
            vector::<3>(required(o, "axis")?)?,
        ))
    };
    let translation = vector::<3>(required(m, "translation")?)?;
    let left = quaternion(required(m, "left_rotation")?)?;
    let scale = vector::<3>(required(m, "scale")?)?;
    let right = quaternion(required(m, "right_rotation")?)?;
    Ok(Mat4::translation(translation)
        .rotate(left)
        .scale(scale)
        .rotate(right))
}
fn optional_transformation(m: &Map<String, Value>) -> Result<Option<Mat4>> {
    get(m, "transformation").map(transformation).transpose()
}
/// `Mth.floor(float)` as `ARGB.as8BitChannel` uses it.
/// `ARGB.as8BitChannel`: `Mth.floor(value * 255.0F)`.
fn channel(v: f32) -> i32 {
    let scaled = v * 255.0;
    let i = scaled as i32;
    if (i as f32) > scaled { i - 1 } else { i }
}

// ---------------------------------------------------------------------------
// The model tree
// ---------------------------------------------------------------------------

/// One `items/<name>.json` file: a model tree plus the client properties.
#[derive(Clone, Debug)]
pub struct ClientItem {
    pub model: ItemModelNode,
    pub hand_animation_on_swap: bool,
    pub oversized_in_gui: bool,
    pub swap_animation_scale: f32,
}
impl ClientItem {
    pub const DEFAULT_PROPERTIES: (bool, bool, f32) = (true, false, 1.0);
    pub fn from_json(text: &str) -> Result<Self> {
        let value: Value = serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(text))?;
        Self::decode(&value)
    }
    pub fn decode(value: &Value) -> Result<Self> {
        let m = object(value)?;
        Ok(Self {
            model: ItemModelNode::decode(required(m, "model")?)?,
            hand_animation_on_swap: get(m, "hand_animation_on_swap").map_or(Ok(true), boolean)?,
            oversized_in_gui: get(m, "oversized_in_gui").map_or(Ok(false), boolean)?,
            swap_animation_scale: get(m, "swap_animation_scale").map_or(Ok(1.0), float)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelLeaf {
    pub model: String,
    pub transformation: Option<Mat4>,
    pub tints: Vec<TintSource>,
}
#[derive(Clone, Debug)]
pub enum ItemModelNode {
    Empty,
    Model(ModelLeaf),
    Composite {
        models: Vec<ItemModelNode>,
        transformation: Option<Mat4>,
    },
    Condition {
        transformation: Option<Mat4>,
        property: ConditionProperty,
        on_true: Box<ItemModelNode>,
        on_false: Box<ItemModelNode>,
    },
    Select {
        transformation: Option<Mat4>,
        property: SelectProperty,
        cases: Vec<SwitchCase>,
        fallback: Option<Box<ItemModelNode>>,
    },
    RangeDispatch {
        transformation: Option<Mat4>,
        property: NumericProperty,
        scale: f32,
        /// Sorted by threshold (a stable sort, as `bake` does).
        entries: Vec<(f32, ItemModelNode)>,
        fallback: Option<Box<ItemModelNode>>,
    },
    Special {
        base: String,
        transformation: Option<Mat4>,
        model: SpecialModel,
    },
    BundleSelectedItem,
}
#[derive(Clone, Debug)]
pub struct SwitchCase {
    /// Property values, in their canonical JSON form.
    pub values: Vec<Value>,
    pub model: ItemModelNode,
}

impl ItemModelNode {
    pub fn decode(v: &Value) -> Result<Self> {
        let m = object(v)?;
        let kind = ident(required(m, "type")?)?;
        Ok(match kind.as_str() {
            "minecraft:empty" => Self::Empty,
            "minecraft:model" => Self::Model(ModelLeaf {
                model: ident(required(m, "model")?)?,
                transformation: optional_transformation(m)?,
                tints: match get(m, "tints") {
                    None => Vec::new(),
                    Some(t) => list(t)?
                        .iter()
                        .map(TintSource::decode)
                        .collect::<Result<_>>()?,
                },
            }),
            "minecraft:composite" => Self::Composite {
                models: list(required(m, "models")?)?
                    .iter()
                    .map(Self::decode)
                    .collect::<Result<_>>()?,
                transformation: optional_transformation(m)?,
            },
            "minecraft:condition" => Self::Condition {
                transformation: optional_transformation(m)?,
                property: ConditionProperty::decode(m)?,
                on_true: Box::new(Self::decode(required(m, "on_true")?)?),
                on_false: Box::new(Self::decode(required(m, "on_false")?)?),
            },
            "minecraft:select" => {
                let transformation = optional_transformation(m)?;
                let property = SelectProperty::decode(m)?;
                let cases = list(required(m, "cases")?)?
                    .iter()
                    .map(|c| SwitchCase::decode(&property, c))
                    .collect::<Result<Vec<_>>>()?;
                if cases.is_empty() {
                    return Err(err("empty case list"));
                }
                let mut seen: Vec<&Value> = Vec::new();
                for value in cases.iter().flat_map(|c| &c.values) {
                    if seen.contains(&value) {
                        return Err(err("duplicate case conditions"));
                    }
                    seen.push(value);
                }
                Self::Select {
                    transformation,
                    property,
                    cases,
                    fallback: get(m, "fallback")
                        .map(|f| Self::decode(f).map(Box::new))
                        .transpose()?,
                }
            }
            "minecraft:range_dispatch" => {
                let mut entries = list(required(m, "entries")?)?
                    .iter()
                    .map(|e| {
                        let e = object(e)?;
                        Ok((
                            float(required(e, "threshold")?)?,
                            Self::decode(required(e, "model")?)?,
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                // `Comparator.comparingDouble`: Double.compare, a total order.
                entries.sort_by(|a, b| f64::from(a.0).total_cmp(&f64::from(b.0)));
                Self::RangeDispatch {
                    transformation: optional_transformation(m)?,
                    property: NumericProperty::decode(m)?,
                    scale: get(m, "scale").map_or(Ok(1.0), float)?,
                    entries,
                    fallback: get(m, "fallback")
                        .map(|f| Self::decode(f).map(Box::new))
                        .transpose()?,
                }
            }
            "minecraft:special" => Self::Special {
                base: ident(required(m, "base")?)?,
                transformation: optional_transformation(m)?,
                model: SpecialModel::decode(required(m, "model")?)?,
            },
            "minecraft:bundle/selected_item" => Self::BundleSelectedItem,
            _ => return Err(err("unknown item model type")),
        })
    }
    /// Every model resource id the tree can ask for.
    pub fn dependencies(&self, out: &mut Vec<String>) {
        match self {
            Self::Empty | Self::BundleSelectedItem => {}
            Self::Model(leaf) => out.push(leaf.model.clone()),
            Self::Composite { models, .. } => models.iter().for_each(|m| m.dependencies(out)),
            Self::Condition {
                on_true, on_false, ..
            } => {
                on_true.dependencies(out);
                on_false.dependencies(out);
            }
            Self::Select {
                cases, fallback, ..
            } => {
                cases.iter().for_each(|c| c.model.dependencies(out));
                fallback.iter().for_each(|f| f.dependencies(out));
            }
            Self::RangeDispatch {
                entries, fallback, ..
            } => {
                entries.iter().for_each(|(_, m)| m.dependencies(out));
                fallback.iter().for_each(|f| f.dependencies(out));
            }
            Self::Special { base, .. } => out.push(base.clone()),
        }
    }
}
impl SwitchCase {
    fn decode(property: &SelectProperty, v: &Value) -> Result<Self> {
        let m = object(v)?;
        let when = required(m, "when")?;
        // `compactListCodec`: a list, or one value; the list must not be empty.
        let raw: Vec<&Value> = match when {
            Value::Array(a) if !a.is_empty() => a.iter().collect(),
            Value::Array(_) => return Err(err("list must have contents")),
            single => vec![single],
        };
        Ok(Self {
            values: raw
                .into_iter()
                .map(|v| property.decode_value(v))
                .collect::<Result<_>>()?,
            model: ItemModelNode::decode(required(m, "model")?)?,
        })
    }
}

// ---------------------------------------------------------------------------
// Tint sources
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum TintSource {
    Constant(i32),
    CustomModelData { index: i32, default: i32 },
    Dye(i32),
    Grass { temperature: f32, downfall: f32 },
    Firework(i32),
    Potion(i32),
    Team(i32),
}
impl TintSource {
    fn decode(v: &Value) -> Result<Self> {
        let m = object(v)?;
        let default = || rgb(required(m, "default")?);
        let unit = |v: &Value| -> Result<f32> {
            let f = float(v)?;
            if (0.0..=1.0).contains(&f) {
                Ok(f)
            } else {
                Err(err("value must be within range [0;1]"))
            }
        };
        Ok(match ident(required(m, "type")?)?.as_str() {
            "minecraft:constant" => Self::Constant(opaque(rgb(required(m, "value")?)?)),
            "minecraft:custom_model_data" => Self::CustomModelData {
                index: get(m, "index").map_or(Ok(0), non_negative)?,
                default: default()?,
            },
            "minecraft:dye" => Self::Dye(default()?),
            "minecraft:grass" => Self::Grass {
                temperature: unit(required(m, "temperature")?)?,
                downfall: unit(required(m, "downfall")?)?,
            },
            "minecraft:firework" => Self::Firework(default()?),
            "minecraft:potion" => Self::Potion(default()?),
            "minecraft:team" => Self::Team(default()?),
            _ => return Err(err("unknown item tint source")),
        })
    }
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum ConditionProperty {
    CustomModelData(i32),
    UsingItem,
    Broken,
    Damaged,
    FishingRodCast,
    HasComponent {
        component: String,
        ignore_default: bool,
    },
    BundleHasSelectedItem,
    Selected,
    Carried,
    ExtendedView,
    KeybindDown(String),
    ViewEntity,
    /// A single `component` predicate; external predicate types use the host.
    Component(Value),
}
impl ConditionProperty {
    fn decode(m: &Map<String, Value>) -> Result<Self> {
        Ok(match ident(required(m, "property")?)?.as_str() {
            "minecraft:custom_model_data" => {
                Self::CustomModelData(get(m, "index").map_or(Ok(0), non_negative)?)
            }
            "minecraft:using_item" => Self::UsingItem,
            "minecraft:broken" => Self::Broken,
            "minecraft:damaged" => Self::Damaged,
            "minecraft:fishing_rod/cast" => Self::FishingRodCast,
            "minecraft:has_component" => Self::HasComponent {
                component: component_type(required(m, "component")?)?,
                ignore_default: get(m, "ignore_default").map_or(Ok(false), boolean)?,
            },
            "minecraft:bundle/has_selected_item" => Self::BundleHasSelectedItem,
            "minecraft:selected" => Self::Selected,
            "minecraft:carried" => Self::Carried,
            "minecraft:extended_view" => Self::ExtendedView,
            "minecraft:keybind_down" => Self::KeybindDown(string(required(m, "keybind")?)?),
            "minecraft:view_entity" => Self::ViewEntity,
            "minecraft:component" => Self::Component(component_predicate::decode(m)?),
            _ => return Err(err("unknown condition property")),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SelectProperty {
    CustomModelData(i32),
    MainHand,
    ChargeType,
    TrimMaterial,
    BlockState(String),
    DisplayContext,
    LocalTime {
        pattern: String,
        locale: String,
        time_zone: Option<String>,
    },
    ContextEntityType,
    ContextDimension,
    Component(String),
}
impl SelectProperty {
    fn decode(m: &Map<String, Value>) -> Result<Self> {
        Ok(match ident(required(m, "property")?)?.as_str() {
            "minecraft:custom_model_data" => {
                Self::CustomModelData(get(m, "index").map_or(Ok(0), non_negative)?)
            }
            "minecraft:main_hand" => Self::MainHand,
            "minecraft:charge_type" => Self::ChargeType,
            "minecraft:trim_material" => Self::TrimMaterial,
            "minecraft:block_state" => {
                Self::BlockState(string(required(m, "block_state_property")?)?)
            }
            "minecraft:display_context" => Self::DisplayContext,
            "minecraft:local_time" => Self::LocalTime {
                pattern: string(required(m, "pattern")?)?,
                locale: get(m, "locale").map_or(Ok(String::new()), string)?,
                time_zone: get(m, "time_zone").map(string).transpose()?,
            },
            "minecraft:context_entity_type" => Self::ContextEntityType,
            "minecraft:context_dimension" => Self::ContextDimension,
            "minecraft:component" => Self::Component(component_type(required(m, "component")?)?),
            _ => return Err(err("unknown select property")),
        })
    }
    /// A `when` value in canonical form (identifiers normalised).
    fn decode_value(&self, v: &Value) -> Result<Value> {
        Ok(match self {
            Self::CustomModelData(_) | Self::BlockState(_) | Self::LocalTime { .. } => {
                Value::String(string(v)?)
            }
            Self::MainHand => match string(v)?.as_str() {
                s @ ("left" | "right") => Value::String(s.to_owned()),
                _ => return Err(err("invalid arm")),
            },
            Self::ChargeType => match string(v)?.as_str() {
                s @ ("none" | "rocket" | "arrow") => Value::String(s.to_owned()),
                _ => return Err(err("invalid charge type")),
            },
            Self::TrimMaterial | Self::ContextEntityType | Self::ContextDimension => {
                Value::String(ident(v)?)
            }
            Self::DisplayContext => {
                let s = string(v)?;
                DisplayContext::by_name(&s).ok_or_else(|| err("invalid display context"))?;
                Value::String(s)
            }
            Self::Component(_) => v.clone(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompassTarget {
    None,
    Lodestone,
    Spawn,
    Recovery,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeSource {
    Random,
    Daytime,
    MoonPhase,
}
/// A `NeedleDirectionHelper.Wobbler`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Wobbler {
    pub rotation: f32,
    pub delta_rotation: f32,
    pub last_update_tick: i64,
    /// Without wobble the needle jumps straight to its target.
    pub target: f32,
}
impl Wobbler {
    pub fn should_update(&self, wobble: bool, tick: i64) -> bool {
        !wobble || self.last_update_tick != tick
    }
    pub fn update(&mut self, wobble: bool, factor: f32, tick: i64, target_rotation: f32) {
        if !wobble {
            self.target = target_rotation;
            return;
        }
        self.last_update_tick = tick;
        let temp = positive_modulo(target_rotation - self.rotation + 0.5, 1.0) - 0.5;
        self.delta_rotation += temp * 0.1;
        self.delta_rotation *= factor;
        self.rotation = positive_modulo(self.rotation + self.delta_rotation, 1.0);
    }
    pub fn value(&self, wobble: bool) -> f32 {
        if wobble { self.rotation } else { self.target }
    }
}
/// `Mth.positiveModulo(float, float)`.
pub fn positive_modulo(input: f32, modulus: f32) -> f32 {
    (input % modulus + modulus) % modulus
}
/// The mutable state a baked `compass` or `time` property keeps.
#[derive(Debug, Default)]
pub struct NeedleState(Mutex<[Wobbler; 2]>);
impl Clone for NeedleState {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl PartialEq for NeedleState {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum NumericProperty {
    BundleFullness,
    Compass {
        wobble: bool,
        target: CompassTarget,
        state: Arc<NeedleState>,
    },
    Cooldown,
    Count {
        normalize: bool,
    },
    CrossbowPull,
    CustomModelData(i32),
    Damage {
        normalize: bool,
    },
    Time {
        wobble: bool,
        source: TimeSource,
        state: Arc<NeedleState>,
    },
    UseCycle(f32),
    UseDuration {
        remaining: bool,
    },
}
impl NumericProperty {
    fn decode(m: &Map<String, Value>) -> Result<Self> {
        let normalize = || get(m, "normalize").map_or(Ok(true), boolean);
        Ok(match ident(required(m, "property")?)?.as_str() {
            "minecraft:custom_model_data" => {
                Self::CustomModelData(get(m, "index").map_or(Ok(0), non_negative)?)
            }
            "minecraft:bundle/fullness" => Self::BundleFullness,
            "minecraft:damage" => Self::Damage {
                normalize: normalize()?,
            },
            "minecraft:cooldown" => Self::Cooldown,
            "minecraft:time" => Self::Time {
                wobble: get(m, "wobble").map_or(Ok(true), boolean)?,
                source: match string(required(m, "source")?)?.as_str() {
                    "random" => TimeSource::Random,
                    "daytime" => TimeSource::Daytime,
                    "moon_phase" => TimeSource::MoonPhase,
                    _ => return Err(err("invalid time source")),
                },
                state: Arc::default(),
            },
            "minecraft:compass" => Self::Compass {
                wobble: get(m, "wobble").map_or(Ok(true), boolean)?,
                target: match string(required(m, "target")?)?.as_str() {
                    "none" => CompassTarget::None,
                    "lodestone" => CompassTarget::Lodestone,
                    "spawn" => CompassTarget::Spawn,
                    "recovery" => CompassTarget::Recovery,
                    _ => return Err(err("invalid compass target")),
                },
                state: Arc::default(),
            },
            "minecraft:crossbow/pull" => Self::CrossbowPull,
            "minecraft:use_cycle" => {
                let period = get(m, "period").map_or(Ok(1.0), float)?;
                if period.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
                    return Err(err("value must be positive"));
                }
                Self::UseCycle(period)
            }
            "minecraft:use_duration" => Self::UseDuration {
                remaining: get(m, "remaining").map_or(Ok(false), boolean)?,
            },
            "minecraft:count" => Self::Count {
                normalize: normalize()?,
            },
            _ => return Err(err("unknown range dispatch property")),
        })
    }
}

// ---------------------------------------------------------------------------
// Stacks and the world around them
// ---------------------------------------------------------------------------

/// The item components a stack carries: a prototype plus an explicit patch
/// (`None` removes a prototype component).
pub type Components = BTreeMap<String, Value>;

/// An item stack as the resolver reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StackView {
    pub item: String,
    pub count: i32,
    pub patch: BTreeMap<String, Option<Value>>,
    /// `BundleContents.selectedItem`, a runtime-only index (-1: none).
    pub bundle_selected: i32,
}
impl StackView {
    pub fn new(item: &str, count: i32) -> Self {
        Self {
            item: item.to_owned(),
            count,
            patch: BTreeMap::new(),
            bundle_selected: -1,
        }
    }
}

/// `ArmorTrim`/`LodestoneTracker`-style positions are plain data.
#[derive(Clone, Debug, PartialEq)]
pub struct GlobalPos {
    pub dimension: String,
    pub pos: [i32; 3],
}
/// The living entity holding the item (`ItemOwner`).
#[derive(Clone, Debug, Default)]
pub struct OwnerState {
    pub position: [f64; 3],
    pub visual_rotation_y_degrees: f32,
    pub entity_type: String,
    pub main_arm_left: bool,
    /// `isUsingItem() && getUseItem() == stack`.
    pub using_this_item: bool,
    pub use_item_remaining_ticks: i32,
    /// `stack.getUseDuration(owner)`.
    pub use_duration: i32,
    /// `CrossbowItem.getChargeDuration(stack, owner)`.
    pub crossbow_charge_duration: i32,
    pub fishing_cast_with_this_item: bool,
    pub is_selected: bool,
    pub is_carried: bool,
    pub is_view_entity: bool,
    pub team_color: Option<i32>,
    /// `ItemCooldowns.getCooldownPercent(stack, 0)`.
    pub cooldown_percent: f32,
    pub is_local_player: bool,
    pub last_death_location: Option<GlobalPos>,
}
/// The `ClientLevel` an item is drawn in.
#[derive(Clone, Debug, Default)]
pub struct LevelState {
    pub dimension: String,
    pub game_time: i64,
    /// `EnvironmentAttributes.SUN_ANGLE` at the owner, in degrees.
    pub sun_angle: f32,
    /// `EnvironmentAttributes.MOON_PHASE` index at the owner.
    pub moon_phase: i32,
    pub respawn: Option<GlobalPos>,
    /// `tickRateManager().runsNormally()`.
    pub runs_normally: bool,
}
/// Everything about the surroundings that is not in the stack.
#[derive(Clone, Debug)]
pub struct Context<'a> {
    pub display: DisplayContext,
    pub owner: Option<&'a OwnerState>,
    pub level: Option<&'a LevelState>,
    pub seed: i32,
}

/// Host services the resolver cannot derive from data alone.
pub trait ItemEnv {
    /// `PotionContents.getColorOr` for the potion/effects part: the mixed
    /// effect colour, or `None` when the contents have no visible effects.
    fn potion_effect_color(&self, _potion_contents: &Value) -> Option<i32> {
        None
    }
    /// `GrassColor.get(temperature, downfall)`.
    fn grass_color(&self, _temperature: f64, _downfall: f64) -> i32 {
        -65281
    }
    /// `BundleItem.getFullnessDisplay`.
    fn bundle_fullness(&self, _stack: &StackView, _components: &Components) -> f32 {
        0.0
    }
    /// Additional component predicate types supplied by the host.
    fn component_matches(&self, _stack: &StackView, _predicate: &Value) -> bool {
        false
    }
    /// Membership in the level's enchantment registry tags.
    fn enchantment_in_tag(&self, _enchantment: &str, _tag: &str) -> bool {
        false
    }
    fn is_keybind_down(&self, _keybind: &str) -> bool {
        false
    }
    fn has_shift_down(&self) -> bool {
        false
    }
    /// `LocalTime`: the formatted current time for a pattern.
    fn local_time(&self, _pattern: &str, _locale: &str, _time_zone: Option<&str>) -> String {
        String::new()
    }
    /// A random float for `time` (random source) and the spinning compass.
    fn random_float(&self) -> f32 {
        0.0
    }
}
/// An [`ItemEnv`] with the trait defaults.
pub struct NoEnv;
impl ItemEnv for NoEnv {}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoilType {
    None,
    Standard,
    Special,
}
#[derive(Clone, Debug, PartialEq)]
pub enum LayerContent {
    /// A cuboid model resource, baked by [`crate::item_bake`].
    Model { model: String },
    /// `MissingItemModel`.
    Missing,
    Special {
        base: String,
        model: SpecialModel,
        /// What `extractArgument` reads from the stack.
        argument: Value,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub content: LayerContent,
    pub tints: Vec<i32>,
    pub foil: FoilType,
    pub transform: Mat4,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResolvedItem {
    pub layers: Vec<Layer>,
    pub oversized_in_gui: bool,
}

/// All item definitions and the default components of every item.
#[derive(Default)]
pub struct ItemModels {
    pub definitions: BTreeMap<String, ClientItem>,
    pub prototypes: BTreeMap<String, Components>,
}

impl ItemModels {
    /// The stack's components: its item's defaults with the stack's patch on top. A stack
    /// with no patch (most stacks a screen draws) borrows the defaults instead of copying them.
    pub fn components(&self, stack: &StackView) -> std::borrow::Cow<'_, Components> {
        static NONE: Components = BTreeMap::new();
        let prototype = self.prototypes.get(&stack.item).unwrap_or(&NONE);
        if stack.patch.is_empty() {
            return std::borrow::Cow::Borrowed(prototype);
        }
        let mut out = prototype.clone();
        for (key, value) in &stack.patch {
            match value {
                Some(v) => out.insert(key.clone(), v.clone()),
                None => out.remove(key),
            };
        }
        std::borrow::Cow::Owned(out)
    }
    /// `ItemModelResolver.updateForTopItem`.
    pub fn resolve(&self, stack: &StackView, ctx: &Context, env: &dyn ItemEnv) -> ResolvedItem {
        let mut out = ResolvedItem::default();
        if stack.count > 0 && stack.item != "minecraft:air" {
            self.append_layers(&mut out, stack, ctx, env);
        }
        out
    }
    /// `ItemModelResolver.appendItemLayers`.
    pub fn append_layers(
        &self,
        out: &mut ResolvedItem,
        stack: &StackView,
        ctx: &Context,
        env: &dyn ItemEnv,
    ) {
        let components = self.components(stack);
        let Some(Value::String(model_id)) = components.get("minecraft:item_model") else {
            return;
        };
        let id = identifier(model_id).unwrap_or_else(|_| model_id.clone());
        let definition = self.definitions.get(&id);
        out.oversized_in_gui = definition.is_some_and(|d| d.oversized_in_gui);
        let mut state = Resolve {
            models: self,
            stack,
            components: &components,
            ctx,
            env,
        };
        match definition {
            Some(d) => state.node(out, &d.model, &Mat4::IDENTITY),
            None => state.missing(out, &Mat4::IDENTITY),
        }
    }
    /// `ItemModelResolver.shouldPlaySwapAnimation`.
    pub fn hand_animation_on_swap(&self, stack: &StackView) -> bool {
        self.properties(stack).0
    }
    /// `ItemModelResolver.swapAnimationScale`.
    pub fn swap_animation_scale(&self, stack: &StackView) -> f32 {
        self.properties(stack).2
    }
    fn properties(&self, stack: &StackView) -> (bool, bool, f32) {
        match self.components(stack).get("minecraft:item_model") {
            Some(Value::String(id)) => identifier(id)
                .ok()
                .and_then(|id| self.definitions.get(&id))
                .map_or(ClientItem::DEFAULT_PROPERTIES, |d| {
                    (
                        d.hand_animation_on_swap,
                        d.oversized_in_gui,
                        d.swap_animation_scale,
                    )
                }),
            _ => ClientItem::DEFAULT_PROPERTIES,
        }
    }
}

struct Resolve<'a> {
    models: &'a ItemModels,
    stack: &'a StackView,
    components: &'a Components,
    ctx: &'a Context<'a>,
    env: &'a dyn ItemEnv,
}

fn compose(parent: &Mat4, own: &Option<Mat4>) -> Mat4 {
    own.as_ref().map_or(*parent, |t| parent.mul(t))
}

impl Resolve<'_> {
    fn component(&self, key: &str) -> Option<&Value> {
        self.components.get(key)
    }
    fn has_foil(&self) -> bool {
        if let Some(v) = self.component("minecraft:enchantment_glint_override") {
            return v.as_bool().unwrap_or(false);
        }
        let enchanted = self
            .component("minecraft:enchantments")
            .is_some_and(enchantments_present);
        enchanted
            || (self.stack.item == "minecraft:compass"
                && self.component("minecraft:lodestone_tracker").is_some())
    }
    fn damageable(&self) -> bool {
        self.component("minecraft:max_damage").is_some()
            && self.component("minecraft:unbreakable").is_none()
            && self.component("minecraft:damage").is_some()
    }
    fn max_damage(&self) -> i32 {
        self.component("minecraft:max_damage")
            .and_then(|v| int(v).ok())
            .unwrap_or(0)
    }
    fn damage_value(&self) -> i32 {
        let d = self
            .component("minecraft:damage")
            .and_then(|v| int(v).ok())
            .unwrap_or(0);
        // `Mth.clamp(int, 0, max)`
        if d < 0 { 0 } else { d.min(self.max_damage()) }
    }
    fn max_stack_size(&self) -> i32 {
        self.component("minecraft:max_stack_size")
            .and_then(|v| int(v).ok())
            .unwrap_or(1)
    }
    fn missing(&self, out: &mut ResolvedItem, transform: &Mat4) {
        out.layers.push(Layer {
            content: LayerContent::Missing,
            tints: Vec::new(),
            foil: FoilType::None,
            transform: *transform,
        });
    }
    fn node(&mut self, out: &mut ResolvedItem, node: &ItemModelNode, parent: &Mat4) {
        match node {
            ItemModelNode::Empty => {}
            ItemModelNode::Model(leaf) => {
                let transform = compose(parent, &leaf.transformation);
                let foil = if self.has_foil() {
                    if matches!(self.stack.item.as_str(), "minecraft:clock")
                        || self.item_in_compass_tag()
                    {
                        FoilType::Special
                    } else {
                        FoilType::Standard
                    }
                } else {
                    FoilType::None
                };
                let tints = leaf.tints.iter().map(|t| self.tint(t)).collect();
                out.layers.push(Layer {
                    content: LayerContent::Model {
                        model: leaf.model.clone(),
                    },
                    tints,
                    foil,
                    transform,
                });
            }
            ItemModelNode::Composite {
                models,
                transformation,
            } => {
                let child = compose(parent, transformation);
                for m in models {
                    self.node(out, m, &child);
                }
            }
            ItemModelNode::Condition {
                transformation,
                property,
                on_true,
                on_false,
            } => {
                let child = compose(parent, transformation);
                let chosen = if self.condition(property) {
                    on_true
                } else {
                    on_false
                };
                self.node(out, chosen, &child);
            }
            ItemModelNode::Select {
                transformation,
                property,
                cases,
                fallback,
            } => {
                let child = compose(parent, transformation);
                let value = self.select_value(property);
                let chosen = value.and_then(|v| {
                    cases
                        .iter()
                        .find(|c| c.values.contains(&v))
                        .map(|c| &c.model)
                });
                match (chosen, fallback) {
                    (Some(m), _) => self.node(out, m, &child),
                    (None, Some(f)) => self.node(out, f, &child),
                    (None, None) => self.missing(out, &child),
                }
            }
            ItemModelNode::RangeDispatch {
                transformation,
                property,
                scale,
                entries,
                fallback,
            } => {
                let child = compose(parent, transformation);
                let value = self.numeric(property) * scale;
                let chosen = if value.is_nan() {
                    None
                } else {
                    last_index_less_or_equal(entries, value).map(|i| &entries[i].1)
                };
                match (chosen, fallback) {
                    (Some(m), _) => self.node(out, m, &child),
                    (None, Some(f)) => self.node(out, f, &child),
                    (None, None) => self.missing(out, &child),
                }
            }
            ItemModelNode::Special {
                base,
                transformation,
                model,
            } => {
                let transform = compose(parent, transformation);
                out.layers.push(Layer {
                    content: LayerContent::Special {
                        base: base.clone(),
                        model: model.clone(),
                        argument: model.argument(&|k| self.component(k).cloned()),
                    },
                    tints: Vec::new(),
                    foil: if self.has_foil() {
                        FoilType::Standard
                    } else {
                        FoilType::None
                    },
                    transform,
                });
            }
            ItemModelNode::BundleSelectedItem => {
                if let Some(selected) = self.bundle_selected() {
                    self.models
                        .append_layers(out, &selected, self.ctx, self.env);
                }
            }
        }
    }
    fn item_in_compass_tag(&self) -> bool {
        // `ItemTags.COMPASSES`: the compass and the recovery compass.
        matches!(
            self.stack.item.as_str(),
            "minecraft:compass" | "minecraft:recovery_compass"
        )
    }
    /// `BundleItem.getSelectedItem`: the template at the selected index.
    fn bundle_selected(&self) -> Option<StackView> {
        let items = self.component("minecraft:bundle_contents")?.as_array()?;
        let template = items.get(usize::try_from(self.stack.bundle_selected).ok()?)?;
        stack_from_template(template)
    }
    fn condition(&self, property: &ConditionProperty) -> bool {
        let owner = self.ctx.owner;
        match property {
            ConditionProperty::CustomModelData(i) => {
                self.component("minecraft:custom_model_data")
                    .and_then(|d| custom_data_at(d, "flags", *i))
                    .and_then(|v| v.as_bool())
                    == Some(true)
            }
            ConditionProperty::UsingItem => owner.is_some_and(|o| o.using_this_item),
            ConditionProperty::Broken => {
                self.damageable() && self.damage_value() >= self.max_damage() - 1
            }
            ConditionProperty::Damaged => self.damageable() && self.damage_value() > 0,
            ConditionProperty::FishingRodCast => {
                owner.is_some_and(|o| o.fishing_cast_with_this_item)
            }
            ConditionProperty::HasComponent {
                component,
                ignore_default,
            } => {
                if *ignore_default {
                    self.stack.patch.contains_key(component)
                } else {
                    self.components.contains_key(component)
                }
            }
            ConditionProperty::BundleHasSelectedItem => self.bundle_selected().is_some(),
            ConditionProperty::Selected => {
                owner.is_some_and(|o| o.is_local_player && o.is_selected)
            }
            ConditionProperty::Carried => owner.is_some_and(|o| o.is_local_player && o.is_carried),
            ConditionProperty::ExtendedView => {
                self.ctx.display == DisplayContext::Gui && self.env.has_shift_down()
            }
            ConditionProperty::KeybindDown(key) => self.env.is_keybind_down(key),
            ConditionProperty::ViewEntity => owner.is_some_and(|o| o.is_view_entity),
            ConditionProperty::Component(predicate) => {
                component_predicate::matches(predicate, self.components, self.env)
                    .unwrap_or_else(|| self.env.component_matches(self.stack, predicate))
            }
        }
    }
    fn select_value(&self, property: &SelectProperty) -> Option<Value> {
        let owner = self.ctx.owner;
        match property {
            SelectProperty::CustomModelData(i) => self
                .component("minecraft:custom_model_data")
                .and_then(|d| custom_data_at(d, "strings", *i))
                .cloned(),
            SelectProperty::MainHand => owner
                .map(|o| Value::String(if o.main_arm_left { "left" } else { "right" }.to_owned())),
            SelectProperty::ChargeType => {
                let projectiles = self
                    .component("minecraft:charged_projectiles")
                    .and_then(Value::as_array);
                let kind = match projectiles {
                    None => "none",
                    Some(p) if p.is_empty() => "none",
                    Some(p) => {
                        if p.iter()
                            .any(|t| template_id(t) == Some("minecraft:firework_rocket"))
                        {
                            "rocket"
                        } else {
                            "arrow"
                        }
                    }
                };
                Some(Value::String(kind.to_owned()))
            }
            SelectProperty::TrimMaterial => {
                let material = self
                    .component("minecraft:trim")?
                    .get("material")?
                    .as_str()?;
                identifier(material).ok().map(Value::String)
            }
            SelectProperty::BlockState(name) => self
                .component("minecraft:block_state")
                .and_then(|v| v.get("properties").or(Some(v)))
                .and_then(|p| p.get(name))
                .map(|v| Value::String(v.as_str().map_or_else(|| v.to_string(), str::to_owned))),
            SelectProperty::DisplayContext => {
                Some(Value::String(self.ctx.display.name().to_owned()))
            }
            SelectProperty::LocalTime {
                pattern,
                locale,
                time_zone,
            } => Some(Value::String(self.env.local_time(
                pattern,
                locale,
                time_zone.as_deref(),
            ))),
            SelectProperty::ContextEntityType => {
                owner.map(|o| Value::String(o.entity_type.clone()))
            }
            SelectProperty::ContextDimension => {
                self.ctx.level.map(|l| Value::String(l.dimension.clone()))
            }
            SelectProperty::Component(component) => self.component(component).cloned(),
        }
    }
    fn numeric(&self, property: &NumericProperty) -> f32 {
        let owner = self.ctx.owner;
        match property {
            NumericProperty::BundleFullness => {
                self.env.bundle_fullness(self.stack, self.components)
            }
            NumericProperty::Compass {
                wobble,
                target,
                state,
            } => self.compass(*wobble, *target, state),
            NumericProperty::Cooldown => owner.map_or(0.0, |o| o.cooldown_percent),
            NumericProperty::Count { normalize } => {
                let count = self.stack.count as f32;
                let max = self.max_stack_size() as f32;
                if *normalize {
                    clamp(count / max, 0.0, 1.0)
                } else {
                    clamp(count, 0.0, max)
                }
            }
            NumericProperty::CrossbowPull => {
                let Some(o) = owner else { return 0.0 };
                if self
                    .component("minecraft:charged_projectiles")
                    .and_then(Value::as_array)
                    .is_some_and(|p| !p.is_empty())
                {
                    return 0.0;
                }
                (o.use_duration - o.use_item_remaining_ticks) as f32
                    / o.crossbow_charge_duration as f32
            }
            NumericProperty::CustomModelData(i) => self
                .component("minecraft:custom_model_data")
                .and_then(|d| custom_data_at(d, "floats", *i))
                .and_then(|v| float(v).ok())
                .unwrap_or(0.0),
            NumericProperty::Damage { normalize } => {
                let damage = self.damage_value() as f32;
                let max = self.max_damage() as f32;
                if *normalize {
                    clamp(damage / max, 0.0, 1.0)
                } else {
                    clamp(damage, 0.0, max)
                }
            }
            NumericProperty::Time {
                wobble,
                source,
                state,
            } => self.time(*wobble, *source, state),
            NumericProperty::UseCycle(period) => match owner {
                Some(o) if o.using_this_item => (o.use_item_remaining_ticks as f32) % period,
                _ => 0.0,
            },
            NumericProperty::UseDuration { remaining } => match owner {
                Some(o) if o.using_this_item => {
                    if *remaining {
                        o.use_item_remaining_ticks as f32
                    } else {
                        (o.use_duration - o.use_item_remaining_ticks) as f32
                    }
                }
                _ => 0.0,
            },
        }
    }
    /// `Time.calculate` (through `NeedleDirectionHelper.get`).
    fn time(&self, wobble: bool, source: TimeSource, state: &NeedleState) -> f32 {
        let (Some(_), Some(level)) = (self.ctx.owner, self.ctx.level) else {
            return 0.0;
        };
        let target = match source {
            TimeSource::Random => self.env.random_float(),
            TimeSource::Daytime => level.sun_angle / 360.0,
            TimeSource::MoonPhase => level.moon_phase as f32 / 8.0,
        };
        let mut wobblers = state.0.lock().unwrap_or_else(|e| e.into_inner());
        let w = &mut wobblers[0];
        if w.should_update(wobble, level.game_time) {
            w.update(wobble, 0.9, level.game_time, target);
        }
        w.value(wobble)
    }
    /// `CompassAngleState.calculate`.
    fn compass(&self, wobble: bool, target: CompassTarget, state: &NeedleState) -> f32 {
        let (Some(owner), Some(level)) = (self.ctx.owner, self.ctx.level) else {
            return 0.0;
        };
        let target_pos: Option<GlobalPos> = match target {
            CompassTarget::None => None,
            CompassTarget::Lodestone => self
                .component("minecraft:lodestone_tracker")
                .and_then(|t| t.get("target"))
                .and_then(|t| {
                    Some(GlobalPos {
                        dimension: identifier(t.get("dimension")?.as_str()?).ok()?,
                        pos: {
                            let p = t.get("pos")?.as_array()?;
                            [
                                int(p.first()?).ok()?,
                                int(p.get(1)?).ok()?,
                                int(p.get(2)?).ok()?,
                            ]
                        },
                    })
                }),
            CompassTarget::Spawn => level.respawn.clone(),
            CompassTarget::Recovery => owner.last_death_location.clone(),
        };
        let valid = target_pos.as_ref().filter(|t| {
            t.dimension == level.dimension && {
                let c = [
                    f64::from(t.pos[0]) + 0.5,
                    f64::from(t.pos[1]) + 0.5,
                    f64::from(t.pos[2]) + 0.5,
                ];
                let d = [
                    c[0] - owner.position[0],
                    c[1] - owner.position[1],
                    c[2] - owner.position[2],
                ];
                // `distToCenterSqr(position) < 1.0E-5F`
                let dist = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                dist.is_nan() || dist >= f64::from(1.0E-5_f32)
            }
        });
        let mut wobblers = state.0.lock().unwrap_or_else(|e| e.into_inner());
        let game_time = level.game_time;
        let Some(target) = valid else {
            // `getRandomlySpinningRotation`
            let no_target = &mut wobblers[1];
            if no_target.should_update(wobble, game_time) {
                let r = self.env.random_float();
                no_target.update(wobble, 0.8, game_time, r);
            }
            let hash = self.ctx.seed.wrapping_mul(1_327_217_883);
            let rotation = no_target.value(wobble) + hash as f32 / 2.147_483_6E9;
            return positive_modulo(rotation, 1.0);
        };
        let center = [
            f64::from(target.pos[0]) + 0.5,
            f64::from(target.pos[2]) + 0.5,
        ];
        let angle_to_target = ((center[1] - owner.position[2]).atan2(center[0] - owner.position[0])
            / f64::from(std::f32::consts::PI * 2.0)) as f32;
        let owner_y = positive_modulo(owner.visual_rotation_y_degrees / 360.0, 1.0);
        let rotation = if owner.is_local_player && level.runs_normally {
            let w = &mut wobblers[0];
            if w.should_update(wobble, game_time) {
                w.update(wobble, 0.8, game_time, 0.5 - (owner_y - 0.25));
            }
            angle_to_target + w.value(wobble)
        } else {
            0.5 - (owner_y - 0.25 - angle_to_target)
        };
        positive_modulo(rotation, 1.0)
    }
    fn tint(&self, source: &TintSource) -> i32 {
        match source {
            TintSource::Constant(v) => *v,
            TintSource::CustomModelData { index, default } => self
                .component("minecraft:custom_model_data")
                .and_then(|d| custom_data_at(d, "colors", *index))
                .and_then(|v| rgb(v).ok())
                .map_or(opaque(*default), opaque),
            TintSource::Dye(default) => match self.component("minecraft:dyed_color") {
                Some(v) => opaque(rgb(v.get("rgb").unwrap_or(v)).unwrap_or(0)),
                None => *default,
            },
            TintSource::Grass {
                temperature,
                downfall,
            } => self
                .env
                .grass_color(f64::from(*temperature), f64::from(*downfall)),
            TintSource::Firework(default) => {
                let colors: Vec<i32> = self
                    .component("minecraft:firework_explosion")
                    .and_then(|e| e.get("colors"))
                    .and_then(Value::as_array)
                    .map(|c| c.iter().filter_map(|v| int(v).ok()).collect())
                    .unwrap_or_default();
                match colors.len() {
                    0 => *default,
                    1 => opaque(colors[0]),
                    n => {
                        let n = n as i32;
                        let sum = |shift: u32| -> i32 {
                            colors.iter().map(|c| (c >> shift) & 255).sum::<i32>() / n
                        };
                        argb(255, sum(16), sum(8), sum(0))
                    }
                }
            }
            TintSource::Potion(default) => match self.component("minecraft:potion_contents") {
                Some(contents) => {
                    let custom = contents.get("custom_color").and_then(|c| int(c).ok());
                    opaque(
                        custom
                            .or_else(|| self.env.potion_effect_color(contents))
                            .unwrap_or(*default),
                    )
                }
                None => opaque(*default),
            },
            TintSource::Team(default) => opaque(
                self.ctx
                    .owner
                    .and_then(|o| o.team_color)
                    .unwrap_or(*default),
            ),
        }
    }
}

/// `Mth.clamp(float, min, max)` with `Math.min`'s NaN propagation.
fn clamp(v: f32, min: f32, max: f32) -> f32 {
    if v < min { min } else { java_min(v, max) }
}
fn java_min(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        a
    } else if b.is_nan() {
        b
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_negative() { a } else { b }
    } else if a < b {
        a
    } else {
        b
    }
}
fn enchantments_present(v: &Value) -> bool {
    // `ItemEnchantments` is a level map, optionally wrapped as {"levels": {...}}.
    let map = v.get("levels").unwrap_or(v);
    map.as_object().is_some_and(|m| !m.is_empty())
}
fn custom_data_at<'a>(data: &'a Value, list: &str, index: i32) -> Option<&'a Value> {
    data.get(list)?
        .as_array()?
        .get(usize::try_from(index).ok()?)
}
fn template_id(t: &Value) -> Option<&str> {
    t.get("id")?.as_str()
}
/// `ItemStackTemplate.create`: id, count and a component patch.
fn stack_from_template(template: &Value) -> Option<StackView> {
    let id = identifier(template.get("id")?.as_str()?).ok()?;
    let count = template.get("count").map_or(Some(1), |c| int(c).ok())?;
    let mut stack = StackView::new(&id, count);
    if let Some(components) = template.get("components").and_then(Value::as_object) {
        for (key, value) in components {
            match key.strip_prefix('!') {
                Some(removed) => {
                    stack.patch.insert(identifier(removed).ok()?, None);
                }
                None => {
                    stack
                        .patch
                        .insert(identifier(key).ok()?, Some(value.clone()));
                }
            }
        }
    }
    Some(stack)
}

/// `RangeSelectItemModel.lastIndexLessOrEqual`.
fn last_index_less_or_equal(entries: &[(f32, ItemModelNode)], needle: f32) -> Option<usize> {
    let haystack: Vec<f32> = entries.iter().map(|e| e.0).collect();
    let found = if haystack.len() < 16 {
        haystack
            .iter()
            .position(|h| *h > needle)
            .map_or(haystack.len() as i64 - 1, |i| i as i64 - 1)
    } else {
        match binary_search(&haystack, needle) {
            Ok(index) => index as i64,
            Err(insertion) => insertion as i64 - 1,
        }
    };
    usize::try_from(found).ok()
}
/// `Arrays.binarySearch(float[], float)`, including its handling of equal
/// values (-0.0 below 0.0, NaN above everything).
fn binary_search(a: &[f32], key: f32) -> std::result::Result<usize, usize> {
    let (mut low, mut high) = (0i64, a.len() as i64 - 1);
    let bits = |v: f32| -> i32 {
        if v.is_nan() {
            0x7fc0_0000
        } else {
            v.to_bits() as i32
        }
    };
    while low <= high {
        let mid = (low + high) >> 1;
        let value = a[mid as usize];
        if value < key {
            low = mid + 1;
        } else if value > key {
            high = mid - 1;
        } else {
            let (mid_bits, key_bits) = (bits(value), bits(key));
            if mid_bits == key_bits {
                return Ok(mid as usize);
            } else if mid_bits < key_bits {
                low = mid + 1;
            } else {
                high = mid - 1;
            }
        }
    }
    Err(low as usize)
}

#[cfg(test)]
mod tests;
