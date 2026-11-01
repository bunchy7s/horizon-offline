//! Game events and the vibration system (`GameEvent`, `VibrationSystem`,
//! `VibrationSelector`) shared by sculk sensors, shriekers and wardens, the
//! sculk block entities and the per-player warden spawn tracker.
pub use crate::chunk::block_entity::{Data, Info, SculkEntity, SculkKind, Selector};
use horizon_client_core::who::Who;
use horizon_core::{
    geometry::{Direction, Vec3, traverse_blocks},
    position::BlockPos,
};
use horizon_world::{level::sculk::SculkSpreader, nbt::Tag};

/// Every `GameEvent` with its `VibrationSystem` frequency (0: none).
const EVENTS: &[(&str, i32)] = &[
    ("minecraft:step", 1),
    ("minecraft:swim", 1),
    ("minecraft:flap", 1),
    ("minecraft:projectile_land", 2),
    ("minecraft:hit_ground", 2),
    ("minecraft:splash", 2),
    ("minecraft:bounce", 2),
    ("minecraft:item_interact_finish", 3),
    ("minecraft:projectile_shoot", 3),
    ("minecraft:instrument_play", 3),
    ("minecraft:entity_action", 4),
    ("minecraft:elytra_glide", 4),
    ("minecraft:unequip", 4),
    ("minecraft:entity_dismount", 5),
    ("minecraft:equip", 5),
    ("minecraft:entity_interact", 6),
    ("minecraft:shear", 6),
    ("minecraft:entity_mount", 6),
    ("minecraft:entity_damage", 7),
    ("minecraft:drink", 8),
    ("minecraft:eat", 8),
    ("minecraft:container_close", 9),
    ("minecraft:block_close", 9),
    ("minecraft:block_deactivate", 9),
    ("minecraft:block_detach", 9),
    ("minecraft:container_open", 10),
    ("minecraft:block_open", 10),
    ("minecraft:block_activate", 10),
    ("minecraft:block_attach", 10),
    ("minecraft:prime_fuse", 10),
    ("minecraft:note_block_play", 10),
    ("minecraft:block_change", 11),
    ("minecraft:block_destroy", 12),
    ("minecraft:fluid_pickup", 12),
    ("minecraft:block_place", 13),
    ("minecraft:fluid_place", 13),
    ("minecraft:entity_place", 14),
    ("minecraft:lightning_strike", 14),
    ("minecraft:teleport", 14),
    ("minecraft:entity_die", 15),
    ("minecraft:explode", 15),
    // Events that exist but make no vibration frequency.
    ("minecraft:item_interact_start", 0),
    ("minecraft:jukebox_play", 0),
    ("minecraft:jukebox_stop_play", 0),
    ("minecraft:sculk_sensor_tendrils_clicking", 0),
    ("minecraft:shriek", 0),
];
/// `GameEvent.RESONATE_1..15`.
pub const RESONANCE: [&str; 15] = [
    "minecraft:resonate_1",
    "minecraft:resonate_2",
    "minecraft:resonate_3",
    "minecraft:resonate_4",
    "minecraft:resonate_5",
    "minecraft:resonate_6",
    "minecraft:resonate_7",
    "minecraft:resonate_8",
    "minecraft:resonate_9",
    "minecraft:resonate_10",
    "minecraft:resonate_11",
    "minecraft:resonate_12",
    "minecraft:resonate_13",
    "minecraft:resonate_14",
    "minecraft:resonate_15",
];
pub const SHRIEK: &str = "minecraft:shriek";
pub const TENDRILS_CLICKING: &str = "minecraft:sculk_sensor_tendrils_clicking";

