//! The movement prediction's replay (docs/anticheat.md): each client tick a player's record holds
//! ([`horizon_anticheat::prediction::Record`]) is played again with the client's own movement code,
//! from where the last one ended, and compared with where the client said it went.
//!
//! The inputs are known exactly: a vanilla client sends ServerboundPlayerInputPacket after its tick's
//! aiStep (auto-jump included), its sprinting just before the move, and its item use from the key
//! handling before the tick. So nothing about them is guessed: a candidate that sprinted or skipped
//! the item-use slowdown where the client said otherwise would let speed and no-slowdown cheats
//! through. What the server can't know is tried, every combination of it, the tick as heard first,
//! keeping the closest outcome: whether the client stood on something on the first tick after a
//! start (a teleport), and on the tick where the server's item use changed, the use as it was before
//! (the server ends a use on its own tick, the client on its own).
//!
//! The replay then goes on from where the client said it is, so a small error never builds up, and a
//! client that lies a little each tick is measured against its own last position.
//!
//! A tick that lands farther than the check's tolerance counts against the player, unless the server
//! excuses it ([`Replay::excuse`]: something the replay can't play yet was near), and then the ticks
//! after it too, until the moves agree again or [`DRIFT`] ticks have gone by: a push the replay missed
//! leaves the client's velocity off for a few ticks.

use std::collections::VecDeque;

use horizon_anticheat::prediction::{Change, ClientTick, KEPT, Record, Restart};
use horizon_core::geometry::Vec3;

use crate::motion::PlayerMotion;
use crate::sprint::Keys;

/// A move shorter than this isn't sent (LocalPlayer.sendPosition: 2.0E-4).
const UNSENT: f64 = 2.0e-4;

/// Below this the replay and the client agree (float noise).
pub const EXACT: f64 = 1e-6;

/// The most ticks an excuse covers after the tick it was for.
pub const DRIFT: u8 = 40;

/// The most a single tick weighs, in tolerances.
const MOST_WEIGHT: f64 = 5.0;

/// Ticks after the server's item use ends during which the client may still be slowed by it: half a
/// second. The client finishes a use on its own ticks and the server on its own (food eaten one after
/// another with the key held), and the server stops one for its own reasons (a shield disabled):
/// the client's ends a few ticks later, more with an uneven ping. Slower than its keys allow is no
/// advantage; the other way round (a use the server has and the client may not) would let a no-slow
/// walk, so a start counts at once, as the client's use packet comes before its tick's move.
const USE_WINDOW: u8 = 10;

/// The ticks of a catch-up that landed farther than the tolerance, none of them excused.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Over {
    pub ticks: u32,
    /// The farthest of them, in blocks.
    pub worst: f64,
    /// Each one's offset in tolerances, at most 5 a tick, added up.
    pub weight: f32,
    /// The farthest tick, as played: what the log tells of it.
    pub at_worst: Option<Played>,
    /// One of them came with a move.
    pub moved: bool,
}

/// Ticks within the tolerance in a row after which a replay that lands off takes the client's
/// velocity ([`Replay::meet`]): half a second.
const STEADY: u8 = 10;

/// The most velocity a replay takes from the client at once, in blocks a tick (a jump is 0.42).
const MOST_MET: f64 = 0.5;

/// Below this the moves agree again after an excuse: within the tolerance is not enough, as the
/// replay's velocity is still being brought to the client's.
const AGREED: f64 = 1e-3;

/// Whether the block at `block` is near enough a player at `at` to change its next moves, `slack`
/// blocks farther for a fast one (see [`slack`]).
pub fn near(at: [f64; 3], block: [i32; 3], slack: f64) -> bool {
    let d = |i: usize| f64::from(block[i]) + 0.5 - at[i];
    let r = 4.5 + slack;
    d(0).abs() <= r && d(2).abs() <= r && (-4.5 - slack..=6.5 + slack).contains(&d(1))
}

/// How much farther to look around a player that moves `moved` in a tick: two ticks of it, as the
/// server's view of what is around it (a firework flying with it) is a tick or so behind.
pub fn slack(moved: [f64; 3]) -> f64 {
    (2.0 * moved.iter().map(|v| v * v).sum::<f64>().sqrt()).min(16.0)
}

/// One client tick played again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Played {
    /// How far the closest outcome ended from where the client said.
    pub offset: f64,
    /// Which of the unknowns it took: 1, the ground the other way (the first tick after a start); 2,
    /// the item use before the server's change; 4, the jump delay the other way (a jump held soon
    /// after an excuse). 0 is the tick as the server heard it.
    pub candidate: u8,
    /// The replay and the client agree about the ground (the move's flag 1).
    pub ground: bool,
    /// Where the client said it went less where the replay ended, in blocks.
    pub off: [f64; 3],
    /// How far the client said it went in the tick.
    pub moved: f64,
    /// What the replayed body was doing when the tick ended ([`Played::state`]).
    pub state: u8,
    /// The tick's keys (ServerboundPlayerInputPacket's flags).
    pub input: u8,
    /// Where the client said it ended the tick.
    pub at: [f64; 3],
    /// The replay's velocity at the end of the tick, in blocks a tick.
    pub velocity: [f64; 3],
}

impl Played {
    /// Bits of [`Played::state`].
    pub const GROUND: u8 = 1;
    pub const WATER: u8 = 2;
    pub const GLIDING: u8 = 4;
    pub const SPRINTING: u8 = 8;
    pub const SNEAKING: u8 = 16;
    pub const SWIMMING: u8 = 32;
    pub const FLYING: u8 = 64;
    pub const RIDING: u8 = 128;

    /// What a log line says of the tick, so a flag can be found again in the world and played:
    /// `off +0.000 +0.173 +0.000, moved 0.281, air sprinting, keys forward jump, at 12.30 64.00
    /// -5.50, velocity +0.000 -0.078 +0.000`.
    pub fn describe(&self) -> String {
        let names = [
            (Self::WATER, "water"),
            (Self::GLIDING, "gliding"),
            (Self::SPRINTING, "sprinting"),
            (Self::SNEAKING, "sneaking"),
            (Self::SWIMMING, "swimming"),
            (Self::FLYING, "flying"),
            (Self::RIDING, "riding"),
        ];
        let mut state = String::from(if self.state & Self::GROUND != 0 {
            "ground"
        } else {
            "air"
        });
        for (bit, name) in names {
            if self.state & bit != 0 {
                state.push(' ');
                state.push_str(name);
            }
        }
        let [x, y, z] = self.off;
        let keys = [
            "forward", "backward", "left", "right", "jump", "shift", "sprint",
        ]
        .into_iter()
        .enumerate()
        .filter(|&(bit, _)| self.input & (1 << bit) != 0)
        .map(|(_, name)| name)
        .collect::<Vec<_>>();
        let keys = if keys.is_empty() {
            "none".to_owned()
        } else {
            keys.join(" ")
        };
        let [ax, ay, az] = self.at;
        let [vx, vy, vz] = self.velocity;
        format!(
            "off {x:+.3} {y:+.3} {z:+.3}, moved {:.3}, {state}, keys {keys}, at {ax:.2} {ay:.2} \
             {az:.2}, velocity {vx:+.3} {vy:+.3} {vz:+.3}",
            self.moved
        )
    }
}

