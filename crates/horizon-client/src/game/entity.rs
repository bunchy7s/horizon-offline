//! `Entity`: what every kind in the game's entity lists answers to, so a system
//! can treat them alike and only write down where vanilla's kinds differ.
//!
//! Default methods follow vanilla's `Entity`; a kind overrides one only where
//! its vanilla class does. [`EntityHandle`] names one entity by list and index,
//! so a system can walk handles and reach each entity after other borrows of
//! `Game` end. The walk allocates nothing and dispatch is static: hot paths pay
//! no vtable. A new kind is one line in `entity_lists!` and one `impl Entity`.

use super::commands::Target;
use super::persist::ChunkKey;
use super::{FallingBlock, Game};
use super::{
    boats::Boat, fireworks::Rocket, minecarts::Minecart, tnt::PrimedTnt, trident::ThrownTrident,
};
use horizon_client::entities::{ArmorStand, ExperienceOrb, ItemEntity};
use horizon_client::entity_rules::EntityRules;
use horizon_client::mobs::{Mob, Species};
use horizon_client::projectiles::Projectile;
use horizon_core::geometry::{Aabb, Direction, Vec3};
use horizon_core::position::BlockPos;
use horizon_world::nbt::Tag;
use horizon_world::{level::data::PushReaction, piston_push};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

/// What `Entity.thunderHit` does to a kind: it sets the entity on fire and
/// calls `hurtServer(lightning_bolt, 5)`, so a kind whose `hurtServer` does
/// nothing (projectiles, rockets, TNT, falling blocks, armor stands here) stays
/// untouched. A strike runs the stages in this order.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ThunderHit {
    Untouched,
    /// Burns, takes 5 damage, and the kinds with their own `thunderHit` change.
    Mob,
    /// `VehicleEntity.hurtServer`: 5 x 10 is over the 40 that destroys it.
    Vehicle,
    /// `ExperienceOrb.hurtServer`: 5 damage off 5 health.
    Orb,
    /// `ItemEntity.hurtServer`: burns up unless fire resistant.
    Item,
}

/// What a blast knows about itself, for the entities in its reach to answer.
#[derive(Clone, Copy)]
pub(super) struct Blast {
    /// `VehicleEntity.ignoreExplosion`: a mob's blast, with mob griefing off.
    pub(super) vehicles_ignore: bool,
    /// `ItemEntity.ignoreExplosion` (`shouldAffectBlocklikeEntities`).
    pub(super) items_ignore: bool,
    /// The exploding mob, which `getEntities(source, box)` leaves out.
    pub(super) spared: Option<u32>,
}

