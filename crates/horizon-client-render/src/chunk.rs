//! Resident client chunk storage. Sections are reference-counted so worker
//! threads can mesh and light immutable snapshots while the game thread edits
//! (copy-on-write through `Arc::make_mut`).
use horizon_core::paletted::{PalettedContainer, Strategy};
use horizon_world::nbt::{Compound, Tag};
use horizon_world::registry::BlockStateId;
use std::sync::{Arc, OnceLock};

pub mod block_entity;

pub const MIN_Y: i32 = -64;
pub const HEIGHT: i32 = 384;
pub const SECTIONS: usize = (HEIGHT / 16) as usize;

/// Biomes are 4x4x4 containers over the biome table (67 entries: a 7-bit
/// global palette).
pub const BIOME_STRATEGY: Strategy = Strategy::biomes(7);

/// Bits of the block states' global palette: 16 for vanilla's 35,723 states
/// (what vanilla servers and clients send), more when content mods add states
/// past 65,536 (docs/modpacks.md). Set once, before the first chunk.
static BLOCK_BITS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(16);

/// Block states are `PalettedContainer`s over the registry: vanilla's 16-bit
/// global palette, wider with enough content mods.
pub fn block_strategy() -> Strategy {
    Strategy::blocks(BLOCK_BITS.load(std::sync::atomic::Ordering::Relaxed))
}

/// Frees `columns` (`count` of them) on a short-lived thread of its own
/// when there are many: a teleport or a dimension change drops a few
/// thousand columns at once, and freeing their sections took the frame
/// 15-20 ms. A few are freed here.
pub fn free_columns<T: Send + 'static>(columns: T, count: usize) {
    if count < 64 {
        return;
    }
    // Should the thread not start, the closure drops `columns` here.
    let _ = std::thread::Builder::new()
        .name("Chunk freeing".into())
        .spawn(move || drop(columns));
}

