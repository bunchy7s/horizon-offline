//! The movement prediction's replay on the server (`lan::prediction`): each guest's client ticks
//! played again with the client's own movement code, in the guest's level.
use super::Game;
use super::lan::LanHost;
use horizon_client::lan::entity::EntityState;
use horizon_client::lan::guest::Guest;
use horizon_client::lan::prediction::{Replay, near, slack};
use horizon_client::motion::{MotionWorld, PlayerMotion};
use horizon_client::world::World;

impl Game<'_> {
    /// Guest `conn`'s client ticks since the server's last tick, played again, before the server
    /// moves the guest's own body.
    pub(super) fn guest_replay(&mut self, host: &mut LanHost, conn: u32) {
        if !host.hub.recording() {
            return;
        }
        let tolerance = host
            .hub
            .anticheat
            .as_deref()
            .map_or(f64::INFINITY, |ac| ac.config.prediction.tolerance);
        // The item the guest is using (eating, a bow, a shield): its client moves at the item's
        // `use_effects` speed (ItemUseModifier, 0.2 for most), and may not sprint. The server's copy
        // of a guest doesn't follow it (Game.sync_player_body is the window's player's), so the
        // replay walked a guest eating at full speed: 0.173 blocks a tick off, the live log's most
        // common flag.
        let item_use = host
            .play
            .get(&conn)
            .and_then(|play| play.using())
            .and_then(|(slot, _)| {
                let g = host.hub.guest(conn)?;
                let stack = if slot == super::shield::OFFHAND_SLOT {
                    g.offhand.as_ref()
                } else {
                    g.inventory.get(slot)?.as_ref()
                }?;
                self.res.items().get(&stack.item)
            })
            .map(|item| (item.use_speed, !item.can_sprint));
        let Some(g) = host.hub.guest_mut(conn) else {
            return;
        };
        // What moves the player isn't its own movement (a vehicle, a bed), or the server holds it:
        // the replay starts again at the teleport that ends it (dismountTo, stopSleeping, respawn).
        let held =
            g.vehicle.is_some() || g.sleeping || g.dead || g.spectator() || !g.client_loaded();
        // A teleport not yet accepted (a setback): the client still moves on its own until it
        // hears it, and the replay follows, but nothing counts.
        let waiting = g.awaiting_teleport.is_some();
        // The server's copy of a guest doesn't follow its armor (Game.glider_worn).
        let hand = |stack: Option<&horizon_client::items::ItemStack>, slot: &str| {
            stack.is_some_and(|s| {
                horizon_client::item_sync::can_glide_using(s, slot, self.res.items())
            })
        };
        let glider = super::gliding_pieces(self.res.items(), &g.armor).contains(&true)
            || hand(
                g.inventory.get(g.selected).and_then(Option::as_ref),
                "mainhand",
            )
            || hand(g.offhand.as_ref(), "offhand");
        if let Some(motion) = g.motion.as_deref_mut() {
            motion.glider = glider;
            motion.item_use_speed = item_use.map(|(speed, _)| speed);
            motion.item_use_slows_sprint = item_use.is_some_and(|(_, slows)| slows);
        }
        let (Some(record), Some(server)) = (g.anticheat.record.as_deref_mut(), g.motion.as_deref())
        else {
            return;
        };
        let mut run = |body: &mut PlayerMotion, keys, sprinting| {
            // Game.sync_player_body's part that depends on the blocks.
            body.nudge = super::closest_space_nudge(body, &self.world, self.res.facts());
            MotionWorld {
                entities: &[],
                player: body,
                scene: &mut self.world,
                assets: &self.pack.assets,
                collision_binding: self.collision,
                resources: self.motion,
            }
            .replayed_tick(keys, sprinting)
        };
        if let Some(restart) = record.take_restart() {
            // What the client keeps through a teleport (its jump delay, a glide...) is the last
            // replay's, once it has played the ticks before it. The client's motion through the
            // teleport is its own (an End gateway's), and a replay that was excused when the teleport
            // came may be off in what it can't see: either way the replay finds the client again.
            let mut excused = restart.kept_motion;
            let mut replay = g
                .replay
                .take()
                .and_then(|mut replay| {
                    let finished = replay.finish(record, server, &restart, &mut run);
                    excused |= replay.excused();
                    finished.then_some(replay)
                })
                .unwrap_or_else(|| Box::new(Replay::new(server, &restart)));
            replay.restart(&restart);
            if excused {
                replay.excuse();
            }
            g.replay = Some(replay);
        }
        if held {
            g.replay = None;
            return;
        }
        let Some(replay) = g.replay.as_deref_mut() else {
            return;
        };
        // Ticks no longer kept (a server that stalled for seconds): wait for the next teleport.
        let Some(over) = replay.catch_up(record, server, tolerance, &mut run) else {
            g.replay = None;
            return;
        };
        if over.ticks == 0 || waiting {
            return;
        }
        let g = host.hub.guest(conn).expect("found above");
        // A client that hasn't said it loaded the level is still on its loading screen, where it
        // doesn't move (the server stopped waiting for it after 60 ticks): the replay falls or drifts
        // where it stands. Its first move counts as any other.
        let loading = g.loading && !over.moved;
        if loading || excused(g, &self.world, host.states.as_slice(), &host.hub.guests) {
            if let Some(replay) = host
                .hub
                .guest_mut(conn)
                .and_then(|g| g.replay.as_deref_mut())
            {
                replay.excuse();
            }
            return;
        }
        let agreed = g.replay.as_deref().expect("played above").agreed();
        for event in host.hub.anticheat_prediction(conn, &over, agreed) {
            self.lan_log_event(event);
        }
    }
}

