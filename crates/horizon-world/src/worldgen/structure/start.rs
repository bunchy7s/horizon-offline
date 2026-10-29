//! Structures, structure sets and their placements, structure starts,
//! references, terrain adaptation and per-chunk piece placement.
use super::{
    BoundingBox, StructureResources,
    jigsaw::{
        JigsawConfig, JigsawContext, PlaceFeature, PoolPiece, Pools, Projection, assemble_drawing,
    },
    legacy::{LegacyPiece, PieceContext},
};
use crate::{
    heightmap::HeightmapType,
    worldgen::{
        DecodeError,
        anchor::WorldGenerationContext,
        biome::BiomeId,
        block_feature::FeatureBlockBindings,
        feature::FeatureWorld,
        height_provider::HeightProvider,
        placement::{PlacementContext, PlacementQueryError},
        weighted::WeightedList,
    },
};
use horizon_core::{
    density::{Beardifier, JigsawJunction, RigidPiece, StructureBounds, TerrainAdjustment},
    position::{BlockPos, ChunkPos},
    random::{LegacyRandom, PositionalRandom, RandomSource, WorldgenRandom},
};
use serde_json::{Value, value::RawValue};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub const STEPS: [&str; 11] = [
    "raw_generation",
    "lakes",
    "local_modifications",
    "underground_structures",
    "surface_structures",
    "strongholds",
    "underground_ores",
    "underground_decoration",
    "fluid_springs",
    "vegetal_decoration",
    "top_layer_modification",
];

