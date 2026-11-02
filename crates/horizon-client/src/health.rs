//! Stored player health and empty-equipment server damage for the resident scene.
//! World/audio/network consumers are queued explicitly; full death and tick
//! lifecycles (drops, respawn, effects) remain separate work.
use crate::{
    Result,
    motion::{MotionEvent, PlayerMotion},
};
use horizon_world::{
    attributes::live::LiveAttributes,
    block_landing::LandingDamage,
    combat_rules::ArmorWorld,
    damage_immunity::{self, DamageRule, ImmunityTag, ImmunityWorld},
    damage_reduction::Bypass,
    damage_source::{Catalog, DamageSource, DamageTypeId, Scaling},
    food::{Difficulty, FoodData},
    living_damage::DamageStat,
    living_hurt::{HurtState, HurtTag, HurtWorld},
    player_health::{PlayerAttributeHealth, PlayerFloatIds},
    player_hurt::{CausingEntity, PlayerHurtWorld, ServerPlayerHurtWorld},
    server_player_combat::{CombatEnvironment, ServerPlayerCombat},
    server_player_damage::ServerDamageEnvironment,
    synced_floats::{FloatValue, Observer, SyncedFloats},
};
use std::{fs, io, path::Path, sync::OnceLock};
pub(crate) type Attributes = LiveAttributes<Box<dyn horizon_world::attributes::live::Identities>>;
pub(crate) type State = HurtState<DamageTypeId>;