/// Whether guest `g`'s last moves may be off for a reason the replay can't play yet: an entity near
/// it (a push, a boat or shulker to stand on, a firework it flies with); a block change near it the
/// server hasn't sent yet (one sent but unseen, or an entity gone that the client may still have, the
/// replay excuses itself); a moving piston; a chunk around it the client doesn't have; a riptide
/// launch (the trident's push is the client's own).
fn excused(g: &Guest, world: &World, entities: &[EntityState], guests: &[Guest]) -> bool {
    let at = g.position;
    let slack = slack(std::array::from_fn(|i| at[i] - g.previous_position[i]));
    let close = |p: [f64; 3]| {
        let r = 3.0 + slack;
        (p[0] - at[0]).abs() <= r
            && (p[2] - at[2]).abs() <= r
            && (-3.0 - slack..=4.0 + slack).contains(&(p[1] - at[1]))
    };
    let unsent = world
        .block_log
        .as_ref()
        .is_some_and(|log| log.iter().any(|&(x, y, z)| near(at, [x, y, z], slack)));
    let pistons = world
        .moving_pistons
        .iter()
        .any(|&(p, _)| near(at, p, slack));
    // A client doesn't move in a chunk it doesn't have.
    let (cx, cz) = ((at[0].floor() as i32) >> 4, (at[2].floor() as i32) >> 4);
    let unloaded =
        (-1..=1).any(|dx| (-1..=1).any(|dz| !g.sent_chunks.contains(&(cx + dx, cz + dz))));
    let riptide = g
        .motion
        .as_deref()
        .is_some_and(|m: &PlayerMotion| m.auto_spin_attack_ticks > 0);
    // Items and experience orbs neither push nor hold anyone up; the list holds the player itself.
    let entity = entities.iter().any(|e| {
        e.id != g.entity_id
            && !matches!(
                e.kind.as_str(),
                "minecraft:item" | "minecraft:experience_orb"
            )
            && close(e.position)
    });
    let player = guests
        .iter()
        .any(|o| o.conn != g.conn && o.placed && o.dimension == g.dimension && close(o.position));
    unsent || pistons || unloaded || riptide || entity || player
}
