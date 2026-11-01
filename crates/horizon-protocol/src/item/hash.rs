//! `HashedPatchMap.create` / `HashedStack.create`: each patched component is
//! hashed as its `Codec` form through `HashOps` (the client's
//! `decoratedHashOpsGenerator`: `component.encodeValue(hashOps).asInt()`).
//!
//! Horizon holds a component as the wire form (registry ids, no names), so the
//! codec form is rebuilt from it with a registry lookup for the ids that the
//! codec writes as identifiers. [`raw`] decodes the stream bytes of the components
//! Horizon keeps as bytes ([`Component::Raw`]). A component with no codec form
//! Horizon can build (vanilla has none, an id has no name, or a codec is not
//! ported yet) has no hash: it is left out of the added map and a vanilla server
//! resyncs that slot.
mod exact;
mod gameplay;
mod raw;
use super::{Component, FireworkExplosion, Hashed, Stack, WirePatch, component as kind};
use crate::hash_ops::{HashCode, HashOps, RecordHash};

/// Registry names for the ids a component holds.
pub trait Names {
    /// The name of entry `id` of `registry` (`minecraft:enchantment`, ...).
    fn name(&self, registry: &str, id: i32) -> Option<String>;
}
/// No registries: only the components that hold no registry id hash.
pub struct NoNames;
impl Names for NoNames {
    fn name(&self, _: &str, _: i32) -> Option<String> {
        None
    }
}

