//! Resource pack selection bound to the game: the pack screen lists, and a
//! full resource reload after a change, while playing.
use super::{
    Game, PackResources, Resources, UiResources, build_renderer, prepare_renderer_textures,
};
use horizon_client::{
    assets::Assets,
    gfx::PreparedTextures,
    hud::HudAtlas,
    lang,
    pack_screen::{self, Compat, PackInfo, Source as PackSource},
    packs::{self, Pack, Position},
    streaming::WorkerContext,
    text::{Style, Text},
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Where overlay roots are built (beside options.txt).
pub fn cache_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_default()
        .join(".horizon-cache/resourcepacks")
}
pub fn packs_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_default()
        .join("resourcepacks")
}
/// `Util.getPlatform().openPath`: the system file manager on the folder.
pub fn open_folder(dir: &Path) {
    let program = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if let Err(e) = std::process::Command::new(program).arg(dir).spawn() {
        log::warn!("Horizon: could not open {}: {e}", dir.display());
    }
}
/// `Util.getPlatform().openUri`: the system browser on the link.
pub fn open_uri(uri: &str) {
    // URLs need Windows' registered protocol handler, not Explorer's path
    // handling. ShellExecute also preserves the URL as one Unicode value,
    // without interpolating its query string into a command interpreter.
    #[cfg(windows)]
    if let Err(error) = open::that_detached(uri) {
        log::warn!("Horizon: could not open URI {uri}: {error}");
    }
    #[cfg(not(windows))]
    open_folder(Path::new(uri));
}
/// The resource root to load from, given the vanilla root, the selection and
/// a server's packs, which go on top (`Pack.Position.TOP`, fixed).
pub fn startup_root(
    vanilla: &Path,
    selected: &[String],
    server: &[Pack],
) -> std::io::Result<PathBuf> {
    let all = packs::discover(vanilla, &packs_dir());
    let mut enabled = packs::enabled(&all, selected);
    enabled.extend_from_slice(server);
    packs::resource_root(vanilla, &enabled, &cache_dir())
}
fn info(p: &Pack) -> PackInfo {
    let built_in = !p.id.starts_with("file/");
    PackInfo {
        id: p.id.clone(),
        title: p.title.clone(),
        description: p.raw_description.clone(),
        description_style: p.description_style.clone(),
        source: if built_in {
            PackSource::BuiltIn
        } else {
            PackSource::Default
        },
        compat: p.compatibility,
        icon: p.icon_png().as_deref().and_then(pack_screen::load_icon),
        required: false,
        position: Position::Top,
        fixed: false,
    }
}
/// A pack from `DownloadedPackSource.loadRequestedPacks`: fixed at the top of
/// the selected list (`DOWNLOADED_PACK_SELECTION`), labelled as a server pack
/// (or a world's own pack when its id is the local-world uuid).
fn downloaded_info(p: &Pack) -> PackInfo {
    // `WorldOpenFlows.WORLD_PACK_ID` as hex in `server/{uuid}/…`.
    const WORLD: &str = "server/640a6a92b6cb48a0b391831586500359/";
    let world = p.id.starts_with(WORLD);
    PackInfo {
        id: p.id.clone(),
        title: p.title.clone(),
        description: p.raw_description.clone(),
        description_style: p.description_style.clone(),
        source: if world {
            PackSource::World
        } else {
            PackSource::Server
        },
        compat: p.compatibility,
        icon: p.icon_png().as_deref().and_then(pack_screen::load_icon),
        required: true,
        position: Position::Top,
        fixed: true,
    }
}
/// The vanilla pack: required, at the bottom (`BuiltInPackSource`), with the
/// client JAR's `pack.png` that the prepared resources hold.
fn vanilla_info(vanilla: &Path) -> PackInfo {
    PackInfo {
        id: "vanilla".into(),
        title: Text::translate("resourcePack.vanilla.name", &[]),
        description: Text::translate("resourcePack.vanilla.description", &[]),
        description_style: Style::default(),
        source: PackSource::BuiltIn,
        compat: Compat::Compatible,
        icon: std::fs::read(vanilla.join("pack.png"))
            .ok()
            .as_deref()
            .and_then(pack_screen::load_icon),
        required: true,
        position: Position::Bottom,
        fixed: false,
    }
}
/// Packs the Resource Packs screen lists: folder and built-in packs, then any
/// active downloaded server (or world) packs, then vanilla.
pub(super) fn pack_infos(vanilla: &Path, server: &[Pack]) -> Vec<PackInfo> {
    let mut infos: Vec<PackInfo> = packs::discover(vanilla, &packs_dir())
        .iter()
        .map(info)
        .collect();
    infos.extend(server.iter().map(downloaded_info));
    infos.push(vanilla_info(vanilla));
    infos
}
impl Game<'_> {
    pub(super) fn vanilla_root(&self) -> PathBuf {
        if self.ui.vanilla_root.as_os_str().is_empty() {
            self.options.resources.clone()
        } else {
            self.ui.vanilla_root.clone()
        }
    }
    /// The packs the pack screen offers and the current selection
    /// (`PackSelectionScreen` makes its `PackSelectionModel` from them).
    pub(super) fn refresh_pack_lists(&mut self) {
        let infos = pack_infos(&self.vanilla_root(), &self.server_packs.active);
        let o = &mut self.ui.options;
        o.pack_infos = infos;
        o.pack_selection = self.settings.resource_packs.clone();
        o.pack_screen = None;
    }
    /// `Minecraft.reloadResourcePacks`: rebuild the overlay and reload every
    /// client resource from it, as vanilla's reload listeners do: block
    /// models and sprites (every section is meshed again,
    /// `LevelRenderer.allChanged`), the shared atlas (GUI, item, entity,
    /// particle and font textures), item models, sounds, languages, GUI
    /// metadata and the cloud and world border textures. The loading runs on
    /// a worker under the progress overlay (`LoadingOverlay` over
    /// `ReloadableResourceManager.createReload`'s background executor), so
    /// the window keeps drawing; `drive_pack_reload` applies it.
    pub(super) fn reload_packs(&mut self) {
        // Pack reload updates tail-owned mob metadata. Keep menus responsive
        // until those resources exist, then apply the latest selection once.
        // A reload already under its overlay finishes first (vanilla's
        // `pendingReload`, which `runTick` starts once the overlay is gone).
        if reload_waits(
            self.rest_loader.is_some() || self.world_loader.is_some(),
            self.pack_reload.is_some(),
        ) {
            self.pending_pack_reload = true;
            return;
        }
        if let Err(e) = self.spawn_pack_reload(ReloadKind::Selection) {
            self.selection_reload_failed(&format!("could not start the pack reload: {e}"));
        }
    }
    /// `reloadResourcePacks`' failure (`Minecraft.rollbackResourcePacks`): with
    /// a pack selected, all selected packs are dropped and the reload runs
    /// again, with the load-failure toast; with none left to drop, the pack
    /// screen again, with the error.
    fn selection_reload_failed(&mut self, error: &str) {
        log::warn!("Horizon: resource reload failed: {error}");
        let all = packs::discover(&self.vanilla_root(), &packs_dir());
        if packs::able_to_clear_any_pack(&all, &self.settings.resource_packs) {
            // clearResourcePacksOnError.
            log::info!("Caught error loading resourcepacks, removing all selected resourcepacks");
            self.settings.resource_packs.clear();
            self.settings.save(&Self::options_path());
            if self.spawn_pack_reload(ReloadKind::Selection).is_ok() {
                // addResourcePackLoadFailToast(null), SystemToast.addOrUpdate.
                let title = lang::tr("resourcePack.load_fail", &[]);
                match &mut self.server_packs.load_fail {
                    Some(toast) => toast.reset(title, None),
                    None => {
                        self.server_packs.load_fail =
                            Some(horizon_client::toasts::SystemToast::new(title, None));
                    }
                }
                return;
            }
        }
        if self.screen != super::Screen::Options {
            self.open_options_at(
                self.title.is_some(),
                horizon_client::options_ui::Page::Packs,
            );
        }
        self.ui.options.resource_error = Some(error.to_string());
    }
    /// Stages the selection with the server's active packs on top on a
    /// worker (`createReload`'s background executor); the frame draws its
    /// progress meanwhile, and the overlay takes no input (`LoadingOverlay`),
    /// so what is held is released (`KeyMapping.releaseAll`).
    pub(super) fn spawn_pack_reload(&mut self, kind: ReloadKind) -> std::io::Result<()> {
        self.keys.clear();
        self.physical_keys.clear();
        self.left = false;
        self.right = false;
        let vanilla = self.vanilla_root();
        if self.ui.vanilla_root.as_os_str().is_empty() {
            self.ui.vanilla_root = vanilla.clone();
        }
        let (selected, server, mods, dev_mods) = (
            self.settings.resource_packs.clone(),
            self.server_packs.active.clone(),
            self.settings.mods.enabled.clone(),
            self.options.dev_mods.clone(),
        );
        let (assets, res, context) = (
            self.pack.assets.clone(),
            self.res.clone(),
            self.pack.context.clone(),
        );
        let (unicode, japanese) = (self.settings.force_unicode, self.settings.japanese_glyphs);
        self.pack_reload = Some(PackReload::spawn(kind, move |progress| {
            stage_pack(
                &vanilla, &selected, &server, &mods, &dev_mods, &assets, &res, &context, unicode,
                japanese, progress,
            )
            .map_err(|e| e.to_string())
        })?);
        Ok(())
    }
    /// `LoadingOverlay.tick` once the reload is done: the staged pack is
    /// applied on the render thread, and its outcome goes to whoever asked
    /// (`reloadResourcePacks`' callback, or `PackReloadConfig.Callbacks`).
    pub(super) fn drive_pack_reload(&mut self) {
        let Some((kind, staged)) = self.pack_reload.as_mut().and_then(PackReload::finished) else {
            return;
        };
        self.pack_reload = None;
        let result =
            staged.and_then(|staged| self.apply_staged_pack(staged).map_err(|e| e.to_string()));
        match kind {
            ReloadKind::Selection => {
                if let Err(e) = result {
                    self.selection_reload_failed(&e);
                }
            }
            ReloadKind::Server => {
                if let Err(e) = &result {
                    log::warn!("Horizon: server resource pack reload failed: {e}");
                }
                self.finish_pack_reload(result.is_ok());
            }
        }
    }
    /// The render-thread half: the renderer on the new pack's textures, then
    /// the swap (`ReloadInstance`'s application listeners).
    pub(super) fn apply_staged_pack(&mut self, staged: StagedPack) -> horizon_client::Result<()> {
        let StagedPack {
            root,
            gui,
            pack,
            textures,
        } = staged;
        // Keep the working GPU resources, and hand its window surface over.
        if let Some(renderer) = &mut self.renderer {
            renderer.release_surface();
        }
        let renderer = match self
            .window
            .clone()
            .map(|window| {
                build_renderer(
                    window,
                    self.options.offscreen,
                    &pack,
                    &gui,
                    self.renderer.as_ref(),
                    Some(textures),
                    self.settings.horizon.api,
                )
            })
            .transpose()
        {
            Ok(renderer) => renderer,
            Err(error) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.recreate_surface().map_err(|restore| {
                        format!(
                            "{error}; could not restore the previous rendering surface: {restore}"
                        )
                    })?;
                }
                return Err(error);
            }
        };
        // Everything loaded: switch over.
        self.waypoint_styles = horizon_client::locator_bar::WaypointStyles::load(&root);
        self.armor_stand_assets.reload_trims(&root);
        self.random_tick_sections.clear();
        self.portals.sections.clear();
        self.options.resources = root.clone();
        let context = pack.context.clone();
        self.pack = pack;
        self.gui = gui;
        // The new atlas's skin and map slots are blank: players and maps
        // upload into it again instead of naming sprites it does not have.
        self.skin_slots.forget();
        self.skin_downloads
            .borrow_mut()
            .set_resource_root(self.options.resources.clone());
        self.maps.slots.forget();
        self.map_patches.clear();
        // Kept mod block entities name the old atlas's sprites.
        self.kept_block_entities.get_mut().clear();
        // The atlas's sprites moved: the particle engine re-reads every rect
        // and the packs' sprite sets, or every particle draws a stale region.
        self.particles
            .reload(&root, &self.gui.atlas.sprites, &self.gui.atlas.translucent);
        horizon_client::ui_theme::reset_atlas();
        // Villager hats come from the pack's texture metadata (shared data, swapped in place).
        *self
            .res
            .mob_data()
            .villager_hats
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            horizon_client::mobs::villager_hats::VillagerHats::load(&root);
        horizon_client::ui::load_metadata(&root);
        lang::set(lang::Language::load(&[&root], &lang::code()));
        self.refresh_remote_language();
        self.audio.reload(&root);
        self.clear_lod();
        self.streaming.reload_resources(&mut self.world, context);
        self.pending_uploads.clear();
        // New GPU textures: a renderer built from the reloaded resources; the
        // sections are uploaded again as they are meshed.
        if let Some(mut renderer) = renderer {
            // The old one's size: a splitscreen player's view is smaller than the window the new
            // one was sized from (it drew a window-sized frame into the view, cut off).
            if let Some(old) = &self.renderer {
                renderer.resize(old.config.width, old.config.height);
            }
            self.renderer = Some(renderer);
            self.apply_ui_settings();
        }
        // Horizon's own shader pack is built again: its sprite tags name the old atlas's
        // layers, and a new renderer runs no pack.
        self.native_shaders = super::native_shaders::NativeShaders::picking(&std::mem::take(
            &mut self.native_shaders.picked,
        ));
        self.ui.chat.rescale(&self.gui.atlas);
        // The creative screen survives the reload; item names may not.
        self.creative_screen.forget_items();
        self.native_client_resources_reloaded();
        Ok(())
    }
}

