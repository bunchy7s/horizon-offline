//! Non-player entities simulated on the client world. Dropped items follow
//! `ItemEntity.tick`: gravity 0.04, 0.98 drag (block friction on the ground),
//! a ground bounce of -0.5, slime landing restitution (0.8, including a moving
//! piston that is carrying slime), water buoyancy, pickup delay, 6000-tick despawn,
//! `HoneyBlock.entityInside` when falling against a honey block's side, and
//! `BubbleColumnBlock.entityInside` (lift / drag, including the surface jump).
use crate::{
    blocks::{BlockFacts, Fluid},
    collisions::SceneCollisions,
    items::ItemStack,
    world::World,
};
use horizon_core::{
    geometry::{Aabb, Direction, Vec3, VoxelShape},
    position::BlockPos,
};
use horizon_world::{
    block_collision::BlockCollisionRegistry,
    level::portal_travel::{BorderBounds, find_free_position},
};

/// Gravity applied each tick when the item is not in a fluid (`ItemEntity.tick`).
const ITEM_GRAVITY: f64 = 0.04;
/// Non-living slime `bounceRestitution` (`1.0 * 0.8`) from
/// `Entity.restituteMovementAfterCollisions`.
const ITEM_SLIME_BOUNCE: f64 = 0.8;

/// After a vertical collision: slime bounces a dropped item by 0.8, matching
/// non-living `Entity.restituteMovementAfterCollisions`. Anything else zeros Y.
fn item_vertical_collision_velocity(wanted_y: f64, slime: bool) -> f64 {
    if slime && wanted_y < 0.0 && -wanted_y > ITEM_GRAVITY {
        -wanted_y * ITEM_SLIME_BOUNCE
    } else {
        0.0
    }
}

fn slime_support_name(block: &str, moving_moved: Option<&str>) -> bool {
    block == "minecraft:slime_block" || moving_moved == Some("minecraft:slime_block")
}

/// The block under an item, including `Entity.getOnPos` (~1e-5) as well as the
/// friction sample (`y - 0.999999`). A piston-moved slime is `moving_piston`
/// with `movedState = slime`, which has the bounce, not the moving piston.
fn item_support_is_slime(world: &World, facts: &BlockFacts, position: Vec3) -> bool {
    let x = position.x.floor() as i32;
    let z = position.z.floor() as i32;
    let friction_y = (position.y - 0.999_999).floor() as i32;
    let on_y = (position.y - f64::from(1.0e-5_f32)).floor() as i32;
    slime_at(world, facts, x, friction_y, z)
        || (on_y != friction_y && slime_at(world, facts, x, on_y, z))
}

fn slime_at(world: &World, facts: &BlockFacts, x: i32, y: i32, z: i32) -> bool {
    let block = &*facts.get(world.block(x, y, z)).name;
    let moving_moved = world
        .moving_pistons
        .iter()
        .find(|(pos, _)| *pos == [x, y, z])
        .map(|(_, piston)| &*facts.get(piston.moved_state).name);
    slime_support_name(block, moving_moved)
}

mod fluids;
pub use fluids::update_entity_fluids;

