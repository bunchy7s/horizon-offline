//! Server resource packs (`ServerPackManager`, `DownloadedPackSource`): the
//! packs a server pushes, the player's answer to its prompt, their downloads,
//! their activation on top of the player's own packs, and the status each one
//! reports back to the server. The game drives it: it shows the prompt,
//! starts the downloads [`ServerPacks::tick`] asks for and reloads the
//! resources with [`ServerPacks::active`] when it asks for a reload.
pub mod download;

use std::path::PathBuf;

/// `ServerboundResourcePackPacket.Action`, by its protocol number.
pub mod action {
    pub const SUCCESSFULLY_LOADED: i32 = 0;
    pub const DECLINED: i32 = 1;
    pub const FAILED_DOWNLOAD: i32 = 2;
    pub const ACCEPTED: i32 = 3;
    pub const DOWNLOADED: i32 = 4;
    pub const INVALID_URL: i32 = 5;
    pub const FAILED_RELOAD: i32 = 6;
    pub const DISCARDED: i32 = 7;
}

/// `ServerPackManager.PackPromptStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prompt {
    Pending,
    Allowed,
    Declined,
}
impl Prompt {
    /// `ConnectScreen.convertPackStatus`: a server list entry's setting
    /// (servers.dat `acceptTextures`; none is Prompt).
    pub fn for_server(accept_textures: Option<bool>) -> Self {
        match accept_textures {
            Some(true) => Self::Allowed,
            Some(false) => Self::Declined,
            None => Self::Pending,
        }
    }
}

/// `ClientCommonPacketListenerImpl.parseResourcePackUrl`: only http and https.
pub fn valid_url(url: &str) -> bool {
    url.split_once(':').is_some_and(|(scheme, _)| {
        scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
    })
}

/// `DownloadedPackSource.tryParseSha1Hash`: 40 hex digits, else no check.
pub fn parse_sha1(hash: &str) -> Option<String> {
    (hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| hash.to_ascii_lowercase())
}

/// One pack to download (`DownloadQueue.DownloadRequest`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: u128,
    pub url: String,
    pub sha1: Option<String>,
}

/// `DownloadQueue.BatchResult`: the file each downloaded pack is in, and the
/// packs that failed.
#[derive(Debug, Default)]
pub struct Batch {
    pub downloaded: Vec<(u128, PathBuf)>,
    pub failed: Vec<u128>,
}

/// What the game has to do after a [`ServerPacks::tick`].
#[derive(Debug, Default, PartialEq)]
pub struct Tick {
    /// Download these, then hand the result to [`ServerPacks::downloaded`].
    pub download: Vec<Request>,
    /// Reload the resources with these packs on top, bottom first
    /// (`PackReloadConfig.scheduleReload`), then call [`ServerPacks::reloaded`].
    pub reload: Option<Vec<(u128, PathBuf)>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Download {
    Requested,
    Pending,
    Done,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Activation {
    Inactive,
    Pending,
    Active,
}
/// `ServerPackManager.RemovalReason`, with the status it reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Removal {
    DownloadFailed,
    ActivationFailed,
    Declined,
    Discarded,
    ServerRemoved,
    ServerReplaced,
}
impl Removal {
    fn reply(self) -> Option<i32> {
        match self {
            Self::DownloadFailed => Some(action::FAILED_DOWNLOAD),
            Self::ActivationFailed => Some(action::FAILED_RELOAD),
            Self::Declined => Some(action::DECLINED),
            Self::Discarded => Some(action::DISCARDED),
            Self::ServerRemoved | Self::ServerReplaced => None,
        }
    }
}

/// `ServerPackManager.ServerPackData`.
#[derive(Debug)]
struct Pack {
    id: u128,
    url: String,
    sha1: Option<String>,
    path: Option<PathBuf>,
    removal: Option<Removal>,
    download: Download,
    activation: Activation,
    accepted: bool,
}
impl Pack {
    fn remove(&mut self, reason: Removal) {
        self.removal.get_or_insert(reason);
    }
    fn removed(&self) -> bool {
        self.removal.is_some()
    }
}

/// The reload in progress: the packs it loads and unloads (the callbacks
/// `triggerReloadIfNeeded` gives `scheduleReload`).
#[derive(Debug)]
struct Reload {
    load: Vec<u128>,
    unload: Vec<u128>,
    /// It reloads what was active before a failed one (`isRecovery`).
    recovery: bool,
}

/// `ServerPackManager` with the feedback `DownloadedPackSource` gives it.
#[derive(Debug)]
pub struct ServerPacks {
    packs: Vec<Pack>,
    prompt: Prompt,
    reload: Option<Reload>,
    /// `registerForUpdate`: something changed, [`Self::tick`] again.
    update: bool,
    /// Report to the server (`configureForServerControl`); otherwise the
    /// statuses are only logged, as for a local world.
    reporting: bool,
    replies: Vec<(u128, i32)>,
}
impl Default for ServerPacks {
    fn default() -> Self {
        Self {
            packs: Vec::new(),
            prompt: Prompt::Pending,
            reload: None,
            update: false,
            reporting: false,
            replies: Vec::new(),
        }
    }
}
impl ServerPacks {
    fn report(&mut self, id: u128, status: i32) {
        log::debug!("Horizon: server pack {id:032x} changed status to {status}");
        if self.reporting {
            self.replies.push((id, status));
        }
    }
    /// The statuses to send the server, oldest first.
    pub fn take_replies(&mut self) -> Vec<(u128, i32)> {
        std::mem::take(&mut self.replies)
    }
    /// Whether [`Self::tick`] has something to do.
    pub fn wants_tick(&self) -> bool {
        self.update
    }

