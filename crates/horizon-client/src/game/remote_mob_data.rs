//! A server's mobs as their synched data has them
//! (ClientPacketListener.handleSetEntityData, then each entity's
//! `onSyncedDataUpdated`): what a client draws from it rather than from its
//! own simulation, which a client of a server doesn't run.
use super::Game;
use horizon_client::mobs::render::{PROFESSIONS, VILLAGER_TYPES};
use horizon_client::mobs::{
    Mob, Species, Who, armadillo, bat, bee, camel, creaking, equine, finalize, illusioner, panda,
    sniffer, sync, tame, zombie_villager,
};
use horizon_client::particles::{ParticleOptions, ParticleType};
use horizon_protocol::entity_data::{Particle, ParticleData, Value};
use horizon_protocol::registry::{Builtin, Dynamic};
use horizon_world::registry::BlockStateId;

/// The values `apply` takes from one SetEntityData packet, by vanilla's entry
/// indices (Entity 0..=7, LivingEntity 8..=14, Mob 15, then the subclasses).
/// `builtin` names the ids of built-in registries.
fn apply(m: &mut Mob, values: &[(u8, i32, Value)], builtin: &Builtin, local_entity: i32) {
    let species = m.species();
    let baby = super::lan::baby_index(&m.kind.id);
    if &*m.kind.id == "minecraft:player" {
        player_poses(m, values);
    }
    for (index, _, value) in values {
        match (*index, value) {
            // Entity.DATA_POSE: a player's or mannequin's box follows it.
            (6, Value::Id(pose))
                if matches!(&*m.kind.id, "minecraft:player" | "minecraft:mannequin") =>
            {
                m.extra.avatar_pose = horizon_client::pose::Pose::from_id(*pose);
            }
            // Entity.DATA_SHARED_FLAGS_ID's FLAG_ONFIRE: Entity.isOnFire on a
            // client, which draws the flames. A client doesn't count the
            // fire down: the flag goes off when the server's fire is out.
            (0, Value::Byte(flags)) => {
                m.fire_ticks = i32::from(flags & 1);
                // Entity.isSprinting: a dashing cat's or ocelot's model.
                m.extra.sprinting = flags & 8 != 0;
                // Drowned.setSwimming: the shared flag of swimming.
                if species == Species::Drowned {
                    m.extra.flag = flags & 16 != 0;
                }
                // The shared flag of invisibility: Entity.isInvisible hides
                // the model (an illusioner shows its mirror images).
                let invisible = flags & 32 != 0;
                if species == Species::Illusioner {
                    illusioner::set_invisible(m, invisible);
                } else if &*m.kind.id != "minecraft:player" {
                    m.invisible = invisible;
                }
            }
            // Entity.DATA_POSE: the pose the species animates by.
            (6, Value::Id(pose)) => sync::set_pose(m, (*pose).clamp(0, 17) as u8),
            // LivingEntity.DATA_LIVING_ENTITY_FLAGS: using an item 1, in the
            // off hand 2. The flag starts and stops the use; the client ticks
            // the count itself (a bow's pull follows it).
            (8, Value::Byte(flags)) => {
                m.extra.using_off_hand = flags & 2 != 0;
                if flags & 1 == 0 {
                    m.extra.using_item = 0;
                } else if m.extra.using_item == 0 {
                    m.extra.using_item = 1;
                }
            }
            // LivingEntity.DATA_HEALTH_ID: the boss bars and the death
            // animation follow it.
            (9, Value::Float(health)) => m.health = *health,
            // LivingEntity.DATA_EFFECT_PARTICLES and DATA_EFFECT_AMBIENCE_ID:
            // the swirls of its effects (`tickEffects` draws them).
            (10, Value::Particles(list)) => m.extra.effect_particles = swirls(list, builtin),
            (11, Value::Bool(ambient)) => m.extra.effect_ambient = *ambient,
            // Mob.DATA_MOB_FLAGS_ID: no AI 1, left-handed 2, aggressive 4.
            // Mob.isAggressive draws the illagers' and skeletons' attack
            // poses and the vindicator's axe.
            (15, Value::Byte(flags)) => {
                m.no_ai = flags & 1 != 0;
                m.extra.left_handed = flags & 2 != 0;
                m.ai.aggressive = flags & 4 != 0;
            }
            (i, Value::Bool(b)) if Some(i) == baby => m.extra.age = if *b { -1 } else { 0 },
            _ => {}
        }
        match (species, *index, value) {
            (Species::Guardian | Species::ElderGuardian, 16, Value::Bool(moving)) => {
                m.extra.flag2 = *moving;
            }
            (Species::Guardian | Species::ElderGuardian, 17, Value::Int(target)) => {
                // Guardian.onSyncedDataUpdated resets the visual charge clock.
                m.extra.counter2 = 0;
                m.extra.flag = *target != 0;
                m.ai.target = (*target != 0).then_some(if *target == local_entity {
                    Who::Player
                } else {
                    Who::Mob(*target as u32)
                });
            }
            // Panda: the counters, the two genes and DATA_ID_FLAGS (after
            // AgeableMob's two entries).
            (Species::Panda, 18, Value::Int(n)) => panda::set_unhappy(m, *n),
            (Species::Panda, 19, Value::Int(n)) => panda::set_sneeze(m, *n),
            (Species::Panda, 20, Value::Int(n)) => panda::set_eat(m, *n),
            (Species::Panda, 21, Value::Byte(gene)) => m.extra.variant = (*gene).clamp(0, 6) as u8,
            (Species::Panda, 22, Value::Byte(gene)) => m.extra.variant2 = (*gene).clamp(0, 6) as u8,
            (Species::Panda, 23, Value::Byte(flags)) => panda::set_flags(m, *flags as u8),
            // PolarBear.DATA_STANDING_ID: the client animates the rearing.
            (Species::PolarBear, 18, Value::Bool(standing)) => m.extra.flag = *standing,
            // Bat.DATA_ID_FLAGS: resting in bit 0.
            (Species::Bat, 16, Value::Byte(flags)) => bat::set_resting(m, flags & 1 != 0),
            // Fox.DATA_TYPE_ID and DATA_FLAGS_ID.
            (Species::Fox, 18, Value::Int(variant)) => {
                m.extra.variant = (*variant).clamp(0, 1) as u8
            }
            (Species::Fox, 19, Value::Byte(flags)) => m.extra.ints[0] = i32::from(*flags as u8),
            // Sheep.DATA_WOOL_ID: the colour in the low nibble, 16 sheared.
            (Species::Sheep, 18, Value::Byte(wool)) => {
                m.extra.color = (*wool & 15) as u8;
                m.extra.sheared = wool & 16 != 0;
            }
            // SnowGolem.DATA_PUMPKIN_ID: 16 while the pumpkin is on.
            (Species::SnowGolem, 16, Value::Byte(flags)) => m.extra.sheared = flags & 16 == 0,
            // MushroomCow.DATA_TYPE: 0 red, 1 brown.
            (Species::Mooshroom, 17, Value::Int(variant)) => {
                m.extra.variant = (*variant == 1) as u8
            }
            // Piglin.DATA_IS_DANCING: the dance pose.
            (Species::Piglin, 19, Value::Bool(dancing)) => m.extra.scratch[0] = i32::from(*dancing),
            // TamableAnimal.DATA_FLAGS_ID: sitting 1, tame 4.
            (Species::Wolf | Species::Cat | Species::Parrot, 18, Value::Byte(flags)) => {
                m.extra.sitting = flags & 1 != 0;
                m.extra.tame = flags & 4 != 0;
            }
            // Cat.IS_LYING and RELAX_STATE_ONE.
            (Species::Cat, 21, Value::Bool(lying)) => m.extra.flag = *lying,
            (Species::Cat, 22, Value::Bool(relaxing)) => m.extra.relax_one = *relaxing,
            // Wolf.DATA_COLLAR_COLOR and Cat's.
            (Species::Wolf, 21, Value::Int(c)) | (Species::Cat, 23, Value::Int(c)) => {
                m.extra.color = (*c & 15) as u8;
            }
            // AbstractCubeMob.ID_SIZE: its size and box (refreshDimensions).
            (Species::Slime | Species::MagmaCube, 18, Value::Int(size)) => {
                m.extra.size = (*size).clamp(1, 127);
            }
            // Enderman.DATA_CREEPY (after DATA_CARRY_STATE): the open mouth
            // and raised head of an angry enderman.
            (Species::Enderman, 17, Value::Bool(creepy)) => m.extra.flag = *creepy,
            // Warden.CLIENT_ANGER_LEVEL: the heartbeat's pace.
            (Species::Warden, 16, Value::Int(anger)) => {
                if let Some(st) = m.extra.warden.as_mut() {
                    st.client_anger = *anger;
                }
            }
            // Sniffer.DATA_STATE (after AgeableMob's two): the animation
            // follows it in `sync::client_tick`.
            (Species::Sniffer, 18, Value::Id(state)) => {
                sniffer::set_state(m, (*state).clamp(0, 6) as u8);
            }
            // Armadillo.ARMADILLO_STATE (after AgeableMob's two): the shell and
            // its animations follow it in `sync::client_tick`.
            (Species::Armadillo, 18, Value::Id(state)) => {
                armadillo::set_state(m, (*state).clamp(0, 3) as u8);
            }
            // Goat.DATA_HAS_LEFT_HORN and DATA_HAS_RIGHT_HORN: the model hides
            // a horn the goat lost.
            (Species::Goat, 19, Value::Bool(has)) => m.extra.lost_horns[0] = !*has,
            (Species::Goat, 20, Value::Bool(has)) => m.extra.lost_horns[1] = !*has,
            // Shulker.DATA_ATTACH_FACE_ID, DATA_PEEK_ID and DATA_COLOR_ID.
            (Species::Shulker, 16, Value::Id(face)) => m.extra.variant3 = (*face).clamp(0, 5) as u8,
            (Species::Shulker, 17, Value::Byte(peek)) => m.extra.counter = i32::from(*peek),
            (Species::Shulker, 18, Value::Byte(color)) => m.extra.color = *color as u8,
            // Camel.DASH and LAST_POSE_CHANGE_TICK (after AbstractHorse's
            // flags).
            (Species::Camel | Species::CamelHusk, 19, Value::Bool(dashing)) => {
                camel::set_dashing(m, *dashing);
            }
            (Species::Camel | Species::CamelHusk, 20, Value::Long(tick)) => {
                camel::set_synced_pose_tick(m, *tick);
            }
            // Vex.DATA_FLAGS_ID: charging in bit 0.
            (Species::Vex, 16, Value::Byte(flags)) => m.extra.flag = flags & 1 != 0,
            // Creaking.CAN_MOVE, IS_ACTIVE and IS_TEARING_DOWN.
            (Species::Creaking, 16, Value::Bool(can_move)) => m.extra.flag2 = !*can_move,
            (Species::Creaking, 17, Value::Bool(active)) => m.extra.flag = *active,
            (Species::Creaking, 18, Value::Bool(true)) => creaking::set_tearing_down(m),
            // Allay.DATA_DANCING and DATA_CAN_DUPLICATE (a cooldown running
            // is any non-zero `egg_time`).
            (Species::Allay, 16, Value::Bool(dancing)) => m.extra.flag = *dancing,
            (Species::Allay, 17, Value::Bool(can)) => m.extra.egg_time = i32::from(!*can),
            // Axolotl.DATA_PLAYING_DEAD (after DATA_VARIANT): the factors
            // ease in `sync::client_tick`.
            (Species::Axolotl, 19, Value::Bool(dead)) => m.extra.axolotl.playing_dead = *dead,
            // CopperGolem.COPPER_GOLEM_STATE (after DATA_WEATHER_STATE).
            (Species::CopperGolem, 17, Value::Id(state)) => {
                m.extra.variant3 = (*state).clamp(0, 4) as u8;
            }
            // AbstractHorse's flags take 18. Horse.DATA_ID_TYPE_VARIANT: the
            // colour in the low byte, the markings in the next.
            (Species::Horse, 19, Value::Int(v)) => {
                m.extra.variant = (v & 0xFF).clamp(0, 6) as u8;
                m.extra.variant2 = ((v >> 8) & 0xFF).clamp(0, 4) as u8;
            }
            // AbstractChestedHorse.DATA_ID_CHEST, then Llama.DATA_STRENGTH_ID
            // and DATA_VARIANT_ID.
            (
                Species::Donkey | Species::Mule | Species::Llama | Species::TraderLlama,
                19,
                Value::Bool(chested),
            ) => m.extra.chested = *chested,
            (Species::Llama | Species::TraderLlama, 20, Value::Int(strength)) => {
                m.extra.variant2 = (*strength).clamp(1, 5) as u8;
            }
            (Species::Llama | Species::TraderLlama, 21, Value::Int(variant)) => {
                m.extra.variant = (*variant).clamp(0, 3) as u8;
            }
            // Axolotl.DATA_VARIANT (lucy 0 to blue 4) and Parrot.DATA_VARIANT_ID
            // (after TamableAnimal's flags and owner).
            (Species::Axolotl, 18, Value::Int(v)) => m.extra.variant = (*v).clamp(0, 4) as u8,
            (Species::Parrot, 20, Value::Int(v)) => m.extra.variant = (*v).clamp(0, 4) as u8,
            // Rabbit.DATA_TYPE_ID: the killer bunny's 99 has no model here.
            (Species::Rabbit, 18, Value::Int(t)) if (0..=5).contains(t) => {
                m.extra.variant = *t as u8;
            }
            // Bee.DATA_FLAGS_ID and DATA_ANGER_END_TIME (after Animal's).
            (Species::Bee, 18, Value::Byte(flags)) => bee::set_synced(m, Some(*flags as u8), None),
            (Species::Bee, 19, Value::Long(end)) => bee::set_synced(m, None, Some(*end)),
            // AbstractFish.FROM_BUCKET takes 16: Salmon.DATA_TYPE (small 0
            // to large 2), Pufferfish.PUFF_STATE and
            // TropicalFish.DATA_ID_TYPE_VARIANT.
            (Species::Salmon, 17, Value::Int(v)) => m.extra.variant = (*v).clamp(0, 2) as u8,
            (Species::Pufferfish, 17, Value::Int(state)) => m.extra.size = (*state).clamp(0, 2),
            (Species::TropicalFish, 17, Value::Int(v)) => tropical_fish(m, *v),
            // Ghast.DATA_IS_CHARGING and WitherBoss.DATA_ID_INV.
            (Species::Ghast, 16, Value::Bool(charging)) => m.extra.flag = *charging,
            (Species::Wither, 19, Value::Int(ticks)) => m.extra.counter = (*ticks).max(0),
            // CopperGolem.DATA_WEATHER_STATE: the oxidation the texture shows.
            (Species::CopperGolem, 16, Value::Id(state)) => {
                m.extra.variant = (*state).clamp(0, 3) as u8;
            }
            // AbstractHorse.DATA_ID_FLAGS: eating 16 and standing 32 (the
            // graze and the rear), which `sync::client_tick` animates.
            (s, 18, Value::Byte(flags)) if equine::grazes(s) => equine::set_flags(m, *flags as u8),
            // Enderman.DATA_CARRY_STATE (0 for none): the block it holds.
            (Species::Enderman, 16, Value::Id(state)) => {
                m.extra.carried = (*state > 0).then_some(BlockStateId(*state as u32));
            }
            // Zombie.DATA_DROWNED_CONVERSION_ID (after DATA_BABY_ID and
            // DATA_SPECIAL_TYPE_ID): the conversion's shaking.
            (Species::Zombie | Species::Husk, 18, Value::Bool(converting)) => {
                m.extra.counter2 = i32::from(*converting);
            }
            // Skeleton.DATA_STRAY_CONVERSION_ID (the first entry after Mob's):
            // the freezing conversion's shaking.
            (Species::Skeleton, 16, Value::Bool(converting)) => {
                m.extra.counter2 = i32::from(*converting);
            }
            // ZombieVillager.DATA_CONVERTING_ID (before its villager data).
            (Species::ZombieVillager, 19, Value::Bool(converting)) => {
                zombie_villager::set_converting(m, *converting);
            }
            // AbstractVillager.DATA_UNHAPPY_COUNTER (after AgeableMob's two):
            // the head shake of `no`, which the server counts down.
            (Species::Villager | Species::WanderingTrader, 18, Value::Int(ticks)) => {
                m.extra.ints[0] = *ticks;
            }
            // Hoglin.DATA_IMMUNE_TO_ZOMBIFICATION (after AgeableMob's two) and
            // AbstractPiglin's: `sync`'s conversion shake reads it.
            (Species::Hoglin, 18, Value::Bool(immune))
            | (Species::Piglin | Species::PiglinBrute, 16, Value::Bool(immune)) => {
                m.extra.flag2 = *immune;
            }
            // Blaze.DATA_FLAGS_ID: charged 1, which makes it burn.
            (Species::Blaze, 16, Value::Byte(flags)) => m.extra.flag = flags & 1 != 0,
            // Turtle.HAS_EGG and LAYING_EGG (after AgeableMob's two).
            (Species::Turtle, 18, Value::Bool(has_egg)) => m.extra.flag = *has_egg,
            (Species::Turtle, 19, Value::Bool(laying)) => m.extra.flag2 = *laying,
            // WitherBoss.DATA_TARGET_B and _C (after DATA_TARGET_A): the
            // entities the side heads turn toward in `wither::client_tick`.
            (Species::Wither, 17, Value::Int(id)) => m.extra.ints[0] = *id,
            (Species::Wither, 18, Value::Int(id)) => m.extra.ints[1] = *id,
            // Pillager.IS_CHARGING_CROSSBOW: the charge pose (Raider's
            // IS_CELEBRATING takes 16).
            (Species::Pillager, 17, Value::Bool(charging)) => m.extra.flag = *charging,
            // HappyGhast.STAYS_STILL (after AgeableMob's two and IS_LEASH_HOLDER):
            // `isOnStillTimeout` on a client, which keeps the ghast solid to
            // the player (`canBeCollidedWith`).
            (Species::HappyGhast, 19, Value::Bool(still)) => m.extra.counter = i32::from(*still),
            // SpellcasterIllager.DATA_SPELL_CASTING_ID: the casting pose.
            (Species::Evoker | Species::Illusioner, 17, Value::Byte(spell)) => {
                m.extra.variant2 = (*spell).max(0) as u8;
            }
            // Villager.DATA_VILLAGER_DATA and ZombieVillager's (after
            // Zombie's three): what VillagerProfessionLayer draws.
            (Species::Villager, 19, Value::Ids(data))
            | (Species::ZombieVillager, 20, Value::Ids(data)) => villager_data(m, data, builtin),
            // Mannequin.DATA_PROFILE (after Avatar's two): whose skin it
            // wears; update_remote_skins resolves it like a tab player's.
            (Species::Mannequin, 17, Value::Profile(profile)) => {
                m.profile = Some(profile.clone());
            }
            _ => {}
        }
    }
}

