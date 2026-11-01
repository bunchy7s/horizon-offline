//! The data components that tune an item's behaviour (`food`, `consumable`,
//! `tool`, `equippable`, `attribute_modifiers`, ...), from their stream form to
//! the codec form. See [`super::raw`] for the rest.
use super::*;
use crate::buf::Reader;
use horizon_world::snbt_text::to_snbt;

const USE_ANIMATION: [&str; 12] = [
    "none",
    "eat",
    "drink",
    "block",
    "bow",
    "trident",
    "crossbow",
    "spyglass",
    "toot_horn",
    "brush",
    "bundle",
    "spear",
];
const SWING: [&str; 3] = ["none", "whack", "stab"];
/// `EquipmentSlot` by `id` (not declaration order).
const SLOT: [&str; 8] = [
    "mainhand", "feet", "legs", "chest", "head", "offhand", "body", "saddle",
];
/// `EquipmentSlotGroup` by id.
const SLOT_GROUP: [&str; 11] = [
    "any", "mainhand", "offhand", "hand", "feet", "legs", "chest", "head", "armor", "body",
    "saddle",
];
const OPERATION: [&str; 3] = ["add_value", "add_multiplied_base", "add_multiplied_total"];

fn int(r: &mut Reader) -> Option<i32> {
    r.varint().ok()
}
fn float(r: &mut Reader) -> Option<f32> {
    r.f32().ok()
}
fn boolean(r: &mut Reader) -> Option<bool> {
    r.bool().ok()
}
fn ident(r: &mut Reader) -> Option<HashCode> {
    Some(string(&r.string(32767).ok()?))
}
/// `Codec.xxx.optionalFieldOf(name, default)` for a float.
fn float_or(rec: RecordHash, name: &str, v: f32, default: f32) -> RecordHash {
    or_default(
        rec,
        name,
        v.to_bits(),
        default.to_bits(),
        HashOps::create_f32(v),
    )
}
fn int_or(rec: RecordHash, name: &str, v: i32, default: i32) -> RecordHash {
    or_default(rec, name, v, default, HashOps::create_int(v))
}
fn bool_or(rec: RecordHash, name: &str, v: bool, default: bool) -> RecordHash {
    or_default(rec, name, v, default, HashOps::create_boolean(v))
}
fn optional<T>(
    r: &mut Reader,
    element: impl FnOnce(&mut Reader) -> Option<T>,
) -> Option<Option<T>> {
    if boolean(r)? {
        Some(Some(element(r)?))
    } else {
        Some(None)
    }
}
fn list<T>(r: &mut Reader, mut element: impl FnMut(&mut Reader) -> Option<T>) -> Option<Vec<T>> {
    let n = r.count(r.remaining()).ok()?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(element(r)?);
    }
    Some(out)
}

