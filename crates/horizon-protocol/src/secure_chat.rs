//! Secure chat as vanilla 26.3's server keeps it: a player's profile key,
//! validated against the services' keys (ProfilePublicKey,
//! RemoteChatSession), the message chain each player signs
//! (SignedMessageChain), what a signature covers (PlayerChatMessage,
//! SignedMessageLink, SignedMessageBody), the window of messages a client
//! acknowledges (LastSeenMessagesValidator) and the cache a connection packs
//! signatures against (MessageSignatureCache).
use crate::{
    packets::play::LastSeenUpdate,
    rsa::{RsaPrivateKey, RsaPublicKey, SignatureHash},
};
use std::collections::{HashSet, VecDeque};

/// MessageSignature.BYTES.
pub const SIGNATURE_BYTES: usize = 256;
/// A message signature (MessageSignature).
pub type Signature = [u8; SIGNATURE_BYTES];
/// LastSeenMessages.LAST_SEEN_MESSAGES_MAX_LENGTH: the window a client acknowledges.
pub const LAST_SEEN: usize = 20;
/// PlayerChatMessage.MESSAGE_EXPIRES_AFTER_SERVER, in milliseconds.
pub const EXPIRES_AFTER_SERVER: i64 = 5 * 60 * 1000;

/// A signature from its packet bytes (always 256 on the wire).
pub fn signature(bytes: &[u8]) -> Option<Signature> {
    bytes.try_into().ok()
}

/// ProfilePublicKey.Data: the player's chat key (X.509 DER), when it
/// expires (epoch milliseconds), and the services' signature over both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileKey {
    pub expires_at: i64,
    pub key: Vec<u8>,
    pub signature: Vec<u8>,
}

impl ProfileKey {
    /// Data.signedPayload: the profile id, the expiry and the key.
    fn signed_payload(&self, profile: u128) -> Vec<u8> {
        let mut out = Vec::with_capacity(24 + self.key.len());
        out.extend_from_slice(&profile.to_be_bytes());
        out.extend_from_slice(&self.expires_at.to_be_bytes());
        out.extend_from_slice(&self.key);
        out
    }
    /// Data.validateSignature with the services' profile-key keys
    /// (SignatureValidator.from(keySet, PROFILE_KEY): any of them, SHA1withRSA).
    pub fn signed_by(&self, services: &[RsaPublicKey], profile: u128) -> bool {
        let payload = self.signed_payload(profile);
        services
            .iter()
            .any(|k| k.verify(SignatureHash::Sha1, &payload, &self.signature))
    }
    /// Data.hasExpired.
    pub fn has_expired(&self, now: i64) -> bool {
        self.expires_at < now
    }
    /// ProfilePublicKey.Data.hasExpired(Duration): the expiry plus a grace period, in milliseconds.
    pub fn has_expired_with(&self, now: i64, grace: i64) -> bool {
        self.expires_at.saturating_add(grace) < now
    }
}

/// ProfilePublicKey.EXPIRY_GRACE_PERIOD (8 hours), in milliseconds: how long past its expiry a
/// player's key still validates the chat a client reads.
pub const EXPIRY_GRACE_PERIOD: i64 = 8 * 60 * 60 * 1000;

/// A validated RemoteChatSession: its id and the player's profile key.
#[derive(Clone, Debug)]
pub struct ChatSession {
    pub id: u128,
    pub profile_key: ProfileKey,
    key: RsaPublicKey,
}

impl ChatSession {
    /// RemoteChatSession.Data.validate (ProfilePublicKey.createValidated);
    /// the error is the disconnect message's translation key.
    pub fn validate(
        id: u128,
        profile_key: ProfileKey,
        services: &[RsaPublicKey],
        profile: u128,
    ) -> Result<Self, &'static str> {
        const INVALID: &str = "multiplayer.disconnect.invalid_public_key_signature";
        let key = RsaPublicKey::from_der(&profile_key.key).map_err(|_| INVALID)?;
        if !profile_key.signed_by(services, profile) {
            return Err(INVALID);
        }
        Ok(Self {
            id,
            profile_key,
            key,
        })
    }
    /// RemoteChatSession.Data, as ClientboundPlayerInfoUpdatePacket's
    /// INITIALIZE_CHAT carries it.
    pub fn data(&self) -> (u128, i64, Vec<u8>, Vec<u8>) {
        let k = &self.profile_key;
        (self.id, k.expires_at, k.key.clone(), k.signature.clone())
    }
}

/// SignedMessageBody: what a player signed besides the chain link.
#[derive(Clone, Copy, Debug)]
pub struct Body<'a> {
    pub content: &'a str,
    /// Epoch milliseconds.
    pub timestamp: i64,
    pub salt: i64,
    pub last_seen: &'a [Signature],
}

/// PlayerChatMessage.updateSignature: the version, the link (sender,
/// session, index) and the body (salt, time in seconds, content, last seen).
pub fn signed_payload(sender: u128, session: u128, index: i32, body: &Body) -> Vec<u8> {
    let content = body.content.as_bytes();
    let mut out = Vec::with_capacity(64 + content.len() + body.last_seen.len() * SIGNATURE_BYTES);
    out.extend_from_slice(&1i32.to_be_bytes());
    out.extend_from_slice(&sender.to_be_bytes());
    out.extend_from_slice(&session.to_be_bytes());
    out.extend_from_slice(&index.to_be_bytes());
    out.extend_from_slice(&body.salt.to_be_bytes());
    out.extend_from_slice(&body.timestamp.div_euclid(1000).to_be_bytes());
    out.extend_from_slice(&(content.len() as i32).to_be_bytes());
    out.extend_from_slice(content);
    out.extend_from_slice(&(body.last_seen.len() as i32).to_be_bytes());
    for s in body.last_seen {
        out.extend_from_slice(s);
    }
    out
}