/// Sizes the block states' global palette for a registry of `states`.
pub fn set_block_states(states: usize) {
    let bits = horizon_core::paletted::ceil_log2(states).max(16);
    BLOCK_BITS.store(bits, std::sync::atomic::Ordering::Relaxed);
}
/// Packed light per block: sky light in the high nibble, block light in the low one.
#[derive(Clone)]
pub enum Light {
    Uniform(u8),
    Full(Box<[u8; 4096]>),
}
impl Light {
    #[inline]
    pub fn get(&self, i: usize) -> u8 {
        match self {
            Self::Uniform(v) => *v,
            Self::Full(a) => a[i],
        }
    }
}
#[inline]
fn column(x: i32, z: i32) -> usize {
    ((z & 15) << 4 | (x & 15)) as usize
}
#[inline]
pub fn index(x: i32, y: i32, z: i32) -> usize {
    ((y & 15) << 8 | (z & 15) << 4 | (x & 15)) as usize
}
#[derive(Clone)]
pub struct Section {
    pub blocks: PalettedContainer,
    /// Biome table index per 4x4x4 cell, `(y << 2 | z) << 2 | x`.
    pub biomes: PalettedContainer,
    pub light: Light,
}
impl Default for Section {
    fn default() -> Self {
        Self {
            blocks: PalettedContainer::new(block_strategy(), 0),
            biomes: PalettedContainer::new(BIOME_STRATEGY, 0),
            light: Light::Uniform(0xF0),
        }
    }
}
impl Section {
    #[inline]
    pub fn get(&self, i: usize) -> u32 {
        self.blocks.get(i)
    }
    pub fn set(&mut self, i: usize, v: u32) -> u32 {
        self.blocks.set(i, v)
    }
    /// All 4,096 block states in index order.
    pub fn unpack(&self, out: &mut [u32; 4096]) {
        self.blocks.unpack(out);
    }
    /// The block states of a whole section at once.
    pub fn from_blocks(values: &[u32; 4096]) -> PalettedContainer {
        PalettedContainer::from_values(block_strategy(), values)
    }
    #[inline]
    pub fn light(&self, i: usize) -> u8 {
        self.light.get(i)
    }
    pub fn is_uniform(&self, v: u32) -> bool {
        self.blocks.is_uniform(v)
    }
    /// Heap bytes held by the block, biome and light storage (benchmark accounting).
    pub fn heap_bytes(&self) -> usize {
        self.blocks.heap_bytes()
            + self.biomes.heap_bytes()
            + match &self.light {
                Light::Uniform(_) => 0,
                Light::Full(_) => 4096,
            }
    }
    /// Collapse storage that became uniform, and drop palette entries no
    /// block uses any more.
    pub fn compact(&mut self) {
        for container in [&mut self.blocks, &mut self.biomes] {
            if container.bits() != 0
                && let Some(v) = container.uniform()
            {
                *container = PalettedContainer::new(container.strategy(), v);
            }
        }
        if let Light::Full(a) = &self.light
            && a.iter().all(|&b| b == a[0])
        {
            self.light = Light::Uniform(a[0]);
        }
    }
}
#[derive(Clone)]
pub struct Chunk {
    pub x: i32,
    pub z: i32,
    pub sections: Vec<Arc<Section>>,
    /// `Heightmap.Types.WORLD_SURFACE` per column (`z << 4 | x`): one above
    /// the highest non-air block, or `MIN_Y`. Built on the first read; a
    /// `set` that can move a column's surface drops it.
    surface: OnceLock<Box<[i16; 256]>>,
    /// `Heightmap.Types.MOTION_BLOCKING` per column, kept like `surface`.
    motion_blocking: OnceLock<Box<[i16; 256]>>,
    pub lit: bool,
    /// Light read from a save with `isLightOn`: no light job is needed.
    pub light_loaded: bool,
    /// Newly generated (not loaded from a save): eligible for generation-time spawns.
    pub fresh: bool,
    /// Block entities read from a save, moved into the world on arrival.
    pub block_entities: Vec<((i32, i32, i32), block_entity::BlockEntity)>,
    /// `CustomName` of the container block entities (shown as the screen title).
    pub block_names: Vec<((i32, i32, i32), String)>,
    /// Generation postprocessing marks (ProtoChunk.postProcessing) as world
    /// positions, applied once by LevelChunk.postProcessGeneration.
    pub postprocess: Vec<[i32; 3]>,
    /// Saved data Horizon keeps but does not simulate (save.rs).
    pub saved: Option<Arc<ChunkExtras>>,
    /// Entities read from a save (vanilla entity NBT), spawned on arrival.
    pub entities: Vec<horizon_world::nbt::Tag>,
    /// Structure starts reaching into this chunk (`structures.References`):
    /// structure id and the start's chunk.
    pub structure_refs: Vec<(Arc<str>, (i32, i32))>,
    /// The starts generation made in this chunk (`structures.starts`), saved
    /// with the state their placement has reached by then.
    pub structure_starts: Vec<Arc<horizon_world::worldgen::structure::start::StructureStart>>,
}
impl Chunk {
    pub fn empty(x: i32, z: i32) -> Self {
        Self {
            x,
            z,
            sections: (0..SECTIONS)
                .map(|_| Arc::new(Section::default()))
                .collect(),
            surface: OnceLock::new(),
            motion_blocking: OnceLock::new(),
            lit: false,
            light_loaded: false,
            fresh: false,
            block_entities: Vec::new(),
            block_names: Vec::new(),
            structure_refs: Vec::new(),
            structure_starts: Vec::new(),
            postprocess: Vec::new(),
            saved: None,
            entities: Vec::new(),
        }
    }
    #[inline]
    pub fn get(&self, x: i32, y: i32, z: i32) -> u32 {
        let s = (y - MIN_Y) >> 4;
        if !(0..SECTIONS as i32).contains(&s) {
            return 0;
        }
        self.sections[s as usize].get(index(x, y, z))
    }
    #[inline]
    pub fn block(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        BlockStateId(self.get(x, y, z))
    }
    /// The highest y where `matches`, scanning down. Sections holding only
    /// `skip` are passed over whole: `matches` must be false for it.
    pub fn highest(&self, skip: u32, mut matches: impl FnMut(i32) -> bool) -> Option<i32> {
        self.sections
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, section)| !section.is_uniform(skip))
            .find_map(|(s, _)| {
                let base = MIN_Y + s as i32 * 16;
                (base..base + 16).rev().find(|&y| matches(y))
            })
    }
    pub fn set(&mut self, x: i32, y: i32, z: i32, v: u32) -> u32 {
        let s = (y - MIN_Y) >> 4;
        if !(0..SECTIONS as i32).contains(&s) {
            return v;
        }
        let previous = Arc::make_mut(&mut self.sections[s as usize]).set(index(x, y, z), v);
        // `Heightmap.update`: a write two or more below a map's height can't
        // move it. Any other change rebuilds that map on the next read.
        if previous != v {
            for map in [&mut self.surface, &mut self.motion_blocking] {
                if map
                    .get()
                    .is_some_and(|heights| y >= i32::from(heights[column(x, z)]) - 1)
                {
                    *map = OnceLock::new();
                }
            }
        }
        previous
    }
    /// `getHeight(Heightmap.Types.WORLD_SURFACE, x, z)` (`x`, `z` within the
    /// chunk): one above the highest block that isn't `air`, or `MIN_Y`.
    /// Sections must change only through `set` once this has been read.
    pub fn world_surface(&self, x: i32, z: i32, air: impl Fn(u32) -> bool) -> i32 {
        let surface = self.surface.get_or_init(|| self.prime(|v| !air(v)));
        i32::from(surface[column(x, z)])
    }
    /// `getHeight(Heightmap.Types.MOTION_BLOCKING, x, z)` (`x`, `z` within
    /// the chunk): one above the highest state where `blocks` (a motion
    /// blocking block or a fluid), or `MIN_Y`. Kept as `world_surface` is.
    pub fn motion_blocking(&self, x: i32, z: i32, blocks: impl Fn(u32) -> bool) -> i32 {
        let heights = self.motion_blocking.get_or_init(|| self.prime(blocks));
        i32::from(heights[column(x, z)])
    }
    /// `Heightmap.primeHeightmaps`: each section from the top, skipping
    /// those of one state that doesn't count, until every column is found.
    fn prime(&self, counts: impl Fn(u32) -> bool) -> Box<[i16; 256]> {
        let mut surface = Box::new([MIN_Y as i16; 256]);
        let mut found = [false; 256];
        let mut left = 256;
        let mut values = [0u32; 4096];
        for (s, section) in self.sections.iter().enumerate().rev() {
            if left == 0 {
                break;
            }
            if section.blocks.uniform().is_some_and(|v| !counts(v)) {
                continue;
            }
            section.unpack(&mut values);
            let base = MIN_Y + s as i32 * 16;
            for i in 0..256 {
                if found[i] {
                    continue;
                }
                if let Some(dy) = (0..16).rev().find(|&dy| counts(values[dy << 8 | i])) {
                    surface[i] = (base + dy as i32 + 1) as i16;
                    found[i] = true;
                    left -= 1;
                }
            }
        }
        surface
    }
    #[inline]
    pub fn light(&self, x: i32, y: i32, z: i32) -> u8 {
        let s = (y - MIN_Y) >> 4;
        if s < 0 {
            return 0;
        }
        if s >= SECTIONS as i32 {
            return 0xF0;
        }
        self.sections[s as usize].light(index(x, y, z))
    }
    /// Heap bytes held by the sections (benchmark accounting).
    pub fn heap_bytes(&self) -> usize {
        self.sections.iter().map(|s| s.heap_bytes()).sum::<usize>()
    }
    /// Biome table index at a block position (`ChunkAccess.getNoiseBiome` with
    /// the quart y clamped to the column).
    #[inline]
    pub fn biome(&self, x: i32, y: i32, z: i32) -> u16 {
        let qy = ((y - MIN_Y) >> 2).clamp(0, HEIGHT / 4 - 1);
        self.sections[(qy >> 2) as usize]
            .biomes
            .get(((qy & 3) << 4 | ((z & 15) >> 2) << 2 | (x & 15) >> 2) as usize) as u16
    }
    /// Sets the biome of one quart cell (`qy` counts from the column's bottom).
    pub fn set_biome_cell(&mut self, qx: i32, qy: i32, qz: i32, biome: u16) {
        let section = Arc::make_mut(&mut self.sections[(qy >> 2) as usize]);
        section.biomes.set(
            ((qy & 3) << 4 | (qz & 3) << 2 | (qx & 3)) as usize,
            u32::from(biome),
        );
    }
}