/// A player as its client moves it.
#[derive(Clone)]
pub struct Replay {
    /// The player at the end of the last tick played.
    body: Box<PlayerMotion>,
    /// Where the client last said it was.
    said: Vec3,
    /// Where the client was when its moves last agreed (or were excused): where a setback sends it.
    agreed: Vec3,
    /// Ticks an excuse still covers.
    drift: u8,
    /// The record's ticks played so far.
    played: u64,
    /// The record's server changes applied so far.
    applied: u64,
    /// No tick played since the start: whether the client stood on something is unknown.
    fresh: bool,
    /// The server's item-use slowdown at the last tick played.
    using: Option<f32>,
    /// The one it had before its last change, and the ticks left in which the client may still be
    /// slowed by it after it ended ([`USE_WINDOW`]).
    used: Option<f32>,
    use_window: u8,
    /// Ticks the client's jump delay is unknown: under an excuse the replay may have jumped where the
    /// client couldn't (on a block it hadn't received), or the other way round.
    jump_unsure: u8,
    /// The last tick played: whether the client said it sprinted, and whether its keys pushed it
    /// forward (for [`Replay::sprints`]).
    claimed: bool,
    forward: bool,
    /// Ticks in a row within the tolerance, at most [`STEADY`].
    steady: u8,
    /// The velocity the client's last tick left it with, less the replay's.
    missed: Vec3,
    /// The next tick also tries this velocity added ([`Replay::meet`]).
    meeting: Option<Vec3>,
    /// The last ticks played, oldest first, at most [`KEPT`].
    pub ticks: VecDeque<Played>,
}

impl Replay {
    /// A replay that starts at a teleport the client accepted, standing still, with the ticks and
    /// changes the record heard after it. `server` is the server's own copy of the player.
    pub fn new(server: &PlayerMotion, restart: &Restart) -> Self {
        let mut body = Box::new(server.clone());
        let at = Vec3::new(restart.at[0], restart.at[1], restart.at[2]);
        (body.position, body.old_position, body.velocity) = (at, at, Vec3::ZERO);
        body.fall_distance = 0.0;
        body.events.clear();
        // The server's body carries the sprint modifier while the player sprints, but its flag only
        // while the game handles the player: the replay's first tick sprints as the client did.
        let _ = body.set_sprint_modifier(body.sprinting);
        Self {
            body,
            said: at,
            agreed: at,
            drift: 0,
            played: restart.ended,
            applied: restart.changed,
            fresh: true,
            using: server.item_use_speed,
            used: server.item_use_speed,
            use_window: 0,
            jump_unsure: 0,
            claimed: false,
            forward: false,
            steady: STEADY,
            missed: Vec3::ZERO,
            meeting: None,
            ticks: VecDeque::new(),
        }
    }

    /// Goes on from another teleport the client accepted: what the client keeps through it (its jump
    /// delay, a glide) stays as the replay had it; it stands still at the new place.
    pub fn restart(&mut self, restart: &Restart) {
        let at = Vec3::new(restart.at[0], restart.at[1], restart.at[2]);
        let body = &mut self.body;
        (body.position, body.old_position, body.velocity) = (at, at, Vec3::ZERO);
        body.fall_distance = 0.0;
        (self.said, self.agreed, self.drift) = (at, at, 0);
        (self.played, self.applied, self.fresh) = (restart.ended, restart.changed, true);
        self.steady = STEADY;
    }

    /// Plays the record's ticks it hasn't, each after what the server had changed by then (the
    /// changes of the server ticks the client had seen when the tick ended), and returns those that
    /// landed farther than `tolerance`. None when the record no longer keeps them, or a tick couldn't
    /// be played: the replay starts again at a teleport.
    pub fn catch_up(
        &mut self,
        record: &Record,
        server: &PlayerMotion,
        tolerance: f64,
        run: impl FnMut(&mut PlayerMotion, Keys, bool) -> crate::Result<()>,
    ) -> Option<Over> {
        self.play_to(record, server, record.ended(), tolerance, run)
    }

    /// Plays the ticks the client ended before it accepted a teleport, which the replay hasn't yet
    /// (a burst of them came with the accept): what they did to the body (its jump delay) goes on
    /// through the teleport. Then applies the changes the server sent before the teleport that those
    /// ticks hadn't seen: the client had them all when it accepted it. Nothing counts. False when the
    /// record no longer keeps them.
    pub fn finish(
        &mut self,
        record: &Record,
        server: &PlayerMotion,
        restart: &Restart,
        run: impl FnMut(&mut PlayerMotion, Keys, bool) -> crate::Result<()>,
    ) -> bool {
        if self
            .play_to(record, server, restart.ended, f64::INFINITY, run)
            .is_none()
        {
            return false;
        }
        let Some(changes) = record.changes_since(self.applied) else {
            return false;
        };
        let left = usize::try_from(restart.changed.saturating_sub(self.applied)).unwrap_or(0);
        for &(_, change) in changes.take(left) {
            self.apply(change, server);
            self.applied += 1;
        }
        true
    }

    fn play_to(
        &mut self,
        record: &Record,
        server: &PlayerMotion,
        ended: u64,
        tolerance: f64,
        mut run: impl FnMut(&mut PlayerMotion, Keys, bool) -> crate::Result<()>,
    ) -> Option<Over> {
        let mut over = Over::default();
        // A tick of this catch-up already counted: the client has been displaced by its lie, so a
        // tick that agrees from there must not replace the setback point (#5132).
        let mut counted = false;
        let mut changes = record.changes_since(self.applied)?.peekable();
        let new = usize::try_from(ended.saturating_sub(self.played)).ok()?;
        for tick in record.since(self.played)?.take(new) {
            while let Some(&&(sent, change)) = changes.peek()
                && tick.seen.is_some_and(|seen| sent <= seen)
            {
                self.apply(change, server);
                self.applied += 1;
                changes.next();
            }
            let played = self.play(tick, server, &mut run)?;
            let offset = played.offset;
            // A tick that agrees only with the velocity the last one left: that tick lied, or slipped
            // (Replay::meet), and the setback point stays before it.
            let met = played.candidate & 8 != 0;
            let steady = self.steady;
            self.steady = if offset <= tolerance {
                steady.saturating_add(1).min(STEADY)
            } else {
                0
            };
            // The server changed the player's abilities, or something near it, after what the
            // client had seen: the tick had the new ones where the client had the old.
            let unseen = || {
                let at = [self.said.x, self.said.y, self.said.z];
                let v = self.body.velocity;
                let slack = slack([v.x, v.y, v.z]);
                changes
                    .clone()
                    .any(|&(_, change)| change == Change::Abilities)
                    || record.unseen_near(tick.seen, |block| near(at, block, slack))
            };
            if offset <= AGREED {
                if !counted && !met {
                    self.agreed = self.said;
                }
                self.drift = 0;
            } else if offset <= tolerance {
                if !counted && !met {
                    self.agreed = self.said;
                }
                self.drift = self.drift.saturating_sub(1);
            } else if self.drift == 0 && unseen() {
                self.excuse();
            } else if self.drift > 0 {
                self.drift -= 1;
                self.agreed = self.said;
            } else {
                counted = true;
                over.ticks += 1;
                over.moved |= tick.position.is_some();
                if offset > over.worst {
                    over.at_worst = self.ticks.back().copied();
                }
                over.worst = over.worst.max(offset);
                over.weight += (offset / tolerance).min(MOST_WEIGHT) as f32;
                if steady == STEADY {
                    self.meet();
                }
            }
        }
        Some(over)
    }