/// `VibrationSystem.getGameEventFrequency`.
pub fn frequency(event: &str) -> i32 {
    if let Some(i) = RESONANCE.iter().position(|e| *e == event) {
        return i as i32 + 1;
    }
    EVENTS
        .iter()
        .find(|(name, _)| *name == event)
        .map_or(0, |(_, f)| *f)
}
/// `GameEvent.notificationRadius`.
pub fn notification_radius(event: &str) -> i32 {
    match event {
        "minecraft:jukebox_play" | "minecraft:jukebox_stop_play" => 10,
        SHRIEK => 32,
        _ => 16,
    }
}
/// The identifier as a `'static` string, for loading saved events.
pub fn intern(event: &str) -> Option<&'static str> {
    RESONANCE
        .iter()
        .chain(EVENTS.iter().map(|(name, _)| name))
        .find(|e| **e == event)
        .copied()
}
/// `GameEventTags.VIBRATIONS`: every event with a frequency, resonance and flap.
pub fn is_vibration(event: &str) -> bool {
    frequency(event) != 0
}
/// `GameEventTags.SHRIEKER_CAN_LISTEN`.
pub fn shrieker_can_listen(event: &str) -> bool {
    event == TENDRILS_CLICKING
}
/// `GameEventTags.WARDEN_CAN_LISTEN`: vibrations except flap, plus shrieks and
/// tendril clicks.
pub fn warden_can_listen(event: &str) -> bool {
    (is_vibration(event) && event != "minecraft:flap")
        || event == SHRIEK
        || shrieker_can_listen(event)
}
/// `GameEventTags.ALLAY_CAN_LISTEN`: note block notes.
pub fn allay_can_listen(event: &str) -> bool {
    event == "minecraft:note_block_play"
}
/// `GameEventTags.IGNORE_VIBRATIONS_SNEAKING`.
pub fn ignored_when_sneaking(event: &str) -> bool {
    matches!(
        event,
        "minecraft:hit_ground"
            | "minecraft:projectile_shoot"
            | "minecraft:step"
            | "minecraft:swim"
            | "minecraft:item_interact_start"
            | "minecraft:item_interact_finish"
    )
}
/// `VibrationSystem.getRedstoneStrengthForDistance`.
pub fn redstone_strength(distance: f32, listener_radius: i32) -> i32 {
    let scale = 15.0 / f64::from(listener_radius);
    (15 - (scale * f64::from(distance)).floor() as i32).max(1)
}
/// `SculkSensorBlock.RESONANCE_PITCH_BEND`: `NoteBlock.getPitchFromNote`.
pub fn resonance_pitch(frequency: i32) -> f32 {
    const TONES: [i32; 16] = [0, 0, 2, 4, 6, 7, 9, 10, 12, 14, 15, 18, 19, 21, 22, 24];
    2.0f32.powf((TONES[frequency.clamp(0, 15) as usize] - 12) as f32 / 12.0)
}

/// What an event's source looks like to listeners, captured when it is
/// posted (`GameEvent.Context` plus the entity facts listeners ask for).
#[derive(Clone, Copy, Debug, Default)]
pub struct Context {
    pub source: Option<Who>,
    /// The projectile's owner, when the source is a projectile.
    pub owner: Option<Who>,
    /// `affectedState` is in `dampens_vibrations`.
    pub dampening_state: bool,
    /// The source is a spectator.
    pub spectator: bool,
    /// `isSteppingCarefully` (sneaking).
    pub stepping_carefully: bool,
    /// `dampensVibrations` (a warden).
    pub dampens: bool,
    /// The source is a living entity wardens may target.
    pub targetable: bool,
    /// The source is a living entity at all.
    pub living: bool,
    /// The source is the player, rides with them, or is their projectile
    /// (`SculkShriekerBlockEntity.tryGetPlayer`).
    pub player: bool,
}
/// `VibrationSystem.User.isValidVibration` without the tag test.
pub fn valid_vibration(listens: bool, event: &str, ctx: &Context) -> bool {
    if !listens {
        return false;
    }
    if ctx.source.is_some() {
        if ctx.spectator {
            return false;
        }
        if ctx.stepping_carefully && ignored_when_sneaking(event) {
            return false;
        }
        if ctx.dampens {
            return false;
        }
    }
    !ctx.dampening_state
}

