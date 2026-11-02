//! Primed TNT (`PrimedTnt`): 80-tick fuse, gravity and drag, explosion of
//! power 4, `TntRenderer` swell and white flash, and `TntBlock.useItemOn`
//! (flint and steel / fire charge).
use super::{Game, entity::Entity};
use horizon_client::entity_rules::EntityRules;
use horizon_client::{
    Result, collisions::SceneCollisions, items::ItemStack, mesher::Vertex, mobs::Who, text::Text,
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::level::flags as level_flags;

#[derive(Clone, Debug)]
pub(super) struct PrimedTnt {
    /// The entity id a server tracks it by (0 until it is tracked).
    pub id: u32,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub fuse: i32,
    pub on_ground: bool,
    pub owner: Option<Who>,
    pub guest: Option<u32>,
    /// `Entity.noGravity` (`NoGravity` in its save).
    pub no_gravity: bool,
    /// `PrimedTnt.explosionPower`: 4 by default, saved as `explosion_power`
    /// and clamped to 0..128 when read back.
    pub power: f32,
}
impl PrimedTnt {
    pub(super) const SIZE: f64 = 0.98;
    pub(super) fn bounds(&self) -> Aabb {
        let h = Self::SIZE / 2.0;
        Aabb::new(
            Vec3::new(self.position.x - h, self.position.y, self.position.z - h),
            Vec3::new(
                self.position.x + h,
                self.position.y + Self::SIZE,
                self.position.z + h,
            ),
        )
    }
    pub(super) fn tick(
        &mut self,
        world: &horizon_client::world::World,
        collision: &horizon_world::block_collision::BlockCollisionRegistry<'_>,
        assets: &horizon_client::assets::Assets,
        facts: &horizon_client::blocks::BlockFacts,
        fluids: &horizon_client::motion::FluidResources,
        border: Option<&horizon_core::geometry::border::BorderGeometry>,
    ) -> Result<Vec<BlockPos>> {
        self.previous = self.position;
        self.velocity.y -= self.get_gravity();
        let scene = SceneCollisions {
            world,
            binding: collision,
            subject: None,
        };
        let wanted = self.velocity;
        // Entity.collide: the world border's walls stop it too.
        let border = border.map(|b| (b, [self.position.x, self.position.z]));
        let moved = scene
            .resolve_in_border(self.bounds(), wanted, 0.0, self.on_ground, border)
            .unwrap_or(Vec3::ZERO);
        if horizon_client::entities::move_applies(wanted, moved) {
            self.position = Vec3::new(
                self.position.x + moved.x,
                self.position.y + moved.y,
                self.position.z + moved.z,
            );
        }
        self.on_ground = wanted.y < 0.0 && moved.y != wanted.y;
        if moved.x != wanted.x {
            self.velocity.x = 0.0;
        }
        if moved.z != wanted.z {
            self.velocity.z = 0.0;
        }
        if moved.y != wanted.y {
            // Entity.restituteMovementAfterCollisions: slime bounces it.
            let below = world.block(
                self.position.x.floor() as i32,
                (self.position.y - f64::from(1.0e-5_f32)).floor() as i32,
                self.position.z.floor() as i32,
            );
            self.velocity.y = horizon_client::entities::vertical_collision_velocity(
                wanted.y,
                moved.y,
                0.04,
                wanted.y < 0.0,
                &*facts.get(below).name == "minecraft:slime_block",
            );
        }
        // Entity.applyEffectsFromBlocks: a bubble column lifts or drags it, and the
        // columns it is at the top of send their surface particles.
        let mut tops = Vec::new();
        horizon_client::motion::bubble_column_push_over(
            world,
            assets.registry,
            facts,
            collision,
            self.bounds(),
            &mut self.velocity,
            |pos| tops.push(pos),
        );
        if self.on_ground {
            // Entity.applyEffectsFromBlocks -> SlimeBlock.stepOn.
            let under = world.block(
                self.position.x.floor() as i32,
                (self.position.y - f64::from(0.2_f32)).floor() as i32,
                self.position.z.floor() as i32,
            );
            if &*facts.get(under).name == "minecraft:slime_block"
                && let Some(scale) = horizon_client::entities::slime_step_on_scale(self.velocity.y)
            {
                self.velocity.x *= scale;
                self.velocity.z *= scale;
            }
        }
        self.velocity = Vec3::new(
            self.velocity.x * 0.98,
            self.velocity.y * 0.98,
            self.velocity.z * 0.98,
        );
        if self.on_ground {
            self.velocity = Vec3::new(
                self.velocity.x * 0.7,
                self.velocity.y * -0.5,
                self.velocity.z * 0.7,
            );
        }
        self.fuse -= 1;
        if self.fuse > 0 {
            // PrimedTnt updates immersion after moving and decrementing its fuse.
            self.push_by_fluid(world, fluids);
        }
        Ok(tops)
    }
}

impl Game<'_> {
    /// Asks the mods whether the TNT at `pos` may be primed
    /// (`native_tnt_primed`); the acting guest, if any, is its player.
    pub(super) fn tnt_prime_allowed(&mut self, pos: BlockPos) -> bool {
        let mut cancel = false;
        self.native_tnt_primed(self.lan_acting_uuid, [pos.x, pos.y, pos.z], &mut cancel);
        !cancel
    }
    /// Primes TNT a level event announced, or, when the mods refuse, puts
    /// back the block the fire or signal had already taken out.
    pub(super) fn prime_tnt_event(&mut self, pos: BlockPos, rot: f64) {
        if self.tnt_prime_allowed(pos) {
            self.add_primed_tnt(pos, rot, 80, None, None);
            return;
        }
        // Refused: the level queued the fuse sound and the game event right
        // after the prime; they are dropped as they come up.
        self.tnt_refused = Some(pos);
        let Some(tnt) = self.pack.assets.registry.default_state("minecraft:tnt") else {
            return;
        };
        if self.world.block(pos.x, pos.y, pos.z) == self.world.air {
            let flags = level_flags::UPDATE_CLIENTS | level_flags::UPDATE_SKIP_ON_PLACE;
            self.with_level(|b, level, ticks| b.set_block(level, ticks, pos, tnt, flags));
        }
    }
    /// `new PrimedTnt(level, x + 0.5, y, z + 0.5, owner)`; `short` applies
    /// `wasExploded`'s random short fuse. Plays `entity.tnt.primed` for primes.
    pub(super) fn spawn_primed_tnt(
        &mut self,
        pos: BlockPos,
        short: bool,
        owner: Option<Who>,
        guest: Option<u32>,
    ) {
        // PrimedTnt: the fuse is lit.
        let who = self.sculk_actor();
        self.post_block_game_event("minecraft:prime_fuse", [pos.x, pos.y, pos.z], who, None);
        let r = &mut self.world.random;
        let rot = r.next_double() * f64::from(std::f32::consts::TAU);
        let fuse = if short {
            // PrimedTnt.getRandomShortFuse
            r.next_int_bound(20).unwrap_or(0) + 10
        } else {
            self.audio.play(
                "minecraft:entity.tnt.primed",
                Some([
                    f64::from(pos.x) + 0.5,
                    f64::from(pos.y),
                    f64::from(pos.z) + 0.5,
                ]),
                1.0,
                1.0,
            );
            80
        };
        self.add_primed_tnt(pos, rot, fuse, owner, guest);
    }
    /// Adds primed TNT at the block's bottom centre, pushed along `rot`.
    pub(super) fn add_primed_tnt(
        &mut self,
        pos: BlockPos,
        rot: f64,
        fuse: i32,
        owner: Option<Who>,
        guest: Option<u32>,
    ) {
        let position = Vec3::new(
            f64::from(pos.x) + 0.5,
            f64::from(pos.y),
            f64::from(pos.z) + 0.5,
        );
        // An id from the entity counter at once, so a command can name it.
        self.next_entity += 1;
        self.tnts.push(PrimedTnt {
            no_gravity: false,
            id: self.next_entity,
            position,
            previous: position,
            velocity: Vec3::new(-rot.sin() * 0.02, f64::from(0.2f32), -rot.cos() * 0.02),
            fuse,
            on_ground: false,
            owner,
            guest,
            // PrimedTnt's constructor: 4 when nothing sets it.
            power: 4.0,
        });
    }
    /// `PrimedTnt.tick` for every primed TNT, in order; an expired fuse
    /// explodes immediately (later TNT in the list feel its knockback this
    /// tick). TNT primed by an explosion starts ticking next tick.
    pub(super) fn tick_tnt(&mut self) -> Result<()> {
        let mut count = self.tnts.len();
        let mut i = 0;
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        while i < count {
            let t = &mut self.tnts[i];
            if !ticking(t.position.x.floor() as i32, t.position.z.floor() as i32) {
                i += 1;
                continue;
            }
            let tops = t.tick(
                &self.world,
                self.collision,
                &self.pack.assets,
                self.res.facts(),
                &self.res.motion().fluids,
                Some(self.border.geometry()),
            )?;
            for pos in tops {
                self.bubble_column_particles(pos);
            }
            let t = &mut self.tnts[i];
            if t.fuse <= 0 {
                let center = [
                    t.position.x,
                    t.position.y + PrimedTnt::SIZE * 0.0625,
                    t.position.z,
                ];
                let bounds = t.bounds();
                let (owner, guest, power) = (t.owner, t.guest, t.power);
                let in_water = self.bounds_in_water(bounds);
                self.tnts.remove(i);
                count -= 1;
                // ExplosionInteraction.TNT: the tnt_explosion_drop_decay rule.
                let interaction = self.destroy_type("tnt_explosion_drop_decay");
                self.combat.explosion_in_water = in_water;
                // `Explosion.getIndirectSourceEntity`: a player-lit TNT names
                // its igniter, which `blast_attacker` and the damage type read.
                self.combat.blast_guest = guest;
                self.combat.blast_owner = tnt_blast_owner(
                    owner,
                    || {
                        self.lan
                            .as_ref()
                            .and_then(|host| self.player_attacker(host, guest))
                    },
                    |id| self.mob_display_name(id),
                );
                let result = self.with_hive_emptying_blast(|g| {
                    g.server_explode(center, power, interaction, false)
                });
                self.combat.blast_owner = None;
                self.combat.blast_guest = None;
                self.combat.explosion_in_water = false;
                result?;
            } else {
                i += 1;
            }
        }
        Ok(())
    }
    /// `TntBlock.useItemOn`: flint and steel or a fire charge primes TNT.
    pub(super) fn ignite_tnt(&mut self, stack: &ItemStack) -> Result<bool> {
        let item = &*stack.item;
        if item != "minecraft:flint_and_steel" && item != "minecraft:fire_charge" {
            return Ok(false);
        }
        let Some(hit) = self.pick() else {
            return Ok(false);
        };
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if &*self.res.facts().get(state).name != "minecraft:tnt" {
            return Ok(false);
        }
        let pos = BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
        if !self.tnt_prime_allowed(pos) {
            return Ok(true);
        }
        let guest = self.lan_acting;
        self.spawn_primed_tnt(pos, false, Some(Who::Player), guest);
        let air = self.world.air;
        self.with_level(|b, level, ticks| {
            b.set_block(level, ticks, pos, air, level_flags::UPDATE_ALL_IMMEDIATE)
        });
        self.process_level_events()?;
        self.swing_arm();
        if item == "minecraft:flint_and_steel" {
            self.damage_held(1);
        } else {
            self.consume_held();
        }
        Ok(true)
    }
    /// `TntRenderer`: the block centred on the entity, swelling over the last
    /// 10 ticks and flashing white every 5 ticks.
    pub(super) fn tnt_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        if self.tnts.is_empty() {
            return;
        }
        let Some(state) = self.pack.assets.registry.default_state("minecraft:tnt") else {
            return;
        };
        // Face sprites from the block model: up, down, north, south, west, east.
        let Ok(models) = self
            .pack
            .render
            .model_catalog
            .collect(state, BlockPos::new(0, 0, 0))
        else {
            return;
        };
        let mut sprites = [None; 6];
        for face in models.iter().flat_map(|(_, m)| m.faces.iter()) {
            use horizon_assets::model::Direction as D;
            let index = match face.quad.facing {
                D::Up => 0,
                D::Down => 1,
                D::North => 2,
                D::South => 3,
                D::West => 4,
                D::East => 5,
            };
            if sprites[index].is_none() {
                sprites[index] = self
                    .pack
                    .layer_names
                    .get(&face.layer)
                    .and_then(|name| self.gui.atlas.sprites.get(name).copied());
            }
        }
        let range = super::entity_culling::default_range(
            glam::DVec3::splat(PrimedTnt::SIZE),
            self.view_scale(),
        );
        for t in &self.tnts {
            // EntityRenderer.shouldRender.
            if !self.standing_entity_in_view(
                camera,
                super::entity_culling::at(t.previous),
                super::entity_culling::at(t.position),
                alpha,
                PrimedTnt::SIZE,
                PrimedTnt::SIZE,
                range,
                0.0,
            ) {
                continue;
            }
            let fuse = t.fuse as f32 - alpha as f32 + 1.0;
            let scale = if fuse < 10.0 {
                let g = (1.0 - fuse / 10.0).clamp(0.0, 1.0);
                1.0 + g * g * g * g * 0.3
            } else {
                1.0
            };
            let lit = fuse >= 0.0 && (fuse / 5.0) as i32 % 2 == 0;
            let base = glam::DVec3::new(
                t.previous.x + (t.position.x - t.previous.x) * alpha,
                t.previous.y + (t.position.y - t.previous.y) * alpha + 0.5,
                t.previous.z + (t.position.z - t.previous.z) * alpha,
            ) - camera;
            let o = base.as_vec3();
            let light = self.world.light(
                t.position.x.floor() as i32,
                (t.position.y + 0.5).floor() as i32,
                t.position.z.floor() as i32,
            );
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            let v = |x: f32, y: f32, z: f32| o + glam::Vec3::new(x - 0.5, y - 0.5, z - 0.5) * scale;
            let faces = [
                (
                    [v(0., 1., 1.), v(1., 1., 1.), v(1., 1., 0.), v(0., 1., 0.)],
                    1.0,
                ),
                (
                    [v(0., 0., 0.), v(1., 0., 0.), v(1., 0., 1.), v(0., 0., 1.)],
                    0.5,
                ),
                (
                    [v(1., 0., 0.), v(0., 0., 0.), v(0., 1., 0.), v(1., 1., 0.)],
                    0.8,
                ),
                (
                    [v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(0., 1., 1.)],
                    0.8,
                ),
                (
                    [v(0., 0., 0.), v(0., 0., 1.), v(0., 1., 1.), v(0., 1., 0.)],
                    0.6,
                ),
                (
                    [v(1., 0., 1.), v(1., 0., 0.), v(1., 1., 0.), v(1., 1., 1.)],
                    0.6,
                ),
            ];
            for ((corners, shade), sprite) in faces.into_iter().zip(sprites) {
                let Some(r) = sprite else { continue };
                let c = (255.0 * shade) as u32;
                let color = 0xFF00_0000 | c << 16 | c << 8 | c;
                for (p, uv) in
                    corners
                        .iter()
                        .zip([[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]])
                {
                    // Overlay 5: OverlayTexture white flash.
                    out.push(Vertex {
                        normal: [0; 4],
                        pos: p.to_array(),
                        uv,
                        layer: if lit { 5 } else { 0 },
                        color,
                        light: packed,

                        glint_uv: [0.0, 0.0],
                    });
                }
            }
        }
    }
}

