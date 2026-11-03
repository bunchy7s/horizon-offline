//! One connection to a server of an older release: which state each direction is in, followed from the
//! packets themselves, and every packet renumbered and translated on its way through.
//!
//! The states change where vanilla's `Connection` switches its codecs: the handshake's intent picks status
//! or login for both directions, `login_finished` and `login_acknowledged` move each direction to
//! configuration, `finish_configuration` to play, and `start_configuration` / `configuration_acknowledged`
//! back to configuration.
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Mutex,
        atomic::{AtomicU8, Ordering},
    },
};

use super::{ServerVersion, translate};
use crate::{
    Result, State,
    buf::{Reader, Writer},
    ids,
    packets::handshake,
};

fn to_u8(state: State) -> u8 {
    match state {
        State::Handshake => 0,
        State::Status => 1,
        State::Login => 2,
        State::Configuration => 3,
        State::Play => 4,
    }
}
fn from_u8(v: u8) -> State {
    match v {
        1 => State::Status,
        2 => State::Login,
        3 => State::Configuration,
        4 => State::Play,
        _ => State::Handshake,
    }
}

/// Shared by a connection's reader and writer.
pub struct Session {
    pub version: ServerVersion,
    reading: AtomicU8,
    writing: AtomicU8,
    /// Entity UUIDs by network id, for a server that is told which entity a spectator picked by its UUID.
    pub(super) uuids: Mutex<HashMap<i32, u128>>,
    /// Entity types (pinned ids) by network id, for entity data laid out by type.
    pub(super) kinds: Mutex<HashMap<i32, i32>>,
    /// Packets the translation made, for Horizon to read before the next one from the server
    /// (pinned id, payload).
    pub(super) incoming: Mutex<VecDeque<(i32, Vec<u8>)>>,
    /// Packet bodies to send right after the one being sent.
    pub(super) outgoing: Mutex<Vec<Vec<u8>>>,
}

impl Session {
    pub fn new(version: ServerVersion) -> Self {
        Self {
            version,
            reading: AtomicU8::new(0),
            writing: AtomicU8::new(0),
            uuids: Mutex::new(HashMap::new()),
            kinds: Mutex::new(HashMap::new()),
            incoming: Mutex::new(VecDeque::new()),
            outgoing: Mutex::new(Vec::new()),
        }
    }
    pub fn reading(&self) -> State {
        from_u8(self.reading.load(Ordering::Acquire))
    }
    pub fn writing(&self) -> State {
        from_u8(self.writing.load(Ordering::Acquire))
    }
    fn set_reading(&self, state: State) {
        self.reading.store(to_u8(state), Ordering::Release);
    }
    fn set_writing(&self, state: State) {
        self.writing.store(to_u8(state), Ordering::Release);
    }

    /// A packet the server sent, as the pinned version numbers and lays it out. `None` drops it: Horizon
    /// knows no such packet.
    pub fn clientbound(&self, id: i32, payload: Vec<u8>) -> Result<Option<(i32, Vec<u8>)>> {
        let state = self.reading();
        let Some(pinned) = self.version.clientbound_id(state, id) else {
            return Ok(None);
        };
        match (state, pinned) {
            (State::Login, ids::login::clientbound::LOGIN_FINISHED) => {
                self.set_reading(State::Configuration)
            }
            (State::Configuration, ids::configuration::clientbound::FINISH_CONFIGURATION) => {
                self.set_reading(State::Play)
            }
            (State::Play, ids::play::clientbound::START_CONFIGURATION) => {
                self.set_reading(State::Configuration)
            }
            _ => {}
        }
        // A packet the translation can't read goes on as it came, as an unknown packet would: Horizon's
        // own reader then skips it or reports it, rather than the connection dropping here.
        match translate::clientbound(self, state, pinned, &payload) {
            Ok(Some((id, Some(translated)))) => Ok(Some((id, translated))),
            Ok(Some((id, None))) => Ok(Some((id, payload))),
            Ok(None) => Ok(None),
            Err(e) => {
                log::warn!(
                    "Horizon: could not translate {} packet {pinned} ({state:?}): {e}",
                    self.version.name()
                );
                Ok(Some((pinned, payload)))
            }
        }
    }

