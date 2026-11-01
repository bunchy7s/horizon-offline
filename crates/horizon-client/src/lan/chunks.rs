//! Encoding resident chunks for the wire: ClientboundLevelChunkWithLightPacket
//! (sections with their paletted containers, block entities and the sky and
//! block DataLayers) and the section block-change batches.
use crate::{
    chunk::{Chunk, ColumnLayout, LevelShape, Light, Section},
    containers::{BlockEntity, ModBlockEntity},
    fast_hash::FxHashMap,
    mobs::spawner::{SpawnDataExt, SpawnerExt},
    section_mask::SectionMask,
};
use horizon_protocol::{
    buf::{BitSet, Codec, List, Writer, set_bit},
    chunk::{
        BIOMES, BLOCKS, BlockEntityInfo, ChunkData, Heightmaps, LightData, Paletted, Strategy,
    },
    packets::play::{ChunkBiomeData, LevelChunkWithLight},
    registry::{Builtin, ceil_log2},
};
use horizon_world::nbt::{Compound, Tag};
use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

const NON_AIR: u8 = 1;
const FLUID: u8 = 2;
const ENTITY: u8 = 4;

/// A mod block entity's update tag (`BlockEntity.getUpdateTag`), what
/// clients get of it; none sends no tag.
pub type ModUpdateTag = fn([i32; 3], &ModBlockEntity) -> Option<Compound>;

/// The vanilla block entity type of a block (BlockEntityType.validBlocks).
pub fn block_entity_type(name: &str) -> Option<&'static str> {
    let n = name.strip_prefix("minecraft:")?;
    Some(match n {
        "chest" => "minecraft:chest",
        "trapped_chest" => "minecraft:trapped_chest",
        "ender_chest" => "minecraft:ender_chest",
        "barrel" => "minecraft:barrel",
        "furnace" => "minecraft:furnace",
        "blast_furnace" => "minecraft:blast_furnace",
        "smoker" => "minecraft:smoker",
        "hopper" => "minecraft:hopper",
        "dispenser" => "minecraft:dispenser",
        "dropper" => "minecraft:dropper",
        "brewing_stand" => "minecraft:brewing_stand",
        "beacon" => "minecraft:beacon",
        "bell" => "minecraft:bell",
        "campfire" | "soul_campfire" => "minecraft:campfire",
        "enchanting_table" => "minecraft:enchanting_table",
        "jukebox" => "minecraft:jukebox",
        "lectern" => "minecraft:lectern",
        "spawner" => "minecraft:mob_spawner",
        "comparator" => "minecraft:comparator",
        "daylight_detector" => "minecraft:daylight_detector",
        "conduit" => "minecraft:conduit",
        "decorated_pot" => "minecraft:decorated_pot",
        "moving_piston" => "minecraft:piston",
        "structure_block" => "minecraft:structure_block",
        "jigsaw" => "minecraft:jigsaw",
        "command_block" | "chain_command_block" | "repeating_command_block" => {
            "minecraft:command_block"
        }
        "end_portal" => "minecraft:end_portal",
        "end_gateway" => "minecraft:end_gateway",
        "sculk_sensor" => "minecraft:sculk_sensor",
        "calibrated_sculk_sensor" => "minecraft:calibrated_sculk_sensor",
        "sculk_catalyst" => "minecraft:sculk_catalyst",
        "sculk_shrieker" => "minecraft:sculk_shrieker",
        "crafter" => "minecraft:crafter",
        "potent_sulfur" => "minecraft:potent_sulfur",
        "chiseled_bookshelf" => "minecraft:chiseled_bookshelf",
        "trial_spawner" => "minecraft:trial_spawner",
        "vault" => "minecraft:vault",
        "beehive" | "bee_nest" => "minecraft:beehive",
        "suspicious_sand" | "suspicious_gravel" => "minecraft:brushable_block",
        "creaking_heart" => "minecraft:creaking_heart",
        "test_block" => "minecraft:test_block",
        "test_instance_block" => "minecraft:test_instance_block",
        n if n.ends_with("shulker_box") => "minecraft:shulker_box",
        n if n.ends_with("_wall_hanging_sign") || n.ends_with("_hanging_sign") => {
            "minecraft:hanging_sign"
        }
        n if n.ends_with("_sign") => "minecraft:sign",
        n if n.ends_with("_banner") => "minecraft:banner",
        n if n.ends_with("_head") || n.ends_with("_skull") => "minecraft:skull",
        n if n.ends_with("_shelf") => "minecraft:shelf",
        n if n.ends_with("copper_golem_statue") => "minecraft:copper_golem_statue",
        _ => return None,
    })
}

/// Per-state facts and registry ids for encoding chunks.
pub struct ChunkEncoder {
    /// What encoding a section needs, shared with the warm-up threads.
    coder: Arc<SectionCoder>,
    /// Block entity type id per state (-1 for none).
    entity_types: Vec<i32>,
    /// Whether the state is a mod block's: its type is the block's own, and
    /// it has a block entity only when the mod made one.
    mod_entities: Vec<bool>,
    /// The mods' update tags (`native_block_entity_update_tag`).
    pub mod_update_tag: Option<ModUpdateTag>,
    /// The block registry id of each state's block, for BlockEvent.
    pub block_ids: Vec<i32>,
    /// The save's tables, for the heightmaps.
    pub tables: Option<Arc<crate::save::SaveTables>>,
    /// Sections already looked at, by address (see `Encoded`).
    cache: std::sync::Mutex<FxHashMap<usize, Encoded>>,
    /// Each column's heightmaps, with the sections they were made from.
    heights: std::sync::Mutex<HashMap<(i32, i32), Heights>>,
    /// The level's sections among a column's: all of them in the Overworld,
    /// y 0 to 255 in the Nether and the End (`DimensionType` min_y and height).
    sections: std::ops::Range<usize>,
    /// The column of the level's chunks, which `sections` counts in.
    column: ColumnLayout,
    /// Whether the level has sky light (`DimensionType.hasSkyLight`).
    sky: bool,
}

/// Per-state flags and biome ids: all a section's encoding reads.
struct SectionCoder {
    flags: Vec<u8>,
    /// Network biome id per internal biome index.
    biome_ids: Vec<i32>,
    biome_bits: u8,
}

/// A column's client heightmaps, valid while it holds the same sections.
struct Heights {
    sections: Vec<std::sync::Weak<Section>>,
    maps: ColumnHeightmaps,
}

/// Heightmap type ids with their packed long arrays.
type ColumnHeightmaps = Arc<Vec<(i32, Vec<i64>)>>;

/// A column's cached heightmaps, while it holds the sections they were made
/// from.
fn cached_heights(
    heights: &HashMap<(i32, i32), Heights>,
    chunk: &Chunk,
) -> Option<ColumnHeightmaps> {
    let h = heights.get(&(chunk.x, chunk.z))?;
    let same = h.sections.len() == chunk.sections.len()
        && h.sections
            .iter()
            .zip(&chunk.sections)
            .all(|(w, s)| std::ptr::eq(w.as_ptr(), Arc::as_ptr(s)));
    same.then(|| h.maps.clone())
}

/// Cache a column's heightmaps with the sections they were made from,
/// dropping the columns whose sections are gone once there are many.
fn keep_heights(
    heights: &mut HashMap<(i32, i32), Heights>,
    key: (i32, i32),
    sections: &[Arc<Section>],
    maps: &ColumnHeightmaps,
) {
    if heights.len() >= CACHE_SWEEP / 4 {
        heights.retain(|_, h| h.sections.iter().all(|w| w.strong_count() > 0));
        if heights.len() >= CACHE_SWEEP / 8 {
            heights.clear();
        }
    }
    heights.insert(
        key,
        Heights {
            sections: sections.iter().map(Arc::downgrade).collect(),
            maps: maps.clone(),
        },
    );
}

/// What a resident section gives the wire, kept while the section is
/// unchanged. Resident sections are immutable behind `Arc` (edits go through
/// `Arc::make_mut`, which moves an edited section to a new address while a
/// `Weak` points at the old one), and the `Weak` keeps the old address from
/// being reused: the same address is the same, unchanged section.
struct Encoded {
    section: std::sync::Weak<Section>,
    non_air: bool,
    /// LevelChunkSection.write, and its block entity blocks.
    wire: Option<(Arc<[u8]>, Entities)>,
    /// The section's light as the wire's two DataLayers, when not uniform.
    light: Option<Arc<Layers>>,
}

/// A section's blocks that have a block entity type, as (index, state) in
/// index order (y, then z, then x).
type Entities = Arc<[(u16, u32)]>;

/// One section's light as DataLayers (two blocks per byte, the even one in
/// the low nibble), and whether each holds any light.
struct Layers {
    sky: Vec<u8>,
    block: Vec<u8>,
    sky_any: bool,
    block_any: bool,
}
impl Layers {
    fn of(light: &Light) -> Self {
        let (mut sky, mut block) = (vec![0u8; 2048], vec![0u8; 2048]);
        let (sky_any, block_any) = match light {
            Light::Uniform(v) => {
                let (sk, bl) = (v >> 4, v & 15);
                sky.fill(sk | sk << 4);
                block.fill(bl | bl << 4);
                (sk != 0, bl != 0)
            }
            Light::Full(a) => {
                for ((pair, sky), block) in
                    a.as_chunks::<2>().0.iter().zip(&mut sky).zip(&mut block)
                {
                    let [even, odd] = *pair;
                    *sky = even >> 4 | (odd & 0xF0);
                    *block = (even & 15) | odd << 4;
                }
                let all = a.iter().fold(0u8, |acc, &v| acc | v);
                (all & 0xF0 != 0, all & 15 != 0)
            }
        };
        Self {
            sky,
            block,
            sky_any,
            block_any,
        }
    }
}

