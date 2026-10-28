//! Every launcher CLI command as an MCP tool, generated from the CLI's own definition (clap) so the
//! two can't drift: the tool `mods_search` is `horizon-launcher mods search`, its arguments are the
//! command's (snake_case), and a call runs that command with `--json` as a child process and
//! returns what it printed. A few commands keep a process or a sign-in going for the session;
//! mcp.rs runs those itself under the same names (`play`, `server`, `account_sign_in`...).
use clap::{ArgAction, Command};
use serde_json::{json, Map, Value};

/// CLI commands that are not generated tools: the MCP server itself; No AI, the player's switch
/// and never an agent's; trusting native mods' code, the player's decision too; and the ones
/// mcp.rs runs in the session.
const NOT_TOOLS: &[&str] = &[
    "mcp",
    "agent-config",
    "no-ai",
    "native-build",
    "play",
    "server",
    "account sign-in",
    "account list",
];

/// Commands that delete the player's things: their tools need `confirm: true`.
pub const DESTRUCTIVE: &[&str] = &[
    "remove",
    "modpack remove",
    "mods remove",
    "servers forget",
    "account sign-out",
    "data delete",
    "worlds delete",
    "multiplayer remove",
    "screenshots delete",
    "packs remove",
    "worlds datapack-remove",
    "unshare-log",
    // It sets shell commands the next Play runs: the player says yes first.
    "instance launch",
];

/// Commands that send something or reach other people: only when the player asks.
pub const OPEN_WORLD: &[&str] = &[
    "data allow",
    "data send",
    "data bug",
    "data benchmark",
    "servers create",
    "share-log",
];

/// A generated tool: its name, the CLI words it runs and the clap command behind them.
pub struct Tool {
    pub name: String,
    pub path: Vec<String>,
    command: Command,
}

/// The tool name of CLI words: `mods search` gives `mods_search`.
pub fn tool_name(path: &[String]) -> String {
    path.join("_").replace('-', "_")
}

/// Every generated tool, in the CLI's order.
pub fn all() -> Vec<Tool> {
    let mut out = Vec::new();
    walk(&crate::cli::command(), &mut Vec::new(), &mut out);
    out
}

fn walk(cmd: &Command, path: &mut Vec<String>, out: &mut Vec<Tool>) {
    for sub in cmd.get_subcommands() {
        path.push(sub.get_name().to_owned());
        if sub.has_subcommands() {
            walk(sub, path, out);
        } else if !NOT_TOOLS.contains(&path.join(" ").as_str()) {
            out.push(Tool {
                name: tool_name(path),
                path: path.clone(),
                command: sub.clone(),
            });
        }
        path.pop();
    }
}

impl Tool {
    fn words(&self) -> String {
        self.path.join(" ")
    }
    pub fn destructive(&self) -> bool {
        DESTRUCTIVE.contains(&self.words().as_str())
    }

    /// The tools/list entry.
    pub fn describe(&self) -> Value {
        let mut props = Map::new();
        let mut required = Vec::new();
        for arg in self.command.get_arguments().filter(|a| shown(a)) {
            let id = arg.get_id().as_str();
            let mut help = arg.get_help().map(ToString::to_string).unwrap_or_default();
            let defaults: Vec<String> = arg
                .get_default_values()
                .iter()
                .map(|d| d.to_string_lossy().into_owned())
                .collect();
            if !defaults.is_empty() {
                help = format!("{help} (default {})", defaults.join(", "))
                    .trim_start()
                    .to_owned();
            }
            let choices: Vec<String> = arg
                .get_possible_values()
                .iter()
                .map(|v| v.get_name().to_owned())
                .collect();
            let one = if !choices.is_empty() {
                json!({ "type": "string", "enum": choices })
            } else if defaults.first().is_some_and(|d| d.parse::<i64>().is_ok()) {
                json!({ "type": "integer" })
            } else {
                json!({ "type": "string" })
            };
            let mut schema = if matches!(arg.get_action(), ArgAction::SetTrue) {
                json!({ "type": "boolean" })
            } else if many(arg) {
                json!({ "type": "array", "items": one })
            } else {
                one
            };
            if !help.is_empty() {
                schema["description"] = help.into();
            }
            props.insert(id.to_owned(), schema);
            if arg.is_required_set() {
                required.push(id.to_owned());
            }
        }
        if self.destructive() {
            props.insert(
                "confirm".into(),
                json!({ "type": "boolean", "description": "true once the player agreed: it cannot be undone" }),
            );
            required.push("confirm".into());
        }
        let about = self
            .command
            .get_about()
            .map(ToString::to_string)
            .unwrap_or_default();
        let mut tool = json!({
            "name": self.name,
            "description": format!("{about}. CLI: horizon-launcher {}", self.words()),
            "inputSchema": { "type": "object", "properties": props, "required": required },
        });
        if self.destructive() {
            tool["annotations"] = json!({ "destructiveHint": true });
        } else if OPEN_WORLD.contains(&self.words().as_str()) {
            tool["annotations"] = json!({ "openWorldHint": true });
        }
        tool
    }

