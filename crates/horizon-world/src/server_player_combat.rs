//! Server-player outer hurt composed with registered attributes and player storage.
use crate::{
    attribute_health::HealthAttributes,
    attributes::entity::EntityAttributes,
    combat_rules::ArmorWorld,
    damage_immunity::{self, ImmunityWorld},
    food::{Difficulty, FoodData},
    living_hurt::{HurtState, HurtTag, HurtWorld},
    player_health::PlayerFloatIds,
    player_hurt::{
        CausingEntity, PlayerHurtWorld, ServerPlayerHurtWorld, server_player_hurt_server,
    },
    server_player_damage::{ServerDamageEnvironment, ServerDamageError, ServerPlayerDamage},
    synced_floats::{DataError, SyncedFloats},
};
use std::io;
type Error<E> = <E as ArmorWorld>::Error;
pub trait CombatEnvironment<A: HealthAttributes = EntityAttributes>:
    ServerDamageEnvironment<A>
    + ServerPlayerHurtWorld<Error = <Self as ArmorWorld>::Error>
    + ImmunityWorld<Error = <Self as ArmorWorld>::Error>
{
    /// Direct death flag, read only after health fails the native <= zero test.
    fn dead_flag(&self) -> bool;
}
pub struct ServerPlayerCombat<'a, E, A: HealthAttributes = EntityAttributes> {
    attributes: &'a mut A,
    data: &'a mut SyncedFloats,
    food: &'a mut FoodData,
    ids: PlayerFloatIds,
    environment: &'a mut E,
}
impl<'a, A: HealthAttributes, E: CombatEnvironment<A>> ServerPlayerCombat<'a, E, A> {
    pub fn new(
        attributes: &'a mut A,
        data: &'a mut SyncedFloats,
        food: &'a mut FoodData,
        ids: PlayerFloatIds,
        environment: &'a mut E,
    ) -> io::Result<Self> {
        ServerPlayerDamage::new(attributes, data, food, ids, environment)?;
        Ok(Self {
            attributes,
            data,
            food,
            ids,
            environment,
        })
    }
    pub fn hurt(
        &mut self,
        state: &mut HurtState<E::Source>,
        damage: f32,
    ) -> Result<bool, ServerDamageError<Error<E>, A::Error>> {
        server_player_hurt_server(state, self, damage)
    }
}
impl<A: HealthAttributes, E: CombatEnvironment<A>> HurtWorld for ServerPlayerCombat<'_, E, A> {
    type Error = ServerDamageError<Error<E>, A::Error>;
    type Source = E::Source;
    type UseItem = E::UseItem;
    type Blocking = E::Blocking;
    type Sound = E::Sound;
    fn source(&self) -> Self::Source {
        self.environment.source()
    }
    fn is_server_player(&self) -> bool {
        true
    }
    fn dead_or_dying(&mut self, _: &mut HurtState<Self::Source>) -> Result<bool, Self::Error> {
        let health = self
            .data
            .get(self.ids.health)
            .map_err(|id| ServerDamageError::Data(DataError::UnknownId(id)))?;
        Ok(health <= 0.0 || self.environment.dead_flag())
    }
    fn actually_hurt(
        &mut self,
        _: &mut HurtState<Self::Source>,
        damage: f32,
    ) -> Result<(), Self::Error> {
        if damage_immunity::server_player_immunity(self.environment)
            .map_err(ServerDamageError::Environment)?
        {
            return Ok(());
        }
        ServerPlayerDamage::new(
            self.attributes,
            self.data,
            self.food,
            self.ids,
            self.environment,
        )
        .expect("validated immutable player schema")
        .apply_after_immunity_check(damage)
    }
    fn blocking_component(&mut self, item: Self::UseItem) -> Option<Self::Blocking> {
        self.environment.blocking_component(item)
    }
    fn invulnerable(&mut self, _: &mut HurtState<Self::Source>) -> Result<bool, Self::Error> {
        damage_immunity::server_player_immunity(self.environment)
            .map_err(ServerDamageError::Environment)
    }
    fn tag(
        &mut self,
        state: &mut HurtState<Self::Source>,
        tag: HurtTag,
    ) -> Result<bool, Self::Error> {
        HurtWorld::tag(self.environment, state, tag).map_err(ServerDamageError::Environment)
    }
    fn fire_resistance(
        &mut self,
        state: &mut HurtState<Self::Source>,
    ) -> Result<bool, Self::Error> {
        HurtWorld::fire_resistance(self.environment, state).map_err(ServerDamageError::Environment)
    }
    fn hurt_start(
        &mut self,
        state: &mut HurtState<Self::Source>,
        damage: &mut f32,
    ) -> Result<bool, Self::Error> {
        HurtWorld::hurt_start(self.environment, state, damage)
            .map_err(ServerDamageError::Environment)
    }
    fn sleeping(&mut self, state: &mut HurtState<Self::Source>) -> Result<bool, Self::Error> {
        HurtWorld::sleeping(self.environment, state).map_err(ServerDamageError::Environment)
    }
    fn stop_sleeping(&mut self, state: &mut HurtState<Self::Source>) -> Result<(), Self::Error> {
        HurtWorld::stop_sleeping(self.environment, state).map_err(ServerDamageError::Environment)
    }
    fn use_item(
        &mut self,
        state: &mut HurtState<Self::Source>,
    ) -> Result<Self::UseItem, Self::Error> {
        HurtWorld::use_item(self.environment, state).map_err(ServerDamageError::Environment)
    }
    fn apply_blocking(
        &mut self,
        state: &mut HurtState<Self::Source>,
        damage: f32,
    ) -> Result<f32, Self::Error> {
        HurtWorld::apply_blocking(self.environment, state, damage)
            .map_err(ServerDamageError::Environment)
    }
    fn freeze_hurts_extra(
        &mut self,
        state: &mut HurtState<Self::Source>,
    ) -> Result<bool, Self::Error> {
        HurtWorld::freeze_hurts_extra(self.environment, state)
            .map_err(ServerDamageError::Environment)
    }
    fn helmet_empty(&mut self, state: &mut HurtState<Self::Source>) -> Result<bool, Self::Error> {
        HurtWorld::helmet_empty(self.environment, state).map_err(ServerDamageError::Environment)
    }
    fn hurt_helmet(
        &mut self,
        state: &mut HurtState<Self::Source>,
        damage: f32,
    ) -> Result<(), Self::Error> {
        HurtWorld::hurt_helmet(self.environment, state, damage)
            .map_err(ServerDamageError::Environment)
    }
    fn resolve_mob(&mut self, state: &mut HurtState<Self::Source>) -> Result<(), Self::Error> {
        HurtWorld::resolve_mob(self.environment, state).map_err(ServerDamageError::Environment)
    }
    fn resolve_player(&mut self, state: &mut HurtState<Self::Source>) -> Result<(), Self::Error> {
        HurtWorld::resolve_player(self.environment, state).map_err(ServerDamageError::Environment)
    }
    fn on_blocked(
        &mut self,
        state: &mut HurtState<Self::Source>,
        block: Self::Blocking,
    ) -> Result<(), Self::Error> {
        HurtWorld::on_blocked(self.environment, state, block)
            .map_err(ServerDamageError::Environment)
    }
    fn broadcast_damage(&mut self, state: &mut HurtState<Self::Source>) -> Result<(), Self::Error> {
        HurtWorld::broadcast_damage(self.environment, state).map_err(ServerDamageError::Environment)
    }
    fn mark_hurt(&mut self, state: &mut HurtState<Self::Source>) -> Result<(), Self::Error> {
        HurtWorld::mark_hurt(self.environment, state).map_err(ServerDamageError::Environment)
    }
    fn knockback(
        &mut self,
        state: &mut HurtState<Self::Source>,
        damage: f32,
        blocked: bool,
    ) -> Result<(), Self::Error> {
        HurtWorld::knockback(self.environment, state, damage, blocked)
            .map_err(ServerDamageError::Environment)
    }
    fn death_protection(
        &mut self,
        state: &mut HurtState<Self::Source>,
    ) -> Result<bool, Self::Error> {
        HurtWorld::death_protection(self.environment, state).map_err(ServerDamageError::Environment)
    }
    fn death_sound(
        &mut self,
        state: &mut HurtState<Self::Source>,
    ) -> Result<Self::Sound, Self::Error> {
        HurtWorld::death_sound(self.environment, state).map_err(ServerDamageError::Environment)
    }
    fn make_sound(
        &mut self,
        state: &mut HurtState<Self::Source>,
        sound: Self::Sound,
    ) -> Result<(), Self::Error> {
        HurtWorld::make_sound(self.environment, state, sound)
            .map_err(ServerDamageError::Environment)
    }
    fn secondary_sound(&mut self, state: &mut HurtState<Self::Source>) -> Result<(), Self::Error> {
        HurtWorld::secondary_sound(self.environment, state).map_err(ServerDamageError::Environment)
    }
    fn die(&mut self, state: &mut HurtState<Self::Source>) -> Result<(), Self::Error> {
        HurtWorld::die(self.environment, state).map_err(ServerDamageError::Environment)
    }
    fn hurt_sound(&mut self, state: &mut HurtState<Self::Source>) -> Result<(), Self::Error> {
        HurtWorld::hurt_sound(self.environment, state).map_err(ServerDamageError::Environment)
    }
    fn game_time(&mut self, state: &mut HurtState<Self::Source>) -> Result<i64, Self::Error> {
        HurtWorld::game_time(self.environment, state).map_err(ServerDamageError::Environment)
    }
    fn effects_on_hurt(
        &mut self,
        state: &mut HurtState<Self::Source>,
        damage: f32,
    ) -> Result<(), Self::Error> {
        HurtWorld::effects_on_hurt(self.environment, state, damage)
            .map_err(ServerDamageError::Environment)
    }
    fn player_hurt_criterion(
        &mut self,
        state: &mut HurtState<Self::Source>,
        original: f32,
        damage: f32,
        blocked: bool,
    ) -> Result<(), Self::Error> {
        HurtWorld::player_hurt_criterion(self.environment, state, original, damage, blocked)
            .map_err(ServerDamageError::Environment)
    }
    fn award_blocked(
        &mut self,
        state: &mut HurtState<Self::Source>,
        amount: i32,
    ) -> Result<(), Self::Error> {
        HurtWorld::award_blocked(self.environment, state, amount)
            .map_err(ServerDamageError::Environment)
    }
    fn attacker_criterion(
        &mut self,
        state: &mut HurtState<Self::Source>,
        original: f32,
        damage: f32,
        blocked: bool,
    ) -> Result<(), Self::Error> {
        HurtWorld::attacker_criterion(self.environment, state, original, damage, blocked)
            .map_err(ServerDamageError::Environment)
    }
}
impl<A: HealthAttributes, E: CombatEnvironment<A>> PlayerHurtWorld
    for ServerPlayerCombat<'_, E, A>
{
    fn ability_invulnerable(&self) -> bool {
        ServerDamageEnvironment::ability_invulnerable(self.environment)
    }
    fn bypasses_invulnerability(
        &mut self,
        state: &mut HurtState<Self::Source>,
    ) -> Result<bool, Self::Error> {
        PlayerHurtWorld::bypasses_invulnerability(self.environment, state)
            .map_err(ServerDamageError::Environment)
    }
    fn remove_shoulders(&mut self, state: &mut HurtState<Self::Source>) -> Result<(), Self::Error> {
        PlayerHurtWorld::remove_shoulders(self.environment, state)
            .map_err(ServerDamageError::Environment)
    }
    fn scales_with_difficulty(
        &mut self,
        state: &mut HurtState<Self::Source>,
    ) -> Result<bool, Self::Error> {
        PlayerHurtWorld::scales_with_difficulty(self.environment, state)
            .map_err(ServerDamageError::Environment)
    }
    fn difficulty(
        &mut self,
        state: &mut HurtState<Self::Source>,
    ) -> Result<Difficulty, Self::Error> {
        PlayerHurtWorld::difficulty(self.environment, state).map_err(ServerDamageError::Environment)
    }
}
impl<A: HealthAttributes, E: CombatEnvironment<A>> ServerPlayerHurtWorld
    for ServerPlayerCombat<'_, E, A>
{
    type Player = E::Player;
    type Arrow = E::Arrow;
    fn causing_entity(
        &mut self,
        state: &mut HurtState<Self::Source>,
    ) -> Result<CausingEntity<Self::Player, Self::Arrow>, Self::Error> {
        self.environment
            .causing_entity(state)
            .map_err(ServerDamageError::Environment)
    }
    fn arrow_player_owner(
        &mut self,
        state: &mut HurtState<Self::Source>,
        arrow: Self::Arrow,
    ) -> Result<Option<Self::Player>, Self::Error> {
        self.environment
            .arrow_player_owner(state, arrow)
            .map_err(ServerDamageError::Environment)
    }
    fn can_harm_player(
        &mut self,
        state: &mut HurtState<Self::Source>,
        player: Self::Player,
    ) -> Result<bool, Self::Error> {
        self.environment
            .can_harm_player(state, player)
            .map_err(ServerDamageError::Environment)
    }
}