/// `Paletted::from_values`' palette (distinct values in first-seen order)
/// and each entry's index in it, from a list palette's stored ids (`raw`
/// indexes `list`), with how many entries use each stored id. A value the
/// stored palette holds twice still gets one network index.
fn network_palette(list: &[u32], raw: &[u16]) -> (Vec<i32>, Vec<u32>, Vec<i16>) {
    let mut map = vec![u32::MAX; list.len()];
    let mut uses = vec![0i16; list.len()];
    let mut palette: Vec<i32> = Vec::with_capacity(list.len());
    let mut ids = Vec::with_capacity(raw.len());
    for &r in raw {
        let r = usize::from(r);
        uses[r] += 1;
        if map[r] == u32::MAX {
            let v = list[r] as i32;
            map[r] = match palette.iter().position(|&p| p == v) {
                Some(i) => i as u32,
                None => {
                    palette.push(v);
                    palette.len() as u32 - 1
                }
            };
        }
        ids.push(map[r]);
    }
    (palette, ids, uses)
}

impl SectionCoder {
    /// LevelChunkSection.write, and its block entity blocks.
    fn wire(&self, section: &Section) -> (Arc<[u8]>, Entities) {
        let (encoded, entities) = self.section(section);
        let mut w = horizon_protocol::buf::Writer::new();
        encoded.write(&mut w);
        (w.data.into(), entities.into())
    }

    fn section(&self, section: &Section) -> (horizon_protocol::chunk::Section, Vec<(u16, u32)>) {
        let (mut non_empty, mut fluids, mut entities) = (0i16, 0i16, Vec::new());
        let mut count = |state: u32, n: i16| {
            let f = self.flags.get(state as usize).copied().unwrap_or(0);
            if f & NON_AIR != 0 {
                non_empty += n;
                if f & FLUID != 0 {
                    fluids += n;
                }
            }
            f & ENTITY != 0
        };
        let global_bits = crate::chunk::block_strategy().global_bits;
        // One block everywhere (all air, mostly): what from_values would give.
        let uniform = section.blocks.uniform();
        let mut raw = [0u16; BLOCKS];
        let states = if let Some(v) = uniform {
            if count(v, BLOCKS as i16) {
                entities = (0..BLOCKS as u16).map(|i| (i, v)).collect();
            }
            Paletted {
                bits: 0,
                palette: vec![v as i32],
                data: Vec::new(),
            }
        } else if let Some(list) = section.blocks.unpack_ids(&mut raw) {
            // The stored palette's ids, renumbered in first-seen order: the
            // palette from_values would find in the values, without them.
            let (palette, ids, uses) = network_palette(list, &raw);
            let mut any_entity = false;
            for (&v, &n) in list.iter().zip(&uses) {
                if n > 0 {
                    any_entity |= count(v, n);
                }
            }
            if any_entity {
                for (i, &r) in raw.iter().enumerate() {
                    let v = list[usize::from(r)];
                    if self.flags.get(v as usize).copied().unwrap_or(0) & ENTITY != 0 {
                        entities.push((i as u16, v));
                    }
                }
            }
            Paletted::from_palette(palette, &ids, Strategy::Blocks, global_bits)
        } else {
            let mut unpacked = [0u32; BLOCKS];
            section.unpack(&mut unpacked);
            let mut values = Vec::with_capacity(BLOCKS);
            for (i, &v) in unpacked.iter().enumerate() {
                if count(v, 1) {
                    entities.push((i as u16, v));
                }
                values.push(v as i32);
            }
            Paletted::from_values(&values, Strategy::Blocks, global_bits)
        };
        (
            horizon_protocol::chunk::Section {
                non_empty,
                fluids,
                states,
                biomes: self.biomes(section),
            },
            entities,
        )
    }

    /// The section's biome container (`PalettedContainerRO.write`'s), in
    /// network biome ids.
    fn biomes(&self, section: &Section) -> Paletted {
        let values: Vec<i32> = (0..BIOMES)
            .map(|j| {
                let internal = section.biomes.get(j) as u16;
                self.biome_ids
                    .get(usize::from(internal))
                    .copied()
                    .unwrap_or(0)
            })
            .collect();
        Paletted::from_values(&values, Strategy::Biomes, self.biome_bits)
    }
}

/// Sections kept before the ones no longer resident are dropped.
const CACHE_SWEEP: usize = 1 << 15;

/// Uncached sections below which a batch is encoded on the tick's thread
/// alone: waking the warm-up threads costs more than a few sections.
const WARM_MIN: usize = 16;

/// The work a batch needs, each piece taken by whichever thread is free
/// first: the heightmaps of its columns not cached yet, then its sections
/// not cached yet, and what each gave (a section: its wire bytes and, with
/// varying light, its layers).
struct Warm {
    coder: Arc<SectionCoder>,
    /// The heightmaps' tables, the column's min_y and the level's (min_y,
    /// height).
    heightmaps: Option<(Arc<crate::save::SaveTables>, i32, i32, i32)>,
    columns: Vec<((i32, i32), Vec<Arc<Section>>)>,
    heights: Vec<OnceLock<ColumnHeightmaps>>,
    sections: Vec<Arc<Section>>,
    next: AtomicUsize,
    done: Vec<OnceLock<Warmed>>,
}
type Warmed = ((Arc<[u8]>, Entities), Option<Arc<Layers>>);
impl Warm {
    /// Take work until none is left.
    fn run(&self) {
        loop {
            let i = self.next.fetch_add(1, Ordering::Relaxed);
            if let Some((_, sections)) = self.columns.get(i) {
                if let Some((tables, column_min_y, min_y, height)) = &self.heightmaps {
                    let maps = crate::save::client_heightmaps_in(
                        sections,
                        *column_min_y,
                        tables,
                        *min_y,
                        *height,
                    );
                    let _ = self.heights[i].set(Arc::new(maps));
                }
                continue;
            }
            let i = i - self.columns.len();
            let Some(s) = self.sections.get(i) else {
                return;
            };
            let light = matches!(s.light, Light::Full(_)).then(|| Arc::new(Layers::of(&s.light)));
            let _ = self.done[i].set((self.coder.wire(s), light));
        }
    }
}

/// The warm-up threads (one per core besides the tick's) and the job they
/// take: its number, and the job while it has sections left to take.
struct WarmPool {
    job: std::sync::Mutex<(u64, Option<Arc<Warm>>)>,
    ready: std::sync::Condvar,
}

/// The warm-up threads, started the first time; none on a single core or
/// when no thread could start.
fn warm_pool() -> Option<&'static WarmPool> {
    static POOL: OnceLock<Option<Arc<WarmPool>>> = OnceLock::new();
    POOL.get_or_init(|| {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get()) - 1;
        let pool = Arc::new(WarmPool {
            job: Default::default(),
            ready: Default::default(),
        });
        let started = (0..threads)
            .filter(|i| {
                let pool = pool.clone();
                std::thread::Builder::new()
                    .name(format!("horizon-chunk-encode-{i}"))
                    .spawn(move || {
                        let mut done = 0;
                        loop {
                            let job = {
                                let mut slot = pool.job.lock().unwrap();
                                while slot.0 == done {
                                    slot = pool.ready.wait(slot).unwrap();
                                }
                                done = slot.0;
                                slot.1.clone()
                            };
                            if let Some(job) = job {
                                job.run();
                            }
                        }
                    })
                    .is_ok()
            })
            .count();
        (started > 0).then_some(pool)
    })
    .as_deref()
}
impl ChunkEncoder {
    /// `biome_names`: the internal biome list in order; `biome_registry`:
    /// the synchronized biome entries in network order.
    pub fn new(
        states: impl IntoIterator<Item = (Arc<str>, bool)>,
        builtin: &Builtin,
        biome_names: &[String],
        biome_registry: &[String],
    ) -> Self {
        let types = builtin.get("minecraft:block_entity_type");
        let blocks = builtin.get("minecraft:block");
        let mut flags = Vec::new();
        let mut entity_types = Vec::new();
        let mut mod_entities = Vec::new();
        let mut block_ids = Vec::new();
        for (name, has_fluid) in states {
            let air = matches!(
                &*name,
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            );
            let mut f = 0u8;
            if !air {
                f |= NON_AIR;
                if has_fluid {
                    f |= FLUID;
                }
            }
            // A mod block's block entity type is named after the block
            // (`mod_content`: registries.json).
            let modded = !name.starts_with("minecraft:");
            let t = if modded {
                Some(&*name)
            } else {
                block_entity_type(&name)
            }
            .and_then(|t| types.and_then(|ids| ids.id(t)))
            .unwrap_or(-1);
            if t >= 0 {
                f |= ENTITY;
            }
            flags.push(f);
            entity_types.push(t);
            mod_entities.push(modded && t >= 0);
            block_ids.push(blocks.and_then(|b| b.id(&name)).unwrap_or(0));
        }
        let biome_ids = biome_names
            .iter()
            .map(|n| {
                biome_registry
                    .iter()
                    .position(|b| b == n)
                    .map_or(0, |i| i as i32)
            })
            .collect();
        Self {
            coder: Arc::new(SectionCoder {
                flags,
                biome_ids,
                biome_bits: ceil_log2(biome_registry.len()),
            }),
            entity_types,
            mod_entities,
            mod_update_tag: None,
            block_ids,
            tables: None,
            cache: Default::default(),
            heights: Default::default(),
            sections: 0..ColumnLayout::VANILLA.sections(),
            column: ColumnLayout::VANILLA,
            sky: true,
        }
    }

