//! Block entities and the items that carry them
//! (`horizon_client::block_entity_item`): the item Pick Block gives, with the
//! block entity's data on Ctrl in creative, and the data and components a
//! placed item gives its block entity.
use super::Game;
use horizon_client::{
    Result, block_entity_item, block_entity_nbt, containers::BlockEntity, item_data,
    items::ItemStack, save,
};
use horizon_world::{
    level::classes::Class,
    nbt::{Compound, JavaString, Tag},
    registry::BlockStateId,
};

/// The component patch of `stack`.
fn components(stack: &ItemStack) -> Compound {
    match item_data::encode_components(stack) {
        Some(Tag::Compound(c)) => c,
        _ => Compound::new(),
    }
}

/// `BlockState.getCloneItemStack`'s item: a stem's seed (`StemBlock` and
/// `AttachedStemBlock` override it; a stem has no block item), else the
/// block's own item.
fn clone_item<'a>(name: &str, block_item: &'a str) -> &'a str {
    match name {
        "minecraft:pumpkin_stem" | "minecraft:attached_pumpkin_stem" => "minecraft:pumpkin_seeds",
        "minecraft:melon_stem" | "minecraft:attached_melon_stem" => "minecraft:melon_seeds",
        _ => block_item,
    }
}

impl Game<'_> {
    /// The block entity at `key` as vanilla saves it, with its custom name.
    fn saved_block_entity(&self, key: (i32, i32, i32)) -> Option<Tag> {
        let entity = self.world.block_entities.get(&key)?;
        let block = &self
            .res
            .facts()
            .get(self.world.block(key.0, key.1, key.2))
            .name;
        let mut tag = save::encode_block_entity(key, entity, block);
        if let (Some(name), Tag::Compound(map)) = (self.world.block_names.get(&key), &mut tag) {
            map.insert("CustomName".into(), Tag::String(name.as_str().into()));
        }
        block_entity_nbt::canonical(&tag)
    }

    /// `BlockState.getCloneItemStack` of the block at `pos`, then, with
    /// `include_data`, `ServerGamePacketListenerImpl.addBlockDataToItem`.
    pub(super) fn picked_stack(&self, pos: [i32; 3], include_data: bool) -> Option<ItemStack> {
        let key = (pos[0], pos[1], pos[2]);
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let facts = self.res.facts().get(state);
        // PistonHeadBlock.getCloneItemStack: the piston the head belongs to.
        let head = self
            .pack
            .assets
            .registry
            .state(state)
            .filter(|s| s.name.as_str() == "minecraft:piston_head")
            .map(|s| match s.properties.get("type") {
                Some(t) if t == "sticky" => "minecraft:sticky_piston",
                _ => "minecraft:piston",
            });
        let item = clone_item(&facts.name, head.unwrap_or(&facts.item));
        if item == "minecraft:air" {
            return None;
        }
        // FlowerPotBlock.getCloneItemStack: a potted plant picks as that plant
        // (`new ItemStack(potted)`), an empty pot as the pot itself.
        let behaviours = self.res.behaviours();
        let plant = behaviours.data(state);
        let plant_item = plant
            .is(Class::FlowerPot)
            .then_some(plant.link)
            .flatten()
            .map(|block| behaviours.block_data(block).item.clone())
            .filter(|item| &**item != "minecraft:air");
        let mut stack = ItemStack::new(plant_item.as_deref().unwrap_or(item), 1);
        let mut patch = Compound::new();
        // BeehiveBlock.getCloneItemStack: the honey level as the item's block state.
        if include_data
            && let Some(level) = self
                .pack
                .assets
                .registry
                .state(state)
                .filter(|s| matches!(s.name.as_str(), "minecraft:beehive" | "minecraft:bee_nest"))
                .and_then(|s| s.properties.get("honey_level"))
        {
            patch.insert(
                "minecraft:block_state".into(),
                Tag::compound([("honey_level", Tag::String(level.as_str().into()))]),
            );
        }
        // CopperGolemStatueBlock.getCloneItemStack: the statue's pose as the item's
        // block_state (`CopperGolemStatueBlockEntity.getItem`), data or not.
        if let Some(pose) = self
            .pack
            .assets
            .registry
            .state(state)
            .filter(|s| s.name.ends_with("copper_golem_statue"))
            .and_then(|s| s.properties.get("copper_golem_pose"))
        {
            patch.insert(
                "minecraft:block_state".into(),
                Tag::compound([("copper_golem_pose", Tag::String(pose.as_str().into()))]),
            );
        }
        // LightBlock.getCloneItemStack, setLightOnStack: the light's level as
        // the item's block state, whether or not the block data is picked.
        if let Some(level) = self
            .pack
            .assets
            .registry
            .state(state)
            .filter(|s| s.name.as_str() == "minecraft:light")
            .and_then(|s| s.properties.get("level"))
        {
            patch.insert(
                "minecraft:block_state".into(),
                Tag::compound([("level", Tag::String(level.as_str().into()))]),
            );
        }
        // AbstractBannerBlock.getCloneItemStack: the banner's own item
        // (BannerBlockEntity.getItem), data or not.
        let banner = matches!(
            self.world.block_entities.get(&key),
            Some(BlockEntity::Banner(_))
        );
        // DecoratedPotBlock.getCloneItemStack: createDecoratedPotInstance, the
        // sherds only; the pot's item comes with include_data.
        let pot = &*facts.name == "minecraft:decorated_pot";
        if (include_data || banner || pot)
            && let Some(saved) = self.saved_block_entity(key)
        {
            let mut picked = block_entity_item::pick(&saved);
            if pot && !include_data {
                picked.retain(|k, _| *k == JavaString::from("minecraft:pot_decorations"));
            }
            patch.extend(picked);
        }
        if !patch.is_empty() {
            let mut all = components(&stack);
            all.extend(patch);
            (stack.damage, stack.extra) = item_data::decode_components(Some(&Tag::Compound(all)));
        }
        Some(stack)
    }

    /// `BlockItem.updateBlockStateFromTag`: the properties of the item's
    /// `block_state` component (a hive's honey level, a statue's pose) set on
    /// the block just placed at `pos`. Returns the state it has now.
    pub(super) fn apply_item_block_state(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        stack: &ItemStack,
    ) -> Result<BlockStateId> {
        let Some(Tag::Compound(saved)) =
            stack.data().and_then(|d| d.other("minecraft:block_state"))
        else {
            return Ok(state);
        };
        let Some(current) = self.pack.assets.registry.state(state) else {
            return Ok(state);
        };
        // Only the properties the block has.
        let props: Vec<(String, String)> = saved
            .iter()
            .filter_map(|(k, v)| match v {
                Tag::String(v) => Some((k.to_string_lossy(), v.to_string_lossy())),
                _ => None,
            })
            .filter(|(k, v)| current.properties.get(k).is_some_and(|old| old != v))
            .collect();
        let props: Vec<(&str, &str)> = props.iter().map(|(k, v)| (&**k, &**v)).collect();
        match self.with_props(state, &props) {
            Some(new) if !props.is_empty() && new != state => {
                self.level_set(pos, new)?;
                Ok(new)
            }
            _ => Ok(state),
        }
    }

    /// `BlockItem.updateCustomBlockEntityTag` and `updateBlockEntityComponents`
    /// for the block entity just placed at `pos` from `stack`; `game_master`:
    /// the placer can use game master blocks (a dispenser can't).
    pub(super) fn apply_placed_item(
        &mut self,
        pos: [i32; 3],
        stack: &ItemStack,
        game_master: bool,
    ) {
        let key = (pos[0], pos[1], pos[2]);
        self.pending.banner.remove(&pos);
        let Some(saved) = self.saved_block_entity(key) else {
            return;
        };
        let placed = block_entity_item::place(&saved, &components(stack), game_master);
        if placed == saved {
            return;
        }
        let Some((_, mut entity)) = save::decode_block_entity(&placed) else {
            return;
        };
        // The new crafter's state flag (CrafterBlock.newBlockEntity), which
        // its saved data has no part of.
        if let (BlockEntity::Crafter(new), Some(BlockEntity::Crafter(old))) =
            (&mut entity, self.world.block_entities.get(&key))
        {
            new.state_triggered = old.state_triggered;
        }
        // Banners and signs keep their name themselves.
        if !matches!(entity, BlockEntity::Banner(_) | BlockEntity::Sign(_)) {
            match save::text(placed.get("CustomName")) {
                Some(name) => self.world.block_names.insert(key, name),
                None => self.world.block_names.remove(&key),
            };
        }
        self.world.block_entities.insert(key, entity);
        self.update_spawner_display(key);
    }
}

#[cfg(test)]
mod tests {
    use super::clone_item;

    /// `StemBlock.getCloneItemStack` and `AttachedStemBlock.getCloneItemStack`:
    /// pumpkin and melon stems, attached or not, pick the seed they grew from;
    /// a block without an override still picks its own item.
    #[test]
    fn stems_pick_their_seeds_and_blocks_their_item() {
        for (stem, seed) in [
            ("minecraft:pumpkin_stem", "minecraft:pumpkin_seeds"),
            ("minecraft:attached_pumpkin_stem", "minecraft:pumpkin_seeds"),
            ("minecraft:melon_stem", "minecraft:melon_seeds"),
            ("minecraft:attached_melon_stem", "minecraft:melon_seeds"),
        ] {
            assert_eq!(clone_item(stem, "minecraft:air"), seed, "{stem}");
        }
        assert_eq!(
            clone_item("minecraft:stone", "minecraft:stone"),
            "minecraft:stone"
        );
    }
}
