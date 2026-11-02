//! The player's `CombatTracker`: the damage taken lately, who dealt it and
//! where the player fell from, turned into the vanilla death message.
use super::{Game, chat::PLAYER_NAME, commands::Target};
use horizon_client::{
    health::HealthEvent,
    mobs::{Mob, Species},
    motion::MotionEvent,
    particles::{ParticleOptions, ParticleType},
    text::Text,
};
use horizon_core::random::RandomSource;

/// `EntityEvent.DROWN_PARTICLES`: broadcast with the drowning damage.
pub(super) const DROWN_EVENT: i8 = 67;

struct Entry {
    damage_type: String,
    damage: f32,
    fall_location: Option<&'static str>,
    fall_distance: f32,
    /// The causing entity: its id (for comparisons) and display name.
    attacker: Option<(u32, Text)>,
    /// The causing player's main-hand stack when it has a custom name, as
    /// held when the hit landed (a mob's is read when the message is built).
    item: Option<Text>,
    /// The direct entity's display name when no entity caused the damage
    /// (`DamageSource.directEntity`: a potion with no thrower, a cloud).
    direct: Option<Text>,
}
/// What is stuck in a player's body: `LivingEntity.DATA_ARROW_COUNT_ID` and
/// `DATA_STINGER_COUNT_ID` with the timers that pull them out
/// (`removeArrowTime`, `removeStingerTime`). Kept with the combat record,
/// which is swapped with the body it belongs to.
#[derive(Default)]
pub(super) struct Stuck {
    pub(super) arrows: i32,
    pub(super) stingers: i32,
    remove_arrow_time: i32,
    remove_stinger_time: i32,
}
impl Stuck {
    /// `AbstractArrow.onHitEntity`: a hit that hurt, from an arrow that
    /// does not pierce, leaves it stuck.
    pub(super) fn add_arrow(&mut self) {
        self.arrows += 1;
    }
    /// `Bee.doHurtTarget`: a sting that hurt.
    pub(super) fn add_stinger(&mut self) {
        self.stingers += 1;
    }
    /// The server's part of `LivingEntity.tick`: one in a count's worth of
    /// `20 * (30 - count)` ticks falls out. Whether the counts changed.
    pub(super) fn tick(&mut self) -> bool {
        let (arrows, stingers) = (self.arrows, self.stingers);
        Self::decay(&mut self.arrows, &mut self.remove_arrow_time);
        Self::decay(&mut self.stingers, &mut self.remove_stinger_time);
        (arrows, stingers) != (self.arrows, self.stingers)
    }
    fn decay(count: &mut i32, remove_time: &mut i32) {
        if *count > 0 {
            if *remove_time <= 0 {
                *remove_time = 20 * (30 - *count);
            }
            *remove_time -= 1;
            if *remove_time <= 0 {
                *count -= 1;
            }
        }
    }
}
#[derive(Default)]
pub(super) struct CombatTracker {
    /// The arrows and bee stingers stuck in the body.
    pub(super) stuck: Stuck,
    entries: Vec<Entry>,
    last_damage: i64,
    in_combat: bool,
    /// Attackers of damage dealt this tick, claimed by the matching
    /// `RecordedDamage` event.
    pending: Vec<(String, (u32, Text), Option<Text>)>,
    /// `pending` for a direct entity with no causing entity: only its name.
    pending_direct: Vec<(String, Text)>,
    /// The lengths of `pending` and `pending_direct` before the first
    /// attacker noted for the hurt being dealt now (`forget_noted`).
    noted_since: Option<(usize, usize)>,
    /// `LivingEntity.lastHurtByMob` and its timestamp (the kill credit).
    last_hurt_by: Option<((u32, Text), i64)>,
    /// The mob the player last hurt (`getLastHurtMob`) and when.
    pub(super) last_hurt_mob: Option<(u32, i64)>,
    /// The mob whose projectile is hitting the player now.
    pub(super) projectile_owner: Option<u32>,
    /// The display name of the projectile hitting the player now: the
    /// `DamageSource.directEntity` the death message names when no owner is
    /// known (`AbstractArrow.onHitEntity` makes an ownerless arrow its own
    /// cause).
    pub(super) projectile_direct: Option<Text>,
    /// The player whose primed TNT is going off now (`PrimedTnt.getOwner`):
    /// its entity id and display name, the blast's causing entity.
    pub(super) blast_owner: Option<(u32, Text)>,
    /// The guest behind `blast_owner`, when that player is a LAN guest; the
    /// host's own blast leaves it None.
    pub(super) blast_guest: Option<u32>,
    pub(super) projectile_attacker: Option<(u32, Text)>,
    /// The projectile hitting the player now is another player's that
    /// `ServerPlayer.canHarmPlayer` refuses (the `pvp` rule, an ally).
    pub(super) refuse_player_hit: bool,
    /// The hit on the player now runs its post-attack effects (thorns) only
    /// if the player lives through it (`WitherSkull.onHitEntity`).
    pub(super) post_attack_if_alive: bool,
    /// A firework rocket's burst is hurting the player now: the rocket is
    /// the direct entity (`rocket_hurt_player`).
    pub(super) rocket_hit: bool,
    /// The player whose melee hit is landing on the player now: where they
    /// stand and the seconds their weapon disables a shield for
    /// (`getSecondsToDisableBlocking`).
    pub(super) player_attacker: Option<([f64; 3], f32)>,
    /// While set, a sweep attack's reach to the other players is kept here
    /// instead of landing, for the caller that holds the LAN host.
    pub(super) sweeps: Option<Vec<super::lan_pvp::Sweep>>,
    /// The mob exploding now (`ServerExplosion.source`): its team's friendly
    /// fire decides whom the blast may hurt.
    pub(super) explosion_source: Option<u32>,
    /// The exploding mob as predicates see it (`ServerExplosion.source`,
    /// `ServerPlayer.currentExplosionCause`), for a mob already out of the list.
    pub(super) explosion_cause: Option<horizon_client::advancements::EntityView>,
    /// The exploding mob named itself (`MobEvent::ExplosionSource`), so the
    /// blast leaves it out (`getEntities(source, box)`). A source guessed
    /// from the mobs near the centre is still hurt: it may be a bystander.
    pub(super) explosion_source_named: bool,
    /// The blast's direct source is in water (`Explosion.getDirectSourceEntity`
    /// `.isInWater`): paintings, item frames and cushions ignore it
    /// (`BlockAttachedEntity.ignoreExplosion`).
    pub(super) explosion_in_water: bool,
    /// The blast now going off comes from a primed TNT, creeper, wither
    /// skull, wither or TNT minecart, the sources that empty a hive they
    /// destroy (`BeehiveBlock.getDrops`).
    pub(super) hive_emptying_blast: bool,
    /// The blast now going off comes from a primed TNT minecart, which
    /// neither resists on nor breaks a rail or the block under one
    /// (`MinecartTNT.getBlockExplosionResistance`, `shouldBlockExplode`).
    pub(super) rail_proof_blast: bool,
    /// The mob that owns the projectile exploding now (`getIndirectSourceEntity`):
    /// the mobs the blast hurts record it as their attacker.
    pub(super) explosion_owner: Option<u32>,
    /// `WitherSkull.getBlockExplosionResistance`: a dangerous skull's blast
    /// caps every destroyable block's resistance at this value.
    pub(super) blast_resistance_cap: Option<f32>,
    /// Charged creepers whose blast just went off and that have not dropped
    /// a head yet (`Creeper.droppedSkulls`): they are gone from the mob list,
    /// but the mobs they killed still ask for their loot.
    pub(super) charged_exploders: Vec<u32>,
    /// The mobs whose blast went off this tick and that left the mob list
    /// (`MobEvent::ExplosionSource`): their names, for the death messages of
    /// the mobs the blast killed.
    pub(super) exploded: Vec<(u32, Text)>,
    /// The player's fall distance before this tick's movement (as of the
    /// end of the last one).
    fall_before: f64,
}
impl CombatTracker {
    fn mark(&mut self) {
        let marks = (self.pending.len(), self.pending_direct.len());
        self.noted_since.get_or_insert(marks);
    }
    /// The player's hurt is over: when a native mod canceled it, the
    /// attackers noted for it go, as no combat entry takes them and the next
    /// hit of the same damage type must not be credited to them.
    pub(super) fn forget_noted(&mut self, canceled: bool) {
        if let Some((attackers, directs)) = self.noted_since.take()
            && canceled
        {
            self.pending.truncate(attackers);
            self.pending_direct.truncate(directs);
        }
    }
}
impl Game<'_> {
    /// `LivingEntity.makeDrownParticles`: eight bubbles around the entity,
    /// moving with it.
    pub(super) fn drown_particles(&mut self, at: [f64; 3], velocity: [f64; 3]) {
        let bubble = ParticleOptions::simple(ParticleType::Bubble);
        for _ in 0..8 {
            let mut triangle = || {
                let r = self.particles.random();
                r.next_double() - r.next_double()
            };
            let p = [at[0] + triangle(), at[1] + triangle(), at[2] + triangle()];
            self.particles.add(&bubble, p, velocity);
        }
    }
    /// Entity event 67 from a server, for this client's own player or an
    /// entity it draws.
    pub(super) fn remote_drown(&mut self, id: i32) {
        let own = self.remote.as_ref().is_some_and(|r| r.entity_id == id);
        let body = if own {
            self.player.as_ref().map(|p| {
                let (at, v) = (p.position, p.velocity);
                ([at.x, at.y, at.z], [v.x, v.y, v.z])
            })
        } else {
            self.mobs.iter().find(|m| m.id == id as u32).map(|m| {
                let (at, v) = (m.position, m.velocity);
                ([at.x, at.y, at.z], [v.x, v.y, v.z])
            })
        };
        if let Some((at, v)) = body {
            self.drown_particles(at, v);
        }
    }
    /// The display name of a mob: its custom name or its type's name.
    pub(super) fn mob_display_name(&self, id: u32) -> Option<Text> {
        let m = self.mobs.iter().find(|m| m.id == id)?;
        Some(match &m.custom_name {
            Some(name) => Text::literal(name.clone()),
            None => Text::translate(&format!("entity.{}", m.kind.id.replace(':', ".")), &[]),
        })
    }
    /// `getLastHurtByMob`: the last mob to hurt the player, for 100 ticks.
    pub(super) fn player_hurt_by(&self) -> Option<(horizon_client::mobs::Who, i64)> {
        let ((id, _), at) = self.combat.last_hurt_by.as_ref()?;
        (self.world.game_time - at <= 100).then_some((horizon_client::mobs::Who::Mob(*id), *at))
    }
    /// Remember who deals the next `damage_type` damage to the player.
    pub(super) fn note_attacker(&mut self, damage_type: &str, mob: u32) {
        if let Some(name) = self.mob_display_name(mob) {
            self.note_named_attacker(damage_type, mob, name);
        }
    }
    /// `note_attacker` for an entity that is not one of `self.mobs` (the
    /// ender dragon).
    pub(super) fn note_named_attacker(&mut self, damage_type: &str, id: u32, name: Text) {
        self.note_player_attacker(damage_type, id, name, None);
    }
    /// `note_named_attacker` for a player, with their custom-named main-hand
    /// item for the death message's `.item` form.
    pub(super) fn note_player_attacker(
        &mut self,
        damage_type: &str,
        id: u32,
        name: Text,
        item: Option<Text>,
    ) {
        self.combat.mark();
        self.combat
            .pending
            .push((damage_type.to_owned(), (id, name), item));
    }
    /// The `DamageSource.directEntity` of the next `damage_type` damage when
    /// nothing caused it (a splash potion nobody threw): the death message
    /// names it instead (`getLocalizedDeathMessage`). It is no kill credit.
    pub(super) fn note_direct_entity(&mut self, damage_type: &str, name: Text) {
        self.combat.mark();
        self.combat
            .pending_direct
            .push((damage_type.to_owned(), name));
    }
    /// `LivingEntity.handleDamageEvent` for the player: the hurt sound of the
    /// source's `DamageEffects` (`Player.getHurtSound`).
    pub(super) fn player_hurt_sound(&mut self, sound: &str, at: [f64; 3]) {
        // Player.getVoicePitch: always 1.0.
        self.audio.play(sound, Some(at), 1.0, 1.0);
    }
    /// `CombatTracker.recordDamage` for the motion events of this tick.
    pub(super) fn record_combat(&mut self, events: &[MotionEvent]) {
        self.recheck_combat();
        for e in events {
            let (name, amount) = match e {
                // LivingEntity.getDamageAfterMagicAbsorb (Resistance) and
                // Player.actuallyHurt (absorption hearts) and
                // LivingEntity.hurtServer (the shield, only for an accepted
                // hit), in tenths.
                MotionEvent::Health(HealthEvent::Statistic { name, amount }) => {
                    match *name {
                        "resisted" => self.stats.custom("damage_resisted", *amount),
                        "absorbed" => self.stats.custom("damage_absorbed", *amount),
                        "blocked" => self.stats.custom("damage_blocked_by_shield", *amount),
                        _ => {}
                    }
                    continue;
                }
                MotionEvent::Health(HealthEvent::RecordedDamage { source, amount }) => {
                    let Some(d) = self.motion.health.damage.definition(*source) else {
                        continue;
                    };
                    (d.name.clone(), *amount)
                }
                _ => continue,
            };
            let (attacker, item) = self
                .combat
                .pending
                .iter()
                .position(|(t, _, _)| *t == name)
                .map(|i| {
                    let (_, attacker, item) = self.combat.pending.remove(i);
                    (Some(attacker), item)
                })
                .unwrap_or_default();
            let direct = match attacker {
                Some(_) => None,
                None => self
                    .combat
                    .pending_direct
                    .iter()
                    .position(|(t, _)| *t == name)
                    .map(|i| self.combat.pending_direct.remove(i).1),
            };
            let (fall_location, fall_distance) = self.fall_location();
            let now = self.world.game_time;
            if let Some(a) = &attacker {
                self.combat.last_hurt_by = Some((a.clone(), now));
                if !self.combat.in_combat {
                    self.combat.in_combat = true;
                }
            }
            // Player.actuallyHurt: damage_taken in tenths.
            self.stats
                .custom("damage_taken", (amount * 10.0).round() as i32);
            self.combat.entries.push(Entry {
                damage_type: name,
                damage: amount,
                fall_location,
                fall_distance,
                attacker,
                item,
                direct,
            });
            self.combat.last_damage = now;
            // InfestedMobEffect.onMobHurt.
            if self.potions.effects.get("infested").is_some() {
                self.player_infested();
            }
        }
        self.combat.pending.clear();
        self.combat.pending_direct.clear();
        self.combat.noted_since = None;
        self.combat.projectile_direct = None;
        self.combat.projectile_attacker = None;
        self.combat.fall_before = self.player.as_ref().map_or(0.0, |p| p.fall_distance);
    }
    /// `FallLocation.getCurrentFallLocation` and the fall distance so far.
    fn fall_location(&self) -> (Option<&'static str>, f32) {
        let Some(p) = self.player.as_ref() else {
            return (None, 0.0);
        };
        let distance = p.fall_distance.max(self.combat.fall_before) as f32;
        let location = if let Some(c) = p.last_climbable {
            let name = self
                .res
                .facts()
                .get(self.world.block(c.x, c.y, c.z))
                .name
                .clone();
            Some(match &*name {
                "minecraft:ladder" => "ladder",
                "minecraft:vine" => "vines",
                "minecraft:weeping_vines" | "minecraft:weeping_vines_plant" => "weeping_vines",
                "minecraft:twisting_vines" | "minecraft:twisting_vines_plant" => "twisting_vines",
                "minecraft:scaffolding" => "scaffolding",
                _ => "other_climbable",
            })
        } else {
            let f = self.res.facts().get(self.world.block(
                p.position.x.floor() as i32,
                (p.position.y + 0.1).floor() as i32,
                p.position.z.floor() as i32,
            ));
            (f.fluid == horizon_client::blocks::Fluid::Water || f.waterlogged).then_some("water")
        };
        (location, distance)
    }
    /// `CombatTracker.recheckStatus`: forget old fights.
    fn recheck_combat(&mut self) {
        let reset = if self.combat.in_combat { 300 } else { 100 };
        if !self.combat.entries.is_empty() && self.world.game_time - self.combat.last_damage > reset
        {
            self.combat.entries.clear();
            self.combat.in_combat = false;
        }
    }
    fn damage_in_tag(&self, name: &str, tag: &str) -> bool {
        let damage = &self.motion.health.damage;
        damage.id(name).is_some_and(|id| {
            damage.in_tag(id, tag) || damage.in_tag(id, &format!("minecraft:{tag}"))
        })
    }
    /// The killing blow as advancement predicates see it (`DamageSource` of the
    /// last combat entry): its type and the causing entity. The projectile
    /// damage types have the projectile as direct entity, not the shooter.
    pub(super) fn combat_killing_blow(&self) -> Option<horizon_client::advancements::DamageView> {
        use horizon_client::advancements::{DamageView, EntityView};
        let e = self.combat.entries.last()?;
        let source = e.attacker.as_ref().and_then(|(id, _)| {
            self.mobs
                .iter()
                .find(|m| m.id == *id)
                .map(|m| self.mob_entity(m))
        });
        let projectile = match e.damage_type.as_str() {
            // The rocket is the direct entity of its `fireworks` damage.
            "minecraft:fireworks" => Some("minecraft:firework_rocket"),
            t @ ("minecraft:arrow"
            | "minecraft:trident"
            | "minecraft:mob_projectile"
            | "minecraft:fireball"
            | "minecraft:unattributed_fireball"
            | "minecraft:wither_skull"
            | "minecraft:wind_charge"
            | "minecraft:thrown"
            | "minecraft:spit") => Some(t),
            _ => None,
        };
        let direct = match projectile {
            Some(kind) => Some(EntityView {
                kind: kind.to_owned(),
                ..Default::default()
            }),
            None => source.clone(),
        };
        Some(DamageView {
            kind: e.damage_type.clone(),
            direct,
            source,
            is_direct: projectile.is_none(),
            original: e.damage,
            actual: e.damage,
            blocked: false,
        })
    }
    /// The type of the mob that dealt the killing blow (`killed_by`).
    pub(super) fn combat_killer_kind(&self) -> Option<String> {
        let (id, _) = self.combat.entries.last()?.attacker.clone()?;
        self.mobs
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.kind.id.clone())
    }
    /// `CombatTracker.getMostSignificantFall`.
    fn most_significant_fall(&self) -> Option<usize> {
        let (mut result, mut alternative) = (None, None);
        let (mut best_fall, mut alt_damage) = (0.0f32, 0.0f32);
        for (i, e) in self.combat.entries.iter().enumerate() {
            let fake = self.damage_in_tag(&e.damage_type, "always_most_significant_fall");
            let distance = if fake { f32::MAX } else { e.fall_distance };
            if (self.damage_in_tag(&e.damage_type, "is_fall") || fake)
                && distance > 0.0
                && (result.is_none() || distance > best_fall)
            {
                result = Some(i.saturating_sub(1));
                best_fall = distance;
            }
            if e.fall_location.is_some() && (alternative.is_none() || e.damage > alt_damage) {
                alternative = Some(i);
                alt_damage = e.damage;
            }
        }
        if best_fall > 5.0 && result.is_some() {
            result
        } else if alt_damage > 5.0 {
            alternative
        } else {
            None
        }
    }
    /// The display component of a custom-named held stack.
    fn mob_main_hand_name(&self, id: u32) -> Option<Text> {
        let m = self.mobs.iter().find(|m| m.id == id)?;
        let mut stack = m.extra.main_hand_stack()?.clone();
        if !has_custom_name(&stack) {
            return None;
        }
        if let Some(g) = horizon_client::mobs::gear::gear(m) {
            stack.data_mut().enchantments = g.slots[horizon_client::mobs::gear::MAIN].clone();
        }
        Some(self.stack_display_name(&stack))
    }
    /// `getMainHandItem().getDisplayName()` of the player in place, when the
    /// stack has a custom name.
    pub(super) fn main_hand_custom_name(&self) -> Option<Text> {
        let stack = self.inventory.get(self.selected)?.as_ref()?;
        has_custom_name(stack).then(|| self.stack_display_name(stack))
    }
    /// `main_hand_custom_name` of a player who is not in place: the guest
    /// `conn` between turns, or none for the host while a guest acts (their
    /// hotbar slot is kept by the swap).
    fn player_main_hand_custom_name(&self, player: Option<u32>) -> Option<Text> {
        if player == self.lan_acting {
            return self.main_hand_custom_name();
        }
        let g = self.lan.as_ref()?.hub.guest(player?)?;
        let stack = g.inventory.get(g.selected)?.as_ref()?;
        has_custom_name(stack).then(|| self.stack_display_name(stack))
    }
    /// The killer's item for `getLocalizedDeathMessage` and
    /// `getMessageForAssistedFall`: a player's as recorded, a mob's now.
    fn entry_item(&self, e: &Entry) -> Option<Text> {
        e.item
            .clone()
            .or_else(|| self.mob_main_hand_name(e.attacker.as_ref()?.0))
    }
    /// `CombatTracker.getDeathMessage`.
    pub(super) fn combat_death_message(&mut self) -> Text {
        self.combat_death_message_for(Text::literal(PLAYER_NAME))
    }
    /// `CombatTracker.getDeathMessage` for a player whose display name is
    /// `me`: a server's guest is not `PLAYER_NAME`.
    pub(super) fn combat_death_message_for(&mut self, me: Text) -> Text {
        self.recheck_combat();
        let entries = std::mem::take(&mut self.combat.entries);
        self.combat.in_combat = false;
        let Some(killing) = entries.last() else {
            return Text::translate("death.attack.generic", &[me]);
        };
        let def = self
            .motion
            .health
            .damage
            .id(&killing.damage_type)
            .and_then(|id| self.motion.health.damage.definition(id))
            .map(|d| (d.message_id.clone(), d.death_message_type.clone()))
            .unwrap_or_else(|| ("generic".into(), "default".into()));
        self.combat.entries = entries;
        let knock = self.most_significant_fall();
        let entries = std::mem::take(&mut self.combat.entries);
        let killing = entries.last().expect("checked above");
        let killer = killing.attacker.clone();
        let direct = killing.direct.clone();
        if def.1 == "fall_variants"
            && let Some(knock) = knock.and_then(|i| entries.get(i))
        {
            let fall = self.damage_in_tag(&knock.damage_type, "is_fall")
                || self.damage_in_tag(&knock.damage_type, "always_most_significant_fall");
            if fall {
                let at = knock.fall_location.unwrap_or("generic");
                return Text::translate(&format!("death.fell.accident.{at}"), &[me]);
            }
            // CombatTracker.getMessageForAssistedFall: the attacker's item
            // with a custom name gets the `.item` message.
            let (key, args) = match (&knock.attacker, &killer) {
                (Some(a), k) if k.as_ref().is_none_or(|k| k.0 != a.0) => {
                    horizon_client::scoreboard::death_message_args(
                        "death.fell.assist",
                        &me,
                        &a.1,
                        self.entry_item(knock).as_ref(),
                    )
                }
                (_, Some(k)) => horizon_client::scoreboard::death_message_args(
                    "death.fell.finish",
                    &me,
                    &k.1,
                    self.entry_item(killing).as_ref(),
                ),
                _ => ("death.fell.killer".into(), vec![me]),
            };
            return Text::translate(&key, &args);
        }
        let key = format!("death.attack.{}", def.0);
        if def.1 == "intentional_game_design" {
            let link = Text::literal("[")
                .append(Text::translate(&format!("{key}.link"), &[]))
                .append(Text::literal("]"));
            return Text::translate(&format!("{key}.message"), &[me, link]);
        }
        // `getLocalizedDeathMessage`: the causing entity's name, or the
        // direct entity's when nothing caused the damage.
        if let Some((_, name)) = killer {
            // DamageSource.getLocalizedDeathMessage: the cause's item with a
            // custom name gets the `<id>.item` message.
            let (key, args) = horizon_client::scoreboard::death_message_args(
                &key,
                &me,
                &name,
                self.entry_item(killing).as_ref(),
            );
            return Text::translate(&key, &args);
        }
        if let Some(name) = direct {
            return Text::translate(&key, &[me, name]);
        }
        // getKillCredit: the last mob that hurt the player in the past 100 ticks.
        if let Some(((_, name), at)) = &self.combat.last_hurt_by
            && self.world.game_time - at <= 100
        {
            return Text::translate(&format!("{key}.player"), &[me, name.clone()]);
        }
        // No entity at all: the one-argument message, as vanilla sends it (a
        // template that names a killer then shows as written).
        Text::translate(&key, &[me])
    }
    /// `TamableAnimal.die`: with `showDeathMessages`, a tame wolf, cat,
    /// parrot or nautilus's owner, when online in any dimension
    /// (`EntityReference.getEntity` looks through every level), is sent how it
    /// died. `killer` is the player who last hurt it (`None` inside for the
    /// host), from `last_hurt_by_player`.
    pub(super) fn tell_owner_pet_died(&mut self, m: &Mob, killer: Option<Option<u32>>) {
        let tamable = matches!(
            m.species(),
            Species::Wolf
                | Species::Cat
                | Species::Parrot
                | Species::Nautilus
                | Species::ZombieNautilus
        );
        let Some(owner) = m.extra.owner.filter(|_| tamable && m.extra.tame) else {
            return;
        };
        if !self.ui.rules.bool("show_death_messages") {
            return;
        }
        let host = self.player.is_some()
            && self.options.dedicated.is_none()
            && horizon_client::mobs::tame::owned_by(
                Some(owner),
                self.level.player_uuid.unwrap_or_default(),
            );
        let uuid = horizon_client::lan::playerdata::ints_uuid(owner);
        let guest = !host
            && self
                .lan
                .as_ref()
                .is_some_and(|l| l.hub.guests.iter().any(|g| g.placed && g.uuid == uuid));
        if !host && !guest {
            return;
        }
        let message = self.mob_death_message(m, killer);
        if host {
            self.system_message(message);
        } else if let Some(lan) = &self.lan {
            let content = horizon_client::lan::data::component_nbt(&message.component());
            lan.hub.system_message_to_player(uuid, content);
        }
    }
    /// `CombatTracker.getDeathMessage` for a mob, from the hits it recorded
    /// (`Mob::combat`), told the way the player's own tracker tells them.
    pub(super) fn mob_death_message(&mut self, m: &Mob, killer: Option<Option<u32>>) -> Text {
        use horizon_client::mobs::{DamageKind, Who};
        let me = mob_name(m);
        // The player behind a `Who::Player` hit: the last one to hurt it, or
        // the one in place (a server with nobody acting has none).
        let player = match killer {
            Some(Some(conn)) => Some((u32::MAX - 1 - conn, self.target_name(Target::Guest(conn)))),
            Some(None) => Some((u32::MAX, self.target_name(Target::Player))),
            None if self.player.is_some() || self.lan_acting.is_some() => {
                Some((u32::MAX, self.target_name(Target::Player)))
            }
            None => None,
        };
        // DamageSource.getLocalizedDeathMessage reads that player's main hand
        // as the message is built.
        let player_item = match killer {
            Some(conn) => self.player_main_hand_custom_name(conn),
            None => player.as_ref().and_then(|_| self.main_hand_custom_name()),
        };
        // The cause's display name: a mob that is gone (a creeper that blew
        // up) is still named by its blast.
        let name = |g: &Self, who: Who| match who {
            Who::Player => player.clone(),
            Who::Mob(id) => g
                .mob_display_name(id)
                .or_else(|| {
                    g.combat
                        .exploded
                        .iter()
                        .find(|(e, _)| *e == id)
                        .map(|(_, n)| n.clone())
                })
                .map(|n| (id, n)),
        };
        let entries = m
            .combat
            .entries
            .iter()
            .map(|e| {
                let attacker = e.source.attacker.and_then(|w| name(self, w));
                let item = (e.source.attacker == Some(Who::Player))
                    .then(|| player_item.clone())
                    .flatten();
                // AbstractArrow.onHitEntity: an arrow nobody shot is its own
                // cause; a rocket nobody fired still names itself.
                let direct = match e.source.kind {
                    _ if attacker.is_some() => None,
                    DamageKind::Arrow => Some(Text::translate("entity.minecraft.arrow", &[])),
                    DamageKind::Fireworks => {
                        Some(Text::translate("entity.minecraft.firework_rocket", &[]))
                    }
                    _ => None,
                };
                Entry {
                    damage_type: super::creatures::source_damage_type(e.source).to_owned(),
                    damage: e.damage,
                    fall_location: e.in_water.then_some("water"),
                    fall_distance: e.fall_distance,
                    attacker,
                    item,
                    direct,
                }
            })
            .collect();
        // LivingEntity.getKillCredit: the player who hurt it lately, else the
        // last mob to hurt it within 100 ticks.
        let credit = if m.ai.last_hurt_by_player > 0 && player.is_some() {
            player.clone()
        } else {
            m.ai.last_hurt_by
                .filter(|_| m.age as i32 - m.ai.last_hurt_by_time <= 100)
                .and_then(|w| name(self, w))
        };
        let now = self.world.game_time;
        let mut tracker = CombatTracker {
            entries,
            last_damage: now,
            in_combat: m.combat.in_combat,
            last_hurt_by: credit.map(|c| (c, now)),
            ..CombatTracker::default()
        };
        std::mem::swap(&mut self.combat, &mut tracker);
        let text = self.combat_death_message_for(me);
        std::mem::swap(&mut self.combat, &mut tracker);
        text
    }
}

