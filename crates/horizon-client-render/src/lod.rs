//! Bounded voxel-column terrain for distances beyond the resident chunk area.
//! Far tiles never enter World, lighting, simulation or the section cave graph.
use crate::{
    blocks::BlockFacts,
    chunk::{Chunk, MIN_Y},
    fast_hash::{FxHashMap, FxHashSet},
    mesher::SectionMesh,
    streaming::{LodSource, WorkerContext},
};
pub mod cache;
mod mesh;
use horizon_world::registry::BlockStateId;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

const CELLS: usize = 16;
const MAX_SAVED: usize = 1024;
/// How many resident chunks may wait for the worker's summary at once
/// (`Lod::observing`).
pub const OBSERVE_AHEAD: usize = 8;
/// How long a tile whose sample waited for data (`Lod::finish_waited`) waits
/// before it is sampled again without word of it.
const WAIT_RETRY: std::time::Duration = std::time::Duration::from_secs(10);
const SAMPLE_JOBS: usize = 4;
/// Sample jobs queued per sampling thread once the near terrain is in.
/// Results are taken and jobs started once a frame: with four in flight, a
/// tile taking milliseconds left the threads idle for most of a slow frame
/// (2-3 fps on a CPU renderer, with far terrain a few tiles a frame).
const QUEUED_PER_SAMPLER: usize = 32;
/// The last `refresh`es `Lod::refreshed_since` tells apart by area.
const MAX_REFRESHED: usize = 64;
/// Threads that sample far tiles beside the mesher. Sampling is the cold-load
/// bottleneck (each task evaluates a shared density lattice for 256 columns),
/// so medium machines use half their cores, not a quarter.
fn sampling_threads(cores: usize) -> usize {
    if cores >= 8 {
        (cores / 2).min(8)
    } else {
        (cores / 4).clamp(1, 3)
    }
}
/// Tile meshing used to be one worker with a whole selection batch at a time.
/// It now has its own pool, sized so ticks and the frame keep a core.
fn meshing_threads(cores: usize) -> usize {
    cores
        .saturating_sub(sampling_threads(cores) + 1)
        .clamp(1, 16)
}
/// While the near terrain loads on a machine with four threads or fewer, far
/// terrain starts one sample job at most this often and no detail: it would
/// otherwise take the cores the near chunks and the frame need.
const HELD_SAMPLE_STEP: std::time::Duration = std::time::Duration::from_millis(500);
/// How long a tile whose sampling failed (`LodSampler::grid` gave no
/// columns: a server refused or lost it) waits before it is asked again.
const RETRY: std::time::Duration = std::time::Duration::from_secs(2);
/// The coarsest tile level: `Plan` roots cover at most `MAX_CHUNKS` chunks
/// around (a level-22 tile is 67 million blocks wide: the whole world).
pub(crate) const MAX_LEVEL: u8 = 22;
/// Tiles this far from the camera (1024 chunks, as far as far terrain went
/// before it covered the world) are planned with `WORLD_SPLIT`.
const WORLD_START: i32 = 1024 * 16;
/// How many tile widths away from the camera a tile of the world-wide terrain
/// splits (`Plan::visit`: cells then span about 1/48 of the distance).
const WORLD_SPLIT: i32 = 3;
/// The farthest `Plan` reaches, in chunks.
pub const MAX_CHUNKS: u32 = 1 << 22;
/// The coarsest tile level whose chunks are generated in full (trees,
/// structures) rather than sampled: cells of up to 4 blocks, out to about
/// 1024 blocks at Full far detail.
const DETAIL_LEVEL: u8 = 2;
/// The most chunks one detail request asks for, one request each 250 ms:
/// generated ones arrive at the decoration threads' pace, cached ones at once.
const DETAIL_REQUEST: usize = 512;
/// Top solid face and optional fluid face, in absolute block coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    pub height: i32,
    pub state: BlockStateId,
    pub water: Option<(i32, BlockStateId)>,
    pub biome: u16,
}
/// A half-open material run; gaps between runs remain air.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub min: i16,
    pub max: i16,
    pub state: BlockStateId,
    pub biome: u16,
    pub light: u16,
}
/// A column's material runs. Almost every far column has one to four, so
/// they live inline: the sampler builds millions of columns and a heap
/// allocation each dominated its memory and allocator time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spans(smallvec::SmallVec<[Span; 4]>);
impl Spans {
    pub const fn new() -> Self {
        Self(smallvec::SmallVec::new_const())
    }
    pub fn with_capacity(capacity: usize) -> Self {
        Self(smallvec::SmallVec::with_capacity(capacity))
    }
    pub fn push(&mut self, span: Span) {
        self.0.push(span);
    }
    pub fn reserve(&mut self, additional: usize) {
        self.0.reserve(additional);
    }
    pub fn as_slice(&self) -> &[Span] {
        &self.0
    }
}
impl std::ops::Deref for Spans {
    type Target = [Span];
    fn deref(&self) -> &[Span] {
        &self.0
    }
}
impl std::ops::DerefMut for Spans {
    fn deref_mut(&mut self) -> &mut [Span] {
        &mut self.0
    }
}
impl From<Vec<Span>> for Spans {
    fn from(spans: Vec<Span>) -> Self {
        Self(spans.into())
    }
}
impl FromIterator<Span> for Spans {
    fn from_iter<I: IntoIterator<Item = Span>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}
