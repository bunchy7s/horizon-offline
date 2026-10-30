//! What mobs read from the level: block solidity, ray clipping for line of
//! sight (`ClipContext.Block.COLLIDER`), sky visibility, light-dependent
//! values and the positions of other entities.
use super::{MobWorld, Seen, Who, data};
use crate::{
    blocks::{BlockFacts, Fluid},
    collisions::SceneCollisions,
};
use horizon_core::{
    geometry::{Aabb, Vec3, traverse_blocks},
    position::BlockPos,
};
use horizon_world::{
    block_collision::{BlockCollisionRegistry, DynamicCollisionWorld},
    registry::BlockStateId,
};

/// Where a target entity is this tick.
#[derive(Clone, Copy, Debug)]
pub struct TargetInfo {
    pub position: Vec3,
    pub eye_y: f64,
    pub bounds: Aabb,
    pub alive: bool,
    pub can_be_seen_as_enemy: bool,
    pub is_player: bool,
    pub creative: bool,
    /// `isPassenger`.
    pub passenger: bool,
}
impl MobWorld<'_> {
    /// The biome's base temperature at a position.
    pub fn temperature(&self, x: i32, y: i32, z: i32) -> f32 {
        self.world
            .chunks
            .get(&(x >> 4, z >> 4))
            .map_or(0.8, |c| self.biomes.get(c.biome(x, y, z)).temperature)
    }
    /// `Heightmap.Types.MOTION_BLOCKING`: above the highest blocking block.
    pub fn motion_blocking_height(&self, x: i32, z: i32) -> i32 {
        let top = crate::chunk::MIN_Y + crate::chunk::HEIGHT;
        (crate::chunk::MIN_Y..top)
            .rev()
            .find(|&y| {
                let f = self.facts.get(self.world.block(x, y, z));
                f.full_collision || f.fluid != crate::blocks::Fluid::None
            })
            .map_or(crate::chunk::MIN_Y, |y| y + 1)
    }
    /// BlockState.isSolid (the legacy solid flag).
    pub fn solid(&self, x: i32, y: i32, z: i32) -> bool {
        self.data.state(self.world.block(x, y, z)).flags & data::SOLID != 0
    }
    pub fn fluid(&self, x: i32, y: i32, z: i32) -> Fluid {
        self.facts.get(self.world.block(x, y, z)).fluid
    }
    pub fn block_name(&self, x: i32, y: i32, z: i32) -> &str {
        &self.facts.get(self.world.block(x, y, z)).name
    }
    /// A block state property's value.
    pub fn prop(&self, x: i32, y: i32, z: i32, key: &str) -> Option<&str> {
        self.registry
            .state(self.world.block(x, y, z))
            .and_then(|s| s.properties.get(key))
            .map(String::as_str)
    }
    pub fn has_tag(&self, tag: &str, block: &str) -> bool {
        self.block_tags.get(tag).is_some_and(|s| s.contains(block))
    }
    /// `findBlocksInBoxByManhattanDistance(at, range_xz, range_y).filterState(..).findFirst()`:
    /// the first block `matches` accepts (given its position and name), going
    /// out from `at` by Manhattan distance as `BlockPos.withinBoxByManhattanDistance` does.
    pub fn nearest_block(
        &self,
        at: [i32; 3],
        range_xz: i32,
        range_y: i32,
        matches: impl Fn([i32; 3], &str) -> bool,
    ) -> Option<[i32; 3]> {
        // Visiting order: distance, then x, then y, then +z before -z.
        let mut best: Option<((i32, i32, i32, bool), [i32; 3])> = None;
        for dx in -range_xz..=range_xz {
            for dy in -range_y..=range_y {
                for dz in -range_xz..=range_xz {
                    let key = (dx.abs() + dy.abs() + dz.abs(), dx, dy, dz < 0);
                    if best.is_some_and(|(k, _)| k <= key) {
                        continue;
                    }
                    let p = [at[0] + dx, at[1] + dy, at[2] + dz];
                    let state = self.world.block(p[0], p[1], p[2]);
                    if !self.facts.is_air(state) && matches(p, &self.facts.get(state).name) {
                        best = Some((key, p));
                    }
                }
            }
        }
        best.map(|(_, p)| p)
    }
    /// Level.clip with COLLIDER shapes (optionally stopping at any fluid): true on a miss.
    pub fn clear_between(&self, from: Vec3, to: Vec3, fluids_block: bool) -> bool {
        let mut scene = SceneCollisions {
            world: self.world,
            binding: self.collision,
            subject: None,
        };
        let result = traverse_blocks(from, to, 512, |p: BlockPos| {
            let state = self.world.block(p.x, p.y, p.z);
            if fluids_block && self.facts.get(state).fluid != Fluid::None {
                return Some(());
            }
            if self.facts.is_air(state) {
                return None;
            }
            let shape = self
                .collision
                .collision_shape(&mut scene, state, [p.x, p.y, p.z])
                .ok()?;
            shape.clip(from, to, p).map(|_| ())
        });
        matches!(result, Ok(None))
    }
    /// Level.clip with VISUAL shapes and no fluids: true on a miss.
    pub fn visual_clear_between(&self, from: Vec3, to: Vec3) -> bool {
        self.visual_clip(from, to) == Some(None)
    }
    /// Level.clip with VISUAL shapes and no fluids: the block the segment
    /// first hits, if it does.
    pub fn visual_hit(&self, from: Vec3, to: Vec3) -> Option<[i32; 3]> {
        self.visual_clip(from, to).flatten()
    }
    fn visual_clip(&self, from: Vec3, to: Vec3) -> Option<Option<[i32; 3]>> {
        let mut scene = SceneCollisions {
            world: self.world,
            binding: self.collision,
            subject: None,
        };
        let state = |x, y, z| self.world.block(x, y, z);
        visual_clip(self.facts, self.collision, &mut scene, &state, from, to)
    }
    /// Level.clip with COLLIDER shapes and no fluids: where the segment
    /// first hits a block, if it does.
    pub fn clip_block(&self, from: Vec3, to: Vec3) -> Option<Vec3> {
        let mut scene = SceneCollisions {
            world: self.world,
            binding: self.collision,
            subject: None,
        };
        let result = traverse_blocks(from, to, 512, |p: BlockPos| {
            let state = self.world.block(p.x, p.y, p.z);
            if self.facts.is_air(state) {
                return None;
            }
            let shape = self
                .collision
                .collision_shape(&mut scene, state, [p.x, p.y, p.z])
                .ok()?;
            shape.clip(from, to, p).map(|hit| hit.location)
        });
        result.ok().flatten()
    }
    /// Level.clip with COLLIDER shapes and no fluids: the block the segment
    /// first hits, if it does.
    pub fn clip_block_at(&self, from: Vec3, to: Vec3) -> Option<[i32; 3]> {
        let mut scene = SceneCollisions {
            world: self.world,
            binding: self.collision,
            subject: None,
        };
        let result = traverse_blocks(from, to, 512, |p: BlockPos| {
            let state = self.world.block(p.x, p.y, p.z);
            if self.facts.is_air(state) {
                return None;
            }
            let shape = self
                .collision
                .collision_shape(&mut scene, state, [p.x, p.y, p.z])
                .ok()?;
            shape.clip(from, to, p).map(|_| [p.x, p.y, p.z])
        });
        result.ok().flatten()
    }
    /// The bounds of the block's collision shape, relative to the block.
    pub fn collision_bounds(&self, p: [i32; 3]) -> Option<horizon_core::geometry::Aabb> {
        let mut scene = SceneCollisions {
            world: self.world,
            binding: self.collision,
            subject: None,
        };
        let state = self.world.block(p[0], p[1], p[2]);
        self.collision
            .collision_shape(&mut scene, state, p)
            .ok()?
            .bounds()
    }
    /// LivingEntity.hasLineOfSight: eye to eye within 128 blocks.
    pub fn line_of_sight(&self, from_eye: Vec3, to: Vec3) -> bool {
        let d =
            (to.x - from_eye.x).powi(2) + (to.y - from_eye.y).powi(2) + (to.z - from_eye.z).powi(2);
        if d > 128.0 * 128.0 {
            return false;
        }
        self.clear_between(from_eye, to, false)
    }
    /// Level.canSeeSky: sky light 15 at the position (no roof above).
    pub fn can_see_sky(&self, x: i32, y: i32, z: i32) -> bool {
        if self.nether {
            return false;
        }
        self.world.light(x, y, z) >> 4 == 15
    }
    /// Level.precipitationAt: raining, open to the sky, nothing motion
    /// blocking above, then the biome's precipitation at the position:
    /// `Some(true)` rain, `Some(false)` snow, `None` nothing.
    pub fn precipitation_at(&self, x: i32, y: i32, z: i32) -> Option<bool> {
        if !self.raining || !self.can_see_sky(x, y, z) || self.motion_blocking_height(x, z) > y {
            return None;
        }
        let chunk = self.world.chunks.get(&(x >> 4, z >> 4))?;
        let biome = self.biomes.get(chunk.biome(x, y, z));
        if !biome.precipitation {
            return None;
        }
        Some(biome.temperature_at(BlockPos::new(x, y, z), self.sea_level) >= 0.15)
    }
    /// Level.isRainingAt: the precipitation at the position is rain, not snow
    /// or none.
    pub fn raining_at(&self, x: i32, y: i32, z: i32) -> bool {
        self.precipitation_at(x, y, z) == Some(true)
    }
    /// Level.getLightLevelDependentMagicValue: the dimension's brightness curve
    /// at the raw (sky darkened) light level.
    pub fn brightness(&self, x: i32, y: i32, z: i32) -> f32 {
        let light = self.world.light(x, y, z);
        let sky = i32::from(light >> 4) - self.sky_darken();
        let level = sky.max(i32::from(light & 15)).clamp(0, 15) as f32;
        // DimensionType.getBrightness with ambient light 0 (overworld) / 0.1 (nether).
        let f = level / 15.0;
        let base = f / (4.0 - 3.0 * f);
        if self.nether { 0.1 + base * 0.9 } else { base }
    }
    pub fn sky_darken(&self) -> i32 {
        if self.daylight { 0 } else { 11 }
    }
    pub fn target_info(&self, who: Who) -> Option<TargetInfo> {
        match who {
            Who::Player => self.player.as_ref().map(|p| TargetInfo {
                position: p.position,
                eye_y: p.eye[1],
                bounds: p.bounds,
                alive: p.alive,
                can_be_seen_as_enemy: true,
                is_player: true,
                creative: p.creative,
                passenger: p.passenger,
            }),
            Who::Mob(id) => self.seen(id).map(|s| TargetInfo {
                position: s.position,
                eye_y: s.eye_y,
                bounds: s.bounds,
                alive: s.alive,
                can_be_seen_as_enemy: !s.playing_dead,
                is_player: false,
                creative: false,
                passenger: s.passenger,
            }),
        }
    }
    /// The other mobs whose boxes may meet `area` (callers test the boxes),
    /// in snapshot order: through the grid when there is one, else all.
    pub fn others_near(&self, area: Aabb) -> impl Iterator<Item = &Seen> {
        let mut near = Vec::new();
        let indexed = self
            .others_grid
            .is_some_and(|grid| grid.candidates(area, &mut near));
        let all = (!indexed).then(|| self.others.iter());
        near.into_iter()
            .map(|i| &self.others[i as usize])
            .chain(all.into_iter().flatten())
    }
    pub fn seen(&self, id: u32) -> Option<&Seen> {
        match self.others_index {
            Some(index) => index
                .get(&id)
                .and_then(|&i| self.others.get(i as usize))
                .filter(|s| s.id == id),
            None => self.others.iter().find(|s| s.id == id),
        }
    }
    /// Living mobs of the given species within `range` of `from`, nearest first.
    pub fn nearby<'s>(
        &'s self,
        from: Vec3,
        range: f64,
        pred: impl Fn(&Seen) -> bool + 's,
    ) -> impl Iterator<Item = &'s Seen> + 's {
        // A position within `range` puts the box's bottom face in this area
        // (one block wider, as the box test is strict).
        let r = range + 1.0;
        let area = Aabb::new(
            Vec3::new(from.x - r, from.y - r, from.z - r),
            Vec3::new(from.x + r, from.y + r, from.z + r),
        );
        let mut v: Vec<&Seen> = self
            .others_near(area)
            .filter(|s| {
                s.alive
                    && (s.position.x - from.x).abs() <= range
                    && (s.position.y - from.y).abs() <= range
                    && (s.position.z - from.z).abs() <= range
            })
            .filter(move |s| pred(s))
            .collect();
        v.sort_by(|a, b| {
            let da = (a.position.x - from.x).powi(2)
                + (a.position.y - from.y).powi(2)
                + (a.position.z - from.z).powi(2);
            let db = (b.position.x - from.x).powi(2)
                + (b.position.y - from.y).powi(2)
                + (b.position.z - from.z).powi(2);
            da.total_cmp(&db)
        });
        v.into_iter()
    }
}