/// SignedMessageChain.DecodeException's messages, and the signed command
/// mismatch (ServerGamePacketListenerImpl.INVALID_COMMAND_SIGNATURE).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    MissingProfileKey,
    ChainBroken,
    ExpiredProfileKey,
    InvalidSignature,
    OutOfOrderChat,
    InvalidCommandSignature,
}

impl DecodeError {
    pub fn key(self) -> &'static str {
        match self {
            Self::MissingProfileKey => "chat.disabled.missingProfileKey",
            Self::ChainBroken => "chat.disabled.chain_broken",
            Self::ExpiredProfileKey => "chat.disabled.expiredProfileKey",
            Self::InvalidSignature => "chat.disabled.invalid_signature",
            Self::OutOfOrderChat => "chat.disabled.out_of_order_chat",
            Self::InvalidCommandSignature => "chat.disabled.invalid_command_signature",
        }
    }
}

/// A message the chain let through: unsigned (PlayerChatMessage.unsigned)
/// or signed at a link of the sender's chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unpacked {
    Unsigned,
    Signed { index: i32, signature: Signature },
}

/// A player's SignedMessageChain.Decoder: unsigned until the player sends a
/// chat session, then keyed to it.
#[derive(Clone, Debug, Default)]
pub struct Chain {
    session: Option<ChatSession>,
    /// The next link's index; None once the chain is broken.
    next: Option<i32>,
    /// The last message's time (lastTimeStamp, from Instant.EPOCH).
    last_timestamp: i64,
}

impl Chain {
    /// RemoteChatSession.createMessageDecoder: a new chain from link 0.
    pub fn keyed(session: ChatSession) -> Self {
        Self {
            session: Some(session),
            next: Some(0),
            last_timestamp: 0,
        }
    }
    pub fn session(&self) -> Option<&ChatSession> {
        self.session.as_ref()
    }
    /// Decoder.unpack for `sender`'s message. Without a session the message
    /// is unsigned unless the server enforces secure chat; with one it must
    /// be the next link, in order, signed by the player's key.
    pub fn unpack(
        &mut self,
        sender: u128,
        signature: Option<&Signature>,
        body: &Body,
        enforce: bool,
        now: i64,
    ) -> Result<Unpacked, DecodeError> {
        let Some(session) = &self.session else {
            return if enforce {
                Err(DecodeError::MissingProfileKey)
            } else {
                Ok(Unpacked::Unsigned)
            };
        };
        let signature = signature.ok_or(DecodeError::MissingProfileKey)?;
        if session.profile_key.has_expired(now) {
            return Err(DecodeError::ExpiredProfileKey);
        }
        let index = self.next.ok_or(DecodeError::ChainBroken)?;
        if body.timestamp < self.last_timestamp {
            self.next = None;
            return Err(DecodeError::OutOfOrderChat);
        }
        self.last_timestamp = body.timestamp;
        let payload = signed_payload(sender, session.id, index, body);
        if !session
            .key
            .verify(SignatureHash::Sha256, &payload, signature)
        {
            self.next = None;
            return Err(DecodeError::InvalidSignature);
        }
        self.next = index.checked_add(1);
        Ok(Unpacked::Signed {
            index,
            signature: *signature,
        })
    }
    /// Decoder.setChainBroken (nothing for the unsigned decoder).
    pub fn set_broken(&mut self) {
        if self.session.is_some() {
            self.next = None;
        }
    }
}

/// A chat packet's message as `PlayerChatMessage` carries it for a validator: its link (the
/// sender's index in their session) and signature, and what the signature covers.
#[derive(Clone, Copy, Debug)]
pub struct Incoming<'a> {
    pub sender: u128,
    pub index: i32,
    pub signature: Option<&'a Signature>,
    pub body: Body<'a>,
    /// The server's replacement for the content, and the filter mask: part of the record
    /// `PlayerChatMessage.equals` compares.
    pub unsigned_content: Option<&'a horizon_world::nbt::Tag>,
    pub filter: &'a (i32, Vec<i64>),
}

/// The record `PlayerChatMessage.equals` compares, kept for the last accepted message.
#[derive(Clone, Debug, PartialEq)]
pub struct Accepted {
    sender: u128,
    session: u128,
    index: i32,
    signature: Signature,
    content: String,
    timestamp: i64,
    salt: i64,
    last_seen: Vec<Signature>,
    unsigned_content: Option<horizon_world::nbt::Tag>,
    filter: (i32, Vec<i64>),
}

/// What a validator made of a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Validated {
    /// `PlayerChatMessage.removeSignature`: shown as an unsigned message.
    Unsigned,
    /// Signed by the sender's key, in order.
    Signed,
}

/// `PlayerInfo.messageValidator`: how a client checks one player's chat
/// (`SignedMessageValidator`).
#[derive(Clone, Debug)]
pub enum MessageValidator {
    /// `ACCEPT_UNSIGNED`: any message, its signature dropped.
    AcceptUnsigned,
    /// `REJECT_ALL`: a player with no chat session on a server that enforces secure chat.
    RejectAll,
    /// `KeyBased`: the player's session key, and the last message of the chain.
    KeyBased {
        session: Box<ChatSession>,
        /// The last accepted message.
        last: Option<Accepted>,
        chain_valid: bool,
    },
}

