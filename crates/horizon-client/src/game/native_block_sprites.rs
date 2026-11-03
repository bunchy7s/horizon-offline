//! Where blocks' textures are in the block sprite arrays, for a client mod drawing blocks with the player's own
//! textures (resource packs included) in its own pass: with `WorldDrawn::block_sprites`, the arrays themselves.
use super::Game;
use horizon_client::block_sprites::{BlockSprites, Lookup, SpriteRef};

#[allow(dead_code)]
impl Game<'_> {
    /// For a client mod: what each block (`minecraft:grass_block`, `oak_log[axis=x]`; properties left out take the
    /// block's default) draws on each of its six sides, as the mesher bakes its model: each face's sprite (array and
    /// layer, as `WorldDrawn::block_sprites` cuts them), where it lies on the side, how its texture runs across it,
    /// its tint source and whether it is cut out, translucent or animated; its particle sprite; whether it is a plain
    /// cube. None for a block the game does not know. The model is the state's first random variant. API
    /// @API:native_block_sprites@.
    pub(super) fn native_block_sprites(&self, blocks: &[&str]) -> Vec<Option<BlockSprites>> {
        let lookup = self.native_sprite_lookup();
        blocks.iter().map(|block| lookup.block(block)).collect()
    }
    /// For a client mod: each texture's sprite by name (`minecraft:block/stone`, `block/stone`), none for one no
    /// model uses. API @API:native_block_sprites@.
    pub(super) fn native_texture_sprites(&self, textures: &[&str]) -> Vec<Option<SpriteRef>> {
        let lookup = self.native_sprite_lookup();
        textures
            .iter()
            .map(|texture| lookup.texture(texture))
            .collect()
    }
    /// For a client mod: changes each time the resources load (a resource pack turned on or off can move every
    /// sprite): ask `native_block_sprites` again then. API @API:native_block_sprites@.
    pub(super) fn native_block_sprites_version(&self) -> u64 {
        self.pack.context.render.generation
    }
    fn native_sprite_lookup(&self) -> Lookup<'_> {
        // (no renderer, no arrays: every layer counts as array 0's)
        let split = self
            .renderer
            .as_ref()
            .map_or(u32::MAX, |r| r.sprite_split());
        Lookup::new(&self.pack.context, split)
    }
}
