//! `horizon-server mcp` (docs/server.md, section 8): the Model Context Protocol over stdin and stdout
//! (JSON-RPC 2.0, one message per line, as `horizon-ctl mcp`), so an AI agent can run and moderate a server.
//! Each tool is one control API call on the server running in the folder, with the token it reads itself;
//! every action shows in the server's log as any other API call does.

use crate::api::Client;
use serde_json::{Map, Value, json};
use std::{
    io::{BufRead, Write},
    path::Path,
};

const PROTOCOL: &str = "2025-06-18";

/// One tool: its name, what it does, its arguments (name, JSON type, what it is, required), and whether it
/// destroys or sends someone away.
struct Tool {
    name: &'static str,
    what: &'static str,
    args: &'static [(&'static str, &'static str, &'static str, bool)],
    destructive: bool,
}

const NAME: (&str, &str, &str, bool) = ("name", "string", "The player's name", true);
const REASON: (&str, &str, &str, bool) = ("reason", "string", "Why (shown to the player)", false);
const CONFIRM: (&str, &str, &str, bool) = ("confirm", "boolean", "Must be true", true);

const TOOLS: &[Tool] = &[
    Tool {
        name: "status",
        what: "How the server does: phase, uptime, crashes, TPS, MSPT, players, RAM, CPU, updates",
        args: &[],
        destructive: false,
    },
    Tool {
        name: "players",
        what: "Who is online (address, ping, op level), the operators, the whitelist and the bans",
        args: &[],
        destructive: false,
    },
    Tool {
        name: "run_command",
        what: "Run a console command (as vanilla's server console) and return what it logged",
        args: &[("command", "string", "One command, without the slash", true)],
        destructive: true,
    },
    Tool {
        name: "logs",
        what: "The log lines after the since-th (the last 2000 are kept); returns next to ask from",
        args: &[(
            "since",
            "integer",
            "Line number to read after (0: all kept)",
            false,
        )],
        destructive: false,
    },
    Tool {
        name: "events",
        what: "What happened after event number since: joins, leaves, kicks, chat, deaths, log lines, phase changes; returns next",
        args: &[
            (
                "since",
                "integer",
                "Event number to read after (0: all kept)",
                false,
            ),
            (
                "types",
                "string",
                "Comma-separated types, e.g. chat,death (default: all)",
                false,
            ),
        ],
        destructive: false,
    },
    Tool {
        name: "kick",
        what: "Kick an online player",
        args: &[NAME, REASON],
        destructive: true,
    },
    Tool {
        name: "ban",
        what: "Ban a player (kicked if online)",
        args: &[NAME, REASON],
        destructive: true,
    },
    Tool {
        name: "pardon",
        what: "Lift a player's ban",
        args: &[NAME],
        destructive: false,
    },
    Tool {
        name: "ban_ip",
        what: "Ban an address, or the address an online player plays from (its players are kicked)",
        args: &[
            ("ip", "string", "An IPv4 or IPv6 address", false),
            ("name", "string", "Or an online player's name", false),
            REASON,
        ],
        destructive: true,
    },
    Tool {
        name: "pardon_ip",
        what: "Lift an address's ban",
        args: &[("ip", "string", "The address", true)],
        destructive: false,
    },
    Tool {
        name: "op",
        what: "Make a player an operator",
        args: &[NAME],
        destructive: false,
    },
    Tool {
        name: "deop",
        what: "Make a player no longer an operator",
        args: &[NAME],
        destructive: true,
    },
    Tool {
        name: "whitelist",
        what: "Change the whitelist: add or remove a player, or turn it on or off",
        args: &[
            ("action", "string", "add, remove, on or off", true),
            ("name", "string", "The player, for add and remove", false),
        ],
        destructive: false,
    },
    Tool {
        name: "list_backups",
        what: "The backups, newest first",
        args: &[],
        destructive: false,
    },
    Tool {
        name: "backup",
        what: "Back up the world and settings now (a running server saves first and stays up)",
        args: &[("name", "string", "A label: letters, digits, - and _", false)],
        destructive: false,
    },
    Tool {
        name: "delete_backup",
        what: "Delete a backup",
        args: &[
            (
                "name",
                "string",
                "Its file name, as list_backups gives it",
                true,
            ),
            CONFIRM,
        ],
        destructive: true,
    },
    Tool {
        name: "restore",
        what: "Restart the server with a backup's world (the current one is kept in backups/)",
        args: &[
            (
                "name",
                "string",
                "The backup: file name, unique prefix or label",
                true,
            ),
            CONFIRM,
        ],
        destructive: true,
    },
    Tool {
        name: "get_config",
        what: "The settings in server.toml",
        args: &[],
        destructive: false,
    },
    Tool {
        name: "set_config",
        what: "Change settings, e.g. {\"world.difficulty\": \"hard\"}; says which apply now and which at the next start",
        args: &[("changes", "object", "section.name to value", true)],
        destructive: false,
    },
    Tool {
        name: "start",
        what: "Start the server in the background and wait until the game is up",
        args: &[],
        destructive: false,
    },
    Tool {
        name: "restart",
        what: "Restart the game (players are disconnected for a moment)",
        args: &[],
        destructive: true,
    },
    Tool {
        name: "stop",
        what: "Stop the game and horizon-server",
        args: &[CONFIRM],
        destructive: true,
    },
];

