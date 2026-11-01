//! Modpacks: import a Java modpack (a Modrinth `.mrpack`, a CurseForge zip, an instance folder),
//! show what Horizon runs of it, port that on this computer from the player's own downloads, and
//! play it as a profile of its own (docs/modpacks.md).
//!
//! Each pack lives in `ports/<slug>/` in the game's home: the work folder `horizon-port-pack`
//! writes (pack.json, inventory, the jars, the carried-over mods, the coverage report) and
//! `home/`, the profile's own saves and options. Nothing is uploaded or shared: mod jars are
//! often All Rights Reserved, so the ports stay on the player's machine.
use crate::backend;
use crate::browser::horizon::pack as horizon_pack;
use horizon_port_pack::{
    fetch::{Fetched, How},
    import::Pack,
    pipeline::{self, Log},
    report::{Row, Status},
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where imported packs live.
pub fn ports_dir() -> PathBuf {
    backend::game_home().join("ports")
}

/// A pack's folder name: its name in lowercase words joined by dashes.
pub fn slug(name: &str) -> String {
    let words: Vec<String> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    if words.is_empty() {
        "pack".into()
    } else {
        words.join("-")
    }
}

/// An imported pack, as its work folder has it.
pub struct Imported {
    pub slug: String,
    pub work: PathBuf,
    pub pack: Pack,
    pub rows: Vec<Row>,
    pub fetched: Fetched,
    /// The carried-over mods the profile loads.
    pub mods: Vec<PathBuf>,
    /// Mods left off, with why: a ported one an earlier one owns a namespace of, or one whose
    /// jar only CurseForge serves (CurseForge downloads coming soon).
    pub left_off: Vec<String>,
    /// The mods the port step gets and carries over (content, data tier).
    pub portable: usize,
}

impl Imported {
    pub fn title(&self) -> &str {
        if self.pack.name.is_empty() {
            &self.slug
        } else {
            &self.pack.name
        }
    }
    pub fn count(&self, status: Status) -> usize {
        self.rows.iter().filter(|r| r.status == status).count()
    }
    /// "Minecraft 1.21.1 · neoforge 21.1.259 · 496 mods".
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.pack.minecraft.is_empty() {
            parts.push(format!("Minecraft {}", self.pack.minecraft));
        }
        if !self.pack.loader.is_empty() {
            parts.push(format!("{} {}", self.pack.loader, self.pack.loader_version));
        }
        parts.push(format!("{} files", self.pack.files.len()));
        parts.join("  ·  ")
    }
    /// What getting the jars did, in one line, or None before it ran.
    pub fn jars(&self) -> Option<String> {
        if self.fetched.files.is_empty() {
            return None;
        }
        let mut parts = Vec::new();
        for (how, what) in [
            (How::Downloaded, "downloaded"),
            (How::Local, "copied from your folder"),
            (How::Present, "already here"),
            (
                How::NeedsKey,
                "only on CurseForge (CurseForge downloads coming soon: choose your installed \
                 instance instead)",
            ),
            (How::NotFound, "not found"),
            (How::Failed, "failed"),
        ] {
            let n = self.fetched.count(how);
            if n > 0 {
                parts.push(format!("{n} {what}"));
            }
        }
        Some(format!("Jars: {}.", parts.join(", ")))
    }
}

/// Reads the pack in `work`.
pub fn load(work: &Path) -> Result<Imported, String> {
    let pack: Pack = pipeline::read_json(&work.join("pack.json"))?;
    let rows: Vec<Row> = pipeline::read_json(&work.join("coverage.json")).unwrap_or_default();
    let records: Vec<horizon_port_pack::inventory::Record> =
        pipeline::read_json(&work.join("inventory.json")).unwrap_or_default();
    let (mods, mut left_off) = pipeline::loadable(&carried(work));
    let fetched = Fetched::load(work);
    left_off.extend(
        fetched
            .files
            .iter()
            .filter(|o| o.how == How::NeedsKey)
            .map(|o| format!("{}: CurseForge downloads coming soon", o.key)),
    );
    Ok(Imported {
        slug: work
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        work: work.to_path_buf(),
        pack,
        rows,
        fetched,
        mods,
        left_off,
        portable: records.iter().filter(|r| pipeline::portable(r)).count(),
    })
}