impl MessageValidator {
    /// `PlayerInfo.fallbackMessageValidator`.
    pub fn fallback(enforces_secure_chat: bool) -> Self {
        if enforces_secure_chat {
            Self::RejectAll
        } else {
            Self::AcceptUnsigned
        }
    }
    /// `RemoteChatSession.createMessageValidator`.
    pub fn keyed(session: ChatSession) -> Self {
        Self::KeyBased {
            session: Box::new(session),
            last: None,
            chain_valid: true,
        }
    }
    /// `SignedMessageValidator.updateAndValidate` at `now` (epoch milliseconds): `None` is a
    /// message that fails validation (`ChatListener.handleChatMessageError`), after which a
    /// keyed chain accepts nothing more.
    pub fn update_and_validate(&mut self, message: &Incoming, now: i64) -> Option<Validated> {
        match self {
            Self::AcceptUnsigned => Some(Validated::Unsigned),
            Self::RejectAll => None,
            Self::KeyBased {
                session,
                last,
                chain_valid,
            } => {
                *chain_valid = *chain_valid && Self::validate(session, last, message, now);
                if !*chain_valid {
                    return None;
                }
                *last = message.signature.map(|s| Accepted {
                    sender: message.sender,
                    session: session.id,
                    index: message.index,
                    signature: *s,
                    content: message.body.content.to_owned(),
                    timestamp: message.body.timestamp,
                    salt: message.body.salt,
                    last_seen: message.body.last_seen.to_vec(),
                    unsigned_content: message.unsigned_content.cloned(),
                    filter: message.filter.clone(),
                });
                Some(Validated::Signed)
            }
        }
    }
    /// `KeyBased.validate`: the key not expired (with the grace period), the signature over the
    /// message, and the chain.
    fn validate(
        session: &ChatSession,
        last: &Option<Accepted>,
        message: &Incoming,
        now: i64,
    ) -> bool {
        if session
            .profile_key
            .has_expired_with(now, EXPIRY_GRACE_PERIOD)
        {
            return false;
        }
        // PlayerChatMessage.verify: unsigned has no signature to verify.
        let Some(signature) = message.signature else {
            return false;
        };
        let payload = signed_payload(message.sender, session.id, message.index, &message.body);
        if !session
            .key
            .verify(SignatureHash::Sha256, &payload, signature)
        {
            return false;
        }
        Self::validate_chain(session, last, message, signature)
    }
    /// `KeyBased.validateChain`: the same message again (`message.equals(lastMessage)`), or a
    /// descendant of the last one (`SignedMessageLink.isDescendantOf`: a greater index in the
    /// same session, from the same sender).
    fn validate_chain(
        session: &ChatSession,
        last: &Option<Accepted>,
        message: &Incoming,
        signature: &Signature,
    ) -> bool {
        let Some(last) = last else {
            return true;
        };
        let same = last.sender == message.sender
            && last.session == session.id
            && last.index == message.index
            && &last.signature == signature
            && last.content == message.body.content
            && last.timestamp == message.body.timestamp
            && last.salt == message.body.salt
            && last.last_seen == message.body.last_seen
            && last.unsigned_content.as_ref() == message.unsigned_content
            && &last.filter == message.filter;
        same || (message.index > last.index
            && message.sender == last.sender
            && session.id == last.session)
    }
}

/// Arrays.hashCode(byte[]).
fn java_hash(bytes: &[u8]) -> i32 {
    bytes.iter().fold(1i32, |h, &b| {
        h.wrapping_mul(31).wrapping_add(i32::from(b as i8))
    })
}

/// LastSeenMessages.computeChecksum: never 0, which means "not checked".
pub fn checksum(entries: &[Signature]) -> i8 {
    let c = entries
        .iter()
        .fold(1i32, |c, e| c.wrapping_mul(31).wrapping_add(java_hash(e)));
    match c as i8 {
        0 => 1,
        b => b,
    }
}

/// LastSeenMessagesValidator: the messages sent to a player that its next
/// acknowledgement may name, each pending until it does.
#[derive(Clone, Debug)]
pub struct LastSeenValidator {
    tracked: VecDeque<Option<(Signature, bool)>>,
    last_pending: Option<Signature>,
}

impl Default for LastSeenValidator {
    fn default() -> Self {
        Self {
            tracked: std::iter::repeat_n(None, LAST_SEEN).collect(),
            last_pending: None,
        }
    }
}

