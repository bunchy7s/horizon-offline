//! Shulker boxes (`ShulkerBoxBlockEntity`: 27 slots that refuse other boxes,
//! kept in the dropped item's `container` component) and ender chests (the
//! player's `PlayerEnderChestContainer`, saved as `EnderItems`).
use super::Game;
use horizon_client::{
    containers::{BlockEntity, ChestHalf, is_chest_block},
    items::ItemStack,
    text::Text,
};
use horizon_core::random::RandomSource;

pub(super) fn is_shulker_box(name: &str) -> bool {
    name.ends_with("shulker_box")
}
impl Game<'_> {
    /// `ServerPlayerGameMode.useItemOn` for a spectator: the block's menu
    /// (`BlockState.getMenuProvider`) opens, and nothing else happens. The
    /// lid, its sound and the `container_open` game event come from
    /// `ContainerOpenersCounter`, which spectators never join
    /// (`ChestBlockEntity.startOpen`); the stat, the piglins' anger and the
    /// shulker box's room check are `useWithoutItem`'s, which is not called.
    /// Only the block containers whose clicks the server takes back from a
    /// spectator (`handleContainerClick`) open here: storage, the furnaces,
    /// the brewing stand and the crafter.
    pub(super) fn spectator_open_menu(&mut self, pos: [i32; 3]) {
        let key = (pos[0], pos[1], pos[2]);
        let name = self
            .res
            .facts()
            .get(self.world.block(pos[0], pos[1], pos[2]))
            .name
            .clone();
        let container = is_shulker_box(&name)
            || matches!(
                name.trim_start_matches("minecraft:"),
                "chest"
                    | "trapped_chest"
                    | "barrel"
                    | "hopper"
                    | "dispenser"
                    | "dropper"
                    | "furnace"
                    | "blast_furnace"
                    | "smoker"
                    | "brewing_stand"
                    | "crafter"
            );
        // ChestBlock.getMenuProvider: none while either half is blocked.
        if !container || self.chest_menu_blocked(key, &name) {
            return;
        }
        let Some(entity) = BlockEntity::for_block(&name) else {
            return;
        };
        self.world.block_entities.get_or_insert_with(key, || entity);
        self.container = Some(key);
        self.screen = super::Screen::Inventory;
        self.capture(false);
        self.right = false;
    }
    /// The custom name of the open container, or its default title
    /// (a double chest is named by whichever half has a name).
    pub(super) fn container_title(&self, default: String) -> String {
        self.container_parts()
            .iter()
            .find_map(|(key, _)| self.world.block_names.get(key))
            .cloned()
            .unwrap_or(default)
    }
    /// ShulkerBoxSlot.mayPlace: no shulker box inside a shulker box.
    pub(super) fn refuses(&self, slot: usize, stack: &ItemStack) -> bool {
        (300..400).contains(&slot)
            && is_shulker_box(&stack.item)
            && self.container.is_some_and(|k| {
                is_shulker_box(&self.res.facts().get(self.world.block(k.0, k.1, k.2)).name)
            })
    }
    /// Opening an ender chest shows the player's ender inventory in its place.
    pub(super) fn open_ender_chest(&mut self, key: (i32, i32, i32)) {
        self.world
            .block_entities
            .insert(key, BlockEntity::Chest(self.ender_items.clone()));
        self.container = Some(key);
        self.ender_open = Some(key);
    }
    /// The block above `key` is a redstone conductor.
    fn conductor_above(&self, key: (i32, i32, i32)) -> bool {
        let above = self.world.block(key.0, key.1 + 1, key.2);
        self.res.behaviours().facts(above).flags
            & horizon_world::level::data::state_flags::CONDUCTOR
            != 0
    }
    /// `ChestBlock.isChestBlockedAt`: a redstone conductor above the chest or
    /// a cat sitting on it.
    fn chest_blocked_at(&self, key: (i32, i32, i32)) -> bool {
        if self.conductor_above(key) {
            return true;
        }
        // isCatSittingOnChest: a cat whose box meets the block above.
        let area = horizon_core::geometry::Aabb::new(
            horizon_core::geometry::Vec3::new(
                f64::from(key.0),
                f64::from(key.1 + 1),
                f64::from(key.2),
            ),
            horizon_core::geometry::Vec3::new(
                f64::from(key.0 + 1),
                f64::from(key.1 + 2),
                f64::from(key.2 + 1),
            ),
        );
        self.mobs.iter().any(|m| {
            m.species() == horizon_client::mobs::Species::Cat
                && m.alive()
                && horizon_client::mobs::tame::cat_sitting(m)
                && m.bounds().intersects(area)
        })
    }
    /// `ChestBlock.getMenuProvider` (`combine(.., false)`) and
    /// `EnderChestBlock.useWithoutItem`: a blocked chest, or a double chest
    /// with either half blocked, opens no menu.
    pub(super) fn chest_menu_blocked(&self, key: (i32, i32, i32), name: &str) -> bool {
        match name {
            n if is_chest_block(n) => {
                self.chest_blocked_at(key)
                    || self
                        .chest_partner(key)
                        .is_some_and(|(other, _)| self.chest_blocked_at(other))
            }
            // The ender chest only looks at the block above (no cat check).
            "minecraft:ender_chest" => self.conductor_above(key),
            _ => false,
        }
    }
    /// Closing it copies the items back and leaves no block-entity contents.
    pub(super) fn close_ender_chest(&mut self) {
        let Some(key) = self.ender_open.take() else {
            return;
        };
        if let Some(BlockEntity::Chest(items)) = self.world.block_entities.remove(&key) {
            self.ender_items = items;
        }
    }
}