/// A tropical fish's packed variant: the pattern's id (the size, then the
/// index << 8), the base colour << 16 and the pattern colour << 24. Here the
/// pattern is 0 to 5 for the small ones and 6 to 11 for the large ones.
fn tropical_fish(m: &mut Mob, packed: i32) {
    let (size, index) = (packed & 0xFF, (packed >> 8) & 0xFF);
    // TropicalFish.Pattern.byId: an unknown id is a kob.
    m.extra.variant = if size <= 1 && index < 6 {
        (size * 6 + index) as u8
    } else {
        0
    };
    m.extra.variant2 = ((packed >> 16) & 0xFF) as u8 & 15;
    m.extra.variant3 = ((packed >> 24) & 0xFF) as u8 & 15;
}

/// The values of one SetEntityData packet that name an entry of a
/// synchronized registry (the variants of the cat, wolf, frog, pig, cow,
/// chicken and zombie nautilus) or count game time (a wolf's anger).
/// `game_time` is the client level's.
fn apply_registry_data(
    m: &mut Mob,
    values: &[(u8, i32, Value)],
    registries: &Dynamic,
    game_time: i64,
) {
    let species = m.species();
    for (index, _, value) in values {
        let registry = match (species, *index, value) {
            (Species::Cat, 20, Value::Id(_)) => "minecraft:cat_variant",
            (Species::Wolf, 23, Value::Id(_)) => "minecraft:wolf_variant",
            (Species::Frog, 18, Value::Id(_)) => "minecraft:frog_variant",
            (Species::Pig, 19, Value::Id(_)) => "minecraft:pig_variant",
            (Species::Cow, 18, Value::Id(_)) => "minecraft:cow_variant",
            (Species::Chicken, 18, Value::Id(_)) => "minecraft:chicken_variant",
            (Species::ZombieNautilus, 21, Value::Id(_)) => "minecraft:zombie_nautilus_variant",
            // Wolf.DATA_ANGER_END_TIME: the angry texture while it lies ahead.
            (Species::Wolf, 22, Value::Long(end)) => {
                m.extra.anger = (end - game_time).clamp(0, i64::from(i32::MAX)) as i32;
                continue;
            }
            _ => continue,
        };
        if let Value::Id(id) = value
            && let Some(name) = registries.name(registry, *id)
        {
            m.extra.variant = finalize::variant_index(species, name);
        }
    }
}

