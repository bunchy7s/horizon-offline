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
use horizon_client::mobs::Mob;
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
            Mob: mobs,
            Item: dropped,
            Boat: boats,
            Cart: carts,
            ArmorStand: armor_stands,
            Tnt: tnts,
            Falling: falling,
            Orb: orbs,
            Projectile: creatures.projectiles,
            Rocket: rockets,
            Trident: tridents,
        }
    };
}
pub(crate) use entity_lists;

macro_rules! define_entity_handles {
    (() $($kind:ident: $($list:ident).+,)*) => {
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
     $($kind:ident: $($list:ident).+,)*) => {
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
}

impl Entity for ArmorStand {
    stored_motion!();
    fn bounds(&self) -> Aabb {
        ArmorStand::bounds(self)
    }
    fn on_ground(&self) -> bool {
        self.on_ground
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
}
