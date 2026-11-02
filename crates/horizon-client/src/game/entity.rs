//! `Entity`: what every kind in the game's entity lists answers to, so a system
//! can treat them alike and only write down where vanilla's kinds differ.
//!
//! Default methods follow vanilla's `Entity`; a kind overrides one only where
//! its vanilla class does. [`EntityHandle`] names one entity by list and index,
//! so a system can walk handles and reach each entity after other borrows of
//! `Game` end. The walk allocates nothing and dispatch is static: hot paths pay
//! no vtable. A new kind is one line in `entity_lists!` and one `impl Entity`.

use super::{FallingBlock, Game};
use super::{
    boats::Boat, fireworks::Rocket, minecarts::Minecart, tnt::PrimedTnt, trident::ThrownTrident,
};
use horizon_client::entities::{ArmorStand, ExperienceOrb, ItemEntity};
use horizon_client::mobs::{Mob, Species};
use horizon_client::projectiles::{Projectile, ProjectileKind};
use horizon_core::geometry::{Aabb, Direction, Vec3};
use horizon_core::position::BlockPos;
use horizon_world::{level::data::PushReaction, piston_push};

/// What a burst of wind asks of a kind: the two flags `ServerExplosion` hands
/// `ignoreExplosion`.
#[derive(Clone, Copy)]
pub(super) struct Gust {
    /// `shouldAffectBlocklikeEntities`: a mace's burst, never a charge's.
    pub(super) blocklike: bool,
    /// A mob's burst (a breeze's) with `mob_griefing` off.
    pub(super) mob_blocked: bool,
}

/// How `Entity.pushFromExplosion` sees one entity: the box exposure is
/// measured through, and the height its direction is taken from.
pub(super) struct ExplosionTarget {
    pub(super) min: [f64; 3],
    pub(super) max: [f64; 3],
    pub(super) eye_y: f64,
}

impl ExplosionTarget {
    /// A box `width` wide and `height` tall on `feet`, eyes `eye` above them.
    fn around(feet: Vec3, width: f64, height: f64, eye: f64) -> Self {
        let half = width / 2.0;
        Self {
            min: [feet.x - half, feet.y, feet.z - half],
            max: [feet.x + half, feet.y + height, feet.z + half],
            eye_y: feet.y + eye,
        }
    }
    fn with_eye_y(self, eye_y: f64) -> Self {
        Self { eye_y, ..self }
    }
    /// `bounds`, with the eyes at 0.85 of its height unless `eye` says
    /// otherwise.
    fn of_bounds(feet: Vec3, bounds: Aabb, eye: Option<f64>) -> Self {
        let (min, max) = (bounds.min_position(), bounds.max_position());
        Self {
            min: [min.x, min.y, min.z],
            max: [max.x, max.y, max.z],
            eye_y: feet.y + eye.unwrap_or_else(|| f64::from((max.y - min.y) as f32 * 0.85f32)),
        }
    }
}

pub(super) trait Entity {
    fn position(&self) -> Vec3;
    fn position_mut(&mut self) -> &mut Vec3;
    fn velocity_mut(&mut self) -> &mut Vec3;
    fn bounds(&self) -> Aabb;