impl IntoIterator for Spans {
    type Item = Span;
    type IntoIter = smallvec::IntoIter<[Span; 4]>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
impl<'a> IntoIterator for &'a Spans {
    type Item = &'a Span;
    type IntoIter = std::slice::Iter<'a, Span>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}
impl<'a> IntoIterator for &'a mut Spans {
    type Item = &'a mut Span;
    type IntoIter = std::slice::IterMut<'a, Span>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
}
impl<const N: usize> PartialEq<[Span; N]> for Spans {
    fn eq(&self, other: &[Span; N]) -> bool {
        self.as_slice() == other
    }
}
impl PartialEq<Vec<Span>> for Spans {
    fn eq(&self, other: &Vec<Span>) -> bool {
        self.as_slice() == other.as_slice()
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Column {
    pub surface: Option<Surface>,
    pub spans: Spans,
}
impl Column {
    /// Reduce an entire horizontal footprint, rather than inflating its corner
    /// column (a single leaf used to become a whole distant tree-sized slab).
    pub fn reduced(columns: &[Column; 256], x: usize, z: usize, width: usize) -> Self {
        if width == 1 {
            return columns[z * 16 + x].clone();
        }
        let mut events = Vec::new();
        for dz in z..z + width {
            for dx in x..x + width {
                for span in &columns[dz * 16 + dx].spans {
                    let material = (span.state, span.biome, span.light);
                    events.push((span.min, material, 1));
                    events.push((span.max, material, -1));
                }
            }
        }
        events.sort_unstable_by_key(|event| event.0);
        let mut result = Self::default();
        let mut votes = Vec::<((BlockStateId, u16), i32)>::new();
        let mut sky = [0i32; 16];
        let mut block = [0i32; 16];
        let mut occupied = 0;
        let mut bottom = MIN_Y as i16;
        for (y, material, change) in events {
            if bottom < y {
                if occupied * 2 >= (width * width) as i32
                    && let Some(&((state, biome), _)) =
                        votes.iter().max_by_key(|&(material, count)| {
                            (*count, std::cmp::Reverse((material.0.0, material.1)))
                        })
                {
                    // Lighting must not split votes for the same material.
                    // Reduction can expose buried faces; retain the brightest
                    // actual light in this occupied footprint, per channel.
                    let light = (sky.iter().rposition(|&n| n > 0).unwrap_or(0) << 4)
                        | (block.iter().rposition(|&n| n > 0).unwrap_or(0) << 12);
                    result.push(Span {
                        min: bottom,
                        max: y,
                        state,
                        biome,
                        light: light as u16,
                    });
                }
                bottom = y;
            }
            occupied += change;
            sky[usize::from((material.2 >> 4) & 15)] += change;
            block[usize::from((material.2 >> 12) & 15)] += change;
            let material = (material.0, material.1);
            if let Some(vote) = votes.iter_mut().find(|vote| vote.0 == material) {
                vote.1 += change;
            } else {
                votes.push((material, change));
            }
        }
        result
    }
    pub fn from_surface(surface: Option<Surface>) -> Self {
        let mut column = Self {
            surface,
            spans: Spans::new(),
        };
        if let Some(s) = surface {
            if s.height > MIN_Y {
                column.spans.push(Span {
                    min: MIN_Y as i16,
                    max: s.height as i16,
                    state: s.state,
                    biome: s.biome,
                    light: 240,
                });
            }
            if let Some((height, state)) = s.water {
                column.spans.push(Span {
                    min: s.height as i16,
                    max: height as i16,
                    state,
                    biome: s.biome,
                    light: 240,
                });
            }
        }
        column
    }
    pub fn push(&mut self, span: Span) {
        if let Some(last) = self.spans.last_mut()
            && last.max == span.min
            && last.state == span.state
            && last.biome == span.biome
            && last.light == span.light
        {
            last.max = span.max;
        } else {
            self.spans.push(span);
        }
    }
    /// Open water and its floor lit as open sky, as the seed's far columns
    /// are: a chunk's own light fades a level per block of water, so the
    /// chunks a player passed drew a darker band through the seed's sea.
    /// The water is one span again.
    fn light_open_water(&mut self, kinds: &[u8]) {
        // Water; lava glows.
        let water = |span: &Span| {
            let kind = kinds.get(span.state.0 as usize).copied().unwrap_or(0);
            kind & FLUID != 0 && kind >> 4 == 0
        };
        if !self.spans.last().is_some_and(water) {
            return;
        }
        let floor = self
            .spans
            .iter()
            .rposition(|span| !water(span))
            .unwrap_or(0);
        for (i, mut span) in std::mem::take(&mut self.spans).into_iter().enumerate() {
            if i >= floor {
                span.light |= 0xf0;
            }
            self.push(span);
        }
    }
    /// `kinds`: `state_kinds`, per block state; `water`: `water_source(kinds)`,
    /// what blocks holding water (kelp, seagrass, waterlogged ones) are drawn as.
    fn from_chunk(
        chunk: &Chunk,
        kinds: &[u8],
        water: Option<BlockStateId>,
        x: i32,
        z: i32,
    ) -> Self {
        let kind = |state: BlockStateId| kinds.get(state.0 as usize).copied().unwrap_or(0);
        let face_light = |y: i32| {
            let mut sky = 0;
            let mut block = 0;
            for (dx, dy, dz) in [
                (0, 0, 0),
                (-1, 0, 0),
                (1, 0, 0),
                (0, -1, 0),
                (0, 1, 0),
                (0, 0, -1),
                (0, 0, 1),
            ] {
                if !(0..16).contains(&(x + dx)) || !(0..16).contains(&(z + dz)) {
                    continue;
                }
                let value = chunk.light(x + dx, y + dy, z + dz);
                sky = sky.max(value & 0xf0);
                block = block.max(value & 15);
            }
            sky | block
        };
        let mut column = Self::default();
        for (sy, section) in chunk.sections.iter().enumerate() {
            let min = chunk.min_y() + sy as i32 * 16;
            let uniform = section.blocks.bits() == 0
                && section.biomes.bits() == 0
                && matches!(section.light, crate::chunk::Light::Uniform(_));
            let uniform = uniform
                && face_light(min) == chunk.light(x, min, z)
                && face_light(min + 15) == chunk.light(x, min, z);
            let step = if uniform { 16 } else { 1 };
            for offset in (0..16).step_by(step) {
                let y = min + offset;
                let mut state = BlockStateId(chunk.get(x, y, z));
                let mut kind = kind(state);
                if kind & KEPT == 0 {
                    continue;
                }
                if kind & WATERY != 0
                    && let Some(water) = water
                {
                    state = water;
                    kind = kinds[water.0 as usize];
                }
                let light = face_light(y);
                column.push(Span {
                    min: y as i16,
                    max: (y + step as i32) as i16,
                    state,
                    biome: chunk.biome(x, y, z),
                    light: u16::from(light & 0xf0) | (u16::from((light & 15).max(kind >> 4)) << 12),
                });
            }
        }
        column.light_open_water(kinds);
        let mut water = None;
        for span in column.spans.iter().rev() {
            if kind(span.state) & FLUID != 0 {
                water.get_or_insert((i32::from(span.max), span.state));
            } else {
                column.surface = Some(Surface {
                    height: i32::from(span.max),
                    state: span.state,
                    water,
                    biome: span.biome,
                });
                break;
            }
        }
        if column.surface.is_none()
            && let Some((height, state)) = water
        {
            column.surface = Some(Surface {
                height: chunk.min_y(),
                state: BlockStateId(
                    kinds.iter().position(|&kind| kind & AIR != 0).unwrap_or(0) as u32
                ),
                water: Some((height, state)),
                biome: chunk.biome(x, height - 1, z),
            });
        }
        column
    }
}
/// A chunk's 256 columns (`Column::from_chunk`), on the far-terrain worker.
fn summarize(chunk: &Chunk, kinds: &[u8]) -> Arc<[Column; 256]> {
    let water = water_source(kinds);
    Arc::new(std::array::from_fn(|i| {
        Column::from_chunk(chunk, kinds, water, (i & 15) as i32, (i >> 4) as i32)
    }))
}
/// The water source state: the first plain kept fluid without light
/// (`minecraft:water` comes before lava, and its level 0 first).
fn water_source(kinds: &[u8]) -> Option<BlockStateId> {
    kinds
        .iter()
        .position(|&kind| kind == KEPT | FLUID)
        .map(|i| BlockStateId(i as u32))
}
/// `state_kinds` bits: kept in columns, a fluid, the air state, a block
/// holding water; the light emission is in the high nibble.
const KEPT: u8 = 1;
const FLUID: u8 = 2;
const AIR: u8 = 4;
/// A block holding water that draws as none (kelp, seagrass, bubble columns,
/// waterlogged blocks that aren't kept): far terrain draws water there, or
/// kelp reaching the surface leaves holes in the sea.
const WATERY: u8 = 8;
/// What `Column::from_chunk` needs of each block state, read from the facts
/// once: matching their names for every block made observing a chunk slow.
fn state_kinds(facts: &BlockFacts) -> Vec<u8> {
    (0..facts.states.len())
        .map(|i| {
            let state = BlockStateId(i as u32);
            let f = facts.get(state);
            let fluid = matches!(&*f.name, "minecraft:water" | "minecraft:lava");
            // Chorus is thin and non-solid but a visible part of the far End.
            let chorus = matches!(
                &*f.name,
                "minecraft:chorus_plant" | "minecraft:chorus_flower"
            );
            let kept = !facts.is_air(state)
                && (f.solid_render
                    || f.full_collision
                    || f.name.ends_with("_leaves")
                    || chorus
                    || fluid
                    || nearly_full(&f.name));
            let watery = !kept && !facts.is_air(state) && f.fluid == crate::blocks::Fluid::Water;
            (u8::from(kept || watery) * KEPT)
                | (u8::from(fluid || watery) * FLUID)
                | (u8::from(watery) * WATERY)
                | (u8::from(state == facts.air) * AIR)
                | (f.emission.min(15) << 4)
        })
        .collect()
}
/// Blocks that fill their cell but neither draw solid nor collide as a full
/// block: far terrain draws them as blocks, or patches of powder snow leave
/// holes in mountains (and paths and fields dips).
fn nearly_full(name: &str) -> bool {
    matches!(
        name,
        "minecraft:powder_snow"
            | "minecraft:dirt_path"
            | "minecraft:farmland"
            | "minecraft:honey_block"
    ) || name.ends_with("_stairs")
}
/// Clears the detail bits of a tile's cells whose chunk lies in `area`, but
/// for the chunks `kept` (seen in the world): `request_detail` asks for them
/// again.
fn forget_detail(
    key: TileKey,
    area: [i32; 4],
    mask: &mut [u64; 4],
    kept: impl Fn((i32, i32)) -> bool,
) {
    let [ox, _, oz] = key.origin();
    let stride = key.size() / 16;
    for index in 0..256 {
        let chunk = (
            (ox + (index as i32 % 16) * stride).div_euclid(16),
            (oz + (index as i32 / 16) * stride).div_euclid(16),
        );
        let fine = TileKey {
            x: chunk.0,
            z: chunk.1,
            level: 0,
        };
        if fine.overlaps(area) && !kept(chunk) {
            mask[index / 64] &= !(1 << (index % 64));
        }
    }
}
/// The tiles over `chunk` to sample again once their source keeps it
/// (`Lod::resample_over`): those whose sample is still being taken, maybe
/// from before, and those of cells wider than a chunk sampled empty there,
/// which a chunk's own detail can't fill (`apply_detail`). The others hold
/// it once `finish_detail` applies it.
fn resampled_over(
    ready: &FxHashMap<TileKey, Arc<Vec<Column>>>,
    pending: &FxHashSet<TileKey>,
    (cx, cz): (i32, i32),
) -> Vec<TileKey> {
    (1..=MAX_LEVEL)
        .filter_map(|level| {
            let size = 16 << level;
            let key = TileKey {
                x: (cx * 16).div_euclid(size),
                z: (cz * 16).div_euclid(size),
                level,
            };
            let resample = match ready.get(&key) {
                None => pending.contains(&key),
                Some(samples) => {
                    let stride = size / CELLS as i32;
                    let [ox, _, oz] = key.origin();
                    let cell = (cz * 16 - oz) / stride * CELLS as i32 + (cx * 16 - ox) / stride;
                    stride > 16
                        && (pending.contains(&key)
                            || samples
                                .get(cell as usize)
                                .is_some_and(|c| *c == Column::default()))
                }
            };
            resample.then_some(key)
        })
        .collect()
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub x: i32,
    pub z: i32,
    pub level: u8,
}
impl TileKey {
    pub fn size(self) -> i32 {
        16 << self.level
    }
    pub fn origin(self) -> [i32; 3] {
        [self.x * self.size(), 0, self.z * self.size()]
    }
    pub(crate) fn parent(self) -> Self {
        Self {
            x: self.x.div_euclid(2),
            z: self.z.div_euclid(2),
            level: self.level + 1,
        }
    }
    fn children(self) -> [Self; 4] {
        std::array::from_fn(|i| Self {
            x: self.x * 2 + (i & 1) as i32,
            z: self.z * 2 + (i >> 1) as i32,
            level: self.level - 1,
        })
    }
    /// The squared horizontal distance from `camera` to the tile's nearest point.
    fn reach(self, camera: [i32; 2]) -> i64 {
        let [x, _, z] = self.origin();
        let gap = |c: i32, lo: i32| i64::from((lo - c).max(c - lo - self.size()).max(0));
        gap(camera[0], x).pow(2) + gap(camera[1], z).pow(2)
    }
    /// Whether the tile covers any of the blocks `[x0, z0, x1, z1)`.
    fn overlaps(self, [x0, z0, x1, z1]: [i32; 4]) -> bool {
        let [ox, _, oz] = self.origin();
        let size = self.size();
        ox < x1 && oz < z1 && ox + size > x0 && oz + size > z0
    }
    /// Whether new data for `area` drawn in cells `widths` wide changes the
    /// tile's samples: its cells are that wide, and it covers `area` or the
    /// blocks around it a cell's depth reads (`bluemap::SKIRT`: a block
    /// around a chunk, the next cells' middles past one).
    fn refreshed_by(self, area: [i32; 4], widths: (i32, i32)) -> bool {
        let cell = self.size() / CELLS as i32;
        (widths.0..=widths.1).contains(&cell) && self.overlaps(Self::around(area, cell))
    }
    fn around([x0, z0, x1, z1]: [i32; 4], cell: i32) -> [i32; 4] {
        let m = if cell <= 16 { 1 } else { cell };
        [x0 - m, z0 - m, x1 + m, z1 + m]
    }
    fn distance(self, camera: [i32; 2]) -> i32 {
        let [x, _, z] = self.origin();
        (x - camera[0])
            .max(camera[0] - x - self.size())
            .max(z - camera[1])
            .max(camera[1] - z - self.size())
            .max(0)
    }
}
#[derive(Default)]
struct Plan {
    roots: Vec<TileKey>,
    nodes: FxHashSet<TileKey>,
    split: FxHashSet<TileKey>,
    /// `nodes` by level: children (level - 1) come before their parents when
    /// selection is rebuilt, without sorting the whole set.
    by_level: [Vec<TileKey>; MAX_LEVEL as usize + 1],
    elevation: i32,
    camera: [i32; 2],
    /// The squared distance within which no coarser tile stands in for
    /// missing detail (`stands_in`): the seam with the near chunks waits
    /// for its own tiles instead of showing broad boxes beside them.
    near: i64,
    /// The squared distance within which planned tiles load before the
    /// coarse backdrop (`wanted_tiles`).
    first: i64,
}
/// Chunks past the render distance where only planned tiles are drawn (two
/// past the last near chunk, `streaming::in_view_distance`), and where they
/// load first.
const SEAM_CHUNKS: i32 = 3;
const FIRST_CHUNKS: i32 = 4;
impl Plan {
    #[cfg(test)]
    fn new(camera: [i32; 2], detail: i32, distance: u32) -> Self {
        Self::with_previous(camera, detail, distance, &Self::default(), 0, false)
    }
    /// `reduced`: Reduced far detail (`graphics::FarDetail`), each level from
    /// half the distance.
    fn with_previous(
        camera: [i32; 2],
        detail: i32,
        distance: u32,
        previous: &Self,
        elevation: i32,
        reduced: bool,
    ) -> Self {
        let range = distance.min(MAX_CHUNKS) as i32 * 16;
        let mut level = 0;
        while (16 << level) < range {
            level += 1;
        }
        let size = 16 << level;
        let mut result = Self {
            elevation,
            camera,
            near: i64::from((detail + SEAM_CHUNKS) * 16).pow(2),
            first: i64::from((detail + FIRST_CHUNKS) * 16).pow(2),
            ..Self::default()
        };
        for z in (camera[1] - range).div_euclid(size)..=(camera[1] + range).div_euclid(size) {
            for x in (camera[0] - range).div_euclid(size)..=(camera[0] + range).div_euclid(size) {
                let key = TileKey { x, z, level };
                result.roots.push(key);
                result.visit(key, camera, detail, range, previous, reduced);
            }
        }
        result
    }
    fn visit(
        &mut self,
        key: TileKey,
        camera: [i32; 2],
        detail: i32,
        range: i32,
        previous: &Self,
        reduced: bool,
    ) {
        // The range is a circle, as the renderer's: a tile leaves it, and
        // fades out (gfx/lod.rs), as the player moves.
        if key.reach(camera) > i64::from(range) * i64::from(range) {
            return;
        }
        if self.nodes.insert(key) {
            self.by_level[key.level as usize].push(key);
        }
        // Keep distant cells small on screen; finest tiles retain individual
        // blocks. Reduced keeps them twice as large (a quarter of the cells).
        let split = if previous.split.contains(&key) { 10 } else { 8 };
        // Beyond the old 1024-chunk limit the cells are as large as `reduced`'s:
        // the whole world is 22 levels, not all of them worth full cells.
        let threshold = key.size()
            * if key.distance(camera) >= WORLD_START {
                WORLD_SPLIT
            } else if reduced {
                split / 2
            } else {
                split
            };
        if key.level > 0
            && (key.distance(camera) < detail * 16 + 16
                || key.distance(camera).max(self.elevation) < threshold)
        {
            self.split.insert(key);
            for child in key.children() {
                self.visit(child, camera, detail, range, previous, reduced);
            }
        }
    }
    #[cfg(test)]
    fn select(
        &self,
        key: TileKey,
        ready: &FxHashMap<TileKey, impl Sized>,
        out: &mut Vec<TileKey>,
    ) -> bool {
        if !self.nodes.contains(&key) {
            return true;
        }
        if self.split.contains(&key) {
            // Visit every child: a missing sibling must not hide ready detail.
            let complete = key
                .children()
                .into_iter()
                .map(|child| self.select(child, ready, out))
                .fold(true, |complete, child| complete & child);
            if complete {
                return true;
            }
            if ready.contains_key(&key) && self.stands_in(key) {
                // Its mesh is clipped around every selected finer tile.
                out.push(key);
                return true;
            }
            false
        } else if ready.contains_key(&key) {
            out.push(key);
            true
        } else {
            false
        }
    }
    /// Whether a split tile may draw for its missing children: only away
    /// from the near chunks, where its cells are small on screen.
    fn stands_in(&self, key: TileKey) -> bool {
        key.reach(self.camera) >= self.near
    }
    /// The coarse tile sampled to cover `leaf` while it loads: two levels
    /// up (16 times fewer tiles, each as cheap as a leaf to sample), one
    /// where that would reach the seam, none beside it.
    fn backdrop(&self, leaf: TileKey) -> Option<TileKey> {
        let mut backdrop = None;
        let mut key = leaf;
        for _ in 0..2 {
            key = key.parent();
            if !self.nodes.contains(&key) || !self.stands_in(key) {
                break;
            }
            backdrop = Some(key);
        }
        backdrop
    }
    /// Whether chunk column (x, z) is drawn by the tile planned there (ready;
    /// at level 0, also `filled`) rather than by a coarser ancestor standing
    /// in for it, or is past the far terrain.
    fn replaces(
        &self,
        (x, z): (i32, i32),
        ready: &FxHashMap<TileKey, impl Sized>,
        filled: impl Fn(TileKey) -> bool,
    ) -> bool {
        let Some(key) = (0..=MAX_LEVEL)
            .map(|level| TileKey {
                x: (x * 16).div_euclid(16 << level),
                z: (z * 16).div_euclid(16 << level),
                level,
            })
            .find(|key| self.nodes.contains(key))
        else {
            return true;
        };
        // A split node's child there is out of range: nothing is drawn.
        self.split.contains(&key) || (ready.contains_key(&key) && (key.level > 0 || filled(key)))
    }
}
/// The tiles to sample, `Wanted`'s order best first. First the planned
/// tiles at the seam with the near chunks, nearest first: coarse tiles
/// there cost a far tile's work for broad boxes that fine tiles replace
/// moments later. Then a coarse backdrop (`Plan::backdrop`) out to the
/// horizon, then every other planned tile nearest first, each replacing its
/// part of the backdrop: the horizon fills in early and sharpens from the
/// player outward, whichever way they look.
/// A tile `ready` and `stale` (`Lod::refresh`) is wanted again, its old
/// samples drawn meanwhile: it needs no backdrop.
fn wanted_tiles(
    plan: &Plan,
    ready: &FxHashMap<TileKey, impl Sized>,
    stale: &FxHashSet<TileKey>,
    camera: [i32; 2],
) -> impl Iterator<Item = Wanted> {
    let sampled = |key: &TileKey| ready.contains_key(key) && !stale.contains(key);
    let wanted = |class, key: TileKey| Wanted {
        class,
        distance: i64::from(key.distance(camera)),
        x: key.x,
        z: key.z,
        level: key.level,
    };
    let mut backdrop = FxHashSet::default();
    let mut tiles: Vec<_> = plan
        .nodes
        .iter()
        .filter(|key| !plan.split.contains(key) && !sampled(key))
        .map(|&key| {
            if !ready.contains_key(&key)
                && let Some(coarse) = plan.backdrop(key)
                && !sampled(&coarse)
            {
                backdrop.insert(coarse);
            }
            let first = key.reach(camera) < plan.first;
            wanted(if first { SEAM } else { REST }, key)
        })
        .collect();
    tiles.extend(backdrop.into_iter().map(|key| wanted(BACKDROP, key)));
    tiles.into_iter()
}
/// `Wanted::class`es, best first.
const SEAM: u8 = 0;
const BACKDROP: u8 = 1;
const REST: u8 = 2;
/// Visited chunks' summaries (`Lod::saved`) and their reductions.
type Saved = FxHashMap<(i32, i32), Arc<[Column; 256]>>;
type Reduced = FxHashMap<(i32, i32), Arc<ReducedColumns>>;
/// What a mesh job reads, as it was when the job began: the worker gathers
/// every tile's inputs from it (gathering on the render thread stalled the
/// frame, tens of milliseconds for a large job on a 4-thread machine).
struct Snapshot {
    samples: FxHashMap<TileKey, Arc<Vec<Column>>>,
    saved: Saved,
    reduced: Reduced,
}
/// One column of a mesh input, by reference into the snapshot. Mesh jobs used
/// to clone all 256 columns of a tile (each with its own span allocation);
/// now an input is a base tile plus a handful of overrides.
enum ColumnSource {
    Tile(Arc<Vec<Column>>, u16),
    Saved(Arc<[Column; 256]>, u16),
    Reduced(Arc<ReducedColumns>, u8, u16, u16),
    Empty,
}
static EMPTY_COLUMN: Column = Column {
    surface: None,
    spans: Spans::new(),
};
impl ColumnSource {
    fn get(&self) -> &Column {
        match self {
            Self::Tile(columns, index) => &columns[*index as usize],
            Self::Saved(columns, index) => &columns[*index as usize],
            Self::Reduced(columns, width, x, z) => {
                columns.get(*x as usize, *z as usize, *width as usize)
            }
            Self::Empty => &EMPTY_COLUMN,
        }
    }
}
struct EdgeSource {
    start: i32,
    end: i32,
    column: ColumnSource,
}
struct MeshInput {
    /// The tile's samples, with `overrides` replacing some of them.
    base: Arc<Vec<Column>>,
    overrides: [Option<ColumnSource>; CELLS * CELLS],
    neighbors: [Vec<EdgeSource>; 4],
    finer: Vec<TileKey>,
}
struct ReducedColumns {
    columns: Arc<[Column; 256]>,
    levels: [Vec<Column>; 4],
}
impl MeshInput {
    /// The column of every cell: the tile's own samples unless the snapshot
    /// has actual visited detail there.
    fn columns(&self) -> Vec<&Column> {
        let mut columns: Vec<&Column> = self.base.iter().collect();
        for (index, source) in self.overrides.iter().enumerate() {
            if let Some(source) = source {
                columns[index] = source.get();
            }
        }
        columns
    }
    fn edges(&self) -> [Vec<mesh::EdgeColumn<'_>>; 4] {
        std::array::from_fn(|side| {
            self.neighbors[side]
                .iter()
                .map(|edge| mesh::EdgeColumn {
                    start: edge.start,
                    end: edge.end,
                    column: edge.column.get(),
                })
                .collect()
        })
    }
}
impl ReducedColumns {
    fn new(columns: Arc<[Column; 256]>) -> Self {
        let levels = std::array::from_fn(|level| {
            let width = 2 << level;
            (0..16 / width)
                .flat_map(|z| {
                    let columns = &columns;
                    (0..16 / width)
                        .map(move |x| Column::reduced(columns, x * width, z * width, width))
                })
                .collect()
        });
        Self { columns, levels }
    }
    fn get(&self, x: usize, z: usize, width: usize) -> &Column {
        if width == 1 {
            &self.columns[z * 16 + x]
        } else {
            &self.levels[width.trailing_zeros() as usize - 1]
                [(z / width) * (16 / width) + x / width]
        }
    }
}
enum Job {
    Sample(TileKey),
    /// Summarize the chunk `Shared::observed` holds for this column.
    Observe((i32, i32)),
    RequestDetail {
        chunks: Vec<(i32, i32)>,
        center: [i32; 2],
        /// `Lod::refreshes` when asked: what was read before a later
        /// `refresh` may be older than what it announced.
        refreshes: u64,
    },
}
enum Done {
    /// A chunk's detail, and the `refreshes` of the request it answers.
    Detail((i32, i32), ReducedColumns, u64),
    /// A visited chunk's summary (`Job::Observe`).
    Observed((i32, i32), ReducedColumns),
    DetailRequested,
    Sample {
        key: TileKey,
        samples: Vec<Column>,
        /// Some of it is still on its way (`LodSampler::waited`).
        waited: bool,
    },
    Mesh {
        key: TileKey,
        version: u64,
        mesh: SectionMesh,
        canceled: Arc<AtomicBool>,
    },
    MeshCanceled {
        key: TileKey,
        version: u64,
    },
}
/// One tile's mesh job. Every job of a batch shares one immutable snapshot,
/// so tiles are meshed in parallel without cloning columns or maps.
struct MeshJob {
    snapshot: Arc<Snapshot>,
    key: TileKey,
    version: u64,
    finer: Vec<TileKey>,
    covered: Arc<FxHashSet<(i32, i32)>>,
    bounds: Option<[i32; 4]>,
    canceled: Arc<AtomicBool>,
}
struct MeshQueued {
    /// Lower is more urgent: near tiles first.
    priority: i64,
    order: u64,
    job: MeshJob,
}
impl PartialEq for MeshQueued {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.order == other.order
    }
}
impl Eq for MeshQueued {}
impl PartialOrd for MeshQueued {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for MeshQueued {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // `BinaryHeap` is a max-heap: lower priority values pop first.
        other
            .priority
            .cmp(&self.priority)
            .then(other.order.cmp(&self.order))
    }
}
/// A tile the sampler should produce next: by `class`
/// (`wanted_tiles`), then nearest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Wanted {
    class: u8,
    distance: i64,
    x: i32,
    z: i32,
    level: u8,
}
struct Shared {
    jobs: Mutex<VecDeque<Job>>,
    /// Tile meshes, nearest and most urgent first.
    mesh_jobs: Mutex<std::collections::BinaryHeap<MeshQueued>>,
    /// The latest chunk to summarize per column, queued as `Job::Observe`
    /// once: edits made before the worker gets to it are summarized once.
    observed: Mutex<FxHashMap<(i32, i32), Arc<Chunk>>>,
    /// `state_kinds`, from the first observed chunk's facts.
    kinds: std::sync::OnceLock<Vec<u8>>,
    wake: Condvar,
    mesh_wake: Condvar,
    stop: AtomicBool,
    /// Reduced far detail: samplers approximate wide cells.
    approximate: AtomicBool,
    /// On small machines, hold queued far meshing while near terrain loads.
    mesh_held: AtomicBool,
}
/// A background worker, sampling and meshing threads, and a bounded queue of
/// sample jobs (`sample_jobs`). Only the current view's quadtree and a
/// bounded set of visited summaries are retained.
pub struct Lod {
    shared: Arc<Shared>,
    thread: Option<thread::JoinHandle<()>>,
    mesh_threads: Vec<thread::JoinHandle<()>>,
    results: mpsc::Receiver<Done>,
    #[cfg(test)]
    context: Option<Arc<WorkerContext>>,
    /// A finished mesh job may have changed the selection of its ancestors.
    reselect: bool,
    area: Option<([i32; 2], i32, u32, i32, bool)>,
    plan: Plan,
    /// Reduced far detail (`set_reduced_detail`).
    reduced_detail: bool,
    pending: FxHashSet<TileKey>,
    detail_masks: FxHashMap<TileKey, [u64; 4]>,
    detail_request_pending: bool,
    /// `refresh`es so far (`Job::RequestDetail`), and the areas of the last
    /// ones, each with its number.
    refreshes: u64,
    refreshed: VecDeque<(u64, [i32; 4])>,
    detail_request_at: std::time::Instant,
    /// Four threads or fewer: far work waits for the near terrain (`hold`).
    small: bool,
    /// Sample jobs that may be queued or running at once.
    sample_jobs: usize,
    /// The near terrain is loading (`set_near_loading`).
    near_loading: bool,
    /// When `request_samples` last started a job while held.
    held_sample_at: std::time::Instant,
    /// Tiles whose sampling failed, and when they may be asked again.
    retry: FxHashMap<TileKey, std::time::Instant>,
    /// Each tile's samples, shared with mesh jobs in flight.
    ready: FxHashMap<TileKey, Arc<Vec<Column>>>,
    /// Fully covered subtrees, by tile.
    complete: FxHashMap<TileKey, bool>,
    /// Sampled tiles whose source has more since (`refresh`): sampled again,
    /// their old samples drawn meanwhile.
    stale: FxHashSet<TileKey>,
    /// Stale tiles `refresh`ed again while sampled: that sample may be older.
    restale: FxHashSet<TileKey>,
    active: FxHashSet<TileKey>,
    /// Tiles needing a mesh, with the revision their job must carry.
    dirty: FxHashMap<TileKey, u64>,
    /// The newest revision of each active tile's mesh.
    mesh_version: FxHashMap<TileKey, u64>,
    /// The newest revision accepted by the renderer for each active tile.
    /// Older in-flight results can still fill a hole while a newer job waits.
    shown_version: FxHashMap<TileKey, u64>,
    /// Cancellation tokens for queued or running jobs of active tiles.
    mesh_cancel: FxHashMap<TileKey, Arc<AtomicBool>>,
    next_version: u64,
    /// Mesh jobs queued or running; the cutout waits for all of them.
    outstanding_meshes: usize,
    /// Workers in the mesh pool (dispatch coalesces above four each).
    mesh_workers: usize,
    /// Tiles whose active replacement mesh has been returned and applied.
    meshed: FxHashSet<TileKey>,
    /// Active tiles whose current revision has no mesh yet.
    unmeshed: FxHashSet<TileKey>,
    /// Tiles that left the selection. The renderer may only fade them out
    /// once every active tile overlapping them has its mesh, or a hole opens.
    removed_pending: FxHashSet<TileKey>,
    /// Released removals, returned to the renderer this update.
    removed: Vec<TileKey>,
    /// The columns left out: `full` and `retiring`.
    covered: FxHashSet<(i32, i32)>,
    covered_bounds: Option<[i32; 4]>,
    /// The full-detail columns `update` was last given.
    full: FxHashSet<(i32, i32)>,
    /// Columns that left the full-detail area, still left out (their
    /// full-detail meshes still drawn) until the tile planned there is ready
    /// (`Plan::replaces`). Tiles inside that area are never sampled: when the
    /// render distance shrinks, their coarsest ancestors would otherwise
    /// stand in for it for minutes.
    retiring: FxHashSet<(i32, i32)>,
    /// The covered columns the active meshes leave out, when a mesh job
    /// has finished since the last `take_cutout`.
    cutout: Option<FxHashSet<(i32, i32)>>,
    /// Covered columns changed while meshes were in flight: the renderer must
    /// not drop full-detail sections before every replacement is uploaded.
    cutout_pending: Option<FxHashSet<(i32, i32)>>,
    /// Active tiles whose mesh must be accepted before the cutout publishes.
    cutout_waiting: FxHashSet<TileKey>,
    /// Unsample final tiles, nearest first. Rebuilt when the view changes,
    /// not scanned every frame.
    wanted: std::collections::BinaryHeap<std::cmp::Reverse<Wanted>>,
    wanted_at: Option<[i32; 2]>,
    wanted_stale: bool,
    saved: Saved,
    reduced: Reduced,
    saved_order: VecDeque<(i32, i32)>,
    /// `LodSource::keeps_observed`: tiles sampled without an observed chunk
    /// are sampled again (`resample_over`).
    keeps_observed: bool,
}
impl Lod {
    pub fn new(source: Arc<dyn LodSource>, context: Arc<WorkerContext>) -> Self {
        Self::assemble(Some(source), Some(context), true)
    }
    /// Scheduler-only instance for tests: no worker threads, so a test can
    /// pop the jobs it queued and finish samples itself.
    #[cfg(test)]
    fn scheduler_for_test() -> Self {
        Self::assemble(None, None, false)
    }
    fn assemble(
        source: Option<Arc<dyn LodSource>>,
        context: Option<Arc<WorkerContext>>,
        workers: bool,
    ) -> Self {
        let keeps_observed = source
            .as_ref()
            .is_some_and(|source| source.keeps_observed());
        let cores = thread::available_parallelism().map_or(1, |n| n.get());
        let shared = Arc::new(Shared {
            jobs: Mutex::new(VecDeque::new()),
            mesh_jobs: Mutex::new(std::collections::BinaryHeap::new()),
            observed: Mutex::new(FxHashMap::default()),
            kinds: std::sync::OnceLock::new(),
            wake: Condvar::new(),
            mesh_wake: Condvar::new(),
            stop: AtomicBool::new(false),
            approximate: AtomicBool::new(false),
            mesh_held: AtomicBool::new(false),
        });
        let worker = shared.clone();
        let (tx, results) = mpsc::channel();
        #[cfg(test)]
        let lod_context = context.clone();
        // Meshing runs on its own pool: one worker serialized every dirty
        // tile of a batch, however many cores the machine had.
        let mut mesh_threads = Vec::new();
        let mut handle = None;
        if workers {
            let source = source.expect("workers need a terrain source");
            for _ in 0..meshing_threads(cores) {
                let shared = shared.clone();
                let tx = tx.clone();
                let context = context.clone().expect("mesh workers need a context");
                mesh_threads.push(
                    thread::Builder::new()
                        .name("Far meshing".into())
                        .spawn(move || {
                            horizon_threads::behind_server();
                            let mut mesher = mesh::Mesher::default();
                            loop {
                                let job = {
                                    let mut jobs = shared.mesh_jobs.lock().unwrap();
                                    loop {
                                        if shared.stop.load(Ordering::Relaxed) {
                                            return;
                                        }
                                        if shared.mesh_held.load(Ordering::Relaxed) {
                                            jobs = shared.mesh_wake.wait(jobs).unwrap();
                                            continue;
                                        }
                                        if let Some(job) = jobs.pop() {
                                            break job.job;
                                        }
                                        jobs = shared.mesh_wake.wait(jobs).unwrap();
                                    }
                                };
                                if job.canceled.load(Ordering::Acquire) {
                                    if tx
                                        .send(Done::MeshCanceled {
                                            key: job.key,
                                            version: job.version,
                                        })
                                        .is_err()
                                    {
                                        return;
                                    }
                                    continue;
                                }
                                let keep_finer = !job.finer.is_empty();
                                let input = job.snapshot.mesh_input(job.key, job.finer);
                                let columns = input.columns();
                                let edges = input.edges();
                                let mut mesh = mesher.mesh(
                                    job.key,
                                    &columns,
                                    &job.covered,
                                    job.bounds,
                                    &context,
                                    &edges,
                                    &input.finer,
                                );
                                mesh.sort_solid();
                                let [ox, _, oz] = job.key.origin();
                                let size = job.key.size();
                                mesh.keep = keep_finer
                                    || job.covered.iter().any(|&(cx, cz)| {
                                        let x = cx * 16;
                                        let z = cz * 16;
                                        x < ox + size && x + 16 > ox && z < oz + size && z + 16 > oz
                                    });
                                if job.canceled.load(Ordering::Acquire) {
                                    if tx
                                        .send(Done::MeshCanceled {
                                            key: job.key,
                                            version: job.version,
                                        })
                                        .is_err()
                                    {
                                        return;
                                    }
                                    continue;
                                }
                                if tx
                                    .send(Done::Mesh {
                                        key: job.key,
                                        version: job.version,
                                        mesh,
                                        canceled: job.canceled,
                                    })
                                    .is_err()
                                {
                                    return;
                                }
                            }
                        })
                        .expect("far meshing worker"),
                );
            }
            handle = Some(
                thread::Builder::new()
                    .name("Far terrain".into())
                    .spawn(move || {
                        // Ticks and frames come first (horizon_threads).
                        horizon_threads::behind_server();
                        let mut sampler = source.sampler();
                        let count = sampling_threads(cores);
                        let mut sampling = Vec::new();
                        for _ in 0..count {
                            let Some(mut sample) = sampler.fork() else {
                                break;
                            };
                            let shared = worker.clone();
                            let tx = tx.clone();
                            sampling.push(
                                thread::Builder::new()
                                    .name("Far sampling".into())
                                    .spawn(move || {
                                        horizon_threads::behind_server();
                                        loop {
                                            let key = {
                                                let mut jobs = shared.jobs.lock().unwrap();
                                                loop {
                                                    if shared.stop.load(Ordering::Relaxed) {
                                                        return;
                                                    }
                                                    if let Some(index) =
                                                        jobs.iter().position(|job| {
                                                            matches!(job, Job::Sample(_))
                                                        })
                                                    {
                                                        let Some(Job::Sample(key)) =
                                                            jobs.remove(index)
                                                        else {
                                                            unreachable!()
                                                        };
                                                        break key;
                                                    }
                                                    jobs = shared.wake.wait(jobs).unwrap();
                                                }
                                            };
                                            let [x, _, z] = key.origin();
                                            sample.set_approximate(
                                                shared.approximate.load(Ordering::Relaxed),
                                            );
                                            let samples =
                                                sample.grid(x, z, key.size() / CELLS as i32, CELLS);
                                            if shared.stop.load(Ordering::Relaxed) {
                                                return;
                                            }
                                            let waited = sample.waited();
                                            if tx
                                                .send(Done::Sample {
                                                    key,
                                                    samples,
                                                    waited,
                                                })
                                                .is_err()
                                            {
                                                return;
                                            }
                                        }
                                    })
                                    .expect("far sampling worker"),
                            );
                        }
                        let mut asked = 0;
                        'work: loop {
                            for (key, columns) in sampler.drain_details() {
                                if tx
                                    .send(Done::Detail(key, ReducedColumns::new(columns), asked))
                                    .is_err()
                                {
                                    break 'work;
                                }
                            }
                            let job = {
                                let mut jobs = worker.jobs.lock().unwrap();
                                if (jobs.is_empty()
                                    || (!sampling.is_empty()
                                        && jobs.iter().all(|job| matches!(job, Job::Sample(_)))))
                                    && !worker.stop.load(Ordering::Relaxed)
                                {
                                    jobs = worker
                                        .wake
                                        .wait_timeout(jobs, std::time::Duration::from_millis(100))
                                        .unwrap()
                                        .0;
                                }
                                if worker.stop.load(Ordering::Relaxed) {
                                    break;
                                }
                                // Summaries and detail requests first: they
                                // would otherwise wait behind a whole queue of
                                // samples where no sampling threads take them.
                                let other =
                                    jobs.iter().position(|job| !matches!(job, Job::Sample(_)));
                                if let Some(index) = other {
                                    jobs.remove(index)
                                } else if sampling.is_empty() {
                                    jobs.pop_front()
                                } else {
                                    None
                                }
                            };
                            let Some(job) = job else { continue };
                            let done = match job {
                                Job::Observe(key) => {
                                    let chunk = worker.observed.lock().unwrap().remove(&key);
                                    let (Some(chunk), Some(kinds)) = (chunk, worker.kinds.get())
                                    else {
                                        continue;
                                    };
                                    let columns = summarize(&chunk, kinds);
                                    sampler.observed(key, &columns);
                                    Done::Observed(key, ReducedColumns::new(columns))
                                }
                                Job::RequestDetail {
                                    chunks,
                                    center,
                                    refreshes,
                                } => {
                                    asked = refreshes;
                                    sampler.request_detail(&chunks, center);
                                    Done::DetailRequested
                                }
                                Job::Sample(key) => {
                                    let [x, _, z] = key.origin();
                                    sampler.set_approximate(
                                        worker.approximate.load(Ordering::Relaxed),
                                    );
                                    let samples =
                                        sampler.grid(x, z, key.size() / CELLS as i32, CELLS);
                                    let waited = sampler.waited();
                                    Done::Sample {
                                        key,
                                        samples,
                                        waited,
                                    }
                                }
                            };
                            if tx.send(done).is_err() {
                                break;
                            }
                        }
                        let jobs = worker.jobs.lock();
                        worker.stop.store(true, Ordering::Relaxed);
                        drop(jobs);
                        worker.wake.notify_all();
                        for handle in sampling {
                            let _ = handle.join();
                        }
                    })
                    .expect("far terrain worker"),
            );
        }

