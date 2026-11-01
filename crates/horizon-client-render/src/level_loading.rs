//! LevelLoadingScreen: "Loading terrain..." over the chunk status map and
//! the server's load progress while a world opens or a portal is crossed.
use crate::{
    hud::{HudAtlas, HudBatch},
    text::Text,
};

/// `Minecraft.doWorldLoad`: `max(5, 3) + ChunkLevel.RADIUS_AROUND_FULL_CHUNK + 1`.
pub const STATUS_RADIUS: i32 = 8;
/// `LevelLoadProgressTracker.EXPECTED_PLAYER_CHUNKS`: the 7x7 chunks
/// `PrepareSpawnTask` waits for around the player.
const PLAYER_CHUNKS: usize = 49;
/// `LevelLoadProgressTracker.PREPARE_SERVER_WEIGHT`.
const PREPARE_WEIGHT: usize = 10;

/// `ChunkStatus`, as far as the chunk status map tells them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Empty,
    StructureStarts,
    StructureReferences,
    Biomes,
    Terrain,
    Features,
    InitializeLight,
    Light,
    Spawn,
    Full,
}
impl Status {
    /// `LevelLoadingScreen.COLORS` (RGB).
    pub fn color(self) -> u32 {
        match self {
            Self::Empty => 0x545454,
            Self::StructureStarts => 0x999999,
            Self::StructureReferences => 0x5F6191,
            Self::Biomes => 0x80B252,
            Self::Terrain => 0x303572,
            Self::Features => 0x21C600,
            Self::InitializeLight => 0xCCCCCC,
            Self::Light => 0xFFE0A0,
            Self::Spawn => 0xF26060,
            Self::Full => 0xFFFFFF,
        }
    }
}

/// `LevelLoadingScreen.Reason`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    NetherPortal,
    EndPortal,
    Other,
}

/// `LevelLoadTracker.CLIENT_WAIT_TIMEOUT_MS`.
pub const CLIENT_WAIT_TIMEOUT_MS: u64 = 30_000;
/// `LevelLoadTracker.LEVEL_LOAD_CLOSE_DELAY_MS`: how long a new world's loading
/// screen stays once the level is ready.
pub const LEVEL_LOAD_CLOSE_DELAY_MS: u64 = 500;

/// `ClientPacketListener.determineLevelLoadingReason`.
pub fn reason_for_respawn(player_died: bool, dimension: &str, old_dimension: &str) -> Reason {
    const NETHER: &str = "minecraft:the_nether";
    const END: &str = "minecraft:the_end";
    if player_died {
        Reason::Other
    } else if dimension == NETHER || old_dimension == NETHER {
        Reason::NetherPortal
    } else if dimension == END || old_dimension == END {
        Reason::EndPortal
    } else {
        Reason::Other
    }
}

/// `LevelLoadTracker.ClientState`, with the times in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClientState {
    /// `WaitingForServer`: until the loading start game event.
    WaitingForServer { timeout_after: u64 },
    /// `WaitingForPlayerChunk`: until the section around the player is shown.
    WaitingForPlayerChunk {
        player_section_ready: bool,
        timeout_after: u64,
    },
    /// `ClientLevelReady`.
    Ready { ready_at: u64 },
}

/// What `WaitingForPlayerChunk.isReady` asks of the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerCheck {
    /// `ClientLevel.isOutsideBuildHeight(camera y)`.
    pub outside_build_height: bool,
    pub spectator: bool,
    pub alive: bool,
}

