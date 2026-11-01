//! Nautili and zombie nautili (`AbstractNautilus`, `Nautilus`,
//! `ZombieNautilus`, `NautilusAi`, `ZombieNautilusAi`): smooth swimming,
//! drying out on land, taming with pufferfish, the saddle and armour, the
//! ridden swim with its dash, charging at pufferfish and at whoever hurt
//! them, the zombie's temperate and coral variants, and `NautilusModel`.
use super::{
    Mob, MobEvent, MobWorld, Species, Who,
    anim::{Poses, State},
    control::{LookKind, MoveKind, Operation},
    goals::{Breed, Cx, Goal, LOOK, LookAtPlayer, MOVE, Panic, Stroll, Tempt, goal_clone},
    keyframes::Animation,
    nav::NavKind,
};
use crate::blocks::Fluid;
use horizon_core::{geometry::Vec3, random::RandomSource};
use std::collections::HashMap;

// Nautilus state: `tame`/`owner`, the saddle, `head` the body armour,
// `variant` the zombie's variant (0 temperate, 1 warm), `anim_ticks` the
// dash cooldown and `flag2` dashing, `counter` the attack target cooldown,
// `counter2` the charge cooldown, `anger` the ticks left of `ANGRY_AT`
// (at `ai.last_hurt_by`), `ints[1]` the rider's breath effect given,
// `scratch[1]` a dash whose sound is still to play.
const DASH_COOLDOWN: i32 = 40;
const TIME_BETWEEN_ATTACKS: i32 = 80;
const ANGER_DURATION: i32 = 400;
/// `ZombieNautilusVariants`: the variant keys in index order.
pub const VARIANTS: [&str; 2] = ["temperate", "warm"];

