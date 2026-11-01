//! Sky and block light for one chunk, flood-filled over its 3x3 neighbourhood
//! with vanilla's `LightEngine` rules:
//! - light entering a block loses `max(1, getLightDampening)` (`getOpacity`);
//! - it cannot cross a face where the two blocks' occlusion faces together
//!   cover the whole square (`shapeOccludes`, using `getFaceOcclusionShape`
//!   only for states with `canOcclude && useShapeForLightOcclusion`), which is
//!   what makes slabs, stairs, snow layers etc. block light directionally;
//! - sky sources run down each column to the first occluded edge
//!   (`ChunkSkyLightSources.isEdgeOccluded`: dampening != 0 or the faces
//!   between the two blocks occlude).
//!
//! The final values of vanilla's queue-based engine are the maximum over all
//! paths, so a flood fill to the fixed point reproduces them. Light travels at
//! most 15 blocks, so the 48-wide region determines the centre exactly.
//! `relight_block` applies vanilla's incremental decrease/increase passes
//! directly to resident chunks after a block change.
use crate::fast_hash::FxHashMap;
use crate::{
    blocks::BlockFacts,
    chunk::{Chunk, ColumnLayout, Light, Section, index},
};
use horizon_core::geometry::{Direction, ShapeOp, VoxelShape};
use horizon_world::registry::{BlockRegistry, BlockStateId};
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    path::Path,
    sync::Arc,
};

const W: usize = 48;
/// Face ID 0: empty occlusion face; 1: the full square.
const EMPTY: u16 = 0;
const FULL: u16 = 1;

#[derive(Clone, Copy)]
struct Cell {
    dampening: u8,
    emission: u8,
    /// Any face is non-empty (`!isEmptyShape`), so edges need the face table.
    shaped: bool,
    faces: [u16; 6],
}
pub struct LightTables {
    cells: Vec<Cell>,
    /// `faceShapeOccludes(a, b)` for every pair of distinct face shapes.
    occludes: Vec<bool>,
    faces: usize,
    /// Number of distinct `(shaped, faces)` configurations.
    types: usize,
    /// Per state: `emission << 16 | type << 4 | dampening`. The low 16 bits are
    /// what `light_chunk` stores per cell, so a cell costs 2 bytes and the
    /// flood fill never reads the (large) `cells` table.
    info: Vec<u32>,
    /// `blocked(type_a, type_b, direction)` as a bit per direction.
    block_masks: Vec<u8>,
}
impl LightTables {
    /// Dampening and emission only: every occlusion face is empty.
    pub fn new(facts: &BlockFacts) -> Self {
        let mut tables = Self {
            cells: facts
                .states
                .iter()
                .map(|s| Cell {
                    dampening: s.dampening.min(15),
                    emission: s.emission.min(15),
                    shaped: false,
                    faces: [EMPTY; 6],
                })
                .collect(),
            occludes: vec![false, true, true, true],
            faces: 2,
            types: 1,
            info: Vec::new(),
            block_masks: Vec::new(),
        };
        tables.rebuild_cache();
        tables
    }
    /// Build the packed per-cell info and the per-configuration blocking masks
    /// used by `light_chunk`'s flood fill.
    fn rebuild_cache(&mut self) {
        let empty: (bool, [u16; 6]) = (false, [EMPTY; 6]);
        let mut by_config: HashMap<(bool, [u16; 6]), u16> = HashMap::new();
        by_config.insert(empty, 0);
        let mut configs: Vec<(bool, [u16; 6])> = vec![empty];
        let mut info = vec![0u32; self.cells.len()];
        for (id, cell) in self.cells.iter().enumerate() {
            let ty = *by_config
                .entry((cell.shaped, cell.faces))
                .or_insert_with(|| {
                    configs.push((cell.shaped, cell.faces));
                    (configs.len() - 1) as u16
                });
            let packed = u32::from(ty) << 4 | u32::from(cell.dampening.min(15));
            info[id] = u32::from(cell.emission.min(15)) << 16 | packed;
        }
        let types = configs.len();
        debug_assert!(
            types <= 4096,
            "light tables: {types} cell configurations exceed the 12-bit type field"
        );
        let mut block_masks = vec![0u8; types * types];
        for a in 0..types {
            for b in 0..types {
                let (shaped_a, faces_a) = configs[a];
                let (shaped_b, faces_b) = configs[b];
                // `blocked`: an unshaped pair covers no face; otherwise the two
                // face shapes are compared through the `occludes` table.
                if !shaped_a && !shaped_b {
                    continue;
                }
                let mut mask = 0u8;
                for d in 0..6 {
                    if self.occludes[faces_a[d] as usize * self.faces + faces_b[d ^ 1] as usize] {
                        mask |= 1 << d;
                    }
                }
                block_masks[a * types + b] = mask;
            }
        }
        self.types = types;
        self.info = info;
        self.block_masks = block_masks;
    }
    /// Full tables with the vanilla occlusion shapes from `occlusion.nbt`.
    pub fn load(root: &Path, registry: &BlockRegistry, facts: &BlockFacts) -> crate::Result<Self> {
        use horizon_world::{collision::CachedCollisionRegistry, registry::BlockTableFile};
        let file = BlockTableFile::open(&root.join("occlusion.nbt"), registry)?;
        let occlusion = CachedCollisionRegistry::from_table(file.table("occlusion")?)?;
        let mut tables = Self::new(facts);
        let block = VoxelShape::block();
        let mut shapes: Vec<VoxelShape> = vec![VoxelShape::empty(), block.clone()];
        let mut by_geometry: HashMap<Vec<u64>, u16> = HashMap::new();
        let mut by_shape: HashMap<*const VoxelShape, [u16; 6]> = HashMap::new();
        for (id, cell) in tables.cells.iter_mut().enumerate() {
            let state = &facts.states[id];
            // LightEngine.isEmptyShape: !canOcclude || !useShapeForLightOcclusion.
            if !state.can_occlude || !state.shape_occlusion {
                continue;
            }
            let Some(shape) = occlusion.cached_shape(BlockStateId(id as u32))? else {
                continue;
            };
            if shape.is_empty() {
                continue;
            }
            let faces = if let Some(faces) = by_shape.get(&std::ptr::from_ref(shape)) {
                *faces
            } else {
                let mut faces = [FULL; 6];
                // BlockState.initCache: solidRender shapes use the full faces.
                if shape.join_is_not_empty(&block, ShapeOp::XOR)? {
                    for direction in Direction::ALL {
                        let face = shape.face_shape(direction)?;
                        faces[direction as usize] = if face.is_empty() {
                            EMPTY
                        } else if face.same_identity(&block) {
                            FULL
                        } else {
                            let key: Vec<u64> = face
                                .boxes()
                                .iter()
                                .flat_map(|b| {
                                    b.min_position()
                                        .components()
                                        .into_iter()
                                        .chain(b.max_position().components())
                                })
                                .map(f64::to_bits)
                                .collect();
                            *by_geometry.entry(key).or_insert_with(|| {
                                shapes.push(face.clone());
                                (shapes.len() - 1) as u16
                            })
                        };
                    }
                }
                by_shape.insert(std::ptr::from_ref(shape), faces);
                faces
            };
            cell.faces = faces;
            cell.shaped = faces.iter().any(|f| *f != EMPTY);
        }
        let n = shapes.len();
        let mut occludes = vec![false; n * n];
        for a in 0..n {
            for b in 0..n {
                occludes[a * n + b] = if a == FULL as usize || b == FULL as usize {
                    true
                } else if a == EMPTY as usize && b == EMPTY as usize {
                    false
                } else {
                    // Shapes.faceShapeOccludes: the union covers the whole face.
                    let union = shapes[a].join_unoptimized(&shapes[b], ShapeOp::OR)?;
                    !block.join_is_not_empty(&union, ShapeOp::ONLY_FIRST)?
                };
            }
        }
        tables.occludes = occludes;
        tables.faces = n;
        tables.rebuild_cache();
        Ok(tables)
    }
    /// Assembled tables (tests and benchmarks): derives the packed light cache.
    #[cfg(test)]
    fn from_parts(cells: Vec<Cell>, occludes: Vec<bool>, faces: usize) -> Self {
        let mut tables = Self {
            cells,
            occludes,
            faces,
            types: 0,
            info: Vec::new(),
            block_masks: Vec::new(),
        };
        tables.rebuild_cache();
        tables
    }
    /// `LightEngine.shapeOccludes(from, to, direction)`.
    #[inline]
    fn blocked(&self, from: usize, to: usize, direction: usize) -> bool {
        let (a, b) = (&self.cells[from], &self.cells[to]);
        if !a.shaped && !b.shaped {
            return false;
        }
        let fa = a.faces[direction] as usize;
        let fb = b.faces[direction ^ 1] as usize;
        self.occludes[fa * self.faces + fb]
    }
    /// `blocked` for two packed cells (`type << 4 | dampening`).
    #[inline]
    fn packed_blocked(&self, a: u16, b: u16, direction: usize) -> bool {
        self.block_masks[(a >> 4) as usize * self.types + (b >> 4) as usize] & (1 << direction) != 0
    }
    /// `ChunkSkyLightSources.isEdgeOccluded(top, bottom)`.
    #[inline]
    fn edge_occluded(&self, top: usize, bottom: usize) -> bool {
        self.cells[bottom].dampening != 0 || self.blocked(top, bottom, Direction::Down as usize)
    }
    /// `edge_occluded` for two packed cells.
    #[inline]
    fn packed_edge_occluded(&self, top: u16, bottom: u16) -> bool {
        bottom & 0xF != 0
            || ((top | bottom) != 0 && self.packed_blocked(top, bottom, Direction::Down as usize))
    }
    pub fn emission(&self, state: u32) -> u8 {
        self.cells[state as usize].emission
    }
    pub fn dampening(&self, state: u32) -> u8 {
        self.cells[state as usize].dampening
    }
    /// Whether a state change can change light (`hasDifferentLightProperties`).
    pub fn differs(&self, old: u32, new: u32) -> bool {
        let (a, b) = (&self.cells[old as usize], &self.cells[new as usize]);
        old != new
            && (a.dampening != b.dampening || a.emission != b.emission || a.shaped || b.shaped)
    }
}
// Direction index order matches `Direction` (down, up, north, south, west, east);
// `d ^ 1` is the opposite direction.
const OFFSETS: [(i32, i32, i32); 6] = [
    (0, -1, 0),
    (0, 1, 0),
    (0, 0, -1),
    (0, 0, 1),
    (-1, 0, 0),
    (1, 0, 0),
];

