//! The running game's tools in the launcher's MCP, so an agent needs one server
//! (docs/agent-control.md): the selected version's `horizon-ctl mcp` runs as a child, its tools
//! are offered as `game_<name>`, and calls go to it unchanged. `play` starts the game
//! with the control socket that child talks to.
use horizon_launcher::backend;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Stdio};

pub const PREFIX: &str = "game_";

/// horizon-ctl tools the launcher covers itself: `play` starts the game.
const HIDDEN: &[&str] = &["launch"];

const CTL: &str = if cfg!(windows) {
    "horizon-ctl.exe"
} else {
    "horizon-ctl"
};

/// The control endpoint of a game started by `play`: a Unix socket, or on Windows the
/// file naming a localhost port and its token.
pub fn socket_path() -> PathBuf {
    backend::data_dir().join(if cfg!(unix) {
        "game-control.sock"
    } else {
        "game-control.json"
    })
}

/// The game arguments that put it under the agent's control.
pub fn control_args() -> Vec<String> {
    vec![
        "--control-socket".into(),
        socket_path().display().to_string(),
    ]
}

/// Whether a game answers on the control endpoint (a crashed game's socket refuses).
pub fn listening() -> bool {
    let path = socket_path();
    #[cfg(unix)]
    return std::os::unix::net::UnixStream::connect(path).is_ok();
    #[cfg(not(unix))]
    return path.exists();
}

pub struct GameTools {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    tools: Vec<Value>,
}

impl GameTools {
    /// Start `horizon-ctl mcp` of an installed version and read its tools.
    pub fn start(tag: &str) -> Result<Self, String> {
        let ctl = backend::version_dir(tag).join(CTL);
        if !ctl.exists() {
            return Err(format!("{tag} has no {CTL}"));
        }
        let mut child = horizon_launcher::process::command(&ctl)
            .arg("--socket")
            .arg(socket_path())
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Could not start {}: {e}", ctl.display()))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("no stdout")?);
        let mut tools = Self {
            child,
            stdin,
            stdout,
            next_id: 0,
            tools: Vec::new(),
        };
        tools.rpc(
            // The child is whichever `horizon-ctl` the player has installed, so open with the
            // handshake revision: older versions do not know the stateless one.
            "initialize",
            json!({ "protocolVersion": horizon_mcp::LEGACY, "capabilities": {},
                    "clientInfo": { "name": "horizon-launcher", "version": backend::VERSION } }),
        )?;
        tools.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))?;
        let listed = tools.rpc("tools/list", json!({}))?;
        tools.tools = prefixed(&listed["tools"]);
        Ok(tools)
    }

    /// The game's tools, named `game_<name>`.
    pub fn tools(&self) -> &[Value] {
        &self.tools
    }

    /// Call a `game_` tool; the result is horizon-ctl's (text and image content).
    pub fn call(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        let inner = name.strip_prefix(PREFIX).ok_or("not a game tool")?;
        let result = self.rpc("tools/call", json!({ "name": inner, "arguments": args }))?;
        Ok(explain_unreachable(result))
    }

    fn send(&mut self, message: &Value) -> Result<(), String> {
        writeln!(self.stdin, "{message}")
            .and_then(|_| self.stdin.flush())
            .map_err(|e| format!("horizon-ctl stopped: {e}"))
    }

    fn rpc(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        let mut line = String::new();
        loop {
            line.clear();
            match self.stdout.read_line(&mut line) {
                Ok(0) | Err(_) => return Err("horizon-ctl stopped".into()),
                Ok(_) => {}
            }
            let Ok(reply) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if reply["id"] != id {
                continue;
            }
            if let Some(error) = reply.get("error") {
                return Err(error["message"].as_str().unwrap_or("error").to_owned());
            }
            return Ok(reply["result"].clone());
        }
    }
}

impl Drop for GameTools {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// horizon-ctl's tools as the launcher offers them.
fn prefixed(tools: &Value) -> Vec<Value> {
    tools
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| !HIDDEN.contains(&t["name"].as_str().unwrap_or_default()))
        .map(|t| {
            let mut t = t.clone();
            t["name"] = format!("{PREFIX}{}", t["name"].as_str().unwrap_or_default()).into();
            t["description"] = format!(
                "In the running game (start it with play): {}",
                t["description"].as_str().unwrap_or_default()
            )
            .into();
            t
        })
        .collect()
}

/// A game tool called while no game listens says how to start one from here.
fn explain_unreachable(mut result: Value) -> Value {
    let unreachable = result["isError"] == true
        && result["content"].as_array().is_some_and(|c| {
            c.iter().any(|block| {
                block["text"]
                    .as_str()
                    .and_then(|t| serde_json::from_str::<Value>(t).ok())
                    .is_some_and(|v| v["kind"] == "connection")
            })
        });
    if unreachable {
        result["content"] = json!([{ "type": "text", "text": json!({
            "error": "The game is not running under agent control. Start it with play, then game_wait_ready.",
            "kind": "state",
        }).to_string() }]);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_the_game_tools_prefixed_without_launch() {
        let listed = json!([
            { "name": "launch", "description": "start", "inputSchema": {} },
            { "name": "click_slot", "description": "Click a slot", "inputSchema": { "type": "object" } },
        ]);
        let tools = prefixed(&listed);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "game_click_slot");
        assert!(tools[0]["description"]
            .as_str()
            .unwrap()
            .ends_with("Click a slot"));
        assert_eq!(tools[0]["inputSchema"], json!({ "type": "object" }));
    }

    #[test]
    fn a_connection_error_says_to_launch_the_client() {
        let text = json!({ "error": "cannot connect", "kind": "connection" }).to_string();
        let result = explain_unreachable(
            json!({ "content": [{ "type": "text", "text": text }], "isError": true }),
        );
        assert!(result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("with play"));
        let other = json!({ "content": [{ "type": "text", "text": "{\"error\":\"no world\"}" }], "isError": true });
        assert_eq!(explain_unreachable(other.clone()), other);
    }
}
