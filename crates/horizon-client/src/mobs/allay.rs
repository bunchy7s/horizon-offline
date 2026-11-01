//! Allays (`Allay`): given an item, they fetch matching drops within 32
//! blocks and bring them to their player (or a liked note block), dance to
//! jukeboxes and duplicate when given an amethyst shard mid-dance.
use super::{
    Mob, MobEvent, MobWorld,
    anim::{Poses, State as AnimState},
    control::MoveKind,
    goals::{Cx, Goal, LookAtPlayer, MOVE, Panic, RandomLookAround, Stroll, goal_clone},
    nav::NavKind,
};
use crate::items::ItemStack;
use crate::sculk::{self, Context, Info};
use horizon_core::geometry::trig::Mth;
use horizon_core::geometry::{Aabb, Vec3};

// Allay state: `main_hand` the held item, `ints[1]` how many it carries,
// `tame` a liked player, `point` the wanted item (set by the game),
// `State::noteblock` the liked note block with `ints[2]` its cooldown,
// `ints[3]` the pickup cooldown, `flag` dancing with `ints[4]` the dance clock,
// `egg_time` the duplication cooldown. AnimState slots: holding (0),
// spinning (1).
pub const HOLD: usize = 0;
pub const SPIN: usize = 1;
/// `Allay.VibrationUser.VIBRATION_EVENT_LISTENER_RANGE`.
const LISTENER_RADIUS: i64 = 16;

/// `AllayAi.TIME_TO_FORGET_NOTEBLOCK`.
const TIME_TO_FORGET_NOTEBLOCK: i32 = 600;
/// `Allay.MAX_NOTEBLOCK_DISTANCE`.
const MAX_NOTEBLOCK_DISTANCE: i32 = 1024;

/// `GlobalPos`: a block position in a dimension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlobalPos {
    pub dimension: &'static str,
    pub pos: [i32; 3],
}
impl GlobalPos {
    /// `GlobalPos.isCloseEnough`: the same dimension and at most
    /// `max_distance` blocks away on every axis (`Vec3i.distChessboard`).
    pub fn is_close_enough(&self, dimension: &str, pos: [i32; 3], max_distance: i32) -> bool {
        let chessboard = (0..3)
            .map(|i| self.pos[i].wrapping_sub(pos[i]).wrapping_abs())
            .max()
            .unwrap_or(0);
        self.dimension == dimension && chessboard <= max_distance
    }
}
/// The identifier of a dimension as a `'static` string, for loading saves.
pub fn dimension_id(id: &str) -> &'static str {
    match id {
        "minecraft:the_nether" => "minecraft:the_nether",
        "minecraft:the_end" => "minecraft:the_end",
        _ => "minecraft:overworld",
    }
}
/// `Level.dimension` of the level the mob ticks in.
pub fn dimension(w: &MobWorld) -> &'static str {
    if w.nether {
        "minecraft:the_nether"
    } else if w.end {
        "minecraft:the_end"
    } else {
        "minecraft:overworld"
    }
}

/// `AllayAi.allayConsidersItemEqual`: the same item (`ItemStack.isSameItem`)
/// and the same potion contents (`!hasNonMatchingPotion`); no
/// `potion_contents` on a stack is the empty contents, and two empty ones match.
// shortcut: only the potion id is compared, not custom effects or colour, which
// `StackData` does not keep; upgrade when it does.
pub fn considers_item_equal(held: &ItemStack, candidate: &ItemStack) -> bool {
    held.item == candidate.item && held.potion() == candidate.potion()
}

/// `Allay.setDancing` marks an allay dancing only under `isEffectiveAi()`, so
/// `NoAI` never dances; panicking stops it, and 10 blocks is the jukebox reach
/// (`distance_sq` to the nearest playing jukebox's centre, infinite for none).
pub fn dances(no_ai: bool, panicking: bool, distance_sq: f64) -> bool {
    !no_ai && !panicking && distance_sq <= 100.0
}

/// `Mob.getPickupReach` (1, 1, 1) around the box: `Mob.aiStep`'s looting loop
/// takes an equal drop from here with or without AI, so it is the only pickup a
/// `NoAI` allay has (`canPickUpLoot` and `wantsToPickUp` never ask for AI;
/// walking to a drop is the sensor's and throwing it is the brain's).
pub fn pickup_reach(bounds: Aabb) -> Aabb {
    bounds.inflate(Vec3::new(1.0, 1.0, 1.0))
}

