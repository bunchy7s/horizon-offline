//! `horizon-mod.toml`, a mod's manifest, and the `.hmod` file around it.
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    io::Read,
    path::{Path, PathBuf},
};

/// The manifest's file name, beside `mod.wasm`.
pub const MANIFEST: &str = "horizon-mod.toml";
/// The component's file name in a `.hmod` or a mod folder.
pub const WASM: &str = "mod.wasm";
/// The API (WIT package `horizon:mods`) versions this build runs: a mod
/// built for any 0.1.x SDK.
pub const API: &str = "0.1";
/// The manifest `api` lines this build loads. 0.2 is 0.1 plus content:
/// `assets/` and `data/` in the `.hmod`, with blocks, items, creative tabs, mob effects,
/// entity types and menu types declared in `data/<id>/horizon/` (docs/mods.md). Code still links API 0.1.
/// 0.3 is a server mod (experimental): the `horizon-server-mod` world.
pub const APIS: [&str; 3] = [API, "0.2", SERVER_API];
/// The line of server mods, experimental: `side = "server"`.
pub const SERVER_API: &str = "0.3";
/// Namespaces no mod may claim: vanilla's, the convention tags' (mods add
/// to those, docs/mods.md) and the game's own.
const RESERVED_NAMESPACES: [&str; 3] = ["minecraft", "c", "horizon"];
/// A mod's data (capability `storage`), in `horizon-mods/` beside the mods
/// (`find` never mistakes it for a mod: it has no manifest).
pub const STORAGE_DIR: &str = ".horizon-mod-data";
/// The biggest `mod.wasm` a mod may ship.
const MAX_WASM: u64 = 32 << 20;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// `[a-z0-9_]`, 1 to 64 characters: the mod's folder, log and settings key.
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    /// The API line the mod was built for: `0.1`, or `0.2` for content.
    pub api: String,
    #[serde(default)]
    pub side: Side,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    /// Namespaces besides `id` that the mod's `assets/` and `data/` use, for
    /// a content mod whose files sit under other names than its id (Towns
    /// and Towers ships `kaisyn` and `towns_and_towers`). No two content mods
    /// that are on may share one (`content::clash`).
    #[serde(default)]
    pub namespaces: Vec<String>,
    /// The experimental capabilities the mod opts into (`["ui"]`): each one
    /// it lists in `capabilities` must be named here too, since it may change
    /// in any release (docs/api/stability.md). One no longer experimental is
    /// accepted here, so the manifest keeps loading once it is promoted.
    #[serde(default)]
    pub experimental: Vec<Capability>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// Runs in the player's game only (HUD, keys, chat commands).
    #[default]
    Client,
    /// Runs where the world is simulated (rules, blocks): `api = "0.3"`.
    Server,
    /// Both, and the server tells clients which version it expects.
    Both,
}

