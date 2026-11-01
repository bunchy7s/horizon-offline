//! What vanilla's management protocol offers, as `rpc.discover` describes it (OpenRPC 1.3.2): every method
//! vanilla 26.3 registers (`IncomingRpcMethods`, `OutgoingRpcMethods`) with its parameter and result, and the
//! schemas they name (`api/Schema`). `tools/test_management_protocol_parity.py` compares the document with
//! the one vanilla's data generator writes (`reports/json-rpc-api-schema.json`).

use serde_json::{Value, json};

/// A method: its name under `minecraft:`, its description, its parameter and its result (none for a
/// notification), each a name and a schema. A schema is `boolean`, `integer`, `number` or `string`,
/// `#name` for one of [`components`], or `[schema]` for an array of them.
pub type Method = (
    &'static str,
    &'static str,
    Option<(&'static str, &'static str)>,
    Option<(&'static str, &'static str)>,
);

/// In vanilla's order: what clients call, then the notifications the server sends.
#[rustfmt::skip]
pub const METHODS: &[Method] = &[
    ("allowlist", "Get the allowlist", None, Some(("allowlist", "[#player]"))),
    ("allowlist/set", "Set the allowlist", Some(("players", "[#player]")), Some(("allowlist", "[#player]"))),
    ("allowlist/add", "Add players to allowlist", Some(("add", "[#player]")), Some(("allowlist", "[#player]"))),
    ("allowlist/remove", "Remove players from allowlist", Some(("remove", "[#player]")), Some(("allowlist", "[#player]"))),
    ("allowlist/clear", "Clear all players in allowlist", None, Some(("allowlist", "[#player]"))),
    ("bans", "Get the ban list", None, Some(("banlist", "[#user_ban]"))),
    ("bans/set", "Set the banlist", Some(("bans", "[#user_ban]")), Some(("banlist", "[#user_ban]"))),
    ("bans/add", "Add players to ban list", Some(("add", "[#user_ban]")), Some(("banlist", "[#user_ban]"))),
    ("bans/remove", "Remove players from ban list", Some(("remove", "[#player]")), Some(("banlist", "[#user_ban]"))),
    ("bans/clear", "Clear all players in ban list", None, Some(("banlist", "[#user_ban]"))),
    ("ip_bans", "Get the ip ban list", None, Some(("banlist", "[#ip_ban]"))),
    ("ip_bans/set", "Set the ip banlist", Some(("banlist", "[#ip_ban]")), Some(("banlist", "[#ip_ban]"))),
    ("ip_bans/add", "Add ip to ban list", Some(("add", "[#incoming_ip_ban]")), Some(("banlist", "[#ip_ban]"))),
    ("ip_bans/remove", "Remove ip from ban list", Some(("ip", "[string]")), Some(("banlist", "[#ip_ban]"))),
    ("ip_bans/clear", "Clear all ips in ban list", None, Some(("banlist", "[#ip_ban]"))),
    ("players", "Get all connected players", None, Some(("players", "[#player]"))),
    ("players/kick", "Kick players", Some(("kick", "[#kick_player]")), Some(("kicked", "[#player]"))),
    ("operators", "Get all oped players", None, Some(("operators", "[#operator]"))),
    ("operators/set", "Set all oped players", Some(("operators", "[#operator]")), Some(("operators", "[#operator]"))),
    ("operators/add", "Op players", Some(("add", "[#operator]")), Some(("operators", "[#operator]"))),
    ("operators/remove", "Deop players", Some(("remove", "[#player]")), Some(("operators", "[#operator]"))),
    ("operators/clear", "Deop all players", None, Some(("operators", "[#operator]"))),
    ("server/status", "Get server status", None, Some(("status", "#server_state"))),
    ("server/save", "Save server state", Some(("flush", "boolean")), Some(("saving", "boolean"))),
    ("server/stop", "Stop server", None, Some(("stopping", "boolean"))),
    ("server/system_message", "Send a system message", Some(("message", "#system_message")), Some(("sent", "boolean"))),
    ("serversettings/autosave", "Get whether automatic world saving is enabled on the server", None, Some(("enabled", "boolean"))),
    ("serversettings/autosave/set", "Enable or disable automatic world saving on the server", Some(("enable", "boolean")), Some(("enabled", "boolean"))),
    ("serversettings/difficulty", "Get the current difficulty level of the server", None, Some(("difficulty", "#difficulty"))),
    ("serversettings/difficulty/set", "Set the difficulty level of the server", Some(("difficulty", "#difficulty")), Some(("difficulty", "#difficulty"))),
    ("serversettings/enforce_allowlist", "Get whether allowlist enforcement is enabled (kicks players immediately when removed from allowlist)", None, Some(("enforced", "boolean"))),
    ("serversettings/enforce_allowlist/set", "Enable or disable allowlist enforcement (when enabled, players are kicked immediately upon removal from allowlist)", Some(("enforce", "boolean")), Some(("enforced", "boolean"))),
    ("serversettings/use_allowlist", "Get whether the allowlist is enabled on the server", None, Some(("used", "boolean"))),
    ("serversettings/use_allowlist/set", "Enable or disable the allowlist on the server (controls whether only allowlisted players can join)", Some(("use", "boolean")), Some(("used", "boolean"))),
    ("serversettings/max_players", "Get the maximum number of players allowed to connect to the server", None, Some(("max", "integer"))),
    ("serversettings/max_players/set", "Set the maximum number of players allowed to connect to the server", Some(("max", "integer")), Some(("max", "integer"))),
    ("serversettings/pause_when_empty_seconds", "Get the number of seconds before the game is automatically paused when no players are online", None, Some(("seconds", "integer"))),
    ("serversettings/pause_when_empty_seconds/set", "Set the number of seconds before the game is automatically paused when no players are online", Some(("seconds", "integer")), Some(("seconds", "integer"))),
    ("serversettings/player_idle_timeout", "Get the number of seconds before idle players are automatically kicked from the server", None, Some(("seconds", "integer"))),
    ("serversettings/player_idle_timeout/set", "Set the number of seconds before idle players are automatically kicked from the server", Some(("seconds", "integer")), Some(("seconds", "integer"))),
    ("serversettings/allow_flight", "Get whether flight is allowed for players in Survival mode", None, Some(("allowed", "boolean"))),
    ("serversettings/allow_flight/set", "Allow or disallow flight for players in Survival mode", Some(("allow", "boolean")), Some(("allowed", "boolean"))),
    ("serversettings/motd", "Get the server's message of the day displayed to players", None, Some(("message", "string"))),
    ("serversettings/motd/set", "Set the server's message of the day displayed to players", Some(("message", "string")), Some(("message", "string"))),
    ("serversettings/spawn_protection_radius", "Get the spawn protection radius in blocks (only operators can edit within this area)", None, Some(("radius", "integer"))),
    ("serversettings/spawn_protection_radius/set", "Set the spawn protection radius in blocks (only operators can edit within this area)", Some(("radius", "integer")), Some(("radius", "integer"))),
    ("serversettings/force_game_mode", "Get whether players are forced to use the server's default game mode", None, Some(("forced", "boolean"))),
    ("serversettings/force_game_mode/set", "Enable or disable forcing players to use the server's default game mode", Some(("force", "boolean")), Some(("forced", "boolean"))),
    ("serversettings/game_mode", "Get the server's default game mode", None, Some(("mode", "#game_type"))),
    ("serversettings/game_mode/set", "Set the server's default game mode", Some(("mode", "#game_type")), Some(("mode", "#game_type"))),
    ("serversettings/view_distance", "Get the server's view distance in chunks", None, Some(("distance", "integer"))),
    ("serversettings/view_distance/set", "Set the server's view distance in chunks", Some(("distance", "integer")), Some(("distance", "integer"))),
    ("serversettings/simulation_distance", "Get the server's simulation distance in chunks", None, Some(("distance", "integer"))),
    ("serversettings/simulation_distance/set", "Set the server's simulation distance in chunks", Some(("distance", "integer")), Some(("distance", "integer"))),
    ("serversettings/accept_transfers", "Get whether the server accepts player transfers from other servers", None, Some(("accepted", "boolean"))),
    ("serversettings/accept_transfers/set", "Enable or disable accepting player transfers from other servers", Some(("accept", "boolean")), Some(("accepted", "boolean"))),
    ("serversettings/status_heartbeat_interval", "Get the interval in seconds between server status heartbeats", None, Some(("seconds", "integer"))),
    ("serversettings/status_heartbeat_interval/set", "Set the interval in seconds between server status heartbeats", Some(("seconds", "integer")), Some(("seconds", "integer"))),
    ("serversettings/operator_user_permission_level", "Get default operator permission level", None, Some(("level", "integer"))),
    ("serversettings/operator_user_permission_level/set", "Set default operator permission level", Some(("level", "integer")), Some(("level", "integer"))),
    ("serversettings/hide_online_players", "Get whether the server hides online player information from status queries", None, Some(("hidden", "boolean"))),
    ("serversettings/hide_online_players/set", "Enable or disable hiding online player information from status queries", Some(("hide", "boolean")), Some(("hidden", "boolean"))),
    ("serversettings/status_replies", "Get whether the server responds to connection status requests", None, Some(("enabled", "boolean"))),
    ("serversettings/status_replies/set", "Enable or disable the server responding to connection status requests", Some(("enable", "boolean")), Some(("enabled", "boolean"))),
    ("serversettings/entity_broadcast_range", "Get the entity broadcast range as a percentage", None, Some(("percentage_points", "integer"))),
    ("serversettings/entity_broadcast_range/set", "Set the entity broadcast range as a percentage", Some(("percentage_points", "integer")), Some(("percentage_points", "integer"))),
    ("gamerules", "Get the available game rule keys and their current values", None, Some(("gamerules", "[#typed_game_rule]"))),
    ("gamerules/update", "Update game rule value", Some(("gamerule", "#untyped_game_rule")), Some(("gamerule", "#typed_game_rule"))),
    ("notification/server/started", "Server started", None, None),
    ("notification/server/stopping", "Server shutting down", None, None),
    ("notification/server/saving", "Server save started", None, None),
    ("notification/server/saved", "Server save completed", None, None),
    ("notification/server/activity", "Server activity occurred. Rate limited to 1 notification per 30 seconds", None, None),
    ("notification/world/upgrade_started", "World upgrade started", None, None),
    ("notification/world/upgrade_progress", "World upgrade progress. Rate limited to 1 notification per second", Some(("progress", "number")), None),
    ("notification/world/upgrade_finished", "World upgrade finished", None, None),
    ("notification/world/upgrade_failed", "World upgrade failed", Some(("reason", "string")), None),
    ("notification/players/joined", "Player joined", Some(("player", "#player")), None),
    ("notification/players/left", "Player left", Some(("player", "#player")), None),
    ("notification/operators/added", "Player was oped", Some(("player", "#operator")), None),
    ("notification/operators/removed", "Player was deoped", Some(("player", "#operator")), None),
    ("notification/allowlist/added", "Player was added to allowlist", Some(("player", "#player")), None),
    ("notification/allowlist/removed", "Player was removed from allowlist", Some(("player", "#player")), None),
    ("notification/ip_bans/added", "Ip was added to ip ban list", Some(("player", "#ip_ban")), None),
    ("notification/ip_bans/removed", "Ip was removed from ip ban list", Some(("player", "string")), None),
    ("notification/bans/added", "Player was added to ban list", Some(("player", "#user_ban")), None),
    ("notification/bans/removed", "Player was removed from ban list", Some(("player", "#player")), None),
    ("notification/gamerules/updated", "Gamerule was changed", Some(("gamerule", "#typed_game_rule")), None),
    ("notification/server/status", "Server status heartbeat, including before the server has spun up", Some(("status", "#server_state")), None),
];