/// The client half of `LevelLoadTracker`: what the loading screen waits for
/// after a login or a respawn, and the screen it is shown on.
#[derive(Clone, Copy, Debug)]
pub struct LevelLoadTracker {
    state: Option<ClientState>,
    close_delay_ms: u64,
    /// `LevelLoadingScreen.reason`.
    pub reason: Reason,
}
impl LevelLoadTracker {
    pub fn new(close_delay_ms: u64) -> Self {
        Self {
            state: None,
            close_delay_ms,
            reason: Reason::Other,
        }
    }
    /// `startClientLoad`, for the screen `reason` (`startWaitingForNewLevel`).
    pub fn start_client_load(&mut self, now: u64, reason: Reason) {
        self.state = Some(ClientState::WaitingForServer {
            timeout_after: now + CLIENT_WAIT_TIMEOUT_MS,
        });
        self.reason = reason;
    }
    /// The tracker's work is over and the screen has closed
    /// (`ClientPacketListener.tick` drops it once the level is ready).
    pub fn finish(&mut self) {
        self.state = None;
    }
    /// A load is under way, so its screen is up.
    pub fn is_active(&self) -> bool {
        self.state.is_some()
    }
    /// `loadingPacketsReceived`: `LEVEL_CHUNKS_LOAD_START` came.
    pub fn loading_packets_received(&mut self) {
        if let Some(ClientState::WaitingForServer { timeout_after }) = self.state {
            self.state = Some(ClientState::WaitingForPlayerChunk {
                player_section_ready: false,
                timeout_after,
            });
        }
    }
    /// `getPlayerCompiledSectionCallback` is not null: the renderer is to say
    /// when the player's section is shown.
    pub fn wants_player_section(&self) -> bool {
        matches!(self.state, Some(ClientState::WaitingForPlayerChunk { .. }))
    }
    /// The callback: the player's section is compiled and visible.
    pub fn player_section_ready(&mut self) {
        if let Some(ClientState::WaitingForPlayerChunk {
            player_section_ready,
            ..
        }) = &mut self.state
        {
            *player_section_ready = true;
        }
    }
    /// `tickClientLoad`.
    pub fn tick_client_load(&mut self, now: u64, player: PlayerCheck) {
        if let Some(ClientState::WaitingForPlayerChunk {
            player_section_ready,
            timeout_after,
        }) = self.state
        {
            let ready = if now > timeout_after {
                log::warn!(
                    "Timed out while waiting for the client to load chunks, letting the player into the world anyway"
                );
                true
            } else if !player.outside_build_height && !player.spectator && player.alive {
                player_section_ready
            } else {
                true
            };
            if ready {
                self.state = Some(ClientState::Ready { ready_at: now });
            }
        }
    }
    /// `isLevelReady`.
    pub fn is_level_ready(&self, now: u64) -> bool {
        matches!(self.state, Some(ClientState::Ready { ready_at }) if now >= ready_at + self.close_delay_ms)
    }
}

/// `LevelLoadListener.Stage`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    StartServer,
    PrepareGlobalSpawn,
    LoadInitialChunks,
    LoadPlayerChunks,
}

/// `LevelLoadProgressTracker` (with `includePlayerChunks`).
#[derive(Debug, Default)]
struct ProgressTracker {
    total_weight: usize,
    finalized_weight: usize,
    segment_weight: usize,
    segment_fraction: f32,
    progress: f32,
}
impl ProgressTracker {
    fn tracks(stage: Stage) -> bool {
        matches!(stage, Stage::LoadInitialChunks | Stage::LoadPlayerChunks)
    }
    fn begin_segment(&mut self, weight: usize) {
        self.segment_weight = weight;
        self.segment_fraction = 0.0;
        self.update_progress();
    }
    fn finish_segment(&mut self) {
        self.finalized_weight += self.segment_weight;
        self.segment_weight = 0;
        self.update_progress();
    }
    fn update_progress(&mut self) {
        self.progress = if self.total_weight == 0 {
            0.0
        } else {
            (self.finalized_weight as f32 + self.segment_fraction * self.segment_weight as f32)
                / self.total_weight as f32
        };
    }
    fn start(&mut self, stage: Stage, total_chunks: usize) {
        match stage {
            Stage::LoadInitialChunks => {
                self.total_weight = PREPARE_WEIGHT + total_chunks + PLAYER_CHUNKS;
                self.begin_segment(PREPARE_WEIGHT);
                self.finish_segment();
                self.begin_segment(total_chunks);
            }
            Stage::LoadPlayerChunks => self.begin_segment(PLAYER_CHUNKS),
            _ => {}
        }
    }
    fn update(&mut self, stage: Stage, current: usize, total: usize) {
        if Self::tracks(stage) {
            self.segment_fraction = if total == 0 {
                0.0
            } else {
                current as f32 / total as f32
            };
            self.update_progress();
        }
    }
    fn finish(&mut self, stage: Stage) {
        if Self::tracks(stage) {
            self.finish_segment();
        }
    }
}

