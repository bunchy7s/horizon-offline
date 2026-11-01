//! Cushions: `CushionItem.useOn` placement on the top face of a block,
//! `Cushion.canBePlacedAt`/`survives`, `Cushion.interact` (sit),
//! `BlockAttachedEntity.hurtServer` (break, dropping the matching colour),
//! `BlockAttachedEntity.tick` and `CushionRenderer`.
//!
//! A cushion is a `BlockAttachedEntity`, not a block: it has no block state,
//! only an entity whose `Pos` is the click point's height. Placement uses
//! outline bounds, collision shapes and suffocation predicates.
use super::{Game, Hit, InteractTarget};
use horizon_client::entities::{Cushion, dye_index};
use horizon_client::items::ItemStack;
use horizon_client::mobs::anim::Poses;
use horizon_core::geometry::{Aabb, Vec3};
use winit::keyboard::KeyCode;

/// `Direction.fromYRot(yaw).toYRot()`: the nearest 90 degree step.
pub(crate) fn snap_yaw(yaw: f32) -> f32 {
    (((f64::from(yaw) / 90.0 + 0.5).floor() as i64) & 3) as f32 * 90.0
}

fn aabb(bounds: ([f64; 3], [f64; 3])) -> Aabb {
    Aabb::new(
        Vec3::new(bounds.0[0], bounds.0[1], bounds.0[2]),
        Vec3::new(bounds.1[0], bounds.1[1], bounds.1[2]),
    )
}

fn overlaps(a: Aabb, b: Aabb) -> bool {
    a.intersects(b)
}
use horizon_client::cushion::blocks_in;

struct PlacementWorld<'a, 'r>(&'a Game<'r>);
impl horizon_client::cushion::World for PlacementWorld<'_, '_> {
    fn outline_bounds(&self, p: [i32; 3]) -> Option<Aabb> {
        let boxes = self.0.outline_boxes(p);
        let mut shape = *boxes.first()?;
        for b in &boxes[1..] {
            for k in 0..3 {
                shape[k] = shape[k].min(b[k]);
                shape[k + 3] = shape[k + 3].max(b[k + 3]);
            }
        }
        Some(Aabb::new(
            Vec3::new(shape[0], shape[1], shape[2]),
            Vec3::new(shape[3], shape[4], shape[5]),
        ))
    }

    fn collision_shapes(&self, bounds: Aabb) -> Vec<horizon_core::geometry::VoxelShape> {
        let binding = self.0.res.collision();
        let mut world = horizon_client::collisions::SceneCollisions {
            world: &self.0.world,
            binding,
            subject: None,
        };
        // Include neighbour blocks whose collision shape can reach outside their cell.
        blocks_in(bounds.inflate(Vec3::new(1.0, 1.0, 1.0)))
            .filter_map(|p| {
                let state = self.0.world.block(p[0], p[1], p[2]);
                binding
                    .collision_shape(&mut world, state, p)
                    .ok()
                    .map(|s| s.moved(Vec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]))))
            })
            .collect()
    }
    fn suffocating(&self, p: [i32; 3]) -> bool {
        self.0
            .state_suffocating(self.0.world.block(p[0], p[1], p[2]))
    }
}