/// `!stack.isEmpty() && stack.has(DataComponents.CUSTOM_NAME)`: the held
/// item that turns a death message into its `.item` form.
fn has_custom_name(stack: &horizon_client::items::ItemStack) -> bool {
    &*stack.item != "minecraft:air"
        && stack.count > 0
        && stack.data().is_some_and(|d| d.custom_name.is_some())
}

/// `Entity.getDisplayName` of a mob: its custom name or its type's name.
fn mob_name(m: &Mob) -> Text {
    match (&m.custom_name_text, &m.custom_name) {
        (Some(text), _) => text.clone(),
        (None, Some(name)) => Text::literal(name.clone()),
        (None, None) => Text::translate(&format!("entity.{}", m.kind.id.replace(':', ".")), &[]),
    }
}

#[cfg(test)]
mod held_item_tests {
    use super::has_custom_name;
    use horizon_client::items::ItemStack;
    use horizon_world::nbt::Tag;

    /// `DamageSource.getLocalizedDeathMessage`: only a non-empty stack with
    /// a `custom_name` component names the item.
    #[test]
    fn only_a_renamed_stack_names_the_item() {
        let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
        assert!(!has_custom_name(&sword));
        // Other components (here a repair cost) are not a custom name.
        sword.data_mut().repair_cost = 1;
        assert!(!has_custom_name(&sword));
        sword.data_mut().custom_name = Some(Tag::String("Peacemaker".into()));
        assert!(has_custom_name(&sword));
        // An empty stack never does, even with the component.
        let mut gone = sword.clone();
        gone.count = 0;
        assert!(!has_custom_name(&gone));
    }
}
#[cfg(test)]
mod noted_tests {
    use super::CombatTracker;
    use horizon_client::text::Text;