#[derive(Clone, Debug)]
pub struct ItemEntity {
    pub health: i32,
    pub fire_ticks: i32,
    pub id: u32,
    pub stack: ItemStack,
    pub thrower: Option<[i32; 4]>,
    /// `ItemEntity.target`: the only player who may pick the item up.
    pub target: Option<[i32; 4]>,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub on_ground: bool,
    pub age: u32,
    pub pickup_delay: u32,
    pub bob_offset: f32,
    /// `Entity.tickCount`: ticks since the item entered the level, which
    /// paces `ItemEntity.mergeWithNeighbours`.
    pub ticks: u32,
    /// `Entity.needsSync` / `syncPosition`: immediate `ServerEntity.sendChanges`
    /// after a bounce or velocity jump (`> 0.1` in `ItemEntity.tick`).
    pub needs_sync: bool,
    /// `Entity.stuckSpeedMultiplier`: set after a move by the cobweb or powder snow
    /// the item is in, scales (and stops) the next one.
    pub stuck: Vec3,
    /// `Entity.noGravity` (`NoGravity` in its save): `applyGravity` does nothing.
    pub no_gravity: bool,
}
/// Environment results needed by the entity owner, including lethal lava damage.
#[derive(Debug)]
pub struct ItemEnvironmentTick {
    pub alive: bool,
    pub lava_hurt: bool,
}
impl ItemEnvironmentTick {
    pub fn should_play_lava_hurt_sound(&self, tick: u32) -> bool {
        self.lava_hurt && (!self.alive || tick.is_multiple_of(10))
    }
}
impl ItemEntity {
    pub const WIDTH: f64 = 0.25;
    pub fn bounds(&self) -> Aabb {
        let h = Self::WIDTH / 2.0;
        Aabb::new(
            Vec3::new(self.position.x - h, self.position.y, self.position.z - h),
            Vec3::new(
                self.position.x + h,
                self.position.y + Self::WIDTH,
                self.position.z + h,
            ),
        )
    }
    pub fn environment_tick(
        &mut self,
        world: &World,
        facts: &BlockFacts,
        registry: &horizon_world::registry::BlockRegistry,
        items: &crate::items::Items,
    ) -> ItemEnvironmentTick {
        let at = world.block(
            self.position.x.floor() as i32,
            (self.position.y + 0.1).floor() as i32,
            self.position.z.floor() as i32,
        );
        let fluid = facts.get(at).fluid;
        let fire_immune = items.resists_fire(&self.stack, "minecraft:in_fire");
        if fluid == Fluid::Water || fire_immune {
            self.fire_ticks = 0;
        }
        if self.fire_ticks > 0 {
            if self.fire_ticks % 20 == 0
                && fluid != Fluid::Lava
                && !items.resists_fire(&self.stack, "minecraft:on_fire")
            {
                self.health -= 1;
            }
            self.fire_ticks -= 1;
        }
        let mut lava_hurt = false;
        if fluid == Fluid::Lava && !fire_immune {
            self.fire_ticks = self.fire_ticks.max(300);
            if !items.resists_fire(&self.stack, "minecraft:lava") {
                self.health -= 4;
                lava_hurt = true;
            }
        }
        let fire_damage = crate::motion::environment::fire_contact(world, registry, self.bounds());
        if fire_damage > 0.0 && !fire_immune {
            self.fire_ticks = self.fire_ticks.max(160);
            if !items.resists_fire(&self.stack, "minecraft:in_fire") {
                self.health -= fire_damage as i32;
            }
        }
        // CactusBlock.entityInside: `entity.hurt(cactus, 1.0)`. An item has no
        // hurt cooldown, so it loses 1 health every tick it touches a cactus
        // (5 health: gone in 5 ticks). Fire resistance does not help;
        // `ItemStack.canBeHurtBy` only skips it for a damage_resistant stack.
        if crate::motion::environment::cactus_contact(world, registry, self.bounds())
            && !items.resists_cactus(&self.stack)
        {
            self.health -= 1;
        }
        ItemEnvironmentTick {
            alive: self.health > 0,
            lava_hurt,
        }
    }
    /// `ItemEntity.tick` on a server: an item whose box is inside a block
    /// (one placed where it lay) has no physics this tick
    /// (`level.noCollision(box.deflate(1.0E-7))`).
    pub fn inside_block(&self, world: &World, collision: &BlockCollisionRegistry<'_>) -> bool {
        let mut scene = SceneCollisions {
            world,
            binding: collision,
            subject: None,
        };
        let inner = self.bounds().deflate(Vec3::new(1.0e-7, 1.0e-7, 1.0e-7));
        !crate::collisions::no_collision_empty(&mut scene, inner)
    }
    /// `Entity.moveTowardsClosestSpace` from the middle of the item's box:
    /// towards the nearest side of its block (north, south, west, east or
    /// up) without a full collision cube, at `speed` (`nextFloat() * 0.2 +
    /// 0.1`).
    pub fn move_towards_closest_space(&mut self, world: &World, facts: &BlockFacts, speed: f32) {
        self.velocity = towards_closest_space(
            self.position,
            self.bounds(),
            self.velocity,
            world,
            facts,
            speed,
        );
    }
    /// Returns false when the item should be removed. `no_physics`: the item
    /// moves through blocks this tick ([`ItemEntity::inside_block`]).
    pub fn tick(
        &mut self,
        world: &World,
        registry: &horizon_world::registry::BlockRegistry,
        facts: &BlockFacts,
        collision: &BlockCollisionRegistry<'_>,
        no_physics: bool,
        fluids: &crate::motion::FluidResources,
        entity_boxes: &[Aabb],
    ) -> bool {
        self.ticks += 1;
        self.previous = self.position;
        if self.pickup_delay > 0 && self.pickup_delay != u32::MAX {
            self.pickup_delay -= 1;
        }
        let (water_height, lava_height) = self.update_fluids(world, fluids);
        let old_velocity = self.velocity;
        let in_water = water_height > f64::from(0.1f32);
        if in_water {
            // ItemEntity.setUnderwaterMovement
            self.velocity = Vec3::new(
                self.velocity.x * 0.99,
                self.velocity.y + if self.velocity.y < 0.06 { 5.0e-4 } else { 0.0 },
                self.velocity.z * 0.99,
            );
        } else if lava_height > f64::from(0.1f32) {
            self.velocity = Vec3::new(
                self.velocity.x * 0.95,
                self.velocity.y + if self.velocity.y < 0.06 { 5.0e-4 } else { 0.0 },
                self.velocity.z * 0.95,
            );
        } else {
            if !self.no_gravity {
                self.velocity.y -= 0.04;
            }
        }
        let horizontal = self.velocity.x * self.velocity.x + self.velocity.z * self.velocity.z;
        // ItemEntity.tick: skip move 3 of 4 ticks while grounded with no
        // horizontal speed. The idle branch still runs bubble-column push
        // (`applyEffectsFromBlocksForLastMovements`), so lift builds up and
        // the item moves on the next wake-up (#4558).
        if !self.on_ground || horizontal > 1.0e-5 || (self.age + self.id).is_multiple_of(4) {
            let mut scene = SceneCollisions {
                world,
                binding: collision,
                subject: None,
            };
            let mut wanted = self.velocity;
            // Entity.move: a block the item sits in (cobweb, powder snow)
            // scales this move once and stops it.
            if self.stuck.length_squared() > 1.0e-7 {
                wanted = wanted.multiply(self.stuck);
                self.stuck = Vec3::ZERO;
                self.velocity = Vec3::ZERO;
            }
            // Entity.move with noPhysics: straight through, the ground as it was.
            let moved = if no_physics {
                wanted
            } else if entity_boxes.is_empty() {
                scene
                    .resolve(self.bounds(), wanted, 0.0, self.on_ground)
                    .unwrap_or(Vec3::ZERO)
            } else {
                // `Entity.collide`: the entities that `canBeCollidedWith` the item stop it
                // too (a boat, a minecart, a shulker, a happy ghast held still).
                crate::collisions::EntityCollisions {
                    blocks: scene,
                    entities: entity_boxes,
                    border: None,
                }
                .resolve(self.bounds(), wanted, 0.0, self.on_ground)
                .unwrap_or(Vec3::ZERO)
            };
            self.position = Vec3::new(
                self.position.x + moved.x,
                self.position.y + moved.y,
                self.position.z + moved.z,
            );
            if !no_physics {
                self.on_ground = wanted.y < 0.0 && moved.y != wanted.y;
            }
            if moved.x != wanted.x {
                self.velocity.x = 0.0;
            }
            if moved.z != wanted.z {
                self.velocity.z = 0.0;
            }
            if moved.y != wanted.y {
                // Entity.move restitutes instead of zeroing Y. A slime block
                // (or a moving piston carrying one) bounces a dropped item;
                // MovingPistonBlock itself has no bounceRestitution.
                let slime = item_support_is_slime(world, facts, self.position);
                self.velocity.y = item_vertical_collision_velocity(wanted.y, slime);
            }
            // Entity.move -> applyEffectsFromBlocks: honey then bubble columns
            // (before air drag), matching ItemEntity.tick.
            self.apply_honey_slide(world, facts);
            self.stuck = self.stuck_multiplier(world, facts).unwrap_or(Vec3::ZERO);
            crate::motion::bubble_column_push(
                world,
                registry,
                facts,
                collision,
                self.bounds(),
                &mut self.velocity,
            );
            let mut friction = 0.98;
            if self.on_ground {
                let below = world.block(
                    self.position.x.floor() as i32,
                    (self.position.y - 0.999_999).floor() as i32,
                    self.position.z.floor() as i32,
                );
                friction = f64::from(facts.get(below).friction) * 0.98;
            }
            self.velocity = Vec3::new(
                self.velocity.x * friction,
                self.velocity.y * 0.98,
                self.velocity.z * friction,
            );
            if self.on_ground && self.velocity.y < 0.0 {
                self.velocity.y *= -0.5;
            }
        } else {
            // applyEffectsFromBlocksForLastMovements: still lift/drag while idle.
            crate::motion::bubble_column_push(
                world,
                registry,
                facts,
                collision,
                self.bounds(),
                &mut self.velocity,
            );
        }
        if !no_physics {
            // Entity.applyEffectsFromBlocks: stepOn, then checkInsideBlocks.
            if self.on_ground {
                let p = BlockPos::containing(self.position.add_vector(Vec3::new(
                    0.0,
                    -f64::from(0.2_f32),
                    0.0,
                )));
                let state = world.block(p.x, p.y, p.z);
                crate::block_contact::emit(crate::block_contact::Event::StepOn(
                    crate::block_contact::Contact {
                        entity: self.id as i32,
                        kind: "minecraft:item",
                        block: &facts.get(state).name,
                        state,
                        position: p,
                    },
                ));
            }
            crate::block_contact::swept_inside(
                self.id as i32,
                "minecraft:item",
                self.previous,
                self.position,
                self.bounds(),
                |p| {
                    let state = world.block(p.x, p.y, p.z);
                    (state, &facts.get(state).name)
                },
            );
        }
        if self.velocity.subtract(old_velocity).length_squared() > 0.01 {
            self.needs_sync = true;
        }
        self.age += 1;
        // ItemEntity.tick refreshes immersion after moving.
        self.update_fluids(world, fluids);
        self.age < 6000 && self.position.y > -128.0
    }
    /// `Entity.makeStuckInBlock` as `WebBlock.entityInside` and
    /// `PowderSnowBlock.entityInside` call it for an item (which, unlike a
    /// living entity, is stuck whatever block its feet are in): the speed
    /// factor of the last such block the item's box (deflated by `1.0E-5`) is
    /// in.
    fn stuck_multiplier(&self, world: &World, facts: &BlockFacts) -> Option<Vec3> {
        let b = self.bounds().deflate(Vec3::new(1e-5, 1e-5, 1e-5));
        let (lo, hi) = (b.min_position(), b.max_position());
        let mut stuck = None;
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                    match &*facts.get(world.block(x, y, z)).name {
                        "minecraft:cobweb" => {
                            stuck = Some(Vec3::new(0.25, f64::from(0.05_f32), 0.25));
                        }
                        "minecraft:powder_snow" => {
                            stuck = Some(Vec3::new(f64::from(0.9_f32), 1.5, f64::from(0.9_f32)));
                        }
                        _ => {}
                    }
                }
            }
        }
        stuck
    }
    /// `PistonMovingBlockEntity.causeBounce` for a dropped item. Vanilla then
    /// runs `Entity.move(MoverType.PISTON)`, which leaves the ground on an
    /// upward push; without that, `ItemEntity.tick` skips movement 3 of 4 ticks
    /// while `onGround` with no horizontal speed and gravity eats the bounce.
    pub fn piston_slime_bounce(&mut self, movement: Direction) {
        self.velocity = horizon_world::piston_push::bounce_velocity(movement, self.velocity);
        self.on_ground = false;
        self.needs_sync = true;
    }
    /// `HoneyBlock.entityInside` for the honey blocks this item's box is in
    /// (`Entity.checkInsideBlocks`, deflated by `1.0E-5`). Slide sounds and
    /// entity event 53 are skipped (`doesEntityDoHoneyBlockSlideEffects` is
    /// false for an item).
    fn apply_honey_slide(&mut self, world: &World, facts: &BlockFacts) {
        let b = self.bounds().deflate(Vec3::new(1e-5, 1e-5, 1e-5));
        let (lo, hi) = (b.min_position(), b.max_position());
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                    if &*facts.get(world.block(x, y, z)).name == "minecraft:honey_block" {
                        self.honey_block_inside([x, y, z]);
                    }
                }
            }
        }
    }
    /// `HoneyBlock.isSlidingDown` then `doSlideMovement`. `getOldDeltaY` /
    /// `getNewDeltaY` wrap the stored velocity the same way they do for every
    /// entity (`vy / 0.98F + 0.08`).
    pub(crate) fn honey_block_inside(&mut self, pos: [i32; 3]) {
        let old_delta_y = |dy: f64| dy / f64::from(0.98_f32) + 0.08;
        let new_delta_y = |dy: f64| (dy - 0.08) * f64::from(0.98_f32);
        let [x, y, z] = pos.map(f64::from);
        if self.on_ground
            || self.position.y > y + 0.9375 - 1.0e-7
            || old_delta_y(self.velocity.y) >= -0.08
        {
            return;
        }
        let dx = (x + 0.5 - self.position.x).abs();
        let dz = (z + 0.5 - self.position.z).abs();
        let overlap = 0.4375 + Self::WIDTH / 2.0;
        if dx + 1.0e-7 <= overlap && dz + 1.0e-7 <= overlap {
            return;
        }
        let v = self.velocity;
        let old = old_delta_y(v.y);
        self.velocity = if old < -0.13 {
            let factor = -0.05 / old;
            Vec3::new(v.x * factor, new_delta_y(-0.05), v.z * factor)
        } else {
            Vec3::new(v.x, new_delta_y(-0.05), v.z)
        };
    }
    /// `ItemEntity.isMergable` at `age`: a stack below its size whose pickup
    /// delay isn't infinite, before its lifetime ends.
    fn mergeable(&self, age: u32, max_stack: u32) -> bool {
        self.pickup_delay != u32::MAX && age < 6000 && self.stack.count < max_stack
    }
}
/// The boxes that stop a dropped item (boats, minecarts, shulkers, a held-still happy ghast),
/// binned by 8-block cell, so an item looks only at the ones near it: a farm with many drops and
/// many vehicles checks a handful of boxes per item, not every box in the level.
#[derive(Default)]
pub struct ObstacleGrid {
    cells: crate::fast_hash::FxHashMap<(i32, i32, i32), Vec<u32>>,
    boxes: Vec<Aabb>,
}
impl ObstacleGrid {
    const CELL: f64 = 8.0;
    pub fn new(boxes: Vec<Aabb>) -> Self {
        let mut cells: crate::fast_hash::FxHashMap<(i32, i32, i32), Vec<u32>> = Default::default();
        for (i, b) in boxes.iter().enumerate() {
            let (lo, hi) = Self::span(*b);
            for x in lo.0..=hi.0 {
                for y in lo.1..=hi.1 {
                    for z in lo.2..=hi.2 {
                        cells.entry((x, y, z)).or_default().push(i as u32);
                    }
                }
            }
        }
        Self { cells, boxes }
    }
    fn span(b: Aabb) -> ((i32, i32, i32), (i32, i32, i32)) {
        let cell = |v: Vec3| {
            (
                (v.x / Self::CELL).floor() as i32,
                (v.y / Self::CELL).floor() as i32,
                (v.z / Self::CELL).floor() as i32,
            )
        };
        (cell(b.min_position()), cell(b.max_position()))
    }
    pub fn is_empty(&self) -> bool {
        self.boxes.is_empty()
    }
    /// The boxes meeting `area` (`getEntityCollisions`'s query box), in the order they were given.
    pub fn near(&self, area: Aabb, out: &mut Vec<Aabb>) {
        out.clear();
        if self.boxes.is_empty() {
            return;
        }
        let (lo, hi) = Self::span(area);
        let mut found: Vec<u32> = Vec::new();
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                for z in lo.2..=hi.2 {
                    if let Some(list) = self.cells.get(&(x, y, z)) {
                        found.extend(list);
                    }
                }
            }
        }
        found.sort_unstable();
        found.dedup();
        out.extend(
            found
                .into_iter()
                .map(|i| self.boxes[i as usize])
                .filter(|b| b.intersects(area)),
        );
    }
}

