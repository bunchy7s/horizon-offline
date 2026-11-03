//! The methods of vanilla's management protocol (`management.rs`), as vanilla 26.3's `methods/*Service` answer
//! them: the players, the server's state, and the lists. Kicks, saves and messages go through the game's
//! console, so they log and save as an operator's command would; the lists change through the game's
//! `management` request (game/dedicated/management.rs), which finds the players and sends the shut-out away as
//! vanilla's services do.

use crate::{
    api::{Phase, Shared},
    engine,
    management::{Failure, INTERNAL_ERROR, invalid_params},
};
use serde_json::{Value, json};
use std::{path::Path, sync::atomic::Ordering};

/// A method's result (`short`: its name without `minecraft:`).
pub fn call(shared: &Shared, client: u32, short: &str, argument: &Value) -> Result<Value, Failure> {
    let (game, lists) = (Game { shared }, Lists(&shared.dir));
    let edit = |entries: Vec<Value>| game.edit(client, short, entries);
    match short {
        "players" => Ok(Value::Array(game.online()?)),
        "players/kick" => game.kick(argument),
        "server/status" => Ok(game.status()),
        "server/save" => {
            let flush = argument
                .as_bool()
                .ok_or_else(|| invalid_params("Not a boolean"))?;
            game.console(if flush { "save-all flush" } else { "save-all" })?;
            Ok(json!(true))
        }
        "server/stop" => {
            shared.stop.store(true, Ordering::SeqCst);
            Ok(json!(true))
        }
        "server/system_message" => game.system_message(argument),
        "serversettings/autosave" => game.autosave(client, None),
        "serversettings/autosave/set" => game.autosave(
            client,
            Some(
                argument
                    .as_bool()
                    .ok_or_else(|| invalid_params(format!("Not a boolean: {argument}")))?,
            ),
        ),
        "gamerules" => game.gamerules(client, None),
        "gamerules/update" => game.gamerules(client, Some(argument)),
        "serversettings/motd" => game.motd(client, None),
        "serversettings/motd/set" => game.motd(
            client,
            Some(
                argument
                    .as_str()
                    .ok_or_else(|| invalid_params("Not a string"))?,
            ),
        ),
        "serversettings/difficulty" => game.difficulty(client, None),
        "serversettings/difficulty/set" => game.difficulty(
            client,
            Some(
                argument
                    .as_str()
                    .ok_or_else(|| invalid_params("Not a string"))?,
            ),
        ),
        "serversettings/game_mode" => game.game_mode(client, None),
        "serversettings/game_mode/set" => game.game_mode(
            client,
            Some(
                argument
                    .as_str()
                    .ok_or_else(|| invalid_params("Not a string"))?,
            ),
        ),
        "serversettings/force_game_mode" => game.force_game_mode(client, None),
        "serversettings/force_game_mode/set" => game.force_game_mode(
            client,
            Some(
                argument
                    .as_bool()
                    .ok_or_else(|| invalid_params(format!("Not a boolean: {argument}")))?,
            ),
        ),
        "serversettings/max_players" => game.max_players(client, None),
        "serversettings/max_players/set" => game.max_players(
            client,
            Some(
                argument
                    .as_i64()
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| invalid_params(format!("Not a number: {argument}")))?,
            ),
        ),
        "serversettings/use_allowlist" => game.allowlist_setting(client, "use_allowlist", None),
        "serversettings/use_allowlist/set" => game.allowlist_setting(
            client,
            "use_allowlist",
            Some(
                argument
                    .as_bool()
                    .ok_or_else(|| invalid_params(format!("Not a boolean: {argument}")))?,
            ),
        ),
        "serversettings/enforce_allowlist" => {
            game.allowlist_setting(client, "enforce_allowlist", None)
        }
        "serversettings/enforce_allowlist/set" => game.allowlist_setting(
            client,
            "enforce_allowlist",
            Some(
                argument
                    .as_bool()
                    .ok_or_else(|| invalid_params(format!("Not a boolean: {argument}")))?,
            ),
        ),
        "allowlist" => Ok(lists.allowlist()),
        "bans" => Ok(lists.bans()),
        "ip_bans" => Ok(lists.ip_bans()),
        "operators" => Ok(lists.operators()),
        "allowlist/add" | "allowlist/remove" | "allowlist/set" => {
            edit(players(argument)?)?;
            Ok(lists.allowlist())
        }
        "allowlist/clear" => {
            edit(Vec::new())?;
            Ok(lists.allowlist())
        }
        "bans/add" | "bans/set" => {
            let bans = list(argument)?
                .iter()
                .map(user_ban)
                .collect::<Result<_, _>>()?;
            edit(bans)?;
            Ok(lists.bans())
        }
        "bans/remove" => {
            edit(players(argument)?)?;
            Ok(lists.bans())
        }
        "bans/clear" => {
            edit(Vec::new())?;
            Ok(lists.bans())
        }
        "ip_bans/add" | "ip_bans/remove" | "ip_bans/set" | "ip_bans/clear" => {
            let entries = list_or_none(argument)?;
            edit(match short {
                "ip_bans/add" => entries
                    .iter()
                    .map(incoming_ip_ban)
                    .collect::<Result<_, _>>()?,
                "ip_bans/remove" => entries.iter().map(ip).collect::<Result<_, _>>()?,
                "ip_bans/set" => entries.iter().map(ip_ban).collect::<Result<_, _>>()?,
                _ => Vec::new(),
            })?;
            Ok(lists.ip_bans())
        }
        "operators/add" | "operators/set" => {
            edit(
                list(argument)?
                    .iter()
                    .map(operator)
                    .collect::<Result<_, _>>()?,
            )?;
            Ok(lists.operators())
        }
        "operators/remove" => {
            edit(players(argument)?)?;
            Ok(lists.operators())
        }
        "operators/clear" => {
            edit(Vec::new())?;
            Ok(lists.operators())
        }
        _ => Err((
            INTERNAL_ERROR,
            Some(format!("minecraft:{short} isn't in Horizon yet")),
        )),
    }
}