/// The integrated server's side of `LevelLoadTracker`: what its
/// `LevelLoadListener` reported (`serverProgress`, `hasProgress`) and the
/// chunk status map around the load focus (`ChunkLoadStatusView`), which the
/// server thread writes and the window's loading screen reads.
#[derive(Debug, Default)]
pub struct ServerLoadView {
    tracker: std::sync::Mutex<ProgressTracker>,
    /// `serverStage != null`.
    has_progress: std::sync::atomic::AtomicBool,
    /// `statusView.get(x, z)` for x and z in `0..=2 * STATUS_RADIUS`.
    cells: std::sync::Mutex<Vec<Option<Status>>>,
}
impl ServerLoadView {
    /// `LevelLoadListener.start`.
    pub fn start(&self, stage: Stage, total_chunks: usize) {
        Self::lock(&self.tracker).start(stage, total_chunks);
        // `serverStage = stage`, whatever the stage.
        self.has_progress
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    /// `LevelLoadListener.update`.
    pub fn update(&self, stage: Stage, current: usize, total: usize) {
        Self::lock(&self.tracker).update(stage, current, total);
    }
    /// `LevelLoadListener.finish`.
    pub fn finish(&self, stage: Stage) {
        Self::lock(&self.tracker).finish(stage);
    }
    /// `LevelLoadTracker.hasProgress`.
    pub fn has_progress(&self) -> bool {
        self.has_progress.load(std::sync::atomic::Ordering::Relaxed)
    }
    /// `LevelLoadTracker.serverProgress`.
    pub fn server_progress(&self) -> f32 {
        Self::lock(&self.tracker).progress
    }
    /// The map `ChunkLoadStatusView.get` reads, replaced as a whole.
    pub fn set_status(&self, cells: Vec<Option<Status>>) {
        *Self::lock(&self.cells) = cells;
    }
    /// `ChunkLoadStatusView.get`.
    pub fn status(&self, x: i32, z: i32) -> Option<Status> {
        let diameter = (STATUS_RADIUS * 2 + 1) as usize;
        let (x, z) = (usize::try_from(x).ok()?, usize::try_from(z).ok()?);
        if x >= diameter || z >= diameter {
            return None;
        }
        Self::lock(&self.cells)
            .get(x * diameter + z)
            .copied()
            .flatten()
    }
    fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// `LevelLoadProgressTracker.get` once the server is prepared, with `ready`
/// of the player's 49 chunks loaded (no forced chunks to load first).
pub fn progress(ready: usize) -> f32 {
    let total = PREPARE_WEIGHT + PLAYER_CHUNKS;
    (PREPARE_WEIGHT + ready.min(PLAYER_CHUNKS)) as f32 / total as f32
}

/// `LevelLoadingScreen.tick`: the bar eases a fifth of the way each tick.
pub fn smooth(smoothed: f32, progress: f32) -> f32 {
    smoothed + (progress - smoothed) * 0.2
}

/// `ARGB.opaque` as the batch's RGBA.
pub const fn opaque(rgb: u32) -> u32 {
    0xFF00_0000 | (rgb & 0xFF) << 16 | (rgb & 0xFF00) | (rgb >> 16 & 0xFF)
}

/// `LoadingOverlay.LOGO_BACKGROUND_COLOR` (RGB: 239, 50, 61).
pub const LOGO_BACKGROUND_COLOR: u32 = opaque(0xEF323D);
/// `LoadingOverlay.LOGO_BACKGROUND_COLOR_DARK` (black).
pub const LOGO_BACKGROUND_COLOR_DARK: u32 = opaque(0x000000);

/// `LoadingOverlay.BRAND_BACKGROUND`: black when `darkMojangStudiosBackground` is set,
/// Mojang red otherwise.
pub fn brand_background(dark: bool) -> u32 {
    if dark {
        LOGO_BACKGROUND_COLOR_DARK
    } else {
        LOGO_BACKGROUND_COLOR
    }
}

/// `LevelLoadingScreen.extractRenderState` over its background: the text,
/// the progress bar and the chunk map, `status(x, z)` for x and z in
/// `0..=2 * STATUS_RADIUS` from the map's north-west corner.
pub fn draw(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    status: Option<&dyn Fn(i32, i32) -> Option<Status>>,
    progress: Option<f32>,
) {
    let (x_center, y_center) = ((batch.width as i32) / 2, (batch.height as i32) / 2);
    let text_top = match status {
        Some(status) => {
            // extractChunksForRendering(size 2, margin 0).
            let diameter = STATUS_RADIUS * 2 + 1;
            let start = (x_center - diameter, y_center - diameter);
            for x in 0..diameter {
                for z in 0..diameter {
                    let color = opaque(status(x, z).map_or(0, Status::color));
                    let (cx, cy) = (start.0 + x * 2, start.1 + z * 2);
                    batch.fill(atlas, cx as f32, cy as f32, 2.0, 2.0, color);
                }
            }
            y_center - STATUS_RADIUS * 2 - 9 * 3
        }
        None => y_center - 50,
    };
    let text = Text::translate("multiplayer.downloadingTerrain", &[]);
    let tw = atlas.font.width_text(&text) as i32;
    batch.text_styled(
        atlas,
        &text,
        (x_center - tw / 2) as f32,
        text_top as f32,
        u32::MAX,
        true,
    );
    if let Some(progress) = progress {
        // drawProgressBar(xCenter - 100, textTop + 12, 200, 2).
        let (left, top) = ((x_center - 100) as f32, (text_top + 9 + 3) as f32);
        batch.fill(atlas, left, top, 200.0, 2.0, opaque(0));
        let done = (progress * 200.0).round();
        batch.fill(atlas, left, top, done, 2.0, opaque(0x00FF00));
    }
}

/// `LoadingOverlay.MOJANG_STUDIOS_LOGO_LOCATION`, from the player's own resources.
pub const MOJANG_STUDIOS_LOGO: &str = "gui/title/mojangstudios";

/// `LoadingOverlay.extractRenderState`'s logo on a GUI `w` x `h`: the left
/// half's corner, then each half's width and height.
pub fn logo_layout(w: f32, h: f32) -> (i32, i32, i32, i32) {
    let (w, h) = (f64::from(w), f64::from(h));
    let (content_x, logo_y) = ((w * 0.5) as i32, (h * 0.5) as i32);
    let logo_height = (w * 0.75).min(h) * 0.25;
    let half_height = (logo_height * 0.5) as i32;
    let half_width = (logo_height * 4.0 * 0.5) as i32;
    (
        content_x - half_width,
        logo_y - half_height,
        half_width,
        logo_height as i32,
    )
}

/// `GuiGraphicsExtractor.fill(x0, y0, x1, y1, color)`: the corners in either order.
fn fill_corners(batch: &mut HudBatch, atlas: &HudAtlas, [x0, y0, x1, y1]: [i32; 4], color: u32) {
    let (left, right) = (x0.min(x1), x0.max(x1));
    let (top, bottom) = (y0.min(y1), y0.max(y1));
    let (w, h) = ((right - left) as f32, (bottom - top) as f32);
    batch.fill(atlas, left as f32, top as f32, w, h, color);
}

/// `LoadingOverlay.extractRenderState`: fills the frame with `BRAND_BACKGROUND`
/// (Mojang red, or black when `darkMojangStudiosBackground` is set), draws the
/// Mojang Studios logo as its two halves side by side, and the progress bar
/// at 83.25% of the height (`extractProgressBar`, unfaded), with the stage's
/// line above it. The logo's `MOJANG_LOGO` pipeline adds it (`SRC_ALPHA, ONE`):
/// its texels are white, which alpha blending matches but for the red channel
/// of the logo's soft edges, a 16/255 difference at most.
pub fn draw_loading_overlay(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    line: &str,
    progress: f32,
    dark: bool,
) {
    let (w, h) = (batch.width, batch.height);
    batch.fill(atlas, 0.0, 0.0, w, h, brand_background(dark));
    let (x, y, half_width, logo_height) = logo_layout(w, h);
    if let Some(r) = atlas.sprites.get(MOJANG_STUDIOS_LOGO).copied() {
        // blit(.., u, v, width, height, 120, 60, 120, 120): the left half
        // samples the texture's top half from u = -0.0625, the right half its
        // bottom half from u = 0.0625 (clamped, as the logo's sampler is).
        let at = |u: f32, v: f32| {
            let (u, v) = ((u / 120.0).clamp(0.0, 1.0), v / 120.0);
            (r.u0 + (r.u1 - r.u0) * u, r.v0 + (r.v1 - r.v0) * v)
        };
        for (i, (u, v)) in [(-0.0625, 0.0), (0.0625, 60.0)].into_iter().enumerate() {
            let (u0, v0) = at(u, v);
            let (u1, v1) = at(u + 120.0, v + 60.0);
            let half = crate::hud::Rect {
                u0,
                v0,
                u1,
                v1,
                ..r
            };
            let left = x + i as i32 * half_width;
            let (lw, lh) = (half_width as f32, logo_height as f32);
            batch.rect_uv(left as f32, y as f32, lw, lh, half, u32::MAX);
        }
    }
    let bar_y = (f64::from(h) * 0.8325) as i32;
    let x_center = (w as i32) / 2;
    let [x0, y0, x1, y1] = [
        x_center - half_width,
        bar_y - 5,
        x_center + half_width,
        bar_y + 5,
    ];
    let filled = (((x1 - x0) - 2) as f32 * progress.clamp(0.0, 1.0)).ceil() as i32;
    let white = u32::MAX;
    fill_corners(batch, atlas, [x0 + 2, y0 + 2, x0 + filled, y1 - 2], white);
    fill_corners(batch, atlas, [x0 + 1, y0, x1 - 1, y0 + 1], white);
    fill_corners(batch, atlas, [x0 + 1, y1, x1 - 1, y1 - 1], white);
    fill_corners(batch, atlas, [x0, y0, x0 + 1, y1], white);
    fill_corners(batch, atlas, [x1, y0, x1 - 1, y1], white);
    let text = Text::literal(line);
    let tw = atlas.font.width_text(&text) as i32;
    let text_top = (y0 - 9 - 3) as f32;
    batch.text_styled(
        atlas,
        &text,
        (x_center - tw / 2) as f32,
        text_top,
        white,
        true,
    );
}

/// The startup loading screen: the stage's line and a progress bar, centred
/// like `LevelLoadingScreen`'s text and bar.
pub fn draw_progress(batch: &mut HudBatch, atlas: &HudAtlas, line: &str, progress: f32) {
    let (x_center, y_center) = ((batch.width as i32) / 2, (batch.height as i32) / 2);
    let text_top = y_center - 50;
    let text = Text::literal(line);
    let tw = atlas.font.width_text(&text) as i32;
    batch.text_styled(
        atlas,
        &text,
        (x_center - tw / 2) as f32,
        text_top as f32,
        u32::MAX,
        true,
    );
    let (left, top) = ((x_center - 100) as f32, (text_top + 9 + 3) as f32);
    batch.fill(atlas, left, top, 200.0, 2.0, opaque(0));
    let done = (progress.clamp(0.0, 1.0) * 200.0).round();
    batch.fill(atlas, left, top, done, 2.0, opaque(0x00FF00));
}

/// `ConnectScreen`'s Cancel button corner on a GUI `w` x `h`:
/// (width / 2 - 100, height / 4 + 120 + 12), 200 x 20.
pub fn connect_cancel(w: f32, h: f32) -> (f32, f32) {
    ((w / 2.0).trunc() - 100.0, (h / 4.0).trunc() + 132.0)
}

/// `ConnectScreen.extractRenderState` over the menu background: the status
/// centred at (width / 2, height / 2 - 50) and, while the connection can
/// still be cancelled, Cancel.
pub fn draw_connecting(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    status: &Text,
    cursor: (f32, f32),
    cancel: bool,
) {
    let (w, h) = (batch.width, batch.height);
    crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
    let tw = atlas.font.width_text(status);
    let (x, y) = (((w - tw) / 2.0).trunc(), (h / 2.0).trunc() - 50.0);
    batch.text_styled(atlas, status, x, y, u32::MAX, true);
    if cancel {
        let (x, y) = connect_cancel(w, h);
        let hover = crate::ui::hovered(cursor, x, y, 200.0, 20.0);
        let label = Text::translate("gui.cancel", &[]);
        crate::ui::button(batch, atlas, x, y, 200.0, 20.0, &label, true, hover);
    }
}

/// `FileFixerProgressScreen`'s Cancel button corner on a GUI `w` x `h`:
/// ((width - 200) / 2, height / 2 + 100), 200 x 20.
pub fn file_fixer_cancel(w: f32, h: f32) -> (f32, f32) {
    (((w as i32 - 200) / 2) as f32, (h as i32 / 2 + 100) as f32)
}

/// `GuiGraphicsExtractor.centeredText` with the screen's grey (`-6250336`) or white.
fn centered_line(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    text: &Text,
    x_center: i32,
    y: i32,
    color: u32,
) {
    let tw = atlas.font.width_text(text) as i32;
    batch.text_styled(
        atlas,
        text,
        (x_center - tw / 2) as f32,
        y as f32,
        color,
        true,
    );
}

/// `FileFixerProgressScreen.extractProgressBar`.
fn file_fixer_bar(batch: &mut HudBatch, atlas: &HudAtlas, x_center: i32, y: i32, progress: f32) {
    let left = x_center - 100;
    batch.fill(atlas, left as f32, y as f32, 200.0, 2.0, opaque(0));
    batch.fill(
        atlas,
        left as f32,
        y as f32,
        (progress * 200.0).round(),
        2.0,
        opaque(0x00FF00),
    );
}

/// `FileFixerProgressScreen.extractRenderState` (with `extractProgress`,
/// `extractProgressBar`, `extractFileStats`, `extractFileFixerCount`,
/// `extractTypeText`, `extractTypeProgress` and `extractScanning`) over the menu background: the
/// title, then either the scanning line or the running fixer's bar, the
/// totals, the stage, the type's label and its bar and percentage; and
/// Cancel, inactive once pressed.
pub fn draw_file_fixer(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    progress: &horizon_world::datafix::progress::UpgradeProgress,
    cursor: (f32, f32),
) {
    const GREY: u32 = 0xFFA0_A0A0;
    let (w, h) = (batch.width, batch.height);
    crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
    let (x_center, y_center) = ((w as i32) / 2, (h as i32) / 2);
    let text_top = y_center - 50;
    centered_line(
        batch,
        atlas,
        &Text::translate("upgradeWorld.title", &[]),
        x_center,
        text_top,
        u32::MAX,
    );
    let total = progress.total_file_fix_stats();
    if total.total_operations() > 0 {
        let of_type = progress.type_file_fix_stats();
        let running = progress.running_file_fixer_stats();
        let mut y = text_top + 9 + 3;
        file_fixer_bar(batch, atlas, x_center, y, running.progress());
        y += 7;
        // extractFileStats
        let converted = Text::tr(
            "upgradeWorld.info.converted",
            &[&total.finished_operations()],
        );
        centered_line(batch, atlas, &converted, x_center, y, GREY);
        let sum = Text::tr("upgradeWorld.info.total", &[&total.total_operations()]);
        centered_line(batch, atlas, &sum, x_center, y + 9 + 3, GREY);
        y += 9 * 2 + 6;
        // extractFileFixerCount
        let stage = Text::tr(
            "upgradeWorld.info.file_fix_stage",
            &[&running.finished_operations(), &running.total_operations()],
        );
        centered_line(batch, atlas, &stage, x_center, y, GREY);
        y += 9 + 30 - 5;
        // extractTypeText
        if let Some(ty) = progress.upgrade_type() {
            let label = Text::translate(&ty.label_key(), &[]);
            centered_line(batch, atlas, &label, x_center, y, GREY);
        }
        y += 9 + 3;
        file_fixer_bar(batch, atlas, x_center, y, of_type.progress());
        y += 7;
        // extractTypeProgress: Mth.floor(progress * 100)
        let percent = (of_type.progress() * 100.0).floor() as i32;
        let text = Text::tr("upgradeWorld.progress.percentage", &[&percent]);
        centered_line(batch, atlas, &text, x_center, y, GREY);
    } else {
        // extractScanning
        let scanning = Text::translate("upgradeWorld.info.scanning", &[]);
        centered_line(batch, atlas, &scanning, x_center, text_top + 9 + 3, GREY);
    }
    let (x, y) = file_fixer_cancel(w, h);
    let active = !progress.is_canceled();
    let hover = crate::ui::hovered(cursor, x, y, 200.0, 20.0);
    let label = Text::translate("gui.cancel", &[]);
    crate::ui::button(batch, atlas, x, y, 200.0, 20.0, &label, active, hover);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mojang_logo_is_a_quarter_of_the_shorter_side_and_four_times_as_wide() {
        // 320x240: min(240, 240) / 4 = 60 high, two halves of 120.
        assert_eq!(logo_layout(320.0, 240.0), (40, 90, 120, 60));
        // A tall frame sizes the logo by three quarters of its width.
        assert_eq!(logo_layout(400.0, 500.0), (50, 213, 150, 75));
    }

    #[test]
    fn progress_weighs_the_prepared_server_and_the_player_chunks() {
        assert_eq!(progress(0), 10.0 / 59.0);
        assert_eq!(progress(49), 1.0);
        assert_eq!(progress(100), 1.0);
        let eased = smooth(0.0, 1.0);
        assert_eq!(eased, 0.2);
        assert!((smooth(eased, 1.0) - 0.36).abs() < 1e-6);
    }

    #[test]
    fn brand_background_matches_dark_mojang_studios_setting() {
        assert_eq!(brand_background(true), LOGO_BACKGROUND_COLOR_DARK);
        assert_eq!(brand_background(false), LOGO_BACKGROUND_COLOR);
        assert_eq!(LOGO_BACKGROUND_COLOR_DARK, 0xFF00_0000);
        assert_eq!(LOGO_BACKGROUND_COLOR, crate::hud::rgba(239, 50, 61, 255));
    }

    #[test]
    fn file_fixer_cancel_sits_below_the_centre() {
        assert_eq!(file_fixer_cancel(427.0, 240.0), (113.0, 220.0));
        assert_eq!(file_fixer_cancel(426.0, 241.0), (113.0, 220.0));
    }

    #[test]
    fn colours_are_opaque_rgba() {
        assert_eq!(opaque(Status::Full.color()), u32::MAX);
        assert_eq!(opaque(0), 0xFF00_0000);
        assert_eq!(opaque(0x00FF00), crate::hud::rgba(0, 255, 0, 255));
        assert_eq!(
            opaque(Status::Biomes.color()),
            crate::hud::rgba(0x80, 0xB2, 0x52, 255)
        );
    }
}

#[cfg(test)]
mod tracker_tests {
    use super::*;

    const ALIVE: PlayerCheck = PlayerCheck {
        outside_build_height: false,
        spectator: false,
        alive: true,
    };

    #[test]
    fn server_progress_follows_the_listener_like_the_progress_tracker() {
        let view = ServerLoadView::default();
        assert!(!view.has_progress() && view.server_progress() == 0.0);
        // A stage the tracker ignores still counts as progress being made.
        view.start(Stage::StartServer, 0);
        assert!(view.has_progress() && view.server_progress() == 0.0);
        // prepareLevels with no forced chunks: the 10 of preparing, of 59.
        view.start(Stage::LoadInitialChunks, 0);
        view.finish(Stage::LoadInitialChunks);
        assert_eq!(view.server_progress(), 10.0 / 59.0);
        // PrepareSpawnTask: the 49 player chunks.
        view.start(Stage::LoadPlayerChunks, 49);
        view.update(Stage::LoadPlayerChunks, 20, 49);
        assert_eq!(view.server_progress(), (10.0 + 20.0) / 59.0);
        assert_eq!(view.server_progress(), progress(20));
        view.update(Stage::LoadPlayerChunks, 49, 49);
        view.finish(Stage::LoadPlayerChunks);
        assert_eq!(view.server_progress(), 1.0);
    }

    #[test]
    fn the_status_map_is_read_from_the_north_west_corner() {
        let view = ServerLoadView::default();
        assert_eq!(view.status(0, 0), None);
        let diameter = (STATUS_RADIUS * 2 + 1) as usize;
        let mut cells = vec![None; diameter * diameter];
        cells[3 * diameter + 5] = Some(Status::Full);
        view.set_status(cells);
        assert_eq!(view.status(3, 5), Some(Status::Full));
        assert_eq!(view.status(5, 3), None);
        assert_eq!(view.status(-1, 0), None);
        assert_eq!(view.status(diameter as i32, 0), None);
    }

    #[test]
    fn the_load_waits_for_the_server_then_the_players_section() {
        let mut tracker = LevelLoadTracker::new(0);
        assert!(!tracker.is_active());
        tracker.start_client_load(1_000, Reason::NetherPortal);
        assert!(tracker.is_active() && tracker.reason == Reason::NetherPortal);
        // Nothing is asked of the renderer before the start event, and a
        // section shown early counts for nothing.
        assert!(!tracker.wants_player_section());
        tracker.player_section_ready();
        tracker.tick_client_load(1_050, ALIVE);
        assert!(!tracker.is_level_ready(1_050));
        tracker.loading_packets_received();
        assert!(tracker.wants_player_section());
        tracker.tick_client_load(1_100, ALIVE);
        assert!(!tracker.is_level_ready(1_100));
        tracker.player_section_ready();
        tracker.tick_client_load(1_150, ALIVE);
        assert!(tracker.is_level_ready(1_150));
        tracker.finish();
        assert!(!tracker.is_active() && !tracker.is_level_ready(2_000));
    }

    #[test]
    fn a_player_who_cannot_see_the_section_does_not_wait_for_it() {
        for player in [
            PlayerCheck {
                outside_build_height: true,
                ..ALIVE
            },
            PlayerCheck {
                spectator: true,
                ..ALIVE
            },
            PlayerCheck {
                alive: false,
                ..ALIVE
            },
        ] {
            let mut tracker = LevelLoadTracker::new(0);
            tracker.start_client_load(0, Reason::Other);
            tracker.loading_packets_received();
            tracker.tick_client_load(10, player);
            assert!(tracker.is_level_ready(10));
        }
    }

    #[test]
    fn the_wait_times_out_after_thirty_seconds_but_only_once_the_server_has_started() {
        let mut tracker = LevelLoadTracker::new(0);
        tracker.start_client_load(0, Reason::Other);
        tracker.tick_client_load(CLIENT_WAIT_TIMEOUT_MS + 1, ALIVE);
        assert!(!tracker.is_level_ready(CLIENT_WAIT_TIMEOUT_MS + 1));
        tracker.loading_packets_received();
        tracker.tick_client_load(CLIENT_WAIT_TIMEOUT_MS, ALIVE);
        assert!(!tracker.is_level_ready(CLIENT_WAIT_TIMEOUT_MS));
        tracker.tick_client_load(CLIENT_WAIT_TIMEOUT_MS + 1, ALIVE);
        assert!(tracker.is_level_ready(CLIENT_WAIT_TIMEOUT_MS + 1));
    }

    #[test]
    fn a_new_worlds_screen_stays_for_the_close_delay() {
        let mut tracker = LevelLoadTracker::new(LEVEL_LOAD_CLOSE_DELAY_MS);
        tracker.start_client_load(0, Reason::Other);
        tracker.loading_packets_received();
        tracker.player_section_ready();
        tracker.tick_client_load(100, ALIVE);
        assert!(!tracker.is_level_ready(599));
        assert!(tracker.is_level_ready(600));
    }

    #[test]
    fn a_respawn_names_its_screen_by_the_dimensions() {
        let (overworld, nether, end) = (
            "minecraft:overworld",
            "minecraft:the_nether",
            "minecraft:the_end",
        );
        assert_eq!(
            reason_for_respawn(false, nether, overworld),
            Reason::NetherPortal
        );
        assert_eq!(
            reason_for_respawn(false, overworld, nether),
            Reason::NetherPortal
        );
        assert_eq!(reason_for_respawn(false, end, overworld), Reason::EndPortal);
        assert_eq!(reason_for_respawn(false, end, nether), Reason::NetherPortal);
        assert_eq!(
            reason_for_respawn(false, overworld, overworld),
            Reason::Other
        );
        // A death never shows a portal.
        assert_eq!(reason_for_respawn(true, nether, overworld), Reason::Other);
    }
}