struct LightScratch {
    states: Vec<u16>,
    sky: Vec<u8>,
    block: Vec<u8>,
    queue: Vec<u32>,
    sky_queue: Vec<u32>,
    present: Vec<bool>,
    lowest: Vec<i32>,
    ids: Box<[u32; 4096]>,
}

impl LightScratch {
    fn new() -> Self {
        Self {
            states: Vec::new(),
            sky: Vec::new(),
            block: Vec::new(),
            queue: Vec::new(),
            sky_queue: Vec::new(),
            present: vec![false; W * W],
            lowest: vec![0; W * W],
            ids: Box::new([0u32; 4096]),
        }
    }

    /// Clear queues and shrink them if capacity exceeds the retention cap.
    fn clear_queues(&mut self) {
        const MAX_QUEUE_CAPACITY: usize = 65_536;
        self.queue.clear();
        if self.queue.capacity() > MAX_QUEUE_CAPACITY {
            self.queue.shrink_to(MAX_QUEUE_CAPACITY);
        }
        self.sky_queue.clear();
        if self.sky_queue.capacity() > MAX_QUEUE_CAPACITY {
            self.sky_queue.shrink_to(MAX_QUEUE_CAPACITY);
        }
    }
}

thread_local! {
    static SCRATCH: RefCell<LightScratch> = RefCell::new(LightScratch::new());
}

/// `region[dz][dx]` holds the chunks around the centre (index 1,1).
/// `sky` is false in dimensions without skylight (the Nether).
pub fn light_chunk(
    region: &[[Option<&Chunk>; 3]; 3],
    tables: &LightTables,
    sky_light: bool,
) -> Vec<Light> {
    SCRATCH.with_borrow_mut(|scratch| light_chunk_with_scratch(region, tables, sky_light, scratch))
}

fn light_chunk_with_scratch(
    region: &[[Option<&Chunk>; 3]; 3],
    tables: &LightTables,
    sky_light: bool,
    scratch: &mut LightScratch,
) -> Vec<Light> {
    let sections = region[1][1].expect("centre chunk").sections.len();
    // Highest occupied section in the region bounds the work.
    let mut top_section = 0;
    for chunk in region.iter().flatten().flatten() {
        for (i, s) in chunk.sections.iter().enumerate().rev() {
            if !s.is_uniform(0) {
                top_section = top_section.max(i + 1);
                break;
            }
        }
    }
    // One extra section above the highest occupied one: block light rises
    // up to 14 blocks into it (sky light there is always 15).
    let h = ((top_section + 1) * 16).min(sections * 16);
    let total = W * W * h;

    scratch.states.resize(total, 0);
    scratch.states.fill(0);
    scratch.sky.resize(total, 0);
    scratch.sky.fill(0);
    scratch.block.resize(total, 0);
    scratch.block.fill(0);
    scratch.clear_queues();

    let states = &mut scratch.states[..total];
    let sky = &mut scratch.sky[..total];
    let block = &mut scratch.block[..total];
    let queue = &mut scratch.queue;
    let sky_queue = &mut scratch.sky_queue;
    let ids = &mut scratch.ids;

    for (dz, row) in region.iter().enumerate() {
        let z_base = dz * 16;
        for (dx, chunk) in row.iter().enumerate() {
            let Some(chunk) = chunk else { continue };
            let x_base = dx * 16;
            for (si, section) in chunk.sections.iter().enumerate().take(top_section) {
                if section.is_uniform(0) {
                    continue;
                }
                section.unpack(ids);
                let y_base = si * 16;
                for ly in 0..16 {
                    let y = y_base + ly;
                    let y_offset = y * (W * W);
                    for lz in 0..16 {
                        let z = z_base + lz;
                        let p_row = y_offset + z * W + x_base;
                        let i_row = (ly << 8) | (lz << 4);
                        for lx in 0..16 {
                            let id = ids[i_row + lx];
                            if id == 0 {
                                continue;
                            }
                            let p = p_row + lx;
                            let info = tables.info[id as usize];
                            states[p] = info as u16;
                            let emission = info >> 16;
                            if emission > 0 {
                                block[p] = emission as u8;
                                queue.push(p as u32);
                            }
                        }
                    }
                }
            }
        }
    }

    // Columns of missing neighbour chunks neither hold nor pass light.
    scratch.present.resize(W * W, false);
    for z in 0..W {
        for x in 0..W {
            scratch.present[z * W + x] = region[z / 16][x / 16].is_some();
        }
    }
    let present = &scratch.present[..W * W];

    scratch.lowest.resize(W * W, h as i32);
    let lowest = &mut scratch.lowest[..W * W];

    for z in 0..W {
        for x in 0..W {
            let col = z * W + x;
            if !present[col] || !sky_light {
                lowest[col] = h as i32;
                continue;
            }
            // The top layer (above the occupied range) is always open sky.
            let mut y = h as i32 - 1;
            let mut p = (h - 1) * (W * W) + col;
            sky[p] = 15;
            while y > 0 {
                let next_p = p - (W * W);
                if tables.packed_edge_occluded(states[p], states[next_p]) {
                    break;
                }
                y -= 1;
                p = next_p;
                sky[p] = 15;
            }
            lowest[col] = y;
        }
    }

    if sky_light {
        for z in 0..W {
            for x in 0..W {
                let col = z * W + x;
                if !present[col] {
                    continue;
                }
                let own = lowest[col];
                let mut deepest = own;
                for (nx, nz) in [
                    (x.wrapping_sub(1), z),
                    (x + 1, z),
                    (x, z.wrapping_sub(1)),
                    (x, z + 1),
                ] {
                    if nx < W && nz < W && present[nz * W + nx] {
                        deepest = deepest.max(lowest[nz * W + nx]);
                    }
                }
                // Seed propagation from the source cells that border darker cells.
                let from = own.max(0) as usize;
                let to = (deepest as usize).min(h.saturating_sub(1));
                if from <= to {
                    let mut p = from * (W * W) + col;
                    for _ in from..=to {
                        if sky[p] == 15 {
                            sky_queue.push(p as u32);
                        }
                        p += W * W;
                    }
                }
            }
        }
    }

    let block_masks = &tables.block_masks;
    let types = tables.types;

    let spread = |light: &mut [u8], queue: &mut Vec<u32>| {
        let mut head = 0;
        while head < queue.len() {
            let p = queue[head] as usize;
            head += 1;
            let level = light[p];
            if level <= 1 {
                continue;
            }
            let min_threshold = level - 1;
            let y = p / (W * W);
            let col = p % (W * W);
            let z = col / W;
            let x = col % W;
            let from = states[p];
            let from_row = (from >> 4) as usize * types;

            // Neighbours by linear offset; `present` only depends on the
            // column, and the axis bounds rule out wrapping.
            macro_rules! step_neighbour {
                ($d:expr, $q:expr) => {{
                    let q = $q;
                    if light[q] < min_threshold {
                        let to = states[q];
                        let damp = (to & 0xF).max(1) as u8;
                        if level > light[q] + damp {
                            if block_masks[from_row + (to >> 4) as usize] & (1 << $d) == 0 {
                                light[q] = level - damp;
                                queue.push(q as u32);
                            }
                        }
                    }
                }};
            }
            if y > 0 {
                step_neighbour!(0, p - W * W);
            }
            if y + 1 < h {
                step_neighbour!(1, p + W * W);
            }
            if z > 0 && present[col - W] {
                step_neighbour!(2, p - W);
            }
            if z + 1 < W && present[col + W] {
                step_neighbour!(3, p + W);
            }
            if x > 0 && present[col - 1] {
                step_neighbour!(4, p - 1);
            }
            if x + 1 < W && present[col + 1] {
                step_neighbour!(5, p + 1);
            }
        }
    };
    spread(sky, sky_queue);
    spread(block, queue);
    let result = pack_lighting(sections, h, sky, block, sky_light);
    scratch.clear_queues();
    result
}

/// Extract just the centre column's light (`sections` of it); never copy
/// worker snapshot sections.
fn pack_lighting(
    sections: usize,
    h: usize,
    sky: &[u8],
    block: &[u8],
    sky_light: bool,
) -> Vec<Light> {
    let open = if sky_light { 0xF0 } else { 0x00 };
    (0..sections)
        .map(|si| {
            if si * 16 >= h {
                return Light::Uniform(open);
            }
            let mut data = Box::new([open; 4096]);
            for ly in 0..16 {
                let y = si * 16 + ly;
                if y >= h {
                    continue;
                }
                let y_offset = y * (W * W);
                for lz in 0..16 {
                    let p_row = y_offset + (16 + lz) * W + 16;
                    let out_row = (ly << 8) | (lz << 4);
                    for lx in 0..16 {
                        let p = p_row + lx;
                        data[out_row + lx] = sky[p] << 4 | block[p];
                    }
                }
            }
            if data.iter().all(|&v| v == data[0]) {
                Light::Uniform(data[0])
            } else {
                Light::Full(data)
            }
        })
        .collect()
}

/// Move a worker's light into current sections, preserving later block/biome
/// edits and immutable snapshots retained by other workers.
pub fn apply_lighting(chunk: &mut Chunk, lights: Vec<Light>) {
    for (section, light) in chunk.sections.iter_mut().zip(lights) {
        // A shared section already lit so stays as it is.
        if let (Light::Uniform(a), Light::Uniform(b)) = (&section.light, &light)
            && a == b
        {
            continue;
        }
        Arc::make_mut(section).light = light;
    }
    crate::chunk::share_uniform(&mut chunk.sections);
}

/// `apply_lighting` for a chunk lit before: writes only the sections whose
/// light changed and adds the cells that changed to `changed`.
pub fn apply_relighting(chunk: &mut Chunk, lights: Vec<Light>, changed: &mut LightChanges) {
    changed.keyed(chunk.layout());
    for (sy, (section, light)) in chunk.sections.iter_mut().zip(lights).enumerate() {
        let old = &section.light;
        if let (Light::Uniform(a), Light::Uniform(b)) = (old, &light)
            && a == b
        {
            continue;
        }
        let mut classes = 0;
        for i in 0..4096 {
            if old.get(i) != light.get(i) {
                let (x, y, z) = (i as i32 & 15, (i >> 8) as i32, (i >> 4) as i32 & 15);
                classes |= 1 << LightChanges::class(x, y, z);
            }
        }
        if classes != 0 {
            changed.add((chunk.x, sy as i32, chunk.z), classes);
            Arc::make_mut(section).light = light;
        }
    }
}