impl Game<'_> {
    /// `BlockBehaviour.Properties.isSuffocating` of a state: the block's
    /// explicit predicate, else `causes_suffocation` with a full collision
    /// shape.
    pub(super) fn state_suffocating(&self, state: horizon_world::registry::BlockStateId) -> bool {
        let facts = self.res.facts().get(state);
        let name = facts.name.as_ref();
        // Blocks' explicit predicates, then the default causes_suffocation/full-shape rule.
        match name {
            "minecraft:dirt_path"
            | "minecraft:farmland"
            | "minecraft:mud"
            | "minecraft:soul_sand" => true,
            "minecraft:piston" | "minecraft:sticky_piston" => self
                .pack
                .assets
                .registry
                .state(state)
                .is_some_and(|s| s.properties.get("extended").is_some_and(|p| p == "false")),
            "minecraft:mangrove_roots" | "minecraft:moving_piston" => false,
            _ if name.ends_with("_leaves")
                || name.ends_with("glass")
                || name.ends_with("copper_grate") =>
            {
                false
            }
            // ShulkerBoxBlock: suffocating while closed; a box seen open
            // here is rare enough to count as closed.
            _ if name.ends_with("shulker_box") => true,
            _ => {
                facts.full_collision
                    && self
                        .res
                        .items()
                        .block_in_tag("minecraft:causes_suffocation", name)
            }
        }
    }

    /// `BlockState.isViewBlocking(level, pos, nearPlaneBox)`: stairs and
    /// slabs (`NEAR_PLANE_INTERSECTS_OUTLINE`) block when the near plane
    /// touches a box of their shape, snow only at eight layers, anything else
    /// when it is suffocating.
    pub(super) fn state_view_blocking(
        &self,
        state: horizon_world::registry::BlockStateId,
        pos: [i32; 3],
        near_plane: Aabb,
    ) -> bool {
        let name = self.res.facts().get(state).name.as_ref();
        if name == "minecraft:snow" {
            return self
                .pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("layers"))
                .is_some_and(|l| l.parse::<i32>().is_ok_and(|l| l >= 8));
        }
        if (name.ends_with("_stairs") || name.ends_with("_slab"))
            && name != "minecraft:petrified_oak_slab"
        {
            return self
                .outline_boxes(pos)
                .iter()
                .any(|b| aabb(([b[0], b[1], b[2]], [b[3], b[4], b[5]])).intersects(near_plane));
        }
        self.state_suffocating(state)
    }
}