/// Level.clip with VISUAL shapes and no fluids: the block the segment first
/// hits, None inside when it hits nothing and outside when the segment is
/// too long to walk. Glass and copper grates (`TransparentBlock`) and bars and panes
/// (`IronBarsBlock`) have no visual shape; mud, soul sand and powder snow
/// look like full blocks.
pub fn visual_clip<W: DynamicCollisionWorld>(
    facts: &BlockFacts,
    collision: &BlockCollisionRegistry,
    scene: &mut W,
    state: &dyn Fn(i32, i32, i32) -> BlockStateId,
    from: Vec3,
    to: Vec3,
) -> Option<Option<[i32; 3]>> {
    let full = Aabb::new(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0));
    let result = traverse_blocks(from, to, 512, |p: BlockPos| {
        let id = state(p.x, p.y, p.z);
        if facts.is_air(id) {
            return None;
        }
        let name = facts.get(id).name.trim_start_matches("minecraft:");
        if name.ends_with("glass")
            || name.ends_with("glass_pane")
            || name.ends_with("_bars")
            || name.ends_with("copper_grate")
        {
            return None;
        }
        if matches!(name, "mud" | "soul_sand" | "powder_snow") {
            return full
                .move_by(Vec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z)))
                .clip(from, to)
                .map(|_| [p.x, p.y, p.z]);
        }
        let shape = collision.collision_shape(scene, id, [p.x, p.y, p.z]).ok()?;
        shape.clip(from, to, p).map(|_| [p.x, p.y, p.z])
    });
    result.ok()
}

