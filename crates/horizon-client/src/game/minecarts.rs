//! Minecarts (`AbstractMinecart` with `OldMinecartBehavior`): following
//! rails and slopes, powered and activator rails, coming off the track,
//! riding with the rider's push, pushing by players and other carts,
//! `MinecartItem` placement, `VehicleEntity` breaking, the chest, hopper,
//! furnace and TNT variants, saving, and `AbstractMinecartRenderer`'s old
//! rail-following render with the display block.
use super::{Game, Hit, InteractTarget, Keys};
use horizon_client::{
    blocks::Fluid,
    collisions::SceneCollisions,
    containers::BlockEntity,
    items::{ItemStack, Items},
    mesher::Vertex,
    mobs::{Part, Species, Who, anim::Poses, riders},
    particles::{ParticleOptions, ParticleType},
    scoreboard::pushable_by,
    sound_engine::{CartSound, SoundEngine, SoundInstance, Sounds},
};
use horizon_core::{
    geometry::{Aabb, Vec3, trig},
    random::RandomSource,
};
use horizon_world::registry::BlockStateId;
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum CartKind {
    Rideable,
    Chest,
    Hopper,
    Furnace,
    Tnt,
    /// `MinecartCommandBlock`: its `BaseCommandBlock` is `Minecart::command`,
    /// run by a powered activator rail (`game/command_minecarts.rs`).
    CommandBlock,
    /// `MinecartSpawner`: a `BaseSpawner` that ticks where the cart is
    /// (`Minecart::spawner`). Only `/summon` makes one: there is no item.
    Spawner,
}
impl CartKind {
    pub fn from_id(id: &str) -> Option<Self> {
        Some(match id.strip_prefix("minecraft:")? {
            "minecart" => Self::Rideable,
            "chest_minecart" => Self::Chest,
            "hopper_minecart" => Self::Hopper,
            "furnace_minecart" => Self::Furnace,
            "tnt_minecart" => Self::Tnt,
            "command_block_minecart" => Self::CommandBlock,
            "spawner_minecart" => Self::Spawner,
            _ => return None,
        })
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Rideable => "minecraft:minecart",
            Self::Chest => "minecraft:chest_minecart",
            Self::Hopper => "minecraft:hopper_minecart",
            Self::Furnace => "minecraft:furnace_minecart",
            Self::Tnt => "minecraft:tnt_minecart",
            Self::CommandBlock => "minecraft:command_block_minecart",
            Self::Spawner => "minecraft:spawner_minecart",
        }
    }
    /// `AbstractMinecart.getDropItem` and `getPickResult`: the item a cart drops and
    /// is picked as (a spawner minecart's is a plain minecart: there is no item of its own).
    pub fn item(self) -> &'static str {
        match self {
            Self::Spawner => "minecraft:minecart",
            kind => kind.id(),
        }
    }
    pub(super) fn slots(self) -> usize {
        match self {
            Self::Chest => 27,
            Self::Hopper => 5,
            _ => 0,
        }
    }
    /// Whether this cart's `interact` passes on the client, so that
    /// `Minecraft.startUseItem` uses the held item instead: `MinecartTNT` has
    /// none of its own (`Entity.interact`), `MinecartCommandBlock.interact`
    /// needs a game master, and `Minecart.interact` a player not sneaking at
    /// an empty cart.
    pub fn interact_passes(self, sneaking: bool, ridden: bool, game_master: bool) -> bool {
        match self {
            Self::Tnt | Self::Spawner => true,
            Self::CommandBlock => !game_master,
            Self::Rideable => sneaking || ridden,
            Self::Chest | Self::Hopper | Self::Furnace => false,
        }
    }
}
#[derive(Clone, Debug)]
pub(super) struct Minecart {
    pub id: u32,
    pub kind: CartKind,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub yaw: f32,
    pub previous_yaw: f32,
    pub flipped: bool,
    pub(super) on_ground: bool,
    horizontal_collision: bool,
    /// The horizontal speed the cart wanted when the collision stopped it:
    /// `MinecartTNT.tick` reads the delta after the move, which a cart on a
    /// rail keeps (`AbstractMinecart.moveAlongTrack` re-applies the track's
    /// velocity), so a cart that ran into a block still counts as moving.
    collision_speed: f64,
    fall_distance: f64,
    pub hurt_time: i32,
    pub hurt_dir: i32,
    pub damage: f32,
    /// `Entity.noGravity` (`NoGravity` in its save).
    pub no_gravity: bool,
    pub items: Vec<Option<ItemStack>>,
    /// Furnace minecarts: fuel ticks and `MinecartFurnace.push`. Its y part
    /// stays between ticks (it sets the push's length), only saving drops it.
    pub fuel: i32,
    pub push: Vec3,
    /// TNT minecarts: ticks until the explosion, -1 when not primed.
    pub fuse: i32,
    /// Hopper minecarts: an unpowered activator rail enables them.
    pub enabled: bool,
    /// Chest minecarts: the loot table rolled into `items` on load, saved
    /// again while the items are still that roll.
    pub loot: Option<horizon_client::save::RolledLoot>,
    /// `Entity.firstTick`: not ticked yet (saved as `HasTicked`).
    pub first_tick: bool,
    /// Command block minecarts: the cart's `BaseCommandBlock` (command,
    /// success count, last output, output tracking). The block-only flags of
    /// `CommandBlock` stay unset.
    pub command: horizon_client::command_block::CommandBlock,
    /// `Entity.tickCount`, which `MinecartCommandBlock.activateMinecart`
    /// measures its `ACTIVATION_DELAY` against.
    pub tick_count: i32,
    /// `MinecartCommandBlock.lastActivated`.
    pub last_activated: i32,
    /// An activator rail asked for the command during this tick; it runs once
    /// the cart is back in the level (`command_minecarts.rs`).
    pub command_due: bool,
    /// Spawner minecarts: the cart's `BaseSpawner` (`Delay`, `SpawnData`,
    /// `SpawnPotentials` and the rest as a spawner block saves them).
    pub spawner: Option<Box<horizon_client::mobs::spawner::Spawner>>,
    /// The synched `DATA_ID_COMMAND_NAME` and `DATA_ID_LAST_OUTPUT`, which
    /// follow the command block only at `onUpdated`.
    pub synced_command: String,
    pub synced_output: String,
}
pub(super) const WIDTH: f64 = 0.98;
pub(super) const HEIGHT: f64 = 0.7;
/// AbstractMinecart.getDefaultDisplayOffset, in sixteenths of a block.
fn display_offset(kind: CartKind) -> f64 {
    if kind == CartKind::Hopper { 1.0 } else { 6.0 }
}
/// `MinecartFurnace.calculateNewPushAlong`: the push turned onto the
/// movement's direction, at the push's own length (3D, so it holds from tick
/// to tick while the movement has a vertical part).
fn push_along(push: Vec3, movement: Vec3) -> Vec3 {
    if push.horizontal_distance_squared() > 1.0e-4 && movement.horizontal_distance_squared() > 0.001
    {
        push.projected_on(movement).normalize().scale(push.length())
    } else {
        push
    }
}
/// AbstractMinecart passenger attachment Y (EntityAttachments).
const PASSENGER_Y: f64 = 0.1875;
/// The minecart's passenger attachment minus Player.DEFAULT_VEHICLE_ATTACHMENT.
pub(super) const SEAT: f64 = PASSENGER_Y - 0.6;
/// The placeholder container key under which an open cart's items live.
pub(super) const CART_KEY_Y: i32 = horizon_client::lan::menu::VEHICLE_KEY_Y;

/// `OldMinecartBehavior.pushAndPickupEntities`: living entities the cart
/// offers `startRiding` instead of `Entity.push`. Players click to board;
/// iron golems always push; a cart that already has a rider is `isVehicle`.
fn pickup_instead_of_push(species: Species, is_passenger: bool, cart_is_vehicle: bool) -> bool {
    !cart_is_vehicle && !is_passenger && species != Species::IronGolem
}

/// `Entity.canRide` for a cart: the boarding cooldown, and a warden never rides.
fn can_ride_cart(species: Species, boarding_cooldown: i32) -> bool {
    boarding_cooldown <= 0 && species != Species::Warden
}

