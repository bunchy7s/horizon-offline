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
fn opaque(rgb: u32) -> u32 {
    0xFF00_0000 | (rgb & 0xFF) << 16 | (rgb & 0xFF00) | (rgb >> 16 & 0xFF)
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

#[cfg(test)]
mod tests {
    use super::*;

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
