//! Maces (`MaceItem`): a hit after falling more than one and a half blocks is
//! a smash attack. It adds damage for the height fallen (more with Density),
//! stops the fall, cancels its damage and knocks the creatures around the
//! target away. The rest of a weapon hit (`Player.itemAttackInteraction`)
//! is here too: `hurtEnemy`, the post-attack enchantments and `postHurtEnemy`.
use super::Game;
use horizon_client::{
    enchant::EffectContext,
    items::ItemStack,
    mobs::{Damage, Mob, Species},
    particles::{ParticleOptions, ParticleType},
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::registry::BlockStateId;

/// `MaceItem.SMASH_ATTACK_FALL_THRESHOLD`.
const FALL_THRESHOLD: f64 = 1.5;
/// `MaceItem.SMASH_ATTACK_HEAVY_THRESHOLD`.
const HEAVY_THRESHOLD: f64 = 5.0;
/// `MaceItem.SMASH_ATTACK_KNOCKBACK_RADIUS`.
const KNOCKBACK_RADIUS: f64 = 3.5;
/// `MaceItem.SMASH_ATTACK_KNOCKBACK_POWER`.
const KNOCKBACK_POWER: f32 = 0.7;

/// `Entity.isAlliedTo` between the player (`player` its UUID) and a mob
/// without teams: `TamableAnimal.considersEntityAsAlly` (a pet it tamed) and
/// `Allay.considersEntityAsAlly` (an allay that likes it).
pub(super) fn ally_of_player(m: &Mob, player: Option<[i32; 4]>) -> bool {
    let tamable = matches!(
        m.species(),
        Species::Wolf
            | Species::Cat
            | Species::Parrot
            | Species::Nautilus
            | Species::ZombieNautilus
    );
    tamable && m.extra.tame && m.extra.owner.is_some() && m.extra.owner == player
        || m.species() == Species::Allay && m.extra.tame
}

/// The entity a sweep leaves out: the one the attack hit (`target` of
/// `Player.doSweepAttack`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Spared {
    Nothing,
    Mob(u32),
    Stand(u32),
    /// A player of the open world, by entity id.
    Player(i32),
}

/// `Player.doSweepAttack`'s `1 + ratio * f`, where `f` is the attack damage
/// after `baseDamageScaleFactor` (0.2 + strength^2 * 0.8).
fn sweep_base_damage(ratio: f32, attack_damage: f32, scale: f32) -> f32 {
    1.0 + ratio * attack_damage * (0.2 + scale * scale * 0.8)
}
/// A smash's knockback on the players, put off while a guest's turn holds
/// the other players.
pub(super) struct SmashPush {
    center: Vec3,
    heavy: f64,
    attacker: Option<u32>,
    team: Option<horizon_client::scoreboard::Team>,
}
/// `MaceItem.getKnockbackPower` and the push of one entity at `at`, `heavy`
/// being 2 after a fall of more than five blocks.
fn smash_push(center: Vec3, heavy: f64, at: Vec3, velocity: &mut Vec3, resistance: f64) {
    let d = [at.x - center.x, at.y - center.y, at.z - center.z];
    let distance_sqr = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    if distance_sqr > KNOCKBACK_RADIUS.powi(2) {
        return;
    }
    let length = distance_sqr.sqrt();
    let power =
        (KNOCKBACK_RADIUS - length) * f64::from(KNOCKBACK_POWER) * heavy * (1.0 - resistance);
    if power <= 0.0 {
        return;
    }
    // Vec3.normalize: a vector shorter than 1e-5 has no direction.
    let (x, z) = if length < f64::from(1.0e-5f32) {
        (0.0, 0.0)
    } else {
        (d[0] / length * power, d[2] / length * power)
    };
    *velocity = Vec3::new(
        velocity.x + x,
        velocity.y + f64::from(KNOCKBACK_POWER),
        velocity.z + z,
    );
}