/// `Entity.positionRider` on an `AbstractMinecart`: passenger attachment less
/// the rider's own vehicle attachment.
fn cart_seat(cart: Vec3, rider_offset: f64) -> Vec3 {
    Vec3::new(cart.x, cart.y + PASSENGER_Y - rider_offset, cart.z)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Shape {
    NorthSouth,
    EastWest,
    AscendingEast,
    AscendingWest,
    AscendingNorth,
    AscendingSouth,
    SouthEast,
    SouthWest,
    NorthWest,
    NorthEast,
}
impl Shape {
    fn parse(s: &str) -> Self {
        match s {
            "east_west" => Self::EastWest,
            "ascending_east" => Self::AscendingEast,
            "ascending_west" => Self::AscendingWest,
            "ascending_north" => Self::AscendingNorth,
            "ascending_south" => Self::AscendingSouth,
            "south_east" => Self::SouthEast,
            "south_west" => Self::SouthWest,
            "north_west" => Self::NorthWest,
            "north_east" => Self::NorthEast,
            _ => Self::NorthSouth,
        }
    }
    pub(super) fn is_slope(self) -> bool {
        matches!(
            self,
            Self::AscendingEast | Self::AscendingWest | Self::AscendingNorth | Self::AscendingSouth
        )
    }
    /// `AbstractMinecart.EXITS`.
    fn exits(self) -> ([i32; 3], [i32; 3]) {
        let (w, e, n, s) = ([-1, 0, 0], [1, 0, 0], [0, 0, -1], [0, 0, 1]);
        let below = |v: [i32; 3]| [v[0], -1, v[2]];
        match self {
            Self::NorthSouth => (n, s),
            Self::EastWest => (w, e),
            Self::AscendingEast => (below(w), e),
            Self::AscendingWest => (w, below(e)),
            Self::AscendingNorth => (n, below(s)),
            Self::AscendingSouth => (below(n), s),
            Self::SouthEast => (s, e),
            Self::SouthWest => (s, w),
            Self::NorthWest => (n, w),
            Self::NorthEast => (n, e),
        }
    }
}
pub(super) struct Rail {
    name: Arc<str>,
    pub(super) shape: Shape,
    powered: bool,
}
impl Minecart {
    /// `Entity.onGround`.
    pub(super) fn on_ground(&self) -> bool {
        self.on_ground
    }
    pub fn new(id: u32, kind: CartKind, position: Vec3) -> Self {
        Self {
            id,
            kind,
            position,
            previous: position,
            velocity: Vec3::ZERO,
            yaw: 0.0,
            previous_yaw: 0.0,
            flipped: false,
            on_ground: false,
            horizontal_collision: false,
            collision_speed: 0.0,
            fall_distance: 0.0,
            hurt_time: 0,
            hurt_dir: 1,
            damage: 0.0,
            no_gravity: false,
            items: vec![None; kind.slots()],
            fuel: 0,
            push: Vec3::ZERO,
            fuse: -1,
            enabled: true,
            loot: None,
            first_tick: true,
            command: horizon_client::command_block::CommandBlock::default(),
            tick_count: 0,
            last_activated: 0,
            command_due: false,
            // A fresh `BaseSpawner`, as `MinecartSpawner`'s constructor makes.
            spawner: (kind == CartKind::Spawner).then(Default::default),
            synced_command: String::new(),
            synced_output: String::new(),
        }
    }
    pub fn bounds(&self) -> Aabb {
        let (w, p) = (WIDTH / 2.0, self.position);
        Aabb::new(
            Vec3::new(p.x - w, p.y, p.z - w),
            Vec3::new(p.x + w, p.y + HEIGHT, p.z + w),
        )
    }
    fn horizontal(&self) -> f64 {
        self.velocity.x.hypot(self.velocity.z)
    }
    /// `AbstractContainerMenu.getRedstoneSignalFromContainer` of `slots`.
    pub(super) fn container_signal(slots: &[Option<ItemStack>], items: &Items) -> i32 {
        let mut total = 0.0f32;
        for s in slots.iter().flatten() {
            let max = items.get(&s.item).map_or(64, |i| i.max_stack).min(64);
            total += s.count as f32 / max as f32;
        }
        total /= slots.len().max(1) as f32;
        // Mth.lerpDiscrete(total, 0, 15)
        (total * 14.0).floor() as i32 + i32::from(total > 0.0)
    }
}
pub(super) struct CartModels {
    pub models: HashMap<CartKind, Part>,
}
impl CartModels {
    pub fn load(document: Option<&horizon_client::mobs::EntityModelDocument>) -> Self {
        let kinds = [
            CartKind::Rideable,
            CartKind::Chest,
            CartKind::Hopper,
            CartKind::Furnace,
            CartKind::Tnt,
            CartKind::CommandBlock,
            CartKind::Spawner,
        ];
        let models = kinds
            .into_iter()
            .filter_map(|k| document?.model(&format!("{}#main", k.id())).map(|m| (k, m)))
            .collect();
        Self { models }
    }
}
impl Game<'_> {
    pub(super) fn rail(&self, x: i32, y: i32, z: i32) -> Option<Rail> {
        let state = self.world.block(x, y, z);
        let name = self.res.facts().get(state).name.clone();
        if !matches!(
            &*name,
            "minecraft:rail"
                | "minecraft:powered_rail"
                | "minecraft:detector_rail"
                | "minecraft:activator_rail"
        ) {
            return None;
        }
        let s = self.pack.assets.registry.state(state)?;
        let shape = Shape::parse(s.properties.get("shape").map_or("", String::as_str));
        let powered = s.properties.get("powered").is_some_and(|v| v == "true");
        Some(Rail {
            name,
            shape,
            powered,
        })
    }
    /// `getCurrentBlockPosOrRailBelow`.
    fn rail_block(&self, p: Vec3) -> [i32; 3] {
        let (x, y, z) = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        if self.rail(x, y - 1, z).is_some() {
            [x, y - 1, z]
        } else {
            [x, y, z]
        }
    }
    fn conductor(&self, x: i32, y: i32, z: i32) -> bool {
        self.res
            .facts()
            .get(self.world.block(x, y, z))
            .full_collision
    }
    fn cart_in_water(&self, c: &Minecart) -> bool {
        let p = c.position;
        let f = self.res.facts().get(self.world.block(
            p.x.floor() as i32,
            (p.y + 0.1).floor() as i32,
            p.z.floor() as i32,
        ));
        f.fluid == Fluid::Water
    }
    /// `OldMinecartBehavior.getPos`: the point on the rail under (x, y, z).
    fn rail_pos(&self, x: f64, y: f64, z: f64) -> Option<[f64; 3]> {
        let [xt, yt, zt] = self.rail_block(Vec3::new(x, y, z));
        let rail = self.rail(xt, yt, zt)?;
        let (e0, e1) = rail.shape.exits();
        let (fx, fy, fz) = (f64::from(xt), f64::from(yt), f64::from(zt));
        let x0 = fx + 0.5 + f64::from(e0[0]) * 0.5;
        let y0 = fy + 0.0625 + f64::from(e0[1]) * 0.5;
        let z0 = fz + 0.5 + f64::from(e0[2]) * 0.5;
        let x1 = fx + 0.5 + f64::from(e1[0]) * 0.5;
        let y1 = fy + 0.0625 + f64::from(e1[1]) * 0.5;
        let z1 = fz + 0.5 + f64::from(e1[2]) * 0.5;
        let (xd, yd, zd) = (x1 - x0, (y1 - y0) * 2.0, z1 - z0);
        let progress = if xd == 0.0 {
            z - fz
        } else if zd == 0.0 {
            x - fx
        } else {
            ((x - x0) * xd + (z - z0) * zd) * 2.0
        };
        let mut out = [x0 + xd * progress, y0 + yd * progress, z0 + zd * progress];
        if yd < 0.0 {
            out[1] += 1.0;
        } else if yd > 0.0 {
            out[1] += 0.5;
        }
        Some(out)
    }
    /// `OldMinecartBehavior.getPosOffs`.
    fn rail_pos_offs(&self, mut x: f64, y: f64, mut z: f64, offs: f64) -> Option<[f64; 3]> {
        let [xt, yt, zt] = self.rail_block(Vec3::new(x, y, z));
        let rail = self.rail(xt, yt, zt)?;
        let mut y = f64::from(yt) + if rail.shape.is_slope() { 1.0 } else { 0.0 };
        let (e0, e1) = rail.shape.exits();
        let (mut xd, mut zd) = (f64::from(e1[0] - e0[0]), f64::from(e1[2] - e0[2]));
        let dd = xd.hypot(zd);
        xd /= dd;
        zd /= dd;
        x += xd * offs;
        z += zd * offs;
        let at = |e: [i32; 3]| x.floor() as i32 - xt == e[0] && z.floor() as i32 - zt == e[2];
        if e0[1] != 0 && at(e0) {
            y += f64::from(e0[1]);
        } else if e1[1] != 0 && at(e1) {
            y += f64::from(e1[1]);
        }
        self.rail_pos(x, y, z)
    }
    /// `AbstractMinecart.canCollideWith`, for `Entity.collide`'s `getEntityCollisions`:
    /// `AbstractBoat.canVehicleCollide` takes any other cart, boat, shulker or pushable mob and
    /// player, except one riding this cart. A cart that runs into one has a horizontal
    /// collision, which sets a TNT minecart off.
    fn cart_entity_collisions(&self, c: &Minecart) -> Vec<Aabb> {
        let near = c.bounds().inflate(Vec3::new(2.0, 2.0, 2.0));
        let mut boxes: Vec<Aabb> = self
            .carts
            .iter()
            .filter(|o| o.id != c.id)
            .map(Minecart::bounds)
            .chain(self.boats.iter().map(super::boats::Boat::bounds))
            .chain(
                self.mobs
                    .iter()
                    .filter(|m| {
                        !m.removed
                            && m.extra.mount != Some(c.id)
                            && (m.pushable()
                                || m.species() == horizon_client::mobs::Species::Shulker
                                    && m.alive())
                    })
                    .map(|m| m.bounds()),
            )
            .collect();
        if self.riding != Some(c.id)
            && let Some(p) = self.player.as_ref()
            && !self.spectator()
        {
            boxes.push(p.bounds());
        }
        if let Some(host) = self.lan.as_ref() {
            boxes.extend(
                host.hub
                    .guests
                    .iter()
                    .filter(|g| {
                        host.hub.here(g)
                            && g.loaded
                            && !g.dead
                            && !g.spectator()
                            && g.vehicle != Some(c.id)
                    })
                    .map(|g| g.bounds()),
            );
        }
        boxes.retain(|b| b.intersects(near));
        boxes
    }
    /// Entity.move for a cart: collisions zero the blocked components.
    fn move_cart(&self, c: &mut Minecart, delta: Vec3) {
        let scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        let entities = self.cart_entity_collisions(c);
        let mut scene = horizon_client::collisions::EntityCollisions {
            blocks: scene,
            entities: &entities,
            border: None,
        };
        if let Ok(moved) = scene.resolve(c.bounds(), delta, 0.0, c.on_ground) {
            c.position = Vec3::new(
                c.position.x + moved.x,
                c.position.y + moved.y,
                c.position.z + moved.z,
            );
            c.on_ground = delta.y < 0.0 && moved.y > delta.y;
            c.horizontal_collision =
                (moved.x - delta.x).abs() > 1.0e-7 || (moved.z - delta.z).abs() > 1.0e-7;
            if c.horizontal_collision {
                c.collision_speed = delta.x * delta.x + delta.z * delta.z;
            }
            if (moved.x - delta.x).abs() > 1.0e-7 {
                c.velocity.x = 0.0;
            }
            if (moved.z - delta.z).abs() > 1.0e-7 {
                c.velocity.z = 0.0;
            }
            if (moved.y - delta.y).abs() > 1.0e-7 {
                c.velocity.y = 0.0;
            }
            if c.on_ground || moved.y >= 0.0 {
                c.fall_distance = 0.0;
            } else {
                c.fall_distance -= moved.y;
            }
        }
    }
    fn max_speed(&self, c: &Minecart) -> f64 {
        let base = if self.cart_in_water(c) { 0.2 } else { 0.4 };
        if c.kind == CartKind::Furnace {
            base * if self.cart_in_water(c) { 0.75 } else { 0.5 }
        } else {
            base
        }
    }
    /// `applyNaturalSlowdown` (with `MinecartFurnace`'s push); `factor` is
    /// `getSlowdownFactor`: 0.997 riding, the empty cart's own by behavior.
    fn natural_slowdown(&self, c: &mut Minecart, ridden: bool, factor: f64) -> Vec3 {
        let mut v = c.velocity;
        let water = self.cart_in_water(c);
        if c.kind == CartKind::Furnace {
            if c.push.length_squared() > 1.0e-7 {
                c.push = push_along(c.push, v);
                v = Vec3::new(v.x * 0.8 + c.push.x, 0.0, v.z * 0.8 + c.push.z);
                if water {
                    v = Vec3::new(v.x * 0.1, 0.0, v.z * 0.1);
                }
            } else {
                v = Vec3::new(v.x * 0.98, 0.0, v.z * 0.98);
            }
        }
        let f = if ridden { 0.997 } else { factor };
        let mut out = Vec3::new(v.x * f, 0.0, v.z * f);
        if water {
            out = Vec3::new(out.x * 0.95, 0.0, out.z * 0.95);
        }
        out
    }
    /// `Entity.isVehicle` for a cart: the local player, a LAN guest or a mob
    /// sits in it. `getSlowdownFactor` (0.997) and the 0.75 speed scale key on
    /// this, not on who rides.
    fn cart_is_vehicle(&self, id: u32) -> bool {
        self.riding == Some(id)
            || self.ridden_by_guest(id)
            || self
                .mobs
                .iter()
                .any(|m| !m.removed && m.extra.mount == Some(id))
    }
    /// `OldMinecartBehavior.moveAlongTrack`.
    fn move_along_track(&mut self, c: &mut Minecart, pos: [i32; 3], rail: &Rail, intent: [f64; 2]) {
        c.fall_distance = 0.0;
        let ridden = self.cart_is_vehicle(c.id);
        let (x, z) = (c.position.x, c.position.z);
        let old = self.rail_pos(x, c.position.y, z);
        let mut y = f64::from(pos[1]);
        let (mut power, mut halt) = (false, false);
        if &*rail.name == "minecraft:powered_rail" {
            power = rail.powered;
            halt = !power;
        }
        let slide = if self.cart_in_water(c) {
            0.0078125 * 0.2
        } else {
            0.0078125
        };
        match rail.shape {
            Shape::AscendingEast => c.velocity.x -= slide,
            Shape::AscendingWest => c.velocity.x += slide,
            Shape::AscendingNorth => c.velocity.z += slide,
            Shape::AscendingSouth => c.velocity.z -= slide,
            _ => {}
        }
        if rail.shape.is_slope() {
            y += 1.0;
        }
        let (e0, e1) = rail.shape.exits();
        let (mut xd, mut zd) = (f64::from(e1[0] - e0[0]), f64::from(e1[2] - e0[2]));
        let length = xd.hypot(zd);
        if c.velocity.x * xd + c.velocity.z * zd < 0.0 {
            xd = -xd;
            zd = -zd;
        }
        let pow = c.horizontal().min(2.0);
        c.velocity = Vec3::new(pow * xd / length, c.velocity.y, pow * zd / length);
        if ridden
            && (intent[0] != 0.0 || intent[1] != 0.0)
            && c.velocity.x.powi(2) + c.velocity.z.powi(2) < 0.01
        {
            c.velocity.x += intent[0] * 0.001;
            c.velocity.z += intent[1] * 0.001;
            halt = false;
        }
        if halt {
            if c.horizontal() < 0.03 {
                c.velocity = Vec3::ZERO;
            } else {
                c.velocity = Vec3::new(c.velocity.x * 0.5, 0.0, c.velocity.z * 0.5);
            }
        }
        let (fx, fz) = (f64::from(pos[0]), f64::from(pos[2]));
        let x0 = fx + 0.5 + f64::from(e0[0]) * 0.5;
        let z0 = fz + 0.5 + f64::from(e0[2]) * 0.5;
        let x1 = fx + 0.5 + f64::from(e1[0]) * 0.5;
        let z1 = fz + 0.5 + f64::from(e1[2]) * 0.5;
        let (xd, zd) = (x1 - x0, z1 - z0);
        let progress = if xd == 0.0 {
            z - fz
        } else if zd == 0.0 {
            x - fx
        } else {
            ((x - x0) * xd + (z - z0) * zd) * 2.0
        };
        c.position = Vec3::new(x0 + xd * progress, y, z0 + zd * progress);
        let scale = if ridden { 0.75 } else { 1.0 };
        let max = self.max_speed(c);
        let delta = Vec3::new(
            (scale * c.velocity.x).clamp(-max, max),
            0.0,
            (scale * c.velocity.z).clamp(-max, max),
        );
        self.move_cart(c, delta);
        let at = |e: [i32; 3], c: &Minecart| {
            c.position.x.floor() as i32 - pos[0] == e[0]
                && c.position.z.floor() as i32 - pos[2] == e[2]
        };
        if e0[1] != 0 && at(e0, c) {
            c.position.y += f64::from(e0[1]);
        } else if e1[1] != 0 && at(e1, c) {
            c.position.y += f64::from(e1[1]);
        }
        c.velocity = self.natural_slowdown(c, ridden, 0.96);
        if let (Some(new), Some(old)) =
            (self.rail_pos(c.position.x, c.position.y, c.position.z), old)
        {
            let speed = (old[1] - new[1]) * 0.05;
            let h = c.horizontal();
            if h > 0.0 {
                c.velocity.x *= (h + speed) / h;
                c.velocity.z *= (h + speed) / h;
            }
            c.position.y = new[1];
        }
        let (xn, zn) = (c.position.x.floor() as i32, c.position.z.floor() as i32);
        if xn != pos[0] || zn != pos[2] {
            let h = c.horizontal();
            c.velocity = Vec3::new(
                h * f64::from(xn - pos[0]),
                c.velocity.y,
                h * f64::from(zn - pos[2]),
            );
        }
        if power {
            let h = c.horizontal();
            if h > 0.01 {
                c.velocity.x += c.velocity.x / h * 0.06;
                c.velocity.z += c.velocity.z / h * 0.06;
            } else if rail.shape == Shape::EastWest {
                if self.conductor(pos[0] - 1, pos[1], pos[2]) {
                    c.velocity.x = 0.02;
                } else if self.conductor(pos[0] + 1, pos[1], pos[2]) {
                    c.velocity.x = -0.02;
                }
            } else if rail.shape == Shape::NorthSouth {
                if self.conductor(pos[0], pos[1], pos[2] - 1) {
                    c.velocity.z = 0.02;
                } else if self.conductor(pos[0], pos[1], pos[2] + 1) {
                    c.velocity.z = -0.02;
                }
            }
        }
    }
    /// `AbstractMinecart`: `minecart_improvements` picks the smoother
    /// physics.
    fn minecart_improvements(&self) -> bool {
        self.level
            .enabled_features
            .iter()
            .any(|f| f == "minecraft:minecart_improvements")
    }
    /// `NewMinecartBehavior.getMaxSpeed`: the game rule over water's half,
    /// every cart alike (the old behavior's furnace bonus is its own).
    fn max_speed_new(&self, c: &Minecart) -> f64 {
        if self.cart_in_water(c) { 0.2 } else { 0.4 }
    }
    /// `AbstractMinecart.getRedstoneDirection` (over the old port's
    /// conductor proxy): which way a powered rail pushes a resting cart.
    fn redstone_direction(&self, pos: [i32; 3], shape: Shape) -> Vec3 {
        let (dx, dz) = match shape {
            Shape::NorthSouth | Shape::AscendingNorth | Shape::AscendingSouth => (0, 1),
            _ => (1, 0),
        };
        // The push points away from the signal's side (getRedstoneDirection).
        if self.conductor(pos[0] - dx, pos[1], pos[2] - dz) {
            Vec3::new(f64::from(dx), 0.0, f64::from(dz))
        } else if self.conductor(pos[0] + dx, pos[1], pos[2] + dz) {
            Vec3::new(f64::from(-dx), 0.0, f64::from(-dz))
        } else {
            Vec3::ZERO
        }
    }
    /// `NewMinecartBehavior.isDecending`: the movement goes down the
    /// shape's rise.
    fn is_descending(movement: Vec3, shape: Shape) -> bool {
        match shape {
            Shape::AscendingEast => movement.x < 0.0,
            Shape::AscendingWest => movement.x > 0.0,
            Shape::AscendingNorth => movement.z > 0.0,
            Shape::AscendingSouth => movement.z < 0.0,
            _ => false,
        }
    }
    /// `NewMinecartBehavior.moveAlongTrack`: the iteration keeps moving
    /// while distance is left, over as many rails as it takes, the speed
    /// recomputed per rail (slope, halt, boost).
    fn move_along_track_new(&mut self, c: &mut Minecart, _rail: &Rail, intent: [f64; 2]) {
        c.fall_distance = 0.0;
        let ridden = self.cart_is_vehicle(c.id);
        // `TrackIteration`: the slope, halt and boost speeds each apply once
        // per tick, and the iteration keeps moving while distance is left.
        let mut first = true;
        let mut movement_left = 0.0;
        let mut slope_done = false;
        let mut halted = false;
        let mut boosted = false;
        for _ in 0..16 {
            if !first && movement_left <= 1.0e-5 {
                break;
            }
            let pos = self.rail_block(c.position);
            let Some(rail) = self.rail(pos[0], pos[1], pos[2]) else {
                self.come_off_track(c);
                return;
            };
            let initial_h = c.velocity.x.hypot(c.velocity.z);
            let mut v = c.velocity;
            // calculateSlopeSpeed (once per tick): the slide grows with the
            // speed, so a descending cart keeps its pace.
            if !slope_done {
                let base: f64 = if self.cart_in_water(c) {
                    0.0078125 * 0.2
                } else {
                    0.0078125
                };
                let slide = base.max(initial_h * 0.02);
                let before = v.x.hypot(v.z);
                match rail.shape {
                    Shape::AscendingEast => v.x -= slide,
                    Shape::AscendingWest => v.x += slide,
                    Shape::AscendingNorth => v.z += slide,
                    Shape::AscendingSouth => v.z -= slide,
                    _ => {}
                }
                slope_done = v.x.hypot(v.z) != before;
            }
            // calculatePlayerInputSpeed (first iteration).
            if first
                && ridden
                && (intent[0] != 0.0 || intent[1] != 0.0)
                && v.x * v.x + v.z * v.z < 0.01
            {
                let l = (intent[0] * intent[0] + intent[1] * intent[1]).sqrt();
                v.x += intent[0] / l * 0.001;
                v.z += intent[1] / l * 0.001;
                halted = true;
            }
            // calculateHaltTrackSpeed (until the input halted).
            if !halted && &*rail.name == "minecraft:powered_rail" && !rail.powered {
                let before = v.x.hypot(v.z);
                if before < 0.03 {
                    v = Vec3::ZERO;
                } else {
                    v = Vec3::new(v.x * 0.5, v.y, v.z * 0.5);
                }
                halted = v.x.hypot(v.z) != before;
            }
            // applyNaturalSlowdown and the speed cap (first iteration).
            if first {
                v = self.natural_slowdown(c, ridden, 0.975);
                let len = v.x.hypot(v.z);
                if len > 0.0 {
                    let speed = len.min(self.max_speed_new(c));
                    v = Vec3::new(v.x / len * speed, v.y, v.z / len * speed);
                }
            }
            // calculateBoostTrackSpeed (once per tick).
            if !boosted && &*rail.name == "minecraft:powered_rail" && rail.powered {
                let len = v.x.hypot(v.z);
                if len > 0.01 {
                    v = Vec3::new(v.x / len * (len + 0.06), v.y, v.z / len * (len + 0.06));
                } else {
                    let dir = self.redstone_direction(pos, rail.shape);
                    if dir != Vec3::ZERO {
                        v = Vec3::new(dir.x * 0.2, v.y, dir.z * 0.2);
                    }
                }
                boosted = true;
            }
            c.velocity = v;
            let new_h = v.x.hypot(v.z);
            movement_left = if first {
                new_h
            } else {
                movement_left + (new_h - initial_h)
            };
            movement_left = self.new_step_along_track(c, pos, &rail, movement_left);
            first = false;
        }
    }
    /// `NewMinecartBehavior.stepAlongTrack`: toward the rail's exit, the
    /// hill's Y projected, a V of opposing slopes resting a slow cart.
    fn new_step_along_track(
        &mut self,
        c: &mut Minecart,
        pos: [i32; 3],
        rail: &Rail,
        mut movement_left: f64,
    ) -> f64 {
        if movement_left < 1.0e-5 {
            return 0.0;
        }
        let old = c.position;
        let (e0, e1) = rail.shape.exits();
        let movement = Vec3::new(c.velocity.x, 0.0, c.velocity.z);
        if movement.x.hypot(movement.z) < 1.0e-5 {
            c.velocity = Vec3::ZERO;
            return 0.0;
        }
        let in_hill = e0[1] != e1[1];
        let mut out_dir = Vec3::new(f64::from(e0[0]) * 0.5, 0.0, f64::from(e0[2]) * 0.5);
        let in_dir = Vec3::new(f64::from(e1[0]) * 0.5, 0.0, f64::from(e1[2]) * 0.5);
        if movement.dot(out_dir) < movement.dot(in_dir) {
            out_dir = in_dir;
        }
        let out_len = out_dir.x.hypot(out_dir.z);
        let mut out_pos = Vec3::new(
            f64::from(pos[0]) + 0.5 + out_dir.x + out_dir.x / out_len * 1.0e-5,
            f64::from(pos[1]) + 0.1,
            f64::from(pos[2]) + 0.5 + out_dir.z + out_dir.z / out_len * 1.0e-5,
        );
        if in_hill && !Self::is_descending(c.velocity, rail.shape) {
            out_pos.y += 1.0;
        }
        let towards_out = c.position.vector_to(out_pos).normalize();
        // The movement turns onto the exit's direction, at the same speed
        // (`this.setDeltaMovement(movement)` at stepAlongTrack's end).
        let h_speed = movement.x.hypot(movement.z);
        let th = towards_out.x.hypot(towards_out.z);
        let movement = towards_out.scale(h_speed / th);
        let mut new_pos = if in_hill {
            old.add_vector(
                movement
                    .normalize()
                    .scale(movement_left * std::f64::consts::SQRT_2),
            )
        } else {
            old.add_vector(movement.normalize().scale(movement_left))
        };
        if out_pos.distance(old) <= new_pos.distance(old) {
            movement_left = out_pos.vector_to(new_pos).horizontal_distance();
            new_pos = out_pos;
        } else {
            movement_left = 0.0;
        }
        self.move_cart(c, old.vector_to(new_pos));
        if in_hill {
            let new_block = self.rail_block(c.position);
            if let Some(new_rail) = self.rail(new_block[0], new_block[1], new_block[2]) {
                // restAtVShape: opposing slopes rest a slow cart.
                if c.velocity.length() < 0.005_f64.sqrt()
                    && new_rail.shape.is_slope()
                    && Self::is_descending(c.velocity, rail.shape)
                    && !Self::is_descending(c.velocity, new_rail.shape)
                {
                    c.velocity = Vec3::ZERO;
                    return 0.0;
                }
            }
            let h_dist = (out_pos.x - c.position.x).hypot(out_pos.z - c.position.z);
            let project = out_pos.y
                + if Self::is_descending(movement, rail.shape) {
                    h_dist
                } else {
                    -h_dist
                };
            if c.position.y < project {
                c.position.y = project;
            }
        }
        // The cart did not move but the step would have: it is stuck.
        if c.position.distance(old) < 1.0e-5 && new_pos.distance(old) > 1.0e-5 {
            c.velocity = Vec3::ZERO;
            return 0.0;
        }
        c.velocity = movement;
        movement_left
    }
    /// `comeOffTrack`.
    fn come_off_track(&self, c: &mut Minecart) {
        let max = self.max_speed(c);
        c.velocity.x = c.velocity.x.clamp(-max, max);
        c.velocity.z = c.velocity.z.clamp(-max, max);
        if c.on_ground {
            c.velocity = Vec3::new(c.velocity.x * 0.5, c.velocity.y * 0.5, c.velocity.z * 0.5);
        }
        let v = c.velocity;
        let fall = c.fall_distance;
        self.move_cart(c, v);
        // `comeOffTrack` moves the cart and keeps the delta the move resolved
        // (then scales it by the air drag), so a cart an obstacle stopped here
        // is not moving any more and `MinecartTNT.tick` does not detonate it -
        // unlike the rail paths, which re-apply the track's velocity.
        c.collision_speed = 0.0;
        if !c.on_ground {
            c.velocity = Vec3::new(
                c.velocity.x * 0.95,
                c.velocity.y * 0.95,
                c.velocity.z * 0.95,
            );
        } else if c.kind == CartKind::Tnt && fall >= 3.0 {
            // MinecartTNT.causeFallDamage
            c.fuse = 0;
            c.velocity = Vec3::new(c.velocity.x, fall / 10.0, c.velocity.z);
        }
    }
    /// `AbstractMinecart.tick` for one cart; returns false once it is gone.
    fn tick_cart(&mut self, c: &mut Minecart, intent: [f64; 2]) -> bool {
        c.first_tick = false;
        c.tick_count += 1;
        c.previous = c.position;
        c.previous_yaw = c.yaw;
        c.hurt_time = (c.hurt_time - 1).max(0);
        c.damage = (c.damage - 1.0).max(0.0);
        // applyGravity
        if !c.no_gravity {
            c.velocity.y -= if self.cart_in_water(c) { 0.005 } else { 0.04 };
        }
        let pos = self.rail_block(c.position);
        if let Some(rail) = self.rail(pos[0], pos[1], pos[2]) {
            if self.minecart_improvements() {
                self.move_along_track_new(c, &rail, intent);
            } else {
                self.move_along_track(c, pos, &rail, intent);
            }
            if &*rail.name == "minecraft:activator_rail" {
                self.activate_cart(c, rail.powered);
            }
        } else {
            self.come_off_track(c);
        }
        // Yaw from the movement, flipped when it turns around.
        let (xd, zd) = (c.previous.x - c.position.x, c.previous.z - c.position.z);
        if xd * xd + zd * zd > 0.001 {
            c.yaw = zd.atan2(xd).to_degrees() as f32;
            if c.flipped {
                c.yaw += 180.0;
            }
        }
        let diff = (c.yaw - c.previous_yaw + 180.0).rem_euclid(360.0) - 180.0;
        if !(-170.0..170.0).contains(&diff) {
            c.yaw += 180.0;
            c.flipped = !c.flipped;
        }
        c.yaw %= 360.0;
        // Variants.
        match c.kind {
            CartKind::Furnace => {
                if c.fuel > 0 {
                    c.fuel -= 1;
                }
                if c.fuel <= 0 {
                    c.push = Vec3::ZERO;
                }
                if c.fuel > 0 && self.world.random.next_int_bound(4).unwrap_or(1) == 0 {
                    let at = [c.position.x, c.position.y + 0.8, c.position.z];
                    self.particles.add(
                        &ParticleOptions::simple(ParticleType::LargeSmoke),
                        at,
                        [0.0; 3],
                    );
                }
            }
            CartKind::Tnt => {
                // MinecartTNT.tick: the fuse and the collision check are separate.
                if c.fuse > 0 {
                    c.fuse -= 1;
                    let at = [c.position.x, c.position.y + 0.5, c.position.z];
                    self.particles
                        .add(&ParticleOptions::simple(ParticleType::Smoke), at, [0.0; 3]);
                } else if c.fuse == 0 {
                    // explode(horizontalDistanceSqr()): a rising cart's
                    // vertical speed adds nothing.
                    let speed = c.velocity.x * c.velocity.x + c.velocity.z * c.velocity.z;
                    self.explode_cart(c, speed);
                    return false;
                }
                if tnt_ignites_by_collision(c.horizontal_collision, c.collision_speed) {
                    self.explode_cart(c, c.collision_speed);
                    return false;
                }
            }
            // MinecartSpawner.tick: `BaseSpawner.serverTick` at the cart's block.
            CartKind::Spawner => {
                if let Some(mut spawner) = c.spawner.take() {
                    let key = (
                        c.position.x.floor() as i32,
                        c.position.y.floor() as i32,
                        c.position.z.floor() as i32,
                    );
                    let _ = self.server_tick_spawner_of(key, &mut spawner, Some(c.id));
                    c.spawner = Some(spawner);
                }
            }
            _ => {}
        }
        true
    }
    /// `activateMinecart` on an activator rail.
    fn activate_cart(&mut self, c: &mut Minecart, powered: bool) {
        match c.kind {
            CartKind::Rideable if powered => {
                // Minecart.activateMinecart: ejectPassengers.
                if self.riding == Some(c.id) {
                    self.riding = None;
                }
                let y = c.position.y;
                for m in self.mobs.iter_mut().filter(|m| m.extra.mount == Some(c.id)) {
                    let at = Vec3::new(m.position.x, m.position.y.max(y), m.position.z);
                    riders::stop_riding(m, Some(at));
                    m.previous = at;
                }
                if c.hurt_time == 0 {
                    c.hurt_dir = -c.hurt_dir;
                    c.hurt_time = 10;
                    c.damage = 50.0;
                }
            }
            CartKind::Tnt if powered && c.fuse < 0 => self.prime_cart(c),
            CartKind::Hopper => c.enabled = !powered,
            // MinecartCommandBlock.activateMinecart: the command runs at most
            // every ACTIVATION_DELAY ticks.
            CartKind::CommandBlock if powered && c.tick_count - c.last_activated >= 4 => {
                c.command_due = true;
                c.last_activated = c.tick_count;
            }
            _ => {}
        }
    }
    fn prime_cart(&mut self, c: &mut Minecart) {
        if !self.ui.rules.bool("tnt_explodes") {
            return;
        }
        c.fuse = 80;
        let at = [c.position.x, c.position.y, c.position.z];
        self.audio
            .play("minecraft:entity.tnt.primed", Some(at), 1.0, 1.0);
    }
    fn explode_cart(&mut self, c: &Minecart, speed_sqr: f64) {
        if !self.ui.rules.bool("tnt_explodes") {
            return;
        }
        let speed = speed_sqr.sqrt().min(5.0);
        let r = self.world.random.next_double();
        let power = (4.0 + r * 1.5 * speed) as f32;
        let at = [c.position.x, c.position.y, c.position.z];
        // The cart is the explosion's source: it is gone before the blast
        // looks for entities, so it can't hit itself.
        if let Some(i) = self.carts.iter().position(|x| x.id == c.id) {
            self.eject_cart_mobs(i);
        }
        self.release_vehicle_container(c.id);
        self.carts.retain(|x| x.id != c.id);
        // ExplosionInteraction.TNT: the tnt_explosion_drop_decay rule.
        let interaction = self.destroy_type("tnt_explosion_drop_decay");
        // MinecartTNT.isPrimed (fuse > -1): a cart that ran into a wall or
        // fell unprimed blows rails up like any blast.
        self.combat.rail_proof_blast = c.fuse >= 0;
        self.combat.explosion_in_water = self.bounds_in_water(c.bounds());
        let _ = self.with_hive_emptying_blast(|g| g.server_explode(at, power, interaction, false));
        self.combat.explosion_in_water = false;
        self.combat.rail_proof_blast = false;
    }
    /// `AbstractMinecart.push` from each player walking into a cart
    /// (`LivingEntity.pushEntities`): the window's player and every server
    /// player (single player's own on its server thread, LAN guests). A
    /// player in a vehicle pushes nothing here. The quarter push the player
    /// gets back only moves the server's copy of it (`Entity.push` marks
    /// `needsSync`, which `ServerEntity` sends to the other players only),
    /// so no player feels it.
    fn push_carts(&mut self) {
        if self.carts.is_empty() {
            return;
        }
        // (feet, box)
        let mut pushers: Vec<(Vec3, Aabb)> = Vec::new();
        if self.riding.is_none()
            && let Some(p) = self.player.as_ref()
        {
            pushers.push((p.position, p.bounds()));
        }
        if let Some(host) = self.lan.as_ref() {
            for g in host.hub.guests.iter().filter(|g| {
                host.hub.here(g) && g.loaded && !g.dead && !g.spectator() && g.vehicle.is_none()
            }) {
                let [x, y, z] = g.position;
                pushers.push((Vec3::new(x, y, z), g.bounds()));
            }
        }
        for (feet, bb) in pushers {
            for c in &mut self.carts {
                if !bb.intersects(c.bounds()) {
                    continue;
                }
                let (mut xa, mut za) = (feet.x - c.position.x, feet.z - c.position.z);
                let dd = xa * xa + za * za;
                if dd < f64::from(1.0e-4f32) {
                    continue;
                }
                let d = dd.sqrt();
                let pow = (1.0 / d).min(1.0);
                xa = xa / d * pow * f64::from(0.1f32) * 0.5;
                za = za / d * pow * f64::from(0.1f32) * 0.5;
                c.velocity.x -= xa;
                c.velocity.z -= za;
            }
        }
    }
    /// `OldMinecartBehavior.pushAndPickupEntities` for carts, at the end of
    /// cart `i`'s own tick: each other cart in its box (0.2 wider on each
    /// side) gets `push(cart i)`. Running it per cart, after that cart moved
    /// and before the next one does, is what splits stacked carts apart.
    fn push_from_cart(&mut self, i: usize) {
        let reach = f64::from(0.2f32);
        let hitbox = self.carts[i].bounds().inflate(Vec3::new(reach, 0.0, reach));
        for j in 0..self.carts.len() {
            if j == i || !hitbox.intersects(self.carts[j].bounds()) {
                continue;
            }
            // AbstractMinecart.push(entity): `this` is cart j, `entity` cart i.
            let (this, other) = (&self.carts[j], &self.carts[i]);
            let (mut xa, mut za) = (
                other.position.x - this.position.x,
                other.position.z - this.position.z,
            );
            let dd = xa * xa + za * za;
            if dd < f64::from(1.0e-4f32) {
                continue;
            }
            let d = dd.sqrt();
            xa /= d;
            za /= d;
            let pow = (1.0 / d).min(1.0);
            xa = xa * pow * f64::from(0.1f32) * 0.5;
            za = za * pow * f64::from(0.1f32) * 0.5;
            // pushOtherMinecart (old movement): only along this cart's facing.
            let dir = glam::DVec2::new(
                other.position.x - this.position.x,
                other.position.z - this.position.z,
            )
            .normalize_or_zero();
            let r = f64::from(this.yaw * 0.017453292f32);
            let facing = glam::DVec2::new(f64::from(trig::cos(r)), f64::from(trig::sin(r)))
                .normalize_or_zero();
            if dir.dot(facing).abs() < f64::from(0.8f32) {
                continue;
            }
            let (mt, mo) = (this.velocity, other.velocity);
            let (ft, fo) = (
                this.kind == CartKind::Furnace,
                other.kind == CartKind::Furnace,
            );
            let (nt, no) = if fo && !ft {
                (
                    Vec3::new(mt.x * 0.2 + (mo.x - xa), mt.y, mt.z * 0.2 + (mo.z - za)),
                    Vec3::new(mo.x * 0.95, mo.y, mo.z * 0.95),
                )
            } else if !fo && ft {
                (
                    Vec3::new(mt.x * 0.95, mt.y, mt.z * 0.95),
                    Vec3::new(mo.x * 0.2 + (mt.x + xa), mo.y, mo.z * 0.2 + (mt.z + za)),
                )
            } else {
                let (xdd, zdd) = ((mo.x + mt.x) / 2.0, (mo.z + mt.z) / 2.0);
                (
                    Vec3::new(mt.x * 0.2 + (xdd - xa), mt.y, mt.z * 0.2 + (zdd - za)),
                    Vec3::new(mo.x * 0.2 + (xdd + xa), mo.y, mo.z * 0.2 + (zdd + za)),
                )
            };
            self.carts[j].velocity = nt;
            self.carts[i].velocity = no;
        }
        self.pickup_from_cart(i);
    }
    /// `OldMinecartBehavior.pushAndPickupEntities` for living entities in cart
    /// `i`'s box (0.2 wider on each side): only a rideable cart moving at
    /// `horizontalDistanceSqr() >= 0.01` picks up; an eligible mob boards an
    /// empty cart (`startRiding`); others get `Entity.push`.
    fn pickup_from_cart(&mut self, i: usize) {
        let v = self.carts[i].velocity;
        if self.carts[i].kind != CartKind::Rideable || v.x * v.x + v.z * v.z < 0.01 {
            return;
        }
        let reach = f64::from(0.2f32);
        let id = self.carts[i].id;
        let pos = self.carts[i].position;
        let hitbox = self.carts[i].bounds().inflate(Vec3::new(reach, 0.0, reach));
        let mut occupied = self.riding == Some(id)
            || self.ridden_by_guest(id)
            || self.mobs.iter().any(|m| m.extra.mount == Some(id));
        let teams = self.mob_teams();
        let candidates: Vec<u32> = (0..self.mobs.len())
            .filter(|&j| {
                let m = &self.mobs[j];
                if m.removed || !m.pushable() || m.extra.mount == Some(id) {
                    return false;
                }
                if !hitbox.intersects(m.bounds()) {
                    return false;
                }
                pushable_by(None, teams.of(Who::Mob(m.id)), true)
            })
            .map(|j| self.mobs[j].id)
            .collect();
        drop(teams);
        for mob in candidates {
            // By id: a mod's mount hook may have removed mobs since the list
            // was made, which moves the others' indices.
            let Some(j) = self.mobs.iter().position(|m| m.id == mob) else {
                continue;
            };
            let species = self.mobs[j].species();
            let player = &*self.mobs[j].kind.id == "minecraft:player";
            let cooldown = self.mobs[j].extra.boarding_cooldown;
            let passenger = self.mobs[j].extra.mount.is_some();
            if pickup_instead_of_push(species, passenger, occupied) && !player {
                // Entity.startRiding: a mod may cancel it (EntityMountEvent).
                if can_ride_cart(species, cooldown)
                    && self.native_mob_may_ride(mob, id, true)
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob)
                {
                    m.extra.mount = Some(id);
                    m.extra.passenger = true;
                    m.ai.nav.stop();
                    m.needs_sync = true;
                    occupied = true;
                    self.break_leash(mob, true, false);
                }
                continue;
            }
            let other = self.mobs[j].position;
            let Some(v) = super::boat_passengers::push_impulse(pos, other) else {
                continue;
            };
            if !occupied {
                self.carts[i].velocity = self.carts[i].velocity.subtract(v);
            }
            let m = &mut self.mobs[j];
            if m.pushable() {
                m.velocity = m.velocity.add_vector(v);
                m.needs_sync = true;
            }
        }
    }
    /// `positionRider` for the mobs in minecarts: each sits on the cart's
    /// passenger attachment, less its own vehicle attachment.
    pub(super) fn seat_cart_mobs(&mut self) {
        for i in 0..self.carts.len() {
            let id = self.carts[i].id;
            let (pos, prev) = (self.carts[i].position, self.carts[i].previous);
            let seats: Vec<(usize, f64)> = self
                .mobs
                .iter()
                .enumerate()
                .filter(|(_, m)| m.extra.mount == Some(id))
                .map(|(j, m)| (j, riders::vehicle_attachment(m)))
                .collect();
            for (j, offset) in seats {
                let m = &mut self.mobs[j];
                m.position = cart_seat(pos, offset);
                m.previous = cart_seat(prev, offset);
                m.velocity = Vec3::ZERO;
                m.fall_distance = 0.0;
                m.on_ground = false;
                m.extra.passenger = true;
                m.ai.nav.stop();
            }
        }
    }
    /// `ejectPassengers` for the mobs in cart `i`.
    pub(super) fn eject_cart_mobs(&mut self, i: usize) {
        let (id, y) = (self.carts[i].id, self.carts[i].position.y);
        let js: Vec<usize> = (0..self.mobs.len())
            .filter(|&j| self.mobs[j].extra.mount == Some(id))
            .collect();
        for j in js {
            let at = {
                let m = &self.mobs[j];
                Vec3::new(m.position.x, m.position.y.max(y), m.position.z)
            };
            riders::stop_riding(&mut self.mobs[j], Some(at));
            self.mobs[j].previous = at;
        }
    }
    /// Every cart but the ridden one (which ticks with its rider).
    pub(super) fn tick_carts(&mut self) {
        self.push_carts();
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        let mut round = self.native_tick_round();
        // A server player's minecart moves with its rider (lan_vehicles.rs).
        let ridden = self.guest_vehicles();
        // By id: an exploding cart can destroy or prime others mid-loop.
        let ids: Vec<u32> = self.carts.iter().map(|c| c.id).collect();
        for &id in &ids {
            let Some(i) = self.carts.iter().position(|c| c.id == id) else {
                continue;
            };
            let p = self.carts[i].position;
            if Some(id) == self.riding
                || ridden.contains(&id)
                || !ticking(p.x.floor() as i32, p.z.floor() as i32)
            {
                continue;
            }
            let kind = self.carts[i].kind.id();
            if !self.native_tick_pre_one(&mut round, id, kind) {
                continue;
            }
            let Some(i) = self.carts.iter().position(|c| c.id == id) else {
                continue;
            };
            let mut c = self.carts[i].clone();
            let alive = self.tick_cart(&mut c, [0.0; 2]);
            if c.kind == CartKind::Hopper && c.enabled {
                self.hopper_cart_suck(&mut c);
            }
            let Some(i) = self.carts.iter().position(|c| c.id == id) else {
                self.native_tick_post_one(&round, id, kind);
                continue;
            };
            if alive {
                let due = std::mem::take(&mut c.command_due);
                self.carts[i] = c;
                self.push_from_cart(i);
                if due {
                    self.perform_cart_command(id);
                }
            } else {
                self.eject_cart_mobs(i);
                self.release_vehicle_container(id);
                self.carts.remove(i);
            }
            // After the ticked cart is stored: a mod reads and changes the cart as it is now.
            self.native_tick_post_one(&round, id, kind);
        }
        // On the server for every cart, the ridden ones too.
        for id in ids {
            if round.canceled(id) {
                continue;
            }
            if let Some(i) = self.carts.iter().position(|c| c.id == id) {
                let p = self.carts[i].position;
                if ticking(p.x.floor() as i32, p.z.floor() as i32) {
                    self.cart_lava_tick(id);
                }
            }
        }
    }
    /// `lavaHurt` twice a tick in lava: from `LavaFluid.entityInside` in the
    /// behaviour's `applyEffectsFromBlocks`, then from `AbstractMinecart.tick`'s
    /// own `isInLava`. The second breaks the cart, or primes a TNT minecart
    /// (`MinecartTNT.destroy` for a fire source). A cart has no `baseTick`,
    /// so the fire `lavaIgnite` lights never burns it.
    fn cart_lava_tick(&mut self, id: u32) {
        for _ in 0..2 {
            let Some(i) = self.carts.iter().position(|c| c.id == id) else {
                return;
            };
            if !self.in_lava(self.carts[i].bounds()) {
                return;
            }
            let p = self.carts[i].position;
            self.hurt_cart(i, 4.0, false, true);
            self.lava_hurt_sound([p.x, p.y, p.z]);
        }
    }
    /// `ClientPacketListener.postAddEntitySoundInstance` for the carts that
    /// joined the level since the last tick, and `LocalPlayer.startRiding`'s
    /// two inside sounds when the player got in one; then what the sound
    /// instances read this tick (the ridden cart and every cart).
    pub(super) fn start_cart_sounds(&mut self) -> (Option<u32>, Vec<CartSound>) {
        let carts: Vec<CartSound> = self
            .carts
            .iter()
            .map(|c| CartSound {
                id: c.id,
                position: [c.position.x, c.position.y, c.position.z],
                // Vec3.horizontalDistance.
                speed: (c.velocity.x * c.velocity.x + c.velocity.z * c.velocity.z).sqrt(),
            })
            .collect();
        self.cart_sounds
            .retain(|id| carts.iter().any(|c| c.id == *id));
        for cart in &carts {
            if self.cart_sounds.insert(cart.id) {
                let instance = SoundInstance::minecart(cart, SoundEngine::unseeded());
                self.audio.play_instance(instance, false);
            }
        }
        let vehicle = self.riding.filter(|id| carts.iter().any(|c| c.id == *id));
        if vehicle != self.riding_cart_sounds
            && let Some(cart) = carts.iter().find(|c| Some(c.id) == vehicle)
        {
            for underwater in [true, false] {
                let instance =
                    SoundInstance::riding_minecart(cart, underwater, SoundEngine::unseeded());
                self.audio.play_instance(instance, false);
            }
        }
        self.riding_cart_sounds = vehicle;
        (vehicle, carts)
    }
    /// The rider's tick: the cart moves with the rider's input as its push.
    pub(super) fn tick_cart_riding(&mut self, keys: Keys) -> bool {
        let Some(i) = self.carts.iter().position(|c| Some(c.id) == self.riding) else {
            return false;
        };
        if keys.shift {
            self.dismount_cart(i);
            return true;
        }
        // ServerPlayer.getLastClientMoveIntent: getInputVector((left, 0, forward), 1, yRot).
        let left = f64::from(i8::from(keys.left) - i8::from(keys.right));
        let forward = f64::from(i8::from(keys.forward) - i8::from(keys.backward));
        let len = left.hypot(forward);
        let (left, forward) = if len > 1.0 {
            (left / len, forward / len)
        } else {
            (left, forward)
        };
        let r = self.yaw.to_radians();
        let (s, co) = (f64::from(r.sin()), f64::from(r.cos()));
        let intent = [left * co - forward * s, forward * co + left * s];
        let mut c = self.carts[i].clone();
        let alive = self.tick_cart(&mut c, intent);
        if let Some(p) = self.player.as_mut() {
            self.previous_position = p.position;
            p.position = Vec3::new(c.position.x, c.position.y + SEAT, c.position.z);
            p.velocity = Vec3::ZERO;
            p.fall_distance = 0.0;
        }
        let Some(i) = self.carts.iter().position(|x| x.id == c.id) else {
            self.riding = None;
            return true;
        };
        if alive {
            self.carts[i] = c;
            self.push_from_cart(i);
        } else {
            self.eject_cart_mobs(i);
            self.release_vehicle_container(c.id);
            self.carts.remove(i);
            self.riding = None;
        }
        // AbstractMinecart.activateMinecart: a powered activator rail ejects
        // the rider, and getDismountLocationForPassenger places it beside the
        // cart instead of leaving it at the seat.
        if alive && self.riding.is_none() {
            self.dismount_cart(i);
        }
        true
    }
    /// `getDismountLocationForPassenger`: beside the cart (across its facing), else on top.
    pub(super) fn dismount_cart(&mut self, i: usize) {
        let id = self.carts[i].id;
        if self.riding == Some(id) && !self.native_may_leave(id) {
            return;
        }
        self.riding = None;
        let c = &self.carts[i];
        let (bx, by, bz) = (
            c.position.x.floor() as i32,
            c.position.y.floor() as i32,
            c.position.z.floor() as i32,
        );
        let mut target = None;
        // DismountHelper.offsetsForDirection of the cart's facing
        // (Direction.fromYRot of its yaw): right, left, back right, back left,
        // forward right, forward left, back, forward.
        let step = |q: i32| [(0, 1), (-1, 0), (0, -1), (1, 0)][(q & 3) as usize];
        let facing = (f64::from(c.yaw) / 90.0 + 0.5).floor() as i32;
        let (fx, fz) = step(facing);
        let (rx, rz) = (-fz, fx);
        let offsets = [
            (rx, rz),
            (-rx, -rz),
            (rx - fx, rz - fz),
            (-rx - fx, -rz - fz),
            (fx + rx, fz + rz),
            (fx - rx, fz - rz),
            (-fx, -fz),
            (fx, fz),
        ];
        'search: for dy in [0, 1, -1] {
            for (dx, dz) in offsets {
                let (x, y, z) = (bx + dx, by + dy, bz + dz);
                let below = self.res.facts().get(self.world.block(x, y - 1, z));
                let here = self.res.facts().get(self.world.block(x, y, z));
                let head = self.res.facts().get(self.world.block(x, y + 1, z));
                if (below.full_collision || below.legacy_solid)
                    && !here.full_collision
                    && !head.full_collision
                    && here.fluid == Fluid::None
                {
                    target = Some(Vec3::new(
                        f64::from(x) + 0.5,
                        f64::from(y),
                        f64::from(z) + 0.5,
                    ));
                    break 'search;
                }
            }
        }
        let top = c.bounds().max_position().y;
        let target = target.unwrap_or(Vec3::new(c.position.x, top, c.position.z));
        if let Some(p) = self.player.as_mut() {
            p.position = target;
            p.velocity = Vec3::ZERO;
        }
    }
    /// `MinecartItem.useOn`: a cart on the clicked rail.
    pub(super) fn place_minecart(&mut self, hit: &Hit, stack: &ItemStack) -> bool {
        let Some(kind) = CartKind::from_id(&stack.item) else {
            return false;
        };
        let Some(rail) = self.rail(hit.pos[0], hit.pos[1], hit.pos[2]) else {
            return false;
        };
        let offset = if rail.shape.is_slope() { 0.5 } else { 0.0 };
        let at = Vec3::new(
            f64::from(hit.pos[0]) + 0.5,
            f64::from(hit.pos[1]) + 0.0625 + offset,
            f64::from(hit.pos[2]) + 0.5,
        );
        self.next_entity += 1;
        let cart = Minecart::new(self.next_entity, kind, at);
        self.world
            .modified
            .insert((hit.pos[0] >> 4, hit.pos[2] >> 4));
        self.carts.push(cart);
        if !self.creative {
            self.consume_held();
        }
        self.swing_arm();
        true
    }
    pub(super) fn cart_hit(&self) -> Option<(usize, f64)> {
        let (eye, dir) = self.aim()?;
        let reach: f64 = self.entity_reach;
        let mut best: Option<(usize, f64)> = None;
        for (i, c) in self.carts.iter().enumerate() {
            if Some(c.id) == self.riding {
                continue;
            }
            let bb = c.bounds();
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
    /// The slots of the cart `id`, for a command: `getChestVehicleItem` and
    /// `setChestVehicleItem` first unpack its loot table with no player, so
    /// no `player_generates_container_loot` fires later. While its menu is
    /// open the live slots are the placeholder copy (`open_cart_container`).
    pub(super) fn cart_slots(&mut self, id: u32) -> Option<&mut Vec<Option<ItemStack>>> {
        let c = self.carts.iter_mut().find(|c| c.id == id)?;
        c.loot = None;
        match self
            .world
            .block_entities
            .get_mut(&(i32::MAX - 15, CART_KEY_Y, id as i32))
        {
            Some(BlockEntity::Chest(open)) => Some(open),
            _ => Some(&mut c.items),
        }
    }
    /// `ContainerEntity.unpackChestVehicleLootTable` for a player who opens a
    /// chest minecart: a roll it still holds fires `player_generates_container_loot`
    /// for its table, and the cart keeps its items from then on.
    fn unpack_cart_loot(&mut self, i: usize) {
        let Some(loot) = self.carts[i].loot.take() else {
            return;
        };
        if loot.items == horizon_client::save::encode_items(&self.carts[i].items) {
            self.trigger_loot_table(&loot.table);
        }
    }
    /// `interact`: ride, open, fuel or ignite the cart under the crosshair, or
    /// the one a guest's interact packet named.
    pub(super) fn interact_cart(&mut self, target: Option<InteractTarget>) -> bool {
        let Some((i, d)) = (match target {
            Some(t) => Self::named_entity(&self.carts, t.id, |c| c.id).map(|i| (i, 0.0)),
            None => self.cart_hit(),
        }) else {
            return false;
        };
        // Only a client's own pick is stopped by a block: the range check passed.
        if target.is_none() && !self.before_block(d) {
            return false;
        }
        let sneaking = self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft);
        let held = self.held().cloned();
        match self.carts[i].kind {
            CartKind::Rideable => {
                let id = self.carts[i].id;
                // isVehicle: a mob already sits in it.
                let has_mob = self.mobs.iter().any(|m| m.extra.mount == Some(id));
                if sneaking || self.riding.is_some() || self.ridden_by_guest(id) || has_mob {
                    return false;
                }
                if !self.native_may_board(id) {
                    return true;
                }
                self.riding = Some(id);
            }
            // AbstractMinecartContainer.interact: no sneak check.
            CartKind::Chest | CartKind::Hopper => self.open_cart_container(i),
            CartKind::Furnace => {
                let fuel = held
                    .as_ref()
                    .is_some_and(|s| matches!(&*s.item, "minecraft:coal" | "minecraft:charcoal"));
                let c = &mut self.carts[i];
                if fuel && c.fuel + 3600 <= 32000 {
                    c.fuel += 3600;
                    let p = self.player.as_ref().map_or(c.position, |p| p.position);
                    c.push = Vec3::new(c.position.x - p.x, 0.0, c.position.z - p.z);
                    if !self.creative {
                        self.consume_held();
                    }
                }
            }
            CartKind::Tnt => {
                let lighter = held.as_ref().is_some_and(|s| {
                    matches!(
                        &*s.item,
                        "minecraft:flint_and_steel" | "minecraft:fire_charge"
                    )
                });
                if !lighter || self.carts[i].fuse >= 0 {
                    return false;
                }
                let mut c = self.carts[i].clone();
                self.prime_cart(&mut c);
                self.carts[i] = c;
            }
            // MinecartCommandBlock.interact: a game master gets SUCCESS (the
            // client opens the editor, `open_cart_command_block`), anyone else
            // PASSes to the held item. SUCCESS swings on the client only.
            CartKind::CommandBlock => return self.can_use_game_master_blocks(),
            // MinecartSpawner has no interact of its own (`Entity.interact` passes).
            CartKind::Spawner => return false,
        }
        self.swing_arm();
        true
    }
    /// `AbstractMinecartContainer.interact`: the menu opens over the cart's
    /// own slots, kept under a placeholder key until it closes
    /// (`store_cart_items`/`close_cart_container`).
    pub(super) fn open_cart_container(&mut self, i: usize) {
        self.unpack_cart_loot(i);
        let c = self.carts[i].clone();
        let key = (i32::MAX - 15, CART_KEY_Y, c.id as i32);
        self.world
            .block_entities
            .insert(key, BlockEntity::Chest(c.items.clone()));
        self.container = Some(key);
        self.screen = super::Screen::Inventory;
        self.capture(false);
        self.right = false;
        if c.kind == CartKind::Chest {
            // `MinecartChest.interact` (which overrides
            // `AbstractMinecartContainer.interact`): a successful open fires
            // CONTAINER_OPEN with the player and angers the piglins that see
            // it. `MinecartHopper` does not override it, so a hopper minecart
            // fires neither. Vanilla plays no open sound for a chest minecart.
            let at = [c.position.x, c.position.y, c.position.z];
            let ctx = self.player_context(None);
            self.post_game_event("minecraft:container_open", at, ctx);
            self.anger_piglins(true);
        }
    }
    /// Closing the cart's container open under `key` copies the items back.
    pub(super) fn close_cart_container(&mut self, key: (i32, i32, i32)) {
        if key.1 != CART_KEY_Y {
            return;
        }
        self.store_cart_items(key);
        self.world.block_entities.remove(&key);
    }
    /// A chest or hopper minecart or a chest boat is about to be removed
    /// (`Containers.dropContents` / `AbstractChestBoat.remove`): what an open
    /// menu edited goes back to the vehicle, so it is what drops or travels
    /// with it, and the menu's stand-in leaves the level. A menu that is
    /// still open over it then has nothing left to take.
    pub(super) fn release_vehicle_container(&mut self, id: u32) {
        let key = (i32::MAX - 15, CART_KEY_Y, id as i32);
        self.store_cart_items(key);
        self.world.block_entities.remove(&key);
        if self.container == Some(key) {
            self.container = None;
        }
    }
    /// The menu of a chest or hopper minecart or a chest boat works on the
    /// vehicle's own items (`ContainerEntity`): the placeholder under `key`
    /// that the menu edits is copied back to the vehicle.
    pub(super) fn store_cart_items(&mut self, key: (i32, i32, i32)) {
        if key.1 != CART_KEY_Y {
            return;
        }
        let Some(BlockEntity::Chest(items)) = self.world.block_entities.get(&key) else {
            return;
        };
        let items = items.clone();
        if let Some(c) = self.carts.iter_mut().find(|c| c.id as i32 == key.2) {
            c.items = items;
        } else if let Some(b) = self.boats.iter_mut().find(|b| b.id as i32 == key.2) {
            b.items = items;
        }
    }
    /// The open cart's kind when a cart container is shown.
    pub(super) fn open_cart_kind(&self) -> Option<CartKind> {
        let key = self.container.filter(|k| k.1 == CART_KEY_Y)?;
        // A chest boat's chest shows like a chest minecart's.
        self.carts
            .iter()
            .find(|c| c.id as i32 == key.2)
            .map(|c| c.kind)
            .or_else(|| {
                self.boats
                    .iter()
                    .any(|b| b.chest && b.id as i32 == key.2)
                    .then_some(CartKind::Chest)
            })
    }
    /// `VehicleEntity.hurtServer` from the player's attack.
    pub(super) fn attack_cart(&mut self) -> bool {
        let Some((i, d)) = self.cart_hit() else {
            return false;
        };
        if !self.before_block(d) || self.mob_hit().is_some_and(|m| m.1 < d) {
            return false;
        }
        let burning = self
            .held()
            .is_some_and(|s| s.enchantment_level("minecraft:fire_aspect") > 0);
        let area = self.carts[i].bounds();
        // Player.attack's full damage against the cart's entity type.
        let id = self.carts[i].id as i32;
        let (damage, _, _, magic) = self.melee_damage(self.carts[i].kind.id(), false);
        self.hurt_cart(i, damage, self.creative, burning);
        self.vehicle_attack_sounds(area, magic.then_some(id));
        // Player.onAttack: the hit starts the attack strength again.
        self.attack_ticks = 0;
        self.swing_arm();
        true
    }
    /// An arrow hit cart `i` (`MinecartTNT.hurtServer`, then
    /// `VehicleEntity.hurtServer`): a burning arrow sets off a TNT minecart.
    pub(super) fn arrow_hits_cart(
        &mut self,
        i: usize,
        damage: f32,
        arrow_velocity: Vec3,
        burning: bool,
        creative: bool,
    ) {
        if i >= self.carts.len() {
            return;
        }
        if burning && self.carts[i].kind == CartKind::Tnt {
            self.eject_cart_mobs(i);
            let id = self.carts[i].id;
            self.release_vehicle_container(id);
            let c = self.carts.remove(i);
            self.explode_cart(&c, arrow_velocity.length_squared());
            return;
        }
        self.hurt_cart(i, damage, creative, false);
    }
    /// `VehicleEntity.hurtServer`: shake, add the damage, break past 40 (at
    /// once for creative players); TNT minecarts prime instead when moving
    /// or hurt by fire or an explosion (`damageSourceIgnitesTnt`).
    pub(super) fn hurt_cart(&mut self, i: usize, damage: f32, creative: bool, fire: bool) {
        let destroyed = {
            let c = &mut self.carts[i];
            c.hurt_dir = -c.hurt_dir;
            c.hurt_time = 10;
            c.damage += damage * 10.0;
            creative || c.damage > 40.0
        };
        if destroyed {
            let speed = self.carts[i].velocity.x * self.carts[i].velocity.x
                + self.carts[i].velocity.z * self.carts[i].velocity.z;
            let tnt_prime =
                self.carts[i].kind == CartKind::Tnt && (fire || speed >= f64::from(0.01f32));
            if !tnt_prime {
                self.eject_cart_mobs(i);
            }
            let id = self.carts[i].id;
            if !tnt_prime {
                self.release_vehicle_container(id);
            }
            let mut c = self.carts.remove(i);
            if c.kind == CartKind::Tnt && (fire || speed >= f64::from(0.01f32)) {
                // MinecartTNT.destroy: prime instead of breaking.
                if c.fuse < 0 {
                    self.prime_cart(&mut c);
                    let r = &mut self.world.random;
                    c.fuse = r.next_int_bound(20).unwrap_or(0) + r.next_int_bound(20).unwrap_or(0);
                }
                self.carts.insert(i, c);
            } else {
                let at = [c.position.x, c.position.y, c.position.z];
                if self.ui.rules.bool("entity_drops") {
                    for s in c.items.iter().flatten() {
                        self.spawn_item(s.clone(), at, [0.0, 0.1, 0.0], 10);
                    }
                    if !creative {
                        self.spawn_item(ItemStack::new(c.kind.item(), 1), at, [0.0, 0.1, 0.0], 10);
                    }
                }
            }
        }
    }
    /// Baked block-state faces with default block tints and terrain shade or diffuse lighting.
    pub(super) fn block_model_vertices(
        &self,
        state: BlockStateId,
        seed: i64,
        m: glam::Mat4,
        light: u32,
        diffuse: bool,
        out: &mut Vec<Vertex>,
    ) {
        // BlockTintSource.color(state) with no level: each tinted face's default colour.
        let grass = self.pack.item_renderer.grass_color(0.5, 1.0) as u32;
        let tables = &self.pack.context.tables;
        let tint = |index: i32| tables.default_tint(state, index, grass);
        self.tinted_block_model_vertices(state, seed, m, |_| light, diffuse, tint, out);
    }
    /// `block_model_vertices` with each face multiplied by `tint` of its
    /// tint index (RGB, `None` for untinted faces) and lit by `light` of
    /// each quad.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn tinted_block_model_vertices(
        &self,
        state: BlockStateId,
        seed: i64,
        m: glam::Mat4,
        light: impl Fn(&horizon_assets::bake::ModelQuad) -> u32,
        diffuse: bool,
        tint: impl Fn(i32) -> Option<u32>,
        out: &mut Vec<Vertex>,
    ) {
        let Ok(models) = self.pack.render.model_catalog.collect_seeded(state, seed) else {
            return;
        };
        let models = models.into_iter().map(|(_, model)| model);
        self.baked_model_vertices(models, m, light, diffuse, tint, out);
    }
    /// `tinted_block_model_vertices` for baked models given as they are (a
    /// state's, or a standalone model).
    pub(super) fn baked_model_vertices<'m>(
        &self,
        models: impl IntoIterator<Item = &'m horizon_client::models::catalog::BakedModel>,
        m: glam::Mat4,
        light: impl Fn(&horizon_assets::bake::ModelQuad) -> u32,
        diffuse: bool,
        tint: impl Fn(i32) -> Option<u32>,
        out: &mut Vec<Vertex>,
    ) {
        for model in models {
            for face in &model.faces {
                let Some(s) = self
                    .pack
                    .layer_names
                    .get(&face.layer)
                    .and_then(|n| self.gui.atlas.sprites.get(n))
                else {
                    continue;
                };
                use horizon_assets::model::Direction as D;
                let shade: u32 = match face.quad.facing {
                    D::Up => 255,
                    D::Down => 127,
                    D::North | D::South => 204,
                    D::West | D::East => 153,
                };
                let mut color = if diffuse {
                    u32::MAX
                } else {
                    0xFF00_0000 | shade << 16 | shade << 8 | shade
                };
                if let Some(t) = tint(face.quad.tint_index) {
                    color = horizon_client::hud_layout::multiply(
                        color,
                        horizon_client::mobs::render::abgr(t),
                    );
                }
                let normal = if diffuse {
                    use horizon_client::item_render::{direction_vector, transform_normal};
                    let n = transform_normal(m, direction_vector(face.quad.facing));
                    let n = n.to_array().map(|v| (v.clamp(-1.0, 1.0) * 127.0) as i8);
                    [n[0], n[1], n[2], 127]
                } else {
                    [0; 4]
                };
                let light = light(&face.quad);
                let light = if diffuse {
                    let emission = u32::from(face.quad.light_emission) * 16;
                    (light & 0xFF) | (light >> 8 & 0xFF).max(emission) << 8
                } else {
                    light
                };
                for k in 0..4 {
                    let p = face.quad.positions[k];
                    let uv = face.quad.uv[k];
                    out.push(Vertex {
                        normal,
                        pos: m.transform_point3(glam::Vec3::from_array(p)).to_array(),
                        uv: [s.u0 + uv[0] * (s.u1 - s.u0), s.v0 + uv[1] * (s.v1 - s.v0)],
                        layer: 0,
                        color,
                        light,
                        glint_uv: [0.0, 0.0],
                    });
                }
            }
        }
    }
    fn display_state(&self, c: &Minecart) -> Option<BlockStateId> {
        let registry = &self.pack.assets.registry;
        let with = |name: &str, props: &[(&str, &str)]| {
            let mut s = registry.state(registry.default_state(name)?)?.clone();
            for (k, v) in props {
                s.properties.insert((*k).to_owned(), (*v).to_owned());
            }
            registry.id(&s)
        };
        match c.kind {
            CartKind::Rideable => None,
            CartKind::Chest => with("minecraft:chest", &[("facing", "north")]),
            CartKind::Hopper => registry.default_state("minecraft:hopper"),
            CartKind::Furnace => with(
                "minecraft:furnace",
                &[
                    ("facing", "north"),
                    ("lit", if c.fuel > 0 { "true" } else { "false" }),
                ],
            ),
            CartKind::Tnt => registry.default_state("minecraft:tnt"),
            CartKind::CommandBlock => registry.default_state("minecraft:command_block"),
            // MinecartSpawner.getDefaultDisplayBlockState.
            CartKind::Spawner => registry.default_state("minecraft:spawner"),
        }
    }
    /// `AbstractMinecartRenderer` (old render): on the rail between the
    /// front and back points, pitched on slopes, with the display block.
    pub(super) fn cart_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        let a = alpha as f32;
        let Some(uv) = self
            .gui
            .atlas
            .sprites
            .get("entity/minecart/minecart")
            .map(|r| [r.u0, r.v0, r.u1, r.v1])
        else {
            return;
        };
        // Entity.shouldRenderAtSqrDistance: 64 times the bounding box's mean
        // edge, scaled by the view scale.
        let range = (WIDTH + HEIGHT + WIDTH) / 3.0 * 64.0 * self.view_scale();
        for c in &self.carts {
            let entity_start = out.len();
            let Some(model) = self.cart_models.models.get(&c.kind) else {
                continue;
            };
            let (x, y, z) = (
                c.previous.x + (c.position.x - c.previous.x) * alpha,
                c.previous.y + (c.position.y - c.previous.y) * alpha,
                c.previous.z + (c.position.z - c.previous.z) * alpha,
            );
            // LevelExtractor.isEntityVisible: the cart the player rides always draws.
            if self.riding != Some(c.id) {
                let d = glam::DVec3::new(c.position.x, c.position.y, c.position.z) - camera;
                if d.length_squared() >= range * range {
                    continue;
                }
                // EntityRenderer.shouldRender: the interpolated box grown by 0.5;
                // AbstractMinecartRenderer.getBoundingBoxForCulling stretches it
                // up by the display block's offset.
                if let Some(view) = &self.view_frustum {
                    let top = HEIGHT
                        + if c.kind == CartKind::Rideable {
                            0.0
                        } else {
                            display_offset(c.kind) * 0.75 / 16.0
                        };
                    let p = glam::DVec3::new(x, y, z) - camera;
                    let lo = p - glam::DVec3::new(WIDTH / 2.0 + 0.5, 0.5, WIDTH / 2.0 + 0.5);
                    let hi = p + glam::DVec3::new(WIDTH / 2.0 + 0.5, top + 0.5, WIDTH / 2.0 + 0.5);
                    if !view.intersects(lo.as_vec3(), hi.as_vec3()) {
                        continue;
                    }
                }
            }
            let mut yaw =
                c.previous_yaw + ((c.yaw - c.previous_yaw + 180.0).rem_euclid(360.0) - 180.0) * a;
            let mut pitch = 0.0f32;
            let mut offset = glam::DVec3::ZERO;
            if let Some(on) = self.rail_pos(x, y, z) {
                let front = self.rail_pos_offs(x, y, z, 0.3).unwrap_or(on);
                let back = self.rail_pos_offs(x, y, z, -0.3).unwrap_or(on);
                offset = glam::DVec3::new(on[0] - x, (front[1] + back[1]) / 2.0 - y, on[2] - z);
                let d =
                    glam::DVec3::new(back[0] - front[0], back[1] - front[1], back[2] - front[2]);
                if d.length() != 0.0 {
                    let d = d.normalize();
                    yaw = d.z.atan2(d.x).to_degrees() as f32;
                    pitch = (d.y.atan() * 73.0) as f32;
                }
            }
            // The per-entity jitter (offsetSeed) that keeps stacked carts from z-fighting.
            let seed = i64::from(c.id).wrapping_mul(493_286_711);
            let seed = seed
                .wrapping_mul(seed)
                .wrapping_mul(4_392_167_121)
                .wrapping_add(seed.wrapping_mul(98_761));
            let jitter = |shift: i64| (((seed >> shift & 7) as f32 + 0.5) / 8.0 - 0.5) * 0.004;
            let p = glam::DVec3::new(x, y, z) + offset - camera;
            let mut base = glam::Mat4::from_translation(
                p.as_vec3() + glam::Vec3::new(jitter(16), jitter(20) + 0.375, jitter(24)),
            ) * glam::Mat4::from_rotation_y((180.0 - yaw).to_radians())
                * glam::Mat4::from_rotation_z((-pitch).to_radians());
            let hurt = c.hurt_time as f32 - a;
            if hurt > 0.0 {
                let damage = (c.damage - a).max(0.0);
                base *= glam::Mat4::from_rotation_x(
                    (hurt.sin() * hurt * damage / 10.0 * c.hurt_dir as f32).to_radians(),
                );
            }
            let l = self.world.light(
                c.position.x.floor() as i32,
                (c.position.y + 0.35).floor() as i32,
                c.position.z.floor() as i32,
            );
            let packed = (u32::from(l >> 4) * 16) | ((u32::from(l & 15) * 16) << 8);
            if let Some(state) = self.display_state(c) {
                let display = base
                    * glam::Mat4::from_scale(glam::Vec3::splat(0.75))
                    * glam::Mat4::from_translation(glam::Vec3::new(
                        -0.5,
                        (display_offset(c.kind) as f32 - 8.0) / 16.0,
                        0.5,
                    ))
                    * glam::Mat4::from_rotation_y(90f32.to_radians());
                if c.kind == CartKind::Chest {
                    self.special_block_model(state, display, l, out);
                } else {
                    // TNT minecarts flash white as the fuse runs out (TntMinecartRenderer).
                    let swell = if c.kind == CartKind::Tnt && c.fuse > -1 {
                        let fuse = c.fuse as f32 - a + 1.0;
                        if fuse < 10.0 {
                            let g = (1.0 - fuse / 10.0).clamp(0.0, 1.0);
                            1.0 + g * g * g * g * 0.3
                        } else {
                            1.0
                        }
                    } else {
                        1.0
                    };
                    let m = display
                        * glam::Mat4::from_translation(glam::Vec3::splat(0.5))
                        * glam::Mat4::from_scale(glam::Vec3::splat(swell))
                        * glam::Mat4::from_translation(glam::Vec3::splat(-0.5));
                    let flash = c.kind == CartKind::Tnt && c.fuse > -1 && (c.fuse / 5) % 2 == 0;
                    let start = out.len();
                    self.block_model_vertices(state, 0, m, packed, false, out);
                    if flash {
                        // submitWhiteSolidBlock: OverlayTexture.u(1.0) on the cart's own light.
                        for v in &mut out[start..] {
                            v.layer = 5;
                        }
                    }
                }
            }
            let m = base * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0));
            let poses = Poses::new(model);
            horizon_client::mobs::render::draw(model, m, &poses, uv, 0xFFFF_FFFF, packed, 0, out);
            self.cull_entity_vertices(out, entity_start);
        }
    }
    /// The model of a block drawn by its block-entity renderer, as a chest
    /// minecart shows its chest and a block display its special block.
    pub(super) fn special_block_model(
        &self,
        state: BlockStateId,
        m: glam::Mat4,
        light: u8,
        out: &mut Vec<Vertex>,
    ) {
        let lid = |_: [i32; 3]| 0.0;
        let scene = horizon_client::block_entity_render::Scene {
            camera: glam::DVec3::ZERO,
            orientation: glam::Quat::IDENTITY,
            anim: &|_| horizon_client::block_entity_render::BlockAnim::default(),
            texts: None,
            partial: 0.0,
            game_time: self.world.game_time,
            max_y: self.world.shape.max_y() - 1,
            world: &self.world,
            sprites: &self.gui.atlas.sprites,
            emissive: &self.gui.atlas.emissive,
            nether: self.dim == super::Dim::Nether,
            lid: &lid,
            sign: &|_| None,
            banner: &|_| Vec::new(),
            skull: &|_| None,
            glyph: &|c| self.gui.atlas.glyph_cell(c),
            glyphs: self.gui.atlas.font.epoch(),
        };
        self.res
            .block_entity_render()
            .render_state(&scene, state.0, m, light, out);
    }
}
/// Saving and loading.
impl Minecart {
    pub fn to_nbt(
        &self,
        uuid: [i32; 4],
        raw: Option<&horizon_world::nbt::Tag>,
    ) -> horizon_world::nbt::Tag {
        use horizon_world::nbt::{JavaString, Tag};
        let mut tag = horizon_client::entity_nbt::vehicle_to_nbt(
            self.kind.id(),
            self.position,
            self.velocity,
            self.yaw,
            uuid,
            raw,
        );
        if let Tag::Compound(map) = &mut tag {
            if self.kind == CartKind::CommandBlock {
                for key in ["LastOutput", "LastExecution"] {
                    map.remove(&JavaString::from(key));
                }
            }
            let mut put = |k: &str, v: Tag| {
                map.insert(JavaString::from(k), v);
            };
            put("FlippedRotation", Tag::Byte(i8::from(self.flipped)));
            // AbstractMinecart saves firstTick as HasTicked. A cart never burns:
            // Entity.move sets -getFireImmuneTicks(), 0 for all but players.
            put("HasTicked", Tag::Byte(i8::from(self.first_tick)));
            put("Fire", Tag::Short(0));
            match self.kind {
                CartKind::Chest | CartKind::Hopper => {
                    put("Items", horizon_client::save::encode_items(&self.items));
                    if self.kind == CartKind::Hopper {
                        put("Enabled", Tag::Byte(i8::from(self.enabled)));
                    }
                }
                CartKind::Furnace => {
                    put("PushX", Tag::Double(self.push.x.to_bits()));
                    put("PushZ", Tag::Double(self.push.z.to_bits()));
                    put("Fuel", Tag::Short(self.fuel as i16));
                }
                CartKind::Tnt => put("fuse", Tag::Int(self.fuse)),
                // BaseCommandBlock.save (its CustomName shares the entity's key).
                CartKind::CommandBlock => {
                    for (key, value) in self.command.save() {
                        if !matches!(key, "CustomName" | "powered" | "conditionMet" | "auto") {
                            put(key, value);
                        }
                    }
                }
                // BaseSpawner.save: the delays, ranges, SpawnData and SpawnPotentials.
                CartKind::Spawner => {
                    if let Some(spawner) = &self.spawner {
                        for (key, value) in
                            horizon_client::mobs::spawner::SpawnerExt::save(&**spawner)
                        {
                            put(key, value);
                        }
                    }
                }
                CartKind::Rideable => {}
            }
            // A loaded loot table goes back while the items are still its roll;
            // otherwise the items replace it, as vanilla's unpackLootTable does.
            if !self.loot.as_ref().is_some_and(|l| l.restore(map)) {
                map.remove(&JavaString::from("LootTable"));
                map.remove(&JavaString::from("LootTableSeed"));
            }
        }
        tag
    }
    pub fn from_nbt(tag: &horizon_world::nbt::Tag, id: u32, kind: CartKind) -> Option<Self> {
        use horizon_world::nbt::Tag;
        let (position, velocity, yaw) = horizon_client::entity_nbt::boat_from_nbt(tag)?;
        let mut c = Self::new(id, kind, position);
        c.velocity = velocity;
        c.yaw = yaw;
        c.previous_yaw = yaw;
        c.no_gravity = horizon_client::entity_nbt::flag(tag, "NoGravity");
        let int = |k: &str| match tag.get(k) {
            Some(Tag::Byte(v)) => Some(i32::from(*v)),
            Some(Tag::Short(v)) => Some(i32::from(*v)),
            Some(Tag::Int(v)) => Some(*v),
            _ => None,
        };
        let double = |k: &str| match tag.get(k) {
            Some(Tag::Double(v)) => f64::from_bits(*v),
            _ => 0.0,
        };
        c.flipped = int("FlippedRotation").unwrap_or(0) != 0;
        c.first_tick = int("HasTicked").unwrap_or(0) != 0;
        if kind.slots() > 0 {
            c.items = horizon_client::save::decode_items(tag.get("Items"), kind.slots());
        }
        c.enabled = int("Enabled").unwrap_or(1) != 0;
        c.fuel = int("Fuel").unwrap_or(0);
        // readAdditionalSaveData: the saved push has no y.
        c.push = Vec3::new(double("PushX"), 0.0, double("PushZ"));
        c.fuse = int("fuse").unwrap_or(-1);
        if kind == CartKind::CommandBlock {
            c.command = horizon_client::command_block::CommandBlock::load(tag);
            // readAdditionalSaveData sets both synched values.
            c.synced_command.clone_from(&c.command.command);
            c.synced_output = c.command.last_output_string();
        }
        // MinecartSpawner.readAdditionalSaveData: BaseSpawner.load.
        if kind == CartKind::Spawner {
            c.spawner = Some(Box::new(horizon_client::mobs::spawner::SpawnerExt::load(
                tag,
            )));
        }
        Some(c)
    }
}