/// `DataLayer.get`: the nibble at index `i` (`y << 8 | z << 4 | x`), low first.
#[inline]
fn nibble(layer: &[u8], i: usize) -> u8 {
    (layer[i >> 1] >> ((i & 1) * 4)) & 15
}

/// A column's light as vanilla stores it, two DataLayers per section (sky,
/// block; `layers[i]` for section `i`, `None` where no layer is stored), into
/// the sections' packed light, which then needs no light job. A missing sky
/// layer repeats the bottom layer of the nearest stored one above it, and
/// `above` (per `z << 4 | x`) above the highest
/// (`SkyLightSectionStorage.getLightValue`); a missing block layer is dark.
pub fn light_from_layers<L: AsRef<[u8]>>(
    chunk: &mut Chunk,
    layers: &[(Option<L>, Option<L>)],
    mut above: [u8; 256],
    sky_light: bool,
) {
    let sections = chunk.sections.len();
    for (si, (sky, block)) in layers.iter().enumerate().take(sections).rev() {
        let sky = sky.as_ref().filter(|_| sky_light);
        let section = Arc::make_mut(&mut chunk.sections[si]);
        if sky.is_none() && block.is_none() && above.iter().all(|&v| v == above[0]) {
            section.light = Light::Uniform(above[0] << 4);
            section.compact();
            continue;
        }
        let mut data = Box::new([0u8; 4096]);
        for (i, packed) in data.iter_mut().enumerate() {
            let s = sky.map_or(above[i & 255], |a| nibble(a.as_ref(), i));
            let b = block.as_ref().map_or(0, |a| nibble(a.as_ref(), i));
            *packed = s << 4 | b;
        }
        for (i, a) in above.iter_mut().enumerate() {
            *a = data[i] >> 4;
        }
        section.light = Light::Full(data);
        section.compact();
    }
    crate::chunk::share_uniform(&mut chunk.sections);
    chunk.lit = true;
    chunk.light_loaded = true;
}

/// A light update's layers (`LevelLightEngine.queueSectionData`): each one
/// given replaces that kind of light in its section, the other kind stays.
/// Returns the sections whose light changed.
pub fn update_light_layers<L: AsRef<[u8]>>(
    chunk: &mut Chunk,
    layers: &[(Option<L>, Option<L>)],
    sky_light: bool,
) -> Vec<usize> {
    let mut changed = Vec::new();
    let sections = chunk.sections.len();
    for (si, (sky, block)) in layers.iter().enumerate().take(sections) {
        let sky = sky.as_ref().filter(|_| sky_light);
        if sky.is_none() && block.is_none() {
            continue;
        }
        let old = &chunk.sections[si].light;
        let mut data = Box::new([0u8; 4096]);
        for (i, packed) in data.iter_mut().enumerate() {
            let was = old.get(i);
            let s = sky.map_or(was >> 4, |a| nibble(a.as_ref(), i));
            let b = block.as_ref().map_or(was & 15, |a| nibble(a.as_ref(), i));
            *packed = s << 4 | b;
        }
        if (0..4096).all(|i| old.get(i) == data[i]) {
            continue;
        }
        let section = Arc::make_mut(&mut chunk.sections[si]);
        section.light = Light::Full(data);
        section.compact();
        changed.push(si);
    }
    changed
}

