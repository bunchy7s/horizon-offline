//! Sniffers (`Sniffer`, `SnifferAi`): the brain's core, idle, sniff and dig
//! activities, scenting and sniffing for diggable ground, walking there,
//! digging up a seed, the explored positions, laying eggs when bred, and
//! `SnifferModel`'s keyframe animations.
use super::{
    Mob, MobEvent,
    anim::{Poses, State},
    keyframes::Animation,
};
use std::collections::HashMap;

// Sniffer state: `variant` the state (below), `scratch[0]` the state the
// animations last saw, `positions` the explored positions (newest first),
// `ints[3]` the tick the dug seed drops. AnimState states: feeling happy,
// scenting, sniffing, digging, rising.
pub const IDLING: u8 = 0;
pub const FEELING_HAPPY: u8 = 1;
pub const SCENTING: u8 = 2;
pub const SNIFFING: u8 = 3;
pub const SEARCHING: u8 = 4;
pub const DIGGING: u8 = 5;
pub const RISING: u8 = 6;

/// `Sniffer.getBabyStartAge`.
pub const BABY_START_AGE: i32 = -48000;

pub fn state(m: &Mob) -> u8 {
    m.extra.variant
}
fn now(m: &Mob) -> i32 {
    m.age as i32
}
/// `Sniffer.getDefaultDimensions`: lower while digging.
pub fn digging(m: &Mob) -> bool {
    state(m) == DIGGING
}
/// `Sniffer.canPlayDiggingSound`.
pub fn can_play_digging_sound(m: &Mob) -> bool {
    matches!(state(m), DIGGING | SEARCHING)
}
pub fn register(m: &mut Mob) {
    use super::path::PathType;
    m.ai.nav.can_float = true;
    m.set_malus(PathType::Water, -1.0);
    m.set_malus(PathType::OnTopOfPowderSnow, -1.0);
    m.set_malus(PathType::DamageCautious, -1.0);
}
/// `Sniffer.setState`.
pub fn set_state(m: &mut Mob, s: u8) {
    m.extra.variant = s;
}
/// `Sniffer.getOnPos`: the block under its feet.
pub fn on_pos(m: &Mob) -> [i32; 3] {
    [
        m.position.x.floor() as i32,
        (m.position.y - f64::from(0.2_f32)).floor() as i32,
        m.position.z.floor() as i32,
    ]
}
/// `Sniffer.getHeadBlock`: 2.25 blocks along `getForward()`, at its feet
/// plus 0.2.
pub fn head_block(m: &Mob) -> [i32; 3] {
    use horizon_core::geometry::trig::Mth;
    // Vec3.directionFromRotation(xRot, yRot).
    let to_rad = std::f32::consts::PI / 180.0;
    let y = -m.yaw * to_rad - std::f32::consts::PI;
    let x = -m.head_pitch * to_rad;
    let (y_cos, y_sin) = (y.mcos(), y.msin());
    let x_cos = -x.mcos();
    let forward = [f64::from(y_sin * x_cos), f64::from(y_cos * x_cos)];
    [
        (m.position.x + forward[0] * 2.25).floor() as i32,
        (m.position.y + f64::from(0.2_f32)).floor() as i32,
        (m.position.z + forward[1] * 2.25).floor() as i32,
    ]
}
/// `Sniffer.tick` and `Sniffer.aiStep` additions: the animation states,
/// the dropped seed and the water malus.
pub fn ai_step(m: &mut Mob, events: &mut Vec<MobEvent>) {
    use super::path::PathType;
    let current = state(m);
    animate(m);
    if current == DIGGING && now(m) == m.extra.ints[3] {
        let h = head_block(m);
        events.push(MobEvent::GiftLoot {
            mob: m.id,
            table: "minecraft:gameplay/sniffer_digging",
            at: [f64::from(h[0]), f64::from(h[1]), f64::from(h[2])],
        });
        events.push(MobEvent::Sound(
            "entity.sniffer.drop_seed",
            m.center(),
            1.0,
            1.0,
        ));
    }
    // The searching sound is the client's (`playSearchingSound`).
    // onPathfindingStart / Done: water only blocks the way while it is dry.
    let wet = m.fluids.in_water || m.fire_ticks > 0;
    m.set_malus(PathType::Water, if wet { 0.0 } else { -1.0 });
}
/// `Sniffer.onSyncedDataUpdated(DATA_STATE)`: the animation states of the
/// state it changed to (on a client, each tick for the state it has).
pub fn animate(m: &mut Mob) {
    let age = m.age as f32;
    let current = state(m);
    if current != m.extra.scratch[0] as u8 {
        // resetAnimations, then the state's own.
        for s in &mut m.anim.states[..5] {
            s.stop();
        }
        match current {
            FEELING_HAPPY => m.anim.states[0].start_if_stopped(age),
            SCENTING => m.anim.states[1].start_if_stopped(age),
            SNIFFING => m.anim.states[2].start_if_stopped(age),
            DIGGING => m.anim.states[3].start_if_stopped(age),
            RISING => m.anim.states[4].start_if_stopped(age),
            _ => {}
        }
        m.extra.scratch[0] = i32::from(current);
    }
}
/// The brain memory holding the explored positions
/// (`SNIFFER_EXPLORED_POSITIONS`).
pub fn to_nbt(m: &Mob, map: &mut horizon_world::nbt::Compound) {
    use horizon_world::nbt::Tag;
    let list: Vec<Tag> = m
        .extra
        .positions
        .iter()
        .map(|p| {
            Tag::compound([
                ("dimension", Tag::String("minecraft:overworld".into())),
                ("pos", Tag::IntArray(p.to_vec())),
            ])
        })
        .collect();
    let mut memories = horizon_world::nbt::Compound::new();
    if !list.is_empty() {
        memories.insert(
            "minecraft:sniffer_explored_positions".into(),
            Tag::compound([("value", Tag::List(list))]),
        );
    }
    map.insert(
        "Brain".into(),
        Tag::compound([("memories", Tag::Compound(memories))]),
    );
}
pub fn from_nbt(m: &mut Mob, tag: &horizon_world::nbt::Tag) {
    use horizon_world::nbt::Tag;
    let Some(Tag::List(list)) = tag
        .get("Brain")
        .and_then(|b| b.get("memories"))
        .and_then(|b| b.get("minecraft:sniffer_explored_positions"))
        .and_then(|b| b.get("value"))
    else {
        return;
    };
    m.extra.positions = list
        .iter()
        .filter_map(|e| match e.get("pos") {
            Some(Tag::IntArray(p)) if p.len() == 3 => Some([p[0], p[1], p[2]]),
            _ => None,
        })
        .collect();
}
/// `Sniffer.canMate`'s state test.
pub fn can_mate_state(s: u8) -> bool {
    matches!(s, IDLING | SCENTING | FEELING_HAPPY)
}
/// Ambient sound: none while digging or searching.
pub fn ambient(m: &Mob) -> Option<&'static str> {
    (!matches!(state(m), DIGGING | SEARCHING)).then_some("entity.sniffer.idle")
}
/// `SnifferModel.setupAnim` (and `SniffletModel`'s baby transform).
pub fn anim(m: &Mob, s: &State, p: &mut Poses, animations: &HashMap<String, Animation>) {
    let get = |n: &str| animations.get(&format!("SnifferAnimation.{n}"));
    p.xrot("head", s.x_rot.to_radians());
    p.yrot("head", s.y_rot.to_radians());
    let walk = if state(m) == SEARCHING {
        "SNIFFER_SNIFF_SEARCH"
    } else {
        "SNIFFER_WALK"
    };
    if let Some(a) = get(walk) {
        a.apply_walk(s.pos, s.speed, 9.0, 100.0, p);
    }
    let age = s.age;
    for (st, name) in [
        (&m.anim.states[3], "SNIFFER_DIG"),
        (&m.anim.states[2], "SNIFFER_LONGSNIFF"),
        (&m.anim.states[4], "SNIFFER_STAND_UP"),
        (&m.anim.states[0], "SNIFFER_HAPPY"),
        (&m.anim.states[1], "SNIFFER_SNIFFSNIFF"),
    ] {
        if let (Some(ms), Some(a)) = (st.millis(age), get(name)) {
            a.apply(ms, 1.0, p);
        }
    }
    // The baby layer is SniffletModel geometry, but SnifferRenderer wraps it
    // in a plain SnifferModel, so SnifferAnimation.BABY_TRANSFORM never runs.
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_calm_states_can_mate() {
        assert!(can_mate_state(IDLING));
        assert!(can_mate_state(SCENTING));
        assert!(can_mate_state(FEELING_HAPPY));
        assert!(!can_mate_state(SNIFFING));
        assert!(!can_mate_state(DIGGING));
    }
}
