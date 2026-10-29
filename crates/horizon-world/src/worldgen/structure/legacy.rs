//! Piece-based ("legacy") structure support: the StructurePiece helpers,
//! object-safe world/random access and template pieces.
use super::{
    BlockInfo, BoundingBox, Mirror, Rotation, StructureResources, StructureTemplate,
    processor::{PlaceSettings, Processor, ProcessorList, ProcessorRandom, place_in_world},
};
use crate::{
    chunk::BlockState,
    fluid::FluidState,
    heightmap::HeightmapType,
    nbt::{Compound, JavaString, Tag},
    registry::BlockStateId,
    worldgen::{
        biome::BiomeId,
        feature::FeatureWorld,
        placement::{PlacementContext, PlacementQueryError},
    },
};
use horizon_core::{
    geometry::Direction,
    position::{BlockPos, ChunkPos},
    random::{LegacyRandom, RandomSource, position_seed},
};
use std::sync::{Arc, Mutex};

pub type Result<T> = std::result::Result<T, PlacementQueryError>;
/// Data markers of a placed template: (metadata, absolute position).
pub type Markers = Vec<(String, [i32; 3])>;

/// The random draws legacy pieces make.
pub trait PieceRandom: ProcessorRandom {
    fn boolean(&mut self) -> bool;
    fn double(&mut self) -> f64;
    /// The concrete random, for callers that need all of `RandomSource`.
    fn as_any(&mut self) -> &mut dyn std::any::Any;
}
impl<R: RandomSource + 'static> PieceRandom for R {
    fn boolean(&mut self) -> bool {
        self.next_boolean()
    }
    fn double(&mut self) -> f64 {
        self.next_double()
    }
    fn as_any(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Object-safe world access for piece post-processing.
pub trait PieceWorld {
    fn block(&mut self, pos: [i32; 3]) -> Result<BlockStateId>;
    fn set(&mut self, pos: [i32; 3], state: BlockStateId, flags: i32) -> Result<bool>;
    fn fluid(&mut self, pos: [i32; 3]) -> Result<FluidState>;
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Result<i32>;
    fn min_y(&mut self) -> Result<i32>;
    fn max_y(&mut self) -> Result<i32>;
    fn block_entity(&mut self, pos: [i32; 3], nbt: Compound) -> Result<()>;
    /// `addFreshEntity`: an entity from its NBT (absolute `Pos`).
    fn entity(&mut self, nbt: Compound) -> Result<()>;
    fn post_process(&mut self, pos: [i32; 3]) -> Result<()>;
    /// `scheduleTick(pos, fluid, delay)`: kept in the proto chunk until it is live.
    fn fluid_tick(
        &mut self,
        pos: [i32; 3],
        fluid: crate::fluid::FluidKind,
        delay: i32,
    ) -> Result<()>;
    /// WorldGenLevel.getSeed.
    fn seed(&mut self) -> Result<i64>;
    /// Draws from WorldGenLevel.getRandom (the region random).
    fn level_boolean(&mut self) -> Result<bool>;
    fn level_float(&mut self) -> Result<f32>;
    fn level_int(&mut self, bound: i32) -> Result<i32>;
    fn sea_level(&mut self) -> Result<i32>;
    /// Block.isFaceFull of the block's shape at `pos` on face `direction`.
    fn face_full(&mut self, pos: [i32; 3], direction: Direction) -> Result<bool>;
    fn face_sturdy(&mut self, pos: [i32; 3], face: Direction) -> Result<bool>;
    fn center_sturdy(&mut self, pos: [i32; 3], face: Direction) -> Result<bool>;
    /// WorldGenLevel.getBiome (zoomed).
    fn biome(&mut self, pos: [i32; 3]) -> Result<BiomeId>;
    #[allow(clippy::too_many_arguments)]
    fn place_template(
        &mut self,
        resources: &StructureResources,
        template: &StructureTemplate,
        position: [i32; 3],
        reference: [i32; 3],
        settings: &mut PlaceSettings<'_>,
        random: &mut dyn ProcessorRandom,
        flags: i32,
    ) -> Result<bool>;
}
impl<W: FeatureWorld + PlacementContext> PieceWorld for W {
    fn block(&mut self, pos: [i32; 3]) -> Result<BlockStateId> {
        self.block_state(pos)
    }
    fn set(&mut self, pos: [i32; 3], state: BlockStateId, flags: i32) -> Result<bool> {
        self.set_block(pos, state, flags)
            .map_err(|_| PlacementQueryError::Failed("piece write".into()))
    }
    fn fluid(&mut self, pos: [i32; 3]) -> Result<FluidState> {
        self.fluid_state(pos)
    }
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Result<i32> {
        PlacementContext::height(self, kind, x, z)
    }
    fn min_y(&mut self) -> Result<i32> {
        PlacementContext::min_y(self)
    }
    fn max_y(&mut self) -> Result<i32> {
        PlacementContext::max_y(self)
    }
    fn entity(&mut self, nbt: Compound) -> Result<()> {
        self.add_entity(nbt)
    }
    fn block_entity(&mut self, pos: [i32; 3], nbt: Compound) -> Result<()> {
        self.set_block_entity(pos, nbt)
    }
    fn post_process(&mut self, pos: [i32; 3]) -> Result<()> {
        self.mark_postprocessing(pos)
    }
    fn fluid_tick(
        &mut self,
        pos: [i32; 3],
        fluid: crate::fluid::FluidKind,
        delay: i32,
    ) -> Result<()> {
        self.schedule_fluid_tick(pos, fluid, delay)
    }
    fn seed(&mut self) -> Result<i64> {
        self.world_seed()
    }
    fn level_boolean(&mut self) -> Result<bool> {
        self.next_world_boolean()
    }
    fn level_float(&mut self) -> Result<f32> {
        self.next_world_float()
    }
    fn level_int(&mut self, bound: i32) -> Result<i32> {
        self.next_world_int(bound)
    }
    fn sea_level(&mut self) -> Result<i32> {
        PlacementContext::sea_level(self)
    }
    fn face_full(&mut self, pos: [i32; 3], direction: Direction) -> Result<bool> {
        self.attachment_at(offset(pos, direction, 1), opposite(direction))
    }
    fn face_sturdy(&mut self, pos: [i32; 3], face: Direction) -> Result<bool> {
        PlacementContext::face_sturdy(self, pos, face)
    }
    fn center_sturdy(&mut self, pos: [i32; 3], face: Direction) -> Result<bool> {
        PlacementContext::center_sturdy(self, pos, face)
    }
    fn biome(&mut self, pos: [i32; 3]) -> Result<BiomeId> {
        self.biome_identity(pos)
    }
    #[allow(clippy::too_many_arguments)]
    fn place_template(
        &mut self,
        resources: &StructureResources,
        template: &StructureTemplate,
        position: [i32; 3],
        reference: [i32; 3],
        settings: &mut PlaceSettings<'_>,
        random: &mut dyn ProcessorRandom,
        flags: i32,
    ) -> Result<bool> {
        place_in_world(
            self,
            resources,
            template,
            position,
            reference,
            settings,
            Some(random),
            flags,
        )
    }
}

/// Everything a piece needs while post-processing one chunk.
pub struct PieceContext<'a> {
    pub world: &'a mut dyn PieceWorld,
    pub resources: &'a StructureResources,
    /// `BlockState.isSolidRender` of the blocks around the piece.
    pub blocks: &'a dyn SolidRender,
    pub random: &'a mut dyn PieceRandom,
    pub chunk_box: BoundingBox,
    pub chunk: ChunkPos,
    pub reference: [i32; 3],
}
/// `BlockState.isSolidRender`: whether a block draws as a solid block. The
/// structure pieces ask it of the blocks around them, the way
/// `StructurePiece.reorient` does.
pub trait SolidRender {
    fn solid_render(&self, state: BlockStateId) -> bool;
}
impl SolidRender for crate::worldgen::tree_blocks::TreeBlocks {
    fn solid_render(&self, state: BlockStateId) -> bool {
        self.has(state, crate::worldgen::tree_blocks::SOLID_RENDER)
    }
}
impl<T: SolidRender + ?Sized> SolidRender for std::sync::Arc<T> {
    fn solid_render(&self, state: BlockStateId) -> bool {
        (**self).solid_render(state)
    }
}
impl PieceContext<'_> {
    pub fn state(&self, name: &str) -> BlockStateId {
        state(self.resources, name, &[])
    }
    pub fn state_with(&self, name: &str, properties: &[(&str, &str)]) -> BlockStateId {
        state(self.resources, name, properties)
    }
    pub fn name(&self, state: BlockStateId) -> &str {
        self.resources
            .registry
            .state(state)
            .map(|s| s.name.as_str())
            .unwrap_or("")
    }
    pub fn is_block(&mut self, pos: [i32; 3], name: &str) -> Result<bool> {
        let state = self.world.block(pos)?;
        Ok(self.name(state) == name)
    }
    pub fn is_air(&self, state: BlockStateId) -> bool {
        matches!(
            self.name(state),
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
        )
    }
    /// BlockState.liquid(): water and lava blocks.
    pub fn is_liquid(&self, state: BlockStateId) -> bool {
        matches!(self.name(state), "minecraft:water" | "minecraft:lava")
    }
    pub fn property<'s>(&'s self, state: BlockStateId, key: &str) -> Option<&'s str> {
        self.resources
            .registry
            .state(state)
            .and_then(|s| s.properties.get(key))
            .map(String::as_str)
    }
    pub fn with_property(&self, state: BlockStateId, key: &str, value: &str) -> BlockStateId {
        let Some(mut descriptor) = self.resources.registry.state(state).cloned() else {
            return state;
        };
        if let Some(slot) = descriptor.properties.get_mut(key) {
            *slot = value.to_owned();
        }
        self.resources.registry.id(&descriptor).unwrap_or(state)
    }
    /// Store a loot-table container's block entity (RandomizableContainer.setLootTable).
    pub fn loot_table(&mut self, pos: [i32; 3], kind: &str, table: &str) -> Result<()> {
        let seed = self.random.long();
        let mut nbt = Compound::new();
        nbt.insert("id".into(), Tag::String(kind.into()));
        nbt.insert("LootTable".into(), Tag::String(table.into()));
        nbt.insert("LootTableSeed".into(), Tag::Long(seed));
        self.world.block_entity(pos, nbt)
    }
    /// Place a spawner and set its entity (SpawnerBlockEntity.setEntityId).
    pub fn spawner(&mut self, pos: [i32; 3], entity: &str) -> Result<()> {
        self.world.set(pos, self.state("spawner"), 2)?;
        let mut id = Compound::new();
        id.insert("id".into(), Tag::String(entity.into()));
        let mut data = Compound::new();
        data.insert("entity".into(), Tag::Compound(id));
        let mut nbt = Compound::new();
        nbt.insert("id".into(), Tag::String("minecraft:mob_spawner".into()));
        nbt.insert("SpawnData".into(), Tag::Compound(data));
        // SpawnerBlockEntity.setEntityId on a fresh spawner: no potentials.
        nbt.insert("SpawnPotentials".into(), Tag::List(Vec::new()));
        self.world.block_entity(pos, nbt)
    }
}
/// A state by block name and properties (missing properties keep defaults).
pub fn state(
    resources: &StructureResources,
    name: &str,
    properties: &[(&str, &str)],
) -> BlockStateId {
    let registry = resources.registry;
    let mut descriptor = BlockState {
        name: if name.contains(':') {
            name.to_owned()
        } else {
            format!("minecraft:{name}")
        },
        properties: Default::default(),
    };
    for (k, v) in properties {
        descriptor
            .properties
            .insert((*k).to_owned(), (*v).to_owned());
    }
    registry
        .resolve(&descriptor)
        .ok()
        .or_else(|| registry.default_state(&descriptor.name))
        .unwrap_or_else(|| registry.default_state("minecraft:air").unwrap())
}

