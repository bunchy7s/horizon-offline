//! The anticheat on a dedicated server's hub (docs/anticheat.md): the checks run where the hub handles
//! what players send, and a flag is carried out here (send back, alert, kick, or a ban the game adds to
//! `banned-players.json`: [`HostEvent::Ban`]). The rules and the
//! violation levels are `horizon_anticheat`'s. A dedicated server has it on by default; the hub holds
//! none of this when `server.toml` turns it off (or for a LAN world), and then does exactly what vanilla's
//! server does.
use super::hub::{HostEvent, Hub, TickCtx};
use crate::collisions::SceneCollisions;
use horizon_anticheat::{Check, Config, Strikes};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_world::nbt::{JavaString, Tag};
use std::{io::Write, time::Instant};

pub struct Anticheat {
    pub config: Config,
    /// `log`, opened at the first flag.
    log: Option<std::fs::File>,
    /// The players' kicks by check, read from `strikes` at the first kick.
    strikes: Option<Strikes>,
    /// When the previous server tick started, for the timer's balance.
    last_tick: Option<Instant>,
    tick_rate: f32,
}

impl Anticheat {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            log: None,
            strikes: None,
            last_tick: None,
            tick_rate: 20.0,
        }
    }

    fn write_log(&mut self, line: &str) {
        if self.config.log.is_empty() {
            return;
        }
        if self.log.is_none() {
            let path = std::path::Path::new(&self.config.log);
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            self.log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok();
        }
        if let Some(file) = &mut self.log {
            let time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
            let _ = writeln!(file, "[{time}] {line}");
        }
    }

    /// Player `uuid` was kicked by `check`: its kicks by it so far and whether this one bans. The
    /// count is saved at once, so a restart doesn't forgive it.
    fn strike(&mut self, uuid: u128, check: Check) -> (u32, bool) {
        let path = std::path::Path::new(&self.config.strikes);
        let strikes = self.strikes.get_or_insert_with(|| {
            std::fs::read(path)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default()
        });
        let ban_after = self.config.actions(check).ban_after;
        let kick = strikes.kick(&uuid_text(uuid), check, ban_after);
        if !self.config.strikes.is_empty()
            && let Ok(json) = serde_json::to_vec_pretty(strikes)
        {
            let _ = std::fs::write(path, json);
        }
        kick
    }
}