/// `UUIDUtil` writes a UUID as 8-4-4-4-12 lowercase hex digits.
fn uuid(id: u128) -> String {
    let h = format!("{id:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

/// A UUID as `UUID.fromString` takes it (8-4-4-4-12 hex digits), or the 32 undashed digits of `UUIDUtil`'s
/// lenient codec; anything else (a short id, a sign, dashes anywhere) is no UUID.
fn parse_uuid(text: &str) -> Option<u128> {
    let digits = match text.len() {
        32 => text.to_owned(),
        36 => {
            let parts: Vec<&str> = text.split('-').collect();
            let lengths: Vec<usize> = parts.iter().map(|p| p.len()).collect();
            if lengths != [8, 4, 4, 4, 12] {
                return None;
            }
            parts.concat()
        }
        _ => return None,
    };
    digits
        .bytes()
        .all(|b| b.is_ascii_hexdigit())
        .then(|| u128::from_str_radix(&digits, 16).ok())
        .flatten()
}

/// A `PlayerDto` from a request: an id, a name, or both.
fn player_dto(value: &Value) -> Result<(Option<u128>, Option<String>), Failure> {
    let Some(object) = value.as_object() else {
        return Err(invalid_params("Not a player"));
    };
    let id = match object.get("id") {
        None => None,
        Some(id) => Some(
            id.as_str()
                .and_then(parse_uuid)
                .ok_or_else(|| invalid_params(format!("Invalid UUID: {id}")))?,
        ),
    };
    let name = match object.get("name") {
        None => None,
        Some(name) => Some(
            name.as_str()
                .ok_or_else(|| invalid_params("Not a string: name"))?
                .to_owned(),
        ),
    };
    Ok((id, name))
}

fn list(argument: &Value) -> Result<&Vec<Value>, Failure> {
    argument
        .as_array()
        .ok_or_else(|| invalid_params("Not a list"))
}

/// `PlayerDto`s, as the game takes them: the id in 32 hex digits.
fn players(argument: &Value) -> Result<Vec<Value>, Failure> {
    list(argument)?.iter().map(player).collect()
}

fn player(value: &Value) -> Result<Value, Failure> {
    let (id, name) = player_dto(value)?;
    Ok(json!({ "id": id.map(|id| format!("{id:032x}")), "name": name }))
}

/// The list a method takes, or none for a `clear`.
fn list_or_none(argument: &Value) -> Result<Vec<Value>, Failure> {
    match argument {
        Value::Null => Ok(Vec::new()),
        _ => list(argument).cloned(),
    }
}

/// An optional string field.
fn text(value: &Value, key: &str) -> Result<Option<String>, Failure> {
    match value.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_str()
            .map(|s| Some(s.to_owned()))
            .ok_or_else(|| invalid_params(format!("Not a string: {v}"))),
    }
}

/// What a ban DTO may give: the reason, the source and the end (an ISO-8601 instant, sent in milliseconds).
fn ban_terms(value: &Value) -> Result<Value, Failure> {
    let expires = match text(value, "expires")? {
        None => None,
        Some(at) => Some(
            chrono::DateTime::parse_from_rfc3339(&at)
                .map_err(|e| invalid_params(format!("Failed to parse instant {at}: {e}")))?
                .timestamp_millis(),
        ),
    };
    Ok(
        json!({ "reason": text(value, "reason")?, "source": text(value, "source")?, "expires": expires }),
    )
}

/// A `UserBanDto`: the player and the ban's terms.
fn user_ban(value: &Value) -> Result<Value, Failure> {
    let Some(dto) = value.get("player") else {
        return Err(invalid_params("No key player"));
    };
    let mut ban = ban_terms(value)?;
    ban["player"] = player(dto)?;
    Ok(ban)
}

/// An `IncomingIpBanDto`: an address or a player (whose address is banned if online), and the terms.
fn incoming_ip_ban(value: &Value) -> Result<Value, Failure> {
    let mut ban = ban_terms(value)?;
    ban["ip"] = json!(text(value, "ip")?);
    if let Some(dto) = value.get("player") {
        ban["player"] = player(dto)?;
    }
    Ok(ban)
}

/// An `IpBanDto`: the address and the terms.
fn ip_ban(value: &Value) -> Result<Value, Failure> {
    let mut ban = ban_terms(value)?;
    ban["ip"] = ip(value.get("ip").ok_or_else(|| invalid_params("No key ip"))?)?["ip"].take();
    Ok(ban)
}

/// An address to unban, as `ip_bans/remove` takes them.
fn ip(value: &Value) -> Result<Value, Failure> {
    let ip = value
        .as_str()
        .ok_or_else(|| invalid_params(format!("Not a string: {value}")))?;
    Ok(json!({ "ip": ip }))
}

/// An `OperatorDto`: the player, and the permission level (0 to 4) and bypass when given.
fn operator(value: &Value) -> Result<Value, Failure> {
    let Some(dto) = value.get("player") else {
        return Err(invalid_params("No key player"));
    };
    let level = match value.get("permissionLevel") {
        None => None,
        Some(level) => Some(
            level
                .as_i64()
                .filter(|l| (0..=4).contains(l))
                .ok_or_else(|| invalid_params(format!("Not a valid permission level: {level}")))?,
        ),
    };
    let bypass = match value.get("bypassesPlayerLimit") {
        None => None,
        Some(b) => Some(
            b.as_bool()
                .ok_or_else(|| invalid_params(format!("Not a boolean: {b}")))?,
        ),
    };
    Ok(json!({ "player": player(dto)?, "permissionLevel": level, "bypassesPlayerLimit": bypass }))
}

/// `Message.asComponent` as a JSON text component; `None` when it has neither text nor key.
fn component(message: &Value) -> Option<Value> {
    if let Some(key) = message["translatable"].as_str() {
        let mut c = json!({ "translate": key });
        if let Some(with) = message["translatableParams"].as_array() {
            c["with"] = Value::Array(with.clone());
        }
        return Some(c);
    }
    message["literal"]
        .as_str()
        .map(|text| json!({ "text": text }))
}

/// The running game, through its control endpoint.
struct Game<'a> {
    shared: &'a Shared,
}