/// Resident chunks the incremental relighter reads and writes.
pub trait LightWorld {
    /// Section `s` of chunk (cx, cz), None unless the chunk is loaded and lit.
    fn section(&self, cx: i32, s: usize, cz: i32) -> Option<&Arc<Section>>;
    /// Replace the light of a section `section` returned.
    fn set_section_light(&mut self, cx: i32, s: usize, cz: i32, light: Light);
    /// The column the world's chunks hold (its level's).
    fn layout(&self) -> ColumnLayout;
}
impl<S: std::hash::BuildHasher> LightWorld for HashMap<(i32, i32), Arc<Chunk>, S> {
    fn layout(&self) -> ColumnLayout {
        self.values()
            .next()
            .map_or(ColumnLayout::VANILLA, |c| c.layout())
    }
    fn section(&self, cx: i32, s: usize, cz: i32) -> Option<&Arc<Section>> {
        self.get(&(cx, cz))
            .filter(|c| c.lit)
            .map(|c| &c.sections[s])
    }
    fn set_section_light(&mut self, cx: i32, s: usize, cz: i32, light: Light) {
        if let Some(chunk) = self.get_mut(&(cx, cz)) {
            Arc::make_mut(&mut Arc::make_mut(chunk).sections[s]).light = light;
        }
    }
}
/// The sections one relight touches, held for the whole flood fill so each
/// cell access is an array index instead of a chunk lookup. Light a section
/// gets written is copied out on the first write and stored back by `finish`.
struct Region<'w> {
    world: &'w mut dyn LightWorld,
    layout: ColumnLayout,
    slots: Vec<Slot>,
    /// Section key (cx, s, cz) to slot, `NONE` where the chunk isn't lit.
    index: FxHashMap<Pos, u32>,
    last: (Pos, u32),
}
struct Slot {
    key: Pos,
    /// The world's section; its light is the value before this relight.
    section: Arc<Section>,
    light: Option<Box<[u8; 4096]>>,
}
const NONE: u32 = u32::MAX;
impl<'w> Region<'w> {
    fn new(world: &'w mut dyn LightWorld) -> Self {
        Self {
            layout: world.layout(),
            world,
            slots: Vec::new(),
            index: FxHashMap::default(),
            last: ((i32::MIN, 0, 0), NONE),
        }
    }
    #[inline]
    fn slot(&mut self, x: i32, y: i32, z: i32) -> Option<usize> {
        let s = self.layout.section_index(y)?;
        let key = (x >> 4, s as i32, z >> 4);
        if self.last.0 != key {
            self.last = (key, self.load(key));
        }
        (self.last.1 != NONE).then_some(self.last.1 as usize)
    }
    fn load(&mut self, key: Pos) -> u32 {
        if let Some(&slot) = self.index.get(&key) {
            return slot;
        }
        let slot = match self.world.section(key.0, key.1 as usize, key.2) {
            Some(section) => {
                self.slots.push(Slot {
                    key,
                    section: section.clone(),
                    light: None,
                });
                (self.slots.len() - 1) as u32
            }
            None => NONE,
        };
        self.index.insert(key, slot);
        slot
    }
    fn state(&mut self, x: i32, y: i32, z: i32) -> Option<u32> {
        let slot = self.slot(x, y, z)?;
        Some(self.slots[slot].section.get(index(x, y, z)))
    }
    /// The one state of the section holding `y`, when it has a single one.
    fn uniform_state(&mut self, x: i32, y: i32, z: i32) -> Option<u32> {
        let slot = self.slot(x, y, z)?;
        let section = &self.slots[slot].section;
        (section.blocks.bits() == 0).then(|| section.get(0))
    }
    /// Packed light (sky << 4 | block), None outside loaded, lit chunks.
    fn light(&mut self, x: i32, y: i32, z: i32) -> Option<u8> {
        let slot = self.slot(x, y, z)?;
        let slot = &self.slots[slot];
        let i = index(x, y, z);
        Some(match &slot.light {
            Some(light) => light[i],
            None => slot.section.light(i),
        })
    }
    /// State and packed light of a cell, with a single section lookup.
    fn cell(&mut self, x: i32, y: i32, z: i32) -> Option<(u32, u8)> {
        let slot = self.slot(x, y, z)?;
        let slot = &self.slots[slot];
        let i = index(x, y, z);
        let light = match &slot.light {
            Some(light) => light[i],
            None => slot.section.light(i),
        };
        Some((slot.section.get(i), light))
    }
    /// Rewrite a cell's packed light; nothing outside loaded, lit chunks.
    fn update_light(&mut self, x: i32, y: i32, z: i32, f: impl FnOnce(u8) -> u8) {
        let Some(slot) = self.slot(x, y, z) else {
            return;
        };
        let slot = &mut self.slots[slot];
        let light = slot.light.get_or_insert_with(|| match &slot.section.light {
            Light::Uniform(v) => Box::new([*v; 4096]),
            Light::Full(a) => a.clone(),
        });
        let i = index(x, y, z);
        light[i] = f(light[i]);
    }
    /// Store back the light of every section whose values changed.
    fn finish(self) {
        for slot in self.slots {
            let Some(light) = slot.light else {
                continue;
            };
            if (0..4096).all(|i| slot.section.light(i) == light[i]) {
                continue;
            }
            // Drop the clone first so the world's section isn't copied.
            drop(slot.section);
            let (cx, s, cz) = slot.key;
            self.world
                .set_section_light(cx, s as usize, cz, Light::Full(light));
        }
    }
}
type Pos = (i32, i32, i32);
/// Changed light by section, as vanilla's light engine reports it to
/// `LevelRenderer` (`setSectionDirty`) instead of cell by cell: for each
/// section (x, section index, z), a bit per class of position the changed
/// cells take in it, `3 * 3 * 3` classes for the low face, the inside and the
/// high face of each axis. Meshes sample light one block across a face, so the
/// class says which neighbouring sections a change dirties too.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LightChanges {
    sections: FxHashMap<Pos, u32>,
    /// The column the section indexes count in.
    layout: ColumnLayout,
}
impl LightChanges {
    /// Changes in chunks of `layout`: what the relighter or a light job adds
    /// after this (they all come from one level).
    fn keyed(&mut self, layout: ColumnLayout) {
        debug_assert!(self.sections.is_empty() || self.layout == layout);
        self.layout = layout;
    }
    /// A cell whose stored light changed.
    #[inline]
    pub fn push(&mut self, (x, y, z): Pos) {
        let key = (x >> 4, (y - self.layout.min_y()) >> 4, z >> 4);
        *self.sections.entry(key).or_default() |= 1 << Self::class(x, y, z);
    }
    /// Cells of section `key` (x, section index, z) in the given classes.
    fn add(&mut self, key: Pos, classes: u32) {
        *self.sections.entry(key).or_default() |= classes;
    }
    /// The class of a cell in its section: per axis 0 on the low face, 1
    /// inside, 2 on the high face.
    fn class(x: i32, y: i32, z: i32) -> u32 {
        let axis = |v: i32| match v & 15 {
            0 => 0,
            15 => 2,
            _ => 1,
        };
        axis(x) + 3 * axis(y) + 9 * axis(z)
    }
    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }
    /// The sections (x, section index, z) holding changed cells.
    pub fn sections(&self) -> impl Iterator<Item = Pos> + '_ {
        self.sections.keys().copied()
    }
    /// Every section (x, section index, z) whose cells lie within one block
    /// of a changed cell, section indexes outside the column left out (a
    /// section next to several changed ones comes once for each).
    pub fn dirty_sections(&self, mut out: impl FnMut(Pos)) {
        // Per class, the bit set of neighbour offsets (dx, dy, dz in -1..=1,
        // bit (dx + 1) + 3 (dy + 1) + 9 (dz + 1)) its cells reach.
        const REACH: [u32; 27] = {
            let mut reach = [0; 27];
            let mut class = 0;
            while class < 27 {
                let axis = [class % 3, class / 3 % 3, class / 9];
                let mut offset = 0;
                while offset < 27 {
                    let d = [offset % 3, offset / 3 % 3, offset / 9];
                    // Every class reaches its own section (d = 1, offset 0);
                    // the low face also -1 (d = 0), the high face +1 (d = 2).
                    if (d[0] == 1 || d[0] == axis[0])
                        && (d[1] == 1 || d[1] == axis[1])
                        && (d[2] == 1 || d[2] == axis[2])
                    {
                        reach[class] |= 1 << offset;
                    }
                    offset += 1;
                }
                class += 1;
            }
            reach
        };
        let sections = self.layout.sections() as i32;
        for (&(x, sy, z), &classes) in &self.sections {
            let mut offsets = 0;
            let mut rest = classes;
            while rest != 0 {
                offsets |= REACH[rest.trailing_zeros() as usize];
                rest &= rest - 1;
            }
            while offsets != 0 {
                let o = offsets.trailing_zeros() as i32;
                offsets &= offsets - 1;
                let s = sy + o / 3 % 3 - 1;
                if (0..sections).contains(&s) {
                    out((x + o % 3 - 1, s, z + o / 9 - 1));
                }
            }
        }
    }
}
fn level(world: &mut Region, sky: bool, p: Pos) -> Option<u8> {
    world
        .light(p.0, p.1, p.2)
        .map(|v| if sky { v >> 4 } else { v & 15 })
}
fn set_level(world: &mut Region, sky: bool, p: Pos, value: u8) {
    world.update_light(p.0, p.1, p.2, |old| {
        if sky {
            (old & 0x0F) | value << 4
        } else {
            (old & 0xF0) | value
        }
    });
}
/// `ChunkSkyLightSources`: the Y of the upper block of the highest occluded
/// edge in the column (every block at or above it is a sky source).
fn lowest_sky_source(world: &mut Region, tables: &LightTables, x: i32, z: i32) -> i32 {
    let (min_y, max_y) = (world.layout.min_y(), world.layout.max_y());
    let mut y = max_y - 1;
    let Some(mut top) = world.state(x, y, z) else {
        return min_y;
    };
    while y > min_y {
        // A section of one state that lets light through itself (the air of
        // a tall column) holds no source edge: go to its bottom block.
        if let Some(state) = world.uniform_state(x, y - 1, z)
            && state == top
            && !tables.edge_occluded(top as usize, top as usize)
        {
            y = ((y - 1 - min_y) & !15) + min_y;
            continue;
        }
        let Some(bottom) = world.state(x, y - 1, z) else {
            break;
        };
        if tables.edge_occluded(top as usize, bottom as usize) {
            return y;
        }
        top = bottom;
        y -= 1;
    }
    min_y
}
/// Vanilla's `checkBlock` + `runLightUpdates` for changed positions on
/// resident, lit chunks: clear the light that may have come through or from
/// the old states (decrease pass), then re-seed sources and propagate back in
/// from the brighter boundary (increase pass). Adds every position whose
/// stored light changed to `changed`, for remeshing. `force` clears around each position
/// as if it had been fully lit, for light computed from outdated blocks.
pub fn relight(
    world: &mut impl LightWorld,
    tables: &LightTables,
    sky_light: bool,
    positions: &[Pos],
    force: bool,
    changed: &mut LightChanges,
) {
    let mut region = Region::new(world);
    changed.keyed(region.layout);
    relight_region(&mut region, tables, sky_light, positions, force, changed);
    region.finish();
}
/// The columns `changed` touched that border a loaded column not lit yet
/// (`unlit`). `relight` reads and writes only lit columns, so an edit's
/// darkening can take away light such a neighbour sends in and never put it
/// back, and nothing relights the column when that neighbour's own light
/// lands: an air block at 0 beside one at 6 (#2786). Vanilla's light engine
/// keeps one light storage for every chunk, so its edits and its chunk
/// lighting see each other. These columns need a full light job, which
/// lights them from the blocks of their whole 3x3, that neighbour's included.
pub fn beside_unlit(changed: &LightChanges, unlit: impl Fn(i32, i32) -> bool) -> Vec<(i32, i32)> {
    let mut columns: Vec<(i32, i32)> = changed.sections().map(|(x, _, z)| (x, z)).collect();
    columns.sort_unstable();
    columns.dedup();
    columns.retain(|&(x, z)| {
        (-1..=1).any(|dz| (-1..=1).any(|dx| (dx, dz) != (0, 0) && unlit(x + dx, z + dz)))
    });
    columns
}
fn relight_region(
    world: &mut Region,
    tables: &LightTables,
    sky_light: bool,
    positions: &[Pos],
    force: bool,
    changed: &mut LightChanges,
) {
    crate::spans::time("lt_block", || {
        relight_layer(world, tables, false, positions.to_vec(), force, changed)
    });
    if sky_light {
        let (min_y, max_y) = (world.layout.min_y(), world.layout.max_y());
        let mut seeds = positions.to_vec();
        let mut columns: FxHashMap<(i32, i32), i32> = FxHashMap::default();
        crate::spans::time("lt_columns", || {
            for &(x, _, z) in positions {
                if columns.contains_key(&(x, z)) {
                    continue;
                }
                let source = lowest_sky_source(world, tables, x, z);
                columns.insert((x, z), source);
                // Cells whose source status changed: non-15 at or above the new
                // lowest source, and old 15s (sources) below it.
                for y in source..max_y {
                    if level(world, true, (x, y, z)).is_some_and(|l| l != 15) {
                        seeds.push((x, y, z));
                    }
                }
                let mut y = source - 1;
                while y >= min_y && level(world, true, (x, y, z)) == Some(15) {
                    seeds.push((x, y, z));
                    y -= 1;
                }
            }
        });
        crate::spans::time("lt_sky", || {
            relight_layer(world, tables, true, seeds, force, changed)
        });
    }
}
fn relight_layer(
    world: &mut Region,
    tables: &LightTables,
    sky: bool,
    seeds: Vec<Pos>,
    force: bool,
    changed: &mut LightChanges,
) {
    let mut decrease: VecDeque<(Pos, u8)> = VecDeque::new();
    let mut cleared: Vec<Pos> = Vec::new();
    let mut increase: VecDeque<Pos> = VecDeque::new();
    for p in seeds {
        let Some(old) = level(world, sky, p) else {
            continue;
        };
        // `BlockLightEngine.checkNode`: a block emitting at least the light
        // already there keeps it and pulls its neighbours' light in. Clearing
        // it would darken and relight everything it lit (flowing lava next to
        // lava: thousands of cells per block).
        if !sky && !force && old > 0 {
            let emission = world
                .state(p.0, p.1, p.2)
                .map_or(0, |s| tables.cells[s as usize].emission);
            if emission >= old {
                if emission > old {
                    set_level(world, sky, p, emission);
                    changed.push(p);
                }
                decrease.push_back((p, 0));
                increase.push_back(p);
                continue;
            }
        }
        if old > 0 {
            set_level(world, sky, p, 0);
            changed.push(p);
        }
        cleared.push(p);
        decrease.push_back((p, if force { 15 } else { old }));
    }
    while let Some((p, from)) = decrease.pop_front() {
        for (ox, oy, oz) in OFFSETS {
            let q = (p.0 + ox, p.1 + oy, p.2 + oz);
            let Some(l) = level(world, sky, q) else {
                continue;
            };
            if l == 0 {
                continue;
            }
            // Sky light runs straight down without loss, so a 15 below a
            // cleared 15 may have come from it too.
            if l < from || (sky && oy == -1 && l == 15 && from == 15) {
                set_level(world, sky, q, 0);
                changed.push(q);
                cleared.push(q);
                decrease.push_back((q, l));
            } else {
                increase.push_back(q);
            }
        }
    }
    // Sources inside the cleared area light up again.
    let mut sources: FxHashMap<(i32, i32), i32> = FxHashMap::default();
    crate::spans::time("lt_sources", || {
        for p in cleared {
            let seed = if sky {
                let lowest = *sources
                    .entry((p.0, p.2))
                    .or_insert_with(|| lowest_sky_source(world, tables, p.0, p.2));
                if p.1 >= lowest { 15 } else { 0 }
            } else {
                world
                    .state(p.0, p.1, p.2)
                    .map_or(0, |s| tables.cells[s as usize].emission)
            };
            if seed > level(world, sky, p).unwrap_or(15) {
                set_level(world, sky, p, seed);
                changed.push(p);
                increase.push_back(p);
            }
        }
    });
    while let Some(p) = increase.pop_front() {
        let Some((from, packed)) = world.cell(p.0, p.1, p.2) else {
            continue;
        };
        let l = if sky { packed >> 4 } else { packed & 15 };
        if l <= 1 {
            continue;
        }
        for (d, (ox, oy, oz)) in OFFSETS.iter().enumerate() {
            let q = (p.0 + ox, p.1 + oy, p.2 + oz);
            let Some((to, packed)) = world.cell(q.0, q.1, q.2) else {
                continue;
            };
            let ql = if sky { packed >> 4 } else { packed & 15 };
            let next = l.saturating_sub(tables.cells[to as usize].dampening.max(1));
            if next > ql && !tables.blocked(from as usize, to as usize, d) {
                set_level(world, sky, q, next);
                changed.push(q);
                increase.push_back(q);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{HEIGHT, MIN_Y, SECTIONS};

    const AIR: u32 = 0;
    const STONE: u32 = 1;
    const TORCH: u32 = 2;
    const SLAB: u32 = 3;
    /// Air, stone, torch and a bottom slab (full bottom face, half sides,
    /// empty top), with a third face shape for the half sides.
    fn tables() -> LightTables {
        let cell = |dampening, emission, faces: [u16; 6]| Cell {
            dampening,
            emission,
            shaped: faces.iter().any(|f| *f != EMPTY),
            faces,
        };
        LightTables::from_parts(
            vec![
                cell(0, 0, [EMPTY; 6]),
                cell(15, 0, [FULL; 6]),
                cell(0, 14, [EMPTY; 6]),
                cell(0, 0, [FULL, EMPTY, 2, 2, 2, 2]),
            ],
            // empty, full, half: only pairs with a full face cover the square.
            vec![false, true, false, true, true, true, false, true, false],
            3,
        )
    }
    fn world(edit: impl Fn(&mut Chunk)) -> HashMap<(i32, i32), Arc<Chunk>> {
        world_of(crate::chunk::ColumnLayout::VANILLA, edit)
    }
    /// Lit chunks around (0, 0) of `layout`, stone up to y 0.
    fn world_of(
        layout: crate::chunk::ColumnLayout,
        edit: impl Fn(&mut Chunk),
    ) -> HashMap<(i32, i32), Arc<Chunk>> {
        let mut chunks = HashMap::new();
        for z in -1..=1 {
            for x in -1..=1 {
                let mut c = Chunk::with_layout(x, z, layout);
                for y in layout.min_y()..0 {
                    for i in 0..256 {
                        c.set(i & 15, y, i >> 4, STONE);
                    }
                }
                if (x, z) == (0, 0) {
                    edit(&mut c);
                }
                chunks.insert((x, z), c);
            }
        }
        let lit: Vec<_> = chunks
            .keys()
            .map(|&(x, z)| {
                let region: [[Option<&Chunk>; 3]; 3] = std::array::from_fn(|dz| {
                    std::array::from_fn(|dx| chunks.get(&(x + dx as i32 - 1, z + dz as i32 - 1)))
                });
                let mut c = chunks[&(x, z)].clone();
                apply_lighting(&mut c, light_chunk(&region, &tables(), true));
                c.lit = true;
                ((x, z), Arc::new(c))
            })
            .collect();
        lit.into_iter().collect()
    }
    fn sky(world: &HashMap<(i32, i32), Arc<Chunk>>, x: i32, y: i32, z: i32) -> u8 {
        world[&(x >> 4, z >> 4)].light(x & 15, y, z & 15) >> 4
    }
    fn block(world: &HashMap<(i32, i32), Arc<Chunk>>, x: i32, y: i32, z: i32) -> u8 {
        world[&(x >> 4, z >> 4)].light(x & 15, y, z & 15) & 15
    }

    #[test]
    fn torch_light_falls_off_by_one_per_block() {
        let w = world(|c| {
            c.set(8, 0, 8, TORCH);
        });
        assert_eq!(block(&w, 8, 0, 8), 14);
        assert_eq!(block(&w, 9, 0, 8), 13);
        assert_eq!(block(&w, 8, 3, 8), 11);
        assert_eq!(block(&w, 8, -1, 8), 0);
    }
    #[test]
    fn bottom_slab_takes_sky_from_above_but_not_through_its_bottom() {
        // A slab roof over a closed stone room at y 0..2.
        let w = world(|c| {
            for x in 4..13 {
                for z in 4..13 {
                    c.set(x, 3, z, SLAB);
                    for y in 0..3 {
                        let wall = x == 4 || x == 12 || z == 4 || z == 12;
                        c.set(x, y, z, if wall { STONE } else { AIR });
                    }
                }
            }
        });
        assert_eq!(
            sky(&w, 8, 3, 8),
            15,
            "slab cell is lit through its open top"
        );
        // Without the shape test the room would be lit (slabs have no dampening).
        assert_eq!(sky(&w, 8, 2, 8), 0, "the full bottom face stops sky light");
    }
    /// #2786: (1, 0) is loaded but its own light job hasn't landed; (0, 0)
    /// was lit from the 3x3's blocks, its torch's light and the far torch's
    /// in (1, 0) both. Taking its torch away darkens everything the torch
    /// lit, the far torch's light at the border too, and `relight` cannot
    /// bring that back from a column it doesn't read. Columns like that are
    /// lit again whole; without it the border stays at 0 for good once
    /// (1, 0)'s light lands beside it.
    #[test]
    fn an_edit_beside_a_column_not_lit_yet_lights_the_column_again() {
        let tables = tables();
        let mut w: HashMap<(i32, i32), Arc<Chunk>> = HashMap::new();
        for z in -1..=1 {
            for x in -1..=1 {
                let mut c = Chunk::empty(x, z);
                for y in MIN_Y..0 {
                    for i in 0..256 {
                        c.set(i & 15, y, i >> 4, STONE);
                    }
                }
                if (x, z) == (0, 0) {
                    c.set(14, 0, 8, TORCH);
                } else if (x, z) == (1, 0) {
                    c.set(6, 0, 8, TORCH);
                }
                w.insert((x, z), Arc::new(c));
            }
        }
        let light = |w: &HashMap<(i32, i32), Arc<Chunk>>, (x, z): (i32, i32)| {
            let region: [[Option<&Chunk>; 3]; 3] = std::array::from_fn(|dz| {
                std::array::from_fn(|dx| {
                    w.get(&(x + dx as i32 - 1, z + dz as i32 - 1)).map(|c| &**c)
                })
            });
            light_chunk(&region, &tables, true)
        };
        let keys: Vec<_> = w.keys().copied().collect();
        for key in &keys {
            let lights = light(&w, *key);
            let c = Arc::make_mut(w.get_mut(key).unwrap());
            apply_lighting(c, lights);
            c.lit = *key != (1, 0);
        }
        // The far torch at x 22 gives 7 at x 15; (0, 0)'s own torch gives 13.
        assert_eq!(block(&w, 15, 0, 8), 13);

        Arc::make_mut(w.get_mut(&(0, 0)).unwrap()).set(14, 0, 8, AIR);
        let mut changed = LightChanges::default();
        relight(&mut w, &tables, true, &[(14, 0, 8)], false, &mut changed);
        let again = beside_unlit(&changed, |x, z| w.get(&(x, z)).is_some_and(|c| !c.lit));
        for &key in &again {
            let lights = light(&w, key);
            let c = Arc::make_mut(w.get_mut(&key).unwrap());
            apply_relighting(c, lights, &mut LightChanges::default());
        }
        // (1, 0)'s own job lands.
        let lights = light(&w, (1, 0));
        let c = Arc::make_mut(w.get_mut(&(1, 0)).unwrap());
        apply_lighting(c, lights);
        c.lit = true;
        assert_eq!(block(&w, 16, 0, 8), 8);
        assert_eq!(block(&w, 15, 0, 8), 7, "air beside 8 across the border");
        assert_eq!(block(&w, 14, 0, 8), 6);
        assert!(again.contains(&(0, 0)), "{again:?}");
    }
    #[test]
    fn relight_keeps_unchanged_and_unlit_sections() {
        let mut w = world(|_| {});
        Arc::make_mut(w.get_mut(&(1, 0)).unwrap()).lit = false;
        let before = w.clone();
        let same = |w: &HashMap<(i32, i32), Arc<Chunk>>, key| {
            w[&key]
                .sections
                .iter()
                .zip(&before[&key].sections)
                .all(|(a, b)| Arc::ptr_eq(a, b))
        };
        // Cleared and lit back to the same values: nothing is stored.
        relight(
            &mut w,
            &tables(),
            true,
            &[(8, 5, 8)],
            false,
            &mut LightChanges::default(),
        );
        assert!(w.keys().all(|&key| same(&w, key)));
        // Light reaching an unlit chunk isn't written into it.
        Arc::make_mut(w.get_mut(&(0, 0)).unwrap()).set(15, 0, 8, TORCH);
        relight(
            &mut w,
            &tables(),
            true,
            &[(15, 0, 8)],
            false,
            &mut LightChanges::default(),
        );
        assert!(same(&w, (1, 0)));
        assert_eq!(block(&w, 14, 0, 8), 13);
    }
    /// Emitters placed in light at most as bright as theirs (flowing lava
    /// next to lava) only brighten: nothing they lit is darkened and lit
    /// again, and the result is the full relight's.
    #[test]
    fn emitters_placed_in_their_own_light_only_brighten() {
        let mut w = world(|c| {
            c.set(8, 0, 8, TORCH);
        });
        let block = |w: &HashMap<(i32, i32), Arc<Chunk>>, p: Pos| {
            w[&(p.0 >> 4, p.2 >> 4)].light(p.0 & 15, p.1, p.2 & 15) & 15
        };
        for (x, y, z) in [(9, 0, 8), (10, 0, 8), (10, 1, 8), (11, 1, 9), (8, 0, 9)] {
            let around: Vec<Pos> = (-15..=15)
                .flat_map(|dx| {
                    (-3..=3).flat_map(move |dy| (-15..=15).map(move |dz| (x + dx, y + dy, z + dz)))
                })
                .collect();
            let before: Vec<u8> = around.iter().map(|&p| block(&w, p)).collect();
            Arc::make_mut(w.get_mut(&(0, 0)).unwrap()).set(x, y, z, TORCH);
            let mut changed = LightChanges::default();
            relight(&mut w, &tables(), true, &[(x, y, z)], false, &mut changed);
            for (&p, &was) in around.iter().zip(&before) {
                assert!(block(&w, p) >= was, "{p:?} darkened from {was}");
            }
        }
        // An emitter checked again with its block light unchanged changes
        // nothing, so no section is remeshed.
        let mut changed = LightChanges::default();
        relight(&mut w, &tables(), false, &[(8, 0, 8)], false, &mut changed);
        assert!(changed.is_empty());
        let region: [[Option<&Chunk>; 3]; 3] = std::array::from_fn(|dz| {
            std::array::from_fn(|dx| w.get(&(dx as i32 - 1, dz as i32 - 1)).map(|c| &**c))
        });
        for (si, s) in light_chunk(&region, &tables(), true).iter().enumerate() {
            for i in 0..4096 {
                assert_eq!(
                    s.get(i),
                    w[&(0, 0)].sections[si].light(i),
                    "section {si} index {i}"
                );
            }
        }
    }
    #[test]
    fn random_border_relights_match_full_relight() {
        // Edits around every border of chunk (0, 0), at the top and bottom of
        // the world, in batches; a held clone of the chunk must not change.
        let mut w = world(|_| {});
        let held = w[&(0, 0)].clone();
        let mut rng = 0x9e37_79b9_u32;
        let mut next = |n: usize| {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            rng as usize % n
        };
        let xz = [-2, -1, 0, 1, 7, 14, 15, 16, 17];
        let ys = [
            MIN_Y,
            MIN_Y + 1,
            -2,
            -1,
            0,
            1,
            15,
            16,
            MIN_Y + HEIGHT - 2,
            MIN_Y + HEIGHT - 1,
        ];
        for _ in 0..60 {
            let positions: Vec<Pos> = (0..1 + next(6))
                .map(|_| (xz[next(xz.len())], ys[next(ys.len())], xz[next(xz.len())]))
                .collect();
            for &(x, y, z) in &positions {
                let state = [TORCH, SLAB, AIR, STONE][next(4)];
                Arc::make_mut(w.get_mut(&(x >> 4, z >> 4)).unwrap()).set(x & 15, y, z & 15, state);
            }
            relight(
                &mut w,
                &tables(),
                true,
                &positions,
                false,
                &mut LightChanges::default(),
            );
        }
        // Every chunk, the outer ring lit as `world` lit it (no neighbours
        // past the 3x3).
        for (&(cx, cz), chunk) in &w {
            let region: [[Option<&Chunk>; 3]; 3] = std::array::from_fn(|dz| {
                std::array::from_fn(|dx| {
                    w.get(&(cx + dx as i32 - 1, cz + dz as i32 - 1))
                        .map(|c| &**c)
                })
            });
            for (si, s) in light_chunk(&region, &tables(), true).iter().enumerate() {
                for i in 0..4096 {
                    assert_eq!(
                        s.get(i),
                        chunk.sections[si].light(i),
                        "chunk {cx},{cz} section {si} index {i}"
                    );
                }
            }
        }
        let fresh = world(|_| {});
        for (a, b) in held.sections.iter().zip(&fresh[&(0, 0)].sections) {
            assert!((0..4096).all(|i| a.light(i) == b.light(i)));
        }
    }
    #[test]
    fn a_tall_column_is_lit_to_its_top() {
        let layout = crate::chunk::ColumnLayout::new(-64, 2096).unwrap();
        let mut chunk = Chunk::with_layout(0, 0, layout);
        for i in 0..256 {
            chunk.set(i & 15, 1600, i >> 4, STONE);
        }
        chunk.set(8, 1500, 8, TORCH);
        let region = [[None; 3], [None, Some(&chunk), None], [None; 3]];
        let lights = light_chunk(&region, &tables(), true);
        assert_eq!(lights.len(), 131);
        apply_lighting(&mut chunk, lights);
        assert_eq!(chunk.light(8, 1500, 8) & 15, 14);
        assert_eq!(chunk.light(8, 1501, 8) & 15, 13);
        assert_eq!(chunk.light(8, 1601, 8) >> 4, 15);
        assert_eq!(chunk.light(8, 1599, 8) >> 4, 0, "under the roof");
        assert_eq!(chunk.light(8, 2031, 8), 0xF0);
        // The empty sections above are one shared allocation.
        let top = chunk.sections.len() - 1;
        assert!(Arc::ptr_eq(&chunk.sections[top], &chunk.sections[top - 10]));
        let other = Chunk::with_layout(1, 0, layout);
        apply_lighting(&mut other.clone(), vec![Light::Uniform(0xF0); 131]);
        let mut lit = other.clone();
        apply_lighting(&mut lit, vec![Light::Uniform(0xF0); 131]);
        assert!(Arc::ptr_eq(&chunk.sections[top], &lit.sections[top]));
    }
    #[test]
    fn a_tall_column_relights_as_its_full_light_and_keys_its_sections() {
        let layout = crate::chunk::ColumnLayout::new(-64, 2096).unwrap();
        let mut w = world_of(layout, |c| {
            for i in 0..256 {
                c.set(i & 15, 1900, i >> 4, STONE);
            }
        });
        let edits = [
            ((8, 1800, 8), TORCH),
            ((8, 1900, 8), AIR),
            ((3, 2020, 3), TORCH),
        ];
        let mut changed = LightChanges::default();
        for ((x, y, z), state) in edits {
            Arc::make_mut(w.get_mut(&(0, 0)).unwrap()).set(x, y, z, state);
            relight(&mut w, &tables(), true, &[(x, y, z)], false, &mut changed);
        }
        let region: [[Option<&Chunk>; 3]; 3] = std::array::from_fn(|dz| {
            std::array::from_fn(|dx| w.get(&(dx as i32 - 1, dz as i32 - 1)).map(|c| &**c))
        });
        let full = light_chunk(&region, &tables(), true);
        assert_eq!(full.len(), 131);
        for (si, s) in full.iter().enumerate() {
            for i in 0..4096 {
                assert_eq!(
                    s.get(i),
                    w[&(0, 0)].sections[si].light(i),
                    "section {si} index {i}"
                );
            }
        }
        // y 2020 is section 130 of the tall column, the top one.
        assert!(
            changed
                .sections()
                .any(|(x, sy, z)| (x, sy, z) == (0, 130, 0))
        );
        let mut dirty = Vec::new();
        changed.dirty_sections(|s| dirty.push(s.1));
        assert!(dirty.iter().all(|&sy| (0..131).contains(&sy)), "{dirty:?}");
    }
    #[test]
    fn incremental_relight_matches_full_relight() {
        let mut w = world(|c| {
            c.set(8, 0, 8, TORCH);
        });
        let edits = [
            ((8, 0, 8), AIR),
            ((9, 1, 8), STONE),
            ((3, 2, 3), TORCH),
            ((8, 5, 8), SLAB),
            ((8, -1, 8), AIR),
        ];
        for ((x, y, z), state) in edits {
            Arc::make_mut(w.get_mut(&(0, 0)).unwrap()).set(x, y, z, state);
            relight(
                &mut w,
                &tables(),
                true,
                &[(x, y, z)],
                false,
                &mut LightChanges::default(),
            );
        }
        let region: [[Option<&Chunk>; 3]; 3] = std::array::from_fn(|dz| {
            std::array::from_fn(|dx| w.get(&(dx as i32 - 1, dz as i32 - 1)).map(|c| &**c))
        });
        let full = light_chunk(&region, &tables(), true);
        for (si, s) in full.iter().enumerate() {
            for i in 0..4096 {
                assert_eq!(
                    s.get(i),
                    w[&(0, 0)].sections[si].light(i),
                    "section {si} index {i}"
                );
            }
        }
    }
    #[test]
    fn lighting_result_preserves_current_edits_and_snapshots() {
        let mut current = Chunk::empty(0, 0);
        current.set(8, MIN_Y, 8, TORCH);
        let region = [[None; 3], [None, Some(&current), None], [None; 3]];
        let lights = light_chunk(&region, &tables(), true);
        let Light::Full(data) = &lights[0] else {
            panic!("torch light varies");
        };
        let result_storage = data.as_ptr();
        let snapshot = current.clone();
        current.set(8, MIN_Y, 8, STONE);
        Arc::make_mut(&mut current.sections[0]).biomes.set(0, 3);
        let edited_snapshot = current.clone();
        apply_lighting(&mut current, lights);
        let Light::Full(data) = &current.sections[0].light else {
            panic!("torch light varies");
        };
        assert_eq!(
            result_storage,
            data.as_ptr(),
            "returned light storage moves without cloning"
        );
        current.lit = true;
        assert_eq!(current.get(8, MIN_Y, 8), STONE);
        assert_eq!(current.sections[0].biomes.get(0), 3);
        assert_eq!(snapshot.get(8, MIN_Y, 8), TORCH);
        assert_eq!(edited_snapshot.sections[0].light(index(8, MIN_Y, 8)), 0xF0);
        let mut world = HashMap::from([((0, 0), Arc::new(current))]);
        // Streaming replays edits made after the job snapshot with force=true.
        let mut changed = LightChanges::default();
        relight(
            &mut world,
            &tables(),
            true,
            &[(8, MIN_Y, 8)],
            true,
            &mut changed,
        );
        assert!(!changed.is_empty());
        let region = [[None; 3], [None, Some(&*world[&(0, 0)]), None], [None; 3]];
        let fresh = light_chunk(&region, &tables(), true);
        for (section, light) in world[&(0, 0)].sections.iter().zip(fresh) {
            for i in 0..4096 {
                assert_eq!(section.light(i), light.get(i));
            }
        }
    }

    #[test]
    fn relighting_reports_exactly_the_changed_positions() {
        let mut chunk = Chunk::empty(1, -1);
        chunk.set(3, MIN_Y + 20, 5, TORCH);
        let lit = |c: &Chunk| {
            let region = [[None; 3], [None, Some(c), None], [None; 3]];
            light_chunk(&region, &tables(), true)
        };
        let light = lit(&chunk);
        apply_lighting(&mut chunk, light);
        let sections: Vec<_> = chunk.sections.iter().map(Arc::as_ptr).collect();
        // The same light again: nothing changes and no section is copied.
        let light = lit(&chunk);
        let mut changed = LightChanges::default();
        apply_relighting(&mut chunk, light, &mut changed);
        assert!(changed.is_empty());
        assert_eq!(
            sections,
            chunk.sections.iter().map(Arc::as_ptr).collect::<Vec<_>>()
        );
        let before = chunk.clone();
        chunk.set(3, MIN_Y + 20, 5, STONE);
        let light = lit(&chunk);
        apply_relighting(&mut chunk, light, &mut changed);
        let mut expected = LightChanges::default();
        for (sy, (old, new)) in before.sections.iter().zip(&chunk.sections).enumerate() {
            for i in 0..4096 {
                if old.light(i) != new.light(i) {
                    let (x, y, z) = (i as i32 & 15, (i >> 8) as i32, (i >> 4) as i32 & 15);
                    expected.push((16 + x, MIN_Y + sy as i32 * 16 + y, -16 + z));
                }
            }
        }
        assert!(!expected.is_empty());
        assert_eq!(changed, expected);
        // Only the torch's section and the two it lights into are copied.
        let copied = sections
            .iter()
            .zip(&chunk.sections)
            .filter(|(old, new)| **old != Arc::as_ptr(new))
            .count();
        assert!(copied <= 3, "{copied} sections copied");
    }

    #[test]
    fn changes_dirty_the_sections_within_a_block_of_each_cell() {
        // Cells inside sections, on each face and corner, at the bottom and
        // top of the world, on negative coordinates.
        let mut cells = Vec::new();
        for x in [-17, -16, -1, 0, 7, 15, 16, 31] {
            for y in [
                MIN_Y,
                MIN_Y + 15,
                MIN_Y + 16,
                70,
                79,
                80,
                MIN_Y + HEIGHT - 1,
            ] {
                for z in [-33, -32, 5, 15] {
                    cells.push((x, y, z));
                }
            }
        }
        for n in [1, 2, cells.len()] {
            for start in 0..cells.len().min(8) {
                let some: Vec<Pos> = cells.iter().copied().cycle().skip(start).take(n).collect();
                let mut changes = LightChanges::default();
                some.iter().for_each(|&c| changes.push(c));
                let mut dirty = std::collections::BTreeSet::new();
                changes.dirty_sections(|s| {
                    dirty.insert(s);
                });
                let mut expected = std::collections::BTreeSet::new();
                for &(x, y, z) in &some {
                    for (dx, dy, dz) in (-1..=1)
                        .flat_map(|a| (-1..=1).flat_map(move |b| (-1..=1).map(move |c| (a, b, c))))
                    {
                        let s = (y + dy - MIN_Y) >> 4;
                        if (0..SECTIONS as i32).contains(&s) {
                            expected.insert(((x + dx) >> 4, s, (z + dz) >> 4));
                        }
                    }
                }
                assert_eq!(dirty, expected, "{some:?}");
                let sections: std::collections::BTreeSet<_> = changes.sections().collect();
                let own: std::collections::BTreeSet<_> = some
                    .iter()
                    .map(|&(x, y, z)| (x >> 4, (y - MIN_Y) >> 4, z >> 4))
                    .collect();
                assert_eq!(sections, own);
            }
        }
    }

    #[test]
    fn stored_layers_fill_the_column_as_vanilla_reads_them() {
        // Section 10's sky layer is 7 on its bottom row and 3 above; no
        // layers anywhere else; a block layer of 5 in section 2.
        let mut sky = vec![0x33u8; 2048];
        sky[..128].fill(0x77);
        let mut layers: Vec<(Option<Vec<u8>>, Option<Vec<u8>>)> = vec![(None, None); SECTIONS];
        layers[10].0 = Some(sky);
        layers[2].1 = Some(vec![0x55; 2048]);
        let mut chunk = Chunk::empty(0, 0);
        light_from_layers(&mut chunk, &layers, [15; 256], true);
        let at = |c: &Chunk, si: usize, i: usize| c.sections[si].light(i);
        assert!(chunk.lit && chunk.light_loaded);
        assert!(
            matches!(chunk.sections[11].light, Light::Uniform(0xF0)),
            "above: open sky"
        );
        assert_eq!((at(&chunk, 10, 0), at(&chunk, 10, 4095)), (0x70, 0x30));
        // Below, each section repeats the bottom row of the stored one above.
        assert!(matches!(chunk.sections[9].light, Light::Uniform(0x70)));
        assert_eq!(at(&chunk, 2, 100), 0x75, "block light only where stored");
        assert!(matches!(chunk.sections[0].light, Light::Uniform(0x70)));

        // A light update replaces one kind of light in the sections it names.
        let mut update: Vec<(Option<Vec<u8>>, Option<Vec<u8>>)> = vec![(None, None); SECTIONS];
        update[2].1 = Some(vec![0; 2048]);
        update[3].0 = Some(vec![0x77; 2048]);
        let changed = update_light_layers(&mut chunk, &update, true);
        assert_eq!(changed, vec![2]);
        assert!(matches!(chunk.sections[2].light, Light::Uniform(0x70)));
        update[3].0 = Some(vec![0xFF; 2048]);
        assert_eq!(
            update_light_layers(&mut chunk, &update, false),
            Vec::<usize>::new(),
            "no sky storage"
        );
    }

    #[test]
    fn empty_chunk_missing_neighbours_and_no_sky() {
        let mut chunk = Chunk::empty(0, 0);
        for sky in [false, true] {
            let region = [[None; 3], [None, Some(&chunk), None], [None; 3]];
            let lights = light_chunk(&region, &tables(), sky);
            assert_eq!(lights.len(), SECTIONS);
            assert!(
                lights
                    .iter()
                    .all(|v| matches!(v, Light::Uniform(n) if *n == if sky { 0xF0 } else { 0 }))
            );
            apply_lighting(&mut chunk, lights);
            // Uniform sections are one shared allocation (`share_uniform`).
            let first = &chunk.sections[0];
            assert!(chunk.sections.iter().all(|s| Arc::ptr_eq(s, first)));
        }
    }

    // Original main's worker packaging and streaming application. Kept here
    // so the benchmark compares the real former copy/compaction work.
    fn baseline_pack(
        center: &Chunk,
        h: usize,
        sky: &[u8],
        block: &[u8],
    ) -> Vec<Arc<crate::chunk::Section>> {
        let at = |x: usize, y: usize, z: usize| (y * W + z) * W + x;
        let mut sections = Vec::with_capacity(SECTIONS);
        for (si, section) in center.sections.iter().enumerate() {
            let mut out = (**section).clone();
            if si * 16 >= h {
                out.light = Light::Uniform(0xF0);
            } else {
                let mut data = Box::new([0u8; 4096]);
                for ly in 0..16 {
                    let y = si * 16 + ly;
                    if y >= h {
                        for i in 0..256 {
                            data[ly << 8 | i] = 0xF0;
                        }
                        continue;
                    }
                    for lz in 0..16 {
                        for lx in 0..16 {
                            let p = at(16 + lx, y, 16 + lz);
                            data[index(lx as i32, ly as i32, lz as i32)] = sky[p] << 4 | block[p];
                        }
                    }
                }
                out.light = Light::Full(data);
            }
            out.compact();
            sections.push(Arc::new(out));
        }
        sections
    }
    fn baseline_apply(chunk: &mut Chunk, sections: Vec<Arc<crate::chunk::Section>>) {
        for (target, lit) in chunk.sections.iter_mut().zip(sections) {
            let mut section = (**target).clone();
            section.light = lit.light.clone();
            *target = Arc::new(section);
        }
    }

    #[test]
    #[ignore = "microbenchmark; run with --ignored --nocapture"]
    fn benchmark_worker_light_results() {
        use std::{hint::black_box, time::Instant};
        let mut fixture = Chunk::empty(0, 0);
        for (si, s) in fixture.sections.iter_mut().enumerate() {
            let section = Arc::make_mut(s);
            for i in 0..4096 {
                section.set(i, ((i + si) % 16) as u32);
            }
            for i in 0..64 {
                section.biomes.set(i, (i % 4) as u32);
            }
            section.light = Light::Full(Box::new([0x87; 4096]));
        }
        let h = HEIGHT as usize;
        let sky: Vec<_> = (0..W * W * h).map(|i| (i % 16) as u8).collect();
        let block: Vec<_> = (0..W * W * h).map(|i| ((i / 16) % 16) as u8).collect();
        let mut old = fixture.clone();
        let mut new = fixture.clone();
        baseline_apply(&mut old, baseline_pack(&fixture, h, &sky, &block));
        apply_lighting(&mut new, pack_lighting(SECTIONS, h, &sky, &block, true));
        for (a, b) in old.sections.iter().zip(&new.sections) {
            for i in 0..4096 {
                assert_eq!(a.light(i), b.light(i));
                assert_eq!(a.get(i), b.get(i));
            }
            for i in 0..64 {
                assert_eq!(a.biomes.get(i), b.biomes.get(i));
            }
        }
        for shared in [false, true] {
            let mut times = [Vec::new(), Vec::new()];
            for round in 0..11 {
                for optimized in [round % 2 == 0, round % 2 != 0] {
                    let mut current = fixture.clone();
                    let held = shared.then(|| current.clone());
                    // Warm up and detach for the uniquely-owned scenario.
                    apply_lighting(&mut current, pack_lighting(SECTIONS, h, &sky, &block, true));
                    let started = Instant::now();
                    for _ in 0..500 {
                        let retained = shared.then(|| current.clone());
                        if optimized {
                            apply_lighting(
                                black_box(&mut current),
                                pack_lighting(
                                    SECTIONS,
                                    h,
                                    black_box(&sky),
                                    black_box(&block),
                                    true,
                                ),
                            );
                        } else {
                            baseline_apply(
                                black_box(&mut current),
                                baseline_pack(
                                    black_box(&fixture),
                                    h,
                                    black_box(&sky),
                                    black_box(&block),
                                ),
                            );
                        }
                        black_box(retained);
                        black_box(&current);
                    }
                    if round > 0 {
                        times[usize::from(optimized)]
                            .push(started.elapsed().as_secs_f64() * 1e6 / 500.0);
                    }
                    black_box(held);
                }
            }
            for values in &mut times {
                values.sort_by(f64::total_cmp);
            }
            println!(
                "pack+apply shared={shared}: baseline {:.3} us optimized {:.3} us speedup {:.2}x",
                times[0][5],
                times[1][5],
                times[0][5] / times[1][5]
            );
        }
        let payload: usize = fixture.sections.iter().map(|s| s.heap_bytes()).sum();
        println!(
            "fixture heap payload {payload} bytes; old worker clones it plus 24 section Arcs; old application clones current payload and 98304 returned light bytes. New worker clones zero section payload; new application clones current payload only when shared."
        );
    }
    /// Relight cost for torches placed and removed near chunk borders with
    /// slabs flickering around them (fluids next to lava, in the game).
    /// `cargo test --profile fast -p horizon-client relight_bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn relight_bench() {
        let mut w: FxHashMap<_, _> = world(|_| {}).into_iter().collect();
        let t = tables();
        let mut rng = 0x2545_f491_u32;
        let mut next = |n: i32| {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            (rng % n as u32) as i32
        };
        let start = std::time::Instant::now();
        let mut changed = LightChanges::default();
        for round in 0..400 {
            let state = [TORCH, SLAB, AIR][round % 3];
            let positions: Vec<Pos> = (0..32)
                .map(|_| (next(32) - 8, next(12), next(32) - 8))
                .collect();
            for &(x, y, z) in &positions {
                Arc::make_mut(w.get_mut(&(x >> 4, z >> 4)).unwrap()).set(x & 15, y, z & 15, state);
            }
            relight(&mut w, &t, true, &positions, false, &mut changed);
        }
        println!(
            "relight_bench: {:?}, {} sections",
            start.elapsed(),
            changed.sections().count()
        );
    }

    #[test]
    #[ignore = "benchmark"]
    fn benchmark_light_chunk() {
        use std::time::Instant;
        let t = tables();
        let layout = ColumnLayout::VANILLA;
        let mut chunks = Vec::new();
        for cz in -1..=1 {
            for cx in -1..=1 {
                let mut c = Chunk::with_layout(cx, cz, layout);
                for y in layout.min_y()..64 {
                    for i in 0..256 {
                        c.set(i & 15, y, i >> 4, STONE);
                    }
                }
                for x in 2..14 {
                    for z in 2..14 {
                        c.set(x, 64, z, SLAB);
                    }
                }
                c.set(8, 65, 8, TORCH);
                c.set(4, 30, 4, TORCH);
                chunks.push(c);
            }
        }
        let region = [
            [Some(&chunks[0]), Some(&chunks[1]), Some(&chunks[2])],
            [Some(&chunks[3]), Some(&chunks[4]), Some(&chunks[5])],
            [Some(&chunks[6]), Some(&chunks[7]), Some(&chunks[8])],
        ];

        for _ in 0..5 {
            let _ = light_chunk(&region, &t, true);
        }

        let start = Instant::now();
        let iters = 200;
        for _ in 0..iters {
            let _ = std::hint::black_box(light_chunk(std::hint::black_box(&region), &t, true));
        }
        let elapsed = start.elapsed();
        println!(
            "benchmark_light_chunk: {:?} total, {:.2} µs/chunk",
            elapsed,
            elapsed.as_micros() as f64 / iters as f64
        );
    }

    #[test]
    fn scratch_queues_shrink_above_capacity_cap() {
        let mut scratch = LightScratch::new();
        scratch.queue.resize(100_000, 0);
        scratch.sky_queue.resize(100_000, 0);
        assert!(scratch.queue.capacity() >= 100_000);
        assert!(scratch.sky_queue.capacity() >= 100_000);
        scratch.clear_queues();
        assert_eq!(scratch.queue.len(), 0);
        assert_eq!(scratch.sky_queue.len(), 0);
        assert!(scratch.queue.capacity() < 100_000);
        assert!(scratch.sky_queue.capacity() < 100_000);
    }

    /// The baseline flood-fill lighting algorithm before the scratch and wavefront optimizations.
    fn old_light_chunk(
        region: &[[Option<&Chunk>; 3]; 3],
        tables: &LightTables,
        sky_light: bool,
    ) -> Vec<Light> {
        let sections = region[1][1].expect("centre chunk").sections.len();
        let mut top_section = 0;
        for chunk in region.iter().flatten().flatten() {
            for (i, s) in chunk.sections.iter().enumerate().rev() {
                if !s.is_uniform(0) {
                    top_section = top_section.max(i + 1);
                    break;
                }
            }
        }
        let h = ((top_section + 1) * 16).min(sections * 16);
        let at = |x: usize, y: usize, z: usize| (y * W + z) * W + x;
        let mut states = vec![0u16; W * W * h];
        let mut sky = vec![0u8; W * W * h];
        let mut block = vec![0u8; W * W * h];
        let mut queue = VecDeque::new();
        let mut sky_queue = VecDeque::new();
        let mut ids = Box::new([0u32; 4096]);
        for (dz, row) in region.iter().enumerate() {
            for (dx, chunk) in row.iter().enumerate() {
                let Some(chunk) = chunk else { continue };
                for (si, section) in chunk.sections.iter().enumerate().take(top_section) {
                    if section.is_uniform(0) {
                        continue;
                    }
                    section.unpack(&mut ids);
                    for (i, &id) in ids.iter().enumerate() {
                        let (lx, lz, ly) = (i & 15, (i >> 4) & 15, i >> 8);
                        let p = at(dx * 16 + lx, si * 16 + ly, dz * 16 + lz);
                        let info = tables.info[id as usize];
                        states[p] = info as u16;
                        let emission = info >> 16;
                        if emission > 0 {
                            block[p] = emission as u8;
                            queue.push_back(p as u32);
                        }
                    }
                }
            }
        }
        let mut present = vec![false; W * W];
        for z in 0..W {
            for x in 0..W {
                present[z * W + x] = region[z / 16][x / 16].is_some();
            }
        }
        let mut lowest = vec![h as i32; W * W];
        let old_packed_edge_occluded = |top: u16, bottom: u16| -> bool {
            bottom & 0xF != 0 || tables.packed_blocked(top, bottom, Direction::Down as usize)
        };
        for z in 0..W {
            for x in 0..W {
                if !present[z * W + x] || !sky_light {
                    continue;
                }
                let mut y = h as i32 - 1;
                sky[at(x, y as usize, z)] = 15;
                while y > 0
                    && !old_packed_edge_occluded(
                        states[at(x, y as usize, z)],
                        states[at(x, y as usize - 1, z)],
                    )
                {
                    y -= 1;
                    sky[at(x, y as usize, z)] = 15;
                }
                lowest[z * W + x] = y;
            }
        }
        for z in 0..W {
            for x in 0..W {
                if !present[z * W + x] || !sky_light {
                    continue;
                }
                let own = lowest[z * W + x];
                let mut deepest = own;
                for (nx, nz) in [
                    (x.wrapping_sub(1), z),
                    (x + 1, z),
                    (x, z.wrapping_sub(1)),
                    (x, z + 1),
                ] {
                    if nx < W && nz < W && present[nz * W + nx] {
                        deepest = deepest.max(lowest[nz * W + nx]);
                    }
                }
                let from = own.max(0) as usize;
                let to = (deepest as usize).min(h.saturating_sub(1));
                for y in from..=to {
                    if sky[at(x, y, z)] == 15 {
                        sky_queue.push_back(at(x, y, z) as u32);
                    }
                }
            }
        }
        let spread = |light: &mut [u8], queue: &mut VecDeque<u32>| {
            while let Some(p) = queue.pop_front() {
                let p = p as usize;
                let level = light[p];
                if level <= 1 {
                    continue;
                }
                let x = p % W;
                let z = (p / W) % W;
                let y = p / (W * W);
                let from = states[p];
                let col = z * W + x;
                macro_rules! neighbour {
                    ($d:expr, $q:expr, $ncol:expr) => {{
                        if present[$ncol] {
                            let q = $q;
                            let to = states[q];
                            let next = level.saturating_sub((to & 0xF).max(1) as u8);
                            if next > light[q] && !tables.packed_blocked(from, to, $d) {
                                light[q] = next;
                                queue.push_back(q as u32);
                            }
                        }
                    }};
                }
                if y > 0 {
                    neighbour!(0, p - W * W, col);
                }
                if y + 1 < h {
                    neighbour!(1, p + W * W, col);
                }
                if z > 0 {
                    neighbour!(2, p - W, col - W);
                }
                if z + 1 < W {
                    neighbour!(3, p + W, col + W);
                }
                if x > 0 {
                    neighbour!(4, p - 1, col - 1);
                }
                if x + 1 < W {
                    neighbour!(5, p + 1, col + 1);
                }
            }
        };
        spread(&mut sky, &mut sky_queue);
        spread(&mut block, &mut queue);
        pack_lighting(sections, h, &sky, &block, sky_light)
    }

    #[test]
    fn light_chunk_matches_old_algorithm() {
        let t = tables();
        let layout = ColumnLayout::VANILLA;

        let mut rng = 0x1234_5678_u64;
        let mut next = |max: usize| -> usize {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            ((rng >> 32) as usize) % max
        };

        let cases = [
            // (sky_light, missing_pattern)
            // 0 = full 3x3
            // 1 = missing corners
            // 2 = missing edges
            // 3 = random missing neighbours
            // 4 = only centre and single neighbour
            (true, 0),
            (false, 0),
            (true, 1),
            (false, 1),
            (true, 2),
            (false, 2),
            (true, 3),
            (false, 3),
            (true, 4),
            (false, 4),
        ];

        for (round, &(sky_light, pattern)) in cases.iter().enumerate() {
            let mut chunks = Vec::new();
            for cz in -1..=1 {
                for cx in -1..=1 {
                    let mut c = Chunk::with_layout(cx, cz, layout);
                    let base_y = -64 + next(64) as i32;
                    for y in layout.min_y()..base_y {
                        for i in 0..256 {
                            c.set(i & 15, y, i >> 4, STONE);
                        }
                    }
                    let features = 10 + next(20);
                    for _ in 0..features {
                        let fx = next(16) as i32;
                        let fz = next(16) as i32;
                        let fy = base_y + next(32) as i32;
                        let block_type = match next(4) {
                            0 => AIR,
                            1 => STONE,
                            2 => TORCH,
                            _ => SLAB,
                        };
                        c.set(fx, fy, fz, block_type);
                    }
                    let roof_y = base_y + 10 + next(10) as i32;
                    for rx in 4..12 {
                        for rz in 4..12 {
                            if next(3) != 0 {
                                c.set(rx, roof_y, rz, SLAB);
                            }
                        }
                    }
                    c.set(8, roof_y - 1, 8, TORCH);
                    c.set(next(16) as i32, roof_y + 2, next(16) as i32, TORCH);
                    c.set(0, base_y + 1, 0, TORCH);
                    c.set(15, base_y + 1, 15, TORCH);

                    chunks.push(c);
                }
            }

            let mut region: [[Option<&Chunk>; 3]; 3] = [[None; 3]; 3];
            for dz in 0..3 {
                for dx in 0..3 {
                    let is_centre = dz == 1 && dx == 1;
                    let chunk_ref = &chunks[dz * 3 + dx];
                    let include = if is_centre {
                        true
                    } else {
                        match pattern {
                            0 => true,
                            1 => (dz + dx) % 2 == 1,
                            2 => (dz + dx) % 2 == 0,
                            3 => next(10) < 6,
                            4 => dz == 1 && dx == 2,
                            _ => true,
                        }
                    };
                    if include {
                        region[dz][dx] = Some(chunk_ref);
                    }
                }
            }

            let old = old_light_chunk(&region, &t, sky_light);
            let new = light_chunk(&region, &t, sky_light);

            assert_eq!(
                old.len(),
                new.len(),
                "round {round}: section count mismatch"
            );
            for si in 0..old.len() {
                for i in 0..4096 {
                    assert_eq!(
                        new[si].get(i),
                        old[si].get(i),
                        "mismatch at round {round} (sky_light={sky_light}, pattern={pattern}), section {si}, index {i} (lx={}, ly={}, lz={}): new={:#04x}, old={:#04x}",
                        i & 15,
                        i >> 8,
                        (i >> 4) & 15,
                        new[si].get(i),
                        old[si].get(i)
                    );
                }
            }
        }
    }
}
