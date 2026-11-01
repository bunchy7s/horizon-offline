//! Block entities placed by world generation (structure chests, barrels and
//! template containers): turned into client containers, with loot tables
//! unpacked as `RandomizableContainer.unpackLootTable` would on first open.
use crate::{containers::BlockEntity, items::ItemStack};
use horizon_world::{
    loot::LootData,
    nbt::{Compound, JavaString, Tag},
    worldgen::generator::ProtoChunk,
};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

/// The `custom_data` key holding an exploration map's search until the game
/// runs it (vanilla runs `ExplorationMapFunction` when the chest is unpacked,
/// on first open; here loot is rolled with the chunk).
const PENDING_MAP: &str = "horizon:exploration_map";
const CUSTOM_DATA: &str = "minecraft:custom_data";

/// A generated chest's item, with its exploration map search kept in
/// `minecraft:custom_data` (which vanilla loads as is).
fn loot_item(s: &horizon_world::loot::LootStack) -> ItemStack {
    let mut item = ItemStack {
        item: Arc::from(s.item.as_str()),
        count: s.count.max(0) as u32,
        damage: s.damage.max(0) as u32,
        extra: None,
    };
    if let Some(map) = &s.exploration {
        let search = Tag::compound([
            ("destination", Tag::String(map.destination.as_str().into())),
            ("decoration", Tag::String(map.decoration.as_str().into())),
            ("zoom", Tag::Byte(map.zoom as i8)),
            ("search_radius", Tag::Int(map.search_radius)),
            (
                "skip_existing_chunks",
                Tag::Byte(i8::from(map.skip_existing_chunks)),
            ),
        ]);
        item.data_mut()
            .other
            .push((CUSTOM_DATA.into(), Tag::compound([(PENDING_MAP, search)])));
    }
    item
}

/// A rolled stack as an item with its enchantments (stored ones on a
/// book), potion and ominous bottle level.
pub fn loot_item_with_components(s: &horizon_world::loot::LootStack) -> ItemStack {
    let mut item = loot_item(s);
    for (id, level) in &s.enchantments {
        let level = (*level).max(0) as u32;
        if &*item.item == "minecraft:enchanted_book" {
            crate::item_data::set_level(&mut item.data_mut().stored_enchantments, id, level);
        } else {
            item.enchant(id, level);
        }
    }
    if let Some(potion) = &s.potion {
        item = item.with_potion(potion);
    }
    if let Some(level) = s.ominous_amplifier.filter(|l| *l > 0) {
        item.data_mut().ominous_amplifier = Some(level.clamp(0, 4) as u8);
    }
    // set_stew_effect's `suspicious_stew_effects`.
    for (effect, duration) in &s.stew_effects {
        item.data_mut()
            .stew_effects
            .push((Arc::from(effect.as_str()), *duration));
    }
    for (id, text) in &s.names {
        item.data_mut().set_loot_name(id, text);
    }
    for (id, value) in &s.components {
        item.data_mut().set_loot_component(id, value);
    }
    item
}

/// The exploration map search an item still waits for, removed from it.
pub fn take_pending_map(item: &mut ItemStack) -> Option<horizon_world::loot::ExplorationMap> {
    let data = item.extra.as_deref_mut()?;
    let i = data
        .other
        .iter()
        .position(|(k, t)| k == CUSTOM_DATA && t.get(PENDING_MAP).is_some())?;
    let (_, mut custom) = data.other.remove(i);
    let search = match &mut custom {
        Tag::Compound(c) => c.remove(&JavaString::from(PENDING_MAP))?,
        _ => return None,
    };
    if matches!(&custom, Tag::Compound(c) if !c.is_empty()) {
        data.other.insert(i, (CUSTOM_DATA.into(), custom));
    }
    let text = |k: &str| match search.get(k) {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    };
    let int = |k: &str| match search.get(k) {
        Some(Tag::Int(v)) => Some(*v),
        Some(Tag::Byte(v)) => Some(i32::from(*v)),
        _ => None,
    };
    Some(horizon_world::loot::ExplorationMap {
        destination: text("destination")?,
        decoration: text("decoration")?,
        zoom: int("zoom").unwrap_or(2).clamp(0, 4) as u8,
        search_radius: int("search_radius").unwrap_or(50),
        skip_existing_chunks: int("skip_existing_chunks").unwrap_or(1) != 0,
    })
}

/// Whether an item still waits for its exploration map search.
pub fn has_pending_map(item: &ItemStack) -> bool {
    item.data().is_some_and(|d| {
        d.other
            .iter()
            .any(|(k, t)| k == CUSTOM_DATA && t.get(PENDING_MAP).is_some())
    })
}

