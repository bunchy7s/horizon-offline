//! Item stacks on the wire (ItemStack.OPTIONAL_STREAM_CODEC): a count, the
//! item's registry id and a data component patch, plus the hashed form the
//! client sends back in container clicks (HashedStack).
use crate::{
    Result,
    buf::{Codec, Reader, Writer, pack_block_pos, unpack_block_pos},
};
use horizon_world::nbt::Tag;
use std::sync::Arc;

#[doc(hidden)]
pub mod custom;
mod hash;
mod skip;

pub use hash::{Names, NoNames};

/// minecraft:data_component_type registry ids (26.3).
pub mod component {
    pub const CUSTOM_DATA: i32 = 0;
    pub const MAX_STACK_SIZE: i32 = 1;
    pub const MAX_DAMAGE: i32 = 2;
    pub const DAMAGE: i32 = 3;
    pub const UNBREAKABLE: i32 = 4;
    pub const CUSTOM_NAME: i32 = 6;
    pub const ITEM_NAME: i32 = 9;
    pub const ITEM_MODEL: i32 = 10;
    pub const LORE: i32 = 11;
    pub const RARITY: i32 = 12;
    pub const ENCHANTMENTS: i32 = 13;
    /// `AdventureModePredicate`: the blocks an adventure player may break.
    pub const CAN_BREAK: i32 = 15;
    pub const CUSTOM_MODEL_DATA: i32 = 17;
    pub const TOOLTIP_DISPLAY: i32 = 18;
    pub const REPAIR_COST: i32 = 19;
    pub const CREATIVE_SLOT_LOCK: i32 = 20;
    pub const ENCHANTMENT_GLINT_OVERRIDE: i32 = 21;
    /// `AttackRange`: six floats, in the record's field order.
    pub const ATTACK_RANGE: i32 = 30;
    pub const STORED_ENCHANTMENTS: i32 = 45;
    pub const PROFILE: i32 = 72;
    pub const BANNER_PATTERNS: i32 = 74;
    pub const BASE_COLOR: i32 = 75;
    pub const DYED_COLOR: i32 = 47;
    pub const MAP_ID: i32 = 48;
    pub const MAP_POST_PROCESSING: i32 = 50;
    pub const CHARGED_PROJECTILES: i32 = 51;
    pub const BUNDLE_CONTENTS: i32 = 52;
    pub const POTION_CONTENTS: i32 = 53;
    pub const SUSPICIOUS_STEW_EFFECTS: i32 = 55;
    pub const WRITABLE_BOOK_CONTENT: i32 = 56;
    pub const WRITTEN_BOOK_CONTENT: i32 = 57;
    pub const TRIM: i32 = 58;
    pub const INSTRUMENT: i32 = 63;
    pub const LODESTONE_TRACKER: i32 = 69;
    pub const FIREWORK_EXPLOSION: i32 = 70;
    pub const FIREWORKS: i32 = 71;
    pub const BLOCK_ENTITY_DATA: i32 = 62;
    pub const OMINOUS_BOTTLE_AMPLIFIER: i32 = 65;
    pub const USE_REMAINDER: i32 = 25;
    pub const POT_DECORATIONS: i32 = 76;
    pub const CONTAINER: i32 = 77;
    pub const SULFUR_CUBE_CONTENT: i32 = 80;
    pub const PAINTING_VARIANT: i32 = 109;
}

/// A written book's content (`WrittenBookContent.STREAM_CODEC`). The raw title
/// and pages are what a book shows; the filtered variants (`Filterable.filtered`,
/// which a server sends when chat filtering is on) are kept only so that the
/// component hashes as the server holds it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrittenBook {
    pub title: String,
    pub title_filtered: Option<String>,
    pub author: String,
    pub generation: i32,
    /// Each page's text component.
    pub pages: Vec<Tag>,
    /// Each page's filtered variant: empty (none filtered) or one per page.
    pub pages_filtered: Vec<Option<Tag>>,
    pub resolved: bool,
}

/// The value of one patched data component. Components whose stream codec
/// Horizon does not implement make the whole stack (and packet) undecodable.
#[derive(Clone, Debug, PartialEq)]
pub enum Component {
    Int(i32),
    Unit,
    Bool(bool),
    Text(Tag),
    Nbt(Tag),
    Ident(String),
    Lore(Vec<Tag>),
    Enchantments(Vec<(i32, i32)>),
    AttackRange([f32; 6]),
    /// `PotionContents`: the potion's `minecraft:potion` id and the custom
    /// colour; the custom effects and name stay their encoded bytes.
    Potion {
        potion: Option<i32>,
        color: Option<i32>,
        rest: Vec<u8>,
    },
    Profile(Profile),
    CustomModelData {
        floats: Vec<f32>,
        flags: Vec<bool>,
        strings: Vec<String>,
        colors: Vec<i32>,
    },
    TooltipDisplay {
        hide: bool,
        hidden: Vec<i32>,
    },
    /// `WritableBookContent`: the pages (raw text) and their filtered variants
    /// (empty, or one per page).
    WritableBook {
        pages: Vec<String>,
        filtered: Vec<Option<String>>,
    },
    WrittenBook(WrittenBook),
    /// `ChargedProjectiles`: a loaded crossbow's ammunition, as templates.
    Stacks(Vec<Stack>),
    /// `Fireworks`: flight duration and its explosions.
    Fireworks {
        flight: i32,
        explosions: Vec<FireworkExplosion>,
    },
    /// `FireworkExplosion` (a firework star).
    FireworkExplosion(FireworkExplosion),
    /// `LodestoneTracker`: the target's dimension and block, and whether it
    /// follows the lodestone.
    LodestoneTracker {
        target: Option<(String, [i32; 3])>,
        tracked: bool,
    },
    /// `ArmorTrim` naming its material and pattern by registry id (`trim_material`,
    /// `trim_pattern`); a trim with a direct holder stays [`Component::Raw`].
    Trim {
        material: i32,
        pattern: i32,
    },
    /// `SuspiciousStewEffects`: each entry's `minecraft:mob_effect` id and
    /// duration in ticks.
    StewEffects(Vec<(i32, i32)>),
    /// `InstrumentComponent` naming its instrument by `minecraft:instrument`
    /// registry id; a direct instrument stays [`Component::Raw`].
    Instrument(i32),
    /// `BannerPatternLayers`: each layer's `banner_pattern` registry id and
    /// its dye id (`DyeColor.getId`). A layer with a direct pattern has no
    /// registry name: that value stays [`Component::Raw`].
    BannerPatterns(Vec<(i32, i32)>),
    /// `painting/variant` naming its `painting_variant` by registry id; a
    /// direct variant stays [`Component::Raw`].
    PaintingVariant(i32),
    /// A component Horizon reads past without interpreting it: its encoded
    /// bytes, written back as they came.
    Raw(Vec<u8>),
}

