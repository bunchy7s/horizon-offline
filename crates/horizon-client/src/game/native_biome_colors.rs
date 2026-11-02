//! Biome colours a client mod changes (`native_biome_colors`), for seasons:
//! the grass, foliage, dry foliage and water tints the mesher blends, as a
//! resource pack's colormaps and biome effects set them, changed while the
//! game runs.

use super::Game;
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
        // A world's own table is asked from the biome files' colours each
        // time, so a mod's colours do not stack; a plain world's is the pack's.
        if let (Some(base), Some(current)) = (&self.world_biomes_base, &self.world_biomes) {
            let Some(biomes) = base.recolored_from(current, |biome, colors| {
                self.native_biome_colors(biome, colors)
            }) else {
                return;
            };
            self.world_biomes = Some(Arc::new(biomes));
        } else {
            let base = self.res.biomes().clone();
            let biomes = base
                .with_colors(|biome, colors| self.native_biome_colors(biome, colors))
                .map_or_else(|| base.clone(), Arc::new);
            if Arc::ptr_eq(&biomes, &self.pack.context.biomes) {
                return;
            }
            self.pack.context = self.pack.context_with(biomes);
        }
        let context = self.worker_context();
        self.clear_lod();
        self.streaming.reload_resources(&mut self.world, context);
    }
}
