//! The Model Context Protocol, as Horizon's three servers speak it (`horizon-ctl mcp`,
//! `horizon-server mcp`, `horizon-launcher mcp`: docs/agent-control.md, docs/server.md section 8):
//! JSON-RPC 2.0 over stdin and stdout, one message per line.
//!
//! Two revisions, newest first. `2026-07-28` is stateless: every request carries its version,
//! its capabilities and who it is in `_meta`, servers answer `server/discover`, every result
//! says `resultType`, and the list results carry `ttlMs` and `cacheScope`. `2025-06-18` opens
//! with `initialize` and is kept for the clients that still do that (the launcher's `game_*`
//! tools talk to whichever `horizon-ctl` version the player has installed, which may be older
//! than this crate). A request carrying `_meta` is served statelessly whatever the process
//! answered before it, as the revision requires.
//!
//! Each server supplies its identity, its instructions and the two tool methods; the protocol
//! methods (`server/discover`, `initialize`), the framing and the line loop live here, so the
//! three cannot drift apart.

use serde_json::{Map, Value, json};
use std::io::{BufRead, Write};

/// The newest revision: no handshake, every request carries `_meta`.
pub const MODERN: &str = "2026-07-28";
/// The handshake revision, still served for the clients that call `initialize`.
pub const LEGACY: &str = "2025-06-18";
/// Every revision served here, newest first: what `server/discover` advertises.
pub const VERSIONS: &[&str] = &[MODERN, LEGACY];
/// The revisions `initialize` may select: the stateless one has no handshake to open.
pub const LEGACY_VERSIONS: &[&str] = &[LEGACY];

/// `_meta` key: the protocol version the request uses (required on every modern request).
pub const VERSION_KEY: &str = "io.modelcontextprotocol/protocolVersion";
/// `_meta` key: the client's capabilities (required on every modern request).
pub const CLIENT_CAPABILITIES_KEY: &str = "io.modelcontextprotocol/clientCapabilities";
/// `_meta` key: who the server is, in every modern result.
pub const SERVER_INFO_KEY: &str = "io.modelcontextprotocol/serverInfo";

/// `UnsupportedProtocolVersion`: the request named a revision this server does not serve.
pub const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

/// `ttlMs` for `tools/list`: starting a game or a server adds tools to the launcher's list, and
/// no `notifications/tools/list_changed` follows it (that needs `subscriptions/listen`), so the
/// hint stays short rather than hiding the new tools until it expires.
pub const LIST_TTL_MS: i64 = 30_000;
/// `ttlMs` for `server/discover`: versions, capabilities and identity are fixed per process.
pub const DISCOVER_TTL_MS: i64 = 3_600_000;
/// Neither list holds anything specific to one caller.
pub const CACHE_SCOPE: &str = "public";

/// One server: who it says it is, and what an agent reads before its first call.
#[derive(Clone, Copy)]
pub struct Server<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub instructions: &'a str,
}

/// A failed request: the JSON-RPC code, its message, and the `data` some errors carry.
struct Fault {
    code: i64,
    message: String,
    data: Option<Value>,
}

impl Fault {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    fn with(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

impl From<(i64, String)> for Fault {
    fn from((code, message): (i64, String)) -> Self {
        Self::new(code, message)
    }
}

/// `initialize` (the handshake revision): the client's version when it is one we serve,
/// otherwise ours. The client has no fall-forward, so never claim one we do not implement.
pub fn negotiate_legacy(params: &Value) -> &'static str {
    let wanted = params.get("protocolVersion").and_then(Value::as_str);
    LEGACY_VERSIONS
        .iter()
        .copied()
        .find(|v| Some(*v) == wanted)
        .unwrap_or(LEGACY_VERSIONS[0])
}

impl<'a> Server<'a> {
    pub const fn new(name: &'a str, version: &'a str, instructions: &'a str) -> Self {
        Server {
            name,
            version,
            instructions,
        }
    }

