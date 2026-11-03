//! Mods in the game (`docs/mods.md`): content mods' blocks, items, data and
//! assets (`content_root`), and the Mods page's rows. Code mods are native
//! (`native_hooks.rs`); WebAssembly code mods don't run any more.
use super::Game;
use horizon_client::{
    mod_list::{self, Installed},
    text::Text,
};
use std::path::PathBuf;

#[derive(Default)]
pub(super) struct ModsState {
    /// `--dev-mod PATH`: always on.
    pub(super) dev: Vec<PathBuf>,
    /// The WebAssembly mods found that no longer run, for the one-time notice.
    stopped: Vec<String>,
}

/// Freeze native component IDs before reading content or opening a connection.
/// With no mod registering any, nothing is installed, which every reader
/// treats as no custom components, and `registries.json` isn't read.
pub(super) fn register_components(root: &std::path::Path) -> horizon_client::Result<()> {
    let mut names = Vec::new();
    Game::native_component_types(&mut names);
    if names.is_empty() {
        return Ok(());
    }
    let path =
        horizon_protocol::registry::Builtin::find(root).ok_or("registries.json not found")?;
    let builtin = horizon_protocol::registry::Builtin::load(&path)?;
    let types = builtin
        .get("minecraft:data_component_type")
        .ok_or("missing data component registry")?;
    let first = i32::try_from(types.names.len())?;
    horizon_protocol::item::custom::install(first, names)
        .map_err(|error| format!("invalid native component registration: {error}"))?;
    Ok(())
}

/// The blocks and items native mods declare in code (`native_blocks`,
/// `native_items`): asked once, so every content root of this run, a pack
/// reload's included, has the same ones.
fn native_declared() -> &'static (
    Vec<horizon_mods::content::Block>,
    Vec<horizon_mods::content::Item>,
) {
    static DECLARED: std::sync::OnceLock<(
        Vec<horizon_mods::content::Block>,
        Vec<horizon_mods::content::Item>,
    )> = std::sync::OnceLock::new();
    DECLARED.get_or_init(|| {
        let (mut blocks, mut items) = (Vec::new(), Vec::new());
        Game::native_blocks(&mut blocks);
        Game::native_items(&mut items);
        (blocks, items)
    })
}
/// The native mods' standalone models (`native_standalone_models`), set once
/// before the resources first load.
pub(super) fn register_standalone_models() {
    let mut names = Vec::new();
    Game::native_standalone_models(&mut names);
    horizon_client::models::register_standalone(names);
}