pub(super) trait Entity: EntityRules {
    /// `Entity.getId`.
    fn id(&self) -> u32;
    fn position(&self) -> Vec3;
    fn position_mut(&mut self) -> &mut Vec3;
    fn velocity_mut(&mut self) -> &mut Vec3;
    fn bounds(&self) -> Aabb;
    /// What a selector finds this entity as (`@e`), or `None` for one it cannot
    /// name yet. Required, so a new kind states which.
    fn selector_target(&self) -> Option<Target>;
    /// The entity type id a native mod's join hook names (`minecraft:oak_boat`).
    fn native_type(&self) -> Cow<'static, str>;
    /// Saves entity `index` of this kind into its chunk's entity list: the
    /// chunk and the NBT when it lies in `keys` and is saved with its chunk.
    /// Required, so a new kind states how it saves.
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)>;

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
    /// `Entity.thunderHit` for this kind.
    fn thunder_hit(&self) -> ThunderHit {
        ThunderHit::Untouched
    }
    /// The box a lightning strike tests against.
    fn thunder_bounds(&self) -> Aabb {
        self.bounds()
    }
    /// `Entity.ignoreExplosion`: the blast neither hurts nor pushes it. Also
    /// answers for what `getEntities(source, box)` never lists: the exploding
    /// mob, and the dead.
    fn ignores_explosion(&self, _blast: &Blast) -> bool {
        false
    }
    /// Whether a burst of wind moves it: what `Entity.push` moves, unless the
    /// kind says otherwise.
    fn pushed_by_wind(&self) -> bool {
        self.pushed_by_explosion()
    }
    /// Whether `Entity.push` moves it. An arrow in the ground, a wind charge and
    /// an armour stand (`explosion_breaks_decorations` takes those) stay put.
    fn pushed_by_explosion(&self) -> bool {
        true
    }
    /// The box the blast casts its rays at: `getSeenPercent`.
    fn explosion_box(&self) -> Aabb {
        self.bounds()
    }
    /// How far above its feet `pushFromExplosion` aims: `getEyeHeight`, which
    /// defaults to 0.85 of the box.
    fn explosion_eye_height(&self) -> f64 {
        let b = self.explosion_box();
        f64::from((b.max_position().y - b.min_position().y) as f32 * 0.85f32)
    }
    /// `Entity.push` with the explosion's knockback.
    fn push_from_explosion(&mut self, push: Vec3) {
        let v = self.velocity_mut();
        *v = Vec3::new(v.x + push.x, v.y + push.y, v.z + push.z);
    }
    /// `Entity.isSupportedBy`.
    fn is_supported_by(&self, _pos: BlockPos) -> bool {
        false
    }
    /// `Entity.getEyeY`, where `updateFluidHeightAndDoFluidPushing` reads the
    /// eye's fluid; the top of the box unless a kind sets it.
    fn fluid_eye_y(&self) -> f64 {
        self.bounds().max_position().y
    }
    /// `Entity.updateFluidHeightAndDoFluidPushing`: its water and lava heights
    /// (`(0, 0)` for a kind that is not pushed by fluid), and the current of
    /// each fluid added to its velocity. One body for every kind.
    fn push_by_fluid(
        &mut self,
        world: &horizon_client::world::World,
        fluids: &horizon_client::motion::FluidResources,
    ) -> (f64, f64) {
        if !self.is_pushed_by_fluid() {
            return (0.0, 0.0);
        }
        let (bounds, at, eye) = (self.bounds(), self.position(), self.fluid_eye_y());
        horizon_client::entities::update_entity_fluids(
            bounds,
            at,
            eye,
            self.velocity_mut(),
            world,
            fluids,
        )
    }
    /// `BubbleColumnBlock.entityInside` through `applyEffectsFromBlocks`: a
    /// column lifts or drags every kind alike.
    fn push_by_bubble_column(
        &mut self,
        world: &horizon_client::world::World,
        registry: &horizon_world::registry::BlockRegistry,
        facts: &horizon_client::blocks::BlockFacts,
        collision: &horizon_world::block_collision::BlockCollisionRegistry<'_>,
    ) -> bool {
        let bounds = self.bounds();
        horizon_client::motion::bubble_column_push(
            world,
            registry,
            facts,
            collision,
            bounds,
            self.velocity_mut(),
        )
    }
    /// `Entity.getPistonPushReaction`.
    fn get_piston_push_reaction(&self) -> PushReaction {
        PushReaction::PushPull
    }
    /// Width and height of the box a portal sweep follows, or None for a kind
    /// the game does not carry through portals (so far armor stands and
    /// falling blocks; fishing hooks are off the walk, and
    /// `FishingHook.canUsePortal` is false in vanilla).
    fn portal_size(&self) -> Option<(f32, f32)> {
        None
    }
    /// `Entity.canUsePortal(false)` for a kind in `portal_size`. A kind that
    /// cannot stays tracked, so its cooldown and process run out.
    fn can_use_portal(&self, _game: &Game<'_>) -> bool {
        true
    }
    /// A boat or minecart: the kinds whose portal pass opens the tick, before
    /// they move (`AbstractMinecart.tick` handles its portal first).
    fn is_vehicle(&self) -> bool {
        false
    }
    /// Where it stood at the start of its tick, for the kinds that store it
    /// (None: `portals.before` has it).
    fn portal_from(&self) -> Option<Vec3> {
        None
    }
    /// Whether this tick's movement can have entered a portal block: a stuck
    /// arrow or a trident in the ground that did not move enters none.
    fn portal_sweeps(&self) -> bool {
        true
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
        fn id(&self) -> u32 {
            self.id
        }
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

            /// Takes the entity out of its list (`Entity.discard` before it ever ticked).
            pub(super) fn discard_entity(&mut self, handle: EntityHandle) {
                match handle {
                    $(EntityHandle::$kind(i) => {
                        self$(.$list)+.remove(i);
                    })*
                }
            }

            /// [`Self::entity_handles`], last of each list first, so the
            /// caller can remove the entity it is on.
            pub(super) fn entity_handles_rev(&self) -> impl Iterator<Item = EntityHandle> + use<> {
                std::iter::empty()
                    $(.chain((0..self$(.$list)+.len()).rev().map(EntityHandle::$kind)))*
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

macro_rules! save_lists {
    (($game:expr, $keys:expr, $out:expr)
     $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        $(for i in 0..$game$(.$list)+.len() {
            if let Some((key, tag)) = <$ty as $crate::game::entity::Entity>::save_nbt($game, i, $keys) {
                $out.entry(key).or_default().push(tag);
            }
        })*
    };
}
pub(crate) use save_lists;

/// Saves every entity of every list in `entity_lists!` into `$out` by chunk,
/// each through its own `Entity::save_nbt`: one loop per list, static dispatch,
/// nothing allocated beyond the tags. A kind added to the lists cannot be left
/// out, since `save_nbt` has no default.
macro_rules! save_entities_of_every_kind {
    ($game:expr, $keys:expr, $out:expr) => {
        $crate::game::entity::entity_lists!(save_lists, $game, $keys, $out)
    };
}

impl Game<'_> {
    /// Saves every entity of every kind that lies in `keys` into `out`, by chunk.
    pub(super) fn save_every_entity(
        &mut self,
        keys: &HashSet<ChunkKey>,
        out: &mut HashMap<ChunkKey, Vec<Tag>>,
    ) {
        save_entities_of_every_kind!(self, keys, out);
    }
}

impl Entity for Mob {
    fn native_type(&self) -> Cow<'static, str> {
        super::native_hooks::mob_kind(self).into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_mob(index, keys)
    }
    stored_motion!();
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((self.width(), self.height()))
    }
    fn can_use_portal(&self, game: &Game<'_>) -> bool {
        game.mob_can_use_portal(self)
    }
    fn selector_target(&self) -> Option<Target> {
        (self.death_time == 0).then_some(Target::Mob(self.id))
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
    /// `Warden.ignoreExplosion`: while it digs or emerges.
    fn ignores_explosion(&self, blast: &Blast) -> bool {
        !self.alive()
            || blast.spared == Some(self.id)
            || (self.species() == Species::Warden
                && horizon_client::mobs::warden::digging_or_emerging(self))
    }
    /// `getEyeHeight` of the mob as it stands: a baby's, a slime's or a
    /// phantom's own scaled eye height, not the adult kind's. The blast's box
    /// is its real `bounds`, so it needs no override.
    fn explosion_eye_height(&self) -> f64 {
        self.eye_height()
    }
    /// `Entity.push` sets `needsSync`.
    fn push_from_explosion(&mut self, push: Vec3) {
        self.velocity = Vec3::new(
            self.velocity.x + push.x,
            self.velocity.y + push.y,
            self.velocity.z + push.z,
        );
        self.needs_sync = true;
    }
    fn is_supported_by(&self, pos: BlockPos) -> bool {
        self.supporting == Some([pos.x, pos.y, pos.z])
    }
    fn thunder_hit(&self) -> ThunderHit {
        if self.alive() {
            ThunderHit::Mob
        } else {
            ThunderHit::Untouched
        }
    }
    /// The kind's own size, as the strike has always read it.
    fn thunder_bounds(&self) -> Aabb {
        let (p, w) = (self.position, self.kind.width / 2.0);
        Aabb::new(
            Vec3::new(p.x - w, p.y, p.z - w),
            Vec3::new(p.x + w, p.y + self.kind.height, p.z + w),
        )
    }
}