impl Game<'_> {
    /// `MinecraftServerSettingsServiceImpl.isUsingWhitelist`, `setUsingWhitelist`,
    /// `isEnforceWhitelist` and `setEnforceWhitelist`.
    fn allowlist_setting(
        &self,
        client: u32,
        setting: &str,
        value: Option<bool>,
    ) -> Result<Value, Failure> {
        let mut request = json!({
            "method": format!("serversettings/{setting}")
        });
        if let Some(value) = value {
            request["method"] = json!(format!("serversettings/{setting}/set"));
            request["value"] = json!(value);
        }
        let reply = self.ask(&format!("management {client} {request}"))?;
        if let Some(error) = reply["error"].as_str() {
            return Err((INTERNAL_ERROR, Some(error.to_owned())));
        }
        if let Some(value) = value {
            let property = match setting {
                "use_allowlist" => "whitelist",
                "enforce_allowlist" => "enforce_whitelist",
                _ => unreachable!("allowlist setting is validated by its callers"),
            };
            crate::config::update(
                &self.shared.dir,
                "properties",
                property,
                toml::Value::Boolean(value),
            )
            .map_err(|e| (INTERNAL_ERROR, Some(e)))?;
        }
        Ok(reply["value"].clone())
    }

    /// `GameRulesService.get` and `update`: the game's rules as typed rules, or the one `update` set.
    fn gamerules(&self, client: u32, update: Option<&Value>) -> Result<Value, Failure> {
        let mut request = json!({ "method": "gamerules" });
        if let Some(update) = update {
            request["method"] = json!("gamerules/update");
            request["value"] = update.clone();
        }
        let reply = self.ask(&format!("management {client} {request}"))?;
        if let Some(invalid) = reply["invalid"].as_str() {
            return Err(invalid_params(invalid));
        }
        if let Some(error) = reply["error"].as_str() {
            return Err((INTERNAL_ERROR, Some(error.to_owned())));
        }
        Ok(reply["value"].clone())
    }

    /// `MinecraftServerSettingsServiceImpl.isAutoSave`: read the game's live save flag.
    /// `MinecraftServerSettingsServiceImpl.setAutoSave`: update that flag and return its new value.
    fn autosave(&self, client: u32, value: Option<bool>) -> Result<Value, Failure> {
        let mut request = json!({ "method": "serversettings/autosave" });
        if let Some(value) = value {
            request["method"] = json!("serversettings/autosave/set");
            request["value"] = json!(value);
        }
        let reply = self.ask(&format!("management {client} {request}"))?;
        if let Some(error) = reply["error"].as_str() {
            return Err((INTERNAL_ERROR, Some(error.to_owned())));
        }
        Ok(reply["value"].clone())
    }

    /// `MinecraftServerSettingsServiceImpl.forceGameMode` and `setForceGameMode`.
    fn force_game_mode(&self, client: u32, value: Option<bool>) -> Result<Value, Failure> {
        let mut request = json!({ "method": "serversettings/force_game_mode" });
        if let Some(value) = value {
            request["method"] = json!("serversettings/force_game_mode/set");
            request["value"] = json!(value);
        }
        let reply = self.ask(&format!("management {client} {request}"))?;
        if let Some(error) = reply["error"].as_str() {
            return Err((INTERNAL_ERROR, Some(error.to_owned())));
        }
        if let Some(value) = value {
            crate::config::update(
                &self.shared.dir,
                "properties",
                "force-gamemode",
                toml::Value::Boolean(value),
            )
            .map_err(|e| (INTERNAL_ERROR, Some(e)))?;
        }
        Ok(reply["value"].clone())
    }

    /// `MinecraftServerSettingsServiceImpl.getMaxPlayers` and `setMaxPlayers`.
    fn max_players(&self, client: u32, value: Option<usize>) -> Result<Value, Failure> {
        let mut request = json!({ "method": "serversettings/max_players" });
        if let Some(value) = value {
            request["method"] = json!("serversettings/max_players/set");
            request["value"] = json!(value);
        }
        let reply = self.ask(&format!("management {client} {request}"))?;
        if let Some(error) = reply["error"].as_str() {
            return Err((INTERNAL_ERROR, Some(error.to_owned())));
        }
        if let Some(value) = value {
            crate::config::update(
                &self.shared.dir,
                "server",
                "max_players",
                toml::Value::Integer(value as i64),
            )
            .map_err(|e| (INTERNAL_ERROR, Some(e)))?;
        }
        Ok(reply["value"].clone())
    }

    /// `MinecraftServerSettingsServiceImpl.getGameMode` and `setGameMode`.
    fn game_mode(&self, client: u32, value: Option<&str>) -> Result<Value, Failure> {
        let mut request = json!({ "method": "serversettings/game_mode" });
        if let Some(value) = value {
            request["method"] = json!("serversettings/game_mode/set");
            request["value"] = json!(value);
        }
        let reply = self.ask(&format!("management {client} {request}"))?;
        if let Some(error) = reply["error"].as_str() {
            return Err((INTERNAL_ERROR, Some(error.to_owned())));
        }
        if let Some(value) = value {
            crate::config::update(
                &self.shared.dir,
                "world",
                "game_mode",
                toml::Value::String(value.into()),
            )
            .map_err(|e| (INTERNAL_ERROR, Some(e)))?;
        }
        Ok(reply["value"].clone())
    }

    /// `MinecraftServerSettingsServiceImpl.getMotd` and `setMotd`: the live server-list message.
    fn motd(&self, client: u32, value: Option<&str>) -> Result<Value, Failure> {
        let mut request = json!({ "method": "serversettings/motd" });
        if let Some(value) = value {
            request["method"] = json!("serversettings/motd/set");
            request["value"] = json!(value);
        }
        let reply = self.ask(&format!("management {client} {request}"))?;
        if let Some(error) = reply["error"].as_str() {
            return Err((INTERNAL_ERROR, Some(error.to_owned())));
        }
        // server.toml's `motd` writes server.properties' at each start: keep the new one there too.
        if let Some(value) = value {
            crate::config::update(
                &self.shared.dir,
                "server",
                "motd",
                toml::Value::String(value.into()),
            )
            .map_err(|e| (INTERNAL_ERROR, Some(e)))?;
        }
        Ok(reply["value"].clone())
    }

    /// `MinecraftServerSettingsServiceImpl.getDifficulty` and `setDifficulty`.
    fn difficulty(&self, client: u32, value: Option<&str>) -> Result<Value, Failure> {
        let mut request = json!({ "method": "serversettings/difficulty" });
        if let Some(value) = value {
            request["method"] = json!("serversettings/difficulty/set");
            request["value"] = json!(value);
        }
        let reply = self.ask(&format!("management {client} {request}"))?;
        if let Some(error) = reply["error"].as_str() {
            return Err((INTERNAL_ERROR, Some(error.to_owned())));
        }
        if let Some(value) = value {
            crate::config::update(
                &self.shared.dir,
                "world",
                "difficulty",
                toml::Value::String(value.into()),
            )
            .map_err(|e| (INTERNAL_ERROR, Some(e)))?;
        }
        Ok(reply["value"].clone())
    }

    fn running(&self) -> bool {
        self.shared.state.lock().unwrap().phase == Phase::Running
    }

    fn ask(&self, line: &str) -> Result<Value, Failure> {
        engine::Control::connect(&self.shared.dir)
            .and_then(|mut game| game.ask(line))
            .map_err(|e| (INTERNAL_ERROR, Some(format!("the game didn't answer: {e}"))))
    }

    fn console(&self, line: &str) -> Result<Value, Failure> {
        self.ask(&format!("console {line}"))
    }

    /// A list edit, which the game makes as vanilla's services make it.
    fn edit(&self, client: u32, method: &str, entries: Vec<Value>) -> Result<(), Failure> {
        let request = json!({ "method": method, "entries": entries });
        let reply = self.ask(&format!("management {client} {request}"))?;
        match reply["error"].as_str() {
            Some(error) => Err((INTERNAL_ERROR, Some(error.to_owned()))),
            None => Ok(()),
        }
    }

    /// `PlayerService.get`: the players online, as `PlayerDto`s.
    fn online(&self) -> Result<Vec<Value>, Failure> {
        let status = self.ask("server-status")?;
        Ok(status["players"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| json!({ "id": p["uuid"], "name": p["name"] }))
            .collect())
    }

    /// An online player's name, by id first, as `PlayerService.getServerPlayer` finds them.
    fn find(&self, online: &[Value], dto: &Value) -> Result<Option<String>, Failure> {
        let (id, name) = player_dto(dto)?;
        let found = online.iter().find(|p| match (id, &name) {
            (Some(id), _) => p["id"] == uuid(id),
            (None, Some(name)) => p["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(name)),
            (None, None) => false,
        });
        Ok(found.and_then(|p| p["name"].as_str().map(str::to_owned)))
    }

    /// `ServerStateService.status`.
    fn status(&self) -> Value {
        let version = json!({
            "name": horizon_core::version::VERSION_ID,
            "protocol": horizon_core::version::PROTOCOL_VERSION,
        });
        let players = if self.running() {
            self.online().ok()
        } else {
            None
        };
        let Some(players) = players else {
            return json!({ "started": false, "version": version });
        };
        let mut state = json!({ "started": true, "version": version });
        // An empty list is left out, as vanilla's codec leaves out its default.
        if !players.is_empty() {
            state["players"] = Value::Array(players);
        }
        state
    }

    /// `PlayerService.kick`: the players found, as they were asked for.
    fn kick(&self, argument: &Value) -> Result<Value, Failure> {
        let kicks = argument
            .as_array()
            .ok_or_else(|| invalid_params("Not a list"))?;
        let online = self.online()?;
        let mut kicked = Vec::new();
        for kick in kicks {
            let Some(name) = self.find(&online, &kick["player"])? else {
                continue;
            };
            let reason = kick.get("message").and_then(component).map(|c| {
                c["text"]
                    .as_str()
                    .or(c["translate"].as_str())
                    .unwrap_or_default()
                    .to_owned()
            });
            let line = match reason {
                Some(reason) if !reason.is_empty() => format!("kick {name} {reason}"),
                _ => format!("kick {name}"),
            };
            self.console(&line)?;
            kicked.push(kick["player"].clone());
        }
        Ok(Value::Array(kicked))
    }

    /// `ServerStateService.systemMessage`: to everyone, or to the players named (those online).
    fn system_message(&self, argument: &Value) -> Result<Value, Failure> {
        if argument["overlay"].as_bool().is_none() {
            return Err(invalid_params("No key overlay"));
        }
        let Some(text) = component(&argument["message"]) else {
            return Ok(json!(false));
        };
        let targets = match argument.get("receivingPlayers").and_then(Value::as_array) {
            Some(players) if players.is_empty() => return Ok(json!(false)),
            Some(players) => {
                let online = self.online()?;
                let mut names = Vec::new();
                for p in players {
                    names.extend(self.find(&online, p)?);
                }
                names
            }
            None => vec!["@a".to_owned()],
        };
        for target in targets {
            if argument["overlay"] == true {
                self.console(&format!("title {target} actionbar {text}"))?;
            } else {
                self.console(&format!("tellraw {target} {text}"))?;
            }
        }
        Ok(json!(true))
    }
}