/// Runs `f` with an empty overworld (`others` its mobs) and the mob types,
/// both from the extracted vanilla data; false, without running it, when
/// that data is missing.
#[cfg(test)]
pub(crate) fn with_test_world(
    others: &[Seen],
    f: impl FnOnce(&MobWorld<'_>, &super::MobTypes),
) -> bool {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("entity-types.json").exists() || !root.join("movement-properties.nbt").exists() {
        return false;
    }
    let assets = crate::assets::Assets::load(&root).unwrap();
    let registry = assets.registry;
    let facts = BlockFacts::load(&root, registry).unwrap();
    let motion = crate::motion::MotionResources::load(&root, &assets).unwrap();
    let data = super::MobData::load(&root, &facts);
    let types = super::MobTypes::load(&root).unwrap();
    let biomes = crate::biome::Biomes::load(&root).unwrap();
    let items = crate::items::Items::load(
        &root,
        registry,
        &facts,
        &horizon_world::loot::PackOverlay::NONE,
    )
    .unwrap();
    let collision = crate::collisions::load(&root, registry).unwrap();
    let air = registry.default_state("minecraft:air").unwrap();
    let world = crate::world::World::new(air, 0);
    let lid_openers = std::collections::HashMap::new();
    let w = MobWorld {
        teams: Default::default(),
        world: &world,
        registry,
        facts: &facts,
        fluid_tables: &motion.fluids,
        movement: &motion.blocks,
        collision: &collision,
        data: &data,
        player: None,
        nearest_player: None,
        others,
        others_grid: None,
        others_index: None,
        daylight: true,
        monsters_burn: false,
        game_time: 1000,
        day_time: 18000,
        raining: false,
        thundering: false,
        nether: false,
        block_tags: &items.block_tags,
        biomes: &biomes,
        mob_griefing: true,
        mob_drops: true,
        max_entity_cramming: 24,
        difficulty: super::spawning::LocalDifficulty::new(2, 18000, 0, 0.0),
        end: false,
        world_seed: 0,
        sea_level: 63,
        items: Some(&items),
        lid_openers: &lid_openers,
    };
    f(&w, &types);
    true
}