/// An allay's vibration listener (`Allay.vibrationData`) and the brain
/// memory `LIKED_NOTEBLOCK_POSITION`; `LIKED_NOTEBLOCK_COOLDOWN_TICKS` is
/// `ints[2]`.
#[derive(Clone, Debug, Default)]
pub struct State {
    pub vibration: sculk::Data,
    pub noteblock: Option<GlobalPos>,
}

pub fn register(m: &mut Mob) {
    m.extra.allay = Some(Box::default());
    m.ai.mov.kind = MoveKind::Flying {
        max_turn: 20.0,
        hovers: true,
    };
    m.ai.nav.kind = NavKind::Flying;
    // FlyingPathNavigation.setRequiredPathLength(48).
    m.set_required_path_length(48.0);
    let g = &mut m.ai.goals;
    g.add(0, Panic::new(2.5));
    g.add(1, Fetch);
    g.add(2, Deliver);
    g.add(3, StayClose);
    g.add(4, Stroll::flying(1.0));
    g.add(5, LookAtPlayer::new(6.0));
    g.add(6, RandomLookAround::default());
}
/// `Entity.blockPosition`.
fn block_pos(m: &Mob) -> [i32; 3] {
    [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ]
}
/// `Allay.VibrationUser.canReceiveVibration`: not without AI; with no liked
/// note block any note will do, with one only that block's, while the allay
/// is within 1024 blocks of it in its dimension (`liked_noteblock_position`).
pub fn can_receive_vibration(m: &Mob, dimension: &str, pos: [i32; 3]) -> bool {
    if m.no_ai {
        return false;
    }
    let Some(liked_noteblock_position) = m.extra.allay.as_ref().and_then(|st| st.noteblock) else {
        return true;
    };
    liked_noteblock_position.is_close_enough(dimension, block_pos(m), MAX_NOTEBLOCK_DISTANCE)
        && liked_noteblock_position.pos == pos
}
/// `AllayAi.hearNoteblock`: an allay without a liked note block likes the
/// one it heard (`liked_noteblock_position`) and waits 600 ticks
/// (`liked_noteblock_cooldown_ticks`); one it already likes starts the wait over.
pub fn hear_noteblock(m: &mut Mob, dimension: &'static str, pos: [i32; 3]) {
    let heard = GlobalPos { dimension, pos };
    let Some(st) = m.extra.allay.as_mut() else {
        return;
    };
    let liked_noteblock_cooldown_ticks = TIME_TO_FORGET_NOTEBLOCK;
    let liked_noteblock_position = st.noteblock;
    match liked_noteblock_position {
        None => {
            st.noteblock = Some(heard);
            m.extra.ints[2] = liked_noteblock_cooldown_ticks;
        }
        Some(liked) if liked == heard => m.extra.ints[2] = liked_noteblock_cooldown_ticks,
        Some(_) => {}
    }
}
/// `Allay.VibrationUser.onReceiveVibration`: a note block's note is heard.
fn receive_vibration(m: &mut Mob, dimension: &'static str, info: Info) {
    if info.event == "minecraft:note_block_play" {
        hear_noteblock(m, dimension, info.pos.map(|v| v.floor() as i32));
    }
}
/// `AllayAi.getItemDepositPosition` for the liked note block: the centre of
/// the block above it while the allay is within 1024 blocks of it, it is
/// still a note block and the cooldown runs (`shouldDepositItemsAtLikedNoteblock`),
/// else the memory (`liked_noteblock_position`) is erased and the liked player
/// is the target.
pub fn noteblock_deposit(
    m: &mut Mob,
    dimension: &str,
    is_note_block: impl Fn([i32; 3]) -> bool,
) -> Option<[f64; 3]> {
    let liked = m.extra.allay.as_ref()?.noteblock?;
    if liked.is_close_enough(dimension, block_pos(m), MAX_NOTEBLOCK_DISTANCE)
        && is_note_block(liked.pos)
        && m.extra.ints[2] > 0
    {
        // `BlockPosTracker(pos.above())`: `Vec3.atCenterOf`.
        return Some([
            f64::from(liked.pos[0]) + 0.5,
            f64::from(liked.pos[1]) + 1.5,
            f64::from(liked.pos[2]) + 0.5,
        ]);
    }
    m.extra.allay.as_mut()?.noteblock = None;
    None
}
/// `AllayAi.getItemDepositPosition`: the liked note block, else the player
/// (`getLikedPlayerPositionTracker`).
fn deposit(m: &mut Mob, cx: &Cx) -> Option<[f64; 3]> {
    let dimension = dimension(cx.w);
    if let Some(p) = noteblock_deposit(m, dimension, |p| {
        cx.w.block_name(p[0], p[1], p[2]) == "minecraft:note_block"
    }) {
        return Some(p);
    }
    if !m.extra.tame {
        return None;
    }
    let p = cx.w.player.as_ref().filter(|p| p.alive && !p.spectator)?;
    (p.position.distance_squared(m.position) <= 64.0 * 64.0).then_some([
        p.position.x,
        p.position.y,
        p.position.z,
    ])
}
/// `GoToWantedItem`: fly to the wanted drop.
#[derive(Clone, Debug)]
struct Fetch;
impl Goal for Fetch {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        m.extra.point.is_some() && m.extra.ints[3] <= 0
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(p) = m.extra.point
            && (m.ai.nav.is_done() || m.age.is_multiple_of(10))
        {
            m.nav_move_to(cx.w, p, 1.75);
        }
    }
}
/// `GoAndGiveItemsToTarget`: carry the load to the deposit and throw it.
#[derive(Clone, Debug)]
struct Deliver;
impl Goal for Deliver {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.extra.ints[1] > 0 && m.extra.point.is_none() && deposit(m, cx).is_some()
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(p) = deposit(m, cx)
            && (m.ai.nav.is_done() || m.age.is_multiple_of(10))
        {
            m.nav_move_to(cx.w, p, 2.25);
        }
    }
}
/// `StayCloseToTarget.create(.., 4, 16, 2.25)`: acts only once the deposit is
/// 16 blocks away or more, then walks until within 4.
#[derive(Clone, Debug)]
struct StayClose;
impl Goal for StayClose {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.extra.point.is_none()
            && deposit(m, cx).is_some_and(|p| {
                let d = (p[0] - m.position.x).powi(2)
                    + (p[1] - m.position.y).powi(2)
                    + (p[2] - m.position.z).powi(2);
                d >= 256.0
            })
    }
    /// `MoveToTargetSink.reachedTarget`: the walk target it set is reached
    /// within a Manhattan block distance of 4 (`closeEnough`).
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.extra.point.is_none()
            && deposit(m, cx).is_some_and(|p| {
                let at = [m.position.x, m.position.y, m.position.z];
                (0..3)
                    .map(|i| (p[i].floor() as i32 - at[i].floor() as i32).abs())
                    .sum::<i32>()
                    > 4
            })
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(p) = deposit(m, cx)
            && (m.ai.nav.is_done() || m.age.is_multiple_of(10))
        {
            m.nav_move_to(cx.w, p, 2.25);
        }
    }
}
/// `VibrationSystem.Listener.handleGameEvent` for an allay: a free listener
/// within 16 blocks of its eyes schedules the note block notes it can hear
/// (`GameEventTags.ALLAY_CAN_LISTEN`). `occluded` is
/// `VibrationSystem.Listener.isOccluded`.
pub fn listen(
    m: &mut Mob,
    now: i64,
    event: &'static str,
    pos: [f64; 3],
    ctx: &Context,
    dimension: &str,
    occluded: impl Fn([f64; 3], [f64; 3]) -> bool,
) {
    let eye = [m.position.x, m.eye_y(), m.position.z];
    let Some(st) = m.extra.allay.as_ref() else {
        return;
    };
    if st.vibration.current.is_some() {
        return;
    }
    let block = |p: [f64; 3]| p.map(|v| i64::from(v.floor() as i32));
    let (source, listener) = (block(pos), block(eye));
    let d = [0, 1, 2].map(|i| source[i] - listener[i]);
    if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > LISTENER_RADIUS * LISTENER_RADIUS {
        return;
    }
    if !sculk::valid_vibration(sculk::allay_can_listen(event), event, ctx)
        || !can_receive_vibration(m, dimension, block(pos).map(|v| v as i32))
        || occluded(pos, eye)
    {
        return;
    }
    let d = [pos[0] - eye[0], pos[1] - eye[1], pos[2] - eye[2]];
    let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() as f32;
    if let Some(st) = m.extra.allay.as_mut() {
        st.vibration.schedule(
            Info {
                event,
                distance,
                pos,
                source: ctx.source,
                owner: ctx.owner,
            },
            now,
        );
    }
}
/// `VibrationSystem.Ticker.tick` for an allay, run after the living tick as
/// the server branch of its `tick` override does: it chooses a vibration, lets
/// it travel to the eyes at a block a tick (the particle starts there) and
/// hands it over on arrival.
pub fn tick(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    let tick = m
        .extra
        .allay
        .as_mut()
        .map(|st| {
            st.vibration
                .tick(w.game_time, true, false, |d| d.floor() as i32)
        })
        .unwrap_or_default();
    if let Some((info, travel)) = tick.started {
        events.push(MobEvent::Vibration {
            from: info.pos,
            to: [m.position.x, m.eye_y(), m.position.z],
            ticks: travel,
        });
    }
    if let Some(info) = tick.received {
        receive_vibration(m, dimension(w), info);
    }
}
/// `Allay.travel`: `travelFlying(input, getSpeed())` — the airborne speed is
/// the one the move control set this tick, and no gravity applies, so an
/// allay never falls under its own travel.
pub fn travel(m: &mut Mob, w: &MobWorld, input: Vec3, events: &mut Vec<MobEvent>) -> bool {
    let speed = m.ai.speed;
    m.travel_flying_speeds(w, input, events, 0.02, 0.02, speed);
    true
}
/// `Allay.aiStep` / `tick`: healing, cooldowns, the hold and spin clocks.
pub fn ai_step(m: &mut Mob) {
    if m.alive() && m.age.is_multiple_of(10) {
        m.health = (m.health + 1.0).min(m.attrs.max_health);
    }
    for i in [2, 3] {
        if m.extra.ints[i] > 0 {
            m.extra.ints[i] -= 1;
        }
    }
    if m.extra.egg_time > 0 {
        m.extra.egg_time -= 1;
    }
    animate(m);
}
/// `Allay.tick`'s client half: the hold and spin clocks (the server's and a
/// client's).
pub fn animate(m: &mut Mob) {
    let holding = m.extra.main_hand.is_some();
    let v = &mut m.anim.v;
    v[HOLD] = (v[HOLD] + if holding { 1.0 } else { -1.0 }).clamp(0.0, 5.0);
    if m.extra.flag {
        m.extra.ints[4] += 1;
        let spinning = (m.extra.ints[4] as f32 % 55.0) < 15.0;
        v[SPIN] = (v[SPIN] + if spinning { 1.0 } else { -1.0 }).clamp(0.0, 15.0);
    } else {
        m.extra.ints[4] = 0;
        v[SPIN] = 0.0;
        m.anim.vo[SPIN] = 0.0;
    }
}
/// `AllayModel.setupAnim`.
pub fn anim(m: &Mob, s: &AnimState, p: &mut Poses) {
    use std::f32::consts::PI;
    let (speed, pos) = (s.speed, s.pos);
    let flap_speed = (s.age * 20.0).to_radians() + pos;
    let flap = flap_speed.mcos() * PI * 0.15 + speed;
    let idle_bob = (s.age * 9.0).to_radians();
    let flying = (speed / 0.3).min(1.0);
    let idle = 1.0 - flying;
    let hold = m.anim.lerp(HOLD, s.partial) / 5.0;
    if m.extra.flag {
        let dance = (s.age * 8.0).to_radians() + speed;
        let freq = dance.mcos() * 16f32.to_radians();
        let spin = m.anim.lerp(SPIN, s.partial) / 15.0;
        let spinning = (m.extra.ints[4] as f32 % 55.0) < 15.0;
        if spinning {
            p.yrot("root", PI * 4.0 * spin);
        }
        p.zrot("root", freq * (1.0 - spin));
        p.yrot("head", dance.mcos() * 30f32.to_radians() * (1.0 - spin));
        p.zrot("head", dance.mcos() * 14f32.to_radians() * (1.0 - spin));
    } else {
        p.xrot("head", s.x_rot.to_radians());
        p.yrot("head", s.y_rot.to_radians());
    }
    p.xrot("right_wing", 0.436_332_32 * (1.0 - flying));
    p.yrot("right_wing", -PI / 4.0 + flap);
    p.xrot("left_wing", 0.436_332_32 * (1.0 - flying));
    p.yrot("left_wing", PI / 4.0 - flap);
    p.xrot("body", flying * PI / 4.0);
    let arm_x = hold * (-PI / 3.0 + (-1.134_464 + PI / 3.0) * flying);
    // `(float) Math.cos`: the exact function, not `Mth.cos`.
    p.get("root")[1] += (f64::from(idle_bob).cos() as f32) * 0.25 * idle;
    p.xrot("right_arm", arm_x);
    p.xrot("left_arm", arm_x);
    let arm_bob = idle * (1.0 - hold);
    let amount = 0.436_332_32 - (idle_bob + PI * 1.5).mcos() * PI * 0.075 * arm_bob;
    p.zrot("left_arm", -amount);
    p.zrot("right_arm", amount);
    p.yrot("right_arm", 0.279_252_68 * hold);
    p.yrot("left_arm", -0.279_252_68 * hold);
}
#[cfg(test)]
mod allay_parity_tests {
    use super::*;

