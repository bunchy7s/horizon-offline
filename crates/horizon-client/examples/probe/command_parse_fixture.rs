//! Command parsing for differential tests against vanilla's Brigadier tree
//! (tools/test_command_parse_parity.py): reads one command per line and prints
//! the parse result, the error (rendered text and UTF-16 cursor), the parsed
//! node names and the completion suggestions at the end of the input, in the
//! same JSON-lines shape as VanillaCommandParseOracle.
//!
//! usage: command_parse_fixture RESOURCES REGISTRIES.json CORPUS PERMISSION OUTPUT
use horizon_client::{
    command::{ArgType, Dispatcher, NodeKind, Registries, args::RegistryIds, vanilla},
    lang,
};
use serde_json::{Value, json};
use std::{fs, path::Path};

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn utf16(input: &str, byte: usize) -> usize {
    let mut end = byte.min(input.len());
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    input[..end].encode_utf16().count()
}

fn parser_id(ty: &ArgType) -> String {
    match ty {
        ArgType::Bool => "brigadier:bool".into(),
        ArgType::Integer { .. } => "brigadier:integer".into(),
        ArgType::Float { .. } => "brigadier:float".into(),
        ArgType::Double { .. } => "brigadier:double".into(),
        ArgType::Word | ArgType::Str | ArgType::Greedy => "brigadier:string".into(),
        ArgType::Entity { .. } => "minecraft:entity".into(),
        ArgType::BlockPos => "minecraft:block_pos".into(),
        ArgType::Vec3 { .. } => "minecraft:vec3".into(),
        ArgType::Rotation => "minecraft:rotation".into(),
        ArgType::Angle => "minecraft:angle".into(),
        ArgType::Item => "minecraft:item_stack".into(),
        ArgType::ItemPredicate => "minecraft:item_predicate".into(),
        ArgType::BlockState => "minecraft:block_state".into(),
        ArgType::BlockPredicate => "minecraft:block_predicate".into(),
        ArgType::Resource(_) => "minecraft:resource".into(),
        ArgType::Id(_) => "minecraft:resource_location".into(),
        ArgType::Res(_) => "minecraft:resource".into(),
        ArgType::ResKey(_) => "minecraft:resource_key".into(),
        ArgType::ResOrTag(_) => "minecraft:resource_or_tag".into(),
        ArgType::ResOrTagKey(_) => "minecraft:resource_or_tag_key".into(),
        ArgType::StrEnum(values, _) => match values.first().copied() {
            Some("world_surface") => "minecraft:heightmap".into(),
            Some("none") if values.len() == 3 && values[1] == "left_right" => {
                "minecraft:template_mirror".into()
            }
            Some("none") if values.len() == 3 => "minecraft:swing_animation".into(),
            _ => "minecraft:template_rotation".into(),
        },
        ArgType::HexColor => "minecraft:hex_color".into(),
        ArgType::ResourceSelector(_) => "minecraft:resource_selector".into(),
        ArgType::Uuid => "minecraft:uuid".into(),
        ArgType::GameProfile => "minecraft:game_profile".into(),
        ArgType::Style => "minecraft:style".into(),
        ArgType::Name(_) => "brigadier:string".into(),
        ArgType::Time { .. } => "minecraft:time".into(),
        ArgType::GameMode => "minecraft:gamemode".into(),
        ArgType::Message => "minecraft:message".into(),
        ArgType::Dimension => "minecraft:dimension".into(),
        ArgType::Anchor => "minecraft:entity_anchor".into(),
        ArgType::Swizzle => "minecraft:swizzle".into(),
        ArgType::Component => "minecraft:component".into(),
        ArgType::Nbt => "minecraft:nbt_compound_tag".into(),
        ArgType::ScoreHolder { .. } => "minecraft:score_holder".into(),
        ArgType::Objective => "minecraft:objective".into(),
        ArgType::Team => "minecraft:team".into(),
        ArgType::Criteria => "minecraft:objective_criteria".into(),
        ArgType::ScoreboardSlot => "minecraft:scoreboard_slot".into(),
        ArgType::Operation => "minecraft:operation".into(),
        ArgType::TeamColor => "minecraft:team_color".into(),
        ArgType::IntRange => "minecraft:int_range".into(),
        ArgType::FloatRange => "minecraft:float_range".into(),
        ArgType::NbtPath => "minecraft:nbt_path".into(),
        ArgType::NbtTag => "minecraft:nbt_tag".into(),
        ArgType::Vec2 => "minecraft:vec2".into(),
        ArgType::ColumnPos => "minecraft:column_pos".into(),
        ArgType::Particle => "minecraft:particle".into(),
        ArgType::Enum(words) => format!("enum:{}", words.join("|")),
        ArgType::Function => "minecraft:function".into(),
        ArgType::Slots => "minecraft:slots".into(),
        ArgType::Slot => "minecraft:item_slot".into(),
        ArgType::SlotSource => "minecraft:slot_source".into(),
        ArgType::ResourceOrId(r) => match *r {
            "minecraft:loot_table" => "minecraft:loot_table".into(),
            "minecraft:item_modifier" => "minecraft:loot_modifier".into(),
            "minecraft:predicate" => "minecraft:loot_predicate".into(),
            "minecraft:context_int_provider" => "minecraft:context_int_provider".into(),
            "minecraft:context_float_provider" => "minecraft:context_float_provider".into(),
            "minecraft:dialog" => "minecraft:dialog".into(),
            _ => "minecraft:feature".into(),
        },
    }
}

