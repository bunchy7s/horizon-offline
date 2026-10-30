//! The wandering trader (`WanderingTrader`): drinking invisibility at night
//! and milk by day, fleeing monsters, walking to the place it came to
//! visit, staying within its home, trading and leaving after its despawn
//! delay.
use super::{
    Mob, MobEvent, MobWorld, Species,
    goals::{
        Avoid, AvoidEntity, Cx, Float, Goal, JUMP, LOOK, LookAtPlayer, MOVE, Panic, Stroll,
        goal_clone,
    },
};
use horizon_core::random::RandomSource;

// Trader state: `point` the wander target, `home` the place it is tied to
// with `ints[4]` the radius, `ints[5]` the despawn delay, `flag2` whether
// a player is trading with it and `using_item` how long it has been
// drinking.

/// `WanderingTraderSpawner`: how long a spawned trader stays.
pub const DESPAWN_DELAY: i32 = 48000;
const DRINK_TICKS: i32 = 32;
const INVISIBILITY: &str = "minecraft:invisibility";

pub fn register(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(
        0,
        DrinkItem {
            item: "minecraft:potion",
            finish_sound: "entity.wandering_trader.disappeared",
            dark: true,
        },
    );
    g.add(
        0,
        DrinkItem {
            item: "minecraft:milk_bucket",
            finish_sound: "entity.wandering_trader.reappeared",
            dark: false,
        },
    );
    g.add(1, TradeWithPlayer);
    for (species, distance) in [
        (Species::Zombie, 8.0),
        (Species::Evoker, 12.0),
        (Species::Vindicator, 8.0),
        (Species::Vex, 8.0),
        (Species::Pillager, 15.0),
        (Species::Illusioner, 12.0),
        (Species::Zoglin, 10.0),
    ] {
        // `Zombie.class` covers the zombie family.
        let list = if species == Species::Zombie {
            vec![
                Species::Zombie,
                Species::Husk,
                Species::Drowned,
                Species::ZombieVillager,
                Species::ZombifiedPiglin,
            ]
        } else {
            vec![species]
        };
        g.add(
            1,
            AvoidEntity::new(Avoid::Species(list), distance, 0.5, 0.5),
        );
    }
    g.add(1, Panic::new(0.5));
    g.add(1, LookAtTradingPlayer);
    g.add(
        2,
        WanderToPosition {
            stop: 2.0,
            speed: 0.35,
        },
    );
    g.add(4, MoveTowardsRestriction::new(0.35));
    g.add(8, Stroll::avoiding_water(0.35));
    g.add(9, LookAtPlayer::interact(3.0, 1.0));
    g.add(10, LookAtPlayer::mobs(8.0));
}
fn invisible(m: &Mob) -> bool {
    m.extra.effects.iter().any(|e| e.0 == INVISIBILITY)
}
pub fn is_invisible(m: &Mob) -> bool {
    invisible(m)
}
/// `UseItemGoal`: drink a potion of invisibility once it is dark outside,
/// or milk to clear it by day.
#[derive(Clone, Debug)]
struct DrinkItem {
    item: &'static str,
    finish_sound: &'static str,
    dark: bool,
}
impl Goal for DrinkItem {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if cx.w.nether {
            return false;
        }
        if self.dark {
            !cx.w.daylight && !invisible(m)
        } else {
            cx.w.daylight && invisible(m)
        }
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.extra.using_item > 0
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.extra.set_main_hand(Some(self.item.into()));
        m.extra.using_item = 1;
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.extra.set_main_hand(None);
        m.extra.using_item = 0;
        let pitch = m.random.next_float() * 0.2 + 0.9;
        cx.events
            .push(MobEvent::Sound(self.finish_sound, m.center(), 1.0, pitch));
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if m.extra.using_item == 0 {
            return;
        }
        m.extra.using_item += 1;
        if m.extra.using_item > DRINK_TICKS {
            // LivingEntity.completeUsingItem: the potion or the milk takes effect.
            if self.dark {
                m.extra.effects.retain(|e| e.0 != INVISIBILITY);
                m.extra.effects.push((INVISIBILITY, 3600, 0));
                cx.events.push(MobEvent::Sound(
                    "entity.wandering_trader.drink_potion",
                    m.center(),
                    1.0,
                    1.0,
                ));
            } else {
                m.extra.effects.clear();
                cx.events.push(MobEvent::Sound(
                    "entity.wandering_trader.drink_milk",
                    m.center(),
                    1.0,
                    1.0,
                ));
            }
            m.extra.using_item = 0;
        }
    }
}
/// `TradeWithPlayerGoal`: stand still while trading.
#[derive(Clone, Debug)]
struct TradeWithPlayer;
impl Goal for TradeWithPlayer {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | JUMP
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.extra.flag2 && m.alive() && !m.fluids.in_water && m.on_ground
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
    }
}
/// `LookAtTradingPlayerGoal`.
#[derive(Clone, Debug)]
struct LookAtTradingPlayer;
impl Goal for LookAtTradingPlayer {
    goal_clone!();
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.extra.flag2 && cx.w.player.is_some()
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(p) = &cx.w.player {
            m.look_at([p.position.x, p.eye[1], p.position.z]);
        }
    }
}
/// `WanderingTrader.WanderToPositionGoal`: head for the wander target, in
/// hops of ten blocks while it is far.
#[derive(Clone, Debug)]
struct WanderToPosition {
    stop: f64,
    speed: f64,
}
fn too_far(m: &Mob, p: [f64; 3], distance: f64) -> bool {
    // BlockPos.closerToCenterThan.
    let (x, y, z) = (p[0].floor() + 0.5, p[1].floor() + 0.5, p[2].floor() + 0.5);
    let d = (x - m.position.x).powi(2) + (y - m.position.y).powi(2) + (z - m.position.z).powi(2);
    d >= distance * distance
}
impl Goal for WanderToPosition {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.extra.point.is_some_and(|p| too_far(m, p, self.stop))
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.extra.point = None;
        m.nav_stop();
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(p) = m.extra.point else { return };
        if !m.ai.nav.is_done() {
            return;
        }
        if too_far(m, p, 10.0) {
            let d = [
                p[0].floor() - m.position.x,
                p[1].floor() - m.position.y,
                p[2].floor() - m.position.z,
            ];
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-9);
            let at = [
                d[0] / len * 10.0 + m.position.x,
                d[1] / len * 10.0 + m.position.y,
                d[2] / len * 10.0 + m.position.z,
            ];
            m.nav_move_to(cx.w, at, self.speed);
        } else {
            m.nav_move_to(cx.w, [p[0].floor(), p[1].floor(), p[2].floor()], self.speed);
        }
    }
}
/// `Mob.isWithinHome`.
pub fn within_home(m: &Mob) -> bool {
    let Some(h) = m.extra.home else { return true };
    let r = m.extra.ints[4];
    if r <= 0 {
        return true;
    }
    let d = (h[0] - m.position.x.floor() as i32).pow(2)
        + (h[1] - m.position.y.floor() as i32).pow(2)
        + (h[2] - m.position.z.floor() as i32).pow(2);
    d < r * r
}
/// `MoveTowardsRestrictionGoal`.
#[derive(Clone, Debug)]
struct MoveTowardsRestriction {
    speed: f64,
    wanted: [f64; 3],
}
impl MoveTowardsRestriction {
    fn new(speed: f64) -> Self {
        Self {
            speed,
            wanted: [0.0; 3],
        }
    }
}
impl Goal for MoveTowardsRestriction {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if within_home(m) {
            return false;
        }
        let Some(h) = m.extra.home else { return false };
        let to = [
            f64::from(h[0]) + 0.5,
            f64::from(h[1]),
            f64::from(h[2]) + 0.5,
        ];
        match m.default_random_pos_towards(cx.w, 16, 7, to, std::f64::consts::FRAC_PI_2) {
            Some(p) => {
                self.wanted = p;
                true
            }
            None => false,
        }
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        !m.ai.nav.is_done()
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.nav_move_to(cx.w, self.wanted, self.speed);
    }
}
/// `WanderingTrader.maybeDespawn` in `aiStep`.
pub fn ai_step(m: &mut Mob, _w: &MobWorld) {
    if m.extra.ints[5] > 0 && !m.extra.flag2 {
        m.extra.ints[5] -= 1;
        if m.extra.ints[5] == 0 {
            m.removed = true;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn despawn_delay_is_two_days() {
        assert_eq!(DESPAWN_DELAY, 2 * 24000);
    }
}
