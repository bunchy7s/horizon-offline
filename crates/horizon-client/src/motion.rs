//! Stored movement state for the resident scene. This binds the validated air,
//! jump, collision, support, restitution and stored health bindings, and
//! powder snow's block-inside effects and freezing. Entity lifecycle, other
//! block-inside effects and audiovisual event consumers remain separate work.
use crate::{Result, assets::Assets, collisions::SceneCollisions, world::World};
use horizon_core::{
    geometry::{Aabb, Axis, Direction, Vec3, VoxelShape, traverse_blocks},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::{
    attribute_map::ValueKind,
    attributes::{Modifier, Operation, entity::Catalog, live::LiveAttributes},
    block_collision::BlockCollisionRegistry,
    block_landing::BlockLandingRegistry,
    climbing::{self, ClimbingRegistry, ClimbingWorld},
    collision_context::{CollisionContext, CollisionEntity, CollisionItem},
    edge_backoff::{self, EdgeBackoffInput, EdgeBackoffWorld},
    entity_move::{self, CollisionFlags, EntityMoveWorld, MovementHistory, MoverType},
    entity_obstruction::EntityIdentity,
    ground_support::{GroundState, find_supporting_block},
    living_air::{self, LivingAirWorld},
    living_fall::ImpulseContext,
    living_motion::{self, LivingMotionWorld},
    movement_emission::{
        self as emission, MovementDistances, MovementEmission, MovementEmissionWorld, MovementEvent,
    },
    movement_properties::{
        BoundMovementProperties, MovementBlockProvider, MovementProperties, MovementTags,
        RegistryMovementWorld,
    },
    movement_restitution::{self, RestitutionInput, RestitutionWorld},
    movement_statistics::{self, MovementStat, MovementStatisticsWorld},
    movement_support::{self, MovementLocation},
    registry::{BlockStateId, BlockTableFile},
    world_movement::{self, MovementRequest},
    worldgen::tags::TagDefinitions,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
    sync::Arc,
};

pub(crate) mod environment;
mod fluids;
pub use fluids::{
    BubbleColumn, FluidResources, FluidTags, LAVA, PlayerFluids, WATER, bubble_column_at,
    bubble_column_push,
};

pub struct MotionResources {
    pub attributes: Arc<Catalog>,
    pub blocks: BoundMovementProperties,
    pub climbable: BTreeSet<String>,
    /// `BlockTags.SOUL_SPEED_BLOCKS`: what Soul Speed's effects look for.
    pub soul_speed_blocks: BTreeSet<String>,
    /// `LivingEntity.onClimbable` tags plus open trapdoors above a ladder.
    pub climbing: ClimbingRegistry<'static>,
    pub fall_reset: BTreeSet<String>,
    /// `BlockTags.INSIDE_STEP_SOUND_BLOCKS`.
    pub inside_step_sound: BTreeSet<String>,
    /// `BlockTags.COMBINATION_STEP_SOUND_BLOCKS`.
    pub combination_step_sound: BTreeSet<String>,
    /// `BlockTags.CRYSTAL_SOUND_BLOCKS`: blocks that chime under footsteps.
    pub crystal_sound: BTreeSet<String>,
    pub landing: BlockLandingRegistry,
    pub health: crate::health::HealthResources,
    pub fluids: FluidResources,
}
impl MotionResources {
    /// [`Self::load_with_facts`] with the block facts decoded here.
    pub fn load(root: &Path, assets: &Assets) -> Result<Self> {
        Self::load_with_facts(
            root,
            assets,
            &crate::blocks::BlockFacts::load(root, assets.registry)?,
        )
    }
    /// The motion tables, with the block facts the game already decoded
    /// (decoding `gameplay-facts.json` again costs a second or more and, with
    /// a big modpack, hundreds of megabytes while it runs).
    pub fn load_with_facts(
        root: &Path,
        assets: &Assets,
        facts: &crate::blocks::BlockFacts,
    ) -> Result<Self> {
        let attributes = Arc::new(Catalog::from_json(&serde_json::from_slice(&fs::read(
            root.join("attributes.json"),
        )?)?)?);
        let file = BlockTableFile::open(&root.join("movement-properties.nbt"), assets.registry)?;
        let properties = MovementProperties::from_table(file.table("movement")?)?;
        let bundle: BTreeMap<String, String> =
            serde_json::from_slice(&fs::read(root.join("block-tags.json"))?)?;
        let mut definitions = TagDefinitions::default();
        for (name, text) in bundle {
            definitions.apply_json(&name, "vanilla", &text)?;
        }
        let built = definitions
            .build(|name, _| assets.registry.default_state(name).map(|_| name.to_owned()))?;
        if !built.missing.is_empty() {
            return Err("unresolved motion block tags".into());
        }
        let tag = |name: &str| -> Result<BTreeSet<String>> {
            Ok(built
                .tags
                .get(name)
                .ok_or_else(|| format!("missing motion tag {name}"))?
                .iter()
                .cloned()
                .collect())
        };
        let climbing = ClimbingRegistry::new(
            assets.registry,
            &tag("minecraft:trapdoors")?,
            &tag("minecraft:climbable")?,
            &tag("minecraft:can_glide_through")?,
        )?;
        Ok(Self {
            landing: crate::landing::load(root, &properties)?,
            health: crate::health::HealthResources::load(root)?,
            fluids: FluidResources::new(assets, facts)?,
            attributes,
            blocks: properties.bind(&MovementTags {
                fences: tag("minecraft:fences")?,
                walls: tag("minecraft:walls")?,
                suppresses_bounce: tag("minecraft:suppresses_bounce")?,
            })?,
            climbable: tag("minecraft:climbable")?,
            soul_speed_blocks: tag("minecraft:soul_speed_blocks")?,
            climbing,
            fall_reset: tag("minecraft:fall_damage_resetting")?,
            inside_step_sound: tag("minecraft:inside_step_sound_blocks")?,
            combination_step_sound: tag("minecraft:combination_step_sound_blocks")?,
            crystal_sound: tag("minecraft:crystal_sound_blocks")?,
        })
    }
}

/// What the `location_changed` enchantment effects (Soul Speed) need to know
/// between ticks.
#[derive(Clone, Debug, Default)]
pub struct LocationEffects {
    /// `MovementAffectedByPredicate`: the block under the player
    /// (`getBlockPosBelowThatAffectsMyMovement`) is in `#minecraft:soul_speed_blocks`.
    pub on_soul_speed_block: bool,
    /// `LivingEntity.lastPos`: the block the player was in last tick.
    pub block: Option<BlockPos>,
    /// A landing ran `onChangedBlock` (`LivingEntity.checkFallDamage`).
    pub landed: bool,
    /// The Soul Speed level of the boots seen last tick (0: none).
    pub boots_level: u32,
    /// The Soul Speed level whose attribute modifiers are on the player (0: none).
    pub applied_level: u32,
}

/// Ordered requests and phase records for world/audio/network consumers. Landing
/// hurt dispatch updates stored health synchronously before these are drained.
#[derive(Clone, Copy, Debug)]
pub enum MotionEvent {
    Waypoint {
        receiver: bool,
        enabled: bool,
    },
    Health(crate::health::HealthEvent),
    /// `LivingEntity.handleDamageEvent`: the hurt sound the player hears.
    HurtSound(&'static str),
    Fall(crate::landing::FallEvent),
    Recorded {
        mover: MoverType,
        movement: Vec3,
    },
    Landing {
        dy: f64,
        grounded: bool,
        state: BlockStateId,
        position: BlockPos,
    },
    /// `Entity.walkingStepSound` on this block: `Player.playStepSound`
    /// mutes it (`playMuffledStepSound`) under a swim sound in water or
    /// under a combination block's own step.
    Emission {
        position: BlockPos,
        state: BlockStateId,
        muffled: bool,
    },
    /// `Entity.playAmethystStepSound`: `block.amethyst_block.chime`.
    Chime {
        volume: f32,
        pitch: f32,
    },
    /// `Entity.waterSwimSound`: `entity.player.swim`.
    Swim {
        volume: f32,
        pitch: f32,
    },
    Bounce {
        position: BlockPos,
        state: BlockStateId,
        fall_distance: f64,
    },
    /// `PowderSnowBlock.entityInside`'s server particle: one snowflake at
    /// `position` with `velocity` (`RandomizationType.ALTERNATIVE`).
    Snowflake {
        position: Vec3,
        velocity: Vec3,
    },
    /// `ServerPlayer.onInsideBlock`: the enter_block criterion trigger.
    EnterBlock {
        state: BlockStateId,
    },
    /// `NetherPortalBlock.entityInside`: the player's box swept through a
    /// nether portal block.
    InsidePortal {
        pos: BlockPos,
    },
    /// `LayeredCauldronBlock.handleEntityOnFireInside`: a burning player in
    /// a water or powder snow cauldron; the server lowers its fill level.
    CauldronExtinguish {
        pos: BlockPos,
    },
    /// `WitherRoseBlock.entityInside`: the player's box swept through a
    /// wither rose; the server adds its Wither unless peaceful.
    InsideWitherRose,
    /// `ServerPlayer.checkFallDamage` after a mace smash: block particles
    /// from the landing block, more the further the fall.
    SmashLanding {
        state: BlockStateId,
        position: BlockPos,
        fall_distance: f64,
    },
    /// `Entity.doWaterSplashEffect`: the player's splash (or high-speed
    /// splash) sound at its position...
    Splash {
        high_speed: bool,
        volume: f32,
        pitch: f32,
    },
    /// ...its BUBBLE (else SPLASH) particles...
    SplashParticle {
        bubble: bool,
        position: Vec3,
        velocity: Vec3,
    },
    /// ...and the SPLASH game event.
    SplashEvent,
    /// `Entity.sendBubbleColumnParticles`: a SPLASH (or BUBBLE) particle
    /// over the top of a bubble column the player rides.
    BubbleColumnParticle {
        bubble: bool,
        position: Vec3,
    },
    /// `HoneyBlock.maybeDoSlideEffects` for a player sliding down the side of
    /// the honey block at `pos`: its slide sound, and the server's entity
    /// event 53 (the five block particles at the player).
    HoneySlide {
        pos: BlockPos,
        sound: bool,
        particles: bool,
    },
    /// Entity event 67 (`LivingEntity.makeDrownParticles`): eight bubbles
    /// around a drowning player.
    Drown,
}
pub struct PlayerMotion {
    pub swimming: bool,
    pub fire_ticks: i32,
    pub air: i32,
    pub respiration: i32,

    pub position: Vec3,
    pub velocity: Vec3,
    pub ground: GroundState,
    pub collision: CollisionFlags,
    pub history: MovementHistory,
    pub fall_distance: f64,
    pub stuck_multiplier: Vec3,
    /// `Entity.pistonDeltas` and `pistonDeltasGameTime`: how far a moving
    /// block has pushed this entity this tick (0.51 at most on any axis).
    pub piston: horizon_world::piston_movement::PistonMovement,
    pub yaw: f32,
    pub head_yaw: f32,
    pub jumping: bool,
    pub no_jump_delay: i32,
    pub jump_trigger_time: i32,
    /// Abilities.mayfly / flying (creative flight).
    pub may_fly: bool,
    /// Abilities.invulnerable (creative): only damage bypassing invulnerability hurts.
    pub invulnerable: bool,
    /// Spectator mode: no physics, always flying (`Player.tick`).
    pub spectator: bool,
    pub flying: bool,
    /// Client-side `Abilities.flyingSpeed`, adjusted by the spectator scroll wheel.
    pub flying_speed: f32,
    /// The jump key last tick (LocalPlayer.aiStep's wasJumping).
    pub was_jumping: bool,
    /// `LocalPlayer.autoJumpEnabled` (the Auto-Jump option) and
    /// `autoJumpTime`; `was_auto_jump` is this tick's `wasAutoJump`.
    pub auto_jump_enabled: bool,
    pub auto_jump_time: i32,
    pub was_auto_jump: bool,
    /// `input.getMoveVector()` this tick (before `modifyInput`).
    pub move_vector: [f32; 2],
    /// A usable glider (elytra) is worn; the game sets it each tick.
    pub glider: bool,
    /// The view pitch (xRot), for gliding.
    pub pitch: f32,
    /// Gliding (shared flag 7) and LivingEntity.fallFlyTicks.
    pub fall_flying: bool,
    /// Asleep in a bed (`LivingEntity.isSleeping`): the sleeping pose.
    pub sleeping: bool,
    pub fall_fly_ticks: i32,
    /// The server saw the glide end last tick (`updateFallFlying`'s `canGlide`
    /// failed): the cleared flag reaches the client at its next tick.
    pub glide_ending: bool,
    /// Glider durability to take this tick (updateFallFlying).
    pub glider_wear: u32,
    /// Soul Speed's `location_changed` bookkeeping.
    pub location_effects: LocationEffects,
    /// `LivingEntity.autoSpinAttackTicks` and `autoSpinAttackDmg`: Riptide's
    /// spin. The damage stays until the game has dealt the hit.
    pub auto_spin_attack_ticks: i32,
    pub auto_spin_attack_dmg: f32,
    /// The living entities the game put near a spinning player (id, box).
    pub spin_targets: Vec<(u32, Aabb)>,
    /// The entity the spin touched this tick (`doAutoAttackOnTouch`).
    pub spin_hit: Option<u32>,
    pub sprinting: bool,
    pub sprint_input: crate::sprint::SprintState,
    pub shift_key_down: bool,
    pub local_crouching: bool,
    pub pose: crate::pose::Pose,
    pub dimensions: crate::pose::Dimensions,
    pub speed: f32,
    /// `LocalPlayer.itemUseSpeedMultiplier` while using an item (not riding).
    pub item_use_speed: Option<f32>,
    /// `LocalPlayer.isSlowDueToUsingItem`: an item in use whose `use_effects`
    /// do not allow sprinting.
    pub item_use_slows_sprint: bool,
    pub last_climbable: Option<BlockPos>,
    /// `Entity.moveDist`, `flyDist` and `nextStep`: footsteps come once per
    /// metre walked, not once per move.
    pub steps: MovementDistances,
    /// `Entity.crystalSoundIntensity` and `lastCrystalSoundPlayTick`: the
    /// amethyst chime grows with steps taken close together.
    pub crystal_sound_intensity: f32,
    pub last_crystal_sound_play_tick: i32,
    pub needs_sync: bool,
    pub sync_position: bool,
    /// Moved by an ender pearl (`ServerPlayer.teleport`): a server sends the
    /// position packet even when the player lands where they stood. The
    /// server takes it at the end of a guest's turn.
    pub teleported: bool,
    pub events: Vec<MotionEvent>,
    pub attributes: LiveAttributes<Box<dyn FnMut() -> i32>>,
    pub impulse: ImpulseContext,
    /// `ServerPlayer.spawnExtraParticlesOnFall`: a mace smash on a grounded
    /// target makes the next landing throw up the block below.
    pub spawn_extra_particles_on_fall: bool,
    pub health: crate::health::PlayerHealth,
    /// Status effects and armor protection seen by movement and damage.
    pub status: crate::effects::StatusFlags,
    /// Water and lava around the player (`Entity.fluidInteraction`).
    pub fluids: PlayerFluids,
    /// `Entity.isInPowderSnow` / `wasInPowderSnow`.
    pub in_powder_snow: bool,
    pub was_in_powder_snow: bool,
    /// `Entity.DATA_TICKS_FROZEN`.
    pub ticks_frozen: i32,
    /// Leather boots on the feet (`PowderSnowBlock.canEntityWalkOnPowderSnow`).
    pub leather_boots: bool,
    /// An armor piece in `#freeze_immune_wearables` (`LivingEntity.canFreeze`).
    pub freeze_immune: bool,
    /// A worn enchantment's `damage_immunity` to `#burn_from_stepping` (frost walker).
    pub burn_from_stepping_immune: bool,
    /// `Entity.oldPosition`: where this tick started.
    pub old_position: Vec3,
    /// The server's copy of a player another client moves (a LAN guest), while
    /// it travels on the server: `isLocalInstanceAuthoritative` is false, so
    /// its moves check no fall damage.
    pub server_copy: bool,
    /// The span of `events` a wall hit on the server's copy pushed (sound and
    /// damage): `server_travel` keeps them while it drops the simulated moves.
    pub wall_hit_events: Option<std::ops::Range<usize>>,
    /// A client's copy of its own player (`Level.isClientSide`): only the
    /// server hurts it (`Entity.hurt` does nothing on a client) and ticks its
    /// food (`ServerPlayer.tick`). Its hurt animation comes from the server.
    pub client_side: bool,
    /// `Entity.applyEffectsFromBlocks` runs after travel (always in the game;
    /// differential fixtures turn it off where their reference does).
    pub block_effects: bool,
    /// The powder snow blocks this tick's movement went into: the game
    /// destroys them if the player is burning, then puts the fire out.
    pub powder_snow_entered: Vec<BlockPos>,
    /// The last hurts taken (`hurt_blocking`), newest last, for tests and
    /// the control socket's `hurtlog`.
    pub hurt_log: Vec<crate::health::HurtRecord>,
    /// The last melee attacks made on mobs, newest last.
    pub attack_log: Vec<crate::health::AttackRecord>,
    /// The level's world border, whose walls stop the player's movement
    /// from inside (the game keeps it current; None: no border).
    pub border: Option<horizon_core::geometry::border::BorderGeometry>,
}
fn attribute_error(error: impl std::fmt::Debug) -> io::Error {
    io::Error::other(format!("movement attribute: {error:?}"))
}
impl PlayerMotion {
    /// `Player.isSweepAttack`, using the resident player's known horizontal velocity.
    pub fn is_sweep_attack(&self, strength: f32, critical: bool, sword: bool) -> bool {
        let movement = self.velocity;
        let limit = f64::from(self.speed) * 2.5;
        strength > 0.9
            && !critical
            && !self.sprinting
            && self.ground.on_ground
            && movement.x * movement.x + movement.z * movement.z < limit * limit
            && sword
    }
    /// Restore login state once; a later respawn starts with fresh health and food.
    pub fn load_saved_state_once(
        &mut self,
        pending: &mut bool,
        level: &crate::save::Level,
    ) -> Result<()> {
        if std::mem::take(pending) {
            let extra = level.player_extra.as_ref();
            self.fire_ticks = extra
                .and_then(|t| crate::save::number(t.get("Fire")))
                .unwrap_or(-20) as i32;
            self.air = extra
                .and_then(|t| crate::save::number(t.get("Air")))
                .unwrap_or(300) as i32;
            self.fall_distance = level.fall_distance;
            self.set_health(level.health, false)?;
            // LivingEntity.readAdditionalSaveData: internalSetAbsorptionAmount.
            self.internal_set_absorption(level.absorption)?;
            self.health.food.food = level.food;
            self.health.food.saturation = level.saturation;
            self.flying = level.flying;
        }
        Ok(())
    }
    pub fn set_remaining_fire_ticks(&mut self, ticks: i32) {
        self.fire_ticks = if self.invulnerable {
            ticks.min(1)
        } else {
            ticks
        };
    }
    pub fn ignite_for_ticks(&mut self, ticks: i32) -> Result<()> {
        let scaled = (f64::from(ticks) * self.attribute("minecraft:burning_time")?).ceil() as i32;
        self.set_remaining_fire_ticks(self.fire_ticks.max(scaled));
        self.ticks_frozen = 0;
        Ok(())
    }

    pub fn new(resources: &MotionResources, position: Vec3) -> Result<Self> {
        // Process-owned identities determine native collection callback order.
        // Differential fixtures can replay a reference process's identities.
        let hashes: Vec<_> = resources
            .attributes
            .attributes()
            .enumerate()
            .map(|(i, _)| i as i32)
            .collect();
        let mut identity = 0i32;
        let next: Box<dyn FnMut() -> i32> = Box::new(move || {
            identity = identity.wrapping_add(1);
            identity
        });
        Self::with_attribute_identities(resources, position, &hashes, next)
    }
    /// Explicit identity allocation for deterministic reference replay.
    pub fn with_attribute_identities(
        resources: &MotionResources,
        position: Vec3,
        hashes: &[i32],
        next: Box<dyn FnMut() -> i32>,
    ) -> Result<Self> {
        let mut attributes = LiveAttributes::new(
            resources.attributes.clone(),
            "minecraft:player",
            hashes,
            next,
        )
        .map_err(attribute_error)?;
        Ok(Self {
            health: crate::health::PlayerHealth::new(&resources.health, &mut attributes)?,
            status: crate::effects::StatusFlags::default(),
            impulse: ImpulseContext::default(),
            spawn_extra_particles_on_fall: false,
            position,
            velocity: Vec3::ZERO,
            ground: GroundState::default(),
            collision: CollisionFlags::default(),
            history: MovementHistory::default(),
            fall_distance: 0.0,
            stuck_multiplier: Vec3::ZERO,
            piston: Default::default(),
            yaw: 0.0,
            head_yaw: 0.0,
            jumping: false,
            no_jump_delay: 0,
            jump_trigger_time: 0,
            may_fly: false,
            invulnerable: false,
            spectator: false,
            flying: false,
            flying_speed: 0.05,
            was_jumping: false,
            auto_jump_enabled: false,
            auto_jump_time: 0,
            was_auto_jump: false,
            move_vector: [0.0, 0.0],
            glider: false,
            pitch: 0.0,
            fall_flying: false,
            sleeping: false,
            fall_fly_ticks: 0,
            glide_ending: false,
            glider_wear: 0,
            location_effects: LocationEffects::default(),
            auto_spin_attack_ticks: 0,
            auto_spin_attack_dmg: 0.0,
            spin_targets: Vec::new(),
            spin_hit: None,
            sprinting: false,
            sprint_input: crate::sprint::SprintState::default(),
            shift_key_down: false,
            local_crouching: false,
            pose: crate::pose::Pose::Standing,
            dimensions: crate::pose::Pose::Standing.dimensions(1.0),
            swimming: false,
            fire_ticks: -20,
            air: 300,
            respiration: 0,
            speed: 0.0,
            item_use_speed: None,
            item_use_slows_sprint: false,
            last_climbable: None,
            steps: MovementDistances {
                next_step: 1.0,
                ..MovementDistances::default()
            },
            crystal_sound_intensity: 0.0,
            last_crystal_sound_play_tick: 0,
            needs_sync: false,
            sync_position: false,
            teleported: false,
            events: Vec::new(),
            attributes,
            fluids: PlayerFluids::default(),
            in_powder_snow: false,
            was_in_powder_snow: false,
            ticks_frozen: 0,
            leather_boots: false,
            freeze_immune: false,
            burn_from_stepping_immune: false,
            old_position: position,
            server_copy: false,
            wall_hit_events: None,
            client_side: false,
            block_effects: true,
            powder_snow_entered: Vec::new(),
            hurt_log: Vec::new(),
            attack_log: Vec::new(),
            border: None,
        })
    }
    /// LivingEntity.aiStep: speeds too small to matter stop.
    fn drop_tiny_velocity(&mut self) {
        let v = &mut self.velocity;
        if v.x * v.x + v.z * v.z < 9.0e-6 {
            (v.x, v.z) = (0.0, 0.0);
        }
        if v.y.abs() < 0.003 {
            v.y = 0.0;
        }
    }
    /// `Entity.getTicksRequiredToFreeze`.
    pub const TICKS_REQUIRED_TO_FREEZE: i32 = 140;
    /// `Entity.getPercentFrozen`.
    pub fn percent_frozen(&self) -> f32 {
        self.ticks_frozen.min(Self::TICKS_REQUIRED_TO_FREEZE) as f32
            / Self::TICKS_REQUIRED_TO_FREEZE as f32
    }
    /// `Entity.isFullyFrozen`.
    pub fn is_fully_frozen(&self) -> bool {
        self.ticks_frozen >= Self::TICKS_REQUIRED_TO_FREEZE
    }
    /// `LivingEntity.canFreeze`: not a spectator, no freeze-immune armor.
    pub fn can_freeze(&self) -> bool {
        !self.spectator && !self.freeze_immune
    }
    /// What powder snow's contextual collision asks of the player.
    pub fn powder_snow_subject(&self) -> crate::collisions::PowderSnowSubject {
        crate::collisions::PowderSnowSubject {
            fall_distance: self.fall_distance,
            falling_block: false,
            leather_boots: self.leather_boots,
            walkable_type: false,
            lava_walker: false,
        }
    }
    /// `LivingEntity.tryAddFrost`: the frozen slowdown, on anything but air.
    fn try_add_frost(&mut self, on_air: bool) -> Result<()> {
        if on_air || self.ticks_frozen <= 0 {
            return Ok(());
        }
        let Some(key) = self.attributes.key("minecraft:movement_speed") else {
            return Ok(());
        };
        let Some(handle) = self
            .attributes
            .get_instance(Some(key))
            .map_err(attribute_error)?
        else {
            return Ok(());
        };
        let slow = -0.05f32 * self.percent_frozen();
        let modifier = Modifier::new(
            "minecraft:powder_snow",
            f64::from(slow),
            Operation::AddValue,
        )?;
        self.attributes
            .edit(&handle, |value, observer| {
                value.add_transient(modifier, observer)
            })
            .map_err(attribute_error)?;
        Ok(())
    }
    pub fn bounds(&self) -> Aabb {
        self.dimensions.bounds(self.position)
    }
    pub fn attribute(&mut self, name: &str) -> Result<f64> {
        let key = self
            .attributes
            .key(name)
            .ok_or_else(|| format!("missing movement attribute {name}"))?;
        Ok(self
            .attributes
            .value(Some(key), ValueKind::Computed)
            .map_err(attribute_error)?)
    }
    /// Add (or remove) a transient attribute modifier, e.g. from equipment
    /// (`LivingEntity.collectEquipmentChanges` applying item modifiers).
    pub fn set_modifier(
        &mut self,
        attribute: &str,
        id: &str,
        amount: f64,
        operation: &str,
        present: bool,
    ) -> Result<()> {
        let Some(key) = self.attributes.key(attribute) else {
            return Ok(());
        };
        let Some(handle) = self
            .attributes
            .get_instance(Some(key))
            .map_err(attribute_error)?
        else {
            return Ok(());
        };
        let operation = match operation {
            "add_multiplied_base" => Operation::AddMultipliedBase,
            "add_multiplied_total" => Operation::AddMultipliedTotal,
            _ => Operation::AddValue,
        };
        let modifier = Modifier::new(id, amount, operation)?;
        self.attributes
            .edit(&handle, |value, observer| {
                value.remove(modifier.id(), observer)?;
                if present {
                    value.add_transient(modifier, observer)?;
                }
                Ok(())
            })
            .map_err(attribute_error)?;
        Ok(())
    }
    pub fn set_sprinting(&mut self, sprinting: bool) -> Result<()> {
        self.sprinting = sprinting;
        let key = self
            .attributes
            .key("minecraft:movement_speed")
            .ok_or("missing movement speed")?;
        let handle = self
            .attributes
            .get_instance(Some(key))
            .map_err(attribute_error)?
            .ok_or("player has no movement speed")?;
        let modifier = Modifier::new(
            "minecraft:sprinting",
            f64::from(0.3_f32),
            Operation::AddMultipliedTotal,
        )?;
        self.attributes
            .edit(&handle, |value, observer| {
                value.remove(modifier.id(), observer)?;
                if sprinting {
                    value.add_transient(modifier, observer)?;
                }
                Ok(())
            })
            .map_err(attribute_error)?;
        Ok(())
    }
    /// Local input sprint phase (`LocalPlayer.aiStep`'s sprint decisions).
    /// Vehicle, item-use and effect bindings remain separate; hunger, flight,
    /// water and collision conditions use this actor's actual state.
    pub fn sprint_input_tick(&mut self, keys: crate::sprint::Keys) -> Result<()> {
        self.sprint_input_tick_with_window(keys, 7)
    }
    /// The same phase with Java's `Options.sprintWindow` value.
    pub fn sprint_input_tick_with_window(
        &mut self,
        keys: crate::sprint::Keys,
        sprint_window: i32,
    ) -> Result<()> {
        let conditions = crate::sprint::Conditions {
            sprinting: self.sprinting,
            enough_food: self.health.food.has_enough_food(),
            may_fly: self.may_fly,
            flying: self.flying,
            // Entity.isInShallowWater.
            shallow_water: self.fluids.was_touching_water && !self.is_under_water(),
            fall_flying: self.fall_flying,
            on_ground: self.ground.on_ground,
            horizontal_collision: self.collision.horizontal,
            minor_horizontal_collision: self.collision.minor,
            slow_item: self.item_use_slows_sprint,
            moving_slowly: self.moving_slowly(),
            in_water: self.fluids.was_touching_water,
            under_water: self.is_under_water(),
            swimming: self.swimming,
            ..Default::default()
        };
        // The sprint setter edits attributes and flags, independent of local
        // keyboard history. Retain the input prefix even if that setter fails.
        let mut state = self.sprint_input;
        let result = state.tick(keys, conditions, sprint_window.clamp(0, 10), |value| {
            self.set_sprinting(value)
        });
        self.sprint_input = state;
        result
    }
    fn remove_frost(&mut self) -> Result<()> {
        let key = self
            .attributes
            .key("minecraft:movement_speed")
            .ok_or("missing movement speed")?;
        // Native removeFrost obtains the instance even when no modifier exists.
        // Pure value lookup would miss lazy creation and its pending updates.
        if let Some(handle) = self
            .attributes
            .get_instance(Some(key))
            .map_err(attribute_error)?
        {
            self.attributes
                .edit(&handle, |value, observer| {
                    value.remove("minecraft:powder_snow", observer)
                })
                .map_err(attribute_error)?;
        }
        Ok(())
    }
}

pub struct MotionWorld<'p, 'w, 'r> {
    /// Entity boxes selected by the host for this player's collision queries.
    pub entities: &'w [Aabb],
    pub player: &'p mut PlayerMotion,
    pub scene: &'w mut World,
    pub assets: &'r Assets,
    pub collision_binding: &'w BlockCollisionRegistry<'r>,
    pub resources: &'w MotionResources,
}
enum TickInput {
    Living([f32; 3]),
    PreparedLocal { direction: [f32; 2], jumping: bool },
}
impl MotionWorld<'_, '_, '_> {
    fn context(&self) -> CollisionContext<'static> {
        CollisionContext::for_entity(
            CollisionEntity {
                identity: EntityIdentity(0),
                y: self.player.position.y,
                descending: self.player.shift_key_down,
                main_hand: Some(CollisionItem::EMPTY),
            },
            false,
        )
    }
    fn collisions(&self) -> SceneCollisions<'_, '_> {
        SceneCollisions {
            world: self.scene,
            binding: self.collision_binding,
            subject: Some(self.player.powder_snow_subject()),
        }
    }
    fn movement_collisions(
        &self,
    ) -> crate::collisions::EntityCollisions<'_, SceneCollisions<'_, '_>> {
        let at = [self.player.position.x, self.player.position.z];
        crate::collisions::EntityCollisions {
            blocks: self.collisions(),
            entities: self.entities,
            border: self.player.border.as_ref().map(|b| (b, at)),
        }
    }
    /// `Level.noCollision(player, bounds)`: blocks and the host's entity boxes.
    pub fn no_player_collision(&mut self, bounds: Aabb) -> Result<bool> {
        let context = self.context();
        Ok(world_movement::no_world_collision(
            &mut self.movement_collisions(),
            world_movement::CollisionCheckRequest {
                entity: EntityIdentity(0),
                context,
                bounds,
                candidate_budget: 1_000_000,
            },
        )?)
    }
    fn block(&self, p: BlockPos) -> BlockStateId {
        self.scene.block(p.x, p.y, p.z)
    }
    fn support(&mut self, offset: f32) -> Result<BlockPos> {
        let position = self.player.position;
        let support = self.player.ground.supporting_block;
        Ok(movement_support::support_position(
            &mut RegistryMovementWorld {
                properties: &self.resources.blocks,
                world: self,
            },
            position,
            support,
            offset,
        )?)
    }
    fn location(&self) -> MovementLocation {
        MovementLocation {
            position: self.player.position,
            block_position: BlockPos::containing(self.player.position),
            supporting_block: self.player.ground.supporting_block,
        }
    }
    /// `Player.onClimbable` (never while flying) over `LivingEntity.onClimbable`
    /// at the player's feet: the climbable tag or an open trapdoor above a
    /// matching ladder. It remembers the block.
    pub fn climbable(&mut self) -> Result<bool> {
        if self.player.flying {
            return Ok(false);
        }
        let resources = self.resources;
        climbing::on_climbable(self, &resources.climbing).map_err(|e| e.to_string().into())
    }
    pub fn travel_in_air(&mut self, input: Vec3) -> Result<()> {
        living_air::travel_in_air(self, input)
    }
    /// Dry, standing movement phases of LivingEntity.aiStep, including powder
    /// snow's block-inside effects and server freezing (thawing, the frost
    /// modifier, freeze damage). Equipment, interpolation, effects and other
    /// world/entity effects remain unbound. Inputs are the native
    /// float xxa/yya/zza fields, before LivingEntity.applyInput damping.
    pub fn movement_tick(&mut self, input: [f32; 3]) -> Result<()> {
        self.dry_tick(TickInput::Living(input))
    }
    /// Dry Player.aiStep movement with a prepared local input vector. The
    /// caller applies LocalPlayer input preparation once. Empty inventory,
    /// equipment and nearby-entity sets are the current scene domain.
    pub fn player_movement_tick(&mut self, direction: [f32; 2], jumping: bool) -> Result<()> {
        self.fluid_base_tick()?;
        self.player_movement_tick_with_fluids(direction, jumping)
    }
    fn player_movement_tick_with_fluids(
        &mut self,
        direction: [f32; 2],
        jumping: bool,
    ) -> Result<()> {
        // Entity.baseTick
        self.player.was_in_powder_snow = self.player.in_powder_snow;
        self.player.in_powder_snow = false;
        self.player.old_position = self.player.position;
        if self.player.jump_trigger_time > 0 {
            self.player.jump_trigger_time -= 1;
        }
        self.player.peaceful_regeneration(self.scene.health_rules)?;
        // LocalPlayer.aiStep: double-tap jump toggles creative flight.
        let was_jumping = std::mem::replace(&mut self.player.was_jumping, jumping);
        let flying_before = self.player.flying;
        if self.player.spectator {
            self.player.flying = true;
            self.player.ground.on_ground = false;
        } else if !self.player.may_fly {
            self.player.flying = false;
        } else if !was_jumping && jumping && !self.player.was_auto_jump {
            if self.player.jump_trigger_time == 0 {
                self.player.jump_trigger_time = 7;
            } else if !self.player.swimming {
                self.player.flying = !self.player.flying;
                if self.player.flying && self.player.ground.on_ground {
                    self.jump_from_ground()?;
                }
                self.player.jump_trigger_time = 0;
            }
        }
        let toggled = flying_before != self.player.flying;
        // The server clears the glide flag from the position the client sent
        // last tick, and the client applies it a tick later, before aiStep: a
        // landing glide lasts one more tick on the ground, with no ground
        // friction, so a jump then keeps the speed (the elytra hop).
        if std::mem::take(&mut self.player.glide_ending) {
            self.player.fall_flying = false;
        }
        // Player.tryToStartFallFlying (canGlide: airborne, not flying, a glider;
        // never from inside a liquid).
        let can_glide = self.player.glider
            && !self.player.ground.on_ground
            && !self.player.flying
            && self.player.status.levitation.is_none()
            && !self.player.fluids.was_touching_water
            && !self.player.is_in_lava();
        // LocalPlayer.aiStep: `!onClimbable()` before tryToStartFallFlying.
        if jumping
            && !toggled
            && !was_jumping
            && !self.climbable()?
            && !self.player.fall_flying
            && can_glide
        {
            self.player.fall_flying = true;
        }
        // LivingEntity.tick fallFlyTicks, then updateFallFlying.
        if self.player.fall_flying {
            // Entity.checkFallDistanceAccumulation: a glide that isn't diving
            // (falling slower than 0.5 a tick) keeps the fall distance at 1, so
            // landing from it hurts no more than a one-block drop.
            if self.player.velocity.y > -0.5 && self.player.fall_distance > 1.0 {
                self.player.fall_distance = 1.0;
            }
            self.player.fall_fly_ticks += 1;
            // canGlide alone: water and lava don't end a glide, levitation does.
            let still = self.player.glider
                && !self.player.ground.on_ground
                && !self.player.flying
                && self.player.status.levitation.is_none();
            if !still {
                self.player.glide_ending = true;
            } else if (self.player.fall_fly_ticks + 1) % 10 == 0
                && ((self.player.fall_fly_ticks + 1) / 10) % 2 == 0
            {
                self.player.glider_wear += 1;
            }
        } else {
            self.player.fall_fly_ticks = 0;
        }
        // LocalPlayer.goDownInWater, before LivingEntity.aiStep's velocity cleanup.
        if self.player.fluids.was_touching_water
            && self.player.shift_key_down
            && !self.player.flying
        {
            self.player.velocity.y += f64::from(-0.04f32);
        }
        if self.player.flying {
            // Vertical flight: jump up, shift down, three times the flying speed.
            let ya = i32::from(jumping) - i32::from(self.player.shift_key_down);
            if ya != 0 {
                self.player.velocity.y += f64::from(ya as f32 * self.player.flying_speed * 3.0);
            }
        }
        self.dry_tick(TickInput::PreparedLocal {
            direction,
            jumping: jumping && !self.player.flying,
        })?;
        if self.player.spectator {
            self.player.ground.on_ground = false;
        }
        if self.player.flying {
            self.player.fall_distance = 0.0;
            if self.player.ground.on_ground && !self.player.spectator {
                self.player.flying = false;
            }
        }
        self.player.head_yaw = self.player.yaw;
        // Player.aiStep refreshes speed after movement, not before travel.
        self.player.speed = self.player.attribute("minecraft:movement_speed")? as f32;
        Ok(())
    }
    /// Resident simulation phases in native order. Enclosing entity/world tick
    /// services are still incomplete; this composes clocks, movement and hunger.
    pub fn simulation_tick(&mut self, direction: [f32; 2], jumping: bool) -> Result<()> {
        self.player.health.advance_clock();
        self.fluid_base_tick()?;
        self.simulation_tick_with_fluids(direction, jumping)
    }
    fn simulation_tick_with_fluids(&mut self, direction: [f32; 2], jumping: bool) -> Result<()> {
        self.tick_environment_damage()?;
        let previous = self.player.position;
        self.player_movement_tick_with_fluids(direction, jumping)?;
        self.apply_fire_contact()?;
        self.player.refresh_attributes()?;
        self.update_player_pose()?;
        // The resident scene accepts the resolved local movement directly.
        // Vanilla dispatches this separately from travel, on packet acceptance.
        self.check_movement_statistics(previous.vector_to(self.player.position))?;
        self.player
            .food_tick(&self.resources.health, self.scene.health_rules)
    }
    pub fn controlled_simulation_tick(&mut self, keys: crate::sprint::Keys) -> Result<()> {
        self.controlled_simulation_tick_with_sprint_window(keys, 7)
    }
    /// Local simulation with the configured double-tap sprint window.
    pub fn controlled_simulation_tick_with_sprint_window(
        &mut self,
        keys: crate::sprint::Keys,
        sprint_window: i32,
    ) -> Result<()> {
        // The base tick (fluids, swimming) precedes LocalPlayer.aiStep.
        self.player.health.advance_clock();
        self.fluid_base_tick()?;
        self.update_local_crouching()?;
        let mut keys = keys;
        // LocalPlayer.aiStep: a pending auto-jump presses jump for this tick
        // (input.makeJump) and keeps the jump from toggling flight.
        self.player.was_auto_jump = self.player.auto_jump_time > 0;
        if self.player.was_auto_jump {
            self.player.auto_jump_time -= 1;
            keys.jump = true;
        }
        self.player
            .sprint_input_tick_with_window(keys, sprint_window)?;
        self.player.shift_key_down = keys.shift;
        let direction = keys.direction();
        self.player.move_vector = direction;
        let sneak = if direction[0] * direction[0] + direction[1] * direction[1] != 0.0
            && self.player.moving_slowly()
        {
            Some(self.player.attribute("minecraft:sneaking_speed")? as f32)
        } else {
            None
        };
        self.simulation_tick_with_fluids(
            crate::input::modify_input(direction, self.player.item_use_speed, sneak),
            keys.jump,
        )
    }
    pub fn check_movement_statistics(&mut self, delta: Vec3) -> Result<()> {
        movement_statistics::check(self, delta)
    }
    fn dry_tick(&mut self, input: TickInput) -> Result<()> {
        if self.player.no_jump_delay > 0 {
            self.player.no_jump_delay -= 1;
        }
        self.player.drop_tiny_velocity();
        let mut input = match input {
            TickInput::Living(v) => Vec3::new(
                f64::from(v[0] * 0.98),
                f64::from(v[1]),
                f64::from(v[2] * 0.98),
            ),
            TickInput::PreparedLocal { direction, jumping } => {
                self.player.jumping = jumping;
                Vec3::new(f64::from(direction[0]), 0.0, f64::from(direction[1]))
            }
        };
        if self.player.health.dead_or_dying() {
            self.player.jumping = false;
            input.x = 0.0;
            input.z = 0.0;
        }
        self.jump_phase()?;
        let before_travel = self.player.bounds();
        self.player_travel(input)?;
        if self.player.block_effects {
            self.apply_effects_from_blocks()?;
        }
        // LivingEntity.aiStep "freezing" (server).
        if !self.player.in_powder_snow || !self.player.can_freeze() {
            self.player.ticks_frozen = (self.player.ticks_frozen - 2).max(0);
        }
        self.player.remove_frost()?;
        let on = self.support(movement_support::LEGACY_SUPPORT_OFFSET)?;
        let on_air = self.assets.registry.state(self.block(on)).is_none_or(|s| {
            matches!(
                s.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            )
        });
        self.player.try_add_frost(on_air)?;
        self.update_soul_speed_block()?;
        if self.player.health.tick_count % 40 == 0
            && self.player.is_fully_frozen()
            && self.player.can_freeze()
        {
            self.freeze_damage()?;
        }
        if self.player.auto_spin_attack_ticks > 0 {
            self.player.auto_spin_attack_ticks -= 1;
            self.check_auto_spin_attack(before_travel);
        }
        Ok(())
    }
    /// Refresh whether the player stands on a soul speed block, for the
    /// `location_changed` enchantment effects (a server's player too, whose
    /// client does the walking).
    pub fn update_soul_speed_block(&mut self) -> Result<()> {
        self.player.location_effects.on_soul_speed_block = self.on_soul_speed_block()?;
        Ok(())
    }
    /// `MovementAffectedByPredicate` for `#minecraft:soul_speed_blocks`.
    fn on_soul_speed_block(&mut self) -> Result<bool> {
        let on = self.support(movement_support::MOVEMENT_SUPPORT_OFFSET)?;
        Ok(self
            .assets
            .registry
            .state(self.block(on))
            .is_some_and(|s| self.resources.soul_speed_blocks.contains(&s.name)))
    }
    /// Player.travel: swim steering, then LivingEntity.travel (fluids first,
    /// then gliding, then air), keeping 0.6 of the vertical speed while flying.
    fn player_travel(&mut self, input: Vec3) -> Result<()> {
        if self.player.swimming {
            let look_y =
                horizon_world::view_direction::view_vector(self.player.pitch, self.player.yaw).y;
            let near_surface = BlockPos::containing(Vec3::new(
                self.player.position.x,
                self.player.position.y + 1.0 - 0.1,
                self.player.position.z,
            ));
            if look_y <= 0.0
                || self.player.jumping
                || self.fluid_at(near_surface).kind() != horizon_world::fluid::FluidKind::Empty
            {
                let blend = if look_y < -0.2 { 0.085 } else { 0.06 };
                self.player.velocity.y += (look_y - self.player.velocity.y) * blend;
            }
        }
        let flying_y = self.player.flying.then_some(self.player.velocity.y);
        if !self.player.flying
            && (self.player.fluids.was_touching_water || self.player.is_in_lava())
        {
            horizon_world::living_fluid::travel_in_fluid(self, input)?;
        } else if self.player.fall_flying {
            // LivingEntity.travelFallFlying: on a climbable the glide ends and
            // the player moves like in the air.
            if self.climbable()? {
                self.travel_in_air(input)?;
                self.player.fall_flying = false;
            } else {
                self.travel_fall_flying()?;
            }
        } else {
            self.travel_in_air(input)?;
        }
        if let Some(y) = flying_y {
            self.player.velocity.y = y * 0.6;
        }
        Ok(())
    }
    /// ServerPlayer's own travel: the server simulates a player too
    /// (`MoveSimulationType.AUTHORITATIVE_SIDE_AND_SERVER`), with no input
    /// (it never sets xxa/zza). Only the delta movement it leaves stays: it
    /// is what a hurt sends the player (`ServerEntity.sendChanges`), and
    /// what the next knockback starts from; the client's next move places
    /// the player.
    pub fn server_travel(&mut self) -> Result<()> {
        let p = &mut *self.player;
        let (position, ground, collision) = (p.position, p.ground, p.collision);
        let (history, fall_distance, steps) = (p.history.clone(), p.fall_distance, p.steps);
        let events = p.events.len();
        p.drop_tiny_velocity();
        p.server_copy = true;
        let travelled = self.player_travel(Vec3::ZERO);
        let p = &mut *self.player;
        p.server_copy = false;
        (p.position, p.ground, p.collision) = (position, ground, collision);
        (p.history, p.fall_distance, p.steps) = (history, fall_distance, steps);
        // The wall hit is the server's own: its sound and damage events stay.
        let kept: Vec<_> = p
            .wall_hit_events
            .take()
            .map(|span| p.events.drain(span).collect())
            .unwrap_or_default();
        p.events.truncate(events);
        p.events.extend(kept);
        travelled
    }
    /// `LivingEntity.checkAutoSpinAttack`: the first living entity in the
    /// box swept this tick takes the hit and ends the spin, bouncing the
    /// player back at a fifth of its speed; a wall ends it too.
    fn check_auto_spin_attack(&mut self, before: Aabb) {
        let swept = before.union_bounds(self.player.bounds());
        let p = &mut self.player;
        if let Some(&(id, _)) = p.spin_targets.iter().find(|(_, b)| b.intersects(swept)) {
            p.spin_hit = Some(id);
            p.auto_spin_attack_ticks = 0;
            p.velocity = p.velocity.scale(-0.2);
        } else if p.collision.horizontal {
            p.auto_spin_attack_ticks = 0;
        }
    }
    /// `hurtServer(damageSources().freeze(), 1)` for a fully frozen player.
    fn freeze_damage(&mut self) -> Result<()> {
        let id = self
            .resources
            .health
            .damage
            .id("minecraft:freeze")
            .ok_or("missing freeze damage type")?;
        self.player.events.push(MotionEvent::Health(
            crate::health::HealthEvent::FreezeDamage(1.0),
        ));
        let rules = self.scene.health_rules;
        self.player.hurt(&self.resources.health, rules, id, 1.0)?;
        Ok(())
    }
    /// `Entity.applyEffectsFromBlocks` for the blocks Horizon's player reacts
    /// to inside its movement path: powder snow (`PowderSnowBlock.entityInside`
    /// and the FREEZE / EXTINGUISH effects), cobwebs, sweet berry bushes,
    /// cactus and lit campfires, and the magma and slime blocks it stands on
    /// (`stepOn`). Pressure plates, tripwires and rails are the world tick's
    /// scan.
    pub fn apply_effects_from_blocks(&mut self) -> Result<()> {
        self.effects_from_blocks(false)
    }
    /// What `applyEffectsFromBlocks` does to a server's player along the
    /// moves its client sent since `from` (in its history): the damage of
    /// magma, cactus, lit campfires and grown sweet berry bushes, and powder
    /// snow's FREEZE, then `LivingEntity.aiStep`'s freezing and its damage.
    /// Its client moves it, so the slowing and the rest are the client's.
    pub fn server_block_damage(&mut self, from: Vec3) -> Result<()> {
        self.player.old_position = from;
        // Entity.baseTick
        self.player.was_in_powder_snow = self.player.in_powder_snow;
        self.player.in_powder_snow = false;
        self.effects_from_blocks(true)?;
        // LivingEntity.aiStep "freezing" (server).
        if !self.player.in_powder_snow || !self.player.can_freeze() {
            self.player.ticks_frozen = (self.player.ticks_frozen - 2).max(0);
        }
        if self.player.health.tick_count % 40 == 0
            && self.player.is_fully_frozen()
            && self.player.can_freeze()
        {
            self.freeze_damage()?;
        }
        Ok(())
    }
    fn effects_from_blocks(&mut self, damage_only: bool) -> Result<()> {
        use horizon_world::entity_move::MovementRecord;
        let mut movements: Vec<MovementRecord> =
            self.player.history.records().iter().copied().collect();
        self.player.history.clear();
        let position = self.player.position;
        match movements.last() {
            None => movements.push(MovementRecord {
                from: self.player.old_position,
                to: position,
                original: None,
            }),
            Some(last) if last.to.distance_squared(position) > f64::from(9.999_999_4e-11_f32) => {
                movements.push(MovementRecord {
                    from: last.to,
                    to: position,
                    original: None,
                })
            }
            _ => {}
        }
        // isAffectedByBlocks: no physics for spectators.
        if self.player.spectator {
            return Ok(());
        }
        // Block.stepOn for the block under an entity on the ground:
        // MagmaBlock's hot floor hurts unless it steps carefully (sneaks).
        if self.player.ground.on_ground {
            let on = self.legacy_on_position()?;
            let under = self.assets.registry.state(self.block(on));
            let magma = under.is_some_and(|s| s.name == "minecraft:magma_block");
            let slime = under.is_some_and(|s| s.name == "minecraft:slime_block");
            if magma && !self.player.shift_key_down {
                self.environment_hurt("minecraft:hot_floor", 1.0)?;
            }
            // TurtleEggBlock.stepOn: a step that is not careful may break an egg.
            if !self.player.shift_key_down {
                let state = self.block(on);
                self.resources
                    .landing
                    .step_on(
                        self,
                        state,
                        on,
                        horizon_world::block_landing::LandingEntityKind::Player,
                    )
                    .map_err(|e| e.to_string())?;
            }
            // SlimeBlock.stepOn: a slow walk on slime drags the horizontal
            // speed (the client moves its own player, so not the server's copy).
            if slime && !damage_only && !self.player.shift_key_down {
                let abs_delta_y = self.player.velocity.y.abs();
                if abs_delta_y < 0.1 {
                    let scale = 0.4 + abs_delta_y * 0.2;
                    self.player.velocity.x *= scale;
                    self.player.velocity.z *= scale;
                }
            }
        }
        let mut visited = std::collections::HashSet::new();
        let mut entered = Vec::new();
        for movement in &movements {
            let mut from = movement.from;
            let delta = movement.to.subtract(movement.from);
            let mut budget = 16;
            match movement.original {
                Some(original) if delta.length_squared() > 0.0 => {
                    for axis in Axis::step_order(original) {
                        let step = delta.component(axis);
                        if step != 0.0 {
                            let to = from.relative(
                                match axis {
                                    Axis::X => Direction::East,
                                    Axis::Y => Direction::Up,
                                    Axis::Z => Direction::South,
                                },
                                step,
                            );
                            budget -= self.check_inside_blocks(
                                from,
                                to,
                                &mut visited,
                                &mut entered,
                                budget,
                                damage_only,
                            )?;
                            from = to;
                        }
                    }
                }
                _ => {
                    budget -= self.check_inside_blocks(
                        movement.from,
                        movement.to,
                        &mut visited,
                        &mut entered,
                        16,
                        damage_only,
                    )?;
                }
            }
            if budget <= 0 {
                self.check_inside_blocks(
                    movement.to,
                    movement.to,
                    &mut visited,
                    &mut entered,
                    1,
                    damage_only,
                )?;
            }
        }
        // InsideBlockEffectApplier.StepBasedCollector: the blocks entered
        // at one step (a sweep iteration differing from the last) form a
        // group whose EXTINGUISH callbacks (the game destroys those blocks if
        // the player burns), FREEZE and EXTINGUISH apply once, in order.
        let mut last_step = -1;
        let mut groups = 0;
        for (index, &(step, p)) in entered.iter().enumerate() {
            if index == 0 || step != last_step {
                groups += 1;
                last_step = step;
            }
            if groups == 1 && !damage_only {
                // A later group finds the fire already out.
                self.player.powder_snow_entered.push(p);
            }
        }
        for _ in 0..groups {
            if self.player.health.dead_or_dying() {
                break;
            }
            self.player.in_powder_snow = true;
            if self.player.can_freeze() {
                self.player.ticks_frozen =
                    (self.player.ticks_frozen + 1).min(PlayerMotion::TICKS_REQUIRED_TO_FREEZE);
            }
        }
        Ok(())
    }
    /// `Entity.checkInsideBlocks(from, to, ...)`: the blocks the box at `to`
    /// sweeps through from `from`; returns the iterations used plus one.
    fn check_inside_blocks(
        &mut self,
        from: Vec3,
        to: Vec3,
        visited: &mut std::collections::HashSet<i64>,
        entered: &mut Vec<(i32, BlockPos)>,
        max_iterations: i32,
        damage_only: bool,
    ) -> Result<i32> {
        let deflated = self.player.dimensions.bounds(to).deflate(Vec3::new(
            f64::from(1.0e-5_f32),
            f64::from(1.0e-5_f32),
            f64::from(1.0e-5_f32),
        ));
        // Mth.square(0.9999900000002526): a long move counts every block it
        // crosses as precise.
        let far = 0.999_990_000_000_252_6_f64;
        let moved_far = from.distance_squared(to) > far * far;
        let mut hits = Vec::new();
        let mut iterations = 0;
        let alive = !self.player.health.dead_or_dying();
        horizon_core::geometry::sweep_intersected_blocks(
            from,
            to,
            deflated,
            1_000_000,
            |p, iteration| {
                if !alive || iteration >= max_iterations {
                    return false;
                }
                iterations = iteration;
                hits.push((iteration, p));
                true
            },
        )
        .map_err(|e| format!("block sweep: {e:?}"))?;
        for (step, p) in hits {
            let state = self.block(p);
            let Some(definition) = self.assets.registry.state(state) else {
                continue;
            };
            let name = definition.name.as_str();
            if matches!(
                name,
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            ) {
                continue;
            }
            let powder_snow = name == "minecraft:powder_snow";
            // A filled cauldron's content top in blocks: the lava's 15/16
            // (LavaCauldronBlock.FILLED_SHAPE), or a layered cauldron's
            // getPixelContentHeight, 6 + 3 per level (FILLED_SHAPES).
            let content_top = match name {
                "minecraft:lava_cauldron" => Some(15.0 / 16.0),
                "minecraft:water_cauldron" | "minecraft:powder_snow_cauldron" => {
                    let level = self
                        .assets
                        .registry
                        .state(state)
                        .and_then(|s| s.properties.get("level"))
                        .and_then(|l| l.parse::<u8>().ok())
                        .unwrap_or(1);
                    Some((6.0 + 3.0 * f64::from(level)) / 16.0)
                }
                _ => None,
            };
            // getEntityInsideCollisionShape is the whole block but for powder
            // snow (its collision shape, the whole block when it is empty),
            // filled cauldrons, end portals and tripwires; Horizon doesn't
            // follow the player into those last two.
            if matches!(name, "minecraft:end_portal" | "minecraft:tripwire") {
                continue;
            }
            let inside = !(powder_snow || content_top.is_some()) || {
                let context = self.context();
                let binding = self.collision_binding;
                let shape = binding.collision_shape_with_context(
                    &mut self.collisions(),
                    state,
                    p,
                    &context,
                )?;
                powder_snow && (shape.is_empty() || shape.boxes() == VoxelShape::block().boxes())
                    || {
                        let offset = Vec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z));
                        let mut boxes: Vec<_> = shape.moved(offset).boxes().to_vec();
                        if let Some(top) = content_top {
                            // The cauldron's shape or its content,
                            // Block.column(12, 4, top).
                            boxes.push(Aabb::new(
                                Vec3::new(2.0 / 16.0, 4.0 / 16.0, 2.0 / 16.0).add_vector(offset),
                                Vec3::new(14.0 / 16.0, top, 14.0 / 16.0).add_vector(offset),
                            ));
                        }
                        self.player
                            .dimensions
                            .bounds(from)
                            .collided_along_vector(to.subtract(from), &boxes)
                    }
            };
            if inside && visited.insert(p.pack()) {
                // CactusBlock, CampfireBlock and the cauldrons' entityInside
                // (`entity.hurt`, the fire and the fill level), and
                // WitherRoseBlock's server-side effect.
                match name {
                    "minecraft:cactus" => self.environment_hurt("minecraft:cactus", 1.0)?,
                    "minecraft:lava_cauldron" => self.lava_cauldron_inside()?,
                    "minecraft:water_cauldron" | "minecraft:powder_snow_cauldron" => {
                        self.layered_cauldron_inside(p);
                    }
                    "minecraft:wither_rose" => {
                        self.player.events.push(MotionEvent::InsideWitherRose);
                    }
                    "minecraft:campfire" | "minecraft:soul_campfire" => {
                        let lit = self
                            .assets
                            .registry
                            .state(state)
                            .and_then(|s| s.properties.get("lit"))
                            .is_some_and(|v| v == "true");
                        let damage = if name == "minecraft:campfire" {
                            1.0
                        } else {
                            2.0
                        };
                        if lit {
                            self.environment_hurt("minecraft:campfire", damage)?;
                        }
                    }
                    "minecraft:sweet_berry_bush" if damage_only => {
                        self.sweet_berry_bush_damage(state)?;
                    }
                    // HoneyBlock.entityInside: a server's player slides too, so
                    // its fall distance resets (its client moves it).
                    "minecraft:honey_block" => self.honey_block_inside(p)?,
                    // PowderSnowBlock.entityInside's FREEZE.
                    "minecraft:powder_snow" if damage_only => entered.push((step, p)),
                    _ => {}
                }
                if damage_only {
                    continue;
                }
                match name {
                    "minecraft:powder_snow" => {
                        self.powder_snow_inside(p)?;
                        entered.push((step, p));
                    }
                    "minecraft:cobweb" => self.web_inside(),
                    "minecraft:sweet_berry_bush" => self.sweet_berry_bush_inside(state)?,
                    "minecraft:bubble_column" => {
                        let precise = moved_far || deflated.intersects(Aabb::block(p));
                        self.bubble_column_inside(state, p, precise)?
                    }
                    "minecraft:nether_portal" => self
                        .player
                        .events
                        .push(MotionEvent::InsidePortal { pos: p }),
                    _ => {}
                }
                // ServerPlayer.onInsideBlock
                self.player.events.push(MotionEvent::EnterBlock { state });
            }
        }
        Ok(iterations + 1)
    }
    /// `HoneyBlock.entityInside`: a falling player touching a side of the block
    /// slides down it (`isSlidingDown`, `doSlideMovement`, `maybeDoSlideEffects`).
    fn honey_block_inside(&mut self, pos: BlockPos) -> Result<()> {
        let old_delta_y = |dy: f64| dy / f64::from(0.98_f32) + 0.08;
        let new_delta_y = |dy: f64| (dy - 0.08) * f64::from(0.98_f32);
        let position = self.player.position;
        let velocity = self.player.velocity;
        if self.player.ground.on_ground
            || position.y > f64::from(pos.y) + 0.9375 - 1.0e-7
            || old_delta_y(velocity.y) >= -0.08
        {
            return Ok(());
        }
        let dx = (f64::from(pos.x) + 0.5 - position.x).abs();
        let dz = (f64::from(pos.z) + 0.5 - position.z).abs();
        let overlap = 0.4375 + f64::from(self.player.dimensions.width / 2.0);
        if !(dx + 1.0e-7 > overlap || dz + 1.0e-7 > overlap) {
            return Ok(());
        }
        // doSlideMovement
        let old = old_delta_y(velocity.y);
        if old < -0.13 {
            let factor = -0.05 / old;
            self.player.velocity =
                Vec3::new(velocity.x * factor, new_delta_y(-0.05), velocity.z * factor);
        } else {
            self.player.velocity = Vec3::new(velocity.x, new_delta_y(-0.05), velocity.z);
        }
        self.player.fall_distance = 0.0;
        // maybeDoSlideEffects: the sound, then the server's event 53.
        let random = &mut self.scene.random;
        let sound = random.next_int_bound(5).map_err(|e| format!("{e:?}"))? == 0;
        // Only the server broadcasts event 53 (`!level.isClientSide()`).
        let particles = !self.player.client_side
            && random.next_int_bound(5).map_err(|e| format!("{e:?}"))? == 0;
        // The server also checks the slide criterion every 20 game ticks,
        // including ticks without a sound or a particle event.
        let criterion = !self.player.client_side && self.scene.game_time % 20 == 0;
        if sound || particles || criterion {
            self.player.events.push(MotionEvent::HoneySlide {
                pos,
                sound,
                particles,
            });
        }
        Ok(())
    }
    /// `Player.makeStuckInBlock`: the next move is scaled by `multiplier`
    /// (and the fall forgotten) unless flying; the impulse context resets
    /// either way.
    fn make_stuck_in_block(&mut self, multiplier: Vec3) {
        if !self.player.flying {
            self.player.fall_distance = 0.0;
            self.player.stuck_multiplier = multiplier;
        }
        self.player.impulse.try_reset();
    }
    /// `LavaCauldronBlock.entityInside`: CLEAR_FREEZE, LAVA_IGNITE
    /// (`Entity.lavaIgnite`, 15 seconds) and then `Entity.lavaHurt` (lava,
    /// 4), as ordinary lava does to the player.
    fn lava_cauldron_inside(&mut self) -> Result<()> {
        self.player.ticks_frozen = 0;
        self.player.ignite_for_ticks(300)?;
        self.environment_hurt("minecraft:lava", 4.0)
    }
    /// `LayeredCauldronBlock.entityInside`: on a server, a burning player
    /// asks for the cauldron's `handleEntityOnFireInside` (runBefore
    /// EXTINGUISH, so while still burning), then EXTINGUISH
    /// (`Entity.clearFire`) puts the fire out.
    fn layered_cauldron_inside(&mut self, pos: BlockPos) {
        if self.player.fire_ticks > 0 && !self.player.spectator {
            self.player
                .events
                .push(MotionEvent::CauldronExtinguish { pos });
        }
        self.player.fire_ticks = self.player.fire_ticks.min(0);
    }
    /// `WebBlock.entityInside`: stuck at a quarter of the speed, half with
    /// the Weaving effect.
    fn web_inside(&mut self) {
        let multiplier = if self.player.status.weaving {
            Vec3::new(0.5, 0.25, 0.5)
        } else {
            Vec3::new(0.25, f64::from(0.05_f32), 0.25)
        };
        self.make_stuck_in_block(multiplier);
    }
    /// `SweetBerryBushBlock.entityInside` for the player: slowed, and pricked
    /// (`sweetBerryBush` damage, 1) by a grown bush while its known movement
    /// is at least 0.003 on X or Z.
    fn sweet_berry_bush_inside(&mut self, state: BlockStateId) -> Result<()> {
        self.make_stuck_in_block(Vec3::new(f64::from(0.8_f32), 0.75, f64::from(0.8_f32)));
        self.sweet_berry_bush_damage(state)
    }
    /// The server's half of `SweetBerryBushBlock.entityInside`: the prick.
    fn sweet_berry_bush_damage(&mut self, state: BlockStateId) -> Result<()> {
        let age = self
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("age"))
            .map_or("0", |a| a.as_str());
        if age == "0" {
            return Ok(());
        }
        // Player.isClientAuthoritative: ServerPlayer.getKnownMovement, this
        // tick's movement.
        let known = self.player.position.subtract(self.player.old_position);
        if known.x * known.x + known.z * known.z > 0.0
            && (known.x.abs() >= f64::from(0.003_f32) || known.z.abs() >= f64::from(0.003_f32))
        {
            let id = self
                .resources
                .health
                .damage
                .id("minecraft:sweet_berry_bush")
                .ok_or("missing sweet_berry_bush damage type")?;
            self.player.events.push(MotionEvent::Health(
                crate::health::HealthEvent::SweetBerryBushDamage(1.0),
            ));
            let rules = self.scene.health_rules;
            self.player.hurt(&self.resources.health, rules, id, 1.0)?;
        }
        Ok(())
    }
    /// `PowderSnowBlock.entityInside` for the player: stuck while the block
    /// at its feet is powder snow, with a snowflake now and then (server).
    fn powder_snow_inside(&mut self, p: BlockPos) -> Result<()> {
        let feet = BlockPos::containing(self.player.position);
        let in_block = self
            .assets
            .registry
            .state(self.block(feet))
            .is_some_and(|s| s.name == "minecraft:powder_snow");
        if !in_block {
            return Ok(());
        }
        self.make_stuck_in_block(Vec3::new(f64::from(0.9_f32), 1.5, f64::from(0.9_f32)));
        // ServerPlayer.getKnownMovement: this tick's movement.
        let known = self.player.position.subtract(self.player.old_position);
        let random = &mut self.scene.random;
        if (known.x != 0.0 || known.z != 0.0) && random.next_boolean() {
            // Mth.randomBetween(random, -1, 1) * (1 / 12)
            let xa = (random.next_float() * 2.0 - 1.0) * 0.083_333_336_f32;
            let za = (random.next_float() * 2.0 - 1.0) * 0.083_333_336_f32;
            self.player.events.push(MotionEvent::Snowflake {
                position: Vec3::new(
                    self.player.position.x,
                    f64::from(p.y + 1),
                    self.player.position.z,
                ),
                velocity: Vec3::new(f64::from(xa), f64::from(0.05_f32), f64::from(za)),
            });
        }
        Ok(())
    }
    pub fn jump_from_ground(&mut self) -> Result<()> {
        living_motion::jump_from_ground(self)?;
        self.player
            .events
            .push(MotionEvent::Health(crate::health::HealthEvent::Statistic {
                name: "jump",
                amount: 1,
            }));
        self.player
            .cause_food_exhaustion(if self.player.sprinting { 0.2 } else { 0.05 });
        Ok(())
    }
    /// LivingEntity.travelFallFlying with updateFallFlyingMovement.
    fn travel_fall_flying(&mut self) -> Result<()> {
        let v = self.player.velocity;
        let (yaw, pitch) = (self.player.yaw.to_radians(), self.player.pitch.to_radians());
        let look = Vec3::new(
            f64::from(-yaw.sin() * pitch.cos()),
            f64::from(-pitch.sin()),
            f64::from(yaw.cos() * pitch.cos()),
        );
        let lean = pitch;
        let look_h = (look.x * look.x + look.z * look.z).sqrt();
        let move_h = (v.x * v.x + v.z * v.z).sqrt();
        let gravity = living_motion::effective_gravity(self)?;
        let lift = f64::from(lean.cos()).powi(2);
        let mut m = Vec3::new(v.x, v.y + gravity * (-1.0 + lift * 0.75), v.z);
        if m.y < 0.0 && look_h > 0.0 {
            let convert = m.y * -0.1 * lift;
            m = Vec3::new(
                m.x + look.x * convert / look_h,
                m.y + convert,
                m.z + look.z * convert / look_h,
            );
        }
        if lean < 0.0 && look_h > 0.0 {
            let convert = move_h * f64::from(-lean.sin()) * 0.04;
            m = Vec3::new(
                m.x - look.x * convert / look_h,
                m.y + convert * 3.2,
                m.z - look.z * convert / look_h,
            );
        }
        if look_h > 0.0 {
            m = Vec3::new(
                m.x + (look.x / look_h * move_h - m.x) * 0.1,
                m.y,
                m.z + (look.z / look_h * move_h - m.z) * 0.1,
            );
        }
        self.player.velocity = Vec3::new(m.x * 0.99, m.y * 0.98, m.z * 0.99);
        let movement = self.player.velocity;
        self.move_self(movement)?;
        // LivingEntity.handleFallFlyingCollisions: only the server hurts.
        if !self.player.client_side && self.player.collision.horizontal {
            let new_h = {
                let v = self.player.velocity;
                (v.x * v.x + v.z * v.z).sqrt()
            };
            let dmg = ((move_h - new_h) * 10.0 - 3.0) as f32;
            if dmg > 0.0 {
                let first = self.player.events.len();
                // getFallDamageSound((int) dmg)
                let sound = if dmg as i32 > 4 {
                    crate::landing::SceneSound::BigFall
                } else {
                    crate::landing::SceneSound::SmallFall
                };
                self.player
                    .events
                    .push(MotionEvent::Fall(crate::landing::FallEvent::Sound {
                        sound,
                        position: None,
                        volume: 1.0,
                        pitch: 1.0,
                    }));
                let id = self
                    .resources
                    .health
                    .damage
                    .id("minecraft:fly_into_wall")
                    .ok_or("missing fly_into_wall damage type")?;
                let rules = self.scene.health_rules;
                self.player.hurt(&self.resources.health, rules, id, dmg)?;
                if self.player.server_copy {
                    self.player.wall_hit_events = Some(first..self.player.events.len());
                }
            }
        }
        Ok(())
    }
    /// `LocalPlayer.move`: the move, then `updateAutoJump` with the float
    /// horizontal displacement.
    pub fn move_self(&mut self, movement: Vec3) -> Result<()> {
        let before = self.player.position;
        entity_move::move_entity(self, MoverType::SelfMovement, movement)?;
        let dx = (self.player.position.x - before.x) as f32;
        let dz = (self.player.position.z - before.z) as f32;
        self.update_auto_jump(dx, dz)
    }
    /// `LocalPlayer.updateAutoJump` behind `canAutoJump`.
    pub fn update_auto_jump(&mut self, xa: f32, za: f32) -> Result<()> {
        let p = &*self.player;
        // canAutoJump; MotionWorld drives an unmounted player (not a passenger).
        let ready = crate::auto_jump::can_auto_jump(
            p.auto_jump_enabled,
            p.auto_jump_time,
            p.ground.on_ground,
            p.shift_key_down,
            false,
            p.move_vector,
            1.0,
        );
        if !ready || f64::from(self.block_jump_factor()?) < 1.0 {
            return Ok(());
        }
        // Player.getSpeed: the movement speed attribute as it is now.
        let speed = self.player.attribute("minecraft:movement_speed")? as f32;
        let p = &*self.player;
        let jumper = crate::auto_jump::AutoJumper {
            position: p.position,
            max_y: p.bounds().max_position().y,
            x_rot: p.pitch,
            y_rot: p.yaw,
            speed,
            move_vector: p.move_vector,
            width: p.dimensions.width,
            height: p.dimensions.height,
            jump_boost: p.status.jump_boost,
        };
        if crate::auto_jump::update_auto_jump(self, &jumper, xa, za)? {
            self.player.auto_jump_time = 1;
        }
        Ok(())
    }
}
impl ClimbingWorld for MotionWorld<'_, '_, '_> {
    type Error = std::convert::Infallible;
    fn is_spectator(&mut self) -> std::result::Result<bool, Self::Error> {
        Ok(self.player.spectator)
    }
    fn block_position(&mut self) -> std::result::Result<BlockPos, Self::Error> {
        Ok(BlockPos::containing(self.player.position))
    }
    fn in_block_state(&mut self) -> std::result::Result<BlockStateId, Self::Error> {
        Ok(self.block(BlockPos::containing(self.player.position)))
    }
    fn is_fall_flying(&mut self) -> std::result::Result<bool, Self::Error> {
        Ok(self.player.fall_flying)
    }
    fn block_state(
        &mut self,
        position: BlockPos,
    ) -> std::result::Result<BlockStateId, Self::Error> {
        Ok(self.block(position))
    }
    fn set_last_climbable_position(&mut self, position: BlockPos) {
        self.player.last_climbable = Some(position);
    }
}
impl crate::auto_jump::AutoJumpWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn collision_shape(&mut self, p: BlockPos) -> Result<horizon_core::geometry::VoxelShape> {
        let context = self.context();
        let state = self.block(p);
        let binding = self.collision_binding;
        Ok(binding.collision_shape_with_context(&mut self.collisions(), state, p, &context)?)
    }
    fn collision_boxes(&mut self, bounds: Aabb) -> Result<Vec<Aabb>> {
        let context = self.context();
        let mut scene = self.collisions();
        let mut boxes = Vec::new();
        for hit in horizon_world::block_collision_scan::BlockCollisionScan::new(
            &mut scene, context, bounds, false, 1_000_000,
        )? {
            boxes.extend_from_slice(hit?.shape.boxes());
        }
        Ok(boxes)
    }
}
// Current resident movement domain: dry, standing, no vehicles or gliding.
// The shared kernel also handles swimming/crouching/flight when those states
// are supplied by their eventual runtime bindings.
impl MovementStatisticsWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn passenger(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn swimming(&mut self) -> Result<bool> {
        Ok(self.player.swimming)
    }
    fn eyes_in_water(&mut self) -> Result<bool> {
        Ok(self.player.is_eye_in_fluid(WATER))
    }
    fn in_water(&mut self) -> Result<bool> {
        Ok(self.player.fluids.was_touching_water)
    }
    fn on_climbable(&mut self) -> Result<bool> {
        self.climbable()
    }
    fn on_ground(&mut self) -> Result<bool> {
        Ok(self.player.ground.on_ground)
    }
    fn sprinting(&mut self) -> Result<bool> {
        Ok(self.player.sprinting)
    }
    fn crouching(&mut self) -> Result<bool> {
        Ok(self.player.pose == crate::pose::Pose::Crouching)
    }
    fn fall_flying(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn award(&mut self, stat: MovementStat, amount: i32) -> Result<()> {
        self.player
            .events
            .push(MotionEvent::Health(crate::health::HealthEvent::Statistic {
                name: stat.name(),
                amount,
            }));
        Ok(())
    }
    fn exhaust(&mut self, amount: f32) -> Result<()> {
        self.player.cause_food_exhaustion(amount);
        Ok(())
    }
}
impl MovementBlockProvider for MotionWorld<'_, '_, '_> {
    type Error = io::Error;
    fn movement_block_state(&mut self, p: BlockPos) -> io::Result<BlockStateId> {
        Ok(self.block(p))
    }
}
impl EdgeBackoffWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn max_up_step(&mut self) -> Result<f32> {
        Ok(self.player.attribute("minecraft:step_height")? as f32)
    }
    fn staying_on_ground_surface(&mut self) -> Result<bool> {
        Ok(self.player.shift_key_down)
    }
    fn on_ground(&mut self) -> Result<bool> {
        Ok(self.player.ground.on_ground)
    }
    fn no_collision(&mut self, bounds: Aabb) -> Result<bool> {
        self.no_player_collision(bounds)
    }
}
impl EntityMoveWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn no_physics(&self) -> bool {
        self.player.spectator
    }
    fn position(&self) -> Vec3 {
        self.player.position
    }
    fn set_position(&mut self, p: Vec3) -> Result<()> {
        self.player.position = p;
        Ok(())
    }
    fn record_movement(&mut self, mover: MoverType, movement: Vec3) -> Result<()> {
        self.player
            .events
            .push(MotionEvent::Recorded { mover, movement });
        Ok(())
    }
    fn collision_flags(&self) -> CollisionFlags {
        self.player.collision
    }
    fn set_collision_flags(&mut self, value: CollisionFlags) {
        self.player.collision = value;
    }
    /// `Entity.limitPistonMovement`: the per-tick 0.51-block budget shared by
    /// every move on an axis. A failed game-time read leaves the accumulator
    /// as it was, as native does.
    fn limit_piston_movement(&mut self, movement: Vec3) -> Result<Vec3> {
        let time = self.scene.game_time;
        self.player
            .piston
            .limit(movement, || Ok::<i64, Box<dyn std::error::Error>>(time))
    }
    fn stuck_multiplier(&self) -> Vec3 {
        self.player.stuck_multiplier
    }
    fn set_stuck_multiplier(&mut self, value: Vec3) {
        self.player.stuck_multiplier = value;
    }
    fn velocity(&mut self) -> Result<Vec3> {
        Ok(self.player.velocity)
    }
    fn set_velocity(&mut self, value: Vec3) -> Result<()> {
        self.player.velocity = value;
        Ok(())
    }
    fn back_off_edge(&mut self, movement: Vec3, mover: MoverType) -> Result<Vec3> {
        let input = EdgeBackoffInput {
            movement,
            bounds: self.player.bounds(),
            // Player.maybeBackOffFromEdge: not while flying.
            flying: self.player.flying,
            self_or_player: matches!(mover, MoverType::SelfMovement | MoverType::Player),
            fall_distance: self.player.fall_distance,
            query_budget: 1_000_000,
        };
        edge_backoff::back_off_from_edge(self, input)
            .map_err(|e| io::Error::other(e.to_string()).into())
    }
    fn collide(&mut self, movement: Vec3) -> Result<Vec3> {
        let max_step = self.player.attribute("minecraft:step_height")? as f32;
        Ok(world_movement::resolve_world_movement(
            &mut self.movement_collisions(),
            MovementRequest {
                entity: EntityIdentity(0),
                context: self.context(),
                bounds: self.player.bounds(),
                movement,
                max_step,
                on_ground: self.player.ground.on_ground,
                candidate_budget: 1_000_000,
            },
        )?)
    }
    fn fall_distance(&self) -> f64 {
        self.player.fall_distance
    }
    fn fall_reset_clip(&mut self, from: Vec3, to: Vec3) -> Result<bool> {
        // ClipContext.Block.FALLDAMAGE_RESETTING: tagged reset blocks are a full
        // block, not their collider; ClipContext.Fluid.WATER: the water's shape.
        // Both shapes are queried for every visited block (the water's first
        // query fixes its cached shape), either one hitting ends the ray.
        let hit = traverse_blocks(from, to, 1_000_000, |p| {
            let water = match self.water_clip(from, to, p) {
                Ok(hit) => hit,
                Err(e) => return Some(Err(e)),
            };
            let name = &self.assets.registry.state(self.block(p)).unwrap().name;
            let reset = self.resources.fall_reset.contains(name)
                && VoxelShape::block().clip(from, to, p).is_some();
            (water || reset).then_some(Ok(()))
        })?;
        hit.transpose().map(|h| h.is_some())
    }
    fn reset_fall_distance(&mut self) -> Result<()> {
        self.player.fall_distance = 0.0;
        Ok(())
    }
    fn movement_history(&mut self) -> &mut MovementHistory {
        &mut self.player.history
    }
    fn locally_authoritative(&mut self) -> Result<bool> {
        Ok(!self.player.server_copy)
    }
    fn set_on_ground_with_movement(
        &mut self,
        below: bool,
        horizontal: bool,
        movement: Vec3,
    ) -> Result<()> {
        let position = self.player.position;
        let bounds = self.player.bounds();
        let context = self.context();
        let mut collisions = SceneCollisions {
            world: self.scene,
            binding: self.collision_binding,
            subject: Some(self.player.powder_snow_subject()),
        };
        self.player.ground.set_with_movement(
            below,
            horizontal,
            bounds,
            Some(movement),
            |bounds| find_supporting_block(&mut collisions, context, position, bounds, 1_000_000),
        )?;
        Ok(())
    }
    fn minor_horizontal_collision(&mut self, _: Vec3) -> Result<bool> {
        Ok(false)
    }
    fn legacy_on_position(&mut self) -> Result<BlockPos> {
        self.support(movement_support::LEGACY_SUPPORT_OFFSET)
    }
    fn block_state(&mut self, p: BlockPos) -> Result<BlockStateId> {
        Ok(self.block(p))
    }
    fn on_ground(&mut self) -> Result<bool> {
        Ok(self.player.ground.on_ground)
    }
    fn check_fall_damage(
        &mut self,
        dy: f64,
        grounded: bool,
        state: BlockStateId,
        position: BlockPos,
    ) -> Result<()> {
        // LivingEntity.aiStep: Slow Falling and Levitation reset the fall
        // distance every tick, so landing under either does not hurt.
        if self.player.status.slow_falling || self.player.status.levitation.is_some() {
            self.player.fall_distance = 0.0;
        }
        // LivingEntity.checkFallDamage: a landing runs onChangedBlock.
        if grounded && self.player.fall_distance > 0.0 {
            self.player.location_effects.landed = true;
        }
        if self.player.spawn_extra_particles_on_fall && grounded && self.player.fall_distance > 0.0
        {
            self.player.events.push(MotionEvent::SmashLanding {
                state,
                position,
                fall_distance: self.player.fall_distance,
            });
            self.player.spawn_extra_particles_on_fall = false;
        }
        self.player.events.push(MotionEvent::Landing {
            dy,
            grounded,
            state,
            position,
        });
        horizon_world::living_landing::check_living_fall_damage(self, dy, grounded, state, position)
    }
    fn removed(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn can_simulate(&mut self) -> Result<bool> {
        Ok(true)
    }
    fn restitute(&mut self, state: BlockStateId, x: bool, z: bool, resolved: Vec3) -> Result<()> {
        let input = RestitutionInput {
            block: state,
            collided_x: x,
            collided_z: z,
            vertical_collision: self.player.collision.vertical,
            collision_below: self.player.collision.below,
            living: true,
            resolved_movement: resolved,
            fall_distance: self.player.fall_distance,
        };
        movement_restitution::restitute_movement(self, input)
    }
    fn client_side(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn movement_emission(&mut self) -> Result<MovementEmission> {
        <Self as MovementEmissionWorld>::movement_emission(self)
    }
    fn passenger(&mut self) -> Result<bool> {
        Ok(false)
    }
    /// `Entity.applyMovementEmissionAndPlaySound`: a footstep (`Emission`)
    /// only once the walked distance passes `nextStep`. Emitting on every
    /// move played a step sound each tick, even standing still on the
    /// ground (gravity moves the player every tick), as an endless loop.
    fn apply_movement_emission(
        &mut self,
        kind: MovementEmission,
        movement: Vec3,
        position: BlockPos,
        state: BlockStateId,
    ) -> Result<()> {
        emission::apply_movement_emission(self, kind, movement, position, state)
    }
    fn block_speed_factor(&mut self) -> Result<f32> {
        let efficiency = self.player.attribute("minecraft:movement_efficiency")? as f32;
        let location = self.location();
        let block = movement_support::block_speed_factor(
            &mut RegistryMovementWorld {
                properties: &self.resources.blocks,
                world: self,
            },
            location,
        )?;
        Ok(block + efficiency * (1.0 - block))
    }
}
// The player's footsteps and swim sounds. Flapping and the step game event
// are not bound here (the sculk listeners track the player's steps themselves).
impl MovementEmissionWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    type State = BlockStateId;
    fn distances(&self) -> MovementDistances {
        self.player.steps
    }
    fn set_movement_distance(&mut self, value: f32) {
        self.player.steps.movement = value;
    }
    fn set_flight_distance(&mut self, value: f32) {
        self.player.steps.flight = value;
    }
    fn set_next_step(&mut self, value: f32) {
        self.player.steps.next_step = value;
    }
    fn on_position(&mut self) -> Result<BlockPos> {
        self.support(movement_support::ON_SUPPORT_OFFSET)
    }
    fn block_state(&mut self, p: BlockPos) -> Result<BlockStateId> {
        Ok(self.block(p))
    }
    fn is_air(&mut self, s: BlockStateId) -> Result<bool> {
        let name = &self
            .assets
            .registry
            .state(s)
            .ok_or("missing scene state")?
            .name;
        Ok(matches!(
            name.as_str(),
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
        ))
    }
    fn climbable_tag(&mut self, s: BlockStateId) -> Result<bool> {
        let name = &self
            .assets
            .registry
            .state(s)
            .ok_or("missing scene state")?
            .name;
        Ok(self.resources.climbable.contains(name))
    }
    fn is_powder_snow(&mut self, s: BlockStateId) -> Result<bool> {
        let name = &self
            .assets
            .registry
            .state(s)
            .ok_or("missing scene state")?
            .name;
        Ok(name == "minecraft:powder_snow")
    }
    fn on_ground(&mut self) -> Result<bool> {
        Ok(self.player.ground.on_ground)
    }
    fn crouching(&mut self) -> Result<bool> {
        Ok(self.player.pose == crate::pose::Pose::Crouching)
    }
    fn on_rails(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn swimming(&mut self) -> Result<bool> {
        Ok(self.player.swimming)
    }
    fn in_water(&mut self) -> Result<bool> {
        Ok(self.player.fluids.was_touching_water)
    }
    /// `Player.playStepSound`: in water a swim sound, then the step muffled;
    /// on land the step of an inside-sound block above the floor, or of a
    /// combination block above it with the floor's step muffled.
    fn walking_step_sound(&mut self, position: BlockPos, state: BlockStateId) -> Result<()> {
        let muffled = self.player.fluids.was_touching_water;
        let mut stepped = false;
        if muffled {
            self.water_swim_sound()?;
        } else {
            // Entity.getPrimaryStepSoundBlockPos.
            let above = BlockPos {
                y: position.y + 1,
                ..position
            };
            let above_state = self.block(above);
            let name = &self
                .assets
                .registry
                .state(above_state)
                .ok_or("missing scene state")?
                .name;
            let combination = self.resources.combination_step_sound.contains(name);
            if combination || self.resources.inside_step_sound.contains(name) {
                self.player.events.push(MotionEvent::Emission {
                    position: above,
                    state: above_state,
                    muffled: false,
                });
                if combination {
                    // Entity.playCombinationStepSounds.
                    self.player.events.push(MotionEvent::Emission {
                        position,
                        state,
                        muffled: true,
                    });
                }
                stepped = true;
            }
        }
        if !stepped {
            self.player.events.push(MotionEvent::Emission {
                position,
                state,
                muffled,
            });
        }
        // Entity.walkingStepSound: the amethyst chime after the step, by the
        // floor's state even when the step sound was the block above's.
        let name = &self
            .assets
            .registry
            .state(state)
            .ok_or("missing scene state")?
            .name;
        let tick = self.player.health.tick_count;
        if self.resources.crystal_sound.contains(name)
            && tick >= self.player.last_crystal_sound_play_tick + 20
        {
            // Entity.playAmethystStepSound (the level random stands in for
            // the entity's).
            let elapsed = f64::from(tick - self.player.last_crystal_sound_play_tick);
            let decayed = self.player.crystal_sound_intensity * 0.997_f64.powf(elapsed) as f32;
            let intensity = 1.0_f32.min(decayed + 0.07);
            self.player.crystal_sound_intensity = intensity;
            let pitch = 0.5 + intensity * self.scene.random.next_float() * 1.2;
            let volume = 0.1 + intensity * 1.2;
            self.player
                .events
                .push(MotionEvent::Chime { volume, pitch });
            self.player.last_crystal_sound_play_tick = tick;
        }
        Ok(())
    }
    fn step_event(&mut self, _: BlockStateId) -> Result<()> {
        Ok(())
    }
    fn next_step(&mut self) -> Result<f32> {
        Ok(self.player.steps.next_step())
    }
    /// `Entity.waterSwimSound` and `playSwimSound` (the level random stands
    /// in for the entity's, as for the splash).
    fn water_swim_sound(&mut self) -> Result<()> {
        let v = self.player.velocity;
        let volume = 1.0_f32.min(
            (v.x * v.x * f64::from(0.2_f32) + v.y * v.y + v.z * v.z * f64::from(0.2_f32)).sqrt()
                as f32
                * 0.35,
        );
        let random = &mut self.scene.random;
        let pitch = 1.0 + (random.next_float() - random.next_float()) * 0.4;
        self.player.events.push(MotionEvent::Swim { volume, pitch });
        Ok(())
    }
    fn entity_event(&mut self, _: MovementEvent) -> Result<()> {
        Ok(())
    }
    fn flapping(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn on_flap(&mut self) -> Result<()> {
        Ok(())
    }
    /// `Player.getMovementEmission`: nothing while flying or sneaking on
    /// the ground.
    fn movement_emission(&mut self) -> Result<MovementEmission> {
        let discreet = self.player.ground.on_ground && self.player.shift_key_down;
        Ok(if self.player.flying || discreet {
            MovementEmission::None
        } else {
            MovementEmission::All
        })
    }
}
impl RestitutionWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn suppressing_bounce(&mut self) -> Result<bool> {
        Ok(self.player.shift_key_down)
    }
    fn entity_bounciness(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:bounciness")
    }
    fn velocity(&mut self) -> Result<Vec3> {
        Ok(self.player.velocity)
    }
    fn effective_gravity(&mut self) -> Result<f64> {
        living_motion::effective_gravity(self)
    }
    fn block_suppresses_bounce(&mut self, state: BlockStateId) -> Result<bool> {
        Ok(self.resources.blocks.get(state)?.suppresses_bounce)
    }
    fn block_bounce_restitution(&mut self, state: BlockStateId) -> Result<f32> {
        Ok(self
            .resources
            .blocks
            .get(state)?
            .coefficients
            .bounce_restitution)
    }
    fn air_drag(&mut self) -> Result<f32> {
        living_motion::air_drag(self)
    }
    fn movement_support_position(&mut self) -> Result<BlockPos> {
        self.support(movement_support::MOVEMENT_SUPPORT_OFFSET)
    }
    fn bounce_on(
        &mut self,
        state: BlockStateId,
        position: BlockPos,
        fall_distance: f64,
    ) -> Result<()> {
        self.player.events.push(MotionEvent::Bounce {
            state,
            position,
            fall_distance,
        });
        self.resources
            .landing
            .bounce_on(
                self,
                state,
                position,
                horizon_world::block_landing::LandingEntityKind::Player,
            )
            .map_err(|e| e.to_string().into())
    }
    fn bounce_event(&mut self) -> Result<()> {
        Ok(())
    }
    fn mark_position_for_sync(&mut self) {
        self.player.sync_position = true;
    }
    fn set_velocity(&mut self, value: Vec3) -> Result<()> {
        self.player.velocity = value;
        Ok(())
    }
}
impl LivingAirWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn movement_support_position(&mut self) -> Result<BlockPos> {
        self.support(movement_support::MOVEMENT_SUPPORT_OFFSET)
    }
    fn on_ground(&mut self) -> Result<bool> {
        Ok(self.player.ground.on_ground)
    }
    fn block_friction(&mut self, p: BlockPos) -> Result<f32> {
        Ok(self
            .resources
            .blocks
            .get(self.block(p))?
            .coefficients
            .friction)
    }
    fn friction_modifier(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:friction_modifier")
    }
    fn speed(&mut self) -> Result<f32> {
        Ok(self.player.attribute("minecraft:movement_speed")? as f32)
    }
    fn flying_speed(&mut self) -> Result<f32> {
        if self.player.flying {
            // Player.getFlyingSpeed: abilities speed, doubled when sprinting.
            return Ok(self.player.flying_speed * if self.player.sprinting { 2.0 } else { 1.0 });
        }
        Ok(if self.player.sprinting {
            0.025999999
        } else {
            0.02
        })
    }
    fn yaw(&mut self) -> Result<f32> {
        Ok(self.player.yaw)
    }
    fn velocity(&self) -> Vec3 {
        self.player.velocity
    }
    fn set_velocity(&mut self, v: Vec3) -> Result<()> {
        self.player.velocity = v;
        Ok(())
    }
    fn on_climbable(&mut self) -> Result<bool> {
        self.climbable()
    }
    fn reset_fall_distance(&mut self) -> Result<()> {
        self.player.fall_distance = 0.0;
        Ok(())
    }
    fn in_scaffolding(&mut self) -> Result<bool> {
        Ok(self
            .assets
            .registry
            .state(self.block(BlockPos::containing(self.player.position)))
            .unwrap()
            .name
            == "minecraft:scaffolding")
    }
    fn suppressing_ladder_slide(&mut self) -> Result<bool> {
        Ok(self.player.shift_key_down)
    }
    fn is_player_class(&self) -> bool {
        true
    }
    fn move_self(&mut self, v: Vec3) -> Result<()> {
        MotionWorld::move_self(self, v)
    }
    fn horizontal_collision(&self) -> bool {
        self.player.collision.horizontal
    }
    fn jumping(&self) -> bool {
        self.player.jumping
    }
    fn was_in_powder_snow(&self) -> bool {
        self.player.was_in_powder_snow
    }
    fn can_walk_powder_snow(&mut self) -> Result<bool> {
        Ok(self.player.leather_boots)
    }
    fn levitation_amplifier(&mut self) -> Result<Option<i32>> {
        Ok(self.player.status.levitation)
    }
    fn is_client_side(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn has_chunk_at(&mut self, p: BlockPos) -> Result<bool> {
        Ok(self.scene.chunks.contains_key(&(p.x >> 4, p.z >> 4)))
    }
    fn effective_gravity(&mut self) -> Result<f64> {
        living_motion::effective_gravity(self)
    }
    fn y(&self) -> f64 {
        self.player.position.y
    }
    fn minimum_y(&mut self) -> Result<i32> {
        Ok(0)
    }
    fn discard_friction(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn air_drag_modifier(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:air_drag_modifier")
    }
    fn omnidirectional_air_mover(&mut self) -> Result<bool> {
        Ok(false)
    }
}
impl LivingMotionWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn velocity(&self) -> Vec3 {
        self.player.velocity
    }
    fn set_velocity(&mut self, v: Vec3) -> Result<()> {
        self.player.velocity = v;
        Ok(())
    }
    fn jump_strength(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:jump_strength")
    }
    fn block_jump_factor(&mut self) -> Result<f32> {
        let location = self.location();
        Ok(movement_support::block_jump_factor(
            &mut RegistryMovementWorld {
                properties: &self.resources.blocks,
                world: self,
            },
            location,
        )?)
    }
    fn has_jump_boost(&mut self) -> Result<bool> {
        Ok(self.player.status.jump_boost.is_some())
    }
    fn jump_boost_amplifier(&mut self) -> Result<i32> {
        self.player
            .status
            .jump_boost
            .ok_or_else(|| "absent jump boost".into())
    }
    fn sprinting(&mut self) -> Result<bool> {
        Ok(self.player.sprinting)
    }
    fn yaw(&mut self) -> Result<f32> {
        Ok(self.player.yaw)
    }
    fn mark_needs_sync(&mut self) {
        self.player.needs_sync = true;
    }
    fn no_gravity(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn gravity_attribute(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:gravity")
    }
    fn has_slow_falling(&mut self) -> Result<bool> {
        Ok(self.player.status.slow_falling)
    }
    fn controlling_passenger_is_player(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn speed(&mut self) -> Result<f32> {
        Ok(self.player.attribute("minecraft:movement_speed")? as f32)
    }
    fn omnidirectional_air_mover(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn air_drag_modifier(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:air_drag_modifier")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sword_sweep_requires_charge_ground_and_slow_horizontal_movement() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION)
            });
        if !root.join("movement-properties.nbt").exists() {
            eprintln!("skipped sword sweep regression: prepared resources unavailable");
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut p = PlayerMotion::new(&resources, Vec3::ZERO).unwrap();
        p.ground.on_ground = true;
        p.speed = 0.1;
        assert!(p.is_sweep_attack(1.0, false, true));
        assert!(!p.is_sweep_attack(0.9, false, true));
        assert!(!p.is_sweep_attack(1.0, true, true));
        assert!(!p.is_sweep_attack(1.0, false, false));
        p.sprinting = true;
        assert!(!p.is_sweep_attack(1.0, false, true));
        p.sprinting = false;
        p.ground.on_ground = false;
        assert!(!p.is_sweep_attack(1.0, false, true));
        p.ground.on_ground = true;
        p.velocity = Vec3::new(0.2, 10.0, 0.0);
        assert!(p.is_sweep_attack(1.0, false, true));
        p.velocity.z = 0.2;
        assert!(!p.is_sweep_attack(1.0, false, true));
        p.velocity = Vec3::new(f64::from(p.speed) * 2.5, 0.0, 0.0);
        assert!(!p.is_sweep_attack(1.0, false, true));
    }

    #[test]
    fn player_stands_walks_sneaks_and_jumps_on_ghast() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION)
            });
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene(&assets, &render, 1).unwrap();
        let entities = [Aabb::new(
            Vec3::new(10.0, 80.0, 10.0),
            Vec3::new(14.0, 84.0, 14.0),
        )];
        let mut player = PlayerMotion::new(&resources, Vec3::new(12.0, 84.0, 12.0)).unwrap();
        let mut tick = |player: &mut PlayerMotion, keys| {
            MotionWorld {
                entities: &entities,
                player,
                scene: &mut scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            }
            .controlled_simulation_tick(keys)
            .unwrap()
        };
        for _ in 0..20 {
            tick(&mut player, Default::default());
        }
        assert_eq!(player.position.y, 84.0);
        assert!(player.ground.on_ground);
        let forward = crate::sprint::Keys {
            forward: true,
            ..Default::default()
        };
        for _ in 0..5 {
            tick(&mut player, forward);
        }
        assert!(player.position.z > 12.0);
        assert_eq!(player.position.y, 84.0);
        tick(
            &mut player,
            crate::sprint::Keys {
                jump: true,
                ..Default::default()
            },
        );
        assert!(player.position.y > 84.0);
        for _ in 0..30 {
            tick(&mut player, Default::default());
        }
        assert_eq!(player.position.y, 84.0);
        for _ in 0..80 {
            tick(
                &mut player,
                crate::sprint::Keys {
                    shift: true,
                    ..forward
                },
            );
        }
        assert_eq!(player.position.y, 84.0);
        assert!(player.position.z < 14.3);
        for _ in 0..30 {
            tick(&mut player, forward);
        }
        assert!(player.position.y < 84.0);
    }

    /// `MovingPistonBlock.getCollisionShape`: a block halfway through a push
    /// up is solid at its moving position, so a player lands on it.
    #[test]
    fn a_player_lands_on_a_block_a_piston_is_moving() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene(&assets, &render, 1).unwrap();
        let registry = assets.registry;
        let moving = registry.default_state("minecraft:moving_piston").unwrap();
        let stone = registry.default_state("minecraft:stone").unwrap();
        scene.set(20, 4, 20, moving).unwrap();
        scene.moving_pistons.push((
            [20, 4, 20],
            horizon_world::piston_shape::MovingPistonState {
                moved_state: stone,
                direction: horizon_core::geometry::Direction::Up,
                extending: true,
                source: false,
                progress: 0.5,
                previous_progress: 0.0,
            },
        ));
        let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 6.0, 20.5)).unwrap();
        for _ in 0..30 {
            MotionWorld {
                entities: &[],
                player: &mut player,
                scene: &mut scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            }
            .controlled_simulation_tick(Default::default())
            .unwrap();
        }
        assert!(player.ground.on_ground);
        assert_eq!(player.position.y, 4.5);
        // Once the block entity is gone, the moving_piston block has no shape.
        scene.moving_pistons.clear();
        for _ in 0..10 {
            MotionWorld {
                entities: &[],
                player: &mut player,
                scene: &mut scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            }
            .controlled_simulation_tick(Default::default())
            .unwrap();
        }
        assert_eq!(player.position.y, 4.0);
    }

    #[test]
    fn a_lava_cauldron_burns_the_player_inside_it() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION)
            });
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene(&assets, &render, 1).unwrap();
        let cauldron = assets
            .registry
            .default_state("minecraft:lava_cauldron")
            .unwrap();
        scene.set(20, 4, 20, cauldron).unwrap();
        let mut tick = |player: &mut PlayerMotion| {
            MotionWorld {
                entities: &[],
                player,
                scene: &mut scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            }
            .controlled_simulation_tick(Default::default())
            .unwrap();
        };
        // Standing on the cauldron's floor, in its lava.
        let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 4.25, 20.5)).unwrap();
        player.ticks_frozen = 40;
        tick(&mut player);
        assert!(player.health.health() < 20.0, "the lava must hurt");
        assert!(player.fire_ticks > 0, "the lava must ignite");
        assert_eq!(player.ticks_frozen, 0);
        // Beside it, nothing.
        let mut away = PlayerMotion::new(&resources, Vec3::new(22.5, 4.0, 20.5)).unwrap();
        tick(&mut away);
        assert_eq!(away.health.health(), 20.0);
        assert!(away.fire_ticks <= 0);
    }

    #[test]
    fn a_water_cauldron_puts_out_a_burning_player() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION)
            });
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene(&assets, &render, 1).unwrap();
        let water = assets
            .registry
            .resolve(&horizon_world::chunk::BlockState {
                name: "minecraft:water_cauldron".into(),
                properties: [("level".to_owned(), "3".to_owned())].into(),
            })
            .unwrap();
        scene.set(20, 4, 20, water).unwrap();
        let mut tick = |player: &mut PlayerMotion| {
            MotionWorld {
                entities: &[],
                player,
                scene: &mut scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            }
            .controlled_simulation_tick(Default::default())
            .unwrap();
        };
        let asks = |player: &PlayerMotion| {
            player
                .events
                .iter()
                .filter(|e| matches!(e, MotionEvent::CauldronExtinguish { .. }))
                .count()
        };
        // Burning, on the cauldron's floor in its water: put out, and the
        // server is asked to lower the water.
        let mut burning = PlayerMotion::new(&resources, Vec3::new(20.5, 4.25, 20.5)).unwrap();
        burning.fire_ticks = 100;
        tick(&mut burning);
        assert!(burning.fire_ticks <= 0, "the water must put the fire out");
        assert_eq!(asks(&burning), 1);
        // Not burning: nothing to lower.
        let mut dry = PlayerMotion::new(&resources, Vec3::new(20.5, 4.25, 20.5)).unwrap();
        tick(&mut dry);
        assert_eq!(asks(&dry), 0);
    }

    #[test]
    fn standing_in_a_wither_rose_asks_for_its_effect() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION)
            });
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene(&assets, &render, 1).unwrap();
        let rose = assets
            .registry
            .default_state("minecraft:wither_rose")
            .unwrap();
        scene.set(20, 4, 20, rose).unwrap();
        let roses = |player: &PlayerMotion| {
            player
                .events
                .iter()
                .filter(|e| matches!(e, MotionEvent::InsideWitherRose))
                .count()
        };
        let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
        MotionWorld {
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets: &assets,
            collision_binding: &binding,
            resources: &resources,
        }
        .controlled_simulation_tick(Default::default())
        .unwrap();
        assert_eq!(roses(&player), 1);
        // A server's player: its client moved it, the server applies the effect.
        player.events.clear();
        let from = player.position;
        MotionWorld {
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets: &assets,
            collision_binding: &binding,
            resources: &resources,
        }
        .server_block_damage(from)
        .unwrap();
        assert_eq!(roses(&player), 1);
        // Two blocks away, nothing.
        let mut away = PlayerMotion::new(&resources, Vec3::new(22.5, 4.0, 20.5)).unwrap();
        MotionWorld {
            entities: &[],
            player: &mut away,
            scene: &mut scene,
            assets: &assets,
            collision_binding: &binding,
            resources: &resources,
        }
        .controlled_simulation_tick(Default::default())
        .unwrap();
        assert_eq!(roses(&away), 0);
    }

    #[test]
    fn server_travel_moves_only_the_velocity() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene(&assets, &render, 1).unwrap();
        let at = Vec3::new(20.5, 4.0, 20.5);
        let mut player = PlayerMotion::new(&resources, at).unwrap();
        let mut travel = |player: &mut PlayerMotion| {
            MotionWorld {
                entities: &[],
                player,
                scene: &mut scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            }
            .server_travel()
            .unwrap()
        };
        // A knockback on the ground (vanilla's server sends 0.21840 0.27516
        // after a zombie's hit): ground friction, then gravity and drag.
        player.ground.on_ground = true;
        player.velocity = Vec3::new(-0.4, 0.3608, 0.0);
        travel(&mut player);
        assert!(
            (player.velocity.x + 0.2184).abs() < 1e-6,
            "{:?}",
            player.velocity
        );
        assert!(
            (player.velocity.y - 0.275184).abs() < 1e-6,
            "{:?}",
            player.velocity
        );
        assert_eq!(player.position, at);
        assert!(player.ground.on_ground);
        // Falling onto the ground hurts no one on the server's copy, and leaves
        // the fall to the client's moves.
        let health = player.health.health();
        player.position = Vec3::new(20.5, 4.5, 20.5);
        player.ground.on_ground = false;
        player.fall_distance = 10.0;
        player.velocity = Vec3::new(0.0, -1.0, 0.0);
        travel(&mut player);
        assert_eq!(player.velocity.y, -0.0784000015258789);
        assert_eq!(player.health.health(), health);
        assert_eq!(player.fall_distance, 10.0);
        assert!(player.events.is_empty());
    }

    #[test]
    fn spectator_scroll_speed_controls_horizontal_and_vertical_flight() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let run = |steps| {
            let mut scene = World::test_scene(&assets, &render, 2).unwrap();
            let start = Vec3::new(20.5, 12.0, 20.5);
            let mut player = PlayerMotion::new(&resources, start).unwrap();
            player.spectator = true;
            player.flying_speed = crate::mouse::spectator_flying_speed(player.flying_speed, steps);
            MotionWorld {
                entities: &[],
                player: &mut player,
                scene: &mut scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            }
            .controlled_simulation_tick(crate::sprint::Keys {
                forward: true,
                jump: true,
                ..Default::default()
            })
            .unwrap();
            player.position.subtract(start)
        };
        let stopped = run(-100);
        assert_eq!(stopped, Vec3::ZERO);
        let normal = run(0);
        let fast = run(10);
        assert!(normal.y > 0.0 && normal.z > 0.0);
        assert!((fast.y - normal.y * 2.0).abs() < 1e-6);
        assert!((fast.z - normal.z * 2.0).abs() < 1e-6);
    }

    #[test]
    fn resident_spectator_flight_ignores_solid_and_climbable_blocks() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let run = |name: &str, spectator: bool| {
            let mut scene = World::test_scene(&assets, &render, 2).unwrap();
            let state = assets.registry.default_state(name).unwrap();
            for x in 19..=23 {
                for y in 10..=17 {
                    for z in 19..=23 {
                        scene.set(x, y, z, state).unwrap();
                    }
                }
            }
            let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 12.0, 20.5)).unwrap();
            player.spectator = spectator;
            player.flying = spectator;
            player.velocity = Vec3::new(0.4, 0.1, 0.4);
            let keys = crate::sprint::Keys {
                forward: true,
                jump: true,
                ..Default::default()
            };
            let mut trajectory = Vec::new();
            for _ in 0..4 {
                MotionWorld {
                    entities: &[],
                    player: &mut player,
                    scene: &mut scene,
                    assets: &assets,
                    collision_binding: &binding,
                    resources: &resources,
                }
                .controlled_simulation_tick(keys)
                .unwrap();
                trajectory.push((player.position, player.velocity));
            }
            trajectory
        };
        let air = run("minecraft:air", true);
        for name in [
            "minecraft:stone",
            "minecraft:ladder",
            "minecraft:vine",
            "minecraft:scaffolding",
        ] {
            assert_eq!(run(name, true), air, "spectator flight in {name}");
        }
        let air = run("minecraft:air", false);
        let ladder = run("minecraft:ladder", false);
        assert!(air.last().unwrap().1.y < 0.0);
        assert!(ladder.last().unwrap().1.y > 0.0);
    }

    #[test]
    fn creative_and_spectator_preserve_hunger_when_switching_from_survival() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION)
            });
        if !root.join("movement-properties.nbt").exists() {
            eprintln!("skipped hunger regression: prepared resources unavailable");
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        // Both abilities are set by creative/spectator mode changes for the
        // resident player and for LAN/dedicated guests.
        for spectator in [false, true] {
            for food in [0, 1, 20] {
                let mut player = PlayerMotion::new(&resources, Vec3::ZERO).unwrap();
                player.health.food.food = food;
                player.health.food.saturation = 0.0;
                player.health.food.exhaustion = 0.0;
                player.health.food.timer = 79;
                player.invulnerable = true;
                player.spectator = spectator;
                for _ in 0..160 {
                    player.cause_food_exhaustion(0.2);
                    player
                        .food_tick(&resources.health, Default::default())
                        .unwrap();
                }
                assert_eq!(player.health.food.food, food);
                assert_eq!(player.health.food.saturation, 0.0);
                assert_eq!(player.health.food.exhaustion, 0.0);
                assert_eq!(player.health.health(), 20.0);
                assert!(!player.health.dead_or_dying());
                assert!(player.events.is_empty());

                // Pending exhaustion from survival is still processed, but an
                // empty food bar cannot hurt the now-invulnerable player.
                player.health.food.exhaustion = 5.0;
                player.health.food.timer = 79;
                player
                    .food_tick(&resources.health, Default::default())
                    .unwrap();
                assert_eq!(player.health.food.food, (food - 1).max(0));
                assert_eq!(player.health.food.exhaustion, 1.0);
                assert_eq!(player.health.health(), 20.0);
                assert!(player.events.is_empty());

                // Returning to survival resumes exhaustion and starvation.
                player.health.food.food = food;
                player.health.food.exhaustion = 0.0;
                player.invulnerable = false;
                player.spectator = false;
                player.cause_food_exhaustion(0.2);
                assert_eq!(player.health.food.exhaustion, 0.2);
                player.health.food.exhaustion = 5.0;
                player.health.food.timer = 79;
                player
                    .food_tick(&resources.health, Default::default())
                    .unwrap();
                assert_eq!(player.health.food.food, (food - 1).max(0));
                assert_eq!(player.health.health(), if food <= 1 { 19.0 } else { 20.0 });
            }
        }
    }

    #[test]
    fn player_fire_damage_queues_hurt_sound_only_for_fresh_hits() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION)
            });
        if !root.join("movement-properties.nbt").exists() {
            eprintln!("skipped damage regression: prepared resources unavailable");
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        for name in ["on_fire", "in_fire", "lava"] {
            let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
            let source = resources
                .health
                .damage
                .id(&format!("minecraft:{name}"))
                .unwrap();
            assert!(
                player
                    .hurt(&resources.health, Default::default(), source, 1.0)
                    .unwrap()
            );
            assert_eq!(
                player
                    .events
                    .iter()
                    .filter(|e| matches!(
                        e,
                        MotionEvent::HurtSound("minecraft:entity.player.hurt_on_fire")
                    ))
                    .count(),
                1
            );
            assert_eq!(player.health.hurt_animation_time, 10);
            player.health.advance_clock();
            player.events.clear();
            assert!(
                !player
                    .hurt(&resources.health, Default::default(), source, 1.0)
                    .unwrap()
            );
            assert!(
                !player
                    .events
                    .iter()
                    .any(|e| matches!(e, MotionEvent::HurtSound(_)))
            );
            assert_eq!(
                player.health.hurt_animation_time, 9,
                "cooldown hit must not restart animation"
            );
            for _ in 0..20 {
                player.health.advance_clock();
            }
            assert_eq!(player.health.hurt_animation_time, 0);
            player.invulnerable = true;
            player.events.clear();
            assert!(
                !player
                    .hurt(&resources.health, Default::default(), source, 1.0)
                    .unwrap()
            );
            assert!(
                !player
                    .events
                    .iter()
                    .any(|e| matches!(e, MotionEvent::HurtSound(_)))
            );
        }
        let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
        let source = resources.health.damage.id("minecraft:mob_attack").unwrap();
        assert!(
            player
                .hurt_blocking(&resources.health, Default::default(), source, 2.0, None)
                .unwrap()
                .hurt
        );
        assert_eq!(
            player
                .events
                .iter()
                .filter(|e| matches!(e, MotionEvent::HurtSound("minecraft:entity.player.hurt")))
                .count(),
            1
        );
        // A shield that takes the whole arrow: onBlocked's sound, no damage
        // event, so the player hears no hurt sound.
        use horizon_world::blocking_rules::{BlockingRules, DamageReduction, ItemDamageFunction};
        let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
        let source = resources.health.damage.id("minecraft:arrow").unwrap();
        let shield = crate::health::Blocking {
            rules: BlockingRules {
                delay_seconds: 0.25,
                disable_scale: 1.0,
                reductions: vec![DamageReduction {
                    horizontal_angle: 90.0,
                    types: None,
                    base: 0.0,
                    factor: 1.0,
                }],
                item_damage: ItemDamageFunction {
                    threshold: 3.0,
                    base: 1.0,
                    factor: 1.0,
                },
            },
            bypassed: false,
            pierce: false,
            angle: 0.0,
        };
        let outcome = player
            .hurt_blocking(
                &resources.health,
                Default::default(),
                source,
                4.0,
                Some(shield.clone()),
            )
            .unwrap();
        assert!(!outcome.hurt && outcome.sound && outcome.blocked == 4.0);
        assert_eq!(
            player.health.hurt.hurt_time, 10,
            "preserve server damage bookkeeping"
        );
        assert_eq!(
            player.health.hurt_animation_time, 0,
            "a fully blocked arrow must not animate hurt"
        );
        assert!(
            !player
                .events
                .iter()
                .any(|e| matches!(e, MotionEvent::HurtSound(_)))
        );
        // An unblocked hit after the shield block must still animate normally.
        for _ in 0..20 {
            player.health.advance_clock();
        }
        assert!(
            player
                .hurt(&resources.health, Default::default(), source, 4.0)
                .unwrap()
        );
        assert_eq!(player.health.hurt_animation_time, 10);
        player.health.advance_clock();
        assert!(
            !player
                .hurt_blocking(
                    &resources.health,
                    Default::default(),
                    source,
                    4.0,
                    Some(shield)
                )
                .unwrap()
                .hurt
        );
        assert_eq!(
            player.health.hurt_animation_time, 9,
            "blocking must not erase or restart an existing animation"
        );
        let mut starving = PlayerMotion::new(&resources, Vec3::ZERO).unwrap();
        starving.health.food.food = 0;
        starving.health.food.timer = 79;
        starving
            .food_tick(&resources.health, Default::default())
            .unwrap();
        assert_eq!(starving.health.health(), 19.0);
        assert_eq!(
            starving.health.hurt_animation_time, 10,
            "starvation must still animate hurt"
        );
    }

    #[test]
    fn a_glide_that_is_not_diving_keeps_the_fall_distance_at_one() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        // Entity.checkFallDistanceAccumulation: capped above -0.5 a tick, not below.
        for (dy, capped) in [(-0.3, true), (-0.49, true), (-0.8, false)] {
            let mut scene = World::test_scene(&assets, &render, 2).unwrap();
            let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 20.0, 20.5)).unwrap();
            player.glider = true;
            player.fall_flying = true;
            player.velocity = Vec3::new(0.0, dy, 0.6);
            player.fall_distance = 30.0;
            let mut w = MotionWorld {
                entities: &[],
                player: &mut player,
                scene: &mut scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            };
            w.player_movement_tick([0.0, 0.0], false).unwrap();
            assert!(w.player.fall_flying);
            if capped {
                assert!(
                    w.player.fall_distance < 2.0,
                    "{dy}: {}",
                    w.player.fall_distance
                );
            } else {
                assert!(
                    w.player.fall_distance > 30.0,
                    "{dy}: {}",
                    w.player.fall_distance
                );
            }
        }
    }
    /// Footsteps on the prepared vanilla resources, when they are present:
    /// none while standing (gravity still moves the player every tick), one
    /// per `nextStep` metre while walking.
    #[test]
    fn resident_water_landing_cancels_a_preset_fall() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        for wet in [false, true] {
            let mut scene = World::test_scene(&assets, &render, 2).unwrap();
            if wet {
                let water = assets.registry.default_state("minecraft:water").unwrap();
                for x in 19..=21 {
                    for z in 19..=21 {
                        scene.set(x, 3, z, water).unwrap();
                    }
                }
            }
            let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 20.0, 20.5)).unwrap();
            player.fall_distance = 30.0;
            let mut w = MotionWorld {
                entities: &[],
                player: &mut player,
                scene: &mut scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            };
            w.move_self(Vec3::new(0.0, -30.0, 0.0)).unwrap();
            assert!(w.player.ground.on_ground);
            assert_eq!(w.player.fluids.was_touching_water, wet);
            if wet {
                assert_eq!(w.player.health.health(), 20.0);
                assert_eq!(w.player.fall_distance, 0.0);
            } else {
                assert!(w.player.health.health() < 20.0);
            }
        }
    }
    /// A client's own fall (multiplayer, a LAN guest, single player on its
    /// server thread) plays its landing but never hurts it: only the server
    /// does (`Entity.hurt` on a client), so a server that cancels fall damage
    /// leaves the screen still.
    #[test]
    fn a_clients_own_fall_does_not_hurt_it() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        for client_side in [false, true] {
            let mut scene = World::test_scene(&assets, &render, 2).unwrap();
            let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 20.0, 20.5)).unwrap();
            player.client_side = client_side;
            player.fall_distance = 30.0;
            let mut w = MotionWorld {
                entities: &[],
                player: &mut player,
                scene: &mut scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            };
            w.move_self(Vec3::new(0.0, -30.0, 0.0)).unwrap();
            assert!(w.player.ground.on_ground);
            assert_eq!(w.player.fall_distance, 0.0);
            // LivingEntity.causeFallDamage's sounds play on both sides.
            assert!(w.player.events.iter().any(|e| matches!(
                e,
                MotionEvent::Fall(crate::landing::FallEvent::Sound {
                    sound: crate::landing::SceneSound::BigFall,
                    ..
                })
            )));
            let hurt = w.player.health.health() < 20.0;
            assert_eq!(hurt, !client_side);
            assert_eq!(w.player.health.hurt_animation_time > 0, !client_side);
            // The server's health then starts the client's hurt animation.
            if client_side {
                w.player.health.flash_health_change(20.0);
                assert_eq!(w.player.health.hurt_animation_time, 0);
                assert_eq!(w.player.health.hurt.cooldown, 0);
                w.player.health.flash_health_change(12.0);
                assert_eq!(w.player.health.hurt_animation_time, 10);
                // LocalPlayer.hurtTo's damageCooldownTime: the HUD's hearts blink.
                assert_eq!(w.player.health.hurt.cooldown, 20);
                w.player.health.flash_health_change(25.0);
                assert_eq!(w.player.health.hurt.cooldown, 10);
            }
        }
    }
    #[test]
    fn slow_falling_clears_the_fall_distance_so_landing_does_not_hurt() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        for slow_falling in [false, true] {
            let mut scene = World::test_scene(&assets, &render, 2).unwrap();
            let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 9.0, 20.5)).unwrap();
            player.fall_distance = 30.0;
            player.status.slow_falling = slow_falling;
            let mut w = MotionWorld {
                entities: &[],
                player: &mut player,
                scene: &mut scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            };
            for _ in 0..200 {
                if w.player.ground.on_ground {
                    break;
                }
                w.player_movement_tick([0.0, 0.0], false).unwrap();
            }
            assert!(w.player.ground.on_ground);
            if slow_falling {
                assert_eq!(w.player.health.health(), 20.0);
                assert_eq!(w.player.fall_distance, 0.0);
            } else {
                assert!(w.player.health.health() < 20.0);
            }
        }
    }
    #[test]
    fn a_server_copy_of_a_riptide_spin_counts_down_and_stands_again() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene(&assets, &render, 2).unwrap();
        let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 10.0, 20.5)).unwrap();
        player.auto_spin_attack_ticks = 2;
        player.auto_spin_attack_dmg = 8.0;
        let mut w = MotionWorld {
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets: &assets,
            collision_binding: &binding,
            resources: &resources,
        };
        w.update_player_pose().unwrap();
        assert_eq!(w.player.pose, crate::pose::Pose::SpinAttack);
        w.player.count_down_server_spin();
        w.update_player_pose().unwrap();
        assert_eq!(w.player.pose, crate::pose::Pose::SpinAttack);
        assert_eq!(w.player.auto_spin_attack_dmg, 8.0);
        w.player.count_down_server_spin();
        assert_eq!(w.player.auto_spin_attack_ticks, 0);
        assert_eq!(w.player.auto_spin_attack_dmg, 0.0);
        w.update_player_pose().unwrap();
        assert_eq!(w.player.pose, crate::pose::Pose::Standing);
        assert_eq!(w.player.dimensions.eye_height, 1.62);
        // Nothing to count once the spin is over.
        w.player.count_down_server_spin();
        assert_eq!(w.player.auto_spin_attack_ticks, 0);
    }
    /// #3684: `/attribute @s minecraft:scale base set 0.1` on the server's
    /// copy of a player. `LivingEntity.tick`'s `refreshDirtyAttributes` runs
    /// `onAttributeUpdated(SCALE)`, which calls `refreshDimensions`: the next
    /// tick the server checks the player's moves with a 0.06 × 0.18 box, not
    /// the 0.6 × 1.8 one that sent a small player back from every wall.
    #[test]
    fn a_server_copy_takes_its_new_scale_box_the_next_tick() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene(&assets, &render, 2).unwrap();
        let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 10.0, 20.5)).unwrap();
        let mut w = MotionWorld {
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets: &assets,
            collision_binding: &binding,
            resources: &resources,
        };
        w.server_player_pose_tick().unwrap();
        assert_eq!(w.player.dimensions.width, 0.6);
        assert_eq!(w.player.dimensions.height, 1.8);
        // AttributeCommand's base set, as the server runs it.
        let key = w.player.attributes.key("minecraft:scale").unwrap();
        let handle = w
            .player
            .attributes
            .get_instance(Some(key))
            .unwrap()
            .unwrap();
        w.player
            .attributes
            .edit(&handle, |value, observer| {
                value.set_base_value(0.1, observer)
            })
            .unwrap();
        w.server_player_pose_tick().unwrap();
        assert_eq!(w.player.pose, crate::pose::Pose::Standing);
        assert_eq!(w.player.dimensions.width, 0.6 * 0.1);
        assert_eq!(w.player.dimensions.height, 1.8 * 0.1);
        assert_eq!(w.player.dimensions.eye_height, 1.62 * 0.1);
    }
    /// #3308: a swimmer who boards a boat stops swimming
    /// (`Entity.updateSwimming` needs `!isPassenger()`) and `updatePlayerPose`
    /// sits them up, eye at 1.62 instead of the swim's 0.4.
    #[test]
    fn a_passenger_stops_swimming_and_stands() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene(&assets, &render, 2).unwrap();
        let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 10.0, 20.5)).unwrap();
        player.sprinting = true;
        player.swimming = true;
        player.set_pose(crate::pose::Pose::Swimming).unwrap();
        assert_eq!(player.dimensions.eye_height, 0.4);
        let mut w = MotionWorld {
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets: &assets,
            collision_binding: &binding,
            resources: &resources,
        };
        w.passenger_pose_tick().unwrap();
        assert!(!w.player.swimming);
        assert_eq!(w.player.pose, crate::pose::Pose::Standing);
        assert_eq!(w.player.dimensions.eye_height, 1.62);
    }
    #[test]
    fn a_riptide_spin_lies_flat_and_ends_on_the_first_entity_it_touches() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene(&assets, &render, 2).unwrap();
        let ahead = Aabb::new(Vec3::new(20.2, 9.0, 21.5), Vec3::new(20.8, 11.0, 22.1));
        for target in [None, Some(ahead)] {
            let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 10.0, 20.5)).unwrap();
            player.auto_spin_attack_ticks = 20;
            player.auto_spin_attack_dmg = 8.0;
            player.velocity = Vec3::new(0.0, 0.0, 1.0);
            player.spin_targets = target.map(|b| (7, b)).into_iter().collect();
            let mut w = MotionWorld {
                entities: &[],
                player: &mut player,
                scene: &mut scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            };
            w.simulation_tick([0.0, 0.0], false).unwrap();
            if target.is_some() {
                // Touched: the hit, no more spin, bounced back at a fifth.
                assert_eq!(w.player.spin_hit, Some(7));
                assert_eq!(w.player.auto_spin_attack_ticks, 0);
                assert!(w.player.velocity.z < 0.0, "{:?}", w.player.velocity);
                assert_eq!(w.player.pose, crate::pose::Pose::Standing);
            } else {
                assert_eq!(w.player.spin_hit, None);
                assert_eq!(w.player.auto_spin_attack_ticks, 19);
                assert_eq!(w.player.pose, crate::pose::Pose::SpinAttack);
                assert_eq!(w.player.dimensions.height, 0.6);
            }
        }
    }
    #[test]
    fn a_landing_glide_lasts_a_tick_so_a_jump_keeps_its_speed() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene(&assets, &render, 2).unwrap();
        let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 4.2, 20.5)).unwrap();
        let mut w = MotionWorld {
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets: &assets,
            collision_binding: &binding,
            resources: &resources,
        };
        w.move_self(Vec3::new(0.0, -1.0, 0.0)).unwrap();
        assert!(w.player.ground.on_ground);
        w.player.set_sprinting(true).unwrap();
        w.player.glider = true;
        w.player.fall_flying = true;
        w.player.velocity = Vec3::new(0.0, 0.0, 1.5);
        // The landed glide's extra tick: the jump's sprint boost, no ground friction.
        w.player_movement_tick([0.0, 1.0], true).unwrap();
        assert!(w.player.fall_flying);
        assert!(w.player.velocity.y > 0.3, "jumped: {:?}", w.player.velocity);
        assert!(
            w.player.velocity.z > 1.5,
            "kept its speed: {:?}",
            w.player.velocity
        );
        // Then the server's cleared flag arrives.
        w.player_movement_tick([0.0, 1.0], true).unwrap();
        assert!(!w.player.fall_flying);
    }

    #[test]
    fn resident_water_and_lava_are_not_treated_as_air() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION)
            });
        if !root.join("movement-properties.nbt").exists() {
            eprintln!("skipping resident water/lava test: prepared resources unavailable");
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene(&assets, &render, 3).unwrap();
        let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        for x in 19..=21 {
            for y in 3..=7 {
                for z in 19..=21 {
                    scene.set(x, y, z, water).unwrap();
                }
            }
        }
        let mut w = MotionWorld {
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets: &assets,
            collision_binding: &binding,
            resources: &resources,
        };
        w.controlled_simulation_tick(Default::default()).unwrap();
        assert!(
            MovementStatisticsWorld::in_water(&mut w).unwrap(),
            "resident water must reach the player movement adapter"
        );
        assert!(MovementStatisticsWorld::eyes_in_water(&mut w).unwrap());
        assert!(
            !w.player.fluids.was_eye_in_water,
            "the first controlled tick must run only one fluid base phase"
        );
        assert!(
            (w.player.velocity.y + 0.005).abs() < 1e-5,
            "water drag/gravity must replace air travel: {:?}",
            w.player.velocity
        );
        let lava = assets.registry.default_state("minecraft:lava").unwrap();
        for x in 19..=21 {
            for y in 3..=7 {
                for z in 19..=21 {
                    w.scene.set(x, y, z, lava).unwrap();
                }
            }
        }
        w.controlled_simulation_tick(Default::default()).unwrap();
        assert!(
            w.player.health.health() < 20.0,
            "lava must damage a survival player"
        );
        // Water extinguishes, swimming is a real pose, and deep water consumes air.
        for x in 19..=21 {
            for y in 3..=7 {
                for z in 19..=21 {
                    w.scene.set(x, y, z, water).unwrap();
                }
            }
        }
        w.player.fire_ticks = 160;
        w.controlled_simulation_tick(Default::default()).unwrap();
        assert!(w.player.fire_ticks <= 0);
        w.player.air = -19;
        w.player.health.hurt.cooldown = 0;
        let hp = w.player.health.health();
        w.controlled_simulation_tick(Default::default()).unwrap();
        assert_eq!(w.player.air, 0);
        assert!(w.player.health.health() < hp, "drowning must hurt");
        w.player.invulnerable = true;
        let hp = w.player.health.health();
        w.player.air = -19;
        w.controlled_simulation_tick(Default::default()).unwrap();
        assert_eq!(w.player.health.health(), hp);
        w.player.invulnerable = false;
        w.player.air = 300;
        w.controlled_simulation_tick(crate::sprint::Keys {
            forward: true,
            sprint: true,
            ..Default::default()
        })
        .unwrap();
        assert!(w.player.sprinting);
        assert!(
            !w.player.swimming,
            "sprint input follows the swimming phase"
        );
        w.controlled_simulation_tick(crate::sprint::Keys {
            forward: true,
            sprint: true,
            ..Default::default()
        })
        .unwrap();
        assert!(w.player.swimming);
        assert_eq!(w.player.pose, crate::pose::Pose::Swimming);
        // Direct fire contact ignites; resistance prevents damage but does not hide flames.
        let fire = assets.registry.default_state("minecraft:fire").unwrap();
        for x in 19..=21 {
            for y in 3..=7 {
                for z in 19..=21 {
                    w.scene.set(x, y, z, fire).unwrap();
                }
            }
        }
        w.player.position = Vec3::new(20.5, 4.0, 20.5);
        w.player.velocity = Vec3::ZERO;
        w.player.fire_ticks = -20;
        w.player.health.hurt.cooldown = 0;
        w.player.ticks_frozen = 140;
        let hp = w.player.health.health();
        w.controlled_simulation_tick(Default::default()).unwrap();
        assert!(
            w.player.fire_ticks < 0,
            "direct fire hurts during the immunity delay"
        );
        assert_eq!(
            w.player.ticks_frozen, 0,
            "fire contact clears freeze during the delay"
        );
        assert!(w.player.health.health() < hp);
        w.player.status.fire_resistance = true;
        w.player.health.hurt.cooldown = 0;
        let hp = w.player.health.health();
        w.controlled_simulation_tick(Default::default()).unwrap();
        assert_eq!(w.player.health.health(), hp);
        w.player.invulnerable = true;
        w.player.fire_ticks = -20;
        w.player.ticks_frozen = 140;
        w.controlled_simulation_tick(Default::default()).unwrap();
        assert_eq!(w.player.ticks_frozen, 0);
        assert_eq!(
            w.player.fire_ticks, -19,
            "creative preserves the fire-contact delay"
        );
        for _ in 0..18 {
            w.apply_fire_contact().unwrap();
        }
        assert_eq!(w.player.fire_ticks, -1);
        w.apply_fire_contact().unwrap();
        assert_eq!(
            w.player.fire_ticks, 1,
            "creative clamps only after ignition"
        );
        w.player.ticks_frozen = 140;
        w.player.ignite_for_ticks(160).unwrap();
        assert_eq!(w.player.ticks_frozen, 0);
        assert_eq!(w.player.fire_ticks, 1);
        w.player.invulnerable = false;
        // Sprint swimming must keep full input, not the dry crawl/sneak multiplier.
        for x in 19..=21 {
            for y in 2..=7 {
                for z in 19..=30 {
                    w.scene.set(x, y, z, water).unwrap();
                }
            }
        }
        w.player.position = Vec3::new(20.5, 4.0, 20.5);
        w.player.velocity = Vec3::ZERO;
        w.player.local_crouching = false;
        w.player.swimming = true;
        w.player.sprinting = true;
        w.player.set_pose(crate::pose::Pose::Swimming).unwrap();
        for _ in 0..30 {
            w.controlled_simulation_tick(crate::sprint::Keys {
                forward: true,
                sprint: true,
                ..Default::default()
            })
            .unwrap();
        }
        assert!(
            w.player.velocity.z > 0.165,
            "native sprint swim speed must not receive crawl slowdown: {:?}",
            w.player.velocity
        );
        assert!(!w.player.moving_slowly());
        w.player.position = Vec3::new(20.5, 4.0, 20.5);
        w.player.velocity = Vec3::ZERO;
        w.player.pitch = 45.0;
        w.controlled_simulation_tick(crate::sprint::Keys {
            forward: true,
            sprint: true,
            ..Default::default()
        })
        .unwrap();
        assert!(
            w.player.velocity.y < -0.04,
            "look down must steer a swimming player downward"
        );
        w.player.position = Vec3::new(20.5, 4.0, 20.5);
        w.player.velocity = Vec3::ZERO;
        w.player.pitch = -45.0;
        w.controlled_simulation_tick(crate::sprint::Keys {
            forward: true,
            sprint: true,
            ..Default::default()
        })
        .unwrap();
        assert!(
            w.player.velocity.y > 0.02,
            "look up must steer a submerged swimmer upward"
        );
        let extra = horizon_world::nbt::Tag::compound([
            ("Fire", horizon_world::nbt::Tag::Short(160)),
            ("Air", horizon_world::nbt::Tag::Short(8)),
        ]);
        let mut level = crate::save::Level {
            health: 13.0,
            food: 11,
            saturation: 2.25,
            player_extra: Some(extra),
            flying: true,
            ..Default::default()
        };
        let mut pending = true;
        w.player
            .load_saved_state_once(&mut pending, &level)
            .unwrap();
        assert_eq!((w.player.fire_ticks, w.player.air), (160, 8));
        assert!(w.player.flying);
        assert_eq!(
            (
                w.player.health.health(),
                w.player.health.food.food,
                w.player.health.food.saturation
            ),
            (13.0, 11, 2.25)
        );
        let mut respawn = PlayerMotion::new(&resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
        respawn.load_saved_state_once(&mut pending, &level).unwrap();
        assert_eq!((respawn.fire_ticks, respawn.air), (-20, 300));
        assert!(!respawn.flying);
        assert_eq!(
            (respawn.health.health(), respawn.health.food.food),
            (20.0, 20)
        );
        // Reopening another world restores its state, including a saved death.
        pending = true;
        level.health = 0.0;
        w.player
            .load_saved_state_once(&mut pending, &level)
            .unwrap();
        assert!(w.player.health.dead_or_dying());
        assert!(!pending);
        w.player.fluids.was_touching_water = false;
        assert!(
            w.player.moving_slowly(),
            "dry crawling must still slow input"
        );
    }

    #[test]
    fn resident_swimming_uses_previous_eyes_and_requires_water_at_the_feet() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene(&assets, &render, 3).unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        for y in 4..=6 {
            scene.set(20, y, 20, water).unwrap();
        }
        let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
        let mut w = MotionWorld {
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets: &assets,
            collision_binding: &binding,
            resources: &resources,
        };
        w.player.local_crouching = true;
        let sprint = crate::sprint::Keys {
            forward: true,
            sprint: true,
            ..Default::default()
        };
        w.fluid_base_tick().unwrap();
        assert!(w.player.is_eye_in_fluid(WATER));
        assert!(!w.player.is_under_water());
        w.player.sprint_input_tick(sprint).unwrap();
        assert!(
            !w.player.sprinting,
            "new eye immersion cannot bypass crouching"
        );
        w.fluid_base_tick().unwrap();
        assert!(w.player.is_under_water());
        w.player.sprint_input_tick(sprint).unwrap();
        assert!(w.player.sprinting);
        assert!(!w.player.swimming, "sprint starts after the base tick");
        w.fluid_base_tick().unwrap();
        assert!(w.player.swimming);
        let air = assets.registry.default_state("minecraft:air").unwrap();
        w.scene.set(20, 4, 20, air).unwrap();
        w.player.swimming = false;
        w.fluid_base_tick().unwrap();
        assert!(w.player.is_under_water());
        assert!(
            !w.player.swimming,
            "starting a swim requires water in the feet cell"
        );
        w.player.swimming = true;
        w.fluid_base_tick().unwrap();
        assert!(
            w.player.swimming,
            "an existing swim only requires touching water"
        );
        w.player.flying = true;
        w.fluid_base_tick().unwrap();
        assert!(!w.player.swimming);
        w.player.flying = false;
        w.player.spectator = true;
        w.scene.set(20, 4, 20, water).unwrap();
        w.fluid_base_tick().unwrap();
        assert!(
            w.player.swimming,
            "spectator flight is applied later in aiStep"
        );
    }

    #[test]
    fn resident_fire_precedes_first_tick_lava_and_water_extinguishing() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        for name in ["minecraft:lava", "minecraft:water"] {
            let mut scene = World::test_scene(&assets, &render, 3).unwrap();
            let fluid = assets.registry.default_state(name).unwrap();
            scene.set(20, 4, 20, fluid).unwrap();
            let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
            player.fire_ticks = 160;
            player.fall_distance = 8.0;
            let mut w = MotionWorld {
                entities: &[],
                player: &mut player,
                scene: &mut scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            };
            w.fluid_base_tick().unwrap();
            assert_eq!(
                w.player.health.health(),
                19.0,
                "base on-fire damage: {name}"
            );
            if name == "minecraft:lava" {
                assert_eq!(w.player.fire_ticks, 159);
                assert_eq!(w.player.fall_distance, 8.0, "first tick excludes lava");
                w.fluid_base_tick().unwrap();
                assert_eq!(
                    w.player.fall_distance, 4.0,
                    "later ticks halve exactly once"
                );
            } else {
                assert_eq!(w.player.fire_ticks, 0, "water extinguishes after the hurt");
                assert_eq!(w.player.fall_distance, 0.0);
                w.player.fire_ticks = -7;
                w.fluid_base_tick().unwrap();
                assert_eq!(
                    w.player.fire_ticks, -7,
                    "clearing fire retains negative delay"
                );
            }
        }
    }

    #[test]
    fn resident_bubble_columns_and_nautilus_preserve_their_air_rules() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene(&assets, &render, 3).unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        scene.set(20, 4, 20, water).unwrap();
        scene.set(20, 5, 20, water).unwrap();
        let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
        let mut w = MotionWorld {
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets: &assets,
            collision_binding: &binding,
            resources: &resources,
        };
        for (extra_effect, expected_air) in [
            (None, -19),
            (Some("water_breathing"), -15),
            (Some("conduit_power"), -15),
        ] {
            let mut effects = crate::effects::PlayerEffects::default();
            effects.add(crate::effects::EffectInstance::new(
                "breath_of_the_nautilus",
                200,
                0,
            ));
            if let Some(id) = extra_effect {
                effects.add(crate::effects::EffectInstance::new(id, 200, 0));
            }
            w.player.status = effects.status();
            assert!(w.player.status.breath_of_the_nautilus);
            w.player.air = -19;
            w.fluid_base_tick().unwrap();
            w.tick_environment_damage().unwrap();
            assert_eq!(w.player.air, expected_air);
            assert_eq!(w.player.health.health(), 20.0);
        }
        w.player.status.water_breathing = false;
        for (name, nautilus) in [
            ("minecraft:bubble_column", false),
            ("minecraft:bubble_column", true),
            ("minecraft:air", true),
        ] {
            w.scene
                .set(20, 5, 20, assets.registry.default_state(name).unwrap())
                .unwrap();
            w.player.status.breath_of_the_nautilus = nautilus;
            w.player.air = -19;
            w.fluid_base_tick().unwrap();
            w.tick_environment_damage().unwrap();
            assert_eq!(w.player.air, -15, "{name}, nautilus={nautilus}");
            assert_eq!(w.player.health.health(), 20.0);
        }
        w.scene.set(20, 5, 20, water).unwrap();
        w.player.status.breath_of_the_nautilus = false;
        w.player.air = -19;
        w.fluid_base_tick().unwrap();
        w.tick_environment_damage().unwrap();
        assert_eq!(w.player.air, 0);
        assert_eq!(w.player.health.health(), 18.0);
    }

    #[test]
    fn resident_mixed_fluids_use_lava_height_for_ground_jumps() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        for (water_level, lava_level, ground_jump) in [(0, 6, true), (6, 0, false)] {
            let mut scene = World::test_scene(&assets, &render, 3).unwrap();
            for (name, level, x) in [
                ("minecraft:water", water_level, 20),
                ("minecraft:lava", lava_level, 21),
            ] {
                let mut state = assets
                    .registry
                    .state(assets.registry.default_state(name).unwrap())
                    .unwrap()
                    .clone();
                state.properties.insert("level".into(), level.to_string());
                scene
                    .set(x, 4, 20, assets.registry.id(&state).unwrap())
                    .unwrap();
            }
            let mut player = PlayerMotion::new(&resources, Vec3::new(21.0, 4.0, 20.5)).unwrap();
            player.ground.on_ground = true;
            let mut w = MotionWorld {
                entities: &[],
                player: &mut player,
                scene: &mut scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            };
            w.fluid_base_tick().unwrap();
            assert!(w.player.fluids.was_touching_water && w.player.is_in_lava());
            assert_eq!(w.player.fluid_height(LAVA) <= 0.4, ground_jump);
            assert_eq!(w.player.fluid_height(WATER) > 0.4, ground_jump);
            w.dry_tick(TickInput::PreparedLocal {
                direction: [0.0; 2],
                jumping: true,
            })
            .unwrap();
            assert_eq!(w.player.no_jump_delay == 10, ground_jump);
        }
    }

    #[test]
    fn footsteps_follow_the_walked_distance_not_the_tick() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let mut scene = World::test_scene(&assets, &render, 3).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
        let mut steps = |player: &mut PlayerMotion, keys: crate::sprint::Keys, ticks: usize| {
            let mut count = 0;
            for _ in 0..ticks {
                MotionWorld {
                    entities: &[],
                    player: &mut *player,
                    scene: &mut scene,
                    assets: &assets,
                    collision_binding: &binding,
                    resources: &resources,
                }
                .controlled_simulation_tick(keys)
                .unwrap();
                count += std::mem::take(&mut player.events)
                    .iter()
                    .filter(|e| matches!(e, MotionEvent::Emission { .. }))
                    .count();
            }
            count
        };
        // Settle onto the floor, then stand still.
        steps(&mut player, Default::default(), 20);
        assert!(player.ground.on_ground);
        assert_eq!(steps(&mut player, Default::default(), 100), 0);
        let start = player.position;
        let forward = crate::sprint::Keys {
            forward: true,
            ..Default::default()
        };
        let walked = steps(&mut player, forward, 100);
        let metres = player.position.subtract(start).length() * 0.6;
        assert!(metres > 5.0, "walked {metres}");
        assert!(
            (walked as f64 - metres).abs() <= 1.5,
            "{walked} steps over {metres} step-metres"
        );
    }
    /// Vanilla sprint is a state, not a held key: `LocalPlayer.aiStep` starts
    /// it with the sprint key and stops it only on a vanilla stop condition.
    /// Releasing the key while swimming keeps the sprint and its swim speed.
    #[test]
    fn a_sprint_survives_the_sprint_key_coming_up_while_swimming() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let forward = crate::sprint::Keys {
            forward: true,
            ..Default::default()
        };
        let sprinting = crate::sprint::Keys {
            sprint: true,
            ..forward
        };
        // The same swim with the sprint key held, and with the key released
        // once the sprint has started: the speed must not change.
        let mut swims = Vec::new();
        for release in [false, true] {
            let mut scene = World::test_scene(&assets, &render, 3).unwrap();
            for x in 19..=21 {
                for y in 4..=8 {
                    for z in 19..=31 {
                        scene.set(x, y, z, water).unwrap();
                    }
                }
            }
            let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
            let mut w = MotionWorld {
                entities: &[],
                player: &mut player,
                scene: &mut scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            };
            let mut speed = Vec::new();
            for tick in 0..12 {
                // The key comes up on the fourth tick, after the sprint has
                // started and the player swims.
                let keys = if release && tick >= 4 {
                    forward
                } else {
                    sprinting
                };
                w.controlled_simulation_tick(keys).unwrap();
                speed.push(w.player.velocity.z);
            }
            assert!(
                w.player.sprinting,
                "the sprint state survives the sprint key in water"
            );
            assert!(w.player.swimming, "a sprinting player in water swims");
            swims.push(speed);
        }
        assert_eq!(
            swims[0], swims[1],
            "the swim speed must not drop when the sprint key comes up"
        );
    }

    /// Vanilla: `LivingEntity.jumpFromGround` adds its 0.2 forward impulse
    /// while the sprint state holds, so a jump taken after the sprint key
    /// comes up is still a sprint jump.
    #[test]
    fn a_sprint_jump_keeps_its_impulse_when_the_sprint_key_comes_up() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene(&assets, &render, 3).unwrap();
        let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
        let mut w = MotionWorld {
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets: &assets,
            collision_binding: &binding,
            resources: &resources,
        };
        let forward = crate::sprint::Keys {
            forward: true,
            ..Default::default()
        };
        let sprinting = crate::sprint::Keys {
            sprint: true,
            ..forward
        };
        for _ in 0..20 {
            w.controlled_simulation_tick(sprinting).unwrap();
        }
        assert!(w.player.ground.on_ground, "the player settles on the floor");
        assert!(w.player.sprinting, "the sprint key starts the sprint");
        let running = w.player.velocity.z;
        // The sprint key comes up and the player jumps.
        w.controlled_simulation_tick(crate::sprint::Keys {
            jump: true,
            ..forward
        })
        .unwrap();
        assert!(
            w.player.sprinting,
            "the sprint state survives the sprint key in the air"
        );
        assert!(
            w.player.velocity.z > running + 0.05,
            "the sprint jump keeps its forward impulse: {:?} vs {running}",
            w.player.velocity
        );
    }

    /// Vanilla `LocalPlayer.aiStep`: an item in use whose `use_effects` do not
    /// allow sprinting (a raised shield) keeps the sprint key, and a
    /// double-tap of forward, from starting a sprint; an item that allows it
    /// (a spear) does not.
    #[test]
    fn an_item_in_use_stops_a_sprint_from_starting_unless_it_allows_sprinting() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene(&assets, &render, 3).unwrap();
        let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 4.0, 20.5)).unwrap();
        let mut w = MotionWorld {
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets: &assets,
            collision_binding: &binding,
            resources: &resources,
        };
        let forward = crate::sprint::Keys {
            forward: true,
            ..Default::default()
        };
        let sprint_key = crate::sprint::Keys {
            sprint: true,
            ..forward
        };
        w.player.item_use_speed = Some(0.2);
        w.player.item_use_slows_sprint = true;
        for _ in 0..10 {
            w.controlled_simulation_tick(sprint_key).unwrap();
        }
        assert!(!w.player.sprinting, "a raised shield stops the sprint key");
        // Forward, off, forward again: the double-tap would sprint.
        for keys in [forward, Default::default(), forward, forward] {
            w.controlled_simulation_tick(keys).unwrap();
        }
        assert!(!w.player.sprinting, "a raised shield stops a double-tap");
        // A spear's use_effects allow sprinting.
        w.player.item_use_speed = Some(1.0);
        w.player.item_use_slows_sprint = false;
        w.controlled_simulation_tick(sprint_key).unwrap();
        assert!(w.player.sprinting, "an item that can_sprint lets it start");
    }

    /// Vanilla `Player.getFlyingSpeed`: a flying player moves at the ability
    /// speed, doubled while the sprint state holds. The sprint key starts the
    /// state; releasing it does not change the speed.
    #[test]
    fn the_flight_speed_follows_the_sprint_state_not_the_sprint_key() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene(&assets, &render, 3).unwrap();
        let mut player = PlayerMotion::new(&resources, Vec3::new(20.5, 10.0, 20.5)).unwrap();
        let mut w = MotionWorld {
            entities: &[],
            player: &mut player,
            scene: &mut scene,
            assets: &assets,
            collision_binding: &binding,
            resources: &resources,
        };
        w.player.may_fly = true;
        w.player.flying = true;
        let forward = crate::sprint::Keys {
            forward: true,
            ..Default::default()
        };
        let sprinting = crate::sprint::Keys {
            sprint: true,
            ..forward
        };
        assert_eq!(
            w.flying_speed().unwrap(),
            0.05,
            "a flying player without a sprint moves at the ability speed"
        );
        // The sprint key starts the sprint, and the state doubles the speed.
        w.controlled_simulation_tick(sprinting).unwrap();
        assert!(w.player.sprinting, "the sprint key starts the sprint");
        assert_eq!(
            w.flying_speed().unwrap(),
            0.1,
            "a sprinting player flies at twice the ability speed"
        );
        // Releasing the sprint key keeps the state, so the speed stays.
        w.controlled_simulation_tick(forward).unwrap();
        assert!(
            w.player.sprinting,
            "the sprint state survives the sprint key in the air"
        );
        assert_eq!(
            w.flying_speed().unwrap(),
            0.1,
            "the flight speed follows the sprint state, not the sprint key"
        );
    }
}