/// Vanilla's lists in the server folder, as the game keeps them.
struct Lists<'a>(&'a Path);

impl Lists<'_> {
    fn load<E: horizon_server_files::user_lists::Entry>(
        &self,
        file: &str,
    ) -> horizon_server_files::user_lists::UserList<E> {
        let mut list = horizon_server_files::user_lists::UserList::new(self.0.join(file));
        let _ = list.load();
        list
    }

    fn allowlist(&self) -> Value {
        let list: horizon_server_files::user_lists::WhiteList = self.load("whitelist.json");
        list.entries()
            .iter()
            .map(|e| json!({ "id": uuid(e.user.id), "name": e.user.name }))
            .collect()
    }

    fn operators(&self) -> Value {
        let list: horizon_server_files::user_lists::OpList = self.load("ops.json");
        list.entries()
            .iter()
            .map(|e| {
                json!({
                    "player": { "id": uuid(e.user.id), "name": e.user.name },
                    "permissionLevel": e.level,
                    "bypassesPlayerLimit": e.bypasses_player_limit,
                })
            })
            .collect()
    }

    fn bans(&self) -> Value {
        let list: horizon_server_files::user_lists::BanList = self.load("banned-players.json");
        list.entries()
            .iter()
            .map(|b| {
                let player = json!({ "id": uuid(b.user.id), "name": b.user.name });
                ban(("player", player), &b.reason, &b.source, b.expires)
            })
            .collect()
    }

    fn ip_bans(&self) -> Value {
        let list: horizon_server_files::user_lists::IpBanList = self.load("banned-ips.json");
        list.entries()
            .iter()
            .map(|b| ban(("ip", json!(b.user)), &b.reason, &b.source, b.expires))
            .collect()
    }
}

