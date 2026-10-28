//! `AbstractArrow.tick` for arrows, tipped and spectral arrows: the flight with
//! its water inertia and bubbles, the base tick's fluids (currents, splash),
//! bubble columns and water along the path, sticking in blocks, falling out
//! when the block goes, and despawning. The tick stops at an entity it hits:
//! the game hurts the entity (`projectile_hit`), then `finish_entity_hit`
//! spends the arrow or turns it back and ends the tick.
use crate::{
    blocks::BlockFacts,
    collisions::SceneCollisions,
    mobs::Who,
    motion::{FluidResources, LAVA, WATER},
    projectiles::{Hit, Projectile, ProjectileKind},
    world::World,
};
use horizon_core::{
    geometry::{Aabb, Direction, Vec3, sweep_intersected_blocks},
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
use horizon_world::{
    block_collision::BlockCollisionRegistry,
    entity_fluid::{
        CurrentTarget, EntityFluidInteraction, FluidInteractionWorld, FluidMembership,
        FluidSections, FluidTagId,
    },
    entity_fluid_tick::{self, EntityFluidTickWorld},
    fluid::{FluidHeightWorld, FluidKind, FluidState},
    fluid_flow::{FluidFlowWorld, flow_at},
    registry::BlockStateId,
    water_splash::{self, SplashParticle, WaterSplashWorld},
};
use std::{collections::HashSet, fmt};

/// `EntityType.ARROW` (and tipped/spectral arrows): 0.5 x 0.5, eyes at 0.13.
const WIDTH: f32 = 0.5;
const HEIGHT: f32 = 0.5;
const EYE_HEIGHT: f32 = 0.13;

/// `AbstractArrow.Pickup`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pickup {
    #[default]
    Disallowed,
    Allowed,
    CreativeOnly,
}
impl Pickup {
    /// The saved `pickup` byte (`LEGACY_CODEC`: the ordinal, out of range is
    /// DISALLOWED).
    pub fn from_byte(b: i64) -> Self {
        match b {
            1 => Self::Allowed,
            2 => Self::CreativeOnly,
            _ => Self::Disallowed,
        }
    }
    pub fn byte(self) -> i8 {
        self as i8
    }
}
/// `DataComponents.INTANGIBLE_PROJECTILE`.
pub const INTANGIBLE: &str = "minecraft:intangible_projectile";
/// `ProjectileWeaponItem.useAmmo`'s drawn stack: one of the ammo, marked
/// intangible when no ammo was used (creative, Infinity, multishot extras).
pub fn drawn_ammo(ammo: &crate::items::ItemStack, intangible: bool) -> crate::items::ItemStack {
    let mut drawn = crate::items::ItemStack {
        count: 1,
        ..ammo.clone()
    };
    if intangible {
        drawn.data_mut().set_other(
            INTANGIBLE,
            Some(horizon_world::nbt::Tag::Compound(Default::default())),
        );
    }
    drawn
}
/// What `AbstractArrow.playerTouch` does for a player touching the arrow.
#[derive(Clone, Debug, PartialEq)]
pub enum Touch {
    /// Not stuck yet, still shaking, or not the player's to take.
    Ignore,
    /// ALLOWED: taken if the inventory takes this stack whole.
    Give(crate::items::ItemStack),
    /// CREATIVE_ONLY in creative: taken without an item.
    Take,
}
/// What an arrow's tick asks the game to play or show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArrowEvent {
    /// `level().addParticle` (the client's copy of the arrow adds them).
    Particle {
        kind: ArrowParticle,
        position: Vec3,
        velocity: Vec3,
    },
    Sound {
        sound: ArrowSound,
        volume: f32,
        pitch: f32,
    },
    /// `Entity.sendBubbleColumnParticles` at this block: two SPLASH (speed 1)
    /// and two BUBBLE (spread 0, 0.01, 0, speed 0.2) particles at the top,
    /// placed by the level random (drawn by the consumer, nothing else draws
    /// it during an arrow's tick).
    BubbleColumnParticles {
        pos: BlockPos,
    },
    /// The block hit reacts (before the arrow sticks).
    Block {
        pos: BlockPos,
        reaction: BlockReaction,
    },
    /// The SPLASH and PROJECTILE_SHOOT game events.
    SplashEvent,
    ShootEvent,
    /// `spawnAtLocation(pickupItem, 0.1)`: a turned-back arrow too slow to fly
    /// on drops what picking it up gives.
    Drop,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrowParticle {
    Bubble,
    Crit,
    Splash,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrowSound {
    /// `entity.generic.splash` (an arrow's splash and high-speed splash).
    Splash,
    /// The arrow's `soundEvent` (`entity.arrow.hit`), for blocks and entities.
    HitGround,
    /// An arrow shot from a crossbow: `item.crossbow.hit`.
    CrossbowHit,
    /// `entity.generic.extinguish_fire`.
    Extinguish,
    /// A thrown trident's `getDefaultHitGroundSoundEvent` (`item.trident.hit_ground`).
    TridentHitGround,
    /// `ThrownTrident.onHitEntity`'s `item.trident.hit`.
    TridentHit,
}
/// What a burning arrow does to the block it hits (`onProjectileHit`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockReaction {
    /// `TntBlock.prime` (primed TNT, its sound and PRIME_FUSE), then
    /// `removeBlock(pos, false)`.
    PrimeTnt,
    /// `setBlock(pos, lit=true, 11)`: campfires and candles.
    Light(BlockStateId),
}
/// The arrow part of `AbstractArrow` and its `Entity` state.
#[derive(Clone)]
pub struct ArrowState {
    pub in_ground: bool,
    pub in_ground_time: i32,
    /// Ticks stuck (`life`): gone at 1200.
    pub life: i32,
    pub shake: i32,
    /// The block it stuck in (`lastState`).
    pub last_state: Option<BlockStateId>,
    pub pickup: Pickup,
    /// `pickupItemStack`: what picking the arrow up gives.
    pub pickup_item: crate::items::ItemStack,
    /// `AbstractArrow.firedFromWeapon`: the crossbow or bow it was shot from.
    pub fired_from_weapon: Option<crate::items::ItemStack>,
    pub no_physics: bool,
    /// `AbstractArrow.getPierceLevel`.
    pub pierce: u8,
    /// `piercingIgnoreEntityIds`: the entities a piercing arrow has gone
    /// through (cleared when it sticks in a block).
    pub pierced: Vec<Who>,
    /// `Entity.remainingFireTicks`.
    pub fire_ticks: i32,
    pub fall_distance: f64,
    pub fluids: EntityFluidInteraction,
    pub was_touching_water: bool,
    pub was_eye_in_water: bool,
    pub first_tick: bool,
    pub has_been_shot: bool,
    /// `ThrownTrident.dealtDamage`: it has hit an entity (or sat in a block
    /// for over 4 ticks) and hits no more.
    pub dealt_damage: bool,
    pub removed: bool,
    /// The arrow's own random (`Entity.random`).
    pub random: LegacyRandom,
    pub events: Vec<ArrowEvent>,
}
impl fmt::Debug for ArrowState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArrowState")
            .field("in_ground", &self.in_ground)
            .field("life", &self.life)
            .field("fire_ticks", &self.fire_ticks)
            .finish_non_exhaustive()
    }
}
impl PartialEq for ArrowState {
    fn eq(&self, other: &Self) -> bool {
        self.in_ground == other.in_ground
            && self.life == other.life
            && self.last_state == other.last_state
            && self.fire_ticks == other.fire_ticks
    }
}
impl ArrowState {
    pub fn new(random: LegacyRandom) -> Self {
        Self {
            in_ground: false,
            in_ground_time: 0,
            life: 0,
            shake: 0,
            last_state: None,
            pickup: Pickup::Disallowed,
            pickup_item: crate::items::ItemStack::new("minecraft:arrow", 1),
            fired_from_weapon: None,
            no_physics: false,
            pierce: 0,
            pierced: Vec::new(),
            fire_ticks: 0,
            fall_distance: 0.0,
            fluids: EntityFluidInteraction::new([WATER, LAVA]),
            was_touching_water: false,
            was_eye_in_water: false,
            first_tick: true,
            has_been_shot: false,
            dealt_damage: false,
            removed: false,
            random,
            events: Vec::new(),
        }
    }
    /// The `AbstractArrow` constructor's `pickupItemStack`: the drawn ammo
    /// (one item). Ammo drawn without being used up (`INTANGIBLE_PROJECTILE`)
    /// makes the arrow CREATIVE_ONLY; vanilla takes the component off the
    /// stack it was given, not its copy, so the item keeps it.
    pub fn set_item(&mut self, item: &crate::items::ItemStack) {
        self.pickup_item = item.clone();
        if item.data().and_then(|d| d.other(INTANGIBLE)).is_some() {
            self.pickup = Pickup::CreativeOnly;
        }
    }
    /// `AbstractArrow.setOwner`: a player's arrows may be picked up.
    pub fn set_owner(&mut self, owner: Option<Who>) {
        if owner == Some(Who::Player) && self.pickup == Pickup::Disallowed {
            self.pickup = Pickup::Allowed;
        }
    }
    /// `AbstractArrow.playerTouch` and `tryPickup` (`creative`:
    /// `Player.hasInfiniteMaterials`). Like vanilla it does not look at
    /// removal: players only touch arrows still in the world.
    pub fn player_touch(&self, creative: bool) -> Touch {
        if !(self.in_ground || self.no_physics) || self.shake > 0 {
            return Touch::Ignore;
        }
        match self.pickup {
            Pickup::Disallowed => Touch::Ignore,
            Pickup::Allowed => Touch::Give(self.pickup_item.clone()),
            Pickup::CreativeOnly if creative => Touch::Take,
            Pickup::CreativeOnly => Touch::Ignore,
        }
    }
    /// `Entity.isOnFire` (arrows are not fire immune).
    pub fn on_fire(&self) -> bool {
        self.fire_ticks > 0
    }
    /// `Entity.clearFire`.
    pub fn clear_fire(&mut self) {
        self.fire_ticks = self.fire_ticks.min(0);
    }
    /// `Entity.igniteForTicks` (an arrow never freezes).
    pub fn ignite_for_ticks(&mut self, ticks: i32) {
        if self.fire_ticks < ticks {
            self.fire_ticks = ticks;
        }
    }
    /// `Entity.igniteForSeconds`: Flame's `projectile_spawned` ignite.
    pub fn ignite_for_seconds(&mut self, seconds: f32) {
        self.ignite_for_ticks((seconds * 20.0).floor() as i32);
    }
    /// `BaseFireBlock.fireIgnite` for an entity that is not a player.
    fn fire_ignite(&mut self) {
        if self.fire_ticks < 0 {
            self.fire_ticks += 1;
        }
        if self.fire_ticks >= 0 {
            self.ignite_for_ticks(160);
        }
    }
}
/// The blocks, fluids and weather an arrow flies through.
pub struct ArrowScene<'a, 'r> {
    pub world: &'a World,
    pub registry: &'a horizon_world::registry::BlockRegistry,
    pub facts: &'a BlockFacts,
    pub collision: &'a BlockCollisionRegistry<'r>,
    pub fluids: &'a FluidResources,
    /// `Level.isRainingAt`.
    pub raining: &'a dyn Fn(BlockPos) -> bool,
    /// The `mob_griefing` and `tnt_explodes` game rules.
    pub mob_griefing: bool,
    pub tnt_explodes: bool,
}
impl ArrowScene<'_, '_> {
    fn block(&self, p: BlockPos) -> BlockStateId {
        self.world.block(p.x, p.y, p.z)
    }
    fn is_air(&self, state: BlockStateId) -> bool {
        self.facts.is_air(state)
    }
    /// `BlockState.getCollisionShape(level, pos)` (an empty collision context).
    fn collision_boxes(&self, state: BlockStateId, p: BlockPos) -> Vec<Aabb> {
        let mut scene = SceneCollisions {
            world: self.world,
            binding: self.collision,
            subject: None,
        };
        self.collision
            .collision_shape(&mut scene, state, [p.x, p.y, p.z])
            .map(|s| s.boxes().to_vec())
            .unwrap_or_default()
    }
    fn property(&self, state: BlockStateId, name: &str) -> Option<&str> {
        self.registry
            .state(state)?
            .properties
            .get(name)
            .map(String::as_str)
    }
    /// `ClipContext.Block.COLLIDER` with `Fluid.NONE`: the first block hit.
    fn clip(&self, from: Vec3, to: Vec3) -> Option<(Vec3, BlockPos, [i32; 3])> {
        let mut scene = SceneCollisions {
            world: self.world,
            binding: self.collision,
            subject: None,
        };
        let hit = horizon_core::geometry::traverse_blocks(from, to, 1024, |p: BlockPos| {
            let state = self.block(p);
            if self.is_air(state) {
                return None;
            }
            let shape = self
                .collision
                .collision_shape(&mut scene, state, [p.x, p.y, p.z])
                .ok()?;
            shape.clip(from, to, p)
        });
        let hit = hit.ok().flatten()?;
        Some((hit.location, hit.block, hit.direction.normal()))
    }
    /// `CollisionGetter.noCollision(null, box)` over blocks.
    fn no_block_collision(&self, b: Aabb) -> bool {
        let (lo, hi) = (b.min_position(), b.max_position());
        for x in (lo.x.floor() as i32 - 1)..=(hi.x.floor() as i32 + 1) {
            for y in (lo.y.floor() as i32 - 1)..=(hi.y.floor() as i32 + 1) {
                for z in (lo.z.floor() as i32 - 1)..=(hi.z.floor() as i32 + 1) {
                    let p = BlockPos::new(x, y, z);
                    let state = self.block(p);
                    if self.is_air(state) {
                        continue;
                    }
                    let origin = Vec3::new(f64::from(x), f64::from(y), f64::from(z));
                    if self
                        .collision_boxes(state, p)
                        .iter()
                        .any(|c| c.move_by(origin).intersects(b))
                    {
                        return false;
                    }
                }
            }
        }
        true
    }
}
fn bounds(at: Vec3) -> Aabb {
    // EntityDimensions.makeBoundingBox: half the float width.
    let w = f64::from(WIDTH / 2.0);
    Aabb::new(
        Vec3::new(at.x - w, at.y, at.z - w),
        Vec3::new(at.x + w, at.y + f64::from(HEIGHT), at.z + w),
    )
}
/// The bounding box of an arrow, or a thrown trident, at `at`.
pub fn arrow_bounds(at: Vec3) -> Aabb {
    bounds(at)
}
/// The blocks an arrow that moved from `from` to `to` was inside
/// (`Entity.checkInsideBlocks`: the sweep of its box, 16 steps at most), for
/// the blocks that react to it (`entityInside`).
pub fn blocks_inside(from: Vec3, to: Vec3) -> Vec<BlockPos> {
    let deflated = bounds(to).deflate(Vec3::new(
        f64::from(1.0e-5f32),
        f64::from(1.0e-5f32),
        f64::from(1.0e-5f32),
    ));
    let mut inside = Vec::new();
    let _ = sweep_intersected_blocks(from, to, deflated, 1_000_000, |p, iteration| {
        if iteration >= 16 {
            return false;
        }
        inside.push(p);
        true
    });
    inside
}
/// `Projectile.lerpRotation`.
fn lerp_rotation(mut old: f32, new: f32) -> f32 {
    while new - old < -180.0 {
        old -= 360.0;
    }
    while new - old >= 180.0 {
        old += 360.0;
    }
    old + 0.2 * (new - old)
}
/// `(float)(Mth.atan2(y, x) * 180.0F / (float)Math.PI)`: compiled as one
/// multiplication by the folded float constant.
fn degrees(y: f64, x: f64) -> f32 {
    horizon_core::geometry::trig::atan2_degrees(y, x)
}
fn signum(v: f64) -> f64 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        v
    }
}
/// One arrow tick: whether to keep it and what it hit. An arrow that hit an
/// entity stops there and is not kept: the game hands it to
/// `finish_entity_hit` once it has hurt the entity.
pub fn tick(p: &mut Projectile, scene: &ArrowScene, targets: &[(Who, Aabb)]) -> (bool, Hit) {
    let Some(mut st) = p.arrow.take() else {
        return (false, Hit::None);
    };
    // Entity.setOldPosAndRot: the renderer interpolates from here, in flight
    // and stuck alike; ServerLevel.tickNonPassenger counts the tick.
    p.previous = p.position;
    p.age += 1;
    let hit = Ticker {
        p,
        st: &mut st,
        scene,
    }
    .tick(targets);
    p.stuck = st.in_ground;
    let keep = !st.removed && !matches!(hit, Hit::Entity { .. });
    p.arrow = Some(st);
    (keep, hit)
}
/// What became of the entity an arrow hit (`AbstractArrow.onHitEntity`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityHit {
    /// `hurtOrSimulate` succeeded: the arrow is spent, with its hit sound.
    Hurt,
    /// It failed (a raised shield, invulnerability): the arrow turns back.
    Refused,
    /// The entity turned it first (`Entity.deflection`: breezes).
    Deflected,
}
/// The rest of the tick of an arrow that hit an entity: whether to keep it.
pub fn finish_entity_hit(p: &mut Projectile, scene: &ArrowScene, outcome: EntityHit) -> bool {
    let Some(mut st) = p.arrow.take() else {
        return false;
    };
    let trident = matches!(p.kind, ProjectileKind::Trident);
    let mut ticker = Ticker {
        p,
        st: &mut st,
        scene,
    };
    match outcome {
        EntityHit::Hurt | EntityHit::Refused if trident => ticker.trident_bounce(),
        EntityHit::Hurt => {
            ticker.hit_sound();
            // A piercing arrow flies on (`getPierceLevel() <= 0` discards).
            if ticker.st.pierce == 0 {
                ticker.st.removed = true;
            }
        }
        EntityHit::Refused => ticker.turn_back(),
        EntityHit::Deflected => {
            // The deflection comes before `onHitEntity` adds the entity to
            // `piercingIgnoreEntityIds`.
            if ticker.st.pierce > 0 {
                ticker.st.pierced.pop();
            }
        }
    }
    ticker.finish();
    p.stuck = st.in_ground;
    let keep = !st.removed;
    p.arrow = Some(st);
    keep
}
struct Ticker<'p, 's, 'a, 'r> {
    p: &'p mut Projectile,
    st: &'s mut ArrowState,
    scene: &'s ArrowScene<'a, 'r>,
}
impl Ticker<'_, '_, '_, '_> {
    fn position(&self) -> Vec3 {
        self.p.position
    }
    fn block_position(&self) -> BlockPos {
        BlockPos::containing(self.p.position)
    }
    /// `Entity.isInRain`: at the block position or the top of the box.
    fn in_rain(&self) -> bool {
        let p = self.block_position();
        (self.scene.raining)(p)
            || (self.scene.raining)(BlockPos::new(
                p.x,
                bounds(self.p.position).max_position().y.floor() as i32,
                p.z,
            ))
    }
    /// Whether this is a `ThrownTrident` (`AbstractArrow` with its own water
    /// inertia, sounds and hit).
    fn trident(&self) -> bool {
        matches!(self.p.kind, ProjectileKind::Trident)
    }
    fn tick(&mut self, targets: &[(Who, Aabb)]) -> Hit {
        // ThrownTrident.tick.
        if self.trident() && self.st.in_ground_time > 4 {
            self.st.dealt_damage = true;
        }
        let physics = !self.st.no_physics;
        let movement = self.p.velocity;
        let block_pos = self.block_position();
        let state = self.scene.block(block_pos);
        if !self.scene.is_air(state) && physics {
            let origin = Vec3::new(
                f64::from(block_pos.x),
                f64::from(block_pos.y),
                f64::from(block_pos.z),
            );
            let at = self.position();
            if self
                .scene
                .collision_boxes(state, block_pos)
                .iter()
                .any(|b| b.move_by(origin).contains(at))
            {
                self.p.velocity = Vec3::ZERO;
                self.st.in_ground = true;
            }
        }
        if self.st.shake > 0 {
            self.st.shake -= 1;
        }
        if self.st.was_touching_water || self.in_rain() {
            self.st.clear_fire();
        }
        if self.st.in_ground && physics {
            if self.st.last_state != Some(state) && self.should_fall() {
                self.start_falling();
            } else {
                // AbstractArrow.tickDespawn.
                self.st.life += 1;
                if self.st.life >= 1200 {
                    self.st.removed = true;
                }
            }
            self.st.in_ground_time += 1;
            if !self.st.removed {
                let at = self.position();
                self.apply_effects_from_blocks(at, at);
            }
            return Hit::None;
        }
        self.st.in_ground_time = 0;
        let original = self.position();
        if self.st.was_touching_water {
            // getWaterInertia (0.99 for a trident), then the bubbles behind it.
            let inertia = if self.trident() { 0.99f32 } else { 0.6f32 };
            self.p.velocity = self.p.velocity.scale(f64::from(inertia));
            let v = self.p.velocity;
            for _ in 0..4 {
                self.st.events.push(ArrowEvent::Particle {
                    kind: ArrowParticle::Bubble,
                    position: Vec3::new(
                        original.x - v.x * 0.25,
                        original.y - v.y * 0.25,
                        original.z - v.z * 0.25,
                    ),
                    velocity: v,
                });
            }
        }
        if self.p.critical() {
            for i in 0..4 {
                let f = f64::from(i) / 4.0;
                self.st.events.push(ArrowEvent::Particle {
                    kind: ArrowParticle::Crit,
                    position: Vec3::new(
                        original.x + movement.x * f,
                        original.y + movement.y * f,
                        original.z + movement.z * f,
                    ),
                    velocity: Vec3::new(-movement.x, -movement.y + 0.2, -movement.z),
                });
            }
        }
        let y_rot = if physics {
            degrees(movement.x, movement.z)
        } else {
            degrees(-movement.x, -movement.z)
        };
        let x_rot = degrees(movement.y, movement.horizontal_distance());
        self.p.pitch = lerp_rotation(self.p.pitch, x_rot);
        self.p.yaw = lerp_rotation(self.p.yaw, y_rot);
        self.check_left_owner(targets);
        let mut hit = Hit::None;
        let end = Vec3::new(
            original.x + movement.x,
            original.y + movement.y,
            original.z + movement.z,
        );
        if physics {
            hit = self.step_move_and_hit(original, end, targets);
            if matches!(hit, Hit::Entity { .. }) {
                return hit;
            }
        } else {
            self.p.position = end;
            self.apply_effects_from_blocks(original, end);
        }
        self.finish();
        hit
    }
    /// The end of a tick in flight: drag and gravity, then `Projectile.tick`
    /// and `Entity.baseTick`'s fluid and fire phases.
    fn finish(&mut self) {
        let physics = !self.st.no_physics;
        if !self.st.was_touching_water {
            self.p.velocity = self.p.velocity.scale(f64::from(0.99f32));
        }
        if physics && !self.st.in_ground {
            // Entity.applyGravity: AbstractArrow.getDefaultGravity.
            self.add_velocity(-0.05);
        }
        // Projectile.tick, then Entity.baseTick's fluid and fire phases.
        if !self.st.has_been_shot {
            self.st.events.push(ArrowEvent::ShootEvent);
            self.st.has_been_shot = true;
        }
        self.st.was_eye_in_water = self.st.fluids.is_eye_in_fluid(&Tags, WATER);
        let mut fluids = std::mem::take(&mut self.st.fluids);
        let _ = entity_fluid_tick::update(&mut fluids, self, WATER, LAVA, 1_000_000);
        self.st.fluids = fluids;
        if self.st.fire_ticks > 0 {
            self.st.fire_ticks -= 1;
        }
        if !self.st.first_tick && self.st.fluids.is_in_fluid(&Tags, LAVA) {
            self.st.fall_distance *= 0.5;
        }
        if self.position().y < f64::from(crate::chunk::MIN_Y - 64) {
            self.st.removed = true;
        }
        self.st.first_tick = false;
    }
    /// `getDeltaMovement().add(0, y, 0)`: the zeros added to x and z also
    /// turn a negative zero positive.
    fn add_velocity(&mut self, y: f64) {
        let v = self.p.velocity;
        self.p.velocity = Vec3::new(v.x + 0.0, v.y + y, v.z + 0.0);
    }
    /// `Projectile.checkLeftOwner`: out of the owner's box once the arrow's
    /// box, swept by its motion and grown by 1, no longer touches it.
    fn check_left_owner(&mut self, targets: &[(Who, Aabb)]) {
        if self.p.left_owner {
            return;
        }
        let owner = targets
            .iter()
            .find(|(who, _)| Some(*who) == self.p.owner)
            .map(|(_, b)| *b);
        let swept = bounds(self.p.position)
            .expand_towards(self.p.velocity)
            .inflate(Vec3::new(1.0, 1.0, 1.0));
        self.p.left_owner = owner.is_none_or(|b| !swept.intersects(b));
    }
    /// `AbstractArrow.shouldFall`.
    fn should_fall(&self) -> bool {
        let at = self.position();
        self.st.in_ground
            && self
                .scene
                .no_block_collision(Aabb::new(at, at).inflate(Vec3::new(0.06, 0.06, 0.06)))
    }
    /// `AbstractArrow.startFalling`.
    fn start_falling(&mut self) {
        self.st.in_ground = false;
        let v = self.p.velocity;
        let x = f64::from(self.st.random.next_float() * 0.2);
        let y = f64::from(self.st.random.next_float() * 0.2);
        let z = f64::from(self.st.random.next_float() * 0.2);
        self.p.velocity = Vec3::new(v.x * x, v.y * y, v.z * z);
        self.st.life = 0;
    }
    /// `AbstractArrow.stepMoveAndHit`: the nearest entity on the way, else the
    /// block. A piercing arrow that hits an entity here goes on from it next
    /// tick (`finish_entity_hit`); vanilla's loop goes on within the same tick.
    fn step_move_and_hit(&mut self, from: Vec3, to: Vec3, targets: &[(Who, Aabb)]) -> Hit {
        let block = self.scene.clip(from, to);
        let end = block.map_or(to, |b| b.0);
        // findHitEntities: ProjectileUtil.computeMargin and the search box.
        let margin = f64::from(((self.p.age as i32 - 2) as f32 / 20.0).clamp(0.0, 0.3));
        let area = bounds(from)
            .expand_towards(self.p.velocity)
            .inflate(Vec3::new(1.0, 1.0, 1.0));
        let mut best: Option<(Who, Vec3, f64)> = None;
        // ThrownTrident.findHitEntity: none once it has dealt damage.
        let targets = if self.st.dealt_damage { &[] } else { targets };
        for (who, b) in targets {
            if Some(*who) == self.p.owner && !self.p.left_owner || !b.intersects(area) {
                continue;
            }
            // canHitEntity: not an entity a piercing arrow has gone through.
            if self.st.pierced.contains(who) {
                continue;
            }
            if let Some(at) = self.entity_hit(b, from, end, margin) {
                // Sorted by the distance to the entity's position (the
                // stable sort keeps the first of equals).
                let c = b.center();
                let d = from.distance_squared(Vec3::new(c.x, b.min_position().y, c.z));
                if best.is_none_or(|bb| d < bb.2) {
                    best = Some((*who, at, d));
                }
            }
        }
        let next = best.map_or(end, |b| b.1);
        self.p.position = next;
        self.apply_effects_from_blocks(from, next);
        if let Some((who, at, _)) = best {
            // AbstractArrow.onHitEntity: past pierce level + 1 entities the
            // arrow is discarded before it hurts the next one.
            if self.st.pierce > 0 {
                if self.st.pierced.len() > usize::from(self.st.pierce) {
                    self.st.removed = true;
                    return Hit::None;
                }
                self.st.pierced.push(who);
            }
            return Hit::Entity { who, at };
        }
        let Some((at, pos, face)) = block else {
            return Hit::None;
        };
        if self.st.removed {
            return Hit::None;
        }
        let state = self.scene.block(pos);
        if self.scene.is_air(state) {
            return Hit::None;
        }
        self.on_hit_block(pos, state);
        Hit::Block {
            pos: [pos.x, pos.y, pos.z],
            at,
            face,
        }
    }
    /// `ProjectileUtil.getManyEntityHitResult` for one entity box: where the
    /// path enters it, else where it enters the box grown by `margin`, if a
    /// line from there towards the centre (stopped by blocks) reaches the box.
    fn entity_hit(&self, b: &Aabb, from: Vec3, to: Vec3, margin: f64) -> Option<Vec3> {
        if let Some(at) = b.clip(from, to) {
            return Some(at);
        }
        if margin <= 0.0 {
            return None;
        }
        let outside = b
            .inflate(Vec3::new(margin, margin, margin))
            .clip(from, to)?;
        let centre = b.center();
        let towards = self.scene.clip(outside, centre).map_or(centre, |h| h.0);
        b.clip(outside, towards).map(|_| outside)
    }
    /// `playSound(soundEvent, 1, 1.2 / (random * 0.2 + 0.9))`.
    fn hit_sound(&mut self) {
        let pitch = 1.2 / (self.st.random.next_float() * 0.2 + 0.9);
        // CrossbowItem.createProjectile: its arrows hit with the crossbow's sound.
        let crossbow = self
            .st
            .fired_from_weapon
            .as_ref()
            .is_some_and(|s| &*s.item == "minecraft:crossbow");
        let sound = if self.trident() {
            ArrowSound::TridentHitGround
        } else if crossbow {
            ArrowSound::CrossbowHit
        } else {
            ArrowSound::HitGround
        };
        self.st.events.push(ArrowEvent::Sound {
            sound,
            volume: 1.0,
            pitch,
        });
    }
    /// `ThrownTrident.onHitEntity` after the hurt, whether it landed or not:
    /// the hit sound, then `deflect(REVERSE, ..., (0.02, 0.2, 0.02))`. The
    /// trident keeps flying (it has dealt its damage) and falls.
    fn trident_bounce(&mut self) {
        self.st.dealt_damage = true;
        self.st.events.push(ArrowEvent::Sound {
            sound: ArrowSound::TridentHit,
            volume: 1.0,
            pitch: 1.0,
        });
        // ProjectileDeflection.REVERSE.
        let rotation = 170.0 + self.st.random.next_float() * 20.0;
        let v = self.p.velocity;
        self.p.velocity = Vec3::new(v.x * (-0.02 * 0.5), v.y * (-0.2 * 0.5), v.z * (-0.02 * 0.5));
        self.p.yaw += rotation;
    }
    /// `onHitEntity` when the entity took no damage: `deflect(REVERSE, 0.2)`,
    /// a half turn give or take ten degrees at a tenth of the speed; one
    /// left almost still drops its pickup (ALLOWED) and is discarded.
    fn turn_back(&mut self) {
        let power = 0.2;
        let rotation = 170.0 + self.st.random.next_float() * 20.0;
        let v = self.p.velocity;
        let k = -power * 0.5;
        self.p.velocity = Vec3::new(v.x * k, v.y * k, v.z * k);
        self.p.yaw += rotation;
        if self.p.velocity.length_squared() < 1.0e-7 {
            if self.st.pickup == Pickup::Allowed {
                self.st.events.push(ArrowEvent::Drop);
            }
            self.st.removed = true;
        }
    }
    /// `AbstractArrow.onHitBlock`: back off the face by 0.05 along each axis
    /// it moved on, stop, and stick.
    fn on_hit_block(&mut self, pos: BlockPos, state: BlockStateId) {
        self.st.last_state = Some(state);
        // Projectile.onHitBlock: BlockState.onProjectileHit.
        if self.st.on_fire()
            && let Some(reaction) = self.block_reaction(state)
        {
            self.st.events.push(ArrowEvent::Block { pos, reaction });
        }
        let v = self.p.velocity;
        let offset = f64::from(0.05f32);
        self.p.position = Vec3::new(
            self.p.position.x - signum(v.x) * offset,
            self.p.position.y - signum(v.y) * offset,
            self.p.position.z - signum(v.z) * offset,
        );
        self.p.velocity = Vec3::ZERO;
        self.hit_sound();
        self.st.in_ground = true;
        self.st.shake = 7;
        self.p.set_critical(false);
        self.st.pierce = 0;
        self.st.pierced.clear();
    }
    /// `Projectile.mayInteract`: a mob's arrows need `mob_griefing`.
    fn may_interact(&self) -> bool {
        !matches!(self.p.owner, Some(Who::Mob(_))) || self.scene.mob_griefing
    }
    /// `onProjectileHit` of the blocks a burning projectile lights.
    fn block_reaction(&self, state: BlockStateId) -> Option<BlockReaction> {
        let name = self.scene.registry.state(state)?.name.as_str();
        let flag = |key: &str| self.scene.property(state, key) == Some("true");
        let lit = || {
            let mut definition = self.scene.registry.state(state)?.clone();
            definition.properties.insert("lit".into(), "true".into());
            self.scene.registry.id(&definition)
        };
        match name {
            "minecraft:tnt" if self.may_interact() && self.scene.tnt_explodes => {
                Some(BlockReaction::PrimeTnt)
            }
            "minecraft:campfire" | "minecraft:soul_campfire"
                if self.may_interact() && !flag("lit") && !flag("waterlogged") =>
            {
                lit().map(BlockReaction::Light)
            }
            // AbstractCandleBlock: candles also must be dry (CandleBlock.canBeLit).
            _ if name.ends_with("candle_cake") && !flag("lit") => lit().map(BlockReaction::Light),
            _ if name.ends_with("candle") && !flag("lit") && !flag("waterlogged") => {
                lit().map(BlockReaction::Light)
            }
            _ => None,
        }
    }
    /// `Entity.applyEffectsFromBlocks(from, to)` for an arrow: the blocks and
    /// fluids its box passes through (bubble columns push, water puts the fire
    /// out), then rain.
    fn apply_effects_from_blocks(&mut self, from: Vec3, to: Vec3) {
        // Entity.isAffectedByBlocks.
        if self.st.removed || self.st.no_physics {
            return;
        }
        let was_on_fire = self.st.on_fire();
        let previous_fire = self.st.fire_ticks;
        let mut visited = HashSet::new();
        let mut effects = Collector::default();
        let used = self.check_inside_blocks(from, to, &mut visited, &mut effects, 16);
        if 16 - used <= 0 {
            self.check_inside_blocks(to, to, &mut visited, &mut effects, 1);
        }
        // StepBasedCollector.applyAndClear.
        for effect in effects.finish() {
            if self.st.removed {
                break;
            }
            match effect {
                Effect::ClearFreeze => {}
                Effect::FireIgnite => self.st.fire_ignite(),
                Effect::LavaIgnite => self.st.ignite_for_ticks(300),
                Effect::Extinguish => self.st.clear_fire(),
            }
        }
        if self.in_rain() {
            self.st.clear_fire();
        }
        if was_on_fire && !self.st.on_fire() {
            // Entity.playEntityOnFireExtinguishedSound.
            let pitch = 1.6 + (self.st.random.next_float() - self.st.random.next_float()) * 0.4;
            self.st.events.push(ArrowEvent::Sound {
                sound: ArrowSound::Extinguish,
                volume: 0.7,
                pitch,
            });
        }
        let ignited = self.st.fire_ticks > previous_fire;
        if !self.st.on_fire() && !ignited {
            // -getFireImmuneTicks(): 0 for arrows.
            self.st.fire_ticks = 0;
        }
    }
    /// `Entity.checkInsideBlocks(from, to, ...)`: returns the iterations used.
    fn check_inside_blocks(
        &mut self,
        from: Vec3,
        to: Vec3,
        visited: &mut HashSet<BlockPos>,
        effects: &mut Collector,
        max_iterations: i32,
    ) -> i32 {
        let deflated = bounds(to).deflate(Vec3::new(
            f64::from(1.0e-5f32),
            f64::from(1.0e-5f32),
            f64::from(1.0e-5f32),
        ));
        let far = from.distance_squared(to) > 0.999_990_000_000_252_6f64 * 0.999_990_000_000_252_6;
        let mut hits = Vec::new();
        let mut iterations = 0;
        let alive = !self.st.removed;
        let _ = sweep_intersected_blocks(from, to, deflated, 1_000_000, |p, iteration| {
            if !alive || iteration >= max_iterations {
                return false;
            }
            iterations = iteration;
            hits.push((p, iteration));
            true
        });
        for (p, step) in hits {
            let state = self.scene.block(p);
            if self.scene.is_air(state) {
                continue;
            }
            // getEntityInsideCollisionShape: the whole block for these.
            let inside_block = true;
            let fluid = self.scene.fluids.fluid(state);
            let inside_fluid = self.collided_with_fluid(fluid, p, from, to);
            if !(inside_block || inside_fluid) || !visited.insert(p) {
                continue;
            }
            effects.advance(step);
            let name = &*self.scene.facts.get(state).name;
            if matches!(name, "minecraft:fire" | "minecraft:soul_fire") {
                // BaseFireBlock.entityInside (its inFire hurt does nothing to arrows).
                effects.apply(Effect::ClearFreeze);
                effects.apply(Effect::FireIgnite);
            }
            if name == "minecraft:bubble_column" {
                let precise = far
                    || deflated.intersects(Aabb::new(
                        Vec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z)),
                        Vec3::new(f64::from(p.x + 1), f64::from(p.y + 1), f64::from(p.z + 1)),
                    ));
                if precise {
                    self.bubble_column(state, p);
                }
            }
            if inside_fluid {
                effects.advance(step);
                match fluid.kind() {
                    // WaterFluid.entityInside.
                    FluidKind::Water | FluidKind::FlowingWater => effects.apply(Effect::Extinguish),
                    // LavaFluid.entityInside (lavaHurt does nothing to arrows).
                    FluidKind::Lava | FluidKind::FlowingLava => {
                        effects.apply(Effect::ClearFreeze);
                        effects.apply(Effect::LavaIgnite);
                    }
                    FluidKind::Empty => {}
                }
            }
        }
        iterations + 1
    }
    /// `Entity.collidedWithFluid`: the fluid's box swept from `from` to `to`.
    fn collided_with_fluid(
        &mut self,
        fluid: FluidState,
        p: BlockPos,
        from: Vec3,
        to: Vec3,
    ) -> bool {
        if fluid.kind() == FluidKind::Empty {
            return false;
        }
        let Ok(height) = fluid.height_at(self, [p.x, p.y, p.z]) else {
            return false;
        };
        let b = Aabb::new(
            Vec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z)),
            Vec3::new(
                f64::from(p.x) + 1.0,
                f64::from(p.y) + f64::from(height),
                f64::from(p.z) + 1.0,
            ),
        );
        bounds(from).collided_along_vector(to.subtract(from), &[b])
    }
    /// `BubbleColumnBlock.entityInside` with `Projectile`'s pushes.
    fn bubble_column(&mut self, state: BlockStateId, p: BlockPos) {
        let drag = self.scene.property(state, "drag") == Some("true");
        let above = self.scene.block(BlockPos::new(p.x, p.y + 1, p.z));
        // The state above's collision shape, queried at this position.
        let nothing_above = self.scene.collision_boxes(above, p).is_empty()
            && self.scene.fluids.fluid(above).kind() == FluidKind::Empty;
        if self.st.in_ground {
            return;
        }
        if nothing_above {
            self.add_velocity(if drag { -0.03 } else { 0.1 });
            self.st
                .events
                .push(ArrowEvent::BubbleColumnParticles { pos: p });
        } else {
            self.add_velocity(if drag { -0.03 } else { 0.06 });
            self.st.fall_distance = 0.0;
        }
    }
}
/// `InsideBlockEffectType`s an arrow meets, in their apply order (FREEZE
/// needs powder snow, which an arrow's collision context passes through).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Effect {
    ClearFreeze,
    FireIgnite,
    LavaIgnite,
    Extinguish,
}
const EFFECT_ORDER: [Effect; 4] = [
    Effect::ClearFreeze,
    Effect::FireIgnite,
    Effect::LavaIgnite,
    Effect::Extinguish,
];
/// `InsideBlockEffectApplier.StepBasedCollector`: each step's effects once,
/// in type order, steps in the order they were reached.
struct Collector {
    last_step: i32,
    step: [bool; 4],
    effects: Vec<Effect>,
}
impl Default for Collector {
    fn default() -> Self {
        Self {
            last_step: -1,
            step: [false; 4],
            effects: Vec::new(),
        }
    }
}
impl Collector {
    fn advance(&mut self, step: i32) {
        if self.last_step != step {
            self.last_step = step;
            self.flush();
        }
    }
    fn apply(&mut self, effect: Effect) {
        self.step[effect as usize] = true;
    }
    fn flush(&mut self) {
        for effect in EFFECT_ORDER {
            if std::mem::take(&mut self.step[effect as usize]) {
                self.effects.push(effect);
            }
        }
    }
    fn finish(mut self) -> Vec<Effect> {
        self.flush();
        self.effects
    }
}
struct Tags;
impl FluidMembership for Tags {
    fn fluid_in_tag(&self, kind: FluidKind, tag: FluidTagId) -> bool {
        if tag == WATER {
            matches!(kind, FluidKind::Water | FluidKind::FlowingWater)
        } else {
            matches!(kind, FluidKind::Lava | FluidKind::FlowingLava)
        }
    }
}
type Error = Box<dyn std::error::Error>;
impl FluidMembership for Ticker<'_, '_, '_, '_> {
    fn fluid_in_tag(&self, kind: FluidKind, tag: FluidTagId) -> bool {
        Tags.fluid_in_tag(kind, tag)
    }
}
impl CurrentTarget for Ticker<'_, '_, '_, '_> {
    type Error = Error;
    fn is_player_class(&self) -> bool {
        false
    }
    fn velocity(&mut self) -> Result<Vec3, Error> {
        Ok(self.p.velocity)
    }
    fn add_delta_movement(&mut self, delta: Vec3) -> Result<(), Error> {
        let v = self.p.velocity;
        let sum = Vec3::new(v.x + delta.x, v.y + delta.y, v.z + delta.z);
        if delta.is_finite() && sum.is_finite() {
            self.p.velocity = sum;
        }
        Ok(())
    }
}
impl FluidInteractionWorld for Ticker<'_, '_, '_, '_> {
    type Error = Error;
    type Chunk = ();
    fn interaction_box(&mut self) -> Result<Option<Aabb>, Error> {
        Ok(Some(
            bounds(self.p.position).deflate(Vec3::new(0.001, 0.001, 0.001)),
        ))
    }
    fn loaded_chunk(&mut self, x: i32, z: i32) -> Result<Option<()>, Error> {
        Ok(self.scene.world.chunks.contains_key(&(x, z)).then_some(()))
    }
    fn chunk_sections(&mut self, _: ()) -> Result<FluidSections<'_>, Error> {
        static SECTIONS: [bool; crate::chunk::SECTIONS] = [true; crate::chunk::SECTIONS];
        Ok(FluidSections {
            minimum_section_y: crate::chunk::MIN_Y >> 4,
            has_fluid: &SECTIONS,
        })
    }
    fn bounding_box_min_y(&self) -> f64 {
        self.p.position.y
    }
    fn block_x(&self) -> i32 {
        self.p.position.x.floor() as i32
    }
    fn eye_y(&mut self) -> Result<f64, Error> {
        Ok(self.p.position.y + f64::from(EYE_HEIGHT))
    }
    fn block_z(&self) -> i32 {
        self.p.position.z.floor() as i32
    }
    fn fluid_state(&mut self, p: BlockPos) -> Result<FluidState, Error> {
        Ok(self.scene.fluids.fluid(self.scene.block(p)))
    }
    fn fluid_height(&mut self, f: FluidState, p: BlockPos) -> Result<f32, Error> {
        f.height_at(self, [p.x, p.y, p.z])
    }
    fn camera_height(&mut self, f: FluidState, p: BlockPos) -> Result<f32, Error> {
        f.camera_height_at(self, [p.x, p.y, p.z])
    }
    fn flow(&mut self, f: FluidState, p: BlockPos) -> Result<Vec3, Error> {
        flow_at(self, f, p)
    }
}
impl EntityFluidTickWorld for Ticker<'_, '_, '_, '_> {
    /// `AbstractArrow.isPushedByFluid`: not when stuck.
    fn is_pushed_by_fluid(&mut self) -> Result<bool, Error> {
        Ok(!self.st.in_ground)
    }
    fn reset_fall_distance(&mut self) -> Result<(), Error> {
        self.st.fall_distance = 0.0;
        Ok(())
    }
    fn was_touching_water(&self) -> bool {
        self.st.was_touching_water
    }
    fn first_tick(&self) -> bool {
        self.st.first_tick
    }
    fn water_splash(&mut self) -> Result<(), Error> {
        water_splash::water_splash(self, 1_000_000).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn set_was_touching_water(&mut self, value: bool) {
        self.st.was_touching_water = value;
    }
    fn fast_lava(&mut self) -> Result<bool, Error> {
        Ok(false)
    }
}
impl FluidHeightWorld for Ticker<'_, '_, '_, '_> {
    type Error = Error;
    fn fluid(&mut self, p: [i32; 3]) -> Result<FluidState, Error> {
        Ok(self
            .scene
            .fluids
            .fluid(self.scene.block(BlockPos::new(p[0], p[1], p[2]))))
    }
    fn sturdy_down(&mut self, p: [i32; 3]) -> Result<bool, Error> {
        let state = self.scene.block(BlockPos::new(p[0], p[1], p[2]));
        Ok(self.scene.fluids.sturdy(state, Direction::Down))
    }
}
impl FluidFlowWorld for Ticker<'_, '_, '_, '_> {
    type Error = Error;
    type Block = BlockStateId;
    fn fluid(&mut self, p: BlockPos) -> Result<FluidState, Error> {
        Ok(self.scene.fluids.fluid(self.scene.block(p)))
    }
    fn block(&mut self, p: BlockPos) -> Result<BlockStateId, Error> {
        Ok(self.scene.block(p))
    }
    fn blocks_fluid_flow(&self, s: BlockStateId) -> bool {
        self.scene.fluids.blocks_flow(s)
    }
    fn is_ice_class(&self, s: BlockStateId) -> bool {
        self.scene.fluids.ice(s)
    }
    fn is_face_sturdy(
        &mut self,
        s: BlockStateId,
        _: BlockPos,
        d: Direction,
    ) -> Result<bool, Error> {
        Ok(self.scene.fluids.sturdy(s, d))
    }
}
/// `Entity.doWaterSplashEffect` with the arrow's own random.
impl WaterSplashWorld for Ticker<'_, '_, '_, '_> {
    type Error = Error;
    type Entity = ();
    type Sound = ();
    fn this_entity(&self) {}
    fn controlling_passenger(&mut self) -> Result<Option<()>, Error> {
        Ok(None)
    }
    fn velocity(&mut self, _: ()) -> Result<Vec3, Error> {
        Ok(self.p.velocity)
    }
    fn splash_sound(&mut self, _: bool) -> Result<(), Error> {
        Ok(())
    }
    fn random_float(&mut self) -> Result<f32, Error> {
        Ok(self.st.random.next_float())
    }
    fn random_double(&mut self) -> Result<f64, Error> {
        Ok(self.st.random.next_double())
    }
    fn play_sound(&mut self, _: (), volume: f32, pitch: f32) -> Result<(), Error> {
        self.st.events.push(ArrowEvent::Sound {
            sound: ArrowSound::Splash,
            volume,
            pitch,
        });
        Ok(())
    }
    fn position(&self) -> Vec3 {
        self.p.position
    }
    fn width(&self) -> f32 {
        WIDTH
    }
    fn particle(
        &mut self,
        kind: SplashParticle,
        position: Vec3,
        velocity: Vec3,
    ) -> Result<(), Error> {
        self.st.events.push(ArrowEvent::Particle {
            kind: match kind {
                SplashParticle::Bubble => ArrowParticle::Bubble,
                SplashParticle::Splash => ArrowParticle::Splash,
            },
            position,
            velocity,
        });
        Ok(())
    }
    fn splash_event(&mut self) -> Result<(), Error> {
        self.st.events.push(ArrowEvent::SplashEvent);
        Ok(())
    }
}
