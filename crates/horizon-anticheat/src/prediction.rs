//! The movement prediction's record: each of a player's client ticks as the server heard them, so
//! the server can play the tick again with vanilla's movement code and compare (docs/anticheat.md).
//!
//! A vanilla client ends every tick with ServerboundClientTickEndPacket, after the tick's input
//! (ServerboundPlayerInputPacket, sent when the keys change) and its move (ServerboundMovePlayerPacket,
//! when it moved or turned). Those packets arrive in order, so the packets between two tick ends are one
//! client tick.
//!
//! What the server changes about the player (a push, a teleport, an effect) takes effect on the client
//! when it handles the packet, which is some client ticks after the server sent it. At the end of each of
//! its ticks the server sends a ClientboundPingPacket whose id is that server tick; the client answers
//! with ServerboundPongPacket as soon as it handles it, before its next tick runs. A client tick knows
//! the last server tick it had handled: every change of that server tick or an earlier one was in effect
//! during the client tick, none of a later one.

use std::collections::VecDeque;

/// Client ticks kept: two seconds at 20 a second.
pub const KEPT: usize = 40;

/// Server changes kept, the oldest dropped first.
const CHANGES_KEPT: usize = 64;

/// Changes near the player kept until every kept tick has seen them.
const NEAR_KEPT: usize = 1024;

/// Something the server sent the player that changes how its client moves. The client applies it when
/// it handles the packet, before its next tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    /// ClientboundSetEntityMotionPacket for the player itself: its new velocity, as the client
    /// decodes it.
    Velocity([f64; 3]),
    /// ClientboundExplodePacket's player knockback, added to the velocity.
    Knockback([f64; 3]),
    /// ClientboundUpdateAttributesPacket for the player itself: the server's attributes as they
    /// are when the replay reaches it, but for the movement speed's sprint modifier, which the
    /// client's sprinting decides: `sprint` is whether the packet had it when it carried the
    /// movement speed (the client takes the modifiers as sent, so a sprint modifier the packet
    /// lacks is gone while the client sprints), None when it didn't (the client keeps its own).
    Attributes { sprint: Option<bool> },
    /// ClientboundSetEntityDataPacket for the player itself with its shared flags: their sprinting
    /// bit, which the client takes as sent (SynchedEntityData.assignValues sets the flag alone, its
    /// speed modifier and wasSprinting stay). A client whose sprint the server cleared this way
    /// starts it again in its next aiStep, which adds the modifier back, and says nothing.
    Sprinting(bool),
    /// ClientboundUpdateMobEffectPacket or ClientboundRemoveMobEffectPacket for the player itself:
    /// the server's effects, as they are when the replay reaches it. (Their attribute modifiers
    /// are the server's: they reach the client as attributes.)
    Effects,
    /// ClientboundPlayerAbilitiesPacket (a game mode, flight given or taken), or the player's chest
    /// slot (a glider put on or taken off): the replay takes these from the server every tick, so a
    /// tick played before the client had them may be off.
    Abilities,
}

/// A teleport the client accepted: the replay starts again from it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Restart {
    /// Where it put the player.
    pub at: [f64; 3],
    /// It kept the player's motion (an End gateway's).
    pub kept_motion: bool,
    /// Client ticks ended and changes heard before it: the ones after are the replay's.
    pub ended: u64,
    pub changed: u64,
}

/// One client tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClientTick {
    /// The keys held (ServerboundPlayerInputPacket's flags: forward, backward, left, right, jump,
    /// shift, sprint), as last sent.
    pub input: u8,
    /// Sprinting, as the player's commands left it (ServerboundPlayerCommandPacket).
    pub sprinting: bool,
    /// The rotation the client last sent.
    pub yaw: f32,
    pub pitch: f32,
    /// Where the tick's move said the player is, if it said: a move with a position.
    pub position: Option<[f64; 3]>,
    /// The tick's move packet's flags (1: on the ground, 2: against a wall), if it sent one.
    pub flags: Option<u8>,
    /// The last server tick whose packets the client had handled (its pong) before this tick ended.
    pub seen: Option<u64>,
}

