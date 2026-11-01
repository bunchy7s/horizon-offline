//! `Mob.finalizeSpawn(level, level.getCurrentDifficultyAt(pos), STRUCTURE, null)`
//! of the mobs structures place (StructureTemplate.placeEntities with
//! finalizeEntities, ocean ruin drowned, mansion illagers, witch huts,
//! monuments), run as decoration places them, with the WorldGenRegion's random.
use crate::{
    enchant::Enchantments,
    entity_nbt,
    items::Items,
    mobs::{
        Mob, MobType, MobTypes,
        finalize::{self, Ctx, FinalizeLevel, Group, Reason, SpawnData, SpawnRegistries},
        spawning::{LocalDifficulty, MOON_BRIGHTNESS},
    },
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::{LegacyRandom, RandomSource, XoroshiroRandom},
};
use horizon_world::{
    nbt::{Compound, Tag},
    worldgen::{generator::StructureMobs, structure::legacy::PieceRandom},
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicI64, AtomicU8, Ordering},
    },
};

struct Data {
    types: HashMap<String, Arc<MobType>>,
    biome_tags: HashMap<String, HashSet<String>>,
    registries: SpawnRegistries,
    enchantments: Enchantments,
    items: Items,
    camel_husk: (f32, f32),
}
fn load(root: &std::path::Path) -> Option<Data> {
    let types = MobTypes::load(root).ok()?.types;
    let camel = types.get("minecraft:camel_husk")?;
    let camel_husk = (camel.width as f32, camel.height as f32);
    Some(Data {
        types: types.into_iter().map(|(k, t)| (k, Arc::new(t))).collect(),
        biome_tags: crate::items::load_tags(
            root,
            "worldgen/biome",
            &horizon_world::loot::PackOverlay::NONE,
        )
        .unwrap_or_default(),
        registries: SpawnRegistries::load(root),
        enchantments: Enchantments::load(root),
        items: Items::tags_only(root, &horizon_world::loot::PackOverlay::NONE).ok()?,
        camel_husk,
    })
}

pub struct StructureMobFinalizer {
    root: PathBuf,
    dimension: String,
    data: OnceLock<Option<Data>>,
    /// The level's difficulty (0 peaceful to 3 hard) and overworld clock.
    difficulty: AtomicU8,
    day_time: AtomicI64,
}
impl StructureMobFinalizer {
    pub fn new(root: PathBuf, dimension: &str) -> Self {
        Self {
            root,
            dimension: dimension.to_owned(),
            data: OnceLock::new(),
            difficulty: AtomicU8::new(2),
            day_time: AtomicI64::new(0),
        }
    }
    pub fn set_conditions(&self, difficulty: u8, day_time: i64) {
        self.difficulty.store(difficulty, Ordering::Relaxed);
        self.day_time.store(day_time, Ordering::Relaxed);
    }
}

struct Level<'a> {
    data: &'a Data,
    biome: &'a str,
    structures: &'a [String],
    difficulty: u8,
    day_time: i64,
    next_id: u32,
}
impl FinalizeLevel for Level<'_> {
    fn biome(&self, _: [i32; 3]) -> String {
        self.biome.to_owned()
    }
    fn biome_tag(&self, tag: &str, _: [i32; 3]) -> bool {
        self.data
            .biome_tags
            .get(tag)
            .is_some_and(|b| b.contains(self.biome))
    }
    fn structures_with_piece_at(&self, _: [i32; 3]) -> Vec<String> {
        self.structures.to_vec()
    }
    fn moon_phase(&self, _: [i32; 3]) -> usize {
        self.day_time.div_euclid(24000).rem_euclid(8) as usize
    }
    fn spawning_monsters(&self) -> bool {
        self.difficulty != 0
    }
    // No block collision: only a husk's camel asks, and no structure places husks.
    fn no_collision(&self, _: Aabb) -> bool {
        true
    }
    // WorldGenRegion holds no entities.
    fn free_chicken(&self, _: Aabb) -> Option<u32> {
        None
    }
    fn ride_chicken(&mut self, _: u32, _: u32) {}
    fn game_time(&self) -> i64 {
        self.day_time
    }
    fn halloween(&self) -> bool {
        finalize::is_halloween()
    }
    fn create(&mut self, kind: &str, _: Reason) -> Option<Mob> {
        if self.difficulty == 0 && !finalize::allowed_in_peaceful(kind) {
            return None;
        }
        let t = self.data.types.get(kind)?.clone();
        self.next_id += 1;
        Some(Mob::new(self.next_id, t, Vec3::ZERO, 0.0))
    }
}

