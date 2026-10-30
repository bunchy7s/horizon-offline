//! Endermen: anger from being stared at, teleporting (randomly in daylight,
//! when hit by projectiles or water, and towards a distant target), freezing
//! while looked at, and picking up / placing `enderman_holdable` blocks.
use super::{
    Damage, Mob, MobEvent, MobWorld, Species, Who,
    anim::{Poses, State},
    goals::{
        Cx, Float, Goal, HurtByTarget, JUMP, LookAtPlayer, MOVE, MeleeAttack, NearestTarget,
        RandomLookAround, Stroll, TARGET, goal_clone, reduced,
    },
};
use crate::blocks::Fluid;
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};

pub fn register(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(1, FreezeWhenLookedAt);
    g.add(2, MeleeAttack::new(1.0, false));
    let mut stroll = Stroll::avoiding_water(1.0);
    stroll.water_avoiding = Some(0.0);
    g.add(7, stroll);
    g.add(8, LookAtPlayer::new(8.0));
    g.add(8, RandomLookAround::default());
    g.add(10, LeaveBlock);
    g.add(11, TakeBlock);
    m.ai.targets.add(
        1,
        LookForPlayer {
            pending: false,
            aggro: 0,
            teleport: 0,
        },
    );
    m.ai.targets.add(2, HurtByTarget::new());
    m.ai.targets.add(
        3,
        NearestTarget::species(&[Species::Endermite], true).reach(),
    );
    m.set_malus(super::path::PathType::Water, -1.0);
}
/// Enderman.isBeingStaredBy: the player's look within a cone of 0.025 / distance
/// of the eyes, with line of sight, and no carved pumpkin.
pub fn stared_at(m: &Mob, w: &MobWorld) -> bool {
    let Some(p) = &w.player else { return false };
    if !p.alive || p.head.as_deref() == Some("minecraft:carved_pumpkin") {
        return false;
    }
    let look = p.look;
    let dir = [
        m.position.x - p.position.x,
        m.eye_y() - p.eye[1],
        m.position.z - p.position.z,
    ];
    let dist = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if dist < 1.0e-6 {
        return false;
    }
    let dot = (look[0] * dir[0] + look[1] * dir[1] + look[2] * dir[2]) / dist;
    dot > 1.0 - 0.025 / dist
        && w.line_of_sight(
            Vec3::new(p.eye[0], p.eye[1], p.eye[2]),
            Vec3::new(m.position.x, m.eye_y(), m.position.z),
        )
}
/// Enderman.teleport(x, y, z): drop to standing ground, require free space without liquid.
pub fn teleport_to(
    m: &mut Mob,
    w: &MobWorld,
    x: f64,
    y: f64,
    z: f64,
    events: &mut Vec<MobEvent>,
) -> bool {
    let (bx, bz) = (x.floor() as i32, z.floor() as i32);
    if !w.world.loaded(bx, bz) {
        return false;
    }
    let mut by = y.floor() as i32;
    while by > crate::chunk::MIN_Y && !w.facts.get(w.world.block(bx, by, bz)).full_collision {
        by -= 1;
    }
    let ground = w.facts.get(w.world.block(bx, by, bz));
    if !ground.full_collision || ground.fluid == Fluid::Water {
        return false;
    }
    let ny = f64::from(by + 1);
    let half = f64::from(m.width()) / 2.0;
    let b = Aabb::new(
        Vec3::new(x - half, ny, z - half),
        Vec3::new(x + half, ny + f64::from(m.height()), z + half),
    );
    if !w.no_collision(b) || w.fluids(b, ny + m.eye_height()).in_water || w.fluids(b, ny).in_lava {
        return false;
    }
    let old = m.center();
    let old_block = [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ];
    m.position = Vec3::new(x, ny, z);
    m.previous = m.position;
    m.velocity = Vec3::ZERO;
    m.ai.nav.stop();
    if !m.silent {
        // Level event 2018: the portal trail, from the old block to the new
        // one, packed as `BlockUtil.clampedPackDifferenceInPosition` does.
        let new_block = [x.floor() as i32, ny.floor() as i32, z.floor() as i32];
        let packed = |k: usize| ((new_block[k] - old_block[k]).clamp(-127, 127) + 127) & 255;
        events.push(MobEvent::LevelEvent {
            id: 2018,
            pos: old_block,
            data: packed(0) << 16 | packed(1) << 8 | packed(2),
        });
    }
    events.push(MobEvent::Sound("entity.enderman.teleport", old, 1.0, 1.0));
    events.push(MobEvent::Sound(
        "entity.enderman.teleport",
        m.center(),
        1.0,
        1.0,
    ));
    true
}
pub fn teleport_random(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) -> bool {
    if !m.alive() {
        return false;
    }
    let x = m.position.x + (m.random.next_double() - 0.5) * 64.0;
    let y = m.position.y + f64::from(m.random.next_int_bound(64).unwrap_or(32) - 32);
    let z = m.position.z + (m.random.next_double() - 0.5) * 64.0;
    teleport_to(m, w, x, y, z, events)
}
fn teleport_towards(
    m: &mut Mob,
    w: &MobWorld,
    at: Vec3,
    eye_y: f64,
    events: &mut Vec<MobEvent>,
) -> bool {
    let mut d = [
        m.position.x - at.x,
        m.position.y + f64::from(m.height()) * 0.5 - eye_y,
        m.position.z - at.z,
    ];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1.0e-9);
    d = d.map(|c| c / len);
    let x = m.position.x + (m.random.next_double() - 0.5) * 8.0 - d[0] * 16.0;
    let y = m.position.y + f64::from(m.random.next_int_bound(16).unwrap_or(8) - 8) - d[1] * 16.0;
    let z = m.position.z + (m.random.next_double() - 0.5) * 8.0 - d[2] * 16.0;
    teleport_to(m, w, x, y, z, events)
}
/// `Enderman.SPEED_MODIFIER_ATTACKING`, an ADD_VALUE modifier `setTarget`
/// adds while the enderman has a target.
pub fn attacking_speed(m: &Mob) -> f64 {
    if m.species() == Species::Enderman && m.ai.target.is_some() {
        f64::from(0.15f32)
    } else {
        0.0
    }
}
/// Enderman.customServerAiStep: in daylight, flee by teleporting. The End has a
/// fixed time, so `Level.isBrightOutside` is never true there.
pub fn custom_server_ai_step(m: &mut Mob, cx: &mut Cx) {
    if cx.w.daylight && !cx.w.end && m.age as i32 >= m.extra.counter + 600 {
        let br = super::hostile::mob_brightness(m, cx.w);
        let (x, y, z) = (
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        );
        if br > 0.5 && cx.w.can_see_sky(x, y, z) && m.random.next_float() * 30.0 < (br - 0.4) * 2.0
        {
            m.ai.target = None;
            teleport_random(m, cx.w, cx.events);
        }
    }
    // Being angry (target set) makes the head "creepy" (render).
    m.extra.flag = m.ai.target.is_some();
}
/// Whether a hit teleports the enderman away instead of hurting it: any
/// projectile, unless it rides something (`!isPassenger()`).
pub fn dodges(m: &Mob, source: &Damage) -> bool {
    source.is_projectile() && !m.extra.passenger
}
/// Enderman.hurtServer: projectiles miss (teleport away), others may teleport.
pub fn on_damage(m: &mut Mob, source: &Damage) -> bool {
    if dodges(m, source) {
        // repeatedlyTryToTeleport is applied by the next AI step.
        m.extra.counter2 = 64;
        return false;
    }
    true
}
pub fn after_hurt(m: &mut Mob, source: &Damage) {
    // A thrown potion's hit (`indirect_magic`, which only a splash potion
    // deals to a mob) leaves `Enderman.hurtServer` before the random chance:
    // `repeatedlyTryToTeleport` replaces it.
    if source.kind != super::DamageKind::IndirectMagic
        && source.attacker.is_none()
        && m.random.next_int_bound(10).unwrap_or(0) != 0
    {
        m.extra.counter2 = 1;
    }
}
/// Pending teleports (repeatedlyTryToTeleport: up to 64 tries).
pub fn ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    // Enderman.aiStep: never jumps.
    m.ai.jumping = false;
    if m.extra.counter2 > 0 {
        let tries = m.extra.counter2;
        m.extra.counter2 = 0;
        for _ in 0..tries {
            if teleport_random(m, w, events) {
                break;
            }
        }
    }
    if (m.fluids.in_water || super::ai::in_rain(m, w)) && m.alive() {
        // Hurt by water: teleport away.
        m.extra.counter2 = m.extra.counter2.max(1);
    }
}
/// EndermanModel.setupAnim: half-speed clamped limbs, held-block arms, creepy head.
pub fn anim(m: &Mob, s: &State, p: &mut Poses) {
    super::anim::humanoid_pub(p, s, m);
    for part in ["right_arm", "left_arm", "right_leg", "left_leg"] {
        let x = p.get(part)[3] * 0.5;
        p.xrot(part, x.clamp(-0.4, 0.4));
    }
    if m.extra.carried.is_some() {
        p.xrot("right_arm", -0.5);
        p.xrot("left_arm", -0.5);
        p.zrot("right_arm", 0.05);
        p.zrot("left_arm", -0.05);
    }
    if m.extra.flag {
        p.get("head")[1] -= 5.0;
        p.get("hat")[1] += 5.0;
    }
}
/// EndermanFreezeWhenLookedAt
#[derive(Clone, Debug)]
struct FreezeWhenLookedAt;
impl Goal for FreezeWhenLookedAt {
    goal_clone!();
    fn flags(&self) -> u8 {
        JUMP | MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.ai.target != Some(Who::Player) {
            return false;
        }
        let Some(p) = &cx.w.player else { return false };
        m.distance_sqr(p.position) <= 256.0 && stared_at(m, cx.w)
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(p) = &cx.w.player {
            let at = [p.position.x, p.eye[1], p.position.z];
            m.look_at(at);
        }
    }
}
/// EndermanLookForPlayerGoal: anger after being stared at for 5 ticks,
/// teleport away when stared at close up, or towards a far target.
#[derive(Clone, Debug)]
struct LookForPlayer {
    pending: bool,
    aggro: i32,
    teleport: i32,
}
impl Goal for LookForPlayer {
    goal_clone!();
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(p) = &cx.w.player else { return false };
        self.pending = p.alive
            && !p.creative
            && m.distance_sqr(p.position) <= m.attrs.follow_range.powi(2)
            && stared_at(m, cx.w);
        self.pending
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        self.aggro = reduced(5);
        self.teleport = 0;
        // setBeingStaredAt: play the stare sound.
        m.extra.counter = m.age as i32;
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        self.pending = false;
        m.ai.target = None;
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(p) = cx.w.player.clone() else {
            return false;
        };
        if self.pending {
            if !stared_at(m, cx.w) {
                return false;
            }
            m.turn_to(p.eye, 10.0, 10.0);
            return true;
        }
        p.alive
            && !p.creative
            && m.ai.target == Some(Who::Player)
            && m.distance_sqr(p.position) <= m.attrs.follow_range.powi(2)
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(p) = cx.w.player.clone() else { return };
        if self.pending {
            self.aggro -= 1;
            if self.aggro <= 0 {
                self.pending = false;
                m.ai.target = Some(Who::Player);
                cx.events.push(MobEvent::Sound(
                    "entity.enderman.stare",
                    m.center(),
                    2.5,
                    1.0,
                ));
            }
        } else if m.ai.target == Some(Who::Player) {
            if stared_at(m, cx.w) {
                if m.distance_sqr(p.position) < 16.0 {
                    teleport_random(m, cx.w, cx.events);
                }
                self.teleport = 0;
            } else {
                let far = m.distance_sqr(p.position) > 256.0;
                let t = self.teleport;
                if far {
                    self.teleport += 1;
                }
                if far
                    && t >= reduced(30)
                    && teleport_towards(m, cx.w, p.position, p.eye[1], cx.events)
                {
                    self.teleport = 0;
                }
            }
        }
    }
}
/// EndermanTakeBlockGoal
#[derive(Clone, Debug)]
struct TakeBlock;
impl Goal for TakeBlock {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        // EndermanTakeBlockGoal.canUse: needs mobGriefing, checked before the draw.
        m.extra.carried.is_none()
            && cx.w.mob_griefing
            && m.random.next_int_bound(reduced(20)).unwrap_or(1) == 0
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let x = (m.position.x - 2.0 + m.random.next_double() * 4.0).floor() as i32;
        let y = (m.position.y + m.random.next_double() * 3.0).floor() as i32;
        let z = (m.position.z - 2.0 + m.random.next_double() * 4.0).floor() as i32;
        let state = cx.w.world.block(x, y, z);
        let name = cx.w.facts.get(state).name.to_string();
        if !cx.w.has_tag("minecraft:enderman_holdable", &name) {
            return;
        }
        // Reachable: the clip from the mob's column hits this block first.
        let from = Vec3::new(
            m.position.x.floor() + 0.5,
            f64::from(y) + 0.5,
            m.position.z.floor() + 0.5,
        );
        let to = Vec3::new(f64::from(x) + 0.5, f64::from(y) + 0.5, f64::from(z) + 0.5);
        let dx = (x - m.position.x.floor() as i32).abs() + (z - m.position.z.floor() as i32).abs();
        if dx > 0
            && !cx.w.clear_between(
                from,
                Vec3::new(
                    to.x - (to.x - from.x) * 0.3,
                    to.y,
                    to.z - (to.z - from.z) * 0.3,
                ),
                false,
            )
        {
            return;
        }
        cx.events.push(MobEvent::SetBlock {
            pos: [x, y, z],
            state: cx.w.world.air,
        });
        m.extra.carried = Some(state);
    }
}
/// EndermanLeaveBlockGoal
#[derive(Clone, Debug)]
struct LeaveBlock;
impl Goal for LeaveBlock {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        // EndermanLeaveBlockGoal.canUse: needs mobGriefing, checked before the draw.
        m.extra.carried.is_some()
            && cx.w.mob_griefing
            && m.random.next_int_bound(reduced(2000)).unwrap_or(1) == 0
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(carried) = m.extra.carried else {
            return;
        };
        let x = (m.position.x - 1.0 + m.random.next_double() * 2.0).floor() as i32;
        let y = (m.position.y + m.random.next_double() * 2.0).floor() as i32;
        let z = (m.position.z - 1.0 + m.random.next_double() * 2.0).floor() as i32;
        let target = cx.w.world.block(x, y, z);
        let below = cx.w.world.block(x, y - 1, z);
        let bf = cx.w.facts.get(below);
        let block = Aabb::new(
            Vec3::new(f64::from(x), f64::from(y), f64::from(z)),
            Vec3::new(f64::from(x + 1), f64::from(y + 1), f64::from(z + 1)),
        );
        let free = !cx.w.others_near(block).any(|s| s.bounds.intersects(block));
        if cx.w.facts.is_air(target)
            && !cx.w.facts.is_air(below)
            && &*bf.name != "minecraft:bedrock"
            && bf.full_collision
            && free
        {
            cx.events.push(MobEvent::SetBlock {
                pos: [x, y, z],
                state: carried,
            });
            m.extra.carried = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::DamageKind;
    use std::sync::Arc;

    fn enderman() -> Mob {
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.species = Species::Enderman;
        Mob::new(1, Arc::new(kind), Vec3::ZERO, 0.0)
    }

    /// Enderman.hurtServer: a projectile makes a free enderman teleport away
    /// unhurt, but one that rides something (`isPassenger`) takes the hit.
    #[test]
    fn projectiles_hit_a_seated_enderman() {
        for kind in [DamageKind::Arrow, DamageKind::Thrown, DamageKind::Fireball] {
            let mut free = enderman();
            assert!(!on_damage(&mut free, &Damage::of(kind)), "{kind:?}");
            assert_eq!(free.extra.counter2, 64);
            // The arrow handler keeps the arrow flying when the enderman dodges.
            assert!(dodges(&free, &Damage::of(kind)), "{kind:?}");

            let mut seated = enderman();
            seated.extra.passenger = true;
            assert!(on_damage(&mut seated, &Damage::of(kind)), "{kind:?}");
            assert_eq!(seated.extra.counter2, 0, "no teleport while seated");
            assert!(!dodges(&seated, &Damage::of(kind)), "{kind:?}");
        }
    }
}
