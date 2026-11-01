//! The management protocol's list edits, which `horizon-server` forwards on the control endpoint as
//! `management <connection> {"method", "entries"}` (its management_methods.rs): vanilla 26.3's
//! `AllowlistService`, `BanlistService`, `IpBanlistService` and `OperatorService` on the running server's lists. Players are found as
//! `MinecraftPlayerListService.getUser` finds them, each change is logged as `JsonRpcLogger` logs it, and the
//! players a change shuts out are disconnected. The supervisor answers with the lists, from their files.
use super::{ServerState, log};
use crate::game::Game;
use horizon_client::{
    server::{
        profiles,
        user_lists::{BanEntry, NameAndId, OpEntry, WhiteEntry},
    },
    text::Text,
};
use horizon_protocol::text::translatable;
use serde_json::{Value, json};

/// A `UserBan`: the player, the reason, the source, the end.
type Ban = BanEntry<NameAndId>;
/// An `IpBan`: the address, the reason, the source, the end.
type IpBan = BanEntry<String>;

/// `OperatorService.Op`: the player and the level and bypass asked for, which a stored operator always has.
#[derive(PartialEq)]
struct Op {
    user: NameAndId,
    level: Option<i32>,
    bypass: Option<bool>,
}

/// `NameAndId`'s record `toString`, as vanilla's log lines show it.
fn shown(user: &NameAndId) -> String {
    format!(
        "NameAndId[id={}, name={}]",
        horizon_protocol::crypto::uuid_string(user.id),
        user.name
    )
}