    fn on_ground(&self) -> bool {
        false
    }
    /// `Entity.maxUpStep`.
    fn get_step_height(&self) -> f32 {
        0.0
    }
    fn is_alive(&self) -> bool {
        true
    }
    /// `Entity.isRemoved`.
    fn is_removed(&self) -> bool {
        false
    }
    /// `instanceof LivingEntity`.
    fn is_living(&self) -> bool {
        false
    }
    /// `Entity.isIgnoringBlockTriggers`.
    fn is_ignoring_block_triggers(&self) -> bool {
        false
    }
    /// Whether it presses plates and tripwires: `getEntitiesOfClass` skips
    /// removed entities, and the block's predicate skips the ones above.
    fn triggers_blocks(&self) -> bool {
        !self.is_removed() && !self.is_ignoring_block_triggers()
    }
    /// Whether `tick_blocks` sends the blocks in its box to `entityInside`
    /// (`applyEffectsFromBlocks`). A kind that function sweeps another way, or
    /// whose tick skips that step, says no.
    fn sweeps_blocks_inside(&self) -> bool {
        true
    }
    /// `Entity.ignoreExplosion`: whether a burst of wind passes it by.
    fn ignores_explosion(&self, _burst: Gust) -> bool {
        false
    }
    /// The box and eye height `Entity.pushFromExplosion` reads; vanilla's
    /// default puts the eyes at 0.85 of the box.
    fn explosion_target(&self) -> ExplosionTarget {
        ExplosionTarget::of_bounds(self.position(), self.bounds(), None)
    }
    /// `Entity.isSupportedBy`.
    fn is_supported_by(&self, _pos: BlockPos) -> bool {
        false
    }
    /// `Entity.getPistonPushReaction`.
    fn get_piston_push_reaction(&self) -> PushReaction {
        PushReaction::PushPull
    }
    /// `Entity.move(MoverType.PISTON, …)` once the movement is resolved.
    fn move_by_piston(&mut self, movement: Vec3) {
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }
    /// `PistonMovingBlockEntity.causeBounce`: a slime block sets the velocity
    /// along the movement axis.
    fn piston_slime_bounce(&mut self, movement: Direction) {
        let velocity = self.velocity_mut();
        *velocity = piston_push::bounce_velocity(movement, *velocity);
    }
}

/// The fields every kind stores under the same names.
macro_rules! stored_motion {
    () => {
        fn position(&self) -> Vec3 {
            self.position
        }
        fn position_mut(&mut self) -> &mut Vec3 {
            &mut self.position
        }
        fn velocity_mut(&mut self) -> &mut Vec3 {
            &mut self.velocity
        }
    };
}

/// The game's lists of entities, in the order systems walk them. Hands them to
/// `$then`, so one list drives the handles and every typed dispatch.
macro_rules! entity_lists {
    ($then:ident, $($args:tt)*) => {
        $crate::game::entity::$then! {
            ($($args)*)
            Mob(Mob) mobs: mobs,
            Item(ItemEntity) dropped: dropped,
            Boat(Boat) boats: boats,
            Cart(Minecart) carts: carts,
            ArmorStand(ArmorStand) armor_stands: armor_stands,
            Tnt(PrimedTnt) tnts: tnts,
            Falling(FallingBlock) falling: falling,
            Orb(ExperienceOrb) orbs: orbs,
            Projectile(Projectile) projectiles: creatures.projectiles,
            Rocket(Rocket) rockets: rockets,
            Trident(ThrownTrident) tridents: tridents,
        }
    };
}
pub(crate) use entity_lists;

macro_rules! define_entity_handles {
    (() $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        /// Every list, borrowed apart from the rest of `Game`, for
        /// [`for_each_entity`] while the world is borrowed mutably.
        #[derive(Clone, Copy)]
        pub(super) struct EntityLists<'w> {
            $(pub(super) $field: &'w [$ty],)*
        }

        /// One entity, by list and index.
        #[derive(Clone, Copy, Hash, PartialEq, Eq)]
        pub(super) enum EntityHandle {
            $($kind(usize),)*
        }

        /// Every list moved out of `Game`, for [`for_each_entity_mut`]: the
        /// world stays readable while the entities change.
        pub(super) struct TakenEntities {
            $(pub(super) $field: Vec<$ty>,)*
        }

        impl Game<'_> {
            /// Moves every list out; give it back with `restore_entities`.
            pub(super) fn take_entities(&mut self) -> TakenEntities {
                TakenEntities {
                    $($field: std::mem::take(&mut self$(.$list)+),)*
                }
            }
            pub(super) fn restore_entities(&mut self, taken: TakenEntities) {
                $(self$(.$list)+ = taken.$field;)*
            }
            /// Every entity's handle: lengths read now, nothing allocated, and
            /// no borrow of `Game` held, so the caller can change it mid-walk.
            pub(super) fn entity_handles(&self) -> impl Iterator<Item = EntityHandle> + use<> {
                std::iter::empty()
                    $(.chain((0..self$(.$list)+.len()).map(EntityHandle::$kind)))*
            }
        }
    };
}
pub(crate) use define_entity_handles;

