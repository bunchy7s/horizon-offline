//! Speaking to servers of an older release: the first layer of the native protocol adapter (roadmap
//! `older-servers`). Horizon's packets are 26.3's; a server on another protocol numbers them differently.
//! This maps packet ids per connection state, both ways; a [`Session`] follows one connection's states
//! and renumbers what goes through it. [`ids`] renumbers the registry ids inside packets as the shared
//! readers decode them, and `translate` rewrites the packets whose layout changed. An older version is
//! translated a release at a time (26.1 to 26.2, then 26.2 to 26.3), with its ids mapped straight to the
//! pinned ones.
pub mod ids;
pub mod session;
mod translate;
pub mod v1_21_11;
mod v1_21_11_ids;
pub mod v26_1_2;
mod v26_1_2_ids;
pub mod v26_2;
mod v26_2_ids;

pub use session::Session;

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
    /// 26.1, 26.1.1 and 26.1.2: one protocol.
    V26_1,
    V1_21_11,
}

impl ServerVersion {
    /// Newest first.
    pub const ALL: [Self; 4] = [Self::Current, Self::V26_2, Self::V26_1, Self::V1_21_11];

    /// Whether this is `other` or an older release: the translation steps from `other` up apply.
    pub fn at_most(self, other: Self) -> bool {
        let age = |v: Self| Self::ALL.iter().position(|&a| a == v).unwrap_or(0);
        age(self) >= age(other)
    }

    /// The protocol number its handshake states.
    pub fn protocol(self) -> i32 {
        match self {
            Self::Current => crate::PROTOCOL_VERSION,
            Self::V26_2 => v26_2::PROTOCOL,
            Self::V26_1 => v26_1_2::PROTOCOL,
            Self::V1_21_11 => v1_21_11::PROTOCOL,
        }
    }

    /// The release name, as a server list shows it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Current => crate::GAME_VERSION,
            Self::V26_2 => v26_2::NAME,
            Self::V26_1 => "26.1",
            Self::V1_21_11 => v1_21_11::NAME,
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

    /// How this version numbers block states and registries, against the pinned one. `None` for the pinned
    /// version itself.
    pub fn ids(self) -> Option<&'static ids::Ids> {
        // Before 26.3 a tag slot display was a bare tag id.
        const BEFORE_26_3: ids::Quirks = ids::Quirks {
            tag_slot_display_is_id: true,
        };
        static V26_2: ids::Ids = ids::Ids {
            quirks: BEFORE_26_3,
            ..v26_2_ids::IDS
        };
        static V26_1: ids::Ids = ids::Ids {
            quirks: BEFORE_26_3,
            ..v26_1_2_ids::IDS
        };
        static V1_21_11: ids::Ids = ids::Ids {
            quirks: BEFORE_26_3,
            // 26.1 registered the cat, cow, pig and chicken sound variants among the others.
            entity_data_serializer: ids::Remap::Runs {
                up: &[
                    (0, 0, 22),
                    (22, 23, 1),
                    (23, 25, 4),
                    (27, 30, 1),
                    (28, 32, 11),
                ],
                down: &[
                    (0, 0, 22),
                    (23, 22, 1),
                    (25, 23, 4),
                    (30, 27, 1),
                    (32, 28, 11),
                ],
            },
            ..v1_21_11_ids::IDS
        };
        match self {
            Self::Current => None,
            Self::V26_2 => Some(&V26_2),
            Self::V26_1 => Some(&V26_1),
            Self::V1_21_11 => Some(&V1_21_11),
        }
    }

    fn map(self, state: State, direction: Direction, id: i32) -> Option<i32> {
        let table = match (self, direction) {
            (Self::Current, _) => return Some(id),
            (Self::V26_2, Direction::Clientbound) => v26_2::clientbound(state),
            (Self::V26_2, Direction::Serverbound) => v26_2::serverbound(state),
            (Self::V26_1, Direction::Clientbound) => v26_1_2::clientbound(state),
            (Self::V26_1, Direction::Serverbound) => v26_1_2::serverbound(state),
            (Self::V1_21_11, Direction::Clientbound) => v1_21_11::clientbound(state),
            (Self::V1_21_11, Direction::Serverbound) => v1_21_11::serverbound(state),
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
        assert_eq!(
            ServerVersion::from_protocol(775),
            Some(ServerVersion::V26_1)
        );
        assert_eq!(
            ServerVersion::from_protocol(774),
            Some(ServerVersion::V1_21_11)
        );
        assert_eq!(ServerVersion::from_protocol(773), None);
        assert!(ServerVersion::V26_1.at_most(ServerVersion::V26_2));
        assert!(!ServerVersion::V26_2.at_most(ServerVersion::V26_1));
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
