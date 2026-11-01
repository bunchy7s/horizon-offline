//! Resident player pose, native dimension arithmetic and dry pose-fit selection.
//! Full synchronized entity schemas, sleeping and fluid/flight lifecycles remain
//! separate from these stored movement fields.
use crate::{
    Result,
    motion::{MotionWorld, PlayerMotion},
};
use horizon_core::geometry::{Aabb, Vec3};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pose {
    #[default]
    Standing,
    Crouching,
    Swimming,
    FallFlying,
    SpinAttack,
    Sleeping,
}
impl Pose {
    /// `Pose` by its network id (`Pose.id`); the poses a player never takes
    /// are standing.
    pub fn from_id(id: i32) -> Self {
        match id {
            1 => Self::FallFlying,
            2 => Self::Sleeping,
            3 => Self::Swimming,
            4 => Self::SpinAttack,
            5 => Self::Crouching,
            _ => Self::Standing,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Standing => "STANDING",
            Self::Crouching => "CROUCHING",
            Self::Swimming => "SWIMMING",
            Self::FallFlying => "FALL_FLYING",
            Self::SpinAttack => "SPIN_ATTACK",
            Self::Sleeping => "SLEEPING",
        }
    }
    /// `Avatar.POSES` (and `LivingEntity.SLEEPING_DIMENSIONS`): the box of a
    /// player or mannequin in this pose; sleeping ignores the scale.
    pub fn dimensions(self, scale: f32) -> Dimensions {
        let (width, height, eye_height) = match self {
            Self::Standing => (0.6 * scale, 1.8 * scale, 1.62 * scale),
            Self::Crouching => (0.6 * scale, 1.5 * scale, 1.27 * scale),
            Self::Swimming | Self::FallFlying | Self::SpinAttack => {
                (0.6 * scale, 0.6 * scale, 0.4 * scale)
            }
            Self::Sleeping => (0.2, 0.2, 0.2),
        };
        Dimensions {
            width,
            height,
            eye_height,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Dimensions {
    pub width: f32,
    pub height: f32,
    pub eye_height: f32,
}
impl Dimensions {
    pub fn bounds(self, position: Vec3) -> Aabb {
        let half = f64::from(self.width / 2.0);
        Aabb::new(
            Vec3::new(position.x - half, position.y, position.z - half),
            Vec3::new(
                position.x + half,
                position.y + f64::from(self.height),
                position.z + half,
            ),
        )
    }
}
impl PlayerMotion {
    /// `LivingEntity.aiStep` counting down `autoSpinAttackTicks` for a
    /// server's copy of a player, which runs no client tick: the spin ends
    /// after its ticks and its damage goes with it, so the pose (and the eye
    /// a bow or a trident is shot from) comes back to standing.
    pub fn count_down_server_spin(&mut self) {
        if self.auto_spin_attack_ticks > 0 {
            self.auto_spin_attack_ticks -= 1;
            if self.auto_spin_attack_ticks <= 0 {
                self.auto_spin_attack_dmg = 0.0;
            }
        }
    }
    pub fn dimensions_for_pose(&mut self, pose: Pose) -> Result<Dimensions> {
        Ok(pose.dimensions(self.attribute("minecraft:scale")? as f32))
    }
    pub fn refresh_dimensions(&mut self) -> Result<()> {
        self.dimensions = self.dimensions_for_pose(self.pose)?;
        Ok(())
    }
    pub fn set_pose(&mut self, pose: Pose) -> Result<()> {
        if self.pose != pose {
            self.pose = pose;
            self.refresh_dimensions()?;
        }
        Ok(())
    }
    /// `LivingEntity.startSleeping` and `stopSleeping` set the pose at once
    /// (the 0.2 box in the bed, standing again after).
    pub fn set_sleeping(&mut self, sleeping: bool) -> Result<()> {
        if self.sleeping != sleeping {
            self.sleeping = sleeping;
            self.set_pose(if sleeping {
                Pose::Sleeping
            } else {
                Pose::Standing
            })?;
        }
        Ok(())
    }
    pub fn moving_slowly(&self) -> bool {
        self.local_crouching || (self.pose == Pose::Swimming && !self.fluids.was_touching_water)
    }
}
impl MotionWorld<'_, '_, '_> {
    pub fn can_fit_pose(&mut self, pose: Pose) -> Result<bool> {
        let bounds = self
            .player
            .dimensions_for_pose(pose)?
            .bounds(self.player.position);
        self.no_player_collision(bounds.deflate(Vec3::new(1.0e-7, 1.0e-7, 1.0e-7)))
    }
    /// LocalPlayer.aiStep computes crouching BEFORE KeyboardInput.tick, using
    /// the old shift input. This is distinct from Entity's current pose.
    pub fn update_local_crouching(&mut self) -> Result<()> {
        self.player.local_crouching = !self.player.flying
            && !self.player.swimming
            && self.can_fit_pose(Pose::Crouching)?
            && (self.player.shift_key_down || !self.can_fit_pose(Pose::Standing)?);
        Ok(())
    }
    /// Dry Player.updatePlayerPose after movement: crawl when neither standing
    /// nor crouching fits; leave the current pose if even crawling cannot fit.
    pub fn update_player_pose(&mut self) -> Result<()> {
        if !self.can_fit_pose(Pose::Swimming)? {
            return Ok(());
        }
        // Player.getDesiredPose: sleeping, swimming, gliding, Riptide's spin.
        let desired = if self.player.sleeping {
            Pose::Sleeping
        } else if self.player.swimming {
            Pose::Swimming
        } else if self.player.fall_flying {
            Pose::FallFlying
        } else if self.player.auto_spin_attack_ticks > 0 {
            Pose::SpinAttack
        } else if self.player.shift_key_down && !self.player.flying {
            Pose::Crouching
        } else {
            Pose::Standing
        };
        let pose = if self.player.spectator || self.can_fit_pose(desired)? {
            desired
        } else if self.can_fit_pose(Pose::Crouching)? {
            Pose::Crouching
        } else {
            Pose::Swimming
        };
        self.player.set_pose(pose)
    }
    /// The end of a server's copy of a player's tick (`ServerPlayer.doTick`
    /// runs `Player.tick`): `LivingEntity.aiStep` counts a Riptide spin down,
    /// `LivingEntity.tick` then runs `refreshDirtyAttributes` (a changed
    /// `minecraft:scale` rebuilds the box, `onAttributeUpdated`), and
    /// `Player.tick` ends with `updatePlayerPose`. The server checks the
    /// player's moves with that box (#3684).
    pub fn server_player_pose_tick(&mut self) -> Result<()> {
        self.player.count_down_server_spin();
        self.player.refresh_attributes()?;
        self.update_player_pose()
    }
    /// The pose part of `Player.tick` for a passenger (`Entity.rideTick` runs
    /// `tick`, not the travel): `Entity.updateSwimming` stops a swim, since it
    /// needs `!isPassenger()`, and `updatePlayerPose` follows. A player who
    /// boards a boat while swimming sits up instead of lying under it (#3308).
    /// The server's `LivingEntity.updateFallFlying` ends a glide the same way
    /// (`canGlide` needs `!isPassenger()`) and `LivingEntity.tick` resets
    /// `fallFlyTicks`: a player who boards while gliding sits up instead of
    /// keeping the glide pose (#3093).
    /// `Entity.baseTick` still samples the fluids first, in the box the vehicle
    /// allows (the boat the game stores in the player's fluid state), so a boat
    /// afloat keeps the eyes dry.
    pub fn passenger_pose_tick(&mut self) -> Result<()> {
        self.player.fluids.was_eye_in_water = self.player.is_eye_in_fluid(crate::motion::WATER);
        self.update_fluids()?;
        // Player.tick: a passenger is never on the ground (setOnGround(false)
        // clears the supporting block too), so its look packets say so.
        self.player.ground.on_ground = false;
        self.player.ground.on_ground_no_blocks = false;
        self.player.ground.supporting_block = None;
        self.player.swimming = false;
        self.player.fall_flying = false;
        self.player.fall_fly_ticks = 0;
        self.update_player_pose()?;
        // A cushion's seat puts the feet in the support block, so neither
        // standing nor crawling fits and `updatePlayerPose` keeps the old
        // pose. A prior swim would stick (#4509); sit up like a boat rider.
        if self.player.pose == Pose::Swimming && !self.player.sleeping {
            self.player.set_pose(Pose::Standing)?;
        }
        Ok(())
    }
}
