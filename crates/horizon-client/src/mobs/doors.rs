//! Doors a mob gets through on its own: `DoorInteractGoal` with its two
//! users, `BreakDoorGoal` (zombies that can break doors, vindicators in a
//! raid) and `OpenDoorGoal` (`AbstractIllager.RaiderOpenDoorGoal`).
use super::{
    Mob, MobEvent, MobWorld, Species,
    goals::{Cx, Goal, MOVE, goal_clone, reduced},
    nav::NavKind,
    raider,
};
use horizon_core::{geometry::Vec3, random::RandomSource};

/// `DefaultDoorBreakTime`: `BreakDoorGoal.getDoorBreakTime` is
/// `max(240, doorBreakTime)`, and the vindicator's 6 is below it.
const DOOR_BREAK_TIME: i32 = 240;

/// `GoalUtils.hasGroundPathNavigation`: ground, wall-climbing and
/// amphibious navigations walk.
pub fn can_navigate_ground(m: &Mob) -> bool {
    matches!(
        m.ai.nav.kind,
        NavKind::Ground | NavKind::WallClimber | NavKind::Amphibious { .. }
    )
}
/// `Zombie.setCanBreakDoors`: only a mob that walks can break doors, and
/// then its navigation opens them.
pub fn set_can_break_doors(m: &mut Mob, can_break: bool) {
    let can_break = can_break && can_navigate_ground(m);
    m.extra.can_break_doors = can_break;
    m.ai.nav.can_open_doors = can_break;
}
/// The species that extend `Zombie` (`CanBreakDoors` is theirs to save).
pub fn is_zombie(s: Species) -> bool {
    matches!(
        s,
        Species::Zombie
            | Species::Husk
            | Species::Drowned
            | Species::ZombieVillager
            | Species::ZombifiedPiglin
    )
}
/// `DoorBlock.isWoodenDoor`: a door a hand opens, which is any but iron.
pub fn wooden_door(w: &MobWorld, p: [i32; 3]) -> bool {
    let name = w.block_name(p[0], p[1], p[2]);
    name.ends_with("_door") && name != "minecraft:iron_door"
}
/// `DoorInteractGoal`'s door: where it is and whether one is there.
#[derive(Clone, Debug, Default)]
struct Interact {
    door: [i32; 3],
    has_door: bool,
}
impl Interact {
    /// `DoorInteractGoal.canUse`: the mob bumped into something and a
    /// wooden door is on its path within 1.5 blocks, or at its own head.
    fn can_use(&mut self, m: &Mob, w: &MobWorld) -> bool {
        if !can_navigate_ground(m) || !m.horizontal_collision {
            return false;
        }
        let Some(path) = m.ai.nav.path.as_ref().filter(|p| !p.is_done()) else {
            return false;
        };
        for node in path
            .nodes
            .iter()
            .take((path.next + 2).min(path.nodes.len()))
        {
            self.door = [node.x, node.y + 1, node.z];
            let dx = m.position.x - f64::from(node.x);
            let dz = m.position.z - f64::from(node.z);
            if dx * dx + dz * dz <= 2.25 {
                self.has_door = wooden_door(w, self.door);
                if self.has_door {
                    return true;
                }
            }
        }
        let at = m.position;
        self.door = [
            at.x.floor() as i32,
            at.y.floor() as i32 + 1,
            at.z.floor() as i32,
        ];
        self.has_door = wooden_door(w, self.door);
        self.has_door
    }
    /// `DoorInteractGoal.isOpen`: the door is still there and open.
    fn is_open(&mut self, w: &MobWorld) -> bool {
        if !self.has_door {
            return false;
        }
        let d = self.door;
        if !w.block_name(d[0], d[1], d[2]).ends_with("_door") {
            self.has_door = false;
            return false;
        }
        w.prop(d[0], d[1], d[2], "open") == Some("true")
    }
    /// `DoorInteractGoal.setOpen`.
    fn set_open(&self, cx: &mut Cx, open: bool) {
        let d = self.door;
        if self.has_door && cx.w.block_name(d[0], d[1], d[2]).ends_with("_door") {
            cx.events.push(MobEvent::SetDoorOpen { pos: d, open });
        }
    }
}
/// `BlockPos.closerToCenterThan`.
pub(super) fn closer_to_center(p: [i32; 3], at: Vec3, distance: f64) -> bool {
    Vec3::new(
        f64::from(p[0]) + 0.5,
        f64::from(p[1]) + 0.5,
        f64::from(p[2]) + 0.5,
    )
    .distance_squared(at)
        < distance * distance
}