/// The CPU half of a pack reload (`PackReload` runs it on a worker;
/// `apply_staged_pack` does the GPU half and the swap): the overlay
/// root, the models and sprites, the menu bundle, the pack's tables and its
/// CPU mip chains.
pub(super) fn stage_pack(
    vanilla_root: &Path,
    selected: &[String],
    server: &[Pack],
    mods_enabled: &BTreeSet<String>,
    dev_mods: &[PathBuf],
    assets: &Arc<Assets>,
    res: &Resources,
    context: &WorkerContext,
    force_unicode: bool,
    japanese: bool,
    progress: &super::Progress,
) -> horizon_client::Result<StagedPack> {
    let root = startup_root(vanilla_root, selected, server)?;
    let root = super::mods::content_root(&root, mods_enabled, dev_mods);
    progress.set(1);
    // The world's models and sprites and the GUI atlas read the same root and nothing of
    // each other: about a second each, side by side.
    let (render, atlas, panorama) = std::thread::scope(|s| {
        let atlas =
            s.spawn(|| HudAtlas::build_over(&root, vanilla_root).map_err(|e| e.to_string()));
        let panorama = s.spawn(|| horizon_client::gfx::panorama::Faces::load(&root));
        (assets.reload(&root), atlas.join(), panorama.join())
    });
    let render = Arc::new(render?);
    let mut atlas = atlas.unwrap_or_else(|p| std::panic::resume_unwind(p))?;
    atlas.font.uniform = force_unicode;
    atlas.font.japanese = japanese;
    let gui = UiResources {
        atlas,
        panorama: Arc::new(panorama.unwrap_or_else(|p| std::panic::resume_unwind(p))),
    };
    progress.set(2);
    let pack = PackResources::load(
        &root,
        assets.clone(),
        render,
        res.facts(),
        context.biomes.clone(),
        context.light.clone(),
    )?;
    let textures = prepare_renderer_textures(&pack);
    progress.set(3);
    Ok(StagedPack {
        root,
        gui,
        pack,
        textures,
    })
}

