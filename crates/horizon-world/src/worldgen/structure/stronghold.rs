//! Strongholds: StrongholdStructure, StrongholdPieces and the concentric
//! ring positions of ChunkGeneratorStructureState.
use super::{
    BoundingBox,
    legacy::{LegacyPiece, PieceContext, PieceFrame, Result, make_bounding_box, random_horizontal},
};
use crate::{registry::BlockStateId, worldgen::biome::BiomeId};
use horizon_core::{
    geometry::Direction,
    position::ChunkPos,
    random::{LegacyRandom, RandomSource, WorldgenRandom},
};
use std::{collections::BTreeSet, f64::consts::PI, sync::Mutex};

/// ChunkGeneratorStructureState.generateRingPositions with
/// BiomeSource.findBiomeHorizontal (radius 112, step 1, not closest).
/// `biomes` samples a search's quarts in order, as one caching resolver.
pub fn ring_positions(
    seed: i64,
    distance: i32,
    spread: i32,
    count: i32,
    preferred: &BTreeSet<BiomeId>,
    biomes: &mut dyn FnMut(&[[i32; 3]]) -> Vec<BiomeId>,
) -> Vec<ChunkPos> {
    ring_searches(seed, distance, spread, count)
        .iter()
        .map(|search| search.find(preferred, biomes))
        .collect()
}

/// findBiomeHorizontal's radius, 112 blocks, in quarts.
const SEARCH_QUARTS: i32 = 28;

/// One ring position before its biome search: the chunk on the ring and the
/// random its search uses. Vanilla runs each search as its own task.
#[derive(Clone, Debug)]
pub struct RingSearch {
    x: i32,
    z: i32,
    random: LegacyRandom,
}
impl RingSearch {
    /// Whether the search can end within `margin` chunks of chunk (`x`,
    /// `z`): it ends on its ring chunk or in the square it samples, at most
    /// 120 blocks from the ring chunk's corner (8 chunks, rounded up).
    pub fn may_land_near(&self, x: i32, z: i32, margin: i32) -> bool {
        let reach = (SEARCH_QUARTS * 4 + 8 + 15) / 16 + margin;
        (self.x - x).abs() <= reach && (self.z - z).abs() <= reach
    }
    /// BiomeSource.findBiomeHorizontal around the ring chunk's centre: a
    /// random preferred biome within 112 blocks, else the ring chunk itself.
    ///
    /// Vanilla samples the square in order on one thread, and its climate
    /// index (Climate.RTree) starts each lookup from the previous result and
    /// keeps it on a tie; so `biomes` gets the whole square at once, in
    /// vanilla's order. Fresh lookups moved seed 1234's first stronghold 6
    /// chunks: one quart on a lush caves / ocean tie went to the ocean.
    /// `FixedBiomeSource.findBiomeHorizontal`: the one biome is allowed or it is not, and an
    /// allowed one gives a random block of the square (two draws), not the reservoir search.
    pub fn find_fixed(
        &self,
        preferred: &BTreeSet<BiomeId>,
        biomes: &mut dyn FnMut(&[[i32; 3]]) -> Vec<BiomeId>,
    ) -> ChunkPos {
        let mut search = self.random.clone();
        let (cx, cz) = (self.x * 16 + 8, self.z * 16 + 8);
        let allowed = biomes(&[[cx >> 2, 0, cz >> 2]])
            .first()
            .is_some_and(|biome| preferred.contains(biome));
        if !allowed {
            return ChunkPos::new(self.x, self.z);
        }
        let r = SEARCH_QUARTS * 4;
        let x = cx - r + search.next_int_bound(r * 2 + 1).unwrap_or(0);
        let z = cz - r + search.next_int_bound(r * 2 + 1).unwrap_or(0);
        ChunkPos::new(x >> 4, z >> 4)
    }
    pub fn find(
        &self,
        preferred: &BTreeSet<BiomeId>,
        biomes: &mut dyn FnMut(&[[i32; 3]]) -> Vec<BiomeId>,
    ) -> ChunkPos {
        let mut search = self.random.clone();
        let (cx, cz) = ((self.x * 16 + 8) >> 2, (self.z * 16 + 8) >> 2);
        let r = SEARCH_QUARTS;
        let quarts: Vec<[i32; 3]> = (-r..=r)
            .flat_map(|dz| (-r..=r).map(move |dx| [cx + dx, 0, cz + dz]))
            .collect();
        let mut found = 0;
        let mut result = None;
        for (quart, biome) in quarts.iter().zip(biomes(&quarts)) {
            if preferred.contains(&biome) {
                if result.is_none() || search.next_int_bound(found + 1).unwrap_or(1) == 0 {
                    result = Some((quart[0] << 2, quart[2] << 2));
                }
                found += 1;
            }
        }
        match result {
            Some((bx, bz)) => ChunkPos::new(bx >> 4, bz >> 4),
            None => ChunkPos::new(self.x, self.z),
        }
    }
}