#[derive(Debug)]
pub struct JigsawDef {
    start_pool: String,
    start_jigsaw: Option<String>,
    max_depth: i32,
    start_height: HeightProvider,
    use_expansion_hack: bool,
    project_to: Option<HeightmapType>,
    max_distance: [i32; 2],
    aliases: Vec<Value>,
    padding: [i32; 2],
    waterlogging: bool,
    /// A carried mod's start rules (`"horizon:start_checks"`).
    checks: super::start_checks::StartChecks,
    /// Vanilla's draws, or YUNG's API's for `yungsapi:yung_jigsaw`.
    draws: super::jigsaw::Draws,
    /// Laid out by Repurposed Structures' and Moog's piece-limited
    /// assembler (`"horizon:piece_limited"`).
    piece_limited: Option<super::piece_limited::Config>,
}
#[derive(Debug)]
pub enum StructureKind {
    Jigsaw(JigsawDef),
    BuriedTreasure,
    Igloo,
    SwampHut,
    DesertPyramid,
    JungleTemple,
    RuinedPortal(Vec<super::ruined_portal::Setup>),
    Mansion,
    /// The biomes allowed around a monument (#required_ocean_monument_surrounding).
    Monument(BTreeSet<BiomeId>),
    Shipwreck {
        beached: bool,
    },
    OceanRuin {
        warm: bool,
        large: f32,
        cluster: f32,
    },
    Mineshaft {
        mesa: bool,
        /// #minecraft:mineshaft_blocking
        blocking: Arc<BTreeSet<BiomeId>>,
    },
    Stronghold,
    Fortress,
    NetherFossil(HeightProvider),
    EndCity,
    /// `horizon:formations`: Formations' SimpleStructure.
    Formations(super::formations::Def),
    /// A structure type whose generator is not implemented yet.
    Unsupported(String),
}
#[derive(Debug)]
pub struct StructureDef {
    pub name: String,
    pub kind: StructureKind,
    pub biomes: BTreeSet<BiomeId>,
    pub step: usize,
    pub terrain: TerrainAdjustment,
    /// A structure library's terrain kernel
    /// (`"horizon:enhanced_terrain_adaptation"`).
    pub enhanced_terrain: Option<super::enhanced_terrain::StructureAdaptation>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reduction {
    Default,
    Legacy1,
    Legacy2,
    Legacy3,
}
#[derive(Debug)]
enum Placement {
    RandomSpread {
        spacing: i32,
        separation: i32,
        triangular: bool,
    },
    ConcentricRings {
        distance: i32,
        spread: i32,
        count: i32,
        preferred: BTreeSet<BiomeId>,
    },
    Unsupported,
}
/// Concentric ring chunk positions by structure set name.
pub type Rings = BTreeMap<String, Vec<ChunkPos>>;
#[derive(Debug)]
pub struct StructureSet {
    pub name: String,
    pub structures: Vec<(usize, i32)>,
    placement: Placement,
    salt: i32,
    frequency: f32,
    reduction: Reduction,
    exclusion: Option<(String, i32)>,
    /// A carried mod's placement rules: no start nearer the world origin
    /// than this many blocks (`"horizon:min_distance_from_world_origin"`),
    /// and none within `chunk_count` chunks of any of several sets
    /// (`"horizon:exclusion_zone"`: `other_sets`, `chunk_count`).
    min_distance: Option<i64>,
    avoid: Option<Avoid>,
    /// `"horizon:ring_sections"`: starts only in odd rings around the origin.
    rings: Option<RingSections>,
    /// `"horizon:avoid_biomes"`: no start with one of these biomes near.
    avoid_biomes: Option<AvoidBiomes>,
    /// `placement.locate_offset` (buried treasure's [9, 0, 9]).
    locate_offset: [i32; 3],
}

/// `"horizon:exclusion_zone"`: no start within `chunk_count` chunks of a
/// start of any of `sets`. With `"their_zones": false` those sets' own
/// exclusion zones are not checked (Mo' Structures' ModStructurePlacement
/// tests the sets it avoids by their spread and frequency alone).
#[derive(Debug, Clone)]
struct Avoid {
    sets: Vec<String>,
    chunk_count: i32,
    their_zones: bool,
}

/// YUNG's Better Strongholds' placement (BetterStrongholdsPlacement
/// .isPlacementChunk): the world is cut into rings `thickness` chunks wide,
/// the first one starting `distance_to_first_ring` chunks out; a start is
/// placed only in the odd ones, and in none past `max_section`.
///
/// ```json
/// "horizon:ring_sections": {"distance_to_first_ring": 80, "thickness": 96, "max_section": 5}
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
struct RingSections {
    first: i32,
    thickness: i32,
    max: Option<i32>,
}
impl RingSections {
    fn decode(v: &Value) -> Self {
        let int = |k: &str| v[k].as_i64().map(|n| n as i32);
        Self {
            first: int("distance_to_first_ring").unwrap_or(0),
            thickness: int("thickness").unwrap_or(1).max(1),
            max: int("max_section"),
        }
    }
    fn allows(&self, x: i32, z: i32) -> bool {
        // Java int arithmetic: the squares wrap past 46,340 chunks, and the
        // square root of a negative int is NaN, which casts to 0.
        let squared = x.wrapping_mul(x).wrapping_add(z.wrapping_mul(z));
        let distance = f64::from(squared).sqrt() as i32;
        let section = distance.wrapping_add(self.thickness - self.first) / self.thickness;
        self.max.is_none_or(|max| section <= max) && section % 2 == 1
    }
}

/// YUNG's Better Desert and Jungle Temples' placement: no start when a
/// biome of the set is found around the chunk's middle block at height `y`
/// (BiomeSource.findBiomeHorizontal with `radius` blocks and `step` quarts
/// between samples, closest first).
///
/// ```json
/// "horizon:avoid_biomes": {"biomes": ["#minecraft:is_river", "#minecraft:is_ocean"],
///   "radius": 48, "step": 2, "y": 120}
/// ```
#[derive(Debug)]
struct AvoidBiomes {
    biomes: BTreeSet<BiomeId>,
    radius: i32,
    step: i32,
    y: i32,
}
impl AvoidBiomes {
    fn decode(
        v: &Value,
        biome_set: &dyn Fn(&Value) -> Result<BTreeSet<BiomeId>, DecodeError>,
    ) -> Result<Self, DecodeError> {
        let mut biomes = BTreeSet::new();
        match &v["biomes"] {
            Value::Array(list) => {
                for entry in list {
                    biomes.extend(biome_set(entry)?);
                }
            }
            other => biomes = biome_set(other)?,
        }
        let int = |k: &str, default: i32| v[k].as_i64().map_or(default, |n| n as i32);
        Ok(Self {
            biomes,
            radius: int("radius", 0),
            step: int("step", 1).max(1),
            y: int("y", 0),
        })
    }
    fn near(&self, chunk: ChunkPos, mut biome: impl FnMut([i32; 3]) -> BiomeId) -> bool {
        // ChunkPos.getMiddleBlockPosition, then QuartPos.fromBlock.
        let (cx, cz) = ((chunk.x * 16 + 8) >> 2, (chunk.z * 16 + 8) >> 2);
        let (radius, y) = (self.radius >> 2, self.y >> 2);
        let mut r = 0;
        while r <= radius {
            let mut dz = -r;
            while dz <= r {
                let z_edge = dz.abs() == r;
                let mut dx = -r;
                while dx <= r {
                    if (z_edge || dx.abs() == r)
                        && self.biomes.contains(&biome([cx + dx, y, cz + dz]))
                    {
                        return true;
                    }
                    dx += self.step;
                }
                dz += self.step;
            }
            r += self.step;
        }
        false
    }
}

fn heightmap(name: &str) -> Option<HeightmapType> {
    HeightmapType::ALL.into_iter().find(|k| k.name() == name)
}
fn terrain(name: Option<&str>) -> TerrainAdjustment {
    match name {
        Some("bury") => TerrainAdjustment::Bury,
        Some("beard_thin") => TerrainAdjustment::BeardThin,
        Some("beard_box") => TerrainAdjustment::BeardBox,
        Some("encapsulate") => TerrainAdjustment::Encapsulate,
        _ => TerrainAdjustment::None,
    }
}
/// A jigsaw field given as one number or as an object of two. In the object a
/// missing `a` is 0 and a missing `b` is `missing_b` (`JigsawStructure.MaxDistance`
/// and `DimensionPadding` default their second value, not copy the first).
fn pair(value: Option<&Value>, a: &str, b: &str, default: i32, missing_b: i32) -> [i32; 2] {
    match value {
        Some(Value::Number(n)) => [n.as_i64().unwrap_or(0) as i32; 2],
        Some(Value::Object(o)) => {
            let get = |k: &str| o.get(k).and_then(Value::as_i64).map(|v| v as i32);
            [get(a).unwrap_or(0), get(b).unwrap_or(missing_b)]
        }
        _ => [default; 2],
    }
}

/// All structures and structure sets of a resource bundle, in registry
/// (identifier) order.
#[derive(Debug)]
pub struct StructureRegistry {
    pub structures: Vec<StructureDef>,
    pub sets: Vec<StructureSet>,
    pub pools: Pools,
}
impl StructureRegistry {
    pub fn new(
        structures: &BTreeMap<String, String>,
        sets: &BTreeMap<String, String>,
        pools: BTreeMap<String, String>,
        biome_set: &dyn Fn(&Value) -> Result<BTreeSet<BiomeId>, DecodeError>,
    ) -> Result<Self, DecodeError> {
        let mut defs = Vec::new();
        for (name, json) in structures {
            let v: Value = serde_json::from_str(json)?;
            let draws = match v["type"].as_str() {
                Some("yungsapi:yung_jigsaw") => super::jigsaw::Draws::Yung,
                _ => super::jigsaw::Draws::Vanilla,
            };
            let kind = match v["type"].as_str().unwrap_or_default() {
                "minecraft:jigsaw" | "yungsapi:yung_jigsaw" => StructureKind::Jigsaw(JigsawDef {
                    draws,
                    start_pool: v["start_pool"].as_str().unwrap_or_default().to_owned(),
                    start_jigsaw: v["start_jigsaw_name"].as_str().map(str::to_owned),
                    max_depth: v["size"].as_i64().unwrap_or(0) as i32,
                    start_height: HeightProvider::from_json(&v["start_height"].to_string())?,
                    use_expansion_hack: v["use_expansion_hack"].as_bool().unwrap_or(false),
                    project_to: v["project_start_to_heightmap"].as_str().and_then(heightmap),
                    max_distance: pair(
                        v.get("max_distance_from_center"),
                        "horizontal",
                        "vertical",
                        80,
                        // DimensionType.Y_SIZE.
                        4064,
                    ),
                    aliases: v["pool_aliases"].as_array().cloned().unwrap_or_default(),
                    padding: pair(v.get("dimension_padding"), "bottom", "top", 0, 0),
                    waterlogging: v["liquid_settings"].as_str() != Some("ignore_waterlogging"),
                    checks: super::start_checks::StartChecks::decode(v.get("horizon:start_checks")),
                    piece_limited: v
                        .get("horizon:piece_limited")
                        .map(super::piece_limited::Config::decode),
                }),
                "minecraft:buried_treasure" => StructureKind::BuriedTreasure,
                "minecraft:igloo" => StructureKind::Igloo,
                "minecraft:swamp_hut" => StructureKind::SwampHut,
                "minecraft:desert_pyramid" => StructureKind::DesertPyramid,
                "minecraft:jungle_temple" => StructureKind::JungleTemple,
                "minecraft:woodland_mansion" => StructureKind::Mansion,
                "minecraft:ocean_monument" => StructureKind::Monument(biome_set(
                    &serde_json::json!("#minecraft:required_ocean_monument_surrounding"),
                )?),
                "minecraft:ruined_portal" => StructureKind::RuinedPortal(
                    v["setups"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(super::ruined_portal::Setup::decode)
                        .collect(),
                ),
                "minecraft:shipwreck" => StructureKind::Shipwreck {
                    beached: v["is_beached"].as_bool().unwrap_or(false),
                },
                "minecraft:ocean_ruin" => StructureKind::OceanRuin {
                    warm: v["biome_temp"].as_str() == Some("warm"),
                    large: v["large_probability"].as_f64().unwrap_or(0.) as f32,
                    cluster: v["cluster_probability"].as_f64().unwrap_or(0.) as f32,
                },
                "minecraft:mineshaft" => StructureKind::Mineshaft {
                    mesa: v["mineshaft_type"].as_str() == Some("mesa"),
                    blocking: Arc::new(biome_set(&Value::String(
                        "#minecraft:mineshaft_blocking".into(),
                    ))?),
                },
                "minecraft:stronghold" => StructureKind::Stronghold,
                "minecraft:fortress" => StructureKind::Fortress,
                "minecraft:end_city" => StructureKind::EndCity,
                "minecraft:nether_fossil" => StructureKind::NetherFossil(
                    HeightProvider::from_json(&v["height"].to_string())?,
                ),
                "horizon:formations" => match super::formations::Def::decode(&v) {
                    Ok(def) => StructureKind::Formations(def),
                    Err(_) => StructureKind::Unsupported("horizon:formations".into()),
                },
                other => StructureKind::Unsupported(other.to_owned()),
            };
            defs.push(StructureDef {
                name: name.clone(),
                kind,
                biomes: biome_set(&v["biomes"])?,
                step: STEPS
                    .iter()
                    .position(|s| Some(*s) == v["step"].as_str())
                    .unwrap_or(4),
                terrain: terrain(v["terrain_adaptation"].as_str()),
                enhanced_terrain: v
                    .get("horizon:enhanced_terrain_adaptation")
                    .map(super::enhanced_terrain::StructureAdaptation::decode)
                    .transpose()?,
            });
        }
        let index = |name: &str| defs.iter().position(|d| d.name == name);
        let mut out = Vec::new();
        for (name, json) in sets {
            let v: Value = serde_json::from_str(json)?;
            let p = &v["placement"];
            let placement = match p["type"].as_str().unwrap_or_default() {
                "minecraft:random_spread" => Placement::RandomSpread {
                    spacing: p["spacing"].as_i64().unwrap_or(1) as i32,
                    separation: p["separation"].as_i64().unwrap_or(0) as i32,
                    triangular: p["spread_type"].as_str() == Some("triangular"),
                },
                "minecraft:concentric_rings" => Placement::ConcentricRings {
                    distance: p["distance"].as_i64().unwrap_or(0) as i32,
                    spread: p["spread"].as_i64().unwrap_or(0) as i32,
                    count: p["count"].as_i64().unwrap_or(0) as i32,
                    preferred: biome_set(&p["preferred_biomes"])?,
                },
                _ => Placement::Unsupported,
            };
            out.push(StructureSet {
                name: name.clone(),
                structures: v["structures"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| {
                        Some((
                            index(e["structure"].as_str()?)?,
                            e["weight"].as_i64().unwrap_or(1) as i32,
                        ))
                    })
                    .collect(),
                placement,
                salt: p["salt"].as_i64().unwrap_or(0) as i32,
                frequency: p["frequency"].as_f64().unwrap_or(1.) as f32,
                locate_offset: {
                    let at = |i: usize| p["locate_offset"][i].as_i64().unwrap_or(0) as i32;
                    [at(0), at(1), at(2)]
                },
                reduction: match p["frequency_reduction_method"].as_str() {
                    Some("legacy_type_1") => Reduction::Legacy1,
                    Some("legacy_type_2") => Reduction::Legacy2,
                    Some("legacy_type_3") => Reduction::Legacy3,
                    _ => Reduction::Default,
                },
                exclusion: p.get("exclusion_zone").map(|z| {
                    (
                        z["other_set"].as_str().unwrap_or_default().to_owned(),
                        z["chunk_count"].as_i64().unwrap_or(0) as i32,
                    )
                }),
                min_distance: p
                    .get("horizon:min_distance_from_world_origin")
                    .and_then(Value::as_i64),
                avoid: p.get("horizon:exclusion_zone").map(|z| Avoid {
                    sets: z["other_sets"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|s| Some(s.as_str()?.to_owned()))
                        .collect(),
                    chunk_count: z["chunk_count"].as_i64().unwrap_or(0) as i32,
                    their_zones: z["their_zones"].as_bool().unwrap_or(true),
                }),
                rings: p.get("horizon:ring_sections").map(RingSections::decode),
                avoid_biomes: p
                    .get("horizon:avoid_biomes")
                    .map(|a| AvoidBiomes::decode(a, biome_set))
                    .transpose()?,
            });
        }
        Ok(Self {
            structures: defs,
            sets: out,
            pools: Pools::new(pools),
        })
    }
    /// ChunkGeneratorStructureState.generatePositions: the ring positions of
    /// each concentric-ring set among `possible_sets` before their biome
    /// searches (set index, searches in order).
    pub fn ring_searches(
        &self,
        possible_sets: &[usize],
        seed: i64,
    ) -> Vec<(usize, Vec<super::stronghold::RingSearch>)> {
        possible_sets
            .iter()
            .filter_map(|&i| match &self.sets[i].placement {
                Placement::ConcentricRings {
                    distance,
                    spread,
                    count,
                    ..
                } => Some((
                    i,
                    super::stronghold::ring_searches(seed, *distance, *spread, *count),
                )),
                _ => None,
            })
            .collect()
    }
    /// How far from a chunk, in chunks, deciding its starts can look at
    /// other sets' start chunks through exclusion zones: every set's zones
    /// summed, a bound on any chain of them.
    pub fn zone_reach(&self) -> i32 {
        self.sets
            .iter()
            .map(|s| {
                s.exclusion.as_ref().map_or(0, |e| e.1.max(0))
                    + s.avoid.as_ref().map_or(0, |a| a.chunk_count.max(0))
            })
            .sum()
    }
    /// The preferred biomes of a concentric-ring set (empty for others).
    pub fn ring_biomes(&self, set: usize) -> &BTreeSet<BiomeId> {
        static NONE: BTreeSet<BiomeId> = BTreeSet::new();
        match &self.sets[set].placement {
            Placement::ConcentricRings { preferred, .. } => preferred,
            _ => &NONE,
        }
    }
    /// RandomSpreadStructurePlacement.getPotentialStructureChunk et al.
    /// RandomSpreadStructurePlacement.getPotentialStructureChunk for the
    /// sector holding chunk (`x`, `z`); `None` for other placements.
    pub fn potential_chunk(set: &StructureSet, seed: i64, x: i32, z: i32) -> Option<ChunkPos> {
        let Placement::RandomSpread {
            spacing,
            separation,
            triangular,
        } = set.placement
        else {
            return None;
        };
        let (gx, gz) = (x.div_euclid(spacing), z.div_euclid(spacing));
        let mut random = WorldgenRandom::new(LegacyRandom::new(0));
        random.set_large_feature_with_salt(seed, gx, gz, set.salt);
        let limit = spacing - separation;
        let spread = |r: &mut WorldgenRandom<LegacyRandom>| {
            if triangular {
                (r.next_int_bound(limit).unwrap_or(0) + r.next_int_bound(limit).unwrap_or(0)) / 2
            } else {
                r.next_int_bound(limit).unwrap_or(0)
            }
        };
        let sx = spread(&mut random);
        let sz = spread(&mut random);
        Some(ChunkPos::new(gx * spacing + sx, gz * spacing + sz))
    }
    pub fn is_structure_chunk(
        &self,
        set: &StructureSet,
        seed: i64,
        x: i32,
        z: i32,
        rings: &Rings,
    ) -> bool {
        self.structure_chunk(set, seed, x, z, rings, true)
    }
    /// [`Self::is_structure_chunk`]; without the set's exclusion zones when
    /// `zones` is false.
    fn structure_chunk(
        &self,
        set: &StructureSet,
        seed: i64,
        x: i32,
        z: i32,
        rings: &Rings,
        zones: bool,
    ) -> bool {
        let placed = match set.placement {
            Placement::RandomSpread {
                spacing,
                separation,
                triangular,
            } => {
                let (gx, gz) = (x.div_euclid(spacing), z.div_euclid(spacing));
                let mut random = WorldgenRandom::new(LegacyRandom::new(0));
                random.set_large_feature_with_salt(seed, gx, gz, set.salt);
                let limit = spacing - separation;
                let spread = |r: &mut WorldgenRandom<LegacyRandom>| {
                    if triangular {
                        (r.next_int_bound(limit).unwrap_or(0)
                            + r.next_int_bound(limit).unwrap_or(0))
                            / 2
                    } else {
                        r.next_int_bound(limit).unwrap_or(0)
                    }
                };
                let sx = spread(&mut random);
                let sz = spread(&mut random);
                gx * spacing + sx == x && gz * spacing + sz == z
            }
            Placement::ConcentricRings { .. } => rings
                .get(&set.name)
                .is_some_and(|r| r.contains(&ChunkPos::new(x, z))),
            Placement::Unsupported => false,
        };
        if !placed {
            return false;
        }
        if set.rings.as_ref().is_some_and(|r| !r.allows(x, z)) {
            return false;
        }
        // AdvancedRandomSpread.isPlacementChunk, in blocks from the chunk's corner.
        if let Some(d) = set.min_distance {
            let (bx, bz) = (i64::from(x) * 16, i64::from(z) * 16);
            if bx * bx + bz * bz < d * d {
                return false;
            }
        }
        if let Some(avoid) = set.avoid.as_ref().filter(|_| zones) {
            if avoid.sets.iter().any(|other| {
                let range = avoid.chunk_count;
                self.has_start_in_range(other, seed, x, z, range, rings, avoid.their_zones)
            }) {
                return false;
            }
        }
        if set.frequency < 1. && !reduce(set.reduction, seed, set.salt, x, z, set.frequency) {
            return false;
        }
        match set.exclusion.as_ref().filter(|_| zones) {
            Some((other, range)) => {
                !self.has_start_in_range(other, seed, x, z, *range, rings, true)
            }
            None => true,
        }
    }
    /// ChunkGeneratorStructureState.hasStructureChunkInRange: whether set
    /// `other` places a structure within `range` chunks of (`x`, `z`),
    /// checking its own exclusion zones when `zones` is true.
    #[allow(clippy::too_many_arguments)]
    fn has_start_in_range(
        &self,
        other: &str,
        seed: i64,
        x: i32,
        z: i32,
        range: i32,
        rings: &Rings,
        zones: bool,
    ) -> bool {
        let Some(other) = self.sets.iter().find(|s| s.name == other) else {
            return false;
        };
        (x - range..=x + range).any(|tx| {
            (z - range..=z + range)
                .any(|tz| self.structure_chunk(other, seed, tx, tz, rings, zones))
        })
    }
}
impl StructureSet {
    /// The random spread's spacing in chunks (`None` for rings).
    pub fn spacing(&self) -> Option<i32> {
        match self.placement {
            Placement::RandomSpread { spacing, .. } => Some(spacing),
            _ => None,
        }
    }
    /// StructurePlacement.getLocatePos: the chunk's corner at y 0, moved by
    /// the set's `locate_offset`.
    pub fn locate_pos(&self, chunk: ChunkPos) -> [i32; 3] {
        let o = self.locate_offset;
        [chunk.x * 16 + o[0], o[1], chunk.z * 16 + o[2]]
    }
}
fn reduce(method: Reduction, seed: i64, salt: i32, x: i32, z: i32, probability: f32) -> bool {
    let mut random = WorldgenRandom::new(LegacyRandom::new(0));
    match method {
        Reduction::Default => {
            random.set_large_feature_with_salt(seed, salt, x, z);
            random.next_float() < probability
        }
        Reduction::Legacy1 => {
            let (cx, cz) = (x >> 4, z >> 4);
            random.set_seed(i64::from(cx ^ (cz << 4)) ^ seed);
            random.next_int();
            random
                .next_int_bound((1. / probability) as i32)
                .unwrap_or(1)
                == 0
        }
        Reduction::Legacy2 => {
            random.set_large_feature_with_salt(seed, x, z, 10_387_320);
            random.next_float() < probability
        }
        Reduction::Legacy3 => {
            random.set_large_feature_seed(seed, x, z);
            random.next_double() < f64::from(probability)
        }
    }
}

#[derive(Debug)]
pub enum Piece {
    Pool(PoolPiece),
    Legacy(Box<dyn LegacyPiece>),
}
impl Piece {
    pub fn bounding_box(&self) -> BoundingBox {
        match self {
            Self::Pool(p) => p.bounding_box,
            Self::Legacy(p) => p.bounding_box(),
        }
    }
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Pool(_) => "minecraft:jigsaw",
            Self::Legacy(p) => p.type_name(),
        }
    }
    /// `StructurePiece.createTag`; None for a type Horizon cannot save yet.
    pub fn save(&self) -> Option<crate::nbt::Compound> {
        match self {
            Self::Pool(p) => Some(p.save()),
            Self::Legacy(p) => p.save(),
        }
    }
}
fn legacy(pieces: Vec<impl LegacyPiece + 'static>) -> Vec<Piece> {
    pieces
        .into_iter()
        .map(|p| Piece::Legacy(Box::new(p)))
        .collect()
}
/// Structure.onTopOfChunkCenter: the start position, or None when no valid
/// biome can exist in the chunk-center column.
fn top_of_chunk_center(
    context: &mut impl StartContext,
    chunk: ChunkPos,
    kind: HeightmapType,
) -> Option<[i32; 3]> {
    let (x, z) = (chunk.x * 16 + 8, chunk.z * 16 + 8);
    let (min, max) = (context.min_y(), context.max_y());
    if !context.column_has_valid_biome(x, z, min - 1, max) {
        return None;
    }
    Some([x, context.base_height(x, z, kind) - 1, z])
}
fn first_occupied(context: &mut impl StartContext, x: i32, z: i32) -> i32 {
    context.base_height(x, z, HeightmapType::WorldSurfaceWg) - 1
}
fn corner_heights(context: &mut impl StartContext, x: i32, sx: i32, z: i32, sz: i32) -> [i32; 4] {
    [
        first_occupied(context, x, z),
        first_occupied(context, x, z + sz),
        first_occupied(context, x + sx, z),
        first_occupied(context, x + sx, z + sz),
    ]
}
#[derive(Debug)]
pub struct StructureStart {
    pub structure: usize,
    pub name: String,
    pub chunk: ChunkPos,
    pub pieces: Vec<Piece>,
    /// Piece union, inflated by 12 for terrain-adapting structures.
    pub bounding_box: BoundingBox,
}
impl StructureStart {
    /// `StructureStart.createTag` in its own chunk; None while one of its
    /// pieces cannot be saved. `references` counts `addReference` calls,
    /// which only locating structures makes: 0 in a freshly made world.
    pub fn save(&self) -> Option<crate::nbt::Tag> {
        use crate::nbt::Tag;
        let children: Option<Vec<Tag>> = self
            .pieces
            .iter()
            .map(|p| p.save().map(Tag::Compound))
            .collect();
        Some(Tag::compound([
            ("id", Tag::String(self.name.as_str().into())),
            ("ChunkX", Tag::Int(self.chunk.x)),
            ("ChunkZ", Tag::Int(self.chunk.z)),
            ("references", Tag::Int(0)),
            ("Children", Tag::List(children?)),
        ]))
    }
}