entity_lists!(define_entity_handles,);

macro_rules! entity_borrow {
    (shared, $place:expr) => {
        &$place
    };
    (exclusive, $place:expr) => {
        &mut $place
    };
}
pub(crate) use entity_borrow;

macro_rules! match_entity {
    (($borrow:ident, $entity:ident, $game:expr, $handle:expr, $body:expr)
     $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        match $handle {
            $($crate::game::entity::EntityHandle::$kind(i) => {
                let $entity = $crate::game::entity::entity_borrow!($borrow, $game$(.$list)+[i]);
                $body
            })*
        }
    };
}
pub(crate) use match_entity;

/// Runs `$body` with `$entity` bound to the handle's entity, as its own type:
/// one match, each arm compiled for its kind, so nothing goes through a vtable.
macro_rules! with_entity {
    ($game:expr, $handle:expr, $entity:ident => $body:expr) => {
        $crate::game::entity::entity_lists!(match_entity, shared, $entity, $game, $handle, $body)
    };
}
pub(super) use with_entity;

macro_rules! borrow_lists {
    (($game:expr) $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        $crate::game::entity::EntityLists {
            $($field: &$game$(.$list)+,)*
        }
    };
}
pub(crate) use borrow_lists;

/// `$game`'s [`EntityLists`]; a macro so the borrow stays field by field.
macro_rules! entity_lists_of {
    ($game:expr) => {
        $crate::game::entity::entity_lists!(borrow_lists, $game)
    };
}
pub(super) use entity_lists_of;

macro_rules! walk_lists {
    (($lists:expr, $entity:ident, $body:block)
     $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        $(for $entity in $lists.$field $body)*
    };
}
pub(crate) use walk_lists;

/// Runs `$body` for every entity, with `$entity` as a reference to its own
/// type: one loop per list, each compiled for its kind, so nothing allocates
/// or goes through a vtable and a check a kind answers with a constant folds
/// away. A kind added to `entity_lists!` joins every walk.
macro_rules! for_each_entity {
    ($lists:expr, $entity:ident => $body:block) => {
        $crate::game::entity::entity_lists!(walk_lists, $lists, $entity, $body)
    };
}
pub(super) use for_each_entity;

macro_rules! walk_lists_mut {
    (($lists:expr, $entity:ident, $body:block)
     $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        $(for $entity in $lists.$field.iter_mut() $body)*
    };
}
pub(crate) use walk_lists_mut;

/// [`for_each_entity`] over [`TakenEntities`], with the entity mutable.
macro_rules! for_each_entity_mut {
    ($lists:expr, $entity:ident => $body:block) => {
        $crate::game::entity::entity_lists!(walk_lists_mut, $lists, $entity, $body)
    };
}
pub(super) use for_each_entity_mut;

/// [`with_entity`], with the entity mutable.
macro_rules! with_entity_mut {
    ($game:expr, $handle:expr, $entity:ident => $body:expr) => {
        $crate::game::entity::entity_lists!(match_entity, exclusive, $entity, $game, $handle, $body)
    };
}
pub(super) use with_entity_mut;

