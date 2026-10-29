//! Mods and resource packs for a Java Edition instance: the source's own files, for the
//! instance's Minecraft version and loader, put in its `mods/` and `resourcepacks/` as they are
//! (nothing is ported). The instance's `mod-browser.json` lists them, as for Horizon's folder.
use super::installed::{self, Entry, Installed, Step};
use super::{details, Kind, Project, Query, Source, Version};
use std::fs;
use std::path::Path;

/// How deep required dependencies are followed.
const MAX_DEPTH: usize = 4;

/// What an instance runs: its Minecraft version and its loader (`fabric`, `quilt`, `neoforge`,
/// `forge`), none for vanilla.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub minecraft: String,
    pub loader: Option<String>,
}

impl Target {
    /// Whether a file made for `loader` runs here. Quilt loads Fabric mods
    /// (<https://quiltmc.org/en/about/faq/>), so a Quilt instance takes them too.
    pub fn accepts(&self, loader: &str) -> bool {
        match self.loader.as_deref() {
            Some(l) => l == loader || (l == "quilt" && loader == "fabric"),
            None => false,
        }
    }

    /// The folder files of `kind` go in, or why they don't go in this instance.
    pub fn folder(&self, kind: Kind) -> Result<&'static str, String> {
        match kind {
            Kind::Mod if self.loader.is_none() => Err(format!(
                "This instance plays vanilla Minecraft {}: give it a loader (Fabric, Quilt...) \
                 for mods.",
                self.minecraft
            )),
            Kind::Mod => Ok("mods"),
            Kind::ResourcePack => Ok("resourcepacks"),
            Kind::Modpack => Err("A modpack is imported as its own instance.".into()),
            Kind::DataPack => Err("A data pack goes in a world.".into()),
            Kind::Shader => Err("Shader packs for Java instances aren't supported yet.".into()),
        }
    }

    /// A search of `kind` for what runs here: made for its version and, for mods, its loader.
    pub fn query(&self, text: &str, kind: Kind) -> Query {
        let mut query = Query::new(text, kind);
        query.filters = details::Filters {
            game_version: Some(self.minecraft.clone()),
            loader: match kind {
                Kind::Mod => self.loader.clone(),
                _ => None,
            },
            ..details::Filters::default()
        };
        query
    }

    /// The page's search: the player's categories, side and sort, for this instance's version
    /// and loader (in place of any the player picked).
    pub fn search(&self, text: &str, kind: Kind, picked: &details::Filters) -> Query {
        let mut query = self.query(text, kind);
        query.filters = details::Filters {
            categories: picked.categories.clone(),
            environment: picked.environment,
            sort: picked.sort,
            ..query.filters
        };
        query
    }

    /// Whether this instance takes files of `kind` from the browser as they are: mods and
    /// resource packs.
    pub fn takes(&self, kind: Kind) -> bool {
        matches!(kind, Kind::Mod | Kind::ResourcePack)
    }

    /// What it plays, as a search with no results names it: `Minecraft 1.21.1 with fabric`.
    pub fn describe(&self, kind: Kind) -> String {
        match (&self.loader, kind) {
            (Some(l), Kind::Mod) => format!("Minecraft {} with {l}", self.minecraft),
            _ => format!("Minecraft {}", self.minecraft),
        }
    }

    /// The newest version made for this instance, a file for its own loader before one it
    /// only accepts (a Fabric file in a Quilt instance).
    pub fn pick<'a>(&self, kind: Kind, versions: &'a [Version]) -> Option<&'a Version> {
        let mut usable = versions.iter().filter(|v| {
            v.blocked.is_none() && v.file().is_some() && v.game_versions.contains(&self.minecraft)
        });
        if kind != Kind::Mod {
            return usable.find(|v| v.loaders.iter().any(|l| l == "minecraft"));
        }
        let own = self.loader.as_deref()?;
        usable
            .clone()
            .find(|v| v.loaders.iter().any(|l| l == own))
            .or_else(|| usable.find(|v| v.loaders.iter().any(|l| self.accepts(l))))
    }
}

/// What installing `project` into the instance takes, its required dependencies first, each
/// checked against the instance before any byte is downloaded.
pub fn plan(source: &dyn Source, target: &Target, project: &Project) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    add(source, target, project.clone(), false, 0, &mut steps)?;
    Ok(steps)
}