    /// Serve the process's stdin and stdout until the client closes them.
    ///
    /// `app` answers the two tool methods (`tools/list` and `tools/call`); the protocol
    /// methods and the framing are this module's.
    pub fn serve<F>(&self, mut app: F) -> Result<(), String>
    where
        F: FnMut(&str, &Value) -> Result<Value, (i64, String)>,
    {
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            if let Some(reply) = self.handle_line(&line, &mut app) {
                writeln!(stdout, "{reply}")
                    .and_then(|_| stdout.flush())
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    /// One line of stdin in, one line of stdout out; none for a notification.
    pub fn handle_line<F>(&self, line: &str, app: &mut F) -> Option<Value>
    where
        F: FnMut(&str, &Value) -> Result<Value, (i64, String)>,
    {
        match serde_json::from_str::<Value>(line) {
            Ok(message) => self.handle(&message, app),
            Err(e) => Some(error(Value::Null, -32700, e.to_string(), None)),
        }
    }

    fn handle<F>(&self, message: &Value, app: &mut F) -> Option<Value>
    where
        F: FnMut(&str, &Value) -> Result<Value, (i64, String)>,
    {
        // A request has an id, a notification has none and gets no reply.
        let id = match message.get("id") {
            None => return None,
            Some(Value::Null) => {
                return Some(error(
                    Value::Null,
                    -32600,
                    "A request's id is a string or an integer, not null".into(),
                    None,
                ));
            }
            Some(id) => id.clone(),
        };
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        Some(match self.dispatch(method, &params, app) {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(Fault {
                code,
                message,
                data,
            }) => error(id, code, message, data),
        })
    }

    fn dispatch<F>(&self, method: &str, params: &Value, app: &mut F) -> Result<Value, Fault>
    where
        F: FnMut(&str, &Value) -> Result<Value, (i64, String)>,
    {
        // `initialize` opens the handshake revision, whatever else the request carries.
        if method == "initialize" {
            return Ok(self.initialize(params));
        }
        // `server/discover` is always answered modern: it is how a client learns the versions
        // this server serves, and the probe a client that speaks both revisions sends first.
        if method == "server/discover" {
            return Ok(self.discover());
        }
        // The revision comes from the request itself. Without `_meta` the client is on the
        // handshake revision; with it, the version it names must be one we serve.
        let version = match meta(params, VERSION_KEY) {
            None => None,
            Some(requested) => match VERSIONS.iter().copied().find(|v| *v == requested) {
                Some(version) => Some(version),
                None => {
                    return Err(Fault::new(
                        UNSUPPORTED_PROTOCOL_VERSION,
                        "Unsupported protocol version",
                    )
                    .with(json!({ "supported": VERSIONS, "requested": requested })));
                }
            },
        };
        // A modern request without the client's capabilities is malformed (-32602), and the
        // server must not rely on capabilities the client did not declare.
        if version == Some(MODERN) && meta_value(params, CLIENT_CAPABILITIES_KEY).is_none() {
            return Err(Fault::new(
                -32602,
                format!("_meta.{CLIENT_CAPABILITIES_KEY} is required"),
            ));
        }
        let result = match method {
            "tools/list" | "tools/call" => app(method, params).map_err(Fault::from)?,
            // `ping` was removed in 2026-07-28: only the handshake revision still has it.
            "ping" if version != Some(MODERN) => json!({}),
            _ => return Err(Fault::new(-32601, format!("Method not found: {method}"))),
        };
        Ok(self.frame(version, method, result))
    }

    /// `initialize`: the handshake revision's answer, unchanged since 2025-06-18.
    fn initialize(&self, params: &Value) -> Value {
        json!({
            "protocolVersion": negotiate_legacy(params),
            "capabilities": self.capabilities(),
            "serverInfo": self.info(),
            "instructions": self.instructions,
        })
    }

    /// `server/discover`: the versions, capabilities and identity of this server.
    fn discover(&self) -> Value {
        json!({
            "resultType": "complete",
            "supportedVersions": VERSIONS,
            "capabilities": self.capabilities(),
            "_meta": meta_object(self.info()),
            "instructions": self.instructions,
            "ttlMs": DISCOVER_TTL_MS,
            "cacheScope": CACHE_SCOPE,
        })
    }

    /// What every revision since 2026-07-28 puts around a result: its type, the cache hints on
    /// the cacheable ones, and who answered. The handshake revision's results carry none of it.
    fn frame(&self, version: Option<&'static str>, method: &str, mut result: Value) -> Value {
        if version != Some(MODERN) {
            return result;
        }
        let Some(object) = result.as_object_mut() else {
            return result;
        };
        object.insert("resultType".into(), json!("complete"));
        if method == "tools/list" {
            object.insert("ttlMs".into(), json!(LIST_TTL_MS));
            object.insert("cacheScope".into(), json!(CACHE_SCOPE));
        }
        let meta = object.entry("_meta").or_insert_with(|| json!({}));
        if let Some(meta) = meta.as_object_mut() {
            meta.insert(SERVER_INFO_KEY.into(), self.info());
        }
        result
    }

    fn info(&self) -> Value {
        json!({ "name": self.name, "version": self.version })
    }

    /// Only tools: no resources, prompts, sampling or logging to declare.
    fn capabilities(&self) -> Value {
        json!({ "tools": {} })
    }
}

/// A header of the request's `params`, whatever its type.
fn meta_value<'v>(params: &'v Value, key: &str) -> Option<&'v Value> {
    params.get("_meta")?.get(key)
}

/// A header of the request's `params`, when it is a string.
fn meta<'v>(params: &'v Value, key: &str) -> Option<&'v str> {
    meta_value(params, key)?.as_str()
}