fn tools() -> Vec<Value> {
    TOOLS
        .iter()
        .map(|t| {
            let properties: Map<String, Value> = t
                .args
                .iter()
                .map(|(name, kind, what, _)| ((*name).to_owned(), json!({ "type": kind, "description": what })))
                .collect();
            let required: Vec<&str> = t.args.iter().filter(|a| a.3).map(|a| a.0).collect();
            json!({
                "name": t.name,
                "description": t.what,
                "inputSchema": { "type": "object", "properties": properties, "required": required },
                "annotations": {
                    "destructiveHint": t.destructive,
                    "readOnlyHint": matches!(t.name, "status" | "players" | "logs" | "events" | "list_backups" | "get_config"),
                },
            })
        })
        .collect()
}

/// The control API call a tool makes: method, path and body.
fn request(name: &str, args: &Value) -> Result<(&'static str, String, Option<Value>), String> {
    let text = |key: &str| args[key].as_str().map(str::to_owned);
    let need = |key: &str| text(key).ok_or_else(|| format!("{name} needs {key}"));
    let confirmed = || {
        (args["confirm"] == true)
            .then_some(())
            .ok_or_else(|| format!("{name} needs confirm: true"))
    };
    let some = |v: Value| -> Value {
        Value::Object(
            v.as_object()
                .into_iter()
                .flatten()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        )
    };
    let since = args["since"].as_u64().unwrap_or(0);
    Ok(match name {
        "status" => ("GET", "/api/v1/status".into(), None),
        "players" => ("GET", "/api/v1/players".into(), None),
        "run_command" => (
            "POST",
            "/api/v1/command".into(),
            Some(json!({ "line": need("command")? })),
        ),
        "logs" => ("GET", format!("/api/v1/logs?since={since}"), None),
        "events" => {
            let types = text("types").unwrap_or_default();
            if !types.chars().all(|c| c.is_ascii_lowercase() || c == ',') {
                return Err("types are words like chat,death, separated by commas".into());
            }
            (
                "GET",
                format!("/api/v1/events?since={since}&types={types}"),
                None,
            )
        }
        "kick" | "ban" => (
            "POST",
            format!("/api/v1/players/{name}"),
            Some(some(
                json!({ "name": need("name")?, "reason": args["reason"] }),
            )),
        ),
        "pardon" | "op" | "deop" => (
            "POST",
            format!("/api/v1/players/{name}"),
            Some(json!({ "name": need("name")? })),
        ),
        "ban_ip" => (
            "POST",
            "/api/v1/players/ban-ip".into(),
            Some(some(
                json!({ "ip": args["ip"], "name": args["name"], "reason": args["reason"] }),
            )),
        ),
        "pardon_ip" => (
            "POST",
            "/api/v1/players/pardon-ip".into(),
            Some(json!({ "ip": need("ip")? })),
        ),
        "whitelist" => match need("action")?.as_str() {
            action @ ("add" | "remove") => (
                "POST",
                format!("/api/v1/whitelist/{action}"),
                Some(json!({ "name": need("name")? })),
            ),
            "on" => (
                "POST",
                "/api/v1/whitelist".into(),
                Some(json!({ "enabled": true })),
            ),
            "off" => (
                "POST",
                "/api/v1/whitelist".into(),
                Some(json!({ "enabled": false })),
            ),
            other => {
                return Err(format!(
                    "whitelist's action is add, remove, on or off, not {other:?}"
                ));
            }
        },
        "list_backups" => ("GET", "/api/v1/backups".into(), None),
        "backup" => (
            "POST",
            "/api/v1/backups".into(),
            Some(some(json!({ "name": args["name"] }))),
        ),
        "delete_backup" => {
            confirmed()?;
            let file = need("name")?;
            if !file
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            {
                return Err("a backup's file name has letters, digits, -, _ and . only".into());
            }
            ("DELETE", format!("/api/v1/backups?name={file}"), None)
        }
        "restore" => {
            confirmed()?;
            (
                "POST",
                "/api/v1/restore".into(),
                Some(json!({ "name": need("name")? })),
            )
        }
        "get_config" => ("GET", "/api/v1/config".into(), None),
        "set_config" => (
            "PATCH",
            "/api/v1/config".into(),
            Some(json!({ "changes": args["changes"] })),
        ),
        "restart" => ("POST", "/api/v1/restart".into(), None),
        "stop" => {
            confirmed()?;
            ("POST", "/api/v1/stop".into(), None)
        }
        other => return Err(format!("unknown tool {other}")),
    })
}

