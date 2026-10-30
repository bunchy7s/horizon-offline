//! Block entities carried by items, vanilla 26.3: Ctrl + Pick Block in creative
//! (`ServerGamePacketListenerImpl.addBlockDataToItem`) and placing such an item
//! (`BlockItem.updateCustomBlockEntityTag`, then
//! `BlockEntity.applyComponentsFromItemStack`). Both work on the block entity's
//! saved NBT (`block_entity_nbt::canonical`) and the item's component patch.
//!
//! What a type turns into item components (`removeComponentsFromTag`,
//! `collectImplicitComponents`) leaves its NBT; the rest goes to
//! `minecraft:block_entity_data`, tagged with the type's id.
use crate::block_entity_nbt::{self, canonical};
use horizon_world::nbt::{Compound, JavaString, Tag};

pub const BLOCK_ENTITY_DATA: &str = "minecraft:block_entity_data";

/// `BlockEntityTypes.OP_ONLY_CUSTOM_DATA`: the types whose data only a player
/// who can use game master blocks places (`BlockEntityType.onlyOpCanSetNbt`).
pub const OP_ONLY: [&str; 6] = [
    "minecraft:command_block",
    "minecraft:lectern",
    "minecraft:sign",
    "minecraft:hanging_sign",
    "minecraft:mob_spawner",
    "minecraft:trial_spawner",
];

/// The `minecraft:block_entity_data` of `stack`: its type id and its data.
pub fn data(stack: &crate::items::ItemStack) -> Option<(String, &Tag)> {
    let data = stack.data()?.other(BLOCK_ENTITY_DATA)?;
    Some((block_entity_nbt::id(data)?, data))
}

/// `BlockItem.shouldPrintOpWarning` for a player with game master permission:
/// the item carries data of an op-only type.
pub fn op_warning(stack: &crate::items::ItemStack) -> bool {
    data(stack).is_some_and(|(id, _)| OP_ONLY.contains(&id.as_str()))
}

/// `SpawnEggItem.shouldPrintOpWarning` for a player with game master
/// permission: a spawn egg carries data of an op-only entity type
/// (`EntityTypes.OP_ONLY_CUSTOM_DATA`).
pub fn spawn_egg_op_warning(stack: &crate::items::ItemStack) -> bool {
    stack.item.ends_with("_spawn_egg")
        && matches!(
            stack
                .data()
                .and_then(|d| d.other("minecraft:entity_data"))
                .and_then(|data| data.get("id")),
            Some(Tag::String(id)) if matches!(
                id.to_string_lossy().as_str(),
                "minecraft:falling_block"
                    | "minecraft:command_block_minecart"
                    | "minecraft:spawner_minecart"
            )
        )
}

/// `SpawnEggItem.useOn` on a spawner: the type it sets (`getType`: the egg's
/// entity data id, else the egg's own type) and, for an op
/// (`PlayerList.isOp`), that entity data for `Spawner.setEntityData`; anyone
/// else sets only the type (`setEntityId`).
pub fn spawn_egg_on_spawner(
    stack: &crate::items::ItemStack,
    op: bool,
) -> (String, Option<Compound>) {
    let data = match stack.data().and_then(|d| d.other("minecraft:entity_data")) {
        Some(Tag::Compound(c)) => Some(c),
        _ => None,
    };
    let id = match data.and_then(|c| c.get(&JavaString::from("id"))) {
        Some(Tag::String(id)) => id.to_string_lossy(),
        _ => {
            let kind = stack.item.trim_end_matches("_spawn_egg");
            format!("minecraft:{}", kind.trim_start_matches("minecraft:"))
        }
    };
    (id, data.filter(|_| op).cloned())
}

/// `Spawner.getSpawnEntityDisplayName` (`SpawnData.entity.id` of the item's
/// data, which `ItemStack.addDetailsToTooltip` reads for spawners and trial
/// spawners alike): the entity type a spawner item spawns.
pub fn spawner_entity(stack: &crate::items::ItemStack) -> Option<String> {
    let (_, data) = data(stack)?;
    match data.get("SpawnData")?.get("entity")?.get("id")? {
        Tag::String(id) => Some(id.to_string_lossy()),
        _ => None,
    }
}