    fn potion(id: &str) -> ItemStack {
        ItemStack::new("minecraft:potion", 1).with_potion(id)
    }

    #[test]
    fn a_held_potion_matches_only_its_own_contents() {
        assert!(!considers_item_equal(
            &potion("minecraft:swiftness"),
            &potion("minecraft:healing")
        ));
        assert!(considers_item_equal(
            &potion("minecraft:swiftness"),
            &potion("minecraft:swiftness")
        ));
    }

    #[test]
    fn stacks_without_potion_contents_match_by_item() {
        let stone = ItemStack::new("minecraft:stone", 1);
        assert!(considers_item_equal(&stone, &stone));
        assert!(!considers_item_equal(
            &stone,
            &ItemStack::new("minecraft:dirt", 1)
        ));
        // An empty potion component on both sides matches; one is not enough.
        assert!(considers_item_equal(
            &ItemStack::new("minecraft:potion", 1),
            &ItemStack::new("minecraft:potion", 1)
        ));
        assert!(!considers_item_equal(
            &ItemStack::new("minecraft:potion", 1),
            &potion("minecraft:swiftness")
        ));
    }

    #[test]
    fn a_no_ai_allay_does_not_dance_to_a_playing_jukebox() {
        // Beside a playing jukebox and at the edge of its 10-block reach.
        for distance_sq in [0.0, 1.5 * 1.5, 99.0, 100.0] {
            assert!(!dances(true, false, distance_sq), "NoAI at {distance_sq}");
            assert!(dances(false, false, distance_sq), "AI at {distance_sq}");
        }
        // Out of reach, or panicking: no allay dances.
        assert!(!dances(false, false, 100.5));
        assert!(!dances(false, true, 1.0));
    }

    #[test]
    fn a_no_ai_allay_takes_only_a_drop_already_in_pickup_reach() {
        // An allay at 0.5, 64, 0.5: 0.6 wide and tall.
        let bounds = Aabb::new(Vec3::new(0.2, 64.0, 0.2), Vec3::new(0.8, 64.6, 0.8));
        let reach = pickup_reach(bounds);
        // `Mob.aiStep`'s looting loop takes a drop at its feet, and one just
        // past the box, with no AI.
        assert!(reach.contains(Vec3::new(0.5, 64.0, 0.5)));
        assert!(reach.contains(Vec3::new(0.5, 64.0, 1.7)));
        // Past the one-block grow it would have to walk (`GoToWantedItem` is
        // the brain's), and 32 blocks is the sensor's reach, not this one.
        assert!(!reach.contains(Vec3::new(0.5, 64.0, 1.9)));
        assert!(!reach.contains(Vec3::new(0.5, 64.0, 20.0)));
        assert!(!reach.contains(Vec3::new(0.5, 66.0, 0.5)));
    }
}