    /// `ClientboundChunksBiomesPacket.ChunkBiomeData`: the column's biome
    /// containers, one per section of the level (`extractChunkData`).
    pub fn biome_data(&self, chunk: &Chunk) -> ChunkBiomeData {
        let biomes: Vec<Paletted> = chunk.sections[self.sections.clone()]
            .iter()
            .map(|s| self.coder.biomes(s))
            .collect();
        ChunkBiomeData::from_biomes([chunk.x, chunk.z], &biomes)
    }

    /// The client heightmaps (Heightmap.prime over the column), from the
    /// cache while the column's sections are unchanged.
    fn heightmaps(&self, chunk: &Chunk, tables: &crate::save::SaveTables) -> ColumnHeightmaps {
        let mut heights = self.heights.lock().unwrap();
        if let Some(maps) = cached_heights(&heights, chunk) {
            return maps;
        }
        let (min_y, height) = self.level();
        let maps = Arc::new(crate::save::client_heightmaps_in(
            &chunk.sections,
            chunk.min_y(),
            tables,
            min_y,
            height,
        ));
        keep_heights(&mut heights, (chunk.x, chunk.z), &chunk.sections, &maps);
        maps
    }

    /// The level's min_y and height (`DimensionType`), the heightmaps' range.
    fn level(&self) -> (i32, i32) {
        (
            self.column.section_y(self.sections.start),
            16 * self.sections.len() as i32,
        )
    }

    /// The cached entry for a section, made with `non_air` when missing.
    fn cached<T>(&self, section: &Arc<Section>, read: impl FnOnce(&mut Encoded) -> T) -> T {
        read(self.entry(&mut self.cache.lock().unwrap(), section))
    }