/// A part of a block entity's NBT that its item holds as a component.
#[derive(Clone, Copy)]
enum Part {
    /// The NBT key and the component hold the same value.
    Same(&'static str, &'static str),
    /// `Items` as `minecraft:container`, in a container of this many slots.
    Items(usize),
    /// `LootTable` and `LootTableSeed` as `minecraft:container_loot`.
    Loot,
    /// `is_waxed` as `minecraft:waxed`.
    Waxed,
    /// A decorated pot's `item` as `minecraft:container`.
    PotItem,
    /// Left out of the item's data, with no component.
    Drop(&'static str),
}
const NAME: Part = Part::Same("CustomName", "minecraft:custom_name");
const LOCK: Part = Part::Same("lock", "minecraft:lock");
const CONTAINER: &str = "minecraft:container";

/// Each type's `removeComponentsFromTag`, `collectImplicitComponents` and
/// `applyImplicitComponents`.
fn parts(id: &str) -> &'static [Part] {
    match id {
        "minecraft:chest"
        | "minecraft:trapped_chest"
        | "minecraft:barrel"
        | "minecraft:shulker_box" => &[NAME, LOCK, Part::Items(27), Part::Loot],
        "minecraft:dispenser" | "minecraft:dropper" | "minecraft:crafter" => {
            &[NAME, LOCK, Part::Items(9), Part::Loot]
        }
        "minecraft:hopper" => &[NAME, LOCK, Part::Items(5), Part::Loot],
        "minecraft:furnace" | "minecraft:smoker" | "minecraft:blast_furnace" => {
            &[NAME, LOCK, Part::Items(3)]
        }
        "minecraft:brewing_stand" => &[NAME, LOCK, Part::Items(5)],
        "minecraft:beacon" => &[NAME, LOCK],
        "minecraft:enchanting_table" => &[NAME],
        "minecraft:command_block" => &[NAME, Part::Drop("conditionMet"), Part::Drop("powered")],
        "minecraft:campfire" => &[Part::Items(4)],
        "minecraft:chiseled_bookshelf" => &[Part::Items(6)],
        "minecraft:shelf" => &[Part::Items(3)],
        "minecraft:beehive" => &[Part::Same("bees", "minecraft:bees")],
        "minecraft:banner" => &[Part::Same("patterns", "minecraft:banner_patterns"), NAME],
        "minecraft:decorated_pot" => &[
            Part::Same("sherds", "minecraft:pot_decorations"),
            Part::PotItem,
        ],
        "minecraft:sign" | "minecraft:hanging_sign" => &[
            Part::Same("front_text", "minecraft:sign_text_front"),
            Part::Same("back_text", "minecraft:sign_text_back"),
            Part::Waxed,
        ],
        "minecraft:skull" => &[
            Part::Same("profile", "minecraft:profile"),
            Part::Same("note_block_sound", "minecraft:note_block_sound"),
            Part::Same("custom_name", "minecraft:custom_name"),
        ],
        _ => &[],
    }
}

/// The component a part reads, and what it holds for `tag` (None: unset).
fn collect(part: Part, tag: &Tag) -> Option<(&'static str, Option<Tag>)> {
    Some(match part {
        Part::Same(key, component) => (
            component,
            tag.get(key).cloned().or_else(|| unset(component)),
        ),
        Part::Items(_) => {
            let slots = match tag.get("Items") {
                Some(Tag::List(items)) => items.iter().filter_map(slot_entry).collect(),
                _ => Vec::new(),
            };
            (CONTAINER, Some(Tag::List(slots)))
        }
        Part::Loot => {
            let table = tag.get("LootTable").cloned();
            let loot = table.map(|t| {
                let mut entries = vec![("loot_table", t)];
                if let Some(seed) = tag.get("LootTableSeed") {
                    entries.push(("seed", seed.clone()));
                }
                Tag::compound(entries)
            });
            ("minecraft:container_loot", loot)
        }
        Part::Waxed => (
            "minecraft:waxed",
            matches!(tag.get("is_waxed"), Some(Tag::Byte(1)))
                .then(|| Tag::Compound(Compound::new())),
        ),
        Part::PotItem => {
            let slots = match tag.get("item") {
                Some(Tag::Compound(item)) => Some(container_slot(0, item.clone())),
                _ => None,
            };
            (CONTAINER, Some(Tag::List(slots.into_iter().collect())))
        }
        Part::Drop(_) => return None,
    })
}

/// What a type that always collects `component` gives when its NBT leaves the
/// value out (its empty value, which the NBT omits).
fn unset(component: &str) -> Option<Tag> {
    Some(match component {
        "minecraft:bees" | "minecraft:banner_patterns" => Tag::List(Vec::new()),
        "minecraft:pot_decorations" => Tag::Compound(Compound::new()),
        "minecraft:sign_text_front" | "minecraft:sign_text_back" => {
            block_entity_nbt::sign_side(None)
        }
        _ => return None,
    })
}

