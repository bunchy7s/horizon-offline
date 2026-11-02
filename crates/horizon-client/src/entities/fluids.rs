//! Dropped-item and armor-stand immersion and currents through the shared vanilla fluid helpers.
use super::{ArmorStand, ItemEntity};
use crate::{
    motion::{FluidResources, FluidTags, LAVA, WATER},
    world::World,
};
use horizon_core::{
    geometry::{Aabb, Direction, Vec3},
    position::BlockPos,
};
use horizon_world::{
    entity_fluid::{
        CurrentTarget, EntityFluidInteraction, FluidInteractionWorld, FluidMembership,
        FluidSections, FluidTagId,
    },
    fluid::{FluidHeightWorld, FluidKind, FluidState},
    fluid_flow::{FluidFlowWorld, flow_at},
    registry::BlockStateId,
};
type Error = std::convert::Infallible;
struct EntityFluidWorld<'a> {
    bounds: Aabb,
    position: Vec3,
    eye_y: f64,
    velocity: &'a mut Vec3,
    world: &'a World,
    fluids: &'a FluidResources,
}
/// `Entity.baseTick`'s fluid step for an entity that is pushed by fluid: its
/// immersion heights, then the current of each fluid it sits in, added to
/// `velocity` with vanilla's scales (water 0.014, lava 0.007 fast or
/// 0.0023333 slow). Items and armour stands call it for themselves; a boat
/// calls it from its own tick, before `floatBoat` scales the velocity.
pub fn update_entity_fluids(
    bounds: Aabb,
    position: Vec3,
    eye_y: f64,
    velocity: &mut Vec3,
    world: &World,
    fluids: &FluidResources,
) -> (f64, f64) {
    let mut interaction = EntityFluidInteraction::new([WATER, LAVA]);
    let mut scene = EntityFluidWorld {
        bounds,
        position,
        eye_y,
        velocity,
        world,
        fluids,
    };
    if interaction.update(&mut scene, false, 4096).is_err() {
        return (0.0, 0.0);
    }
    let water_height = interaction.fluid_height(&scene, WATER);
    let lava_height = interaction.fluid_height(&scene, LAVA);
    let _ = interaction.apply_current(&mut scene, WATER, 0.014);
    let _ = interaction.apply_current(
        &mut scene,
        LAVA,
        if world.fast_lava {
            0.007
        } else {
            0.0023333333333333335
        },
    );
    (water_height, lava_height)
}
impl ItemEntity {
    pub(super) fn update_fluids(&mut self, world: &World, fluids: &FluidResources) -> (f64, f64) {
        update_entity_fluids(
            self.bounds(),
            self.position,
            self.position.y + f64::from(0.2125f32),
            &mut self.velocity,
            world,
            fluids,
        )
    }
}
impl ArmorStand {
    /// `Entity.updateFluidHeightAndDoFluidPushing`: refreshes water and lava
    /// immersion over the stand's deflated box and pushes its velocity by any
    /// water or lava current. Returns whether the stand is in water (`isInWater`).
    pub fn update_fluids(&mut self, world: &World, fluids: &FluidResources) -> bool {
        if self.marker {
            return false;
        }
        let eye = if self.small {
            f64::from(0.9875f32 * 0.9)
        } else {
            f64::from(1.7775f32)
        };
        let (water_height, _) = update_entity_fluids(
            self.bounds(),
            self.position,
            self.position.y + eye,
            &mut self.velocity,
            world,
            fluids,
        );
        water_height > 0.0
    }
}
impl FluidMembership for EntityFluidWorld<'_> {
    fn fluid_in_tag(&self, kind: FluidKind, tag: FluidTagId) -> bool {
        FluidTags.fluid_in_tag(kind, tag)
    }
}
impl CurrentTarget for EntityFluidWorld<'_> {
    type Error = Error;
    fn is_player_class(&self) -> bool {
        false
    }
    fn velocity(&mut self) -> Result<Vec3, Error> {
        Ok(*self.velocity)
    }
    fn add_delta_movement(&mut self, delta: Vec3) -> Result<(), Error> {
        let v = *self.velocity;
        let sum = Vec3::new(v.x + delta.x, v.y + delta.y, v.z + delta.z);
        if delta.is_finite() && sum.is_finite() {
            *self.velocity = sum;
        }
        Ok(())
    }
}
impl FluidInteractionWorld for EntityFluidWorld<'_> {
    type Error = Error;
    type Chunk = ();
    fn interaction_box(&mut self) -> Result<Option<Aabb>, Error> {
        Ok(Some(self.bounds.deflate(Vec3::new(0.001, 0.001, 0.001))))
    }
    fn loaded_chunk(&mut self, x: i32, z: i32) -> Result<Option<()>, Error> {
        Ok(self.world.chunks.contains_key(&(x, z)).then_some(()))
    }
    fn chunk_sections(&mut self, _: ()) -> Result<FluidSections<'_>, Error> {
        let layout = self.world.layout();
        Ok(FluidSections {
            minimum_section_y: layout.min_y() >> 4,
            has_fluid: crate::chunk::every_section(layout),
        })
    }
    fn bounding_box_min_y(&self) -> f64 {
        self.position.y
    }
    fn block_x(&self) -> i32 {
        self.position.x.floor() as i32
    }
    fn eye_y(&mut self) -> Result<f64, Error> {
        Ok(self.eye_y)
    }
    fn block_z(&self) -> i32 {
        self.position.z.floor() as i32
    }
    fn fluid_state(&mut self, p: BlockPos) -> Result<FluidState, Error> {
        Ok(self.fluids.fluid(self.world.block(p.x, p.y, p.z)))
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
impl FluidHeightWorld for EntityFluidWorld<'_> {
    type Error = Error;
    fn fluid(&mut self, p: [i32; 3]) -> Result<FluidState, Error> {
        Ok(self.fluids.fluid(self.world.block(p[0], p[1], p[2])))
    }
    fn sturdy_down(&mut self, p: [i32; 3]) -> Result<bool, Error> {
        let state = self.world.block(p[0], p[1], p[2]);
        Ok(self.fluids.sturdy(state, Direction::Down))
    }
}
impl FluidFlowWorld for EntityFluidWorld<'_> {
    type Error = Error;
    type Block = BlockStateId;
    fn fluid(&mut self, p: BlockPos) -> Result<FluidState, Error> {
        Ok(self.fluids.fluid(self.world.block(p.x, p.y, p.z)))
    }
    fn block(&mut self, p: BlockPos) -> Result<BlockStateId, Error> {
        Ok(self.world.block(p.x, p.y, p.z))
    }
    fn blocks_fluid_flow(&self, s: BlockStateId) -> bool {
        self.fluids.blocks_flow(s)
    }
    fn is_ice_class(&self, s: BlockStateId) -> bool {
        self.fluids.ice(s)
    }
    fn is_face_sturdy(
        &mut self,
        s: BlockStateId,
        _: BlockPos,
        d: Direction,
    ) -> Result<bool, Error> {
        Ok(self.fluids.sturdy(s, d))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn items_follow_flowing_water_but_still_water_has_no_horizontal_push() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = FluidResources::new(&assets, &facts).unwrap();
        let collision = crate::collisions::load(&root, assets.registry).unwrap();
        let mut world = World::test_scene(&assets, &render, 3).unwrap();
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let mut state = assets.registry.state(water).unwrap().clone();
        state.properties.insert("level".into(), "1".into());
        let flowing = assets.registry.resolve(&state).unwrap();
        for x in 19..=21 {
            for z in 19..=21 {
                world.set(x, 3, z, stone).unwrap();
                world.set(x, 4, z, water).unwrap();
            }
        }
        let at = Vec3::new(20.5, 4.0, 20.5);
        let item = ItemEntity {
            no_gravity: false,
            id: 1,
            health: 5,
            fire_ticks: 0,
            stack: crate::items::ItemStack {
                item: "minecraft:sugar_cane".into(),
                count: 1,
                damage: 0,
                extra: None,
            },
            thrower: None,
            target: None,
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
        let mut still = item.clone();
        assert!(still.tick(
            &world,
            assets.registry,
            &facts,
            &collision,
            false,
            &fluids,
            &[]
        ));
        assert_eq!(still.position.x, at.x);
        assert_eq!(still.velocity.x, 0.0);
        world.set(21, 4, 20, flowing).unwrap();
        let mut carried = item;
        assert!(carried.tick(
            &world,
            assets.registry,
            &facts,
            &collision,
            false,
            &fluids,
            &[]
        ));
        // First push is damped by buoyancy before motion, second follows drag.
        assert!((carried.position.x - at.x - 0.014 * 0.99).abs() < 1e-12);
        assert!((carried.velocity.x - (0.014 * 0.99 * 0.98 + 0.014)).abs() < 1e-12);
        for _ in 0..20 {
            carried.tick(
                &world,
                assets.registry,
                &facts,
                &collision,
                false,
                &fluids,
                &[],
            );
        }
        assert!(carried.position.x > at.x + 0.5);
    }

    /// `Entity.baseTick` pushes a boat by the current it sits in: a boat box
    /// in water[level=1..7] gains velocity along the flow, and the same box in
    /// still source water does not. Vanilla's boat tick runs this before
    /// `floatBoat` scales the velocity, which is what `game::boats` does.
    #[test]
    fn a_boat_box_is_pushed_by_the_current_it_sits_in() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = FluidResources::new(&assets, &facts).unwrap();
        let mut world = World::test_scene(&assets, &render, 3).unwrap();
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let mut state = assets.registry.state(water).unwrap().clone();
        state.properties.insert("level".into(), "1".into());
        let flowing = assets.registry.resolve(&state).unwrap();
        for x in 19..=21 {
            for z in 19..=21 {
                world.set(x, 3, z, stone).unwrap();
                world.set(x, 4, z, water).unwrap();
            }
        }
        // Boat dimensions: 1.375 wide, 0.5625 tall, eye at its top.
        let at = Vec3::new(20.5, 4.0, 20.5);
        let half = 1.375 / 2.0;
        let bounds = Aabb::new(
            Vec3::new(at.x - half, at.y, at.z - half),
            Vec3::new(at.x + half, at.y + 0.5625, at.z + half),
        );
        let mut still = Vec3::ZERO;
        update_entity_fluids(bounds, at, at.y + 0.5625, &mut still, &world, &fluids);
        assert_eq!(still.x, 0.0, "source water has no current");
        world.set(21, 4, 20, flowing).unwrap();
        let mut carried = Vec3::ZERO;
        update_entity_fluids(bounds, at, at.y + 0.5625, &mut carried, &world, &fluids);
        assert!(
            carried.x > 0.0,
            "the flow pushes the boat along +x, got {}",
            carried.x
        );
    }

    #[test]
    fn armor_stands_follow_flowing_water_but_markers_and_still_water_stay_in_place() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = FluidResources::new(&assets, &facts).unwrap();
        let collision = crate::collisions::load(&root, assets.registry).unwrap();
        let mut world = World::test_scene(&assets, &render, 3).unwrap();
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let mut state = assets.registry.state(water).unwrap().clone();
        state.properties.insert("level".into(), "1".into());
        let flowing = assets.registry.resolve(&state).unwrap();
        for x in 19..=21 {
            for z in 19..=21 {
                world.set(x, 3, z, stone).unwrap();
                world.set(x, 4, z, water).unwrap();
            }
        }
        let at = Vec3::new(20.5, 4.0, 20.5);
        let stand = ArmorStand::new(1, at, 0.0);
        let mut still = stand.clone();
        let in_water = still.update_fluids(&world, &fluids);
        assert!(in_water);
        let mut scene = crate::collisions::SceneCollisions {
            world: &world,
            binding: &collision,
            subject: None,
        };
        still.tick(&mut scene, &facts, in_water, &mut |_, _| {});
        assert_eq!(still.position.x, at.x);
        assert_eq!(still.velocity.x, 0.0);

        world.set(21, 4, 20, flowing).unwrap();
        let mut marker = stand.clone();
        marker.marker = true;
        let in_water = marker.update_fluids(&world, &fluids);
        assert!(!in_water);
        let mut scene = crate::collisions::SceneCollisions {
            world: &world,
            binding: &collision,
            subject: None,
        };
        marker.tick(&mut scene, &facts, in_water, &mut |_, _| {});
        assert_eq!(marker.position, at);
        assert_eq!(marker.velocity, Vec3::ZERO);

        let mut carried = stand;
        let in_water = carried.update_fluids(&world, &fluids);
        assert!(in_water);
        let mut scene = crate::collisions::SceneCollisions {
            world: &world,
            binding: &collision,
            subject: None,
        };
        carried.tick(&mut scene, &facts, in_water, &mut |_, _| {});
        assert!((carried.position.x - (at.x + 0.014)).abs() < 1e-12);
        assert!((carried.velocity.x - 0.014 * f64::from(0.8f32)).abs() < 1e-12);
        for _ in 0..20 {
            let in_water = carried.update_fluids(&world, &fluids);
            let mut scene = crate::collisions::SceneCollisions {
                world: &world,
                binding: &collision,
                subject: None,
            };
            carried.tick(&mut scene, &facts, in_water, &mut |_, _| {});
        }
        assert!(carried.position.x > at.x + 0.5);
    }
}