impl Game<'_> {
    fn holds_mace(&self) -> bool {
        self.inventory[self.selected]
            .as_ref()
            .is_some_and(|s| &*s.item == "minecraft:mace")
    }
    /// `MaceItem.canSmashAttack` for the player: falling and not gliding.
    fn can_smash_attack(&self) -> bool {
        self.player
            .as_ref()
            .is_some_and(|p| p.fall_distance > FALL_THRESHOLD && !p.fall_flying)
    }
    /// A held mace about to smash (`MaceItem.getItemDamageSource`: the attack
    /// is `mace_smash` damage).
    pub(super) fn smash_attack(&self) -> bool {
        self.holds_mace() && self.can_smash_attack()
    }
    /// The damage source of the player's hit from `from` with the held item.
    pub(super) fn attack_source(&self, from: [f64; 3]) -> Damage {
        if self.smash_attack() {
            Damage::mace(from)
        } else {
            Damage::player(from)
        }
    }
    /// `MaceItem.getAttackDamageBonus` against `victim`: 4 per block for the
    /// first three blocks fallen, 2 for the next five, 1 after that, plus
    /// `smash_damage_per_fallen_block` (Density) for every block.
    pub(super) fn smash_damage_bonus(&mut self, victim: &str) -> f32 {
        if !self.smash_attack() {
            return 0.0;
        }
        let fall = self.player.as_ref().map_or(0.0, |p| p.fall_distance);
        let damage = if fall <= 3.0 {
            4.0 * fall
        } else if fall <= 8.0 {
            12.0 + 2.0 * (fall - 3.0)
        } else {
            22.0 + fall - 8.0
        };
        let held = self.inventory[self.selected].clone();
        let ctx = EffectContext {
            this: Some(victim),
            attacker: Some("minecraft:player"),
            direct_attacker: Some("minecraft:player"),
            is_direct: true,
            damage_type: Some("minecraft:mace_smash"),
            items: Some(self.res.items()),
            ..Default::default()
        };
        // EnchantmentHelper.modifyFallBasedDamage(..., 0)
        let per_block = self.res.stations().enchantments.modify_value(
            held.as_ref(),
            "minecraft:smash_damage_per_fallen_block",
            &ctx,
            &mut self.world.random,
            0.0,
        );
        (damage + f64::from(per_block) * fall) as f32
    }
    /// `LivingEntity.hurtServer` of the mob at `index` by the player's hit:
    /// the weapon's `armor_effectiveness` enchantments (Breach) thin its armor.
    pub(super) fn hurt_by_player(&mut self, index: usize, amount: f32, source: Damage) -> bool {
        let held = self.held().cloned();
        let kind = self.mobs[index].kind.id.clone();
        let ctx = EffectContext {
            this: Some(&kind),
            attacker: Some("minecraft:player"),
            direct_attacker: Some("minecraft:player"),
            is_direct: true,
            damage_type: Some(match source.kind {
                horizon_client::mobs::DamageKind::MaceSmash => "minecraft:mace_smash",
                horizon_client::mobs::DamageKind::Spear => "minecraft:spear",
                _ => "minecraft:player_attack",
            }),
            items: Some(self.res.items()),
            ..Default::default()
        };
        let enchantments = &self.res.stations().enchantments;
        let random = &mut self.world.random;
        // CombatRules.getDamageAfterAbsorb: EnchantmentHelper.modifyArmorEffectiveness.
        let mut breach = |fraction: f32| {
            enchantments.modify_value(
                held.as_ref(),
                "minecraft:armor_effectiveness",
                &ctx,
                random,
                fraction,
            )
        };
        let old_health = self.mobs[index].health;
        let hurt = self.mobs[index].damage_with(amount, source, Some(&mut breach));
        if hurt && self.mobs[index].health != old_health {
            self.mob_damage_event(self.mobs[index].id);
        }
        hurt
    }
    /// `Player.itemAttackInteraction` after the player's hit landed on the mob
    /// at `index`: the item's `hurtEnemy` (the "used" statistic of a weapon),
    /// the post-attack enchantments, then `postHurtEnemy` with the weapon's
    /// wear.
    pub(super) fn weapon_hit(&mut self, index: usize) {
        let Some(stack) = self.held().cloned() else {
            return;
        };
        let weapon = self
            .res
            .items()
            .get(&stack.item)
            .and_then(|i| i.weapon_damage.map(|d| (d, i.max_damage)));
        let smash = self.smash_attack();
        if smash {
            self.mace_hurt_enemy(index);
        }
        if weapon.is_some() {
            self.stats.add("used", &stack.item, 1);
        }
        self.post_attack(index);
        // MaceItem.postHurtEnemy
        if smash && let Some(p) = self.player.as_mut() {
            p.fall_distance = 0.0;
        }
        self.weapon_wear(&stack);
    }
    /// `ItemStack.postHurtEnemy`: `hurtAndBreak(weapon.itemDamagePerAttack)` of
    /// the weapon in the acting hand.
    pub(super) fn weapon_wear(&mut self, stack: &ItemStack) {
        let weapon = self
            .res
            .items()
            .get(&stack.item)
            .and_then(|i| i.weapon_damage.map(|d| (d, i.max_damage)));
        if let Some((per_attack, max)) = weapon
            && max > 0
            && !self.creative
        {
            let loss = self.wear(stack, per_attack);
            let cell = self.held_mut();
            if let Some(held) = cell.as_mut() {
                held.damage += loss;
                if held.damage >= max {
                    *cell = None;
                }
            }
        }
    }
    /// `MaceItem.hurtEnemy` for a smash: the fall stops and its damage is
    /// waived from where the hit landed, the ground or air smash sounds, and
    /// the knockback around the target.
    fn mace_hurt_enemy(&mut self, index: usize) {
        let target_on_ground = self.mobs[index].on_ground;
        let Some(p) = self.player.as_mut() else {
            return;
        };
        p.velocity = Vec3::new(p.velocity.x, f64::from(0.01f32), p.velocity.z);
        // A server player's client is told (ClientboundSetEntityMotionPacket): it moves by
        // itself, so without it a Wind Burst's push would add to the speed of the fall.
        p.health.sync_velocity = true;
        // calculateImpactPosition: an earlier impact below the player stays.
        let impact = match p.impulse.impact_position {
            Some(at) if at.y <= p.position.y => at,
            _ => p.position,
        };
        p.impulse.set_ignore_fall_damage(true, Some(impact));
        let fall = p.fall_distance;
        let feet = [p.position.x, p.position.y, p.position.z];
        let sound = if target_on_ground {
            p.spawn_extra_particles_on_fall = true;
            if fall > HEAVY_THRESHOLD {
                "minecraft:item.mace.smash_ground_heavy"
            } else {
                "minecraft:item.mace.smash_ground"
            }
        } else {
            "minecraft:item.mace.smash_air"
        };
        self.audio.play(sound, Some(feet), 1.0, 1.0);
        self.smash_knockback(index, fall);
    }
    /// `MaceItem.knockback`: the dust pillar (level event 2013) under the
    /// target, then every living entity within 3.5 blocks of it but the
    /// player and the target pushed away and up, harder after a fall of more
    /// than five blocks. Spectators, the player's allies (`isAlliedTo`: its
    /// pets and the allays that like it), marker armor stands and creative
    /// flyers are spared; the predicate's own tamed-animal test asks whether
    /// the target owns the animal, which a mob never does.
    fn smash_knockback(&mut self, index: usize, fall: f64) {
        let target = &self.mobs[index];
        let (id, center) = (target.id, target.position);
        // Entity.getOnPos without a supporting block.
        let on_pos = [
            center.x.floor() as i32,
            (center.y - f64::from(1.0e-5f32)).floor() as i32,
            center.z.floor() as i32,
        ];
        self.level_event_at(2013, on_pos, 750);
        let heavy = if fall > HEAVY_THRESHOLD { 2.0 } else { 1.0 };
        let player = self.level.player_uuid.or(Some([0, 0, 0, 1]));
        for m in self
            .mobs
            .iter_mut()
            .filter(|m| m.id != id && m.alive() && !m.removed && !ally_of_player(m, player))
        {
            smash_push(
                center,
                heavy,
                m.position,
                &mut m.velocity,
                m.attrs.knockback_resistance,
            );
        }
        for s in self.armor_stands.iter_mut().filter(|s| !s.marker) {
            smash_push(center, heavy, s.position, &mut s.velocity, 0.0);
        }
        // The host and the LAN guests are living entities too. A guest's turn
        // holds the other players (`as_guest`): the push waits for it to end.
        let push = SmashPush {
            center,
            heavy,
            attacker: self.lan_acting,
            team: self.player_team().cloned(),
        };
        if self.lan_acting.is_some() {
            self.creatures.smash_pushes.push(push);
        } else {
            self.smash_push_players(&push);
        }
    }
    /// `MaceItem.knockback` on the players but the attacker: the host unless
    /// it attacked, and every guest but the attacker.
    fn smash_push_players(&mut self, push: &SmashPush) {
        if push.attacker.is_some() {
            self.smash_push_player(push);
        }
        self.each_guest_hit_except(push.attacker, |g| g.smash_push_player(push));
    }
    /// The pushes guests' smashes put off.
    pub(super) fn flush_smash_pushes(&mut self) {
        for push in std::mem::take(&mut self.creatures.smash_pushes) {
            self.smash_push_players(&push);
        }
    }
    /// `MaceItem.knockback` on the player here (the host, or the guest swapped
    /// in): not a spectator, an ally of the attacker (`isAlliedTo`: the same
    /// team) or a creative player flying.
    fn smash_push_player(&mut self, push: &SmashPush) {
        if self.spectator()
            || horizon_client::scoreboard::teams_allied(push.team.as_ref(), self.player_team())
        {
            return;
        }
        let creative = self.creative;
        let Some(p) = self.player.as_mut() else {
            return;
        };
        if creative && p.flying {
            return;
        }
        let resistance = p.attribute("minecraft:knockback_resistance").unwrap_or(0.0);
        smash_push(
            push.center,
            push.heavy,
            p.position,
            &mut p.velocity,
            resistance,
        );
    }
    /// `Player.doSweepAttack`'s victims: every other living mob and every
    /// armor stand (but a marker one) in `target`'s box inflated by
    /// (1, 0.25, 1) and under three blocks from the player takes
    /// `getEnchantedDamage(1 + sweeping ratio * attack damage) * strength`,
    /// knocked back 0.4 along the player's yaw. The player's allies and the
    /// target (`spared`) are spared.
    pub(super) fn sweep_damage(&mut self, target: Aabb, spared: Spared, scale: f32) {
        let Some(player) = self.player.as_ref() else {
            return;
        };
        let from = [player.position.x, player.position.y, player.position.z];
        let area = target.inflate(Vec3::new(1.0, 0.25, 1.0));
        let near = |at: Vec3| {
            (0..3)
                .map(|a| [at.x, at.y, at.z][a] - from[a])
                .map(|d| d * d)
                .sum::<f64>()
                < 9.0
        };
        // The enchantment's `sweeping_damage_ratio` attribute modifier (level /
        // (level + 1)) on the attribute's 0 to 1 range.
        let ratio = self
            .res
            .stations()
            .enchantments
            .attribute_add(
                self.inventory[self.selected].as_ref(),
                "minecraft:sweeping_damage_ratio",
            )
            .clamp(0.0, 1.0) as f32;
        // `f` of doSweepAttack: the attack damage attribute after
        // baseDamageScaleFactor (a sweep is never a smash or a critical hit).
        let spin = self.player.as_ref().map_or(0.0, |p| p.auto_spin_attack_dmg);
        let attribute = if spin > 0.0 {
            spin
        } else {
            1.0 + self.attack_item_info().map_or(0.0, |i| i.attack_damage)
                + self.effect_attack_bonus()
        };
        let base = sweep_base_damage(ratio, attribute, scale);
        let (xd, zd) = self.extra_knockback_direction();
        let uuid = self.level.player_uuid.or(Some([0, 0, 0, 1]));
        let allies = self.player_allies();
        // getEntitiesOfClass walks the entity sections (x, then z, then y)
        // under the box, each in the order its entities were added.
        let section = |at: Vec3| {
            (
                (at.x.floor() as i32) >> 4,
                (at.z.floor() as i32) >> 4,
                (at.y.floor() as i32) >> 4,
            )
        };
        let mut victims: Vec<((i32, i32, i32), u32, bool)> = Vec::new();
        for m in &self.mobs {
            if spared != Spared::Mob(m.id)
                && m.alive()
                && !m.removed
                && !allies.contains(&m.id)
                && !ally_of_player(m, uuid)
                && m.bounds().intersects(area)
                && near(m.position)
            {
                victims.push((section(m.position), m.id, false));
            }
        }
        for s in &self.armor_stands {
            if !s.marker
                && spared != Spared::Stand(s.id)
                && s.bounds().intersects(area)
                && near(s.position)
            {
                victims.push((section(s.position), s.id, true));
            }
        }
        victims.sort_by_key(|&(key, id, _)| (key, id));
        for (_, id, is_stand) in victims {
            if is_stand {
                let Some(i) = self.armor_stands.iter().position(|s| s.id == id) else {
                    continue;
                };
                // knockback(0.4, ..) follows a hurtServer that landed, on a
                // stand that is still there (LivingEntity.knockback).
                if !self.punch_armor_stand(i) {
                    continue;
                }
                let (mut kx, mut kz) = (xd, zd);
                while kx * kx + kz * kz < f64::from(1.0e-5_f32) {
                    let r = &mut self.world.random;
                    kx = (r.next_double() - r.next_double()) * 0.01;
                    kz = (r.next_double() - r.next_double()) * 0.01;
                }
                let len = (kx * kx + kz * kz).sqrt();
                if let Some(s) = self.armor_stands.iter_mut().find(|s| s.id == id) {
                    s.velocity = Vec3::new(
                        s.velocity.x / 2.0 - kx / len * 0.4,
                        if s.on_ground {
                            (s.velocity.y / 2.0 + 0.4).min(0.4)
                        } else {
                            s.velocity.y
                        },
                        s.velocity.z / 2.0 - kz / len * 0.4,
                    );
                }
                continue;
            }
            let Some(i) = self.mobs.iter().position(|m| m.id == id) else {
                continue;
            };
            let kind = self.mobs[i].kind.id.clone();
            let kind = format!("minecraft:{}", kind.trim_start_matches("minecraft:"));
            let amount = self.enchanted_damage(&kind, base) * scale;
            let source = self.attack_source(from);
            // hurtServer first; the knockback and post-attack effects follow
            // only when it landed.
            if self.hurt_by_player(i, amount, source) {
                self.mobs[i].knockback(0.4, xd, zd);
                self.last_hurt_by_player
                    .insert(self.mobs[i].id, self.lan_acting);
                self.post_attack(i);
            }
        }
        // `getEntitiesOfClass(LivingEntity.class)` takes the other players too.
        let mut sweep = self.player_sweep(base, scale);
        sweep.hit_box = area;
        sweep.skip = match spared {
            Spared::Player(id) => Some(id),
            _ => None,
        };
        self.sweep_other_players(sweep);
    }
    /// `ServerPlayer.checkFallDamage` after a smash on a grounded target:
    /// the landing block's particles, 50 for every block fallen (200 at most).
    pub(super) fn smash_landing_particles(
        &mut self,
        state: BlockStateId,
        pos: BlockPos,
        fall_distance: f64,
    ) {
        let Some((sprite, tint)) = self.particle_sprite(state, [pos.x, pos.y, pos.z]) else {
            return;
        };
        let count = (50.0 * fall_distance).clamp(0.0, 200.0) as u32;
        let spread = f64::from(0.3f32);
        self.particles.send_particles(
            &ParticleOptions::block(ParticleType::Block, sprite, tint),
            [
                f64::from(pos.x) + 0.5,
                f64::from(pos.y) + 1.0,
                f64::from(pos.z) + 0.5,
            ],
            [spread; 3],
            f64::from(0.15f32),
            count,
        );
    }
    /// Level event 2013 (`ParticleUtils.spawnSmashAttackParticles`): dust
    /// pillars of the block at `pos`, a cluster over it and a ring 3.5 out.
    pub(super) fn smash_dust(&mut self, pos: BlockPos, count: i32) {
        let state = self.world.block(pos.x, pos.y, pos.z);
        let Some((sprite, tint)) = self.particle_sprite(state, [pos.x, pos.y, pos.z]) else {
            return;
        };
        let options = ParticleOptions::block(ParticleType::DustPillar, sprite, tint);
        let center = [
            f64::from(pos.x) + 0.5,
            f64::from(pos.y) + 1.0,
            f64::from(pos.z) + 0.5,
        ];
        let near = f64::from(0.2f32);
        let mut i = 0;
        while (i as f32) < count as f32 / 3.0 {
            let r = self.particles.random();
            let x = center[0] + r.next_gaussian() / 2.0;
            let z = center[2] + r.next_gaussian() / 2.0;
            let v = [
                r.next_gaussian() * near,
                r.next_gaussian() * near,
                r.next_gaussian() * near,
            ];
            self.particles.add(&options, [x, center[1], z], v);
            i += 1;
        }
        let far = f64::from(0.05f32);
        let mut i = 0;
        while (i as f32) < count as f32 / 1.5 {
            let r = self.particles.random();
            let angle = f64::from(i);
            let x = center[0] + KNOCKBACK_RADIUS * angle.cos() + r.next_gaussian() / 2.0;
            let z = center[2] + KNOCKBACK_RADIUS * angle.sin() + r.next_gaussian() / 2.0;
            let v = [
                r.next_gaussian() * far,
                r.next_gaussian() * far,
                r.next_gaussian() * far,
            ];
            self.particles.add(&options, [x, center[1], z], v);
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_damage_is_one_without_the_enchantment() {
        // A fully charged diamond sword (8 attack damage): neighbours take 1.
        assert_eq!(sweep_base_damage(0.0, 8.0, 1.0), 1.0);
        // Sweeping Edge III adds the ratio of the (charged) attack damage.
        assert_eq!(sweep_base_damage(0.75, 8.0, 1.0), 7.0);
        // baseDamageScaleFactor at 0.95 strength.
        let scaled = sweep_base_damage(0.5, 8.0, 0.95);
        assert!((scaled - (1.0 + 0.5 * 8.0 * (0.2 + 0.95 * 0.95 * 0.8))).abs() < 1e-6);
    }
}
