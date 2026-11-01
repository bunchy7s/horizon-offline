//! Instances: separate game folders, each with its own worlds, options and packs.
//!
//! Nothing that exists moves. The game's home (`backend::game_home`) stays the default instance,
//! "Horizon", with its saves and options where they are. Imported modpacks (`ports/<pack>/home/`)
//! and registry modpacks (`horizon-packs/<id>/home/`) are instances too, played from their own
//! folders as before. New instances get `instances/<slug>/` in the game's home. What the player
//! chooses about an instance (its name, icon, group) is kept in `instances.json` in the
//! launcher's data folder, so renaming one never moves a folder.
use crate::backend;
use crate::browser::horizon::pack as registry_pack;
use crate::launch::Launch;
use crate::modpacks;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The default instance's id: the game's home itself.
pub const DEFAULT: &str = "horizon";
const FILE: &str = "instances.json";
/// Where new instances live, in the game's home.
const OWN: &str = "instances";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The game's home: what every player had before instances.
    Default,
    /// Made in the launcher: `instances/<slug>/`.
    Own,
    /// An imported Java modpack (modpacks.rs).
    Port,
    /// A modpack from Horizon's registry.
    Pack,
}

/// What an instance shows in the sidebar: a block or item texture from the player's prepared
/// files (never shipped), or a colour.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Icon {
    /// A texture name: `grass_block` (block/ is looked in first, then item/).
    Texture(String),
    /// `#rrggbb`.
    Colour(String),
}

impl Icon {
    /// `#3fa7d6` is a colour; a texture's name is lowercase letters, digits and `_`.
    pub fn parse(text: &str) -> Result<Icon, String> {
        let text = text.trim();
        if let Some(hex) = text.strip_prefix('#') {
            if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Ok(Icon::Colour(format!("#{}", hex.to_ascii_lowercase())));
            }
        } else if !text.is_empty()
            && text
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Ok(Icon::Texture(text.into()));
        }
        Err(format!(
            "{text}: give a block or item name (grass_block) or a colour (#3fa7d6)"
        ))
    }

    /// The texture's file in prepared resources, if it is a texture they hold.
    pub fn texture(&self, resources: &Path) -> Option<PathBuf> {
        let Icon::Texture(name) = self else {
            return None;
        };
        let textures = resources.join("assets/minecraft/textures");
        ["block", "item"]
            .iter()
            .map(|kind| textures.join(kind).join(format!("{name}.png")))
            .find(|p| p.is_file())
    }
}

/// An instance's own picture, in its game folder: one the player chose, or its modpack's icon.
/// Drawn instead of [`Meta::icon`] while it is there.
pub const PICTURE: &str = "icon.png";
/// How many pixels a side a picture is kept at: twice the largest it is drawn.
pub const PICTURE_SIDE: u32 = 128;

/// Any image file's `bytes` as a picture: PNG, scaled down to fit [`PICTURE_SIDE`].
pub fn picture(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    crate::browser::images::fit_png(bytes, PICTURE_SIDE, 8192, 256 << 20).ok_or_else(|| {
        "That file isn't a picture the launcher can read (PNG, JPEG, GIF, WebP).".into()
    })
}

/// Keeps `png` (made by [`picture`]) as the picture of the instance playing in `home`, or, with
/// None, gives its picture to `to_trash`.
pub fn set_picture(
    home: &Path,
    png: Option<&[u8]>,
    to_trash: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let target = home.join(PICTURE);
    let Some(png) = png else {
        return if target.is_file() {
            to_trash(&target)
        } else {
            Ok(())
        };
    };
    fs::create_dir_all(home).map_err(|e| format!("{}: {e}", home.display()))?;
    let part = home.join("icon.png.part");
    fs::write(&part, png).map_err(|e| format!("{}: {e}", part.display()))?;
    fs::rename(&part, &target).map_err(|e| format!("{}: {e}", target.display()))
}

/// Makes the icon at `url` (a modpack's, from the source it was installed from) the picture of the
/// instance playing in `home`, unless it has one already. Best effort: a pack without its icon
/// still plays.
pub fn adopt_icon(home: &Path, url: &str) {
    if home.join(PICTURE).exists() {
        return;
    }
    let image = crate::browser::images::fetch(&backend::data_dir(), url, PICTURE_SIDE);
    let png = image
        .ok()
        .and_then(|f| fs::read(f).ok())
        .and_then(|b| picture(b).ok());
    if let Some(png) = png {
        let _ = set_picture(home, Some(&png), &|_| Ok(()));
    }
}

