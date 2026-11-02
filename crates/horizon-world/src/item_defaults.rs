//! Items' default components, as everything in the game reads them: the
//! resource root's `items.json` (vanilla's default-component report, with a
//! content mod's items), then what native mods change in it
//! ([`native_modify_default_components`], NeoForge's `ModifyDefaultComponentsEvent`).
//! The item table, loot, enchanting, item models and the other readers all
//! come through [`read`], so a mod's change reaches each of them, and stacks
//! are saved and sent against the changed defaults as against vanilla's.

use serde_json::{Map, Value};
use std::{collections::BTreeMap, fs, io, path::Path};

/// Hook point for native mods (docs/native-mods.md, listed in `native-hooks.json`
/// with those of `horizon-client/src/game/native_hooks.rs`): `components` holds
/// `item`'s default components in `items.json`'s form (`"minecraft:max_stack_size": 16`,
/// `"minecraft:food": {"nutrition": 4, "saturation": 2.4}`); insert or replace one to
/// set it, remove it to take it away (a stack size outside 1 to 99 is brought back
/// into it). Asked for every item each time the game reads
/// the defaults, from any thread, several times as resources load and again when a
/// world's data packs load: answer the same each time. Client and server each ask
/// their own mods, as NeoForge does, so both sides need the mod.
/// 79.
#[inline(never)]
pub fn native_modify_default_components(item: &str, components: &mut Map<String, Value>) {
    crate::native_load_hooks::modify_default_components(item, components);
}

/// Every item's default components at `root`, by item id, with the mods'
/// changes applied and the stack size kept to vanilla's 1 to 99
/// (`ExtraCodecs.intRange`).
pub fn read(root: &Path) -> io::Result<BTreeMap<String, Value>> {
    let mut items: BTreeMap<String, Value> =
        serde_json::from_slice(&fs::read(root.join("items.json"))?)?;
    for (item, components) in &mut items {
        if let Value::Object(components) = components {
            native_modify_default_components(item, components);
            // ExtraCodecs.intRange(1, 99): nothing decodes a mod's change, and a
            // size of 0 would break stack arithmetic.
            if let Some(size) = components
                .get_mut("minecraft:max_stack_size")
                .filter(|size| size.is_number())
            {
                *size = size.as_f64().map_or(64, |n| (n as i64).clamp(1, 99)).into();
            }
        }
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use std::fs;

    /// A stack size outside vanilla's codec range is kept to 1..=99 for every
    /// reader, so stack arithmetic never sees a 0.
    #[test]
    fn stack_sizes_stay_in_vanilla_range() {
        let root = std::env::temp_dir().join(format!("horizon-stack-sizes-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let items = r#"{"a:none": {"minecraft:max_stack_size": 0},
            "a:less": {"minecraft:max_stack_size": -5},
            "a:many": {"minecraft:max_stack_size": 500},
            "a:some": {"minecraft:max_stack_size": 16}, "a:unset": {}}"#;
        fs::write(root.join("items.json"), items).unwrap();
        let items = super::read(&root).unwrap();
        fs::remove_dir_all(&root).unwrap();
        let size = |id: &str| items[id].get("minecraft:max_stack_size").cloned();
        assert_eq!(
            ["a:none", "a:less", "a:many", "a:some", "a:unset"].map(size),
            [
                Some(1.into()),
                Some(1.into()),
                Some(99.into()),
                Some(16.into()),
                None
            ]
        );
    }
}
