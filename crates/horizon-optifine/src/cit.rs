//! Custom Item Textures: the rules of `optifine/cit/**/*.properties` and the pack's global
//! `cit.properties`, parsed into what they test on a stack and what they change. Matching stacks
//! against them is the client's (`horizon_client::cit`): it knows the items and their components.
//!
//! Written from OptiFine's `cit.properties` and `cit_single.properties` docs, CIT Resewn's wiki
//! (citresewn.shcm.io) and CIT Resewn Continuation's MIT-licensed source (its conditions, read
//! for the 1.21 component rules). Where OptiFine and CIT Resewn differ, CIT Resewn is followed,
//! but for keys neither reads: those are ignored, as OptiFine does, whereas CIT Resewn would
//! never match the rule.

use crate::matchers::{IdList, Name, RangeList, namespaced};
use crate::paths::{resolve, valid};
use crate::properties::Properties;

/// What a rule changes (`type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Item,
    Armor,
    Elytra,
    Enchantment,
}

/// `hand`: the hand the item is drawn in; GUI items count as the main hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hand {
    Any,
    Main,
    Off,
}

/// `damage` (with `%` against the item's max damage) and `damageMask`.
#[derive(Clone, Debug, PartialEq)]
pub struct Damage {
    pub ranges: RangeList,
    pub percent: bool,
    pub mask: Option<i32>,
}

impl Damage {
    pub fn matches(&self, damage: u32, max_damage: u32) -> bool {
        let mut value = damage as i32;
        if let Some(mask) = self.mask {
            value &= mask;
        }
        if !self.percent {
            return self.ranges.contains(value);
        }
        if max_damage == 0 {
            return false;
        }
        let percent = 100.0 * f64::from(value) / f64::from(max_damage);
        self.ranges
            .0
            .iter()
            .any(|&(lo, hi)| f64::from(lo) <= percent && percent <= f64::from(hi))
    }
}

/// An OpenGL blend factor (`GL_ONE`, `GL_SRC_ALPHA`...), as `glBlendFuncSeparate` takes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Factor {
    Zero,
    One,
    SrcColor,
    OneMinusSrcColor,
    SrcAlpha,
    OneMinusSrcAlpha,
    DstAlpha,
    OneMinusDstAlpha,
    DstColor,
    OneMinusDstColor,
    SrcAlphaSaturate,
}

impl Factor {
    /// A GL constant's name (`GL_DST_COLOR`) or its value, decimal or `0x` hex.
    fn parse(s: &str) -> Option<Self> {
        let code = match s.strip_prefix("0x") {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => s.parse().ok(),
        };
        let code = code.or_else(|| {
            let names = [
                "GL_ZERO",
                "GL_ONE",
                "GL_SRC_COLOR",
                "GL_ONE_MINUS_SRC_COLOR",
                "GL_SRC_ALPHA",
                "GL_ONE_MINUS_SRC_ALPHA",
                "GL_DST_ALPHA",
                "GL_ONE_MINUS_DST_ALPHA",
                "GL_DST_COLOR",
                "GL_ONE_MINUS_DST_COLOR",
                "GL_SRC_ALPHA_SATURATE",
            ];
            let i = names.iter().position(|n| *n == s)? as u32;
            Some(if i < 2 { i } else { 0x300 + i - 2 })
        })?;
        use Factor::*;
        Some(match code {
            0 => Zero,
            1 => One,
            0x300 => SrcColor,
            0x301 => OneMinusSrcColor,
            0x302 => SrcAlpha,
            0x303 => OneMinusSrcAlpha,
            0x304 => DstAlpha,
            0x305 => OneMinusDstAlpha,
            0x306 => DstColor,
            0x307 => OneMinusDstColor,
            0x308 => SrcAlphaSaturate,
            _ => return None,
        })
    }
}

/// `blend`: the factors for colour and alpha; `None` (`replace`) draws without blending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Blend {
    pub src: Factor,
    pub dst: Factor,
    pub src_alpha: Factor,
    pub dst_alpha: Factor,
}

/// `blend=` as CIT Resewn reads it: a named method, or 2 or 4 GL factors (alpha defaulting to
/// `GL_ZERO GL_ONE`). `Some(None)` is `replace`.
pub fn parse_blend(value: &str) -> Option<Option<Blend>> {
    use Factor::*;
    let pair = |src, dst| {
        Some(Some(Blend {
            src,
            dst,
            src_alpha: Zero,
            dst_alpha: One,
        }))
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "replace" => return Some(None),
        "glint" => return pair(SrcColor, One),
        "alpha" => return pair(SrcAlpha, OneMinusSrcAlpha),
        "add" => return pair(SrcAlpha, One),
        "subtract" => return pair(OneMinusDstColor, Zero),
        "multiply" => return pair(DstColor, OneMinusSrcAlpha),
        "dodge" => return pair(One, One),
        "burn" => return pair(Zero, OneMinusSrcColor),
        "screen" => return pair(One, OneMinusSrcColor),
        "overlay" => return pair(DstColor, SrcColor),
        _ => {}
    }
    let factors: Option<Vec<Factor>> = value.split_whitespace().map(Factor::parse).collect();
    match factors?[..] {
        [src, dst] => pair(src, dst),
        [src, dst, src_alpha, dst_alpha] => Some(Some(Blend {
            src,
            dst,
            src_alpha,
            dst_alpha,
        })),
        _ => None,
    }
}

/// A `type=enchantment` rule's glint layer.
#[derive(Clone, Debug, PartialEq)]
pub struct Glint {
    pub layer: i32,
    pub blend: Option<Blend>,
    /// Scroll speed: the texture runs through vanilla's glint periods at `speed` times the
    /// clock in milliseconds (vanilla's own glint at 4 by default).
    pub speed: f32,
    /// Degrees the texture turns by (as in CIT Resewn, its scroll keeps vanilla's direction).
    pub rotation: f32,
    /// Seconds on this layer when `method=cycle` (1 by default, as OptiFine's doc has it).
    pub duration: f32,
    /// `r`, `g`, `b`, `a`: multipliers of the texture's colour, never negative.
    pub rgba: [f32; 4],
    /// Whether the default glint still draws under this layer.
    pub use_glint: bool,
    /// Linear filtering, else nearest.
    pub blur: bool,
}

/// OptiFine's defaults (`blur`, which it does not document, is CIT Resewn's): still, unturned,
/// a second per cycle, added on top.
impl Default for Glint {
    fn default() -> Self {
        Self {
            layer: 0,
            blend: parse_blend("add").flatten(),
            speed: 0.0,
            rotation: 0.0,
            duration: 1.0,
            rgba: [1.0; 4],
            use_glint: false,
            blur: true,
        }
    }
}