/// One tool call: its result, and whether it failed.
fn call(dir: &Path, name: &str, args: &Value) -> (Value, bool) {
    if name == "start" {
        // `start` prints as it waits: its output goes in the result, not on the protocol's stdout.
        let started = std::env::current_exe().and_then(|exe| {
            std::process::Command::new(exe)
                .arg("--dir")
                .arg(dir)
                .arg("start")
                .stdin(std::process::Stdio::null())
                .output()
        });
        return match started {
            Ok(out) => (
                json!({ "output": format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)) }),
                !out.status.success(),
            ),
            Err(e) => (json!({ "error": e.to_string() }), true),
        };
    }
    let result = request(name, args).and_then(|(method, path, body)| {
        let client = Client::open(dir)?.ok_or_else(|| {
            format!(
                "no server is running in {} (the start tool starts it)",
                dir.display()
            )
        })?;
        client.call(method, &path, body.as_ref())
    });
    match result {
        Ok(value) => (value, false),
        Err(e) => (json!({ "error": e }), true),
    }
}

fn handle(dir: &Path, method: &str, params: &Value) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "horizon-server", "version": crate::VERSION },
            "instructions": "Run and moderate a Horizon (Minecraft Java Edition 26.3) server. Start with status and players. Moderation tools (kick, ban, pardon, ban_ip, op, whitelist...) run vanilla's console commands and answer `changed` and the console's lines; events and logs read what happened since a number they return. Destructive tools that can't be undone need confirm: true.",
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or((-32602, "name is required".to_owned()))?;
            if !TOOLS.iter().any(|t| t.name == name) {
                return Err((-32602, format!("Unknown tool {name}")));
            }
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let (result, error) = call(dir, name, &args);
            let text = serde_json::to_string_pretty(&result).unwrap_or_default();
            Ok(json!({ "content": [{ "type": "text", "text": text }], "isError": error }))
        }
        _ => Err((-32601, format!("Method not found: {method}"))),
    }
}

