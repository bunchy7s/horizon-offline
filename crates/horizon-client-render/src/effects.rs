//! Player status effects (`MobEffectInstance` / `LivingEntity.activeEffects`):
//! add/update with hidden effects, per-tick application of the periodic
//! effects (regeneration, poison, wither, hunger, saturation, instant
//! effects), and the derived flags the movement and damage code read.
use horizon_client_core::potions;

#[derive(Clone, Debug, PartialEq)]
pub struct EffectInstance {
    /// Effect id without namespace (`speed`, `poison`, ...).
    pub id: &'static str,
    /// Remaining ticks; -1 is infinite.
    pub duration: i32,
    pub amplifier: i32,
    pub ambient: bool,
    pub visible: bool,
    pub hidden: Option<Box<EffectInstance>>,
}
impl EffectInstance {
    pub fn new(id: &'static str, duration: i32, amplifier: i32) -> Self {
        Self {
            id,
            duration,
            amplifier: amplifier.clamp(0, 255),
            ambient: false,
            visible: true,
            hidden: None,
        }
    }
    fn infinite(&self) -> bool {
        self.duration == -1
    }
    /// `MobEffectInstance.endsWithin`.
    pub fn ends_within(&self, ticks: i32) -> bool {
        !self.infinite() && self.duration <= ticks
    }
    fn shorter_than(&self, other: &Self) -> bool {
        !self.infinite() && (self.duration < other.duration || other.infinite())
    }
    /// `MobEffectInstance.update`.
    pub fn update(&mut self, take: &Self) -> bool {
        let mut changed = false;
        if take.amplifier > self.amplifier {
            if take.shorter_than(self) {
                let previous = self.hidden.take();
                let mut copy = self.clone();
                copy.hidden = previous;
                self.hidden = Some(Box::new(copy));
            }
            self.amplifier = take.amplifier;
            self.duration = take.duration;
            changed = true;
        } else if self.shorter_than(take) {
            if take.amplifier == self.amplifier {
                self.duration = take.duration;
                changed = true;
            } else if let Some(h) = self.hidden.as_mut() {
                h.update(take);
            } else {
                self.hidden = Some(Box::new(EffectInstance {
                    hidden: None,
                    ..take.clone()
                }));
            }
        }
        if !take.ambient && self.ambient || changed {
            self.ambient = take.ambient;
            changed = true;
        }
        if take.visible != self.visible {
            self.visible = take.visible;
            changed = true;
        }
        changed
    }
    /// `MobEffectInstance.tickClient`: a client counts its effects down
    /// (the server removes them).
    pub fn tick_client(&mut self) {
        if self.infinite() || self.duration > 0 {
            self.tick_down();
            self.downgrade();
        }
    }
    fn tick_down(&mut self) {
        if let Some(h) = self.hidden.as_mut() {
            h.tick_down();
        }
        if !self.infinite() {
            self.duration -= 1;
        }
    }
    fn downgrade(&mut self) {
        if self.duration == 0
            && let Some(h) = self.hidden.take()
        {
            let h = *h;
            self.duration = h.duration;
            self.amplifier = h.amplifier;
            self.ambient = h.ambient;
            self.visible = h.visible;
            self.hidden = h.hidden;
        }
    }
}
/// What an effect tick asks the game to do to the player.
#[derive(Clone, Debug, PartialEq)]
pub enum EffectAction {
    Heal(f32),
    /// Damage with a damage type id (`minecraft:magic`, `minecraft:wither`).
    Hurt(&'static str, f32),
    /// PoisonMobEffect: one magic damage while above one health.
    Poison,
    Exhaust(f32),
    /// FoodData.eat(nutrition, saturation modifier).
    Eat(i32, f32),
}
/// `shouldApplyEffectTickThisTick` + `applyEffectTick` for the periodic effects.
fn periodic(id: &str, tick: i32, amp: i32) -> Option<EffectAction> {
    let every = |base: i32| {
        let interval = base >> amp;
        interval <= 0 || tick % interval == 0
    };
    match id {
        "regeneration" if every(50) => Some(EffectAction::Heal(1.0)),
        "poison" if every(25) => Some(EffectAction::Poison),
        "wither" if every(40) => Some(EffectAction::Hurt("minecraft:wither", 1.0)),
        "hunger" => Some(EffectAction::Exhaust(0.005 * (amp + 1) as f32)),
        "saturation" if tick >= 1 => Some(EffectAction::Eat(amp + 1, 1.0)),
        "instant_health" if tick >= 1 => Some(EffectAction::Heal((4 << amp.min(30)) as f32)),
        "instant_damage" if tick >= 1 => Some(EffectAction::Hurt(
            "minecraft:magic",
            (6 << amp.min(30)) as f32,
        )),
        _ => None,
    }
}
/// `HealOrHarmMobEffect.applyInstantaneousEffect` (potions apply these directly;
/// the amount rounds to the nearest whole point).
pub fn instant(id: &str, amp: i32, scale: f64) -> Option<EffectAction> {
    match id {
        "instant_health" => Some(EffectAction::Heal(
            (scale * f64::from(4 << amp.min(30)) + 0.5) as i32 as f32,
        )),
        "instant_damage" => Some(EffectAction::Hurt(
            "minecraft:magic",
            (scale * f64::from(6 << amp.min(30)) + 0.5) as i32 as f32,
        )),
        "saturation" => Some(EffectAction::Eat(amp + 1, 1.0)),
        _ => None,
    }
}
/// What an omen's `applyEffectTick` decided.
#[derive(Clone, Debug, PartialEq)]
pub enum OmenTick {
    /// It returned true: the effect counts down as usual.
    Continue,
    /// It returned false: the effect goes.
    Remove,
    /// It added this effect, then returned false.
    Add(EffectInstance),
}
#[derive(Clone, Debug, Default)]
pub struct PlayerEffects {
    pub list: Vec<EffectInstance>,
    pub tick_count: i32,
}
impl PlayerEffects {
    pub fn get(&self, id: &str) -> Option<&EffectInstance> {
        self.list.iter().find(|e| e.id == id)
    }
    pub fn amplifier(&self, id: &str) -> Option<i32> {
        self.get(id).map(|e| e.amplifier)
    }
    /// `LivingEntity.addEffect`; returns whether anything changed.
    pub fn add(&mut self, effect: EffectInstance) -> bool {
        match self.list.iter_mut().find(|e| e.id == effect.id) {
            Some(existing) => existing.update(&effect),
            None => {
                self.list.push(effect);
                true
            }
        }
    }
    pub fn clear(&mut self) {
        self.list.clear();
    }
    /// The effects of a saved player (`Player.active_effects`: id,
    /// duration, amplifier, ambient, particles visible).
    pub fn from_saved(saved: &[(String, i32, i32, bool, bool)]) -> Self {
        let mut effects = Self::default();
        for (id, duration, amplifier, ambient, particles) in saved {
            if let Some(def) = potions::effect(id) {
                let mut e = EffectInstance::new(def.id, *duration, *amplifier);
                e.ambient = *ambient;
                e.visible = *particles;
                effects.add(e);
            }
        }
        effects
    }
    /// `Player.active_effects` for the save.
    pub fn saved(&self) -> Vec<(String, i32, i32, bool, bool)> {
        self.list
            .iter()
            .map(|e| {
                (
                    format!("minecraft:{}", e.id),
                    e.duration,
                    e.amplifier,
                    e.ambient,
                    e.visible,
                )
            })
            .collect()
    }
    /// ClientboundUpdateMobEffectPacket's flags without blending: ambient,
    /// particles visible and the icon shown (always, for these effects).
    pub fn flags(e: &EffectInstance) -> i8 {
        i8::from(e.ambient) | i8::from(e.visible) << 1 | 4
    }
    /// `LivingEntity.tickEffects` (server part) with the omens' effect ticks:
    /// `omen` answers `BadOmenMobEffect` (every tick) and `RaidOmenMobEffect`
    /// (on its last tick); `absorption_empty` is `AbsorptionMobEffect`'s
    /// answer: the effect ends once the absorption is used up. Returns the
    /// actions to apply, in order, and whether Raid Omen was newly added.
    ///
    /// Bad Omen turning into a new Raid Omen adds an entry to the map being
    /// iterated: vanilla's iterator then throws a
    /// `ConcurrentModificationException`, which `tickEffects` swallows. Bad
    /// Omen stays (not counted down) and the effects after it are not
    /// ticked; the next tick refreshes Raid Omen and drops Bad Omen.
    pub fn tick_with(
        &mut self,
        omen: &mut dyn FnMut(&EffectInstance) -> OmenTick,
        absorption_empty: bool,
    ) -> (Vec<EffectAction>, bool) {
        self.tick_count = self.tick_count.wrapping_add(1);
        let mut actions = Vec::new();
        let mut added = false;
        let mut i = 0;
        while i < self.list.len() {
            let e = &self.list[i];
            if !(e.infinite() || e.duration > 0) {
                self.list.remove(i);
                continue;
            }
            let t = if e.infinite() {
                self.tick_count
            } else {
                e.duration
            };
            if let Some(a) = periodic(e.id, t, e.amplifier) {
                actions.push(a);
            }
            let answer = match e.id {
                "bad_omen" => omen(e),
                "raid_omen" if t == 1 => omen(e),
                "absorption" if absorption_empty => OmenTick::Remove,
                _ => OmenTick::Continue,
            };
            match answer {
                OmenTick::Continue => {}
                OmenTick::Remove => {
                    self.list.remove(i);
                    continue;
                }
                OmenTick::Add(effect) => {
                    if let Some(existing) = self.list.iter_mut().find(|x| x.id == effect.id) {
                        existing.update(&effect);
                        self.list.remove(i);
                        continue;
                    }
                    self.list.push(effect);
                    added = true;
                    break;
                }
            }
            let e = &mut self.list[i];
            e.tick_down();
            e.downgrade();
            if e.infinite() || e.duration > 0 {
                i += 1;
            } else {
                self.list.remove(i);
            }
        }
        (actions, added)
    }
    /// `LivingEntity.tickEffects` (server part): actions to apply, in order.
    pub fn tick(&mut self) -> Vec<EffectAction> {
        self.tick_count = self.tick_count.wrapping_add(1);
        let mut actions = Vec::new();
        let tick_count = self.tick_count;
        self.list.retain_mut(|e| {
            if !(e.infinite() || e.duration > 0) {
                return false;
            }
            let t = if e.infinite() { tick_count } else { e.duration };
            if let Some(a) = periodic(e.id, t, e.amplifier) {
                actions.push(a);
            }
            e.tick_down();
            e.downgrade();
            e.infinite() || e.duration > 0
        });
        actions
    }
    /// Effects applied by drinking a potion (`PotionContents.applyToLivingEntity`).
    pub fn potion_effects(
        potion: Option<&str>,
        scale: f32,
    ) -> (Vec<EffectInstance>, Vec<EffectAction>) {
        let mut lasting = Vec::new();
        let mut now = Vec::new();
        if let Some(def) = potion.and_then(potions::potion) {
            for (id, duration, amp) in def.effects {
                let id = potions::effect(id).map_or("speed", |e| e.id);
                if potions::effect(id).is_some_and(|e| e.instant) {
                    now.extend(instant(id, *amp, 1.0));
                } else {
                    // withScaledDuration: max(floor(d * scale), 1).
                    let d = ((*duration as f32) * scale).floor() as i32;
                    lasting.push(EffectInstance::new(id, d.max(1), *amp));
                }
            }
        }
        (lasting, now)
    }
    /// Flags for movement and damage code.
    pub fn status(&self) -> StatusFlags {
        StatusFlags {
            jump_boost: self.amplifier("jump_boost"),
            slow_falling: self.get("slow_falling").is_some(),
            levitation: self.amplifier("levitation"),
            resistance: self.amplifier("resistance"),
            fire_resistance: self.get("fire_resistance").is_some(),
            dolphins_grace: self.get("dolphins_grace").is_some(),
            weaving: self.get("weaving").is_some(),
            water_breathing: self.get("water_breathing").is_some()
                || self.get("conduit_power").is_some(),
            breath_of_the_nautilus: self.get("breath_of_the_nautilus").is_some(),
            protection: Vec::new(),
        }
    }
    /// Attribute modifiers of active effects: (attribute, id, amount, operation).
    pub fn modifiers(&self) -> Vec<(&'static str, &'static str, f64, &'static str)> {
        let mut out = Vec::new();
        for e in &self.list {
            if let Some((a, id, amount, op)) = effect_modifier(e.id) {
                out.push((a, id, amount * f64::from(e.amplifier + 1), op));
            }
        }
        out
    }
    /// `Player.getDestroySpeed` effect factors (haste/conduit, mining fatigue).
    pub fn mining_speed_factor(&self) -> f32 {
        let mut f = 1.0;
        let haste = self.amplifier("haste").max(self.amplifier("conduit_power"));
        if let Some(a) = haste {
            f *= 1.0 + (a + 1) as f32 * 0.2;
        }
        if let Some(a) = self.amplifier("mining_fatigue") {
            f *= horizon_core::jmath::pow(0.3, f64::from(a + 1)) as f32;
        }
        f
    }
}
/// Effect and equipment facts the movement and damage kernels query.
#[derive(Clone, Debug, Default)]
pub struct StatusFlags {
    pub jump_boost: Option<i32>,
    pub slow_falling: bool,
    pub levitation: Option<i32>,
    pub resistance: Option<i32>,
    pub fire_resistance: bool,
    pub dolphins_grace: bool,
    /// Weaving: cobwebs hold half as much (`WebBlock.entityInside`).
    pub weaving: bool,
    pub water_breathing: bool,
    pub breath_of_the_nautilus: bool,
    /// Armor enchantment protection per damage type id (`EnchantmentHelper.getDamageProtection`).
    pub protection: Vec<(String, f32)>,
}
impl StatusFlags {
    pub fn protection(&self, damage_type: &str) -> f32 {
        self.protection
            .iter()
            .find(|(t, _)| *t == damage_type)
            .map_or(0.0, |(_, p)| *p)
    }
}

/// The attribute modifier an effect adds at amplifier 0
/// (`MobEffect.addAttributeModifier`): (attribute, id, amount, operation).
/// Each level multiplies the amount (`amplifier + 1`).
pub fn effect_modifier(effect: &str) -> Option<(&'static str, &'static str, f64, &'static str)> {
    match effect {
        "speed" => Some((
            "minecraft:movement_speed",
            "minecraft:effect.speed",
            0.2,
            "add_multiplied_total",
        )),
        "slowness" => Some((
            "minecraft:movement_speed",
            "minecraft:effect.slowness",
            -0.15,
            "add_multiplied_total",
        )),
        "haste" => Some((
            "minecraft:attack_speed",
            "minecraft:effect.haste",
            0.1,
            "add_multiplied_total",
        )),
        "mining_fatigue" => Some((
            "minecraft:attack_speed",
            "minecraft:effect.mining_fatigue",
            -0.1,
            "add_multiplied_total",
        )),
        "strength" => Some((
            "minecraft:attack_damage",
            "minecraft:effect.strength",
            3.0,
            "add_value",
        )),
        "weakness" => Some((
            "minecraft:attack_damage",
            "minecraft:effect.weakness",
            -4.0,
            "add_value",
        )),
        "jump_boost" => Some((
            "minecraft:safe_fall_distance",
            "minecraft:effect.jump_boost",
            1.0,
            "add_value",
        )),
        "health_boost" => Some((
            "minecraft:max_health",
            "minecraft:effect.health_boost",
            4.0,
            "add_value",
        )),
        "absorption" => Some((
            "minecraft:max_absorption",
            "minecraft:effect.absorption",
            4.0,
            "add_value",
        )),
        "luck" => Some(("minecraft:luck", "minecraft:effect.luck", 1.0, "add_value")),
        "unluck" => Some((
            "minecraft:luck",
            "minecraft:effect.unluck",
            -1.0,
            "add_value",
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_omen_in_a_village_becomes_raid_omen_over_two_ticks() {
        let mut fx = PlayerEffects::default();
        fx.add(EffectInstance::new("bad_omen", 100, 1));
        fx.add(EffectInstance::new("speed", 50, 0));
        let mut convert = |e: &EffectInstance| match e.id {
            "bad_omen" => OmenTick::Add(EffectInstance::new("raid_omen", 600, e.amplifier)),
            _ => OmenTick::Continue,
        };
        // The first tick adds Raid Omen and stops there (the iterator's
        // ConcurrentModificationException): Bad Omen and Speed stay as they were.
        let (_, added) = fx.tick_with(&mut convert, false);
        assert!(added);
        assert_eq!(fx.get("bad_omen").unwrap().duration, 100);
        assert_eq!(fx.get("speed").unwrap().duration, 50);
        assert_eq!(fx.get("raid_omen").unwrap().duration, 600);
        // The second refreshes Raid Omen and drops Bad Omen.
        let (_, added) = fx.tick_with(&mut convert, false);
        assert!(!added);
        assert!(fx.get("bad_omen").is_none());
        assert_eq!(fx.get("speed").unwrap().duration, 49);
        assert_eq!(fx.get("raid_omen").unwrap().amplifier, 1);
    }
    #[test]
    fn raid_omen_acts_on_its_last_tick() {
        let mut fx = PlayerEffects::default();
        fx.add(EffectInstance::new("raid_omen", 2, 0));
        let mut seen = Vec::new();
        let mut watch = |e: &EffectInstance| {
            seen.push(e.duration);
            OmenTick::Remove
        };
        fx.tick_with(&mut watch, false);
        assert!(fx.get("raid_omen").is_some());
        fx.tick_with(&mut watch, false);
        assert!(fx.get("raid_omen").is_none());
        assert_eq!(seen, vec![1]);
    }

    #[test]
    fn update_keeps_stronger_and_hides_longer() {
        let mut fx = PlayerEffects::default();
        fx.add(EffectInstance::new("speed", 9600, 0));
        // A stronger but shorter effect takes over and hides the long one.
        fx.add(EffectInstance::new("speed", 1800, 1));
        let e = fx.get("speed").unwrap();
        assert_eq!((e.amplifier, e.duration), (1, 1800));
        assert_eq!(
            e.hidden.as_ref().map(|h| (h.amplifier, h.duration)),
            Some((0, 9600))
        );
        for _ in 0..1800 {
            fx.tick();
        }
        let e = fx.get("speed").unwrap();
        assert_eq!((e.amplifier, e.duration), (0, 7800));
        // Weaker and shorter: ignored.
        assert!(!fx.add(EffectInstance::new("speed", 100, 0)));
    }

    #[test]
    fn regeneration_and_poison_intervals() {
        let mut fx = PlayerEffects::default();
        fx.add(EffectInstance::new("regeneration", 900, 0));
        let heals = (0..900)
            .flat_map(|_| fx.tick())
            .filter(|a| *a == EffectAction::Heal(1.0))
            .count();
        // Duration 900..1 divisible by 50: 18 heals.
        assert_eq!(heals, 18);
        assert!(fx.get("regeneration").is_none());
        let mut fx = PlayerEffects::default();
        fx.add(EffectInstance::new("poison", 432, 1));
        let hurts = (0..432)
            .flat_map(|_| fx.tick())
            .filter(|a| matches!(a, EffectAction::Poison))
            .count();
        // Interval 25 >> 1 = 12: 432/12 = 36 hits.
        assert_eq!(hurts, 36);
    }

    #[test]
    fn potions_split_instant_and_lasting() {
        let (lasting, now) = PlayerEffects::potion_effects(Some("minecraft:strong_healing"), 1.0);
        assert!(lasting.is_empty());
        assert_eq!(now, vec![EffectAction::Heal(8.0)]);
        let (lasting, _) = PlayerEffects::potion_effects(Some("minecraft:long_swiftness"), 0.25);
        assert_eq!((lasting[0].id, lasting[0].duration), ("speed", 2400));
        let (lasting, _) = PlayerEffects::potion_effects(Some("minecraft:turtle_master"), 1.0);
        assert_eq!(lasting.len(), 2);
        let mut fx = PlayerEffects::default();
        fx.add(EffectInstance::new("haste", 100, 1));
        assert!((fx.mining_speed_factor() - 1.4).abs() < 1e-6);
        assert_eq!(fx.modifiers().len(), 1);
    }

    #[test]
    fn absorption_modifier_scales_with_amplifier() {
        let mut fx = PlayerEffects::default();
        fx.add(EffectInstance::new("absorption", 2400, 0));
        assert_eq!(
            fx.modifiers(),
            vec![(
                "minecraft:max_absorption",
                "minecraft:effect.absorption",
                4.0,
                "add_value",
            )]
        );
        fx.add(EffectInstance::new("absorption", 2400, 3));
        assert_eq!(
            fx.modifiers(),
            vec![(
                "minecraft:max_absorption",
                "minecraft:effect.absorption",
                16.0,
                "add_value",
            )]
        );
    }
}
