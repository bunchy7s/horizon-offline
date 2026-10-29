//! Spears (`KineticWeapon`): holding the use button charges the spear, and
//! from `delay_ticks` on every living entity along the view that the
//! player's speed allows is stabbed: the damage grows with the speed
//! relative to the target, the knockback and the dismount need the
//! attacker's own speed, and each condition runs out after its time
//! (`Item.use`, `ItemStack.onUseTick`, `KineticWeapon.damageEntities`,
//! `Player.stabAttack`). The hit's feedback (`onKineticHit`) reaches the
//! players around as entity event 2. A left click jabs instead
//! (`PiercingWeapon.attack`), which Lunge turns into a dash.
use super::{Game, view_vector};
use horizon_client::{
    attack_strength::delay,
    collisions::SceneCollisions,
    first_person::Hand,
    items::ItemStack,
    mobs::Damage,
    spear::{HIT_FEEDBACK_TICKS, Kinetic, REACH},
};
use horizon_core::{
    geometry::{Aabb, Vec3, traverse_blocks},
    position::BlockPos,
};

/// The key of this window's own player in `Game::kinetic_feedback`.
pub(super) const OWN: i32 = i32::MIN;

impl Game<'_> {
    /// `Item.use` of a kinetic weapon: `startUsingItem` with a new
    /// `recentKineticEnemies`, and `KineticWeapon.makeSound` at the user.
    pub(super) fn use_spear(&mut self, stack: &ItemStack) -> bool {
        let Some(kinetic) = Kinetic::of(&stack.item) else {
            return false;
        };
        if self.using.is_none() {
            self.using = Some((self.held_slot(), 0));
            let actor = self.lan_acting;
            self.stabbed.retain(|(who, _), _| *who != actor);
            if let Some(p) = &self.player {
                let at = [p.position.x, p.position.y, p.position.z];
                self.audio.play(kinetic.sound, Some(at), 1.0, 1.0);
            }
        }
        true
    }

    /// `ServerPlayer.getKnownMovement` of the acting player: what they moved
    /// in the last tick. A guest's comes with their moves. A rider's is its
    /// mount's move (`getKnownSpeed`: the vehicle's, or the move the driver's
    /// client sent for it), so a charge on horseback carries the horse's speed.
    pub(super) fn known_player_movement(&self) -> Vec3 {
        if let Some(i) = self
            .riding
            .and_then(|v| self.mobs.iter().position(|m| m.id == v))
        {
            return self.mob_speed(i).scale(1.0 / 20.0);
        }
        if self.lan_acting.is_some() {
            return self.known_movement;
        }
        self.player
            .as_ref()
            .map_or(Vec3::ZERO, |p| p.position.subtract(self.previous_position))
    }

    /// `Entity.getKnownSpeed` of the mob at `index`, per second: its root
    /// vehicle's (`KineticWeapon.getMotion`).
    fn mob_speed(&self, index: usize) -> Vec3 {
        let mut mob = &self.mobs[index];
        while let Some(vehicle) = mob
            .extra
            .mount
            .and_then(|v| self.mobs.iter().find(|m| m.id == v))
        {
            mob = vehicle;
        }
        mob.position.subtract(mob.previous).scale(20.0)
    }

    /// `Level.clip` with COLLIDER shapes and no fluids: where the segment
    /// first meets a block, if it does (tall grass lets a stab through).
    fn clip_collider(&self, from: Vec3, to: Vec3) -> Option<Vec3> {
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        traverse_blocks(from, to, 512, |p: BlockPos| {
            let state = self.world.block(p.x, p.y, p.z);
            if self.res.facts().is_air(state) {
                return None;
            }
            let shape = self
                .collision
                .collision_shape(&mut scene, state, [p.x, p.y, p.z])
                .ok()?;
            shape.clip(from, to, p).map(|hit| hit.location)
        })
        .ok()
        .flatten()
    }

    /// The boxes of the mobs a spear can hit: alive, and not the vehicle
    /// ridden (`PiercingWeapon.canHitEntity`), with their index.
    pub(super) fn hittable_mob_boxes(&self) -> Vec<(usize, Aabb)> {
        (self.mobs.iter().enumerate())
            .filter(|(_, m)| m.alive() && !m.removed && self.riding != Some(m.id))
            .map(|(i, m)| (i, m.bounds()))
            .collect()
    }

    /// `ProjectileUtil.getHitEntitiesAlong` with the spear's `AttackRange`.
    fn stab_targets(&self, eye: Vec3, look: Vec3, movement: Vec3) -> Vec<usize> {
        // AttackRange.effectiveMinRange and effectiveMaxRange for a player.
        let (min, max) = if self.creative {
            (REACH.min_creative, REACH.max_creative)
        } else {
            (REACH.min, REACH.max)
        };
        let reach = (min, max, REACH.hitbox_margin);
        let clip = |from, to| self.clip_collider(from, to);
        let boxes = self.hittable_mob_boxes();
        hits_in_reach(&boxes, eye, look, movement, reach, clip)
            .into_iter()
            .map(|(i, _)| i)
            .collect()
    }

    /// `KineticWeapon.damageEntities`: one tick of the spear in use, `ticks`
    /// ticks into the use.
    pub(super) fn spear_tick(&mut self, stack: &ItemStack, ticks: u32) {
        let Some(kinetic) = Kinetic::of(&stack.item) else {
            return;
        };
        let used = ticks as i32 - kinetic.delay_ticks;
        let (Some(eye), Some(feet)) = (
            self.eye_position(),
            self.player.as_ref().map(|p| p.position),
        ) else {
            return;
        };
        if used < 0 || self.spectator() {
            return;
        }
        let eye = Vec3::new(eye[0], eye[1], eye[2]);
        let look = view_vector(self.yaw, self.pitch);
        let look = Vec3::new(look[0], look[1], look[2]);
        let movement = self.known_player_movement();
        let attacker_speed = look.dot(movement.scale(20.0));
        let now = self.world.game_time;
        let actor = self.lan_acting;
        let mut affected = false;
        for i in self.stab_targets(eye, look, movement) {
            let key = (actor, self.mobs[i].id);
            let recent = self
                .stabbed
                .get(&key)
                .is_some_and(|&t| now - t < i64::from(kinetic.contact_cooldown_ticks));
            if recent {
                continue;
            }
            self.stabbed.insert(key, now);
            let target_speed = look.dot(self.mob_speed(i));
            let relative = (attacker_speed - target_speed).max(0.0);
            // An entity factor of 1 for a player.
            let dismounts = kinetic.dismount.test(used, attacker_speed, relative, 1.0);
            let pushes = kinetic.knockback.test(used, attacker_speed, relative, 1.0);
            let damages = kinetic.damage.test(used, attacker_speed, relative, 1.0);
            if dismounts || pushes || damages {
                // The attack damage attribute's base value (1) and the speed.
                let amount = 1.0 + (relative * f64::from(kinetic.damage_multiplier)).floor() as f32;
                affected |= self.stab_mob(i, amount, damages, pushes, dismounts);
            }
        }
        if affected {
            // Level.broadcastEntityEvent(player, 2), felt by the player too.
            if self.audio.capture.is_some() {
                self.lan_kinetic_hits.push(actor);
            }
            let who = actor.map_or(OWN, |conn| conn as i32);
            self.kinetic_hit_feedback(who, &kinetic, [feet.x, feet.y, feet.z]);
            // SPEAR_MOBS_TRIGGER: the living entities stabbed in this use.
            let stabbed = self.stabbed.keys().filter(|(who, _)| *who == actor).count();
            self.trigger_spear_mobs(stabbed);
        }
    }

    /// `Player.stabAttack` on the mob at `index`: whether it was affected.
    fn stab_mob(
        &mut self,
        index: usize,
        base_damage: f32,
        deals_damage: bool,
        deals_knockback: bool,
        dismounts: bool,
    ) -> bool {
        let kind = format!(
            "minecraft:{}",
            self.mobs[index].kind.id.trim_start_matches("minecraft:")
        );
        let id = self.mobs[index].id;
        let Some(p) = self.player.as_ref() else {
            return false;
        };
        let from = [p.position.x, p.position.y, p.position.z];
        // The enchantments' share, scaled by the attack strength unless the
        // weapon is in use.
        let magic_boost = self.enchant_damage_bonus(&kind);
        let (base_damage, magic_boost) = if self.using.is_none() {
            let scale = self.attack_strength_scale();
            (
                base_damage * (0.2 + scale * scale * 0.8),
                magic_boost * scale,
            )
        } else {
            (base_damage, magic_boost)
        };
        let total = if deals_damage {
            base_damage + magic_boost
        } else {
            0.0
        };
        let old_health = self.mobs[index].health;
        let hurt = deals_damage && self.hurt_by_player(index, total, Damage::spear(from));
        // causeExtraKnockback: a push of 0.4, then the weapon's Knockback.
        if deals_knockback {
            let (xd, zd) = self.extra_knockback_direction();
            self.mobs[index].knockback(0.4, xd, zd);
            let (strength, _) = self.extra_knockback(&kind, 0.0);
            self.mobs[index].knockback(f64::from(strength), xd, zd);
            // ...and the attacker stops sprinting.
            self.stop_sprint_after_extra_knockback();
        }
        let dismounted = dismounts && self.mobs[index].extra.mount.is_some();
        if dismounted {
            self.dismount_mob(id);
        }
        if !hurt && !deals_knockback && !dismounted {
            return false;
        }
        let mob = &self.mobs[index];
        let (feet, width, height) = (
            [mob.position.x, mob.position.y, mob.position.z],
            mob.kind.width,
            mob.kind.height,
        );
        // attackVisualEffects: no attack sound, the enchantments' magic crit.
        if magic_boost > 0.0 {
            self.critical_hit(self.lan_acting, id as i32, feet, width, height, true);
        }
        self.combat.last_hurt_mob = Some((id, self.world.game_time));
        // itemAttackInteraction: hurtEnemy (statistic, wear), and the
        // post-attack enchantments when the target was hurt.
        if let Some(stack) = self.held().cloned() {
            self.stats.add("used", &stack.item, 1);
            if hurt {
                self.post_attack(index);
            }
            self.weapon_wear(&stack);
        }
        // damageStatsAndHearts.
        let actual = old_health - self.mobs[index].health;
        self.stats
            .custom("damage_dealt", (actual * 10.0).round() as i32);
        if actual > 2.0 {
            let indicator = horizon_client::particles::ParticleOptions::simple(
                horizon_client::particles::ParticleType::DamageIndicator,
            );
            self.particles.send_particles(
                &indicator,
                [feet[0], feet[1] + height * 0.5, feet[2]],
                [0.1, 0.0, 0.1],
                0.2,
                (actual * 0.5) as u32,
            );
        }
        if hurt {
            self.last_hurt_by_player.insert(id, self.lan_acting);
        }
        if let Some(p) = self.player.as_mut()
            && !self.creative
        {
            p.cause_food_exhaustion(0.1);
        }
        true
    }

    /// `Player.cannotAttackWithItem` for the held item: a spear needs its
    /// `minimum_attack_charge` of 1, counting `tolerance` ticks more than the
    /// `ticker` ticks charged.
    pub(super) fn cannot_attack_with_item(&self, ticker: i32, tolerance: i32) -> bool {
        self.held().is_some_and(|s| Kinetic::of(&s.item).is_some())
            && (ticker + tolerance) as f32 / delay(self.main_hand_attack_speed()) < 1.0
    }

    /// `PiercingWeapon.attack` of the spear in the main hand, for a left
    /// click: every mob along the view is stabbed, then `postPiercingAttack`
    /// (Lunge), the hit sound and the swing. Gives the sound of the jab
    /// (`makeSound`), which everyone but the user hears from the server.
    pub(super) fn jab(&mut self) -> Option<&'static str> {
        let kinetic = self.held().and_then(|s| Kinetic::of(&s.item))?;
        let eye = self.eye_position()?;
        let eye = Vec3::new(eye[0], eye[1], eye[2]);
        let look = view_vector(self.yaw, self.pitch);
        let look = Vec3::new(look[0], look[1], look[2]);
        // The attack damage attribute.
        let damage = 1.0
            + self.attack_item_info().map_or(0.0, |i| i.attack_damage)
            + self.effect_attack_bonus();
        let movement = self.known_player_movement();
        let mut hit = false;
        for i in self.stab_targets(eye, look, movement) {
            hit |= self.stab_mob(i, damage, true, true, false);
        }
        // Player.onAttack, then postPiercingAttack.
        self.attack_ticks = 0;
        self.post_piercing_attack();
        if hit {
            self.play_at_feet(kinetic.hit_sound);
        }
        // swingAndResetAttackStrength (ServerPlayer): a swing that starts
        // resets the attack strength.
        let swings = self.swings_started;
        self.swing_hand(Hand::Main, kinetic.attack_animation());
        if self.swings_started != swings {
            self.reset_attack_strength();
        }
        Some(kinetic.attack_sound)
    }

    /// A sound at the player's feet.
    pub(super) fn play_at_feet(&mut self, sound: &str) {
        if let Some(p) = &self.player {
            let at = [p.position.x, p.position.y, p.position.z];
            self.audio.play(sound, Some(at), 1.0, 1.0);
        }
    }

    /// `LivingEntity.onKineticHit` for `entity`: the hit feels (the spear
    /// moves back, `kinetic_hit_ticks`) and sounds, at most once every ten
    /// ticks.
    pub(super) fn kinetic_hit_feedback(&mut self, entity: i32, kinetic: &Kinetic, at: [f64; 3]) {
        let now = self.world.game_time;
        if self
            .kinetic_feedback
            .get(&entity)
            .is_none_or(|&t| now - t > HIT_FEEDBACK_TICKS)
        {
            self.kinetic_feedback.insert(entity, now);
            self.audio
                .play_unshared(kinetic.hit_sound, Some(at), 1.0, 1.0);
        }
    }

    /// `getTicksSinceLastKineticHitFeedback` of this window's player.
    pub(super) fn kinetic_hit_ticks(&self, partial: f32) -> f32 {
        self.kinetic_feedback
            .get(&OWN)
            .map_or(0.0, |&t| (self.world.game_time - t) as f32 + partial)
    }

    /// `LivingEntity.handleEntityEvent(2)` on a server's client: the
    /// player `id` (this one, or another) was hit by the spear it uses.
    pub(super) fn remote_kinetic_hit(&mut self, id: i32) {
        let own = self.remote.as_ref().is_some_and(|r| r.entity_id == id);
        let (item, at) = if own {
            let item = self
                .using
                .and_then(|(slot, _)| self.stack_in(slot))
                .map(|s| s.item.clone());
            let at = self.player.as_ref().map(|p| p.position);
            (item, at)
        } else {
            let mob = self
                .mobs
                .iter()
                .find(|m| m.id == id as u32 && m.extra.using_item > 0);
            let item = mob.and_then(|m| {
                if m.extra.using_off_hand {
                    m.extra.off_hand.clone()
                } else {
                    m.extra.main_hand.clone()
                }
            });
            (item, mob.map(|m| m.position))
        };
        if let (Some(kinetic), Some(at)) = (item.as_deref().and_then(Kinetic::of), at) {
            self.kinetic_hit_feedback(if own { OWN } else { id }, &kinetic, [at.x, at.y, at.z]);
        }
    }
}

