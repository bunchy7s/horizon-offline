//! `cargo horizon-mod check <file>`: what the registry checks of a submission,
//! run anywhere: its kind's own checks (docs/api/mod-index.md), its signature
//! when given a key, and for a mod a trial run in the sandbox: it must start,
//! survive ticks, frames, a chat line and each of its commands (a server mod:
//! a player joining, chatting, typing them and leaving), and import no more
//! than its capabilities.
use horizon_mods::{
    Clock, GameMode, Mods, Output, Package, Player, ServerPlayer, Snapshot, Status,
    index::{self, Facts, Kind},
    manifest::component_imports,
};
use serde_json::{Value, json};
use std::path::Path;

/// How long the trial run plays: two seconds of game.
const TICKS: u64 = 40;
const FRAMES_PER_TICK: u64 = 3;

pub struct Report {
    pub kind: Kind,
    pub facts: Facts,
    pub sha256: String,
    pub size: u64,
    pub signed: bool,
    /// A mod has code to run (a content mod may have none).
    pub code: bool,
    /// A mod's component imports.
    pub imports: Vec<String>,
    /// A mod's commands, each run once.
    pub commands: Vec<String>,
    /// What the mod showed during the trial run.
    pub output: Vec<String>,
}

impl Report {
    pub fn json(&self) -> Value {
        let requires: Vec<Value> = self
            .facts
            .requires
            .iter()
            .map(|r| json!({"id": r.id, "version": r.version, "sha256": r.sha256}))
            .collect();
        json!({
            "kind": self.kind.name(),
            "id": self.facts.id,
            "version": self.facts.version,
            "api": self.facts.api,
            "side": self.facts.side,
            "capabilities": self.facts.capabilities,
            "api_level": self.facts.api_level,
            "game": self.facts.game,
            "requires": requires,
            "sha256": self.sha256,
            "size": self.size,
            "signed": self.signed,
            "code": self.code,
            "imports": self.imports,
            "commands": self.commands,
            "output": self.output,
        })
    }

    pub fn text(&self) -> String {
        let f = &self.facts;
        let mut out = format!(
            "{} {} ({}",
            f.id.as_deref().unwrap_or("resource pack"),
            f.version.as_deref().unwrap_or(""),
            self.kind.name()
        );
        if let Some(side) = f.side {
            let side = serde_json::to_value(side).unwrap_or_default();
            out += &format!(", API {}, {}", f.api, side.as_str().unwrap_or("?"));
        } else if self.kind == Kind::Shader {
            out += &format!(", shader API {}", f.api);
        } else if !f.api.is_empty() {
            out += &format!(", format {}", f.api);
        }
        if let Some(game) = &f.game {
            out += &format!(", for {game}");
        }
        let signed = if self.signed {
            "signature verified"
        } else {
            "unsigned"
        };
        out += &format!(
            ")\n  {} bytes, SHA-256 {}, {signed}\n",
            self.size, self.sha256
        );
        if let Some(level) = f.api_level {
            out += &format!("  needs mod API level {level} or newer\n");
        }
        for c in &f.capabilities {
            out += &format!("  may {}\n", c.describe());
        }
        for r in &f.requires {
            out += &format!("  needs {} {} ({})\n", r.id, r.version, &r.sha256[..12]);
        }
        if self.kind == Kind::NativeMod {
            out += "  native source: unsandboxed; not executed by the registry check. Compile and test against the exact pinned Horizon and Mixins revisions before review.\n";
        }
        if self.kind == Kind::Mod && !self.code {
            out += "  no code: content only, nothing to run\n";
        } else if self.kind == Kind::Mod {
            let commands = if self.commands.is_empty() {
                String::new()
            } else {
                format!(", /{}", self.commands.join(", /"))
            };
            out += &format!(
                "  trial run: started, {TICKS} ticks, {} frames, a chat line{commands}: ok\n",
                TICKS * FRAMES_PER_TICK
            );
        }
        out
    }
}

/// The kind a file's extension says.
pub fn kind_of(path: &Path) -> Option<Kind> {
    match path.extension()?.to_str()? {
        "hmod" => Some(Kind::Mod),
        "hnmod" => Some(Kind::NativeMod),
        "hpack" => Some(Kind::Modpack),
        "zip" => Some(Kind::ResourcePack),
        "hshader" => Some(Kind::Shader),
        _ => None,
    }
}

/// Checks the file at `path` as `kind`; `key` and `signature` (the
/// `.minisig` text) check its signature.
pub fn check(
    path: &Path,
    kind: Kind,
    key: Option<&str>,
    signature: Option<&str>,
) -> Result<Report, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    if size > index::MAX_FILE {
        return Err(format!("over {} MiB", index::MAX_FILE >> 20));
    }
    index::open(&kind)?;
    let facts = index::inspect(&kind, path)?;
    let signed = match (key, signature) {
        (Some(key), Some(sig)) => {
            index::verify_signature(key, sig, path)?;
            true
        }
        (None, None) => false,
        _ => return Err("--key and --signature go together".into()),
    };
    let mut report = Report {
        kind,
        facts,
        sha256: index::sha256_file(path)?,
        size,
        signed,
        code: false,
        imports: Vec::new(),
        commands: Vec::new(),
        output: Vec::new(),
    };
    if report.kind == Kind::Mod {
        trial_run(path, &mut report)?;
    }
    Ok(report)
}