/// The ring chunks and search randoms of generateRingPositions, in order:
/// the serial part, cheap; the searches can then run anywhere.
pub fn ring_searches(seed: i64, distance: i32, spread: i32, count: i32) -> Vec<RingSearch> {
    let mut random = LegacyRandom::new(seed);
    let mut angle = random.next_double() * PI * 2.0;
    let (mut in_circle, mut circle, mut spread) = (0, 0, spread);
    let mut out = Vec::with_capacity(count.max(0) as usize);
    for i in 0..count {
        let dist = f64::from(4 * distance + distance * circle * 6)
            + (random.next_double() - 0.5) * (f64::from(distance) * 2.5);
        let round = |v: f64| (v + 0.5).floor() as i64 as i32;
        out.push(RingSearch {
            x: round(horizon_core::jmath::cos(angle) * dist),
            z: round(horizon_core::jmath::sin(angle) * dist),
            random: random.fork(),
        });
        angle += PI * 2.0 / f64::from(spread);
        in_circle += 1;
        if in_circle == spread {
            circle += 1;
            in_circle = 0;
            spread += 2 * spread / (circle + 1);
            spread = spread.min(count - i);
            angle += random.next_double() * PI * 2.0;
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Door {
    Opening,
    Wood,
    Grates,
    Iron,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Straight,
    PrisonHall,
    LeftTurn,
    RightTurn,
    RoomCrossing,
    StraightStairsDown,
    StairsDown,
    FiveCrossing,
    ChestCorridor,
    Library,
    PortalRoom,
    Filler,
    Start,
}
/// STRONGHOLD_PIECE_WEIGHTS: kind, weight, max place count.
const WEIGHTS: [(Kind, i32, i32); 11] = [
    (Kind::Straight, 40, 0),
    (Kind::PrisonHall, 5, 5),
    (Kind::LeftTurn, 20, 0),
    (Kind::RightTurn, 20, 0),
    (Kind::RoomCrossing, 10, 6),
    (Kind::StraightStairsDown, 5, 5),
    (Kind::StairsDown, 5, 5),
    (Kind::FiveCrossing, 5, 4),
    (Kind::ChestCorridor, 5, 4),
    (Kind::Library, 10, 2),
    (Kind::PortalRoom, 20, 1),
];

#[derive(Debug)]
pub struct StrongholdPiece {
    kind: Kind,
    frame: PieceFrame,
    depth: i32,
    door: Door,
    /// Straight: left, right; FiveCrossing: leftLow, leftHigh, rightLow,
    /// rightHigh; Library: tall.
    flags: [bool; 4],
    /// RoomCrossing type or FillerCorridor steps.
    value: i32,
    /// ChestCorridor chest / PortalRoom spawner placed.
    placed: Mutex<bool>,
}

fn int(r: &mut impl RandomSource, bound: i32) -> i32 {
    r.next_int_bound(bound).unwrap_or(0)
}
/// BoundingBox.orientBox.
#[allow(clippy::too_many_arguments)]
pub(super) fn orient_box(
    [x, y, z]: [i32; 3],
    [ox, oy, oz]: [i32; 3],
    [w, h, d]: [i32; 3],
    direction: Direction,
) -> BoundingBox {
    match direction {
        Direction::North => BoundingBox::new(
            [x + ox, y + oy, z - d + 1 + oz],
            [x + w - 1 + ox, y + h - 1 + oy, z + oz],
        ),
        Direction::West => BoundingBox::new(
            [x - d + 1 + oz, y + oy, z + ox],
            [x + oz, y + h - 1 + oy, z + w - 1 + ox],
        ),
        Direction::East => BoundingBox::new(
            [x + oz, y + oy, z + ox],
            [x + d - 1 + oz, y + h - 1 + oy, z + w - 1 + ox],
        ),
        _ => BoundingBox::new(
            [x + ox, y + oy, z + oz],
            [x + w - 1 + ox, y + h - 1 + oy, z + d - 1 + oz],
        ),
    }
}
fn random_door(r: &mut impl RandomSource) -> Door {
    match int(r, 5) {
        2 => Door::Wood,
        3 => Door::Grates,
        4 => Door::Iron,
        _ => Door::Opening,
    }
}

struct Builder {
    pieces: Vec<StrongholdPiece>,
    weights: Vec<(Kind, i32, i32, i32)>,
    imposed: Option<Kind>,
    previous: Option<Kind>,
    pending: Vec<usize>,
    portal: bool,
}
impl Builder {
    fn collides(&self, b: &BoundingBox) -> bool {
        self.pieces
            .iter()
            .any(|p| p.frame.bounding_box.intersects(b))
    }
    fn piece(kind: Kind, b: BoundingBox, d: Direction, depth: i32) -> StrongholdPiece {
        StrongholdPiece {
            kind,
            frame: PieceFrame::new(b, Some(d)),
            depth,
            door: Door::Opening,
            flags: [false; 4],
            value: 0,
            placed: Mutex::new(false),
        }
    }
    /// StrongholdPieces.findAndCreatePieceFactory.
    fn create(
        &self,
        kind: Kind,
        r: &mut impl RandomSource,
        foot: [i32; 3],
        d: Direction,
        depth: i32,
    ) -> Option<StrongholdPiece> {
        let ok = |b: &BoundingBox| b.min[1] > 10 && !self.collides(b);
        let (offset, size) = match kind {
            Kind::Straight | Kind::ChestCorridor => ([-1, -1, 0], [5, 5, 7]),
            Kind::PrisonHall => ([-1, -1, 0], [9, 5, 11]),
            Kind::LeftTurn | Kind::RightTurn => ([-1, -1, 0], [5, 5, 5]),
            Kind::RoomCrossing => ([-4, -1, 0], [11, 7, 11]),
            Kind::StraightStairsDown => ([-1, -7, 0], [5, 11, 8]),
            Kind::StairsDown => ([-1, -7, 0], [5, 11, 5]),
            Kind::FiveCrossing => ([-4, -3, 0], [10, 9, 11]),
            Kind::Library => ([-4, -1, 0], [14, 11, 15]),
            Kind::PortalRoom => ([-4, -1, 0], [11, 8, 16]),
            Kind::Filler | Kind::Start => return None,
        };
        let mut b = orient_box(foot, offset, size, d);
        if !ok(&b) {
            if kind != Kind::Library {
                return None;
            }
            b = orient_box(foot, offset, [14, 6, 15], d);
            if !ok(&b) {
                return None;
            }
        }
        let mut piece = Self::piece(kind, b, d, depth);
        if kind != Kind::PortalRoom {
            piece.door = random_door(r);
        }
        match kind {
            Kind::Straight => {
                piece.flags[0] = int(r, 2) == 0;
                piece.flags[1] = int(r, 2) == 0;
            }
            Kind::RoomCrossing => piece.value = int(r, 5),
            Kind::FiveCrossing => {
                piece.flags[0] = r.next_boolean();
                piece.flags[1] = r.next_boolean();
                piece.flags[2] = r.next_boolean();
                piece.flags[3] = int(r, 3) > 0;
            }
            Kind::Library => piece.flags[0] = b.span()[1] > 6,
            _ => {}
        }
        Some(piece)
    }
    /// StrongholdPieces.generatePieceFromSmallDoor.
    fn piece_from_small_door(
        &mut self,
        r: &mut impl RandomSource,
        foot: [i32; 3],
        d: Direction,
        depth: i32,
    ) -> Option<StrongholdPiece> {
        let any = self.weights.iter().any(|w| w.2 > 0 && w.3 < w.2);
        let total: i32 = self.weights.iter().map(|w| w.1).sum();
        if !any {
            return None;
        }
        if let Some(kind) = self.imposed.take() {
            if let Some(piece) = self.create(kind, r, foot, d, depth) {
                return Some(piece);
            }
        }
        for _ in 0..5 {
            let mut selection = int(r, total);
            for i in 0..self.weights.len() {
                let (kind, weight, max, count) = self.weights[i];
                selection -= weight;
                if selection < 0 {
                    let allowed = (max == 0 || count < max)
                        && match kind {
                            Kind::Library => depth > 4,
                            Kind::PortalRoom => depth > 5,
                            _ => true,
                        };
                    if !allowed || self.previous == Some(kind) {
                        break;
                    }
                    if let Some(piece) = self.create(kind, r, foot, d, depth) {
                        self.weights[i].3 += 1;
                        self.previous = Some(kind);
                        if max != 0 && count + 1 >= max {
                            self.weights.remove(i);
                        }
                        return Some(piece);
                    }
                }
            }
        }
        let b = orient_box(foot, [-1, -1, 0], [5, 5, 4], d);
        let collision = self
            .pieces
            .iter()
            .find(|p| p.frame.bounding_box.intersects(&b))?
            .frame
            .bounding_box;
        if collision.min[1] != b.min[1] {
            return None;
        }
        for length in [2, 1] {
            if !collision.intersects(&orient_box(foot, [-1, -1, 0], [5, 5, length], d)) {
                let b = orient_box(foot, [-1, -1, 0], [5, 5, length + 1], d);
                if b.min[1] <= 1 {
                    return None;
                }
                let mut piece = Self::piece(Kind::Filler, b, d, depth);
                piece.value = b.span()[if matches!(d, Direction::North | Direction::South) {
                    2
                } else {
                    0
                }];
                return Some(piece);
            }
        }
        None
    }
    /// StrongholdPieces.generateAndAddPiece.
    fn add(&mut self, r: &mut impl RandomSource, foot: [i32; 3], d: Direction, depth: i32) {
        if depth > 50 {
            return;
        }
        let start = self.pieces[0].frame.bounding_box;
        if (foot[0] - start.min[0]).abs() > 112 || (foot[2] - start.min[2]).abs() > 112 {
            return;
        }
        if let Some(piece) = self.piece_from_small_door(r, foot, d, depth + 1) {
            self.pieces.push(piece);
            self.pending.push(self.pieces.len() - 1);
        }
    }
    fn forward(&mut self, r: &mut impl RandomSource, i: usize, x_off: i32, y_off: i32) {
        let p = &self.pieces[i];
        let (b, depth) = (p.frame.bounding_box, p.depth);
        let Some(o) = p.frame.orientation else { return };
        let y = b.min[1] + y_off;
        let foot = match o {
            Direction::North => [b.min[0] + x_off, y, b.min[2] - 1],
            Direction::South => [b.min[0] + x_off, y, b.max[2] + 1],
            Direction::West => [b.min[0] - 1, y, b.min[2] + x_off],
            Direction::East => [b.max[0] + 1, y, b.min[2] + x_off],
            _ => return,
        };
        self.add(r, foot, o, depth);
    }
    fn side(&mut self, r: &mut impl RandomSource, i: usize, right: bool, y_off: i32, z_off: i32) {
        let p = &self.pieces[i];
        let (b, depth) = (p.frame.bounding_box, p.depth);
        let Some(o) = p.frame.orientation else { return };
        let y = b.min[1] + y_off;
        let (foot, d) = match (o, right) {
            (Direction::North | Direction::South, false) => {
                ([b.min[0] - 1, y, b.min[2] + z_off], Direction::West)
            }
            (Direction::North | Direction::South, true) => {
                ([b.max[0] + 1, y, b.min[2] + z_off], Direction::East)
            }
            (Direction::West | Direction::East, false) => {
                ([b.min[0] + z_off, y, b.min[2] - 1], Direction::North)
            }
            (Direction::West | Direction::East, true) => {
                ([b.min[0] + z_off, y, b.max[2] + 1], Direction::South)
            }
            _ => return,
        };
        self.add(r, foot, d, depth);
    }
    fn add_children(&mut self, r: &mut impl RandomSource, i: usize) {
        let p = &self.pieces[i];
        let (kind, flags, o) = (p.kind, p.flags, p.frame.orientation);
        let north_or_east = matches!(o, Some(Direction::North | Direction::East));
        match kind {
            Kind::Start | Kind::StairsDown => {
                if kind == Kind::Start {
                    self.imposed = Some(Kind::FiveCrossing);
                }
                self.forward(r, i, 1, 1);
            }
            Kind::ChestCorridor | Kind::PrisonHall | Kind::StraightStairsDown => {
                self.forward(r, i, 1, 1)
            }
            Kind::Straight => {
                self.forward(r, i, 1, 1);
                if flags[0] {
                    self.side(r, i, false, 1, 2);
                }
                if flags[1] {
                    self.side(r, i, true, 1, 2);
                }
            }
            Kind::FiveCrossing => {
                let (a, b) = if matches!(o, Some(Direction::West | Direction::North)) {
                    (5, 3)
                } else {
                    (3, 5)
                };
                self.forward(r, i, 5, 1);
                if flags[0] {
                    self.side(r, i, false, a, 1);
                }
                if flags[1] {
                    self.side(r, i, false, b, 7);
                }
                if flags[2] {
                    self.side(r, i, true, a, 1);
                }
                if flags[3] {
                    self.side(r, i, true, b, 7);
                }
            }
            Kind::LeftTurn => self.side(r, i, !north_or_east, 1, 1),
            Kind::RightTurn => self.side(r, i, north_or_east, 1, 1),
            Kind::RoomCrossing => {
                self.forward(r, i, 4, 1);
                self.side(r, i, false, 1, 4);
                self.side(r, i, true, 1, 4);
            }
            Kind::PortalRoom => self.portal = true,
            Kind::Library | Kind::Filler => {}
        }
    }
}

/// StrongholdStructure.generatePieces: retried with seed + tries until a
/// portal room is placed.
pub fn generate(
    random: &mut WorldgenRandom<LegacyRandom>,
    seed: i64,
    chunk: [i32; 2],
    sea_level: i32,
    min_y: i32,
) -> Vec<StrongholdPiece> {
    let mut tries = 0i64;
    loop {
        random.set_large_feature_seed(seed.wrapping_add(tries), chunk[0], chunk[1]);
        tries += 1;
        let d = random_horizontal(random);
        let b = make_bounding_box(chunk[0] * 16 + 2, 64, chunk[1] * 16 + 2, d, 5, 11, 5);
        let mut g = Builder {
            pieces: vec![Builder::piece(Kind::Start, b, d, 0)],
            weights: WEIGHTS.iter().map(|&(k, w, m)| (k, w, m, 0)).collect(),
            imposed: None,
            previous: None,
            pending: Vec::new(),
            portal: false,
        };
        g.add_children(random, 0);
        while !g.pending.is_empty() {
            let index = int(random, g.pending.len() as i32) as usize;
            let next = g.pending.remove(index);
            g.add_children(random, next);
        }
        let mut all = g.pieces[0].frame.bounding_box;
        for p in &g.pieces[1..] {
            all.encapsulate(&p.frame.bounding_box);
        }
        let max = sea_level - 10;
        let mut y1 = all.span()[1] + min_y + 1;
        if y1 < max {
            y1 += int(random, max - y1);
        }
        let dy = y1 - all.max[1];
        for p in &mut g.pieces {
            p.frame.bounding_box = p.frame.bounding_box.moved([0, dy, 0]);
        }
        if g.portal {
            return g.pieces;
        }
    }
}

fn smooth(c: &mut PieceContext<'_>, edge: bool) -> BlockStateId {
    if !edge {
        return c.state("cave_air");
    }
    let s = c.random.float();
    c.state(if s < 0.2 {
        "cracked_stone_bricks"
    } else if s < 0.5 {
        "mossy_stone_bricks"
    } else if s < 0.55 {
        "infested_stone_bricks"
    } else {
        "stone_bricks"
    })
}
impl StrongholdPiece {
    fn smooth_box(
        &self,
        c: &mut PieceContext<'_>,
        a: [i32; 3],
        b: [i32; 3],
        skip_air: bool,
    ) -> Result<()> {
        self.frame
            .generate_box_selector(c, a, b, skip_air, &mut |c, _, _, _, edge| smooth(c, edge))
    }
    fn fill(
        &self,
        c: &mut PieceContext<'_>,
        a: [i32; 3],
        b: [i32; 3],
        state: BlockStateId,
    ) -> Result<()> {
        self.frame.generate_box(c, a, b, state, state, false)
    }
    fn air(&self, c: &mut PieceContext<'_>, a: [i32; 3], b: [i32; 3]) -> Result<()> {
        let air = c.state("cave_air");
        self.fill(c, a, b, air)
    }
    fn put(
        &self,
        c: &mut PieceContext<'_>,
        state: BlockStateId,
        x: i32,
        y: i32,
        z: i32,
    ) -> Result<()> {
        self.frame.place(c, state, x, y, z)
    }
    fn small_door(
        &self,
        c: &mut PieceContext<'_>,
        door: Door,
        x: i32,
        y: i32,
        z: i32,
    ) -> Result<()> {
        let bricks = c.state("stone_bricks");
        let frame = [(0, 0), (0, 1), (0, 2), (1, 2), (2, 2), (2, 1), (2, 0)];
        match door {
            Door::Opening => self.air(c, [x, y, z], [x + 2, y + 2, z])?,
            Door::Wood | Door::Iron => {
                for (dx, dy) in frame {
                    self.put(c, bricks, x + dx, y + dy, z)?;
                }
                let block = if door == Door::Wood {
                    "oak_door"
                } else {
                    "iron_door"
                };
                self.put(c, c.state(block), x + 1, y, z)?;
                self.put(
                    c,
                    c.state_with(block, &[("half", "upper")]),
                    x + 1,
                    y + 1,
                    z,
                )?;
                if door == Door::Iron {
                    self.put(
                        c,
                        c.state_with("stone_button", &[("facing", "north")]),
                        x + 2,
                        y + 1,
                        z + 1,
                    )?;
                    self.put(
                        c,
                        c.state_with("stone_button", &[("facing", "south")]),
                        x + 2,
                        y + 1,
                        z - 1,
                    )?;
                }
            }
            Door::Grates => {
                let air = c.state("cave_air");
                let west = c.state_with("iron_bars", &[("west", "true")]);
                let east = c.state_with("iron_bars", &[("east", "true")]);
                let both = c.state_with("iron_bars", &[("east", "true"), ("west", "true")]);
                self.put(c, air, x + 1, y, z)?;
                self.put(c, air, x + 1, y + 1, z)?;
                self.put(c, west, x, y, z)?;
                self.put(c, west, x, y + 1, z)?;
                self.put(c, both, x, y + 2, z)?;
                self.put(c, both, x + 1, y + 2, z)?;
                self.put(c, both, x + 2, y + 2, z)?;
                self.put(c, east, x + 2, y + 1, z)?;
                self.put(c, east, x + 2, y, z)?;
            }
        }
        Ok(())
    }
    fn torch(c: &PieceContext<'_>, facing: &str) -> BlockStateId {
        c.state_with("wall_torch", &[("facing", facing)])
    }
    fn chest_corridor(&self, c: &mut PieceContext<'_>) -> Result<()> {
        self.smooth_box(c, [0, 0, 0], [4, 4, 6], true)?;
        self.small_door(c, self.door, 1, 1, 0)?;
        self.small_door(c, Door::Opening, 1, 1, 6)?;
        let bricks = c.state("stone_bricks");
        self.fill(c, [3, 1, 2], [3, 1, 4], bricks)?;
        let slab = c.state("stone_brick_slab");
        for (x, y, z) in [
            (3, 1, 1),
            (3, 1, 5),
            (3, 2, 2),
            (3, 2, 4),
            (2, 1, 2),
            (2, 1, 3),
            (2, 1, 4),
        ] {
            self.put(c, slab, x, y, z)?;
        }
        if !*self.placed.lock().unwrap() && c.chunk_box.is_inside(self.frame.world_pos(3, 2, 3)) {
            *self.placed.lock().unwrap() = true;
            self.frame
                .create_chest(c, 3, 2, 3, "minecraft:chests/stronghold_corridor")?;
        }
        Ok(())
    }
    fn filler(&self, c: &mut PieceContext<'_>) -> Result<()> {
        let bricks = c.state("stone_bricks");
        let air = c.state("cave_air");
        for i in 0..self.value {
            for x in 0..=4 {
                self.put(c, bricks, x, 0, i)?;
            }
            for y in 1..=3 {
                self.put(c, bricks, 0, y, i)?;
                for x in 1..=3 {
                    self.put(c, air, x, y, i)?;
                }
                self.put(c, bricks, 4, y, i)?;
            }
            for x in 0..=4 {
                self.put(c, bricks, x, 4, i)?;
            }
        }
        Ok(())
    }
    fn five_crossing(&self, c: &mut PieceContext<'_>) -> Result<()> {
        let [left_low, left_high, right_low, right_high] = self.flags;
        self.smooth_box(c, [0, 0, 0], [9, 8, 10], true)?;
        self.small_door(c, self.door, 4, 3, 0)?;
        if left_low {
            self.air(c, [0, 3, 1], [0, 5, 3])?;
        }
        if right_low {
            self.air(c, [9, 3, 1], [9, 5, 3])?;
        }
        if left_high {
            self.air(c, [0, 5, 7], [0, 7, 9])?;
        }
        if right_high {
            self.air(c, [9, 5, 7], [9, 7, 9])?;
        }
        self.air(c, [5, 1, 10], [7, 3, 10])?;
        self.smooth_box(c, [1, 2, 1], [8, 2, 6], false)?;
        self.smooth_box(c, [4, 1, 5], [4, 4, 9], false)?;
        self.smooth_box(c, [8, 1, 5], [8, 4, 9], false)?;
        self.smooth_box(c, [1, 4, 7], [3, 4, 9], false)?;
        self.smooth_box(c, [1, 3, 5], [3, 3, 6], false)?;
        let slab = c.state("smooth_stone_slab");
        self.fill(c, [1, 3, 4], [3, 3, 4], slab)?;
        self.fill(c, [1, 4, 6], [3, 4, 6], slab)?;
        self.smooth_box(c, [5, 1, 7], [7, 1, 8], false)?;
        self.fill(c, [5, 1, 9], [7, 1, 9], slab)?;
        self.fill(c, [5, 2, 7], [7, 2, 7], slab)?;
        self.fill(c, [4, 5, 7], [4, 5, 9], slab)?;
        self.fill(c, [8, 5, 7], [8, 5, 9], slab)?;
        let double = c.state_with("smooth_stone_slab", &[("type", "double")]);
        self.fill(c, [5, 5, 7], [7, 5, 9], double)?;
        self.put(c, Self::torch(c, "south"), 6, 5, 6)
    }
    fn turn(&self, c: &mut PieceContext<'_>, left: bool) -> Result<()> {
        self.smooth_box(c, [0, 0, 0], [4, 4, 4], true)?;
        self.small_door(c, self.door, 1, 1, 0)?;
        let north_or_east = matches!(
            self.frame.orientation,
            Some(Direction::North | Direction::East)
        );
        if north_or_east == left {
            self.air(c, [0, 1, 1], [0, 3, 3])
        } else {
            self.air(c, [4, 1, 1], [4, 3, 3])
        }
    }
    fn library(&self, c: &mut PieceContext<'_>) -> Result<()> {
        let tall = self.flags[0];
        let height = if tall { 11 } else { 6 };
        self.smooth_box(c, [0, 0, 0], [13, height - 1, 14], true)?;
        self.small_door(c, self.door, 4, 1, 0)?;
        let web = c.state("cobweb");
        self.frame
            .generate_maybe_box(c, 0.07, [2, 1, 1], [11, 4, 13], web, web, false, false)?;
        let planks = c.state("oak_planks");
        let shelf = c.state("bookshelf");
        for d in 1..=13 {
            let block = if (d - 1) % 4 == 0 { planks } else { shelf };
            self.fill(c, [1, 1, d], [1, 4, d], block)?;
            self.fill(c, [12, 1, d], [12, 4, d], block)?;
            if block == planks {
                self.put(c, Self::torch(c, "east"), 2, 3, d)?;
                self.put(c, Self::torch(c, "west"), 11, 3, d)?;
            }
            if tall {
                self.fill(c, [1, 6, d], [1, 9, d], block)?;
                self.fill(c, [12, 6, d], [12, 9, d], block)?;
            }
        }
        for d in (3..12).step_by(2) {
            self.fill(c, [3, 1, d], [4, 3, d], shelf)?;
            self.fill(c, [6, 1, d], [7, 3, d], shelf)?;
            self.fill(c, [9, 1, d], [10, 3, d], shelf)?;
        }
        if tall {
            self.fill(c, [1, 5, 1], [3, 5, 13], planks)?;
            self.fill(c, [10, 5, 1], [12, 5, 13], planks)?;
            self.fill(c, [4, 5, 1], [9, 5, 2], planks)?;
            self.fill(c, [4, 5, 12], [9, 5, 13], planks)?;
            self.put(c, planks, 9, 5, 11)?;
            self.put(c, planks, 8, 5, 11)?;
            self.put(c, planks, 9, 5, 10)?;
            let fence = |c: &PieceContext<'_>, sides: &[&str]| {
                let props: Vec<(&str, &str)> = sides.iter().map(|s| (*s, "true")).collect();
                c.state_with("oak_fence", &props)
            };
            let we = fence(c, &["west", "east"]);
            let ns = fence(c, &["north", "south"]);
            self.fill(c, [3, 6, 3], [3, 6, 11], ns)?;
            self.fill(c, [10, 6, 3], [10, 6, 9], ns)?;
            self.fill(c, [4, 6, 2], [9, 6, 2], we)?;
            self.fill(c, [4, 6, 12], [7, 6, 12], we)?;
            self.put(c, fence(c, &["north", "east"]), 3, 6, 2)?;
            self.put(c, fence(c, &["south", "east"]), 3, 6, 12)?;
            self.put(c, fence(c, &["north", "west"]), 10, 6, 2)?;
            for i in 0..=2 {
                self.put(c, fence(c, &["south", "west"]), 8 + i, 6, 12 - i)?;
                if i != 2 {
                    self.put(c, fence(c, &["north", "east"]), 8 + i, 6, 11 - i)?;
                }
            }
            let ladder = c.state_with("ladder", &[("facing", "south")]);
            for y in 1..=7 {
                self.put(c, ladder, 10, y, 13)?;
            }
            let e = fence(c, &["east"]);
            let w = fence(c, &["west"]);
            self.put(c, e, 6, 9, 7)?;
            self.put(c, w, 7, 9, 7)?;
            self.put(c, e, 6, 8, 7)?;
            self.put(c, w, 7, 8, 7)?;
            let all = fence(c, &["north", "south", "west", "east"]);
            self.put(c, all, 6, 7, 7)?;
            self.put(c, all, 7, 7, 7)?;
            self.put(c, e, 5, 7, 7)?;
            self.put(c, w, 8, 7, 7)?;
            self.put(c, fence(c, &["east", "north"]), 6, 7, 6)?;
            self.put(c, fence(c, &["east", "south"]), 6, 7, 8)?;
            self.put(c, fence(c, &["west", "north"]), 7, 7, 6)?;
            self.put(c, fence(c, &["west", "south"]), 7, 7, 8)?;
            let torch = c.state("torch");
            for (x, z) in [(5, 7), (8, 7), (6, 6), (6, 8), (7, 6), (7, 8)] {
                self.put(c, torch, x, 8, z)?;
            }
        }
        self.frame
            .create_chest(c, 3, 3, 5, "minecraft:chests/stronghold_library")?;
        if tall {
            self.put(c, c.state("cave_air"), 12, 9, 1)?;
            self.frame
                .create_chest(c, 12, 8, 1, "minecraft:chests/stronghold_library")?;
        }
        Ok(())
    }
    fn portal_room(&self, c: &mut PieceContext<'_>) -> Result<()> {
        self.smooth_box(c, [0, 0, 0], [10, 7, 15], false)?;
        self.small_door(c, Door::Grates, 4, 1, 0)?;
        for (a, b) in [
            ([1, 6, 1], [1, 6, 14]),
            ([9, 6, 1], [9, 6, 14]),
            ([2, 6, 1], [8, 6, 2]),
            ([2, 6, 14], [8, 6, 14]),
            ([1, 1, 1], [2, 1, 4]),
            ([8, 1, 1], [9, 1, 4]),
        ] {
            self.smooth_box(c, a, b, false)?;
        }
        let lava = c.state("lava");
        self.fill(c, [1, 1, 1], [1, 1, 3], lava)?;
        self.fill(c, [9, 1, 1], [9, 1, 3], lava)?;
        self.smooth_box(c, [3, 1, 8], [7, 1, 12], false)?;
        self.fill(c, [4, 1, 9], [6, 1, 11], lava)?;
        let ns = c.state_with("iron_bars", &[("north", "true"), ("south", "true")]);
        let we = c.state_with("iron_bars", &[("west", "true"), ("east", "true")]);
        for z in (3..14).step_by(2) {
            self.fill(c, [0, 3, z], [0, 4, z], ns)?;
            self.fill(c, [10, 3, z], [10, 4, z], ns)?;
        }
        for x in (2..9).step_by(2) {
            self.fill(c, [x, 3, 15], [x, 4, 15], we)?;
        }
        let stairs = c.state_with("stone_brick_stairs", &[("facing", "north")]);
        self.smooth_box(c, [4, 1, 5], [6, 1, 7], false)?;
        self.smooth_box(c, [4, 2, 6], [6, 2, 7], false)?;
        self.smooth_box(c, [4, 3, 7], [6, 3, 7], false)?;
        for x in 4..=6 {
            self.put(c, stairs, x, 1, 4)?;
            self.put(c, stairs, x, 2, 5)?;
            self.put(c, stairs, x, 3, 6)?;
        }
        let mut eyes = [false; 12];
        for e in &mut eyes {
            *e = c.random.float() > 0.9;
        }
        let frames = [
            ("north", 4, 8),
            ("north", 5, 8),
            ("north", 6, 8),
            ("south", 4, 12),
            ("south", 5, 12),
            ("south", 6, 12),
            ("east", 3, 9),
            ("east", 3, 10),
            ("east", 3, 11),
            ("west", 7, 9),
            ("west", 7, 10),
            ("west", 7, 11),
        ];
        for (i, (facing, x, z)) in frames.into_iter().enumerate() {
            let eye = if eyes[i] { "true" } else { "false" };
            let state = c.state_with("end_portal_frame", &[("facing", facing), ("eye", eye)]);
            self.put(c, state, x, 3, z)?;
        }
        if eyes.iter().all(|e| *e) {
            let portal = c.state("end_portal");
            for z in 9..=11 {
                for x in 4..=6 {
                    self.put(c, portal, x, 3, z)?;
                }
            }
        }
        if !*self.placed.lock().unwrap() {
            let pos = self.frame.world_pos(5, 3, 6);
            if c.chunk_box.is_inside(pos) {
                *self.placed.lock().unwrap() = true;
                c.spawner(pos, "minecraft:silverfish")?;
            }
        }
        Ok(())
    }
    fn prison_hall(&self, c: &mut PieceContext<'_>) -> Result<()> {
        self.smooth_box(c, [0, 0, 0], [8, 4, 10], true)?;
        self.small_door(c, self.door, 1, 1, 0)?;
        self.air(c, [1, 1, 10], [3, 3, 10])?;
        for z in [1, 3, 7, 9] {
            self.smooth_box(c, [4, 1, z], [4, 3, z], false)?;
        }
        let ns = c.state_with("iron_bars", &[("north", "true"), ("south", "true")]);
        let nse = c.state_with(
            "iron_bars",
            &[("north", "true"), ("south", "true"), ("east", "true")],
        );
        let we = c.state_with("iron_bars", &[("west", "true"), ("east", "true")]);
        for y in 1..=3 {
            self.put(c, ns, 4, y, 4)?;
            self.put(c, nse, 4, y, 5)?;
            self.put(c, ns, 4, y, 6)?;
            self.put(c, we, 5, y, 5)?;
            self.put(c, we, 6, y, 5)?;
            self.put(c, we, 7, y, 5)?;
        }
        self.put(c, ns, 4, 3, 2)?;
        self.put(c, ns, 4, 3, 8)?;
        let bottom = c.state_with("iron_door", &[("facing", "west")]);
        let top = c.state_with("iron_door", &[("facing", "west"), ("half", "upper")]);
        for z in [2, 8] {
            self.put(c, bottom, 4, 1, z)?;
            self.put(c, top, 4, 2, z)?;
        }
        Ok(())
    }
    fn room_crossing(&self, c: &mut PieceContext<'_>) -> Result<()> {
        self.smooth_box(c, [0, 0, 0], [10, 6, 10], true)?;
        self.small_door(c, self.door, 4, 1, 0)?;
        self.air(c, [4, 1, 10], [6, 3, 10])?;
        self.air(c, [0, 1, 4], [0, 3, 6])?;
        self.air(c, [10, 1, 4], [10, 3, 6])?;
        let bricks = c.state("stone_bricks");
        match self.value {
            0 => {
                for y in 1..=3 {
                    self.put(c, bricks, 5, y, 5)?;
                }
                self.put(c, Self::torch(c, "west"), 4, 3, 5)?;
                self.put(c, Self::torch(c, "east"), 6, 3, 5)?;
                self.put(c, Self::torch(c, "south"), 5, 3, 4)?;
                self.put(c, Self::torch(c, "north"), 5, 3, 6)?;
                let slab = c.state("smooth_stone_slab");
                for (x, z) in [
                    (4, 4),
                    (4, 5),
                    (4, 6),
                    (6, 4),
                    (6, 5),
                    (6, 6),
                    (5, 4),
                    (5, 6),
                ] {
                    self.put(c, slab, x, 1, z)?;
                }
            }
            1 => {
                for i in 0..5 {
                    self.put(c, bricks, 3, 1, 3 + i)?;
                    self.put(c, bricks, 7, 1, 3 + i)?;
                    self.put(c, bricks, 3 + i, 1, 3)?;
                    self.put(c, bricks, 3 + i, 1, 7)?;
                }
                for y in 1..=3 {
                    self.put(c, bricks, 5, y, 5)?;
                }
                self.put(c, c.state("water"), 5, 4, 5)?;
            }
            2 => {
                let cobble = c.state("cobblestone");
                for z in 1..=9 {
                    self.put(c, cobble, 1, 3, z)?;
                    self.put(c, cobble, 9, 3, z)?;
                }
                for x in 1..=9 {
                    self.put(c, cobble, x, 3, 1)?;
                    self.put(c, cobble, x, 3, 9)?;
                }
                for (x, y, z) in [
                    (5, 1, 4),
                    (5, 1, 6),
                    (5, 3, 4),
                    (5, 3, 6),
                    (4, 1, 5),
                    (6, 1, 5),
                    (4, 3, 5),
                    (6, 3, 5),
                ] {
                    self.put(c, cobble, x, y, z)?;
                }
                for y in 1..=3 {
                    self.put(c, cobble, 4, y, 4)?;
                    self.put(c, cobble, 6, y, 4)?;
                    self.put(c, cobble, 4, y, 6)?;
                    self.put(c, cobble, 6, y, 6)?;
                }
                self.put(c, c.state("wall_torch"), 5, 3, 5)?;
                let planks = c.state("oak_planks");
                for z in 2..=8 {
                    self.put(c, planks, 2, 3, z)?;
                    self.put(c, planks, 3, 3, z)?;
                    if z <= 3 || z >= 7 {
                        self.put(c, planks, 4, 3, z)?;
                        self.put(c, planks, 5, 3, z)?;
                        self.put(c, planks, 6, 3, z)?;
                    }
                    self.put(c, planks, 7, 3, z)?;
                    self.put(c, planks, 8, 3, z)?;
                }
                let ladder = c.state_with("ladder", &[("facing", "west")]);
                for y in 1..=3 {
                    self.put(c, ladder, 9, y, 3)?;
                }
                self.frame
                    .create_chest(c, 3, 4, 8, "minecraft:chests/stronghold_crossing")?;
            }
            _ => {}
        }
        Ok(())
    }
    fn stairs_down(&self, c: &mut PieceContext<'_>) -> Result<()> {
        self.smooth_box(c, [0, 0, 0], [4, 10, 4], true)?;
        self.small_door(c, self.door, 1, 7, 0)?;
        self.small_door(c, Door::Opening, 1, 1, 4)?;
        let bricks = c.state("stone_bricks");
        let slab = c.state("smooth_stone_slab");
        for (state, x, y, z) in [
            (bricks, 2, 6, 1),
            (bricks, 1, 5, 1),
            (slab, 1, 6, 1),
            (bricks, 1, 5, 2),
            (bricks, 1, 4, 3),
            (slab, 1, 5, 3),
            (bricks, 2, 4, 3),
            (bricks, 3, 3, 3),
            (slab, 3, 4, 3),
            (bricks, 3, 3, 2),
            (bricks, 3, 2, 1),
            (slab, 3, 3, 1),
            (bricks, 2, 2, 1),
            (bricks, 1, 1, 1),
            (slab, 1, 2, 1),
            (bricks, 1, 1, 2),
            (slab, 1, 1, 3),
        ] {
            self.put(c, state, x, y, z)?;
        }
        Ok(())
    }
    fn straight(&self, c: &mut PieceContext<'_>) -> Result<()> {
        self.smooth_box(c, [0, 0, 0], [4, 4, 6], true)?;
        self.small_door(c, self.door, 1, 1, 0)?;
        self.small_door(c, Door::Opening, 1, 1, 6)?;
        let east = Self::torch(c, "east");
        let west = Self::torch(c, "west");
        self.frame.maybe_block(c, 0.1, 1, 2, 1, east)?;
        self.frame.maybe_block(c, 0.1, 3, 2, 1, west)?;
        self.frame.maybe_block(c, 0.1, 1, 2, 5, east)?;
        self.frame.maybe_block(c, 0.1, 3, 2, 5, west)?;
        if self.flags[0] {
            self.air(c, [0, 1, 2], [0, 3, 4])?;
        }
        if self.flags[1] {
            self.air(c, [4, 1, 2], [4, 3, 4])?;
        }
        Ok(())
    }
    fn straight_stairs_down(&self, c: &mut PieceContext<'_>) -> Result<()> {
        self.smooth_box(c, [0, 0, 0], [4, 10, 7], true)?;
        self.small_door(c, self.door, 1, 7, 0)?;
        self.small_door(c, Door::Opening, 1, 1, 7)?;
        let stairs = c.state_with("cobblestone_stairs", &[("facing", "south")]);
        let bricks = c.state("stone_bricks");
        for i in 0..6 {
            for x in 1..=3 {
                self.put(c, stairs, x, 6 - i, 1 + i)?;
            }
            if i < 5 {
                for x in 1..=3 {
                    self.put(c, bricks, x, 5 - i, 1 + i)?;
                }
            }
        }
        Ok(())
    }
}
impl LegacyPiece for StrongholdPiece {
    fn bounding_box(&self) -> BoundingBox {
        self.frame.bounding_box
    }
    fn save(&self) -> Option<crate::nbt::Compound> {
        use crate::nbt::Tag;
        let flag = |b: bool| Tag::Byte(i8::from(b));
        let mut tag = super::legacy::piece_tag(
            self.type_name(),
            self.frame.bounding_box,
            self.frame.orientation,
            self.depth,
        );
        // SmallDoorType.LEGACY_CODEC: the enum name.
        let door = match self.door {
            Door::Opening => "OPENING",
            Door::Wood => "WOOD_DOOR",
            Door::Grates => "GRATES",
            Door::Iron => "IRON_DOOR",
        };
        let placed = *self.placed.lock().unwrap();
        let [a, b, c, d] = self.flags;
        let mut entries = vec![("EntryDoor", Tag::String(door.into()))];
        match self.kind {
            Kind::ChestCorridor => entries.push(("Chest", flag(placed))),
            Kind::Filler => entries.push(("Steps", Tag::Int(self.value))),
            Kind::FiveCrossing => entries.extend([
                ("leftLow", flag(a)),
                ("leftHigh", flag(b)),
                ("rightLow", flag(c)),
                ("rightHigh", flag(d)),
            ]),
            Kind::Library => entries.push(("Tall", flag(a))),
            Kind::PortalRoom => entries.push(("Mob", flag(placed))),
            Kind::RoomCrossing => entries.push(("Type", Tag::Int(self.value))),
            // StartPiece is a StairsDown that is the source.
            Kind::StairsDown => entries.push(("Source", flag(false))),
            Kind::Start => entries.push(("Source", flag(true))),
            Kind::Straight => entries.extend([("Left", flag(a)), ("Right", flag(b))]),
            _ => {}
        }
        for (key, value) in entries {
            tag.insert(key.into(), value);
        }
        Some(tag)
    }
    fn type_name(&self) -> &'static str {
        match self.kind {
            Kind::Straight => "minecraft:shs",
            Kind::PrisonHall => "minecraft:shph",
            Kind::LeftTurn => "minecraft:shlt",
            Kind::RightTurn => "minecraft:shrt",
            Kind::RoomCrossing => "minecraft:shrc",
            Kind::StraightStairsDown => "minecraft:shssd",
            Kind::StairsDown => "minecraft:shsd",
            Kind::FiveCrossing => "minecraft:sh5c",
            Kind::ChestCorridor => "minecraft:shcc",
            Kind::Library => "minecraft:shli",
            Kind::PortalRoom => "minecraft:shpr",
            Kind::Filler => "minecraft:shfc",
            Kind::Start => "minecraft:shstart",
        }
    }
    fn post_process(&self, c: &mut PieceContext<'_>) -> Result<()> {
        match self.kind {
            Kind::Straight => self.straight(c),
            Kind::PrisonHall => self.prison_hall(c),
            Kind::LeftTurn => self.turn(c, true),
            Kind::RightTurn => self.turn(c, false),
            Kind::RoomCrossing => self.room_crossing(c),
            Kind::StraightStairsDown => self.straight_stairs_down(c),
            Kind::StairsDown | Kind::Start => self.stairs_down(c),
            Kind::FiveCrossing => self.five_crossing(c),
            Kind::ChestCorridor => self.chest_corridor(c),
            Kind::Library => self.library(c),
            Kind::PortalRoom => self.portal_room(c),
            Kind::Filler => self.filler(c),
        }
    }
}