/// A `SoundEvent` holder: the codec form and, for a reference, its name (what a
/// field default compares against).
pub(super) struct Sound {
    pub(super) code: HashCode,
    name: Option<String>,
}
/// `SoundEvent.STREAM_CODEC`: 0 and the event itself (`DIRECT_CODEC`: `sound_id`
/// and an optional `range`), or the registry id plus one.
pub(super) fn sound(r: &mut Reader, names: &dyn Names) -> Option<Sound> {
    match int(r)? {
        0 => {
            let id = ident(r)?;
            let range = optional(r, float)?;
            Some(Sound {
                code: RecordHash::new()
                    .field("sound_id", id)
                    .optional("range", range.map(HashOps::create_f32))
                    .build(),
                name: None,
            })
        }
        id => {
            let name = names.name("minecraft:sound_event", id - 1)?;
            Some(Sound {
                code: string(&name),
                name: Some(name),
            })
        }
    }
}
/// `SoundEvent.CODEC.optionalFieldOf(name, default)`.
fn sound_or(rec: RecordHash, name: &str, s: Sound, default: &str) -> RecordHash {
    if s.name.as_deref() == Some(default) {
        rec
    } else {
        rec.field(name, s.code)
    }
}
/// `ByteBufCodecs.holderSet`: 0 and a tag, or the size plus one and the ids.
/// `HolderSetCodec` writes `#tag`, a lone entry as itself, else a list.
fn holder_set(registry: &str, names: &dyn Names, r: &mut Reader) -> Option<HashCode> {
    match int(r)? {
        0 => Some(string(&format!("#{}", r.string(32767).ok()?))),
        n => {
            let n = usize::try_from(n - 1)
                .ok()
                .filter(|n| *n <= r.remaining())?;
            let mut ids = Vec::with_capacity(n);
            for _ in 0..n {
                ids.push(entry(names, registry, int(r)?)?);
            }
            Some(if let [only] = ids[..] {
                only
            } else {
                HashOps::create_list(ids)
            })
        }
    }
}
/// `ByteBufCodecs.holderRegistry`.
fn holder(registry: &str, names: &dyn Names, r: &mut Reader) -> Option<HashCode> {
    entry(names, registry, int(r)?)
}

/// `ConsumeEffect.CODEC`: a `type` field and the type's fields.
fn consume_effect(r: &mut Reader, names: &dyn Names) -> Option<HashCode> {
    let kind = int(r)?;
    let rec = RecordHash::new();
    Some(
        match kind {
            0 => {
                let effects = list(r, |r| super::raw::mob_effect(r, names))?;
                let probability = float(r)?;
                float_or(
                    rec.field("type", string("minecraft:apply_effects"))
                        .field("effects", HashOps::create_list(effects)),
                    "probability",
                    probability,
                    1.0,
                )
            }
            1 => rec
                .field("type", string("minecraft:remove_effects"))
                .field("effects", holder_set("minecraft:mob_effect", names, r)?),
            2 => rec.field("type", string("minecraft:clear_all_effects")),
            3 => {
                let diameter = float(r)?;
                let particles = boolean(r)?;
                bool_or(
                    float_or(
                        rec.field("type", string("minecraft:teleport_randomly")),
                        "diameter",
                        diameter,
                        16.0,
                    ),
                    "directional_particles",
                    particles,
                    true,
                )
            }
            4 => rec
                .field("type", string("minecraft:play_sound"))
                .field("sound", sound(r, names)?.code),
            _ => return None,
        }
        .build(),
    )
}

/// `SwingAnimation`: `DEFAULT` is whack for 6 ticks.
fn swing(r: &mut Reader) -> Option<HashCode> {
    let kind = int(r)?;
    let duration = int(r)?;
    let rec = or_default(RecordHash::new(), "type", kind, 1, pick(&SWING, kind)?);
    Some(int_or(rec, "duration", duration, 6).build())
}

/// `ResolvableInt` / `ResolvableFloat`: a constant, or a context provider's key.
fn resolvable(r: &mut Reader, constant: fn(&mut Reader) -> Option<HashCode>) -> Option<HashCode> {
    if boolean(r)? { constant(r) } else { ident(r) }
}
fn fixed_int(r: &mut Reader) -> Option<HashCode> {
    Some(HashOps::create_int(r.i32().ok()?))
}
fn fixed_float(r: &mut Reader) -> Option<HashCode> {
    Some(HashOps::create_f32(float(r)?))
}

/// `KineticWeapon.Condition`.
fn kinetic_condition(r: &mut Reader) -> Option<HashCode> {
    let ticks = int(r)?;
    let speed = float(r)?;
    let relative = float(r)?;
    let rec = RecordHash::new().field("max_duration_ticks", HashOps::create_int(ticks));
    let rec = float_or(rec, "min_speed", speed, 0.0);
    Some(float_or(rec, "min_relative_speed", relative, 0.0).build())
}