/// Dropped items by block cell, for `ItemEntity.mergeWithNeighbours`'s
/// search around an item. Holds indexes into the tick's item list.
#[derive(Default)]
pub struct ItemCells {
    cells: crate::fast_hash::FxHashMap<(i32, i32, i32), Vec<u32>>,
}
impl ItemCells {
    pub fn new(items: &[ItemEntity]) -> Self {
        let mut cells = Self::default();
        for (i, item) in items.iter().enumerate() {
            cells.add(i, item.position);
        }
        cells
    }
    fn cell(p: Vec3) -> (i32, i32, i32) {
        (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)
    }
    fn add(&mut self, i: usize, at: Vec3) {
        self.cells.entry(Self::cell(at)).or_default().push(i as u32);
    }
    pub fn remove(&mut self, i: usize, at: Vec3) {
        if let Some(list) = self.cells.get_mut(&Self::cell(at)) {
            list.retain(|&j| j as usize != i);
        }
    }
    pub fn moved(&mut self, i: usize, from: Vec3, to: Vec3) {
        if Self::cell(from) != Self::cell(to) {
            self.remove(i, from);
            self.add(i, to);
        }
    }
    /// The items whose cells could hold a box meeting `area`, in the order
    /// they were added (vanilla lists a section's entities in the order they
    /// entered it).
    fn near(&self, area: Aabb, out: &mut Vec<u32>) {
        out.clear();
        // An item's box reaches 0.125 sideways and 0.25 up from its position.
        let (min, max) = (area.min_position(), area.max_position());
        let (lo, hi) = (
            Self::cell(Vec3::new(min.x - 0.125, min.y - 0.25, min.z - 0.125)),
            Self::cell(Vec3::new(max.x + 0.125, max.y, max.z + 0.125)),
        );
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                for z in lo.2..=hi.2 {
                    if let Some(list) = self.cells.get(&(x, y, z)) {
                        out.extend_from_slice(list);
                    }
                }
            }
        }
        out.sort_unstable();
    }
}
/// `ItemEntity.mergeWithNeighbours` for `items[i]`, which has just moved in
/// its tick (its `age` already counts this tick; vanilla's doesn't yet): each
/// mergeable item whose box meets this one's grown by 0.5 sideways takes this
/// stack or gives it its own, the smaller into the larger
/// (`ItemEntity.tryToMerge`), until this item is used up. An emptied item is
/// marked in `removed`.
pub fn merge_with_neighbours(
    items: &mut [ItemEntity],
    removed: &mut [bool],
    cells: &mut ItemCells,
    i: usize,
    max_stack: &dyn Fn(&ItemStack) -> u32,
) {
    let age = |items: &[ItemEntity], j: usize| {
        if j == i {
            items[j].age - 1
        } else {
            items[j].age
        }
    };
    if removed[i] || !items[i].mergeable(age(items, i), max_stack(&items[i].stack)) {
        return;
    }
    let area = items[i].bounds().inflate(Vec3::new(0.5, 0.0, 0.5));
    let mut near = Vec::new();
    cells.near(area, &mut near);
    for j in near {
        let j = j as usize;
        if j == i
            || removed[j]
            || !items[j].bounds().intersects(area)
            || !items[j].mergeable(age(items, j), max_stack(&items[j].stack))
        {
            continue;
        }
        // ItemEntity.areMergable: both fit one stack of the same item and components.
        let (this, other) = (&items[i].stack, &items[j].stack);
        if items[i].target != items[j].target
            || this.count + other.count > max_stack(other)
            || !this.same_item_same_components(other)
        {
            continue;
        }
        let (to, from) = if other.count < this.count {
            (i, j)
        } else {
            (j, i)
        };
        // ItemEntity.merge(toStack, fromStack, 64).
        let room = max_stack(&items[to].stack).min(64) - items[to].stack.count;
        let moved = room.min(items[from].stack.count);
        items[to].stack.count += moved;
        items[from].stack.count -= moved;
        let (delay, from_age, to_age) =
            (items[from].pickup_delay, age(items, from), age(items, to));
        let to_item = &mut items[to];
        to_item.pickup_delay = to_item.pickup_delay.max(delay);
        // This item's age goes up once its tick ends.
        to_item.age = to_age.min(from_age) + u32::from(to == i);
        if items[from].stack.count == 0 {
            removed[from] = true;
            cells.remove(from, items[from].position);
        }
        if removed[i] {
            return;
        }
    }
}
/// ExperienceOrb: gravity 0.03, homing towards the player within 8 blocks.
#[derive(Clone, Debug)]
pub struct ExperienceOrb {
    /// The entity id servers track it by.
    pub id: u32,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub value: u32,
    pub age: u32,
    pub on_ground: bool,
    /// `ExperienceOrb.count`: orbs merged into this one; each pickup gives
    /// `value` once and takes one off.
    pub count: u32,
    /// `Entity.tickCount`, which paces `ExperienceOrb.scanForMerges`.
    pub ticks: u32,
    /// `ExperienceOrb.health`: damage wears it down from 5 (`hurtServer`).
    pub health: i32,
    /// `Entity.noGravity`: `ExperienceOrb.tick`'s `applyGravity` does nothing.
    pub no_gravity: bool,
}
impl ExperienceOrb {
    /// `ExperienceOrb.hurtServer`: the damage comes off the health as an
    /// `int`; an orb at 0 or below is discarded. True when it is gone.
    pub fn hurt(&mut self, damage: f32) -> bool {
        self.health = (self.health as f32 - damage) as i32;
        self.health <= 0
    }
    /// The box of an orb whose `position` (the bottom of the box) is `at`.
    pub fn box_at(at: Vec3) -> Aabb {
        let h = 0.25;
        Aabb::new(
            Vec3::new(at.x - h, at.y, at.z - h),
            Vec3::new(at.x + h, at.y + 0.5, at.z + h),
        )
    }
    pub fn bounds(&self) -> Aabb {
        Self::box_at(self.position)
    }
    /// `Entity.unstuckIfPossible`, which `ExperienceOrb`'s constructor runs as
    /// `if (!level.noCollision(box)) unstuckIfPossible(box.getSize())`: the free
    /// spot nearest the box's centre, searched in a cube of the box's size
    /// around it. `None` when the box is already free (the orb then keeps its
    /// exact position) or when nothing free is that close. `shape(p)` is the
    /// block's collision shape at `p`, in world coordinates.
    pub fn unstuck(
        at: Vec3,
        border: &BorderBounds,
        mut shape: impl FnMut(BlockPos) -> Option<VoxelShape>,
    ) -> Option<Vec3> {
        let bounds = Self::box_at(at);
        let size = bounds.average_size();
        let lo = bounds.min_position();
        let cells = |v: f64| (v - 1.0e-7).floor() as i32 - 1..=(v + 1.0e-7).floor() as i32 + 1;
        // noCollision(box): a block shape reaching into it leaves the orb stuck.
        let mut stuck = false;
        'cells: for x in cells(lo.x) {
            for y in cells(lo.y) {
                for z in cells(lo.z) {
                    if shape(BlockPos::new(x, y, z))
                        .is_some_and(|s| s.boxes().iter().any(|b| b.intersects(bounds)))
                    {
                        stuck = true;
                        break 'cells;
                    }
                }
            }
        }
        if !stuck {
            return None;
        }
        // `AABB.ofSize(center, size, size, size)`: where the box's centre may
        // go, a cube of the box's mean size around its centre.
        let allowed = Aabb::of_size(bounds.center(), Vec3::new(size, size, size));
        find_free_position(allowed, bounds.center(), bounds.size(), border, shape)
            .map(|p| p.subtract(Vec3::new(0.0, bounds.size().y / 2.0, 0.0)))
    }
    /// `ExperienceOrb.canMerge(orb, id, value)` for an orb that is not removed:
    /// the same value and ids a multiple of 40 apart.
    pub fn can_merge(&self, id: u32, value: u32) -> bool {
        self.value == value && (self.id as i32).wrapping_sub(id as i32) % 40 == 0
    }
    /// ExperienceOrb.getExperienceValue: the largest orb size not above `value`.
    pub fn split(value: u32) -> u32 {
        [2477, 1237, 617, 307, 149, 73, 37, 17, 7, 3, 1]
            .into_iter()
            .find(|&v| value >= v)
            .unwrap_or(1)
    }
    /// ExperienceOrb.getIcon
    pub fn icon(&self) -> u32 {
        match self.value {
            2477.. => 10,
            1237.. => 9,
            617.. => 8,
            307.. => 7,
            149.. => 6,
            73.. => 5,
            37.. => 4,
            17.. => 3,
            7.. => 2,
            3.. => 1,
            _ => 0,
        }
    }
    pub fn tick(
        &mut self,
        world: &World,
        facts: &BlockFacts,
        collision: &BlockCollisionRegistry<'_>,
        player: Option<[f64; 3]>,
        server: bool,
        next_float: &mut dyn FnMut() -> f32,
    ) -> bool {
        self.ticks += 1;
        self.previous = self.position;
        let block = world.block(
            self.position.x.floor() as i32,
            self.position.y.floor() as i32,
            self.position.z.floor() as i32,
        );
        let bounds = self.bounds();
        let mut scene = SceneCollisions {
            world,
            binding: collision,
            subject: None,
        };
        // `boolean colliding = !level.noCollision(getBoundingBox())`: an orb
        // inside blocks takes no gravity until it is pushed out below.
        let colliding = !crate::collisions::no_collision_empty(&mut scene, bounds);
        if facts.get(block).fluid == Fluid::Water {
            self.velocity = Vec3::new(
                self.velocity.x * 0.99,
                (self.velocity.y + 5.0e-4).min(0.06),
                self.velocity.z * 0.99,
            );
        } else if !colliding && !self.no_gravity {
            self.velocity.y -= 0.03;
        }
        if facts.get(block).fluid == Fluid::Lava {
            // ExperienceOrb.tick: lava throws the orb back out.
            self.velocity = Vec3::new(
                f64::from((next_float() - next_float()) * 0.2),
                f64::from(0.2f32),
                f64::from((next_float() - next_float()) * 0.2),
            );
        }
        if let Some(p) = player {
            let d = [
                (p[0] - self.position.x) / 8.0,
                (p[1] - self.position.y) / 8.0,
                (p[2] - self.position.z) / 8.0,
            ];
            let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            let power = 1.0 - length;
            if power > 0.0 && length > 1e-6 {
                let power = power * power;
                self.velocity = Vec3::new(
                    self.velocity.x + d[0] / length * power * 0.1,
                    self.velocity.y + d[1] / length * power * 0.1,
                    self.velocity.z + d[2] / length * power * 0.1,
                );
            }
        }
        if player.is_none() && server && colliding {
            // `followingPlayer == null && !level().isClientSide() && colliding`: an orb that would still
            // be stuck at its next position gets
            // `Entity.moveTowardsClosestSpace`, a nudge towards the nearest
            // free side of its block.
            if !crate::collisions::no_collision_empty(&mut scene, bounds.move_by(self.velocity)) {
                let speed = next_float() * 0.2 + 0.1;
                self.move_towards_closest_space(world, facts, speed);
            }
        }
        // `double fallSpeed = getDeltaMovement().y`, taken after the stuck
        // push and before the move.
        let wanted = self.velocity;
        let moved = scene
            .resolve(bounds, wanted, 0.0, self.on_ground)
            .unwrap_or(Vec3::ZERO);
        self.position = Vec3::new(
            self.position.x + moved.x,
            self.position.y + moved.y,
            self.position.z + moved.z,
        );
        self.on_ground = wanted.y < 0.0 && moved.y != wanted.y;
        // Entity.move's restituteMovementAfterCollisions, an orb's bounciness
        // 0: each collided axis's velocity is zeroed — x and z by Mth.equal's
        // 1.0E-5, y exactly.
        if (wanted.x - moved.x).abs() >= 1.0e-5 {
            self.velocity.x = 0.0;
        }
        if (wanted.z - moved.z).abs() >= 1.0e-5 {
            self.velocity.z = 0.0;
        }
        if moved.y != wanted.y {
            self.velocity.y = 0.0;
        }
        // `float friction = getAirDrag()` (0.98F), the ground's block
        // friction multiplied in as a float, widened once for the scale.
        let mut friction = 0.98_f32;
        if self.on_ground {
            let below = world.block(
                self.position.x.floor() as i32,
                (self.position.y - 0.999_999).floor() as i32,
                self.position.z.floor() as i32,
            );
            friction *= facts.get(below).friction;
        }
        let friction = f64::from(friction);
        // The drag scales every axis by the same friction, the ground's
        // block friction included.
        self.velocity = Vec3::new(
            self.velocity.x * friction,
            self.velocity.y * friction,
            self.velocity.z * friction,
        );
        // `if (verticalCollisionBelow && fallSpeed < -getGravity())`: the
        // landing bounce is 0.4 of the pre-move speed, not a fixed
        // restitution.
        if self.on_ground && wanted.y < -0.03 {
            self.velocity.y = -wanted.y * 0.4;
        }
        self.age += 1;
        self.age < 6000
    }
    /// `Entity.moveTowardsClosestSpace` from the middle of the orb's box:
    /// towards the nearest side of its block (north, south, west, east or
    /// up) without a full collision cube, at `speed` (the orb's own
    /// random, `nextFloat() * 0.2 + 0.1`).
    fn move_towards_closest_space(&mut self, world: &World, facts: &BlockFacts, speed: f32) {
        self.velocity = towards_closest_space(
            self.position,
            self.bounds(),
            self.velocity,
            world,
            facts,
            speed,
        );
    }
}
/// `Entity.moveTowardsClosestSpace` from the middle of an entity's box
/// (`bounds`, its feet at `position`): the velocity pushed towards the
/// nearest side of its block (north, south, west, east or up) without a full
/// collision cube, at `speed` (`nextFloat() * 0.2 + 0.1`). Items and
/// experience orbs share it.
fn towards_closest_space(
    position: Vec3,
    bounds: Aabb,
    velocity: Vec3,
    world: &World,
    facts: &BlockFacts,
    speed: f32,
) -> Vec3 {
    let at = Vec3::new(
        position.x,
        (bounds.min_position().y + bounds.max_position().y) / 2.0,
        position.z,
    );
    let pos = [
        at.x.floor() as i32,
        at.y.floor() as i32,
        at.z.floor() as i32,
    ];
    let delta = [
        at.x - f64::from(pos[0]),
        at.y - f64::from(pos[1]),
        at.z - f64::from(pos[2]),
    ];
    // (axis, step): NORTH, SOUTH, WEST, EAST, UP.
    let mut closest = (1, 1);
    let mut nearest = f64::MAX;
    for (axis, step) in [(2, -1), (2, 1), (0, -1), (0, 1), (1, 1)] {
        let mut n = pos;
        n[axis] += step;
        if !facts.get(world.block(n[0], n[1], n[2])).full_collision {
            let d = if step > 0 {
                1.0 - delta[axis]
            } else {
                delta[axis]
            };
            if d < nearest {
                nearest = d;
                closest = (axis, step);
            }
        }
    }
    let push = f64::from(closest.1 as f32 * speed);
    let v = velocity.scale(0.75);
    match closest.0 {
        0 => Vec3::new(push, v.y, v.z),
        1 => Vec3::new(v.x, push, v.z),
        _ => Vec3::new(v.x, v.y, push),
    }
}
/// Player experience (Player.giveExperiencePoints / getXpNeededForNextLevel).
#[derive(Clone, Copy, Debug, Default)]
pub struct Experience {
    pub level: i32,
    pub progress: f32,
    pub total: i32,
}
impl Experience {
    /// `Player.getBaseExperienceReward`: keep-inventory and spectators drop none.
    pub fn death_reward(&self, keep: bool) -> u32 {
        if keep {
            0
        } else {
            self.level.wrapping_mul(7).clamp(0, 100) as u32
        }
    }
    /// `ServerPlayer.restoreFrom`: only keep-inventory and spectators retain XP.
    pub fn respawn(&mut self, keep: bool) {
        if !keep {
            *self = Self::default();
        }
    }
    pub fn needed(level: i32) -> i32 {
        if level >= 30 {
            112 + (level - 30) * 9
        } else if level >= 15 {
            37 + (level - 15) * 5
        } else {
            7 + level * 2
        }
    }
    /// Returns the first multiple of 5 the player reached, where each level
    /// gained would play the level-up sound (`Player.giveExperienceLevels`).
    pub fn give(&mut self, points: i32) -> Option<i32> {
        let mut sound_level = None;
        self.progress += points as f32 / Self::needed(self.level) as f32;
        self.total = self.total.saturating_add(points).max(0);
        while self.progress >= 1.0 {
            self.progress = (self.progress - 1.0) * Self::needed(self.level) as f32;
            self.level += 1;
            if self.level % 5 == 0 {
                sound_level.get_or_insert(self.level);
            }
            self.progress /= Self::needed(self.level) as f32;
        }
        sound_level
    }
}
/// A painting (`Painting`, a `HangingEntity`): the block it hangs in, the
/// 2D data value of the direction it faces, and its variant.
#[derive(Clone, Debug)]
pub struct Painting {
    pub id: u32,
    pub pos: [i32; 3],
    /// Direction.get2DDataValue: south 0, west 1, north 2, east 3.
    pub facing: u8,
    pub variant: std::sync::Arc<str>,
    pub width: i32,
    pub height: i32,
    /// BlockAttachedEntity.checkInterval.
    pub check_interval: u32,
}
impl Painting {
    pub fn step(&self) -> [i32; 3] {
        [[0, 0, 1], [-1, 0, 0], [0, 0, -1], [1, 0, 0]][usize::from(self.facing & 3)]
    }
    /// Direction.getCounterClockWise (the painting's left as it faces out).
    pub fn left(&self) -> [i32; 3] {
        [[1, 0, 0], [0, 0, 1], [-1, 0, 0], [0, 0, -1]][usize::from(self.facing & 3)]
    }
    /// Painting.calculateBoundingBox: (min, max).
    pub fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        let s = self.step().map(f64::from);
        let l = self.left().map(f64::from);
        let off = |n: i32| if n % 2 == 0 { 0.5 } else { 0.0 };
        let (h, v) = (off(self.width), off(self.height));
        let c = [
            f64::from(self.pos[0]) + 0.5 - s[0] * 0.46875 + l[0] * h,
            f64::from(self.pos[1]) + 0.5 + v,
            f64::from(self.pos[2]) + 0.5 - s[2] * 0.46875 + l[2] * h,
        ];
        let along_x = s[0] != 0.0;
        let size = [
            if along_x {
                0.0625
            } else {
                f64::from(self.width)
            },
            f64::from(self.height),
            if along_x {
                f64::from(self.width)
            } else {
                0.0625
            },
        ];
        (
            std::array::from_fn(|i| c[i] - size[i] / 2.0),
            std::array::from_fn(|i| c[i] + size[i] / 2.0),
        )
    }
}
/// The `DyeColor` ordinal of a colour name (`DyeColor.byName`), white when
/// the name is unknown.
pub fn dye_index(name: &str) -> u8 {
    crate::mobs::render::DYE_NAMES
        .iter()
        .position(|n| *n == name)
        .unwrap_or(0) as u8
}
/// A cushion (`Cushion`, a `BlockAttachedEntity`): a 1 x 0.25 block pillow
/// that seats a player. It sits on the top face of a block, at the clicked
/// height, and is coloured by its `DyeColor`.
#[derive(Clone, Debug)]
pub struct Cushion {
    pub id: u32,
    /// `Entity.position`: the bottom centre of the cushion.
    pub position: Vec3,
    /// `BlockAttachedEntity.pos` (the saved `block_pos`).
    pub pos: [i32; 3],
    pub yaw: f32,
    /// `DyeColor` ordinal: white 0, orange 1, ..., black 15.
    pub color: u8,
    /// BlockAttachedEntity.ticksSinceLastCheck.
    pub check_interval: u32,
}
impl Cushion {
    /// `Entity.makeBoundingBox`: x and z half a block out, y to y + 0.25.
    pub fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        let p = self.position;
        (
            [p.x - 0.5, p.y, p.z - 0.5],
            [p.x + 0.5, p.y + 0.25, p.z + 0.5],
        )
    }
    /// The `DyeColor` name of the cushion.
    pub fn color_name(&self) -> &'static str {
        crate::mobs::render::DYE_NAMES[usize::from(self.color.min(15))]
    }
    /// `Cushion.getCushionItemStackWithData`: the matching coloured item.
    pub fn item(&self) -> String {
        format!("minecraft:{}_cushion", self.color_name())
    }
    /// `Cushion.interact`'s seat: the vehicle's PASSENGER attachment
    /// `(0, 0.25, 0)` less the player's `(0, 0.6, 0)`.
    pub fn seat(&self) -> Vec3 {
        Vec3::new(self.position.x, self.position.y - 0.35, self.position.z)
    }
    /// `Entity.getDismountLocationForPassenger`: on top of the cushion.
    pub fn dismount(&self) -> Vec3 {
        Vec3::new(self.position.x, self.position.y + 0.25, self.position.z)
    }
}
/// An item frame or glow item frame (`ItemFrame`).
#[derive(Clone, Debug)]
pub struct ItemFrame {
    pub id: u32,
    pub pos: [i32; 3],
    /// Direction.get3DDataValue: down 0, up 1, north 2, south 3, west 4, east 5.
    pub facing: u8,
    pub glow: bool,
    pub item: Option<crate::items::ItemStack>,
    /// 0..8, in 45 degree steps.
    pub rotation: u8,
    pub drop_chance: f32,
    pub fixed: bool,
    pub invisible: bool,
    pub check_interval: u32,
}
impl ItemFrame {
    pub fn step(&self) -> [i32; 3] {
        [
            [0, -1, 0],
            [0, 1, 0],
            [0, 0, -1],
            [0, 0, 1],
            [-1, 0, 0],
            [1, 0, 0],
        ][usize::from(self.facing.min(5))]
    }
    pub fn kind(&self) -> &'static str {
        if self.glow {
            "minecraft:glow_item_frame"
        } else {
            "minecraft:item_frame"
        }
    }
    /// ItemFrame.createBoundingBox (without a framed map): (min, max).
    pub fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        let s = self.step().map(f64::from);
        let c: [f64; 3] = std::array::from_fn(|i| f64::from(self.pos[i]) + 0.5 - s[i] * 0.46875);
        let size: [f64; 3] = std::array::from_fn(|i| if s[i] != 0.0 { 0.0625 } else { 0.75 });
        (
            std::array::from_fn(|i| c[i] - size[i] / 2.0),
            std::array::from_fn(|i| c[i] + size[i] / 2.0),
        )
    }
}
/// An armor stand (`ArmorStand`).
#[derive(Clone, Debug)]
pub struct ArmorStand {
    pub fire_ticks: i32,
    pub health: f32,
    pub id: u32,
    pub position: Vec3,
    /// `Entity.xo`, `yo`, `zo`: the position at the start of the tick, which
    /// the renderer blends toward `position` by the partial tick.
    pub previous: Vec3,
    pub velocity: Vec3,
    pub on_ground: bool,
    /// `yRot`, which the body follows (`ArmorStand.tickHeadTurn`: `yBodyRot`).
    pub yaw: f32,
    /// `yBodyRotO`: the body's yaw at the start of the tick.
    pub previous_yaw: f32,
    /// mainhand, offhand, feet, legs, chest, head (EquipmentSlot order).
    pub equipment: [Option<crate::items::ItemStack>; 6],
    pub small: bool,
    pub show_arms: bool,
    pub no_base_plate: bool,
    pub invisible: bool,
    pub marker: bool,
    /// `CustomName`: the text of a hologram.
    pub custom_name: Option<crate::text::Text>,
    /// `CustomNameVisible`.
    pub name_visible: bool,
    /// Game time of the last hit (the wobble and the second-hit window).
    pub last_hit: i64,
    /// `DATA_HEAD_POSE` to `DATA_RIGHT_LEG_POSE`: head, body, left arm, right
    /// arm, left leg, right leg, in degrees (`POSE_KEYS` order).
    pub pose: [[f32; 3]; 6],
    /// `Entity.vehicle` on a server's client (`SetPassengers`): the mob it
    /// rides, which seats it every tick (`positionRider`), as NPC plugins hang
    /// a second name over a player's head.
    pub vehicle: Option<u32>,
    /// `ArmorStand.disabledSlots`: bit `filterFlag` locks a slot, bit
    /// `filterFlag + 8` takes from it and bit `filterFlag + 16` puts into it.
    pub disabled_slots: i32,
    /// Server-only `LivingEntity.lastPos` for location-based enchantments.
    pub enchantment_block: Option<BlockPos>,
    /// Travel accumulated for `LivingEntity.checkFallDamage`'s landing callback.
    pub enchantment_fall_distance: f64,
    /// Last equipment seen by `collectEquipmentChanges`; not saved to NBT.
    pub enchantment_equipment: [Option<ItemStack>; 6],
}
impl ArmorStand {
    pub const SLOTS: [&'static str; 6] = ["mainhand", "offhand", "feet", "legs", "chest", "head"];
    /// The `Pose` compound's keys.
    pub const POSE_KEYS: [&'static str; 6] =
        ["Head", "Body", "LeftArm", "RightArm", "LeftLeg", "RightLeg"];
    /// `ArmorStand.DEFAULT_*_POSE`.
    pub const DEFAULT_POSE: [[f32; 3]; 6] = [
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0],
        [-10.0, 0.0, -10.0],
        [-15.0, 0.0, 10.0],
        [-1.0, 0.0, -1.0],
        [1.0, 0.0, 1.0],
    ];
    /// Where the renderer draws the stand `alpha` of the way through the tick
    /// (`EntityRenderer`'s `Mth.lerp(partialTick, xo, x)` and so on).
    pub fn render_position(&self, alpha: f64) -> Vec3 {
        self.previous.lerp(self.position, alpha)
    }
    /// The body's yaw `alpha` of the way through the tick
    /// (`LivingEntityRenderer.solveBodyRot`: `Mth.rotLerp(partialTick,
    /// yBodyRotO, yBodyRot)`).
    pub fn render_yaw(&self, alpha: f32) -> f32 {
        self.previous_yaw + alpha * crate::camera::wrap_degrees(self.yaw - self.previous_yaw)
    }
    /// A server's stand on its client, each tick:
    /// `setOldPosAndRot` (`ClientLevel.tickNonPassenger`), then the step of
    /// its `SteppedInterpolationHandler` (`step`: the position and yaw it is
    /// put at, when one is due). The yaw
    /// turns the short way and stays continuous, so a frame between ticks
    /// never spins it.
    pub fn interpolate(&mut self, step: Option<(Vec3, f32)>) {
        self.previous = self.position;
        self.previous_yaw = self.yaw;
        if let Some((at, yaw)) = step {
            self.position = at;
            self.yaw += crate::camera::wrap_degrees(yaw - self.yaw);
        }
    }
    /// Put there at once, with nothing to blend from
    /// (`Entity.snapTo`).
    pub fn snap(&mut self, at: Vec3, yaw: f32) {
        self.position = at;
        self.previous = at;
        self.yaw = yaw;
        self.previous_yaw = yaw;
    }
    pub fn new(id: u32, position: Vec3, yaw: f32) -> Self {
        Self {
            fire_ticks: 0,
            health: 20.0,
            id,
            position,
            previous: position,
            velocity: Vec3::ZERO,
            on_ground: true,
            yaw,
            previous_yaw: yaw,
            equipment: Default::default(),
            small: false,
            show_arms: false,
            no_base_plate: false,
            invisible: false,
            marker: false,
            custom_name: None,
            name_visible: false,
            last_hit: i64::MIN / 2,
            pose: Self::DEFAULT_POSE,
            vehicle: None,
            disabled_slots: 0,
            enchantment_block: None,
            enchantment_fall_distance: 0.0,
            enchantment_equipment: Default::default(),
        }
    }
    /// `EquipmentSlot.getFilterFlag` of an `equipment` index (main hand, off
    /// hand, feet, legs, chest, head).
    fn filter_flag(slot: usize) -> u32 {
        [0, 5, 1, 2, 3, 4][slot]
    }
    /// `ArmorStand.isDisabled`: the slot is locked, or it is a hand and the
    /// stand has no arms.
    pub fn is_disabled(&self, slot: usize) -> bool {
        self.disabled_slots & (1 << Self::filter_flag(slot)) != 0 || (slot < 2 && !self.show_arms)
    }
    /// The `swapItem` locks: taking from a filled slot or putting into an
    /// empty one can each be disabled.
    pub fn swap_locked(&self, slot: usize) -> bool {
        let offset = if self.equipment[slot].is_some() {
            8
        } else {
            16
        };
        self.disabled_slots & (1 << (Self::filter_flag(slot) + offset)) != 0
    }
    pub fn environment_tick(
        &mut self,
        world: &World,
        facts: &BlockFacts,
        registry: &horizon_world::registry::BlockRegistry,
    ) -> bool {
        let (lo, hi) = (self.bounds().min_position(), self.bounds().max_position());
        let mut water = false;
        let mut lava = false;
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..hi.y.ceil() as i32 {
                for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                    match facts.get(world.block(x, y, z)).fluid {
                        Fluid::Water => water = true,
                        Fluid::Lava => lava = true,
                        _ => {}
                    }
                }
            }
        }
        if water {
            self.fire_ticks = 0;
        }
        if self.fire_ticks > 0 {
            if self.fire_ticks % 20 == 0
                && !lava
                && !self.invisible
                && !self.marker
                && self.health > 0.5
            {
                self.health -= 4.0;
            }
            self.fire_ticks -= 1;
        }
        if lava {
            self.fire_ticks = self.fire_ticks.max(300);
        }
        let fire = crate::motion::environment::fire_contact(world, registry, self.bounds()) > 0.0;
        if fire && !self.invisible && !self.marker {
            if self.fire_ticks > 0 {
                self.health -= 0.15;
            } else {
                self.fire_ticks = 100;
            }
        }
        self.health > 0.5
    }
    /// EntityType.ARMOR_STAND dimensions (0.5 x 1.975, half for small).
    pub fn bounds(&self) -> Aabb {
        let s = if self.small { 0.5 } else { 1.0 };
        let (w, h) = (0.25 * s, 1.975 * s);
        let p = self.position;
        Aabb::new(
            Vec3::new(p.x - w, p.y, p.z - w),
            Vec3::new(p.x + w, p.y + h, p.z + w),
        )
    }
    /// Leather boots on its feet: it walks on powder snow (`PowderSnowBlock.canEntityWalkOnPowderSnow`).
    fn feet_are_leather_boots(&self) -> bool {
        self.equipment[2]
            .as_ref()
            .is_some_and(|s| &*s.item == "minecraft:leather_boots")
    }
    /// LivingEntity.travel (armor stands have no other movement): clamp tiny
    /// velocity components (`LivingEntity.aiStep`), move by the velocity (a
    /// landing stops the fall, `Block.updateEntityMovementAfterFallOn`, and a
    /// wall collision zeros that horizontal axis), let the blocks it is inside
    /// push it (`push`: a bubble column), then in water the 0.8 slowdown and
    /// `getFluidFallingAdjustedMovement`, in air gravity 0.08, ground/air
    /// horizontal drag (`friction * 0.91`), and the 0.98 vertical drag, so a
    /// stand at rest on the ground keeps -0.0784 as vanilla's.
    pub fn tick(
        &mut self,
        scene: &mut crate::collisions::SceneCollisions,
        facts: &BlockFacts,
        in_water: bool,
        push: &mut dyn FnMut(Aabb, &mut Vec3),
    ) {
        self.previous = self.position;
        if self.marker {
            return;
        }
        scene.subject =
            self.feet_are_leather_boots()
                .then_some(crate::collisions::PowderSnowSubject {
                    leather_boots: true,
                    ..Default::default()
                });
        let v = self.velocity;
        let wanted = Vec3::new(
            if v.x.abs() < 0.003 { 0.0 } else { v.x },
            if v.y.abs() < 0.003 { 0.0 } else { v.y },
            if v.z.abs() < 0.003 { 0.0 } else { v.z },
        );
        self.velocity = wanted;
        // `travelInFluid` reads `isFalling` before the move and any push;
        // `travelInAir` reads ground friction before the move.
        let falling = wanted.y <= 0.0;
        let friction = if !in_water && self.on_ground {
            let below = scene.world.block(
                self.position.x.floor() as i32,
                (self.position.y - 0.500_001).floor() as i32,
                self.position.z.floor() as i32,
            );
            facts.get(below).friction
        } else {
            1.0
        };
        let mut velocity = wanted;
        if let Ok(moved) = scene.resolve(self.bounds(), wanted, 0.0, self.on_ground) {
            self.position = Vec3::new(
                self.position.x + moved.x,
                self.position.y + moved.y,
                self.position.z + moved.z,
            );
            self.on_ground = wanted.y < 0.0 && moved.y > wanted.y;
            if (wanted.x - moved.x).abs() >= f64::from(1.0e-5_f32) {
                velocity.x = 0.0;
            }
            if moved.y != wanted.y {
                velocity.y = 0.0;
            }
            if (wanted.z - moved.z).abs() >= f64::from(1.0e-5_f32) {
                velocity.z = 0.0;
            }
        }
        push(self.bounds(), &mut velocity);
        self.velocity = if in_water {
            // travelInWater: slowDown 0.8, then `getFluidFallingAdjustedMovement`
            // with gravity / 16 (the stand is falling while its fall is not
            // already nearly stopped).
            let slow = f64::from(0.8f32);
            let y = velocity.y * slow;
            let adjusted =
                if falling && (y - 0.005).abs() >= 0.003 && (y - 0.08 / 16.0).abs() < 0.003 {
                    -0.003
                } else {
                    y - 0.08 / 16.0
                };
            Vec3::new(velocity.x * slow, adjusted, velocity.z * slow)
        } else {
            let drag = f64::from(friction * 0.91f32);
            Vec3::new(
                velocity.x * drag,
                (velocity.y - 0.08) * f64::from(0.98f32),
                velocity.z * drag,
            )
        };
    }
}

