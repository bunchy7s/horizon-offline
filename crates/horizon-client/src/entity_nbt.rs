//! Vanilla 26.3 entity NBT (`Entity.saveWithoutId` and the subclasses'
//! `addAdditionalSaveData`) for the entities Horizon simulates: mobs, item
//! entities, experience orbs, arrows and falling blocks. Encoding starts from
//! the compound the entity was loaded with, when there is one, so fields
//! Horizon does not model (variants, attributes, equipment, custom names)
//! survive a Horizon save.
use crate::{
    arrows::Pickup,
    entities::{ArmorStand, Cushion, ExperienceOrb, ItemEntity, ItemFrame, Painting},
    mobs::{Mob, MobType, Who},
    projectiles::{Projectile, ProjectileKind},
    save::{
        doubles, float, floats, item_from_nbt, item_to_nbt, number, put, read_doubles, text,
        uuid_from_tag,
    },
};
use horizon_core::geometry::Vec3;
use horizon_world::nbt::{Compound, Tag};
use std::{collections::HashMap, sync::Arc};

fn string(s: &str) -> Tag {
    Tag::String(s.into())
}
/// Entity type ID (`id`).
pub fn kind(tag: &Tag) -> Option<String> {
    text(tag.get("id"))
}
pub fn position(tag: &Tag) -> Option<Vec3> {
    read_doubles::<3>(tag.get("Pos")).map(|p| Vec3::new(p[0], p[1], p[2]))
}
pub fn uuid(tag: &Tag) -> Option<[i32; 4]> {
    uuid_from_tag(tag.get("UUID"))
}
pub fn motion(tag: &Tag) -> Vec3 {
    read_doubles::<3>(tag.get("Motion")).map_or(Vec3::ZERO, |m| Vec3::new(m[0], m[1], m[2]))
}
pub fn rotation(tag: &Tag) -> [f32; 2] {
    read_doubles::<2>(tag.get("Rotation")).map_or([0.0; 2], |r| [r[0] as f32, r[1] as f32])
}
fn flag(tag: &Tag, key: &str) -> bool {
    number(tag.get(key)).unwrap_or(0) != 0
}
/// `Entity.saveWithoutId` fields every entity has, over the loaded compound.
struct Base<'a> {
    id: &'a str,
    uuid: [i32; 4],
    position: Vec3,
    motion: Vec3,
    rotation: [f32; 2],
    on_ground: bool,
    fall_distance: f64,
}
impl Base<'_> {
    fn tag(&self, raw: Option<&Tag>) -> Tag {
        let mut tag = match raw {
            Some(t @ Tag::Compound(_)) => t.clone(),
            _ => Tag::Compound(Compound::new()),
        };
        for (key, value) in [
            ("id", string(self.id)),
            ("UUID", Tag::IntArray(self.uuid.to_vec())),
            (
                "Pos",
                doubles(&[self.position.x, self.position.y, self.position.z]),
            ),
            (
                "Motion",
                doubles(&[self.motion.x, self.motion.y, self.motion.z]),
            ),
            ("Rotation", floats(&self.rotation)),
            ("OnGround", Tag::Byte(i8::from(self.on_ground))),
            ("fall_distance", Tag::Double(self.fall_distance.to_bits())),
        ] {
            put(&mut tag, key, value);
        }
        if let Tag::Compound(map) = &mut tag {
            for (key, value) in [
                ("Fire", Tag::Short(-1)),
                ("Air", Tag::Short(300)),
                ("Invulnerable", Tag::Byte(0)),
                ("PortalCooldown", Tag::Int(0)),
            ] {
                map.entry(key.into()).or_insert(value);
            }
        }
        tag
    }
}