/// The `Codec.BOOL` fields of the component predicates' codecs
/// (`FireworkPredicate`, `WrittenBookPredicate`, `MobEffectInstancePredicate`):
/// they travel as NBT bytes and hash as booleans. The text components inside a
/// written book's page predicates add [`TEXT_BOOLS`].
pub(super) const PREDICATE_BOOLS: [&str; 12] = [
    "has_twinkle",
    "has_trail",
    "resolved",
    "ambient",
    "visible",
    "bold",
    "italic",
    "underlined",
    "strikethrough",
    "obfuscated",
    "interpret",
    "plain",
];
/// `DataComponentPredicate.STREAM_CODEC`: each entry is a type (a predicate
/// type, or a component type for `AnyValue`) and the predicate as the NBT its
/// `CODEC` encodes (`fromCodecWithRegistries`), hashed as the map
/// `DataComponentPredicate.CODEC` (`Codec.dispatchedMap`) writes.
fn partial_predicate(r: &mut Reader, names: &dyn Names) -> Option<(HashCode, HashCode)> {
    let registry = if boolean(r)? {
        "minecraft:data_component_predicate_type"
    } else {
        "minecraft:data_component_type"
    };
    let key = entry(names, registry, int(r)?)?;
    let tag = r.nbt().ok()?;
    // The nested `components` of an item predicate (`container`,
    // `bundle_contents`) are component values as NBT: `exact` hashes them by
    // each component's codec.
    Some((key, super::exact::predicate(&tag, names)?))
}

/// `BlockPredicate.STREAM_CODEC` as `BlockPredicate.CODEC` writes it.
fn block_predicate(r: &mut Reader, names: &dyn Names) -> Option<HashCode> {
    let blocks = optional(r, |r| holder_set("minecraft:block", names, r))?;
    // StatePropertiesPredicate: a name and an exact value or a (min, max) range.
    let state = optional(r, |r| {
        let props = list(r, |r| {
            let name = string(&r.string(32767).ok()?);
            let matcher = if boolean(r)? {
                string(&r.string(32767).ok()?)
            } else {
                let min = optional(r, |r| Some(string(&r.string(32767).ok()?)))?;
                let max = optional(r, |r| Some(string(&r.string(32767).ok()?)))?;
                RecordHash::new()
                    .optional("min", min)
                    .optional("max", max)
                    .build()
            };
            Some((name, matcher))
        })?;
        Some(HashOps::create_map(props))
    })?;
    // NbtPredicate: `TagParser.LENIENT_CODEC` encodes with its flattened codec,
    // the tag's SNBT string.
    let nbt = optional(r, |r| match r.nbt().ok()? {
        tag @ horizon_world::nbt::Tag::Compound(_) => Some(string(&to_snbt(&tag))),
        _ => None,
    })?;
    // DataComponentExactPredicate: the expected components, by type name.
    let exact = list(r, |r| {
        let kind = int(r)?;
        let name = entry(names, "minecraft:data_component_type", kind)?;
        let value = Component::read(kind, r).ok()?.code(kind, names)?;
        Some((name, value))
    })?;
    // DataComponentPredicate: the partial matchers (at most 64).
    let partial = list(r, |r| partial_predicate(r, names))?;
    if partial.len() > 64 {
        return None;
    }
    let rec = RecordHash::new()
        .optional("blocks", blocks)
        .optional("state", state)
        .optional("nbt", nbt);
    let rec = if exact.is_empty() {
        rec
    } else {
        rec.field("components", HashOps::create_map(exact))
    };
    Some(
        if partial.is_empty() {
            rec
        } else {
            rec.field("predicates", HashOps::create_map(partial))
        }
        .build(),
    )
}

