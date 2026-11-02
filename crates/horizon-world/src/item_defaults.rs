//! Items' default components, as everything in the game reads them: the
//! resource root's `items.json` (vanilla's default-component report, with a
//! content mod's items), then what native mods change in it
//! ([`native_modify_default_components`], NeoForge's `ModifyDefaultComponentsEvent`).
//! The item table, loot, enchanting, item models and the other readers all
//! come through [`read`], so a mod's change reaches each of them, and stacks
//! are saved and sent against the changed defaults as against vanilla's.

use serde_json::{Map, Value};
use std::{collections::BTreeMap, fs, io, path::Path};

/// The largest stack size a native mod may set: `ItemStack.count` is a `u32`,
/// but the count is a VarInt on the wire and is saved as an `int`, so a
/// larger one would turn negative (and lose its item) on the way out.
pub const MAX_STACK_SIZE: i64 = i32::MAX as i64;

/// Vanilla's `ExtraCodecs.intRange(1, 99)` on a data pack's own value.
const VANILLA_MAX_STACK_SIZE: i64 = 99;
const STACK_SIZE: &str = "minecraft:max_stack_size";

/// `value` as a stack size in `1..=max`, or None when it is not a number.
fn stack_size(value: &Value, max: i64) -> Option<i64> {
    value.as_f64().map(|n| (n as i64).clamp(1, max))
}

/// Hook point for native mods (docs/native-mods.md, listed in `native-hooks.json`
/// with those of `horizon-client/src/game/native_hooks.rs`): `components` holds
/// `item`'s default components in `items.json`'s form (`"minecraft:max_stack_size": 16`,
/// `"minecraft:food": {"nutrition": 4, "saturation": 2.4}`); insert or replace one to
/// set it, remove it to take it away (a stack size outside 1 to `i32::MAX` is
/// brought back into it, and one that is not a number leaves the data pack's
/// own). Asked for every item each time the game reads the defaults, from any
/// thread, several times as resources load and again when a world's data packs
/// load: answer the same each time. Client and server each ask their own mods,
/// as NeoForge does, so both sides need the mod.
/// 79.
#[inline(never)]
pub fn native_modify_default_components(item: &str, components: &mut Map<String, Value>) {
    crate::native_load_hooks::modify_default_components(item, components);
}

/// Every item's default components at `root`, by item id, with the mods'
/// changes applied. A data pack's own stack size is kept to vanilla's 1 to 99
/// (`ExtraCodecs.intRange`); a mod's, which no codec decodes, to 1 to
/// [`MAX_STACK_SIZE`], the count the wire and the save carry.
pub fn read(root: &Path) -> io::Result<BTreeMap<String, Value>> {
    let mut items: BTreeMap<String, Value> =
        serde_json::from_slice(&fs::read(root.join("items.json"))?)?;
    for (item, components) in &mut items {
        if let Value::Object(components) = components {
            // A data pack's own value goes through vanilla's codec.
            if let Some(size) = components
                .get(STACK_SIZE)
                .and_then(|size| stack_size(size, VANILLA_MAX_STACK_SIZE))
            {
                components.insert(STACK_SIZE.into(), size.into());
            }
            let pack = components.get(STACK_SIZE).cloned();
            native_modify_default_components(item, components);
            // The hook bypasses the codec, and a size of 0 would break stack
            // arithmetic; keep whatever it leaves a number every reader can
            // use. A mod that takes the component away keeps it away, and a
            // value that is not a number falls back to the data pack's (each
            // table would otherwise read its own default from it).
            match components.get(STACK_SIZE).cloned() {
                None => {}
                Some(size) => match stack_size(&size, MAX_STACK_SIZE) {
                    Some(size) => {
                        components.insert(STACK_SIZE.into(), size.into());
                    }
                    None => match pack {
                        Some(pack) => {
                            components.insert(STACK_SIZE.into(), pack);
                        }
                        None => {
                            components.remove(STACK_SIZE);
                        }
                    },
                },
            }
        }
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use std::fs;

    /// A data pack's stack size is kept to vanilla's 1..=99, so stack
    /// arithmetic never sees a 0, and a mod may raise it above 99 (up to the
    /// count the wire and the save carry).
    #[test]
    fn data_pack_sizes_stay_in_vanilla_range() {
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

    /// A mod's own size is kept to 1..=i32::MAX, a value that is not a number
    /// leaves the data pack's, and a removed component stays removed.
    #[test]
    fn mod_sizes_are_sanitised() {
        let _ = crate::native_load_hooks::install(crate::native_load_hooks::NativeLoadHooks {
            loot_table_load: |_, _| {},
            modify_default_components: |item, components| match item {
                "a:raised" => {
                    components.insert(super::STACK_SIZE.into(), 500.into());
                }
                "a:zero" => {
                    components.insert(super::STACK_SIZE.into(), 0.into());
                }
                "a:huge" => {
                    components.insert(super::STACK_SIZE.into(), u64::MAX.into());
                }
                "a:string" => {
                    components.insert(super::STACK_SIZE.into(), "500".into());
                }
                "a:removed" => {
                    components.remove(super::STACK_SIZE);
                }
                _ => {}
            },
            loot_function: |_, _, _, _| {},
            loot_condition: |_, _, _, _| {},
        });
        let root = std::env::temp_dir().join(format!("horizon-mod-sizes-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let items = r#"{"a:raised": {"minecraft:max_stack_size": 64},
            "a:zero": {"minecraft:max_stack_size": 16},
            "a:huge": {"minecraft:max_stack_size": 64},
            "a:string": {"minecraft:max_stack_size": 16},
            "a:removed": {"minecraft:max_stack_size": 64}}"#;
        fs::write(root.join("items.json"), items).unwrap();
        let items = super::read(&root).unwrap();
        fs::remove_dir_all(&root).unwrap();
        let size = |id: &str| items[id].get("minecraft:max_stack_size").cloned();
        assert_eq!(
            ["a:raised", "a:zero", "a:huge", "a:string", "a:removed"].map(size),
            [
                Some(500.into()),
                Some(1.into()),
                Some(super::MAX_STACK_SIZE.into()),
                Some(16.into()),
                None
            ]
        );
    }
}