#[cfg(test)]
mod experience_tests {
    use super::*;

    #[test]
    fn death_reward_uses_levels_and_caps_the_recoverable_points() {
        for (level, reward) in [(0, 0), (1, 7), (14, 98), (15, 100), (30, 100)] {
            let xp = Experience {
                level,
                progress: 0.75,
                total: 10_000,
            };
            assert_eq!(xp.death_reward(false), reward);
            assert_eq!(xp.death_reward(true), 0);
        }
    }

    #[test]
    fn respawn_clears_all_xp_unless_it_is_kept() {
        let mut xp = Experience::default();
        xp.give(155);
        let before = (xp.level, xp.progress, xp.total);
        xp.respawn(true);
        assert_eq!((xp.level, xp.progress, xp.total), before);
        xp.respawn(false);
        assert_eq!((xp.level, xp.progress, xp.total), (0, 0.0, 0));
        xp.give(7);
        assert_eq!((xp.level, xp.progress, xp.total), (1, 0.0, 7));
    }
}

#[cfg(test)]
mod fire_environment_tests {
    use super::*;
    use horizon_world::loot::PackOverlay;
    #[test]
    fn ordinary_items_burn_but_native_netherite_resists_lava() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let facts = BlockFacts::load(&root, assets.registry).unwrap();
        let items =
            crate::items::Items::load(&root, assets.registry, &facts, &PackOverlay::NONE).unwrap();
        let mut world = World::test_scene(&assets, &render, 3).unwrap();
        let at = Vec3::new(20.5, 4.0, 20.5);
        world
            .set(
                20,
                4,
                20,
                assets.registry.default_state("minecraft:lava").unwrap(),
            )
            .unwrap();
        let make = |id: &str| ItemEntity {
            no_gravity: false,
            id: 1,
            thrower: None,
            target: None,
            stack: ItemStack {
                item: std::sync::Arc::from(id),
                count: 1,
                damage: 0,
                extra: None,
            },
            health: 5,
            fire_ticks: 0,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            on_ground: false,
            age: 0,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 0,
            needs_sync: false,
            stuck: Vec3::ZERO,
        };
        let mut wood = make("minecraft:oak_planks");
        let mut metal = make("minecraft:netherite_ingot");
        let first = wood.environment_tick(&world, &facts, assets.registry, &items);
        assert!(first.alive && first.lava_hurt);
        assert!(
            !first.should_play_lava_hurt_sound(1),
            "ordinary entry damage must not repeat the burn sound"
        );
        assert!(first.should_play_lava_hurt_sound(10));
        assert!(
            wood.fire_ticks > 0,
            "lava contact must ignite dropped items"
        );
        let last = wood.environment_tick(&world, &facts, assets.registry, &items);
        assert!(
            !last.alive && last.lava_hurt,
            "lethal lava damage still emits burn sound"
        );
        assert!(
            last.should_play_lava_hurt_sound(2),
            "the lethal hit must always play the sound"
        );
        for _ in 0..40 {
            let result = metal.environment_tick(&world, &facts, assets.registry, &items);
            assert!(result.alive && !result.lava_hurt);
            assert!(!result.should_play_lava_hurt_sound(10));
        }
        assert_eq!(metal.health, 5);
        assert_eq!(metal.fire_ticks, 0, "fire-resistant items must not ignite");
        world
            .set(
                20,
                4,
                20,
                assets.registry.default_state("minecraft:fire").unwrap(),
            )
            .unwrap();
        let mut burning = make("minecraft:oak_planks");
        let result = burning.environment_tick(&world, &facts, assets.registry, &items);
        assert!(result.alive && !result.lava_hurt);
        assert!(burning.fire_ticks > 0);
        world
            .set(
                20,
                4,
                20,
                assets
                    .registry
                    .default_state("minecraft:soul_fire")
                    .unwrap(),
            )
            .unwrap();
        let mut soul = make("minecraft:oak_planks");
        soul.environment_tick(&world, &facts, assets.registry, &items);
        assert_eq!(soul.health, 3);
        let mut stand = ArmorStand::new(2, at, 0.0);
        world
            .set(
                20,
                4,
                20,
                assets.registry.default_state("minecraft:lava").unwrap(),
            )
            .unwrap();
        for _ in 0..150 {
            assert!(stand.environment_tick(&world, &facts, assets.registry));
        }
        assert_eq!(stand.health, 20.0, "lava does not hurt armor stands");
    }
}
#[cfg(test)]
mod armor_stand_render_tests {
    use super::*;

