//! The game's side of the Launch API (docs/api/launch.md): the options another launcher may pass,
//! `--launch-api`'s reply, the exit codes, and docs/api/launch-reference.md, generated from them.
//! Update it with: `HORIZON_WRITE_DOCS=1 cargo test -p horizon-client --bin horizon-client launch_api`.

/// The Launch API's version (docs/api/stability.md): an addition makes the next patch.
pub const VERSION: &str = "0.1.0";

/// One option of the contract: its name, its value's placeholder (none for a flag), what it does.
pub struct LaunchOption {
    pub name: &'static str,
    pub value: Option<&'static str>,
    /// For the reference only.
    #[cfg_attr(not(test), allow(dead_code))]
    pub what: &'static str,
}

const fn option(
    name: &'static str,
    value: Option<&'static str>,
    what: &'static str,
) -> LaunchOption {
    LaunchOption { name, value, what }
}

/// What a launcher may pass. The game takes more (`--help` lists them), but only these are
/// promised: they keep their meaning within a line of this API.
pub const OPTIONS: &[LaunchOption] = &[
    option(
        "--resources",
        Some("DIR"),
        "The prepared resources (horizon-prepare's output). Default: `resources` beside the executable, else `.local/horizon-client/26.3`.",
    ),
    option(
        "--game-dir",
        Some("DIR"),
        "The game folder: `saves/`, `options.txt`, `resourcepacks/`, `horizon-mods/`, `screenshots/`. Created when missing. Default: the working directory. `--gameDir` is the same.",
    ),
    option(
        "--home",
        Some("DIR"),
        "Logs (`logs/latest.log`) and crash reports (`crash-reports/`). Default: `HORIZON_HOME`, else `horizon` in the platform data folder.",
    ),
    option(
        "--width",
        Some("N"),
        "The window's starting width (default 1280).",
    ),
    option(
        "--height",
        Some("N"),
        "The window's starting height (default 720).",
    ),
    option(
        "--fullscreen",
        None,
        "Opens the window fullscreen, whatever `options.txt` says; `options.txt` is not changed.",
    ),
    option(
        "--session-file",
        Some("PATH"),
        "The signed-in account, as the session JSON below. Read once, then deleted.",
    ),
    option(
        "--session-stdin",
        None,
        "The same JSON on standard input, so it never touches the disk.",
    ),
    option(
        "--quickPlaySingleplayer",
        Some("WORLD"),
        "Opens this world of `saves/` at start, only if it exists (else the title screen). WORLD is one folder name: a path is refused.",
    ),
    option(
        "--quickPlayMultiplayer",
        Some("HOST[:PORT]"),
        "Joins this server at start.",
    ),
    option(
        "--launch-api",
        None,
        "Prints the reply below and exits 0, before anything else runs.",
    ),
];

/// How the game ends, for the launcher that started it.
pub mod exit_code {
    /// The player quit, or `--help` and `--launch-api` printed.
    pub const OK: i32 = 0;
    /// The game stopped on an error; a crash report was saved.
    pub const ERROR: i32 = 1;
    /// The command line was refused (an unknown option, a missing or bad value); no crash report.
    pub const USAGE: i32 = 2;
    /// A panic; a crash report was saved.
    pub const PANIC: i32 = 101;
}

const EXIT_CODES: &[(i32, &str)] = &[
    (
        exit_code::OK,
        "The player quit, or `--help` or `--launch-api` printed.",
    ),
    (
        exit_code::ERROR,
        "The game stopped on an error. A crash report was saved.",
    ),
    (
        exit_code::USAGE,
        "The command line was refused: an unknown option, a missing or bad value. No crash report.",
    ),
    (
        exit_code::PANIC,
        "The game panicked. A crash report was saved.",
    ),
];

/// A command line the game refuses: exit code 2, no crash report.
#[derive(Debug)]
pub struct UsageError(pub String);

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UsageError {}

/// `--launch-api`'s reply for a build of this `horizon` version.
pub fn reply(horizon: &str) -> String {
    let options: Vec<_> = OPTIONS
        .iter()
        .map(|o| serde_json::json!({ "name": o.name, "value": o.value }))
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "launch_api": VERSION,
        "horizon": horizon,
        "minecraft": horizon_core::VANILLA_VERSION,
        "options": options,
        "exit_codes": EXIT_CODES.iter().map(|(code, _)| code).collect::<Vec<_>>(),
        "crash_report_line": horizon_client::crash::SAVED_LINE,
    }))
    .expect("a JSON value")
}