    /// The CLI words for a call's arguments.
    pub fn argv(&self, args: &Value) -> Result<Vec<String>, String> {
        if self.destructive() && args.get("confirm").and_then(Value::as_bool) != Some(true) {
            return Err(format!(
                "{} cannot be undone: ask the player, then call it with confirm: true",
                self.name
            ));
        }
        let text = |v: &Value| match v {
            Value::String(s) => Ok(s.clone()),
            Value::Number(n) => Ok(n.to_string()),
            Value::Bool(b) => Ok(b.to_string()),
            other => Err(format!("{other} is not a value")),
        };
        let mut words = self.path.clone();
        let mut positional = Vec::new();
        let mut last = Vec::new();
        for arg in self.command.get_arguments().filter(|a| shown(a)) {
            let id = arg.get_id().as_str();
            let Some(value) = args.get(id).filter(|v| !v.is_null()) else {
                if arg.is_required_set() {
                    return Err(format!("{id} is required"));
                }
                continue;
            };
            let values = match value {
                Value::Array(items) => items.iter().map(text).collect::<Result<Vec<_>, _>>()?,
                v => vec![text(v)?],
            };
            if matches!(arg.get_action(), ArgAction::SetTrue) {
                if value.as_bool().ok_or(format!("{id} is true or false"))? {
                    words.push(format!("--{}", arg.get_long().unwrap_or(id)));
                }
            } else if arg.is_positional() {
                let into = if arg.is_last_set() {
                    &mut last
                } else {
                    &mut positional
                };
                into.push((arg.get_index().unwrap_or(0), values));
            } else {
                let long = arg.get_long().unwrap_or(id);
                for v in values {
                    // `=` keeps a value that starts with a dash (game arguments) a value.
                    words.push(format!("--{long}={v}"));
                }
            }
        }
        positional.sort_by_key(|(index, _)| *index);
        words.extend(positional.into_iter().flat_map(|(_, v)| v));
        if !last.is_empty() {
            words.push("--".into());
            words.extend(last.into_iter().flat_map(|(_, v)| v));
        }
        Ok(words)
    }

    /// Runs the command as `horizon-launcher --json ...` and returns what it printed.
    pub fn call(&self, args: &Value) -> Result<String, String> {
        let words = self.argv(args)?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let out = horizon_launcher::process::command(exe)
            .arg("--json")
            .args(&words)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("Could not run horizon-launcher: {e}"))?;
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        let text = match (stdout.is_empty(), stderr.is_empty()) {
            (false, true) => stdout,
            (true, false) => stderr,
            (true, true) => "Done.".into(),
            (false, false) => format!("{stdout}\n{stderr}"),
        };
        if out.status.success() {
            Ok(text)
        } else {
            Err(text)
        }
    }
}

/// The tools mcp.rs runs itself for a CLI command, and what an agent uses instead of the
/// player-only ones.
#[cfg(test)]
fn session_tool(words: &str) -> &'static str {
    match words {
        "play" => "`play`, then the `game_*` tools",
        "server" => "`server`, `server_command`, `server_log`, `server_stop`",
        "account sign-in" => "`account_sign_in`",
        "account list" => "`account_list`",
        "mcp" => "none: this command is the MCP server",
        "agent-config" => "none: it prints how to connect a client to `mcp`",
        _ => "none: the player's switch",
    }
}

