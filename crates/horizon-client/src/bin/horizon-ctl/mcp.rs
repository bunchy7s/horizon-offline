//! `horizon-ctl mcp`: the Model Context Protocol over stdin and stdout
//! (JSON-RPC 2.0, one message per line, like the launcher's `mcp`), so AI
//! agents can play Horizon. Each tool is a horizon-ctl command from the
//! shared table, run through the same `Session` as the CLI.
use crate::{Session, commands};
use serde_json::{Value, json};

/// What an agent reads before its first call.
const INSTRUCTIONS: &str = "Play Horizon, a Rust reimplementation of Minecraft Java Edition. Start with launch (it finds the game installed by the Horizon launcher and opens or creates a world), then read the world with state, inventory, screen, entities and find_block, and act with goto, look_at_block, break, place, interact, click_slot and click_button. Actions block until they finish and report what happened. Coordinates are block coordinates; yaw 0 faces south (+Z).";

/// The revisions this server advertises: `horizon_mcp` serves them (`2026-07-28` stateless,
/// `2025-06-18` for the clients that still open with `initialize`).
fn server() -> horizon_mcp::Server<'static> {
    horizon_mcp::Server::new("horizon-ctl", horizon_core::release::VERSION, INSTRUCTIONS)
}

fn tools() -> Vec<Value> {
    commands::COMMANDS
        .iter()
        .filter(|c| c.mcp)
        .map(|c| {
            let mut description = format!("{}. Usage: horizon-ctl {}", capital(c.summary), c.usage);
            if !c.details.is_empty() {
                description.push_str(". ");
                description.push_str(c.details);
            }
            let mut schema = commands::schema(c);
            if c.name == "screenshot" {
                schema = json!({ "type": "object", "properties": {} });
                description = "Capture the game window and return it as a PNG image.".into();
            }
            let mut tool = json!({ "name": commands::tool_name(c), "description": description, "inputSchema": schema });
            if commands::DESTRUCTIVE.contains(&c.name) {
                tool["annotations"] = json!({ "destructiveHint": true });
            } else if commands::OPEN_WORLD.contains(&c.name) {
                tool["annotations"] = json!({ "openWorldHint": true });
            }
            tool
        })
        .collect()
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// Standard base64 (RFC 4648) for image content.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

impl Session {
    /// One tool call: (content blocks, is_error).
    fn call(&mut self, name: &str, args: &Value) -> (Vec<Value>, bool) {
        let text = |v: &Value| json!({ "type": "text", "text": serde_json::to_string_pretty(v).unwrap_or_default() });
        let Some(cmd) = commands::COMMANDS
            .iter()
            .find(|c| c.mcp && commands::tool_name(c) == name)
        else {
            return (
                vec![text(&json!({ "error": format!("unknown tool {name}") }))],
                true,
            );
        };
        if cmd.name == "screenshot" {
            let path = std::env::temp_dir().join(format!("horizon-ctl-{}.png", std::process::id()));
            let result = self
                .run(&["screenshot".to_owned(), path.display().to_string()])
                .and_then(|reply| {
                    std::fs::read(&path)
                        .map(|png| (reply, png))
                        .map_err(|e| crate::Failure {
                            reply: json!({ "error": format!("{}: {e}", path.display()) }),
                            status: 1,
                        })
                });
            let _ = std::fs::remove_file(&path);
            return match result {
                Ok((reply, png)) => (
                    vec![
                        json!({ "type": "image", "data": base64(&png), "mimeType": "image/png" }),
                        text(&reply),
                    ],
                    false,
                ),
                Err(f) => (vec![text(&f.reply)], true),
            };
        }
        if cmd.name == "logs" {
            let words = match commands::line_for(cmd, args) {
                Ok(w) => w,
                Err(e) => return (vec![text(&json!({ "error": e, "kind": "usage" }))], true),
            };
            return match self.run(&words) {
                Ok(reply) => (
                    vec![
                        json!({ "type": "text", "text": format!("{}\n\n{}", reply["path"].as_str().unwrap_or_default(), reply["text"].as_str().unwrap_or_default()) }),
                    ],
                    false,
                ),
                Err(f) => (vec![text(&f.reply)], true),
            };
        }
        let mut words = match commands::line_for(cmd, args) {
            Ok(w) => w,
            Err(e) => return (vec![text(&json!({ "error": e, "kind": "usage" }))], true),
        };
        if cmd.name == "launch" {
            // An agent wants a world it can play once the call returns.
            if args.get("wait").and_then(Value::as_bool) != Some(false)
                && !words.contains(&"--wait".to_owned())
            {
                words.push("--wait".into());
            }
            words.push("--no-pause".into());
        }
        match self.run(&words) {
            Ok(reply) => (vec![text(&reply)], false),
            Err(f) => (vec![text(&f.reply)], true),
        }
    }

    /// The two tool methods; `horizon_mcp` answers the protocol's own (`initialize`,
    /// `server/discover`) and frames these results for the revision the request speaks.
    fn handle(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or((-32602, "name is required".to_string()))?;
                if !tools().iter().any(|t| t["name"] == name) {
                    return Err((-32602, format!("Unknown tool {name}")));
                }
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                let (content, error) = self.call(name, &args);
                Ok(json!({ "content": content, "isError": error }))
            }
            _ => Err((-32601, format!("Method not found: {method}"))),
        }
    }
}

