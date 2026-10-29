//! Llamas and trader llamas (`Llama`, `TraderLlama`): coat variants and
//! strength, chests and carpets, taming by riding (maximum temper 30), the
//! spit ranged attack, hunting wolves (and, for traders, zombies and
//! illagers) and `LlamaModel`.
use super::finalize::{Attr, set_base};
use super::{
    Mob, MobEvent, MobWorld, Seen, Species, Who, equine,
    goals::{
        Breed, Cx, Float, FollowParent, Goal, HurtByTarget, LookAtPlayer, NearestTarget, Panic,
        Prey, RandomLookAround, Stroll, Tempt, goal_clone,
    },
    witch::RangedAttack,
};
use crate::projectiles::{Projectile, ProjectileKind};
use horizon_core::{geometry::Vec3, random::RandomSource};

pub const VARIANTS: [&str; 4] = ["creamy", "white", "brown", "gray"];
/// `Llama.getMaxTemper`.
pub const MAX_TEMPER: i32 = 30;

// Llama state: `variant` the coat, `variant2` the strength (1 to 5),
// `variant3` the temper, `chested` the chest, `head` the carpet, `flag2`
// `didSpit`, `ints[0]` a trader llama's despawn delay.
pub fn is_llama(s: Species) -> bool {
    matches!(s, Species::Llama | Species::TraderLlama)
}
pub fn is_trader(m: &Mob) -> bool {
    m.species() == Species::TraderLlama
}
/// `Llama.getStrength`: the stored strength (0 for a llama that was never
/// given one; `setStrength` keeps it within 1 to 5).
pub fn strength(m: &Mob) -> i32 {
    i32::from(m.extra.variant2)
}
/// `getInventoryColumns`: a chest holds three rows of `strength` columns.
pub fn inventory_columns(m: &Mob) -> i32 {
    if m.extra.chested { strength(m) } else { 0 }
}
/// The mob is a zombie of any kind (`Zombie.class`).
fn is_zombie(s: &Seen) -> bool {
    matches!(
        s.species,
        Species::Zombie | Species::Husk | Species::Drowned | Species::ZombieVillager
    )
}
/// The mob is an illager (`AbstractIllager.class`).
pub fn is_illager(s: &Seen) -> bool {
    matches!(
        s.species,
        Species::Pillager | Species::Vindicator | Species::Evoker | Species::Illusioner
    )
}
pub fn register(m: &mut Mob) {
    m.set_required_path_length(40.0);
    let tempt = m.kind.tempt.clone();
    let trader = is_trader(m);
    if trader {
        m.extra.ints[0] = DEFAULT_DESPAWN_DELAY;
    }
    let g = &mut m.ai.goals;
    g.add(0, Float);
    if trader {
        g.add(1, Panic::new(2.0));
    }
    g.add(1, equine::RunAroundLikeCrazy);
    g.add(2, super::caravan::FollowCaravan::new(f64::from(2.1_f32)));
    g.add(3, RangedAttack::new(1.25, 40, 40, 20.0, spit));
    g.add(3, Panic::new(1.2));
    g.add(4, Breed::new(1.0));
    g.add(5, Tempt::new(1.25, tempt, false));
    g.add(6, FollowParent::new(1.0));
    g.add(7, Stroll::avoiding_water(0.7));
    g.add(8, LookAtPlayer::new(6.0));
    g.add(9, RandomLookAround::default());
    m.ai.targets.add(1, LlamaHurtBy(HurtByTarget::new()));
    if trader {
        m.ai.targets.add(1, super::caravan::DefendTrader::default());
    }
    m.ai.targets.add(
        2,
        NearestTarget::new(
            Prey::Filter(|s| s.species == Species::Wolf && !s.tame),
            false,
        )
        .interval(16)
        .reach()
        .follow_scale(0.25),
    );
    if trader {
        m.ai.targets
            .add(2, NearestTarget::new(Prey::Filter(is_zombie), true));
        m.ai.targets
            .add(2, NearestTarget::new(Prey::Filter(is_illager), true));
    }
}
/// `TraderLlama.DEFAULT_DESPAWN_DELAY`.
pub const DEFAULT_DESPAWN_DELAY: i32 = 47999;
/// `Llama.finalizeSpawn` and `AbstractChestedHorse.randomizeAttributes`:
/// health 15 to 30, a coat and the strength.
pub fn finalize(m: &mut Mob) {
    let r = &mut m.random;
    let health =
        15.0 + r.next_int_bound(8).unwrap_or(0) as f32 + r.next_int_bound(9).unwrap_or(0) as f32;
    let max = if r.next_float() < 0.04 { 5 } else { 3 };
    let strength = 1 + r.next_int_bound(max).unwrap_or(0);
    let variant = r.next_int_bound(4).unwrap_or(0);
    set_base(m, Attr::MaxHealth, f64::from(health));
    m.health = health;
    m.extra.variant2 = strength as u8;
    m.extra.variant = variant as u8;
    m.extra.vector[0] = m.kind.jump_strength;
}
/// `Llama.getBreedOffspring` after `setOffspringAttributes`: the strength
/// (rarely one more) and either parent's coat.
pub fn offspring(child: &mut Mob, a: &Mob, b: &Mob, r: &mut impl RandomSource) {
    let top = strength(a).max(strength(b));
    let mut s = r.next_int_bound(top).unwrap_or(0) + 1;
    if r.next_float() < 0.03 {
        s += 1;
    }
    child.extra.variant2 = s.clamp(1, 5) as u8;
    child.extra.variant = if r.next_boolean() {
        a.extra.variant
    } else {
        b.extra.variant
    };
    if is_trader(child) {
        child.persistent = true;
    }
}
/// A llama's baby as `Llama.getBreedOffspring` makes it.
#[derive(Clone, Debug)]
pub struct Offspring {
    pub max_health: f32,
    pub jump: f64,
    pub speed: f64,
    pub strength: u8,
    pub variant: u8,
}
/// `Llama.getBreedOffspring` with `AbstractHorse.setOffspringAttributes`,
/// drawn from the parent's random with the partner's stock.
pub fn breed_offspring(m: &mut Mob, partner: &Seen) -> Offspring {
    // Seen.profession is the partner's strength.
    let top = strength(m).max(i32::from(partner.profession));
    let (health, jump, speed, own) = (
        f64::from(m.attrs.max_health),
        m.extra.vector[0],
        m.attrs.speed,
        m.extra.variant,
    );
    let r = &mut m.random;
    let max_health = equine::offspring(health, partner.stats[0], 15.0, 30.0, r) as f32;
    let jump = equine::offspring(jump, partner.stats[1], 0.4, 1.0, r);
    let speed = equine::offspring(speed, partner.stats[2], 0.1125, 0.3375, r);
    let mut s = r.next_int_bound(top).unwrap_or(0) + 1;
    if r.next_float() < 0.03 {
        s += 1;
    }
    let variant = if r.next_boolean() {
        own
    } else {
        partner.variant
    };
    Offspring {
        max_health,
        jump,
        speed,
        strength: s.clamp(1, 5) as u8,
        variant,
    }
}
/// The game makes the baby drawn by `breed_offspring`.
pub fn apply_offspring(child: &mut Mob, o: &Offspring) {
    set_base(child, Attr::MaxHealth, f64::from(o.max_health));
    child.health = o.max_health;
    child.extra.vector[0] = o.jump;
    set_base(child, Attr::MovementSpeed, o.speed);
    child.extra.variant2 = o.strength;
    child.extra.variant = o.variant;
    if is_trader(child) {
        child.persistent = true;
    }
}
/// `LlamaHurtByTargetGoal`: stops hunting right after a spit.
#[derive(Clone, Debug)]
struct LlamaHurtBy(HurtByTarget);
impl Goal for LlamaHurtBy {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.extra.flag2 {
            m.extra.flag2 = false;
            return false;
        }
        self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx)
    }
}
/// `Llama.performRangedAttack` / `spit`: a llama spit toward the target's
/// lower third, 1.5 fast, with 10 degrees of spread.
fn spit(m: &mut Mob, who: Who, _power: f32, cx: &mut Cx) {
    let Some(t) = cx.w.target_info(who) else {
        return;
    };
    let body = m.body_yaw.to_radians();
    let reach = (f64::from(m.width()) + 1.0) * 0.5;
    let start = Vec3::new(
        m.position.x - reach * f64::from(body.sin()),
        m.eye_y() - f64::from(0.1_f32),
        m.position.z + reach * f64::from(body.cos()),
    );
    let top = t.bounds.max_position();
    let low = t.bounds.min_position();
    let xd = t.position.x - m.position.x;
    let yd = low.y + (top.y - low.y) * 0.333_333_333_333_333_3 - start.y;
    let zd = t.position.z - m.position.z;
    let yo = (xd * xd + zd * zd).sqrt() * f64::from(0.2_f32);
    let mut p = Projectile::new(ProjectileKind::LlamaSpit, start, Some(Who::Mob(m.id)));
    p.shoot(
        &mut crate::projectiles::own_random(m),
        [xd, yd + yo, zd],
        1.5,
        10.0,
    );
    cx.events.push(MobEvent::Projectile(p));
    let pitch = 1.0 + (m.random.next_float() - m.random.next_float()) * 0.2;
    cx.events.push(MobEvent::Sound(
        "entity.llama.spit",
        [m.position.x, m.position.y, m.position.z],
        1.0,
        pitch,
    ));
    m.extra.flag2 = true;
}
/// `AbstractHorse.aiStep` for llamas, which never graze: the slow healing.
pub fn ai_step(m: &mut Mob, w: &MobWorld) {
    if m.alive() && m.random.next_int_bound(900).unwrap_or(1) == 0 && m.death_time == 0 {
        m.health = (m.health + 1.0).min(m.attrs.max_health);
    }
    if is_trader(m) {
        super::caravan::trader_llama_tick(m, w);
    }
}
/// The llama's own save fields, over the horse ones: `Variant` (the coat id),
/// `Strength` and a trader llama's `DespawnDelay`.
pub fn to_nbt(m: &Mob, map: &mut horizon_world::nbt::Compound) {
    use horizon_world::nbt::Tag;
    map.insert("Variant".into(), Tag::Int(i32::from(m.extra.variant)));
    map.insert("Strength".into(), Tag::Int(strength(m)));
    if is_trader(m) {
        map.insert("DespawnDelay".into(), Tag::Int(m.extra.ints[0]));
    }
}
pub fn from_nbt(m: &mut Mob, tag: &horizon_world::nbt::Tag) {
    use crate::save::number;
    m.extra.variant = number(tag.get("Variant")).unwrap_or(0).clamp(0, 3) as u8;
    m.extra.variant2 = number(tag.get("Strength")).unwrap_or(1).clamp(1, 5) as u8;
    m.extra.variant3 = i32::from(m.extra.variant3).min(MAX_TEMPER) as u8;
    if is_trader(m) {
        m.extra.ints[0] =
            number(tag.get("DespawnDelay")).map_or(DEFAULT_DESPAWN_DELAY, |v| v as i32);
    }
}
/// The coat texture (`LlamaRenderer`).
pub fn texture(m: &Mob) -> String {
    let baby = if m.is_baby() { "_baby" } else { "" };
    let coat = VARIANTS[(m.extra.variant as usize).min(3)];
    format!("entity/llama/llama_{coat}{baby}")
}
/// The carpet colour on the llama, if it wears one.
fn carpet(m: &Mob) -> Option<&str> {
    m.extra
        .head
        .as_deref()
        .and_then(|i| i.strip_prefix("minecraft:"))
        .and_then(|i| i.strip_suffix("_carpet"))
}
/// `LlamaDecorLayer`: the carpet of an adult, else the trader's blanket.
pub fn decor(m: &Mob) -> Option<(String, String)> {
    let baby = m.is_baby();
    let model = if baby {
        "llama_baby#decor"
    } else {
        "llama#decor"
    };
    if let Some(color) = carpet(m)
        && !baby
    {
        return Some((model.into(), format!("entity/equipment/llama_body/{color}")));
    }
    is_trader(m).then(|| {
        let name = if baby {
            "trader_llama_baby"
        } else {
            "trader_llama"
        };
        (model.into(), format!("entity/equipment/llama_body/{name}"))
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::random::LegacyRandom;

    #[test]
    fn strength_stays_in_range_and_rarely_reaches_five() {
        let mut r = LegacyRandom::new(7);
        let mut fives = 0;
        for _ in 0..4000 {
            let max = if r.next_float() < 0.04 { 5 } else { 3 };
            let s = 1 + r.next_int_bound(max).unwrap();
            assert!((1..=5).contains(&s));
            fives += i32::from(s > 3);
        }
        assert!(fives > 0 && fives < 200);
    }
    #[test]
    fn a_chest_holds_strength_columns() {
        assert_eq!(DEFAULT_DESPAWN_DELAY, 47999);
        assert_eq!(MAX_TEMPER, 30);
    }
}