/// What a mod may do beyond writing its log. Each is a WIT interface the
/// component imports; the player sees them before turning the mod on.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    GameRead,
    Chat,
    Hud,
    Storage,
    /// Runs vanilla commands as the player, with the player's permissions.
    Command,
    /// New blocks, items and mob effects from the mod's `data/` (API 0.2): no WIT
    /// interface, the game reads them before the world loads.
    Registries,
    /// A screen the mod owns and a settings page in Options > Mods, drawn
    /// with the menus' own widgets (interface `ui`, calls `ui-events`).
    Ui,
    /// Shapes on the HUD and the camera's shake, drawn by the game's own
    /// renderer (interface `render`).
    Render,
    /// The use button on items the mod claims, and its own keys
    /// (interface `input`).
    Input,
    /// The players on the server and messages to them (server mods).
    Players,
    /// Blocks in the server's loaded chunks, and changes to them (server
    /// mods; client mods later, where the game runs the world).
    World,
    /// The recipes and items the game knows, for recipe viewers
    /// (interface `recipes`).
    Recipes,
}
impl Capability {
    pub const ALL: [Capability; 12] = [
        Capability::GameRead,
        Capability::Chat,
        Capability::Hud,
        Capability::Storage,
        Capability::Command,
        Capability::Registries,
        Capability::Ui,
        Capability::Render,
        Capability::Input,
        Capability::Players,
        Capability::World,
        Capability::Recipes,
    ];
    /// Whether a server mod may have it: the others act on a local player.
    pub fn on_server(self) -> bool {
        matches!(
            self,
            Capability::Storage | Capability::Players | Capability::World
        )
    }
    /// The WIT interface it grants, in `horizon:mods`.
    pub fn interface(self) -> &'static str {
        match self {
            Capability::GameRead => "game",
            Capability::Chat => "chat",
            Capability::Hud => "hud",
            Capability::Storage => "storage",
            Capability::Command => "command",
            Capability::Registries => "registries",
            Capability::Ui => "ui",
            Capability::Render => "render",
            Capability::Input => "input",
            Capability::Players => "players",
            Capability::World => "world",
            Capability::Recipes => "recipes",
        }
    }
    /// May change in any release: a mod uses it only with
    /// `experimental = ["<name>"]` in its manifest.
    pub fn is_experimental(self) -> bool {
        matches!(
            self,
            Capability::Ui | Capability::Render | Capability::Input | Capability::Recipes
        )
    }
    /// What the player reads before turning the mod on: "May <this>".
    pub fn describe(self) -> &'static str {
        match self {
            Capability::GameRead => "see your position, health and the time",
            Capability::Chat => "show you chat messages",
            Capability::Hud => "draw text on your screen",
            Capability::Storage => "save its own settings to disk",
            Capability::Command => "run commands as you, with your permissions",
            Capability::Registries => {
                "add blocks, items, mob effects and entity types to your worlds"
            }
            Capability::Ui => "show its own screens and a settings page",
            Capability::Render => "draw on your screen, shake and zoom the camera",
            Capability::Input => {
                "see the keys you hold and your clicks, take over the use button on its own items and the mouse wheel, and add keys"
            }
            Capability::Players => "see who is on the server and message them",
            Capability::World => "read and change the world's blocks",
            Capability::Recipes => {
                "read the game's recipes and items and which recipes you know, place recipes in your crafting grid and, in creative mode, give you items"
            }
        }
    }
    /// The capability a component import needs, `Ok(None)` for one every
    /// mod has; `Err` for an import Horizon does not offer.
    pub fn of_import(name: &str) -> Result<Option<Capability>, String> {
        let unknown = || format!("imports {name}, which Horizon does not offer");
        let rest = name.strip_prefix("horizon:mods/").ok_or_else(unknown)?;
        let (interface, version) = rest.split_once('@').ok_or_else(unknown)?;
        if !version.starts_with("0.1.") {
            return Err(format!("needs API {version}; this Horizon runs {API}"));
        }
        if interface == "log" {
            return Ok(None);
        }
        // A recipe viewer's panel is part of capability `ui`.
        if interface == "browser" {
            return Ok(Some(Capability::Ui));
        }
        Capability::ALL
            .into_iter()
            .filter(|c| *c != Capability::Registries)
            .find(|c| c.interface() == interface)
            .map(Some)
            .ok_or_else(unknown)
    }
}
impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Capability::GameRead => "game-read",
            Capability::Chat => "chat",
            Capability::Hud => "hud",
            Capability::Storage => "storage",
            Capability::Command => "command",
            Capability::Registries => "registries",
            Capability::Ui => "ui",
            Capability::Render => "render",
            Capability::Input => "input",
            Capability::Players => "players",
            Capability::World => "world",
            Capability::Recipes => "recipes",
        };
        f.write_str(name)
    }
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, String> {
        let m: Manifest = toml::from_str(text).map_err(|e| format!("{MANIFEST}: {e}"))?;
        let id_ok = (1..=64).contains(&m.id.len())
            && m.id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !id_ok {
            return Err(format!(
                "{MANIFEST}: id {:?} is not [a-z0-9_]{{1,64}}",
                m.id
            ));
        }
        if !APIS.contains(&m.api.as_str()) {
            return Err(format!(
                "built for API {}; this Horizon runs {}",
                m.api,
                APIS.join(" and ")
            ));
        }
        if m.capabilities.contains(&Capability::Registries) && m.api == API {
            return Err(format!(
                "{MANIFEST}: capability registries needs api = \"0.2\""
            ));
        }
        let server = m.side == Side::Server;
        if server != (m.api == SERVER_API) {
            return Err(format!(
                "{MANIFEST}: a server mod is side = \"server\" with api = \"{SERVER_API}\", and only it"
            ));
        }
        if let Some(c) = m
            .capabilities
            .iter()
            .find(|c| c.on_server() != server && **c != Capability::Storage)
        {
            let side = if server { "client" } else { "server" };
            return Err(format!("{MANIFEST}: capability {c} is for {side} mods"));
        }
        if let Some(c) = m
            .capabilities
            .iter()
            .find(|c| c.is_experimental() && !m.experimental.contains(c))
        {
            return Err(format!(
                "{MANIFEST}: capability {c} is experimental and may change in any release: \
                 add experimental = [\"{c}\"] to use it"
            ));
        }
        if !m.namespaces.is_empty() && !m.capabilities.contains(&Capability::Registries) {
            return Err(format!(
                "{MANIFEST}: namespaces are for content, which needs capability registries"
            ));
        }
        for (i, ns) in m.namespaces.iter().enumerate() {
            // Vanilla's namespace characters, at most as long as an id.
            let chars_ok = (1..=64).contains(&ns.len())
                && ns
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b));
            if !chars_ok {
                return Err(format!(
                    "{MANIFEST}: namespace {ns:?} is not [a-z0-9_.-]{{1,64}}"
                ));
            }
            if RESERVED_NAMESPACES.contains(&ns.as_str()) {
                return Err(format!(
                    "{MANIFEST}: namespace {ns} is not a mod's to claim (only to add to its tags)"
                ));
            }
            if *ns == m.id || m.namespaces[..i].contains(ns) {
                return Err(format!("{MANIFEST}: namespace {ns} is listed twice"));
            }
        }
        Ok(m)
    }
    /// The namespaces the mod's content owns: its id, then `namespaces`.
    pub fn owned_namespaces(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.id.as_str()).chain(self.namespaces.iter().map(String::as_str))
    }
    /// Checks that the component's imports need only declared capabilities.
    pub fn check_imports<'a>(
        &self,
        imports: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), String> {
        for name in imports {
            if let Some(c) = Capability::of_import(name)?
                && !self.capabilities.contains(&c)
            {
                return Err(format!("uses capability {c}, missing from {MANIFEST}"));
            }
        }
        Ok(())
    }
}