// ---- chest menus: double chests, titles and shift-click ----

/// A chest block state's half: its block, `type` and `facing`.
fn chest_half(state: &horizon_world::chunk::BlockState) -> Option<ChestHalf<'_>> {
    Some(ChestHalf {
        block: &state.name,
        kind: state.properties.get("type")?,
        facing: state.properties.get("facing")?,
    })
}
fn facing_step(facing: &str) -> (i32, i32) {
    match facing {
        "north" => (0, -1),
        "south" => (0, 1),
        "west" => (-1, 0),
        _ => (1, 0),
    }
}

impl Game<'_> {
    /// The other half of the open double chest and whether the open one is
    /// the right half (`ChestBlock.getConnectedDirection`).
    pub(super) fn chest_partner(&self, key: (i32, i32, i32)) -> Option<((i32, i32, i32), bool)> {
        let registry = self.pack.assets.registry;
        let state = registry.state(self.world.block(key.0, key.1, key.2))?;
        if !is_chest_block(&state.name) {
            return None;
        }
        let own = chest_half(state)?;
        let (dx, dz) = own.connected_step()?;
        let other = (key.0 + dx, key.1, key.2 + dz);
        let other_state = registry.state(self.world.block(other.0, other.1, other.2))?;
        let paired = own.pairs_with(&chest_half(other_state)?)
            && matches!(
                self.world.block_entities.get(&other),
                Some(BlockEntity::Chest(items)) if items.len() == 27
            );
        paired.then_some((other, own.kind == "right"))
    }
    /// The open or close sound of a container block (`ChestBlockEntity.playSound`,
    /// `BarrelBlockEntity.playSound`, `ShulkerBoxBlockEntity.startOpen`): volume
    /// 0.5 and pitch 0.9 to 1.0. A double chest sounds once from the middle of
    /// both halves, a barrel from the face it opens on.
    pub(super) fn container_sound(&mut self, sound: &str, key: (i32, i32, i32), name: &str) {
        let mut at = [
            f64::from(key.0) + 0.5,
            f64::from(key.1) + 0.5,
            f64::from(key.2) + 0.5,
        ];
        if let Some((other, _)) = self.chest_partner(key) {
            at[0] = f64::from(key.0 + other.0) / 2.0 + 0.5;
            at[2] = f64::from(key.2 + other.2) / 2.0 + 0.5;
        } else if name == "minecraft:barrel"
            && let Some(state) = self
                .pack
                .assets
                .registry
                .state(self.world.block(key.0, key.1, key.2))
            && let Some(facing) = state.properties.get("facing")
        {
            let (dx, dy, dz) = match facing.as_str() {
                "up" => (0.0, 0.5, 0.0),
                "down" => (0.0, -0.5, 0.0),
                f => {
                    let (x, z) = facing_step(f);
                    (f64::from(x) / 2.0, 0.0, f64::from(z) / 2.0)
                }
            };
            at = [at[0] + dx, at[1] + dy, at[2] + dz];
        }
        let pitch = self.world.random.next_float() * 0.1 + 0.9;
        self.audio.play(sound, Some(at), 0.5, pitch);
    }
    /// The open or close sound of the container block at `key`, if it has one.
    /// Callers play it only when the opener count crosses zero
    /// (`ContainerOpenersCounter.onOpen` / `onClose`).
    pub(super) fn container_edge_sound(&mut self, key: (i32, i32, i32), open: bool) {
        let name = self
            .res
            .facts()
            .get(self.world.block(key.0, key.1, key.2))
            .name
            .clone();
        let sound = if is_shulker_box(&name) {
            if open {
                "minecraft:block.shulker_box.open"
            } else {
                "minecraft:block.shulker_box.close"
            }
        } else if &*name == "minecraft:ender_chest" {
            if open {
                "minecraft:block.ender_chest.open"
            } else {
                "minecraft:block.ender_chest.close"
            }
        } else if name.contains("chest") {
            if open {
                "minecraft:block.chest.open"
            } else {
                "minecraft:block.chest.close"
            }
        } else if &*name == "minecraft:barrel" {
            if open {
                "minecraft:block.barrel.open"
            } else {
                "minecraft:block.barrel.close"
            }
        } else {
            return;
        };
        self.container_sound(sound, key, &name);
    }
    /// The other half of a double chest at `key`, for neighbour updates.
    pub(super) fn chest_other_half(&self, key: (i32, i32, i32)) -> Option<(i32, i32, i32)> {
        self.chest_partner(key).map(|p| p.0)
    }
    /// The block entities of the open container in menu order, with their
    /// sizes: a double chest is its right half then its left half.
    pub(super) fn container_parts(&self) -> Vec<((i32, i32, i32), usize)> {
        let Some(key) = self.container else {
            return Vec::new();
        };
        let Some(entity) = self.world.block_entities.get(&key) else {
            return Vec::new();
        };
        let own = (key, entity.slots().len());
        match self.chest_partner(key) {
            Some((other, true)) => vec![own, (other, 27)],
            Some((other, false)) => vec![(other, 27), own],
            None => vec![own],
        }
    }
    /// `Container.stillValidBlockEntity`: the open block container closes once
    /// the player's eyes are out of reach of any of its blocks (a 4-block buffer).
    pub(super) fn tick_container_reach(&mut self) {
        // A server's menu ends when the server closes it.
        if self.remote.is_some() {
            return;
        }
        let Some(eye) = self.eye_position() else {
            return;
        };
        if self
            .container
            .is_none_or(|k| k.1 == super::minecarts::CART_KEY_Y)
        {
            return;
        }
        let range = self.reach() + 4.0;
        let near = self.container_parts().iter().all(|&((x, y, z), _)| {
            horizon_client::lan::guest::within_block_range(eye, range, [x, y, z])
        });
        if !near {
            self.close_container();
            self.screen = super::Screen::Playing;
            self.capture(true);
        }
    }
    /// `CraftingMenu.stillValid`: the open crafting table closes once its
    /// block is no longer a crafting table or the player's eyes are out of
    /// reach (a 4-block buffer).
    pub(super) fn tick_craft_table(&mut self) {
        // A server's menu ends when the server closes it.
        if self.remote.is_some() || self.craft_size != 3 {
            return;
        }
        let (Some(pos), Some(eye)) = (self.craft_table, self.eye_position()) else {
            return;
        };
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let table = &*self.res.facts().get(state).name == "minecraft:crafting_table";
        let range = self.reach() + 4.0;
        if !(table && horizon_client::lan::guest::within_block_range(eye, range, pos)) {
            self.close_container();
            self.screen = super::Screen::Playing;
            self.capture(true);
        }
    }
    /// The number of slots of the open container.
    pub(super) fn container_size(&self) -> usize {
        self.container_parts().iter().map(|p| p.1).sum()
    }
    /// Container slot `n` (menu slot `300 + n`).
    pub(super) fn container_slot_mut(&mut self, mut n: usize) -> &mut Option<ItemStack> {
        let mut target = None;
        for (pos, size) in self.container_parts() {
            if n < size {
                target = Some((pos, n));
                break;
            }
            n -= size;
        }
        match target.and_then(|(pos, i)| {
            self.world
                .block_entities
                .get_mut(&pos)
                .and_then(|e| e.slots_mut().get_mut(i))
        }) {
            Some(slot) => slot,
            None => &mut self.scratch_slot,
        }
    }
    pub(super) fn container_block_is_shulker(&self) -> bool {
        self.container_block()
            .as_deref()
            .is_some_and(is_shulker_box)
    }
    fn container_block(&self) -> Option<std::sync::Arc<str>> {
        let k = self.container?;
        Some(
            self.res
                .facts()
                .get(self.world.block(k.0, k.1, k.2))
                .name
                .clone(),
        )
    }
    /// Rows of the open generic 9-wide container (`ChestMenu`), or None for
    /// the other menus (hoppers, dispensers, shulker boxes, furnaces...).
    pub(super) fn chest_rows(&self) -> Option<usize> {
        if let Some((rows, _)) = self.remote_chest_menu() {
            return Some(rows);
        }
        if !matches!(self.container_kind()?, BlockEntity::Chest(_)) {
            return None;
        }
        let generic = match self.container_block().as_deref() {
            Some("minecraft:barrel" | "minecraft:ender_chest") => true,
            Some(n) if is_chest_block(n) => true,
            _ => self.open_cart_kind() == Some(super::minecarts::CartKind::Chest),
        };
        generic.then(|| self.container_size() / 9)
    }
    /// The title of the open chest-like menu.
    fn chest_title(&self, rows: usize) -> Text {
        if let Some((_, title)) = self.remote_chest_menu() {
            return title;
        }
        let (key, fallback) = match self.container_block().as_deref() {
            Some("minecraft:barrel") => ("container.barrel", "Barrel"),
            Some("minecraft:ender_chest") => ("container.enderchest", "Ender Chest"),
            _ if self.open_cart_kind().is_some() => {
                ("entity.minecraft.chest_minecart", "Minecart with Chest")
            }
            _ if rows > 3 => ("container.chestDouble", "Large Chest"),
            _ => ("container.chest", "Chest"),
        };
        Text::literal(self.container_title(self.translate(key, fallback))).resolve_legacy()
    }
    /// `ContainerScreen` / `ShulkerBoxScreen`: the background with its
    /// container rows, the title and the inventory label.
    pub(super) fn draw_chest_screen(
        &self,
        batch: &mut horizon_client::hud::HudBatch,
        ox: f32,
        oy: f32,
    ) {
        use horizon_client::hud::rgba;
        let atlas = &self.gui.atlas;
        let grey = rgba(64, 64, 64, 255);
        if self
            .container_block()
            .as_deref()
            .is_some_and(is_shulker_box)
        {
            batch.sprite_region(
                atlas,
                "gui/container/shulker_box",
                [ox, oy, 176.0, 167.0],
                256.0,
                256.0,
            );
            // ShulkerBoxScreen draws the menu's title: the box's custom name
            // (BaseContainerBlockEntity.getDisplayName) or its default name.
            let title = self.remote_menu_title().unwrap_or_else(|| {
                Text::literal(
                    self.container_title(self.translate("container.shulkerBox", "Shulker Box")),
                )
                .resolve_legacy()
            });
            batch.text_styled(atlas, &title, ox + 8.0, oy + 6.0, grey, false);
            let inventory = self.translate("container.inventory", "Inventory");
            batch.text(atlas, &inventory, ox + 8.0, oy + 73.0, grey, false);
            return;
        }
        let rows = self.chest_rows().unwrap_or(3);
        let top = rows as f32 * 18.0 + 17.0;
        batch.sprite_region(
            atlas,
            "gui/container/generic_54",
            [ox, oy, 176.0, top],
            256.0,
            256.0,
        );
        // The player half of the texture starts at v = 126.
        if let Some(mut r) = atlas.sprites.get("gui/container/generic_54").copied() {
            let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
            r.v0 += 126.0 * dv;
            r.v1 = r.v0 + 96.0 * dv;
            r.u1 = r.u0 + 176.0 * du;
            batch.rect_uv(ox, oy + top, 176.0, 96.0, r, u32::MAX);
        }
        batch.text_styled(
            atlas,
            &self.chest_title(rows),
            ox + 8.0,
            oy + 6.0,
            grey,
            false,
        );
        let inventory = self.translate("container.inventory", "Inventory");
        batch.text(
            atlas,
            &inventory,
            ox + 8.0,
            oy + 114.0 + rows as f32 * 18.0 - 94.0,
            grey,
            false,
        );
    }
}

