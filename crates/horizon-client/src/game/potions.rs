//! Potions and status effects in the game: drinking (`PotionItem` via
//! `Consumable`), throwing splash/lingering potions (`ThrownSplashPotion`),
//! filling glass bottles (`BottleItem`), the per-tick effect application,
//! effect attribute modifiers and the HUD effect icons (`Hud.extractEffects`).
use super::{
    Game, Screen,
    dragon::{Breath, FightPlayer, FightWho},
    item_draw::owner_at,
    remote_projectiles::cloud_client_tick,
    splash_water::thrown_block_hit,
};
use horizon_assets::item_model::DisplayContext;
use horizon_client::{
    block_entity_render::vertex_light,
    brewing,
    effects::{EffectAction, EffectInstance, PlayerEffects},
    hud::HudBatch,
    hud_layout, hud_overlays, inventory_effects,
    items::ItemStack,
    mesher::Vertex,
    particles::{ParticleOptions, ParticleType},
    text::Text,
};
use horizon_core::{
    geometry::{Aabb, Direction, Vec3},
    random::{LegacyRandom, RandomSource},
};

/// A thrown splash or lingering potion in flight.
#[derive(Clone, Debug)]
pub(super) struct ThrownPotion {
    /// Its entity id once a server's clients track it (0 until then).
    id: u32,
    stack: ItemStack,
    /// The position before the last tick, for drawing between ticks.
    previous: Vec3,
    position: Vec3,
    velocity: Vec3,
    age: u32,
    /// `Projectile.getOwner`'s display name: the player who threw it (none
    /// for a dispenser's).
    owner: Option<Text>,
}
/// `AreaEffectCloud` wait time set by `ThrownLingeringPotion.onHitAsPotion`.
pub(super) const LINGERING_WAIT: i32 = 10;
/// `AreaEffectCloud.DEFAULT_REAPPLICATION_DELAY`.
const REAPPLICATION_DELAY: i32 = 20;
/// An entity a cloud has affected (`AreaEffectCloud.victims`).
#[derive(Clone, Copy, PartialEq)]
enum Victim {
    Mob(u32),
    Player(FightWho),
}
/// What a lingering potion's `AreaEffectCloud` applies: the potion's effects
/// (at the lingering item's 0.25 duration scale) and who it has affected.
pub(super) struct PotionCloud {
    effects: &'static [(&'static str, i32, i32)],
    /// The potion's colour (`PotionContents.getColor`).
    color: u32,
    /// Each victim and the tick it can be affected again.
    victims: Vec<(Victim, i32)>,
    /// Who its instant damage names (`indirectMagic(cloud, owner)`): the
    /// thrower, else the cloud.
    source: Text,
}
impl PotionCloud {
    /// `AreaEffectCloud.updateParticle`: an entity effect in the potion's colour.
    pub(super) fn particle(&self) -> ParticleOptions {
        let c = |shift: u32| (self.color >> shift & 255) as f32 / 255.0;
        ParticleOptions::colored(ParticleType::EntityEffect, [c(16), c(8), c(0), 1.0])
    }
}
/// A mob's effect ids carry the `minecraft:` namespace.
pub(super) fn mob_effect_id(id: &str) -> Option<&'static str> {
    use std::{collections::HashMap, sync::OnceLock};
    static IDS: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    let id = id.trim_start_matches("minecraft:");
    IDS.get_or_init(|| {
        brewing::EFFECTS
            .iter()
            .map(|e| {
                let full: &'static str = Box::leak(format!("minecraft:{}", e.id).into_boxed_str());
                (e.id, full)
            })
            .collect()
    })
    .get(id)
    .copied()
}
/// `LivingEntity.canBeAffected` with the mob overrides (`WitherBoss` and
/// `WitherSkeleton` shrug off wither, `Spider` and `AbstractNautilus` poison,
/// `Parched` weakness): the immune-to-infested and immune-to-oozing tags
/// decide first, then the tag of mobs that ignore poison and regeneration.
pub(super) fn mob_can_be_affected(
    tags: &std::collections::HashMap<String, std::collections::HashSet<String>>,
    kind: &str,
    effect: &str,
) -> bool {
    let effect = effect.trim_start_matches("minecraft:");
    let kind = kind.trim_start_matches("minecraft:");
    let overridden = match kind {
        "wither" | "wither_skeleton" => effect == "wither",
        "spider" | "cave_spider" | "nautilus" | "zombie_nautilus" => effect == "poison",
        "parched" => effect == "weakness",
        _ => false,
    };
    if overridden {
        return false;
    }
    let in_tag = |tag: &str| {
        tags.get(tag)
            .is_some_and(|t| t.contains(&format!("minecraft:{kind}")))
    };
    if in_tag("minecraft:immune_to_infested") {
        effect != "infested"
    } else if in_tag("minecraft:immune_to_oozing") {
        effect != "oozing"
    } else {
        !in_tag("minecraft:ignores_poison_and_regen")
            || (effect != "regeneration" && effect != "poison")
    }
}
/// `ThrownSplashPotion.onHitAsPotion`: the squared distance from the potion's
/// box at the hit point to an entity's box grown by the hit margin
/// (`ProjectileUtil.computeMargin`), for entities that meet the potion's box
/// grown by (4, 2, 4); none when it is 16 or more.
fn splash_distance_sqr(at: Vec3, age: u32, bounds: Aabb) -> Option<f64> {
    let potion = Aabb::new(
        Vec3::new(at.x - 0.125, at.y, at.z - 0.125),
        Vec3::new(at.x + 0.125, at.y + 0.25, at.z + 0.125),
    );
    if !potion.inflate(Vec3::new(4.0, 2.0, 4.0)).intersects(bounds) {
        return None;
    }
    let margin = ((f64::from(age) - 2.0) / 20.0).clamp(0.0, 0.3);
    let d2 = potion.distance_squared_to_box(bounds.inflate(Vec3::new(margin, margin, margin)));
    (d2 < 16.0).then_some(d2)
}
impl Game<'_> {
    /// `LivingEntity.triggerOnDeathMobEffects` (`onMobRemoved` with the reason
    /// KILLED) for what Oozing, Weaving and Wind Charged do when their bearer
    /// dies. `bearer` is the dying mob's id, `player` whether it is a player,
    /// `random` the dying mob's own (`mob.getRandom()`; the world's for the
    /// player).
    pub(super) fn effects_on_death(
        &mut self,
        effects: &[&str],
        bounds: Aabb,
        player: bool,
        bearer: Option<u32>,
        mut random: Option<&mut LegacyRandom>,
    ) {
        let feet = bounds.bottom_center();
        if effects.contains(&"oozing") {
            // OozingMobEffect.numberOfSlimesToSpawn: two, fewer when the
            // slimes close by already fill the cramming limit.
            let limit = self.ui.rules.int("max_entity_cramming");
            let mut count = 2;
            if limit >= 1 {
                let near = bounds.inflate(Vec3::new(2.0, 2.0, 2.0));
                let nearby = self
                    .mobs
                    .iter()
                    .filter(|m| {
                        m.species() == horizon_client::mobs::Species::Slime
                            && Some(m.id) != bearer
                            && m.bounds().intersects(near)
                    })
                    .count()
                    .min(limit as usize) as i32;
                count = (limit - nearby).clamp(0, count);
            }
            for _ in 0..count {
                // spawnSlimeOffspring: a size 2 slime half a block up.
                if let Some(id) = self.spawn_mob_for(
                    "minecraft:slime",
                    [feet.x, feet.y + 0.5, feet.z],
                    false,
                    true,
                ) && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    horizon_client::mobs::cube::set_size(m, 2);
                }
            }
        }
        if effects.contains(&"weaving") && (player || self.ui.rules.bool("mob_griefing")) {
            self.spawn_cobwebs_around(
                [
                    feet.x.floor() as i32,
                    feet.y.floor() as i32,
                    feet.z.floor() as i32,
                ],
                random.as_deref_mut(),
            );
        }
        if effects.contains(&"wind_charged") {
            // A burst centred at half the height, radius 3 to 5.
            let center = [feet.x, feet.y + bounds.size().y / 2.0, feet.z];
            let r = random.unwrap_or(&mut self.world.random);
            let radius = 3.0 + r.next_float() * 2.0;
            let _ = self.wind_burst(center, radius, 1.0, false, false);
        }
    }
    /// `WeavingMobEffect.spawnCobwebsRandomlyAround`: two or three cobwebs in
    /// free spaces with a solid floor, picked among 15 random blocks within one
    /// block of `pos`.
    fn spawn_cobwebs_around(&mut self, pos: [i32; 3], mut random: Option<&mut LegacyRandom>) {
        let r = random.as_deref_mut().unwrap_or(&mut self.world.random);
        let wanted = 2 + r.next_int_bound(2).unwrap_or(0) as usize;
        let mut chosen: Vec<[i32; 3]> = Vec::new();
        for _ in 0..15 {
            let r = random.as_deref_mut().unwrap_or(&mut self.world.random);
            let at = [
                pos[0] - 1 + r.next_int_bound(3).unwrap_or(0),
                pos[1] - 1 + r.next_int_bound(3).unwrap_or(0),
                pos[2] - 1 + r.next_int_bound(3).unwrap_or(0),
            ];
            let facts = self.res.facts();
            let here = facts.get(self.world.block(at[0], at[1], at[2]));
            let below = facts.get(self.world.block(at[0], at[1] - 1, at[2]));
            if !chosen.contains(&at)
                && here.replaceable
                && below.sturdy_full & (1 << Direction::Up as u8) != 0
            {
                chosen.push(at);
                if chosen.len() >= wanted {
                    break;
                }
            }
        }
        for at in chosen {
            if self.set_state_named(at, "minecraft:cobweb", &[]).is_ok() {
                self.level_event_at(3018, at, 0);
            }
        }
    }
    /// `InfestedMobEffect.onMobHurt` for the player: from half its height up,
    /// along where it looks.
    pub(super) fn player_infested(&mut self) {
        if let Some(p) = self.player.as_ref() {
            let at = [
                p.position.x,
                p.position.y + f64::from(p.dimensions.height) / 2.0,
                p.position.z,
            ];
            let look = super::view_vector(self.yaw, self.pitch);
            self.infested_hurt(at, look, None);
        }
    }
    /// A living mob's own random (`Entity.random`), else the world's.
    fn bearer_random(&mut self, bearer: Option<u32>) -> &mut LegacyRandom {
        match bearer.and_then(|id| self.mobs.iter().position(|m| m.id == id)) {
            Some(i) => &mut self.mobs[i].random,
            None => &mut self.world.random,
        }
    }
    /// `InfestedMobEffect.onMobHurt`: a 10% chance of one or two silverfish at
    /// `at` (half the bearer's height up), thrown along its look direction.
    /// The draws are the bearer's (`mob.getRandom()`): `bearer` is the mob,
    /// none for the player (the world's random).
    pub(super) fn infested_hurt(&mut self, at: [f64; 3], look: [f64; 3], bearer: Option<u32>) {
        if self.bearer_random(bearer).next_float() > 0.1 {
            return;
        }
        let count = 1 + self.bearer_random(bearer).next_int_bound(2).unwrap_or(0);
        for _ in 0..count {
            // Mth.randomBetween(-PI / 2, PI / 2), then the look direction
            // scaled by (0.3, 0.45, 0.3) and turned about the y axis.
            let half = std::f32::consts::FRAC_PI_2;
            let angle = self.bearer_random(bearer).next_float() * (2.0 * half) - half;
            let (sin, cos) = angle.sin_cos();
            let (x, z) = (look[0] * 0.3, look[2] * 0.3);
            let velocity = Vec3::new(
                f64::from(x as f32 * cos + z as f32 * sin),
                look[1] * 0.3 * 1.5,
                f64::from(-x as f32 * sin + z as f32 * cos),
            );
            if let Some(id) = self.spawn_mob_for("minecraft:silverfish", at, false, true)
                && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
            {
                m.velocity = velocity;
            }
            self.audio
                .play("minecraft:entity.silverfish.hurt", Some(at), 1.0, 1.0);
        }
    }
    /// A splash potion, lingering potion or experience bottle in flight (the
    /// player's throws and dispensers both end up here).
    pub(super) fn launch_thrown(
        &mut self,
        stack: ItemStack,
        position: Vec3,
        velocity: Vec3,
        owner: Option<Text>,
    ) {
        self.potions.thrown.push(ThrownPotion {
            id: 0,
            stack,
            previous: position,
            position,
            velocity,
            age: 0,
            owner,
        });
    }
}
/// What a player's body was last given for its effects and enchantments:
/// a guest keeps their own while the host's are in `PotionState`.
#[derive(Default)]
pub(super) struct BodyPotions {
    applied: Vec<(&'static str, &'static str, f64, &'static str)>,
    protection_for: Option<Vec<Option<ItemStack>>>,
    enchant_modifiers: Vec<(String, String, f64, String)>,
}
/// Effects, thrown potions and the attribute modifiers last applied.
#[derive(Default)]
pub(super) struct PotionState {
    pub(super) effects: PlayerEffects,
    thrown: Vec<ThrownPotion>,
    /// The area effect clouds a server tracks (game/remote_projectiles.rs).
    pub(super) clouds: Vec<super::remote_projectiles::RemoteCloud>,
    applied: Vec<(&'static str, &'static str, f64, &'static str)>,
    /// Armor the protection table was computed for.
    protection_for: Option<Vec<Option<ItemStack>>>,

    /// Enchantment attribute modifiers on the player: (attribute, id, amount, operation).
    pub(super) enchant_modifiers: Vec<(String, String, f64, String)>,
}

impl PotionState {
    /// Trade the player's own state (effects and what their body was given)
    /// with a guest's, around `as_guest`. Potions in flight stay.
    pub(super) fn swap_player(&mut self, effects: &mut PlayerEffects, body: &mut BodyPotions) {
        std::mem::swap(&mut self.effects, effects);
        std::mem::swap(&mut self.applied, &mut body.applied);
        std::mem::swap(&mut self.protection_for, &mut body.protection_for);
        std::mem::swap(&mut self.enchant_modifiers, &mut body.enchant_modifiers);
    }
    /// Thrown potions in flight as (entity type, position, motion), for the
    /// dispenser parity fixture.
    pub(super) fn thrown_entities(&self) -> Vec<(&'static str, Vec3, Vec3)> {
        self.thrown
            .iter()
            .map(|t| (thrown_kind(&t.stack), t.position, t.velocity))
            .collect()
    }
    pub(super) fn clear_thrown(&mut self) {
        self.thrown.clear();
    }
    /// Their entity ids, for a server to number them.
    pub(super) fn thrown_ids(&mut self) -> impl Iterator<Item = &mut u32> {
        self.thrown.iter_mut().map(|t| &mut t.id)
    }
    /// The numbered ones as a server's clients track them: (id, entity
    /// type, item, position, motion).
    pub(super) fn thrown_tracked(
        &self,
    ) -> impl Iterator<Item = (u32, &'static str, &ItemStack, Vec3, Vec3)> {
        self.thrown.iter().filter(|t| t.id != 0).map(|t| {
            (
                t.id,
                thrown_kind(&t.stack),
                &t.stack,
                t.position,
                t.velocity,
            )
        })
    }
    /// `handleAddEntity` for a thrown potion or bottle on a server's client:
    /// drawn where the server says (its item comes in its entity data).
    pub(super) fn add_remote_thrown(&mut self, id: u32, kind: &str, at: Vec3, velocity: Vec3) {
        self.thrown.retain(|t| t.id != id);
        let item = match kind {
            "minecraft:lingering_potion" | "minecraft:experience_bottle" => kind,
            _ => "minecraft:splash_potion",
        };
        self.thrown.push(ThrownPotion {
            id,
            stack: ItemStack::new(item, 1),
            previous: at,
            position: at,
            velocity,
            age: 0,
            owner: None,
        });
    }
    /// ThrowableItemProjectile.DATA_ITEM_STACK: the potion it shows.
    pub(super) fn remote_thrown_item(&mut self, id: u32, stack: ItemStack) {
        if let Some(t) = self.thrown.iter_mut().find(|t| t.id == id) {
            t.stack = stack;
        }
    }
    /// A server's move: thrown potions have no interpolation handler, so
    /// `moveOrInterpolateTo` puts them there at once. False when `id` isn't one.
    pub(super) fn move_remote_thrown(&mut self, id: u32, at: Vec3) -> bool {
        let Some(t) = self.thrown.iter_mut().find(|t| t.id == id) else {
            return false;
        };
        t.position = at;
        true
    }
    /// `handleSetEntityMotion` for a thrown potion. False when `id` isn't one.
    pub(super) fn remote_thrown_motion(&mut self, id: u32, velocity: Vec3) -> bool {
        let Some(t) = self.thrown.iter_mut().find(|t| t.id == id) else {
            return false;
        };
        t.velocity = velocity;
        true
    }
    pub(super) fn remove_remote_thrown(&mut self, id: u32) {
        self.thrown.retain(|t| t.id != id);
    }
    pub(super) fn has_thrown(&self, id: u32) -> bool {
        self.thrown.iter().any(|t| t.id == id)
    }
}

/// The entity type of a thrown potion or bottle.
fn thrown_kind(stack: &ItemStack) -> &'static str {
    match &*stack.item {
        "minecraft:lingering_potion" => "minecraft:lingering_potion",
        "minecraft:experience_bottle" => "minecraft:experience_bottle",
        _ => "minecraft:splash_potion",
    }
}

impl Game<'_> {
    /// Right-click with a potion, splash/lingering potion or glass bottle.
    pub(super) fn use_potion_item(&mut self, stack: &ItemStack) -> horizon_client::Result<bool> {
        match &*stack.item {
            "minecraft:potion" | "minecraft:ominous_bottle" => {
                if self.using.is_none() {
                    self.using = Some((self.held_slot(), 0));
                }
                Ok(true)
            }
            "minecraft:splash_potion"
            | "minecraft:lingering_potion"
            | "minecraft:experience_bottle" => {
                let Some(eye) = self.eye_position() else {
                    return Ok(true);
                };
                let pitch = 0.4 / (self.world.random.next_float() * 0.4 + 0.8);
                let (sound, power) = match &*stack.item {
                    "minecraft:splash_potion" => ("minecraft:entity.splash_potion.throw", 0.5),
                    "minecraft:experience_bottle" => {
                        ("minecraft:entity.experience_bottle.throw", 0.7)
                    }
                    _ => ("minecraft:entity.lingering_potion.throw", 0.5),
                };
                self.audio.play(sound, Some(eye), 0.5, pitch);
                // Projectile.shootFromRotation(xRot, yRot, -20, 0.5, 1.0).
                let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
                let dir = [
                    -yaw.sin() * pitch.cos(),
                    -(self.pitch - 20.0).to_radians().sin(),
                    yaw.cos() * pitch.cos(),
                ];
                let l = dir
                    .iter()
                    .map(|c| f64::from(*c).powi(2))
                    .sum::<f64>()
                    .sqrt()
                    .max(1e-9);
                // getMovementToShoot: normalize, add triangle(0, 0.0172275 * uncertainty), scale.
                let r = &mut self.world.random;
                let mut jitter = || r.triangle(0.0, 0.0172275);
                let v = Vec3::new(
                    (f64::from(dir[0]) / l + jitter()) * power,
                    (f64::from(dir[1]) / l + jitter()) * power,
                    (f64::from(dir[2]) / l + jitter()) * power,
                );
                let pv = self.known_player_movement();
                let on_ground = self.player.as_ref().is_some_and(|p| p.ground.on_ground);
                let v = Vec3::new(
                    v.x + pv.x,
                    v.y + if on_ground { 0.0 } else { pv.y },
                    v.z + pv.z,
                );
                self.launch_thrown(
                    ItemStack {
                        count: 1,
                        ..stack.clone()
                    },
                    Vec3::new(eye[0], eye[1] - 0.1, eye[2]),
                    v,
                    Some(self.player_display_text()),
                );
                self.swing_arm();
                self.consume_held();
                Ok(true)
            }
            "minecraft:glass_bottle" => {
                let Some(hit) = self.pick_fluid() else {
                    return Ok(false);
                };
                let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
                if self.res.facts().get(state).fluid != horizon_client::blocks::Fluid::Water {
                    return Ok(false);
                }
                let at = [
                    f64::from(hit.pos[0]) + 0.5,
                    f64::from(hit.pos[1]) + 0.5,
                    f64::from(hit.pos[2]) + 0.5,
                ];
                self.audio
                    .play("minecraft:item.bottle.fill", Some(at), 1.0, 1.0);
                let water = ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water");
                self.fill_held(water);
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    /// `ItemUtils.createFilledResult`: replace one of the held stack.
    fn fill_held(&mut self, filled: ItemStack) {
        if self.creative {
            let _ = self.give(filled);
            return;
        }
        self.consume_held();
        if self.held().is_none() {
            *self.held_mut() = Some(filled);
        } else if let Some(rest) = self.give(filled)
            && let Some(eye) = self.eye_position()
        {
            // `player.drop(stack, false)`.
            self.throw_from_eyes(rest, eye, false);
        }
    }
    /// Drinking a potion while right-click is held; true when handled.
    pub(super) fn tick_drinking(&mut self, slot: usize, ticks: u32, stack: &ItemStack) -> bool {
        let ominous = &*stack.item == "minecraft:ominous_bottle";
        if &*stack.item != "minecraft:potion" && !ominous {
            return false;
        }
        let ticks = ticks + 1;
        let remaining = 32u32.saturating_sub(ticks);
        let eye = self.eye_position().unwrap_or_default();
        // Consumable.emitParticlesAndSounds: the drink sound every 4 ticks near the end.
        if remaining <= 25 && remaining.is_multiple_of(4) {
            let pitch = self.world.random.next_float() * 0.1 + 0.9;
            self.audio
                .play("minecraft:entity.generic.drink", Some(eye), 0.5, pitch);
        }
        if ticks < 32 {
            self.using = Some((slot, ticks));
            return true;
        }
        self.using = None;
        self.send_use_item_complete();
        self.post_entity_event("minecraft:drink", horizon_client::mobs::Who::Player);
        if ominous {
            self.drink_ominous_bottle(slot, stack);
            return true;
        }
        let (lasting, now) = PlayerEffects::potion_effects(stack.potion(), 1.0);
        for e in lasting {
            self.add_player_effect(e);
        }
        for a in now {
            self.apply_effect_action(a);
        }
        if !self.creative {
            if let Some(s) = self.stack_in_mut(slot).as_mut() {
                s.count -= 1;
                if s.count == 0 {
                    *self.stack_in_mut(slot) = None;
                }
            }
            let bottle = ItemStack::new("minecraft:glass_bottle", 1);
            if self.stack_in(slot).is_none() {
                *self.stack_in_mut(slot) = Some(bottle);
            } else if let Some(rest) = self.give(bottle) {
                self.spawn_item(rest, eye, [0.0, 0.1, 0.0], 40);
            }
        }
        true
    }
    pub(super) fn apply_effect_action(&mut self, action: EffectAction) {
        let creative = self.creative;
        let rules = self.world.health_rules;
        let Some(p) = self.player.as_mut() else {
            return;
        };
        match action {
            EffectAction::Heal(amount) => {
                let max = p.attribute("minecraft:max_health").unwrap_or(20.0) as f32;
                let h = p.health.health();
                if h > 0.0 {
                    let _ = p.set_health((h + amount).min(max), false);
                }
            }
            EffectAction::Hurt(source, amount) => {
                if creative {
                    return;
                }
                if let Some(id) = self.motion.health.damage.id(source) {
                    let _ = p.hurt(&self.motion.health, rules, id, amount);
                }
            }
            EffectAction::Poison => {
                if !creative
                    && p.health.health() > 1.0
                    && let Some(id) = self.motion.health.damage.id("minecraft:magic")
                {
                    let _ = p.hurt(&self.motion.health, rules, id, 1.0);
                }
            }
            EffectAction::Exhaust(amount) => {
                p.cause_food_exhaustion(amount);
            }
            EffectAction::Eat(nutrition, saturation) => {
                let food = &mut p.health.food;
                food.food = (food.food + nutrition).clamp(0, 20);
                food.saturation = (food.saturation + nutrition as f32 * saturation * 2.0)
                    .clamp(0.0, food.food as f32);
            }
        }
    }
    /// The local player's effects (`tick_mob_effects`), then the potions
    /// in flight, which a server without a local player still has. A frozen
    /// tick (`/tick freeze`) leaves the potions where they are
    /// (`TickRateManager.isEntityFrozen`); players' effects still run.
    pub(super) fn tick_effects(&mut self, running: bool) {
        self.tick_mob_effects();
        if running {
            self.tick_thrown_potions();
            self.tick_potion_clouds();
        }
    }
    /// `LivingEntity.tickEffects` (server side) for `self.player`, the local
    /// player or a guest acting (`as_guest`), then its attribute modifiers
    /// and status flags.
    pub(super) fn tick_mob_effects(&mut self) {
        let Some(player) = self.player.as_ref() else {
            return;
        };
        if player.health.dead_or_dying() {
            // LivingEntity.die → removeAllEffects. Vanilla runs the effects
            // that act on death (`onMobRemoved`) when the dead player is
            // removed, up to 20 ticks later; here it is as the effects go.
            let w = f64::from(player.dimensions.width) / 2.0;
            let bounds = Aabb::new(
                Vec3::new(
                    player.position.x - w,
                    player.position.y,
                    player.position.z - w,
                ),
                Vec3::new(
                    player.position.x + w,
                    player.position.y + f64::from(player.dimensions.height),
                    player.position.z + w,
                ),
            );
            let ids: Vec<&'static str> = self.potions.effects.list.iter().map(|e| e.id).collect();
            self.potions.effects.clear();
            if !ids.is_empty() {
                self.effects_on_death(&ids, bounds, true, None, None);
            }
        }
        // The omens act from inside the effect tick (BadOmenMobEffect,
        // RaidOmenMobEffect).
        let absorption_empty = self
            .player
            .as_ref()
            .is_some_and(|p| p.health.absorption() <= 0.0);
        let mut effects = std::mem::take(&mut self.potions.effects);
        let (actions, raid_omen_added) =
            effects.tick_with(&mut |e| self.omen_tick(e), absorption_empty);
        self.potions.effects = effects;
        if raid_omen_added {
            self.raid_omen_added();
        }
        for action in actions {
            self.apply_effect_action(action);
        }
        self.turtle_helmet_tick();
        self.apply_effect_status();
    }
    /// `Player.tick` -> `turtleHelmetTick`: with a turtle helmet on the head
    /// and the eyes out of the water, Water Breathing for 200 ticks (no
    /// particles, an icon).
    fn turtle_helmet_tick(&mut self) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let eye_y = p.position.y + f64::from(p.dimensions.eye_height);
        if !p.health.dead_or_dying()
            && self.armor[0]
                .as_ref()
                .is_some_and(|s| &*s.item == "minecraft:turtle_helmet")
            && !self.eye_in_water(p.position.x, eye_y, p.position.z)
        {
            let mut effect = EffectInstance::new("water_breathing", 200, 0);
            effect.visible = false;
            self.potions.effects.add(effect);
        }
    }
    /// A client's own effects (`LivingEntity.tickEffects` on the client,
    /// `MobEffectInstance.tickClient`): they count down and blend, and the
    /// server removes them. Their modifiers and flags move the predicted
    /// player.
    pub(super) fn tick_client_effects(&mut self) {
        client_tick_effects(&mut self.potions.effects, &mut self.sculk.darkness);
        self.apply_effect_status();
    }
    /// Add an active effect to the player and update attributes / absorption.
    pub(super) fn add_player_effect(&mut self, effect: EffectInstance) -> bool {
        let id = effect.id;
        let amp = effect.amplifier;
        let changed = self.potions.effects.add(effect);
        self.apply_effect_status();
        if id == "absorption"
            && let Some(p) = self.player.as_mut()
        {
            let target = 4.0 * (amp + 1) as f32;
            let current = p.health.absorption();
            let _ = p.set_health(current.max(target), true);
        }
        changed
    }
    /// The effects' attribute modifiers, armor protection and status flags
    /// on `self.player`.
    pub(super) fn apply_effect_status(&mut self) {
        let modifiers = self.potions.effects.modifiers();
        if modifiers != self.potions.applied
            && let Some(p) = self.player.as_mut()
        {
            for (attr, id, _, op) in &self.potions.applied {
                let _ = p.set_modifier(attr, id, 0.0, op, false);
            }
            for (attr, id, amount, op) in &modifiers {
                let _ = p.set_modifier(attr, id, *amount, op, true);
            }
            self.potions.applied = modifiers;
        }
        if let Some(p) = self.player.as_mut()
            && self.potions.effects.amplifier("absorption").is_none()
            && p.health.absorption() > 0.0
        {
            let _ = p.set_health(0.0, true);
        }
        // EnchantmentHelper.getDamageProtection per damage type, recomputed on armor change.
        let armor: Vec<Option<ItemStack>> = self.armor.to_vec();
        if self.potions.protection_for.as_ref() != Some(&armor) {
            let mut table = Vec::new();
            let e = &self.stations().enchantments;
            let mut random = horizon_core::random::LegacyRandom::new(0);
            for (_, def) in self.motion.health.damage.entries() {
                let ctx = horizon_client::enchant::EffectContext {
                    items: Some(self.items()),
                    damage_type: Some(&def.name),
                    this: Some("minecraft:player"),
                    ..Default::default()
                };
                let total: f32 = armor
                    .iter()
                    .flatten()
                    .map(|s| {
                        e.modify_value(
                            Some(s),
                            "minecraft:damage_protection",
                            &ctx,
                            &mut random,
                            0.0,
                        )
                    })
                    .sum();
                if total > 0.0 {
                    table.push((def.name.clone(), total));
                }
            }
            self.potions.protection_for = Some(armor);
            if let Some(p) = self.player.as_mut() {
                p.status.protection = table;
            }
        }
        let mut status = self.potions.effects.status();
        if let Some(p) = self.player.as_mut() {
            status.protection = std::mem::take(&mut p.status.protection);
            p.status = status;
        }
    }
    /// `ThrowableProjectile.tick` for the potions in flight: the motion, then
    /// the first block or mob the step meets (`ProjectileUtil.getHitResultOnMoveVector`),
    /// where the potion splashes (`Projectile.onHit`).
    fn tick_thrown_potions(&mut self) {
        let mut thrown = std::mem::take(&mut self.potions.thrown);
        let mut landed = Vec::new();
        thrown.retain_mut(|t| {
            t.age += 1;
            t.previous = t.position;
            self.thrown_potion_motion(t);
            let next = Vec3::new(
                t.position.x + t.velocity.x,
                t.position.y + t.velocity.y,
                t.position.z + t.velocity.z,
            );
            // Hit a block along the step (sampled), a mob, or fell out of the world.
            let steps = 4;
            let mut hit = None;
            for i in 1..=steps {
                let f = f64::from(i) / f64::from(steps);
                let p = Vec3::new(
                    t.position.x + t.velocity.x * f,
                    t.position.y + t.velocity.y * f,
                    t.position.z + t.velocity.z * f,
                );
                let state =
                    self.world
                        .block(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
                let mob = self.mobs.iter().find(|m| {
                    m.alive()
                        && (m.position.x - p.x).abs() < m.kind.width / 2.0 + 0.125
                        && (m.position.z - p.z).abs() < m.kind.width / 2.0 + 0.125
                        && p.y >= m.position.y
                        && p.y <= m.position.y + m.kind.height
                });
                if self.res.facts().get(state).full_collision || mob.is_some() {
                    // A block hit has the face it came in by
                    // (`onHitBlock`); a mob hit has none.
                    let block = mob
                        .is_none()
                        .then(|| thrown_block_hit(t.position, p, t.velocity));
                    hit = Some((p, mob.map(|m| m.position), block));
                    break;
                }
            }
            if let Some((p, mob, block)) = hit {
                landed.push((t.stack.clone(), p, mob, block, t.age, t.owner.clone()));
                return false;
            }
            t.position = next;
            t.age < 1200 && t.position.y > -128.0
        });
        self.potions.thrown = thrown;
        for (stack, at, mob, block, age, owner) in landed {
            self.splash(&stack, at, mob, block, age, owner);
        }
    }
    /// `AreaEffectCloud.tick` for the lingering potions' clouds: the client
    /// tick's particles, then the server tick.
    fn tick_potion_clouds(&mut self) {
        if self.breath_clouds.iter().all(|c| c.potion.is_none()) {
            return;
        }
        let players: Vec<FightPlayer> = self
            .fight_players()
            .into_iter()
            .filter(|p| p.alive)
            .collect();
        let mut clouds = std::mem::take(&mut self.breath_clouds);
        clouds.retain_mut(|c| c.potion.is_none() || self.tick_potion_cloud(c, &players));
        self.breath_clouds.append(&mut clouds);
    }
    /// `AreaEffectCloud.serverTick` for a lingering potion's cloud: after its
    /// wait it shrinks, and every 5 ticks it affects the living entities in
    /// its box and radius that were not affected in the last 20 ticks, each
    /// taking some of its radius. False once it is gone.
    fn tick_potion_cloud(&mut self, c: &mut Breath, players: &[FightPlayer]) -> bool {
        c.age += 1;
        let Some(potion) = c.potion.as_mut() else {
            return true;
        };
        if c.age - LINGERING_WAIT >= c.duration {
            return false;
        }
        let waiting = c.age < LINGERING_WAIT;
        cloud_client_tick(
            &mut self.particles,
            c.at,
            c.radius,
            waiting,
            &potion.particle(),
        );
        if waiting {
            return true;
        }
        // setRadiusPerTick(-radius / duration).
        c.radius += -3.0 / 600.0;
        if c.radius < 0.5 {
            return false;
        }
        if c.age % 5 != 0 {
            return true;
        }
        potion.victims.retain(|(_, until)| c.age < *until);
        let r = f64::from(c.radius);
        let area = Aabb::new(
            Vec3::new(c.at[0] - r, c.at[1], c.at[2] - r),
            Vec3::new(c.at[0] + r, c.at[1] + 0.5, c.at[2] + r),
        );
        let mut inside = Vec::new();
        for p in players.iter().filter(|p| p.bounds.intersects(area)) {
            inside.push((Victim::Player(p.who), p.position.x, p.position.z));
        }
        let effects = potion.effects;
        let tags = &self.stations().enchantments.entity_tags;
        // Only mobs that at least one of the cloud's effects can affect are
        // taken (`allEffects.stream().anyMatch(entity::canBeAffected)`), so
        // the others neither become victims nor use up its radius.
        for m in self.mobs.iter().filter(|m| {
            m.alive()
                && m.bounds().intersects(area)
                && effects
                    .iter()
                    .any(|(id, ..)| mob_can_be_affected(tags, &m.kind.id, id))
        }) {
            inside.push((Victim::Mob(m.id), m.position.x, m.position.z));
        }
        for (victim, x, z) in inside {
            let (dx, dz) = (x - c.at[0], z - c.at[2]);
            if potion.victims.iter().any(|(v, _)| *v == victim)
                || dx * dx + dz * dz > f64::from(c.radius * c.radius)
            {
                continue;
            }
            potion.victims.push((victim, c.age + REAPPLICATION_DELAY));
            self.cloud_affect(victim, effects, c.at, &potion.source);
            // setRadiusOnUse(-0.5).
            c.radius -= 0.5;
            if c.radius < 0.5 {
                return false;
            }
        }
        true
    }
    /// What a cloud puts on one entity: instant effects at half strength
    /// (`applyInstantaneousEffect` with 0.5), the others for their duration
    /// at the lingering item's 0.25 scale (`PotionContents.forEachEffect`).
    fn cloud_affect(
        &mut self,
        victim: Victim,
        effects: &'static [(&'static str, i32, i32)],
        at: [f64; 3],
        source: &Text,
    ) {
        let scaled = |duration: i32| ((f64::from(duration) * 0.25) as i32).max(1);
        match victim {
            Victim::Player(who) => self.as_fight_player(who, |g| {
                for (id, duration, amp) in effects {
                    let Some(e) = brewing::effect(id) else {
                        continue;
                    };
                    if e.instant {
                        if let Some(a) = horizon_client::effects::instant(id, *amp, 0.5) {
                            g.apply_indirect_effect_action(a, source);
                        }
                    } else {
                        g.add_player_effect(EffectInstance::new(e.id, scaled(*duration), *amp));
                    }
                }
            }),
            Victim::Mob(mob) => {
                let Some(i) = self.mobs.iter().position(|m| m.id == mob) else {
                    return;
                };
                // Undead invert healing and harming.
                let tags = &world_stations!(self).enchantments.entity_tags;
                let inverted = tags
                    .get("minecraft:inverted_healing_and_harm")
                    .is_some_and(|t| t.contains(&self.mobs[i].kind.id));
                for (id, duration, amp) in effects {
                    let m = &mut self.mobs[i];
                    match *id {
                        "instant_damage" | "instant_health" => {
                            let amount =
                                |base: i32| (0.5 * f64::from(base << amp) + 0.5) as i32 as f32;
                            if (*id == "instant_damage") != inverted {
                                let harm = m.potion_harm(amount(6));
                                m.hurt(harm, Some(at), &mut self.world.random);
                            } else {
                                m.health = (m.health + amount(4)).min(m.kind.max_health);
                            }
                        }
                        _ => {
                            // `LivingEntity.addEffect` refuses what the mob
                            // cannot be affected by.
                            if mob_can_be_affected(tags, &m.kind.id, id)
                                && let Some(name) = mob_effect_id(id)
                            {
                                m.add_effect(name, scaled(*duration), *amp);
                            }
                        }
                    }
                }
            }
        }
    }
    /// `ThrowableProjectile.tick`'s `applyGravity` and `applyInertia`, which
    /// come before the move: gravity 0.05 (0.07 for an experience bottle),
    /// then drag 0.99 (0.8 in water).
    fn thrown_potion_motion(&self, t: &mut ThrownPotion) {
        let gravity = if &*t.stack.item == "minecraft:experience_bottle" {
            0.07
        } else {
            0.05
        };
        let at = t.position;
        let water = self
            .res
            .facts()
            .get(self.world.block(
                at.x.floor() as i32,
                at.y.floor() as i32,
                at.z.floor() as i32,
            ))
            .fluid
            == horizon_client::blocks::Fluid::Water;
        let inertia = if water { 0.8 } else { 0.99 };
        t.velocity = Vec3::new(
            t.velocity.x * inertia,
            (t.velocity.y - gravity) * inertia,
            t.velocity.z * inertia,
        );
    }
    /// `ThrowableProjectile.tick` on a server's client: it falls and moves
    /// by itself between the server's updates and stops at a block; only
    /// the server breaks it.
    pub(super) fn step_remote_thrown(&mut self) {
        let mut thrown = std::mem::take(&mut self.potions.thrown);
        for t in &mut thrown {
            t.previous = t.position;
            t.age += 1;
            self.thrown_potion_motion(t);
            let next = Vec3::new(
                t.position.x + t.velocity.x,
                t.position.y + t.velocity.y,
                t.position.z + t.velocity.z,
            );
            let state = self.world.block(
                next.x.floor() as i32,
                next.y.floor() as i32,
                next.z.floor() as i32,
            );
            if self.res.facts().get(state).full_collision {
                t.velocity = Vec3::ZERO;
            } else {
                t.position = next;
            }
        }
        self.potions.thrown = thrown;
    }
    /// `ThrownItemRenderer` for thrown splash and lingering potions and
    /// experience bottles: the item (GROUND) facing the camera at scale 1,
    /// lit at the light probe (the eye, 0.85 of the 0.25 height).
    pub(super) fn thrown_potion_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        let orientation = glam::Mat4::from_quat(self.camera_orientation());
        for t in &self.potions.thrown {
            let previous = glam::DVec3::new(t.previous.x, t.previous.y, t.previous.z);
            let position = glam::DVec3::new(t.position.x, t.position.y, t.position.z);
            // ThrowableProjectile.shouldRenderAtSqrDistance: hidden for its
            // first two ticks within 3.5 blocks (not in the thrower's face).
            if t.age < 2 && position.distance_squared(camera) < 12.25 {
                continue;
            }
            if !self.standing_entity_in_view(
                camera,
                super::entity_culling::at(t.previous),
                super::entity_culling::at(t.position),
                alpha,
                0.25,
                0.25,
                64.0,
                0.0,
            ) {
                continue;
            }
            let p = previous.lerp(position, alpha);
            let owner = owner_at(p.to_array(), 0.0, &t.stack.item);
            let state = self.resolve_item(&t.stack, DisplayContext::Ground, Some(&owner), 0);
            if state.is_empty() {
                continue;
            }
            let pose = glam::Mat4::from_translation((p - camera).as_vec3()) * orientation;
            let light = self.world.light(
                p.x.floor() as i32,
                (p.y + 0.2125).floor() as i32,
                p.z.floor() as i32,
            );
            self.submit_item_world(&state, pose, vertex_light(light), out);
        }
    }
    /// `HealOrHarmMobEffect.applyInstantaneousEffect` with a source (a
    /// potion or a cloud): indirectMagic, not plain magic.
    fn apply_indirect_effect_action(&mut self, a: EffectAction, source: &Text) {
        let a = match a {
            EffectAction::Hurt("minecraft:magic", amount) => {
                self.note_direct_entity("minecraft:indirect_magic", source.clone());
                EffectAction::Hurt("minecraft:indirect_magic", amount)
            }
            a => a,
        };
        self.apply_effect_action(a);
    }
    /// A splash potion's effects on the player in place, by their distance.
    fn splash_on_player(
        &mut self,
        effects: &'static [(&'static str, i32, i32)],
        at: Vec3,
        age: u32,
        source: &Text,
    ) {
        if let Some(p) = self.player.as_ref() {
            let w = f64::from(p.dimensions.width) / 2.0;
            let bounds = Aabb::new(
                Vec3::new(p.position.x - w, p.position.y, p.position.z - w),
                Vec3::new(
                    p.position.x + w,
                    p.position.y + f64::from(p.dimensions.height),
                    p.position.z + w,
                ),
            );
            if let Some(d2) = splash_distance_sqr(at, age, bounds) {
                let intensity = 1.0 - d2.sqrt() / 4.0;
                for (id, duration, amp) in effects {
                    let instant = brewing::effect(id).is_some_and(|e| e.instant);
                    if instant {
                        if let Some(a) = horizon_client::effects::instant(id, *amp, intensity) {
                            self.apply_indirect_effect_action(a, source);
                        }
                    } else {
                        let d = (intensity * f64::from(*duration) + 0.5) as i32;
                        if d > 20
                            && let Some(e) = brewing::effect(id)
                        {
                            self.add_player_effect(EffectInstance::new(e.id, d, *amp));
                        }
                    }
                }
            }
        }
    }
    /// `AbstractThrownPotion.onHit` with `ThrownSplashPotion.onHitAsPotion`, or
    /// `ThrownLingeringPotion.onHitAsPotion` for a lingering potion, read from
    /// the stack's `potion_contents`; `mob` is where it hit a mob, `hit` the
    /// block and face it hit, `age` its age.
    fn splash(
        &mut self,
        stack: &ItemStack,
        at: Vec3,
        mob: Option<Vec3>,
        hit: Option<([i32; 3], Direction)>,
        age: u32,
        owner: Option<Text>,
    ) {
        let block = [at.x, at.y, at.z].map(|v| v.floor() as i32);
        if &*stack.item == "minecraft:experience_bottle" {
            // ThrownExperienceBottle.onHit: the water-coloured splash (level
            // event 2002, its sound 1053) and 3 to 11 experience.
            self.level_event_at(2002, block, 0xFF38_5DC6u32 as i32);
            self.level_event_at(1053, block, 0);
            let r = &mut self.world.random;
            let value = 3 + r.next_int_bound(5).unwrap_or(0) + r.next_int_bound(5).unwrap_or(0);
            self.spawn_orbs([at.x, at.y, at.z], value as u32);
            return;
        }
        // Projectile.onHit calls onHitBlock first, then the potion's
        // affectEntitiesAround; both read the stack's `potion_contents`.
        if let Some((cell, face)) = hit {
            self.potion_hit_block(stack, cell, face);
        }
        self.affect_entities_around(stack, at, owner.is_some());
        let lingering = &*stack.item == "minecraft:lingering_potion";
        // AbstractThrownPotion.onHit: level event 2007 and sound 1054 for a
        // potion with instant effects, 2002 and 1053 otherwise.
        let potion = stack.potion().and_then(brewing::potion);
        let instant = potion.is_some_and(|d| {
            d.effects
                .iter()
                .any(|(id, ..)| brewing::effect(id).is_some_and(|e| e.instant))
        });
        let color = brewing::potion_color(stack.potion());
        self.level_event_at(if instant { 2007 } else { 2002 }, block, color as i32);
        self.level_event_at(if instant { 1054 } else { 1053 }, block, 0);
        // AbstractThrownPotion.affectEntitiesAround: a water potion (the only
        // #rehydrates_axolotls) refills the air of the axolotls in the
        // projectile's box inflated by 4, 2 and 4 (Axolotl.rehydrate, 1800).
        if stack.potion() == Some("minecraft:water") {
            let (lo, hi) = (
                [at.x - 0.125 - 4.0, at.y - 2.0, at.z - 0.125 - 4.0],
                [at.x + 0.125 + 4.0, at.y + 0.25 + 2.0, at.z + 0.125 + 4.0],
            );
            for m in self
                .mobs
                .iter_mut()
                .filter(|m| m.species() == horizon_client::mobs::Species::Axolotl && !m.removed)
            {
                let (w, h, _) = m.dims();
                let half = w / 2.0;
                let (x, y, z) = (m.position.x, m.position.y, m.position.z);
                if x - half < hi[0]
                    && x + half > lo[0]
                    && y < hi[1]
                    && y + h > lo[1]
                    && z - half < hi[2]
                    && z + half > lo[2]
                {
                    m.air = (m.air + 1800).min(horizon_client::mobs::axolotl::MAX_AIR);
                }
            }
        }
        let Some(def) = potion else {
            return;
        };
        if lingering {
            // The cloud sits at the mob it hit (its feet), else the impact;
            // radius 3, wait 10, 600 ticks, shrinking to nothing, and a
            // potion with no effects leaves none.
            if !def.effects.is_empty() {
                let at = mob.unwrap_or(at);
                self.breath_clouds.push(Breath {
                    id: 0,
                    at: [at.x, at.y, at.z],
                    radius: 3.0,
                    age: 0,
                    duration: 600,
                    from_dragon: false,
                    potion: Some(PotionCloud {
                        effects: def.effects,
                        color,
                        victims: Vec::new(),
                        source: owner.unwrap_or_else(|| {
                            Text::translate("entity.minecraft.area_effect_cloud", &[])
                        }),
                    }),
                });
            }
            return;
        }
        // Every player in range (`ThrownSplashPotion.applySplash` over
        // `LivingEntity`s, the players among them).
        // Instant damage is indirectMagic(potion, owner), which the death
        // message names by the owner, else by the potion.
        let source =
            owner.unwrap_or_else(|| Text::translate("entity.minecraft.splash_potion", &[]));
        self.for_each_player(|g| g.splash_on_player(def.effects, at, age, &source));
        // Mobs take the instant effects (undead invert healing and harming).
        let undead = &self.stations().enchantments.entity_tags;
        let mut hits = Vec::new();
        for (i, m) in self.mobs.iter().enumerate() {
            if m.alive()
                && let Some(d2) = splash_distance_sqr(at, age, m.bounds())
            {
                let inverted = undead
                    .get("minecraft:inverted_healing_and_harm")
                    .is_some_and(|t| t.contains(&m.kind.id));
                hits.push((i, 1.0 - d2.sqrt() / 4.0, inverted));
            }
        }
        for (i, intensity, inverted) in hits {
            for (id, duration, amp) in def.effects {
                let harm = match *id {
                    "instant_damage" => !inverted,
                    "instant_health" => inverted,
                    _ => {
                        // `Potion.applySplash`: timed effects last in
                        // proportion to how close the mob was.
                        let d = (intensity * f64::from(*duration) + 0.5) as i32;
                        if d > 20
                            && let Some(name) = mob_effect_id(id)
                        {
                            self.mobs[i].add_effect(name, d, *amp);
                        }
                        continue;
                    }
                };
                let m = &mut self.mobs[i];
                if harm {
                    let harm = m.potion_harm((intensity * f64::from(6 << amp) + 0.5) as i32 as f32);
                    m.hurt(harm, Some([at.x, at.y, at.z]), &mut self.world.random);
                } else {
                    m.health = (m.health + (intensity * f64::from(4 << amp) + 0.5) as i32 as f32)
                        .min(m.kind.max_health);
                }
            }
        }
    }
    /// `Screen.showsActiveEffects`: the survival and creative inventories
    /// draw their own effect list when `EffectsInInventory.canSeeEffects`
    /// (32 or more pixels right of the screen).
    fn screen_shows_active_effects(&self, batch: &HudBatch) -> bool {
        let (ox, image_width) = match self.screen {
            Screen::Inventory
                if self.container.is_none() && self.craft_size == 2 && !self.mount_active() =>
            {
                (self.inventory_origin(batch).0, 176)
            }
            Screen::Creative => (
                Self::creative_origin(batch).0,
                horizon_client::creative::Creative::WIDTH as i32,
            ),
            _ => return false,
        };
        batch.width as i32 - (ox as i32 + image_width + 2) >= 32
    }
    /// `Hud.extractEffects`: icons top-right, beneficial row then harmful row;
    /// every screen but the inventories keeps them.
    pub(super) fn draw_effect_icons(&self, batch: &mut HudBatch) {
        if self.potions.effects.list.is_empty() || self.screen_shows_active_effects(batch) {
            return;
        }
        let icons: Vec<hud_overlays::EffectIcon> = self
            .potions
            .effects
            .list
            .iter()
            .map(hud_overlays::EffectIcon::from)
            .collect();
        let draws = hud_overlays::effects(batch.width as i32, false, &icons);
        hud_layout::paint(batch, &self.gui.atlas, &draws);
    }
    /// `EffectsInInventory`: panels to the right of survival/creative inventory.
    pub(super) fn draw_inventory_effects(
        &self,
        batch: &mut HudBatch,
        origin: (f32, f32),
        image_width: i32,
    ) {
        let panels = inventory_effects::panels(
            batch.width as i32,
            [origin.0 as i32, origin.1 as i32],
            image_width,
            &self.potions.effects.list,
            self.clock.rate(),
            &|key| self.pack.names.display_get(key),
            &|text| self.gui.atlas.font.width(text).ceil() as i32,
        );
        if let Some(lines) = inventory_effects::paint(
            batch,
            &self.gui.atlas,
            &panels,
            [self.cursor.0 as i32, self.cursor.1 as i32],
        ) {
            self.draw_tooltip_lines(batch, &lines, self.cursor.0, self.cursor.1);
        }
    }
    /// Effects saved in level.dat (`Player.active_effects`).
    pub(super) fn restore_effects(&mut self) {
        self.potions = PotionState::default();
        self.potions.effects = PlayerEffects::from_saved(&self.level.active_effects);
    }
    pub(super) fn effects_json(&self) -> serde_json::Value {
        serde_json::json!({
            "active": self.potions.effects.list.iter().map(|e| serde_json::json!({ "id": e.id, "duration": e.duration, "amplifier": e.amplifier })).collect::<Vec<_>>(),
            "thrown": self.potions.thrown.iter().map(|t| [t.position.x, t.position.y, t.position.z]).collect::<Vec<_>>(),
        })
    }
}