    /// A canceled hit forgets what was noted for it, also before its hurt
    /// started (a PvP blow, `/damage`), and nothing a landed hit noted.
    #[test]
    fn a_canceled_hit_forgets_only_its_own_attackers() {
        let mut c = CombatTracker::default();
        let arrow = |c: &mut CombatTracker, id| {
            c.mark();
            c.pending.push((
                "minecraft:arrow".into(),
                (id, Text::literal("skeleton")),
                None,
            ));
        };
        arrow(&mut c, 1);
        c.forget_noted(false);
        arrow(&mut c, 2);
        c.mark();
        c.pending_direct
            .push(("minecraft:arrow".into(), Text::literal("arrow")));
        c.forget_noted(true);
        assert_eq!(c.pending.iter().map(|p| p.1.0).collect::<Vec<_>>(), [1]);
        assert!(c.pending_direct.is_empty());
        // A hit noted and landed, then one noted and canceled: the landed
        // one keeps its attacker.
        c.forget_noted(false);
        arrow(&mut c, 3);
        c.forget_noted(false);
        arrow(&mut c, 4);
        c.forget_noted(true);
        assert_eq!(c.pending.iter().map(|p| p.1.0).collect::<Vec<_>>(), [1, 3]);
    }
}
#[cfg(test)]
mod stuck_tests {
    use super::Stuck;

    /// `LivingEntity.tick`: with `n` stuck the timer starts at
    /// `20 * (30 - n)` and one falls out when it runs down.
    #[test]
    fn arrows_fall_out_after_twenty_ticks_times_the_gap_to_thirty() {
        let mut s = Stuck::default();
        s.add_arrow();
        s.add_arrow();
        let wait = 20 * (30 - 2);
        for _ in 0..wait - 1 {
            assert!(!s.tick());
        }
        assert!(s.tick());
        assert_eq!(s.arrows, 1);
        // The timer starts again from the new count.
        for _ in 0..20 * (30 - 1) - 1 {
            assert!(!s.tick());
        }
        assert!(s.tick());
        assert_eq!(s.arrows, 0);
        assert!(!s.tick());
    }

    #[test]
    fn stingers_have_their_own_timer() {
        let mut s = Stuck::default();
        s.add_stinger();
        s.add_arrow();
        for _ in 0..20 * 29 - 1 {
            assert!(!s.tick());
        }
        // Both timers run 580 ticks.
        assert!(s.tick());
        assert_eq!((s.arrows, s.stingers), (0, 0));
    }
}
