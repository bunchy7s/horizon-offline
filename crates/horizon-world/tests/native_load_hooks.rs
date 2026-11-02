use horizon_world::{
    item_defaults, loot,
    native_load_hooks::{self, NativeLoadHooks},
};
use serde_json::{Map, Value, json};

#[test]
fn load_hooks_are_noops_before_install_and_use_first_callbacks_after() {
    let mut table = json!([]);
    let mut components = Map::new();
    loot::native_loot_table_load("table", &mut table);
    item_defaults::native_modify_default_components("item", &mut components);
    assert_eq!(table, json!([]));
    assert!(components.is_empty());

    let hooks = || NativeLoadHooks {
        loot_table_load: |id, table| table.as_array_mut().unwrap().push(id.into()),
        modify_default_components: |item, components| {
            components.insert("item".into(), Value::String(item.into()));
        },
        loot_function: |_, _, _, stack| stack.count += 1,
        loot_condition: |kind, _, _, holds| *holds = kind == "mymod:held",
    };
    assert!(native_load_hooks::install(hooks()).is_ok());
    assert!(
        native_load_hooks::install(NativeLoadHooks {
            loot_table_load: |_, table| *table = json!("second"),
            modify_default_components: |_, components| components.clear(),
            loot_function: |_, _, _, stack| stack.count = 0,
            loot_condition: |_, _, _, holds| *holds = false,
        })
        .is_err()
    );
    loot::native_loot_table_load("table", &mut table);
    item_defaults::native_modify_default_components("item", &mut components);
    assert_eq!(table, json!(["table"]));
    assert_eq!(components["item"], json!("item"));
    let ctx = loot::NativeLootContext {
        entity: None,
        damage: None,
        player: None,
        tool: None,
        block: None,
        luck: 0.0,
        biome: None,
        seed: 0,
        place: None,
    };
    for (kind, expected) in [("mymod:held", true), ("mymod:other", false)] {
        let mut holds = !expected;
        loot::native_loot_condition(kind, &json!({}), &ctx, &mut holds);
        assert_eq!(holds, expected, "{kind}");
    }
}
