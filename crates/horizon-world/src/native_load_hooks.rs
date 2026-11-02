//! Callbacks into the executable for data-loading native trait mods.
use serde_json::{Map, Value};
use std::sync::OnceLock;

/// Data-loading hooks, installed before any data loads. Earlier loads see no mods.
pub struct NativeLoadHooks {
    pub loot_table_load: fn(&str, &mut Value),
    pub modify_default_components: fn(&str, &mut Map<String, Value>),
}

static HOOKS: OnceLock<NativeLoadHooks> = OnceLock::new();

/// Install the process-wide callbacks once; a second installation keeps the first.
pub fn install(hooks: NativeLoadHooks) -> Result<(), NativeLoadHooks> {
    HOOKS.set(hooks)
}

pub(crate) fn loot_table_load(id: &str, table: &mut Value) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.loot_table_load)(id, table);
    }
}

pub(crate) fn modify_default_components(item: &str, components: &mut Map<String, Value>) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.modify_default_components)(item, components);
    }
}
