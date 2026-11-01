//! The components Horizon keeps as their stream bytes ([`Component::Raw`]),
//! decoded into the `Codec` form that vanilla's `HashOps` sees. Each function
//! reads exactly what the component's `STREAM_CODEC` writes (the readers in
//! `item/skip.rs` step over the same bytes) and builds what its `CODEC`
//! encodes: a component without a `networkSynchronized` stream codec goes as
//! its codec's NBT, so that NBT converts to hashes as it is (`NbtOps.convertTo`).
use super::*;
use crate::buf::{Codec, Reader};
use crate::item::{Profile, Template};
use horizon_world::nbt::Tag;

/// `DyeColor` names by id.
fn dye(id: i32) -> Option<HashCode> {
    pick(&DYE, id)
}
/// The `StringRepresentable` names of an enum whose stream codec is an `idMapper`
/// of the ids `0..`.
fn named(table: &[&str], r: &mut Reader) -> Option<HashCode> {
    pick(table, r.varint().ok()?)
}
/// `ByteBufCodecs.holderRegistry`: the registry id itself; the codec writes the
/// entry's identifier.
fn registry_entry(registry: &str, names: &dyn Names, r: &mut Reader) -> Option<HashCode> {
    entry(names, registry, r.varint().ok()?)
}
fn float(r: &mut Reader) -> Option<HashCode> {
    Some(HashOps::create_f32(r.f32().ok()?))
}
fn int(r: &mut Reader) -> Option<HashCode> {
    Some(HashOps::create_int(r.varint().ok()?))
}
/// A one-field record `{name: VAR_INT}`.
fn int_record(name: &str, r: &mut Reader) -> Option<HashCode> {
    Some(RecordHash::new().field(name, int(r)?).build())
}

const FOX: [&str; 2] = ["red", "snow"];
const SALMON: [&str; 3] = ["small", "medium", "large"];
const PARROT: [&str; 5] = ["red_blue", "blue", "green", "yellow_blue", "gray"];
const MOOSHROOM: [&str; 2] = ["red", "brown"];
const HORSE: [&str; 7] = [
    "white",
    "creamy",
    "chestnut",
    "brown",
    "black",
    "gray",
    "dark_brown",
];
const LLAMA: [&str; 4] = ["creamy", "white", "brown", "gray"];
const AXOLOTL: [&str; 5] = ["lucy", "wild", "gold", "cyan", "blue"];
const RABBIT: [&str; 6] = ["brown", "white", "black", "white_splotched", "gold", "salt"];
/// `TropicalFish.Pattern` by packed id (`base | index << 8`).
const TROPICAL: [(i32, &str); 12] = [
    (0, "kob"),
    (256, "sunstreak"),
    (512, "snooper"),
    (768, "dasher"),
    (1024, "brinely"),
    (1280, "spotty"),
    (1, "flopper"),
    (257, "stripey"),
    (513, "glitter"),
    (769, "blockfish"),
    (1025, "betty"),
    (1281, "clayfish"),
];

/// `TypedEntityData.codec`: the NBT with the type's identifier as `id`.
fn typed_data(registry: &str, names: &dyn Names, r: &mut Reader) -> Option<HashCode> {
    let id = registry_entry(registry, names, r)?;
    let Tag::Compound(mut tag) = r.nbt().ok()? else {
        return None;
    };
    // `stripId` leaves the data without its own id; the codec writes the type's.
    tag.remove(&"id".into());
    Some(typed_with_id(id, &tag))
}
fn typed_with_id(id: HashCode, tag: &horizon_world::nbt::Compound) -> HashCode {
    let mut entries: Vec<(HashCode, HashCode)> = tag
        .iter()
        .map(|(k, v)| {
            (
                HashOps::create_string_units(&k.0),
                HashOps::from_nbt(v, &[]),
            )
        })
        .collect();
    entries.push((string("id"), id));
    HashOps::create_map(entries)
}
/// A list written as a count and its elements, hashed as a list.
fn list(
    r: &mut Reader,
    mut element: impl FnMut(&mut Reader) -> Option<HashCode>,
) -> Option<HashCode> {
    let n = r.count(r.remaining()).ok()?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(element(r)?);
    }
    Some(HashOps::create_list(out))
}
fn optional(
    r: &mut Reader,
    element: impl FnOnce(&mut Reader) -> Option<HashCode>,
) -> Option<Option<HashCode>> {
    if r.bool().ok()? {
        Some(Some(element(r)?))
    } else {
        Some(None)
    }
}
pub(super) fn template_code(r: &mut Reader, names: &dyn Names) -> Option<HashCode> {
    let stack = <Template as Codec<Stack>>::read(r).ok()?;
    template(&stack, names)
}
/// `ByteBufCodecs.fixedSizeList(4)` of text components.
fn four_texts(r: &mut Reader) -> Option<(Vec<Tag>, HashCode)> {
    let tags: Vec<Tag> = (0..4).map(|_| r.nbt().ok()).collect::<Option<_>>()?;
    let code = HashOps::create_list(tags.iter().map(text));
    Some((tags, code))
}