impl Glint {
    /// A `type=enchantment` rule's glint keys; `None` (dropping the rule) when one is
    /// malformed. CIT Resewn clamps `duration` and the colour at 0.
    fn parse(props: &Properties) -> Option<Self> {
        let d = Self::default();
        let float =
            |key: &str, default: f32| optional(props, &[key], finite).map(|v| v.unwrap_or(default));
        let flag = |key: &str, default: bool| {
            props
                .get(key)
                .map_or(default, |v| v.trim().eq_ignore_ascii_case("true"))
        };
        let blend = match props.get("blend") {
            Some(v) => parse_blend(v)?,
            None => d.blend,
        };
        Some(Self {
            layer: optional(props, &["layer"], |v| v.parse::<i32>().ok())?.unwrap_or(d.layer),
            blend,
            speed: float("speed", d.speed)?,
            rotation: float("rotation", d.rotation)?,
            duration: float("duration", d.duration)?.max(0.0),
            rgba: [
                float("r", 1.0)?.max(0.0),
                float("g", 1.0)?.max(0.0),
                float("b", 1.0)?.max(0.0),
                float("a", 1.0)?.max(0.0),
            ],
            use_glint: flag("useGlint", d.use_glint),
            blur: flag("blur", d.blur),
        })
    }
}

/// A number, where NaN and the infinities (`nan`, `inf`, `Infinity` in any case) are malformed.
fn finite(v: &str) -> Option<f32> {
    v.trim().parse::<f32>().ok().filter(|f| f.is_finite())
}

/// Where a component test reads its value.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// A data component by id (`minecraft:custom_name`), then a path inside its NBT.
    Component(String),
    /// Pre-1.20.5 `Enchantments` / `StoredEnchantments`: a list of `{id, lvl}` compounds, which
    /// the client builds from the stack's enchantments.
    LegacyEnchantments { stored: bool },
}

/// One `components.<id>.<path>=<value>` or `nbt.<path>=<value>` test.
#[derive(Clone, Debug)]
pub struct ComponentTest {
    pub source: Source,
    /// Compound keys and list indexes; `*` is any key or element, `count` a list's length. The
    /// tag walked decides which a segment is: `0` is an index in a list, a key in a compound.
    pub path: Vec<String>,
    pub value: ValueTest,
    /// A leading `!`: the test passes when the value does not match.
    pub negate: bool,
}

/// The right-hand side of a component test.
#[derive(Clone, Debug)]
pub enum ValueTest {
    /// `exists:true` / `exists:false`.
    Exists(bool),
    /// `range:1 5 7-9`: an integer value.
    Range(RangeList),
    /// `raw:<name>`: the value written as SNBT.
    Raw(Name),
    /// `pattern:`, `regex:` (and their `i` forms) against a string, a text component's plain
    /// text, or a number's decimal form.
    Text(Name),
    /// A bare value: the same string, the same number (`#rrggbb` and `0x` hex are integers), or
    /// for a compound or list, SNBT that the value contains.
    Literal(Literal),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Literal {
    pub text: String,
    pub int: Option<i64>,
    pub float: Option<f64>,
}

impl Literal {
    fn parse(text: &str) -> Self {
        let hex = text
            .strip_prefix('#')
            .or_else(|| text.strip_prefix("0x"))
            .and_then(|h| i64::from_str_radix(h, 16).ok());
        Self {
            text: text.to_owned(),
            int: hex.or_else(|| text.parse().ok()),
            float: text.parse().ok(),
        }
    }
}

impl ValueTest {
    fn parse(value: &str) -> Option<(Self, bool)> {
        let (negate, value) = match value.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, value),
        };
        let test = if let Some(v) = value.strip_prefix("exists:") {
            ValueTest::Exists(v.trim().parse().ok()?)
        } else if let Some(v) = value.strip_prefix("range:") {
            ValueTest::Range(RangeList::parse(v)?)
        } else if let Some(v) = value.strip_prefix("raw:") {
            ValueTest::Raw(Name::parse(v)?)
        } else if ["pattern:", "ipattern:", "regex:", "iregex:"]
            .iter()
            .any(|p| value.starts_with(p))
        {
            ValueTest::Text(Name::parse(value)?)
        } else {
            ValueTest::Literal(Literal::parse(value))
        };
        Some((test, negate))
    }
}

impl ComponentTest {
    /// `key` is the part after `nbt.` or `components.`.
    fn parse(legacy: bool, key: &str, value: &str) -> Option<Self> {
        // `~` is the minecraft namespace where it starts a segment (`levels.~thorns`).
        let mut segments: Vec<String> = key
            .split('.')
            .map(|s| match s.strip_prefix('~') {
                Some(rest) => format!("minecraft:{rest}"),
                None => s.to_owned(),
            })
            .collect();
        if segments.iter().any(String::is_empty) {
            return None;
        }
        let (source, mut path) = if legacy {
            legacy_source(segments)?
        } else {
            let id = namespaced(&segments.remove(0));
            (Source::Component(id), segments)
        };
        // Enchantments were `{levels: {id: level}}` until 1.21.5 and are the bare map since, as
        // OptiFine's doc still writes them (`components.enchantments.levels.~thorns`).
        if let Source::Component(id) = &source {
            if matches!(
                &**id,
                "minecraft:enchantments" | "minecraft:stored_enchantments"
            ) && path.first().is_some_and(|s| s == "levels")
            {
                path.remove(0);
            }
        }
        let (value, negate) = ValueTest::parse(value)?;
        Some(Self {
            source,
            path,
            value,
            negate,
        })
    }
}