/// `root` with the content mods that are on layered over it
/// (docs/modpacks.md): their blocks, items, data and assets. Read once at
/// start, before the resources load; a mod whose content is refused is left
/// out here and its row on the Mods page says why.
pub(super) fn content_root(
    root: &std::path::Path,
    enabled: &std::collections::BTreeSet<String>,
    dev: &[std::path::PathBuf],
) -> std::path::PathBuf {
    use horizon_client::mod_content::{self, ContentMod};
    use horizon_mods::{Capability, Content, Package};
    let here = std::env::current_dir().unwrap_or_default();
    let mut paths = horizon_mods::find(&here.join(mod_list::DIR));
    paths.retain(|p| !dev.contains(p));
    paths.extend(dev.iter().cloned());
    let mut known = None;
    let mut mods: Vec<ContentMod> = Vec::new();
    // The first load's list stays (`set_local`): a reload does not hash the files again.
    let announce = !horizon_protocol::mod_list::is_set();
    let mut announced = Vec::new();
    for path in &paths {
        let Ok(package) = Package::read(path) else {
            continue;
        };
        let m = &package.manifest;
        let on = enabled.contains(&m.id) || dev.contains(path);
        if !on
            || !m.capabilities.contains(&Capability::Registries)
            || mods.iter().any(|c| c.id == m.id)
        {
            continue;
        }
        // The items this mod's native code declares may be a file item's crafting remainder.
        let declared: Vec<String> = native_declared()
            .1
            .iter()
            .filter(|i| i.id.split_once(':').is_some_and(|(ns, _)| ns == m.id))
            .map(|i| i.id.clone())
            .collect();
        let content = match Content::read_declaring(path, m, &declared) {
            Ok(content) => content,
            Err(e) => {
                log::warn!("Horizon: mod {} content not loaded: {e}", m.id);
                continue;
            }
        };
        let known = match &mut known {
            Some(types) => types,
            None => match mod_content::known_components(root) {
                Ok(types) => known.insert(types),
                Err(e) => {
                    log::warn!("Horizon: content mods not loaded: {e}");
                    register_mob_effects(&[]);
                    return root.to_owned();
                }
            },
        };
        let candidate = ContentMod {
            id: m.id.clone(),
            content,
        };
        match mod_content::accept_mod(&mut mods, candidate, known) {
            Ok(_) if announce => announced.push(announced_entry(m, path)),
            Ok(_) => {}
            Err(e) => log::warn!("Horizon: mod {} content not loaded: {e}", m.id),
        }
    }
    let (blocks, items) = native_declared();
    if !blocks.is_empty() || !items.is_empty() {
        if known.is_none() {
            match mod_content::known_components(root) {
                Ok(types) => known = Some(types),
                Err(e) => log::warn!("Horizon: native mods' blocks and items not loaded: {e}"),
            }
        }
        if let Some(known) = &known {
            for e in mod_content::accept_declared(&mut mods, blocks, items, known) {
                log::warn!("Horizon: {e}");
            }
            // A file item's remainder named a code-declared item that was refused: that mod
            // alone is left out, not every content mod through the overlay's check.
            for e in mod_content::drop_dangling_remainders(&mut mods) {
                log::warn!("Horizon: {e}");
            }
        }
    }
    match mod_content::resource_root(root, &mods, &here.join(".horizon-cache")) {
        Ok(out) => {
            if announce {
                horizon_protocol::mod_list::set_local(announced);
            }
            if out != root {
                log::info!(
                    "Horizon: content from {} mod(s) in {}",
                    mods.len(),
                    out.display()
                );
            }
            register_mob_effects(&mods);
            register_entity_types(&mods);
            register_menu_types(&mods);
            out
        }
        Err(e) => {
            log::warn!("Horizon: content mods not loaded: {e}");
            register_mob_effects(&[]);
            register_entity_types(&[]);
            register_menu_types(&[]);
            root.to_owned()
        }
    }
}

/// A loaded mod as a server tells joining clients (`horizon_protocol::mod_list`): its manifest's
/// id, version and side, and the SHA-256 of its `.hmod` (none for a folder).
fn announced_entry(
    m: &horizon_mods::Manifest,
    path: &std::path::Path,
) -> horizon_protocol::mod_list::Entry {
    use horizon_mods::Side;
    use horizon_protocol::mod_list;
    use sha2::{Digest, Sha256};
    let sha256 = if path.is_file() {
        std::fs::read(path)
            .ok()
            .map(|bytes| Sha256::digest(bytes).into())
    } else {
        None
    };
    mod_list::Entry {
        id: m.id.clone(),
        version: m.version.clone(),
        sha256,
        side: match m.side {
            Side::Both => mod_list::Side::Both,
            Side::Server => mod_list::Side::Server,
            Side::Client => mod_list::Side::Client,
        },
    }
}

