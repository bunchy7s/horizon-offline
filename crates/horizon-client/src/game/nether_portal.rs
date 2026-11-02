//! Nether portals for the player and the other entities:
//! `Entity.setAsInsidePortal` / `handlePortal` with `PortalProcessor`,
//! `NetherPortalBlock.getPortalTransitionTime` and `getPortalDestination`
//! (`PortalForcer` through `horizon_world::level::portal_travel`), and
//! `LocalPlayer.handlePortalTransitionEffect`. Items and mobs also go through
//! End portals here (`EndPortalBlock.entityInside` / `getPortalDestination`);
//! the player's End portals are game/end.rs's.
//!
//! A server keeps one level per dimension. Entity portal processes and
//! cooldowns follow their level; shared `Arrival`s hold each traveller's
//! saved compound until its destination chunks are loaded.
use super::{Dim, Game, Screen};
use horizon_client::Result;
use horizon_client::fast_hash::FxHashMap;
use horizon_client::nether_portal::{ENTITY_DELAY, PortalState};
use horizon_core::{
    geometry::{Aabb, Axis, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::{
    level::portal_travel::{
        BorderBounds, FoundRectangle, closest_portal, collision_free_position, exit_position,
        largest_rectangle_around, relative_position,
    },
    nbt::Tag,
};
use std::collections::HashMap;

/// `Player.getDimensionChangingDelay`.
pub(super) const PLAYER_DELAY: i32 = 10;

/// Where in its portal a traveller stood, and where its exit is to be found.
#[derive(Clone, Copy, Debug)]
pub(super) struct Plan {
    pub(super) dim: Dim,
    /// The border-clamped, scaled position (`approximateExitPos`).
    pub(super) approximate: BlockPos,
    entry_x_axis: bool,
    offset: Vec3,
    width: f32,
    height: f32,
    /// Through an End portal: `approximate` is the End's spawn point, or the
    /// Overworld's shared spawn.
    pub(super) end: bool,
}

impl Plan {
    /// `EndPortalBlock.getPortalDestination` for an entity other than a
    /// player in `from`: the End's spawn point, or out of the End the
    /// Overworld's shared spawn `world_spawn`.
    fn end_portal(from: Dim, world_spawn: BlockPos, width: f32, height: f32) -> Self {
        let (dim, approximate) = if from == Dim::End {
            (Dim::Overworld, world_spawn)
        } else {
            let [x, y, z] = super::end::END_SPAWN;
            (Dim::End, BlockPos::new(x, y, z))
        };
        Self {
            dim,
            approximate,
            entry_x_axis: true,
            offset: Vec3::ZERO,
            width,
            height,
            end: true,
        }
    }
}

/// An entity on its way to a dimension that isn't loaded.
#[derive(Clone, Debug)]
struct Arrival {
    plan: Plan,
    tag: Tag,
}

/// The nether portal state of the world (`Game::portals`).
#[derive(Default)]
pub(super) struct Portals {
    /// The player's.
    pub(super) player: PortalState,
    /// The player's travel, while the destination loads.
    pub(super) travel: Option<Plan>,
    /// `LocalPlayer.portalProcess.isInsidePortalThisTick`.
    client_inside: bool,
    /// Items and mobs, by entity id.
    pub(super) entities: HashMap<u32, PortalState>,
    /// Positions before this tick's moves, by entity id.
    pub(super) before: FxHashMap<u32, Vec3>,
    arrivals: Vec<Arrival>,
    /// The vehicles server players ride that went through a portal this
    /// tick, in their level, with their entry: they carry their riders
    /// (`tick_server_travel`).
    carrying: Vec<(Dim, u32, BlockPos)>,
    loaded: bool,
    dirty: bool,
    /// Per-chunk portal palette masks, kept across ticks (portal_sections).
    pub(super) sections: horizon_client::portal_sections::PortalSections,
}

fn rotate_y(v: Vec3, degrees: f32) -> Vec3 {
    // Vec3.yRot((float)Math.toRadians(degrees))
    let angle = f64::from(degrees.to_radians());
    let (cos, sin) = (
        f64::from(horizon_core::geometry::trig::cos(angle)),
        f64::from(horizon_core::geometry::trig::sin(angle)),
    );
    Vec3::new(v.x * cos + v.z * sin, v.y, v.z * cos - v.x * sin)
}
/// Whether a projectile is an arrow, which goes through a portal like the
/// other entities.
fn is_arrow(p: &horizon_client::projectiles::Projectile) -> bool {
    matches!(
        p.kind,
        horizon_client::projectiles::ProjectileKind::Arrow { .. }
    )
}
/// The projectiles besides arrows that a portal sends on: the thrown items
/// and fireballs a chunk saves. A pearl has its own rules (it keeps its owner's
/// dimension) and a wind charge is not saved, so both stay.
fn is_portal_projectile(p: &horizon_client::projectiles::Projectile) -> bool {
    use horizon_client::projectiles::ProjectileKind;
    !is_arrow(p)
        && !matches!(
            p.kind,
            ProjectileKind::EnderPearl
                | ProjectileKind::WindCharge
                | ProjectileKind::BreezeWindCharge
        )
        && horizon_client::entity_nbt::saved_with_chunk(p)
}
/// Width and height of a projectile that goes through a portal.
fn projectile_box(p: &horizon_client::projectiles::Projectile) -> (f32, f32) {
    if is_arrow(p) {
        (
            horizon_client::arrows::WIDTH,
            horizon_client::arrows::HEIGHT,
        )
    } else {
        let size = p.kind.size() as f32;
        (size, size)
    }
}
fn doubles(values: [f64; 3]) -> Tag {
    Tag::List(values.iter().map(|v| Tag::Double(v.to_bits())).collect())
}
/// Whether a `width` by `height` box moving from `from` to `to` meets the End
/// portal at `pos`: `Entity.collidedWithShapeMovingFrom` against its
/// entity-inside shape, y 6/16..12/16 of the block.
fn touches_end_portal(from: Vec3, to: Vec3, width: f32, height: f32, pos: BlockPos) -> bool {
    let half = f64::from(width) / 2.0;
    let start = Aabb::new(
        Vec3::new(from.x - half, from.y, from.z - half),
        Vec3::new(from.x + half, from.y + f64::from(height), from.z + half),
    );
    let (x, y, z) = (f64::from(pos.x), f64::from(pos.y), f64::from(pos.z));
    let shape = Aabb::new(
        Vec3::new(x, y + 0.375, z),
        Vec3::new(x + 1.0, y + 0.75, z + 1.0),
    );
    start.collided_along_vector(to.subtract(from), &[shape])
}
/// `PositionMoveRotation.calculateAbsolute` for an End portal's transition
/// of an entity facing `[yaw, pitch]`: into the End it faces west and keeps
/// its pitch (`Relative.X_ROT`); out of it both turn by the world spawn's
/// angles (`Relative.ROTATION`). The pitch stays within ±90.
fn end_portal_rotation(into_end: bool, [yaw, pitch]: [f32; 2], spawn: [f32; 2]) -> [f32; 2] {
    let (yaw, pitch) = if into_end {
        // Direction.WEST.toYRot().
        (90.0, pitch)
    } else {
        (yaw + spawn[0], pitch + spawn[1])
    };
    [yaw, pitch.clamp(-90.0, 90.0)]
}
/// A waiting entity as `horizon_portal_arrivals.dat` keeps it.
fn arrival_tag(a: &Arrival) -> Tag {
    let p = &a.plan;
    Tag::compound([
        ("dimension", Tag::String(p.dim.id().into())),
        ("x", Tag::Int(p.approximate.x)),
        ("y", Tag::Int(p.approximate.y)),
        ("z", Tag::Int(p.approximate.z)),
        ("x_axis", Tag::Int(i32::from(p.entry_x_axis))),
        ("right", Tag::Double(p.offset.x.to_bits())),
        ("up", Tag::Double(p.offset.y.to_bits())),
        ("forward", Tag::Double(p.offset.z.to_bits())),
        ("width", Tag::Float(p.width.to_bits())),
        ("height", Tag::Float(p.height.to_bits())),
        ("end", Tag::Int(i32::from(p.end))),
        ("entity", a.tag.clone()),
    ])
}
/// The waiting entity `arrival_tag` wrote.
fn arrival_from_tag(a: &Tag) -> Option<Arrival> {
    let int = |k: &str| match a.get(k) {
        Some(Tag::Int(v)) => Some(*v),
        _ => None,
    };
    let float = |k: &str| match a.get(k) {
        Some(Tag::Float(v)) => Some(f32::from_bits(*v)),
        _ => None,
    };
    let double = |k: &str| match a.get(k) {
        Some(Tag::Double(v)) => Some(f64::from_bits(*v)),
        _ => None,
    };
    let dim = Dim::from_id(&horizon_client::save::text(a.get("dimension"))?);
    let (Some(x), Some(y), Some(z), Some(tag)) = (int("x"), int("y"), int("z"), a.get("entity"))
    else {
        return None;
    };
    Some(Arrival {
        plan: Plan {
            dim,
            approximate: BlockPos::new(x, y, z),
            entry_x_axis: int("x_axis").unwrap_or(1) != 0,
            offset: Vec3::new(
                double("right").unwrap_or(0.5),
                double("up").unwrap_or(0.0),
                double("forward").unwrap_or(0.0),
            ),
            width: float("width").unwrap_or(0.25),
            height: float("height").unwrap_or(0.25),
            end: int("end").unwrap_or(0) != 0,
        },
        tag: tag.clone(),
    })
}

impl Game<'_> {
    /// The world border's extent.
    pub(super) fn border_bounds(&self) -> BorderBounds {
        let [min_x, min_z, max_x, max_z] = self.border.bounds();
        BorderBounds {
            min_x,
            min_z,
            max_x,
            max_z,
        }
    }
    /// `ServerLevel.isAllowedToEnterPortal`.
    pub(super) fn may_enter(&self, target: Dim) -> bool {
        target != Dim::Nether || self.ui.rules.bool("allow_entering_nether_using_portals")
    }
    /// The plan of an entity of `width` by `height` at `at` going through the
    /// portal at `entry` (`getDimensionTransitionFromExit` and the scaled,
    /// clamped exit of `getPortalDestination`).
    pub(super) fn portal_plan(
        &self,
        entry: BlockPos,
        at: Vec3,
        width: f32,
        height: f32,
        reset_forward: bool,
    ) -> Plan {
        let target = if self.dim == Dim::Nether {
            Dim::Overworld
        } else {
            Dim::Nether
        };
        let state = self.world.block(entry.x, entry.y, entry.z);
        let b = self.res.behaviours();
        let (entry_x_axis, offset) = match b.portal_axis(state) {
            Some(x_axis) => {
                let area = largest_rectangle_around(
                    entry,
                    if x_axis { Axis::X } else { Axis::Z },
                    21,
                    Axis::Y,
                    21,
                    |p| self.world.block(p.x, p.y, p.z) == state,
                );
                let mut offset = relative_position(area, x_axis, at, width, height);
                if reset_forward {
                    offset.z = 0.0;
                }
                (x_axis, offset)
            }
            None => (true, Vec3::new(0.5, 0.0, 0.0)),
        };
        // DimensionType.getTeleportationScale: 8 out of the Nether, 1/8 into it.
        let scale = if self.dim == Dim::Nether {
            8.0
        } else {
            1.0 / 8.0
        };
        Plan {
            dim: target,
            approximate: self.border_bounds().clamp(at.x * scale, at.y, at.z * scale),
            entry_x_axis,
            offset,
            width,
            height,
            end: false,
        }
    }
    /// The plan of an item or mob going through the portal block at `entry`:
    /// an End portal's fixed destination, else the Nether portal's.
    fn entity_plan(
        &self,
        entry: BlockPos,
        at: Vec3,
        width: f32,
        height: f32,
        reset_forward: bool,
    ) -> Plan {
        if !self.is_end_portal(self.world.block(entry.x, entry.y, entry.z)) {
            return self.portal_plan(entry, at, width, height, reset_forward);
        }
        let spawn = self.level.world_spawn.unwrap_or_else(|| {
            self.spawn.map_or([0, 64, 0], |s| {
                [s.x.floor() as i32, s.y.floor() as i32, s.z.floor() as i32]
            })
        });
        Plan::end_portal(
            self.dim,
            BlockPos::new(spawn[0], spawn[1], spawn[2]),
            width,
            height,
        )
    }
    fn is_end_portal(&self, state: horizon_world::registry::BlockStateId) -> bool {
        &*self.res.facts().get(state).name == "minecraft:end_portal"
    }
    /// Where an item or mob through an End portal lands once `plan`'s level
    /// has loaded around it: the bottom centre of the End's spawn point,
    /// above the platform built below it (`EndPlatformFeature.createEndPlatform`);
    /// out of the End, the top of the shared spawn's column
    /// (`Entity.adjustSpawnLocation`).
    fn end_portal_exit(&mut self, plan: &Plan) -> Vec3 {
        let p = plan.approximate;
        let centre = Vec3::new(f64::from(p.x) + 0.5, f64::from(p.y), f64::from(p.z) + 0.5);
        if plan.dim == Dim::End {
            // In the End the arrival only builds the platform: no spawn search.
            if let Err(e) = self.end_arrival(centre, &mut None) {
                log::error!("End platform: {e}");
            }
            return centre;
        }
        let y = self.motion_top(p.x, p.z);
        Vec3::new(centre.x, f64::from(y), centre.z)
    }
    /// The nether portal blocks of the chunks within `radius` of `center`
    /// (the points of interest `PortalForcer` searches): loaded chunks as
    /// they are, the others as saved; chunks never generated have none.
    fn portal_blocks(&self, center: BlockPos, radius: i32) -> Vec<BlockPos> {
        let b = self.res.behaviours();
        let chunk_radius = radius.div_euclid(16) + 1;
        let (cx, cz) = (center.x >> 4, center.z >> 4);
        let mut out = Vec::new();
        for z in cz - chunk_radius..=cz + chunk_radius {
            for x in cx - chunk_radius..=cx + chunk_radius {
                let saved;
                let chunk = match self.world.chunks.get(&(x, z)) {
                    Some(c) => &**c,
                    None => {
                        match self
                            .save
                            .load(x, z, self.pack.assets.registry, world_biomes!(self))
                        {
                            Some(c) => {
                                saved = c;
                                &saved
                            }
                            None => continue,
                        }
                    }
                };
                for (s, section) in chunk.sections.iter().enumerate() {
                    if !section.blocks.maybe_has(|v| {
                        b.portal_axis(horizon_world::registry::BlockStateId(v))
                            .is_some()
                    }) {
                        continue;
                    }
                    let base = self.world.bottom_y() + 16 * s as i32;
                    for y in base..base + 16 {
                        for lz in 0..16 {
                            for lx in 0..16 {
                                let (wx, wz) = (x * 16 + lx, z * 16 + lz);
                                if b.portal_axis(chunk.block(wx, y, wz)).is_some() {
                                    out.push(BlockPos::new(wx, y, wz));
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }
    /// The exit portal of `plan` in the loaded dimension: the closest one
    /// within reach, else one built by `PortalForcer.createPortal`.
    pub(super) fn nether_exit_portal(&mut self, plan: &Plan) -> Option<FoundRectangle> {
        let to_nether = plan.dim == Dim::Nether;
        let radius = if to_nether { 16 } else { 128 };
        let border = self.border_bounds();
        let portals = self.portal_blocks(plan.approximate, radius);
        let (min_section, max_section) = if to_nether { (0, 15) } else { (-4, 19) };
        let found = closest_portal(
            &portals,
            plan.approximate,
            to_nether,
            &border,
            min_section,
            max_section,
        );
        if let Some(found) = found {
            // NetherPortalBlock.getExitPortal: the ticket goes on the chunk
            // of the portal found.
            self.place_portal_ticket((found.x >> 4, found.z >> 4));
            let state = self.world.block(found.x, found.y, found.z);
            let x_axis = self.res.behaviours().portal_axis(state).unwrap_or(true);
            return Some(largest_rectangle_around(
                found,
                if x_axis { Axis::X } else { Axis::Z },
                21,
                Axis::Y,
                21,
                |p| self.world.block(p.x, p.y, p.z) == state,
            ));
        }
        let logical = if to_nether { 128 } else { 384 };
        let (approximate, x_axis) = (plan.approximate, plan.entry_x_axis);
        let created = self.with_level(|b, level, ticks| {
            b.create_portal(level, ticks, approximate, x_axis, &border, logical)
        });
        let _ = self.process_level_events();
        if let Some(exit) = created {
            // A portal built for the entity: the ticket goes on the chunk it
            // arrives in (`TeleportTransition.PLACE_PORTAL_TICKET`).
            let (at, _) = self.arrival(plan, exit);
            self.place_portal_ticket(Self::chunk_of(at));
        }
        created
    }
    /// `ChunkPos.containing`: the chunk a position is in.
    pub(super) fn chunk_of(at: Vec3) -> (i32, i32) {
        ((at.x.floor() as i32) >> 4, (at.z.floor() as i32) >> 4)
    }
    /// `Entity.placePortalTicket`: a `TicketType.PORTAL` ticket of radius 3
    /// on `chunk` in the level in place, which keeps the area running for 300
    /// ticks with no player in it (the entity that just came through ticks
    /// there, and goes back through the portal when its cooldown ends).
    /// Only a server holds levels with tickets of their own.
    pub(super) fn place_portal_ticket(&mut self, chunk: (i32, i32)) {
        if !self.options.execution_role.is_client() {
            self.tickets.add_portal(chunk);
        }
    }
    /// `NetherPortalBlock.createDimensionTransition`: the arrival position
    /// and the yaw turned by.
    pub(super) fn arrival(&self, plan: &Plan, exit: FoundRectangle) -> (Vec3, f32) {
        let b = self.res.behaviours();
        let corner = exit.min_corner;
        let exit_x_axis = b
            .portal_axis(self.world.block(corner.x, corner.y, corner.z))
            .unwrap_or(true);
        let (bottom, rotation) = exit_position(
            exit,
            exit_x_axis,
            plan.entry_x_axis,
            plan.offset,
            plan.width,
            plan.height,
        );
        let border = self.border_bounds();
        let at = collision_free_position(bottom, plan.width, plan.height, &border, |p| {
            b.collision_shape(self.world.block(p.x, p.y, p.z))
                .map(|s| s.moved(Vec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z))))
        });
        (at, rotation)
    }

    // ---- the player ----

    /// `NetherPortalBlock.entityInside` for the player: `canUsePortal`, then
    /// `setAsInsidePortal` (and the client's own, which drives the effect).
    pub(super) fn player_inside_nether_portal(&mut self, pos: BlockPos) {
        let alive = self
            .player
            .as_ref()
            .is_some_and(|p| !p.health.dead_or_dying());
        if !alive || self.riding.is_some() || self.sleeping.is_some() {
            return;
        }
        // The client's LocalPlayer has the same cooldown (the Respawn
        // packet's spawn info): standing in the arrival portal, its overlay
        // fades instead of building up again.
        let cooling = self.portals.player.cooldown > 0;
        self.portals.player.set_inside(pos, PLAYER_DELAY);
        if !cooling {
            self.portals.client_inside = true;
        }
    }
    /// The same on a server's client: `Entity.canUsePortal` (alive, not a
    /// passenger), then only the client's `setAsInsidePortal`, which drives
    /// the confusion effect (`client_portal_effect`). On a server's client
    /// `portals.player` holds only the LocalPlayer's `portalCooldown`.
    pub(super) fn client_inside_nether_portal(&mut self) {
        let alive = self
            .player
            .as_ref()
            .is_some_and(|p| !p.health.dead_or_dying());
        if !alive || self.riding.is_some() {
            return;
        }
        if self.portals.player.cooldown > 0 {
            self.portals.player.cooldown = PLAYER_DELAY;
        } else {
            self.portals.client_inside = true;
        }
    }
    /// `ServerPlayer.handlePortal` (not while changing dimension: there is no
    /// player then). Players wait `players_nether_portal_default_delay`
    /// ticks, or the creative delay with invulnerable abilities.
    pub(super) fn tick_nether_portal(&mut self) -> Result<()> {
        // A server's portals are its own.
        if self.remote.is_some() {
            return Ok(());
        }
        let Some(p) = self.player.as_ref() else {
            return Ok(());
        };
        if self.travelling.is_some() {
            return Ok(());
        }
        let allowed = !p.health.dead_or_dying() && self.riding.is_none() && self.sleeping.is_none();
        let rule = if p.invulnerable {
            "players_nether_portal_creative_delay"
        } else {
            "players_nether_portal_default_delay"
        };
        let transition = self.ui.rules.int(rule).max(0);
        let Some(entry) = self
            .portals
            .player
            .handle(allowed, transition, PLAYER_DELAY)
        else {
            return Ok(());
        };
        self.player_through_nether_portal(entry)
    }
    /// `Entity.handlePortal` once the player's portal process at `entry`
    /// finished: `NetherPortalBlock.getPortalDestination`, then
    /// `teleportToPortalDestination` (`isAllowedToEnterPortal`).
    pub(super) fn player_through_nether_portal(&mut self, entry: BlockPos) -> Result<()> {
        let Some(p) = self.player.as_ref() else {
            return Ok(());
        };
        let plan = self.portal_plan(
            entry,
            p.position,
            p.dimensions.width,
            p.dimensions.height,
            true,
        );
        if !self.may_enter(plan.dim) {
            return Ok(());
        }
        self.portals.travel = Some(plan);
        self.end_portal_travel = false;
        self.enter_dimension(plan.dim)?;
        let centre = self.arrival_centre(&plan);
        if let Some((_, target)) = self.travelling.as_mut() {
            *target = centre;
        }
        Ok(())
    }
    /// Where to wait for the destination of `plan` to load, once its level
    /// is open: the portal its points of interest lead to, or the
    /// approximate exit.
    pub(super) fn arrival_centre(&self, plan: &Plan) -> Vec3 {
        let to_nether = plan.dim == Dim::Nether;
        let portals = self.portal_blocks(plan.approximate, if to_nether { 16 } else { 128 });
        let (min_section, max_section) = if to_nether { (0, 15) } else { (-4, 19) };
        let border = self.border_bounds();
        let found = closest_portal(
            &portals,
            plan.approximate,
            to_nether,
            &border,
            min_section,
            max_section,
        );
        let centre = found.unwrap_or(plan.approximate);
        Vec3::new(
            f64::from(centre.x) + 0.5,
            f64::from(centre.y),
            f64::from(centre.z) + 0.5,
        )
    }
    /// The player's arrival once the destination has loaded around the
    /// approximate exit: the waiting entities first, then the exit portal
    /// (found or built), the position in it, the turned yaw and motion, and
    /// `TeleportTransition.PLAY_PORTAL_SOUND` (level event 1032).
    pub(super) fn arrive_through_nether_portal(
        &mut self,
        mut player: horizon_client::motion::PlayerMotion,
        target: Vec3,
    ) -> Result<()> {
        self.resolve_arrivals();
        let plan = self.portals.travel.take();
        let Some(plan) = plan else {
            // A command teleport can change dimensions without going through
            // a portal. It has no portal placement, rotation, or sound effect.
            player.position = target;
            player.velocity = Vec3::ZERO;
            player.fall_distance = 0.0;
            self.previous_position = target;
            self.player = Some(player);
            self.pad.player_replaced();
            self.sway = Default::default();
            self.apply_equipment();
            return Ok(());
        };
        let exit = self.nether_exit_portal(&plan);
        let (position, rotation) = match exit {
            Some(exit) => self.arrival(&plan, exit),
            None => (target, 0.0),
        };
        player.position = position;
        // Relative.ROTATION and Relative.DELTA: the yaw turns, the motion
        // turns with it.
        player.yaw += rotation;
        player.head_yaw += rotation;
        self.yaw += rotation;
        player.velocity = rotate_y(player.velocity, -rotation);
        self.previous_position = player.position;
        self.player = Some(player);
        self.pad.player_replaced();
        self.sway = Default::default();
        self.apply_equipment();
        let pitch = self.world.random.next_float() * 0.4 + 0.8;
        self.audio
            .play("minecraft:block.portal.travel", None, 0.25, pitch);
        Ok(())
    }
    /// `LocalPlayer.handlePortalTransitionEffect`: whether the confusion
    /// effect grows this tick. Entering a portal closes screens other than
    /// the chat and death screens and plays `block.portal.trigger` when the
    /// effect starts from nothing.
    pub(super) fn client_portal_effect(&mut self) -> bool {
        if !std::mem::take(&mut self.portals.client_inside) {
            return false;
        }
        match self.screen {
            Screen::Playing | Screen::Chat | Screen::Dead => {}
            Screen::Inventory | Screen::Creative => {
                self.close_container();
                self.carried = None;
                self.screen = Screen::Playing;
                self.capture(true);
            }
            _ => {
                self.screen = Screen::Playing;
                self.capture(true);
            }
        }
        if self.screen_fx.portal == 0.0 {
            let pitch = self.world.random.next_float() * 0.4 + 0.8;
            self.audio
                .play("minecraft:block.portal.trigger", None, 0.25, pitch);
        }
        true
    }

    // ---- other entities ----

    /// Positions before the entities move this tick, and `Entity.handlePortal`
    /// at the start of their tick: an item or mob that stayed in a portal
    /// goes through (transition time 0) and waits for its dimension.
    pub(super) fn handle_entity_portals(&mut self) {
        if self.remote.is_some() {
            return;
        }
        self.load_portal_arrivals();
        let mut leaving: Vec<(u32, BlockPos)> = Vec::new();
        self.portals.before.clear();
        for e in &self.dropped {
            self.portals.before.insert(e.id, e.position);
        }
        for m in &self.mobs {
            self.portals.before.insert(m.id, m.position);
        }
        for b in &self.boats {
            self.portals.before.insert(b.id, b.position);
        }
        for c in &self.carts {
            self.portals.before.insert(c.id, c.position);
        }
        for t in &self.tnts {
            self.portals.before.insert(t.id, t.position);
        }
        for o in &self.orbs {
            self.portals.before.insert(o.id, o.position);
        }
        // Only an entity in a portal, or cooling down from one, has a state
        // here: whether the others could use a portal is not worth asking
        // for each of them on every tick.
        let mut ids: Vec<(u32, bool)> = Vec::new();
        if !self.portals.entities.is_empty() {
            let tracked = &self.portals.entities;
            for e in &self.dropped {
                if tracked.contains_key(&e.id) {
                    ids.push((e.id, true));
                }
            }
            for m in &self.mobs {
                if tracked.contains_key(&m.id) {
                    ids.push((m.id, self.mob_can_use_portal(m)));
                }
            }
            for t in &self.tnts {
                if tracked.contains_key(&t.id) {
                    ids.push((t.id, self.tnt_can_use_portal(t.id)));
                }
            }
            let vehicles = self.boats.iter().map(|b| b.id);
            for id in vehicles.chain(self.carts.iter().map(|c| c.id)) {
                if tracked.contains_key(&id) {
                    ids.push((id, self.vehicle_can_use_portal(id)));
                }
            }
            for p in &self.creatures.projectiles {
                if tracked.contains_key(&p.id) && (is_arrow(p) || is_portal_projectile(p)) {
                    ids.push((p.id, true));
                }
            }
            for o in &self.orbs {
                if tracked.contains_key(&o.id) {
                    ids.push((o.id, true));
                }
            }
        }
        let present: std::collections::HashSet<u32> = ids.iter().map(|(id, _)| *id).collect();
        self.portals.entities.retain(|id, _| present.contains(id));
        for (id, allowed) in ids {
            let Some(state) = self.portals.entities.get_mut(&id) else {
                continue;
            };
            if let Some(entry) = state.handle(allowed, 0, ENTITY_DELAY) {
                leaving.push((id, entry));
            }
            if state.cooldown == 0 && state.process.is_none() {
                self.portals.entities.remove(&id);
            }
        }
        for (id, entry) in leaving {
            // `Entity.teleportCrossDimension` carries the passengers: a
            // vehicle server players ride takes them along.
            if self.guest_rides(id) {
                self.portals.carrying.push((self.dim, id, entry));
            } else {
                self.send_through_portal(id, entry);
            }
        }
    }
    /// `Entity.load`: the saved `PortalCooldown` (an entity that just came
    /// through stands in its exit portal for 300 ticks without going back).
    pub(super) fn restore_portal_cooldown(&mut self, id: u32, tag: &Tag) {
        if let Some(Tag::Int(cooldown)) = tag.get("PortalCooldown")
            && *cooldown > 0
        {
            self.portals.entities.insert(
                id,
                PortalState {
                    cooldown: *cooldown,
                    process: None,
                },
            );
        }
    }
    /// `Entity.setAsInsidePortal` on an entity that is still on its portal
    /// cooldown, whatever the portal: the cooldown restarts and it stays.
    pub(super) fn restart_portal_cooldown(&mut self, id: u32) -> bool {
        match self.portals.entities.get_mut(&id) {
            Some(state) if state.cooldown > 0 => {
                state.cooldown = ENTITY_DELAY;
                true
            }
            _ => false,
        }
    }
    /// `Entity.setPortalCooldown` on an entity that has just spawned in a
    /// portal: it stands in the portal for 300 ticks without going through.
    pub(super) fn set_portal_cooldown(&mut self, id: u32) {
        self.portals.entities.insert(
            id,
            PortalState {
                cooldown: ENTITY_DELAY,
                process: None,
            },
        );
    }
    /// `canUsePortal(false)` of a mob: alive, riding nothing, not asleep,
    /// not carrying the player, and not a boss that never travels.
    pub(super) fn mob_can_use_portal(&self, m: &horizon_client::mobs::Mob) -> bool {
        use horizon_client::mobs::Species;
        m.alive()
            && m.extra.mount.is_none()
            && self.riding != Some(m.id)
            && (!self.guest_rides(m.id) || self.carries_riders())
            && m.species() != Species::Wither
    }
    /// `canUsePortal(false)` of a boat or minecart: empty, or ridden by
    /// server players only, whom it carries along.
    /// `Entity.canUsePortal(false)` for a primed TNT: it carries no mob.
    fn tnt_can_use_portal(&self, id: u32) -> bool {
        // Id 0: not tracked yet (the in-frame path), nothing to key its portal state by.
        id != 0 && !self.mobs.iter().any(|m| m.extra.mount == Some(id))
    }
    fn vehicle_can_use_portal(&self, id: u32) -> bool {
        if !self.vehicle_occupied(id) {
            return true;
        }
        self.carries_riders()
            && self.riding != Some(id)
            && !self.mobs.iter().any(|m| m.extra.mount == Some(id))
    }
    /// Whether a vehicle can take the server players riding it through a
    /// portal: only a server moves players between its levels. Elsewhere a
    /// ridden vehicle stays, rather than leave its rider behind.
    fn carries_riders(&self) -> bool {
        self.options.dedicated.is_some()
    }
    /// The vehicles that went through a portal in `dim` with server players
    /// on them.
    pub(super) fn take_carrying(&mut self, dim: Dim) -> Vec<(u32, BlockPos)> {
        let (here, other) = std::mem::take(&mut self.portals.carrying)
            .into_iter()
            .partition(|(d, ..)| *d == dim);
        self.portals.carrying = other;
        here.into_iter().map(|(_, id, entry)| (id, entry)).collect()
    }
    /// The plan of vehicle `id` through the portal at `entry`: from its own
    /// position and size (`getPortalDestination(level, vehicle)`).
    pub(super) fn vehicle_plan(&self, id: u32, entry: BlockPos) -> Option<Plan> {
        if let Some(m) = self.mobs.iter().find(|m| m.id == id) {
            return Some(self.entity_plan(entry, m.position, m.width(), m.height(), true));
        }
        if let Some(b) = self.boats.iter().find(|b| b.id == id) {
            let (width, height) = (super::boats::WIDTH as f32, super::boats::HEIGHT as f32);
            return Some(self.entity_plan(entry, b.position, width, height, false));
        }
        let c = self.carts.iter().find(|c| c.id == id)?;
        let (width, height) = (
            super::minecarts::WIDTH as f32,
            super::minecarts::HEIGHT as f32,
        );
        Some(self.entity_plan(entry, c.position, width, height, false))
    }
    /// Whether the player, a server player or a mob rides vehicle `id`.
    fn vehicle_occupied(&self, id: u32) -> bool {
        self.riding == Some(id)
            || self.ridden_by_guest(id)
            || self.mobs.iter().any(|m| m.extra.mount == Some(id))
    }
    /// `NetherPortalBlock.entityInside` and `EndPortalBlock.entityInside`
    /// for the items, mobs, empty boats and minecarts, and arrows that moved
    /// this tick: the portal blocks their box swept through (an End portal's
    /// transition time is 0 too).
    pub(super) fn find_entity_portals(&mut self) {
        if self.remote.is_some() {
            return;
        }
        let mut inside: Vec<(u32, BlockPos)> = Vec::new();
        let items: Vec<(u32, Vec3, f32, f32)> = self
            .dropped
            .iter()
            .map(|e| (e.id, e.position, 0.25, 0.25))
            .chain(self.orbs.iter().map(|o| (o.id, o.position, 0.5, 0.5)))
            .collect();
        // PrimedTnt is 0.98 across; one that carries a mob stays (the rider would be left behind).
        let tnts: Vec<(u32, Vec3, f32, f32)> = self
            .tnts
            .iter()
            .filter(|t| self.tnt_can_use_portal(t.id))
            .map(|t| (t.id, t.position, 0.98, 0.98))
            .collect();
        let mobs: Vec<(u32, Vec3, f32, f32)> = self
            .mobs
            .iter()
            .filter(|m| self.mob_can_use_portal(m))
            .map(|m| (m.id, m.position, m.width(), m.height()))
            .collect();
        let vehicles: Vec<(u32, Vec3, f32, f32)> = self
            .boats
            .iter()
            .map(|b| (b.id, b.position, super::boats::WIDTH, super::boats::HEIGHT))
            .chain(self.carts.iter().map(|c| {
                (
                    c.id,
                    c.position,
                    super::minecarts::WIDTH,
                    super::minecarts::HEIGHT,
                )
            }))
            .filter(|(id, ..)| self.vehicle_can_use_portal(*id))
            .map(|(id, at, w, h)| (id, at, w as f32, h as f32))
            .collect();
        let mut portal_sections = std::mem::take(&mut self.portals.sections);
        for (id, to, width, height) in items.into_iter().chain(tnts).chain(mobs).chain(vehicles) {
            let from = self.portals.before.get(&id).copied().unwrap_or(to);
            if let Some(pos) = self.swept_portal(from, to, width, height, &mut portal_sections) {
                inside.push((id, pos));
            }
        }
        // The arrow's `previous` is where it stood at the start of its tick,
        // which is also where an arrow shot this tick began. A stuck arrow
        // that did not move this tick enters no block.
        for p in self.creatures.projectiles.iter().filter(|p| {
            (is_arrow(p) && (!p.stuck || p.previous != p.position)) || is_portal_projectile(p)
        }) {
            let (width, height) = projectile_box(p);
            if let Some(pos) =
                self.swept_portal(p.previous, p.position, width, height, &mut portal_sections)
            {
                inside.push((p.id, pos));
            }
        }
        self.portals.sections = portal_sections;
        for (id, pos) in inside {
            self.portals
                .entities
                .entry(id)
                .or_default()
                .set_inside(pos, ENTITY_DELAY);
        }
    }
    /// The first nether or End portal block an entity box swept through
    /// (`Entity.checkInsideBlocks`, whose first portal sets the entry).
    fn swept_portal(
        &self,
        from: Vec3,
        to: Vec3,
        width: f32,
        height: f32,
        portal_sections: &mut horizon_client::portal_sections::PortalSections,
    ) -> Option<BlockPos> {
        let half = f64::from(width) / 2.0;
        let bounds = horizon_core::geometry::Aabb::new(
            Vec3::new(to.x - half, to.y, to.z - half),
            Vec3::new(to.x + half, to.y + f64::from(height), to.z + half),
        )
        .deflate(Vec3::new(
            f64::from(1.0e-5_f32),
            f64::from(1.0e-5_f32),
            f64::from(1.0e-5_f32),
        ));
        // Every block the sweep visits lies in the box covering both ends.
        let reach = bounds
            .union_bounds(bounds.move_by(from.subtract(to)))
            .inflate(Vec3::new(1.0, 1.0, 1.0));
        if !self.may_hold_portal(reach, portal_sections) {
            return None;
        }
        let mut found = None;
        let _ = horizon_core::geometry::sweep_intersected_blocks(
            from,
            to,
            bounds,
            1_000_000,
            |p, _| {
                let state = self.world.block(p.x, p.y, p.z);
                if self.res.behaviours().portal_axis(state).is_some()
                    || (self.is_end_portal(state) && touches_end_portal(from, to, width, height, p))
                {
                    found = Some(p);
                    return false;
                }
                true
            },
        );
        found
    }
    /// Whether a section `reach` touches has a nether or End portal state in
    /// its palette: without one, no block there is a portal. Palette answers are
    /// kept per chunk across ticks (`portal_sections`); a box over many
    /// sections is not checked.
    fn may_hold_portal(
        &self,
        reach: horizon_core::geometry::Aabb,
        sections: &mut horizon_client::portal_sections::PortalSections,
    ) -> bool {
        use horizon_client::section_mask::SectionMask;
        let (min, max) = (reach.min_position(), reach.max_position());
        if !min.is_finite() || !max.is_finite() {
            return true;
        }
        let (min, max) = (BlockPos::containing(min), BlockPos::containing(max));
        let top = self.world.layout().sections() as i32 - 1;
        let (x0, x1) = (min.x >> 4, max.x >> 4);
        let (z0, z1) = (min.z >> 4, max.z >> 4);
        let (y0, y1) = (
            ((min.y - self.world.bottom_y()) >> 4).clamp(0, top),
            ((max.y - self.world.bottom_y()) >> 4).clamp(0, top),
        );
        let count = (i64::from(x1) - i64::from(x0) + 1)
            * (i64::from(y1) - i64::from(y0) + 1)
            * (i64::from(z1) - i64::from(z0) + 1);
        if count > 64 {
            return true;
        }
        let y_mask = SectionMask::below(y1 as usize + 1).from(y0 as usize);
        let behaviours = self.res.behaviours();
        let chunks = &self.world.chunks;
        (x0..=x1).any(|cx| {
            (z0..=z1).any(|cz| {
                chunks.get(&(cx, cz)).is_some_and(|chunk| {
                    let portals = sections.get((cx, cz), chunk, |v| {
                        let state = horizon_world::registry::BlockStateId(v);
                        behaviours.portal_axis(state).is_some() || self.is_end_portal(state)
                    });
                    !(portals & y_mask).is_empty()
                })
            })
        })
    }
    /// `Entity.teleportCrossDimension` away from the loaded dimension: the
    /// entity is saved and removed, and waits for its dimension to load.
    fn send_through_portal(&mut self, id: u32, entry: BlockPos) {
        if let Some(i) = self.dropped.iter().position(|e| e.id == id) {
            let e = &self.dropped[i];
            let plan = self.entity_plan(entry, e.position, 0.25, 0.25, false);
            if !self.may_enter(plan.dim) {
                return;
            }
            let uuid = self.uuid_for(id);
            let tag = horizon_client::entity_nbt::item_entity_to_nbt(
                &self.dropped[i],
                uuid,
                self.persist_raw(id),
            );
            self.remove_dropped(i);
            self.detach_entity(id);
            self.portals.arrivals.push(Arrival { plan, tag });
            self.portals.dirty = true;
        } else if let Some(i) = self.tnts.iter().position(|t| t.id == id) {
            let t = &self.tnts[i];
            let plan = self.entity_plan(entry, t.position, 0.98, 0.98, false);
            if !self.may_enter(plan.dim) {
                return;
            }
            let uuid = self.uuid_for(id);
            let t = &self.tnts[i];
            let tag = horizon_client::entity_nbt::tnt_to_nbt(
                t.position,
                t.velocity,
                t.on_ground,
                t.fuse,
                uuid,
                self.persist_raw(id),
            );
            self.tnts.remove(i);
            self.forget_entity(id);
            self.portals.arrivals.push(Arrival { plan, tag });
            self.portals.dirty = true;
        } else if let Some(i) = self.orbs.iter().position(|o| o.id == id) {
            let plan = self.entity_plan(entry, self.orbs[i].position, 0.5, 0.5, false);
            if !self.may_enter(plan.dim) {
                return;
            }
            let uuid = self.uuid_for(id);
            let tag = horizon_client::entity_nbt::orb_to_nbt(&self.orbs[i], uuid);
            self.orbs.remove(i);
            self.forget_entity(id);
            self.portals.arrivals.push(Arrival { plan, tag });
            self.portals.dirty = true;
        } else if let Some(i) = self.mobs.iter().position(|m| m.id == id) {
            let m = &self.mobs[i];
            let plan = self.entity_plan(entry, m.position, m.width(), m.height(), true);
            if !self.may_enter(plan.dim) {
                return;
            }
            let mut tag = self.mob_tree_nbt(i);
            // Entity.removeAfterChangingDimensions drops the leash.
            if let Tag::Compound(c) = &mut tag {
                c.remove(&"leash".into());
            }
            let mut gone = vec![id];
            let mut k = 0;
            while k < gone.len() {
                let v = gone[k];
                gone.extend(
                    self.mobs
                        .iter()
                        .filter(|m| m.extra.mount == Some(v))
                        .map(|m| m.id),
                );
                k += 1;
            }
            self.mobs.retain(|m| !gone.contains(&m.id));
            for g in gone {
                self.detach_entity(g);
            }
            self.portals.arrivals.push(Arrival { plan, tag });
            self.portals.dirty = true;
        } else if let Some(i) = self.boats.iter().position(|b| b.id == id) {
            let b = &self.boats[i];
            let (width, height) = (super::boats::WIDTH as f32, super::boats::HEIGHT as f32);
            let plan = self.entity_plan(entry, b.position, width, height, false);
            if !self.may_enter(plan.dim) {
                return;
            }
            self.release_vehicle_container(id);
            let uuid = self.uuid_for(id);
            let b = &self.boats[i];
            let tag = horizon_client::entity_nbt::boat_to_nbt(
                &b.entity_type(),
                b.position,
                b.velocity,
                b.yaw,
                uuid,
                self.persist_raw(id),
                b.chest.then_some(&b.items[..]),
            );
            self.boats.remove(i);
            self.detach_entity(id);
            self.portals.arrivals.push(Arrival { plan, tag });
            self.portals.dirty = true;
        } else if let Some(i) = self.carts.iter().position(|c| c.id == id) {
            let c = &self.carts[i];
            let (width, height) = (
                super::minecarts::WIDTH as f32,
                super::minecarts::HEIGHT as f32,
            );
            let plan = self.entity_plan(entry, c.position, width, height, false);
            if !self.may_enter(plan.dim) {
                return;
            }
            self.release_vehicle_container(id);
            let uuid = self.uuid_for(id);
            let tag = self.carts[i].to_nbt(uuid, self.persist_raw(id));
            self.carts.remove(i);
            self.detach_entity(id);
            self.portals.arrivals.push(Arrival { plan, tag });
            self.portals.dirty = true;
        } else if let Some(i) = self
            .creatures
            .projectiles
            .iter()
            .position(|p| p.id == id && (is_arrow(p) || is_portal_projectile(p)))
        {
            let p = &self.creatures.projectiles[i];
            let (width, height) = projectile_box(p);
            let plan = self.entity_plan(entry, p.position, width, height, false);
            if !self.may_enter(plan.dim) {
                return;
            }
            let uuid = self.uuid_for(id);
            let p = &self.creatures.projectiles[i];
            let in_block = p
                .arrow
                .as_ref()
                .and_then(|a| a.last_state)
                .map(|s| self.state_to_nbt(s));
            let raw = self.persist_raw(id);
            let Some(tag) =
                horizon_client::entity_nbt::arrow_to_nbt(p, uuid, p.owner_uuid, in_block, raw)
                    .or_else(|| {
                        horizon_client::entity_nbt::fireball_to_nbt(p, uuid, p.owner_uuid, raw)
                    })
                    .or_else(|| {
                        horizon_client::entity_nbt::throwable_to_nbt(p, uuid, p.owner_uuid, raw)
                    })
            else {
                return;
            };
            self.creatures.projectiles.remove(i);
            self.forget_entity(id);
            self.portals.arrivals.push(Arrival { plan, tag });
            self.portals.dirty = true;
        }
    }
    /// The chunks of the arrivals still waiting to land in `dim`, for the
    /// streaming `extra` areas that light them: vanilla's portal ticket
    /// (`Entity.teleport` adds `TicketType.PORTAL` at the destination) keeps
    /// the area loaded without a player.
    pub(super) fn arrival_chunks(&self, dim: Dim) -> Vec<(i32, i32)> {
        self.portals
            .arrivals
            .iter()
            .filter(|a| a.plan.dim == dim)
            .map(|a| (a.plan.approximate.x >> 4, a.plan.approximate.z >> 4))
            .collect()
    }
    /// Entities waiting for the loaded dimension whose exit area is loaded:
    /// their exit portal (found or built) or End destination, position,
    /// turned yaw and motion, and portal cooldown; they join the world with the chunk entities.
    pub(super) fn resolve_arrivals(&mut self) {
        if self.remote.is_some() {
            return;
        }
        let mut i = 0;
        while i < self.portals.arrivals.len() {
            let plan = self.portals.arrivals[i].plan;
            let (cx, cz) = (plan.approximate.x >> 4, plan.approximate.z >> 4);
            let ready = plan.dim == self.dim
                && (-1..=1).all(|dz| (-1..=1).all(|dx| self.streaming.is_lit(cx + dx, cz + dz)));
            if !ready {
                i += 1;
                continue;
            }
            let Arrival { plan, mut tag } = self.portals.arrivals.remove(i);
            self.portals.dirty = true;
            let motion = horizon_client::entity_nbt::motion(&tag);
            let [yaw, pitch] = horizon_client::entity_nbt::rotation(&tag);
            let (at, motion, [yaw, pitch]) = if plan.end {
                // Relative.DELTA: the motion is kept.
                let at = self.end_portal_exit(&plan);
                // EndPortalBlock.getPortalDestination: PLACE_PORTAL_TICKET.
                self.place_portal_ticket(Self::chunk_of(at));
                let into_end = plan.dim == Dim::End;
                let turn =
                    end_portal_rotation(into_end, [yaw, pitch], self.level.world_spawn_angle);
                (at, motion, turn)
            } else {
                let Some(exit) = self.nether_exit_portal(&plan) else {
                    continue;
                };
                let (at, rotation) = self.arrival(&plan, exit);
                (at, rotate_y(motion, -rotation), [yaw + rotation, pitch])
            };
            if let Tag::Compound(c) = &mut tag {
                c.insert("Pos".into(), doubles([at.x, at.y, at.z]));
                c.insert("Motion".into(), doubles([motion.x, motion.y, motion.z]));
                c.insert(
                    "Rotation".into(),
                    Tag::List(vec![Tag::Float(yaw.to_bits()), Tag::Float(pitch.to_bits())]),
                );
                // setPortalCooldown before the teleport, copied by restoreFrom.
                c.insert("PortalCooldown".into(), Tag::Int(ENTITY_DELAY));
            }
            // ServerLevel.addDuringTeleport: a fresh add, not loaded from disk.
            super::persist::mark_fresh(&mut tag);
            let key = ((at.x.floor() as i32) >> 4, (at.z.floor() as i32) >> 4);
            match self
                .world
                .loaded_entities
                .iter_mut()
                .find(|(k, _)| *k == key)
            {
                Some((_, tags)) => tags.push(tag),
                None => self.world.loaded_entities.push((key, vec![tag])),
            }
        }
    }

    /// `FallingBlockEntity.tick`: block effects mark its portal, then
    /// `Entity.handlePortal` transfers it before the landing branch runs.
    pub(super) fn falling_through_portal(&mut self, f: &mut super::FallingBlock) -> Option<Dim> {
        if self.remote.is_some() {
            return None;
        }
        let mut sections = std::mem::take(&mut self.portals.sections);
        let entry = self.swept_portal(f.previous, f.position, 0.98, 0.98, &mut sections);
        self.portals.sections = sections;
        if let Some(entry) = entry {
            f.portal.set_inside(entry, ENTITY_DELAY);
        }
        let entry = f.portal.handle(true, 0, ENTITY_DELAY)?;
        let plan = self.entity_plan(entry, f.position, 0.98, 0.98, false);
        if !self.may_enter(plan.dim) {
            return None;
        }
        let uuid = f.saved.as_ref().map_or_else(
            || {
                if f.id == 0 {
                    horizon_client::save::random_uuid()
                } else {
                    self.uuid_for(f.id)
                }
            },
            |saved| saved.0,
        );
        let modelled = horizon_client::entity_nbt::falling_block_to_nbt(
            self.state_to_nbt(f.state),
            f.position,
            f.velocity,
            f.age,
            f.drop_item,
            f.damage,
            f.fall_distance,
            uuid,
        );
        let mut tag = super::persist::with_raw(modelled, f.saved.as_ref().map(|s| &s.1));
        if let Tag::Compound(fields) = &mut tag {
            fields.insert("PortalCooldown".into(), Tag::Int(ENTITY_DELAY));
            if f.no_gravity {
                fields.insert("NoGravity".into(), Tag::Byte(1));
            } else {
                fields.remove(&"NoGravity".into());
            }
        }
        if f.id != 0 {
            self.forget_entity(f.id);
        }
        self.portals.arrivals.push(Arrival { plan, tag });
        self.portals.dirty = true;
        Some(plan.dim)
    }

    // ---- persistence ----

    fn arrivals_path(&self) -> std::path::PathBuf {
        self.world_root
            .join("data")
            .join("horizon_portal_arrivals.dat")
    }
    /// The waiting entities of the open world (read once).
    pub(super) fn load_portal_arrivals(&mut self) {
        if self.portals.loaded || self.remote.is_some() {
            return;
        }
        self.portals.loaded = true;
        let Some(root) = horizon_client::save::read_dat(&self.arrivals_path()) else {
            return;
        };
        let Some(Tag::List(list)) = root.get("arrivals") else {
            return;
        };
        self.portals
            .arrivals
            .extend(list.iter().filter_map(arrival_from_tag));
    }
    /// Write the waiting entities when they changed.
    pub(super) fn save_portal_arrivals(&mut self) {
        if !self.portals.dirty || self.remote.is_some() {
            return;
        }
        let list = self
            .portals
            .arrivals
            .iter()
            .map(|arrival| {
                let mut tag = arrival_tag(arrival);
                if let Tag::Compound(root) = &mut tag
                    && let Some(entity) = root.get_mut(&"entity".into())
                {
                    super::persist::write_attachments(entity, &self.entity_attachments);
                }
                tag
            })
            .collect();
        let root = Tag::compound([("arrivals", Tag::List(list))]);
        if horizon_client::save::write_dat(&self.arrivals_path(), root).is_ok() {
            self.portals.dirty = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_portals_lead_to_the_end_spawn_and_back_to_the_world_spawn() {
        let spawn = BlockPos::new(-7, 70, 12);
        for from in [Dim::Overworld, Dim::Nether] {
            let plan = Plan::end_portal(from, spawn, 0.9, 1.4);
            assert_eq!(plan.dim, Dim::End);
            assert_eq!(plan.approximate, BlockPos::new(100, 50, 0));
            assert!(plan.end);
        }
        let plan = Plan::end_portal(Dim::End, spawn, 0.25, 0.25);
        assert_eq!(plan.dim, Dim::Overworld);
        assert_eq!(plan.approximate, spawn);
    }

    #[test]
    fn the_end_portal_shape_takes_falling_items_and_standing_mobs() {
        let portal = BlockPos::new(0, 100, 3);
        let at = |y| Vec3::new(0.5, y, 3.5);
        // A dropped item falling through the slab, or in it.
        assert!(touches_end_portal(
            at(100.6),
            at(100.56),
            0.25,
            0.25,
            portal
        ));
        // One tick's fall across the whole slab, in it at neither end.
        assert!(touches_end_portal(at(101.0), at(99.9), 0.25, 0.25, portal));
        // Resting below the slab (on the floor) or above it.
        assert!(!touches_end_portal(
            at(100.0),
            at(100.0),
            0.25,
            0.25,
            portal
        ));
        assert!(!touches_end_portal(
            at(100.8),
            at(100.8),
            0.25,
            0.25,
            portal
        ));
        // A cow standing in the portal block.
        assert!(touches_end_portal(at(100.0), at(100.0), 0.9, 1.4, portal));
        // Beside it.
        let beside = Vec3::new(2.0, 100.0, 3.5);
        assert!(!touches_end_portal(beside, beside, 0.9, 1.4, portal));
    }

    #[test]
    fn end_portals_turn_entities_like_their_teleport_transition() {
        // Into the End: facing west, pitch kept.
        assert_eq!(
            end_portal_rotation(true, [-30.0, 20.0], [45.0, 10.0]),
            [90.0, 20.0]
        );
        // Out of it: turned by the world spawn's yaw and pitch, pitch clamped.
        assert_eq!(
            end_portal_rotation(false, [-30.0, 20.0], [45.0, 10.0]),
            [15.0, 30.0]
        );
        assert_eq!(
            end_portal_rotation(false, [0.0, 80.0], [0.0, 30.0]),
            [0.0, 90.0]
        );
    }

    #[test]
    fn waiting_arrivals_keep_their_end_destination_when_saved() {
        let tag = Tag::compound([("id", Tag::String("minecraft:cow".into()))]);
        for (from, dim) in [(Dim::Overworld, Dim::End), (Dim::End, Dim::Overworld)] {
            let arrival = Arrival {
                plan: Plan::end_portal(from, BlockPos::new(3, 64, -5), 0.9, 1.4),
                tag: tag.clone(),
            };
            let back = arrival_from_tag(&arrival_tag(&arrival)).unwrap();
            assert_eq!(back.plan.dim, dim);
            assert_eq!(back.plan.approximate, arrival.plan.approximate);
            assert!(back.plan.end);
            assert_eq!(back.tag, tag);
        }
        // Arrivals saved before End portals were Nether portal plans.
        let mut old = arrival_tag(&Arrival {
            plan: Plan::end_portal(Dim::End, BlockPos::new(0, 64, 0), 0.25, 0.25),
            tag,
        });
        if let Tag::Compound(c) = &mut old {
            c.remove(&"end".into());
        }
        assert!(!arrival_from_tag(&old).unwrap().plan.end);
    }
}
