//! A server's resource packs bound to the game (`ClientCommonPacketListenerImpl.
//! handleResourcePackPush` and `handleResourcePackPop`, its `PackConfirmScreen`,
//! and `DownloadedPackSource` as `Minecraft` drives it): the prompt, the
//! server list's setting, downloads into `downloads/`, and a resource reload
//! with the server's packs on top of the player's own.
use super::{Game, Progress};
use horizon_client::{
    Result,
    confirm_screen::Confirm,
    hud::{HudAtlas, HudBatch},
    java_text, multiplayer, packs,
    scoreboard::{comp_text, tag_to_json},
    server_packs::{Prompt, ServerPacks, action, download, parse_sha1, valid_url},
    text::{Text, named_color},
    toasts::SystemToast,
    ui,
};
use horizon_core::jmath;
use horizon_protocol::{packets::configuration::ResourcePackPush, servers};
use std::{
    path::PathBuf,
    sync::mpsc::{Receiver, TryRecvError},
};

/// `ClientCommonPacketListenerImpl.PackConfirmScreen`: the packs it asks
/// about, whether the last one is required, and the server's message.
pub(super) struct PackPrompt {
    requests: Vec<(u128, String, String)>,
    required: bool,
    prompt: Option<Text>,
}
impl PackPrompt {
    fn texts(&self) -> (Text, Text, [Text; 2]) {
        let t = |key: &str| Text::translate(key, &[]);
        let (title, line2) = if self.required {
            let yellow = named_color("yellow").unwrap_or(0xFFFF55);
            (
                t("multiplayer.requiredTexturePrompt.line1"),
                t("multiplayer.requiredTexturePrompt.line2")
                    .with_color(yellow)
                    .bold(),
            )
        } else {
            (
                t("multiplayer.texturePrompt.line1"),
                t("multiplayer.texturePrompt.line2"),
            )
        };
        // preparePackPrompt.
        let message = match &self.prompt {
            Some(prompt) => Text::translate(
                "multiplayer.texturePrompt.serverPrompt",
                &[line2, prompt.clone()],
            ),
            None => line2,
        };
        let buttons = if self.required {
            [t("gui.proceed"), t("menu.disconnect")]
        } else {
            [t("gui.yes"), t("gui.no")]
        };
        (title, message, buttons)
    }
    fn layout(&self, atlas: &HudAtlas, size: (f32, f32)) -> Confirm {
        let (title, message, _) = self.texts();
        Confirm::new(&atlas.font, size, &title, &message)
    }
}

impl ServerPackState {
    /// The `ToastManager` slots the download toasts hold.
    pub(super) fn toast_slots(&self) -> impl Iterator<Item = usize> + '_ {
        self.notifiers.iter().filter_map(|n| n.toast.as_ref()?.slot)
    }
}