/// The mods' mob effects join vanilla's (`brewing::effect`), once the root
/// holding their network ids is built. The startup root's effects, none
/// included, stay until restart: a pack reload after a Mods switch builds a
/// root with others.
fn register_mob_effects(mods: &[horizon_client::mod_content::ContentMod]) {
    use horizon_client::brewing::{self, Category};
    use horizon_mods::content::EffectCategory;
    let effects = horizon_client::mod_content::mob_effects(mods);
    let ids: Vec<String> = effects.iter().map(|e| e.id.clone()).collect();
    let set = brewing::register_mod_effects(effects.into_iter().map(|e| {
        let category = match e.category {
            EffectCategory::Beneficial => Category::Beneficial,
            EffectCategory::Harmful => Category::Harmful,
            EffectCategory::Neutral => Category::Neutral,
        };
        (e.id.clone(), e.color, category)
    }));
    if !set
        && brewing::mod_effects()
            .iter()
            .map(|e| e.id)
            .ne(ids.iter().map(String::as_str))
    {
        log::warn!("Horizon: the mods' mob effects changed; restart the game to use them");
    }
}

/// the mods' thrown entity types, as their mob effects: set once, from the
/// startup root.
fn register_entity_types(mods: &[horizon_client::mod_content::ContentMod]) {
    use horizon_client::projectiles;
    let types = horizon_client::mod_content::entity_types(mods);
    let set = projectiles::register_thrown_types(
        types
            .iter()
            .map(|t| (t.id.clone(), t.item.clone(), t.damage)),
    );
    if !set
        && projectiles::thrown_types()
            .iter()
            .map(|t| t.id)
            .ne(types.iter().map(|t| t.id.as_str()))
    {
        log::warn!("Horizon: the mods' entity types changed; restart the game to use them");
    }
}

/// the mods' menu types, as their entity types: set once, from the startup
/// root.
fn register_menu_types(mods: &[horizon_client::mod_content::ContentMod]) {
    use horizon_client::containers;
    let menus = horizon_client::mod_content::menu_types(mods);
    let set = containers::register_mod_menu_types(menus.iter().map(|m| {
        let results = m.results.iter().map(|&r| usize::from(r)).collect();
        (m.id.clone(), m.slots.into(), m.data_slots.into(), results)
    }));
    if !set
        && containers::mod_menu_types()
            .iter()
            .map(|m| m.id)
            .ne(menus.iter().map(|m| m.id.as_str()))
    {
        log::warn!("Horizon: the mods' menu types changed; restart the game to use them");
    }
}

/// What the launcher writes beside a game it built with native mods: their ids under `mods`.
const BUILD_INFO: &str = "native-mods.json";

/// `native-mods.json`'s `mods` list is not empty: the game was built with mods.
fn build_lists_mods(built: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(built)
        .ok()
        .and_then(|info| info["mods"].as_array().map(|mods| !mods.is_empty()))
        .unwrap_or(false)
}

/// Whether the game playing was built with native mods: the launcher writes their ids into
/// [`BUILD_INFO`] beside the executable, and a release or Nightly has no such file.
pub(super) fn built_with_native_mods() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| std::fs::read_to_string(exe.with_file_name(BUILD_INFO)).ok())
        .is_some_and(|built| build_lists_mods(&built))
}

/// The native mods the launcher built this game with, `id version, id version`, for the crash
/// report (`built`: its [`BUILD_INFO`]). None for a release or Nightly, which have no mods.
fn native_mods_detail(built: &str) -> Option<String> {
    let info: serde_json::Value = serde_json::from_str(built).unwrap_or_default();
    let list = info["mods"]
        .as_array()?
        .iter()
        .map(|m| {
            let text = |key: &str| m[key].as_str().unwrap_or("?").to_owned();
            format!("{} {}", text("id"), text("version"))
        })
        .collect::<Vec<_>>()
        .join(", ");
    (!list.is_empty()).then_some(list)
}