/// A result's `_meta`: who answered.
fn meta_object(info: Value) -> Value {
    let mut meta = Map::new();
    meta.insert(SERVER_INFO_KEY.to_owned(), info);
    Value::Object(meta)
}

fn error(id: Value, code: i64, message: String, data: Option<Value>) -> Value {
    let mut error = json!({ "code": code, "message": message });
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": error })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVER: Server = Server::new("test-server", "1.2.3", "Do the thing.");

    /// The answer to one request, with a tool list and a tool result the way a server answers them.
    fn answer(request: Value) -> Value {
        let mut app = |method: &str, _params: &Value| match method {
            "tools/list" => Ok(json!({ "tools": [{ "name": "state" }] })),
            "tools/call" => {
                Ok(json!({ "content": [{ "type": "text", "text": "ok" }], "isError": false }))
            }
            other => Err((-32601, format!("Method not found: {other}"))),
        };
        SERVER.handle_line(&request.to_string(), &mut app).unwrap()
    }

    /// A modern request's `params`: the version and the capabilities `_meta` must carry.
    fn modern(mut params: Value, version: &str) -> Value {
        let mut meta = Map::new();
        meta.insert(VERSION_KEY.to_owned(), json!(version));
        meta.insert(CLIENT_CAPABILITIES_KEY.to_owned(), json!({}));
        params["_meta"] = Value::Object(meta);
        params
    }

    fn request(id: i64, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    #[test]
    fn initialize_answers_a_version_we_serve() {
        let reply = answer(request(
            1,
            "initialize",
            json!({ "protocolVersion": "2025-06-18" }),
        ));
        assert_eq!(reply["result"]["protocolVersion"], LEGACY);
        assert_eq!(reply["result"]["serverInfo"]["name"], "test-server");
        assert_eq!(reply["result"]["serverInfo"]["version"], "1.2.3");
        assert_eq!(reply["result"]["instructions"], "Do the thing.");
        assert_eq!(reply["result"]["capabilities"]["tools"], json!({}));
        // The handshake revision's results carry no `resultType`.
        assert!(reply["result"].get("resultType").is_none());
    }

    #[test]
    fn initialize_never_claims_a_version_we_do_not_serve() {
        for wanted in [
            json!({ "protocolVersion": MODERN }),
            json!({ "protocolVersion": "1999-01-01" }),
            json!({ "protocolVersion": 7 }),
            json!({}),
        ] {
            let reply = answer(request(1, "initialize", wanted));
            assert_eq!(reply["result"]["protocolVersion"], LEGACY);
        }
    }

    #[test]
    fn discover_lists_what_this_server_serves() {
        // No `_meta`: the stdio probe a client that speaks both revisions sends first.
        let reply = answer(request(1, "server/discover", json!({})));
        let result = &reply["result"];
        assert_eq!(result["resultType"], "complete");
        assert_eq!(result["supportedVersions"], json!(VERSIONS));
        assert_eq!(result["capabilities"]["tools"], json!({}));
        assert_eq!(result["_meta"][SERVER_INFO_KEY]["name"], "test-server");
        assert_eq!(result["instructions"], "Do the thing.");
        assert_eq!(result["ttlMs"], json!(DISCOVER_TTL_MS));
        assert_eq!(result["cacheScope"], CACHE_SCOPE);
    }

    #[test]
    fn an_unsupported_version_is_an_error_that_lists_ours() {
        let reply = answer(request(1, "tools/list", modern(json!({}), "1900-01-01")));
        assert_eq!(
            reply["error"]["code"],
            json!(UNSUPPORTED_PROTOCOL_VERSION),
            "{reply}"
        );
        assert_eq!(reply["error"]["data"]["requested"], "1900-01-01");
        assert_eq!(reply["error"]["data"]["supported"], json!(VERSIONS));
    }

    #[test]
    fn a_modern_request_without_capabilities_is_malformed() {
        let mut params = json!({});
        params["_meta"] = json!({ VERSION_KEY: MODERN });
        let reply = answer(request(1, "tools/list", params));
        assert_eq!(reply["error"]["code"], json!(-32602));
        assert!(
            reply["error"]["message"]
                .as_str()
                .unwrap()
                .contains(CLIENT_CAPABILITIES_KEY)
        );
    }

    #[test]
    fn tools_list_is_cacheable_and_identified() {
        let reply = answer(request(1, "tools/list", modern(json!({}), MODERN)));
        let result = &reply["result"];
        assert_eq!(result["resultType"], "complete");
        assert_eq!(result["ttlMs"], json!(LIST_TTL_MS));
        assert_eq!(result["cacheScope"], CACHE_SCOPE);
        assert_eq!(result["_meta"][SERVER_INFO_KEY]["name"], "test-server");
        assert_eq!(result["tools"][0]["name"], "state");
    }

    #[test]
    fn tools_call_says_it_completed_but_is_not_cacheable() {
        let reply = answer(request(
            1,
            "tools/call",
            modern(json!({ "name": "state" }), MODERN),
        ));
        let result = &reply["result"];
        assert_eq!(result["resultType"], "complete");
        assert_eq!(result["isError"], false);
        assert_eq!(result["_meta"][SERVER_INFO_KEY]["version"], "1.2.3");
        assert!(result.get("ttlMs").is_none(), "{result}");
    }

    #[test]
    fn the_handshake_revisions_results_stay_as_they_were() {
        // A legacy client sends no `_meta`, and its results carry no modern framing.
        let list = answer(request(1, "tools/list", json!({})));
        assert_eq!(list["result"], json!({ "tools": [{ "name": "state" }] }));
        let call = answer(request(2, "tools/call", json!({ "name": "state" })));
        assert_eq!(
            call["result"],
            json!({ "content": [{ "type": "text", "text": "ok" }], "isError": false })
        );
    }

    #[test]
    fn ping_belongs_to_the_handshake_revision_only() {
        assert_eq!(answer(request(1, "ping", json!({})))["result"], json!({}));
        assert_eq!(
            answer(request(1, "ping", modern(json!({}), MODERN)))["error"]["code"],
            json!(-32601)
        );
    }

    #[test]
    fn unknown_methods_and_notifications() {
        let reply = answer(request(1, "nope/nope", json!({})));
        assert_eq!(reply["error"]["code"], json!(-32601));
        // A notification (no id) gets no reply at all.
        let mut app = |_: &str, _: &Value| Ok(json!({}));
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(SERVER.handle_line(&note.to_string(), &mut app).is_none());
    }

    #[test]
    fn a_request_whose_id_is_null_is_invalid() {
        let mut app = |_: &str, _: &Value| Ok(json!({}));
        let line = json!({ "jsonrpc": "2.0", "id": null, "method": "tools/list" });
        let reply = SERVER.handle_line(&line.to_string(), &mut app).unwrap();
        assert_eq!(reply["error"]["code"], json!(-32600));
    }

    #[test]
    fn a_broken_line_is_a_parse_error() {
        let mut app = |_: &str, _: &Value| Ok(json!({}));
        let reply = SERVER.handle_line("{not json", &mut app).unwrap();
        assert_eq!(reply["error"]["code"], json!(-32700));
        assert_eq!(reply["id"], Value::Null);
    }

    #[test]
    fn an_app_error_stays_a_json_rpc_error() {
        let mut app = |_: &str, _: &Value| Err((-32602, "name is required".to_string()));
        let reply = SERVER
            .handle_line(&request(1, "tools/call", json!({})).to_string(), &mut app)
            .unwrap();
        assert_eq!(reply["error"]["code"], json!(-32602));
        assert_eq!(reply["error"]["message"], "name is required");
    }
}