    /// `DownloadedPackSource.configureForServerControl`.
    pub fn connect(&mut self, prompt: Prompt) {
        self.reporting = true;
        match prompt {
            Prompt::Allowed => self.allow(),
            Prompt::Declined => self.reject(),
            Prompt::Pending => self.prompt = Prompt::Pending,
        }
    }
    /// `DownloadedPackSource.configureForLocalWorld`: single player's own
    /// server; its packs are taken, and nothing is reported.
    pub fn local_world(&mut self) {
        self.reporting = false;
        self.allow();
    }
    /// `DownloadedPackSource.cleanupAfterDisconnect`: every pack goes, and
    /// the next server starts from its own setting.
    pub fn disconnect(&mut self) {
        self.pop_all();
        self.reporting = false;
        self.replies.clear();
        self.prompt = Prompt::Pending;
    }

    pub fn push(&mut self, id: u128, url: &str, sha1: Option<String>) {
        if self.prompt == Prompt::Declined {
            self.report(id, action::DECLINED);
            return;
        }
        for pack in self.packs.iter_mut().filter(|p| p.id == id) {
            pack.remove(Removal::ServerReplaced);
        }
        self.packs.push(Pack {
            id,
            url: url.to_owned(),
            sha1,
            path: None,
            removal: None,
            download: Download::Requested,
            activation: Activation::Inactive,
            accepted: false,
        });
        if self.prompt == Prompt::Allowed {
            self.accept(self.packs.len() - 1);
        }
        self.update = true;
    }
    /// `ServerPackManager.pushLocalPack`: a pack already on disk, taken as
    /// downloaded (a world's own `resourcepacks/resources.zip`).
    pub fn push_local(&mut self, id: u128, path: PathBuf) {
        if self.prompt == Prompt::Declined {
            self.report(id, action::DECLINED);
            return;
        }
        for pack in self.packs.iter_mut().filter(|p| p.id == id) {
            pack.remove(Removal::ServerReplaced);
        }
        self.packs.push(Pack {
            id,
            // Never downloaded: the file is already on disk.
            url: String::new(),
            sha1: None,
            path: Some(path),
            removal: None,
            download: Download::Done,
            activation: Activation::Inactive,
            accepted: false,
        });
        if self.prompt == Prompt::Allowed {
            self.accept(self.packs.len() - 1);
        }
        self.update = true;
    }
    fn accept(&mut self, i: usize) {
        self.packs[i].accepted = true;
        let id = self.packs[i].id;
        self.report(id, action::ACCEPTED);
    }
    pub fn pop(&mut self, id: u128) {
        if let Some(pack) = self.packs.iter_mut().find(|p| !p.removed() && p.id == id) {
            pack.remove(Removal::ServerRemoved);
            self.update = true;
        }
    }
    pub fn pop_all(&mut self) {
        for pack in &mut self.packs {
            pack.remove(Removal::ServerRemoved);
        }
        self.update = true;
    }
    /// `allowServerPacks`: the prompt said yes (or the server is Enabled).
    pub fn allow(&mut self) {
        self.prompt = Prompt::Allowed;
        for i in 0..self.packs.len() {
            if !self.packs[i].accepted && !self.packs[i].removed() {
                self.accept(i);
            }
        }
        self.update = true;
    }
    /// `rejectServerPacks`: the prompt said no (or the server is Disabled).
    pub fn reject(&mut self) {
        self.prompt = Prompt::Declined;
        for pack in self.packs.iter_mut().filter(|p| !p.accepted) {
            pack.remove(Removal::Declined);
        }
        self.update = true;
    }

