//! Entity fluid immersion trackers and current accumulation across update ticks.
use crate::fluid::{FluidKind, FluidState};
use horizon_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FluidTagId(pub u32);
pub trait FluidMembership {
    fn fluid_in_tag(&self, fluid: FluidKind, tag: FluidTagId) -> bool;
}
#[derive(Clone, Copy, Debug)]
pub struct FluidTracker {
    pub fluid: FluidKind,
    pub height: f64,
    pub eyes_inside: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct CurrentAccumulator {
    pub height: f64,
    pub accumulated: Vec3,
    pub count: i32,
}
impl Default for CurrentAccumulator {
    fn default() -> Self {
        Self {
            height: 0.0,
            accumulated: Vec3::ZERO,
            count: 0,
        }
    }
}
pub trait CurrentTarget {
    type Error;
    fn is_player_class(&self) -> bool;
    fn velocity(&mut self) -> Result<Vec3, Self::Error>;
    /// Entity.addDeltaMovement, including finite-addition filtering and a fresh
    /// velocity read. Entity-specific overrides remain the provider's dispatch.
    fn add_delta_movement(&mut self, delta: Vec3) -> Result<(), Self::Error>;
}
impl CurrentAccumulator {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn accumulate(&mut self, current: Vec3) {
        self.accumulated = add(self.accumulated, current);
        self.count = self.count.wrapping_add(1);
    }
    pub fn apply_to<W: CurrentTarget>(&self, world: &mut W, speed: f64) -> Result<(), W::Error> {
        if self.count == 0 || self.accumulated.length_squared() < f64::from(1.0e-5f32) {
            return Ok(());
        }
        let mut current = if world.is_player_class() {
            self.accumulated.scale(1.0 / f64::from(self.count))
        } else {
            self.accumulated.normalize()
        };
        let velocity = world.velocity()?;
        current = current.scale(speed);
        if velocity.x.abs() < 0.003
            && velocity.z.abs() < 0.003
            && current.length() < 0.0045000000000000005
        {
            current = current.normalize().scale(0.0045000000000000005);
        }
        world.add_delta_movement(current)
    }
}
pub struct FluidSections<'a> {
    pub minimum_section_y: i32,
    pub has_fluid: &'a [bool],
}
pub trait FluidInteractionWorld: FluidMembership {
    type Error;
    type Chunk: Copy;
    fn interaction_box(&mut self) -> Result<Option<Aabb>, Self::Error>;
    /// FULL chunk lookup without generating/loading absent chunks.
    fn loaded_chunk(&mut self, x: i32, z: i32) -> Result<Option<Self::Chunk>, Self::Error>;
    fn chunk_sections(&mut self, chunk: Self::Chunk) -> Result<FluidSections<'_>, Self::Error>;
    fn bounding_box_min_y(&self) -> f64;
    fn block_x(&self) -> i32;
    fn eye_y(&mut self) -> Result<f64, Self::Error>;
    fn block_z(&self) -> i32;
    fn fluid_state(&mut self, position: BlockPos) -> Result<FluidState, Self::Error>;
    fn fluid_height(&mut self, state: FluidState, position: BlockPos) -> Result<f32, Self::Error>;
    fn camera_height(&mut self, state: FluidState, position: BlockPos) -> Result<f32, Self::Error>;
    fn flow(&mut self, state: FluidState, position: BlockPos) -> Result<Vec3, Self::Error>;
}
#[derive(Debug)]
pub enum FluidInteractionError<E> {
    World(E),
    NonFiniteBounds,
    BudgetExceeded,
}
impl<E: fmt::Display> fmt::Display for FluidInteractionError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::NonFiniteBounds => f.write_str("nonfinite immersion bounds"),
            Self::BudgetExceeded => f.write_str("immersion query budget exceeded"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for FluidInteractionError<E> {}
fn spend<E>(budget: &mut usize) -> Result<(), FluidInteractionError<E>> {
    if *budget == 0 {
        return Err(FluidInteractionError::BudgetExceeded);
    }
    *budget -= 1;
    Ok(())
}
fn add(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.x + b.x, a.y + b.y, a.z + b.z)
}
fn java_max(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        a
    } else if b.is_nan() {
        b
    } else if a == 0.0 && b == 0.0 {
        f64::from_bits(a.to_bits() & b.to_bits())
    } else {
        a.max(b)
    }
}
/// Every required chunk must be loaded, even after a wet section is found.
/// Chunk order is Z then X; section order is ascending Y without early success.
pub fn has_fluid_and_loaded<W: FluidInteractionWorld>(
    world: &mut W,
    min: BlockPos,
    max: BlockPos,
    budget: &mut usize,
) -> Result<bool, FluidInteractionError<W::Error>> {
    let mut present = false;
    for z in (min.z >> 4)..=(max.z >> 4) {
        for x in (min.x >> 4)..=(max.x >> 4) {
            spend(budget)?;
            let Some(chunk) = world
                .loaded_chunk(x, z)
                .map_err(FluidInteractionError::World)?
            else {
                return Ok(false);
            };
            let sections = world
                .chunk_sections(chunk)
                .map_err(FluidInteractionError::World)?;
            for y in (min.y >> 4)..=(max.y >> 4) {
                spend(budget)?;
                let index = y.wrapping_sub(sections.minimum_section_y);
                if index >= 0 && (index as usize) < sections.has_fluid.len() {
                    present |= sections.has_fluid[index as usize];
                }
            }
        }
    }
    Ok(present)
}
#[derive(Clone, Default)]
pub struct EntityFluidInteraction {
    trackers: Vec<FluidTracker>,
    currents: Vec<(FluidTagId, CurrentAccumulator)>,
}
impl EntityFluidInteraction {
    /// Preserve the caller's tag-set iteration order: a fluid's current belongs
    /// to the first matching tag, even when several tags contain that fluid.
    pub fn new(tags: impl IntoIterator<Item = FluidTagId>) -> Self {
        let mut state = Self::default();
        for tag in tags {
            if !state.currents.iter().any(|(t, _)| *t == tag) {
                state.currents.push((tag, CurrentAccumulator::default()));
            }
        }
        state
    }
    pub fn trackers(&self) -> &[FluidTracker] {
        &self.trackers
    }
    pub fn currents(&self) -> &[(FluidTagId, CurrentAccumulator)] {
        &self.currents
    }
    pub fn fluid_height<W: FluidMembership>(&self, tags: &W, tag: FluidTagId) -> f64 {
        let mut height = 0.0;
        for t in &self.trackers {
            if t.height > height && tags.fluid_in_tag(t.fluid, tag) {
                height = t.height;
            }
        }
        height
    }
    pub fn is_in_fluid<W: FluidMembership>(&self, tags: &W, tag: FluidTagId) -> bool {
        self.fluid_height(tags, tag) > 0.0
    }
    pub fn is_eye_in_fluid<W: FluidMembership>(&self, tags: &W, tag: FluidTagId) -> bool {
        self.trackers
            .iter()
            .any(|t| t.eyes_inside && tags.fluid_in_tag(t.fluid, tag))
    }
    pub fn apply_current<W: CurrentTarget>(
        &self,
        world: &mut W,
        tag: FluidTagId,
        speed: f64,
    ) -> Result<(), W::Error> {
        if let Some((_, current)) = self.currents.iter().find(|(t, _)| *t == tag) {
            current.apply_to(world, speed)?;
        }
        Ok(())
    }
    /// Reset previous tick state before any query. Provider failures and budget
    /// exhaustion retain all preceding tracker/current writes.
    pub fn update<W: FluidInteractionWorld>(
        &mut self,
        world: &mut W,
        ignore_current: bool,
        mut budget: usize,
    ) -> Result<bool, FluidInteractionError<W::Error>> {
        self.trackers.retain_mut(|t| {
            if t.height == 0.0 && !t.eyes_inside {
                return false;
            }
            t.height = 0.0;
            t.eyes_inside = false;
            true
        });
        for (_, c) in &mut self.currents {
            c.reset();
        }
        let Some(bounds) = world
            .interaction_box()
            .map_err(FluidInteractionError::World)?
        else {
            return Ok(false);
        };
        let a = bounds.min_position();
        let b = bounds.max_position();
        if [a.x, a.y, a.z, b.x, b.y, b.z]
            .iter()
            .any(|v| !v.is_finite())
        {
            return Err(FluidInteractionError::NonFiniteBounds);
        }
        let low = BlockPos::new(a.x.floor() as i32, a.y.floor() as i32, a.z.floor() as i32);
        let high = BlockPos::new(
            (b.x.ceil() as i32).wrapping_sub(1),
            (b.y.ceil() as i32).wrapping_sub(1),
            (b.z.ceil() as i32).wrapping_sub(1),
        );
        if !has_fluid_and_loaded(
            world,
            BlockPos::new(low.x.wrapping_sub(1), low.y, low.z.wrapping_sub(1)),
            BlockPos::new(high.x.wrapping_add(1), high.y, high.z.wrapping_add(1)),
            &mut budget,
        )? {
            return Ok(false);
        }
        let base_y = world.bounding_box_min_y();
        let block_x = world.block_x();
        let eye = world.eye_y().map_err(FluidInteractionError::World)?;
        let block_z = world.block_z();
        let mut last: Option<(FluidKind, usize, Option<usize>)> = None;
        for x in low.x..=high.x {
            for y in low.y..=high.y {
                for z in low.z..=high.z {
                    spend(&mut budget)?;
                    let p = BlockPos::new(x, y, z);
                    let state = world.fluid_state(p).map_err(FluidInteractionError::World)?;
                    if state.kind() == FluidKind::Empty {
                        continue;
                    }
                    let surface = f64::from(y)
                        + f64::from(
                            world
                                .fluid_height(state, p)
                                .map_err(FluidInteractionError::World)?,
                        );
                    if surface < a.y {
                        continue;
                    }
                    let kind = state.kind();
                    let (_, index, current) = if let Some(last) =
                        last.filter(|(k, _, _)| *k == kind)
                    {
                        last
                    } else {
                        let index =
                            if let Some(i) = self.trackers.iter().position(|t| t.fluid == kind) {
                                i
                            } else {
                                self.trackers.push(FluidTracker {
                                    fluid: kind,
                                    height: 0.0,
                                    eyes_inside: false,
                                });
                                self.trackers.len() - 1
                            };
                        let current = if ignore_current {
                            None
                        } else {
                            self.currents
                                .iter()
                                .position(|(tag, _)| world.fluid_in_tag(kind, *tag))
                        };
                        let entry = (kind, index, current);
                        last = Some(entry);
                        entry
                    };
                    if x == block_x && z == block_z && eye >= f64::from(y) {
                        let top = f64::from(y)
                            + f64::from(
                                world
                                    .camera_height(state, p)
                                    .map_err(FluidInteractionError::World)?,
                            );
                        if eye <= top {
                            self.trackers[index].eyes_inside = true;
                        }
                    }
                    self.trackers[index].height =
                        java_max(surface - base_y, self.trackers[index].height);
                    if let Some(i) = current {
                        let mut flow =
                            world.flow(state, p).map_err(FluidInteractionError::World)?;
                        let c = &mut self.currents[i].1;
                        c.height = java_max(self.trackers[index].height, c.height);
                        if c.height < 0.4 {
                            flow = flow.scale(c.height);
                        }
                        c.accumulate(flow);
                    }
                }
            }
        }
        Ok(last.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body the accumulator pushes: the player flag and the velocity it
    /// reads are the only things `apply_to` asks for.
    struct Body {
        player: bool,
        velocity: Vec3,
        added: Vec3,
    }
    impl CurrentTarget for Body {
        type Error = std::convert::Infallible;
        fn is_player_class(&self) -> bool {
            self.player
        }
        fn velocity(&mut self) -> Result<Vec3, Self::Error> {
            Ok(self.velocity)
        }
        fn add_delta_movement(&mut self, delta: Vec3) -> Result<(), Self::Error> {
            self.added = self.added.add_vector(delta);
            Ok(())
        }
    }

    /// `EntityFluidInteraction$CurrentAccumulator.applyTo`: a body that is not
    /// a player takes the direction of the accumulated current, so two samples
    /// of one block push as hard as one; a player takes their average, so the
    /// push grows with the number of blocks and is halved by two of them.
    #[test]
    fn a_current_pushes_a_player_by_its_average_and_anything_else_by_its_direction() {
        let mut acc = CurrentAccumulator::default();
        acc.accumulate(Vec3::new(1.0, 0.0, 0.0));
        let mut mob = Body {
            player: false,
            velocity: Vec3::ZERO,
            added: Vec3::ZERO,
        };
        acc.apply_to(&mut mob, 0.014).unwrap();
        assert!((mob.added.x - 0.014).abs() < 1e-12, "mob: {}", mob.added.x);
        acc.accumulate(Vec3::new(1.0, 0.0, 0.0));
        let mut mob = Body {
            player: false,
            velocity: Vec3::ZERO,
            added: Vec3::ZERO,
        };
        acc.apply_to(&mut mob, 0.014).unwrap();
        assert!(
            (mob.added.x - 0.014).abs() < 1e-12,
            "a second sample does not double a mob's push: {}",
            mob.added.x
        );
        let mut player = Body {
            player: true,
            velocity: Vec3::ZERO,
            added: Vec3::ZERO,
        };
        acc.apply_to(&mut player, 0.014).unwrap();
        assert!(
            (player.added.x - 0.014).abs() < 1e-12,
            "the average of two (1,0,0) samples is (1,0,0): {}",
            player.added.x
        );
    }

    /// The same body with a single weak sample: the average is (0.5,0,0) for a
    /// player, and a nearly still body gets the 0.0045 lift either way.
    #[test]
    fn a_weak_current_on_a_still_body_is_lifted_to_the_minimum() {
        let mut acc = CurrentAccumulator::default();
        // A tenth of a block a tick: 0.0014 once scaled, under the minimum.
        acc.accumulate(Vec3::new(0.1, 0.0, 0.0));
        let mut player = Body {
            player: true,
            velocity: Vec3::ZERO,
            added: Vec3::ZERO,
        };
        acc.apply_to(&mut player, 0.014).unwrap();
        assert!(
            (player.added.x - 0.0045).abs() < 1e-12,
            "{}",
            player.added.x
        );
        // A body that is already moving on x or z takes the small push as it is.
        let mut moving = Body {
            player: true,
            velocity: Vec3::new(0.05, 0.0, 0.0),
            added: Vec3::ZERO,
        };
        acc.apply_to(&mut moving, 0.014).unwrap();
        assert!(
            (moving.added.x - 0.0014).abs() < 1e-12,
            "{}",
            moving.added.x
        );
    }

    /// `applyTo` returns before reading the velocity with no samples, and when
    /// the accumulated current is under the 1e-5 of a squared length.
    #[test]
    fn no_current_and_a_current_under_the_threshold_push_nothing() {
        let mut body = Body {
            player: false,
            velocity: Vec3::ZERO,
            added: Vec3::ZERO,
        };
        CurrentAccumulator::default()
            .apply_to(&mut body, 0.014)
            .unwrap();
        assert_eq!(body.added, Vec3::ZERO);
        let mut acc = CurrentAccumulator::default();
        acc.accumulate(Vec3::new(0.001, 0.0, 0.0));
        acc.apply_to(&mut body, 0.014).unwrap();
        assert_eq!(body.added, Vec3::ZERO);
    }
}