impl Game<'_> {
    /// The cushion under the crosshair, its distance and the hit point.
    pub(super) fn cushion_hit(&self) -> Option<(usize, f64, [f64; 3])> {
        let (eye, dir) = self.aim()?;
        let reach: f64 = self.entity_reach;
        let mut best: Option<(usize, f64, [f64; 3])> = None;
        for (i, c) in self.cushions.iter().enumerate() {
            let Some(d) = super::ray_box(eye, dir, reach, aabb(c.bounds())) else {
                continue;
            };
            if best.is_none_or(|b| d < b.1) {
                best = Some((i, d, std::array::from_fn(|k| eye[k] + dir[k] * d)));
            }
        }
        best
    }

    pub(super) fn cushion_survives(&self, c: &Cushion) -> bool {
        horizon_client::cushion::survives(&PlacementWorld(self), aabb(c.bounds()))
    }

    /// `Cushion.destroyIfInFire`: a fire-tagged block inside the deflated box.
    fn cushion_in_fire(&self, c: &Cushion) -> bool {
        let bounds = aabb(c.bounds()).next_deflated();
        blocks_in(bounds).any(|p| {
            let facts = self.res.facts().get(self.world.block(p[0], p[1], p[2]));
            self.items().block_in_tag("minecraft:fire", &facts.name)
        })
    }

    /// `CushionItem.useOn` up to `Cushion.canBePlacedAt`, which the client
    /// checks too: on the top face only, at the click's height above the
    /// clicked block (or the replaceable block itself). The position and
    /// spawn box of a cushion that may go there.
    pub(super) fn cushion_spawn(&self, hit: &Hit) -> Option<(Vec3, Aabb)> {
        // CushionItem.recalculateContextForSpecialCollisionShapes: open containers
        // (cauldrons, hoppers and composters) use the collision ray's actual height.
        let mut adjusted = Hit {
            pos: hit.pos,
            face: hit.face,
            point: hit.point,
        };
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if self.items().block_in_tag(
            "minecraft:cushion_uses_collision_shape",
            &self.res.facts().get(state).name,
        ) && let Some(eye) = self.eye_position()
        {
            let start = Vec3::new(eye[0], eye[1], eye[2]);
            let point = Vec3::new(hit.point[0], hit.point[1], hit.point[2]);
            let end = point.add_vector(point.subtract(start).normalize().scale(0.001));
            let binding = self.res.collision();
            let mut world = horizon_client::collisions::SceneCollisions {
                world: &self.world,
                binding,
                subject: None,
            };
            if let Ok(shape) = binding.collision_shape(&mut world, state, hit.pos)
                && let Some(result) = shape.clip(
                    start,
                    end,
                    horizon_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
                )
            {
                adjusted.face = result.direction.normal();
                adjusted.point = [result.location.x, result.location.y, result.location.z];
            }
        }
        let hit = &adjusted;
        if hit.face != [0, 1, 0] {
            return None;
        }
        // BlockPlaceContext: a replaceable clicked block is replaced.
        let clicked = self
            .res
            .facts()
            .get(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]));
        let pos = if clicked.replaceable {
            hit.pos
        } else {
            std::array::from_fn(|i| hit.pos[i] + hit.face[i])
        };
        // `Vec3.atCenterOfWithY(clickedPos, clickLocation.y)`.
        let y = hit.point[1];
        let position = Vec3::new(f64::from(pos[0]) + 0.5, y, f64::from(pos[2]) + 0.5);
        let bounds = aabb((
            [position.x - 0.5, position.y, position.z - 0.5],
            [position.x + 0.5, position.y + 0.25, position.z + 0.5],
        ));
        horizon_client::cushion::can_place(&PlacementWorld(self), bounds)
            .then_some((position, bounds))
    }

    /// `CushionItem.useOn`: spawn the cushion where `cushion_spawn` allows,
    /// unless another cushion is in the way.
    pub(super) fn place_cushion(&mut self, hit: &Hit, stack: &ItemStack) -> bool {
        let Some(info) = self.items().get(&stack.item).cloned() else {
            return false;
        };
        let Some(default_color) = info.cushion_color.as_deref() else {
            return false;
        };
        let color_override = stack
            .data()
            .and_then(|d| d.other("minecraft:cushion/color"))
            .and_then(|tag| match tag {
                horizon_world::nbt::Tag::String(s) => Some(s.to_string_lossy()),
                _ => None,
            });
        let color = dye_index(color_override.as_deref().unwrap_or(default_color));
        let Some((position, bounds)) = self.cushion_spawn(hit) else {
            return false;
        };
        // `getEntitiesOfClass(Cushion.class, spawnAABB).isEmpty()`.
        if self
            .cushions
            .iter()
            .any(|c| overlaps(aabb(c.bounds()), bounds))
        {
            return false;
        }
        self.next_entity += 1;
        let id = self.next_entity;
        // `BlockAttachedEntity.setPos`: `BlockPos.containing(position)`.
        let attached = [
            position.x.floor() as i32,
            position.y.floor() as i32,
            position.z.floor() as i32,
        ];
        self.cushions.push(Cushion {
            id,
            position,
            pos: attached,
            yaw: snap_yaw(self.yaw),
            color,
            check_interval: 0,
        });
        self.world
            .modified
            .insert((attached[0] >> 4, attached[2] >> 4));
        // `destroyIfInFire`: placed into fire, it breaks at once.
        let placed = self.cushions.len() - 1;
        if self.cushion_in_fire(&self.cushions[placed]) {
            self.break_cushion(placed, false);
        }
        self.audio.play(
            "minecraft:entity.cushion.place",
            Some([position.x, position.y, position.z]),
            0.75,
            0.8,
        );
        let ctx = self.who_context(Some(horizon_client::mobs::Who::Player), None);
        self.post_game_event(
            "minecraft:entity_place",
            [position.x, position.y, position.z],
            ctx,
        );
        if !self.creative {
            self.consume_held();
        }
        // `CushionItem.useOn` returns SUCCESS: the hand swings.
        self.swing_arm();
        true
    }

    /// `Cushion.interact`: sit down unless sneaking or already occupied. The
    /// cushion under the crosshair, or the one a guest's interact packet named.
    pub(super) fn interact_cushion(&mut self, target: Option<InteractTarget>) -> bool {
        let Some((i, d, _)) = (match target {
            Some(t) => {
                Self::named_entity(&self.cushions, t.id, |c| c.id).map(|i| (i, 0.0, [0.0; 3]))
            }
            None => self.cushion_hit(),
        }) else {
            return false;
        };
        // Only a client's own pick is stopped by a block: the range check passed.
        if target.is_none() && !self.before_block(d) {
            return false;
        }
        // isSecondaryUseActive.
        if self.keys.contains(&KeyCode::ShiftLeft) {
            return false;
        }
        let id = self.cushions[i].id;
        let occupied = self.riding == Some(id)
            || self
                .lan
                .as_ref()
                .is_some_and(|h| h.hub.guests.iter().any(|g| g.vehicle == Some(id)));
        // `Cushion.interact`: an occupied cushion (`isVehicle`) passes.
        if occupied {
            return false;
        }
        // `Entity.startRiding`: a passenger first gets off what it rides
        // (`stopRiding`), so a seated player moves to this cushion.
        self.stop_riding();
        self.riding = Some(id);
        let p = self.cushions[i].position;
        self.audio.play(
            "minecraft:entity.cushion.sit",
            Some([p.x, p.y, p.z]),
            1.0,
            1.0,
        );
        self.swing_arm();
        true
    }

    /// `Cushion.hurtServer` from a player: killed, then the item dropped.
    pub(super) fn attack_cushion(&mut self) -> bool {
        let Some((i, d, _)) = self.cushion_hit() else {
            return false;
        };
        if !self.before_block(d) || self.mob_hit().is_some_and(|m| m.1 < d) {
            return false;
        }
        // `skipAttackInteraction` when `mayInteract` is false.
        if !self.may_build() {
            return true;
        }
        self.break_cushion(i, true);
        true
    }

    /// `Cushion.dropItem`: the break sound and particles, then the coloured
    /// item unless drops are off or the breaker has infinite materials.
    pub(super) fn break_cushion(&mut self, index: usize, by_player: bool) {
        let c = self.cushions.remove(index);
        let p = c.position;
        self.world.modified.insert((c.pos[0] >> 4, c.pos[2] >> 4));
        if self.riding == Some(c.id) {
            self.dismount_removed_vehicle(p.y);
        }
        self.audio.play(
            "minecraft:entity.cushion.break",
            Some([p.x, p.y, p.z]),
            1.0,
            1.0,
        );
        self.cushion_break_particles(&c);
        if !self.ui.rules.bool("entity_drops") || (by_player && self.creative) {
            return;
        }
        let stack = ItemStack {
            item: std::sync::Arc::from(c.item()),
            count: 1,
            damage: 0,
            extra: None,
        };
        self.spawn_item(stack, [p.x, p.y, p.z], [0.0; 3], 10);
    }

    /// `Cushion.showBreakingParticles`: ten of the matching wool block.
    fn cushion_break_particles(&mut self, c: &Cushion) {
        let name = format!("minecraft:{}_wool", c.color_name());
        let Some(state) = self.pack.assets.registry.default_state(&name) else {
            return;
        };
        let p = c.position;
        // Entity.getY(2/3) is relative to the cushion's quarter-block height.
        let particle_at = [p.x, p.y + 0.25 * (2.0 / 3.0), p.z];
        // Guest actions temporarily take self.lan; flush once the hub is back.
        // Queue before resolving a sprite: a dedicated server has no renderer.
        if self.audio.capture.is_some() {
            self.lan_cushion_breaks.push((state, particle_at));
        }
        let at = [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32];
        let Some((sprite, tint)) = self.particle_sprite(state, at) else {
            return;
        };
        let options = horizon_client::particles::ParticleOptions::block(
            horizon_client::particles::ParticleType::Block,
            sprite,
            tint,
        );
        self.particles
            .send_particles(&options, particle_at, [0.25, 0.0625, 0.25], 0.05, 10);
    }

    /// `BlockAttachedEntity.tick`: survive and burn checks after the post-increment reaches 100.
    pub(super) fn tick_cushions(&mut self) {
        let ticking = self.entity_ticking_area();
        let mut i = 0;
        while i < self.cushions.len() {
            let p = self.cushions[i].position;
            if !ticking(p.x.floor() as i32, p.z.floor() as i32) {
                i += 1;
                continue;
            }
            let previous = self.cushions[i].check_interval;
            self.cushions[i].check_interval += 1;
            if previous < 100 {
                i += 1;
                continue;
            }
            self.cushions[i].check_interval = 0;
            let c = self.cushions[i].clone();
            if self.cushion_in_fire(&c) || !self.cushion_survives(&c) {
                self.break_cushion(i, false);
                continue;
            }
            i += 1;
        }
    }

    /// `LivingEntity.stopRiding` off a cushion: `dismountVehicle` beside it,
    /// and `Cushion.removePassenger`'s get-up sound for a rider that left
    /// rather than one whose cushion was killed.
    pub(super) fn get_up_from_cushion(&mut self, c: &Cushion) {
        let at = c.dismount();
        self.riding = None;
        if let Some(p) = self.player.as_mut() {
            self.previous_position = p.position;
            p.position = at;
            p.velocity = Vec3::ZERO;
            p.fall_distance = 0.0;
        }
        self.audio.play(
            "minecraft:entity.cushion.get_up",
            Some([c.position.x, c.position.y, c.position.z]),
            1.0,
            1.0,
        );
    }

    /// `LivingEntity.dismountVehicle` from a removed vehicle (a broken
    /// cushion) whose position was at `vehicle_y`: up to the higher of the
    /// rider's and the vehicle's heights, then the free spot nearest there
    /// (`findFreePosition` within the rider's own box). No get-up sound:
    /// `Cushion.removePassenger` plays none once the cushion is removed.
    pub(super) fn dismount_removed_vehicle(&mut self, vehicle_y: f64) {
        self.riding = None;
        let border = self.border_bounds();
        let b = self.res.behaviours();
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let (w, h) = (
            f64::from(p.dimensions.width),
            f64::from(p.dimensions.height),
        );
        let mut at = Vec3::new(p.position.x, p.position.y.max(vehicle_y), p.position.z);
        // `getBbWidth() <= 4 && getBbHeight() <= 4`.
        if w <= 4.0 && h <= 4.0 {
            let center = at.add_vector(Vec3::new(0.0, h / 2.0, 0.0));
            let size = Vec3::new(w, h, w);
            let free = horizon_world::level::portal_travel::find_free_position(
                Aabb::of_size(center, size),
                center,
                size,
                &border,
                |q| {
                    b.collision_shape(self.world.block(q.x, q.y, q.z))
                        .map(|s| s.moved(Vec3::new(f64::from(q.x), f64::from(q.y), f64::from(q.z))))
                },
            );
            if let Some(free) = free {
                at = free.add_vector(Vec3::new(0.0, -h / 2.0, 0.0));
            }
        }
        if let Some(p) = self.player.as_mut() {
            self.previous_position = p.position;
            p.position = at;
            p.velocity = Vec3::ZERO;
            p.fall_distance = 0.0;
        }
    }

    /// `Player.rideTick` on a cushion: hold the seat, and get up on sneak.
    pub(super) fn tick_cushion_riding(&mut self, keys: super::Keys) -> bool {
        let Some(c) = self
            .cushions
            .iter()
            .find(|c| Some(c.id) == self.riding)
            .cloned()
        else {
            return false;
        };
        if keys.shift {
            self.get_up_from_cushion(&c);
            return true;
        }
        let seat = c.seat();
        if let Some(p) = self.player.as_mut() {
            self.previous_position = p.position;
            p.position = seat;
            p.velocity = Vec3::ZERO;
            p.fall_distance = 0.0;
        }
        true
    }

    /// `CushionRenderer`: the coloured model, rotated to the nearest
    /// direction, with its bottom on the cushion's position.
    pub(super) fn cushion_geometry(
        &self,
        camera: glam::DVec3,
        out: &mut Vec<horizon_client::mesher::Vertex>,
    ) {
        let Some(model) = self.cushion_model.as_ref() else {
            return;
        };
        let sprites = &self.gui.atlas.sprites;
        for c in &self.cushions {
            let entity_start = out.len();
            let a = aabb(c.bounds());
            let (lo, hi) = (a.min_position(), a.max_position());
            let (lo, hi) = (
                glam::DVec3::new(lo.x, lo.y, lo.z),
                glam::DVec3::new(hi.x, hi.y, hi.z),
            );
            let range = super::entity_culling::default_range(hi - lo, self.view_scale());
            if !self.entity_in_view(camera, (lo + hi) / 2.0, range, lo, hi) {
                continue;
            }
            let Some(uv) = sprites
                .get(&format!("entity/cushion/{}_cushion", c.color_name()))
                .map(|r| [r.u0, r.v0, r.u1, r.v1])
            else {
                continue;
            };
            let p = glam::DVec3::new(c.position.x, c.position.y, c.position.z) - camera;
            let base = glam::Mat4::from_translation(p.as_vec3())
                * glam::Mat4::from_rotation_y((180.0 - snap_yaw(c.yaw)).to_radians())
                * glam::Mat4::from_rotation_x(std::f32::consts::PI)
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, -0.25, 0.0));
            // `EntityRenderer.getPackedLightCoords` probes at the eye height.
            let l = self.world.light(
                c.position.x.floor() as i32,
                (c.position.y + 0.2125).floor() as i32,
                c.position.z.floor() as i32,
            );
            let packed = (u32::from(l >> 4) * 16) | ((u32::from(l & 15) * 16) << 8);
            horizon_client::mobs::render::draw(
                model,
                base,
                &Poses::new(model),
                uv,
                0xFFFF_FFFF,
                packed,
                0,
                out,
            );
            self.cull_entity_vertices(out, entity_start);
        }
    }
}
