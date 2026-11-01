//! Chat on the server side, as ServerGamePacketListenerImpl and PlayerList
//! handle it: chat sessions validated against the services' keys, signed
//! messages checked link by link, the acknowledged window, and messages
//! relayed with each connection's numbering and signature cache.
use super::{HostEvent, Hub};
use horizon_protocol::{
    packets::play::{self, Clientbound as C},
    secure_chat::{self, Body, Chain, ChatSession, DecodeError, ProfileKey, Signature, Unpacked},
    text::{literal, translatable},
};
use horizon_world::nbt::{JavaString, Tag};

/// Now, in epoch milliseconds (Instant.now()).
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// StringUtil.isAllowedChatCharacter.
fn allowed(c: char) -> bool {
    c != '\u{a7}' && c >= ' ' && c != '\u{7f}'
}

/// A translatable message in red.
fn red(key: &str) -> Tag {
    let mut t = translatable(key, &[]);
    if let Tag::Compound(map) = &mut t {
        map.insert(
            JavaString::from("color"),
            Tag::String(JavaString::from("red")),
        );
    }
    t
}

/// PlayerChatMessage as the server relays it.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatMessage {
    pub sender: u128,
    /// The link's index in the sender's chain (0 when unsigned).
    pub index: i32,
    pub signature: Option<Signature>,
    pub content: String,
    /// Epoch milliseconds.
    pub timestamp: i64,
    pub salt: i64,
    pub last_seen: Vec<Signature>,
    /// What clients show instead of the signed text (withUnsignedContent).
    pub unsigned_content: Option<Tag>,
}

impl ChatMessage {
    /// The message a chain let through: a signed one keeps its body;
    /// PlayerChatMessage.unsigned takes the time now, no salt, nothing seen.
    pub fn new(sender: u128, unpacked: Unpacked, body: &Body, now: i64) -> Self {
        let (index, signature, timestamp, salt, last_seen) = match unpacked {
            Unpacked::Unsigned => (0, None, now, 0, Vec::new()),
            Unpacked::Signed { index, signature } => (
                index,
                Some(signature),
                body.timestamp,
                body.salt,
                body.last_seen.to_vec(),
            ),
        };
        Self {
            sender,
            index,
            signature,
            content: body.content.to_owned(),
            timestamp,
            salt,
            last_seen,
            unsigned_content: None,
        }
    }
    /// hasExpiredServer.
    pub fn expired(&self, now: i64) -> bool {
        now > self.timestamp + secure_chat::EXPIRES_AFTER_SERVER
    }
    /// PlayerList.verifyChatTrusted: signed and not expired.
    pub fn trusted(&self, now: i64) -> bool {
        self.signature.is_some() && !self.expired(now)
    }
}

impl Hub {
    /// MinecraftServer.enforceSecureProfile.
    pub fn enforces_secure_chat(&self) -> bool {
        self.lan.shared.config.enforces_secure_chat()
    }

    /// Component.getString in the server's language, for the log.
    pub(crate) fn english(&self, tag: &Tag) -> String {
        match &self.lan.shared.config.log {
            Some(log) => horizon_protocol::text::plain(tag, &log.lang),
            None => horizon_protocol::text::plain(tag, &Default::default()),
        }
    }

    /// tryHandleChat: an illegal character disconnects; with chat hidden a
    /// message (not a command) is refused. Whether to go on.
    pub fn try_handle_chat(&mut self, i: usize, message: &str, command: bool) -> bool {
        let conn = self.guests[i].conn;
        if !message.chars().all(allowed) {
            self.kick(
                conn,
                translatable("multiplayer.disconnect.illegal_characters", &[]),
            );
            return false;
        }
        // ChatVisiblity.HIDDEN.
        if !command && self.guests[i].info.chat_visibility == 2 {
            self.guests[i].send(&C::SystemChat(play::SystemChat {
                content: red("chat.disabled.options"),
                overlay: false,
            }));
            return false;
        }
        true
    }