pub fn serve(session: &mut Session) -> Result<(), String> {
    server().serve(|method, params| session.handle(method, params))
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn base64_matches_rfc_4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn tools_have_schemas_and_unique_names() {
        let list = tools();
        let mut names = std::collections::HashSet::new();
        for t in &list {
            assert!(names.insert(t["name"].as_str().unwrap().to_owned()));
            assert_eq!(t["inputSchema"]["type"], "object");
        }
        for want in [
            "launch",
            "state",
            "goto",
            "break_block",
            "place",
            "click_slot",
            "click_button",
            "screenshot",
            "inventory",
            "logs",
        ] {
            let want = if want == "break_block" { "break" } else { want };
            assert!(names.contains(want), "{want}");
        }
    }

    #[test]
    fn protocol_basics() {
        let mut s = Session::new(std::env::temp_dir().join("horizon-ctl-mcp-test.sock"), None);
        let mut app = |method: &str, params: &Value| s.handle(method, params);
        let server = server();
        let line = |method: &str, params: Value| {
            json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }).to_string()
        };

        // A client that opens with the handshake revision.
        let init = server
            .handle_line(
                &line("initialize", json!({ "protocolVersion": "2025-06-18" })),
                &mut app,
            )
            .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(init["result"]["serverInfo"]["name"], "horizon-ctl");

        // A modern client: stateless, and the same tools.
        let mut meta = serde_json::Map::new();
        meta.insert(
            horizon_mcp::VERSION_KEY.to_owned(),
            json!(horizon_mcp::MODERN),
        );
        meta.insert(horizon_mcp::CLIENT_CAPABILITIES_KEY.to_owned(), json!({}));
        let params = json!({ "_meta": Value::Object(meta) });
        let list = server
            .handle_line(&line("tools/list", params), &mut app)
            .unwrap();
        assert_eq!(list["result"]["resultType"], "complete");
        assert!(list["result"]["tools"].as_array().unwrap().len() > 3);

        let r = server
            .handle_line(
                &line(
                    "tools/call",
                    json!({ "name": "goto", "arguments": { "x": 1 } }),
                ),
                &mut app,
            )
            .unwrap();
        assert_eq!(r["result"]["isError"], true);
        assert!(
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("y is required")
        );
        // No game: a connection error, reported as a tool error.
        let r = server
            .handle_line(
                &line("tools/call", json!({ "name": "state", "arguments": {} })),
                &mut app,
            )
            .unwrap();
        assert_eq!(r["result"]["isError"], true);
        // Methods this server does not have are the protocol's to refuse.
        assert_eq!(
            server
                .handle_line(&line("nope/nope", json!({})), &mut app)
                .unwrap()["error"]["code"],
            json!(-32601)
        );
    }
}