/// What another player's pose draws from: gliding (Entity.isFallFlying, flag
/// 0x80 of the shared flags), the pose (swimming), Riptide's spin
/// (LivingEntity.isAutoSpinAttack, flag 4) and a loaded crossbow in the
/// hand (the held stack comes with SetEquipment).
fn player_poses(m: &mut Mob, values: &[(u8, i32, Value)]) {
    for (index, _, value) in values {
        match (*index, value) {
            (0, Value::Byte(flags)) => m.extra.fall_flying = flags & -128 != 0,
            (6, Value::Id(pose)) => m.extra.synced_pose = (*pose).clamp(0, 17) as u8,
            (8, Value::Byte(flags)) => m.extra.auto_spin_attack = flags & 4 != 0,
            _ => {}
        }
    }
}

/// The options of a synched particle list: an entity effect carries its ARGB
/// colour, the omens none.
fn swirls(list: &[Particle], builtin: &Builtin) -> Vec<ParticleOptions> {
    let channel = |argb: i32, shift: u32| ((argb >> shift) & 255) as f32 / 255.0;
    list.iter()
        .filter_map(|p| {
            let name = builtin.get("minecraft:particle_type")?.name(p.id)?;
            let ty = ParticleType::from_name(name)?;
            Some(match p.data {
                Some(ParticleData::Color(argb)) => ParticleOptions::colored(
                    ty,
                    [
                        channel(argb, 16),
                        channel(argb, 8),
                        channel(argb, 0),
                        channel(argb, 24),
                    ],
                ),
                _ => ParticleOptions::simple(ty),
            })
        })
        .collect()
}