    /// A packet body (VarInt id + payload) Horizon sends, as this server numbers and lays it out. `None`:
    /// the server has no such packet, so nothing is sent.
    pub fn serverbound(&self, body: &[u8]) -> Result<Option<Vec<u8>>> {
        let state = self.writing();
        let mut r = Reader::new(body);
        let id = r.varint()?;
        let payload = r.rest();
        match (state, id) {
            (State::Handshake, ids::handshake::serverbound::INTENTION) => {
                return self.intention(payload).map(Some);
            }
            (State::Login, ids::login::serverbound::LOGIN_ACKNOWLEDGED) => {
                self.set_writing(State::Configuration)
            }
            (State::Configuration, ids::configuration::serverbound::FINISH_CONFIGURATION) => {
                self.set_writing(State::Play)
            }
            (State::Play, ids::play::serverbound::CONFIGURATION_ACKNOWLEDGED) => {
                self.set_writing(State::Configuration)
            }
            _ => {}
        }
        let Some((mapped, payload)) = translate::serverbound(self, state, id, payload)? else {
            return Ok(None);
        };
        let mut w = Writer::new();
        w.varint(mapped);
        w.bytes(&payload);
        Ok(Some(w.data))
    }

    /// A packet the translation made that Horizon reads next, already in the pinned version's numbering.
    pub fn next_incoming(&self) -> Option<(i32, Vec<u8>)> {
        self.incoming.lock().unwrap().pop_front()
    }
    /// Bodies the translation added after the packet just sent, in the server's numbering.
    pub fn take_outgoing(&self) -> Vec<Vec<u8>> {
        std::mem::take(&mut *self.outgoing.lock().unwrap())
    }

    /// The handshake states this server's protocol, and its intent moves both directions on.
    fn intention(&self, payload: &[u8]) -> Result<Vec<u8>> {
        let mut r = Reader::new(payload);
        let _pinned = r.varint()?;
        let host = r.string(255)?;
        let port = r.u16()?;
        let intent = r.varint()?;
        let next = if intent == handshake::STATUS {
            State::Status
        } else {
            State::Login
        };
        self.set_writing(next);
        self.set_reading(next);
        let mut w = Writer::new();
        w.varint(ids::handshake::serverbound::INTENTION);
        w.varint(self.version.protocol());
        w.string(&host);
        w.u16(port);
        w.varint(intent);
        Ok(w.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(id: i32, payload: &[u8]) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(id);
        w.bytes(payload);
        w.data
    }

    #[test]
    fn the_handshake_states_the_older_protocol_and_moves_both_directions() {
        let s = Session::new(ServerVersion::V26_2);
        let mut payload = Writer::new();
        payload.varint(crate::PROTOCOL_VERSION);
        payload.string("example.org");
        payload.u16(25565);
        payload.varint(handshake::LOGIN);
        let sent = s.serverbound(&body(0, &payload.data)).unwrap().unwrap();
        let mut r = Reader::new(&sent);
        assert_eq!(r.varint().unwrap(), 0);
        assert_eq!(r.varint().unwrap(), 776);
        assert_eq!(r.string(255).unwrap(), "example.org");
        assert_eq!((s.reading(), s.writing()), (State::Login, State::Login));
    }

    #[test]
    fn each_direction_follows_its_own_switches() {
        let s = Session::new(ServerVersion::V26_2);
        s.set_reading(State::Login);
        s.set_writing(State::Login);
        // login_finished arrives: the server already speaks configuration, Horizon still login.
        s.clientbound(ids::login::clientbound::LOGIN_FINISHED, Vec::new())
            .unwrap();
        assert_eq!(
            (s.reading(), s.writing()),
            (State::Configuration, State::Login)
        );
        s.serverbound(&body(ids::login::serverbound::LOGIN_ACKNOWLEDGED, &[]))
            .unwrap();
        assert_eq!(s.writing(), State::Configuration);
        // finish_configuration: 26.2 numbers it 3 as 26.3 does.
        s.clientbound(3, Vec::new()).unwrap();
        assert_eq!(s.reading(), State::Play);
        s.serverbound(&body(
            ids::configuration::serverbound::FINISH_CONFIGURATION,
            &[],
        ))
        .unwrap();
        assert_eq!(s.writing(), State::Play);
    }

    #[test]
    fn packets_are_renumbered_and_unknown_ones_dropped() {
        let s = Session::new(ServerVersion::V26_2);
        s.set_reading(State::Play);
        s.set_writing(State::Play);
        // 26.2's keep_alive is 44, 26.3's 45.
        let (id, payload) = s.clientbound(44, vec![0; 8]).unwrap().unwrap();
        assert_eq!((id, payload.len()), (ids::play::clientbound::KEEP_ALIVE, 8));
        assert_eq!(s.clientbound(141, Vec::new()).unwrap(), None);
        // resource_pack: Horizon's 50 is 26.2's 49.
        let sent = s
            .serverbound(&body(ids::play::serverbound::RESOURCE_PACK, &[7]))
            .unwrap()
            .unwrap();
        assert_eq!(sent, vec![49, 7]);
    }
}
