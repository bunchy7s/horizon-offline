//! The End: End portal frames filled with eyes of ender, End portal and End
//! gateway travel, the obsidian arrival platform, the exit portal and the End
//! sky. The dragon fight itself lives in game/dragon.rs: the podium is
//! inactive until the dragon dies.
use super::{Dim, Game};
use horizon_client::{
    Result,
    containers::{BlockEntity, EndGateway},
    environment::Environment,
    items::ItemStack,
    mesher::Vertex,
    motion::PlayerMotion,
    particles::{ParticleOptions, ParticleType},
    vanillagen::VanillaTerrain,
    world_spawn::PlayerSpawnFinder,
};
use horizon_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};
use std::{collections::HashMap, sync::Arc};

/// ServerLevel.END_SPAWN_POINT.
pub(super) const END_SPAWN: [i32; 3] = [100, 50, 0];
/// EnderDragonFight gateway ring radius and height.
const GATEWAY_RADIUS: f64 = 96.0;
const GATEWAY_Y: i32 = 75;
/// File (in the End region folder) keeping End gateway exits.
const GATEWAY_FILE: &str = "horizon-end-gateways.json";

/// A gateway teleport waiting for its destination chunks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GatewayJob {
    /// TheEndGatewayBlockEntity.findOrCreateValidTeleportPos from this gateway.
    /// `exact` is the gateway's own `ExactTeleport`, read while its chunk is loaded.
    Find { gateway: [i32; 3], exact: bool },
    /// A known exit (exact, or findExitPosition around it).
    Known { exit: [i32; 3], exact: bool },
}

impl GatewayJob {
    /// TheEndGatewayBlockEntity.getPortalPosition, with legacy save fallbacks.
    fn for_gateway(
        gateway: [i32; 3],
        entity: Option<&BlockEntity>,
        legacy: Option<([i32; 3], bool)>,
    ) -> Self {
        let configured = match entity {
            Some(BlockEntity::EndGateway(g)) => g.exit_portal.map(|exit| (exit, g.exact_teleport)),
            _ => None,
        };
        match configured.or(legacy) {
            Some((exit, exact)) => Self::Known { exit, exact },
            // Worldgen return gateways (end_gateway_return) lead exactly to the End spawn.
            None if f64::from(gateway[0]).hypot(f64::from(gateway[2])) > 512.0 => Self::Known {
                exit: END_SPAWN,
                exact: true,
            },
            None => Self::Find {
                gateway,
                exact: exact_teleport(entity),
            },
        }
    }
}

/// `TheEndGatewayBlockEntity.exactTeleport` of the block entity at a gateway
/// (false without one).
fn exact_teleport(entity: Option<&BlockEntity>) -> bool {
    matches!(entity, Some(BlockEntity::EndGateway(g)) if g.exact_teleport)
}

/// Moves an exit that only the side file knew into the gateway's block entity, unless the block
/// entity has its own; true when it took the saved one.
fn adopt_saved_exit(entity: &mut EndGateway, (exit, exact): ([i32; 3], bool)) -> bool {
    if entity.exit_portal.is_some() {
        return false;
    }
    entity.exit_portal = Some(exit);
    entity.exact_teleport = exact;
    true
}

/// The end of `getPortalPosition`: the exit itself when the teleport is exact,
/// else the spot [`Game::resolve_gateway`] finds around it (`findExitPosition`).
fn exit_target(exit: [i32; 3], exact: bool, find: impl FnOnce([i32; 3]) -> [i32; 3]) -> [i32; 3] {
    if exact { exit } else { find(exit) }
}

#[derive(Default)]
pub(super) struct EndState {
    /// Gateway position -> (exit, exact teleport); loaded lazily from the save.
    exits: Option<HashMap<[i32; 3], ([i32; 3], bool)>>,
    job: Option<GatewayJob>,
    /// Ticks until the player can use a gateway again (TheEndGatewayBlockEntity cooldown).
    gateway_cooldown: u32,
    /// The Overworld generator, for stronghold positions (ServerLevel.findNearestMapStructure).
    locator: std::sync::Mutex<Option<Arc<VanillaTerrain>>>,
    /// Thrown eyes of ender in flight.
    eyes: Vec<EyeOfEnder>,
    flash: EndFlash,
}

/// EndFlashState: a purple flash in the End sky every 600 ticks that also
/// lights the world (the lightmap's sky factor).
#[derive(Default)]
struct EndFlash {
    seed: Option<i64>,
    offset: i32,
    duration: i32,
    intensity: f32,
    old_intensity: f32,
    x_angle: f32,
    y_angle: f32,
    /// Ticks until the delayed weather.end_flash sound.
    sound_in: Option<u32>,
}
impl EndFlash {
    fn tick(&mut self, clock: i64) {
        let seed = clock.div_euclid(600);
        if self.seed != Some(seed) {
            let mut random = LegacyRandom::new(seed);
            random.next_float();
            let between = |r: &mut LegacyRandom, lo: i32, hi: i32| {
                r.next_int_bound(hi - lo + 1).unwrap_or(0) + lo
            };
            self.offset = between(&mut random, 0, 200);
            self.duration = between(&mut random, 100, 380.min(600 - self.offset));
            self.x_angle = random.next_float() * 70.0 - 60.0;
            self.y_angle = random.next_float() * 360.0 - 180.0;
            self.seed = Some(seed);
        }
        self.old_intensity = self.intensity;
        let t = clock.rem_euclid(600) as i32;
        self.intensity = if t >= self.offset && t <= self.offset + self.duration {
            ((t - self.offset) as f32 * std::f32::consts::PI / self.duration as f32).sin()
        } else {
            0.0
        };
        if self.intensity > 0.0 && self.old_intensity <= 0.0 {
            self.sound_in = Some(30);
        }
    }
    fn intensity(&self, partial: f32) -> f32 {
        self.old_intensity + (self.intensity - self.old_intensity) * partial
    }
}
impl EndState {
    pub(super) fn locator(&self) -> Option<Arc<VanillaTerrain>> {
        self.locator.lock().unwrap().clone()
    }
    pub(super) fn set_locator(&self, terrain: Option<Arc<VanillaTerrain>>) {
        *self.locator.lock().unwrap() = terrain;
    }
}