impl Selector {
    pub fn add_candidate(&mut self, new: Info, tick: i64) {
        if self.should_replace(&new, tick) {
            self.current = Some((new, tick));
        }
    }
    fn should_replace(&self, new: &Info, tick: i64) -> bool {
        let Some((previous, previous_tick)) = &self.current else {
            return true;
        };
        if tick != *previous_tick {
            return false;
        }
        if new.distance < previous.distance {
            true
        } else if new.distance > previous.distance {
            false
        } else {
            frequency(new.event) > frequency(previous.event)
        }
    }
    pub fn chosen_candidate(&self, time: i64) -> Option<Info> {
        self.current
            .filter(|(_, tick)| *tick < time)
            .map(|(info, _)| info)
    }
    pub fn start_over(&mut self) {
        self.current = None;
    }
    /// The candidate kept so far and the tick it came in.
    pub fn current(&self) -> Option<(Info, i64)> {
        self.current
    }
}
/// What one `VibrationSystem.Ticker.tick` did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tick {
    /// A vibration was chosen: its travel time (the particle starts).
    pub started: Option<(Info, i32)>,
    /// A vibration arrived.
    pub received: Option<Info>,
    /// `onDataChanged`.
    pub changed: bool,
}
impl Data {
    /// `Listener.handleGameEvent` once the user accepted the event.
    pub fn schedule(&mut self, info: Info, game_time: i64) {
        self.selector.add_candidate(info, game_time);
    }
    /// `VibrationSystem.Ticker.tick`.
    pub fn tick(
        &mut self,
        game_time: i64,
        chunks_ticking: bool,
        needs_ticking_chunks: bool,
        travel_time: impl Fn(f32) -> i32,
    ) -> Tick {
        let mut out = Tick::default();
        if self.current.is_none()
            && let Some(info) = self.selector.chosen_candidate(game_time)
        {
            self.current = Some(info);
            self.travel = travel_time(info.distance);
            out.started = Some((info, self.travel));
            out.changed = true;
            self.selector.start_over();
        }
        if let Some(info) = self.current {
            let mut changed = self.travel > 0;
            self.travel = (self.travel - 1).max(0);
            if self.travel <= 0 {
                if needs_ticking_chunks && !chunks_ticking {
                    changed = false;
                } else {
                    out.received = Some(info);
                    self.current = None;
                    changed = true;
                }
            }
            out.changed |= changed;
        }
        out
    }
    /// The `listener` compound (`VibrationSystem.Data.CODEC`).
    pub fn to_nbt(&self) -> Tag {
        let mut entries: Vec<(&str, Tag)> = Vec::new();
        if let Some(info) = &self.current {
            entries.push(("event", info_to_nbt(info)));
        }
        let mut selector: Vec<(&str, Tag)> = Vec::new();
        let chosen = self.selector.current;
        if let Some((info, _)) = &chosen {
            selector.push(("event", info_to_nbt(info)));
        }
        selector.push(("tick", Tag::Long(chosen.map_or(-1, |(_, t)| t))));
        entries.push(("selector", Tag::compound(selector)));
        entries.push(("event_delay", Tag::Int(self.travel)));
        Tag::compound(entries)
    }
    pub fn from_nbt(tag: &Tag) -> Self {
        let mut data = Self {
            current: tag.get("event").and_then(info_from_nbt),
            ..Self::default()
        };
        if let Some(Tag::Int(t)) = tag.get("event_delay") {
            data.travel = (*t).max(0);
        }
        if let Some(selector) = tag.get("selector") {
            let info = selector.get("event").and_then(info_from_nbt);
            let tick = match selector.get("tick") {
                Some(Tag::Long(t)) => *t,
                _ => -1,
            };
            data.selector.current = info.map(|i| (i, tick));
        }
        data
    }
}
fn info_to_nbt(info: &Info) -> Tag {
    Tag::compound(vec![
        ("game_event", Tag::String(info.event.into())),
        ("distance", Tag::Float(info.distance.to_bits())),
        (
            "pos",
            Tag::List(info.pos.iter().map(|v| Tag::Double(v.to_bits())).collect()),
        ),
    ])
}
fn info_from_nbt(tag: &Tag) -> Option<Info> {
    let event = intern(&horizon_client_core::nbt_value::text(
        tag.get("game_event"),
    )?)?;
    let distance = match tag.get("distance") {
        Some(Tag::Float(f)) => f32::from_bits(*f),
        _ => 0.0,
    };
    let pos = match tag.get("pos") {
        Some(Tag::List(v)) if v.len() == 3 => {
            let mut p = [0.0; 3];
            for (i, t) in v.iter().enumerate() {
                if let Tag::Double(d) = t {
                    p[i] = f64::from_bits(*d);
                }
            }
            p
        }
        _ => return None,
    };
    Some(Info {
        event,
        distance,
        pos,
        source: None,
        owner: None,
    })
}