/// A proto chunk saved before it became final. Decoration writes into a
/// chunk's neighbours, and decorated chunks wait for theirs, so the chunks
/// past the finished ones hold writes that generating them again would lose
/// (whole structure pieces, trees, ores). Vanilla saves its proto chunks too.
pub struct SavedProto {
    /// Blocks, scheduled ticks (in `saved`) and post-processing marks.
    pub column: Chunk,
    /// Decorated itself (`features`), not only written to by its neighbours
    /// (`terrain`).
    pub decorated: bool,
    /// Block entity data in placement order, with `x`, `y` and `z`.
    pub block_entities: Vec<Tag>,
    /// Entities placed by structures, with absolute `Pos`.
    pub entities: Vec<Tag>,
}

/// A container's loot table and what Horizon rolled from it. Vanilla keeps
/// `LootTable` and `LootTableSeed` until the container is first opened;
/// Horizon rolls when it generates or loads the chunk, and saves the table
/// again while the container still holds exactly the rolled items.
#[derive(Clone, Debug, PartialEq)]
pub struct RolledLoot {
    pub pos: (i32, i32, i32),
    pub table: String,
    /// The saved `LootTableSeed`, written back unless 0 (trySaveLootTable).
    pub seed: Option<i64>,
    /// The rolled `Items`.
    pub items: Tag,
}
impl RolledLoot {
    /// Puts the loot table back in place of `Items` while they are still the
    /// roll; false once the container holds anything else.
    pub fn restore(&self, map: &mut Compound) -> bool {
        if map.get(&"Items".into()) != Some(&self.items) {
            return false;
        }
        map.remove(&"Items".into());
        map.insert("LootTable".into(), Tag::String(self.table.as_str().into()));
        if let Some(seed) = self.seed.filter(|s| *s != 0) {
            map.insert("LootTableSeed".into(), Tag::Long(seed));
        }
        true
    }
}