/// `MinecartTNT.tick`'s collision detonation: a cart still moving at a tenth
/// of a block a tick or more when a block stops it blows up.
fn tnt_ignites_by_collision(horizontal_collision: bool, speed_sqr: f64) -> bool {
    horizontal_collision && speed_sqr >= 0.009_999_999_776_482_582
}

#[cfg(test)]
mod tests {
    /// The collision detonation of a TNT minecart needs both the collision and
    /// a cart that was still moving: a nudge below a tenth of a block a tick
    /// does not set it off, and a fast cart on an open track does not either.
    #[test]
    fn a_tnt_cart_detonates_only_when_a_collision_stops_a_moving_one() {
        assert!(tnt_ignites_by_collision(true, 0.01));
        assert!(tnt_ignites_by_collision(true, 0.36), "three tenths a tick");
        assert!(!tnt_ignites_by_collision(true, 0.0), "stopped at the wall");
        assert!(!tnt_ignites_by_collision(true, 0.009), "a nudge");
        assert!(!tnt_ignites_by_collision(false, 0.36), "nothing in the way");
    }
    use super::*;

    #[test]
    fn a_tnt_cart_lets_the_held_item_be_used() {
        // Entity.interact passes: a crossbow aimed at it shoots.
        assert!(CartKind::Tnt.interact_passes(false, false, true));
        assert!(CartKind::CommandBlock.interact_passes(false, false, false));
        assert!(!CartKind::CommandBlock.interact_passes(false, false, true));
        assert!(!CartKind::Rideable.interact_passes(false, false, false));
        assert!(CartKind::Rideable.interact_passes(true, false, false));
        assert!(CartKind::Rideable.interact_passes(false, true, false));
        for kind in [CartKind::Chest, CartKind::Hopper, CartKind::Furnace] {
            assert!(!kind.interact_passes(true, true, false));
        }
    }