/// Where a pre-1.20.5 `nbt.` path lives since items moved to data components. Tags that became
/// no component of their own were moved into `minecraft:custom_data` by the game's data fixer,
/// so paths outside the tags 1.20.5 knew are read there. `None` for a tag that became a component
/// whose shape no longer matches the old one (`AttributeModifiers`, `pages`...): such a test
/// would never pass, or always pass when negated.
fn legacy_source(path: Vec<String>) -> Option<(Source, Vec<String>)> {
    let component = |id: &str, prefix: &[&str], rest: &[String]| {
        let path = prefix.iter().map(|s| (*s).to_owned());
        Some((
            Source::Component(format!("minecraft:{id}")),
            path.chain(rest.iter().cloned()).collect(),
        ))
    };
    let strs: Vec<&str> = path.iter().map(String::as_str).collect();
    match strs[..] {
        ["display", "Name", ..] => component("custom_name", &[], &path[2..]),
        ["display", "Lore", ..] => component("lore", &[], &path[2..]),
        ["display", "color", ..] => component("dyed_color", &[], &path[2..]),
        // `map_color` is gone in 26.x; `LocName` became a translated `item_name`.
        ["display", "MapColor" | "LocName", ..] => None,
        ["CustomModelData", ..] => component("custom_model_data", &["floats", "0"], &path[1..]),
        ["Potion", ..] => component("potion_contents", &["potion"], &path[1..]),
        ["CustomPotionColor", ..] => component("potion_contents", &["custom_color"], &path[1..]),
        ["Damage", ..] => component("damage", &[], &path[1..]),
        ["RepairCost", ..] => component("repair_cost", &[], &path[1..]),
        ["Trim", ..] => component("trim", &[], &path[1..]),
        ["title", ..] => component("written_book_content", &["title", "raw"], &path[1..]),
        ["author", ..] => component("written_book_content", &["author"], &path[1..]),
        // `generation` and `flight_duration` are left out when 0: the matcher reads that default.
        ["generation", ..] => component("written_book_content", &["generation"], &path[1..]),
        ["SkullOwner"] | ["SkullOwner", "Name", ..] => {
            component("profile", &["name"], path.get(2..).unwrap_or_default())
        }
        ["SkullOwner", "Id", ..] => component("profile", &["id"], &path[2..]),
        // `Properties.textures` became `profile.properties`, a list of `{name, value}`.
        ["SkullOwner", ..] => None,
        ["Fireworks", "Flight", ..] => component("fireworks", &["flight_duration"], &path[2..]),
        ["map", ..] => component("map_id", &[], &path[1..]),
        ["EntityTag", "variant", ..] => component("painting/variant", &[], &path[2..]),
        ["EntityTag", ..] => component("entity_data", &[], &path[1..]),
        // The 1.20.2 name of `CustomPotionEffects`, whose fields kept their names.
        ["custom_potion_effects", ..] => {
            component("potion_contents", &["custom_effects"], &path[1..])
        }
        // The data fixer took these out of `BlockEntityTag` into components of their own shape.
        [
            "BlockEntityTag",
            "Base" | "Patterns" | "Items" | "Lock" | "LootTable" | "LootTableSeed" | "sherds"
            | "item" | "Bees" | "bees" | "note_block_sound",
            ..,
        ] => None,
        ["BlockEntityTag", ..] => component("block_entity_data", &[], &path[1..]),
        ["instrument", ..] => component("instrument", &[], &path[1..]),
        ["Recipes", ..] => component("recipes", &[], &path[1..]),
        ["DebugProperty", ..] => component("debug_stick_state", &[], &path[1..]),
        ["BlockStateTag", ..] => component("block_state", &[], &path[1..]),
        ["Enchantments", ..] => Some((
            Source::LegacyEnchantments { stored: false },
            path[1..].to_vec(),
        )),
        ["StoredEnchantments", ..] => Some((
            Source::LegacyEnchantments { stored: true },
            path[1..].to_vec(),
        )),
        [
            "Unbreakable"
            | "CanDestroy"
            | "CanPlaceOn"
            | "HideFlags"
            | "AttributeModifiers"
            | "CustomPotionEffects"
            | "effects"
            | "Decorations"
            | "LodestoneDimension"
            | "LodestonePos"
            | "LodestoneTracked"
            | "pages"
            | "filtered_pages"
            | "filtered_title"
            | "resolved"
            | "Fireworks"
            | "Explosion"
            | "BucketVariantTag"
            | "Items"
            | "ChargedProjectiles"
            | "Charged"
            | "CustomCreativeLock"
            // Bucket mobs': into `bucket_entity_data`, and `Variant` became a name.
            | "Variant"
            | "NoAI"
            | "Silent"
            | "NoGravity"
            | "Glowing"
            | "Invulnerable"
            | "Health"
            | "Age"
            | "HuntingCooldown",
            ..,
        ] => None,
        _ => component("custom_data", &[], &path),
    }
}

/// One `.properties` file under `optifine/cit/`.
#[derive(Clone, Debug)]
pub struct Rule {
    /// The file's resource path: the tie-breaker between rules of equal weight.
    pub path: String,
    pub kind: Kind,
    pub weight: i32,
    /// Namespaced item ids; empty (enchantment rules only) is any item.
    pub items: Vec<String>,
    pub stack_size: Option<RangeList>,
    pub damage: Option<Damage>,
    pub hand: Hand,
    /// Namespaced enchantment ids, any of which the stack must have.
    pub enchantments: Vec<String>,
    /// Levels one of the stack's enchantments (of `enchantments`, when given) must have.
    pub enchantment_levels: Option<RangeList>,
    pub components: Vec<ComponentTest>,
    /// `texture`, as a resource path to a `.png`.
    pub texture: Option<String>,
    /// `texture.<name>`: (name, resource path).
    pub textures: Vec<(String, String)>,
    /// `model`, as a resource path to a `.json`.
    pub model: Option<String>,
    /// `model.<name>`.
    pub models: Vec<(String, String)>,
    /// What an Enchantment rule draws; the defaults for other kinds.
    pub glint: Glint,
}