impl EndState {
    /// The eyes' entity ids, for a server to number them.
    pub(super) fn eye_ids(&mut self) -> impl Iterator<Item = &mut u32> {
        self.eyes.iter_mut().map(|e| &mut e.id)
    }
    /// The numbered eyes as a server's clients track them: (id, position, motion).
    pub(super) fn eyes_tracked(&self) -> impl Iterator<Item = (u32, [f64; 3], [f64; 3])> {
        self.eyes
            .iter()
            .filter(|e| e.id != 0)
            .map(|e| (e.id, e.position.to_array(), e.velocity.to_array()))
    }
    /// `handleAddEntity` for an eye of ender on a server's client: it flies
    /// where the server says.
    pub(super) fn add_remote_eye(&mut self, id: u32, at: [f64; 3], velocity: [f64; 3]) {
        self.eyes.retain(|e| e.id != id);
        let at = glam::DVec3::from_array(at);
        self.eyes.push(EyeOfEnder {
            id,
            position: at,
            previous: at,
            velocity: glam::DVec3::from_array(velocity),
            target: at,
            life: 0,
            survive: false,
        });
    }
    /// Where each eye in flight was last tick and is now (the hitbox view).
    pub(super) fn eye_motions(&self) -> impl Iterator<Item = (glam::DVec3, glam::DVec3)> + '_ {
        self.eyes.iter().map(|e| (e.previous, e.position))
    }
    /// `EyeOfEnder.tick` on a server's client: it moves by its last known
    /// motion; only the server steers it.
    pub(super) fn step_remote_eyes(&mut self) {
        for e in &mut self.eyes {
            e.previous = e.position;
            e.life += 1;
            e.position += e.velocity;
        }
    }
    /// A server's move: an eye has no interpolation handler, so
    /// `moveOrInterpolateTo` puts it there at once. False when `id` isn't one.
    pub(super) fn move_remote_eye(&mut self, id: u32, at: [f64; 3]) -> bool {
        let Some(e) = self.eyes.iter_mut().find(|e| e.id == id) else {
            return false;
        };
        e.position = glam::DVec3::from_array(at);
        true
    }
    /// `handleSetEntityMotion` for an eye. False when `id` isn't one.
    pub(super) fn remote_eye_motion(&mut self, id: u32, velocity: [f64; 3]) -> bool {
        let Some(e) = self.eyes.iter_mut().find(|e| e.id == id) else {
            return false;
        };
        e.velocity = glam::DVec3::from_array(velocity);
        true
    }
    pub(super) fn remove_remote_eye(&mut self, id: u32) {
        self.eyes.retain(|e| e.id != id);
    }
    pub(super) fn clear_remote_eyes(&mut self) {
        self.eyes.clear();
    }
    /// Where each eye is, for `status`.
    pub(super) fn eye_positions(&self) -> impl Iterator<Item = (u32, [f64; 3])> {
        self.eyes.iter().map(|e| (e.id, e.position.to_array()))
    }
}

/// The EyeOfEnder entity.
struct EyeOfEnder {
    /// Its entity id once a server's clients track it (0 until then).
    id: u32,
    position: glam::DVec3,
    previous: glam::DVec3,
    velocity: glam::DVec3,
    target: glam::DVec3,
    life: u32,
    survive: bool,
}
impl EyeOfEnder {
    /// EyeOfEnder.updateDeltaMovement.
    fn update_delta(old: glam::DVec3, position: glam::DVec3, target: glam::DVec3) -> glam::DVec3 {
        let horizontal = glam::DVec3::new(target.x - position.x, 0.0, target.z - position.z);
        let length = horizontal.length();
        let old_horizontal = old.x.hypot(old.z);
        let mut wanted_speed = old_horizontal + 0.0025 * (length - old_horizontal);
        let mut y = old.y;
        if length < 1.0 {
            wanted_speed *= 0.8;
            y *= 0.8;
        }
        let wanted_y = if position.y - old.y < target.y {
            1.0
        } else {
            -1.0
        };
        horizontal * (wanted_speed / length)
            + glam::DVec3::new(0.0, y + (wanted_y - y) * 0.015, 0.0)
    }
}