/// The player's choices about one instance.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// The Horizon version this instance plays, kept when newer ones are installed. None: the
    /// launcher's own version, which follows updates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Seconds played in it, every run started by the launcher (the window or `play`) added up.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub played_secs: u64,
    /// When its last run ended, in seconds since 1970.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_played: Option<u64>,
    /// What runs around the game: a pre-launch command, a wrapper, variables (`launch`).
    #[serde(default, skip_serializing_if = "Launch::is_empty")]
    pub launch: Launch,
    /// Set for a Minecraft: Java Edition instance: it plays the real game, not Horizon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java: Option<JavaGame>,
}

/// A Minecraft: Java Edition instance: the real game of one version, with the Java it runs on
/// (`crate::java` picks one when `home` is none).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JavaGame {
    /// The Minecraft version, as Mojang names it (`1.21.4`).
    pub minecraft: String,
    /// A Java home to run it on instead of the automatic pick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<PathBuf>,
    /// The most memory the game may use, in MiB (`-Xmx`); none: [`DEFAULT_MEMORY_MB`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u32>,
    /// Extra JVM arguments, split on whitespace.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub jvm_args: String,
    /// The mod loader it plays with, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader: Option<crate::loaders::Loader>,
    /// The game window's size, width then height; none: the game's own 854×480.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<(u32, u32)>,
}

impl Instance {
    /// Its variables for the game started as `program` and the commands around it
    /// ([`crate::launch::instance_vars`]).
    pub fn vars(&self, program: &std::ffi::OsStr) -> crate::launch::Vars {
        let java = self.java.as_ref().map(|_| program);
        crate::launch::instance_vars(&self.name, &self.id, &self.home, java)
    }
}

impl JavaGame {
    /// What the mod browser installs for: its Minecraft version and its loader.
    pub fn target(&self) -> crate::browser::java_instance::Target {
        crate::browser::java_instance::Target {
            minecraft: self.minecraft.clone(),
            loader: self.loader.as_ref().map(|l| l.kind.id().to_string()),
        }
    }
}

/// 4 GiB: enough for vanilla and most modpacks, as other launchers start with.
pub const DEFAULT_MEMORY_MB: u32 = 4096;

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// `instances.json`: each instance's choices, by id.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    instances: BTreeMap<String, Meta>,
}

impl Store {
    fn load(data: &Path) -> Store {
        fs::read_to_string(data.join(FILE))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }
    fn save(&self, data: &Path) -> Result<(), String> {
        fs::create_dir_all(data).map_err(|e| format!("{}: {e}", data.display()))?;
        let text = serde_json::to_string_pretty(self).unwrap();
        fs::write(data.join(FILE), text).map_err(|e| format!("{FILE}: {e}"))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    /// `horizon`, an own instance's folder name, `port:<pack>` or `pack:<id>`.
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// The folder the pack's tools work in: the game's home for Default and Own instances.
    pub work: PathBuf,
    /// The game folder it plays in: saves, options.txt, packs.
    pub home: PathBuf,
    pub icon: Option<Icon>,
    pub group: Option<String>,
    /// Its pinned Horizon version, if any (`Meta::version`).
    pub version: Option<String>,
    pub played: Duration,
    pub last_played: Option<SystemTime>,
    /// What runs around the game (`launch::wrap`).
    pub launch: Launch,
    /// The Minecraft: Java Edition it plays, for a Java instance.
    pub java: Option<JavaGame>,
}

impl Instance {
    /// Ok when it plays Horizon; for a Java instance, which starts its own game
    /// (`backend::java_game`), that Horizon's options don't apply to it.
    fn horizon(&self) -> Result<(), String> {
        match &self.java {
            None => Ok(()),
            Some(java) => Err(format!(
                "{} plays Minecraft {} (Java Edition), not Horizon: Play starts it, without a \
                 Horizon version.",
                self.name, java.minecraft
            )),
        }
    }