/// An `Items` entry as a `minecraft:container` slot.
fn slot_entry(entry: &Tag) -> Option<Tag> {
    let Tag::Compound(entry) = entry else {
        return None;
    };
    let mut item = entry.clone();
    let slot = match item.remove(&JavaString::from("Slot"))? {
        Tag::Byte(s) => i32::from(s),
        _ => return None,
    };
    Some(container_slot(slot, item))
}

/// An `ItemContainerContents` slot: `ItemStackTemplate` leaves out a count of 1.
fn container_slot(slot: i32, mut item: Compound) -> Tag {
    if item.get(&JavaString::from("count")) == Some(&Tag::Int(1)) {
        item.remove(&JavaString::from("count"));
    }
    Tag::compound([("slot", Tag::Int(slot)), ("item", Tag::Compound(item))])
}

/// The value of `component` on the clone item of a type that collects it: a
/// picked item leaves it out of its patch, and a placed item without it in its
/// patch gives it to the block entity.
fn item_default(component: &str) -> Option<Tag> {
    Some(match component {
        CONTAINER | "minecraft:bees" | "minecraft:banner_patterns" => Tag::List(Vec::new()),
        "minecraft:pot_decorations" => {
            let brick = || Tag::compound([("id", Tag::String("minecraft:brick".into()))]);
            Tag::compound([
                ("back", brick()),
                ("front", brick()),
                ("left", brick()),
                ("right", brick()),
            ])
        }
        "minecraft:sign_text_front" | "minecraft:sign_text_back" => {
            block_entity_nbt::sign_side(None)
        }
        _ => return None,
    })
}

const METADATA: [&str; 6] = ["id", "x", "y", "z", "keepPacked", "components"];