/// VillagerData as the network sends it: the type and the profession by
/// their built-in registry ids, then the level.
fn villager_data(m: &mut Mob, data: &[i32], builtin: &Builtin) {
    let &[kind, profession, level] = data else {
        return;
    };
    let index = |registry: &str, id: i32, names: &[&str]| {
        let name = builtin
            .get(registry)?
            .name(id)?
            .strip_prefix("minecraft:")?;
        names.iter().position(|n| *n == name).map(|i| i as u8)
    };
    if let Some(t) = index("minecraft:villager_type", kind, &VILLAGER_TYPES) {
        m.extra.variant = t;
    }
    if let Some(p) = index("minecraft:villager_profession", profession, &PROFESSIONS) {
        m.extra.variant2 = p;
    }
    m.extra.variant3 = level.clamp(0, i32::from(u8::MAX)) as u8;
}

impl Game<'_> {
    /// SetEntityData for a mob of the server's.
    pub(super) fn remote_mob_data(&mut self, id: i32, values: &[(u8, i32, Value)]) {
        let Some(r) = &self.remote else {
            return;
        };
        // LivingEntity.onSyncedDataUpdated(SLEEPING_POS_ID): a villager lies
        // in its bed the way the bed faces, or gets up.
        let bed = values.iter().find_map(|v| match v {
            (14, _, Value::OptPos(bed)) => Some(bed.map_or(0, |b| self.bed_facing(b))),
            _ => None,
        });
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id as u32) {
            apply(m, values, &r.builtin, r.entity_id);
            apply_registry_data(m, values, &r.registries, self.world.game_time);
            if let Some(code) = bed
                && m.species() == Species::Villager
            {
                m.extra.ints[1] = code;
            }
        }
    }
}

