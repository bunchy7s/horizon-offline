//! The minimap extra (`docs/extras.md`): a 16x16 tile per loaded chunk with
//! each column's vanilla map colour (`MapColor`, as `MapItem.update` picks
//! it), height and water depth, and a 128x128 image composed from the tiles
//! around the player with the map item's height and water shading.
//!
//! Tiles are filled nearest first within a time budget and rescanned after
//! block changes; the image is recomposed only when the view moved or a tile
//! changed.
use crate::{
    blocks::{BlockFacts, Fluid, MAP_COLORS},
    chunk::{Chunk, MIN_Y, SECTIONS},
    fast_hash::FxHashMap,
    world::World,
};
use std::time::{Duration, Instant};

/// The image's side in texels.
pub const SIZE: usize = 128;
/// Marks a column the map cuts through (a wall at the player's height).
const WALL: u8 = 0x80;
/// Water's and lava's map colours (`MapColor.WATER`, `FIRE`).
const WATER: u8 = 12;
const LAVA: u8 = 4;
/// A tile whose chunk changed is rescanned at most this often.
const RESCAN: Duration = Duration::from_secs(1);

/// What the map shows of each column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// The top block, as map items show it.
    Surface,
    /// Underground, in the Nether or under a roof: the floor below this
    /// height (the player's head), and walls cut at it.
    Below(i32),
}

/// What the scan needs of each block state.
pub struct Palette {
    /// Map colour id, 0 for none.
    color: Vec<u8>,
    /// 0 none, 1 water, 2 lava; | 4 for air; | 8 when sturdy on top.
    flags: Vec<u8>,
}
const FLUID: u8 = 3;
const AIR: u8 = 4;
const STURDY_TOP: u8 = 8;
impl Palette {
    pub fn new(facts: &BlockFacts) -> Self {
        let mut color = Vec::with_capacity(facts.states.len());
        let mut flags = Vec::with_capacity(facts.states.len());
        for (i, f) in facts.states.iter().enumerate() {
            color.push(f.map_color);
            let mut b = match f.fluid {
                Fluid::None => 0,
                Fluid::Water => 1,
                Fluid::Lava => 2,
            };
            if facts.is_air(horizon_world::registry::BlockStateId(i as u32)) {
                b |= AIR;
            }
            // isFaceSturdy(UP, FULL): Direction.UP's ordinal is 1.
            if f.sturdy_full >> 1 & 1 != 0 {
                b |= STURDY_TOP;
            }
            flags.push(b);
        }
        Self { color, flags }
    }
    /// A palette from (colour, flags) per state id (tests).
    #[cfg(test)]
    fn from_parts(parts: &[(u8, u8)]) -> Self {
        Self {
            color: parts.iter().map(|p| p.0).collect(),
            flags: parts.iter().map(|p| p.1).collect(),
        }
    }
    #[inline]
    fn get(&self, id: u32) -> (u8, u8) {
        let i = id as usize;
        (
            self.color.get(i).copied().unwrap_or(0),
            self.flags.get(i).copied().unwrap_or(AIR),
        )
    }
}