impl Rule {
    /// The rule `props` (at resource path `path`, read with `Properties::parse_equals_first` as
    /// CIT Resewn reads them) describes. `exists` answers whether a resource path names a file,
    /// to find textures and models the way OptiFine and CIT Resewn look for them. `None` for an
    /// unknown type, a malformed condition, or a rule that changes nothing.
    pub fn parse(path: &str, props: &Properties, exists: impl Fn(&str) -> bool) -> Option<Self> {
        let kind = match props.get("type").unwrap_or("item") {
            "item" => Kind::Item,
            "armor" => Kind::Armor,
            "elytra" => Kind::Elytra,
            "enchantment" => Kind::Enchantment,
            _ => return None,
        };
        let stem = path
            .rsplit('/')
            .next()?
            .strip_suffix(".properties")
            .unwrap_or_default();
        let mut items: Vec<String> = props
            .get("items")
            .or_else(|| props.get("matchItems"))
            .map(|v| IdList::parse(v).include)
            .unwrap_or_default();
        if kind == Kind::Elytra {
            items = vec!["minecraft:elytra".to_owned()];
        } else if items.is_empty() && kind != Kind::Enchantment {
            // CIT Resewn: a rule named after an item is for that item (a bare id, so a
            // `minecraft:` one whatever the rule's namespace).
            items.push(namespaced(stem));
        }
        let int = |v: &str| v.parse::<i32>().ok();
        let damage = match props.get("damage") {
            Some(v) => Some(Damage {
                ranges: RangeList::parse(&v.replace('%', ""))?,
                percent: v.contains('%'),
                mask: optional(props, &["damageMask", "damage_mask"], int)?,
            }),
            None => None,
        };
        let enchantments = props
            .get("enchantments")
            .or_else(|| props.get("enchantmentIDs"))
            .map(|v| v.split_whitespace().map(enchantment_id).collect())
            .unwrap_or_default();
        let mut components = Vec::new();
        // `component.` is CIT Resewn's alias of `components.`.
        for (legacy, prefix) in [
            (true, "nbt."),
            (false, "components."),
            (false, "component."),
        ] {
            for (key, value) in props.with_prefix(prefix) {
                components.push(ComponentTest::parse(legacy, key, value)?);
            }
        }
        let tex = |v: &str| asset(path, v, "png", "textures", &exists);
        let model_at = |v: &str| asset(path, v, "json", "models", &exists);
        // A named asset that is not there drops the rule, as in CIT Resewn: half a bow's textures
        // would draw a vanilla frame among custom ones.
        let mut texture = optional(props, &["texture"], tex)?;
        let mut model = optional(props, &["model"], model_at)?;
        let textures = props
            .with_prefix("texture.")
            .map(|(name, v)| Some((name.to_owned(), tex(v)?)))
            .collect::<Option<Vec<_>>>()?;
        let models = props
            .with_prefix("model.")
            .map(|(name, v)| Some((name.to_owned(), model_at(v)?)))
            .collect::<Option<Vec<_>>>()?;
        if texture.is_none() && model.is_none() && textures.is_empty() && models.is_empty() {
            // Neither named: a model, then a texture, named after the file beside it.
            let dir = path.rsplit_once('/').map_or("", |(d, _)| d);
            let beside = |ext: &str| {
                let p = format!("{dir}/{stem}.{ext}");
                exists(&p).then_some(p)
            };
            if kind == Kind::Item {
                model = beside("json");
            }
            if model.is_none() {
                texture = beside("png");
            }
        }
        if texture.is_none() && model.is_none() && textures.is_empty() && models.is_empty() {
            return None;
        }
        // A glint is a texture: CIT Resewn drops an enchantment rule without one ("No texture
        // specified"), whatever models it names.
        if kind == Kind::Enchantment && texture.is_none() {
            return None;
        }
        // Only an enchantment rule reads the glint keys: an item rule's `blend=` is no reason to
        // drop it.
        let glint = if kind == Kind::Enchantment {
            Glint::parse(props)?
        } else {
            Glint::default()
        };
        Some(Self {
            path: path.to_owned(),
            kind,
            weight: optional(props, &["weight", "cit_weight", "citWeight"], int)?.unwrap_or(0),
            items,
            stack_size: optional(
                props,
                &["stackSize", "stack_size", "amount"],
                RangeList::parse,
            )?,
            damage,
            hand: match props.get("hand").map(str::to_ascii_lowercase).as_deref() {
                // CIT Resewn's spellings too.
                None | Some("any" | "either" | "*") => Hand::Any,
                Some("main" | "mainhand" | "main_hand") => Hand::Main,
                Some("off" | "offhand" | "off_hand") => Hand::Off,
                Some(_) => return None,
            },
            enchantments,
            enchantment_levels: optional(
                props,
                &["enchantmentLevels", "enchantment_levels"],
                RangeList::parse,
            )?,
            components,
            texture,
            textures,
            model,
            models,
            glint,
        })
    }

    /// Which of two matching rules is chosen: `Less` for the one that wins, the higher weight,
    /// then the path that sorts first.
    pub fn priority(&self, other: &Self) -> std::cmp::Ordering {
        (other.weight, &self.path).cmp(&(self.weight, &other.path))
    }
}

/// `method` of `cit.properties`: how several enchantment layers on one item share it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// Every layer at full strength, together (CIT Resewn's `none`).
    None,
    /// Each layer's intensity is its enchantment's level over the sum of levels.
    Average,
    /// Over the highest level.
    Layered,
    /// One layer at a time, each for its `duration`.
    Cycle,
}

/// The pack's `cit.properties`.
#[derive(Clone, Debug, PartialEq)]
pub struct Global {
    pub method: Method,
    /// At most this many enchantment layers per item (the topmost win).
    pub cap: Option<u32>,
    /// Seconds a cycle takes to fade from one layer to the next.
    pub fade: f32,
    /// Whether the default glint draws on items no enchantment rule matches.
    pub use_glint: bool,
}

impl Default for Global {
    fn default() -> Self {
        Self {
            method: Method::Average,
            cap: None,
            fade: 0.5,
            use_glint: true,
        }
    }
}

impl Global {
    /// Where the global file may be, the first that exists winning, as both docs say (CIT
    /// Resewn's code merges the files' keys instead): CIT Resewn's and the OptiFine doc's folders.
    pub const PATHS: [&str; 4] = [
        "assets/minecraft/citresewn/cit.properties",
        "assets/minecraft/optifine/cit.properties",
        "assets/minecraft/optifine/cit/cit.properties",
        "assets/minecraft/mcpatcher/cit.properties",
    ];

    /// Unknown or malformed values keep their defaults.
    pub fn parse(props: &Properties) -> Self {
        let mut out = Self::default();
        let lower = |key| props.get(key).map(|v| v.trim().to_ascii_lowercase());
        match lower("method").as_deref() {
            Some("layered") => out.method = Method::Layered,
            Some("cycle") => out.method = Method::Cycle,
            Some("none") => out.method = Method::None,
            _ => {}
        }
        out.cap = props.get("cap").and_then(|v| v.trim().parse().ok());
        if let Some(fade) = props.get("fade").and_then(finite) {
            out.fade = fade.max(0.0);
        }
        out.use_glint = lower("useGlint").as_deref() != Some("false");
        out
    }
}

/// An optional key, under any of its names (CIT Resewn's aliases): `Some(None)` when absent,
/// `None` (dropping the rule) when malformed.
fn optional<T>(
    props: &Properties,
    keys: &[&str],
    parse: impl Fn(&str) -> Option<T>,
) -> Option<Option<T>> {
    match keys.iter().find_map(|k| props.get(k)) {
        Some(v) => parse(v).map(Some),
        None => Some(None),
    }
}