/// A row for each native mod the launcher installed in `dir` (`<id>.hnmod`) that this game wasn't
/// built with (`built`: its [`BUILD_INFO`], empty for a release or Nightly) and that has no row
/// yet. The page said "No mods" before, and players copied the files into `horizon-mods` (#4770).
/// Its toggle is the `mod.<id>` line the launcher's next build follows.
fn not_built(dir: &std::path::Path, built: &str, rows: &[Installed]) -> Vec<Installed> {
    let info: serde_json::Value = serde_json::from_str(built).unwrap_or_default();
    let in_build = |id: &str| {
        info["mods"]
            .as_array()
            .is_some_and(|mods| mods.iter().any(|m| m["id"] == id))
    };
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    paths.retain(|p| p.extension().is_some_and(|e| e == "hnmod"));
    paths.sort();
    let mut out: Vec<Installed> = Vec::new();
    for path in paths {
        let Ok(bundle) = horizon_mods::index::native::Bundle::read(&path) else {
            continue;
        };
        let m = bundle.manifest;
        let listed = rows.iter().chain(&out).any(|row| row.id == m.id);
        if !listed && !in_build(&m.id) {
            out.push(Installed {
                title: format!("{} {}", m.id, m.version),
                id: m.id,
                description: String::new(),
                problem: Some(mod_list::NOT_BUILT.to_owned()),
                dev: false,
                native: None,
            });
        }
    }
    out
}

/// A WebAssembly code mod's row on the Mods page.
const STOPPED: &str = "Doesn't run any more: needs a native port";
/// The player was told once about the WebAssembly mods that stopped (`horizon-mods/<this>`).
const NOTICE_SEEN: &str = ".native-notice";

