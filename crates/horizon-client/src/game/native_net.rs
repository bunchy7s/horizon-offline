//! Mod messages for native mods (docs/native-mods.md, "Mod messages", API
//! 23): a mod's server half and its client half send each other bytes
//! (`horizon_protocol::mod_net`). The hub is out of the game while a guest's
//! packet or turn runs, so both directions wait here: what a guest sent
//! reaches `native_net_from_client` once the hub is back, and what a mod sends
//! a player leaves at the tick's end, from whichever hook sent it.
use super::{Game, lan::LanHost};
use horizon_protocol::{
    mod_net::{self, Message},
    packets::{
        configuration::CustomPayload,
        play::{Clientbound as C, Serverbound as S},
    },
};
use std::collections::HashMap;

/// Messages a tick at most: from one player, to one player, and from the
/// window to its server.
pub(super) const PER_TICK: u32 = 32;

/// The messages waiting, and what each player used of this tick's allowance.
#[derive(Default)]
pub(super) struct NativeNet {
    from_players: Vec<(u128, Message)>,
    from_counts: HashMap<u128, u32>,
    to_players: Vec<(u128, Vec<u8>)>,
    to_counts: HashMap<u128, u32>,
    pub(super) to_server: u32,
}

/// A message's payload as `native_net_send_*` check it.
fn payload(channel: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    if !mod_net::channel_ok(channel) {
        return Err(format!("the channel {channel:?} is not mod_id:name"));
    }
    mod_net::encode(&Message {
        channel: channel.to_owned(),
        data: data.to_vec(),
    })
    .map_err(|e| e.to_string())
}

impl Game<'_> {
    /// A `horizon:mod_net` payload from `conn`: queued for
    /// `run_native_net`. Past the caps it is dropped in silence.
    pub(super) fn guest_native_net(&mut self, host: &LanHost, conn: u32, data: &[u8]) {
        let Some(player) = host.hub.guest(conn).map(|g| g.uuid) else {
            return;
        };
        let count = self.native_net.from_counts.entry(player).or_default();
        if *count >= PER_TICK {
            return;
        }
        *count += 1;
        if let Ok(message) = mod_net::decode(data) {
            self.native_net.from_players.push((player, message));
        }
    }

    /// Whether messages from players wait for `run_native_net`.
    pub(super) fn native_net_waiting(&self) -> bool {
        !self.native_net.from_players.is_empty()
    }

    /// The waiting messages reach the mods, with the hub back in the game.
    pub(super) fn run_native_net(&mut self) {
        for (player, m) in std::mem::take(&mut self.native_net.from_players) {
            self.native_net_from_client(player, &m.channel, &m.data);
        }
    }

    /// What the mods sent this tick leaves; the tick's allowances start over.
    pub(super) fn flush_native_net(&mut self, host: &mut LanHost) {
        for (player, data) in std::mem::take(&mut self.native_net.to_players) {
            if let Some(g) = host
                .hub
                .guests
                .iter()
                .find(|g| g.placed && g.uuid == player && g.brand == "horizon")
            {
                host.hub.send_to(
                    g.conn,
                    &C::CustomPayload(CustomPayload {
                        channel: mod_net::CHANNEL.into(),
                        data,
                    }),
                );
            }
        }
        // The allowance is a server tick's, not a level pass's: it starts over
        // after the last level.
        if self.level_pass.last {
            self.native_net.from_counts.clear();
            self.native_net.to_counts.clear();
        }
    }

    /// A `horizon:mod_net` payload the server sent the window.
    pub(super) fn remote_native_net(&mut self, data: &[u8]) {
        match mod_net::decode(data) {
            Ok(m) => self.native_net_from_server(&m.channel, &m.data),
            Err(e) => log::debug!("Horizon: a bad mod message: {e}"),
        }
    }
}

// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// Sends `data` on `channel` (`mod_id:name`, at most 30 000 bytes, at
    /// most 32 a player a tick) to `player`'s client, at the tick's end, from
    /// any server hook. Ok means accepted: a player who left or isn't on a
    /// Horizon client doesn't get it. API 40.
    pub(super) fn native_net_send_to_player(
        &mut self,
        player: u128,
        channel: &str,
        data: &[u8],
    ) -> Result<(), String> {
        if self.remote.is_some() {
            return Err("a client has no players to send to".into());
        }
        let payload = payload(channel, data)?;
        let count = self.native_net.to_counts.entry(player).or_default();
        if *count >= PER_TICK {
            return Err(format!("past {PER_TICK} messages to this player this tick"));
        }
        *count += 1;
        self.native_net.to_players.push((player, payload));
        Ok(())
    }

    /// The client half: sends `data` on `channel` to the server (the same
    /// caps, 32 a tick). An error when the server doesn't carry mod messages
    /// (a vanilla server), and until the player has joined: a server says it
    /// carries them during configuration, before it can take one. API 40.
    pub(super) fn native_net_send_to_server(
        &mut self,
        channel: &str,
        data: &[u8],
    ) -> Result<(), String> {
        // Not joined first: the announcement may still be on its way, and a
        // mod that gives up on "carries no mod messages" never tries again.
        let Some(remote) = self.remote.as_ref() else {
            return Err("the server carries no mod messages".into());
        };
        // A server that reconfigures the player has them out of the play state.
        if !remote.joined || remote.reconfiguring.is_some() {
            return Err("the player has not joined yet".into());
        }
        if !remote.mod_net {
            return Err("the server carries no mod messages".into());
        }
        let payload = payload(channel, data)?;
        if self.native_net.to_server >= PER_TICK {
            return Err(format!("past {PER_TICK} messages this tick"));
        }
        self.native_net.to_server += 1;
        self.send(S::CustomPayload(CustomPayload {
            channel: mod_net::CHANNEL.into(),
            data: payload,
        }));
        Ok(())
    }
}