    /// A falling stand is drawn between last tick's position and this one's,
    /// so it slides down every frame instead of jumping once a tick (#4416).
    #[test]
    fn a_falling_stand_is_drawn_between_its_ticks() {
        let mut stand = ArmorStand::new(1, Vec3::new(0.5, 70.0, 0.5), 0.0);
        // A new stand has no move behind it.
        assert_eq!(stand.render_position(0.5), stand.position);
        let fall = |s: &ArmorStand| Some((Vec3::new(0.5, s.position.y - 1.0, 0.5), 0.0));
        stand.interpolate(fall(&stand));
        assert_eq!(stand.render_position(0.0).y, 70.0);
        assert_eq!(stand.render_position(0.25).y, 69.75);
        assert_eq!(stand.render_position(1.0).y, 69.0);
        stand.interpolate(fall(&stand));
        assert_eq!(stand.render_position(0.5).y, 68.5);
        // Landed: no step is due, so it stays put, and so does every frame.
        stand.interpolate(None);
        assert_eq!(stand.render_position(0.5).y, 68.0);
    }

    /// A turning stand's body turns between ticks too, the short way round.
    #[test]
    fn a_turning_stand_is_drawn_between_its_ticks() {
        let mut stand = ArmorStand::new(1, Vec3::ZERO, 170.0);
        assert_eq!(stand.render_yaw(0.5), 170.0);
        // To -170 is 20 degrees on through 180, not 340 back through 0.
        stand.interpolate(Some((Vec3::ZERO, -170.0)));
        assert_eq!(stand.render_yaw(0.0), 170.0);
        assert_eq!(stand.render_yaw(0.5), 180.0);
        assert_eq!(stand.render_yaw(1.0), 190.0);
        stand.interpolate(None);
        assert_eq!(stand.render_yaw(0.5), 190.0);
    }

