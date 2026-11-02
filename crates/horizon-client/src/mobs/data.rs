//! Facts the mob AI and renderer read from vanilla code, exported by
//! `VanillaMobDataExporter` into `mob-data.json`: per-state pathfinding types
//! (`WalkNodeEvaluator.getPathTypeFromState` with vanilla's tags bound),
//! pathfindability, spawn-surface predicates, collision tops, each entity
//! type's spawn placement and the client's keyframe animation definitions.
use super::keyframes::Animation;
use super::path::PathType;
use crate::blocks::{BlockFacts, Fluid};
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

pub const LAND: u16 = 1;
pub const WATER: u16 = 2;
pub const AIR: u16 = 4;
/// `isValidSpawn` for a typical monster (zombie).
pub const VALID_SPAWN: u16 = 8;
/// `isValidSpawn` for ocelots/parrots (leaves allowed).
pub const VALID_SPAWN_LEAVES: u16 = 16;
/// `isValidSpawn` for polar bears (ice allowed).
pub const VALID_SPAWN_ICE: u16 = 32;
pub const REDSTONE_CONDUCTOR: u16 = 64;
/// `NaturalSpawner.isValidEmptySpawnBlock` for a zombie.
pub const VALID_EMPTY_SPAWN: u16 = 128;
pub const STURDY_TOP: u16 = 256;
/// BlockState.isSolid
pub const SOLID: u16 = 512;
pub const SOLID_RENDER: u16 = 1024;

