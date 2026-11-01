//! Boats and rafts (`AbstractBoat`): water/land/air status, floating,
//! paddling controls, collisions, riding (`positionRider`, `clampRotation`,
//! dismounting), `BoatItem` placement, `VehicleEntity.hurtServer` breaking,
//! saving and `AbstractBoatRenderer` with animated paddles.
use super::{Game, InteractTarget, Keys, entity_culling::at, view_vector};
use horizon_client::{
    blocks::Fluid,
    collisions::{EntityCollisions, SceneCollisions},
    items::ItemStack,
    mesher::Vertex,
    mobs::{
        Part,
        anim::Poses,
        leash::{Holder, Pending},
        riders,
    },
    motion::{BubbleColumn, RiddenBoat, bubble_column_at},
};
use horizon_core::{
    geometry::{Aabb, ShapeOp, Vec3, VoxelShape},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::collision_context::CollisionContext;
use std::{collections::HashMap, f32::consts::PI, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Status {
    InWater,
    UnderWater,
    UnderFlowingWater,
    OnLand,
    InAir,
}
#[derive(Clone, Debug)]
pub(super) struct Boat {
    pub id: u32,
    /// Wood type (`oak`, `bamboo` for the raft).
    pub wood: Arc<str>,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub yaw: f32,
    pub previous_yaw: f32,
    pub delta_rotation: f32,
    status: Status,
    old_status: Status,
    water_level: f64,
    land_friction: f32,
    out_of_control: f32,
    /// `AbstractBoat.DATA_ID_BUBBLE_TIME` (synched): the ticks of `BUBBLE_TIME`
    /// left before a bubble column lifts or drags the boat; 0 when it is not
    /// over one.
    pub bubble_time: i32,
    /// `AbstractBoat.isAboveBubbleColumn`, set by `onAboveBubbleColumn` and
    /// cleared by `tickBubbleColumn` (the server's).
    above_bubble_column: bool,
    /// `AbstractBoat.bubbleColumnDirectionIsDown`.
    bubble_down: bool,
    /// A bubble column's drag asked a rider the server can't move itself (a
    /// player) to get out: `guest_vehicle_tick` takes it (`ejectPassengers`).
    pub eject_riders: bool,
    /// `Entity.tickCount`, counted where the wobble is (a client's tick).
    tick_count: i32,
    /// `AbstractBoat.bubbleMultiplier`, `bubbleAngle` and `bubbleAngleO`
    /// (a client's).
    bubble_multiplier: f32,
    bubble_angle: f32,
    bubble_angle_old: f32,
    pub hurt_time: i32,
    pub hurt_dir: i32,
    pub damage: f32,
    pub paddles: [f32; 2],
    pub paddling: [bool; 2],
    /// left, right, up, down
    pub input: [bool; 4],
    pub on_ground: bool,
    /// `ChestBoat`: a chest with 27 slots in the back seat.
    pub chest: bool,
    pub items: Vec<Option<ItemStack>>,
    /// `AbstractBoat.leashData`: who holds its lead (`boat_leads.rs`).
    pub leash: Option<Holder>,
    /// The player a `Holder::Player` lead is tied to (their UUID).
    pub leash_player: Option<[i32; 4]>,
    /// A holder read from a save that is not in the world yet.
    pub leash_pending: Option<Pending>,
    /// `LeashData.angularMomentum`.
    pub leash_spin: f64,
    /// `Entity.tickCount`, for the 100 ticks a saved holder is waited for.
    pub age: i32,
}
/// `ChestBoat.CONTAINER_SIZE`.
pub(super) const CHEST_SLOTS: usize = 27;
/// `AbstractBoat.BUBBLE_TIME`: the ticks over a bubble column before it
/// lifts or drags the boat.
const BUBBLE_TIME: i32 = 60;
/// `EntityEvent` 71 and 72, which `AbstractBoat.tickBubbleColumn` broadcasts
/// for a down and an up column.
pub(super) const BUBBLE_DOWN_EVENT: i8 = 71;
pub(super) const BUBBLE_UP_EVENT: i8 = 72;
pub(super) const WIDTH: f64 = 1.375;
pub(super) const HEIGHT: f64 = 0.5625;
impl Boat {
    pub fn new(id: u32, wood: Arc<str>, position: Vec3, yaw: f32) -> Self {
        Self {
            id,
            wood,
            position,
            previous: position,
            velocity: Vec3::ZERO,
            yaw,
            previous_yaw: yaw,
            delta_rotation: 0.0,
            status: Status::InAir,
            old_status: Status::InAir,
            water_level: 0.0,
            land_friction: 0.0,
            out_of_control: 0.0,
            bubble_time: 0,
            above_bubble_column: false,
            bubble_down: false,
            eject_riders: false,
            tick_count: 0,
            bubble_multiplier: 0.0,
            bubble_angle: 0.0,
            bubble_angle_old: 0.0,
            hurt_time: 0,
            hurt_dir: 1,
            damage: 0.0,
            paddles: [0.0; 2],
            paddling: [false; 2],
            input: [false; 4],
            on_ground: false,
            chest: false,
            items: Vec::new(),
            leash: None,
            leash_player: None,
            leash_pending: None,
            leash_spin: 0.0,
            age: 0,
        }
    }
    /// A chest boat or chest raft.
    pub fn with_chest(mut self) -> Self {
        self.chest = true;
        self.items = vec![None; CHEST_SLOTS];
        self
    }
    pub fn bounds(&self) -> Aabb {
        let (w, p) = (WIDTH / 2.0, self.position);
        Aabb::new(
            Vec3::new(p.x - w, p.y, p.z - w),
            Vec3::new(p.x + w, p.y + HEIGHT, p.z + w),
        )
    }
    /// `isUnderWater`; it stands in for `canAddPassenger`'s
    /// `isEyeInFluid(WATER)` too.
    pub fn under_water(&self) -> bool {
        matches!(self.status, Status::UnderWater | Status::UnderFlowingWater)
    }
    /// `Entity.isInLiquid`, for `Leashable.angularFriction`.
    pub fn in_liquid(&self) -> bool {
        matches!(
            self.status,
            Status::InWater | Status::UnderWater | Status::UnderFlowingWater
        )
    }
    /// `getMaxPassengers`: two, one beside a chest.
    pub fn max_passengers(&self) -> usize {
        if self.chest { 1 } else { 2 }
    }
    /// `rideHeight`: a third of a boat's height, 8/9 of a raft's.
    fn ride_height(&self) -> f64 {
        let height = HEIGHT as f32;
        f64::from(if &*self.wood == "bamboo" {
            height * 0.888_888_9
        } else {
            height / 3.0
        })
    }
    /// Where the passenger at `index` of `count` stands, at the boat's current
    /// (or, with `previous`, last) place: `getPassengerAttachmentPoint`
    /// turned with the boat, less the passenger's `rider_offset`.
    pub fn seat(
        &self,
        index: usize,
        count: usize,
        animal: bool,
        rider_offset: f64,
        previous: bool,
    ) -> Vec3 {
        let mut z = if self.chest { 0.15f32 } else { 0.0 };
        if count > 1 {
            z = if index == 0 { 0.2 } else { -0.6 };
            if animal {
                z += 0.2;
            }
        }
        let (at, yaw) = if previous {
            (self.previous, self.previous_yaw)
        } else {
            (self.position, self.yaw)
        };
        riders::seat(
            [0.0, self.ride_height(), f64::from(z)],
            rider_offset,
            at,
            yaw,
        )
    }
    /// `AbstractBoat.getBubbleAngle`: the wobble, in degrees, `a` of the way
    /// from the last tick's to this one's.
    pub fn bubble_angle(&self, a: f32) -> f32 {
        self.bubble_angle_old + a * (self.bubble_angle - self.bubble_angle_old)
    }
    /// `AbstractBoat.tickBubbleColumn`: on a client, the wobble grows by 0.05 a
    /// tick while the synched bubble time runs and fades by 0.1 otherwise.
    pub(super) fn tick_bubble_wobble(&mut self) {
        self.tick_count += 1;
        if self.bubble_time > 0 {
            self.bubble_multiplier += 0.05;
        } else {
            self.bubble_multiplier -= 0.1;
        }
        self.bubble_multiplier = self.bubble_multiplier.clamp(0.0, 1.0);
        self.bubble_angle_old = self.bubble_angle;
        self.bubble_angle =
            10.0 * (0.5 * f64::from(self.tick_count)).sin() as f32 * self.bubble_multiplier;
    }
    /// `AbstractBoat.tickBubbleColumn`'s countdown on the server: the bubble
    /// time drops while the boat stays over a column and resets when it leaves.
    /// `Some(down)` on the tick it runs out: the boat is thrown up (`false`) or
    /// pulled down (`true`).
    pub(super) fn tick_bubble_countdown(&mut self) -> Option<bool> {
        if !self.above_bubble_column {
            self.bubble_time = 0;
        }
        if self.bubble_time <= 0 {
            return None;
        }
        self.bubble_time -= 1;
        let diff = BUBBLE_TIME - self.bubble_time - 1;
        let fired = (diff > 0 && self.bubble_time == 0).then_some(self.bubble_down);
        self.above_bubble_column = false;
        fired
    }
    /// `getRowingTime`.
    pub fn rowing_time(&self, side: usize, a: f32) -> f32 {
        if self.paddling[side] {
            self.paddles[side] - PI / 8.0 + a * PI / 8.0
        } else {
            0.0
        }
    }
}
/// Item id -> (boat entity id, wood).
impl Boat {
    /// The vanilla entity type id (`oak_boat`, `bamboo_chest_raft`), also
    /// the item it drops.
    pub(super) fn entity_type(&self) -> String {
        let chest = if self.chest { "chest_" } else { "" };
        if &*self.wood == "bamboo" {
            format!("minecraft:bamboo_{chest}raft")
        } else {
            format!("minecraft:{}_{chest}boat", self.wood)
        }
    }
}
/// The wood of a boat or chest boat item or entity type, and whether it
/// carries a chest.
pub(super) fn any_boat_wood(item: &str) -> Option<(&str, bool)> {
    let name = item.strip_prefix("minecraft:")?;
    if let Some(wood) = name
        .strip_suffix("_chest_boat")
        .or_else(|| name.strip_suffix("_chest_raft"))
    {
        return Some((wood, true));
    }
    boat_wood(item).map(|w| (w, false))
}
pub(super) fn boat_wood(item: &str) -> Option<&str> {
    let name = item.strip_prefix("minecraft:")?;
    name.strip_suffix("_boat")
        .or_else(|| name.strip_suffix("_raft"))
        .filter(|w| !w.ends_with("_chest"))
}
pub(super) struct BoatModels {
    pub models: HashMap<String, Part>,
    pub chest_models: HashMap<String, Part>,
    /// `ModelLayers.BOAT_WATER_PATCH`, shared by every boat (not rafts).
    pub water_patch: Option<Part>,
}
impl BoatModels {
    /// Whether a boat of this wood (with or without a chest) can be drawn.
    pub fn has(&self, wood: &str, chest: bool) -> bool {
        if chest {
            self.chest_models.contains_key(wood)
        } else {
            self.models.contains_key(wood)
        }
    }
    pub fn load(document: Option<&horizon_client::mobs::EntityModelDocument>) -> Self {
        let mut models = HashMap::new();
        let mut chest_models = HashMap::new();
        if let Some(document) = document {
            for key in document.keys() {
                if let Some(wood) = key
                    .strip_prefix("minecraft:boat/")
                    .and_then(|k| k.strip_suffix("#main"))
                    && let Some(part) = document.model(key)
                {
                    models.insert(wood.to_owned(), part);
                }
                if let Some(wood) = key
                    .strip_prefix("minecraft:chest_boat/")
                    .and_then(|k| k.strip_suffix("#main"))
                    && let Some(part) = document.model(key)
                {
                    chest_models.insert(wood.to_owned(), part);
                }
            }
        }
        Self {
            models,
            chest_models,
            water_patch: document.and_then(|d| d.model("minecraft:boat#water_patch")),
        }
    }
}
impl Game<'_> {
    fn fluid_height(&self, x: i32, y: i32, z: i32) -> Option<(f32, bool)> {
        self.fluid_height_of(x, y, z, Some(Fluid::Water))
    }
    /// The horizontal water plane around the camera for the water mirror: the median
    /// standing-water surface of a grid of columns 48 blocks around, each scanned down from
    /// the camera to its ground. A lake or the sea has one plane; the mirror shows only the
    /// water on it. With no water in reach the last plane found stays, so the mirror never
    /// switches off as the camera moves.
    pub(super) fn water_plane(&self, camera: glam::DVec3) -> Option<f32> {
        const RADIUS: i32 = 48;
        const STEP: usize = 4;
        const DEPTH: i32 = 96;
        let (cx, cz) = (camera.x.floor() as i32, camera.z.floor() as i32);
        let top = camera.y.floor() as i32 + 2;
        // Scanned again once the camera moves 8 blocks, or every 5 seconds: a scan reads up
        // to 60 000 blocks on the game thread, and the plane rarely changes.
        let scan = (
            [cx >> 3, top >> 3, cz >> 3],
            self.world.game_time.div_euclid(100),
        );
        if self.water_plane_scan.replace(Some(scan)) == Some(scan) {
            return self.water_plane_memo.get();
        }
        let mut heights = Vec::with_capacity(625);
        for x in (cx - RADIUS..=cx + RADIUS).step_by(STEP) {
            for z in (cz - RADIUS..=cz + RADIUS).step_by(STEP) {
                // The top of the column's water: the highest standing water under the
                // camera (a full block's water continues above it).
                for y in (top - DEPTH..=top).rev() {
                    if let Some((h, source)) = self.fluid_height(x, y, z) {
                        if source && h < 1.0 {
                            heights.push(y as f32 + h);
                        }
                        break;
                    }
                    if self.res.facts().get(self.world.block(x, y, z)).solid_render {
                        break;
                    }
                }
            }
        }
        if heights.is_empty() {
            return self.water_plane_memo.get();
        }
        heights.sort_by(f32::total_cmp);
        let found = heights[heights.len() / 2];
        // The plane in use stays while a quarter of the columns are still on it, so two
        // lakes at different heights do not make the mirror flicker between them.
        let plane = match self.water_plane_memo.get() {
            Some(last)
                if 4 * heights.iter().filter(|h| (*h - last).abs() < 0.05).count()
                    >= heights.len() =>
            {
                last
            }
            _ => found,
        };
        self.water_plane_memo.set(Some(plane));
        Some(plane)
    }
    /// `FluidState.getHeight` and whether it is a source, for `fluid` or,
    /// with `None`, any fluid (`ClipContext.Fluid.ANY`).
    pub(super) fn fluid_height_of(
        &self,
        x: i32,
        y: i32,
        z: i32,
        fluid: Option<Fluid>,
    ) -> Option<(f32, bool)> {
        let f = self.res.facts().get(self.world.block(x, y, z));
        if f.fluid == Fluid::None || fluid.is_some_and(|want| f.fluid != want) {
            return None;
        }
        let above = self.res.facts().get(self.world.block(x, y + 1, z));
        // FluidState.getHeight: 1 under the same fluid, else amount / 9.
        let height = if above.fluid == f.fluid {
            1.0
        } else {
            f32::from(f.fluid_level) / 9.0
        };
        Some((height, f.fluid_level == 8 && !f.fluid_falling))
    }
    /// `getWaterLevelAbove`: the top of the water column the hull is in.
    fn water_level_above(&self, b: &Boat) -> f64 {
        let bb = b.bounds();
        let (lo, hi) = (bb.min_position(), bb.max_position());
        let (y0, y1) = (hi.y.floor() as i32, hi.y.ceil() as i32);
        'rows: for y in y0..y1 {
            let mut height = 0.0f32;
            for x in lo.x.floor() as i32..hi.x.ceil() as i32 {
                for z in lo.z.floor() as i32..hi.z.ceil() as i32 {
                    if let Some((h, _)) = self.fluid_height(x, y, z) {
                        height = height.max(h);
                    }
                    if height >= 1.0 {
                        continue 'rows;
                    }
                }
            }
            if height < 1.0 {
                return f64::from(y) + f64::from(height);
            }
        }
        f64::from(y1 + 1)
    }
    fn boat_status(&self, b: &mut Boat) -> Status {
        let bb = b.bounds();
        let (lo, hi) = (bb.min_position(), bb.max_position());
        // isUnderwater
        let max_y = hi.y + 0.001;
        let mut under = false;
        for x in lo.x.floor() as i32..hi.x.ceil() as i32 {
            for y in hi.y.floor() as i32..max_y.ceil() as i32 {
                for z in lo.z.floor() as i32..hi.z.ceil() as i32 {
                    if let Some((h, source)) = self.fluid_height(x, y, z)
                        && max_y < f64::from(y) + f64::from(h)
                    {
                        if !source {
                            b.water_level = hi.y;
                            return Status::UnderFlowingWater;
                        }
                        under = true;
                    }
                }
            }
        }
        if under {
            b.water_level = hi.y;
            return Status::UnderWater;
        }
        // checkInWater
        let mut in_water = false;
        b.water_level = -f64::MAX;
        for x in lo.x.floor() as i32..hi.x.ceil() as i32 {
            for y in lo.y.floor() as i32..(lo.y + 0.001).ceil() as i32 {
                for z in lo.z.floor() as i32..hi.z.ceil() as i32 {
                    if let Some((h, _)) = self.fluid_height(x, y, z) {
                        let level = f64::from(y) + f64::from(h);
                        b.water_level = b.water_level.max(level);
                        in_water |= lo.y < level;
                    }
                }
            }
        }
        if in_water {
            return Status::InWater;
        }
        let friction = self.boat_ground_friction(bb);
        if friction > 0.0 {
            b.land_friction = friction;
            Status::OnLand
        } else {
            Status::InAir
        }
    }
    /// `AbstractBoat.getGroundFriction`: average friction of the blocks whose
    /// collision shape meets a thin slab just under the hull (lily pads skipped).
    fn boat_ground_friction(&self, bb: Aabb) -> f32 {
        let (lo, hi) = (bb.min_position(), bb.max_position());
        let ground = Aabb::new(
            Vec3::new(lo.x, lo.y - 0.001, lo.z),
            Vec3::new(hi.x, lo.y, hi.z),
        );
        let (g_lo, g_hi) = (ground.min_position(), ground.max_position());
        let (x0, x1, y0, y1, z0, z1) = (
            g_lo.x.floor() as i32 - 1,
            g_hi.x.ceil() as i32 + 1,
            g_lo.y.floor() as i32 - 1,
            g_hi.y.ceil() as i32 + 1,
            g_lo.z.floor() as i32 - 1,
            g_hi.z.ceil() as i32 + 1,
        );
        let slab = VoxelShape::from_box(ground).unwrap_or_else(|_| VoxelShape::empty());
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        let (mut friction, mut count) = (0.0f32, 0);
        for x in x0..x1 {
            for z in z0..z1 {
                let edges = i32::from(x == x0 || x == x1 - 1) + i32::from(z == z0 || z == z1 - 1);
                if edges == 2 {
                    continue;
                }
                for y in y0..y1 {
                    if edges == 1 && (y == y0 || y == y1 - 1) {
                        continue;
                    }
                    let state = self.world.block(x, y, z);
                    let f = self.res.facts().get(state);
                    if f.name.as_ref() == "minecraft:air" || f.has_class("LilyPadBlock") {
                        continue;
                    }
                    let Ok(shape) = self.collision.collision_shape_with_context(
                        &mut scene,
                        state,
                        BlockPos::new(x, y, z),
                        &CollisionContext::empty(false),
                    ) else {
                        continue;
                    };
                    let world_shape =
                        shape.moved(Vec3::new(f64::from(x), f64::from(y), f64::from(z)));
                    if world_shape
                        .join_is_not_empty(&slab, ShapeOp::AND)
                        .unwrap_or(false)
                    {
                        friction += f.friction;
                        count += 1;
                    }
                }
            }
        }
        if count > 0 {
            friction / count as f32
        } else {
            0.0
        }
    }
    /// `AbstractBoat.tick` for one boat (`controlled` when the player rides it).
    pub(super) fn tick_boat(&mut self, mut b: Boat, controlled: bool) -> Boat {
        b.previous = b.position;
        b.previous_yaw = b.yaw;
        // Entity.baseTick ends with Leashable.tickLeash, on a server only
        // (a ridden boat's lead ticks in `tick_boats`).
        if !controlled && self.remote.is_none() {
            self.tick_boat_leash(&mut b);
        }
        b.old_status = b.status;
        b.status = self.boat_status(&mut b);
        b.out_of_control = if matches!(b.status, Status::UnderWater | Status::UnderFlowingWater) {
            b.out_of_control + 1.0
        } else {
            0.0
        };
        b.hurt_time = (b.hurt_time - 1).max(0);
        b.damage = (b.damage - 1.0).max(0.0);
        if !controlled {
            b.paddling = [false; 2];
            b.input = [false; 4];
        }
        // floatBoat
        let mut vspeed = -0.04;
        let mut buoyancy = 0.0;
        let inv_friction: f32;
        let obstacles = self.boat_move_obstacles(&b);
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        if b.old_status == Status::InAir && b.status != Status::InAir && b.status != Status::OnLand
        {
            b.water_level = b.position.y;
            let target = self.water_level_above(&b) - HEIGHT + 0.101;
            let moved = Aabb::new(
                Vec3::new(
                    b.bounds().min_position().x,
                    target,
                    b.bounds().min_position().z,
                ),
                Vec3::new(
                    b.bounds().max_position().x,
                    target + HEIGHT,
                    b.bounds().max_position().z,
                ),
            );
            if scene.unobstructed(moved).unwrap_or(false) {
                b.position.y = target;
                b.velocity.y = 0.0;
            }
            b.status = Status::InWater;
        } else {
            match b.status {
                Status::InWater => {
                    buoyancy = (b.water_level - b.position.y) / HEIGHT;
                    inv_friction = 0.9;
                }
                Status::UnderFlowingWater => {
                    vspeed = -7.0e-4;
                    inv_friction = 0.9;
                }
                Status::UnderWater => {
                    buoyancy = 0.01;
                    inv_friction = 0.45;
                }
                Status::InAir => inv_friction = 0.9,
                Status::OnLand => {
                    inv_friction = b.land_friction;
                    if controlled {
                        b.land_friction /= 2.0;
                    }
                }
            }
            let f = f64::from(inv_friction);
            b.velocity = Vec3::new(b.velocity.x * f, b.velocity.y + vspeed, b.velocity.z * f);
            b.delta_rotation *= inv_friction;
            if buoyancy > 0.0 {
                b.velocity.y = (b.velocity.y + buoyancy * (0.04 / 0.65)) * 0.75;
            }
        }
        // controlBoat
        if controlled {
            let [left, right, up, down] = b.input;
            let mut acceleration = 0.0f32;
            if left {
                b.delta_rotation -= 1.0;
            }
            if right {
                b.delta_rotation += 1.0;
            }
            if right != left && !up && !down {
                acceleration += 0.005;
            }
            b.yaw += b.delta_rotation;
            if up {
                acceleration += 0.04;
            }
            if down {
                acceleration -= 0.005;
            }
            let r = b.yaw.to_radians();
            b.velocity.x += f64::from((-r).sin() * acceleration);
            b.velocity.z += f64::from(r.cos() * acceleration);
            b.paddling = [right && !left || up, left && !right || up];
        }
        // Entity.move: collisions zero the blocked components.
        let mut scene = EntityCollisions {
            blocks: scene,
            entities: &obstacles,
            border: None,
        };
        if let Ok(moved) = scene.resolve(b.bounds(), b.velocity, 0.0, b.on_ground) {
            b.position = Vec3::new(
                b.position.x + moved.x,
                b.position.y + moved.y,
                b.position.z + moved.z,
            );
            b.on_ground = b.velocity.y < 0.0 && moved.y > b.velocity.y;
            if (moved.x - b.velocity.x).abs() > 1.0e-7 {
                b.velocity.x = 0.0;
            }
            if (moved.z - b.velocity.z).abs() > 1.0e-7 {
                b.velocity.z = 0.0;
            }
            if (moved.y - b.velocity.y).abs() > 1.0e-7 {
                b.velocity.y = 0.0;
            }
        }
        // Paddle sounds at the top of each stroke.
        for i in 0..2 {
            if b.paddling[i] {
                let p = b.paddles[i] % (2.0 * PI);
                if p <= PI / 4.0 && (b.paddles[i] + PI / 8.0) % (2.0 * PI) >= PI / 4.0 {
                    let sound = match b.status {
                        Status::InWater | Status::UnderWater | Status::UnderFlowingWater => {
                            Some("minecraft:entity.boat.paddle_water")
                        }
                        Status::OnLand => Some("minecraft:entity.boat.paddle_land"),
                        Status::InAir => None,
                    };
                    if let Some(sound) = sound {
                        let v = view_vector(b.yaw, 0.0);
                        let (dx, dz) = if i == 1 { (-v[2], v[0]) } else { (v[2], -v[0]) };
                        let pitch = 0.8 + 0.4 * self.world.random.next_float();
                        self.audio.play(
                            sound,
                            Some([b.position.x + dx, b.position.y, b.position.z + dz]),
                            1.0,
                            pitch,
                        );
                    }
                }
                b.paddles[i] += PI / 8.0;
            } else {
                b.paddles[i] = 0.0;
            }
        }
        b
    }
    /// Every boat but the ridden one (which ticks with its rider).
    pub(super) fn tick_boats(&mut self) {
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        // A server player's boat is driven by its client (lan_vehicles.rs).
        let driven = self.guest_vehicles();
        for i in 0..self.boats.len() {
            let p = self.boats[i].position;
            if Some(self.boats[i].id) == self.riding
                || driven.contains(&self.boats[i].id)
                || !ticking(p.x.floor() as i32, p.z.floor() as i32)
            {
                // Its rider's client moves it, but the lead still ticks.
                if ticking(p.x.floor() as i32, p.z.floor() as i32) && self.remote.is_none() {
                    let mut b = self.boats[i].clone();
                    self.tick_boat_leash(&mut b);
                    self.boats[i] = b;
                }
                continue;
            }
            let b = self.boats[i].clone();
            self.boats[i] = self.tick_boat(b, false);
        }
        // AbstractBoat.tick's applyEffectsFromBlocks, on the server for every
        // boat, the client-driven ones too.
        let mut i = 0;
        while i < self.boats.len() {
            let p = self.boats[i].position;
            let skip = Some(self.boats[i].id) == self.riding
                || !ticking(p.x.floor() as i32, p.z.floor() as i32);
            if skip || !self.boat_lava_tick(i) {
                i += 1;
            }
        }
        // The rest of AbstractBoat.tick on the server, for every boat: the
        // passengers of one under water too long get out, and it picks up or
        // pushes what it touches.
        for i in 0..self.boats.len() {
            let p = self.boats[i].position;
            if !ticking(p.x.floor() as i32, p.z.floor() as i32) {
                continue;
            }
            if self.boats[i].out_of_control >= 60.0 {
                self.eject_boat_mobs(i, false);
            }
            self.boat_bubble_tick(i);
            self.boat_contacts(i);
        }
    }
    /// `Entity.canSimulateMovement`: for a boat (`AUTHORITATIVE_SIDE`) a
    /// server leaves the boat its player drives to that player's client, and a
    /// client moves only the boat its player drives.
    fn boat_can_simulate_movement(&self, id: u32) -> bool {
        if self.remote.is_some() {
            self.riding == Some(id)
        } else {
            !self.guest_vehicles().contains(&id)
        }
    }
    /// `AbstractBoat.tick`: `status = getStatus()` for a boat this window
    /// does not tick (`tick_boat` sets it for the others): a client's boats
    /// other than the one it drives, and a server's boats its players drive.
    pub(super) fn refresh_boat_status(&mut self, i: usize) {
        let b = &self.boats[i];
        let mut probe = Boat::new(b.id, b.wood.clone(), b.position, b.yaw);
        let status = self.boat_status(&mut probe);
        self.boats[i].status = status;
    }
    /// `BubbleColumnBlock.entityInside`: the bubble columns boat `i` touches
    /// (its box deflated by 1e-5 as `Entity.checkInsideBlocks` does), in x, y,
    /// z order.
    pub(super) fn boat_bubble_columns(&self, i: usize) -> Vec<BubbleColumn> {
        let b = self.boats[i].bounds().deflate(Vec3::new(
            f64::from(1.0e-5f32),
            f64::from(1.0e-5f32),
            f64::from(1.0e-5f32),
        ));
        let (lo, hi) = (b.min_position(), b.max_position());
        let mut columns = Vec::new();
        for x in lo.x.floor() as i32..=(hi.x.ceil() as i32 - 1) {
            for y in lo.y.floor() as i32..=(hi.y.ceil() as i32 - 1) {
                for z in lo.z.floor() as i32..=(hi.z.ceil() as i32 - 1) {
                    if let Some(column) = bubble_column_at(
                        &self.world,
                        self.pack.assets.registry,
                        self.res.facts(),
                        self.collision,
                        BlockPos::new(x, y, z),
                    ) {
                        columns.push(column);
                    }
                }
            }
        }
        columns
    }
    /// `Entity.handleOnInsideBubbleColumn`: a column under more water lifts
    /// boat `i` up to 0.7 or drags it down to -0.3 (`AbstractBoat` does not
    /// override `onInsideBubbleColumn`), applied by the column's own push below
    /// the surface.
    pub(super) fn boat_inside_bubble_column(&mut self, i: usize, column: BubbleColumn) {
        if !self.boat_can_simulate_movement(self.boats[i].id) {
            return;
        }
        column.push(&mut self.boats[i].velocity);
    }
    /// `AbstractBoat.onAboveBubbleColumn`: whether the splash comes up, one
    /// time in a hundred for a boat that is not under water.
    pub(super) fn boat_splash_roll(&mut self, i: usize) -> bool {
        !self.boats[i].under_water() && self.world.random.next_int_bound(100).unwrap_or(1) == 0
    }
    /// `AbstractBoat.onAboveBubbleColumn`: the client's half after the roll,
    /// the splash sound and particle at the boat.
    pub(super) fn boat_bubble_splash(&mut self, i: usize) {
        use horizon_client::particles::{ParticleOptions, ParticleType};
        let p = self.boats[i].position;
        let pitch = 0.8 + 0.4 * self.world.random.next_float();
        self.audio.play(
            "minecraft:entity.generic.splash",
            Some([p.x, p.y, p.z]),
            1.0,
            pitch,
        );
        let x = p.x + f64::from(self.world.random.next_float());
        let z = p.z + f64::from(self.world.random.next_float());
        self.particles.add(
            &ParticleOptions::simple(ParticleType::Splash),
            [x, p.y + 0.7, z],
            [0.0; 3],
        );
    }
    /// `AbstractBoat.onAboveBubbleColumn`: on the server (this window
    /// simulates the boat) the column is remembered, the countdown starts and
    /// the splash's game event goes to the listeners (its sound and particle
    /// are the client's).
    fn boat_above_bubble_column(&mut self, i: usize, drag_down: bool) {
        let b = &mut self.boats[i];
        b.above_bubble_column = true;
        b.bubble_down = drag_down;
        if b.bubble_time == 0 {
            b.bubble_time = BUBBLE_TIME;
        }
        if !self.boat_splash_roll(i) {
            return;
        }
        // A window that is its own client hears and sees the splash too.
        if self.options.execution_role.is_client() {
            self.boat_bubble_splash(i);
        }
        let (id, p) = (self.boats[i].id, self.boats[i].position);
        // Entity.gameEvent(SPLASH, getControllingPassenger()): the window's
        // own player when it drives the boat.
        let ctx = if self.riding == Some(id) && self.player.is_some() {
            self.player_context(None)
        } else {
            self.who_context(None, None)
        };
        self.post_game_event("minecraft:splash", [p.x, p.y, p.z], ctx);
    }
    /// `AbstractBoat.handleBubbleColumnEffect`: an up column throws the boat
    /// up at 0.6, a down column pulls it down by 0.7.
    pub(super) fn handle_bubble_column_effect(&mut self, i: usize, drag_down: bool) {
        if !self.boat_can_simulate_movement(self.boats[i].id) {
            return;
        }
        let v = &mut self.boats[i].velocity;
        if drag_down {
            // movement.add(0.0, -0.7, 0.0)
            v.y += -0.7;
        } else {
            v.y = 0.6;
        }
    }
    /// `Entity.ejectPassengers`: for a down column the mobs get out here, this
    /// window's player gets out, and a server's player is asked to
    /// (`guest_vehicle_tick` lets it out through its client).
    fn eject_boat_riders(&mut self, i: usize) {
        self.eject_boat_mobs(i, false);
        let id = self.boats[i].id;
        if self.riding == Some(id) && self.player.is_some() {
            self.dismount(i);
        } else if self.guest_rides(id) {
            self.boats[i].eject_riders = true;
        }
    }
    /// `AbstractBoat.tickBubbleColumn`: on the server, a boat that stays over
    /// a column for `BUBBLE_TIME` ticks is thrown up or pulled down (a down
    /// column lets the riders out first), and every client is told.
    fn tick_boat_bubble_column(&mut self, i: usize) {
        if let Some(down) = self.boats[i].tick_bubble_countdown() {
            let (id, p) = (self.boats[i].id, self.boats[i].position);
            let event = if down {
                BUBBLE_DOWN_EVENT
            } else {
                BUBBLE_UP_EVENT
            };
            if down {
                self.eject_boat_riders(i);
            }
            self.lan_entity_event(id as i32, event, [p.x, p.y, p.z]);
            self.handle_bubble_column_effect(i, down);
        }
    }
    /// `AbstractBoat.tick`: the end of it on the server for boat `i`, the two
    /// `applyEffectsFromBlocks` that reach bubble columns (the lava is
    /// `boat_lava_tick`'s), then `tickBubbleColumn`; and the wobble when this
    /// window is its own client.
    fn boat_bubble_tick(&mut self, i: usize) {
        let id = self.boats[i].id;
        // The boat a server's player drives is not ticked here: its status is.
        if self.guest_vehicles().contains(&id) {
            self.refresh_boat_status(i);
        }
        // An ejection nobody took (its player left the boat first) is void.
        if !self.guest_rides(id) {
            self.boats[i].eject_riders = false;
        }
        let touches = self.boat_bubble_columns(i);
        // AbstractBoat.tick calls applyEffectsFromBlocks twice.
        for _ in 0..2 {
            for &column in &touches {
                if column.surface {
                    self.boat_above_bubble_column(i, column.drag);
                } else {
                    self.boat_inside_bubble_column(i, column);
                }
            }
        }
        self.tick_boat_bubble_column(i);
        if self.options.execution_role.is_client() {
            self.boats[i].tick_bubble_wobble();
        }
    }
    /// `LavaFluid.entityInside` (`lavaHurt`) from the two
    /// `applyEffectsFromBlocks` calls in `AbstractBoat.tick`: twice 40
    /// damage, so lava breaks a boat on its first tick inside, and the fire
    /// `lavaIgnite` lights never gets to burn it. True when the boat broke.
    fn boat_lava_tick(&mut self, i: usize) -> bool {
        if !self.in_lava(self.boats[i].bounds()) {
            return false;
        }
        for _ in 0..2 {
            let p = self.boats[i].position;
            let broke = self.hurt_boat(i, 4.0, false);
            self.lava_hurt_sound([p.x, p.y, p.z]);
            if broke {
                return true;
            }
        }
        false
    }
    /// `Entity.lavaHurt`'s sound once the hurt went through.
    pub(super) fn lava_hurt_sound(&mut self, at: [f64; 3]) {
        let pitch = 2.0 + self.world.random.next_float() * 0.4;
        self.audio
            .play("minecraft:entity.generic.burn", Some(at), 0.4, pitch);
    }
    /// Whether a vehicle's box, deflated by 1e-5, reaches into lava below its
    /// surface (`Entity.collidedWithFluid`).
    pub(super) fn in_lava(&self, bb: Aabb) -> bool {
        let (lo, hi) = (bb.min_position(), bb.max_position());
        let e = 1.0e-5;
        for x in (lo.x + e).floor() as i32..=(hi.x - e).floor() as i32 {
            for y in (lo.y + e).floor() as i32..=(hi.y - e).floor() as i32 {
                for z in (lo.z + e).floor() as i32..=(hi.z - e).floor() as i32 {
                    if let Some((height, _)) = self.fluid_height_of(x, y, z, Some(Fluid::Lava))
                        && lo.y + e < f64::from(y) + f64::from(height)
                    {
                        return true;
                    }
                }
            }
        }
        false
    }
    /// `AbstractBoat.modifyPassengerFluidInteractionBox`: the ridden boat's box
    /// and whether it is underwater, for `Entity.getFluidInteractionBox`. A boat
    /// keeps only the part of the passenger's box above its own top, so a boat
    /// bobbing on water never sinks the passenger's eyes or body.
    pub(super) fn riding_boat_fluid_box(&self) -> Option<RiddenBoat> {
        self.boats
            .iter()
            .find(|b| Some(b.id) == self.riding)
            .map(|b| RiddenBoat {
                under_water: b.under_water(),
                max_y: b.bounds().max_position().y,
            })
    }
    /// The player's tick while riding: steer, tick the boat, sit in it.
    pub(super) fn tick_riding(&mut self, keys: Keys) -> horizon_client::Result<()> {
        if self.tick_cushion_riding(keys) {
            return Ok(());
        }
        let Some(i) = self.boats.iter().position(|b| Some(b.id) == self.riding) else {
            if self.tick_cart_riding(keys) || self.tick_mount_riding(keys) {
                return Ok(());
            }
            self.riding = None;
            return Ok(());
        };
        if keys.shift || self.boats[i].out_of_control >= 60.0 {
            self.dismount(i);
            return Ok(());
        }
        // The boat steers with the input `LocalPlayer.rideTick` latched on the
        // previous tick; this tick's keys are latched after it ticked.
        let mut b = self.tick_boat(self.boats[i].clone(), true);
        b.input = [keys.left, keys.right, keys.forward, keys.backward];
        // positionRider: the seat, turned with the boat; clampRotation to 105 degrees.
        self.yaw += b.delta_rotation;
        let delta = (self.yaw - b.yaw + 180.0).rem_euclid(360.0) - 180.0;
        self.yaw += delta.clamp(-105.0, 105.0) - delta;
        self.boats[i] = b;
        let seat = self.player_boat_seat(i);
        if let Some(p) = self.player.as_mut() {
            self.previous_position = p.position;
            p.position = seat;
            p.velocity = Vec3::ZERO;
            p.fall_distance = 0.0;
        }
        if self.boat_lava_tick(i) {
            self.riding = None;
        }
        Ok(())
    }
    /// `getDismountLocationForPassenger`: in front of the player if there is
    /// floor there, else on top of the boat.
    pub(super) fn dismount(&mut self, i: usize) {
        self.riding = None;
        let target = self.boat_dismount_target(i, 0.6, self.yaw);
        if let Some(p) = self.player.as_mut() {
            p.position = target;
            p.velocity = Vec3::ZERO;
        }
    }
    /// `getDismountLocationForPassenger` for a passenger `width` wide
    /// facing `yaw`.
    pub(super) fn boat_dismount_target(&self, i: usize, width: f64, yaw: f32) -> Vec3 {
        let b = &self.boats[i];
        let r = yaw.to_radians();
        let distance = (WIDTH * std::f64::consts::SQRT_2 + width + 1.0e-5) / 2.0;
        let (s, c) = (-(r.sin()) as f64, r.cos() as f64);
        let m = s.abs().max(c.abs());
        let (x, z) = (
            b.position.x + s * distance / m,
            b.position.z + c * distance / m,
        );
        let top = b.bounds().max_position().y;
        let (bx, by, bz) = (x.floor() as i32, top.floor() as i32, z.floor() as i32);
        let below = self.res.facts().get(self.world.block(bx, by - 1, bz));
        let here = self.res.facts().get(self.world.block(bx, by, bz));
        if below.fluid != Fluid::Water
            && !here.full_collision
            && (below.full_collision || below.legacy_solid)
        {
            Vec3::new(x, f64::from(by), z)
        } else {
            Vec3::new(b.position.x, top, b.position.z)
        }
    }
    /// `BoatItem.use`: aim at a block or the water surface.
    pub(super) fn place_boat(&mut self, stack: &ItemStack) -> bool {
        let Some(boat) = self.boat_to_place(stack) else {
            return false;
        };
        self.next_entity += 1;
        let key = (
            boat.position.x.floor() as i32 >> 4,
            boat.position.z.floor() as i32 >> 4,
        );
        self.world.modified.insert(key);
        self.boats.push(boat);
        if !self.creative {
            self.consume_held();
        }
        self.swing_arm();
        true
    }
    /// The boat `BoatItem.use` would place for `stack` (its SUCCESS), or
    /// None when it aims at nothing (PASS) or the spot is taken (FAIL).
    pub(super) fn boat_to_place(&self, stack: &ItemStack) -> Option<Boat> {
        let (wood, chest) = any_boat_wood(&stack.item)?;
        if !self.boat_models.has(wood, chest) {
            return None;
        }
        let (eye, dir) = self.aim()?;
        let reach = self.reach();
        // ClipContext.Fluid.ANY: the first fluid surface (water or lava) or
        // block along the ray.
        let block = self.pick().map(|h| {
            (
                h.point,
                (0..3)
                    .map(|a| (h.point[a] - eye[a]).powi(2))
                    .sum::<f64>()
                    .sqrt(),
            )
        });
        let mut fluid = None;
        let mut t = 0.0;
        while t < reach {
            let p: [f64; 3] = std::array::from_fn(|a| eye[a] + dir[a] * t);
            let (x, y, z) = (
                p[0].floor() as i32,
                p[1].floor() as i32,
                p[2].floor() as i32,
            );
            if let Some((h, _)) = self.fluid_height_of(x, y, z, None)
                && p[1] <= f64::from(y) + f64::from(h)
            {
                fluid = Some(([p[0], f64::from(y) + f64::from(h), p[2]], t));
                break;
            }
            t += 0.05;
        }
        let hit = match (block, fluid) {
            (Some(b), Some(w)) => {
                if w.1 < b.1 {
                    w.0
                } else {
                    b.0
                }
            }
            (Some(b), None) => b.0,
            (None, Some(w)) => w.0,
            (None, None) => return None,
        };
        let mut boat = Boat::new(
            self.next_entity + 1,
            Arc::from(wood),
            Vec3::new(hit[0], hit[1], hit[2]),
            self.yaw,
        );
        if chest {
            boat = boat.with_chest();
        }
        let bb = boat.bounds();
        let (lo, hi) = (bb.min_position(), bb.max_position());
        let e = 1.0e-7;
        let inner = Aabb::new(
            Vec3::new(lo.x + e, lo.y + e, lo.z + e),
            Vec3::new(hi.x - e, hi.y - e, hi.z - e),
        );
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        if !scene.unobstructed(inner).unwrap_or(false) || self.boat_entity_collision(bb) {
            return None;
        }
        Some(boat)
    }
    pub(super) fn boat_hit(&self) -> Option<(usize, f64)> {
        let (eye, dir) = self.aim()?;
        let reach: f64 = self.entity_reach;
        let mut best: Option<(usize, f64)> = None;
        for (i, b) in self.boats.iter().enumerate() {
            if Some(b.id) == self.riding {
                continue;
            }
            let bb = b.bounds();
            let (lo, hi) = (bb.min_position(), bb.max_position());
            let (lo, hi) = ([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]);
            let (mut t0, mut t1) = (0.0f64, reach);
            let mut hit = true;
            for a in 0..3 {
                if dir[a].abs() < 1e-9 {
                    hit &= eye[a] >= lo[a] && eye[a] <= hi[a];
                    continue;
                }
                let (mut ta, mut tb) = ((lo[a] - eye[a]) / dir[a], (hi[a] - eye[a]) / dir[a]);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
            }
            if hit && t0 <= t1 && best.is_none_or(|b| t0 < b.1) {
                best = Some((i, t0));
            }
        }
        best
    }
    /// `AbstractBoat.interact`: board the boat under the crosshair, or the one
    /// a guest's interact packet named.
    pub(super) fn interact_boat(&mut self, target: Option<InteractTarget>) -> bool {
        let Some((i, d)) = (match target {
            Some(t) => Self::named_entity(&self.boats, t.id, |b| b.id).map(|i| (i, 0.0)),
            None => self.boat_hit(),
        }) else {
            return false;
        };
        let sneaking = self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft);
        // Only a client's own pick is stopped by a block: the range check passed.
        if target.is_none() && !self.before_block(d) {
            return false;
        }
        // AbstractBoat.interact starts with Entity.interact: the lead.
        if self.interact_boat_leash(i) {
            self.swing_arm();
            return true;
        }
        // ChestBoat.interact: sneaking (or a full seat) opens the chest.
        if self.boats[i].chest && (sneaking || self.riding.is_some() || !self.boat_has_room(i)) {
            self.open_chest_boat_container(i);
            self.swing_arm();
            return true;
        }
        if sneaking
            || self.boats[i].out_of_control >= 60.0
            || self.riding.is_some()
            || self.ridden_by_guest(self.boats[i].id)
            || !self.boat_has_room(i)
        {
            return false;
        }
        self.riding = Some(self.boats[i].id);
        self.yaw = self.boats[i].yaw;
        // ServerPlayer.startRiding.
        self.trigger_player("minecraft:started_riding");
        true
    }
    /// `ContainerEntity.interactWithContainerVehicle` (`AbstractChestBoat`):
    /// the chest's 27 slots go under a placeholder key that the menu edits; a
    /// server hands them to the player as a `ChestMenu` (`as_guest` ->
    /// `lan_open_container`). `AbstractChestBoat.interact` fires
    /// `CONTAINER_OPEN` and angers the piglins that see it.
    pub(super) fn open_chest_boat_container(&mut self, i: usize) {
        let b = &self.boats[i];
        let key = (i32::MAX - 15, super::minecarts::CART_KEY_Y, b.id as i32);
        let at = [b.position.x, b.position.y, b.position.z];
        let items = b.items.clone();
        self.world
            .block_entities
            .insert(key, horizon_client::containers::BlockEntity::Chest(items));
        self.container = Some(key);
        self.screen = super::Screen::Inventory;
        self.capture(false);
        self.right = false;
        let ctx = self.player_context(None);
        self.post_game_event("minecraft:container_open", at, ctx);
        self.anger_piglins(true);
    }
    /// `AbstractChestBoat.openCustomInventoryScreen` (`HasCustomInventoryScreen`),
    /// the inventory key while riding a chest boat: its chest opens, not the
    /// player's inventory.
    pub(super) fn open_ridden_chest_boat(&mut self) -> bool {
        let Some(id) = self.riding else {
            return false;
        };
        let Some(i) = self.boats.iter().position(|b| b.id == id) else {
            return false;
        };
        if !self.boats[i].chest {
            return false;
        }
        self.open_chest_boat_container(i);
        true
    }
    /// Whether the player rides a chest boat: `HasCustomInventoryScreen`, so
    /// the inventory key sends `ServerboundPlayerCommandPacket.Action.OPEN_INVENTORY`
    /// (`ServerGamePacketListenerImpl.handlePlayerCommand` opens its screen).
    pub(super) fn riding_chest_boat(&self) -> bool {
        self.riding
            .and_then(|id| self.boats.iter().find(|b| b.id == id))
            .is_some_and(|b| b.chest)
    }
    /// `VehicleEntity.hurtServer` from the player's attack.
    pub(super) fn attack_boat(&mut self) -> bool {
        let Some((i, d)) = self.boat_hit() else {
            return false;
        };
        if !self.before_block(d) || self.mob_hit().is_some_and(|m| m.1 < d) {
            return false;
        }
        let area = self.boats[i].bounds();
        // Player.attack's full damage against the boat's entity type.
        let (kind, id) = (self.boats[i].entity_type(), self.boats[i].id as i32);
        let (damage, _, _, magic) = self.melee_damage(&kind, false);
        self.hurt_boat(i, damage, self.creative);
        self.vehicle_attack_sounds(area, magic.then_some(id));
        self.swing_arm();
        true
    }
    /// `VehicleEntity.hurtServer`: wobble, add the damage and break past 40
    /// (`creative`: a creative player's hit discards it straight away).
    /// `AbstractChestBoat.remove` drops the chest contents for every removal
    /// reason that destroys the boat (DISCARDED included), while the boat item
    /// itself only drops on the death path (`VehicleEntity.destroy`), not when
    /// a creative hit discards it. True when the boat was removed.
    pub(super) fn hurt_boat(&mut self, i: usize, damage: f32, creative: bool) -> bool {
        let b = &mut self.boats[i];
        b.hurt_dir = -b.hurt_dir;
        b.hurt_time = 10;
        b.damage += damage * 10.0;
        let destroyed = creative || b.damage > 40.0;
        if destroyed {
            self.eject_boat_mobs(i, true);
            let id = self.boats[i].id;
            self.release_vehicle_container(id);
            let b = self.boats.remove(i);
            let at = [b.position.x, b.position.y, b.position.z];
            // AbstractBoat.remove: a destroyed boat drops its lead.
            self.boat_removed(&b);
            // `AbstractChestBoat.remove` spills the chest through
            // `Containers.dropContents` for every destroying removal reason;
            // that has no `entity_drops` gate of its own.
            for stack in b.items.iter().flatten() {
                self.spawn_item(stack.clone(), at, super::dispenser::new_item_motion(), 10);
            }
            // `VehicleEntity.destroy(level, dropItem)` drops the boat item on
            // the death path only, and only with `entity_drops` on.
            if !creative && self.ui.rules.bool("entity_drops") {
                let item = b.entity_type();
                self.spawn_item(
                    ItemStack {
                        item: Arc::from(item.as_str()),
                        count: 1,
                        damage: 0,
                        extra: None,
                    },
                    at,
                    [0.0, 0.1, 0.0],
                    10,
                );
            }
        }
        destroyed
    }
    /// AbstractBoatRenderer.
    pub(super) fn boat_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        let a = alpha as f32;
        let range = super::entity_culling::default_range(
            glam::DVec3::new(WIDTH, HEIGHT, WIDTH),
            self.view_scale(),
        );
        for b in &self.boats {
            let entity_start = out.len();
            // EntityRenderer.shouldRender.
            let (previous, position) = (at(b.previous), at(b.position));
            if !self.standing_entity_in_view(
                camera, previous, position, alpha, WIDTH, HEIGHT, range, 0.0,
            ) {
                continue;
            }
            let models = if b.chest {
                &self.boat_models.chest_models
            } else {
                &self.boat_models.models
            };
            let Some(model) = models.get(&*b.wood) else {
                continue;
            };
            let folder = if b.chest { "chest_boat" } else { "boat" };
            let Some(uv) = self
                .gui
                .atlas
                .sprites
                .get(&format!("entity/{folder}/{}", b.wood))
                .map(|r| [r.u0, r.v0, r.u1, r.v1])
            else {
                continue;
            };
            let p = glam::DVec3::new(
                b.previous.x + (b.position.x - b.previous.x) * alpha,
                b.previous.y + (b.position.y - b.previous.y) * alpha,
                b.previous.z + (b.position.z - b.previous.z) * alpha,
            ) - camera;
            let yaw = b.previous_yaw + (b.yaw - b.previous_yaw) * a;
            let hurt = b.hurt_time as f32 - a;
            let damage = (b.damage - a).max(0.0);
            let mut base =
                glam::Mat4::from_translation(p.as_vec3() + glam::Vec3::new(0.0, 0.375, 0.0))
                    * glam::Mat4::from_rotation_y((180.0 - yaw).to_radians());
            if hurt > 0.0 {
                base *= glam::Mat4::from_rotation_x(
                    (hurt.sin() * hurt * damage / 10.0 * b.hurt_dir as f32).to_radians(),
                );
            }
            // AbstractBoatRenderer.extractRenderState: bubbleAngle.
            let wobble = b.bubble_angle(a);
            // AbstractBoatRenderer.submit: a boat above water wobbles in a
            // bubble column, about the axis (1, 0, 1).
            if !b.under_water() && wobble.abs() >= 1.0e-5 {
                base *= glam::Mat4::from_axis_angle(
                    glam::Vec3::new(1.0, 0.0, 1.0).normalize(),
                    wobble * (std::f64::consts::PI / 180.0) as f32,
                );
            }
            base *= glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0))
                * glam::Mat4::from_rotation_y(90f32.to_radians());
            let mut poses = Poses::new(model);
            // AbstractBoatModel.animatePaddle
            for (side, name) in [(0usize, "left_paddle"), (1, "right_paddle")] {
                if !poses.has(name) {
                    continue;
                }
                let t = b.rowing_time(side, a);
                let lerp = |f: f32, lo: f32, hi: f32| lo + f.clamp(0.0, 1.0) * (hi - lo);
                let pose = poses.get(name);
                pose[3] = lerp(((-t).sin() + 1.0) / 2.0, -PI / 3.0, -PI / 12.0);
                pose[4] = lerp(((-t + 1.0).sin() + 1.0) / 2.0, -PI / 4.0, PI / 4.0);
                if side == 1 {
                    pose[4] = PI - pose[4];
                }
            }
            let l = self.world.light(
                b.position.x.floor() as i32,
                (b.position.y + 0.3).floor() as i32,
                b.position.z.floor() as i32,
            );
            let packed = (u32::from(l >> 4) * 16) | ((u32::from(l & 15) * 16) << 8);
            horizon_client::mobs::render::draw(
                model,
                base,
                &poses,
                uv,
                0xFFFF_FFFF,
                packed,
                0,
                out,
            );
            // BoatRenderer.submitTypeAdditions: the water patch hides the water
            // inside the hull unless the boat is under water (RaftRenderer has none).
            if let Some(patch) = self.boat_models.water_patch.as_ref()
                && &*b.wood != "bamboo"
                && !matches!(b.status, Status::UnderWater | Status::UnderFlowingWater)
            {
                horizon_client::mobs::render::draw(
                    patch,
                    base,
                    &Poses::new(patch),
                    uv,
                    0xFFFF_FFFF,
                    packed,
                    horizon_client::gfx::WATER_MASK_OVERLAY,
                    out,
                );
            }
            self.cull_entity_vertices(out, entity_start);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boat() -> Boat {
        Boat::new(1, Arc::from("oak"), Vec3::ZERO, 0.0)
    }

    /// `AbstractBoat.tickBubbleColumn` on a client: the wobble grows while the
    /// bubble time runs and fades after.
    #[test]
    fn the_wobble_grows_while_the_bubble_time_runs_and_fades_after() {
        let mut b = boat();
        b.bubble_time = BUBBLE_TIME;
        for _ in 0..30 {
            b.tick_bubble_wobble();
        }
        // 0.05 a tick, capped at 1.
        assert_eq!(b.bubble_multiplier, 1.0);
        // 10 * sin(0.5 * tickCount) * multiplier.
        let expected = 10.0 * (0.5 * 30.0f64).sin() as f32;
        assert!((b.bubble_angle - expected).abs() < 1.0e-4);
        // The render angle lerps between the last two ticks.
        let mid = b.bubble_angle(0.5);
        assert!((mid - (b.bubble_angle_old + b.bubble_angle) / 2.0).abs() < 1.0e-5);
        // Once the bubble time is 0 the multiplier fades by 0.1 a tick.
        b.bubble_time = 0;
        for _ in 0..4 {
            b.tick_bubble_wobble();
        }
        assert!((b.bubble_multiplier - 0.6).abs() < 1.0e-5);
        for _ in 0..10 {
            b.tick_bubble_wobble();
        }
        assert_eq!(b.bubble_multiplier, 0.0);
        assert_eq!(b.bubble_angle, 0.0);
    }

    /// `AbstractBoat.tickBubbleColumn` on the server: a boat held over a
    /// column fires on the 60th tick, with the column's direction, then the
    /// time restarts.
    #[test]
    fn the_bubble_countdown_fires_every_sixty_ticks_over_a_column() {
        for down in [false, true] {
            let mut b = boat();
            let mut fired = Vec::new();
            for tick in 1..=130 {
                // What boat_above_bubble_column does each tick.
                b.above_bubble_column = true;
                b.bubble_down = down;
                if b.bubble_time == 0 {
                    b.bubble_time = BUBBLE_TIME;
                }
                if let Some(d) = b.tick_bubble_countdown() {
                    fired.push((tick, d));
                }
                assert!(!b.above_bubble_column);
            }
            assert_eq!(fired, vec![(60, down), (120, down)]);
        }
    }

    /// Leaving the column resets the time: nothing fires, and a boat with no
    /// time left does nothing over a column until the column sets it.
    #[test]
    fn leaving_the_column_resets_the_bubble_countdown() {
        let mut b = boat();
        b.bubble_time = BUBBLE_TIME;
        for _ in 0..30 {
            b.above_bubble_column = true;
            assert_eq!(b.tick_bubble_countdown(), None);
        }
        assert_eq!(b.bubble_time, 30);
        // A tick away from the column.
        assert_eq!(b.tick_bubble_countdown(), None);
        assert_eq!(b.bubble_time, 0);
        b.above_bubble_column = true;
        assert_eq!(b.tick_bubble_countdown(), None);
        assert!(b.above_bubble_column);
    }

    #[test]
    fn a_boat_that_never_saw_a_column_does_not_wobble() {
        let mut b = boat();
        for _ in 0..10 {
            b.tick_bubble_wobble();
        }
        assert_eq!(b.bubble_angle, 0.0);
        assert_eq!(b.bubble_angle(0.5), 0.0);
    }
}
