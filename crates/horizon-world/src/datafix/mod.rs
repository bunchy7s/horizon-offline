//! Vanilla's DataFixers (`net.minecraft.util.datafix`, DataVersion 99 on):
//! the data of worlds saved by 1.8 to 26.2 brought to the 26.3 format exactly
//! as 26.3 does when it reads them, and the file fixes it runs when it opens
//! such a world (`net.minecraft.util.filefix`).
//!
//! DataFixerUpper reads the data once with the schema of its version (running
//! the schema's read hooks), applies every fix with a key above that version
//! in order, and writes it with the current schema. Each fix finds its values
//! with the schema just below its own key ([`schema`]). Every
//! fix of the server data is ported ([`OLDEST_SUPPORTED`]). The fixes of client
//! files (options, hotbars, debug profiles) are not ported either.
mod block_state_data;
mod chunk_flatten;
mod chunk_leaves;
pub mod dynamic;
pub mod files;
mod fixes_a;
mod fixes_b;
mod fixes_c;
mod fixes_cliffs;
mod fixes_flat;
mod fixes_legacy;
mod fixes_mid;
mod fixes_old;
mod flat_tables;
mod items;
pub mod json;
pub mod progress;
pub mod schema;
mod text;
pub mod world_upgrader;

use crate::nbt::Tag;
use dynamic::{as_str, string};
use schema::{Key, Ty, key, walk};

/// The DataVersion of the game.
pub const CURRENT: i32 = crate::DATA_VERSION;
/// Data saved before this DataVersion needs fixes that are not ported. Every
/// server fix is, down to the saves of 1.8, which have no DataVersion (read as -1).
pub const OLDEST_SUPPORTED: i32 = -1;

/// `Identifier.tryParse` validity, as `NamespacedSchema.ensureNamespaced` uses it.
pub fn identifier(id: &str) -> Option<(String, String)> {
    let (ns, path) = match id.split_once(':') {
        Some((ns, path)) => (if ns.is_empty() { "minecraft" } else { ns }, path),
        None => ("minecraft", id),
    };
    let ns_ok = ns
        .bytes()
        .all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.'));
    let path_ok = path
        .bytes()
        .all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/'));
    (ns_ok && path_ok).then(|| (ns.to_owned(), path.to_owned()))
}

/// What a fix works on: the root value, its type and the fix's key.
pub struct Cx<'a> {
    root: Ty,
    data: &'a mut Tag,
    k: Key,
}

impl Cx<'_> {
    /// Every value of `ty`, read with the fix's input schema.
    pub fn each(&mut self, ty: Ty, mut f: impl FnMut(&mut Tag)) {
        walk(self.root, self.data, self.k - 1, ty, &mut f);
    }
    /// Every value of `ty` read with the fix's output schema (fixes declared
    /// on `getOutputSchema().getType(...)`).
    pub fn each_out(&mut self, ty: Ty, mut f: impl FnMut(&mut Tag)) {
        walk(self.root, self.data, self.k, ty, &mut f);
    }
    /// Every entity of one type (`NamedEntityFix` and friends).
    pub fn entity(&mut self, id: &str, mut f: impl FnMut(&mut Tag)) {
        self.each(Ty::Entity, |e| {
            if id_is(e, id) {
                f(e);
            }
        });
    }
    /// Every block entity of one type.
    pub fn block_entity(&mut self, id: &str, mut f: impl FnMut(&mut Tag)) {
        self.each(Ty::BlockEntity, |e| {
            if id_is(e, id) {
                f(e);
            }
        });
    }
    /// Every item stack whose id passes `filter`.
    pub fn item(&mut self, filter: impl Fn(&str) -> bool, mut f: impl FnMut(&mut Tag)) {
        self.each(Ty::ItemStack, |s| {
            if let Some(id) = as_str(s.get("id"))
                && filter(&dynamic::namespaced(&id))
            {
                f(s);
            }
        });
    }
    /// A name type renamed in place (`fixTypeEverywhere` over a namespaced string).
    pub fn rename(&mut self, ty: Ty, rename: impl Fn(&str) -> Option<&'static str>) {
        self.each(ty, |v| {
            if let Some(name) = as_str(Some(v))
                && let Some(new) = rename(&name)
            {
                *v = string(new);
            }
        });
    }
}

/// DataFixerUpper reads a record's typed fields from its map without taking
/// them out: the record's remainder keeps the whole map it was read from, and
/// writing lays the typed fields over it. A typed field that a fix drops (by
/// casting to a type without it) therefore comes back on write with its value
/// from that read. Each entity keeps that map under this key while fixes run.
pub(crate) const LAST_READ: &str = "\u{0}last_read";

/// Remember a record's current fields as the map it was read from.
pub fn mark_read(e: &mut Tag) {
    if let Tag::Compound(_) = e {
        let mut raw = e.clone();
        strip_last_read(&mut raw);
        dynamic::set(e, LAST_READ, raw);
    }
}

/// `Util.writeAndReadTypedOrThrow` over an entity by the fix with key `k`: it is
/// written and read again with the output schema, so it (and every entity inside
/// it) now remembers its current fields as the map it was last read from.
pub fn reread(k: Key, e: &mut Tag) {
    reread_as(Ty::Entity, k, e);
}