    /// unpackAndApplyLastSeen: the acknowledged messages, or a disconnect.
    pub fn apply_last_seen(
        &mut self,
        i: usize,
        update: &play::LastSeenUpdate,
        events: &mut Vec<HostEvent>,
    ) -> Option<Vec<Signature>> {
        let g = &mut self.guests[i];
        match g
            .last_seen
            .apply_update(update.offset, update.acknowledged, update.checksum)
        {
            Ok(seen) => Some(seen),
            // Vanilla logs the ValidationException's stack trace after the line.
            Err(_) => {
                events.push(HostEvent::Error(format!(
                    "Failed to validate message acknowledgements from {}",
                    g.name
                )));
                let conn = g.conn;
                self.kick(
                    conn,
                    translatable("multiplayer.disconnect.chat_validation_failed", &[]),
                );
                None
            }
        }
    }

    /// handleChatAck.
    pub(super) fn chat_ack(&mut self, i: usize, offset: i32, events: &mut Vec<HostEvent>) {
        let g = &mut self.guests[i];
        if g.last_seen.apply_offset(offset).is_err() {
            events.push(HostEvent::Error(format!(
                "Failed to validate message acknowledgement offset from {}",
                g.name
            )));
            let conn = g.conn;
            self.kick(
                conn,
                translatable("multiplayer.disconnect.chat_validation_failed", &[]),
            );
        }
    }

    /// handleMessageDecodeFailure: logged, and the reason in red for the player.
    pub fn decode_failure(&mut self, i: usize, e: DecodeError, events: &mut Vec<HostEvent>) {
        let reason = red(e.key());
        events.push(HostEvent::Warn(format!(
            "Failed to update secure chat state for {}: '{}'",
            self.guests[i].name,
            self.english(&reason)
        )));
        self.guests[i].send(&C::SystemChat(play::SystemChat {
            content: reason,
            overlay: false,
        }));
    }

    /// handleChat: the acknowledgements, then the message through the
    /// sender's chain, then to everyone.
    pub(super) fn handle_chat(&mut self, i: usize, c: play::Chat, events: &mut Vec<HostEvent>) {
        let Some(last_seen) = self.apply_last_seen(i, &c.last_seen, events) else {
            return;
        };
        if !self.try_handle_chat(i, &c.message, false) {
            return;
        }
        let (enforce, now) = (self.enforces_secure_chat(), now_millis());
        let body = Body {
            content: &c.message,
            timestamp: c.timestamp,
            salt: c.salt,
            last_seen: &last_seen,
        };
        let signature = c.signature.as_deref().and_then(secure_chat::signature);
        let g = &mut self.guests[i];
        let (uuid, name) = (g.uuid, g.name.clone());
        let message = match g
            .chain
            .unpack(uuid, signature.as_ref(), &body, enforce, now)
        {
            Ok(unpacked) => ChatMessage::new(uuid, unpacked, &body, now),
            Err(e) => return self.decode_failure(i, e, events),
        };
        if message.signature.is_some() && message.expired(now) {
            events.push(HostEvent::Warn(format!(
                "Received expired chat: '{}'. Is the client/server system time unsynchronized?",
                message.content
            )));
        }
        // The chat filter (docs/chat-filter.md): after the chain took the message, before anyone sees it.
        if !self.chat_allowed(
            i,
            &[&message.content],
            super::chat_filter::Place::Chat,
            events,
        ) {
            return;
        }
        let bound = play::ChatTypeBound {
            chat_type: self.chat_type,
            name: self.player_display_name(uuid, &name),
            target: None,
        };
        let line = format!("<{name}> {}", message.content);
        self.broadcast_chat(&message, &bound, line, events);
        events.push(HostEvent::PlayerChat {
            sender: uuid,
            text: message.content,
        });
    }

