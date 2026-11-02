//! Secure chat on a server's client (ClientPacketListener): the chat
//! session made from the account's chat key pair (setKeyPair), signed chat
//! and signed command arguments (sendChat, sendCommand), and the messages
//! the client acknowledges as seen (LastSeenMessagesTracker,
//! markMessageAsProcessed). Without a signed-in
//! account, or on a server that plays the account under another profile
//! (offline mode), there is no session and everything goes unsigned.
use super::{Game, Screen, books::BookScreen, dialogs::ClosedDialog};
use horizon_client::{
    auth::{self, ChatKeyPair},
    command::{Dispatcher, Registries},
    confirm_screen::Choice,
    hud::HudBatch,
    text::Text,
};
use horizon_protocol::{
    packets::play::{self, Serverbound as S},
    secure_chat::{Body, Encoder, LastSeenTracker, Signature},
};
use std::sync::Arc;
use winit::keyboard::KeyCode;

/// ClientPacketListener.sendChatAcknowledgement's threshold.
const ACK_AFTER: i32 = 64;

/// ClientPacketListener.CommandCheckResult.
#[derive(Clone, Copy, Debug, PartialEq)]
enum CommandCheck {
    NoIssues,
    ParseErrors,
    SignatureRequired,
    PermissionsRequired,
}

/// ClientPacketListener.verifyCommand over the tree the server sent.
fn verify_command(tree: &Dispatcher, registries: &Registries, command: &str) -> CommandCheck {
    let parse = tree.parse(command, registries);
    if !parse.is_valid() {
        return CommandCheck::ParseErrors;
    }
    if !parse.signable_arguments(tree).is_empty() {
        return CommandCheck::SignatureRequired;
    }
    if !tree
        .without_restricted()
        .parse(command, registries)
        .is_valid()
    {
        return CommandCheck::PermissionsRequired;
    }
    CommandCheck::NoIssues
}

/// What the confirmation's first button does.
#[derive(Clone, Copy, Debug, PartialEq)]
enum CommandAccept {
    /// openCommandSendConfirmationWindow: the command goes as it is.
    Run,
    /// openSignedCommandSendConfirmationWindow: into the chat box, for the
    /// player to read and send.
    Suggest,
    /// The same with a screen to come back to: on the clipboard.
    Copy,
}

/// The screen a clicked command left, which answering no returns to
/// (openSendConfirmationWindow's `currentScreen`).
pub(super) enum Behind {
    Book(Box<BookScreen>),
    Dialog(ClosedDialog),
}

/// The `ConfirmScreen` of ClientPacketListener.openSendConfirmationWindow.
pub(super) struct CommandPrompt {
    command: String,
    message: &'static str,
    accept: CommandAccept,
    /// A screen is open behind it: the second button says Back, not Cancel.
    over_screen: bool,
    behind: Option<Behind>,
    /// The button Tab or an arrow key moved to.
    focus: Option<usize>,
}

/// `FriendlyByteBuf.readUtf(256)` counts UTF-16 code units, not Unicode scalar
/// values. Keep a whole scalar at the boundary so the wire string is valid.
fn truncate_utf16(content: &str, max_units: usize) -> String {
    let mut units = 0;
    content
        .chars()
        .take_while(|ch| {
            let next = units + ch.len_utf16();
            (next <= max_units).then(|| units = next).is_some()
        })
        .collect()
}

/// A connection's secure chat state, new with every login packet.
#[derive(Default)]
pub(super) struct SecureChat {
    /// LocalChatSession: the key pair it signs with, and its chain.
    session: Option<(Arc<ChatKeyPair>, Encoder)>,
    tracker: LastSeenTracker,
}

impl SecureChat {
    pub(super) fn signed(&self) -> bool {
        self.session.is_some()
    }
    /// handleDeleteChat: a deleted message that is still pending is not
    /// acknowledged.
    pub(super) fn ignore_pending(&mut self, signature: &Signature) {
        self.tracker.ignore_pending(signature);
    }
}

/// Crypt.SaltSupplier.getLong: a SecureRandom long.
fn salt() -> i64 {
    i64::from_be_bytes(random())
}

/// Bytes from the OS. A failure is logged, not hidden: what it leaves is not random.
fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    if let Err(e) = horizon_protocol::crypto::random_bytes(&mut bytes) {
        log::error!("Secure chat has no randomness: {e}");
    }
    bytes
}