/// Save data loaded with a chunk that Horizon keeps but does not simulate.
#[derive(Clone, Debug, Default)]
pub struct ChunkExtras {
    /// Chunk root keys written back as loaded (structures, PostProcessing,
    /// InhabitedTime, blending data, ...).
    pub root: Compound,
    /// Block entities Horizon does not model.
    pub block_entities: Vec<Tag>,
    /// The NBT of the modelled block entities as loaded, to restore the fields and
    /// detail Horizon does not model when it saves them (`block_entity_nbt::overlay`).
    pub block_entity_sources: Vec<((i32, i32, i32), Tag)>,
    /// Every saved block entity's position in the order of the save's list,
    /// the order `SerializableChunkData` puts them in the chunk's maps.
    pub block_entity_order: Vec<(i32, i32, i32)>,
    /// Scheduled ticks (`block_ticks`/`fluid_ticks`): delays relative to the
    /// save time. Registered with the level's tick scheduler on arrival and
    /// replaced with the scheduler's pending ticks when saving.
    pub block_ticks: Vec<horizon_core::ticks::SavedTick>,
    pub fluid_ticks: Vec<horizon_core::ticks::SavedTick>,
    /// Game time written as `LastUpdate`.
    pub last_update: i64,
    /// Containers still holding their loot table's roll.
    pub rolled_loot: Vec<RolledLoot>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn highest_skips_only_sections_that_cannot_match() {
        let (air, cave_air, stone) = (0, 7, 1);
        let mut chunk = Chunk::empty(0, 0);
        // Stone low down, cave air (uniform in its section) above it, and a
        // section of air with one stone block higher up.
        for y in MIN_Y..MIN_Y + 40 {
            chunk.set(3, y, 4, stone);
        }
        for i in 0..4096 {
            Arc::make_mut(&mut chunk.sections[5]).set(i, cave_air);
        }
        chunk.set(3, 150, 4, stone);
        let full = |c: &Chunk, matches: &dyn Fn(u32) -> bool| {
            (MIN_Y..MIN_Y + HEIGHT)
                .rev()
                .find(|&y| matches(c.get(3, y, 4)))
        };
        for matches in [
            &(|v: u32| v != air) as &dyn Fn(u32) -> bool,
            &|v: u32| v == stone,
            &|v: u32| v == cave_air,
        ] {
            let fast = chunk.highest(air, |y| matches(chunk.get(3, y, 4)));
            assert_eq!(fast, full(&chunk, matches));
        }
        assert_eq!(
            chunk.highest(air, |y| chunk.get(3, y, 4) == stone),
            Some(150)
        );
        assert_eq!(Chunk::empty(0, 0).highest(air, |_| unreachable!()), None);
    }
    /// The kept WORLD_SURFACE heights match a scan of every column after
    /// each of a few thousand writes (air, cave air, stone, whole sections).
    #[test]
    fn world_surface_follows_writes() {
        let (air, cave_air, stone) = (0, 7, 1);
        let is_air = |v: u32| v == air || v == cave_air;
        let scan = |c: &Chunk, x: i32, z: i32| {
            (MIN_Y..MIN_Y + HEIGHT)
                .rev()
                .find(|&y| !is_air(c.get(x, y, z)))
                .map_or(MIN_Y, |y| y + 1)
        };
        let mut chunk = Chunk::empty(0, 0);
        assert_eq!(chunk.world_surface(5, 5, is_air), MIN_Y);
        for y in MIN_Y..MIN_Y + 70 {
            for i in 0..256 {
                chunk.set(i & 15, y, i >> 4, stone);
            }
        }
        for i in 0..4096 {
            Arc::make_mut(&mut chunk.sections[8]).set(i, cave_air);
        }
        let mut random = horizon_core::random::LegacyRandom::new(9);
        use horizon_core::random::RandomSource;
        for step in 0..4000 {
            let x = random.next_int_bound(16).unwrap();
            let z = random.next_int_bound(16).unwrap();
            let y = MIN_Y + 40 + random.next_int_bound(120).unwrap();
            let v = [air, cave_air, stone][random.next_int_bound(3).unwrap() as usize];
            chunk.set(x, y, z, v);
            if step % 7 == 0 {
                for i in 0..256 {
                    assert_eq!(
                        chunk.world_surface(i & 15, i >> 4, is_air),
                        scan(&chunk, i & 15, i >> 4)
                    );
                }
            }
        }
        // Out-of-range writes and unchanged states keep the map.
        let before = chunk.world_surface(3, 3, is_air);
        chunk.set(3, MIN_Y + HEIGHT, 3, stone);
        chunk.set(3, before - 1, 3, chunk.get(3, before - 1, 3));
        assert!(chunk.surface.get().is_some());
        assert_eq!(chunk.world_surface(3, 3, |_| unreachable!()), before);
    }
    /// MOTION_BLOCKING ignores states that don't count (air, grass) and
    /// follows writes as the surface map does, both maps kept at once.
    #[test]
    fn motion_blocking_follows_writes() {
        let (air, grass, water, stone) = (0, 3, 5, 1);
        let blocks = |v: u32| v == water || v == stone;
        let scan = |c: &Chunk, x: i32, z: i32| {
            (MIN_Y..MIN_Y + HEIGHT)
                .rev()
                .find(|&y| blocks(c.get(x, y, z)))
                .map_or(MIN_Y, |y| y + 1)
        };
        let mut chunk = Chunk::empty(0, 0);
        assert_eq!(chunk.motion_blocking(5, 5, blocks), MIN_Y);
        for y in MIN_Y..MIN_Y + 70 {
            for i in 0..256 {
                chunk.set(i & 15, y, i >> 4, stone);
            }
        }
        for i in 0..4096 {
            Arc::make_mut(&mut chunk.sections[8]).set(i, grass);
        }
        let mut random = horizon_core::random::LegacyRandom::new(11);
        use horizon_core::random::RandomSource;
        for step in 0..4000 {
            let x = random.next_int_bound(16).unwrap();
            let z = random.next_int_bound(16).unwrap();
            let y = MIN_Y + 40 + random.next_int_bound(120).unwrap();
            let v = [air, grass, water, stone][random.next_int_bound(4).unwrap() as usize];
            chunk.set(x, y, z, v);
            if step % 7 == 0 {
                for i in 0..256 {
                    assert_eq!(
                        chunk.motion_blocking(i & 15, i >> 4, blocks),
                        scan(&chunk, i & 15, i >> 4)
                    );
                    chunk.world_surface(i & 15, i >> 4, |v| v == air);
                }
            }
        }
    }
}
