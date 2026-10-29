//! Primed TNT (`PrimedTnt`): 80-tick fuse, gravity and drag, explosion of
//! power 4, `TntRenderer` swell and white flash, and `TntBlock.useItemOn`
//! (flint and steel / fire charge).
use super::Game;
use horizon_client::{Result, collisions::SceneCollisions, items::ItemStack, mesher::Vertex};
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
}
impl PrimedTnt {
    const SIZE: f64 = 0.98;
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
    ) -> Result<()> {
        self.previous = self.position;
        self.velocity.y -= 0.04;
        let mut scene = SceneCollisions {
            world,
            binding: collision,
            subject: None,
        };
        let wanted = self.velocity;
        let moved = scene
            .resolve(self.bounds(), wanted, 0.0, self.on_ground)
            .unwrap_or(Vec3::ZERO);
        self.position = Vec3::new(
            self.position.x + moved.x,
            self.position.y + moved.y,
            self.position.z + moved.z,
        );
        self.on_ground = wanted.y < 0.0 && moved.y != wanted.y;
        if moved.x != wanted.x {
            self.velocity.x = 0.0;
        }
        if moved.z != wanted.z {
            self.velocity.z = 0.0;
        }
        if moved.y != wanted.y {
            self.velocity.y = 0.0;
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
            let mut fluids =
                horizon_world::entity_fluid::EntityFluidInteraction::new([WATER, LAVA]);
            let mut fluid_world = TntFluidWorld {
                tnt: self,
                world,
                assets,
                facts,
            };
            fluids
                .update(&mut fluid_world, false, 4096)
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            fluids.apply_current(&mut fluid_world, WATER, 0.014)?;
            fluids.apply_current(
                &mut fluid_world,
                LAVA,
                if world.fast_lava {
                    0.007
                } else {
                    0.0023333333333333335
                },
            )?;
        }
        Ok(())
    }
}