pub use crate::world::HealthRules;
/// A hit's `DamageSource` for [`HURT_SERVER`]: its type (`minecraft:fall`),
/// and the entity ids of `getEntity` (`causing`) and `getDirectEntity`
/// (`direct`), 0 for none or not known.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HurtSource<'a> {
    pub damage_type: &'a str,
    pub causing: i32,
    pub direct: i32,
}
/// What `LivingEntity.hurtServer` is about to hurt, for [`HURT_SERVER`], or
/// `LivingEntity.knockback` to push, for [`KNOCKBACK`]: the entity and the
/// hit's source.
pub enum Hurting<'a> {
    /// A player, by its entity id.
    Player(i32, HurtSource<'a>),
    /// A mob, and what hurts it.
    Mob(&'a crate::mobs::Mob, &'a crate::mobs::Damage),
    /// The ender dragon, by its entity id.
    Dragon(i32, HurtSource<'a>),
}
/// The game's native damage hook (`native_living_incoming_damage`), set once by the
/// game: `LivingEntity.hurtServer` once its guards passed. It may change the
/// amount; true cancels the hit.
pub static HURT_SERVER: OnceLock<fn(Hurting<'_>, &mut f32) -> bool> = OnceLock::new();
/// Whether [`HURT_SERVER`] canceled the hit (never while it is unset).
pub fn hurt_server_canceled(hurting: Hurting<'_>, amount: &mut f32) -> bool {
    HURT_SERVER.get().is_some_and(|hook| hook(hurting, amount))
}
/// A knockback's strength and direction, as `LivingEntity.knockback` has them
/// once the knockback resistance is applied. The entity is pushed away from
/// the direction (`x`, `z`), which vanilla points at the source of the hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knock {
    pub strength: f64,
    pub x: f64,
    pub z: f64,
}
/// The game's native knockback hook (`native_living_knockback`), set once by
/// the game: `LivingEntity.knockback`, at its start (`LivingKnockBackEvent`), on the raw strength
/// and before the knockback resistance and the motion change. It may change the [`Knock`]; true
/// cancels it.
pub static KNOCKBACK: OnceLock<fn(Hurting<'_>, &mut Knock) -> bool> = OnceLock::new();
/// Whether [`KNOCKBACK`] canceled the knockback (never while it is unset). A
/// value it leaves that is not a finite number is put back as it was.
pub fn knockback_canceled(pushed: Hurting<'_>, knock: &mut Knock) -> bool {
    let Some(hook) = KNOCKBACK.get() else {
        return false;
    };
    let before = *knock;
    let canceled = hook(pushed, knock);
    for (now, was) in [
        (&mut knock.strength, before.strength),
        (&mut knock.x, before.x),
        (&mut knock.z, before.z),
    ] {
        if !now.is_finite() {
            *now = was;
        }
    }
    canceled
}
pub struct HealthResources {
    pub damage: Catalog,
    pub ids: PlayerFloatIds,
}
impl HealthResources {
    pub fn load(root: &Path) -> Result<Self> {
        let read = |name: &str| -> Result<serde_json::Value> {
            Ok(serde_json::from_slice(&fs::read(root.join(name))?)?)
        };
        let ids = read("player-float-ids.json")?;
        let id = |name: &str| -> Result<u8> {
            Ok(u8::try_from(
                ids[name].as_u64().ok_or("missing player float ID")?,
            )?)
        };
        let damage = Catalog::from_json(&read("damage-catalog.json")?, &read("damage-tags.json")?)?;
        for name in ["minecraft:fall", "minecraft:stalagmite", "minecraft:starve"] {
            if damage.id(name).is_none() {
                return Err(format!("missing damage type {name}").into());
            }
        }
        Ok(Self {
            damage,
            ids: PlayerFloatIds {
                health: id("health")?,
                absorption: id("absorption")?,
            },
        })
    }
    pub fn landing_source(&self, source: LandingDamage) -> DamageTypeId {
        self.damage
            .id(match source {
                LandingDamage::Fall => "minecraft:fall",
                LandingDamage::Stalagmite => "minecraft:stalagmite",
            })
            .expect("validated landing source")
    }
}
/// `DamageEffects.sound()` by its serialized id: what `Player.getHurtSound`
/// plays for a damage type's `effects` (unknown ids fall back to `hurt`, the codec default).
pub fn hurt_sound(effects: &str) -> &'static str {
    match effects {
        "drowning" => "minecraft:entity.player.hurt_drown",
        "burning" => "minecraft:entity.player.hurt_on_fire",
        "poking" => "minecraft:entity.player.hurt_sweet_berry_bush",
        "freezing" => "minecraft:entity.player.hurt_freeze",
        // HURT and THORNS (both SoundEvents.PLAYER_HURT).
        _ => "minecraft:entity.player.hurt",
    }
}
#[derive(Clone, Copy, Debug)]
pub enum HealthEvent {
    /// `FoodData.tick` calling `hurtServer` with starvation. Only a hit that
    /// lands is recorded (`RecordedDamage`).
    StarvationDamage(f32),
    /// `LivingEntity.aiStep` hurting a fully frozen entity.
    FreezeDamage(f32),
    /// `SweetBerryBushBlock.entityInside` pricking a moving entity.
    SweetBerryBushDamage(f32),
    /// `LivingEntity.baseTick` drowning an entity out of air.
    DrownDamage(f32),
    Updated {
        id: u8,
        value: f32,
        dirty: bool,
    },
    Statistic {
        name: &'static str,
        amount: i32,
    },
    RecordedDamage {
        source: DamageTypeId,
        amount: f32,
    },
    Phase(&'static str),
    /// `Player.hurtArmor`: a hit that doesn't bypass armor wears the worn
    /// pieces (`LivingEntity.getDamageAfterArmorAbsorb`); the game applies it.
    ArmorHurt {
        source: DamageTypeId,
        amount: f32,
    },
    /// `ServerLevel.broadcastDamageEvent`: the client's
    /// `LivingEntity.handleDamageEvent` plays the source's hurt sound.
    DamageBroadcast {
        source: DamageTypeId,
    },
    Criterion {
        original: f32,
        amount: f32,
        blocked: bool,
    },
}
impl HealthEvent {
    pub fn trace(self) -> String {
        match self {
            Self::StarvationDamage(amount) => format!("damage starve {:08x}", amount.to_bits()),
            Self::FreezeDamage(amount) => format!("damage freeze {:08x}", amount.to_bits()),
            Self::SweetBerryBushDamage(amount) => {
                format!("damage sweet_berry_bush {:08x}", amount.to_bits())
            }
            Self::DrownDamage(amount) => format!("damage drown {:08x}", amount.to_bits()),
            Self::Updated { id, value, dirty } => {
                format!("updated {id} {:08x} {dirty}", value.to_bits())
            }
            Self::Statistic { name, amount } => format!("health-stat {name} {amount}"),
            Self::RecordedDamage { amount, .. } => {
                format!("record-damage {:08x}", amount.to_bits())
            }
            Self::Phase(name) => name.into(),
            Self::ArmorHurt { amount, .. } => format!("hurt-armor {:08x}", amount.to_bits()),
            Self::DamageBroadcast { .. } => "broadcast-damage".into(),
            Self::Criterion { .. } => "hurt-criterion".into(),
        }
    }
}
pub(crate) struct Updates<'a>(pub(crate) &'a mut Vec<MotionEvent>);
impl Observer for Updates<'_> {
    type Error = io::Error;
    fn updated(&mut self, data: &mut SyncedFloats, id: u8) -> io::Result<()> {
        self.0.push(MotionEvent::Health(HealthEvent::Updated {
            id,
            value: data.get(id).expect("existing updated field"),
            dirty: data.is_dirty(),
        }));
        Ok(())
    }
}
#[derive(Clone)]
pub struct PlayerHealth {
    pub data: SyncedFloats,
    pub ids: PlayerFloatIds,
    pub food: FoodData,
    pub hurt: State,
    /// Client hurt animation; fully blocked hits only refresh the server's hurt state.
    pub hurt_animation_time: i32,
    /// `Player.hurtDir` on a client: the yaw `ClientboundHurtAnimationPacket`
    /// gave, which `GameRenderer.bobHurt` tilts the camera around.
    pub hurt_dir: f32,
    /// `ServerPlayer.hurtDir` of a hit the player's client hasn't been told
    /// of yet (`indicateDamage`).
    pub indicated_dir: Option<f32>,
    pub dead: bool,
    pub sync_velocity: bool,
    pub game_time: i64,
    pub tick_count: i32,
    /// A death-protection item (totem) is in either hand.
    pub totem_ready: bool,
    /// The totem saved the player this tick; the game applies its effects.
    pub totem_used: bool,
}
impl PlayerHealth {
    pub fn new(resources: &HealthResources, attributes: &mut Attributes) -> Result<Self> {
        let ids = resources.ids;
        let mut data = SyncedFloats::new(&[
            FloatValue {
                id: ids.health,
                value: 1.0,
            },
            FloatValue {
                id: ids.absorption,
                value: 0.0,
            },
        ])?;
        let mut ignored = Vec::new();
        let mut observer = Updates(&mut ignored);
        let maximum = attributes
            .key("minecraft:max_health")
            .ok_or("missing max health")?;
        let maximum = attributes
            .value(
                Some(maximum),
                horizon_world::attribute_map::ValueKind::Computed,
            )
            .map_err(|e| io::Error::other(format!("health attribute: {e:?}")))?
            as f32;
        PlayerAttributeHealth::new(attributes, &mut data, ids, false, &mut observer)?
            .set_health(maximum)?;
        Ok(Self {
            data,
            ids,
            food: FoodData::default(),
            hurt: HurtState {
                no_action_time: 0,
                cooldown: 0,
                last_hurt: 0.0,
                hurt_duration: 0,
                hurt_time: 0,
                last_source: None,
                last_stamp: 0,
            },
            hurt_animation_time: 0,
            hurt_dir: 0.0,
            indicated_dir: None,
            dead: false,
            sync_velocity: false,
            game_time: 0,
            tick_count: 0,
            totem_ready: false,
            totem_used: false,
        })
    }
    pub fn health(&self) -> f32 {
        self.data
            .get(self.ids.health)
            .expect("validated health field")
    }
    pub fn absorption(&self) -> f32 {
        self.data
            .get(self.ids.absorption)
            .expect("validated absorption field")
    }
    /// The game time the damage clock has counted.
    pub fn game_time(&self) -> i64 {
        self.game_time
    }
    pub fn dead_or_dying(&self) -> bool {
        self.health() <= 0.0 || self.dead
    }
    /// The damage-clock phases of ServerPlayer.tick and LivingEntity.baseTick.
    /// The complete enclosing ticks, including death removal, are not yet bound.
    pub fn advance_clock(&mut self) {
        self.game_time = self.game_time.wrapping_add(1);
        self.tick_count = self.tick_count.wrapping_add(1);
        if self.hurt.cooldown > 0 {
            self.hurt.cooldown -= 1;
        }
        if self.hurt.hurt_time > 0 {
            self.hurt.hurt_time -= 1;
        }
        if self.hurt_animation_time > 0 {
            self.hurt_animation_time -= 1;
        }
    }
    /// `LivingEntity.animateHurt` / `handleDamageEvent` on a client: the hurt
    /// tilt and red flash restart.
    pub fn animate_hurt(&mut self) {
        self.hurt.hurt_duration = 10;
        self.hurt.hurt_time = 10;
        self.hurt_animation_time = 10;
    }
    /// `LivingEntity.animateHurt(yaw)` on a player: the tilt restarts around
    /// `yaw` (`Player.animateHurt` keeps it as `hurtDir`).
    pub fn animate_hurt_dir(&mut self, yaw: f32) {
        self.animate_hurt();
        self.hurt_dir = yaw;
    }
    /// `ServerPlayer.indicateDamage`: the hit's direction relative to where
    /// the player looks, kept for the packet to its client.
    pub fn indicate_damage(&mut self, xd: f64, zd: f64, yaw: f32) {
        use horizon_core::geometry::trig::{RAD_TO_DEG, atan2};
        self.indicated_dir = Some((atan2(zd, xd) * f64::from(RAD_TO_DEG) - f64::from(yaw)) as f32);
    }
    /// `LocalPlayer.hurtTo` with `flashOnSetHealth`: a drop below the current
    /// health starts the hurt animation; `damageCooldownTime` (20 on a drop,
    /// 10 on a gain) is what lets `Hud.extractPlayerHealth` blink the hearts.
    pub fn flash_health_change(&mut self, health: f32) {
        let damage = self.health() - health;
        if damage <= 0.0 {
            if damage < 0.0 {
                self.hurt.cooldown = 10;
            }
        } else {
            self.hurt.last_hurt = damage;
            self.hurt.cooldown = 20;
            self.animate_hurt();
        }
    }
}
impl PlayerMotion {
    pub fn hurt(
        &mut self,
        resources: &HealthResources,
        rules: HealthRules,
        source: DamageTypeId,
        amount: f32,
    ) -> Result<bool> {
        Ok(self
            .hurt_blocking(resources, rules, source, amount, None)?
            .hurt)
    }
    pub fn hurt_blocking(
        &mut self,
        resources: &HealthResources,
        rules: HealthRules,
        source: DamageTypeId,
        amount: f32,
        blocking: Option<Blocking>,
    ) -> Result<BlockOutcome> {
        self.hurt_caused_by(resources, rules, source, amount, blocking, false, 0)
    }
    /// `hurt_blocking` from a source whose causing entity is a living
    /// non-player (`living_cause`), which the `when_caused_by_living_non_player`
    /// damage types scale with the difficulty. `causing` is that entity's id
    /// when it is known (0 otherwise), for the native damage hook.
    pub fn hurt_caused_by(
        &mut self,
        resources: &HealthResources,
        rules: HealthRules,
        source: DamageTypeId,
        amount: f32,
        blocking: Option<Blocking>,
        living_cause: bool,
        causing: i32,
    ) -> Result<BlockOutcome> {
        if self.client_side {
            return Ok(BlockOutcome::default());
        }
        let source = resources
            .damage
            .source::<()>(source, None, None, None)
            .ok_or("unknown resident damage source")?;
        let armor = self.attribute("minecraft:armor").unwrap_or(0.0) as f32;
        let toughness = self.attribute("minecraft:armor_toughness").unwrap_or(0.0) as f32;
        let before = self.health.health() + self.health.absorption();
        let fire_ticks = self.fire_ticks;
        let source_name = source.definition().name.clone();
        let totem_ready = self.health.totem_ready;
        let invulnerable = self.invulnerable;
        let h = &mut self.health;
        let mut environment = Environment {
            entity: self.entity_id,
            causing,
            invulnerable,
            client_loaded: self.client_loaded,
            rules,
            source,
            living_cause,
            events: &mut self.events,
            dead: &mut h.dead,
            sync_velocity: &mut h.sync_velocity,
            game_time: h.game_time,
            status: self.status.clone(),
            blocking,
            blocked: 0.0,
            fully_blocked: false,
            block_sound: false,
            knocked: false,
            totem: totem_ready,
            totem_fired: false,
            canceled: false,
        };
        let hurt = ServerPlayerCombat::new(
            &mut self.attributes,
            &mut h.data,
            &mut h.food,
            h.ids,
            &mut environment,
        )?
        .hurt(&mut h.hurt, amount)?;
        let outcome = BlockOutcome {
            hurt,
            blocked: environment.blocked,
            fully_blocked: environment.fully_blocked,
            sound: environment.block_sound,
            knockback: environment.knocked,
            canceled: environment.canceled,
        };
        if environment.totem_fired {
            h.totem_used = true;
            h.totem_ready = false;
        }
        if outcome.hurt {
            h.hurt_animation_time = h.hurt.hurt_time;
        }
        // `dealDefaultKnockback` ends in `indicateDamage` unless blocked: a
        // hit with no direction of its own (`xd` and `zd` zero) is sent as
        // the opposite of the player's yaw. A hit that has a direction
        // replaces this (`mob_hits_player`).
        if outcome.knockback && outcome.blocked <= 0.0 {
            h.indicate_damage(0.0, 0.0, self.yaw);
        }
        let after = h.health() + h.absorption();
        if self.hurt_log.len() >= HURT_LOG {
            self.hurt_log.remove(0);
        }
        self.hurt_log.push(HurtRecord {
            game_time: h.game_time,
            source: source_name,
            amount,
            blocked: outcome.blocked,
            taken: before - after,
            hurt: outcome.hurt,
            armor,
            toughness,
            health: after,
            fire_ticks,
            knockback: None,
        });
        Ok(outcome)
    }
    pub fn set_health(&mut self, value: f32, absorption: bool) -> Result<()> {
        let h = &mut self.health;
        let mut observer = Updates(&mut self.events);
        let mut binding = PlayerAttributeHealth::new(
            &mut self.attributes,
            &mut h.data,
            h.ids,
            h.dead,
            &mut observer,
        )?;
        if absorption {
            binding.set_absorption(value)?;
        } else {
            binding.set_health(value)?;
        }
        Ok(())
    }
}
pub(crate) struct Environment<'a> {
    /// The player's entity id, and the causing entity's (0: none or not known).
    pub(crate) entity: i32,
    pub(crate) causing: i32,
    /// Abilities.invulnerable.
    pub(crate) invulnerable: bool,
    /// The player's client has loaded the world since its last respawn or
    /// level change (`ServerGamePacketListenerImpl.hasClientLoaded`).
    pub(crate) client_loaded: bool,
    pub(crate) rules: HealthRules,
    pub(crate) source: DamageSource<'a, ()>,
    /// The causing entity is a living non-player (a mob, or its projectile's owner).
    pub(crate) living_cause: bool,
    pub(crate) events: &'a mut Vec<MotionEvent>,
    pub(crate) dead: &'a mut bool,
    pub(crate) sync_velocity: &'a mut bool,
    pub(crate) game_time: i64,
    /// Resistance, fire resistance and armor protection (effects.rs).
    pub(crate) status: crate::effects::StatusFlags,
    /// The raised blocking item, if any, and what it blocked.
    pub(crate) blocking: Option<Blocking>,
    pub(crate) blocked: f32,
    /// What it blocked covers the whole hit, as difficulty scaled it.
    pub(crate) fully_blocked: bool,
    pub(crate) block_sound: bool,
    /// `dealDefaultKnockback` ran: a fresh hit, not fully blocked.
    pub(crate) knocked: bool,
    /// LivingEntity.checkTotemDeathProtection: a totem is held / was used.
    pub(crate) totem: bool,
    pub(crate) totem_fired: bool,
    /// A native mod canceled the hit (`living_incoming_damage`).
    pub(crate) canceled: bool,
}
/// `LivingEntity.applyItemBlocking` inputs for a raised `BlocksAttacks` item:
/// reductions whose damage-type sets are already matched against the source.
#[derive(Clone, Debug)]
pub struct Blocking {
    pub rules: horizon_world::blocking_rules::BlockingRules<bool>,
    /// `bypassed_by` contains the source's damage type.
    pub bypassed: bool,
    /// The direct entity is an arrow with piercing.
    pub pierce: bool,
    /// Angle between the head's view and the source position (pi without one).
    pub angle: f64,
}
/// How many hurts `PlayerMotion::hurt_log` keeps.
const HURT_LOG: usize = 64;
/// One hurt as it was taken: what came in, what a raised blocking item
/// stopped, the armour it met and what health it left.
#[derive(Clone, Debug, PartialEq)]
pub struct HurtRecord {
    pub game_time: i64,
    /// The damage type.
    pub source: String,
    /// The damage before blocking, armour and effects.
    pub amount: f32,
    pub blocked: f32,
    /// Health and absorption lost.
    pub taken: f32,
    pub hurt: bool,
    /// `minecraft:armor` and `minecraft:armor_toughness` when it came.
    pub armor: f32,
    pub toughness: f32,
    pub health: f32,
    /// The fire ticks before the hurt.
    pub fire_ticks: i32,
    /// The push the hurt gave, if the caller applied one.
    pub knockback: Option<[f64; 3]>,
}
/// One melee attack on a mob: the damage after the attack strength, the
/// strength (`getAttackStrengthScale`), and the health it took.
#[derive(Clone, Debug, PartialEq)]
pub struct AttackRecord {
    pub game_time: i64,
    pub target: String,
    /// The mob's entity id, and whether the blow killed it.
    pub id: u32,
    pub killed: bool,
    pub amount: f32,
    pub scale: f32,
    pub crit: bool,
    pub landed: bool,
    pub dealt: f32,
}
/// The outcome of a hurt with a raised blocking item.
#[derive(Clone, Copy, Debug, Default)]
pub struct BlockOutcome {
    pub hurt: bool,
    pub blocked: f32,
    /// `applyItemBlocking`'s `damageBlocked >= damage`, against the damage
    /// `Player.hurtServer` scaled with the difficulty.
    pub fully_blocked: bool,
    /// `BlocksAttacks.onBlocked` ran (the block sound plays).
    pub sound: bool,
    /// The hit knocks the player back (`dealDefaultKnockback`): not one in
    /// the hurt cooldown, nor one the shield took whole.
    pub knockback: bool,
    /// A native mod canceled the hit: its attacker must not be credited.
    pub canceled: bool,
}
struct MatchedSets;
impl horizon_world::blocking_rules::TypeMatchWorld for MatchedSets {
    type Error = io::Error;
    type Set = bool;
    fn source_in_set(&mut self, set: bool) -> io::Result<bool> {
        Ok(set)
    }
}
impl Environment<'_> {
    fn event(&mut self, event: HealthEvent) {
        self.events.push(MotionEvent::Health(event));
    }
    fn phase(&mut self, name: &'static str) -> io::Result<()> {
        self.event(HealthEvent::Phase(name));
        Ok(())
    }
    fn stat(&mut self, name: &'static str, amount: i32) -> io::Result<()> {
        self.event(HealthEvent::Statistic { name, amount });
        Ok(())
    }
}
impl Observer for Environment<'_> {
    type Error = io::Error;
    fn updated(&mut self, data: &mut SyncedFloats, id: u8) -> io::Result<()> {
        Updates(self.events).updated(data, id)
    }
}
impl ArmorWorld for Environment<'_> {
    type Error = io::Error;
    type Weapon = ();
    type ServerLevel = ();
    fn weapon(&mut self) -> io::Result<Option<()>> {
        Ok(None)
    }
    fn server_level(&mut self) -> io::Result<Option<()>> {
        Ok(Some(()))
    }
    fn modify_armor_effectiveness(&mut self, _: (), _: (), value: f32) -> io::Result<f32> {
        Ok(value)
    }
}
impl ServerDamageEnvironment<Attributes> for Environment<'_> {
    type Attacker = ();
    fn invulnerable(&mut self) -> io::Result<bool> {
        damage_immunity::server_player_immunity(self)
    }
    fn bypasses(&mut self, kind: Bypass) -> io::Result<bool> {
        Ok(self.source.bypasses(kind))
    }
    fn hurt_armor(&mut self, _: &mut Attributes, damage: f32) -> io::Result<()> {
        // The worn armor lives in the game, which wears it from this event.
        self.event(HealthEvent::ArmorHurt {
            source: self.source.type_id(),
            amount: damage,
        });
        Ok(())
    }
    fn has_resistance(&mut self) -> io::Result<bool> {
        Ok(self.status.resistance.is_some())
    }
    fn resistance_amplifier(&mut self) -> io::Result<i32> {
        self.status
            .resistance
            .ok_or_else(|| io::Error::other("no resistance effect"))
    }
    fn award_resisted(&mut self, value: i32) -> io::Result<()> {
        self.stat("resisted", value)
    }
    fn attacker(&mut self) -> io::Result<Option<()>> {
        Ok(None)
    }
    fn award_dealt_resisted(&mut self, _: i32) -> io::Result<()> {
        Err(io::Error::other("no damage attacker"))
    }
    fn damage_protection(&mut self, _: ()) -> io::Result<f32> {
        Ok(self.status.protection(&self.source.definition().name))
    }
    fn award_self(&mut self, stat: DamageStat, value: i32) -> io::Result<()> {
        self.stat(
            match stat {
                DamageStat::Absorbed => "absorbed",
                DamageStat::Taken => "taken",
                DamageStat::DealtAbsorbed => return Err(io::Error::other("attacker statistic")),
            },
            value,
        )
    }
    fn award_attacker(&mut self, _: (), _: DamageStat, _: i32) -> io::Result<()> {
        Err(io::Error::other("no damage attacker"))
    }
    fn source_food_exhaustion(&mut self) -> io::Result<f32> {
        Ok(self.source.definition().exhaustion)
    }
    fn ability_invulnerable(&self) -> bool {
        self.invulnerable
    }
    fn level_is_client(&mut self) -> io::Result<bool> {
        Ok(false)
    }
    fn record_damage(&mut self, amount: f32) -> io::Result<()> {
        self.event(HealthEvent::RecordedDamage {
            source: self.source.type_id(),
            amount,
        });
        Ok(())
    }
    fn damage_event(&mut self) -> io::Result<()> {
        self.phase("entity-damage")
    }
}
impl CombatEnvironment<Attributes> for Environment<'_> {
    fn dead_flag(&self) -> bool {
        *self.dead
    }
}
impl ImmunityWorld for Environment<'_> {
    type Error = io::Error;
    fn removed(&self) -> bool {
        false
    }
    fn permanently_invulnerable(&mut self) -> io::Result<bool> {
        Ok(false)
    }
    fn temporarily_invulnerable(&mut self) -> io::Result<bool> {
        Ok(false)
    }
    fn tag(&mut self, tag: ImmunityTag) -> io::Result<bool> {
        Ok(self.source.immunity_tag(tag))
    }
    fn creative_source(&mut self) -> io::Result<bool> {
        Ok(false)
    }
    fn fire_immune(&mut self) -> io::Result<bool> {
        Ok(false)
    }
    fn fall_immune(&mut self) -> io::Result<bool> {
        Ok(false)
    }
    fn enchantment_immunity(&mut self) -> io::Result<bool> {
        Ok(false)
    }
    fn damage_rule(&mut self, rule: DamageRule) -> io::Result<bool> {
        Ok(match rule {
            DamageRule::Drowning => self.rules.drowning_damage,
            DamageRule::Fall => self.rules.fall_damage,
            DamageRule::Fire => self.rules.fire_damage,
            DamageRule::Freeze => self.rules.freeze_damage,
        })
    }
    fn changing_dimension(&mut self) -> io::Result<bool> {
        Ok(false)
    }
    fn ender_pearl(&mut self) -> io::Result<bool> {
        Ok(self.source.is_type("minecraft:ender_pearl"))
    }
    fn client_loaded(&mut self) -> io::Result<bool> {
        Ok(self.client_loaded)
    }
}
impl HurtWorld for Environment<'_> {
    type Error = io::Error;
    type Source = DamageTypeId;
    type UseItem = ();
    type Blocking = ();
    type Sound = ();
    fn source(&self) -> DamageTypeId {
        self.source.type_id()
    }
    fn is_server_player(&self) -> bool {
        true
    }
    fn invulnerable(&mut self, _: &mut State) -> io::Result<bool> {
        damage_immunity::server_player_immunity(self)
    }
    fn dead_or_dying(&mut self, _: &mut State) -> io::Result<bool> {
        Err(io::Error::other("combat binding supplies health predicate"))
    }
    fn tag(&mut self, _: &mut State, tag: HurtTag) -> io::Result<bool> {
        Ok(self.source.hurt_tag(tag))
    }
    fn fire_resistance(&mut self, _: &mut State) -> io::Result<bool> {
        Ok(self.status.fire_resistance)
    }
    fn hurt_start(&mut self, _: &mut State, damage: &mut f32) -> io::Result<bool> {
        let source = HurtSource {
            damage_type: &self.source.definition().name,
            causing: self.causing,
            direct: 0,
        };
        let canceled = hurt_server_canceled(Hurting::Player(self.entity, source), damage);
        self.canceled |= canceled;
        Ok(canceled)
    }
    fn sleeping(&mut self, _: &mut State) -> io::Result<bool> {
        Ok(false)
    }
    fn stop_sleeping(&mut self, _: &mut State) -> io::Result<()> {
        Ok(())
    }
    fn use_item(&mut self, _: &mut State) -> io::Result<()> {
        Ok(())
    }
    fn apply_blocking(&mut self, _: &mut State, damage: f32) -> io::Result<f32> {
        let Some(b) = &self.blocking else {
            return Ok(0.0);
        };
        if damage <= 0.0 || b.bypassed || b.pierce {
            return Ok(0.0);
        }
        self.blocked = b.rules.resolve(&mut MatchedSets, damage, b.angle)?;
        self.fully_blocked = self.blocked >= damage;
        Ok(self.blocked)
    }
    fn freeze_hurts_extra(&mut self, _: &mut State) -> io::Result<bool> {
        Ok(false)
    }
    fn helmet_empty(&mut self, _: &mut State) -> io::Result<bool> {
        Ok(true)
    }
    fn hurt_helmet(&mut self, _: &mut State, _: f32) -> io::Result<()> {
        Ok(())
    }
    fn actually_hurt(&mut self, _: &mut State, _: f32) -> io::Result<()> {
        Err(io::Error::other("combat binding supplies inner damage"))
    }
    fn resolve_mob(&mut self, _: &mut State) -> io::Result<()> {
        Ok(())
    }
    fn resolve_player(&mut self, _: &mut State) -> io::Result<()> {
        Ok(())
    }
    fn blocking_component(&mut self, _: ()) -> Option<()> {
        self.blocking.as_ref().map(|_| ())
    }
    fn on_blocked(&mut self, _: &mut State, _: ()) -> io::Result<()> {
        self.block_sound = true;
        Ok(())
    }
    fn broadcast_damage(&mut self, _: &mut State) -> io::Result<()> {
        self.event(HealthEvent::DamageBroadcast {
            source: self.source.type_id(),
        });
        Ok(())
    }
    fn mark_hurt(&mut self, _: &mut State) -> io::Result<()> {
        *self.sync_velocity = true;
        Ok(())
    }
    /// The caller applies it: these sources carry no entity or position.
    fn knockback(&mut self, _: &mut State, _: f32, _: bool) -> io::Result<()> {
        self.knocked = true;
        Ok(())
    }
    fn death_protection(&mut self, _: &mut State) -> io::Result<bool> {
        if !self.totem
            || self
                .source
                .immunity_tag(ImmunityTag::BypassesInvulnerability)
        {
            return Ok(false);
        }
        self.totem_fired = true;
        Ok(true)
    }
    fn death_sound(&mut self, _: &mut State) -> io::Result<()> {
        Ok(())
    }
    fn make_sound(&mut self, _: &mut State, _: ()) -> io::Result<()> {
        self.phase("death-sound")
    }
    fn secondary_sound(&mut self, _: &mut State) -> io::Result<()> {
        if self.source.definition().effects == "thorns" {
            self.phase("thorns-sound")?;
        }
        Ok(())
    }
    fn die(&mut self, _: &mut State) -> io::Result<()> {
        *self.dead = true;
        self.phase("death-request")
    }
    /// `Player.playSound` leaves the player itself out: others hear this one.
    fn hurt_sound(&mut self, _: &mut State) -> io::Result<()> {
        if !self.block_sound {
            let sound = hurt_sound(&self.source.definition().effects);
            self.events.push(MotionEvent::HurtSound(sound));
        }
        self.phase("hurt-sound")
    }
    fn game_time(&mut self, _: &mut State) -> io::Result<i64> {
        Ok(self.game_time)
    }
    fn effects_on_hurt(&mut self, _: &mut State, _: f32) -> io::Result<()> {
        Ok(())
    }
    fn player_hurt_criterion(
        &mut self,
        _: &mut State,
        original: f32,
        amount: f32,
        blocked: bool,
    ) -> io::Result<()> {
        self.event(HealthEvent::Criterion {
            original,
            amount,
            blocked,
        });
        Ok(())
    }
    fn award_blocked(&mut self, _: &mut State, amount: i32) -> io::Result<()> {
        self.stat("blocked", amount)
    }
    fn attacker_criterion(&mut self, _: &mut State, _: f32, _: f32, _: bool) -> io::Result<()> {
        Ok(())
    }
}
impl PlayerHurtWorld for Environment<'_> {
    fn ability_invulnerable(&self) -> bool {
        self.invulnerable
    }
    fn bypasses_invulnerability(&mut self, _: &mut State) -> io::Result<bool> {
        Ok(self.source.in_tag("minecraft:bypasses_invulnerability"))
    }
    fn remove_shoulders(&mut self, _: &mut State) -> io::Result<()> {
        Ok(())
    }
    fn scales_with_difficulty(&mut self, _: &mut State) -> io::Result<bool> {
        // DamageSource.scalesWithDifficulty.
        Ok(match self.source.definition().scaling {
            Scaling::Never => false,
            Scaling::Always => true,
            Scaling::LivingNonPlayer => self.living_cause,
        })
    }
    fn difficulty(&mut self, _: &mut State) -> io::Result<Difficulty> {
        Ok(self.rules.difficulty)
    }
}
impl ServerPlayerHurtWorld for Environment<'_> {
    type Player = ();
    type Arrow = ();
    fn causing_entity(&mut self, _: &mut State) -> io::Result<CausingEntity<(), ()>> {
        Ok(CausingEntity::Other)
    }
    fn arrow_player_owner(&mut self, _: &mut State, _: ()) -> io::Result<Option<()>> {
        Ok(None)
    }
    fn can_harm_player(&mut self, _: &mut State, _: ()) -> io::Result<bool> {
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_hit_in_the_hurt_cooldown_knocks_nobody_back() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let mut player =
            crate::motion::PlayerMotion::new(&resources, horizon_core::geometry::Vec3::ZERO)
                .unwrap();
        let (health, rules) = (&resources.health, super::HealthRules::default());
        let id = health.damage.id("minecraft:mob_attack").unwrap();
        let mut hit = |amount| {
            player
                .hurt_blocking(health, rules, id, amount, None)
                .unwrap()
        };
        let first = hit(1.0);
        assert!(first.hurt && first.knockback);
        // A stronger hit inside the cooldown hurts by the difference only
        // (LivingEntity.hurtServer), with no dealDefaultKnockback.
        let second = hit(2.0);
        assert!(second.hurt && !second.knockback);
        // A weaker one doesn't hurt at all.
        let third = hit(1.5);
        assert!(!third.hurt && !third.knockback);
    }

    /// #2800: on Easy a mob's 4 is scaled to 3 before `applyItemBlocking`,
    /// so a shield that stops all 3 blocks fully (`damageBlocked >= damage`)
    /// and `blockedByItem` must not pull the player toward the mob.
    #[test]
    fn a_shield_fully_blocks_a_hit_scaled_down_by_easy() {
        use horizon_world::blocking_rules::{BlockingRules, DamageReduction, ItemDamageFunction};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let mut player =
            crate::motion::PlayerMotion::new(&resources, horizon_core::geometry::Vec3::ZERO)
                .unwrap();
        let health = &resources.health;
        let rules = super::HealthRules {
            difficulty: horizon_world::food::Difficulty::Easy,
            ..Default::default()
        };
        let shield = super::Blocking {
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
        let id = health.damage.id("minecraft:mob_attack").unwrap();
        let o = player
            .hurt_caused_by(health, rules, id, 4.0, Some(shield), true, 0)
            .unwrap();
        assert_eq!(o.blocked, 3.0, "Easy scales 4 to 3 before the shield");
        assert!(o.fully_blocked && !o.knockback);
    }

    /// #2141: an arrow that takes the last of the player's health must kill
    /// them: `LivingEntity.hurt` leaves the health at zero and `Player.die`
    /// follows.
    #[test]
    fn an_arrow_kills_a_player_at_one_health() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let mut player =
            crate::motion::PlayerMotion::new(&resources, horizon_core::geometry::Vec3::ZERO)
                .unwrap();
        player.set_health(1.0, false).unwrap();
        let (health, rules) = (&resources.health, super::HealthRules::default());
        let id = health.damage.id("minecraft:arrow").unwrap();
        let hurt = player.hurt(health, rules, id, 5.0).unwrap();
        assert!(hurt, "the arrow must hurt");
        assert_eq!(player.health.health(), 0.0, "the health must be spent");
        assert!(player.health.dead_or_dying(), "the player must be dead");
    }

    /// What the hook of these tests saw: entity, damage type (a mob's kind)
    /// and causing entity, amount.
    static SEEN: std::sync::Mutex<Vec<(i32, String, f32)>> = std::sync::Mutex::new(Vec::new());
    /// Sets the native damage hook of these tests (global: it stays set for
    /// the process): for entities 9001 to 9004 and 9011 to 9013 it records the
    /// hit, cancels x1's, doubles x2's, makes 9003's -5 and 9004's NaN.
    fn set_hook() -> fn(i32) -> Vec<(String, f32)> {
        use super::Hurting;
        fn hook(hurting: Hurting<'_>, amount: &mut f32) -> bool {
            let (entity, seen) = match hurting {
                Hurting::Player(e, s) | Hurting::Dragon(e, s) => {
                    (e, format!("{} {}", s.damage_type, s.causing))
                }
                Hurting::Mob(mob, source) => (mob.id as i32, format!("{:?}", source.kind)),
            };
            if !matches!(entity, 9001..=9004 | 9011..=9013) {
                return false;
            }
            SEEN.lock().unwrap().push((entity, seen, *amount));
            match entity {
                9002 | 9012 => *amount *= 2.0,
                9003 => *amount = -5.0,
                9004 => *amount = f32::NAN,
                _ => {}
            }
            entity % 10 == 1
        }
        let installed = *super::HURT_SERVER.get_or_init(|| hook);
        assert!(std::ptr::fn_addr_eq(
            installed,
            hook as fn(Hurting<'_>, &mut f32) -> bool
        ));
        |entity| {
            let seen = SEEN.lock().unwrap();
            let hits = seen.iter().filter(|s| s.0 == entity);
            hits.map(|s| (s.1.clone(), s.2)).collect()
        }
    }

    #[test]
    fn the_native_damage_hook_changes_or_cancels_a_mobs_hit() {
        use crate::mobs::{Damage, DamageKind, Mob};
        let seen = set_hook();
        let mob = |id| {
            Mob::new(
                id,
                crate::mobs::sensor_scan_tests::kind(),
                Default::default(),
                0.0,
            )
        };
        let hit = Damage {
            kind: DamageKind::PlayerAttack,
            attacker: None,
            from: Some([3.0, 0.0, 0.0]),
        };
        // Canceled: no damage, cooldown, sound or knockback.
        let mut kept = mob(9001);
        assert!(!kept.damage(2.0, hit));
        assert_eq!(
            (kept.health, kept.invulnerable, kept.hurt_time),
            (10.0, 0, 0)
        );
        assert!(kept.outbox.is_empty() && kept.velocity == Default::default());
        assert_eq!(seen(9001), [("PlayerAttack".to_owned(), 2.0)]);
        let mut doubled = mob(9002);
        assert!(doubled.damage(2.0, hit));
        assert_eq!(doubled.health, 6.0);
        // LivingEntity.hurtServer: a negative amount hurts by 0, a NaN by
        // Float.MAX_VALUE.
        let (mut negative, mut nan) = (mob(9003), mob(9004));
        assert!(negative.damage(2.0, hit) && nan.damage(2.0, hit));
        assert!(negative.health == 10.0 && nan.health <= 0.0);
    }

    /// Needs the prepared data in `.local/horizon-client/26.3`; skipped
    /// without it.
    #[test]
    fn the_native_damage_hook_changes_or_cancels_a_players_hit() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let seen = set_hook();
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let (health, rules) = (&resources.health, super::HealthRules::default());
        let id = health.damage.id("minecraft:arrow").unwrap();
        let player = |entity| {
            let mut p = crate::motion::PlayerMotion::new(&resources, Default::default()).unwrap();
            p.entity_id = entity;
            p
        };
        let mut kept = player(9011);
        assert!(!kept.hurt(health, rules, id, 5.0).unwrap());
        assert_eq!((kept.health.health(), kept.health.hurt.cooldown), (20.0, 0));
        let mut doubled = player(9012);
        assert!(doubled.hurt(health, rules, id, 5.0).unwrap());
        assert_eq!(doubled.health.health(), 10.0);
        // The causing mob when it is known, else 0.
        let hurt = player(9013).hurt_caused_by(health, rules, id, 5.0, None, false, 44);
        assert!(hurt.unwrap().hurt);
        assert_eq!(seen(9011), [("minecraft:arrow 0".to_owned(), 5.0)]);
        assert_eq!(seen(9013), [("minecraft:arrow 44".to_owned(), 5.0)]);
    }

    /// What the knockback hook of these tests saw: the entity, the hit that
    /// causes it (a mob's damage kind) and the push it was handed.
    static PUSHES: std::sync::Mutex<Vec<(i32, String, super::Knock)>> =
        std::sync::Mutex::new(Vec::new());
    /// Sets the native knockback hook of these tests (global: it stays set for
    /// the process), for mobs 9101 to 9106 only: it records the push, cancels
    /// 9101's, doubles 9102's strength and turns its direction around, leaves
    /// 9103 non-finite numbers, and gives 9105 (whose resistance leaves
    /// nothing) a strength of 0.4.
    fn set_knockback_hook() -> fn(i32) -> Vec<(String, super::Knock)> {
        use super::{Hurting, Knock};
        fn hook(pushed: Hurting<'_>, knock: &mut Knock) -> bool {
            let Hurting::Mob(mob, source) = pushed else {
                return false;
            };
            let entity = mob.id as i32;
            if !(9101..=9106).contains(&entity) {
                return false;
            }
            let hit = format!("{:?}", source.kind);
            PUSHES.lock().unwrap().push((entity, hit, *knock));
            match entity {
                9101 => return true,
                9102 => {
                    knock.strength *= 2.0;
                    knock.x = -knock.x;
                }
                9103 => {
                    knock.strength = f64::NAN;
                    knock.x = f64::INFINITY;
                    knock.z = f64::NEG_INFINITY;
                }
                9105 => knock.strength = 0.4,
                _ => {}
            }
            false
        }
        let installed = *super::KNOCKBACK.get_or_init(|| hook);
        assert!(std::ptr::fn_addr_eq(
            installed,
            hook as fn(Hurting<'_>, &mut Knock) -> bool
        ));
        |entity| {
            let seen = PUSHES.lock().unwrap();
            let pushes = seen.iter().filter(|p| p.0 == entity);
            pushes.map(|p| (p.1.clone(), p.2)).collect()
        }
    }

    /// `LivingEntity.knockback`: the hook runs once the knockback resistance
    /// took its share and before the motion changes; a mod's strength or
    /// direction is what pushes, a cancel pushes nothing, and with no mod in
    /// the way the push is vanilla's. Expected values are worked out by hand
    /// from vanilla's formula: the horizontal speed halves and the push is
    /// subtracted along the normalized direction; on the ground the vertical
    /// speed becomes `min(0.4, vy / 2 + strength)`.
    #[test]
    fn the_native_knockback_hook_changes_or_cancels_a_mobs_push() {
        use super::Knock;
        use crate::mobs::{Damage, Mob};
        let seen = set_knockback_hook();
        let mob = |id, resistance| {
            let mut m = Mob::new(
                id,
                crate::mobs::sensor_scan_tests::kind(),
                Default::default(),
                0.0,
            );
            m.on_ground = true;
            m.attrs.knockback_resistance = resistance;
            m
        };
        let velocity = |m: &Mob| (m.velocity.x, m.velocity.y, m.velocity.z);
        let hit = Damage::player([3.0, 0.0, 0.0]);
        let push = |id, resistance| {
            let mut m = mob(id, resistance);
            m.knockback(0.4, 3.0, 0.0, &hit);
            m
        };
        // No mod in the way (the hook leaves 9100 alone): vanilla's push, away from x = 3.
        let plain = push(9100, 0.0);
        assert_eq!(velocity(&plain), (-0.4, 0.4, 0.0));
        assert!(plain.needs_sync);
        // Canceled: no motion, no resync. The hook saw the push the game made.
        let kept = push(9101, 0.0);
        assert_eq!(velocity(&kept), (0.0, 0.0, 0.0));
        assert!(!kept.needs_sync);
        let made = Knock {
            strength: 0.4,
            x: 3.0,
            z: 0.0,
        };
        assert_eq!(seen(9101), [("PlayerAttack".to_owned(), made)]);
        // A stronger push the other way: twice the strength, x turned around.
        let turned = push(9102, 0.0);
        assert_eq!(velocity(&turned), (0.8, 0.4, 0.0));
        // Numbers that are not finite are put back as the game made them.
        assert_eq!(velocity(&push(9103, 0.0)), (-0.4, 0.4, 0.0));
        // Before the resistance (`LivingKnockBackEvent`): the hook is handed the raw 0.4, and
        // the push that follows is halved.
        let resisted = push(9104, 0.5);
        assert_eq!(velocity(&resisted), (-0.2, 0.2, 0.0));
        assert_eq!(seen(9104)[0].1.strength, 0.4);
        // A full resistance takes all of what the hook leaves: no push.
        let stopped = push(9105, 1.0);
        assert_eq!(seen(9105)[0].1.strength, 0.4);
        assert_eq!(velocity(&stopped), (0.0, 0.0, 0.0));
        // The game's own hit on a mob reaches the hook with its damage.
        let mut hurt = mob(9106, 0.0);
        assert!(hurt.damage(2.0, hit));
        // dealDefaultKnockback pushes with the float 0.4F.
        let strength = f64::from(0.4_f32);
        assert_eq!(velocity(&hurt), (-strength, 0.4, 0.0));
        let made = Knock { strength, ..made };
        assert_eq!(seen(9106), [("PlayerAttack".to_owned(), made)]);
    }

    /// #4301: `ServerPlayer.isInvulnerableTo`'s `!connection.hasClientLoaded`
    /// term spares a respawned player the void until the client has loaded the
    /// world; once it has, the void hurts again.
    #[test]
    fn the_void_spares_a_player_whose_client_has_not_loaded() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let resources = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let mut player =
            crate::motion::PlayerMotion::new(&resources, horizon_core::geometry::Vec3::ZERO)
                .unwrap();
        let (health, rules) = (&resources.health, super::HealthRules::default());
        let id = health.damage.id("minecraft:out_of_world").unwrap();
        player.client_loaded = false;
        assert!(
            !player.hurt(health, rules, id, 4.0).unwrap(),
            "an unloaded client must not take void damage"
        );
        assert_eq!(player.health.health(), 20.0);
        player.client_loaded = true;
        assert!(
            player.hurt(health, rules, id, 4.0).unwrap(),
            "a loaded client takes the void's 4.0"
        );
        assert_eq!(player.health.health(), 16.0);
    }

    #[test]
    fn hurt_sounds_follow_damage_effects() {
        // DamageEffects: HURT, THORNS, DROWNING, BURNING, POKING, FREEZING.
        for (effects, sound) in [
            ("hurt", "minecraft:entity.player.hurt"),
            ("thorns", "minecraft:entity.player.hurt"),
            ("drowning", "minecraft:entity.player.hurt_drown"),
            ("burning", "minecraft:entity.player.hurt_on_fire"),
            ("poking", "minecraft:entity.player.hurt_sweet_berry_bush"),
            ("freezing", "minecraft:entity.player.hurt_freeze"),
        ] {
            assert_eq!(super::hurt_sound(effects), sound, "{effects}");
        }
    }
}