    /// A snap (a move too far to interpolate) leaves nothing to blend from.
    #[test]
    fn a_snapped_stand_is_drawn_where_it_landed() {
        let mut stand = ArmorStand::new(1, Vec3::new(0.5, 70.0, 0.5), 0.0);
        stand.interpolate(Some((Vec3::new(0.5, 69.0, 0.5), 0.0)));
        stand.snap(Vec3::new(200.5, 64.0, 0.5), 90.0);
        assert_eq!(stand.render_position(0.5), Vec3::new(200.5, 64.0, 0.5));
        assert_eq!(stand.render_yaw(0.5), 90.0);
    }
}
#[cfg(test)]
mod merge_tests {
    use super::*;

    fn item(id: &str, count: u32, x: f64, age: u32) -> ItemEntity {
        let at = Vec3::new(x, 64.0, 0.5);
        ItemEntity {
            no_gravity: false,
            id: 0,
            thrower: None,
            target: None,
            stack: ItemStack {
                item: std::sync::Arc::from(id),
                count,
                damage: 0,
                extra: None,
            },
            health: 5,
            fire_ticks: 0,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            on_ground: true,
            age,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: age,
            needs_sync: false,
            stuck: Vec3::ZERO,
        }
    }
    fn max_stack(stack: &ItemStack) -> u32 {
        if &*stack.item == "minecraft:ender_pearl" {
            16
        } else {
            64
        }
    }
    /// Merges around `items[i]` as its tick would; returns the counts left.
    fn merge(items: &mut [ItemEntity], i: usize) -> Vec<Option<u32>> {
        let mut removed = vec![false; items.len()];
        let mut cells = ItemCells::new(items);
        merge_with_neighbours(items, &mut removed, &mut cells, i, &max_stack);
        items
            .iter()
            .zip(removed)
            .map(|(item, gone)| (!gone).then_some(item.stack.count))
            .collect()
    }

    #[test]
    fn the_smaller_stack_goes_into_the_larger() {
        let mut items = [
            item("minecraft:sand", 3, 0.5, 41),
            item("minecraft:sand", 5, 1.2, 7),
        ];
        assert_eq!(merge(&mut items, 0), [None, Some(8)]);
        // ItemEntity.merge keeps the younger age; the ticking item's is
        // still the one before its tick.
        assert_eq!(items[1].age, 7);
        let mut items = [
            item("minecraft:sand", 5, 0.5, 41),
            item("minecraft:sand", 3, 1.2, 7),
        ];
        assert_eq!(merge(&mut items, 0), [Some(8), None]);
        assert_eq!(items[0].age, 8, "age 7, plus the rest of this tick");
    }

    #[test]
    fn items_merge_only_within_half_a_block_sideways() {
        // Boxes 0.25 wide grown by 0.5: centres closer than 0.75 meet.
        let mut items = [
            item("minecraft:sand", 1, 0.5, 1),
            item("minecraft:sand", 1, 1.24, 1),
        ];
        assert_eq!(merge(&mut items, 0), [None, Some(2)]);
        let mut items = [
            item("minecraft:sand", 1, 0.5, 1),
            item("minecraft:sand", 1, 1.26, 1),
        ];
        assert_eq!(merge(&mut items, 0), [Some(1), Some(1)]);
        let mut items = [
            item("minecraft:sand", 1, 0.5, 1),
            item("minecraft:sand", 1, 0.5, 1),
        ];
        items[1].position.y += 0.3;
        assert_eq!(
            merge(&mut items, 0),
            [Some(1), Some(1)],
            "no vertical growth"
        );
    }

    #[test]
    fn different_or_full_stacks_stay_apart() {
        let mut items = [
            item("minecraft:sand", 1, 0.5, 1),
            item("minecraft:gravel", 1, 0.5, 1),
        ];
        assert_eq!(merge(&mut items, 0), [Some(1), Some(1)]);
        let mut items = [
            item("minecraft:sand", 60, 0.5, 1),
            item("minecraft:sand", 10, 0.5, 1),
        ];
        assert_eq!(
            merge(&mut items, 0),
            [Some(60), Some(10)],
            "70 is over a stack"
        );
        let mut items = [
            item("minecraft:ender_pearl", 10, 0.5, 1),
            item("minecraft:ender_pearl", 6, 0.5, 1),
        ];
        assert_eq!(merge(&mut items, 0), [Some(16), None]);
        let mut items = [
            item("minecraft:ender_pearl", 16, 0.5, 1),
            item("minecraft:ender_pearl", 1, 0.5, 1),
        ];
        assert_eq!(
            merge(&mut items, 0),
            [Some(16), Some(1)],
            "a full stack doesn't merge"
        );
    }

    #[test]
    fn unmergeable_items_are_skipped() {
        let mut items = [
            item("minecraft:sand", 1, 0.5, 1),
            item("minecraft:sand", 1, 0.5, 1),
        ];
        items[1].pickup_delay = u32::MAX;
        assert_eq!(merge(&mut items, 0), [Some(1), Some(1)]);
        // At age 6000 the other item's lifetime is over; this one's tick
        // started at 5999, so it may still merge into a live item.
        let mut items = [
            item("minecraft:sand", 1, 0.5, 6000),
            item("minecraft:sand", 1, 0.5, 6000),
        ];
        assert_eq!(merge(&mut items, 0), [Some(1), Some(1)]);
        items[1].age = 10;
        assert_eq!(merge(&mut items, 0), [None, Some(2)]);
        assert_eq!(items[1].age, 10);
    }

    #[test]
    fn a_stack_takes_its_neighbours_in_spawn_order_until_full() {
        let mut items = [
            item("minecraft:sand", 40, 0.5, 1),
            item("minecraft:sand", 20, 0.7, 1),
            item("minecraft:sand", 10, 0.3, 1),
            item("minecraft:sand", 3, 0.4, 1),
        ];
        // 40 + 20 = 60; 60 + 10 is over 64; then 3 more.
        assert_eq!(merge(&mut items, 0), [Some(63), None, Some(10), None]);
        let mut items = [
            item("minecraft:sand", 1, 0.5, 1),
            item("minecraft:sand", 1, 0.5, 1),
        ];
        items[1].pickup_delay = 30;
        assert_eq!(merge(&mut items, 0), [None, Some(2)]);
        assert_eq!(items[1].pickup_delay, 30, "the longer pickup delay stays");
    }

    #[test]
    fn the_cells_follow_moves() {
        let mut items = [
            item("minecraft:sand", 1, 0.5, 1),
            item("minecraft:sand", 1, 5.5, 1),
        ];
        let mut removed = [false; 2];
        let mut cells = ItemCells::new(&items);
        let from = items[1].position;
        items[1].position.x = 0.9;
        cells.moved(1, from, items[1].position);
        merge_with_neighbours(&mut items, &mut removed, &mut cells, 0, &max_stack);
        assert_eq!(removed, [true, false]);
        assert_eq!(items[1].stack.count, 2);
    }
}

#[cfg(test)]
mod honey_slide_tests {
    use super::*;