    /// The installed version this instance plays: its pinned one, else `requested` (the
    /// command line's `--version`) or the launcher's (`backend::resolve_version`).
    pub fn resolve_version(&self, requested: Option<&str>) -> Result<String, String> {
        self.horizon()?;
        match (requested, &self.version) {
            (None, Some(pinned)) => backend::resolve_version(Some(pinned)).map_err(|_| {
                format!(
                    "{} plays {pinned}, which is not installed: install it, or horizon-launcher \
                     instance version \"{}\" latest",
                    self.name, self.name
                )
            }),
            _ => backend::resolve_version(requested),
        }
    }
}

/// The instances, by the default game home and the launcher's data folder: Horizon first, then
/// by group and name.
pub fn list() -> Vec<Instance> {
    list_in(&backend::game_home(), &backend::data_dir())
}

fn list_in(game_home: &Path, data: &Path) -> Vec<Instance> {
    let mut store = Store::load(data);
    let mut found = vec![(
        DEFAULT.to_string(),
        "Horizon".to_string(),
        Kind::Default,
        game_home.to_path_buf(),
        game_home.to_path_buf(),
    )];
    let mut own: Vec<_> = fs::read_dir(game_home.join(OWN))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    own.sort();
    // An own instance whose folder is gone (deleted outside the launcher) is forgotten, so a new
    // one of that name starts clean. Only when the folder of instances itself can be read.
    if game_home.join(OWN).is_dir() {
        let before = store.instances.len();
        store
            .instances
            .retain(|id, _| id == DEFAULT || id.contains(':') || own.contains(id));
        if store.instances.len() != before {
            let _ = store.save(data);
        }
    }
    for slug in own {
        let dir = game_home.join(OWN).join(&slug);
        found.push((slug.clone(), slug, Kind::Own, dir.clone(), dir));
    }
    for p in modpacks::list_in(&game_home.join("ports")) {
        let home = modpacks::profile_home(&p.work);
        found.push((
            format!("port:{}", p.slug),
            p.title().into(),
            Kind::Port,
            p.work,
            home,
        ));
    }
    for p in registry_pack::list_in(&game_home.join("horizon-packs")) {
        let home = registry_pack::home(&p.work);
        found.push((format!("pack:{}", p.id), p.name, Kind::Pack, p.work, home));
    }
    let mut out: Vec<Instance> = found
        .into_iter()
        .map(|(id, name, kind, work, home)| {
            let meta = store.instances.get(&id).cloned().unwrap_or_default();
            Instance {
                name: meta.name.unwrap_or(name),
                icon: meta.icon,
                group: meta.group,
                version: meta.version,
                played: Duration::from_secs(meta.played_secs),
                last_played: meta
                    .last_played
                    .map(|s| UNIX_EPOCH + Duration::from_secs(s)),
                launch: meta.launch,
                java: meta.java,
                id,
                kind,
                work,
                home,
            }
        })
        .collect();
    out[1..].sort_by_key(|i| (i.group.clone(), i.name.to_lowercase()));
    out
}

/// The instance an id, a name (any case) or a pack's folder name names.
pub fn find(query: &str) -> Result<Instance, String> {
    find_in(&list(), query)
}

fn find_in(all: &[Instance], query: &str) -> Result<Instance, String> {
    let q = query.trim();
    let bare = |id: &str| id.split_once(':').map_or(id, |(_, rest)| rest).to_string();
    all.iter()
        .find(|i| i.id == q)
        .or_else(|| all.iter().find(|i| i.name.eq_ignore_ascii_case(q)))
        .or_else(|| all.iter().find(|i| bare(&i.id) == q))
        .cloned()
        .ok_or_else(|| format!("No instance {q}: horizon-launcher instance list shows them"))
}

/// Moves an instance's folder to the Trash (where the player can get it back, worlds and all)
/// and forgets it: its choices, and the selection when Play started it. Horizon's own instance,
/// the game's home, stays.
pub fn delete(query: &str) -> Result<Instance, String> {
    let gone = delete_in(
        &backend::game_home(),
        &backend::data_dir(),
        query,
        &crate::desktop_seams::to_trash,
    )?;
    let mut settings = backend::Settings::load();
    if settings.instance.as_ref() == Some(&gone.id) {
        settings.instance = None;
        settings.save();
    }
    Ok(gone)
}

fn delete_in(
    game_home: &Path,
    data: &Path,
    query: &str,
    trash: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<Instance, String> {
    let instance = find_in(&list_in(game_home, data), query)?;
    if instance.kind == Kind::Default {
        return Err(format!(
            "{} is the game's own folder: it can't be deleted.",
            instance.name
        ));
    }
    trash(&instance.work)
        .map_err(|e| format!("{} could not go to the Trash: {e}", instance.name))?;
    let mut store = Store::load(data);
    if store.instances.remove(&instance.id).is_some() {
        store.save(data)?;
    }
    Ok(instance)
}

/// The instance Play starts: the one picked in the launcher, else Horizon.
pub fn selected() -> Instance {
    let all = list();
    backend::Settings::load()
        .instance
        .and_then(|id| all.iter().find(|i| i.id == id).cloned())
        .unwrap_or_else(|| all[0].clone())
}

pub fn select(query: &str) -> Result<Instance, String> {
    let instance = find(query)?;
    let mut settings = backend::Settings::load();
    settings.instance = (instance.kind != Kind::Default).then(|| instance.id.clone());
    settings.save();
    Ok(instance)
}

/// What a copy of a game folder leaves out: other instances and packs, logs and crash reports,
/// caches, data collection's state, and sign-in sessions (a session never leaves its folder).
const NOT_COPIED: &[&str] = &[
    OWN,
    "ports",
    "horizon-packs",
    "logs",
    "crash-reports",
    "sessions",
    "telemetry",
    "mod-browser",
    ".horizon-cache",
];

/// Files of the player, not of one instance, that a new instance starts with: the keys and video
/// settings, and the offline name (so the player stays the same player in its worlds).
const PLAYER_FILES: &[&str] = &["options.txt", "horizon-player.txt"];

/// Creates an instance named `name`: empty but for the player's options and offline name, or a
/// copy of `from`'s game folder (its worlds, options and packs). Nothing of `from` changes.
pub fn create(name: &str, from: Option<&str>) -> Result<Instance, String> {
    let from = from.map(find).transpose()?;
    create_in(
        &backend::game_home(),
        &backend::data_dir(),
        name,
        from.as_ref(),
        None,
    )
}

/// Creates a Minecraft: Java Edition instance named `name` playing `minecraft` (a version id
/// from Mojang's list), with an empty game folder: the real game keeps its own options.
pub fn create_java(name: &str, minecraft: &str) -> Result<Instance, String> {
    create_in(
        &backend::game_home(),
        &backend::data_dir(),
        name,
        None,
        Some(minecraft),
    )
}

fn create_in(
    game_home: &Path,
    data: &Path,
    name: &str,
    from: Option<&Instance>,
    minecraft: Option<&str>,
) -> Result<Instance, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("An instance needs a name.".into());
    }
    let all = list_in(game_home, data);
    if all.iter().any(|i| i.name.eq_ignore_ascii_case(name)) {
        return Err(format!("There is already an instance called {name}."));
    }
    if let Some(from) = from.filter(|f| matches!(f.kind, Kind::Port | Kind::Pack)) {
        return Err(format!(
            "{} plays its pack's mods: import the pack again for a second copy.",
            from.name
        ));
    }
    let base = modpacks::slug(name);
    let taken = |s: &str| s == DEFAULT || game_home.join(OWN).join(s).exists();
    let slug = (1..)
        .map(|n| {
            if n == 1 {
                base.clone()
            } else {
                format!("{base}-{n}")
            }
        })
        .find(|s| !taken(s))
        .unwrap();
    let dir = game_home.join(OWN).join(&slug);
    let made = match from {
        Some(from) => copy_home(&from.home, &dir),
        None if minecraft.is_some() => fs::create_dir_all(&dir),
        None => fs::create_dir_all(&dir).and_then(|()| {
            for f in PLAYER_FILES {
                let own = game_home.join(f);
                if own.is_file() {
                    fs::copy(own, dir.join(f))?;
                }
            }
            Ok(())
        }),
    };
    if let Err(e) = made {
        let _ = fs::remove_dir_all(&dir);
        return Err(format!("Could not make {}: {e}", dir.display()));
    }
    let mut store = Store::load(data);
    let meta = store.instances.entry(slug.clone()).or_default();
    meta.name = (name != slug).then(|| name.to_string());
    meta.icon = from.and_then(|f| f.icon.clone());
    meta.group = from.and_then(|f| f.group.clone());
    meta.java = from.and_then(|f| f.java.clone()).or_else(|| {
        minecraft.map(|v| JavaGame {
            minecraft: v.to_string(),
            ..JavaGame::default()
        })
    });
    store.save(data)?;
    find_in(&list_in(game_home, data), &slug)
}