    /// `ServerPackManager.tick`: start the downloads the accepted packs
    /// need; once none is left, reload if the active packs should change;
    /// then forget the packs that are gone, reporting why.
    pub fn tick(&mut self) -> Tick {
        self.update = false;
        let mut out = Tick::default();
        let mut downloading = false;
        for pack in self.packs.iter_mut().filter(|p| !p.removed() && p.accepted) {
            downloading |= pack.download != Download::Done;
            if pack.download == Download::Requested {
                pack.download = Download::Pending;
                out.download.push(Request {
                    id: pack.id,
                    url: pack.url.clone(),
                    sha1: pack.sha1.clone(),
                });
            }
        }
        if !downloading {
            out.reload = self.reload_if_needed();
        }
        let mut gone = Vec::new();
        self.packs.retain(|p| {
            let keep = p.activation != Activation::Inactive || p.removal.is_none();
            if !keep && let Some(reply) = p.removal.and_then(Removal::reply) {
                gone.push((p.id, reply));
            }
            keep
        });
        for (id, reply) in gone {
            self.report(id, reply);
        }
        out
    }
    fn reload_if_needed(&mut self) -> Option<Vec<(u128, PathBuf)>> {
        if self.reload.is_some()
            || self
                .packs
                .iter()
                .any(|p| p.activation == Activation::Pending)
        {
            return None;
        }
        let mut changed = false;
        let (mut load, mut unload) = (Vec::new(), Vec::new());
        for pack in &self.packs {
            let wanted = pack.accepted && pack.download == Download::Done && !pack.removed();
            match pack.activation {
                Activation::Inactive if wanted => {
                    load.push(pack.id);
                    changed = true;
                }
                Activation::Active if wanted => load.push(pack.id),
                Activation::Active => {
                    unload.push(pack.id);
                    changed = true;
                }
                _ => {}
            }
        }
        if !changed {
            return None;
        }
        for pack in &mut self.packs {
            let loading = load.contains(&pack.id) && pack.activation != Activation::Active;
            if loading || unload.contains(&pack.id) {
                pack.activation = Activation::Pending;
            }
        }
        let paths = self.paths(&load);
        self.reload = Some(Reload {
            load,
            unload,
            recovery: false,
        });
        Some(paths)
    }
    /// The packs to load (`PackReloadConfig.Callbacks.packsToLoad`), bottom first.
    fn paths(&self, ids: &[u128]) -> Vec<(u128, PathBuf)> {
        ids.iter()
            .filter_map(|id| {
                let pack = self
                    .packs
                    .iter()
                    .find(|p| p.id == *id && p.path.is_some())?;
                Some((*id, pack.path.clone()?))
            })
            .collect()
    }