/// Every imported pack, by name.
pub fn list() -> Vec<Imported> {
    list_in(&ports_dir())
}

pub(crate) fn list_in(ports: &Path) -> Vec<Imported> {
    let mut packs: Vec<Imported> = fs::read_dir(ports)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| load(&e.path()).ok())
        .collect();
    packs.sort_by_key(|p| p.title().to_ascii_lowercase());
    packs
}

/// The carried-over mods in a work folder (`mods/<id>/carry/` with its manifest).
pub fn carried(work: &Path) -> Vec<PathBuf> {
    let mut mods: Vec<PathBuf> = fs::read_dir(work.join("mods"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("carry"))
        .filter(|c| c.join("horizon-mod.toml").is_file())
        .collect();
    mods.sort();
    mods
}

/// Imports the pack at `path` into `ports/<slug>/` and writes its coverage report from
/// metadata (Modrinth answers by file hash; nothing is downloaded yet). Importing the same pack
/// again keeps the jars and ports it already has.
pub fn import(path: &Path, log: Log) -> Result<Imported, String> {
    let pack = horizon_port_pack::import::import(path)?;
    let name = if pack.name.is_empty() || pack.name == "modlist" {
        path.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        pack.name.clone()
    };
    let work = ports_dir().join(slug(&name));
    log(format!("Importing {name} into {}", work.display()));
    pipeline::import(path, &work, log)?;
    pipeline::inventory(&work, false, log)?;
    pipeline::report(&work, log)?;
    load(&work)
}

/// Ports what this computer can: the portable mods' jars (from `from` when the player has them,
/// else the pack's links or Modrinth), carried over for `resources`, then the report again.
pub fn port(
    work: &Path,
    resources: Option<&Path>,
    from: Option<&Path>,
    log: Log,
) -> Result<Imported, String> {
    pipeline::port(work, resources, from, log)?;
    load(work)
}

/// What the mod browser does with a modpack: imports it, then ports what this computer can
/// (from the pack's links and Modrinth), so it plays straight away. A port that fails leaves the
/// pack imported; the error says so.
pub fn import_and_port(
    path: &Path,
    resources: Option<&Path>,
    log: Log,
) -> Result<Imported, String> {
    let pack = import(path, log)?;
    if pack.portable == 0 {
        return Ok(pack);
    }
    port(&pack.work, resources, None, log)
        .map_err(|e| format!("{} is imported, but porting it failed: {e}", pack.title()))
}

/// One line on what an import and port gave: the mods carried over and the ones that weren't.
pub fn outcome(pack: &Imported) -> String {
    let not = pack.count(Status::NotYet);
    format!(
        "{} is imported{}: {} of its {} files play in Horizon ({} ported, {} partly), {} don't \
         yet, {} aren't needed{}. Its list says why.",
        pack.title(),
        if pack.mods.is_empty() {
            ""
        } else {
            " and ported"
        },
        pack.count(Status::Ported) + pack.count(Status::Partial),
        pack.rows.len(),
        pack.count(Status::Ported),
        pack.count(Status::Partial),
        not,
        pack.count(Status::NotNeeded),
        match pack.left_off.len() {
            0 => String::new(),
            n => format!("; {n} are left off (listed under the pack)"),
        },
    )
}

/// Removes an imported pack: its jars, ports and the profile's saves.
pub fn remove(work: &Path) -> Result<(), String> {
    if horizon_pack::is_pack(work) {
        return horizon_pack::remove(work);
    }
    if work.parent() != Some(ports_dir().as_path()) {
        return Err(format!("{} is not an imported pack", work.display()));
    }
    fs::remove_dir_all(work).map_err(|e| format!("{}: {e}", work.display()))
}

/// The profile's own game home: saves, options.txt, logs.
pub fn profile_home(work: &Path) -> PathBuf {
    work.join("home")
}

/// Turns the game's command into this pack's profile: its own home (the player's options.txt
/// copied in the first time, so keys and video settings carry over) and its carried-over mods.
/// A modpack from Horizon's registry (`horizon-packs/<id>/`) plays its own profile the same way.
pub fn apply_profile(cmd: &mut Command, work: &Path) -> Result<(), String> {
    if horizon_pack::is_pack(work) {
        return horizon_pack::apply(cmd, work);
    }
    // Only mods that load together: the game would refuse the later of two sharing a namespace.
    let (mods, _) = pipeline::loadable(&carried(work));
    if mods.is_empty() {
        return Err("Nothing is ported yet: port the pack first.".into());
    }
    let home = profile_home(work);
    fs::create_dir_all(&home).map_err(|e| format!("{}: {e}", home.display()))?;
    let options = home.join("options.txt");
    let own = backend::game_home().join("options.txt");
    if !options.exists() && own.is_file() {
        let _ = fs::copy(own, &options);
    }
    cmd.current_dir(&home).env("HORIZON_HOME", &home);
    for m in mods {
        cmd.arg("--dev-mod").arg(m);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("All the Mods 10"), "all-the-mods-10");
        assert_eq!(slug("Create: Above & Beyond!"), "create-above-beyond");
        assert_eq!(slug("  "), "pack");
    }

    #[test]
    fn jars_only_curseforge_serves_are_left_off_as_coming_soon() {
        use horizon_port_pack::fetch::Outcome;
        let work = std::env::temp_dir().join(format!("horizon-launcher-cf-{}", std::process::id()));
        let _ = fs::remove_dir_all(&work);
        fs::create_dir_all(&work).unwrap();
        pipeline::write_json(&work.join("pack.json"), &Pack::default()).unwrap();
        let fetched = Fetched {
            files: vec![Outcome {
                key: "allthemodium".into(),
                file: "mods/allthemodium.jar".into(),
                how: How::NeedsKey,
                detail: String::new(),
            }],
        };
        pipeline::write_json(&work.join("fetch.json"), &fetched).unwrap();
        let pack = load(&work).unwrap();
        assert_eq!(
            pack.left_off,
            ["allthemodium: CurseForge downloads coming soon"]
        );
        assert!(pack
            .jars()
            .unwrap()
            .contains("CurseForge downloads coming soon"));
        fs::remove_dir_all(&work).unwrap();
    }

    #[test]
    fn a_profile_has_its_own_home_and_mods() {
        let root =
            std::env::temp_dir().join(format!("horizon-launcher-pack-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let work = root.join("ports/small");
        let mut cmd = Command::new("true");
        assert!(apply_profile(&mut cmd, &work).is_err());
        for (id, namespaces) in [("a", ""), ("b", ""), ("d", "\"a\"")] {
            let carry = work.join("mods").join(id).join("carry");
            fs::create_dir_all(&carry).unwrap();
            fs::write(
                carry.join("horizon-mod.toml"),
                format!(
                    "id = \"{id}\"\nname = \"{id}\"\nversion = \"1\"\napi = \"0.2\"\n\
                     capabilities = [\"registries\"]\nnamespaces = [{namespaces}]\n"
                ),
            )
            .unwrap();
        }
        // Left out: no manifest (c), and one owning a namespace of an earlier mod (d).
        fs::create_dir_all(work.join("mods/c/carry")).unwrap();
        apply_profile(&mut cmd, &work).unwrap();
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args.len(), 4);
        assert_eq!(
            Path::new(&args[1]),
            work.join("mods").join("a").join("carry")
        );
        assert_eq!(
            Path::new(&args[3]),
            work.join("mods").join("b").join("carry")
        );
        assert_eq!(cmd.get_current_dir(), Some(profile_home(&work).as_path()));
        assert!(profile_home(&work).is_dir());
        fs::remove_dir_all(&root).unwrap();
    }
}