/// A block of a noise column (ChunkGenerator.getBaseColumn), by what
/// structures ask of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseBlock {
    Air,
    /// The water block (`is(Blocks.WATER)`), as the aquifers and the sea
    /// fill it.
    Water,
    Lava,
    /// BlockState.isSolid: the noise's own blocks (stone, deepslate, ore
    /// veins, netherrack...).
    Solid,
    Other,
}
impl NoiseBlock {
    pub fn is_solid(self) -> bool {
        self == Self::Solid
    }
}
/// ChunkGenerator.getBaseColumn's blocks from `min` up; past its ends a
/// NoiseColumn reads air.
#[derive(Clone, Debug, Default)]
pub struct NoiseColumn {
    pub min: i32,
    pub blocks: Vec<NoiseBlock>,
}
impl NoiseColumn {
    pub fn get(&self, y: i32) -> NoiseBlock {
        usize::try_from(y - self.min)
            .ok()
            .and_then(|i| self.blocks.get(i).copied())
            .unwrap_or(NoiseBlock::Air)
    }
}

/// Biome and height queries for one structure generation attempt.
pub trait StartContext: JigsawContext {
    fn seed(&self) -> i64;
    fn sea_level(&self) -> i32;
    /// ChunkGenerator.getBaseColumn as heightmap opacity, indexed from min_y.
    fn base_column(&mut self, x: i32, z: i32, kind: HeightmapType) -> Vec<bool>;
    /// base_column for several heightmap kinds from one noise-column pass.
    fn base_columns(&mut self, x: i32, z: i32, kinds: &[HeightmapType]) -> Vec<Vec<bool>>;
    /// ChunkGenerator.getBaseColumn's blocks: the noise column's states, as
    /// mods' placements test them (Formations').
    fn noise_column(&mut self, x: i32, z: i32) -> NoiseColumn;
    /// Biome.coldEnoughToSnow at a position.
    fn cold_enough_to_snow(&mut self, biome: BiomeId, pos: [i32; 3]) -> bool;
    fn generation(&self) -> WorldGenerationContext;
    /// Noise biome at a quart position (caching resolver, no zoom).
    fn noise_biome(&mut self, quart: [i32; 3]) -> BiomeId;
    /// Noise biomes of `quarts` in order through one resolver, as
    /// BiomeSource.findBiomeHorizontal samples: each climate lookup starts
    /// from the previous result, which a tie keeps.
    fn noise_biomes(&mut self, quarts: &[[i32; 3]]) -> Vec<BiomeId>;
    fn set_valid_biomes(&mut self, biomes: &BTreeSet<BiomeId>);
}