/// A `texture`/`model` value as a resource path to an existing `.ext` file, trying what CIT
/// Resewn tries in its order: as written (OptiFine's paths, see `paths::resolve`, or a full
/// `assets/...` path), in the namespace's typed folder (`item/x` → `textures/item/x.png`), then
/// beside the rule (`sub/x`, and `./../x` up from its folder).
fn asset(
    props: &str,
    value: &str,
    ext: &str,
    typed: &str,
    exists: &impl Fn(&str) -> bool,
) -> Option<String> {
    let last = value.rsplit('/').next().unwrap_or(value);
    let value = if last.ends_with(&format!(".{ext}")) {
        value.to_owned()
    } else {
        format!("{value}.{ext}")
    };
    let dir = props.rsplit_once('/').map_or("", |(d, _)| d);
    let mut candidates = Vec::new();
    if value.split('/').any(|s| s == "..") {
        // Only up from the rule's folder: `~/` and `ns:` paths name their folder outright.
        if !value.starts_with("~/") && !value.contains(':') {
            candidates.extend(up_from(dir, &value));
        }
    } else if value.starts_with("assets/") {
        candidates.extend(valid(&value).then(|| value.clone()));
    } else if let Some(resolved) = resolve(props, &value) {
        let plain = !value.starts_with("~/") && !value.starts_with("./");
        if plain && value.contains(['/', ':']) {
            if let Some((ns, rest)) = resolved
                .strip_prefix("assets/")
                .and_then(|p| p.split_once('/'))
            {
                if !rest.starts_with(&format!("{typed}/")) {
                    candidates.push(format!("assets/{ns}/{typed}/{rest}"));
                }
            }
            if !value.contains(':') {
                candidates.push(format!("{dir}/{value}"));
            }
        }
        candidates.insert(0, resolved);
    }
    candidates.into_iter().find(|p| exists(p))
}

/// `value` (`./../x`, `../x`) in the folder `dir`, each `..` taking a folder off; `None` when it
/// would climb out of `assets/<namespace>/`.
fn up_from(dir: &str, value: &str) -> Option<String> {
    let mut out: Vec<&str> = dir.split('/').collect();
    for segment in value.strip_prefix("./").unwrap_or(value).split('/') {
        match segment {
            ".." if out.len() > 2 => {
                out.pop();
            }
            ".." => return None,
            s => out.push(s),
        }
    }
    let path = out.join("/");
    (out.len() > 2 && out[0] == "assets" && valid(&path)).then_some(path)
}

