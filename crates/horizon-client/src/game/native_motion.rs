//! The player's motion for mods (docs/native-mods.md, "A player's motion"): a push for the local player, on a
//! client, and a player's creative flying speed, on a server.
use super::Game;

/// Each component of a push is clamped to this, in blocks a tick.
const PUSH_MAX: f32 = 4.0;
/// The fastest flying speed a mod may set: 1000 times vanilla's 0.05, for crossing a world's distances in seconds.
/// It only stops a stray value from throwing the player millions of blocks in a tick; past about 2 the player
/// outruns the chunks loading. Past about 0.45 (sprinting; 0.9 without) a player flies more than 10 blocks a
/// tick: the single-player owner is never checked, as in vanilla, but anyone else meets the server's moved-too-quickly
/// check, which can send them back (`Hub::position_change`, `ServerGamePacketListenerImpl.handleMovePlayer`).
const FLYING_MAX: f32 = 50.0;
/// Vanilla's creative flying speed (`Abilities.flyingSpeed`).
const FLYING_VANILLA: f32 = 0.05;

/// The velocity after a push: `kick` added, each component clamped; none when one of `kick`'s isn't a number.
fn pushed(velocity: [f64; 3], kick: [f32; 3]) -> Option<[f64; 3]> {
    if !kick.iter().all(|v| v.is_finite()) {
        return None;
    }
    Some(std::array::from_fn(|i| {
        velocity[i] + f64::from(kick[i].clamp(-PUSH_MAX, PUSH_MAX))
    }))
}

#[allow(dead_code)]
impl Game<'_> {
    /// For a client mod: a push added to the local player's velocity, in blocks a tick, as an
    /// explosion's knockback is (the window's player moves itself, as vanilla's client does). Each component is
    /// clamped to -4..4; a spectator, or a value that is not a number, is left alone. API 105.
    pub(super) fn native_push_player(&mut self, kick: [f32; 3]) {
        if let Some(p) = self.player.as_mut()
            && !p.spectator
            && let Some([x, y, z]) = pushed([p.velocity.x, p.velocity.y, p.velocity.z], kick)
        {
            p.velocity = horizon_core::geometry::Vec3::new(x, y, z);
            if kick[1] > 0.0 {
                p.ground.on_ground = false;
            }
        }
    }
    /// For a server mod: `player`'s creative flying speed (vanilla's 0.05 when none, at most `FLYING_MAX`), kept on
    /// the server's player and sent to their client. A respawn or a game mode change keeps it, and it is saved
    /// with the player (`abilities.flySpeed`, as vanilla saves it). False for a player not here or a client of a
    /// server, and inside a player's hook, where the server's player list is busy: call it from a tick hook.
    /// API 105.
    pub(super) fn native_set_flying_speed(&mut self, player: u128, speed: Option<f32>) -> bool {
        let speed = speed
            .filter(|s| s.is_finite())
            .map_or(FLYING_VANILLA, |s| s.clamp(0.0, FLYING_MAX));
        let Some(host) = self.lan.as_mut() else {
            return false;
        };
        let Some(conn) = host
            .hub
            .guests
            .iter()
            .find(|g| g.uuid == player && g.placed)
            .map(|g| g.conn)
        else {
            return false;
        };
        host.hub.set_flying_speed(conn, speed);
        true
    }
}

#[cfg(test)]
mod motion_tests {
    use super::*;

    #[test]
    fn a_push_adds_to_the_velocity_clamped() {
        assert_eq!(
            pushed([0.1, -0.08, 0.0], [0.8, 0.55, -0.5]),
            Some([0.1 + 0.8f32 as f64, -0.08 + 0.55f32 as f64, -0.5])
        );
        assert_eq!(pushed([0.0; 3], [9.0, -9.0, 0.0]), Some([4.0, -4.0, 0.0]));
        assert_eq!(pushed([1.0; 3], [f32::NAN, 0.0, 0.0]), None);
        assert_eq!(pushed([1.0; 3], [0.0, f32::INFINITY, 0.0]), None);
    }
}
