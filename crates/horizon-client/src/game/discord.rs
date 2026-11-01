//! What the player is doing, for Discord Rich Presence (`horizon_client::discord_presence`).
use super::*;
use horizon_client::discord_presence::{self as presence, Dimension, Play, Status};

impl Game<'_> {
    /// Each frame: only a change reaches the presence thread.
    pub(super) fn update_discord_presence(&self) {
        // One Discord status per computer: P1's, not a splitscreen player's.
        if self.options.split_guest {
            return;
        }
        presence::set(self.discord_status());
    }
    fn discord_status(&self) -> Status {
        if !self.settings.extras.discord_presence {
            return Status::Off;
        }
        if self.title.is_some() {
            return Status::Menus;
        }
        let play = if let Some(host) = &self.lan {
            Play::Hosting {
                players: 1 + host.hub.guests.len(),
            }
        } else if let Some(server) = &self.integrated {
            match (server.published(), &self.remote) {
                (Some(_), Some(remote)) => Play::Hosting {
                    players: remote.player_count(),
                },
                _ => Play::Singleplayer,
            }
        } else if self.remote.is_some() {
            Play::Multiplayer
        } else {
            Play::Singleplayer
        };
        Status::World {
            play,
            dimension: match self.dim {
                Dim::Overworld => Dimension::Overworld,
                Dim::Nether => Dimension::Nether,
                Dim::End => Dimension::End,
            },
            mode: self.mode,
        }
    }
}