fn tree(d: &Dispatcher, node: usize) -> Value {
    let n = &d.nodes[node];
    let mut children = serde_json::Map::new();
    for &c in &n.children {
        let child = &d.nodes[c];
        let mut v = tree(d, c);
        match &child.kind {
            NodeKind::Literal(_) => v["type"] = json!("literal"),
            NodeKind::Argument { ty, .. } => {
                v["type"] = json!("argument");
                v["parser"] = json!(parser_id(ty));
            }
            NodeKind::Root => {}
        }
        children.insert(child.name().to_owned(), v);
    }
    let order: Vec<&str> = n.children.iter().map(|&c| d.nodes[c].name()).collect();
    let mut v = json!({"children": children, "order": order});
    if n.command.is_some() {
        v["executable"] = json!(true);
    }
    if n.redirect.is_some() {
        v["redirect"] = json!(true);
    }
    v
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    if args.get(1).map(String::as_str) == Some("tree") {
        let d = &vanilla::registered(false);
        let mut v = tree(d, 0);
        v["type"] = json!("root");
        fs::write(&args[2], v.to_string())?;
        return Ok(());
    }
    if args.len() != 6 {
        return Err(
            "usage: command_parse_fixture RESOURCES REGISTRIES CORPUS PERMISSION OUTPUT".into(),
        );
    }
    let root = Path::new(&args[1]).to_path_buf();
    lang::set(lang::Language::load(&[&root], lang::DEFAULT));
    let r: Value = serde_json::from_slice(&fs::read(&args[2])?)?;
    let mut reg = Registries {
        permission: args[4].parse()?,
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
    let dispatcher = vanilla::release_dispatcher();
    let mut out = String::new();
    for input in fs::read_to_string(&args[3])?.lines() {
        let parse = dispatcher.parse(input, &reg);
        let result = match parse.executable() {
            Ok(_) => {
                let mut nodes = Vec::new();
                let mut c = Some(&parse.context);
                while let Some(ctx) = c {
                    nodes.extend(
                        ctx.nodes
                            .iter()
                            .map(|(n, _)| dispatcher.nodes[*n].name().to_owned()),
                    );
                    c = ctx.child.as_deref();
                }
                json!({"ok": true, "nodes": nodes})
            }
            Err(e) => json!({
                "ok": false,
                "error": e.message.plain(),
                // CommandSyntaxException.getCursor is -1 without context.
                "cursor": if e.input.is_some() { utf16(input, e.cursor) as i64 } else { -1 },
            }),
        };
        let (range, list) = dispatcher.suggestions(&parse, input.len(), &reg);
        let list: Vec<String> = list
            .into_iter()
            .map(|s| match s.tooltip {
                Some(t) => format!("{}\t{t}", s.text),
                None => s.text,
            })
            .collect();
        let row = json!({
            "in": input,
            "parse": result,
            "sugg": {"start": utf16(input, range.0), "end": utf16(input, range.1), "list": list},
        });
        out.push_str(&row.to_string());
        out.push('\n');
    }
    fs::write(&args[5], out)?;
    Ok(())
}