/// A mod as found on disk: its manifest and component bytes.
pub struct Package {
    pub manifest: Manifest,
    /// None for a content-only mod (capability `registries`, no code).
    pub wasm: Option<Vec<u8>>,
}
impl Package {
    /// A `.hmod` (a zip holding `horizon-mod.toml`, `mod.wasm` and, for
    /// content, `assets/` and `data/`) or a folder holding the same.
    pub fn read(path: &Path) -> Result<Self, String> {
        let (toml, wasm) = if path.is_dir() {
            let toml = std::fs::read_to_string(path.join(MANIFEST));
            let wasm = path.join(WASM);
            let wasm = match wasm.exists() {
                true => Some(std::fs::read(wasm).map_err(|e| format!("{WASM}: {e}"))?),
                false => None,
            };
            (toml.map_err(|e| format!("{MANIFEST}: {e}"))?, wasm)
        } else {
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
            let mut entry = |name: &str| -> Result<Option<Vec<u8>>, String> {
                let f = match zip.by_name(name) {
                    Ok(f) => f,
                    Err(zip::result::ZipError::FileNotFound) => return Ok(None),
                    Err(e) => return Err(format!("{name}: {e}")),
                };
                let mut out = Vec::new();
                f.take(MAX_WASM + 1)
                    .read_to_end(&mut out)
                    .map_err(|e| e.to_string())?;
                Ok(Some(out))
            };
            let toml = entry(MANIFEST)?.ok_or_else(|| format!("{MANIFEST}: missing"))?;
            let toml = String::from_utf8(toml).map_err(|_| format!("{MANIFEST}: not UTF-8"))?;
            (toml, entry(WASM)?)
        };
        if wasm.as_ref().is_some_and(|w| w.len() as u64 > MAX_WASM) {
            return Err(format!("{WASM} is over {} MiB", MAX_WASM >> 20));
        }
        let manifest = Manifest::parse(&toml)?;
        if wasm.is_none() && !manifest.capabilities.contains(&Capability::Registries) {
            return Err(format!(
                "{WASM}: missing (only a content mod, with capability registries, may have no code)"
            ));
        }
        Ok(Package { manifest, wasm })
    }
    /// Whether the mod at `path` (a `.hmod` or a folder) has a [`MANIFEST`]: content, or a legacy
    /// WebAssembly mod. A `.hmod` holding only native code has none.
    pub fn has_manifest(path: &Path) -> bool {
        if path.is_dir() {
            return path.join(MANIFEST).is_file();
        }
        std::fs::File::open(path)
            .ok()
            .and_then(|f| zip::ZipArchive::new(f).ok())
            .is_some_and(|mut zip| zip.by_name(MANIFEST).is_ok())
    }
    /// The `.hmod` bytes for `manifest_text` and a component.
    pub fn write(manifest_text: &str, wasm: &[u8]) -> Result<Vec<u8>, String> {
        Self::write_files(manifest_text, Some(wasm), &[])
    }
    /// The `.hmod` bytes for `manifest_text`, a component if the mod has
    /// code, and content `files` (`assets/...`, `data/...`) by path.
    pub fn write_files(
        manifest_text: &str,
        wasm: Option<&[u8]>,
        files: &[(&str, &[u8])],
    ) -> Result<Vec<u8>, String> {
        use std::io::Write;
        Manifest::parse(manifest_text)?;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        let err = |e: zip::result::ZipError| e.to_string();
        let mut put = |name: &str, bytes: &[u8]| -> Result<(), String> {
            zip.start_file(name, options).map_err(err)?;
            zip.write_all(bytes).map_err(|e| e.to_string())
        };
        put(MANIFEST, manifest_text.as_bytes())?;
        if let Some(wasm) = wasm {
            put(WASM, wasm)?;
        }
        for (name, bytes) in files {
            crate::content::check_path(name)?;
            put(name, bytes)?;
        }
        Ok(zip.finish().map_err(err)?.into_inner())
    }
}