/// A player's last client ticks.
#[derive(Clone, Debug, Default)]
pub struct Record {
    /// The tick being heard.
    current: ClientTick,
    /// The ended ones, oldest first, at most [`KEPT`].
    ticks: VecDeque<ClientTick>,
    /// The last server tick the client answered for.
    seen: Option<u64>,
    /// Client ticks ended since the record began.
    ended: u64,
    /// Where an accepted teleport put the player, until the replay starts again from there.
    restart: Option<Restart>,
    /// What the server changed, with the server tick that sent it, oldest first.
    changes: VecDeque<(u64, Change)>,
    /// Changes heard since the record began.
    changed: u64,
    /// The blocks the server changed near the player (None: an entity it stopped showing the player
    /// from near it, which counts wherever the player has gone since), with the server tick that sent
    /// each, until every kept tick has seen them.
    near: VecDeque<(u64, Option<[i32; 3]>)>,
    /// The server tick of the last of them dropped early ([`NEAR_KEPT`] came after it).
    near_dropped: Option<u64>,
}

impl Record {
    /// The tick's move: its position if it has one, the rotation if it has one, and its flags.
    pub fn moved(&mut self, position: Option<[f64; 3]>, rotation: Option<(f32, f32)>, flags: u8) {
        if position.is_some() {
            self.current.position = position;
        }
        if let Some((yaw, pitch)) = rotation {
            (self.current.yaw, self.current.pitch) = (yaw, pitch);
        }
        self.current.flags = Some(flags);
    }

    /// ServerboundPongPacket for the ping sent at the end of `server_tick`. False for an answer to a
    /// ping that was never sent, or one older than an answer already heard: a vanilla client answers
    /// every ping, once, in order.
    pub fn pong(&mut self, server_tick: u64, sent_up_to: u64) -> bool {
        if server_tick > sent_up_to || self.seen.is_some_and(|seen| server_tick <= seen) {
            return false;
        }
        self.seen = Some(server_tick);
        true
    }

    /// ServerboundClientTickEndPacket: the tick ends with these keys held and this sprinting. The
    /// next tick starts with the same keys and rotation, until the client says otherwise.
    pub fn tick_end(&mut self, input: u8, sprinting: bool) -> &ClientTick {
        self.current.input = input;
        self.current.sprinting = sprinting;
        self.current.seen = self.seen;
        let next = ClientTick {
            yaw: self.current.yaw,
            pitch: self.current.pitch,
            ..ClientTick::default()
        };
        let ended = std::mem::replace(&mut self.current, next);
        if self.ticks.len() == KEPT {
            self.ticks.pop_front();
        }
        self.ticks.push_back(ended);
        self.ended += 1;
        // What the oldest tick kept had seen, every later one had too.
        if let Some(seen) = self.ticks.front().and_then(|t| t.seen) {
            while self.near.front().is_some_and(|&(sent, _)| sent <= seen) {
                self.near.pop_front();
            }
            self.near_dropped = self.near_dropped.filter(|&sent| sent > seen);
        }
        self.ticks.back().expect("just pushed")
    }

    /// The ended ticks, oldest first.
    pub fn ticks(&self) -> impl DoubleEndedIterator<Item = &ClientTick> + ExactSizeIterator {
        self.ticks.iter()
    }

    /// The last server tick the client answered for.
    pub fn seen(&self) -> Option<u64> {
        self.seen
    }

    /// Client ticks ended since the record began.
    pub fn ended(&self) -> u64 {
        self.ended
    }

    /// The ticks ended after the first `played`, oldest first; None when some of them are no longer
    /// kept.
    pub fn since(&self, played: u64) -> Option<impl Iterator<Item = &ClientTick>> {
        let new = self.ended.checked_sub(played)?;
        let new = usize::try_from(new)
            .ok()
            .filter(|&n| n <= self.ticks.len())?;
        Some(self.ticks.iter().skip(self.ticks.len() - new))
    }

    /// `change`, sent during server tick `server_tick` (before that tick's ping).
    pub fn changed(&mut self, server_tick: u64, change: Change) {
        if self.changes.len() == CHANGES_KEPT {
            self.changes.pop_front();
        }
        self.changes.push_back((server_tick, change));
        self.changed += 1;
    }

    /// Changes heard since the record began.
    pub fn changes(&self) -> u64 {
        self.changed
    }

