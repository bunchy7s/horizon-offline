//! Mod messages: bytes a native mod's server half and its client half send
//! each other, on one custom payload channel in the play state
//! (docs/native-mods.md, "Mod messages").
//!
//! The server announces it carries them with an empty [`CHANNEL`] payload in
//! the configuration phase (a vanilla client ignores it); a Horizon client
//! that hears it may send them, and the server sends only to clients that
//! identified as Horizon. A payload is the message's channel, then the bytes.
use crate::{
    Error, Result,
    buf::{Reader, Writer},
};

/// The custom payload channel, both ways.
pub const CHANNEL: &str = "horizon:mod_net";
/// Characters in each half of a message's channel at most.
pub const MAX_PART: usize = 64;
/// Bytes of a message's data at most: a serverbound payload may hold
/// `ServerboundCustomPayloadPacket.MAX_PAYLOAD_SIZE` (32767) in all.
pub const MAX_DATA: usize = 30_000;

/// A message as it travels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub channel: String,
    pub data: Vec<u8>,
}

/// A message's channel: `mod_id:name`, the mod's id `[a-z0-9_]{1,64}` and a
/// name of its own `[a-z0-9_./-]{1,64}`. `minecraft` and `horizon` are the
/// game's.
pub fn channel_ok(channel: &str) -> bool {
    let Some((id, name)) = channel.split_once(':') else {
        return false;
    };
    (1..=MAX_PART).contains(&id.len())
        && (1..=MAX_PART).contains(&name.len())
        && !matches!(id, "minecraft" | "horizon")
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && name.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.' | b'/' | b'-')
        })
}

/// A message's payload; an error for a channel or size outside the caps.
pub fn encode(message: &Message) -> Result<Vec<u8>> {
    if !channel_ok(&message.channel) {
        return Err(Error::new("mod net: bad channel"));
    }
    if message.data.len() > MAX_DATA {
        return Err(Error::new("mod net: data too big"));
    }
    let mut w = Writer::new();
    w.string(&message.channel);
    w.byte_array(&message.data);
    Ok(w.data)
}

/// A payload's message; an error for one that breaks a cap.
pub fn decode(data: &[u8]) -> Result<Message> {
    let mut r = Reader::new(data);
    let channel = r.string(MAX_PART * 2 + 1)?;
    let data = r.byte_array(MAX_DATA)?;
    if r.remaining() > 0 {
        return Err(Error::new("mod net: trailing bytes"));
    }
    if !channel_ok(&channel) {
        return Err(Error::new("mod net: bad channel"));
    }
    Ok(Message { channel, data })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(data: Vec<u8>) -> Message {
        Message {
            channel: "cells:cell/state".into(),
            data,
        }
    }

    #[test]
    fn a_message_round_trips() {
        for data in [vec![], vec![0, 255, 7], vec![9; MAX_DATA]] {
            let m = message(data);
            assert_eq!(decode(&encode(&m).unwrap()).unwrap(), m);
        }
    }

    #[test]
    fn channels_name_a_mod_and_leave_the_games_alone() {
        for good in ["cells:state", "my_mod:a/b.c-d", "m:n"] {
            assert!(channel_ok(good), "{good}");
        }
        for bad in [
            "",
            "state",
            ":state",
            "cells:",
            "Cells:state",
            "cells:Has Spaces",
            "cells:a:b",
            "minecraft:brand",
            "horizon:mod_net",
        ] {
            assert!(!channel_ok(bad), "{bad}");
        }
        assert!(channel_ok(&format!(
            "{}:{}",
            "x".repeat(MAX_PART),
            "n".repeat(MAX_PART)
        )));
        assert!(!channel_ok(&format!("{}:n", "x".repeat(MAX_PART + 1))));
        assert!(!channel_ok(&format!("x:{}", "n".repeat(MAX_PART + 1))));
    }

    #[test]
    fn caps_are_enforced_both_ways() {
        assert!(encode(&message(vec![0; MAX_DATA + 1])).is_err());
        let mut bad = message(vec![]);
        bad.channel = "no_name".into();
        assert!(encode(&bad).is_err());
        // A payload built by hand with a channel outside the rules.
        let mut w = Writer::new();
        w.string("minecraft:brand");
        w.byte_array(&[]);
        assert!(decode(&w.data).is_err());
        let mut ok = encode(&message(vec![1])).unwrap();
        ok.push(0);
        assert!(decode(&ok).is_err(), "trailing bytes");
        assert!(decode(&[]).is_err());
        assert!(decode(&[200, 200, 200]).is_err());
    }

    #[test]
    fn a_biggest_message_fits_a_serverbound_payload() {
        let m = Message {
            channel: format!("{}:{}", "x".repeat(MAX_PART), "n".repeat(MAX_PART)),
            data: vec![0; MAX_DATA],
        };
        assert!(encode(&m).unwrap().len() <= 32767);
    }
}