/// `LivingEntity.tickEffects` on a client: each `MobEffectInstance.tickClient`
/// counts its effect down, then ticks its `BlendState`. The darkness effect's
/// is the fade `darkness_lighting` and `darkness_fog` draw; the integrated
/// level's `tick_sculk_entities` is not run for a server's client (#3393).
fn client_tick_effects(effects: &mut PlayerEffects, darkness: &mut (f32, f32)) {
    for e in &mut effects.list {
        e.tick_client();
    }
    *darkness = super::sculk::darkness_blend_tick(effects, *darkness);
}

#[cfg(test)]
mod tests {
    use super::super::sculk::darkness_blend_added;
    use super::*;

    /// A server's client (multiplayer and the default single player) fades
    /// the darkness effect in over its 22 blend ticks, holds it, and fades
    /// it out over the last 22 (#3393).
    #[test]
    fn a_servers_client_blends_the_darkness_effect() {
        let mut effects = PlayerEffects::default();
        effects.add(EffectInstance::new("darkness", 60, 0));
        let mut darkness = (0.0, 0.0);
        client_tick_effects(&mut effects, &mut darkness);
        assert_eq!(darkness, (0.0, 1.0 / 22.0), "the first tick fades in");
        for _ in 1..22 {
            client_tick_effects(&mut effects, &mut darkness);
        }
        assert!((darkness.1 - 1.0).abs() < 1e-5, "{darkness:?}");
        // 38 ticks left: full until 22 remain, then the fade out.
        for _ in 0..16 {
            client_tick_effects(&mut effects, &mut darkness);
        }
        assert_eq!(effects.get("darkness").map(|e| e.duration), Some(22));
        assert!(
            darkness.1 < darkness.0,
            "fading out at 22 left: {darkness:?}"
        );
        for _ in 0..21 {
            client_tick_effects(&mut effects, &mut darkness);
        }
        assert!(darkness.1.abs() < 1e-5, "{darkness:?}");
    }

    /// `ClientPacketListener.handleUpdateMobEffect`: a new darkness fades in
    /// when the server blends it; one sent unblended (joining, changing
    /// dimension) is at full strength at once.
    #[test]
    fn a_new_darkness_starts_blended_or_at_once() {
        let e = EffectInstance::new("darkness", 260, 0);
        assert_eq!(darkness_blend_added(&e, true), (0.0, 0.0));
        assert_eq!(darkness_blend_added(&e, false), (1.0, 1.0));
        let ending = EffectInstance::new("darkness", 10, 0);
        assert_eq!(darkness_blend_added(&ending, false), (0.0, 0.0));
        let infinite = EffectInstance::new("darkness", -1, 0);
        assert_eq!(darkness_blend_added(&infinite, false), (1.0, 1.0));
    }
}