impl Entity for ItemEntity {
    fn native_type(&self) -> Cow<'static, str> {
        "minecraft:item".into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_item(index, keys)
    }
    stored_motion!();
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((0.25, 0.25))
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Item(self.id))
    }
    fn thunder_hit(&self) -> ThunderHit {
        ThunderHit::Item
    }
    fn ignores_explosion(&self, blast: &Blast) -> bool {
        blast.items_ignore
    }
    fn explosion_eye_height(&self) -> f64 {
        0.2125
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
    fn native_type(&self) -> Cow<'static, str> {
        "minecraft:experience_orb".into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_orb(index, keys)
    }
    stored_motion!();
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((0.5, 0.5))
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Orb(self.id))
    }
    fn thunder_hit(&self) -> ThunderHit {
        ThunderHit::Orb
    }
    fn explosion_eye_height(&self) -> f64 {
        f64::from(0.5f32 * 0.85f32)
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
    fn native_type(&self) -> Cow<'static, str> {
        self.entity_type().into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_boat(index, keys)
    }
    stored_motion!();
    fn is_vehicle(&self) -> bool {
        true
    }
    /// `Boat.getEyeY`: the top of the 0.5625 box.
    fn fluid_eye_y(&self) -> f64 {
        self.position.y + super::boats::HEIGHT
    }
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((super::boats::WIDTH as f32, super::boats::HEIGHT as f32))
    }
    fn can_use_portal(&self, game: &Game<'_>) -> bool {
        game.vehicle_can_use_portal(self.id)
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Boat(self.id))
    }
    fn thunder_hit(&self) -> ThunderHit {
        ThunderHit::Vehicle
    }
    fn ignores_explosion(&self, blast: &Blast) -> bool {
        blast.vehicles_ignore
    }
    /// `EntityType.OAK_BOAT & co`: the full 0.5625.
    fn explosion_eye_height(&self) -> f64 {
        0.5625
    }
    fn bounds(&self) -> Aabb {
        Boat::bounds(self)
    }
    fn on_ground(&self) -> bool {
        self.on_ground
    }
}