fn run<R: RandomSource>(
    level: &mut Level<'_>,
    random: &mut R,
    difficulty: LocalDifficulty,
    data: &SpawnData<'_>,
    m: &mut Mob,
    reason: Reason,
    group: &mut Option<Group>,
) -> Vec<Mob> {
    let mut cx = Ctx {
        level,
        random,
        difficulty,
        data,
        created: Vec::new(),
        jockey_rider: None,
    };
    *group = cx.finalize(m, reason, group.take());
    if let Some(rider) = cx.jockey_rider {
        debug_assert_eq!(rider.id, m.id);
        *m = rider;
    }
    cx.created
}

/// The saved compound of mob `i` with the mobs riding it under `Passengers`.
fn tree(mobs: &[(Mob, Option<Tag>)], i: usize, dimension: &str) -> Compound {
    let (m, raw) = &mobs[i];
    // No block registry here: a carried block stays as the template had it.
    let Tag::Compound(mut c) = entity_nbt::mob_to_nbt(m, [0; 4], raw.as_ref(), None) else {
        return Compound::new();
    };
    c.remove(&"UUID".into());
    // Keys in vanilla's templates that no entity reads: the chunk keeps the
    // entity as its save writes it (ProtoChunk.addEntity), without them.
    for key in [
        "CatType",
        "OwnerUUID",
        "Riches",
        "Willing",
        "food_level",
        "lastRestock",
    ] {
        c.remove(&key.into());
    }
    entity_nbt::put_brain(&mut c, m, dimension);
    let riders: Vec<Tag> = (0..mobs.len())
        .filter(|&j| mobs[j].0.extra.mount == Some(m.id))
        .map(|j| Tag::Compound(tree(mobs, j, dimension)))
        .collect();
    if !riders.is_empty() {
        c.insert("Passengers".into(), Tag::List(riders));
    }
    c
}

impl StructureMobs for StructureMobFinalizer {
    fn finalize(
        &self,
        nbt: &mut Compound,
        biome: &str,
        structures: &[String],
        random: &mut dyn PieceRandom,
    ) -> bool {
        let Some(data) = self.data() else {
            return false;
        };
        let mut raw = Tag::Compound(nbt.clone());
        if let Tag::Compound(c) = &mut raw {
            c.remove(&"horizon:finalize".into());
        }
        let Some(m) = entity_nbt::mob_from_nbt(&raw, &data.types, 1, None) else {
            // Not a mob: vanilla finalizes only Mob instances.
            return true;
        };
        let reason = Reason::Structure;
        match self.finish(
            data,
            m,
            Some(raw),
            biome,
            structures,
            random,
            reason,
            &mut None,
        ) {
            Some(saved) => {
                *nbt = saved;
                true
            }
            None => false,
        }
    }
}