/// The game's `DownloadedPackSource`.
#[derive(Default)]
pub(super) struct ServerPackState {
    packs: ServerPacks,
    /// `downloads/` in the game folder, opened at the first download.
    queue: Option<download::Queue>,
    pub(super) prompt: Option<PackPrompt>,
    /// The server list entry the setting is saved to (none: a direct join).
    pub(super) address: Option<String>,
    /// The account the downloads are made for (the download headers).
    user: (String, u128),
    /// The packs loaded on top of the player's own, bottom first.
    pub(super) active: Vec<packs::Pack>,
    /// The download batches' notifiers, oldest first.
    notifiers: Vec<PackNotifier>,
    /// A `tick`-asked reload staged on a worker (`LoadingOverlay`): the
    /// frame draws its progress until `tick_server_packs` applies it.
    pub(super) reload: Option<PackReload>,
}
/// `DownloadedPackSource.createDownloadNotifier`: one batch's progress in its
/// own `SystemToast`.
struct PackNotifier {
    progress: Receiver<download::Progress>,
    total: usize,
    /// `count`: the requests started.
    count: usize,
    /// `failCount`.
    failed: usize,
    /// `totalBytes`: the Content-Length of the request in flight.
    size: Option<u64>,
    /// The bytes it has written so far: none before the first response.
    bytes: Option<u64>,
    /// The last request has finished.
    done: bool,
    toast: Option<SystemToast>,
}
impl PackNotifier {
    fn new(progress: Receiver<download::Progress>, total: usize) -> Self {
        Self {
            progress,
            total,
            count: 0,
            failed: 0,
            size: None,
            bytes: None,
            done: false,
            toast: None,
        }
    }
    /// The batch's events since the last call, with the toast updated once
    /// for all of them. False once the batch has ended and its toast is gone.
    fn poll(&mut self) -> bool {
        let (mut changed, mut hide) = (false, false);
        let ended = loop {
            let event = match self.progress.try_recv() {
                Ok(event) => event,
                Err(e) => break e == TryRecvError::Disconnected,
            };
            changed |= match event {
                download::Progress::RequestStart => {
                    self.count += 1;
                    true
                }
                download::Progress::DownloadStart(size) => {
                    self.size = size;
                    self.bytes = Some(0);
                    true
                }
                download::Progress::Bytes(bytes) => {
                    self.bytes = Some(bytes);
                    true
                }
                download::Progress::RequestFinished(ok) => {
                    self.failed += usize::from(!ok);
                    self.done = self.count == self.total;
                    // Every pack came: forceHide; else the failure title.
                    hide |= self.done && self.failed == 0;
                    self.done && self.failed > 0
                }
            };
        };
        if changed {
            let (title, message) = self.text();
            let (title, message) = (title.plain(), message.map(|m| m.plain()));
            // SystemToast.addOrUpdate.
            match &mut self.toast {
                Some(toast) => toast.reset(title, message),
                None => self.toast = Some(SystemToast::new(title, message)),
            }
        }
        if hide && let Some(toast) = &mut self.toast {
            toast.force_hide();
        }
        !ended || self.toast.is_some()
    }
    /// The toast's title and message.
    fn text(&self) -> (Text, Option<Text>) {
        if self.done && self.failed > 0 {
            let failed = Text::tr("download.pack.failed", &[&self.failed, &self.total]);
            return (failed, None);
        }
        let title = Text::tr("download.pack.title", &[&self.count, &self.total]);
        let message = self.bytes.map(|bytes| match self.size {
            Some(size) if size > 0 => {
                Text::tr("download.pack.progress.percent", &[&(bytes * 100 / size)])
            }
            _ => Text::tr("download.pack.progress.bytes", &[&human_readable(bytes)]),
        });
        (title, message)
    }
}
/// `com.mojang.realmsclient.Unit.humanReadable`, for
/// `download.pack.progress.bytes`.
fn human_readable(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let exp = (jmath::log(bytes as f64) / jmath::log(1024.0)) as i32;
    let prefix = &"KMGTPE"[exp as usize - 1..exp as usize];
    let value = bytes as f64 / 1024f64.powi(exp);
    format!("{} {prefix}B", java_text::format_fixed(value, 1))
}

pub(super) fn saves() -> PathBuf {
    std::env::current_dir().unwrap_or_default().join("saves")
}

/// `LoadingOverlay`'s stage lines for a pack reload, indexed by `progress`.
const RELOAD_LINES: [&str; 4] = [
    "Preparing resources",
    "Loading assets",
    "Loading pack tables",
    "Applying resources",
];
/// A pack reload `ServerPacks::tick` asked for: the CPU half runs on a
/// worker while `progress` feeds the loading screen, then the apply lands
/// on the render thread.
pub(super) struct PackReload {
    pub(super) progress: Progress,
    handle: std::thread::JoinHandle<std::result::Result<super::packs::StagedPack, String>>,
}