/// `ProjectileUtil.getManyEntityHitResult` over `boxes`: those the segment
/// `from`..`to` meets, or comes within the margin of with no block between
/// (`clip`: where a segment first meets a block), each with where it is hit.
fn hits_along(
    boxes: &[(usize, Aabb)],
    from: Vec3,
    to: Vec3,
    margin: f64,
    clip: &impl Fn(Vec3, Vec3) -> Option<Vec3>,
) -> Vec<(usize, Vec3)> {
    let grow = Vec3::new(margin, margin, margin);
    let mut hits = Vec::new();
    for &(i, bounds) in boxes {
        let hit = if bounds.contains(from) {
            Some(from)
        } else if let Some(at) = bounds.clip(from, to) {
            Some(at)
        } else if margin > 0.0 {
            bounds.inflate(grow).clip(from, to).and_then(|outside| {
                let centre = bounds.center();
                let toward = clip(outside, centre).unwrap_or(centre);
                bounds.clip(outside, toward)
            })
        } else {
            None
        };
        if let Some(at) = hit {
            hits.push((i, at));
        }
    }
    hits
}

/// `ProjectileUtil.getHitEntitiesAlong` with an `AttackRange` of `reach` (its
/// effective min and max range and hitbox margin): the boxes along `look`
/// from the minimum reach to the maximum plus the attacker's own movement,
/// none when a block (`clip`) is closer than the minimum.
pub(super) fn hits_in_reach(
    boxes: &[(usize, Aabb)],
    eye: Vec3,
    look: Vec3,
    movement: Vec3,
    (min, max, margin): (f32, f32, f32),
    clip: impl Fn(Vec3, Vec3) -> Option<Vec3>,
) -> Vec<(usize, Vec3)> {
    let from = eye.add_vector(look.scale(f64::from(min)));
    let mut to = eye.add_vector(look.scale(f64::from(max) + movement.dot(look).max(0.0)));
    // A block closer than the minimum reach takes the whole stab.
    if let Some(block) = clip(eye, to) {
        to = block;
        if eye.distance_squared(to) < eye.distance_squared(from) {
            return Vec::new();
        }
    }
    hits_along(boxes, from, to, f64::from(margin), &clip)
}