/// `ServerStateService.status`, as `server/status` answers it (and the heartbeat carries it).
pub fn status(shared: &Shared) -> Value {
    Game { shared }.status()
}

/// The notification a change of one of the lists is, from the game's typed event (`operators_added`,
/// `bans_removed`...): `JsonRpcNotificationService`'s `playerOped`/`playerDeoped` (an `OperatorDto`),
/// `playerAddedToAllowlist`/`playerRemovedFromAllowlist` and `playerUnbanned` (a `PlayerDto`), `playerBanned`
/// (a `UserBanDto`), `ipBanned` (an `IpBanDto`) and `ipUnbanned` (the address).
pub fn list_notification(kind: &str, event: &Value) -> Option<(&'static str, Value)> {
    let player = || json!({ "id": event["uuid"], "name": event["name"] });
    let operator = || {
        json!({
            "player": player(),
            "permissionLevel": event["level"],
            "bypassesPlayerLimit": event["bypassesPlayerLimit"],
        })
    };
    let reason = event["reason"].as_str().map(str::to_owned);
    let source = event["source"].as_str().unwrap_or_default();
    let expires = event["expires"].as_i64();
    Some(match kind {
        "operators_added" => ("notification/operators/added", operator()),
        "operators_removed" => ("notification/operators/removed", operator()),
        "allowlist_added" => ("notification/allowlist/added", player()),
        "allowlist_removed" => ("notification/allowlist/removed", player()),
        "bans_added" => (
            "notification/bans/added",
            ban(("player", player()), &reason, source, expires),
        ),
        // `MinecraftServer.onGameRuleChanged`: the rule as a typed rule (the event's own `type` is its kind).
        "gamerule" => (
            "notification/gamerules/updated",
            json!({ "key": event["key"], "type": event["valueType"], "value": event["value"] }),
        ),
        "bans_removed" => ("notification/bans/removed", player()),
        "ip_bans_added" => (
            "notification/ip_bans/added",
            ban(("ip", event["ip"].clone()), &reason, source, expires),
        ),
        "ip_bans_removed" => ("notification/ip_bans/removed", event["ip"].clone()),
        _ => return None,
    })
}

