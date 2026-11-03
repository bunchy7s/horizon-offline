//! PURPOSE: a mob landing on a block runs that block's registered `Block.fallOn` rule (hay, honey, slime, powder snow, bed, stalagmite, farmland, turtle egg), as the player already does in landing.rs.
//! WHY: mobs used to take the whole fall whatever block they landed on (a pig falling 8 blocks onto powder snow lost 6 health; vanilla loses none).
//! REFS: Block.fallOn and its overrides (HayBlock, HoneyBlock, SlimeBlock, PowderSnowBlock, BedBlock, PointedDripstoneBlock, FarmlandBlock, TurtleEggBlock); crate::landing (the player's adapter); horizon_world::block_landing.
//! LOGS: none; the block writes and sounds go out as MobEvents.
//! READS-BY: mobs::living::Mob::fall_on_block.
//! ENDS-WHEN: the landing registry covers a mob's whole fall and this adapter is merged into crate::landing.
use super::{DamageKind, Mob, MobEvent, MobWorld};
use crate::collisions::SceneCollisions;
use horizon_core::{
    geometry::{Aabb, Vec3, VoxelShape},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::{
    block_entity_push::BlockPushWorld,
    block_landing::{
        BlockLandingWorld, DamageSourceOwner, LandingDamage, LandingEvent, LandingSound,
    },
    entity_obstruction::EntityIdentity,
    registry::BlockStateId,
};

/// A mob mid-landing: the block under it is at `pos`, the fall was `distance`.
pub(super) struct MobLanding<'a, 'w> {
    pub mob: &'a mut Mob,
    pub w: &'a MobWorld<'w>,
    pub events: &'a mut Vec<MobEvent>,
    pub pos: [i32; 3],
}
impl MobLanding<'_, '_> {
    fn at(&self) -> [f64; 3] {
        let p = self.mob.position;
        [p.x, p.y, p.z]
    }
}
impl BlockPushWorld for MobLanding<'_, '_> {
    type Error = String;
    fn collision_shape(&mut self, state: BlockStateId, p: BlockPos) -> Result<VoxelShape, String> {
        let mut scene = SceneCollisions {
            world: self.w.world,
            binding: self.w.collision,
            subject: None,
        };
        self.w
            .collision
            .collision_shape(&mut scene, state, [p.x, p.y, p.z])
            .map_err(|e| e.to_string())
    }
    // Only the landing mob is lifted off a block that grows under it.
    fn entities(&mut self, bounds: Aabb) -> Result<Vec<EntityIdentity>, String> {
        Ok(if bounds.intersects(self.mob.bounds()) {
            vec![EntityIdentity(0)]
        } else {
            vec![]
        })
    }
    fn entity_bounds(&self, _: EntityIdentity) -> Aabb {
        self.mob.bounds()
    }
    fn teleport_relative(&mut self, _: EntityIdentity, m: Vec3) -> Result<(), String> {
        self.mob.position = Vec3::new(
            self.mob.position.x + m.x,
            self.mob.position.y + m.y,
            self.mob.position.z + m.z,
        );
        Ok(())
    }
}
impl BlockLandingWorld for MobLanding<'_, '_> {
    type DamageSource = DamageKind;
    /// The sound event, or none for a sound a mob's landing never makes.
    type Sound = Option<&'static str>;
    fn is_server_level(&self) -> bool {
        true
    }
    fn is_client_side(&mut self) -> Result<bool, String> {
        Ok(false)
    }
    fn random_float(&mut self) -> Result<f32, String> {
        Ok(self.mob.random.next_float())
    }
    fn random_int(&mut self, bound: i32) -> Result<i32, String> {
        self.mob
            .random
            .next_int_bound(bound)
            .map_err(|e| e.to_string())
    }
    fn under_spawn_protection(&mut self, _: BlockPos) -> Result<bool, String> {
        Ok(false)
    }
    fn mob_griefing(&mut self) -> Result<bool, String> {
        Ok(self.w.mob_griefing)
    }
    fn entity_dimensions(&self) -> (f32, f32) {
        (self.mob.width(), self.mob.height())
    }
    // A mob never sneaks, so nothing suppresses a slime block's bounce.
    fn suppressing_bounce(&mut self) -> Result<bool, String> {
        Ok(false)
    }
    fn damage_source(
        &mut self,
        _: DamageSourceOwner,
        kind: LandingDamage,
    ) -> Result<DamageKind, String> {
        Ok(match kind {
            LandingDamage::Fall => DamageKind::Fall,
            LandingDamage::Stalagmite => DamageKind::Stalagmite,
        })
    }
    fn cause_fall_damage(
        &mut self,
        distance: f64,
        multiplier: f32,
        source: DamageKind,
    ) -> Result<bool, String> {
        let contact = Some((self.w, self.pos));
        Ok(self
            .mob
            .cause_fall_damage_at(distance, multiplier, self.events, contact, source))
    }
    fn known_sound(&self, sound: LandingSound) -> Option<&'static str> {
        match sound {
            LandingSound::HoneySlide => Some("block.honey_block.slide"),
            LandingSound::HoneyFall => Some("block.honey_block.fall"),
            LandingSound::TurtleEggBreak => Some("block.turtle_egg.break"),
            LandingSound::ShelfMushroomBounce => None,
        }
    }
    // LivingEntity.getFallDamageSound: Monster.getFallSounds is the hostile pair.
    fn living_fall_sound(&mut self, big: bool) -> Result<Option<&'static str>, String> {
        let monster = super::ai::is_monster(self.mob.species());
        Ok(Some(match (monster, big) {
            (true, true) => "entity.hostile.big_fall",
            (true, false) => "entity.hostile.small_fall",
            (false, true) => "entity.generic.big_fall",
            (false, false) => "entity.generic.small_fall",
        }))
    }
    fn entity_sound(
        &mut self,
        sound: Option<&'static str>,
        volume: f32,
        pitch: f32,
    ) -> Result<(), String> {
        if let Some(name) = sound
            && !self.mob.silent
        {
            let at = self.at();
            self.events.push(MobEvent::Sound(name, at, volume, pitch));
        }
        Ok(())
    }
    fn block_sound(
        &mut self,
        position: BlockPos,
        sound: Option<&'static str>,
        volume: f32,
        pitch: f32,
    ) -> Result<(), String> {
        if let Some(name) = sound {
            let at = [
                f64::from(position.x) + 0.5,
                f64::from(position.y) + 0.5,
                f64::from(position.z) + 0.5,
            ];
            self.events.push(MobEvent::Sound(name, at, volume, pitch));
        }
        Ok(())
    }
    fn broadcast_entity_event(&mut self, event: u8) -> Result<(), String> {
        let at = self.at();
        self.events.push(MobEvent::EntityEvent {
            mob: self.mob.id,
            event: event as i8,
            at,
        });
        Ok(())
    }
    fn destroy_block(&mut self, position: BlockPos, _drops: bool) -> Result<bool, String> {
        let air = self
            .w
            .registry
            .default_state("minecraft:air")
            .ok_or("missing air")?;
        self.set_block(position, air, 3)
    }
    fn set_block(
        &mut self,
        position: BlockPos,
        state: BlockStateId,
        _: u32,
    ) -> Result<bool, String> {
        self.events.push(MobEvent::SetBlock {
            pos: [position.x, position.y, position.z],
            state,
        });
        Ok(true)
    }
    // Neighbour updates and the break particles ride on the host's SetBlock.
    fn block_event(
        &mut self,
        _: LandingEvent,
        _: BlockPos,
        _: BlockStateId,
        _: bool,
    ) -> Result<(), String> {
        Ok(())
    }
    fn level_event(&mut self, _: i32, _: BlockPos, _: i32) -> Result<(), String> {
        Ok(())
    }
}