/// docs/api/launch-reference.md.
#[cfg(test)]
pub fn markdown() -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let w = &mut out;
    let _ = writeln!(
        w,
        "<!-- Generated from crates/horizon-client/src/game/launch_api.rs; a test fails when this file is out of date. Update it with: HORIZON_WRITE_DOCS=1 cargo test -p horizon-client --bin horizon-client launch_api -->"
    );
    let _ = writeln!(w, "# Launch API reference\n");
    let _ = writeln!(
        w,
        "What `horizon-client` promises a launcher that starts it, Launch API {VERSION}. The guide: \
         [launch.md](launch.md); stability and versions: [stability.md](stability.md).\n"
    );
    let _ = writeln!(w, "## Options\n");
    let _ = writeln!(
        w,
        "Any other option is refused with exit code 2, so ask `--launch-api` before passing one \
         a build may not know.\n"
    );
    let _ = writeln!(w, "| Option | What it does |\n| --- | --- |");
    for o in OPTIONS {
        let usage = match o.value {
            Some(value) => format!("`{} {value}`", o.name),
            None => format!("`{}`", o.name),
        };
        let _ = writeln!(w, "| {usage} | {} |", o.what);
    }
    let _ = writeln!(w, "\n## `--launch-api`\n");
    let _ = writeln!(
        w,
        "Prints one JSON object on stdout and exits 0. `horizon` is the build's version; readers \
         ignore fields they don't know.\n"
    );
    let _ = writeln!(w, "```json\n{}\n```\n", reply("<version>"));
    let _ = writeln!(w, "## Exit codes\n\n| Code | Meaning |\n| --- | --- |");
    for (code, what) in EXIT_CODES {
        let _ = writeln!(w, "| {code} | {what} |");
    }
    let _ = writeln!(
        w,
        "\nWhen a crash report is saved, the last line on stderr is `{}` followed by the \
         report's path.",
        horizon_client::crash::SAVED_LINE.trim_end()
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> horizon_client::Result<Option<super::super::Options>> {
        super::super::Options::parse(args.iter().map(|a| a.to_string()))
    }

    /// Every promised option is one the game accepts.
    #[test]
    fn launch_api_options_are_accepted() {
        for o in OPTIONS {
            let args: Vec<&str> = std::iter::once(o.name)
                .chain(o.value.map(|v| if v == "N" { "100" } else { "x" }))
                .collect();
            assert!(parse(&args).is_ok(), "{} is refused", o.name);
        }
    }

    #[test]
    fn launch_api_flags_are_read() {
        let o = parse(&["--fullscreen", "--game-dir", "g", "--session-stdin"])
            .unwrap()
            .unwrap();
        assert!(o.fullscreen && o.session_stdin);
        assert_eq!(o.game_dir.as_deref(), Some(std::path::Path::new("g")));
        assert!(parse(&["--launch-api"]).unwrap().is_none());
        assert!(parse(&["--no-such-option"]).is_err());
    }

    /// A launcher shows the error to the player, so it names the option.
    #[test]
    fn launch_api_bad_value_names_its_option() {
        let error = |args: &[&str]| parse(args).err().unwrap().to_string();
        assert_eq!(
            error(&["--width", "wide"]),
            "--width: invalid digit found in string"
        );
        assert_eq!(error(&["--bogus", "x"]), "--bogus: unknown option");
    }

    /// Quick play opens one folder of saves/, as on vanilla: nothing outside it.
    #[test]
    fn launch_api_quick_play_is_one_folder_of_saves() {
        for name in ["../x", "a/b", "/abs", ".", ".."] {
            assert!(
                parse(&["--quickPlaySingleplayer", name]).is_err(),
                "{name} is accepted"
            );
        }
        assert!(parse(&["--quickPlaySingleplayer", "E2E World"]).is_ok());
    }

    /// docs/api/launch-reference.md is what `markdown` writes.
    #[test]
    fn launch_api_reference_is_current() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("docs/api/launch-reference.md");
        let want = markdown();
        if std::env::var_os("HORIZON_WRITE_DOCS").is_some() {
            std::fs::write(&path, &want).unwrap();
        }
        let have = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            have == want,
            "docs/api/launch-reference.md is out of date: HORIZON_WRITE_DOCS=1 cargo test -p horizon-client --bin horizon-client launch_api"
        );
    }
}