/// A `UserBanDto` or `IpBanDto`: the reason when there is one, the end as an ISO-8601 instant.
fn ban(who: (&str, Value), reason: &Option<String>, source: &str, expires: Option<i64>) -> Value {
    let mut b = json!({ "source": source });
    b[who.0] = who.1;
    if let Some(reason) = reason {
        b["reason"] = json!(reason);
    }
    if let Some(at) = expires.and_then(chrono::DateTime::from_timestamp_millis) {
        b["expires"] = json!(at.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true));
    }
    b
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_player_id_must_be_a_whole_uuid() {
        use super::player_dto;
        let id = |s: &str| player_dto(&serde_json::json!({ "id": s })).map(|(id, _)| id);
        let want = Some(0x0123_4567_89ab_cdef_0123_4567_89ab_cdefu128);
        assert_eq!(id("01234567-89ab-cdef-0123-456789abcdef").unwrap(), want);
        assert_eq!(id("0123456789abcdef0123456789abcdef").unwrap(), want);
        for bad in [
            "abc",
            "1",
            "+123456789abcdef0123456789abcdef",
            "0123-4567-89ab-cdef-0123456789abcdef",
            "01234567-89ab-cdef-0123-456789abcdeg",
            "",
        ] {
            assert!(id(bad).is_err(), "{bad:?} is no UUID");
        }
    }

    use super::*;

    fn shared() -> std::sync::Arc<Shared> {
        let dir = std::env::temp_dir().join(format!(
            "horizon-management-methods-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Shared::with_install(&dir, None)
    }

    #[test]
    fn a_change_of_a_list_is_the_notification_vanilla_sends() {
        let id = "93e029a5-04a0-349d-90b2-f7e145b2644f";
        let event = |fields: Value| {
            let mut event = json!({ "name": "Kim0", "uuid": id });
            event
                .as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            event
        };
        let player = json!({ "id": id, "name": "Kim0" });
        let note = |kind, fields| list_notification(kind, &event(fields)).unwrap();
        assert_eq!(
            note(
                "operators_added",
                json!({ "level": 3, "bypassesPlayerLimit": true })
            ),
            (
                "notification/operators/added",
                json!({ "player": player, "permissionLevel": 3, "bypassesPlayerLimit": true })
            )
        );
        assert_eq!(
            note(
                "operators_removed",
                json!({ "level": 4, "bypassesPlayerLimit": false })
            )
            .0,
            "notification/operators/removed"
        );
        assert_eq!(
            note("allowlist_added", json!({})),
            ("notification/allowlist/added", player.clone())
        );
        assert_eq!(
            note("bans_removed", json!({})),
            ("notification/bans/removed", player.clone())
        );
        // A ban says why and until when, as the lists do; no reason or end leaves them out.
        assert_eq!(
            note(
                "bans_added",
                json!({ "reason": "griefing", "source": "Kim", "expires": 1_793_491_200_000_i64 })
            ),
            (
                "notification/bans/added",
                json!({ "player": player, "source": "Kim", "reason": "griefing", "expires": "2026-11-01T00:00:00Z" })
            )
        );
        assert_eq!(
            note(
                "ip_bans_added",
                json!({ "ip": "10.0.0.1", "source": "Rcon", "reason": null, "expires": null })
            ),
            (
                "notification/ip_bans/added",
                json!({ "ip": "10.0.0.1", "source": "Rcon" })
            )
        );
        assert_eq!(
            note("ip_bans_removed", json!({ "ip": "10.0.0.1" })),
            ("notification/ip_bans/removed", json!("10.0.0.1"))
        );
        // A game rule changed: the rule as a typed rule, whose `type` the event keeps as `valueType`.
        assert_eq!(
            note(
                "gamerule",
                json!({ "key": "minecraft:pvp", "valueType": "boolean", "value": false })
            ),
            (
                "notification/gamerules/updated",
                json!({ "key": "minecraft:pvp", "type": "boolean", "value": false })
            )
        );
        assert!(list_notification("kick", &event(json!({}))).is_none());
    }

    #[test]
    fn the_lists_read_as_vanillas_dtos() {
        let shared = shared();
        let dir = &shared.dir;
        std::fs::write(
            dir.join("banned-players.json"),
            r#"[{"uuid":"93e029a5-04a0-349d-90b2-f7e145b2644f","name":"Kim0","created":"2026-10-31 14:00:00 +0000",
                "source":"Management server","expires":"2026-11-01 00:00:00 +0000","reason":"griefing"}]"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("ops.json"),
            r#"[{"uuid":"93e029a5-04a0-349d-90b2-f7e145b2644f","name":"Kim0","level":3,"bypassesPlayerLimit":true}]"#,
        )
        .unwrap();
        let lists = Lists(dir);
        assert_eq!(
            lists.bans(),
            json!([{ "player": { "id": "93e029a5-04a0-349d-90b2-f7e145b2644f", "name": "Kim0" },
                     "source": "Management server", "reason": "griefing", "expires": "2026-11-01T00:00:00Z" }])
        );
        assert_eq!(lists.operators()[0]["permissionLevel"], 3);
        assert_eq!(lists.operators()[0]["bypassesPlayerLimit"], true);
        assert_eq!(lists.allowlist(), json!([]));
        assert_eq!(lists.ip_bans(), json!([]));
    }

    #[test]
    fn list_edits_reach_the_game_checked_and_normalized() {
        let kim = json!({ "id": "93e029a5-04a0-349d-90b2-f7e145b2644f" });
        assert_eq!(
            user_ban(&json!({ "player": kim, "expires": "2027-01-01T00:00:00Z" })).unwrap(),
            json!({ "player": { "id": "93e029a504a0349d90b2f7e145b2644f", "name": null },
                    "reason": null, "source": null, "expires": 1798761600000i64 })
        );
        assert!(user_ban(&json!({ "reason": "x" })).is_err());
        assert!(user_ban(&json!({ "player": kim, "expires": "tomorrow" })).is_err());
        assert_eq!(
            ip_ban(&json!({ "ip": "203.0.113.7", "source": "Bot" })).unwrap(),
            json!({ "ip": "203.0.113.7", "reason": null, "source": "Bot", "expires": null })
        );
        assert!(ip_ban(&json!({ "reason": "x" })).is_err());
        assert_eq!(
            incoming_ip_ban(&json!({ "player": { "name": "Kim0" } })).unwrap()["player"]["name"],
            "Kim0"
        );
        assert_eq!(
            operator(&json!({ "player": { "name": "Kim0" }, "permissionLevel": 2 })).unwrap(),
            json!({ "player": { "id": null, "name": "Kim0" }, "permissionLevel": 2, "bypassesPlayerLimit": null })
        );
        assert!(operator(&json!({ "player": { "name": "Kim0" }, "permissionLevel": 5 })).is_err());
        assert!(
            operator(&json!({ "player": { "name": "Kim0" }, "bypassesPlayerLimit": 1 })).is_err()
        );
        assert_eq!(list_or_none(&Value::Null).unwrap(), Vec::<Value>::new());
    }

    #[test]
    fn the_status_says_when_the_game_isnt_up() {
        let shared = shared();
        let reply = crate::management::receive(
            &shared,
            1,
            r#"{"jsonrpc":"2.0","id":null,"method":"server/status"}"#,
        );
        assert_eq!(
            reply.unwrap().unwrap(),
            json!({ "jsonrpc": "2.0", "id": null, "result": {
                "started": false, "version": { "name": "26.3", "protocol": 777 } } })
        );
    }
}