/// docs/launcher-commands.md: every CLI command, its arguments and its MCP tool, from the CLI's
/// definition. A test keeps the file equal to this.
#[cfg(test)]
fn markdown() -> String {
    let mut out = String::from(
        "# Launcher commands\n\n<!-- Generated from launcher/src/cli.rs by cli_tools::markdown(); a test fails when this file is out of date. Update it with: HORIZON_WRITE_DOCS=1 cargo test --manifest-path launcher/Cargo.toml the_launcher_reference -->\n\n\
Every command of `horizon-launcher`, and the tool `horizon-launcher mcp` offers for it. A tool's \
arguments are the command's, in snake_case. A tool runs the command with `--json` and returns what it printed. \
The tools that change the player's things for good need `confirm: true`. \
The tools that send something or reach other people say \"only when the player asks\". \
The game's own tools, `game_*`, are in [horizon-ctl-commands.md](horizon-ctl-commands.md). \
With No AI on, `mcp` and `agent-config` are hidden and refuse; every other command stays. See [agent-control.md](agent-control.md#no-ai).\n\n",
    );
    let mut rows = Vec::new();
    collect(&crate::cli::command(), &mut Vec::new(), &mut rows);
    let generated: Vec<String> = all().iter().map(|t| t.words()).collect();
    for (words, cmd) in rows {
        let usage = cmd
            .clone()
            .bin_name(format!("horizon-launcher {words}"))
            .render_usage()
            .to_string();
        let usage = usage
            .trim_start_matches("Usage: ")
            .replace(" [OPTIONS]", "");
        out += &format!("## `horizon-launcher {words}`\n\n");
        if let Some(about) = cmd.get_about() {
            out += &format!("{about}.\n\n");
        }
        out += &format!("    {usage}\n\n");
        let args: Vec<&clap::Arg> = cmd.get_arguments().filter(|a| shown(a)).collect();
        for arg in &args {
            let name = match arg.get_long() {
                Some(long) => format!("--{long}"),
                None => arg.get_id().as_str().to_uppercase(),
            };
            let help = arg.get_help().map(ToString::to_string).unwrap_or_default();
            let choices: Vec<String> = arg
                .get_possible_values()
                .iter()
                .map(|v| v.get_name().to_owned())
                .collect();
            let choices = if choices.is_empty() || matches!(arg.get_action(), ArgAction::SetTrue) {
                String::new()
            } else {
                format!(" ({})", choices.join(", "))
            };
            let help = if help.is_empty() {
                String::new()
            } else {
                format!(": {help}")
            };
            out += &format!("- `{name}`{choices}{help}\n");
        }
        if !args.is_empty() {
            out += "\n";
        }
        let mcp = if generated.contains(&words) {
            let path: Vec<String> = words.split(' ').map(str::to_owned).collect();
            let mut text = format!("`{}`", tool_name(&path));
            if DESTRUCTIVE.contains(&words.as_str()) {
                text += ", needs `confirm: true`";
            } else if OPEN_WORLD.contains(&words.as_str()) {
                text += ", only when the player asks";
            }
            text
        } else {
            session_tool(&words).to_owned()
        };
        out += &format!("MCP: {mcp}.\n\n");
    }
    out.truncate(out.trim_end().len());
    out.push('\n');
    out
}

/// Every command a player can type, groups included when they run on their own.
#[cfg(test)]
fn collect(cmd: &Command, path: &mut Vec<String>, out: &mut Vec<(String, Command)>) {
    for sub in cmd.get_subcommands().filter(|c| c.get_name() != "help") {
        path.push(sub.get_name().to_owned());
        if sub.has_subcommands() {
            collect(sub, path, out);
        } else {
            out.push((path.join(" "), sub.clone()));
        }
        path.pop();
    }
}

/// Whether an argument is part of a tool (not --help, --version or the global --json).
fn shown(arg: &clap::Arg) -> bool {
    !arg.is_hide_set() && !matches!(arg.get_id().as_str(), "help" | "version" | "json")
}