/// Encoder.pack; a message that can't be signed goes without, and says so.
fn sign(keys: &ChatKeyPair, encoder: &mut Encoder, body: &Body) -> Option<Signature> {
    let signature = encoder.sign(&keys.private, body);
    if signature.is_none() {
        log::error!("Failed to sign a chat message");
    }
    signature
}

/// UUID.randomUUID: version 4, IETF variant.
fn random_uuid() -> u128 {
    let mut bytes: [u8; 16] = random();
    bytes[6] = bytes[6] & 0x0f | 0x40;
    bytes[8] = bytes[8] & 0x3f | 0x80;
    u128::from_be_bytes(bytes)
}

impl Game<'_> {
    /// handleLogin: a new session state and signature cache, and prepareKeyPair.
    pub(super) fn remote_chat_login(&mut self) {
        if let Some(r) = self.remote.as_mut() {
            r.secure_chat = SecureChat::default();
            r.signatures = Default::default();
        }
        // Only when the server says it is in online mode (handleLogin).
        if self.remote.as_ref().is_some_and(|r| r.online_mode) {
            auth::prepare_chat_keys(true);
            self.update_chat_session();
        }
    }

    /// ClientPacketListener.tick's key refresh (shouldRefreshKeyPair), then
    /// setKeyPair with whatever key pair is ready.
    pub(super) fn tick_chat_session(&mut self) {
        // shouldRefreshKeyPair is asked only once there is a chat session; a
        // first key pair arrives through the join's prepareKeyPair.
        if self
            .remote
            .as_ref()
            .is_some_and(|r| r.logged_in && r.online_mode)
        {
            if self.remote.as_ref().is_some_and(|r| r.secure_chat.signed()) {
                auth::prepare_chat_keys(false);
            }
            self.update_chat_session();
        }
    }

    /// setKeyPair: only for the signed-in account's own profile
    /// (isLocalPlayer), and only when the key pair changed: a new session
    /// (random id, chain from link 0) the server hears of at once.
    fn update_chat_session(&mut self) {
        let Some(account) = auth::current() else {
            return;
        };
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        if !r.logged_in || r.profile() != account.uuid {
            return;
        }
        let Some(keys) = auth::chat_keys() else {
            return;
        };
        if r.secure_chat
            .session
            .as_ref()
            .is_some_and(|(k, _)| Arc::ptr_eq(k, &keys))
        {
            return;
        }
        let id = random_uuid();
        let packet = S::ChatSessionUpdate(play::ChatSessionUpdate {
            session: id,
            expires_at: keys.public.expires_at,
            key: keys.public.key.clone(),
            key_signature: keys.public.signature.clone(),
        });
        r.secure_chat.session = Some((keys, Encoder::new(account.uuid, id)));
        self.send(packet);
    }

    /// ClientPacketListener.sendChat: the last-seen update goes with every
    /// message; the signature only with a chat session.
    pub(super) fn send_signed_chat(&mut self, content: &str) {
        let content = truncate_utf16(content, 256);
        let (timestamp, salt) = (auth::now_millis(), salt());
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let chat = &mut r.secure_chat;
        let seen = chat.tracker.update();
        let signature = chat.session.as_mut().and_then(|(keys, encoder)| {
            sign(
                keys,
                encoder,
                &Body {
                    content: &content,
                    timestamp,
                    salt,
                    last_seen: &seen.last_seen,
                },
            )
        });
        self.send(S::Chat(play::Chat {
            message: content,
            timestamp,
            salt,
            signature: signature.map(|s| s.to_vec()),
            last_seen: seen.update,
        }));
    }

    /// ClientPacketListener.sendCommand: a command with message arguments
    /// (SignableCommand.of over the chat's tree, the server's once it sent one) has each of them signed,
    /// all with the same time, salt and last seen (without a chat session the
    /// signature list is empty, as in vanilla); any other command goes as the
    /// unsigned packet.
    pub(super) fn send_command(&mut self, command: &str) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let arguments: Vec<(String, String)> = if r.chat_tree_received {
            r.chat_tree
                .parse(command, &self.ui.registries)
                .signable_arguments(&r.chat_tree)
        } else {
            Vec::new()
        };
        if arguments.is_empty() {
            self.send(S::ChatCommand(play::ChatCommand {
                command: command.to_owned(),
            }));
            return;
        }
        let (timestamp, salt) = (auth::now_millis(), salt());
        let chat = &mut r.secure_chat;
        let seen = chat.tracker.update();
        let mut signatures = Vec::new();
        if let Some((keys, encoder)) = chat.session.as_mut() {
            for (name, value) in arguments {
                let body = Body {
                    content: &value,
                    timestamp,
                    salt,
                    last_seen: &seen.last_seen,
                };
                if let Some(signature) = sign(keys, encoder, &body) {
                    signatures.push((name, signature.to_vec()));
                }
            }
        }
        self.send(S::ChatCommandSigned(play::ChatCommandSigned {
            command: command.to_owned(),
            timestamp,
            salt,
            arguments: signatures,
            last_seen: seen.update,
        }));
    }

    /// ClientPacketListener.sendUnattendedCommand: a command the player
    /// didn't type (a clicked chat line, a dialog's button, a mod's command)
    /// goes unsigned when `verify_command` has no issue with it. Any other
    /// asks first, and one with a message argument is never sent from here:
    /// the player can only send it themselves, from the chat box. So neither
    /// a server nor a mod can have the player sign a message they never saw.
    pub(super) fn send_unattended_command(&mut self, command: &str) {
        let command = command.trim();
        let command = command.strip_prefix('/').unwrap_or(command);
        if command.is_empty() || !self.chat_abilities().allows(&format!("/{command}")) {
            return;
        }
        let Some(r) = self.remote.as_ref() else {
            return;
        };
        // Vanilla's dispatcher is empty until the server sends its tree.
        let check = if r.chat_tree_received {
            verify_command(&r.chat_tree, &self.ui.registries, command)
        } else {
            CommandCheck::ParseErrors
        };
        // screenAfterCommand: what stays once it has run, a dialog included.
        let over_screen = self.screen != Screen::Playing || self.dialog_open();
        let (message, accept) = match check {
            CommandCheck::NoIssues => {
                self.send(S::ChatCommand(play::ChatCommand {
                    command: command.to_owned(),
                }));
                return;
            }
            CommandCheck::ParseErrors => (
                "multiplayer.confirm_command.parse_errors",
                CommandAccept::Run,
            ),
            CommandCheck::PermissionsRequired => (
                "multiplayer.confirm_command.permissions_required",
                CommandAccept::Run,
            ),
            CommandCheck::SignatureRequired => (
                "multiplayer.confirm_command.signature_required",
                if over_screen {
                    CommandAccept::Copy
                } else {
                    CommandAccept::Suggest
                },
            ),
        };
        self.command_prompt = Some(CommandPrompt {
            command: command.to_owned(),
            message,
            accept,
            over_screen,
            behind: None,
            focus: None,
        });
        self.capture(false);
    }

    /// The command just clicked left this screen: if it asks, no returns there.
    pub(super) fn command_prompt_returns_to(&mut self, behind: Behind) {
        if let Some(prompt) = self.command_prompt.as_mut() {
            prompt.behind = Some(behind);
            prompt.over_screen = true;
        }
    }

    /// The translation key of the open confirmation's message.
    pub(super) fn command_prompt_message(&self) -> Option<&'static str> {
        self.command_prompt.as_ref().map(|p| p.message)
    }

    /// The confirmation's title, the command in yellow in its message, and
    /// its two buttons.
    fn command_prompt_layout(&self, prompt: &CommandPrompt, size: (f32, f32)) -> Choice {
        let command = Text::literal(&prompt.command).with_color(0xFF_FF55);
        let mut choice = Choice::new(
            &self.gui.atlas.font,
            size,
            &Text::translate("multiplayer.confirm_command.title", &[]),
            &Text::translate(prompt.message, &[command]),
            2,
            150.0,
        );
        choice.focus = prompt.focus;
        choice
    }

    pub(super) fn draw_command_prompt(&self, batch: &mut HudBatch) {
        let Some(prompt) = &self.command_prompt else {
            return;
        };
        horizon_client::widgets::begin_screen(|| {
            super::screen_info::confirm(
                "multiplayer/confirm_command",
                &super::screen_info::tr("multiplayer.confirm_command.title", &[]),
                &prompt.command,
            )
        });
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        horizon_client::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        let labels = [
            Text::translate(
                match prompt.accept {
                    CommandAccept::Run => "multiplayer.confirm_command.run_command",
                    CommandAccept::Suggest => "multiplayer.confirm_command.suggest_command",
                    CommandAccept::Copy => "chat.copy",
                },
                &[],
            ),
            Text::translate(
                if prompt.over_screen {
                    "gui.back"
                } else {
                    "gui.cancel"
                },
                &[],
            ),
        ];
        self.command_prompt_layout(prompt, (w, h))
            .draw(batch, atlas, self.cursor, &labels);
    }

    /// The confirmation's answer; on no, what was behind it is back.
    pub(super) fn answer_command_prompt(&mut self, yes: bool) {
        let Some(prompt) = self.command_prompt.take() else {
            return;
        };
        match (yes, prompt.accept, prompt.behind) {
            (true, CommandAccept::Run, _) => {
                if self.remote.is_some() {
                    self.send(S::ChatCommand(play::ChatCommand {
                        command: prompt.command,
                    }));
                }
            }
            // Gui.openChatAndAddText.
            (true, CommandAccept::Suggest, _) => self.open_chat(&format!("/{}", prompt.command)),
            (true, CommandAccept::Copy, _) => {
                use horizon_client::clipboard::Clipboard;
                self.clipboard.set(&format!("/{}", prompt.command));
            }
            (false, _, Some(Behind::Book(book))) => self.book = Some(*book),
            (false, _, Some(Behind::Dialog(dialog))) => self.reopen_dialog(dialog),
            (false, _, None) => {}
        }
        if self.screen == Screen::Playing && !self.screen_over_playing() {
            self.capture(true);
        }
    }

    /// ConfirmScreen.keyPressed, and what its buttons take from the keyboard
    /// (Screen's focus navigation, AbstractButton's Enter and Space).
    pub(super) fn command_prompt_key(&mut self, key: KeyCode) {
        let (size, shift) = (self.gui_size(), self.has_shift_down());
        let Some(prompt) = &self.command_prompt else {
            return;
        };
        let mut choice = self.command_prompt_layout(prompt, size);
        match key {
            KeyCode::Escape => return self.answer_command_prompt(false),
            KeyCode::Tab => choice.tab(shift),
            KeyCode::ArrowLeft | KeyCode::ArrowRight => choice.arrow(key == KeyCode::ArrowLeft),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                if let Some(button) = choice.pressed() {
                    self.audio.play_button_click();
                    self.answer_command_prompt(button == 0);
                }
                return;
            }
            _ => return,
        }
        let focus = choice.focus;
        if let Some(prompt) = self.command_prompt.as_mut() {
            prompt.focus = focus;
        }
    }

    pub(super) fn command_prompt_click(&mut self) {
        let Some(prompt) = &self.command_prompt else {
            return;
        };
        let pressed = self
            .command_prompt_layout(prompt, self.gui_size())
            .pressed_at(self.cursor);
        if let Some(button) = pressed {
            self.audio.play_button_click();
            self.answer_command_prompt(button == 0);
        }
    }

    /// markMessageAsProcessed, when the chat takes a signed message (past
    /// any Chat Delay): whether it was shown, and an acknowledgement once more
    /// than 64 wait (sendChatAcknowledgement).
    pub(super) fn mark_message_processed(&mut self, signature: &Signature, shown: bool) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let tracker = &mut r.secure_chat.tracker;
        if tracker.add_pending(signature, shown) && tracker.offset() > ACK_AFTER {
            let offset = tracker.take_offset();
            self.send(S::ChatAck(play::ChatAck { offset }));
        }
    }

    /// `ClientPacketListener.sendChatAcknowledgement`, used after a
    /// configuration transition flushes the delayed chat queue.
    pub(super) fn send_chat_acknowledgement(&mut self) {
        let offset = self
            .remote
            .as_mut()
            .map(|r| r.secure_chat.tracker.take_offset())
            .unwrap_or(0);
        if offset > 0 {
            self.send(S::ChatAck(play::ChatAck { offset }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_utf16;

    #[test]
    fn chat_limit_counts_utf16_units() {
        let emojis = "😀".repeat(256);
        assert_eq!(truncate_utf16(&emojis, 256).encode_utf16().count(), 256);
        assert_eq!(truncate_utf16(&emojis, 255).encode_utf16().count(), 254);
        let mixed = format!("{}😀tail", "a".repeat(255));
        assert_eq!(truncate_utf16(&mixed, 256), "a".repeat(255));
    }
}