/// A placed structure piece.
pub trait LegacyPiece: std::fmt::Debug + Send + Sync {
    fn bounding_box(&self) -> BoundingBox;
    /// The vanilla StructurePieceType id, for diagnostics and comparisons.
    fn type_name(&self) -> &'static str;
    /// `StructurePiece.createTag`; None while Horizon cannot save this type,
    /// and then its start is not saved.
    fn save(&self) -> Option<Compound> {
        None
    }
    fn post_process(&self, context: &mut PieceContext<'_>) -> Result<()>;
    /// Structure.afterPlace, run on the first piece once per chunk after all
    /// pieces were placed, with every piece's current box.
    fn after_place(&self, _context: &mut PieceContext<'_>, _pieces: &[BoundingBox]) -> Result<()> {
        Ok(())
    }
}

/// `StructurePiece.createTag`'s own fields: the type, `BB`, `O` (the
/// orientation's 2D data value, -1 for none) and `GD` (genDepth).
pub fn piece_tag(
    id: &str,
    b: BoundingBox,
    orientation: Option<Direction>,
    gen_depth: i32,
) -> Compound {
    let o = orientation.map_or(-1, crate::level::data_2d);
    [
        ("id", Tag::String(id.into())),
        (
            "BB",
            Tag::IntArray(vec![
                b.min[0], b.min[1], b.min[2], b.max[0], b.max[1], b.max[2],
            ]),
        ),
        ("O", Tag::Int(o)),
        ("GD", Tag::Int(gen_depth)),
    ]
    .into_iter()
    .map(|(k, v)| (JavaString::from(k), v))
    .collect()
}

/// StructurePiece's orientation transform and block helpers.
#[derive(Clone, Copy, Debug)]
pub struct PieceFrame {
    pub bounding_box: BoundingBox,
    pub orientation: Option<Direction>,
    pub mirror: Mirror,
    pub rotation: Rotation,
    /// Blocks placeBlock must not replace (StructurePiece.canBeReplaced).
    pub keep: &'static [&'static str],
}
pub const HORIZONTAL: [Direction; 4] = [
    Direction::North,
    Direction::East,
    Direction::South,
    Direction::West,
];
/// Direction.Plane.HORIZONTAL.getRandomDirection.
pub fn random_horizontal(random: &mut impl RandomSource) -> Direction {
    HORIZONTAL[random.next_int_bound(4).unwrap_or(0) as usize]
}
pub fn make_bounding_box(
    x: i32,
    y: i32,
    z: i32,
    direction: Direction,
    width: i32,
    height: i32,
    depth: i32,
) -> BoundingBox {
    if matches!(direction, Direction::North | Direction::South) {
        BoundingBox::new([x, y, z], [x + width - 1, y + height - 1, z + depth - 1])
    } else {
        BoundingBox::new([x, y, z], [x + depth - 1, y + height - 1, z + width - 1])
    }
}
impl PieceFrame {
    pub fn new(bounding_box: BoundingBox, orientation: Option<Direction>) -> Self {
        let (mirror, rotation) = match orientation {
            Some(Direction::South) => (Mirror::LeftRight, Rotation::None),
            Some(Direction::West) => (Mirror::LeftRight, Rotation::Clockwise90),
            Some(Direction::East) => (Mirror::None, Rotation::Clockwise90),
            _ => (Mirror::None, Rotation::None),
        };
        Self {
            bounding_box,
            orientation,
            mirror,
            rotation,
            keep: &[],
        }
    }
    pub fn world_x(&self, x: i32, z: i32) -> i32 {
        match self.orientation {
            Some(Direction::North | Direction::South) => self.bounding_box.min[0] + x,
            Some(Direction::West) => self.bounding_box.max[0] - z,
            Some(Direction::East) => self.bounding_box.min[0] + z,
            _ => x,
        }
    }
    pub fn world_y(&self, y: i32) -> i32 {
        if self.orientation.is_none() {
            y
        } else {
            y + self.bounding_box.min[1]
        }
    }
    pub fn world_z(&self, x: i32, z: i32) -> i32 {
        match self.orientation {
            Some(Direction::North) => self.bounding_box.max[2] - z,
            Some(Direction::South) => self.bounding_box.min[2] + z,
            Some(Direction::West | Direction::East) => self.bounding_box.min[2] + x,
            _ => z,
        }
    }
    pub fn world_pos(&self, x: i32, y: i32, z: i32) -> [i32; 3] {
        [self.world_x(x, z), self.world_y(y), self.world_z(x, z)]
    }
    /// StructurePiece.placeBlock (canBeReplaced is always true here).
    pub fn place(
        &self,
        c: &mut PieceContext<'_>,
        state: BlockStateId,
        x: i32,
        y: i32,
        z: i32,
    ) -> Result<()> {
        let pos = self.world_pos(x, y, z);
        if !c.chunk_box.is_inside(pos) {
            return Ok(());
        }
        if !self.keep.is_empty() {
            let current = c.world.block(pos)?;
            if self.keep.contains(&c.name(current)) {
                return Ok(());
            }
        }
        let mut state = state;
        if self.mirror != Mirror::None {
            state = c.resources.transforms.mirror(state, self.mirror);
        }
        if self.rotation != Rotation::None {
            state = c.resources.transforms.rotate(state, self.rotation);
        }
        c.world.set(pos, state, 2)?;
        let fluid = c.world.fluid(pos)?;
        if fluid.kind() != crate::fluid::FluidKind::Empty {
            c.world.fluid_tick(pos, fluid.kind(), 0)?;
        }
        if matches!(
            c.name(state),
            "minecraft:nether_brick_fence"
                | "minecraft:torch"
                | "minecraft:wall_torch"
                | "minecraft:oak_fence"
                | "minecraft:spruce_fence"
                | "minecraft:dark_oak_fence"
                | "minecraft:pale_oak_fence"
                | "minecraft:acacia_fence"
                | "minecraft:birch_fence"
                | "minecraft:jungle_fence"
                | "minecraft:ladder"
                | "minecraft:iron_bars"
        ) {
            c.world.post_process(pos)?;
        }
        Ok(())
    }
    pub fn get(&self, c: &mut PieceContext<'_>, x: i32, y: i32, z: i32) -> Result<BlockStateId> {
        let pos = self.world_pos(x, y, z);
        if c.chunk_box.is_inside(pos) {
            c.world.block(pos)
        } else {
            Ok(c.state("air"))
        }
    }
    pub fn is_interior(&self, c: &mut PieceContext<'_>, x: i32, y: i32, z: i32) -> Result<bool> {
        let pos = self.world_pos(x, y + 1, z);
        if !c.chunk_box.is_inside(pos) {
            return Ok(false);
        }
        Ok(pos[1]
            < c.world
                .height(HeightmapType::OceanFloorWg, pos[0], pos[2])?)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn generate_box(
        &self,
        c: &mut PieceContext<'_>,
        [x0, y0, z0]: [i32; 3],
        [x1, y1, z1]: [i32; 3],
        edge: BlockStateId,
        fill: BlockStateId,
        skip_air: bool,
    ) -> Result<()> {
        for y in y0..=y1 {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    if skip_air {
                        let s = self.get(c, x, y, z)?;
                        if c.is_air(s) {
                            continue;
                        }
                    }
                    let interior = y != y0 && y != y1 && x != x0 && x != x1 && z != z0 && z != z1;
                    self.place(c, if interior { fill } else { edge }, x, y, z)?;
                }
            }
        }
        Ok(())
    }
    /// generateBox with a block selector called as (random, x, y, z, edge).
    pub fn generate_box_selector(
        &self,
        c: &mut PieceContext<'_>,
        [x0, y0, z0]: [i32; 3],
        [x1, y1, z1]: [i32; 3],
        skip_air: bool,
        selector: &mut dyn FnMut(&mut PieceContext<'_>, i32, i32, i32, bool) -> BlockStateId,
    ) -> Result<()> {
        for y in y0..=y1 {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    if skip_air {
                        let s = self.get(c, x, y, z)?;
                        if c.is_air(s) {
                            continue;
                        }
                    }
                    let edge = y == y0 || y == y1 || x == x0 || x == x1 || z == z0 || z == z1;
                    let state = selector(c, x, y, z, edge);
                    self.place(c, state, x, y, z)?;
                }
            }
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub fn generate_maybe_box(
        &self,
        c: &mut PieceContext<'_>,
        probability: f32,
        [x0, y0, z0]: [i32; 3],
        [x1, y1, z1]: [i32; 3],
        edge: BlockStateId,
        fill: BlockStateId,
        skip_air: bool,
        inside: bool,
    ) -> Result<()> {
        for y in y0..=y1 {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    if c.random.float() > probability {
                        continue;
                    }
                    if skip_air {
                        let s = self.get(c, x, y, z)?;
                        if c.is_air(s) {
                            continue;
                        }
                    }
                    if inside && !self.is_interior(c, x, y, z)? {
                        continue;
                    }
                    let interior = y != y0 && y != y1 && x != x0 && x != x1 && z != z0 && z != z1;
                    self.place(c, if interior { fill } else { edge }, x, y, z)?;
                }
            }
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub fn maybe_block(
        &self,
        c: &mut PieceContext<'_>,
        probability: f32,
        x: i32,
        y: i32,
        z: i32,
        state: BlockStateId,
    ) -> Result<()> {
        if c.random.float() < probability {
            self.place(c, state, x, y, z)?;
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub fn upper_half_sphere(
        &self,
        c: &mut PieceContext<'_>,
        [x0, y0, z0]: [i32; 3],
        [x1, y1, z1]: [i32; 3],
        fill: BlockStateId,
        skip_air: bool,
    ) -> Result<()> {
        let dx = (x1 - x0 + 1) as f32;
        let dy = (y1 - y0 + 1) as f32;
        let dz = (z1 - z0 + 1) as f32;
        let cx = x0 as f32 + dx / 2.;
        let cz = z0 as f32 + dz / 2.;
        for y in y0..=y1 {
            let ny = (y - y0) as f32 / dy;
            for x in x0..=x1 {
                let nx = (x as f32 - cx) / (dx * 0.5);
                for z in z0..=z1 {
                    let nz = (z as f32 - cz) / (dz * 0.5);
                    if skip_air {
                        let s = self.get(c, x, y, z)?;
                        if c.is_air(s) {
                            continue;
                        }
                    }
                    if nx * nx + ny * ny + nz * nz <= 1.05 {
                        self.place(c, fill, x, y, z)?;
                    }
                }
            }
        }
        Ok(())
    }
    pub fn fill_column_down(
        &self,
        c: &mut PieceContext<'_>,
        state: BlockStateId,
        x: i32,
        y: i32,
        z: i32,
    ) -> Result<()> {
        let mut pos = self.world_pos(x, y, z);
        if !c.chunk_box.is_inside(pos) {
            return Ok(());
        }
        let min = c.world.min_y()?;
        loop {
            let current = c.world.block(pos)?;
            let replaceable = c.is_air(current)
                || c.is_liquid(current)
                || matches!(
                    c.name(current),
                    "minecraft:glow_lichen" | "minecraft:seagrass" | "minecraft:tall_seagrass"
                );
            if !replaceable || pos[1] <= min + 1 {
                break;
            }
            c.world.set(pos, state, 2)?;
            pos[1] -= 1;
        }
        Ok(())
    }
    /// StructurePiece.createChest at piece coordinates.
    pub fn create_chest(
        &self,
        c: &mut PieceContext<'_>,
        x: i32,
        y: i32,
        z: i32,
        table: &str,
    ) -> Result<bool> {
        let pos = self.world_pos(x, y, z);
        create_chest(c, pos, table, None)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn create_dispenser(
        &self,
        c: &mut PieceContext<'_>,
        x: i32,
        y: i32,
        z: i32,
        facing: Direction,
        table: &str,
    ) -> Result<bool> {
        let pos = self.world_pos(x, y, z);
        if !c.chunk_box.is_inside(pos) || c.is_block(pos, "minecraft:dispenser")? {
            return Ok(false);
        }
        let state = c.state_with("dispenser", &[("facing", direction_name(facing))]);
        self.place(c, state, x, y, z)?;
        c.loot_table(pos, "minecraft:dispenser", table)?;
        Ok(true)
    }
}
pub fn direction_name(d: Direction) -> &'static str {
    match d {
        Direction::Down => "down",
        Direction::Up => "up",
        Direction::North => "north",
        Direction::South => "south",
        Direction::West => "west",
        Direction::East => "east",
    }
}
pub fn offset(pos: [i32; 3], d: Direction, n: i32) -> [i32; 3] {
    let s = d.normal();
    [pos[0] + s[0] * n, pos[1] + s[1] * n, pos[2] + s[2] * n]
}
/// `BlockState.isSolidRender` of a state: whether it draws as a solid block.
pub fn solid_render(c: &PieceContext<'_>, state: BlockStateId) -> bool {
    c.blocks.solid_render(state)
}
/// StructurePiece.createChest with an absolute position.
pub fn create_chest(
    c: &mut PieceContext<'_>,
    pos: [i32; 3],
    table: &str,
    state: Option<BlockStateId>,
) -> Result<bool> {
    if !c.chunk_box.is_inside(pos) || c.is_block(pos, "minecraft:chest")? {
        return Ok(false);
    }
    let state = match state {
        Some(s) => s,
        None => reorient(c, pos, c.state("chest"))?,
    };
    c.world.set(pos, state, 2)?;
    c.loot_table(pos, "minecraft:chest", table)?;
    Ok(true)
}
/// `StructurePiece.reorient` for chests: the facing a chest takes from the
/// blocks around it.
fn reorient(c: &mut PieceContext<'_>, pos: [i32; 3], state: BlockStateId) -> Result<BlockStateId> {
    let mut states = [BlockStateId(0); 4];
    for (index, d) in HORIZONTAL.iter().enumerate() {
        states[index] = c.world.block(offset(pos, *d, 1))?;
    }
    let facing = match c.property(state, "facing") {
        Some("south") => Direction::South,
        Some("east") => Direction::East,
        Some("west") => Direction::West,
        _ => Direction::North,
    };
    let blocks = c.blocks;
    let is_chest = |s: BlockStateId| c.name(s) == "minecraft:chest";
    let Some(facing) = reorient_facing(facing, states, &is_chest, blocks) else {
        return Ok(state);
    };
    Ok(c.with_property(state, "facing", direction_name(facing)))
}
/// `StructurePiece.reorient`: the facing of a chest, from the blocks around
/// it. `states` are the four horizontal neighbours (north, east, south, west).
/// `is_chest` tells whether a state is a chest, and `blocks` whether it draws
/// as solid (`BlockState.isSolidRender`). None: a chest is next to it, and
/// the state is kept.
fn reorient_facing(
    facing: Direction,
    states: [BlockStateId; 4],
    is_chest: &dyn Fn(BlockStateId) -> bool,
    blocks: &dyn SolidRender,
) -> Option<Direction> {
    let index = |d: Direction| match d {
        Direction::North => 0,
        Direction::East => 1,
        Direction::South => 2,
        Direction::West => 3,
        _ => 0,
    };
    let mut solid_neighbour: Option<Direction> = None;
    for (i, d) in HORIZONTAL.iter().enumerate() {
        if is_chest(states[i]) {
            return None;
        }
        if blocks.solid_render(states[i]) {
            if solid_neighbour.is_some() {
                solid_neighbour = None;
                break;
            }
            solid_neighbour = Some(*d);
        }
    }
    if let Some(d) = solid_neighbour {
        return Some(opposite(d));
    }
    let mut lock = facing;
    if blocks.solid_render(states[index(lock)]) {
        lock = opposite(lock);
    }
    if blocks.solid_render(states[index(lock)]) {
        lock = clockwise(lock);
    }
    if blocks.solid_render(states[index(lock)]) {
        lock = opposite(lock);
    }
    Some(lock)
}
pub fn opposite(d: Direction) -> Direction {
    match d {
        Direction::Down => Direction::Up,
        Direction::Up => Direction::Down,
        Direction::North => Direction::South,
        Direction::South => Direction::North,
        Direction::West => Direction::East,
        Direction::East => Direction::West,
    }
}
pub fn clockwise(d: Direction) -> Direction {
    match d {
        Direction::North => Direction::East,
        Direction::East => Direction::South,
        Direction::South => Direction::West,
        Direction::West => Direction::North,
        other => other,
    }
}
pub fn counter_clockwise(d: Direction) -> Direction {
    match d {
        Direction::North => Direction::West,
        Direction::West => Direction::South,
        Direction::South => Direction::East,
        Direction::East => Direction::North,
        other => other,
    }
}

/// TemplateStructurePiece: one template with fixed settings; data markers
/// are handed to the owning structure.
/// NBT for an entity a structure piece spawns at the block centre of `pos`
/// (y at the block's floor): `finalize` asks the game for `finalizeSpawn`
/// (STRUCTURE), `persistent` sets `PersistenceRequired`.
pub fn spawn_nbt(id: &str, pos: [i32; 3], finalize: bool, persistent: bool) -> Compound {
    let mut nbt = Compound::new();
    nbt.insert("id".into(), Tag::String(id.into()));
    let p = [
        f64::from(pos[0]) + 0.5,
        f64::from(pos[1]),
        f64::from(pos[2]) + 0.5,
    ];
    nbt.insert(
        "Pos".into(),
        Tag::List(p.iter().map(|v| Tag::Double(v.to_bits())).collect()),
    );
    nbt.insert(
        "Rotation".into(),
        Tag::List(vec![Tag::Float(0f32.to_bits()), Tag::Float(0f32.to_bits())]),
    );
    if persistent {
        nbt.insert("PersistenceRequired".into(), Tag::Byte(1));
    }
    if finalize {
        nbt.insert("horizon:finalize".into(), Tag::Byte(1));
    }
    nbt
}
#[derive(Debug)]
pub struct TemplatePiece {
    pub type_name: &'static str,
    pub template_name: String,
    pub template: Arc<StructureTemplate>,
    pub position: [i32; 3],
    pub rotation: Rotation,
    pub mirror: Mirror,
    pub pivot: [i32; 3],
    pub processors: Arc<ProcessorList>,
    pub waterlogging: bool,
    /// Ignore structure blocks (and air, for ocean ruins) before processors.
    pub ignore: Vec<String>,
    pub bounding_box: BoundingBox,
    pub handler: TemplateHandler,
    /// Per-piece state mutated by vanilla post-processing (e.g. ruined
    /// portals record nothing; shipwrecks/igloos re-place at a height).
    pub state: Mutex<Option<i32>>,
}
#[derive(Clone, Copy, Debug)]
pub enum TemplateHandler {
    Igloo,
    Shipwreck {
        beached: bool,
    },
    OceanRuin {
        warm: bool,
        large: bool,
        integrity: f32,
    },
    RuinedPortal(super::ruined_portal::PortalProps),
    Mansion,
    Plain,
}
impl TemplatePiece {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        type_name: &'static str,
        template_name: &str,
        template: Arc<StructureTemplate>,
        position: [i32; 3],
        rotation: Rotation,
        mirror: Mirror,
        pivot: [i32; 3],
        processors: Arc<ProcessorList>,
        waterlogging: bool,
        ignore: Vec<String>,
        handler: TemplateHandler,
    ) -> Self {
        Self {
            bounding_box: template.bounding_box(position, rotation, pivot, mirror),
            type_name,
            template_name: template_name.to_owned(),
            template,
            position,
            rotation,
            mirror,
            pivot,
            processors,
            waterlogging,
            ignore,
            handler,
            state: Mutex::new(None),
        }
    }
    pub fn settings(&self) -> PlaceSettings<'_> {
        let mut s = PlaceSettings::new(self.rotation);
        s.mirror = self.mirror;
        s.pivot = self.pivot;
        s.waterlogging = self.waterlogging;
        // Mansions and end cities spawn their mobs from data markers.
        s.ignore_entities = self.template_name.contains("woodland_mansion/")
            || self.template_name.contains("end_city/");
        s
    }
    /// TemplateStructurePiece.postProcess at a (possibly shifted) position;
    /// returns the data markers (metadata, absolute position) on success.
    pub fn place_at(
        &self,
        c: &mut PieceContext<'_>,
        position: [i32; 3],
        extra: &[Processor],
    ) -> Result<Option<Markers>> {
        let ignore = Processor::BlockIgnore(self.ignore.iter().cloned().collect());
        let mut settings = self.settings();
        settings.bounding_box = Some(c.chunk_box);
        settings.processors.push(&ignore);
        settings.processors.extend(extra.iter());
        settings.processors.extend(self.processors.0.iter());
        let resources = c.resources;
        let reference = c.reference;
        if !c.world.place_template(
            resources,
            &self.template,
            position,
            reference,
            &mut settings,
            &mut *c.random,
            2,
        )? {
            return Ok(None);
        }
        let mut markers = Vec::new();
        let mut jigsaws = Vec::new();
        for info in filter_blocks(resources, &self.template, position, &settings) {
            let name = c.name(info.state).to_owned();
            let Some(nbt) = &info.nbt else { continue };
            let text = |key: &str| match nbt.get(&JavaString::from(key)) {
                Some(Tag::String(s)) => s.to_string_lossy(),
                _ => String::new(),
            };
            if name == "minecraft:structure_block" && text("mode").eq_ignore_ascii_case("data") {
                markers.push((text("metadata"), info.pos));
            } else if name == "minecraft:jigsaw" {
                let final_state = nbt
                    .get(&JavaString::from("final_state"))
                    .map(|_| text("final_state"))
                    .unwrap_or_else(|| "minecraft:air".into());
                jigsaws.push((info.pos, final_state));
            }
        }
        for (pos, final_state) in jigsaws {
            let state = super::template::parse_state_string(resources.registry, &final_state)
                .unwrap_or_else(|| c.state("air"));
            c.world.set(pos, state, 3)?;
        }
        Ok(Some(markers))
    }
}
/// StructureTemplate.filterBlocks for structure and jigsaw blocks (palette
/// chosen by settings' random or a position seed), within the settings box.
fn filter_blocks(
    resources: &StructureResources,
    template: &StructureTemplate,
    position: [i32; 3],
    settings: &PlaceSettings<'_>,
) -> Vec<BlockInfo> {
    if template.palettes().is_empty() {
        return Vec::new();
    }
    let palette = LegacyRandom::new(position_seed(BlockPos::new(
        position[0],
        position[1],
        position[2],
    )))
    .next_int_bound(template.palettes().len() as i32)
    .unwrap_or(0) as usize;
    template.palettes()[palette]
        .blocks()
        .iter()
        .filter(|b| {
            let name = resources
                .registry
                .state(b.state)
                .map(|s| s.name.as_str())
                .unwrap_or("");
            name == "minecraft:structure_block" || name == "minecraft:jigsaw"
        })
        .filter_map(|b| {
            let relative = StructureTemplate::transform(
                b.pos,
                settings.mirror,
                settings.rotation,
                settings.pivot,
            );
            let pos: [i32; 3] = std::array::from_fn(|i| relative[i] + position[i]);
            settings
                .bounding_box
                .is_none_or(|bb| bb.is_inside(pos))
                .then(|| BlockInfo {
                    pos,
                    state: resources.transforms.rotate(b.state, settings.rotation),
                    nbt: b.nbt.clone(),
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const AIR: BlockStateId = BlockStateId(0);
    const GLASS: BlockStateId = BlockStateId(1);
    const STONE: BlockStateId = BlockStateId(2);
    const CHEST: BlockStateId = BlockStateId(3);

    /// `BlockState.isSolidRender`: only the stone draws as a solid block;
    /// glass, leaves, ice, a spawner and a barrier do not.
    struct Blocks;
    impl SolidRender for Blocks {
        fn solid_render(&self, state: BlockStateId) -> bool {
            state == STONE
        }
    }

    /// `StructurePiece.reorient`: the chest's facing from the blocks around
    /// it, as vanilla has it. The solidity is `BlockState.isSolidRender`:
    /// glass is not solid to render, stone is.
    #[test]
    fn a_chest_next_to_glass_faces_away_from_the_stone() {
        // North: glass, east: stone.
        assert_eq!(
            reorient_facing(
                Direction::North,
                [GLASS, STONE, AIR, AIR],
                &|s| s == CHEST,
                &Blocks
            ),
            Some(Direction::West)
        );
    }

    /// Two solid neighbours: the chest keeps its own facing, turned away from
    /// the one it faced.
    #[test]
    fn two_solid_neighbours_turn_the_chest_away_from_one_of_them() {
        // North and east are solid.
        assert_eq!(
            reorient_facing(
                Direction::North,
                [STONE, STONE, AIR, AIR],
                &|s| s == CHEST,
                &Blocks
            ),
            Some(Direction::South)
        );
    }

    /// A chest next to it: the state is kept.
    #[test]
    fn a_chest_next_to_a_chest_keeps_its_facing() {
        assert_eq!(
            reorient_facing(
                Direction::North,
                [CHEST, AIR, AIR, AIR],
                &|s| s == CHEST,
                &Blocks
            ),
            None
        );
    }
}
