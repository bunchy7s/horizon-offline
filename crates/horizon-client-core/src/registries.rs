//! Registry contents and source state that commands validate and complete
//! arguments against (what the server's registry access holds).
use std::collections::BTreeMap;

/// Registry contents and source state used to validate and complete arguments.
#[derive(Clone, Debug, Default)]
pub struct Registries {
    /// Permission level of the command source.
    pub permission: u8,
    pub items: Vec<String>,
    pub item_tags: Vec<String>,
    /// Block id -> (property, allowed values).
    pub blocks: BTreeMap<String, Vec<(String, Vec<String>)>>,
    pub block_tags: Vec<String>,
    /// Entity types that `summon` can create.
    pub summonable: Vec<String>,
    /// Every entity type (selector `type=`).
    pub entity_types: Vec<String>,
    pub effects: Vec<String>,
    pub enchantments: Vec<String>,
    pub structures: Vec<String>,
    pub biomes: Vec<String>,
    pub dimensions: Vec<String>,
    pub clocks: Vec<String>,
    pub timelines: Vec<String>,
    pub time_markers: Vec<String>,
    pub advancements: Vec<String>,
    /// Game rule name -> integer-valued.
    pub game_rules: Vec<(String, bool)>,
    pub players: Vec<String>,
    /// A dedicated server's lists, for the completions of `/op`, `/deop`, `/pardon`, `/pardon-ip` and
    /// `/whitelist`: the operators' names, the banned names and addresses, the whitelisted names.
    pub ops: Vec<String>,
    pub banned_players: Vec<String>,
    pub banned_ips: Vec<String>,
    pub whitelisted: Vec<String>,
    /// `ClientSuggestionProvider.customCompletionSuggestions`: the extra chat
    /// completions the server sent (ClientboundCustomChatCompletionsPacket),
    /// each once.
    pub custom_completions: std::collections::BTreeSet<String>,
    /// Scoreboard objective names, team names and tracked score holders.
    pub objectives: Vec<String>,
    pub teams: Vec<String>,
    pub tracked: Vec<String>,
    /// Loaded datapack functions (`ns:path`) and function tags (`ns:path`).
    pub functions: Vec<String>,
    /// `/data ... storage` ids.
    pub storages: Vec<String>,
    /// Boss bar ids and datapack predicate ids.
    pub bossbars: Vec<String>,
    pub predicates: Vec<String>,
    pub item_modifiers: Vec<String>,
    pub schedules: Vec<String>,
    pub stopwatches: Vec<String>,
    pub recipes: Vec<String>,
    pub sounds: Vec<String>,
    pub damage_types: Vec<String>,
    pub attributes: Vec<String>,
    pub loot_tables: Vec<String>,
    pub templates: Vec<String>,
    pub random_sequences: Vec<String>,
    pub datapacks_enabled: Vec<String>,
    pub datapacks_disabled: Vec<String>,
    pub function_tags: Vec<String>,
    /// The block the source is looking at (relevant coordinates).
    pub target_block: Option<[i32; 3]>,
    /// `ClientSuggestionProvider.getSelectedEntities`: the UUID of the
    /// entity under the crosshair (`Entity.getStringUUID`), if any.
    pub selected_entities: Vec<String>,
    /// `ClientSuggestionProvider.getAbsoluteCoordinates`: the exact point of
    /// that block hit, each coordinate as `%.2f` (`Locale.ROOT`).
    pub target_point: Option<[String; 3]>,
    /// Every registry's element and tag ids (`minecraft:item` -> ...), as
    /// the server's registry access and reloadable registries hold them.
    pub by_registry: BTreeMap<String, RegistryIds>,
    /// Blocks whose default state has a block entity.
    pub block_entities: std::collections::BTreeSet<String>,
    /// Block tag -> member blocks.
    pub block_tag_members: BTreeMap<String, Vec<String>>,
    /// Data component types with a codec (not transient).
    pub persistent_components: Vec<String>,
    /// Recipes `/recipe` can give (not special).
    pub learnable_recipes: Vec<String>,
    /// Advancement id -> criterion names.
    pub advancement_criteria: BTreeMap<String, Vec<String>>,
    /// World clock -> time markers shown in commands.
    pub clock_time_markers: BTreeMap<String, Vec<String>>,
    /// Trigger objectives the source may use (`/trigger` suggestions).
    pub trigger_objectives: Vec<String>,
    /// Objectives with the `trigger` criterion -> holder -> whether its score is locked.
    pub trigger_scores: BTreeMap<String, BTreeMap<String, bool>>,
}
/// The ids and tag ids of one registry.
#[derive(Clone, Debug, Default)]
pub struct RegistryIds {
    pub ids: Vec<String>,
    pub tags: Vec<String>,
}
const EMPTY: &[String] = &[];
/// ClientboundCustomChatCompletionsPacket.Action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustomCompletionAction {
    Add,
    Remove,
    Set,
}
impl CustomCompletionAction {
    /// `Action.BY_ID`: `ByIdMap.continuous` with `OutOfBoundsStrategy.ZERO`,
    /// so an unknown id is `ADD`.
    pub fn from_id(id: i32) -> Self {
        match id {
            1 => Self::Remove,
            2 => Self::Set,
            _ => Self::Add,
        }
    }
}
/// `ClientSuggestionProvider.modifyCustomCompletions`: the server adds,
/// removes or replaces the extra chat completions.
pub fn modify_custom_completions(
    set: &mut std::collections::BTreeSet<String>,
    action: CustomCompletionAction,
    entries: &[String],
) {
    match action {
        CustomCompletionAction::Add => set.extend(entries.iter().cloned()),
        CustomCompletionAction::Remove => {
            for e in entries {
                set.remove(e);
            }
        }
        CustomCompletionAction::Set => {
            set.clear();
            set.extend(entries.iter().cloned());
        }
    }
}
impl Registries {
    /// `ClientSuggestionProvider.getCustomTabSuggestions`: what chat (not a
    /// command) completes, the online players' names with the server's custom
    /// completions, each once.
    pub fn custom_tab_suggestions(&self) -> std::collections::BTreeSet<&str> {
        self.players
            .iter()
            .chain(&self.custom_completions)
            .map(String::as_str)
            .collect()
    }
    /// Fill what the game did not build itself from vanilla's exported
    /// registries (`command-registries.json`, written by
    /// `VanillaCommandParseOracle registries`): built-in registries such as
    /// mob effects, data component types and dialogs, block entities and tag
    /// members. Registries that data packs reload stay with the game.
    pub fn fill_from_export(&mut self, r: &serde_json::Value) {
        fn strings(v: &serde_json::Value) -> Vec<String> {
            v.as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        }
        let fill = |field: &mut Vec<String>, key: &str| {
            if field.is_empty() {
                *field = strings(&r[key]);
            }
        };
        fill(&mut self.effects, "effects");
        fill(&mut self.enchantments, "enchantments");
        fill(&mut self.attributes, "attributes");
        fill(&mut self.damage_types, "damage_types");
        fill(&mut self.templates, "templates");
        fill(&mut self.persistent_components, "persistent_components");
        fill(&mut self.learnable_recipes, "learnable_recipes");
        const RELOADED: [&str; 5] = [
            "minecraft:advancement",
            "minecraft:recipe",
            "minecraft:loot_table",
            "minecraft:predicate",
            "minecraft:item_modifier",
        ];
        for (name, entry) in r["by_registry"].as_object().into_iter().flatten() {
            if RELOADED.contains(&name.as_str()) || self.by_registry.contains_key(name) {
                continue;
            }
            self.by_registry.insert(
                name.clone(),
                RegistryIds {
                    ids: strings(&entry["ids"]),
                    tags: strings(&entry["tags"]),
                },
            );
        }
        if self.block_entities.is_empty() {
            self.block_entities = strings(&r["block_entities"]).into_iter().collect();
        }
        if self.block_tag_members.is_empty() {
            for (tag, members) in r["block_tag_members"].as_object().into_iter().flatten() {
                self.block_tag_members.insert(tag.clone(), strings(members));
            }
        }
        if self.clock_time_markers.is_empty() {
            for (clock, markers) in r["clock_time_markers"].as_object().into_iter().flatten() {
                self.clock_time_markers
                    .insert(clock.clone(), strings(markers));
            }
        }
        if self.advancement_criteria.is_empty() {
            for (id, criteria) in r["advancement_criteria"].as_object().into_iter().flatten() {
                self.advancement_criteria
                    .insert(id.clone(), strings(criteria));
            }
        }
    }
    /// Only what [`Self::fill_from_export`] takes from an export, to keep
    /// in place of the export itself (a third of its memory).
    pub fn from_export(r: &serde_json::Value) -> Self {
        let mut out = Self::default();
        out.fill_from_export(r);
        out
    }
    /// [`Self::fill_from_export`] from an export already read with
    /// [`Self::from_export`]: each field the game left empty, each registry
    /// it did not list.
    pub fn fill_from_exported(&mut self, e: &Self) {
        let fill = |field: &mut Vec<String>, from: &Vec<String>| {
            if field.is_empty() {
                field.clone_from(from);
            }
        };
        fill(&mut self.effects, &e.effects);
        fill(&mut self.enchantments, &e.enchantments);
        fill(&mut self.attributes, &e.attributes);
        fill(&mut self.damage_types, &e.damage_types);
        fill(&mut self.templates, &e.templates);
        fill(&mut self.persistent_components, &e.persistent_components);
        fill(&mut self.learnable_recipes, &e.learnable_recipes);
        for (name, ids) in &e.by_registry {
            if !self.by_registry.contains_key(name) {
                self.by_registry.insert(name.clone(), ids.clone());
            }
        }
        if self.block_entities.is_empty() {
            self.block_entities.clone_from(&e.block_entities);
        }
        if self.block_tag_members.is_empty() {
            self.block_tag_members.clone_from(&e.block_tag_members);
        }
        if self.clock_time_markers.is_empty() {
            self.clock_time_markers.clone_from(&e.clock_time_markers);
        }
        if self.advancement_criteria.is_empty() {
            self.advancement_criteria
                .clone_from(&e.advancement_criteria);
        }
    }
    /// The element ids of a registry (sorted).
    pub fn ids(&self, registry: &str) -> &[String] {
        if let Some(r) = self.by_registry.get(registry) {
            return &r.ids;
        }
        match registry {
            "minecraft:item" => &self.items,
            "minecraft:entity_type" => &self.entity_types,
            "minecraft:mob_effect" => &self.effects,
            "minecraft:enchantment" => &self.enchantments,
            "minecraft:worldgen/structure" => &self.structures,
            "minecraft:worldgen/biome" => &self.biomes,
            "minecraft:world_clock" => &self.clocks,
            "minecraft:timeline" => &self.timelines,
            "minecraft:attribute" => &self.attributes,
            "minecraft:damage_type" => &self.damage_types,
            "minecraft:sound_event" => &self.sounds,
            "minecraft:predicate" => &self.predicates,
            "minecraft:item_modifier" => &self.item_modifiers,
            "minecraft:loot_table" => &self.loot_tables,
            "minecraft:recipe" => &self.recipes,
            "minecraft:advancement" => &self.advancements,
            _ => EMPTY,
        }
    }
    /// The tag ids of a registry (sorted).
    pub fn tags(&self, registry: &str) -> &[String] {
        if let Some(r) = self.by_registry.get(registry) {
            return &r.tags;
        }
        match registry {
            "minecraft:item" => &self.item_tags,
            "minecraft:block" => &self.block_tags,
            _ => EMPTY,
        }
    }
}