/// A mob as a client constructs it before its data arrives: the synched
/// defaults the server leaves out (`getNonDefaultValues`) that differ from
/// what `Mob::new` gives, such as a wolf's and a cat's red collar.
pub(super) fn client_defaults(m: &mut Mob) {
    if matches!(m.species(), Species::Wolf | Species::Cat) {
        m.extra.color = tame::RED;
    }
    if m.species() == Species::Bat {
        // Bat's constructor sets resting only on the server; the data default is 0.
        bat::set_resting(m, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_protocol::entity_data::serializer as s;

    /// A mob of the prepared resources' type `kind`, as a client spawns it
    /// (None without the resources).
    fn root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION)
    }

    /// The prepared resources' built-in registries (empty without them).
    fn builtin() -> Builtin {
        Builtin::load(&root().join("registries.json")).unwrap_or_default()
    }

    fn apply(m: &mut Mob, values: &[(u8, i32, Value)]) {
        super::apply(m, values, &builtin(), 99);
    }

    fn mob(kind: &str) -> Option<Mob> {
        let types = horizon_client::mobs::MobTypes::load(&root()).ok()?;
        let t = std::sync::Arc::new(types.types.get(kind)?.clone());
        let mut m = Mob::new(1, t, horizon_core::geometry::Vec3::ZERO, 0.0);
        client_defaults(&mut m);
        Some(m)
    }

    #[test]
    fn guardian_charge_follows_target_updates_and_caps_at_its_duration() {
        use horizon_client::mobs::guardian::{beam, client_tick};
        for (kind, duration) in [("guardian", 80), ("elder_guardian", 60)] {
            let Some(mut m) = mob(&format!("minecraft:{kind}")) else {
                return;
            };
            assert!(beam(&m, 0.0).is_none());
            apply(
                &mut m,
                &[
                    (16, s::BOOLEAN, Value::Bool(true)),
                    (17, s::INT, Value::Int(99)),
                ],
            );
            assert!(m.extra.flag2);
            assert_eq!(m.ai.target, Some(Who::Player));
            assert_eq!(beam(&m, 0.0), Some((0.0, 0.0)));
            for _ in 0..duration + 10 {
                client_tick(&mut m);
            }
            assert_eq!(beam(&m, 0.0), Some((duration as f32, 1.0)));
            // Another player or a mob retains its network entity id.
            apply(&mut m, &[(17, s::INT, Value::Int(42))]);
            assert_eq!(m.ai.target, Some(Who::Mob(42)));
            assert_eq!(m.extra.counter2, 0);
            client_tick(&mut m);
            assert_eq!(m.extra.counter2, 1);
            apply(
                &mut m,
                &[
                    (16, s::BOOLEAN, Value::Bool(false)),
                    (17, s::INT, Value::Int(0)),
                ],
            );
            client_tick(&mut m);
            assert!(!m.extra.flag2);
            assert_eq!(m.ai.target, None);
            assert_eq!(m.extra.counter2, 0);
            assert!(beam(&m, 0.0).is_none());
        }
    }

    #[test]
    fn a_sheep_takes_its_wool_and_a_calf_its_size() {
        let Some(mut m) = mob("minecraft:sheep") else {
            return;
        };
        apply(&mut m, &[(18, s::BYTE, Value::Byte(14 | 16))]);
        assert_eq!((m.extra.color, m.extra.sheared), (14, true));
        apply(&mut m, &[(18, s::BYTE, Value::Byte(0))]);
        assert_eq!((m.extra.color, m.extra.sheared), (0, false));
        let mut calf = mob("minecraft:cow").unwrap();
        apply(&mut calf, &[(16, s::BOOLEAN, Value::Bool(true))]);
        assert!(calf.is_baby());
        apply(&mut calf, &[(16, s::BOOLEAN, Value::Bool(false))]);
        assert!(!calf.is_baby());
    }

    #[test]
    fn bat_flags_select_continuous_client_clocks_without_changing_motion() {
        use horizon_client::mobs::{MobTypes, anim, keyframes::Animation};
        use horizon_core::geometry::Vec3;
        use serde_json::json;
        use std::{collections::HashMap, fs, sync::Arc};

        // Original minimal resources keep this regression runnable without vanilla data.
        let root =
            std::env::temp_dir().join(format!("horizon-bat-animation-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("entity-types.json"), r#"{"minecraft:bat": {}}"#).unwrap();
        fs::write(
            root.join("entity-models.json"),
            r#"{"minecraft:bat#main": {"children": {"right_wing": {}, "head": {}}}}"#,
        )
        .unwrap();
        let mut types = MobTypes::load(&root).unwrap();
        fs::remove_dir_all(root).unwrap();
        let position = Vec3::new(1.0, 2.25, 3.0);
        let velocity = Vec3::new(0.2, 0.3, 0.4);
        let mut m = Mob::new(
            1,
            Arc::new(types.types.remove("minecraft:bat").unwrap()),
            position,
            0.0,
        );
        assert!(bat::resting(&m));
        client_defaults(&mut m);
        assert!(!bat::resting(&m));
        m.velocity = velocity;
        m.age = 1;
        bat::setup_animation_states(&mut m);
        m.age = 4;
        apply(&mut m, &[(16, s::BYTE, Value::Byte(2))]);
        bat::setup_animation_states(&mut m);
        assert_eq!(m.anim.states[0].started, Some(1.0));

        let curve = Animation::parse(&json!({"length": 1, "looping": true,
            "bones": {"right_wing": [{"target": "rotation", "keyframes":
                [[0,[0,0,0],[0,0,0],"linear"],[1,[0,1,0],[0,1,0],"linear"]]}]}}));
        let animations = HashMap::from([("BatAnimation.BAT_FLYING".into(), curve)]);
        let wing = |partial| {
            let mut poses = anim::Poses::new(&m.kind.model);
            anim::setup(&m, &anim::state(&m, partial), &mut poses, &animations);
            poses.get("right_wing")[4]
        };
        assert!(wing(0.5) > wing(0.0));

        m.age = 5;
        apply(&mut m, &[(16, s::BYTE, Value::Byte(3))]);
        bat::setup_animation_states(&mut m);
        assert!(bat::resting(&m));
        assert_eq!(m.anim.states[0].started, None);
        assert_eq!(m.anim.states[1].started, Some(5.0));
        m.age = 7;
        apply(&mut m, &[(16, s::BYTE, Value::Byte(0))]);
        bat::setup_animation_states(&mut m);
        assert_eq!(m.anim.states[0].started, Some(7.0));
        assert_eq!(m.anim.states[1].started, None);
        assert_eq!((m.position, m.velocity), (position, velocity));
    }

    #[test]
    fn a_wolf_takes_its_owner_flags_and_collar() {
        let Some(mut m) = mob("minecraft:wolf") else {
            return;
        };
        assert_eq!(m.extra.color, tame::RED);
        apply(
            &mut m,
            &[
                (18, s::BYTE, Value::Byte(4 | 1)),
                (21, s::INT, Value::Int(11)),
            ],
        );
        assert!(m.extra.tame && m.extra.sitting);
        assert_eq!(m.extra.color, 11);
    }

    #[test]
    fn fire_health_and_size_follow_the_server() {
        let Some(mut m) = mob("minecraft:slime") else {
            return;
        };
        apply(
            &mut m,
            &[
                (0, s::BYTE, Value::Byte(1)),
                (9, s::FLOAT, Value::Float(12.5)),
                (18, s::INT, Value::Int(4)),
            ],
        );
        assert_eq!((m.fire_ticks, m.health, m.extra.size), (1, 12.5, 4));
        apply(&mut m, &[(0, s::BYTE, Value::Byte(0))]);
        assert_eq!(m.fire_ticks, 0);
    }

    #[test]
    fn mob_flags_turn_aggression_on_and_off() {
        let Some(mut m) = mob("minecraft:vindicator") else {
            return;
        };
        apply(&mut m, &[(15, s::BYTE, Value::Byte(4 | 2))]);
        assert!(m.ai.aggressive && m.extra.left_handed && !m.no_ai);
        assert!(horizon_client::mobs::illager::shows_item(&m));
        apply(&mut m, &[(15, s::BYTE, Value::Byte(0))]);
        assert!(!m.ai.aggressive && !m.extra.left_handed);
        assert!(!horizon_client::mobs::illager::shows_item(&m));
    }

    #[test]
    fn a_villager_takes_its_type_profession_and_level() {
        let Some(mut m) = mob("minecraft:villager") else {
            return;
        };
        let registries = builtin();
        let id = |registry: &str, name: &str| registries.get(registry)?.id(name);
        let (Some(desert), Some(farmer), Some(librarian)) = (
            id("minecraft:villager_type", "minecraft:desert"),
            id("minecraft:villager_profession", "minecraft:farmer"),
            id("minecraft:villager_profession", "minecraft:librarian"),
        ) else {
            return;
        };
        let name = |m: &Mob| {
            (
                VILLAGER_TYPES[m.extra.variant as usize],
                PROFESSIONS[m.extra.variant2 as usize],
                m.extra.variant3,
            )
        };
        apply(
            &mut m,
            &[(19, s::VILLAGER_DATA, Value::Ids(vec![desert, farmer, 2]))],
        );
        assert_eq!(name(&m), ("desert", "farmer", 2));
        // A job taken later: only the profession and the level change.
        apply(
            &mut m,
            &[(19, s::VILLAGER_DATA, Value::Ids(vec![desert, librarian, 5]))],
        );
        assert_eq!(name(&m), ("desert", "librarian", 5));
        // A zombie villager's data comes after Zombie's three entries.
        let mut z = mob("minecraft:zombie_villager").unwrap();
        apply(
            &mut z,
            &[(20, s::VILLAGER_DATA, Value::Ids(vec![desert, farmer, 3]))],
        );
        assert_eq!(name(&z), ("desert", "farmer", 3));
    }

    #[test]
    fn the_mob_flags_drive_the_aggressive_poses() {
        let Some(mut m) = mob("minecraft:skeleton") else {
            return;
        };
        apply(&mut m, &[(15, s::BYTE, Value::Byte(2 | 4))]);
        assert!(m.ai.aggressive && m.extra.left_handed && !m.no_ai);
        apply(&mut m, &[(15, s::BYTE, Value::Byte(0))]);
        assert!(!m.ai.aggressive && !m.extra.left_handed);
    }

    #[test]
    fn the_living_flags_start_and_stop_a_mob_using_an_item() {
        let Some(mut m) = mob("minecraft:skeleton") else {
            return;
        };
        apply(&mut m, &[(8, s::BYTE, Value::Byte(1))]);
        assert_eq!(m.extra.using_item, 1);
        assert!(!m.extra.using_off_hand);
        // Using in the off hand.
        apply(&mut m, &[(8, s::BYTE, Value::Byte(3))]);
        assert!(m.extra.using_off_hand);
        apply(&mut m, &[(8, s::BYTE, Value::Byte(0))]);
        assert_eq!(m.extra.using_item, 0);
    }

    #[test]
    fn a_pillager_charges_and_a_spellcaster_casts() {
        let Some(mut p) = mob("minecraft:pillager") else {
            return;
        };
        apply(&mut p, &[(17, s::BOOLEAN, Value::Bool(true))]);
        assert!(p.extra.flag);
        let mut e = mob("minecraft:evoker").unwrap();
        apply(&mut e, &[(17, s::BYTE, Value::Byte(2))]);
        assert_eq!(e.extra.variant2, 2);
    }

    #[test]
    fn a_mannequin_exists_and_takes_its_profile() {
        let Some(mut m) = mob("minecraft:mannequin") else {
            return;
        };
        assert_eq!(&*m.kind.id, "minecraft:mannequin");
        let profile = horizon_protocol::entity_data::Profile {
            name: Some("Gommunity".into()),
            uuid: Some(1),
            ..Default::default()
        };
        apply(
            &mut m,
            &[(17, s::RESOLVABLE_PROFILE, Value::Profile(profile.clone()))],
        );
        assert_eq!(m.profile, Some(profile));
    }
}