const RARITY: [&str; 4] = ["common", "uncommon", "rare", "epic"];
const DYE: [&str; 16] = [
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
/// `FireworkExplosion.Shape` names by id.
const SHAPE: [&str; 5] = ["small_ball", "large_ball", "star", "creeper", "burst"];
/// The `Codec.BOOL` fields of the text component codec (`Style`, `NbtContents`,
/// `PlayerSprite`).
const TEXT_BOOLS: [&str; 7] = [
    "bold",
    "italic",
    "underlined",
    "strikethrough",
    "obfuscated",
    "interpret",
    "plain",
];

fn string(s: &str) -> HashCode {
    HashOps::create_string(s)
}
/// `Identifier.CODEC` of registry entry `id`.
fn entry(names: &dyn Names, registry: &str, id: i32) -> Option<HashCode> {
    names.name(registry, id).map(|n| string(&n))
}
fn pick(table: &[&str], id: i32) -> Option<HashCode> {
    usize::try_from(id)
        .ok()
        .and_then(|i| table.get(i))
        .map(|n| string(n))
}
/// `ComponentSerialization.CODEC`.
fn text(tag: &horizon_world::nbt::Tag) -> HashCode {
    HashOps::from_nbt(tag, &TEXT_BOOLS)
}
/// `Filterable.codec`: the full record (`Codec.withAlternative` encodes with
/// the first codec), the filtered variant absent when there is none.
fn filterable(raw: HashCode, filtered: Option<HashCode>) -> HashCode {
    RecordHash::new()
        .field("raw", raw)
        .optional("filtered", filtered)
        .build()
}
/// `Codec.xxx.optionalFieldOf(name, default)`: absent when equal to the default.
fn or_default<T: PartialEq>(
    rec: RecordHash,
    name: &str,
    v: T,
    default: T,
    hash: HashCode,
) -> RecordHash {
    if v == default {
        rec
    } else {
        rec.field(name, hash)
    }
}
fn int_list(values: &[i32]) -> HashCode {
    HashOps::create_list(values.iter().map(|v| HashOps::create_int(*v)))
}
fn explosion(e: &FireworkExplosion) -> Option<HashCode> {
    let rec = RecordHash::new().field("shape", pick(&SHAPE, e.shape)?);
    let rec = or_default(
        rec,
        "colors",
        e.colors.is_empty(),
        true,
        int_list(&e.colors),
    );
    let rec = or_default(
        rec,
        "fade_colors",
        e.fade.is_empty(),
        true,
        int_list(&e.fade),
    );
    let rec = or_default(
        rec,
        "has_trail",
        e.trail,
        false,
        HashOps::create_boolean(e.trail),
    );
    Some(
        or_default(
            rec,
            "has_twinkle",
            e.twinkle,
            false,
            HashOps::create_boolean(e.twinkle),
        )
        .build(),
    )
}
/// `ItemEnchantments.CODEC`: an identifier-keyed map of levels.
fn enchantments(list: &[(i32, i32)], names: &dyn Names) -> Option<HashCode> {
    let mut entries = Vec::with_capacity(list.len());
    for (id, level) in list {
        entries.push((
            entry(names, "minecraft:enchantment", *id)?,
            HashOps::create_int(*level),
        ));
    }
    Some(HashOps::create_map(entries))
}
/// `ItemStackTemplate.CODEC` (the map form): the item, a count that is absent
/// at 1 and a `DataComponentPatch.CODEC` that is absent when empty.
fn template(s: &Stack, names: &dyn Names) -> Option<HashCode> {
    let rec = RecordHash::new().field("id", entry(names, "minecraft:item", s.item)?);
    let rec = or_default(rec, "count", s.count, 1, HashOps::create_int(s.count));
    let mut patch = Vec::new();
    for (k, c) in &s.added {
        patch.push((
            entry(names, "minecraft:data_component_type", *k)?,
            c.code(*k, names)?,
        ));
    }
    for k in &s.removed {
        let name = names.name("minecraft:data_component_type", *k)?;
        patch.push((string(&format!("!{name}")), HashOps::empty_map()));
    }
    Some(
        if patch.is_empty() {
            rec
        } else {
            rec.field("components", HashOps::create_map(patch))
        }
        .build(),
    )
}

impl Component {
    /// The component's `Codec` form hashed (`TypedDataComponent.encodeValue(hashOps)`),
    /// or `None` when Horizon cannot build that form.
    pub fn code(&self, id: i32, names: &dyn Names) -> Option<HashCode> {
        Some(match self {
            Self::Int(v) => match id {
                kind::RARITY => pick(&RARITY, *v)?,
                kind::BASE_COLOR => pick(&DYE, *v)?,
                // MapPostProcessing has no persistent codec.
                kind::MAP_POST_PROCESSING => return None,
                _ => HashOps::create_int(*v),
            },
            // `Unit.CODEC` is an empty map; creative_slot_lock has no codec.
            Self::Unit if id == kind::CREATIVE_SLOT_LOCK => return None,
            Self::Unit => HashOps::empty_map(),
            Self::Bool(v) => HashOps::create_boolean(*v),
            Self::Text(t) => text(t),
            Self::Nbt(t) => HashOps::from_nbt(t, &[]),
            Self::Ident(s) => string(s),
            Self::Lore(lines) => HashOps::create_list(lines.iter().map(text)),
            Self::Enchantments(list) => enchantments(list, names)?,
            Self::AttackRange(v) => {
                const FIELDS: [(&str, f32); 6] = [
                    ("min_reach", 0.0),
                    ("max_reach", 3.0),
                    ("min_creative_reach", 0.0),
                    ("max_creative_reach", 5.0),
                    ("hitbox_margin", 0.3),
                    ("mob_factor", 1.0),
                ];
                let mut rec = RecordHash::new();
                for ((name, default), v) in FIELDS.iter().zip(v) {
                    rec = or_default(
                        rec,
                        name,
                        v.to_bits(),
                        default.to_bits(),
                        HashOps::create_f32(*v),
                    );
                }
                rec.build()
            }
            Self::Potion {
                potion,
                color,
                rest,
            } => {
                let rec = RecordHash::new()
                    .optional(
                        "potion",
                        match potion {
                            Some(p) => Some(entry(names, "minecraft:potion", *p)?),
                            None => None,
                        },
                    )
                    .optional("custom_color", color.map(HashOps::create_int));
                // An empty tail is no custom effects and no name.
                if rest.is_empty() {
                    rec
                } else {
                    raw::potion_tail(rec, rest, names)?
                }
                .build()
            }
            Self::CustomModelData {
                floats,
                flags,
                strings,
                colors,
            } => {
                let list = |rec: RecordHash, name: &str, items: Vec<HashCode>| {
                    if items.is_empty() {
                        rec
                    } else {
                        rec.field(name, HashOps::create_list(items))
                    }
                };
                let rec = list(
                    RecordHash::new(),
                    "floats",
                    floats.iter().map(|f| HashOps::create_f32(*f)).collect(),
                );
                let rec = list(
                    rec,
                    "flags",
                    flags.iter().map(|f| HashOps::create_boolean(*f)).collect(),
                );
                let rec = list(rec, "strings", strings.iter().map(|s| string(s)).collect());
                list(
                    rec,
                    "colors",
                    colors.iter().map(|c| HashOps::create_int(*c)).collect(),
                )
                .build()
            }
            Self::TooltipDisplay { hide, hidden } => {
                let rec = or_default(
                    RecordHash::new(),
                    "hide_tooltip",
                    *hide,
                    false,
                    HashOps::create_boolean(*hide),
                );
                let mut list = Vec::with_capacity(hidden.len());
                for h in hidden {
                    list.push(entry(names, "minecraft:data_component_type", *h)?);
                }
                if list.is_empty() {
                    rec
                } else {
                    rec.field("hidden_components", HashOps::create_list(list))
                }
                .build()
            }
            Self::WritableBook { pages, filtered } => if pages.is_empty() {
                RecordHash::new()
            } else {
                RecordHash::new().field(
                    "pages",
                    HashOps::create_list(pages.iter().enumerate().map(|(i, p)| {
                        filterable(
                            string(p),
                            filtered.get(i).and_then(Option::as_ref).map(|f| string(f)),
                        )
                    })),
                )
            }
            .build(),
            Self::WrittenBook(b) => {
                let rec = RecordHash::new()
                    .field(
                        "title",
                        filterable(string(&b.title), b.title_filtered.as_deref().map(string)),
                    )
                    .field("author", string(&b.author));
                let rec = or_default(
                    rec,
                    "generation",
                    b.generation,
                    0,
                    HashOps::create_int(b.generation),
                );
                let rec = if b.pages.is_empty() {
                    rec
                } else {
                    rec.field(
                        "pages",
                        HashOps::create_list(b.pages.iter().enumerate().map(|(i, p)| {
                            filterable(
                                text(p),
                                b.pages_filtered.get(i).and_then(Option::as_ref).map(text),
                            )
                        })),
                    )
                };
                or_default(
                    rec,
                    "resolved",
                    b.resolved,
                    false,
                    HashOps::create_boolean(b.resolved),
                )
                .build()
            }
            Self::Stacks(stacks) => {
                let mut list = Vec::with_capacity(stacks.len());
                for s in stacks {
                    list.push(template(s, names)?);
                }
                HashOps::create_list(list)
            }
            Self::Fireworks { flight, explosions } => {
                // `ExtraCodecs.UNSIGNED_BYTE` is `Codec.BYTE`: a byte tag.
                let rec = or_default(
                    RecordHash::new(),
                    "flight_duration",
                    *flight,
                    0,
                    HashOps::create_byte(*flight as i8),
                );
                let mut list = Vec::with_capacity(explosions.len());
                for e in explosions {
                    list.push(explosion(e)?);
                }
                if list.is_empty() {
                    rec
                } else {
                    rec.field("explosions", HashOps::create_list(list))
                }
                .build()
            }
            Self::FireworkExplosion(e) => explosion(e)?,
            Self::LodestoneTracker { target, tracked } => {
                let target = target.as_ref().map(|(dimension, pos)| {
                    RecordHash::new()
                        .field("dimension", string(dimension))
                        .field("pos", HashOps::create_int_list(pos))
                        .build()
                });
                or_default(
                    RecordHash::new().optional("target", target),
                    "tracked",
                    *tracked,
                    true,
                    HashOps::create_boolean(*tracked),
                )
                .build()
            }
            Self::Trim { material, pattern } => RecordHash::new()
                .field(
                    "material",
                    entry(names, "minecraft:trim_material", *material)?,
                )
                .field("pattern", entry(names, "minecraft:trim_pattern", *pattern)?)
                .build(),
            Self::StewEffects(list) => {
                let mut out = Vec::with_capacity(list.len());
                for (effect, duration) in list {
                    let rec = RecordHash::new()
                        .field("id", entry(names, "minecraft:mob_effect", *effect)?);
                    out.push(
                        or_default(
                            rec,
                            "duration",
                            *duration,
                            160,
                            HashOps::create_int(*duration),
                        )
                        .build(),
                    );
                }
                HashOps::create_list(out)
            }
            Self::Instrument(i) => entry(names, "minecraft:instrument", *i)?,
            Self::BannerPatterns(layers) => {
                let mut out = Vec::with_capacity(layers.len());
                for (pattern, color) in layers {
                    out.push(
                        RecordHash::new()
                            .field(
                                "pattern",
                                entry(names, "minecraft:banner_pattern", *pattern)?,
                            )
                            .field("color", pick(&DYE, *color)?)
                            .build(),
                    );
                }
                HashOps::create_list(out)
            }
            Self::PaintingVariant(i) => entry(names, "minecraft:painting_variant", *i)?,
            Self::Profile(p) => raw::profile(p),
            Self::Raw(bytes) => raw::code(id, bytes, names)?,
        })
    }

    /// [`Component::code`] as `HashCode.asInt`, the value the packet carries.
    pub fn hash(&self, id: i32, names: &dyn Names) -> Option<i32> {
        self.code(id, names).map(HashCode::as_int)
    }
}

impl Hashed {
    /// `HashedStack.create` for a non-empty stack: `HashedPatchMap.create` hashes
    /// each added component (those Horizon cannot hash are left out) and sends the
    /// removed ones' ids.
    pub fn create(stack: &Stack, names: &dyn Names) -> Self {
        // A stack kept as the server sent it goes back as its own patch bytes
        // ([`Stack::wire`]): that patch is what it holds.
        let decoded = stack
            .wire
            .0
            .as_deref()
            .and_then(|patch| WirePatch::decode(patch).ok());
        let (added, removed) = match &decoded {
            Some((added, removed)) => (added, removed),
            None => (&stack.added, &stack.removed),
        };
        Self {
            item: stack.item,
            count: stack.count,
            added: added
                .iter()
                .filter_map(|(id, c)| Some((*id, c.hash(*id, names)?)))
                .collect(),
            removed: removed.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{Component as C, WrittenBook, component as k};
    use horizon_world::nbt::Tag;

    /// Ids for the tests, by registry (the position in the table is the id).
    struct Table;
    impl Names for Table {
        fn name(&self, registry: &str, id: i32) -> Option<String> {
            let names: &[&str] = match registry {
                "minecraft:enchantment" => &["minecraft:sharpness", "minecraft:protection"],
                "minecraft:potion" => &["minecraft:water", "minecraft:swiftness"],
                "minecraft:mob_effect" => &["minecraft:speed", "minecraft:night_vision"],
                "minecraft:trim_material" => &["minecraft:gold"],
                "minecraft:trim_pattern" => &["minecraft:sentry"],
                "minecraft:instrument" => &["minecraft:ponder_goat_horn"],
                "minecraft:banner_pattern" => &["minecraft:creeper"],
                "minecraft:painting_variant" => &["minecraft:kebab"],
                "minecraft:item" => &["minecraft:arrow", "minecraft:firework_rocket"],
                "minecraft:data_component_type" => {
                    return match id {
                        k::DAMAGE => Some("minecraft:damage"),
                        k::ENCHANTMENTS => Some("minecraft:enchantments"),
                        k::RARITY => Some("minecraft:rarity"),
                        _ => None,
                    }
                    .map(str::to_owned);
                }
                _ => return None,
            };
            names.get(usize::try_from(id).ok()?).map(|n| n.to_string())
        }
    }

    fn text(s: &str) -> Tag {
        Tag::String(s.into())
    }
    fn explosion(shape: i32, colors: &[i32], fade: &[i32], trail: bool) -> FireworkExplosion {
        FireworkExplosion {
            shape,
            colors: colors.to_vec(),
            fade: fade.to_vec(),
            trail,
            twinkle: false,
        }
    }

    /// Hashes printed by vanilla 26.3 (`TypedDataComponent.encodeValue` through a
    /// `RegistryOps<HashCode>` over `HashOps.CRC32C_INSTANCE`, then `asInt`) for
    /// the same component values.
    #[test]
    fn component_hashes_match_vanilla() {
        let nbt = Tag::compound([
            ("n", Tag::Int(7)),
            ("flag", Tag::Byte(1)),
            ("s", text("hi")),
            ("l", Tag::List(vec![Tag::Int(1)])),
        ]);
        let styled = Tag::compound([
            ("text", text("Hi")),
            ("bold", Tag::Byte(1)),
            ("color", text("red")),
        ]);
        let extra = Tag::compound([
            ("text", text("A")),
            (
                "extra",
                Tag::List(vec![Tag::compound([
                    ("translate", text("block.minecraft.stone")),
                    ("italic", Tag::Byte(0)),
                ])]),
            ),
        ]);
        let two = Tag::compound([("text", text("two")), ("underlined", Tag::Byte(1))]);
        let fe = explosion(2, &[0xFF0000, 0x00FF00], &[1], true);
        let fireworks_template = {
            let mut s = Stack::new(1, 3);
            s.added.push((k::DAMAGE, C::Int(2)));
            s.removed.push(k::RARITY);
            s
        };
        let cases: Vec<(&str, i32, C, i32)> = vec![
            ("damage", k::DAMAGE, C::Int(5), 645064431),
            ("max_stack_size", k::MAX_STACK_SIZE, C::Int(16), 1769065625),
            ("repair_cost", k::REPAIR_COST, C::Int(3), -499649379),
            ("rarity", k::RARITY, C::Int(3), -292715907),
            ("unbreakable", k::UNBREAKABLE, C::Unit, -982207288),
            (
                "glint",
                k::ENCHANTMENT_GLINT_OVERRIDE,
                C::Bool(true),
                -1019818302,
            ),
            (
                "item_model",
                k::ITEM_MODEL,
                C::Ident("minecraft:stick".into()),
                -1153337500,
            ),
            ("custom_data", k::CUSTOM_DATA, C::Nbt(nbt), 588551639),
            (
                "enchantments",
                k::ENCHANTMENTS,
                C::Enchantments(vec![(0, 3), (1, 4)]),
                156916527,
            ),
            (
                "custom_name",
                k::CUSTOM_NAME,
                C::Text(text("Hi")),
                485089409,
            ),
            (
                "custom_name_styled",
                k::CUSTOM_NAME,
                C::Text(styled),
                -204970011,
            ),
            ("item_name_extra", k::ITEM_NAME, C::Text(extra), 868407222),
            ("lore", k::LORE, C::Lore(vec![text("one"), two]), 931033850),
            ("lore_empty", k::LORE, C::Lore(vec![]), -1978007022),
            ("dyed", k::DYED_COLOR, C::Int(0xA06540), -1304070639),
            ("base_color", k::BASE_COLOR, C::Int(14), -1939582294),
            ("map_id", k::MAP_ID, C::Int(12), 225652657),
            (
                "ominous",
                k::OMINOUS_BOTTLE_AMPLIFIER,
                C::Int(2),
                1064459813,
            ),
            (
                "cmd",
                k::CUSTOM_MODEL_DATA,
                C::CustomModelData {
                    floats: vec![1.5],
                    flags: vec![true, false],
                    strings: vec!["a".into()],
                    colors: vec![0xFF0000],
                },
                -2044548697,
            ),
            (
                "cmd_flags",
                k::CUSTOM_MODEL_DATA,
                C::CustomModelData {
                    floats: vec![],
                    flags: vec![true],
                    strings: vec![],
                    colors: vec![],
                },
                1169437062,
            ),
            (
                "tooltip",
                k::TOOLTIP_DISPLAY,
                C::TooltipDisplay {
                    hide: true,
                    hidden: vec![k::DAMAGE, k::ENCHANTMENTS],
                },
                1281445049,
            ),
            (
                "tooltip_default",
                k::TOOLTIP_DISPLAY,
                C::TooltipDisplay {
                    hide: false,
                    hidden: vec![],
                },
                -982207288,
            ),
            (
                "attack_range",
                k::ATTACK_RANGE,
                C::AttackRange([0.0, 4.0, 0.0, 6.0, 0.3, 1.0]),
                -287603881,
            ),
            (
                "attack_range_default",
                k::ATTACK_RANGE,
                C::AttackRange([0.0, 3.0, 0.0, 5.0, 0.3, 1.0]),
                -982207288,
            ),
            (
                "firework_explosion",
                k::FIREWORK_EXPLOSION,
                C::FireworkExplosion(fe.clone()),
                282963373,
            ),
            (
                "firework_explosion_plain",
                k::FIREWORK_EXPLOSION,
                C::FireworkExplosion(explosion(4, &[], &[], false)),
                -1702512602,
            ),
            (
                "fireworks",
                k::FIREWORKS,
                C::Fireworks {
                    flight: 2,
                    explosions: vec![fe],
                },
                -1263695327,
            ),
            (
                "fireworks_empty",
                k::FIREWORKS,
                C::Fireworks {
                    flight: 0,
                    explosions: vec![],
                },
                -982207288,
            ),
            (
                "potion",
                k::POTION_CONTENTS,
                C::Potion {
                    potion: Some(1),
                    color: Some(0x123456),
                    rest: vec![],
                },
                360529701,
            ),
            (
                "potion_water",
                k::POTION_CONTENTS,
                C::Potion {
                    potion: Some(0),
                    color: None,
                    rest: vec![],
                },
                1311226275,
            ),
            (
                "stew",
                k::SUSPICIOUS_STEW_EFFECTS,
                C::StewEffects(vec![(0, 160), (1, 100)]),
                -1574156649,
            ),
            (
                "lodestone",
                k::LODESTONE_TRACKER,
                C::LodestoneTracker {
                    target: Some(("minecraft:overworld".into(), [1, 64, -3])),
                    tracked: false,
                },
                -1499562110,
            ),
            (
                "lodestone_none",
                k::LODESTONE_TRACKER,
                C::LodestoneTracker {
                    target: None,
                    tracked: true,
                },
                -982207288,
            ),
            (
                "trim",
                k::TRIM,
                C::Trim {
                    material: 0,
                    pattern: 0,
                },
                -1936516638,
            ),
            ("instrument", k::INSTRUMENT, C::Instrument(0), -1123313158),
            (
                "banner",
                k::BANNER_PATTERNS,
                C::BannerPatterns(vec![(0, 5)]),
                1048823812,
            ),
            (
                "painting",
                k::PAINTING_VARIANT,
                C::PaintingVariant(0),
                -670067346,
            ),
            (
                "writable",
                k::WRITABLE_BOOK_CONTENT,
                C::WritableBook {
                    pages: vec!["page one".into(), "two".into()],
                    filtered: vec![],
                },
                1900254632,
            ),
            (
                "written",
                k::WRITTEN_BOOK_CONTENT,
                C::WrittenBook(WrittenBook {
                    title: "Title".into(),
                    author: "Author".into(),
                    generation: 1,
                    pages: vec![text("p1")],
                    resolved: true,
                    ..WrittenBook::default()
                }),
                -112185549,
            ),
            (
                "written_min",
                k::WRITTEN_BOOK_CONTENT,
                C::WrittenBook(WrittenBook {
                    title: "T".into(),
                    author: "A".into(),
                    generation: 0,
                    pages: vec![],
                    resolved: false,
                    ..WrittenBook::default()
                }),
                -809804424,
            ),
            (
                "charged",
                k::CHARGED_PROJECTILES,
                C::Stacks(vec![Stack::new(0, 1), fireworks_template.clone()]),
                -1839603258,
            ),
            (
                "bundle",
                k::BUNDLE_CONTENTS,
                C::Stacks(vec![fireworks_template]),
                785458737,
            ),
        ];
        for (name, id, component, expected) in cases {
            assert_eq!(component.hash(id, &Table), Some(expected), "{name}");
        }
    }

    #[test]
    fn components_without_a_codec_form_are_not_hashed() {
        // Unresolvable registry ids, bytes Horizon only reads past, and the
        // components vanilla has no persistent codec for.
        assert_eq!(
            C::Enchantments(vec![(9, 1)]).hash(k::ENCHANTMENTS, &Table),
            None
        );
        // additional_trade_cost has no persistent codec either.
        assert_eq!(C::Raw(vec![1]).hash(42, &Table), None);
        assert_eq!(C::Unit.hash(k::CREATIVE_SLOT_LOCK, &Table), None);
        assert_eq!(C::Int(0).hash(k::MAP_POST_PROCESSING, &Table), None);
        // Custom effects cut short.
        let custom_effects = C::Potion {
            potion: None,
            color: None,
            rest: vec![1, 0, 0],
        };
        assert_eq!(custom_effects.hash(k::POTION_CONTENTS, &Table), None);
    }

    #[test]
    fn a_hashed_stack_carries_each_hashable_component_and_the_removed_ones() {
        let mut stack = Stack::new(1, 2);
        stack.added.push((k::DAMAGE, C::Int(5)));
        stack.added.push((42, C::Raw(vec![0])));
        stack.removed.push(k::RARITY);
        assert_eq!(
            Hashed::create(&stack, &Table),
            Hashed {
                item: 1,
                count: 2,
                added: vec![(k::DAMAGE, 645064431)],
                removed: vec![k::RARITY]
            }
        );
        // Without names the plain ones still hash.
        assert_eq!(Hashed::of(&stack).added, vec![(k::DAMAGE, 645064431)]);
        // A stack kept as the server sent it hashes the patch it holds.
        use crate::buf::Codec as _;
        let mut w = crate::buf::Writer::new();
        crate::item::DelimitedSlot::write(&Some(stack.clone()), &mut w);
        let kept = crate::item::DelimitedSlot::read(&mut crate::buf::Reader::new(&w.data))
            .unwrap()
            .unwrap();
        let mut bare = Stack::new(1, 2);
        bare.wire = kept.wire;
        assert_eq!(
            Hashed::create(&bare, &Table),
            Hashed::create(&stack, &Table)
        );
    }
}
