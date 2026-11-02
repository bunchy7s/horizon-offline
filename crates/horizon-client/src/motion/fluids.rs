//! The player's fluid immersion over the resident scene: `Entity.updateFluidInteraction`
//! (water, waterlogged blocks, bubble columns and lava, with their currents), the
//! water-entry splash and the fall resets that hinge on touching water.
use super::*;
use horizon_world::{
    entity_fluid::{
        CurrentTarget, EntityFluidInteraction, FluidInteractionWorld, FluidMembership,
        FluidSections, FluidTagId,
    },
    entity_fluid_tick::{self, EntityFluidTickWorld},
    fluid::{FluidHeightWorld, FluidKind, FluidState, FluidStateTable},
    fluid_flow::{FluidFlowWorld, flow_at},
    fluid_shape::FluidShapeCache,
    water_splash::{self, SplashParticle, WaterSplashWorld},
};
use std::sync::Mutex;

/// `FluidTags.WATER` and `FluidTags.LAVA`, the fluids with a current.
pub const WATER: FluidTagId = FluidTagId(0);
pub const LAVA: FluidTagId = FluidTagId(1);

/// Per-state fluid facts the player's immersion and fall-reset rays read.
pub struct FluidResources {
    states: FluidStateTable,
    /// `isFaceSturdy(FULL)` per `Direction` (bit = ordinal).
    sturdy: Vec<u8>,
    /// `#minecraft:blocks_fluid_flow` membership.
    blocks_flow: Vec<bool>,
    /// `IceBlock` and its subclasses.
    ice: Vec<bool>,
    /// `FlowingFluid.shapes`: the shape of the first query of each fluid
    /// state, kept for the whole process like the registered fluid's map.
    pub shapes: Mutex<FluidShapeCache>,
}
impl FluidResources {
    pub fn new(assets: &Assets, facts: &crate::blocks::BlockFacts) -> Result<Self> {
        Ok(Self {
            states: FluidStateTable::new(assets.registry)?,
            sturdy: facts.states.iter().map(|s| s.sturdy_full).collect(),
            blocks_flow: facts
                .states
                .iter()
                .map(|s| assets.blocks_fluid_flow.contains(&*s.name))
                .collect(),
            ice: facts
                .states
                .iter()
                .map(|s| s.has_class("IceBlock"))
                .collect(),
            shapes: Mutex::new(FluidShapeCache::default()),
        })
    }
    pub fn fluid(&self, state: BlockStateId) -> FluidState {
        self.states.get(state).unwrap_or(FluidState::EMPTY)
    }
    /// `BlockState.isFaceSturdy(FULL)` on that face.
    pub fn sturdy(&self, state: BlockStateId, d: Direction) -> bool {
        self.sturdy
            .get(state.0 as usize)
            .is_some_and(|s| s & 1 << d as u8 != 0)
    }
    /// `#minecraft:blocks_fluid_flow` membership.
    pub fn blocks_flow(&self, state: BlockStateId) -> bool {
        self.blocks_flow
            .get(state.0 as usize)
            .copied()
            .unwrap_or(false)
    }
    /// `IceBlock` or one of its subclasses.
    pub fn ice(&self, state: BlockStateId) -> bool {
        self.ice.get(state.0 as usize).copied().unwrap_or(false)
    }
}
/// The player's `EntityFluidInteraction` and the water flags around it.
#[derive(Clone)]
pub struct PlayerFluids {
    pub interaction: EntityFluidInteraction,
    /// `Entity.wasTouchingWater` (`isInWater`).
    pub was_touching_water: bool,
    /// `Entity.wasEyeInWater`.
    pub was_eye_in_water: bool,
    /// `Entity.firstTick`: no splash, and no lava, until the first base tick ends.
    pub first_tick: bool,
    /// The dimension's `FAST_LAVA` (the Nether), set by the game.
    pub fast_lava: bool,
    /// The boat the player sits in (`Entity.getVehicle`), set by the game.
    pub boat: Option<RiddenBoat>,
}
/// What `AbstractBoat.modifyPassengerFluidInteractionBox` reads of a boat.
#[derive(Clone, Copy, Debug)]
pub struct RiddenBoat {
    /// `AbstractBoat.isUnderWater`.
    pub under_water: bool,
    /// The top of the boat's box (`getBoundingBox().maxY`).
    pub max_y: f64,
}
impl Default for PlayerFluids {
    fn default() -> Self {
        Self {
            interaction: EntityFluidInteraction::new([WATER, LAVA]),
            was_touching_water: false,
            was_eye_in_water: false,
            first_tick: true,
            fast_lava: false,
            boat: None,
        }
    }
}
fn is_tagged(kind: FluidKind, tag: FluidTagId) -> bool {
    if tag == WATER {
        matches!(kind, FluidKind::Water | FluidKind::FlowingWater)
    } else {
        matches!(kind, FluidKind::Lava | FluidKind::FlowingLava)
    }
}
impl PlayerMotion {
    pub fn fluid_height(&self, tag: FluidTagId) -> f64 {
        self.fluids.interaction.fluid_height(&FluidTags, tag)
    }
    pub fn is_eye_in_fluid(&self, tag: FluidTagId) -> bool {
        self.fluids.interaction.is_eye_in_fluid(&FluidTags, tag)
    }
    /// `LocalPlayer.isUnderWater`: `wasUnderwater`, the eye sample `Player.tick`
    /// takes before the base tick (`Entity.isUnderWater` also needs the body
    /// still in water).
    pub fn is_under_water(&self) -> bool {
        self.fluids.was_eye_in_water
    }
    /// `Entity.isInLava`.
    pub fn is_in_lava(&self) -> bool {
        !self.fluids.first_tick && self.fluids.interaction.is_in_fluid(&FluidTags, LAVA)
    }
}
/// `BubbleColumnBlock.entityInside` for an entity that is not the player (a mob,
/// a dropped item, an ender pearl): the bubble columns its box touches push it
/// up or drag it down, harder at the surface (`Entity.handleOnAboveBubbleColumn`
/// and `handleOnInsideBubbleColumn`). Returns whether it was inside a column
/// below the surface, which resets its fall distance.
pub fn bubble_column_push(
    world: &World,
    registry: &horizon_world::registry::BlockRegistry,
    facts: &crate::blocks::BlockFacts,
    collision: &horizon_world::block_collision::BlockCollisionRegistry<'_>,
    bounds: Aabb,
    velocity: &mut Vec3,
) -> bool {
    bubble_column_push_over(world, registry, facts, collision, bounds, velocity, |_| {})
}
/// [`bubble_column_push`], telling `surface` the position of each column block
/// the entity is at the top of: `handleOnAboveBubbleColumn` sends the splash and
/// bubble particles over it (`Entity.sendBubbleColumnParticles`).
pub fn bubble_column_push_over(
    world: &World,
    registry: &horizon_world::registry::BlockRegistry,
    facts: &crate::blocks::BlockFacts,
    collision: &horizon_world::block_collision::BlockCollisionRegistry<'_>,
    bounds: Aabb,
    velocity: &mut Vec3,
    mut surface: impl FnMut(BlockPos),
) -> bool {
    let mut reset_fall = false;
    bubble_columns_touching(world, registry, facts, collision, bounds, |pos, column| {
        reset_fall |= column.push(velocity);
        if column.surface {
            surface(pos);
        }
    });
    reset_fall
}
/// The column blocks `bounds` touches that it is at the top of: where the
/// particles of `Entity.sendBubbleColumnParticles` start.
pub fn bubble_column_surfaces(
    world: &World,
    registry: &horizon_world::registry::BlockRegistry,
    facts: &crate::blocks::BlockFacts,
    collision: &horizon_world::block_collision::BlockCollisionRegistry<'_>,
    bounds: Aabb,
) -> Vec<BlockPos> {
    let mut surfaces = Vec::new();
    bubble_columns_touching(world, registry, facts, collision, bounds, |pos, column| {
        if column.surface {
            surfaces.push(pos);
        }
    });
    surfaces
}
/// Every bubble column block the box touches (`Entity.getInsideBlockPoints`).
fn bubble_columns_touching(
    world: &World,
    registry: &horizon_world::registry::BlockRegistry,
    facts: &crate::blocks::BlockFacts,
    collision: &horizon_world::block_collision::BlockCollisionRegistry<'_>,
    bounds: Aabb,
    mut each: impl FnMut(BlockPos, BubbleColumn),
) {
    let b = bounds.deflate(Vec3::new(
        f64::from(1.0e-5f32),
        f64::from(1.0e-5f32),
        f64::from(1.0e-5f32),
    ));
    let (lo, hi) = (b.min_position(), b.max_position());
    for x in lo.x.floor() as i32..=(hi.x.ceil() as i32 - 1) {
        for y in lo.y.floor() as i32..=(hi.y.ceil() as i32 - 1) {
            for z in lo.z.floor() as i32..=(hi.z.ceil() as i32 - 1) {
                let pos = BlockPos::new(x, y, z);
                if let Some(column) = bubble_column_at(world, registry, facts, collision, pos) {
                    each(pos, column);
                }
            }
        }
    }
}
/// A bubble column block as `BubbleColumnBlock.entityInside` sees it: whether
/// it drags down (`DRAG_DOWN`), and whether it is the surface (nothing above it
/// to collide with and no fluid there).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BubbleColumn {
    pub drag: bool,
    pub surface: bool,
}
impl BubbleColumn {
    /// `Entity.handleOnAboveBubbleColumn` at the surface (it throws harder),
    /// else `handleOnInsideBubbleColumn`. Returns whether it was inside below
    /// the surface, which resets the fall distance.
    pub fn push(self, velocity: &mut Vec3) -> bool {
        velocity.y = match (self.surface, self.drag) {
            (true, true) => (-0.9_f64).max(velocity.y - 0.03),
            (true, false) => 1.8_f64.min(velocity.y + 0.1),
            (false, true) => (-0.3_f64).max(velocity.y - 0.03),
            (false, false) => 0.7_f64.min(velocity.y + 0.06),
        };
        !self.surface
    }
}
/// The bubble column at `pos`, if there is one.
pub fn bubble_column_at(
    world: &World,
    registry: &horizon_world::registry::BlockRegistry,
    facts: &crate::blocks::BlockFacts,
    collision: &horizon_world::block_collision::BlockCollisionRegistry<'_>,
    pos: BlockPos,
) -> Option<BubbleColumn> {
    let state = world.block(pos.x, pos.y, pos.z);
    if &*facts.get(state).name != "minecraft:bubble_column" {
        return None;
    }
    let drag = registry
        .state(state)
        .and_then(|s| s.properties.get("drag"))
        .is_some_and(|d| d == "true");
    let above = world.block(pos.x, pos.y + 1, pos.z);
    // stateAbove.getCollisionShape(level, pos): the shape is asked
    // at the column's own position.
    let mut scene = SceneCollisions {
        world,
        binding: collision,
        subject: None,
    };
    let context = horizon_world::collision_context::CollisionContext::empty(false);
    let surface = collision
        .collision_shape_with_context(&mut scene, above, pos, &context)
        .is_ok_and(|shape| shape.is_empty())
        && facts.get(above).fluid == crate::blocks::Fluid::None;
    Some(BubbleColumn { drag, surface })
}
/// Which fluids `FluidTags.WATER` and `FluidTags.LAVA` hold.
pub struct FluidTags;
impl FluidMembership for FluidTags {
    fn fluid_in_tag(&self, kind: FluidKind, tag: FluidTagId) -> bool {
        is_tagged(kind, tag)
    }
}
impl MotionWorld<'_, '_, '_> {
    pub(super) fn fluid_at(&self, p: BlockPos) -> FluidState {
        self.resources.fluids.fluid(self.block(p))
    }
    /// `Entity.baseTick`'s fluid phase: `wasEyeInWater`, `updateFluidInteraction`,
    /// swimming and remaining fire, lava's fall halving, then the first-tick clear.
    pub fn fluid_base_tick(&mut self) -> Result<()> {
        self.player.fluids.was_eye_in_water = self.player.is_eye_in_fluid(WATER);
        self.update_fluids()?;
        // `Player.updateSwimming`'s flying guard over `Entity.updateSwimming`,
        // whose both branches need `!isPassenger()`: a rider keeps no swim, so
        // the server's copy of a guest who boards a boat while swimming is not
        // left in the swimming box (eye 0.4) in the seat, at the waterline,
        // where the boat drowned and suffocated them (#4427).
        self.player.swimming = !self.player.flying
            && !self.player.is_passenger
            && self.player.sprinting
            && if self.player.swimming {
                self.player.fluids.was_touching_water
            } else {
                self.player.is_under_water()
                    && is_tagged(
                        self.fluid_at(BlockPos::containing(self.player.position))
                            .kind(),
                        WATER,
                    )
            };
        self.tick_remaining_fire()?;
        if self.player.is_in_lava() {
            self.player.fall_distance *= 0.5;
        }
        self.player.fluids.first_tick = false;
        Ok(())
    }
    /// `Entity.updateFluidInteraction`: immersion heights, then touching water
    /// resets the fall and splashes on entry, then the currents push.
    pub fn update_fluids(&mut self) -> Result<bool> {
        let mut interaction = std::mem::take(&mut self.player.fluids.interaction);
        let result = entity_fluid_tick::update(&mut interaction, self, WATER, LAVA, 1_000_000);
        self.player.fluids.interaction = interaction;
        result.map_err(|e| e.to_string().into())
    }
    /// `ClipContext.Fluid.WATER` in a fall-reset ray: the water's shape.
    pub(super) fn water_clip(&mut self, from: Vec3, to: Vec3, p: BlockPos) -> Result<bool> {
        let fluid = self.fluid_at(p);
        if !is_tagged(fluid.kind(), WATER) {
            return Ok(false);
        }
        let resources = self.resources;
        let shape = resources
            .fluids
            .shapes
            .lock()
            .map_err(|_| "fluid shape cache poisoned")?
            .shape(self, fluid, p)?;
        Ok(shape.clip(from, to, p).is_some())
    }
}
impl FluidMembership for MotionWorld<'_, '_, '_> {
    fn fluid_in_tag(&self, kind: FluidKind, tag: FluidTagId) -> bool {
        is_tagged(kind, tag)
    }
}
impl CurrentTarget for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn is_player_class(&self) -> bool {
        true
    }
    fn velocity(&mut self) -> Result<Vec3> {
        Ok(self.player.velocity)
    }
    fn add_delta_movement(&mut self, delta: Vec3) -> Result<()> {
        // Entity.addDeltaMovement and setDeltaMovement keep only finite values.
        let v = self.player.velocity;
        let sum = Vec3::new(v.x + delta.x, v.y + delta.y, v.z + delta.z);
        if delta.is_finite() && sum.is_finite() {
            self.player.velocity = sum;
        }
        Ok(())
    }
}
impl FluidInteractionWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    type Chunk = ();
    fn interaction_box(&mut self) -> Result<Option<Aabb>> {
        // Entity.getFluidInteractionBox: the box less 0.001, then the vehicle's
        // say (`AbstractBoat.modifyPassengerFluidInteractionBox`).
        let b = self.player.bounds().deflate(Vec3::new(0.001, 0.001, 0.001));
        let Some(boat) = self.player.fluids.boat.filter(|boat| !boat.under_water) else {
            return Ok(Some(b));
        };
        let (lo, hi) = (b.min_position(), b.max_position());
        // A boat that reaches the passenger's head shelters all of it; else
        // only what lies above the boat's top counts.
        if boat.max_y >= hi.y {
            return Ok(None);
        }
        Ok(Some(Aabb::new(
            Vec3::new(lo.x, lo.y.max(boat.max_y), lo.z),
            hi,
        )))
    }
    fn loaded_chunk(&mut self, x: i32, z: i32) -> Result<Option<()>> {
        Ok(self.scene.chunks.contains_key(&(x, z)).then_some(()))
    }
    fn chunk_sections(&mut self, _: ()) -> Result<FluidSections<'_>> {
        // Resident sections keep no fluid count: scanning the box finds no
        // fluid where a section without fluid would have skipped it.
        let layout = self.scene.layout();
        Ok(FluidSections {
            minimum_section_y: layout.min_y() >> 4,
            has_fluid: crate::chunk::every_section(layout),
        })
    }
    fn bounding_box_min_y(&self) -> f64 {
        self.player.bounds().min_position().y
    }
    fn block_x(&self) -> i32 {
        self.player.position.x.floor() as i32
    }
    fn eye_y(&mut self) -> Result<f64> {
        Ok(self.player.position.y + f64::from(self.player.dimensions.eye_height))
    }
    fn block_z(&self) -> i32 {
        self.player.position.z.floor() as i32
    }
    fn fluid_state(&mut self, p: BlockPos) -> Result<FluidState> {
        Ok(self.fluid_at(p))
    }
    fn fluid_height(&mut self, f: FluidState, p: BlockPos) -> Result<f32> {
        f.height_at(self, [p.x, p.y, p.z])
    }
    fn camera_height(&mut self, f: FluidState, p: BlockPos) -> Result<f32> {
        f.camera_height_at(self, [p.x, p.y, p.z])
    }
    fn flow(&mut self, f: FluidState, p: BlockPos) -> Result<Vec3> {
        flow_at(self, f, p)
    }
}
impl EntityFluidTickWorld for MotionWorld<'_, '_, '_> {
    /// `Player.isPushedByFluid`: not while flying.
    fn is_pushed_by_fluid(&mut self) -> Result<bool> {
        Ok(!self.player.flying)
    }
    fn reset_fall_distance(&mut self) -> Result<()> {
        self.player.fall_distance = 0.0;
        Ok(())
    }
    fn was_touching_water(&self) -> bool {
        self.player.fluids.was_touching_water
    }
    fn first_tick(&self) -> bool {
        self.player.fluids.first_tick
    }
    fn water_splash(&mut self) -> Result<()> {
        // `Player.doWaterSplashEffect`: spectators skip the effect (and its random draws).
        if self.player.spectator {
            return Ok(());
        }
        water_splash::water_splash(self, 1_000_000).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn set_was_touching_water(&mut self, value: bool) {
        self.player.fluids.was_touching_water = value;
    }
    fn fast_lava(&mut self) -> Result<bool> {
        Ok(self.player.fluids.fast_lava)
    }
}
impl FluidHeightWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn fluid(&mut self, p: [i32; 3]) -> Result<FluidState> {
        Ok(self.fluid_at(BlockPos::new(p[0], p[1], p[2])))
    }
    fn sturdy_down(&mut self, p: [i32; 3]) -> Result<bool> {
        let state = MotionWorld::block(self, BlockPos::new(p[0], p[1], p[2]));
        Ok(self.resources.fluids.sturdy(state, Direction::Down))
    }
}
impl FluidFlowWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    type Block = BlockStateId;
    fn fluid(&mut self, p: BlockPos) -> Result<FluidState> {
        Ok(self.fluid_at(p))
    }
    fn block(&mut self, p: BlockPos) -> Result<BlockStateId> {
        Ok(MotionWorld::block(self, p))
    }
    fn blocks_fluid_flow(&self, s: BlockStateId) -> bool {
        self.resources.fluids.blocks_flow(s)
    }
    fn is_ice_class(&self, s: BlockStateId) -> bool {
        self.resources.fluids.ice(s)
    }
    fn is_face_sturdy(&mut self, s: BlockStateId, _: BlockPos, d: Direction) -> Result<bool> {
        Ok(self.resources.fluids.sturdy(s, d))
    }
}
/// `Entity.doWaterSplashEffect` for the player: the world random draws the
/// pitch and particle offsets.
impl WaterSplashWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    type Entity = ();
    /// `Player.getSwimHighSpeedSplashSound` when true, else `getSwimSplashSound`.
    type Sound = bool;
    fn this_entity(&self) {}
    fn controlling_passenger(&mut self) -> Result<Option<()>> {
        Ok(None)
    }
    fn velocity(&mut self, _: ()) -> Result<Vec3> {
        Ok(self.player.velocity)
    }
    fn splash_sound(&mut self, high_speed: bool) -> Result<bool> {
        Ok(high_speed)
    }
    fn random_float(&mut self) -> Result<f32> {
        Ok(self.scene.random.next_float())
    }
    fn random_double(&mut self) -> Result<f64> {
        Ok(self.scene.random.next_double())
    }
    fn play_sound(&mut self, high_speed: bool, volume: f32, pitch: f32) -> Result<()> {
        self.player.events.push(MotionEvent::Splash {
            high_speed,
            volume,
            pitch,
        });
        Ok(())
    }
    fn position(&self) -> Vec3 {
        self.player.position
    }
    fn width(&self) -> f32 {
        self.player.dimensions.width
    }
    fn particle(&mut self, kind: SplashParticle, position: Vec3, velocity: Vec3) -> Result<()> {
        self.player.events.push(MotionEvent::SplashParticle {
            bubble: matches!(kind, SplashParticle::Bubble),
            position,
            velocity,
        });
        Ok(())
    }
    fn splash_event(&mut self) -> Result<()> {
        // The SPLASH game event: sculk listens to the player's moves itself.
        self.player.events.push(MotionEvent::SplashEvent);
        Ok(())
    }
}