#[derive(Clone)]
struct Tile {
    /// Map colour id per column (`z * 16 + x`), `| WALL` for cut walls.
    color: [u8; 256],
    height: [i16; 256],
    /// Water depth below the surface, for the map's water shading.
    depth: [u8; 256],
    revision: u64,
    layer: Layer,
    scanned: Instant,
}
/// `MapItem.update` for one column: the first block with a map colour
/// going down, the fluid's colour over a fluid unless the block is sturdy on
/// top (`getCorrectStateForFluidBlock`), and the fluid's depth.
fn scan_column(
    chunk: &Chunk,
    palette: &Palette,
    x: i32,
    z: i32,
    top: i32,
    layer: Layer,
) -> (u8, i16, u8) {
    let mut y = top;
    if let Layer::Below(head) = layer {
        let (color, flags) = palette.get(chunk.get(x, head, z));
        if flags & AIR == 0 && color != 0 && flags & FLUID == 0 {
            return (color | WALL, head as i16, 0);
        }
    }
    while y > MIN_Y {
        let (mut color, flags) = palette.get(chunk.get(x, y, z));
        if color == 0 {
            y -= 1;
            continue;
        }
        let mut depth = 0u8;
        let fluid = flags & FLUID;
        if fluid != 0 {
            let mut below = y - 1;
            while below > MIN_Y && palette.get(chunk.get(x, below, z)).1 & FLUID != 0 {
                depth = depth.saturating_add(1);
                below -= 1;
            }
            depth = depth.saturating_add(1);
            if flags & STURDY_TOP == 0 {
                color = if fluid == 2 { LAVA } else { WATER };
            }
        }
        return (color, y as i16, depth);
    }
    (0, MIN_Y as i16, 0)
}
/// The highest block of a chunk worth scanning from.
fn top_of(chunk: &Chunk, air: u32, layer: Layer) -> i32 {
    let surface = (0..SECTIONS)
        .rev()
        .find(|&s| {
            chunk
                .sections
                .get(s)
                .is_some_and(|sec| !sec.is_uniform(air))
        })
        .map_or(MIN_Y, |s| MIN_Y + s as i32 * 16 + 15);
    match layer {
        Layer::Surface => surface,
        Layer::Below(head) => surface.min(head),
    }
}
fn scan_tile(chunk: &Chunk, palette: &Palette, air: u32, layer: Layer, revision: u64) -> Tile {
    let top = top_of(chunk, air, layer);
    let mut tile = Tile {
        color: [0; 256],
        height: [MIN_Y as i16; 256],
        depth: [0; 256],
        revision,
        layer,
        scanned: Instant::now(),
    };
    for z in 0..16 {
        for x in 0..16 {
            let (c, h, d) = scan_column(chunk, palette, x, z, top, layer);
            let i = (z * 16 + x) as usize;
            tile.color[i] = c;
            tile.height[i] = h;
            tile.depth[i] = d;
        }
    }
    tile
}

/// Where the map looks from and how.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub x: f64,
    pub z: f64,
    /// The player's yaw in degrees (0 faces south).
    pub yaw: f32,
    /// The map turns with the player (forward is up), or north is up.
    pub rotate: bool,
    pub blocks_per_pixel: f32,
    pub round: bool,
}
impl View {
    /// Whether the image for `self` would look different from `other`'s.
    fn differs(&self, other: &View) -> bool {
        let moved = ((self.x - other.x).abs().max((self.z - other.z).abs()) as f32)
            >= self.blocks_per_pixel / 4.0;
        let turned = self.rotate && (self.yaw - other.yaw).abs() >= 0.5;
        moved
            || turned
            || self.rotate != other.rotate
            || self.round != other.round
            || self.blocks_per_pixel != other.blocks_per_pixel
    }
    /// The world's right and forward directions as the map's x and up.
    fn axes(&self) -> ([f64; 2], [f64; 2]) {
        if !self.rotate {
            return ([1.0, 0.0], [0.0, -1.0]);
        }
        let (s, c) = f64::from(self.yaw).to_radians().sin_cos();
        ([-c, -s], [-s, c])
    }
    /// Where north is on the map's rim, as a unit direction (x right, y down).
    pub fn north(&self) -> (f32, f32) {
        let (r, f) = self.axes();
        // The direction (0, -1) in the map's (right, -forward) basis.
        (-r[1] as f32, f[1] as f32)
    }
}