impl StructureMobFinalizer {
    fn data(&self) -> Option<&Data> {
        self.data
            .get_or_init(|| {
                let data = load(&self.root);
                if data.is_none() {
                    log::error!("Horizon: no mob data, structure mobs are finalized when loaded");
                }
                data
            })
            .as_ref()
    }
    /// A mob chunk generation spawns (`spawnOriginalMobs`): a new one of
    /// `kind` finalized as a structure mob is, as the chunk saves it; `group`
    /// carries a pack's group data from one member to the next.
    pub fn spawn_generated(
        &self,
        kind: &str,
        position: Vec3,
        yaw: f32,
        biome: &str,
        random: &mut dyn PieceRandom,
        group: &mut Option<Group>,
    ) -> Option<Compound> {
        let data = self.data()?;
        let m = Mob::new(1, data.types.get(kind)?.clone(), position, yaw);
        let reason = Reason::ChunkGeneration;
        self.finish(data, m, None, biome, &[], random, reason, group)
    }
    #[allow(clippy::too_many_arguments)]
    fn finish(
        &self,
        data: &Data,
        mut m: Mob,
        raw: Option<Tag>,
        biome: &str,
        structures: &[String],
        random: &mut dyn PieceRandom,
        reason: Reason,
        group: &mut Option<Group>,
    ) -> Option<Compound> {
        let base = self.difficulty.load(Ordering::Relaxed);
        let day_time = self.day_time.load(Ordering::Relaxed);
        let mut level = Level {
            data,
            biome,
            structures,
            difficulty: base,
            day_time,
            next_id: 1,
        };
        let moon = MOON_BRIGHTNESS[level.moon_phase([0; 3])];
        // WorldGenRegion.getCurrentDifficultyAt: no inhabited time.
        let difficulty = LocalDifficulty::new(base, day_time, 0, moon);
        let spawn = SpawnData {
            registries: &data.registries,
            enchantments: &data.enchantments,
            items: &data.items,
            camel_husk_size: data.camel_husk,
        };
        let any = random.as_any();
        let created = if let Some(r) = any.downcast_mut::<LegacyRandom>() {
            run(&mut level, r, difficulty, &spawn, &mut m, reason, group)
        } else if let Some(r) = any.downcast_mut::<XoroshiroRandom>() {
            run(&mut level, r, difficulty, &spawn, &mut m, reason, group)
        } else {
            log::error!("Horizon: unknown region random, structure mobs are finalized when loaded");
            return None;
        };
        let mut mobs = vec![(m, raw)];
        mobs.extend(created.into_iter().map(|c| (c, None)));
        let root = mobs
            .iter()
            .position(|(m, _)| m.extra.mount.is_none())
            .unwrap_or(0);
        Some(tree(&mobs, root, &self.dimension))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bastion's crossbow piglin keeps its crossbow (STRUCTURE skips
    /// createSpawnWeapon) and saves what Piglin.addAdditionalSaveData writes.
    #[test]
    fn bastion_crossbow_piglin_finalizes_as_structure() {
        let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES").map(PathBuf::from) else {
            return;
        };
        let file = root.join("worldgen/structure/minecraft/bastion/mobs/crossbow_piglin.nbt");
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(
            &mut flate2::read::GzDecoder::new(std::fs::File::open(file).unwrap()),
            &mut bytes,
        )
        .unwrap();
        let limits = horizon_world::nbt::Limits::templates();
        let template = horizon_world::nbt::read_named(&bytes, limits)
            .unwrap()
            .0
            .tag;
        let Some(Tag::List(entities)) = template.get("entities") else {
            panic!("no entities");
        };
        let Some(Tag::Compound(mut nbt)) = entities[0].get("nbt").cloned() else {
            panic!("no nbt");
        };
        nbt.remove(&"UUID".into());
        let hook = StructureMobFinalizer::new(root, "minecraft:the_nether");
        let mut random = LegacyRandom::new(12345);
        hook.finalize(&mut nbt, "minecraft:nether_wastes", &[], &mut random);
        let keys: Vec<String> = {
            let mut k: Vec<String> = nbt.keys().map(|k| k.to_string_lossy()).collect();
            k.sort();
            k
        };
        assert_eq!(
            keys,
            [
                "AbsorptionAmount",
                "Air",
                "Brain",
                "CanPickUpLoot",
                "CannotHunt",
                "DeathTime",
                "FallFlying",
                "Fire",
                "Health",
                "HurtTime",
                "Inventory",
                "Invulnerable",
                "IsBaby",
                "IsImmuneToZombification",
                "LeftHanded",
                "Motion",
                "OnGround",
                "PersistenceRequired",
                "PortalCooldown",
                "Pos",
                "Rotation",
                "TimeInOverworld",
                "attributes",
                "current_impulse_context_reset_grace_time",
                "equipment",
                "fall_distance",
                "id",
            ]
        );
        let tag = Tag::Compound(nbt);
        let held = tag.get("equipment").and_then(|e| e.get("mainhand"));
        assert!(
            matches!(held.and_then(|h| h.get("id")), Some(Tag::String(s)) if s.to_string_lossy() == "minecraft:crossbow")
        );
        let hunted = tag
            .get("Brain")
            .and_then(|b| b.get("memories"))
            .and_then(|m| m.get("minecraft:hunted_recently"));
        assert!(matches!(
            hunted.and_then(|h| h.get("ttl")),
            Some(Tag::Long(600..=2400))
        ));
        let Some(Tag::List(attributes)) = tag.get("attributes") else {
            panic!("no attributes");
        };
        assert!(attributes.iter().any(|a| {
            matches!(a.get("id"), Some(Tag::String(s)) if s.to_string_lossy() == "minecraft:follow_range")
                && matches!(a.get("modifiers"), Some(Tag::List(m)) if m.len() == 1)
        }));
    }