impl Game<'_> {
    /// The environment attributes of the current dimension.
    pub(super) fn dim_environment(&self) -> &Environment {
        match self.dim {
            Dim::Overworld => self.res.environment(),
            Dim::Nether => self.res.nether_environment(),
            Dim::End => self.res.end_environment(),
        }
    }

    /// SkyRenderer.renderEndFlash, drawn after the renderer's resident sky mesh.
    pub(super) fn end_flash_geometry(&self) -> Vec<Vertex> {
        let mut out = Vec::new();
        // The old combined sky/flash draw also skipped the flash without a sky sprite.
        if !self.gui.atlas.sprites.contains_key("environment/end_sky") {
            return out;
        }
        // SkyRenderer.renderEndFlash: the end_flash sprite at height 100, scale 60.
        let intensity = self.end.flash.intensity(self.clock.fraction() as f32);
        if intensity > 1e-5
            && let Some(r) = self
                .gui
                .atlas
                .sprites
                .get("environment/celestial/end_flash")
                .copied()
        {
            let flash = &self.end.flash;
            let pose = glam::Mat4::from_rotation_y((180.0 - flash.y_angle).to_radians())
                * glam::Mat4::from_rotation_x((-90.0 - flash.x_angle).to_radians());
            let level = (intensity.clamp(0.0, 1.0) * 255.0) as u32;
            let color = level << 24 | level << 16 | level << 8 | level;
            let corners = [
                (-1.0, -1.0, [r.u0, r.v0]),
                (1.0, -1.0, [r.u1, r.v0]),
                (1.0, 1.0, [r.u1, r.v1]),
                (-1.0, 1.0, [r.u0, r.v1]),
            ];
            for (x, z, uv) in corners {
                let p = pose.transform_point3(glam::Vec3::new(x * 60.0, 100.0, z * 60.0));
                out.push(Vertex {
                    normal: [0; 4],
                    pos: p.to_array(),
                    uv,
                    layer: 2,
                    color,
                    light: 0,

                    glint_uv: [0.0, 0.0],
                });
            }
        }
        out
    }

    /// LightmapRenderStateExtractor: End flashes add to the sky light factor
    /// unless Hide Lightning Flashes is on (the sky sprite is not gated), a
    /// third of it under a boss bar's world fog.
    pub(super) fn end_flash_sky(&self, sky: &mut horizon_client::environment::Sky, partial: f64) {
        if self.dim == Dim::End && !self.settings.file.bool("hideLightningFlashes") {
            let intensity = self.end.flash.intensity(partial as f32);
            sky.sky_light_factor += if self.boss_overlay.should_create_world_fog() {
                intensity / 3.0
            } else {
                intensity
            };
        }
    }

    pub(super) fn block_name(&self, p: [i32; 3]) -> std::sync::Arc<str> {
        self.res
            .facts()
            .get(self.world.block(p[0], p[1], p[2]))
            .name
            .clone()
    }

    /// EnderEyeItem.use's `getPlayerPOVHitResult`: whether the block aimed
    /// at is an End portal frame (the use passes there).
    pub(super) fn aims_at_end_portal_frame(&self) -> bool {
        self.pick()
            .is_some_and(|hit| &*self.block_name(hit.pos) == "minecraft:end_portal_frame")
    }

    /// EnderEyeItem.useOn: fill an empty End portal frame, then light the
    /// portal when the ring of twelve filled frames is complete.
    pub(super) fn use_ender_eye(&mut self, stack: &ItemStack, use_on_block: bool) -> Result<bool> {
        if &*stack.item != "minecraft:ender_eye" {
            return Ok(false);
        }
        let registry = self.pack.assets.registry;
        let target = self.pick().and_then(|hit| {
            let state = registry.state(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]))?;
            (state.name == "minecraft:end_portal_frame").then(|| (hit, state.clone()))
        });
        let Some((hit, state)) = target else {
            // EnderEyeItem.useOn passes off a frame: a guest's UseItem throws it.
            if matches!(self.guest_use, Some(super::GuestUse::OnBlock(_))) {
                return Ok(false);
            }
            // EnderEyeItem.use: startUsingItem (getUseDuration 0, so the use
            // lasts until the button lets go), then the eye is thrown towards
            // the nearest stronghold if there is one (CONSUME without).
            if self.using.is_none() {
                self.using = Some((self.held_slot(), 0));
            }
            self.throw_ender_eye();
            return Ok(true);
        };
        if !use_on_block {
            // EnderEyeItem.use passes on a frame; only useOn can insert an eye.
            return Ok(false);
        }
        if state.properties.get("eye").map(String::as_str) == Some("true") {
            // Both useOn and use pass on a filled frame.
            return Ok(true);
        }
        let mut filled = state;
        filled.properties.insert("eye".into(), "true".into());
        let id = registry.resolve(&filled)?;
        self.level_set(hit.pos, id)?;
        // Block.pushEntitiesUp: the eye raises the frame's top from 13/16 to a full block.
        if let Some(p) = self.player.as_mut() {
            let top = f64::from(hit.pos[1]) + 1.0;
            let w = f64::from(p.dimensions.width) / 2.0;
            let over = |c: f64, b: i32| c + w > f64::from(b) && c - w < f64::from(b) + 1.0;
            if over(p.position.x, hit.pos[0])
                && over(p.position.z, hit.pos[2])
                && p.position.y >= top - 0.25
                && p.position.y < top
            {
                p.position.y = top;
            }
        }
        self.consume_held();
        self.swing_arm();
        // EnderEyeItem.useOn emits this event for all nearby players.
        self.level_event_at(1503, hit.pos, 0);
        if let Some(c) = self.portal_ring_center(hit.pos) {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    self.set_state_named(
                        [c[0] + dx, c[1], c[2] + dz],
                        "minecraft:end_portal",
                        &[],
                    )?;
                }
            }
            // EnderEyeItem.useOn: global level event 1038 (END_PORTAL_SPAWN),
            // heard by every player (vanilla places it at the frame's corner
            // + (1, 0, 1); only its direction matters).
            self.global_level_event(1038, c, 0);
        }
        Ok(true)
    }

    /// EndPortalFrameBlock.getOrCreatePortalShape matched around `frame`: a 5×5
    /// ring (corners free) of filled frames, all facing the centre or all
    /// facing away from it. Returns the centre of the 3×3 opening.
    fn portal_ring_center(&self, frame: [i32; 3]) -> Option<[i32; 3]> {
        let registry = self.pack.assets.registry;
        // (offset from the centre, facing when the frames face the centre)
        let mut ring = Vec::with_capacity(12);
        for i in -1..=1 {
            ring.push(([i, -2], "south"));
            ring.push(([i, 2], "north"));
            ring.push(([-2, i], "east"));
            ring.push(([2, i], "west"));
        }
        let opposite = |f: &str| match f {
            "south" => "north",
            "north" => "south",
            "east" => "west",
            _ => "east",
        };
        for &([ox, oz], _) in &ring {
            let c = [frame[0] - ox, frame[1], frame[2] - oz];
            for inward in [true, false] {
                let ok = ring.iter().all(|&([dx, dz], facing)| {
                    let id = self.world.block(c[0] + dx, c[1], c[2] + dz);
                    let Some(s) = registry.state(id) else {
                        return false;
                    };
                    let want = if inward { facing } else { opposite(facing) };
                    s.name == "minecraft:end_portal_frame"
                        && s.properties.get("eye").map(String::as_str) == Some("true")
                        && s.properties.get("facing").map(String::as_str) == Some(want)
                });
                if ok {
                    return Some(c);
                }
            }
        }
        None
    }

    /// End portals, End gateways and the exit portal, once per tick.
    pub(super) fn tick_end(&mut self) -> Result<()> {
        self.end.gateway_cooldown = self.end.gateway_cooldown.saturating_sub(1);
        if self.dim == Dim::End {
            self.tick_fight();
            self.tick_dragon_fireballs();
        }
        self.tick_crystals();
        self.tick_eyes();
        if self.dim == Dim::End {
            self.end.flash.tick(self.world.game_time);
            if let Some(n) = self.end.flash.sound_in.as_mut() {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    self.end.flash.sound_in = None;
                    self.audio
                        .play("minecraft:weather.end_flash", None, 1.0, 1.0);
                }
            }
        }
        if self.dim == Dim::End {
            self.ensure_exit_portal()?;
        }
        let Some(p) = self.player.as_ref() else {
            return Ok(());
        };
        if self.travelling.is_some() || p.health.dead_or_dying() {
            return Ok(());
        }
        let (portal, gateway) = self.inside_end_portals(
            p.position,
            f64::from(p.dimensions.width),
            f64::from(p.dimensions.height),
        );
        if portal && self.dim == Dim::End && !self.level.seen_credits {
            // EndPortalBlock.entityInside: the first exit shows the credits.
            self.show_end_credits()?;
        } else if portal {
            if self.portals.player.cooldown == 0 {
                let target = if self.dim == Dim::End {
                    Dim::Overworld
                } else {
                    Dim::End
                };
                self.end_portal_travel = true;
                let result = self.enter_dimension(target);
                self.end_portal_travel = false;
                result?;
                // Player.getDimensionChangingDelay.
                self.portals.player.cooldown = 10;
            } else {
                // Entity.setAsInsidePortal while cooling down restarts the cooldown.
                self.portals.player.cooldown = 10;
            }
        } else if let Some(g) = gateway
            && self.end.gateway_cooldown == 0
            && self.gateway_enter(g)
        {
            self.enter_gateway(g)?;
        }
        Ok(())
    }

    /// Whether a `width` by `height` box at `position` is inside an End
    /// portal (its entity-inside shape), and the End gateway it touches.
    pub(super) fn inside_end_portals(
        &self,
        position: Vec3,
        width: f64,
        height: f64,
    ) -> (bool, Option<[i32; 3]>) {
        let w = width / 2.0;
        let lo = [position.x - w, position.y, position.z - w];
        let hi = [position.x + w, position.y + height, position.z + w];
        let mut portal = false;
        let mut gateway = None;
        for x in lo[0].floor() as i32..=hi[0].floor() as i32 {
            for y in lo[1].floor() as i32..=hi[1].floor() as i32 {
                for z in lo[2].floor() as i32..=hi[2].floor() as i32 {
                    let name = self.block_name([x, y, z]);
                    // EndPortalBlock entity-inside shape: y 6/16..12/16.
                    if &*name == "minecraft:end_portal"
                        && lo[1] < f64::from(y) + 0.75
                        && hi[1] > f64::from(y) + 0.375
                    {
                        portal = true;
                    } else if &*name == "minecraft:end_gateway" && self.dim == Dim::End {
                        gateway = Some([x, y, z]);
                    }
                }
            }
        }
        (portal, gateway)
    }

    /// EndPortalBlock.getPortalDestination for the pending enter_dimension.
    pub(super) fn end_destination(&self, target: Dim) -> Option<Vec3> {
        if !self.end_portal_travel {
            return None;
        }
        if target == Dim::End {
            // Vec3.atBottomCenterOf(END_SPAWN_POINT), one block lower for players.
            return Some(Vec3::new(
                f64::from(END_SPAWN[0]) + 0.5,
                f64::from(END_SPAWN[1] - 1),
                f64::from(END_SPAWN[2]) + 0.5,
            ));
        }
        // Leaving the End: the respawn position, a bed's found once it has
        // loaded (findRespawnPositionAndUseSpawnBlock, not using it up).
        if let Some(c) = self.respawn_config.filter(|c| c.dim == target) {
            return Some(Vec3::new(
                f64::from(c.pos[0]) + 0.5,
                f64::from(c.pos[1]),
                f64::from(c.pos[2]) + 0.5,
            ));
        }
        let s = self.level.spawn.unwrap_or([0.0, 64.0, 0.0]);
        Some(Vec3::new(s[0].floor() + 0.5, s[1], s[2].floor() + 0.5))
    }

    /// Arrival after End portal or gateway travel, once the destination is loaded.
    pub(super) fn place_after_end_travel(
        &mut self,
        mut player: PlayerMotion,
        target: Vec3,
    ) -> Result<()> {
        self.end_travel = false;
        // Player.getDimensionChangingDelay (enter_dimension uses the Nether portal value).
        self.portals.player.cooldown = 10;
        if player.health.dead_or_dying() {
            // Respawning out of the End: a fresh player at the spawn.
            player = PlayerMotion::new(self.motion, target)?;
            self.eye = 1.62;
            self.previous_eye = 1.62;
            self.clock.reset();
        }
        let position = if let Some(job) = self.end.job.take() {
            self.resolve_gateway(job, target)?
        } else {
            let mut finder = self.player_spawn.take();
            let arrival = self.end_arrival(target, &mut finder);
            self.player_spawn = finder;
            let Some((position, yaw)) = arrival? else {
                // A spawn candidate's chunk is loading: try again next tick.
                self.travelling = Some((player, target));
                self.end_travel = true;
                return Ok(());
            };
            if let Some(yaw) = yaw {
                self.yaw = yaw;
                player.yaw = yaw;
                player.head_yaw = yaw;
            }
            position
        };
        player.position = position;
        player.velocity = Vec3::ZERO;
        player.fall_distance = 0.0;
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

    /// Where an End portal's traveller lands once `target` has loaded, and
    /// the yaw it faces there: the platform below the End's spawn point,
    /// facing west, or the top of the Overworld's spawn column.
    /// None while a candidate of the world spawn's search loads (`finder`
    /// keeps it, see `search_spawn`).
    pub(super) fn end_arrival(
        &mut self,
        target: Vec3,
        finder: &mut Option<PlayerSpawnFinder>,
    ) -> Result<Option<(Vec3, Option<f32>)>> {
        if self.dim == Dim::End {
            // EndPlatformFeature.createEndPlatform below the spawn point.
            let origin = [END_SPAWN[0], END_SPAWN[1] - 1, END_SPAWN[2]];
            for dz in -2..=2 {
                for dx in -2..=2 {
                    for dy in -1..3 {
                        let p = [origin[0] + dx, origin[1] + dy, origin[2] + dz];
                        let name = if dy == -1 {
                            "minecraft:obsidian"
                        } else {
                            "minecraft:air"
                        };
                        if &*self.block_name(p) != name {
                            self.set_state_named(p, name, &[])?;
                        }
                    }
                }
            }
            // Direction.WEST.toYRot().
            return Ok(Some((target, Some(90.0))));
        }
        // ServerPlayer.findRespawnPositionAndUseSpawnBlock: where the bed or
        // anchor puts the player, facing it; else onto the spawn column.
        let mut target = target;
        if let Some(c) = self.respawn_config.filter(|c| c.dim == self.dim) {
            if let Some((at, yaw)) = self.find_respawn_and_use(c, false)? {
                return Ok(Some((at, Some(yaw))));
            }
            let s = self.level.spawn.unwrap_or([0.0, 64.0, 0.0]);
            target = Vec3::new(s[0].floor() + 0.5, s[1], s[2].floor() + 0.5);
        }
        let (x, z) = (target.x.floor() as i32, target.z.floor() as i32);
        // TeleportTransition.missingRespawnBlock: PlayerSpawnFinder around
        // the world spawn.
        if let Some(world) = self.level.world_spawn.filter(|w| w[0] == x && w[2] == z) {
            return Ok(match self.search_spawn(world, finder) {
                Ok(at) => Some((at, None)),
                Err(chunk) => {
                    self.spawn_wait.push(chunk);
                    None
                }
            });
        }
        let y = self.land_column(x, z).map_or(target.y, f64::from);
        Ok(Some((Vec3::new(target.x, y, target.z), None)))
    }

    /// Dying away from the Overworld respawns at the world spawn there.
    pub(super) fn respawn_out_of_end(&mut self) -> Result<()> {
        if self.dim == Dim::Overworld || self.player.is_none() {
            return Ok(());
        }
        self.end_portal_travel = true;
        let result = self.enter_dimension(Dim::Overworld);
        self.end_portal_travel = false;
        // The arrival places a fresh player at the spawn.
        self.end_travel = true;
        result
    }

    /// EnderDragonFight without a dragon: once the centre of the main island
    /// is loaded, place the active exit portal (EndPodiumFeature) and the
    /// first gateway (spawnNewGateway) if the portal is not there yet. Like
    /// scanState and spawnExitPortal, only while the fight has no exit portal
    /// location: a podium that is later broken stays broken.
    fn ensure_exit_portal(&mut self) -> Result<()> {
        if self.travelling.is_some()
            || !(-1..=0).all(|cz| (-1..=0).all(|cx| self.streaming.is_lit(cx, cz)))
            || !self.fight_ready()
        {
            return Ok(());
        }
        let won = self.fight.previously_killed;
        let gateway = self.first_gateway();
        if self.exit_portal().is_none() {
            // findExitPortal remembers the location of a podium that is already there.
            if let Some(level) = self.podium_level() {
                self.set_exit_portal([0, level, 0]);
            }
        }
        if self.exit_portal().is_some() {
            // The gateway may have been out of range when it was placed.
            if won
                && self.world.loaded(gateway[0], gateway[2])
                && &*self.block_name(gateway) != "minecraft:end_gateway"
                && &*self.block_name([gateway[0], gateway[1] + 2, gateway[2]])
                    != "minecraft:bedrock"
            {
                self.place_gateway(gateway)?;
            }
            return Ok(());
        }
        // getHeightmapPos(MOTION_BLOCKING_NO_LEAVES, podium location).below()
        let mut y = (0..256)
            .rev()
            .find(|&y| {
                let f = self.res.facts().get(self.world.block(0, y, 0));
                (f.full_collision || f.fluid != horizon_client::blocks::Fluid::None)
                    && !f.name.ends_with("_leaves")
            })
            .unwrap_or(0);
        while &*self.block_name([0, y, 0]) == "minecraft:bedrock" && y > 63 {
            y -= 1;
        }
        let origin = [0, y.max(1), 0];
        self.set_exit_portal(origin);
        self.place_podium(origin, won)?;
        if won && self.world.loaded(gateway[0], gateway[2]) {
            self.place_gateway(gateway)?;
        }
        Ok(())
    }
    /// The portal level of the podium around the centre column, read from the
    /// bedrock that EndPodiumFeature places (rim at the portal level, disc
    /// below it, the torch column) so a broken column does not hide it.
    fn podium_level(&self) -> Option<i32> {
        let mut offsets: Vec<[i32; 3]> = (0..4).map(|dy| [0, dy, 0]).collect();
        for z in -4..=4i32 {
            for x in -4..=4i32 {
                let flat = f64::from(x * x + z * z);
                if (6.25..12.25).contains(&flat) {
                    offsets.push([x, 0, z]);
                } else if flat + 1.0 < 6.25 {
                    offsets.push([x, -1, z]);
                }
            }
        }
        (1..253)
            .map(|y| {
                let score = offsets
                    .iter()
                    .filter(|o| &*self.block_name([o[0], y + o[1], o[2]]) == "minecraft:bedrock")
                    .count();
                (score, y)
            })
            .filter(|&(score, _)| score >= 3)
            .max()
            .map(|(_, y)| y)
    }
    /// `spawnExitPortal(true)` after the dragon dies.
    pub(super) fn activate_exit_portal(&mut self) -> Result<()> {
        let origin = match self.exit_portal() {
            Some(o) => o,
            None => {
                // The rim's level, found without assuming the centre column is intact.
                [0, self.podium_level().unwrap_or(61), 0]
            }
        };
        self.place_podium(origin, true)
    }
    pub(super) fn place_gateway_at(&mut self, pos: [i32; 3]) -> Result<()> {
        self.place_gateway(pos)
    }

    /// The first gateway of the shuffled ring (EnderDragonFight constructor
    /// and spawnNewGateway).
    fn first_gateway(&self) -> [i32; 3] {
        let mut gateways: Vec<i32> = (0..20).collect();
        let mut random = LegacyRandom::new(self.level.seed);
        for i in (2..=gateways.len()).rev() {
            let j = random.next_int_bound(i as i32).unwrap_or(0) as usize;
            gateways.swap(i - 1, j);
        }
        let index = f64::from(gateways[gateways.len() - 1]);
        let angle = 2.0 * (-std::f64::consts::PI + std::f64::consts::PI / 20.0 * index);
        [
            (GATEWAY_RADIUS * angle.cos()).floor() as i32,
            GATEWAY_Y,
            (GATEWAY_RADIUS * angle.sin()).floor() as i32,
        ]
    }

    /// EndPodiumFeature(active).place at `origin` (the portal level).
    pub(super) fn place_podium(&mut self, origin: [i32; 3], active: bool) -> Result<()> {
        for y in origin[1] - 1..=origin[1] + 32 {
            for z in origin[2] - 4..=origin[2] + 4 {
                for x in origin[0] - 4..=origin[0] + 4 {
                    let d =
                        (x - origin[0]).pow(2) + (y - origin[1]).pow(2) + (z - origin[2]).pow(2);
                    let d = f64::from(d);
                    let inside_rim = d < 2.5 * 2.5;
                    if !(inside_rim || d < 3.5 * 3.5) {
                        continue;
                    }
                    let name = if y < origin[1] {
                        if inside_rim {
                            "minecraft:bedrock"
                        } else {
                            "minecraft:end_stone"
                        }
                    } else if y > origin[1] {
                        "minecraft:air"
                    } else if !inside_rim {
                        "minecraft:bedrock"
                    } else if active {
                        "minecraft:end_portal"
                    } else {
                        "minecraft:air"
                    };
                    self.set_state_named([x, y, z], name, &[])?;
                }
            }
        }
        for dy in 0..4 {
            self.set_state_named(
                [origin[0], origin[1] + dy, origin[2]],
                "minecraft:bedrock",
                &[],
            )?;
        }
        let registry = self.pack.assets.registry;
        for (facing, d) in [
            ("north", [0, -1]),
            ("south", [0, 1]),
            ("west", [-1, 0]),
            ("east", [1, 0]),
        ] {
            let Some(default) = registry.default_state("minecraft:wall_torch") else {
                continue;
            };
            let mut state = registry.state(default).cloned().ok_or("wall torch")?;
            state.properties.insert("facing".into(), facing.into());
            let id = registry.resolve(&state)?;
            self.level_set([origin[0] + d[0], origin[1] + 2, origin[2] + d[1]], id)?;
        }
        Ok(())
    }

    /// EndGatewayFeature.place: the gateway block in a bedrock cross.
    fn place_gateway(&mut self, origin: [i32; 3]) -> Result<()> {
        for y in -2i32..=2 {
            for z in -1..=1 {
                for x in -1..=1 {
                    let (same_x, same_y, same_z) = (x == 0, y == 0, z == 0);
                    let end = y.abs() == 2;
                    let name = if same_x && same_y && same_z {
                        "minecraft:end_gateway"
                    } else if same_y {
                        "minecraft:air"
                    } else if end && same_x && same_z || (same_x || same_z) && !end {
                        "minecraft:bedrock"
                    } else {
                        "minecraft:air"
                    };
                    self.set_state_named([origin[0] + x, origin[1] + y, origin[2] + z], name, &[])?;
                }
            }
        }
        self.world
            .block_entities
            .get_or_insert_with((origin[0], origin[1], origin[2]), || {
                BlockEntity::EndGateway(Default::default())
            });
        Ok(())
    }

    fn gateway_exits(&mut self) -> &mut HashMap<[i32; 3], ([i32; 3], bool)> {
        if self.end.exits.is_none() {
            let path = horizon_client::save::dimension_dir(&self.world_root, "minecraft:the_end")
                .join(GATEWAY_FILE);
            let parsed: Option<Vec<([i32; 3], [i32; 3], bool)>> = std::fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok());
            self.end.exits = Some(
                parsed
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(g, e, x)| (g, (e, x)))
                    .collect(),
            );
        }
        self.end.exits.as_mut().expect("loaded exits")
    }

    fn save_gateway_exits(&mut self) -> Result<()> {
        let list: Vec<_> = self
            .gateway_exits()
            .iter()
            .map(|(g, (e, x))| (*g, *e, *x))
            .collect();
        let dir = horizon_client::save::dimension_dir(&self.world_root, "minecraft:the_end");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(GATEWAY_FILE), serde_json::to_vec(&list)?)?;
        Ok(())
    }

    /// Drops a gateway's entry from the side file: its block entity is the source of truth.
    fn forget_gateway_exit(&mut self, gateway: [i32; 3]) -> Result<()> {
        if self.gateway_exits().remove(&gateway).is_some() {
            self.save_gateway_exits()?;
        }
        Ok(())
    }

    fn set_gateway_exit(&mut self, gateway: [i32; 3], exit: [i32; 3], exact: bool) -> Result<()> {
        // TheEndGatewayBlockEntity.setExitPosition (also EndGatewayFeature.knownExit).
        let key = (gateway[0], gateway[1], gateway[2]);
        let chunk = (gateway[0] >> 4, gateway[2] >> 4);
        // The gateway's chunk can be unloaded by now (travel waits for the destination): the
        // exit then waits in the side file until the gateway is used again (`gateway_job`).
        if self.world.chunks.contains_key(&chunk)
            && let BlockEntity::EndGateway(g) = self
                .world
                .block_entities
                .get_or_insert_with(key, || BlockEntity::EndGateway(Default::default()))
        {
            g.exit_portal = Some(exit);
            g.exact_teleport = exact;
            self.world.modified.insert(chunk);
            return self.forget_gateway_exit(gateway);
        }
        self.gateway_exits().insert(gateway, (exit, exact));
        self.save_gateway_exits()
    }

    /// EndGatewayBlock.entityInside and TheEndGatewayBlockEntity.getPortalPosition:
    /// travel within the End, waiting for the destination chunks to load.
    fn enter_gateway(&mut self, gateway: [i32; 3]) -> Result<()> {
        // TheEndGatewayBlockEntity.triggerCooldown: 40 ticks.
        self.end.gateway_cooldown = 40;
        let (job, destination) = self.gateway_job(gateway);
        let Some(player) = self.player.take() else {
            return Ok(());
        };
        self.end.job = Some(job);
        self.end_travel = true;
        self.travelling = Some((player, destination));
        Ok(())
    }

    /// Where `gateway` leads (TheEndGatewayBlockEntity.getPortalPosition):
    /// its exit, or the search for one, and the spot to load before either
    /// is resolved ([`Game::resolve_gateway`]).
    pub(super) fn gateway_job(&mut self, gateway: [i32; 3]) -> (GatewayJob, Vec3) {
        let mut known = self.gateway_exits().get(&gateway).copied();
        // Once the gateway's chunk is loaded its block entity is the source of truth: an exit the
        // side file kept for it moves there, and the entry is dropped, so removing or replacing
        // the exit (`/data remove block`) is not undone by an old entry.
        let key = (gateway[0], gateway[1], gateway[2]);
        let chunk = (gateway[0] >> 4, gateway[2] >> 4);
        if let Some(saved) = known
            && self.world.chunks.contains_key(&chunk)
            && let BlockEntity::EndGateway(g) = self
                .world
                .block_entities
                .get_or_insert_with(key, || BlockEntity::EndGateway(Default::default()))
        {
            if adopt_saved_exit(g, saved) {
                self.world.modified.insert(chunk);
            }
            known = None;
            let _ = self.forget_gateway_exit(gateway);
        }
        let job = GatewayJob::for_gateway(
            gateway,
            self.world
                .block_entities
                .get(&(gateway[0], gateway[1], gateway[2])),
            known,
        );
        let destination = match job {
            GatewayJob::Known { exit, .. } => Vec3::new(
                f64::from(exit[0]) + 0.5,
                f64::from(exit[1]),
                f64::from(exit[2]) + 0.5,
            ),
            GatewayJob::Find { gateway, .. } => {
                let d = glam::DVec2::new(f64::from(gateway[0]), f64::from(gateway[2]))
                    .normalize_or_zero()
                    * 1024.0;
                Vec3::new(d.x, 75.0, d.y)
            }
        };
        (job, destination)
    }

    fn chunk_empty(&self, x: f64, z: f64) -> Option<bool> {
        let chunk = self
            .world
            .chunks
            .get(&((x / 16.0).floor() as i32, (z / 16.0).floor() as i32))?;
        Some(chunk.sections.iter().all(|s| s.is_uniform(0)))
    }

    /// TheEndGatewayBlockEntity.findTallestBlock.
    fn tallest_block(&self, around: [i32; 3], dist: i32, allow_bedrock: bool) -> [i32; 3] {
        let mut tallest: Option<[i32; 3]> = None;
        for xd in -dist..=dist {
            for zd in -dist..=dist {
                if xd == 0 && zd == 0 && !allow_bedrock {
                    continue;
                }
                let floor = tallest.map_or(0, |t| t[1]);
                let mut y = 255;
                while y > floor {
                    let p = [around[0] + xd, y, around[2] + zd];
                    let f = self.res.facts().get(self.world.block(p[0], p[1], p[2]));
                    if f.full_collision && (allow_bedrock || &*f.name != "minecraft:bedrock") {
                        tallest = Some(p);
                        break;
                    }
                    y -= 1;
                }
            }
        }
        tallest.unwrap_or(around)
    }

    /// The teleport position of a gateway job, creating the exit gateway (and
    /// an island when the chunk has no landing spot) for new destinations.
    pub(super) fn resolve_gateway(&mut self, job: GatewayJob, target: Vec3) -> Result<Vec3> {
        let bottom_center = |p: [i32; 3]| {
            Vec3::new(
                f64::from(p[0]) + 0.5,
                f64::from(p[1]),
                f64::from(p[2]) + 0.5,
            )
        };
        let find_exit = |game: &Self, exit: [i32; 3]| {
            let p = game.tallest_block([exit[0], exit[1] + 2, exit[2]], 5, false);
            [p[0], p[1] + 1, p[2]]
        };
        match job {
            GatewayJob::Known { exit, exact } => Ok(bottom_center(exit_target(exit, exact, |e| {
                find_exit(self, e)
            }))),
            GatewayJob::Find { gateway, exact } => {
                // findExitPortalXZPosTentative over the loaded chunks.
                let dir = glam::DVec2::new(f64::from(gateway[0]), f64::from(gateway[2]))
                    .normalize_or_zero();
                let mut pos = glam::DVec2::new(target.x, target.z);
                let mut limit = 16;
                while self.chunk_empty(pos.x, pos.y) == Some(false) && limit > 0 {
                    limit -= 1;
                    pos -= dir * 16.0;
                }
                let mut limit = 16;
                while self.chunk_empty(pos.x, pos.y) == Some(true) && limit > 0 {
                    limit -= 1;
                    pos += dir * 16.0;
                }
                let (cx, cz) = ((pos.x / 16.0).floor() as i32, (pos.y / 16.0).floor() as i32);
                // findValidSpawnInChunk: end stone with two non-full blocks above, closest to 0,0,0.
                let mut closest: Option<([i32; 3], f64)> = None;
                for y in 30..256 {
                    for z in cz * 16..cz * 16 + 16 {
                        for x in cx * 16..cx * 16 + 16 {
                            if &*self.block_name([x, y, z]) != "minecraft:end_stone"
                                || self
                                    .res
                                    .facts()
                                    .get(self.world.block(x, y + 1, z))
                                    .full_collision
                                || self
                                    .res
                                    .facts()
                                    .get(self.world.block(x, y + 2, z))
                                    .full_collision
                            {
                                continue;
                            }
                            let d = (f64::from(x) + 0.5).powi(2)
                                + (f64::from(y) + 0.5).powi(2)
                                + (f64::from(z) + 0.5).powi(2);
                            if closest.is_none_or(|c| d < c.1) {
                                closest = Some(([x, y, z], d));
                            }
                        }
                    }
                }
                let spot = match closest {
                    Some((p, _)) => p,
                    None => {
                        let p = [
                            (pos.x + 0.5).floor() as i32,
                            75,
                            (pos.y + 0.5).floor() as i32,
                        ];
                        self.place_end_island(p)?;
                        p
                    }
                };
                let tallest = self.tallest_block(spot, 16, true);
                let exit = [tallest[0], tallest[1] + 10, tallest[2]];
                self.place_gateway(exit)?;
                self.set_gateway_exit(exit, gateway, false)?;
                // setExitPosition keeps the gateway's own exactTeleport.
                self.set_gateway_exit(gateway, exit, exact)?;
                Ok(bottom_center(exit_target(exit, exact, |e| {
                    find_exit(self, e)
                })))
            }
        }
    }

    /// EndIslandFeature with RandomSource.create(pos.asLong()).
    fn place_end_island(&mut self, origin: [i32; 3]) -> Result<()> {
        let packed = ((i64::from(origin[0]) & 0x3FF_FFFF) << 38)
            | ((i64::from(origin[2]) & 0x3FF_FFFF) << 12)
            | (i64::from(origin[1]) & 0xFFF);
        let mut random = LegacyRandom::new(packed);
        let mut size = random.next_int_bound(3).unwrap_or(0) as f32 + 4.0;
        let mut y = 0;
        while size > 0.5 {
            for x in (-size).floor() as i32..=size.ceil() as i32 {
                for z in (-size).floor() as i32..=size.ceil() as i32 {
                    if (x * x + z * z) as f32 <= (size + 1.0) * (size + 1.0) {
                        self.set_state_named(
                            [origin[0] + x, origin[1] + y, origin[2] + z],
                            "minecraft:end_stone",
                            &[],
                        )?;
                    }
                }
            }
            size -= random.next_int_bound(2).unwrap_or(0) as f32 + 0.5;
            y -= 1;
        }
        Ok(())
    }

    /// End crystals (on the spikes, or placed in any dimension) and thrown
    /// eyes of ender, as camera-relative entity geometry (End portals and
    /// gateways are block entities).
    pub(super) fn end_geometry(&self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        self.end_crystal_geometry(camera, out);
        self.eye_geometry(camera, out);
    }

    /// EndCrystalRenderer and EndCrystalModel for the end crystals, and the
    /// beam to a crystal's target (the spikes being summoned back).
    fn end_crystal_geometry(&self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        let (Some(model), Some(r)) = (
            self.crystal_model.as_ref(),
            self.gui
                .atlas
                .sprites
                .get("entity/end_crystal/end_crystal")
                .copied(),
        ) else {
            return;
        };
        let uv = [r.u0, r.v0, r.u1, r.v1];
        let partial = self.clock.fraction() as f32;
        for crystal in &self.crystals {
            let feet = glam::DVec3::new(crystal.position.x, crystal.position.y, crystal.position.z);
            let rel = feet - camera;
            if rel.length_squared() > 256.0 * 256.0 {
                continue;
            }
            let age = crystal.time as f32 + partial;
            let light = self.world.light(
                feet.x.floor() as i32,
                feet.y.floor() as i32,
                feet.z.floor() as i32,
            );
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            let poses = horizon_client::end_crystal::poses(model, age, crystal.show_bottom);
            // poseStack.scale(2) then translate(0, -0.5, 0), not flipped.
            let base = glam::Mat4::from_translation(rel.as_vec3())
                * glam::Mat4::from_scale(glam::Vec3::splat(2.0))
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, -0.5, 0.0));
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
            if let Some(target) = crystal.beam {
                // The beam from the target's centre up to the bobbing crystal.
                let from = glam::DVec3::new(
                    f64::from(target[0]) + 0.5,
                    f64::from(target[1]) + 0.5,
                    f64::from(target[2]) + 0.5,
                );
                let delta = feet - from
                    + glam::DVec3::new(0.0, f64::from(horizon_client::end_crystal::y(age)), 0.0);
                self.crystal_beam(from, delta, age, camera, out);
            }
        }
    }

    /// EnderEyeItem.use: findNearestMapStructure over the stronghold rings,
    /// then an EyeOfEnder signalled towards it. Without a stronghold (other
    /// dimensions) nothing is thrown.
    fn throw_ender_eye(&mut self) {
        let rings = self
            .end
            .locator
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|t| t.ring_positions("minecraft:strongholds"));
        let (Some(rings), Some(p)) = (rings, self.player.as_ref()) else {
            return;
        };
        let block = [
            p.position.x.floor(),
            p.position.y.floor(),
            p.position.z.floor(),
        ];
        // ChunkGenerator.getNearestGeneratedStructure: the ring chunk closest to
        // its (8, 32, 8) block, located at the chunk's corner.
        let Some(nearest) = rings.iter().min_by(|a, b| {
            let d = |c: &horizon_core::position::ChunkPos| {
                (f64::from(c.x * 16 + 8) - block[0]).powi(2)
                    + (32.0 - block[1]).powi(2)
                    + (f64::from(c.z * 16 + 8) - block[2]).powi(2)
            };
            d(a).total_cmp(&d(b))
        }) else {
            return;
        };
        let target = glam::DVec3::new(f64::from(nearest.x * 16), 0.0, f64::from(nearest.z * 16));
        let start = glam::DVec3::new(
            p.position.x,
            p.position.y + f64::from(p.dimensions.height) * 0.5,
            p.position.z,
        );
        // EyeOfEnder.signalTo.
        let delta = target - start;
        let horizontal = delta.x.hypot(delta.z);
        let target = if horizontal > 12.0 {
            start
                + glam::DVec3::new(
                    delta.x / horizontal * 12.0,
                    8.0,
                    delta.z / horizontal * 12.0,
                )
        } else {
            target
        };
        let survive = self.world.random.next_int_bound(5).unwrap_or(0) > 0;
        self.end.eyes.push(EyeOfEnder {
            id: 0,
            position: start,
            previous: start,
            velocity: glam::DVec3::ZERO,
            target,
            life: 0,
            survive,
        });
        let r = self.world.random.next_float();
        let pitch = 0.33 + r * (0.5 - 0.33);
        let at = [p.position.x, p.position.y, p.position.z];
        self.audio
            .play("minecraft:entity.ender_eye.launch", Some(at), 1.0, pitch);
        self.consume_held();
        self.swing_arm();
    }

    /// EyeOfEnder.tick: fly, then drop (4 in 5) or shatter after 80 ticks.
    fn tick_eyes(&mut self) {
        let mut eyes = std::mem::take(&mut self.end.eyes);
        let mut shattered = Vec::new();
        eyes.retain_mut(|eye| {
            eye.previous = eye.position;
            let next = eye.position + eye.velocity;
            eye.velocity = EyeOfEnder::update_delta(eye.velocity, next, eye.target);
            eye.position = next;
            eye.life += 1;
            if eye.life <= 80 {
                return true;
            }
            let at = eye.position.to_array();
            self.audio
                .play("minecraft:entity.ender_eye.death", Some(at), 1.0, 1.0);
            if eye.survive {
                // new ItemEntity(level, x, y, z, item): random spread, 0.2 up.
                let vx = self.world.random.next_double() * 0.2 - 0.1;
                let vz = self.world.random.next_double() * 0.2 - 0.1;
                let stack = ItemStack {
                    item: "minecraft:ender_eye".into(),
                    count: 1,
                    damage: 0,
                    extra: None,
                };
                self.spawn_item(stack, at, [vx, 0.2, vz], 0);
            } else {
                shattered.push(at.map(|v| v.floor() as i32));
            }
            false
        });
        self.end.eyes = eyes;
        for pos in shattered {
            self.level_event_at(2003, pos, 0);
        }
    }

    /// `LevelEventHandler` 2003: a shattered eye's item crumbs and two rings
    /// of portal particles drawn in towards it.
    pub(super) fn eye_break_particles(&mut self, pos: [i32; 3]) {
        let (x, y, z) = (
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]),
            f64::from(pos[2]) + 0.5,
        );
        let crumbs = self.item_particle(&ItemStack::new("minecraft:ender_eye", 1));
        for _ in 0..8 {
            let r = self.particles.random();
            let v = [
                r.next_gaussian() * 0.15,
                r.next_double() * 0.2,
                r.next_gaussian() * 0.15,
            ];
            self.particles.add(&crumbs, [x, y, z], v);
        }
        let portal = ParticleOptions::simple(ParticleType::Portal);
        let mut angle = 0.0f64;
        while angle < std::f64::consts::PI * 2.0 {
            let (c, s) = (angle.cos(), angle.sin());
            let at = [x + c * 5.0, y - 0.4, z + s * 5.0];
            self.particles.add(&portal, at, [c * -5.0, 0.0, s * -5.0]);
            self.particles.add(&portal, at, [c * -7.0, 0.0, s * -7.0]);
            angle += std::f64::consts::PI / 20.0;
        }
    }

    /// `EyeOfEnder.tick` on a client: behind each eye (a quarter of its last
    /// move back), four bubbles in water, else a portal particle.
    pub(super) fn eye_particles(&mut self) {
        for i in 0..self.end.eyes.len() {
            let e = &self.end.eyes[i];
            let movement = (e.position - e.previous).to_array();
            let origin = (e.position - (e.position - e.previous) * 0.25).to_array();
            let block = origin.map(|v| v.floor() as i32);
            let water = self
                .res
                .facts()
                .get(self.world.block(block[0], block[1], block[2]))
                .fluid
                == horizon_client::blocks::Fluid::Water;
            if water {
                let bubble = ParticleOptions::simple(ParticleType::Bubble);
                for _ in 0..4 {
                    self.particles.add(&bubble, origin, movement);
                }
            } else {
                let r = self.particles.random();
                let x = origin[0] + r.next_double() * 0.6 - 0.3;
                let z = origin[2] + r.next_double() * 0.6 - 0.3;
                let portal = ParticleOptions::simple(ParticleType::Portal);
                self.particles
                    .add(&portal, [x, origin[1] - 0.5, z], movement);
            }
        }
    }
    /// ThrownItemRenderer: the ender eye item facing the camera.
    fn eye_geometry(&self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        let Some(r) = self.gui.atlas.sprites.get("item/ender_eye").copied() else {
            return;
        };
        let alpha = self.clock.fraction();
        let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
        let right = glam::Vec3::new(-yaw.cos(), 0.0, -yaw.sin()) * 0.25;
        let up = glam::Vec3::new(
            -yaw.sin() * pitch.sin(),
            pitch.cos(),
            yaw.cos() * pitch.sin(),
        ) * 0.25;
        let full = (15 * 16) | ((15 * 16) << 8);
        for eye in &self.end.eyes {
            let p = eye.previous + (eye.position - eye.previous) * alpha - camera;
            let c = p.as_vec3();
            let corners = [
                (c - right - up, [r.u0, r.v1]),
                (c + right - up, [r.u1, r.v1]),
                (c + right + up, [r.u1, r.v0]),
                (c - right + up, [r.u0, r.v0]),
            ];
            for (pos, uv) in corners {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: pos.to_array(),
                    uv,
                    layer: 0,
                    color: 0xFFFF_FFFF,
                    light: full,

                    glint_uv: [0.0, 0.0],
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::containers::EndGateway;

    #[test]
    fn a_saved_exit_moves_into_a_gateway_without_one() {
        let saved = ([8, 100, 0], true);
        let mut bare = EndGateway::default();
        assert!(adopt_saved_exit(&mut bare, saved));
        assert_eq!(bare.exit_portal, Some([8, 100, 0]));
        assert!(bare.exact_teleport);
        // The block entity's own exit wins over a stale entry.
        let mut own = EndGateway {
            exit_portal: Some([1, 2, 3]),
            ..Default::default()
        };
        assert!(!adopt_saved_exit(&mut own, saved));
        assert_eq!(own.exit_portal, Some([1, 2, 3]));
        assert!(!own.exact_teleport);
    }

    #[test]
    fn a_new_exit_keeps_the_gateways_exact_teleport() {
        let exact = BlockEntity::EndGateway(EndGateway {
            exact_teleport: true,
            ..Default::default()
        });
        let plain = BlockEntity::EndGateway(EndGateway::default());
        assert!(exact_teleport(Some(&exact)));
        assert!(!exact_teleport(Some(&plain)));
        assert!(!exact_teleport(None));
        assert!(!exact_teleport(Some(&BlockEntity::DaylightDetector)));
        let exit = [8, 100, 0];
        // Exact: the exit itself, nothing is searched for.
        assert_eq!(exit_target(exit, true, |_| panic!("searched")), exit);
        assert_eq!(
            exit_target(exit, false, |e| [e[0], e[1] + 11, e[2]]),
            [8, 111, 0]
        );
    }

    #[test]
    fn end_gateway_job_prefers_block_entity_then_legacy_then_heuristic() {
        let gateway = [702, 100, 0];
        let exit = [8, 100, 0];
        for exact in [true, false] {
            let entity = BlockEntity::EndGateway(EndGateway {
                exit_portal: Some(exit),
                exact_teleport: exact,
                ..Default::default()
            });
            assert_eq!(
                GatewayJob::for_gateway(gateway, Some(&entity), Some((END_SPAWN, !exact))),
                GatewayJob::Known { exit, exact },
            );
        }
        let empty = BlockEntity::EndGateway(EndGateway::default());
        for entity in [None, Some(&empty)] {
            assert_eq!(
                GatewayJob::for_gateway(gateway, entity, Some((exit, false))),
                GatewayJob::Known { exit, exact: false }
            );
            assert_eq!(
                GatewayJob::for_gateway(gateway, entity, None),
                GatewayJob::Known {
                    exit: END_SPAWN,
                    exact: true
                }
            );
            let nearby = [96, 75, 0];
            let exact = BlockEntity::EndGateway(EndGateway {
                exact_teleport: true,
                ..Default::default()
            });
            assert_eq!(
                GatewayJob::for_gateway(nearby, Some(&exact), None),
                GatewayJob::Find {
                    gateway: nearby,
                    exact: true
                }
            );
            assert_eq!(
                GatewayJob::for_gateway(nearby, entity, None),
                GatewayJob::Find {
                    gateway: nearby,
                    exact: false
                }
            );
        }
    }
}