    /// handleChatSessionUpdate: a new profile key, validated against the
    /// services' keys, starts a new chain and is announced to everyone.
    pub(super) fn chat_session_update(
        &mut self,
        i: usize,
        p: play::ChatSessionUpdate,
        events: &mut Vec<HostEvent>,
    ) {
        let key = ProfileKey {
            expires_at: p.expires_at,
            key: p.key,
            signature: p.key_signature,
        };
        let g = &self.guests[i];
        let (conn, uuid) = (g.conn, g.uuid);
        let old = g.chain.session().map(|s| s.profile_key.expires_at);
        if g.chain.session().is_some_and(|s| s.profile_key == key) {
            return;
        }
        if old.is_some_and(|old| key.expires_at < old) {
            self.kick(
                conn,
                translatable("multiplayer.disconnect.expired_public_key", &[]),
            );
            return;
        }
        let secure = self.lan.shared.config.secure_chat.clone();
        let keys = secure.as_deref().map_or(&[][..], |s| s.keys());
        if keys.is_empty() {
            events.push(HostEvent::Warn(format!(
                "Ignoring chat session from {} due to missing Services public key",
                g.name
            )));
            return;
        }
        match ChatSession::validate(p.session, key, keys, uuid) {
            Ok(session) => {
                let data = session.data();
                self.guests[i].chain = Chain::keyed(session);
                // resetPlayerChatState: INITIALIZE_CHAT to every player.
                self.broadcast(&C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                    actions: 2,
                    entries: vec![play::PlayerInfo {
                        uuid,
                        chat_session: Some(data),
                        ..Default::default()
                    }],
                }));
            }
            Err(reason) => {
                events.push(HostEvent::Error(
                    "Failed to validate profile key".to_owned(),
                ));
                self.kick(conn, translatable(reason, &[]));
            }
        }
    }

    /// PlayerList.broadcastChatMessage: logged (`[Not Secure]` unless signed
    /// and fresh), then sent to every player.
    pub fn broadcast_chat(
        &mut self,
        message: &ChatMessage,
        bound: &play::ChatTypeBound,
        line: String,
        events: &mut Vec<HostEvent>,
    ) {
        let secure = message.trusted(now_millis());
        events.push(HostEvent::Chat { line, secure });
        for k in 0..self.guests.len() {
            if self.guests[k].placed {
                self.send_chat(k, message, bound);
            }
        }
    }

    /// ServerPlayer.sendChatMessage (players who show chat) and
    /// sendPlayerChatMessage: numbered for the connection, the last seen
    /// packed against its cache; a signed message joins the cache and the
    /// messages the player must acknowledge.
    pub fn send_chat(&mut self, k: usize, message: &ChatMessage, bound: &play::ChatTypeBound) {
        let g = &mut self.guests[k];
        if g.info.chat_visibility != 0 {
            return;
        }
        let last_seen = message
            .last_seen
            .iter()
            .map(|s| g.signatures.pack(s))
            .collect();
        g.send(&C::PlayerChat(play::PlayerChat {
            global_index: g.chat_index,
            sender: message.sender,
            index: message.index,
            signature: message.signature.map(|s| s.to_vec()),
            content: message.content.clone(),
            timestamp: message.timestamp,
            salt: message.salt,
            last_seen,
            unsigned_content: message.unsigned_content.clone(),
            filter: (0, Vec::new()),
            chat_type: bound.clone(),
        }));
        g.chat_index += 1;
        if let Some(signature) = &message.signature {
            g.signatures.push(&message.last_seen, Some(signature));
            g.last_seen.add_pending(signature);
            if g.last_seen.tracked_count() > 4096 {
                let conn = g.conn;
                self.kick(
                    conn,
                    translatable("multiplayer.disconnect.too_many_pending_chats", &[]),
                );
            }
        }
    }

    /// sendChatMessage of a system message: a disguised chat, to players
    /// who show chat.
    fn send_disguised(&self, k: usize, message: Tag, chat_type: play::ChatTypeBound) {
        let g = &self.guests[k];
        if g.info.chat_visibility == 0 {
            g.send(&C::DisguisedChat(play::DisguisedChat {
                message,
                chat_type,
            }));
        }
    }

    /// collectSignedArguments, or collectUnsignedArguments when the client
    /// signed none: each of the command's message arguments (`parsed`, by
    /// name) through the sender's chain.
    pub fn signed_arguments(
        &mut self,
        i: usize,
        p: &play::ChatCommandSigned,
        parsed: &[(String, String)],
        last_seen: &[Signature],
        events: &mut Vec<HostEvent>,
    ) -> Result<Vec<(String, ChatMessage)>, DecodeError> {
        let (enforce, now) = (self.enforces_secure_chat(), now_millis());
        let g = &mut self.guests[i];
        let uuid = g.uuid;
        let mut out = Vec::new();
        if p.arguments.is_empty() {
            for (name, value) in parsed {
                // SignedMessageBody.unsigned.
                let body = Body {
                    content: value,
                    timestamp: now,
                    salt: 0,
                    last_seen: &[],
                };
                let unpacked = g.chain.unpack(uuid, None, &body, enforce, now)?;
                out.push((name.clone(), ChatMessage::new(uuid, unpacked, &body, now)));
            }
            return Ok(out);
        }
        let mismatch = |events: &mut Vec<HostEvent>| {
            let names = |l: &mut dyn Iterator<Item = &String>| {
                l.map(String::as_str).collect::<Vec<_>>().join(", ")
            };
            events.push(HostEvent::Error(format!(
                "Signed command mismatch between server and client ('{}'): got [{}] from client, but expected [{}]",
                p.command,
                names(&mut p.arguments.iter().map(|a| &a.0)),
                names(&mut parsed.iter().map(|a| &a.0)),
            )));
            DecodeError::InvalidCommandSignature
        };
        for (name, signature) in &p.arguments {
            let Some((_, value)) = parsed.iter().find(|(n, _)| n == name) else {
                g.chain.set_broken();
                return Err(mismatch(events));
            };
            let body = Body {
                content: value,
                timestamp: p.timestamp,
                salt: p.salt,
                last_seen,
            };
            let signature = secure_chat::signature(signature);
            let unpacked = g
                .chain
                .unpack(uuid, signature.as_ref(), &body, enforce, now)?;
            out.push((name.clone(), ChatMessage::new(uuid, unpacked, &body, now)));
        }
        if parsed.iter().any(|(n, _)| !out.iter().any(|(o, _)| o == n)) {
            return Err(mismatch(events));
        }
        Ok(out)
    }

    /// A message command's chat (SayCommand, EmoteCommands): to everyone,
    /// bound to `chat_type` and the sender's name.
    pub fn broadcast_command_chat(
        &mut self,
        i: usize,
        chat_type: &str,
        message: Option<ChatMessage>,
        text: &str,
        events: &mut Vec<HostEvent>,
    ) {
        let (uuid, name) = (self.guests[i].uuid, self.guests[i].name.clone());
        let bound = play::ChatTypeBound {
            chat_type: self.chat_type_id(chat_type),
            name: self.player_display_name(uuid, &name),
            target: None,
        };
        let line = match chat_type {
            "say_command" => format!("[{name}] {text}"),
            _ => format!("* {name} {text}"),
        };
        match message {
            Some(message) => self.broadcast_chat(&message, &bound, line, events),
            // An unsigned command packet (no signing context):
            // PlayerChatMessage.system, a disguised chat.
            None => {
                events.push(HostEvent::Chat {
                    line,
                    secure: false,
                });
                for k in 0..self.guests.len() {
                    if self.guests[k].placed {
                        self.send_disguised(k, literal(text), bound.clone());
                    }
                }
            }
        }
    }

    /// MsgCommand.sendMessage to each target: the outgoing line for the
    /// sender, the incoming one for the target. Nothing is logged.
    pub fn whisper(
        &mut self,
        i: usize,
        targets: &[usize],
        message: Option<ChatMessage>,
        text: &str,
    ) {
        let (uuid, name) = (self.guests[i].uuid, self.guests[i].name.clone());
        let (outgoing, incoming) = (
            self.chat_type_id("msg_command_outgoing"),
            self.chat_type_id("msg_command_incoming"),
        );
        for &t in targets {
            let target = self.player_display_name(self.guests[t].uuid, &self.guests[t].name);
            let out = play::ChatTypeBound {
                chat_type: outgoing,
                name: self.player_display_name(uuid, &name),
                target: Some(target),
            };
            let inc = play::ChatTypeBound {
                chat_type: incoming,
                name: self.player_display_name(uuid, &name),
                target: None,
            };
            match &message {
                Some(m) => {
                    self.send_chat(i, m, &out);
                    self.send_chat(t, m, &inc);
                }
                None => {
                    self.send_disguised(i, literal(text), out);
                    self.send_disguised(t, literal(text), inc);
                }
            }
        }
    }

    /// A chat type's registry id.
    fn chat_type_id(&self, name: &str) -> i32 {
        self.sync
            .id("minecraft:chat_type", &format!("minecraft:{name}"))
            .unwrap_or(self.chat_type)
    }
}