impl LastSeenValidator {
    pub fn add_pending(&mut self, signature: &Signature) {
        if self.last_pending.as_ref() != Some(signature) {
            self.tracked.push_back(Some((*signature, true)));
            self.last_pending = Some(*signature);
        }
    }
    pub fn tracked_count(&self) -> usize {
        self.tracked.len()
    }
    /// applyOffset: drop the `offset` oldest messages.
    pub fn apply_offset(&mut self, offset: i32) -> Result<(), String> {
        let max = self.tracked.len() as i64 - LAST_SEEN as i64;
        if offset >= 0 && i64::from(offset) <= max {
            self.tracked.drain(..offset as usize);
            Ok(())
        } else {
            Err(format!(
                "Advanced last seen window by {offset} messages, but expected at most {max}"
            ))
        }
    }
    /// applyUpdate: the offset, then the acknowledged messages of the
    /// window (a 20-bit set, little-endian bytes), checked against the
    /// client's checksum unless it sent 0.
    pub fn apply_update(
        &mut self,
        offset: i32,
        acknowledged: [u8; 3],
        checksum: i8,
    ) -> Result<Vec<Signature>, String> {
        self.apply_offset(offset)?;
        let bit = |i: usize| acknowledged[i / 8] >> (i % 8) & 1 != 0;
        let length = (0..24).rev().find(|&i| bit(i)).map_or(0, |i| i + 1);
        if length > LAST_SEEN {
            return Err(format!(
                "Last seen update contained {length} messages, but maximum window size is {LAST_SEEN}"
            ));
        }
        let mut seen = Vec::with_capacity(LAST_SEEN);
        for i in 0..LAST_SEEN {
            let entry = self.tracked[i];
            if bit(i) {
                let Some((s, _)) = entry else {
                    return Err(format!(
                        "Last seen update acknowledged unknown or previously ignored message at index {i}"
                    ));
                };
                self.tracked[i] = Some((s, false));
                seen.push(s);
            } else {
                if let Some((_, false)) = entry {
                    return Err(format!(
                        "Last seen update ignored previously acknowledged message at index {i}"
                    ));
                }
                self.tracked[i] = None;
            }
        }
        if checksum != 0 && checksum != self::checksum(&seen) {
            return Err(
                "Checksum mismatch on last seen update: the client and server must have desynced"
                    .to_owned(),
            );
        }
        Ok(seen)
    }
}

/// MessageSignatureCache: the 128 signatures a connection's client also
/// holds, so a packed signature can name one by index.
#[derive(Clone, Debug)]
pub struct SignatureCache {
    entries: Vec<Option<Signature>>,
}

impl Default for SignatureCache {
    fn default() -> Self {
        Self {
            entries: vec![None; 128],
        }
    }
}

impl SignatureCache {
    /// MessageSignature.pack: the cache index, or the full signature.
    pub fn pack(&self, signature: &Signature) -> (i32, Option<Vec<u8>>) {
        match self
            .entries
            .iter()
            .position(|e| e.as_ref() == Some(signature))
        {
            Some(i) => (i as i32, None),
            None => (-1, Some(signature.to_vec())),
        }
    }
    /// `MessageSignature.Packed.unpack`: a full signature as it is, an
    /// index as the cache holds it; `None` for an empty or unknown slot.
    pub fn unpack(&self, packed: &(i32, Option<Vec<u8>>)) -> Option<Signature> {
        match packed {
            (_, Some(full)) => signature(full),
            (id, None) => usize::try_from(*id)
                .ok()
                .and_then(|i| self.entries.get(i).copied().flatten()),
        }
    }
    /// push(body, signature): the message's last seen and its signature go
    /// to the front, most recent first; what they displace moves back.
    pub fn push(&mut self, last_seen: &[Signature], signature: Option<&Signature>) {
        let mut queue: VecDeque<Signature> = last_seen.iter().chain(signature).copied().collect();
        let new: HashSet<Signature> = queue.iter().copied().collect();
        for slot in &mut self.entries {
            let Some(next) = queue.pop_back() else { break };
            if let Some(old) = slot.replace(next)
                && !new.contains(&old)
            {
                queue.push_front(old);
            }
        }
    }
}

/// The local player's SignedMessageChain.Encoder (LocalChatSession.
/// createMessageEncoder): each message signed with the chat key at the next
/// link of the chain, from link 0 of a new session.
#[derive(Clone, Debug)]
pub struct Encoder {
    sender: u128,
    session: u128,
    /// The next link's index; None once the chain has run out.
    next: Option<i32>,
}

impl Encoder {
    pub fn new(sender: u128, session: u128) -> Self {
        Self {
            sender,
            session,
            next: Some(0),
        }
    }
    pub fn session(&self) -> u128 {
        self.session
    }
    /// Encoder.pack: SHA256withRSA over PlayerChatMessage.updateSignature
    /// at the next link, which then advances. None (the message goes
    /// unsigned) once the chain has run out or the key can't sign a
    /// MessageSignature-sized signature.
    pub fn sign(&mut self, key: &RsaPrivateKey, body: &Body) -> Option<Signature> {
        let index = self.next?;
        let payload = signed_payload(self.sender, self.session, index, body);
        let signature = signature(&key.sign(SignatureHash::Sha256, &payload).ok()?)?;
        self.next = index.checked_add(1);
        Some(signature)
    }
}

/// LastSeenMessagesTracker.Update: the signatures a message signs as seen,
/// and the acknowledgement its packet carries.
#[derive(Clone, Debug, PartialEq)]
pub struct SeenUpdate {
    pub last_seen: Vec<Signature>,
    pub update: LastSeenUpdate,
}

/// LastSeenMessagesTracker: the client's ring of the last 20 signed
/// messages it processed (None for one it hid), and how many arrived since
/// the last acknowledgement.
#[derive(Clone, Debug)]
pub struct LastSeenTracker {
    /// LastSeenTrackedEntry: the signature and whether it is still pending.
    tracked: Vec<Option<(Signature, bool)>>,
    tail: usize,
    offset: i32,
    last_tracked: Option<Signature>,
}