    /// Gravity still stacks on idle ticks; waking every 4th tick zeroes Y on
    /// the ground (`ItemEntity.tick` with `onGround`).
    #[test]
    fn a_resting_item_skips_move_three_of_four_ticks() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = crate::motion::FluidResources::new(&assets, &facts).unwrap();
        let collision = crate::collisions::load(&root, assets.registry).unwrap();
        let mut world = World::test_scene(&assets, &render, 3).unwrap();
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        for x in 19..=21 {
            for z in 19..=21 {
                world.set(x, 3, z, stone).unwrap();
            }
        }
        let at = Vec3::new(20.5, 4.0, 20.5);
        let mut item = falling_item(at.x, at.y, at.z, 0.0, 0.0, 0.0);
        item.on_ground = true;
        item.age = 0;
        item.id = 1;
        let mut ys = Vec::new();
        for _ in 0..8 {
            item.tick(
                &world,
                assets.registry,
                &facts,
                &collision,
                false,
                &fluids,
                &[],
            );
            ys.push(item.velocity.y);
        }
        // Idle ticks keep gravity; the 4th tick's move hits the floor and
        // clears Y (vanilla's sequence for a resting item).
        assert_eq!(ys, vec![-0.04, -0.08, -0.12, 0.0, -0.04, -0.08, -0.12, 0.0]);
        assert_eq!(item.position.y, at.y);
    }

    /// #4558: idle ticks still run `BubbleColumnBlock.entityInside`, so lift
    /// builds and the next wake-up rises off the soul sand.
    #[test]
    fn a_grounded_item_in_a_soul_sand_column_rises() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = crate::motion::FluidResources::new(&assets, &facts).unwrap();
        let collision = crate::collisions::load(&root, assets.registry).unwrap();
        let mut world = World::test_scene(&assets, &render, 3).unwrap();
        let soul = assets
            .registry
            .default_state("minecraft:soul_sand")
            .unwrap();
        let mut column = assets
            .registry
            .state(
                assets
                    .registry
                    .default_state("minecraft:bubble_column")
                    .unwrap(),
            )
            .unwrap()
            .clone();
        column.properties.insert("drag".into(), "false".into());
        let column = assets.registry.resolve(&column).unwrap();
        for x in 19..=21 {
            for z in 19..=21 {
                world.set(x, 3, z, soul).unwrap();
                for y in 4..=8 {
                    world.set(x, y, z, column).unwrap();
                }
            }
        }
        let at = Vec3::new(20.5, 4.0, 20.5);
        let mut item = falling_item(at.x, at.y, at.z, 0.0, 0.0, 0.0);
        item.on_ground = true;
        item.age = 0;
        item.id = 1;
        let before = item.position.y;
        for _ in 0..16 {
            item.tick(
                &world,
                assets.registry,
                &facts,
                &collision,
                false,
                &fluids,
                &[],
            );
        }
        assert!(
            item.position.y > before + 0.5,
            "expected rise from soul-sand column, y={} (was {})",
            item.position.y,
            before
        );
    }

    /// `Entity.sendBubbleColumnParticles` is for the top of a column only: an item
    /// inside the column shows nothing, one in its surface block shows the block.
    #[test]
    fn an_item_at_the_top_of_a_bubble_column_is_at_its_surface() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let collision = crate::collisions::load(&root, assets.registry).unwrap();
        let mut world = World::test_scene(&assets, &render, 3).unwrap();
        let mut column = assets
            .registry
            .state(
                assets
                    .registry
                    .default_state("minecraft:bubble_column")
                    .unwrap(),
            )
            .unwrap()
            .clone();
        column.properties.insert("drag".into(), "false".into());
        let column = assets.registry.resolve(&column).unwrap();
        for y in 4..=8 {
            world.set(20, y, 20, column).unwrap();
        }
        let surfaces = |y: f64| {
            let item = falling_item(20.5, y, 20.5, 0.0, 0.0, 0.0);
            crate::motion::bubble_column_surfaces(
                &world,
                assets.registry,
                &facts,
                &collision,
                item.bounds(),
            )
        };
        assert_eq!(surfaces(8.2), vec![BlockPos::new(20, 8, 20)]);
        assert_eq!(surfaces(5.0), Vec::new());
        // Straddling the two top blocks, it is still at the top of one column.
        assert_eq!(surfaces(7.9), vec![BlockPos::new(20, 8, 20)]);
        assert_eq!(surfaces(9.0), Vec::new());
    }

    pub(super) fn falling_item(x: f64, y: f64, z: f64, vx: f64, vy: f64, vz: f64) -> ItemEntity {
        let at = Vec3::new(x, y, z);
        ItemEntity {
            no_gravity: false,
            id: 1,
            thrower: None,
            target: None,
            stack: ItemStack {
                item: std::sync::Arc::from("minecraft:stone"),
                count: 1,
                damage: 0,
                extra: None,
            },
            health: 5,
            fire_ticks: 0,
            position: at,
            previous: at,
            velocity: Vec3::new(vx, vy, vz),
            on_ground: false,
            age: 10,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 10,
            needs_sync: false,
            stuck: Vec3::ZERO,
        }
    }

    fn old_delta_y(dy: f64) -> f64 {
        dy / f64::from(0.98_f32) + 0.08
    }
    fn new_delta_y(dy: f64) -> f64 {
        (dy - 0.08) * f64::from(0.98_f32)
    }

    #[test]
    fn a_falling_item_beside_honey_slides_down() {
        // Item width 0.25: overlap 0.5625. Honey at (0, 10, 0); centre 0.5.
        // x = -0.0625 is against the west face (inset 1/16).
        let mut item = falling_item(-0.0625, 10.4, 0.5, 0.12, -0.20, 0.04);
        assert!(old_delta_y(-0.20) < -0.08);
        assert!(old_delta_y(-0.20) >= -0.13);
        item.honey_block_inside([0, 10, 0]);
        assert_eq!(item.velocity.x, 0.12);
        assert_eq!(item.velocity.z, 0.04);
        assert_eq!(item.velocity.y, new_delta_y(-0.05));
    }

    #[test]
    fn a_fast_fall_scales_the_horizontal_speed() {
        let mut item = falling_item(-0.0625, 10.4, 0.5, 0.2, -0.30, -0.1);
        let old = old_delta_y(-0.30);
        assert!(old < -0.13);
        item.honey_block_inside([0, 10, 0]);
        let factor = -0.05 / old;
        assert_eq!(item.velocity.x, 0.2 * factor);
        assert_eq!(item.velocity.z, -0.1 * factor);
        assert_eq!(item.velocity.y, new_delta_y(-0.05));
    }

    #[test]
    fn an_item_on_the_ground_or_on_top_does_not_slide() {
        let mut grounded = falling_item(-0.0625, 10.4, 0.5, 0.0, -0.20, 0.0);
        grounded.on_ground = true;
        grounded.honey_block_inside([0, 10, 0]);
        assert_eq!(grounded.velocity.y, -0.20);

        let mut on_top = falling_item(0.5, 10.9375, 0.5, 0.0, -0.20, 0.0);
        on_top.honey_block_inside([0, 10, 0]);
        assert_eq!(on_top.velocity.y, -0.20, "standing on the honey");
    }

    #[test]
    fn a_centred_or_slow_item_does_not_slide() {
        let mut centred = falling_item(0.5, 10.4, 0.5, 0.0, -0.20, 0.0);
        centred.honey_block_inside([0, 10, 0]);
        assert_eq!(centred.velocity.y, -0.20, "inside, not on a side");

        let mut slow = falling_item(-0.0625, 10.4, 0.5, 0.0, -0.04, 0.0);
        assert!(old_delta_y(-0.04) >= -0.08);
        slow.honey_block_inside([0, 10, 0]);
        assert_eq!(slow.velocity.y, -0.04);
    }
}

#[cfg(test)]
mod slime_bounce_tests {
    use super::*;

    #[test]
    fn landing_on_slime_bounces_a_dropped_item() {
        assert_eq!(item_vertical_collision_velocity(-1.0, true), 0.8);
        assert_eq!(item_vertical_collision_velocity(-0.5, true), 0.4);
        assert_eq!(
            item_vertical_collision_velocity(-0.04, true),
            0.0,
            "gravity alone is not a bounce"
        );
        assert_eq!(item_vertical_collision_velocity(-0.041, true), 0.041 * 0.8);
    }

    #[test]
    fn landing_on_stone_or_hitting_a_ceiling_zeros_y() {
        assert_eq!(item_vertical_collision_velocity(-1.0, false), 0.0);
        assert_eq!(item_vertical_collision_velocity(1.0, true), 0.0);
        assert_eq!(item_vertical_collision_velocity(0.96, true), 0.0);
    }

    #[test]
    fn a_moving_piston_carrying_slime_still_counts_as_slime() {
        assert!(slime_support_name("minecraft:slime_block", None));
        assert!(slime_support_name(
            "minecraft:moving_piston",
            Some("minecraft:slime_block")
        ));
        assert!(slime_support_name(
            "minecraft:air",
            Some("minecraft:slime_block")
        ));
        assert!(!slime_support_name(
            "minecraft:moving_piston",
            Some("minecraft:stone")
        ));
        assert!(!slime_support_name("minecraft:stone", None));
        assert!(!slime_support_name(
            "minecraft:honey_block",
            Some("minecraft:honey_block")
        ));
    }

    fn grounded_item() -> ItemEntity {
        let at = Vec3::new(0.5, 2.0, 0.5);
        ItemEntity {
            no_gravity: false,
            id: 1,
            thrower: None,
            target: None,
            stack: ItemStack {
                item: std::sync::Arc::from("minecraft:stone"),
                count: 1,
                damage: 0,
                extra: None,
            },
            health: 5,
            fire_ticks: 0,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            on_ground: true,
            age: 10,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 10,
            needs_sync: false,
            stuck: Vec3::ZERO,
        }
    }

    #[test]
    fn a_piston_slime_bounce_launches_the_item_off_the_ground() {
        let mut item = grounded_item();
        item.piston_slime_bounce(Direction::Up);
        assert_eq!(item.velocity, Vec3::new(0.0, 1.0, 0.0));
        assert!(!item.on_ground);
        assert!(item.needs_sync);
        item.needs_sync = false;
        item.piston_slime_bounce(Direction::East);
        assert_eq!(item.velocity, Vec3::new(1.0, 1.0, 0.0));
        assert!(!item.on_ground);
        assert!(item.needs_sync);
    }

    #[test]
    fn a_slime_landing_stays_upward_after_item_friction() {
        // ItemEntity then multiplies Y by 0.98 and applies -0.5 only if still
        // negative on the ground. A slime bounce must remain positive.
        let bounced = item_vertical_collision_velocity(-1.0, true) * 0.98;
        assert!(bounced > 0.0);
        assert_eq!(bounced, 0.8 * 0.98);
    }

    #[test]
    fn a_slime_bounce_marks_the_item_for_immediate_sync() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = crate::motion::FluidResources::new(&assets, &facts).unwrap();
        let collision = crate::collisions::load(&root, assets.registry).unwrap();
        let mut world = World::test_scene(&assets, &render, 3).unwrap();
        let slime = assets
            .registry
            .default_state("minecraft:slime_block")
            .unwrap();
        for x in 19..=21 {
            for z in 19..=21 {
                world.set(x, 3, z, slime).unwrap();
            }
        }
        // ItemEntity.tick sets needsSync when getDeltaMovement().subtract(oldMovement).lengthSqr() > 0.01.
        let mut falling = grounded_item();
        falling.position = Vec3::new(20.5, 4.2, 20.5);
        falling.previous = falling.position;
        falling.velocity = Vec3::new(0.0, -0.5, 0.0);
        falling.on_ground = false;
        falling.age = 0;
        falling.id = 1;
        falling.tick(
            &world,
            assets.registry,
            &facts,
            &collision,
            false,
            &fluids,
            &[],
        );
        assert_eq!(falling.position.y, 4.0);
        assert!(falling.velocity.y > 0.0);
        assert!(falling.on_ground);
        assert!(falling.needs_sync);

        // An idle tick where velocity only steps by -0.04 (0.04^2 = 0.0016 <= 0.01)
        // does not set needs_sync.
        falling.velocity = Vec3::ZERO;
        falling.needs_sync = false;
        falling.tick(
            &world,
            assets.registry,
            &facts,
            &collision,
            false,
            &fluids,
            &[],
        );
        assert!(!falling.needs_sync);
    }
}