/// `Listener.isOccluded`: whether wool-like blocks block every line between
/// the centres of the source and listener blocks (nudged one direction each).
pub fn is_occluded(
    origin: [f64; 3],
    dest: [f64; 3],
    mut occludes: impl FnMut(BlockPos) -> bool,
) -> bool {
    let centre =
        |p: [f64; 3]| Vec3::new(p[0].floor() + 0.5, p[1].floor() + 0.5, p[2].floor() + 0.5);
    let from = centre(origin);
    let to = centre(dest);
    for direction in Direction::ALL {
        let nudged = from.relative(direction, f64::from(1.0e-5f32));
        let hit = traverse_blocks(nudged, to, 512, |p| occludes(p).then_some(()));
        if !matches!(hit, Ok(Some(()))) {
            return false;
        }
    }
    true
}

impl SculkEntity {
    pub fn new(kind: SculkKind) -> Self {
        Self {
            kind,
            vibration: Data::default(),
            last_frequency: 0,
            warning_level: 0,
            spreader: SculkSpreader::default(),
        }
    }
    /// Whether its block entity tick does nothing: no vibration chosen
    /// or on its way, none waiting to be chosen, and no charge to spread.
    pub fn idle(&self) -> bool {
        self.vibration.current.is_none()
            && self.vibration.selector.current().is_none()
            && self.spreader.cursors.is_empty()
    }
    pub fn for_block(name: &str) -> Option<Self> {
        Some(Self::new(match name {
            "minecraft:sculk_sensor" => SculkKind::Sensor,
            "minecraft:calibrated_sculk_sensor" => SculkKind::Calibrated,
            "minecraft:sculk_shrieker" => SculkKind::Shrieker,
            "minecraft:sculk_catalyst" => SculkKind::Catalyst,
            _ => return None,
        }))
    }
    pub fn id(&self) -> &'static str {
        match self.kind {
            SculkKind::Sensor => "minecraft:sculk_sensor",
            SculkKind::Calibrated => "minecraft:calibrated_sculk_sensor",
            SculkKind::Shrieker => "minecraft:sculk_shrieker",
            SculkKind::Catalyst => "minecraft:sculk_catalyst",
        }
    }
    /// `VibrationSystem.User.getListenerRadius`.
    pub fn listener_radius(&self) -> i32 {
        match self.kind {
            SculkKind::Calibrated => 16,
            SculkKind::Sensor | SculkKind::Shrieker | SculkKind::Catalyst => 8,
        }
    }
    /// The block entity's saved fields.
    pub fn to_nbt(&self) -> Vec<(&'static str, Tag)> {
        match self.kind {
            SculkKind::Sensor | SculkKind::Calibrated => vec![
                ("last_vibration_frequency", Tag::Int(self.last_frequency)),
                ("listener", self.vibration.to_nbt()),
            ],
            SculkKind::Shrieker => vec![
                ("warning_level", Tag::Int(self.warning_level)),
                ("listener", self.vibration.to_nbt()),
            ],
            SculkKind::Catalyst => vec![("cursors", cursors_to_nbt(&self.spreader))],
        }
    }
    pub fn load(&mut self, tag: &Tag) {
        match self.kind {
            SculkKind::Sensor | SculkKind::Calibrated => {
                if let Some(Tag::Int(f)) = tag.get("last_vibration_frequency") {
                    self.last_frequency = *f;
                }
                if let Some(l) = tag.get("listener") {
                    self.vibration = Data::from_nbt(l);
                }
            }
            SculkKind::Shrieker => {
                if let Some(Tag::Int(w)) = tag.get("warning_level") {
                    self.warning_level = *w;
                }
                if let Some(l) = tag.get("listener") {
                    self.vibration = Data::from_nbt(l);
                }
            }
            SculkKind::Catalyst => {
                if let Some(c) = tag.get("cursors") {
                    self.spreader = cursors_from_nbt(c);
                }
            }
        }
    }
}
fn cursors_to_nbt(spreader: &SculkSpreader) -> Tag {
    Tag::List(
        spreader
            .cursors
            .iter()
            .map(|c| {
                let mut e: Vec<(&str, Tag)> = vec![
                    ("pos", Tag::IntArray(vec![c.pos.x, c.pos.y, c.pos.z])),
                    ("charge", Tag::Int(c.charge)),
                    ("decay_delay", Tag::Int(c.decay_delay)),
                    ("update_delay", Tag::Int(c.update_delay)),
                ];
                if let Some(faces) = &c.facings {
                    e.push((
                        "facings",
                        Tag::List(
                            faces
                                .iter()
                                .map(|d| Tag::String(direction_name(*d).into()))
                                .collect(),
                        ),
                    ));
                }
                Tag::compound(e)
            })
            .collect(),
    )
}
fn cursors_from_nbt(tag: &Tag) -> SculkSpreader {
    use horizon_world::level::sculk::ChargeCursor;
    let mut spreader = SculkSpreader::default();
    let Tag::List(items) = tag else {
        return spreader;
    };
    for item in items.iter().take(32) {
        let Some(Tag::IntArray(p)) = item.get("pos") else {
            continue;
        };
        if p.len() != 3 {
            continue;
        }
        let int = |k: &str, default: i32| match item.get(k) {
            Some(Tag::Int(v)) => *v,
            _ => default,
        };
        let facings = match item.get("facings") {
            Some(Tag::List(v)) => Some(
                v.iter()
                    .filter_map(|t| match t {
                        Tag::String(s) => direction_from_name(&String::from_utf16_lossy(&s.0)),
                        _ => None,
                    })
                    .collect(),
            ),
            _ => None,
        };
        spreader.add_cursor(ChargeCursor {
            pos: BlockPos::new(p[0], p[1], p[2]),
            charge: int("charge", 0).clamp(0, 1000),
            update_delay: int("update_delay", 0).max(0),
            decay_delay: int("decay_delay", 1).clamp(0, 1),
            facings,
        });
    }
    spreader
}
fn direction_name(d: Direction) -> &'static str {
    match d {
        Direction::Down => "down",
        Direction::Up => "up",
        Direction::North => "north",
        Direction::South => "south",
        Direction::West => "west",
        Direction::East => "east",
    }
}
fn direction_from_name(s: &str) -> Option<Direction> {
    Direction::ALL.into_iter().find(|d| direction_name(*d) == s)
}