impl Default for LastSeenTracker {
    fn default() -> Self {
        Self {
            tracked: vec![None; LAST_SEEN],
            tail: 0,
            offset: 0,
            last_tracked: None,
        }
    }
}

impl LastSeenTracker {
    /// addPending: false for a repeat of the last message.
    pub fn add_pending(&mut self, signature: &Signature, shown: bool) -> bool {
        if self.last_tracked.as_ref() == Some(signature) {
            return false;
        }
        self.last_tracked = Some(*signature);
        self.tracked[self.tail] = shown.then_some((*signature, true));
        self.tail = (self.tail + 1) % LAST_SEEN;
        self.offset += 1;
        true
    }
    /// ignorePending: a deleted message that is still pending is dropped.
    pub fn ignore_pending(&mut self, signature: &Signature) {
        if let Some(entry) = self
            .tracked
            .iter_mut()
            .find(|e| matches!(e, Some((s, true)) if s == signature))
        {
            *entry = None;
        }
    }
    /// Messages added since the last acknowledgement.
    pub fn offset(&self) -> i32 {
        self.offset
    }
    /// getAndClearOffset.
    pub fn take_offset(&mut self) -> i32 {
        std::mem::take(&mut self.offset)
    }
    /// generateAndApplyUpdate: the window from oldest to newest, each entry
    /// acknowledged, with the offset since the last update and the checksum.
    pub fn update(&mut self) -> SeenUpdate {
        let offset = self.take_offset();
        let mut acknowledged = [0u8; 3];
        let mut last_seen = Vec::with_capacity(LAST_SEEN);
        for i in 0..LAST_SEEN {
            let index = (self.tail + i) % LAST_SEEN;
            if let Some((signature, pending)) = &mut self.tracked[index] {
                acknowledged[i / 8] |= 1 << (i % 8);
                last_seen.push(*signature);
                *pending = false;
            }
        }
        let checksum = checksum(&last_seen);
        SeenUpdate {
            last_seen,
            update: LastSeenUpdate {
                offset,
                acknowledged,
                checksum,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(b: u8) -> Signature {
        [b; SIGNATURE_BYTES]
    }

    #[test]
    fn payload_layout_matches_update_signature() {
        let body = Body {
            content: "hé",
            timestamp: 1_700_000_123_456,
            salt: -2,
            last_seen: &[sig(7)],
        };
        let p = signed_payload(1, 2, 3, &body);
        assert_eq!(&p[..4], &[0, 0, 0, 1]);
        assert_eq!(&p[4..20], &1u128.to_be_bytes());
        assert_eq!(&p[20..36], &2u128.to_be_bytes());
        assert_eq!(&p[36..40], &3i32.to_be_bytes());
        assert_eq!(&p[40..48], &(-2i64).to_be_bytes());
        assert_eq!(&p[48..56], &1_700_000_123i64.to_be_bytes());
        assert_eq!(&p[56..60], &3i32.to_be_bytes());
        assert_eq!(&p[60..63], "hé".as_bytes());
        assert_eq!(&p[63..67], &1i32.to_be_bytes());
        assert_eq!(p.len(), 67 + 256);
        // Instant.getEpochSecond floors.
        let before = Body {
            timestamp: -1,
            ..body
        };
        assert_eq!(
            &signed_payload(1, 2, 3, &before)[48..56],
            &(-1i64).to_be_bytes()
        );
    }

    #[test]
    fn unsigned_chain_follows_enforcement() {
        let mut chain = Chain::default();
        let body = Body {
            content: "hi",
            timestamp: 5,
            salt: 0,
            last_seen: &[],
        };
        assert_eq!(
            chain.unpack(1, None, &body, false, 0),
            Ok(Unpacked::Unsigned)
        );
        assert_eq!(
            chain.unpack(1, Some(&sig(1)), &body, false, 0),
            Ok(Unpacked::Unsigned)
        );
        assert_eq!(
            chain.unpack(1, None, &body, true, 0),
            Err(DecodeError::MissingProfileKey)
        );
    }

    #[test]
    fn checksums_follow_java_hash_codes() {
        // Arrays.hashCode(new byte[]{-1}) = 31 + (-1) = 30.
        assert_eq!(java_hash(&[0xff]), 30);
        assert_eq!(checksum(&[]), 1);
        let one = [sig(0)];
        let expected = 31i32.wrapping_add(java_hash(&sig(0))) as i8;
        assert_eq!(checksum(&one), if expected == 0 { 1 } else { expected });
    }

    #[test]
    fn last_seen_window() {
        let mut v = LastSeenValidator::default();
        assert_eq!(v.tracked_count(), 20);
        v.add_pending(&sig(1));
        v.add_pending(&sig(1));
        v.add_pending(&sig(2));
        assert_eq!(v.tracked_count(), 22);
        assert!(v.apply_offset(3).is_err());
        // Slide by two: the two messages are the window's last entries.
        let mut ack = [0u8; 3];
        ack[18 / 8] |= 1 << (18 % 8);
        ack[19 / 8] |= 1 << (19 % 8);
        let seen = v.apply_update(2, ack, checksum(&[sig(1), sig(2)])).unwrap();
        assert_eq!(seen, vec![sig(1), sig(2)]);
        // Once acknowledged, a message can't be ignored.
        assert!(v.apply_update(0, [0; 3], 0).is_err());
        let mut v = LastSeenValidator::default();
        v.add_pending(&sig(1));
        assert!(v.apply_update(1, [0, 0, 0x10], 0).is_err(), "bit 20");
        let mut v = LastSeenValidator::default();
        assert!(v.apply_update(0, [1, 0, 0], 0).is_err(), "unknown");
        let mut v = LastSeenValidator::default();
        v.add_pending(&sig(1));
        let mut ack = [0u8; 3];
        ack[2] |= 1 << 3;
        assert!(v.apply_update(1, ack, 99).is_err(), "checksum");
    }

    #[test]
    fn packed_signatures_resolve_against_the_cache() {
        let mut c = SignatureCache::default();
        assert_eq!(c.unpack(&(-1, Some(sig(4).to_vec()))), Some(sig(4)));
        assert_eq!(c.unpack(&(0, None)), None, "an empty slot");
        c.push(&[sig(1)], Some(&sig(2)));
        assert_eq!(c.unpack(&(0, None)), Some(sig(2)));
        assert_eq!(c.unpack(&(1, None)), Some(sig(1)));
        assert_eq!(c.unpack(&(2, None)), None);
        assert_eq!(c.unpack(&(128, None)), None, "past the cache");
        // What the server packs, the client unpacks to the same signature.
        assert_eq!(c.unpack(&c.pack(&sig(1))), Some(sig(1)));
    }

    #[test]
    fn cache_moves_recent_signatures_first() {
        let mut c = SignatureCache::default();
        assert_eq!(c.pack(&sig(1)), (-1, Some(sig(1).to_vec())));
        c.push(&[], Some(&sig(1)));
        assert_eq!(c.pack(&sig(1)), (0, None));
        c.push(&[sig(1)], Some(&sig(2)));
        assert_eq!(c.pack(&sig(2)), (0, None));
        assert_eq!(c.pack(&sig(1)), (1, None));
        c.push(&[], Some(&sig(3)));
        assert_eq!(
            [c.pack(&sig(3)).0, c.pack(&sig(2)).0, c.pack(&sig(1)).0],
            [0, 1, 2]
        );
    }

    use crate::rsa::tests::{CHAT_KEY_PKCS8, TEST_D, TEST_SPKI};

    /// The services' key (the 1024-bit test key) signing a player's chat
    /// key as the certificates endpoint's publicKeySignatureV2 does.
    fn services() -> RsaPrivateKey {
        let n = RsaPublicKey::from_der(&crate::rsa::tests::hex(TEST_SPKI))
            .unwrap()
            .modulus;
        let body = [
            crate::rsa::der_integer(&[0]),
            crate::rsa::der_integer(&n),
            crate::rsa::der_integer(&[1, 0, 1]),
            crate::rsa::der_integer(&crate::rsa::tests::hex(TEST_D)),
        ]
        .concat();
        RsaPrivateKey::from_der(&crate::rsa::der_element(0x30, &body)).unwrap()
    }

    fn session(player: &RsaPrivateKey, profile: u128, id: u128) -> ChatSession {
        let services = services();
        let mut key = ProfileKey {
            expires_at: 2_000_000_000_000,
            key: player.public_der(),
            signature: Vec::new(),
        };
        key.signature = services
            .sign(SignatureHash::Sha1, &key.signed_payload(profile))
            .unwrap();
        ChatSession::validate(id, key, std::slice::from_ref(&services.public), profile).unwrap()
    }

    /// What the encoder signs, the server's decoder (Chain) accepts, link by
    /// link, and refuses once a message is tampered with.
    #[test]
    fn encoder_signs_what_the_chain_accepts() {
        let player = RsaPrivateKey::from_der(&crate::rsa::tests::hex(CHAT_KEY_PKCS8)).unwrap();
        let (profile, id) = (0x1234u128, 0x9876u128);
        let mut chain = Chain::keyed(session(&player, profile, id));
        let mut encoder = Encoder::new(profile, id);
        assert_eq!(encoder.session(), id);
        let seen = [sig(4)];
        for (i, text) in ["hello", "/msg-like text", "é"].iter().enumerate() {
            let body = Body {
                content: text,
                timestamp: 1_700_000_000_000 + i as i64,
                salt: 7 * i as i64,
                last_seen: &seen,
            };
            let signature = encoder.sign(&player, &body).unwrap();
            assert_eq!(
                chain.unpack(profile, Some(&signature), &body, true, 0),
                Ok(Unpacked::Signed {
                    index: i as i32,
                    signature
                })
            );
        }
        let body = Body {
            content: "original",
            timestamp: 1_700_000_001_000,
            salt: 1,
            last_seen: &[],
        };
        let signature = encoder.sign(&player, &body).unwrap();
        let forged = Body {
            content: "forged",
            ..body
        };
        assert_eq!(
            chain.unpack(profile, Some(&signature), &forged, true, 0),
            Err(DecodeError::InvalidSignature)
        );
        // Another sender's chain doesn't take the player's signatures.
        let mut other = Chain::keyed(session(&player, profile, id));
        let mut encoder = Encoder::new(profile + 1, id);
        let signature = encoder.sign(&player, &body).unwrap();
        assert_eq!(
            other.unpack(profile, Some(&signature), &body, true, 0),
            Err(DecodeError::InvalidSignature)
        );
        // A 1024-bit key can't make a MessageSignature: unsigned, not wrong.
        let small = services();
        assert_eq!(Encoder::new(profile, id).sign(&small, &body), None);
    }

    /// The client's tracker and the server's validator agree on every
    /// update: the window, the offset and the checksum.
    #[test]
    fn tracker_updates_satisfy_the_validator() {
        let mut tracker = LastSeenTracker::default();
        let mut validator = LastSeenValidator::default();
        let first = tracker.update();
        assert_eq!(first.last_seen, Vec::<Signature>::new());
        assert_eq!(first.update.offset, 0);
        assert_eq!(first.update.checksum, checksum(&[]));
        for round in 0..4u8 {
            for n in 0..(round * 9 + 1) {
                let s = sig(round * 40 + n + 1);
                validator.add_pending(&s);
                assert!(tracker.add_pending(&s, true));
                assert!(!tracker.add_pending(&s, true), "a repeat");
            }
            let u = tracker.update();
            assert!(u.last_seen.len() <= LAST_SEEN);
            let seen = validator
                .apply_update(u.update.offset, u.update.acknowledged, u.update.checksum)
                .unwrap();
            assert_eq!(seen, u.last_seen);
            assert_eq!(tracker.offset(), 0);
        }
        // The last 20, oldest first.
        let u = tracker.update();
        assert_eq!(u.last_seen.len(), LAST_SEEN);
        assert_eq!(u.last_seen.last(), Some(&sig(3 * 40 + 28)));
        assert_eq!(u.update.acknowledged, [0xff, 0xff, 0x0f]);
    }

    /// A hidden message and a deleted pending one leave a hole the
    /// validator expects; a deleted message already acknowledged stays.
    #[test]
    fn tracker_holes() {
        let mut tracker = LastSeenTracker::default();
        let mut validator = LastSeenValidator::default();
        for s in [sig(1), sig(2), sig(3)] {
            validator.add_pending(&s);
        }
        tracker.add_pending(&sig(1), true);
        tracker.add_pending(&sig(2), false);
        tracker.add_pending(&sig(3), true);
        tracker.ignore_pending(&sig(3));
        assert_eq!(tracker.offset(), 3);
        let u = tracker.update();
        assert_eq!(u.last_seen, vec![sig(1)]);
        let seen = validator
            .apply_update(u.update.offset, u.update.acknowledged, u.update.checksum)
            .unwrap();
        assert_eq!(seen, vec![sig(1)]);
        tracker.ignore_pending(&sig(1));
        assert_eq!(tracker.update().last_seen, vec![sig(1)]);
        assert_eq!(tracker.take_offset(), 0);
    }

    /// A session with a 2048-bit test key, sender 1 and session 2; the signatures are
    /// SHA256withRSA over `signed_payload`, made outside Rust with the key's private half.
    fn test_session(expires_at: i64) -> ChatSession {
        let spki = "30820122300d06092a864886f70d01010105000382010f003082010a0282010100dbe0bf1391ce3498fd40343c9f6d569c76f39c0cd6fca817a5d8102aed03ed5dbf4daedc097e786a85d21fa7aea19c23266a1b560916dd027700eae45cf54f072db2f0869c19ae26aa09d5fdf96bed9d9044c4e50b4dad1ee97d552e6d4c8d599771245f9afc5b34a470705baac58a0ba2d889ad89bf7f3d9a57cfb75b3938e7388b574176bd8b43a72806df5aec4a5471acb48a9d31d34de7468f0cca70c7598734693541936f098fc04290576deaa00929e56958d5d21bdb8db4df561960ca7402501a26471f327026d078ea6b8f8cea96f0d077c175d1bf9da6fe89908551ceed1737d9bfe6c633842b4a666450edb7ab279f506695f8d87df6ee13024fab0203010001";
        let der = hex(spki);
        ChatSession {
            id: 2,
            key: RsaPublicKey::from_der(&der).unwrap(),
            profile_key: ProfileKey {
                expires_at,
                key: der,
                signature: Vec::new(),
            },
        }
    }
    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }
    fn signature_of(text: &str) -> Signature {
        hex(text).try_into().unwrap()
    }
    const NOW: i64 = 1_700_000_100_000;
    const SIG_0: &str = "620c7c55ef0184ebed11c2d0c6c8df7e6b75cd7a1f7f9e81b782cca33be8c23935a7a5bb2e01c924f205c1e0ab19357fc454c95b04b81bd636e588be227848ced911506554296e4a0219c7fa2d5852dd8b80d6aaa3a8b50d3ddb0afd14d5921148fdc0b866976ebeaeb97b075ba34882c3926cd50dabe4dba3c781ae0665044745db33df7228c37abab74049862235e021c227435de470ec1aa9023daddf5129afb7039ea482a97a0fdc8c817e9731143042e05da28be4a4362905c981c55256ceab07976a8f3590c1e7bd0bc17d2ce73a64cbe9be23ab3f9688eec4c8b90298dd087632d6c66e59f7acbda485dad2825a5f8e1f30d76ef9abc47af0a8d7b346";
    const SIG_1: &str = "9ca9c9f0ba97e0a5cc56a63025c8d5935bddde07fd065a567bdaa5157b9bc2b435128913284c80e3d22aaa4c757e0c6be026868dbe84e8a377cd188e95f826ced5fbb8cb433d29a764cfeaf8b45eb50ebaeb7ce846dd35aa3f02de974f250fc2a98519e3a8d213f1c4387e677cde131cc3b8434625b1f364fd9269a4eeb89cddb85191e159a5cea41b839298360366a715f77746503c4e1c6109cb23d152637110324f0094b9b533c90db15cc1908b8f4b6b86b94d3c76cd3e35e384c1583d22c05dda56b7c76a9b427fcdd5d1061a58494e480499562710ad2bc711c4f1f3c9321dd20ed291cbbe4f9a96f757b6ec65a544d0bbb21d9f16944539306e00b348";
    const SIG_OLD: &str = "7acb0ede0ef535ae69c0f52abb73a91e133905c4d590d68dbb392a1c2205611868a55626e574b63af318ada5d9ad5cd7102968bf8c6b0bea98a1dc78929463ece8c035f1e2ab271aeb156a9a1711d6214b04e15186b2a8dca8f2476a5a4199716d2186acff705f45939f605e224d8274cc0d5d8f7986bb15dd38e530b4abdd7582b7e062c8cfc31e6f7c87ac1a2cd6fe84ab9e9b7a73972586f8834043c25eec1a1c8865f47183f7aeb808989d62850df3cca5e0aba3285cdfd507eb55aebc716a05297e84a6d26d8c7844a7e27e1b5d5c34a5c79ccaf1f6a0fb04bc1ddd1411a93819d9b1cb869ea24968ef15c09dc7229dcd867cb1d187352583c1707f40ec";

    fn incoming<'a>(
        signature: Option<&'a Signature>,
        index: i32,
        salt: i64,
        timestamp: i64,
        content: &'a str,
        filter: &'a (i32, Vec<i64>),
    ) -> Incoming<'a> {
        Incoming {
            sender: 1,
            index,
            signature,
            body: Body {
                content,
                timestamp,
                salt,
                last_seen: &[],
            },
            unsigned_content: None,
            filter,
        }
    }