    #[test]
    fn display_offset_matches_vanilla_for_each_cart_kind() {
        assert_eq!(display_offset(CartKind::Hopper), 1.0);
        assert_eq!(display_offset(CartKind::Chest), 6.0);
        assert_eq!(display_offset(CartKind::Furnace), 6.0);
        assert_eq!(display_offset(CartKind::Tnt), 6.0);
        assert_eq!(display_offset(CartKind::CommandBlock), 6.0);
        assert_eq!(display_offset(CartKind::Rideable), 6.0);
    }

    /// `MinecartSpawner`: its `BaseSpawner` is saved and loaded with the cart (the delay, the
    /// ranges and the `SpawnData`), it drops and is picked as a plain minecart, and it shows a
    /// spawner block.
    #[test]
    fn a_spawner_cart_keeps_its_spawner_and_drops_a_minecart() {
        use horizon_world::nbt::Tag;
        let saved = Tag::compound([
            ("id", Tag::String("minecraft:spawner_minecart".into())),
            (
                "Pos",
                Tag::List(
                    [0.5f64, 40.5, 0.5]
                        .map(|v| Tag::Double(v.to_bits()))
                        .to_vec(),
                ),
            ),
            ("Delay", Tag::Short(37)),
            ("MaxSpawnDelay", Tag::Short(120)),
            ("SpawnRange", Tag::Short(2)),
            (
                "SpawnData",
                Tag::compound([(
                    "entity",
                    Tag::compound([("id", Tag::String("minecraft:zombie".into()))]),
                )]),
            ),
        ]);
        let mut cart = Minecart::from_nbt(&saved, 1, CartKind::Spawner).unwrap();
        let spawner = cart.spawner.as_mut().expect("a spawner cart has a spawner");
        assert_eq!(
            (
                spawner.spawn_delay,
                spawner.max_spawn_delay,
                spawner.spawn_range
            ),
            (37, 120, 2)
        );
        // No raw tag to copy from: what comes out is the cart's own save.
        spawner.spawn_delay = 41;
        let out = cart.to_nbt([0; 4], None);
        assert_eq!(out.get("Delay"), Some(&Tag::Short(41)));
        assert_eq!(out.get("MaxSpawnDelay"), Some(&Tag::Short(120)));
        assert_eq!(
            out.get("SpawnData")
                .and_then(|d| d.get("entity"))
                .and_then(|e| e.get("id")),
            Some(&Tag::String("minecraft:zombie".into()))
        );
        assert_eq!(CartKind::Spawner.item(), "minecraft:minecart");
        assert_eq!(
            CartKind::from_id("minecraft:spawner_minecart"),
            Some(CartKind::Spawner)
        );
        // A summoned one has a spawner of its own.
        assert!(
            Minecart::new(2, CartKind::Spawner, Vec3::ZERO)
                .spawner
                .is_some()
        );
    }