/// `BreakDoorGoal`: crack the door in the way for 240 ticks, then remove it.
/// A zombie does it on hard only; `Vindicator.VindicatorBreakDoorGoal` on
/// normal and hard, in a raid, and holds the mob's movement.
#[derive(Clone, Debug)]
pub struct BreakDoor {
    interact: Interact,
    vindicator: bool,
    break_time: i32,
    last_break_progress: i32,
}
impl BreakDoor {
    /// `Zombie.breakDoorGoal`, added when the zombie can break doors.
    pub fn zombie() -> Self {
        Self::new(false)
    }
    /// `Vindicator.VindicatorBreakDoorGoal`.
    pub fn vindicator() -> Self {
        Self::new(true)
    }
    fn new(vindicator: bool) -> Self {
        Self {
            interact: Interact::default(),
            vindicator,
            break_time: 0,
            last_break_progress: -1,
        }
    }
    /// `DOOR_BREAKING_PREDICATE`.
    fn valid_difficulty(&self, w: &MobWorld) -> bool {
        if self.vindicator {
            w.difficulty.normal || w.difficulty.hard
        } else {
            w.difficulty.hard
        }
    }
    /// `BreakDoorGoal.canUse` after `DoorInteractGoal.canUse`.
    fn can_break(&mut self, m: &Mob, w: &MobWorld) -> bool {
        self.interact.can_use(m, w)
            && w.mob_griefing
            && self.valid_difficulty(w)
            && !self.interact.is_open(w)
    }
}
impl Goal for BreakDoor {
    goal_clone!();
    fn flags(&self) -> u8 {
        if self.vindicator { MOVE } else { 0 }
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if self.vindicator {
            return raider::has_active_raid(m)
                && m.random.next_int_bound(reduced(10)).unwrap_or(1) == 0
                && self.can_break(m, cx.w);
        }
        m.extra.can_break_doors && self.can_break(m, cx.w)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        (!self.vindicator || raider::has_active_raid(m))
            && self.break_time <= DOOR_BREAK_TIME
            && !self.interact.is_open(cx.w)
            && closer_to_center(self.interact.door, m.position, 2.0)
            && self.valid_difficulty(cx.w)
    }
    fn start(&mut self, m: &mut Mob, _cx: &mut Cx) {
        self.break_time = 0;
        if self.vindicator {
            m.ai.no_action_time = 0;
        }
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        cx.events.push(MobEvent::DestroyProgress {
            mob: m.id,
            pos: self.interact.door,
            progress: -1,
        });
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let door = self.interact.door;
        if m.random.next_int_bound(20).unwrap_or(1) == 0 {
            cx.events.push(MobEvent::LevelEvent {
                id: 1019,
                pos: door,
                data: 0,
            });
            if !m.ai.swinging {
                m.swing();
            }
        }
        self.break_time += 1;
        let progress = (self.break_time as f32 / DOOR_BREAK_TIME as f32 * 10.0) as i32;
        if progress != self.last_break_progress {
            cx.events.push(MobEvent::DestroyProgress {
                mob: m.id,
                pos: door,
                progress,
            });
            self.last_break_progress = progress;
        }
        if self.break_time == DOOR_BREAK_TIME && self.valid_difficulty(cx.w) {
            cx.events.push(MobEvent::BreakDoor { pos: door });
        }
    }
}