/// `MobEffectInstance.Details.MAP_CODEC` (the fields of a record, flattened into
/// `rec`) from `STREAM_CODEC`: amplifier, duration, ambient, show_particles,
/// show_icon, then the hidden effect's details.
fn effect_details(rec: RecordHash, r: &mut Reader) -> Option<RecordHash> {
    let amplifier = r.varint().ok()?;
    let duration = r.varint().ok()?;
    let ambient = r.bool().ok()?;
    let particles = r.bool().ok()?;
    let icon = r.bool().ok()?;
    // `ExtraCodecs.UNSIGNED_BYTE` is a byte tag.
    let rec = or_default(
        rec,
        "amplifier",
        amplifier,
        0,
        HashOps::create_byte(amplifier as i8),
    );
    let rec = or_default(rec, "duration", duration, 0, HashOps::create_int(duration));
    let rec = or_default(
        rec,
        "ambient",
        ambient,
        false,
        HashOps::create_boolean(ambient),
    );
    let rec = or_default(
        rec,
        "show_particles",
        particles,
        true,
        HashOps::create_boolean(particles),
    );
    // `show_icon` is always written.
    let rec = rec.field("show_icon", HashOps::create_boolean(icon));
    Some(if r.bool().ok()? {
        let hidden = effect_details(RecordHash::new(), r)?.build();
        rec.field("hidden_effect", hidden)
    } else {
        rec
    })
}
/// `MobEffectInstance.CODEC`: the effect's identifier and its details.
pub(super) fn mob_effect(r: &mut Reader, names: &dyn Names) -> Option<HashCode> {
    let rec = RecordHash::new().field("id", registry_entry("minecraft:mob_effect", names, r)?);
    Some(effect_details(rec, r)?.build())
}

/// `PotionContents` after its potion and colour: the custom effects and name
/// (`custom_effects` is absent when empty).
pub(super) fn potion_tail(rec: RecordHash, rest: &[u8], names: &dyn Names) -> Option<RecordHash> {
    let mut r = Reader::new(rest);
    let n = r.count(r.remaining()).ok()?;
    let mut effects = Vec::with_capacity(n);
    for _ in 0..n {
        effects.push(mob_effect(&mut r, names)?);
    }
    let name = if r.bool().ok()? {
        Some(string(&r.string(32767).ok()?))
    } else {
        None
    };
    (r.remaining() == 0).then_some(())?;
    let rec = if effects.is_empty() {
        rec
    } else {
        rec.field("custom_effects", HashOps::create_list(effects))
    };
    Some(rec.optional("custom_name", name))
}

/// `ResolvableProfile.CODEC` (`FULL_CODEC`): the game profile's or the partial's
/// fields, then the skin patch's. Both forms write `id`, `name` and a
/// `properties` list that is absent when empty.
pub(super) fn profile(p: &Profile) -> HashCode {
    let rec = RecordHash::new()
        .optional("name", p.name.as_deref().map(string))
        .optional(
            "id",
            // `UUIDUtil.CODEC`: an int array.
            p.id.map(|id| {
                HashOps::create_int_list(&[
                    (id >> 96) as i32,
                    (id >> 64) as i32,
                    (id >> 32) as i32,
                    id as i32,
                ])
            }),
        );
    let rec = if p.properties.is_empty() {
        rec
    } else {
        rec.field(
            "properties",
            HashOps::create_list(p.properties.iter().map(|(name, value, signature)| {
                RecordHash::new()
                    .field("name", string(name))
                    .field("value", string(value))
                    .optional("signature", signature.as_deref().map(string))
                    .build()
            })),
        )
    };
    rec.optional("texture", p.skin.body.as_deref().map(string))
        .optional("cape", p.skin.cape.as_deref().map(string))
        .optional("elytra", p.skin.elytra.as_deref().map(string))
        .optional(
            "model",
            p.skin
                .slim
                .map(|slim| string(if slim { "slim" } else { "wide" })),
        )
        .build()
}

/// `ByteBufCodecs.holder(key, direct)` as `RegistryCodecs.holder(key, directCodec)`
/// writes it: a reference by identifier, a direct value as its record.
fn holder_or(
    registry: &str,
    names: &dyn Names,
    r: &mut Reader,
    direct: impl FnOnce(&mut Reader) -> Option<HashCode>,
) -> Option<HashCode> {
    match r.varint().ok()? {
        0 => direct(r),
        id => entry(names, registry, id - 1),
    }
}
/// `TrimMaterial.DIRECT_CODEC`.
pub(super) fn trim_material(r: &mut Reader) -> Option<HashCode> {
    let palette = string(&r.string(32767).ok()?);
    let description = text(&r.nbt().ok()?);
    Some(
        RecordHash::new()
            .field("palette_id", palette)
            .field("description", description)
            .build(),
    )
}
/// `TrimPattern.DIRECT_CODEC` (`decal` is always written).
fn trim_pattern(r: &mut Reader) -> Option<HashCode> {
    let asset = string(&r.string(32767).ok()?);
    let description = text(&r.nbt().ok()?);
    let decal = r.bool().ok()?;
    Some(
        RecordHash::new()
            .field("asset_id", asset)
            .field("description", description)
            .field("decal", HashOps::create_boolean(decal))
            .build(),
    )
}
/// `Instrument.DIRECT_CODEC`.
fn instrument(r: &mut Reader, names: &dyn Names) -> Option<HashCode> {
    let sound = super::gameplay::sound(r, names)?.code;
    let (duration, range) = (r.f32().ok()?, r.f32().ok()?);
    let damage = r.varint().ok()?;
    let description = text(&r.nbt().ok()?);
    let rec = RecordHash::new()
        .field("sound_event", sound)
        .field("use_duration", HashOps::create_f32(duration))
        .field("range", HashOps::create_f32(range));
    Some(
        or_default(
            rec,
            "durability_damage",
            damage,
            0,
            HashOps::create_int(damage),
        )
        .field("description", description)
        .build(),
    )
}
/// `BannerPattern.DIRECT_CODEC`.
fn banner_pattern(r: &mut Reader) -> Option<HashCode> {
    let asset = string(&r.string(32767).ok()?);
    let key = string(&r.string(32767).ok()?);
    Some(
        RecordHash::new()
            .field("asset_id", asset)
            .field("translation_key", key)
            .build(),
    )
}

