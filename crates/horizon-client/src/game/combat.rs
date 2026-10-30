//! The player's `CombatTracker`: the damage taken lately, who dealt it and
//! where the player fell from, turned into the vanilla death message.
use super::{Game, chat::PLAYER_NAME};
use horizon_client::{
    health::HealthEvent,
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
    /// The direct entity's display name when no entity caused the damage
    /// (`DamageSource.directEntity`: a potion with no thrower, a cloud).
    direct: Option<Text>,
}
#[derive(Default)]
pub(super) struct CombatTracker {
    entries: Vec<Entry>,
    last_damage: i64,
    in_combat: bool,
    /// Attackers of damage dealt this tick, claimed by the matching
    /// `RecordedDamage` event.
    pending: Vec<(String, (u32, Text))>,
    /// `pending` for a direct entity with no causing entity: only its name.
    pending_direct: Vec<(String, Text)>,
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
    /// The projectile hitting the player now is another player's that
    /// `ServerPlayer.canHarmPlayer` refuses (the `pvp` rule, an ally).
    pub(super) refuse_player_hit: bool,
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
    /// The exploding mob named itself (`MobEvent::ExplosionSource`), so the
    /// blast leaves it out (`getEntities(source, box)`). A source guessed
    /// from the mobs near the centre is still hurt: it may be a bystander.
    pub(super) explosion_source_named: bool,
    /// The blast now going off comes from a primed TNT, creeper, wither
    /// skull, wither or TNT minecart, the sources that empty a hive they
    /// destroy (`BeehiveBlock.getDrops`).
    pub(super) hive_emptying_blast: bool,
    /// The mob that owns the projectile exploding now (`getIndirectSourceEntity`):
    /// the mobs the blast hurts record it as their attacker.
    pub(super) explosion_owner: Option<u32>,
    /// Charged creepers whose blast just went off and that have not dropped
    /// a head yet (`Creeper.droppedSkulls`): they are gone from the mob list,
    /// but the mobs they killed still ask for their loot.
    pub(super) charged_exploders: Vec<u32>,
    /// The player's fall distance before this tick's movement (as of the
    /// end of the last one).
    fall_before: f64,
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
        self.combat
            .pending
            .push((damage_type.to_owned(), (id, name)));
    }
    /// The `DamageSource.directEntity` of the next `damage_type` damage when
    /// nothing caused it (a splash potion nobody threw): the death message
    /// names it instead (`getLocalizedDeathMessage`). It is no kill credit.
    pub(super) fn note_direct_entity(&mut self, damage_type: &str, name: Text) {
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
                MotionEvent::Health(HealthEvent::StarvationDamage(amount)) => {
                    ("minecraft:starve".to_owned(), *amount)
                }
                _ => continue,
            };
            let attacker = self
                .combat
                .pending
                .iter()
                .position(|(t, _)| *t == name)
                .map(|i| self.combat.pending.remove(i).1);
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
        self.combat.projectile_direct = None;
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
        stack.custom_name()?;
        if let Some(g) = horizon_client::mobs::gear::gear(m) {
            stack.data_mut().enchantments = g.slots[horizon_client::mobs::gear::MAIN].clone();
        }
        Some(self.stack_display_name(&stack))
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
            return match (&knock.attacker, &killer) {
                (Some(a), k) if k.as_ref().is_none_or(|k| k.0 != a.0) => {
                    Text::translate("death.fell.assist", &[me, a.1.clone()])
                }
                (_, Some(k)) => Text::translate("death.fell.finish", &[me, k.1.clone()]),
                _ => Text::translate("death.fell.killer", &[me]),
            };
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
        if let Some((id, name)) = killer {
            // DamageSource.getLocalizedDeathMessage: the cause's item with a
            // custom name gets the `<id>.item` message.
            let (key, args) = horizon_client::scoreboard::death_message_args(
                &key,
                &me,
                &name,
                self.mob_main_hand_name(id).as_ref(),
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
}
