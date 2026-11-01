//! Resident-world landing, registered block reactions and fall-damage requests.
//! Health, sound/particle playback and network delivery remain event consumers.
use crate::{
    Result,
    collisions::SceneCollisions,
    motion::{MotionEvent, MotionWorld},
};
use horizon_core::{
    geometry::{Aabb, Vec3, VoxelShape},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::{
    block_entity_push::BlockPushWorld,
    block_landing::{
        BlockLandingRegistry, BlockLandingWorld, DamageSourceOwner, FallDefinition, LandingDamage,
        LandingDefinition, LandingEntityKind, LandingEvent, LandingSound,
    },
    entity_obstruction::EntityIdentity,
    fall_damage::FallDamageWorld,
    living_fall::{self, LivingFallWorld},
    living_landing::LivingLandingWorld,
    movement_properties::MovementProperties,
    registry::BlockStateId,
};
use std::{collections::BTreeMap, fs, path::Path};

pub fn load(root: &Path, properties: &MovementProperties<'_>) -> Result<BlockLandingRegistry> {
    let raw: BTreeMap<String, serde_json::Value> =
        serde_json::from_slice(&fs::read(root.join("landing-definitions.json"))?)?;
    let mut definitions = BTreeMap::new();
    for (name, d) in raw {
        let float = |key: &str| -> Result<f32> {
            Ok(f32::from_bits(u32::from_str_radix(
                d[key].as_str().ok_or("missing landing float")?,
                16,
            )?))
        };
        let fall = match d["fall"].as_str().ok_or("missing landing family")? {
            "Block" => FallDefinition::Default,
            "HayBlock" => FallDefinition::Hay,
            "HoneyBlock" => FallDefinition::Honey {
                volume: float("volume")?,
                pitch: float("pitch")?,
            },
            "SlimeBlock" => FallDefinition::Slime,
            "PowderSnowBlock" => FallDefinition::PowderSnow,
            "PointedDripstoneBlock" => FallDefinition::PointedDripstone,
            "FarmlandBlock" => FallDefinition::Farmland {
                base_block: d["base"].as_str().ok_or("missing farmland base")?.into(),
            },
            "TurtleEggBlock" => FallDefinition::TurtleEgg,
            _ => return Err("unsupported landing family".into()),
        };
        definitions.insert(
            name,
            LandingDefinition {
                fall,
                shelf_mushroom_bounce: d["bounce"].as_bool().ok_or("missing bounce flag")?,
            },
        );
    }
    Ok(BlockLandingRegistry::new(properties, &definitions)?)
}
#[derive(Clone, Copy, Debug)]
pub enum SceneSound {
    Block(LandingSound),
    SmallFall,
    BigFall,
}
impl SceneSound {
    /// The sound event of `LivingEntity.getFallDamageSound` for a player, if
    /// this is one of the two fall sounds.
    pub fn player_fall_sound(self) -> Option<&'static str> {
        match self {
            Self::BigFall => Some("minecraft:entity.player.big_fall"),
            Self::SmallFall => Some("minecraft:entity.player.small_fall"),
            Self::Block(_) => None,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::SmallFall => "small",
            Self::BigFall => "big",
            Self::Block(v) => match v {
                LandingSound::HoneySlide => "honey-slide",
                LandingSound::HoneyFall => "honey-fall",
                LandingSound::TurtleEggBreak => "egg-break",
                LandingSound::ShelfMushroomBounce => "mushroom",
            },
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum FallEvent {
    Particles {
        state: BlockStateId,
        position: Vec3,
        count: i32,
        speed: f64,
    },
    HitGround {
        position: Vec3,
        state: BlockStateId,
    },
    Statistic(i32),
    Damage {
        source: LandingDamage,
        amount: f32,
    },
    Sound {
        sound: SceneSound,
        position: Option<BlockPos>,
        volume: f32,
        pitch: f32,
    },
    BlockFallSound,
    Broadcast(u8),
    Write {
        position: BlockPos,
        state: BlockStateId,
        flags: u32,
    },
    Destroy {
        position: BlockPos,
        drops: bool,
    },
    BlockEvent {
        kind: LandingEvent,
        position: BlockPos,
        state: BlockStateId,
        with_entity: bool,
    },
    LevelEvent {
        event: i32,
        position: BlockPos,
        data: i32,
    },
}
impl FallEvent {
    /// Exact-bit diagnostic representation shared by the headless scene probe.
    pub fn trace(self) -> String {
        fn p(v: BlockPos) -> String {
            format!("{},{},{}", v.x, v.y, v.z)
        }
        fn v(v: Vec3) -> String {
            format!(
                "{:016x},{:016x},{:016x}",
                v.x.to_bits(),
                v.y.to_bits(),
                v.z.to_bits()
            )
        }
        match self {
            Self::Particles {
                state,
                position,
                count,
                speed,
            } => format!(
                "particles {} {} {} {:016x}",
                state.0,
                v(position),
                count,
                speed.to_bits()
            ),
            Self::HitGround { position, state } => format!("hit {} {}", v(position), state.0),
            Self::Statistic(n) => format!("stat {n}"),
            Self::Damage { source, amount } => format!(
                "damage {} {:08x}",
                if source == LandingDamage::Fall {
                    "fall"
                } else {
                    "stalagmite"
                },
                amount.to_bits()
            ),
            Self::Sound {
                sound,
                position,
                volume,
                pitch,
            } => format!(
                "sound {} {} {:08x} {:08x}",
                sound.name(),
                position.map_or_else(|| "entity".into(), p),
                volume.to_bits(),
                pitch.to_bits()
            ),
            Self::BlockFallSound => "block-fall-sound".into(),
            Self::Broadcast(event) => format!("broadcast {event}"),
            Self::Write {
                position,
                state,
                flags,
            } => format!("write {} {} {}", p(position), state.0, flags),
            Self::Destroy { position, drops } => format!("destroy {} {}", p(position), drops),
            Self::BlockEvent {
                kind,
                position,
                state,
                with_entity,
            } => format!(
                "event {} {} {} {}",
                if kind == LandingEvent::BlockChange {
                    "change"
                } else {
                    "destroy"
                },
                p(position),
                state.0,
                with_entity
            ),
            Self::LevelEvent {
                event,
                position,
                data,
            } => format!("level-event {} {} {}", event, p(position), data),
        }
    }
}
impl MotionWorld<'_, '_, '_> {
    fn mod_block_fall(
        &self,
        state: BlockStateId,
        position: BlockPos,
        distance: f64,
        damage: f32,
    ) -> f32 {
        let block = self
            .assets
            .registry
            .state(state)
            .map(|s| s.name.as_str())
            .unwrap_or("minecraft:air");
        crate::block_contact::fall(
            crate::block_contact::Contact {
                entity: self.player.entity_id,
                kind: "minecraft:player",
                block,
                state,
                position,
            },
            distance,
            damage,
        )
    }
    fn fall_event(&mut self, event: FallEvent) {
        self.player.events.push(MotionEvent::Fall(event));
    }
    fn player_fall_damage(
        &mut self,
        distance: f64,
        multiplier: f32,
        source: LandingDamage,
        contact: Option<(BlockStateId, BlockPos, f64)>,
    ) -> Result<bool> {
        // Player.causeFallDamage exempts players with the mayfly ability,
        // including creative players who are currently walking.
        if self.player.may_fly {
            if let Some((state, position, original_distance)) = contact {
                self.mod_block_fall(state, position, original_distance, 0.0);
            }
            return Ok(false);
        }
        // Record centimeters before delegating to LivingEntity.
        if distance >= 2.0 || distance.is_nan() {
            let scaled = distance * 100.0;
            let floor = scaled.floor();
            let rounded = (if scaled - floor >= 0.5 {
                floor + 1.0
            } else {
                floor
            }) as i64;
            self.fall_event(FallEvent::Statistic(rounded as i32));
        }
        living_fall::cause_living_fall_damage(
            &mut FallCall {
                world: self,
                source,
                contact,
            },
            distance,
            multiplier,
        )
    }
    /// `Entity.propagateFallToPassengers`: the call on this player, whose
    /// vehicle fell `distance` blocks at `multiplier`, so the player takes
    /// `Player.causeFallDamage` of it.
    pub fn cause_mount_fall(&mut self, distance: f64, multiplier: f32) -> Result<bool> {
        BlockLandingWorld::cause_fall_damage(self, distance, multiplier, LandingDamage::Fall)
    }
}
impl FallDamageWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    type State = BlockStateId;
    fn is_in_water(&mut self) -> Result<bool> {
        Ok(self.player.fluids.was_touching_water)
    }
    fn fall_distance(&self) -> f64 {
        self.player.fall_distance
    }
    fn set_fall_distance(&mut self, value: f64) {
        self.player.fall_distance = value;
    }
    fn fall_on(&mut self, state: BlockStateId, position: BlockPos, distance: f64) -> Result<()> {
        let block = self
            .assets
            .registry
            .state(state)
            .map(|s| s.name.as_str())
            .unwrap_or("minecraft:air");
        let previous = self.player.block_landing.take();
        if !block.starts_with("minecraft:") {
            self.player.block_landing = Some((state, position, distance));
        }
        // Keep template landing rules (hay, honey, slime, etc.) in control.
        let result = self
            .resources
            .landing
            .fall_on(self, state, position, LandingEntityKind::Player, distance)
            .map_err(|e| e.to_string().into());
        let pending = self.player.block_landing.take();
        self.player.block_landing = previous;
        if result.is_ok()
            && let Some((state, position, distance)) = pending
        {
            // Some landing rules never call causeFallDamage.
            self.mod_block_fall(state, position, distance, 0.0);
        }
        result
    }

    fn position(&self) -> Vec3 {
        self.player.position
    }
    fn supporting_block(&self) -> Option<BlockPos> {
        self.player.ground.supporting_block
    }
    fn block_state(&mut self, p: BlockPos) -> Result<BlockStateId> {
        Ok(self.scene.block(p.x, p.y, p.z))
    }
    fn hit_ground(&mut self, position: Vec3, state: BlockStateId) -> Result<()> {
        self.fall_event(FallEvent::HitGround { position, state });
        Ok(())
    }
    fn reset_fall_distance(&mut self) -> Result<()> {
        self.player.fall_distance = 0.0;
        Ok(())
    }
}
impl LivingLandingWorld for MotionWorld<'_, '_, '_> {
    fn update_fluid_interaction(&mut self) -> Result<bool> {
        self.update_fluids()
    }
    fn is_server_level(&self) -> bool {
        true
    }
    fn on_changed_block(&mut self, _: BlockPos) -> Result<()> {
        Ok(())
    } // empty equipment
    fn safe_fall_distance(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:safe_fall_distance")
    }
    fn state_is_air(&mut self, state: BlockStateId) -> Result<bool> {
        Ok(self.scene.is_air(state))
    }
    fn block_position(&mut self) -> Result<BlockPos> {
        Ok(BlockPos::containing(self.player.position))
    }
    fn landing_particles(
        &mut self,
        state: BlockStateId,
        position: Vec3,
        count: i32,
        speed: f64,
    ) -> Result<()> {
        self.fall_event(FallEvent::Particles {
            state,
            position,
            count,
            speed,
        });
        Ok(())
    }
    fn clear_last_climbable_position(&mut self) {
        self.player.last_climbable = None;
    }
}
impl BlockPushWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn collision_shape(&mut self, state: BlockStateId, p: BlockPos) -> Result<VoxelShape> {
        let mut world = SceneCollisions {
            world: self.scene,
            binding: self.collision_binding,
            subject: None,
        };
        self.collision_binding
            .collision_shape(&mut world, state, [p.x, p.y, p.z])
            .map_err(|e| e.to_string().into())
    }
    fn entities(&mut self, bounds: Aabb) -> Result<Vec<EntityIdentity>> {
        Ok(if bounds.intersects(self.player.bounds()) {
            vec![EntityIdentity(0)]
        } else {
            vec![]
        })
    }
    fn entity_bounds(&self, _: EntityIdentity) -> Aabb {
        self.player.bounds()
    }
    fn teleport_relative(&mut self, _: EntityIdentity, movement: Vec3) -> Result<()> {
        self.player.position = Vec3::new(
            self.player.position.x + movement.x,
            self.player.position.y + movement.y,
            self.player.position.z + movement.z,
        );
        Ok(())
    }
}
impl BlockLandingWorld for MotionWorld<'_, '_, '_> {
    type DamageSource = LandingDamage;
    type Sound = SceneSound;
    fn is_server_level(&self) -> bool {
        // A remote client's world is a client level: it never tramples,
        // the server's block change and teleport do it (FarmlandBlock.fallOn
        // and TurtleEggBlock.fallOn check `level instanceof ServerLevel`).
        !self.player.client_side
    }
    fn is_client_side(&mut self) -> Result<bool> {
        Ok(self.player.client_side)
    }
    fn random_float(&mut self) -> Result<f32> {
        Ok(self.scene.random.next_float())
    }
    fn random_int(&mut self, bound: i32) -> Result<i32> {
        Ok(self.scene.random.next_int_bound(bound)?)
    }
    fn under_spawn_protection(&mut self, _: BlockPos) -> Result<bool> {
        Ok(false)
    }
    fn mob_griefing(&mut self) -> Result<bool> {
        Ok(true)
    }
    fn entity_dimensions(&self) -> (f32, f32) {
        (self.player.dimensions.width, self.player.dimensions.height)
    }
    fn suppressing_bounce(&mut self) -> Result<bool> {
        Ok(self.player.shift_key_down)
    }
    fn damage_source(
        &mut self,
        _: DamageSourceOwner,
        kind: LandingDamage,
    ) -> Result<LandingDamage> {
        Ok(kind)
    }
    fn cause_fall_damage(
        &mut self,
        distance: f64,
        multiplier: f32,
        source: LandingDamage,
    ) -> Result<bool> {
        let contact = self.player.block_landing.take();
        self.player_fall_damage(distance, multiplier, source, contact)
    }
    fn known_sound(&self, s: LandingSound) -> SceneSound {
        SceneSound::Block(s)
    }
    fn living_fall_sound(&mut self, big: bool) -> Result<SceneSound> {
        Ok(if big {
            SceneSound::BigFall
        } else {
            SceneSound::SmallFall
        })
    }
    fn entity_sound(&mut self, sound: SceneSound, volume: f32, pitch: f32) -> Result<()> {
        self.fall_event(FallEvent::Sound {
            sound,
            position: None,
            volume,
            pitch,
        });
        Ok(())
    }
    fn block_sound(
        &mut self,
        position: BlockPos,
        sound: SceneSound,
        volume: f32,
        pitch: f32,
    ) -> Result<()> {
        self.fall_event(FallEvent::Sound {
            sound,
            position: Some(position),
            volume,
            pitch,
        });
        Ok(())
    }
    fn broadcast_entity_event(&mut self, event: u8) -> Result<()> {
        self.fall_event(FallEvent::Broadcast(event));
        Ok(())
    }
    fn destroy_block(&mut self, position: BlockPos, drops: bool) -> Result<bool> {
        let air = self
            .assets
            .registry
            .default_state("minecraft:air")
            .ok_or("missing air")?;
        self.scene.set(position.x, position.y, position.z, air)?;
        self.fall_event(FallEvent::Destroy { position, drops });
        Ok(true)
    }
    fn set_block(&mut self, position: BlockPos, state: BlockStateId, flags: u32) -> Result<bool> {
        self.scene.set(position.x, position.y, position.z, state)?;
        self.fall_event(FallEvent::Write {
            position,
            state,
            flags,
        });
        Ok(true)
    }
    fn block_event(
        &mut self,
        kind: LandingEvent,
        position: BlockPos,
        state: BlockStateId,
        with_entity: bool,
    ) -> Result<()> {
        self.fall_event(FallEvent::BlockEvent {
            kind,
            position,
            state,
            with_entity,
        });
        Ok(())
    }
    fn level_event(&mut self, event: i32, position: BlockPos, data: i32) -> Result<()> {
        self.fall_event(FallEvent::LevelEvent {
            event,
            position,
            data,
        });
        Ok(())
    }
}
struct FallCall<'a, 'p, 'w, 'r> {
    world: &'a mut MotionWorld<'p, 'w, 'r>,
    source: LandingDamage,
    contact: Option<(BlockStateId, BlockPos, f64)>,
}
impl LivingFallWorld for FallCall<'_, '_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    type Sound = SceneSound;
    fn impact_position(&self) -> Option<Vec3> {
        self.world.player.impulse.impact_position
    }
    fn y(&self) -> f64 {
        self.world.player.position.y
    }
    fn reset_impulse(&mut self) -> Result<()> {
        self.world.player.impulse.reset();
        Ok(())
    }
    fn try_reset_impulse(&mut self) -> Result<()> {
        self.world.player.impulse.try_reset();
        Ok(())
    }
    fn immune_to_fall_damage(&mut self) -> Result<bool> {
        Ok(false)
    } // player type
    fn propagate_fall_to_passengers(&mut self, _: f64, _: f32) -> Result<()> {
        Ok(())
    } // no passengers
    fn safe_fall_distance(&mut self) -> Result<f64> {
        self.world.player.attribute("minecraft:safe_fall_distance")
    }
    fn fall_damage_multiplier(&mut self) -> Result<f64> {
        self.world
            .player
            .attribute("minecraft:fall_damage_multiplier")
    }
    fn modify_fall_damage(&mut self, _distance: f64, damage: f32) -> Result<f32> {
        Ok(self.contact.map_or(damage, |(state, position, distance)| {
            self.world.mod_block_fall(state, position, distance, damage)
        }))
    }
    fn fall_sound(&mut self, big: bool) -> Result<SceneSound> {
        self.world.living_fall_sound(big)
    }
    fn play_fall_sound(&mut self, sound: SceneSound) -> Result<()> {
        self.world.entity_sound(sound, 1.0, 1.0)
    }
    fn play_block_fall_sound(&mut self) -> Result<()> {
        self.world.fall_event(FallEvent::BlockFallSound);
        Ok(())
    }
    fn hurt(&mut self, amount: f32) -> Result<()> {
        self.world.fall_event(FallEvent::Damage {
            source: self.source,
            amount,
        });
        let resources = &self.world.resources.health;
        self.world.player.hurt(
            resources,
            self.world.scene.health_rules,
            resources.landing_source(self.source),
            amount,
        )?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fall_sounds_map_to_player_events() {
        assert_eq!(
            SceneSound::BigFall.player_fall_sound(),
            Some("minecraft:entity.player.big_fall")
        );
        assert_eq!(
            SceneSound::SmallFall.player_fall_sound(),
            Some("minecraft:entity.player.small_fall")
        );
        assert_eq!(
            SceneSound::Block(LandingSound::HoneyFall).player_fall_sound(),
            None
        );
    }
}