    #[test]
    fn a_cow_boards_an_empty_cart_and_an_iron_golem_does_not() {
        assert!(pickup_instead_of_push(Species::Cow, false, false));
        assert!(can_ride_cart(Species::Cow, 0));
        assert!(!pickup_instead_of_push(Species::IronGolem, false, false));
        assert!(!pickup_instead_of_push(Species::Cow, false, true));
        assert!(!pickup_instead_of_push(Species::Cow, true, false));
        assert!(!can_ride_cart(Species::Cow, 60));
        assert!(!can_ride_cart(Species::Warden, 0));
    }

    #[test]
    fn a_cart_seat_matches_the_player_offset() {
        let at = Vec3::new(1.0, 10.0, 2.0);
        assert!((cart_seat(at, 0.0).y - 10.1875).abs() < 1e-12);
        assert!((cart_seat(at, 0.6).y - (10.0 + SEAT)).abs() < 1e-12);
        assert_eq!(cart_seat(at, 0.0).x, 1.0);
        assert_eq!(cart_seat(at, 0.0).z, 2.0);
    }

    #[test]
    fn a_cart_saves_as_never_ticked_until_it_ticks() {
        use horizon_world::nbt::Tag;
        let generated = Tag::compound([
            ("id", Tag::String("minecraft:chest_minecart".into())),
            (
                "Pos",
                Tag::List(
                    [0.5f64, 40.5, 0.5]
                        .map(|v| Tag::Double(v.to_bits()))
                        .to_vec(),
                ),
            ),
            ("HasTicked", Tag::Byte(1)),
            ("Fire", Tag::Short(0)),
        ]);
        let mut cart = Minecart::from_nbt(&generated, 1, CartKind::Chest).unwrap();
        let saved = cart.to_nbt([0; 4], Some(&generated));
        assert_eq!(saved.get("HasTicked"), Some(&Tag::Byte(1)));
        assert_eq!(saved.get("Fire"), Some(&Tag::Short(0)));
        cart.first_tick = false;
        let saved = cart.to_nbt([0; 4], Some(&generated));
        assert_eq!(saved.get("HasTicked"), Some(&Tag::Byte(0)));
        assert_eq!(saved.get("Fire"), Some(&Tag::Short(0)));
    }