    /// `SignedMessageValidator.KeyBased`: a chain of signed messages in order passes, the same
    /// message again passes, and a message out of order, with a bad signature, unsigned or from
    /// an expired key ends the chain.
    #[test]
    fn a_keyed_validator_follows_the_chain() {
        let filter = (0, Vec::new());
        let (s0, s1, old) = (
            signature_of(SIG_0),
            signature_of(SIG_1),
            signature_of(SIG_OLD),
        );
        let m0 = incoming(Some(&s0), 0, 7, 1_700_000_000_000, "hello", &filter);
        let m1 = incoming(Some(&s1), 1, 8, 1_700_000_001_000, "world", &filter);
        let mut v = MessageValidator::keyed(test_session(NOW));
        assert_eq!(v.update_and_validate(&m0, NOW), Some(Validated::Signed));
        assert_eq!(v.update_and_validate(&m1, NOW), Some(Validated::Signed));
        // The same message again is the last one: accepted.
        assert_eq!(v.update_and_validate(&m1, NOW), Some(Validated::Signed));
        // A valid signature at index 0 after index 1: not a descendant, and the chain is over.
        let again = incoming(Some(&old), 0, 9, 1_700_000_002_000, "again", &filter);
        assert_eq!(v.update_and_validate(&again, NOW), None);
        let mut v2 = MessageValidator::keyed(test_session(NOW));
        assert_eq!(v2.update_and_validate(&m0, NOW), Some(Validated::Signed));
        assert_eq!(v2.update_and_validate(&m1, NOW), Some(Validated::Signed));
        assert_eq!(v2.update_and_validate(&again, NOW), None);
        assert_eq!(
            v2.update_and_validate(&m1, NOW),
            None,
            "a broken chain accepts nothing more"
        );
    }