/// `MapColor.getColorFromPackedId` as RGBA.
fn packed_rgba(id: u8, brightness: u8) -> [u8; 4] {
    let id = usize::from(id);
    if id == 0 || id >= MAP_COLORS.len() {
        return [0; 4];
    }
    let modifier = [180u32, 220, 255, 135][usize::from(brightness & 3)];
    let col = MAP_COLORS[id];
    let ch = |s: u32| ((col >> s & 255) * modifier / 255) as u8;
    [ch(16), ch(8), ch(0), 255]
}
/// The map item's shading at scale 1: brighter facing a lower north
/// neighbour, darker facing a higher one, dithered; water by depth.
fn brightness(color: u8, height: i16, north: Option<i16>, depth: u8, parity: i32) -> u8 {
    if color & WALL != 0 {
        return 3;
    }
    let parity = f64::from(parity & 1);
    if color == WATER {
        let diff = f64::from(depth) * 0.1 + parity * 0.2;
        return if diff < 0.5 {
            2
        } else if diff > 0.9 {
            0
        } else {
            1
        };
    }
    let north = north.unwrap_or(height);
    let diff = f64::from(height - north) * 4.0 / 5.0 + (parity - 0.5) * 0.4;
    if diff > 0.6 {
        2
    } else if diff < -0.6 {
        0
    } else {
        1
    }
}
const UNKNOWN: [u8; 4] = [20, 20, 20, 200];
const BORDER: [u8; 4] = [10, 10, 10, 230];