pub fn serve(dir: &Path) -> Result<(), String> {
    let mut stdout = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Err(e) => Some(
                json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } }),
            ),
            Ok(message) => {
                let method = message
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let params = message.get("params").cloned().unwrap_or(json!({}));
                // Notifications (no id) get no reply.
                message.get("id").cloned().map(|id| match handle(dir, method, &params) {
                    Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
                })
            }
        };
        if let Some(reply) = reply {
            writeln!(stdout, "{reply}")
                .and_then(|()| stdout.flush())
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_schema_and_a_call() {
        let list = tools();
        assert_eq!(list.len(), TOOLS.len());
        for (tool, t) in list.iter().zip(TOOLS) {
            assert_eq!(tool["inputSchema"]["type"], "object");
            if t.name == "start" {
                continue;
            }
            // With every argument given, each tool makes a call (confirm included).
            let mut args = json!({ "confirm": true, "changes": {} });
            for (name, kind, _, _) in t.args {
                if args.get(*name).is_none() {
                    args[*name] = match *kind {
                        "integer" => json!(0),
                        _ if *name == "action" => json!("add"),
                        _ if *name == "ip" => json!("203.0.113.7"),
                        _ if *name == "types" => json!("chat"),
                        _ => json!("Kim0"),
                    };
                }
            }
            assert!(request(t.name, &args).is_ok(), "{}", t.name);
        }
    }

    #[test]
    fn calls_map_onto_the_api() {
        let (method, path, body) =
            request("ban", &json!({ "name": "Lou0", "reason": "cheating" })).unwrap();
        assert_eq!((method, path.as_str()), ("POST", "/api/v1/players/ban"));
        assert_eq!(body, Some(json!({ "name": "Lou0", "reason": "cheating" })));
        let (_, _, body) = request("kick", &json!({ "name": "Kim0" })).unwrap();
        assert_eq!(body, Some(json!({ "name": "Kim0" })));
        let (_, path, _) =
            request("whitelist", &json!({ "action": "remove", "name": "Kim0" })).unwrap();
        assert_eq!(path, "/api/v1/whitelist/remove");
        let (_, path, _) =
            request("events", &json!({ "since": 41, "types": "chat,death" })).unwrap();
        assert_eq!(path, "/api/v1/events?since=41&types=chat,death");
        // What can't be undone needs confirm: true; nothing reaches the query unchecked.
        assert!(request("stop", &json!({})).is_err());
        assert!(request("restore", &json!({ "name": "x", "confirm": false })).is_err());
        assert!(request("delete_backup", &json!({ "name": "a&b", "confirm": true })).is_err());
        assert!(request("events", &json!({ "types": "chat&since=0" })).is_err());
        assert!(request("whitelist", &json!({ "action": "clear" })).is_err());
    }

    #[test]
    fn the_protocol_answers_without_a_server() {
        let dir = std::env::temp_dir().join(format!("horizon-server-mcp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let init = handle(
            &dir,
            "initialize",
            &json!({ "protocolVersion": "2025-03-26" }),
        )
        .unwrap();
        assert_eq!(init["protocolVersion"], "2025-03-26");
        assert!(
            handle(&dir, "tools/list", &json!({})).unwrap()["tools"]
                .as_array()
                .unwrap()
                .len()
                > 20
        );
        let reply = handle(
            &dir,
            "tools/call",
            &json!({ "name": "players", "arguments": {} }),
        )
        .unwrap();
        assert_eq!(reply["isError"], true);
        assert!(
            reply["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("no server is running")
        );
        assert_eq!(
            handle(&dir, "tools/call", &json!({ "name": "nope" }))
                .unwrap_err()
                .0,
            -32602
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