/// The hash of component `kind` from its stream, or `None`.
pub(super) fn code(kind: i32, r: &mut Reader, names: &dyn Names) -> Option<HashCode> {
    Some(match kind {
        // UseEffects: canSprint, interactVibrations, speedMultiplier.
        5 => {
            let (sprint, vibrations, speed) = (boolean(r)?, boolean(r)?, float(r)?);
            let rec = bool_or(RecordHash::new(), "can_sprint", sprint, false);
            let rec = bool_or(rec, "interact_vibrations", vibrations, true);
            float_or(rec, "speed_multiplier", speed, 0.2).build()
        }
        // ItemAttributeModifiers: attribute, modifier, slot, display.
        16 => HashOps::create_list(list(r, |r| {
            let attribute = holder("minecraft:attribute", names, r)?;
            let id = ident(r)?;
            let amount = r.f64().ok()?;
            let operation = pick(&OPERATION, int(r)?)?;
            let slot = int(r)?;
            let rec = RecordHash::new()
                .field("type", attribute)
                .field("id", id)
                .field("amount", HashOps::create_double(amount.to_bits()))
                .field("operation", operation);
            let rec = or_default(rec, "slot", slot, 0, pick(&SLOT_GROUP, slot)?);
            Some(
                match int(r)? {
                    0 => rec,
                    1 => rec.field(
                        "display",
                        RecordHash::new().field("type", string("hidden")).build(),
                    ),
                    2 => rec.field(
                        "display",
                        RecordHash::new()
                            .field("type", string("override"))
                            .field("value", text(&r.nbt().ok()?))
                            .build(),
                    ),
                    _ => return None,
                }
                .build(),
            )
        })?),
        // FoodProperties.DIRECT_CODEC: nutrition, saturation, canAlwaysEat.
        23 => {
            let nutrition = int(r)?;
            let saturation = float(r)?;
            bool_or(
                RecordHash::new()
                    .field("nutrition", HashOps::create_int(nutrition))
                    .field("saturation", HashOps::create_f32(saturation)),
                "can_always_eat",
                boolean(r)?,
                false,
            )
            .build()
        }
        // Consumable: consumeSeconds, animation, sound, hasConsumeParticles, effects.
        24 => {
            let seconds = float(r)?;
            let animation = int(r)?;
            let snd = sound(r, names)?;
            let particles = boolean(r)?;
            let effects = list(r, |r| consume_effect(r, names))?;
            let rec = float_or(RecordHash::new(), "consume_seconds", seconds, 1.6);
            let rec = or_default(
                rec,
                "animation",
                animation,
                1,
                pick(&USE_ANIMATION, animation)?,
            );
            let rec = sound_or(rec, "sound", snd, "minecraft:entity.generic.eat");
            let rec = bool_or(rec, "has_consume_particles", particles, true);
            if effects.is_empty() {
                rec
            } else {
                rec.field("on_consume_effects", HashOps::create_list(effects))
            }
            .build()
        }
        // UseCooldown: seconds, cooldownGroup.
        26 => {
            let seconds = float(r)?;
            RecordHash::new()
                .field("seconds", HashOps::create_f32(seconds))
                .optional("cooldown_group", optional(r, ident)?)
                .build()
        }
        // DamageResistant, Repairable: a holder set.
        27 => RecordHash::new()
            .field("types", holder_set("minecraft:damage_type", names, r)?)
            .build(),
        33 => RecordHash::new()
            .field("items", holder_set("minecraft:item", names, r)?)
            .build(),
        // Tool: rules, defaultMiningSpeed, damagePerBlock, canDestroyBlocksInCreative.
        28 => {
            let rules = list(r, |r| {
                let blocks = holder_set("minecraft:block", names, r)?;
                let speed = optional(r, float)?;
                let drops = optional(r, boolean)?;
                Some(
                    RecordHash::new()
                        .field("blocks", blocks)
                        .optional("speed", speed.map(HashOps::create_f32))
                        .optional("correct_for_drops", drops.map(HashOps::create_boolean))
                        .build(),
                )
            })?;
            let (mining, damage, creative) = (float(r)?, int(r)?, boolean(r)?);
            let rec = RecordHash::new().field("rules", HashOps::create_list(rules));
            let rec = float_or(rec, "default_mining_speed", mining, 1.0);
            let rec = int_or(rec, "damage_per_block", damage, 1);
            bool_or(rec, "can_destroy_blocks_in_creative", creative, true).build()
        }
        // Weapon: itemDamagePerAttack, disableBlockingForSeconds.
        29 => {
            let damage = int(r)?;
            let seconds = float(r)?;
            float_or(
                int_or(RecordHash::new(), "item_damage_per_attack", damage, 1),
                "disable_blocking_for_seconds",
                seconds,
                0.0,
            )
            .build()
        }
        // Equippable.
        32 => {
            let slot = pick(&SLOT, int(r)?)?;
            let equip = sound(r, names)?;
            let asset = optional(r, ident)?;
            let overlay = optional(r, ident)?;
            let entities = optional(r, |r| holder_set("minecraft:entity_type", names, r))?;
            let flags = [
                boolean(r)?,
                boolean(r)?,
                boolean(r)?,
                boolean(r)?,
                boolean(r)?,
            ];
            let shearing = sound(r, names)?;
            let rec = RecordHash::new().field("slot", slot);
            let rec = sound_or(
                rec,
                "equip_sound",
                equip,
                "minecraft:item.armor.equip_generic",
            )
            .optional("asset_id", asset)
            .optional("camera_overlay", overlay)
            .optional("allowed_entities", entities);
            let rec = bool_or(rec, "dispensable", flags[0], true);
            let rec = bool_or(rec, "swappable", flags[1], true);
            let rec = bool_or(rec, "damage_on_hurt", flags[2], true);
            let rec = bool_or(rec, "equip_on_interact", flags[3], false);
            let rec = bool_or(rec, "can_be_sheared", flags[4], false);
            sound_or(
                rec,
                "shearing_sound",
                shearing,
                "minecraft:item.shears.snip",
            )
            .build()
        }
        // DeathProtection: deathEffects.
        36 => {
            let effects = list(r, |r| consume_effect(r, names))?;
            if effects.is_empty() {
                RecordHash::new()
            } else {
                RecordHash::new().field("death_effects", HashOps::create_list(effects))
            }
            .build()
        }
        // BlocksAttacks.
        37 => {
            let delay = float(r)?;
            let scale = float(r)?;
            let reductions = list(r, |r| {
                let angle = float(r)?;
                let types = optional(r, |r| holder_set("minecraft:damage_type", names, r))?;
                let (base, factor) = (float(r)?, float(r)?);
                Some((angle, types, base, factor))
            })?;
            let item = [float(r)?, float(r)?, float(r)?];
            let bypassed = optional(r, |r| holder_set("minecraft:damage_type", names, r))?;
            let block = optional(r, |r| sound(r, names))?;
            let disabled = optional(r, |r| sound(r, names))?;
            let rec = float_or(RecordHash::new(), "block_delay_seconds", delay, 0.0);
            let rec = float_or(rec, "disable_cooldown_scale", scale, 1.0);
            // The default is one reduction: 90 degrees, any damage, base 0, factor 1.
            let is_default = matches!(
                &reductions[..],
                [(a, None, b, f)] if a.to_bits() == 90f32.to_bits() && *b == 0.0 && *f == 1.0
            );
            let rec = if is_default {
                rec
            } else {
                rec.field(
                    "damage_reductions",
                    HashOps::create_list(reductions.into_iter().map(
                        |(angle, types, base, factor)| {
                            float_or(RecordHash::new(), "horizontal_blocking_angle", angle, 90.0)
                                .optional("type", types)
                                .field("base", HashOps::create_f32(base))
                                .field("factor", HashOps::create_f32(factor))
                                .build()
                        },
                    )),
                )
            };
            let rec = if item.map(f32::to_bits) == [1.0f32, 0.0, 1.0].map(f32::to_bits) {
                rec
            } else {
                rec.field(
                    "item_damage",
                    RecordHash::new()
                        .field("threshold", HashOps::create_f32(item[0]))
                        .field("base", HashOps::create_f32(item[1]))
                        .field("factor", HashOps::create_f32(item[2]))
                        .build(),
                )
            };
            rec.optional("bypassed_by", bypassed)
                .optional("block_sound", block.map(|s| s.code))
                .optional("disabled_sound", disabled.map(|s| s.code))
                .build()
        }
        // PiercingWeapon: dealsKnockback, dismounts, sound, hitSound.
        38 => {
            let (knockback, dismounts) = (boolean(r)?, boolean(r)?);
            let snd = optional(r, |r| sound(r, names))?;
            let hit = optional(r, |r| sound(r, names))?;
            bool_or(
                bool_or(RecordHash::new(), "deals_knockback", knockback, true),
                "dismounts",
                dismounts,
                false,
            )
            .optional("sound", snd.map(|s| s.code))
            .optional("hit_sound", hit.map(|s| s.code))
            .build()
        }
        // KineticWeapon.
        39 => {
            let (cooldown, delay) = (int(r)?, int(r)?);
            let dismount = optional(r, kinetic_condition)?;
            let knockback = optional(r, kinetic_condition)?;
            let damage = optional(r, kinetic_condition)?;
            let (forward, multiplier) = (float(r)?, float(r)?);
            let snd = optional(r, |r| sound(r, names))?;
            let hit = optional(r, |r| sound(r, names))?;
            let rec = int_or(RecordHash::new(), "contact_cooldown_ticks", cooldown, 10);
            let rec = int_or(rec, "delay_ticks", delay, 0)
                .optional("dismount_conditions", dismount)
                .optional("knockback_conditions", knockback)
                .optional("damage_conditions", damage);
            let rec = float_or(rec, "forward_movement", forward, 0.0);
            float_or(rec, "damage_multiplier", multiplier, 1.0)
                .optional("sound", snd.map(|s| s.code))
                .optional("hit_sound", hit.map(|s| s.code))
                .build()
        }
        40 | 41 => swing(r)?,
        // `JukeboxSong.CODEC` is a `RegistryFixedCodec`: a direct song cannot be
        // encoded (vanilla's client would throw), so only a reference hashes.
        66 => match int(r)? {
            0 => return None,
            id => entry(names, "minecraft:jukebox_song", id - 1)?,
        },
        67 => holder_set("minecraft:banner_pattern", names, r)?,
        83 => sound(r, names)?.code,
        84 => RecordHash::new()
            .field("layers", resolvable(r, fixed_int)?)
            .build(),
        // CookingFuel (burnTime, speedMultiplier), BrewingFuel (uses, speedMultiplier).
        85 | 86 => {
            let first = resolvable(r, fixed_int)?;
            let speed = resolvable(r, fixed_float)?;
            RecordHash::new()
                .field(if kind == 85 { "burn_time" } else { "uses" }, first)
                .field("speed_multiplier", speed)
                .build()
        }
        // MobVisibility: targetingEntityTypes, visibility.
        87 => {
            let types = holder_set("minecraft:entity_type", names, r)?;
            RecordHash::new()
                .field("targeting_entity_types", types)
                .field("visibility", HashOps::create_f32(float(r)?))
                .build()
        }
        // AdventureModePredicate (can_place_on, can_break): `compactListCodec`
        // writes one predicate as itself and `nonEmptyList` refuses none.
        14 | 15 => {
            let mut predicates = list(r, |r| block_predicate(r, names))?;
            match predicates.len() {
                0 => return None,
                1 => predicates.remove(0),
                _ => HashOps::create_list(predicates),
            }
        }
        // UseRemainder: one template.
        25 => super::raw::template_code(r, names)?,
        _ => return None,
    })
}
