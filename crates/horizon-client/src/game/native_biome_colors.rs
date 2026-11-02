//! Biome colours a client mod changes (`native_biome_colors`), for seasons:
//! the grass, foliage, dry foliage and water tints the mesher blends, as a
//! resource pack's colormaps and biome effects set them, changed while the
//! game runs.

use super::Game;
use horizon_client::streaming::WorkerContext;
use std::sync::Arc;

// Native mods call it; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// Asks `native_biome_colors` again for every biome and, when the colours
    /// differ from those in use, rebuilds every loaded section and the far
    /// terrain with them (`LevelRenderer.allChanged`, as changing Biome Blend
    /// does). Call it when your colours change (a new season, a new day of a
    /// slow fade) and once when the player joins, not every frame: a rebuild
    /// takes the workers a few seconds. API 107.
    pub(super) fn native_refresh_biome_colors(&mut self) {
        let base = self.res.biomes().clone();
        let biomes = base
            .with_colors(|biome, colors| self.native_biome_colors(biome, colors))
            .map_or(base, Arc::new);
        if Arc::ptr_eq(&biomes, &self.pack.context.biomes) {
            return;
        }
        let old = &self.pack.context;
        let context = Arc::new(WorkerContext {
            assets: old.assets.clone(),
            render: old.render.clone(),
            tables: old.tables.clone(),
            biomes,
            light: old.light.clone(),
        });
        self.pack.context = context.clone();
        self.clear_lod();
        self.streaming.reload_resources(&mut self.world, context);
    }
}
