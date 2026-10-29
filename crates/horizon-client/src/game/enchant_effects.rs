//! Enchantment effects on gameplay, read from the data-driven effect
//! components: efficiency and aqua affinity (`Player.getDestroySpeed`),
//! unbreaking (`minecraft:item_damage`), sharpness/smite/bane and power
//! (`minecraft:damage`), knockback, fire aspect, infinity (`minecraft:ammo_use`),
//! thorns (`minecraft:post_attack`), mending (`minecraft:repair_with_xp`) and
//! the armor attribute effects (respiration, depth strider, ...), Soul
//! Speed's `location_changed` and `tick` effects and the
//! `minecraft:location_changed` disk effect (frost walker).
use super::Game;
use horizon_client::{
    enchant::{EffectContext, Enchantments, level_value},
    items::ItemStack,
    mobs::Who,
};
use horizon_core::{position::BlockPos, random::RandomSource};
use horizon_world::{fluid::FluidKind, level::LevelAccess, registry::BlockStateId};
use serde_json::Value;

/// Where Mending found a damaged item (`EnchantedItemInUse.inSlot`).
#[derive(Clone, Copy)]
enum MendSlot {
    Main,
    Off,
    /// An `armor` index (head first).
    Armor(usize),
}

impl Game<'_> {
    /// `Player.getDestroySpeed` after the tool speed: mining efficiency, haste
    /// and mining fatigue.
    pub(super) fn modify_destroy_speed(&self, mut speed: f32) -> f32 {
        if speed > 1.0 {
            let held = self.inventory[self.selected].as_ref();
            speed += self
                .res
                .stations()
                .enchantments
                .attribute_add(held, "minecraft:mining_efficiency") as f32;
        }
        speed * self.potions.effects.mining_speed_factor()
    }
    /// `Attributes.SUBMERGED_MINING_SPEED` (0.2, raised by aqua affinity).
    pub(super) fn submerged_mining_speed(&self) -> f32 {
        let mut total = 0.0;
        for stack in self.armor.iter().flatten() {
            for (attr, _, amount, op) in self.res.stations().enchantments.attributes(stack) {
                if attr == "minecraft:submerged_mining_speed" && op == "add_multiplied_total" {
                    total += amount;
                }
            }
        }
        (0.2 * (1.0 + total)) as f32
    }
    /// `EnchantmentHelper.processDurabilityChange` (unbreaking) for `amount` points.
    pub(super) fn durability_loss(&mut self, stack: &ItemStack, amount: u32) -> u32 {
        let e = &self.res.stations().enchantments;
        if !e.has_component(stack, "minecraft:item_damage") {
            return amount;
        }
        let ctx = EffectContext {
            tool: Some(&stack.item),
            items: Some(self.res.items()),
            ..Default::default()
        };
        e.modify_value(
            Some(stack),
            "minecraft:item_damage",
            &ctx,
            &mut self.world.random,
            amount as f32,
        )
        .max(0.0) as u32
    }
    /// `ItemStack.hurtAndBreak` for the player's `stack`: the wear Unbreaking
    /// leaves, and `ItemDurabilityTrigger` when some is left (`applyDamage`).
    pub(super) fn wear(&mut self, stack: &ItemStack, amount: u32) -> u32 {
        let loss = self.durability_loss(stack, amount);
        if loss > 0 {
            self.trigger_item_durability(stack);
        }
        loss
    }
    /// `getEnchantedDamage - base`: damage bonus of the held weapon against a mob type.
    pub(super) fn enchant_damage_bonus(&mut self, target: &str) -> f32 {
        self.enchanted_damage(target, 0.0)
    }
    /// `Player.getEnchantedDamage`: `base` run through the held weapon's
    /// damage effects against a mob type.
    pub(super) fn enchanted_damage(&mut self, target: &str, base: f32) -> f32 {
        let Some(held) = self.held().cloned() else {
            return base;
        };
        let ctx = EffectContext {
            this: Some(target),
            attacker: Some("minecraft:player"),
            direct_attacker: Some("minecraft:player"),
            is_direct: true,
            damage_type: Some("minecraft:player_attack"),
            items: Some(self.res.items()),
            ..Default::default()
        };
        self.res.stations().enchantments.modify_value(
            Some(&held),
            "minecraft:damage",
            &ctx,
            &mut self.world.random,
            base,
        )
    }
    /// Strength / weakness on the attack damage attribute.
    pub(super) fn effect_attack_bonus(&self) -> f32 {
        let fx = &self.potions.effects;
        fx.amplifier("strength")
            .map_or(0.0, |a| 3.0 * (a + 1) as f32)
            - fx.amplifier("weakness")
                .map_or(0.0, |a| 4.0 * (a + 1) as f32)
    }
    /// `Player.attack`'s extra knockback: `getKnockback` (the weapon's knockback
    /// enchantment, halved) plus 0.5 for a sprinting full-strength hit, and
    /// whether the hit is that knockback attack.
    pub(super) fn extra_knockback(&mut self, target_kind: &str, scale: f32) -> (f32, bool) {
        let knockback_attack = scale > 0.9 && self.player.as_ref().is_some_and(|p| p.sprinting);
        let enchanted = match self.held().cloned() {
            Some(held) => {
                let ctx = EffectContext {
                    this: Some(target_kind),
                    attacker: Some("minecraft:player"),
                    direct_attacker: Some("minecraft:player"),
                    is_direct: true,
                    items: Some(self.res.items()),
                    ..Default::default()
                };
                self.res.stations().enchantments.modify_value(
                    Some(&held),
                    "minecraft:knockback",
                    &ctx,
                    &mut self.world.random,
                    0.0,
                )
            }
            None => 0.0,
        };
        let bonus = if knockback_attack { 0.5 } else { 0.0 };
        (enchanted / 2.0 + bonus, knockback_attack)
    }
    /// `causeExtraKnockback`'s direction, `(sin yRot, -cos yRot)` from Mth's
    /// sine table: the target flies where the attacker looks, not away from
    /// it (straight along an axis when the attacker faces one).
    pub(super) fn extra_knockback_direction(&self) -> (f64, f64) {
        use horizon_core::geometry::trig;
        let yaw = f64::from(self.yaw * trig::DEG_TO_RAD);
        (f64::from(trig::sin(yaw)), -f64::from(trig::cos(yaw)))
    }
    /// The attacker's side of `causeExtraKnockback` as the attacking client
    /// sees it: the server's player stops sprinting, and its flag reaches the
    /// client. (The server's 0.6 speed cut never does: the client's own
    /// `Player.attack` simulates no hurt, so it skips the knockback.)
    pub(super) fn stop_sprint_after_extra_knockback(&mut self) {
        if let Some(p) = self.player.as_mut() {
            let _ = p.set_sprinting(false);
        }
    }
    /// After a successful melee hit: fire aspect and wind burst.
    pub(super) fn post_attack(&mut self, mob: usize) {
        let Some(held) = self.held().cloned() else {
            return;
        };
        let Some(m) = self.mobs.get(mob) else { return };
        let kind = m.kind.id.clone();
        let ctx = EffectContext {
            this: Some(&kind),
            attacker: Some("minecraft:player"),
            direct_attacker: Some("minecraft:player"),
            is_direct: true,
            items: Some(self.res.items()),
            ..Default::default()
        };
        // minecraft:post_attack effects of the attacker's weapon on the victim:
        // ignite (fire aspect) and apply_mob_effect (bane of arthropods).
        let mut ignite = 0.0f32;
        let mut mob_effects = Vec::new();
        for (id, level) in held.enchantments() {
            let Some(def) = self.res.stations().enchantments.get(id) else {
                continue;
            };
            for effect in def.effects["minecraft:post_attack"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let e = &effect["effect"];
                let kind_of = e["type"].as_str();
                if effect["enchanted"] != "attacker"
                    || effect["affected"] != "victim"
                    || !matches!(
                        kind_of,
                        Some("minecraft:ignite" | "minecraft:apply_mob_effect")
                    )
                {
                    continue;
                }
                let mut random = self.world.random.clone();
                let ok = effect.get("requirements").is_none_or(|r| {
                    self.res
                        .stations()
                        .enchantments
                        .condition(r, *level, &ctx, &mut random)
                });
                self.world.random = random;
                if !ok {
                    continue;
                }
                if kind_of == Some("minecraft:ignite") {
                    ignite =
                        ignite.max(horizon_client::enchant::level_value(&e["duration"], *level));
                } else {
                    mob_effects.push((e.clone(), *level));
                }
            }
        }
        if ignite > 0.0
            && let Some(m) = self.mobs.get_mut(mob)
            && !m.kind.flags.fire_immune
        {
            m.ignite_for_ticks((ignite * 20.0) as i32);
        }
        for (effect, level) in mob_effects {
            self.apply_mob_effect(mob, &effect, level);
        }
        self.post_attack_bursts(&held, &kind);
    }
    /// `EnchantmentHelper.doPostPiercingAttackEffects`: the
    /// `minecraft:post_piercing_attack` effects of the weapon in hand after a
    /// jab (Lunge's dash).
    pub(super) fn post_piercing_attack(&mut self) {
        let Some(held) = self.held().cloned() else {
            return;
        };
        let mut effects = Vec::new();
        for (id, level) in held.enchantments() {
            let Some(def) = self.res.stations().enchantments.get(id) else {
                continue;
            };
            for entry in def.effects["minecraft:post_piercing_attack"]
                .as_array()
                .into_iter()
                .flatten()
            {
                effects.push((*level, entry["effect"].clone()));
            }
        }
        // Lunge's requirements: not riding, gliding or in water, and a
        // player outside creative with at least 7 food.
        let Some(p) = self.player.as_ref() else {
            return;
        };
        if effects.is_empty()
            || self.riding.is_some()
            || p.fall_flying
            || p.fluids.was_touching_water
            || !(self.creative || p.health.food.food >= 7)
        {
            return;
        }
        for (level, effect) in effects {
            self.apply_piercing_effect(&effect, level);
        }
    }
    /// One `post_piercing_attack` effect on the player: `AllOf`,
    /// `ChangeItemDamage`, `ApplyExhaustion`, `ApplyEntityImpulse` and
    /// `PlaySoundEffect`.
    fn apply_piercing_effect(&mut self, effect: &serde_json::Value, level: u32) {
        let amount = || level_value(&effect["amount"], level);
        match effect["type"].as_str() {
            Some("minecraft:all_of") => {
                for e in effect["effects"].as_array().into_iter().flatten() {
                    self.apply_piercing_effect(e, level);
                }
            }
            Some("minecraft:change_item_damage") => self.damage_held(amount() as u32),
            Some("minecraft:apply_exhaustion") => {
                if let Some(p) = self.player.as_mut() {
                    p.cause_food_exhaustion(amount());
                }
            }
            // The direction turned by the look (`Entity.getLookQuaternion`:
            // pitch about x, then yaw about y), scaled by coordinate_scale
            // and the magnitude, added to the velocity (the guest's client
            // is told of it by `guest_hit_by`).
            Some("minecraft:apply_impulse") => {
                let vec3 = |key: &str| [0, 1, 2].map(|i| effect[key][i].as_f64().unwrap_or(0.0));
                let (d, scale) = (vec3("direction"), vec3("coordinate_scale"));
                let magnitude = f64::from(level_value(&effect["magnitude"], level));
                let (yaw, pitch) = (
                    f64::from(self.yaw).to_radians(),
                    f64::from(self.pitch).to_radians(),
                );
                let (y1, z1) = (
                    d[1] * pitch.cos() - d[2] * pitch.sin(),
                    d[1] * pitch.sin() + d[2] * pitch.cos(),
                );
                let impulse = horizon_core::geometry::Vec3::new(
                    (d[0] * yaw.cos() - z1 * yaw.sin()) * scale[0] * magnitude,
                    y1 * scale[1] * magnitude,
                    (d[0] * yaw.sin() + z1 * yaw.cos()) * scale[2] * magnitude,
                );
                if let Some(p) = self.player.as_mut() {
                    p.velocity = p.velocity.add_vector(impulse);
                }
            }
            // The sound for the level (a list picks by level), heard by all.
            Some("minecraft:play_sound") => {
                let sounds: Vec<&str> = match &effect["sound"] {
                    serde_json::Value::Array(list) => {
                        list.iter().filter_map(|s| s.as_str()).collect()
                    }
                    one => one.as_str().into_iter().collect(),
                };
                let index = (level as usize).saturating_sub(1);
                if let Some(sound) = sounds.get(index.min(sounds.len().saturating_sub(1))) {
                    let float = |key: &str| effect[key].as_f64().unwrap_or(1.0) as f32;
                    let at = self
                        .player
                        .as_ref()
                        .map(|p| [p.position.x, p.position.y, p.position.z]);
                    self.audio.play(sound, at, float("volume"), float("pitch"));
                }
            }
            _ => {}
        }
    }
    /// `ApplyMobEffect.apply`: one of `to_apply` (a single id or a list) for a
    /// random duration and amplifier between the level-based bounds.
    fn apply_mob_effect(&mut self, mob: usize, effect: &serde_json::Value, level: u32) {
        use horizon_client::enchant::level_value;
        let Some(m) = self.mobs.get_mut(mob) else {
            return;
        };
        let ids: Vec<&str> = match &effect["to_apply"] {
            serde_json::Value::String(id) => vec![id.as_str()],
            serde_json::Value::Array(list) => list.iter().filter_map(|v| v.as_str()).collect(),
            _ => Vec::new(),
        };
        if ids.is_empty() {
            return;
        }
        // HolderSet.getRandomElement, then Mth.randomBetween for both values.
        let pick = m.random.next_int_bound(ids.len() as i32).unwrap_or(0) as usize;
        let mut between = |min: &str, max: &str| {
            let (lo, hi) = (
                level_value(&effect[min], level),
                level_value(&effect[max], level),
            );
            m.random.next_float() * (hi - lo) + lo
        };
        let ticks = (between("min_duration", "max_duration") * 20.0).round() as i32;
        let amplifier = (between("min_amplifier", "max_amplifier").round() as i32).max(0);
        if let Some(id) = super::potions::mob_effect_id(ids[pick]) {
            m.add_effect(id, ticks, amplifier);
        }
    }
    /// `minecraft:post_attack` explode effects on the attacker (wind burst):
    /// a burst at the player's feet that only knocks back and triggers
    /// blocks (`ExplodeEffect` with `block_interaction: trigger`).
    fn post_attack_bursts(&mut self, held: &ItemStack, victim: &str) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        // EntityFlagsPredicate.isFlying: gliding or flying.
        let motion = (p.fall_distance, p.fall_flying || p.flying);
        let feet = [p.position.x, p.position.y, p.position.z];
        let ctx = EffectContext {
            this: Some(victim),
            attacker: Some("minecraft:player"),
            direct_attacker: Some("minecraft:player"),
            is_direct: true,
            items: Some(self.res.items()),
            direct_motion: Some(motion),
            ..Default::default()
        };
        let mut bursts = Vec::new();
        for (id, level) in held.enchantments() {
            let Some(def) = self.res.stations().enchantments.get(id) else {
                continue;
            };
            for effect in def.effects["minecraft:post_attack"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let e = &effect["effect"];
                if effect["enchanted"] != "attacker"
                    || effect["affected"] != "attacker"
                    || e["type"] != "minecraft:explode"
                    || e["block_interaction"] != "trigger"
                {
                    continue;
                }
                let ok = effect.get("requirements").is_none_or(|r| {
                    self.res.stations().enchantments.condition(
                        r,
                        *level,
                        &ctx,
                        &mut self.world.random,
                    )
                });
                if !ok {
                    continue;
                }
                let radius = horizon_client::enchant::level_value(&e["radius"], *level).max(0.0);
                // ExplosionDamageCalculator.getKnockbackMultiplier: 1 unless given.
                let knockback = if e["knockback_multiplier"].is_null() {
                    1.0
                } else {
                    horizon_client::enchant::level_value(&e["knockback_multiplier"], *level)
                };
                bursts.push((radius, knockback));
            }
        }
        for (radius, knockback) in bursts {
            let _ = self.wind_burst(feet, radius, knockback, false, false);
        }
    }
    /// Thorns on worn armor when the mob `attacker` hits the player: the
    /// `affected: attacker` effect of `Enchantment.doPostAttack` hurts the
    /// entity that caused the hit.
    pub(super) fn thorns(&mut self, attacker: u32) {
        let Some(index) = self.mobs.iter().position(|m| m.id == attacker && m.alive()) else {
            return;
        };
        for damage in self.thorns_damage() {
            let m = &mut self.mobs[index];
            m.hurt(damage, None, &mut self.world.random);
            let center = [m.position.x, m.position.y, m.position.z];
            self.audio
                .play("minecraft:enchant.thorns.hit", Some(center), 1.0, 1.0);
        }
    }
    /// The victim's side of thorns: for each worn piece whose chance comes up,
    /// the damage to deal the attacker (`DamageEntity` min 1, max 5) and the
    /// piece's wear (`ChangeItemDamage` 2).
    pub(super) fn thorns_damage(&mut self) -> Vec<f32> {
        let mut hits = Vec::new();
        for slot in 0..4 {
            let Some(stack) = self.armor[slot].clone() else {
                continue;
            };
            let level = stack.enchantment_level("minecraft:thorns");
            if level == 0 {
                continue;
            }
            let Some(def) = self.res.stations().enchantments.get("minecraft:thorns") else {
                continue;
            };
            let effect = def.effects["minecraft:post_attack"][0].clone();
            let ctx = EffectContext {
                items: Some(self.res.items()),
                ..Default::default()
            };
            let mut random = self.world.random.clone();
            let ok = self.res.stations().enchantments.condition(
                &effect["requirements"],
                level,
                &ctx,
                &mut random,
            );
            self.world.random = random;
            if !ok {
                continue;
            }
            hits.push(1.0 + self.world.random.next_float() * 4.0);
            // ChangeItemDamage -> ItemStack.hurtAndBreak: unbreaking applies.
            let loss = self.wear(&stack, 2);
            if let Some(s) = self.armor[slot].as_mut() {
                s.damage += loss;
                if self
                    .res
                    .items()
                    .get(&s.item)
                    .is_some_and(|i| s.damage >= i.max_damage)
                {
                    self.armor[slot] = None;
                }
            }
        }
        hits
    }
    /// `ExperienceOrb.repairPlayerItems`: returns the experience left over.
    pub(super) fn mending(&mut self, amount: u32) -> u32 {
        let mut amount = amount;
        loop {
            // EnchantmentHelper.getRandomItemWith(REPAIR_WITH_XP, damaged), in
            // EquipmentSlot order: the main hand, the off hand, then feet, legs,
            // chest and head (`armor` holds head first).
            let mending = |s: &Option<ItemStack>| {
                s.as_ref()
                    .is_some_and(|s| s.damage > 0 && s.enchantment_level("minecraft:mending") > 0)
            };
            let candidates: Vec<MendSlot> = [MendSlot::Main, MendSlot::Off]
                .into_iter()
                .chain((0..self.armor.len()).rev().map(MendSlot::Armor))
                .filter(|&slot| mending(self.mend_slot(slot)))
                .collect();
            if candidates.is_empty() {
                return amount;
            }
            let slot = candidates[self
                .world
                .random
                .next_int_bound(candidates.len() as i32)
                .unwrap_or(0) as usize];
            let Some(stack) = self.mend_slot(slot).clone() else {
                return amount;
            };
            let ctx = EffectContext {
                items: Some(self.res.items()),
                ..Default::default()
            };
            let to_repair_xp = self.res.stations().enchantments.modify_value(
                Some(&stack),
                "minecraft:repair_with_xp",
                &ctx,
                &mut self.world.random,
                amount as f32,
            ) as u32;
            let repaired = to_repair_xp.min(stack.damage);
            if let Some(s) = self.mend_slot_mut(slot).as_mut() {
                s.damage -= repaired;
            }
            if repaired == 0 || to_repair_xp == 0 {
                return 0;
            }
            let remaining = amount.saturating_sub(repaired * amount / to_repair_xp);
            if remaining == 0 {
                return 0;
            }
            amount = remaining;
        }
    }
    fn mend_slot(&self, slot: MendSlot) -> &Option<ItemStack> {
        match slot {
            MendSlot::Main => &self.inventory[self.selected],
            MendSlot::Off => &self.offhand,
            MendSlot::Armor(i) => &self.armor[i],
        }
    }
    fn mend_slot_mut(&mut self, slot: MendSlot) -> &mut Option<ItemStack> {
        match slot {
            MendSlot::Main => &mut self.inventory[self.selected],
            MendSlot::Off => &mut self.offhand,
            MendSlot::Armor(i) => &mut self.armor[i],
        }
    }
    /// Bow enchantments at release: (extra base damage, infinity keeps the arrow).
    /// `ammo` is the item being fired: `Enchantment.itemContext` hands it to the
    /// `ammo_use` conditions as the tool, so Infinity only frees plain arrows.
    pub(super) fn bow_enchantments(&mut self, ammo: &str) -> (f64, bool) {
        let Some(bow) = self.held().cloned() else {
            return (0.0, false);
        };
        let e = &self.res.stations().enchantments;
        let ctx = EffectContext {
            direct_attacker: Some("minecraft:arrow"),
            attacker: Some("minecraft:player"),
            items: Some(self.res.items()),
            ..Default::default()
        };
        let power = e.modify_value(
            Some(&bow),
            "minecraft:damage",
            &ctx,
            &mut self.world.random,
            0.0,
        );
        let ammo_ctx = EffectContext {
            tool: Some(ammo),
            items: Some(self.res.items()),
            ..Default::default()
        };
        let infinity = e.has_component(&bow, "minecraft:ammo_use")
            && e.modify_value(
                Some(&bow),
                "minecraft:ammo_use",
                &ammo_ctx,
                &mut self.world.random,
                1.0,
            ) == 0.0;
        (f64::from(power), infinity)
    }
    /// `EnchantmentHelper.onProjectileSpawned` ignite effects of the held bow
    /// (flame): the seconds the new arrow burns for.
    pub(super) fn bow_projectile_ignite(&mut self) -> Option<f32> {
        let bow = self.held().cloned()?;
        let ctx = EffectContext {
            this: Some("minecraft:arrow"),
            items: Some(self.res.items()),
            ..Default::default()
        };
        let mut ignite: Option<f32> = None;
        for (id, level) in bow.enchantments() {
            let Some(def) = self.res.stations().enchantments.get(id) else {
                continue;
            };
            for effect in def.effects["minecraft:projectile_spawned"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if effect["effect"]["type"] != "minecraft:ignite" {
                    continue;
                }
                let ok = effect.get("requirements").is_none_or(|r| {
                    self.res.stations().enchantments.condition(
                        r,
                        *level,
                        &ctx,
                        &mut self.world.random,
                    )
                });
                if ok {
                    let seconds =
                        horizon_client::enchant::level_value(&effect["effect"]["duration"], *level);
                    ignite = Some(ignite.map_or(seconds, |s: f32| s.max(seconds)));
                }
            }
        }
        ignite
    }
    /// Soul Speed (`data/minecraft/enchantment/soul_speed.json`), the parts of
    /// `Enchantment.runLocationChangedEffects` and `tick` that file uses. The
    /// `location_changed` effects run when the player enters a new block, lands,
    /// or its boots change:
    /// - the speed and movement efficiency modifiers turn on over a soul speed
    ///   block (not flying, not riding) and stay on while airborne, then off
    ///   (`enchantment_active_check`);
    /// - the boots lose 1 durability with a 4% chance when on such a block.
    ///
    /// Every fifth tick the player walking on one also gets a soul particle
    /// (returned as its position and velocity, for the caller to show or send
    /// to the players around) and sometimes a sound. Runs for the window's own
    /// player and for each guest on the server (`guest_server_tick`); a remote
    /// client leaves it to its server.
    pub(super) fn tick_soul_speed(&mut self) -> Option<([f64; 3], [f64; 3])> {
        const ID: &str = "minecraft:soul_speed";
        let player = self.player.as_mut()?;
        if player.client_side {
            return None;
        }
        let level = self.armor[3]
            .as_ref()
            .map_or(0, |s| s.enchantment_level(ID));
        let position = player.position;
        let (on_ground, flying) = (player.ground.on_ground, player.flying);
        let moved = position.subtract(player.old_position);
        let (tick, width) = (player.health.tick_count, player.dimensions.width);
        let fx = &mut player.location_effects;
        let on_soul = fx.on_soul_speed_block;
        let block = BlockPos::containing(position);
        let changed_block = fx.block.replace(block) != Some(block);
        let landed = std::mem::take(&mut fx.landed);
        // Changed boots: `stopLocationBasedEffects`, then the effects run again.
        let equipped = std::mem::replace(&mut fx.boots_level, level) != level;
        let was_active = fx.applied_level;
        let mut active = was_active > 0 && !equipped;
        let mut wear = false;
        if level > 0 && (changed_block || landed || equipped) {
            let riding = self.riding.is_some();
            let wanted = !riding && !flying && (on_soul || (active && !on_ground));
            active = wanted;
            // `random_chance` is the first term of the wear's `all_of`: it
            // draws from the level random before the block is looked at.
            let roll = self.world.random.next_float() < 0.04;
            wear = roll && on_ground && on_soul;
        }
        fx.applied_level = if active { level } else { 0 };
        // `Entity.getKnownMovement` horizontal speed of at least 1.0E-5.
        let walking = moved.x * moved.x + moved.z * moved.z >= 1.0e-10;
        let tick_effects = level > 0 && !flying && on_ground && walking && on_soul && tick % 5 == 0;
        // The attribute effects were applied to the player before: swap them.
        if was_active > 0 && (!active || equipped) {
            for (attribute, id, _, op) in
                soul_speed_modifiers(&self.res.stations().enchantments, was_active)
            {
                let _ = player.set_modifier(&attribute, &id, 0.0, &op, false);
            }
        }
        if active && (was_active == 0 || equipped) {
            for (attribute, id, amount, op) in
                soul_speed_modifiers(&self.res.stations().enchantments, level)
            {
                let _ = player.set_modifier(&attribute, &id, amount, &op, true);
            }
        }
        if wear && !self.creative {
            self.wear_boots(1);
        }
        if level == 0 {
            return None;
        }
        let r = &mut self.world.random;
        // `spawn_particles`: in the bounding box, 0.1 above the feet, thrown
        // back against the movement (-0.2) and up (0.1).
        let particle = tick_effects.then(|| {
            let x = position.x + (r.next_double() - 0.5) * f64::from(width);
            let z = position.z + (r.next_double() - 0.5) * f64::from(width);
            (
                [x, position.y + 0.1, z],
                [moved.x * -0.2, 0.1, moved.z * -0.2],
            )
        });
        // `play_sound`: its `random_chance` (35%) comes first in its `all_of`,
        // so the level random is drawn every tick the boots are worn; volume
        // 0.6, pitch 0.6 to 1.0.
        if r.next_float() < 0.35 && tick_effects {
            let pitch = r.next_float() * 0.4 + 0.6;
            self.audio.play(
                "minecraft:particle.soul_escape",
                Some([position.x, position.y, position.z]),
                0.6,
                pitch,
            );
        }
        particle
    }
    /// `ItemStack.hurtAndBreak` on the worn boots (`change_item_damage`):
    /// Unbreaking applies, and they break when worn out.
    fn wear_boots(&mut self, amount: u32) {
        let Some(stack) = self.armor[3].clone() else {
            return;
        };
        let loss = self.wear(&stack, amount);
        let max = self
            .res
            .items()
            .get(&stack.item)
            .map_or(0, |info| info.max_damage);
        if max == 0 {
            return;
        }
        let Some(boots) = self.armor[3].as_mut() else {
            return;
        };
        boots.damage += loss;
        if boots.damage >= max {
            self.armor[3] = None;
            if let Some(eye) = self.eye_position() {
                self.audio.play(
                    "minecraft:entity.item.break",
                    Some(eye),
                    0.8,
                    0.8 + self.world.random.next_float() * 0.4,
                );
            }
            self.apply_equipment();
        }
    }
    /// `EnchantmentHelper.runLocationChangedEffects` for the worn armor of the
    /// player, run on the server once the player has moved: `LivingEntity.baseTick`
    /// runs it when the block position changed, `checkFallDamage` when it lands.
    /// `from` and `was_on_ground` are where and how the tick began.
    pub(super) fn location_changed_effects(&mut self, from: [f64; 3], was_on_ground: bool) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let at = [p.position.x, p.position.y, p.position.z];
        let on_ground = p.ground.on_ground;
        let cell = |v: [f64; 3]| v.map(|c| c.floor() as i32);
        if cell(from) == cell(at) && !(on_ground && !was_on_ground) {
            return;
        }
        let riding = self.riding.is_some();
        let mut disks = Vec::new();
        for (stack, slot) in self.armor.iter().zip(["head", "chest", "legs", "feet"]) {
            let Some(stack) = stack else { continue };
            for (id, level) in stack.enchantments() {
                let Some(def) = self.res.stations().enchantments.get(id) else {
                    continue;
                };
                if !def
                    .slots
                    .iter()
                    .any(|s| s == slot || s == "armor" || s == "any")
                {
                    continue;
                }
                for effect in def.effects["minecraft:location_changed"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    if effect["effect"]["type"] == "minecraft:replace_disk"
                        && location_requirements(&effect["requirements"], on_ground, riding)
                    {
                        disks.push((effect["effect"].clone(), *level));
                    }
                }
            }
        }
        for (effect, level) in disks {
            self.replace_disk(&effect, level, at);
        }
    }
    /// `ReplaceDisk.apply`: the blocks of the disk around `at` that pass the
    /// predicate become the effect's state, and each placement is a game event.
    fn replace_disk(&mut self, effect: &Value, level: u32, at: [f64; 3]) {
        let registry = self.pack.assets.registry;
        let spec = &effect["block_state"];
        let Some(default) = spec["id"].as_str().and_then(|n| registry.default_state(n)) else {
            return;
        };
        let Some(mut state) = registry.state(default).cloned() else {
            return;
        };
        for (k, v) in spec["properties"].as_object().into_iter().flatten() {
            state
                .properties
                .insert(k.clone(), v.as_str().unwrap_or("").to_owned());
        }
        let new = registry.resolve(&state).unwrap_or(default);
        let radius = level_value(&effect["radius"], level) as i32;
        let height = level_value(&effect["height"], level) as i32;
        let offset = cell_offset(effect);
        let center = [0, 1, 2].map(|i| at[i].floor() as i32 + offset[i]);
        let event = effect["trigger_game_event"].as_str() == Some("minecraft:block_place");
        for dy in (height - 1).min(0)..=0 {
            for dz in -radius..=radius {
                for dx in -radius..=radius {
                    let pos = [center[0] + dx, center[1] + dy, center[2] + dz];
                    let (x, z) = (
                        f64::from(pos[0]) + 0.5 - at[0],
                        f64::from(pos[2]) + 0.5 - at[2],
                    );
                    if x * x + z * z >= f64::from(radius * radius)
                        || !self.disk_predicate(&effect["predicate"], pos, new)
                        || self.level_set(pos, new).is_err()
                    {
                        continue;
                    }
                    if event {
                        self.post_block_game_event(
                            "minecraft:block_place",
                            pos,
                            Some(Who::Player),
                            Some(new),
                        );
                    }
                }
            }
        }
    }
    /// The `BlockPredicate` subset the disk effect uses, tested at `pos`; no
    /// predicate matches everything.
    fn disk_predicate(&mut self, p: &Value, pos: [i32; 3], placed: BlockStateId) -> bool {
        if p.is_null() {
            return true;
        }
        let offset = cell_offset(p);
        let at = [0, 1, 2].map(|i| pos[i] + offset[i]);
        let state = self.world.block(at[0], at[1], at[2]);
        let b = self.res.behaviours();
        let names = |v: &Value| -> Vec<String> {
            match v {
                Value::String(s) => vec![s.clone()],
                Value::Array(a) => a
                    .iter()
                    .filter_map(|n| n.as_str().map(String::from))
                    .collect(),
                _ => Vec::new(),
            }
        };
        match p["type"].as_str().unwrap_or("") {
            "minecraft:all_of" => p["predicates"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|q| self.disk_predicate(q, pos, placed)),
            // `MatchingBlockTagPredicate`
            "minecraft:matching_block_tag" => b
                .tag(p["tag"].as_str().unwrap_or("").trim_start_matches('#'))
                .is_some_and(|t| t.contains(b.block(state))),
            // `MatchingBlocksPredicate`
            "minecraft:matching_blocks" => {
                names(&p["blocks"]).iter().any(|n| n == b.table.name(state))
            }
            // `MatchingFluidsPredicate`: Fluids.WATER is the source.
            "minecraft:matching_fluids" => {
                let fluid = b.fluid(state);
                names(&p["fluids"]).iter().any(|n| match n.as_str() {
                    "minecraft:water" => fluid.kind() == FluidKind::Water && fluid.is_source(),
                    "minecraft:flowing_water" => {
                        fluid.kind() == FluidKind::Water && !fluid.is_source()
                    }
                    _ => false,
                })
            }
            // `UnobstructedPredicate`: no building-blocking entity in the block.
            "minecraft:unobstructed" => self.with_level(|_, level, _| {
                level.is_unobstructed(BlockPos::new(pos[0], pos[1], pos[2]), placed)
            }),
            _ => false,
        }
    }
    /// Whether a worn enchantment gives immunity to `#burn_from_stepping`
    /// damage (frost walker's `minecraft:damage_immunity`).
    pub(crate) fn wears_burn_from_stepping_immunity(&self) -> bool {
        self.armor
            .iter()
            .zip(["head", "chest", "legs", "feet"])
            .any(|(stack, slot)| {
                stack
                    .iter()
                    .flat_map(ItemStack::enchantments)
                    .any(|(id, _)| {
                        self.res.stations().enchantments.get(id).is_some_and(|def| {
                            def.effects.get("minecraft:damage_immunity").is_some()
                                && def
                                    .slots
                                    .iter()
                                    .any(|s| s == slot || s == "armor" || s == "any")
                        })
                    })
            })
    }
    /// Enchantment attribute modifiers of worn armor (respiration, depth strider...).
    /// Each piece's modifier id ends with its slot (`EnchantmentAttributeEffect.idForSlot`),
    /// so the same enchantment on two pieces stacks instead of replacing itself.
    pub(super) fn apply_enchantment_attributes(&mut self) {
        let mut wanted: Vec<(String, String, f64, String)> = Vec::new();
        for (stack, slot) in self.armor.iter().zip(["head", "chest", "legs", "feet"]) {
            let Some(stack) = stack else { continue };
            for (attr, id, amount, op) in self.res.stations().enchantments.attributes(stack) {
                wanted.push((attr, format!("{id}/{slot}"), amount, op));
            }
        }
        let Some(p) = self.player.as_mut() else {
            return;
        };
        for (attr, id, _, op) in std::mem::take(&mut self.potions.enchant_modifiers) {
            let _ = p.set_modifier(&attr, &id, 0.0, &op, false);
        }
        for (attr, id, amount, op) in &wanted {
            let _ = p.set_modifier(attr, id, *amount, op, true);
        }
        self.potions.enchant_modifiers = wanted;
    }
}