    /// A section's entry in the locked cache, made with `non_air` when missing.
    fn entry<'a>(
        &self,
        cache: &'a mut FxHashMap<usize, Encoded>,
        section: &Arc<Section>,
    ) -> &'a mut Encoded {
        if cache.len() >= CACHE_SWEEP {
            cache.retain(|_, e| e.section.strong_count() > 0);
            if cache.len() >= CACHE_SWEEP / 2 {
                cache.clear();
            }
        }
        cache
            .entry(Arc::as_ptr(section) as usize)
            .or_insert_with(|| Encoded {
                section: Arc::downgrade(section),
                non_air: self.non_air(section),
                wire: None,
                light: None,
            })
    }

    /// LevelChunkSection.hasOnlyAir, negated.
    fn non_air(&self, section: &Section) -> bool {
        let non_air =
            |v: u32| self.coder.flags.get(v as usize).copied().unwrap_or(0) & NON_AIR != 0;
        // The palette first: most sections are all air, or one block.
        if !section.blocks.maybe_has(non_air) {
            false
        } else if let Some(v) = section.blocks.uniform() {
            non_air(v)
        } else {
            // Stops at the first block, usually near the bottom.
            (0..BLOCKS).any(|i| non_air(section.blocks.get(i)))
        }
    }

    /// A section on the wire, from the cache or encoded now.
    fn section_wire(&self, section: &Arc<Section>) -> (Arc<[u8]>, Entities) {
        if let Some(wire) = self.cached(section, |e| e.wire.clone()) {
            return wire;
        }
        let wire = self.coder.wire(section);
        self.cached(section, |e| e.wire = Some(wire.clone()));
        wire
    }
    /// Encode for a level of `shape`: its chunks hold `shape.column()`, of
    /// which the level's own sections go on the wire.
    pub fn set_dimension(&mut self, shape: LevelShape) {
        let column = shape.column();
        let lo = usize::try_from((shape.min_y - column.min_y()) >> 4).unwrap_or(0);
        let n = usize::try_from(shape.height >> 4).unwrap_or(0);
        self.column = column;
        self.sections = lo.min(column.sections())..(lo + n).min(column.sections());
        self.sky = shape.sky_light;
        // The heightmaps depend on the level's height.
        self.heights.get_mut().unwrap().clear();
    }
    pub fn entity_type(&self, state: u32) -> Option<i32> {
        self.entity_types
            .get(state as usize)
            .filter(|t| **t >= 0)
            .copied()
    }
    /// The update tag of the block entity at `pos` (`getUpdateTag`): a
    /// mod's from its hook, a vanilla one's from [`entity_tag`].
    pub fn update_tag(&self, pos: (i32, i32, i32), entity: &BlockEntity) -> Option<Tag> {
        match entity {
            BlockEntity::Mod(m) => self
                .mod_update_tag
                .and_then(|tag| tag([pos.0, pos.1, pos.2], m))
                .map(Tag::Compound),
            _ => entity_tag(entity),
        }
    }
    /// Whether the state holds a fluid (water, lava, or a waterlogged block).
    pub fn has_fluid(&self, state: u32) -> bool {
        self.coder
            .flags
            .get(state as usize)
            .is_some_and(|f| f & FLUID != 0)
    }
    pub fn is_air(&self, state: u32) -> bool {
        self.coder
            .flags
            .get(state as usize)
            .is_none_or(|f| f & NON_AIR == 0)
    }

    /// The chunk column with its light and block entities.
    /// The non-air sections of a column as bits (LevelChunkSection.hasOnlyAir).
    fn occupied(&self, chunk: &Chunk) -> SectionMask {
        let mut bits = SectionMask::EMPTY;
        // One lock for the column: body_in asks for nine columns' sections.
        let mut cache = self.cache.lock().unwrap();
        for (i, s) in chunk.sections[self.sections.clone()].iter().enumerate() {
            if self.entry(&mut cache, s).non_air {
                bits.insert(i);
            }
        }
        bits
    }

    /// The non-air sections of the column and its neighbours, as bits. A
    /// neighbour still a proto chunk counts too, as vanilla's light storage
    /// counts a neighbour at INITIALIZE_LIGHT (#4294).
    fn around(
        &self,
        chunk: &Chunk,
        world: &crate::world::World,
        protos: &dyn Fn(i32, i32) -> Option<SectionMask>,
    ) -> SectionMask {
        let mut around = SectionMask::EMPTY;
        for dz in -1..=1 {
            for dx in -1..=1 {
                let (x, z) = (chunk.x + dx, chunk.z + dz);
                if let Some(c) = world.chunks.get(&(x, z)) {
                    around |= self.occupied(c);
                } else if let Some(sections) = protos(x, z) {
                    // Column sections: this level's range from 0.
                    for i in sections.iter() {
                        if self.sections.contains(&i) {
                            around.insert(i - self.sections.start);
                        }
                    }
                }
            }
        }
        around
    }

    /// The chunk column with its light and block entities, as the body
    /// `Clientbound::encode` gives for its LevelChunkWithLight. `world` gives
    /// the neighbouring columns, whose sections decide which light layers
    /// exist. It is written once into a buffer of its final size: the cached
    /// sections and light layers go straight to it, with no packet built and
    /// copied again first.
    pub fn body_in(&self, chunk: &Chunk, world: &crate::world::World) -> Vec<u8> {
        self.body_with(chunk, world, &|_, _| None)
    }
    /// [`Self::body_in`], with `protos` giving the non-air sections of a
    /// neighbour that is still a proto chunk.
    pub fn body_with(
        &self,
        chunk: &Chunk,
        world: &crate::world::World,
        protos: &dyn Fn(i32, i32) -> Option<SectionMask>,
    ) -> Vec<u8> {
        self.body(
            chunk,
            &world.block_entities,
            self.around(chunk, world, protos),
        )
    }

    /// An encoder of the same level with caches of its own, for threads that encode saved chunks no
    /// level holds (`lan::far`).
    pub fn detached(&self) -> Self {
        Self {
            coder: self.coder.clone(),
            entity_types: self.entity_types.clone(),
            mod_entities: self.mod_entities.clone(),
            mod_update_tag: self.mod_update_tag,
            block_ids: self.block_ids.clone(),
            tables: self.tables.clone(),
            cache: Default::default(),
            heights: Default::default(),
            sections: self.sections.clone(),
            column: self.column,
            sky: self.sky,
        }
    }
    /// The body of a column read from the save that no level holds: its own block entities, and every
    /// light section (as `light_update`; no neighbour is at hand to tell which exist). Nothing of it
    /// stays cached: the column is dropped once sent.
    pub fn body_alone(&self, chunk: &Chunk) -> Vec<u8> {
        let entities: HashMap<_, _> = chunk.block_entities.iter().cloned().collect();
        let body = self.body(chunk, &entities, SectionMask::ALL);
        let mut cache = self.cache.lock().unwrap();
        for s in &chunk.sections {
            cache.remove(&(Arc::as_ptr(s) as usize));
        }
        drop(cache);
        self.heights.lock().unwrap().remove(&(chunk.x, chunk.z));
        body
    }

    /// Make what these columns need and is not cached yet (their heightmaps
    /// and sections) before their bodies are made, shared out to the warm-up
    /// threads: a batch of new terrain costs mostly its sections' first
    /// encoding. This thread takes work too until none is left, then caches
    /// what is done; it never waits for the other threads (they keep their
    /// results to themselves until then), and what one of them is still on
    /// is made again by `body_in`, the same.
    pub fn warm(&self, columns: &[&Chunk]) {
        let Some(job) = self.job(columns) else {
            return;
        };
        let Some(pool) = warm_pool() else {
            return;
        };
        let job = Arc::new(job);
        {
            let mut slot = pool.job.lock().unwrap();
            slot.0 += 1;
            slot.1 = Some(job.clone());
        }
        pool.ready.notify_all();
        job.run();
        // Everything is claimed: the job leaves the slot.
        {
            let mut slot = pool.job.lock().unwrap();
            if slot.1.as_ref().is_some_and(|j| Arc::ptr_eq(j, &job)) {
                slot.1 = None;
            }
        }
        self.keep(&job);
    }

    /// The work `warm` shares out: the heightmaps of the columns and the
    /// sections not cached yet; none when there is too little to wake the
    /// warm-up threads for.
    fn job(&self, columns: &[&Chunk]) -> Option<Warm> {
        let stale: Vec<((i32, i32), Vec<Arc<Section>>)> = match &self.tables {
            Some(_) => {
                let heights = self.heights.lock().unwrap();
                let mut seen = crate::fast_hash::FxHashSet::default();
                columns
                    .iter()
                    .filter(|c| seen.insert((c.x, c.z)) && cached_heights(&heights, c).is_none())
                    .map(|c| ((c.x, c.z), c.sections.clone()))
                    .collect()
            }
            None => Vec::new(),
        };
        let sections: Vec<Arc<Section>> = {
            let cache = self.cache.lock().unwrap();
            columns
                .iter()
                .flat_map(|c| &c.sections[self.sections.clone()])
                .filter(|s| {
                    cache
                        .get(&(Arc::as_ptr(s) as usize))
                        .is_none_or(|e| e.wire.is_none())
                })
                .cloned()
                .collect()
        };
        if sections.len() + stale.len() < WARM_MIN {
            return None;
        }
        let (min_y, height) = self.level();
        Some(Warm {
            coder: self.coder.clone(),
            heightmaps: self
                .tables
                .clone()
                .map(|t| (t, self.column.min_y(), min_y, height)),
            heights: stale.iter().map(|_| OnceLock::new()).collect(),
            columns: stale,
            done: sections.iter().map(|_| OnceLock::new()).collect(),
            sections,
            next: AtomicUsize::new(0),
        })
    }

    /// Cache what a job's threads have finished.
    fn keep(&self, job: &Warm) {
        {
            let mut heights = self.heights.lock().unwrap();
            for ((key, sections), done) in job.columns.iter().zip(&job.heights) {
                if let Some(maps) = done.get() {
                    keep_heights(&mut heights, *key, sections, maps);
                }
            }
        }
        let mut cache = self.cache.lock().unwrap();
        for (s, done) in job.sections.iter().zip(&job.done) {
            if let Some((wire, light)) = done.get() {
                let e = self.entry(&mut cache, s);
                e.wire = Some(wire.clone());
                if light.is_some() {
                    e.light = light.clone();
                }
            }
        }
    }

    fn body(
        &self,
        chunk: &Chunk,
        block_entities: &HashMap<(i32, i32, i32), BlockEntity>,
        around: SectionMask,
    ) -> Vec<u8> {
        let (wires, entities) = self.blocks(chunk, block_entities);
        let heightmaps = self.heightmaps_of(chunk);
        let lights = self.light_sections(chunk, around);
        let buffer: usize = wires.iter().map(|w| w.len()).sum();
        // Before the light: their tags' size is only known once written.
        let mut tags = Writer::new();
        List::<BlockEntityInfo>::write(&entities, &mut tags);
        // The masks, counts and lengths fit the 64 bytes.
        let size = 64
            + heightmaps
                .iter()
                .map(|(_, h)| 10 + 8 * h.len())
                .sum::<usize>()
            + buffer
            + tags.data.len()
            + lights.len() * 2 * (2048 + 3);
        let mut w = Writer {
            data: Vec::with_capacity(size),
        };
        w.varint(horizon_protocol::ids::play::clientbound::LEVEL_CHUNK_WITH_LIGHT);
        w.i32(chunk.x);
        w.i32(chunk.z);
        Heightmaps::write(&*heightmaps, &mut w);
        w.varint(buffer as i32);
        for wire in &wires {
            w.bytes(wire);
        }
        w.bytes(&tags.data);
        write_light(&lights, self.sky, &mut w);
        w.data
    }

    /// ClientboundLightUpdatePacket for the light sections over `changed` (a
    /// bit per section of the column, `sectionLightChanged`), as vanilla's
    /// `ClientboundLightUpdatePacketData` writes them, with both kinds of
    /// each section (Horizon's light engine doesn't tell them apart; the
    /// client keeps the same light either way). A section changed as its
    /// last block went (light dimming around a removed block) is sent too:
    /// a client not tracking the block would otherwise keep its old light.
    pub fn light_update(&self, chunk: &Chunk, changed: SectionMask) -> Vec<u8> {
        let first = self.sections.start;
        let lights: Vec<(usize, SectionLight)> = self
            .light_sections(chunk, SectionMask::ALL)
            .into_iter()
            .filter(|&(b, _)| {
                // Light section b holds the column's section first + b - 1.
                (1..=self.sections.len()).contains(&b) && changed.contains(first + b - 1)
            })
            .collect();
        horizon_protocol::packets::play::Clientbound::LightUpdate(
            horizon_protocol::packets::play::LightUpdate {
                x: chunk.x,
                z: chunk.z,
                light: light_data_of(&lights, self.sky),
            },
        )
        .encode()
    }

    pub fn encode(
        &self,
        chunk: &Chunk,
        block_entities: &HashMap<(i32, i32, i32), BlockEntity>,
    ) -> LevelChunkWithLight {
        let mut packet = self.encode_blocks(chunk, block_entities);
        packet.light = self.light_data(chunk, SectionMask::ALL);
        packet
    }

    /// The packet without its light.
    fn encode_blocks(
        &self,
        chunk: &Chunk,
        block_entities: &HashMap<(i32, i32, i32), BlockEntity>,
    ) -> LevelChunkWithLight {
        let (wires, block_entities) = self.blocks(chunk, block_entities);
        LevelChunkWithLight {
            x: chunk.x,
            z: chunk.z,
            chunk: ChunkData {
                heightmaps: self.heightmaps_of(chunk).to_vec(),
                buffer: wires.concat(),
                block_entities,
            },
            light: LightData {
                sky_mask: Vec::new(),
                block_mask: Vec::new(),
                empty_sky_mask: Vec::new(),
                empty_block_mask: Vec::new(),
                sky: Vec::new(),
                block: Vec::new(),
            },
        }
    }

    fn heightmaps_of(&self, chunk: &Chunk) -> ColumnHeightmaps {
        self.tables
            .as_ref()
            .map(|t| self.heightmaps(chunk, t))
            .unwrap_or_default()
    }

    /// The level's sections on the wire, and the column's block entities in
    /// the packet's order.
    fn blocks(
        &self,
        chunk: &Chunk,
        block_entities: &HashMap<(i32, i32, i32), BlockEntity>,
    ) -> (Vec<Arc<[u8]>>, Vec<BlockEntityInfo>) {
        let mut wires = Vec::with_capacity(self.sections.len());
        let mut entity_infos = Vec::new();
        for (i, s) in chunk
            .sections
            .iter()
            .enumerate()
            .skip(self.sections.start)
            .take(self.sections.len())
        {
            let (encoded, entities) = self.section_wire(s);
            // The section's block entity blocks, kept with its bytes, in the
            // order a scan of every block would find them.
            for &(index, state) in entities.iter() {
                let Some(kind) = self.entity_type(state) else {
                    continue;
                };
                let (x, y, z) = (
                    i32::from(index & 15),
                    i32::from(index >> 8),
                    i32::from(index >> 4 & 15),
                );
                let wy = chunk.min_y() + (i as i32) * 16 + y;
                let pos = (chunk.x * 16 + x, wy, chunk.z * 16 + z);
                let entity = block_entities.get(&pos);
                // A mod block whose mod made no block entity has none to send.
                if self.mod_entities[state as usize] && !matches!(entity, Some(BlockEntity::Mod(_)))
                {
                    continue;
                }
                entity_infos.push(BlockEntityInfo {
                    packed_xz: ((x << 4) | z) as i8,
                    y: wy as i16,
                    kind,
                    // BlockEntityInfo.create: an empty update tag is not sent.
                    tag: entity
                        .and_then(|e| self.update_tag(pos, e))
                        .or_else(|| pot_tag(chunk, pos))
                        .filter(|t| !matches!(t, Tag::Compound(c) if c.is_empty()))
                        .unwrap_or(Tag::End),
                });
            }
            wires.push(encoded);
        }
        // A loaded chunk's block entities enter its maps in the save's order
        // (ChunkAccess.pendingBlockEntities, then LevelChunk.blockEntities):
        // keys that share a bucket keep that order. Others follow in scan order.
        if let Some(order) = chunk.saved.as_ref().map(|s| &s.block_entity_order)
            && !order.is_empty()
        {
            let at = |e: &BlockEntityInfo| {
                let pos = (
                    chunk.x * 16 + i32::from(e.packed_xz as u8 >> 4),
                    i32::from(e.y),
                    chunk.z * 16 + i32::from(e.packed_xz as u8 & 15),
                );
                order.iter().position(|p| *p == pos).unwrap_or(usize::MAX)
            };
            entity_infos.sort_by_key(at);
        }
        java_hash_order(&mut entity_infos, |e| {
            (
                chunk.x * 16 + i32::from(e.packed_xz as u8 >> 4),
                i32::from(e.y),
                chunk.z * 16 + i32::from(e.packed_xz as u8 & 15),
            )
        });
        (wires, entity_infos)
    }

    /// ClientboundLightUpdatePacketData: the light layers of the sections
    /// that exist around the column (`around`: the non-air sections of it
    /// and its neighbours). A resident section's layers come from
    /// the cache while it is unchanged (uniform light is quicker to fill than
    /// to keep).
    fn light_data(&self, chunk: &Chunk, around: SectionMask) -> LightData {
        light_data_of(&self.light_sections(chunk, around), self.sky)
    }

    fn light_sections(&self, chunk: &Chunk, around: SectionMask) -> Vec<(usize, SectionLight)> {
        light_sections(&chunk.sections[self.sections.clone()], around, |s| {
            if let Light::Uniform(v) = s.light {
                return SectionLight::Uniform(v);
            }
            SectionLight::Layers(self.cached(s, |e| e.light.clone()).unwrap_or_else(|| {
                let layers = Arc::new(Layers::of(&s.light));
                self.cached(s, |e| e.light = Some(layers.clone()));
                layers
            }))
        })
    }
}