        Self {
            shared,
            thread: handle,
            mesh_threads,
            results,
            #[cfg(test)]
            context: lod_context,
            reselect: true,
            area: None,
            reduced_detail: false,
            plan: Plan::default(),
            pending: FxHashSet::default(),
            detail_masks: FxHashMap::default(),
            detail_request_pending: false,
            refreshes: 0,
            refreshed: VecDeque::new(),
            detail_request_at: std::time::Instant::now(),
            small: cores <= 4,
            sample_jobs: sampling_threads(cores) * QUEUED_PER_SAMPLER,
            near_loading: false,
            held_sample_at: std::time::Instant::now(),
            retry: FxHashMap::default(),
            ready: FxHashMap::default(),
            complete: FxHashMap::default(),
            active: FxHashSet::default(),
            dirty: FxHashMap::default(),
            mesh_version: FxHashMap::default(),
            shown_version: FxHashMap::default(),
            mesh_cancel: FxHashMap::default(),
            next_version: 1,
            outstanding_meshes: 0,
            mesh_workers: meshing_threads(cores),
            meshed: FxHashSet::default(),
            unmeshed: FxHashSet::default(),
            removed_pending: FxHashSet::default(),
            stale: FxHashSet::default(),
            restale: FxHashSet::default(),
            removed: Vec::new(),
            covered: FxHashSet::default(),
            covered_bounds: None,
            full: FxHashSet::default(),
            retiring: FxHashSet::default(),
            cutout: None,
            cutout_pending: None,
            cutout_waiting: FxHashSet::default(),
            wanted: std::collections::BinaryHeap::new(),
            wanted_at: None,
            wanted_stale: true,
            saved: FxHashMap::default(),
            reduced: FxHashMap::default(),
            saved_order: VecDeque::new(),
            keeps_observed,
        }
    }
    /// The solid ground at block column (x, z): its top face's height and
    /// its block, from the finest tile sampled there. For aiming past the
    /// resident chunks (`game.raycast`); water is passed, as the view's ray
    /// passes it.
    pub fn ground(&self, x: i32, z: i32) -> Option<(i32, BlockStateId)> {
        (0..=MAX_LEVEL).find_map(|level| {
            let size = 16 << level;
            let key = TileKey {
                x: x.div_euclid(size),
                z: z.div_euclid(size),
                level,
            };
            let columns = self.ready.get(&key).filter(|c| c.len() == CELLS * CELLS)?;
            let cell = size / CELLS as i32;
            let (cx, cz) = ((x - key.x * size) / cell, (z - key.z * size) / cell);
            let surface = columns[cz as usize * CELLS + cx as usize].surface?;
            Some((surface.height, surface.state))
        })
    }
    pub fn stats(&self) -> (usize, usize) {
        (self.active.len(), self.ready.len())
    }
    pub fn progress(&self) -> (usize, usize) {
        let mut desired = 0;
        let mut sampled = 0;
        for key in &self.plan.nodes {
            if !self.plan.split.contains(key) {
                desired += 1;
                sampled += usize::from(self.ready.contains_key(key));
            }
        }
        (desired, sampled)
    }
    /// Drawn columns that take their chunk's generated detail (trees,
    /// structures; `request_detail`), and how many of them have it.
    /// `refined_samples` counts the detail of every drawn tile instead.
    pub fn detail_progress(&self) -> (usize, usize) {
        self.active
            .iter()
            .filter(|key| self.wants_detail(key))
            .fold((0, 0), |(wanted, refined), key| {
                let mask = self.detail_masks.get(key).map_or(0, detail_count);
                (wanted + CELLS * CELLS, refined + mask)
            })
    }
    pub fn refined_samples(&self) -> usize {
        self.active
            .iter()
            .filter_map(|key| self.detail_masks.get(key))
            .map(detail_count)
            .sum()
    }
    /// An active tile whose columns take generated chunks' detail.
    fn wants_detail(&self, key: &TileKey) -> bool {
        key.level <= DETAIL_LEVEL
            && !self.plan.split.contains(key)
            && !(key.level == 0 && self.covered.contains(&(key.x, key.z)))
    }
    pub fn removed(&mut self) -> Vec<TileKey> {
        std::mem::take(&mut self.removed)
    }
    /// The full-detail columns the active meshes leave out, once per finished
    /// mesh job. Until a column leaves this set, nothing distant stands in for
    /// it: its full-detail terrain must still be drawn, in or past the render
    /// distance, or a hole opens there.
    pub fn take_cutout(&mut self) -> Option<FxHashSet<(i32, i32)>> {
        self.cutout.take()
    }
    /// Marks an active tile for remeshing. Versioned so a result from an
    /// older batch can never overwrite a newer one.
    fn cancel_mesh(&mut self, key: TileKey) {
        if let Some(token) = self.mesh_cancel.remove(&key) {
            token.store(true, Ordering::Release);
            self.shared.mesh_wake.notify_all();
        }
    }
    fn mark_mesh(&mut self, key: TileKey) {
        if !self.active.contains(&key) {
            return;
        }
        let version = self.next_version;
        self.next_version += 1;
        self.dirty.insert(key, version);
        self.mesh_version.insert(key, version);
        self.meshed.remove(&key);
        self.unmeshed.insert(key);
    }
    /// The tile's selection changed: its own mesh, every selected fallback
    /// ancestor that clips around its footprint, and its face neighbours.
    fn mark_selection_changed(&mut self, key: TileKey) {
        self.mark_mesh(key);
        // A fallback ancestor's mesh is clipped around this tile.
        let mut node = key;
        while node.level < MAX_LEVEL {
            node = node.parent();
            if self.active.contains(&node) {
                self.mark_mesh(node);
            }
        }
        self.mark_side_neighbors_of(key);
        self.mark_active_descendants(key);
    }
    /// The tile's columns changed but its footprint did not. A fallback
    /// ancestor clips around footprints, not column values, so only the tile
    /// and the neighbours whose edge columns read it need remeshing.
    fn mark_data_changed(&mut self, key: TileKey) {
        self.mark_mesh(key);
        self.mark_side_neighbors_of(key);
        if self.active.contains(&key) {
            self.mark_active_descendants(key);
        }
    }
    /// An active descendant's edge columns can resolve to this tile when the
    /// finer tile it faces is not selected; its mesh must follow this one.
    fn mark_active_descendants(&mut self, key: TileKey) {
        if !self.plan.split.contains(&key) {
            return;
        }
        for child in key.children() {
            if !self.plan.nodes.contains(&child) {
                continue;
            }
            if self.active.contains(&child) {
                self.mark_mesh(child);
            }
            self.mark_active_descendants(child);
        }
    }
    /// Neighbours only along the sides this footprint touches: a large
    /// changed tile otherwise marks the whole world.
    fn mark_side_neighbors_of(&mut self, key: TileKey) {
        let [kx, _, kz] = key.origin();
        let size = key.size();
        let mut node = key;
        loop {
            let [nx, _, nz] = node.origin();
            let extent = node.size();
            if kx == nx {
                self.mark_side_neighbors(node, 0, key);
            }
            if kx + size == nx + extent {
                self.mark_side_neighbors(node, 1, key);
            }
            if kz == nz {
                self.mark_side_neighbors(node, 2, key);
            }
            if kz + size == nz + extent {
                self.mark_side_neighbors(node, 3, key);
            }
            if node.level == MAX_LEVEL {
                break;
            }
            node = node.parent();
        }
    }
    /// The interval a tile covers along the edge of `side` (west/east run
    /// along z, north/south along x).
    fn side_interval(key: TileKey, side: usize) -> (i32, i32) {
        let [x, _, z] = key.origin();
        if side < 2 {
            (z, z + key.size())
        } else {
            (x, x + key.size())
        }
    }
    fn side_overlaps(candidate: TileKey, side: usize, changed: TileKey) -> bool {
        let (c0, c1) = Self::side_interval(candidate, side);
        let (k0, k1) = Self::side_interval(changed, side);
        c0 < k1 && k0 < c1
    }
    /// Whether `child` sits on the face of `parent` shared with the tile the
    /// walk came from. Children behind that face are not adjacent to it.
    fn side_flush(child: TileKey, side: usize, parent: TileKey) -> bool {
        let (cs, ps) = (child.size(), parent.size());
        match side {
            0 => (child.x + 1) * cs == (parent.x + 1) * ps,
            1 => child.x * cs == parent.x * ps,
            2 => (child.z + 1) * cs == (parent.z + 1) * ps,
            _ => child.z * cs == parent.z * ps,
        }
    }
    /// The selected tiles across one side of `node`, descending only through
    /// split neighbours that face the changed footprint.
    fn mark_side_neighbors(&mut self, node: TileKey, side: usize, changed: TileKey) {
        let (dx, dz) = match side {
            0 => (-1, 0),
            1 => (1, 0),
            2 => (0, -1),
            _ => (0, 1),
        };
        let neighbor = TileKey {
            x: node.x + dx,
            z: node.z + dz,
            level: node.level,
        };
        self.mark_neighbor_tree(neighbor, side, changed);
    }
    fn mark_neighbor_tree(&mut self, neighbor: TileKey, side: usize, changed: TileKey) {
        if !self.plan.nodes.contains(&neighbor) {
            return;
        }
        if self.active.contains(&neighbor) {
            self.mark_mesh(neighbor);
        }
        if self.plan.split.contains(&neighbor) {
            for child in neighbor.children() {
                if Self::side_flush(child, side, neighbor)
                    && Self::side_overlaps(child, side, changed)
                {
                    self.mark_neighbor_tree(child, side, changed);
                }
            }
        }
    }
    /// `Plan::select` as a cached state: a node is complete when its ready
    /// subtree can stand alone, and active when its own mesh is drawn.
    fn evaluate(&self, key: TileKey) -> (bool, bool) {
        let present = self.ready.contains_key(&key);
        // Covered level-0 stand-ins complete their parent but are never drawn.
        let ready = present && !(key.level == 0 && self.covered.contains(&(key.x, key.z)));
        // Mirrors `Plan::select`: a ready split tile away from the seam is a
        // complete stand-in even while its children are still incomplete,
        // so no second fallback is drawn above it.
        let (complete, active) = if self.plan.split.contains(&key) {
            let children = key.children().into_iter().all(|child| {
                !self.plan.nodes.contains(&child)
                    || self.complete.get(&child).copied().unwrap_or(false)
            });
            let ready = ready && self.plan.stands_in(key);
            (children || ready, ready && !children)
        } else {
            (present, ready)
        };
        (complete, active)
    }
    /// Re-evaluates `key` and its ancestors after its readiness or coverage
    /// changed. Work is O(tree depth), not O(plan), per finished sample.
    fn refresh_path(&mut self, key: TileKey) {
        let mut node = key;
        loop {
            if !self.plan.nodes.contains(&node) {
                break;
            }
            let (complete, active) = self.evaluate(node);
            let old = self.complete.insert(node, complete);
            let was_active = self.active.contains(&node);
            if active != was_active {
                if active {
                    self.active.insert(node);
                    self.removed_pending.remove(&node);
                } else {
                    self.active.remove(&node);
                    self.removed_pending.insert(node);
                    self.cancel_mesh(node);
                    self.mesh_version.remove(&node);
                    self.shown_version.remove(&node);
                    self.meshed.remove(&node);
                    self.unmeshed.remove(&node);
                    self.dirty.remove(&node);
                    self.transfer_cutout_wait(node);
                }
            }
            // A pure completion change on an undrawn node changes no mesh;
            // only the ancestors' own transitions can. Marking descendants
            // here made tree completion remesh the whole view.
            if active != was_active {
                self.mark_selection_changed(node);
            }
            if old == Some(complete) || node.level >= MAX_LEVEL {
                break;
            }
            node = node.parent();
        }
    }
    /// New or replaced samples for `key`. `existed`: its old data was drawn,
    /// so the tile and its edge neighbours must be remeshed; a new tile is
    /// already dirty through its selection change.
    fn refresh_ready(&mut self, key: TileKey, existed: bool) {
        self.refresh_path(key);
        if existed {
            self.mark_data_changed(key);
        }
    }
    pub fn invalidate_chunk(&mut self, x: i32, z: i32) {
        // The tiles touching the chunk: a few per level, not every active one.
        for level in 0..=MAX_LEVEL {
            let size = 16 << level;
            for kz in (z * 16).div_euclid(size) - 1..=((z + 1) * 16).div_euclid(size) {
                for kx in (x * 16).div_euclid(size) - 1..=((x + 1) * 16).div_euclid(size) {
                    let key = TileKey {
                        x: kx,
                        z: kz,
                        level,
                    };
                    let [ox, _, oz] = key.origin();
                    if x * 16 <= ox + size
                        && (x + 1) * 16 >= ox
                        && z * 16 <= oz + size
                        && (z + 1) * 16 >= oz
                        && self.active.contains(&key)
                    {
                        self.mark_mesh(key);
                    }
                }
            }
        }
    }
    /// The source has new data for the blocks `[x0, z0, x1, z1)` (a map tile
    /// downloaded since), drawn in cells `widths` `(min, max)` blocks wide:
    /// the tiles of those cells over them or beside (`refreshed_by`) are
    /// sampled again, and their chunks' detail asked again, but for the
    /// chunks seen in the world. A tile whose new samples are the old ones
    /// isn't meshed again.
    pub fn refresh(&mut self, area: [i32; 4], widths: (i32, i32)) {
        self.refreshes += 1;
        if self.refreshed.len() >= MAX_REFRESHED {
            self.refreshed.pop_front();
        }
        // Detail is of cells up to a chunk wide, which read a block around:
        // a chunk around is plenty.
        let m = 16;
        let [x0, z0, x1, z1] = area;
        self.refreshed.push_back((
            self.refreshes,
            [
                x0.saturating_sub(m),
                z0.saturating_sub(m),
                x1.saturating_add(m),
                z1.saturating_add(m),
            ],
        ));
        // Tiles waiting out a retry (`finish_waited`) are asked again now.
        let retrying = self.retry.len();
        self.retry.retain(|key, _| !key.refreshed_by(area, widths));
        if self.retry.len() != retrying {
            self.wanted_stale = true;
        }
        // A first sample in flight read the old data too.
        let keys: Vec<_> = self
            .ready
            .keys()
            .chain(self.pending.iter())
            .copied()
            .filter(|key| {
                key.refreshed_by(area, widths)
                    && !(key.level == 0 && self.covered.contains(&(key.x, key.z)))
            })
            .collect();
        for key in keys {
            self.mark_stale(key);
            if let Some(mask) = self.detail_masks.get_mut(&key) {
                let area = TileKey::around(area, key.size() / CELLS as i32);
                forget_detail(key, area, mask, |chunk| self.saved.contains_key(&chunk));
            }
        }
    }
    /// Whether chunk `(cx, cz)` was under a `refresh` after request `asked`
    /// (or one too old to tell).
    fn refreshed_since(&self, (cx, cz): (i32, i32), asked: u64) -> bool {
        if asked >= self.refreshes {
            return false;
        }
        let oldest = self.refreshed.front().map_or(u64::MAX, |&(n, _)| n);
        asked + 1 < oldest
            || self.refreshed.iter().any(|&(n, [x0, z0, x1, z1])| {
                n > asked
                    && cx * 16 < x1
                    && (cx + 1) * 16 > x0
                    && cz * 16 < z1
                    && (cz + 1) * 16 > z0
            })
    }
    /// The tile's samples lack data its source has now: sampled again, the
    /// old ones drawn meanwhile, and once more after a sample in flight.
    fn mark_stale(&mut self, key: TileKey) {
        self.stale.insert(key);
        self.wanted_stale = true;
        if self.pending.contains(&key) {
            self.restale.insert(key);
        }
    }
    /// Capture actual block columns, including canopy gaps and material
    /// layers: the worker summarizes the chunk (`summarize`, a few
    /// milliseconds a chunk), then `finish_observed` keeps it.
    pub fn observe(&mut self, chunk: Arc<Chunk>, facts: &BlockFacts) {
        self.shared.kinds.get_or_init(|| state_kinds(facts));
        let key = (chunk.x, chunk.z);
        if self
            .shared
            .observed
            .lock()
            .unwrap()
            .insert(key, chunk)
            .is_none()
        {
            self.shared
                .jobs
                .lock()
                .unwrap()
                .push_back(Job::Observe(key));
            self.shared.wake.notify_all();
        }
    }
    /// Chunks waiting for the worker to summarize them. A resident chunk
    /// held here is copied by its next edit (`Arc::make_mut`), so callers
    /// hand over a few at a time (`OBSERVE_AHEAD`).
    pub fn observing(&self) -> usize {
        self.shared.observed.lock().unwrap().len()
    }
    /// Whether tiles new to the plan are being sampled: far terrain on its way,
    /// meshed as soon as it is.
    pub fn sampling(&self) -> bool {
        !self.pending.is_empty()
    }
    /// Keeps a visited chunk's summary, unless it is unchanged.
    fn finish_observed(&mut self, key: (i32, i32), columns: ReducedColumns) {
        if self
            .saved
            .get(&key)
            .is_some_and(|saved| **saved == *columns.columns)
        {
            return;
        }
        if !self.saved.contains_key(&key) {
            while self.saved.len() >= MAX_SAVED {
                if let Some(old) = self.saved_order.pop_front() {
                    self.saved.remove(&old);
                    self.reduced.remove(&old);
                }
            }
            self.saved_order.push_back(key);
        }
        self.saved.insert(key, columns.columns.clone());
        if self.keeps_observed {
            self.resample_over(key);
        }
        self.finish_detail(key, columns);
    }
    /// The source keeps the chunk now: the tiles over it whose samples lack
    /// it are sampled again (`stale`), their old samples drawn meanwhile.
    fn resample_over(&mut self, chunk: (i32, i32)) {
        for key in resampled_over(&self.ready, &self.pending, chunk) {
            self.mark_stale(key);
        }
    }
    /// A chunk's own columns as its level-0 tile; the same columns again
    /// (asked again after a `refresh`) mesh nothing.
    fn apply_fine(&mut self, (cx, cz): (i32, i32), columns: &[Column; 256]) {
        let fine = TileKey {
            x: cx,
            z: cz,
            level: 0,
        };
        if !self.plan.nodes.contains(&fine) && !self.ready.contains_key(&fine) {
            return;
        }
        self.detail_masks.insert(fine, [u64::MAX; 4]);
        if self
            .ready
            .get(&fine)
            .is_some_and(|old| old[..] == columns[..])
        {
            return;
        }
        let existed = self
            .ready
            .insert(fine, Arc::new(columns.to_vec()))
            .is_some();
        self.refresh_ready(fine, existed);
    }
    fn apply_detail(&mut self, (cx, cz): (i32, i32), columns: &ReducedColumns) {
        self.apply_fine((cx, cz), &columns.columns);
        let mut modified = Vec::new();
        // A single generated chunk cannot describe a larger cell's area
        // (strides over 16 blocks, levels over 4); at each level up to that,
        // only the tile holding the chunk overlaps it.
        for level in 0..=4u8 {
            let size = 16 << level;
            let key = TileKey {
                x: (cx * 16).div_euclid(size),
                z: (cz * 16).div_euclid(size),
                level,
            };
            let Some(samples) = self.ready.get_mut(&key) else {
                continue;
            };
            let [ox, _, oz] = key.origin();
            let stride = key.size() / 16;
            let x0 = ((cx * 16 - ox + stride - 1).div_euclid(stride)).clamp(0, 16);
            let x1 = (((cx + 1) * 16 - ox + stride - 1).div_euclid(stride)).clamp(0, 16);
            let z0 = ((cz * 16 - oz + stride - 1).div_euclid(stride)).clamp(0, 16);
            let z1 = (((cz + 1) * 16 - oz + stride - 1).div_euclid(stride)).clamp(0, 16);
            if x0 == x1 || z0 == z1 {
                continue;
            }
            let mask = self.detail_masks.entry(key).or_default();
            let mut changed = false;
            for z in z0..z1 {
                for x in x0..x1 {
                    let index = (z * 16 + x) as usize;
                    let column = columns.get(
                        (ox + x * stride).rem_euclid(16) as usize,
                        (oz + z * stride).rem_euclid(16) as usize,
                        stride as usize,
                    );
                    // Copied (from a mesh job's snapshot) only to change it.
                    if samples[index] != *column {
                        Arc::make_mut(samples)[index] = column.clone();
                        changed = true;
                    }
                    mask[index / 64] |= 1 << (index % 64);
                }
            }
            if changed {
                modified.push(key);
            }
        }
        for key in modified {
            self.mark_data_changed(key);
        }
    }
    /// A sample with a hole, its data still on its way and nothing coarser
    /// to stand in: drawn, the hole stood where the coarser tile or the older
    /// sample it replaces had terrain, until the data came (`refresh`) and
    /// the tile was sampled again. So it is thrown away, and the tile sampled
    /// again then (at once if it came while this one was taken), or after
    /// `WAIT_RETRY`: a shorter wait let the retries of near tiles take every
    /// slot while the rest waited. One whose waiting cells BlueMap's coarser
    /// levels stood in for is drawn (`finish_any_sample`).
    fn finish_waited(&mut self, key: TileKey) {
        self.pending.remove(&key);
        if self.restale.remove(&key) {
            self.wanted_stale = true;
        } else if self.plan.nodes.contains(&key) {
            self.retry
                .insert(key, std::time::Instant::now() + WAIT_RETRY);
        }
    }
    /// A sample in: thrown away when a cell of it is a hole, its data on its
    /// way (`finish_waited`); one whose waiting cells a coarser level stood
    /// in for is drawn (`LodSampler::waited`).
    fn finish_any_sample(&mut self, key: TileKey, samples: Vec<Column>, waited: bool) {
        if waited {
            self.finish_waited(key);
        } else {
            self.finish_sample(key, samples);
        }
    }
    fn finish_sample(&mut self, key: TileKey, samples: Vec<Column>) {
        self.pending.remove(&key);
        if samples.len() != CELLS * CELLS {
            let now = std::time::Instant::now();
            if self.retry.len() > 4096 {
                self.retry.retain(|_, at| *at > now);
            }
            self.retry.insert(key, now + RETRY);
            self.restale.remove(&key);
            return;
        }
        self.retry.remove(&key);
        if self.plan.nodes.contains(&key) {
            let mut samples = samples;
            if let (Some(old), Some(mask)) = (self.ready.get(&key), self.detail_masks.get(&key)) {
                for i in 0..256 {
                    if mask[i / 64] & (1 << (i % 64)) != 0 {
                        samples[i] = old[i].clone();
                    }
                }
            }
            // Refreshed again meanwhile: sampled once more after this.
            if self.restale.remove(&key) {
                self.wanted_stale = true;
            } else {
                self.stale.remove(&key);
            }
            if self.ready.get(&key).is_some_and(|old| **old == samples) {
                return;
            }
            let existed = self.ready.insert(key, Arc::new(samples)).is_some();
            self.refresh_ready(key, existed);
        }
    }
    fn finish_detail(&mut self, key: (i32, i32), columns: ReducedColumns) {
        if let Some(visited) = self.saved.get(&key).cloned()
            && !Arc::ptr_eq(&visited, &columns.columns)
        {
            // Its latest reduction is queued; never restore old edits or seed data.
            self.apply_fine(key, &visited);
            return;
        }
        self.apply_detail(key, &columns);
        if self.saved.contains_key(&key) {
            self.reduced.insert(key, Arc::new(columns));
        }
        // `apply_detail` marks every tile whose columns it replaced, and their
        // neighbours. Coarser tiles do not read visited columns (stride > 16),
        // so re-meshing every tile touching the chunk was pure waste.
    }
    /// Whether the near terrain is still loading: far terrain then queues
    /// `SAMPLE_JOBS` at most, and on small machines holds back
    /// (`HELD_SAMPLE_STEP`).
    pub fn set_near_loading(&mut self, loading: bool) {
        if loading && !self.near_loading {
            // The queue filled while nothing loaded nearby; the near chunks
            // must not wait behind it. Larger machines keep its first
            // `SAMPLE_JOBS`, small ones none.
            let mut keep = if self.small { 0 } else { SAMPLE_JOBS };
            let dropped = drop_queued_samples(&self.shared, &mut self.pending, |_| {
                let kept = keep > 0;
                keep = keep.saturating_sub(1);
                kept
            });
            // Sampled again from the queue: they read what `refresh` announced.
            for key in &dropped {
                self.restale.remove(key);
            }
            // Back in line where `wanted_tiles` put them: the near terrain can
            // start and stop loading every few frames, and rebuilding the whole
            // queue each time cost more than the samples it held back.
            let camera = self.plan.camera;
            self.wanted.extend(dropped.into_iter().map(|key| {
                let class = if self.plan.split.contains(&key) {
                    BACKDROP
                } else if key.reach(camera) < self.plan.first {
                    SEAM
                } else {
                    REST
                };
                std::cmp::Reverse(Wanted {
                    class,
                    distance: i64::from(key.distance(camera)),
                    x: key.x,
                    z: key.z,
                    level: key.level,
                })
            }));
            // A sample may still be running: the held pace starts from now.
            self.held_sample_at = std::time::Instant::now();
        }
        self.near_loading = loading;
        let held = self.held();
        if self.shared.mesh_held.swap(held, Ordering::Release) && !held {
            self.shared.mesh_wake.notify_all();
        }
    }
    /// Reduced far detail (`graphics::FarDetail::Reduced`): the next update
    /// replans when it changes, and tiles sampled from then on approximate
    /// wide cells (`GeneratorWorker::lod_grid`).
    pub fn set_reduced_detail(&mut self, reduced: bool) {
        self.reduced_detail = reduced;
        self.shared.approximate.store(reduced, Ordering::Relaxed);
    }
    fn held(&self) -> bool {
        self.small && self.near_loading
    }
    fn request_detail(&mut self, camera: [i32; 2]) {
        if self.held()
            || self.detail_request_pending
            || self.detail_request_at.elapsed() < std::time::Duration::from_millis(250)
        {
            return;
        }
        self.detail_request_at = std::time::Instant::now();
        let mut keys: Vec<_> = self
            .active
            .iter()
            .filter(|key| {
                self.wants_detail(key) && self.detail_masks.get(key) != Some(&[u64::MAX; 4])
            })
            .copied()
            .collect();
        // Retiring fine columns are empty stand-ins, excluded from `active`
        // while their normal meshes still cover the gap. Request their detail
        // explicitly: neither the active-tile requests nor sampling an already
        // ready stand-in can fill it, so its GPU sections would never retire.
        keys.extend(retiring_detail(
            &self.plan,
            &self.retiring,
            &self.saved,
            &self.detail_masks,
        ));
        keys.sort_unstable_by_key(|key| (key.distance(camera), *key));
        let mut chunks = FxHashSet::default();
        for key in keys {
            let [ox, _, oz] = key.origin();
            let stride = key.size() / 16;
            let mask = self.detail_masks.get(&key).copied().unwrap_or_default();
            for index in 0..256 {
                if mask[index / 64] & (1 << (index % 64)) == 0 {
                    let chunk = (
                        (ox + (index as i32 % 16) * stride).div_euclid(16),
                        (oz + (index as i32 / 16) * stride).div_euclid(16),
                    );
                    // A retiring column's tile is drawn soon: it needs its detail.
                    if self.full.contains(&chunk) {
                        self.detail_masks.entry(key).or_default()[index / 64] |= 1 << (index % 64);
                    } else {
                        chunks.insert(chunk);
                    }
                }
            }
            if chunks.len() >= DETAIL_REQUEST {
                break;
            }
        }
        let mut chunks: Vec<_> = chunks.into_iter().collect();
        chunks.sort_unstable_by_key(|&(x, z)| {
            let dx = i64::from(x * 16 + 8 - camera[0]);
            let dz = i64::from(z * 16 + 8 - camera[1]);
            dx * dx + dz * dz
        });
        chunks.truncate(DETAIL_REQUEST);
        if !chunks.is_empty() {
            self.detail_request_pending = true;
            self.shared
                .jobs
                .lock()
                .unwrap()
                .push_back(Job::RequestDetail {
                    chunks,
                    center: camera,
                    refreshes: self.refreshes,
                });
            self.shared.wake.notify_all();
        }
    }
    pub fn update(
        &mut self,
        camera: [f64; 3],
        detail: i32,
        distance: u32,
        covered: Option<FxHashSet<(i32, i32)>>,
    ) -> Vec<(TileKey, SectionMesh)> {
        let elevation =
            (((camera[1] - f64::from(MIN_Y + crate::chunk::HEIGHT)).clamp(0., 16384.)) as i32 / 32)
                * 32;
        let camera = [camera[0].floor() as i32, camera[2].floor() as i32];
        // Replan at chunk boundaries, not on every camera movement.
        let area = (
            [camera[0].div_euclid(16), camera[1].div_euclid(16)],
            detail,
            distance,
            elevation,
            self.reduced_detail,
        );
        if self.area != Some(area) {
            crate::spans::time("lod_plan", || {
                self.area = Some(area);
                self.replan(camera, detail, distance, elevation);
            });
        }
        let mut meshes = Vec::new();
        crate::spans::time("lod_results", || self.receive(&mut meshes));
        crate::spans::time("lod_retiring", || self.prune_retiring());
        // Uploaded over frames in this order (`Renderer::upload_lods`): a
        // tile is meshed clipped around its finer ones, so they go first,
        // and the nearest first, where far terrain meets the near chunks.
        meshes.sort_by_key(|(key, _)| (key.level, key.distance(camera)));
        crate::spans::time("lod_covered", || self.cover(covered));
        if std::mem::take(&mut self.reselect) {
            self.rebuild_selection();
        }
        crate::spans::time("lod_request", || self.request_detail(camera));
        crate::spans::time("lod_wanted", || self.request_samples(camera));
        crate::spans::time("lod_dispatch", || self.dispatch_meshes(camera));
        crate::spans::time("lod_removed", || self.release_removed());
        self.publish_cutout_if_ready();
        meshes
    }
    /// Rebuilds the quadtree and the selection it implies. Only runs when the
    /// planned area changes (a chunk boundary), never per finished sample.
    fn replan(&mut self, camera: [i32; 2], detail: i32, distance: u32, elevation: i32) {
        self.plan = Plan::with_previous(
            camera,
            detail,
            distance,
            &self.plan,
            elevation,
            self.reduced_detail,
        );
        self.ready.retain(|key, _| self.plan.nodes.contains(key));
        // Queued tiles the view left are not sampled (a running one is
        // dropped when it finishes, `finish_sample`).
        drop_queued_samples(&self.shared, &mut self.pending, |key| {
            self.plan.nodes.contains(key)
        });
        self.stale.retain(|key| self.plan.nodes.contains(key));
        self.restale.retain(|key| self.plan.nodes.contains(key));
        self.detail_masks
            .retain(|key, _| self.plan.nodes.contains(key));
        // Complete resident columns need no duplicate noise work.
        let mut standins = Vec::new();
        for &key in &self.plan.by_level[0] {
            if self.covered.contains(&(key.x, key.z)) && !self.ready.contains_key(&key) {
                self.ready
                    .insert(key, Arc::new(vec![Column::default(); CELLS * CELLS]));
                standins.push(key);
            }
        }
        self.rebuild_selection();
        for key in standins {
            self.refresh_path(key);
        }
    }
    /// Recomputes every active tile after a plan change, then marks only the
    /// tiles whose selection changed (plus their neighbours).
    fn rebuild_selection(&mut self) {
        self.reselect = false;
        let old = std::mem::take(&mut self.active);
        self.complete.clear();
        // The plan indexes nodes by level, children before parents. Plans
        // built by tests may not; sort those once.
        let keys: Vec<TileKey> = if self.plan.by_level.iter().any(|level| !level.is_empty()) {
            self.plan.by_level.iter().flatten().copied().collect()
        } else {
            let mut keys: Vec<_> = self.plan.nodes.iter().copied().collect();
            keys.sort_unstable_by_key(|key| (key.level, key.x, key.z));
            keys
        };
        for key in keys {
            let (complete, active) = self.evaluate(key);
            self.complete.insert(key, complete);
            if active {
                self.active.insert(key);
            }
        }
        self.mesh_version.retain(|key, _| self.active.contains(key));
        self.shown_version
            .retain(|key, _| self.active.contains(key));
        self.dirty.retain(|key, _| self.active.contains(key));
        self.meshed.retain(|key| self.active.contains(key));
        self.unmeshed.retain(|key| self.active.contains(key));
        let inactive_waiting: Vec<_> = self
            .cutout_waiting
            .iter()
            .copied()
            .filter(|key| !self.active.contains(key))
            .collect();
        for key in inactive_waiting {
            self.transfer_cutout_wait(key);
        }
        self.removed_pending
            .retain(|key| !self.active.contains(key));
        let changed: Vec<_> = old.symmetric_difference(&self.active).copied().collect();
        for &key in &changed {
            if old.contains(&key) && !self.active.contains(&key) {
                self.cancel_mesh(key);
                self.removed_pending.insert(key);
                self.meshed.remove(&key);
                self.dirty.remove(&key);
            }
            self.mark_selection_changed(key);
        }
        self.wanted_stale = true;
    }
    fn receive(&mut self, meshes: &mut Vec<(TileKey, SectionMesh)>) {
        while let Ok(done) = self.results.try_recv() {
            match done {
                // Read before a `refresh` over it since: asked again (its
                // bits stay clear), never kept over the newer data.
                Done::Detail(key, _, asked) if self.refreshed_since(key, asked) => {}
                Done::Detail(key, columns, _) => {
                    crate::spans::time("lod_detail", || self.finish_detail(key, columns))
                }
                Done::Observed(key, columns) => {
                    crate::spans::time("lod_detail", || self.finish_observed(key, columns))
                }
                Done::DetailRequested => self.detail_request_pending = false,
                Done::Sample {
                    key,
                    samples,
                    waited,
                } => self.finish_any_sample(key, samples, waited),
                Done::Mesh {
                    key,
                    version,
                    mesh,
                    canceled,
                } => {
                    self.outstanding_meshes = self.outstanding_meshes.saturating_sub(1);
                    self.accept_mesh(key, version, mesh, canceled.load(Ordering::Acquire), meshes);
                }
                Done::MeshCanceled { key, version } => {
                    self.outstanding_meshes = self.outstanding_meshes.saturating_sub(1);
                    if self.mesh_version.get(&key) == Some(&version) {
                        self.mesh_cancel.remove(&key);
                    }
                }
            }
        }
    }
    /// A finished mesh: shown when it is newer than what the tile shows now.
    /// A result newer than the frame's current one is still useful while a
    /// newer job is pending — re-marking a tile re-dirties its neighbours and
    /// ancestors, so on a busy frontier the newest job is not the one that
    /// arrives next, and accepting only it left tiles unmeshed with their
    /// removals blocked.
    fn accept_mesh(
        &mut self,
        key: TileKey,
        version: u64,
        mesh: SectionMesh,
        canceled: bool,
        meshes: &mut Vec<(TileKey, SectionMesh)>,
    ) {
        let newest = self.mesh_version.get(&key) == Some(&version);
        if canceled
            || !self.active.contains(&key)
            || self
                .shown_version
                .get(&key)
                .is_some_and(|shown| version <= *shown)
        {
            return;
        }
        self.shown_version.insert(key, version);
        meshes.push((key, mesh));
        self.meshed.insert(key);
        // Only the newest revision settles the tile; an older one fills the
        // frame and stays dirty, so the pending job still runs.
        if newest {
            self.mesh_cancel.remove(&key);
            self.unmeshed.remove(&key);
            self.cutout_waiting.remove(&key);
        }
    }
    /// Takes the new full-detail columns, when they changed (`None`: not).
    fn cover(&mut self, covered: Option<FxHashSet<(i32, i32)>>) {
        if let Some(full) = covered
            && self.full != full
        {
            self.retiring.extend(self.full.difference(&full).copied());
            self.retiring.retain(|column| !full.contains(column));
            self.full = full;
            self.leave_out();
        }
    }
    /// Releases retiring columns whose replacement is ready, then leaves out
    /// the full-detail and retiring columns: tiles touching a change remesh.
    fn prune_retiring(&mut self) {
        if self.retiring.is_empty() {
            return;
        }
        let (plan, ready, masks) = (&self.plan, &self.ready, &self.detail_masks);
        let keep: FxHashSet<_> = self
            .retiring
            .iter()
            .copied()
            .filter(|&column| {
                // A saved summary alone is not detail: the tile may still be
                // the empty covered stand-in. Only a completed mask proves
                // its columns replaced the full-detail mesh.
                !plan.replaces(column, ready, |key| masks.get(&key) == Some(&[u64::MAX; 4]))
            })
            .collect();
        if keep.len() != self.retiring.len() {
            self.retiring = keep;
            self.leave_out();
        }
    }
    fn leave_out(&mut self) {
        let covered: FxHashSet<_> = self.full.union(&self.retiring).copied().collect();
        if self.covered == covered {
            return;
        }
        let added: Vec<_> = covered.difference(&self.covered).copied().collect();
        let changes: Vec<_> = self
            .covered
            .symmetric_difference(&covered)
            .copied()
            .collect();
        self.covered = covered;
        self.covered_bounds = self.covered.iter().fold(None, |bounds, &(x, z)| {
            Some(match bounds {
                None => [x * 16, z * 16, (x + 1) * 16, (z + 1) * 16],
                Some([x0, z0, x1, z1]) => [
                    x0.min(x * 16),
                    z0.min(z * 16),
                    x1.max((x + 1) * 16),
                    z1.max((z + 1) * 16),
                ],
            })
        });
        for &(x, z) in &changes {
            self.invalidate_chunk(x, z);
        }
        let mut standins = Vec::new();
        for &(x, z) in &added {
            let key = TileKey { x, z, level: 0 };
            if self.plan.nodes.contains(&key) && !self.ready.contains_key(&key) {
                self.ready
                    .insert(key, Arc::new(vec![Column::default(); CELLS * CELLS]));
                standins.push(key);
            }
        }
        for key in standins {
            self.refresh_path(key);
        }
        for &(x, z) in &changes {
            self.refresh_path(TileKey { x, z, level: 0 });
        }
        self.cutout_pending = Some(self.covered.clone());
        // Every active tile whose mesh clips an old/new covered column is
        // dirty now; the cutout waits for their replacement meshes, not for
        // every later job.
        self.cutout_waiting.extend(
            self.dirty
                .keys()
                .filter(|key| self.active.contains(key))
                .copied(),
        );
    }
    /// A waiting tile that left the selection is replaced by its active
    /// descendants; the cutout must wait for those meshes instead.
    fn transfer_cutout_wait(&mut self, key: TileKey) {
        if !self.cutout_waiting.remove(&key) {
            return;
        }
        if key.level == 0 {
            return;
        }
        let mut descendants = Vec::new();
        self.collect_active_descendants(key, &mut descendants);
        self.cutout_waiting.extend(
            descendants
                .into_iter()
                .filter(|key| !self.meshed.contains(key)),
        );
    }
    /// Publishes the pending cutout once every active tile that had to stop
    /// clipping the changed columns has an accepted mesh.
    fn publish_cutout_if_ready(&mut self) {
        if self.cutout_pending.is_none() || !self.cutout_waiting.is_empty() {
            return;
        }
        self.cutout = self.cutout_pending.take();
    }
    /// Releases tiles whose every overlapping replacement already has a mesh.
    /// Quadtree tiles are nested or disjoint, so blocking is an ancestor test:
    /// O(pending + unm meshed active) tree walks, not pending x active.
    fn release_removed(&mut self) {
        if self.removed_pending.is_empty() {
            return;
        }
        let mut blocked = FxHashSet::default();
        for &removed in &self.removed_pending {
            let mut node = removed;
            loop {
                if self.active.contains(&node) && !self.meshed.contains(&node) {
                    blocked.insert(removed);
                    break;
                }
                if node.level >= MAX_LEVEL {
                    break;
                }
                node = node.parent();
            }
        }
        for &active in &self.unmeshed {
            let mut node = active;
            loop {
                if self.removed_pending.contains(&node) {
                    blocked.insert(node);
                }
                if node.level >= MAX_LEVEL {
                    break;
                }
                node = node.parent();
            }
        }
        let pending: Vec<_> = self.removed_pending.iter().copied().collect();
        for key in pending {
            if self.active.contains(&key) || blocked.contains(&key) {
                continue;
            }
            self.removed_pending.remove(&key);
            self.removed.push(key);
        }
        self.removed_pending
            .retain(|key| !self.active.contains(key));
    }
    /// Queues every dirty active tile with a shared immutable snapshot. One
    /// batch used to clone all maps and serialize the whole view on one worker.
    fn dispatch_meshes(&mut self, camera: [i32; 2]) {
        if self.dirty.is_empty() || self.held() {
            return;
        }
        // Under load, let the pool drain and send one larger batch with one
        // snapshot instead of rebuilding the snapshot 20 times a second.
        if self.outstanding_meshes >= self.mesh_workers * 4 {
            return;
        }
        let keys: Vec<(TileKey, u64)> = self
            .dirty
            .drain()
            .filter(|(key, _)| self.active.contains(key) && self.ready.contains_key(key))
            .collect();
        if keys.is_empty() {
            return;
        }
        let snapshot = Arc::new(Snapshot {
            samples: self
                .active
                .iter()
                .filter_map(|key| self.ready.get(key).map(|columns| (*key, columns.clone())))
                .collect(),
            saved: self.saved.clone(),
            reduced: self.reduced.clone(),
        });
        let covered = Arc::new(self.covered.clone());
        let mut queued = self.shared.mesh_jobs.lock().unwrap();
        let count = keys.len();
        for (key, version) in keys {
            let finer = self.finer_of(key);
            let canceled = Arc::new(AtomicBool::new(false));
            self.mesh_cancel.insert(key, canceled.clone());
            // Screen-space urgency: nearer blocks cost more pixels per work.
            let priority = i64::from(key.distance(camera)) * 16 / i64::from(key.size());
            queued.push(MeshQueued {
                priority,
                order: version,
                job: MeshJob {
                    snapshot: snapshot.clone(),
                    key,
                    version,
                    finer,
                    covered: covered.clone(),
                    bounds: self.covered_bounds,
                    canceled,
                },
            });
        }
        drop(queued);
        self.outstanding_meshes += count;
        self.shared.mesh_wake.notify_all();
    }
    fn finer_of(&self, key: TileKey) -> Vec<TileKey> {
        if !self.plan.split.contains(&key) {
            return Vec::new();
        }
        let mut finer = Vec::new();
        self.collect_active_descendants(key, &mut finer);
        finer
    }
    fn collect_active_descendants(&self, key: TileKey, out: &mut Vec<TileKey>) {
        for child in key.children() {
            if !self.plan.nodes.contains(&child) {
                continue;
            }
            if self.active.contains(&child) {
                out.push(child);
            }
            if self.plan.split.contains(&child) {
                self.collect_active_descendants(child, out);
            }
        }
    }
    fn request_samples(&mut self, camera: [i32; 2]) {
        // Every sample slot busy: no tile can be requested this frame.
        let free = if self.held() {
            usize::from(
                self.pending.is_empty() && self.held_sample_at.elapsed() >= HELD_SAMPLE_STEP,
            )
        } else {
            // `SAMPLE_JOBS` while the near terrain loads, the full queue
            // (`QUEUED_PER_SAMPLER` a thread) once it is in.
            let slots = if self.near_loading {
                SAMPLE_JOBS
            } else {
                self.sample_jobs
            };
            slots.saturating_sub(self.pending.len())
        };
        if free == 0 {
            return;
        }
        let now = std::time::Instant::now();
        if self.held() {
            self.held_sample_at = now;
        }
        // A tile whose sampling failed waits out `RETRY`; once it has, the
        // queue is rebuilt so the tile is asked again.
        let retrying = self.retry.len();
        self.retry.retain(|_, at| *at > now);
        if self.retry.len() != retrying {
            self.wanted_stale = true;
        }
        // The queue is rebuilt when the view moves a chunk, not scanned and
        // sorted on every frame.
        let view = [camera[0].div_euclid(16), camera[1].div_euclid(16)];
        if self.wanted_stale || self.wanted_at != Some(view) {
            self.rebuild_wanted(camera);
            self.wanted_at = Some(view);
            self.wanted_stale = false;
        }
        let mut issued = 0;
        let mut jobs = self.shared.jobs.lock().unwrap();
        while issued < free {
            let Some(std::cmp::Reverse(wanted)) = self.wanted.pop() else {
                break;
            };
            let key = TileKey {
                x: wanted.x,
                z: wanted.z,
                level: wanted.level,
            };
            // Final tiles and the backdrop only (`wanted_tiles`).
            if self.pending.contains(&key)
                || self.sampled(key)
                || (self.plan.split.contains(&key) && wanted.class != BACKDROP)
                || !self.plan.nodes.contains(&key)
                || self.retry.contains_key(&key)
            {
                continue;
            }
            self.pending.insert(key);
            jobs.push_back(Job::Sample(key));
            issued += 1;
        }
        drop(jobs);
        if issued > 0 {
            self.shared.wake.notify_all();
        }
    }
    /// Sampled, and not since `refresh`ed.
    fn sampled(&self, key: TileKey) -> bool {
        self.ready.contains_key(&key) && !self.stale.contains(&key)
    }
    fn rebuild_wanted(&mut self, camera: [i32; 2]) {
        self.wanted.clear();
        // A failed tile waits out its retry delay (`retry`) before it is asked again.
        let retry = &self.retry;
        self.wanted.extend(
            wanted_tiles(&self.plan, &self.ready, &self.stale, camera)
                .filter(|w| {
                    !retry.contains_key(&TileKey {
                        x: w.x,
                        z: w.z,
                        level: w.level,
                    })
                })
                .map(std::cmp::Reverse),
        );
    }