/// Everything a pack reload builds on the CPU (`apply_staged_pack` finishes
/// it on the render thread).
pub(super) struct StagedPack {
    pub root: PathBuf,
    pub gui: UiResources,
    pub pack: PackResources,
    pub textures: PreparedTextures,
}

/// `LoadingOverlay`'s stage lines for a pack reload, indexed by `progress`.
const RELOAD_LINES: [&str; 4] = [
    "Preparing resources",
    "Loading assets",
    "Loading pack tables",
    "Applying resources",
];
/// Whether a reload asked for now waits: for the world's resources, or for
/// the reload already under its overlay (`Minecraft.reloadResourcePacks`
/// hands back `pendingReload` while a `LoadingOverlay` is up).
fn reload_waits(world_loading: bool, in_flight: bool) -> bool {
    world_loading || in_flight
}
/// Who asked for a pack reload: its outcome goes back there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReloadKind {
    /// The player's own selection (`Minecraft.reloadResourcePacks`).
    Selection,
    /// A server's packs (`DownloadedPackSource.startReload`).
    Server,
}
type StageResult = std::result::Result<StagedPack, String>;
/// A pack reload whose CPU half runs on a worker while `progress` feeds the
/// loading overlay; `Game::drive_pack_reload` applies it on the render thread.
pub(super) struct PackReload {
    pub(super) progress: super::Progress,
    kind: ReloadKind,
    handle: Option<std::thread::JoinHandle<StageResult>>,
}
impl PackReload {
    /// Starts `stage` on its own thread and returns at once.
    fn spawn(
        kind: ReloadKind,
        stage: impl FnOnce(&super::Progress) -> StageResult + Send + 'static,
    ) -> std::io::Result<Self> {
        let progress = super::Progress::with(&RELOAD_LINES);
        let worker = progress.clone();
        let handle = std::thread::Builder::new()
            .name("pack-reload".into())
            .spawn(move || stage(&worker))?;
        Ok(Self {
            progress,
            kind,
            handle: Some(handle),
        })
    }
    pub(super) fn kind(&self) -> ReloadKind {
        self.kind
    }
    /// The staged pack once the worker is done (taken once); none while it runs.
    fn finished(&mut self) -> Option<(ReloadKind, StageResult)> {
        if !self.handle.as_ref()?.is_finished() {
            return None;
        }
        let staged = self
            .handle
            .take()?
            .join()
            .unwrap_or_else(|_| Err("the pack reload worker panicked".into()));
        Some((self.kind, staged))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Duration};