    /// `ServerPackManager.onDownload`.
    pub fn downloaded(&mut self, batch: Batch) {
        if !batch.failed.is_empty() {
            for pack in self
                .packs
                .iter_mut()
                .filter(|p| p.activation != Activation::Active)
            {
                pack.remove(if batch.failed.contains(&pack.id) {
                    Removal::DownloadFailed
                } else {
                    Removal::Discarded
                });
            }
        }
        let mut done = Vec::new();
        for (id, path) in batch.downloaded {
            if let Some(pack) = self
                .packs
                .iter_mut()
                .find(|p| p.id == id && p.download == Download::Pending)
            {
                pack.download = Download::Done;
                pack.path = Some(path);
                if !pack.removed() {
                    done.push(id);
                }
            }
        }
        for id in done {
            self.report(id, action::DOWNLOADED);
        }
        self.update = true;
    }

    /// The reload [`Tick::reload`] asked for finished (`Callbacks.onSuccess`),
    /// or its packs could not be loaded (`onFailure`). After a first failure
    /// the packs that were active are loaded again: the packs to reload with
    /// are returned, and this is called again with that reload's outcome.
    pub fn reloaded(&mut self, ok: bool) -> Option<Vec<(u128, PathBuf)>> {
        let mut reload = self.reload.take()?;
        self.update = true;
        if ok {
            let mut applied = Vec::new();
            for pack in &mut self.packs {
                if reload.load.contains(&pack.id) {
                    pack.activation = Activation::Active;
                    if pack.removal.is_none() {
                        applied.push(pack.id);
                    }
                } else if reload.unload.contains(&pack.id) {
                    pack.activation = Activation::Inactive;
                }
            }
            for id in applied {
                self.report(id, action::SUCCESSFULLY_LOADED);
            }
            return None;
        }
        if reload.recovery {
            // onFailure(true): nothing of the server's is loaded.
            for pack in &mut self.packs {
                if pack.activation == Activation::Pending {
                    pack.activation = Activation::Inactive;
                }
            }
            return None;
        }
        reload.load.clear();
        for pack in &mut self.packs {
            match pack.activation {
                Activation::Inactive => pack.remove(Removal::Discarded),
                Activation::Pending => {
                    pack.activation = Activation::Inactive;
                    pack.remove(Removal::ActivationFailed);
                }
                Activation::Active => reload.load.push(pack.id),
            }
        }
        reload.recovery = true;
        let paths = self.paths(&reload.load);
        self.reload = Some(reload);
        Some(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use action::*;

    fn ok(id: u128) -> Batch {
        Batch {
            downloaded: vec![(id, PathBuf::from(format!("/p/{id}")))],
            failed: vec![],
        }
    }
    /// Runs the updates as `createUpdateScheduler` does, downloading and
    /// reloading with `download` and `reload` as the game would.
    fn settle(
        sp: &mut ServerPacks,
        download: &mut dyn FnMut(&[Request]) -> Batch,
        reload: &mut dyn FnMut(&[(u128, PathBuf)]) -> bool,
    ) {
        let mut rounds = 0;
        while sp.wants_tick() {
            rounds += 1;
            assert!(rounds < 50);
            let t = sp.tick();
            if !t.download.is_empty() {
                let batch = download(&t.download);
                sp.downloaded(batch);
            }
            if let Some(mut packs) = t.reload {
                while let Some(back) = sp.reloaded(reload(&packs)) {
                    packs = back;
                }
            }
        }
    }

    #[test]
    fn an_enabled_server_s_pack_is_accepted_downloaded_and_loaded() {
        let mut sp = ServerPacks::default();
        sp.connect(Prompt::Allowed);
        sp.push(1, "http://a/p.zip", None);
        let mut loaded = Vec::new();
        settle(&mut sp, &mut |r| ok(r[0].id), &mut |p| {
            loaded = p.to_vec();
            true
        });
        assert_eq!(
            sp.take_replies(),
            [(1, ACCEPTED), (1, DOWNLOADED), (1, SUCCESSFULLY_LOADED)]
        );
        assert_eq!(loaded, [(1, PathBuf::from("/p/1"))]);
    }

    #[test]
    fn the_prompt_holds_packs_until_answered() {
        let mut sp = ServerPacks::default();
        sp.connect(Prompt::Pending);
        sp.push(1, "http://a/p.zip", None);
        settle(&mut sp, &mut |_| unreachable!(), &mut |_| unreachable!());
        assert!(sp.take_replies().is_empty());
        sp.reject();
        settle(&mut sp, &mut |_| unreachable!(), &mut |_| unreachable!());
        assert_eq!(sp.take_replies(), [(1, DECLINED)]);
        // Declined: later pushes are declined at once.
        sp.push(2, "http://a/q.zip", None);
        assert_eq!(sp.take_replies(), [(2, DECLINED)]);
    }

    #[test]
    fn a_failed_download_discards_the_rest_of_the_batch() {
        let mut sp = ServerPacks::default();
        sp.connect(Prompt::Allowed);
        sp.push(1, "http://a/p.zip", None);
        sp.push(2, "http://a/dead.zip", None);
        settle(
            &mut sp,
            &mut |_| Batch {
                downloaded: vec![(1, PathBuf::from("/p/1"))],
                failed: vec![2],
            },
            &mut |_| unreachable!(),
        );
        assert_eq!(
            sp.take_replies(),
            [
                (1, ACCEPTED),
                (2, ACCEPTED),
                (1, DISCARDED),
                (2, FAILED_DOWNLOAD)
            ]
        );
    }

    #[test]
    fn a_pack_that_does_not_load_fails_its_reload() {
        let mut sp = ServerPacks::default();
        sp.connect(Prompt::Allowed);
        sp.push(1, "http://a/p.zip", None);
        let mut reloads = Vec::new();
        settle(&mut sp, &mut |r| ok(r[0].id), &mut |p| {
            reloads.push(p.to_vec());
            reloads.len() > 1
        });
        assert_eq!(
            sp.take_replies(),
            [(1, ACCEPTED), (1, DOWNLOADED), (1, FAILED_RELOAD)]
        );
        // The recovery reload has none of the server's packs.
        assert_eq!(reloads[1], []);
    }

    #[test]
    fn popping_and_disconnecting_unload_the_packs() {
        let mut sp = ServerPacks::default();
        sp.connect(Prompt::Allowed);
        sp.push(1, "http://a/p.zip", None);
        sp.push(2, "http://a/q.zip", None);
        let last = std::cell::RefCell::new(Vec::new());
        let mut reload = |p: &[(u128, PathBuf)]| {
            *last.borrow_mut() = p.to_vec();
            true
        };
        settle(
            &mut sp,
            &mut |r| Batch {
                downloaded: r
                    .iter()
                    .map(|q| (q.id, PathBuf::from(format!("/p/{}", q.id))))
                    .collect(),
                failed: vec![],
            },
            &mut reload,
        );
        sp.take_replies();
        sp.pop(1);
        settle(&mut sp, &mut |_| unreachable!(), &mut reload);
        // Callbacks.onSuccess reports every pack the reload loaded, again.
        assert_eq!(sp.take_replies(), [(2, SUCCESSFULLY_LOADED)]);
        assert_eq!(*last.borrow(), [(2, PathBuf::from("/p/2"))]);
        sp.disconnect();
        settle(&mut sp, &mut |_| unreachable!(), &mut reload);
        assert!(last.borrow().is_empty());
        assert!(sp.packs.is_empty());
    }

    #[test]
    fn a_pushed_id_replaces_its_pack() {
        let mut sp = ServerPacks::default();
        sp.connect(Prompt::Allowed);
        sp.push(1, "http://a/old.zip", None);
        sp.push(1, "http://a/new.zip", None);
        let mut urls = Vec::new();
        settle(
            &mut sp,
            &mut |r| {
                urls.extend(r.iter().map(|q| q.url.clone()));
                ok(1)
            },
            &mut |_| true,
        );
        assert_eq!(urls, ["http://a/new.zip"]);
    }

    #[test]
    fn urls_and_hashes_are_read_as_vanilla_reads_them() {
        assert!(valid_url("https://example.com/p.zip"));
        assert!(valid_url("HTTP://example.com/p.zip"));
        assert!(!valid_url("ftp://example.com/p.zip"));
        assert!(!valid_url("example.com/p.zip"));
        let sha1 = "0123456789ABCDEF0123456789abcdef01234567";
        assert_eq!(parse_sha1(sha1), Some(sha1.to_ascii_lowercase()));
        assert_eq!(parse_sha1("abc"), None);
    }
}