    /// The changes after the first `applied`, oldest first; None when some of them are no longer kept.
    pub fn changes_since(
        &self,
        applied: u64,
    ) -> Option<impl Iterator<Item = &(u64, Change)> + Clone> {
        let new = self.changed.checked_sub(applied)?;
        let new = usize::try_from(new)
            .ok()
            .filter(|&n| n <= self.changes.len())?;
        Some(self.changes.iter().skip(self.changes.len() - new))
    }

    /// During server tick `server_tick` (before that tick's ping) the server sent the client the block
    /// at `pos`.
    pub fn changed_near(&mut self, server_tick: u64, pos: [i32; 3]) {
        self.note_near(server_tick, Some(pos));
    }

    /// During server tick `server_tick` the server stopped showing the client an entity near it (a
    /// firework it flew with, a mob that pushed it): the client may still have it, wherever it goes.
    pub fn entity_gone(&mut self, server_tick: u64) {
        if self.near.back() != Some(&(server_tick, None)) {
            self.note_near(server_tick, None);
        }
    }

    fn note_near(&mut self, server_tick: u64, pos: Option<[i32; 3]>) {
        if self.near.len() == NEAR_KEPT {
            self.near_dropped = self.near.pop_front().map(|(sent, _)| sent);
        }
        self.near.push_back((server_tick, pos));
    }

    /// Whether a client tick that had `seen` up to that server tick (its [`ClientTick::seen`]) may
    /// not have had a change at a position for which `near` holds: it still had the old block, or the
    /// entity, where the server no longer has it.
    pub fn unseen_near(&self, seen: Option<u64>, near: impl Fn([i32; 3]) -> bool) -> bool {
        let unseen = |sent: u64| seen.is_none_or(|seen| sent > seen);
        self.near_dropped.is_some_and(unseen)
            || self
                .near
                .iter()
                .any(|&(sent, pos)| unseen(sent) && pos.is_none_or(&near))
    }

    /// The client accepted a teleport to `at` (ServerboundAcceptTeleportationPacket): it goes on from
    /// there, standing still unless the teleport `kept_motion` (an End gateway's). The ticks it ended
    /// before stay, for a replay that hasn't played them yet; the one under way starts again.
    pub fn teleported(&mut self, at: [f64; 3], kept_motion: bool) {
        self.restart_tick();
        self.restart = Some(Restart {
            at,
            kept_motion,
            ended: self.ended,
            changed: self.changed,
        });
    }

    /// The last teleport the client accepted, once.
    pub fn take_restart(&mut self) -> Option<Restart> {
        self.restart.take()
    }

    /// The ticks before now say nothing about the ones after.
    pub fn clear(&mut self) {
        self.ticks.clear();
        self.restart_tick();
    }

