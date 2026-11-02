//! Attachment sync: the values of a native mod's synced attachments, sent by a
//! Horizon server to the Horizon clients that see their holder
//! (docs/native-mods.md, "Server attachments").
//!
//! A payload is the holder (a varint kind and its fields), whether the
//! client's copy of the holder is replaced, then each change: the attachment's
//! namespace and name and its value as an NBT compound, or no tag for a
//! removal.
use horizon_world::{
    attachments::{AttachmentSync, Holder},
    nbt::{self, Tag},
};

use crate::{
    Error, Result,
    buf::{Reader, Writer},
};

/// The custom payload channel, clientbound only: a client sends nothing back.
pub const CHANNEL: &str = "horizon:attachment_sync";
/// Characters in a namespace or a name at most.
pub const MAX_NAME: usize = 256;
/// Bytes of a payload at most
/// (ClientboundCustomPayloadPacket.MAX_PAYLOAD_SIZE).
pub const MAX_PAYLOAD: usize = 1_048_576;

/// A holder's changes as they travel.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub holder: Holder,
    /// The changes are the holder's whole synced state: drop what the client
    /// holds first.
    pub replace: bool,
    pub changes: Vec<AttachmentSync>,
}

/// A message's payload; an error for a holder it cannot write, a name past
/// [`MAX_NAME`], a value that is not an encodable compound, or a payload past
/// [`MAX_PAYLOAD`].
pub fn encode(message: &Message) -> Result<Vec<u8>> {
    let mut w = Writer::new();
    match message.holder {
        Holder::Entity(id) => {
            w.varint(0);
            w.varint(id as i32);
        }
        Holder::Player(uuid) => {
            w.varint(1);
            w.uuid(uuid);
        }
        Holder::BlockEntity(pos) => {
            w.varint(2);
            pos.iter().for_each(|&axis| w.i32(axis));
        }
        Holder::Chunk(pos) => {
            w.varint(3);
            pos.iter().for_each(|&axis| w.i32(axis));
        }
        Holder::Level => w.varint(4),
        _ => return Err(Error::new("attachment sync: unknown holder")),
    }
    w.bool(message.replace);
    w.varint(message.changes.len() as i32);
    for change in &message.changes {
        for name in [&change.namespace, &change.name] {
            let name = name
                .to_string()
                .map_err(|_| Error::new("attachment sync: name is not UTF-16"))?;
            if name.encode_utf16().count() > MAX_NAME {
                return Err(Error::new(format!(
                    "attachment sync: name {name} is too long"
                )));
            }
            w.string(&name);
        }
        match &change.value {
            Some(value) => {
                let tag = Tag::Compound(value.clone());
                let bytes = nbt::write_network(&tag, nbt::Limits::default())
                    .map_err(|e| Error::new(format!("attachment sync: value: {e}")))?;
                w.bytes(&bytes);
            }
            None => w.bytes(&[0]),
        }
    }
    if w.data.len() > MAX_PAYLOAD {
        return Err(Error::new("attachment sync: payload too big"));
    }
    Ok(w.data)
}

/// A payload's message; an error for one that breaks a cap.
pub fn decode(data: &[u8]) -> Result<Message> {
    if data.len() > MAX_PAYLOAD {
        return Err(Error::new("attachment sync: payload too big"));
    }
    let mut r = Reader::new(data);
    let holder = match r.varint()? {
        0 => Holder::Entity(r.varint()? as u32),
        1 => Holder::Player(r.uuid()?),
        2 => Holder::BlockEntity([r.i32()?, r.i32()?, r.i32()?]),
        3 => Holder::Chunk([r.i32()?, r.i32()?]),
        4 => Holder::Level,
        kind => return Err(Error::new(format!("attachment sync: holder kind {kind}"))),
    };
    let replace = r.bool()?;
    let max = r.remaining();
    let changes = (0..r.count(max)?)
        .map(|_| {
            let namespace = r.string(MAX_NAME)?;
            let name = r.string(MAX_NAME)?;
            let value = match r.nbt()? {
                Tag::Compound(value) => Some(value),
                Tag::End => None,
                _ => return Err(Error::new("attachment sync: value is not a compound")),
            };
            Ok(AttachmentSync {
                namespace: namespace.as_str().into(),
                name: name.as_str().into(),
                value,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if r.remaining() > 0 {
        return Err(Error::new("attachment sync: trailing bytes"));
    }
    Ok(Message {
        holder,
        replace,
        changes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changes() -> Vec<AttachmentSync> {
        vec![
            AttachmentSync {
                namespace: "mana".into(),
                name: "pool".into(),
                value: Some([("value".into(), Tag::Int(12))].into()),
            },
            AttachmentSync {
                namespace: "mana".into(),
                name: "gone".into(),
                value: None,
            },
        ]
    }

    #[test]
    fn every_holder_round_trips_with_either_replace_and_a_removal() {
        let holders = [
            Holder::Entity(4_000_000_000),
            Holder::Player(0x0123_4567_89ab_cdef_0011_2233_4455_6677),
            Holder::BlockEntity([-30_000_000, -64, 29_999_999]),
            Holder::Chunk([-5, 7]),
            Holder::Level,
        ];
        for holder in holders {
            for replace in [true, false] {
                let message = Message {
                    holder,
                    replace,
                    changes: changes(),
                };
                assert_eq!(decode(&encode(&message).unwrap()).unwrap(), message);
            }
        }
    }

    #[test]
    fn decode_refuses_trailing_bytes_and_an_unknown_holder_kind() {
        let message = Message {
            holder: Holder::Level,
            replace: false,
            changes: changes(),
        };
        let mut payload = encode(&message).unwrap();
        payload.push(0);
        assert!(decode(&payload).is_err(), "trailing bytes");
        assert!(decode(&[5, 0, 0]).is_err(), "unknown holder kind");
    }

    #[test]
    fn encode_refuses_a_change_the_client_would_not_read_as_sent() {
        let mut message = Message {
            holder: Holder::Level,
            replace: false,
            changes: changes(),
        };
        message.changes[0].name = "n".repeat(MAX_NAME).as_str().into();
        assert!(decode(&encode(&message).unwrap()).is_ok());
        message.changes[0].name = "n".repeat(MAX_NAME + 1).as_str().into();
        assert!(encode(&message).is_err(), "name");
        message.changes[0].name = "pool".into();
        let mut deep = Tag::Int(0);
        for _ in 0..600 {
            deep = Tag::Compound([("v".into(), deep)].into());
        }
        let Tag::Compound(deep) = deep else {
            unreachable!()
        };
        message.changes[0].value = Some(deep);
        assert!(encode(&message).is_err(), "unwritable value");
        let big = Tag::ByteArray(vec![0; MAX_PAYLOAD]);
        message.changes[0].value = Some([("big".into(), big)].into());
        assert!(encode(&message).is_err(), "payload");
    }
}
