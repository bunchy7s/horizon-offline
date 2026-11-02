//! Speaking to servers of an older release: the first layer of the native protocol adapter (roadmap
//! `older-servers`). Horizon's packets are 26.3's; a server on another protocol numbers them differently.
//! This maps packet ids per connection state, both ways. It does not translate packet contents.
pub mod v26_2;

use crate::{
    Direction, Result, State,
    buf::{Reader, write_varint},
};

/// The protocol a server speaks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ServerVersion {
    /// The pinned version ([`crate::PROTOCOL_VERSION`]): nothing to translate.
    #[default]
    Current,
    V26_2,
}

impl ServerVersion {
    pub const ALL: [Self; 2] = [Self::Current, Self::V26_2];

    /// The protocol number its handshake states.
    pub fn protocol(self) -> i32 {
        match self {
            Self::Current => crate::PROTOCOL_VERSION,
            Self::V26_2 => v26_2::PROTOCOL,
        }
    }

    /// The release name, as a server list shows it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Current => crate::GAME_VERSION,
            Self::V26_2 => v26_2::NAME,
        }
    }

    /// The version a server's status reports by its protocol number.
    pub fn from_protocol(protocol: i32) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.protocol() == protocol)
    }

    /// The pinned version's id for a packet this version's server sent as `id`. `None` when the pinned
    /// version has no such packet.
    pub fn clientbound_id(self, state: State, id: i32) -> Option<i32> {
        self.map(state, Direction::Clientbound, id)
    }

    /// The id this version's server knows a packet Horizon sends as `id` by. `None` when that server has
    /// no such packet, so it must not be sent.
    pub fn serverbound_id(self, state: State, id: i32) -> Option<i32> {
        self.map(state, Direction::Serverbound, id)
    }

    fn map(self, state: State, direction: Direction, id: i32) -> Option<i32> {
        let table = match (self, direction) {
            (Self::Current, _) => return Some(id),
            (Self::V26_2, Direction::Clientbound) => v26_2::clientbound(state),
            (Self::V26_2, Direction::Serverbound) => v26_2::serverbound(state),
        };
        let mapped = *table.get(usize::try_from(id).ok()?)?;
        mapped.map(i32::from)
    }

    /// A packet body (VarInt id + payload) Horizon sends, renumbered for this version's server. `None`
    /// when that server has no such packet.
    pub fn serverbound_body(self, state: State, body: &[u8]) -> Result<Option<Vec<u8>>> {
        let mut r = Reader::new(body);
        let id = r.varint()?;
        let payload = r.rest();
        let Some(mapped) = self.serverbound_id(state, id) else {
            return Ok(None);
        };
        if mapped == id {
            return Ok(Some(body.to_vec()));
        }
        let mut out = Vec::with_capacity(body.len() + 1);
        write_varint(&mut out, mapped);
        out.extend_from_slice(payload);
        Ok(Some(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids;

    // Expected ids below are read off the 26.2 and 26.3 packets reports by hand
    // (`version_port.py diff 26.2 26.3`), not from the generated tables.

    #[test]
    fn a_26_2_server_is_known_by_its_protocol() {
        assert_eq!(
            ServerVersion::from_protocol(776),
            Some(ServerVersion::V26_2)
        );
        assert_eq!(
            ServerVersion::from_protocol(crate::PROTOCOL_VERSION),
            Some(ServerVersion::Current)
        );
        assert_eq!(ServerVersion::from_protocol(775), None);
    }

    #[test]
    fn renumbered_clientbound_play_packets_reach_their_26_3_ids() {
        let v = ServerVersion::V26_2;
        // keep_alive 44 -> 45, login 49 -> 50, respawn 82 -> 84, clear_dialog 139 -> 142.
        assert_eq!(
            v.clientbound_id(State::Play, 44),
            Some(ids::play::clientbound::KEEP_ALIVE)
        );
        assert_eq!(
            v.clientbound_id(State::Play, 49),
            Some(ids::play::clientbound::LOGIN)
        );
        assert_eq!(
            v.clientbound_id(State::Play, 82),
            Some(ids::play::clientbound::RESPAWN)
        );
        assert_eq!(
            v.clientbound_id(State::Play, 139),
            Some(ids::play::clientbound::CLEAR_DIALOG)
        );
        // Unchanged before the first insertion (add_transient_block at 37).
        assert_eq!(v.clientbound_id(State::Play, 0), Some(0));
        // Past 26.2's 141 packets.
        assert_eq!(v.clientbound_id(State::Play, 141), None);
        // Configuration: store_cookie 10 -> 11 (post_effects took 10).
        assert_eq!(
            v.clientbound_id(State::Configuration, 10),
            Some(ids::configuration::clientbound::STORE_COOKIE)
        );
    }

    #[test]
    fn serverbound_packets_26_2_lacks_are_not_sent() {
        let v = ServerVersion::V26_2;
        // punch is new in 26.3; swing, which 26.2 has at 63, is gone.
        assert_eq!(
            v.serverbound_id(State::Play, ids::play::serverbound::PUNCH),
            None
        );
        // resource_pack 50 -> 49, spectator_action 63 -> 62, keep_alive unchanged.
        assert_eq!(
            v.serverbound_id(State::Play, ids::play::serverbound::RESOURCE_PACK),
            Some(49)
        );
        assert_eq!(
            v.serverbound_id(State::Play, ids::play::serverbound::SPECTATOR_ACTION),
            Some(62)
        );
        assert_eq!(
            v.serverbound_id(State::Play, ids::play::serverbound::KEEP_ALIVE),
            Some(ids::play::serverbound::KEEP_ALIVE)
        );
        assert_eq!(
            v.serverbound_id(State::Handshake, ids::handshake::serverbound::INTENTION),
            Some(0)
        );
    }

    #[test]
    fn serverbound_bodies_keep_their_payload_under_the_new_id() {
        let v = ServerVersion::V26_2;
        let mut body = Vec::new();
        write_varint(&mut body, ids::play::serverbound::RESOURCE_PACK);
        body.extend_from_slice(&[1, 2, 3]);
        assert_eq!(
            v.serverbound_body(State::Play, &body).unwrap(),
            Some(vec![49, 1, 2, 3])
        );
        let mut punch = Vec::new();
        write_varint(&mut punch, ids::play::serverbound::PUNCH);
        assert_eq!(v.serverbound_body(State::Play, &punch).unwrap(), None);
        assert_eq!(
            ServerVersion::Current
                .serverbound_body(State::Play, &body)
                .unwrap(),
            Some(body)
        );
    }
}