/// Whether an argument takes several values (a list).
fn many(arg: &clap::Arg) -> bool {
    matches!(arg.get_action(), ArgAction::Append)
        || arg.get_num_args().is_some_and(|n| n.max_values() > 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> Tool {
        all().into_iter().find(|t| t.name == name).unwrap()
    }

    #[test]
    fn every_cli_command_is_a_tool_named_after_its_words() {
        let names: Vec<String> = all().into_iter().map(|t| t.name).collect();
        for want in [
            "versions",
            "install",
            "remove",
            "prepare",
            "notes",
            "update",
            "game_args",
            "settings",
            "logs",
            "mods_search",
            "mods_install",
            "modpack_import",
            "servers_create",
            "servers_run",
            "data_status",
            "data_bug",
            "account_select",
            "benchmark",
        ] {
            assert!(names.iter().any(|n| n == want), "{want} missing: {names:?}");
        }
        for not in ["mcp", "no_ai", "play", "server", "account_sign_in"] {
            assert!(!names.iter().any(|n| n == not), "{not}");
        }
    }

    #[test]
    fn arguments_become_cli_words() {
        let search = tool("mods_search");
        let schema = search.describe();
        assert_eq!(schema["inputSchema"]["properties"]["text"]["type"], "array");
        let kinds = &schema["inputSchema"]["properties"]["kind"]["enum"];
        assert!(
            kinds.as_array().unwrap().contains(&json!("modpack"))
                && kinds.as_array().unwrap().contains(&json!("shader")),
            "{kinds}"
        );
        assert_eq!(
            search
                .argv(&json!({ "text": ["fresh", "animations"], "kind": "resource", "everything": true }))
                .unwrap(),
            ["mods", "search", "--kind=resource", "--everything", "fresh", "animations"]
        );
        let run = tool("servers_run");
        assert_eq!(
            run.argv(&json!({ "server": "My server", "args": ["status"] }))
                .unwrap(),
            ["servers", "run", "My server", "--", "status"]
        );
        assert!(run
            .argv(&json!({ "server": "x" }))
            .unwrap_err()
            .contains("args is required"));
        let logs = tool("logs").describe();
        assert_eq!(
            logs["inputSchema"]["properties"]["lines"]["type"],
            "integer"
        );
    }

    /// Whether the reference on disk says `want`, whatever line endings the checkout gave it: Git
    /// for Windows checks text out with CRLF (core.autocrlf) where .gitattributes does not pin LF.
    fn up_to_date(have: &str, want: &str) -> bool {
        have.replace("\r\n", "\n") == want
    }

    #[test]
    fn a_windows_checkout_of_the_reference_is_up_to_date() {
        let want = markdown();
        assert!(up_to_date(&want, &want));
        assert!(up_to_date(&want.replace('\n', "\r\n"), &want));
        assert!(!up_to_date(&want.replacen("## ", "### ", 1), &want));
    }

    #[test]
    fn the_launcher_reference_is_up_to_date() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/launcher-commands.md");
        let want = markdown();
        if std::env::var_os("HORIZON_WRITE_DOCS").is_some() {
            std::fs::write(&path, &want).unwrap();
        }
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            up_to_date(&have, &want),
            "docs/launcher-commands.md is out of date: HORIZON_WRITE_DOCS=1 cargo test --manifest-path launcher/Cargo.toml the_launcher_reference"
        );
        for tool in all() {
            assert!(
                have.contains(&format!("`{}`", tool.name)),
                "{} undocumented",
                tool.name
            );
        }
    }

    #[test]
    fn destructive_tools_need_confirm() {
        let remove = tool("remove");
        assert_eq!(remove.describe()["annotations"]["destructiveHint"], true);
        assert!(remove
            .argv(&json!({ "tag": "v1" }))
            .unwrap_err()
            .contains("confirm"));
        assert_eq!(
            remove
                .argv(&json!({ "tag": "v1", "confirm": true }))
                .unwrap(),
            ["remove", "v1"]
        );
        for words in DESTRUCTIVE.iter().chain(OPEN_WORLD) {
            assert!(
                all().iter().any(|t| t.words() == *words),
                "{words} is not a tool"
            );
        }
    }
}