/// The `.hmod` files and mod folders in `dir`, by name.
pub fn find(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "hmod") || p.join(MANIFEST).is_file())
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
id = "coords_hud"
name = "Coordinates HUD"
version = "0.1.0"
api = "0.1"
capabilities = ["game-read", "hud"]
"#;

    #[test]
    fn parses_and_checks_imports() {
        let m = Manifest::parse(EXAMPLE).unwrap();
        assert_eq!(m.side, Side::Client);
        assert_eq!(m.capabilities, [Capability::GameRead, Capability::Hud]);
        let ok = [
            "horizon:mods/log@0.1.0",
            "horizon:mods/game@0.1.0",
            "horizon:mods/hud@0.1.2",
        ];
        assert_eq!(m.check_imports(ok), Ok(()));
        let chat = m.check_imports(["horizon:mods/chat@0.1.0"]);
        assert_eq!(
            chat,
            Err("uses capability chat, missing from horizon-mod.toml".into())
        );
        assert!(m.check_imports(["wasi:filesystem/types@0.2.0"]).is_err());
        assert!(m.check_imports(["horizon:mods/game@0.2.0"]).is_err());
        let storage = m.check_imports(["horizon:mods/storage@0.1.0"]);
        assert_eq!(
            storage,
            Err("uses capability storage, missing from horizon-mod.toml".into())
        );
        assert_eq!(
            m.check_imports(["horizon:mods/command@0.1.0"]),
            Err("uses capability command, missing from horizon-mod.toml".into())
        );
        let with_storage = EXAMPLE.replace("\"hud\"", "\"hud\", \"storage\"");
        let m = Manifest::parse(&with_storage).unwrap();
        assert_eq!(m.check_imports(["horizon:mods/storage@0.1.0"]), Ok(()));
    }

    #[test]
    fn refuses_bad_ids_and_apis() {
        let bad_id = EXAMPLE.replace("coords_hud", "Coords HUD");
        assert!(Manifest::parse(&bad_id).unwrap_err().contains("is not"));
        let bad_api = EXAMPLE.replace("api = \"0.1\"", "api = \"0.4\"");
        assert!(Manifest::parse(&bad_api).unwrap_err().contains("API 0.4"));
        let unknown = format!("{EXAMPLE}network = true\n");
        assert!(Manifest::parse(&unknown).is_err());
    }

    #[test]
    fn server_mods_are_api_0_3_with_server_capabilities() {
        let server = |api: &str, side: &str, caps: &str| {
            Manifest::parse(&format!(
                "id = \"w\"\nname = \"W\"\nversion = \"1\"\napi = \"{api}\"\nside = \"{side}\"\ncapabilities = [{caps}]\n"
            ))
        };
        let m = server("0.3", "server", "\"players\", \"storage\"").unwrap();
        let imports = ["horizon:mods/players@0.1.0", "horizon:mods/storage@0.1.0"];
        assert_eq!(m.check_imports(imports), Ok(()));
        for (api, side) in [("0.3", "client"), ("0.3", "both"), ("0.1", "server")] {
            let e = server(api, side, "").unwrap_err();
            assert!(e.contains("a server mod is side"), "{e}");
        }
        let e = server("0.3", "server", "\"hud\"").unwrap_err();
        assert!(e.contains("capability hud is for client mods"), "{e}");
        let e = server("0.1", "client", "\"players\"").unwrap_err();
        assert!(e.contains("capability players is for server mods"), "{e}");
    }

    #[test]
    fn checks_extra_namespaces() {
        let content = "id = \"t_and_t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n";
        let with = |list: &str| Manifest::parse(&format!("{content}namespaces = [{list}]\n"));
        let m = with("\"kaisyn\", \"towns_and_towers\"").unwrap();
        let owned: Vec<&str> = m.owned_namespaces().collect();
        assert_eq!(owned, ["t_and_t", "kaisyn", "towns_and_towers"]);
        for reserved in ["minecraft", "c", "horizon"] {
            let e = with(&format!("\"{reserved}\"")).unwrap_err();
            assert!(e.contains("not a mod's to claim"), "{e}");
        }
        assert!(with("\"t_and_t\"").unwrap_err().contains("twice"));
        assert!(with("\"a\", \"a\"").unwrap_err().contains("twice"));
        assert!(with("\"Kaisyn\"").unwrap_err().contains("is not"));
        assert!(with("\"\"").unwrap_err().contains("is not"));
        let code = format!("{EXAMPLE}namespaces = [\"kaisyn\"]\n");
        assert!(Manifest::parse(&code).unwrap_err().contains("registries"));
    }

    #[test]
    fn hmod_round_trips() {
        let dir = std::env::temp_dir().join(format!("horizon-mods-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("coords_hud.hmod");
        std::fs::write(&path, Package::write(EXAMPLE, b"\0asm").unwrap()).unwrap();
        let p = Package::read(&path).unwrap();
        assert_eq!(p.manifest.id, "coords_hud");
        assert_eq!(p.wasm.as_deref(), Some(&b"\0asm"[..]));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn experimental_capabilities_need_the_opt_in() {
        let base = "id = \"m\"\nname = \"M\"\nversion = \"1\"\napi = \"0.1\"\n";
        let err = Manifest::parse(&format!("{base}capabilities = [\"ui\"]\n")).unwrap_err();
        assert!(err.contains("experimental = [\"ui\"]"), "{err}");
        let m = Manifest::parse(&format!(
            "{base}capabilities = [\"ui\"]\nexperimental = [\"ui\"]\n"
        ))
        .unwrap();
        assert_eq!(m.experimental, [Capability::Ui]);
        // A capability that is (or became) stable is fine in the list.
        Manifest::parse(&format!(
            "{base}capabilities = [\"hud\"]\nexperimental = [\"hud\"]\n"
        ))
        .unwrap();
    }
}