/// ChunkGenerator.createStructures for one chunk: the starts it owns.
pub fn create_starts(
    registry: &StructureRegistry,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
    context: &mut impl StartContext,
    possible_sets: &[usize],
    rings: &Rings,
    chunk: ChunkPos,
) -> Vec<StructureStart> {
    let seed = context.seed();
    let mut starts = Vec::new();
    for &set_index in possible_sets {
        let set = &registry.sets[set_index];
        if !registry.is_structure_chunk(set, seed, chunk.x, chunk.z, rings) {
            continue;
        }
        // Part of the set's placement in YUNG's mods; checked here, where the
        // biome source is at hand, so another set's exclusion zone naming
        // this one does not see it.
        if set
            .avoid_biomes
            .as_ref()
            .is_some_and(|a| a.near(chunk, |q| context.noise_biome(q)))
        {
            continue;
        }
        if set.structures.len() == 1 {
            if let Some(start) = try_generate(
                registry,
                resources,
                bindings,
                context,
                set.structures[0].0,
                chunk,
            ) {
                starts.push(start);
            }
            continue;
        }
        let mut options = set.structures.clone();
        let mut random = WorldgenRandom::new(LegacyRandom::new(0));
        random.set_large_feature_seed(seed, chunk.x, chunk.z);
        let mut total: i32 = options.iter().map(|o| o.1).sum();
        while !options.is_empty() {
            let mut choice = random.next_int_bound(total).unwrap_or(0);
            let mut index = 0;
            for option in &options {
                choice -= option.1;
                if choice < 0 {
                    break;
                }
                index += 1;
            }
            let selected = options[index];
            if let Some(start) =
                try_generate(registry, resources, bindings, context, selected.0, chunk)
            {
                starts.push(start);
                break;
            }
            options.remove(index);
            total -= selected.1;
        }
    }
    starts
}
fn try_generate(
    registry: &StructureRegistry,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
    context: &mut impl StartContext,
    structure: usize,
    chunk: ChunkPos,
) -> Option<StructureStart> {
    let def = &registry.structures[structure];
    context.set_valid_biomes(&def.biomes);
    let seed = context.seed();
    let mut random = WorldgenRandom::new(LegacyRandom::new(0));
    random.set_large_feature_seed(seed, chunk.x, chunk.z);
    let (position, pieces) = match &def.kind {
        StructureKind::Jigsaw(jigsaw) => {
            let height = jigsaw
                .start_height
                .sample(&mut random, context.generation())
                .ok()?;
            if !jigsaw
                .checks
                .pass(context, &def.biomes, chunk, height, jigsaw.project_to)
            {
                return None;
            }
            let start = [chunk.x * 16, height, chunk.z * 16];
            let aliases = resolve_aliases(&jigsaw.aliases, start, seed);
            let config = JigsawConfig {
                start_pool: &jigsaw.start_pool,
                start_jigsaw: jigsaw.start_jigsaw.as_deref(),
                max_depth: jigsaw.max_depth,
                use_expansion_hack: jigsaw.use_expansion_hack,
                project_to: jigsaw.project_to,
                max_distance: jigsaw.max_distance,
                aliases: &aliases,
                padding: jigsaw.padding,
                waterlogging: jigsaw.waterlogging,
            };
            let (position, mut pieces) = match &jigsaw.piece_limited {
                Some(limits) => super::piece_limited::assemble(
                    &config,
                    limits,
                    context,
                    &registry.pools,
                    resources,
                    bindings,
                    chunk,
                    start,
                )?,
                None => assemble_drawing(
                    jigsaw.draws,
                    &config,
                    context,
                    &registry.pools,
                    resources,
                    bindings,
                    &mut random,
                    start,
                )?,
            };
            // Vanilla's JigsawPlacement keeps no piece at size 0; the
            // piece-limited assembler keeps the start piece.
            if jigsaw.max_depth <= 0 && jigsaw.piece_limited.is_none() {
                pieces.clear();
            }
            (
                position,
                pieces.into_iter().map(Piece::Pool).collect::<Vec<_>>(),
            )
        }
        StructureKind::BuriedTreasure => {
            let stub = top_of_chunk_center(context, chunk, HeightmapType::OceanFloorWg)?;
            let piece = super::simple::BuriedTreasurePiece {
                position: [chunk.x * 16 + 9, 90, chunk.z * 16 + 9],
                found: std::sync::Mutex::new(None),
            };
            (stub, legacy(vec![piece]))
        }
        StructureKind::Igloo => {
            let stub = top_of_chunk_center(context, chunk, HeightmapType::WorldSurfaceWg)?;
            let rotation = super::Rotation::random(&mut random);
            let pieces = super::simple::igloo_pieces(
                resources,
                [chunk.x * 16, 90, chunk.z * 16],
                rotation,
                &mut random,
            )?;
            (stub, legacy(pieces))
        }
        StructureKind::SwampHut => {
            let stub = top_of_chunk_center(context, chunk, HeightmapType::WorldSurfaceWg)?;
            let piece = super::simple::SwampHutPiece::new(&mut random, chunk.x * 16, chunk.z * 16);
            (stub, legacy(vec![piece]))
        }
        StructureKind::DesertPyramid | StructureKind::JungleTemple => {
            let desert = matches!(def.kind, StructureKind::DesertPyramid);
            let (width, depth) = if desert { (21, 21) } else { (12, 15) };
            let (x, z) = (chunk.x * 16, chunk.z * 16);
            if !context.column_has_valid_biome(x + 8, z + 8, context.min_y() - 1, context.max_y()) {
                return None;
            }
            let lowest = corner_heights(context, x, width, z, depth)
                .into_iter()
                .min()
                .unwrap_or(0);
            if lowest < context.sea_level() {
                return None;
            }
            let stub = [x + 8, first_occupied(context, x + 8, z + 8), z + 8];
            let piece: Box<dyn LegacyPiece> = if desert {
                Box::new(super::temple::DesertPyramidPiece::new(&mut random, x, z))
            } else {
                Box::new(super::temple::JungleTemplePiece::new(&mut random, x, z))
            };
            (stub, vec![Piece::Legacy(piece)])
        }
        StructureKind::Monument(surrounding) => {
            let (x, z) = (chunk.x * 16 + 9, chunk.z * 16 + 9);
            let (y, r) = (context.sea_level(), 29);
            for qz in (z - r) >> 2..=(z + r) >> 2 {
                for qx in (x - r) >> 2..=(x + r) >> 2 {
                    for qy in (y - r) >> 2..=(y + r) >> 2 {
                        if !surrounding.contains(&context.noise_biome([qx, qy, qz])) {
                            return None;
                        }
                    }
                }
            }
            let stub = top_of_chunk_center(context, chunk, HeightmapType::OceanFloorWg)?;
            let direction = super::legacy::random_horizontal(&mut random);
            let piece = super::monument::MonumentBuilding::new(
                &mut random,
                chunk.x * 16 - 29,
                chunk.z * 16 - 29,
                direction,
            );
            (stub, legacy(vec![piece]))
        }
        StructureKind::Mansion => {
            let (x, z) = (chunk.x * 16 + 7, chunk.z * 16 + 7);
            if !context.column_has_valid_biome(x, z, context.min_y() - 1, context.max_y()) {
                return None;
            }
            let rotation = super::Rotation::random(&mut random);
            let (sx, sz) = match rotation {
                super::Rotation::None => (5, 5),
                super::Rotation::Clockwise90 => (-5, 5),
                super::Rotation::Clockwise180 => (-5, -5),
                super::Rotation::Counterclockwise90 => (5, -5),
            };
            let y = corner_heights(context, x, sx, z, sz)
                .into_iter()
                .min()
                .unwrap_or(0);
            if y < 60 {
                return None;
            }
            let start = [x, y, z];
            let pieces = super::mansion::pieces(resources, start, rotation, &mut random)?;
            (start, legacy(pieces))
        }
        StructureKind::RuinedPortal(setups) => {
            let (position, piece) = super::ruined_portal::start(
                context,
                resources,
                bindings,
                &mut random,
                chunk,
                setups,
            )?;
            (position, legacy(vec![piece]))
        }
        StructureKind::Shipwreck { beached } => {
            let kind = if *beached {
                HeightmapType::WorldSurfaceWg
            } else {
                HeightmapType::OceanFloorWg
            };
            let stub = top_of_chunk_center(context, chunk, kind)?;
            let rotation = super::Rotation::random(&mut random);
            let piece = super::simple::shipwreck_piece(
                resources,
                [chunk.x * 16, 90, chunk.z * 16],
                rotation,
                &mut random,
                *beached,
            )?;
            if super::simple::shipwreck_too_big(&piece.template) {
                let b = piece.bounding_box;
                let [sx, _, sz] = b.span();
                let heights = corner_heights(context, b.min[0], sx, b.min[2], sz);
                let height = if *beached {
                    let min = heights.into_iter().min().unwrap_or(0);
                    min - piece.template.size[1] / 2 - random.next_int_bound(3).unwrap_or(0)
                } else {
                    heights.iter().sum::<i32>() / 4
                };
                *piece.state.lock().unwrap() = Some(height);
            }
            (stub, legacy(vec![piece]))
        }
        StructureKind::OceanRuin {
            warm,
            large,
            cluster,
        } => {
            let stub = top_of_chunk_center(context, chunk, HeightmapType::OceanFloorWg)?;
            let rotation = super::Rotation::random(&mut random);
            let config = super::ocean_ruin::OceanRuinConfig {
                warm: *warm,
                large_probability: *large,
                cluster_probability: *cluster,
            };
            let pieces = super::ocean_ruin::pieces(
                resources,
                bindings,
                [chunk.x * 16, 90, chunk.z * 16],
                rotation,
                &mut random,
                &config,
            )?;
            (stub, legacy(pieces))
        }
        StructureKind::Mineshaft { mesa, blocking } => {
            let (sea, min) = (context.sea_level(), context.min_y());
            let (position, pieces) = super::mineshaft::generate(
                &mut random,
                [chunk.x, chunk.z],
                *mesa,
                blocking.clone(),
                sea,
                min,
                &mut |x, z| context.base_height(x, z, HeightmapType::WorldSurfaceWg),
            );
            (position, legacy(pieces))
        }
        StructureKind::Stronghold => {
            let (sea, min) = (context.sea_level(), context.min_y());
            let pieces =
                super::stronghold::generate(&mut random, seed, [chunk.x, chunk.z], sea, min);
            ([chunk.x * 16, 0, chunk.z * 16], legacy(pieces))
        }
        StructureKind::EndCity => {
            let (x, z) = (chunk.x * 16 + 7, chunk.z * 16 + 7);
            if !context.column_has_valid_biome(x, z, context.min_y() - 1, context.max_y()) {
                return None;
            }
            let rotation = super::Rotation::random(&mut random);
            let (sx, sz) = match rotation {
                super::Rotation::None => (5, 5),
                super::Rotation::Clockwise90 => (-5, 5),
                super::Rotation::Clockwise180 => (-5, -5),
                super::Rotation::Counterclockwise90 => (5, -5),
            };
            let y = corner_heights(context, x, sx, z, sz)
                .into_iter()
                .min()
                .unwrap_or(0);
            if y < 60 {
                return None;
            }
            let start = [x, y, z];
            if !def
                .biomes
                .contains(&context.noise_biome(start.map(|v| v >> 2)))
            {
                return None;
            }
            let pieces = super::end_city::pieces(resources, start, rotation, &mut random)?;
            (start, legacy(pieces))
        }
        StructureKind::Fortress => {
            let pieces = super::fortress::generate(&mut random, [chunk.x, chunk.z]);
            ([chunk.x * 16, 64, chunk.z * 16], legacy(pieces))
        }
        StructureKind::NetherFossil(height) => {
            let x = chunk.x * 16 + random.next_int_bound(16).unwrap_or(0);
            let z = chunk.z * 16 + random.next_int_bound(16).unwrap_or(0);
            let sea = context.sea_level();
            let y = height.sample(&mut random, context.generation()).ok()?;
            let columns = context.base_columns(
                x,
                z,
                &[HeightmapType::WorldSurfaceWg, HeightmapType::OceanFloorWg],
            );
            let y = super::nether_fossil::find_floor(
                &columns[0],
                &columns[1],
                context.min_y(),
                y,
                sea,
            )?;
            let position = [x, y, z];
            let quart = position.map(|v| v >> 2);
            if !def.biomes.contains(&context.noise_biome(quart)) {
                return None;
            }
            let piece = super::nether_fossil::piece(resources, &mut random, position)?;
            (position, legacy(vec![piece]))
        }
        StructureKind::Formations(formations) => {
            let (position, pieces) = super::formations::generate(
                formations,
                context,
                &registry.pools,
                resources,
                bindings,
                &mut random,
                chunk,
            )?;
            (position, pieces.into_iter().map(Piece::Pool).collect())
        }
        StructureKind::Unsupported(_) => return None,
    };
    let quart = position.map(|v| v >> 2);
    if !def.biomes.contains(&context.noise_biome(quart)) || pieces.is_empty() {
        return None;
    }
    Some(StructureStart::new(structure, def, chunk, pieces))
}
impl StructureStart {
    /// A start of `def` (registry index `structure`) from its pieces, at
    /// least one: their box, grown where the structure adapts the terrain.
    pub fn new(structure: usize, def: &StructureDef, chunk: ChunkPos, pieces: Vec<Piece>) -> Self {
        let mut bounding_box = pieces[0].bounding_box();
        for piece in &pieces[1..] {
            bounding_box.encapsulate(&piece.bounding_box());
        }
        if def.terrain != TerrainAdjustment::None {
            bounding_box = bounding_box.inflated(12, 12);
        }
        if let Some(enhanced) = def.enhanced_terrain.as_ref().filter(|e| e.inflate) {
            let r = enhanced.radius();
            bounding_box = bounding_box.inflated(r, r);
        }
        Self {
            structure,
            name: def.name.clone(),
            chunk,
            pieces,
            bounding_box,
        }
    }
}
/// PoolAliasLookup.create.
fn resolve_aliases(bindings: &[Value], position: [i32; 3], seed: i64) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    if bindings.is_empty() {
        return map;
    }
    let mut random = LegacyRandom::new(seed).fork_positional().at(BlockPos::new(
        position[0],
        position[1],
        position[2],
    ));
    fn visit(binding: &Value, random: &mut LegacyRandom, map: &mut BTreeMap<String, String>) {
        let weighted = |key: &str| -> Option<WeightedList<Value>> {
            let raw: Box<RawValue> = serde_json::from_str(&binding[key].to_string()).ok()?;
            WeightedList::decode(&raw, |v| Ok(serde_json::from_str::<Value>(v.get())?)).ok()
        };
        match binding["type"].as_str().unwrap_or_default() {
            "minecraft:direct" => {
                map.insert(
                    binding["alias"].as_str().unwrap_or_default().to_owned(),
                    binding["target"].as_str().unwrap_or_default().to_owned(),
                );
            }
            "minecraft:random" => {
                if let Some(target) =
                    weighted("targets").and_then(|w| w.sample(random).ok().cloned())
                {
                    map.insert(
                        binding["alias"].as_str().unwrap_or_default().to_owned(),
                        target.as_str().unwrap_or_default().to_owned(),
                    );
                }
            }
            "minecraft:random_group" => {
                if let Some(group) = weighted("groups").and_then(|w| w.sample(random).ok().cloned())
                {
                    for b in group.as_array().into_iter().flatten() {
                        visit(b, random, map);
                    }
                }
            }
            _ => {}
        }
    }
    for binding in bindings {
        visit(binding, &mut random, &mut map);
    }
    map
}