/// [`reread`] for a value of another type (a block entity).
pub fn reread_as(ty: Ty, k: Key, v: &mut Tag) {
    // What was untyped is read again with the new schema.
    schema::flat::strip_untyped(v);
    schema::flat::mark_untyped(ty, v, k);
    walk(ty, v, k, Ty::BlockEntity, &mut mark_read);
    walk(ty, v, k, Ty::Entity, &mut mark_read);
}

/// A typed field dropped from an entity: what writing the entity gives back for
/// it (its value in the map the entity was last read from, if any).
pub fn drop_typed(e: &mut Tag, field: &str) {
    match e.get(LAST_READ).and_then(|raw| raw.get(field)).cloned() {
        Some(raw) => dynamic::set(e, field, raw),
        None => {
            dynamic::remove(e, field);
        }
    }
}

fn strip_last_read(v: &mut Tag) {
    match v {
        Tag::Compound(c) => {
            c.remove(&dynamic::key(LAST_READ));
            c.remove(&dynamic::key(schema::flat::UNTYPED));
            for value in c.values_mut() {
                strip_last_read(value);
            }
        }
        Tag::List(items) => items.iter_mut().for_each(strip_last_read),
        _ => {}
    }
}

/// The entity or block entity `id` field, namespaced.
pub fn id_is(v: &Tag, id: &str) -> bool {
    as_str(v.get("id")).is_some_and(|s| dynamic::namespaced(&s) == id)
}

/// `createRenamer`: the id namespaced, looked up in a table.
pub fn renamer<'a>(
    table: &'a [(&'a str, &'static str)],
) -> impl Fn(&str) -> Option<&'static str> + 'a {
    move |id: &str| {
        let id = dynamic::namespaced(id);
        table
            .iter()
            .find(|(from, _)| *from == id)
            .map(|(_, to)| *to)
    }
}

type FixFn = fn(&mut Cx);

/// Every ported fix (from the 1.19 schemas on), in DataFixers order.
fn fixes() -> Vec<(Key, FixFn)> {
    let mut out = Vec::new();
    fixes_legacy::register(&mut out);
    fixes_flat::register(&mut out);
    fixes_mid::register(&mut out);
    fixes_cliffs::register(&mut out);
    fixes_old::register(&mut out);
    fixes_a::register(&mut out);
    fixes_b::register(&mut out);
    fixes_c::register(&mut out);
    // DataFixerBuilder sorts the fixes of equal keys by registration, the rest by key.
    out.sort_by_key(|(k, _)| *k);
    out
}

/// `DataFixer.update(type, data, from, to)` for a root value.
pub fn update(root: Ty, data: &mut Tag, from: i32, to: i32) {
    if from >= to {
        return;
    }
    let from_key = key(from.max(0) as u32, 0);
    let to_version = to as u32;
    // The data is read once with the schema of its version (its read hooks)...
    walk(root, data, from_key, Ty::BlockEntity, &mut mark_read);
    walk(root, data, from_key, Ty::Entity, &mut mark_read);
    schema::flat::mark_untyped(root, data, from_key);
    read_hooks(root, data, from_key);
    // ...then every fix above that key, up to the target version...
    let mut refreshed = from_key >= schema::flat::END;
    for (k, fix) in fixes() {
        if k > from_key && k / 10 <= to_version {
            if !refreshed && k >= schema::flat::END {
                // The data of before 1.14 is read again once its own fixes are done.
                refreshed = true;
                walk(
                    root,
                    data,
                    schema::flat::END - 1,
                    Ty::BlockEntity,
                    &mut mark_read,
                );
                walk(
                    root,
                    data,
                    schema::flat::END - 1,
                    Ty::Entity,
                    &mut mark_read,
                );
            }
            fix(&mut Cx { root, data, k });
        }
    }
    // ...and written with the target schema (its write hooks).
    write_hooks(root, data, key(to_version, 9));
    strip_last_read(data);
}

/// The read hooks of the schema the data was saved with.
fn read_hooks(root: Ty, data: &mut Tag, k: Key) {
    walk(root, data, k, Ty::ItemStack, &mut |stack| {
        if k < key(3818, 5) {
            items::add_names_at(stack, k);
        } else if let Tag::String(_) = stack {
            // V3818_5 UNPACK_PLAIN_ID: a bare item id is a stack of one.
            *stack = Tag::compound([("id", stack.clone()), ("count", Tag::Int(1))]);
        }
    });
    if k >= key(1451, 6) {
        walk(root, data, k, Ty::Objective, &mut fixes_a::unpack_objective);
    }
    // Block names are numbers or namespaced strings until V1451_4: a number is read as an
    // int and written as one.
    walk(root, data, k, Ty::BlockName, &mut |name| {
        if let Some(n) = as_str(Some(name)) {
            *name = string(&dynamic::namespaced(&n));
        } else if k < key(1451, 4)
            && let Some(n) = dynamic::as_number(name)
        {
            *name = Tag::Int(n.int_value());
        }
    });
    // Namespaced name types read their strings with `ensureNamespaced`.
    for ty in [Ty::Biome, Ty::EntityName, Ty::ItemName] {
        walk(root, data, k, ty, &mut |name| {
            if let Some(n) = as_str(Some(name)) {
                *name = string(&dynamic::namespaced(&n));
            }
        });
    }
}