impl Game<'_> {
    /// `positionRider` for the mobs riding primed TNT: the passenger attachment
    /// (the top of its 0.98 box), less the rider's own vehicle attachment.
    pub(super) fn seat_tnt_mobs(&mut self) {
        for i in 0..self.tnts.len() {
            let (id, pos, prev) = (
                self.tnts[i].id,
                self.tnts[i].position,
                self.tnts[i].previous,
            );
            if id == 0 {
                continue;
            }
            for m in self.mobs.iter_mut().filter(|m| m.extra.mount == Some(id)) {
                let offset = horizon_client::mobs::riders::vehicle_attachment(m);
                m.position = tnt_seat(pos, offset);
                m.previous = tnt_seat(prev, offset);
                m.velocity = Vec3::ZERO;
                m.fall_distance = 0.0;
                m.on_ground = false;
                m.extra.passenger = true;
                m.ai.nav.stop();
            }
        }
    }
}

/// `PrimedTnt.getOwner` as the blast's causing entity: the player, or a mob
/// that is still in the level (null once it is gone).
fn tnt_blast_owner(
    owner: Option<Who>,
    player: impl FnOnce() -> Option<(u32, Text)>,
    mob_name: impl Fn(u32) -> Option<Text>,
) -> Option<(u32, Text)> {
    match owner? {
        Who::Player => player(),
        Who::Mob(id) => mob_name(id).map(|name| (id, name)),
    }
}