/// `LivingEntity` + `Mob` (+ `AgeableMob`, creeper and skeleton specifics).
/// The mobs whose class extends vanilla's `Animal` (and so save `InLove`):
/// not allays, villagers, golems, dolphins or fish.
fn is_animal(id: &str) -> bool {
    matches!(
        id.strip_prefix("minecraft:").unwrap_or(id),
        "pig"
            | "cow"
            | "mooshroom"
            | "sheep"
            | "chicken"
            | "rabbit"
            | "horse"
            | "donkey"
            | "mule"
            | "zombie_horse"
            | "skeleton_horse"
            | "llama"
            | "trader_llama"
            | "goat"
            | "panda"
            | "polar_bear"
            | "fox"
            | "wolf"
            | "cat"
            | "ocelot"
            | "parrot"
            | "bee"
            | "turtle"
            | "armadillo"
            | "frog"
            | "axolotl"
            | "hoglin"
            | "strider"
            | "camel"
            | "camel_husk"
            | "sniffer"
            | "happy_ghast"
            | "nautilus"
            | "zombie_nautilus"
    )
}
/// The `NeutralMob`s whose anger is `Extra::anger`: they write and read
/// `anger_end_time` and `angry_at`. The bee keeps its own anger, and a
/// piglin's is a brain memory, not `NeutralMob`'s.
fn persistent_anger(s: crate::mobs::Species) -> bool {
    use crate::mobs::Species as S;
    matches!(
        s,
        S::Wolf | S::IronGolem | S::PolarBear | S::Enderman | S::ZombifiedPiglin
    )
}
/// The single player's UUID (`offline_uuid("Player")`): the entity the
/// neutral mobs are angry at.
fn player_uuid() -> [i32; 4] {
    let u = horizon_protocol::crypto::offline_uuid("Player");
    [
        (u >> 96) as u32 as i32,
        (u >> 64) as u32 as i32,
        (u >> 32) as u32 as i32,
        u as u32 as i32,
    ]
}
pub fn mob_to_nbt(m: &Mob, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
    let base = Base {
        id: &m.kind.id,
        uuid,
        position: m.position,
        motion: m.velocity,
        rotation: [m.yaw, m.head_pitch],
        on_ground: m.on_ground,
        fall_distance: m.fall_distance,
    };
    let mut tag = base.tag(raw);
    for (key, value) in [
        (
            "Fire",
            Tag::Short(m.fire_ticks.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16),
        ),
        ("Air", Tag::Short(m.air as i16)),
        ("Health", Tag::Float(m.health.max(0.0).to_bits())),
        (
            "HurtTime",
            Tag::Short(m.hurt_time.clamp(0, i32::from(i16::MAX)) as i16),
        ),
        (
            "DeathTime",
            Tag::Short(m.death_time.clamp(0, i32::from(i16::MAX)) as i16),
        ),
        ("PersistenceRequired", Tag::Byte(i8::from(m.persistent))),
    ] {
        put(&mut tag, key, value);
    }
    // `NeutralMob.addPersistentAngerSaveData`: the anger ends at an
    // absolute game time, and the target is the entity the mob is angry at.
    if persistent_anger(m.species()) {
        put(
            &mut tag,
            "anger_end_time",
            Tag::Long(if m.extra.anger_end > 0 {
                m.extra.anger_end
            } else {
                -1
            }),
        );
        if m.extra.angry_at == Some(crate::mobs::Who::Player) {
            put(&mut tag, "angry_at", Tag::IntArray(player_uuid().to_vec()));
        }
    }
    // `Entity.saveWithoutId` writes the flag only when set.
    if m.name_visible {
        put(&mut tag, "CustomNameVisible", Tag::Byte(1));
    } else if let Tag::Compound(map) = &mut tag {
        map.remove(&"CustomNameVisible".into());
    }
    // A styled name from the save stays unless the name changed.
    if custom_name(&tag) != m.custom_name
        && let Some(name) = &m.custom_name
    {
        put(&mut tag, "CustomName", Tag::String(name.as_str().into()));
    }
    let Tag::Compound(map) = &mut tag else {
        return tag;
    };
    for (key, on) in [
        ("NoAI", m.no_ai),
        ("Silent", m.silent),
        ("NoGravity", m.no_gravity),
        ("Glowing", m.glowing),
    ] {
        if on {
            map.insert(key.into(), Tag::Byte(1));
        } else {
            map.remove(&horizon_world::nbt::JavaString::from(key));
        }
    }
    // LivingEntity.addAdditionalSaveData: `active_effects` only when there are
    // any (MobEffectInstance.CODEC; a mob's effects are never ambient, and one
    // given with `hideParticles` shows neither particles nor its icon).
    if m.extra.effects.is_empty() {
        map.remove(&horizon_world::nbt::JavaString::from("active_effects"));
    } else {
        let effects = m
            .extra
            .effects
            .iter()
            .map(|&(effect, duration, amplifier)| {
                let shown = !m.extra.hidden_effects.contains(&effect);
                let mut e = vec![
                    ("id", string(crate::brewing::namespaced(effect))),
                    ("amplifier", Tag::Byte(amplifier.clamp(0, 255) as u8 as i8)),
                    ("duration", Tag::Int(duration)),
                ];
                if !shown {
                    e.push(("show_particles", Tag::Byte(0)));
                }
                e.push(("show_icon", Tag::Byte(i8::from(shown))));
                Tag::compound(e)
            });
        map.insert("active_effects".into(), Tag::List(effects.collect()));
    }
    map.insert(
        "Invulnerable".into(),
        Tag::Byte(i8::from(m.invulnerable_flag)),
    );
    map.insert(
        "LeftHanded".into(),
        Tag::Byte(i8::from(m.extra.left_handed)),
    );
    // Keys of older formats that structure templates still carry.
    for key in [
        "HandItems",
        "ArmorItems",
        "HandDropChances",
        "ArmorDropChances",
        "Dimension",
        "HurtByTimestamp",
        "Leashed",
    ] {
        map.remove(&horizon_world::nbt::JavaString::from(key));
    }
    if m.ticks_frozen > 0 {
        map.insert("TicksFrozen".into(), Tag::Int(m.ticks_frozen));
    } else {
        map.remove(&horizon_world::nbt::JavaString::from("TicksFrozen"));
    }
    for (key, value) in [
        ("AbsorptionAmount", Tag::Float(0)),
        ("FallFlying", Tag::Byte(0)),
        ("CanPickUpLoot", Tag::Byte(i8::from(m.can_pick_up_loot()))),
        ("current_impulse_context_reset_grace_time", Tag::Int(0)),
        (
            "Brain",
            Tag::compound([("memories", Tag::Compound(Compound::new()))]),
        ),
    ] {
        map.entry(key.into()).or_insert(value);
    }
    let id = m.kind.id.as_str();
    // Only breedable animals have an age; a loaded creature that was saved without
    // one (a frog) stays without.
    let aged = raw.is_none_or(|r| r.get("Age").is_some());
    if (m.kind.category == "creature" || m.species() == crate::mobs::Species::Hoglin) && aged {
        map.insert("Age".into(), Tag::Int(m.extra.age));
        map.insert("AgeLocked".into(), Tag::Byte(i8::from(m.extra.age_locked)));
        map.insert("ForcedAge".into(), Tag::Int(m.extra.forced_age));
        for (key, value) in [
            ("Age", Tag::Int(0)),
            ("ForcedAge", Tag::Int(0)),
            ("AgeLocked", Tag::Byte(0)),
        ] {
            map.entry(key.into()).or_insert(value);
        }
    }
    // Animal.addAdditionalSaveData: the love ticks left, whatever the save had.
    if is_animal(id) {
        map.insert("InLove".into(), Tag::Int(m.extra.in_love.max(0)));
    }
    let species = m.species();
    if let Some((_, variant)) =
        crate::mobs::finalize::species_variant_component(species, m.extra.variant)
    {
        map.insert("variant".into(), string(&variant));
    }
    if species == crate::mobs::Species::Chicken {
        map.insert("EggLayTime".into(), Tag::Int(m.extra.egg_time));
    }
    let sounds = crate::mobs::finalize::sound_variants(species);
    if !sounds.is_empty() {
        let sound = m.extra.sound_variant.and_then(|i| sounds.get(i as usize));
        let sound = sound.copied().unwrap_or("classic");
        map.insert(
            "sound_variant".into(),
            string(&format!("minecraft:{sound}")),
        );
    }
    match id {
        // Fox.addAdditionalSaveData; Horizon models only the type.
        "minecraft:fox" => {
            let kind = if m.extra.variant == 1 { "snow" } else { "red" };
            map.insert("Type".into(), string(kind));
            for (key, value) in [
                ("Trusted", Tag::List(Vec::new())),
                ("Sleeping", Tag::Byte(0)),
                ("Sitting", Tag::Byte(0)),
                ("Crouching", Tag::Byte(0)),
            ] {
                map.entry(key.into()).or_insert(value);
            }
        }
        // Rabbit.addAdditionalSaveData (the legacy id; 99 is the killer bunny).
        "minecraft:rabbit" => {
            map.insert("RabbitType".into(), Tag::Int(i32::from(m.extra.variant)));
            map.entry("MoreCarrotTicks".into()).or_insert(Tag::Int(0));
        }
        "minecraft:goat" => {
            map.insert(
                "IsScreamingGoat".into(),
                Tag::Byte(i8::from(m.extra.screaming)),
            );
            let [left, right] = m.extra.lost_horns;
            map.insert("HasLeftHorn".into(), Tag::Byte(i8::from(!left)));
            map.insert("HasRightHorn".into(), Tag::Byte(i8::from(!right)));
        }
        "minecraft:creeper" => {
            for (key, value) in [("Fuse", Tag::Short(30)), ("ExplosionRadius", Tag::Byte(3))] {
                map.entry(key.into()).or_insert(value);
            }
            map.insert("powered".into(), Tag::Byte(i8::from(m.extra.charged)));
            map.insert("ignited".into(), Tag::Byte(i8::from(m.extra.flag)));
        }
        "minecraft:cat" => {
            if let Some(owner) = m.extra.owner {
                map.insert("Owner".into(), Tag::IntArray(owner.to_vec()));
            } else {
                map.remove(&"Owner".into());
            }
            map.insert("Sitting".into(), Tag::Byte(i8::from(m.extra.sitting)));
            map.insert("CollarColor".into(), Tag::Byte(m.extra.color as i8));
        }
        "minecraft:wolf" => {
            if let Some(owner) = m.extra.owner {
                map.insert("Owner".into(), Tag::IntArray(owner.to_vec()));
            } else {
                map.remove(&"Owner".into());
            }
            map.insert("Sitting".into(), Tag::Byte(i8::from(m.extra.sitting)));
            map.insert("CollarColor".into(), Tag::Byte(m.extra.color as i8));
        }
        "minecraft:panda" => {
            use crate::mobs::panda::GENES;
            for (key, gene) in [
                ("MainGene", m.extra.variant),
                ("HiddenGene", m.extra.variant2),
            ] {
                map.insert(
                    key.into(),
                    string(GENES[usize::from(gene).min(GENES.len() - 1)]),
                );
            }
        }
        "minecraft:piglin" | "minecraft:piglin_brute" | "minecraft:hoglin" => {
            // Only Piglin.addAdditionalSaveData writes CannotHunt.
            if id == "minecraft:piglin_brute" {
                map.remove(&"CannotHunt".into());
            }
            map.insert(
                "IsImmuneToZombification".into(),
                Tag::Byte(i8::from(m.extra.flag2)),
            );
            map.insert("TimeInOverworld".into(), Tag::Int(m.extra.counter2));
            if id == "minecraft:piglin" {
                // InventoryCarrier.writeInventoryToTag.
                map.insert(
                    "Inventory".into(),
                    Tag::List(
                        m.extra
                            .inventory
                            .iter()
                            .map(crate::save::item_to_nbt)
                            .collect(),
                    ),
                );
            }
        }
        "minecraft:horse"
        | "minecraft:donkey"
        | "minecraft:mule"
        | "minecraft:skeleton_horse"
        | "minecraft:zombie_horse"
        | "minecraft:llama"
        | "minecraft:trader_llama" => {
            map.insert("Tame".into(), Tag::Byte(i8::from(m.extra.tame)));
            map.insert("Temper".into(), Tag::Int(i32::from(m.extra.variant3)));
            // AbstractHorse.addAdditionalSaveData; Horizon models neither.
            for key in ["EatingHaystack", "Bred"] {
                map.entry(key.into()).or_insert(Tag::Byte(0));
            }
            if let Some(owner) = m.extra.owner {
                map.insert("Owner".into(), Tag::IntArray(owner.to_vec()));
            }
            if id == "minecraft:horse" {
                let v = i32::from(m.extra.variant) | i32::from(m.extra.variant2) << 8;
                map.insert("Variant".into(), Tag::Int(v));
            }
            let llama = crate::mobs::llama::is_llama(m.species());
            if llama {
                crate::mobs::llama::to_nbt(m, map);
            }
            // Only llamas, donkeys and mules carry chests.
            if llama || matches!(id, "minecraft:donkey" | "minecraft:mule") {
                map.insert("ChestedHorse".into(), Tag::Byte(i8::from(m.extra.chested)));
            }
            if m.extra.chested && !m.extra.items.is_empty() {
                map.insert("Items".into(), crate::save::encode_items(&m.extra.items));
            } else {
                map.remove(&"Items".into());
            }
            let mut equipment = Vec::new();
            if let Some(saddle) = m.saddle_item() {
                equipment.push(("saddle", crate::save::item_to_nbt(&saddle)));
            }
            if let Some(armor) = &m.extra.head {
                equipment.push((
                    "body",
                    Tag::compound([("id", string(armor)), ("count", Tag::Int(1))]),
                ));
            }
            if equipment.is_empty() {
                map.remove(&"equipment".into());
            } else {
                map.insert("equipment".into(), Tag::compound(equipment));
            }
            let modelled = [
                ("minecraft:max_health", f64::from(m.attrs.max_health)),
                ("minecraft:movement_speed", m.attrs.speed),
                ("minecraft:jump_strength", m.extra.vector[0]),
            ];
            // AttributeMap.pack writes the instances that exist: the ones the
            // entity was loaded with, or all three randomized by finalizeSpawn.
            let attributes = match map.get(&"attributes".into()) {
                Some(Tag::List(list)) => list
                    .iter()
                    .map(|a| {
                        let id = text(a.get("id"));
                        let value = modelled.iter().find(|(m, _)| Some(*m) == id.as_deref());
                        match (a, value) {
                            (Tag::Compound(c), Some((_, v))) => {
                                let mut c = c.clone();
                                c.insert("base".into(), Tag::Double(v.to_bits()));
                                Tag::Compound(c)
                            }
                            _ => a.clone(),
                        }
                    })
                    .collect(),
                _ => modelled
                    .iter()
                    .map(|(id, v)| {
                        Tag::compound([("id", string(id)), ("base", Tag::Double(v.to_bits()))])
                    })
                    .collect(),
            };
            map.insert("attributes".into(), Tag::List(attributes));
        }
        "minecraft:pig" | "minecraft:strider" => {
            let mut equipment = match map.get(&"equipment".into()) {
                Some(Tag::Compound(c)) => c.clone(),
                _ => Compound::new(),
            };
            if let Some(saddle) = m.saddle_item() {
                equipment.insert("saddle".into(), crate::save::item_to_nbt(&saddle));
            } else {
                equipment.remove(&"saddle".into());
            }
            map.insert("equipment".into(), Tag::Compound(equipment));
        }
        "minecraft:bat" => {
            map.insert("BatFlags".into(), Tag::Byte(i8::from(m.extra.flag)));
        }
        "minecraft:turtle" => {
            map.insert("has_egg".into(), Tag::Byte(i8::from(m.extra.flag)));
            if let Some(home) = m.extra.home {
                map.insert("home_pos".into(), Tag::IntArray(home.to_vec()));
            }
        }
        "minecraft:creaking" => {
            // home_pos: the heart it is bound to.
            if let Some(home) = m.extra.home {
                map.insert("home_pos".into(), Tag::IntArray(home.to_vec()));
            } else {
                map.remove(&"home_pos".into());
            }
        }
        "minecraft:zombie_villager" => {
            map.insert("VillagerData".into(), villager_data(m));
            map.insert("VillagerDataFinalized".into(), Tag::Byte(1));
            let xp = m.extra.trade.as_deref().map_or(0, |t| t.xp);
            map.insert("Xp".into(), Tag::Int(xp));
            put_offers(map, m);
            crate::mobs::zombie_villager::write_nbt(m, map);
        }
        "minecraft:villager" => {
            map.insert("VillagerData".into(), villager_data(m));
            let t = m.extra.trade.as_deref().cloned().unwrap_or_default();
            map.insert("Xp".into(), Tag::Int(t.xp));
            map.insert("LastRestock".into(), Tag::Long(t.last_restock));
            map.insert("RestocksToday".into(), Tag::Int(t.restocks_today));
            put_offers(map, m);
            put_memories(map, m);
            crate::mobs::villager::write_nbt(m, map);
        }
        "minecraft:wandering_trader" => put_offers(map, m),
        "minecraft:axolotl" => {
            write_axolotl_memories(m, map);
            map.insert(
                "Variant".into(),
                Tag::Int(i32::from(m.extra.variant.min(4))),
            );
        }
        "minecraft:parrot" => {
            map.insert(
                "Variant".into(),
                Tag::Int(i32::from(m.extra.variant.min(4))),
            );
            if let Some(owner) = m.extra.owner {
                map.insert("Owner".into(), Tag::IntArray(owner.to_vec()));
            } else {
                map.remove(&"Owner".into());
            }
            map.insert("Sitting".into(), Tag::Byte(i8::from(m.extra.sitting)));
        }
        "minecraft:salmon" => {
            let size = SALMON_SIZES[usize::from(m.extra.variant).min(2)];
            map.insert("type".into(), string(size));
        }
        // TropicalFish.addAdditionalSaveData: the packed variant.
        "minecraft:tropical_fish" => {
            let pattern = m.extra.variant.min(11);
            let packed = i32::from(pattern >= 6)
                | i32::from(pattern % 6) << 8
                | i32::from(m.extra.variant2 & 15) << 16
                | i32::from(m.extra.variant3 & 15) << 24;
            map.insert("Variant".into(), Tag::Int(packed));
        }
        "minecraft:allay" => {
            // InventoryCarrier: the fetched stack; the liked player memory.
            let mut inv = Vec::new();
            if let (Some(item), n) = (&m.extra.main_hand, m.extra.ints[1])
                && n > 0
            {
                inv.push(Tag::compound([
                    ("id", Tag::String(item.as_ref().into())),
                    ("count", Tag::Int(n)),
                ]));
            }
            map.insert("Inventory".into(), Tag::List(inv));
            map.insert(
                "DuplicationCooldown".into(),
                Tag::Long(i64::from(m.extra.egg_time)),
            );
            if m.extra.tame {
                map.insert(
                    "Brain".into(),
                    Tag::compound([(
                        "memories",
                        Tag::compound([(
                            "minecraft:liked_player",
                            Tag::compound([(
                                "value",
                                Tag::IntArray(m.extra.owner.unwrap_or([0, 0, 0, 1]).to_vec()),
                            )]),
                        )]),
                    )]),
                );
            }
        }
        "minecraft:camel" | "minecraft:camel_husk" => {
            // LastPoseTick: negative while sitting.
            let sitting = crate::mobs::camel::sitting(m);
            map.insert(
                "LastPoseTick".into(),
                Tag::Long(if sitting { -1 } else { 0 }),
            );
            if let Some(saddle) = m.saddle_item() {
                map.insert(
                    "equipment".into(),
                    Tag::compound([("saddle", crate::save::item_to_nbt(&saddle))]),
                );
            } else {
                map.remove(&"equipment".into());
            }
        }
        "minecraft:nautilus" | "minecraft:zombie_nautilus" => {
            crate::mobs::nautilus::to_nbt(m, map);
        }
        "minecraft:sulfur_cube" => crate::mobs::sulfur_cube::to_nbt(m, map),
        "minecraft:bee" => crate::mobs::bee::to_nbt(m, map),
        "minecraft:sniffer" => crate::mobs::sniffer::to_nbt(m, map),
        "minecraft:wither" => {
            map.insert("Invul".into(), Tag::Int(m.extra.counter));
        }
        // IronGolem.addAdditionalSaveData.
        "minecraft:iron_golem" => {
            map.insert(
                "PlayerCreated".into(),
                Tag::Byte(i8::from(m.extra.player_created)),
            );
        }
        "minecraft:warden" => {
            if let Some(st) = &m.extra.warden {
                map.insert("listener".into(), st.vibration.to_nbt());
                map.insert("anger".into(), Tag::compound(Vec::<(&str, Tag)>::new()));
            }
        }
        // Armadillo.addAdditionalSaveData: the time left to the next scute.
        "minecraft:armadillo" => {
            map.insert("scute_time".into(), Tag::Int(m.extra.egg_time));
        }
        "minecraft:ravager" => {
            map.insert("AttackTick".into(), Tag::Int(m.extra.counter));
            map.insert("StunTick".into(), Tag::Int(m.extra.counter2));
            map.insert("RoarTick".into(), Tag::Int(m.extra.anim_ticks));
        }
        "minecraft:vex" => {
            if let Some(h) = m.extra.home {
                map.insert("bound_pos".into(), Tag::IntArray(h.to_vec()));
            }
            if m.extra.egg_time > 0 {
                map.insert("life_ticks".into(), Tag::Int(m.extra.egg_time));
            }
        }
        "minecraft:endermite" => {
            map.insert("Lifetime".into(), Tag::Int(m.extra.counter));
        }
        "minecraft:shulker" => {
            map.insert("AttachFace".into(), Tag::Byte(m.extra.variant3 as i8));
            map.insert("Peek".into(), Tag::Byte(m.extra.counter as i8));
            map.insert("Color".into(), Tag::Byte(m.extra.color as i8));
        }
        "minecraft:slime" | "minecraft:magma_cube" => {
            map.insert("Size".into(), Tag::Int(m.extra.size.max(1) - 1));
            map.insert("wasOnGround".into(), Tag::Byte(i8::from(m.extra.flag2)));
        }
        // Skeletons are spawned holding a bow (finalizeSpawn); a mob loaded
        // from NBT keeps only its saved equipment.
        "minecraft:skeleton" | "minecraft:stray" | "minecraft:bogged"
            if raw.is_none() && m.extra.main_hand.as_deref() == Some("minecraft:bow") =>
        {
            map.entry("equipment".into()).or_insert_with(|| {
                Tag::compound([(
                    "mainhand",
                    Tag::compound([("id", string("minecraft:bow")), ("count", Tag::Int(1))]),
                )])
            });
        }
        "minecraft:zombie" | "minecraft:husk" | "minecraft:drowned" => {
            for (key, value) in [
                ("DrownedConversionTime", Tag::Int(-1)),
                ("InWaterTime", Tag::Int(-1)),
            ] {
                map.entry(key.into()).or_insert(value);
            }
        }
        // Sheep.addAdditionalSaveData: its wool as it is now.
        "minecraft:sheep" => {
            map.insert("Color".into(), Tag::Byte((m.extra.color & 15) as i8));
            map.insert("Sheared".into(), Tag::Byte(i8::from(m.extra.sheared)));
        }
        // CopperGolem.addAdditionalSaveData.
        "minecraft:copper_golem" => {
            map.insert(
                "next_weather_age".into(),
                Tag::Long(m.extra.vector[0] as i64),
            );
            map.insert(
                "weather_state".into(),
                string(crate::mobs::copper_golem::WEATHER[m.extra.variant.min(3) as usize]),
            );
        }
        _ => {}
    }
    // Zombie.addAdditionalSaveData: CanBreakDoors.
    if crate::mobs::doors::is_zombie(m.species()) {
        map.insert(
            "CanBreakDoors".into(),
            Tag::Byte(i8::from(m.extra.can_break_doors)),
        );
    }
    if matches!(
        id,
        "minecraft:zombie"
            | "minecraft:husk"
            | "minecraft:drowned"
            | "minecraft:zombie_villager"
            | "minecraft:zombified_piglin"
            | "minecraft:piglin"
            | "minecraft:zoglin"
    ) {
        map.insert("IsBaby".into(), Tag::Byte(i8::from(m.is_baby())));
    }
    if let Some(a) = m.extra.attributes.as_deref() {
        put_attributes(map, m, a);
    }
    crate::mobs::raider::write_nbt(m, map);
    crate::mobs::riders::write_nbt(m, map);
    // LivingEntity equipment: what the mob holds now.
    if !crate::mobs::equine::is_equine(m.species()) {
        let mut equipment = match map.get(&"equipment".into()) {
            Some(Tag::Compound(c)) => c.clone(),
            _ => horizon_world::nbt::Compound::new(),
        };
        for (index, (slot, item)) in [
            ("mainhand", &m.extra.main_hand),
            ("offhand", &m.extra.off_hand),
            (
                // A happy ghast wears its harness in the body slot (and a
                // nautilus its armour, a sulfur cube its swallowed block).
                if crate::mobs::species::body_slot(m.species()) {
                    "body"
                } else {
                    "head"
                },
                &m.extra.head,
            ),
            ("chest", &m.extra.chest),
            ("legs", &m.extra.legs),
            ("feet", &m.extra.feet),
        ]
        .into_iter()
        .enumerate()
        {
            // Enchanted or trimmed gear is saved with its components.
            if let (Some(id), Some(g)) = (item, crate::mobs::gear::gear(m))
                && (!g.slots[index].is_empty() || g.trims[index].is_some())
            {
                let mut stack = crate::items::ItemStack::new(id, 1);
                stack.data_mut().enchantments = g.slots[index].clone();
                stack.data_mut().trim = g.trims[index].clone();
                equipment.insert(slot.into(), crate::save::item_to_nbt(&stack));
                continue;
            }
            // A wolf's armor keeps the durability it lost.
            if slot == "body"
                && m.species() == crate::mobs::Species::Wolf
                && let Some(id) = item
            {
                let mut stack = crate::items::ItemStack::new(id, 1);
                stack.damage = m.extra.body_damage.max(0) as u32;
                stack.extra = m.extra.body_extra.clone();
                equipment.insert(slot.into(), crate::save::item_to_nbt(&stack));
                continue;
            }
            match item {
                Some(id) => {
                    let keep = matches!(
                        equipment.get(&slot.into()).and_then(|e| e.get("id")),
                        Some(Tag::String(s)) if s.to_string_lossy() == **id
                    );
                    if slot == "head"
                        && &**id == crate::mobs::raider::BANNER
                        && crate::mobs::raider::is_raider(m.species())
                    {
                        // The captain's ominous banner keeps its patterns.
                        let has_patterns = equipment
                            .get(&slot.into())
                            .and_then(|e| e.get("components"))
                            .is_some();
                        if !has_patterns {
                            equipment.insert(
                                slot.into(),
                                crate::save::item_to_nbt(&crate::mobs::raider::ominous_banner()),
                            );
                        }
                    } else if !keep {
                        equipment.insert(
                            slot.into(),
                            Tag::compound([
                                ("id", Tag::String(id.as_ref().into())),
                                ("count", Tag::Int(1)),
                            ]),
                        );
                    }
                }
                None => {
                    equipment.remove(&slot.into());
                }
            }
        }
        // Vanilla saves `equipment` only when something is equipped.
        if equipment.is_empty() {
            map.remove(&"equipment".into());
        } else {
            map.insert("equipment".into(), Tag::Compound(equipment));
        }
    }
    tag
}
/// `AttributeMap.pack` for the attributes with a base or permanent modifier
/// finalizeSpawn set, over the saved list.
fn put_attributes(map: &mut Compound, m: &Mob, a: &crate::mobs::finalize::Attributes) {
    use crate::mobs::finalize::{self, Attr, Op};
    let mut list = match map.get(&"attributes".into()) {
        Some(Tag::List(l)) => l.clone(),
        _ => Vec::new(),
    };
    for attr in Attr::ALL {
        let modifiers: Vec<Tag> = a
            .modifiers
            .iter()
            .filter(|x| x.attr == attr)
            .map(|x| {
                let op = match x.op {
                    Op::AddValue => "add_value",
                    Op::AddMultipliedBase => "add_multiplied_base",
                    Op::AddMultipliedTotal => "add_multiplied_total",
                };
                Tag::compound([
                    ("id", string(x.id)),
                    ("amount", Tag::Double(x.amount.to_bits())),
                    ("operation", string(op)),
                ])
            })
            .collect();
        if modifiers.is_empty() && !a.bases.iter().any(|(b, _)| *b == attr) {
            continue;
        }
        let id = format!("minecraft:{}", attr.name());
        let mut entry = vec![
            ("id", string(&id)),
            ("base", Tag::Double(finalize::base_value(m, attr).to_bits())),
        ];
        if !modifiers.is_empty() {
            entry.push(("modifiers", Tag::List(modifiers)));
        }
        let entry = Tag::compound(entry);
        match list
            .iter_mut()
            .find(|t| text(t.get("id")).as_deref() == Some(id.as_str()))
        {
            Some(t) => *t = entry,
            None => list.push(entry),
        }
    }
    map.insert("attributes".into(), Tag::List(list));
}
/// `Brain.Packed`: the memories finalizeSpawn set, over the saved ones.
pub fn put_brain(map: &mut Compound, m: &Mob, dimension: &str) {
    if m.extra.memories.is_empty() {
        return;
    }
    let mut memories = match map.get(&"Brain".into()).and_then(|b| b.get("memories")) {
        Some(Tag::Compound(c)) => c.clone(),
        _ => Compound::new(),
    };
    for &(name, value, ttl) in &m.extra.memories {
        let value = match name {
            "hunted_recently" => Tag::Byte(value as i8),
            "home" => match m.extra.home {
                Some(p) => Tag::compound([
                    ("dimension", string(dimension)),
                    ("pos", Tag::IntArray(p.to_vec())),
                ]),
                None => continue,
            },
            _ => Tag::Int(value as i32),
        };
        let mut memory = vec![("value", value)];
        if ttl != i64::MAX {
            memory.push(("ttl", Tag::Long(ttl)));
        }
        memories.insert(
            format!("minecraft:{name}").as_str().into(),
            Tag::compound(memory),
        );
    }
    map.insert(
        "Brain".into(),
        Tag::compound([("memories", Tag::Compound(memories))]),
    );
}
/// `CustomName`: a text component saved as a string or a `{text: ..}` compound.
fn custom_name(tag: &Tag) -> Option<String> {
    match tag.get("CustomName")? {
        Tag::String(s) => Some(s.to_string_lossy()),
        t @ Tag::Compound(_) => match t.get("text") {
            Some(Tag::String(s)) => Some(s.to_string_lossy()),
            _ => None,
        },
        _ => None,
    }
}
pub use horizon_client_core::component::name_text;
/// Mob.finalizeSpawn's follow range bonus in the saved attributes.
pub(crate) fn spawn_bonus(tag: &Tag) -> Option<f64> {
    let Some(Tag::List(attrs)) = tag.get("attributes") else {
        return None;
    };
    let range = attrs
        .iter()
        .find(|a| text(a.get("id")).as_deref() == Some("minecraft:follow_range"))?;
    let Some(Tag::List(modifiers)) = range.get("modifiers") else {
        return None;
    };
    let bonus = modifiers
        .iter()
        .find(|x| text(x.get("id")).as_deref() == Some("minecraft:random_spawn_bonus"))?;
    float(bonus.get("amount"))
}
pub fn mob_from_nbt(tag: &Tag, types: &HashMap<String, Arc<MobType>>, id: u32) -> Option<Mob> {
    let kind = types.get(&self::kind(tag)?)?.clone();
    let position = position(tag)?;
    let [yaw, pitch] = rotation(tag);
    let mut m = Mob::new(id, kind, position, yaw);
    m.velocity = motion(tag);
    m.head_pitch = pitch;
    m.on_ground = flag(tag, "OnGround");
    m.fall_distance = float(tag.get("fall_distance")).unwrap_or(0.0);
    // LivingEntity.readAdditionalSaveData: the saved attribute bases, then
    // the health, the max health when there is none.
    if let Some(Tag::List(attrs)) = tag.get("attributes") {
        for a in attrs {
            let (Some(id), Some(Tag::Double(bits))) = (text(a.get("id")), a.get("base")) else {
                continue;
            };
            let v = f64::from_bits(*bits);
            // A mount's rolled health and speed stay attribute bases, which
            // the server syncs (its hearts over the rider's hotbar).
            let equine = crate::mobs::equine::is_equine(m.species());
            match id.as_str() {
                "minecraft:max_health" if equine => crate::mobs::finalize::set_base(
                    &mut m,
                    crate::mobs::finalize::Attr::MaxHealth,
                    v,
                ),
                "minecraft:movement_speed" if equine => crate::mobs::finalize::set_base(
                    &mut m,
                    crate::mobs::finalize::Attr::MovementSpeed,
                    v,
                ),
                "minecraft:max_health" => m.attrs.max_health = v as f32,
                "minecraft:movement_speed" => m.attrs.speed = v,
                "minecraft:attack_damage" => m.attrs.attack_damage = v as f32,
                _ => {}
            }
        }
    }
    if let Some(bonus) = spawn_bonus(tag) {
        use crate::mobs::finalize::{Attr, Op, add_modifier};
        let id = "minecraft:random_spawn_bonus";
        add_modifier(&mut m, Attr::FollowRange, id, bonus, Op::AddMultipliedBase);
    }
    m.health = float(tag.get("Health")).map_or(m.attrs.max_health, |h| h as f32);
    // LivingEntity.setHealth clamps to the max health: a horse or llama is made
    // with the type's and saved before it drops to the one it rolled. (Only
    // the base is read here, so the clamp would cut a leader zombie's bonus.)
    if crate::mobs::equine::is_equine(m.species()) {
        m.health = m.health.min(m.attrs.max_health);
    }
    m.hurt_time = number(tag.get("HurtTime")).unwrap_or(0) as i32;
    // What it holds (equipment.mainhand / offhand), when saved.
    if let Some(eq) = tag.get("equipment") {
        m.extra.main_hand = eq
            .get("mainhand")
            .and_then(|i| text(i.get("id")))
            .map(Into::into);
        m.extra.off_hand = eq
            .get("offhand")
            .and_then(|i| text(i.get("id")))
            .map(Into::into);
        if !crate::mobs::equine::is_equine(m.species()) {
            let slot = |key| eq.get(key).and_then(|i| text(i.get("id"))).map(Into::into);
            m.extra.head = slot(if crate::mobs::species::body_slot(m.species()) {
                "body"
            } else {
                "head"
            });
            m.extra.chest = slot("chest");
            m.extra.legs = slot("legs");
            m.extra.feet = slot("feet");
        }
        // Their enchantments and trims.
        for (index, key) in crate::mobs::gear::KEYS.iter().enumerate() {
            if let Some(stack) = eq.get(key).and_then(crate::save::item_from_nbt) {
                crate::mobs::gear::set(&mut m, index, stack.enchantments().to_vec());
                let trim = stack.data().and_then(|d| d.trim.clone());
                crate::mobs::gear::set_trim(&mut m, index, trim);
            }
        }
    } else {
        // Registration hands out default weapons (finalizeSpawn does in
        // vanilla); a loaded mob holds only what its file lists.
        m.extra.main_hand = None;
    }
    m.death_time = number(tag.get("DeathTime")).unwrap_or(0) as i32;
    // Mob.readAdditionalSaveData overrides what the constructor set (copper
    // golems and elder guardians start persistent).
    m.persistent = flag(tag, "PersistenceRequired");
    // `NeutralMob.readPersistentAngerSaveData`: the anger end time and the
    // target come back with the mob.
    if persistent_anger(m.species()) {
        m.extra.anger_end = number(tag.get("anger_end_time")).unwrap_or(-1);
        if tag
            .get("angry_at")
            .is_some_and(|t| uuid_from_tag(Some(t)) == Some(player_uuid()))
        {
            m.extra.angry_at = Some(crate::mobs::Who::Player);
        }
    }
    m.extra.pick_up_loot = flag(tag, "CanPickUpLoot");
    m.custom_name = custom_name(tag);
    m.custom_name_text = tag.get("CustomName").map(name_text);
    m.extra.left_handed = flag(tag, "LeftHanded");
    if m.kind.category == "creature" || m.species() == crate::mobs::Species::Hoglin {
        m.extra.age = number(tag.get("Age")).unwrap_or(0) as i32;
        m.extra.forced_age = number(tag.get("ForcedAge")).unwrap_or(0) as i32;
        m.extra.age_locked = flag(tag, "AgeLocked");
    }
    if is_animal(&m.kind.id) {
        m.extra.in_love = number(tag.get("InLove")).unwrap_or(0) as i32;
    }
    m.name_visible = flag(tag, "CustomNameVisible");
    m.no_ai = flag(tag, "NoAI");
    m.no_gravity = flag(tag, "NoGravity");
    m.glowing = flag(tag, "Glowing");
    m.ticks_frozen = number(tag.get("TicksFrozen")).unwrap_or(0) as i32;
    // LivingEntity.readAdditionalSaveData: the saved `active_effects`, the last
    // entry of an effect winning.
    if let Some(Tag::List(effects)) = tag.get("active_effects") {
        for e in effects {
            let Some(def) = text(e.get("id")).and_then(|id| crate::brewing::effect(&id)) else {
                continue;
            };
            let effect = crate::brewing::namespaced(def.id);
            let duration = number(e.get("duration")).unwrap_or(0) as i32;
            let amplifier = number(e.get("amplifier")).unwrap_or(0) as i32;
            m.extra.effects.retain(|x| x.0 != effect);
            m.extra.effects.push((effect, duration, amplifier));
            // Details.CODEC: `show_particles` defaults to true.
            m.extra.hidden_effects.retain(|h| *h != effect);
            if number(e.get("show_particles")) == Some(0) {
                m.extra.hidden_effects.push(effect);
            }
        }
    }
    m.silent = flag(tag, "Silent");
    m.invulnerable_flag = flag(tag, "Invulnerable");
    if m.species() == crate::mobs::Species::Bat {
        // Bat.readAdditionalSaveData: BatFlags bit 0, resting.
        m.extra.flag = number(tag.get("BatFlags")).unwrap_or(0) & 1 != 0;
    }
    if m.species() == crate::mobs::Species::Creeper {
        m.extra.charged = flag(tag, "powered");
        // Creeper.readAdditionalSaveData: `ignited` lights the fuse.
        m.extra.flag = flag(tag, "ignited");
    }
    if matches!(
        m.species(),
        crate::mobs::Species::Piglin
            | crate::mobs::Species::PiglinBrute
            | crate::mobs::Species::Hoglin
    ) {
        m.extra.flag2 = flag(tag, "IsImmuneToZombification");
        m.extra.counter2 = number(tag.get("TimeInOverworld")).unwrap_or(0) as i32;
        if m.species() == crate::mobs::Species::Piglin
            && let Some(Tag::List(items)) = tag.get("Inventory")
        {
            // InventoryCarrier.readInventoryFromTag (eight slots).
            m.extra.inventory = items
                .iter()
                .filter_map(crate::save::item_from_nbt)
                .take(8)
                .collect();
        }
    }
    if matches!(
        m.species(),
        crate::mobs::Species::Pig | crate::mobs::Species::Strider
    ) {
        let saddle = tag
            .get("equipment")
            .and_then(|e| e.get("saddle"))
            .and_then(crate::save::item_from_nbt);
        if saddle.is_some() || flag(tag, "Saddle") {
            m.set_saddle(Some(saddle.unwrap_or_else(|| {
                crate::items::ItemStack::new("minecraft:saddle", 1)
            })));
        }
    }
    if crate::mobs::equine::is_equine(m.species()) {
        m.extra.tame = flag(tag, "Tame");
        m.extra.variant3 = number(tag.get("Temper")).unwrap_or(0).clamp(0, 100) as u8;
        m.extra.owner = crate::save::uuid_from_tag(tag.get("Owner"));
        let v = number(tag.get("Variant")).unwrap_or(0) as i32;
        m.extra.variant = (v & 0xFF).clamp(0, 6) as u8;
        m.extra.variant2 = ((v >> 8) & 0xFF).clamp(0, 4) as u8;
        if crate::mobs::llama::is_llama(m.species()) {
            crate::mobs::llama::from_nbt(&mut m, tag);
        }
        m.extra.chested = flag(tag, "ChestedHorse");
        if m.extra.chested {
            m.extra.items = crate::save::decode_items(tag.get("Items"), 15);
        }
        let eq = tag.get("equipment");
        m.set_saddle(
            eq.and_then(|e| e.get("saddle"))
                .and_then(crate::save::item_from_nbt),
        );
        m.extra.head = eq
            .and_then(|e| e.get("body"))
            .and_then(|b| text(b.get("id")))
            .map(Into::into);
        if let Some(Tag::List(attrs)) = tag.get("attributes") {
            for a in attrs {
                let (Some(id), Some(Tag::Double(bits))) = (text(a.get("id")), a.get("base")) else {
                    continue;
                };
                if id == "minecraft:jump_strength" {
                    m.extra.vector[0] = f64::from_bits(*bits);
                }
            }
        }
        if m.extra.vector[0] <= 0.0 {
            m.extra.vector[0] = 0.7;
        }
    }
    if crate::mobs::finalize::species_variant_component(m.species(), 0).is_some()
        && let Some(v) = text(tag.get("variant"))
    {
        m.extra.variant = crate::mobs::finalize::variant_index(m.species(), &v);
    }
    if m.species() == crate::mobs::Species::Chicken
        && let Some(t) = number(tag.get("EggLayTime"))
    {
        m.extra.egg_time = t as i32;
    }
    if let Some(v) = text(tag.get("sound_variant")) {
        let v = v.trim_start_matches("minecraft:");
        let sounds = crate::mobs::finalize::sound_variants(m.species());
        m.extra.sound_variant = sounds.iter().position(|n| *n == v).map(|i| i as u8);
    }
    match m.species() {
        crate::mobs::Species::Fox => {
            m.extra.variant = u8::from(text(tag.get("Type")).as_deref() == Some("snow"));
        }
        crate::mobs::Species::Rabbit => {
            // Rabbit.Variant.byId: an unknown id is brown.
            m.extra.variant = match number(tag.get("RabbitType")).unwrap_or(0) {
                v @ (0..=5 | 99) => v as u8,
                _ => 0,
            };
        }
        crate::mobs::Species::Goat => {
            m.extra.screaming = flag(tag, "IsScreamingGoat");
            let horn = |key| number(tag.get(key)).is_some_and(|v| v == 0);
            m.extra.lost_horns = [horn("HasLeftHorn"), horn("HasRightHorn")];
        }
        _ => {}
    }
    if m.species() == crate::mobs::Species::Cat {
        m.extra.owner = crate::save::uuid_from_tag(tag.get("Owner"));
        m.extra.tame = m.extra.owner.is_some();
        m.extra.sitting = flag(tag, "Sitting");
        m.extra.color =
            number(tag.get("CollarColor")).map_or(crate::mobs::tame::RED, |c| (c as u8) & 15);
    }
    if m.species() == crate::mobs::Species::Panda {
        let gene = |key| text(tag.get(key)).map_or(0, |n| crate::mobs::panda::gene_from_name(&n));
        m.extra.variant = gene("MainGene");
        m.extra.variant2 = gene("HiddenGene");
    }
    if m.species() == crate::mobs::Species::Parrot {
        m.extra.owner = crate::save::uuid_from_tag(tag.get("Owner"));
        m.extra.tame = m.extra.owner.is_some();
        m.extra.sitting = flag(tag, "Sitting");
    }
    if m.species() == crate::mobs::Species::Wolf {
        let health = m.health;
        m.extra.owner = crate::save::uuid_from_tag(tag.get("Owner"));
        let tame = m.extra.owner.is_some();
        crate::mobs::tame::set_tame(&mut m, tame);
        m.health = health;
        m.extra.sitting = flag(tag, "Sitting");
        let armor = tag
            .get("equipment")
            .and_then(|e| e.get("body"))
            .and_then(crate::save::item_from_nbt);
        m.extra.body_damage = armor.as_ref().map_or(0, |s| s.damage as i32);
        m.extra.body_extra = armor.and_then(|s| s.extra);
        m.extra.color =
            number(tag.get("CollarColor")).map_or(crate::mobs::tame::RED, |c| (c as u8) & 15);
    }
    if matches!(
        m.species(),
        crate::mobs::Species::Villager | crate::mobs::Species::WanderingTrader
    ) {
        read_villager(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::ZombieVillager {
        read_villager(&mut m, tag);
        let xp = number(tag.get("Xp")).unwrap_or(0) as i32;
        if xp != 0 {
            m.extra.trade.get_or_insert_with(Default::default).xp = xp;
        }
        crate::mobs::zombie_villager::read_nbt(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::Axolotl {
        restore_axolotl_memories(&mut m, tag);
    }
    read_variant(&mut m, tag);
    m.fire_ticks = number(tag.get("Fire")).unwrap_or(0) as i32;
    m.air = number(tag.get("Air")).unwrap_or(300) as i32;
    if m.species() == crate::mobs::Species::Allay {
        if let Some(Tag::List(inv)) = tag.get("Inventory")
            && let Some(first) = inv.first()
        {
            m.extra.ints[1] = number(first.get("count")).unwrap_or(1) as i32;
        }
        m.extra.egg_time = number(tag.get("DuplicationCooldown")).unwrap_or(0) as i32;
        let liked = tag
            .get("Brain")
            .and_then(|b| b.get("memories"))
            .and_then(|b| b.get("minecraft:liked_player"));
        m.extra.tame = liked.is_some();
        m.extra.owner = liked
            .and_then(|l| l.get("value"))
            .and_then(|v| crate::save::uuid_from_tag(Some(v)));
    }
    if crate::mobs::camel::is_camel(m.species()) {
        let sitting = matches!(tag.get("LastPoseTick"), Some(Tag::Long(v)) if *v < 0);
        crate::mobs::camel::stand_up_instantly(&mut m);
        if sitting {
            m.extra.egg_time = -1 - 400;
        }
        // Camels are always tame (`Camel.register` in Horizon).
        m.extra.tame = true;
    }
    if crate::mobs::nautilus::is_nautilus(m.species()) {
        crate::mobs::nautilus::from_nbt(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::SulfurCube {
        crate::mobs::sulfur_cube::from_nbt(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::Bee {
        crate::mobs::bee::from_nbt(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::Creaking
        && let Some(Tag::IntArray(home)) = tag.get("home_pos")
        && home.len() == 3
    {
        crate::mobs::creaking::set_transient(&mut m, [home[0], home[1], home[2]]);
    }
    if m.species() == crate::mobs::Species::Turtle {
        // Turtle.readAdditionalSaveData: `home_pos` (else where it stands) and `has_egg`.
        m.extra.flag = flag(tag, "has_egg");
        m.extra.home = Some(match tag.get("home_pos") {
            Some(Tag::IntArray(home)) if home.len() == 3 => [home[0], home[1], home[2]],
            _ => [
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            ],
        });
    }
    if m.species() == crate::mobs::Species::Wither {
        m.extra.counter = number(tag.get("Invul")).unwrap_or(0) as i32;
    }
    if m.species() == crate::mobs::Species::Sniffer {
        crate::mobs::sniffer::from_nbt(&mut m, tag);
    }
    if m.species() == crate::mobs::Species::CopperGolem {
        // CopperGolem.readAdditionalSaveData: -1 (unset) when missing.
        m.extra.vector[0] = match tag.get("next_weather_age") {
            Some(Tag::Long(v)) => *v as f64,
            _ => -1.0,
        };
        let weather = match tag.get("weather_state") {
            Some(Tag::String(s)) => s.to_string_lossy(),
            _ => String::new(),
        };
        m.extra.variant = crate::mobs::copper_golem::WEATHER
            .iter()
            .position(|w| *w == weather)
            .unwrap_or(0) as u8;
    }
    m.extra.leash_pending = crate::mobs::leash::pending_from_nbt(tag.get("leash"));
    if m.species() == crate::mobs::Species::Warden {
        // The brain starts afresh: a minute before it digs back down.
        crate::mobs::warden::finalize(&mut m, false, &mut Vec::new());
        if let (Some(st), Some(listener)) = (m.extra.warden.as_mut(), tag.get("listener")) {
            st.vibration = crate::sculk::Data::from_nbt(listener);
        }
    }
    // Armadillo.readAdditionalSaveData: keeps the constructor's time when absent.
    if m.species() == crate::mobs::Species::Armadillo
        && let Some(t) = number(tag.get("scute_time"))
    {
        m.extra.egg_time = t as i32;
    }
    if m.species() == crate::mobs::Species::Ravager {
        m.extra.counter = number(tag.get("AttackTick")).unwrap_or(0) as i32;
        m.extra.counter2 = number(tag.get("StunTick")).unwrap_or(0) as i32;
        m.extra.anim_ticks = number(tag.get("RoarTick")).unwrap_or(0) as i32;
    }
    if m.species() == crate::mobs::Species::Endermite {
        m.extra.counter = number(tag.get("Lifetime")).unwrap_or(0) as i32;
    }
    if m.species() == crate::mobs::Species::Vex {
        if let Some(Tag::IntArray(v)) = tag.get("bound_pos")
            && v.len() == 3
        {
            m.extra.home = Some([v[0], v[1], v[2]]);
        }
        m.extra.egg_time = number(tag.get("life_ticks")).map_or(-1, |v| v as i32);
    }
    // Sheep.readAdditionalSaveData: white and unsheared when absent.
    if m.species() == crate::mobs::Species::Sheep {
        m.extra.color = number(tag.get("Color")).map_or(0, |c| (c as u8) & 15);
        m.extra.sheared = flag(tag, "Sheared");
    }
    // IronGolem.readAdditionalSaveData: not player created when absent.
    if m.species() == crate::mobs::Species::IronGolem {
        m.extra.player_created = flag(tag, "PlayerCreated");
    }
    if m.species() == crate::mobs::Species::Shulker {
        m.extra.variant3 = number(tag.get("AttachFace")).unwrap_or(0).clamp(0, 5) as u8;
        m.extra.counter = number(tag.get("Peek")).unwrap_or(0).clamp(0, 100) as i32;
        m.extra.color = number(tag.get("Color")).map_or(16, |c| c.clamp(0, 16) as u8);
        m.anim.v[0] = m.extra.counter as f32 * 0.01;
        m.anim.vo[0] = m.anim.v[0];
        // Shulker.setPos: square on its block.
        m.position = horizon_core::geometry::Vec3::new(
            m.position.x.floor() + 0.5,
            (m.position.y + 0.5).floor(),
            m.position.z.floor() + 0.5,
        );
        m.previous = m.position;
    }
    if matches!(
        m.species(),
        crate::mobs::Species::Slime | crate::mobs::Species::MagmaCube
    ) {
        // AbstractCubeMob.readAdditionalSaveData: the size's attributes, then
        // the saved health, the max health when there is none.
        crate::mobs::cube::set_size(&mut m, number(tag.get("Size")).unwrap_or(0) as i32 + 1);
        if let Some(h) = float(tag.get("Health")) {
            m.health = h as f32;
        }
        m.extra.flag2 = flag(tag, "wasOnGround");
    }
    // Zombie, Piglin and Zoglin.readAdditionalSaveData: IsBaby (the baby
    // zombies of trial spawners).
    if !crate::mobs::finalize::ageable(m.species()) && flag(tag, "IsBaby") {
        crate::mobs::finalize::set_baby(&mut m, true);
    }
    // Zombie.readAdditionalSaveData: setCanBreakDoors.
    if crate::mobs::doors::is_zombie(m.species()) {
        crate::mobs::doors::set_can_break_doors(&mut m, flag(tag, "CanBreakDoors"));
    }
    crate::mobs::raider::read_nbt(&mut m, tag);
    crate::mobs::riders::read_nbt(&mut m, tag);
    // A mob saved mid-death finishes dying; one saved dead is dropped.
    if m.health <= 0.0 && m.death_time == 0 {
        return None;
    }
    // DeathTime only counts while dead.
    if m.health > 0.0 {
        m.death_time = 0;
    }
    Some(m)
}

/// `ItemEntity`: Health, Age, PickupDelay, Item, Thrower.
pub fn item_entity_to_nbt(e: &ItemEntity, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
    let base = Base {
        id: "minecraft:item",
        uuid,
        position: e.position,
        motion: e.velocity,
        rotation: [0.0, 0.0],
        on_ground: e.on_ground,
        fall_distance: 0.0,
    };
    let mut tag = base.tag(raw);
    for (key, value) in [
        ("Item", item_to_nbt(&e.stack)),
        ("Health", Tag::Short(e.health as i16)),
        ("Fire", Tag::Short(e.fire_ticks as i16)),
        ("Age", Tag::Short(e.age.min(i16::MAX as u32) as i16)),
        (
            "PickupDelay",
            Tag::Short(e.pickup_delay.min(i16::MAX as u32) as i16),
        ),
    ] {
        put(&mut tag, key, value);
    }
    if let Some(thrower) = e.thrower {
        put(&mut tag, "Thrower", Tag::IntArray(thrower.to_vec()));
    } else if let Tag::Compound(map) = &mut tag {
        map.remove(&"Thrower".into());
    }
    if let Some(target) = e.target {
        put(&mut tag, "Owner", Tag::IntArray(target.to_vec()));
    } else if let Tag::Compound(map) = &mut tag {
        map.remove(&"Owner".into());
    }
    tag
}
pub fn item_entity_from_nbt(tag: &Tag, id: u32, bob_offset: f32) -> Option<ItemEntity> {
    let stack = item_from_nbt(tag.get("Item")?)?;
    let position = position(tag)?;
    Some(ItemEntity {
        health: number(tag.get("Health")).unwrap_or(5) as i32,
        fire_ticks: number(tag.get("Fire")).unwrap_or(0) as i32,
        id,
        stack,
        thrower: uuid_from_tag(tag.get("Thrower")),
        target: uuid_from_tag(tag.get("Owner")),
        position,
        previous: position,
        velocity: motion(tag),
        on_ground: flag(tag, "OnGround"),
        // Age -32768 marks items that never despawn; Horizon counts upwards.
        age: number(tag.get("Age")).unwrap_or(0).max(0) as u32,
        pickup_delay: number(tag.get("PickupDelay")).unwrap_or(0).clamp(0, 32767) as u32,
        bob_offset,
        ticks: 0,
    })
}

/// `OminousItemSpawner.addAdditionalSaveData`: its item (left out when
/// empty) and `spawn_item_after_ticks`. Its tick count is the entity's
/// `tickCount`, which isn't saved: a reloaded spawner counts from 0 again.
pub fn ominous_item_spawner_to_nbt(
    item: &crate::items::ItemStack,
    position: Vec3,
    spawn_item_after: i64,
    uuid: [i32; 4],
    raw: Option<&Tag>,
) -> Tag {
    let base = Base {
        id: "minecraft:ominous_item_spawner",
        uuid,
        position,
        motion: Vec3::ZERO,
        rotation: [0.0, 0.0],
        on_ground: false,
        fall_distance: 0.0,
    };
    let mut tag = base.tag(raw);
    put(&mut tag, "item", item_to_nbt(item));
    put(
        &mut tag,
        "spawn_item_after_ticks",
        Tag::Long(spawn_item_after),
    );
    tag
}
/// `OminousItemSpawner.readAdditionalSaveData`: the item, where it floats
/// and `spawn_item_after_ticks` (0 when absent). None without an item:
/// such a spawner would spawn nothing.
pub fn ominous_item_spawner_from_nbt(tag: &Tag) -> Option<(crate::items::ItemStack, Vec3, i64)> {
    let item = tag.get("item").and_then(item_from_nbt)?;
    let after = number(tag.get("spawn_item_after_ticks")).unwrap_or(0);
    Some((item, position(tag)?, after))
}

/// `ExperienceOrb`: Health, Age, Value, Count.
pub fn orb_to_nbt(o: &ExperienceOrb, uuid: [i32; 4]) -> Tag {
    let base = Base {
        id: "minecraft:experience_orb",
        uuid,
        position: o.position,
        motion: o.velocity,
        rotation: [0.0, 0.0],
        on_ground: o.on_ground,
        fall_distance: 0.0,
    };
    let mut tag = base.tag(None);
    for (key, value) in [
        ("Health", Tag::Short(5)),
        ("Age", Tag::Short(o.age.min(i16::MAX as u32) as i16)),
        ("Value", Tag::Short(o.value.min(i16::MAX as u32) as i16)),
        ("Count", Tag::Int(o.count.min(i32::MAX as u32) as i32)),
    ] {
        put(&mut tag, key, value);
    }
    tag
}
/// One orb with its merged `Count`.
pub fn orbs_from_nbt(tag: &Tag) -> Vec<ExperienceOrb> {
    let Some(position) = position(tag) else {
        return Vec::new();
    };
    let count = number(tag.get("Count")).unwrap_or(1).max(1) as u32;
    let orb = ExperienceOrb {
        id: 0,
        position,
        previous: position,
        velocity: motion(tag),
        value: number(tag.get("Value")).unwrap_or(1).max(1) as u32,
        age: number(tag.get("Age")).unwrap_or(0).max(0) as u32,
        on_ground: flag(tag, "OnGround"),
        count,
        ticks: 0,
    };
    vec![orb]
}

/// `Projectile` + `AbstractArrow.addAdditionalSaveData` of a live arrow;
/// `in_block` is its `inBlockState`, `raw` the compound it was loaded with.
pub fn arrow_to_nbt(
    p: &Projectile,
    uuid: [i32; 4],
    owner: Option<[i32; 4]>,
    in_block: Option<Tag>,
    raw: Option<&Tag>,
) -> Option<Tag> {
    let a = p.arrow.as_deref()?;
    let ProjectileKind::Arrow {
        base_damage,
        critical,
        ..
    } = p.kind
    else {
        return None;
    };
    let base = Base {
        id: "minecraft:arrow",
        uuid,
        position: p.position,
        motion: p.velocity,
        rotation: [p.yaw, p.pitch],
        on_ground: false,
        fall_distance: a.fall_distance,
    };
    let mut tag = base.tag(raw);
    for (key, value) in [
        ("Fire", Tag::Short(a.fire_ticks as i16)),
        ("life", Tag::Short(a.life as i16)),
        ("shake", Tag::Byte(a.shake as i8)),
        ("inGround", Tag::Byte(i8::from(a.in_ground))),
        ("pickup", Tag::Byte(a.pickup.byte())),
        ("damage", Tag::Double(base_damage.to_bits())),
        ("crit", Tag::Byte(i8::from(critical))),
        ("PierceLevel", Tag::Byte(a.pierce as i8)),
        ("HasBeenShot", Tag::Byte(i8::from(a.has_been_shot))),
        ("item", item_to_nbt(&a.pickup_item)),
    ] {
        put(&mut tag, key, value);
    }
    if let Tag::Compound(map) = &mut tag {
        map.entry("SoundEvent".into())
            .or_insert_with(|| string("minecraft:entity.arrow.hit"));
        // Projectile: LeftOwner only when set; Owner as the UUID.
        map.remove(&"LeftOwner".into());
        map.remove(&"inBlockState".into());
        map.remove(&"weapon".into());
    }
    if let Some(weapon) = &a.fired_from_weapon {
        put(&mut tag, "weapon", item_to_nbt(weapon));
    }
    if p.left_owner {
        put(&mut tag, "LeftOwner", Tag::Byte(1));
    }
    if let Some(state) = in_block {
        put(&mut tag, "inBlockState", state);
    }
    if let Some(owner) = owner {
        put(&mut tag, "Owner", Tag::IntArray(owner.to_vec()));
    }
    Some(tag)
}
/// A saved arrow as a live one (`readAdditionalSaveData`), with its
/// `inBlockState` compound for the caller to resolve. `effect` is the tipped
/// arrow's lasting effect from its item.
pub fn arrow_from_nbt(
    tag: &Tag,
    player: Option<[i32; 4]>,
    effect: impl FnOnce(&crate::items::ItemStack) -> Option<(&'static str, i32)>,
) -> Option<(Projectile, Option<Tag>)> {
    let position = position(tag)?;
    let owner = uuid_from_tag(tag.get("Owner"));
    let from_player = player.is_some() && owner == player;
    let item = tag
        .get("item")
        .and_then(item_from_nbt)
        .unwrap_or_else(|| crate::items::ItemStack::new("minecraft:arrow", 1));
    let kind = ProjectileKind::Arrow {
        base_damage: float(tag.get("damage")).unwrap_or(2.0),
        critical: flag(tag, "crit"),
        effect: effect(&item),
    };
    let mut p = Projectile::new(kind, position, from_player.then_some(Who::Player));
    p.velocity = motion(tag);
    [p.yaw, p.pitch] = rotation(tag);
    p.left_owner = flag(tag, "LeftOwner");
    let a = p.arrow.as_mut()?;
    a.life = number(tag.get("life")).unwrap_or(0) as i16 as i32;
    a.shake = (number(tag.get("shake")).unwrap_or(0) as i8 as u8).into();
    a.in_ground = flag(tag, "inGround");
    a.pickup = Pickup::from_byte(number(tag.get("pickup")).unwrap_or(0));
    a.pierce = number(tag.get("PierceLevel")).unwrap_or(0) as u8;
    a.has_been_shot = flag(tag, "HasBeenShot");
    a.fire_ticks = number(tag.get("Fire")).unwrap_or(0) as i16 as i32;
    a.fall_distance = float(tag.get("fall_distance")).unwrap_or(0.0);
    a.pickup_item = item;
    a.fired_from_weapon = tag.get("weapon").and_then(item_from_nbt);
    p.stuck = a.in_ground;
    Some((p, tag.get("inBlockState").cloned()))
}

/// `LargeFireball` and `SmallFireball`: `AbstractHurtingProjectile`'s
/// `acceleration_power`, `Projectile`'s owner and flags, `ExplosionPower`.
pub fn fireball_to_nbt(
    p: &Projectile,
    uuid: [i32; 4],
    owner: Option<[i32; 4]>,
    raw: Option<&Tag>,
) -> Option<Tag> {
    let power = match p.kind {
        ProjectileKind::LargeFireball { power } => Some(power),
        ProjectileKind::SmallFireball => None,
        _ => return None,
    };
    let base = Base {
        id: p.kind.id(),
        uuid,
        position: p.position,
        motion: p.velocity,
        rotation: [p.yaw, p.pitch],
        on_ground: false,
        fall_distance: 0.0,
    };
    let mut tag = base.tag(raw);
    put(
        &mut tag,
        "acceleration_power",
        Tag::Double(p.acceleration.to_bits()),
    );
    // Projectile: LeftOwner only when set; HasBeenShot once it has ticked
    // (a fireball saved before its first tick keeps what it was loaded with).
    if let Tag::Compound(map) = &mut tag {
        map.remove(&"LeftOwner".into());
    }
    if p.left_owner {
        put(&mut tag, "LeftOwner", Tag::Byte(1));
    }
    if p.age > 0 || tag.get("HasBeenShot").is_none() {
        put(&mut tag, "HasBeenShot", Tag::Byte(i8::from(p.age > 0)));
    }
    if let Some(power) = power {
        put(&mut tag, "ExplosionPower", Tag::Byte(power as i8));
    }
    if let Some(owner) = owner {
        put(&mut tag, "Owner", Tag::IntArray(owner.to_vec()));
    }
    Some(tag)
}

pub fn fireball_from_nbt(tag: &Tag, player: Option<[i32; 4]>) -> Option<Projectile> {
    let kind = match kind(tag)?.as_str() {
        "minecraft:fireball" => ProjectileKind::LargeFireball {
            power: number(tag.get("ExplosionPower")).unwrap_or(1) as i32,
        },
        "minecraft:small_fireball" => ProjectileKind::SmallFireball,
        _ => return None,
    };
    let owner = uuid_from_tag(tag.get("Owner"));
    let from_player = player.is_some() && owner == player;
    let mut p = Projectile::new(kind, position(tag)?, from_player.then_some(Who::Player));
    p.velocity = motion(tag);
    [p.yaw, p.pitch] = rotation(tag);
    p.left_owner = flag(tag, "LeftOwner");
    p.acceleration = float(tag.get("acceleration_power")).unwrap_or(0.1);
    Some(p)
}

/// `FireworkRocketEntity.readAdditionalSaveData`: Life, LifeTime (0 when
/// absent: a bare `/summon` explodes at once), FireworksItem and
/// ShotAtAngle, with `Projectile`'s Owner and LeftOwner.
pub struct RocketNbt {
    pub position: Vec3,
    pub motion: Vec3,
    pub life: i32,
    pub lifetime: i32,
    pub item: Option<crate::items::ItemStack>,
    pub angled: bool,
    pub owner: Option<Who>,
    pub left_owner: bool,
}
pub fn firework_rocket_from_nbt(tag: &Tag, player: Option<[i32; 4]>) -> Option<RocketNbt> {
    let owner = uuid_from_tag(tag.get("Owner"));
    Some(RocketNbt {
        position: position(tag)?,
        motion: motion(tag),
        life: number(tag.get("Life")).unwrap_or(0) as i32,
        lifetime: number(tag.get("LifeTime")).unwrap_or(0) as i32,
        item: tag.get("FireworksItem").and_then(item_from_nbt),
        angled: flag(tag, "ShotAtAngle"),
        owner: (player.is_some() && owner == player).then_some(Who::Player),
        left_owner: flag(tag, "LeftOwner"),
    })
}

/// `FallingBlockEntity`: BlockState, Time, DropItem, HurtEntities.
#[allow(clippy::too_many_arguments)]
pub fn falling_block_to_nbt(
    state: Tag,
    position: Vec3,
    motion: Vec3,
    time: i32,
    drop_item: bool,
    damage: crate::falling_block::Damage,
    fall_distance: f64,
    uuid: [i32; 4],
) -> Tag {
    let base = Base {
        id: "minecraft:falling_block",
        uuid,
        position,
        motion,
        rotation: [0.0, 0.0],
        on_ground: false,
        fall_distance,
    };
    let mut tag = base.tag(None);
    for (key, value) in [
        ("BlockState", state),
        ("Time", Tag::Int(time)),
        ("DropItem", Tag::Byte(i8::from(drop_item))),
    ] {
        put(&mut tag, key, value);
    }
    damage.save(&mut tag);
    tag
}
/// (block state tag, position, motion, time, fall distance)
pub fn falling_block_from_nbt(tag: &Tag) -> Option<(Tag, Vec3, Vec3, i32, f64)> {
    Some((
        tag.get("BlockState")?.clone(),
        position(tag)?,
        motion(tag),
        number(tag.get("Time")).unwrap_or(0) as i32,
        float(tag.get("fall_distance")).unwrap_or(0.0),
    ))
}

/// `NoGravity` and `DropItem` (true when absent) of a saved falling block.
pub fn falling_block_flags(tag: &Tag) -> (bool, bool) {
    (
        flag(tag, "NoGravity"),
        number(tag.get("DropItem")).is_none_or(|v| v != 0),
    )
}

/// Painting.addAdditionalSaveData: `facing`, `variant` and `block_pos`.
pub fn painting_to_nbt(p: &Painting, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
    let (lo, hi) = p.bounds();
    let base = Base {
        id: "minecraft:painting",
        uuid,
        position: Vec3::new(
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ),
        motion: Vec3::ZERO,
        rotation: [f32::from(p.facing) * 90.0, 0.0],
        on_ground: false,
        fall_distance: 0.0,
    };
    let mut tag = base.tag(raw);
    put(&mut tag, "facing", Tag::Byte(p.facing as i8));
    put(&mut tag, "variant", string(&p.variant));
    put(&mut tag, "block_pos", Tag::IntArray(p.pos.to_vec()));
    tag
}
/// The block position, facing and variant of a saved painting.
pub fn painting_from_nbt(tag: &Tag) -> Option<([i32; 3], u8, String)> {
    let pos = match tag.get("block_pos") {
        Some(Tag::IntArray(v)) if v.len() == 3 => [v[0], v[1], v[2]],
        _ => return None,
    };
    let facing = (number(tag.get("facing")).unwrap_or(0) & 3) as u8;
    Some((pos, facing, text(tag.get("variant"))?))
}
/// Cushion.addAdditionalSaveData: the block it sits on and its colour.
pub fn cushion_to_nbt(c: &Cushion, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
    let base = Base {
        id: "minecraft:cushion",
        uuid,
        position: c.position,
        motion: Vec3::ZERO,
        rotation: [c.yaw, 0.0],
        on_ground: false,
        fall_distance: 0.0,
    };
    let mut tag = base.tag(raw);
    put(&mut tag, "block_pos", Tag::IntArray(c.pos.to_vec()));
    put(&mut tag, "color", string(c.color_name()));
    tag
}
/// The position, block position, yaw and colour of a saved cushion.
pub fn cushion_from_nbt(tag: &Tag, id: u32) -> Option<Cushion> {
    let position = position(tag)?;
    let pos = match tag.get("block_pos") {
        Some(Tag::IntArray(v)) if v.len() == 3 => [v[0], v[1], v[2]],
        _ => [
            position.x.floor() as i32,
            position.y.floor() as i32,
            position.z.floor() as i32,
        ],
    };
    let yaw = match tag.get("Rotation") {
        Some(Tag::List(l)) => l
            .first()
            .and_then(|v| crate::save::float(Some(v)))
            .unwrap_or(0.0) as f32,
        _ => 0.0,
    };
    let color =
        crate::entities::dye_index(&text(tag.get("color")).unwrap_or_else(|| "white".into()));
    Some(Cushion {
        id,
        position,
        pos,
        yaw,
        color,
        check_interval: 0,
    })
}

#[cfg(test)]
mod cushion_tests {
    use super::*;

    #[test]
    fn cushion_save_preserves_color_anchor_rotation_and_unknown_data() {
        let cushion = Cushion {
            id: 7,
            position: Vec3::new(-1.5, 64.5, 3.5),
            pos: [-2, 64, 3],
            yaw: 270.0,
            color: 14,
            check_interval: 55,
        };
        let raw = Tag::compound([("CustomName", string("Seat"))]);
        let tag = cushion_to_nbt(&cushion, [1, 2, 3, 4], Some(&raw));
        assert_eq!(text(tag.get("CustomName")).as_deref(), Some("Seat"));
        let loaded = cushion_from_nbt(&tag, 8).unwrap();
        assert_eq!(loaded.id, 8);
        assert_eq!(loaded.position, cushion.position);
        assert_eq!(loaded.pos, cushion.pos);
        assert_eq!(loaded.yaw, cushion.yaw);
        assert_eq!(loaded.color_name(), "red");
        assert_eq!(loaded.check_interval, 0);
        assert_eq!(loaded.bounds(), cushion.bounds());
    }
}
/// ItemFrame.addAdditionalSaveData.
pub fn item_frame_to_nbt(f: &ItemFrame, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
    let (lo, hi) = f.bounds();
    let base = Base {
        id: f.kind(),
        uuid,
        position: Vec3::new(
            (lo[0] + hi[0]) / 2.0,
            (lo[1] + hi[1]) / 2.0,
            (lo[2] + hi[2]) / 2.0,
        ),
        motion: Vec3::ZERO,
        rotation: [0.0, 0.0],
        on_ground: false,
        fall_distance: 0.0,
    };
    let mut tag = base.tag(raw);
    put(&mut tag, "Facing", Tag::Byte(f.facing as i8));
    put(&mut tag, "block_pos", Tag::IntArray(f.pos.to_vec()));
    put(&mut tag, "ItemRotation", Tag::Byte(f.rotation as i8));
    put(
        &mut tag,
        "ItemDropChance",
        Tag::Float(f.drop_chance.to_bits()),
    );
    put(&mut tag, "Invisible", Tag::Byte(i8::from(f.invisible)));
    put(&mut tag, "Fixed", Tag::Byte(i8::from(f.fixed)));
    if let Tag::Compound(map) = &mut tag {
        map.remove(&horizon_world::nbt::JavaString::from("Item"));
    }
    if let Some(item) = &f.item {
        put(&mut tag, "Item", item_to_nbt(item));
    }
    tag
}
pub fn item_frame_from_nbt(tag: &Tag, id: u32) -> Option<ItemFrame> {
    let pos = match tag.get("block_pos") {
        Some(Tag::IntArray(v)) if v.len() == 3 => [v[0], v[1], v[2]],
        _ => return None,
    };
    Some(ItemFrame {
        id,
        pos,
        facing: number(tag.get("Facing")).unwrap_or(2).clamp(0, 5) as u8,
        glow: kind(tag).as_deref() == Some("minecraft:glow_item_frame"),
        item: tag.get("Item").and_then(item_from_nbt),
        rotation: (number(tag.get("ItemRotation")).unwrap_or(0) & 7) as u8,
        drop_chance: float(tag.get("ItemDropChance")).map_or(1.0, |v| v as f32),
        fixed: flag(tag, "Fixed"),
        invisible: flag(tag, "Invisible"),
        check_interval: 0,
    })
}
/// ArmorStand.addAdditionalSaveData with LivingEntity's `equipment`.
pub fn armor_stand_to_nbt(a: &ArmorStand, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
    let base = Base {
        id: "minecraft:armor_stand",
        uuid,
        position: a.position,
        motion: a.velocity,
        rotation: [a.yaw, 0.0],
        on_ground: a.on_ground,
        fall_distance: 0.0,
    };
    let mut tag = base.tag(raw);
    let equipment: Vec<(&str, Tag)> = ArmorStand::SLOTS
        .iter()
        .zip(&a.equipment)
        .filter_map(|(slot, stack)| Some((*slot, item_to_nbt(stack.as_ref()?))))
        .collect();
    // `equipment` is saved only when something is worn, `Marker` only when set.
    if equipment.is_empty() {
        if let Tag::Compound(map) = &mut tag {
            map.remove(&"equipment".into());
        }
    } else {
        put(&mut tag, "equipment", Tag::compound(equipment));
    }
    for (key, value) in [
        ("Small", a.small),
        ("ShowArms", a.show_arms),
        ("NoBasePlate", a.no_base_plate),
        ("Invisible", a.invisible),
    ] {
        put(&mut tag, key, Tag::Byte(i8::from(value)));
    }
    if a.marker {
        put(&mut tag, "Marker", Tag::Byte(1));
    } else if let Tag::Compound(map) = &mut tag {
        map.remove(&"Marker".into());
    }
    if a.name_visible {
        put(&mut tag, "CustomNameVisible", Tag::Byte(1));
    } else if let Tag::Compound(map) = &mut tag {
        map.remove(&"CustomNameVisible".into());
    }
    // A name from the save keeps its own shape unless it changed.
    if let Some(name) = &a.custom_name
        && !tag.get("CustomName").is_some_and(|t| name_text(t) == *name)
    {
        put(
            &mut tag,
            "CustomName",
            crate::scoreboard::json_to_tag(&name.component()),
        );
    }
    put(&mut tag, "Health", Tag::Float(a.health.to_bits()));
    put(&mut tag, "Fire", Tag::Short(a.fire_ticks as i16));
    // ArmorStand.addAdditionalSaveData: the `Pose` parts that are not default.
    let pose: Vec<(&str, Tag)> = ArmorStand::POSE_KEYS
        .into_iter()
        .zip(a.pose)
        .zip(ArmorStand::DEFAULT_POSE)
        .filter(|((_, rotation), default)| rotation != default)
        .map(|((key, rotation), _)| (key, floats(&rotation)))
        .collect();
    put(&mut tag, "Pose", Tag::compound(pose));
    // LivingEntity.addAdditionalSaveData and ArmorStand's own fields.
    if let Tag::Compound(map) = &mut tag {
        for (key, value) in [
            ("HurtTime", Tag::Short(0)),
            ("DeathTime", Tag::Short(0)),
            ("AbsorptionAmount", Tag::Float(0)),
            ("FallFlying", Tag::Byte(0)),
            ("current_impulse_context_reset_grace_time", Tag::Int(0)),
            ("attributes", Tag::List(Vec::new())),
            (
                "Brain",
                Tag::compound([("memories", Tag::Compound(Compound::new()))]),
            ),
            ("DisabledSlots", Tag::Int(0)),
            ("Pose", Tag::Compound(Compound::new())),
        ] {
            map.entry(key.into()).or_insert(value);
        }
    }
    tag
}
pub fn armor_stand_from_nbt(tag: &Tag, id: u32) -> Option<ArmorStand> {
    let mut a = ArmorStand::new(id, position(tag)?, rotation(tag)[0]);
    a.velocity = motion(tag);
    a.health = float(tag.get("Health")).unwrap_or(20.0) as f32;
    a.fire_ticks = number(tag.get("Fire")).unwrap_or(0) as i32;
    a.custom_name = tag.get("CustomName").map(name_text);
    a.name_visible = flag(tag, "CustomNameVisible");
    if let Some(eq) = tag.get("equipment") {
        for (i, slot) in ArmorStand::SLOTS.iter().enumerate() {
            a.equipment[i] = eq.get(slot).and_then(item_from_nbt);
        }
    }
    a.small = flag(tag, "Small");
    a.show_arms = flag(tag, "ShowArms");
    a.no_base_plate = flag(tag, "NoBasePlate");
    a.invisible = flag(tag, "Invisible");
    a.marker = flag(tag, "Marker");
    a.on_ground = flag(tag, "OnGround");
    // ArmorStand.readAdditionalSaveData: each part of `Pose` that is a list of
    // three numbers, the others staying at their defaults.
    if let Some(pose) = tag.get("Pose") {
        for (rotation, key) in a.pose.iter_mut().zip(ArmorStand::POSE_KEYS) {
            if let Some(Tag::List(values)) = pose.get(key)
                && values.len() == 3
            {
                for (r, v) in rotation.iter_mut().zip(values) {
                    *r = float(Some(v)).unwrap_or(0.0) as f32;
                }
            }
        }
    }
    Some(a)
}
/// A boat entity (`minecraft:<wood>_boat`, `minecraft:bamboo_chest_raft`...):
/// chest boats keep their slots in `Items` (`ContainerHelper.saveAllItems`).
pub fn boat_to_nbt(
    id: &str,
    position: Vec3,
    motion: Vec3,
    yaw: f32,
    uuid: [i32; 4],
    raw: Option<&Tag>,
    items: Option<&[Option<crate::items::ItemStack>]>,
) -> Tag {
    let mut tag = Base {
        id,
        uuid,
        position,
        motion,
        rotation: [yaw, 0.0],
        on_ground: false,
        fall_distance: 0.0,
    }
    .tag(raw);
    if let (Some(items), Tag::Compound(map)) = (items, &mut tag) {
        map.insert("Items".into(), crate::save::encode_items(items));
    }
    tag
}
/// A vehicle's base entity tag (boats and minecarts).
pub fn vehicle_to_nbt(
    id: &str,
    position: Vec3,
    motion: Vec3,
    yaw: f32,
    uuid: [i32; 4],
    raw: Option<&Tag>,
) -> Tag {
    Base {
        id,
        uuid,
        position,
        motion,
        rotation: [yaw, 0.0],
        on_ground: false,
        fall_distance: 0.0,
    }
    .tag(raw)
}
pub fn boat_from_nbt(tag: &Tag) -> Option<(Vec3, Vec3, f32)> {
    Some((position(tag)?, motion(tag), rotation(tag)[0]))
}

/// `VillagerData`: type, profession and level.
fn villager_data(m: &Mob) -> Tag {
    use crate::mobs::render::{PROFESSIONS, VILLAGER_TYPES};
    Tag::compound([
        (
            "type",
            string(&format!(
                "minecraft:{}",
                VILLAGER_TYPES[m.extra.variant as usize % VILLAGER_TYPES.len()]
            )),
        ),
        (
            "profession",
            string(&format!(
                "minecraft:{}",
                PROFESSIONS[m.extra.variant2 as usize % PROFESSIONS.len()]
            )),
        ),
        ("level", Tag::Int(i32::from(m.extra.variant3.max(1)))),
    ])
}
/// `AbstractVillager` "Offers" (`MerchantOffers.CODEC`).
fn put_offers(map: &mut Compound, m: &Mob) {
    let Some(t) = m.extra.trade.as_ref().filter(|t| !t.unset) else {
        return;
    };
    map.insert("Offers".into(), offers_to_nbt(&t.offers));
}
/// `ItemCost.CODEC`: id, count (always written) and the exact components.
fn item_cost_to_nbt(stack: &crate::items::ItemStack) -> Tag {
    let mut tag = item_to_nbt(stack);
    if let Tag::Compound(c) = &mut tag
        && !c.contains_key(&"count".into())
    {
        c.insert("count".into(), Tag::Int(stack.count as i32));
    }
    tag
}
/// `MerchantOffers.CODEC`: `{Recipes: [...]}`, each offer without the
/// fields at their defaults.
pub fn offers_to_nbt(offers: &[crate::merchant::MerchantOffer]) -> Tag {
    let recipes = offers
        .iter()
        .map(|o| {
            let mut entries = vec![
                ("buy", item_cost_to_nbt(&o.base_cost_a.stack)),
                ("sell", item_to_nbt(&o.result)),
            ];
            // `MerchantOffer.CODEC` omits fields at their defaults.
            if o.uses != 0 {
                entries.push(("uses", Tag::Int(o.uses)));
            }
            if o.max_uses != 4 {
                entries.push(("maxUses", Tag::Int(o.max_uses)));
            }
            if o.price_multiplier != 0.0 {
                entries.push(("priceMultiplier", Tag::Float(o.price_multiplier.to_bits())));
            }
            if o.xp != 1 {
                entries.push(("xp", Tag::Int(o.xp)));
            }
            if !o.reward_exp {
                entries.push(("rewardExp", Tag::Byte(0)));
            }
            if o.special_price_diff != 0 {
                entries.push(("specialPrice", Tag::Int(o.special_price_diff)));
            }
            if o.demand != 0 {
                entries.push(("demand", Tag::Int(o.demand)));
            }
            if let Some(b) = &o.cost_b {
                entries.push(("buyB", item_cost_to_nbt(&b.stack)));
            }
            Tag::compound(entries)
        })
        .collect::<Vec<_>>();
    // An empty list is the field's default: left out.
    if recipes.is_empty() {
        return Tag::Compound(Compound::new());
    }
    Tag::compound([("Recipes", Tag::List(recipes))])
}
/// The offers of a `MerchantOffers.CODEC` tag.
pub fn offers_from_nbt(tag: &Tag) -> Vec<crate::merchant::MerchantOffer> {
    use crate::merchant::{ItemCost, MerchantOffer};
    let Some(Tag::List(recipes)) = tag.get("Recipes") else {
        return Vec::new();
    };
    recipes
        .iter()
        .filter_map(|r| {
            let cost = |key| {
                r.get(key)
                    .and_then(item_from_nbt)
                    .map(|stack| ItemCost { stack })
            };
            Some(MerchantOffer {
                base_cost_a: cost("buy")?,
                cost_b: cost("buyB"),
                result: r.get("sell").and_then(item_from_nbt)?,
                uses: number(r.get("uses")).unwrap_or(0) as i32,
                max_uses: number(r.get("maxUses")).unwrap_or(4) as i32,
                reward_exp: number(r.get("rewardExp")).unwrap_or(1) != 0,
                special_price_diff: number(r.get("specialPrice")).unwrap_or(0) as i32,
                demand: number(r.get("demand")).unwrap_or(0) as i32,
                price_multiplier: float(r.get("priceMultiplier")).unwrap_or(0.0) as f32,
                xp: number(r.get("xp")).unwrap_or(1) as i32,
            })
        })
        .collect()
}
/// The villager brain's `job_site`, `home` and `meeting_point` memories.
fn put_memories(map: &mut Compound, m: &Mob) {
    let mut memories: Vec<(&str, Tag)> = [
        ("minecraft:job_site", m.extra.home),
        ("minecraft:home", m.extra.bed),
        ("minecraft:meeting_point", m.extra.meeting),
    ]
    .into_iter()
    .filter_map(|(key, p)| {
        let pos = Tag::compound([
            ("dimension", string("minecraft:overworld")),
            ("pos", Tag::IntArray(p?.to_vec())),
        ]);
        Some((key, Tag::compound([("value", pos)])))
    })
    .collect();
    crate::mobs::villager::write_memories(m, &mut memories);
    map.insert(
        "Brain".into(),
        Tag::compound([("memories", Tag::compound(memories))]),
    );
}
fn read_villager(m: &mut Mob, tag: &Tag) {
    use crate::{
        merchant::Trader,
        mobs::render::{PROFESSIONS, VILLAGER_TYPES},
    };
    if let Some(data) = tag.get("VillagerData") {
        let name = |key| text(data.get(key)).map(|s| s.trim_start_matches("minecraft:").to_owned());
        if let Some(t) = name("type").and_then(|t| VILLAGER_TYPES.iter().position(|v| *v == t)) {
            m.extra.variant = t as u8;
        }
        m.extra.variant2 = name("profession")
            .and_then(|p| PROFESSIONS.iter().position(|v| *v == p))
            .unwrap_or(0) as u8;
        m.extra.variant3 = number(data.get("level")).unwrap_or(1).clamp(1, 5) as u8;
    }
    let memory = |key: &str| {
        tag.get("Brain")
            .and_then(|b| b.get("memories"))
            .and_then(|b| b.get(key))
            .and_then(|j| j.get("value"))
            .and_then(|v| match v.get("pos") {
                Some(Tag::IntArray(a)) if a.len() == 3 => Some([a[0], a[1], a[2]]),
                _ => None,
            })
    };
    m.extra.home = memory("minecraft:job_site");
    m.extra.bed = memory("minecraft:home");
    m.extra.meeting = memory("minecraft:meeting_point");
    if m.species() == crate::mobs::Species::Villager {
        crate::mobs::villager::read_nbt(m, tag);
    }
    let offers_tag = tag.get("Offers");
    let xp = number(tag.get("Xp")).unwrap_or(0) as i32;
    if offers_tag.is_none() && xp == 0 && tag.get("LastRestock").is_none() {
        return;
    }
    let offers = offers_tag.map(offers_from_nbt).unwrap_or_default();
    m.extra.trade = Some(Box::new(Trader {
        unset: offers_tag.is_none(),
        xp: number(tag.get("Xp")).unwrap_or(0) as i32,
        offers,
        last_restock: number(tag.get("LastRestock")).unwrap_or(0),
        restocks_today: number(tag.get("RestocksToday")).unwrap_or(0) as i32,
        check_day: 0,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::ItemStack;

    /// The item, the spawn delay and the fields Horizon doesn't model come
    /// back; a compound without an item is no spawner.
    #[test]
    fn ominous_item_spawner_round_trip() {
        let item = ItemStack {
            item: Arc::from("minecraft:ominous_trial_key"),
            count: 1,
            damage: 0,
            extra: None,
        };
        let raw = Tag::compound([("CustomName", string("kept"))]);
        let at = Vec3::new(10.5, 64.25, -3.5);
        let tag = ominous_item_spawner_to_nbt(&item, at, 97, [1, 2, 3, 4], Some(&raw));
        assert_eq!(
            kind(&tag).as_deref(),
            Some("minecraft:ominous_item_spawner")
        );
        assert_eq!(uuid(&tag), Some([1, 2, 3, 4]));
        assert!(matches!(
            tag.get("spawn_item_after_ticks"),
            Some(Tag::Long(97))
        ));
        assert!(tag.get("CustomName").is_some());
        let (back, pos, after) = ominous_item_spawner_from_nbt(&tag).unwrap();
        assert_eq!(
            (back.item.as_ref(), back.count),
            ("minecraft:ominous_trial_key", 1)
        );
        assert_eq!((pos, after), (at, 97));
        assert!(ominous_item_spawner_from_nbt(&raw).is_none());
    }

    #[test]
    fn armor_stand_names_keep_their_formatting_across_a_save() {
        let tag = Tag::compound([
            (
                "Pos",
                Tag::List(vec![
                    Tag::Double(0.0f64.to_bits()),
                    Tag::Double(64.0f64.to_bits()),
                    Tag::Double(0.0f64.to_bits()),
                ]),
            ),
            ("CustomName", string("§6Hello")),
            ("CustomNameVisible", Tag::Byte(1)),
        ]);
        let a = armor_stand_from_nbt(&tag, 7).unwrap();
        let name = a.custom_name.as_ref().unwrap();
        assert_eq!(name.plain(), "Hello");
        assert!(name.spans[0].style.color.is_some());
        assert!(a.name_visible);
        let saved = armor_stand_to_nbt(&a, [0; 4], Some(&tag));
        let restored = armor_stand_from_nbt(&saved, 7).unwrap();
        assert_eq!(restored.custom_name, a.custom_name);
        assert!(restored.name_visible);
    }

    #[test]
    fn falling_block_round_trip_keeps_impact_distance_and_settings() {
        let state = Tag::compound([("Name", string("minecraft:damaged_anvil"))]);
        let position = Vec3::new(3.5, 100.0, -7.5);
        let velocity = Vec3::new(0.0, -1.25, 0.0);
        let damage = crate::falling_block::Damage {
            cancel_drop: true,
            ..crate::falling_block::Damage::anvil()
        };
        let tag = falling_block_to_nbt(
            state.clone(),
            position,
            velocity,
            30,
            false,
            damage,
            19.75,
            [1, 2, 3, 4],
        );
        let (loaded_state, pos, motion, age, distance) = falling_block_from_nbt(&tag).unwrap();
        assert_eq!(loaded_state, state);
        assert_eq!(
            (pos, motion, age, distance),
            (position, velocity, 30, 19.75)
        );
        assert_eq!(falling_block_flags(&tag), (false, false));
        assert_eq!(crate::falling_block::Damage::from_nbt(&tag, true), damage);
        assert_eq!(uuid(&tag), Some([1, 2, 3, 4]));
    }

    #[test]
    fn item_entities_round_trip() {
        let e = ItemEntity {
            health: 2,
            fire_ticks: 73,
            thrower: Some([5, 6, 7, 8]),
            target: None,
            id: 7,
            stack: ItemStack {
                item: Arc::from("minecraft:diamond_pickaxe"),
                count: 1,
                damage: 5,
                extra: None,
            },
            position: Vec3::new(100.5, 64.0, -31.5),
            previous: Vec3::ZERO,
            velocity: Vec3::new(0.0, -0.04, 0.0),
            on_ground: true,
            age: 120,
            pickup_delay: 10,
            ticks: 0,
            bob_offset: 0.0,
        };
        let tag = item_entity_to_nbt(&e, [1, 2, 3, 4], None);
        assert_eq!(kind(&tag).as_deref(), Some("minecraft:item"));
        assert_eq!(uuid(&tag), Some([1, 2, 3, 4]));
        let back = item_entity_from_nbt(&tag, 9, 0.0).unwrap();
        assert_eq!(back.stack.damage, 5);
        assert_eq!((back.health, back.fire_ticks), (2, 73));
        assert_eq!(
            (back.age, back.pickup_delay, back.on_ground),
            (120, 10, true)
        );
        assert_eq!(back.position.x, 100.5);
        assert_eq!(back.thrower, e.thrower);
        let mut cleared = back;
        cleared.thrower = None;
        let cleared = item_entity_to_nbt(&cleared, [1, 2, 3, 4], Some(&tag));
        assert!(cleared.get("Thrower").is_none());
    }
    #[test]
    fn unknown_fields_survive() {
        let raw = Tag::compound([
            ("id", string("minecraft:item")),
            ("CustomName", string("x")),
            ("Pos", doubles(&[0.5, 64.0, 0.5])),
            (
                "Item",
                Tag::compound([("id", string("minecraft:stone")), ("count", Tag::Int(3))]),
            ),
        ]);
        let e = item_entity_from_nbt(&raw, 1, 0.0).unwrap();
        let tag = item_entity_to_nbt(&e, [0; 4], Some(&raw));
        assert_eq!(text(tag.get("CustomName")).as_deref(), Some("x"));
        assert_eq!(number(tag.get("Health")), Some(5));
    }
    #[test]
    fn orbs_keep_their_count() {
        let o = ExperienceOrb {
            id: 0,
            position: Vec3::new(1.0, 2.0, 3.0),
            previous: Vec3::ZERO,
            velocity: Vec3::ZERO,
            value: 7,
            age: 3,
            on_ground: false,
            count: 3,
            ticks: 0,
        };
        let tag = orb_to_nbt(&o, [0; 4]);
        let back = orbs_from_nbt(&tag);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].value, 7);
        assert_eq!(back[0].count, 3);
    }
}

fn write_axolotl_memories(m: &Mob, map: &mut Compound) {
    let brain = map
        .entry("Brain".into())
        .or_insert_with(|| Tag::Compound(Compound::new()));
    if !matches!(brain, Tag::Compound(_)) {
        *brain = Tag::Compound(Compound::new());
    }
    let Tag::Compound(brain) = brain else { return };
    let memories = brain
        .entry("memories".into())
        .or_insert_with(|| Tag::Compound(Compound::new()));
    if !matches!(memories, Tag::Compound(_)) {
        *memories = Tag::Compound(Compound::new());
    }
    let Tag::Compound(memories) = memories else {
        return;
    };
    let mm = m.extra.animal_brain.as_deref().map(|b| &b.mem);
    let hunting = crate::mobs::animal_brain::hunting_cooldown(m)
        .map(|ttl| Tag::compound([("value", Tag::Byte(1)), ("ttl", Tag::Long(ttl))]));
    let play_dead = mm
        .and_then(|m| m.play_dead_ticks.value())
        .map(|v| Tag::compound([("value", Tag::Int(v))]));
    let temptation = mm
        .and_then(|m| m.temptation_cooldown_ticks.value())
        .map(|v| Tag::compound([("value", Tag::Int(v))]));
    for (key, value) in [
        ("minecraft:has_hunting_cooldown", hunting),
        ("minecraft:play_dead_ticks", play_dead),
        ("minecraft:temptation_cooldown_ticks", temptation),
    ] {
        if let Some(value) = value {
            memories.insert(key.into(), value);
        } else {
            memories.remove(&key.into());
        }
    }
}
/// The variant tags `Axolotl`, `Rabbit`, `Parrot`, `Salmon` and
/// `TropicalFish` read; a missing tag is the default variant (a medium
/// salmon, otherwise index 0).
fn read_variant(m: &mut Mob, tag: &Tag) {
    use crate::mobs::Species as S;
    let int = |key| number(tag.get(key)).unwrap_or(0) as i32;
    match m.species() {
        S::Axolotl | S::Parrot => m.extra.variant = int("Variant").clamp(0, 4) as u8,
        // Rabbit.Variant.LEGACY_CODEC: an unknown id is brown.
        S::Rabbit => {
            m.extra.variant = match int("RabbitType") {
                v @ (0..=5 | 99) => v as u8,
                _ => 0,
            }
        }
        S::Salmon => {
            m.extra.variant = text(tag.get("type"))
                .and_then(|t| SALMON_SIZES.iter().position(|v| *v == t))
                .unwrap_or(1) as u8;
        }
        S::TropicalFish => {
            let packed = int("Variant");
            let (size, index) = (packed & 0xFF, (packed >> 8) & 0xFF);
            // TropicalFish.Pattern.byId: an unknown id is a kob.
            m.extra.variant = if size <= 1 && index < 6 {
                (size * 6 + index) as u8
            } else {
                0
            };
            m.extra.variant2 = ((packed >> 16) & 15) as u8;
            m.extra.variant3 = ((packed >> 24) & 15) as u8;
        }
        _ => {}
    }
}
fn restore_axolotl_memories(m: &mut Mob, tag: &Tag) {
    let memories = tag.get("Brain").and_then(|b| b.get("memories"));
    let ttl = memories
        .and_then(|b| b.get("minecraft:has_hunting_cooldown"))
        .and_then(|v| number(v.get("ttl")))
        .unwrap_or(0);
    crate::mobs::animal_brain::set_hunting_cooldown(m, ttl);
    if let Some(b) = m.extra.animal_brain.as_mut() {
        if let Some(v) = memories
            .and_then(|b| b.get("minecraft:play_dead_ticks"))
            .and_then(|v| number(v.get("value")))
        {
            b.mem.play_dead_ticks.set(v as i32);
        }
        if let Some(v) = memories
            .and_then(|b| b.get("minecraft:temptation_cooldown_ticks"))
            .and_then(|v| number(v.get("value")))
        {
            b.mem.temptation_cooldown_ticks.set(v as i32);
        }
    }
}
const AXOLOTL_VARIANTS: [&str; 5] = ["lucy", "wild", "gold", "cyan", "blue"];
/// `Salmon.Variant` names, by id.
const SALMON_SIZES: [&str; 3] = ["small", "medium", "large"];
/// `TropicalFish.Pattern` names in enum order: the six small patterns, then
/// the six large ones (the order of `extra.variant`).
const TROPICAL_PATTERNS: [&str; 12] = [
    "kob",
    "sunstreak",
    "snooper",
    "dasher",
    "brinely",
    "spotty",
    "flopper",
    "stripey",
    "glitter",
    "blockfish",
    "betty",
    "clayfish",
];
/// `Bucketable.saveDefaultDataToBucketTag`, with the age an axolotl and a
/// tadpole add (`saveToBucketTag`).
pub fn bucket_data(m: &Mob) -> Tag {
    let mut data = Tag::compound([("Health", Tag::Float(m.health.to_bits()))]);
    if matches!(
        m.species(),
        crate::mobs::Species::Axolotl | crate::mobs::Species::Tadpole
    ) {
        put(&mut data, "Age", Tag::Int(m.extra.age));
        put(
            &mut data,
            "AgeLocked",
            Tag::Byte(i8::from(m.extra.age_locked)),
        );
    }
    for (key, on) in [
        ("NoAI", m.no_ai),
        ("Silent", m.silent),
        ("NoGravity", m.no_gravity),
        ("Glowing", m.glowing),
        ("Invulnerable", m.invulnerable_flag),
        ("PersistenceRequired", m.persistent),
    ] {
        if on {
            put(&mut data, key, Tag::Byte(1));
        }
    }
    if let Some(ttl) = crate::mobs::animal_brain::hunting_cooldown(m) {
        put(&mut data, "HuntingCooldown", Tag::Long(ttl));
    }
    data
}
pub fn bucket_components(m: &Mob) -> crate::item_data::StackData {
    let mut data = crate::item_data::StackData {
        custom_name: m.custom_name.clone(),
        ..Default::default()
    };
    data.set_other("minecraft:bucket_entity_data", Some(bucket_data(m)));
    if m.species() == crate::mobs::Species::Axolotl {
        let variant = AXOLOTL_VARIANTS
            .get(usize::from(m.extra.variant))
            .unwrap_or(&"lucy");
        data.set_other("minecraft:axolotl/variant", Some(string(variant)));
    }
    // Salmon.saveToBucketTag: the size.
    if m.species() == crate::mobs::Species::Salmon {
        let size = SALMON_SIZES
            .get(usize::from(m.extra.variant))
            .unwrap_or(&"medium");
        data.set_other("minecraft:salmon/size", Some(string(size)));
    }
    // TropicalFish.saveToBucketTag: the pattern and both colours.
    if m.species() == crate::mobs::Species::TropicalFish {
        let dye = |i: u8| crate::mobs::render::DYE_NAMES[usize::from(i) & 15];
        let pattern = TROPICAL_PATTERNS
            .get(usize::from(m.extra.variant))
            .unwrap_or(&"kob");
        data.set_other("minecraft:tropical_fish/pattern", Some(string(pattern)));
        data.set_other(
            "minecraft:tropical_fish/base_color",
            Some(string(dye(m.extra.variant2))),
        );
        data.set_other(
            "minecraft:tropical_fish/pattern_color",
            Some(string(dye(m.extra.variant3))),
        );
    }
    data
}
pub fn load_bucket_data(m: &mut Mob, stack: &crate::items::ItemStack) {
    let Some(components) = stack.data() else {
        return;
    };
    if let Some(name) = &components.custom_name {
        m.custom_name = Some(name.clone());
    }
    if let Some(data) = components.other("minecraft:bucket_entity_data") {
        if let Some(v) = number(data.get("Age")) {
            m.extra.age = v as i32;
        }
        if let Some(v) = number(data.get("AgeLocked")) {
            m.extra.age_locked = v != 0;
        }
        // Keep old Horizon buckets readable; the modern component below takes precedence.
        if let Some(v) = number(data.get("Variant")) {
            m.extra.variant = v as u8;
        }
        if let Some(v) = float(data.get("Health")) {
            m.health = v as f32;
        }
        for (key, flag) in [
            ("NoAI", &mut m.no_ai),
            ("Silent", &mut m.silent),
            ("NoGravity", &mut m.no_gravity),
            ("Glowing", &mut m.glowing),
            ("Invulnerable", &mut m.invulnerable_flag),
        ] {
            if let Some(v) = number(data.get(key)) {
                *flag = v != 0;
            }
        }
        m.persistent |= flag(data, "PersistenceRequired");
        if m.species() == crate::mobs::Species::Axolotl {
            crate::mobs::animal_brain::set_hunting_cooldown(
                m,
                number(data.get("HuntingCooldown")).unwrap_or(0),
            );
        }
    }
    if m.species() == crate::mobs::Species::Axolotl
        && let Some(name) = text(components.other("minecraft:axolotl/variant"))
        && let Some(id) = AXOLOTL_VARIANTS
            .iter()
            .position(|&v| v == name.trim_start_matches("minecraft:"))
    {
        m.extra.variant = id as u8;
    }
    // Salmon.applyImplicitComponents: the size.
    if m.species() == crate::mobs::Species::Salmon
        && let Some(name) = text(components.other("minecraft:salmon/size"))
        && let Some(id) = SALMON_SIZES
            .iter()
            .position(|&v| v == name.trim_start_matches("minecraft:"))
    {
        m.extra.variant = id as u8;
    }
    // TropicalFish.applyImplicitComponent: the pattern and both colours.
    if m.species() == crate::mobs::Species::TropicalFish {
        let name = |key| text(components.other(key));
        let dye = |key| {
            name(key).and_then(|n| {
                crate::mobs::render::DYE_NAMES
                    .iter()
                    .position(|&v| v == n.trim_start_matches("minecraft:"))
            })
        };
        if let Some(id) = name("minecraft:tropical_fish/pattern").and_then(|n| {
            TROPICAL_PATTERNS
                .iter()
                .position(|&v| v == n.trim_start_matches("minecraft:"))
        }) {
            m.extra.variant = id as u8;
        }
        if let Some(id) = dye("minecraft:tropical_fish/base_color") {
            m.extra.variant2 = id as u8;
        }
        if let Some(id) = dye("minecraft:tropical_fish/pattern_color") {
            m.extra.variant3 = id as u8;
        }
    }
}

#[cfg(test)]
mod axolotl_nbt_tests {
    use super::*;
    #[test]
    fn bucket_cooldown_and_brain_memories_survive_and_expired_cooldown_is_removed() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let data = crate::mobs::MobTypes::load(&root).unwrap();
        let kind = Arc::new(data.types.get("minecraft:axolotl").unwrap().clone());
        let mut m = Mob::new(1, kind.clone(), Vec3::new(0.0, 4.0, 0.0), 0.0);
        crate::mobs::animal_brain::set_hunting_cooldown(&mut m, 123);
        m.extra.variant = 4;
        m.extra.age = -24000;
        m.extra.age_locked = true;
        m.no_ai = true;
        m.silent = true;
        m.no_gravity = true;
        m.glowing = true;
        m.invulnerable_flag = true;
        m.persistent = true;
        m.custom_name = Some("Bucket friend".into());
        m.health = 5.0;

        {
            let mm = &mut m.extra.animal_brain.as_mut().unwrap().mem;
            mm.play_dead_ticks.set(78);
            mm.temptation_cooldown_ticks.set(42);
        }
        let raw = Tag::compound([(
            "Brain",
            Tag::compound([(
                "memories",
                Tag::compound([("test:unrelated", Tag::compound([("value", Tag::Int(9))]))]),
            )]),
        )]);
        let tag = mob_to_nbt(&m, [1, 2, 3, 4], Some(&raw));
        let memories = tag.get("Brain").unwrap().get("memories").unwrap();
        assert!(memories.get("test:unrelated").is_some());
        assert_eq!(
            number(
                memories
                    .get("minecraft:play_dead_ticks")
                    .unwrap()
                    .get("value")
            ),
            Some(78)
        );
        assert_eq!(
            number(
                memories
                    .get("minecraft:temptation_cooldown_ticks")
                    .unwrap()
                    .get("value")
            ),
            Some(42)
        );
        let mut stack = crate::items::ItemStack::new("minecraft:axolotl_bucket", 1);
        stack.extra = Some(Box::new(bucket_components(&m)));
        assert_eq!(
            text(stack.data().unwrap().other("minecraft:axolotl/variant")),
            Some("blue".into())
        );
        assert!(bucket_data(&m).get("Variant").is_none());
        let stack = item_from_nbt(&item_to_nbt(&stack)).unwrap();
        let mut restored = Mob::new(2, kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
        load_bucket_data(&mut restored, &stack);
        assert_eq!(
            crate::mobs::animal_brain::hunting_cooldown(&restored),
            Some(123)
        );
        assert_eq!(
            (
                restored.extra.variant,
                restored.extra.age,
                restored.extra.age_locked
            ),
            (4, -24000, true)
        );
        assert!(
            restored.no_ai
                && restored.silent
                && restored.no_gravity
                && restored.glowing
                && restored.invulnerable_flag
                && restored.persistent
        );
        assert_eq!(restored.custom_name.as_deref(), Some("Bucket friend"));
        assert_eq!(restored.health, 5.0);
        m.glowing = false;
        m.add_effect("glowing", 200, 0);
        assert!(m.has_effect("glowing"));
        assert!(bucket_data(&m).get("Glowing").is_none());
        m.glowing = true;
        assert_eq!(number(bucket_data(&m).get("Glowing")), Some(1));
        for (persistent, expected) in [(None, false), (Some(0), false), (Some(1), true)] {
            let mut data = Tag::Compound(Compound::new());
            if let Some(value) = persistent {
                put(&mut data, "PersistenceRequired", Tag::Byte(value));
            }
            let mut bucket = crate::items::ItemStack::new("minecraft:axolotl_bucket", 1);
            bucket
                .data_mut()
                .set_other("minecraft:bucket_entity_data", Some(data));
            restored.persistent = false;
            load_bucket_data(&mut restored, &bucket);
            assert_eq!(restored.persistent, expected);
            restored.persistent = true;
            load_bucket_data(&mut restored, &bucket);
            assert!(restored.persistent, "bucket data cannot clear persistence");
        }
        restored.ticks_frozen = 140;
        restored.ignite_for_ticks(160);
        assert_eq!(restored.ticks_frozen, 0);
        restored.ticks_frozen = 140;
        restored.ignite_for_ticks(1);
        assert_eq!(
            restored.ticks_frozen, 0,
            "even an ignition that does not extend burning clears freeze"
        );
        for (id, name) in AXOLOTL_VARIANTS.iter().enumerate() {
            let mut vanilla = crate::items::ItemStack::new("minecraft:axolotl_bucket", 1);
            vanilla.extra = Some(Box::new(crate::item_data::StackData {
                other: vec![("minecraft:axolotl/variant".into(), string(name))],
                ..Default::default()
            }));
            load_bucket_data(&mut restored, &vanilla);
            assert_eq!(
                usize::from(restored.extra.variant),
                id,
                "component-only vanilla buckets work"
            );
        }
        let mut legacy = crate::items::ItemStack::new("minecraft:axolotl_bucket", 1);
        legacy.extra = Some(Box::new(crate::item_data::StackData {
            other: vec![(
                "minecraft:bucket_entity_data".into(),
                Tag::compound([("Variant", Tag::Int(3))]),
            )],
            ..Default::default()
        }));
        load_bucket_data(&mut restored, &legacy);
        assert_eq!(restored.extra.variant, 3);
        legacy
            .data_mut()
            .set_other("minecraft:axolotl/variant", Some(string("gold")));
        load_bucket_data(&mut restored, &legacy);
        assert_eq!(
            restored.extra.variant, 2,
            "component wins over old Horizon data"
        );

        crate::mobs::animal_brain::set_hunting_cooldown(&mut m, 0);
        assert!(bucket_data(&m).get("HuntingCooldown").is_none());
        let saved = mob_to_nbt(&m, [1, 2, 3, 4], Some(&tag));
        assert!(
            saved
                .get("Brain")
                .unwrap()
                .get("memories")
                .unwrap()
                .get("minecraft:has_hunting_cooldown")
                .is_none()
        );
        restore_axolotl_memories(&mut restored, &saved);
        let mm = &restored.extra.animal_brain.as_ref().unwrap().mem;
        assert_eq!(mm.play_dead_ticks.value(), Some(78));
        assert_eq!(mm.temptation_cooldown_ticks.value(), Some(42));
    }
}

#[cfg(test)]
mod sheep_nbt_tests {
    use super::*;
    #[test]
    fn a_sheep_keeps_its_dyed_and_sheared_wool() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let data = crate::mobs::MobTypes::load(&root).unwrap();
        let mut m = Mob::new(
            1,
            Arc::new(data.types["minecraft:sheep"].clone()),
            Vec3::new(0.0, 4.0, 0.0),
            0.0,
        );
        // Summoned white, then dyed red and sheared: the save has it now.
        let raw = Tag::compound([("Color", Tag::Byte(0)), ("Sheared", Tag::Byte(0))]);
        (m.extra.color, m.extra.sheared) = (14, true);
        let tag = mob_to_nbt(&m, [1, 2, 3, 4], Some(&raw));
        assert_eq!(
            (number(tag.get("Color")), number(tag.get("Sheared"))),
            (Some(14), Some(1))
        );
        let types: HashMap<String, Arc<MobType>> = data
            .types
            .iter()
            .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
            .collect();
        let back = mob_from_nbt(&tag, &types, 2).unwrap();
        assert_eq!((back.extra.color, back.extra.sheared), (14, true));
    }

    #[test]
    fn farm_animals_keep_their_variant_and_sound_variant() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let data = crate::mobs::MobTypes::load(&root).unwrap();
        let types: HashMap<String, Arc<MobType>> = data
            .types
            .iter()
            .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
            .collect();
        let mut m = Mob::new(
            1,
            types["minecraft:pig"].clone(),
            Vec3::new(0.0, 4.0, 0.0),
            0.0,
        );
        let tag = mob_to_nbt(&m, [1, 2, 3, 4], None);
        assert_eq!(
            text(tag.get("variant")).as_deref(),
            Some("minecraft:temperate")
        );
        assert_eq!(
            text(tag.get("sound_variant")).as_deref(),
            Some("minecraft:classic")
        );
        // Spawned in a warm biome with the mini sounds.
        (m.extra.variant, m.extra.sound_variant) = (1, Some(2));
        let tag = mob_to_nbt(&m, [1, 2, 3, 4], None);
        assert_eq!(text(tag.get("variant")).as_deref(), Some("minecraft:warm"));
        assert_eq!(
            text(tag.get("sound_variant")).as_deref(),
            Some("minecraft:mini")
        );
        let back = mob_from_nbt(&tag, &types, 2).unwrap();
        assert_eq!((back.extra.variant, back.extra.sound_variant), (1, Some(2)));
        let mut hen = Mob::new(
            3,
            types["minecraft:chicken"].clone(),
            Vec3::new(0.0, 4.0, 0.0),
            0.0,
        );
        (hen.extra.variant, hen.extra.egg_time) = (2, 7795);
        let tag = mob_to_nbt(&hen, [1, 2, 3, 4], None);
        assert_eq!(text(tag.get("variant")).as_deref(), Some("minecraft:cold"));
        let back = mob_from_nbt(&tag, &types, 4).unwrap();
        assert_eq!((back.extra.variant, back.extra.egg_time), (2, 7795));
        // The sound variants are indices into the registries finalizeSpawn draws from.
        use crate::mobs::{Species, finalize};
        let registries = finalize::SpawnRegistries::load(&root);
        for (species, registry) in [
            (Species::Pig, &registries.pig_sound),
            (Species::Cow, &registries.cow_sound),
            (Species::Chicken, &registries.chicken_sound),
            (Species::Cat, &registries.cat_sound),
            (Species::Wolf, &registries.wolf_sound),
        ] {
            assert_eq!(
                finalize::sound_variants(species),
                registry.names,
                "{species:?}"
            );
        }
    }

    #[test]
    fn foxes_rabbits_and_goats_keep_their_kind() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let data = crate::mobs::MobTypes::load(&root).unwrap();
        let types: HashMap<String, Arc<MobType>> = data
            .types
            .iter()
            .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
            .collect();
        let new = |kind: &str| Mob::new(1, types[kind].clone(), Vec3::new(0.0, 4.0, 0.0), 0.0);
        let round_trip = |m: &Mob| {
            let tag = mob_to_nbt(m, [1, 2, 3, 4], None);
            (mob_from_nbt(&tag, &types, 2).unwrap(), tag)
        };
        let mut fox = new("minecraft:fox");
        fox.extra.variant = 1;
        let (back, tag) = round_trip(&fox);
        assert_eq!(text(tag.get("Type")).as_deref(), Some("snow"));
        assert_eq!(number(tag.get("Sleeping")), Some(0));
        assert_eq!(back.extra.variant, 1);
        let mut rabbit = new("minecraft:rabbit");
        rabbit.extra.variant = 4;
        let (back, tag) = round_trip(&rabbit);
        assert_eq!(number(tag.get("RabbitType")), Some(4));
        assert_eq!(back.extra.variant, 4);
        let mut goat = new("minecraft:goat");
        (goat.extra.screaming, goat.extra.lost_horns) = (true, [false, true]);
        let (back, tag) = round_trip(&goat);
        assert_eq!(number(tag.get("HasRightHorn")), Some(0));
        assert_eq!(
            (back.extra.screaming, back.extra.lost_horns),
            (true, [false, true])
        );
        // A goat saved without horn flags has both, as vanilla reads it.
        let bare = Tag::compound([
            ("id", string("minecraft:goat")),
            ("Pos", doubles(&[0.5, 64.0, 0.5])),
        ]);
        assert_eq!(
            mob_from_nbt(&bare, &types, 3).unwrap().extra.lost_horns,
            [false; 2]
        );
    }
}

#[cfg(test)]
mod fireball_nbt_tests {
    use super::*;
    #[test]
    fn a_fireball_round_trips_its_flight() {
        let raw = Tag::compound([
            ("id", Tag::String("minecraft:fireball".into())),
            ("Pos", doubles(&[1.0, 70.0, -2.5])),
            ("Motion", doubles(&[0.0, -0.5, 1.0])),
            ("ExplosionPower", Tag::Byte(4)),
            ("acceleration_power", Tag::Double(0.0f64.to_bits())),
        ]);
        let p = fireball_from_nbt(&raw, None).unwrap();
        assert_eq!(p.kind, ProjectileKind::LargeFireball { power: 4 });
        assert_eq!((p.velocity.z, p.acceleration), (1.0, 0.0));
        let back = fireball_to_nbt(&p, [1, 2, 3, 4], None, None).unwrap();
        assert_eq!(number(back.get("ExplosionPower")), Some(4));
        assert_eq!(float(back.get("acceleration_power")), Some(0.0));
        // Projectile.addAdditionalSaveData: no LeftOwner unless set, HasBeenShot always.
        assert!(back.get("LeftOwner").is_none());
        assert_eq!(number(back.get("HasBeenShot")), Some(0));
        assert_eq!(
            read_doubles::<3>(back.get("Motion")),
            Some([0.0, -0.5, 1.0])
        );
        // A default fireball keeps vanilla's power and acceleration.
        let plain = Tag::compound([
            ("id", Tag::String("minecraft:fireball".into())),
            ("Pos", doubles(&[0.0, 0.0, 0.0])),
        ]);
        let p = fireball_from_nbt(&plain, None).unwrap();
        assert_eq!(
            (p.kind, p.acceleration),
            (ProjectileKind::LargeFireball { power: 1 }, 0.1)
        );
    }
}

#[cfg(test)]
mod neutral_anger_tests {
    use super::*;

    /// #2784: a neutral mob's anger survives the save: the anger ends at an
    /// absolute game time and the target comes back with it
    /// (`NeutralMob.addPersistentAngerSaveData` / `readPersistentAngerSaveData`).
    #[test]
    fn a_neutral_mobs_anger_survives_the_save() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let data = crate::mobs::MobTypes::load(&root).unwrap();
        let types: HashMap<String, Arc<MobType>> = data
            .types
            .iter()
            .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
            .collect();
        let mut m = Mob::new(
            1,
            types["minecraft:wolf"].clone(),
            Vec3::new(0.0, 64.0, 0.0),
            0.0,
        );
        m.extra.anger = 400;
        m.extra.anger_end = 3000;
        m.extra.angry_at = Some(crate::mobs::Who::Player);
        let tag = mob_to_nbt(&m, [1, 2, 3, 4], None);
        assert_eq!(number(tag.get("anger_end_time")), Some(3000));
        assert!(tag.get("angry_at").is_some());
        let back = mob_from_nbt(&tag, &types, 2).unwrap();
        assert_eq!(back.extra.anger_end, 3000);
        assert_eq!(back.extra.angry_at, Some(crate::mobs::Who::Player));
        // A mob that is no NeutralMob writes no anger.
        let pig = Mob::new(3, types["minecraft:pig"].clone(), Vec3::ZERO, 0.0);
        let tag = mob_to_nbt(&pig, [1, 2, 3, 5], None);
        assert!(tag.get("anger_end_time").is_none());
        // The anger that is left when the mob comes back.
        assert_eq!(crate::mobs::anger_left(3000, 1000), 2000);
        assert_eq!(crate::mobs::anger_left(3000, 4000), 0);
    }
}