    fn restart_tick(&mut self) {
        self.current = ClientTick {
            yaw: self.current.yaw,
            pitch: self.current.pitch,
            ..ClientTick::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_near_is_unseen_by_the_ticks_before_its_pong() {
        let mut r = Record::default();
        r.changed_near(4, [1, 2, 3]);
        r.changed_near(5, [7, 2, 3]);
        let near = |p: [i32; 3]| p[0] < 5;
        assert!(r.unseen_near(None, near));
        assert!(r.unseen_near(Some(3), near));
        assert!(!r.unseen_near(Some(4), near));
        assert!(r.unseen_near(Some(4), |_| true));
        // Kept until the oldest tick kept had seen it: one that ended before the pong hadn't.
        r.tick_end(0, false);
        assert!(r.pong(5, 5));
        r.tick_end(0, false);
        assert!(r.unseen_near(Some(3), near));
        for _ in 0..KEPT {
            r.tick_end(0, false);
        }
        assert!(!r.unseen_near(None, |_| true));
        // An entity gone counts anywhere.
        r.entity_gone(5);
        r.entity_gone(5);
        assert!(r.unseen_near(Some(4), |_| false));
        assert!(!r.unseen_near(Some(5), |_| false));
        // One dropped early counts for the ticks that hadn't seen it.
        for i in 0..=NEAR_KEPT as u64 {
            r.changed_near(6 + i, [100, 0, 0]);
        }
        assert!(
            r.unseen_near(Some(5), near),
            "a change dropped unseen counts"
        );
        assert!(!r.unseen_near(Some(6), near));
    }

    #[test]
    fn a_tick_is_the_packets_between_two_tick_ends() {
        let mut r = Record::default();
        r.moved(Some([1.0, 2.0, 3.0]), Some((90.0, 10.0)), 1);
        let t = *r.tick_end(0b1001, true);
        assert_eq!(t.position, Some([1.0, 2.0, 3.0]));
        assert_eq!((t.yaw, t.pitch, t.flags), (90.0, 10.0, Some(1)));
        assert_eq!((t.input, t.sprinting, t.seen), (0b1001, true, None));
        // A tick without a move keeps the rotation and has no position.
        let t = *r.tick_end(0, false);
        assert_eq!((t.position, t.flags, t.yaw), (None, None, 90.0));
        assert_eq!(r.ticks().len(), 2);
    }

    #[test]
    fn pongs_say_which_server_ticks_a_client_tick_had_seen() {
        let mut r = Record::default();
        assert!(r.pong(5, 6));
        r.tick_end(0, false);
        assert!(r.pong(6, 6));
        r.tick_end(0, false);
        r.tick_end(0, false);
        let seen: Vec<_> = r.ticks().map(|t| t.seen).collect();
        assert_eq!(seen, [Some(5), Some(6), Some(6)]);
        // Never sent, answered twice, or out of order.
        assert!(!r.pong(9, 6));
        assert!(!r.pong(6, 6));
        assert!(!r.pong(4, 6));
        assert_eq!(r.seen(), Some(6));
    }

    #[test]
    fn only_the_last_two_seconds_are_kept() {
        let mut r = Record::default();
        for i in 0..KEPT + 5 {
            r.moved(Some([i as f64, 0.0, 0.0]), None, 0);
            r.tick_end(0, false);
        }
        assert_eq!(r.ticks().len(), KEPT);
        assert_eq!(r.ticks().next().unwrap().position, Some([5.0, 0.0, 0.0]));
        r.clear();
        assert_eq!(r.ticks().len(), 0);
    }

    #[test]
    fn the_replay_takes_the_ticks_it_has_not_played() {
        let mut r = Record::default();
        for i in 0..3 {
            r.moved(Some([i as f64, 0.0, 0.0]), None, 0);
            r.tick_end(0, false);
        }
        fn x(r: &Record, played: u64) -> Option<Vec<f64>> {
            r.since(played)
                .map(|t| t.map(|t| t.position.unwrap()[0]).collect())
        }
        assert_eq!(x(&r, 1), Some(vec![1.0, 2.0]));
        assert_eq!(x(&r, 3), Some(vec![]));
        // Played ahead of the record, or so far behind the ticks are gone.
        assert_eq!(x(&r, 4), None);
        r.clear();
        assert_eq!(x(&r, 2), None);
        assert_eq!(x(&r, 3), Some(vec![]));
        r.moved(Some([8.0, 0.0, 0.0]), None, 0);
        r.tick_end(0, false);
        r.moved(Some([9.0, 0.0, 0.0]), None, 0);
        r.teleported([5.0, 6.0, 7.0], false);
        // A tick that ends before the replay starts again is still one after the teleport.
        r.tick_end(0, false);
        let restart = r.take_restart().unwrap();
        assert_eq!((restart.at, restart.kept_motion), ([5.0, 6.0, 7.0], false));
        assert_eq!(r.since(restart.ended).unwrap().count(), 1);
        // The tick ended before it stays for a replay that hasn't played it; the move of the one
        // under way is gone.
        let positions: Vec<_> = r.since(3).unwrap().map(|t| t.position).collect();
        assert_eq!(positions, [Some([8.0, 0.0, 0.0]), None]);
        assert_eq!(r.take_restart(), None);
    }

    #[test]
    fn changes_keep_the_server_tick_that_sent_them() {
        let mut r = Record::default();
        r.changed(3, Change::Velocity([0.0, 0.4, 0.0]));
        r.changed(5, Change::Effects);
        let all: Vec<_> = r.changes_since(0).unwrap().copied().collect();
        assert_eq!(
            all,
            [(3, Change::Velocity([0.0, 0.4, 0.0])), (5, Change::Effects)]
        );
        assert_eq!(r.changes_since(1).unwrap().count(), 1);
        assert!(r.changes_since(3).is_none());
        for i in 0..CHANGES_KEPT as u64 {
            r.changed(6 + i, Change::Effects);
        }
        // The first two are gone.
        assert!(r.changes_since(1).is_none());
        assert_eq!(r.changes_since(2).unwrap().count(), CHANGES_KEPT);
    }
}