    #[cfg(test)]
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            samples: self
                .active
                .iter()
                .filter_map(|key| self.ready.get(key).map(|columns| (*key, columns.clone())))
                .collect(),
            saved: self.saved.clone(),
            reduced: self.reduced.clone(),
        }
    }
    #[cfg(test)]
    fn mesh(&self, key: TileKey) -> SectionMesh {
        let finer = self.finer_of(key);
        let input = self.snapshot().mesh_input(key, finer);
        let columns = input.columns();
        let edges = input.edges();
        mesh::Mesher::default().mesh(
            key,
            &columns,
            &self.covered,
            self.covered_bounds,
            self.context.as_deref().expect("test mesh context"),
            &edges,
            &input.finer,
        )
    }
}
impl Snapshot {
    /// Actual visited columns at this cell, when a saved/observed chunk has
    /// them. Sampled stand-ins are coarser.
    fn visited_source(&self, wx: i32, wz: i32, width: i32) -> Option<ColumnSource> {
        let key = (wx.div_euclid(16), wz.div_euclid(16));
        if width == 1 {
            self.saved.get(&key).map(|columns| {
                ColumnSource::Saved(
                    columns.clone(),
                    (wz.rem_euclid(16) * 16 + wx.rem_euclid(16)) as u16,
                )
            })
        } else if width <= 16 {
            self.reduced.get(&key).map(|columns| {
                ColumnSource::Reduced(
                    columns.clone(),
                    width as u8,
                    wx.rem_euclid(16) as u16,
                    wz.rem_euclid(16) as u16,
                )
            })
        } else {
            None
        }
    }
    fn neighbor_source(&self, wx: i32, wz: i32) -> Option<(ColumnSource, i32)> {
        for level in 0..=MAX_LEVEL {
            let size = 16 << level;
            let key = TileKey {
                x: wx.div_euclid(size),
                z: wz.div_euclid(size),
                level,
            };
            // `samples` holds exactly the active tiles.
            let Some(columns) = self.samples.get(&key) else {
                continue;
            };
            let stride = 1 << level;
            let [ox, _, oz] = key.origin();
            let x = (wx - ox) / stride;
            let z = (wz - oz) / stride;
            let (sx, sz) = (ox + x * stride, oz + z * stride);
            if let Some(column) = self.visited_source(sx, sz, stride) {
                return Some((column, stride));
            }
            return Some((
                ColumnSource::Tile(columns.clone(), (z * CELLS as i32 + x) as u16),
                stride,
            ));
        }
        None
    }
    fn mesh_input(&self, key: TileKey, finer: Vec<TileKey>) -> MeshInput {
        let stride = key.size() / CELLS as i32;
        let [ox, _, oz] = key.origin();
        let base = self
            .samples
            .get(&key)
            .cloned()
            .unwrap_or_else(|| Arc::new(vec![Column::default(); CELLS * CELLS]));
        let overrides = std::array::from_fn(|index| {
            let x = index % CELLS;
            let z = index / CELLS;
            self.visited_source(ox + x as i32 * stride, oz + z as i32 * stride, stride)
        });
        let neighbors = std::array::from_fn(|direction| {
            let mut edge = Vec::new();
            let mut offset = 0;
            while offset < key.size() {
                let (wx, wz) = match direction {
                    0 => (ox - 1, oz + offset),
                    1 => (ox + key.size(), oz + offset),
                    2 => (ox + offset, oz - 1),
                    _ => (ox + offset, oz + key.size()),
                };
                let (column, neighbor_stride) = self
                    .neighbor_source(wx, wz)
                    .unwrap_or((ColumnSource::Empty, stride));
                let coordinate = if direction < 2 { wz } else { wx };
                let width = (neighbor_stride - coordinate.rem_euclid(neighbor_stride))
                    .min(key.size() - offset);
                edge.push(EdgeSource {
                    start: offset,
                    end: offset + width,
                    column,
                });
                offset += width;
            }
            edge
        });
        MeshInput {
            base,
            overrides,
            neighbors,
            finer,
        }
    }
}
impl Drop for Lod {
    fn drop(&mut self) {
        // Under each pool's own lock: a worker between its stop check and its
        // wait must not miss the wake, or join() hangs.
        let jobs = self.shared.jobs.lock();
        let mesh_jobs = self.shared.mesh_jobs.lock();
        self.shared.stop.store(true, Ordering::Relaxed);
        drop(mesh_jobs);
        drop(jobs);
        self.shared.wake.notify_all();
        self.shared.mesh_wake.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        for thread in self.mesh_threads.drain(..) {
            let _ = thread.join();
        }
    }
}
/// The columns of a tile's `detail_masks` entry that have their detail.
fn detail_count(mask: &[u64; 4]) -> usize {
    mask.iter().map(|bits| bits.count_ones() as usize).sum()
}
/// Removes the queued sample jobs `keep` rejects (in queue order, best
/// first) and frees their slots; the tiles removed.
fn drop_queued_samples(
    shared: &Shared,
    pending: &mut FxHashSet<TileKey>,
    mut keep: impl FnMut(&TileKey) -> bool,
) -> Vec<TileKey> {
    let mut dropped = Vec::new();
    shared.jobs.lock().unwrap().retain(|job| match job {
        Job::Sample(key) if !keep(key) => {
            pending.remove(key);
            dropped.push(*key);
            false
        }
        _ => true,
    });
    dropped
}
/// Fine retiring stand-ins need actual column detail before `Plan::replaces`
/// can release their normal meshes. The caller keeps the usual detail-request
/// budget and waits for the replacement mesh before changing cutout.
fn retiring_detail<'a>(
    plan: &'a Plan,
    retiring: &'a FxHashSet<(i32, i32)>,
    _saved: &'a Saved,
    masks: &'a FxHashMap<TileKey, [u64; 4]>,
) -> impl Iterator<Item = TileKey> + 'a {
    retiring.iter().filter_map(move |&(x, z)| {
        let key = TileKey { x, z, level: 0 };
        (plan.nodes.contains(&key) && masks.get(&key) != Some(&[u64::MAX; 4])).then_some(key)
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesher::TerrainVertex;
    #[test]
    fn retiring_fine_stand_ins_request_detail_before_their_gpu_meshes_can_retire() {
        let column = (12, 0);
        let fine = TileKey {
            x: 12,
            z: 0,
            level: 0,
        };
        let mut plan = Plan::default();
        plan.nodes.insert(fine);
        let ready: FxHashMap<TileKey, ()> = [(fine, ())].into_iter().collect();
        let retiring = [column].into_iter().collect();
        let saved = Saved::default();
        let mut masks = FxHashMap::default();
        // A covered stand-in is ready, but empty: ordinary sampling skips it,
        // and selection leaves it out of `active`. It cannot yet replace the
        // old full-detail column, even though the near world has unloaded it.
        assert!(!plan.replaces(column, &ready, |key| masks.get(&key)
            == Some(&[u64::MAX; 4])));
        assert_eq!(
            retiring_detail(&plan, &retiring, &saved, &masks).collect::<Vec<_>>(),
            [fine]
        );
        // The existing RequestDetail -> apply_fine path marks complete detail;
        // only then may selection retire it and publish the new mesh cutout.
        masks.insert(fine, [u64::MAX; 4]);
        assert!(
            retiring_detail(&plan, &retiring, &saved, &masks)
                .next()
                .is_none()
        );
        assert!(plan.replaces(column, &ready, |key| masks.get(&key)
            == Some(&[u64::MAX; 4])));
    }
    #[test]
    fn retiring_detail_requests_only_planned_columns_still_missing_detail() {
        let fine = |x| TileKey { x, z: 0, level: 0 };
        let mut plan = Plan::default();
        plan.nodes.extend((0..4).map(fine));
        plan.nodes.insert(fine(5));
        let retiring = (0..5).map(|x| (x, 0)).collect();
        let saved = [((1, 0), Arc::new(std::array::from_fn(|_| Column::default())))]
            .into_iter()
            .collect();
        let masks = [
            (fine(2), [u64::MAX; 4]),
            (fine(3), [u64::MAX, u64::MAX, u64::MAX, u64::MAX - 1]),
        ]
        .into_iter()
        .collect();
        let mut requested: Vec<_> = retiring_detail(&plan, &retiring, &saved, &masks).collect();
        requested.sort_unstable();
        // Complete detail needs no request. A saved summary without a
        // completed mask still does: it may predate the tile's stand-in.
        // An unplanned retiree and a planned column that is not retiring
        // must not consume the budget.
        assert_eq!(requested, [fine(0), fine(1), fine(3)]);
    }
    #[test]
    fn reduced_far_detail_only_merges_tiles_away_from_the_near_terrain() {
        let leaves = |plan: &Plan| {
            plan.nodes
                .iter()
                .filter(|k| !plan.split.contains(k))
                .count()
        };
        for camera in [[0, 0], [100, -40]] {
            for distance in [64, 128, 512, 1024] {
                let full = Plan::new(camera, 12, distance);
                let reduced = Plan::with_previous(camera, 12, distance, &Plan::default(), 0, true);
                // Never finer than Full anywhere, and as fine next to the
                // full-detail area, whose edge the finest tiles match.
                assert!(reduced.split.is_subset(&full.split));
                assert_eq!(reduced.roots, full.roots);
                for key in reduced.nodes.iter().filter(|k| !reduced.split.contains(k)) {
                    if key.distance(camera) < 12 * 16 + 16 {
                        assert_eq!(key.level, 0, "{key:?}");
                    }
                }
                let (full, reduced) = (leaves(&full), leaves(&reduced));
                // At 64 chunks most tiles are the near area's finest, the
                // same in both: about half there, well under from 128.
                let bound = if distance == 64 { full * 21 / 20 } else { full };
                assert!(
                    reduced * 2 < bound,
                    "{reduced} of {full} tiles at {distance}"
                );
            }
        }
    }
    #[test]
    fn far_plan_is_bounded_and_near_tiles_align_with_chunks() {
        for camera in [[0, 0], [-1, -17], [1023, -4097]] {
            for distance in [64, 256, 1024] {
                let plan = Plan::new(camera, 12, distance);
                assert!(plan.nodes.len() < 12000, "{} nodes", plan.nodes.len());
                assert!(plan.roots.iter().all(|key| key.level <= MAX_LEVEL));
                for key in &plan.nodes {
                    if !plan.split.contains(key) && key.distance(camera) < 12 * 16 {
                        assert_eq!(key.level, 0);
                    }
                }
                let cells = plan
                    .nodes
                    .iter()
                    .filter(|k| !plan.split.contains(k))
                    .count();
                assert!(cells < 10000);
            }
        }
        assert_eq!(
            TileKey {
                x: -1,
                z: -2,
                level: 1
            }
            .origin(),
            [-32, 0, -64]
        );
    }
    #[test]
    fn world_wide_plans_stay_small_and_cover_every_level() {
        // About 10 000 nodes at a corner of the world, 9 000 at its centre.
        const WORLD_PLAN_NODES: usize = 16_384;
        for (camera, elevation) in [
            ([0, 0], 0),
            ([115, 135], 49_000_000),
            ([-30_000_000, 29_999_000], 90),
        ] {
            let plan =
                Plan::with_previous(camera, 12, MAX_CHUNKS, &Plan::default(), elevation, false);
            let reduced =
                Plan::with_previous(camera, 12, MAX_CHUNKS, &Plan::default(), elevation, true);
            let top = plan.nodes.iter().map(|k| k.level).max().unwrap();
            assert_eq!(top, MAX_LEVEL, "{camera:?} y={elevation}: roots stop short");
            assert!(
                plan.nodes.len() < WORLD_PLAN_NODES,
                "{camera:?} y={elevation}: {} nodes",
                plan.nodes.len()
            );
            assert!(
                reduced.nodes.len() <= plan.nodes.len(),
                "{camera:?} y={elevation}: reduced has {} nodes, full {}",
                reduced.nodes.len(),
                plan.nodes.len()
            );
            // Every level from the finest planned one up to the roots has tiles.
            let finest = plan.nodes.iter().map(|k| k.level).min().unwrap();
            for level in finest..=MAX_LEVEL {
                assert!(
                    plan.nodes.iter().any(|k| k.level == level),
                    "{camera:?} y={elevation}: no level-{level} tile"
                );
            }
        }
    }
    #[test]
    fn aerial_distance_avoids_unseen_full_detail_outside_the_near_area() {
        let ground = Plan::new([0, 0], 12, 1024);
        let high = Plan::with_previous([0, 0], 12, 1024, &Plan::default(), 1024, false);
        assert!(high.nodes.len() < ground.nodes.len());
        assert!(
            high.nodes.contains(&TileKey {
                x: 0,
                z: 0,
                level: 0
            }),
            "near full-detail footprint stays exact"
        );
        assert!(
            !high.nodes.contains(&TileKey {
                x: 20,
                z: 0,
                level: 0
            }),
            "far terrain viewed from high above needs distance-appropriate detail"
        );
    }
    #[test]
    fn late_parent_keeps_ready_detail_and_only_fills_missing_children() {
        let parent = TileKey {
            x: -1,
            z: 0,
            level: 1,
        };
        let mut plan = Plan::default();
        plan.roots.push(parent);
        plan.nodes.insert(parent);
        plan.split.insert(parent);
        plan.nodes.extend(parent.children());
        let mut ready: FxHashMap<TileKey, Vec<Column>> = FxHashMap::default();
        ready.insert(parent, vec![]);
        ready.insert(parent.children()[0], vec![]);
        let mut selected = Vec::new();
        ready.remove(&parent);
        assert!(!plan.select(parent, &ready, &mut selected));
        assert_eq!(selected, [parent.children()[0]]);
        ready.insert(parent, vec![]);
        selected.clear();
        assert!(plan.select(parent, &ready, &mut selected));
        assert_eq!(selected, [parent.children()[0], parent]);
        for child in parent.children() {
            ready.insert(child, vec![]);
        }
        selected.clear();
        assert!(plan.select(parent, &ready, &mut selected));
        assert_eq!(selected, parent.children());
    }
    #[test]
    fn a_shrunk_render_distance_keeps_columns_until_their_planned_tiles_are_ready() {
        // Render distance 64 under LOD 64: every tile inside it is a level-0
        // stand-in for a covered column, and only the roots are sampled.
        let camera = [8, 8];
        let before = Plan::new(camera, 64, 64);
        let after = Plan::with_previous(camera, 8, 64, &before, 0, false);
        let mut ready: FxHashMap<TileKey, ()> = before
            .nodes
            .iter()
            .filter(|key| key.level == 0 || before.roots.contains(key))
            .map(|&key| (key, ()))
            .collect();
        ready.retain(|key, _| after.nodes.contains(key));
        let contains = |key: &TileKey, (x, z): (i32, i32)| {
            let [ox, _, oz] = key.origin();
            (ox..ox + key.size()).contains(&(x * 16)) && (oz..oz + key.size()).contains(&(z * 16))
        };
        // 24 chunks out, now past the render distance: its planned tile was
        // split before, so never sampled. The ready root reaches the seam, so
        // it stands in for nothing: the column stays until its tile is in.
        let column = (24, 0);
        let mut selected = Vec::new();
        for &root in &after.roots {
            after.select(root, &ready, &mut selected);
        }
        assert!(!selected.iter().any(|key| contains(key, column)));
        assert!(!after.replaces(column, &ready, |_| true));
        let planned = TileKey {
            x: 12,
            z: 0,
            level: 1,
        };
        assert!(after.nodes.contains(&planned) && before.split.contains(&planned));
        ready.insert(planned, ());
        assert!(after.replaces(column, &ready, |_| true));
        // 12 chunks out, its level-0 tile is the stand-in until filled.
        assert!(!after.replaces((12, 0), &ready, |_| false));
        assert!(after.replaces((12, 0), &ready, |_| true));
        // Past the far terrain nothing waits.
        assert!(after.replaces((100, 0), &ready, |_| false));
    }
    #[test]
    fn detail_survives_movement_across_a_split_threshold() {
        let before = Plan::new([0, 0], 2, 256);
        let after = Plan::with_previous([16, 0], 2, 256, &before, 0, false);
        let mut retained = 0;
        let range = i64::from(256 * 16);
        for key in &before.split {
            if key.distance([16, 0]) < key.size() * 10 && key.reach([16, 0]) <= range * range {
                assert!(after.split.contains(key));
                retained += 1;
            }
        }
        assert!(retained > 100);
        assert!(after.nodes.len() < 16000);
    }
    #[test]
    fn reduced_material_votes_ignore_light_and_retain_exposure() {
        let mut columns: [Column; 256] = std::array::from_fn(|_| Column::default());
        for i in 0..16 {
            columns[(i / 4) * 16 + i % 4].spans.push(Span {
                min: 60,
                max: 64,
                state: BlockStateId(if i < 10 { 2 } else { 1 }),
                biome: 0,
                light: if i < 10 { (i as u16 + 6) << 4 } else { 0 },
            });
        }
        let coarse = Column::reduced(&columns, 0, 0, 4);
        assert_eq!(coarse.spans.len(), 1);
        assert_eq!(
            coarse.spans[0].state,
            BlockStateId(2),
            "majority grass must beat dark dirt"
        );
        assert_eq!(coarse.spans[0].light, 240, "retain sampled sky exposure");
        for column in &mut columns {
            for span in &mut column.spans {
                span.light = 0;
            }
        }
        assert_eq!(
            Column::reduced(&columns, 0, 0, 4).spans[0].light,
            0,
            "unlit caves stay unlit"
        );
        columns[0].spans[0].light = 7 << 12;
        assert_eq!(
            Column::reduced(&columns, 0, 0, 4).spans[0].light,
            7 << 12,
            "block light does not invent skylight"
        );
    }
    #[test]
    fn open_water_and_its_floor_are_lit_as_the_seed_lights_them() {
        let span = |min, max, state, light| Span {
            min,
            max,
            state: BlockStateId(state),
            biome: 0,
            light,
        };
        // 0 air, 1 stone, 2 water, 3 lava.
        let kinds = [AIR, KEPT, KEPT | FLUID, KEPT | FLUID | 15 << 4];
        // A chunk's own light: a cave, the floor, water fading a level a block.
        let mut sea = Column {
            surface: None,
            spans: vec![
                span(30, 40, 1, 0),
                span(40, 48, 1, 0x10 | 2 << 12),
                span(48, 61, 2, 0x20),
                span(61, 62, 2, 0xe0),
                span(62, 63, 2, 0xf0),
            ]
            .into(),
        };
        sea.light_open_water(&kinds);
        assert_eq!(
            sea.spans,
            [
                span(30, 40, 1, 0),
                span(40, 48, 1, 0xf0 | 2 << 12),
                span(48, 63, 2, 0xf0)
            ]
        );
        // Land, and lava, keep their light.
        for top in [1, 3] {
            let mut column = Column {
                surface: None,
                spans: vec![span(40, 50, 2, 0x20), span(50, 63, top, 0x80)].into(),
            };
            let before = column.spans.clone();
            column.light_open_water(&kinds);
            assert_eq!(column.spans, before);
        }
    }
    #[test]
    fn nearly_full_blocks_are_kept() {
        assert!(nearly_full("minecraft:powder_snow"));
        assert!(nearly_full("minecraft:dirt_path"));
        assert!(nearly_full("minecraft:oak_stairs"));
        assert!(!nearly_full("minecraft:oak_fence"));
        assert!(!nearly_full("minecraft:wheat"));
    }
    #[test]
    fn kelp_to_the_surface_is_drawn_as_water() {
        // 0 air, 1 stone, 2 water, 3 kelp (holds water, draws as none).
        let kinds = [AIR, KEPT, KEPT | FLUID, KEPT | FLUID | WATERY];
        let mut chunk = Chunk::empty(0, 0);
        for x in [3, 4] {
            for y in MIN_Y..40 {
                chunk.set(x, y, 3, 1);
            }
        }
        for y in 40..63 {
            chunk.set(3, y, 3, 3);
            chunk.set(4, y, 3, 2);
        }
        let water = water_source(&kinds);
        assert_eq!(water, Some(BlockStateId(2)));
        let kelp = Column::from_chunk(&chunk, &kinds, water, 3, 3);
        let sea = Column::from_chunk(&chunk, &kinds, water, 4, 3);
        assert_eq!(kelp.spans, sea.spans);
        assert_eq!(
            kelp.spans.last().map(|s| (s.min, s.max, s.state)),
            Some((40, 63, BlockStateId(2)))
        );
        assert_eq!(kelp.surface.unwrap().water, Some((63, BlockStateId(2))));
        assert_eq!(kelp.surface.unwrap().height, 40);
    }
    #[test]
    fn coarse_voxels_reduce_the_area_instead_of_inflating_one_leaf() {
        let span = |min, max, state| Span {
            min,
            max,
            state: BlockStateId(state),
            biome: 0,
            light: 240,
        };
        let mut columns = std::array::from_fn(|_| Column {
            surface: None,
            spans: vec![span(0, 4, 1)].into(),
        });
        columns[0].spans.push(span(8, 10, 2));
        assert_eq!(Column::reduced(&columns, 0, 0, 4).spans, [span(0, 4, 1)]);
        for z in 0..4 {
            for x in 0..4 {
                columns[z * 16 + x].spans.push(span(12, 14, 3));
            }
        }
        assert_eq!(
            Column::reduced(&columns, 0, 0, 4).spans,
            [span(0, 4, 1), span(12, 14, 3)]
        );
        assert_eq!(Column::reduced(&columns, 0, 0, 1), columns[0]);
        let reduced = ReducedColumns::new(Arc::new(columns));
        assert_eq!(
            reduced.get(0, 0, 1).spans,
            [span(0, 4, 1), span(8, 10, 2), span(12, 14, 3)]
        );
        assert_eq!(reduced.get(0, 0, 4).spans, [span(0, 4, 1), span(12, 14, 3)]);
        assert_eq!(reduced.get(4, 8, 4).spans, [span(0, 4, 1)]);
        assert_eq!(reduced.get(0, 0, 16).spans, [span(0, 4, 1)]);
    }
    /// Detail read before a refresh over its chunk is dropped (asked
    /// again); elsewhere, or read after, it is kept.
    #[test]
    fn only_detail_a_later_refresh_covers_is_dropped() {
        let mut lod = Lod::scheduler_for_test();
        let asked = lod.refreshes;
        lod.refresh([0, 0, 64, 64], (1, 16));
        assert!(lod.refreshed_since((1, 1), asked));
        assert!(lod.refreshed_since((4, 0), asked), "a chunk around");
        assert!(!lod.refreshed_since((40, 40), asked), "elsewhere");
        assert!(!lod.refreshed_since((1, 1), lod.refreshes), "read after");
        for _ in 0..MAX_REFRESHED {
            lod.refresh([10_000, 10_000, 10_064, 10_064], (1, 16));
        }
        assert!(lod.refreshed_since((40, 40), asked), "too old to tell");
    }
    #[test]
    fn refreshed_areas_forget_detail_but_seen_chunks() {
        let key = TileKey {
            x: 1,
            z: 0,
            level: 1,
        };
        assert!(key.overlaps([40, 0, 50, 1]) && !key.overlaps([64, 0, 80, 32]));
        // Only cells of the widths the new data is drawn in, a cell around.
        assert!(key.refreshed_by([64, 0, 80, 32], (1, 16)));
        assert!(!key.refreshed_by([65, 0, 80, 32], (1, 16)));
        assert!(!key.refreshed_by([40, 0, 50, 1], (25, i32::MAX)));
        let root = TileKey {
            x: 0,
            z: 0,
            level: MAX_LEVEL,
        };
        assert!(!root.refreshed_by([40, 0, 72, 32], (1, 16)));
        assert!(root.refreshed_by([0, 0, 800, 800], (25, i32::MAX)));
        // Level 1: 2 × 2 cells a chunk, chunks 2..4 × 0..2.
        let mut mask = [u64::MAX; 4];
        forget_detail(key, [48, 0, 64, 16], &mut mask, |_| false);
        let cleared: Vec<_> = (0..256)
            .filter(|i| mask[i / 64] & (1 << (i % 64)) == 0)
            .collect();
        assert_eq!(cleared.len(), 64);
        assert!(cleared.contains(&8) && cleared.contains(&(7 * 16 + 15)));
        let mut mask = [u64::MAX; 4];
        forget_detail(key, [0, 0, 1024, 1024], &mut mask, |chunk| chunk == (2, 1));
        assert_eq!(mask.iter().map(|m| m.count_ones()).sum::<u32>(), 64);
    }
    #[test]
    fn tiles_sampled_without_a_kept_chunk_are_sampled_again() {
        let tile = |level: u8| {
            let size = 16 << level;
            TileKey {
                x: (3 * 16i32).div_euclid(size),
                z: (-5 * 16i32).div_euclid(size),
                level,
            }
        };
        let empty = || Arc::new(vec![Column::default(); CELLS * CELLS]);
        let filled = || {
            let column = Column {
                surface: None,
                spans: vec![Span {
                    min: -64,
                    max: 70,
                    state: BlockStateId(1),
                    biome: 0,
                    light: 0,
                }]
                .into(),
            };
            Arc::new(vec![column; CELLS * CELLS])
        };
        let mut ready = FxHashMap::default();
        let mut pending = FxHashSet::default();
        // Cells of a chunk or less take its detail; wider ones sampled empty
        // there are sampled again, and filled ones aren't.
        ready.insert(tile(2), empty());
        ready.insert(tile(5), empty());
        ready.insert(tile(6), filled());
        assert_eq!(resampled_over(&ready, &pending, (3, -5)), [tile(5)]);
        // Samples in flight may predate it: those that detail can't patch.
        pending.extend([tile(1), tile(2), tile(6), tile(7)]);
        assert_eq!(
            resampled_over(&ready, &pending, (3, -5)),
            [tile(1), tile(5), tile(6), tile(7)]
        );
        assert!(resampled_over(&ready, &FxHashSet::default(), (40, 40)).is_empty());
    }
    #[test]
    #[ignore = "requires prepared models/textures in HORIZON_TEST_RESOURCES"]
    fn real_mesh_clips_near_chunks_and_preserves_water_and_edited_surfaces() -> crate::Result<()> {
        use crate::{
            assets::Assets, biome::Biomes, light::LightTables, mesher::MeshTables,
            testgen::TestTerrain,
        };
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES"),
        );
        let assets = Arc::new(Assets::load(&root)?);
        let render = Arc::new(assets.reload(&root)?);
        let facts = BlockFacts::load(&root, assets.registry)?;
        let biomes = Arc::new(Biomes::load(&root)?);
        let tables = Arc::new(MeshTables::new(
            &assets,
            &render,
            &facts,
            crate::models::load_visibility(&root, assets.registry)?,
        )?);
        let context = Arc::new(WorkerContext {
            light: Arc::new(LightTables::load(&root, assets.registry, &facts)?),
            tables,
            assets: assets.clone(),
            render,
            biomes: biomes.clone(),
        });
        let source = Arc::new(TestTerrain::new(assets.registry, &biomes)?);
        let mut lod = Lod::new(source, context);
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let key = TileKey {
            x: -1,
            z: -1,
            level: 5,
        };
        lod.ready.insert(
            key,
            Arc::new(vec![
                Column::from_surface(Some(Surface {
                    height: 70,
                    state: stone,
                    water: Some((80, water)),
                    biome: biomes.id("minecraft:plains"),
                }));
                CELLS * CELLS
            ]),
        );
        // A chunk covers only one quarter of this parent's 32² cell. Both
        // opaque terrain and water must leave precisely that 16² footprint.
        lod.covered.insert((-1, -2));
        lod.covered_bounds = Some([-16, -32, 0, -16]);
        let mesh = lod.mesh(key);
        let top_area = |vertices: &[TerrainVertex], y: f32| {
            vertices
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|quad| quad.iter().all(|v| (v.pos[1] - y).abs() < 0.001))
                .map(|quad| {
                    let a = glam::Vec3::from(quad[0].pos);
                    let b = glam::Vec3::from(quad[1].pos);
                    let c = glam::Vec3::from(quad[2].pos);
                    let normal = (b - a).cross(c - a);
                    assert!(normal.y > 0.0, "surface must face the camera from above");
                    let center = (a + c) * 0.5;
                    let [ox, _, oz] = key.origin();
                    let chunk = (
                        ((center.x as i32) + ox).div_euclid(16),
                        ((center.z as i32) + oz).div_euclid(16),
                    );
                    assert!(
                        !lod.covered.contains(&chunk),
                        "LOD overlaps a full-detail chunk"
                    );
                    normal.y
                })
                .sum::<f32>()
        };
        let expected = (key.size() * key.size() - 16 * 16) as f32;
        assert_eq!(top_area(&mesh.solid, 70.0), expected);
        assert_eq!(top_area(&mesh.translucent, 80.0 - 1.0 / 9.0), expected);
        assert!(mesh.translucent.iter().all(|v| v.light == 240));
        let side = |axis: usize, plane: f32, low: f32, high: f32| {
            let other = if axis == 0 { 2 } else { 0 };
            mesh.solid
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|quad| {
                    quad.iter().all(|v| v.pos[axis] == plane)
                        && quad.iter().map(|v| v.pos[1]).fold(f32::INFINITY, f32::min)
                            == MIN_Y as f32
                        && quad
                            .iter()
                            .map(|v| v.pos[1])
                            .fold(f32::NEG_INFINITY, f32::max)
                            == 70.0
                        && quad
                            .iter()
                            .map(|v| v.pos[other])
                            .fold(f32::INFINITY, f32::min)
                            == low
                        && quad
                            .iter()
                            .map(|v| v.pos[other])
                            .fold(f32::NEG_INFINITY, f32::max)
                            == high
                })
                .map(move |quad| {
                    let a = glam::Vec3::from(quad[0].pos);
                    (glam::Vec3::from(quad[1].pos) - a).cross(glam::Vec3::from(quad[2].pos) - a)
                        [axis]
                })
                .collect::<Vec<_>>()
        };
        // The two exposed edges face the missing full-detail chunk and seal
        // the height mismatch all the way down. Shared subpieces add no walls.
        let east_cut = side(0, 496.0, 480.0, 496.0);
        assert_eq!(east_cut.len(), 1);
        assert!(east_cut[0] > 0.0);
        let north_cut = side(2, 496.0, 496.0, 512.0);
        assert_eq!(north_cut.len(), 1);
        assert!(north_cut[0] < 0.0);
        assert!(side(0, 496.0, 496.0, 512.0).is_empty());

        // Block edits do not refresh Chunk.heights; observation must scan the
        // actual blocks, including builds above the old heightmap. The worker
        // summarizes the chunk; its result is kept on receipt.
        let receive_observed = |lod: &mut Lod| {
            let Done::Observed(key, columns) = lod
                .results
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
            else {
                panic!("expected the observed chunk's summary");
            };
            lod.finish_observed(key, columns);
        };
        let mut chunk = Chunk::empty(-1, -1);
        chunk.set(0, 120, 0, stone.0);
        chunk.set(4, 54, 4, stone.0);
        for y in 55..64 {
            chunk.set(4, y, 4, water.0);
        }
        // Chorus is thin and non-solid but must survive the visit summary.
        let end_stone = assets
            .registry
            .default_state("minecraft:end_stone")
            .unwrap();
        let chorus = assets
            .registry
            .default_state("minecraft:chorus_plant")
            .unwrap();
        let flower = assets
            .registry
            .default_state("minecraft:chorus_flower")
            .unwrap();
        chunk.set(8, 70, 8, end_stone.0);
        chunk.set(8, 71, 8, chorus.0);
        chunk.set(8, 72, 8, chorus.0);
        chunk.set(8, 73, 8, flower.0);
        // Kelp to the surface is water; powder snow and a waterlogged stair
        // are kept as themselves.
        let state = |name: &str| assets.registry.default_state(name).unwrap();
        let (kelp, powder_snow) = (state("minecraft:kelp"), state("minecraft:powder_snow"));
        let stair = facts
            .states
            .iter()
            .position(|f| &*f.name == "minecraft:oak_stairs" && f.waterlogged)
            .map(|i| BlockStateId(i as u32))
            .unwrap();
        chunk.set(2, 54, 2, stone.0);
        for y in 55..64 {
            chunk.set(2, y, 2, kelp.0);
        }
        chunk.set(6, 90, 6, stone.0);
        chunk.set(6, 91, 6, powder_snow.0);
        chunk.set(10, 54, 10, stone.0);
        chunk.set(10, 55, 10, stair.0);
        lod.observe(Arc::new(chunk.clone()), &facts);
        receive_observed(&mut lod);
        let observed = &lod.saved[&(-1, -1)];
        assert_eq!(observed[0].surface.unwrap().height, 121);
        assert_eq!(observed[0].surface.unwrap().state, stone);
        assert_eq!(observed[68].surface.unwrap().height, 55);
        assert_eq!(observed[68].surface.unwrap().water, Some((64, water)));
        let chorus_column = &observed[8 * 16 + 8];
        assert_eq!(chorus_column.surface.unwrap().state, flower);
        assert_eq!(
            chorus_column
                .spans
                .iter()
                .map(|span| (span.min, span.max, span.state))
                .collect::<Vec<_>>(),
            vec![(70, 71, end_stone), (71, 73, chorus), (73, 74, flower)]
        );
        let kelp_column = &observed[2 * 16 + 2];
        assert_eq!(kelp_column.surface.unwrap().height, 55);
        assert_eq!(kelp_column.surface.unwrap().water, Some((64, water)));
        assert_eq!(observed[6 * 16 + 6].surface.unwrap().state, powder_snow);
        assert_eq!(observed[6 * 16 + 6].surface.unwrap().height, 92);
        assert_eq!(observed[10 * 16 + 10].surface.unwrap().state, stair);

        let fine = TileKey {
            x: -1,
            z: -1,
            level: 0,
        };
        lod.ready
            .insert(fine, Arc::new(vec![Column::default(); 256]));
        lod.finish_detail(
            (-1, -1),
            ReducedColumns::new(Arc::new(std::array::from_fn(|_| Column::default()))),
        );
        assert_eq!(
            lod.ready[&fine][0].surface.unwrap().height,
            121,
            "late seed generation cannot overwrite a visited build"
        );
        assert_eq!(lod.detail_masks[&fine], [u64::MAX; 4]);
        lod.plan.nodes.insert(fine);
        lod.pending.insert(fine);
        lod.finish_sample(fine, vec![Column::default(); 256]);
        assert_eq!(
            lod.ready[&fine][0].surface.unwrap().height,
            121,
            "a late coarse sampling job cannot erase completed detail"
        );
        assert_eq!(lod.detail_masks[&fine], [u64::MAX; 4]);
        lod.plan.nodes.remove(&fine);

        // The worker's summary comes with the reusable pyramid; a newer
        // observation replaces both, and an unchanged one changes nothing.
        assert!(Arc::ptr_eq(
            &lod.reduced[&(-1, -1)].columns,
            &lod.saved[&(-1, -1)]
        ));
        assert!(
            lod.snapshot()
                .visited_source(-16, -16, 2)
                .unwrap()
                .get()
                .spans
                .is_empty()
        );
        let old = lod.saved[&(-1, -1)].clone();
        lod.observe(Arc::new(chunk.clone()), &facts);
        receive_observed(&mut lod);
        assert!(Arc::ptr_eq(&lod.saved[&(-1, -1)], &old));
        chunk.set(0, 130, 0, stone.0);
        lod.observe(Arc::new(chunk.clone()), &facts);
        receive_observed(&mut lod);
        assert_eq!(lod.ready[&fine][0].surface.unwrap().height, 131);
        assert!(Arc::ptr_eq(
            &lod.reduced[&(-1, -1)].columns,
            &lod.saved[&(-1, -1)]
        ));
        // A seed reduction arriving after the edit cannot restore old data.
        lod.finish_detail((-1, -1), ReducedColumns::new(old));
        assert_eq!(lod.ready[&fine][0].surface.unwrap().height, 131);

        // Exercise the actual worker/update seam: a parent stays displayed
        // until all four child meshes can replace it in the same update.
        let camera = [-256.0, 100.0, -256.0];
        lod.area = Some(([-16, -16], 2, 64, 0, false));
        lod.plan.roots = vec![key];
        lod.plan.nodes.insert(key);
        // As a replan does (`update` only selects after a change).
        lod.reselect = true;
        let await_meshes = |lod: &mut Lod| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let meshes = lod.update(camera, 2, 64, None);
                if !meshes.is_empty() {
                    return meshes;
                }
                assert!(std::time::Instant::now() < deadline, "mesh worker stalled");
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        };
        let uploaded = await_meshes(&mut lod);
        assert_eq!(uploaded.len(), 1);
        assert_eq!(uploaded[0].0, key);
        assert!(lod.active.contains(&key));
        lod.plan.split.insert(key);
        for child in key.children() {
            lod.plan.nodes.insert(child);
            lod.ready.insert(child, lod.ready[&key].clone());
        }
        lod.reselect = true;
        assert!(lod.update(camera, 2, 64, None).is_empty());
        assert!(
            lod.active.contains(&key),
            "parent retired before child meshes"
        );
        assert!(lod.removed().is_empty());
        let uploaded = await_meshes(&mut lod);
        assert_eq!(uploaded.len(), 4);
        assert!(
            key.children()
                .iter()
                .all(|child| lod.active.contains(child))
        );
        assert_eq!(lod.removed(), [key]);

        let covered_fine = TileKey {
            x: -1,
            z: -2,
            level: 0,
        };
        lod.plan = Plan::default();
        lod.plan.roots.push(covered_fine);
        lod.plan.nodes.insert(covered_fine);
        lod.ready
            .insert(covered_fine, Arc::new(vec![Column::default(); 256]));
        lod.covered.insert((-1, -2));
        lod.reselect = true;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            lod.update(camera, 2, 64, None);
            if lod.outstanding_meshes == 0 && lod.active.is_empty() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "covered normal chunk remained an active LOD mask"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(lod.progress(), (1, 1));

        // Keep one sample outstanding, as on ordinary frames. Completing just
        // one job at a time must still let both loading queues make progress.
        lod.shared.stop.store(true, Ordering::Relaxed);
        lod.shared.wake.notify_all();
        lod.thread.take().unwrap().join().unwrap();
        while lod.results.try_recv().is_ok() {}
        lod.shared.jobs.lock().unwrap().clear();
        lod.plan = Plan::new([-256, -256], 2, 64);
        let parent = TileKey {
            x: -2,
            z: -2,
            level: 3,
        };
        lod.plan.roots = vec![parent];
        lod.ready.clear();
        lod.ready
            .insert(parent, Arc::new(vec![Column::default(); 256]));
        lod.active = [parent].into_iter().collect();
        lod.sample_jobs = SAMPLE_JOBS;
        lod.pending = (0..SAMPLE_JOBS - 1)
            .map(|i| TileKey {
                x: 999 + i as i32,
                z: 999,
                level: 0,
            })
            .collect();
        lod.detail_request_pending = true;
        lod.wanted_stale = true;
        lod.update(camera, 2, 64, Some(FxHashSet::default()));
        let Some(Job::Sample(nearby)) = lod.shared.jobs.lock().unwrap().pop_front() else {
            panic!("nearby detail was not scheduled");
        };
        assert_eq!(nearby.level, 0);
        assert_eq!(
            nearby.distance([-256, -256]),
            0,
            "the nearest tiles load first"
        );
        lod.finish_sample(nearby, vec![Column::default(); 256]);
        lod.update(camera, 2, 64, Some(FxHashSet::default()));
        let Some(Job::Sample(next)) = lod.shared.jobs.lock().unwrap().pop_front() else {
            panic!("the next ring was not scheduled");
        };
        assert!(
            !lod.plan.split.contains(&next) && next.distance([-256, -256]) <= 64,
            "the next nearest final tile, never a coarse one under it: {next:?}"
        );
        lod.shared.jobs.lock().unwrap().clear();
        let near = TileKey {
            x: -4,
            z: -4,
            level: 2,
        };
        let far = TileKey {
            x: 0,
            z: 0,
            level: 6,
        };
        lod.plan = Plan::default();
        lod.plan.roots = vec![near, far];
        lod.ready.clear();
        for parent in [near, far] {
            lod.plan.nodes.insert(parent);
            lod.plan.split.insert(parent);
            lod.plan.nodes.extend(parent.children());
            lod.ready
                .insert(parent, Arc::new(vec![Column::default(); 256]));
        }
        lod.active = [near, far].into_iter().collect();
        lod.shared.jobs.lock().unwrap().clear();
        lod.sample_jobs = SAMPLE_JOBS;
        lod.pending = (0..SAMPLE_JOBS - 1)
            .map(|i| TileKey {
                x: 999 + i as i32,
                z: 999,
                level: 0,
            })
            .collect();
        lod.update(camera, 2, 64, Some(FxHashSet::default()));
        let Some(Job::Sample(first)) = lod.shared.jobs.lock().unwrap().pop_front() else {
            panic!("no final tile scheduled")
        };
        assert_eq!(
            (first.level, first.parent()),
            (1, near),
            "the nearest parent's final tiles come first"
        );
        lod.shared.jobs.lock().unwrap().clear();
        lod.active = (0..DETAIL_REQUEST as i32 + 8)
            .map(|x| TileKey { x, z: 0, level: 0 })
            .collect();
        lod.plan.split.clear();
        lod.detail_masks.clear();
        lod.covered = [(0, 0)].into_iter().collect();
        lod.detail_request_pending = false;
        lod.detail_request_at = std::time::Instant::now() - std::time::Duration::from_secs(1);
        lod.request_detail([0, 0]);
        let Some(Job::RequestDetail { chunks, .. }) = lod.shared.jobs.lock().unwrap().pop_front()
        else {
            panic!("detail batch was not requested");
        };
        assert_eq!(
            chunks,
            (1..=DETAIL_REQUEST as i32)
                .map(|x| (x, 0))
                .collect::<Vec<_>>(),
            "one unfinished fine tile must not starve neighbouring requests"
        );

        // The extra sampling threads' slots open once the near terrain is in.
        lod.small = false;
        lod.sample_jobs = SAMPLE_JOBS + 3;
        lod.plan = Plan::new([-256, -256], 2, 64);
        lod.ready.clear();
        lod.active.clear();
        lod.pending.clear();
        lod.shared.jobs.lock().unwrap().clear();
        let queued = |lod: &mut Lod, near_loading: bool| {
            lod.set_near_loading(near_loading);
            lod.shared.jobs.lock().unwrap().clear();
            lod.pending.clear();
            lod.wanted_stale = true;
            lod.update([-256., 64., -256.], 2, 64, Some(FxHashSet::default()));
            let jobs = lod.shared.jobs.lock().unwrap();
            jobs.iter()
                .filter(|job| matches!(job, Job::Sample(_)))
                .count()
        };
        assert_eq!(queued(&mut lod, true), SAMPLE_JOBS);
        assert_eq!(queued(&mut lod, false), SAMPLE_JOBS + 3);
        Ok(())
    }

    #[test]
    fn far_cells_past_render_distance_12_start_one_block_wide() {
        // Leaves' cells: 1 block to 256 blocks out, 2 to 512 (Full); Reduced
        // keeps 1 to the near terrain's edge and 2 to 256.
        for (reduced, [one, two]) in [(false, [256, 512]), (true, [208, 256])] {
            let plan = Plan::with_previous([8, 8], 12, 128, &Plan::default(), 0, reduced);
            for key in plan.nodes.iter().filter(|key| !plan.split.contains(key)) {
                let distance = key.distance([8, 8]);
                if distance < one {
                    assert_eq!(key.size() / 16, 1, "{key:?} reduced {reduced}");
                } else if distance < two {
                    assert!(key.size() / 16 <= 2, "{key:?} reduced {reduced}");
                }
            }
        }
    }
    /// Loads a render distance 12 view one sample job at a time, in
    /// `wanted_tiles`' order. Counts the jobs, and their density lattice
    /// points (each job's cost), before every far column within two chunks
    /// of the near terrain is drawn as finely as the plan wants it (one
    /// block wide in Full): until then nothing is drawn there. And before
    /// every far column is drawn, by its tile or a coarse stand-in: until
    /// then the far terrain ends short of the horizon. Checks that no coarse
    /// tile is ever drawn at the seam.
    fn jobs_until_fine_seam(camera: [i32; 2], reduced: bool) -> ([(usize, i32); 2], usize) {
        let (detail, distance) = (12, 128);
        let center = (camera[0].div_euclid(16), camera[1].div_euclid(16));
        let mut covered = FxHashSet::default();
        for z in center.1 - detail..=center.1 + detail {
            for x in center.0 - detail..=center.0 + detail {
                if crate::streaming::in_view_distance(center, detail, x, z) {
                    covered.insert((x, z));
                }
            }
        }
        let plan = Plan::with_previous(camera, detail, distance, &Plan::default(), 0, reduced);
        let leaf = |x: i32, z: i32| {
            (0..=MAX_LEVEL)
                .map(|level| TileKey {
                    x: x.div_euclid(16 << level),
                    z: z.div_euclid(16 << level),
                    level,
                })
                .find(|key| plan.nodes.contains(key) && !plan.split.contains(key))
        };
        let mut seam = Vec::new();
        for z in (camera[1] - 256..camera[1] + 256).step_by(2) {
            for x in (camera[0] - 256..camera[0] + 256).step_by(2) {
                let chunk = (x.div_euclid(16), z.div_euclid(16));
                let near = (-2..=2)
                    .any(|dz| (-2..=2).any(|dx| covered.contains(&(chunk.0 + dx, chunk.1 + dz))));
                if !covered.contains(&chunk) && near {
                    seam.push((x, z, leaf(x, z).unwrap()));
                }
            }
        }
        let mut horizon = Vec::new();
        for z in (camera[1] - 2048..camera[1] + 2048).step_by(64) {
            for x in (camera[0] - 2048..camera[0] + 2048).step_by(64) {
                if !covered.contains(&(x.div_euclid(16), z.div_euclid(16))) && leaf(x, z).is_some()
                {
                    horizon.push((x, z));
                }
            }
        }
        let mut ready: FxHashMap<TileKey, ()> = FxHashMap::default();
        for key in &plan.nodes {
            if key.level == 0 && covered.contains(&(key.x, key.z)) {
                ready.insert(*key, ());
            }
        }
        let leaves = plan
            .nodes
            .iter()
            .filter(|k| !plan.split.contains(k))
            .count();
        let (mut cost, mut fine, mut reached) = (0, None, None);
        for jobs in 0.. {
            let mut selected = Vec::new();
            for &root in &plan.roots {
                plan.select(root, &ready, &mut selected);
            }
            let active: FxHashSet<_> = selected.into_iter().collect();
            let drawn = |x: i32, z: i32, levels: u8| {
                (0..=levels).any(|level| {
                    active.contains(&TileKey {
                        x: x.div_euclid(16 << level),
                        z: z.div_euclid(16 << level),
                        level,
                    })
                })
            };
            for &(x, z, leaf) in &seam {
                assert!(drawn(x, z, leaf.level) || !drawn(x, z, MAX_LEVEL));
            }
            if fine.is_none() && seam.iter().all(|&(x, z, leaf)| drawn(x, z, leaf.level)) {
                fine = Some((jobs, cost));
            }
            if reached.is_none() && horizon.iter().all(|&(x, z)| drawn(x, z, MAX_LEVEL)) {
                reached = Some((jobs, cost));
            }
            if let (Some(fine), Some(reached)) = (fine, reached) {
                return ([fine, reached], leaves);
            }
            let next = wanted_tiles(&plan, &ready, &FxHashSet::default(), camera)
                .min()
                .unwrap();
            let next = TileKey {
                x: next.x,
                z: next.z,
                level: next.level,
            };
            // `lod_grid`'s density lattice: one point per max(stride, 4) blocks.
            let stride = next.size() / 16;
            let lattice = 16 * stride / stride.max(4) + 1;
            cost += lattice * lattice;
            ready.insert(next, ());
        }
        unreachable!()
    }
    #[test]
    fn the_far_terrain_next_to_the_near_chunks_loads_first() {
        // Measured: 270 to 301 jobs, all of them fine tiles (7500 to 8500
        // lattice points). A coarse tile first costs up to 289 points; the
        // order that filled a ring ahead first and refined broad tiles every
        // other job took 54000 to 119000 for the seam in view alone.
        for camera in [[8, 8], [40, -9], [-300, 517]] {
            for reduced in [false, true] {
                let ([(jobs, cost), _], _) = jobs_until_fine_seam(camera, reduced);
                assert!(jobs < 320, "{jobs} jobs");
                assert!(cost < 9000, "{cost} lattice points");
            }
        }
    }
    #[test]
    fn a_coarse_backdrop_reaches_the_horizon_early() {
        // Sampling only planned tiles, nearest first, reached the horizon
        // with the last of ~3000 jobs in Full (500000 lattice points), ~1050
        // in Reduced (200000): for minutes on a slow machine the far terrain
        // ended in a hard edge. The backdrop adds ~230 coarse tiles (~80 in
        // Reduced), about a tenth more work in all.
        for camera in [[8, 8], [40, -9], [-300, 517]] {
            for reduced in [false, true] {
                let ([_, (jobs, cost)], leaves) = jobs_until_fine_seam(camera, reduced);
                assert!(jobs < 1000 && jobs < leaves, "{jobs} jobs of {leaves}");
                assert!(cost < 100_000, "{cost} lattice points");
            }
        }
    }
    #[test]
    fn extra_sampling_threads_only_on_large_machines() {
        assert_eq!([1, 2, 4, 8, 11].map(sampling_threads), [1, 1, 1, 4, 5]);
        assert_eq!([12, 16, 64].map(sampling_threads), [6, 8, 8]);
        assert_eq!([2, 8, 12, 16].map(meshing_threads), [1, 3, 5, 7]);
    }

    /// Review point 3 on #4747: a result from a batch that is no longer the
    /// newest still fills the frame when it is newer than what the tile shows,
    /// and the tile keeps waiting for the job that is still pending. Accepting
    /// only the newest revision threw every older result away — and re-marking
    /// a tile happens again before its mesh arrives on a busy frontier, so no
    /// result was ever accepted and the tile's removal stayed blocked.
    #[test]
    fn a_result_newer_than_the_shown_one_is_kept_while_a_newer_one_is_pending() {
        let mut lod = Lod::scheduler_for_test();
        let key = TileKey {
            x: 0,
            z: 0,
            level: 0,
        };
        lod.active.insert(key);
        // Two marks: the second revision is what the tile is waiting for.
        lod.mark_mesh(key);
        let older = *lod.mesh_version.get(&key).unwrap();
        lod.mark_mesh(key);
        let newest = *lod.mesh_version.get(&key).unwrap();
        assert!(newest > older);
        let mut meshes = Vec::new();

        // The older batch's result arrives while the newer job is pending.
        lod.accept_mesh(key, older, SectionMesh::default(), false, &mut meshes);
        assert_eq!(meshes.len(), 1, "an older result newer than shown is used");
        assert_eq!(lod.shown_version.get(&key), Some(&older));
        assert!(lod.meshed.contains(&key));
        // Still waiting: the pending job owns the tile, so it stays unmeshed
        // and keeps being dispatched.
        assert!(lod.unmeshed.contains(&key), "the pending job still owns it");

        // A result older than what is shown is not drawn again.
        lod.accept_mesh(key, older, SectionMesh::default(), false, &mut meshes);
        assert_eq!(meshes.len(), 1, "the same revision is not redrawn");

        // The newest settles the tile.
        lod.accept_mesh(key, newest, SectionMesh::default(), false, &mut meshes);
        assert_eq!(meshes.len(), 2);
        assert_eq!(lod.shown_version.get(&key), Some(&newest));
        assert!(
            !lod.unmeshed.contains(&key),
            "the newest revision settles it"
        );

        // A canceled result, and one for a tile that left the view, are dropped
        // however new they are.
        lod.mark_mesh(key);
        let next = *lod.mesh_version.get(&key).unwrap();
        lod.accept_mesh(key, next, SectionMesh::default(), true, &mut meshes);
        assert_eq!(meshes.len(), 2, "a canceled result is dropped");
        lod.active.remove(&key);
        lod.accept_mesh(key, next + 1, SectionMesh::default(), false, &mut meshes);
        assert_eq!(meshes.len(), 2, "a tile that left the view is dropped");
    }

    #[test]
    fn far_meshing_holds_while_small_machine_near_terrain_loads() {
        let mut lod = Lod::scheduler_for_test();
        let key = TileKey {
            x: 0,
            z: 0,
            level: 0,
        };
        lod.small = true;
        lod.active.insert(key);
        lod.ready
            .insert(key, Arc::new(vec![Column::default(); CELLS * CELLS]));
        lod.mark_mesh(key);
        lod.set_near_loading(true);
        lod.dispatch_meshes([0, 0]);
        assert!(lod.shared.mesh_jobs.lock().unwrap().is_empty());

        lod.set_near_loading(false);
        lod.dispatch_meshes([0, 0]);
        assert_eq!(lod.shared.mesh_jobs.lock().unwrap().len(), 1);
    }

    #[test]
    fn queued_samples_follow_the_view_and_make_way_for_the_near_terrain() {
        let mut lod = Lod::scheduler_for_test();
        lod.small = true;
        lod.sample_jobs = 16;
        let queued = |lod: &Lod| -> Vec<TileKey> {
            let jobs = lod.shared.jobs.lock().unwrap();
            jobs.iter()
                .filter_map(|job| match job {
                    Job::Sample(key) => Some(*key),
                    _ => None,
                })
                .collect()
        };
        let observing = |lod: &Lod| {
            let jobs = lod.shared.jobs.lock().unwrap();
            jobs.iter().any(|job| matches!(job, Job::Observe((5, 5))))
        };
        let pending = |lod: &Lod| -> Vec<TileKey> {
            let mut keys: Vec<_> = lod.pending.iter().copied().collect();
            keys.sort();
            keys
        };
        let sorted = |mut keys: Vec<TileKey>| {
            keys.sort();
            keys
        };
        let none = || Some(FxHashSet::default());
        lod.update([8., 64., 8.], 2, 64, none());
        let before = queued(&lod);
        assert_eq!(before.len(), 16, "a full queue once the near terrain is in");
        lod.shared
            .jobs
            .lock()
            .unwrap()
            .push_back(Job::Observe((5, 5)));

        // A chunk's move: the tiles still planned keep their place in line.
        lod.update([24., 64., 8.], 2, 64, none());
        let moved = queued(&lod);
        let kept: Vec<_> = before
            .iter()
            .copied()
            .filter(|key| lod.plan.nodes.contains(key))
            .collect();
        assert!(!kept.is_empty());
        assert_eq!(moved[..kept.len()], kept[..]);
        assert_eq!(moved.len(), 16);
        assert!(observing(&lod), "only sample jobs are dropped");

        // A teleport: none of the old view's tiles is sampled any more.
        lod.update([100_000., 64., 100_000.], 2, 64, none());
        let after = queued(&lod);
        assert_eq!(after.len(), 16);
        assert!(after.iter().all(|key| !moved.contains(key)));
        assert!(after.iter().all(|key| lod.plan.nodes.contains(key)));
        assert_eq!(
            pending(&lod),
            sorted(after.clone()),
            "dropped jobs free their slots"
        );

        // New chunks load nearby: the queue gives way at once, not after it
        // drained, and the dropped tiles come back, best first, once they are in.
        lod.set_near_loading(true);
        assert!(queued(&lod).is_empty());
        assert!(lod.pending.is_empty());
        assert!(observing(&lod));
        lod.set_near_loading(false);
        lod.update([100_000., 64., 100_000.], 2, 64, none());
        assert_eq!(queued(&lod), after);

        // Larger machines keep the first `SAMPLE_JOBS` of the queue.
        lod.small = false;
        lod.set_near_loading(true);
        assert_eq!(queued(&lod), after[..SAMPLE_JOBS]);
        assert_eq!(pending(&lod), sorted(after[..SAMPLE_JOBS].to_vec()));
    }

    #[test]
    fn detail_progress_counts_the_columns_that_take_generated_detail() {
        let mut lod = Lod::scheduler_for_test();
        let tile = |x, level| TileKey { x, z: 0, level };
        let (fine, covered, coarse, split) = (tile(0, 0), tile(1, 0), tile(1, 3), tile(1, 2));
        lod.active = [fine, covered, coarse, split].into_iter().collect();
        lod.covered.insert((1, 0));
        lod.plan.split.insert(split);
        lod.detail_masks.insert(fine, [u64::MAX, 1, 0, 0]);
        lod.detail_masks.insert(coarse, [u64::MAX; 4]);
        // Only the fine tile: the covered one is the near chunk's, the split
        // one stands in for finer tiles and the coarse one is past the detail.
        assert_eq!(lod.detail_progress(), (256, 65));
    }

    /// Drives the scheduler by hand: every sample the new incremental path
    /// produces must select exactly the tiles `Plan::select` selects, after
    /// every single sample, and across camera moves and distance changes.
    #[test]
    fn marking_a_tile_dirties_fallback_ancestors_and_touching_neighbors_only() {
        let mut lod = Lod::scheduler_for_test();
        let key = TileKey {
            x: 0,
            z: 0,
            level: 0,
        };
        let parent = key.parent();
        let grand = parent.parent();
        let west = TileKey {
            x: -1,
            z: 0,
            level: 2,
        };
        let near = TileKey {
            x: -1,
            z: 0,
            level: 1,
        };
        let far = TileKey {
            x: -1,
            z: 1,
            level: 1,
        };
        let near_child = TileKey {
            x: -1,
            z: 0,
            level: 0,
        };
        let behind_child = TileKey {
            x: -2,
            z: 0,
            level: 0,
        };
        let far_child = TileKey {
            x: -1,
            z: 2,
            level: 0,
        };
        lod.plan.nodes.extend([
            key,
            parent,
            grand,
            west,
            near,
            far,
            near_child,
            behind_child,
            far_child,
        ]);
        lod.plan.split.extend([parent, grand, west, near, far]);
        lod.active
            .extend([key, grand, near_child, behind_child, far_child]);
        lod.mark_selection_changed(key);
        let dirty: FxHashSet<_> = lod.dirty.keys().copied().collect();
        assert!(dirty.contains(&key), "the tile itself");
        assert!(dirty.contains(&grand), "the active fallback ancestor");
        assert!(dirty.contains(&near_child), "the neighbor across the side");
        assert!(
            !dirty.contains(&far_child),
            "a neighbor child beyond the changed interval"
        );
        assert!(
            !dirty.contains(&behind_child),
            "a neighbor child behind the shared face"
        );
        assert!(!dirty.contains(&near) && !dirty.contains(&west));
        // New column data does not move footprints: no fallback ancestor.
        lod.dirty.clear();
        lod.mark_data_changed(key);
        let dirty: FxHashSet<_> = lod.dirty.keys().copied().collect();
        assert!(dirty.contains(&key));
        assert!(dirty.contains(&near_child), "edge columns still change");
        assert!(
            !dirty.contains(&grand),
            "a fallback ancestor's clipping did not"
        );
    }

    #[test]
    fn only_children_on_the_shared_face_are_walked() {
        let west = TileKey {
            x: -1,
            z: 0,
            level: 1,
        };
        assert!(Lod::side_flush(
            TileKey {
                x: -1,
                z: 0,
                level: 0
            },
            0,
            west
        ));
        assert!(!Lod::side_flush(
            TileKey {
                x: -2,
                z: 0,
                level: 0
            },
            0,
            west
        ));
        let east = TileKey {
            x: 1,
            z: 0,
            level: 1,
        };
        assert!(Lod::side_flush(
            TileKey {
                x: 2,
                z: 0,
                level: 0
            },
            1,
            east
        ));
        assert!(!Lod::side_flush(
            TileKey {
                x: 3,
                z: 0,
                level: 0
            },
            1,
            east
        ));
        let north = TileKey {
            x: 0,
            z: -1,
            level: 1,
        };
        assert!(Lod::side_flush(
            TileKey {
                x: 0,
                z: -1,
                level: 0
            },
            2,
            north
        ));
        assert!(!Lod::side_flush(
            TileKey {
                x: 0,
                z: -2,
                level: 0
            },
            2,
            north
        ));
        let south = TileKey {
            x: 0,
            z: 1,
            level: 1,
        };
        assert!(Lod::side_flush(
            TileKey {
                x: 0,
                z: 2,
                level: 0
            },
            3,
            south
        ));
        assert!(!Lod::side_flush(
            TileKey {
                x: 0,
                z: 3,
                level: 0
            },
            3,
            south
        ));
    }

    #[test]
    fn a_cutout_wait_follows_a_retired_tile_to_its_children() {
        let mut lod = Lod::scheduler_for_test();
        let parent = TileKey {
            x: 0,
            z: 0,
            level: 1,
        };
        let child = parent.children()[0];
        lod.plan.nodes.insert(parent);
        lod.plan.split.insert(parent);
        lod.plan.nodes.insert(child);
        lod.active.insert(child);
        lod.unmeshed.insert(child);
        lod.cutout_pending = Some([(0, 0)].into_iter().collect());
        lod.cutout_waiting.insert(parent);
        lod.transfer_cutout_wait(parent);
        assert!(!lod.cutout_waiting.contains(&parent));
        assert!(lod.cutout_waiting.contains(&child));
        lod.publish_cutout_if_ready();
        assert!(lod.take_cutout().is_none());
        lod.cutout_waiting.remove(&child);
        lod.publish_cutout_if_ready();
        assert!(lod.take_cutout().is_some());
        // A level-0 waiter has no children to hand the wait to.
        let fine = TileKey {
            x: 5,
            z: 5,
            level: 0,
        };
        lod.cutout_pending = Some([(5, 5)].into_iter().collect());
        lod.cutout_waiting.insert(fine);
        lod.transfer_cutout_wait(fine);
        assert!(!lod.cutout_waiting.contains(&fine));
        lod.publish_cutout_if_ready();
        assert!(lod.take_cutout().is_some());
    }

    #[test]
    fn a_cutout_waits_for_the_tiles_it_affected() {
        let mut lod = Lod::scheduler_for_test();
        let tile = TileKey {
            x: 0,
            z: 0,
            level: 0,
        };
        lod.cutout_pending = Some([(1, 2)].into_iter().collect());
        lod.cutout_waiting.insert(tile);
        lod.publish_cutout_if_ready();
        assert!(lod.take_cutout().is_none(), "cutout before its replacement");
        // A later, unrelated mesh does not stand in for the affected tile.
        lod.meshed.insert(TileKey {
            x: 8,
            z: 8,
            level: 0,
        });
        lod.publish_cutout_if_ready();
        assert!(lod.take_cutout().is_none());
        lod.cutout_waiting.remove(&tile);
        lod.publish_cutout_if_ready();
        assert_eq!(lod.take_cutout(), Some([(1, 2)].into_iter().collect()));
        // A second change unions tiles still waiting from the first.
        lod.cutout_pending = Some([(3, 4)].into_iter().collect());
        lod.cutout_waiting.insert(tile);
        lod.cutout_waiting.remove(&tile);
        lod.publish_cutout_if_ready();
        assert_eq!(lod.take_cutout(), Some([(3, 4)].into_iter().collect()));
    }

    /// A saved summary is not detail: a covered empty stand-in must not let
    /// the cutout release the full-detail columns under it.
    #[test]
    fn a_saved_summary_does_not_release_an_unfilled_stand_in() {
        let mut lod = Lod::scheduler_for_test();
        let key = TileKey {
            x: 3,
            z: -2,
            level: 0,
        };
        lod.plan.nodes.insert(key);
        lod.ready
            .insert(key, Arc::new(vec![Column::default(); 256]));
        lod.saved.insert(
            (3, -2),
            Arc::new(std::array::from_fn(|_| Column::default())),
        );
        lod.retiring.insert((3, -2));
        lod.covered.insert((3, -2));
        lod.prune_retiring();
        assert!(lod.retiring.contains(&(3, -2)), "released without detail");
        assert!(lod.covered.contains(&(3, -2)));
        lod.detail_masks.insert(key, [u64::MAX; 4]);
        lod.prune_retiring();
        assert!(!lod.retiring.contains(&(3, -2)));
        assert!(!lod.covered.contains(&(3, -2)));
    }

    /// Completing a subtree must not remesh every tile inside it: only a
    /// tile's own active transition changes the meshes below it.
    #[test]
    fn completing_an_undrawn_subtree_marks_no_descendants() {
        let mut lod = Lod::scheduler_for_test();
        let parent = TileKey {
            x: 0,
            z: 0,
            level: 1,
        };
        let children = parent.children();
        lod.plan.nodes.insert(parent);
        lod.plan.split.insert(parent);
        lod.active.extend(children);
        for child in children {
            lod.plan.nodes.insert(child);
            lod.ready
                .insert(child, Arc::new(vec![Column::default(); 256]));
            lod.complete.insert(child, true);
        }
        lod.complete.insert(parent, false);
        lod.dirty.clear();
        lod.refresh_path(parent);
        assert!(lod.dirty.is_empty(), "completion remeshed descendants");
        assert!(lod.complete[&parent]);
        assert!(!lod.active.contains(&parent));
    }

    #[test]
    fn removals_wait_for_every_overlapping_replacement_mesh() {
        let mut lod = Lod::scheduler_for_test();
        let parent = TileKey {
            x: 0,
            z: 0,
            level: 1,
        };
        let child = TileKey {
            x: 0,
            z: 0,
            level: 0,
        };
        lod.active.insert(child);
        lod.unmeshed.insert(child);
        lod.removed_pending.insert(parent);
        lod.release_removed();
        assert!(
            lod.removed().is_empty(),
            "removed before the replacement was meshed"
        );
        lod.meshed.insert(child);
        lod.unmeshed.remove(&child);
        lod.release_removed();
        assert_eq!(lod.removed(), [parent]);
        // Out of range: nothing overlaps it, so it may fade out at once.
        let gone = TileKey {
            x: 100,
            z: 100,
            level: 3,
        };
        lod.removed_pending.insert(gone);
        lod.release_removed();
        assert_eq!(lod.removed(), [gone]);
    }

    #[test]
    fn incremental_selection_matches_plan_select_after_every_sample() {
        let mut lod = Lod::scheduler_for_test();
        let column = Column::from_surface(Some(Surface {
            height: 64,
            state: BlockStateId(1),
            water: None,
            biome: 0,
        }));
        let columns = vec![column; CELLS * CELLS];
        let mut camera = [0.0, 80.0, 0.0];
        let finish_queued = |lod: &mut Lod| {
            let mut keys = Vec::new();
            {
                let mut jobs = lod.shared.jobs.lock().unwrap();
                let mut i = 0;
                while i < jobs.len() {
                    if matches!(jobs[i], Job::Sample(_)) {
                        let Some(Job::Sample(key)) = jobs.remove(i) else {
                            unreachable!()
                        };
                        keys.push(key);
                    } else {
                        i += 1;
                    }
                }
            }
            for key in &keys {
                lod.finish_sample(*key, columns.clone());
            }
            !keys.is_empty()
        };
        let check = |lod: &Lod| {
            let mut expected = Vec::new();
            for &root in &lod.plan.roots {
                lod.plan.select(root, &lod.ready, &mut expected);
            }
            expected.retain(|key| key.level != 0 || !lod.covered.contains(&(key.x, key.z)));
            let expected: FxHashSet<_> = expected.into_iter().collect();
            assert_eq!(lod.active, expected);
        };
        for _ in 0..2 {
            for _ in 0..4000 {
                lod.update(camera, 2, 64, Some(FxHashSet::default()));
                check(&lod);
                if !finish_queued(&mut lod) {
                    break;
                }
            }
            let (desired, sampled) = lod.progress();
            assert_eq!((desired, sampled), (desired, desired), "all leaves sampled");
            check(&lod);
            // Move a chunk: retained samples stay selected, new ones join.
            camera[0] += 16.0;
            lod.update(camera, 2, 64, Some(FxHashSet::default()));
            check(&lod);
        }
        // A covered level-0 stand-in completes its parent but is not drawn.
        lod.update(camera, 2, 64, Some([(0, 0)].into_iter().collect()));
        check(&lod);
        assert!(lod.ready.contains_key(&TileKey {
            x: 0,
            z: 0,
            level: 0
        }));
        assert!(!lod.active.contains(&TileKey {
            x: 0,
            z: 0,
            level: 0
        }));
        // Shrinking the distance retires tiles and reports them once.
        lod.update(camera, 0, 32, Some(FxHashSet::default()));
        check(&lod);
        let _ = lod.removed();
    }
    /// The prepared resources' far-terrain worker context and block facts.
    fn test_context() -> crate::Result<(Arc<WorkerContext>, BlockFacts)> {
        use crate::{assets::Assets, biome::Biomes, light::LightTables, mesher::MeshTables};
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES"),
        );
        let assets = Arc::new(Assets::load(&root)?);
        let render = Arc::new(assets.reload(&root)?);
        let facts = BlockFacts::load(&root, assets.registry)?;
        let biomes = Arc::new(Biomes::load(&root)?);
        let tables = Arc::new(MeshTables::new(
            &assets,
            &render,
            &facts,
            crate::models::load_visibility(&root, assets.registry)?,
        )?);
        let context = Arc::new(WorkerContext {
            light: Arc::new(LightTables::load(&root, assets.registry, &facts)?),
            tables,
            assets,
            render,
            biomes,
        });
        Ok((context, facts))
    }
    /// A server's far terrain (`server_terrain`) in a scratch folder, and a
    /// chunk of stone at y 70 to observe.
    struct ServerLod {
        lod: Lod,
        facts: BlockFacts,
        stone: BlockStateId,
        dir: std::path::PathBuf,
    }
    impl ServerLod {
        fn new(name: &str) -> crate::Result<Self> {
            Self::with(name, |dir| {
                crate::server_terrain::ServerTerrain::new(dir.into(), None)
            })
        }
        fn with(
            name: &str,
            source: impl FnOnce(&std::path::Path) -> Arc<dyn LodSource>,
        ) -> crate::Result<Self> {
            let (context, facts) = test_context()?;
            let stone = context
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let dir = std::env::temp_dir().join(format!("horizon-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            Ok(Self {
                lod: Lod::new(source(&dir), context),
                facts,
                stone,
                dir,
            })
        }
        fn observe(&mut self, x: i32, z: i32) {
            let mut chunk = Chunk::empty(x, z);
            for i in 0..256 {
                chunk.set(i % 16, 70, i / 16, self.stone.0);
            }
            self.lod.observe(Arc::new(chunk), &self.facts);
        }
        /// Updates until nothing is left to sample, summarize or mesh (`settle`:
        /// for a while), or only a few times.
        fn run(
            &mut self,
            camera: [f64; 3],
            detail: i32,
            distance: u32,
            covered: &FxHashSet<(i32, i32)>,
            settle: bool,
        ) {
            let lod = &mut self.lod;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
            let mut covered = Some(covered.clone());
            let mut quiet = 0;
            while quiet < if settle { 50 } else { 3 } {
                lod.update(camera, detail, distance, covered.take());
                lod.removed();
                // Every tile sampled, but those waiting for their data.
                let unsampled = lod.plan.nodes.iter().any(|key| {
                    !lod.plan.split.contains(key)
                        && !lod.ready.contains_key(key)
                        && !lod.retry.contains_key(key)
                });
                let idle = lod.pending.is_empty()
                    && lod.outstanding_meshes == 0
                    && lod.dirty.is_empty()
                    && lod.observing() == 0
                    && lod
                        .shared
                        .jobs
                        .lock()
                        .unwrap()
                        .iter()
                        .all(|job| matches!(job, Job::RequestDetail { .. }))
                    && !unsampled;
                quiet = if idle { quiet + 1 } else { 0 };
                assert!(
                    std::time::Instant::now() < deadline,
                    "far terrain never settled: {} pending, {} meshing, {} observing, {} jobs, {} unsampled, {} waiting",
                    lod.pending.len(),
                    lod.outstanding_meshes,
                    lod.observing(),
                    lod.shared.jobs.lock().unwrap().len(),
                    lod.plan
                        .nodes
                        .iter()
                        .filter(|key| !lod.plan.split.contains(key)
                            && !lod.ready.contains_key(key)
                            && !lod.retry.contains_key(key))
                        .count(),
                    lod.retry.len(),
                );
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        /// The chunks of `chunks` the active tiles don't draw as observed:
        /// the visited column or the tile's sample there, at the chunk's middle.
        fn missing(&self, chunks: impl Iterator<Item = (i32, i32)>) -> Vec<(i32, i32)> {
            chunks
                .filter(|&chunk| {
                    self.drawn(chunk)
                        .is_none_or(|c| c.spans.last().is_none_or(|s| s.max != 71))
                })
                .collect()
        }
        /// The column the active tiles draw at a chunk's middle.
        fn drawn(&self, (cx, cz): (i32, i32)) -> Option<Column> {
            let lod = &self.lod;
            let snapshot = lod.snapshot();
            let drawn = |x: i32, z: i32| {
                let key = lod.active.iter().copied().find(|key| {
                    let [ox, _, oz] = key.origin();
                    (ox..ox + key.size()).contains(&x) && (oz..oz + key.size()).contains(&z)
                })?;
                let stride = key.size() / CELLS as i32;
                let [ox, _, oz] = key.origin();
                let (cx, cz) = ((x - ox) / stride, (z - oz) / stride);
                let (wx, wz) = (ox + cx * stride, oz + cz * stride);
                Some(match snapshot.visited_source(wx, wz, stride) {
                    Some(source) => source.get().clone(),
                    None => lod.ready[&key][(cz * CELLS as i32 + cx) as usize].clone(),
                })
            };
            drawn(cx * 16 + 8, cz * 16 + 8)
        }
    }
    impl Drop for ServerLod {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
    #[test]
    #[ignore = "requires prepared models/textures in HORIZON_TEST_RESOURCES"]
    fn chunks_a_server_dropped_stay_drawn_as_far_terrain() -> crate::Result<()> {
        let mut far = ServerLod::new("lod-dropped")?;
        // A server's view distance of 10 (it sends a ring more), drawn to a
        // render distance of 32 (`streaming.radius`, the covered area).
        let (view, detail, distance) = (10, 32, 64);
        let mut resident = FxHashSet::default();
        let mut dropped = FxHashSet::default();
        // The player flies east past some 1800 chunks: more than `MAX_SAVED`.
        let end = 60;
        for step in 0..=end {
            let center = (step, 0);
            let now: FxHashSet<_> = (-view..=view)
                .flat_map(|dz| (-view..=view).map(move |dx| (center.0 + dx, center.1 + dz)))
                .collect();
            for &(x, z) in now.difference(&resident) {
                far.observe(x, z);
            }
            for &(x, z) in resident.difference(&now) {
                // `forget_remote_chunk`: the last of it, for far terrain.
                far.observe(x, z);
                dropped.insert((x, z));
            }
            resident = now;
            let covered = resident
                .iter()
                .copied()
                .filter(|&(x, z)| crate::streaming::in_view_distance(center, detail, x, z))
                .collect();
            let camera = [f64::from(center.0 * 16 + 8), 100.0, 8.0];
            far.run(camera, detail, distance, &covered, step == end);
        }
        // Those within the LOD distance of where the player is now.
        let near = |&(x, z): &(i32, i32)| (x - end).pow(2) + z * z < (distance as i32 - 4).pow(2);
        let missing = far.missing(dropped.iter().copied().filter(near));
        assert!(
            missing.is_empty(),
            "{} dropped chunks aren't drawn, as {:?}",
            missing.len(),
            &missing[..missing.len().min(8)]
        );
        Ok(())
    }
    #[test]
    #[ignore = "requires prepared models/textures in HORIZON_TEST_RESOURCES"]
    fn far_cells_sampled_before_their_chunks_were_kept_fill_in() -> crate::Result<()> {
        let mut far = ServerLod::new("lod-late")?;
        // Seen from 5000 blocks: cells of 32 blocks and wider over them.
        let camera = [-5000.0, 100.0, 8.0];
        let (detail, distance) = (10, 512);
        let none = FxHashSet::default();
        // Every tile sampled while nothing is kept, then 32 × 32 chunks are.
        far.run(camera, detail, distance, &none, true);
        let chunks: Vec<_> = (0..32).flat_map(|z| (0..32).map(move |x| (x, z))).collect();
        assert_eq!(far.missing(chunks.iter().copied()).len(), chunks.len());
        for &(x, z) in &chunks {
            far.observe(x, z);
        }
        far.run(camera, detail, distance, &none, true);
        let missing = far.missing(chunks.iter().copied());
        assert!(
            missing.is_empty(),
            "{} kept chunks aren't drawn",
            missing.len()
        );
        Ok(())
    }
    /// A BlueMap whose coarse levels are in, its finest (cells up to a
    /// chunk) still downloading until `arrived`.
    struct Downloading {
        stone: BlockStateId,
        arrived: Arc<std::sync::atomic::AtomicBool>,
    }
    struct DownloadingSampler {
        stone: BlockStateId,
        arrived: Arc<std::sync::atomic::AtomicBool>,
        details: Vec<crate::streaming::LodDetail>,
    }
    impl LodSource for Downloading {
        fn sampler(self: Arc<Self>) -> Box<dyn crate::streaming::LodSampler> {
            Box::new(DownloadingSampler {
                stone: self.stone,
                arrived: self.arrived.clone(),
                details: Vec::new(),
            })
        }
    }
    impl crate::streaming::LodSampler for DownloadingSampler {
        fn fork(&self) -> Option<Box<dyn crate::streaming::LodSampler>> {
            Some(Box::new(Self {
                stone: self.stone,
                arrived: self.arrived.clone(),
                details: Vec::new(),
            }))
        }
        fn sample(&mut self, _x: i32, _z: i32) -> Option<Surface> {
            None
        }
        /// The coarse levels' stone, the finest's next state, and the
        /// coarse one's while the finest is on its way (`server_terrain`):
        /// stood in for, so not `waited`.
        fn reduced_column(&mut self, _x: i32, _z: i32, width: i32) -> Column {
            let state = if width > 16 {
                self.stone
            } else if self.arrived.load(Ordering::Relaxed) {
                BlockStateId(self.stone.0 + 1)
            } else {
                self.stone
            };
            Column {
                surface: None,
                spans: vec![Span {
                    min: MIN_Y as i16,
                    max: 71,
                    state,
                    biome: 0,
                    light: 0xF0,
                }]
                .into(),
            }
        }
        /// Chunks' detail once arrived, as `server_terrain`'s: asked again
        /// while on its way.
        fn request_detail(&mut self, chunks: &[(i32, i32)], _center: [i32; 2]) {
            if self.arrived.load(Ordering::Relaxed) {
                let columns = Arc::new(std::array::from_fn(|_| self.reduced_column(0, 0, 1)));
                self.details
                    .extend(chunks.iter().map(|&chunk| (chunk, columns.clone())));
            }
        }
        fn drain_details(&mut self) -> Vec<crate::streaming::LodDetail> {
            std::mem::take(&mut self.details)
        }
    }
    #[test]
    #[ignore = "requires prepared models/textures in HORIZON_TEST_RESOURCES"]
    fn coarser_stand_ins_keep_the_ring_past_the_near_chunks_drawn() -> crate::Result<()> {
        let arrived = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut far = ServerLod::with("lod-waiting", |_| {
            let stone = BlockStateId(1);
            Arc::new(Downloading {
                stone,
                arrived: arrived.clone(),
            })
        })?;
        let (camera, detail, distance) = ([8.0, 100.0, 8.0], 10, 64);
        let none = FxHashSet::default();
        // The ring past the near terrain, where nothing stood.
        let ring: Vec<_> = (-24..=24)
            .flat_map(|z| (-24..=24).map(move |x| (x, z)))
            .filter(|&(x, z)| x * x + z * z > 11 * 11)
            .collect();
        far.run(camera, detail, distance, &none, true);
        let missing = far.missing(ring.iter().copied());
        assert!(
            missing.is_empty(),
            "{} of {} chunks past the near terrain show nothing while their tiles wait",
            missing.len(),
            ring.len()
        );
        // The map's tiles arrive (`BlueMap::take_changed`): the finest replace them.
        arrived.store(true, Ordering::Relaxed);
        far.lod.refresh([-1024, -1024, 1024, 1024], (1, 24));
        far.run(camera, detail, distance, &none, true);
        assert!(far.lod.retry.is_empty());
        let fine = BlockStateId(2);
        let coarse: Vec<_> = ring
            .iter()
            .copied()
            .filter(|&chunk| {
                far.drawn(chunk)
                    .is_none_or(|c| c.spans.last().is_none_or(|s| s.state != fine))
            })
            .collect();
        assert!(
            coarse.is_empty(),
            "{} chunks still drawn coarse",
            coarse.len()
        );
        assert!(far.lod.active.iter().any(|key| key.level == 0));
        Ok(())
    }

    /// The sample jobs `request_samples` queues now, taken off the queue.
    fn queued(lod: &mut Lod) -> Vec<TileKey> {
        lod.request_samples([0, 0]);
        let mut jobs = lod.shared.jobs.lock().unwrap();
        jobs.drain(..)
            .filter_map(|job| match job {
                Job::Sample(key) => Some(key),
                _ => None,
            })
            .collect()
    }
    fn samples(top: i16) -> Vec<Column> {
        let mut column = Column::default();
        column.spans.push(Span {
            min: -64,
            max: top,
            state: BlockStateId(1),
            biome: 0,
            light: 0,
        });
        vec![column; CELLS * CELLS]
    }
    const AREA: [i32; 4] = [0, 0, 128, 128];
    const ALL: (i32, i32) = (1, i32::MAX);

    /// A first sample that read a map tile still downloading is thrown
    /// away; if the tile came while it was taken, it is sampled again at
    /// once, else after `WAIT_RETRY` or when the tile comes.
    #[test]
    fn waited_samples_are_taken_again_when_their_data_comes() {
        let mut lod = Lod::scheduler_for_test();
        let key = TileKey {
            x: 0,
            z: 0,
            level: 3,
        };
        lod.plan.nodes.insert(key);
        assert_eq!(queued(&mut lod), [key]);
        lod.finish_any_sample(key, vec![Column::default(); CELLS * CELLS], true);
        assert!(lod.retry.contains_key(&key) && !lod.ready.contains_key(&key));
        assert!(queued(&mut lod).is_empty(), "it waits for its data");
        lod.refresh(AREA, ALL);
        assert!(!lod.retry.contains_key(&key));
        assert_eq!(queued(&mut lod), [key], "the data came");

        // The data comes while it is being sampled.
        lod.refresh(AREA, ALL);
        lod.finish_any_sample(key, vec![Column::default(); CELLS * CELLS], true);
        assert!(!lod.retry.contains_key(&key));
        assert_eq!(queued(&mut lod), [key], "sampled again at once");
        // Coarser levels stood in for what waited: drawn.
        lod.finish_any_sample(key, samples(70), false);
        assert!(lod.ready.contains_key(&key) && !lod.stale.contains(&key));
        assert!(queued(&mut lod).is_empty());
        // Refreshed, then sampled with a hole where a tile is still on its
        // way: what is drawn stays, the hole isn't.
        lod.refresh(AREA, ALL);
        assert_eq!(queued(&mut lod), [key]);
        let mut holed = samples(80);
        holed[..CELLS].fill(Column::default());
        lod.finish_any_sample(key, holed, true);
        assert_eq!(lod.ready[&key][0], samples(70)[0]);
        assert!(lod.retry.contains_key(&key));
    }

    /// A tile refreshed while its sample is in flight is sampled once more:
    /// that sample read the old data. Samples equal to the drawn ones mesh
    /// nothing.
    #[test]
    fn refreshed_tiles_are_sampled_again_and_only_meshed_when_changed() {
        let mut lod = Lod::scheduler_for_test();
        let key = TileKey {
            x: 0,
            z: 0,
            level: 3,
        };
        lod.plan.nodes.insert(key);
        lod.active.insert(key);
        lod.ready.insert(key, Arc::new(samples(70)));
        assert!(queued(&mut lod).is_empty());
        lod.refresh(AREA, ALL);
        assert_eq!(queued(&mut lod), [key]);
        lod.refresh(AREA, ALL);
        lod.finish_sample(key, samples(80));
        assert!(lod.stale.contains(&key), "refreshed again meanwhile");
        assert!(lod.dirty.contains_key(&key), "new samples are meshed");
        assert_eq!(queued(&mut lod), [key]);
        lod.dirty.clear();
        lod.finish_sample(key, samples(80));
        assert!(!lod.stale.contains(&key));
        assert!(lod.dirty.is_empty(), "the same samples mesh nothing");
        assert!(queued(&mut lod).is_empty());
        // Data drawn in other cell widths leaves it alone.
        lod.refresh(AREA, (17, 24));
        assert!(queued(&mut lod).is_empty());
    }

    /// With a source that keeps what it observes (a server's chunks), a wide
    /// tile sampled empty where a chunk is now kept is sampled again.
    #[test]
    fn kept_chunks_resample_the_wide_tiles_over_them() {
        let mut lod = Lod::scheduler_for_test();
        lod.keeps_observed = true;
        let key = TileKey {
            x: 0,
            z: 0,
            level: 5,
        };
        lod.plan.nodes.insert(key);
        lod.ready
            .insert(key, Arc::new(vec![Column::default(); CELLS * CELLS]));
        let column = samples(70).swap_remove(0);
        let columns: Arc<[Column; 256]> = Arc::new(std::array::from_fn(|_| column.clone()));
        lod.finish_observed((1, 1), ReducedColumns::new(columns));
        assert!(lod.stale.contains(&key));
        assert_eq!(queued(&mut lod), [key]);
    }
}