/// The hash of component `kind` from its stream bytes, or `None` when Horizon
/// has no codec form for it (no persistent codec in vanilla, a direct holder, an
/// id with no name, a component still to be ported).
pub(super) fn code(kind: i32, bytes: &[u8], names: &dyn Names) -> Option<HashCode> {
    let mut r = Reader::new(bytes);
    let r = &mut r;
    let code = match kind {
        // `Unit.CODEC`: an empty map.
        34 | 120 => HashOps::empty_map(),
        // Persistent codecs only: the stream form is the codec's NBT.
        // intangible_projectile, map_decorations, debug_stick_state, recipes,
        // lock (an `ItemPredicate`) and container_loot.
        22 | 49 | 59 | 68 | 81 | 82 => HashOps::from_nbt(&r.nbt().ok()?, &[]),
        // `Codec.FLOAT`: minimum_attack_charge, potion_duration_scale.
        7 | 54 => float(r)?,
        // `Identifier.CODEC`: tooltip_style, note_block_sound.
        35 | 73 => string(&r.string(32767).ok()?),
        // `Enchantable`, `VillagerFood`.
        31 => int_record("value", r)?,
        44 => int_record("nutrition", r)?,
        // `DyeColor.CODEC`: dye, wolf/collar, tropical_fish/base_color and
        // pattern_color, cat/collar, sheep/color, shulker/color, cushion/color.
        46 | 91 | 96 | 97 | 114 | 115 | 116 | 121 => dye(r.varint().ok()?)?,
        8 => registry_entry("minecraft:damage_type", names, r)?,
        43 => registry_entry("minecraft:block_transformer", names, r)?,
        88 => registry_entry("minecraft:villager_type", names, r)?,
        89 => registry_entry("minecraft:wolf_variant", names, r)?,
        90 => registry_entry("minecraft:wolf_sound_variant", names, r)?,
        100 => registry_entry("minecraft:pig_variant", names, r)?,
        101 => registry_entry("minecraft:pig_sound_variant", names, r)?,
        102 => registry_entry("minecraft:cow_variant", names, r)?,
        103 => registry_entry("minecraft:cow_sound_variant", names, r)?,
        104 => registry_entry("minecraft:chicken_variant", names, r)?,
        105 => registry_entry("minecraft:chicken_sound_variant", names, r)?,
        106 => registry_entry("minecraft:zombie_nautilus_variant", names, r)?,
        107 => registry_entry("minecraft:frog_variant", names, r)?,
        112 => registry_entry("minecraft:cat_variant", names, r)?,
        113 => registry_entry("minecraft:cat_sound_variant", names, r)?,
        117 => registry_entry("minecraft:decorated_pot_pattern", names, r)?,
        // The mob variants that are enums.
        92 => named(&FOX, r)?,
        93 => named(&SALMON, r)?,
        94 => named(&PARROT, r)?,
        95 => {
            let id = r.varint().ok()?;
            string(TROPICAL.iter().find(|(packed, _)| *packed == id)?.1)
        }
        98 => named(&MOOSHROOM, r)?,
        99 => match r.varint().ok()? {
            99 => string("evil"),
            id => pick(&RABBIT, id)?,
        },
        108 => named(&HORSE, r)?,
        110 => named(&LLAMA, r)?,
        111 => named(&AXOLOTL, r)?,
        // `TypedEntityData` (entity_data, block_entity_data) and `CustomData`.
        60 => typed_data("minecraft:entity_type", names, r)?,
        62 => typed_data("minecraft:block_entity_type", names, r)?,
        61 => HashOps::from_nbt(&r.nbt().ok()?, &[]),
        // `BlockItemStateProperties`: a string map.
        78 => {
            let n = r.count(r.remaining()).ok()?;
            let mut entries = Vec::with_capacity(n);
            for _ in 0..n {
                let k = string(&r.string(32767).ok()?);
                entries.push((k, string(&r.string(32767).ok()?)));
            }
            HashOps::create_map(entries)
        }
        // `ArmorTrim`, `InstrumentComponent`, `provides_trim_material` and
        // `BannerPatternLayers` holders (a reference stays typed in Horizon but
        // a direct one comes here).
        58 => {
            let material = holder_or("minecraft:trim_material", names, r, trim_material)?;
            let pattern = holder_or("minecraft:trim_pattern", names, r, trim_pattern)?;
            RecordHash::new()
                .field("material", material)
                .field("pattern", pattern)
                .build()
        }
        63 => holder_or("minecraft:instrument", names, r, |r| instrument(r, names))?,
        64 => holder_or("minecraft:trim_material", names, r, trim_material)?,
        74 => list(r, |r| {
            let pattern = holder_or("minecraft:banner_pattern", names, r, banner_pattern)?;
            let color = dye(r.varint().ok()?)?;
            Some(
                RecordHash::new()
                    .field("pattern", pattern)
                    .field("color", color)
                    .build(),
            )
        })?,
        // `SulfurCubeContent`: one template.
        80 => template_code(r, names)?,
        // `PotDecorations`: back, left, right, front.
        76 => {
            let mut rec = RecordHash::new();
            for side in ["back", "left", "right", "front"] {
                rec = rec.optional(side, optional(r, |r| template_code(r, names))?);
            }
            rec.build()
        }
        // `ItemContainerContents`: the occupied slots as `{slot, item}`.
        77 => {
            let n = r.count(256).ok()?;
            let mut slots = Vec::new();
            for slot in 0..n {
                if let Some(item) = optional(r, |r| template_code(r, names))? {
                    slots.push(
                        RecordHash::new()
                            .field("slot", HashOps::create_int(slot as i32))
                            .field("item", item)
                            .build(),
                    );
                }
            }
            HashOps::create_list(slots)
        }
        // `Bees`: occupants.
        79 => list(r, |r| {
            let data = typed_data("minecraft:entity_type", names, r)?;
            let ticks = r.varint().ok()?;
            let min = r.varint().ok()?;
            Some(
                RecordHash::new()
                    .field("entity_data", data)
                    .field("ticks_in_hive", HashOps::create_int(ticks))
                    .field("min_ticks_in_hive", HashOps::create_int(min))
                    .build(),
            )
        })?,
        // `SignText`: messages, the filtered ones when they differ, colour, glow.
        118 | 119 => {
            let (messages, code) = four_texts(r)?;
            // `filteredMessagesForSerialization`: absent when equal to the messages.
            let filtered = if r.bool().ok()? {
                let (tags, code) = four_texts(r)?;
                (tags != messages).then_some(code)
            } else {
                None
            };
            let color = dye(r.varint().ok()?)?;
            let glow = r.bool().ok()?;
            RecordHash::new()
                .field("messages", code)
                .optional("filtered_messages", filtered)
                .field("color", color)
                .field("has_glowing_text", HashOps::create_boolean(glow))
                .build()
        }
        _ => super::gameplay::code(kind, r, names)?,
    };
    (r.remaining() == 0).then_some(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Component;

    /// The ids vanilla 26.3 gave the names the cases use.
    struct Pinned;
    impl Names for Pinned {
        fn name(&self, registry: &str, id: i32) -> Option<String> {
            let name = match (registry, id) {
                ("minecraft:item", 1) => "minecraft:stone",
                ("minecraft:item", 1009) => "minecraft:arrow",
                ("minecraft:item", 1060) => "minecraft:stick",
                ("minecraft:item", 1142) => "minecraft:brick",
                ("minecraft:mob_effect", 0) => "minecraft:speed",
                ("minecraft:mob_effect", 1) => "minecraft:slowness",
                ("minecraft:potion", 13) => "minecraft:swiftness",
                ("minecraft:entity_type", 11) => "minecraft:bee",
                ("minecraft:entity_type", 30) => "minecraft:cow",
                ("minecraft:block_entity_type", 1) => "minecraft:chest",
                ("minecraft:data_component_type", 1) => "minecraft:max_stack_size",
                ("minecraft:data_component_type", 2) => "minecraft:max_damage",
                ("minecraft:data_component_type", 3) => "minecraft:damage",
                ("minecraft:data_component_type", 12) => "minecraft:rarity",
                ("minecraft:villager_type", 0) => "minecraft:desert",
                ("minecraft:data_component_predicate_type", 0) => "minecraft:damage",
                ("minecraft:data_component_predicate_type", 1) => "minecraft:enchantments",
                ("minecraft:data_component_predicate_type", 2) => "minecraft:stored_enchantments",
                ("minecraft:data_component_predicate_type", 3) => "minecraft:potion_contents",
                ("minecraft:data_component_predicate_type", 4) => "minecraft:custom_data",
                ("minecraft:data_component_predicate_type", 5) => "minecraft:container",
                ("minecraft:data_component_predicate_type", 6) => "minecraft:bundle_contents",
                ("minecraft:data_component_predicate_type", 7) => "minecraft:firework_explosion",
                ("minecraft:data_component_predicate_type", 8) => "minecraft:fireworks",
                ("minecraft:data_component_predicate_type", 9) => "minecraft:writable_book_content",
                ("minecraft:data_component_predicate_type", 10) => "minecraft:written_book_content",
                ("minecraft:data_component_predicate_type", 11) => "minecraft:attribute_modifiers",
                ("minecraft:data_component_predicate_type", 12) => "minecraft:trim",
                ("minecraft:data_component_predicate_type", 13) => "minecraft:jukebox_playable",
                ("minecraft:data_component_predicate_type", 14) => "minecraft:villager/variant",
                ("minecraft:attribute", 1) => "minecraft:armor",
                ("minecraft:attribute", 3) => "minecraft:attack_damage",
                ("minecraft:attribute", 23) => "minecraft:max_health",
                ("minecraft:sound_event", 50) => "minecraft:block.anvil.land",
                ("minecraft:sound_event", 70) => "minecraft:item.armor.equip_generic",
                ("minecraft:sound_event", 71) => "minecraft:item.armor.equip_gold",
                ("minecraft:sound_event", 702) => "minecraft:entity.generic.eat",
                ("minecraft:sound_event", 1314) => "minecraft:entity.player.burp",
                ("minecraft:sound_event", 1447) => "minecraft:item.shears.snip",
                ("minecraft:sound_event", 1459) => "minecraft:item.shield.block",
                ("minecraft:mob_effect", 18) => "minecraft:poison",
                ("minecraft:block", 1) => "minecraft:stone",
                ("minecraft:block", 9) => "minecraft:dirt",
                ("minecraft:item", 1006) => "minecraft:bowl",
                ("minecraft:item", 1018) => "minecraft:iron_ingot",
                ("minecraft:entity_type", 32) => "minecraft:creeper",
                ("minecraft:entity_type", 101) => "minecraft:pig",
                ("minecraft:entity_type", 154) => "minecraft:zombie",
                _ => return None,
            };
            Some(name.to_owned())
        }
    }

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap())
            .collect()
    }

    /// The stream bytes and hashes vanilla 26.3 printed for components built from
    /// their codecs (`STREAM_CODEC.encode`, then `TypedDataComponent.encodeValue`
    /// through `HashOps.CRC32C_INSTANCE`).
    #[test]
    fn components_read_past_hash_as_vanilla_does() {
        let cases: [(&str, i32, &str, i32); 48] = [
            (
                "potion_contents",
                53,
                "010d01000000ff020001c8010001000001000000010100010178",
                568571899,
            ),
            ("potion_contents", 53, "0000010000000100000000", -1716828889),
            ("potion_contents", 53, "00000001026e6d", -1256264264),
            (
                "profile",
                72,
                "010000000100000002000000030000000405537465766501087465787475726573036162630103736967010d6d696e6563726166743a666f6f010e6d696e6563726166743a63617065000101",
                1753620268,
            ),
            ("profile", 72, "0001054e6f746368000000000000", -1744970702),
            (
                "profile",
                72,
                "000001fffffffb000000060000000700000008000000000100",
                -196647545,
            ),
            (
                "profile",
                72,
                "010000000100000001000000010000000101610108746578747572657301760000000000",
                -1883525005,
            ),
            (
                "entity_data",
                60,
                "1e0a0100044e6f4149010300034167650000000500",
                1343601188,
            ),
            (
                "block_entity_data",
                62,
                "010a0800044c6f636b00017800",
                -1404731534,
            ),
            (
                "bucket_entity_data",
                61,
                "0a0500064865616c7468404000000100044e6f41490100",
                2142887662,
            ),
            (
                "container",
                77,
                "0601a4080300000000000001f1070101000304",
                92591498,
            ),
            ("container", 77, "00", -1978007022),
            (
                "pot_decorations",
                76,
                "01f60801000000000101020000",
                1078935147,
            ),
            (
                "block_state",
                78,
                "02036c6974047472756506666163696e67056e6f727468",
                1069123732,
            ),
            ("bees", 79, "010b0a030003416765000000030003d804", -122152224),
            ("sulfur_cube_content", 80, "01040000", 231192959),
            (
                "sign_text_front",
                118,
                "08000161080000080000080000000e01",
                -837409994,
            ),
            (
                "sign_text_back",
                119,
                "0800016108000008000008000001080001620800000800000800000f00",
                450855835,
            ),
            (
                "lock",
                81,
                "0a0800056974656d73000f6d696e6563726166743a737469636b00",
                791108361,
            ),
            (
                "container_loot",
                82,
                "0a04000473656564000000000000000508000a6c6f6f745f7461626c65001f6d696e6563726166743a6368657374732f73696d706c655f64756e67656f6e00",
                33155076,
            ),
            (
                "container_loot",
                82,
                "0a08000a6c6f6f745f7461626c65001f6d696e6563726166743a6368657374732f73696d706c655f64756e67656f6e00",
                -2040272502,
            ),
            (
                "map_decorations",
                49,
                "0a0a000161050008726f746174696f6e40400000060001783ff00000000000000600017a40000000000000000800047479706500106d696e6563726166743a706c617965720000",
                1757442925,
            ),
            (
                "debug_stick_state",
                59,
                "0a0800146d696e6563726166743a6f616b5f7374616972730006666163696e6700",
                -1281127558,
            ),
            (
                "recipes",
                68,
                "090800000001000f6d696e6563726166743a737469636b",
                -533729841,
            ),
            ("intangible_projectile", 22, "0a00", -982207288),
            ("minimum_attack_charge", 7, "3f000000", -1631070359),
            ("potion_duration_scale", 54, "40000000", -1320370368),
            (
                "tooltip_style",
                35,
                "0f6d696e6563726166743a66616e6379",
                -1403952026,
            ),
            (
                "note_block_sound",
                73,
                "106d696e6563726166743a637573746f6d",
                1404676026,
            ),
            ("enchantable", 31, "05", -1577171417),
            ("villager_food", 44, "03", 18785838),
            ("glider", 34, "", -982207288),
            ("waxed", 120, "", -982207288),
            ("dye", 46, "0e", -1939582294),
            ("wolf/collar", 91, "0b", 199633461),
            ("sheep/color", 115, "05", -1201809287),
            ("cushion/color", 121, "0f", 1821130656),
            ("villager/variant", 88, "00", 1867638223),
            ("fox/variant", 92, "01", -1511535024),
            ("salmon/size", 93, "02", -886537313),
            ("parrot/variant", 94, "04", -727074367),
            ("tropical_fish/pattern", 95, "8108", 312926869),
            ("mooshroom/variant", 98, "01", -541737949),
            ("rabbit/variant", 99, "63", 1619861300),
            ("rabbit/variant", 99, "05", 359467842),
            ("horse/variant", 108, "06", -337512376),
            ("llama/variant", 110, "03", -727074367),
            ("axolotl/variant", 111, "04", 199633461),
        ];
        for (name, kind, hex, expected) in cases {
            let bytes = unhex(hex);
            let mut r = Reader::new(&bytes);
            let c = Component::read(kind, &mut r).unwrap();
            assert_eq!(r.remaining(), 0, "{name}");
            assert_eq!(c.hash(kind, &Pinned), Some(expected), "{name} {hex}");
        }
    }

    /// The same for the components that tune an item's behaviour.
    #[test]
    fn gameplay_components_hash_as_vanilla_does() {
        let cases: [(&str, i32, &str, i32); 52] = [
            (
                "trim",
                58,
                "000b6d696e6563726166743a700800034d6174000b6d696e6563726166743a610a080005636f6c6f7200037265640800047465787400035061740001",
                58182709,
            ),
            (
                "instrument",
                63,
                "00000d6d696e6563726166743a666f6f004000000042c8000000080004486f726e",
                -2003793539,
            ),
            (
                "instrument",
                63,
                "00bf054000000042c80000030a080004746578740004486f726e010004626f6c640100",
                -1995771085,
            ),
            (
                "provides_trim_material",
                64,
                "000b6d696e6563726166743a7108000164",
                167259767,
            ),
            (
                "banner_patterns",
                74,
                "02000b6d696e6563726166743a62016b0e000b6d696e6563726166743a63016c0b",
                -178464072,
            ),
            ("can_break", 15, "0101020100000000", -1161294463),
            (
                "can_place_on",
                14,
                "02010301090103036c6974000101300006666163696e6701056e6f727468036167650001013101013300000001020900000000",
                -279938577,
            ),
            ("can_break", 15, "010000000203030c0200", 1412654667),
            ("can_break", 15, "010000000000", -982207288),
            ("can_break", 15, "01000100000000", 281125914),
            ("use_effects", 5, "01013f000000", -1226654547),
            ("use_effects", 5, "00013e4ccccd", -982207288),
            (
                "attribute_modifiers",
                16,
                "03030e6d696e6563726166743a746573744014000000000000000100170b6d696e6563726166743a78c004000000000000010001010b6d696e6563726166743a793ff00000000000000208020800024869",
                694631198,
            ),
            ("food", 23, "044020000001", 1478675052),
            ("food", 23, "010000000000", -72303299),
            ("consumable", 24, "3fcccccd01bf050100", -982207288),
            (
                "consumable",
                24,
                "4040000002a30a00050001000064000101003f0000000102120203410000000104000d6d696e6563726166743a666f6f0140800000",
                -1668265051,
            ),
            (
                "consumable",
                24,
                "3fcccccd01000d6d696e6563726166743a626172000100",
                1520278491,
            ),
            ("use_remainder", 25, "ee07010000", -1449413585),
            (
                "use_cooldown",
                26,
                "40000000010d6d696e6563726166743a677270",
                -783582811,
            ),
            ("use_cooldown", 26, "3f00000000", -818046427),
            (
                "tool",
                28,
                "020301090141000000010102010000400000000200",
                -1435046669,
            ),
            ("tool", 28, "003f8000000101", 937295970),
            ("weapon", 29, "023fc00000", 896609515),
            ("weapon", 29, "0100000000", -982207288),
            (
                "equippable",
                32,
                "0448010e6d696e6563726166743a676f6c64011a6d696e6563726166743a6d6973632f70756d706b696e626c757201029a010001010001a80b",
                -1318691560,
            ),
            ("equippable", 32, "00470000000101010000a80b", -352294351),
            (
                "equippable",
                32,
                "0547000001031e650100000100a80b",
                696858640,
            ),
            ("equippable", 32, "0747000000010101000033", -1214114539),
            ("equippable", 32, "06470000000101010000a80b", 1294431019),
            ("repairable", 33, "02fa07", -646002763),
            ("repairable", 33, "03fa07ee07", 1894900226),
            ("death_protection", 36, "0102", 1103435691),
            ("death_protection", 36, "00", -982207288),
            (
                "blocks_attacks",
                37,
                "3f0000003f8000000142340000003f8000003f000000404000003f8000003e4ccccd0001b40b00",
                -1535582378,
            ),
            (
                "blocks_attacks",
                37,
                "000000003f8000000142b4000000000000003f8000003f800000000000003f800000000000",
                -982207288,
            ),
            (
                "blocks_attacks",
                37,
                "000000003f8000000142b4000000000000003f8000003f800000000000003f800000000000",
                -982207288,
            ),
            (
                "blocks_attacks",
                37,
                "000000003f800000003f800000000000003f800000000000",
                354948751,
            ),
            ("piercing_weapon", 38, "010101bf0500", 1744160152),
            ("piercing_weapon", 38, "01000000", -982207288),
            (
                "kinetic_weapon",
                39,
                "050001143f8000000000000000003f0000003f80000001bf0500",
                775699335,
            ),
            (
                "kinetic_weapon",
                39,
                "0a00000000000000003f8000000000",
                -982207288,
            ),
            ("attack_animation", 40, "020a", -2140542379),
            ("interact_animation", 41, "0106", -982207288),
            ("interact_animation", 41, "0006", -178357229),
            ("break_sound", 83, "bf05", 1901640159),
            ("break_sound", 83, "000b6d696e6563726166743a7100", 924623336),
            ("compostable", 84, "0100000003", -1585875247),
            (
                "compostable",
                84,
                "000e6d696e6563726166743a736f6d65",
                190800813,
            ),
            ("cooking_fuel", 85, "01000000c8013fc00000", -1393125704),
            ("brewing_fuel", 86, "01000000050140000000", -1389039553),
            ("mob_visibility", 87, "02203f000000", 1234665895),
        ];
        for (name, kind, hex, expected) in cases {
            let bytes = unhex(hex);
            let mut r = Reader::new(&bytes);
            let c = Component::read(kind, &mut r).unwrap();
            assert_eq!(r.remaining(), 0, "{name}");
            assert_eq!(c.hash(kind, &Pinned), Some(expected), "{name} {hex}");
        }
    }

    #[test]
    fn what_vanilla_cannot_hash_or_horizon_cannot_name_stays_out() {
        // additional_trade_cost has no persistent codec (vanilla's client would throw).
        assert_eq!(Component::Raw(vec![3]).hash(42, &Pinned), None);
        // A registry id with no name.
        assert_eq!(Component::Raw(vec![9]).hash(88, &Pinned), None);
        // Bytes left over after the value.
        assert_eq!(Component::Raw(vec![0, 0]).hash(34, &Pinned), None);
    }

    /// Block predicates with an `nbt` or partial component predicates, and books
    /// with filtered text: stream bytes and hashes vanilla 26.3 printed (the same
    /// driver as above, SNBT through the component's codec). The `container` and
    /// `bundle_contents` predicates whose nested item predicate holds exact `components` are
    /// hashed too, when every byte in them is a named `Codec.BOOL`.
    #[test]
    fn predicates_and_filtered_books_hash_as_vanilla_does() {
        let cases: [(&str, i32, &str, Option<i32>); 24] = [
            (
                "stone_nbt",
                14,
                "0101020100010a010001610108000162000178000000",
                Some(-359417396),
            ),
            (
                "nbt_all_types",
                14,
                "010000010a08000361206200027122020001730007060001654202a05f20000000050001783fc000000600017940000000000000000b0001690000000200000001000000020900054974656d730a000000010800026964000e6d696e6563726166743a64697274010005436f756e7401000700017a0000000201020400016c00000000000000050c00014c0000000100000000000000030500016e00000000000000",
                Some(18640168),
            ),
            (
                "damage_partial",
                14,
                "010102010000000101000a0a000664616d6167650300036d696e000000010300036d6178000000050000",
                Some(-752850259),
            ),
            (
                "custom_data",
                14,
                "010000000001010408000d7b613a312c623a5b312c325d7d",
                Some(-1320400474),
            ),
            (
                "twinkle",
                14,
                "01000000000101070a080005736861706500047374617201000b6861735f7477696e6b6c650100",
                Some(-1476889576),
            ),
            (
                "trail_false",
                14,
                "01000000000101070a0100096861735f747261696c0000",
                Some(-1034820270),
            ),
            ("any_value", 14, "01000000000100020a00", Some(-1904716790)),
            (
                "villager",
                14,
                "010000000001010e0800106d696e6563726166743a646573657274",
                Some(1625640440),
            ),
            (
                "potion_effects",
                14,
                "01000000000101030a0a000765666665637473090008636f6e7461696e730a000000010a000f6d696e6563726166743a73706565640a00086475726174696f6e0300036d6178000000640001000776697369626c6500030009616d706c696669657200000001010007616d6269656e740100000000",
                Some(150754403),
            ),
            (
                "container",
                14,
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a0005636f756e740300036d696e00000002000800056974656d73000f6d696e6563726166743a73746f6e650003000473697a65000000030000",
                Some(-820017514),
            ),
            (
                "fireworks",
                14,
                "01000000000101080a0a000a6578706c6f73696f6e7303000473697a65000000010003000f666c696768745f6475726174696f6e0000000200",
                Some(-551561987),
            ),
            (
                "attribute_modifiers",
                14,
                "010000000001010b0a0a00096d6f64696669657273090008636f6e7461696e730a000000010a0006616d6f756e740600036d696e3ff800000000000000080009617474726962757465000f6d696e6563726166743a61726d6f72080004736c6f740004686561640800096f7065726174696f6e00096164645f76616c7565000000",
                Some(-233125185),
            ),
            (
                "bundle_stored",
                14,
                "01000000000201060a0a00056974656d73090008636f6e7461696e730a000000010800056974656d73000f6d696e6563726166743a73746f6e650000000102090a000000010300066c6576656c730000000300",
                Some(1901599730),
            ),
            (
                "writable_book",
                14,
                "01000000000101090a0a00057061676573090008636f6e7461696e7308000000010001780000",
                Some(-883070685),
            ),
            (
                "written_book",
                14,
                "010000000001010a0a03000a67656e65726174696f6e000000020a00057061676573090008636f6e7461696e730a000000010800047465787400026869010004626f6c64010000080006617574686f720001410100087265736f6c7665640000",
                Some(1470044104),
            ),
            (
                "all_fields",
                14,
                "010000010a0300016100000001000101050301000a03000a6475726162696c697479000000030001040800027b7d00020a00",
                Some(808431166),
            ),
            (
                "container_components",
                14,
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730100246d696e6563726166743a656e6368616e746d656e745f676c696e745f6f7665727269646501000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                Some(-1100559882),
            ),
            (
                "container_components_false",
                14,
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730100246d696e6563726166743a656e6368616e746d656e745f676c696e745f6f7665727269646500000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                Some(-566759991),
            ),
            (
                "bundle_components_damage_and_name",
                14,
                "01000000000101060a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730300106d696e6563726166743a64616d616765000000050a00156d696e6563726166743a637573746f6d5f6e616d650800047465787400026869010004626f6c640100000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                Some(-473617213),
            ),
            (
                "container_components_food",
                14,
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730a000e6d696e6563726166743a666f6f6405000a73617475726174696f6e400000000300096e7574726974696f6e0000000401000e63616e5f616c776179735f6561740100000800056974656d73000f6d696e6563726166743a6170706c65000a000473697a650300036d696e00000001000000",
                Some(1890827479),
            ),
            (
                "can_break_two",
                15,
                "0201020100010a000000000000000100020a00",
                Some(2146318348),
            ),
            (
                "written_filtered",
                57,
                "01540101460161010208000170010a08000474657874000171010004626f6c640100080005706c61696e0001",
                Some(1884774986),
            ),
            (
                "written_plain",
                57,
                "01540001610001080001700000",
                Some(100213250),
            ),
            (
                "writable_filtered",
                56,
                "03036f6e6501022a2a0374776f000133010133",
                Some(1192237218),
            ),
        ];
        for (name, kind, hex, expected) in cases {
            let bytes = unhex(hex);
            let mut r = Reader::new(&bytes);
            let c = Component::read(kind, &mut r).unwrap();
            assert_eq!(r.remaining(), 0, "{name}");
            assert_eq!(c.hash(kind, &Pinned), expected, "{name} {hex}");
        }
    }

    /// Item predicates inside a `container` or `bundle_contents` predicate with
    /// exact `components`: each component's value is NBT whose booleans (found
    /// by that component's codec) hash as booleans. Stream bytes and hashes
    /// vanilla 26.3 printed (a throwaway driver on the real jars: SNBT through
    /// `can_place_on`'s codec, `STREAM_CODEC.encode`, `encodeValue` through
    /// `HashOps.CRC32C_INSTANCE`).
    #[test]
    fn exact_components_in_predicates_hash_as_vanilla_does() {
        let cases: [(&str, &str, i32); 21] = [
            (
                "glint_true",
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730100246d696e6563726166743a656e6368616e746d656e745f676c696e745f6f7665727269646501000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                -1100559882,
            ),
            (
                "glint_false",
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730100246d696e6563726166743a656e6368616e746d656e745f676c696e745f6f7665727269646500000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                -566759991,
            ),
            (
                "custom_data_raw",
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730a00156d696e6563726166743a637573746f6d5f6461746101000161010a000a636f6d706f6e656e74730100017801000900016c01000000010100000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                1397969118,
            ),
            (
                "food",
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730a000e6d696e6563726166743a666f6f6405000a73617475726174696f6e3f8000000300096e7574726974696f6e0000000201000e63616e5f616c776179735f6561740100000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                371373365,
            ),
            (
                "equippable",
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730a00146d696e6563726166743a65717569707061626c6501000b64697370656e7361626c6500080004736c6f74000468656164010009737761707061626c650001000e63616e5f62655f736865617265640100000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                560962810,
            ),
            (
                "model_data",
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730a001b6d696e6563726166743a637573746f6d5f6d6f64656c5f64617461090006666c6f61747305000000013f800000090007737472696e67730800000001000161090005666c61677301000000020100090006636f6c6f727303000000010000000500000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                -378751391,
            ),
            (
                "tooltip",
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730a00196d696e6563726166743a746f6f6c7469705f646973706c617901000c686964655f746f6f6c7469700109001168696464656e5f636f6d706f6e656e7473080000000100166d696e6563726166743a656e6368616e746d656e747300000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                226858365,
            ),
            (
                "consumable",
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730a00146d696e6563726166743a636f6e73756d61626c650900126f6e5f636f6e73756d655f656666656374730a00000002090007656666656374730a000000010300086475726174696f6e0000001401000973686f775f69636f6e00010007616d6269656e74010800026964000f6d696e6563726166743a7370656564000800047479706500176d696e6563726166743a6170706c795f6566666563747300010015646972656374696f6e616c5f7061727469636c6573000500086469616d657465724100000008000474797065001b6d696e6563726166743a74656c65706f72745f72616e646f6d6c79000100156861735f636f6e73756d655f7061727469636c65730000000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                2077954757,
            ),
            (
                "potion",
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730a00196d696e6563726166743a706f74696f6e5f636f6e74656e747309000e637573746f6d5f656666656374730a0000000101000973686f775f69636f6e00010007616d6269656e74010800026964000f6d696e6563726166743a737065656401000e73686f775f7061727469636c6573000000000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                1557644663,
            ),
            (
                "names",
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e747309000e6d696e6563726166743a6c6f72650a00000002080004746578740001610100066974616c696301000800000005706c61696e000a00156d696e6563726166743a637573746f6d5f6e616d650800047465787400026869010004626f6c640100000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                -1338505994,
            ),
            (
                "book",
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730a001e6d696e6563726166743a7772697474656e5f626f6f6b5f636f6e74656e7403000a67656e65726174696f6e0000000109000570616765730a0000000108000866696c74657265640001780a000372617708000474657874000170010004626f6c64010000080006617574686f720001410a00057469746c65080003726177000154000100087265736f6c7665640100000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                -875258993,
            ),
            (
                "fireworks",
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730a00136d696e6563726166743a66697265776f726b7309000a6578706c6f73696f6e730a000000010100096861735f747261696c010800057368617065000473746172090006636f6c6f72730300000001000000010001000f666c696768745f6475726174696f6e0200000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                863655503,
            ),
            (
                "lodestone",
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730a001b6d696e6563726166743a6c6f646573746f6e655f747261636b6572010007747261636b65640000000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                -536467480,
            ),
            (
                "piercing",
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730a00196d696e6563726166743a7069657263696e675f776561706f6e01000f6465616c735f6b6e6f636b6261636b000100096469736d6f756e74730100000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                -335896069,
            ),
            (
                "tool",
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730a000e6d696e6563726166743a746f6f6c01001e63616e5f64657374726f795f626c6f636b735f696e5f63726561746976650009000572756c65730a00000001080006626c6f636b73000f6d696e6563726166743a73746f6e65010011636f72726563745f666f725f64726f7073010500057370656564400000000000000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                889667295,
            ),
            (
                "modifiers",
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e747309001d6d696e6563726166743a6174747269627574655f6d6f646966696572730a00000001060006616d6f756e743ff00000000000000a0007646973706c61790800047479706500086f766572726964650a000576616c756508000474657874000178010004626f6c640100000800026964000b6d696e6563726166743a6108000474797065000f6d696e6563726166743a61726d6f720800096f7065726174696f6e00096164645f76616c756500000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                -1960494218,
            ),
            (
                "nested_container",
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730900136d696e6563726166743a636f6e7461696e65720a000000010a00046974656d0a000a636f6d706f6e656e74730a001b6d696e6563726166743a637573746f6d5f6d6f64656c5f64617461090005666c616773010000000101000100246d696e6563726166743a656e6368616e746d656e745f676c696e745f6f76657272696465010a0011216d696e6563726166743a64616d6167650000030005636f756e74000000020800026964000f6d696e6563726166743a73746f6e6500030004736c6f740000000000000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                -1517206508,
            ),
            (
                "nested_bundle",
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730900196d696e6563726166743a62756e646c655f636f6e74656e74730a000000020a000a636f6d706f6e656e74730a00156d696e6563726166743a637573746f6d5f64617461010001620100000800026964000e6d696e6563726166743a64697274000800026964000f6d696e6563726166743a73746f6e6500000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                -1880632630,
            ),
            (
                "two",
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730a000e6d696e6563726166743a666f6f6405000a73617475726174696f6e000000000300096e7574726974696f6e00000001000100246d696e6563726166743a656e6368616e746d656e745f676c696e745f6f76657272696465010300146d696e6563726166743a6d61785f64616d61676500000005000800056974656d73000f6d696e6563726166743a73746f6e65000000",
                -414677810,
            ),
            (
                "nested_predicates",
                "01000000000101050a0a00056974656d73090008636f6e7461696e730a000000010a000a636f6d706f6e656e74730100246d696e6563726166743a656e6368616e746d656e745f676c696e745f6f7665727269646501000a000a707265646963617465730a001c6d696e6563726166743a66697265776f726b5f6578706c6f73696f6e0100096861735f747261696c0100000003000473697a65000000030000",
                -374469656,
            ),
            (
                "bundle_pred",
                "01000000000101060a0a00056974656d73090005636f756e740a000000010a0004746573740a000a636f6d706f6e656e74730a000e6d696e6563726166743a666f6f6405000a73617475726174696f6e3f8000000300096e7574726974696f6e0000000301000e63616e5f616c776179735f6561740100000800056974656d73000e6d696e6563726166743a64697274000a0005636f756e740300036d696e0000000100000000",
                -166266908,
            ),
        ];
        for (name, hex, expected) in cases {
            let bytes = unhex(hex);
            let mut r = Reader::new(&bytes);
            let c = Component::read(14, &mut r).unwrap();
            assert_eq!(r.remaining(), 0, "{name}");
            assert_eq!(c.hash(14, &Pinned), Some(expected), "{name}");
        }
    }
}
