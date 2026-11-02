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
use horizon_client::projectiles::Projectile;
use horizon_core::geometry::{Aabb, Direction, Vec3};
use horizon_core::position::BlockPos;
use horizon_world::{level::data::PushReaction, piston_push};

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

        impl Game<'_> {
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

/// [`with_entity`], with the entity mutable.
macro_rules! with_entity_mut {
    ($game:expr, $handle:expr, $entity:ident => $body:expr) => {
        $crate::game::entity::entity_lists!(match_entity, exclusive, $entity, $game, $handle, $body)
    };
}
pub(super) use with_entity_mut;

impl Entity for Mob {
    stored_motion!();
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
    fn bounds(&self) -> Aabb {
        Boat::bounds(self)
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
}

impl Entity for Minecart {
    stored_motion!();
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