/// Beardifier.forStructuresInChunk over the starts referencing a chunk.
pub fn beardifier(
    registry: &StructureRegistry,
    starts: &[Arc<StructureStart>],
    chunk: ChunkPos,
) -> Option<Beardifier> {
    let (bx, bz) = (chunk.x * 16, chunk.z * 16);
    let mut rigids = Vec::new();
    let mut junctions = Vec::new();
    let mut any: Option<BoundingBox> = None;
    let include = |b: BoundingBox, any: &mut Option<BoundingBox>| match any {
        Some(a) => a.encapsulate(&b),
        None => *any = Some(b),
    };
    let (mut kernel_pieces, mut kernel_junctions) = (Vec::new(), Vec::new());
    let mut kernels: Option<BoundingBox> = None;
    for start in starts {
        let def = &registry.structures[start.structure];
        if let Some(enhanced) = &def.enhanced_terrain {
            if let Some(b) =
                enhanced.collect(start, chunk, &mut kernel_pieces, &mut kernel_junctions)
            {
                include(b, &mut kernels);
            }
        }
        let adjustment = def.terrain;
        if adjustment == TerrainAdjustment::None {
            continue;
        }
        for piece in &start.pieces {
            let b = piece.bounding_box();
            let near = b.max[0] >= bx - 12
                && b.min[0] <= bx + 15 + 12
                && b.max[2] >= bz - 12
                && b.min[2] <= bz + 15 + 12;
            if !near {
                continue;
            }
            let Piece::Pool(pool) = piece else {
                rigids.push(RigidPiece {
                    bounds: StructureBounds::new(b.min, b.max)?,
                    adjustment,
                    ground_level_delta: 0,
                });
                include(b, &mut any);
                continue;
            };
            if pool.element.projection() == Projection::Rigid {
                rigids.push(RigidPiece {
                    bounds: StructureBounds::new(b.min, b.max)?,
                    adjustment,
                    ground_level_delta: pool.ground_level_delta,
                });
                include(b, &mut any);
            }
            for j in &pool.junctions {
                if j.source_x > bx - 12
                    && j.source_z > bz - 12
                    && j.source_x < bx + 15 + 12
                    && j.source_z < bz + 15 + 12
                {
                    junctions.push(JigsawJunction {
                        source_x: j.source_x,
                        source_ground_y: j.source_ground_y,
                        source_z: j.source_z,
                    });
                    let p = [j.source_x, j.source_ground_y, j.source_z];
                    include(BoundingBox::new(p, p), &mut any);
                }
            }
        }
    }
    // The libraries' kernels reach as far as their radius, beyond vanilla's.
    let mut affected = any.map(|a| a.inflated(24, 24));
    if let Some(k) = kernels {
        include(k, &mut affected);
    }
    let affected = affected?;
    Some(
        Beardifier::new(
            rigids,
            junctions,
            StructureBounds::new(affected.min, affected.max),
        )
        .with_kernels(kernel_pieces, kernel_junctions),
    )
}

