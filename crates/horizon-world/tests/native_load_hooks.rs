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
    };
    assert!(native_load_hooks::install(hooks()).is_ok());
    assert!(
        native_load_hooks::install(NativeLoadHooks {
            loot_table_load: |_, table| *table = json!("second"),
            modify_default_components: |_, components| components.clear(),
        })
        .is_err()
    );
    loot::native_loot_table_load("table", &mut table);
    item_defaults::native_modify_default_components("item", &mut components);
    assert_eq!(table, json!(["table"]));
    assert_eq!(components["item"], json!("item"));
}