impl Game<'_> {
    pub(super) fn management(&mut self, state: &mut ServerState, request: &str) -> Value {
        let (connection, body) = request.split_once(' ').unwrap_or((request, "{}"));
        let rpc =
            |message: String| log("INFO", &format!("RPC Connection #{connection}: {message}"));
        let Ok(body) = serde_json::from_str::<Value>(body) else {
            return json!({ "error": "a management request is a connection number and a JSON object" });
        };
        let entries = body["entries"].as_array().cloned().unwrap_or_default();
        let lists = state.lists.clone();
        match body["method"].as_str().unwrap_or_default() {
            "allowlist/add" => {
                for user in entries.iter().filter_map(|e| self.management_user(e)) {
                    rpc(format!("Add player '{}' to allowlist", shown(&user)));
                    lists.lock().unwrap().white.add(WhiteEntry { user });
                }
            }
            "allowlist/remove" => {
                for user in entries.iter().filter_map(|e| self.management_user(e)) {
                    rpc(format!("Remove player '{}' from allowlist", shown(&user)));
                    lists.lock().unwrap().white.remove(&user);
                }
                rpc("Kick unlisted players".into());
                self.kick_unlisted_players(&lists);
            }
            "allowlist/set" => {
                let wanted: Vec<NameAndId> = entries
                    .iter()
                    .filter_map(|e| self.management_user(e))
                    .collect();
                let current: Vec<NameAndId> = lists
                    .lock()
                    .unwrap()
                    .white
                    .entries()
                    .iter()
                    .map(|e| e.user.clone())
                    .collect();
                for user in current.iter().filter(|u| !wanted.contains(u)) {
                    rpc(format!("Remove player '{}' from allowlist", shown(user)));
                    lists.lock().unwrap().white.remove(user);
                }
                for user in wanted.iter().filter(|u| !current.contains(u)) {
                    rpc(format!("Add player '{}' to allowlist", shown(user)));
                    lists
                        .lock()
                        .unwrap()
                        .white
                        .add(WhiteEntry { user: user.clone() });
                }
                rpc("Kick unlisted players".into());
                self.kick_unlisted_players(&lists);
            }
            "allowlist/clear" => {
                rpc("Clear allowlist".into());
                lists.lock().unwrap().white.clear();
            }
            "bans/add" => {
                let bans: Vec<Ban> = entries
                    .iter()
                    .filter_map(|e| self.management_ban(e))
                    .collect();
                for ban in bans {
                    self.management_add_ban(&rpc, state, ban);
                }
            }
            "bans/remove" => {
                for user in entries.iter().filter_map(|e| self.management_user(e)) {
                    rpc(format!("Remove player '{}' from banlist", shown(&user)));
                    lists.lock().unwrap().bans.remove(&user);
                }
            }
            "bans/set" => {
                let wanted: Vec<Ban> = entries
                    .iter()
                    .filter_map(|e| self.management_ban(e))
                    .collect();
                let current: Vec<Ban> = lists
                    .lock()
                    .unwrap()
                    .bans
                    .entries()
                    .into_iter()
                    .cloned()
                    .collect();
                for ban in current.iter().filter(|b| !wanted.contains(b)) {
                    rpc(format!("Remove player '{}' from banlist", shown(&ban.user)));
                    lists.lock().unwrap().bans.remove(&ban.user);
                }
                for ban in wanted.into_iter().filter(|b| !current.contains(b)) {
                    self.management_add_ban(&rpc, state, ban);
                }
            }
            // MinecraftBanListServiceImpl.clearUserBans logs nothing.
            "bans/clear" => lists.lock().unwrap().bans.clear(),
            "ip_bans/add" => {
                for entry in &entries {
                    // An address, else the address of the player named, if online.
                    let ip = entry["ip"]
                        .as_str()
                        .filter(|ip| ip.parse::<std::net::IpAddr>().is_ok());
                    let ip = ip.map(str::to_owned).or_else(|| {
                        let player = entry.get("player")?;
                        self.management_online(player).map(|(_, ip)| ip)
                    });
                    if let Some(ip) = ip {
                        self.management_ip_ban(&rpc, state, management_ip_ban(entry, ip));
                    }
                }
            }
            "ip_bans/remove" => {
                for ip in entries.iter().filter_map(|e| e["ip"].as_str()) {
                    rpc(format!("Remove ip '{ip}' from ban list"));
                    lists.lock().unwrap().ip_bans.remove(&ip.to_owned());
                }
            }
            "ip_bans/set" => {
                let wanted: Vec<IpBan> = entries
                    .iter()
                    .filter_map(|e| {
                        let ip = e["ip"]
                            .as_str()
                            .filter(|ip| ip.parse::<std::net::IpAddr>().is_ok())?;
                        Some(management_ip_ban(e, ip.to_owned()))
                    })
                    .collect();
                let current: Vec<IpBan> = lists
                    .lock()
                    .unwrap()
                    .ip_bans
                    .entries()
                    .into_iter()
                    .cloned()
                    .collect();
                for ban in current.iter().filter(|b| !wanted.contains(b)) {
                    rpc(format!("Remove ip '{}' from ban list", ban.user));
                    lists.lock().unwrap().ip_bans.remove(&ban.user);
                }
                for ban in wanted.into_iter().filter(|b| !current.contains(b)) {
                    self.management_ip_ban(&rpc, state, ban);
                }
            }
            "ip_bans/clear" => {
                rpc("Clear ip ban list".into());
                lists.lock().unwrap().ip_bans.clear();
            }
            "operators/add" => {
                let ops: Vec<Op> = entries
                    .iter()
                    .filter_map(|e| self.management_op(e))
                    .collect();
                for op in ops {
                    self.management_add_op(&rpc, state, op);
                }
            }
            "operators/remove" => {
                for user in entries.iter().filter_map(|e| self.management_user(e)) {
                    rpc(format!("Deop '{}'", shown(&user)));
                    lists.lock().unwrap().ops.remove(&user);
                }
                self.sync_ops(&lists);
            }
            "operators/set" => {
                let wanted: Vec<Op> = entries
                    .iter()
                    .filter_map(|e| self.management_op(e))
                    .collect();
                let current: Vec<Op> = lists
                    .lock()
                    .unwrap()
                    .ops
                    .entries()
                    .iter()
                    .map(|e| Op {
                        user: e.user.clone(),
                        level: Some(e.level),
                        bypass: Some(e.bypasses_player_limit),
                    })
                    .collect();
                for op in current.iter().filter(|o| !wanted.contains(o)) {
                    rpc(format!("Deop '{}'", shown(&op.user)));
                    lists.lock().unwrap().ops.remove(&op.user);
                }
                for op in wanted.into_iter().filter(|o| !current.contains(o)) {
                    self.management_add_op(&rpc, state, op);
                }
                self.sync_ops(&lists);
            }
            "operators/clear" => {
                rpc("Clear operator list".into());
                lists.lock().unwrap().ops.clear();
                self.sync_ops(&lists);
            }
            other => return json!({ "error": format!("unknown management method {other:?}") }),
        }
        json!({ "ok": true })
    }

    /// `addUserBan`, then the banned player, if online, is sent away.
    fn management_add_ban(&mut self, rpc: &impl Fn(String), state: &ServerState, ban: Ban) {
        let reason = ban.reason.clone().unwrap_or_else(|| {
            Text::translate("multiplayer.disconnect.banned.reason.default", &[]).plain()
        });
        rpc(format!(
            "Add player '{}' to banlist. Reason: '{reason}'",
            ban.user.name
        ));
        let id = ban.user.id;
        state.lists.lock().unwrap().bans.add(ban);
        self.kick_uuid(id, translatable("multiplayer.disconnect.banned", &[]));
    }

    /// `addIpBan`, then whoever is online from that address is sent away.
    fn management_ip_ban(&mut self, rpc: &impl Fn(String), state: &ServerState, ban: IpBan) {
        rpc(format!("Add ip '{}' to ban list", ban.user));
        let ip = ban.user.clone();
        state.lists.lock().unwrap().ip_bans.add(ban);
        let banned: Vec<u128> = self.lan.as_ref().map_or_else(Vec::new, |h| {
            h.hub
                .guests
                .iter()
                .filter(|g| g.address.ip().to_string() == ip)
                .map(|g| g.uuid)
                .collect()
        });
        for uuid in banned {
            self.kick_uuid(uuid, translatable("multiplayer.disconnect.ip_banned", &[]));
        }
    }

    /// `PlayerList.op`: the level asked for, else `op-permission-level`; the bypass asked for, else the one
    /// the player had. The commands an online player may run follow (`sync_ops`, after the edit).
    fn management_add_op(&mut self, rpc: &impl Fn(String), state: &ServerState, op: Op) {
        rpc(format!("Op '{}'", shown(&op.user)));
        let mut lists = state.lists.lock().unwrap();
        let had = lists
            .ops
            .get(&op.user)
            .is_some_and(|e| e.bypasses_player_limit);
        let entry = OpEntry {
            level: op.level.unwrap_or(lists.op_level),
            bypasses_player_limit: op.bypass.unwrap_or(had),
            user: op.user,
        };
        lists.ops.add(entry);
        drop(lists);
        self.sync_ops(&state.lists);
    }

    /// An `OperatorDto`: the player found, and the level and bypass when given.
    fn management_op(&self, entry: &Value) -> Option<Op> {
        Some(Op {
            user: self.management_user(&entry["player"])?,
            level: entry["permissionLevel"].as_i64().map(|l| l as i32),
            bypass: entry["bypassesPlayerLimit"].as_bool(),
        })
    }

    /// `PlayerListService.getPlayer(id, name)`: an online player, by id first; their address.
    fn management_online(&self, player: &Value) -> Option<(u128, String)> {
        let id = player["id"]
            .as_str()
            .and_then(|s| u128::from_str_radix(s, 16).ok());
        let name = player["name"].as_str();
        let guest = self
            .lan
            .as_ref()?
            .hub
            .guests
            .iter()
            .find(|g| match (id, name) {
                (Some(id), _) => g.uuid == id,
                (None, Some(name)) => g.name.eq_ignore_ascii_case(name),
                (None, None) => false,
            })?;
        Some((guest.uuid, guest.address.ip().to_string()))
    }

    /// `UserBanDto.toUserBan`: the player found, the source "Management server" unless one is given; the ban
    /// starts now.
    fn management_ban(&self, entry: &Value) -> Option<Ban> {
        let user = self.management_user(&entry["player"])?;
        let source = entry["source"].as_str().unwrap_or("Management server");
        let reason = entry["reason"].as_str().map(str::to_owned);
        Some(BanEntry::new(
            user,
            Some(source.to_owned()),
            entry["expires"].as_i64(),
            reason,
        ))
    }

    /// `MinecraftPlayerListService.getUser`: by id, the user cache, else the session service; by name, the
    /// user cache, else the profile service (an offline server's offline player).
    fn management_user(&self, player: &Value) -> Option<NameAndId> {
        if let Some(id) = player["id"]
            .as_str()
            .and_then(|s| u128::from_str_radix(s, 16).ok())
        {
            let cached = super::USER_CACHE
                .lock()
                .unwrap()
                .as_mut()
                .and_then(|c| c.get_by_id(id));
            return cached.or_else(|| profiles::find_by_id(&profiles::discovery_url(), id));
        }
        self.resolve_name(player["name"].as_str()?)
    }
}

/// `IpBan`: the reason, the source "Management server" unless one is given, the end; it starts now.
fn management_ip_ban(entry: &Value, ip: String) -> IpBan {
    let source = entry["source"].as_str().unwrap_or("Management server");
    let reason = entry["reason"].as_str().map(str::to_owned);
    BanEntry::new(
        ip,
        Some(source.to_owned()),
        entry["expires"].as_i64(),
        reason,
    )
}
