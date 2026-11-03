//! Far terrain for a server's Horizon clients (`horizon_client::lod_net`): a
//! `horizon:lod` request is checked against the player's limits and position,
//! then sampled from the level the player is in on the far terrain thread.
//! The replies come back here and leave with the tick's other packets. A
//! refused request is answered at once, so the client knows to ask again.
use super::{Dim, Game, lan::LanHost};
use horizon_client::{
    lan::conn::PlayerSender,
    lod_net::{CHANNEL, Delivery, Job, Verdict},
};
use horizon_protocol::{
    packets::{configuration::CustomPayload, play},
    text::translatable,
};
use std::sync::atomic::Ordering;

/// Bytes a player's connection may have waiting before far terrain holds its
/// replies back: a client that reads slowly gets the game's packets first.
const BACKLOG: usize = 1 << 20;

fn send(sender: &PlayerSender, data: Vec<u8>) {
    let packet = play::Clientbound::CustomPayload(CustomPayload {
        channel: CHANNEL.into(),
        data,
    });
    sender.send(packet.encode());
}

impl Game<'_> {
    /// A `horizon:lod` payload from `conn`.
    pub(super) fn guest_far_terrain(&mut self, host: &mut LanHost, conn: u32, data: &[u8]) {
        let Some(far) = host.far.as_mut() else {
            return;
        };
        // Only the clients that heard the offer (lan::conn) may ask.
        let Some(guest) = host.hub.guest(conn).filter(|g| g.brand == "horizon") else {
            return;
        };
        // Players who left are forgotten once they could matter.
        if far.players() > host.hub.guests.len() + 8 {
            let guests = &host.hub.guests;
            far.retain(|c| guests.iter().any(|g| g.conn == c));
        }
        let dim = Dim::from_id(&guest.dimension);
        let source = self.lod_source_in(dim);
        let now = std::time::Instant::now();
        match far.request(conn, data, guest.position, source.is_some(), now) {
            Verdict::Serve(request) => far.push(Job {
                conn,
                level: dim.id().to_owned(),
                source: source.expect("served when open"),
                request,
            }),
            Verdict::Refuse(reply) => send(&guest.sender, reply),
            Verdict::Ignore => {}
            Verdict::Kick => host.hub.kick(
                conn,
                translatable("multiplayer.disconnect.invalid_packet", &[]),
            ),
        }
    }

    /// The tiles the far terrain thread finished go to their players, once
    /// their connections have room.
    pub(super) fn flush_far_terrain(&mut self, host: &mut LanHost) {
        let Some(far) = host.far.as_mut() else {
            return;
        };
        let guests = &host.hub.guests;
        far.deliver(|conn, data| match guests.iter().find(|g| g.conn == conn) {
            None => Delivery::Gone,
            Some(g) if g.sender.pending.load(Ordering::Relaxed) > BACKLOG => Delivery::Busy,
            Some(g) => {
                send(&g.sender, std::mem::take(data));
                Delivery::Sent
            }
        });
    }
}