/// Runs a mod in the sandbox, as the game does, from a scratch copy so its
/// storage lands in a folder removed afterwards. A content mod has no code to
/// run: `inspect` already read its content as the game does.
fn trial_run(path: &Path, report: &mut Report) -> Result<(), String> {
    let package = Package::read(path)?;
    let Some(wasm) = package.wasm.as_deref() else {
        return Ok(());
    };
    report.code = true;
    report.imports = component_imports(wasm)?;
    package
        .manifest
        .check_imports(report.imports.iter().map(String::as_str))?;
    let dir = std::env::temp_dir().join(format!("horizon-mod-check-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let copy = dir.join("mod.hmod");
    let result = std::fs::copy(path, &copy)
        .map_err(|e| e.to_string())
        .and_then(|_| play(&copy, report));
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn play(path: &Path, report: &mut Report) -> Result<(), String> {
    let mut mods = Mods::load(&[path.to_path_buf()], |_| true, |_| false);
    let failed = |mods: &Mods| match &mods.entries()[0].status {
        Status::Failed(reason) => Some(reason.clone()),
        _ => None,
    };
    if let Some(reason) = failed(&mods) {
        return Err(format!("it didn't start: {reason}"));
    }
    let client: Vec<String> = mods.commands().map(str::to_owned).collect();
    let server: Vec<String> = mods.server_commands().map(|c| c.name.clone()).collect();
    report.commands = [client.clone(), server.clone()].concat();
    // A server mod plays with one player on, who joins, chats and leaves.
    let steve = ServerPlayer {
        id: "00000000-0000-4000-8000-000000000001".into(),
        name: "Steve".into(),
        x: 0.5,
        y: 64.0,
        z: 0.5,
        dimension: "minecraft:overworld".into(),
        game_mode: GameMode::Survival,
        health: 20.0,
    };
    let snapshot = |t: u64| Snapshot {
        player: Some(Player {
            x: 0.5 + t as f64 * 0.1,
            y: 64.0,
            z: 0.5,
            yaw: 90.0,
            pitch: 0.0,
            health: 20.0,
            food: 20,
            dimension: "minecraft:overworld".into(),
        }),
        clock: Some(Clock {
            game_time: t,
            day_time: 1000 + t,
        }),
        players: vec![steve.clone()],
        // The GUI of a 1280 x 720 window at scale 3.
        gui_size: (427.0, 240.0),
        ..Snapshot::default()
    };
    mods.set_snapshot(snapshot(0));
    mods.player_joined(&steve);
    for t in 0..TICKS {
        mods.set_snapshot(snapshot(t));
        mods.tick(t);
        mods.server_tick(t);
        for _ in 0..FRAMES_PER_TICK {
            mods.frame(1.0 / 60.0);
        }
    }
    mods.chat("<Steve> hello");
    mods.player_chat(&steve, "hello");
    for name in &client {
        mods.command(name);
    }
    for name in &server {
        mods.server_command(&steve, name);
    }
    mods.set_snapshot(Snapshot::default());
    mods.player_left(&steve);
    for out in mods.drain() {
        report.output.push(match out {
            Output::Chat { text, .. } => format!("chat: {text}"),
            Output::Tell { text, .. } => format!("tell: {text}"),
            Output::Broadcast { text, .. } => format!("broadcast: {text}"),
            Output::Feedback { text, error } | Output::Reply { text, error, .. } => {
                format!("{}: {text}", if error { "error" } else { "feedback" })
            }
            Output::Command { line } => format!("command: /{line}"),
            Output::QuietCommands { lines, .. } => {
                format!("{} quiet command(s)", lines.len())
            }
            Output::Stopped { reason, .. } => return Err(format!("it stopped: {reason}")),
            Output::SetBlocks {
                dimension, blocks, ..
            } => {
                format!("set {} block(s) in {dimension}", blocks.len())
            }
            Output::OpenScreen { .. } => "opened a screen".to_string(),
            Output::CloseScreen { .. } => "closed its screen".to_string(),
            Output::PlaceRecipe { book_id, all } => {
                format!("placed recipe {book_id}{}", if all { " (all)" } else { "" })
            }
            Output::Give { item, count, .. } => format!("gave {count} {item}"),
            Output::Shake { strength, seconds } => {
                format!("shook the camera: {strength:.2} for {seconds:.1} s")
            }
        });
    }
    let shapes = mods.hud_shapes().count();
    if shapes > 0 {
        report.output.push(format!("drew {shapes} HUD shape(s)"));
    }
    match failed(&mods) {
        Some(reason) => Err(format!("it stopped: {reason}")),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../horizon-mods/tests/index")
            .join(name)
    }

    #[test]
    fn checks_each_kind_its_own_way() {
        let pack = fixture("cozy-1.0.0.hpack");
        let report = check(&pack, kind_of(&pack).unwrap(), None, None).unwrap();
        assert_eq!(report.facts.requires[0].id, "coords_hud");
        assert!(
            report
                .text()
                .contains("cozy 1.0.0 (modpack, format 1, for 26.3)")
        );
        // The fixture mod's code is a stand-in: its manifest reads, its run fails.
        let hmod = fixture("coords_hud-0.1.0.hmod");
        let key = std::fs::read_to_string(fixture("test.pub")).unwrap();
        let sig = std::fs::read_to_string(fixture("coords_hud-0.1.0.hmod.minisig")).unwrap();
        let key = key.lines().last().unwrap();
        let e = check(&hmod, Kind::Mod, Some(key), Some(&sig))
            .err()
            .unwrap();
        assert!(e.contains("didn't start"), "{e}");
        let e = check(
            &hmod,
            Kind::Mod,
            Some(key),
            Some("untrusted comment: x\nRUQ=\n"),
        )
        .err()
        .unwrap();
        assert!(e.contains("signature"), "{e}");
        let e = check(&pack, Kind::Shader, None, None).err().unwrap();
        assert!(e.contains("horizon-shaders.toml"), "{e}");
        assert!(check(&pack, Kind::Mod, None, None).is_err());
    }
}