/// An `enchantments` entry: an id (`flame`, `minecraft:flame`) or a pre-1.13 numeric id. An
/// unknown number stays an id no enchantment has, as in CIT Resewn, so its rule never matches.
fn enchantment_id(token: &str) -> String {
    if token.eq_ignore_ascii_case("sweeping") || token.eq_ignore_ascii_case("minecraft:sweeping") {
        return "minecraft:sweeping_edge".to_owned();
    }
    let Ok(n) = token.parse::<u32>() else {
        return namespaced(token);
    };
    let name = match n {
        0 => "protection",
        1 => "fire_protection",
        2 => "feather_falling",
        3 => "blast_protection",
        4 => "projectile_protection",
        5 => "respiration",
        6 => "aqua_affinity",
        7 => "thorns",
        8 => "depth_strider",
        9 => "frost_walker",
        10 => "binding_curse",
        16 => "sharpness",
        17 => "smite",
        18 => "bane_of_arthropods",
        19 => "knockback",
        20 => "fire_aspect",
        21 => "looting",
        22 => "sweeping_edge",
        32 => "efficiency",
        33 => "silk_touch",
        34 => "unbreaking",
        35 => "fortune",
        48 => "power",
        49 => "punch",
        50 => "flame",
        51 => "infinity",
        61 => "luck_of_the_sea",
        62 => "lure",
        70 => "mending",
        71 => "vanishing_curse",
        _ => return namespaced(token),
    };
    format!("minecraft:{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(path: &str, text: &str, files: &[&str]) -> Option<Rule> {
        Rule::parse(path, &Properties::parse_equals_first(text), |p| {
            files.contains(&p)
        })
    }

    const SWORD: &str = "assets/minecraft/optifine/cit/swords/flame.properties";

    #[test]
    fn an_item_rule_with_every_condition() {
        let r = rule(
            SWORD,
            "items=diamond_sword minecraft:iron_sword\ntexture=flame_sword\nweight=5\n\
             stackSize=1\ndamage=0-50%\nhand=off\nenchantmentIDs=20 sharpness\n\
             enchantmentLevels=3-\nnbt.display.Name=ipattern:*flame*\n",
            &["assets/minecraft/optifine/cit/swords/flame_sword.png"],
        )
        .unwrap();
        assert_eq!(r.kind, Kind::Item);
        assert_eq!(r.items, ["minecraft:diamond_sword", "minecraft:iron_sword"]);
        assert_eq!(
            r.texture.as_deref(),
            Some("assets/minecraft/optifine/cit/swords/flame_sword.png")
        );
        assert_eq!(r.weight, 5);
        assert_eq!(r.hand, Hand::Off);
        let files = ["assets/minecraft/optifine/cit/swords/flame.png"];
        for (v, hand) in [
            ("Offhand", Hand::Off),
            ("main_hand", Hand::Main),
            ("*", Hand::Any),
        ] {
            let spelled = rule(SWORD, &format!("items=stick\nhand={v}"), &files).unwrap();
            assert_eq!(spelled.hand, hand, "{v}");
        }
        let stack = r.stack_size.as_ref().unwrap();
        assert!(stack.contains(1) && !stack.contains(2));
        assert_eq!(
            r.enchantments,
            ["minecraft:fire_aspect", "minecraft:sharpness"]
        );
        assert!(r.enchantment_levels.as_ref().unwrap().contains(7));
        assert!(!r.enchantment_levels.as_ref().unwrap().contains(2));
        // A diamond sword lasts 1561: half of it is 780.5 used.
        let damage = r.damage.as_ref().unwrap();
        assert!(damage.matches(780, 1561));
        assert!(!damage.matches(781, 1561));
        assert!(!damage.matches(0, 0), "no max damage, no percentage");
        let [name] = &r.components[..] else { panic!() };
        assert_eq!(
            name.source,
            Source::Component("minecraft:custom_name".into())
        );
        assert!(name.path.is_empty());
        let ValueTest::Text(n) = &name.value else {
            panic!()
        };
        assert!(n.matches("The FLAME Blade"));
    }

    #[test]
    fn damage_masks_apply_before_the_ranges() {
        let r = rule(
            SWORD,
            "items=potion\ndamage=3\ndamageMask=15\n",
            &["assets/minecraft/optifine/cit/swords/flame.png"],
        )
        .unwrap();
        let d = r.damage.unwrap();
        assert!(
            d.matches(16_387, 0),
            "splash fire resistance: 16387 & 15 = 3"
        );
        assert!(!d.matches(16_388, 0));
    }

    #[test]
    fn default_items_and_assets_come_from_the_file_name() {
        let dir = "assets/minecraft/optifine/cit/sticks";
        let path = format!("{dir}/stick.properties");
        // No texture or model named: the model beside it wins over the texture.
        let both = [&*format!("{dir}/stick.json"), &*format!("{dir}/stick.png")];
        let r = rule(&path, "nbt.CustomModelData=1\n", &both).unwrap();
        assert_eq!(r.items, ["minecraft:stick"]);
        assert_eq!(r.model.as_deref(), Some(&*format!("{dir}/stick.json")));
        assert_eq!(r.texture, None);
        let r = rule(&path, "", &both[1..]).unwrap();
        assert_eq!(r.texture.as_deref(), Some(&*format!("{dir}/stick.png")));
        // The file name is a bare id: a minecraft one even in another namespace's folder.
        let other = rule(
            "assets/mymod/optifine/cit/stick.properties",
            "",
            &["assets/mymod/optifine/cit/stick.png"],
        );
        assert_eq!(other.unwrap().items, ["minecraft:stick"]);
        // Nothing to draw: no rule.
        assert!(rule(&path, "", &[]).is_none());
    }

    #[test]
    fn asset_paths_try_the_typed_folders() {
        let files = [
            "assets/minecraft/textures/item/excalibur.png",
            "assets/minecraft/models/item/excalibur.json",
            "assets/minecraft/optifine/cit/shared/glow.png",
            "assets/minecraft/optifine/cit/swords/sub/x.png",
        ];
        let tex = |v| rule(SWORD, &format!("items=stick\ntexture={v}"), &files)?.texture;
        let at = |s: &str| Some(s.to_owned());
        assert_eq!(tex("item/excalibur"), at(files[0]));
        assert_eq!(tex("minecraft:item/excalibur.png"), at(files[0]));
        assert_eq!(tex("textures/item/excalibur"), at(files[0]));
        assert_eq!(
            tex("assets/minecraft/textures/item/excalibur.png"),
            at(files[0])
        );
        assert_eq!(tex("~/cit/shared/glow"), at(files[2]));
        assert_eq!(tex("./sub/x"), at(files[3]));
        assert_eq!(tex("./../swords/sub/x"), at(files[3]));
        assert_eq!(tex("sub/x"), at(files[3]), "beside the rule, last");
        // `..` climbs only from the rule's folder: `~/` and `ns:` name theirs outright.
        let beside = ["assets/minecraft/optifine/cit/swords/x.png"];
        let up = |v| rule(SWORD, &format!("items=stick\ntexture={v}"), &beside);
        assert!(up("./a/../x").is_some());
        assert!(up("~/../x").is_none());
        assert!(up("minecraft:a/../x").is_none());
        let model = rule(SWORD, "items=stick\nmodel=item/excalibur", &files).unwrap();
        assert_eq!(model.model.as_deref(), Some(files[1]));
    }

    #[test]
    fn climbing_stops_at_the_namespace() {
        // Files at every folder a climb could reach: only the namespace's own are found.
        let files = ["assets/x.png", "x.png", "assets/minecraft/x.png"];
        let tex = |v: &str| rule(SWORD, &format!("items=stick\ntexture={v}"), &files)?.texture;
        assert_eq!(tex("../../../x").as_deref(), Some("assets/minecraft/x.png"));
        assert_eq!(tex("../../../../x"), None);
        assert_eq!(tex("../../../../../x"), None);
        // A Windows `\` is refused by `paths::valid`, whose tests cover it.
    }

    #[test]
    fn a_named_asset_that_is_missing_drops_the_rule() {
        // flame.png sits beside flame.properties, but the rule names another texture.
        let beside = ["assets/minecraft/optifine/cit/swords/flame.png"];
        assert!(rule(SWORD, "items=stick\ntexture=typo", &beside).is_none());
        assert!(rule(SWORD, "items=stick", &beside).is_some());
    }

    #[test]
    fn sub_textures_and_armor_layers() {
        let dir = "assets/minecraft/optifine/cit/bow";
        let files: Vec<String> = ["a0", "a1", "diamond1", "diamond2"]
            .iter()
            .map(|n| format!("{dir}/{n}.png"))
            .collect();
        let refs: Vec<&str> = files.iter().map(String::as_str).collect();
        let r = rule(
            &format!("{dir}/bow.properties"),
            "items=bow\ntexture.bow_pulling_0=a0\ntexture.bow_pulling_1=a1\n",
            &refs,
        )
        .unwrap();
        let names: Vec<_> = r.textures.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["bow_pulling_0", "bow_pulling_1"]);
        // One frame missing: no rule, rather than a bow that is half vanilla.
        let missing = "items=bow\ntexture.bow_pulling_0=a0\ntexture.bow_pulling_2=missing\n";
        assert!(rule(&format!("{dir}/bow.properties"), missing, &refs).is_none());
        let armor = rule(
            &format!("{dir}/armor.properties"),
            "type=armor\nitems=diamond_chestplate\ntexture.diamond_layer_1=diamond1\ntexture.diamond_layer_2=diamond2\n",
            &refs,
        )
        .unwrap();
        assert_eq!(armor.kind, Kind::Armor);
        assert_eq!(
            armor.textures[1],
            ("diamond_layer_2".into(), files[3].clone())
        );
        let glint = rule(
            &format!("{dir}/glint.properties"),
            "type=enchantment\ntexture=a0",
            &refs,
        )
        .unwrap();
        assert!(
            glint.items.is_empty(),
            "an enchantment rule without items is for every item"
        );
        // Elytra rules are always for the elytra.
        let elytra = rule(
            &format!("{dir}/x.properties"),
            "type=elytra\nitems=stick\ntexture=a0",
            &refs,
        )
        .unwrap();
        assert_eq!(elytra.items, ["minecraft:elytra"]);
    }

    #[test]
    fn an_enchantment_rule_needs_a_texture() {
        let dir = "assets/minecraft/optifine/cit/glint";
        let files = [
            format!("{dir}/shine.json"),
            format!("{dir}/shine.png"),
            format!("{dir}/other.png"),
        ];
        let refs: Vec<&str> = files.iter().map(String::as_str).collect();
        let path = format!("{dir}/shine.properties");
        // The model is there: an item rule takes it.
        assert!(rule(&path, "items=stick\nmodel=shine", &refs).is_some());
        assert!(rule(&path, "type=enchantment\nmodel=shine", &refs).is_none());
        assert!(rule(&path, "type=enchantment\ntexture.a=other", &refs).is_none());
        // The texture beside the file still counts.
        let beside = rule(&path, "type=enchantment", &refs).unwrap();
        assert_eq!(beside.texture.as_deref(), Some(&*files[1]));
    }

    #[test]
    fn legacy_nbt_paths_land_on_components() {
        let source = |key: &str| {
            let t = ComponentTest::parse(true, key, "x").unwrap();
            let Source::Component(id) = t.source else {
                return (format!("{:?}", t.source), t.path);
            };
            (id, t.path)
        };
        let s = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            source("display.Lore.*"),
            ("minecraft:lore".into(), s(&["*"]))
        );
        assert_eq!(
            source("CustomModelData"),
            ("minecraft:custom_model_data".into(), s(&["floats", "0"]))
        );
        assert_eq!(
            source("Potion"),
            ("minecraft:potion_contents".into(), s(&["potion"]))
        );
        assert_eq!(
            source("ExtraAttributes.id"),
            (
                "minecraft:custom_data".into(),
                s(&["ExtraAttributes", "id"])
            )
        );
        let ench = ComponentTest::parse(true, "StoredEnchantments.0.id", "x").unwrap();
        assert_eq!(ench.source, Source::LegacyEnchantments { stored: true });
        assert_eq!(ench.path, s(&["0", "id"]));
        // `~` is the minecraft namespace; a bare component id is in it too.
        // Since 1.21.5 the enchantments component has no `levels` level.
        let c = ComponentTest::parse(false, "enchantments.levels.~thorns", "3").unwrap();
        assert_eq!(c.source, Source::Component("minecraft:enchantments".into()));
        assert_eq!(c.path, s(&["minecraft:thorns"]));
        let c = ComponentTest::parse(false, "custom_data.my~key.~x", "3").unwrap();
        assert_eq!(
            c.path,
            s(&["my~key", "minecraft:x"]),
            "~ only opens a segment"
        );
        let c = ComponentTest::parse(false, "custom_data.levels", "3").unwrap();
        assert_eq!(c.path, s(&["levels"]));
        assert!(ComponentTest::parse(false, "lore..0", "x").is_none());
    }

    #[test]
    fn component_values() {
        let v = |s: &str| ValueTest::parse(s).unwrap();
        let (ValueTest::Literal(l), false) = v("#ff99cc") else {
            panic!()
        };
        assert_eq!(l.int, Some(0xff99cc));
        let (ValueTest::Literal(l), false) = v("0x1F") else {
            panic!()
        };
        assert_eq!(l.int, Some(31));
        let (ValueTest::Literal(l), false) = v("1.5") else {
            panic!()
        };
        assert_eq!((l.int, l.float), (None, Some(1.5)));
        let (ValueTest::Literal(l), false) = v("53") else {
            panic!()
        };
        assert_eq!((l.int, l.float), (Some(53), Some(53.0)));
        let (ValueTest::Literal(l), true) = v("!Bob") else {
            panic!()
        };
        assert_eq!((l.text.as_str(), l.int), ("Bob", None));
        let (ValueTest::Range(r), false) = v("range:(-25) (-10)-10 15") else {
            panic!()
        };
        assert!(r.contains(-25) && r.contains(0) && !r.contains(11));
        assert!(matches!(
            v("exists:false"),
            (ValueTest::Exists(false), false)
        ));
        let (ValueTest::Raw(n), false) = v("raw:pattern:*Chopper*") else {
            panic!()
        };
        assert!(n.matches("{text:\"Chopper\"}"));
        assert!(ValueTest::parse("regex:(").is_none());
        assert!(ValueTest::parse("exists:maybe").is_none());
    }

    #[test]
    fn unknown_enchantment_ids_never_match_instead_of_matching_all() {
        let files = ["assets/minecraft/optifine/cit/swords/flame.png"];
        let r = rule(SWORD, "items=stick\nenchantmentIDs=99 sweeping", &files).unwrap();
        assert_eq!(r.enchantments, ["minecraft:99", "minecraft:sweeping_edge"]);
    }

    #[test]
    fn resewn_key_spellings() {
        let files = ["assets/minecraft/optifine/cit/swords/flame.png"];
        let text = "items=arrow\namount=64\ncitWeight=5\ndamage=3\ndamage_mask=15\n\
                    enchantment_levels=2\ncomponents.minecraft:custom_name=Quiver\n";
        let r = rule(SWORD, text, &files).unwrap();
        assert!(r.stack_size.unwrap().contains(64));
        assert_eq!(r.weight, 5);
        assert_eq!(r.damage.unwrap().mask, Some(15));
        assert!(r.enchantment_levels.unwrap().contains(2));
        let [name] = &r.components[..] else { panic!() };
        assert_eq!(
            name.source,
            Source::Component("minecraft:custom_name".into())
        );
    }

    #[test]
    fn tags_that_became_components_are_read_there_or_drop_the_rule() {
        let at = |key: &str| {
            let t = ComponentTest::parse(true, key, "x")?;
            let Source::Component(id) = t.source else {
                return None;
            };
            Some((id, t.path.join(".")))
        };
        let is = |id: &str, path: &str| Some((format!("minecraft:{id}"), path.to_owned()));
        assert_eq!(at("SkullOwner"), is("profile", "name"));
        assert_eq!(at("SkullOwner.Name"), is("profile", "name"));
        assert_eq!(at("generation"), is("written_book_content", "generation"));
        assert_eq!(at("EntityTag.CustomName"), is("entity_data", "CustomName"));
        assert_eq!(at("Fireworks.Flight"), is("fireworks", "flight_duration"));
        // Moved to a component of another shape: no test, and so no rule.
        assert_eq!(at("AttributeModifiers.0.Amount"), None);
        assert_eq!(at("HideFlags"), None);
        assert_eq!(at("SkullOwner.Properties.textures.0.Value"), None);
        assert_eq!(at("BlockEntityTag.Patterns.0.Pattern"), None);
        assert_eq!(at("display.MapColor"), None);
        assert_eq!(
            at("BlockEntityTag.CustomName"),
            is("block_entity_data", "CustomName")
        );
        assert_eq!(at("instrument"), is("instrument", ""));
        assert_eq!(
            at("DebugProperty.minecraft:stone"),
            is("debug_stick_state", "minecraft:stone")
        );
        assert_eq!(at("EntityTag.variant"), is("painting/variant", ""));
        assert_eq!(at("display.LocName"), None);
        // Bucket mobs' tags went into `bucket_entity_data`, and `Variant` became a name.
        assert_eq!(at("Variant"), None);
        // What the fixer left in `display` stayed in custom data.
        assert_eq!(at("display.Foo"), is("custom_data", "display.Foo"));
        let files = ["assets/minecraft/optifine/cit/swords/flame.png"];
        assert!(rule(SWORD, "items=stick\nnbt.Unbreakable=!1", &files).is_none());
    }

    #[test]
    fn malformed_conditions_drop_the_rule() {
        let files = ["assets/minecraft/optifine/cit/swords/flame.png"];
        for bad in [
            "damage=lots",
            "damage=3\ndamageMask=abc",
            "stackSize=1-x",
            "hand=left",
            "texture=flame\nmodel=typo",
            "weight=heavy",
            "type=hat",
            "nbt.display.Name=regex:(",
        ] {
            assert!(
                rule(SWORD, &format!("items=stick\n{bad}\n"), &files).is_none(),
                "{bad}"
            );
        }
    }

    #[test]
    fn heavier_rules_then_earlier_paths_win() {
        let files = ["assets/minecraft/optifine/cit/a.png"];
        let at = |path: &str, weight: i32| {
            rule(
                path,
                &format!("items=stick\ntexture=~/cit/a\nweight={weight}"),
                &files,
            )
            .unwrap()
        };
        let a = at("assets/minecraft/optifine/cit/a.properties", 0);
        let b = at("assets/minecraft/optifine/cit/b.properties", 0);
        let heavy = at("assets/minecraft/optifine/cit/c.properties", 3);
        let mut rules = [b, heavy, a];
        rules.sort_by(Rule::priority);
        let order: Vec<_> = rules
            .iter()
            .map(|r| r.path.rsplit('/').next().unwrap())
            .collect();
        assert_eq!(order, ["c.properties", "a.properties", "b.properties"]);
    }

    #[test]
    fn enchantment_layers() {
        let files = ["assets/minecraft/optifine/cit/swords/flame.png"];
        let r = rule(
            SWORD,
            "type=enchantment\ntexture=flame\nlayer=2\nblend=GL_DST_COLOR GL_ONE_MINUS_SRC_ALPHA\n\
             speed=0.5\nrotation=45\nduration=2\nr=0.5\ng=-1\n",
            &files,
        )
        .unwrap();
        assert_eq!(r.glint.layer, 2);
        assert_eq!(r.glint.blend, parse_blend("multiply").unwrap());
        assert_eq!(
            (r.glint.speed, r.glint.rotation, r.glint.duration),
            (0.5, 45.0, 2.0)
        );
        assert_eq!(
            r.glint.rgba,
            [0.5, 0.0, 1.0, 1.0],
            "colours never go below 0"
        );
        assert_eq!(parse_blend("replace"), Some(None));
        let custom = parse_blend("0x306 771 0 1").unwrap().unwrap();
        assert_eq!(Some(Some(custom)), parse_blend("multiply"));
        assert_eq!(parse_blend("GL_NOPE GL_ONE"), None);
        // OptiFine's defaults: still, unturned, a second per cycle, added on top.
        let plain = rule(SWORD, "type=enchantment\ntexture=flame", &files).unwrap();
        assert_eq!(
            (plain.glint.layer, plain.glint.speed, plain.glint.rotation),
            (0, 0.0, 0.0)
        );
        assert_eq!(plain.glint.duration, 1.0);
        assert_eq!(
            plain.glint.blend,
            parse_blend("GL_SRC_ALPHA GL_ONE").unwrap()
        );
        assert!(!plain.glint.use_glint && plain.glint.blur);
        assert!(rule(SWORD, "type=enchantment\ntexture=flame\nspeed=fast", &files).is_none());
        assert!(rule(SWORD, "type=enchantment\ntexture=flame\nblend=mix", &files).is_none());
        // NaN and the infinities are malformed too.
        for bad in ["speed=NaN", "r=inf", "duration=-Infinity", "a=nan"] {
            let text = format!("type=enchantment\ntexture=flame\n{bad}");
            assert!(rule(SWORD, &text, &files).is_none(), "{bad}");
        }
    }

    #[test]
    fn glint_keys_are_only_an_enchantment_rules() {
        let files = ["assets/minecraft/optifine/cit/swords/flame.png"];
        let r = rule(
            SWORD,
            "items=diamond_sword\ntexture=flame\nblend=nonsense\nspeed=fast\nr=nan",
            &files,
        )
        .expect("an item rule ignores the glint keys");
        assert_eq!(r.glint, Glint::default());
    }

    #[test]
    fn named_blends() {
        use Factor::*;
        let pair = |src, dst| Blend {
            src,
            dst,
            src_alpha: Zero,
            dst_alpha: One,
        };
        // CIT Resewn's doc's table.
        let table = [
            ("glint", pair(SrcColor, One)),
            ("alpha", pair(SrcAlpha, OneMinusSrcAlpha)),
            ("add", pair(SrcAlpha, One)),
            ("subtract", pair(OneMinusDstColor, Zero)),
            ("multiply", pair(DstColor, OneMinusSrcAlpha)),
            ("dodge", pair(One, One)),
            ("burn", pair(Zero, OneMinusSrcColor)),
            ("screen", pair(One, OneMinusSrcColor)),
            ("overlay", pair(DstColor, SrcColor)),
        ];
        for (name, blend) in table {
            assert_eq!(parse_blend(name), Some(Some(blend)), "{name}");
        }
        assert_eq!(parse_blend("replace"), Some(None));
        // Names are case-blind (CIT Resewn upper-cases them); GL constants are not (it looks
        // them up as GL11's field names).
        assert_eq!(parse_blend("Multiply"), parse_blend("multiply"));
        assert_eq!(parse_blend("gl_one GL_ONE"), None);
    }

    #[test]
    fn global_properties() {
        let g = Global::parse(&Properties::parse(
            "method=cycle\ncap=2\nfade=1.5\nuseGlint=false",
        ));
        assert_eq!(
            g,
            Global {
                method: Method::Cycle,
                cap: Some(2),
                fade: 1.5,
                use_glint: false
            }
        );
        assert_eq!(
            Global::parse(&Properties::parse("method=odd\ncap=x\nfade=-1")),
            Global {
                fade: 0.0,
                ..Global::default()
            }
        );
        assert_eq!(
            Global::parse(&Properties::parse("method=Layered\nuseGlint=FALSE")).method,
            Method::Layered
        );
        assert!(!Global::parse(&Properties::parse("useGlint=FALSE")).use_glint);
        assert_eq!(
            Global::parse(&Properties::parse("method=NONE")).method,
            Method::None
        );
        for fade in ["nan", "INF", "-infinity"] {
            let g = Global::parse(&Properties::parse(&format!("fade={fade}")));
            assert_eq!(g.fade, 0.5, "{fade}");
        }
    }
}