    /// The ticks just caught up landed off for a reason the replay can't play yet: they count for
    /// nothing, and neither do the ones after until the moves agree again (at most [`DRIFT`]).
    pub fn excuse(&mut self) {
        (self.agreed, self.drift, self.jump_unsure) = (self.said, DRIFT, DRIFT);
    }

    /// The first tick off after the moves agreed for a while: the next tick also tries the velocity
    /// the client was left with (up to [`MOST_MET`], across only), and keeps it if it lands closer. A
    /// single slip (a hit the replay missed) then counts once, not on every tick its velocity
    /// takes to die out; a single lie about the position (a blink) leaves the velocity as it was. A
    /// client that lies on every tick is off on every tick all the same: nothing is tried again until
    /// the moves have agreed for [`STEADY`] ticks.
    fn meet(&mut self) {
        let m = self.missed;
        let length = (m.x * m.x + m.y * m.y + m.z * m.z).sqrt();
        let scale = if length > MOST_MET {
            MOST_MET / length
        } else {
            1.0
        };
        self.meeting = Some(Vec3::new(m.x * scale, m.y * scale, m.z * scale));
    }

    /// Whether an excuse still covers the ticks.
    pub fn excused(&self) -> bool {
        self.drift > 0
    }

    /// Where the client was when its moves last agreed.
    pub fn agreed(&self) -> [f64; 3] {
        [self.agreed.x, self.agreed.y, self.agreed.z]
    }

    /// The client handling a change the server sent.
    fn apply(&mut self, change: Change, server: &PlayerMotion) {
        let body = &mut self.body;
        let vector = |v: [f64; 3]| Vec3::new(v[0], v[1], v[2]);
        match change {
            Change::Velocity(v) => body.velocity = vector(v),
            Change::Knockback(k) => body.velocity = body.velocity.add_vector(vector(k)),
            Change::Attributes { sprint } => {
                // ClientPacketListener.handleUpdateAttributes: the client takes the modifiers of
                // the attributes sent, the sprint modifier included, and goes on sprinting or not
                // as it was (a sprint modifier gone while it sprints stays gone until it stops);
                // an attribute not sent keeps its own.
                let sprint = sprint.unwrap_or_else(|| body.has_sprint_modifier());
                body.attributes = server.attributes.clone();
                let _ = body.set_sprint_modifier(sprint);
            }
            // ClientPacketListener.handleSetEntityData: the flag alone. A sprint the tick goes on
            // with is then started again (`MotionWorld::replayed_tick`), its modifier with it, as
            // LocalPlayer.aiStep does when the sprint key is held ([`Replay::sprints`]).
            Change::Sprinting(sprinting) => body.sprinting = sprinting,
            Change::Effects => body.status = server.status.clone(),
            // `sync` takes them every tick.
            Change::Abilities => {}
        }
    }

    /// Whether the client sprints in `tick`: as it said (ServerboundPlayerCommandPacket), but for a
    /// sprint the server stopped (its flag, [`Change::Sprinting`]) that the client says goes on. That
    /// client never said STOP: its LocalPlayer.aiStep started the sprint again, which takes the sprint
    /// key held, or forward pressed again (a double tap). Without either it would have stopped and
    /// said so: one that sprints on is played walking, until it holds the key or says STOP.
    fn sprints(&mut self, tick: &ClientTick, keys: Keys) -> bool {
        let restarted = tick.sprinting && self.claimed && !self.body.sprinting;
        self.claimed = tick.sprinting;
        let forward = std::mem::replace(&mut self.forward, keys.direction()[1] > 1.0e-5);
        tick.sprinting && (!restarted || keys.sprint || !forward)
    }