impl Entity for Minecart {
    fn native_type(&self) -> Cow<'static, str> {
        self.kind.id().into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_cart(index, keys)
    }
    stored_motion!();
    fn is_vehicle(&self) -> bool {
        true
    }
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((
            super::minecarts::WIDTH as f32,
            super::minecarts::HEIGHT as f32,
        ))
    }
    fn can_use_portal(&self, game: &Game<'_>) -> bool {
        game.vehicle_can_use_portal(self.id)
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Cart(self.id))
    }
    fn thunder_hit(&self) -> ThunderHit {
        ThunderHit::Vehicle
    }
    fn ignores_explosion(&self, blast: &Blast) -> bool {
        blast.vehicles_ignore
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

/// Width and height `EntityType.ARMOR_STAND` gives a stand.
fn armor_stand_size(stand: &ArmorStand) -> (f64, f64) {
    if stand.marker {
        (0.0, 0.0)
    } else if stand.small {
        (0.25, f64::from(0.9875f32))
    } else {
        (0.5, f64::from(1.975f32))
    }
}

impl Entity for ArmorStand {
    fn native_type(&self) -> Cow<'static, str> {
        "minecraft:armor_stand".into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_armor_stand(index, keys)
    }
    stored_motion!();
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Stand(self.id))
    }
    /// `ArmorStand.ignoreExplosion`: a visible stand feels only a burst that
    /// affects block-like entities (`items_ignore` is that flag, negated).
    fn ignores_explosion(&self, blast: &Blast) -> bool {
        self.invisible || blast.items_ignore
    }
    /// `EntityType.ARMOR_STAND`'s size, halved when small; a marker has none.
    fn explosion_box(&self) -> Aabb {
        let (w, h) = armor_stand_size(self);
        let half = w / 2.0;
        let p = self.position;
        Aabb::new(
            Vec3::new(p.x - half, p.y, p.z - half),
            Vec3::new(p.x + half, p.y + h, p.z + half),
        )
    }
    fn explosion_eye_height(&self) -> f64 {
        if self.marker {
            0.0
        } else if self.small {
            f64::from(0.9875f32)
        } else {
            f64::from(1.7775f32)
        }
    }
    /// A burst of wind pushes a stand that a blast only breaks.
    fn pushed_by_wind(&self) -> bool {
        true
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
    /// A NoGravity stand is not swept for the blocks it stands in (plates,
    /// fire): measured beside the real server, it never presses a plate nor
    /// catches fire, and does again once `NoGravity` is cleared.
    fn sweeps_blocks_inside(&self) -> bool {
        !self.no_gravity
    }
    fn pushed_by_explosion(&self) -> bool {
        false
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
    fn native_type(&self) -> Cow<'static, str> {
        "minecraft:tnt".into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_tnt(index, keys)
    }
    stored_motion!();
    /// `PrimedTnt` eye height: `0.98 * 0.85`.
    fn fluid_eye_y(&self) -> f64 {
        self.position.y + f64::from(0.98f32 * 0.85f32)
    }
    /// `PrimedTnt` is 0.98 across.
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((0.98, 0.98))
    }
    fn can_use_portal(&self, game: &Game<'_>) -> bool {
        game.tnt_can_use_portal(self.id)
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Tnt(self.id))
    }
    /// Pushed from its position, not its eyes.
    fn explosion_eye_height(&self) -> f64 {
        0.0
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
    fn native_type(&self) -> Cow<'static, str> {
        "minecraft:falling_block".into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_falling(index, keys)
    }
    stored_motion!();
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Falling(self.id))
    }
    fn bounds(&self) -> Aabb {
        horizon_client::falling_block::bounds(self.position)
    }
}