/// `FireworkExplosion.STREAM_CODEC`: shape id, colors, fade colors, trail
/// and twinkle (FireworkExplosion.Shape's BY_ID order).
#[derive(Clone, Debug, PartialEq)]
pub struct FireworkExplosion {
    pub shape: i32,
    pub colors: Vec<i32>,
    pub fade: Vec<i32>,
    pub trail: bool,
    pub twinkle: bool,
}
impl FireworkExplosion {
    fn read(r: &mut Reader) -> Result<Self> {
        let ints = |r: &mut Reader| -> Result<Vec<i32>> {
            let n = r.count(1024)?;
            (0..n).map(|_| r.i32()).collect()
        };
        Ok(Self {
            shape: r.varint()?,
            colors: ints(r)?,
            fade: ints(r)?,
            trail: r.bool()?,
            twinkle: r.bool()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.varint(self.shape);
        w.varint(self.colors.len() as i32);
        self.colors.iter().for_each(|c| w.i32(*c));
        w.varint(self.fade.len() as i32);
        self.fade.iter().for_each(|c| w.i32(*c));
        w.bool(self.trail);
        w.bool(self.twinkle);
    }
}

/// `Filterable.streamCodec`: the raw value, then an optional filtered one.
fn filterable<T>(
    r: &mut Reader,
    value: impl Fn(&mut Reader) -> Result<T>,
) -> Result<(T, Option<T>)> {
    let raw = value(r)?;
    let filtered = if r.bool()? { Some(value(r)?) } else { None };
    Ok((raw, filtered))
}
/// The filtered variants of a list of pages, or none when no page has one.
fn filtered_variants<T>(pages: &mut Vec<(T, Option<T>)>) -> (Vec<T>, Vec<Option<T>>) {
    let any = pages.iter().any(|(_, f)| f.is_some());
    let (raw, filtered): (Vec<_>, Vec<_>) = pages.drain(..).unzip();
    (raw, if any { filtered } else { Vec::new() })
}

/// `ResolvableProfile`: an optional name and UUID and the profile's property
/// list, followed by `PlayerSkin.Patch`'s four optional textures.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub name: Option<String>,
    pub id: Option<u128>,
    pub properties: Vec<(String, String, Option<String>)>,
    pub skin: SkinPatch,
}
/// `PlayerSkin.Patch`: the optional body, cape and elytra texture ids and the
/// model type (`true` for slim).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkinPatch {
    pub body: Option<String>,
    pub cape: Option<String>,
    pub elytra: Option<String>,
    pub slim: Option<bool>,
}
impl SkinPatch {
    fn read(r: &mut Reader) -> Result<Self> {
        let texture = |r: &mut Reader| -> Result<Option<String>> {
            if r.bool()? {
                Ok(Some(r.string(32767)?))
            } else {
                Ok(None)
            }
        };
        let (body, cape, elytra) = (texture(r)?, texture(r)?, texture(r)?);
        let slim = if r.bool()? { Some(r.bool()?) } else { None };
        Ok(Self {
            body,
            cape,
            elytra,
            slim,
        })
    }
    fn write(&self, w: &mut Writer) {
        for texture in [&self.body, &self.cape, &self.elytra] {
            w.bool(texture.is_some());
            if let Some(texture) = texture {
                w.string(texture);
            }
        }
        w.bool(self.slim.is_some());
        if let Some(slim) = self.slim {
            w.bool(slim);
        }
    }
}
impl Component {
    pub(crate) fn read(kind: i32, r: &mut Reader) -> Result<Self> {
        use component::*;
        Ok(match kind {
            MAX_STACK_SIZE
            | MAX_DAMAGE
            | DAMAGE
            | RARITY
            | REPAIR_COST
            | MAP_ID
            | MAP_POST_PROCESSING
            | OMINOUS_BOTTLE_AMPLIFIER => Self::Int(r.varint()?),
            WRITABLE_BOOK_CONTENT => {
                let n = r.count(100)?;
                let mut pages = (0..n)
                    .map(|_| filterable(r, |r| r.string(1024)))
                    .collect::<Result<Vec<_>>>()?;
                let (pages, filtered) = filtered_variants(&mut pages);
                Self::WritableBook { pages, filtered }
            }
            WRITTEN_BOOK_CONTENT => {
                let (title, title_filtered) = filterable(r, |r| r.string(32))?;
                let author = r.string(32767)?;
                let generation = r.varint()?;
                let n = r.count(r.remaining())?;
                let mut pages = (0..n)
                    .map(|_| filterable(r, |r| r.nbt()))
                    .collect::<Result<Vec<_>>>()?;
                let (pages, pages_filtered) = filtered_variants(&mut pages);
                Self::WrittenBook(WrittenBook {
                    title,
                    title_filtered,
                    author,
                    generation,
                    pages,
                    pages_filtered,
                    resolved: r.bool()?,
                })
            }
            CHARGED_PROJECTILES | BUNDLE_CONTENTS => {
                let n = r.count(1024)?;
                Self::Stacks((0..n).map(|_| Template::read(r)).collect::<Result<_>>()?)
            }
            DYED_COLOR => Self::Int(r.i32()?),
            UNBREAKABLE | CREATIVE_SLOT_LOCK => Self::Unit,
            ENCHANTMENT_GLINT_OVERRIDE => Self::Bool(r.bool()?),
            CUSTOM_NAME | ITEM_NAME => Self::Text(r.nbt()?),
            CUSTOM_DATA => Self::Nbt(r.nbt()?),
            ITEM_MODEL => Self::Ident(r.string(32767)?),
            LORE => {
                let n = r.count(256)?;
                Self::Lore((0..n).map(|_| r.nbt()).collect::<Result<_>>()?)
            }
            // SuspiciousStewEffects.Entry.STREAM_CODEC: MobEffect.STREAM_CODEC
            // (the registry id), then the duration, both varints.
            SUSPICIOUS_STEW_EFFECTS => {
                let n = r.count(r.remaining())?;
                Self::StewEffects(
                    (0..n)
                        .map(|_| Ok((r.varint()?, r.varint()?)))
                        .collect::<Result<_>>()?,
                )
            }
            ENCHANTMENTS | STORED_ENCHANTMENTS => {
                let n = r.count(r.remaining())?;
                Self::Enchantments(
                    (0..n)
                        .map(|_| Ok((r.varint()?, r.varint()?)))
                        .collect::<Result<_>>()?,
                )
            }
            POTION_CONTENTS => {
                let potion = if r.bool()? { Some(r.varint()?) } else { None };
                let color = if r.bool()? { Some(r.i32()?) } else { None };
                let start = r.position();
                skip::potion_effects_and_name(r)?;
                // No custom effects and no name: an empty tail.
                let rest = match r.since(start) {
                    [0, 0] => Vec::new(),
                    bytes => bytes.to_vec(),
                };
                Self::Potion {
                    potion,
                    color,
                    rest,
                }
            }
            ATTACK_RANGE => {
                let mut values = [0.0f32; 6];
                for v in &mut values {
                    *v = r.f32()?;
                }
                Self::AttackRange(values)
            }
            PROFILE => {
                fn properties(r: &mut Reader) -> Result<Vec<(String, String, Option<String>)>> {
                    let n = r.count(1024)?;
                    let mut out = Vec::with_capacity(n);
                    for _ in 0..n {
                        let name = r.string(32767)?;
                        let value = r.string(32767)?;
                        let signature = if r.bool()? {
                            Some(r.string(32767)?)
                        } else {
                            None
                        };
                        out.push((name, value, signature));
                    }
                    Ok(out)
                }
                // Either<GameProfile, Partial>: the partial form carries
                // optional fields, the resolved one a UUID, a name and the
                // properties.
                let (name, id, properties) = if r.u8()? == 0 {
                    let name = if r.bool()? { Some(r.string(64)?) } else { None };
                    let id = if r.bool()? { Some(r.uuid()?) } else { None };
                    (name, id, properties(r)?)
                } else {
                    let id = Some(r.uuid()?);
                    let name = Some(r.string(64)?);
                    (name, id, properties(r)?)
                };
                // PlayerSkin.Patch: three texture ids and the model type.
                Self::Profile(Profile {
                    name,
                    id,
                    properties,
                    skin: SkinPatch::read(r)?,
                })
            }
            CUSTOM_MODEL_DATA => {
                let n = r.count(r.remaining())?;
                let floats = (0..n).map(|_| r.f32()).collect::<Result<_>>()?;
                let n = r.count(r.remaining())?;
                let flags = (0..n).map(|_| r.bool()).collect::<Result<_>>()?;
                let n = r.count(r.remaining())?;
                let strings = (0..n).map(|_| r.string(32767)).collect::<Result<_>>()?;
                let n = r.count(r.remaining())?;
                let colors = (0..n).map(|_| r.i32()).collect::<Result<_>>()?;
                Self::CustomModelData {
                    floats,
                    flags,
                    strings,
                    colors,
                }
            }
            TOOLTIP_DISPLAY => {
                let hide = r.bool()?;
                let n = r.count(r.remaining())?;
                Self::TooltipDisplay {
                    hide,
                    hidden: (0..n).map(|_| r.varint()).collect::<Result<_>>()?,
                }
            }
            FIREWORKS => {
                let flight = r.varint()?;
                let n = r.count(256)?;
                Self::Fireworks {
                    flight,
                    explosions: (0..n)
                        .map(|_| FireworkExplosion::read(r))
                        .collect::<Result<_>>()?,
                }
            }
            FIREWORK_EXPLOSION => Self::FireworkExplosion(FireworkExplosion::read(r)?),
            LODESTONE_TRACKER => {
                let target = if r.bool()? {
                    Some((r.string(32767)?, unpack_block_pos(r.i64()?)))
                } else {
                    None
                };
                Self::LodestoneTracker {
                    target,
                    tracked: r.bool()?,
                }
            }
            // ArmorTrim.STREAM_CODEC: two holders, each a reference's id plus
            // one. A direct material or pattern has no registry name: read past.
            TRIM => {
                let start = r.position();
                skip::component(TRIM, r)?;
                let bytes = r.since(start);
                let mut ids = Reader::new(bytes);
                match (ids.varint(), ids.varint()) {
                    (Ok(material @ 1..), Ok(pattern @ 1..)) if ids.remaining() == 0 => Self::Trim {
                        material: material - 1,
                        pattern: pattern - 1,
                    },
                    _ => Self::Raw(bytes.to_vec()),
                }
            }
            // InstrumentComponent.STREAM_CODEC: a holder, a reference's id plus one.
            // A direct instrument has no registry name: read past.
            INSTRUMENT => {
                let start = r.position();
                skip::component(INSTRUMENT, r)?;
                let bytes = r.since(start);
                let mut ids = Reader::new(bytes);
                match ids.varint() {
                    Ok(id @ 1..) if ids.remaining() == 0 => Self::Instrument(id - 1),
                    _ => Self::Raw(bytes.to_vec()),
                }
            }
            // Holder<PaintingVariant>: a reference's id plus one, or 0 and
            // the variant itself (no registry name: read past).
            PAINTING_VARIANT => {
                let start = r.position();
                skip::component(PAINTING_VARIANT, r)?;
                let bytes = r.since(start);
                let mut ids = Reader::new(bytes);
                match ids.varint() {
                    Ok(id @ 1..) if ids.remaining() == 0 => Self::PaintingVariant(id - 1),
                    _ => Self::Raw(bytes.to_vec()),
                }
            }
            // DyeColor.STREAM_CODEC: its id.
            BASE_COLOR => Self::Int(r.varint()?),
            // BannerPatternLayers.STREAM_CODEC: a list of a pattern holder (a
            // reference's id plus one, or 0 and the pattern itself) and a dye.
            BANNER_PATTERNS => {
                let start = r.position();
                skip::component(BANNER_PATTERNS, r)?;
                let bytes = r.since(start);
                match Self::banner_layers(bytes) {
                    Some(layers) => Self::BannerPatterns(layers),
                    None => Self::Raw(bytes.to_vec()),
                }
            }
            other if custom::contains(other) => Self::Nbt(r.nbt()?),
            other => {
                let start = r.position();
                skip::component(other, r)?;
                Self::Raw(r.since(start).to_vec())
            }
        })
    }
    /// The layers of encoded `BannerPatternLayers` whose patterns are all
    /// registry references (`None` when one is direct).
    fn banner_layers(bytes: &[u8]) -> Option<Vec<(i32, i32)>> {
        let mut r = Reader::new(bytes);
        let n = r.count(r.remaining()).ok()?;
        let mut layers = Vec::with_capacity(n);
        for _ in 0..n {
            let pattern = r.varint().ok()?;
            let color = r.varint().ok()?;
            if pattern < 1 {
                return None;
            }
            layers.push((pattern - 1, color));
        }
        (r.remaining() == 0).then_some(layers)
    }
    pub(crate) fn write(&self, kind: i32, w: &mut Writer) {
        match self {
            Self::Raw(bytes) => w.bytes(bytes),
            Self::BannerPatterns(layers) => {
                w.varint(layers.len() as i32);
                for (pattern, color) in layers {
                    w.varint(pattern + 1);
                    w.varint(*color);
                }
            }
            Self::Trim { material, pattern } => {
                w.varint(material + 1);
                w.varint(pattern + 1);
            }
            Self::Instrument(id) => w.varint(id + 1),
            Self::PaintingVariant(id) => w.varint(id + 1),
            Self::Int(v) if kind == component::DYED_COLOR => w.i32(*v),
            Self::Int(v) => w.varint(*v),
            Self::Unit => {}
            Self::Bool(v) => w.bool(*v),
            Self::Text(t) | Self::Nbt(t) => w.nbt(t),
            Self::Ident(s) => w.string(s),
            Self::Lore(lines) => {
                w.varint(lines.len() as i32);
                lines.iter().for_each(|l| w.nbt(l));
            }
            Self::Enchantments(list) | Self::StewEffects(list) => {
                w.varint(list.len() as i32);
                for (e, level) in list {
                    w.varint(*e);
                    w.varint(*level);
                }
            }
            Self::AttackRange(values) => values.iter().for_each(|v| w.f32(*v)),
            Self::Potion {
                potion,
                color,
                rest,
            } => {
                w.bool(potion.is_some());
                if let Some(id) = potion {
                    w.varint(*id);
                }
                w.bool(color.is_some());
                if let Some(color) = color {
                    w.i32(*color);
                }
                // An empty tail is no custom effects and no name.
                if rest.is_empty() {
                    w.varint(0);
                    w.bool(false);
                } else {
                    w.bytes(rest);
                }
            }
            Self::Profile(p) => {
                w.u8(0);
                w.bool(p.name.is_some());
                if let Some(name) = &p.name {
                    w.string(name);
                }
                w.bool(p.id.is_some());
                if let Some(id) = p.id {
                    w.uuid(id);
                }
                w.varint(p.properties.len() as i32);
                for (name, value, signature) in &p.properties {
                    w.string(name);
                    w.string(value);
                    w.bool(signature.is_some());
                    if let Some(signature) = signature {
                        w.string(signature);
                    }
                }
                p.skin.write(w);
            }
            Self::CustomModelData {
                floats,
                flags,
                strings,
                colors,
            } => {
                w.varint(floats.len() as i32);
                floats.iter().for_each(|f| w.f32(*f));
                w.varint(flags.len() as i32);
                flags.iter().for_each(|f| w.bool(*f));
                w.varint(strings.len() as i32);
                strings.iter().for_each(|s| w.string(s));
                w.varint(colors.len() as i32);
                colors.iter().for_each(|c| w.i32(*c));
            }
            Self::TooltipDisplay { hide, hidden } => {
                w.bool(*hide);
                w.varint(hidden.len() as i32);
                hidden.iter().for_each(|h| w.varint(*h));
            }
            Self::Stacks(stacks) => {
                w.varint(stacks.len() as i32);
                stacks.iter().for_each(|s| Template::write(s, w));
            }
            Self::Fireworks { flight, explosions } => {
                w.varint(*flight);
                w.varint(explosions.len() as i32);
                explosions.iter().for_each(|e| e.write(w));
            }
            Self::FireworkExplosion(e) => e.write(w),
            Self::LodestoneTracker { target, tracked } => {
                w.bool(target.is_some());
                if let Some((dimension, pos)) = target {
                    w.string(dimension);
                    w.i64(pack_block_pos(*pos));
                }
                w.bool(*tracked);
            }
            Self::WritableBook { pages, filtered } => {
                w.varint(pages.len() as i32);
                for (i, page) in pages.iter().enumerate() {
                    w.string(page);
                    match filtered.get(i).and_then(Option::as_ref) {
                        Some(f) => {
                            w.bool(true);
                            w.string(f);
                        }
                        None => w.bool(false),
                    }
                }
            }
            Self::WrittenBook(b) => {
                w.string(&b.title);
                match &b.title_filtered {
                    Some(f) => {
                        w.bool(true);
                        w.string(f);
                    }
                    None => w.bool(false),
                }
                w.string(&b.author);
                w.varint(b.generation);
                w.varint(b.pages.len() as i32);
                for (i, page) in b.pages.iter().enumerate() {
                    w.nbt(page);
                    match b.pages_filtered.get(i).and_then(Option::as_ref) {
                        Some(f) => {
                            w.bool(true);
                            w.nbt(f);
                        }
                        None => w.bool(false),
                    }
                }
                w.bool(b.resolved);
            }
        }
    }
}