    fn wait(reload: &mut PackReload) -> (ReloadKind, StageResult) {
        for _ in 0..500 {
            if let Some(done) = reload.finished() {
                return done;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the reload worker never finished");
    }

    #[test]
    fn a_reload_stages_on_a_worker_while_the_caller_goes_on() {
        // The stage blocks until the caller lets it go: `spawn` returning at
        // all means the window thread is free to draw the overlay.
        let (release, blocked) = mpsc::channel::<()>();
        let (thread, ran_on) = mpsc::channel();
        let mut reload = PackReload::spawn(ReloadKind::Selection, move |progress| {
            blocked.recv().unwrap();
            progress.set(2);
            thread
                .send(std::thread::current().name().map(str::to_owned))
                .unwrap();
            Err("staged".into())
        })
        .unwrap();
        assert!(reload.finished().is_none(), "still staging");
        assert_eq!(reload.progress.line(), "Preparing resources");
        release.send(()).unwrap();
        let (kind, staged) = wait(&mut reload);
        assert_eq!(kind, ReloadKind::Selection);
        assert_eq!(staged.err().as_deref(), Some("staged"));
        assert_eq!(reload.progress.line(), "Loading pack tables");
        assert_eq!(ran_on.recv().unwrap().as_deref(), Some("pack-reload"));
        assert!(reload.finished().is_none(), "the outcome is taken once");
    }

    #[test]
    fn a_panicking_stage_fails_the_reload() {
        let mut reload = PackReload::spawn(ReloadKind::Server, |_| panic!("bad pack")).unwrap();
        let (kind, staged) = wait(&mut reload);
        assert_eq!(kind, ReloadKind::Server);
        assert_eq!(
            staged.err().as_deref(),
            Some("the pack reload worker panicked")
        );
    }

    #[test]
    fn a_reload_waits_for_world_resources_and_the_one_in_flight() {
        assert!(!reload_waits(false, false));
        assert!(reload_waits(true, false));
        assert!(reload_waits(false, true));
        assert!(reload_waits(true, true));
    }
}
