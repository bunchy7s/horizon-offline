//! Callbacks into the executable for data-loading native trait mods.
use crate::loot::{LootStack, NativeLootContext};
use serde_json::{Map, Value};
use std::sync::OnceLock;

/// Data-loading hooks, installed before any data loads. Earlier loads see no mods.
pub struct NativeLoadHooks {
    pub loot_table_load: fn(&str, &mut Value),
    pub modify_default_components: fn(&str, &mut Map<String, Value>),
    pub loot_function: fn(&str, &Value, &NativeLootContext<'_>, &mut LootStack),
    pub loot_condition: fn(&str, &Value, &NativeLootContext<'_>, &mut bool),
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

pub(crate) fn loot_function(
    kind: &str,
    data: &Value,
    ctx: &NativeLootContext<'_>,
    stack: &mut LootStack,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.loot_function)(kind, data, ctx, stack);
    }
}

pub(crate) fn loot_condition(
    kind: &str,
    data: &Value,
    ctx: &NativeLootContext<'_>,
    holds: &mut bool,
) {
    if let Some(hooks) = HOOKS.get() {
        (hooks.loot_condition)(kind, data, ctx, holds);
    }
}