/// A component patch as `DelimitedSlot` writes it (both counts, then each
/// value behind its length), kept from a stack that was read so one nobody
/// edited goes back byte for byte. It is no part of a stack's equality.
#[derive(Clone, Debug, Default)]
pub struct WirePatch(pub Option<Arc<[u8]>>);
impl WirePatch {
    /// Whether a patch in this form sets component `kind`.
    pub fn adds(patch: &[u8], kind: i32) -> bool {
        let mut r = Reader::new(patch);
        let Ok(positive) = r.count(65536) else {
            return false;
        };
        if r.count(65536).is_err() {
            return false;
        }
        (0..positive).any(|_| {
            let Ok(k) = r.varint() else {
                return false;
            };
            let Ok(len) = r.count(r.remaining()) else {
                return false;
            };
            r.bytes(len).is_ok() && k == kind
        })
    }
}
impl WirePatch {
    /// The components and removals a patch in this form holds, read as
    /// [`DelimitedSlot`] does.
    pub fn decode(patch: &[u8]) -> Result<(Vec<(i32, Component)>, Vec<i32>)> {
        let mut r = Reader::new(patch);
        let positive = r.count(65536)?;
        let negative = r.count(65536)?;
        let mut added = Vec::with_capacity(positive.min(64));
        for _ in 0..positive {
            let kind = r.varint()?;
            let len = r.count(r.remaining())?;
            added.push((
                kind,
                Component::read(kind, &mut Reader::new(r.bytes(len)?))?,
            ));
        }
        let removed = (0..negative).map(|_| r.varint()).collect::<Result<_>>()?;
        Ok((added, removed))
    }
}
impl PartialEq for WirePatch {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// A non-empty item stack.
#[derive(Clone, Debug, PartialEq)]
pub struct Stack {
    pub item: i32,
    pub count: i32,
    pub added: Vec<(i32, Component)>,
    pub removed: Vec<i32>,
    /// The patch as it was read; [`DelimitedSlot`] writes it in place of
    /// `added` and `removed` when set. Only for a stack nobody has edited.
    pub wire: WirePatch,
}
impl Stack {
    pub fn new(item: i32, count: i32) -> Self {
        Self {
            item,
            count,
            added: Vec::new(),
            removed: Vec::new(),
            wire: WirePatch::default(),
        }
    }
    pub fn component(&self, kind: i32) -> Option<&Component> {
        self.added.iter().find(|(k, _)| *k == kind).map(|(_, c)| c)
    }
    pub fn damage(&self) -> i32 {
        match self.component(component::DAMAGE) {
            Some(Component::Int(d)) => *d,
            _ => 0,
        }
    }
}
/// ItemStack.OPTIONAL_STREAM_CODEC: `None` is the empty stack (count 0).
pub struct Slot;
impl Codec<Option<Stack>> for Slot {
    fn read(r: &mut Reader) -> Result<Option<Stack>> {
        read_stack(r, false)
    }
    fn write(v: &Option<Stack>, w: &mut Writer) {
        let Some(s) = v else {
            w.varint(0);
            return;
        };
        w.varint(s.count);
        w.varint(s.item);
        w.varint(s.added.len() as i32);
        w.varint(s.removed.len() as i32);
        for (kind, c) in &s.added {
            w.varint(*kind);
            c.write(*kind, w);
        }
        for kind in &s.removed {
            w.varint(*kind);
        }
    }
}

/// A stack of either optional codec: with `delimited` each component value
/// has a length in front (DataComponentPatch.DELIMITED_STREAM_CODEC). The
/// patch is also kept, in the delimited form, for [`Stack::wire`].
fn read_stack(r: &mut Reader, delimited: bool) -> Result<Option<Stack>> {
    let count = r.varint()?;
    if count <= 0 {
        return Ok(None);
    }
    let item = r.varint()?;
    let positive = r.count(65536)?;
    let negative = r.count(65536)?;
    let mut added = Vec::with_capacity(positive.min(64));
    let mut patch = Writer::new();
    patch.varint(positive as i32);
    patch.varint(negative as i32);
    for _ in 0..positive {
        let kind = r.varint()?;
        let (component, value) = if delimited {
            let len = r.count(r.remaining())?;
            let value = r.bytes(len)?;
            let mut component_reader = Reader::new(value);
            let component = Component::read(kind, &mut component_reader)?;
            if custom::contains(kind) && component_reader.remaining() != 0 {
                return Err(crate::Error::new("trailing custom component data"));
            }
            (component, value)
        } else {
            let start = r.position();
            (Component::read(kind, r)?, r.since(start))
        };
        patch.varint(kind);
        patch.byte_array(value);
        added.push((kind, component));
    }
    let removed: Vec<i32> = (0..negative).map(|_| r.varint()).collect::<Result<_>>()?;
    removed.iter().for_each(|kind| patch.varint(*kind));
    Ok(Some(Stack {
        item,
        count,
        added,
        removed,
        wire: WirePatch((positive + negative > 0).then(|| patch.data.into())),
    }))
}

/// ItemStack.OPTIONAL_UNTRUSTED_STREAM_CODEC (`SetCreativeModeSlot`): as [`Slot`],
/// but every component value goes behind its byte length
/// (DataComponentPatch.DELIMITED_STREAM_CODEC), so a server skips what it can't read.
pub struct DelimitedSlot;
impl Codec<Option<Stack>> for DelimitedSlot {
    fn read(r: &mut Reader) -> Result<Option<Stack>> {
        read_stack(r, true)
    }
    fn write(v: &Option<Stack>, w: &mut Writer) {
        let Some(s) = v else {
            w.varint(0);
            return;
        };
        w.varint(s.count);
        w.varint(s.item);
        if let Some(patch) = &s.wire.0 {
            w.bytes(patch);
            return;
        }
        w.varint(s.added.len() as i32);
        w.varint(s.removed.len() as i32);
        for (kind, c) in &s.added {
            w.varint(*kind);
            let mut value = Writer::new();
            c.write(*kind, &mut value);
            w.byte_array(&value.data);
        }
        for kind in &s.removed {
            w.varint(*kind);
        }
    }
}

/// ItemStackTemplate.STREAM_CODEC: the item, the count, then the patch.
pub struct Template;
impl Codec<Stack> for Template {
    fn read(r: &mut Reader) -> Result<Stack> {
        let item = r.varint()?;
        let count = r.varint()?;
        let positive = r.count(65536)?;
        let negative = r.count(65536)?;
        let mut added = Vec::with_capacity(positive.min(64));
        for _ in 0..positive {
            let kind = r.varint()?;
            added.push((kind, Component::read(kind, r)?));
        }
        let removed = (0..negative).map(|_| r.varint()).collect::<Result<_>>()?;
        Ok(Stack {
            item,
            count,
            added,
            removed,
            wire: WirePatch::default(),
        })
    }
    fn write(s: &Stack, w: &mut Writer) {
        w.varint(s.item);
        w.varint(s.count);
        w.varint(s.added.len() as i32);
        w.varint(s.removed.len() as i32);
        for (kind, c) in &s.added {
            w.varint(*kind);
            c.write(*kind, w);
        }
        for kind in &s.removed {
            w.varint(*kind);
        }
    }
}

/// HashedStack: what the client believes a slot holds. Component values are
/// sent as hashes ([`Hashed::create`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Hashed {
    pub item: i32,
    pub count: i32,
    pub added: Vec<(i32, i32)>,
    pub removed: Vec<i32>,
}
impl Hashed {
    /// The stack hashed without registry names: only components that hold no
    /// registry id get a hash (see [`Hashed::create`]).
    pub fn of(stack: &Stack) -> Self {
        Self::create(stack, &NoNames)
    }
}
pub struct HashedSlot;
impl Codec<Option<Hashed>> for HashedSlot {
    fn read(r: &mut Reader) -> Result<Option<Hashed>> {
        if !r.bool()? {
            return Ok(None);
        }
        let item = r.varint()?;
        let count = r.varint()?;
        let n = r.count(256)?;
        let added = (0..n)
            .map(|_| Ok((r.varint()?, r.i32()?)))
            .collect::<Result<_>>()?;
        let n = r.count(256)?;
        let removed = (0..n).map(|_| r.varint()).collect::<Result<_>>()?;
        Ok(Some(Hashed {
            item,
            count,
            added,
            removed,
        }))
    }
    fn write(v: &Option<Hashed>, w: &mut Writer) {
        w.bool(v.is_some());
        if let Some(h) = v {
            w.varint(h.item);
            w.varint(h.count);
            w.varint(h.added.len() as i32);
            for (k, hash) in &h.added {
                w.varint(*k);
                w.i32(*hash);
            }
            w.varint(h.removed.len() as i32);
            for k in &h.removed {
                w.varint(*k);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::literal;

    fn round<T: PartialEq + std::fmt::Debug, C: Codec<T>>(v: T) {
        let mut w = Writer::new();
        C::write(&v, &mut w);
        let mut r = Reader::new(&w.data);
        assert_eq!(C::read(&mut r).unwrap(), v);
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn a_wire_patch_tells_what_it_adds() {
        // Two added components (damage 3, then can_break's bytes as read)
        // and one removed.
        let mut w = Writer::new();
        w.varint(2);
        w.varint(1);
        w.varint(component::DAMAGE);
        w.byte_array(&[3]);
        w.varint(component::CAN_BREAK);
        w.byte_array(&[1, 0, 0, 0, 0, 0]);
        w.varint(component::ENCHANTMENTS);
        assert!(WirePatch::adds(&w.data, component::DAMAGE));
        assert!(WirePatch::adds(&w.data, component::CAN_BREAK));
        assert!(!WirePatch::adds(&w.data, component::ENCHANTMENTS));
        assert!(!WirePatch::adds(&[], component::CAN_BREAK));
    }

    #[test]
    fn potion_contents_round_trip() {
        let mut arrow = Stack::new(7, 3);
        arrow.added.push((
            component::POTION_CONTENTS,
            Component::Potion {
                potion: Some(31),
                color: None,
                rest: Vec::new(),
            },
        ));
        round::<Option<Stack>, Slot>(Some(arrow));
        // A custom colour, one custom effect (speed II for 600 ticks, shown,
        // no hidden effect) and a custom name survive as they came.
        let mut w = Writer::new();
        w.bool(false);
        w.bool(true);
        w.i32(0x00FF_8800);
        w.varint(1);
        for v in [1, 1, 600] {
            w.varint(v);
        }
        for b in [false, true, true, false] {
            w.bool(b);
        }
        w.bool(true);
        w.string("Tea");
        let mut r = Reader::new(&w.data);
        let read = Component::read(component::POTION_CONTENTS, &mut r).unwrap();
        assert_eq!(r.remaining(), 0);
        let Component::Potion {
            potion,
            color,
            rest,
        } = &read
        else {
            panic!("{read:?}");
        };
        assert_eq!((*potion, *color), (None, Some(0x00FF_8800)));
        assert!(!rest.is_empty());
        let mut again = Writer::new();
        read.write(component::POTION_CONTENTS, &mut again);
        assert_eq!(again.data, w.data);
    }

    #[test]
    fn stacks_round_trip() {
        round::<Option<Stack>, Slot>(None);
        round::<Option<Stack>, Slot>(Some(Stack::new(42, 64)));
        let mut sword = Stack::new(900, 1);
        sword.added.push((component::DAMAGE, Component::Int(17)));
        sword.added.push((
            component::CUSTOM_NAME,
            Component::Text(literal("Excalibur")),
        ));
        sword.added.push((
            component::ENCHANTMENTS,
            Component::Enchantments(vec![(5, 3), (12, 1)]),
        ));
        sword
            .added
            .push((component::DYED_COLOR, Component::Int(-6265536)));
        sword.added.push((
            component::LORE,
            Component::Lore(vec![literal("a"), literal("b")]),
        ));
        sword.removed.push(component::REPAIR_COST);
        let mut book = Stack::new(1100, 1);
        book.added.push((
            component::WRITTEN_BOOK_CONTENT,
            Component::WrittenBook(WrittenBook {
                title: "Notes".into(),
                author: "Guest".into(),
                generation: 1,
                pages: vec![literal("one"), literal("two")],
                pages_filtered: vec![None, Some(literal("2"))],
                title_filtered: Some("N".into()),
                resolved: true,
            }),
        ));
        book.added
            .push((component::MAP_POST_PROCESSING, Component::Int(1)));
        round::<Option<Stack>, Slot>(Some(book));
        // OminousBottleAmplifier.STREAM_CODEC: a VarInt.
        let mut bottle = Stack::new(1200, 1);
        bottle
            .added
            .push((component::OMINOUS_BOTTLE_AMPLIFIER, Component::Int(3)));
        let mut w = Writer::new();
        Slot::write(&Some(bottle.clone()), &mut w);
        assert_eq!(w.data, [1, 0xb0, 0x09, 1, 0, 65, 3]);
        round::<Option<Stack>, Slot>(Some(bottle));
        let mut quill = Stack::new(1099, 1);
        quill.added.push((
            component::WRITABLE_BOOK_CONTENT,
            Component::WritableBook {
                pages: vec!["a".into(), String::new()],
                filtered: vec![None, Some("b".into())],
            },
        ));
        round::<Option<Stack>, Slot>(Some(quill));
        assert_eq!(sword.damage(), 17);
        round::<Option<Stack>, Slot>(Some(sword.clone()));
        round::<Option<Hashed>, HashedSlot>(Some(Hashed::of(&sword)));
        round::<Option<Hashed>, HashedSlot>(None);
    }

    #[test]
    fn fireworks_components_round_trip() {
        let mut rocket = Stack::new(42, 1);
        rocket.added.push((
            component::FIREWORKS,
            Component::Fireworks {
                flight: 3,
                explosions: vec![
                    FireworkExplosion {
                        shape: 4,
                        colors: vec![0xff0000, 0x00ff00],
                        fade: vec![0x0000ff],
                        trail: true,
                        twinkle: false,
                    },
                    FireworkExplosion {
                        shape: 0,
                        colors: vec![],
                        fade: vec![],
                        trail: false,
                        twinkle: true,
                    },
                ],
            },
        ));
        rocket.added.push((
            component::FIREWORK_EXPLOSION,
            Component::FireworkExplosion(FireworkExplosion {
                shape: 2,
                colors: vec![7],
                fade: vec![],
                trail: false,
                twinkle: false,
            }),
        ));
        round::<Option<Stack>, Slot>(Some(rocket));
    }

    #[test]
    fn charged_projectiles_are_item_stack_templates() {
        // A crossbow (id 7) loaded with one arrow (id 9): ChargedProjectiles
        // is a list of ItemStackTemplate, the item before the count.
        let bytes = [1, 7, 1, 0, 51, 1, 9, 1, 0, 0];
        let stack = Slot::read(&mut Reader::new(&bytes)).unwrap().unwrap();
        assert_eq!(
            stack.component(component::CHARGED_PROJECTILES),
            Some(&Component::Stacks(vec![Stack::new(9, 1)]))
        );
        let mut w = Writer::new();
        Slot::write(&Some(stack), &mut w);
        assert_eq!(w.data, bytes);
    }

    #[test]
    fn bundle_contents_are_item_stack_templates() {
        // A bundle (id 7) holding 32 of item 9: BundleContents is a list of
        // ItemStackTemplate, as ChargedProjectiles is.
        let bytes = [1, 7, 1, 0, 52, 1, 9, 32, 0, 0];
        let stack = Slot::read(&mut Reader::new(&bytes)).unwrap().unwrap();
        assert_eq!(
            stack.component(component::BUNDLE_CONTENTS),
            Some(&Component::Stacks(vec![Stack::new(9, 32)]))
        );
        let mut w = Writer::new();
        Slot::write(&Some(stack), &mut w);
        assert_eq!(w.data, bytes);
    }

    #[test]
    fn filtered_book_pages_are_read_and_kept() {
        // A writable book with one page "hi" and its filtered "**".
        let bytes = [1, 7, 1, 0, 56, 1, 2, b'h', b'i', 1, 2, b'*', b'*'];
        let stack = Slot::read(&mut Reader::new(&bytes)).unwrap().unwrap();
        assert_eq!(
            stack.component(component::WRITABLE_BOOK_CONTENT),
            Some(&Component::WritableBook {
                pages: vec!["hi".into()],
                filtered: vec![Some("**".into())],
            })
        );
    }

    #[test]
    fn empty_stack_is_a_zero_count() {
        let mut w = Writer::new();
        Slot::write(&None, &mut w);
        assert_eq!(w.data, [0]);
        let mut w = Writer::new();
        Slot::write(&Some(Stack::new(1, 2)), &mut w);
        assert_eq!(w.data, [2, 1, 0, 0]);
    }

    #[test]
    fn unknown_components_are_errors() {
        // 122 is past DataComponents' last id (cushion_color, 121).
        let bytes = [1, 5, 1, 0, 122];
        assert!(Slot::read(&mut Reader::new(&bytes)).is_err());
    }

    #[test]
    fn attack_range_and_profiles_round_trip() {
        let mut sword = Stack::new(900, 1);
        sword.added.push((
            component::ATTACK_RANGE,
            Component::AttackRange([0.0, 3.0, 0.0, 4.0, 0.1, 1.0]),
        ));
        sword.added.push((
            component::PROFILE,
            Component::Profile(Profile {
                name: Some("Player53470".into()),
                id: Some(0x7a5d_6a1a_30d7_4187_88c5_efe9_1201_8922),
                properties: vec![("textures".into(), "dGV4dHVyZXM=".into(), None)],
                skin: SkinPatch {
                    body: Some("minecraft:entity/player/wide/steve".into()),
                    cape: None,
                    elytra: None,
                    slim: Some(true),
                },
            }),
        ));
        round::<Option<Stack>, Slot>(Some(sword));
    }

    #[test]
    fn a_partial_profile_decodes_from_the_wire() {
        // count 1, item 1, one added component of kind 72 (profile): a partial
        // profile with an empty name, a UUID, one unsigned texture property
        // and an empty skin patch.
        let uuid = [
            0x7a, 0x5d, 0x6a, 0x1a, 0x30, 0xd7, 0x41, 0x87, 0x88, 0xc5, 0xef, 0xe9, 0x12, 0x01,
            0x89, 0x22,
        ];
        let mut bytes = vec![1, 1, 1, 0, 72, 0, 1, 0, 1];
        bytes.extend_from_slice(&uuid);
        bytes.extend_from_slice(&[1, 8]);
        bytes.extend_from_slice(b"textures");
        bytes.extend_from_slice(&[1, b'x', 0, 0, 0, 0, 0]);
        let mut r = Reader::new(&bytes);
        let stack = Slot::read(&mut r).unwrap().unwrap();
        assert_eq!(r.remaining(), 0);
        let Some(Component::Profile(profile)) = stack.component(component::PROFILE) else {
            panic!("the profile component did not decode");
        };
        assert_eq!(profile.name.as_deref(), Some(""));
        assert_eq!(profile.id, Some(0x7a5d_6a1a_30d7_4187_88c5_efe9_1201_8922));
        assert_eq!(
            profile.properties,
            [("textures".to_owned(), "x".to_owned(), None)]
        );
    }

    #[test]
    fn components_read_past_keep_their_bytes() {
        // Vanilla 26.3's encoding (tools/test_item_wire_parity.py) of a chestplate's
        // armour trim (decoded: both holders are references), a banner's patterns
        // (one direct), a pot's sherds and a stew's effects: one server's
        // inventory, recipe book and advancements had them.
        for (hex, kind) in [
            ("01bd0801003a0502", 58),
            ("01890b01004a02200e000b6d696e6563726166743a7801790b", 74),
            ("01880301004c01f60801000001be0c0100000001d30c010000", 76),
            ("01a408010037020f6412a001", 55),
            // A goat horn's instrument by reference (InstrumentComponent's holder).
            ("01bd0801003f02", 63),
        ] {
            let bytes: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            let mut r = Reader::new(&bytes);
            let stack = Slot::read(&mut r).unwrap();
            assert_eq!(r.remaining(), 0, "component {kind}");
            let expected = |c: &Component| match kind {
                58 => {
                    *c == Component::Trim {
                        material: 4,
                        pattern: 1,
                    }
                }
                // night_vision for 100 ticks, poison for the default 160.
                55 => *c == Component::StewEffects(vec![(15, 100), (18, 160)]),
                63 => *c == Component::Instrument(1),
                _ => matches!(c, Component::Raw(_)),
            };
            assert!(
                stack
                    .as_ref()
                    .and_then(|s| s.component(kind))
                    .is_some_and(expected)
            );
            let mut w = Writer::new();
            Slot::write(&stack, &mut w);
            assert_eq!(w.data, bytes, "component {kind}");
        }
    }

    #[test]
    fn painting_variants_decode_by_reference() {
        // Holder<PaintingVariant>: a reference (its id plus one) is typed; a
        // direct variant (0, width, height, asset id, no title, no author)
        // is read past. Both encode back to the same bytes.
        let asset = b"minecraft:x";
        let mut direct = vec![1u8, 7, 1, 0, 109, 0, 2, 1, asset.len() as u8];
        direct.extend_from_slice(asset);
        direct.extend_from_slice(&[0, 0]);
        for (bytes, expected) in [
            (
                vec![1u8, 7, 1, 0, 109, 5],
                Some(Component::PaintingVariant(4)),
            ),
            (direct, None),
        ] {
            let mut r = Reader::new(&bytes);
            let stack = Slot::read(&mut r).unwrap();
            assert_eq!(r.remaining(), 0);
            let c = stack
                .as_ref()
                .and_then(|s| s.component(component::PAINTING_VARIANT))
                .unwrap();
            match &expected {
                Some(e) => assert_eq!(c, e),
                None => assert!(matches!(c, Component::Raw(_))),
            }
            let mut w = Writer::new();
            Slot::write(&stack, &mut w);
            assert_eq!(w.data, bytes);
        }
    }
}