    #[test]
    fn a_keyed_validator_rejects_a_bad_signature_a_missing_one_and_an_expired_key() {
        let filter = (0, Vec::new());
        let s0 = signature_of(SIG_0);
        // The signature of "hello" is not the one of "hellO".
        let tampered = incoming(Some(&s0), 0, 7, 1_700_000_000_000, "hellO", &filter);
        let mut v = MessageValidator::keyed(test_session(NOW));
        assert_eq!(v.update_and_validate(&tampered, NOW), None);
        let unsigned = incoming(None, 0, 7, 1_700_000_000_000, "hello", &filter);
        let mut v = MessageValidator::keyed(test_session(NOW));
        assert_eq!(v.update_and_validate(&unsigned, NOW), None);
        // Past the key's expiry and the eight hours of grace.
        let good = incoming(Some(&s0), 0, 7, 1_700_000_000_000, "hello", &filter);
        let mut v = MessageValidator::keyed(test_session(NOW));
        assert_eq!(
            v.update_and_validate(&good, NOW + EXPIRY_GRACE_PERIOD),
            Some(Validated::Signed),
            "the grace period still validates"
        );
        let mut v = MessageValidator::keyed(test_session(NOW));
        assert_eq!(
            v.update_and_validate(&good, NOW + EXPIRY_GRACE_PERIOD + 1),
            None
        );
    }

    /// `PlayerInfo.fallbackMessageValidator`: with no chat session a message is shown unsigned,
    /// or refused when the server enforces secure chat.
    #[test]
    fn the_fallback_validator_accepts_unsigned_or_rejects_all() {
        let filter = (0, Vec::new());
        let s0 = signature_of(SIG_0);
        let m0 = incoming(Some(&s0), 0, 7, 1_700_000_000_000, "hello", &filter);
        assert_eq!(
            MessageValidator::fallback(false).update_and_validate(&m0, NOW),
            Some(Validated::Unsigned)
        );
        assert_eq!(
            MessageValidator::fallback(true).update_and_validate(&m0, NOW),
            None
        );
    }
}