/// `saveCustomOnly`: the saved NBT without its id, position and components.
fn custom_only(saved: &Tag) -> Compound {
    match saved {
        Tag::Compound(c) => c
            .iter()
            .filter(|(k, _)| !METADATA.contains(&k.to_string_lossy().as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        _ => Compound::new(),
    }
}

/// `addBlockDataToItem`: the component patch Ctrl + Pick Block gives the clone
/// item of the block entity saved as `saved` (a full, canonical compound).
pub fn pick(saved: &Tag) -> Compound {
    let id = block_entity_nbt::id(saved).unwrap_or_default();
    let mut patch = Compound::new();
    // BlockItem.setBlockEntityData after removeComponentsFromTag.
    let mut data = custom_only(saved);
    for part in parts(&id) {
        let key = match *part {
            Part::Same(key, _) | Part::Drop(key) => key,
            Part::Items(_) => "Items",
            Part::Loot => {
                data.remove(&JavaString::from("LootTableSeed"));
                "LootTable"
            }
            Part::Waxed => "is_waxed",
            Part::PotItem => "item",
        };
        data.remove(&JavaString::from(key));
    }
    if !data.is_empty() {
        data.insert("id".into(), Tag::String(id.as_str().into()));
        patch.insert(BLOCK_ENTITY_DATA.into(), Tag::Compound(data));
    }
    // ItemStack.applyComponents(collectComponents()): the block entity's own
    // components, then the implicit ones; a value equal to the item's own is
    // no change.
    let mut collected = match saved.get("components") {
        Some(Tag::Compound(c)) => c.clone(),
        _ => Compound::new(),
    };
    for part in parts(&id) {
        if let Some((component, value)) = collect(*part, saved) {
            match value {
                Some(v) => collected.insert(component.into(), v),
                None => collected.remove(&JavaString::from(component)),
            };
        }
    }
    for (component, value) in collected {
        if item_default(&component.to_string_lossy()).as_ref() == Some(&value) {
            patch.remove(&component);
        } else {
            patch.insert(component, value);
        }
    }
    patch
}

/// The block entity saved as `saved` after an item with component patch
/// `components` is placed as it: `BlockItem.updateCustomBlockEntityTag` (the
/// item's data of the same type, op-only types only when `game_master`, the
/// player can use game master blocks), then `applyComponentsFromItemStack`.
pub fn place(saved: &Tag, components: &Compound, game_master: bool) -> Tag {
    let Some(id) = block_entity_nbt::id(saved) else {
        return saved.clone();
    };
    let Tag::Compound(mut tag) = saved.clone() else {
        return saved.clone();
    };
    // TypedEntityData.loadInto: saveCustomOnly, merge, loadCustomOnly when changed.
    if let Some(Tag::Compound(data)) = components.get(&JavaString::from(BLOCK_ENTITY_DATA))
        && data
            .get(&JavaString::from("id"))
            .is_some_and(|t| matches!(t, Tag::String(s) if s.to_string_lossy() == id))
        && (game_master || !OP_ONLY.contains(&id.as_str()))
    {
        let custom = custom_only(saved);
        let mut merged = custom.clone();
        let mut data = data.clone();
        data.remove(&JavaString::from("id"));
        crate::data_cmd::merge_compound(&mut merged, &data);
        if merged != custom {
            for key in METADATA {
                if let Some(v) = tag.get(&JavaString::from(key)) {
                    merged.insert(key.into(), v.clone());
                }
            }
            if let Some(Tag::Compound(loaded)) = canonical(&Tag::Compound(merged)) {
                tag = loaded;
            }
        }
    }
    // applyImplicitComponents, then the rest of the patch as the block
    // entity's own components.
    let mut read = vec![BLOCK_ENTITY_DATA, "minecraft:block_state"];
    // The item's components: its patch over its own defaults.
    let get = |c: &str| match components.get(&JavaString::from(c)) {
        Some(v) => Some(v.clone()),
        None if components.contains_key(&JavaString::from(format!("!{c}").as_str())) => None,
        None => item_default(c),
    };
    let mut set = |key: &str, value: Option<Tag>| match value {
        Some(v) => tag.insert(key.into(), v),
        None => tag.remove(&JavaString::from(key)),
    };
    for part in parts(&id) {
        match *part {
            Part::Same(key, component) => {
                read.push(component);
                set(key, get(component));
            }
            Part::Items(size) => {
                read.push(CONTAINER);
                let items = container_slots(get(CONTAINER).as_ref())
                    .filter(|(slot, _)| (*slot as usize) < size)
                    .map(|(slot, mut item)| {
                        item.insert("Slot".into(), Tag::Byte(slot as i8));
                        Tag::Compound(item)
                    })
                    .collect();
                set("Items", Some(Tag::List(items)));
            }
            Part::Loot => {
                read.push("minecraft:container_loot");
                if let Some(loot) = get("minecraft:container_loot") {
                    set("LootTable", loot.get("loot_table").cloned());
                    set("LootTableSeed", loot.get("seed").cloned());
                }
            }
            Part::Waxed => {
                read.push("minecraft:waxed");
                set(
                    "is_waxed",
                    Some(Tag::Byte(i8::from(get("minecraft:waxed").is_some()))),
                );
            }
            Part::PotItem => {
                read.push(CONTAINER);
                let first = container_slots(get(CONTAINER).as_ref()).find(|(slot, _)| *slot == 0);
                set("item", first.map(|(_, item)| Tag::Compound(item)));
            }
            Part::Drop(_) => {}
        }
    }
    let own: Compound = components
        .iter()
        .filter(|(k, _)| {
            let k = k.to_string_lossy();
            !k.starts_with('!') && !read.contains(&k.as_str())
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    tag.insert("components".into(), Tag::Compound(own));
    let tag = Tag::Compound(tag);
    canonical(&tag).unwrap_or(tag)
}

/// The slots of a `minecraft:container` component.
fn container_slots(container: Option<&Tag>) -> impl Iterator<Item = (i32, Compound)> + '_ {
    let list = match container {
        Some(Tag::List(list)) => list.as_slice(),
        _ => &[],
    };
    list.iter().filter_map(|entry| {
        let slot = match entry.get("slot")? {
            Tag::Int(s) => *s,
            _ => return None,
        };
        match entry.get("item")? {
            Tag::Compound(item) => Some((slot, item.clone())),
            _ => None,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snbt(text: &str) -> Tag {
        horizon_world::nbt::parse_snbt(text).expect("snbt").0
    }
    fn saved(text: &str) -> Tag {
        canonical(&snbt(text)).expect("known type")
    }
    fn canon(patch: &Compound) -> String {
        crate::item_data::canon_tag(&Tag::Compound(patch.clone()))
    }

    #[test]
    fn a_filled_named_chest_gives_its_items_and_name_as_components() {
        let chest = saved(
            "{id:\"minecraft:chest\",x:1,y:2,z:3,CustomName:\"Loot\",\
             Items:[{Slot:4b,id:\"minecraft:stone\",count:5}]}",
        );
        let patch = pick(&chest);
        assert_eq!(
            canon(&patch),
            crate::item_data::canon_tag(&snbt(
                "{\"minecraft:container\":[{slot:4,item:{id:\"minecraft:stone\",count:5}}],\
                 \"minecraft:custom_name\":\"Loot\"}"
            ))
        );
        // Placing it back gives the same chest.
        let empty = saved("{id:\"minecraft:chest\",x:1,y:2,z:3}");
        assert_eq!(place(&empty, &patch, false), chest);
        // An empty chest has nothing to carry.
        assert!(pick(&empty).is_empty());
    }

    #[test]
    fn a_command_block_keeps_its_command_in_block_entity_data() {
        let block = saved(
            "{id:\"minecraft:command_block\",x:0,y:0,z:0,Command:\"say hi\",auto:1b,powered:1b}",
        );
        let patch = pick(&block);
        let data = patch
            .get(&JavaString::from(BLOCK_ENTITY_DATA))
            .expect("data");
        assert_eq!(data.get("Command"), Some(&Tag::String("say hi".into())));
        assert_eq!(data.get("powered"), None);
        assert_eq!(
            data.get("id"),
            Some(&Tag::String("minecraft:command_block".into()))
        );
        // Only a player who can use game master blocks places it.
        let fresh = saved("{id:\"minecraft:command_block\",x:0,y:0,z:0}");
        assert_eq!(place(&fresh, &patch, false), fresh);
        assert_eq!(
            place(&fresh, &patch, true).get("Command"),
            Some(&Tag::String("say hi".into()))
        );
        // Data of another type is ignored.
        let chest = saved("{id:\"minecraft:chest\",x:0,y:0,z:0}");
        assert_eq!(place(&chest, &patch, true), chest);
    }

    #[test]
    fn a_spawn_egg_sets_its_data_type_and_an_ops_data() {
        let egg = |entity_data: Option<&str>| {
            let mut stack = crate::items::ItemStack::new("minecraft:zombie_spawn_egg", 1);
            if let Some(text) = entity_data {
                let components = snbt(&format!("{{\"minecraft:entity_data\":{text}}}"));
                (stack.damage, stack.extra) =
                    crate::item_data::decode_components(Some(&components));
            }
            stack
        };
        // A plain egg: its own type, no data, op or not.
        assert_eq!(
            spawn_egg_on_spawner(&egg(None), true),
            ("minecraft:zombie".into(), None)
        );
        // Entity data: its type for anyone (getType), its data only for an op.
        let custom = egg(Some("{id:\"minecraft:skeleton\",CustomName:\"Guard\"}"));
        let (id, data) = spawn_egg_on_spawner(&custom, false);
        assert_eq!((id.as_str(), data), ("minecraft:skeleton", None));
        let (id, data) = spawn_egg_on_spawner(&custom, true);
        assert_eq!(id, "minecraft:skeleton");
        let data = data.expect("an op's egg carries its data");
        assert_eq!(
            data.get(&JavaString::from("CustomName")),
            Some(&Tag::String("Guard".into()))
        );
    }

    #[test]
    fn item_data_names_its_spawn_and_op_only_types() {
        let item = |text: &str| {
            let components = snbt(&format!("{{\"minecraft:block_entity_data\":{text}}}"));
            let (damage, extra) = crate::item_data::decode_components(Some(&components));
            crate::items::ItemStack {
                damage,
                extra,
                ..crate::items::ItemStack::new("minecraft:spawner", 1)
            }
        };
        let spawner =
            item("{id:\"minecraft:mob_spawner\",SpawnData:{entity:{id:\"minecraft:zombie\"}}}");
        assert_eq!(
            spawner_entity(&spawner).as_deref(),
            Some("minecraft:zombie")
        );
        assert!(op_warning(&spawner));
        let chest = item("{id:\"minecraft:chest\",LootTable:\"x\"}");
        assert_eq!(spawner_entity(&chest), None);
        assert!(!op_warning(&chest));
    }

    #[test]
    fn a_written_sign_gives_its_text_and_wax() {
        let sign = saved(
            "{id:\"minecraft:sign\",x:0,y:0,z:0,is_waxed:1b,\
             front_text:{messages:[\"a\",\"b\",\"\",\"\"]}}",
        );
        let patch = pick(&sign);
        let keys: Vec<String> = patch.keys().map(|k| k.to_string_lossy()).collect();
        assert_eq!(keys, ["minecraft:sign_text_front", "minecraft:waxed"]);
        let fresh = saved("{id:\"minecraft:sign\",x:0,y:0,z:0}");
        assert_eq!(place(&fresh, &patch, false), sign);
    }
}