/// The order LevelChunk.getBlockEntities() iterates in: a fastutil
/// Object2ObjectOpenHashMap keyed by BlockPos (linear probing from
/// HashCommon.mix(hashCode), grown at 3/4 by reinserting from the top of
/// the table, iterated from the top down), filled in the order of the
/// java.util.HashMap of pending block entities that the loaded chunk's list
/// (`items` in section order) went through.
fn java_hash_order<T>(items: &mut Vec<T>, pos: impl Fn(&T) -> (i32, i32, i32)) {
    // Vec3i.hashCode.
    let hash = |&(x, y, z): &(i32, i32, i32)| {
        y.wrapping_add(z.wrapping_mul(31))
            .wrapping_mul(31)
            .wrapping_add(x)
    };
    let keys: Vec<i32> = items.iter().map(|i| hash(&pos(i))).collect();
    // java.util.HashMap: buckets in index order, insertion order within one.
    let mut capacity = 16usize;
    while keys.len() * 4 > capacity * 3 {
        capacity *= 2;
    }
    let mut order: Vec<usize> = (0..keys.len()).collect();
    order.sort_by_key(|&i| {
        ((keys[i] ^ ((keys[i] as u32) >> 16) as i32) as u32 as usize) & (capacity - 1)
    });
    // Object2ObjectOpenHashMap.
    let mix = |h: i32| {
        let h = h.wrapping_mul(0x9E37_79B9_u32 as i32) as u32;
        (h ^ (h >> 16)) as usize
    };
    let put = |table: &mut Vec<Option<usize>>, i: usize| {
        let mask = table.len() - 1;
        let mut at = mix(keys[i]) & mask;
        while table[at].is_some() {
            at = (at + 1) & mask;
        }
        table[at] = Some(i);
    };
    // Default expected size 16: HashCommon.arraySize(16, 0.75) = 32 slots.
    let mut table: Vec<Option<usize>> = vec![None; 32];
    for (size, &i) in order.iter().enumerate() {
        put(&mut table, i);
        // insert: `if (size++ >= maxFill) rehash(arraySize(size + 1, f))`.
        let max_fill = (table.len() * 3).div_ceil(4).min(table.len() - 1);
        if size >= max_fill {
            let grown = ((size + 2) * 4).div_ceil(3).next_power_of_two().max(2);
            let old = std::mem::replace(&mut table, vec![None; grown]);
            for &j in old.iter().rev().flatten() {
                put(&mut table, j);
            }
        }
    }
    let iteration: Vec<usize> = table.iter().rev().flatten().copied().collect();
    let mut slots: Vec<Option<T>> = items.drain(..).map(Some).collect();
    items.extend(iteration.into_iter().filter_map(|i| slots[i].take()));
}

/// The update tag of a block entity (getUpdateTag) for what the client shows.
pub fn entity_tag(entity: &BlockEntity) -> Option<Tag> {
    match entity {
        BlockEntity::Sign(sign) => Some(Tag::compound(sign.encode())),
        BlockEntity::Spawner(s) => Some(s.update_tag()),
        BlockEntity::Brushable(b) => {
            let mut entries = Vec::new();
            if let Some(face) = b.hit_direction {
                entries.push(("hit_direction", Tag::Byte(face as i8)));
            }
            if let Some(item) = &b.item {
                entries.push(("item", crate::save::item_to_nbt(item)));
            }
            Some(Tag::compound(entries))
        }
        // StructureBlockEntity.getUpdateTag: saveCustomOnly.
        BlockEntity::StructureBlock(b) => Some(Tag::compound(b.save())),
        // JigsawBlockEntity.getUpdateTag: saveCustomOnly.
        BlockEntity::JigsawBlock(b) => Some(Tag::compound(b.save())),
        // TestBlockEntity.getUpdateTag: saveCustomOnly.
        BlockEntity::TestBlock(b) => Some(Tag::compound(b.save())),
        // TestInstanceBlockEntity.getUpdateTag: saveCustomOnly.
        BlockEntity::TestInstanceBlock(b) => Some(Tag::compound(b.save())),
        BlockEntity::Shelf(s) => Some(Tag::compound([
            ("Items", crate::save::encode_items(&s.items)),
            (
                "align_items_to_bottom",
                Tag::Byte(i8::from(s.align_to_bottom)),
            ),
        ])),
        // BannerBlockEntity.getUpdateTag: saveWithoutMetadata, so the patterns,
        // the custom name and the components.
        BlockEntity::Banner(b) => {
            let mut entries = Vec::new();
            if !b.layers.is_empty() {
                entries.push(("patterns", crate::banner::layers_to_nbt(&b.layers)));
            }
            if let Some(name) = &b.name {
                entries.push(("CustomName", Tag::String(name.as_str().into())));
            }
            entries.push(("components", Tag::Compound(b.components.clone())));
            Some(Tag::compound(entries))
        }
        // VaultBlockEntity.getUpdateTag: the shared data, which holds the display item.
        BlockEntity::Vault(v) => Some(v.update_tag()),
        // TrialSpawnerStateData.getUpdateTag: the spawn data the mob is drawn from.
        // next_mob_spawns_at (ACTIVE only) is left out: nothing in the client
        // reads it, and the block state it depends on is not known here.
        BlockEntity::TrialSpawner(t) => t
            .next_spawn_data
            .as_ref()
            .map(|d| Tag::compound([("spawn_data", d.encode())])),
        // SkullBlockEntity: the profile is the whole reason a head differs.
        BlockEntity::Skull(s) => {
            let mut entries = Vec::new();
            if let Some(profile) = &s.profile {
                entries.push(("profile", profile.clone()));
            }
            if let Some(sound) = &s.note_block_sound {
                entries.push(("note_block_sound", Tag::String(sound.as_str().into())));
            }
            if let Some(name) = &s.custom_name {
                entries.push(("custom_name", name.clone()));
            }
            Some(Tag::compound(entries))
        }
        // BeaconBlockEntity.getUpdateTag: saveCustomOnly, the effects and the
        // levels (no name or lock is kept here).
        BlockEntity::Beacon(b) => {
            let mut entries = Vec::new();
            if let Some(effect) = &b.primary {
                entries.push(("primary_effect", Tag::String(effect.as_str().into())));
            }
            if let Some(effect) = &b.secondary {
                entries.push(("secondary_effect", Tag::String(effect.as_str().into())));
            }
            entries.push(("Levels", Tag::Int(b.levels)));
            Some(Tag::compound(entries))
        }
        // TheEndGatewayBlockEntity.getUpdateTag: saveCustomOnly, so the age the
        // client's beam counts from, the exit and ExactTeleport when set.
        BlockEntity::EndGateway(g) => {
            let mut entries = vec![("Age", Tag::Long(g.age))];
            if let Some(exit) = g.exit_portal {
                entries.push(("exit_portal", Tag::IntArray(exit.to_vec())));
            }
            if g.exact_teleport {
                entries.push(("ExactTeleport", Tag::Byte(1)));
            }
            Some(Tag::compound(entries))
        }
        // CreakingHeartBlockEntity.getUpdateTag: saveCustomOnly, the protector.
        BlockEntity::CreakingHeart(h) => Some(Tag::compound(
            h.creaking
                .map(|(_, uuid)| ("creaking", crate::save::uuid_tag(uuid))),
        )),
        // No getUpdatePacket: these, chests, hives, command blocks and the rest.
        BlockEntity::Jukebox(_) | BlockEntity::Lectern(_) => None,
        // CampfireBlockEntity.getUpdateTag: the items alone.
        BlockEntity::Campfire(c) => Some(Tag::compound([(
            "Items",
            crate::save::encode_items(&c.items),
        )])),
        _ => None,
    }
}

/// The update tag of the decorated pot at `pos` (DecoratedPotBlockEntity.getUpdateTag,
/// saveCustomOnly). A pot is not a block entity here: its tag is the chunk's saved one.
pub fn pot_tag(chunk: &Chunk, pos: (i32, i32, i32)) -> Option<Tag> {
    let at = chunk.saved.as_ref()?.block_entities.iter().find(|t| {
        t.get("id") == Some(&Tag::String("minecraft:decorated_pot".into()))
            && t.get("x") == Some(&Tag::Int(pos.0))
            && t.get("y") == Some(&Tag::Int(pos.1))
            && t.get("z") == Some(&Tag::Int(pos.2))
    })?;
    let entries: Vec<_> = ["sherds", "LootTable", "LootTableSeed", "item"]
        .into_iter()
        .filter_map(|k| Some((k, at.get(k)?.clone())))
        .collect();
    (!entries.is_empty()).then(|| Tag::compound(entries))
}