fn add(
    source: &dyn Source,
    target: &Target,
    project: Project,
    dependency: bool,
    depth: usize,
    steps: &mut Vec<Step>,
) -> Result<(), String> {
    if steps.iter().any(|s| s.project.id == project.id) {
        return Ok(());
    }
    if depth > MAX_DEPTH {
        return Err(format!("{}'s dependencies go too deep.", project.title));
    }
    target.folder(project.kind)?;
    let versions = source.versions(&project)?;
    let Some(version) = target.pick(project.kind, &versions).cloned() else {
        return Err(match versions.iter().find_map(|v| v.blocked.clone()) {
            Some(why) => format!("{} can't be installed from here: {why}", project.title),
            None => format!(
                "{} has no version for Minecraft {}{}.",
                project.title,
                target.minecraft,
                match (project.kind, &target.loader) {
                    (Kind::Mod, Some(l)) => format!(" with {l}"),
                    _ => String::new(),
                }
            ),
        });
    };
    for id in &version.required {
        let needed = source
            .project(id)
            .map_err(|e| format!("{} needs {id}: {e}", project.title))?;
        add(source, target, needed, true, depth + 1, steps)?;
    }
    steps.push(Step {
        project,
        version,
        dependency,
    });
    Ok(())
}

/// Installs `project` (and what it needs) into the instance folder `home`; one already there is
/// updated to the version chosen.
pub fn install(
    source: &dyn Source,
    target: &Target,
    project: &Project,
    home: &Path,
    progress: &dyn Fn(&str, f32),
) -> Result<Vec<Entry>, String> {
    let steps = plan(source, target, project)?;
    let folder = |kind| target.folder(kind).expect("plan checks every kind");
    let was = Installed::load(home).entries;
    let done = installed::put(source, steps, home, &folder, progress)?;
    // A mod the player turned off stays off through an update, and its old file goes.
    for e in &done {
        let Some(old) = was
            .iter()
            .find(|o| o.source == e.source && o.project == e.project && !is_on(home, o))
        else {
            continue;
        };
        if old.file != e.file {
            let _ = fs::remove_file(off(home, old));
        }
        set_on(home, e, false)?;
    }
    Ok(done)
}

/// A mod turned off keeps its file under this suffix, which the loaders don't read: they load
/// the `.jar` files in `mods/`.
const OFF: &str = ".disabled";

fn off(home: &Path, entry: &Entry) -> std::path::PathBuf {
    home.join(format!("{}{OFF}", entry.file))
}

/// Whether the game loads `entry`: everything but a mod turned off.
pub fn is_on(home: &Path, entry: &Entry) -> bool {
    entry.kind != Kind::Mod || !off(home, entry).is_file()
}

/// Turns the mod `entry` on or off by renaming its file; it stays installed.
pub fn set_on(home: &Path, entry: &Entry, on: bool) -> Result<(), String> {
    if entry.kind != Kind::Mod {
        return Err(format!(
            "{} isn't a mod: only mods turn on and off.",
            entry.title
        ));
    }
    let (jar, off) = (home.join(&entry.file), off(home, entry));
    let (from, to) = if on { (off, jar) } else { (jar, off) };
    if !from.exists() && to.is_file() {
        return Ok(());
    }
    fs::rename(&from, &to).map_err(|e| format!("{}: {e}", from.display()))
}

/// The newer version `entry` can be updated to, if any.
pub fn update_for(
    source: &dyn Source,
    target: &Target,
    entry: &Entry,
) -> Result<Option<Version>, String> {
    let project = source.project(&entry.project)?;
    let versions = source.versions(&project)?;
    Ok(target
        .pick(project.kind, &versions)
        .filter(|v| v.id != entry.version)
        .cloned())
}

/// Removes what the browser installed in the instance for `name` (a project id or slug).
pub fn remove(home: &Path, name: &str) -> Result<Entry, String> {
    let mut installed = Installed::load(home);
    let entry = installed
        .lookup(name)
        .cloned()
        .ok_or_else(|| format!("{name} wasn't installed by the browser."))?;
    for path in [home.join(&entry.file), off(home, &entry)] {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    installed
        .entries
        .retain(|e| !(e.source == entry.source && e.project == entry.project));
    installed.save(home)?;
    Ok(entry)
}

#[cfg(test)]
mod tests;