impl Entity for Projectile {
    fn native_type(&self) -> Cow<'static, str> {
        self.kind.id().into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_projectile(index, keys)
    }
    stored_motion!();
    /// Arrows and the thrown items and fireballs a chunk saves.
    fn portal_size(&self) -> Option<(f32, f32)> {
        (super::nether_portal::is_arrow(self) || super::nether_portal::is_portal_projectile(self))
            .then(|| super::nether_portal::projectile_box(self))
    }
    fn portal_from(&self) -> Option<Vec3> {
        Some(self.previous)
    }
    fn portal_sweeps(&self) -> bool {
        !super::nether_portal::is_arrow(self) || !self.stuck || self.previous != self.position
    }
    fn selector_target(&self) -> Option<Target> {
        use horizon_client::projectiles::ProjectileKind as K;
        // The kinds /data can write (entity_nbt).
        let selectable = self.is_arrow()
            || horizon_client::entity_nbt::saved_with_chunk(self)
            || matches!(
                self.kind,
                K::LargeFireball { .. } | K::SmallFireball | K::WindCharge | K::BreezeWindCharge
            );
        selectable.then_some(Target::Projectile(self.id))
    }
    /// `AbstractArrow.push` does nothing in the ground; a wind charge's motion
    /// stays 0.
    fn pushed_by_explosion(&self) -> bool {
        !self.stuck && !self.kind.ignores_blast()
    }
    fn explosion_eye_height(&self) -> f64 {
        self.kind.eye_height()
    }
    fn push_from_explosion(&mut self, push: Vec3) {
        self.velocity = Vec3::new(
            self.velocity.x + push.x,
            self.velocity.y + push.y,
            self.velocity.z + push.z,
        );
        self.needs_sync = true;
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
    fn native_type(&self) -> Cow<'static, str> {
        "minecraft:firework_rocket".into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_rocket(index, keys)
    }
    stored_motion!();
    /// A rocket that carries its owner stays.
    fn portal_size(&self) -> Option<(f32, f32)> {
        (self.attached_to.is_none() && !self.attached).then_some((0.25, 0.25))
    }
    fn portal_from(&self) -> Option<Vec3> {
        Some(self.previous)
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Rocket(self.id))
    }
    /// 0.25 wide, so 0.2125 to the eyes.
    fn explosion_eye_height(&self) -> f64 {
        0.2125
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
    fn native_type(&self) -> Cow<'static, str> {
        "minecraft:trident".into()
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        game.save_trident(index, keys)
    }
    stored_motion!();
    fn portal_size(&self) -> Option<(f32, f32)> {
        Some((0.5, 0.5))
    }
    fn portal_from(&self) -> Option<Vec3> {
        Some(self.previous)
    }
    fn portal_sweeps(&self) -> bool {
        !self.in_ground || self.previous != self.position
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Trident(self.id))
    }
    /// Like any arrow (`AbstractArrow.push`) not pushed while in the ground.
    fn pushed_by_explosion(&self) -> bool {
        !self.in_ground
    }
    /// `EntityType.TRIDENT`: eyes at 0.13.
    fn explosion_eye_height(&self) -> f64 {
        f64::from(0.13_f32)
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

impl EntityRules for Rocket {}
impl EntityRules for ThrownTrident {}
impl EntityRules for Boat {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
    /// `AbstractBoat.getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        0.04
    }
}
impl EntityRules for Minecart {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
    /// `AbstractMinecart.getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        if self.in_water { 0.005 } else { 0.04 }
    }
}
impl EntityRules for PrimedTnt {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
    /// `PrimedTnt.getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        0.04
    }
}
impl EntityRules for FallingBlock {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
    /// `FallingBlockEntity.getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        0.04
    }
}

#[cfg(test)]
mod no_gravity_stand_tests {
    use super::Entity as _;
    use horizon_client::entities::ArmorStand;
    use horizon_core::geometry::Vec3;

    #[test]
    fn a_no_gravity_stand_is_not_swept_for_the_blocks_it_is_in() {
        let mut stand = ArmorStand::new(1, Vec3::new(0.5, 100.0, 0.5), 0.0);
        // A resting stand presses a plate (measured beside the real server)...
        assert!(stand.sweeps_blocks_inside() && stand.triggers_blocks());
        // ...a NoGravity one does not, nor does a marker.
        stand.no_gravity = true;
        assert!(!stand.sweeps_blocks_inside());
        stand.no_gravity = false;
        stand.marker = true;
        assert!(!stand.triggers_blocks());
    }
}

#[cfg(test)]
mod trident_selector_tests {
    use super::Entity as _;
    use crate::game::{commands::Target, trident::ThrownTrident};
    use horizon_core::geometry::Vec3;

    /// `@e`, `/kill` and `/data` reach a thrown trident like any other entity
    /// (`EntityTypeTest.forClass(Entity.class)`: every entity in the level).
    #[test]
    fn a_thrown_trident_is_a_selector_target() {
        let t = ThrownTrident::remote(9, Vec3::new(0.5, 100.0, 0.5), 0.0, 0.0);
        assert_eq!(t.selector_target(), Some(Target::Trident(9)));
    }
}