/// The loot data and the [`loot_key`] it was loaded for; None until
/// `loot_data` runs. `Some((_, None))` when the tables failed to load.
type LoadedLoot = (u64, Option<Arc<LootData>>);
static LOOT: Mutex<Option<LoadedLoot>> = Mutex::new(None);

/// The resource root and the packs' files the loot data was loaded from.
fn loot_key(root: &Path, overlay: &horizon_world::loot::PackOverlay) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    root.hash(&mut hasher);
    overlay.files.hash(&mut hasher);
    hasher.finish()
}

fn load_loot(root: &Path, overlay: &horizon_world::loot::PackOverlay) -> LoadedLoot {
    let data = match LootData::load(root, overlay) {
        Ok(data) => Some(Arc::new(data)),
        Err(e) => {
            log::warn!("Horizon: loot tables unavailable, structure containers stay empty: {e}");
            None
        }
    };
    (loot_key(root, overlay), data)
}

/// Loot data of the resource root with the world's packs over it, loaded
/// once per process (a world open with other packs, or `/reload`, loads it
/// again).
pub fn loot_data(root: &Path, overlay: &horizon_world::loot::PackOverlay) -> Option<Arc<LootData>> {
    let mut loot = LOOT.lock().unwrap_or_else(|e| e.into_inner());
    loot.get_or_insert_with(|| load_loot(root, overlay))
        .1
        .clone()
}

/// [`loot_data`] for a world being opened: the copy already loaded when it
/// came from the same root and packs (world generation loads it first, with
/// no packs; a second copy held 8 MB for the whole run), else a fresh one.
pub fn world_loot_data(
    root: &Path,
    overlay: &horizon_world::loot::PackOverlay,
) -> Option<Arc<LootData>> {
    reuse_or_load(
        &mut LOOT.lock().unwrap_or_else(|e| e.into_inner()),
        root,
        overlay,
    )
}

fn reuse_or_load(
    loot: &mut Option<LoadedLoot>,
    root: &Path,
    overlay: &horizon_world::loot::PackOverlay,
) -> Option<Arc<LootData>> {
    let key = loot_key(root, overlay);
    if loot.as_ref().is_none_or(|(loaded, _)| *loaded != key) {
        *loot = Some(load_loot(root, overlay));
    }
    loot.as_ref().and_then(|(_, data)| data.clone())
}

/// The loot data `loot_data` loaded, if it ran.
pub fn loaded_loot_data() -> Option<Arc<LootData>> {
    LOOT.lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|(_, data)| data.clone())
}