/// Soul Speed's `attribute` effects at `level` (attribute, id, amount, operation),
/// the id taking the boots slot like `EnchantmentAttributeEffect.idForSlot`.
fn soul_speed_modifiers(
    enchantments: &Enchantments,
    level: u32,
) -> Vec<(String, String, f64, String)> {
    let Some(e) = enchantments.get("minecraft:soul_speed") else {
        return Vec::new();
    };
    e.effects["minecraft:location_changed"][0]["effect"]["effects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["type"] == "minecraft:attribute")
        .map(|a| {
            (
                a["attribute"].as_str().unwrap_or("").to_owned(),
                format!("{}/feet", a["id"].as_str().unwrap_or("")),
                f64::from(level_value(&a["amount"], level)),
                a["operation"].as_str().unwrap_or("add_value").to_owned(),
            )
        })
        .collect()
}

/// An effect's or predicate's `offset` vector (zero when absent).
fn cell_offset(v: &Value) -> [i32; 3] {
    let at = |i: usize| {
        v["offset"]
            .get(i)
            .and_then(Value::as_i64)
            .map_or(0, |n| n as i32)
    };
    [at(0), at(1), at(2)]
}
/// The loot-condition subset a `location_changed` effect requires of the
/// wearer: standing on the ground and not riding (frost walker).
fn location_requirements(req: &Value, on_ground: bool, riding: bool) -> bool {
    match req["type"].as_str().unwrap_or("") {
        "" if req.is_null() => true,
        "minecraft:all_of" => req["terms"]
            .as_array()
            .into_iter()
            .flatten()
            .all(|t| location_requirements(t, on_ground, riding)),
        "minecraft:inverted" => !location_requirements(&req["term"], on_ground, riding),
        "minecraft:entity_properties" => {
            let p = &req["predicate"];
            p["minecraft:flags"]["is_on_ground"]
                .as_bool()
                .is_none_or(|v| v == on_ground)
                && (p["minecraft:vehicle"].is_null() || riding)
        }
        _ => false,
    }
}
