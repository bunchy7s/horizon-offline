//! Phantoms: circling an anchor high above, swooping at the player every
//! eight to twelve seconds, scared off by cats (`Phantom`).
use super::{
    Mob, MobEvent, MobWorld, Species, Who,
    control::MoveKind,
    goals::{Cx, Goal, MOVE, TARGET, goal_clone, reduced},
};
use horizon_core::geometry::trig::Mth;
use horizon_core::{geometry::Vec3, random::RandomSource};

/// Phantom state in `Extra`: `flag` swooping, `home` the anchor,
/// `vector` the move target, `attrs.speed` the move control's speed.
pub fn register(m: &mut Mob) {
    m.ai.mov.kind = MoveKind::Custom;
    m.attrs.speed = 0.1;
    m.extra.home = Some([
        m.position.x.floor() as i32,
        m.position.y.floor() as i32 + 5,
        m.position.z.floor() as i32,
    ]);
    m.ai.goals.add(1, AttackStrategy { next_sweep: 0 });
    m.ai.goals.add(
        2,
        SweepAttack {
            scared: false,
            cat_search: 0,
        },
    );
    m.ai.goals.add(3, CircleAroundAnchor::default());
    m.ai.targets.add(
        1,
        AttackPlayerTarget {
            next_scan: reduced(20),
        },
    );
}
/// `Phantom.setPhantomSize` with `updatePhantomSizeInfo`: the size, clamped to
/// 0 to 64, and an attack damage base of 6 plus the size. The box and the
/// drawn model grow by `1 + 0.15 * size` (the mob's dimensions, `PhantomRenderer.scale`).
pub fn set_size(m: &mut Mob, size: i32) {
    use super::finalize::{Attr, set_base};
    let size = size.clamp(0, 64);
    if size != m.extra.size {
        m.extra.size = size;
        set_base(m, Attr::AttackDamage, f64::from(6 + size));
    }
}
fn can_attack(w: &MobWorld) -> bool {
    w.player
        .as_ref()
        .is_some_and(|p| p.alive && !p.creative && !p.spectator)
}
/// `Phantom.canAttack(level, player, TargetingConditions.DEFAULT)`: as above,
/// and in sight (`Sensing.hasLineOfSight`), so a roof or a wall hides the player.
fn can_attack_seen(m: &mut Mob, w: &MobWorld) -> bool {
    can_attack(w) && m.sees(w, Who::Player)
}
fn sea_level() -> i32 {
    63
}
/// `PhantomAttackPlayerTargetGoal`: every three seconds, the player
/// within 16 blocks across and 64 up or down.
#[derive(Clone, Debug)]
struct AttackPlayerTarget {
    next_scan: i32,
}
impl Goal for AttackPlayerTarget {
    goal_clone!();
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if self.next_scan > 0 {
            self.next_scan -= 1;
            return false;
        }
        self.next_scan = reduced(60);
        let Some(p) = &cx.w.player else {
            return false;
        };
        let d = p.position;
        let near = (d.x - m.position.x).abs() <= 16.0 + f64::from(m.width())
            && (d.y - m.position.y).abs() <= 64.0
            && (d.z - m.position.z).abs() <= 16.0 + f64::from(m.width());
        if near && can_attack_seen(m, cx.w) {
            m.ai.target = Some(Who::Player);
            return true;
        }
        false
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.ai.target.is_some() && can_attack_seen(m, cx.w)
    }
}
/// `PhantomAttackStrategyGoal`: circle, then swoop.
#[derive(Clone, Debug)]
struct AttackStrategy {
    next_sweep: i32,
}
impl AttackStrategy {
    fn anchor_above_target(m: &mut Mob, cx: &Cx) {
        let Some(t) = m.ai.target.and_then(|t| cx.w.target_info(t)) else {
            return;
        };
        let y = (t.position.y.floor() as i32 + 20 + m.random.next_int_bound(20).unwrap_or(0))
            .max(sea_level() + 1);
        m.extra.home = Some([t.position.x.floor() as i32, y, t.position.z.floor() as i32]);
    }
}
impl Goal for AttackStrategy {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.ai.target.is_some() && can_attack_seen(m, cx.w)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.next_sweep = reduced(10);
        m.extra.flag = false;
        Self::anchor_above_target(m, cx);
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(a) = m.extra.home {
            let top = cx.w.motion_blocking_height(a[0], a[2]);
            m.extra.home = Some([
                a[0],
                top + 10 + m.random.next_int_bound(20).unwrap_or(0),
                a[2],
            ]);
        }
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if !m.extra.flag {
            self.next_sweep -= 1;
            if self.next_sweep <= 0 {
                m.extra.flag = true;
                Self::anchor_above_target(m, cx);
                self.next_sweep = reduced((8 + m.random.next_int_bound(4).unwrap_or(0)) * 20);
                let pitch = 0.95 + m.random.next_float() * 0.1;
                cx.events.push(MobEvent::Sound(
                    "entity.phantom.swoop",
                    m.center(),
                    10.0,
                    pitch,
                ));
            }
        }
    }
}
/// `PhantomCircleAroundAnchorGoal`.
#[derive(Clone, Debug, Default)]
struct CircleAroundAnchor {
    angle: f32,
    distance: f32,
    height: f32,
    clockwise: f32,
}
impl CircleAroundAnchor {
    fn select_next(&mut self, m: &mut Mob) {
        let a = *m.extra.home.get_or_insert([
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ]);
        self.angle += self.clockwise * 15.0f32.to_radians();
        m.extra.vector = [
            f64::from(a[0]) + f64::from(self.distance * self.angle.mcos()),
            f64::from(a[1]) + f64::from(-4.0 + self.height),
            f64::from(a[2]) + f64::from(self.distance * self.angle.msin()),
        ];
    }
}
fn empty(w: &MobWorld, x: i32, y: i32, z: i32) -> bool {
    w.facts.get(w.world.block(x, y, z)).name.as_ref() == "minecraft:air"
}
impl Goal for CircleAroundAnchor {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.ai.target.is_none() || !m.extra.flag
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        self.distance = 5.0 + m.random.next_float() * 10.0;
        self.height = -4.0 + m.random.next_float() * 9.0;
        self.clockwise = if m.random.next_int_bound(2).unwrap_or(0) == 0 {
            1.0
        } else {
            -1.0
        };
        self.select_next(m);
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if m.random.next_int_bound(reduced(350)).unwrap_or(1) == 0 {
            self.height = -4.0 + m.random.next_float() * 9.0;
        }
        if m.random.next_int_bound(reduced(250)).unwrap_or(1) == 0 {
            self.distance += 1.0;
            if self.distance > 15.0 {
                self.distance = 5.0;
                self.clockwise = -self.clockwise;
            }
        }
        if m.random.next_int_bound(reduced(450)).unwrap_or(1) == 0 {
            self.angle = m.random.next_float() * 2.0 * std::f32::consts::PI;
            self.select_next(m);
        }
        let t = m.extra.vector;
        let d2 = (t[0] - m.position.x).powi(2)
            + (t[1] - m.position.y).powi(2)
            + (t[2] - m.position.z).powi(2);
        if d2 < 4.0 {
            self.select_next(m);
        }
        let (x, y, z) = (
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        );
        if m.extra.vector[1] < m.position.y && !empty(cx.w, x, y - 1, z) {
            self.height = self.height.max(1.0);
            self.select_next(m);
        }
        if m.extra.vector[1] > m.position.y && !empty(cx.w, x, y + 1, z) {
            self.height = self.height.min(-1.0);
            self.select_next(m);
        }
    }
}
/// `PhantomSweepAttackGoal`: dive at the target, bite on contact.
#[derive(Clone, Debug)]
struct SweepAttack {
    scared: bool,
    cat_search: u32,
}
impl Goal for SweepAttack {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.ai.target.is_some() && m.extra.flag
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !can_attack(cx.w) || m.ai.target.is_none() || !m.extra.flag {
            return false;
        }
        if m.age > self.cat_search {
            self.cat_search = m.age + 20;
            let cats: Vec<u32> =
                cx.w.nearby(m.position, 16.0, |s| s.species == Species::Cat && s.alive)
                    .map(|s| s.id)
                    .collect();
            for cat in &cats {
                cx.events.push(MobEvent::Hiss { mob: *cat });
            }
            self.scared = !cats.is_empty();
        }
        !self.scared
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.target = None;
        m.extra.flag = false;
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(who) = m.ai.target else {
            return;
        };
        let Some(t) = cx.w.target_info(who) else {
            return;
        };
        let mid = (t.bounds.min_position().y + t.bounds.max_position().y) * 0.5;
        m.extra.vector = [t.position.x, mid, t.position.z];
        let reach = m.bounds().inflate(Vec3::new(0.2, 0.2, 0.2));
        if reach.intersects(t.bounds) {
            super::ai::do_hurt_target(m, who, cx);
            m.extra.flag = false;
            // LevelEvent 1039: the bite.
            cx.events
                .push(MobEvent::Sound("entity.phantom.bite", m.center(), 1.0, 1.0));
        } else if m.horizontal_collision || m.hurt_time > 0 {
            m.extra.flag = false;
        }
    }
}
fn approach(from: f32, to: f32, step: f32) -> f32 {
    if from < to {
        (from + step).min(to)
    } else {
        (from - step).max(to)
    }
}
fn wrap(d: f32) -> f32 {
    let r = d.rem_euclid(360.0);
    if r >= 180.0 { r - 360.0 } else { r }
}
/// `PhantomMoveControl`: bank toward the move target, faster on a straight
/// line.
pub fn move_control(m: &mut Mob) {
    let mut speed = m.attrs.speed as f32;
    if m.horizontal_collision {
        m.yaw += 180.0;
        speed = 0.1;
    }
    let t = m.extra.vector;
    let (mut dx, dy, mut dz) = (
        t[0] - m.position.x,
        t[1] - m.position.y,
        t[2] - m.position.z,
    );
    let mut sd = (dx * dx + dz * dz).sqrt();
    if sd.abs() > 1.0e-5 {
        let scale = 1.0 - (dy * 0.7).abs() / sd;
        dx *= scale;
        dz *= scale;
        sd = (dx * dx + dz * dz).sqrt();
        let sd2 = (dx * dx + dz * dz + dy * dy).sqrt();
        let prev = m.yaw;
        let angle = dz.atan2(dx) as f32;
        let a = wrap(m.yaw + 90.0);
        let b = wrap(angle.to_degrees());
        // Mth.approachDegrees
        let diff = wrap(b - a);
        m.yaw = a + diff.clamp(-4.0, 4.0) - 90.0;
        m.body_yaw = m.yaw;
        if wrap(prev - m.yaw).abs() < 3.0 {
            speed = approach(speed, 1.8, 0.005 * (1.8 / speed));
        } else {
            speed = approach(speed, 0.2, 0.025);
        }
        let x_rot = -((-dy).atan2(sd) as f32).to_degrees();
        m.head_pitch = x_rot;
        let move_angle = (m.yaw + 90.0).to_radians();
        let tx = f64::from(speed * move_angle.mcos()) * (dx / sd2).abs();
        let tz = f64::from(speed * move_angle.msin()) * (dz / sd2).abs();
        let ty = f64::from(speed * x_rot.to_radians().msin()) * (dy / sd2).abs();
        let v = m.velocity;
        m.velocity = Vec3::new(
            v.x + (tx - v.x) * 0.2,
            v.y + (ty - v.y) * 0.2,
            v.z + (tz - v.z) * 0.2,
        );
    }
    m.attrs.speed = f64::from(speed);
    // PhantomBodyRotationControl: the head follows the body.
    m.head_yaw = m.body_yaw;
}
/// `PhantomModel.setupAnim` (flap time offset by the id).
pub fn anim(m: &Mob, s: &super::anim::State, p: &mut super::anim::Poses) {
    let flap = (m.id as f32 * 3.0 + s.age) * 7.448_451_f32.to_radians();
    let wing = flap.mcos() * 16.0f32.to_radians();
    p.zrot("left_wing_base", wing);
    p.zrot("left_wing_tip", wing);
    p.zrot("right_wing_base", -wing);
    p.zrot("right_wing_tip", -wing);
    let tail = -(5.0 + (flap * 2.0).mcos() * 5.0f32).to_radians();
    p.xrot("tail_base", tail);
    p.xrot("tail_tip", tail);
}
/// The flap sound when a wingbeat begins (`Phantom.isFlapping`).
pub fn flap_sound(m: &Mob, events: &mut Vec<MobEvent>) {
    let t = m.id as f32 * 3.0 + m.age as f32;
    let a = (t * 7.448_451_f32.to_radians() + std::f32::consts::PI).mcos();
    let b = ((t + 1.0) * 7.448_451_f32.to_radians() + std::f32::consts::PI).mcos();
    if a > 0.0 && b <= 0.0 {
        events.push(MobEvent::Sound(
            "entity.phantom.flap",
            m.center(),
            0.95 + 0.05,
            0.95,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::world::{test_player, with_built_test_world};

    /// `TargetingConditions.DEFAULT` checks line of sight (#4929): a phantom
    /// above takes a player in the open, not one boxed in stone or glass.
    #[test]
    fn a_phantom_cannot_target_a_player_it_cannot_see() {
        for roof in [None, Some("minecraft:stone"), Some("minecraft:glass")] {
            with_built_test_world(
                &[],
                |world, registry| {
                    for x in -1..=1 {
                        for z in -1..=1 {
                            world.chunks.insert(
                                (x, z),
                                std::sync::Arc::new(crate::chunk::Chunk::empty(x, z)),
                            );
                        }
                    }
                    let Some(block) = roof.and_then(|b| registry.default_state(b)) else {
                        return;
                    };
                    // A hollow 5x5x5 box around the player at (0.5, 64, 0.5).
                    for x in -2i32..=2 {
                        for y in 63..=67 {
                            for z in -2i32..=2 {
                                let shell = x.abs() == 2 || z.abs() == 2 || y == 63 || y == 67;
                                if shell {
                                    world.set(x, y, z, block).unwrap();
                                }
                            }
                        }
                    }
                },
                |w, types| {
                    let mut w = w.clone();
                    w.player = Some(test_player(Vec3::new(0.5, 64.0, 0.5)));
                    let kind = std::sync::Arc::new(types.types["minecraft:phantom"].clone());
                    let mut m = Mob::new(1, kind, Vec3::new(0.5, 80.0, 0.5), 0.0);
                    let mut target = AttackPlayerTarget { next_scan: 0 };
                    let mut events = Vec::new();
                    let mut lr = horizon_core::random::LegacyRandom::new(0);
                    let mut cx = Cx {
                        w: &w,
                        lr: &mut lr,
                        events: &mut events,
                    };
                    assert_eq!(target.can_use(&mut m, &mut cx), roof.is_none(), "{roof:?}");
                    m.ai.target = Some(Who::Player);
                    m.ai.seen_cache.clear();
                    assert_eq!(
                        target.can_continue(&mut m, &mut cx),
                        roof.is_none(),
                        "{roof:?}"
                    );
                },
            );
        }
    }
}