/// `AbstractIllager.RaiderOpenDoorGoal` (`OpenDoorGoal` that does not wait
/// to close): open the door in the way during a raid. It never continues, so
/// it closes the door again as soon as it stops.
#[derive(Clone, Debug, Default)]
pub struct RaiderOpenDoor {
    interact: Interact,
}
impl Goal for RaiderOpenDoor {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.interact.can_use(m, cx.w) && raider::has_active_raid(m)
    }
    fn can_continue(&mut self, _m: &mut Mob, _cx: &mut Cx) -> bool {
        false
    }
    fn start(&mut self, _m: &mut Mob, cx: &mut Cx) {
        self.interact.set_open(cx, true);
    }
    fn stop(&mut self, _m: &mut Mob, cx: &mut Cx) {
        self.interact.set_open(cx, false);
    }
}

/// `InteractWithDoor.create()`, the piglin brain's core behaviour: while it
/// follows a path, open the doors it is leaving and entering and close the
/// ones it opened or passed through (within 3 blocks, and not the two of the
/// step it takes). It takes no flags and runs every tick, as a core behaviour
/// does. (Another piglin coming through a door is not looked for.)
#[derive(Clone, Debug, Default)]
pub struct InteractWithDoor {
    /// `lastCheckedNode` and `remainingCooldown`.
    last_checked: Option<[i32; 3]>,
    cooldown: i32,
    /// `DOORS_TO_CLOSE`.
    doors: Vec<[i32; 3]>,
}
impl InteractWithDoor {
    /// A door mobs open (`MOB_INTERACTABLE_DOORS`) at `p`, and whether it is open.
    fn door_open(w: &MobWorld, p: [i32; 3]) -> Option<bool> {
        let name = w.block_name(p[0], p[1], p[2]);
        (name.ends_with("_door") && w.has_tag("minecraft:mob_interactable_doors", name))
            .then(|| w.prop(p[0], p[1], p[2], "open") == Some("true"))
    }
    /// `closeDoorsThatIHaveOpenedOrPassedThrough`.
    fn close_doors(&mut self, m: &Mob, cx: &mut Cx, from: [i32; 3], to: [i32; 3]) {
        let doors = std::mem::take(&mut self.doors);
        for door in doors {
            if from == door || to == door {
                self.doors.push(door);
                continue;
            }
            if !closer_to_center(door, m.position, 3.0) || Self::door_open(cx.w, door) != Some(true)
            {
                continue;
            }
            cx.events.push(MobEvent::SetDoorOpen {
                pos: door,
                open: false,
            });
        }
    }
}
impl Goal for InteractWithDoor {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        m.ai.nav
            .path
            .as_ref()
            .is_some_and(|p| p.next > 0 && !p.is_done())
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.can_use(m, cx)
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(path) =
            m.ai.nav
                .path
                .as_ref()
                .filter(|p| p.next > 0 && !p.is_done())
        else {
            return;
        };
        let node = |i: usize| [path.nodes[i].x, path.nodes[i].y, path.nodes[i].z];
        let (from, to) = (node(path.next - 1), node(path.next));
        if self.last_checked == Some(to) {
            self.cooldown = 20;
        } else {
            self.cooldown -= 1;
            if self.cooldown > 0 {
                return;
            }
        }
        self.last_checked = Some(to);
        // The door being left is remembered even when it is open already; the
        // one ahead only when this mob opens it.
        for (p, always) in [(from, true), (to, false)] {
            let Some(open) = Self::door_open(cx.w, p) else {
                continue;
            };
            if !open {
                cx.events.push(MobEvent::SetDoorOpen { pos: p, open: true });
            }
            if (always || !open) && !self.doors.contains(&p) {
                self.doors.push(p);
            }
        }
        self.close_doors(m, cx, from, to);
    }
}