/// Light sections run from one below the world to one above it. A layer
/// exists where vanilla's light storage keeps one: within one section of a
/// non-air section of this column or a neighbouring one (`around`, index i
/// for section i; LayerLightSectionStorage.updateSectionStatus). An existing
/// layer is sent, or marked empty when all dark; the others are left out.
#[cfg(test)]
fn light_data(sections: &[Arc<Section>], around: SectionMask) -> LightData {
    let lights = light_sections(sections, around, |s| {
        SectionLight::Layers(Arc::new(Layers::of(&s.light)))
    });
    light_data_of(&lights, true)
}

/// A light section's two DataLayers: a resident section's from the cache,
/// or one value everywhere, which is quicker to write than to keep.
enum SectionLight {
    Layers(Arc<Layers>),
    /// Sky light in the high nibble, block light in the low one.
    Uniform(u8),
}
impl SectionLight {
    /// Whether the sky and the block layer hold any light.
    fn any(&self) -> (bool, bool) {
        match self {
            Self::Layers(l) => (l.sky_any, l.block_any),
            Self::Uniform(v) => (v >> 4 != 0, v & 15 != 0),
        }
    }
    /// Append the sky or the block layer's 2048 bytes.
    fn put(&self, sky: bool, out: &mut Vec<u8>) {
        match self {
            Self::Layers(l) => out.extend_from_slice(if sky { &l.sky } else { &l.block }),
            Self::Uniform(v) => {
                let n = if sky { v >> 4 } else { v & 15 };
                out.resize(out.len() + 2048, n | n << 4);
            }
        }
    }
}

/// The existing light sections, by index from below the world, with their
/// layers (`section` giving a section of the world's).
fn light_sections(
    sections: &[Arc<Section>],
    around: SectionMask,
    section: impl Fn(&Arc<Section>) -> SectionLight,
) -> Vec<(usize, SectionLight)> {
    let n = sections.len();
    // Light section b holds chunk section b - 1.
    let exists =
        |b: usize| (b.saturating_sub(2)..=b.min(n - 1)).any(|s| s + 2 >= b && around.contains(s));
    (0..n + 2)
        .filter(|&b| exists(b))
        .map(|b| {
            let light = match b {
                // Below the world nothing is lit; above it the sky is full.
                0 => SectionLight::Uniform(0),
                _ if b == n + 1 => SectionLight::Uniform(0xF0),
                _ => section(&sections[b - 1]),
            };
            (b, light)
        })
        .collect()
}

/// The four masks of ClientboundLightUpdatePacketData (sky, block, empty
/// sky, empty block); a level without sky light has no sky storage at all.
fn light_masks(lights: &[(usize, SectionLight)], sky: bool) -> [Vec<u8>; 4] {
    let mut masks: [Vec<u8>; 4] = Default::default();
    for (b, light) in lights {
        let (sky_any, block_any) = light.any();
        if sky {
            set_bit(&mut masks[if sky_any { 0 } else { 2 }], *b);
        }
        set_bit(&mut masks[if block_any { 1 } else { 3 }], *b);
    }
    masks
}

/// The layers sent of one kind: those with any light.
fn sent_layers(
    lights: &[(usize, SectionLight)],
    sky: bool,
) -> impl Iterator<Item = &SectionLight> + Clone {
    lights.iter().map(|(_, l)| l).filter(move |l| {
        let (sky_any, block_any) = l.any();
        if sky { sky_any } else { block_any }
    })
}

fn light_data_of(lights: &[(usize, SectionLight)], sky: bool) -> LightData {
    let [sky_mask, block_mask, empty_sky_mask, empty_block_mask] = light_masks(lights, sky);
    let layers = |kind: bool| -> Vec<Vec<u8>> {
        sent_layers(lights, kind)
            .map(|l| {
                let mut layer = Vec::with_capacity(2048);
                l.put(kind, &mut layer);
                layer
            })
            .collect()
    };
    LightData {
        sky_mask,
        block_mask,
        empty_sky_mask,
        empty_block_mask,
        sky: if sky { layers(true) } else { Vec::new() },
        block: layers(false),
    }
}