#[derive(Clone, Copy, Debug)]
pub struct StateData {
    pub path: PathType,
    pub flags: u16,
    /// Collision shape top (0 when the shape is empty).
    pub top: f32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    OnGround,
    InWater,
    InLava,
    NoRestrictions,
}
pub struct MobData {
    pub states: Vec<StateData>,
    pub placements: HashMap<String, Placement>,
    /// Each entity type's `SpawnPlacements.getHeightmapType`.
    pub heightmaps: HashMap<String, String>,
    pub animations: HashMap<String, Animation>,
    /// Replaced when the resource packs change (the rest of `MobData` never is).
    pub villager_hats: std::sync::RwLock<super::villager_hats::VillagerHats>,
    /// The packs' custom entity models (OptiFine CEM); replaced when the resource packs change.
    pub cem: std::sync::RwLock<std::sync::Arc<super::cem::CemModels>>,
    /// The packs' random entity textures, replaced with them too.
    pub random_textures: std::sync::RwLock<horizon_optifine::random_textures::RandomTextures>,
    /// The packs' emissive overlays of entity textures, replaced with them too.
    pub emissive: std::sync::RwLock<std::sync::Arc<horizon_optifine::emissive::Emissive>>,
    /// ETF's blinking textures, replaced with the packs too.
    pub blinks: std::sync::RwLock<horizon_optifine::blink::Blinks>,
    /// Biome tags (spawns_warm_variant_farm_animals, is_ocean...) by tag id.
    pub biome_tags: HashMap<String, std::collections::HashSet<String>>,
    /// Entity type tags (`data/*/tags/entity_type`).
    pub entity_tags: HashMap<String, std::collections::HashSet<String>>,
    /// The villager POI type of every block state (`PoiTypes.forState`),
    /// worked out the first time a villager looks for one.
    pub pois: std::sync::OnceLock<Vec<Option<super::villager_brain::Poi>>>,
    /// [`MobData::placement`] of each species' entity type (by `Species as
    /// usize`), worked out the first time a spawn asks.
    pub species_placements: std::sync::OnceLock<Vec<Placement>>,
}
impl MobData {
    pub fn load(root: &Path, facts: &BlockFacts) -> Self {
        let parsed: Option<Value> = fs::read(root.join("mob-data.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let mut data = match parsed {
            Some(v)
                if v["states"]
                    .as_array()
                    .is_some_and(|a| a.len() == facts.states.len()) =>
            {
                Self::from_json(&v)
            }
            _ => Self::fallback(facts),
        };
        data.biome_tags = crate::items::load_tags(
            root,
            "worldgen/biome",
            &horizon_world::loot::PackOverlay::NONE,
        )
        .unwrap_or_default();
        data.entity_tags =
            crate::items::load_tags(root, "entity_type", &horizon_world::loot::PackOverlay::NONE)
                .unwrap_or_default();
        data.villager_hats = std::sync::RwLock::new(super::villager_hats::VillagerHats::load(root));
        let random = horizon_optifine::random_textures::RandomTextures::scan(root);
        let blinks = horizon_optifine::blink::Blinks::scan(root, &random);
        let mut emissive = horizon_optifine::emissive::Emissive::scan(root, &random);
        emissive.pair_beside(root, blinks.files());
        data.emissive = std::sync::RwLock::new(std::sync::Arc::new(emissive));
        data.blinks = std::sync::RwLock::new(blinks);
        data.random_textures = std::sync::RwLock::new(random);
        data
    }
    /// Only the keyframe animation definitions of `mob-data.json`.
    pub fn load_animations(root: &Path) -> HashMap<String, Animation> {
        fs::read(root.join("mob-data.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .map(|v| Self::from_json(&v).animations)
            .unwrap_or_default()
    }
    pub fn entity_has(&self, tag: &str, entity: &str) -> bool {
        self.entity_tags
            .get(tag)
            .is_some_and(|s| s.contains(entity))
    }
    pub fn biome_has(&self, tag: &str, biome: &str) -> bool {
        self.biome_tags.get(tag).is_some_and(|s| s.contains(biome))
    }
    fn from_json(v: &Value) -> Self {
        let states = v["states"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| StateData {
                path: PathType::from_name(s[0].as_str().unwrap_or("BLOCKED")),
                flags: s[1].as_u64().unwrap_or(0) as u16,
                top: s[2].as_f64().unwrap_or(0.0) as f32,
            })
            .collect();
        let placements = v["placements"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, p)| {
                let placement = match p["placement"].as_str() {
                    Some("on_ground") => Placement::OnGround,
                    Some("in_water") => Placement::InWater,
                    Some("in_lava") => Placement::InLava,
                    _ => Placement::NoRestrictions,
                };
                (k.clone(), placement)
            })
            .collect();
        let heightmaps = v["placements"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, p)| Some((k.clone(), p["heightmap"].as_str()?.to_owned())))
            .collect();
        let animations = v["animations"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, a)| (k.clone(), Animation::parse(a)))
            .collect();
        Self {
            states,
            placements,
            heightmaps,
            animations,
            villager_hats: Default::default(),
            cem: Default::default(),
            random_textures: Default::default(),
            emissive: Default::default(),
            blinks: Default::default(),
            biome_tags: HashMap::new(),
            entity_tags: HashMap::new(),
            pois: Default::default(),
            species_placements: Default::default(),
        }
    }
    /// Without the export, approximate path types from the gameplay facts.
    pub fn fallback(facts: &BlockFacts) -> Self {
        let states = facts
            .states
            .iter()
            .map(|f| {
                let name = &*f.name;
                let path = if matches!(
                    name,
                    "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
                ) {
                    PathType::Open
                } else if f.fluid == Fluid::Lava && !f.waterlogged {
                    PathType::Lava
                } else if name.ends_with("fence") || name.ends_with("_wall") {
                    PathType::Fence
                } else if name.ends_with("leaves") {
                    PathType::Leaves
                } else if name == "minecraft:fire" || name == "minecraft:magma_block" {
                    PathType::Fire
                } else if f.full_collision {
                    PathType::Blocked
                } else if f.fluid == Fluid::Water {
                    PathType::Water
                } else {
                    PathType::Open
                };
                let mut flags = 0;
                if !f.full_collision {
                    flags |= LAND | AIR | VALID_EMPTY_SPAWN;
                }
                if f.fluid == Fluid::Water {
                    flags |= WATER;
                }
                if f.full_collision {
                    flags |=
                        VALID_SPAWN | VALID_SPAWN_ICE | REDSTONE_CONDUCTOR | STURDY_TOP | SOLID;
                }
                StateData {
                    path,
                    flags,
                    top: if f.full_collision { 1.0 } else { 0.0 },
                }
            })
            .collect();
        Self {
            states,
            placements: HashMap::new(),
            heightmaps: HashMap::new(),
            animations: HashMap::new(),
            villager_hats: Default::default(),
            cem: Default::default(),
            random_textures: Default::default(),
            emissive: Default::default(),
            blinks: Default::default(),
            biome_tags: HashMap::new(),
            entity_tags: HashMap::new(),
            pois: Default::default(),
            species_placements: Default::default(),
        }
    }
    pub fn state(&self, id: horizon_world::registry::BlockStateId) -> StateData {
        self.states
            .get(id.0 as usize)
            .copied()
            .unwrap_or(StateData {
                path: PathType::Blocked,
                flags: 0,
                top: 1.0,
            })
    }
    /// `SpawnPlacements.getHeightmapType` (MOTION_BLOCKING_NO_LEAVES unless registered).
    pub fn heightmap(&self, entity: &str) -> &str {
        self.heightmaps
            .get(entity)
            .map_or("MOTION_BLOCKING_NO_LEAVES", String::as_str)
    }
    pub fn placement(&self, entity: &str) -> Placement {
        self.placements
            .get(entity)
            .copied()
            .unwrap_or(Placement::OnGround)
    }
    /// [`MobData::placement`] of the species' entity type, without building
    /// its id: natural spawning asks it for every position it tries.
    pub fn species_placement(&self, s: super::Species) -> Placement {
        let table = self.species_placements.get_or_init(|| {
            let defs = super::species::ALL;
            let len = defs
                .iter()
                .map(|d| d.species as usize + 1)
                .max()
                .unwrap_or(0);
            let mut table = vec![Placement::OnGround; len];
            // `species::def` takes a species' first definition: so does this.
            for d in defs.iter().rev() {
                table[d.species as usize] = self.placement(&format!("minecraft:{}", d.name));
            }
            table
        });
        table.get(s as usize).copied().unwrap_or_else(|| {
            self.placement(&format!("minecraft:{}", super::species::def(s).name))
        })
    }
}