fn write_hooks(root: Ty, data: &mut Tag, k: Key) {
    walk(root, data, k, Ty::Objective, &mut fixes_a::repack_objective);
}

/// `SimpleRegionStorage.upgradeChunkTag` for a region chunk (`ChunkMap`):
/// the dimension and generator are passed to the fixes as `__context`.
pub fn update_chunk(tag: &mut Tag, dimension: &str, generator: Option<&str>) {
    update_chunk_to(tag, dimension, generator, CURRENT);
}

/// [`update_chunk`] up to an older DataVersion than the current one (the
/// harness checks the fixes of a range of versions on their own).
pub fn update_chunk_to(tag: &mut Tag, dimension: &str, generator: Option<&str>, to: i32) {
    let version = dynamic::as_int(tag.get("DataVersion"), -1);
    if version >= to {
        return;
    }
    let mut context = Tag::compound([("dimension", string(dimension))]);
    if let Some(g) = generator {
        dynamic::set(&mut context, "generator", string(g));
    }
    dynamic::set(tag, "__context", context);
    update(Ty::Chunk, tag, version, to);
    dynamic::remove(tag, "__context");
    dynamic::set(tag, "DataVersion", Tag::Int(to));
}

/// `SimpleRegionStorage.upgradeChunkTag` for entity chunks (default version -1)
/// and POI chunks (default 1945).
pub fn update_region_tag(root: Ty, tag: &mut Tag) {
    update_region_tag_to(root, tag, CURRENT);
}

/// [`update_region_tag`] up to an older DataVersion.
pub fn update_region_tag_to(root: Ty, tag: &mut Tag, to: i32) {
    let default = if root == Ty::PoiChunk { 1945 } else { -1 };
    let version = dynamic::as_int(tag.get("DataVersion"), default);
    if version >= to {
        return;
    }
    update(root, tag, version, to);
    dynamic::set(tag, "DataVersion", Tag::Int(to));
}

/// `PlayerDataStorage.load`: the DataVersion field stays as loaded.
pub fn update_player(tag: &mut Tag) {
    let version = dynamic::as_int(tag.get("DataVersion"), -1);
    update(Ty::Player, tag, version, CURRENT);
}

/// [`update_player`] up to an older DataVersion.
pub fn update_player_to(tag: &mut Tag, to: i32) {
    let version = dynamic::as_int(tag.get("DataVersion"), -1);
    update(Ty::Player, tag, version, to);
    dynamic::set(tag, "DataVersion", Tag::Int(to));
}

/// `HotbarManager.load`: a creative toolbar file (`hotbar.nbt`, its DataVersion
/// 1343 when missing) brought to the current version.
pub fn update_hotbar(tag: &mut Tag) {
    let version = dynamic::as_int(tag.get("DataVersion"), 1343);
    update(Ty::Hotbar, tag, version, CURRENT);
}

/// The saved data type of a file below a `data` folder (`namespace/path.dat`).
pub fn saved_data_type(relative: &str) -> Option<Ty> {
    let name = relative.strip_suffix(".dat").unwrap_or(relative);
    if name.ends_with("/command_storage") {
        return Some(Ty::SavedCommandStorage);
    }
    if let Some(n) = name.strip_prefix("minecraft/maps/")
        && !n.is_empty()
        && n.bytes().all(|b| b.is_ascii_digit())
    {
        return Some(Ty::SavedMapData);
    }
    Some(match name {
        "minecraft/custom_boss_events" => Ty::SavedCustomBossEvents,
        "minecraft/scoreboard" => Ty::SavedScoreboard,
        "minecraft/chunk_tickets" => Ty::SavedTickets,
        "minecraft/world_border" => Ty::SavedWorldBorder,
        "minecraft/game_rules" => Ty::SavedGameRules,
        "minecraft/ender_dragon_fight" => Ty::SavedEnderDragonFight,
        "minecraft/wandering_trader" => Ty::SavedWanderingTrader,
        "minecraft/maps/last_id" => Ty::SavedMapIndex,
        "minecraft/weather" => Ty::SavedWeather,
        "minecraft/scheduled_events" => Ty::SavedScheduledEvents,
        "minecraft/world_gen_settings" => Ty::SavedWorldGenSettings,
        "minecraft/random_sequences" => Ty::SavedRandomSequences,
        "minecraft/stopwatches" => Ty::SavedStopwatches,
        "minecraft/raids" => Ty::SavedRaids,
        "minecraft/world_clocks" => Ty::SavedWorldClocks,
        _ => return None,
    })
}

/// `SavedDataStorage.readTagFromDisk`: default version 1343, field kept.
pub fn update_saved_data(ty: Ty, tag: &mut Tag) {
    let version = dynamic::as_int(tag.get("DataVersion"), 1343);
    update(ty, tag, version, CURRENT);
}

#[cfg(test)]
mod tests;