/// LightData.write, from the layers themselves.
fn write_light(lights: &[(usize, SectionLight)], sky: bool, w: &mut Writer) {
    for mask in light_masks(lights, sky) {
        BitSet::write(&mask, w);
    }
    for kind in [true, false] {
        let layers = sent_layers(lights, kind).filter(|_| sky || !kind);
        w.varint(layers.clone().count() as i32);
        for l in layers {
            w.varint(2048);
            l.put(kind, &mut w.data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{MIN_Y, SECTIONS};
    use horizon_world::nbt::JavaString;

    #[test]
    fn block_entity_blocks_are_recognised() {
        assert_eq!(
            block_entity_type("minecraft:oak_sign"),
            Some("minecraft:sign")
        );
        assert_eq!(
            block_entity_type("minecraft:oak_wall_hanging_sign"),
            Some("minecraft:hanging_sign")
        );
        assert_eq!(
            block_entity_type("minecraft:red_shulker_box"),
            Some("minecraft:shulker_box")
        );
        assert_eq!(block_entity_type("minecraft:stone"), None);
    }

    #[test]
    fn block_entities_come_from_the_cached_section_in_scan_order() {
        let builtin = Builtin::parse(
            r#"{"minecraft:block_entity_type": {"entries": {
                "minecraft:chest": {"protocol_id": 1},
                "minecraft:sign": {"protocol_id": 7}}}}"#,
        )
        .unwrap();
        let states = [
            "minecraft:air",
            "minecraft:stone",
            "minecraft:chest",
            "minecraft:oak_sign",
        ]
        .map(|n| (Arc::<str>::from(n), false));
        let encoder = ChunkEncoder::new(states, &builtin, &[], &[]);
        let mut chunk = Chunk::empty(2, -3);
        for (x, y, z, state) in [(15, 3, 0, 2), (0, 3, 15, 3), (4, 3, 4, 1), (7, 40, 2, 2)] {
            chunk.set(x, MIN_Y + y, z, state);
        }
        // Every block of a section looked at, as the scan did before the cache.
        let scan = |chunk: &Chunk| {
            let mut found = Vec::new();
            for (i, s) in chunk.sections.iter().enumerate() {
                for y in 0..16 {
                    for z in 0..16 {
                        for x in 0..16 {
                            let state = s.get(crate::chunk::index(x, y, z));
                            if let Some(kind) = encoder.entity_type(state) {
                                found.push((x, MIN_Y + i as i32 * 16 + y, z, kind));
                            }
                        }
                    }
                }
            }
            // Then the order the packet gives them, as encode_blocks does.
            java_hash_order(&mut found, |e| {
                (chunk.x * 16 + e.0, e.1, chunk.z * 16 + e.2)
            });
            found
        };
        let sent = |chunk: &Chunk| {
            let packet = encoder.encode(chunk, &HashMap::new());
            let infos = packet.chunk.block_entities;
            infos
                .iter()
                .map(|e| {
                    let xz = e.packed_xz as u8;
                    (
                        i32::from(xz >> 4),
                        i32::from(e.y),
                        i32::from(xz & 15),
                        e.kind,
                    )
                })
                .collect::<Vec<_>>()
        };
        let expected = scan(&chunk);
        assert_eq!(expected.len(), 3);
        assert_eq!(sent(&chunk), expected);
        // From the cache the second time, and after an edit.
        assert_eq!(sent(&chunk), expected);
        chunk.set(15, MIN_Y + 3, 0, 0);
        chunk.set(9, MIN_Y + 3, 9, 3);
        assert_eq!(sent(&chunk), scan(&chunk));
    }

    /// A mod block's block entity goes out with its block's own type and the
    /// tag its mod gives; a mod block whose mod made none is left out.
    #[test]
    fn mod_block_entities_carry_their_blocks_type_and_the_mods_tag() {
        let builtin = Builtin::parse(
            r#"{"minecraft:block_entity_type": {"entries": {
                "minecraft:chest": {"protocol_id": 1},
                "test:machine": {"protocol_id": 49},
                "test:casing": {"protocol_id": 50}}}}"#,
        )
        .unwrap();
        let states =
            ["minecraft:air", "test:machine", "test:casing"].map(|n| (Arc::<str>::from(n), false));
        let mut encoder = ChunkEncoder::new(states, &builtin, &[], &[]);
        encoder.mod_update_tag = Some(|pos, entity| {
            let mut tag = Compound::new();
            tag.insert(
                "active".into(),
                entity.nbt.get(&JavaString::from("active"))?.clone(),
            );
            tag.insert("x".into(), Tag::Int(pos[0]));
            Some(tag)
        });
        let mut chunk = Chunk::empty(0, 0);
        chunk.set(1, MIN_Y + 2, 3, 1);
        chunk.set(5, MIN_Y + 2, 5, 2);
        let mut machine = ModBlockEntity {
            id: "test:machine".into(),
            ..ModBlockEntity::default()
        };
        machine.nbt.insert("active".into(), Tag::Byte(1));
        machine.nbt.insert("recipe".into(), Tag::Int(7));
        let block_entities =
            HashMap::from([((1, MIN_Y + 2, 3), BlockEntity::Mod(Box::new(machine)))]);
        let infos = encoder.encode(&chunk, &block_entities).chunk.block_entities;
        assert_eq!(infos.len(), 1, "the casing has no block entity");
        assert_eq!((infos[0].kind, infos[0].y), (49, (MIN_Y + 2) as i16));
        let mut expected = Compound::new();
        expected.insert("active".into(), Tag::Byte(1));
        expected.insert("x".into(), Tag::Int(1));
        assert_eq!(infos[0].tag, Tag::Compound(expected));
        // As does an encoder for saved chunks past the view distance (`lan::far`).
        let far = encoder.detached().encode(&chunk, &block_entities);
        assert_eq!(far.chunk.block_entities[0].tag, infos[0].tag);
        // An empty tag is not sent either (BlockEntityInfo.create).
        encoder.mod_update_tag = Some(|_, _| Some(Compound::new()));
        let infos = encoder.encode(&chunk, &block_entities).chunk.block_entities;
        assert_eq!(infos[0].tag, Tag::End);
        // Without the hook, nothing of it is sent.
        encoder.mod_update_tag = None;
        let infos = encoder.encode(&chunk, &block_entities).chunk.block_entities;
        assert_eq!(infos[0].tag, Tag::End);
    }

    fn encoder() -> ChunkEncoder {
        let states = ["minecraft:air", "minecraft:stone", "minecraft:water"]
            .map(|n| (Arc::<str>::from(n), n.ends_with("water")));
        ChunkEncoder::new(states, &Builtin::default(), &[], &[])
    }

    /// The bytes on the wire for a section, encoded afresh.
    fn fresh(encoder: &ChunkEncoder, section: &Section) -> Vec<u8> {
        let mut w = horizon_protocol::buf::Writer::new();
        encoder.coder.section(section).0.write(&mut w);
        w.data
    }

    #[test]
    fn cached_sections_follow_edits() {
        let encoder = encoder();
        let mut chunk = Chunk::empty(0, 0);
        chunk.set(1, MIN_Y + 2, 3, 1);
        let before = encoder.encode(&chunk, &HashMap::new());
        assert_eq!(encoder.occupied(&chunk), SectionMask::single(0));
        assert_eq!(
            &*encoder.section_wire(&chunk.sections[0]).0,
            fresh(&encoder, &chunk.sections[0])
        );
        // An edit moves the section: the cache sees a new one.
        chunk.set(4, MIN_Y + 20, 4, 2);
        chunk.set(1, MIN_Y + 2, 3, 0);
        assert_eq!(encoder.occupied(&chunk), SectionMask::single(1));
        let after = encoder.encode(&chunk, &HashMap::new());
        assert_ne!(before.chunk.buffer, after.chunk.buffer);
        let mut expected = Vec::new();
        for s in &chunk.sections {
            expected.extend(fresh(&encoder, s));
        }
        assert_eq!(after.chunk.buffer, expected);
    }

    #[test]
    fn light_layers_pack_two_blocks_per_byte() {
        let mut chunk = Chunk::empty(0, 0);
        let mut light = Box::new([0u8; 4096]);
        for (i, v) in light.iter_mut().enumerate() {
            *v = (i * 37 % 251) as u8;
        }
        Arc::make_mut(&mut chunk.sections[3]).light = Light::Full(light.clone());
        let data = light_data(&chunk.sections, SectionMask::ALL);
        // Layer 4 holds section 3. Below the world (layer 0) there is no sky
        // light, so the sky layers listed start at layer 1; section 3 has the
        // only block light.
        let (sky, block) = (&data.sky[3], &data.block[0]);
        for j in 0..4096 {
            let (s, b) = if j & 1 == 0 {
                (sky[j >> 1] & 15, block[j >> 1] & 15)
            } else {
                (sky[j >> 1] >> 4, block[j >> 1] >> 4)
            };
            assert_eq!((s, b), (light[j] >> 4, light[j] & 15), "block {j}");
        }
    }

    #[test]
    fn cached_light_follows_edits() {
        let encoder = encoder();
        let mut chunk = Chunk::empty(0, 0);
        let mut light = Box::new([0x5Au8; 4096]);
        Arc::make_mut(&mut chunk.sections[2]).light = Light::Full(light.clone());
        let first = encoder.encode(&chunk, &HashMap::new()).light;
        assert_eq!(first, light_data(&chunk.sections, SectionMask::ALL));
        // Relighting moves the section, so the cached layers aren't reused.
        light[7] = 0xF3;
        Arc::make_mut(&mut chunk.sections[2]).light = Light::Full(light);
        let second = encoder.encode(&chunk, &HashMap::new()).light;
        assert_ne!(first, second);
        assert_eq!(second, light_data(&chunk.sections, SectionMask::ALL));
    }

    /// A neighbour still a proto chunk counts for the light layers as it
    /// will once loaded (#4294: vanilla counts a neighbour at
    /// INITIALIZE_LIGHT).
    #[test]
    fn a_proto_neighbour_counts_for_the_light_layers() {
        let encoder = encoder();
        let mut world = crate::world::World::new(horizon_world::registry::BlockStateId(0), 0);
        for x in -1..=1 {
            for z in -1..=1 {
                let mut chunk = Chunk::empty(x, z);
                chunk.set(0, MIN_Y, 0, 1);
                world.chunks.insert((x, z), Arc::new(chunk));
            }
        }
        // The neighbour east has a block ten sections up.
        let mut tall = Chunk::empty(1, 0);
        tall.set(0, MIN_Y, 0, 1);
        tall.set(0, MIN_Y + 16 * 10, 0, 1);
        world.chunks.insert((1, 0), Arc::new(tall));
        let centre = world.chunks[&(0, 0)].clone();
        let loaded = encoder.body_in(&centre, &world);
        world.chunks.remove(&(1, 0));
        assert_ne!(encoder.body_in(&centre, &world), loaded);
        let mut sections = SectionMask::EMPTY;
        sections.insert(0);
        sections.insert(10);
        let proto = |x, z| ((x, z) == (1, 0)).then_some(sections);
        assert_eq!(encoder.body_with(&centre, &world, &proto), loaded);
    }

    #[test]
    fn light_masks_cover_the_world_plus_borders() {
        let chunk = Chunk::empty(0, 0);
        // A column with nothing around it has no light layers at all.
        assert!(
            light_data(&chunk.sections, SectionMask::EMPTY)
                .sky
                .is_empty()
        );
        let data = light_data(&chunk.sections, SectionMask::ALL);
        // Default sections are full sky light and no block light.
        assert_eq!(data.sky.len(), SECTIONS + 1);
        assert_eq!(data.block.len(), 0);
        assert_eq!(data.sky[0][0], 0xFF);
        assert!(horizon_protocol::buf::bit(&data.empty_block_mask, 5));
    }

    #[test]
    fn the_body_is_the_packet_encoded() {
        let builtin = Builtin::parse(
            r#"{"minecraft:block_entity_type": {"entries": {
                "minecraft:chest": {"protocol_id": 1},
                "minecraft:sign": {"protocol_id": 7}}}}"#,
        )
        .unwrap();
        let states = [
            "minecraft:air",
            "minecraft:stone",
            "minecraft:chest",
            "minecraft:oak_sign",
        ]
        .map(|n| (Arc::<str>::from(n), false));
        let mut encoder = ChunkEncoder::new(states, &builtin, &[], &[]);
        let mut chunk = Chunk::empty(-7, 12);
        // Every kind of light section: uniform dark, uniform lit, sky and
        // block light varying, none at all.
        let mut light = Box::new([0u8; 4096]);
        for (i, v) in light.iter_mut().enumerate() {
            *v = (i * 53 % 256) as u8;
        }
        Arc::make_mut(&mut chunk.sections[4]).light = Light::Full(light);
        Arc::make_mut(&mut chunk.sections[5]).light = Light::Uniform(0x07);
        Arc::make_mut(&mut chunk.sections[6]).light = Light::Uniform(0);
        Arc::make_mut(&mut chunk.sections[7]).light = Light::Full(Box::new([0x30; 4096]));
        for (x, y, z, state) in [
            (3, 70, 9, 1),
            (5, 70, 5, 2),
            (0, -60, 15, 2),
            (15, 300, 0, 1),
            (8, 64, 1, 3),
        ] {
            chunk.set(x, y, z, state);
        }
        // A sign's update tag goes with the column.
        let mut entities = HashMap::new();
        entities.insert(
            (-7 * 16 + 8, 64, 12 * 16 + 1),
            BlockEntity::Sign(crate::signs::SignData::default()),
        );
        let packet = |encoder: &ChunkEncoder, around: SectionMask| {
            let mut packet = encoder.encode_blocks(&chunk, &entities);
            packet.light = encoder.light_data(&chunk, around);
            horizon_protocol::packets::play::Clientbound::LevelChunkWithLight(packet).encode()
        };
        let sent = encoder
            .encode_blocks(&chunk, &entities)
            .chunk
            .block_entities;
        assert_eq!(sent.len(), 3);
        assert!(sent.iter().any(|e| e.tag != Tag::End));
        for around in [
            SectionMask::EMPTY,
            SectionMask::single(0),
            SectionMask::from_iter([5, 7]),
            SectionMask::single(23),
            SectionMask::ALL,
        ] {
            assert_eq!(
                encoder.body(&chunk, &entities, around),
                packet(&encoder, around),
                "around {around:?}"
            );
        }
        encoder.set_dimension(LevelShape::NETHER);
        assert_eq!(
            encoder.body(&chunk, &entities, SectionMask::ALL),
            packet(&encoder, SectionMask::ALL)
        );
    }

    /// Columns whose sections and heightmaps the warm-up threads made give
    /// the bytes each gives on its own, also with a column twice and its
    /// neighbours' sections deciding the light layers.
    #[test]
    fn stored_palette_ids_pack_as_the_values_do() {
        // Lists with values held twice and ids no entry uses, as `set` can
        // leave them, at every list width blocks use.
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        for len in [2usize, 3, 15, 16, 17, 40, 200, 256] {
            let list: Vec<u32> = (0..len).map(|_| next(30_000) as u32).collect();
            let mut list = list;
            list[len - 1] = list[0];
            let used = (len - 1).max(1) as u64;
            let raw: Vec<u16> = (0..BLOCKS).map(|_| next(used) as u16).collect();
            let values: Vec<i32> = raw.iter().map(|&r| list[usize::from(r)] as i32).collect();
            let (palette, ids, uses) = network_palette(&list, &raw);
            assert_eq!(
                Paletted::from_palette(palette, &ids, Strategy::Blocks, 15),
                Paletted::from_values(&values, Strategy::Blocks, 15),
                "{len} stored ids"
            );
            assert_eq!(
                uses.iter().map(|&n| i32::from(n)).sum::<i32>(),
                BLOCKS as i32
            );
        }
    }

    #[test]
    fn warmed_columns_give_the_same_bodies() {
        let builtin = Builtin::parse(
            r#"{"minecraft:block_entity_type": {"entries": {
                "minecraft:sign": {"protocol_id": 7}}}}"#,
        )
        .unwrap();
        let states = [
            "minecraft:air",
            "minecraft:stone",
            "minecraft:water",
            "minecraft:oak_sign",
        ]
        .map(|n| (Arc::<str>::from(n), n.ends_with("water")));
        let mut world = crate::world::World::new(horizon_world::registry::BlockStateId(0), 0);
        for x in -2..2 {
            for z in -2..2 {
                let mut chunk = Chunk::empty(x, z);
                let n = (x * 7 + z * 13).rem_euclid(17);
                for i in 0..40 + n * 30 {
                    let (bx, bz) = ((i * 5 + n) % 16, (i * 11) % 16);
                    chunk.set(bx, MIN_Y + (i * 37 + n * 50) % 384, bz, 1 + (i % 3) as u32);
                }
                let mut light = Box::new([0u8; 4096]);
                for (i, v) in light.iter_mut().enumerate() {
                    *v = (i * 31 + n as usize) as u8;
                }
                Arc::make_mut(&mut chunk.sections[(n % 24) as usize]).light = Light::Full(light);
                world.block_entities.insert(
                    (x * 16 + 2, 70, z * 16 + 3),
                    BlockEntity::Sign(crate::signs::SignData::default()),
                );
                chunk.set(2, 70, 3, 3);
                world.chunks.insert((x, z), Arc::new(chunk));
            }
        }
        let mut keys: Vec<(i32, i32)> =
            (-2..2).flat_map(|x| (-2..2).map(move |z| (x, z))).collect();
        keys.push(keys[5]);
        let chunks: Vec<&Chunk> = keys
            .iter()
            .map(|k| &**world.chunks.get(k).unwrap())
            .collect();
        // (air, fluid, blocks motion, blocks motion and is not leaves) per state.
        let tables = Arc::new(crate::save::SaveTables::for_heightmaps(
            crate::assets::Properties::for_tests(&[
                (true, false, false, false),
                (false, false, true, true),
                (false, true, true, true),
                (false, false, false, false),
            ]),
        ));
        let mut alone = ChunkEncoder::new(states.clone(), &builtin, &[], &[]);
        alone.tables = Some(tables.clone());
        let expected: Vec<Vec<u8>> = chunks.iter().map(|c| alone.body_in(c, &world)).collect();
        let mut warmed = ChunkEncoder::new(states.clone(), &builtin, &[], &[]);
        warmed.tables = Some(tables);
        let bodies = |encoder: &ChunkEncoder| -> Vec<Vec<u8>> {
            chunks.iter().map(|c| encoder.body_in(c, &world)).collect()
        };
        assert!(chunks.len() * SECTIONS >= WARM_MIN);
        warmed.warm(&chunks);
        assert_eq!(bodies(&warmed), expected);
        // Run on this thread alone, the job makes every column's heightmaps
        // (once for the column there twice) and sections.
        let mut cold = ChunkEncoder::new(states, &builtin, &[], &[]);
        cold.tables = alone.tables.clone();
        let job = cold.job(&chunks).unwrap();
        assert_eq!((job.columns.len(), job.sections.len()), (16, 17 * SECTIONS));
        job.run();
        cold.keep(&job);
        assert_eq!(cold.heights.lock().unwrap().len(), 16);
        assert_eq!(bodies(&cold), expected);
        // Again, from the cache, with nothing left to warm.
        warmed.warm(&chunks);
        assert_eq!(bodies(&warmed), expected);
        warmed.warm(&[]);
    }

    /// A far chunk's body (`lan::far`) leaves nothing of it cached, and is the same each time.
    #[test]
    fn a_body_alone_leaves_nothing_cached() {
        let encoder = encoder();
        let mut chunk = Chunk::empty(3, -1);
        chunk.set(1, 64, 1, 1);
        let body = encoder.body_alone(&chunk);
        assert!(encoder.cache.lock().unwrap().is_empty());
        assert!(encoder.heights.lock().unwrap().is_empty());
        assert_eq!(encoder.detached().body_alone(&chunk), body);
        assert_eq!(encoder.body_alone(&chunk), body);
    }

    #[test]
    fn a_nether_column_sends_its_sixteen_sections_without_sky_light() {
        let chunk = Chunk::empty(0, 0);
        let mut encoder = ChunkEncoder::new(
            [("minecraft:air".into(), false)],
            &Builtin::default(),
            &[],
            &[],
        );
        let overworld = encoder.encode(&chunk, &HashMap::new());
        encoder.set_dimension(LevelShape::NETHER);
        let nether = encoder.encode(&chunk, &HashMap::new());
        // An all-air section is the same bytes wherever it is.
        let per_section = overworld.chunk.buffer.len() / SECTIONS;
        assert_eq!(nether.chunk.buffer.len(), per_section * 16);
        assert!(nether.light.sky.is_empty() && nether.light.sky_mask.is_empty());
        // Block light layers: one below and one above the sixteen sections.
        assert!(horizon_protocol::buf::bit(
            &nether.light.empty_block_mask,
            17
        ));
        assert!(!horizon_protocol::buf::bit(
            &nether.light.empty_block_mask,
            18
        ));
    }

    #[test]
    fn a_tall_column_sends_its_whole_height() {
        let shape = LevelShape::new(-64, 2096, 2096, true).unwrap();
        let mut chunk = Chunk::with_layout(0, 0, shape.column());
        chunk.set(3, 2000, 4, 1);
        let mut encoder = encoder();
        let vanilla = encoder.encode(&Chunk::empty(0, 0), &HashMap::new());
        encoder.set_dimension(shape);
        let tall = encoder.encode(&chunk, &HashMap::new());
        // 131 sections, one of them holding stone instead of air: longer than
        // 131 air sections, and the sky light layers reach one above the top.
        let per_section = vanilla.chunk.buffer.len() / SECTIONS;
        assert!(tall.chunk.buffer.len() > per_section * 131);
        assert_eq!(tall.light.sky.len(), 132);
    }

    #[test]
    fn beacon_gateway_and_heart_update_tags_load_on_the_client() {
        use crate::containers::{Beacon, CreakingHeart, EndGateway};
        // What a client loads from the tag (handleBlockEntityData).
        let load = |id: &str, entity: BlockEntity| {
            let Some(Tag::Compound(mut tag)) = entity_tag(&entity) else {
                panic!("no update tag for {id}");
            };
            tag.insert("id".into(), Tag::String(id.into()));
            for k in ["x", "y", "z"] {
                tag.insert(k.into(), Tag::Int(0));
            }
            crate::save::decode_block_entity(&Tag::Compound(tag))
                .expect("loads")
                .1
        };
        let beacon = Beacon {
            levels: 4,
            primary: Some("minecraft:speed".into()),
            secondary: Some("minecraft:regeneration".into()),
            ..Default::default()
        };
        let BlockEntity::Beacon(b) =
            load("minecraft:beacon", BlockEntity::Beacon(Box::new(beacon)))
        else {
            panic!("not a beacon");
        };
        assert_eq!(b.levels, 4);
        assert_eq!(b.primary.as_deref(), Some("minecraft:speed"));
        assert_eq!(b.secondary.as_deref(), Some("minecraft:regeneration"));
        let gateway = EndGateway {
            age: 150,
            exit_portal: Some([8, 100, 0]),
            exact_teleport: false,
        };
        let tag = entity_tag(&BlockEntity::EndGateway(gateway.clone())).unwrap();
        assert_eq!(tag.get("ExactTeleport"), None);
        let BlockEntity::EndGateway(g) = load(
            "minecraft:end_gateway",
            BlockEntity::EndGateway(gateway.clone()),
        ) else {
            panic!("not a gateway");
        };
        assert_eq!(g, gateway);
        let exact = EndGateway {
            exact_teleport: true,
            ..gateway
        };
        let tag = entity_tag(&BlockEntity::EndGateway(exact.clone())).unwrap();
        assert_eq!(tag.get("ExactTeleport"), Some(&Tag::Byte(1)));
        let BlockEntity::EndGateway(g) = load(
            "minecraft:end_gateway",
            BlockEntity::EndGateway(exact.clone()),
        ) else {
            panic!("not a gateway");
        };
        assert_eq!(g, exact);
        let heart = CreakingHeart {
            creaking: Some((Some(7), [1, 2, 3, 4])),
            ..Default::default()
        };
        let BlockEntity::CreakingHeart(h) = load(
            "minecraft:creaking_heart",
            BlockEntity::CreakingHeart(Box::new(heart)),
        ) else {
            panic!("not a creaking heart");
        };
        assert_eq!(h.creaking, Some((None, [1, 2, 3, 4])));
        // Without a protector the tag is empty.
        let lone = entity_tag(&BlockEntity::CreakingHeart(Box::default()));
        assert_eq!(
            lone,
            Some(Tag::Compound(horizon_world::nbt::Compound::new()))
        );
    }
}