impl Game<'_> {
    /// `new PrimedTnt(level, x + 0.5, y, z + 0.5, owner)`; `short` applies
    /// `wasExploded`'s random short fuse. Plays `entity.tnt.primed` for primes.
    pub(super) fn spawn_primed_tnt(&mut self, pos: BlockPos, short: bool) {
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
        self.add_primed_tnt(pos, rot, fuse);
    }
    /// Adds primed TNT at the block's bottom centre, pushed along `rot`.
    pub(super) fn add_primed_tnt(&mut self, pos: BlockPos, rot: f64, fuse: i32) {
        let position = Vec3::new(
            f64::from(pos.x) + 0.5,
            f64::from(pos.y),
            f64::from(pos.z) + 0.5,
        );
        self.tnts.push(PrimedTnt {
            id: 0,
            position,
            previous: position,
            velocity: Vec3::new(-rot.sin() * 0.02, f64::from(0.2f32), -rot.cos() * 0.02),
            fuse,
            on_ground: false,
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
            t.tick(
                &self.world,
                self.collision,
                &self.pack.assets,
                self.res.facts(),
            )?;
            if t.fuse <= 0 {
                let center = [
                    t.position.x,
                    t.position.y + PrimedTnt::SIZE * 0.0625,
                    t.position.z,
                ];
                self.tnts.remove(i);
                count -= 1;
                // ExplosionInteraction.TNT: the tnt_explosion_drop_decay rule.
                let interaction = self.destroy_type("tnt_explosion_drop_decay");
                self.with_hive_emptying_blast(|g| {
                    g.server_explode(center, 4.0, interaction, false)
                })?;
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
        self.spawn_primed_tnt(pos, false);
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

use horizon_world::{
    entity_fluid::{
        CurrentTarget, FluidInteractionWorld, FluidMembership, FluidSections, FluidTagId,
    },
    fluid::{FluidHeightWorld, FluidKind, FluidState},
    fluid_flow::{FluidFlowWorld, flow_at},
};
const WATER: FluidTagId = FluidTagId(0);
const LAVA: FluidTagId = FluidTagId(1);
struct TntFluidWorld<'a> {
    tnt: &'a mut PrimedTnt,
    world: &'a horizon_client::world::World,
    assets: &'a horizon_client::assets::Assets,
    facts: &'a horizon_client::blocks::BlockFacts,
}
impl TntFluidWorld<'_> {
    fn state(&self, p: BlockPos) -> horizon_world::registry::BlockStateId {
        self.world.block(p.x, p.y, p.z)
    }
    fn fluid_at(&self, p: BlockPos) -> std::io::Result<FluidState> {
        FluidState::from_block(
            self.assets
                .registry
                .state(self.state(p))
                .ok_or_else(|| std::io::Error::other("missing TNT fluid state"))?,
        )
    }
}
impl FluidMembership for TntFluidWorld<'_> {
    fn fluid_in_tag(&self, kind: FluidKind, tag: FluidTagId) -> bool {
        match tag {
            WATER => matches!(kind, FluidKind::Water | FluidKind::FlowingWater),
            LAVA => matches!(kind, FluidKind::Lava | FluidKind::FlowingLava),
            _ => false,
        }
    }
}
impl CurrentTarget for TntFluidWorld<'_> {
    type Error = std::io::Error;
    fn is_player_class(&self) -> bool {
        false
    }
    fn velocity(&mut self) -> std::io::Result<Vec3> {
        Ok(self.tnt.velocity)
    }
    fn add_delta_movement(&mut self, v: Vec3) -> std::io::Result<()> {
        self.tnt.velocity = Vec3::new(
            self.tnt.velocity.x + v.x,
            self.tnt.velocity.y + v.y,
            self.tnt.velocity.z + v.z,
        );
        Ok(())
    }
}
impl FluidInteractionWorld for TntFluidWorld<'_> {
    type Error = std::io::Error;
    type Chunk = (i32, i32);
    fn interaction_box(&mut self) -> std::io::Result<Option<Aabb>> {
        Ok(Some(
            self.tnt.bounds().deflate(Vec3::new(0.001, 0.001, 0.001)),
        ))
    }
    fn loaded_chunk(&mut self, x: i32, z: i32) -> std::io::Result<Option<Self::Chunk>> {
        Ok(self.world.chunks.contains_key(&(x, z)).then_some((x, z)))
    }
    fn chunk_sections(&mut self, _: Self::Chunk) -> std::io::Result<FluidSections<'_>> {
        static SECTIONS: [bool; horizon_client::chunk::SECTIONS] =
            [true; horizon_client::chunk::SECTIONS];
        Ok(FluidSections {
            minimum_section_y: horizon_client::chunk::MIN_Y / 16,
            has_fluid: &SECTIONS,
        })
    }
    fn bounding_box_min_y(&self) -> f64 {
        self.tnt.position.y
    }
    fn block_x(&self) -> i32 {
        self.tnt.position.x.floor() as i32
    }
    fn block_z(&self) -> i32 {
        self.tnt.position.z.floor() as i32
    }
    fn eye_y(&mut self) -> std::io::Result<f64> {
        Ok(self.tnt.position.y + f64::from(0.98f32 * 0.85f32))
    }
    fn fluid_state(&mut self, p: BlockPos) -> std::io::Result<FluidState> {
        self.fluid_at(p)
    }
    fn fluid_height(&mut self, s: FluidState, p: BlockPos) -> std::io::Result<f32> {
        s.height_at(self, [p.x, p.y, p.z])
    }
    fn camera_height(&mut self, s: FluidState, p: BlockPos) -> std::io::Result<f32> {
        s.camera_height_at(self, [p.x, p.y, p.z])
    }
    fn flow(&mut self, s: FluidState, p: BlockPos) -> std::io::Result<Vec3> {
        flow_at(self, s, p)
    }
}
impl FluidHeightWorld for TntFluidWorld<'_> {
    type Error = std::io::Error;
    fn fluid(&mut self, p: [i32; 3]) -> std::io::Result<FluidState> {
        self.fluid_at(BlockPos::new(p[0], p[1], p[2]))
    }
    fn sturdy_down(&mut self, p: [i32; 3]) -> std::io::Result<bool> {
        Ok(self
            .facts
            .get(self.state(BlockPos::new(p[0], p[1], p[2])))
            .sturdy_full
            & 1
            != 0)
    }
}
impl FluidFlowWorld for TntFluidWorld<'_> {
    type Error = std::io::Error;
    type Block = horizon_world::registry::BlockStateId;
    fn fluid(&mut self, p: BlockPos) -> std::io::Result<FluidState> {
        self.fluid_at(p)
    }
    fn block(&mut self, p: BlockPos) -> std::io::Result<Self::Block> {
        Ok(self.state(p))
    }
    fn blocks_fluid_flow(&self, s: Self::Block) -> bool {
        self.assets
            .blocks_fluid_flow
            .contains(&self.assets.registry.state(s).unwrap().name)
    }
    fn is_ice_class(&self, s: Self::Block) -> bool {
        self.facts
            .get(s)
            .classes
            .iter()
            .any(|c| c.as_ref() == "IceBlock")
    }
    fn is_face_sturdy(
        &mut self,
        s: Self::Block,
        _: BlockPos,
        d: horizon_core::geometry::Direction,
    ) -> std::io::Result<bool> {
        Ok(self.facts.get(s).sturdy_full & (1 << d as u8) != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            id: 0,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            fuse: 80,
            on_ground: false,
        };
        tnt.tick(&world, &collision, &assets, &facts).unwrap();
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
        tnt.tick(&world, &collision, &assets, &facts).unwrap();
        assert!((tnt.position.x - at.x - 0.014).abs() < 1e-12);
        assert!(tnt.on_ground);
        tnt.position = at;
        tnt.velocity = Vec3::ZERO;
        tnt.fuse = 1;
        tnt.tick(&world, &collision, &assets, &facts).unwrap();
        assert_eq!(tnt.fuse, 0);
        assert_eq!(
            tnt.velocity.x, 0.0,
            "expired TNT must not receive a current"
        );
        world.set(21, 4, 20, water).unwrap();
        tnt.position = at;
        tnt.velocity = Vec3::ZERO;
        tnt.fuse = 80;
        tnt.tick(&world, &collision, &assets, &facts).unwrap();
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
        tnt.tick(&world, &collision, &assets, &facts).unwrap();
        assert!(
            tnt.position.y < at.y && tnt.velocity.y < 0.0,
            "TNT sinks; water does not add buoyancy"
        );
    }
}