    /// Plays `tick` again. `server` is the server's copy of the player, for what the server decides
    /// (abilities, effects, attributes); `run` moves a body one client tick with these keys and
    /// sprinting (`MotionWorld::replayed_tick`). None when no candidate could be played.
    fn play(
        &mut self,
        tick: &ClientTick,
        server: &PlayerMotion,
        run: &mut impl FnMut(&mut PlayerMotion, Keys, bool) -> crate::Result<()>,
    ) -> Option<Played> {
        self.played += 1;
        sync(&mut self.body, server);
        self.body.yaw = tick.yaw;
        self.body.head_yaw = tick.yaw;
        self.body.pitch = tick.pitch;
        // The first tick after a start guesses the ground from the client's move; the replay finds
        // it after that.
        let fresh = std::mem::take(&mut self.fresh);
        if fresh {
            self.body.ground.on_ground = tick.flags.is_some_and(|f| f & 1 != 0);
        }
        let using = std::mem::replace(&mut self.using, server.item_use_speed);
        if using != server.item_use_speed {
            (self.used, self.use_window) = (using, USE_WINDOW);
        } else {
            self.use_window = self.use_window.saturating_sub(1);
        }
        let used = self.used;
        let use_unsure = using != server.item_use_speed
            || self.use_window > 0 && server.item_use_speed.is_none() && used.is_some();
        let said = tick.position.map(|p| Vec3::new(p[0], p[1], p[2]));
        let keys = keys(tick.input);
        let sprinting = self.sprints(tick, keys);
        let last = self.said;
        let offset = |body: &PlayerMotion| match said {
            Some(p) => body.position.distance(p),
            // No move: the client is within 2.0E-4 of where it last said.
            None => (body.position.distance(last) - UNSENT).max(0.0),
        };
        let meeting = self.meeting.take();
        let jump_unsure = self.jump_unsure > 0 && keys.jump;
        self.jump_unsure = self.jump_unsure.saturating_sub(1);
        let unknown = |candidate: u8| {
            candidate & 1 != 0 && fresh
                || candidate & 2 != 0 && use_unsure
                || candidate & 4 != 0 && jump_unsure
                || candidate & 8 != 0 && meeting.is_some()
        };
        let (offset, candidate) = if ![1, 2, 4, 8].into_iter().any(unknown) {
            // Nothing to try (most ticks): the body itself plays the tick.
            run(&mut self.body, keys, sprinting).ok()?;
            (offset(&self.body), 0)
        } else {
            let mut best: Option<(f64, u8, Box<PlayerMotion>)> = None;
            for candidate in (0..16u8)
                .filter(|&c| c == 0 || [1, 2, 4, 8].into_iter().all(|b| c & b == 0 || unknown(b)))
            {
                let mut body = self.body.clone();
                if candidate & 1 != 0 {
                    body.ground.on_ground = !body.ground.on_ground;
                }
                if candidate & 2 != 0 {
                    body.item_use_speed = used;
                }
                if candidate & 4 != 0 {
                    body.no_jump_delay = i32::from(body.no_jump_delay == 0);
                }
                if let Some(met) = meeting.filter(|_| candidate & 8 != 0) {
                    body.velocity = body.velocity.add_vector(met);
                }
                if run(&mut body, keys, sprinting).is_err() {
                    continue;
                }
                let off = offset(&body);
                if best.as_ref().is_none_or(|b| off < b.0) {
                    best = Some((off, candidate, body));
                }
                if off <= EXACT {
                    break;
                }
            }
            let (off, candidate, body) = best?;
            self.body = body;
            (off, candidate)
        };
        let body = &mut self.body;
        let end = said.unwrap_or(last);
        // The replay's own velocity, before an excuse brings it to the client's.
        let velocity = body.velocity;
        let off = [
            end.x - body.position.x,
            end.y - body.position.y,
            end.z - body.position.z,
        ];
        let moved = said.map_or(0.0, |p| p.distance(last));
        // The velocity the client's tick left it with, less the replay's, across: the move it missed,
        // slowed as the replay's own move was each way (its drag or friction). Not up and down: there
        // the drag (0.98) is so slight that a client rising a little each tick on its own (a slow
        // fly) would look like one that slipped once for a second or more.
        let slowed = |missed: f64, moved: f64, left: f64| {
            if moved.abs() > 1e-3 {
                missed * (left / moved).clamp(0.0, 1.0)
            } else {
                missed * 0.91
            }
        };
        self.missed = Vec3::new(
            slowed(off[0], body.position.x - last.x, body.velocity.x),
            0.0,
            slowed(off[2], body.position.z - last.z, body.velocity.z),
        );
        let state = [
            (body.ground.on_ground, Played::GROUND),
            (body.fluids.was_touching_water, Played::WATER),
            (body.fall_flying, Played::GLIDING),
            (body.sprinting, Played::SPRINTING),
            (body.shift_key_down, Played::SNEAKING),
            (body.swimming, Played::SWIMMING),
            (body.flying, Played::FLYING),
            (body.is_passenger, Played::RIDING),
        ]
        .into_iter()
        .filter(|&(on, _)| on)
        .fold(0, |bits, (_, bit)| bits | bit);
        body.events.clear();
        body.item_use_speed = server.item_use_speed;
        // Under an excuse the client's velocity is off by about what its move was: take it, and the
        // replay meets the client again within a few ticks. A client that sent no move stayed
        // where it last said (on a boat, say, which the replay doesn't stand on).
        if self.drift > 0 && offset > 0.0 {
            let p = said.unwrap_or(last);
            body.velocity = body.velocity.add_vector(p.subtract(body.position));
        }
        match said {
            Some(p) => {
                body.position = p;
                self.said = p;
            }
            None if offset > 0.0 => body.position = self.said,
            None => {}
        }
        let ground = tick
            .flags
            .is_none_or(|f| (f & 1 != 0) == self.body.ground.on_ground);
        let velocity = [velocity.x, velocity.y, velocity.z];
        let played = Played {
            offset,
            candidate,
            ground,
            off,
            moved,
            state,
            input: tick.input,
            at: [end.x, end.y, end.z],
            velocity,
        };
        if self.ticks.len() == KEPT {
            self.ticks.pop_front();
        }
        self.ticks.push_back(played);
        Some(played)
    }
}

/// ServerboundPlayerInputPacket's flags as keys.
fn keys(input: u8) -> Keys {
    let key = |bit: u8| input & bit != 0;
    Keys {
        forward: key(1),
        backward: key(2),
        left: key(4),
        right: key(8),
        jump: key(16),
        shift: key(32),
        sprint: key(64),
        analog: None,
    }
}