    fn id(t: Option<&Tag>) -> String {
        match t.and_then(|t| t.get("id")) {
            Some(Tag::String(s)) => s.to_string_lossy().trim_start_matches("minecraft:").into(),
            _ => "-".into(),
        }
    }
    /// What a vanilla world saves of a structure mob that finalizeSpawn decides.
    fn describe(mob: &Tag) -> String {
        let eq = mob.get("equipment");
        let gear: Vec<String> = ["mainhand", "head", "chest", "legs", "feet"]
            .iter()
            .map(|slot| id(eq.and_then(|e| e.get(slot))))
            .collect();
        let ttl = match mob
            .get("Brain")
            .and_then(|b| b.get("memories"))
            .and_then(|m| m.get("minecraft:hunted_recently"))
            .and_then(|h| h.get("ttl"))
        {
            Some(Tag::Long(t)) => t.to_string(),
            _ => "-".into(),
        };
        let bonus = crate::entity_nbt::spawn_bonus(mob).unwrap_or(f64::NAN);
        let flag = |k: &str| matches!(mob.get(k), Some(Tag::Byte(1)));
        let riders = match mob.get("Passengers") {
            Some(Tag::List(p)) => format!(
                " [{}]",
                p.iter().map(describe).collect::<Vec<_>>().join("; ")
            ),
            _ => String::new(),
        };
        format!(
            "{} {} ttl={ttl} bonus={bonus:.6} left={} baby={}{riders}",
            id(Some(mob)),
            gear.join(","),
            flag("LeftHanded"),
            flag("IsBaby") || matches!(mob.get("Age"), Some(Tag::Int(a)) if *a < 0),
        )
    }
    /// The described entities of chunk (x, z) after decorating the 3x3 around it.
    fn chunk_mobs(
        root: &std::path::Path,
        seed: i64,
        dimension: &str,
        x: i32,
        z: i32,
    ) -> Vec<String> {
        use crate::streaming::{ChunkGenerator, Proto};
        let assets = crate::assets::Assets::load(root).unwrap();
        let biomes = crate::biome::Biomes::load(root).unwrap();
        let terrain = super::super::VanillaTerrain::for_dimension(
            &root.join("worldgen"),
            assets.registry,
            &biomes,
            seed,
            dimension,
        )
        .unwrap();
        let mut worker = terrain.worker();
        let mut region: [Proto; 9] = std::array::from_fn(|i| {
            terrain.proto(&mut worker, x + i as i32 % 3 - 1, z + i as i32 / 3 - 1)
        });
        terrain.decorate(&mut worker, &mut region);
        let center = std::mem::replace(&mut region[4], Box::new(()));
        let mut got: Vec<String> = terrain
            .finish(center)
            .entities
            .iter()
            .map(describe)
            .collect();
        got.sort();
        got
    }
    /// Seed 12345's bastion at chunk (-21, 8): the mobs of chunk (-23, 8) as two
    /// vanilla 26.3 runs saved them (weapons and armour, hunting memory, follow
    /// range bonus, handedness), from the region random of that chunk. The
    /// memories are as placed: vanilla saved them 13 brain ticks later.
    #[test]
    fn bastion_chunk_mobs_match_vanilla() {
        let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES").map(PathBuf::from) else {
            return;
        };
        let got = chunk_mobs(&root, 12345, "minecraft:the_nether", -23, 8);
        let mut expected = [
            "piglin golden_sword,-,golden_chestplate,-,- ttl=1760 bonus=0.013784",
            "piglin crossbow,-,-,-,- ttl=806 bonus=-0.011458",
            "piglin golden_sword,-,-,-,- ttl=1315 bonus=-0.084585",
            "piglin golden_sword,-,-,-,- ttl=1489 bonus=-0.038292",
            "piglin_brute golden_axe,-,-,-,- ttl=- bonus=-0.020574",
            "piglin crossbow,-,-,-,- ttl=1620 bonus=-0.079135",
            "hoglin -,-,-,-,- ttl=- bonus=-0.012052",
            "piglin golden_sword,-,-,-,- ttl=783 bonus=-0.077071",
            "piglin crossbow,-,-,golden_leggings,- ttl=1431 bonus=0.007058",
        ]
        .map(|e| format!("{e} left=false baby=false"));
        expected.sort();
        assert_eq!(got, expected);
    }

    /// Seed 12345's ocean ruin at (53, 39, 424): Drowned.finalizeSpawn mounts the
    /// trident drowned on a zombie nautilus, as vanilla saved chunk (3, 26).
    #[test]
    fn ocean_ruin_drowned_match_vanilla() {
        let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES").map(PathBuf::from) else {
            return;
        };
        let got = chunk_mobs(&root, 12345, "minecraft:overworld", 3, 26);
        let d = |gear: &str, bonus: &str| {
            format!("drowned {gear},-,-,-,- ttl=- bonus={bonus} left=false baby=false")
        };
        let mut expected = vec![
            d("-", "0.001996"),
            format!(
                "zombie_nautilus -,-,-,-,- ttl=- bonus=-0.044983 left=false baby=false [{}]",
                d("trident", "-0.045604")
            ),
            d("fishing_rod", "-0.059079"),
            d("-", "0.021658"),
        ];
        expected.sort();
        assert_eq!(got, expected);
    }
}
