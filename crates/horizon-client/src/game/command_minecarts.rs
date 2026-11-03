//! `MinecartCommandBlock`: its `BaseCommandBlock` run by a powered activator
//! rail (`performCommand`, `CloseableCommandBlockSource`), the entity data
//! that carries its command and last output to clients, and the edit screen's
//! `ServerboundSetCommandMinecartPacket` with `handleSetCommandMinecart`.
use super::{
    Game,
    command_blocks::CommandBlockScreen,
    commands::{Target, execute::Source},
    lan::LanHost,
    minecarts::{CartKind, Minecart},
};
use horizon_client::{command_block::Mode, text::Text};
use horizon_protocol::packets::play::{self, Serverbound as S};

impl Game<'_> {
    fn command_cart_mut(&mut self, id: u32) -> Option<&mut Minecart> {
        self.carts
            .iter_mut()
            .find(|c| c.id == id && c.kind == CartKind::CommandBlock)
    }
    /// `MinecartCommandBase.onUpdated`: the cart's command and last output go
    /// into its synched data (`DATA_ID_COMMAND_NAME`, `DATA_ID_LAST_OUTPUT`).
    fn cart_command_updated(&mut self, id: u32) {
        if let Some(c) = self.command_cart_mut(id) {
            c.synced_command = c.command.command.clone();
            c.synced_output = c.command.last_output_string();
        }
    }
    /// `BaseCommandBlock.performCommand` for the cart: false when it already
    /// ran this game tick.
    pub(super) fn perform_cart_command(&mut self, id: u32) -> bool {
        let game_time = self.world.game_time;
        let rules_work = self.ui.rules.bool("command_blocks_work");
        let Some(c) = self.command_cart_mut(id) else {
            return false;
        };
        if c.command.last_execution == game_time {
            return false;
        }
        if c.command.command.eq_ignore_ascii_case("Searge") {
            c.command.last_output = Some(Text::literal("#itzlipofutzli"));
            c.command.success_count = 1;
            return true;
        }
        c.command.success_count = 0;
        let command = c.command.command.clone();
        if rules_work && !command.is_empty() {
            c.command.last_output = None;
            // MinecartCommandBase.createCommandSourceStack: the cart's
            // position and rotation at game master level.
            let source = Source {
                entity: Some(Target::Cart(id)),
                pos: [c.position.x, c.position.y, c.position.z],
                rot: [c.yaw, 0.0],
                eyes: false,
                silent: false,
                store: None,
                dimension: self.dim,
                level: None,
                block: None,
                cart: Some(id),
            };
            let successes = self.run_as_game_master(&command, source);
            if let Some(c) = self.command_cart_mut(id) {
                c.command.success_count = successes;
            }
        }
        if let Some(c) = self.command_cart_mut(id) {
            c.command.last_execution = if c.command.update_last_execution {
                game_time
            } else {
                -1
            };
        }
        true
    }
    /// `CloseableCommandBlockSource.sendSystemMessage` for the cart: its last
    /// output stamped with the time, then `onUpdated`. Nothing when it does
    /// not track output (`CommandSource.NULL`).
    pub(super) fn cart_command_output(&mut self, id: u32, message: Text) {
        let Some(c) = self.command_cart_mut(id) else {
            return;
        };
        if c.command.track_output {
            let time = chrono::Local::now().format("[%H:%M:%S] ").to_string();
            c.command.last_output = Some(Text::literal(time).append(message));
            self.cart_command_updated(id);
        }
    }
    /// `CommandSourceStack.sendSuccess` from a command minecart: its output
    /// when `send_command_feedback`, and `broadcastToAdmins` when
    /// `command_block_output`.
    pub(super) fn cart_command_success(&mut self, id: u32, message: Text, broadcast: bool) {
        let Some(name) = self
            .command_cart_mut(id)
            .filter(|c| c.command.track_output)
            .map(|c| c.command.name())
        else {
            return;
        };
        let feedback = self.ui.rules.bool("send_command_feedback");
        if feedback {
            self.cart_command_output(id, message.clone());
        }
        if broadcast && feedback && self.ui.rules.bool("command_block_output") {
            self.broadcast_source_to_admins(name, message);
        }
    }

    /// `LocalPlayer.openMinecartCommandBlock`: the editor over the cart's
    /// command (`MinecartCommandBlockEditScreen`).
    pub(super) fn open_cart_command_block(&mut self, id: u32) {
        let Some(block) = self
            .carts
            .iter()
            .find(|c| c.id == id && c.kind == CartKind::CommandBlock)
            .map(|c| c.command.clone())
        else {
            return;
        };
        self.open_edit_screen((0, 0, 0), Some(id), block, Mode::Redstone, false);
    }
    /// The edit screen's Done for a cart (`populateAndSendPacket`, then
    /// `onDone` clears the last output of a cart that no longer tracks it): a
    /// server's client sends the packet, the window's own world applies it.
    pub(super) fn set_command_minecart(&mut self, id: u32, s: &CommandBlockScreen) {
        if !self.can_use_game_master_blocks() {
            self.system_message(Text::translate("advMode.notAllowed", &[]));
            return;
        }
        let command = s.command.value().to_owned();
        let track_output = s.track_output;
        // The output button set the cart's own flag as it was pressed.
        if let Some(c) = self.command_cart_mut(id) {
            c.command.track_output = track_output;
        }
        if self.remote.is_some() {
            self.send(S::SetCommandMinecart(play::SetCommandMinecart {
                entity: id as i32,
                command,
                track_output,
            }));
        } else {
            self.apply_cart_command(id, command, track_output);
        }
        if !track_output && let Some(c) = self.command_cart_mut(id) {
            c.command.last_output = None;
        }
    }
    /// `ServerGamePacketListenerImpl.handleSetCommandMinecart` for a guest.
    pub(super) fn lan_set_command_minecart(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        p: play::SetCommandMinecart,
    ) {
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let allowed = g.creative() && g.permission >= 2;
        self.as_guest_messaged(host, conn, |game| {
            if allowed {
                game.apply_cart_command(p.entity as u32, p.command, p.track_output);
            } else {
                game.system_message(Text::translate("advMode.notAllowed", &[]));
            }
        });
    }
    /// What `handleSetCommandMinecart` does once the player may edit: the
    /// command (which resets the success count), output tracking, the cleared
    /// last output, `onUpdated` while command blocks work, and the chat note.
    pub(super) fn apply_cart_command(&mut self, id: u32, command: String, track_output: bool) {
        let enabled = self.ui.rules.bool("command_blocks_work");
        let Some(c) = self.command_cart_mut(id) else {
            return;
        };
        c.command.set_command(&command);
        c.command.track_output = track_output;
        if !track_output {
            c.command.last_output = None;
        }
        if enabled {
            self.cart_command_updated(id);
        }
        if !command.is_empty() {
            let key = if enabled {
                "advMode.setCommand.success"
            } else {
                "advMode.setCommand.disabled"
            };
            self.system_message(Text::translate(key, &[Text::literal(command)]));
        }
    }
}