fn tnt_seat(tnt: Vec3, rider_offset: f64) -> Vec3 {
    Vec3::new(tnt.x, tnt.y + PrimedTnt::SIZE - rider_offset, tnt.z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tnt_lit_by_a_skeletons_arrow_names_the_skeleton_as_its_cause() {
        let name = || Text::translate("entity.minecraft.skeleton", &[]);
        let by_mob = tnt_blast_owner(Some(Who::Mob(7)), || None, |id| (id == 7).then(name));
        assert_eq!(by_mob, Some((7, name())));
        // The skeleton died before the blast: `getOwner` is null.
        assert_eq!(tnt_blast_owner(Some(Who::Mob(7)), || None, |_| None), None);
        let player = Text::literal("Steve");
        let by_player = tnt_blast_owner(Some(Who::Player), || Some((1, player.clone())), |_| None);
        assert_eq!(by_player, Some((1, player)));
        assert_eq!(tnt_blast_owner(None, || None, |_| None), None);
    }

    #[test]
    fn a_rider_sits_on_top_of_the_tnt_less_its_own_attachment() {
        // 64 + 0.98 (the box's top) - 0.14 (a rider's vehicle attachment).
        let seat = tnt_seat(Vec3::new(1.5, 64.0, -2.5), 0.14);
        assert_eq!((seat.x, seat.z), (1.5, -2.5));
        assert!((seat.y - 64.84).abs() < 1e-9);
    }

    #[test]
    fn a_piston_slime_bounce_launches_primed_tnt() {
        use horizon_core::geometry::Direction;
        use horizon_world::piston_push::bounce_velocity;
        let mut tnt = PrimedTnt {
            no_gravity: false,
            id: 0,
            position: Vec3::new(0.5, 1.0, 0.5),
            previous: Vec3::new(0.5, 1.0, 0.5),
            velocity: Vec3::ZERO,
            fuse: 80,
            on_ground: true,
            owner: None,
            guest: None,
            power: 4.0,
        };
        tnt.velocity = bounce_velocity(Direction::Up, tnt.velocity);
        tnt.on_ground = false;
        assert_eq!(tnt.velocity.y, 1.0);
        assert!(!tnt.on_ground);
    }

    /// `Entity.collectColliders` (#7080): the world border's wall stops primed
    /// TNT. A border of size 10 around (20, 20) has its east wall at x = 25;
    /// the TNT starts at x = 24 on the test scene's flat ground, 1.5 east.
    #[test]
    fn the_world_border_stops_primed_tnt() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = horizon_client::assets::Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let facts = horizon_client::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = horizon_client::motion::FluidResources::new(&assets, &facts).unwrap();
        let collision = horizon_client::collisions::load(&root, assets.registry).unwrap();
        let world = horizon_client::world::World::test_scene(&assets, &render, 3).unwrap();
        let border =
            horizon_core::geometry::border::BorderGeometry::new([20.0, 20.0], 10.0, 29_999_984)
                .unwrap();
        for bordered in [false, true] {
            let at = Vec3::new(24.0, 4.0, 20.5);
            let mut tnt = PrimedTnt {
                no_gravity: false,
                id: 0,
                position: at,
                previous: at,
                velocity: Vec3::new(1.5, 0.0, 0.0),
                fuse: 80,
                on_ground: true,
                owner: None,
                guest: None,
                power: 4.0,
            };
            let border = bordered.then_some(&border);
            tnt.tick(&world, &collision, &assets, &facts, &fluids, border)
                .unwrap();
            let east = tnt.bounds().max_position().x;
            if bordered {
                assert!(
                    (east - 25.0).abs() < 1.0e-7,
                    "stops on the wall, got {east}"
                );
            } else {
                assert!(east > 25.0, "no border: goes on, got {east}");
            }
        }
    }

    #[test]
    fn primed_tnt_follows_water_currents_after_moving() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = horizon_client::assets::Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let facts = horizon_client::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = horizon_client::motion::FluidResources::new(&assets, &facts).unwrap();
        let collision = horizon_client::collisions::load(&root, assets.registry).unwrap();
        let mut world = horizon_client::world::World::test_scene(&assets, &render, 3).unwrap();
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let mut state = assets.registry.state(water).unwrap().clone();
        state.properties.insert("level".into(), "1".into());
        let flowing = assets.registry.resolve(&state).unwrap();
        for x in 19..=22 {
            for z in 19..=21 {
                world.set(x, 3, z, stone).unwrap();
            }
        }
        world.set(20, 4, 20, water).unwrap();
        world.set(21, 4, 20, flowing).unwrap();
        let at = Vec3::new(20.5, 4.0, 20.5);
        let mut tnt = PrimedTnt {
            no_gravity: false,
            id: 0,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            fuse: 80,
            on_ground: false,
            owner: None,
            guest: None,
            power: 4.0,
        };
        tnt.tick(&world, &collision, &assets, &facts, &fluids, None)
            .unwrap();
        assert_eq!(
            tnt.position.x, at.x,
            "current is applied after motion, not before"
        );
        assert!(
            (tnt.velocity.x - 0.014).abs() < 1e-12,
            "water must push primed TNT east: {:?}",
            tnt.velocity
        );
        assert_eq!(tnt.fuse, 79);
        tnt.tick(&world, &collision, &assets, &facts, &fluids, None)
            .unwrap();
        assert!((tnt.position.x - at.x - 0.014).abs() < 1e-12);
        assert!(tnt.on_ground);
        tnt.position = at;
        tnt.velocity = Vec3::ZERO;
        tnt.fuse = 1;
        tnt.tick(&world, &collision, &assets, &facts, &fluids, None)
            .unwrap();
        assert_eq!(tnt.fuse, 0);
        assert_eq!(
            tnt.velocity.x, 0.0,
            "expired TNT must not receive a current"
        );
        world.set(21, 4, 20, water).unwrap();
        tnt.position = at;
        tnt.velocity = Vec3::ZERO;
        tnt.fuse = 80;
        tnt.tick(&world, &collision, &assets, &facts, &fluids, None)
            .unwrap();
        assert_eq!(
            tnt.velocity.x, 0.0,
            "still water must not add horizontal motion"
        );
        world
            .set(
                20,
                3,
                20,
                assets.registry.default_state("minecraft:air").unwrap(),
            )
            .unwrap();
        tnt.on_ground = false;
        tnt.velocity = Vec3::ZERO;
        tnt.tick(&world, &collision, &assets, &facts, &fluids, None)
            .unwrap();
        assert!(
            tnt.position.y < at.y && tnt.velocity.y < 0.0,
            "TNT sinks; water does not add buoyancy"
        );
    }

    #[test]
    fn a_soul_sand_bubble_column_lifts_primed_tnt() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = horizon_client::assets::Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let facts = horizon_client::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = horizon_client::motion::FluidResources::new(&assets, &facts).unwrap();
        let collision = horizon_client::collisions::load(&root, assets.registry).unwrap();
        let mut world = horizon_client::world::World::test_scene(&assets, &render, 3).unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let soul_sand = assets
            .registry
            .default_state("minecraft:soul_sand")
            .unwrap();
        // Soul sand makes an up-pushing column (`drag=false`); the block's
        // default state is the down-dragging one.
        let drag_down = assets
            .registry
            .default_state("minecraft:bubble_column")
            .unwrap();
        let mut up = assets.registry.state(drag_down).unwrap().clone();
        up.properties.insert("drag".into(), "false".into());
        let column = assets.registry.resolve(&up).unwrap();
        for y in 3..=8 {
            world.set(20, y, 20, column).unwrap();
        }
        world.set(20, 2, 20, soul_sand).unwrap();
        world.set(20, 9, 20, water).unwrap();
        // BubbleColumnBlock.entityInside: the columns its box touches lift it
        // (Entity.handleOnInsideBubbleColumn / handleOnAboveBubbleColumn).
        let at = Vec3::new(20.5, 6.0, 20.5);
        let mut tnt = PrimedTnt {
            no_gravity: false,
            id: 0,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            fuse: 80,
            on_ground: false,
            owner: None,
            guest: None,
            power: 4.0,
        };
        for _ in 0..10 {
            tnt.tick(&world, &collision, &assets, &facts, &fluids, None)
                .unwrap();
        }
        assert!(
            tnt.velocity.y > 0.0,
            "soul sand lifts primed TNT, got {:?}",
            tnt.velocity
        );
        assert!(
            tnt.position.y > at.y,
            "the column carries it up, got {}",
            tnt.position.y
        );
        // Without the column the same TNT only falls.
        for y in 3..=8 {
            world.set(20, y, 20, water).unwrap();
        }
        tnt.position = at;
        tnt.velocity = Vec3::ZERO;
        tnt.on_ground = false;
        for _ in 0..4 {
            tnt.tick(&world, &collision, &assets, &facts, &fluids, None)
                .unwrap();
        }
        assert!(
            tnt.position.y < at.y,
            "water alone does not lift it, got {}",
            tnt.position.y
        );
    }

    /// `Entity.onAboveBubbleColumn` sends the surface particles only from the top of
    /// a column: `tick` names the column block it is at the top of, and none while
    /// the column has water above it.
    #[test]
    fn primed_tnt_reports_the_surface_of_a_bubble_column() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = horizon_client::assets::Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let facts = horizon_client::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = horizon_client::motion::FluidResources::new(&assets, &facts).unwrap();
        let collision = horizon_client::collisions::load(&root, assets.registry).unwrap();
        let mut world = horizon_client::world::World::test_scene(&assets, &render, 3).unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let soul_sand = assets
            .registry
            .default_state("minecraft:soul_sand")
            .unwrap();
        let default = assets
            .registry
            .default_state("minecraft:bubble_column")
            .unwrap();
        let mut up = assets.registry.state(default).unwrap().clone();
        up.properties.insert("drag".into(), "false".into());
        let column = assets.registry.resolve(&up).unwrap();
        for y in 3..=8 {
            world.set(20, y, 20, column).unwrap();
        }
        world.set(20, 2, 20, soul_sand).unwrap();
        let at = Vec3::new(20.5, 6.0, 20.5);
        let primed = || PrimedTnt {
            no_gravity: false,
            id: 0,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            fuse: 80,
            on_ground: false,
            owner: None,
            guest: None,
            power: 4.0,
        };
        let mut tops = Vec::new();
        let mut tnt = primed();
        for _ in 0..30 {
            tops.extend(
                tnt.tick(&world, &collision, &assets, &facts, &fluids, None)
                    .unwrap(),
            );
        }
        assert!(!tops.is_empty(), "it reaches the top of the column");
        assert!(
            tops.iter().all(|p| *p == BlockPos::new(20, 8, 20)),
            "only the surface block, got {tops:?}"
        );
        // Water above the top block: it is no longer the surface.
        world.set(20, 9, 20, water).unwrap();
        let mut tnt = primed();
        for _ in 0..10 {
            assert!(
                tnt.tick(&world, &collision, &assets, &facts, &fluids, None)
                    .unwrap()
                    .is_empty()
            );
        }
    }
}