#[derive(Default)]
pub struct Minimap {
    tiles: FxHashMap<(i32, i32), Tile>,
    palette: Option<Palette>,
    composed: Option<View>,
    /// Tiles changed since the image was composed.
    changed: bool,
    /// The composed image, `SIZE` x `SIZE` RGBA.
    pub image: Vec<u8>,
}
impl Minimap {
    /// Forget everything (a new world or dimension, or the minimap turned off).
    pub fn clear(&mut self) {
        let palette = self.palette.take();
        *self = Self {
            palette,
            ..Self::default()
        };
    }
    /// Fill and refresh tiles around chunk `center` within `radius` chunks,
    /// nearest first, until `budget` is spent. Returns the tiles scanned.
    pub fn update(
        &mut self,
        world: &World,
        facts: &BlockFacts,
        center: (i32, i32),
        radius: i32,
        layer: Layer,
        budget: Duration,
    ) -> usize {
        let palette = self.palette.get_or_insert_with(|| Palette::new(facts));
        let started = Instant::now();
        let air = world.air.0;
        // Tiles of unloaded or distant chunks go.
        self.tiles.retain(|k, _| {
            (k.0 - center.0).abs() <= radius + 2
                && (k.1 - center.1).abs() <= radius + 2
                && world.chunks.contains_key(k)
        });
        let mut scanned = 0;
        for ring in 0..=radius {
            for (dx, dz) in ring_offsets(ring) {
                let key = (center.0 + dx, center.1 + dz);
                let Some(chunk) = world.chunks.get(&key) else {
                    continue;
                };
                let stale = match self.tiles.get(&key) {
                    None => true,
                    Some(t) if t.layer != layer => true,
                    Some(t) => t.revision != world.revision && t.scanned.elapsed() >= RESCAN,
                };
                if !stale {
                    continue;
                }
                let tile = scan_tile(chunk, palette, air, layer, world.revision);
                if self.tiles.get(&key).is_none_or(|t| {
                    t.color != tile.color || t.height != tile.height || t.depth != tile.depth
                }) {
                    self.changed = true;
                }
                self.tiles.insert(key, tile);
                scanned += 1;
                if started.elapsed() >= budget {
                    return scanned;
                }
            }
        }
        scanned
    }
    /// Recompose the image if the view or the tiles changed; true when it did.
    pub fn compose(&mut self, view: View) -> bool {
        if !self.changed
            && self.composed.is_some_and(|c| !view.differs(&c))
            && !self.image.is_empty()
        {
            return false;
        }
        self.changed = false;
        self.composed = Some(view);
        self.image.resize(SIZE * SIZE * 4, 0);
        let (r, f) = view.axes();
        let bpp = f64::from(view.blocks_per_pixel);
        let half = SIZE as f64 / 2.0;
        let mut cached: Option<((i32, i32), &Tile)> = None;
        let mut column = |x: i32, z: i32| -> Option<(u8, i16, u8)> {
            let key = (x >> 4, z >> 4);
            let tile = match cached {
                Some((k, t)) if k == key => t,
                _ => {
                    let t = self.tiles.get(&key)?;
                    cached = Some((key, t));
                    t
                }
            };
            let i = ((z & 15) * 16 + (x & 15)) as usize;
            Some((tile.color[i], tile.height[i], tile.depth[i]))
        };
        for j in 0..SIZE {
            for i in 0..SIZE {
                let (u, v) = (i as f64 + 0.5 - half, j as f64 + 0.5 - half);
                let o = (j * SIZE + i) * 4;
                let edge = if view.round {
                    let d = (u * u + v * v).sqrt();
                    if d > half {
                        self.image[o..o + 4].copy_from_slice(&[0; 4]);
                        continue;
                    }
                    d > half - 1.5
                } else {
                    i == 0 || j == 0 || i == SIZE - 1 || j == SIZE - 1
                };
                if edge {
                    self.image[o..o + 4].copy_from_slice(&BORDER);
                    continue;
                }
                let wx = view.x + bpp * (u * r[0] - v * f[0]);
                let wz = view.z + bpp * (u * r[1] - v * f[1]);
                let (x, z) = (wx.floor() as i32, wz.floor() as i32);
                let rgba = match column(x, z) {
                    Some((color, height, depth)) if color & !WALL != 0 => {
                        let north = column(x, z - 1).filter(|c| c.0 & !WALL != 0).map(|c| c.1);
                        let b = brightness(color, height, north, depth, x + z);
                        packed_rgba(color & !WALL, b)
                    }
                    _ => UNKNOWN,
                };
                self.image[o..o + 4].copy_from_slice(&rgba);
            }
        }
        true
    }
    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }
}
/// Chunk offsets at Chebyshev distance `ring`.
fn ring_offsets(ring: i32) -> impl Iterator<Item = (i32, i32)> {
    (-ring..=ring).flat_map(move |dz| {
        (-ring..=ring)
            .filter(move |&dx| dx.abs() == ring || dz.abs() == ring)
            .map(move |dx| (dx, dz))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // States: 0 air, 1 grass (colour 1), 2 stone (11), 3 water, 4 glass (no
    // colour), 5 waterlogged slab (sturdy on top? no), 6 lava.
    fn palette() -> Palette {
        Palette::from_parts(&[
            (0, AIR),
            (1, STURDY_TOP),
            (11, STURDY_TOP),
            (WATER, 1),
            (0, 0),
            (11, 1),
            (LAVA, 2),
        ])
    }
    fn chunk(columns: &[(i32, i32, &[(i32, u32)])]) -> Chunk {
        let mut c = Chunk::empty(0, 0);
        for &(x, z, blocks) in columns {
            for &(y, id) in blocks {
                c.set(x, y, z, id);
            }
        }
        c
    }

    #[test]
    fn columns_take_the_first_coloured_block_and_fluids_over_it() {
        let p = palette();
        let c = chunk(&[
            (0, 0, &[(60, 2), (61, 1)]),
            // Glass has no map colour: the grass under it shows.
            (1, 0, &[(61, 1), (62, 4)]),
            // Three water over stone.
            (2, 0, &[(60, 2), (61, 3), (62, 3), (63, 3)]),
            (3, 0, &[(70, 6)]),
        ]);
        let top = top_of(&c, 0, Layer::Surface);
        assert_eq!(top, 79);
        assert_eq!(scan_column(&c, &p, 0, 0, top, Layer::Surface), (1, 61, 0));
        assert_eq!(scan_column(&c, &p, 1, 0, top, Layer::Surface), (1, 61, 0));
        assert_eq!(
            scan_column(&c, &p, 2, 0, top, Layer::Surface),
            (WATER, 63, 3)
        );
        assert_eq!(
            scan_column(&c, &p, 3, 0, top, Layer::Surface),
            (LAVA, 70, 1)
        );
        assert_eq!(scan_column(&c, &p, 4, 0, top, Layer::Surface).0, 0);
    }

    #[test]
    fn a_waterlogged_block_not_sturdy_on_top_shows_water() {
        let p = palette();
        let c = chunk(&[(0, 0, &[(60, 2), (61, 5)])]);
        assert_eq!(
            scan_column(&c, &p, 0, 0, 70, Layer::Surface),
            (WATER, 61, 1)
        );
    }

    #[test]
    fn below_a_height_shows_the_floor_and_cuts_walls() {
        let p = palette();
        // A cave: stone everywhere from 0 to 20, air at 10 and 11 in column 0.
        let mut blocks: Vec<(i32, u32)> = (0..=20).map(|y| (y, 2)).collect();
        let solid = blocks.clone();
        blocks.retain(|&(y, _)| y != 10 && y != 11);
        let c = chunk(&[(0, 0, &blocks), (1, 0, &solid)]);
        let layer = Layer::Below(11);
        let top = top_of(&c, 0, layer);
        assert_eq!(top, 11);
        assert_eq!(scan_column(&c, &p, 0, 0, top, layer), (11, 9, 0));
        assert_eq!(scan_column(&c, &p, 1, 0, top, layer), (11 | WALL, 11, 0));
        // The surface view sees the roof.
        assert_eq!(scan_column(&c, &p, 0, 0, 31, Layer::Surface), (11, 20, 0));
    }

    #[test]
    fn shading_follows_the_map_item() {
        // Higher than the north neighbour: bright; lower: dark; level: normal.
        assert_eq!(brightness(1, 65, Some(64), 0, 0), 2);
        assert_eq!(brightness(1, 63, Some(64), 0, 1), 0);
        assert_eq!(brightness(1, 64, Some(64), 0, 0), 1);
        assert_eq!(brightness(1, 64, Some(64), 0, 1), 1);
        // Shallow water is bright, deep water dark.
        assert_eq!(brightness(WATER, 62, None, 1, 0), 2);
        assert_eq!(brightness(WATER, 62, None, 10, 0), 0);
        assert_eq!(brightness(1 | WALL, 0, None, 0, 0), 3);
        assert_eq!(packed_rgba(1, 2), [127, 178, 56, 255]);
        assert_eq!(packed_rgba(0, 2), [0; 4]);
    }

    fn world_with(chunks: Vec<Chunk>) -> World {
        let mut w = World::new(horizon_world::registry::BlockStateId(0), 0);
        for c in chunks {
            w.chunks.insert((c.x, c.z), Arc::new(c));
        }
        w
    }
    fn grass_chunk(x: i32, z: i32) -> Chunk {
        let mut c = Chunk::empty(x, z);
        for bz in 0..16 {
            for bx in 0..16 {
                c.set(bx, 64, bz, 1);
            }
        }
        c
    }

    #[test]
    fn tiles_fill_nearest_first_and_follow_changes() {
        let mut map = Minimap {
            palette: Some(palette()),
            ..Default::default()
        };
        let facts = BlockFacts {
            states: Vec::new(),
            air: horizon_world::registry::BlockStateId(0),
        };
        let mut world = world_with(
            (-2..=2)
                .flat_map(|x| (-2..=2).map(move |z| grass_chunk(x, z)))
                .collect(),
        );
        // A zero budget still scans one tile: the player's own chunk.
        assert_eq!(
            map.update(&world, &facts, (0, 0), 2, Layer::Surface, Duration::ZERO),
            1
        );
        assert!(map.tiles.contains_key(&(0, 0)));
        let n = map.update(
            &world,
            &facts,
            (0, 0),
            2,
            Layer::Surface,
            Duration::from_secs(1),
        );
        assert_eq!(n, 24);
        assert_eq!(map.tile_count(), 25);
        // Nothing changed: nothing to scan.
        assert_eq!(
            map.update(
                &world,
                &facts,
                (0, 0),
                2,
                Layer::Surface,
                Duration::from_secs(1)
            ),
            0
        );
        // A changed world is rescanned once the tiles are old enough.
        world
            .set(0, 70, 0, horizon_world::registry::BlockStateId(2))
            .unwrap();
        assert_eq!(
            map.update(
                &world,
                &facts,
                (0, 0),
                2,
                Layer::Surface,
                Duration::from_secs(1)
            ),
            0
        );
        for t in map.tiles.values_mut() {
            t.scanned -= RESCAN;
        }
        assert_eq!(
            map.update(
                &world,
                &facts,
                (0, 0),
                2,
                Layer::Surface,
                Duration::from_secs(1)
            ),
            25
        );
        assert_eq!(map.tiles[&(0, 0)].color[0], 11);
        // A new layer rescans everything; unloaded chunks drop out.
        world.chunks.remove(&(2, 2));
        let n = map.update(
            &world,
            &facts,
            (0, 0),
            2,
            Layer::Below(70),
            Duration::from_secs(1),
        );
        assert_eq!(n, 24);
        assert!(!map.tiles.contains_key(&(2, 2)));
    }

    #[test]
    fn the_image_is_north_up_or_turns_with_the_player() {
        let mut map = Minimap {
            palette: Some(palette()),
            ..Default::default()
        };
        let facts = BlockFacts {
            states: Vec::new(),
            air: horizon_world::registry::BlockStateId(0),
        };
        // Grass everywhere, one stone block 10 blocks north of the player.
        let mut world = world_with(
            (-4..=4)
                .flat_map(|x| (-4..=4).map(move |z| grass_chunk(x, z)))
                .collect(),
        );
        world
            .set(0, 64, -10, horizon_world::registry::BlockStateId(2))
            .unwrap();
        map.update(
            &world,
            &facts,
            (0, 0),
            4,
            Layer::Surface,
            Duration::from_secs(1),
        );
        let pixel = |map: &Minimap, i: usize, j: usize| {
            let o = (j * SIZE + i) * 4;
            [map.image[o], map.image[o + 1], map.image[o + 2]]
        };
        let stone = [96, 96, 96];
        let mut view = View {
            x: 0.0,
            z: 0.0,
            yaw: 0.0,
            rotate: false,
            blocks_per_pixel: 1.0,
            round: false,
        };
        assert!(map.compose(view));
        assert!(!map.compose(view), "the same view is not recomposed");
        // North up: the stone is 10 pixels above the centre.
        assert_eq!(pixel(&map, 64, 54), stone);
        // Turning with the player facing east (yaw -90): north is to the left.
        view.rotate = true;
        view.yaw = -90.0;
        assert!(map.compose(view));
        assert_eq!(pixel(&map, 54, 63), stone);
        let (nx, ny) = view.north();
        assert!((nx + 1.0).abs() < 1e-5 && ny.abs() < 1e-5, "{nx} {ny}");
        // Facing north, north is up.
        view.yaw = 180.0;
        map.compose(view);
        assert_eq!(pixel(&map, 64, 54), stone);
        // Round maps are transparent in the corners, framed at the edge.
        view.round = true;
        map.compose(view);
        assert_eq!(map.image[3], 0);
        assert_eq!(&map.image[(64 * SIZE) * 4..(64 * SIZE) * 4 + 4], &BORDER);
        // A small move recomposes only once it is visible.
        view.x += 0.1;
        assert!(!map.compose(view));
        view.x += 0.2;
        assert!(map.compose(view));
    }

    #[test]
    fn rings_cover_each_offset_once() {
        let mut seen = std::collections::HashSet::new();
        for r in 0..=3 {
            for o in ring_offsets(r) {
                assert!(seen.insert(o));
            }
        }
        assert_eq!(seen.len(), 49);
    }
}