impl Game<'_> {
    /// `quickMoveStack` of the open chest-like, hopper, dispenser or furnace
    /// menu for a shift-click on `slot`, run by the verified menu engine
    /// (`horizon_client::menu`); false when the menu has none.
    pub(super) fn container_quick_move(&mut self, slot: usize) -> bool {
        self.menu_click(Some(slot), 0, horizon_client::menu::Input::QuickMove)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_chest_halves_connect_around_the_facing() {
        let half = |kind, facing| ChestHalf {
            block: "minecraft:chest",
            kind,
            facing,
        };
        // A left half facing north connects east, a right half west.
        assert_eq!(half("left", "north").connected_step(), Some((1, 0)));
        assert_eq!(half("right", "north").connected_step(), Some((-1, 0)));
        assert_eq!(half("left", "south").connected_step(), Some((-1, 0)));
        assert_eq!(half("right", "east").connected_step(), Some((0, -1)));
        assert_eq!(half("single", "east").connected_step(), None);
    }
    #[test]
    fn copper_chests_join_like_chests() {
        let half = |block, kind| ChestHalf {
            block,
            kind,
            facing: "north",
        };
        for block in [
            "minecraft:chest",
            "minecraft:trapped_chest",
            "minecraft:copper_chest",
            "minecraft:waxed_weathered_copper_chest",
        ] {
            assert!(is_chest_block(block));
            assert!(half(block, "left").pairs_with(&half(block, "right")));
        }
        // ChestBlock.combine needs the same block on both halves.
        let copper = half("minecraft:copper_chest", "left");
        assert!(!copper.pairs_with(&half("minecraft:exposed_copper_chest", "right")));
        assert!(!is_chest_block("minecraft:ender_chest"));
    }
    #[test]
    fn shulker_boxes_are_recognised_by_name() {
        assert!(is_shulker_box("minecraft:red_shulker_box"));
        assert!(is_shulker_box("minecraft:shulker_box"));
        assert!(!is_shulker_box("minecraft:chest"));
    }
}