/// StructureStart.placeInChunk for pieces intersecting the chunk's writable box.
#[allow(clippy::too_many_arguments)]
pub fn place_in_chunk<W: FeatureWorld + PlacementContext, R: RandomSource + 'static>(
    start: &StructureStart,
    world: &mut W,
    resources: &StructureResources,
    blocks: &dyn crate::worldgen::structure::legacy::SolidRender,
    random: &mut R,
    chunk_box: BoundingBox,
    place_feature: &mut PlaceFeature<'_, W, R>,
) -> Result<(), PlacementQueryError> {
    let Some(first) = start.pieces.first() else {
        return Ok(());
    };
    let center_box = first.bounding_box();
    let center = center_box.center();
    let reference = [center[0], center_box.min[1], center[2]];
    for piece in &start.pieces {
        if piece.bounding_box().intersects(&chunk_box) {
            match piece {
                Piece::Pool(p) => {
                    p.place(
                        world,
                        resources,
                        random,
                        chunk_box,
                        reference,
                        place_feature,
                    )?;
                }
                Piece::Legacy(p) => {
                    let mut context = PieceContext {
                        world,
                        resources,
                        blocks,
                        random,
                        chunk_box,
                        chunk: ChunkPos::new(chunk_box.min[0] >> 4, chunk_box.min[2] >> 4),
                        reference,
                    };
                    p.post_process(&mut context)?;
                }
            }
        }
    }
    if let Piece::Legacy(p) = first {
        let boxes: Vec<BoundingBox> = start.pieces.iter().map(Piece::bounding_box).collect();
        let mut context = PieceContext {
            world,
            resources,
            blocks,
            random,
            chunk_box,
            chunk: ChunkPos::new(chunk_box.min[0] >> 4, chunk_box.min[2] >> 4),
            reference,
        };
        p.after_place(&mut context, &boxes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn registry(structures: &[(&str, Value)], sets: &[(&str, Value)]) -> StructureRegistry {
        let text = |l: &[(&str, Value)]| {
            l.iter()
                .map(|(k, v)| ((*k).to_owned(), v.to_string()))
                .collect::<BTreeMap<_, _>>()
        };
        // Biome tags: `#t:one` is biome 1, `#t:two` biome 2.
        StructureRegistry::new(&text(structures), &text(sets), BTreeMap::new(), &|v| {
            Ok(match v.as_str() {
                Some("#t:one") => BTreeSet::from([BiomeId(1)]),
                Some("#t:two") => BTreeSet::from([BiomeId(2)]),
                _ => BTreeSet::new(),
            })
        })
        .unwrap()
    }

    #[test]
    fn exclusion_zones_may_skip_the_other_sets_own() {
        let set = |avoid: Value| {
            json!({"structures": [{"structure": "t:s", "weight": 1}],
                "placement": {"type": "minecraft:random_spread", "spacing": 1, "separation": 0,
                    "salt": 1, "horizon:exclusion_zone": avoid}})
        };
        let r = registry(
            &[("t:s", json!({"type": "minecraft:igloo", "biomes": []}))],
            &[
                ("t:c", set(json!({"other_sets": [], "chunk_count": 0}))),
                // Every chunk is b's spread chunk, and c keeps b out of all.
                ("t:b", set(json!({"other_sets": ["t:c"], "chunk_count": 1}))),
                (
                    "t:full",
                    set(json!({"other_sets": ["t:b"], "chunk_count": 1})),
                ),
                (
                    "t:spread",
                    set(json!({"other_sets": ["t:b"], "chunk_count": 1, "their_zones": false})),
                ),
            ],
        );
        let chunk = |name: &str| {
            let s = r.sets.iter().find(|s| s.name == name).unwrap();
            r.is_structure_chunk(s, 0, 5, -3, &Rings::new())
        };
        assert!(!chunk("t:b"));
        assert!(
            chunk("t:full"),
            "b places nothing once its own zone is checked"
        );
        assert!(!chunk("t:spread"), "b's spread alone is in range");
    }

    #[test]
    fn ring_sections_keep_the_odd_rings() {
        let rings = RingSections::decode(
            &json!({"distance_to_first_ring": 80, "thickness": 96, "max_section": 3}),
        );
        // (d + 16) / 96: 0 inside 80 chunks, 1 to 175, 2 to 271, 3 to 367.
        assert!(!rings.allows(79, 0));
        assert!(rings.allows(80, 0));
        assert!(rings.allows(0, -175));
        assert!(!rings.allows(176, 0));
        assert!(rings.allows(272, 0));
        assert!(!rings.allows(464, 0), "section 5 is past max_section 3");
        // Java's int square wraps: 50,000² + 0 is negative, its root NaN, 0.
        assert!(!rings.allows(50_000, 0));
        let open = RingSections::decode(&json!({"distance_to_first_ring": 80, "thickness": 96}));
        assert!(open.allows(464, 0));
    }

    #[test]
    fn avoid_biomes_samples_around_the_middle_block() {
        let r = registry(
            &[],
            &[(
                "yung:temples",
                json!({"structures": [], "placement": {"type": "minecraft:random_spread",
                    "spacing": 24, "separation": 8, "salt": 1,
                    "horizon:avoid_biomes": {"biomes": ["#t:one", "#t:two"],
                        "radius": 48, "step": 2, "y": 120}}}),
            )],
        );
        let avoid = r.sets[0].avoid_biomes.as_ref().unwrap();
        assert_eq!(avoid.biomes, BTreeSet::from([BiomeId(1), BiomeId(2)]));
        // Chunk (1, -1): middle block (24, 120, -8), quart (6, 30, -2).
        let mut asked = Vec::new();
        assert!(!avoid.near(ChunkPos::new(1, -1), |q| {
            asked.push(q);
            BiomeId(0)
        }));
        assert_eq!(asked[0], [6, 30, -2]);
        // Every even offset within 12 quarts, once: 13 x 13.
        assert_eq!(asked.len(), 169);
        assert!(asked.contains(&[6 - 12, 30, -2 + 12]));
        assert!(!asked.contains(&[7, 30, -2]));
        // An ocean on the edge of the square is found.
        assert!(avoid.near(ChunkPos::new(1, -1), |q| {
            BiomeId(if q == [18, 30, 10] { 2 } else { 0 })
        }));
    }
}