#[cfg(test)]
mod unstuck_tests {
    use super::*;

    const BORDER: BorderBounds = BorderBounds {
        min_x: -1.0e6,
        min_z: -1.0e6,
        max_x: 1.0e6,
        max_z: 1.0e6,
    };

    /// A level of full blocks at `solid`; every other cell is air.
    fn shapes(solid: &[(i32, i32, i32)]) -> impl FnMut(BlockPos) -> Option<VoxelShape> + '_ {
        move |p| {
            if solid.contains(&(p.x, p.y, p.z)) {
                VoxelShape::from_box(Aabb::block(p)).ok()
            } else {
                None
            }
        }
    }

    /// `level.noCollision(box)` over the same cells the search reads.
    fn free(solid: &[(i32, i32, i32)], b: Aabb) -> bool {
        !solid
            .iter()
            .any(|&(x, y, z)| Aabb::block(BlockPos::new(x, y, z)).intersects(b))
    }

    #[test]
    fn an_orb_spawned_in_a_block_moves_to_a_free_box() {
        // One block at y 0..1 and an orb standing in its upper half, as a mob
        // killed inside one leaves it: its box overlaps the block by 0.1.
        let solid = [(0, 0, 0)];
        let at = Vec3::new(0.5, 0.9, 0.5);
        let moved = ExperienceOrb::unstuck(at, &BORDER, shapes(&solid)).expect("the orb moves");
        // The nearest free box sits on the block's top face.
        assert_eq!(moved, Vec3::new(0.5, 1.0, 0.5));
        assert!(!ExperienceOrb::box_at(moved).intersects(Aabb::block(BlockPos::new(0, 0, 0))));
    }

    #[test]
    fn an_orb_in_open_air_keeps_its_position() {
        let solid = [(0, 0, 0)];
        let at = Vec3::new(0.5, 2.0, 0.5);
        assert!(free(&solid, ExperienceOrb::box_at(at)));
        // Nothing to move for: the spawn sites keep the position as given.
        assert_eq!(ExperienceOrb::unstuck(at, &BORDER, shapes(&solid)), None);
    }
}

#[cfg(test)]
mod entity_collision_tests {
    use super::honey_slide_tests::falling_item;
    use super::*;

    /// `Entity.collide` stops an item at the entities that `canBeCollidedWith` it: dropped
    /// above a boat's box (or a still happy ghast's) it lands on the box's top and rests,
    /// where with no box it falls to the floor of the scene.
    #[test]
    fn an_item_lands_on_an_entity_box_under_it() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = crate::motion::FluidResources::new(&assets, &facts).unwrap();
        let collision = crate::collisions::load(&root, assets.registry).unwrap();
        let mut world = World::test_scene(&assets, &render, 3).unwrap();
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        world.set(20, 3, 20, stone).unwrap();
        let rest = |boxes: &[Aabb]| {
            let mut item = falling_item(20.5, 12.0, 20.5, 0.0, 0.0, 0.0);
            item.id = 1;
            for _ in 0..80 {
                item.tick(
                    &world,
                    assets.registry,
                    &facts,
                    &collision,
                    false,
                    &fluids,
                    boxes,
                );
            }
            item.position.y
        };
        // The stone's top is at 4.0; a box from 5.0 to 9.0 is in the way.
        let platform = Aabb::new(Vec3::new(19.0, 5.0, 19.0), Vec3::new(22.0, 9.0, 22.0));
        assert!((rest(&[]) - 4.0).abs() < 1.0e-6, "on the stone with no box");
        assert!(
            (rest(&[platform]) - 9.0).abs() < 1.0e-6,
            "on top of the box, at its top"
        );
    }

    /// `WebBlock.entityInside` / `PowderSnowBlock.entityInside` make an item stuck: a
    /// fast item sliding into a cobweb is stopped inside it, in powder snow it is slowed.
    #[test]
    fn a_cobweb_and_powder_snow_slow_an_item() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = crate::motion::FluidResources::new(&assets, &facts).unwrap();
        let collision = crate::collisions::load(&root, assets.registry).unwrap();
        let mut world = World::test_scene(&assets, &render, 3).unwrap();
        let state = |n: &str| assets.registry.default_state(n).unwrap();
        for x in 16..32 {
            world.set(x, 3, 20, state("minecraft:stone")).unwrap();
        }
        let mut slide = |block: Option<&str>| {
            world
                .set(21, 4, 20, state(block.unwrap_or("minecraft:air")))
                .unwrap();
            let mut item = falling_item(20.5, 4.0, 20.5, 0.4, 0.0, 0.0);
            item.id = 1;
            for _ in 0..12 {
                item.tick(
                    &world,
                    assets.registry,
                    &facts,
                    &collision,
                    false,
                    &fluids,
                    &[],
                );
            }
            item.position.x
        };
        let free = slide(None);
        let web = slide(Some("minecraft:cobweb"));
        let snow = slide(Some("minecraft:powder_snow"));
        assert!(free > 21.4, "an unobstructed item slides on to {free}");
        assert!(
            web > 20.8 && web < free - 0.3,
            "the web holds it: {web} vs {free}"
        );
        assert!(snow < free, "powder snow slows it: {snow} vs {free}");
    }

    #[test]
    fn obstacle_grid_returns_only_the_boxes_near_an_item() {
        // A boat every 100 blocks along a line, 1000 of them.
        let boat = |x: f64| {
            Aabb::new(
                Vec3::new(x, 64.0, 0.0),
                Vec3::new(x + 1.375, 64.5625, 1.375),
            )
        };
        let grid = ObstacleGrid::new((0..1000).map(|i| boat(f64::from(i) * 100.0)).collect());
        let mut out = Vec::new();
        // Around the 7th boat: only it.
        let area = Aabb::new(Vec3::new(699.0, 63.0, -1.0), Vec3::new(701.0, 66.0, 1.0));
        grid.near(area, &mut out);
        assert_eq!(out, vec![boat(700.0)]);
        // Between two boats: none.
        let area = Aabb::new(Vec3::new(750.0, 63.0, 0.0), Vec3::new(751.0, 66.0, 1.0));
        grid.near(area, &mut out);
        assert!(out.is_empty());
        // A box spanning cells is found from any of them, once.
        let wide = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(20.0, 1.0, 1.0));
        let grid = ObstacleGrid::new(vec![wide]);
        let area = Aabb::new(Vec3::new(17.0, 0.0, 0.0), Vec3::new(18.0, 1.0, 1.0));
        grid.near(area, &mut out);
        assert_eq!(out, vec![wide]);
    }

    /// A farm: 5000 drops and 1000 separated vehicles. Run with `-- --ignored --nocapture`.
    #[test]
    #[ignore = "timing"]
    fn obstacle_grid_beats_scanning_every_box_for_a_big_farm() {
        let boat = |i: u32| {
            let (x, z) = (f64::from(i % 40) * 6.0, f64::from(i / 40) * 6.0);
            Aabb::new(
                Vec3::new(x, 64.0, z),
                Vec3::new(x + 1.375, 64.5625, z + 1.375),
            )
        };
        let boxes: Vec<Aabb> = (0..1000).map(boat).collect();
        let items: Vec<Aabb> = (0..5000)
            .map(|i| {
                let (x, z) = (f64::from(i % 70) * 3.4, f64::from(i / 70) * 3.4);
                Aabb::new(Vec3::new(x, 64.0, z), Vec3::new(x + 0.25, 64.25, z + 0.25))
            })
            .collect();
        let start = std::time::Instant::now();
        let mut hits = 0;
        for item in &items {
            let area = item.inflate(Vec3::new(1.0, 1.0, 1.0));
            hits += boxes.iter().filter(|b| b.intersects(area)).count();
        }
        let scan = start.elapsed();
        let grid = ObstacleGrid::new(boxes);
        let start = std::time::Instant::now();
        let (mut out, mut found) = (Vec::new(), 0);
        for item in &items {
            grid.near(item.inflate(Vec3::new(1.0, 1.0, 1.0)), &mut out);
            found += out.len();
        }
        let binned = start.elapsed();
        assert_eq!(hits, found);
        println!("scan {scan:?}, grid {binned:?}");
    }
}

#[cfg(test)]
mod armor_stand_motion_tests {
    use super::*;

    #[test]
    fn a_new_stand_starts_with_matching_previous() {
        let at = Vec3::new(1.0, 2.0, 3.0);
        let stand = ArmorStand::new(1, at, 90.0);
        assert_eq!(stand.previous, at);
        assert_eq!(stand.position, at);
    }

    fn tick_stand(
        stand: &mut ArmorStand,
        world: &World,
        facts: &BlockFacts,
        collision: &BlockCollisionRegistry<'_>,
    ) {
        let mut scene = SceneCollisions {
            world,
            binding: collision,
            subject: None,
        };
        stand.tick(&mut scene, facts, false, &mut |_, _| {});
    }

    #[test]
    fn a_tick_keeps_the_previous_position_for_interpolation() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let collision = crate::collisions::load(&root, assets.registry).unwrap();
        let facts = BlockFacts::load(&root, assets.registry).unwrap();
        let world = World::test_scene(&assets, &render, 3).unwrap();
        let start = Vec3::new(20.5, 20.0, 20.5);
        let mut stand = ArmorStand::new(1, start, 0.0);
        stand.on_ground = false;
        // A summoned stand has no motion yet: the first tick only builds it
        // (vanilla moves by the velocity, then applies gravity), so it falls
        // from the second tick on (measured on the real server: drop 0, 0, 0.0784 blocks).
        tick_stand(&mut stand, &world, &facts, &collision);
        assert_eq!(stand.position, start);
        assert!((stand.velocity.y + 0.0784).abs() < 1e-6);
        tick_stand(&mut stand, &world, &facts, &collision);
        assert_eq!(stand.previous, start);
        assert!(
            stand.position.y < start.y,
            "an airborne stand must fall, y={}",
            stand.position.y
        );
        let mid = stand.previous.y + (stand.position.y - stand.previous.y) * 0.5;
        assert!(
            mid < start.y && mid > stand.position.y,
            "alpha 0.5 must sit between previous and current"
        );
    }

    #[test]
    fn leather_boots_walk_on_powder_snow() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let collision = crate::collisions::load(&root, assets.registry).unwrap();
        let facts = BlockFacts::load(&root, assets.registry).unwrap();
        let mut world = World::test_scene(&assets, &render, 3).unwrap();
        let snow = assets
            .registry
            .default_state("minecraft:powder_snow")
            .unwrap();
        for x in 19..=21 {
            for z in 19..=21 {
                world.set(x, 10, z, snow).unwrap();
            }
        }
        let start = Vec3::new(20.5, 11.0, 20.5);
        let mut bare = ArmorStand::new(1, start, 0.0);
        bare.on_ground = true;
        for _ in 0..8 {
            tick_stand(&mut bare, &world, &facts, &collision);
        }
        assert!(
            bare.position.y < start.y - 0.2,
            "bare feet must sink into powder snow, y={}",
            bare.position.y
        );

        let mut booted = ArmorStand::new(2, start, 0.0);
        booted.on_ground = true;
        booted.equipment[2] = Some(ItemStack::new("minecraft:leather_boots", 1));
        for _ in 0..8 {
            tick_stand(&mut booted, &world, &facts, &collision);
        }
        assert!(
            (booted.position.y - start.y).abs() < 1.0e-6,
            "leather boots must walk on powder snow, y={}",
            booted.position.y
        );
    }
}