fn schema(spec: &str) -> Value {
    if let Some(inner) = spec.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        json!({ "type": "array", "items": schema(inner) })
    } else if let Some(name) = spec.strip_prefix('#') {
        json!({ "$ref": format!("#/components/schemas/{name}") })
    } else {
        json!({ "type": spec })
    }
}

/// The schemas methods refer to by `#name`.
fn components() -> Value {
    let string = || json!({ "type": "string" });
    let player = || schema("#player");
    let ban = |extra: (&str, Value)| {
        let mut b = json!({ "expires": string(), "reason": string(), "source": string() });
        b[extra.0] = extra.1;
        b
    };
    let both = json!(["boolean", "integer"]);
    json!({
        "difficulty": { "type": "string", "enum": ["peaceful", "easy", "normal", "hard"] },
        "game_type": { "type": "string", "enum": ["survival", "creative", "adventure", "spectator"] },
        "incoming_ip_ban": { "type": "object", "properties": {
            "expires": string(), "ip": string(), "player": player(), "reason": string(), "source": string(),
        } },
        "ip_ban": { "type": "object", "properties": ban(("ip", string())) },
        "kick_player": { "type": "object", "properties": {
            "message": schema("#message"), "player": player(),
        } },
        "message": { "type": "object", "properties": {
            "literal": string(), "translatable": string(), "translatableParams": schema("[string]"),
        } },
        "operator": { "type": "object", "properties": {
            "bypassesPlayerLimit": schema("boolean"), "permissionLevel": schema("integer"), "player": player(),
        } },
        "player": { "type": "object", "properties": { "id": string(), "name": string() } },
        "server_state": { "type": "object", "properties": {
            "players": schema("[#player]"), "started": schema("boolean"), "version": schema("#version"),
        } },
        "system_message": { "type": "object", "properties": {
            "message": schema("#message"), "overlay": schema("boolean"), "receivingPlayers": schema("[#player]"),
        } },
        "typed_game_rule": { "type": "object", "properties": {
            "type": { "type": "string", "enum": ["integer", "boolean"] }, "key": string(), "value": { "type": both },
        } },
        "untyped_game_rule": { "type": "object", "properties": { "key": string(), "value": { "type": both } } },
        "user_ban": { "type": "object", "properties": ban(("player", player())) },
        "version": { "type": "object", "properties": { "name": string(), "protocol": schema("integer") } },
    })
}

/// `rpc.discover`'s result.
pub fn discover() -> Value {
    let methods: Vec<Value> = METHODS
        .iter()
        .map(|(name, description, param, result)| {
            let mut m = json!({
                "name": format!("minecraft:{name}"),
                "description": description,
                "params": param.map_or_else(Vec::new, |(name, spec)| {
                    vec![json!({ "name": name, "required": true, "schema": schema(spec) })]
                }),
            });
            if let Some((name, spec)) = result {
                m["result"] = json!({ "name": name, "schema": schema(spec) });
            }
            m
        })
        .collect();
    json!({
        "openrpc": "1.3.2",
        "info": { "title": "Minecraft Server JSON-RPC", "version": "3.1.0" },
        "methods": methods,
        "components": { "schemas": components() },
    })
}

/// A method clients may call (not a notification), by its full name.
pub fn incoming(name: &str) -> Option<&'static Method> {
    let short = name.strip_prefix("minecraft:").unwrap_or(name);
    METHODS
        .iter()
        .find(|m| m.0 == short && !m.0.starts_with("notification/"))
}