    #[test]
    fn a_command_cart_saves_its_base_command_block() {
        use horizon_world::nbt::Tag;
        let saved = Tag::compound([
            (
                "Pos",
                Tag::List(
                    [0.5f64, 40.5, 0.5]
                        .map(|v| Tag::Double(v.to_bits()))
                        .to_vec(),
                ),
            ),
            ("Command", Tag::String("say hi".into())),
            ("SuccessCount", Tag::Int(3)),
            ("TrackOutput", Tag::Byte(1)),
            ("LastOutput", Tag::String("[12:00:00] hi".into())),
            ("LastExecution", Tag::Long(42)),
        ]);
        let cart = Minecart::from_nbt(&saved, 1, CartKind::CommandBlock).unwrap();
        assert_eq!(cart.command.command, "say hi");
        assert_eq!(cart.command.success_count, 3);
        // readAdditionalSaveData syncs both values.
        assert_eq!(cart.synced_command, "say hi");
        assert_eq!(cart.synced_output, "[12:00:00] hi");
        let out = cart.to_nbt([0; 4], Some(&saved));
        for key in [
            "Command",
            "SuccessCount",
            "TrackOutput",
            "LastOutput",
            "LastExecution",
        ] {
            assert_eq!(out.get(key), saved.get(key), "{key}");
        }
        assert_eq!(out.get("UpdateLastExecution"), Some(&Tag::Byte(1)));
        // A block-only flag never reaches the entity.
        assert!(out.get("powered").is_none() && out.get("auto").is_none());
        // Untracked output drops the saved last output.
        let mut quiet = cart.clone();
        quiet.command.track_output = false;
        let out = quiet.to_nbt([0; 4], Some(&saved));
        assert!(out.get("LastOutput").is_none());
        assert_eq!(out.get("TrackOutput"), Some(&Tag::Byte(0)));
    }