pub fn is_nautilus(s: Species) -> bool {
    matches!(s, Species::Nautilus | Species::ZombieNautilus)
}
fn zombie(m: &Mob) -> bool {
    m.species() == Species::ZombieNautilus
}
pub fn register(m: &mut Mob) {
    // SmoothSwimmingMoveControl(85, 10, 0.011, 0, true), SmoothSwimmingLookControl(10).
    m.ai.mov.kind = MoveKind::SmoothSwimming {
        max_turn_x: 85.0,
        max_turn_y: 10.0,
        in_water: 0.011,
        outside: 0.0,
        gravity: true,
    };
    m.ai.look.kind = LookKind::SmoothSwimming(10.0);
    m.ai.nav.kind = NavKind::Water { breach: false };
    m.set_malus(super::path::PathType::Water, 0.0);
    let tempt = m.kind.tempt.clone();
    let stop = if m.is_baby() { 2.5 } else { 3.5 };
    let is_zombie = zombie(m);
    let g = &mut m.ai.goals;
    if is_zombie {
        // ZombieNautilusAi: no panic, no love, slower temptation and charge.
        let mut t = Tempt::new(0.9, tempt, false);
        t.stop_distance = stop;
        g.add(1, t);
        g.add(2, Charge::new(0.5));
        g.add(3, Stroll::swimming(1.0, 1));
        g.add(3, LookAtPlayer::new(6.0));
    } else {
        g.add(0, Panic::new(1.6));
        g.add(1, Breed::new(0.4));
        let mut t = Tempt::new(1.3, tempt, false);
        t.stop_distance = stop;
        g.add(2, t);
        g.add(3, Charge::new(0.6));
        g.add(4, Stroll::swimming(1.0, 1));
        g.add(4, LookAtPlayer::new(6.0));
    }
}
/// `finalizeSpawn`: `NautilusAi.initMemories` (2400 to 3600 ticks before it
/// first looks for prey) and the zombie's variant by biome.
pub fn finalize(m: &mut Mob, warm_ocean: bool) {
    m.extra.counter = 2400 + m.random.next_int_bound(1201).unwrap_or(0);
    if zombie(m) {
        m.extra.variant = u8::from(warm_ocean);
    }
}
/// `getControllingPassenger`: a player on a saddled nautilus.
pub fn controlled(m: &Mob) -> bool {
    m.rider.is_some() && m.extra.saddled
}
/// `AbstractNautilus.isFood`: taming items while wild, food otherwise.
pub fn is_food(m: &Mob, item: &str, taming: &[String]) -> bool {
    if !m.extra.tame && !m.is_baby() {
        taming.iter().any(|t| t == item)
    } else {
        m.kind.food.iter().any(|t| t == item)
    }
}
/// The ambient, hurt and death sounds by age and whether it is under water.
pub fn sounds(m: &Mob) -> [&'static str; 3] {
    let under = m.fluids.eye_in_water;
    match (zombie(m), m.is_baby(), under) {
        (true, _, true) => [
            "entity.zombie_nautilus.ambient",
            "entity.zombie_nautilus.hurt",
            "entity.zombie_nautilus.death",
        ],
        (true, _, false) => [
            "entity.zombie_nautilus.ambient_land",
            "entity.zombie_nautilus.hurt_land",
            "entity.zombie_nautilus.death_land",
        ],
        (false, true, true) => [
            "entity.baby_nautilus.ambient",
            "entity.baby_nautilus.hurt",
            "entity.baby_nautilus.death",
        ],
        (false, true, false) => [
            "entity.baby_nautilus.ambient_land",
            "entity.baby_nautilus.hurt_land",
            "entity.baby_nautilus.death_land",
        ],
        (false, false, true) => [
            "entity.nautilus.ambient",
            "entity.nautilus.hurt",
            "entity.nautilus.death",
        ],
        (false, false, false) => [
            "entity.nautilus.ambient_land",
            "entity.nautilus.hurt_land",
            "entity.nautilus.death_land",
        ],
    }
}
fn dash_sound(m: &Mob) -> &'static str {
    match (zombie(m), m.fluids.eye_in_water) {
        (true, true) => "entity.zombie_nautilus.dash",
        (true, false) => "entity.zombie_nautilus.dash_land",
        (false, true) => "entity.nautilus.dash",
        (false, false) => "entity.nautilus.dash_land",
    }
}
fn dash_ready_sound(m: &Mob) -> &'static str {
    match (zombie(m), m.fluids.eye_in_water) {
        (true, true) => "entity.zombie_nautilus.dash_ready",
        (true, false) => "entity.zombie_nautilus.dash_ready_land",
        (false, true) => "entity.nautilus.dash_ready",
        (false, false) => "entity.nautilus.dash_ready_land",
    }
}
/// `playEatingSound`.
pub fn eat_sound(m: &Mob) -> &'static str {
    match (zombie(m), m.is_baby()) {
        (true, _) => "entity.zombie_nautilus.eat",
        (false, true) => "entity.baby_nautilus.eat",
        (false, false) => "entity.nautilus.eat",
    }
}
/// `getEquipSound(SADDLE)`.
pub fn saddle_sound(m: &Mob) -> &'static str {
    if m.fluids.eye_in_water {
        "item.nautilus_saddle_underwater_equip"
    } else {
        "item.nautilus_saddle_equip"
    }
}
fn sound(m: &Mob, name: &'static str, events: &mut Vec<MobEvent>) {
    events.push(MobEvent::Sound(name, m.center(), 1.0, 1.0));
}
/// `AbstractNautilus.travelInWater`: its own speed, no fluid drag but 0.9.
pub fn travel(m: &mut Mob, w: &MobWorld, input: Vec3, events: &mut Vec<MobEvent>) -> bool {
    if !m.fluids.in_water {
        return false;
    }
    let speed = m.ai.speed;
    m.move_relative(speed, input);
    m.do_move(w, events);
    m.velocity = Vec3::new(m.velocity.x * 0.9, m.velocity.y * 0.9, m.velocity.z * 0.9);
    true
}
/// `tickRidden` and `getRiddenInput`: turn half way to the rider, swim along
/// their look, faster in water (`getRiddenSpeed`), and dash on the jump.
pub fn ridden(m: &mut Mob) {
    let Some(r) = m.rider else { return };
    let diff = super::living::wrap(r.yaw - m.yaw);
    m.yaw += diff * 0.5;
    m.head_pitch = r.pitch * 0.5;
    m.body_yaw = m.yaw;
    m.head_yaw = m.yaw;
    m.ai.nav.stop();
    m.ai.mov.operation = Operation::Wait;
    let (mut forward, mut up) = (0.0, 0.0);
    if r.forward != 0.0 {
        let mut f = r.pitch.to_radians().cos();
        let mut u = -r.pitch.to_radians().sin();
        if r.forward < 0.0 {
            f *= -0.5;
            u *= -0.5;
        }
        forward = f;
        up = u;
    }
    let factor = if m.fluids.in_water { 0.0325 } else { 0.02 };
    m.ai.speed = factor * m.movement_speed() as f32;
    m.ai.xxa = r.strafe;
    m.ai.yya = up;
    m.ai.zza = forward;
    if r.jump > 0.0 {
        if m.extra.anim_ticks <= 0 && !m.extra.flag2 {
            dash(m, r.jump);
        }
        if let Some(rider) = m.rider.as_mut() {
            rider.jump = 0.0;
        }
    }
}
/// `executeRidersJump` (with `handleStartJump`'s sound): along the rider's
/// look, 1.2 in water and 0.5 on land.
fn dash(m: &mut Mob, amount: f32) {
    let Some(r) = m.rider else { return };
    let (yaw, pitch) = (r.yaw.to_radians(), r.pitch.to_radians());
    let look = Vec3::new(
        f64::from(-yaw.sin() * pitch.cos()),
        f64::from(-pitch.sin()),
        f64::from(yaw.cos() * pitch.cos()),
    );
    let power = if m.fluids.in_water { 1.2 } else { 0.5 };
    let k = f64::from(power * amount) * m.movement_speed();
    m.velocity = Vec3::new(
        m.velocity.x + look.x * k,
        m.velocity.y + look.y * k,
        m.velocity.z + look.z * k,
    );
    m.extra.anim_ticks = DASH_COOLDOWN;
    m.extra.flag2 = true;
    // The sound plays on its next tick.
    m.extra.scratch[1] = 1;
}
/// `AbstractNautilus.getNautilusRestrictionRadius`: 32 blocks, or 16 for a
/// baby or a saddled one.
fn restriction_radius(m: &Mob) -> i32 {
    if !m.is_baby() && !m.extra.saddled {
        32
    } else {
        16
    }
}
/// `AbstractNautilus.checkRestriction`: a tame nautilus that is neither
/// leashed nor ridden keeps its home where it is, within its radius.
fn check_restriction(m: &mut Mob) {
    if m.extra.leash.is_some() || m.rider.is_some() || !m.extra.tame {
        return;
    }
    let radius = restriction_radius(m);
    let at = [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ];
    let kept = m.ai.home.is_some_and(|(c, r)| {
        let d = [c[0] - at[0], c[1] - at[1], c[2] - at[2]];
        let sq = f64::from(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
        sq < f64::from(radius + 8).powi(2) && r == radius
    });
    if !kept {
        m.ai.home = Some((at, radius));
    }
}
/// `AbstractNautilus.tick` and the brains' memories: the rider's breath, the
/// dash clock, bubbles, and choosing whom to charge at
/// (`NautilusAi.findNearestValidAttackTarget`).
pub fn ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    // customServerAiStep: the home is kept before the rest of the step.
    check_restriction(m);
    if m.extra.scratch[1] != 0 {
        m.extra.scratch[1] = 0;
        sound(m, dash_sound(m), events);
    }
    // applyEffects: the rider breathes under water.
    if m.rider.is_some() {
        if m.extra.ints[1] == 0 || w.game_time % 40 == 0 {
            m.extra.ints[1] = 1;
            events.push(MobEvent::EffectPlayer {
                effect: "minecraft:breath_of_the_nautilus",
                ticks: 60,
                amplifier: 0,
            });
        }
    } else {
        m.extra.ints[1] = 0;
    }
    if m.extra.flag2 && m.extra.anim_ticks < 35 {
        m.extra.flag2 = false;
    }
    if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks -= 1;
        if m.extra.anim_ticks == 0 {
            sound(m, dash_ready_sound(m), events);
        }
    }
    if m.fluids.in_water {
        bubbles(m, events);
    }
    // CountDownCooldownTicks and the ANGRY_AT expiry.
    if m.extra.counter > 0 {
        m.extra.counter -= 1;
    }
    if m.extra.counter2 > 0 {
        m.extra.counter2 -= 1;
    }
    if m.extra.anger > 0 {
        m.extra.anger -= 1;
    }
    if m.ai.target.is_none() && m.alive() && !m.no_ai {
        m.ai.target = find_target(m, w);
    }
}
/// `spawnBubbles`: more as it swims faster, from behind its mouth.
fn bubbles(m: &mut Mob, events: &mut Vec<MobEvent>) {
    let v = m.velocity;
    let speed = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    let chance = (speed * 2.0).clamp(f64::from(0.15f32), 1.0);
    if f64::from(m.random.next_float()) >= chance {
        return;
    }
    let (yaw, pitch) = (
        m.yaw.to_radians(),
        m.head_pitch.clamp(-10.0, 10.0).to_radians(),
    );
    let dir = [
        f64::from(-yaw.sin() * pitch.cos()),
        f64::from(-pitch.sin()),
        f64::from(yaw.cos() * pitch.cos()),
    ];
    let spread = m.random.next_double() * 0.8 * (1.0 + speed);
    let velocity = [
        (f64::from(m.random.next_float()) - 0.5) * spread,
        (f64::from(m.random.next_float()) - 0.5) * spread,
        (f64::from(m.random.next_float()) - 0.5) * spread,
    ];
    events.push(MobEvent::ParticleAt {
        kind: "bubble",
        at: [
            m.position.x - dir[0] * 1.1,
            m.position.y - dir[1] + 0.25,
            m.position.z - dir[2] * 1.1,
        ],
        velocity,
    });
}
/// `AbstractNautilus.tick` on a client of a server: the bubbles of a nautilus
/// in the water, from the motion the server sent.
pub fn client_tick(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if m.fluids.in_water {
        bubbles(m, events);
    }
}
fn in_water_at(w: &MobWorld, p: Vec3) -> bool {
    w.fluid(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32) == Fluid::Water
}
/// `NautilusAi.findNearestValidAttackTarget`: whoever it is angry at, else
/// now and then (a coin flip every 2400 to 3600 ticks) the nearest pufferfish.
fn find_target(m: &mut Mob, w: &MobWorld) -> Option<Who> {
    if m.extra.in_love > 0 || !m.fluids.in_water || m.is_baby() || m.extra.tame {
        return None;
    }
    if m.extra.anger > 0
        && let Some(who) = m.ai.last_hurt_by
        && let Some(t) = w.target_info(who)
        && t.alive
        && !(t.is_player && t.creative)
        && in_water_at(w, t.position)
    {
        return Some(who);
    }
    if m.extra.counter > 0 {
        return None;
    }
    m.extra.counter = 2400 + m.random.next_int_bound(1201).unwrap_or(0);
    if m.random.next_float() < 0.5 {
        return None;
    }
    w.nearby(m.position, 16.0, |s| {
        s.species == Species::Pufferfish && in_water_at(w, s.position)
    })
    .next()
    .map(|s| Who::Mob(s.id))
}
/// `hurtServer`: `NautilusAi.setAngerTarget` for 400 ticks.
pub fn after_hurt(m: &mut Mob, source: &super::Damage) {
    if source.attacker.is_some() {
        m.extra.anger = ANGER_DURATION + 1;
    }
}
/// `ChargeAttack(80, ..., speed, 2, 12, 11, dash)`: rush straight at the
/// target and ram it once.
#[derive(Clone, Debug)]
struct Charge {
    speed: f64,
    start: Vec3,
    velocity: Vec3,
}
impl Charge {
    fn new(speed: f64) -> Self {
        Self {
            speed,
            start: Vec3::ZERO,
            velocity: Vec3::ZERO,
        }
    }
    fn still_ok(&self, m: &Mob, cx: &Cx) -> bool {
        let Some(t) = m.ai.target.and_then(|who| cx.w.target_info(who)) else {
            return false;
        };
        if !t.alive || t.is_player && t.creative || m.extra.tame || m.extra.counter2 > 0 {
            return false;
        }
        if m.position.distance_squared(self.start) >= 12.0 * 12.0
            || t.position.distance_squared(m.position) >= 11.0 * 11.0
        {
            return false;
        }
        cx.w.line_of_sight(
            Vec3::new(m.position.x, m.eye_y(), m.position.z),
            Vec3::new(t.position.x, t.eye_y, t.position.z),
        )
    }
}
impl Goal for Charge {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.ai.target.is_some() && m.extra.counter2 <= 0 && m.extra.in_love <= 0
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.still_ok(m, cx)
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.start = m.position;
        let Some(t) = m.ai.target.and_then(|who| cx.w.target_info(who)) else {
            return;
        };
        let d = Vec3::new(
            t.position.x - m.position.x,
            t.position.y - m.position.y,
            t.position.z - m.position.z,
        );
        let len = d.length();
        self.velocity = if len < 1.0e-4 {
            Vec3::ZERO
        } else {
            Vec3::new(
                d.x / len * self.speed,
                d.y / len * self.speed,
                d.z / len * self.speed,
            )
        };
        if self.still_ok(m, cx) {
            let name = if zombie(m) {
                "entity.zombie_nautilus.dash"
            } else {
                "entity.nautilus.dash"
            };
            sound(m, name, cx.events);
        }
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.extra.counter2 = TIME_BETWEEN_ATTACKS;
        m.ai.target = None;
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(who) = m.ai.target else { return };
        let Some(t) = cx.w.target_info(who) else {
            return;
        };
        m.turn_to([t.position.x, t.eye_y, t.position.z], 360.0, 360.0);
        m.velocity = self.velocity;
        if !m.bounds().intersects(t.bounds) {
            return;
        }
        // dealDamageToTarget and dealKnockBack (causeExtraKnockback along its facing).
        let from = [m.position.x, m.position.y, m.position.z];
        let amount = m.attrs.attack_damage;
        cx.events.push(match who {
            Who::Player => MobEvent::AttackPlayer(amount, from),
            Who::Mob(id) => MobEvent::AttackMob {
                target: id,
                amount,
                source: super::Damage::mob(m.id, from),
            },
        });
        let level = |e: &str| {
            m.extra
                .effects
                .iter()
                .find(|x| x.0 == e)
                .map_or(0, |x| x.2 + 1)
        };
        let boost = 0.25 * (level("minecraft:speed") - level("minecraft:slowness")) as f32;
        let factor = (self.speed as f32 * m.movement_speed() as f32).clamp(0.2, 2.0) + boost;
        let power = f64::from(factor * 2.0);
        if power > 0.0 {
            let yaw = m.yaw.to_radians();
            cx.events.push(MobEvent::Push {
                target: who,
                velocity: [
                    -f64::from(yaw.sin()) * power,
                    0.0,
                    f64::from(yaw.cos()) * power,
                ],
            });
            m.velocity = Vec3::new(m.velocity.x * 0.6, m.velocity.y, m.velocity.z * 0.6);
        }
        m.ai.target = None;
    }
}
/// The body texture: the baby's, the adult's, or the zombie's variant.
pub fn texture(m: &Mob) -> String {
    match (zombie(m), m.is_baby(), m.extra.variant) {
        (true, _, 1) => "entity/nautilus/zombie_nautilus_coral".into(),
        (true, _, _) => "entity/nautilus/zombie_nautilus".into(),
        (false, true, _) => "entity/nautilus/nautilus_baby".into(),
        (false, false, _) => "entity/nautilus/nautilus".into(),
    }
}
/// The body model: `ZombieNautilusRenderer` picks the coral model for the
/// warm variant.
pub fn model(m: &Mob) -> &'static str {
    match (zombie(m), m.is_baby(), m.extra.variant) {
        (true, _, 1) => "zombie_nautilus_coral#main",
        (true, _, _) => "zombie_nautilus#main",
        (false, true, _) if m.kind.layer("nautilus_baby#main").is_some() => "nautilus_baby#main",
        _ => "nautilus#main",
    }
}
/// The `NAUTILUS_BODY` equipment texture of a nautilus armour item.
pub fn armor_texture(item: &str) -> Option<String> {
    let material = item
        .strip_prefix("minecraft:")?
        .strip_suffix("_nautilus_armor")?;
    let material = if material == "golden" {
        "gold"
    } else {
        material
    };
    Some(format!("entity/equipment/nautilus_body/{material}"))
}
/// `NautilusModel.setupAnim` (and `ZombieNautilusCoralModel`'s corals,
/// hidden under armour).
pub fn anim(m: &Mob, s: &State, p: &mut Poses, animations: &HashMap<String, Animation>) {
    let deg = (std::f64::consts::PI / 180.0) as f32;
    let y = s.y_rot.clamp(-10.0, 10.0);
    let x = s.x_rot.clamp(-10.0, 10.0);
    p.yrot("body", y * deg);
    p.xrot("body", x * deg);
    if let Some(a) = animations.get("NautilusAnimation.SWIMMING") {
        a.apply_walk(s.pos + s.age / 5.0, s.speed + 0.2, 2.0, 3.0, p);
    }
    if m.extra.head.is_some() {
        p.visible("corals", false);
    }
}
/// Saved fields: `TamableAnimal`'s owner, the saddle, the zombie's variant
/// and the brain's attack cooldown.
pub fn to_nbt(m: &Mob, map: &mut horizon_world::nbt::Compound) {
    use horizon_world::nbt::Tag;
    if let Some(owner) = m.extra.owner {
        map.insert("Owner".into(), Tag::IntArray(owner.to_vec()));
    } else {
        map.remove(&"Owner".into());
    }
    map.insert("Sitting".into(), Tag::Byte(0));
    if zombie(m) {
        let v = VARIANTS[usize::from(m.extra.variant).min(1)];
        map.insert(
            "variant".into(),
            Tag::String(format!("minecraft:{v}").as_str().into()),
        );
    }
    let mut equipment = match map.get(&"equipment".into()) {
        Some(Tag::Compound(c)) => c.clone(),
        _ => horizon_world::nbt::Compound::new(),
    };
    match m.saddle_item() {
        Some(saddle) => {
            equipment.insert("saddle".into(), crate::save::item_to_nbt(&saddle));
        }
        None => {
            equipment.remove(&"saddle".into());
        }
    }
    map.insert("equipment".into(), Tag::Compound(equipment));
    map.insert(
        "Brain".into(),
        Tag::compound([(
            "memories",
            Tag::compound([(
                "minecraft:attack_target_cooldown",
                Tag::compound([("value", Tag::Int(m.extra.counter))]),
            )]),
        )]),
    );
}
pub fn from_nbt(m: &mut Mob, tag: &horizon_world::nbt::Tag) {
    use horizon_world::nbt::Tag;
    m.extra.owner = crate::save::uuid_from_tag(tag.get("Owner"));
    m.extra.tame = m.extra.owner.is_some();
    if let Some(Tag::String(v)) = tag.get("variant") {
        let v = v.to_string().unwrap_or_default();
        m.extra.variant = u8::from(v.trim_start_matches("minecraft:") == "warm");
    }
    let saddle = tag
        .get("equipment")
        .and_then(|e| e.get("saddle"))
        .and_then(crate::save::item_from_nbt);
    m.set_saddle(saddle);
    if let Some(Tag::Int(v)) = tag
        .get("Brain")
        .and_then(|b| b.get("memories"))
        .and_then(|b| b.get("minecraft:attack_target_cooldown"))
        .and_then(|b| b.get("value"))
    {
        m.extra.counter = *v;
    }
}