fn copy_home(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if NOT_COPIED.iter().any(|n| name == *n) {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            copy_dir(&path, &to.join(&name))?;
        } else {
            fs::copy(&path, to.join(&name))?;
        }
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Changes what the player chose about an instance: its name, icon or group. No folder moves.
pub fn edit(query: &str, change: impl FnOnce(&mut Meta)) -> Result<Instance, String> {
    edit_in(&backend::game_home(), &backend::data_dir(), query, change)
}

fn edit_in(
    game_home: &Path,
    data: &Path,
    query: &str,
    change: impl FnOnce(&mut Meta),
) -> Result<Instance, String> {
    let all = list_in(game_home, data);
    let instance = find_in(&all, query)?;
    let mut store = Store::load(data);
    let meta = store.instances.entry(instance.id.clone()).or_default();
    change(meta);
    meta.name = meta.name.take().map(|n| n.trim().to_string());
    meta.group = meta
        .group
        .take()
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty());
    if let Some(name) = &meta.name {
        if name.is_empty() {
            return Err("An instance needs a name.".into());
        }
        // Only a new name is checked: an import can share a name, and changing anything
        // else about it must still work.
        let renamed = !name.eq_ignore_ascii_case(&instance.name);
        if renamed
            && all
                .iter()
                .any(|i| i.id != instance.id && i.name.eq_ignore_ascii_case(name))
        {
            return Err(format!("There is already an instance called {name}."));
        }
    }
    store.save(data)?;
    find_in(&list_in(game_home, data), &instance.id)
}