impl Game<'_> {
    /// Lists the installed mods on the Mods page. Content mods load with the resources
    /// (`content_root`); native ones are built into the game (`native_hooks.rs`). WebAssembly code
    /// mods don't run any more: their rows say so, and the player hears it once in the chat
    /// (`tell_native_now`).
    pub(super) fn start_mods(&mut self) {
        let dir = Self::game_dir().join(mod_list::DIR);
        let mut paths = horizon_mods::find(&dir);
        paths.retain(|p| !self.mods.dev.contains(p));
        paths.extend(self.mods.dev.iter().cloned());
        self.settings.mods.installed = paths
            .iter()
            .map(|path| {
                let file = path.file_name().unwrap_or_default().to_string_lossy();
                let package = match horizon_mods::Package::read(path) {
                    Ok(package) => package,
                    Err(e) => {
                        return Installed {
                            id: String::new(),
                            title: file.into_owned(),
                            description: String::new(),
                            problem: Some(format!("Can't be read: {e}")),
                            dev: false,
                            native: None,
                        };
                    }
                };
                let m = &package.manifest;
                let problem = package.wasm.is_some().then(|| STOPPED.to_owned());
                Installed {
                    id: m.id.clone(),
                    title: format!("{} {}", m.name, m.version),
                    description: m.description.clone(),
                    problem,
                    dev: self.mods.dev.contains(path),
                    native: None,
                }
            })
            .collect();
        // A mod with a native version built in says so instead, and isn't news.
        self.list_native_mods();
        let built = std::env::current_exe()
            .ok()
            .and_then(|exe| std::fs::read_to_string(exe.with_file_name(BUILD_INFO)).ok())
            .unwrap_or_default();
        let built_mods = native_mods_detail(&built);
        let built_with_mods = built_mods.is_some();
        if let Some(list) = built_mods {
            horizon_client::crash::set_detail("Native Mods", list);
        }
        horizon_client::gfx::native_render::set_mods_built_in(
            built_with_mods
                || self
                    .settings
                    .mods
                    .installed
                    .iter()
                    .any(|m| m.native.is_some()),
        );
        let native_dir = Self::game_dir().join(mod_list::NATIVE_DIR);
        let unbuilt = not_built(&native_dir, &built, &self.settings.mods.installed);
        self.settings.mods.installed.extend(unbuilt);
        let installed = &self.settings.mods.installed;
        self.mods.stopped = installed
            .iter()
            .filter(|row| row.problem.as_deref() == Some(STOPPED))
            .map(|row| row.title.clone())
            .collect();
    }
    /// Once, in the first world after the update: which WebAssembly mods stopped.
    pub(super) fn tell_native_now(&mut self) {
        if self.mods.stopped.is_empty() || self.player.is_none() {
            return;
        }
        let names = std::mem::take(&mut self.mods.stopped).join(", ");
        let seen = Self::game_dir().join(mod_list::DIR).join(NOTICE_SEEN);
        if seen.exists() {
            return;
        }
        self.system_message(Text::literal(format!(
            "Horizon mods are native now: {names} stopped running until they have a native version. The launcher's Mods page has the native ones."
        )));
        let _ = std::fs::write(&seen, "");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Only a build whose `native-mods.json` lists a mod counts as built with mods.
    #[test]
    fn only_a_listed_mod_makes_a_build_a_mod_build() {
        assert!(!build_lists_mods(""));
        assert!(!build_lists_mods("{}"));
        assert!(!build_lists_mods(r#"{"mods":[]}"#));
        assert!(!build_lists_mods(r#"{"mods":"worldmap"}"#));
        assert!(build_lists_mods(r#"{"mods":[{"id":"worldmap"}]}"#));
    }

    /// A minimal `.hnmod` the registry accepts: its manifest and its mixin's.
    fn hnmod(dir: &std::path::Path, id: &str) {
        let horizon = "a".repeat(40);
        let manifest = serde_json::json!({
            "schema": 1, "id": id, "version": "0.3.0", "game": "26.3",
            "horizon": horizon, "mixins": "b".repeat(40),
        });
        let mixin = format!(
            "[mod]\nid = \"{id}\"\nversion = \"0.3.0\"\n[compatibility]\nhorizon_revision = \"{horizon}\"\n"
        );
        let file = std::fs::File::create(dir.join(format!("{id}.hnmod"))).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("horizon-native.json", options).unwrap();
        zip.write_all(manifest.to_string().as_bytes()).unwrap();
        zip.start_file(format!("mixins/{id}/horizon-mixin.toml"), options)
            .unwrap();
        zip.write_all(mixin.as_bytes()).unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn the_crash_report_lists_the_builds_native_mods() {
        let built = r#"{"mods":[{"id":"machine_sample","version":"0.1.0","sha256":null},{"id":"pipes","version":"2.0"}]}"#;
        assert_eq!(
            native_mods_detail(built).as_deref(),
            Some("machine_sample 0.1.0, pipes 2.0")
        );
        // A release has no file; a build without mods has an empty list.
        assert_eq!(native_mods_detail(""), None);
        assert_eq!(native_mods_detail(r#"{"mods":[]}"#), None);
    }

    #[test]
    fn native_mods_installed_but_not_built_get_a_row_saying_so() {
        let dir = std::env::temp_dir().join(format!("horizon-not-built-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for id in ["veinminer", "zoom", "minimap"] {
            hnmod(&dir, id);
        }
        std::fs::write(dir.join("notes.txt"), "not a mod").unwrap();
        std::fs::write(dir.join("broken.hnmod"), "not a zip").unwrap();
        // A release: none is built in; the page lists them, sorted, instead of "No mods".
        let rows = not_built(&dir, "", &[]);
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["minimap", "veinminer", "zoom"]);
        assert_eq!(rows[1].title, "veinminer 0.3.0");
        assert_eq!(rows[1].problem.as_deref(), Some(mod_list::NOT_BUILT));
        assert!(rows.iter().all(|r| r.native.is_none() && !r.dev));
        // A native build with zoom, and minimap's row already listed by its own hook.
        let built =
            r#"{"base":"nightly-2026-10-30-1da354b","mods":[{"id":"zoom","version":"0.3.0"}]}"#;
        let listed = Installed {
            id: "minimap".into(),
            ..rows[0].clone()
        };
        let rows = not_built(&dir, built, &[listed]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "veinminer");
        // No folder: no rows.
        assert!(not_built(&dir.join("missing"), "", &[]).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
