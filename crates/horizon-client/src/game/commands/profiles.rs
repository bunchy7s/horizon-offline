//! `/op`, `/deop`, `/ban`, `/pardon`, `/ban-ip`, `/pardon-ip`, `/banlist`,
//! `/setidletimeout`, `/transfer` and `/whitelist` (OpCommand, DeOpCommands,
//! BanPlayerCommands, PardonCommand, BanIpCommands, PardonIpCommand,
//! BanListCommands, SetPlayerIdleTimeoutCommand, TransferCommand,
//! WhitelistCommand): commands of a dedicated server, run on its lists by an
//! operator in game (the console has its own handlers in game/dedicated.rs).
//! Also `/stop`, `/save-all`, `/save-off` and `/save-on` (StopCommand,
//! SaveAllCommand, SaveOffCommand, SaveOnCommand), on the switches the lists
//! share.
use super::{Arg, Context, Game, R, Target, err};
use horizon_client::{
    server::user_lists::{BanEntry, NameAndId, OpEntry, WhiteEntry},
    text::Text,
};
use horizon_protocol::text::translatable;

impl Game<'_> {
    /// Commands.CommandSelection.DEDICATED: on an integrated server these do
    /// not exist.
    fn dedicated_lists(
        &self,
    ) -> R<std::sync::Arc<std::sync::Mutex<super::super::dedicated::Lists>>> {
        match (&self.server_lists, self.options.dedicated.is_some()) {
            (Some(lists), true) => Ok(lists.clone()),
            _ => Err(err("command.unknown.command", &[])),
        }
    }

    /// StopCommand: the server loop (game/dedicated.rs) ends after this tick.
    pub(super) fn cmd_stop(&mut self) -> R<i32> {
        let lists = self.dedicated_lists()?;
        self.feedback(Text::translate("commands.stop.stopping", &[]));
        lists.lock().unwrap().stop = true;
        Ok(1)
    }

    /// SaveAllCommand. Every save waits for the chunks to be written, so
    /// `flush` changes nothing.
    pub(super) fn cmd_save_all(&mut self) -> R<i32> {
        self.dedicated_lists()?;
        self.feedback(Text::translate("commands.save.saving", &[]));
        if self.lan_acting.is_some() {
            // The acting guest's record holds the game's state until their
            // turn ends: saving now would write it to their file.
            // shortcut: the other operators aren't told of a guest's save, add it if anyone asks.
            self.save_after_guest_turn = true;
            return Ok(1);
        }
        let reply = self.save_all_reply();
        self.feedback(reply);
        Ok(1)
    }

    /// Save everything; `commands.save.success`, or `commands.save.failed` in red.
    pub(in crate::game) fn save_all_reply(&mut self) -> Text {
        match self.save_game() {
            Ok(()) => Text::translate("commands.save.success", &[]),
            Err(e) => {
                log::error!("Horizon: /save-all: {e}");
                Text::translate("commands.save.failed", &[]).with_color(horizon_client::text::RED)
            }
        }
    }

    /// SaveOffCommand and SaveOnCommand.
    pub(super) fn cmd_auto_save(&mut self, enabled: bool) -> R<i32> {
        let lists = self.dedicated_lists()?;
        if !super::super::dedicated::set_auto_save(&lists, enabled) {
            let already = if enabled {
                "commands.save.alreadyOn"
            } else {
                "commands.save.alreadyOff"
            };
            return Err(err(already, &[]));
        }
        let done = if enabled {
            "commands.save.enabled"
        } else {
            "commands.save.disabled"
        };
        self.feedback(Text::translate(done, &[]));
        Ok(1)
    }

    /// `NameAndId` of a player target.
    fn target_profile(&self, t: Target) -> Option<NameAndId> {
        match t {
            Target::Player => Some(NameAndId {
                id: self.lan_acting_uuid?,
                name: self.acting_player_name().to_owned(),
            }),
            Target::Guest(conn) => self.command_guest(conn).map(|g| NameAndId {
                id: g.uuid,
                name: g.name.clone(),
            }),
            _ => None,
        }
    }

    /// `GameProfileArgument.getGameProfiles`: a selector's players
    /// (`SelectorResult.getNames`, no players is an error), else the name in
    /// the user cache.
    fn game_profiles(&mut self, ctx: &Context, name: &str) -> R<Vec<NameAndId>> {
        match ctx.get(name).cloned() {
            Some(Arg::Entity(s)) => {
                let players: Vec<Target> = self
                    .select(&s)
                    .into_iter()
                    .filter(|t| t.is_player())
                    .collect();
                if players.is_empty() {
                    return Err(err("argument.entity.notfound.player", &[]));
                }
                Ok(players
                    .into_iter()
                    .filter_map(|t| self.target_profile(t))
                    .collect())
            }
            Some(Arg::Str(text)) => self
                .resolve_name(&text)
                .map(|p| vec![p])
                .ok_or_else(|| err("argument.player.unknown", &[])),
            _ => Err(err("argument.player.unknown", &[])),
        }
    }

    /// `CommandSourceStack.getTextName`.
    fn source_text_name(&self) -> String {
        if self.console_broadcast.is_some() {
            "Server".to_owned()
        } else {
            self.acting_player_name().to_owned()
        }
    }

    /// `OpCommand.opPlayers`, `DeOpCommands.deopPlayers`,
    /// `BanPlayerCommands.banPlayers` and `PardonCommand.pardonPlayers`.
    pub(super) fn cmd_profiles(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let lists = self.dedicated_lists()?;
        let players = self.game_profiles(ctx, "targets")?;
        let lit = |s: &str| Text::literal(s.to_owned());
        let mut count = 0;
        match id {
            "op" => {
                for player in &players {
                    let added = {
                        let mut l = lists.lock().unwrap();
                        let level = l.op_level;
                        !l.ops.contains(player)
                            && l.ops.add(OpEntry {
                                user: player.clone(),
                                level,
                                bypasses_player_limit: false,
                            })
                    };
                    if added {
                        self.sync_ops(&lists);
                        count += 1;
                        self.feedback(Text::translate("commands.op.success", &[lit(&player.name)]));
                    }
                }
                if count == 0 {
                    return Err(err("commands.op.failed", &[]));
                }
            }
            "deop" => {
                for player in &players {
                    let removed = {
                        let mut l = lists.lock().unwrap();
                        l.ops.contains(player) && l.ops.remove(player)
                    };
                    if removed {
                        self.sync_ops(&lists);
                        count += 1;
                        self.feedback(Text::translate(
                            "commands.deop.success",
                            &[lit(&player.name)],
                        ));
                    }
                }
                if count == 0 {
                    return Err(err("commands.deop.failed", &[]));
                }
                self.kick_unlisted_players(&lists);
            }
            "ban" => {
                let reason = match ctx.get("reason") {
                    Some(Arg::Str(raw)) => {
                        let raw = raw.clone();
                        Some(self.expand_selectors(&raw).plain().to_string())
                    }
                    _ => None,
                };
                let source = self.source_text_name();
                for player in &players {
                    let banned = lists.lock().unwrap().bans.contains(player);
                    if banned {
                        continue;
                    }
                    lists.lock().unwrap().bans.add(BanEntry::new(
                        player.clone(),
                        Some(source.clone()),
                        None,
                        reason.clone(),
                    ));
                    count += 1;
                    // BanListEntry.getReasonMessage.
                    let shown = reason.as_deref().map_or_else(
                        || Text::translate("multiplayer.disconnect.banned.reason.default", &[]),
                        lit,
                    );
                    self.feedback(Text::translate(
                        "commands.ban.success",
                        &[lit(&player.name), shown],
                    ));
                    self.kick_uuid(
                        player.id,
                        translatable("multiplayer.disconnect.banned", &[]),
                    );
                }
                if count == 0 {
                    return Err(err("commands.ban.failed", &[]));
                }
            }
            _ => {
                for player in &players {
                    let removed = {
                        let mut l = lists.lock().unwrap();
                        l.bans.contains(player) && l.bans.remove(player)
                    };
                    if removed {
                        count += 1;
                        self.feedback(Text::translate(
                            "commands.pardon.success",
                            &[lit(&player.name)],
                        ));
                    }
                }
                if count == 0 {
                    return Err(err("commands.pardon.failed", &[]));
                }
            }
        }
        Ok(count)
    }

    /// `BanIpCommands.banIpOrName` and `PardonIpCommand.unban`: an address,
    /// or for `/ban-ip` an online player's.
    pub(super) fn cmd_ip_bans(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let lists = self.dedicated_lists()?;
        let lit = |s: &str| Text::literal(s.to_owned());
        let target = match ctx.get("target") {
            Some(Arg::Str(t)) => t.clone(),
            _ => return Err(err("command.unknown.argument", &[])),
        };
        let is_ip = target.parse::<std::net::IpAddr>().is_ok();
        if id == "pardon-ip" {
            if !is_ip {
                return Err(err("commands.pardonip.invalid", &[]));
            }
            let removed = {
                let mut l = lists.lock().unwrap();
                l.ip_bans.get(&target).is_some() && l.ip_bans.remove(&target)
            };
            if !removed {
                return Err(err("commands.pardonip.failed", &[]));
            }
            self.feedback(Text::translate(
                "commands.pardonip.success",
                &[lit(&target)],
            ));
            return Ok(1);
        }
        // PlayerList.getPlayerByName, then ServerPlayer.getIpAddress.
        let address = |name: &str| {
            let hub = &self.lan.as_ref()?.hub;
            hub.guests
                .iter()
                .find(|g| g.name.eq_ignore_ascii_case(name))
                .map(|g| g.address.ip().to_string())
        };
        let ip = if is_ip {
            Some(target)
        } else {
            address(&target)
        };
        let Some(ip) = ip else {
            return Err(err("commands.banip.invalid", &[]));
        };
        if lists.lock().unwrap().ip_bans.get(&ip).is_some() {
            return Err(err("commands.banip.failed", &[]));
        }
        let players: Vec<(u128, String)> = self
            .lan
            .as_ref()
            .map(|h| {
                h.hub
                    .guests
                    .iter()
                    .filter(|g| g.address.ip().to_string() == ip)
                    .map(|g| (g.uuid, g.name.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let reason = match ctx.get("reason") {
            Some(Arg::Str(raw)) => {
                let raw = raw.clone();
                Some(self.expand_selectors(&raw).plain().to_string())
            }
            _ => None,
        };
        let source = self.source_text_name();
        lists.lock().unwrap().ip_bans.add(BanEntry::new(
            ip.clone(),
            Some(source),
            None,
            reason.clone(),
        ));
        self.feedback(Text::translate(
            "commands.banip.success",
            &[lit(&ip), Self::reason_message(reason.as_deref())],
        ));
        if !players.is_empty() {
            let names: Vec<&str> = players.iter().map(|(_, n)| n.as_str()).collect();
            self.feedback(Text::translate(
                "commands.banip.info",
                &[lit(&players.len().to_string()), lit(&names.join(", "))],
            ));
        }
        for (uuid, _) in &players {
            self.kick_uuid(*uuid, translatable("multiplayer.disconnect.ip_banned", &[]));
        }
        Ok(players.len() as i32)
    }

    /// `SetPlayerIdleTimeoutCommand.setIdleTimeout`: the minutes of idleness
    /// after which a player is kicked (`multiplayer.disconnect.idling`), 0 for
    /// never. The reply names the minutes, or says the timeout is off, and the
    /// command's result is the number of minutes.
    pub(super) fn cmd_setidle_timeout(&mut self, ctx: &Context) -> R<i32> {
        let lists = self.dedicated_lists()?;
        let Some(Arg::Int(minutes)) = ctx.get("minutes") else {
            return Ok(0);
        };
        let minutes = *minutes;
        // DedicatedServer.setPlayerIdleTimeout: the live sweep reads it, and
        // server.properties keeps it.
        self.set_player_idle_timeout(&lists, minutes);
        self.feedback(idle_timeout_reply(minutes));
        Ok(minutes)
    }

    /// `TransferCommand.register` and `TransferCommand.transfer`: each target's
    /// client is handed to `hostname:port` (`ClientboundTransferPacket`), and
    /// the source is told who went — the player's name for one, the count for
    /// several. Without a players argument the source's own player is the
    /// target, as `getPlayerOrException` asks.
    pub(super) fn cmd_transfer(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Str(hostname)) = ctx.get("hostname").cloned() else {
            return Ok(0);
        };
        // `IntegerArgumentType.getInteger(c, "port")`, else 25565.
        let port = match ctx.get("port") {
            Some(Arg::Int(port)) => *port,
            _ => 25565,
        };
        let targets = if ctx.get("players").is_some() {
            self.entities(ctx, "players", true)?
        } else if self.lan_acting.is_none() {
            // CommandSourceStack.getPlayerOrException: a console or a command
            // block has no player to send.
            return Err(err("permissions.requires.player", &[]));
        } else {
            vec![Target::Player]
        };
        let players: Vec<(u32, String)> = targets
            .iter()
            .filter_map(|target| {
                let conn = match *target {
                    Target::Player => self.lan_acting?,
                    Target::Guest(conn) => conn,
                    _ => return None,
                };
                Some((conn, self.command_guest(conn)?.name.clone()))
            })
            .collect();
        if players.is_empty() {
            return Err(err("commands.transfer.error.no_players", &[]));
        }
        if let Some(host) = self.lan.as_mut() {
            for (conn, _) in &players {
                host.hub.transfer(*conn, &hostname, port);
            }
        }
        // CommandResponseTracker.messages: one player by display name, several
        // by count, both with the hostname and the port.
        let lit = |s: &str| Text::literal(s.to_owned());
        let (key, first) = match players.len() {
            1 => ("commands.transfer.success.single", lit(&players[0].1)),
            n => ("commands.transfer.success.multiple", lit(&n.to_string())),
        };
        self.feedback(Text::translate(
            key,
            &[first, lit(&hostname), lit(&port.to_string())],
        ));
        Ok(players.len() as i32)
    }

    /// `BanListCommands.showList`: the players' bans then the addresses'
    /// (`banlist`), or one of them; told to the source only.
    pub(super) fn cmd_banlist(&mut self, id: &str) -> R<i32> {
        let lists = self.dedicated_lists()?;
        let lit = |s: &str| Text::literal(s.to_owned());
        let entries: Vec<(String, String, Option<String>)> = {
            let l = lists.lock().unwrap();
            let players = l
                .bans
                .entries()
                .into_iter()
                .map(|b| (b.user.name.clone(), b.source.clone(), b.reason.clone()));
            let ips = l
                .ip_bans
                .entries()
                .into_iter()
                .map(|b| (b.user.clone(), b.source.clone(), b.reason.clone()));
            match id {
                "banlist.ips" => ips.collect(),
                "banlist.players" => players.collect(),
                _ => players.chain(ips).collect(),
            }
        };
        if entries.is_empty() {
            self.feedback_unbroadcast(Text::translate("commands.banlist.none", &[]));
            return Ok(0);
        }
        self.feedback_unbroadcast(Text::translate(
            "commands.banlist.list",
            &[lit(&entries.len().to_string())],
        ));
        for (name, source, reason) in &entries {
            self.feedback_unbroadcast(Text::translate(
                "commands.banlist.entry",
                &[
                    lit(name),
                    lit(source),
                    Self::reason_message(reason.as_deref()),
                ],
            ));
        }
        Ok(entries.len() as i32)
    }

    /// `BanListEntry.getReasonMessage`.
    fn reason_message(reason: Option<&str>) -> Text {
        reason.map_or_else(
            || Text::translate("multiplayer.disconnect.banned.reason.default", &[]),
            |r| Text::literal(r.to_owned()),
        )
    }

    /// `WhitelistCommand`: `on`, `off`, `list`, `reload`, `add` and `remove`.
    pub(super) fn cmd_whitelist(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let lists = self.dedicated_lists()?;
        let lit = |s: &str| Text::literal(s.to_owned());
        match id {
            "whitelist.reload" => {
                // PlayerList.reloadWhiteList.
                let _ = lists.lock().unwrap().white.load();
                self.feedback(Text::translate("commands.whitelist.reloaded", &[]));
                self.kick_unlisted_players(&lists);
                Ok(1)
            }
            "whitelist.on" => {
                if lists.lock().unwrap().use_whitelist {
                    return Err(err("commands.whitelist.alreadyOn", &[]));
                }
                self.set_using_whitelist(&lists, true);
                self.feedback(Text::translate("commands.whitelist.enabled", &[]));
                self.kick_unlisted_players(&lists);
                Ok(1)
            }
            "whitelist.off" => {
                if !lists.lock().unwrap().use_whitelist {
                    return Err(err("commands.whitelist.alreadyOff", &[]));
                }
                self.set_using_whitelist(&lists, false);
                self.feedback(Text::translate("commands.whitelist.disabled", &[]));
                Ok(1)
            }
            "whitelist.list" => {
                let names: Vec<String> = lists
                    .lock()
                    .unwrap()
                    .white
                    .entries()
                    .iter()
                    .map(|e| e.user.name.clone())
                    .collect();
                if names.is_empty() {
                    self.feedback_unbroadcast(Text::translate("commands.whitelist.none", &[]));
                } else {
                    self.feedback_unbroadcast(Text::translate(
                        "commands.whitelist.list",
                        &[lit(&names.len().to_string()), lit(&names.join(", "))],
                    ));
                }
                Ok(names.len() as i32)
            }
            _ => {
                let add = id == "whitelist.add";
                let targets = self.game_profiles(ctx, "targets")?;
                let mut count = 0;
                for target in &targets {
                    let done = {
                        let mut l = lists.lock().unwrap();
                        if add {
                            !l.white.contains(target)
                                && l.white.add(WhiteEntry {
                                    user: target.clone(),
                                })
                        } else {
                            l.white.contains(target) && l.white.remove(target)
                        }
                    };
                    if done {
                        count += 1;
                        let key = if add {
                            "commands.whitelist.add.success"
                        } else {
                            "commands.whitelist.remove.success"
                        };
                        self.feedback(Text::translate(key, &[lit(&target.name)]));
                    }
                }
                if count == 0 {
                    return Err(err(
                        if add {
                            "commands.whitelist.add.failed"
                        } else {
                            "commands.whitelist.remove.failed"
                        },
                        &[],
                    ));
                }
                if !add {
                    self.kick_unlisted_players(&lists);
                }
                Ok(count)
            }
        }
    }
}

/// `SetPlayerIdleTimeoutCommand.setIdleTimeout`'s reply: the minutes, or
/// `commands.setidletimeout.success.disabled` when the timeout is off.
fn idle_timeout_reply(minutes: i32) -> Text {
    if minutes > 0 {
        Text::translate(
            "commands.setidletimeout.success",
            &[Text::literal(minutes.to_string())],
        )
    } else {
        Text::translate("commands.setidletimeout.success.disabled", &[])
    }
}

#[cfg(test)]
mod tests {
    use super::idle_timeout_reply;

    /// `SetPlayerIdleTimeoutCommand.setIdleTimeout`: minutes above zero are
    /// named in `commands.setidletimeout.success`, and zero turns the timeout
    /// off with `commands.setidletimeout.success.disabled`.
    #[test]
    fn the_idle_timeout_reply_names_the_minutes_or_says_it_is_off() {
        let reply = idle_timeout_reply(5);
        assert_eq!(
            reply.translation_key(),
            Some("commands.setidletimeout.success"),
            "{reply:?}"
        );
        assert_eq!(reply.component()["with"], serde_json::json!([5]));
        let off = idle_timeout_reply(0);
        assert_eq!(
            off.translation_key(),
            Some("commands.setidletimeout.success.disabled")
        );
        // The disabled message takes no argument.
        assert!(off.component().get("with").is_none(), "{off:?}");
    }
}