/// Adds a finished run of the game to the instance's play time (by id: a renamed instance keeps
/// it).
pub fn record_play(id: &str, played: Duration) -> Result<(), String> {
    record_play_in(&backend::data_dir(), id, played, SystemTime::now())
}

fn record_play_in(
    data: &Path,
    id: &str,
    played: Duration,
    ended: SystemTime,
) -> Result<(), String> {
    let mut store = Store::load(data);
    let meta = store.instances.entry(id.to_string()).or_default();
    meta.played_secs = meta.played_secs.saturating_add(played.as_secs());
    meta.last_played = ended.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs());
    store.save(data)
}

/// "12 min", "3 h 05 min", "140 h": the play time as players read it.
pub fn play_time(played: Duration) -> String {
    let minutes = played.as_secs() / 60;
    match (minutes / 60, minutes % 60) {
        (0, 0) => crate::i18n::tr("under a minute").into(),
        (0, m) => format!("{m} min"),
        (h, m) if h < 100 => format!("{h} h {m:02} min"),
        (h, _) => format!("{h} h"),
    }
}

/// Turns the game's command into this instance: its own game folder, and a pack's mods. The
/// player's answers that hold everywhere follow it in: data collection and No AI as the
/// launcher has them, and options.txt and the offline name when it has none yet.
pub fn apply(cmd: &mut Command, instance: &Instance) -> Result<(), String> {
    instance.horizon()?;
    match instance.kind {
        Kind::Default => return Ok(()),
        Kind::Own => {
            fs::create_dir_all(&instance.home)
                .map_err(|e| format!("{}: {e}", instance.home.display()))?;
            cmd.current_dir(&instance.home)
                .env("HORIZON_HOME", &instance.home);
        }
        Kind::Port | Kind::Pack => modpacks::apply_profile(cmd, &instance.work)?,
    }
    follow_player(&backend::game_home(), &instance.home);
    Ok(())
}

fn follow_player(game_home: &Path, home: &Path) {
    for f in PLAYER_FILES {
        if !home.join(f).exists() && game_home.join(f).is_file() {
            let _ = fs::copy(game_home.join(f), home.join(f));
        }
    }
    let consent = horizon_telemetry::consent::Consent::path(game_home);
    if consent.is_file() {
        let to = horizon_telemetry::consent::Consent::path(home);
        let _ = fs::create_dir_all(to.parent().unwrap());
        let _ = fs::copy(consent, to);
    }
    let no_ai = horizon_core::no_ai::is_on(game_home);
    if horizon_core::no_ai::is_on(home) != no_ai {
        let _ = horizon_core::no_ai::set(home, no_ai);
    }
}

/// A fresh made-up name everywhere the game keeps one: `horizon-player.txt` is per game
/// folder (`PLAYER_FILES`), so the same new name goes to the default home and every
/// instance's — the player stays one player in its worlds. The file's `name:` line and the
/// `Player` + five digits scheme are horizon-client's (`offline_name.rs`).
pub fn new_player_name() -> Result<String, String> {
    new_player_name_in(&backend::game_home(), &backend::data_dir())
}

fn new_player_name_in(game_home: &Path, data: &Path) -> Result<String, String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let name = format!(
        "Player{:05}",
        (nanos ^ u64::from(std::process::id()).rotate_left(32)) % 100_000
    );
    for instance in list_in(game_home, data) {
        if instance.home.is_dir() {
            fs::write(
                instance.home.join("horizon-player.txt"),
                format!("name:{name}\n"),
            )
            .map_err(|e| format!("{}: {e}", instance.home.display()))?;
        }
    }
    Ok(name)
}

#[cfg(test)]
#[path = "instances/tests.rs"]
mod tests;