/// What the server decides about the player, from its own copy (its attributes and effects come
/// with the changes the server sent): the rest is the client's.
fn sync(body: &mut PlayerMotion, server: &PlayerMotion) {
    (body.may_fly, body.flying, body.flying_speed) =
        (server.may_fly, server.flying, server.flying_speed);
    (body.invulnerable, body.spectator, body.sleeping) =
        (server.invulnerable, server.spectator, server.sleeping);
    body.glider = server.glider;
    (body.item_use_speed, body.item_use_slows_sprint) =
        (server.item_use_speed, server.item_use_slows_sprint);
    (
        body.leather_boots,
        body.freeze_immune,
        body.burn_from_stepping_immune,
    ) = (
        server.leather_boots,
        server.freeze_immune,
        server.burn_from_stepping_immune,
    );
    body.border = server.border.clone();
    // The keys already hold the client's auto-jump.
    (body.auto_jump_enabled, body.auto_jump_time) = (false, 0);
    (body.client_side, body.server_copy) = (true, false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        assets::Assets,
        motion::{MotionResources, MotionWorld},
        world::World,
    };
    use horizon_anticheat::prediction::Record;
    use std::path::Path;

    /// Keys for each tick: 20 walking, 12 sprint-jumping, 12 strafing while turning, 6 still.
    fn script() -> Vec<(u8, f32)> {
        let mut ticks = Vec::new();
        ticks.extend((0..20).map(|_| (1, 0.0)));
        ticks.extend((0..12).map(|_| (1 | 16 | 64, 0.0)));
        ticks.extend((0..12).map(|i| (1 | 4, 3.0 * i as f32)));
        ticks.extend((0..6).map(|_| (0, 36.0)));
        ticks
    }

    /// A velocity the client gets on a tick of the script: whether the server sent it (`heard`),
    /// whether the server excuses the ticks it is off (`excused`), and whether the client takes it.
    struct Push {
        tick: usize,
        velocity: [f64; 3],
        heard: bool,
        excused: bool,
        taken: bool,
    }

    /// One tick of the script as the server saw it.
    struct Step {
        played: Played,
        over: Over,
        /// Where the client said it was, and where the replay last agreed.
        said: [f64; 3],
        agreed: [f64; 3],
    }

    /// The offsets of a client that walks `script` in the test scene, its moves passed through
    /// `cheat` (the position it sends and stays at, from the one before and the honest one). Each
    /// tick of the script is a server tick too, whose ping the client answers before its own.
    fn offsets(cheat: impl Fn(Vec3, Vec3) -> Vec3, push: Option<Push>) -> Option<Vec<Played>> {
        Some(
            steps(|_, before, honest| cheat(before, honest), push)?
                .into_iter()
                .map(|s| s.played)
                .collect(),
        )
    }

    /// Where the client starts in the test scene and what it plays.
    struct Run {
        start: Vec3,
        /// Keys and yaw for each tick.
        script: Vec<(u8, f32)>,
        pitch: f32,
        /// An elytra on.
        glider: bool,
        /// From tick `.0` on, the ticks come in a burst that the server only plays at tick `.1`,
        /// when the client accepts a teleport back to where the burst began.
        teleport: Option<(usize, usize)>,
        /// Ticks `.0 .. .1` arrive together: the server plays them all at tick `.1`, in one
        /// catch-up, as it does when its tick lags the client's.
        burst: Option<(usize, usize)>,
        /// Blocks set in the scene after the floor.
        blocks: Vec<([i32; 3], &'static str)>,
        /// On tick `.0`, the server sends the player attributes (the server's body has no sprint
        /// modifier): `.1` as in [`Change::Attributes`], which the client takes.
        attributes: Option<(usize, Option<bool>)>,
        /// On each tick `.0`, the client handles what the server sent when it heard the client's
        /// sprint `.1`: the shared flags with that sprinting bit, and the movement speed with the
        /// sprint modifier or without ([`Change::Sprinting`], [`Change::Attributes`]).
        server_sprint: Vec<(usize, bool)>,
        /// The client ignores the sprinting flag and movement speed the server sends (a cheat that
        /// keeps its sprint through a hit).
        keep_sprint: bool,
        /// The first ticks the client is on its loading screen: it ends them without moving or
        /// sending a move.
        frozen: usize,
        /// The ticks the client uses an item (slowed to 0.2), and those the server thinks it does.
        item_use: Option<(std::ops::Range<usize>, std::ops::Range<usize>)>,
    }

    impl Run {
        /// [`script`] on the floor.
        fn walk() -> Self {
            Self {
                start: Vec3::new(20.5, 5.0, 20.5),
                script: script(),
                pitch: 0.0,
                glider: false,
                teleport: None,
                burst: None,
                blocks: Vec::new(),
                attributes: None,
                server_sprint: Vec::new(),
                keep_sprint: false,
                frozen: 0,
                item_use: None,
            }
        }
    }

    /// [`offsets`], with what the check makes of each tick; `cheat` also gets the tick's index.
    fn steps(cheat: impl Fn(usize, Vec3, Vec3) -> Vec3, push: Option<Push>) -> Option<Vec<Step>> {
        steps_in(Run::walk(), cheat, push)
    }

    /// [`steps`] for any [`Run`].
    fn steps_in(
        run: Run,
        cheat: impl Fn(usize, Vec3, Vec3) -> Vec3,
        push: Option<Push>,
    ) -> Option<Vec<Step>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return None;
        }
        let assets = Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let binding = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let mut scene = World::test_scene(&assets, &render, 2).unwrap();
        // A clean stone floor with its top at y 5, and a slab on the walk to step up on.
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let air = assets.registry.default_state("minecraft:air").unwrap();
        for x in 10..=30 {
            for z in 10..=40 {
                scene.set(x, 4, z, stone).unwrap();
                for y in 5..=12 {
                    scene.set(x, y, z, air).unwrap();
                }
            }
        }
        let slab = assets
            .registry
            .default_state("minecraft:smooth_stone_slab")
            .unwrap();
        scene.set(20, 5, 23, slab).unwrap();
        for &(at, block) in &run.blocks {
            let state = assets.registry.default_state(block).unwrap();
            scene.set(at[0], at[1], at[2], state).unwrap();
        }
        let start = run.start;
        let mut client = PlayerMotion::new(&resources, start).unwrap();
        client.client_side = true;
        let mut server = PlayerMotion::new(&resources, start).unwrap();
        (client.glider, server.glider) = (run.glider, run.glider);
        client.pitch = run.pitch;
        let mut world = |player: &mut PlayerMotion, f: &mut dyn FnMut(&mut MotionWorld)| {
            f(&mut MotionWorld {
                entities: &[],
                player,
                scene: &mut scene,
                assets: &assets,
                collision_binding: &binding,
                resources: &resources,
            })
        };
        // Settle on the floor first (or fall for a while).
        for _ in 0..10 {
            world(&mut client, &mut |w| {
                w.controlled_simulation_tick(Keys::default()).unwrap()
            });
        }
        // The start is a teleport: the client goes on from there, standing still.
        client.velocity = Vec3::ZERO;
        let at = client.position;
        let mut record = Record::default();
        record.teleported([at.x, at.y, at.z], false);
        let mut replay = Replay::new(&server, &record.take_restart().unwrap());
        let mut played = Vec::new();
        let run_teleport = run.teleport;
        let run_burst = run.burst;
        let run_attributes = run.attributes;
        let run_frozen = run.frozen;
        let run_item_use = run.item_use;
        let mut back = start;
        for (i, (input, yaw)) in run.script.into_iter().enumerate() {
            let tick = i as u64;
            if let Some(push) = push.as_ref().filter(|p| p.tick == i) {
                let velocity = horizon_protocol::buf::LpVec3::quantize(push.velocity);
                if push.heard {
                    record.changed(tick, Change::Velocity(velocity));
                }
                if push.taken {
                    client.velocity = Vec3::new(velocity[0], velocity[1], velocity[2]);
                }
            }
            if let Some((_, sprint)) = run_attributes.filter(|&(at, _)| at == i) {
                record.changed(tick, Change::Attributes { sprint });
                if let Some(kept) = sprint {
                    client.set_sprint_modifier(kept).unwrap();
                }
            }
            for &(_, sprint) in run.server_sprint.iter().filter(|&&(at, _)| at == i) {
                record.changed(tick, Change::Sprinting(sprint));
                record.changed(
                    tick,
                    Change::Attributes {
                        sprint: Some(sprint),
                    },
                );
                if !run.keep_sprint {
                    client.sprinting = sprint;
                    client.set_sprint_modifier(sprint).unwrap();
                }
            }
            record.pong(tick, tick);
            let before = client.position;
            if run_teleport.is_some_and(|(burst, _)| burst == i) {
                back = before;
            }
            client.yaw = yaw;
            if let Some((ref on_client, ref on_server)) = run_item_use {
                let speed = |r: &std::ops::Range<usize>| r.contains(&i).then_some(0.2);
                (client.item_use_speed, server.item_use_speed) =
                    (speed(on_client), speed(on_server));
            }
            if i >= run_frozen {
                world(&mut client, &mut |w| {
                    w.controlled_simulation_tick(keys(input)).unwrap()
                });
                client.position = cheat(i, before, client.position);
                let p = client.position;
                let ground = u8::from(client.ground.on_ground);
                record.moved(Some([p.x, p.y, p.z]), Some((yaw, run.pitch)), ground);
            }
            let p = client.position;
            record.tick_end(input, client.sprinting);
            // A burst: the server hears these ticks now and plays them together at the last one.
            if run_burst.is_some_and(|(from, to)| (from..to).contains(&i)) {
                continue;
            }
            let mut run = |body: &mut PlayerMotion, keys, sprinting| {
                let mut result = Ok(());
                world(body, &mut |w| result = w.replayed_tick(keys, sprinting));
                result
            };
            if run_teleport.is_some_and(|(burst, at)| (burst..at).contains(&i)) {
                continue;
            }
            if run_teleport.is_some_and(|(_, at)| at == i) {
                record.teleported([back.x, back.y, back.z], false);
                (client.position, client.velocity) = (back, Vec3::ZERO);
                let restart = record.take_restart().unwrap();
                assert!(replay.finish(&record, &server, &restart, &mut run));
                replay.restart(&restart);
            }
            let over = replay.catch_up(&record, &server, 0.03, run).unwrap();
            if over.ticks > 0 && push.as_ref().is_some_and(|p| p.excused) {
                replay.excuse();
            }
            played.push(Step {
                played: *replay.ticks.back().unwrap(),
                over,
                said: [p.x, p.y, p.z],
                agreed: replay.agreed(),
            });
        }
        Some(played)
    }

    /// A client still loading the level (#5849) neither moves nor sends a move, while the replay
    /// falls from where it joined: those ticks are over, but none with a move, which is what lets
    /// the server excuse them. Its first move after them counts as any other.
    #[test]
    fn a_client_on_its_loading_screen_is_over_without_a_move() {
        let run = Run {
            start: Vec3::new(20.5, 12.0, 20.5),
            script: vec![(0, 0.0); 20],
            frozen: 7,
            ..Run::walk()
        };
        let Some(steps) = steps_in(run, |_, _, honest| honest, None) else {
            return;
        };
        let (frozen, after) = steps.split_at(7);
        assert!(frozen.iter().any(|s| s.over.ticks > 0), "the replay falls");
        assert!(frozen.iter().all(|s| !s.over.moved));
        assert!(after.iter().all(|s| s.over.ticks == 0 || s.over.moved));
    }

    /// Walking forward the whole script.
    fn forward() -> Vec<(u8, f32)> {
        vec![(1, 0.0); 70]
    }

    /// Food eaten one after another with the key held, walking (#5860): the client finishes each a
    /// few ticks after the server does. Nothing counts.
    #[test]
    fn item_uses_a_few_ticks_apart_from_the_servers_are_played() {
        let run = Run {
            script: forward(),
            item_use: Some((5..40, 5..37)),
            ..Run::walk()
        };
        let Some(steps) = steps_in(run, |_, _, honest| honest, None) else {
            return;
        };
        assert!(steps.iter().all(|s| s.over.ticks == 0));
        let run = Run {
            script: forward(),
            item_use: Some((10..42, 10..33)),
            ..Run::walk()
        };
        let steps = steps_in(run, |_, _, honest| honest, None).unwrap();
        assert!(steps.iter().all(|s| s.over.ticks == 0));
    }

    /// A client that keeps walking at full speed through a use the server started (a no-slow) is
    /// caught once the window has passed.
    #[test]
    fn a_use_without_its_slowdown_is_caught() {
        let run = Run {
            script: forward(),
            item_use: Some((0..0, 5..65)),
            ..Run::walk()
        };
        let Some(steps) = steps_in(run, |_, _, honest| honest, None) else {
            return;
        };
        let counted =
            |r: std::ops::Range<usize>| steps[r].iter().filter(|s| s.over.ticks > 0).count();
        assert!(counted(17..65) >= 40, "{}", counted(17..65));
    }

    #[test]
    fn a_played_tick_tells_its_offset_and_state() {
        let played = Played {
            offset: 0.173,
            candidate: 0,
            ground: true,
            off: [0.0, 0.1729, -0.001],
            moved: 0.2806,
            state: Played::SPRINTING | Played::WATER,
            input: 1 | 16 | 64,
            at: [12.3, 63.004, -5.5],
            velocity: [0.0, 0.0412, -0.25],
        };
        assert_eq!(
            played.describe(),
            "off +0.000 +0.173 -0.001, moved 0.281, air water sprinting, keys forward jump sprint, \
             at 12.30 63.00 -5.50, velocity +0.000 +0.041 -0.250"
        );
    }

    #[test]
    fn a_cheats_worst_tick_is_kept_with_its_offset() {
        let Some(steps) = steps(
            |i, _, honest| honest.add_vector(Vec3::new(if i == 10 { 1.0 } else { 0.0 }, 0.0, 0.0)),
            None,
        ) else {
            return;
        };
        let worst = steps[10].over.at_worst.expect("the lying tick is kept");
        assert!((worst.off[0] - 1.0).abs() < 0.05, "{worst:?}");
        assert!(worst.off[1].abs() < 1e-6 && worst.moved > 0.9, "{worst:?}");
        assert!(
            worst.describe().starts_with("off +1.0"),
            "{}",
            worst.describe()
        );
    }

    #[test]
    fn an_honest_client_is_played_again_exactly() {
        let Some(played) = offsets(|_, honest| honest, None) else {
            return;
        };
        assert_eq!(played.len(), script().len());
        for (i, p) in played.iter().enumerate() {
            assert!(p.offset <= EXACT, "tick {i}: {p:?}");
            assert!(p.ground, "tick {i}: {p:?}");
            assert!(p.candidate == 0 || i == 0, "tick {i}: {p:?}");
        }
    }

    #[test]
    fn a_faster_or_flying_client_is_not() {
        let Some(speed) = offsets(
            |before, honest| {
                let d = honest.subtract(before);
                before.add_vector(Vec3::new(d.x * 1.3, d.y, d.z * 1.3))
            },
            None,
        ) else {
            return;
        };
        let far = speed.iter().filter(|p| p.offset > 0.01).count();
        assert!(far > 30, "{far} of {} ticks off: {speed:?}", speed.len());
        let fly = offsets(
            |_, honest| honest.add_vector(Vec3::new(0.0, 0.1, 0.0)),
            None,
        )
        .unwrap();
        assert!(fly.iter().all(|p| p.offset > 0.05), "{fly:?}");
    }

    /// Attributes sent mid sprint-jump: one without the movement speed leaves the client's sprint
    /// modifier alone (the server's body, which never sprinted here, has none to copy); one with the
    /// movement speed but no sprint modifier takes it away while the client sprints on.
    #[test]
    fn attributes_sent_mid_sprint_are_played_as_the_client_takes_them() {
        for sprint in [None, Some(false), Some(true)] {
            let run = Run {
                attributes: Some((25, sprint)),
                ..Run::walk()
            };
            let Some(steps) = steps_in(run, |_, _, honest| honest, None) else {
                return;
            };
            for (i, s) in steps.iter().enumerate() {
                assert!(
                    s.played.offset <= EXACT,
                    "{sprint:?}, tick {i}: {:?}",
                    s.played
                );
            }
        }
    }

    /// A knockback (ClientboundSetEntityMotionPacket) mid-walk: the replay applies it on the first
    /// client tick that had seen the server tick that sent it.
    #[test]
    fn a_push_is_played_on_the_tick_the_client_took_it() {
        let push = |taken| Push {
            tick: 25,
            velocity: [0.3, 0.42, -0.5],
            heard: true,
            excused: false,
            taken,
        };
        let Some(honest) = offsets(|_, honest| honest, Some(push(true))) else {
            return;
        };
        for (i, p) in honest.iter().enumerate() {
            assert!(p.offset <= EXACT && p.ground, "tick {i}: {p:?}");
        }
        // Anti-knockback: a client that ignores the push.
        let ignored = offsets(|_, honest| honest, Some(push(false))).unwrap();
        assert!(ignored[..25].iter().all(|p| p.offset <= EXACT));
        assert!(ignored[25].offset > 0.3, "{:?}", ignored[25]);
    }

    /// A client that starts flying mid-walk: each tick it flies counts, weighed by how far it is off,
    /// and the setback point stays where it last walked honestly.
    #[test]
    fn a_cheat_counts_from_where_it_started() {
        let Some(steps) = steps(
            |i, _, honest| honest.add_vector(Vec3::new(0.0, if i >= 20 { 0.06 } else { 0.0 }, 0.0)),
            None,
        ) else {
            return;
        };
        for (i, s) in steps.iter().enumerate() {
            if i < 20 {
                assert_eq!(s.over, Over::default(), "tick {i}");
                assert_eq!(s.agreed, s.said, "tick {i}");
            } else {
                assert_eq!(s.over.ticks, 1, "tick {i}: {:?}", s.over);
                assert!(s.over.weight > 1.9, "tick {i}: {:?}", s.over);
                assert_eq!(s.agreed, steps[19].said, "tick {i}");
            }
        }
    }

    /// Ticks that arrive together when the server's tick lags the client's are played in one
    /// catch-up: a lie on the first makes the replay stand at the client's spot, and the honest
    /// tick after it must not become the setback point (#5132).
    #[test]
    fn a_burst_keeps_the_setback_before_its_lie() {
        let Some(steps) = steps_in(
            Run {
                burst: Some((20, 21)),
                ..Run::walk()
            },
            |i, _, honest| {
                if i == 20 {
                    honest.add_vector(Vec3::new(3.0, 0.0, 0.0))
                } else {
                    honest
                }
            },
            None,
        ) else {
            return;
        };
        // Ticks 0-19 each play on their own; tick 20's lie arrives with tick 21, so the burst's
        // single step is the 21st: the lie counts once, and the setback point stays where the walk
        // agreed, before it.
        assert_eq!(
            steps.len(),
            script().len() - 1,
            "one catch-up for the burst"
        );
        let burst = &steps[20];
        assert_eq!(burst.over.ticks, 1, "{:?}", burst.over);
        assert!(burst.over.worst > 2.9, "{:?}", burst.over);
        assert_eq!(
            burst.agreed, steps[19].said,
            "the setback point must be the last honest spot"
        );
    }

    /// A push the server never sent (another entity's): excused, the replay takes the client's
    /// velocity, is back within the tolerance in a few ticks, and nothing counts.
    #[test]
    fn an_excused_push_is_caught_up_with() {
        let push = Push {
            tick: 25,
            velocity: [0.3, 0.42, -0.5],
            heard: false,
            excused: true,
            taken: true,
        };
        let Some(steps) = steps(|_, _, honest| honest, Some(push)) else {
            return;
        };
        assert!(steps[25].played.offset > 0.3, "{:?}", steps[25].played);
        assert_eq!(steps[25].over.ticks, 1);
        for (i, s) in steps.iter().enumerate().skip(26) {
            assert_eq!(s.over, Over::default(), "tick {i}");
        }
        let met = steps[26..].iter().position(|s| s.played.offset <= 0.03);
        assert!(
            met.is_some_and(|t| t < 6),
            "{:?}",
            steps.iter().map(|s| s.played.offset).collect::<Vec<_>>()
        );
    }

    /// A push across the server never sent and doesn't excuse (a hit the replay missed): the replay
    /// tries the velocity it left on the next tick, so it counts once, not on each tick it takes to
    /// die out.
    #[test]
    fn a_missed_push_counts_once() {
        for velocity in [[0.3, 0.0, -0.2], [-0.25, 0.0, 0.1], [0.0, 0.0, 0.4]] {
            // The client's alone: the record never has it, and nothing excuses it.
            let push = Push {
                tick: 25,
                velocity,
                heard: false,
                excused: false,
                taken: true,
            };
            let Some(steps) = steps(|_, _, honest| honest, Some(push)) else {
                return;
            };
            let counted: Vec<usize> = steps
                .iter()
                .enumerate()
                .filter(|(_, s)| s.over.ticks > 0)
                .map(|(i, _)| i)
                .collect();
            assert_eq!(
                counted,
                vec![25],
                "{velocity:?}: {:?}",
                steps.iter().map(|s| s.played.offset).collect::<Vec<_>>()
            );
        }
    }

    /// What the catch-up of [`a_missed_push_counts_once`] costs: a small speed lie kept up on every
    /// tick of a sprint-jump (a twentieth of a block a tick) goes uncounted only while the velocity
    /// the replay took from its first tick still hides it; then it counts on every tick.
    #[test]
    fn a_small_speed_lie_kept_up_still_counts() {
        let run = Run {
            script: vec![(1 | 16 | 64, 0.0); 60],
            ..Run::walk()
        };
        let Some(steps) = steps_in(
            run,
            |i, _, honest| honest.add_vector(Vec3::new(0.0, 0.0, if i >= 20 { 0.05 } else { 0.0 })),
            None,
        ) else {
            return;
        };
        let counted: Vec<usize> = steps
            .iter()
            .enumerate()
            .filter(|(_, s)| s.over.ticks > 0)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(counted.first(), Some(&20), "{counted:?}");
        assert!(counted.iter().all(|&i| i >= 20), "{counted:?}");
        // Measured: ticks 21 to 25 go uncounted, 35 of the 40 count.
        assert!(counted.len() >= 30, "{} of 40: {counted:?}", counted.len());
    }

    /// A sprint stopped for one tick (forward let go) and started again: the server hears the STOP
    /// and the START in two of its ticks, and sends each back (the shared flags and the movement
    /// speed). The STOP's reach the client `late` ticks after it sprints again: its flag cleared, it
    /// starts the sprint again in its next aiStep, its modifier with it, and says nothing. The
    /// START's come one tick later (the live server's `ground sprinting` flags, 0.045 off).
    fn sprint_restarted(late: usize, gap: usize) -> Run {
        let mut script = vec![(1 | 64, 0.0); 20];
        script.push((0, 0.0));
        script.extend(vec![(1 | 64, 0.0); 20]);
        Run {
            script,
            server_sprint: vec![(21 + late, false), (21 + late + gap, true)],
            ..Run::walk()
        }
    }

    #[test]
    fn a_sprint_the_server_cleared_is_started_again() {
        for (late, gap) in [(1, 1), (2, 1), (3, 2)] {
            let Some(steps) = steps_in(sprint_restarted(late, gap), |_, _, honest| honest, None)
            else {
                return;
            };
            for (i, s) in steps.iter().enumerate() {
                assert!(
                    s.played.offset <= EXACT && s.over == Over::default(),
                    "late {late}, gap {gap}, tick {i}: {:?}",
                    s.played
                );
            }
        }
    }

    /// The same with a speed lie (a twentieth of a block a tick) from the tick the STOP's packets
    /// reach the client: it counts from that tick on.
    #[test]
    fn a_speed_lie_through_a_sprint_restart_still_counts() {
        let Some(steps) = steps_in(
            sprint_restarted(2, 1),
            |i, _, honest| honest.add_vector(Vec3::new(0.0, 0.0, if i >= 23 { 0.05 } else { 0.0 })),
            None,
        ) else {
            return;
        };
        let counted: Vec<usize> = steps
            .iter()
            .enumerate()
            .filter(|(_, s)| s.over.ticks > 0)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(counted.first(), Some(&23), "{counted:?}");
        assert!(counted.len() >= 10, "{} of 18: {counted:?}", counted.len());
    }

    /// A sprint the server stopped after a hit (Player.causeExtraKnockback), which the client's own
    /// attack on a mob doesn't: the server's flag and movement speed reach the client on tick 14.
    /// Forward is held throughout, the sprint key for the first 10 ticks, and for the rest if `key`.
    /// The client's own commands say what it does: with the key it starts the sprint again in its
    /// next aiStep and says nothing (the live server's `ground, keys forward sprint` flags, where
    /// the record took the server's player and played the rest of the sprint as a walk); without it
    /// it stops and says STOP. `water`: the same swimming across a pool, toward its surface.
    fn sprint_hit(key: bool, water: bool) -> Run {
        let sprint = if key { 64 } else { 0 };
        let mut script = vec![(1 | 64, 0.0); 10];
        script.extend(vec![(1 | sprint, 0.0); 40]);
        let mut blocks = Vec::new();
        if water {
            for x in 14..=26 {
                for y in 5..=7 {
                    for z in 14..=40 {
                        blocks.push(([x, y, z], "minecraft:water"));
                    }
                }
            }
        }
        Run {
            script,
            pitch: if water { -8.0 } else { 0.0 },
            blocks,
            server_sprint: vec![(14, false)],
            ..Run::walk()
        }
    }

    #[test]
    fn a_sprint_the_server_stopped_after_a_hit_is_the_clients() {
        for (key, water) in [(true, false), (false, false), (true, true), (false, true)] {
            let Some(steps) = steps_in(sprint_hit(key, water), |_, _, honest| honest, None) else {
                return;
            };
            for (i, s) in steps.iter().enumerate() {
                assert!(
                    s.played.offset <= EXACT && s.over == Over::default(),
                    "key {key}, water {water}, tick {i}: {:?}",
                    s.played
                );
            }
            // The client sprints on after the hit only with the key.
            let last = steps.last().unwrap().played.state;
            assert_eq!(
                last & Played::SPRINTING != 0,
                key,
                "water {water}: {last:b}"
            );
        }
    }

    /// A client that sprints on through the server's stop without the sprint key, which vanilla's
    /// starts again only with it, is played walking from the tick the stop reaches it; one that
    /// holds the key and moves a twentieth of a block a tick too far counts from there too.
    #[test]
    fn a_sprint_kept_without_the_key_or_sped_up_after_a_hit_counts() {
        for water in [false, true] {
            let kept = Run {
                keep_sprint: true,
                ..sprint_hit(false, water)
            };
            let fast = |i: usize, _, honest: Vec3| {
                honest.add_vector(Vec3::new(0.0, 0.0, if i >= 14 { 0.05 } else { 0.0 }))
            };
            for (name, steps) in [
                ("kept", steps_in(kept, |_, _, honest| honest, None)),
                ("fast", steps_in(sprint_hit(true, water), fast, None)),
            ] {
                let Some(steps) = steps else {
                    return;
                };
                let counted: Vec<usize> = steps
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.over.ticks > 0)
                    .map(|(i, _)| i)
                    .collect();
                assert!(
                    counted.first().is_some_and(|&i| (14..=16).contains(&i)),
                    "{name}, water {water}: {counted:?}"
                );
                assert!(
                    counted.len() >= 25,
                    "{name}, water {water}: {} of 36: {counted:?}",
                    counted.len()
                );
            }
        }
    }

    /// An elytra: a jump while falling starts a glide, played again exactly.
    #[test]
    fn a_glide_is_played_again_exactly() {
        let mut script = vec![(0, 0.0); 4];
        script.push((16, 0.0));
        script.extend(vec![(0, 0.0); 25]);
        let run = Run {
            start: Vec3::new(20.5, 40.0, 12.5),
            script,
            pitch: 10.0,
            glider: true,
            ..Run::walk()
        };
        let Some(steps) = steps_in(run, |_, _, honest| honest, None) else {
            return;
        };
        let offsets: Vec<f64> = steps.iter().map(|s| s.played.offset).collect();
        assert!(offsets.iter().all(|&o| o <= EXACT), "{offsets:?}");
        // It glided: forward, not just down.
        let forward = steps.last().unwrap().said[2] - steps[4].said[2];
        assert!(forward > 5.0, "{forward}");
    }

    /// A teleport accepted after a burst of ticks the server hadn't played yet: the replay plays
    /// them first, so the jump delay they started goes on through the teleport as the client's does.
    #[test]
    fn a_burst_before_a_teleport_is_played_first() {
        let mut run = Run::walk();
        // The burst holds the first sprint-jump (tick 20). Back on the floor at tick 23, the client
        // holds jump but its jump delay (10 ticks from 20) holds it until tick 30.
        run.teleport = Some((19, 23));
        let Some(steps) = steps_in(run, |_, _, honest| honest, None) else {
            return;
        };
        let offsets: Vec<f64> = steps.iter().map(|s| s.played.offset).collect();
        assert!(offsets.iter().all(|&o| o <= EXACT), "{offsets:?}");
        assert!(steps.iter().all(|s| s.over == Over::default()));
    }

    /// A client walking 40 ticks towards +z, over the slab, into `blocks`, its moves passed through
    /// `cheat`: the tick the cheat starts counts, and none before it does.
    fn caught(blocks: Vec<([i32; 3], &'static str)>, cheat: impl Fn(Vec3, Vec3) -> Option<Vec3>) {
        let run = Run {
            script: vec![(1, 0.0); 40],
            blocks,
            ..Run::walk()
        };
        let started = std::cell::Cell::new(None);
        let Some(steps) = steps_in(
            run,
            |i, before, honest| match cheat(before, honest) {
                Some(p) => {
                    started.set(started.get().or(Some(i)));
                    p
                }
                None => honest,
            },
            None,
        ) else {
            return;
        };
        let start = started.get().expect("the cheat never started");
        for (i, s) in steps.iter().enumerate().take(start) {
            assert_eq!(s.over, Over::default(), "tick {i}");
        }
        assert_eq!(steps[start].over.ticks, 1, "{:?}", steps[start].played);
        let counted = steps[start..].iter().filter(|s| s.over.ticks > 0).count();
        assert!(counted > 0, "{counted}");
    }

    /// A wall at z 25 (from y 5 up), which the walk runs into at z 24.7.
    fn wall(height: i32) -> Vec<([i32; 3], &'static str)> {
        (19..=21)
            .flat_map(|x| (5..5 + height).map(move |y| ([x, y, 25], "minecraft:stone")))
            .collect()
    }

    /// Step: up onto a full block without a jump (the most a walk steps up is 0.6).
    #[test]
    fn a_step_is_caught() {
        caught(wall(1), |before, honest| {
            (honest.z > 24.6 && before.y < 6.0).then(|| Vec3::new(honest.x, 6.0, 25.1))
        });
    }

    /// Spider: up a wall, 0.2 a tick, with no ladder or vine.
    #[test]
    fn a_spider_is_caught() {
        caught(wall(4), |before, honest| {
            (honest.z > 24.6).then(|| Vec3::new(honest.x, before.y + 0.2, honest.z))
        });
    }

    /// Jesus: walking on water as on the ground.
    #[test]
    fn a_jesus_is_caught() {
        let water = (19..=21)
            .flat_map(|x| (25..=40).map(move |z| ([x, 4, z], "minecraft:water")))
            .collect();
        caught(water, |before, honest| {
            (honest.z > 25.4 && honest.y < before.y)
                .then(|| Vec3::new(honest.x, before.y, honest.z))
        });
    }
}