    #[test]
    fn furnace_push_keeps_its_length_under_gravity() {
        // Gravity gives the movement a vertical part. The push turns onto the
        // movement at the same 3D length every tick, as vanilla's does.
        let movement = Vec3::new(0.2, -0.04, 0.0);
        let mut push = Vec3::new(0.5, 0.0, 0.0);
        for _ in 0..1000 {
            push = push_along(push, movement);
        }
        assert!((push.length() - 0.5).abs() < 1e-12);
        assert!((push.x - 0.5 * 0.2 / movement.length()).abs() < 1e-12);
    }

    #[test]
    fn a_furnace_saves_its_push_without_y() {
        use horizon_world::nbt::Tag;
        let generated = Tag::compound([
            ("id", Tag::String("minecraft:furnace_minecart".into())),
            (
                "Pos",
                Tag::List(
                    [0.5f64, 40.5, 0.5]
                        .map(|v| Tag::Double(v.to_bits()))
                        .to_vec(),
                ),
            ),
        ]);
        let mut cart = Minecart::from_nbt(&generated, 1, CartKind::Furnace).unwrap();
        cart.push = Vec3::new(0.5, 0.7, 0.25);
        let saved = cart.to_nbt([0; 4], Some(&generated));
        assert_eq!(saved.get("PushX"), Some(&Tag::Double(0.5f64.to_bits())));
        assert_eq!(saved.get("PushZ"), Some(&Tag::Double(0.25f64.to_bits())));
        let loaded = Minecart::from_nbt(&saved, 1, CartKind::Furnace).unwrap();
        assert_eq!(loaded.push, Vec3::new(0.5, 0.0, 0.25));
    }
}