/// A UUID as `banned-players.json` writes it.
fn uuid_text(uuid: u128) -> String {
    let h = format!("{uuid:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

/// Server ticks a claim of the ground keeps its height after the ground last held the player.
const GROUND_GRACE: u64 = 10;

/// Whether an entity of `kind` at `at` may carry a player standing at `player`: the entities players
/// stand on (boats, rafts, minecarts, shulkers, happy ghasts, 4 blocks tall), within 4 blocks sideways and
/// from a block above the player to a little more than a happy ghast's height below.
fn carries_player(kind: &str, at: [f64; 3], player: [f64; 3]) -> bool {
    let carrier = kind.ends_with("boat")
        || kind.ends_with("raft")
        || kind.contains("minecart")
        || kind == "minecraft:shulker"
        || kind == "minecraft:happy_ghast";
    let up = player[1] - at[1];
    carrier
        && (at[0] - player[0]).abs() < 4.0
        && (at[2] - player[2]).abs() < 4.0
        && (-1.0..=4.5).contains(&up)
}

fn text(s: &str, color: &str) -> Tag {
    Tag::compound([
        ("text", Tag::String(JavaString::from(s))),
        ("color", Tag::String(JavaString::from(color))),
    ])
}

impl Hub {
    /// Turn the anticheat on with these settings, or off (`config.enabled` false).
    pub fn set_anticheat(&mut self, config: Config) {
        self.anticheat = config.enabled.then(|| Box::new(Anticheat::new(config)));
    }

    /// The start of a server tick: the time since the previous one fills each player's timer balance.
    pub(super) fn anticheat_tick(&mut self, tick_rate: f32) {
        let Some(ac) = self.anticheat.as_deref_mut() else {
            return;
        };
        let now = Instant::now();
        let elapsed = ac.last_tick.replace(now).map(|t| now - t);
        if !ac.config.runs(Check::Timer) {
            return;
        }
        // A new tick rate reaches the client a little later: both start again.
        let restart = ac.tick_rate != tick_rate;
        ac.tick_rate = tick_rate;
        let ms = elapsed.map_or(0.0, |e| e.as_secs_f64() * 1000.0);
        for g in &mut self.guests {
            if restart {
                g.anticheat.timer.reset();
            }
            g.anticheat.timer.elapse(ms, &ac.config.timer);
        }
    }

    /// ServerboundClientTickEndPacket: one client tick spent.
    pub(super) fn anticheat_tick_end(&mut self, i: usize, events: &mut Vec<HostEvent>) {
        let Some(ac) = self.anticheat.as_deref() else {
            return;
        };
        if ac.config.runs(Check::Combat)
            && let Some(foul) = self.guests[i].anticheat.attacks.tick_end()
        {
            self.combat_foul(i, foul, events);
        }
        let Some(ac) = self.anticheat.as_deref() else {
            return;
        };
        if ac.config.runs(Check::Prediction) {
            let g = &mut self.guests[i];
            let (input, sprinting) = (g.input, g.anticheat.sprinting);
            g.anticheat
                .record
                .get_or_insert_default()
                .tick_end(input, sprinting);
        }
        if !ac.config.runs(Check::Timer) {
            return;
        }
        let (rate, config) = (ac.tick_rate, &ac.config.timer);
        if let Some(ahead) = self.guests[i].anticheat.timer.tick_end(rate, config) {
            self.flag(
                i,
                Check::Timer,
                1.0,
                &format!("{ahead:.0} ms ahead"),
                events,
            );
        }
    }

    /// ServerboundAttackPacket on entity `target`: `combat`. Returns whether the attack is dropped:
    /// from `setback` on, every attack is, until the level decays below it.
    pub(super) fn anticheat_attack(
        &mut self,
        i: usize,
        target: i32,
        events: &mut Vec<HostEvent>,
    ) -> bool {
        let Some(ac) = self.anticheat.as_deref() else {
            return false;
        };
        if !ac.config.runs(Check::Combat) {
            return false;
        }
        if let Some(foul) = self.guests[i].anticheat.attacks.attack(target)
            && self.combat_foul(i, foul, events)
        {
            return true;
        }
        let Some(ac) = self.anticheat.as_deref() else {
            return false;
        };
        let setback = ac.config.combat.setback;
        setback > 0.0
            && self.guests[i]
                .anticheat
                .vl(&ac.config, Check::Combat, self.ticks)
                >= setback
    }

    /// ServerboundPunchPacket: the arm swung, for `combat`.
    pub(super) fn anticheat_swing(&mut self, i: usize) {
        if self.anticheat.is_some() {
            self.guests[i].anticheat.attacks.swing();
        }
    }

    fn combat_foul(
        &mut self,
        i: usize,
        foul: horizon_anticheat::combat::Foul,
        events: &mut Vec<HostEvent>,
    ) -> bool {
        use horizon_anticheat::combat::Foul;
        let detail = match foul {
            Foul::NoSwing(n) => {
                format!("attacked without swinging ({n} more attacks than swings in a tick)")
            }
            Foul::TwoTargets(first, then) => {
                format!("attacked entities {first} and {then} in one tick")
            }
        };
        self.flag(i, Check::Combat, 1.0, &detail, events)
    }

    /// Whether the prediction records the players' client ticks.
    pub fn recording(&self) -> bool {
        self.anticheat
            .as_deref()
            .is_some_and(|ac| ac.config.runs(Check::Prediction))
    }

    /// A move packet from guest `i`, as it came: the prediction's record of the client tick.
    pub(super) fn anticheat_moved(
        &mut self,
        i: usize,
        pos: Option<[f64; 3]>,
        rot: Option<(f32, f32)>,
        flags: u8,
    ) {
        if self.recording() {
            let record = self.guests[i].anticheat.record.get_or_insert_default();
            record.moved(pos, rot, flags);
        }
    }

    /// ServerboundPongPacket: the client has handled every packet of the server tick `id` (the ping
    /// [`Hub::anticheat_ping`] sent at its end).
    pub(super) fn anticheat_pong(&mut self, i: usize, id: i32) {
        let ticks = self.ticks;
        if self.recording() {
            let record = self.guests[i].anticheat.record.get_or_insert_default();
            // Ids are server ticks, cut to 32 bits.
            record.pong(u64::from(id as u32), ticks);
        }
    }

    /// The end of a server tick, after everything it sent: a ping whose id is the tick to each
    /// player in the world, so the prediction knows from which client tick on that tick's packets
    /// were in effect.
    pub fn anticheat_ping(&mut self) {
        if !self.recording() {
            return;
        }
        // What the tick sent the players that changes their movement went out before this ping.
        let ticks = self.ticks;
        for g in &mut self.guests {
            let changes = std::mem::take(
                g.changes
                    .get_mut()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            if let Some(record) = g.anticheat.record.as_deref_mut() {
                for change in changes {
                    record.changed(ticks, change);
                }
            }
        }
        let ping = horizon_protocol::packets::play::Clientbound::Ping(
            horizon_protocol::packets::configuration::Ping {
                id: self.ticks as u32 as i32,
            },
        );
        for g in self.guests.iter().filter(|g| g.placed) {
            g.send(&ping);
        }
    }

    /// `ground`, for a move from guest `i` that says it ends on the ground at `to` (`stands`: the move
    /// itself was stopped by something below): None when the claim stands (or isn't judged, or the check
    /// only logs: `setback = 0`), else whether the move is refused because the player was kicked. A
    /// false claim is dropped (vanilla's fall damage follows); the player is never sent back, and the
    /// move doesn't count toward vanilla's floating kick. A claim at the height where the
    /// ground last held the player, within half a second, isn't judged: the block may have gone
    /// server-side (broken, pushed) before the client heard.
    pub(super) fn anticheat_ground(
        &mut self,
        i: usize,
        to: [f64; 3],
        stands: bool,
        ctx: &TickCtx,
        events: &mut Vec<HostEvent>,
    ) -> Option<bool> {
        if !self.anticheat.as_deref()?.config.runs(Check::Ground) {
            return None;
        }
        // Standing still: no fall to skip, whatever holds the player up (a block the server doesn't
        // have, seen on the official server after a long teleport). Hovering is vanilla's floating kick.
        if self.guests[i].position == to {
            return None;
        }
        let ticks = self.ticks;
        // Entity.move resolves Y first, where the body starts, then X and Z: on the tick a player
        // walks off an edge (or lands on one and slides off it), the client stands on what is under
        // the move's start. A vanilla client did that once a step down every slope and staircase.
        let from = self.guests[i].position;
        let held_at_start = || self.ground_holds(i, [from[0], to[1], from[2]], ctx) != Some(false);
        if stands || self.ground_holds(i, to, ctx)? || held_at_start() {
            self.guests[i].anticheat.ground = Some((ticks, to[1]));
            return None;
        }
        if self.guests[i].anticheat.ground.is_some_and(|(t, y)| {
            ticks.saturating_sub(t) <= GROUND_GRACE && (y - to[1]).abs() < 1.0E-3
        }) {
            return None;
        }
        let (fx, fz) = (to[0].floor() as i32, to[2].floor() as i32);
        let below = if self
            .encoder
            .has_fluid(ctx.world.block(fx, to[1].floor() as i32 - 1, fz).0)
        {
            "on a fluid"
        } else {
            "in the air"
        };
        let detail = format!(
            "claims the ground {below} at {:.1} {:.1} {:.1}",
            to[0], to[1], to[2]
        );
        self.flag(i, Check::Ground, 1.0, &detail, events);
        if self.anticheat.as_deref()?.config.ground.setback == 0.0 {
            return None;
        }
        // The correction is the dropped claim, never a setback: sent back into the air it claimed to
        // stand in, an honest player (on a block the server doesn't have) claims it again and again,
        // and hovers there until vanilla's floating kick. Only the anticheat's own kick ends it.
        Some(!self.guests[i].left_reason.is_empty())
    }

    /// Whether something at `to` can hold guest `i` up; None: not judged. Only blocks that can't hold
    /// anyone are judged: nothing but air and fluids around the feet, and no collision a tenth of a block
    /// below. Anything else (powder snow, scaffolding, a block the server doesn't have yet) and a boat,
    /// minecart, shulker or happy ghast under the player: not judged.
    fn ground_holds(&self, i: usize, to: [f64; 3], ctx: &TickCtx) -> Option<bool> {
        let g = &self.guests[i];
        // No fall damage to skip.
        if g.creative() || g.spectator() || g.fall_flying {
            return None;
        }
        let binding = ctx.collision?;
        if !ctx.world.loaded(to[0].floor() as i32, to[2].floor() as i32) {
            return None;
        }
        let half = f64::from(g.dimensions().width) / 2.0;
        let lo =
            [to[0] - half - 0.0625, to[1] - 0.6, to[2] - half - 0.0625].map(|v| v.floor() as i32);
        let hi = [to[0] + half + 0.0625, to[1], to[2] + half + 0.0625].map(|v| v.floor() as i32);
        for x in lo[0]..=hi[0] {
            for z in lo[2]..=hi[2] {
                for y in lo[1]..=hi[1] {
                    let state = ctx.world.block(x, y, z).0;
                    if !self.encoder.is_air(state) && !self.encoder.has_fluid(state) {
                        return None;
                    }
                }
            }
        }
        if ctx
            .entities
            .iter()
            .any(|e| carries_player(&e.kind, e.position, to))
        {
            return None;
        }
        // A waterlogged slab or stairs: their shape holds the player.
        let height = f64::from(g.dimensions().height);
        let feet = Aabb::new(
            Vec3::new(to[0] - half, to[1], to[2] - half),
            Vec3::new(to[0] + half, to[1] + height, to[2] + half),
        );
        let mut scene = SceneCollisions {
            world: ctx.world,
            binding,
            subject: None,
        };
        let down = Vec3::new(0.0, -0.1, 0.0);
        Some(
            scene
                .resolve(feet, down, 0.0, false)
                .is_ok_and(|done| done.y > down.y),
        )
    }

    /// `check` failed for guest `i`: raise its level, log it, and act on it. Returns whether the move or
    /// action is cancelled (the player is sent back, or kicked).
    pub(super) fn flag(
        &mut self,
        i: usize,
        check: Check,
        weight: f32,
        detail: &str,
        events: &mut Vec<HostEvent>,
    ) -> bool {
        self.flag_to(i, check, weight, detail, events, None)
    }

    /// `prediction`: guest `conn`'s ticks that landed off the replay. A setback sends the player
    /// back to `agreed`, where its moves last agreed.
    pub fn anticheat_prediction(
        &mut self,
        conn: u32,
        over: &super::prediction::Over,
        agreed: [f64; 3],
    ) -> Vec<HostEvent> {
        let mut events = Vec::new();
        if let Some(i) = self.guests.iter().position(|g| g.conn == conn) {
            let ticks = if over.ticks == 1 { "tick" } else { "ticks" };
            // The worst tick as played, so the log tells a lag, a glide or a ladder from a cheat.
            let worst = over
                .at_worst
                .map_or(String::new(), |p| format!("; {}", p.describe()));
            let detail = format!(
                "moved up to {:.3} blocks off what its keys allow ({} {ticks}{worst})",
                over.worst, over.ticks
            );
            let weight = over.weight;
            self.flag_to(
                i,
                Check::Prediction,
                weight,
                &detail,
                &mut events,
                Some(agreed),
            );
        }
        events
    }

    /// [`Hub::flag`], whose setback sends the player to `back` (where it was by default).
    fn flag_to(
        &mut self,
        i: usize,
        check: Check,
        weight: f32,
        detail: &str,
        events: &mut Vec<HostEvent>,
        back: Option<[f64; 3]>,
    ) -> bool {
        let ticks = self.ticks;
        let Some(ac) = self.anticheat.as_deref_mut() else {
            return false;
        };
        // Kicked or banned already, and leaving: what it still had queued (a lagging client's ticks)
        // is dropped, and counts no second strike in the session the first kick warned.
        if !self.guests[i].left_reason.is_empty() {
            return true;
        }
        let g = &mut self.guests[i];
        let v = g.anticheat.flag(&ac.config, check, weight, ticks);
        let (uuid, name) = (g.uuid, g.name.clone());
        let ban_after = ac.config.actions(check).ban_after;
        let (kicks, ban) = if v.kick {
            ac.strike(uuid, check)
        } else {
            (0, false)
        };
        let warned;
        let action = if ban {
            ", banned"
        } else if v.kick && ban_after > 0 {
            warned = format!(", kicked ({kicks} of {ban_after} before a ban)");
            &warned
        } else if v.kick {
            ", kicked"
        } else if v.setback && check == Check::Combat {
            ", attacks dropped"
        } else if v.setback && check == Check::Ground {
            ", claim dropped"
        } else if v.setback {
            ", sent back"
        } else {
            ""
        };
        let line = format!(
            "{name} failed {} (VL {:.1}): {detail}{action}",
            check.name(),
            v.vl
        );
        ac.write_log(&line);
        // A kick or a ban is always told, so an operator can undo one that was wrong.
        if v.alert || v.kick {
            events.push(HostEvent::Warn(format!("[Anticheat] {line}")));
            if ac.config.alerts {
                let level = ac.config.alert_level;
                let content = Tag::compound([
                    ("text", Tag::String(JavaString::from("[Anticheat] "))),
                    ("color", Tag::String(JavaString::from("red"))),
                    ("extra", Tag::List(vec![text(&line, "gray")])),
                ]);
                let packet = horizon_protocol::packets::play::Clientbound::SystemChat(
                    horizon_protocol::packets::play::SystemChat {
                        content,
                        overlay: false,
                    },
                );
                for op in self
                    .guests
                    .iter()
                    .filter(|o| o.placed && o.permission >= level)
                {
                    op.send(&packet);
                }
            }
        }
        if ban {
            // The game bans as `/ban` does, and its kick says so.
            events.push(HostEvent::Ban {
                uuid,
                name,
                reason: format!("Horizon anticheat: {}", check.name()),
            });
        } else if v.kick {
            self.anticheat_kick(self.guests[i].conn);
        } else if v.setback
            && !matches!(check, Check::Combat | Check::Ground)
            && self.guests[i].awaiting_teleport.is_none()
        {
            // A combat setback drops the attack (`anticheat_attack`), a ground one the claim
            // (`anticheat_ground`); the others send the player back.
            self.resend_teleport(i, back);
        }
        v.setback || v.kick
    }

    /// Disconnects `conn` for the anticheat.
    pub fn anticheat_kick(&mut self, conn: u32) {
        self.kick(conn, text("Kicked by the server's anticheat", "white"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_are_written_as_the_ban_list_writes_them() {
        assert_eq!(
            uuid_text(0x069a79f4_44e9_4726_a5be_fca90e38aaf5),
            "069a79f4-44e9-4726-a5be-fca90e38aaf5"
        );
    }

    #[test]
    fn a_happy_ghasts_rider_and_a_boats_are_carried() {
        // HappyGhast is 4 blocks tall: its rider stands 4.0 above it.
        assert!(carries_player(
            "minecraft:happy_ghast",
            [0.0, 70.0, 0.0],
            [1.5, 74.0, -1.0]
        ));
        assert!(carries_player(
            "minecraft:oak_boat",
            [0.0, 62.0, 0.0],
            [0.0, 62.5625, 0.0]
        ));
        assert!(!carries_player(
            "minecraft:zombie",
            [0.0, 70.0, 0.0],
            [0.0, 71.0, 0.0]
        ));
        assert!(!carries_player(
            "minecraft:happy_ghast",
            [0.0, 70.0, 0.0],
            [0.0, 80.0, 0.0]
        ));
    }
}
