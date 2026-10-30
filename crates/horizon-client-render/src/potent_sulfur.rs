//! `PotentSulfurBlockEntity`'s reads of the level: where the noxious gas of a
//! potent sulfur block comes out, how far a geyser throws, and whether the gas
//! reaches a point. The client's particles and the server's nausea and
//! launches read the level the same way, through a [`ParticleLevel`].
use crate::{blocks::Fluid, collisions::SceneCollisions, particles::ParticleLevel};
use horizon_core::{
    geometry::{Vec3, traverse_blocks},
    position::BlockPos,
};

/// `PotentSulfurBlock.ALLOWED_WATER_BLOCKS_ABOVE`.
const ALLOWED_WATER_BLOCKS_ABOVE: i32 = 4;

/// `PotentSulfurBlockEntity.isGeyserPassableBlock`: air, water, or a block
/// with no collision shape.
pub fn is_geyser_passable_block(level: &ParticleLevel, pos: [i32; 3]) -> bool {
    let state = level.world.block(pos[0], pos[1], pos[2]);
    if level.facts.is_air(state) || &*level.facts.get(state).name == "minecraft:water" {
        return true;
    }
    let mut scene = SceneCollisions {
        world: level.world,
        binding: level.collision,
        subject: None,
    };
    match level.collision.collision_shape(&mut scene, state, pos) {
        Ok(shape) => shape.is_empty(),
        Err(_) => !level.facts.get(state).full_collision,
    }
}

/// `PotentSulfurBlockEntity.isWater`: the block at `pos` holds a water source
/// (`FluidState.isSourceOfType`).
fn is_water_source(level: &ParticleLevel, pos: [i32; 3]) -> bool {
    let (fluid, source) = level.fluid_source(pos);
    fluid == Fluid::Water && source
}

/// `PotentSulfurBlockEntity.findNoxiousGasSourceBlock`: the first block above
/// the sulfur, within the water column's reach, that is not water-filled and
/// lets gas through; None when something solid or a deeper column stops it.
pub fn find_noxious_gas_source_block(level: &ParticleLevel, origin: [i32; 3]) -> Option<[i32; 3]> {
    let max_y = origin[1] + ALLOWED_WATER_BLOCKS_ABOVE + 1;
    let mut pos = [origin[0], origin[1] + 1, origin[2]];
    while pos[1] <= max_y {
        let state = level.world.block(pos[0], pos[1], pos[2]);
        let is_water_block = &*level.facts.get(state).name == "minecraft:water";
        if !is_water_source(level, pos)
            || (!is_water_block && !is_geyser_passable_block(level, pos))
        {
            if level.facts.is_air(state) || is_geyser_passable_block(level, pos) {
                return Some(pos);
            }
            break;
        }
        pos[1] += 1;
    }
    None
}

/// `PotentSulfurBlockEntity.getUnobstructedBlockCount`: how many blocks from
/// `pos` up, at most six per water block, a geyser's push goes through.
pub fn unobstructed_block_count(level: &ParticleLevel, pos: [i32; 3], water_blocks: i32) -> i32 {
    let geyser_force_height = 6 * water_blocks;
    for i in 0..geyser_force_height {
        if !is_geyser_passable_block(level, [pos[0], pos[1] + i, pos[2]]) {
            return i;
        }
    }
    geyser_force_height
}

/// `PotentSulfurBlockEntity.canBeReachedByNoxiousGas`: `pos` is in a block gas
/// can enter, within three blocks of the source block's centre, and above
/// water the source's water can be seen from.
pub fn can_be_reached_by_noxious_gas(
    level: &ParticleLevel,
    source_block: [i32; 3],
    pos: [f64; 3],
) -> bool {
    let block = ParticleLevel::block_at(pos);
    if !is_geyser_passable_block(level, block) {
        return false;
    }
    let center = source_block.map(|c| f64::from(c) + 0.5);
    let distance_sqr =
        (center[0] - pos[0]).powi(2) + (center[1] - pos[1]).powi(2) + (center[2] - pos[2]).powi(2);
    if distance_sqr > 9.0 {
        return false;
    }
    let below_source = [center[0], f64::from(source_block[1] - 1) + 0.5, center[2]];
    let below_pos = [pos[0], pos[1] - 1.0, pos[2]];
    is_water_source(level, ParticleLevel::block_at(below_pos))
        && have_line_of_sight(level, below_source, below_pos)
}

/// `PotentSulfurBlockEntity.haveLineOfSight`: a clip over collider shapes and
/// no fluids (as the mobs' line of sight does) finds no block between the
/// points.
fn have_line_of_sight(level: &ParticleLevel, a: [f64; 3], b: [f64; 3]) -> bool {
    let (from, to) = (Vec3::new(a[0], a[1], a[2]), Vec3::new(b[0], b[1], b[2]));
    let mut scene = SceneCollisions {
        world: level.world,
        binding: level.collision,
        subject: None,
    };
    let hit = traverse_blocks(from, to, 512, |p: BlockPos| {
        let state = level.world.block(p.x, p.y, p.z);
        if level.facts.is_air(state) {
            return None;
        }
        let shape = level
            .collision
            .collision_shape(&mut scene, state, [p.x, p.y, p.z])
            .ok()?;
        shape.clip(from, to, p).map(|_| ())
    });
    !matches!(hit, Ok(Some(())))
}
