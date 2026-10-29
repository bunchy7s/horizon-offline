//! The registries VanillaCommandParseOracle exports (`registries` mode), as
//! the command dispatcher reads them; shared by the command fixtures.
use horizon_client::command::{Registries, args::RegistryIds};
use serde_json::Value;

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

pub fn registries(r: &Value, permission: u8) -> Registries {
    let mut reg = Registries {
        permission,
        ..Registries::default()
    };
    reg.items = strings(&r["items"]);
    reg.item_tags = strings(&r["item_tags"]);
    reg.block_tags = strings(&r["block_tags"]);
    for (id, props) in r["blocks"].as_object().into_iter().flatten() {
        let props = props
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| (p[0].as_str().unwrap_or("").to_owned(), strings(&p[1])))
            .collect();
        reg.blocks.insert(id.clone(), props);
    }
    reg.summonable = strings(&r["summonable"]);
    reg.entity_types = strings(&r["entity_types"]);
    reg.effects = strings(&r["effects"]);
    reg.attributes = strings(&r["attributes"]);
    reg.sounds = strings(&r["sounds"]);
    reg.enchantments = strings(&r["enchantments"]);
    reg.structures = strings(&r["structures"]);
    reg.biomes = strings(&r["biomes"]);
    reg.damage_types = strings(&r["damage_types"]);
    reg.clocks = strings(&r["clocks"]);
    reg.timelines = strings(&r["timelines"]);
    reg.dimensions = strings(&r["dimensions"]);
    reg.players = strings(&r["players"]);
    reg.selected_entities = strings(&r["selected_entities"]);
    reg.teams = strings(&r["teams"]);
    reg.objectives = strings(&r["objectives"]);
    reg.tracked = strings(&r["tracked"]);
    for (name, entry) in r["by_registry"].as_object().into_iter().flatten() {
        reg.by_registry.insert(
            name.clone(),
            RegistryIds {
                ids: strings(&entry["ids"]),
                tags: strings(&entry["tags"]),
            },
        );
    }
    reg.block_entities = strings(&r["block_entities"]).into_iter().collect();
    for (tag, members) in r["block_tag_members"].as_object().into_iter().flatten() {
        reg.block_tag_members.insert(tag.clone(), strings(members));
    }
    reg.templates = strings(&r["templates"]);
    reg.persistent_components = strings(&r["persistent_components"]);
    reg.learnable_recipes = strings(&r["learnable_recipes"]);
    for (clock, markers) in r["clock_time_markers"].as_object().into_iter().flatten() {
        reg.clock_time_markers
            .insert(clock.clone(), strings(markers));
    }
    for (id, criteria) in r["advancement_criteria"].as_object().into_iter().flatten() {
        reg.advancement_criteria
            .insert(id.clone(), strings(criteria));
    }
    reg.datapacks_enabled = strings(&r["datapacks_selected"]);
    let enabled = reg.datapacks_enabled.clone();
    reg.datapacks_disabled = strings(&r["datapacks_enableable"])
        .into_iter()
        .filter(|p| !enabled.contains(p))
        .collect();
    reg.advancements = reg.ids("minecraft:advancement").to_vec();
    reg.recipes = reg.ids("minecraft:recipe").to_vec();
    reg.loot_tables = reg.ids("minecraft:loot_table").to_vec();
    reg.predicates = reg.ids("minecraft:predicate").to_vec();
    reg.item_modifiers = reg.ids("minecraft:item_modifier").to_vec();
    reg
}