/// `WardenSpawnTracker`: a player's warning level toward summoning a warden.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WardenSpawnTracker {
    pub ticks_since_last_warning: i32,
    pub warning_level: i32,
    pub cooldown_ticks: i32,
}
impl WardenSpawnTracker {
    pub const MAX_WARNING_LEVEL: i32 = 4;
    const DECREASE_EVERY: i32 = 12000;
    const INCREASE_COOLDOWN: i32 = 200;
    pub fn tick(&mut self) {
        if self.ticks_since_last_warning >= Self::DECREASE_EVERY {
            self.set_warning_level(self.warning_level - 1);
            self.ticks_since_last_warning = 0;
        } else {
            self.ticks_since_last_warning += 1;
        }
        if self.cooldown_ticks > 0 {
            self.cooldown_ticks -= 1;
        }
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn on_cooldown(&self) -> bool {
        self.cooldown_ticks > 0
    }
    /// `increaseWarningLevel`.
    pub fn increase_warning_level(&mut self) {
        if !self.on_cooldown() {
            self.ticks_since_last_warning = 0;
            self.cooldown_ticks = Self::INCREASE_COOLDOWN;
            self.set_warning_level(self.warning_level + 1);
        }
    }
    pub fn set_warning_level(&mut self, level: i32) {
        self.warning_level = level.clamp(0, Self::MAX_WARNING_LEVEL);
    }
    /// `WardenSpawnTracker.tryWarn` over the trackers of the players around a
    /// shrieker (the triggering player included): the new warning level, or
    /// None when a warden is near or a player is cooling down. Every tracker
    /// takes over the highest one.
    pub fn try_warn(trackers: &mut [WardenSpawnTracker], warden_nearby: bool) -> Option<i32> {
        if warden_nearby || trackers.iter().any(|t| t.on_cooldown()) {
            return None;
        }
        // Stream.max keeps the last of equal maxima.
        let index = trackers
            .iter()
            .enumerate()
            .max_by_key(|(i, t)| (t.warning_level, *i))
            .map(|(i, _)| i)?;
        trackers[index].increase_warning_level();
        let copy = trackers[index];
        for t in trackers.iter_mut() {
            *t = copy;
        }
        Some(copy.warning_level)
    }
    pub fn to_nbt(&self) -> Tag {
        Tag::compound(vec![
            (
                "ticks_since_last_warning",
                Tag::Int(self.ticks_since_last_warning),
            ),
            ("warning_level", Tag::Int(self.warning_level)),
            ("cooldown_ticks", Tag::Int(self.cooldown_ticks)),
        ])
    }
    pub fn from_nbt(tag: &Tag) -> Self {
        let int = |k: &str| match tag.get(k) {
            Some(Tag::Int(v)) => (*v).max(0),
            _ => 0,
        };
        Self {
            ticks_since_last_warning: int("ticks_since_last_warning"),
            warning_level: int("warning_level").min(Self::MAX_WARNING_LEVEL),
            cooldown_ticks: int("cooldown_ticks"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(event: &'static str, distance: f32) -> Info {
        Info {
            event,
            distance,
            pos: [0.0; 3],
            source: None,
            owner: None,
        }
    }

    #[test]
    fn frequencies_match_the_vanilla_table() {
        assert_eq!(frequency("minecraft:step"), 1);
        assert_eq!(frequency("minecraft:hit_ground"), 2);
        assert_eq!(frequency("minecraft:projectile_shoot"), 3);
        assert_eq!(frequency("minecraft:entity_damage"), 7);
        assert_eq!(frequency("minecraft:eat"), 8);
        assert_eq!(frequency("minecraft:block_open"), 10);
        assert_eq!(frequency("minecraft:block_change"), 11);
        assert_eq!(frequency("minecraft:block_place"), 13);
        assert_eq!(frequency("minecraft:explode"), 15);
        assert_eq!(frequency("minecraft:resonate_9"), 9);
        assert_eq!(frequency("minecraft:shriek"), 0);
        assert_eq!(frequency(TENDRILS_CLICKING), 0);
        assert_eq!(frequency("minecraft:jukebox_play"), 0);
        assert_eq!(frequency("minecraft:made_up"), 0);
    }

    #[test]
    fn event_tables_cover_resonance() {
        for (i, name) in RESONANCE.iter().enumerate() {
            assert_eq!(frequency(name), i as i32 + 1, "{name}");
            assert!(is_vibration(name));
            assert!(warden_can_listen(name));
        }
        assert!(warden_can_listen(SHRIEK));
        assert!(warden_can_listen(TENDRILS_CLICKING));
        assert!(!warden_can_listen("minecraft:flap"));
        assert!(is_vibration("minecraft:flap"));
        assert!(shrieker_can_listen(TENDRILS_CLICKING));
        assert!(!shrieker_can_listen("minecraft:step"));
        assert_eq!(notification_radius(SHRIEK), 32);
        assert_eq!(notification_radius("minecraft:jukebox_play"), 10);
        assert_eq!(notification_radius("minecraft:step"), 16);
    }

    #[test]
    fn redstone_strength_falls_with_distance() {
        assert_eq!(redstone_strength(0.0, 8), 15);
        assert_eq!(redstone_strength(1.0, 8), 14);
        assert_eq!(redstone_strength(4.0, 8), 8);
        assert_eq!(redstone_strength(8.0, 8), 1);
        assert_eq!(redstone_strength(8.0, 16), 8);
        assert_eq!(redstone_strength(16.0, 16), 1);
    }

    #[test]
    fn selector_prefers_the_nearest_then_the_loudest() {
        let mut s = Selector::default();
        s.add_candidate(info("minecraft:step", 5.0), 10);
        s.add_candidate(info("minecraft:eat", 6.0), 10);
        assert_eq!(s.chosen_candidate(11).unwrap().event, "minecraft:step");
        s.add_candidate(info("minecraft:block_place", 5.0), 10);
        assert_eq!(
            s.chosen_candidate(11).unwrap().event,
            "minecraft:block_place"
        );
        s.add_candidate(info("minecraft:eat", 2.0), 10);
        assert_eq!(s.chosen_candidate(11).unwrap().event, "minecraft:eat");
        // A later tick never replaces the pending candidate.
        s.add_candidate(info("minecraft:explode", 0.0), 11);
        assert_eq!(s.chosen_candidate(11).unwrap().event, "minecraft:eat");
        // It is chosen only on a later tick than it was posted.
        assert!(s.chosen_candidate(10).is_none());
        s.start_over();
        assert!(s.chosen_candidate(12).is_none());
    }

    #[test]
    fn vibrations_travel_one_block_per_tick() {
        let mut d = Data::default();
        d.schedule(info("minecraft:step", 3.2), 100);
        let travel = |distance: f32| distance.floor() as i32;
        // Posted this tick: nothing yet.
        let t = d.tick(100, true, true, travel);
        assert_eq!(t, Tick::default());
        // Chosen next tick with a travel time of three.
        let t = d.tick(101, true, true, travel);
        assert_eq!(t.started.map(|(_, n)| n), Some(3));
        assert!(t.received.is_none());
        assert!(d.tick(102, true, true, travel).received.is_none());
        let t = d.tick(103, true, true, travel);
        assert_eq!(t.received.map(|i| i.event), Some("minecraft:step"));
        assert!(d.current.is_none());
    }

    #[test]
    fn nearby_vibrations_arrive_at_once() {
        let mut d = Data::default();
        d.schedule(info("minecraft:eat", 0.4), 5);
        let t = d.tick(6, true, true, |x| x.floor() as i32);
        assert!(t.started.is_some());
        assert!(t.received.is_some());
    }

    #[test]
    fn unloaded_neighbours_hold_a_vibration_back() {
        let mut d = Data::default();
        d.schedule(info("minecraft:eat", 0.0), 5);
        let t = d.tick(6, false, true, |_| 0);
        assert!(t.started.is_some());
        assert!(t.received.is_none());
        assert!(d.current.is_some());
        let t = d.tick(7, true, true, |_| 0);
        assert!(t.received.is_some());
        // Entities do not need the neighbours.
        d.schedule(info("minecraft:eat", 0.0), 8);
        assert!(d.tick(9, false, false, |_| 0).received.is_some());
    }

    #[test]
    fn vibration_data_round_trips_through_nbt() {
        let mut d = Data::default();
        d.schedule(info("minecraft:block_destroy", 4.0), 40);
        d.tick(41, true, true, |x| x as i32);
        d.schedule(info("minecraft:step", 2.0), 41);
        let back = Data::from_nbt(&d.to_nbt());
        assert_eq!(back.travel, d.travel);
        assert_eq!(back.current.map(|i| i.event), d.current.map(|i| i.event));
        assert_eq!(back.selector.current.map(|(_, t)| t), Some(41));
    }

    #[test]
    fn wool_on_the_line_occludes() {
        let origin = [0.5, 0.5, 0.5];
        let dest = [4.5, 0.5, 0.5];
        // A full wall of wool between them blocks all six nudged lines.
        let wall = |p: BlockPos| p.x == 2;
        assert!(is_occluded(origin, dest, wall));
        // One wool block on the line blocks the nudged lines too.
        let single = |p: BlockPos| p == BlockPos::new(2, 0, 0);
        assert!(is_occluded(origin, dest, single));
        // Wool off to the side lets the vibration through.
        let aside = |p: BlockPos| p == BlockPos::new(2, 1, 0);
        assert!(!is_occluded(origin, dest, aside));
        assert!(!is_occluded(origin, dest, |_| false));
    }

    #[test]
    fn sneaking_ignores_footsteps_but_not_damage() {
        let sneaking = Context {
            source: Some(Who::Player),
            stepping_carefully: true,
            ..Context::default()
        };
        assert!(!valid_vibration(true, "minecraft:step", &sneaking));
        assert!(!valid_vibration(true, "minecraft:hit_ground", &sneaking));
        assert!(valid_vibration(true, "minecraft:entity_damage", &sneaking));
        let wardenish = Context {
            source: Some(Who::Mob(1)),
            dampens: true,
            ..Context::default()
        };
        assert!(!valid_vibration(
            true,
            "minecraft:entity_damage",
            &wardenish
        ));
        let on_wool = Context {
            dampening_state: true,
            ..Context::default()
        };
        assert!(!valid_vibration(true, "minecraft:block_place", &on_wool));
        assert!(!valid_vibration(
            false,
            "minecraft:step",
            &Context::default()
        ));
    }

    #[test]
    fn resonance_pitch_follows_the_tone_map() {
        assert!((resonance_pitch(0) - 0.5).abs() < 1e-6);
        assert!((resonance_pitch(4) - 2.0f32.powf(-6.0 / 12.0)).abs() < 1e-6);
        assert!((resonance_pitch(15) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn warning_levels_rise_with_a_cooldown_and_decay() {
        let mut t = WardenSpawnTracker::default();
        t.increase_warning_level();
        assert_eq!(t.warning_level, 1);
        assert_eq!(t.cooldown_ticks, 200);
        // Cooling down: no increase.
        t.increase_warning_level();
        assert_eq!(t.warning_level, 1);
        for _ in 0..200 {
            t.tick();
        }
        t.increase_warning_level();
        assert_eq!(t.warning_level, 2);
        t.set_warning_level(9);
        assert_eq!(t.warning_level, 4);
        t.cooldown_ticks = 0;
        t.ticks_since_last_warning = 12000;
        t.tick();
        assert_eq!(t.warning_level, 3);
        assert_eq!(t.ticks_since_last_warning, 0);
    }

    #[test]
    fn nearby_players_share_the_highest_warning() {
        let mut trackers = [
            WardenSpawnTracker::default(),
            WardenSpawnTracker {
                warning_level: 2,
                ..Default::default()
            },
        ];
        assert_eq!(WardenSpawnTracker::try_warn(&mut trackers, true), None);
        assert_eq!(WardenSpawnTracker::try_warn(&mut trackers, false), Some(3));
        assert_eq!(trackers[0], trackers[1]);
        assert_eq!(trackers[0].cooldown_ticks, 200);
        // Everyone is cooling down now.
        assert_eq!(WardenSpawnTracker::try_warn(&mut trackers, false), None);
    }

    #[test]
    fn cursors_round_trip() {
        let mut e = SculkEntity::new(SculkKind::Catalyst);
        e.spreader.add_cursors(BlockPos::new(3, 4, 5), 1500);
        e.spreader.cursors[1].facings = Some(vec![Direction::Up, Direction::East]);
        let tag = Tag::compound(e.to_nbt());
        let mut back = SculkEntity::new(SculkKind::Catalyst);
        back.load(&tag);
        assert_eq!(back.spreader, e.spreader);
        assert_eq!(back.spreader.cursors[0].charge, 1000);
        assert_eq!(back.spreader.cursors[1].charge, 500);
    }
}