impl Entity for Mob {
    stored_motion!();
    fn ignores_explosion(&self, _burst: Gust) -> bool {
        !self.alive()
    }
    fn explosion_target(&self) -> ExplosionTarget {
        let feet = self.position;
        let (w, h) = (self.kind.width, f64::from(self.height()));
        ExplosionTarget::around(feet, w, h, 0.0).with_eye_y(self.eye_y())
    }
    fn bounds(&self) -> Aabb {
        Mob::bounds(self)
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
    fn get_step_height(&self) -> f32 {
        self.kind.step_height as f32
    }
    fn is_alive(&self) -> bool {
        !self.removed && self.alive()
    }
    fn is_removed(&self) -> bool {
        self.removed
    }
    fn is_living(&self) -> bool {
        true
    }
    /// `Bat.isIgnoringBlockTriggers`.
    fn is_ignoring_block_triggers(&self) -> bool {
        self.species() == Species::Bat
    }
    fn is_supported_by(&self, pos: BlockPos) -> bool {
        self.supporting == Some([pos.x, pos.y, pos.z])
    }
}

impl Entity for ItemEntity {
    stored_motion!();
    /// `ItemEntity.ignoreExplosion`: only a burst that affects block-like
    /// entities.
    fn ignores_explosion(&self, burst: Gust) -> bool {
        !burst.blocklike
    }
    fn explosion_target(&self) -> ExplosionTarget {
        ExplosionTarget::around(self.position, 0.25, 0.25, 0.2125)
    }
    fn bounds(&self) -> Aabb {
        ItemEntity::bounds(self)
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
    fn move_by_piston(&mut self, movement: Vec3) {
        self.position = self.position.add_vector(movement);
        // Entity.move(MoverType.PISTON): onGround only if the push was down.
        if movement.y != 0.0 {
            self.on_ground = movement.y < 0.0;
        }
    }
    fn piston_slime_bounce(&mut self, movement: Direction) {
        ItemEntity::piston_slime_bounce(self, movement);
    }
}

impl Entity for ExperienceOrb {
    stored_motion!();
    fn explosion_target(&self) -> ExplosionTarget {
        ExplosionTarget::around(self.position, 0.5, 0.5, f64::from(0.5f32 * 0.85f32))
    }
    fn bounds(&self) -> Aabb {
        ExperienceOrb::bounds(self)
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
    fn move_by_piston(&mut self, movement: Vec3) {
        self.position = self.position.add_vector(movement);
        if movement.y != 0.0 {
            self.on_ground = movement.y < 0.0;
        }
    }
    fn piston_slime_bounce(&mut self, movement: Direction) {
        self.velocity = piston_push::bounce_velocity(movement, self.velocity);
        self.on_ground = false;
    }
}

impl Entity for Boat {
    stored_motion!();
    /// `VehicleEntity.ignoreExplosion`.
    fn ignores_explosion(&self, burst: Gust) -> bool {
        burst.mob_blocked
    }
    /// `EntityType.OAK_BOAT` & co: the eye height is the full 0.5625.
    fn explosion_target(&self) -> ExplosionTarget {
        ExplosionTarget::of_bounds(self.position, Boat::bounds(self), Some(0.5625))
    }
    fn bounds(&self) -> Aabb {
        Boat::bounds(self)
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
}

impl Entity for Minecart {
    stored_motion!();
    /// `VehicleEntity.ignoreExplosion`.
    fn ignores_explosion(&self, burst: Gust) -> bool {
        burst.mob_blocked
    }
    fn bounds(&self) -> Aabb {
        Minecart::bounds(self)
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
    /// `tick_blocks` sweeps minecarts along with the players.
    fn sweeps_blocks_inside(&self) -> bool {
        false
    }
}

impl Entity for ArmorStand {
    stored_motion!();
    /// `ArmorStand.ignoreExplosion`: a visible stand feels only a burst that
    /// affects block-like entities.
    fn ignores_explosion(&self, burst: Gust) -> bool {
        self.invisible || !burst.blocklike
    }
    /// `EntityType.ARMOR_STAND`'s size, halved when small; a marker has none.
    fn explosion_target(&self) -> ExplosionTarget {
        let (w, h, eye) = if self.marker {
            (0.0, 0.0, 0.0)
        } else if self.small {
            (0.25, f64::from(0.9875f32), f64::from(0.9875f32))
        } else {
            (0.5, f64::from(1.975f32), f64::from(1.7775f32))
        };
        ExplosionTarget::around(self.position, w, h, eye)
    }
    fn bounds(&self) -> Aabb {
        ArmorStand::bounds(self)
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
    fn is_living(&self) -> bool {
        true
    }
    /// `ArmorStand.isIgnoringBlockTriggers`.
    fn is_ignoring_block_triggers(&self) -> bool {
        self.marker
    }
    /// `ArmorStand.getPistonPushReaction`: a marker is ignored.
    fn get_piston_push_reaction(&self) -> PushReaction {
        if self.marker {
            PushReaction::IgnoreEntity
        } else {
            PushReaction::PushPull
        }
    }
}

impl Entity for PrimedTnt {
    stored_motion!();
    fn explosion_target(&self) -> ExplosionTarget {
        ExplosionTarget::around(self.position, 0.25, 0.25, 0.0)
    }
    fn bounds(&self) -> Aabb {
        PrimedTnt::bounds(self)
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
    fn piston_slime_bounce(&mut self, movement: Direction) {
        self.velocity = piston_push::bounce_velocity(movement, self.velocity);
        // Leave the ground so the next tick does not apply the on-ground
        // Y * -0.5 friction that cancels the launch.
        self.on_ground = false;
    }
}

impl Entity for FallingBlock {
    stored_motion!();
    fn bounds(&self) -> Aabb {
        horizon_client::falling_block::bounds(self.position)
    }
}

impl Entity for Projectile {
    stored_motion!();
    /// A stuck projectile, and the charges themselves, ignore a burst of wind.
    fn ignores_explosion(&self, _burst: Gust) -> bool {
        self.stuck
            || matches!(
                self.kind,
                ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
            )
    }
    fn explosion_target(&self) -> ExplosionTarget {
        ExplosionTarget::around(self.position, 0.25, 0.25, 0.0)
    }
    fn bounds(&self) -> Aabb {
        Projectile::bounds(self)
    }
    fn move_by_piston(&mut self, movement: Vec3) {
        self.position = self.position.add_vector(movement);
        self.needs_sync = true;
    }
    fn piston_slime_bounce(&mut self, movement: Direction) {
        self.velocity = piston_push::bounce_velocity(movement, self.velocity);
        self.needs_sync = true;
    }
}

impl Entity for Rocket {
    stored_motion!();
    /// 0.25 wide, eyes at 0.85 of it.
    fn explosion_target(&self) -> ExplosionTarget {
        ExplosionTarget::around(self.position, 0.25, 0.25, 0.2125)
    }
    fn bounds(&self) -> Aabb {
        Rocket::bounds(self)
    }
    /// `FireworkRocketEntity.tick`: boosting a player skips the step.
    fn sweeps_blocks_inside(&self) -> bool {
        !self.attached
    }
}

impl Entity for ThrownTrident {
    stored_motion!();
    /// An arrow stuck in the ground is not pushed.
    fn ignores_explosion(&self, _burst: Gust) -> bool {
        self.in_ground
    }
    fn explosion_target(&self) -> ExplosionTarget {
        ExplosionTarget::around(self.position, 0.5, 0.5, f64::from(0.13_f32))
    }
    fn bounds(&self) -> Aabb {
        let p = self.position;
        Aabb::new(
            Vec3::new(p.x - 0.25, p.y, p.z - 0.25),
            Vec3::new(p.x + 0.25, p.y + 0.5, p.z + 0.25),
        )
    }
    /// `GameLevel::tick_blocks` sweeps tridents itself.
    fn sweeps_blocks_inside(&self) -> bool {
        false
    }
}