/// Let go of the loot data: for a preload no world used
/// (game/world_loading.rs). The next `loot_data` loads it again.
pub fn forget_loot_data() {
    *LOOT.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// `RandomizableContainer.unpackLootTable` of a container at `pos`: `table`
/// rolled into `slots`. A missing or zero seed is the level's random in
/// vanilla; Horizon derives one from the position. Without loot data the
/// container stays empty and still saves its table.
pub fn roll_container(
    slots: &mut [Option<ItemStack>],
    loot: Option<&LootData>,
    table: String,
    saved_seed: Option<i64>,
    pos: (i32, i32, i32),
    biome: &str,
) -> crate::save::RolledLoot {
    let seed = saved_seed.filter(|s| *s != 0).unwrap_or_else(|| {
        horizon_core::random::position_seed(horizon_core::position::BlockPos::new(
            pos.0, pos.1, pos.2,
        ))
    });
    if let Some(loot) = loot {
        // LootTable.fill spreads the items over the container's own size.
        let rolled = loot.fill(&table, seed, slots.len(), Some(biome));
        for (slot, stack) in slots.iter_mut().zip(rolled) {
            // With what the rolls put on them: enchantments
            // (enchant_with_levels in End city chests), potions and an
            // ominous bottle's level.
            *slot = stack.map(|s| loot_item_with_components(&s));
        }
    }
    crate::save::RolledLoot {
        pos,
        table,
        seed: saved_seed,
        items: crate::save::encode_items(slots),
    }
}

/// A saved `LootTableSeed`, if any.
pub fn loot_seed(nbt: &Tag) -> Option<i64> {
    match nbt.get("LootTableSeed") {
        Some(Tag::Long(seed)) => Some(*seed),
        _ => None,
    }
}

fn text(nbt: &Compound, key: &str) -> Option<String> {
    match nbt.get(&JavaString::from(key)) {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}

/// The client block entity for one of a chunk's generated block entities,
/// before a container's loot is unpacked: `StructureTemplate.placeInWorld`
/// hands the template's block entity NBT to `loadWithComponents`, which is
/// what a chunk load does with a saved one.
fn generated_block_entity(block: &str, nbt: &Compound) -> Option<BlockEntity> {
    let fallback = BlockEntity::for_block(block)?;
    let mut tag = nbt.clone();
    // RandomizableContainer.tryLoadLootTable: with a table the items are not read.
    if text(&tag, "LootTable").is_some() {
        tag.remove(&JavaString::from("Items"));
    }
    // Programmatic block entities (and a template that saved none) carry no id.
    if !tag.contains_key(&JavaString::from("id")) {
        tag.insert("id".into(), Tag::String(fallback.id().into()));
    }
    for axis in ["x", "y", "z"] {
        tag.insert(axis.into(), Tag::Int(0));
    }
    // A saved kind the client cannot decode keeps the block's own entity.
    Some(
        crate::save::decode_block_entity(&Tag::Compound(tag))
            .map_or(fallback, |(_, entity)| entity),
    )
}

/// Client block entities for the generated block entities of a chunk, and the
/// loot rolled into its containers.
pub fn block_entities(
    chunk: &ProtoChunk,
    loot: Option<&LootData>,
) -> (
    Vec<((i32, i32, i32), BlockEntity)>,
    Vec<crate::save::RolledLoot>,
) {
    let mut out = Vec::new();
    let mut rolled = Vec::new();
    for (pos, nbt) in &chunk.block_entities {
        let [x, y, z] = *pos;
        let block = chunk.block_name(x, y, z);
        let Some(mut entity) = generated_block_entity(block, nbt) else {
            continue;
        };
        if let (BlockEntity::Chest(slots), Some(table)) = (&mut entity, text(nbt, "LootTable")) {
            let seed = loot_seed(&Tag::Compound(nbt.clone()));
            rolled.push(roll_container(
                slots,
                loot,
                table,
                seed,
                (x, y, z),
                chunk.biome_at(x, y, z),
            ));
        }
        out.push(((x, y, z), entity));
    }
    (out, rolled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::loot::{ExplorationMap, LootStack};

    #[test]
    fn generated_end_gateway_keeps_its_exit() {
        let Tag::Compound(nbt) = Tag::compound([
            ("exit_portal", Tag::IntArray(vec![100, 50, 0])),
            ("ExactTeleport", Tag::Byte(1)),
        ]) else {
            unreachable!()
        };
        let Some(BlockEntity::EndGateway(g)) =
            generated_block_entity("minecraft:end_gateway", &nbt)
        else {
            panic!("gateway");
        };
        assert_eq!(g.exit_portal, Some([100, 50, 0]));
        assert!(g.exact_teleport);
    }

    #[test]
    fn placed_chest_loot_keeps_its_enchantments_and_potions() {
        let stack =
            |item: &str, enchantments: Vec<(String, i32)>, potion: Option<&str>| LootStack {
                item: item.into(),
                count: 1,
                damage: 0,
                enchantments,
                potion: potion.map(str::to_owned),
                map: false,
                exploration: None,
                ominous_amplifier: None,
                stew_effects: Vec::new(),
                names: Vec::new(),
                components: Vec::new(),
            };
        // An End city chest's enchant_with_levels roll.
        let boots = loot_item_with_components(&stack(
            "minecraft:diamond_boots",
            vec![
                ("minecraft:protection".into(), 3),
                ("minecraft:unbreaking".into(), 2),
            ],
            None,
        ));
        assert_eq!(boots.enchantments().len(), 2);
        // A book's go into stored_enchantments.
        let book = loot_item_with_components(&stack(
            "minecraft:enchanted_book",
            vec![("minecraft:mending".into(), 1)],
            None,
        ));
        assert!(book.enchantments().is_empty());
        assert_eq!(book.stored_enchantments().len(), 1);
        let arrow = loot_item_with_components(&stack(
            "minecraft:tipped_arrow",
            Vec::new(),
            Some("minecraft:slowness"),
        ));
        assert_eq!(arrow.potion(), Some("minecraft:slowness"));
    }

    /// Opening a world keeps the loot data world generation loaded when the
    /// packs are the same, and loads it again when they differ.
    #[test]
    fn a_world_open_reuses_loot_data_of_the_same_packs() {
        use horizon_world::loot::PackOverlay;
        let root = std::env::temp_dir().join(format!("horizon-loot-reuse-{}", std::process::id()));
        std::fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let mut slot = None;
        let first = reuse_or_load(&mut slot, &root, &PackOverlay::NONE).expect("loot data");
        let again = reuse_or_load(&mut slot, &root, &PackOverlay::NONE).unwrap();
        assert!(Arc::ptr_eq(&first, &again));
        let files = [(
            "loot_table/chests/x".to_owned(),
            vec![br#"{"pools": []}"#.to_vec()],
        )];
        let packs = PackOverlay { files: &files };
        let other = reuse_or_load(&mut slot, &root, &packs).unwrap();
        assert!(!Arc::ptr_eq(&first, &other));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_container_rolled_without_loot_data_saves_its_table() {
        let mut slots = vec![None; 27];
        let rolled = roll_container(
            &mut slots,
            None,
            "minecraft:chests/simple_dungeon".into(),
            Some(7),
            (1, 2, 3),
            "minecraft:plains",
        );
        let mut saved = horizon_world::nbt::Compound::new();
        saved.insert("Items".into(), crate::save::encode_items(&slots));
        assert!(rolled.restore(&mut saved));
        assert!(!saved.contains_key(&"Items".into()));
        assert_eq!(saved.get(&"LootTableSeed".into()), Some(&Tag::Long(7)));
    }

    #[test]
    fn exploration_maps_wait_in_custom_data_until_searched() {
        let mut stack = LootStack {
            item: "minecraft:buried_treasure_map".into(),
            count: 1,
            damage: 0,
            enchantments: Vec::new(),
            potion: None,
            map: true,
            exploration: Some(Box::new(ExplorationMap::from_json(&serde_json::json!({
                "decoration": "minecraft:red_x",
                "destination": "#minecraft:on_treasure_maps",
                "skip_existing_chunks": false,
                "zoom": 1
            })))),
            ominous_amplifier: None,
            stew_effects: Vec::new(),
            names: Vec::new(),
            components: Vec::new(),
        };
        let mut item = loot_item(&stack);
        assert!(has_pending_map(&item));
        // The rest of custom_data stays.
        if let Some(Tag::Compound(c)) = item
            .data_mut()
            .other
            .iter_mut()
            .find(|(k, _)| k == CUSTOM_DATA)
            .map(|(_, t)| t)
        {
            c.insert("note".into(), Tag::Int(7));
        }
        let map = take_pending_map(&mut item).unwrap();
        assert_eq!(
            map,
            ExplorationMap {
                destination: "minecraft:on_treasure_maps".into(),
                decoration: "minecraft:red_x".into(),
                zoom: 1,
                search_radius: 50,
                skip_existing_chunks: false,
            }
        );
        assert!(!has_pending_map(&item));
        assert!(take_pending_map(&mut item).is_none());
        let custom = &item.data().unwrap().other;
        assert_eq!(custom.len(), 1);
        assert_eq!(custom[0].1.get("note"), Some(&Tag::Int(7)));
        // Only the search: the component goes away with it.
        stack.item = "minecraft:abandoned_camp_map".into();
        let mut item = loot_item(&stack);
        take_pending_map(&mut item).unwrap();
        assert!(item.data().unwrap().other.is_empty());
        // A plain item has nothing pending.
        stack.exploration = None;
        assert!(!has_pending_map(&loot_item(&stack)));
    }

    /// The End cities' tower tops save their banners' patterns in the
    /// template; the placed banner must keep them (`EndCityPieces`).
    #[test]
    fn a_generated_banner_keeps_its_template_patterns() {
        let patterns = Tag::List(vec![
            Tag::compound([
                ("color", Tag::String("black".into())),
                ("pattern", Tag::String("minecraft:triangle_top".into())),
            ]),
            Tag::compound([
                ("color", Tag::String("black".into())),
                ("pattern", Tag::String("minecraft:triangle_bottom".into())),
            ]),
        ]);
        let nbt = Tag::compound([
            ("id", Tag::String("minecraft:banner".into())),
            ("patterns", patterns),
        ]);
        let Tag::Compound(nbt) = nbt else {
            unreachable!()
        };
        let Some(BlockEntity::Banner(banner)) =
            generated_block_entity("minecraft:magenta_banner", &nbt)
        else {
            panic!("a banner block makes a banner block entity");
        };
        assert_eq!(banner.layers.len(), 2);
        assert_eq!(&*banner.layers[0].pattern, "minecraft:triangle_top");
        assert_eq!(&*banner.layers[1].pattern, "minecraft:triangle_bottom");
        assert_eq!(&*banner.layers[0].color, "black");
        // A banner the template saved without patterns stays bare.
        let Some(BlockEntity::Banner(bare)) =
            generated_block_entity("minecraft:white_banner", &Compound::new())
        else {
            panic!("a banner block makes a banner block entity");
        };
        assert!(bare.layers.is_empty());
    }
}