impl Game<'_> {
    /// `ConnectScreen`: the server's setting decides what its packs do
    /// (`configureForServerControl`); single player's own server takes them
    /// all, quietly (`configureForLocalWorld`).
    pub(super) fn server_packs_connect(&mut self, address: Option<&str>, user: (String, u128)) {
        let state = &mut self.server_packs;
        state.user = user;
        state.prompt = None;
        state.notifiers.clear();
        let Some(address) = address else {
            state.address = None;
            state.packs.local_world();
            return;
        };
        let file = multiplayer::servers_file(&saves());
        let entry = servers::load(&file).into_iter().find(|s| s.ip == address);
        let setting = entry.as_ref().and_then(|e| e.accept_textures);
        state.address = entry.map(|e| e.ip);
        state.packs.connect(Prompt::for_server(setting));
    }
    /// `Minecraft.clearDownloadedResourcePacks`: every server pack goes at once.
    pub(super) fn server_packs_disconnect(&mut self) {
        self.server_packs.prompt = None;
        self.server_packs.notifiers.clear();
        self.server_packs.packs.disconnect();
        self.tick_server_packs();
    }

    /// `handleResourcePackPush`.
    pub(super) fn handle_pack_push(&mut self, p: ResourcePackPush) {
        if !valid_url(&p.url) {
            self.send_pack_status(p.id, action::INVALID_URL);
            return;
        }
        let setting = self.server_setting();
        let prompt = Prompt::for_server(setting);
        if prompt != Prompt::Pending && (!p.required || prompt != Prompt::Declined) {
            self.server_packs
                .packs
                .push(p.id, &p.url, parse_sha1(&p.hash));
            return;
        }
        // addOrUpdatePackPrompt: one prompt for every pack pushed before it is answered.
        let text = p.prompt.as_ref().map(|t| comp_text(&tag_to_json(t)));
        let state = &mut self.server_packs;
        let open = state.prompt.get_or_insert(PackPrompt {
            requests: Vec::new(),
            required: p.required,
            prompt: None,
        });
        open.requests.push((p.id, p.url, p.hash));
        open.required = p.required;
        open.prompt = text;
        // Gui.setScreen: KeyMapping.releaseAll, so no walking behind the prompt.
        self.keys.clear();
        self.capture(false);
    }
    /// `handleResourcePackPop`: one pack, or all of them.
    pub(super) fn handle_pack_pop(&mut self, id: Option<u128>) {
        match id {
            Some(id) => self.server_packs.packs.pop(id),
            None => self.server_packs.packs.pop_all(),
        }
    }
    /// The joined server's entry in the server list: its `acceptTextures`.
    fn server_setting(&self) -> Option<bool> {
        let address = self.server_packs.address.as_deref()?;
        servers::load(&multiplayer::servers_file(&saves()))
            .into_iter()
            .find(|s| s.ip == address)?
            .accept_textures
    }
    fn send_pack_status(&self, id: u128, status: i32) {
        if let Some(r) = &self.remote {
            r.session.send_pack_status(id, status);
        }
    }

    /// The prompt's answer (`PackConfirmScreen`'s callback): the server's
    /// setting follows it, declining a required pack disconnects, and every
    /// pack it asked about is pushed.
    pub(super) fn answer_pack_prompt(&mut self, yes: bool) -> Result<()> {
        let Some(prompt) = self.server_packs.prompt.take() else {
            return Ok(());
        };
        if self.player.is_some() && self.screen == super::Screen::Playing {
            self.capture(true);
        }
        let setting = if yes {
            self.server_packs.packs.allow();
            Some(true)
        } else {
            self.server_packs.packs.reject();
            if prompt.required {
                let reason = Text::translate("multiplayer.requiredTexturePrompt.disconnect", &[]);
                return self.leave_server(Some(reason.plain()));
            }
            Some(false)
        };
        for (id, url, hash) in prompt.requests {
            self.server_packs.packs.push(id, &url, parse_sha1(&hash));
        }
        // ServerList.saveSingleServer.
        if let Some(address) = &self.server_packs.address {
            let file = multiplayer::servers_file(&saves());
            let mut list = servers::load(&file);
            for entry in list.iter_mut().filter(|s| &s.ip == address) {
                entry.accept_textures = setting;
            }
            if let Err(e) = servers::save(&file, &list) {
                log::warn!("Horizon: could not save {}: {e}", file.display());
            }
        }
        Ok(())
    }

    /// Runs what the packs need (`createUpdateScheduler`): the downloads
    /// that finished, the ones to start, a resource reload when the active
    /// packs change, and the statuses for the server.
    pub(super) fn tick_server_packs(&mut self) {
        self.server_packs.notifiers.retain_mut(PackNotifier::poll);
        // A reload waits for the world's resources (`reload_packs`).
        if self.rest_loader.is_some() || self.world_loader.is_some() {
            return;
        }
        self.drive_pack_reload();
        let finished = self
            .server_packs
            .queue
            .as_ref()
            .map(download::Queue::finished)
            .unwrap_or_default();
        for batch in finished {
            self.server_packs.packs.downloaded(batch);
        }
        while self.server_packs.packs.wants_tick() {
            let tick = self.server_packs.packs.tick();
            if !tick.download.is_empty() {
                self.start_pack_downloads(tick.download);
            }
            if let Some(load) = tick.reload {
                self.start_pack_reload(&load);
            }
        }
        for (id, status) in self.server_packs.packs.take_replies() {
            self.send_pack_status(id, status);
        }
    }
    fn start_pack_downloads(&mut self, requests: Vec<horizon_client::server_packs::Request>) {
        let state = &mut self.server_packs;
        if state.queue.is_none() {
            let dir = std::env::current_dir()
                .unwrap_or_default()
                .join("downloads");
            match download::Queue::open(dir) {
                Ok(queue) => state.queue = Some(queue),
                Err(e) => {
                    log::error!("Horizon: failed to open the download queue: {e}");
                    // Every request failed, as the notifier would hear it.
                    let (progress, events) = std::sync::mpsc::channel();
                    for _ in &requests {
                        let _ = progress.send(download::Progress::RequestStart);
                        let _ = progress.send(download::Progress::RequestFinished(false));
                    }
                    state
                        .notifiers
                        .push(PackNotifier::new(events, requests.len()));
                    let failed = requests.iter().map(|r| r.id).collect();
                    state.packs.downloaded(horizon_client::server_packs::Batch {
                        downloaded: Vec::new(),
                        failed,
                    });
                    return;
                }
            }
        }
        let headers = download::headers(&state.user.0, state.user.1);
        let (progress, events) = std::sync::mpsc::channel();
        state
            .notifiers
            .push(PackNotifier::new(events, requests.len()));
        if let Some(queue) = &state.queue {
            queue.download(requests, headers, progress);
        }
    }
    /// `DownloadedPackSource.startReload`: the packs on top of the player's
    /// own, staged on a worker (`Minecraft.reloadResourcePacks` under a
    /// `LoadingOverlay`); `drive_pack_reload` applies it. A pack with no
    /// readable `pack.mcmeta` fails the reload at once, as `onFailure`.
    fn start_pack_reload(&mut self, load: &[(u128, PathBuf)]) {
        let server: Option<Vec<_>> = load
            .iter()
            .map(|(id, path)| packs::server_pack(*id, path))
            .collect();
        let Some(server) = server else {
            log::warn!("Horizon: invalid pack metadata in a server pack, ignoring all");
            self.finish_pack_reload(false);
            return;
        };
        self.server_packs.active = server.clone();
        // The overlay swallows input until the apply (`LoadingOverlay`):
        // release what is held so nothing keeps acting while it covers the
        // frame (`KeyMapping.releaseAll` when a screen opens).
        self.keys.clear();
        self.physical_keys.clear();
        self.left = false;
        self.right = false;
        let vanilla = self.vanilla_root();
        if self.ui.vanilla_root.as_os_str().is_empty() {
            self.ui.vanilla_root = vanilla.clone();
        }
        let (selected, mods, dev_mods) = (
            self.settings.resource_packs.clone(),
            self.settings.mods.enabled.clone(),
            self.options.dev_mods.clone(),
        );
        let (assets, res, context) = (
            self.pack.assets.clone(),
            self.res.clone(),
            self.pack.context.clone(),
        );
        let (unicode, japanese) = (self.settings.force_unicode, self.settings.japanese_glyphs);
        let progress = Progress::with(&RELOAD_LINES);
        let worker = progress.clone();
        let handle = std::thread::Builder::new()
            .name("server-pack-reload".into())
            .spawn(move || {
                super::packs::stage_pack(
                    &vanilla, &selected, &server, &mods, &dev_mods, &assets, &res, &context,
                    unicode, japanese, &worker,
                )
                .map_err(|e| e.to_string())
            });
        match handle {
            Ok(handle) => self.server_packs.reload = Some(PackReload { progress, handle }),
            Err(e) => {
                log::error!("Horizon: could not start the pack reload worker: {e}");
                self.finish_pack_reload(false);
            }
        }
    }
    /// `PackReloadConfig.Callbacks`: applies the staged reload once its
    /// worker is done; on failure the packs active before reload again.
    fn drive_pack_reload(&mut self) {
        let Some(reload) = &self.server_packs.reload else {
            return;
        };
        if !reload.handle.is_finished() {
            return;
        }
        let reload = self.server_packs.reload.take().unwrap();
        let staged = reload
            .handle
            .join()
            .unwrap_or_else(|_| Err("the pack reload worker panicked".into()));
        let ok = match staged
            .and_then(|staged| self.apply_staged_pack(staged).map_err(|e| e.to_string()))
        {
            Ok(()) => true,
            Err(e) => {
                log::warn!("Horizon: server resource pack reload failed: {e}");
                false
            }
        };
        self.finish_pack_reload(ok);
    }
    /// `ServerPacks::reloaded` and the recovery reload it can ask for.
    fn finish_pack_reload(&mut self, ok: bool) {
        if let Some(recovery) = self.server_packs.packs.reloaded(ok) {
            self.start_pack_reload(&recovery);
        }
    }

    pub(super) fn draw_pack_prompt(&self, batch: &mut HudBatch) {
        let Some(prompt) = &self.server_packs.prompt else {
            return;
        };
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        if self.player.is_none() {
            // In place of the connecting screen, on its background.
            batch.fill(
                atlas,
                0.0,
                0.0,
                w,
                h,
                horizon_client::hud::rgba(20, 20, 20, 255),
            );
        }
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, self.player.is_some());
        let (_, _, [yes, no]) = prompt.texts();
        prompt
            .layout(atlas, (w, h))
            .draw(batch, atlas, self.cursor, [&yes, &no]);
    }
    /// The download notifiers' toasts, each in a free 32-pixel slot of the
    /// `ToastManager` (`hidden`, F1, keeps them timing out undrawn).
    pub(super) fn draw_pack_toasts(&mut self, batch: &mut HudBatch, hidden: bool) {
        let multiplier = f64::from(self.settings.notification_display_time);
        let mut used = self.toast_slots_used();
        for notifier in &mut self.server_packs.notifiers {
            let Some(toast) = &mut notifier.toast else {
                continue;
            };
            if toast.slot.is_none() {
                let Some(slot) = (0..Self::TOAST_SLOTS).find(|s| used & (1 << s) == 0) else {
                    continue;
                };
                used |= 1 << slot;
                toast.slot = Some(slot);
            }
            let (sound, finished) = toast.update(multiplier);
            if let Some(show) = sound {
                let event = if show { "ui.toast.in" } else { "ui.toast.out" };
                self.audio
                    .play(&format!("minecraft:{event}"), None, 1.0, 1.0);
            }
            if finished {
                notifier.toast = None;
                continue;
            }
            if let (false, Some(slot)) = (hidden, toast.slot) {
                toast.draw(&self.gui.atlas, batch, 32.0 * slot as f32);
            }
        }
    }
    /// A left click on the prompt: its answer when it hit a button.
    pub(super) fn pack_prompt_click(&mut self) -> Result<()> {
        let Some(prompt) = &self.server_packs.prompt else {
            return Ok(());
        };
        let answer = prompt
            .layout(&self.gui.atlas, self.gui_size())
            .answer_at(self.cursor);
        match answer {
            Some(yes) => {
                self.audio.play_button_click();
                self.answer_pack_prompt(yes)
            }
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use download::Progress::*;

    fn notifier(total: usize, events: &[download::Progress]) -> PackNotifier {
        let (send, receive) = std::sync::mpsc::channel();
        for event in events {
            send.send(*event).unwrap();
        }
        let mut notifier = PackNotifier::new(receive, total);
        assert!(notifier.poll());
        notifier
    }
    fn texts(n: &PackNotifier) -> (serde_json::Value, Option<serde_json::Value>) {
        let (title, message) = n.text();
        (title.component(), message.map(|m| m.component()))
    }

    #[test]
    fn the_toast_counts_requests_and_shows_progress() {
        let n = notifier(2, &[RequestStart]);
        let (title, message) = texts(&n);
        assert_eq!(title["translate"], "download.pack.title");
        assert_eq!(title["with"], serde_json::json!([1, 2]));
        assert_eq!(message, None);
        let n = notifier(2, &[RequestStart, DownloadStart(Some(400)), Bytes(100)]);
        assert_eq!(
            texts(&n).1.unwrap(),
            serde_json::json!({"translate": "download.pack.progress.percent", "with": [25]})
        );
        let n = notifier(1, &[RequestStart, DownloadStart(None), Bytes(1280)]);
        assert_eq!(
            texts(&n).1.unwrap()["with"],
            serde_json::json!([{"text": "1.3 KB"}])
        );
    }

    #[test]
    fn a_failed_pack_turns_the_toast_into_the_failure_count() {
        let events = [
            RequestStart,
            RequestFinished(false),
            RequestStart,
            DownloadStart(Some(3)),
            Bytes(3),
        ];
        let (send, receive) = std::sync::mpsc::channel();
        for event in events {
            send.send(event).unwrap();
        }
        let mut n = PackNotifier::new(receive, 2);
        assert!(n.poll());
        // Not the last request yet: still downloading.
        assert_eq!(texts(&n).0["translate"], "download.pack.title");
        send.send(RequestFinished(true)).unwrap();
        drop(send);
        assert!(n.poll(), "the failure toast stays until it slides out");
        let (title, message) = texts(&n);
        assert_eq!(
            title,
            serde_json::json!({"translate": "download.pack.failed", "with": [1, 2]})
        );
        assert_eq!(message, None);
    }

    #[test]
    fn a_finished_batch_without_a_toast_is_dropped() {
        let (send, receive) = std::sync::mpsc::channel::<download::Progress>();
        drop(send);
        assert!(!PackNotifier::new(receive, 1).poll());
    }

    #[test]
    fn sizes_read_as_realms_unit_human_readable() {
        // com.mojang.realmsclient.Unit.humanReadable, run on the 26.3 jar.
        for (bytes, text) in [
            (0, "0 B"),
            (1023, "1023 B"),
            (1024, "1.0 KB"),
            (1280, "1.3 KB"),
            (1535, "1.5 KB"),
            (10_000, "9.8 KB"),
            (1_048_575, "1024.0 KB"),
            (1_048_576, "1.0 MB"),
            (52_428_800, "50.0 MB"),
            (262_144_000, "250.0 MB"),
            (1_073_741_824, "1.0 GB"),
        ] {
            assert_eq!(human_readable(bytes), text, "{bytes}");
        }
    }
}
